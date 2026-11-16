import { callBackend, subscribeBackend, type Unsubscribe } from '../transport'
import type { AiPreferences } from '../preferences/aiSettingsStorage'
import { resolveAiPreferencesForTransport } from '../preferences/aiSettingsStorage'
import type { StoredChatAttachment, StoredChatDocument, StoredChatMessage } from './chatDocumentStorage'
import type { TaskExecutionStep } from './chatAgentTypes'
import type {
  AgentConfirmationDecision,
  AgentProgressEvent,
  MutationPreview,
  WorkspaceAiSnapshot,
} from '../../types/ai/agentContracts'

/**
 * Chat turn client. The backend reads the chat, picks the messages the agent
 * sees, runs the agent, saves the turn and names new chats; this module sends
 * the message, forwards the streamed events and relays the questions of the
 * agent to the views that answer them.
 */

const BACKEND_EVENT = 'notia:backend-event'
const INTERACTION_EVENT = 'ai-chat-interaction'
const TITLE_EVENT = 'ai-chat-title'
const AGENT_EVENT = 'ai-chat-agent'

export type ChatTurnTarget =
  | { kind: 'saved'; path: string }
  | { kind: 'transient'; messages: StoredChatMessage[] }

export interface ChatTurnInput {
  libraryId: string
  mode: 'chat' | 'meeting' | 'published'
  /** Preferences in use; the backend prefers the library's saved AI settings. */
  preferences?: AiPreferences
  scope?: string
  message: string
  /** Temporary context: the active room or view, or the Meeting transcript. */
  context?: string | null
  attachments?: StoredChatAttachment[]
  promptName?: string
  undoOperationId?: string
  workspace?: WorkspaceAiSnapshot | null
  selection?: {
    scopeKey: string | null
    files: string[]
    mode: 'direct' | 'index'
    keepChatContext: boolean
  }
  libraryUserId?: string
  chat: ChatTurnTarget
  /**
   * Answer the saved chat's last request again: the backend takes it (with
   * its attachments) and replaces the replies after it. `message` only
   * shows it while the turn runs.
   */
  retry?: boolean
}

/** Agent of the chat that is answering. */
export interface ChatAgentSpeaker {
  fileName: string
  name: string
  initials: string
}

type AgentEvent =
  | { requestId: string; phase: 'start'; runRequestId: string; agent: ChatAgentSpeaker }
  | { requestId: string; phase: 'message'; runRequestId: string; message: StoredChatMessage }
  | { requestId: string; phase: 'silent'; runRequestId: string }

/**
 * A command the agent runs in the terminal of the host PC, as its events
 * tell it: `output` holds whole lines and `partial` the line being written.
 */
export interface ChatTerminalRun {
  callId: string
  command: string
  shell: string
  cwd: string
  output: string
  partial: string
  /** What the light agent read in the live output; empty until it speaks. */
  status: string
}

export interface ChatTurnOutcome {
  /** Notia's answer, or the last agent's message. */
  answer: string
  dataChanged: boolean
  document?: StoredChatDocument
  undoneOperationId?: string
  /** Why the turn stopped; what happened until then is saved in `document`. */
  error?: { code: string; message: string; retryable: boolean }
}

export interface ChatTurnHandlers {
  /** An agent of the chat starts answering; the deltas that follow are its own. */
  onAgentStart?: (agent: ChatAgentSpeaker) => void
  /** An agent finished its message, which the backend saves with the turn. */
  onAgentMessage?: (message: StoredChatMessage) => void
  /** Text the agent wrote while it keeps working; the turn saves it too. */
  onAssistantNote?: (text: string) => void
  /** An agent answered nothing. */
  onAgentSilent?: () => void
  onMessageDelta?: (delta: string) => void
  onThinkingDelta?: (delta: string) => void
  onAgentProgress?: (event: AgentProgressEvent) => void
  /** A command starts in the terminal of the host PC. */
  onTerminalStarted?: (run: Pick<ChatTerminalRun, 'callId' | 'command' | 'shell' | 'cwd'>) => void
  /** The light agent's status of the running command. */
  onTerminalStatus?: (callId: string, status: string) => void
  /** Output of the command: lines to append and the line being written. */
  onTerminalOutput?: (callId: string, text: string, partial: string) => void
  /** The command ended; `markdown` is the message the turn saves for it. */
  onTerminalFinished?: (callId: string, markdown: string) => void
  requestClarification?: (question: string, signal: AbortSignal, choices?: string[]) => Promise<string>
  requestConfirmation?: (question: string, signal: AbortSignal, preview?: MutationPreview) => Promise<boolean | AgentConfirmationDecision>
  requestExecutionPlanApproval?: (steps: TaskExecutionStep[], signal: AbortSignal) => Promise<{ approved: boolean; steps?: TaskExecutionStep[]; suggestion?: string; approveAll?: boolean }>
}

export interface ChatTurnHandle {
  requestId: string
  abort: () => void
  promise: Promise<ChatTurnOutcome>
}

type Interaction =
  | { type: 'clarification'; question: string; choices: string[] }
  | { type: 'confirmation'; preview: Omit<MutationPreview, 'hunks' | 'assumptions' | 'risks'> & { hunks: Array<Omit<MutationPreview['hunks'][number], 'status'>> } }
  | { type: 'plan'; title: string; steps: TaskExecutionStep[] }

type InteractionAnswer =
  | { type: 'clarification'; answer: string }
  | { type: 'confirmation'; accepted: boolean; hunkIds: string[]; approveAll?: boolean; suggestion?: string }
  | { type: 'plan'; accepted: boolean; stepIds?: string[]; suggestion?: string; approveAll?: boolean }

interface BackendEventEnvelope {
  requestId: string
  sequence: number
  event: { type: string; [key: string]: unknown }
}

function turnError(error: unknown): Error {
  const value = error as { code?: unknown; message?: unknown } | null
  if (value && typeof value === 'object' && value.code === 'cancelled') {
    return new DOMException('Consulta cancelada.', 'AbortError')
  }
  if (error instanceof Error) return error
  if (typeof error === 'string' && error.trim()) return new Error(error)
  if (value && typeof value.message === 'string' && value.message.trim()) return new Error(value.message)
  return new Error('No se pudo completar la consulta con la IA.')
}

function forwardEvent(requestId: string, envelope: BackendEventEnvelope, handlers: ChatTurnHandlers): void {
  const event = envelope.event
  if (event.type === 'assistant-delta' && typeof event.delta === 'string') handlers.onMessageDelta?.(event.delta)
  if (event.type === 'assistant-note' && typeof event.text === 'string') handlers.onAssistantNote?.(event.text)
  if (event.type === 'thinking-summary' && typeof event.summary === 'string') handlers.onThinkingDelta?.(event.summary)
  if (event.type === 'terminal-started' && typeof event.callId === 'string') {
    handlers.onTerminalStarted?.({
      callId: event.callId,
      command: typeof event.command === 'string' ? event.command : '',
      shell: typeof event.shell === 'string' ? event.shell : '',
      cwd: typeof event.cwd === 'string' ? event.cwd : '',
    })
  }
  if (event.type === 'terminal-output' && typeof event.callId === 'string') {
    handlers.onTerminalOutput?.(event.callId, typeof event.text === 'string' ? event.text : '', typeof event.partial === 'string' ? event.partial : '')
  }
  if (event.type === 'terminal-status' && typeof event.callId === 'string' && typeof event.status === 'string') {
    handlers.onTerminalStatus?.(event.callId, event.status)
  }
  if (event.type === 'terminal-finished' && typeof event.callId === 'string' && typeof event.markdown === 'string') {
    handlers.onTerminalFinished?.(event.callId, event.markdown)
  }
  if (event.type === 'tool-completed' && typeof event.toolName === 'string') {
    handlers.onAgentProgress?.({
      type: 'tool-completed',
      requestId,
      operationId: typeof event.operationId === 'string' ? event.operationId : null,
      round: typeof event.round === 'number' ? event.round : 0,
      toolName: event.toolName,
      ok: event.ok === true,
      changed: typeof event.changed === 'boolean' ? event.changed : null,
    })
  }
}

async function answerInteraction(
  interaction: Interaction,
  signal: AbortSignal,
  handlers: ChatTurnHandlers,
): Promise<InteractionAnswer | null> {
  if (interaction.type === 'clarification') {
    if (!handlers.requestClarification) return null
    const answer = await handlers.requestClarification(interaction.question, signal, interaction.choices.length ? interaction.choices : undefined)
    return { type: 'clarification', answer }
  }
  if (interaction.type === 'confirmation') {
    if (!handlers.requestConfirmation) return null
    // Hunks start pending; the confirmation view lets the person reject some.
    const preview: MutationPreview = {
      ...interaction.preview,
      hunks: interaction.preview.hunks.map((hunk) => ({ ...hunk, status: 'pending' as const })),
      assumptions: [],
      risks: [],
    }
    const decision = await handlers.requestConfirmation(preview.summary, signal, preview)
    const normalized = typeof decision === 'boolean' ? { accepted: decision } : decision
    return {
      type: 'confirmation',
      accepted: normalized.accepted,
      hunkIds: [...(normalized.hunkIds ?? [])],
      approveAll: 'approveAll' in normalized ? normalized.approveAll : undefined,
      suggestion: 'suggestion' in normalized ? normalized.suggestion : undefined,
    }
  }
  if (!handlers.requestExecutionPlanApproval) return null
  const decision = await handlers.requestExecutionPlanApproval(interaction.steps, signal)
  return { type: 'plan', accepted: decision.approved, stepIds: decision.steps?.map((step) => step.id), suggestion: decision.suggestion, approveAll: decision.approveAll }
}

/** What the backend decided about a message sent while a turn runs. */
export type ChatInterruptDecision = 'cancel' | 'cancel-and-queue' | 'queue'

/**
 * Sends a message typed while the turn `requestId` runs. The backend decides,
 * with a short call to the model, whether it stops the turn and whether the
 * message goes next, and cancels the turn itself; the interface only queues
 * the message when told so.
 */
export async function interjectChatTurn(requestId: string, message: string): Promise<ChatInterruptDecision> {
  const outcome = await callBackend<{ decision: ChatInterruptDecision }>('ai_chat_interject', {
    payload: { requestId, message },
  })
  return outcome.decision
}

/** Starts a chat turn; the promise settles when the backend answered and saved it. */
export function startChatTurn(input: ChatTurnInput, handlers: ChatTurnHandlers = {}): ChatTurnHandle {
  const requestId = crypto.randomUUID()
  const controller = new AbortController()
  // Each agent of the chat runs with its own request id; the turn follows
  // the events of all of them, each with its own sequence.
  const lastSequences = new Map<string, number>([[requestId, 0]])

  const cancel = () => {
    void callBackend('ai_chat_cancel', { payload: { requestId } }).catch(() => undefined)
  }

  const run = async (): Promise<ChatTurnOutcome> => {
    const unlisteners: Unsubscribe[] = []
    try {
      unlisteners.push(await subscribeBackend<BackendEventEnvelope>(BACKEND_EVENT, (envelope) => {
        // Events can arrive twice (live emit plus replay); only newer ones count.
        const lastSequence = lastSequences.get(envelope.requestId)
        if (lastSequence === undefined || envelope.sequence <= lastSequence) return
        lastSequences.set(envelope.requestId, envelope.sequence)
        forwardEvent(envelope.requestId, envelope, handlers)
      }))
      unlisteners.push(await subscribeBackend<AgentEvent>(AGENT_EVENT, (event) => {
        if (event.requestId !== requestId) return
        if (event.phase === 'start') {
          lastSequences.set(event.runRequestId, 0)
          handlers.onAgentStart?.(event.agent)
        } else if (event.phase === 'message') {
          handlers.onAgentMessage?.(event.message)
        } else {
          handlers.onAgentSilent?.()
        }
      }))
      unlisteners.push(await subscribeBackend<{ requestId: string; interaction: Interaction }>(INTERACTION_EVENT, (event) => {
        if (event.requestId !== requestId) return
        void answerInteraction(event.interaction, controller.signal, handlers)
          .then((answer) => answer
            ? callBackend('ai_chat_answer', { payload: { requestId, answer } })
            : cancel())
          .catch(cancel)
      }))
      if (controller.signal.aborted) throw new DOMException('Consulta cancelada.', 'AbortError')
      const { preferences, ...request } = input
      return await callBackend<ChatTurnOutcome>('ai_chat_send', {
        payload: {
          ...request,
          requestId,
          settings: preferences ? resolveAiPreferencesForTransport(preferences) : undefined,
        },
      })
    } catch (error) {
      throw turnError(error)
    } finally {
      unlisteners.forEach((unlisten) => unlisten())
      // A turn a message stopped may end while a question waits: it closes.
      if (!controller.signal.aborted) controller.abort()
    }
  }

  return {
    requestId,
    abort: () => {
      if (controller.signal.aborted) return
      controller.abort()
      cancel()
    },
    promise: run(),
  }
}

/** AI titles the backend saves for new chats. */
export function subscribeChatTitles(
  handler: (event: { libraryId: string; path: string; title: string }) => void,
): Promise<Unsubscribe> {
  return subscribeBackend<{ libraryId: string; path: string; title: string }>(TITLE_EVENT, handler)
}
