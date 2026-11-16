import { useEffect, useState } from 'react'
import { Minus, Pencil, Plus, Video, X } from 'lucide-react'
import { NotiaModalShell } from '../../../components/notia/NotiaModalShell'
import type { NotiaLibrary } from '../../../types/notia'
import { applyCatalogMutation, asGymError, getExercise, getExerciseVideo, readMedia, setExerciseMedia } from '../services/gymService'
import type { BodyView, Choice, ExerciseDetail, ExerciseEdit } from '../types/gymTypes'
import { BodyGraph } from './BodyGraph'
import { CommitInput, CommitTextArea } from './GymInputs'

interface ExerciseCardProps {
  library: NotiaLibrary
  exerciseId: string
  editable: boolean
  canEdit: boolean
  groups: Choice[]
  body: BodyView | null
  onClose: () => void
  onEdit: () => void
  onNotice: (message: string) => void
  /** Versión celular: la ficha ocupa toda la pantalla. */
  phone?: boolean
}

/** Un video de Rust como URL que el `<video>` puede reproducir. */
function videoUrl(base64: string, mediaType: string): string {
  const bytes = Uint8Array.from(atob(base64), (character) => character.charCodeAt(0))
  return URL.createObjectURL(new Blob([bytes], { type: mediaType }))
}

export function ExerciseCard({ library, exerciseId, editable, canEdit, groups, body, onClose, onEdit, onNotice, phone = false }: ExerciseCardProps) {
  const [detail, setDetail] = useState<ExerciseDetail | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [video, setVideo] = useState<string | null>(null)
  const [uploading, setUploading] = useState(false)

  useEffect(() => {
    let current = true
    setDetail(null)
    void getExercise(library, exerciseId)
      .then((next) => { if (current) setDetail(next) })
      .catch((reason) => { if (current) setError(asGymError(reason).message) })
    return () => { current = false }
  }, [library, exerciseId])

  const hasVideo = detail?.hasVideo ?? false
  useEffect(() => {
    if (!hasVideo) {
      setVideo(null)
      return undefined
    }
    let url: string | null = null
    let current = true
    void getExerciseVideo(library, exerciseId)
      .then((media) => {
        if (!current || !media) return
        url = videoUrl(media.base64, media.mediaType)
        setVideo(url)
      })
      .catch(() => undefined)
    return () => {
      current = false
      if (url) URL.revokeObjectURL(url)
    }
  }, [library, exerciseId, hasVideo, detail?.path])

  const edit = async (change: ExerciseEdit) => {
    try {
      const result = await applyCatalogMutation(library, { type: 'update-exercise', exerciseId, edit: change })
      if (result.exercise) setDetail(result.exercise)
    } catch (reason) {
      onNotice(asGymError(reason).message)
    }
  }

  const upload = async (file: File) => {
    setUploading(true)
    try {
      setDetail(await setExerciseMedia(library, exerciseId, await readMedia(file)))
    } catch (reason) {
      onNotice(asGymError(reason).message)
    } finally {
      setUploading(false)
    }
  }

  const levels = detail?.muscles ?? {}
  const hasMedia = Boolean(detail && (detail.hasVideo || detail.image))

  return (
    <NotiaModalShell open onClose={onClose} size="xl" fill panelClassName={`gym-modal-panel gym-card-panel${phone ? ' gym-card-panel--phone' : ''}`}>
      <div className="gym-exercise" role="dialog" aria-modal="true" aria-label={`Ficha de ${detail?.name ?? 'ejercicio'}`} data-group={detail?.group}>
        <header className="gym-exercise-head">
          <div className="gym-exercise-titles">
            {detail && editable ? (
              <>
                <CommitInput className="gym-title-input gym-title-input--card" aria-label="Nombre del ejercicio" placeholder="Nombre del ejercicio" value={detail.name} onCommit={(value) => void edit({ field: 'name', value })} />
                <div className="gym-exercise-controls">
                  <label className="gym-inline-field">
                    Grupo
                    <select value={detail.group} onChange={(event) => void edit({ field: 'group', value: event.target.value })}>
                      {groups.map((group) => <option key={group.key} value={group.key}>{group.label}</option>)}
                    </select>
                  </label>
                  <button type="button" className="gym-switch" aria-pressed={detail.weighted} onClick={() => void edit({ field: 'weighted', value: !detail.weighted })}>
                    <span className="gym-switch-track" aria-hidden="true"><span className="gym-switch-knob" /></span>Se hace con peso
                  </button>
                  <button type="button" className="gym-switch" aria-pressed={detail.timed} onClick={() => void edit({ field: 'timed', value: !detail.timed })}>
                    <span className="gym-switch-track" aria-hidden="true"><span className="gym-switch-knob" /></span>Por tiempo
                  </button>
                  <button type="button" className="gym-switch" aria-pressed={detail.unilateral} onClick={() => void edit({ field: 'unilateral', value: !detail.unilateral })}>
                    <span className="gym-switch-track" aria-hidden="true"><span className="gym-switch-knob" /></span>De a un lado
                  </button>
                </div>
              </>
            ) : (
              <>
                <h2 className="gym-h1 gym-h1--card">{detail?.name ?? (error ? 'No se pudo abrir la ficha' : 'Cargando…')}</h2>
                {detail ? <span className="gym-muted gym-exercise-meta"><span className="gym-dot gym-dot--group" aria-hidden="true" />{detail.meta}</span> : null}
              </>
            )}
          </div>
          {canEdit && !editable ? <button type="button" className="gym-button" onClick={onEdit}><Pencil size={16} aria-hidden="true" />Editar ficha</button> : null}
          {editable ? <button type="button" className="gym-button gym-button--primary" onClick={onClose}>Listo</button> : null}
          <button type="button" className="gym-icon-button gym-icon-button--boxed" aria-label="Cerrar ficha" onClick={onClose}><X size={18} /></button>
        </header>

        {error ? <p className="gym-error" role="alert">{error}</p> : null}
        {detail ? (
          <div className="gym-exercise-body">
            <div className="gym-exercise-column">
              <section aria-label="Demostración" className="gym-side-section">
                <div className="gym-field-row">
                  <h3 className="gym-h3">Demostración</h3>
                  {editable && hasMedia ? <button type="button" className="gym-text-button gym-text-button--muted" onClick={() => void edit({ field: 'remove-media' })}>Quitar</button> : null}
                </div>
                {video ? (
                  <div className="gym-media"><video src={video} controls loop muted autoPlay playsInline /></div>
                ) : detail.image ? (
                  <div className="gym-media"><img src={detail.image} alt={`Demostración de ${detail.name}`} /></div>
                ) : editable ? null : (
                  <div className="gym-media gym-media--empty"><strong>Sin demostración cargada</strong><span className="gym-muted">Podés sumar un video o una imagen desde Editar ficha.</span></div>
                )}
                {editable ? (
                  <label className="gym-upload" data-busy={uploading || undefined}>
                    <input type="file" accept="video/mp4,video/webm,video/quicktime,image/*" disabled={uploading} onChange={(event) => { const file = event.target.files?.[0]; if (file) void upload(file) }} />
                    <span className="gym-add-icon"><Video size={22} aria-hidden="true" /></span>
                    <strong>{uploading ? 'Subiendo…' : hasMedia ? 'Reemplazar el video o la imagen' : 'Subí un video, GIF o imagen del movimiento'}</strong>
                    <span className="gym-muted">MP4, WEBM, GIF, PNG o JPG</span>
                  </label>
                ) : null}
              </section>

              <section aria-label="Pasos" className="gym-side-section">
                <h3 className="gym-h3">Cómo se hace</h3>
                {detail.steps.length === 0 ? (
                  <p className="gym-muted">{editable ? 'Todavía no hay pasos. Escribí el movimiento de a un paso por vez.' : 'Todavía no hay pasos cargados para este ejercicio.'}</p>
                ) : null}
                <ol className="gym-steps">
                  {detail.steps.map((step, index) => (
                    <li key={index}>
                      <span className="gym-step-number">{index + 1}</span>
                      {editable ? (
                        <>
                          <CommitTextArea label={`Paso ${index + 1}`} placeholder="Describí este paso" value={step} onCommit={(text) => void edit({ field: 'step', index, text })} />
                          <button type="button" className="gym-icon-button" aria-label={`Quitar paso ${index + 1}`} onClick={() => void edit({ field: 'remove-step', index })}><X size={16} /></button>
                        </>
                      ) : (
                        <p>{step}</p>
                      )}
                    </li>
                  ))}
                </ol>
                {editable ? <button type="button" className="gym-text-button" onClick={() => void edit({ field: 'add-step' })}><Plus size={18} aria-hidden="true" />Agregar paso</button> : null}
              </section>
            </div>

            <div className="gym-exercise-column">
              <section aria-label="Músculos que trabaja" className="gym-side-section">
                <div className="gym-field-row">
                  <h3 className="gym-h3">Músculos que trabaja</h3>
                  <span className="gym-level-key">
                    <span data-level="2"><span className="gym-swatch" aria-hidden="true" />Principal</span>
                    <span data-level="1"><span className="gym-swatch" aria-hidden="true" />Secundario</span>
                  </span>
                </div>
                <div className="gym-body-pair gym-body-pair--boxed gym-body-pair--wide">
                  <figure>
                    <BodyGraph figure={body?.front ?? null} tones={levels} scale="level" height={300} label={`Músculos de ${detail.name}, de frente`} onPick={editable ? (muscle) => void edit({ field: 'cycle-muscle', muscle }) : undefined} />
                    <figcaption>Frente</figcaption>
                  </figure>
                  <figure>
                    <BodyGraph figure={body?.back ?? null} tones={levels} scale="level" height={300} label={`Músculos de ${detail.name}, de espalda`} onPick={editable ? (muscle) => void edit({ field: 'cycle-muscle', muscle }) : undefined} />
                    <figcaption>Espalda</figcaption>
                  </figure>
                </div>
                {editable ? (
                  <>
                    <p className="gym-muted gym-small">Tocá un músculo en el cuerpo o en la lista: una vez lo marca como principal, otra como secundario y otra lo quita.</p>
                    <div role="group" aria-label="Músculos" className="gym-chips">
                      {detail.muscleChips.map((chip) => (
                        <button key={chip.key} type="button" className="gym-chip gym-muscle-chip" data-level={chip.level} aria-label={`${chip.label}: ${chip.level === 2 ? 'principal' : chip.level === 1 ? 'secundario' : 'no trabaja'}`} onClick={() => void edit({ field: 'cycle-muscle', muscle: chip.key })}>{chip.label}</button>
                      ))}
                    </div>
                  </>
                ) : (
                  <div className="gym-level-text">
                    <span><strong>Principales:</strong> {detail.primaryText}</span>
                    <span><strong>Secundarios:</strong> {detail.secondaryText}</span>
                  </div>
                )}
              </section>

              <section aria-label="Equipamiento necesario" className="gym-side-section">
                <h3 className="gym-h3">Equipamiento necesario</h3>
                {editable ? (
                  <div role="group" aria-label="Equipamiento necesario" className="gym-chips gym-chips--scroll">
                    {detail.equipment.map((item) => (
                      <button key={item.id} type="button" className="gym-chip" aria-pressed={item.required} onClick={() => void edit({ field: 'toggle-equipment', equipmentId: item.id })}>{item.name}</button>
                    ))}
                  </div>
                ) : (
                  <span>{detail.requiredText}</span>
                )}
                <span className="gym-status" data-missing={detail.missing || undefined}><span className="gym-dot" aria-hidden="true" />{detail.statusText}</span>
              </section>

              <section aria-label="Calorías" className="gym-side-section">
                <h3 className="gym-h3">Calorías</h3>
                {editable ? (
                  <div className="gym-kcal-stepper">
                    <button type="button" className="gym-step-button" aria-label="Restar medio kcal por minuto" onClick={() => void edit({ field: 'kcal-step', delta: -0.5 })}><Minus size={14} /></button>
                    <CommitInput className="gym-number-input gym-number-input--kcal" inputMode="decimal" aria-label="Calorías por minuto" value={detail.kcal} onCommit={(value) => void edit({ field: 'kcal', value })} />
                    <button type="button" className="gym-step-button" aria-label="Sumar medio kcal por minuto" onClick={() => void edit({ field: 'kcal-step', delta: 0.5 })}><Plus size={14} /></button>
                    <span className="gym-muted">kcal por minuto de trabajo</span>
                  </div>
                ) : (
                  <span className="gym-muted"><strong className="gym-figure-strong">{detail.kcal}</strong> kcal por minuto de trabajo</span>
                )}
                <div className="gym-mini-stats gym-mini-stats--three">
                  {detail.projections.map((projection) => <div key={projection.label}><span>{projection.label}</span><strong>{projection.value}</strong></div>)}
                </div>
                <p className="gym-muted gym-small">Es una estimación de referencia: el gasto real cambia con tu peso, la carga y el ritmo.</p>
              </section>
            </div>
          </div>
        ) : null}
      </div>
    </NotiaModalShell>
  )
}
