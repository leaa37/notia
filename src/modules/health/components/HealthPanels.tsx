import { useState } from 'react'
import { asHealthError } from '../services/healthService'
import type { AdjustView, BmiPanel, DailyPanel, ObjectivePanel, WaterPanel, WeightPanel } from '../types/healthTypes'
import { RangeBarView, WeightChartView } from './HealthCharts'
import { toNumber, type DailyPanelCopy } from './healthUi'

export function BmiPanelView({ panel }: { panel: BmiPanel }) {
  return (
    <section className="hl-panel hl-bmi" aria-labelledby="hl-bmi-title">
      <div>
        <h2 id="hl-bmi-title" className="hl-h2">Índice de masa corporal</h2>
        <div className="hl-bmi__value">
          <span className="hl-big">{panel.valueLabel}</span>
          <span className="hl-chip hl-chip--large" data-tone={panel.state.tone}>{panel.state.label}</span>
        </div>
        <p className="hl-small hl-muted">{panel.message}</p>
      </div>
      <div className="hl-bmi__bar">
        <RangeBarView bar={panel.bar} height={14} />
        <div className="hl-legend">
          <span><i className="hl-legend__mark" data-kind="value" aria-hidden="true" />{panel.todayLabel}</span>
          {panel.goal && (
            <span>
              <i className="hl-legend__mark" data-kind="goal" aria-hidden="true" />{panel.goal.label}
              {panel.goal.state && <span className="hl-chip" data-tone={panel.goal.state.tone}>{panel.goal.state.label}</span>}
            </span>
          )}
          <span className="hl-muted">{panel.healthyRangeLabel}</span>
        </div>
      </div>
    </section>
  )
}

interface WeightPanelProps {
  panel: WeightPanel
  today: string
  onRange: (range: string) => void
  onAdd: (date: string, kg: number | null) => Promise<boolean>
  onDelete: (date: string) => void
}

export function WeightPanelView({ panel, today, onRange, onAdd, onDelete }: WeightPanelProps) {
  const [date, setDate] = useState(today)
  const [kg, setKg] = useState('')
  const [showList, setShowList] = useState(false)
  const save = async () => {
    if (!kg.trim()) return
    if (await onAdd(date, toNumber(kg))) setKg('')
  }
  return (
    <section className="hl-panel hl-weight" aria-labelledby="hl-weight-title">
      <div className="hl-panel__top">
        <div>
          <h2 id="hl-weight-title" className="hl-h2">Peso</h2>
          <div className="hl-figure"><span className="hl-big">{panel.currentLabel}</span><span className="hl-muted">kg</span></div>
          {panel.summary && <p className="hl-small hl-muted">{panel.summary}</p>}
        </div>
        <div className="hl-seg" role="group" aria-label="Período del gráfico">
          {panel.ranges.map((range) => (
            <button key={range.value} type="button" aria-pressed={range.selected} onClick={() => onRange(range.value)}>{range.label}</button>
          ))}
        </div>
      </div>
      <div className="hl-weight__chart">
        {panel.chart ? <WeightChartView chart={panel.chart} /> : <div className="hl-well hl-empty-chart">{panel.emptyText}</div>}
      </div>
      <form className="hl-weight__form" onSubmit={(event) => { event.preventDefault(); void save() }}>
        <label className="hl-field">
          <span>Fecha</span>
          <input type="date" className="hl-input" value={date} max={today} onChange={(event) => setDate(event.target.value)} />
        </label>
        <label className="hl-field hl-field--narrow">
          <span>Peso (kg)</span>
          <input className="hl-input" inputMode="decimal" placeholder="82,4" value={kg} onChange={(event) => setKg(event.target.value)} />
        </label>
        <button type="submit" className="hl-button hl-button--primary" disabled={!kg.trim()}>Agregar peso</button>
        <button type="button" className="hl-button hl-weight__toggle" aria-expanded={showList} onClick={() => setShowList(!showList)}>
          {showList ? 'Ocultar registros' : `Ver registros (${panel.entries.length})`}
        </button>
      </form>
      {showList && (
        <ul className="hl-well hl-records">
          {panel.entries.map((entry) => (
            <li key={entry.date}>
              <span className="hl-muted">{entry.dateLabel}</span>
              <span className="hl-records__value">
                <span>{entry.kgLabel}</span>
                <button type="button" className="hl-link" onClick={() => onDelete(entry.date)} aria-label={`Eliminar peso del ${entry.dateLabel}`}>Eliminar</button>
              </span>
            </li>
          ))}
        </ul>
      )}
    </section>
  )
}


interface DailyPanelProps {
  panel: DailyPanel
  copy: DailyPanelCopy
  today: string
  onRange: (range: string) => void
  /** El valor como se escribió («8.432», «7:30»); Rust lo lee y lo valida. */
  onAdd: (date: string, value: string) => Promise<boolean>
  onDelete: (date: string) => void
}

/** Pasos o sueño: el valor del día, el promedio, el gráfico, la carga y los registros. */
export function DailyPanelView({ panel, copy, today, onRange, onAdd, onDelete }: DailyPanelProps) {
  const [date, setDate] = useState(today)
  const [value, setValue] = useState('')
  const [showList, setShowList] = useState(false)
  const titleId = `hl-${copy.id}-title`
  const save = async () => {
    if (!value.trim()) return
    if (await onAdd(date, value.trim())) setValue('')
  }
  return (
    <section className={`hl-panel hl-daily hl-daily--${copy.id}`} aria-labelledby={titleId}>
      <div className="hl-panel__top">
        <div>
          <h2 id={titleId} className="hl-h2">{copy.title}</h2>
          <div className="hl-figure">
            <span className="hl-big">{panel.todayLabel}</span>
            {copy.unit && panel.todayLabel !== '—' && <span className="hl-muted">{copy.unit}</span>}
            <span className="hl-small hl-muted">{panel.todayCaption}</span>
          </div>
          {panel.summary && <p className="hl-small hl-muted">{panel.summary}</p>}
          {panel.heart && (
            <p className={panel.heart.tone === 'warn' ? 'hl-small hl-warn' : 'hl-small hl-muted'}>
              <span className="hl-strong">{panel.heart.label}.</span> {panel.heart.detail}
            </p>
          )}
        </div>
        <div className="hl-seg" role="group" aria-label={`Período del gráfico de ${copy.title.toLowerCase()}`}>
          {panel.ranges.map((range) => (
            <button key={range.value} type="button" aria-pressed={range.selected} onClick={() => onRange(range.value)}>{range.label}</button>
          ))}
        </div>
      </div>
      <div className="hl-weight__chart">
        {panel.chart ? <WeightChartView chart={panel.chart} /> : <div className="hl-well hl-empty-chart">{panel.emptyText}</div>}
      </div>
      <form className="hl-weight__form" onSubmit={(event) => { event.preventDefault(); void save() }}>
        <label className="hl-field">
          <span>Fecha</span>
          <input type="date" className="hl-input" value={date} max={today} onChange={(event) => setDate(event.target.value)} />
        </label>
        <label className="hl-field hl-field--narrow">
          <span>{copy.inputLabel}</span>
          <input className="hl-input" inputMode={copy.id === 'steps' ? 'numeric' : 'text'} placeholder={copy.placeholder} value={value} onChange={(event) => setValue(event.target.value)} />
        </label>
        <button type="submit" className="hl-button hl-button--primary" disabled={!value.trim()}>{copy.addLabel}</button>
        <button type="button" className="hl-button hl-weight__toggle" aria-expanded={showList} onClick={() => setShowList(!showList)}>
          {showList ? 'Ocultar registros' : `Ver registros (${panel.entries.length})`}
        </button>
      </form>
      {showList && (
        <ul className="hl-well hl-records">
          {panel.entries.map((entry) => (
            <li key={entry.date}>
              <span className="hl-muted">{entry.dateLabel}</span>
              <span className="hl-records__value">
                <span>{entry.valueLabel}</span>
                <button type="button" className="hl-link" onClick={() => onDelete(entry.date)} aria-label={`Eliminar ${copy.recordName} del ${entry.dateLabel}`}>Eliminar</button>
              </span>
            </li>
          ))}
        </ul>
      )}
    </section>
  )
}

interface ObjectivePanelProps {
  panel: ObjectivePanel
  onObjective: (targetKg: number | null, pace: number) => void
  onGeneratePlan: () => Promise<void>
  onCalculatePlan: () => Promise<void>
  onClearPlan: () => void
  onActivity: (activity: number) => void
  onMeasuredExpenditure: (enabled: boolean) => void
}

/** El gasto medido y el nivel de actividad que muestran tus datos. */
function AdjustBlock({ adjust, onActivity, onMeasuredExpenditure }: { adjust: AdjustView; onActivity: (activity: number) => void; onMeasuredExpenditure: (enabled: boolean) => void }) {
  return (
    <div className="hl-well hl-adjust">
      <h3 className="hl-small hl-strong">Ajustes con tus datos</h3>
      <dl className="hl-pairs">
        <dt>Gasto por fórmula</dt><dd>{adjust.formulaLabel ?? '—'}</dd>
        <dt>Gasto medido</dt><dd>{adjust.measuredLabel ?? '—'}</dd>
      </dl>
      <p className="hl-small hl-muted">{adjust.measuredDetail}</p>
      {adjust.canUseMeasured && (
        <label className="hl-small">
          <input type="checkbox" checked={adjust.usingMeasured} onChange={(event) => onMeasuredExpenditure(event.target.checked)} />
          {' '}Usar el gasto medido para mis calorías
        </label>
      )}
      <p className="hl-small">Nivel de actividad: {adjust.activityLabel}</p>
      {adjust.activitySuggestion && (
        <>
          <p className="hl-small">Según tus datos sería <span className="hl-strong">{adjust.activitySuggestion.label}</span>. {adjust.activitySuggestion.reason}</p>
          <button type="button" className="hl-button" onClick={() => onActivity(Number(adjust.activitySuggestion?.value))}>Usar este nivel</button>
        </>
      )}
    </div>
  )
}

export function ObjectivePanelView({ panel, onObjective, onGeneratePlan, onCalculatePlan, onClearPlan, onActivity, onMeasuredExpenditure }: ObjectivePanelProps) {
  const [target, setTarget] = useState(panel.targetKg === null ? '' : String(panel.targetKg).replace('.', ','))
  const [busy, setBusy] = useState<'ai' | 'calculated' | null>(null)
  const [error, setError] = useState<string | null>(null)
  const saveTarget = () => {
    const value = toNumber(target)
    if (value !== panel.targetKg) onObjective(value, panel.pace)
  }
  const run = async (mode: 'ai' | 'calculated') => {
    setBusy(mode)
    setError(null)
    try {
      await (mode === 'ai' ? onGeneratePlan() : onCalculatePlan())
    } catch (reason) {
      setError(asHealthError(reason).message)
    } finally {
      setBusy(null)
    }
  }
  return (
    <section className="hl-panel hl-objective" aria-labelledby="hl-objective-title">
      <h2 id="hl-objective-title" className="hl-h2">Peso objetivo</h2>
      <div className="hl-grid-2">
        <label className="hl-field">
          <span>Objetivo (kg)</span>
          <input className="hl-input" inputMode="decimal" placeholder="76" value={target} onChange={(event) => setTarget(event.target.value)} onBlur={saveTarget} />
        </label>
        <label className="hl-field">
          <span>Ritmo por semana</span>
          <select className="hl-input" value={String(panel.pace)} onChange={(event) => onObjective(panel.targetKg, Number(event.target.value))}>
            {panel.paces.map((pace) => <option key={pace.value} value={pace.value}>{pace.label}</option>)}
          </select>
        </label>
      </div>
      {panel.differenceLabel && (
        <dl className="hl-well hl-pairs">
          <dt>Diferencia</dt><dd>{panel.differenceLabel}</dd>
          <dt>IMC con ese peso</dt>
          <dd>{panel.targetBmiLabel} {panel.targetState && <span className="hl-chip" data-tone={panel.targetState.tone}>{panel.targetState.label}</span>}</dd>
        </dl>
      )}
      {panel.lowBmiWarning && (
        <p className="hl-small hl-warn">Ese peso te deja con un IMC por debajo de 18,5, que se considera bajo peso. Conviene revisarlo con un profesional.</p>
      )}
      <div className="hl-actions">
        <button type="button" className="hl-button hl-button--primary" disabled={!panel.canPlan || busy !== null} onClick={() => { void run('ai') }}>
          {busy === 'ai' ? 'Generando plan…' : 'Generar plan con IA'}
        </button>
        <button type="button" className="hl-button" disabled={!panel.canPlan || busy !== null} onClick={() => { void run('calculated') }}>Calcular sin IA</button>
      </div>
      {error && <p className="hl-small hl-error" role="alert">{error}</p>}
      {panel.plan && (
        <div className="hl-plan">
          <div className="hl-plan__head">
            <span className="hl-mid">{panel.plan.kcalLabel} <span className="hl-small hl-muted">kcal por día</span></span>
            <span className="hl-small hl-faint">{panel.plan.sourceLabel}</span>
          </div>
          {panel.plan.weeksLabel && <p className="hl-small hl-muted">{panel.plan.weeksLabel}</p>}
          {panel.plan.summary && <p className="hl-small">{panel.plan.summary}</p>}
          {panel.plan.notes.map((note) => <p key={note} className="hl-small hl-warn">{note}</p>)}
          {panel.plan.recommendations.length > 0 && (
            <ul className="hl-small hl-muted hl-plan__tips">
              {panel.plan.recommendations.map((tip) => <li key={tip}>{tip}</li>)}
            </ul>
          )}
          <button type="button" className="hl-link" onClick={onClearPlan}>Quitar plan y volver a mantenimiento</button>
        </div>
      )}
      {panel.adjust && <AdjustBlock adjust={panel.adjust} onActivity={onActivity} onMeasuredExpenditure={onMeasuredExpenditure} />}
    </section>
  )
}

const RING_RADIUS = 54
const RING_LENGTH = 2 * Math.PI * RING_RADIUS

interface WaterPanelProps {
  panel: WaterPanel
  onAdd: (deltaMl: number) => void
}

export function WaterPanelView({ panel, onAdd }: WaterPanelProps) {
  const [other, setOther] = useState('')
  const addOther = () => {
    const ml = Number(other)
    if (!ml) return
    onAdd(ml)
    setOther('')
  }
  return (
    <section className="hl-panel hl-water" aria-labelledby="hl-water-title">
      <h2 id="hl-water-title" className="hl-h2">Agua de hoy</h2>
      <div className="hl-water__ring">
        <svg width="132" height="132" viewBox="0 0 132 132" role="img" aria-label={panel.ariaLabel}>
          <circle cx="66" cy="66" r={RING_RADIUS} className="hl-water__track" />
          <circle cx="66" cy="66" r={RING_RADIUS} className="hl-water__fill" strokeDasharray={RING_LENGTH} strokeDashoffset={RING_LENGTH * (1 - panel.ratio)} transform="rotate(-90 66 66)" />
          <text x="66" y="64" textAnchor="middle" className="hl-water__percent">{panel.percentLabel}</text>
          <text x="66" y="84" textAnchor="middle" className="hl-water__caption">del objetivo</text>
        </svg>
        <div>
          <div className="hl-mid">{panel.litersLabel}</div>
          <div className="hl-small hl-muted">{panel.goalLabel}</div>
          <div className="hl-small hl-faint">{panel.basisLabel}</div>
        </div>
      </div>
      <div className="hl-actions">
        <button type="button" className="hl-button hl-button--water" onClick={() => onAdd(250)}>+ Vaso 250 ml</button>
        <button type="button" className="hl-button hl-button--water" onClick={() => onAdd(500)}>+ Botella 500 ml</button>
      </div>
      <form className="hl-inline" onSubmit={(event) => { event.preventDefault(); addOther() }}>
        <label className="hl-visually-hidden" htmlFor="hl-water-other">Otra cantidad en mililitros</label>
        <input id="hl-water-other" className="hl-input" inputMode="numeric" placeholder="Otra cantidad (ml)" value={other} onChange={(event) => setOther(event.target.value.replace(/\D/g, ''))} />
        <button type="submit" className="hl-button" disabled={!other}>Sumar</button>
      </form>
      {panel.ml > 0 && <button type="button" className="hl-link" onClick={() => onAdd(-250)}>Restar 250 ml</button>}
      <div className="hl-water__week">
        <div className="hl-small hl-muted">Últimos 7 días</div>
        <div className="hl-water__bars">
          {panel.week.map((day) => (
            <div key={day.date} className="hl-water__day" title={day.title}>
              <div className="hl-water__bar"><div data-met={day.met} style={{ height: `${day.fill}%` }} /></div>
              <span className="hl-faint">{day.dayLabel}</span>
            </div>
          ))}
        </div>
      </div>
    </section>
  )
}
