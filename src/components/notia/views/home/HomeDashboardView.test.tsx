// @vitest-environment happy-dom
import { afterEach, describe, expect, it, vi } from 'vitest'
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import type { NotiaLibrary } from '../../../../types/notia'
import type { HomeDashboard } from '../../../../services/home/homeTypes'
import type { HomeWeather } from '../../../../services/home/weatherService'
import { HomeDashboardView } from './HomeDashboardView'

const mocks = vi.hoisted(() => ({
  callBackend: vi.fn(),
  requestChatPanel: vi.fn(),
  actions: { railActionClick: vi.fn(), openFile: vi.fn(), explorerToolClick: vi.fn(), toggleFolder: vi.fn() } as Record<string, unknown>,
  dashboard: null as unknown,
  weather: null as unknown,
}))

vi.mock('../../../../services/transport', () => ({
  callBackend: mocks.callBackend,
  subscribeBackend: vi.fn(async () => () => undefined),
  backendSupports: () => false,
  backendKind: () => 'local',
}))
vi.mock('../../../../services/chat/chatComposerRequests', () => ({ requestChatPanel: mocks.requestChatPanel }))
vi.mock('../../../../store/hooks', () => ({ useAppDispatch: () => vi.fn(), useAppSelector: () => [] }))
vi.mock('../../../../context/notiaActions/useNotiaAction', () => ({ useNotiaAction: (name: string) => mocks.actions[name] }))
// The space of a phone: happy-dom does not measure.
vi.mock('../../../../hooks/useNarrowContainer', () => ({ useNarrowContainer: () => true }))
vi.mock('./useHomeDashboard', () => ({ useHomeDashboard: () => ({ dashboard: mocks.dashboard, error: null, isLoading: false, reload: vi.fn(async () => undefined) }) }))
vi.mock('./useHomeWeather', () => ({ useHomeWeather: () => ({ weather: mocks.weather, error: null, isLoading: false, reload: vi.fn() }) }))

const library = { id: 'lib-1', name: 'gaia', path: 'C:/gaia' } as NotiaLibrary

const idleTimer = {
  phase: 'work', runState: 'idle', remainingSeconds: 1500, endTimestamp: null, completedWorkCycles: 0, selectedTaskPath: null,
  isDeviationActive: false, deviationStartedAt: null, deviationBaseRemainingSeconds: 0, phaseDeviationSeconds: 0,
  durations: { workMinutes: 25, shortBreakMinutes: 5, longBreakMinutes: 15 },
}

const forecastDay = (day: string) => ({ day, sky: 'sun' as const, rain: '', min: '11°', max: '21°', barLeft: 0, barWidth: 50, title: `${day}: soleado` })
mocks.weather = {
  place: 'Buenos Aires',
  now: { temp: '18°', condition: 'Parcialmente nublado', sky: 'partly', feels: '17°', humidity: '64 %', wind: '15 km/h SE', updated: '10:30', max: '21°', min: '11°' },
  next: [forecastDay('Dom'), forecastDay('Lun'), forecastDay('Mar')],
  hours: [{ label: 'Ahora', sky: 'partly', temp: '18°', rain: '—' }],
  days: [forecastDay('Hoy'), forecastDay('Dom')],
} satisfies HomeWeather

mocks.dashboard = {
  dateLabel: 'Sábado 26 de septiembre · gaia',
  todayLabel: 'Sábado 26 de septiembre',
  greeting: 'Buen día',
  summary: '1 evento en los próximos 7 días',
  agenda: { data: { days: [{ date: '2026-09-26', weekday: 'Sáb', day: 26, label: 'sábado 26', isToday: true, hasEvents: false }], events: [] } },
  tasks: { data: { completed: 17, sprint: 5, review: 2, blocked: 1, urgentCount: 0, urgent: [], focus: { filePath: 't/a.md', title: 'Performance', selected: false } } },
  finance: { data: { month: '2026-09', monthLabel: 'septiembre', expenses: [], expenseCount: 0, income: [], left: [], pending: [], pendingCount: 0, saved: [] } },
  routine: { data: { today: '2026-09-26', monthLabel: 'septiembre', monthPct: 75, weekPct: 76, bestStreak: 13, pendingToday: 0, week: [], routines: [] } },
  notes: { data: { items: [], pending: 0 } },
  recent: { items: [], folders: [{ name: 'Facultad', path: 'C:/gaia/Facultad' }] },
} satisfies HomeDashboard

describe('HomeDashboardView on a phone', () => {
  afterEach(() => {
    cleanup()
    vi.clearAllMocks()
  })

  const renderPhone = () => {
    mocks.callBackend.mockImplementation(async (command: string) => {
      if (command === 'chat_agents_catalog') return { dynamics: [], agents: [{ fileName: 'general.md', name: 'General', description: '', initials: 'G', valid: true }] }
      if (command === 'backend_agent_prompts') return { prompts: [], selected: 'general.md' }
      if (command === 'finance_dollar_quotes') return []
      if (command === 'task_manager_pomodoro') return { state: idleTimer, changed: false }
      return null
    })
    return render(<HomeDashboardView library={library} />)
  }

  it('follows the phone board: the day without the library and the cards in its order', () => {
    renderPhone()
    expect(document.querySelector('.home-view--phone')).toBeTruthy()
    expect(screen.getByText('Sábado 26 de septiembre')).toBeTruthy()
    expect(screen.queryByText(/· gaia/)).toBeNull()
    expect(screen.getAllByRole('heading', { level: 2 }).map((heading) => heading.textContent)).toEqual([
      'Agenda', 'Tareas', 'Rutinas', 'Anotador rápido', 'Finanzas', 'Seguir donde quedaste',
    ])
    expect(screen.getByRole('button', { name: 'Calendario' })).toBeTruthy()
    expect(screen.getByRole('button', { name: 'Task Manager' })).toBeTruthy()
    expect(screen.queryByText(/completadas/)).toBeNull()
    expect(screen.queryByRole('button', { name: 'Ver historial' })).toBeNull()
    expect(screen.getByText('75 %')).toBeTruthy()
    expect(screen.getByText('13 días')).toBeTruthy()
  })

  it('sends the question to the side chat and confirms it under the box', async () => {
    renderPhone()
    await waitFor(() => expect(screen.getByRole('button', { name: /Agente: General/ })).toBeTruthy())
    fireEvent.change(screen.getByPlaceholderText('Preguntale al asistente…'), { target: { value: '¿Qué tengo hoy?' } })
    fireEvent.click(screen.getByRole('button', { name: 'Enviar al asistente' }))
    expect(mocks.requestChatPanel).toHaveBeenCalledWith({ kind: 'send', text: '¿Qué tengo hoy?', agentFileName: 'general.md' })
    expect(screen.getByRole('status').textContent).toBe('Enviado al asistente · agente General')
  })

  it('creates a note from the phone button', () => {
    renderPhone()
    fireEvent.click(screen.getByRole('button', { name: 'Nueva nota' }))
    expect(mocks.actions.explorerToolClick).toHaveBeenCalledWith('new-note')
  })
})
