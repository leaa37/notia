import { describe, expect, it } from 'vitest'
import documentsReducer, { HOME_WORKSPACE_TAB_PATH, resetTabs, setActiveTabPath } from './documentsSlice'
import uiReducer from '../ui/uiSlice'

describe('the workspace at start', () => {
  it('opens on Inicio, also after the tabs of a library are reset', () => {
    const initial = documentsReducer(undefined, { type: 'init' })
    expect(initial.activeTabPath).toBe(HOME_WORKSPACE_TAB_PATH)
    expect(initial.specialTabs.home).toBe(true)

    const elsewhere = documentsReducer(initial, setActiveTabPath('Notas/idea.md'))
    const reset = documentsReducer(elsewhere, resetTabs())
    expect(reset.activeTabPath).toBe(HOME_WORKSPACE_TAB_PATH)
    expect(reset.specialTabs.home).toBe(true)
    expect(reset.openTabs).toEqual([])
  })

  it('starts with the file explorer hidden', () => {
    expect(uiReducer(undefined, { type: 'init' }).isSidebarOpen).toBe(false)
  })
})
