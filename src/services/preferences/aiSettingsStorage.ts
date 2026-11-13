const AI_SETTINGS_STORAGE_KEY = 'notia:ai-settings:v1'
const DEFAULT_OLLAMA_API_URL = 'https://ollama.com'

/*
 * AI preferences of the interface. Each library stores its own in its
 * configuration, which the backend normalizes (`library_config.rs`); this
 * module keeps the last ones used on this device as the starting point of a
 * library without them, and holds the credential only in memory.
 */

// localStorage keeps only non-secret preferences. The active credential is
// held in memory here and the library config synchronizer persists it for the
// active library. Native adapters receive it only when a request is sent.
let sessionApiKey = ''
const sessionApiKeyListeners = new Set<() => void>()

export type AiProgressMode = 'minimal' | 'standard' | 'detailed' | 'off'

export interface AiPreferences {
  ollamaUrl: string
  apiKey: string
  selectedModel: string
  /** Model that reads the images; empty sends them to `selectedModel`. */
  imageModel?: string
  /** Modules whose records the assistant saves without asking (`salud`, `gimnasio`, `rutina`). */
  autoConfirmAreas?: string[]
  thinkingEnabled: boolean
  thinkingLevel: AiThinkingLevel
  progressMode?: AiProgressMode
  showPlan?: boolean
  showReasoningSummary?: boolean
  editProgressMessage?: boolean
}

export type AiThinkingLevel = 'low' | 'medium' | 'high'

const DEFAULT_AI_PREFERENCES: AiPreferences = {
  ollamaUrl: DEFAULT_OLLAMA_API_URL,
  apiKey: '',
  selectedModel: '',
  imageModel: '',
  autoConfirmAreas: [],
  thinkingEnabled: true,
  thinkingLevel: 'medium',
  progressMode: 'minimal',
  showPlan: true,
  showReasoningSummary: true,
  editProgressMessage: true,
}

/** Stored preferences of this device, without the credential. */
export function loadAiPreferences(): AiPreferences {
  try {
    const rawValue = window.localStorage.getItem(AI_SETTINGS_STORAGE_KEY)
    const stored = rawValue ? JSON.parse(rawValue) as Partial<AiPreferences> | null : null
    if (!stored || typeof stored !== 'object') return { ...DEFAULT_AI_PREFERENCES }
    if (stored.apiKey) {
      // Older versions stored the credential; it must not stay on disk.
      window.localStorage.setItem(AI_SETTINGS_STORAGE_KEY, JSON.stringify({ ...stored, apiKey: '' }))
    }
    return { ...DEFAULT_AI_PREFERENCES, ...stored, apiKey: '' }
  } catch {
    return { ...DEFAULT_AI_PREFERENCES }
  }
}

export function saveAiPreferences(value: AiPreferences): void {
  if (sessionApiKey !== value.apiKey) {
    sessionApiKey = value.apiKey
    sessionApiKeyListeners.forEach((listener) => listener())
  }
  window.localStorage.setItem(AI_SETTINGS_STORAGE_KEY, JSON.stringify({ ...value, apiKey: '' }))
}

export function getSessionAiApiKey(): string {
  return sessionApiKey
}

/** Notifies when the session credential changes; for `useSyncExternalStore`. */
export function subscribeSessionAiApiKey(listener: () => void): () => void {
  sessionApiKeyListeners.add(listener)
  return () => {
    sessionApiKeyListeners.delete(listener)
  }
}

/** Adds the session credential only at the native transport boundary. */
export function resolveAiPreferencesForTransport(value: AiPreferences): AiPreferences {
  return value.apiKey ? value : { ...value, apiKey: sessionApiKey }
}

export function getDefaultOllamaApiUrl(): string {
  return DEFAULT_OLLAMA_API_URL
}
