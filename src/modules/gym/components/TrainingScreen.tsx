import { useEffect, useState } from 'react'
import { AlertTriangle, Check, ChevronDown, ChevronLeft, ChevronUp, Clock, Info, Pause, Play } from 'lucide-react'
import type { GymMutation, TrainItem, TrainingView, TrainSet } from '../types/gymTypes'
import { CommitInput } from './GymInputs'
import { useConfirmationEngine } from '../../../context/confirmation/useConfirmationEngine'

const RING_RADIUS = 84
const RING_LENGTH = 2 * Math.PI * RING_RADIUS

interface TrainingScreenProps {
  training: TrainingView
  /** Diferencia entre el reloj de Rust y el de este dispositivo. */
  clockOffsetMs: number
  apply: (mutation: GymMutation) => void
  /** Rust todavía no respondió un cambio: los controles de la sesión esperan. */
  applying: boolean
  onBack: () => void
  onOpenExercise: (exerciseId: string) => void
  /** Versión celular: el reloj en una tarjeta y el descanso en una barra abajo. */
  phone?: boolean
}

function pad(value: number) {
  return value < 10 ? `0${value}` : String(value)
}

function clock(ms: number) {
  const total = Math.floor(Math.max(0, ms) / 1000)
  const hours = Math.floor(total / 3600)
  const minutes = Math.floor((total % 3600) / 60)
  const seconds = total % 60
  return hours > 0 ? `${hours}:${pad(minutes)}:${pad(seconds)}` : `${pad(minutes)}:${pad(seconds)}`
}

function restClock(ms: number) {
  const total = Math.ceil(Math.max(0, ms) / 1000)
  return `${Math.floor(total / 60)}:${pad(total % 60)}`
}

/** La hora de Rust, que avanza sola mientras corre el reloj o un descanso. */
function useServerNow(offsetMs: number, ticking: boolean) {
  const [now, setNow] = useState(() => Date.now() + offsetMs)
  useEffect(() => {
    setNow(Date.now() + offsetMs)
    if (!ticking) return undefined
    const timer = window.setInterval(() => setNow(Date.now() + offsetMs), 500)
    return () => window.clearInterval(timer)
  }, [offsetMs, ticking])
  return now
}

export function TrainingScreen({ training, clockOffsetMs, apply, applying, onBack, onOpenExercise, phone = false }: TrainingScreenProps) {
  const [open, setOpen] = useState<Record<string, boolean>>({})
  const routineId = training.routineId
  const expandedOf = (item: TrainItem) => open[item.id] ?? item.current

  return (
    <div className="gym-training">
      <main className="gym-main gym-training-main">
        {phone ? <button type="button" className="gym-back" onClick={onBack}><ChevronLeft size={18} aria-hidden="true" />Volver a la rutina</button> : null}
        <header className="gym-training-header">
          <div className="gym-routine-header-row">
            <div className="gym-routine-titles">
              <h1 className="gym-h1">{training.name}</h1>
              {phone ? null : <span className="gym-muted">{training.focusLabel}</span>}
            </div>
            {phone ? null : <button type="button" className="gym-button" onClick={onBack}><ChevronLeft size={18} aria-hidden="true" />Volver a la rutina</button>}
          </div>
          <div className="gym-progress">
            <div className="gym-progress-text">
              <span><strong>{training.doneSets}</strong> de {training.totalSets} series hechas</span>
              <span>{training.percent}%</span>
            </div>
            <div className="gym-progress-track"><div className="gym-progress-fill" style={{ width: `${training.percent}%` }} /></div>
          </div>
        </header>

        {phone ? <SessionClock training={training} clockOffsetMs={clockOffsetMs} apply={apply} applying={applying} /> : null}

        {training.items.length === 0 ? <div className="gym-empty">Esta rutina no tiene ejercicios. Pasá a Editar para sumarlos.</div> : null}

        <section aria-label="Ejercicios para hacer" className="gym-items">
          {training.items.map((item) => {
            const expanded = expandedOf(item)
            return (
              <article key={item.id} className="gym-item" data-group={item.group} data-current={item.current || undefined}>
                <div className="gym-item-head">
                  <button type="button" className="gym-item-toggle" aria-expanded={expanded} onClick={() => setOpen((current) => ({ ...current, [item.id]: !expanded }))}>
                    {item.allDone ? (
                      <span className="gym-item-number gym-item-number--done"><Check size={16} aria-hidden="true" /></span>
                    ) : (
                      <span className="gym-item-number">{item.n}</span>
                    )}
                    <span className="gym-item-text">
                      <span className="gym-item-title-row">
                        <span className="gym-item-name" data-done={item.allDone || undefined}>{item.name}</span>
                        {item.current ? <span className="gym-badge">En curso</span> : null}
                      </span>
                      <span className="gym-item-meta">
                        <span className="gym-item-group"><span className="gym-dot gym-dot--group" aria-hidden="true" />{item.groupLabel}</span>
                        <span className="gym-item-rest"><Clock size={12} aria-hidden="true" />{item.restLabel} de descanso</span>
                        {item.unilateral ? <span>Por lado</span> : null}
                      </span>
                      {item.missing ? <span className="gym-missing"><AlertTriangle size={13} aria-hidden="true" />{item.missing}</span> : null}
                      {item.sideText ? <span className="gym-side-text" role="status">{item.sideText}</span> : null}
                    </span>
                    <span className="gym-item-count">{item.count}</span>
                    {expanded ? <ChevronUp size={18} aria-hidden="true" /> : <ChevronDown size={18} aria-hidden="true" />}
                  </button>
                  <button type="button" className="gym-button gym-button--small" aria-label={`Ver cómo se hace ${item.name}`} onClick={() => onOpenExercise(item.exerciseId)}>
                    <Info size={15} aria-hidden="true" /><span className="gym-hide-narrow">Cómo se hace</span>
                  </button>
                </div>
                {expanded ? (
                  <div className="gym-sets gym-sets--train">
                    <div className="gym-set-row gym-set-row--train gym-set-row--head">
                      <span>Serie</span>
                      <span>Peso (kg)</span>
                      <span>{item.timed ? 'Segundos' : 'Repeticiones'}</span>
                      <span className="gym-center">Hecha</span>
                    </div>
                    {item.sets.map((set, index) => (
                      <div key={index} className="gym-set-row gym-set-row--train" data-next={set.next || undefined} data-done={set.done || undefined}>
                        <span className="gym-set-n">{index + 1}</span>
                        {item.weighted ? (
                          <CommitInput className="gym-number-input" inputMode="decimal" aria-label={`Peso de la serie ${index + 1}`} placeholder="0" value={set.weight} onCommit={(value) => apply({ type: 'set-value', routineId, itemId: item.id, index, field: 'weight', value })} />
                        ) : (
                          <span className="gym-bodyweight">Peso corporal</span>
                        )}
                        <CommitInput className="gym-number-input" inputMode="numeric" aria-label={`${item.timed ? 'Segundos' : 'Repeticiones'} de la serie ${index + 1}`} placeholder="0" value={set.reps} onCommit={(value) => apply({ type: 'set-value', routineId, itemId: item.id, index, field: 'reps', value })} />
                        <button
                          type="button"
                          className="gym-check"
                          disabled={training.status === 'done'}
                          aria-pressed={set.done}
                          data-side={set.oneSide || undefined}
                          aria-label={checkLabel(item, set, index)}
                          onClick={() => apply({ type: 'toggle-set-done', routineId, itemId: item.id, index })}
                        >
                          {set.oneSide ? null : <Check size={20} aria-hidden="true" />}
                        </button>
                      </div>
                    ))}
                  </div>
                ) : null}
              </article>
            )
          })}
        </section>
      </main>

      {phone ? (
        <RestBar training={training} clockOffsetMs={clockOffsetMs} apply={apply} />
      ) : (
        <TrainingTimers training={training} clockOffsetMs={clockOffsetMs} apply={apply} applying={applying} />
      )}
    </div>
  )
}

type TimerProps = Pick<TrainingScreenProps, 'training' | 'clockOffsetMs' | 'apply'>
type ControlProps = TimerProps & Pick<TrainingScreenProps, 'applying'>

/**
 * Los relojes y el descanso: solo esta parte se vuelve a dibujar mientras
 * corre el tiempo, no la lista de ejercicios.
 */
function TrainingTimers({ training, clockOffsetMs, apply, applying }: ControlProps) {
  const running = training.status === 'running'
  const now = useServerNow(clockOffsetMs, running)
  const elapsed = training.accMs + (running ? Math.max(0, now - training.startedMs) : 0)
  return (
    <aside className="gym-side gym-timers" aria-label="Temporizadores">
      <SessionClock training={training} clockOffsetMs={clockOffsetMs} apply={apply} applying={applying} />
      <RestRing training={training} clockOffsetMs={clockOffsetMs} apply={apply} />
      <section aria-label="Resumen de la sesión" className="gym-mini-stats gym-mini-stats--three">
        <div><span>Series</span><strong>{training.doneSets}/{training.totalSets}</strong></div>
        <div><span>Levantado</span><strong>{training.volume}</strong></div>
        <div><span>Calorías aprox.</span><strong>≈ {Math.round((elapsed / 60000) * training.kcalPerMin).toLocaleString('es-AR')}</strong></div>
      </section>
    </aside>
  )
}

/** El tiempo del entrenamiento y sus controles (empezar, pausar, terminar…). */
function SessionClock({ training, clockOffsetMs, apply, applying }: ControlProps) {
  const { confirm } = useConfirmationEngine()
  const running = training.status === 'running'
  const now = useServerNow(clockOffsetMs, running)
  const elapsed = training.accMs + (running ? Math.max(0, now - training.startedMs) : 0)
  const routineId = training.routineId

  // Mientras Rust no respondió un cambio, los controles de la sesión no
  // toman toques: los que quedaron encolados con la pantalla trabada caerían
  // sobre el botón que aparece en el mismo lugar. No se ven deshabilitados,
  // para que no parpadeen con cada serie.
  const control = (action: () => void) => () => {
    if (!applying) action()
  }

  // Terminar guarda el entrenamiento: un toque de más no lo cierra a mitad de camino.
  const finish = () => {
    void confirm({
      title: 'Terminar entrenamiento',
      message: `¿Terminar el entrenamiento? Se guarda con ${training.doneSets} de ${training.totalSets} series hechas.`,
      confirmLabel: 'Terminar',
      cancelLabel: 'Seguir entrenando',
    }).then((accepted) => {
      if (accepted) apply({ type: 'finish-session' })
    })
  }

  return (
    <section aria-label="Tiempo de entrenamiento" className="gym-timer-block">
      <span className="gym-muted">
        {training.status === 'paused' ? 'Tiempo de entrenamiento (en pausa)' : training.status === 'done' ? 'Tiempo total' : 'Tiempo de entrenamiento'}
      </span>
      <span role="timer" className="gym-clock">{clock(elapsed)}</span>
      {training.status === 'idle' ? (
        <button type="button" className="gym-button gym-button--primary gym-button--big" aria-disabled={applying || undefined} onClick={control(() => apply({ type: 'start-session', routineId }))}>
          <Play size={16} fill="currentColor" aria-hidden="true" />Empezar entrenamiento
        </button>
      ) : null}
      {training.status === 'running' || training.status === 'paused' ? (
        <div className="gym-two-buttons">
          {running ? (
            <button type="button" className="gym-button gym-button--big gym-button--raised" aria-disabled={applying || undefined} onClick={control(() => apply({ type: 'pause-session' }))}><Pause size={15} fill="currentColor" aria-hidden="true" />Pausar</button>
          ) : (
            <button type="button" className="gym-button gym-button--primary gym-button--big" aria-disabled={applying || undefined} onClick={control(() => apply({ type: 'resume-session' }))}><Play size={15} fill="currentColor" aria-hidden="true" />Reanudar</button>
          )}
          <button type="button" className="gym-button gym-button--big gym-button--danger" aria-disabled={applying || undefined} onClick={control(finish)}>Terminar</button>
        </div>
      ) : null}
      {training.status === 'done' ? (
        <div className="gym-done">
          <span>{training.doneText}</span>
          <button type="button" className="gym-button gym-button--big gym-button--raised" aria-disabled={applying || undefined} onClick={control(() => apply({ type: 'restart-session' }))}>Empezar de nuevo</button>
        </div>
      ) : null}
    </section>
  )
}

/** Qué hace tocar la serie: en un ejercicio de a un lado, cada lado. */
function checkLabel(item: TrainItem, set: TrainSet, index: number) {
  const n = index + 1
  if (set.done) return `Desmarcar serie ${n}`
  if (set.oneSide) return `Marcar el otro lado de la serie ${n}`
  return item.unilateral ? `Marcar el primer lado de la serie ${n}` : `Marcar como hecha la serie ${n}`
}

/** El descanso que corre o terminó, con la hora de Rust. */
function useRest(training: TrainingView, clockOffsetMs: number) {
  const now = useServerNow(clockOffsetMs, training.restEndMs !== null)
  const resting = (training.restEndMs !== null && training.restEndMs > now) || training.restLeftMs !== null
  const ready = training.restEndMs !== null && training.restEndMs <= now
  const remaining = training.restEndMs !== null ? Math.max(0, training.restEndMs - now) : (training.restLeftMs ?? 0)
  const fraction = resting ? remaining / Math.max(1, training.restTotalMs) : ready ? 1 : 0
  const nextText = training.nextText
  const side = training.items.find((item) => item.sideText)?.sideText ?? null
  const text = side ?? (resting
    ? `Después: ${nextText ?? 'terminaste'}.`
    : ready
      ? nextText ? `Seguí con la ${nextText}.` : 'Completaste todas las series.'
      : nextText ? 'Marcá una serie como hecha y el descanso arranca solo.' : 'Completaste todas las series.')
  const sub = resting ? (training.restLeftMs !== null ? 'Pausado' : `de ${restClock(training.restTotalMs)}`) : ready ? 'Terminó el descanso' : 'Sin descanso activo'
  return { resting, ready, fraction, text, sub, center: resting ? restClock(remaining) : ready ? '0:00' : '–' }
}

type Rest = ReturnType<typeof useRest>

function RestButtons({ rest, apply }: { rest: Rest; apply: TimerProps['apply'] }) {
  const off = !rest.resting && !rest.ready
  return (
    <div className="gym-three-buttons">
      <button type="button" className="gym-step-button gym-step-button--wide" disabled={off} onClick={() => apply({ type: 'adjust-rest-timer', deltaMs: -15000 })}>−15 s</button>
      <button type="button" className="gym-step-button gym-step-button--wide" disabled={off} onClick={() => apply({ type: 'adjust-rest-timer', deltaMs: 15000 })}>+15 s</button>
      <button type="button" className="gym-step-button gym-step-button--wide" disabled={off} onClick={() => apply({ type: 'skip-rest' })}>{rest.ready ? 'Cerrar' : 'Saltar'}</button>
    </div>
  )
}

function RingSvg({ size, fraction }: { size: number; fraction: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 200 200" aria-hidden="true">
      <circle cx="100" cy="100" r={RING_RADIUS} className="gym-ring-track" />
      <circle
        cx="100"
        cy="100"
        r={RING_RADIUS}
        className="gym-ring-fill"
        strokeDasharray={RING_LENGTH.toFixed(2)}
        strokeDashoffset={(RING_LENGTH * (1 - fraction)).toFixed(2)}
        transform="rotate(-90 100 100)"
      />
    </svg>
  )
}

/** El descanso con el anillo grande, en la columna de los relojes. */
function RestRing({ training, clockOffsetMs, apply }: TimerProps) {
  const rest = useRest(training, clockOffsetMs)
  return (
    <section aria-label="Descanso" className="gym-timer-block gym-rest">
      <div className="gym-rest-head">
        <h2 className="gym-h2">Descanso</h2>
        <span className="gym-muted">{training.currentRest ?? ''}</span>
      </div>
      <div className="gym-ring" data-ready={rest.ready || undefined}>
        <RingSvg size={208} fraction={rest.fraction} />
        <div role="timer" className="gym-ring-center" data-idle={!rest.resting && !rest.ready ? true : undefined}>
          <span className="gym-ring-time">{rest.center}</span>
          <span className="gym-muted">{rest.sub}</span>
        </div>
      </div>
      <p className="gym-ring-next">{rest.text}</p>
      <RestButtons rest={rest} apply={apply} />
    </section>
  )
}

/** Celular: el descanso queda abajo, siempre a mano, con un anillo chico. */
function RestBar({ training, clockOffsetMs, apply }: TimerProps) {
  const rest = useRest(training, clockOffsetMs)
  return (
    <section aria-label="Descanso" className="gym-restbar">
      <div className="gym-restbar-row">
        <div className="gym-ring gym-ring--small" data-ready={rest.ready || undefined}>
          <RingSvg size={64} fraction={rest.fraction} />
          <span role="timer" className="gym-ring-center gym-ring-center--small" data-idle={!rest.resting && !rest.ready ? true : undefined}>{rest.center}</span>
        </div>
        <div className="gym-restbar-text">
          <span className="gym-restbar-title">Descanso <span className="gym-muted">{rest.sub}</span></span>
          <span className="gym-muted">{rest.text}</span>
        </div>
      </div>
      <RestButtons rest={rest} apply={apply} />
    </section>
  )
}
