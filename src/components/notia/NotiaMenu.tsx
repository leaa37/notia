import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { shallowEqual } from 'react-redux'
import { TAB_ICON, TITLEBAR_RIGHT_ACTIONS } from '../../constants/notiaMenu'
import { controlWindow, exitApplication, hasHostWindow, subscribeExitRequest } from '../../services/window/windowRuntime'
import { NotiaActionsProvider } from '../../context/notiaActions/NotiaActionsContext'
import { getRuntimeDevice } from '../../utils/platform/getRuntimeDevice'
import { NotiaSidebar } from './NotiaSidebar'
import { NotiaWorkspace } from './NotiaWorkspace'
import { NotiaRightPanel } from './NotiaRightPanel'
import { NotiaModals } from './NotiaModals'
import { WindowTitleBar } from './WindowTitleBar'
import { type TaskManagerChatContext } from '../../modules/task-manager/types/taskManagerTypes'
import { useGraphWorkspace } from './hooks/useGraphWorkspace'
import { useLibraryConfigSync } from './hooks/useLibraryConfigSync'
import { useRightPanelChatContext } from './hooks/useRightPanelChatContext'
import { useRightPanelChatFiles } from './hooks/useRightPanelChatFiles'
import { useLibrarySearch } from './hooks/useLibrarySearch'
import { useTextDocumentAutosave } from './hooks/useTextDocumentAutosave'
import { useColdPassSession } from './hooks/useColdPassSession'
import { useLibraryTreeSync } from './hooks/useLibraryTreeSync'
import { useTabManager } from './hooks/useTabManager'
import { useDocumentOpener } from './hooks/useDocumentOpener'
import { useToolbarActions } from './hooks/useToolbarActions'
import { useFileTreeActions } from './hooks/useFileTreeActions'
import { useLibraryManagerActions } from './hooks/useLibraryManagerActions'
import { useRightPanelMount } from './hooks/useRightPanelMount'
import { useHeavyViewMount } from './hooks/useHeavyViewMount'
import { useGlobalEventListeners } from './hooks/useGlobalEventListeners'
import { useTelegramLibraryChanges } from './hooks/useTelegramLibraryChanges'
import { useAgentDocumentChanges, useAgentEntryMoves } from './hooks/useAgentDocumentChanges'
import { store } from '../../store/index'
import { isTextFileDocument } from '../../types/views/fileDocument'
import { readLibraryDocument } from '../../services/libraries/libraryDocumentRuntime'
import { flushPendingEditorChanges } from '../../services/markdown/pendingEditorChanges'
import { useConfirmationEngine } from '../../context/confirmation/useConfirmationEngine'
import { useResumeWorkout } from '../../modules/gym/hooks/useResumeWorkout'
import { useAppDispatch, useAppSelector, type RootState } from '../../store/hooks'
import { toggleSidebar, toggleRightChatPanel, setSettingsOpen, setLibraryManagerOpen, setRightChatPanelOpen } from '../../features/ui/uiSlice'
import { selectIsRightChatPanelOpen } from '../../features/ui/uiSelectors'
import { toggleTheme, setAiSettings, setInkMathPreferences, setExplorerRefreshIntervalMs, setTelegramSettings, setTaskManagerPublicationPreferences } from '../../features/preferences/preferencesSlice'
import { selectTheme, selectAiSettings, selectInkMathPreferences, selectExplorerRefreshIntervalMs, selectTelegramSettings, selectTaskManagerPublicationPreferences } from '../../features/preferences/preferencesSelectors'
import { setSelectedLibraryId } from '../../features/library/librarySlice'
import { selectSelectedLibraryId, selectActiveLibrary } from '../../features/library/librarySelectors'
import { activateSpecialTab, setActiveTabPath, COLDPASS_WORKSPACE_TAB_PATH, HOME_WORKSPACE_TAB_PATH } from '../../features/documents/documentsSlice'
import { selectTreeNodes, selectActiveDocument, selectActiveWorkspaceView, selectFlatFileList } from '../../features/documents/documentsSelectors'
import { notiaTimer } from '../../services/runtime/notiaLogger'
import { saveAiPreferences } from '../../services/preferences/aiSettingsStorage'
import { useBackupSettingsMigration } from './hooks/useBackupSettingsMigration'
import { useDevicePreferencesPersistence } from './hooks/useDevicePreferencesPersistence'
import type { MarkdownDocumentUpdate, MarkdownSelectionContext } from '../../types/views/markdownSelection'
import { DEFAULT_LIBRARY_CONTEXTS, type LibraryContext } from '../../services/contexts/libraryContexts'
import { TASK_MANAGER_LOCAL_LIBRARY_USER_ID } from '../../modules/task-manager/types/taskManagerTypes'
import { PerformanceProfiler } from './PerformanceProfiler'

/**
 * What the shell reads of the active document: its text changes on every
 * pause in typing and must not draw the whole shell again.
 */
function selectActiveDocumentIdentity(state: RootState) {
  const document = selectActiveDocument(state)
  return document ? { path: document.path, name: document.name, viewKind: document.viewKind } : null
}

function NotiaMenuComponent() {
  const dispatch = useAppDispatch()
  const isRightChatPanelOpen = useAppSelector(selectIsRightChatPanelOpen)
  const theme = useAppSelector(selectTheme)
  const explorerRefreshIntervalMs = useAppSelector(selectExplorerRefreshIntervalMs)
  const inkMathPreferences = useAppSelector(selectInkMathPreferences, shallowEqual)
  const aiPreferences = useAppSelector(selectAiSettings, shallowEqual)
  const telegramPreferences = useAppSelector(selectTelegramSettings, shallowEqual)
  const taskManagerPublicationPreferences = useAppSelector(selectTaskManagerPublicationPreferences, shallowEqual)
  const activeLibraryId = useAppSelector(selectSelectedLibraryId)
  const activeLibrary = useAppSelector(selectActiveLibrary)
  useResumeWorkout(activeLibrary)
  const treeNodes = useAppSelector(selectTreeNodes)
  const flatFileList = useAppSelector(selectFlatFileList)
  const activeDocument = useAppSelector(selectActiveDocumentIdentity, shallowEqual)
  const activeWorkspaceView = useAppSelector(selectActiveWorkspaceView)
  // The catalog is loaded and saved by `AppAuthGate`, around the menu.
  useDevicePreferencesPersistence()
  useBackupSettingsMigration()

  useEffect(() => {
    const mountTimer = notiaTimer('ui', 'NotiaMenu.mount')
    return () => {
      mountTimer.success()
    }
  }, [])

  const [taskManagerActivePanelId, setTaskManagerActivePanelId] = useState('default')
  const [taskManagerChatContext, setTaskManagerChatContext] = useState<TaskManagerChatContext | null>(null)
  const [markdownSelection, setMarkdownSelection] = useState<MarkdownSelectionContext | null>(null)
  const [markdownExternalUpdate, setMarkdownExternalUpdate] = useState<MarkdownDocumentUpdate | null>(null)
  const [libraryContexts, setLibraryContexts] = useState<LibraryContext[]>(() => DEFAULT_LIBRARY_CONTEXTS.map((context) => ({ ...context })))
  const markdownExternalUpdateRevisionRef = useRef(0)
  const closeColdPassTabRef = useRef<() => void>(() => {})
  const isExitingApplicationRef = useRef(false)

  const {
    coldPassSession,
    coldPassEntries,
    coldPassNotice,
    coldPassPromptState,
    coldPassCredentialModalState,
    coldPassDeletePromptState,
    coldPassImportPromptState,
    isImportingVault,
    handleSubmitColdPassUnlock,
    handleUnlockColdPassWithBiometric,
    handleCloseColdPassPrompt,
    handleOpenColdPassCredentialModal,
    handleEditColdPassCredential,
    handleCloseColdPassCredentialModal,
    handleDeleteColdPassCredential,
    handleImportColdPassVault,
    handleSubmitColdPassCredential,
    handleCloseColdPassDeletePrompt,
    handleSubmitColdPassDeletePassword,
    handleCloseColdPassImportPrompt,
    handleSubmitColdPassImportPassword,
    resetColdPassSession,
  } = useColdPassSession({
    activeLibrary,
    activeWorkspaceView,
    closeColdPassTab: () => { closeColdPassTabRef.current() },
  })

  const runtimeDevice = useMemo(() => getRuntimeDevice(), [])
  const isAndroidRuntime = runtimeDevice === 'Android'
  const titlebarRightActions = useMemo(
    () => (isAndroidRuntime || !hasHostWindow() ? [] : TITLEBAR_RIGHT_ACTIONS),
    [isAndroidRuntime],
  )

  const activeLibraryPath = activeLibrary?.path

  // --- Stable callback refs for circular dependencies between hooks ---
  const persistTextDocumentSourceRef = useRef<((targetPath: string, targetSource: string) => Promise<boolean>) | null>(null)
  const persistDirtyTextDocumentsRef = useRef<(() => Promise<boolean>) | null>(null)
  const bumpLibraryIndexRevisionRef = useRef<() => void>(() => {})
  const persistDirtyTextDocumentsForTransition = useCallback(
    () => persistDirtyTextDocumentsRef.current?.() ?? Promise.resolve(true),
    [],
  )

  const { clearPendingTextSaveByPath } = useTextDocumentAutosave({
    persistTextDocumentSource: useCallback(
      (targetPath: string, targetSource: string) => persistTextDocumentSourceRef.current?.(targetPath, targetSource) ?? Promise.resolve(true),
      [],
    ),
  })

  // resetTabs from useTabManager is needed by useLibraryTreeSync,
  // but bumpLibraryIndexRevision from useLibraryTreeSync is needed by useTabManager.
  // Break the cycle: useTabManager takes bumpLibraryIndexRevision via ref.
  // A library that loads opens on Inicio.
  const resetTabsActionCallback = useCallback(() => {
    resetColdPassSession()
    dispatch(activateSpecialTab('home'))
    dispatch(setActiveTabPath(HOME_WORKSPACE_TAB_PATH))
  }, [dispatch, resetColdPassSession])

  const {
    handleToggleFolder,
    handleCollapseAllFolders,
    handleExpandAllFolders,
    notifyLibraryTreeChanged,
    bumpLibraryIndexRevision,
  } = useLibraryTreeSync({
    activeLibraryId,
    persistDirtyTextDocuments: persistDirtyTextDocumentsForTransition,
    resetTabsAndClearDrawioControllers: resetTabsActionCallback,
  })

  // Wire up the bumpLibraryIndexRevision ref so useTabManager can use it
  useEffect(() => {
    bumpLibraryIndexRevisionRef.current = bumpLibraryIndexRevision
  }, [bumpLibraryIndexRevision])

  const bumpLibraryIndexRevisionCallback = useCallback(() => { bumpLibraryIndexRevisionRef.current() }, [])

  // The text the backend wrote after merging goes to the open editor; it is
  // wired below, once the editor update exists.
  const mergedSourceRef = useRef<(path: string, source: string, revision?: string) => void>(() => {})
  const handleMergedSource = useCallback((path: string, source: string, revision?: string) => {
    mergedSourceRef.current(path, source, revision)
  }, [])
  const { confirm } = useConfirmationEngine()
  const confirmDiscard = useCallback((name: string, reason: string) => confirm({
    title: 'Cerrar sin guardar',
    message: `${reason} Si cerrás «${name}», se pierden esos cambios.`,
    confirmLabel: 'Cerrar sin guardar',
    cancelLabel: 'Seguir editando',
    tone: 'danger',
  }), [confirm])

  const tabManager = useTabManager({
    clearPendingTextSaveByPath,
    bumpLibraryIndexRevision: bumpLibraryIndexRevisionCallback,
    resetColdPassSession,
    activeLibraryId,
    onMergedSource: handleMergedSource,
    confirmDiscard,
  })
  const { persistDirtyTextDocuments } = tabManager

  // Wire up the persistTextDocumentSource ref so autosave uses the tab manager version
  useEffect(() => {
    persistTextDocumentSourceRef.current = tabManager.persistTextDocumentSource
  }, [tabManager.persistTextDocumentSource])

  useEffect(() => {
    persistDirtyTextDocumentsRef.current = tabManager.persistDirtyTextDocuments
  }, [tabManager.persistDirtyTextDocuments])

  useEffect(() => {
    let isMounted = true
    let unlisten: (() => void) | null = null

    void subscribeExitRequest(() => {
      if (isExitingApplicationRef.current) { return }
      isExitingApplicationRef.current = true
      void (async () => {
        if (await persistDirtyTextDocuments()) {
          if (await exitApplication()) { return }
        }
        isExitingApplicationRef.current = false
      })()
    }).then((cleanup) => {
      if (!isMounted) {
        cleanup()
        return
      }
      unlisten = cleanup
    }).catch((error: unknown) => {
      console.error('[notia] failed to subscribe to application exit request', error)
    })

    return () => {
      isMounted = false
      unlisten?.()
    }
  }, [persistDirtyTextDocuments])

  const handleSelectLibrary = useCallback((libraryId: string) => {
    void (async () => {
      if (await persistDirtyTextDocuments()) {
        dispatch(setSelectedLibraryId(libraryId))
      }
    })()
  }, [dispatch, persistDirtyTextDocuments])

  useEffect(() => {
    closeColdPassTabRef.current = () => { void tabManager.closeTabByPath(COLDPASS_WORKSPACE_TAB_PATH) }
  }, [tabManager])

  const {
    handleOpenFile,
    handleOpenFileFromView,
  } = useDocumentOpener({
    openDocumentInTab: tabManager.openDocumentInTab,
    activeLibrary: activeLibrary ? { id: activeLibrary.id, path: activeLibrary.path } : null,
  })

  const activeLibraryForToolbar = activeLibrary ?? null

  const {
    handleHeaderActionClick,
    handleRailActionClick,
    handleExplorerToolClick,
  } = useToolbarActions({
    activeLibrary: activeLibraryForToolbar,
    handleCollapseAllFolders,
    handleExpandAllFolders,
  })

  const {
    handleSubmitPendingCreation,
    handleCancelPendingCreation,
    handleNodeContextMenu,
    handleEmptyContextMenu,
    handleRenameSubmit,
    handleCancelRename,
    handleMoveNode,
  } = useFileTreeActions({
    activeLibrary: activeLibraryForToolbar,
    notifyLibraryTreeChanged,
    persistDirtyTextDocuments,
    closeTabsByPath: tabManager.closeTabsByPath,
    renameOpenTabPath: tabManager.renameOpenTabPath,
  })

  const {
    handleLibraryAdded,
    handleLibraryRemoved,
  } = useLibraryManagerActions({
    closeTabsByPath: tabManager.closeTabsByPath,
    persistDirtyTextDocuments,
  })

  // --- Mounting hooks ---

  const { mountedHeavyWorkspaceView } = useHeavyViewMount({ activeWorkspaceView, isAndroidRuntime })

  useRightPanelMount({ isAndroidRuntime, isRightChatPanelOpen })

  const handleNewNoteShortcut = useCallback(() => { handleExplorerToolClick('new-note') }, [handleExplorerToolClick])
  const handleGoToFileShortcut = useCallback(() => { handleHeaderActionClick('search') }, [handleHeaderActionClick])

  useGlobalEventListeners({
    handleCloseActiveTab: tabManager.handleCloseActiveTab,
    handleCycleToNextTab: tabManager.handleCycleToNextTab,
    handleNewNote: handleNewNoteShortcut,
    handleGoToFile: handleGoToFileShortcut,
  })

  // --- Simple dispatch-only callbacks ---

  const handleSidebarToggle = useCallback(() => { dispatch(toggleSidebar()) }, [dispatch])
  const handleRightChatPanelToggle = useCallback(() => { dispatch(toggleRightChatPanel()) }, [dispatch])
  const handleThemeToggle = useCallback(() => { dispatch(toggleTheme()) }, [dispatch])

  useEffect(() => {
    if (activeWorkspaceView === 'chat') { dispatch(setRightChatPanelOpen(false)) }
  }, [activeWorkspaceView, dispatch])

  const handleAiPreferencesChange = useCallback<(value: Parameters<typeof setAiSettings>[0]) => void>(
    (next) => {
      saveAiPreferences(next)
      dispatch(setAiSettings({ ...next, apiKey: '' }))
    },
    [dispatch],
  )
  const handleExplorerRefreshIntervalMsChange = useCallback<(value: number) => void>(
    (next) => dispatch(setExplorerRefreshIntervalMs(next)),
    [dispatch],
  )
  const handleInkMathPreferencesChange = useCallback<(value: Parameters<typeof setInkMathPreferences>[0]) => void>(
    (next) => dispatch(setInkMathPreferences(next)),
    [dispatch],
  )
  const handleTelegramPreferencesChange = useCallback<(value: Parameters<typeof setTelegramSettings>[0]) => void>(
    (next) => dispatch(setTelegramSettings(next)), [dispatch],
  )
  // Stable, so the modals (memoized) are not drawn again with the shell.
  const handleTaskManagerPublicationPreferencesChange = useCallback<(value: Parameters<typeof setTaskManagerPublicationPreferences>[0]) => void>(
    (next) => dispatch(setTaskManagerPublicationPreferences(next)), [dispatch],
  )

  useLibraryConfigSync({
    activeLibrary: activeLibraryForToolbar,
    aiPreferences,
    explorerRefreshIntervalMs,
    inkMathPreferences,
    setAiPreferences: handleAiPreferencesChange,
    setExplorerRefreshIntervalMs: handleExplorerRefreshIntervalMsChange,
    setInkMathPreferences: handleInkMathPreferencesChange,
    telegramPreferences,
    setTelegramPreferences: handleTelegramPreferencesChange,
    contexts: libraryContexts,
    setContexts: setLibraryContexts,
  })

  useTelegramLibraryChanges(activeLibraryId, () => notifyLibraryTreeChanged(activeLibraryPath ?? undefined))

  useLibrarySearch({
    treeNodes,
    treeNodesLibraryId: activeLibraryId,
    flatFileList,
  })


  const handleWindowAction = useCallback((action: NotiaWindowAction) => {
    if (action !== 'close') {
      void controlWindow(action)
      return
    }

    if (isExitingApplicationRef.current) { return }
    isExitingApplicationRef.current = true
    void (async () => {
      try {
        if (await persistDirtyTextDocuments()) {
          await controlWindow(action)
        }
      } finally {
        isExitingApplicationRef.current = false
      }
    })()
  }, [persistDirtyTextDocuments])
  const handleOpenLibraryManager = useCallback(() => { dispatch(setLibraryManagerOpen(true)) }, [dispatch])
  const handleOpenSettings = useCallback(() => { dispatch(setSettingsOpen(true)) }, [dispatch])
  const handleChatWorkspaceTreeChanged = useCallback((pathHint?: string) => {
    notifyLibraryTreeChanged(pathHint ?? activeLibraryPath ?? undefined)
  }, [activeLibraryPath, notifyLibraryTreeChanged])

  // Stable tabManager action selectors — avoid rebuilding the whole object on every render
  const {
    handleCloseTab,
    handleCloseActiveTab,
    handleCycleToNextTab,
    handleActivateTab,
    handleTextDocumentChange,
    handleExternalTextDocumentChange,
  } = tabManager

  const handleActiveMarkdownDocumentChanged = useCallback((documentPath: string, source: string, revision?: string) => {
    // The note already shows it (the agent's write was reloaded as it happened).
    const open = store.getState().documents.openTabs.find((tab) => tab.document.path === documentPath)
    if (open && isTextFileDocument(open.document) && open.document.source === source) return
    handleExternalTextDocumentChange(documentPath, source, revision)
    markdownExternalUpdateRevisionRef.current += 1
    setMarkdownExternalUpdate({
      documentPath,
      source,
      revision: markdownExternalUpdateRevisionRef.current,
    })
  }, [handleExternalTextDocumentChange])

  // A note the agent of a chat writes (a confirmed edit included) shows the
  // change in its open tab right away.
  const getOpenTextPaths = useCallback(() => (
    store.getState().documents.openTabs.filter((tab) => isTextFileDocument(tab.document)).map((tab) => tab.document.path)
  ), [])
  const { persistTextDocumentSource } = tabManager
  const reloadAgentChangedDocuments = useCallback((paths: string[]) => {
    const libraryId = activeLibraryId
    if (!libraryId) return
    // Keystrokes still in the editor count as unsaved text.
    flushPendingEditorChanges()
    for (const path of paths) {
      const tab = store.getState().documents.openTabs.find((item) => item.document.path === path)
      if (!tab || !isTextFileDocument(tab.document)) continue
      // Unsaved text is never replaced: saving it merges it with the change.
      if (tab.document.source !== tab.latestSavedSource) {
        clearPendingTextSaveByPath(path)
        void persistTextDocumentSource(path, tab.document.source)
        continue
      }
      const markdown = tab.document.viewKind === 'markdown'
      void readLibraryDocument(libraryId, path, { markdownDefaults: markdown }).then((read) => {
        const current = store.getState().documents.openTabs.find((item) => item.document.path === path)
        if (!read.ok || !current || !isTextFileDocument(current.document) || current.document.source === read.content) return
        if (markdown) handleActiveMarkdownDocumentChanged(path, read.content, read.revision)
        else handleExternalTextDocumentChange(path, read.content, read.revision)
      })
    }
  }, [activeLibraryId, clearPendingTextSaveByPath, handleActiveMarkdownDocumentChanged, handleExternalTextDocumentChange, persistTextDocumentSource])
  useEffect(() => {
    mergedSourceRef.current = (path, source, revision) => {
      const tab = store.getState().documents.openTabs.find((item) => item.document.path === path)
      if (tab && isTextFileDocument(tab.document) && tab.document.viewKind === 'markdown') handleActiveMarkdownDocumentChanged(path, source, revision)
      else handleExternalTextDocumentChange(path, source, revision)
    }
  }, [handleActiveMarkdownDocumentChanged, handleExternalTextDocumentChange])
  useAgentDocumentChanges(activeLibraryId, getOpenTextPaths, reloadAgentChangedDocuments)
  // A note or folder the agent moves keeps its open tabs, at the new path.
  const getOpenTabPaths = useCallback(() => store.getState().documents.openTabs.map((tab) => tab.document.path), [])
  const { renameOpenTabPath } = tabManager
  const followAgentMovedEntries = useCallback((moves: Array<{ from: string; to: string }>) => {
    for (const move of moves) renameOpenTabPath(move.from, move.to, move.to.split('/').pop() ?? move.to)
    notifyLibraryTreeChanged(activeLibraryPath ?? undefined)
  }, [activeLibraryPath, notifyLibraryTreeChanged, renameOpenTabPath])
  useAgentEntryMoves(activeLibraryId, getOpenTabPaths, followAgentMovedEntries)

  // --- Derived values ---

  const previousChatFiles = useRightPanelChatFiles({
    libraryId: activeLibrary?.id,
    activeWorkspaceView,
    isRightChatPanelOpen,
    treeNodes,
  })

  const activeTaskManagerVault = useMemo(
    () => (activeLibrary
      ? {
        path: activeLibrary.path,
        androidTreeUri: activeLibrary.androidTreeUri,
        libraryId: activeLibrary.id,
        libraryUserId: TASK_MANAGER_LOCAL_LIBRARY_USER_ID,
      }
      : null),
    [activeLibrary],
  )

  const {
    graphChatContextSummary,
    graphChatEffectivePaths,
    graphChatSelectedPaths,
    graphModel,
    searchGraph,
    isGraphLoading,
    setGraphChatSelectedPaths,
  } = useGraphWorkspace({
    activeLibrary,
    activeWorkspaceView,
  })

  const {
    agentCorpusPaths: rightPanelAgentCorpusPaths,
    agentScope: rightPanelAgentScope,
    preferredContextMode: rightPanelPreferredContextMode,
    preferredContextName: rightPanelPreferredContextName,
    preferredContextPaths: rightPanelPreferredContextPaths,
    preferredContextScopeKey: rightPanelPreferredContextScopeKey,
    rightPanelChatContextKey,
    rightPanelChatContext,
    transientContextMode: rightPanelTransientContextMode,
    transientContextPaths: rightPanelTransientContextPaths,
    transientContextSummary: rightPanelTransientContextSummary,
  } = useRightPanelChatContext({
    activeDocument,
    activeWorkspaceView,
    graphChatContextSummary,
    graphChatEffectivePaths,
    graphChatHasExplicitSelection: graphChatSelectedPaths.length > 0,
    taskManagerActivePanelId,
    taskManagerChatContext,
    markdownSelection,
  })

  // --- NotiaActions context value ---
  const actionsValue = useMemo(() => ({
    openFile: handleOpenFile,
    openFileFromView: handleOpenFileFromView,
    closeTab: handleCloseTab,
    closeActiveTab: handleCloseActiveTab,
    cycleToNextTab: handleCycleToNextTab,
    toggleFolder: handleToggleFolder,
    toggleSidebar: handleSidebarToggle,
    toggleRightChatPanel: handleRightChatPanelToggle,
    toggleTheme: handleThemeToggle,
    selectLibrary: handleSelectLibrary,
    activateTab: handleActivateTab,
    openSettings: handleOpenSettings,
    openLibraryManager: handleOpenLibraryManager,
    railActionClick: handleRailActionClick,
    headerActionClick: handleHeaderActionClick,
    explorerToolClick: handleExplorerToolClick,
    submitPendingCreation: handleSubmitPendingCreation,
    cancelPendingCreation: handleCancelPendingCreation,
    renameSubmit: handleRenameSubmit,
    cancelRename: handleCancelRename,
    nodeContextMenu: handleNodeContextMenu,
    emptyContextMenu: handleEmptyContextMenu,
    moveNode: handleMoveNode,
    libraryAdded: handleLibraryAdded,
    libraryRemoved: handleLibraryRemoved,
    textDocumentChange: handleTextDocumentChange,
    chatWorkspaceTreeChanged: handleChatWorkspaceTreeChanged,
    windowAction: handleWindowAction,
    coldPassOpenCredentialModal: handleOpenColdPassCredentialModal,
    coldPassImportVault: handleImportColdPassVault,
    coldPassEditCredential: handleEditColdPassCredential,
    coldPassDeleteCredential: handleDeleteColdPassCredential,
  }), [
    handleOpenFile,
    handleOpenFileFromView,
    handleCloseTab,
    handleCloseActiveTab,
    handleCycleToNextTab,
    handleToggleFolder,
    handleSidebarToggle,
    handleRightChatPanelToggle,
    handleThemeToggle,
    handleSelectLibrary,
    handleActivateTab,
    handleOpenSettings,
    handleOpenLibraryManager,
    handleRailActionClick,
    handleHeaderActionClick,
    handleExplorerToolClick,
    handleSubmitPendingCreation,
    handleCancelPendingCreation,
    handleRenameSubmit,
    handleCancelRename,
    handleNodeContextMenu,
    handleEmptyContextMenu,
    handleMoveNode,
    handleLibraryAdded,
    handleLibraryRemoved,
    handleTextDocumentChange,
    handleChatWorkspaceTreeChanged,
    handleWindowAction,
    handleOpenColdPassCredentialModal,
    handleImportColdPassVault,
    handleEditColdPassCredential,
    handleDeleteColdPassCredential,
  ])

  return (
    <NotiaActionsProvider actions={actionsValue}>
      <div
        className={`notia-app-shell ${theme === 'dark' ? 'notia-theme-dark' : 'notia-theme-light'} ${
          isAndroidRuntime ? 'notia-app-shell--android' : ''
        }`.trim()}
      >
        <div className="notia-workspace" data-notia-prevent-menu-close>
          <PerformanceProfiler id="explorer">
            <NotiaSidebar />
          </PerformanceProfiler>
          <div className="notia-main-column" data-notia-prevent-menu-close>
            <WindowTitleBar
              tabIcon={TAB_ICON}
              rightActions={titlebarRightActions}
              showRightPanelToggle={activeWorkspaceView !== 'chat'}
            />
            <PerformanceProfiler id="workspace">
              <NotiaWorkspace
                mountedHeavyWorkspaceView={mountedHeavyWorkspaceView}
                isAndroidRuntime={isAndroidRuntime}
                coldPassEntries={coldPassEntries}
                coldPassNotice={coldPassNotice}
                coldPassSession={coldPassSession}
                activeTaskManagerVault={activeTaskManagerVault}
                libraryContexts={libraryContexts}
                graphModel={graphModel}
                searchGraph={searchGraph}
                isGraphLoading={isGraphLoading}
                graphChatSelectedPaths={graphChatSelectedPaths}
                setGraphChatSelectedPaths={setGraphChatSelectedPaths}
                previousChatFiles={previousChatFiles}
                setTaskManagerActivePanelId={setTaskManagerActivePanelId}
                setTaskManagerChatContext={setTaskManagerChatContext}
                isImportingVault={isImportingVault}
                onMarkdownSelectionChange={setMarkdownSelection}
                markdownExternalUpdate={markdownExternalUpdate}
              />
            </PerformanceProfiler>
          </div>
          <PerformanceProfiler id="right-panel">
            <NotiaRightPanel
              isMeetingContext={activeWorkspaceView === 'meeting'}
              agentCorpusPaths={rightPanelAgentCorpusPaths}
              agentScope={rightPanelAgentScope}
              previousChats={previousChatFiles}
              rightPanelChatContextKey={rightPanelChatContextKey}
              rightPanelChatContext={rightPanelChatContext}
              rightPanelPreferredContextPaths={rightPanelPreferredContextPaths}
              rightPanelPreferredContextName={rightPanelPreferredContextName}
              rightPanelPreferredContextMode={rightPanelPreferredContextMode}
              rightPanelPreferredContextScopeKey={rightPanelPreferredContextScopeKey}
              rightPanelTransientContextPaths={rightPanelTransientContextPaths}
              rightPanelTransientContextMode={rightPanelTransientContextMode}
              rightPanelTransientContextSummary={rightPanelTransientContextSummary}
              rightPanelTransientSelectedPaths={graphChatSelectedPaths}
              onRightPanelTransientSelectedPathsChange={setGraphChatSelectedPaths}
              markdownSelection={markdownSelection}
              onActiveMarkdownDocumentChanged={handleActiveMarkdownDocumentChanged}
            />
          </PerformanceProfiler>
        </div>
      <NotiaModals
          onAiPreferencesChange={handleAiPreferencesChange}
          onExplorerRefreshIntervalMsChange={handleExplorerRefreshIntervalMsChange}
          onInkMathPreferencesChange={handleInkMathPreferencesChange}
          contexts={libraryContexts}
          onContextsChange={setLibraryContexts}
        onTelegramPreferencesChange={handleTelegramPreferencesChange}
        taskManagerPublicationPreferences={taskManagerPublicationPreferences}
        onTaskManagerPublicationPreferencesChange={handleTaskManagerPublicationPreferencesChange}
          coldPassPromptState={coldPassPromptState}
          coldPassDeletePromptState={coldPassDeletePromptState}
          coldPassImportPromptState={coldPassImportPromptState}
          coldPassCredentialModalState={coldPassCredentialModalState}
          coldPassSession={coldPassSession}
          handleSubmitColdPassUnlock={handleSubmitColdPassUnlock}
          handleUnlockColdPassWithBiometric={handleUnlockColdPassWithBiometric}
          handleCloseColdPassPrompt={handleCloseColdPassPrompt}
          handleSubmitColdPassDeletePassword={handleSubmitColdPassDeletePassword}
          handleCloseColdPassDeletePrompt={handleCloseColdPassDeletePrompt}
          handleSubmitColdPassImportPassword={handleSubmitColdPassImportPassword}
          handleCloseColdPassImportPrompt={handleCloseColdPassImportPrompt}
          handleSubmitColdPassCredential={handleSubmitColdPassCredential}
          handleCloseColdPassCredentialModal={handleCloseColdPassCredentialModal}
        />
      </div>
    </NotiaActionsProvider>
  )
}

export function NotiaMenu() {
  return <NotiaMenuComponent />
}
