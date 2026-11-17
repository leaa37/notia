// @vitest-environment happy-dom
import { afterEach, describe, expect, it, vi } from 'vitest'
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react'
import type { GymMutation, TrainingView } from '../types/gymTypes'
import { TrainingScreen } from './TrainingScreen'

const confirmation = vi.hoisted(() => ({ confirm: vi.fn() }))

vi.mock('../../../context/confirmation/useConfirmationEngine', () => ({
  useConfirmationEngine: () => confirmation,
}))

function training(status: TrainingView['status'], oneSide = false): TrainingView {
  return {
    routineId: 'r1',
    name: 'Pecho',
    focusLabel: 'Sin enfoque',
    status,
    startedMs: 0,
    accMs: 0,
    restEndMs: null,
    restLeftMs: null,
    restTotalMs: 0,
    doneSets: 1,
    totalSets: 3,
    percent: 33,
    volume: '600 kg',
    kcalPerMin: 5,
    nextText: 'serie 2 de Press',
    currentRest: null,
    doneText: 'Terminaste.',
    items: [{
      id: 'i1',
      n: 1,
      exerciseId: 'press',
      name: 'Press',
      group: 'pecho',
      groupLabel: 'Pecho',
      restLabel: '1:30',
      missing: null,
      weighted: true,
      timed: false,
      unilateral: oneSide,
      count: '1/3',
      allDone: false,
      current: true,
      sets: [
        { weight: '60', reps: '10', done: true, oneSide: false, next: false },
        { weight: '60', reps: '10', done: false, oneSide, next: true },
        { weight: '60', reps: '10', done: false, oneSide: false, next: false },
      ],
      sideText: oneSide ? 'Falta el otro lado de la serie 2 de 3: después del descanso, hacelo y tocá la serie de nuevo.' : null,
    }],
  }
}

function renderScreen(status: TrainingView['status'], applying = false, oneSide = false) {
  const apply = vi.fn<(mutation: GymMutation) => void>()
  render(
    <TrainingScreen training={training(status, oneSide)} clockOffsetMs={0} apply={apply} applying={applying} onBack={() => undefined} onOpenExercise={() => undefined} />,
  )
  return apply
}

describe('TrainingScreen', () => {
  afterEach(() => {
    cleanup()
    confirmation.confirm.mockReset()
  })

  it('asks before finishing the workout', async () => {
    confirmation.confirm.mockResolvedValueOnce(false).mockResolvedValueOnce(true)
    const apply = renderScreen('running')
    await act(async () => { fireEvent.click(screen.getByRole('button', { name: 'Terminar' })) })
    expect(confirmation.confirm).toHaveBeenCalledWith(expect.objectContaining({ message: expect.stringContaining('1 de 3 series') }))
    expect(apply).not.toHaveBeenCalled()
    await act(async () => { fireEvent.click(screen.getByRole('button', { name: 'Terminar' })) })
    expect(apply).toHaveBeenCalledWith({ type: 'finish-session' })
  })

  it('ignores the session controls while a change is waiting for Rust', () => {
    const apply = renderScreen('running', true)
    fireEvent.click(screen.getByRole('button', { name: /Pausar/ }))
    fireEvent.click(screen.getByRole('button', { name: 'Terminar' }))
    expect(apply).not.toHaveBeenCalled()
    expect(confirmation.confirm).not.toHaveBeenCalled()
    // The sets keep taking taps.
    fireEvent.click(screen.getByRole('button', { name: 'Marcar como hecha la serie 2' }))
    expect(apply).toHaveBeenCalledWith({ type: 'toggle-set-done', routineId: 'r1', itemId: 'i1', index: 1 })
  })

  it('asks for the other side of a set done with one side', () => {
    const apply = renderScreen('running', false, true)
    expect(screen.getAllByText('Falta el otro lado de la serie 2 de 3: después del descanso, hacelo y tocá la serie de nuevo.').length).toBeGreaterThan(0)
    expect(screen.getByText('Por lado')).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: 'Marcar el otro lado de la serie 2' }))
    expect(apply).toHaveBeenCalledWith({ type: 'toggle-set-done', routineId: 'r1', itemId: 'i1', index: 1 })
    expect(screen.getByRole('button', { name: 'Marcar el primer lado de la serie 3' })).toBeTruthy()
  })

  it('does not take set taps once the workout is finished', () => {
    const apply = renderScreen('done')
    const set = screen.getByRole('button', { name: 'Marcar como hecha la serie 2' })
    expect(set).toHaveProperty('disabled', true)
    fireEvent.click(set)
    expect(apply).not.toHaveBeenCalled()
  })
})
