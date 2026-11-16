import { memo, useEffect, useLayoutEffect, useRef, useState } from 'react'
import { Bot, Check, Circle, Copy, Files, LoaderCircle, OctagonX, RotateCcw, Sparkles, SquareTerminal, User2 } from 'lucide-react'
import { ChatMarkdownMessage } from './ChatMarkdownMessage'
import type { StoredChatAttachment, StoredChatMessage } from '../../../../services/chat/chatDocumentStorage'
import type { TaskExecutionStep } from '../../../../services/chat/chatAgentTypes'
import { agentToolLabel } from '../../../../services/ai/agentToolLabels'
import type { MutationPreview } from '../../../../types/ai/agentContracts'
import type { AiOperationHistoryEntry } from '../../../../services/ai/aiOperationHistory'
import type { ChatAgentSpeaker, ChatTerminalRun } from '../../../../services/chat/aiChatRuntime'
import { ChatAgentAvatar } from './ChatAgentPanel'
import type { ChatAgentLook } from './useChatAgentSettings'

interface ChatThreadProps {
  messages: StoredChatMessage[]
  isSubmitting: boolean
  isChatLoading: boolean
  isCheckingAiHealth: boolean
  aiAvailabilityMessage: string | null
  selectedChatFilePath: string | null
  showHistoryPanel: boolean
  streamingThinking: string
  streamingAssistantMessage: string
  /** How each agent of the chat looks, by its prompt file. */
  agentLooks?: Record<string, ChatAgentLook>
  /** Agent whose answer is streaming; `null` while Notia answers. */
  streamingAgent?: ChatAgentSpeaker | null
  /** Command running in the terminal of the host PC, shown live. */
  streamingTerminal?: ChatTerminalRun | null
  pendingAgentQuestion?: { question: string; choices: string[] } | null
  pendingAgentAnswer?: string | null
  pendingAgentConfirmation?: string | null
  pendingAgentPreview?: MutationPreview | null
  pendingAgentHunkIds?: string[]
  agentExecutionPlan?: TaskExecutionStep[]
  awaitingAgentExecutionPlanApproval?: boolean
  onApproveAgentExecutionPlan?: (steps?: TaskExecutionStep[]) => void
  /** Approves the plan and every change the rest of the turn asks for. */
  onApproveAllAgentExecutionPlan?: (steps?: TaskExecutionStep[]) => void
  onCancelAgentExecutionPlan?: () => void
  onSuggestAgentExecutionPlanChanges?: () => void
  lastAppliedOperationId?: string | null
  onUndoLastAiOperation?: () => void
  aiOperationHistory?: AiOperationHistoryEntry[]
  onUndoAiOperation?: (operationId: string) => void
  aiOperationDiff?: AiOperationHistoryDiff | null
  onViewAiOperationDiff?: (operationId: string) => void
  onCloseAiOperationDiff?: () => void
  onConfirmAgentAction?: () => void
  /** Confirms this change and every other one the rest of the turn asks for. */
  onConfirmAllAgentActions?: () => void
  onDeclineAgentAction?: () => void
  /** Declines the change and writes what to do instead. */
  onProposeAgentAlternative?: () => void
  onToggleAgentHunk?: (hunkId: string) => void
  onSelectAgentClarificationOption?: (choice: string) => void
  threadRef: React.RefObject<HTMLDivElement | null>
  onOpenAiSettings?: () => void
  /** Asks the backend to answer the last request again; shown on the last reply. */
  onRetryLastReply?: () => void
}

export interface AiOperationHistoryDiff {
  operationId: string
  summary: string
  files: readonly {
    path: string
    previousSource: string
    nextSource: string
  }[]
}

const COPY_FEEDBACK_MS = 1_600
const NO_AGENT_LOOKS: Record<string, ChatAgentLook> = {}

/** The agent that wrote a message, or `null` for Notia and the person. */
const SHOWN_IMAGE_TYPES = new Set(['image/png', 'image/jpeg', 'image/webp', 'image/gif'])

/** Images the message shows inline: the person's photos and the ones the agent made. */
function isShownImage(attachment: StoredChatAttachment): boolean {
  return attachment.kind === 'image' && Boolean(attachment.base64) && SHOWN_IMAGE_TYPES.has(attachment.mimeType)
}

function agentLookOf(fileName: string | undefined, looks: Record<string, ChatAgentLook>): ChatAgentLook | null {
  if (!fileName) return null
  return looks[fileName] ?? { name: fileName.replace(/\.md$/i, ''), initials: fileName.slice(0, 2).toUpperCase(), colorIndex: 0 }
}

function AssistantAvatar({ agent }: { agent: ChatAgentLook | null }) {
  return agent
    ? <ChatAgentAvatar look={agent} size="small" />
    : <div className="notia-chat-message-avatar" aria-hidden="true"><Sparkles size={16} /></div>
}

function CopyMessageButton({ source, onRetry }: { source: string; onRetry?: () => void }) {
  const [copyState, setCopyState] = useState<'idle' | 'copied' | 'failed'>('idle')

  useEffect(() => {
    if (copyState === 'idle') return
    const timer = window.setTimeout(() => setCopyState('idle'), COPY_FEEDBACK_MS)
    return () => window.clearTimeout(timer)
  }, [copyState])

  const label = copyState === 'copied' ? 'Copiado' : copyState === 'failed' ? 'No se pudo copiar' : 'Copiar respuesta'
  return (
    <div className="notia-chat-message-actions">
      <button
        type="button"
        className="notia-chat-icon-button notia-chat-icon-button--small"
        aria-label={label}
        title={label}
        onClick={() => {
          void navigator.clipboard.writeText(source)
            .then(() => setCopyState('copied'))
            .catch(() => setCopyState('failed'))
        }}
      >
        {copyState === 'copied' ? <Check size={14} /> : <Copy size={14} />}
      </button>
      {onRetry ? (
        <button
          type="button"
          className="notia-chat-icon-button notia-chat-icon-button--small"
          aria-label="Volver a responder"
          title="Pedir otra respuesta a tu último mensaje"
          onClick={onRetry}
        >
          <RotateCcw size={14} />
        </button>
      ) : null}
      <span className="notia-chat-message-actions-status" role="status">
        {copyState === 'idle' ? '' : label}
      </span>
    </div>
  )
}

/** Distance from the bottom under which the live output keeps following new lines. */
const TERMINAL_FOLLOW_PX = 48

/** A command running in the terminal of the host PC; the turn saves it once it ends. */
function TerminalRunBlock({ run }: { run: ChatTerminalRun }) {
  const outputRef = useRef<HTMLPreElement | null>(null)
  const followRef = useRef(true)
  const shown = run.output + run.partial

  useLayoutEffect(() => {
    const output = outputRef.current
    if (output && followRef.current) output.scrollTop = output.scrollHeight
  }, [shown])

  return (
    <article className="notia-chat-message notia-chat-message--assistant">
      <div className="notia-chat-message-avatar" aria-hidden="true">
        <SquareTerminal size={16} />
      </div>
      <div className="notia-chat-message-bubble notia-chat-terminal">
        <span className="notia-chat-message-role">
          <LoaderCircle size={12} className="notia-chat-terminal-spinner" aria-hidden="true" />
          Terminal · ejecutando
        </span>
        <div className="notia-chat-terminal-command">
          <span className="notia-chat-terminal-shell">{run.shell}</span>
          <code>{run.command}</code>
        </div>
        {run.cwd ? <small className="notia-chat-terminal-cwd">{run.cwd}</small> : null}
        {run.status ? (
          <p className="notia-chat-terminal-status" role="status">
            <Sparkles size={13} aria-hidden="true" />
            <span>{run.status}</span>
          </p>
        ) : null}
        <pre
          ref={outputRef}
          className="notia-chat-terminal-output"
          tabIndex={0}
          aria-label="Salida de la terminal"
          aria-live="polite"
          onScroll={(event) => {
            const output = event.currentTarget
            followRef.current = output.scrollHeight - output.scrollTop - output.clientHeight < TERMINAL_FOLLOW_PX
          }}
        >
          {shown || 'Todavía sin salida…'}
        </pre>
      </div>
    </article>
  )
}

function ChatThreadComponent({
  messages,
  isSubmitting,
  isChatLoading,
  isCheckingAiHealth,
  aiAvailabilityMessage,
  selectedChatFilePath,
  showHistoryPanel,
  streamingThinking,
  streamingAssistantMessage,
  agentLooks = NO_AGENT_LOOKS,
  streamingAgent = null,
  streamingTerminal = null,
  pendingAgentQuestion,
  pendingAgentAnswer,
  pendingAgentConfirmation,
  pendingAgentPreview = null,
  pendingAgentHunkIds = [],
  agentExecutionPlan = [],
  awaitingAgentExecutionPlanApproval = false,
  onApproveAgentExecutionPlan,
  onApproveAllAgentExecutionPlan,
  onCancelAgentExecutionPlan,
  onSuggestAgentExecutionPlanChanges,
  lastAppliedOperationId = null,
  onUndoLastAiOperation,
  aiOperationHistory = [],
  onUndoAiOperation,
  aiOperationDiff = null,
  onViewAiOperationDiff,
  onCloseAiOperationDiff,
  onConfirmAgentAction,
  onConfirmAllAgentActions,
  onDeclineAgentAction,
  onProposeAgentAlternative,
  onToggleAgentHunk,
  onSelectAgentClarificationOption,
  threadRef,
  onOpenAiSettings,
  onRetryLastReply,
}: ChatThreadProps) {
  const hasMessages = messages.length > 0
  // Finance, routine and mail previews are applied whole; only note edits choose hunks.
  const canSelectAgentHunks = Boolean(pendingAgentPreview?.allowedActions.includes('apply-selected'))
  const streamingAgentLook: ChatAgentLook | null = streamingAgent
    ? agentLooks[streamingAgent.fileName] ?? { name: streamingAgent.name, initials: streamingAgent.initials, colorIndex: 0 }
    : null
  const thinkingContentRef = useRef<HTMLDivElement | null>(null)
  const [isEditingPlan, setIsEditingPlan] = useState(false)
  const [draftPlan, setDraftPlan] = useState<TaskExecutionStep[]>([])
  // Closing the preview only hides it; the pending action stays until confirmed or cancelled.
  const [isAgentPreviewOpen, setIsAgentPreviewOpen] = useState(true)

  useEffect(() => {
    setIsAgentPreviewOpen(true)
  }, [pendingAgentPreview])

  useEffect(() => {
    if (isEditingPlan) return
    setDraftPlan(agentExecutionPlan.map((step) => ({
      ...step,
      affectedPaths: step.affectedPaths ? [...step.affectedPaths] : [],
      dependsOn: step.dependsOn ? [...step.dependsOn] : [],
    })))
  }, [agentExecutionPlan, isEditingPlan])

  useLayoutEffect(() => {
    const container = thinkingContentRef.current
    if (!container || !streamingThinking) {
      return
    }
    container.scrollTop = container.scrollHeight
  }, [streamingThinking])

  const hasNoSelectedHunk = Boolean(pendingAgentPreview && pendingAgentHunkIds.length === 0)
  // Rendered inside the preview while it is open: on touch screens it is a bottom sheet over the thread.
  const agentConfirmationActions = (
    <div className="notia-chat-agent-confirmation-actions" role="group" aria-label="Confirmar acción del agente">
      <button
        type="button"
        className="notia-chat-agent-confirmation-button is-primary"
        onClick={onConfirmAgentAction}
        disabled={hasNoSelectedHunk}
      >
        {canSelectAgentHunks && pendingAgentPreview && pendingAgentHunkIds.length < pendingAgentPreview.hunks.length ? 'Confirmar seleccionados' : 'Confirmar'}
      </button>
      {onConfirmAllAgentActions ? (
        <button
          type="button"
          className="notia-chat-agent-confirmation-button"
          title="Confirma este cambio y todos los que siga pidiendo en este pedido, sin volver a preguntar"
          onClick={onConfirmAllAgentActions}
          disabled={hasNoSelectedHunk}
        >
          Confirmar todos
        </button>
      ) : null}
      {onProposeAgentAlternative ? (
        <button type="button" className="notia-chat-agent-confirmation-button" onClick={onProposeAgentAlternative}>
          Proponer otra cosa
        </button>
      ) : null}
      {pendingAgentPreview && !isAgentPreviewOpen ? (
        <button type="button" className="notia-chat-agent-confirmation-button" onClick={() => setIsAgentPreviewOpen(true)}>
          Ver cambios
        </button>
      ) : null}
      <button type="button" className="notia-chat-agent-confirmation-button" onClick={onDeclineAgentAction}>
        Cancelar
      </button>
    </div>
  )

  return (
    <section
      ref={threadRef}
      className="notia-chat-thread"
      tabIndex={-1}
      aria-live="polite"
    >
      {isCheckingAiHealth ? (
        <div className="notia-chat-empty">
          <strong>Verificando IA...</strong>
          <p>Esperá un momento antes de abrir el chat.</p>
        </div>
      ) : aiAvailabilityMessage ? (
        <div className="notia-chat-empty">
          <Bot size={18} />
          <strong>La IA no está disponible</strong>
          <p>{aiAvailabilityMessage}</p>
          {onOpenAiSettings ? (
            <button
              type="button"
              className="notia-chat-empty-action"
              onClick={onOpenAiSettings}
            >
              Configurar IA
            </button>
          ) : null}
        </div>
      ) : hasMessages ? (
        <>
          {messages.map((message, index) => {
            const agent = message.role === 'assistant' ? agentLookOf(message.agent, agentLooks) : null
            return (
            <article
              key={`${message.role}-${index}-${message.content.length}`}
              className={`notia-chat-message notia-chat-message--${message.role}`}
            >
              {message.role === 'assistant'
                ? <AssistantAvatar agent={agent} />
                : <div className="notia-chat-message-avatar" aria-hidden="true"><User2 size={16} /></div>}
              <div className="notia-chat-message-bubble">
                <span className="notia-chat-message-role">
                  {message.role === 'assistant' ? agent?.name ?? 'Notia' : 'Vos'}
                </span>
                {message.attachments?.some((attachment) => !isShownImage(attachment)) ? (
                  <div className="notia-chat-message-attachments" role="status" aria-label="Archivos adjuntos conservados en este mensaje">
                    <Files size={14} />
                    <span>{message.attachments.filter((attachment) => !isShownImage(attachment)).map((attachment) => attachment.name).join(', ')}</span>
                  </div>
                ) : null}
                {message.attachments?.some(isShownImage) ? (
                  <div className="notia-chat-message-images">
                    {message.attachments.filter(isShownImage).map((attachment, imageIndex) => (
                      <figure key={`${attachment.name}-${imageIndex}`} className="notia-chat-message-image">
                        <a href={`data:${attachment.mimeType};base64,${attachment.base64}`} download={attachment.name} title={`Guardar ${attachment.name}`}>
                          <img src={`data:${attachment.mimeType};base64,${attachment.base64}`} alt={attachment.name} loading="lazy" />
                        </a>
                        <figcaption>{attachment.name}</figcaption>
                      </figure>
                    ))}
                  </div>
                ) : null}
                <ChatMarkdownMessage source={message.content} />
                {message.role === 'assistant' ? (
                  <CopyMessageButton
                    source={message.content}
                    onRetry={index === messages.length - 1 && !isSubmitting ? onRetryLastReply : undefined}
                  />
                ) : null}
              </div>
            </article>
            )
          })}
          {agentExecutionPlan.length > 0 ? (
            <article className="notia-chat-message notia-chat-message--assistant">
              <div className="notia-chat-message-avatar" aria-hidden="true"><Sparkles size={16} /></div>
              <div className="notia-chat-message-bubble notia-chat-agent-plan">
                <span className="notia-chat-message-role">Plan de ejecución</span>
                <ol className="notia-chat-agent-plan-list">
                  {(isEditingPlan ? draftPlan : agentExecutionPlan).map((step) => (
                    <li key={step.id} className={`is-${step.status}`}>
                      <span className="notia-chat-agent-plan-status" aria-hidden="true">
                        {step.status === 'completed' ? <Check size={14} /> : step.status === 'in-progress' ? <LoaderCircle size={14} /> : step.status === 'blocked' || step.status === 'failed' || step.status === 'cancelled' ? <OctagonX size={14} /> : <Circle size={12} />}
                      </span>
                      <span>
                        <strong>{step.label}</strong>
                        {step.description ? <small>{step.description}</small> : null}
                        {step.affectedPaths?.length ? <small>Archivos: {step.affectedPaths.join(', ')}</small> : null}
                        {step.dependsOn?.length ? <small>Depende de: {step.dependsOn.join(', ')}</small> : null}
                        <small>{step.risk ? `Riesgo: ${step.risk}` : ''}{step.plannedToolName ? ` · ${agentToolLabel(step.plannedToolName)}` : ''}</small>
                      </span>
                    </li>
                  ))}
                </ol>
                {awaitingAgentExecutionPlanApproval && isEditingPlan ? (
                  <div className="notia-chat-agent-plan-editor" aria-label="Editar plan de ejecucion">
                    {draftPlan.map((step, stepIndex) => (
                      <fieldset key={step.id} className="notia-chat-agent-plan-editor-step">
                        <legend>Paso {stepIndex + 1}</legend>
                        <label>
                          Nombre
                          <input value={step.label} onChange={(event) => setDraftPlan((current) => current.map((candidate) => candidate.id === step.id ? { ...candidate, label: event.target.value } : candidate))} />
                        </label>
                        <label>
                          Que hara
                          <textarea value={step.description ?? ''} onChange={(event) => setDraftPlan((current) => current.map((candidate) => candidate.id === step.id ? { ...candidate, description: event.target.value } : candidate))} rows={2} />
                        </label>
                        <label>
                          Archivos o entidades (separados por coma)
                          <input value={(step.affectedPaths ?? []).join(', ')} onChange={(event) => setDraftPlan((current) => current.map((candidate) => candidate.id === step.id ? { ...candidate, affectedPaths: event.target.value.split(',').map((value) => value.trim()).filter(Boolean) } : candidate))} />
                        </label>
                        <div className="notia-chat-agent-plan-editor-grid">
                          <label>
                            Herramienta
                            <input value={step.plannedToolName ?? ''} onChange={(event) => setDraftPlan((current) => current.map((candidate) => candidate.id === step.id ? { ...candidate, plannedToolName: event.target.value } : candidate))} />
                          </label>
                          <label>
                            Riesgo
                            <select value={step.risk ?? 'low'} onChange={(event) => setDraftPlan((current) => current.map((candidate) => candidate.id === step.id ? { ...candidate, risk: event.target.value as TaskExecutionStep['risk'] } : candidate))}>
                              <option value="low">Bajo</option>
                              <option value="medium">Medio</option>
                              <option value="high">Alto</option>
                              <option value="critical">Critico</option>
                            </select>
                          </label>
                        </div>
                        <label>
                          Depende de pasos (IDs separados por coma)
                          <input value={(step.dependsOn ?? []).join(', ')} onChange={(event) => setDraftPlan((current) => current.map((candidate) => candidate.id === step.id ? { ...candidate, dependsOn: event.target.value.split(',').map((value) => value.trim()).filter(Boolean) } : candidate))} />
                        </label>
                      </fieldset>
                    ))}
                  </div>
                ) : null}
                {awaitingAgentExecutionPlanApproval ? (
                  <div className="notia-chat-agent-confirmation-actions" role="group" aria-label="Revisar plan de ejecución">
                    <button type="button" className="notia-chat-agent-confirmation-button is-primary" onClick={() => { onApproveAgentExecutionPlan?.(isEditingPlan ? draftPlan : agentExecutionPlan); setIsEditingPlan(false) }}>
                      {isEditingPlan ? 'Guardar y confirmar' : 'Confirmar'}
                    </button>
                    {onApproveAllAgentExecutionPlan ? (
                      <button
                        type="button"
                        className="notia-chat-agent-confirmation-button"
                        title="Aprueba el plan y todos los cambios de este pedido sin volver a preguntar"
                        onClick={() => { onApproveAllAgentExecutionPlan(isEditingPlan ? draftPlan : agentExecutionPlan); setIsEditingPlan(false) }}
                      >
                        Confirmar todos
                      </button>
                    ) : null}
                    <button type="button" className="notia-chat-agent-confirmation-button" onClick={() => setIsEditingPlan((current) => !current)}>
                      {isEditingPlan ? 'Cerrar editor' : 'Editar plan'}
                    </button>
                    <button type="button" className="notia-chat-agent-confirmation-button" onClick={() => { setIsEditingPlan(false); onSuggestAgentExecutionPlanChanges?.() }}>
                      Proponer otra cosa
                    </button>
                    {onCancelAgentExecutionPlan ? (
                      <button type="button" className="notia-chat-agent-confirmation-button" onClick={() => { setIsEditingPlan(false); onCancelAgentExecutionPlan() }}>
                        Cancelar
                      </button>
                    ) : null}
                  </div>
                ) : null}
              </div>
            </article>
          ) : null}
          {lastAppliedOperationId && onUndoLastAiOperation ? (
            <div className="notia-chat-agent-operation-actions" role="group" aria-label="Acciones del último cambio de IA">
              <span>Último cambio aplicado</span>
              <button type="button" className="notia-chat-agent-confirmation-button" onClick={onUndoLastAiOperation}>
                Deshacer
              </button>
            </div>
          ) : null}
          {aiOperationHistory.length > 0 ? (
            <details className="notia-chat-agent-operation-history">
              <summary>Historial de cambios ({aiOperationHistory.length})</summary>
              <ul>
                {aiOperationHistory.map((entry) => (
                  <li key={entry.operationId}>
                    <div>
                      <strong>{entry.summary}</strong>
                      <small>{entry.documentPath} · {new Date(entry.appliedAt).toLocaleString()}</small>
                      <small>{entry.status === 'undone' ? 'Deshecho' : 'Aplicado'}</small>
                    </div>
                    {entry.status === 'applied' && onUndoAiOperation ? (
                      <button
                        type="button"
                        className="notia-chat-agent-confirmation-button"
                        onClick={() => onUndoAiOperation(entry.operationId)}
                        disabled={isSubmitting}
                      >
                        Deshacer
                      </button>
                    ) : null}
                    {onViewAiOperationDiff ? (
                      <button
                        type="button"
                        className="notia-chat-agent-confirmation-button"
                        onClick={() => onViewAiOperationDiff(entry.operationId)}
                      >
                        Ver diff
                      </button>
                    ) : null}
                  </li>
                ))}
              </ul>
            </details>
          ) : null}
          {aiOperationDiff ? (
            <div className="notia-chat-agent-operation-diff" role="dialog" aria-label="Diff de operación de IA">
              <div className="notia-chat-diff-sheet-header">
                <strong>{aiOperationDiff.summary}</strong>
                <button type="button" className="notia-chat-agent-confirmation-button" onClick={onCloseAiOperationDiff}>
                  Cerrar
                </button>
              </div>
              {aiOperationDiff.files.map((file) => (
                <section key={file.path}>
                  <strong>{file.path}</strong>
                  <div className="notia-chat-agent-operation-diff-columns">
                    <div>
                      <small>Antes</small>
                      <pre>{file.previousSource || '(vacío)'}</pre>
                    </div>
                    <div>
                      <small>Después</small>
                      <pre>{file.nextSource || '(vacío)'}</pre>
                    </div>
                  </div>
                </section>
              ))}
            </div>
          ) : null}
          {isSubmitting || pendingAgentQuestion ? (
            <>
              {pendingAgentQuestion ? (
                <article className="notia-chat-message notia-chat-message--assistant">
                  <div className="notia-chat-message-avatar" aria-hidden="true">
                    <Sparkles size={16} />
                  </div>
                  <div className="notia-chat-message-bubble notia-chat-agent-confirmation">
                    <span className="notia-chat-message-role">Notia · necesita una aclaración</span>
                    <ChatMarkdownMessage source={pendingAgentQuestion.question} />
                    {pendingAgentQuestion.choices.length > 0 ? (
                      <div className="notia-chat-agent-confirmation-actions" role="group" aria-label="Opciones de aclaración">
                        {pendingAgentQuestion.choices.map((choice) => (
                          <button
                            key={choice}
                            type="button"
                            className="notia-chat-agent-confirmation-button"
                            onClick={() => onSelectAgentClarificationOption?.(choice)}
                          >
                            {choice}
                          </button>
                        ))}
                      </div>
                    ) : (
                      <span className="notia-chat-agent-interaction-hint">Respondé usando el campo de mensaje.</span>
                    )}
                  </div>
                </article>
              ) : null}
              {pendingAgentAnswer ? (
                <article className="notia-chat-message notia-chat-message--user">
                  <div className="notia-chat-message-avatar" aria-hidden="true">
                    <User2 size={16} />
                  </div>
                  <div className="notia-chat-message-bubble">
                    <span className="notia-chat-message-role">Vos</span>
                    <ChatMarkdownMessage source={pendingAgentAnswer} />
                  </div>
                </article>
              ) : null}
              {pendingAgentConfirmation ? (
                <article className="notia-chat-message notia-chat-message--assistant">
                  <div className="notia-chat-message-avatar" aria-hidden="true">
                    <Sparkles size={16} />
                  </div>
                  <div className="notia-chat-message-bubble notia-chat-agent-confirmation">
                    <span className="notia-chat-message-role">Notia · requiere confirmación</span>
                    <ChatMarkdownMessage source={pendingAgentConfirmation} />
                    {pendingAgentPreview && isAgentPreviewOpen ? (
                      <div className="notia-chat-diff-preview" role="dialog" aria-modal="true" aria-label="Vista previa de cambios">
                        <div className="notia-chat-diff-sheet-header">
                          <strong>Vista previa de cambios</strong>
                          <button
                            type="button"
                            className="notia-chat-agent-confirmation-button"
                            onClick={() => setIsAgentPreviewOpen(false)}
                            aria-label="Cerrar vista previa sin cancelar"
                          >
                            Cerrar
                          </button>
                        </div>
                        <div className="notia-chat-diff-summary">
                          <strong>{pendingAgentPreview.summary}</strong>
                          {pendingAgentPreview.risk ? <span>Riesgo: {pendingAgentPreview.risk}</span> : null}
                          {pendingAgentPreview.documents.map((document) => (
                            <span key={document.path}>{document.path}</span>
                          ))}
                          <small>Contenido fuera de los hunks: se conserva sin cambios.</small>
                          {pendingAgentPreview.assumptions.length > 0 ? <small>Supuestos: {pendingAgentPreview.assumptions.join(' · ')}</small> : null}
                          {pendingAgentPreview.risks.length > 0 ? <small>Riesgos: {pendingAgentPreview.risks.join(' · ')}</small> : null}
                        </div>
                        <div className="notia-chat-diff-hunks">
                          {pendingAgentPreview.hunks.map((hunk, index) => {
                            const selected = pendingAgentHunkIds.includes(hunk.id)
                            return (
                              <label key={hunk.id} className={`notia-chat-diff-hunk${selected ? ' is-selected' : ''}`}>
                                <span className="notia-chat-diff-hunk-header">
                                  {canSelectAgentHunks ? (
                                    <input
                                      type="checkbox"
                                      checked={selected}
                                      onChange={() => onToggleAgentHunk?.(hunk.id)}
                                    />
                                  ) : null}
                                  <span>Hunk {index + 1} · líneas {hunk.startLine}-{hunk.endLine}</span>
                                </span>
                                <pre className="notia-chat-diff-hunk-old">{hunk.oldText || '(vacío)'}</pre>
                                <pre className="notia-chat-diff-hunk-new">{hunk.newText || '(vacío)'}</pre>
                              </label>
                            )
                          })}
                        </div>
                        {agentConfirmationActions}
                      </div>
                    ) : null}
                    {pendingAgentPreview && isAgentPreviewOpen ? null : agentConfirmationActions}
                  </div>
                </article>
              ) : null}
              {streamingTerminal ? <TerminalRunBlock run={streamingTerminal} /> : null}
              {!pendingAgentQuestion && !pendingAgentConfirmation && !streamingTerminal ? (
              <article className="notia-chat-message notia-chat-message--assistant">
                <AssistantAvatar agent={streamingAgentLook} />
                <div className={`notia-chat-message-bubble notia-chat-message-bubble--thinking${streamingThinking.trim() ? ' notia-chat-message-bubble--thinking-text' : ''}`}>
                  <span className="notia-chat-message-role">
                    {streamingAgentLook ? `${streamingAgentLook.name} · pensando` : 'Progreso'}
                  </span>
                  {streamingThinking.trim() ? (
                    <div
                      ref={thinkingContentRef}
                      className="notia-chat-thinking-content"
                      aria-live="polite"
                    >
                      <ChatMarkdownMessage source={streamingThinking} />
                    </div>
                  ) : (
                    <div className="notia-chat-thinking" role="status" aria-label="Pensando">
                      <span />
                      <span />
                      <span />
                    </div>
                  )}
                </div>
              </article>
              ) : null}
              {streamingAssistantMessage.trim() ? (
                <article className="notia-chat-message notia-chat-message--assistant">
                  <AssistantAvatar agent={streamingAgentLook} />
                  <div className="notia-chat-message-bubble">
                    <span className="notia-chat-message-role">{streamingAgentLook?.name ?? 'Notia'}</span>
                    <ChatMarkdownMessage source={streamingAssistantMessage} />
                  </div>
                </article>
              ) : null}
            </>
          ) : null}
        </>
      ) : isChatLoading ? (
        <div className="notia-chat-empty">
          <strong>Cargando chat...</strong>
        </div>
      ) : (
        <div className="notia-chat-empty">
          <Bot size={18} />
          <strong>{selectedChatFilePath ? 'Empeza una conversacion' : 'Selecciona o crea un chat'}</strong>
          <p>
            {selectedChatFilePath
              ? 'Usa una sugerencia o escribi abajo para iniciar el chat con la IA.'
              : showHistoryPanel
                ? 'Escribí abajo para crear un chat automático o elegí uno existente para continuar.'
                : 'Escribí abajo y el panel lateral crea un chat automático con la configuración rápida.'}
          </p>
        </div>
      )}
    </section>
  )
}

export const ChatThread = memo(ChatThreadComponent)
ChatThread.displayName = 'ChatThread'
