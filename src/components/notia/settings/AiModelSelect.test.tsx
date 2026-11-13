// @vitest-environment happy-dom
import { afterEach, describe, expect, it, vi } from 'vitest'
import { cleanup, fireEvent, render, screen } from '@testing-library/react'
import type { AiModelOption } from '../../../services/ai/aiRuntime'
import { AiModelSelect } from './AiModelSelect'

const model = (name: string, supportsVision: boolean): AiModelOption => ({
  name,
  supportsThinking: false,
  supportsThinkingLevels: false,
  supportsVision,
  supportsTools: true,
})

const models = [model('qwen3:8b', false), model('gemma3:12b', true)]

describe('AiModelSelect', () => {
  afterEach(() => cleanup())

  it('lists the models with what they can do and picks one', () => {
    const onSelect = vi.fn()
    render(<AiModelSelect id="menu" label="Modelo para imágenes" value="" models={models} isLoading={false} emptyOption={{ label: 'El mismo modelo', hint: 'Usa el modelo de Ollama' }} onSelect={onSelect} />)

    const trigger = screen.getByRole('button', { name: 'Modelo para imágenes: El mismo modelo' })
    fireEvent.click(trigger)
    const options = screen.getAllByRole('option')
    expect(options.map((option) => option.textContent)).toEqual([
      'El mismo modeloUsa el modelo de Ollama',
      'qwen3:8b Tools',
      'gemma3:12b Vision Tools',
    ])
    expect(options[0].getAttribute('aria-selected')).toBe('true')

    fireEvent.click(screen.getByRole('option', { name: /gemma3:12b/ }))
    expect(onSelect).toHaveBeenCalledWith('gemma3:12b')
    expect(screen.queryByRole('listbox')).toBeNull()
  })

  it('shows the chosen model, offers no empty option unless asked and waits while loading', () => {
    const onSelect = vi.fn()
    const { rerender } = render(<AiModelSelect id="menu" label="Modelo de Ollama" value="qwen3:8b" models={models} isLoading={false} onSelect={onSelect} />)
    fireEvent.click(screen.getByRole('button', { name: 'Modelo de Ollama: qwen3:8b' }))
    expect(screen.getAllByRole('option')).toHaveLength(2)

    rerender(<AiModelSelect id="menu" label="Modelo de Ollama" value="" models={[]} isLoading onSelect={onSelect} />)
    const loading = screen.getByRole('button', { name: 'Modelo de Ollama: sin seleccionar' })
    expect(loading.textContent).toContain('Cargando modelos…')
    expect((loading as HTMLButtonElement).disabled).toBe(true)
  })
})
