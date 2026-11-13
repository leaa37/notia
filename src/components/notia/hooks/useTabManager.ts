import { useCallback } from 'react'
import { useAppDispatch } from '../../../store/hooks'
import { store } from '../../../store/index'
import { selectActiveTabPath } from '../../../features/documents/documentsSelectors'
import {
  setOpenTabs,
  addOpenTab,
  updateTabSource,
  updateTabSaveStatus,
  updateTabSavedSource,
  renameTabPath,
  setActiveTabPath,
  setSpecialTabs,
  setDialogState,
  resetTabs as resetTabsAction,
  GRAPH_WORKSPACE_TAB_PATH,
  CHAT_WORKSPACE_TAB_PATH,
  TASK_MANAGER_WORKSPACE_TAB_PATH,
  COLDPASS_WORKSPACE_TAB_PATH,
  MEETING_WORKSPACE_TAB_PATH,
  FINANCE_WORKSPACE_TAB_PATH,
  AGENDA_WORKSPACE_TAB_PATH,
  ROUTINE_WORKSPACE_TAB_PATH,
  AI_ACTIONS_WORKSPACE_TAB_PATH,
  AI_TASKS_WORKSPACE_TAB_PATH,
  RECIPES_WORKSPACE_TAB_PATH,
  HEALTH_WORKSPACE_TAB_PATH,
  GYM_WORKSPACE_TAB_PATH,
  HOME_WORKSPACE_TAB_PATH,
} from '../../../features/documents/documentsSlice'
import { writeLibraryDocument } from '../../../services/libraries/libraryDocumentRuntime'
import {
  isTextFileDocument,
  type OpenFileDocument,
  type NotiaDocumentSaveStatus,
} from '../../../types/views/fileDocument'
import { getDirtyOpenTextDocumentPaths } from '../../../engines/documents/documentSaveEngine'
import { flushPendingEditorChanges } from '../../../services/markdown/pendingEditorChanges'

interface OpenDocumentTab {
  document: OpenFileDocument
  saveStatus: NotiaDocumentSaveStatus
  latestSavedSource: string
}

interface WorkspaceTitleTab {
  path: string
  title: string
}

interface OpenWorkspaceSpecialTabs {
  graph: boolean
  chat: boolean
  taskManager: boolean
  coldPass: boolean
  meeting: boolean
  finance: boolean
  agenda: boolean
  routine: boolean
  aiActions: boolean
  aiTasks: boolean
  recipes: boolean
  health: boolean
  gym: boolean
  home: boolean
}

function getDisplayBaseName(value: string): string {
  const trimmed = value.trim()
  if (!trimmed) { return value }

  const stripQueryAndHash = (input: string): string => input.replace(/[?#].*$/, '')
  const decodePath = (input: string): string => {
    try { return decodeURIComponent(input) } catch { return input }
  }

  let normalized = stripQueryAndHash(trimmed).replace(/\\/g, '/')
  if (/^[a-zA-Z][a-zA-Z0-9+.-]*:\/\//.test(trimmed)) {
    try {
      const parsed = new URL(trimmed)
      normalized = decodePath(`${parsed.hostname}${parsed.pathname}`).replace(/\\/g, '/')
    } catch {
      normalized = decodePath(normalized)
    }
  } else {
    normalized = decodePath(normalized)
  }

  let leaf = normalized.split('/').filter(Boolean).pop() ?? normalized
  if (!leaf) { leaf = trimmed }

  const lastColonIndex = leaf.lastIndexOf(':')
  if (lastColonIndex > 0) { leaf = leaf.slice(lastColonIndex + 1) }

  const lastDotIndex = leaf.lastIndexOf('.')
  if (lastDotIndex > 0) { return leaf.slice(0, lastDotIndex) }
  return leaf
}

function resolveWorkspaceTabTitle(tab: OpenDocumentTab): string {
  return getDisplayBaseName(tab.document.name || tab.document.path)
}

export function buildWorkspaceTitleTabs(
  documentTabs: OpenDocumentTab[],
  specialTabs: OpenWorkspaceSpecialTabs,
): WorkspaceTitleTab[] {
  const tabs: WorkspaceTitleTab[] = documentTabs.map((tab) => ({
    path: tab.document.path,
    title: resolveWorkspaceTabTitle(tab),
  }))

  if (specialTabs.graph) { tabs.push({ path: GRAPH_WORKSPACE_TAB_PATH, title: 'Graph view' }) }
  if (specialTabs.chat) { tabs.push({ path: CHAT_WORKSPACE_TAB_PATH, title: 'Chat' }) }
  if (specialTabs.taskManager) { tabs.push({ path: TASK_MANAGER_WORKSPACE_TAB_PATH, title: 'Task manager' }) }
  if (specialTabs.coldPass) { tabs.push({ path: COLDPASS_WORKSPACE_TAB_PATH, title: 'ColdPass' }) }
  if (specialTabs.meeting) { tabs.push({ path: MEETING_WORKSPACE_TAB_PATH, title: 'Meeting' }) }
  if (specialTabs.finance) { tabs.push({ path: FINANCE_WORKSPACE_TAB_PATH, title: 'Finanzas' }) }
  if (specialTabs.agenda) { tabs.push({ path: AGENDA_WORKSPACE_TAB_PATH, title: 'Agenda' }) }
  if (specialTabs.routine) { tabs.push({ path: ROUTINE_WORKSPACE_TAB_PATH, title: 'Rutina' }) }
  if (specialTabs.aiActions) { tabs.push({ path: AI_ACTIONS_WORKSPACE_TAB_PATH, title: 'Acciones IA' }) }
  if (specialTabs.aiTasks) { tabs.push({ path: AI_TASKS_WORKSPACE_TAB_PATH, title: 'Tareas IA' }) }
  if (specialTabs.recipes) { tabs.push({ path: RECIPES_WORKSPACE_TAB_PATH, title: 'Recetas' }) }
  if (specialTabs.health) { tabs.push({ path: HEALTH_WORKSPACE_TAB_PATH, title: 'Salud' }) }
  if (specialTabs.gym) { tabs.push({ path: GYM_WORKSPACE_TAB_PATH, title: 'Gimnasio' }) }
  if (specialTabs.home) { tabs.push({ path: HOME_WORKSPACE_TAB_PATH, title: 'Inicio' }) }

  return tabs
}

export function isSameOrNestedPath(basePath: string, candidatePath: string): boolean {
  const normalizedBase = basePath.replace(/\\/g, '/').replace(/\/+$/, '')
  const normalizedCandidate = candidatePath.replace(/\\/g, '/').replace(/\/+$/, '')
  if (normalizedBase === normalizedCandidate) { return true }
  return normalizedCandidate.startsWith(`${normalizedBase}/`)
}

export function getParentDirectory(filePath: string): string {
  const lastForwardSlash = filePath.lastIndexOf('/')
  const lastBackwardSlash = filePath.lastIndexOf('\\')
  const separatorIndex = Math.max(lastForwardSlash, lastBackwardSlash)
  if (separatorIndex < 0) { return filePath }
  if (separatorIndex === 0) { return filePath.slice(0, 1) }
  if (separatorIndex === 2 && /^[a-zA-Z]:[\\/]/.test(filePath)) { return filePath.slice(0, 3) }
  return filePath.slice(0, separatorIndex)
}

export function joinParentPath(parentPath: string, originalPath: string, name: string): string {
  const separator = originalPath.includes('\\') ? '\\' : '/'
  if (parentPath.endsWith('/') || parentPath.endsWith('\\')) {
    return `${parentPath}${name}`
  }
  return `${parentPath}${separator}${name}`
}

interface UseTabManagerParams {
  clearPendingTextSaveByPath: (path: string) => void
  bumpLibraryIndexRevision: () => void
  resetColdPassSession: () => void
  activeLibraryId: string | null
  /** Shows in the open editor the text the backend wrote after merging. */
  onMergedSource: (path: string, source: string, revision?: string) => void
  /** Whether to close a note whose changes could not be saved. */
  confirmDiscard: (name: string, reason: string) => Promise<boolean>
}

export function useTabManager({
  clearPendingTextSaveByPath,
  bumpLibraryIndexRevision,
  resetColdPassSession,
  activeLibraryId,
  onMergedSource,
  confirmDiscard,
}: UseTabManagerParams) {
  const dispatch = useAppDispatch()

  const persistTextDocumentSource = useCallback(
    async (targetPath: string, targetSource: string): Promise<boolean> => {
      const currentTab = store.getState().documents.openTabs.find((tab) => tab.document.path === targetPath)
      if (!currentTab || !isTextFileDocument(currentTab.document) || currentTab.document.source !== targetSource) {
        return true
      }
      if (!activeLibraryId) {
        return false
      }
      dispatch(updateTabSaveStatus({ path: targetPath, status: 'saving' }))
      const result = await writeLibraryDocument(activeLibraryId, targetPath, targetSource, {
        // Only the version the editor loaded or saved is overwritten; if the
        // file changed since, the backend merges both from that text.
        ...(currentTab.latestSavedRevision
          ? { expectedRevision: currentTab.latestSavedRevision, baseContent: currentTab.latestSavedSource }
          : {}),
      })
      // Keystrokes still in the editor count before deciding what it shows.
      if (result.ok && result.content !== undefined) flushPendingEditorChanges()
      const latestTab = store.getState().documents.openTabs.find((tab) => tab.document.path === targetPath)
      if (!latestTab || !isTextFileDocument(latestTab.document) || latestTab.document.source !== targetSource) {
        // The person kept typing: the next save merges again from the same base.
        return true
      }
      if (result.ok) {
        if (result.content !== undefined && result.content !== targetSource) {
          onMergedSource(targetPath, result.content, result.revision)
          if (result.mergeConflicts) {
            dispatch(setDialogState({
              type: 'info',
              title: 'Cambios combinados',
              message: result.mergeConflicts === 1
                ? 'Mientras escribías, la nota cambió en el mismo lugar que editabas. Quedaron las dos versiones, una debajo de la otra: revisalas y borrá la que sobre.'
                : `Mientras escribías, la nota cambió en ${result.mergeConflicts} lugares que editabas. Quedaron las dos versiones de cada uno, una debajo de la otra: revisalas y borrá las que sobren.`,
            }))
          }
        } else {
          dispatch(updateTabSavedSource({ path: targetPath, source: targetSource, revision: result.revision }))
        }
        bumpLibraryIndexRevision()
        return true
      }
      dispatch(updateTabSaveStatus({ path: targetPath, status: 'error' }))
      dispatch(setDialogState({
        type: 'info',
        title: 'No se pudo guardar',
        message: result.error ?? 'No se pudo guardar el archivo. Tu texto sigue en el editor y se vuelve a intentar con el próximo cambio.',
      }))
      return false
    },
    [activeLibraryId, bumpLibraryIndexRevision, dispatch, onMergedSource],
  )

  const persistOpenTabBeforeClose = useCallback(
    async (tabPath: string): Promise<boolean> => {
      // The open editor hands over the text it still holds before the tab is read.
      flushPendingEditorChanges()
      clearPendingTextSaveByPath(tabPath)

      while (true) {
        const tab = store.getState().documents.openTabs.find((item) => item.document.path === tabPath)
        if (!tab) { return true }
        if (!isTextFileDocument(tab.document) || tab.document.source === tab.latestSavedSource) {
          return true
        }

        if (!(await persistTextDocumentSource(tabPath, tab.document.source))) {
          return false
        }
      }
    },
    [clearPendingTextSaveByPath, persistTextDocumentSource],
  )

  const closeTabByPath = useCallback(async (tabPath: string) => {
    if (
      tabPath === GRAPH_WORKSPACE_TAB_PATH
      || tabPath === CHAT_WORKSPACE_TAB_PATH
      || tabPath === TASK_MANAGER_WORKSPACE_TAB_PATH
      || tabPath === COLDPASS_WORKSPACE_TAB_PATH
      || tabPath === MEETING_WORKSPACE_TAB_PATH
      || tabPath === FINANCE_WORKSPACE_TAB_PATH
      || tabPath === AGENDA_WORKSPACE_TAB_PATH
      || tabPath === ROUTINE_WORKSPACE_TAB_PATH
      || tabPath === AI_ACTIONS_WORKSPACE_TAB_PATH
      || tabPath === AI_TASKS_WORKSPACE_TAB_PATH
      || tabPath === RECIPES_WORKSPACE_TAB_PATH
      || tabPath === HEALTH_WORKSPACE_TAB_PATH
      || tabPath === GYM_WORKSPACE_TAB_PATH
      || tabPath === HOME_WORKSPACE_TAB_PATH
    ) {
      const currentSpecialTabs = store.getState().documents.specialTabs
      if (
        (tabPath === GRAPH_WORKSPACE_TAB_PATH && !currentSpecialTabs.graph)
        || (tabPath === CHAT_WORKSPACE_TAB_PATH && !currentSpecialTabs.chat)
        || (tabPath === TASK_MANAGER_WORKSPACE_TAB_PATH && !currentSpecialTabs.taskManager)
        || (tabPath === COLDPASS_WORKSPACE_TAB_PATH && !currentSpecialTabs.coldPass)
        || (tabPath === MEETING_WORKSPACE_TAB_PATH && !currentSpecialTabs.meeting)
        || (tabPath === FINANCE_WORKSPACE_TAB_PATH && !currentSpecialTabs.finance)
        || (tabPath === AGENDA_WORKSPACE_TAB_PATH && !currentSpecialTabs.agenda)
        || (tabPath === ROUTINE_WORKSPACE_TAB_PATH && !currentSpecialTabs.routine)
        || (tabPath === AI_ACTIONS_WORKSPACE_TAB_PATH && !currentSpecialTabs.aiActions)
        || (tabPath === AI_TASKS_WORKSPACE_TAB_PATH && !currentSpecialTabs.aiTasks)
        || (tabPath === RECIPES_WORKSPACE_TAB_PATH && !currentSpecialTabs.recipes)
        || (tabPath === HEALTH_WORKSPACE_TAB_PATH && !currentSpecialTabs.health)
        || (tabPath === GYM_WORKSPACE_TAB_PATH && !currentSpecialTabs.gym)
        || (tabPath === HOME_WORKSPACE_TAB_PATH && !currentSpecialTabs.home)
      ) { return }

      const currentTabs = buildWorkspaceTitleTabs(store.getState().documents.openTabs, currentSpecialTabs)
      const closingIndex = currentTabs.findIndex((tab) => tab.path === tabPath)
      if (closingIndex < 0) { return }

      const nextSpecialTabs: OpenWorkspaceSpecialTabs = {
        graph: tabPath === GRAPH_WORKSPACE_TAB_PATH ? false : currentSpecialTabs.graph,
        chat: tabPath === CHAT_WORKSPACE_TAB_PATH ? false : currentSpecialTabs.chat,
        taskManager: tabPath === TASK_MANAGER_WORKSPACE_TAB_PATH ? false : currentSpecialTabs.taskManager,
        coldPass: tabPath === COLDPASS_WORKSPACE_TAB_PATH ? false : currentSpecialTabs.coldPass,
        meeting: tabPath === MEETING_WORKSPACE_TAB_PATH ? false : currentSpecialTabs.meeting,
        finance: tabPath === FINANCE_WORKSPACE_TAB_PATH ? false : currentSpecialTabs.finance,
        agenda: tabPath === AGENDA_WORKSPACE_TAB_PATH ? false : currentSpecialTabs.agenda,
        routine: tabPath === ROUTINE_WORKSPACE_TAB_PATH ? false : currentSpecialTabs.routine,
        aiActions: tabPath === AI_ACTIONS_WORKSPACE_TAB_PATH ? false : currentSpecialTabs.aiActions,
        aiTasks: tabPath === AI_TASKS_WORKSPACE_TAB_PATH ? false : currentSpecialTabs.aiTasks,
        recipes: tabPath === RECIPES_WORKSPACE_TAB_PATH ? false : currentSpecialTabs.recipes,
        health: tabPath === HEALTH_WORKSPACE_TAB_PATH ? false : currentSpecialTabs.health,
        gym: tabPath === GYM_WORKSPACE_TAB_PATH ? false : currentSpecialTabs.gym,
        home: tabPath === HOME_WORKSPACE_TAB_PATH ? false : currentSpecialTabs.home,
      }
      const remainingTabs = buildWorkspaceTitleTabs(store.getState().documents.openTabs, nextSpecialTabs)
      const currentActiveTabPath = selectActiveTabPath(store.getState())
      let nextActiveTabPath = currentActiveTabPath
      if (currentActiveTabPath === tabPath) {
        if (remainingTabs.length === 0) {
          nextActiveTabPath = null
        } else {
          const fallbackIndex = closingIndex > 0 ? closingIndex - 1 : 0
          const safeFallbackIndex = Math.min(fallbackIndex, remainingTabs.length - 1)
          nextActiveTabPath = remainingTabs[safeFallbackIndex].path
        }
      }
      if (nextActiveTabPath && !remainingTabs.some((tab) => tab.path === nextActiveTabPath)) {
        nextActiveTabPath = remainingTabs.length > 0 ? remainingTabs[remainingTabs.length - 1].path : null
      }
      dispatch(setSpecialTabs(nextSpecialTabs))
      if (tabPath === COLDPASS_WORKSPACE_TAB_PATH) { resetColdPassSession() }
      dispatch(setActiveTabPath(nextActiveTabPath))
      return
    }

    const tabToClose = store.getState().documents.openTabs.find((tab) => tab.document.path === tabPath)
    if (!tabToClose) { return }
    const canClose = await persistOpenTabBeforeClose(tabPath)
    // A note that cannot be saved never traps its tab: the person decides.
    if (!canClose && !(await confirmDiscard(tabToClose.document.name, 'No se pudieron guardar los últimos cambios de esta nota.'))) { return }

    const currentTabs = store.getState().documents.openTabs
    const closingIndex = currentTabs.findIndex((tab) => tab.document.path === tabPath)
    if (closingIndex < 0) { return }

    const remainingDocumentTabs = currentTabs.filter((tab) => tab.document.path !== tabPath)
    const specialTabs = store.getState().documents.specialTabs
    const workspaceTabsBeforeClose = buildWorkspaceTitleTabs(currentTabs, specialTabs)
    const workspaceTabsAfterClose = buildWorkspaceTitleTabs(remainingDocumentTabs, specialTabs)
    const currentActiveTabPath = selectActiveTabPath(store.getState())
    let nextActiveTabPath = currentActiveTabPath
    if (currentActiveTabPath === tabPath) {
      if (workspaceTabsAfterClose.length === 0) {
        nextActiveTabPath = null
      } else {
        const workspaceClosingIndex = workspaceTabsBeforeClose.findIndex((tab) => tab.path === tabPath)
        const fallbackIndex = workspaceClosingIndex > 0 ? workspaceClosingIndex - 1 : 0
        const safeFallbackIndex = Math.min(fallbackIndex, workspaceTabsAfterClose.length - 1)
        nextActiveTabPath = workspaceTabsAfterClose[safeFallbackIndex].path
      }
    }
    if (nextActiveTabPath && !workspaceTabsAfterClose.some((tab) => tab.path === nextActiveTabPath)) {
      nextActiveTabPath = workspaceTabsAfterClose.length > 0 ? workspaceTabsAfterClose[workspaceTabsAfterClose.length - 1].path : null
    }
    dispatch(setOpenTabs(remainingDocumentTabs))
    dispatch(setActiveTabPath(nextActiveTabPath))
  }, [confirmDiscard, dispatch, persistOpenTabBeforeClose, resetColdPassSession])

  const handleCloseTab = useCallback((tabPath: string) => { void closeTabByPath(tabPath) }, [closeTabByPath])

  const handleCloseActiveTab = useCallback(() => {
    const currentActiveTabPath = selectActiveTabPath(store.getState())
    if (!currentActiveTabPath) { return }
    void closeTabByPath(currentActiveTabPath)
  }, [closeTabByPath])

  const handleCycleToNextTab = useCallback(() => {
    const currentTabs = buildWorkspaceTitleTabs(store.getState().documents.openTabs, store.getState().documents.specialTabs)
    if (currentTabs.length <= 1) { return }
    const currentActiveTabPath = selectActiveTabPath(store.getState())
    const activeIndex = currentTabs.findIndex((tab) => tab.path === currentActiveTabPath)
    const nextIndex = activeIndex < 0 ? 0 : (activeIndex + 1) % currentTabs.length
    dispatch(setActiveTabPath(currentTabs[nextIndex].path))
  }, [dispatch])

  const persistDirtyTextDocuments = useCallback(async (): Promise<boolean> => {
    // A note whose only change is still in the editor counts as dirty too.
    flushPendingEditorChanges()
    const dirtyTabPaths = getDirtyOpenTextDocumentPaths(store.getState().documents.openTabs)

    for (const tabPath of dirtyTabPaths) {
      const didPersist = await persistOpenTabBeforeClose(tabPath)
      if (!didPersist) { return false }
    }

    return true
  }, [persistOpenTabBeforeClose])

  const closeTabsByPath = useCallback(async (path: string): Promise<boolean> => {
    const tabsToClose = store.getState().documents.openTabs
      .filter((tab) => isSameOrNestedPath(path, tab.document.path))

    if (tabsToClose.length === 0) { return true }

    for (const tab of tabsToClose) {
      const didPersist = await persistOpenTabBeforeClose(tab.document.path)
      if (!didPersist) { return false }
    }

    const currentTabs = store.getState().documents.openTabs
    const remainingDocumentTabs = currentTabs.filter((tab) => !isSameOrNestedPath(path, tab.document.path))
    if (remainingDocumentTabs.length === currentTabs.length) { return true }
    for (const tab of currentTabs) {
      if (isSameOrNestedPath(path, tab.document.path)) {
        clearPendingTextSaveByPath(tab.document.path)
      }
    }
    const workspaceTabsAfterClose = buildWorkspaceTitleTabs(remainingDocumentTabs, store.getState().documents.specialTabs)
    const currentActiveTabPath = selectActiveTabPath(store.getState())
    const nextActiveTabPath =
      currentActiveTabPath && workspaceTabsAfterClose.some((tab) => tab.path === currentActiveTabPath)
        ? currentActiveTabPath
        : workspaceTabsAfterClose.length > 0
          ? workspaceTabsAfterClose[workspaceTabsAfterClose.length - 1].path
          : null
    dispatch(setOpenTabs(remainingDocumentTabs))
    dispatch(setActiveTabPath(nextActiveTabPath))
    return true
  }, [clearPendingTextSaveByPath, dispatch, persistOpenTabBeforeClose])

  const renameOpenTabPath = useCallback((path: string, nextPath: string, name: string) => {
    // The editor of the old path closes with the rename: its text goes to the tab first.
    flushPendingEditorChanges()
    clearPendingTextSaveByPath(path)
    dispatch(renameTabPath({ oldPath: path, newPath: nextPath, name }))
    if (selectActiveTabPath(store.getState()) === path) { dispatch(setActiveTabPath(nextPath)) }
  }, [clearPendingTextSaveByPath, dispatch])

  const openDocumentInTab = useCallback((document: OpenFileDocument, latestSavedSource: string, latestSavedRevision?: string) => {
    dispatch(addOpenTab({ document, saveStatus: 'idle', latestSavedSource, latestSavedRevision }))
    dispatch(setActiveTabPath(document.path))
  }, [dispatch])

  const handleActivateTab = useCallback((tabPath: string) => {
    if (
      tabPath === GRAPH_WORKSPACE_TAB_PATH
      || tabPath === CHAT_WORKSPACE_TAB_PATH
      || tabPath === TASK_MANAGER_WORKSPACE_TAB_PATH
      || tabPath === COLDPASS_WORKSPACE_TAB_PATH
      || tabPath === MEETING_WORKSPACE_TAB_PATH
      || tabPath === FINANCE_WORKSPACE_TAB_PATH
      || tabPath === AGENDA_WORKSPACE_TAB_PATH
      || tabPath === ROUTINE_WORKSPACE_TAB_PATH
      || tabPath === AI_ACTIONS_WORKSPACE_TAB_PATH
      || tabPath === AI_TASKS_WORKSPACE_TAB_PATH
      || tabPath === RECIPES_WORKSPACE_TAB_PATH
      || tabPath === HEALTH_WORKSPACE_TAB_PATH
      || tabPath === GYM_WORKSPACE_TAB_PATH
      || tabPath === HOME_WORKSPACE_TAB_PATH
    ) {
      const specialTabs = store.getState().documents.specialTabs
      if (
        (tabPath === GRAPH_WORKSPACE_TAB_PATH && !specialTabs.graph)
        || (tabPath === CHAT_WORKSPACE_TAB_PATH && !specialTabs.chat)
        || (tabPath === TASK_MANAGER_WORKSPACE_TAB_PATH && !specialTabs.taskManager)
        || (tabPath === COLDPASS_WORKSPACE_TAB_PATH && !specialTabs.coldPass)
        || (tabPath === MEETING_WORKSPACE_TAB_PATH && !specialTabs.meeting)
        || (tabPath === FINANCE_WORKSPACE_TAB_PATH && !specialTabs.finance)
        || (tabPath === AGENDA_WORKSPACE_TAB_PATH && !specialTabs.agenda)
        || (tabPath === ROUTINE_WORKSPACE_TAB_PATH && !specialTabs.routine)
        || (tabPath === AI_ACTIONS_WORKSPACE_TAB_PATH && !specialTabs.aiActions)
        || (tabPath === AI_TASKS_WORKSPACE_TAB_PATH && !specialTabs.aiTasks)
        || (tabPath === RECIPES_WORKSPACE_TAB_PATH && !specialTabs.recipes)
        || (tabPath === HEALTH_WORKSPACE_TAB_PATH && !specialTabs.health)
        || (tabPath === GYM_WORKSPACE_TAB_PATH && !specialTabs.gym)
        || (tabPath === HOME_WORKSPACE_TAB_PATH && !specialTabs.home)
      ) { return }
      dispatch(setActiveTabPath(tabPath))
      return
    }
    if (!store.getState().documents.openTabs.some((tab) => tab.document.path === tabPath)) { return }
    dispatch(setActiveTabPath(tabPath))
  }, [dispatch])

  /** A text editor's new source; the Markdown editor names its note, which may no longer be the active tab. */
  const handleTextDocumentChange = useCallback((nextSource: string, documentPath?: string) => {
    const targetPath = documentPath ?? selectActiveTabPath(store.getState())
    if (!targetPath) { return }
    dispatch(updateTabSource({ path: targetPath, source: nextSource }))
  }, [dispatch])

  const handleExternalTextDocumentChange = useCallback((targetPath: string, nextSource: string, nextRevision?: string) => {
    const currentTab = store.getState().documents.openTabs.find((tab) => tab.document.path === targetPath)
    if (!currentTab || !isTextFileDocument(currentTab.document)) { return }
    clearPendingTextSaveByPath(targetPath)
    dispatch(updateTabSource({ path: targetPath, source: nextSource }))
    dispatch(updateTabSavedSource({ path: targetPath, source: nextSource, revision: nextRevision }))
  }, [clearPendingTextSaveByPath, dispatch])

  // The reset leaves Inicio open (see `resetTabs` in documentsSlice).
  const resetTabs = useCallback(() => {
    dispatch(resetTabsAction())
    resetColdPassSession()
  }, [dispatch, resetColdPassSession])

  return {
    closeTabByPath,
    handleCloseTab,
    handleCloseActiveTab,
    handleCycleToNextTab,
    closeTabsByPath,
    renameOpenTabPath,
    openDocumentInTab,
    handleActivateTab,
    handleTextDocumentChange,
    handleExternalTextDocumentChange,
    persistTextDocumentSource,
    persistDirtyTextDocuments,
    resetTabs,
  }
}
