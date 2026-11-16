import { useEffect, useRef } from 'react'
import { loadChatDocument, type StoredChatDocument, type StoredChatMessage } from '../../../../services/chat/chatDocumentStorage'
import { DEFAULT_CHAT_SETTINGS } from '../../../../services/chat/chatAgentsRuntime'
import { createChatDraftFile } from '../../../../services/chat/chatSessionStorage'
import { checkAiHealth } from '../../../../services/ai/aiRuntime'
import {
  interjectChatTurn,
  startChatTurn,
  subscribeChatTitles,
  type ChatInterruptDecision,
  type ChatTurnHandle,
} from '../../../../services/chat/aiChatRuntime'
import { startPerformanceMeasurement } from '../../../../services/runtime/performanceBaseline'
import { readLibraryDocument } from '../../../../services/libraries/libraryDocumentRuntime'
import { buildAutoCreateChatPayload, normalizeChatTitle } from './useChatState'
import type {
  UseChatSubmitMessageDependencies,
  UseChatSubmitMessageState,
} from './ChatWorkspaceViewTypes'
import { describeAiFeedbackError } from '../../../../services/ai/aiFeedbackRuntime'

/** The live terminal view keeps the end of a long output; the turn saves the transcript. */
const TERMINAL_VIEW_CHARS = 200_000

/**
 * Sends the composer's message as a chat turn. The backend picks the
 * messages the agent sees, runs it, saves the turn and names new chats; this
 * hook shows the turn optimistically, streams the answer and restores the
 * composer when the turn fails. A message sent while the turn runs goes to
 * the backend, which decides whether it stops the turn or waits for the next.
 */
export function useChatSubmitMessage(
  deps: UseChatSubmitMessageDependencies,
  state: UseChatSubmitMessageState,
): {
  submitMessage: (rawMessage: string, undoOperationId?: string, fromQueue?: boolean) => Promise<void>
  retryLastReply: () => Promise<void>
  cancelActiveReply: () => void
  interjectMessage: (rawMessage: string) => Promise<ChatInterruptDecision>
} {
  const {
    agentCorpusPaths,
    agentScope,
    agentPromptFileName,
    requestAgentClarification,
    requestAgentConfirmation,
    onAgentExecutionPlanChange,
    onAgentProgress,
    requestAgentExecutionPlanApproval,
    library,
    aiPreferences,
    activeChatDocument,
    selectedChatFilePath,
    effectiveSelectedContextPaths,
    effectiveSelectedContextMode,
    selectedLibraryFilePaths,
    selectedLibraryFileOptions,
    selectedImageAttachments,
    selectedFileContextMode,
    selectedLibraryFolderPaths = [],
    libraryRagEnabled = true,
    newChatAgentMemoryEnabled = true,
    newChatSettings = DEFAULT_CHAT_SETTINGS,
    preferredContextScopeKey,
    persistTransientContext,
    hasTransientContext,
    transientContextContent,
    activeMarkdownSource,
    workspaceSnapshot,
    onActiveMarkdownDocumentChanged,
    onChatCreated,
  } = deps

  const {
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
  } = state

  const activeReplyRef = useRef<ChatTurnHandle | null>(null)
  const mountedRef = useRef(true)
  const selectedChatFilePathRef = useRef(selectedChatFilePath)
  selectedChatFilePathRef.current = selectedChatFilePath
  const libraryId = library?.id ?? null

  useEffect(() => {
    mountedRef.current = true
    // La solicitud de IA vive en el runtime nativo y puede continuar aunque la
    // interfaz pase a segundo plano (pantalla bloqueada, cambio de app). La
    // cancelación solo ocurre al desmontar o por pedido explícito de la persona.
    const cancelOnPageHide = () => {
      activeReplyRef.current?.abort()
      activeReplyRef.current = null
    }
    window.addEventListener('pagehide', cancelOnPageHide)
    return () => {
      window.removeEventListener('pagehide', cancelOnPageHide)
      mountedRef.current = false
      cancelOnPageHide()
    }
  }, [])

  // The backend names a new chat after its first turn.
  useEffect(() => {
    if (!libraryId) return
    let disposed = false
    let unlisten: (() => void) | null = null
    void subscribeChatTitles((event) => {
      if (event.libraryId !== libraryId) return
      setActiveChatDocument((current) => (
        current && selectedChatFilePathRef.current === event.path ? { ...current, title: event.title } : current
      ))
      setChatTitleOverrides((current) => ({ ...current, [event.path]: normalizeChatTitle(event.title) }))
    }).then((stop) => {
      if (disposed) stop()
      else unlisten = stop
    })
    return () => {
      disposed = true
      unlisten?.()
    }
  }, [libraryId, setActiveChatDocument, setChatTitleOverrides])

  function cancelActiveReply(): void {
    activeReplyRef.current?.abort()
    activeReplyRef.current = null
  }

  /** A message sent while a turn runs; without a running turn it simply waits for the next one. */
  function interjectMessage(rawMessage: string): Promise<ChatInterruptDecision> {
    const message = rawMessage.trim()
    const turn = activeReplyRef.current
    if (!message || !turn) return Promise.resolve('queue')
    return interjectChatTurn(turn.requestId, message)
  }

  /**
   * `fromQueue`: a message the person typed during an earlier turn; the composer keeps what they write now.
   * `retry`: answer the chat's last request again; the backend takes it and replaces its replies, and the
   * thread shows it without the old replies meanwhile. The composer is left as it is.
   */
  const submitMessage = async (rawMessage: string, undoOperationId?: string, fromQueue = false, retry = false) => {
    const retried = retry ? lastRequestOf(activeChatDocument) : null
    if (retry && (!retried || !selectedChatFilePath)) return
    const trimmedMessage = retried ? retried.message.content.trim() : rawMessage.trim()
    // Files alone are a message too; the backend decides what they ask for.
    if ((!retried && !trimmedMessage && (fromQueue || selectedImageAttachments.length === 0)) || isSubmitting || !library) {
      return
    }

    const submitMeasurement = startPerformanceMeasurement('chat.submit_message', {
      chatFilePath: selectedChatFilePath ?? undefined,
      contextFileCount: effectiveSelectedContextPaths.length,
      hasImage: selectedImageAttachments.length > 0,
      libraryId: library.id,
      messageLength: trimmedMessage.length,
    })

    const healthResult = await checkAiHealth(aiPreferences)
    if (!mountedRef.current) return
    if (!healthResult.ok) {
      submitMeasurement.error(new Error(healthResult.message || 'No se pudo conectar con la IA.'), {
        stage: 'health_check',
      })
      setDialogMessage(healthResult.message || 'No se pudo conectar con la IA.')
      return
    }

    const selection = {
      scopeKey: preferredContextScopeKey,
      files: effectiveSelectedContextPaths,
      folders: hasTransientContext ? [] : selectedLibraryFolderPaths,
      mode: effectiveSelectedContextMode,
      libraryRag: libraryRagEnabled,
      keepChatContext: hasTransientContext && !persistTransientContext,
    }
    let targetChatDocument = activeChatDocument
    let targetChatFilePath = selectedChatFilePath

    if (!targetChatDocument || !targetChatFilePath) {
      try {
        const created = await createChatDraftFile(
          library,
          buildAutoCreateChatPayload(newChatAgentMemoryEnabled),
          selection,
          newChatSettings,
        )
        if (!mountedRef.current) return
        setPendingAutoCreatedChatFilePath(created.filePath)
        setSelectedChatFilePath(created.filePath)
        await onChatCreated?.(created.filePath)
        if (!mountedRef.current) return
        targetChatDocument = created.document
        targetChatFilePath = created.filePath
      } catch (error) {
        submitMeasurement.error(error, {
          stage: 'create_chat',
        })
        setDialogMessage(
          error instanceof Error && error.message.trim()
            ? error.message
            : 'No se pudo crear el chat automáticamente.',
        )
        return
      }
    }

    const userMessage: StoredChatMessage = retried?.message ?? {
      role: 'user',
      content: trimmedMessage,
      ...(selectedImageAttachments.length > 0 ? { attachments: selectedImageAttachments } : {}),
    }
    const previousImageAttachments = selectedImageAttachments
    const previousLibraryFilePaths = selectedLibraryFilePaths
    const previousLibraryFileOptions = selectedLibraryFileOptions
    const previousFileContextMode = selectedFileContextMode
    const previousChatDocument: StoredChatDocument = targetChatDocument
    const optimisticMessages = [...(retried ? targetChatDocument.messages.slice(0, retried.index) : targetChatDocument.messages), userMessage]
    const previousDraft = draft
    // Messages of the chat's agents, shown as each one finishes.
    let turnMessages = optimisticMessages
    let agentMessageCount = 0

    if (!fromQueue && !retried) setDraft('')
    setIsSubmitting(true)
    setOptimisticThreadMessages(optimisticMessages)
    setIsAttachmentMenuOpen(false)
    setStreamingThinking('')
    setStreamingAssistantMessage('')
    if (!retried) setSelectedImageAttachments([])
    setActiveChatDocument({ ...targetChatDocument, messages: optimisticMessages })

    const aiReplyMeasurement = startPerformanceMeasurement('chat.ai_reply', {
      contextFileCount: effectiveSelectedContextPaths.length,
      hasImage: selectedImageAttachments.length > 0,
      libraryId: library.id,
      messageLength: trimmedMessage.length,
    })

    try {
      const effectiveAgentScope = agentScope ?? 'library'
      onAgentExecutionPlanChange([])
      const turn = startChatTurn({
        libraryId: library.id,
        mode: 'chat',
        preferences: aiPreferences,
        scope: effectiveAgentScope,
        message: trimmedMessage,
        context: transientContextContent,
        attachments: retried ? [] : selectedImageAttachments,
        ...(retried ? { retry: true } : {}),
        promptName: agentPromptFileName || undefined,
        undoOperationId,
        workspace: workspaceSnapshot,
        selection,
        chat: { kind: 'saved', path: targetChatFilePath },
      }, {
        onAgentStart: (agent) => {
          setStreamingAgent?.(agent)
          setStreamingThinking('')
          setStreamingAssistantMessage('')
        },
        onAgentMessage: (message) => {
          agentMessageCount += 1
          turnMessages = [...turnMessages, message]
          setOptimisticThreadMessages(turnMessages)
          setStreamingThinking('')
          setStreamingAssistantMessage('')
        },
        // What the agent wrote before its next step stays in the thread
        // instead of being replaced by the next round.
        onAssistantNote: (text) => {
          agentMessageCount += 1
          turnMessages = [...turnMessages, { role: 'assistant', content: text }]
          setOptimisticThreadMessages(turnMessages)
          setStreamingThinking('')
          setStreamingAssistantMessage('')
        },
        onAgentSilent: () => {
          setStreamingThinking('')
          setStreamingAssistantMessage('')
        },
        onThinkingDelta: (delta) => setStreamingThinking((current) => current + delta),
        onMessageDelta: (delta) => setStreamingAssistantMessage((current) => current + delta),
        onTerminalStarted: (run) => setStreamingTerminal?.({ ...run, output: '', partial: '', status: '' }),
        onTerminalStatus: (callId, status) => setStreamingTerminal?.((current) => (current?.callId === callId ? { ...current, status } : current)),
        onTerminalOutput: (callId, text, partial) => setStreamingTerminal?.((current) => {
          if (current?.callId !== callId) return current
          const output = current.output + text
          return { ...current, output: output.length > TERMINAL_VIEW_CHARS ? output.slice(-TERMINAL_VIEW_CHARS) : output, partial }
        }),
        // The finished command stays in the thread as the message the turn saves.
        onTerminalFinished: (callId, markdown) => {
          agentMessageCount += 1
          turnMessages = [...turnMessages, { role: 'assistant', content: markdown }]
          setOptimisticThreadMessages(turnMessages)
          setStreamingTerminal?.((current) => (current?.callId === callId ? null : current))
        },
        onAgentProgress,
        requestClarification: requestAgentClarification,
        requestConfirmation: requestAgentConfirmation,
        requestExecutionPlanApproval: requestAgentExecutionPlanApproval,
      })
      activeReplyRef.current = turn
      const outcome = await turn.promise
      if (!mountedRef.current) return
      activeReplyRef.current = null
      if (outcome.undoneOperationId) {
        onAgentProgress?.({
          type: 'tool-completed',
          requestId: turn.requestId,
          operationId: outcome.undoneOperationId,
          round: 0,
          toolName: 'undo_ai_operation',
          ok: true,
          changed: true,
        })
      }
      // The open note is reloaded after the reply so the editor shows the agent's edit.
      const activeDocumentPath = effectiveAgentScope === 'document' ? agentCorpusPaths[0] : undefined
      if (outcome.dataChanged && activeDocumentPath && onActiveMarkdownDocumentChanged) {
        const refreshed = await readLibraryDocument(library.id, activeDocumentPath)
        if (refreshed.ok && refreshed.content !== activeMarkdownSource) {
          await onActiveMarkdownDocumentChanged(activeDocumentPath, refreshed.content, refreshed.revision)
        }
      }
      // A turn that stopped after working is saved with a note; the person
      // still learns why it stopped.
      if (outcome.error) aiReplyMeasurement.error(new Error(outcome.error.message))
      else aiReplyMeasurement.success({ responseLength: outcome.answer.length })
      if (outcome.error && outcome.error.code !== 'cancelled') {
        setDialogMessage(describeAiFeedbackError(outcome.error, 'No se pudo completar la consulta con la IA.'))
      }

      const persistedDocument: StoredChatDocument = outcome.document ?? {
        ...targetChatDocument,
        messages: [...optimisticMessages, { role: 'assistant', content: outcome.answer }],
      }
      setActiveChatDocument(persistedDocument)
      setOptimisticThreadMessages(null)
      setStreamingThinking('')
      setStreamingAssistantMessage('')
      if (targetChatFilePath) {
        setChatTitleOverrides((current) => ({
          ...current,
          [targetChatFilePath]: normalizeChatTitle(persistedDocument.title),
        }))
      }
      submitMeasurement.success({
        autoCreatedChat: !selectedChatFilePath,
        contextFileCount: effectiveSelectedContextPaths.length,
        responseLength: outcome.answer.length,
        totalMessageCount: persistedDocument.messages.length,
      })
    } catch (error) {
      activeReplyRef.current = null
      if (!mountedRef.current) return
      aiReplyMeasurement.error(error)
      submitMeasurement.error(error, {
        stage: 'stream_or_persist',
      })
      setPendingAutoCreatedChatFilePath((current) => (targetChatFilePath && current === targetChatFilePath ? null : current))
      setOptimisticThreadMessages(null)
      setStreamingThinking('')
      setStreamingAssistantMessage('')
      if (agentMessageCount > 0 && targetChatFilePath) {
        // The backend saved the message and what the agents said before the
        // error or the cancel: the chat shows that instead of the draft.
        const savedDocument = await loadChatDocument(targetChatFilePath, previousChatDocument.title, library)
          .catch(() => ({ ...previousChatDocument, messages: turnMessages }))
        if (!mountedRef.current) return
        setActiveChatDocument(savedDocument)
      } else {
        // A queued message that failed goes back to an empty composer.
        if (fromQueue) setDraft((current) => (current.trim() ? current : trimmedMessage))
        else setDraft(previousDraft)
        setActiveChatDocument(previousChatDocument)
      }
      setSelectedImageAttachments(previousImageAttachments)
      setSelectedLibraryFilePaths(previousLibraryFilePaths)
      setSelectedLibraryFileOptions(previousLibraryFileOptions)
      setSelectedFileContextMode(previousFileContextMode)
      setDialogMessage(
        describeAiFeedbackError(error, 'No se pudo completar la consulta con la IA.'),
      )
    } finally {
      if (mountedRef.current) {
        setStreamingAgent?.(null)
        setStreamingTerminal?.(null)
        setIsSubmitting(false)
        // The submit button ended a tap; registering the window here lets the
        // tap-to-mount path suppress the phantom native click that follows.
        deps.beginPhantomClickSuppression?.(document.activeElement)
      }
    }
  }

  const retryLastReply = () => submitMessage('', undefined, false, true)

  return { submitMessage, retryLastReply, cancelActiveReply, interjectMessage }
}

/**
 * The request «Reintentar» answers again, as the thread shows it while the
 * turn runs: the last message of the person and where it is. The backend
 * takes the saved one.
 */
function lastRequestOf(document: StoredChatDocument | null): { message: StoredChatMessage; index: number } | null {
  if (!document) return null
  for (let index = document.messages.length - 1; index >= 0; index -= 1) {
    const message = document.messages[index]
    if (message.role === 'user') return { message, index }
  }
  return null
}
