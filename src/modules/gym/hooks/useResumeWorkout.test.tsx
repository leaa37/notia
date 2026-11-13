// @vitest-environment happy-dom
import { afterEach, describe, expect, it, vi } from 'vitest'
import { act, cleanup, renderHook } from '@testing-library/react'
import type { ReactNode } from 'react'
import { Provider } from 'react-redux'
import { configureStore } from '@reduxjs/toolkit'
import documentsReducer, { GYM_WORKSPACE_TAB_PATH, HOME_WORKSPACE_TAB_PATH, setActiveTabPath } from '../../../features/documents/documentsSlice'
import type { NotiaLibrary } from '../../../types/notia'
import { useResumeWorkout } from './useResumeWorkout'

const service = vi.hoisted(() => ({ getResumableWorkout: vi.fn() }))
vi.mock('../services/gymService', () => service)

const library = { id: 'library-1' } as NotiaLibrary

function setup() {
  const store = configureStore({ reducer: { documents: documentsReducer } })
  const wrapper = ({ children }: { children: ReactNode }) => <Provider store={store}>{children}</Provider>
  return { store, wrapper }
}

describe('useResumeWorkout', () => {
  afterEach(() => {
    cleanup()
    vi.resetAllMocks()
  })

  it('opens Gimnasio when a workout is in progress', async () => {
    service.getResumableWorkout.mockResolvedValue('routine-1')
    const { store, wrapper } = setup()
    renderHook(() => useResumeWorkout(library), { wrapper })
    await act(async () => undefined)
    expect(store.getState().documents.activeTabPath).toBe(GYM_WORKSPACE_TAB_PATH)
    expect(store.getState().documents.specialTabs.gym).toBe(true)
  })

  it('stays on Inicio, where the app opens, without a workout', async () => {
    service.getResumableWorkout.mockResolvedValue(null)
    const { store, wrapper } = setup()
    renderHook(() => useResumeWorkout(library), { wrapper })
    await act(async () => undefined)
    expect(store.getState().documents.activeTabPath).toBe(HOME_WORKSPACE_TAB_PATH)
  })

  it('does not take over what the person already opened', async () => {
    let answer: (routineId: string) => void = () => undefined
    service.getResumableWorkout.mockReturnValue(new Promise<string>((resolve) => { answer = resolve }))
    const { store, wrapper } = setup()
    renderHook(() => useResumeWorkout(library), { wrapper })
    act(() => { store.dispatch(setActiveTabPath('Notas/idea.md')) })
    await act(async () => { answer('routine-1') })
    expect(store.getState().documents.activeTabPath).toBe('Notas/idea.md')
  })
})
