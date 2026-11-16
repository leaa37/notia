import { beforeEach, describe, expect, it, vi } from 'vitest'

const reactMocks = vi.hoisted(() => ({
  cleanups: [] as Array<() => void>,
}))

const mocks = vi.hoisted(() => ({
  createChatDraftFile: vi.fn(),
  checkAiHealth: vi.fn(),
  startChatTurn: vi.fn(),
  subscribeChatTitles: vi.fn(),
  startPerformanceMeasurement: vi.fn(),
  buildAutoCreateChatPayload: vi.fn(),
  normalizeChatTitle: vi.fn(),
}))

vi.mock('react', async () => {
  const actual = await vi.importActual<typeof import('react')>('react')
  return {
    ...actual,
    useRef: <T,>(initial: T) => ({ current: initial }),
    useEffect: (effect: () => (() => void) | void) => {
      const cleanup = effect()
      if (cleanup) reactMocks.cleanups.push(cleanup)
    },
  }
})
vi.mock('../../../../services/chat/chatSessionStorage', () => ({
  createChatDraftFile: mocks.createChatDraftFile,
}))
vi.mock('../../../../services/ai/aiRuntime', () => ({
  checkAiHealth: mocks.checkAiHealth,
}))
vi.mock('../../../../services/chat/aiChatRuntime', () => ({
  startChatTurn: mocks.startChatTurn,
  subscribeChatTitles: mocks.subscribeChatTitles,
}))
vi.mock('../../../../services/runtime/performanceBaseline', () => ({
  startPerformanceMeasurement: mocks.startPerformanceMeasurement,
}))
vi.mock('./useChatState', () => ({
  buildAutoCreateChatPayload: mocks.buildAutoCreateChatPayload,
  normalizeChatTitle: mocks.normalizeChatTitle,
}))

import { useChatSubmitMessage } from './useChatSubmitMessage'

describe('useChatSubmitMessage lifecycle', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    reactMocks.cleanups = []
    Object.defineProperty(globalThis, 'window', {
      configurable: true,
      value: { addEventListener: vi.fn(), removeEventListener: vi.fn() },
    })
    Object.defineProperty(globalThis, 'document', {
      configurable: true,
      value: { visibilityState: 'visible', addEventListener: vi.fn(), removeEventListener: vi.fn() },
    })
    mocks.checkAiHealth.mockResolvedValue({ ok: true, message: '' })
    mocks.subscribeChatTitles.mockResolvedValue(() => undefined)
    mocks.startPerformanceMeasurement.mockReturnValue({
      success: vi.fn(),
      error: vi.fn(),
      cancel: vi.fn(),
    })
  })

  it('cancels the turn and ignores a late answer after the chat surface unmounts', async () => {
    let resolveReply!: (outcome: { answer: string; dataChanged: boolean }) => void
    const abort = vi.fn()
    mocks.startChatTurn.mockReturnValue({
      requestId: 'request-1',
      abort,
      promise: new Promise((resolve) => { resolveReply = resolve }),
    })

    const library = { id: 'library-1', name: 'Vault', path: 'C:/vault' } as never
    const activeChatDocument = {
      title: 'Chat',
      agentMemoryEnabled: true,
      contextMemoryEnabled: true,
      contextMemoryMessageCount: 10,
      contextScopeKey: null,
      selectedContextMode: 'direct' as const,
      selectedContextFiles: [],
      selectedContextFolders: [],
      libraryRagEnabled: true,
      toolsEnabled: true,
      writeEnabled: true,
      permanentContext: '',
      dynamic: null,
      agents: [],
      messages: [],
    }
    const setState = vi.fn()
    const { submitMessage } = useChatSubmitMessage({
      agentCorpusPaths: [],
      agentScope: 'document',
      agentPromptFileName: 'default.md',
      requestAgentClarification: vi.fn(),
      requestAgentConfirmation: vi.fn(),
      onAgentExecutionPlanChange: vi.fn(),
      requestAgentExecutionPlanApproval: vi.fn(),
      library,
      aiPreferences: {
        ollamaUrl: 'http://localhost:11434', apiKey: '', selectedModel: 'qwen3',
        thinkingEnabled: false, thinkingLevel: 'medium',
      },
      activeChatDocument,
      selectedChatFilePath: 'C:/vault/chat.md',
      effectiveSelectedContextPaths: [],
      effectiveSelectedContextMode: 'direct',
      selectedLibraryFilePaths: [],
      selectedLibraryFileOptions: [],
      selectedImageAttachments: [],
      selectedFileContextMode: 'direct',
      preferredContextScopeKey: null,
      persistTransientContext: false,
      hasTransientContext: false,
      activeMarkdownSource: '# Borrador',
      workspaceSnapshot: null,
    }, {
      draft: '',
      setDraft: setState,
      isSubmitting: false,
      setStreamingThinking: setState,
      setStreamingAssistantMessage: setState,
      setOptimisticThreadMessages: setState,
      setSelectedChatFilePath: setState,
      setActiveChatDocument: setState,
      setChatTitleOverrides: setState,
      setSelectedImageAttachments: setState,
      setSelectedLibraryFilePaths: setState,
      setSelectedLibraryFileOptions: setState,
      setSelectedFileContextMode: setState,
      setPendingAutoCreatedChatFilePath: setState,
      setIsAttachmentMenuOpen: setState,
      setDialogMessage: setState,
      setIsSubmitting: setState,
    })

    const submitPromise = submitMessage('Mejorá la selección')
    await vi.waitFor(() => expect(mocks.startChatTurn).toHaveBeenCalled())
    const callsBeforeAnswer = setState.mock.calls.length
    reactMocks.cleanups[0]?.()
    expect(abort).toHaveBeenCalledOnce()
    resolveReply({ answer: 'respuesta tardía', dataChanged: false })
    await submitPromise

    expect(setState.mock.calls.length).toBe(callsBeforeAnswer)
  })

  it('sends the saved chat and the message; the backend picks the history', async () => {
    const attachment = {
      name: 'teoria.png',
      mimeType: 'image/png',
      base64: 'base64-fixture',
      kind: 'image' as const,
    }
    const activeChatDocument = {
      title: 'Chat',
      agentMemoryEnabled: true,
      contextMemoryEnabled: true,
      contextMemoryMessageCount: 10,
      contextScopeKey: null,
      selectedContextMode: 'direct' as const,
      selectedContextFiles: [],
      selectedContextFolders: [],
      libraryRagEnabled: true,
      toolsEnabled: true,
      writeEnabled: true,
      permanentContext: '',
      dynamic: null,
      agents: [],
      messages: [
        { role: 'user' as const, content: 'Analiza esta teoria.', attachments: [attachment] },
        { role: 'assistant' as const, content: 'Voy a revisarla.' },
      ],
    }
    const setState = vi.fn()
    mocks.startChatTurn.mockReturnValue({
      requestId: 'request-2',
      abort: vi.fn(),
      promise: Promise.resolve({ answer: 'Listo.', dataChanged: false }),
    })

    const { submitMessage } = useChatSubmitMessage({
      agentCorpusPaths: [],
      agentScope: 'document',
      agentPromptFileName: 'default.md',
      requestAgentClarification: vi.fn(),
      requestAgentConfirmation: vi.fn(),
      onAgentExecutionPlanChange: vi.fn(),
      requestAgentExecutionPlanApproval: vi.fn(),
      library: { id: 'library-1', name: 'Vault', path: 'C:/vault' } as never,
      aiPreferences: {
        ollamaUrl: 'http://localhost:11434', apiKey: '', selectedModel: 'qwen3',
        thinkingEnabled: false, thinkingLevel: 'medium',
      },
      activeChatDocument,
      selectedChatFilePath: 'C:/vault/chat.md',
      effectiveSelectedContextPaths: [],
      effectiveSelectedContextMode: 'direct',
      selectedLibraryFilePaths: [],
      selectedLibraryFileOptions: [],
      selectedImageAttachments: [],
      selectedFileContextMode: 'direct',
      selectedLibraryFolderPaths: ['C:/vault/notas'],
      libraryRagEnabled: false,
      preferredContextScopeKey: null,
      persistTransientContext: false,
      hasTransientContext: false,
      activeMarkdownSource: '# Borrador',
      workspaceSnapshot: null,
    }, {
      draft: '',
      setDraft: setState,
      isSubmitting: false,
      setStreamingThinking: setState,
      setStreamingAssistantMessage: setState,
      setOptimisticThreadMessages: setState,
      setSelectedChatFilePath: setState,
      setActiveChatDocument: setState,
      setChatTitleOverrides: setState,
      setSelectedImageAttachments: setState,
      setSelectedLibraryFilePaths: setState,
      setSelectedLibraryFileOptions: setState,
      setSelectedFileContextMode: setState,
      setPendingAutoCreatedChatFilePath: setState,
      setIsAttachmentMenuOpen: setState,
      setDialogMessage: setState,
      setIsSubmitting: setState,
    })

    await submitMessage('Hacelo')

    // The backend reads the chat and attaches the files of its history.
    expect(mocks.startChatTurn).toHaveBeenCalledWith(
      expect.objectContaining({
        mode: 'chat',
        message: 'Hacelo',
        scope: 'document',
        chat: { kind: 'saved', path: 'C:/vault/chat.md' },
        // Folders and the library search switch travel to Rust, which expands and enforces them.
        selection: expect.objectContaining({ folders: ['C:/vault/notas'], libraryRag: false }),
      }),
      expect.anything(),
    )
  })

  it('sends files without text and never an empty message without files', async () => {
    const attachment = { name: 'ticket-coto.jpg', mimeType: 'image/jpeg', base64: 'base64-fixture', kind: 'image' as const }
    mocks.startChatTurn.mockReturnValue({
      requestId: 'request-3',
      abort: vi.fn(),
      promise: Promise.resolve({ answer: 'Cargué el ticket.', dataChanged: true }),
    })
    const useSubmitWith = (selectedImageAttachments: typeof attachment[]) => {
      const setState = vi.fn()
      return useChatSubmitMessage({
        agentCorpusPaths: [],
        agentScope: 'library',
        agentPromptFileName: 'default.md',
        requestAgentClarification: vi.fn(),
        requestAgentConfirmation: vi.fn(),
        onAgentExecutionPlanChange: vi.fn(),
        requestAgentExecutionPlanApproval: vi.fn(),
        library: { id: 'library-1', name: 'Vault', path: 'C:/vault' } as never,
        aiPreferences: {
          ollamaUrl: 'http://localhost:11434', apiKey: '', selectedModel: 'qwen3',
          thinkingEnabled: false, thinkingLevel: 'medium',
        },
        activeChatDocument: {
          title: 'Chat', agentMemoryEnabled: true, contextMemoryEnabled: true, contextMemoryMessageCount: 10,
          contextScopeKey: null, selectedContextMode: 'direct' as const, selectedContextFiles: [], selectedContextFolders: [],
          libraryRagEnabled: true, toolsEnabled: true, writeEnabled: true, permanentContext: '', dynamic: null, agents: [], messages: [],
        },
        selectedChatFilePath: 'C:/vault/chat.md',
        effectiveSelectedContextPaths: [],
        effectiveSelectedContextMode: 'direct',
        selectedLibraryFilePaths: [],
        selectedLibraryFileOptions: [],
        selectedImageAttachments,
        selectedFileContextMode: 'direct',
        preferredContextScopeKey: null,
        persistTransientContext: false,
        hasTransientContext: false,
        activeMarkdownSource: null,
        workspaceSnapshot: null,
      }, {
        draft: '', setDraft: setState, isSubmitting: false, setStreamingThinking: setState,
        setStreamingAssistantMessage: setState, setOptimisticThreadMessages: setState, setSelectedChatFilePath: setState,
        setActiveChatDocument: setState, setChatTitleOverrides: setState, setSelectedImageAttachments: setState,
        setSelectedLibraryFilePaths: setState, setSelectedLibraryFileOptions: setState, setSelectedFileContextMode: setState,
        setPendingAutoCreatedChatFilePath: setState, setIsAttachmentMenuOpen: setState, setDialogMessage: setState,
        setIsSubmitting: setState,
      })
    }

    await useSubmitWith([]).submitMessage('   ')
    expect(mocks.startChatTurn).not.toHaveBeenCalled()

    await useSubmitWith([attachment]).submitMessage('')
    expect(mocks.startChatTurn).toHaveBeenCalledWith(
      expect.objectContaining({ message: '', attachments: [attachment] }),
      expect.anything(),
    )
  })

  it('keeps what the agent wrote between steps in the thread instead of replacing it', async () => {
    mocks.startChatTurn.mockImplementation((_request: unknown, handlers: { onAssistantNote?: (text: string) => void }) => {
      handlers.onAssistantNote?.('Reviso las líneas del resumen.')
      return { requestId: 'request-5', abort: vi.fn(), promise: Promise.resolve({ answer: 'Listo.', dataChanged: false }) }
    })
    const setState = vi.fn()
    const setOptimisticThreadMessages = vi.fn()
    const setStreamingAssistantMessage = vi.fn()
    const { submitMessage } = useChatSubmitMessage({
      agentCorpusPaths: [], agentScope: 'library', agentPromptFileName: 'default.md',
      requestAgentClarification: vi.fn(), requestAgentConfirmation: vi.fn(), onAgentExecutionPlanChange: vi.fn(),
      requestAgentExecutionPlanApproval: vi.fn(),
      library: { id: 'library-1', name: 'Vault', path: 'C:/vault' } as never,
      aiPreferences: {
        ollamaUrl: 'http://localhost:11434', apiKey: '', selectedModel: 'qwen3',
        thinkingEnabled: false, thinkingLevel: 'medium',
      },
      activeChatDocument: {
        title: 'Chat', agentMemoryEnabled: true, contextMemoryEnabled: true, contextMemoryMessageCount: 10,
        contextScopeKey: null, selectedContextMode: 'direct' as const, selectedContextFiles: [], selectedContextFolders: [],
        libraryRagEnabled: true, toolsEnabled: true, writeEnabled: true, permanentContext: '', dynamic: null, agents: [], messages: [],
      },
      selectedChatFilePath: 'C:/vault/chat.md',
      effectiveSelectedContextPaths: [], effectiveSelectedContextMode: 'direct', selectedLibraryFilePaths: [],
      selectedLibraryFileOptions: [], selectedImageAttachments: [], selectedFileContextMode: 'direct',
      preferredContextScopeKey: null, persistTransientContext: false, hasTransientContext: false,
      activeMarkdownSource: null, workspaceSnapshot: null,
    }, {
      draft: '', setDraft: setState, isSubmitting: false, setStreamingThinking: setState,
      setStreamingAssistantMessage, setOptimisticThreadMessages, setSelectedChatFilePath: setState,
      setActiveChatDocument: setState, setChatTitleOverrides: setState, setSelectedImageAttachments: setState,
      setSelectedLibraryFilePaths: setState, setSelectedLibraryFileOptions: setState, setSelectedFileContextMode: setState,
      setPendingAutoCreatedChatFilePath: setState, setIsAttachmentMenuOpen: setState, setDialogMessage: setState,
      setIsSubmitting: setState,
    })

    await submitMessage('categorizá las líneas')

    expect(setOptimisticThreadMessages).toHaveBeenCalledWith([
      expect.objectContaining({ role: 'user', content: 'categorizá las líneas' }),
      { role: 'assistant', content: 'Reviso las líneas del resumen.' },
    ])
    expect(setStreamingAssistantMessage).toHaveBeenCalledWith('')
  })

  it('shows a turn saved with an error and keeps the message out of the composer', async () => {
    const emptyChat = {
      title: 'Chat', agentMemoryEnabled: true, contextMemoryEnabled: true, contextMemoryMessageCount: 10,
      contextScopeKey: null, selectedContextMode: 'direct' as const, selectedContextFiles: [], selectedContextFolders: [],
      libraryRagEnabled: true, toolsEnabled: true, writeEnabled: true, permanentContext: '', dynamic: null, agents: [], messages: [],
    }
    const savedChat = {
      ...emptyChat,
      messages: [
        { role: 'user' as const, content: 'dale' },
        { role: 'assistant' as const, content: 'No pude terminar: El movimiento financiero no es válido.' },
      ],
    }
    mocks.startChatTurn.mockReturnValue({
      requestId: 'request-4',
      abort: vi.fn(),
      promise: Promise.resolve({
        answer: savedChat.messages[1].content,
        dataChanged: true,
        document: savedChat,
        error: { code: 'invalid-input', message: 'El movimiento financiero no es válido.', retryable: false },
      }),
    })
    const setState = vi.fn()
    const setDraft = vi.fn()
    const setDialogMessage = vi.fn()
    const setActiveChatDocument = vi.fn()
    const { submitMessage } = useChatSubmitMessage({
      agentCorpusPaths: [], agentScope: 'library', agentPromptFileName: 'default.md',
      requestAgentClarification: vi.fn(), requestAgentConfirmation: vi.fn(), onAgentExecutionPlanChange: vi.fn(),
      requestAgentExecutionPlanApproval: vi.fn(),
      library: { id: 'library-1', name: 'Vault', path: 'C:/vault' } as never,
      aiPreferences: {
        ollamaUrl: 'http://localhost:11434', apiKey: '', selectedModel: 'qwen3',
        thinkingEnabled: false, thinkingLevel: 'medium',
      },
      activeChatDocument: emptyChat,
      selectedChatFilePath: 'C:/vault/chat.md',
      effectiveSelectedContextPaths: [], effectiveSelectedContextMode: 'direct', selectedLibraryFilePaths: [],
      selectedLibraryFileOptions: [], selectedImageAttachments: [], selectedFileContextMode: 'direct',
      preferredContextScopeKey: null, persistTransientContext: false, hasTransientContext: false,
      activeMarkdownSource: null, workspaceSnapshot: null,
    }, {
      draft: 'dale', setDraft, isSubmitting: false, setStreamingThinking: setState,
      setStreamingAssistantMessage: setState, setOptimisticThreadMessages: setState, setSelectedChatFilePath: setState,
      setActiveChatDocument, setChatTitleOverrides: setState, setSelectedImageAttachments: setState,
      setSelectedLibraryFilePaths: setState, setSelectedLibraryFileOptions: setState, setSelectedFileContextMode: setState,
      setPendingAutoCreatedChatFilePath: setState, setIsAttachmentMenuOpen: setState, setDialogMessage,
      setIsSubmitting: setState,
    })

    await submitMessage('dale')

    expect(setActiveChatDocument).toHaveBeenLastCalledWith(savedChat)
    expect(setDialogMessage).toHaveBeenCalledWith('El movimiento financiero no es válido.')
    expect(setDraft).not.toHaveBeenCalledWith('dale')
  })

  it('answers the last request again without touching the composer', async () => {
    const photo = { name: 'plato.png', mimeType: 'image/png', base64: 'base64-fixture', kind: 'image' as const }
    const chat = {
      title: 'Saludo inicial', agentMemoryEnabled: true, contextMemoryEnabled: true, contextMemoryMessageCount: 10,
      contextScopeKey: null, selectedContextMode: 'direct' as const, selectedContextFiles: [], selectedContextFolders: [],
      libraryRagEnabled: true, toolsEnabled: true, writeEnabled: true, permanentContext: '', dynamic: null, agents: [],
      messages: [
        { role: 'user' as const, content: 'hola' },
        { role: 'assistant' as const, content: '¡Hola! ¿En qué te ayudo?' },
        { role: 'user' as const, content: '¿qué puede hacer este módulo?', attachments: [photo] },
        { role: 'assistant' as const, content: 'article 4 hola' },
      ],
    }
    const answered = { ...chat, messages: [...chat.messages.slice(0, 3), { role: 'assistant' as const, content: 'Inicio junta lo de hoy.' }] }
    mocks.startChatTurn.mockReturnValue({
      requestId: 'request-5',
      abort: vi.fn(),
      promise: Promise.resolve({ answer: 'Inicio junta lo de hoy.', dataChanged: false, document: answered }),
    })
    const setState = vi.fn()
    const setDraft = vi.fn()
    const setOptimisticThreadMessages = vi.fn()
    const setSelectedImageAttachments = vi.fn()
    const setActiveChatDocument = vi.fn()
    const { retryLastReply } = useChatSubmitMessage({
      agentCorpusPaths: [], agentScope: 'library', agentPromptFileName: 'default.md',
      requestAgentClarification: vi.fn(), requestAgentConfirmation: vi.fn(), onAgentExecutionPlanChange: vi.fn(),
      requestAgentExecutionPlanApproval: vi.fn(),
      library: { id: 'library-1', name: 'Vault', path: 'C:/vault' } as never,
      aiPreferences: {
        ollamaUrl: 'http://localhost:11434', apiKey: '', selectedModel: 'qwen3',
        thinkingEnabled: false, thinkingLevel: 'medium',
      },
      activeChatDocument: chat,
      selectedChatFilePath: 'C:/vault/chat.md',
      effectiveSelectedContextPaths: [], effectiveSelectedContextMode: 'direct', selectedLibraryFilePaths: [],
      selectedLibraryFileOptions: [], selectedImageAttachments: [], selectedFileContextMode: 'direct',
      preferredContextScopeKey: null, persistTransientContext: false, hasTransientContext: false,
      activeMarkdownSource: null, workspaceSnapshot: null,
    }, {
      draft: 'algo que estoy escribiendo', setDraft, isSubmitting: false, setStreamingThinking: setState,
      setStreamingAssistantMessage: setState, setOptimisticThreadMessages, setSelectedChatFilePath: setState,
      setActiveChatDocument, setChatTitleOverrides: setState, setSelectedImageAttachments,
      setSelectedLibraryFilePaths: setState, setSelectedLibraryFileOptions: setState, setSelectedFileContextMode: setState,
      setPendingAutoCreatedChatFilePath: setState, setIsAttachmentMenuOpen: setState, setDialogMessage: setState,
      setIsSubmitting: setState,
    })

    await retryLastReply()

    // The backend takes the saved request; the payload only says to retry.
    expect(mocks.startChatTurn).toHaveBeenCalledWith(
      expect.objectContaining({ retry: true, message: '¿qué puede hacer este módulo?', attachments: [], chat: { kind: 'saved', path: 'C:/vault/chat.md' } }),
      expect.anything(),
    )
    // Meanwhile the thread shows the request without its old reply.
    expect(setOptimisticThreadMessages).toHaveBeenCalledWith(chat.messages.slice(0, 3))
    expect(setActiveChatDocument).toHaveBeenLastCalledWith(answered)
    expect(setDraft).not.toHaveBeenCalled()
    expect(setSelectedImageAttachments).not.toHaveBeenCalledWith([])
  })
})
