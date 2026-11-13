import { useState } from 'react'
import { Brain, ChevronDown, Eye, Wrench } from 'lucide-react'
import { useSubmenuEngine } from '../../../hooks/useSubmenuEngine'
import type { AiModelOption } from '../../../services/ai/aiRuntime'
import { NotiaSubmenuPanel } from '../NotiaSubmenuPanel'

interface AiModelSelectProps {
  /** Id of the list; also tells the trigger which list it opens. */
  id: string
  /** What the selector chooses, for its accessible names. */
  label: string
  value: string
  models: AiModelOption[]
  isLoading: boolean
  /** First option, with an empty value: the selector may stay unset. */
  emptyOption?: { label: string, hint: string }
  onSelect: (name: string) => void
}

/** Ollama models with what each one can do; it only shows and picks. */
export function AiModelSelect({ id, label, value, models, isLoading, emptyOption, onSelect }: AiModelSelectProps) {
  const [isOpen, setIsOpen] = useState(false)
  const { triggerRef, panelRef } = useSubmenuEngine<HTMLButtonElement, HTMLDivElement>({
    open: isOpen,
    onClose: () => setIsOpen(false),
  })
  const shown = value || emptyOption?.label || (isLoading ? 'Cargando modelos…' : 'No hay modelos disponibles')
  const choose = (name: string) => {
    onSelect(name)
    setIsOpen(false)
  }
  return (
    <div className="notia-ai-model-select">
      <button
        ref={triggerRef}
        type="button"
        className="notia-ai-model-select-trigger notia-settings-field"
        aria-label={`${label}: ${value || emptyOption?.label || 'sin seleccionar'}`}
        aria-haspopup="listbox"
        aria-expanded={isOpen}
        aria-controls={isOpen ? id : undefined}
        onClick={() => setIsOpen((current) => !current)}
        disabled={isLoading || models.length === 0}
      >
        <span>{shown}</span>
        <ChevronDown size={16} aria-hidden="true" />
      </button>
      {isOpen ? (
        <NotiaSubmenuPanel ref={panelRef} id={id} className="notia-ai-model-select-menu" role="listbox" aria-label={label}>
          {emptyOption ? (
            <button
              type="button"
              role="option"
              aria-selected={value === ''}
              className={`notia-ai-model-select-option${value === '' ? ' is-selected' : ''}`}
              onClick={() => choose('')}
            >
              <span className="notia-ai-model-select-name">{emptyOption.label}</span>
              <span className="notia-ai-model-capabilities"><span>{emptyOption.hint}</span></span>
            </button>
          ) : null}
          {models.map((model) => (
            <button
              type="button"
              role="option"
              aria-selected={model.name === value}
              className={`notia-ai-model-select-option${model.name === value ? ' is-selected' : ''}`}
              key={model.name}
              onClick={() => choose(model.name)}
            >
              <span className="notia-ai-model-select-name">{model.name}</span>
              <span className="notia-ai-model-capabilities">
                {model.supportsThinking ? <span title="Admite thinking"><Brain size={13} /> Thinking</span> : null}
                {model.supportsVision ? <span title="Admite imágenes"><Eye size={13} /> Vision</span> : null}
                {model.supportsTools ? <span title="Admite tool calling nativo"><Wrench size={13} /> Tools</span> : null}
                {!model.supportsThinking && !model.supportsVision && !model.supportsTools ? <span>Texto</span> : null}
              </span>
            </button>
          ))}
        </NotiaSubmenuPanel>
      ) : null}
    </div>
  )
}
