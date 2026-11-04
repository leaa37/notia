/* eslint-disable react-refresh/only-export-components */
import { createContext, useCallback, useContext, useLayoutEffect, useState, useSyncExternalStore, type ReactNode } from 'react'
import type { NotiaFileNode, NotiaLibrary } from '../../types/notia'

export interface NotiaActions {
  openFile: (filePath: string) => Promise<void>
  openFileFromView: (filePath: string) => void
  closeTab: (tabPath: string) => void
  closeActiveTab: () => void
  cycleToNextTab: () => void
  toggleFolder: (folderId: string) => void
  toggleSidebar: () => void
  toggleRightChatPanel: () => void
  toggleTheme: () => void
  selectLibrary: (libraryId: string) => void
  activateTab: (tabPath: string) => void
  openSettings: () => void
  openLibraryManager: () => void
  railActionClick: (actionId: string) => void
  headerActionClick: (id: string) => void
  explorerToolClick: (toolId: string) => void
  submitPendingCreation: (name: string) => Promise<void>
  cancelPendingCreation: () => void
  renameSubmit: (path: string, name: string) => Promise<void>
  cancelRename: () => void
  nodeContextMenu: (node: NotiaFileNode, position: { x: number; y: number }) => void
  emptyContextMenu: (position: { x: number; y: number }) => void
  moveNode: (sourcePath: string, targetDirectoryPath: string) => void
  libraryAdded: (library: NotiaLibrary) => Promise<void>
  libraryRemoved: (library: NotiaLibrary) => Promise<void>
  /** New source of a text editor; without `documentPath` it belongs to the active tab. */
  textDocumentChange: (nextSource: string, documentPath?: string) => void
  /** A shared note changed and another editor saves it (nothing left to save here). */
  chatWorkspaceTreeChanged: (pathHint?: string) => void
  windowAction: (action: NotiaWindowAction) => void
  coldPassOpenCredentialModal: () => void
  coldPassImportVault: () => void
  coldPassEditCredential: (index: number, options?: { generate?: boolean }) => void
  coldPassDeleteCredential: (index: number) => Promise<void>
}

export interface NotiaActionsStore {
  getSnapshot: () => NotiaActions
  subscribe: (listener: () => void) => () => void
  update: (actions: NotiaActions) => void
}

function createNotiaActionsStore(initialActions: NotiaActions): NotiaActionsStore {
  let snapshot = initialActions
  const listeners = new Set<() => void>()

  return {
    getSnapshot: () => snapshot,
    subscribe: (listener) => {
      listeners.add(listener)
      return () => listeners.delete(listener)
    },
    update: (actions) => {
      if (actions === snapshot) {
        return
      }
      snapshot = actions
      listeners.forEach((listener) => listener())
    },
  }
}

export const NotiaActionsContext = createContext<NotiaActionsStore | null>(null)

export function NotiaActionsProvider({ actions, children }: { actions: NotiaActions; children: ReactNode }) {
  const [store] = useState(() => createNotiaActionsStore(actions))
  useLayoutEffect(() => {
    store.update(actions)
  }, [actions, store])

  return (
    <NotiaActionsContext.Provider value={store}>
      {children}
    </NotiaActionsContext.Provider>
  )
}

function useActionsStore(): NotiaActionsStore {
  const store = useContext(NotiaActionsContext)
  if (!store) {
    throw new Error('Notia actions store is unavailable')
  }
  return store
}

export function useNotiaActions(): NotiaActions {
  const store = useActionsStore()
  return useSyncExternalStore(store.subscribe, store.getSnapshot, store.getSnapshot)
}

export function useNotiaActionValue<K extends keyof NotiaActions>(actionName: K): NotiaActions[K] {
  const store = useActionsStore()
  const getSnapshot = useCallback(() => store.getSnapshot()[actionName], [actionName, store])
  return useSyncExternalStore(store.subscribe, getSnapshot, getSnapshot)
}
