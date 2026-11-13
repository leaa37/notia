import { createSlice, type PayloadAction } from '@reduxjs/toolkit'
import { invalidateMermaidCache } from '../../modules/mermaid/engines/mermaidEngine'
import type { NotiaFileNode, NotiaFlatFileEntry } from '../../types/notia'
import type { NotiaDocumentSaveStatus } from '../../types/views/fileDocument'
import type { DocumentsState, OpenDocumentTab, OpenWorkspaceSpecialTabs } from './documentsTypes'

const GRAPH_WORKSPACE_TAB_PATH = '__workspace_graph__'
const CHAT_WORKSPACE_TAB_PATH = '__workspace_chat__'
const TASK_MANAGER_WORKSPACE_TAB_PATH = '__workspace_task_manager__'
const COLDPASS_WORKSPACE_TAB_PATH = '__workspace_coldpass__'
const MEETING_WORKSPACE_TAB_PATH = '__workspace_meeting__'
const FINANCE_WORKSPACE_TAB_PATH = '__workspace_finance__'
const AGENDA_WORKSPACE_TAB_PATH = '__workspace_agenda__'
const ROUTINE_WORKSPACE_TAB_PATH = '__workspace_routine__'
const AI_ACTIONS_WORKSPACE_TAB_PATH = '__workspace_ai_actions__'
const AI_TASKS_WORKSPACE_TAB_PATH = '__workspace_ai_tasks__'
const RECIPES_WORKSPACE_TAB_PATH = '__workspace_recipes__'
const HEALTH_WORKSPACE_TAB_PATH = '__workspace_health__'
const GYM_WORKSPACE_TAB_PATH = '__workspace_gym__'
const HOME_WORKSPACE_TAB_PATH = '__workspace_home__'

export { GRAPH_WORKSPACE_TAB_PATH, CHAT_WORKSPACE_TAB_PATH, TASK_MANAGER_WORKSPACE_TAB_PATH, COLDPASS_WORKSPACE_TAB_PATH, MEETING_WORKSPACE_TAB_PATH, FINANCE_WORKSPACE_TAB_PATH, AGENDA_WORKSPACE_TAB_PATH, ROUTINE_WORKSPACE_TAB_PATH, AI_ACTIONS_WORKSPACE_TAB_PATH, AI_TASKS_WORKSPACE_TAB_PATH, RECIPES_WORKSPACE_TAB_PATH, HEALTH_WORKSPACE_TAB_PATH, GYM_WORKSPACE_TAB_PATH, HOME_WORKSPACE_TAB_PATH }

/** With nothing open, the workspace shows Inicio: at start and after a library loads. */
const initialState: DocumentsState = {
  openTabs: [],
  activeTabPath: HOME_WORKSPACE_TAB_PATH,
  specialTabs: {
    graph: false,
    chat: false,
    taskManager: false,
    coldPass: false,
    meeting: false,
    finance: false,
    agenda: false,
    routine: false,
    aiActions: false,
    aiTasks: false,
    recipes: false,
    health: false,
    gym: false,
    home: true,
  },
  treeNodes: [],
  searchQuery: '',
  searchMatchedPaths: [],
  isSearchLoading: false,
  pendingCreation: null,
  renamingPath: null,
  clipboardEntry: null,
  contextMenu: null,
  dialogState: null,
  loadingFolderIds: [],
  folderLoadError: null,
  flatFileList: [],
}

const documentsSlice = createSlice({
  name: 'documents',
  initialState,
  reducers: {
    setOpenTabs(state, action: PayloadAction<OpenDocumentTab[]>) {
      state.openTabs = action.payload
    },
    addOpenTab(state, action: PayloadAction<OpenDocumentTab>) {
      const existing = state.openTabs.some((tab) => tab.document.path === action.payload.document.path)
      if (!existing) {
        state.openTabs.push(action.payload)
      }
      state.activeTabPath = action.payload.document.path
    },
    updateTabSource(state, action: PayloadAction<{ path: string; source: string }>) {
      const tab = state.openTabs.find((t) => t.document.path === action.payload.path)
      if (tab && tab.document.viewKind !== 'image') {
        const textDoc = tab.document as { source: string }
        if (textDoc.source !== action.payload.source) {
          textDoc.source = action.payload.source
          tab.saveStatus = action.payload.source === tab.latestSavedSource ? 'idle' : 'saving'
        }
      }
    },
    updateTabSaveStatus(state, action: PayloadAction<{ path: string; status: NotiaDocumentSaveStatus }>) {
      const tab = state.openTabs.find((t) => t.document.path === action.payload.path)
      if (tab) {
        tab.saveStatus = action.payload.status
      }
    },
    updateTabSavedSource(state, action: PayloadAction<{ path: string; source: string; revision?: string }>) {
      const tab = state.openTabs.find((t) => t.document.path === action.payload.path)
      if (tab) {
        tab.latestSavedSource = action.payload.source
        tab.latestSavedRevision = action.payload.revision
        tab.saveStatus = 'idle'
      }
    },
    removeTabByPath(state, action: PayloadAction<string>) {
      state.openTabs = state.openTabs.filter((tab) => tab.document.path !== action.payload)
    },
    renameTabPath(state, action: PayloadAction<{ oldPath: string; newPath: string; name: string }>) {
      const tab = state.openTabs.find((t) => t.document.path === action.payload.oldPath)
      if (tab) {
        tab.document.path = action.payload.newPath
        tab.document.name = action.payload.name
        tab.document.extension = action.payload.name.includes('.')
          ? action.payload.name.split('.').pop() ?? ''
          : ''
      }
      if (state.activeTabPath === action.payload.oldPath) {
        state.activeTabPath = action.payload.newPath
      }
    },
    setActiveTabPath(state, action: PayloadAction<string | null>) {
      state.activeTabPath = action.payload
    },
    setSpecialTabs(state, action: PayloadAction<OpenWorkspaceSpecialTabs>) {
      state.specialTabs = action.payload
    },
    activateSpecialTab(state, action: PayloadAction<keyof OpenWorkspaceSpecialTabs>) {
      state.specialTabs[action.payload] = true
      const pathMap: Record<keyof OpenWorkspaceSpecialTabs, string> = {
        graph: GRAPH_WORKSPACE_TAB_PATH,
        chat: CHAT_WORKSPACE_TAB_PATH,
        taskManager: TASK_MANAGER_WORKSPACE_TAB_PATH,
        coldPass: COLDPASS_WORKSPACE_TAB_PATH,
        meeting: MEETING_WORKSPACE_TAB_PATH,
        finance: FINANCE_WORKSPACE_TAB_PATH,
        agenda: AGENDA_WORKSPACE_TAB_PATH,
        routine: ROUTINE_WORKSPACE_TAB_PATH,
        aiActions: AI_ACTIONS_WORKSPACE_TAB_PATH,
        aiTasks: AI_TASKS_WORKSPACE_TAB_PATH,
        recipes: RECIPES_WORKSPACE_TAB_PATH,
        health: HEALTH_WORKSPACE_TAB_PATH,
        gym: GYM_WORKSPACE_TAB_PATH,
        home: HOME_WORKSPACE_TAB_PATH,
      }
      state.activeTabPath = pathMap[action.payload]
    },
    deactivateSpecialTab(state, action: PayloadAction<keyof OpenWorkspaceSpecialTabs>) {
      state.specialTabs[action.payload] = false
    },
    setTreeNodes(state, action: PayloadAction<NotiaFileNode[]>) {
      state.treeNodes = action.payload
    },
    setSearchQuery(state, action: PayloadAction<string>) {
      state.searchQuery = action.payload
    },
    setSearchMatchedPaths(state, action: PayloadAction<string[]>) {
      // Without a search every save clears it again: an empty result keeps
      // the same array, so the explorer is not drawn again.
      if (state.searchMatchedPaths.length === 0 && action.payload.length === 0) return
      state.searchMatchedPaths = action.payload
    },
    setIsSearchLoading(state, action: PayloadAction<boolean>) {
      state.isSearchLoading = action.payload
    },
    setPendingCreation(state, action: PayloadAction<DocumentsState['pendingCreation']>) {
      state.pendingCreation = action.payload
    },
    setRenamingPath(state, action: PayloadAction<string | null>) {
      state.renamingPath = action.payload
    },
    setClipboardEntry(state, action: PayloadAction<DocumentsState['clipboardEntry']>) {
      state.clipboardEntry = action.payload
    },
    setContextMenu(state, action: PayloadAction<DocumentsState['contextMenu']>) {
      state.contextMenu = action.payload
    },
    setDialogState(state, action: PayloadAction<DocumentsState['dialogState']>) {
      state.dialogState = action.payload
    },
    resetTabs(state) {
      state.openTabs = []
      state.activeTabPath = HOME_WORKSPACE_TAB_PATH
      state.specialTabs = { graph: false, chat: false, taskManager: false, coldPass: false, meeting: false, finance: false, agenda: false, routine: false, aiActions: false, aiTasks: false, recipes: false, health: false, gym: false, home: true }
      state.pendingCreation = null
      state.renamingPath = null
      state.contextMenu = null
      state.dialogState = null
      state.clipboardEntry = null
      state.searchQuery = ''
      state.searchMatchedPaths = []
      state.isSearchLoading = false
      state.folderLoadError = null
      invalidateMermaidCache()
    },
    closeAllTextDocuments(state) {
      state.openTabs = state.openTabs.filter((tab) => tab.document.viewKind !== 'text')
      invalidateMermaidCache()
    },
    resetForLibrarySwitch(state) {
      state.treeNodes = []
      state.flatFileList = []
      state.pendingCreation = null
      state.renamingPath = null
      state.contextMenu = null
      state.dialogState = null
      state.folderLoadError = null
      state.searchQuery = ''
      state.searchMatchedPaths = []
      state.isSearchLoading = false
      state.clipboardEntry = null
    },
    addLoadingFolderId(state, action: PayloadAction<string>) {
      if (!state.loadingFolderIds.includes(action.payload)) {
        state.loadingFolderIds.push(action.payload)
      }
    },
    removeLoadingFolderId(state, action: PayloadAction<string>) {
      state.loadingFolderIds = state.loadingFolderIds.filter((id) => id !== action.payload)
    },
    setFolderLoadError(state, action: PayloadAction<DocumentsState['folderLoadError']>) {
      state.folderLoadError = action.payload
    },
    setFlatFileList(state, action: PayloadAction<NotiaFlatFileEntry[]>) {
      state.flatFileList = action.payload
    },
  },
})

export const {
  setOpenTabs,
  addOpenTab,
  updateTabSource,
  updateTabSaveStatus,
  updateTabSavedSource,
  removeTabByPath,
  renameTabPath,
  setActiveTabPath,
  setSpecialTabs,
  activateSpecialTab,
  deactivateSpecialTab,
  setTreeNodes,
  setSearchQuery,
  setSearchMatchedPaths,
  setIsSearchLoading,
  setPendingCreation,
  setRenamingPath,
  setClipboardEntry,
  setContextMenu,
  setDialogState,
  resetTabs,
  closeAllTextDocuments,
  resetForLibrarySwitch,
  addLoadingFolderId,
  removeLoadingFolderId,
  setFolderLoadError,
  setFlatFileList,
} = documentsSlice.actions

export default documentsSlice.reducer
