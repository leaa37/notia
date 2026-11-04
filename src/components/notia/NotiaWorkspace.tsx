import { memo, Suspense, useCallback, useMemo, lazy, type ReactNode } from 'react'
import { shallowEqual } from 'react-redux'
import { useAppSelector } from '../../store/hooks'
import { selectIsHeavyWorkspaceView } from '../../features/ui/uiSelectors'
import { selectActiveWorkspaceView, selectActiveDocument, selectSaveStatus, selectTreeNodes } from '../../features/documents/documentsSelectors'
import { selectActiveLibraryName, selectActiveLibrary } from '../../features/library/librarySelectors'
import { selectAiSettings, selectTheme } from '../../features/preferences/preferencesSelectors'
import { useNotiaAction } from '../../context/notiaActions/useNotiaAction'
import { MainView } from './MainView'
import { PerformanceProfiler } from './PerformanceProfiler'
import { ChatWorkspaceView } from './views/chat/ChatWorkspaceView'
import { HomeView } from './views/HomeView'
import { useWikiLinkTargets } from './hooks/useWikiLinkTargets'
import type { ColdPassEntryView } from '../../types/coldpass'
import type { ColdPassNotice } from './hooks/useColdPassSession'
import type { TaskManagerChatContext, TaskManagerVaultRef } from '../../modules/task-manager/types/taskManagerTypes'
import type { LibraryGraphModel } from '../../types/graph/libraryGraph'
import type { MarkdownDocumentUpdate, MarkdownSelectionContext } from '../../types/views/markdownSelection'
import type { LibraryContext } from '../../services/contexts/libraryContexts'
import type { GraphSearchResult } from '../../hooks/useLibraryGraphData'
import { mutateLibraryEntry } from '../../services/libraries/libraryRuntime'
import { getParentDirectory } from './hooks/useTabManager'

const GraphView = lazy(async () => {
  const module = await import('./views/GraphView')
  return { default: module.GraphView }
})
const TaskManagerApp = lazy(async () => {
  const module = await import('../../modules/task-manager/components/TaskManagerApp')
  return { default: module.TaskManagerApp }
})
// The modules load when opened: the start of the app parses less code,
// which on Android delays the first screen.
const ColdPassView = lazy(async () => ({ default: (await import('./views/ColdPassView')).ColdPassView }))
const MeetingView = lazy(async () => ({ default: (await import('./views/MeetingView')).MeetingView }))
const FinanceView = lazy(async () => ({ default: (await import('./views/FinanceView')).FinanceView }))
const AgendaView = lazy(async () => ({ default: (await import('./views/AgendaView')).AgendaView }))
const RoutineView = lazy(async () => ({ default: (await import('./views/RoutineView')).RoutineView }))
const AiActionsView = lazy(async () => ({ default: (await import('./views/AiActionsView')).AiActionsView }))
const AiTasksView = lazy(async () => ({ default: (await import('./views/AiTasksView')).AiTasksView }))
const RecipesView = lazy(async () => ({ default: (await import('./views/RecipesView')).RecipesView }))
const HealthView = lazy(async () => ({ default: (await import('./views/HealthView')).HealthView }))
const GymView = lazy(async () => ({ default: (await import('./views/GymView')).GymView }))

interface NotiaWorkspaceProps {
  mountedHeavyWorkspaceView: string
  isAndroidRuntime: boolean
  coldPassEntries: ColdPassEntryView[]
  coldPassNotice: ColdPassNotice | null
  coldPassSession: object | null
  activeTaskManagerVault: TaskManagerVaultRef | null
  libraryContexts: LibraryContext[]
  graphModel: LibraryGraphModel
  searchGraph: (query: string) => Promise<GraphSearchResult[]>
  isGraphLoading: boolean
  graphChatSelectedPaths: string[]
  setGraphChatSelectedPaths: (paths: string[]) => void
  previousChatFiles: { id: string; filePath: string; title: string }[]
  setTaskManagerActivePanelId: (id: string) => void
  setTaskManagerChatContext: (ctx: TaskManagerChatContext | null) => void
  isImportingVault: boolean
  onMarkdownSelectionChange: (selection: MarkdownSelectionContext | null) => void
  markdownExternalUpdate: MarkdownDocumentUpdate | null
}

/** A module loaded on demand, with the waiting card while it arrives. */
function ModuleView({ children }: { children: ReactNode }) {
  return <Suspense fallback={<WorkspaceFallback label="Abriendo el módulo" />}>{children}</Suspense>
}

function WorkspaceFallback({ label }: { label: string }) {
  return (
    <main className="notia-main" role="status" aria-live="polite">
      <div className="notia-workspace-deferred-view">
        <div className="notia-workspace-deferred-card">
          <strong>{label}</strong>
          <span>Cargando módulo pesado...</span>
        </div>
      </div>
    </main>
  )
}

function NotiaWorkspaceComponent({
  mountedHeavyWorkspaceView,
  isAndroidRuntime,
  coldPassEntries,
  coldPassNotice,
  coldPassSession,
  activeTaskManagerVault,
  libraryContexts,
  graphModel,
  searchGraph,
  isGraphLoading,
  graphChatSelectedPaths,
  setGraphChatSelectedPaths,
  previousChatFiles,
  setTaskManagerActivePanelId,
  setTaskManagerChatContext,
  isImportingVault,
  onMarkdownSelectionChange,
  markdownExternalUpdate,
}: NotiaWorkspaceProps) {
  const handleOpenFileFromView = useNotiaAction('openFileFromView')
  const handleChatWorkspaceTreeChanged = useNotiaAction('chatWorkspaceTreeChanged')
  const handleOpenFile = useNotiaAction('openFile')
  const handleOpenColdPassCredentialModal = useNotiaAction('coldPassOpenCredentialModal')
  const handleColdPassImportVault = useNotiaAction('coldPassImportVault')
  const handleColdPassEditCredential = useNotiaAction('coldPassEditCredential')
  const handleColdPassDeleteCredential = useNotiaAction('coldPassDeleteCredential')
  const handleTextDocumentChange = useNotiaAction('textDocumentChange')

  const chatCallbacks = useMemo(() => ({
    onChatCreated: handleChatWorkspaceTreeChanged,
    onChatDeleted: handleChatWorkspaceTreeChanged,
  }), [handleChatWorkspaceTreeChanged])

  const handleTaskManagerPanelChange = useCallback((id: string) => {
    setTaskManagerActivePanelId(id)
  }, [setTaskManagerActivePanelId])

  const handleTaskManagerChatContextChange = useCallback((ctx: TaskManagerChatContext | null) => {
    setTaskManagerChatContext(ctx)
  }, [setTaskManagerChatContext])

  const activeWorkspaceView = useAppSelector(selectActiveWorkspaceView)
  const activeDocument = useAppSelector(selectActiveDocument)
  const saveStatus = useAppSelector(selectSaveStatus)
  const isHeavyWorkspaceView = useAppSelector(selectIsHeavyWorkspaceView)
  const libraryName = useAppSelector(selectActiveLibraryName)
  const treeNodes = useAppSelector(selectTreeNodes)
  const activeLibrary = useAppSelector(selectActiveLibrary)
  const aiPreferences = useAppSelector(selectAiSettings, shallowEqual)
  const appTheme = useAppSelector(selectTheme)
  const isMarkdownDocumentActive = activeDocument?.viewKind === 'markdown'

  const markdownWikiLinkTargets = useWikiLinkTargets(isMarkdownDocumentActive ? activeLibrary?.id : undefined, treeNodes)
  const activeDocumentPath = activeDocument?.path

  // «Crear nota» of a link property: the new note goes next to the open one.
  const handleCreateLinkedNote = useCallback(async (title: string): Promise<string | null> => {
    if (!activeLibrary || !activeDocumentPath) return 'No hay una biblioteca abierta.'
    const hasFolder = /[\\/]/.test(activeDocumentPath)
    const parentPath = hasFolder ? getParentDirectory(activeDocumentPath) : activeLibrary.path
    const result = await mutateLibraryEntry(activeLibrary, { action: 'create', parentPath, name: title, kind: 'note' })
    if (!result.ok) return result.error ?? 'No se pudo crear la nota.'
    handleChatWorkspaceTreeChanged(parentPath)
    return null
  }, [activeDocumentPath, activeLibrary, handleChatWorkspaceTreeChanged])

  const shouldDeferHeavyWorkspaceMount =
    isAndroidRuntime
    && isHeavyWorkspaceView
    && mountedHeavyWorkspaceView !== activeWorkspaceView

  if (shouldDeferHeavyWorkspaceMount) {
    return (
      <main className="notia-main">
        <div className="notia-workspace-deferred-view" role="status" aria-live="polite">
          <div className="notia-workspace-deferred-card">
            <strong>
              {activeWorkspaceView === 'graph'
                ? 'Preparando graph view'
                : activeWorkspaceView === 'task-manager'
                  ? 'Preparando Task Manager'
                  : 'Preparando chat'}
            </strong>
            <span>
              {activeWorkspaceView === 'graph'
                ? 'Android muestra primero la vista y completa la carga pesada justo despues.'
                : activeWorkspaceView === 'task-manager'
                  ? 'Android abre primero el modulo y deja la lectura del vault para el siguiente frame.'
                  : 'Android abre primero el espacio y completa la carga del historial a continuacion.'}
            </span>
          </div>
        </div>
      </main>
    )
  }

  if (activeWorkspaceView === 'graph') {
    return (
      <Suspense fallback={<WorkspaceFallback label="Preparando graph view" />}>
        <PerformanceProfiler id="graph-view">
          <GraphView
            graphModel={graphModel}
            searchGraph={searchGraph}
            libraryName={libraryName}
            isLoading={isGraphLoading}
            onOpenFile={handleOpenFileFromView}
            chatSelectedPaths={graphChatSelectedPaths}
            onChatSelectedPathsChange={setGraphChatSelectedPaths}
          />
        </PerformanceProfiler>
      </Suspense>
    )
  }

  if (activeWorkspaceView === 'chat') {
    return (
      <PerformanceProfiler id="chat">
        <ChatWorkspaceView
          agentScope="library"
          library={activeLibrary}
          aiPreferences={aiPreferences}
          previousChats={previousChatFiles}
          onChatCreated={chatCallbacks.onChatCreated}
          onChatDeleted={chatCallbacks.onChatDeleted}
        />
      </PerformanceProfiler>
    )
  }

  if (activeWorkspaceView === 'task-manager') {
    return (
      <Suspense fallback={<WorkspaceFallback label="Preparando Task Manager" />}>
        <TaskManagerApp
          embedded
          vault={activeTaskManagerVault}
          onOpenTaskFile={handleOpenFile}
          onActivePanelChange={handleTaskManagerPanelChange}
          onActiveChatContextChange={handleTaskManagerChatContextChange}
          contexts={libraryContexts}
        />
      </Suspense>
    )
  }

  if (activeWorkspaceView === 'coldpass') {
    return (
      <ModuleView>
        <ColdPassView
          entries={coldPassEntries}
          notice={coldPassNotice}
          isUnlocked={Boolean(coldPassSession)}
          isImportingVault={isImportingVault}
          onCreateCredential={handleOpenColdPassCredentialModal}
          onImportVault={handleColdPassImportVault}
          onEditCredential={handleColdPassEditCredential}
          onDeleteCredential={handleColdPassDeleteCredential}
        />
      </ModuleView>
    )
  }

  if (activeWorkspaceView === 'meeting') {
    return <ModuleView><MeetingView /></ModuleView>
  }

  if (activeWorkspaceView === 'finance') {
    return <ModuleView><FinanceView library={activeLibrary} /></ModuleView>
  }

  if (activeWorkspaceView === 'agenda') {
    return <ModuleView><AgendaView library={activeLibrary} /></ModuleView>
  }

  if (activeWorkspaceView === 'home') {
    return <HomeView library={activeLibrary} />
  }

  if (activeWorkspaceView === 'routine') {
    return <ModuleView><RoutineView library={activeLibrary} /></ModuleView>
  }
  if (activeWorkspaceView === 'ai-actions') {
    return <ModuleView><AiActionsView library={activeLibrary} /></ModuleView>
  }
  if (activeWorkspaceView === 'ai-tasks') {
    return <ModuleView><AiTasksView library={activeLibrary} /></ModuleView>
  }
  if (activeWorkspaceView === 'recipes') {
    return <ModuleView><RecipesView library={activeLibrary} /></ModuleView>
  }
  if (activeWorkspaceView === 'health') {
    return <ModuleView><HealthView library={activeLibrary} /></ModuleView>
  }
  if (activeWorkspaceView === 'gym') {
    return <ModuleView><GymView library={activeLibrary} /></ModuleView>
  }

  return (
    <MainView
      activeDocument={activeDocument}
      saveStatus={saveStatus}
      onTextDocumentChange={handleTextDocumentChange}
      onSelectionChange={onMarkdownSelectionChange}
      externalSourceUpdate={markdownExternalUpdate}
      markdownWikiLinkTargets={markdownWikiLinkTargets}
      onOpenLinkedFile={handleOpenFileFromView}
      onCreateLinkedNote={handleCreateLinkedNote}
      theme={appTheme}
      contexts={libraryContexts}
      activeLibrary={activeLibrary}
    />
  )
}

export const NotiaWorkspace = memo(NotiaWorkspaceComponent)
NotiaWorkspace.displayName = 'NotiaWorkspace'
