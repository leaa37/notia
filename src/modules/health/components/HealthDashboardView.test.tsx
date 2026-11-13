// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { act, cleanup, fireEvent, render, screen, within } from '@testing-library/react'
import type { HealthDashboard } from '../types/healthTypes'
import { HealthDashboardView } from './HealthDashboardView'
import fullFixture from './__fixtures__/dashboard.json'
import emptyFixture from './__fixtures__/dashboard-empty.json'

const callBackend = vi.fn()
vi.mock('../../../services/transport', () => ({
  callBackend: (...args: unknown[]) => callBackend(...args),
  subscribeBackend: () => Promise.resolve(() => undefined),
}))

// Tableros armados por Rust (`build_dashboard`) con el escenario «Sobrepeso».
const FULL = fullFixture as unknown as HealthDashboard
const EMPTY = emptyFixture as unknown as HealthDashboard
const LIBRARY = { id: 'lib-1', name: 'gaia', path: 'C:/gaia' }
const CONTEXT = { libraryId: 'lib-1', actorLibraryUserId: 'user-owner' }

function serve(dashboard: HealthDashboard) {
  callBackend.mockImplementation((command: string) => {
    if (command === 'health_estimate_meal') return Promise.resolve({ kcal: 290, proteinG: 5, carbsG: 36, fatG: 14, fiberG: 1 })
    return Promise.resolve(dashboard)
  })
}

beforeEach(() => {
  callBackend.mockReset()
  serve(FULL)
})

afterEach(() => cleanup())

describe('HealthDashboardView', () => {
  it('shows every panel as Rust computed it and sends the quick actions', async () => {
    render(<HealthDashboardView library={LIBRARY} />)
    expect(await screen.findByText('Estás 3,5 kg por encima del rango normal para tu altura.')).toBeTruthy()
    expect(screen.getByText('IMC 26, sobrepeso')).toBeTruthy()
    expect(screen.getByText('-3,6 kg en el período. Te faltan 6,4 kg para tu objetivo.')).toBeTruthy()
    expect(screen.getByText('de 1.886 kcal')).toBeTruthy()
    expect(screen.getByRole('img', { name: '1250 de 3050 ml' })).toBeTruthy()

    fireEvent.click(screen.getByRole('button', { name: '+ Vaso 250 ml' }))
    await vi.waitFor(() => expect(callBackend).toHaveBeenCalledWith('health_apply', expect.objectContaining({
      context: CONTEXT,
      mutation: { kind: 'addWater', date: '2026-09-28', deltaMl: 250 },
    })))

    fireEvent.click(within(screen.getByRole('group', { name: 'Período del gráfico' })).getByRole('button', { name: '30 días' }))
    await vi.waitFor(() => expect(callBackend).toHaveBeenCalledWith('health_dashboard', {
      context: CONTEXT,
      query: { foodDate: null, weightRange: '30', measurementDate: null, stepsRange: '30', sleepRange: '30' },
    }))

    fireEvent.change(screen.getByLabelText('Peso (kg)'), { target: { value: '82,1' } })
    await act(async () => { fireEvent.click(screen.getByRole('button', { name: 'Agregar peso' })) })
    await vi.waitFor(() => expect(callBackend).toHaveBeenCalledWith('health_apply', expect.objectContaining({ mutation: { kind: 'addWeight', date: '2026-09-28', kg: 82.1 } })))

    fireEvent.click(screen.getByRole('button', { name: 'Día anterior' }))
    await vi.waitFor(() => expect(callBackend).toHaveBeenCalledWith('health_dashboard', expect.objectContaining({ query: expect.objectContaining({ foodDate: '2026-09-27' }) })))

    await act(async () => { fireEvent.click(screen.getByRole('button', { name: 'Generar plan con IA' })) })
    await vi.waitFor(() => expect(callBackend).toHaveBeenCalledWith('health_generate_plan', expect.objectContaining({ context: CONTEXT })))
  })

  it('shows steps and sleep with their history and sends what was typed to Rust', async () => {
    render(<HealthDashboardView library={LIBRARY} />)
    const steps = await screen.findByRole('region', { name: 'Pasos' })
    expect(within(steps).getByText('4.310')).toBeTruthy()
    expect(within(steps).getByText(/Promedio de la semana: 7\.890 pasos/)).toBeTruthy()
    const sleep = screen.getByRole('region', { name: 'Sueño' })
    expect(within(sleep).getByText('6 h 30 min')).toBeTruthy()
    expect(within(sleep).getByText('Anoche')).toBeTruthy()

    fireEvent.change(within(steps).getByLabelText('Pasos'), { target: { value: '9.120' } })
    await act(async () => { fireEvent.click(within(steps).getByRole('button', { name: 'Agregar pasos' })) })
    expect(callBackend).toHaveBeenCalledWith('health_apply', expect.objectContaining({ mutation: { kind: 'setSteps', date: '2026-09-28', steps: '9.120' } }))

    fireEvent.change(within(sleep).getByLabelText('Horas'), { target: { value: '7:30' } })
    await act(async () => { fireEvent.click(within(sleep).getByRole('button', { name: 'Agregar sueño' })) })
    expect(callBackend).toHaveBeenCalledWith('health_apply', expect.objectContaining({ mutation: { kind: 'setSleep', date: '2026-09-28', duration: '7:30' } }))

    fireEvent.click(within(sleep).getByRole('button', { name: /Ver registros/ }))
    await act(async () => { fireEvent.click(within(sleep).getByRole('button', { name: 'Eliminar el sueño del 27 sept 2026' })) })
    expect(callBackend).toHaveBeenCalledWith('health_apply', expect.objectContaining({ mutation: { kind: 'deleteSleep', date: '2026-09-27' } }))

    fireEvent.click(within(within(sleep).getByRole('group')).getByRole('button', { name: '90 días' }))
    await vi.waitFor(() => expect(callBackend).toHaveBeenCalledWith('health_dashboard', expect.objectContaining({ query: expect.objectContaining({ sleepRange: '90', stepsRange: '30' }) })))
  })

  it('logs a meal with the AI estimate and shows what Rust rejects', async () => {
    render(<HealthDashboardView library={LIBRARY} />)
    fireEvent.click(await screen.findByRole('button', { name: 'Agregar a Merienda' }))
    const dialog = await screen.findByRole('dialog')
    expect(within(dialog).getByRole('radio', { name: 'Merienda' }).getAttribute('aria-checked')).toBe('true')
    fireEvent.change(within(dialog).getByLabelText('Qué comiste'), { target: { value: 'Mate con 2 medialunas' } })
    await act(async () => { fireEvent.click(within(dialog).getByRole('button', { name: /Estimar con IA/ })) })
    expect(callBackend).toHaveBeenCalledWith('health_estimate_meal', { context: CONTEXT, description: 'Mate con 2 medialunas' })
    expect((within(dialog).getByLabelText('Calorías') as HTMLInputElement).value).toBe('290')

    callBackend.mockImplementation((command: string) => command === 'health_apply'
      ? Promise.reject({ code: 'validation', message: 'La fecha no puede ser futura.', fields: [{ field: 'date', message: 'La fecha no puede ser futura.' }] })
      : Promise.resolve(FULL))
    await act(async () => { fireEvent.click(within(dialog).getByRole('button', { name: 'Agregar comida' })) })
    expect(callBackend).toHaveBeenCalledWith('health_apply', expect.objectContaining({
      mutation: { kind: 'saveMeal', id: null, input: { date: '2026-09-28', category: 'merienda', name: 'Mate con 2 medialunas', kcal: 290, proteinG: 5, carbsG: 36, fatG: 14, fiberG: 1 } },
    }))
    expect(await within(dialog).findByRole('alert')).toBeTruthy()

    // Editar una comida guardada trae sus valores.
    fireEvent.click(within(dialog).getByRole('button', { name: 'Cancelar' }))
    serve(FULL)
    fireEvent.click(screen.getByRole('button', { name: 'Editar Milanesa de pollo con ensalada' }))
    const edit = await screen.findByRole('dialog')
    expect((within(edit).getByLabelText('Calorías') as HTMLInputElement).value).toBe('520')
    await act(async () => { fireEvent.click(within(edit).getByRole('button', { name: 'Eliminar' })) })
    expect(callBackend).toHaveBeenCalledWith('health_apply', expect.objectContaining({ mutation: { kind: 'deleteMeal', id: 'm2' } }))
  })

  it('starts with the profile when there is none', async () => {
    serve(EMPTY)
    render(<HealthDashboardView library={LIBRARY} />)
    expect(await screen.findByText('Empezá por tu perfil')).toBeTruthy()
    expect(screen.queryByRole('button', { name: 'Registrar medición' })).toBeNull()
    fireEvent.click(screen.getAllByRole('button', { name: 'Configurar perfil' })[1])
    const dialog = await screen.findByRole('dialog')
    fireEvent.change(within(dialog).getByLabelText('Fecha de nacimiento'), { target: { value: '1992-03-15' } })
    fireEvent.change(within(dialog).getByLabelText('Altura (cm)'), { target: { value: '178' } })
    fireEvent.change(within(dialog).getByLabelText('Peso actual (kg)'), { target: { value: '82,4' } })
    callBackend.mockImplementationOnce(() => Promise.reject({ code: 'validation', message: 'La altura tiene que estar entre 100 y 250 cm.', fields: [{ field: 'heightCm', message: 'La altura tiene que estar entre 100 y 250 cm.' }] }))
    await act(async () => { fireEvent.click(within(dialog).getByRole('button', { name: 'Guardar perfil' })) })
    expect(callBackend).toHaveBeenCalledWith('health_apply', expect.objectContaining({
      mutation: { kind: 'saveProfile', input: { birthDate: '1992-03-15', sex: 'M', heightCm: 178, activity: 1.375, weightKg: 82.4, waterGoalLiters: null } },
    }))
    expect(await within(dialog).findByText('La altura tiene que estar entre 100 y 250 cm.')).toBeTruthy()
  })

  it('shows the sleeping pulse and offers the level and the expenditure the data show', async () => {
    serve({
      ...FULL,
      objective: {
        ...FULL.objective,
        adjust: {
          activityLabel: 'Ligero (1 a 3 días por semana)',
          activitySuggestion: { value: '1.55', label: 'Moderado (3 a 5 días por semana)', reason: 'Promedio de 9.000 pasos por día.' },
          formulaLabel: '2.436 kcal',
          measuredLabel: '2.680 kcal',
          measuredDetail: 'Comiste 2.400 kcal por día en promedio.',
          usingMeasured: false,
          canUseMeasured: true,
        },
      },
      sleep: { ...FULL.sleep, heart: { label: 'Pulso dormido 56 lpm', detail: '6 lpm más que tu promedio de 2 semanas (50).', tone: 'warn' } },
    })
    render(<HealthDashboardView library={LIBRARY} />)
    expect(await screen.findByText('Pulso dormido 56 lpm.')).toBeTruthy()
    expect(screen.getByText('2.680 kcal')).toBeTruthy()
    await act(async () => { fireEvent.click(screen.getByRole('button', { name: 'Usar este nivel' })) })
    expect(callBackend).toHaveBeenCalledWith('health_apply', expect.objectContaining({ mutation: { kind: 'setActivity', activity: 1.55 } }))
    await act(async () => { fireEvent.click(screen.getByLabelText(/Usar el gasto medido/)) })
    expect(callBackend).toHaveBeenCalledWith('health_apply', expect.objectContaining({ mutation: { kind: 'setMeasuredExpenditure', enabled: true } }))
  })

  it('sends the water goal written in the profile, in liters', async () => {
    serve(FULL)
    render(<HealthDashboardView library={LIBRARY} />)
    fireEvent.click(await screen.findByRole('button', { name: 'Editar perfil' }))
    const dialog = await screen.findByRole('dialog')
    const goal = within(dialog).getByLabelText(/^Meta de agua \(litros\)/) as HTMLInputElement
    expect(goal.value).toBe('')
    expect(goal.placeholder).toBe('Calculada: 3,05 L')
    fireEvent.change(goal, { target: { value: '2,5' } })
    await act(async () => { fireEvent.click(within(dialog).getByRole('button', { name: 'Guardar perfil' })) })
    expect(callBackend).toHaveBeenCalledWith('health_apply', expect.objectContaining({
      mutation: { kind: 'saveProfile', input: expect.objectContaining({ waterGoalLiters: 2.5 }) },
    }))
  })
})
