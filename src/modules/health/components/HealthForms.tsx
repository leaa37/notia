import { useState, type ReactNode } from 'react'
import { Sparkles, Trash2, X } from 'lucide-react'
import { NotiaModalShell } from '../../../components/notia/NotiaModalShell'
import { asHealthError } from '../services/healthService'
import type { HealthDashboard, HealthError, MealEstimate, MealInput, MeasurementInput, ProfileInput, RecentMeal } from '../types/healthTypes'
import { categoryIcon, toNumber } from './healthUi'

interface HealthModalProps {
  title: string
  wide?: boolean
  busy?: boolean
  /** En el celular el modal es una hoja que sube desde abajo, con su manija. */
  phone?: boolean
  onClose: () => void
  children: ReactNode
}

export function HealthModal({ title, wide = false, busy = false, phone = false, onClose, children }: HealthModalProps) {
  return (
    <NotiaModalShell
      open
      onClose={busy ? () => undefined : onClose}
      size="lg"
      panelClassName={phone ? 'hl-modal-panel hl-sheet' : 'hl-modal-panel'}
      panelStyle={phone ? undefined : { width: `min(${wide ? 720 : 460}px, calc(100vw - 24px))` }}
    >
      <div className="hl-modal">
        {phone && <span className="hl-sheet__handle" aria-hidden="true" />}
        <div className="hl-modal__head">
          <h2 className="hl-mid">{title}</h2>
          <button type="button" className="hl-icon-button" onClick={onClose} disabled={busy} aria-label="Cerrar"><X size={20} strokeWidth={phone ? 2 : 1.8} /></button>
        </div>
        <div className="hl-modal__body">{children}</div>
      </div>
    </NotiaModalShell>
  )
}

function fieldError(error: HealthError | null, ...fields: string[]): string | null {
  return error?.fields.find((item) => fields.includes(item.field))?.message ?? null
}

function FormAlert({ error }: { error: HealthError | null }) {
  if (!error) return null
  return <p className="hl-alert" role="alert">{error.message}</p>
}

const asText = (value: number | null | undefined) => (value === null || value === undefined ? '' : String(value).replace('.', ','))

/** Guarda y deja el error a la vista; `true` si se guardó. */
async function attempt(save: () => Promise<void>, setError: (error: HealthError | null) => void, setBusy: (busy: boolean) => void): Promise<boolean> {
  setBusy(true)
  setError(null)
  try {
    await save()
    return true
  } catch (reason) {
    setError(asHealthError(reason))
    return false
  } finally {
    setBusy(false)
  }
}

interface ProfileFormProps {
  dashboard: HealthDashboard
  phone?: boolean
  onSave: (input: ProfileInput) => Promise<void>
  onClose: () => void
}

export function ProfileFormView({ dashboard, phone = false, onSave, onClose }: ProfileFormProps) {
  const initial = dashboard.profileForm
  const [birthDate, setBirthDate] = useState(initial.birthDate)
  const [sex, setSex] = useState(initial.sex)
  const [height, setHeight] = useState(asText(initial.heightCm))
  const [weight, setWeight] = useState(asText(initial.weightKg))
  const [activity, setActivity] = useState(String(initial.activity))
  const [waterGoal, setWaterGoal] = useState(asText(initial.waterGoalLiters))
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<HealthError | null>(null)
  const ready = birthDate && height.trim() && weight.trim()
  const submit = async () => {
    const saved = await attempt(
      () => onSave({ birthDate, sex, heightCm: toNumber(height), activity: Number(activity), weightKg: toNumber(weight), waterGoalLiters: toNumber(waterGoal) }),
      setError,
      setBusy,
    )
    if (saved) onClose()
  }
  return (
    <HealthModal title="Tu perfil" busy={busy} phone={phone} onClose={onClose}>
      <form className="hl-form" noValidate onSubmit={(event) => { event.preventDefault(); void submit() }}>
        <FormAlert error={error && error.fields.length === 0 ? error : null} />
        <div className="hl-grid-2">
          <label className="hl-field">
            <span>Fecha de nacimiento</span>
            <input type="date" className="hl-input" value={birthDate} max={dashboard.today} onChange={(event) => setBirthDate(event.target.value)} aria-invalid={Boolean(fieldError(error, 'birthDate'))} />
            {fieldError(error, 'birthDate') && <span className="hl-error">{fieldError(error, 'birthDate')}</span>}
          </label>
          <label className="hl-field">
            <span>Sexo biológico</span>
            <select className="hl-input" value={sex} onChange={(event) => setSex(event.target.value)}>
              <option value="M">Masculino</option>
              <option value="F">Femenino</option>
            </select>
            <span className="hl-hint">Se usa para el metabolismo basal y los rangos.</span>
          </label>
          <label className="hl-field">
            <span>Altura (cm)</span>
            <input className="hl-input" inputMode="decimal" placeholder="178" value={height} onChange={(event) => setHeight(event.target.value)} aria-invalid={Boolean(fieldError(error, 'heightCm'))} />
            {fieldError(error, 'heightCm') && <span className="hl-error">{fieldError(error, 'heightCm')}</span>}
          </label>
          <label className="hl-field">
            <span>Peso actual (kg)</span>
            <input className="hl-input" inputMode="decimal" placeholder="82,4" value={weight} onChange={(event) => setWeight(event.target.value)} aria-invalid={Boolean(fieldError(error, 'weightKg'))} />
            {fieldError(error, 'weightKg') && <span className="hl-error">{fieldError(error, 'weightKg')}</span>}
          </label>
        </div>
        <label className="hl-field">
          <span>Nivel de actividad</span>
          <select className="hl-input" value={activity} onChange={(event) => setActivity(event.target.value)}>
            {initial.activities.map((option) => <option key={option.value} value={option.value}>{option.label}</option>)}
          </select>
        </label>
        <label className="hl-field">
          <span>Meta de agua (litros)</span>
          <input
            className="hl-input"
            inputMode="decimal"
            placeholder={initial.waterGoalAutoLabel ? `Calculada: ${initial.waterGoalAutoLabel}` : 'Calculada'}
            value={waterGoal}
            onChange={(event) => setWaterGoal(event.target.value)}
            aria-invalid={Boolean(fieldError(error, 'waterGoalLiters'))}
          />
          {fieldError(error, 'waterGoalLiters')
            ? <span className="hl-error">{fieldError(error, 'waterGoalLiters')}</span>
            : <span className="hl-hint">Vacía, Notia la calcula con tu altura y tu peso.</span>}
        </label>
        <div className="hl-form__foot">
          <button type="button" className="hl-button" onClick={onClose} disabled={busy}>Cancelar</button>
          <button type="submit" className="hl-button hl-button--primary" disabled={!ready || busy}>Guardar perfil</button>
        </div>
      </form>
    </HealthModal>
  )
}

interface MeasurementFormProps {
  dashboard: HealthDashboard
  phone?: boolean
  onSave: (input: MeasurementInput) => Promise<void>
  onClose: () => void
}

export function MeasurementFormView({ dashboard, phone = false, onSave, onClose }: MeasurementFormProps) {
  const [date, setDate] = useState(dashboard.today)
  const [weight, setWeight] = useState('')
  const [values, setValues] = useState<Record<string, string>>({})
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<HealthError | null>(null)
  const submit = async () => {
    const numbers = Object.fromEntries(
      Object.entries(values).map(([key, value]) => [key, toNumber(value)]).filter((entry): entry is [string, number] => entry[1] !== null),
    )
    const saved = await attempt(() => onSave({ date, weight: toNumber(weight), values: numbers }), setError, setBusy)
    if (saved) onClose()
  }
  return (
    <HealthModal title="Registrar medición de la balanza" wide busy={busy} phone={phone} onClose={onClose}>
      <form className="hl-form" noValidate onSubmit={(event) => { event.preventDefault(); void submit() }}>
        <FormAlert error={error} />
        <div className="hl-grid-2">
          <label className="hl-field">
            <span>Fecha</span>
            <input type="date" className="hl-input" value={date} max={dashboard.today} onChange={(event) => setDate(event.target.value)} />
          </label>
          <label className="hl-field">
            <span>Peso (kg)</span>
            <input className="hl-input" inputMode="decimal" value={weight} onChange={(event) => setWeight(event.target.value)} aria-invalid={Boolean(fieldError(error, 'weight'))} />
          </label>
        </div>
        {dashboard.measurementFields.map((group) => (
          <fieldset key={group.id} className="hl-fieldset" data-group={group.id}>
            <legend><i className="hl-square" aria-hidden="true" />{group.title}</legend>
            <div className="hl-grid-3">
              {group.fields.map((field) => (
                <label key={field.key} className="hl-field">
                  <span>{field.label}{field.unit && ` (${field.unit})`}</span>
                  <input
                    className="hl-input"
                    inputMode="decimal"
                    value={values[field.key] ?? ''}
                    onChange={(event) => setValues({ ...values, [field.key]: event.target.value })}
                    aria-invalid={Boolean(fieldError(error, field.key))}
                  />
                </label>
              ))}
            </div>
          </fieldset>
        ))}
        <p className="hl-small hl-faint">Si dejás vacíos el IMC, los kilos de grasa, agua, músculo o el peso sin grasa, se calculan a partir del peso y los porcentajes.</p>
        <div className="hl-form__foot">
          <button type="button" className="hl-button" onClick={onClose} disabled={busy}>Cancelar</button>
          <button type="submit" className="hl-button hl-button--primary" disabled={!weight.trim() || busy}>Guardar medición</button>
        </div>
      </form>
    </HealthModal>
  )
}

/** Comida a cargar o editar. */
export interface MealTarget {
  id: string | null
  date: string
  category: string | null
  values: RecentMeal | null
}

interface MealFormProps {
  dashboard: HealthDashboard
  target: MealTarget
  phone?: boolean
  onSave: (id: string | null, input: MealInput) => Promise<void>
  onDelete: (id: string) => Promise<void>
  onEstimate: (description: string) => Promise<MealEstimate>
  onClose: () => void
}

type MacroField = 'kcal' | 'proteinG' | 'carbsG' | 'fatG' | 'fiberG'
const MACRO_FIELDS: Array<{ key: Exclude<MacroField, 'kcal'>; label: string }> = [
  { key: 'proteinG', label: 'Proteínas (g)' },
  { key: 'carbsG', label: 'Carbohidratos (g)' },
  { key: 'fatG', label: 'Grasas (g)' },
  { key: 'fiberG', label: 'Fibra (g)' },
]

function valuesText(values: RecentMeal | MealEstimate | null): Record<MacroField, string> {
  return {
    kcal: asText(values?.kcal),
    proteinG: asText(values?.proteinG),
    carbsG: asText(values?.carbsG),
    fatG: asText(values?.fatG),
    fiberG: asText(values?.fiberG),
  }
}

export function MealFormView({ dashboard, target, phone = false, onSave, onDelete, onEstimate, onClose }: MealFormProps) {
  const [category, setCategory] = useState(target.values?.category ?? target.category ?? dashboard.mealForm.suggestedCategory)
  const [name, setName] = useState(target.values?.name ?? '')
  const [numbers, setNumbers] = useState(valuesText(target.values))
  const [busy, setBusy] = useState(false)
  const [estimating, setEstimating] = useState(false)
  const [error, setError] = useState<HealthError | null>(null)
  const suggestions = dashboard.mealForm.recent.filter((meal) => meal.category === category && meal.name !== name).slice(0, 5)

  const estimate = async () => {
    setEstimating(true)
    setError(null)
    try {
      setNumbers(valuesText(await onEstimate(name.trim())))
    } catch (reason) {
      setError(asHealthError(reason))
    } finally {
      setEstimating(false)
    }
  }
  const submit = async () => {
    const input: MealInput = {
      date: target.date,
      category,
      name,
      kcal: toNumber(numbers.kcal),
      proteinG: toNumber(numbers.proteinG),
      carbsG: toNumber(numbers.carbsG),
      fatG: toNumber(numbers.fatG),
      fiberG: toNumber(numbers.fiberG),
    }
    const saved = await attempt(() => onSave(target.id, input), setError, setBusy)
    if (saved) onClose()
  }
  const remove = async () => {
    if (!target.id) return
    const id = target.id
    const removed = await attempt(() => onDelete(id), setError, setBusy)
    if (removed) onClose()
  }
  return (
    <HealthModal title={target.id ? 'Editar comida' : 'Agregar comida'} wide busy={busy} phone={phone} onClose={onClose}>
      <form className="hl-form" noValidate onSubmit={(event) => { event.preventDefault(); void submit() }}>
        <div className="hl-field">
          <span id="hl-meal-category">Categoría</span>
          <div className="hl-seg hl-seg--wrap" role="radiogroup" aria-labelledby="hl-meal-category">
            {dashboard.mealForm.categories.map((option) => {
              const Icon = categoryIcon(option.value)
              return (
                <button key={option.value} type="button" role="radio" aria-checked={category === option.value} onClick={() => setCategory(option.value)}>
                  <Icon size={14} aria-hidden="true" />{option.label}
                </button>
              )
            })}
          </div>
        </div>
        <div className="hl-field">
          <label htmlFor="hl-meal-name">Qué comiste</label>
          <div className="hl-inline">
            <input id="hl-meal-name" className="hl-input" value={name} placeholder="Ej.: 2 empanadas de carne al horno" onChange={(event) => setName(event.target.value)} aria-invalid={Boolean(fieldError(error, 'name'))} />
            <button type="button" className="hl-button" disabled={!name.trim() || estimating || busy} onClick={() => { void estimate() }}>
              <Sparkles size={16} aria-hidden="true" />{estimating ? 'Estimando…' : 'Estimar con IA'}
            </button>
          </div>
          {fieldError(error, 'name') && <span className="hl-error">{fieldError(error, 'name')}</span>}
        </div>
        {suggestions.length > 0 && (
          <div className="hl-field">
            <span>Repetir una anterior</span>
            <div className="hl-chips">
              {suggestions.map((meal) => (
                <button key={meal.name} type="button" className="hl-suggestion" onClick={() => { setName(meal.name); setNumbers(valuesText(meal)) }}>{meal.name}</button>
              ))}
            </div>
          </div>
        )}
        <div className="hl-grid-5">
          <label className="hl-field">
            <span>Calorías</span>
            <input className="hl-input" inputMode="decimal" placeholder="kcal" value={numbers.kcal} onChange={(event) => setNumbers({ ...numbers, kcal: event.target.value })} aria-invalid={Boolean(fieldError(error, 'kcal'))} />
          </label>
          {MACRO_FIELDS.map((field) => (
            <label key={field.key} className="hl-field" data-macro={field.key}>
              <span><i className="hl-dot hl-dot--small" aria-hidden="true" />{field.label}</span>
              <input className="hl-input" inputMode="decimal" value={numbers[field.key]} onChange={(event) => setNumbers({ ...numbers, [field.key]: event.target.value })} />
            </label>
          ))}
        </div>
        <p className="hl-small hl-faint">Si no cargás calorías, se calculan desde los macros.</p>
        <FormAlert error={error} />
        <div className="hl-form__foot">
          {target.id && (
            <button type="button" className="hl-button hl-button--danger hl-form__delete" onClick={() => { void remove() }} disabled={busy}>
              <Trash2 size={16} aria-hidden="true" />Eliminar
            </button>
          )}
          <button type="button" className="hl-button" onClick={onClose} disabled={busy}>Cancelar</button>
          <button type="submit" className="hl-button hl-button--primary" disabled={!name.trim() || busy}>{target.id ? 'Guardar cambios' : 'Agregar comida'}</button>
        </div>
      </form>
    </HealthModal>
  )
}
