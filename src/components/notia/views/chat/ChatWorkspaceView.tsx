import { memo, useEffect, useLayoutEffect, useRef, useCallback, useState, type CSSProperties } from 'react'
import { useAppDispatch } from '../../../../store/hooks'
import { useNarrowContainer } from '../../../../hooks/useNarrowContainer'
import { openSettingsToSection } from '../../../../features/ui/uiSlice'
import { useSubmenuEngine } from '../../../../hooks/useSubmenuEngine'
import { NotiaSubmenuPanel } from '../../NotiaSubmenuPanel'
import { useConfirmationEngine } from '../../../../context/confirmation/useConfirmationEngine'
import { AppDialogModal } from '../../AppDialogModal'
import { ChatLibraryFilesModal } from './ChatLibraryFilesModal'
import { ChatThread } from './ChatThread'
import { ChatComposer } from './ChatComposer'
import { ChatHistoryPanel } from './ChatHistoryPanel'
import { ChatPanelHeader, type ChatPanelAgent, type ChatPanelAgentLook } from './ChatPanelHeader'
import { ChatPanelWelcome } from './ChatPanelWelcome'
import { ChatContextPanel, ChatStarterCards, ChatTopBar, ChatWelcomeHero } from './ChatWorkspacePanels'
import { ChatPhoneAgentsBar, ChatPhoneHeader, ChatPhoneWelcome } from './ChatPhone'
import {
  ChatPhoneAgentsSheet,
  ChatPhoneAttachSheet,
  ChatPhoneContextSheet,
  ChatPhoneDynamicSheet,
  type ChatPhoneSheetKind,
} from './ChatPhoneSheets'
import { ChatPhoneHistoryDrawer } from './ChatPhoneHistory'
import { useKeyboardInset } from './useKeyboardInset'
import { chatAgentLookOf } from './useChatAgentSettings'
import './chatPhone.css'
import { buildAttachmentDisplayName } from '../../../../services/chat/chatAttachmentRuntime'
import { deleteChatDraftFile } from '../../../../services/chat/chatSessionStorage'
import { writeAgentMemories } from '../../../../services/ai/agentPromptRuntime'
import { readChatFileAsAttachment } from './chatImageAttachment'
import { subscribeToChatPanelRequests, type ChatPanelRequest } from '../../../../services/chat/chatComposerRequests'
import { useChatState } from './useChatState'
import { useChatSubmitMessage } from './useChatSubmitMessage'
import { useChatAgentSettings } from './useChatAgentSettings'
import { ChatAgentPanelSections } from './ChatAgentPanel'
import type { ChatAgentSpeaker, ChatTerminalRun } from '../../../../services/chat/aiChatRuntime'
import { useChatAttachmentMenu } from './useChatAttachmentMenu'
import { beginPhantomClickSuppression } from '../../../../utils/interactions/phantomClickSuppression'
import { notiaTimer } from '../../../../services/runtime/notiaLogger'
import {
  loadAgentPromptSelection,
  saveSelectedAgentPromptFileName,
  type AgentPromptOption,
} from '../../../../services/ai/agentPromptRuntime'
import type { ChatWorkspaceViewProps, QueuedChatMessage } from './ChatWorkspaceViewTypes'
import { DEFAULT_CHAT_STARTERS } from './chatStarters'
import type { TaskExecutionStep } from '../../../../services/chat/chatAgentTypes'
import type { AgentConfirmationDecision, AgentProgressEvent, MutationPreview } from '../../../../types/ai/agentContracts'
import {
  answerPendingClarification,
  clearPendingClarification,
  loadPendingClarification,
  savePendingClarification,
} from '../../../../services/ai/clarificationPersistence'
import { useWorkspaceAiSnapshot } from '../../hooks/useWorkspaceAiSnapshot'
import { listAiOperationHistory, loadAiOperationDiff, type AiOperationHistoryEntry } from '../../../../services/ai/aiOperationHistory'
import type { AiOperationHistoryDiff } from './ChatThread'

const EMPTY_PREVIOUS_CHATS: Array<{ id: string; title: string; filePath: string }> = []
const EMPTY_CONTEXT_PATHS: string[] = []
/** The context panel starts open only where it fits beside the conversation. */
const CONTEXT_PANEL_DOCKED_QUERY = '(min-width: 1280px)'
/** Width of the view below which the phone boards of the canvas apply. */
const PHONE_LAYOUT_MAX_WIDTH = 600
const CHAT_MENU_WIDTH = 184
const CHAT_MENU_MARGIN = 12

/** What the phone layout shows over the conversation; a sheet may open over another. */
type ChatPhoneSurface = 'history' | ChatPhoneSheetKind

function isNaturalUndoRequest(value: string): boolean {
  const normalized = value.normalize('NFD').replace(/[\u0300-\u036f]/g, '').toLowerCase()
  return /\b(undo|deshace|deshacer|deshacelo|volve atras|volver atras|reverti|revertir|revertelo)\b/.test(normalized)
}

export function ChatWorkspaceViewComponent({
  agentCorpusPaths = EMPTY_CONTEXT_PATHS,
  agentScope = null,
  library,
  aiPreferences,
  previousChats = EMPTY_PREVIOUS_CHATS,
  suggestions = DEFAULT_CHAT_STARTERS,
  showHistoryPanel = true,
  composerContext = null,
  onClosePanel,
  preferredContextPaths = EMPTY_CONTEXT_PATHS,
  preferredContextName = null,
  preferredContextMode = null,
  preferredContextScopeKey = null,
  transientContextPaths = EMPTY_CONTEXT_PATHS,
  transientContextMode = null,
  transientContextSummary = null,
  transientContextContent = transientContextSummary,
  transientContextDisplayPaths = EMPTY_CONTEXT_PATHS,
  onTransientContextPathRemove,
  persistTransientContext = false,
  selectMatchingChatOnly = false,
  onChatCreated,
  onChatDeleted,
  markdownSelection = null,
  activeMarkdownSource = null,
  onActiveMarkdownDocumentChanged,
}: ChatWorkspaceViewProps) {
  const mountTimerRef = useRef(
    notiaTimer('chat', 'ChatWorkspaceView mount', {
      showHistoryPanel,
      selectMatchingChatOnly,
    }),
  )
  useEffect(() => {
    const timer = mountTimerRef.current
    return () => {
      timer.success()
    }
  }, [])

  const chatState = useChatState({
    library,
    aiPreferences,
    previousChats,
    suggestions,
    preferredContextPaths,
    preferredContextName,
    preferredContextMode,
    preferredContextScopeKey,
    transientContextPaths,
    transientContextMode,
    transientContextSummary,
    persistTransientContext,
    selectMatchingChatOnly,
    showHistoryPanel,
  })
  const [agentPromptOptions, setAgentPromptOptions] = useState<AgentPromptOption[]>([
    { fileName: 'default.md', name: 'default' },
  ])
  const [agentPromptFileName, setAgentPromptFileName] = useState('default.md')
  const [newChatAgentMemoryEnabled, setNewChatAgentMemoryEnabled] = useState(true)
  const [isLibraryFoldersModalOpen, setIsLibraryFoldersModalOpen] = useState(false)
  const [isContextPanelOpen, setIsContextPanelOpen] = useState(
    () => typeof window === 'undefined' || window.matchMedia(CONTEXT_PANEL_DOCKED_QUERY).matches,
  )
  // The full chat view follows the phone boards when its own width is a
  // phone's (the side chat keeps its layout).
  const [rootElement, setRootElement] = useState<HTMLElement | null>(null)
  const isPhone = useNarrowContainer(showHistoryPanel ? rootElement : null, PHONE_LAYOUT_MAX_WIDTH) && showHistoryPanel
  const keyboardInset = useKeyboardInset(isPhone ? rootElement : null)
  const [phoneSurfaces, setPhoneSurfaces] = useState<ChatPhoneSurface[]>([])
  const phoneSurface = isPhone ? phoneSurfaces[phoneSurfaces.length - 1] ?? null : null
  const openPhoneSurface = (surface: ChatPhoneSurface) => setPhoneSurfaces([surface])
  const stackPhoneSurface = (surface: ChatPhoneSurface) => setPhoneSurfaces((current) => [...current, surface])
  const closePhoneSurface = () => setPhoneSurfaces((current) => current.slice(0, -1))
  const [pendingAgentQuestion, setPendingAgentQuestion] = useState<{
    question: string
    choices: string[]
  } | null>(null)
  const [pendingAgentAnswer, setPendingAgentAnswer] = useState<string | null>(null)
  const [pendingAgentConfirmation, setPendingAgentConfirmation] = useState<string | null>(null)
  const [pendingAgentPreview, setPendingAgentPreview] = useState<MutationPreview | null>(null)
  const [pendingAgentHunkIds, setPendingAgentHunkIds] = useState<string[]>([])
  const [agentExecutionPlan, setAgentExecutionPlan] = useState<TaskExecutionStep[]>([])
  const [awaitingAgentExecutionPlanApproval, setAwaitingAgentExecutionPlanApproval] = useState(false)
  const [lastAppliedOperationId, setLastAppliedOperationId] = useState<string | null>(null)
  const [aiOperationHistory, setAiOperationHistory] = useState<AiOperationHistoryEntry[]>([])
  const [aiOperationDiff, setAiOperationDiff] = useState<AiOperationHistoryDiff | null>(null)
  const clarificationResolverRef = useRef<((answer: string) => void) | null>(null)
  const rehydratedClarificationRef = useRef(false)
  const confirmationResolverRef = useRef<((decision: AgentConfirmationDecision) => void) | null>(null)
  const planApprovalResolverRef = useRef<((decision: { approved: boolean; suggestion?: string; steps?: TaskExecutionStep[]; approveAll?: boolean }) => void) | null>(null)
  const activeLibraryId = library?.id

  // The plan shown is the one the backend asked to approve in this session.
  useEffect(() => {
    setAgentExecutionPlan([])
  }, [activeLibraryId])

  useEffect(() => {
    if (!library) {
      setAgentPromptOptions([{ fileName: 'default.md', name: 'default' }])
      setAgentPromptFileName('default.md')
      return
    }

    let isCurrent = true
    const refreshAgentPrompts = async () => {
      try {
        const { prompts, selected } = await loadAgentPromptSelection(library)
        if (!isCurrent) return
        setAgentPromptOptions(prompts)
        setAgentPromptFileName(selected)
      } catch {
        if (!isCurrent) return
        setAgentPromptOptions([{ fileName: 'default.md', name: 'default' }])
        setAgentPromptFileName('default.md')
      }
    }

    void refreshAgentPrompts()
    window.addEventListener('focus', refreshAgentPrompts)
    return () => {
      isCurrent = false
      window.removeEventListener('focus', refreshAgentPrompts)
    }
  }, [agentScope, library])

  const {
    selectedChatFilePath,
    setSelectedChatFilePath,
    activeChatDocument,
    setActiveChatDocument,
    isChatLoading,
    setChatTitleOverrides,
    setMatchedPreferredChatFilePath,
    setPendingAutoCreatedChatFilePath,
    pickChat,

    draft,
    setDraft,
    isSubmitting,
    setIsSubmitting,

    isHistoryPanelOpen,
    setIsHistoryPanelOpen,
    dialogMessage,
    setDialogMessage,
    isChatToolsModalOpen,
    setIsChatToolsModalOpen,
    isLibraryFilesModalOpen,
    setIsLibraryFilesModalOpen,
    isClearingAgentMemory,
    setIsClearingAgentMemory,
    chatContextMenuState,
    setChatContextMenuState,
    locallyDeletedChatPaths,
    setLocallyDeletedChatPaths,

    isAttachmentMenuOpen,
    setIsAttachmentMenuOpen,
    attachmentMenuPosition,
    setAttachmentMenuPosition,
    selectedImageAttachments,
    setSelectedImageAttachments,
    selectedLibraryFilePaths,
    setSelectedLibraryFilePaths,
    selectedLibraryFileOptions,
    setSelectedLibraryFileOptions,
    selectedFileContextMode,
    setSelectedFileContextMode,
    selectedLibraryFolderPaths,
    setSelectedLibraryFolderPaths,
    libraryRagEnabled,
    setLibraryRagEnabled,

    streamingThinking,
    setStreamingThinking,
    streamingAssistantMessage,
    setStreamingAssistantMessage,
    setOptimisticThreadMessages,

    isCheckingAiHealth,
    aiAvailabilityMessage,

    displayedMessages,
    effectiveSelectedContextPaths,
    effectiveSelectedContextMode,
    hasTransientContext,
    transientContextSummaryLabel,
    selectedLibraryFileSummary,
    resolvedPreviousChats,
    filteredPreviousChats,
    chatHistoryQuery,
    setChatHistoryQuery,
    availablePreviousChats,
    compactRecentChats,
    visibleSuggestions,
    canSubmit,
    isAiAvailable,
    activeModelLabel,
    isResolvingActiveModel,

    chatHistoryListRef,
    virtualChatHistoryItems,
    chatHistoryTotalSize,
  } = chatState

  const agentSettings = useChatAgentSettings({
    library,
    selectedChatFilePath,
    activeChatDocument,
    setActiveChatDocument,
  })
  // The agent of the chat whose answer is streaming; `null` while Notia answers.
  const [streamingAgent, setStreamingAgent] = useState<ChatAgentSpeaker | null>(null)
  const [streamingTerminal, setStreamingTerminal] = useState<ChatTerminalRun | null>(null)

  // Views ask the side chat to fill its composer (Finanzas), to send a
  // message with an agent or to open a chat (Home). The request waits here
  // until the chat can carry it out.
  const [composerFocusRequest, setComposerFocusRequest] = useState(0)
  const [panelRequest, setPanelRequest] = useState<ChatPanelRequest | null>(null)
  useEffect(() => {
    if (showHistoryPanel) return undefined
    return subscribeToChatPanelRequests(setPanelRequest)
  }, [showHistoryPanel])

  const hidesAttachedFileContext = agentScope === 'task-manager'
  const resolvedSelectedLibraryFilePaths = hidesAttachedFileContext ? EMPTY_CONTEXT_PATHS : selectedLibraryFilePaths
  const resolvedSelectedLibraryFileOptions = hidesAttachedFileContext ? [] : selectedLibraryFileOptions
  const resolvedEffectiveContextPaths = hidesAttachedFileContext ? EMPTY_CONTEXT_PATHS : effectiveSelectedContextPaths
  const resolvedSelectedLibraryFileSummary = hidesAttachedFileContext ? [] : selectedLibraryFileSummary
  const workspaceSnapshot = useWorkspaceAiSnapshot({
    scope: agentScope,
    activeMarkdownSource,
    markdownSelection,
  })
  const activeDocumentPath = workspaceSnapshot?.activeDocument?.path ?? null

  const refreshAiOperationHistory = useCallback(() => {
    if (!library) {
      setAiOperationHistory([])
      return
    }
    void listAiOperationHistory(library).then(setAiOperationHistory).catch(() => setAiOperationHistory([]))
  }, [library])

  useEffect(() => {
    refreshAiOperationHistory()
    setAiOperationDiff(null)
  }, [activeDocumentPath, refreshAiOperationHistory])

  useEffect(() => {
    if (!library) {
      rehydratedClarificationRef.current = false
      return
    }
    let isCurrent = true
    // A document-scoped question waits for the active tab to hydrate; the
    // backend answers `waiting` instead of discarding it.
    void loadPendingClarification(library.id, {
      scope: agentScope ?? 'library',
      documentPath: workspaceSnapshot?.activeDocument?.path ?? null,
      revision: workspaceSnapshot?.activeDocumentRevision ?? null,
    }).then((pending) => {
      if (!isCurrent || pending.status === 'waiting') return
      if (pending.status === 'none') {
        rehydratedClarificationRef.current = false
        if (!clarificationResolverRef.current) setPendingAgentQuestion(null)
        return
      }
      rehydratedClarificationRef.current = true
      if (!clarificationResolverRef.current) {
        setPendingAgentQuestion({ question: pending.question, choices: pending.choices })
        setPendingAgentAnswer(null)
      }
    }).catch(() => undefined)
    return () => { isCurrent = false }
  }, [agentScope, library, workspaceSnapshot?.activeDocument?.path, workspaceSnapshot?.activeDocumentRevision])

  useEffect(() => {
    if (!hidesAttachedFileContext) {
      return
    }

    setSelectedLibraryFilePaths((current) => current.length > 0 ? [] : current)
    setSelectedLibraryFileOptions((current) => current.length > 0 ? [] : current)
    setSelectedFileContextMode((current) => current === 'index' ? current : 'index')
  }, [
    hidesAttachedFileContext,
    selectedFileContextMode,
    selectedLibraryFileOptions.length,
    selectedLibraryFilePaths.length,
    setSelectedFileContextMode,
    setSelectedLibraryFileOptions,
    setSelectedLibraryFilePaths,
  ])

  useEffect(() => {
    if (!isSubmitting) {
      setPendingAgentAnswer(null)
    }
  }, [isSubmitting])

  const { confirm } = useConfirmationEngine()
  const dispatch = useAppDispatch()
  const imageInputRef = useRef<HTMLInputElement | null>(null)
  const chatThreadRef = useRef<HTMLDivElement | null>(null)

  useLayoutEffect(() => {
    const thread = chatThreadRef.current
    if (!thread) {
      return
    }
    thread.scrollTop = thread.scrollHeight
  }, [
    displayedMessages.length,
    agentExecutionPlan,
    isSubmitting,
    pendingAgentConfirmation,
    pendingAgentQuestion,
    streamingAssistantMessage,
    streamingThinking,
  ])

  const handleOpenAiSettings = useCallback(() => {
    dispatch(openSettingsToSection('IA'))
  }, [dispatch])

  const workspaceContextFiles = [
    ...resolvedSelectedLibraryFileSummary.map((option) => ({ path: option.path, name: option.name })),
    ...resolvedSelectedLibraryFilePaths
      .filter((path) => !resolvedSelectedLibraryFileSummary.some((option) => option.path === path))
      .map((path) => ({ path, name: buildAttachmentDisplayName(path) })),
  ]
  // An open chat keeps the memory choice it was created with; the switch
  // only decides it for the next new chat.
  const isAgentMemoryChoiceLocked = Boolean(selectedChatFilePath)
  const agentMemoryEnabled = isAgentMemoryChoiceLocked
    ? activeChatDocument?.agentMemoryEnabled ?? true
    : newChatAgentMemoryEnabled

  const workspaceContextFolders = selectedLibraryFolderPaths.map((path) => ({ path, name: buildAttachmentDisplayName(path) }))

  // The side chat's model button shows only the name, as the canvas does.
  const resolvedActiveModel = activeModelLabel
    ?? (isResolvingActiveModel ? 'Buscando modelo…' : 'Modelo por defecto')

  const {
    panelRef: chatContextMenuPanelRef,
  } = useSubmenuEngine<HTMLButtonElement, HTMLDivElement>({
    open: Boolean(chatContextMenuState),
    onClose: () => {
      setChatContextMenuState(null)
    },
  })
  const {
    triggerRef: attachmentMenuTriggerRef,
    panelRef: attachmentMenuPanelRef,
  } = useSubmenuEngine<HTMLButtonElement, HTMLDivElement>({
    open: isAttachmentMenuOpen,
    onClose: () => {
      setIsAttachmentMenuOpen(false)
    },
  })

  useChatAttachmentMenu(
    isAttachmentMenuOpen,
    setAttachmentMenuPosition,
    attachmentMenuTriggerRef,
    attachmentMenuPanelRef,
  )

  const { submitMessage, retryLastReply, cancelActiveReply, interjectMessage } = useChatSubmitMessage(
    {
      agentCorpusPaths,
      agentScope,
      agentPromptFileName,
      beginPhantomClickSuppression,
      requestAgentClarification: (question, signal, choices = []) => new Promise<string>((resolve, reject) => {
        const handleAbort = () => {
          clarificationResolverRef.current = null
          setPendingAgentQuestion(null)
          if (library) void clearPendingClarification(library.id).catch(() => undefined)
          reject(new Error('Se canceló la aclaración solicitada por el agente.'))
        }
        signal.addEventListener('abort', handleAbort, { once: true })
        clarificationResolverRef.current = (answer) => {
          signal.removeEventListener('abort', handleAbort)
          clarificationResolverRef.current = null
          setPendingAgentQuestion(null)
          if (library) void clearPendingClarification(library.id).catch(() => undefined)
          resolve(answer)
        }
        rehydratedClarificationRef.current = false
        setPendingAgentAnswer(null)
        setPendingAgentQuestion({ question, choices })
        if (library) {
          void savePendingClarification(library.id, question, choices, {
            scope: agentScope ?? 'library',
            documentPath: workspaceSnapshot?.activeDocument?.path ?? null,
            revision: workspaceSnapshot?.activeDocumentRevision ?? null,
          }).catch(() => undefined)
        }
        setStreamingThinking('')
        setStreamingAssistantMessage('')
      }),
      requestAgentConfirmation: (question, signal, preview) => new Promise<boolean | AgentConfirmationDecision>((resolve, reject) => {
        const handleAbort = () => {
          confirmationResolverRef.current = null
          // A proposal being written («Proponer otra cosa») goes too.
          clarificationResolverRef.current = null
          setPendingAgentQuestion(null)
          setPendingAgentConfirmation(null)
          setPendingAgentPreview(null)
          setPendingAgentHunkIds([])
          reject(new Error('Se canceló la confirmación solicitada por el agente.'))
        }
        signal.addEventListener('abort', handleAbort, { once: true })
        confirmationResolverRef.current = (decision) => {
          signal.removeEventListener('abort', handleAbort)
          confirmationResolverRef.current = null
          setPendingAgentConfirmation(null)
          setPendingAgentPreview(null)
          setPendingAgentHunkIds([])
          window.setTimeout(() => chatThreadRef.current?.focus(), 0)
          resolve(preview ? decision : decision.accepted)
        }
        setPendingAgentConfirmation(question)
        setPendingAgentPreview(preview ?? null)
        setPendingAgentHunkIds(preview?.hunks.filter((hunk) => hunk.status !== 'rejected').map((hunk) => hunk.id) ?? [])
        setStreamingThinking('')
        setStreamingAssistantMessage('')
      }),
      onAgentExecutionPlanChange: setAgentExecutionPlan,
      onAgentProgress: (event: AgentProgressEvent) => {
        if (event.type === 'phase-changed' && event.phase === 'preparing') {
          setLastAppliedOperationId(null)
          return
        }
        if (agentScope !== 'document' || event.type !== 'tool-completed' || !event.ok || !event.changed || !event.operationId) return
        if (event.toolName === 'undo_ai_operation') {
          setLastAppliedOperationId(null)
          refreshAiOperationHistory()
          return
        }
        setLastAppliedOperationId(event.operationId)
        refreshAiOperationHistory()
      },
      requestAgentExecutionPlanApproval: (steps, signal) => new Promise((resolve, reject) => {
        const handleAbort = () => {
          planApprovalResolverRef.current = null
          clarificationResolverRef.current = null
          setAwaitingAgentExecutionPlanApproval(false)
          setAgentExecutionPlan([])
          setPendingAgentQuestion(null)
          reject(new Error('Se canceló la aprobación del plan del agente.'))
        }
        signal.addEventListener('abort', handleAbort, { once: true })
        // Once decided, the backend adds the plan and the answer to the
        // thread and saves them; the card goes away.
        planApprovalResolverRef.current = (decision) => {
          signal.removeEventListener('abort', handleAbort)
          planApprovalResolverRef.current = null
          setAwaitingAgentExecutionPlanApproval(false)
          setAgentExecutionPlan([])
          resolve(decision)
        }
        setAgentExecutionPlan(steps)
        setAwaitingAgentExecutionPlanApproval(true)
        setStreamingThinking('')
        setStreamingAssistantMessage('')
      }),
      library,
      aiPreferences,
      activeChatDocument,
      selectedChatFilePath,
      effectiveSelectedContextPaths: resolvedEffectiveContextPaths,
      effectiveSelectedContextMode,
      selectedLibraryFilePaths: resolvedSelectedLibraryFilePaths,
      selectedLibraryFileOptions: resolvedSelectedLibraryFileOptions,
      selectedImageAttachments,
      selectedFileContextMode,
      // Folders and turning the library search off belong to the main chat.
      selectedLibraryFolderPaths: showHistoryPanel ? selectedLibraryFolderPaths : [],
      libraryRagEnabled: showHistoryPanel ? libraryRagEnabled : true,
      newChatAgentMemoryEnabled,
      newChatSettings: showHistoryPanel ? agentSettings.newChatSettings : undefined,
      preferredContextScopeKey,
      persistTransientContext,
      hasTransientContext,
      transientContextContent,
      onChatCreated,
      activeMarkdownSource,
      workspaceSnapshot,
      onActiveMarkdownDocumentChanged,
    },
    {
      draft,
      setDraft,
      isSubmitting,
      setIsSubmitting,
      setOptimisticThreadMessages,
      setStreamingThinking,
      setStreamingAssistantMessage,
      setStreamingAgent,
      setStreamingTerminal,
      setSelectedChatFilePath,
      setActiveChatDocument,
      setChatTitleOverrides,
      setSelectedImageAttachments,
      setSelectedLibraryFilePaths,
      setSelectedLibraryFileOptions,
      setSelectedFileContextMode,
      setPendingAutoCreatedChatFilePath,
      setIsAttachmentMenuOpen,
      setDialogMessage,
    },
  )

  // Messages typed while a turn runs, sent in order when it ends. The backend
  // decides for each one whether it stops the turn or waits.
  const [queuedMessages, setQueuedMessages] = useState<QueuedChatMessage[]>([])
  const [isSendingQueuedMessage, setIsSendingQueuedMessage] = useState(false)

  const interjectComposerMessage = async (message: string): Promise<void> => {
    const text = message.trim()
    if (!text) return
    const id = crypto.randomUUID()
    setDraft('')
    setQueuedMessages((current) => [...current, { id, text, deciding: true }])
    // If the decision fails the message still waits: it is never lost.
    const decision = await interjectMessage(text).catch(() => 'queue' as const)
    setQueuedMessages((current) => (decision === 'cancel'
      ? current.filter((item) => item.id !== id)
      : current.map((item) => (item.id === id ? { ...item, deciding: false } : item))))
  }

  useEffect(() => {
    const next = queuedMessages[0]
    if (isSubmitting || isSendingQueuedMessage || !next || next.deciding) return
    setQueuedMessages((current) => current.slice(1))
    setIsSendingQueuedMessage(true)
    void submitMessage(next.text, undefined, true).finally(() => setIsSendingQueuedMessage(false))
  }, [isSendingQueuedMessage, isSubmitting, queuedMessages, submitMessage])

  const submitComposerMessage = (message: string): Promise<void> => {
    if (isSubmitting) return interjectComposerMessage(message)
    const operationId = lastAppliedOperationId && isNaturalUndoRequest(message)
      ? lastAppliedOperationId
      : undefined
    if (operationId) setLastAppliedOperationId(null)
    return submitMessage(message, operationId)
  }

  const consumeRehydratedClarification = async (answer: string): Promise<string | null> => {
    if (!rehydratedClarificationRef.current || !library) return null
    rehydratedClarificationRef.current = false
    return answerPendingClarification(library.id, answer).catch(() => null)
  }

  const handleDeleteChat = async () => {
    if (!chatContextMenuState) {
      return
    }
    const targetChat = chatContextMenuState
    setChatContextMenuState(null)
    await deleteChat(targetChat)
  }

  /** Asks and deletes a chat of the history (full view menu or side panel list). */
  const deleteChat = async (targetChat: { filePath: string; title: string }) => {
    if (!library) {
      return
    }

    const accepted = await confirm({
      title: 'Eliminar chat',
      message: `Se va a eliminar "${targetChat.title}" de forma permanente. No hay forma de recuperarlo despues. ¿Querés continuar?`,
      confirmLabel: 'Eliminar',
      cancelLabel: 'Cancelar',
      tone: 'danger',
    })

    if (!accepted) {
      return
    }

    // The open chat is closed before its file goes away: the list changes
    // while it is deleted, and the chat would be read again after it was gone.
    const wasSelected = selectedChatFilePath === targetChat.filePath
    try {
      if (wasSelected) {
        setSelectedChatFilePath(null)
        setActiveChatDocument(null)
      }
      setLocallyDeletedChatPaths((current) => (
        current.includes(targetChat.filePath) ? current : [...current, targetChat.filePath]
      ))
      await deleteChatDraftFile(targetChat.filePath, library)
      await onChatDeleted?.(targetChat.filePath)
    } catch (error) {
      setLocallyDeletedChatPaths((current) => current.filter((filePath) => filePath !== targetChat.filePath))
      if (wasSelected) setSelectedChatFilePath(targetChat.filePath)
      setDialogMessage(
        error instanceof Error && error.message.trim()
          ? error.message
          : 'No se pudo eliminar el archivo del chat.',
      )
    }
  }

  const handleClearAgentMemory = async () => {
    if (!library || isClearingAgentMemory) {
      return
    }

    const accepted = await confirm({
      title: 'Borrar memoria persistente',
      message: 'Se va a vaciar la memoria persistente del agente. Una vez hecho, no hay vuelta atrás. ¿Querés continuar?',
      confirmLabel: 'Borrar memoria',
      cancelLabel: 'Cancelar',
      tone: 'danger',
    })

    if (!accepted) {
      return
    }

    setIsClearingAgentMemory(true)

    try {
      await writeAgentMemories(library, [])
      setIsChatToolsModalOpen(false)
    } catch (error) {
      setDialogMessage(
        error instanceof Error && error.message.trim()
          ? error.message
          : 'No se pudo vaciar la memoria persistente del agente.',
      )
    } finally {
      setIsClearingAgentMemory(false)
    }
  }

  useEffect(() => {
    const key = 'onChatFileSelected'
    const windowProxy = window as unknown as { [key]?: (files: File[]) => Promise<void> }
    windowProxy[key] = async (files: File[]) => {
      try {
        const attachments = await Promise.all(files.map((file) => readChatFileAsAttachment(file)))
        setSelectedImageAttachments((current) => [...current, ...attachments])
      } catch (error) {
        setDialogMessage(
          error instanceof Error && error.message.trim()
            ? error.message
            : 'No se pudo cargar el archivo seleccionado.',
        )
      }
    }
    return () => {
      delete windowProxy[key]
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  const handleRemoveSelectedFolder = (path: string) => {
    setSelectedLibraryFolderPaths((current) => current.filter((item) => item !== path))
  }

  const handleRemoveSelectedFile = (path: string) => {
    setSelectedLibraryFilePaths((current) => current.filter((item) => item !== path))
  }

  const handleOpenAttachmentMenu = () => {
    if (!library || !isAiAvailable) {
      return
    }
    setIsAttachmentMenuOpen((current) => !current)
  }

  const handleOpenPhoneAttachSheet = () => {
    if (!library || !isAiAvailable) {
      return
    }
    openPhoneSurface('attach')
  }

  const startNewChat = () => {
    // A new chat starts empty; the file is created with its first message.
    setSelectedChatFilePath(null)
    setMatchedPreferredChatFilePath(null)
    setActiveChatDocument(null)
  }

  /** Opens a chat's options (delete) under the button that asked for them. */
  const openChatOptions = (chat: { id: string; filePath: string; title: string }, anchor: DOMRect) => {
    setChatContextMenuState({
      chatId: chat.id,
      filePath: chat.filePath,
      title: chat.title,
      top: anchor.bottom + 4,
      left: Math.min(Math.max(CHAT_MENU_MARGIN, anchor.right - CHAT_MENU_WIDTH), window.innerWidth - CHAT_MENU_WIDTH - CHAT_MENU_MARGIN),
    })
  }

  const handleViewAiOperationDiff = (operationId: string): void => {
    if (!library) return
    void loadAiOperationDiff(library, operationId).then((diff) => {
      if (diff) setAiOperationDiff(diff)
      else setDialogMessage('El cambio es demasiado grande para mostrar su diff, pero el historial conserva su registro.')
    }).catch(() => setDialogMessage('No se pudo leer el diff del cambio.'))
  }

  const lastAssistantMessage = pendingAgentConfirmation
    ?? (awaitingAgentExecutionPlanApproval ? 'Preparé un plan de ejecución. ¿Querés aprobarlo?' : null)
    ?? pendingAgentQuestion?.question
    ?? [...displayedMessages].reverse().find((message) => message.role === 'assistant')?.content
    ?? null

  const isWelcomeState = showHistoryPanel
    && !isCheckingAiHealth
    && !aiAvailabilityMessage
    && !isChatLoading
    && !isSubmitting
    && displayedMessages.length === 0
    && !pendingAgentQuestion
    && !pendingAgentConfirmation
    && agentExecutionPlan.length === 0

  const isPanelWelcomeState = !showHistoryPanel
    && !isCheckingAiHealth
    && !aiAvailabilityMessage
    && !isChatLoading
    && !isSubmitting
    && displayedMessages.length === 0
    && !pendingAgentQuestion
    && !pendingAgentConfirmation
    && agentExecutionPlan.length === 0

  // On a phone the virtual keyboard shrinks the conversation; its end stays in view.
  const isPhoneThreadShown = isPhone && !isWelcomeState
  useEffect(() => {
    const thread = chatThreadRef.current
    if (!isPhoneThreadShown || !thread || typeof ResizeObserver === 'undefined') return undefined
    let lastHeight = thread.clientHeight
    const observer = new ResizeObserver(() => {
      const wasAtEnd = thread.scrollHeight - thread.scrollTop - lastHeight < 24
      if (thread.clientHeight < lastHeight && wasAtEnd) thread.scrollTop = thread.scrollHeight
      lastHeight = thread.clientHeight
    })
    observer.observe(thread)
    return () => observer.disconnect()
  }, [isPhoneThreadShown])

  // The side chat answers with a prompt file of `.agent/promps`; the catalog
  // brings its name, description and initials.
  const panelAgents: ChatPanelAgent[] = !agentScope ? [] : agentSettings.catalog.agents.length > 0
    ? agentSettings.catalog.agents.map((agent, index) => ({
      fileName: agent.fileName,
      name: agent.name,
      description: agent.description,
      initials: agent.initials,
      colorIndex: index % 6,
    }))
    : agentPromptOptions.map((option, index) => ({
      fileName: option.fileName,
      name: option.name,
      description: '',
      initials: option.name.slice(0, 2).toUpperCase(),
      colorIndex: index % 6,
    }))
  const panelAgent = panelAgents.find((agent) => agent.fileName === agentPromptFileName) ?? panelAgents[0] ?? null
  // A string, so the effect below depends on the names and not on a new array each render.
  const panelAgentFileNames = panelAgents.map((agent) => agent.fileName).join('\n')
  const panelAgentLookOf = (agentFile: string | null): ChatPanelAgentLook => {
    const fileName = agentFile ?? 'default.md'
    const agent = panelAgents.find((candidate) => candidate.fileName === fileName)
    if (agent) return agent
    const name = fileName.replace(/\.md$/i, '')
    return { name, initials: name.slice(0, 2).toUpperCase(), colorIndex: 0 }
  }
  const selectPanelAgent = (nextFileName: string) => {
    setAgentPromptFileName(nextFileName)
    if (library) {
      void saveSelectedAgentPromptFileName(library.id, nextFileName).catch(() => undefined)
    }
  }

  // A message sent from Home starts a new chat with its agent once the
  // running turn ends; each step waits for the render that applies the last.
  useEffect(() => {
    if (!panelRequest) return
    if (panelRequest.kind === 'compose') {
      setPanelRequest(null)
      if (panelRequest.text !== null) setDraft(panelRequest.text)
      setComposerFocusRequest((request) => request + 1)
      return
    }
    if (panelRequest.kind === 'open') {
      setPanelRequest(null)
      pickChat(panelRequest.filePath)
      // A chat opens with the agent it talks with.
      const agentFile = panelRequest.agentFileName
      if (agentFile && agentFile !== agentPromptFileName && panelAgentFileNames.split('\n').includes(agentFile)) {
        setAgentPromptFileName(agentFile)
        if (library) void saveSelectedAgentPromptFileName(library.id, agentFile).catch(() => undefined)
      }
      return
    }
    if (isSubmitting || isChatLoading || !library) return
    if (selectedChatFilePath !== null || activeChatDocument !== null) {
      pickChat(null)
      return
    }
    if (panelRequest.agentFileName && panelRequest.agentFileName !== agentPromptFileName) {
      setAgentPromptFileName(panelRequest.agentFileName)
      return
    }
    setPanelRequest(null)
    void submitMessage(panelRequest.text)
  }, [
    activeChatDocument,
    agentPromptFileName,
    isChatLoading,
    isSubmitting,
    library,
    panelAgentFileNames,
    panelRequest,
    pickChat,
    selectedChatFilePath,
    setDraft,
    submitMessage,
  ])

  const thread = (
    <ChatThread
      messages={displayedMessages}
      isSubmitting={isSubmitting}
      onRetryLastReply={selectedChatFilePath ? () => { void retryLastReply() } : undefined}
      isChatLoading={isChatLoading}
      isCheckingAiHealth={isCheckingAiHealth}
      aiAvailabilityMessage={aiAvailabilityMessage}
      selectedChatFilePath={selectedChatFilePath}
      showHistoryPanel={showHistoryPanel}
      streamingThinking={streamingThinking}
      streamingAssistantMessage={streamingAssistantMessage}
      agentLooks={agentSettings.agentLooks}
      streamingAgent={streamingAgent}
      streamingTerminal={streamingTerminal}
      pendingAgentQuestion={pendingAgentQuestion}
      pendingAgentAnswer={pendingAgentAnswer}
      pendingAgentConfirmation={pendingAgentConfirmation}
      pendingAgentPreview={pendingAgentPreview}
      pendingAgentHunkIds={pendingAgentHunkIds}
      agentExecutionPlan={agentExecutionPlan}
      awaitingAgentExecutionPlanApproval={awaitingAgentExecutionPlanApproval}
      onApproveAgentExecutionPlan={(steps) => planApprovalResolverRef.current?.({ approved: true, steps })}
      onApproveAllAgentExecutionPlan={(steps) => planApprovalResolverRef.current?.({ approved: true, steps, approveAll: true })}
      onCancelAgentExecutionPlan={() => planApprovalResolverRef.current?.({ approved: false })}
      onSuggestAgentExecutionPlanChanges={() => {
        const planResolver = planApprovalResolverRef.current
        if (!planResolver) return
        setAwaitingAgentExecutionPlanApproval(false)
        setPendingAgentQuestion({ question: '¿Qué cambios querés hacerle al TO-DO?', choices: [] })
        clarificationResolverRef.current = (suggestion) => {
          clarificationResolverRef.current = null
          setPendingAgentQuestion(null)
          planResolver({ approved: false, suggestion })
        }
      }}
      lastAppliedOperationId={lastAppliedOperationId}
      aiOperationHistory={activeDocumentPath
        ? aiOperationHistory.filter((entry) => entry.documentPath === activeDocumentPath)
        : []}
      aiOperationDiff={aiOperationDiff}
      onViewAiOperationDiff={handleViewAiOperationDiff}
      onCloseAiOperationDiff={() => setAiOperationDiff(null)}
      onUndoAiOperation={(operationId) => {
        if (isSubmitting) return
        setLastAppliedOperationId(null)
        void submitMessage('Volvé atrás el cambio de IA seleccionado.', operationId)
      }}
      onUndoLastAiOperation={() => {
        if (!lastAppliedOperationId || isSubmitting) return
        const operationId = lastAppliedOperationId
        setLastAppliedOperationId(null)
        void submitMessage('Volvé atrás el último cambio de IA.', operationId)
      }}
      onConfirmAgentAction={() => confirmationResolverRef.current?.({ accepted: true, hunkIds: pendingAgentHunkIds })}
      onConfirmAllAgentActions={() => confirmationResolverRef.current?.({ accepted: true, hunkIds: pendingAgentHunkIds, approveAll: true })}
      onDeclineAgentAction={() => confirmationResolverRef.current?.({ accepted: false })}
      onProposeAgentAlternative={() => {
        // The proposal is written in the question box; the turn goes on
        // with it instead of the change.
        const confirmationResolver = confirmationResolverRef.current
        if (!confirmationResolver) return
        setPendingAgentConfirmation(null)
        setPendingAgentPreview(null)
        setPendingAgentQuestion({ question: '¿Qué querés que haga en su lugar?', choices: [] })
        clarificationResolverRef.current = (suggestion) => {
          clarificationResolverRef.current = null
          setPendingAgentQuestion(null)
          confirmationResolver({ accepted: false, suggestion })
        }
      }}
      onToggleAgentHunk={(hunkId) => {
        setPendingAgentHunkIds((current) => current.includes(hunkId)
          ? current.filter((id) => id !== hunkId)
          : [...current, hunkId])
      }}
      onSelectAgentClarificationOption={(choice) => {
        const resolver = clarificationResolverRef.current
        if (!resolver) {
          setPendingAgentQuestion(null)
          void consumeRehydratedClarification(choice).then((resumePrompt) => {
            if (!resumePrompt) {
              setPendingAgentAnswer('Cancelada')
              return
            }
            setPendingAgentAnswer(choice)
            void submitMessage(resumePrompt)
          })
          return
        }
        setPendingAgentQuestion(null)
        resolver(choice)
      }}
      threadRef={chatThreadRef}
      onOpenAiSettings={handleOpenAiSettings}
    />
  )

  const composer = (
    <ChatComposer
      variant={showHistoryPanel ? 'workspace' : 'panel'}
      libraryRagEnabled={libraryRagEnabled}
      onLibraryRagChange={showHistoryPanel ? setLibraryRagEnabled : undefined}
      libraryRagLabel={isPhone ? 'Librería' : undefined}
      selectedLibraryFolderPaths={showHistoryPanel ? selectedLibraryFolderPaths : []}
      onRemoveFolder={handleRemoveSelectedFolder}
      onOpenLibraryFoldersModal={showHistoryPanel
        ? () => {
          setIsAttachmentMenuOpen(false)
          setIsLibraryFoldersModalOpen(true)
        }
        : undefined}
      draft={draft}
      setDraft={setDraft}
      focusRequest={composerFocusRequest}
      canSubmit={canSubmit || (isSubmitting && Boolean(library) && draft.trim().length > 0)}
      isSubmitting={isSubmitting}
      queuedMessages={queuedMessages}
      onRemoveQueuedMessage={(id) => setQueuedMessages((current) => current.filter((item) => item.id !== id))}
      awaitingAgentClarification={Boolean(pendingAgentQuestion && clarificationResolverRef.current)}
      isAiAvailable={isAiAvailable}
      library={library}
      composerContext={composerContext}
      onOpenModelSettings={handleOpenAiSettings}
      activeModelLabel={resolvedActiveModel}
      selectedImageAttachments={selectedImageAttachments}
      selectedLibraryFileSummary={resolvedSelectedLibraryFileSummary}
      selectedLibraryFilePaths={resolvedSelectedLibraryFilePaths}
      effectiveSelectedContextPaths={resolvedEffectiveContextPaths}
      effectiveSelectedContextMode={effectiveSelectedContextMode}
      transientContextSummaryLabel={transientContextSummaryLabel}
      transientContextDisplayPaths={transientContextDisplayPaths}
      hasTransientContext={hasTransientContext}
      // On a phone the attach options open as a sheet instead of a menu.
      isAttachmentMenuOpen={!isPhone && isAttachmentMenuOpen}
      attachmentMenuPosition={attachmentMenuPosition}
      onRemoveImage={(index) => {
        setSelectedImageAttachments((current) => current.filter((_, currentIndex) => currentIndex !== index))
      }}
      onRemoveFile={handleRemoveSelectedFile}
      onTransientContextPathRemove={onTransientContextPathRemove}
      onToggleAttachmentMenu={isPhone ? handleOpenPhoneAttachSheet : handleOpenAttachmentMenu}
      onSelectImage={() => {
        setIsAttachmentMenuOpen(false)
        imageInputRef.current?.click()
      }}
      onOpenLibraryFilesModal={() => {
        setIsAttachmentMenuOpen(false)
        setIsLibraryFilesModalOpen(true)
      }}
      onSubmit={() => {
        const clarificationResolver = clarificationResolverRef.current
        if (clarificationResolver) {
          const answer = draft.trim()
          if (!answer) return
          // The backend adds the question and this answer to the thread.
          setDraft('')
          clarificationResolver(answer)
          return
        }
        if (rehydratedClarificationRef.current) {
          const answer = draft.trim()
          if (!answer) return
          setPendingAgentAnswer(answer)
          setPendingAgentQuestion(null)
          setDraft('')
          void consumeRehydratedClarification(answer).then((resumePrompt) => {
            if (resumePrompt) void submitMessage(resumePrompt)
          })
          return
        }
        setPendingAgentAnswer(null)
        void submitComposerMessage(draft)
      }}
      onSubmitText={(text) => {
        const normalizedVoiceAnswer = text.normalize('NFD').replace(/[\u0300-\u036f]/g, '').trim().toLowerCase()
        if (confirmationResolverRef.current && /^(si|confirmo|acepto|confirmar)\b/.test(normalizedVoiceAnswer)) {
          confirmationResolverRef.current({ accepted: true, hunkIds: pendingAgentHunkIds })
          return Promise.resolve()
        }
        if (confirmationResolverRef.current && /^(no|cancelo|rechazo|cancelar)\b/.test(normalizedVoiceAnswer)) {
          confirmationResolverRef.current({ accepted: false })
          return Promise.resolve()
        }
        if (planApprovalResolverRef.current && awaitingAgentExecutionPlanApproval && /^(si|apruebo|acepto|confirmo)\b/.test(normalizedVoiceAnswer)) {
          planApprovalResolverRef.current({ approved: true })
          return Promise.resolve()
        }
        const clarificationResolver = clarificationResolverRef.current
        if (clarificationResolver) {
          setPendingAgentQuestion(null)
          clarificationResolver(text)
          return Promise.resolve()
        }
        if (rehydratedClarificationRef.current) {
          setPendingAgentAnswer(text)
          setPendingAgentQuestion(null)
          return consumeRehydratedClarification(text).then((resumePrompt) => (resumePrompt ? submitMessage(resumePrompt) : undefined))
        }
        setPendingAgentAnswer(null)
        return submitComposerMessage(text)
      }}
      lastAssistantMessage={lastAssistantMessage}
      onCancel={cancelActiveReply}
      triggerRef={attachmentMenuTriggerRef}
      panelRef={attachmentMenuPanelRef}
      imageInputRef={imageInputRef}
    />
  )

  const chatSettings = agentSettings.settings
  const updateChatSettings = (next: typeof chatSettings) => void agentSettings.updateSettings(next)
  const areChatSettingsDisabled = !library || isSubmitting
  const phoneDynamicName = chatSettings.dynamic
    ? agentSettings.catalog.dynamics.find((dynamic) => dynamic.fileName === chatSettings.dynamic)?.name
      ?? chatSettings.dynamic.replace(/\.md$/i, '')
    : 'Sin dinámica'

  const phoneMain = isPhone ? (
    <section className="notia-chat-main">
      <ChatPhoneHeader
        title={activeChatDocument?.title ?? 'Nuevo chat'}
        modelLabel={activeModelLabel ?? (isResolvingActiveModel ? 'Resolviendo modelo…' : 'Modelo por defecto')}
        isAiAvailable={isAiAvailable}
        agentCount={chatSettings.agents.length}
        onOpenHistory={() => openPhoneSurface('history')}
        onOpenModelSettings={handleOpenAiSettings}
        onOpenContext={() => openPhoneSurface('context')}
        onNewChat={startNewChat}
      />
      {chatSettings.agents.length > 0 ? (
        <ChatPhoneAgentsBar
          dynamicName={phoneDynamicName}
          agents={chatSettings.agents.map((fileName, index) => chatAgentLookOf(fileName, index, agentSettings.agentLooks))}
          onOpen={() => openPhoneSurface('agents')}
        />
      ) : null}
      {isWelcomeState ? (
        <ChatPhoneWelcome libraryName={library?.name ?? null} starters={visibleSuggestions} onSelectStarter={setDraft} />
      ) : thread}
      <div className="notia-chat-composer-dock">{composer}</div>
    </section>
  ) : null

  const phoneOverlay = phoneSurface === 'history' ? (
    <ChatPhoneHistoryDrawer
      library={library}
      selectedChatFilePath={selectedChatFilePath}
      hiddenChatPaths={locallyDeletedChatPaths}
      onPickChat={(chat) => {
        setSelectedChatFilePath(chat.filePath)
        setPhoneSurfaces([])
      }}
      onNewChat={() => {
        startNewChat()
        setPhoneSurfaces([])
      }}
      onOpenChatOptions={openChatOptions}
      onClose={closePhoneSurface}
    />
  ) : phoneSurface === 'context' ? (
    <ChatPhoneContextSheet
      key="context"
      libraryName={library?.name ?? null}
      libraryRagEnabled={libraryRagEnabled}
      contextFiles={workspaceContextFiles}
      contextFolders={workspaceContextFolders}
      contextMode={selectedFileContextMode}
      starters={visibleSuggestions}
      isDisabled={!library || !isAiAvailable}
      onChooseFiles={() => setIsLibraryFilesModalOpen(true)}
      onChooseFolders={() => setIsLibraryFoldersModalOpen(true)}
      onRemoveFile={handleRemoveSelectedFile}
      onRemoveFolder={handleRemoveSelectedFolder}
      onSelectStarter={(prompt) => {
        setDraft(prompt)
        setPhoneSurfaces([])
      }}
      agentMemoryEnabled={agentMemoryEnabled}
      isAgentMemoryChoiceLocked={isAgentMemoryChoiceLocked}
      onAgentMemoryChange={setNewChatAgentMemoryEnabled}
      onOpenMemory={() => setIsChatToolsModalOpen(true)}
      catalog={agentSettings.catalog}
      settings={chatSettings}
      looks={agentSettings.agentLooks}
      areSettingsDisabled={areChatSettingsDisabled}
      onSettingsChange={updateChatSettings}
      settingsError={agentSettings.settingsError}
      onDismissSettingsError={agentSettings.dismissSettingsError}
      onOpenDynamic={() => stackPhoneSurface('dynamic')}
      onOpenAgents={() => stackPhoneSurface('agents')}
      onClose={closePhoneSurface}
    />
  ) : phoneSurface === 'attach' ? (
    <ChatPhoneAttachSheet
      key="attach"
      libraryName={library?.name ?? null}
      libraryRagEnabled={libraryRagEnabled}
      isRagDisabled={!library || isSubmitting}
      onSelectFile={() => {
        setPhoneSurfaces([])
        imageInputRef.current?.click()
      }}
      onOpenLibraryFiles={() => {
        setPhoneSurfaces([])
        setIsLibraryFilesModalOpen(true)
      }}
      onOpenLibraryFolders={() => {
        setPhoneSurfaces([])
        setIsLibraryFoldersModalOpen(true)
      }}
      onLibraryRagChange={setLibraryRagEnabled}
      onClose={closePhoneSurface}
    />
  ) : phoneSurface === 'agents' ? (
    <ChatPhoneAgentsSheet
      key="agents"
      agents={agentSettings.catalog.agents}
      selected={chatSettings.agents}
      looks={agentSettings.agentLooks}
      isDisabled={areChatSettingsDisabled}
      onChange={(agents) => updateChatSettings({ ...chatSettings, agents })}
      settingsError={agentSettings.settingsError}
      onDismissSettingsError={agentSettings.dismissSettingsError}
      onClose={closePhoneSurface}
    />
  ) : phoneSurface === 'dynamic' ? (
    <ChatPhoneDynamicSheet
      key="dynamic"
      dynamics={agentSettings.catalog.dynamics}
      selected={chatSettings.dynamic}
      isDisabled={areChatSettingsDisabled}
      onSelect={(dynamic) => {
        updateChatSettings({ ...chatSettings, dynamic })
        closePhoneSurface()
      }}
      settingsError={agentSettings.settingsError}
      onDismissSettingsError={agentSettings.dismissSettingsError}
      onClose={closePhoneSurface}
    />
  ) : null

  return (
    <main
      ref={setRootElement}
      className={`notia-main notia-chat-view${showHistoryPanel ? ' notia-chat-view--workspace' : ''}${isPhone ? ' notia-chat-view--phone' : ''}`}
      style={isPhone ? { '--notia-chat-keyboard-inset': `${keyboardInset}px` } as CSSProperties : undefined}
      data-notia-prevent-menu-close
    >
      <section className="notia-chat-shell" data-notia-prevent-menu-close>
        <div className="notia-chat-layout" data-notia-prevent-menu-close>
          {phoneMain}
          {!isPhone && showHistoryPanel ? (
            <ChatHistoryPanel
              library={library}
              selectedChatFilePath={selectedChatFilePath}
              setSelectedChatFilePath={setSelectedChatFilePath}
              onCreateChat={startNewChat}
              setChatContextMenuState={setChatContextMenuState}
              isHistoryPanelOpen={isHistoryPanelOpen}
              setIsHistoryPanelOpen={setIsHistoryPanelOpen}
              resolvedPreviousChats={resolvedPreviousChats}
              filteredPreviousChats={filteredPreviousChats}
              chatHistoryQuery={chatHistoryQuery}
              setChatHistoryQuery={setChatHistoryQuery}
              availablePreviousChats={availablePreviousChats}
              compactRecentChats={compactRecentChats}
              virtualChatHistoryItems={virtualChatHistoryItems}
              chatHistoryTotalSize={chatHistoryTotalSize}
              chatHistoryListRef={chatHistoryListRef}
            />
          ) : null}

          {isPhone ? null : <section className="notia-chat-main">
            {showHistoryPanel ? (
              <ChatTopBar
                title={activeChatDocument?.title ?? 'Nuevo chat'}
                isHistoryPanelOpen={isHistoryPanelOpen}
                onOpenHistory={() => setIsHistoryPanelOpen(true)}
                modelLabel={activeModelLabel}
                isResolvingModel={isResolvingActiveModel}
                isAiAvailable={isAiAvailable}
                onOpenAiSettings={handleOpenAiSettings}
                isAgentMemoryOff={!agentMemoryEnabled}
                isContextPanelOpen={isContextPanelOpen}
                onToggleContextPanel={() => setIsContextPanelOpen((current) => !current)}
              />
            ) : (
              <ChatPanelHeader
                agents={panelAgents}
                selectedAgentFileName={agentPromptFileName}
                isAgentChangeDisabled={isSubmitting}
                onSelectAgent={selectPanelAgent}
                chatTitle={selectedChatFilePath ? activeChatDocument?.title ?? null : null}
                library={library}
                selectedChatFilePath={selectedChatFilePath}
                agentLookOf={panelAgentLookOf}
                onPickChat={(chat) => {
                  pickChat(chat.filePath)
                  // A chat opens with the agent it talks with.
                  const agentFile = chat.agent ?? 'default.md'
                  if (agentFile !== agentPromptFileName && panelAgents.some((agent) => agent.fileName === agentFile)) {
                    selectPanelAgent(agentFile)
                  }
                }}
                onNewChat={() => pickChat(null)}
                onDeleteChat={deleteChat}
                onClose={onClosePanel}
              />
            )}

            {showHistoryPanel ? (
              <div className={`notia-chat-stage${isWelcomeState ? ' notia-chat-stage--welcome' : ''}`}>
                {isWelcomeState ? <ChatWelcomeHero libraryName={library?.name ?? null} /> : thread}
                <div className="notia-chat-composer-dock">{composer}</div>
                {isWelcomeState ? (
                  <ChatStarterCards starters={visibleSuggestions} onSelectStarter={setDraft} />
                ) : null}
              </div>
            ) : (
              <>
                {isPanelWelcomeState ? (
                  <ChatPanelWelcome
                    agent={panelAgent}
                    starters={visibleSuggestions}
                    isDisabled={!library || !isAiAvailable}
                    onSendStarter={(prompt) => void submitComposerMessage(prompt)}
                  />
                ) : thread}
                {composer}
                <p className="notia-chat-panel-hint">Enter para enviar · Shift + Enter para salto de línea</p>
              </>
            )}
          </section>}

          {!isPhone && showHistoryPanel && isContextPanelOpen ? (
            <ChatContextPanel
              libraryName={library?.name ?? null}
              contextFiles={workspaceContextFiles}
              contextFolders={workspaceContextFolders}
              contextMode={selectedFileContextMode}
              libraryRagEnabled={libraryRagEnabled}
              starters={visibleSuggestions}
              isDisabled={!library || !isAiAvailable}
              onChooseFiles={() => setIsLibraryFilesModalOpen(true)}
              onChooseFolders={() => setIsLibraryFoldersModalOpen(true)}
              onRemoveFile={handleRemoveSelectedFile}
              onRemoveFolder={handleRemoveSelectedFolder}
              onSelectStarter={setDraft}
              agentMemoryEnabled={agentMemoryEnabled}
              isAgentMemoryChoiceLocked={isAgentMemoryChoiceLocked}
              onAgentMemoryChange={setNewChatAgentMemoryEnabled}
              onOpenMemory={() => setIsChatToolsModalOpen(true)}
              onClose={() => setIsContextPanelOpen(false)}
            >
              {agentSettings.settingsError ? (
                <p className="notia-chat-context-error" role="alert">
                  {agentSettings.settingsError}
                  <button
                    type="button"
                    className="notia-chat-icon-button notia-chat-icon-button--small"
                    aria-label="Cerrar aviso"
                    onClick={agentSettings.dismissSettingsError}
                  >
                    ×
                  </button>
                </p>
              ) : null}
              <ChatAgentPanelSections
                catalog={agentSettings.catalog}
                settings={agentSettings.settings}
                looks={agentSettings.agentLooks}
                isDisabled={!library || isSubmitting}
                onChange={(next) => void agentSettings.updateSettings(next)}
              />
            </ChatContextPanel>
          ) : null}
        </div>
        {phoneOverlay}

        <ChatLibraryFilesModal
          open={isLibraryFilesModalOpen}
          library={library}
          selectedPaths={selectedLibraryFilePaths}
          contextMode={selectedFileContextMode}
          onClose={() => {
            setIsLibraryFilesModalOpen(false)
          }}
          onApply={({ selectedPaths, selectedOptions, contextMode }) => {
            setSelectedLibraryFilePaths(selectedPaths)
            setSelectedLibraryFileOptions(selectedOptions)
            setSelectedFileContextMode(contextMode)
            setIsLibraryFilesModalOpen(false)
          }}
        />
        <ChatLibraryFilesModal
          kind="folders"
          open={isLibraryFoldersModalOpen}
          library={library}
          selectedPaths={selectedLibraryFolderPaths}
          contextMode={selectedFileContextMode}
          onClose={() => {
            setIsLibraryFoldersModalOpen(false)
          }}
          onApply={({ selectedPaths, contextMode }) => {
            setSelectedLibraryFolderPaths(selectedPaths)
            setSelectedFileContextMode(contextMode)
            setIsLibraryFoldersModalOpen(false)
          }}
        />
        <AppDialogModal
          open={isChatToolsModalOpen}
          title="Memoria del agente"
          message="Notia usa las reglas de .agent/memory/rules.md y la memoria de .agent/memory/memory.md en todos los chats. Agrega una regla cuando le das una instrucción permanente y una memoria cuando mencionás un dato personal o duradero; después ordena memory.md en segundo plano. Borrar la memoria vacía memory.md; las reglas no cambian."
          confirmLabel={isClearingAgentMemory ? 'Borrando...' : 'Borrar memoria'}
          cancelLabel="Cerrar"
          onConfirm={() => {
            void handleClearAgentMemory()
          }}
          onClose={() => {
            if (isClearingAgentMemory) {
              return
            }
            setIsChatToolsModalOpen(false)
          }}
        />
        {chatContextMenuState ? (
          <NotiaSubmenuPanel
            ref={chatContextMenuPanelRef}
            className="notia-chat-context-menu"
            style={{
              position: 'fixed',
              top: `${chatContextMenuState.top}px`,
              left: `${chatContextMenuState.left}px`,
            }}
          >
            <button
              type="button"
              className="notia-chat-context-menu-item notia-chat-context-menu-item--danger"
              onClick={() => {
                void handleDeleteChat()
              }}
            >
              Eliminar chat
            </button>
          </NotiaSubmenuPanel>
        ) : null}
        <AppDialogModal
          open={Boolean(dialogMessage)}
          title="No se pudo completar la accion"
          message={dialogMessage ?? ''}
          onConfirm={() => {
            setDialogMessage(null)
          }}
          onClose={() => {
            setDialogMessage(null)
          }}
        />
      </section>
    </main>
  )
}

function areStringArraysEqual(left: string[], right: string[]): boolean {
  if (left.length !== right.length) {
    return false
  }

  return left.every((value, index) => value === right[index])
}

function arePreviousChatArraysEqual(
  left: Array<{ id: string; title: string; filePath: string }>,
  right: Array<{ id: string; title: string; filePath: string }>,
): boolean {
  if (left.length !== right.length) {
    return false
  }

  return left.every((chat, index) => {
    const candidate = right[index]
    return Boolean(candidate)
      && chat.id === candidate.id
      && chat.filePath === candidate.filePath
      && chat.title === candidate.title
  })
}

function areMarkdownSelectionsEqual(
  left: ChatWorkspaceViewProps['markdownSelection'],
  right: ChatWorkspaceViewProps['markdownSelection'],
): boolean {
  if (left === right) return true
  if (!left || !right) return false
  return left.documentPath === right.documentPath
    && left.from === right.from
    && left.to === right.to
    && left.selectedText === right.selectedText
    && left.blocks.length === right.blocks.length
    && left.blocks.every((block, index) => {
      const other = right.blocks[index]
      return Boolean(other)
        && block.index === other.index
        && block.type === other.type
        && block.text === other.text
        && block.from === other.from
        && block.to === other.to
    })
}

function areChatWorkspaceViewPropsEqual(
  previous: ChatWorkspaceViewProps,
  next: ChatWorkspaceViewProps,
): boolean {
  if (previous.library !== next.library) {
    return false
  }

  if (previous.aiPreferences !== next.aiPreferences) {
    return false
  }

  if (previous.agentScope !== next.agentScope) {
    return false
  }

  if (!areStringArraysEqual(previous.agentCorpusPaths ?? EMPTY_CONTEXT_PATHS, next.agentCorpusPaths ?? EMPTY_CONTEXT_PATHS)) {
    return false
  }

  if (!arePreviousChatArraysEqual(previous.previousChats ?? EMPTY_PREVIOUS_CHATS, next.previousChats ?? EMPTY_PREVIOUS_CHATS)) {
    return false
  }

  if ((previous.suggestions ?? DEFAULT_CHAT_STARTERS) !== (next.suggestions ?? DEFAULT_CHAT_STARTERS)) {
    return false
  }

  if (previous.showHistoryPanel !== next.showHistoryPanel) {
    return false
  }

  if (previous.composerContext?.label !== next.composerContext?.label
    || previous.composerContext?.kind !== next.composerContext?.kind) {
    return false
  }

  if (previous.onClosePanel !== next.onClosePanel) {
    return false
  }

  if (!areStringArraysEqual(previous.preferredContextPaths ?? EMPTY_CONTEXT_PATHS, next.preferredContextPaths ?? EMPTY_CONTEXT_PATHS)) {
    return false
  }

  if (previous.preferredContextName !== next.preferredContextName) {
    return false
  }

  if (previous.preferredContextMode !== next.preferredContextMode) {
    return false
  }

  if (previous.preferredContextScopeKey !== next.preferredContextScopeKey) {
    return false
  }

  if (!areStringArraysEqual(previous.transientContextPaths ?? EMPTY_CONTEXT_PATHS, next.transientContextPaths ?? EMPTY_CONTEXT_PATHS)) {
    return false
  }

  if (previous.transientContextMode !== next.transientContextMode) {
    return false
  }

  if (previous.transientContextSummary !== next.transientContextSummary) {
    return false
  }

  if (previous.transientContextContent !== next.transientContextContent) {
    return false
  }

  if (!areStringArraysEqual(previous.transientContextDisplayPaths ?? EMPTY_CONTEXT_PATHS, next.transientContextDisplayPaths ?? EMPTY_CONTEXT_PATHS)) {
    return false
  }

  if (previous.onTransientContextPathRemove !== next.onTransientContextPathRemove) {
    return false
  }

  if (previous.persistTransientContext !== next.persistTransientContext) {
    return false
  }

  if (previous.selectMatchingChatOnly !== next.selectMatchingChatOnly) {
    return false
  }

  if (previous.onChatCreated !== next.onChatCreated) {
    return false
  }

  if (previous.onChatDeleted !== next.onChatDeleted) {
    return false
  }

  if (previous.onActiveMarkdownDocumentChanged !== next.onActiveMarkdownDocumentChanged) {
    return false
  }

  if (!areMarkdownSelectionsEqual(previous.markdownSelection, next.markdownSelection)) {
    return false
  }

  if (previous.activeMarkdownSource !== next.activeMarkdownSource) {
    return false
  }

  return true
}

export const ChatWorkspaceView = memo(ChatWorkspaceViewComponent, areChatWorkspaceViewPropsEqual)
ChatWorkspaceView.displayName = 'ChatWorkspaceView'
