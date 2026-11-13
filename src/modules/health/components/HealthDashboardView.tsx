import { useState } from 'react'
import { X } from 'lucide-react'
import type { NotiaLibrary } from '../../../types/notia'
import { useNarrowContainer } from '../../../hooks/useNarrowContainer'
import { useHealthDashboard } from '../hooks/useHealthDashboard'
import { asHealthError, estimateMeal } from '../services/healthService'
import type { HealthMutation, WeightRange } from '../types/healthTypes'
import { CompositionPanelView, FoodPanelView } from './FoodAndBodyPanels'
import { MealFormView, MeasurementFormView, ProfileFormView, type MealTarget } from './HealthForms'
import { BmiPanelView, DailyPanelView, ObjectivePanelView, WaterPanelView, WeightPanelView } from './HealthPanels'
import { SLEEP_COPY, STEPS_COPY } from './healthUi'
import { HEALTH_PHONE_MAX_WIDTH, HealthPhone, type HealthPanels } from './HealthPhone'
import '../styles/health.css'

type Modal = { kind: 'profile' } | { kind: 'measurement' } | { kind: 'meal'; target: MealTarget }

export function HealthDashboardView({ library }: { library: NotiaLibrary }) {
  const { dashboard, status, loadError, updateQuery, reload, apply, generatePlan } = useHealthDashboard(library)
  const [root, setRoot] = useState<HTMLElement | null>(null)
  const phone = useNarrowContainer(root, HEALTH_PHONE_MAX_WIDTH)
  const [modal, setModal] = useState<Modal | null>(null)
  const [actionError, setActionError] = useState<string | null>(null)

  /** Un cambio de un panel: el error queda arriba, a la vista. */
  const run = async (mutation: HealthMutation): Promise<boolean> => {
    setActionError(null)
    try {
      await apply(mutation)
      return true
    } catch (reason) {
      setActionError(asHealthError(reason).message)
      return false
    }
  }

  if (!dashboard) {
    return (
      <main ref={setRoot} className="notia-main health-view">
        <div className="hl-page">
          {status === 'error' ? (
            <div className="hl-state" role="alert">
              <p>{loadError}</p>
              <button type="button" className="hl-button" onClick={() => { void reload() }}>Reintentar</button>
            </div>
          ) : (
            <p className="hl-state" role="status">Cargando…</p>
          )}
        </div>
      </main>
    )
  }

  const today = dashboard.today
  const openProfile = () => setModal({ kind: 'profile' })
  const openMeasurement = () => setModal({ kind: 'measurement' })
  const addMeal = (date: string, category: string | null) => setModal({ kind: 'meal', target: { id: null, date, category, values: null } })

  // Los paneles son los mismos en el escritorio y en el celular.
  const panels: HealthPanels = {
    welcome: (
      <div className="hl-panel hl-welcome">
        <h2 className="hl-mid">Empezá por tu perfil</h2>
        <p className="hl-muted">
          Con tu fecha de nacimiento, altura y peso se calculan tus calorías diarias, el agua que necesitás y tu IMC. Después podés sumar las mediciones de tu balanza.
        </p>
        <div className="hl-actions">
          <button type="button" className="hl-button hl-button--primary" onClick={openProfile}>Configurar perfil</button>
        </div>
      </div>
    ),
    alert: actionError && (
      <div className="hl-alert hl-alert--page" role="alert">
        <span>{actionError}</span>
        <button type="button" className="hl-icon-button hl-icon-button--small" aria-label="Cerrar aviso" onClick={() => setActionError(null)}><X size={16} /></button>
      </div>
    ),
    bmi: dashboard.bmi && <BmiPanelView panel={dashboard.bmi} />,
    weight: (
      <WeightPanelView
        panel={dashboard.weight}
        today={today}
        onRange={(range) => updateQuery({ weightRange: range as WeightRange })}
        onAdd={(date, kg) => run({ kind: 'addWeight', date, kg })}
        onDelete={(date) => { void run({ kind: 'deleteWeight', date }) }}
      />
    ),
    objective: (
      <ObjectivePanelView
        key={`${dashboard.objective.targetKg ?? ''}-${dashboard.objective.pace}`}
        panel={dashboard.objective}
        onObjective={(targetKg, pace) => { void run({ kind: 'setObjective', targetKg, pace }) }}
        onGeneratePlan={generatePlan}
        onCalculatePlan={() => apply({ kind: 'calculatePlan' })}
        onClearPlan={() => { void run({ kind: 'clearPlan' }) }}
        onActivity={(activity) => { void run({ kind: 'setActivity', activity }) }}
        onMeasuredExpenditure={(enabled) => { void run({ kind: 'setMeasuredExpenditure', enabled }) }}
      />
    ),
    food: dashboard.food && (
      <FoodPanelView
        panel={dashboard.food}
        phone={phone}
        onDate={(date) => updateQuery({ foodDate: date })}
        onAdd={(category) => addMeal(dashboard.food?.date ?? today, category)}
        onEdit={(meal) => setModal({ kind: 'meal', target: { id: meal.id, date: dashboard.food?.date ?? today, category: null, values: meal.values } })}
      />
    ),
    water: dashboard.water && <WaterPanelView panel={dashboard.water} onAdd={(deltaMl) => { void run({ kind: 'addWater', date: today, deltaMl }) }} />,
    steps: (
      <DailyPanelView
        panel={dashboard.steps}
        copy={STEPS_COPY}
        today={today}
        onRange={(range) => updateQuery({ stepsRange: range as WeightRange })}
        onAdd={(date, steps) => run({ kind: 'setSteps', date, steps })}
        onDelete={(date) => { void run({ kind: 'deleteSteps', date }) }}
      />
    ),
    sleep: (
      <DailyPanelView
        panel={dashboard.sleep}
        copy={SLEEP_COPY}
        today={today}
        onRange={(range) => updateQuery({ sleepRange: range as WeightRange })}
        onAdd={(date, duration) => run({ kind: 'setSleep', date, duration })}
        onDelete={(date) => { void run({ kind: 'deleteSleep', date }) }}
      />
    ),
    composition: (
      <CompositionPanelView
        panel={dashboard.composition}
        onNew={openMeasurement}
        onDate={(date) => updateQuery({ measurementDate: date })}
        onDelete={(date) => { updateQuery({ measurementDate: null }); void run({ kind: 'deleteMeasurement', date }) }}
      />
    ),
  }

  return (
    <main ref={setRoot} className={`notia-main health-view${phone ? ' health-view--phone' : ''}`}>
      {phone ? (
        <HealthPhone dashboard={dashboard} panels={panels} onProfile={openProfile} onMeasurement={openMeasurement} onMeal={(date) => addMeal(date, null)} />
      ) : (
        <div className="hl-page">
          <header className="hl-top">
            <div>
              <h1 className="hl-h1">Salud</h1>
              {dashboard.header.chips.length > 0 && (
                <div className="hl-top__chips">
                  {dashboard.header.chips.map((chip) => <span key={chip} className="hl-well hl-top__chip">{chip}</span>)}
                  {dashboard.header.bmi && <span className="hl-top__chip hl-top__chip--tone" data-tone={dashboard.header.bmi.tone}>{dashboard.header.bmi.label}</span>}
                </div>
              )}
            </div>
            <div className="hl-actions">
              <button type="button" className="hl-button" onClick={openProfile}>{dashboard.hasProfile ? 'Editar perfil' : 'Configurar perfil'}</button>
              {dashboard.hasProfile && <button type="button" className="hl-button hl-button--primary" onClick={openMeasurement}>Registrar medición</button>}
            </div>
          </header>

          {panels.alert}

          {!dashboard.hasProfile ? panels.welcome : (
            <div className="hl-grid">
              {panels.bmi && <div className="hl-grid__full">{panels.bmi}</div>}
              <div className="hl-grid__wide">{panels.weight}</div>
              {panels.objective}
              {panels.food && <div className="hl-grid__wide">{panels.food}</div>}
              {panels.water && <div className="hl-grid__side">{panels.water}</div>}
              <div className="hl-grid__full hl-daily-pair">{panels.steps}{panels.sleep}</div>
              <div className="hl-grid__full">{panels.composition}</div>
            </div>
          )}
        </div>
      )}

      {modal?.kind === 'profile' && (
        <ProfileFormView dashboard={dashboard} phone={phone} onClose={() => setModal(null)} onSave={(input) => apply({ kind: 'saveProfile', input })} />
      )}
      {modal?.kind === 'measurement' && (
        <MeasurementFormView dashboard={dashboard} phone={phone} onClose={() => setModal(null)} onSave={(input) => apply({ kind: 'saveMeasurement', input })} />
      )}
      {modal?.kind === 'meal' && (
        <MealFormView
          dashboard={dashboard}
          target={modal.target}
          phone={phone}
          onClose={() => setModal(null)}
          onSave={(id, input) => apply({ kind: 'saveMeal', id, input })}
          onDelete={(id) => apply({ kind: 'deleteMeal', id })}
          onEstimate={(description) => estimateMeal(library, description)}
        />
      )}
    </main>
  )
}
