import { useEffect, useRef, useState, useSyncExternalStore, type KeyboardEvent as ReactKeyboardEvent } from 'react'
import { useAppDispatch, useAppSelector } from '../../store/hooks'
import { selectSettingsActiveSection } from '../../features/ui/uiSelectors'
import { FolderOpen, KeyRound, Lock, Pencil, Play, Plus, RotateCcw, Trash2, TriangleAlert, Unlink, X } from 'lucide-react'
import {
  clampOcrDebounceMs,
  INKMATH_OCR_DEBOUNCE_MAX_MS,
  INKMATH_OCR_DEBOUNCE_MIN_MS,
} from '../../modules/inkmath/settings'
import type { InkMathPreferences } from '../../services/preferences/inkMathSettingsStorage'
import {
  getDefaultOllamaApiUrl,
  getSessionAiApiKey,
  subscribeSessionAiApiKey,
  type AiPreferences,
} from '../../services/preferences/aiSettingsStorage'
import { getRuntimeDevice } from '../../utils/platform/getRuntimeDevice'
import { getExplorerRefreshIntervalBounds } from '../../services/preferences/explorerPanelStorage'
import { getAppVersion } from '../../services/runtime/appVersion'
import { checkAiHealth, listAiModels, type AiModelOption } from '../../services/ai/aiRuntime'
import { NotiaModalShell } from './NotiaModalShell'
import { NotiaButton } from '../common/NotiaButton'
import type { TelegramPreferences } from '../../services/preferences/telegramSettingsStorage'
import { checkTelegramBot } from '../../services/telegram/telegramRuntime'
import { MailAccountsSection } from './settings/MailAccountsSection'
import { PublicSpacesSection } from './settings/PublicSpacesSection'
import { WebhookSection } from './settings/WebhookSection'
import { WhatsAppSection } from './settings/WhatsAppSection'
import { WeatherSection } from './settings/WeatherSection'
import { ImageModelsSection } from './settings/ImageModelsSection'
import { RunModeSection } from './settings/RunModeSection'
import { fetchConnection } from '../../services/connection/connectionRuntime'
import { selectQwen3TtsSettings, selectSpeechRecognitionSettings, selectTheme } from '../../features/preferences/preferencesSelectors'
import { setQwen3TtsSettings, setSpeechRecognitionSettings } from '../../features/preferences/preferencesSlice'
import { QWEN3_TTS_VOICES } from '../../services/preferences/qwen3TtsSettingsStorage'
import { checkQwen3TtsConnection, getQwen3TtsStatus, reloadQwen3Tts } from '../../services/qwen3Tts/qwen3TtsRuntime'
import { selectActiveLibrary } from '../../features/library/librarySelectors'
import { clearAllFinanceData } from '../../modules/finance/services/financeService'
import { financeErrorMessage } from '../../modules/finance/engines/financeError'
import { notifyFinanceDataChanged } from '../../modules/finance/services/financeDataEvents'
import { ConfirmationDialogModal } from './ConfirmationDialogModal'
import { AiModelSelect } from './settings/AiModelSelect'
import { NotiaSelectMenu } from '../common/NotiaSelectMenu'
import { disableBackups, loadBackupStatus, pickBackupDirectory, type BackupStatus } from '../../services/preferences/backupSettingsStorage'
import { saveDevicePreferences } from '../../services/preferences/devicePreferencesStorage'
import type { TaskManagerPublicationPreferences } from '../../services/preferences/taskManagerPublicationSettingsStorage'
import { readTaskBoardView } from '../../modules/task-manager/services/taskManagerService'
import { TASK_MANAGER_LOCAL_LIBRARY_USER_ID, type Board } from '../../modules/task-manager/types/taskManagerTypes'
import { getTaskManagerPublicationStatus, getTaskManagerPublicationUrl, openTaskManagerPublication, publishTaskManagerBoards, stopTaskManagerPublication, type TaskManagerPublicationStatusSnapshot } from '../../modules/task-manager/services/taskManagerPublicationRuntime'
import { loadTaskManagerPublicationTelemetry, recordTaskManagerPublicationTelemetry } from '../../modules/task-manager/services/taskManagerPublicationTelemetry'
import { isProtectedContext, normalizeContextTag, type LibraryContext } from '../../services/contexts/libraryContexts'
import { backendPlatform, backendSupports } from '../../services/transport'
import {
  createLibraryRole,
  createLibraryUser,
  deleteLibraryUser,
  listLibraryRoles,
  listLibraryUsers,
  unlinkLibraryUserTelegram,
  updateLibraryUserName,
  updateLibraryUserPassword,
  updateLibraryUserRole,
  updateLibraryUserContexts,
  type LibraryRole,
  type LibraryUser,
} from '../../services/libraries/libraryUsers'
import { SETTINGS_SECTION_META, SETTINGS_SECTIONS, settingsGroupOf, type SettingsSection } from './settings/settingsSections'
import { SettingsNav } from './settings/SettingsNav'
import { logoutApp } from '../../services/auth/appAuthRuntime'
import {
  SettingsAvatar,
  SettingsBadge,
  SettingsCard,
  SettingsChip,
  SettingsFooter,
  SettingsNotice,
  SettingsRange,
  SettingsRow,
  SettingsStat,
  SettingsSwitch,
} from './settings/SettingsControls'
import './settings/settings.css'

const OWNER_USER_ID = 'user-owner'
const OWNER_ROLE_ID = 'role-owner'
const THINKING_LEVEL_LABELS = { low: 'Bajo', medium: 'Medio', high: 'Alto' } as const
/** Modules whose records may be saved without asking; Rust decides which tools count. */
const AUTO_CONFIRM_MODULES = [
  { id: 'salud', label: 'Salud', description: 'Comidas (y corregirlas), agua, peso, pasos, sueño y mediciones. No el perfil, el objetivo ni el plan.' },
  { id: 'gimnasio', label: 'Gimnasio', description: 'El entrenamiento en curso (empezar, series, terminar) y registrar uno hecho. No las rutinas, el equipamiento ni las fichas.' },
  { id: 'rutina', label: 'Rutina', description: 'Marcar las tareas y hábitos del día. No crear ni cambiar la rutina.' },
] as const
const FINANCE_DELETED_DATA = ['Cuentas', 'Categorías personalizadas', 'Movimientos', 'Tickets', 'Productos y precios', 'Sueldos', 'Ahorro', 'Cuotas', 'Inversiones', 'Archivos de extracción']

interface SettingsModalProps {
  open: boolean
  onClose: () => void
  explorerRefreshIntervalMs: number
  onExplorerRefreshIntervalMsChange: (value: number) => void
  inkMathPreferences: InkMathPreferences
  onInkMathPreferencesChange: (value: InkMathPreferences) => void
  aiPreferences: AiPreferences
  onAiPreferencesChange: (value: AiPreferences) => void
  telegramPreferences: TelegramPreferences
  onTelegramPreferencesChange: (value: TelegramPreferences) => void
  taskManagerPublicationPreferences: TaskManagerPublicationPreferences
  onTaskManagerPublicationPreferencesChange: (value: TaskManagerPublicationPreferences) => void
  contexts: LibraryContext[]
  onContextsChange: (value: LibraryContext[]) => void
}

const VALID_SETTINGS_SECTIONS = new Set<SettingsSection>(SETTINGS_SECTIONS)

function formatPublicationBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return '0 B'
  if (bytes < 1024) return `${Math.round(bytes)} B`
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KiB`
  return `${(bytes / (1024 * 1024)).toFixed(1)} MiB`
}

function formatPublicationMilliseconds(milliseconds: number | null): string {
  if (milliseconds === null || !Number.isFinite(milliseconds)) return 'sin datos'
  return `${Math.max(0, Math.round(milliseconds))} ms`
}

function formatPublicationTimestamp(timestamp: number | null): string {
  if (timestamp === null || !Number.isFinite(timestamp)) return 'sin cambios'
  return new Date(timestamp).toLocaleTimeString('es-AR', {
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
  })
}

export function SettingsModal({
  open,
  onClose,
  explorerRefreshIntervalMs,
  onExplorerRefreshIntervalMsChange,
  inkMathPreferences,
  onInkMathPreferencesChange,
  aiPreferences,
  onAiPreferencesChange,
  telegramPreferences,
  onTelegramPreferencesChange,
  taskManagerPublicationPreferences,
  onTaskManagerPublicationPreferencesChange,
  contexts,
  onContextsChange,
}: SettingsModalProps) {
  const dispatch = useAppDispatch()
  const qwen3TtsPreferences = useAppSelector(selectQwen3TtsSettings)
  const speechRecognitionPreferences = useAppSelector(selectSpeechRecognitionSettings)
  const activeLibrary = useAppSelector(selectActiveLibrary)
  const appTheme = useAppSelector(selectTheme)
  const [qwen3TtsStatus, setQwen3TtsStatus] = useState('Consultando el runtime local...')
  const [isCheckingQwen3Tts, setIsCheckingQwen3Tts] = useState(false)
  const [qwen3TtsLoadedSelection, setQwen3TtsLoadedSelection] = useState<{ model: string, device: string } | null>(null)
  const sessionApiKey = useSyncExternalStore(subscribeSessionAiApiKey, getSessionAiApiKey)
  // The backend normalizes the preferences when it stores them.
  const incomingAiPreferences = {
    ...aiPreferences,
    progressMode: aiPreferences.progressMode ?? 'minimal',
    showPlan: aiPreferences.showPlan ?? true,
    showReasoningSummary: aiPreferences.showReasoningSummary ?? true,
    editProgressMessage: aiPreferences.editProgressMessage ?? true,
    apiKey: sessionApiKey,
  }
  const requestedSection = useAppSelector(selectSettingsActiveSection)
  const [activeSection, setActiveSection] = useState<SettingsSection>(() => {
    if (requestedSection && VALID_SETTINGS_SECTIONS.has(requestedSection)) {
      return requestedSection
    }
    return 'General'
  })

  useEffect(() => {
    if (open && requestedSection && VALID_SETTINGS_SECTIONS.has(requestedSection)) {
      setActiveSection(requestedSection)
    }
  }, [open, requestedSection])

  useEffect(() => {
    if (!open || activeSection !== 'Voz') return
    let active = true
    void getQwen3TtsStatus()
      .then((status) => {
        if (!active) return
        if (status.ready && !qwen3TtsLoadedSelection) setQwen3TtsLoadedSelection({ model: qwen3TtsPreferences.model, device: qwen3TtsPreferences.device })
        if (status.ready) setQwen3TtsStatus(`Runtime Qwen3-TTS listo con backend ${status.backend ?? 'desconocido'}.`)
        else if (status.loading) setQwen3TtsStatus('Cargando el modelo seleccionado...')
        else setQwen3TtsStatus(status.error ?? 'El runtime nativo todavía no está listo.')
      })
      .catch((error) => {
        if (active) setQwen3TtsStatus(error instanceof Error ? error.message : 'No se pudo consultar el runtime local.')
      })
    return () => { active = false }
  }, [activeSection, open, qwen3TtsLoadedSelection, qwen3TtsPreferences.model, qwen3TtsPreferences.device])
  const [ollamaUrlDraft, setOllamaUrlDraft] = useState(incomingAiPreferences.ollamaUrl)
  const [apiKeyDraft, setApiKeyDraft] = useState(incomingAiPreferences.apiKey)
  const [selectedModelDraft, setSelectedModelDraft] = useState(incomingAiPreferences.selectedModel)
  const [imageModelDraft, setImageModelDraft] = useState(incomingAiPreferences.imageModel ?? '')
  const [autoConfirmAreasDraft, setAutoConfirmAreasDraft] = useState(incomingAiPreferences.autoConfirmAreas ?? [])
  const [thinkingEnabledDraft, setThinkingEnabledDraft] = useState(incomingAiPreferences.thinkingEnabled)
  const [thinkingLevelDraft, setThinkingLevelDraft] = useState(incomingAiPreferences.thinkingLevel)
  const [progressModeDraft, setProgressModeDraft] = useState(incomingAiPreferences.progressMode)
  const [showPlanDraft, setShowPlanDraft] = useState(incomingAiPreferences.showPlan)
  const [showReasoningSummaryDraft, setShowReasoningSummaryDraft] = useState(incomingAiPreferences.showReasoningSummary)
  const [editProgressMessageDraft, setEditProgressMessageDraft] = useState(incomingAiPreferences.editProgressMessage)
  const [telegramTokenDraft, setTelegramTokenDraft] = useState(telegramPreferences.botToken)
  const [telegramStatus, setTelegramStatus] = useState('Todavía no se probó la conexión.')
  const [telegramStatusTone, setTelegramStatusTone] = useState<'idle' | 'success' | 'error'>('idle')
  const [isCheckingTelegram, setIsCheckingTelegram] = useState(false)
  const [isFinanceDeleteConfirmationOpen, setIsFinanceDeleteConfirmationOpen] = useState(false)
  const [isClearingFinanceData, setIsClearingFinanceData] = useState(false)
  const [backupStatus, setBackupStatus] = useState('')
  const [backupSettings, setBackupSettings] = useState<BackupStatus | null>(null)
  useEffect(() => {
    if (activeSection !== 'Backups') return
    let isCurrent = true
    void loadBackupStatus()
      .then((status) => { if (isCurrent) setBackupSettings(status) })
      .catch(() => { if (isCurrent) setBackupStatus('No se pudo leer la configuración de backups.') })
    return () => { isCurrent = false }
  }, [activeSection])
  const [publicationStatus, setPublicationStatus] = useState('Seleccioná uno o más tableros para habilitar la publicación local.')
  const [publicationUrl, setPublicationUrl] = useState<string | null>(null)
  const [isPublishingBoards, setIsPublishingBoards] = useState(false)

  const [publicationMetrics, setPublicationMetrics] = useState<TaskManagerPublicationStatusSnapshot | null>(null)
  const [publicationTelemetrySamples, setPublicationTelemetrySamples] = useState(() => loadTaskManagerPublicationTelemetry().samples.length)
  const [financeClearStatus, setFinanceClearStatus] = useState<{
    tone: 'idle' | 'success' | 'error'
    message: string
  }>({ tone: 'idle', message: 'Esta acción elimina definitivamente todos los datos del módulo Finanzas en la biblioteca activa.' })
  const [availableModels, setAvailableModels] = useState<AiModelOption[]>([])
  const [newContextTag, setNewContextTag] = useState('')
  const [newContextColor, setNewContextColor] = useState('#64748B')
  const [libraryRoles, setLibraryRoles] = useState<LibraryRole[]>([])
  const [libraryUsers, setLibraryUsers] = useState<LibraryUser[]>([])
  const [libraryDataStatus, setLibraryDataStatus] = useState<{ tone: 'idle' | 'loading' | 'success' | 'error', message: string }>({ tone: 'idle', message: '' })
  const [newRoleName, setNewRoleName] = useState('')
  const [newUserName, setNewUserName] = useState('')
  const [selectedUserRoleId, setSelectedUserRoleId] = useState('')
  const [isSavingLibraryData, setIsSavingLibraryData] = useState(false)
  const [passwordUserId, setPasswordUserId] = useState<string | null>(null)
  const [passwordDraft, setPasswordDraft] = useState('')
  const [passwordConfirmationDraft, setPasswordConfirmationDraft] = useState('')
  const [showPasswordDraft, setShowPasswordDraft] = useState(false)
  const [renameUserId, setRenameUserId] = useState<string | null>(null)
  const [renameDraft, setRenameDraft] = useState('')
  const [deleteUser, setDeleteUser] = useState<LibraryUser | null>(null)
  const libraryDataGenerationRef = useRef(0)
  const [isLoadingModels, setIsLoadingModels] = useState(false)
  const [modelsErrorMessage, setModelsErrorMessage] = useState<string | null>(null)
  const [aiHealthStatus, setAiHealthStatus] = useState<{
    tone: 'idle' | 'success' | 'error'
    message: string
  }>({
    tone: 'idle',
    message: 'Todavía no se probó la conexión.',
  })
  const [isCheckingAiHealth, setIsCheckingAiHealth] = useState(false)
  const projectVersion = getAppVersion()
  // Backups, publication and Telegram run in the backend: what counts is the
  // platform of the backend (the server's, for a browser), not this device.
  const platform = backendPlatform()
  const runtimeDevice = getRuntimeDevice()
  const isAndroidBackend = platform === 'android'
  const visibleSections = platform === 'windows'
    ? SETTINGS_SECTIONS
    : SETTINGS_SECTIONS.filter((section) => section !== 'Backups' && section !== 'Publicar' && (!isAndroidBackend || section !== 'Telegram'))
  const [searchQuery, setSearchQuery] = useState('')
  const [financeConfirmText, setFinanceConfirmText] = useState('')
  const paneRef = useRef<HTMLDivElement | null>(null)

  useEffect(() => {
    paneRef.current?.scrollTo({ top: 0 })
  }, [activeSection])

  useEffect(() => {
    if (isAndroidBackend && activeSection === 'Telegram') setActiveSection('General')
  }, [activeSection, isAndroidBackend])
  const publishedBoardNames = new Set(taskManagerPublicationPreferences.publishedBoardNames)
  const refreshBounds = getExplorerRefreshIntervalBounds()
  const refreshSliderMin = refreshBounds.allowDisabled ? 0 : refreshBounds.minSeconds
  const isAutoRefreshDisabled = refreshBounds.allowDisabled && explorerRefreshIntervalMs <= 0
  const refreshIntervalSeconds = isAutoRefreshDisabled
    ? 0
    : Math.max(refreshBounds.minSeconds, Math.round(explorerRefreshIntervalMs / 1000))
  const refreshIntervalLabel = isAutoRefreshDisabled ? 'Manual' : `${refreshIntervalSeconds} s`
  const refreshIntervalDescription = refreshBounds.allowDisabled
    ? `Tiempo mínimo entre dos chequeos. En 0 queda manual; si no, entre ${refreshBounds.minSeconds} s y ${refreshBounds.maxSeconds} s.`
    : `Tiempo mínimo entre dos chequeos. Entre ${refreshBounds.minSeconds} s y ${refreshBounds.maxSeconds} s.`
  const ocrDebounceMs = clampOcrDebounceMs(inkMathPreferences.debounceMs)
  const ocrDebounceLabel = `${ocrDebounceMs} ms`
  const draftAiPreferences: AiPreferences = {
    ollamaUrl: ollamaUrlDraft,
    apiKey: apiKeyDraft,
    selectedModel: selectedModelDraft,
    imageModel: imageModelDraft,
    autoConfirmAreas: autoConfirmAreasDraft,
    thinkingEnabled: thinkingEnabledDraft,
    thinkingLevel: thinkingLevelDraft,
    progressMode: progressModeDraft,
    showPlan: showPlanDraft,
    showReasoningSummary: showReasoningSummaryDraft,
    editProgressMessage: editProgressMessageDraft,
  }
  const selectedModelOption = availableModels.find((model) => model.name === selectedModelDraft) ?? null
  const imageModelOption = availableModels.find((model) => model.name === imageModelDraft) ?? null

  useEffect(() => {
    if (!open) {
      return
    }

    setOllamaUrlDraft(incomingAiPreferences.ollamaUrl)
    setApiKeyDraft(incomingAiPreferences.apiKey)
    setSelectedModelDraft(incomingAiPreferences.selectedModel)
    setImageModelDraft(incomingAiPreferences.imageModel ?? '')
    setAutoConfirmAreasDraft(incomingAiPreferences.autoConfirmAreas ?? [])
    setThinkingEnabledDraft(incomingAiPreferences.thinkingEnabled)
    setThinkingLevelDraft(incomingAiPreferences.thinkingLevel)
    setProgressModeDraft(incomingAiPreferences.progressMode)
    setShowPlanDraft(incomingAiPreferences.showPlan)
    setShowReasoningSummaryDraft(incomingAiPreferences.showReasoningSummary)
    setEditProgressMessageDraft(incomingAiPreferences.editProgressMessage)
  }, [
    incomingAiPreferences.apiKey,
    incomingAiPreferences.ollamaUrl,
    incomingAiPreferences.selectedModel,
    incomingAiPreferences.imageModel,
    incomingAiPreferences.autoConfirmAreas,
    incomingAiPreferences.thinkingEnabled,
    incomingAiPreferences.thinkingLevel,
    incomingAiPreferences.progressMode,
    incomingAiPreferences.showPlan,
    incomingAiPreferences.showReasoningSummary,
    incomingAiPreferences.editProgressMessage,
    open,
  ])

  useEffect(() => {
    if (open) setTelegramTokenDraft(telegramPreferences.botToken)
  }, [open, telegramPreferences.botToken])

  // A client never turns Telegram on: its host runs the bot (the backend
  // keeps the host's Telegram settings whatever a client sends).
  const [isClientDevice, setIsClientDevice] = useState(false)
  useEffect(() => {
    if (!open) return
    let active = true
    fetchConnection()
      .then((view) => { if (active) setIsClientDevice(view.mode === 'client') })
      // A browser of a headless server cannot ask: it is not a client.
      .catch(() => { if (active) setIsClientDevice(false) })
    return () => { active = false }
  }, [open])

  useEffect(() => {
    if (!open) setIsFinanceDeleteConfirmationOpen(false)
  }, [open])

  const toggleFinanceConfirmation = (openConfirmation: boolean) => {
    setFinanceConfirmText('')
    setIsFinanceDeleteConfirmationOpen(openConfirmation)
  }

  // Boards of the active library, to choose what to publish and to show
  // which contexts are in use.
  const [taskManagerBoards, setTaskManagerBoards] = useState<Board[]>([])
  useEffect(() => {
    if (!open || !activeLibrary) {
      setTaskManagerBoards([])
      return
    }
    let current = true
    void readTaskBoardView({ libraryId: activeLibrary.id, libraryUserId: TASK_MANAGER_LOCAL_LIBRARY_USER_ID })
      .then((view) => { if (current) setTaskManagerBoards(view.boards) })
      .catch(() => { if (current) setTaskManagerBoards([]) })
    return () => { current = false }
  }, [activeLibrary, open])

  useEffect(() => {
    libraryDataGenerationRef.current += 1
    const generation = libraryDataGenerationRef.current
    setIsSavingLibraryData(false)
    setRenameUserId(null)
    setPasswordUserId(null)
    if (!open || !activeLibrary) {
      setLibraryRoles([])
      setLibraryUsers([])
      setSelectedUserRoleId('')
      setLibraryDataStatus({ tone: 'idle', message: '' })
      return
    }
    const context = { libraryPath: activeLibrary.path, androidDirectoryUri: activeLibrary.androidTreeUri }
    setLibraryRoles([])
    setLibraryUsers([])
    setSelectedUserRoleId('')
    setLibraryDataStatus({ tone: 'loading', message: 'Cargando roles y usuarios desde SQLite…' })
    void Promise.all([listLibraryRoles(context), listLibraryUsers(context)])
      .then(([roles, users]) => {
        if (generation !== libraryDataGenerationRef.current) return
        setLibraryRoles(roles)
        setLibraryUsers(users)
        setSelectedUserRoleId(roles[0]?.id ?? '')
        setLibraryDataStatus({ tone: 'success', message: '' })
      })
      .catch((error: unknown) => {
        if (generation !== libraryDataGenerationRef.current) return
        setLibraryDataStatus({ tone: 'error', message: error instanceof Error ? error.message : 'No se pudieron cargar los datos de la biblioteca.' })
      })
    return () => { libraryDataGenerationRef.current += 1 }
  }, [activeLibrary, open])

  useEffect(() => {
    if (!open || platform !== 'windows') return
    let cancelled = false
    void getTaskManagerPublicationUrl()
      .then((url) => {
        if (!cancelled) setPublicationUrl(url)
      })
      .catch(() => {
        if (!cancelled) setPublicationUrl(null)
      })
    return () => { cancelled = true }
  }, [open, platform])

  useEffect(() => {
    if (!open || activeSection !== 'Publicar') return
    const refreshStatus = () => void getTaskManagerPublicationStatus()
      .then((status) => {
        setPublicationMetrics(status)
        setPublicationTelemetrySamples(recordTaskManagerPublicationTelemetry(status).samples.length)
      })
      .catch(() => setPublicationMetrics(null))
    refreshStatus()
    const timer = window.setInterval(refreshStatus, 2000)
    return () => window.clearInterval(timer)
  }, [activeSection, open])

  useEffect(() => {
    if (!open) {
      return
    }

    let cancelled = false
    const currentPreferences = aiPreferences

    setIsLoadingModels(true)
    setModelsErrorMessage(null)

    void listAiModels(currentPreferences)
      .then((models) => {
        if (cancelled) {
          return
        }

        const names = models.map((model) => model.name)
        setAvailableModels(models)

        const nextSelectedModel = currentPreferences.selectedModel && names.includes(currentPreferences.selectedModel)
          ? currentPreferences.selectedModel
          : names[0] ?? ''

        if (nextSelectedModel !== currentPreferences.selectedModel) {
          onAiPreferencesChange({
            ...currentPreferences,
            selectedModel: nextSelectedModel,
          })
        }

        setSelectedModelDraft(nextSelectedModel)
      })
      .catch((error) => {
        if (cancelled) {
          return
        }

        setAvailableModels([])
        setModelsErrorMessage(
          error instanceof Error && error.message.trim()
            ? error.message
            : 'No se pudieron cargar los modelos.',
        )
      })
      .finally(() => {
        if (!cancelled) {
          setIsLoadingModels(false)
        }
      })

    return () => {
      cancelled = true
    }
  }, [aiPreferences, onAiPreferencesChange, open])

  const commitAiPreferences = () => {
    onAiPreferencesChange(draftAiPreferences)
  }

  const commitAiOnEnter = (event: ReactKeyboardEvent<HTMLInputElement>) => {
    if (event.key !== 'Enter') return
    event.preventDefault()
    commitAiPreferences()
  }

  // Save pending changes when modal closes
  useEffect(() => {
    if (!open) {
      // Commit any pending changes when closing
      commitAiPreferences()
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open])

  const handleCheckAiConnection = async () => {
    onAiPreferencesChange(draftAiPreferences)
    setIsCheckingAiHealth(true)

    const result = await checkAiHealth(draftAiPreferences, { fresh: true })
    setAiHealthStatus({
      tone: result.ok ? 'success' : 'error',
      message: result.message,
    })
    setIsCheckingAiHealth(false)
  }

  const commitTelegramToken = () => {
    onTelegramPreferencesChange({ ...telegramPreferences, botToken: telegramTokenDraft })
  }

  const handleCheckTelegram = async () => {
    const token = telegramTokenDraft.trim()
    commitTelegramToken()
    setIsCheckingTelegram(true)
    try {
      const bot = await checkTelegramBot(token)
      setTelegramStatus(`Conexión correcta con @${bot.username ?? bot.displayName}. Enviá /start al bot para emparejar.`)
      setTelegramStatusTone('success')
    } catch (error) {
      setTelegramStatus(error instanceof Error ? error.message : 'No se pudo verificar el bot.')
      setTelegramStatusTone('error')
    } finally { setIsCheckingTelegram(false) }
  }

  const handleAddLibraryRole = async () => {
    if (!activeLibrary || !newRoleName.trim() || isSavingLibraryData) return
    const generation = libraryDataGenerationRef.current
    const context = { libraryPath: activeLibrary.path, androidDirectoryUri: activeLibrary.androidTreeUri }
    setIsSavingLibraryData(true)
    setLibraryDataStatus({ tone: 'loading', message: 'Guardando rol…' })
    try {
      const roles = await createLibraryRole(context, newRoleName)
      if (generation === libraryDataGenerationRef.current) {
        setLibraryRoles(roles)
        setSelectedUserRoleId((current) => current || roles[0]?.id || '')
        setNewRoleName('')
        setLibraryDataStatus({ tone: 'success', message: 'Rol guardado.' })
      }
    } catch (error) {
      if (generation === libraryDataGenerationRef.current) setLibraryDataStatus({ tone: 'error', message: error instanceof Error ? error.message : 'No se pudo guardar el rol.' })
     } finally { if (generation === libraryDataGenerationRef.current) setIsSavingLibraryData(false) }
  }

  const handleAddLibraryUser = async () => {
    if (!activeLibrary || !newUserName.trim() || !selectedUserRoleId || isSavingLibraryData) return
    const generation = libraryDataGenerationRef.current
    const context = { libraryPath: activeLibrary.path, androidDirectoryUri: activeLibrary.androidTreeUri }
    setIsSavingLibraryData(true)
    setLibraryDataStatus({ tone: 'loading', message: 'Guardando usuario…' })
    try {
      const users = await createLibraryUser(context, newUserName, selectedUserRoleId)
      if (generation === libraryDataGenerationRef.current) {
        setLibraryUsers(users)
        setNewUserName('')
        setLibraryDataStatus({ tone: 'success', message: 'Usuario guardado.' })
      }
    } catch (error) {
      if (generation === libraryDataGenerationRef.current) setLibraryDataStatus({ tone: 'error', message: error instanceof Error ? error.message : 'No se pudo guardar el usuario.' })
     } finally { if (generation === libraryDataGenerationRef.current) setIsSavingLibraryData(false) }
  }

  const handleLibraryMutationError = (error: unknown, generation = libraryDataGenerationRef.current) => {
    if (generation !== libraryDataGenerationRef.current) return
    setLibraryDataStatus({ tone: 'error', message: error instanceof Error ? error.message : 'No se pudo actualizar la biblioteca.' })
  }

  const handleUpdateLibraryUserName = async (userId: string) => {
    if (!activeLibrary || !renameDraft.trim() || isSavingLibraryData) return
    const generation = libraryDataGenerationRef.current
    const context = { libraryPath: activeLibrary.path, androidDirectoryUri: activeLibrary.androidTreeUri }
    setIsSavingLibraryData(true)
    try {
      const users = await updateLibraryUserName(context, userId, renameDraft)
      if (generation !== libraryDataGenerationRef.current) return
      setLibraryUsers(users)
      setRenameUserId(null)
      setRenameDraft('')
      setLibraryDataStatus({ tone: 'success', message: 'Nombre actualizado.' })
    } catch (error) {
      handleLibraryMutationError(error, generation)
    } finally {
      if (generation === libraryDataGenerationRef.current) setIsSavingLibraryData(false)
    }
  }

  const handleUpdateLibraryUserRole = async (userId: string, roleId: string) => {
    if (!activeLibrary || isSavingLibraryData) return
    const generation = libraryDataGenerationRef.current
    const context = { libraryPath: activeLibrary.path, androidDirectoryUri: activeLibrary.androidTreeUri }
    setIsSavingLibraryData(true)
    try {
      const users = await updateLibraryUserRole(context, userId, roleId)
      if (generation !== libraryDataGenerationRef.current) return
      setLibraryUsers(users)
      setLibraryDataStatus({ tone: 'success', message: 'Rol actualizado.' })
    } catch (error) {
      handleLibraryMutationError(error, generation)
    } finally {
      if (generation === libraryDataGenerationRef.current) setIsSavingLibraryData(false)
    }
  }

  const handleUpdateLibraryUserContexts = async (user: LibraryUser, contextTag: string, checked: boolean) => {
    if (!activeLibrary || user.allContexts || isSavingLibraryData) return
    const generation = libraryDataGenerationRef.current
    const context = { libraryPath: activeLibrary.path, androidDirectoryUri: activeLibrary.androidTreeUri }
    const nextTags = new Set(user.allowedContexts.map((tag) => tag.toLowerCase()))
    if (checked) nextTags.add(contextTag.toLowerCase())
    else nextTags.delete(contextTag.toLowerCase())
    const selectedTags = contexts
      .filter((item) => nextTags.has(item.tag.toLowerCase()))
      .map((item) => item.tag)
    setIsSavingLibraryData(true)
    try {
      const users = await updateLibraryUserContexts(context, user.id, selectedTags)
      if (generation !== libraryDataGenerationRef.current) return
      setLibraryUsers(users)
      setLibraryDataStatus({ tone: 'success', message: 'Contextos permitidos actualizados.' })
    } catch (error) {
      handleLibraryMutationError(error, generation)
    } finally {
      if (generation === libraryDataGenerationRef.current) setIsSavingLibraryData(false)
    }
  }

  const handleUnlinkLibraryUserTelegram = async (userId: string) => {
    if (!activeLibrary || isSavingLibraryData) return
    const generation = libraryDataGenerationRef.current
    const context = { libraryPath: activeLibrary.path, androidDirectoryUri: activeLibrary.androidTreeUri }
    setIsSavingLibraryData(true)
    try {
      const users = await unlinkLibraryUserTelegram(context, userId)
      if (generation !== libraryDataGenerationRef.current) return
      setLibraryUsers(users)
      setLibraryDataStatus({ tone: 'success', message: 'Telegram desvinculado.' })
    } catch (error) {
      handleLibraryMutationError(error, generation)
    } finally {
      if (generation === libraryDataGenerationRef.current) setIsSavingLibraryData(false)
    }
  }

  const handleUpdateLibraryUserPassword = async (userId: string) => {
    if (!activeLibrary || isSavingLibraryData) return
    if (passwordDraft.length < 8 || passwordDraft.length > 256) {
      setLibraryDataStatus({ tone: 'error', message: 'La contraseña debe tener entre 8 y 256 caracteres.' })
      return
    }
    if (passwordDraft !== passwordConfirmationDraft) {
      setLibraryDataStatus({ tone: 'error', message: 'Las contraseñas no coinciden.' })
      return
    }
    const generation = libraryDataGenerationRef.current
    const context = { libraryPath: activeLibrary.path, androidDirectoryUri: activeLibrary.androidTreeUri }
    setIsSavingLibraryData(true)
    try {
      const users = await updateLibraryUserPassword(context, userId, passwordDraft)
      if (generation !== libraryDataGenerationRef.current) return
      setLibraryUsers(users)
      setPasswordUserId(null)
      setPasswordDraft('')
      setPasswordConfirmationDraft('')
      setLibraryDataStatus({ tone: 'success', message: 'Contraseña actualizada.' })
    } catch (error) {
      handleLibraryMutationError(error, generation)
    } finally {
      if (generation === libraryDataGenerationRef.current) setIsSavingLibraryData(false)
    }
  }

  const handleDeleteLibraryUser = async (userId: string) => {
    if (!activeLibrary || isSavingLibraryData) return
    const generation = libraryDataGenerationRef.current
    const context = { libraryPath: activeLibrary.path, androidDirectoryUri: activeLibrary.androidTreeUri }
    setIsSavingLibraryData(true)
    try {
      const users = await deleteLibraryUser(context, userId)
      if (generation !== libraryDataGenerationRef.current) return
      setLibraryUsers(users)
      setLibraryDataStatus({ tone: 'success', message: 'Usuario eliminado.' })
    } catch (error) {
      handleLibraryMutationError(error, generation)
    } finally {
      if (generation === libraryDataGenerationRef.current) setIsSavingLibraryData(false)
    }
  }

  const handleClearFinanceData = async () => {
    if (!activeLibrary || isClearingFinanceData) return
    setIsFinanceDeleteConfirmationOpen(false)
    setIsClearingFinanceData(true)
    setFinanceClearStatus({ tone: 'idle', message: 'Eliminando los datos financieros…' })
    try {
      await clearAllFinanceData(activeLibrary)
      notifyFinanceDataChanged()
      setFinanceClearStatus({ tone: 'success', message: 'Se eliminaron los datos financieros y se restauraron las categorías iniciales.' })
    } catch (error) {
      setFinanceClearStatus({
        tone: 'error',
        message: financeErrorMessage(error, 'No se pudieron eliminar los datos financieros.'),
      })
    } finally {
      setIsClearingFinanceData(false)
    }
  }

  const handlePublicationBoardToggle = (boardName: string) => {
    const nextBoardNames = new Set(taskManagerPublicationPreferences.publishedBoardNames)
    if (nextBoardNames.has(boardName)) nextBoardNames.delete(boardName)
    else nextBoardNames.add(boardName)
    onTaskManagerPublicationPreferencesChange({
      ...taskManagerPublicationPreferences,
      publishedBoardNames: Array.from(nextBoardNames),
    })
    setPublicationUrl(null)
    setPublicationStatus('La publicación anterior se detuvo. Publicá la nueva selección para generar una URL.')
    void stopTaskManagerPublication().catch((error: unknown) => {
      setPublicationStatus(error instanceof Error ? error.message : 'No se pudo detener la publicación anterior.')
    })
  }

  const handlePublishBoards = async () => {
    if (!activeLibrary?.path || taskManagerPublicationPreferences.publishedBoardNames.length === 0) {
      setPublicationStatus('Seleccioná al menos un tablero y asegurate de tener una biblioteca activa.')
      return
    }
   setIsPublishingBoards(true)
   try {
      // The backend reads the selection it stores; save it before publishing.
      await saveDevicePreferences({ taskManagerPublication: taskManagerPublicationPreferences })
      const url = await publishTaskManagerBoards(activeLibrary.id, appTheme)
      setPublicationUrl(url)
      setPublicationStatus('Tableros publicados en la red local. La URL solo funciona mientras Notia esté abierta.')
    } catch (error) {
      setPublicationStatus(error instanceof Error ? error.message : 'No se pudieron publicar los tableros.')
    } finally {
      setIsPublishingBoards(false)
    }
  }

  const chooseBackupDirectory = () => {
    void pickBackupDirectory().then((status) => {
      setBackupSettings(status)
      if (status.directoryPath) setBackupStatus('Carpeta de backups configurada.')
    }).catch((error: unknown) => setBackupStatus(error instanceof Error ? error.message : 'No se pudo elegir la carpeta.'))
  }

  const turnOffBackups = () => {
    void disableBackups().then((status) => {
      setBackupSettings(status)
      setBackupStatus('Backups desactivados.')
    }).catch((error: unknown) => setBackupStatus(error instanceof Error ? error.message : 'No se pudieron desactivar los backups.'))
  }

  const checkQwen3Voice = () => {
    setIsCheckingQwen3Tts(true)
    void checkQwen3TtsConnection(qwen3TtsPreferences)
      .then(() => setQwen3TtsStatus('Runtime Qwen3-TTS y voz verificados.'))
      .catch((error) => setQwen3TtsStatus(error instanceof Error ? error.message : 'No se pudo iniciar la voz local.'))
      .finally(() => setIsCheckingQwen3Tts(false))
  }

  const pickSection = (section: SettingsSection) => {
    setActiveSection(section)
    toggleFinanceConfirmation(false)
  }

  const addContext = () => {
    const tag = normalizeContextTag(newContextTag)
    if (!tag || contexts.some((item) => item.tag.toLowerCase() === tag.toLowerCase())) return
    onContextsChange([...contexts, { tag, color: newContextColor }])
    setNewContextTag('')
  }

  if (!open) {
    return null
  }

  const libraryName = activeLibrary?.name ?? 'Sin biblioteca activa'
  const libraryDataNotice = libraryDataStatus.tone === 'error' || libraryDataStatus.tone === 'loading'
    ? <SettingsNotice tone={libraryDataStatus.tone}>{libraryDataStatus.message}</SettingsNotice>
    : null
  const roleOptions = libraryRoles.map((role) => ({ value: role.id, label: role.name }))
  const roleName = (roleId: string) => libraryRoles.find((role) => role.id === roleId)?.name ?? ''
  const linkedTelegramUsers = libraryUsers.filter((user) => user.telegramLinked)
  const qwen3TtsNeedsReload = Boolean(qwen3TtsLoadedSelection
    && (qwen3TtsLoadedSelection.model !== qwen3TtsPreferences.model || qwen3TtsLoadedSelection.device !== qwen3TtsPreferences.device))
  const backupsSupported = backupSettings?.supported !== false && backendSupports('backend_pick_backup_directory')
  const backupsOn = Boolean(backupSettings?.directoryPath)
  const isPublicationActive = Boolean(publicationMetrics?.active)
  const financeConfirmMatches = Boolean(activeLibrary) && financeConfirmText.trim() === activeLibrary?.name

  return (
    <NotiaModalShell open={open} onClose={onClose} size="xl" panelClassName="notia-settings-modal">
      <div className="notia-settings-layout">
        <SettingsNav
          sections={visibleSections}
          active={activeSection}
          query={searchQuery}
          footer={`Notia v${projectVersion} · ${runtimeDevice}`}
          onQueryChange={setSearchQuery}
          onPick={pickSection}
        />
        <div className="notia-settings-main">
          <header className="notia-settings-heading">
            <div className="notia-settings-heading-text">
              <div className="notia-settings-heading-group">{settingsGroupOf(activeSection)}</div>
              <h2>{activeSection}</h2>
              <p>{SETTINGS_SECTION_META[activeSection].description}</p>
            </div>
            <NotiaButton size="icon" variant="ghost" className="notia-settings-icon-button" aria-label="Cerrar configuraciones" title="Cerrar" onClick={onClose}>
              <X size={18} aria-hidden="true" />
            </NotiaButton>
          </header>
          <div className="notia-settings-pane" ref={paneRef}>
            <div className="notia-settings-stack">
              {activeSection === 'General' ? (
                <>
                <RunModeSection />
                <SettingsCard>
                  <SettingsRow label="Versión" description="Versión instalada del proyecto." inline>
                    <span className="notia-settings-value">v{projectVersion}</span>
                  </SettingsRow>
                  <SettingsRow label="Dispositivo" description="Algunas funciones dependen de la plataforma." inline>
                    <span className="notia-settings-plain">{runtimeDevice}</span>
                  </SettingsRow>
                  <SettingsRow label="Biblioteca activa" description="Los ajustes de esta ventana se aplican a esta biblioteca." inline>
                    <span className="notia-settings-plain notia-settings-plain--strong">{libraryName}</span>
                  </SettingsRow>
                </SettingsCard>
                </>
              ) : activeSection === 'Contextos' ? (
                <>
                  <SettingsCard>
                    <form className="notia-settings-toolbar" onSubmit={(event) => { event.preventDefault(); addContext() }}>
                      <input type="color" className="notia-settings-swatch" aria-label="Color del nuevo contexto" value={newContextColor} onChange={(event) => setNewContextColor(event.target.value.toUpperCase())} />
                      <input
                        className="notia-settings-field notia-settings-field--grow"
                        aria-label="Nombre del nuevo contexto"
                        placeholder="#NuevoContexto"
                        value={newContextTag}
                        onChange={(event) => setNewContextTag(event.target.value)}
                      />
                      <NotiaButton type="submit" variant="primary" disabled={!newContextTag.trim()}>
                        <Plus size={15} aria-hidden="true" />Agregar
                      </NotiaButton>
                    </form>
                    {contexts.map((context, index) => {
                      const isUsedByBoard = taskManagerBoards.some((board) => board.contexto?.toLowerCase() === context.tag.toLowerCase())
                      const isProtected = isProtectedContext(context.tag)
                      const blockedReason = isProtected
                        ? 'Predeterminado'
                        : isUsedByBoard ? 'Usado por un tablero' : contexts.length <= 1 ? 'Único contexto' : null
                      return (
                        // Keyed by position: renaming changes the tag and must not remount the input being typed in.
                        <div key={index} className="notia-settings-list-row">
                          <input
                            type="color"
                            className="notia-settings-swatch"
                            aria-label={`Color de ${context.tag}`}
                            value={context.color}
                            onChange={(event) => onContextsChange(contexts.map((item) => item === context ? { ...item, color: event.target.value.toUpperCase() } : item))}
                          />
                          <input
                            className="notia-settings-inline-input"
                            aria-label={`Nombre del contexto ${context.tag}`}
                            value={context.tag}
                            readOnly={isProtected}
                            title={isProtected ? 'Contexto predeterminado: no se renombra ni se elimina' : undefined}
                            onChange={(event) => {
                              const nextTag = normalizeContextTag(event.target.value)
                              if (!nextTag || contexts.some((item) => item !== context && item.tag.toLowerCase() === nextTag.toLowerCase())) return
                              onContextsChange(contexts.map((item) => item === context ? { ...item, tag: nextTag } : item))
                            }}
                          />
                          {blockedReason ? <span className="notia-settings-row-meta">{blockedReason}</span> : null}
                          <NotiaButton
                            size="icon"
                            variant="ghost"
                            className="notia-settings-icon-button"
                            aria-label={blockedReason ? `${context.tag} no se puede eliminar` : `Eliminar ${context.tag}`}
                            title={blockedReason ?? 'Eliminar'}
                            disabled={blockedReason !== null}
                            onClick={() => onContextsChange(contexts.filter((item) => item !== context))}
                          >
                            <Trash2 size={16} aria-hidden="true" />
                          </NotiaButton>
                        </div>
                      )
                    })}
                  </SettingsCard>
                  <p className="notia-settings-note">En una nota: <code>contexto: #Personal</code>. Editá un nombre para renombrarlo.</p>
                </>
              ) : activeSection === 'Roles' ? (
                <SettingsCard>
                  <form className="notia-settings-toolbar" onSubmit={(event) => { event.preventDefault(); void handleAddLibraryRole() }}>
                    <input
                      className="notia-settings-field notia-settings-field--grow"
                      aria-label="Nombre del nuevo rol"
                      placeholder="Nombre del rol"
                      maxLength={64}
                      value={newRoleName}
                      onChange={(event) => setNewRoleName(event.target.value)}
                    />
                    <NotiaButton type="submit" variant="primary" disabled={!activeLibrary || !newRoleName.trim() || isSavingLibraryData}>
                      <Plus size={15} aria-hidden="true" />{isSavingLibraryData ? 'Guardando…' : 'Agregar rol'}
                    </NotiaButton>
                  </form>
                  {libraryDataNotice}
                  {libraryRoles.map((role) => {
                    const userCount = libraryUsers.filter((user) => user.roleId === role.id).length
                    return (
                      <div key={role.id} className="notia-settings-list-row">
                        <span className="notia-settings-list-name">{role.name}</span>
                        {role.id === OWNER_ROLE_ID ? <SettingsBadge icon={<Lock size={11} aria-hidden="true" />}>Protegido</SettingsBadge> : null}
                        <span className="notia-settings-row-meta notia-settings-count">
                          {userCount === 0 ? 'Sin usuarios' : `${userCount} ${userCount === 1 ? 'usuario' : 'usuarios'}`}
                        </span>
                      </div>
                    )
                  })}
                  {libraryDataStatus.tone !== 'loading' && libraryRoles.length === 0 ? <div className="notia-settings-empty">No hay roles para mostrar.</div> : null}
                </SettingsCard>
              ) : activeSection === 'Usuarios' ? (
                <>
                <SettingsCard>
                  <SettingsRow
                    emphasis
                    inline
                    label="Sesión del Owner"
                    description="Cierra la sesión en este equipo: Notia vuelve a pedir la contraseña del Owner y deja de recordar la sesión."
                  >
                    <NotiaButton onClick={() => { void logoutApp(activeLibrary?.id ?? null) }}>Cerrar sesión</NotiaButton>
                  </SettingsRow>
                </SettingsCard>
                <SettingsCard>
                  <form className="notia-settings-toolbar" onSubmit={(event) => { event.preventDefault(); void handleAddLibraryUser() }}>
                    <input
                      className="notia-settings-field notia-settings-field--grow"
                      aria-label="Nombre del nuevo usuario"
                      placeholder="Nombre del usuario"
                      maxLength={64}
                      value={newUserName}
                      onChange={(event) => setNewUserName(event.target.value)}
                    />
                    <NotiaSelectMenu
                      className="notia-settings-field notia-settings-field--role"
                      value={selectedUserRoleId}
                      options={[{ value: '', label: 'Rol', disabled: true }, ...roleOptions]}
                      onChange={setSelectedUserRoleId}
                      ariaLabel="Rol del nuevo usuario"
                      disabled={libraryRoles.length === 0}
                    />
                    <NotiaButton type="submit" variant="primary" disabled={!activeLibrary || !newUserName.trim() || !selectedUserRoleId || libraryRoles.length === 0 || isSavingLibraryData}>
                      <Plus size={15} aria-hidden="true" />{isSavingLibraryData ? 'Guardando…' : 'Agregar usuario'}
                    </NotiaButton>
                  </form>
                  {libraryDataNotice}
                  {libraryUsers.map((user, index) => {
                    const isOwner = user.id === OWNER_USER_ID
                    const details = [user.passwordConfigured ? 'Contraseña configurada' : 'Sin contraseña', user.telegramLinked ? 'Telegram vinculado' : null]
                      .filter(Boolean)
                      .join(' · ')
                    return (
                      <div key={user.id} className="notia-settings-user">
                        <div className="notia-settings-user-head">
                          <SettingsAvatar name={user.name} index={index} />
                          <div className="notia-settings-user-text">
                            <div className="notia-settings-row-label">
                              <span className="notia-settings-list-name">{user.name}</span>
                              {isOwner ? <SettingsBadge icon={<Lock size={11} aria-hidden="true" />}>Owner protegido</SettingsBadge> : null}
                            </div>
                            <div className="notia-settings-row-description">{details}</div>
                          </div>
                          <div className="notia-settings-user-controls">
                            <NotiaSelectMenu
                              className="notia-settings-field notia-settings-field--role"
                              ariaLabel={`Rol de ${user.name}`}
                              value={user.roleId}
                              options={roleOptions}
                              disabled={isSavingLibraryData || isOwner}
                              onChange={(roleId) => { void handleUpdateLibraryUserRole(user.id, roleId) }}
                            />
                            <div className="notia-settings-icon-group">
                              <NotiaButton
                                size="icon"
                                variant="ghost"
                                className="notia-settings-icon-button"
                                aria-label={`Cambiar nombre de ${user.name}`}
                                title="Cambiar nombre"
                                disabled={isSavingLibraryData}
                                onClick={() => { setRenameUserId(user.id); setRenameDraft(user.name) }}
                              >
                                <Pencil size={16} aria-hidden="true" />
                              </NotiaButton>
                              <NotiaButton
                                size="icon"
                                variant="ghost"
                                className="notia-settings-icon-button"
                                aria-label={`Establecer nueva contraseña para ${user.name}`}
                                title="Establecer nueva contraseña"
                                disabled={isSavingLibraryData}
                                onClick={() => { setPasswordUserId(user.id); setPasswordDraft(''); setPasswordConfirmationDraft(''); setShowPasswordDraft(false) }}
                              >
                                <KeyRound size={16} aria-hidden="true" />
                              </NotiaButton>
                              {user.telegramLinked ? (
                                <NotiaButton
                                  size="icon"
                                  variant="ghost"
                                  className="notia-settings-icon-button"
                                  aria-label={`Desvincular Telegram de ${user.name}`}
                                  title="Desvincular Telegram"
                                  disabled={isSavingLibraryData}
                                  onClick={() => { void handleUnlinkLibraryUserTelegram(user.id) }}
                                >
                                  <Unlink size={16} aria-hidden="true" />
                                </NotiaButton>
                              ) : null}
                              {isOwner ? null : (
                                <NotiaButton
                                  size="icon"
                                  variant="ghost"
                                  className="notia-settings-icon-button notia-settings-icon-button--danger"
                                  aria-label={`Eliminar usuario ${user.name}`}
                                  title="Eliminar usuario"
                                  disabled={isSavingLibraryData}
                                  onClick={() => setDeleteUser(user)}
                                >
                                  <Trash2 size={16} aria-hidden="true" />
                                </NotiaButton>
                              )}
                            </div>
                          </div>
                        </div>
                        {renameUserId === user.id ? (
                          <form className="notia-settings-inline-form" onSubmit={(event) => { event.preventDefault(); void handleUpdateLibraryUserName(user.id) }}>
                            <input
                              className="notia-settings-field notia-settings-field--grow"
                              aria-label={`Nuevo nombre para ${user.name}`}
                              value={renameDraft}
                              maxLength={64}
                              onChange={(event) => setRenameDraft(event.target.value)}
                            />
                            <NotiaButton type="submit" variant="primary" disabled={isSavingLibraryData || !renameDraft.trim()}>Guardar</NotiaButton>
                            <NotiaButton type="button" variant="ghost" onClick={() => { setRenameUserId(null); setRenameDraft('') }}>Cancelar</NotiaButton>
                          </form>
                        ) : null}
                        {passwordUserId === user.id ? (
                          <form className="notia-settings-inline-form" onSubmit={(event) => { event.preventDefault(); void handleUpdateLibraryUserPassword(user.id) }}>
                            <input
                              className="notia-settings-field notia-settings-field--grow"
                              aria-label="Nueva contraseña"
                              type={showPasswordDraft ? 'text' : 'password'}
                              autoComplete="new-password"
                              minLength={8}
                              maxLength={256}
                              value={passwordDraft}
                              placeholder="Nueva contraseña"
                              onChange={(event) => setPasswordDraft(event.target.value)}
                            />
                            <input
                              className="notia-settings-field notia-settings-field--grow"
                              aria-label="Confirmar nueva contraseña"
                              type={showPasswordDraft ? 'text' : 'password'}
                              autoComplete="new-password"
                              minLength={8}
                              maxLength={256}
                              value={passwordConfirmationDraft}
                              placeholder="Confirmar contraseña"
                              onChange={(event) => setPasswordConfirmationDraft(event.target.value)}
                            />
                            <NotiaButton type="button" onClick={() => setShowPasswordDraft((current) => !current)}>{showPasswordDraft ? 'Ocultar' : 'Mostrar'}</NotiaButton>
                            <NotiaButton type="submit" variant="primary" disabled={isSavingLibraryData}>Guardar</NotiaButton>
                            <NotiaButton type="button" variant="ghost" onClick={() => setPasswordUserId(null)}>Cancelar</NotiaButton>
                          </form>
                        ) : null}
                        <div className="notia-settings-user-contexts">
                          <span className="notia-settings-row-description">Contextos:</span>
                          {user.allContexts ? (
                            <span className="notia-settings-plain">todos</span>
                          ) : contexts.length === 0 ? (
                            <span className="notia-settings-row-description">sin contextos configurados</span>
                          ) : contexts.map((context) => {
                            const allowed = user.allowedContexts.some((tag) => tag.toLowerCase() === context.tag.toLowerCase())
                            return (
                              <SettingsChip
                                key={context.tag}
                                pressed={allowed}
                                color={context.color}
                                disabled={isSavingLibraryData}
                                label={`${context.tag} permitido para ${user.name}`}
                                onClick={() => { void handleUpdateLibraryUserContexts(user, context.tag, !allowed) }}
                              >
                                {context.tag}
                              </SettingsChip>
                            )
                          })}
                        </div>
                      </div>
                    )
                  })}
                  {libraryUsers.length === 0 && libraryDataStatus.tone !== 'loading' ? <div className="notia-settings-empty">No hay usuarios para mostrar.</div> : null}
                </SettingsCard>
                </>
              ) : activeSection === 'Panel desplegable' ? (
                <SettingsCard>
                  <SettingsRow label="Chequeo automático de cambios" description={refreshIntervalDescription} htmlFor="notia-settings-refresh">
                    <SettingsRange
                      id="notia-settings-refresh"
                      label="Chequeo automático de cambios"
                      min={refreshSliderMin}
                      max={refreshBounds.maxSeconds}
                      step={1}
                      value={refreshIntervalSeconds}
                      valueLabel={refreshIntervalLabel}
                      onChange={(seconds) => {
                        if (refreshBounds.allowDisabled && seconds <= 0) {
                          onExplorerRefreshIntervalMsChange(0)
                          return
                        }
                        onExplorerRefreshIntervalMsChange(seconds * 1000)
                      }}
                    />
                  </SettingsRow>
                </SettingsCard>
              ) : activeSection === 'InkMath' ? (
                <SettingsCard>
                  <SettingsRow label="Debounce OCR" description="Inactividad que espera antes de enviar la fórmula manuscrita a Ollama." htmlFor="notia-settings-ocr">
                    <SettingsRange
                      id="notia-settings-ocr"
                      label="Debounce OCR"
                      min={INKMATH_OCR_DEBOUNCE_MIN_MS}
                      max={INKMATH_OCR_DEBOUNCE_MAX_MS}
                      step={50}
                      value={ocrDebounceMs}
                      valueLabel={ocrDebounceLabel}
                      onChange={(value) => onInkMathPreferencesChange({ ...inkMathPreferences, debounceMs: clampOcrDebounceMs(value) })}
                    />
                  </SettingsRow>
                </SettingsCard>
              ) : activeSection === 'IA' ? (
                <>
                  <SettingsCard title="Conexión">
                    <SettingsRow label="Host" description="Ollama Cloud por defecto. Podés usar una URL local propia." htmlFor="notia-settings-ollama-host">
                      <input
                        id="notia-settings-ollama-host"
                        className="notia-settings-field notia-settings-field--mono"
                        type="text"
                        inputMode="url"
                        value={ollamaUrlDraft}
                        placeholder={getDefaultOllamaApiUrl()}
                        onChange={(event) => setOllamaUrlDraft(event.target.value)}
                        onBlur={commitAiPreferences}
                        onKeyDown={commitAiOnEnter}
                      />
                    </SettingsRow>
                    <SettingsRow
                      label="API key"
                      htmlFor="notia-settings-api-key"
                      badge={<SettingsBadge tone={draftAiPreferences.apiKey ? 'accent' : undefined}>{draftAiPreferences.apiKey ? 'Configurada' : 'No configurada'}</SettingsBadge>}
                      description={<>Se envía como <code>Authorization: Bearer</code>. Se guarda solo en <code>.notia/notiaConfig.json</code> de la biblioteca activa; no compartas ese archivo.</>}
                    >
                      <input
                        id="notia-settings-api-key"
                        className="notia-settings-field"
                        type="password"
                        value={apiKeyDraft}
                        placeholder="ollama-api-key"
                        autoComplete="off"
                        onChange={(event) => setApiKeyDraft(event.target.value)}
                        onBlur={commitAiPreferences}
                        onKeyDown={commitAiOnEnter}
                      />
                    </SettingsRow>
                    <SettingsFooter tone={isCheckingAiHealth ? 'loading' : aiHealthStatus.tone} message={isCheckingAiHealth ? 'Probando la conexión…' : aiHealthStatus.message}>
                      <NotiaButton onClick={() => { void handleCheckAiConnection() }} disabled={isCheckingAiHealth}>
                        {isCheckingAiHealth ? 'Probando…' : 'Probar conexión'}
                      </NotiaButton>
                    </SettingsFooter>
                  </SettingsCard>
                  <SettingsCard title="Modelo">
                    <SettingsRow label="Modelo de Ollama" description="Resuelve los pedidos y usa las herramientas. Si elegís un modelo para imágenes, este no necesita ver imágenes.">
                      <AiModelSelect
                        id="notia-ai-model-select-menu"
                        label="Modelo de Ollama"
                        value={selectedModelDraft}
                        models={availableModels}
                        isLoading={isLoadingModels}
                        onSelect={(name) => {
                          setSelectedModelDraft(name)
                          onAiPreferencesChange({ ...draftAiPreferences, selectedModel: name })
                        }}
                      />
                    </SettingsRow>
                    <SettingsRow label="Modelo para imágenes" description="Lee las fotos, capturas y PDF escaneados y le pasa al modelo de Ollama lo que ve. Con «El mismo modelo», las imágenes van directo al modelo de Ollama. No es el que genera imágenes.">
                      <AiModelSelect
                        id="notia-ai-image-model-select-menu"
                        label="Modelo para imágenes"
                        value={imageModelDraft}
                        models={availableModels}
                        isLoading={isLoadingModels}
                        emptyOption={{ label: 'El mismo modelo', hint: 'Usa el modelo de Ollama' }}
                        onSelect={(name) => {
                          setImageModelDraft(name)
                          onAiPreferencesChange({ ...draftAiPreferences, imageModel: name })
                        }}
                      />
                    </SettingsRow>
                    {imageModelOption && !imageModelOption.supportsVision ? (
                      <SettingsNotice tone="error">{imageModelOption.name} no indica que pueda ver imágenes: elegí uno con Vision.</SettingsNotice>
                    ) : null}
                    {modelsErrorMessage ? <SettingsNotice tone="error">{modelsErrorMessage}</SettingsNotice> : null}
                    {selectedModelOption?.supportsThinking ? (
                      <SettingsRow
                        label="Thinking"
                        htmlFor="notia-settings-thinking"
                        inline
                        description={selectedModelOption.supportsThinkingLevels
                          ? 'Muestra el razonamiento separado de la respuesta.'
                          : 'Muestra el razonamiento separado de la respuesta. Este modelo lo activa o desactiva, sin niveles.'}
                      >
                        <SettingsSwitch
                          id="notia-settings-thinking"
                          label="Thinking"
                          checked={thinkingEnabledDraft}
                          onChange={(nextEnabled) => {
                            setThinkingEnabledDraft(nextEnabled)
                            onAiPreferencesChange({ ...draftAiPreferences, thinkingEnabled: nextEnabled })
                          }}
                        />
                      </SettingsRow>
                    ) : null}
                    {selectedModelOption?.supportsThinking && selectedModelOption.supportsThinkingLevels ? (
                      <SettingsRow label="Nivel de thinking">
                        <div className="notia-settings-segmented" role="group" aria-label="Nivel de thinking">
                          {(['low', 'medium', 'high'] as const).map((level) => (
                            <NotiaButton
                              key={level}
                              aria-pressed={thinkingLevelDraft === level}
                              disabled={!thinkingEnabledDraft}
                              onClick={() => {
                                setThinkingLevelDraft(level)
                                onAiPreferencesChange({ ...draftAiPreferences, thinkingLevel: level })
                              }}
                            >
                              {THINKING_LEVEL_LABELS[level]}
                            </NotiaButton>
                          ))}
                        </div>
                      </SettingsRow>
                    ) : null}
                  </SettingsCard>
                  <SettingsCard title="Registrar sin confirmar">
                    <SettingsNotice tone="idle">Lo que pedís desde la app o Telegram en estos módulos se guarda sin preguntar, y el asistente te cuenta qué registró. Borrar, cambiar ajustes y lo que corre solo (Acciones IA, Tareas IA, webhook) siguen pidiendo confirmación.</SettingsNotice>
                    {AUTO_CONFIRM_MODULES.map((module) => (
                      <SettingsRow key={module.id} inline label={module.label} htmlFor={`notia-settings-auto-confirm-${module.id}`} description={module.description}>
                        <SettingsSwitch
                          id={`notia-settings-auto-confirm-${module.id}`}
                          label={`Registrar sin confirmar en ${module.label}`}
                          checked={autoConfirmAreasDraft.includes(module.id)}
                          onChange={(checked) => {
                            const next = AUTO_CONFIRM_MODULES
                              .filter((item) => (item.id === module.id ? checked : autoConfirmAreasDraft.includes(item.id)))
                              .map((item) => item.id)
                            setAutoConfirmAreasDraft(next)
                            onAiPreferencesChange({ ...draftAiPreferences, autoConfirmAreas: next })
                          }}
                        />
                      </SettingsRow>
                    ))}
                  </SettingsCard>
                  <SettingsCard title="Feedback del agente">
                    <SettingsRow label="Detalle del progreso">
                      <NotiaSelectMenu
                        className="notia-settings-field notia-settings-field--short"
                        ariaLabel="Detalle del progreso"
                        value={progressModeDraft}
                        options={[
                          { value: 'minimal', label: 'Mínimo' },
                          { value: 'standard', label: 'Estándar' },
                          { value: 'detailed', label: 'Detallado' },
                          { value: 'off', label: 'Desactivado' },
                        ]}
                        onChange={(value) => {
                          if (value !== 'minimal' && value !== 'standard' && value !== 'detailed' && value !== 'off') return
                          setProgressModeDraft(value)
                          onAiPreferencesChange({ ...draftAiPreferences, progressMode: value })
                        }}
                      />
                    </SettingsRow>
                    <SettingsRow label="Mostrar TO-DO" htmlFor="notia-settings-show-plan" inline>
                      <SettingsSwitch
                        id="notia-settings-show-plan"
                        label="Mostrar TO-DO"
                        checked={showPlanDraft}
                        onChange={(checked) => {
                          setShowPlanDraft(checked)
                          onAiPreferencesChange({ ...draftAiPreferences, showPlan: checked })
                        }}
                      />
                    </SettingsRow>
                    <SettingsRow label="Mostrar resumen del enfoque" htmlFor="notia-settings-reasoning-summary" inline>
                      <SettingsSwitch
                        id="notia-settings-reasoning-summary"
                        label="Mostrar resumen del enfoque"
                        checked={showReasoningSummaryDraft}
                        onChange={(checked) => {
                          setShowReasoningSummaryDraft(checked)
                          onAiPreferencesChange({ ...draftAiPreferences, showReasoningSummary: checked })
                        }}
                      />
                    </SettingsRow>
                    <SettingsRow label="Editar un único mensaje de progreso" description="En vez de enviar un mensaje nuevo por cada paso." htmlFor="notia-settings-edit-progress" inline>
                      <SettingsSwitch
                        id="notia-settings-edit-progress"
                        label="Editar un único mensaje de progreso"
                        checked={editProgressMessageDraft}
                        onChange={(checked) => {
                          setEditProgressMessageDraft(checked)
                          onAiPreferencesChange({ ...draftAiPreferences, editProgressMessage: checked })
                        }}
                      />
                    </SettingsRow>
                  </SettingsCard>
                  <p className="notia-settings-note">En Android, Notia usa el bridge nativo hacia Python embebido, con el mismo esquema Bearer.</p>
                </>
              ) : activeSection === 'Voz' ? (
                <>
                  <SettingsCard>
                    <SettingsRow
                      emphasis
                      inline
                      label="Reconocimiento de voz"
                      htmlFor="notia-settings-stt"
                      description="Whisper large-v3-turbo con whisper.cpp: en Windows usa la GPU por Vulkan y en Android la CPU. El idioma es un código como «es»; «auto» lo detecta."
                    >
                      <span className="notia-settings-row-meta">{speechRecognitionPreferences.enabled ? 'Activo' : 'Inactivo'}</span>
                      <SettingsSwitch
                        id="notia-settings-stt"
                        label="Reconocimiento de voz"
                        checked={speechRecognitionPreferences.enabled}
                        onChange={(enabled) => dispatch(setSpeechRecognitionSettings({ ...speechRecognitionPreferences, enabled }))}
                      />
                    </SettingsRow>
                    <SettingsRow label="Idioma" htmlFor="notia-settings-stt-language">
                      <input
                        id="notia-settings-stt-language"
                        className="notia-settings-field notia-settings-field--code"
                        value={speechRecognitionPreferences.language}
                        onChange={(event) => dispatch(setSpeechRecognitionSettings({ ...speechRecognitionPreferences, language: event.target.value }))}
                      />
                    </SettingsRow>
                  </SettingsCard>
                  <SettingsCard>
                    <SettingsRow
                      emphasis
                      inline
                      label="Síntesis de voz · Qwen3-TTS"
                      htmlFor="notia-settings-tts"
                      description="Motor GGML nativo, precargado al iniciar Notia en Windows y Android."
                    >
                      <span className="notia-settings-row-meta">{qwen3TtsPreferences.enabled ? 'Activo' : 'Inactivo'}</span>
                      <SettingsSwitch
                        id="notia-settings-tts"
                        label="Síntesis de voz"
                        checked={qwen3TtsPreferences.enabled}
                        onChange={(enabled) => dispatch(setQwen3TtsSettings({ ...qwen3TtsPreferences, enabled }))}
                      />
                    </SettingsRow>
                    <SettingsRow label="Modelo">
                      <NotiaSelectMenu
                        className="notia-settings-field"
                        ariaLabel="Modelo de Qwen3-TTS"
                        value={qwen3TtsPreferences.model}
                        options={[{ value: '0.6b', label: 'Qwen3-TTS 0.6B' }, { value: '1.7b', label: 'Qwen3-TTS 1.7B' }]}
                        onChange={(value) => dispatch(setQwen3TtsSettings({ ...qwen3TtsPreferences, model: value as '0.6b' | '1.7b' }))}
                      />
                    </SettingsRow>
                    <SettingsRow label="Dispositivo" description="CUDA en Windows, CPU como respaldo.">
                      <NotiaSelectMenu
                        className="notia-settings-field"
                        ariaLabel="Dispositivo de Qwen3-TTS"
                        value={qwen3TtsPreferences.device}
                        options={[{ value: 'cpu', label: 'Automático' }]}
                        onChange={() => dispatch(setQwen3TtsSettings({ ...qwen3TtsPreferences, device: 'cpu' }))}
                      />
                    </SettingsRow>
                    <SettingsRow label="Voz e idioma">
                      <NotiaSelectMenu
                        className="notia-settings-field notia-settings-field--voice"
                        ariaLabel="Voz de Qwen3-TTS"
                        value={qwen3TtsPreferences.voice}
                        options={QWEN3_TTS_VOICES.map((voice) => ({ value: voice, label: voice }))}
                        onChange={(voice) => dispatch(setQwen3TtsSettings({ ...qwen3TtsPreferences, voice }))}
                      />
                      <input
                        className="notia-settings-field notia-settings-field--code"
                        aria-label="Idioma de Qwen3-TTS"
                        value={qwen3TtsPreferences.language}
                        onChange={(event) => dispatch(setQwen3TtsSettings({ ...qwen3TtsPreferences, language: event.target.value }))}
                      />
                    </SettingsRow>
                    <SettingsRow label="Velocidad" htmlFor="notia-settings-tts-speed">
                      <SettingsRange
                        id="notia-settings-tts-speed"
                        label="Velocidad"
                        min={0.7}
                        max={1.8}
                        step={0.05}
                        value={qwen3TtsPreferences.speed}
                        valueLabel={`${qwen3TtsPreferences.speed.toFixed(2)}×`}
                        onChange={(speed) => dispatch(setQwen3TtsSettings({ ...qwen3TtsPreferences, speed }))}
                      />
                    </SettingsRow>
                    <SettingsRow label="Pausa para enviar" description="Silencio que espera antes de enviar lo dictado." htmlFor="notia-settings-tts-pause">
                      <SettingsRange
                        id="notia-settings-tts-pause"
                        label="Pausa para enviar"
                        min={600}
                        max={4000}
                        step={100}
                        value={qwen3TtsPreferences.pauseDetectionMs}
                        valueLabel={`${qwen3TtsPreferences.pauseDetectionMs} ms`}
                        onChange={(pauseDetectionMs) => dispatch(setQwen3TtsSettings({ ...qwen3TtsPreferences, pauseDetectionMs }))}
                      />
                    </SettingsRow>
                    <SettingsRow label="Saludo inicial" htmlFor="notia-settings-tts-greeting">
                      <input
                        id="notia-settings-tts-greeting"
                        className="notia-settings-field"
                        value={qwen3TtsPreferences.greeting}
                        onChange={(event) => dispatch(setQwen3TtsSettings({ ...qwen3TtsPreferences, greeting: event.target.value }))}
                      />
                    </SettingsRow>
                    <SettingsFooter tone={isCheckingQwen3Tts ? 'loading' : 'idle'} message={qwen3TtsStatus}>
                      {qwen3TtsNeedsReload ? (
                        <NotiaButton onClick={() => { void reloadQwen3Tts().then(() => setQwen3TtsLoadedSelection(null)) }}>
                          <RotateCcw size={14} aria-hidden="true" />Recargar modelo
                        </NotiaButton>
                      ) : null}
                      <NotiaButton disabled={isCheckingQwen3Tts} onClick={checkQwen3Voice}>
                        <Play size={14} aria-hidden="true" />{isCheckingQwen3Tts ? 'Probando…' : 'Probar voz'}
                      </NotiaButton>
                    </SettingsFooter>
                  </SettingsCard>
                </>
              ) : activeSection === 'Telegram' ? (
                <>
                  <SettingsCard>
                    {isClientDevice ? (
                      <SettingsNotice tone="idle">
                        Este equipo es un cliente: Telegram lo maneja el host y se configura desde el equipo host.
                      </SettingsNotice>
                    ) : null}
                    <SettingsRow
                      emphasis
                      inline
                      label="Bot de Telegram"
                      htmlFor="notia-settings-telegram"
                      description="Un bot por biblioteca. Los usuarios se vinculan desde el bot con /start."
                    >
                      <span className="notia-settings-row-meta">{telegramPreferences.enabled ? 'Activo' : 'Inactivo'}</span>
                      <SettingsSwitch
                        id="notia-settings-telegram"
                        label="Bot de Telegram"
                        checked={telegramPreferences.enabled}
                        disabled={isClientDevice || !telegramTokenDraft.trim()}
                        onChange={(enabled) => onTelegramPreferencesChange({ ...telegramPreferences, botToken: telegramTokenDraft.trim(), enabled })}
                      />
                    </SettingsRow>
                    <SettingsRow label="Token del bot" htmlFor="notia-settings-telegram-token">
                      <input
                        id="notia-settings-telegram-token"
                        className="notia-settings-field"
                        type="password"
                        value={telegramTokenDraft}
                        autoComplete="off"
                        placeholder="123456:ABC..."
                        disabled={isClientDevice}
                        onChange={(event) => setTelegramTokenDraft(event.target.value)}
                        onBlur={commitTelegramToken}
                        onKeyDown={(event) => { if (event.key === 'Enter') { event.preventDefault(); commitTelegramToken() } }}
                      />
                    </SettingsRow>
                    <SettingsRow
                      inline
                      label="Agente autónomo"
                      htmlFor="notia-settings-telegram-autonomous"
                      description="Al llegar un mail lo revisa y, si hace falta, te escribe por Telegram (solo al Owner). La revisión de cada hora está en Acciones IA."
                    >
                      <SettingsSwitch
                        id="notia-settings-telegram-autonomous"
                        label="Agente autónomo"
                        checked={telegramPreferences.autonomousAgent}
                        disabled={isClientDevice || !telegramPreferences.enabled}
                        onChange={(autonomousAgent) => onTelegramPreferencesChange({ ...telegramPreferences, autonomousAgent })}
                      />
                    </SettingsRow>
                    <SettingsFooter tone={isCheckingTelegram ? 'loading' : telegramStatusTone} message={telegramStatus}>
                      <NotiaButton disabled={isClientDevice || !telegramTokenDraft.trim() || isCheckingTelegram} onClick={() => { void handleCheckTelegram() }}>
                        {isCheckingTelegram ? 'Probando…' : 'Probar conexión'}
                      </NotiaButton>
                    </SettingsFooter>
                  </SettingsCard>
                  <SettingsCard>
                    <SettingsRow
                      emphasis
                      label="Usuarios vinculados"
                      description={<>Cada usuario se vincula desde el bot con <code>/start</code> y su contraseña. Solo se aceptan chats privados.</>}
                    />
                    {linkedTelegramUsers.length === 0 ? <div className="notia-settings-empty">No hay usuarios vinculados.</div> : linkedTelegramUsers.map((user) => (
                      <div key={user.id} className="notia-settings-list-row">
                        <SettingsAvatar name={user.name} index={libraryUsers.indexOf(user)} small />
                        <div className="notia-settings-user-text">
                          <div className="notia-settings-list-name">{user.name}</div>
                          <div className="notia-settings-row-description">{roleName(user.roleId)}</div>
                        </div>
                        <NotiaButton disabled={isSavingLibraryData} onClick={() => { void handleUnlinkLibraryUserTelegram(user.id) }}>Desvincular</NotiaButton>
                      </div>
                    ))}
                  </SettingsCard>
                </>
              ) : activeSection === 'WhatsApp' ? (
                <WhatsAppSection libraryId={activeLibrary?.id ?? null} />
              ) : activeSection === 'Webhook' ? (
                <WebhookSection libraryId={activeLibrary?.id ?? null} />
              ) : activeSection === 'Cuentas asociadas' ? (
                <MailAccountsSection libraryId={activeLibrary?.id ?? null} />
              ) : activeSection === 'Imágenes' ? (
                <ImageModelsSection />
              ) : activeSection === 'Clima' ? (
                <WeatherSection libraryId={activeLibrary?.id ?? null} />
              ) : activeSection === 'Finanzas' ? (
                <>
                  <SettingsCard>
                    <SettingsRow label="Biblioteca" description="Los datos financieros se guardan por biblioteca." inline>
                      <span className="notia-settings-plain notia-settings-plain--strong">{libraryName}</span>
                    </SettingsRow>
                  </SettingsCard>
                  <SettingsCard tone="danger">
                    <div className="notia-settings-danger">
                      <span className="notia-settings-danger-icon" aria-hidden="true"><TriangleAlert size={17} /></span>
                      <div className="notia-settings-danger-text">
                        <h3>Eliminar datos financieros</h3>
                        <p>Borra definitivamente todo el módulo Finanzas de {libraryName} y restaura las diez categorías de gasto iniciales. No se puede deshacer.</p>
                        <ul className="notia-settings-danger-list" aria-label="Datos que se eliminan">
                          {FINANCE_DELETED_DATA.map((item) => <li key={item}>{item}</li>)}
                        </ul>
                      </div>
                      {isFinanceDeleteConfirmationOpen ? null : (
                        <NotiaButton variant="danger" disabled={!activeLibrary || isClearingFinanceData} onClick={() => toggleFinanceConfirmation(true)}>
                          {isClearingFinanceData ? 'Eliminando…' : 'Eliminar datos…'}
                        </NotiaButton>
                      )}
                    </div>
                    {isFinanceDeleteConfirmationOpen ? (
                      <form className="notia-settings-danger-confirm" onSubmit={(event) => { event.preventDefault(); if (financeConfirmMatches) void handleClearFinanceData() }}>
                        <label htmlFor="notia-settings-finance-confirm">Escribí <b>{activeLibrary?.name}</b> para confirmar</label>
                        <input
                          id="notia-settings-finance-confirm"
                          className="notia-settings-field notia-settings-field--grow notia-settings-field--mono"
                          autoComplete="off"
                          value={financeConfirmText}
                          onChange={(event) => setFinanceConfirmText(event.target.value)}
                        />
                        <NotiaButton type="button" onClick={() => toggleFinanceConfirmation(false)}>Cancelar</NotiaButton>
                        <NotiaButton type="submit" variant="danger" className="notia-settings-button--danger-solid" disabled={!financeConfirmMatches || isClearingFinanceData}>
                          Eliminar definitivamente
                        </NotiaButton>
                      </form>
                    ) : null}
                  </SettingsCard>
                  {isClearingFinanceData || financeClearStatus.tone !== 'idle' ? (
                    <SettingsNotice tone={isClearingFinanceData ? 'loading' : financeClearStatus.tone}>{financeClearStatus.message}</SettingsNotice>
                  ) : null}
                </>
              ) : activeSection === 'Backups' ? (
                <SettingsCard>
                  <SettingsRow
                    emphasis
                    inline
                    label="Backups automáticos"
                    htmlFor="notia-settings-backups"
                    badge={<SettingsBadge>Solo Windows</SettingsBadge>}
                    description="Un ZIP de la biblioteca activa por hora."
                  >
                    <span className="notia-settings-row-meta">{backupsOn ? 'Activos' : 'Desactivados'}</span>
                    <SettingsSwitch
                      id="notia-settings-backups"
                      label="Backups automáticos"
                      checked={backupsOn}
                      disabled={!backupsSupported && !backupsOn}
                      onChange={(checked) => { if (checked) chooseBackupDirectory(); else turnOffBackups() }}
                    />
                  </SettingsRow>
                  <SettingsRow
                    label="Carpeta de destino"
                    description={backupSettings?.directoryPath ? <span className="notia-settings-path">{backupSettings.directoryPath}</span> : 'Elegí una carpeta para activar los backups.'}
                  >
                    <NotiaButton disabled={!backupsSupported} onClick={chooseBackupDirectory}>
                      <FolderOpen size={14} aria-hidden="true" />{backupsOn ? 'Cambiar carpeta' : 'Elegir carpeta'}
                    </NotiaButton>
                  </SettingsRow>
                  <div className="notia-settings-stats">
                    <SettingsStat label="Frecuencia" value="Cada hora" />
                    <SettingsStat label="Retención" value="2 días · 48 copias" />
                    <SettingsStat label="Último backup" value={backupSettings?.lastBackupAt ? new Date(backupSettings.lastBackupAt * 1000).toLocaleString() : 'Todavía no hay'} />
                  </div>
                  {backupSettings?.lastError ? <SettingsNotice tone="error">Último error: {backupSettings.lastError}</SettingsNotice> : null}
                  {backupStatus ? <SettingsFooter message={backupStatus} /> : null}
                </SettingsCard>
              ) : activeSection === 'Publicar' ? (
                <>
                  <SettingsCard>
                    <div className="notia-settings-block">
                      <div className="notia-settings-row-label">
                        <span className="notia-settings-block-title">Tableros del Task Manager</span>
                        <SettingsBadge>Solo Windows</SettingsBadge>
                      </div>
                      <div className="notia-settings-row-description">El mismo Task Manager, con vistas y edición, en cualquier navegador de la red local. Notia tiene que seguir abierta.</div>
                      {taskManagerBoards.length === 0 ? (
                        <div className="notia-settings-row-description">Todavía no hay tableros disponibles.</div>
                      ) : (
                        <div className="notia-settings-chips" role="group" aria-label="Tableros publicados">
                          {taskManagerBoards.map((board) => (
                            <SettingsChip key={board.name} pressed={publishedBoardNames.has(board.name)} onClick={() => handlePublicationBoardToggle(board.name)}>
                              {board.name}
                            </SettingsChip>
                          ))}
                        </div>
                      )}
                    </div>
                    <SettingsRow label="Puerto fijo" htmlFor="notia-settings-publication-port" inline>
                      <input
                        id="notia-settings-publication-port"
                        className="notia-settings-field notia-settings-field--number"
                        type="number"
                        min="1024"
                        max="65535"
                        value={taskManagerPublicationPreferences.port}
                        onChange={(event) => {
                          const port = Number(event.target.value)
                          if (Number.isInteger(port) && port >= 1024 && port <= 65535) onTaskManagerPublicationPreferencesChange({ ...taskManagerPublicationPreferences, port })
                        }}
                      />
                    </SettingsRow>
                    <SettingsRow label="Clientes simultáneos máximos" htmlFor="notia-settings-publication-clients" inline>
                      <input
                        id="notia-settings-publication-clients"
                        className="notia-settings-field notia-settings-field--number"
                        type="number"
                        min="1"
                        max="64"
                        value={taskManagerPublicationPreferences.maxClients}
                        onChange={(event) => {
                          const maxClients = Number(event.target.value)
                          if (Number.isInteger(maxClients) && maxClients >= 1 && maxClients <= 64) onTaskManagerPublicationPreferencesChange({ ...taskManagerPublicationPreferences, maxClients })
                        }}
                      />
                    </SettingsRow>
                    {publicationUrl ? (
                      <SettingsRow
                        label="URL"
                        description="Si otro equipo no puede abrirla, permití Notia en el Firewall de Windows para redes privadas. Usa HTTPS con un certificado autofirmado: en cada equipo remoto aceptá o instalá el certificado de Notia la primera vez."
                      >
                        <span className="notia-settings-path" aria-label="URL de publicación">{publicationUrl}</span>
                      </SettingsRow>
                    ) : null}
                    <SettingsFooter tone={isPublicationActive ? 'success' : 'idle'} message={`${isPublicationActive ? 'Publicado' : 'No publicado'} · ${publicationStatus}`}>
                      <NotiaButton onClick={() => void openTaskManagerPublication()} disabled={!publicationUrl}>Abrir en el navegador</NotiaButton>
                      <NotiaButton
                        variant="primary"
                        onClick={() => void handlePublishBoards()}
                        disabled={isPublishingBoards || !activeLibrary || taskManagerPublicationPreferences.publishedBoardNames.length === 0}
                      >
                        {isPublishingBoards ? 'Publicando…' : 'Publicar y actualizar'}
                      </NotiaButton>
                    </SettingsFooter>
                  </SettingsCard>
                  <PublicSpacesSection libraryId={activeLibrary?.id ?? null} />
                  {publicationMetrics ? (
                    <SettingsCard title="Estado del servidor">
                      {publicationMetrics.active ? (
                        <>
                          {publicationMetrics.recoveryRequired ? <SettingsNotice tone="error">La publicación requiere recuperación: quedó una operación parcial o no verificada. Revisá el workspace y ejecutá una operación del Task Manager que termine correctamente; no se reejecutará nada automáticamente.</SettingsNotice> : null}
                          {publicationMetrics.websocketSessions >= publicationMetrics.maxWebsocketSessions ? <SettingsNotice tone="error">La publicación alcanzó su capacidad de WebSocket. Los nuevos accesos serán rechazados hasta que se desconecte alguien.</SettingsNotice> : null}
                          {publicationMetrics.mutationLatencyLastMs !== null && publicationMetrics.mutationLatencyLastMs > 1000 ? <SettingsNotice tone="error">El filesystem está tardando más de un segundo en confirmar cambios. Revisá la carga del host antes de continuar con operaciones masivas.</SettingsNotice> : null}
                          <div className="notia-settings-stats">
                            <SettingsStat mono label="WebSocket" value={`${publicationMetrics.websocketSessions}/${publicationMetrics.maxWebsocketSessions}`} />
                            <SettingsStat mono label="Sesiones" value={`${publicationMetrics.authenticatedSessions}/${publicationMetrics.maxAuthenticatedSessions}`} />
                            <SettingsStat mono label="Conflictos" value={publicationMetrics.conflicts} />
                            <SettingsStat mono label="Errores" value={publicationMetrics.mutationErrors} />
                          </div>
                          <div className="notia-settings-log">
                            <span>latencia mutaciones · última {formatPublicationMilliseconds(publicationMetrics.mutationLatencyLastMs)} · p95 {formatPublicationMilliseconds(publicationMetrics.mutationLatencyP95Ms)} · {publicationMetrics.mutationLatencySamples} muestras</span>
                            <span>frames {publicationMetrics.websocketFramesReceived}/{publicationMetrics.websocketFramesSent} · bytes {formatPublicationBytes(publicationMetrics.websocketBytesReceived)}/{formatPublicationBytes(publicationMetrics.websocketBytesSent)} · streams cancelados {publicationMetrics.aiStreamCancellations}</span>
                            <span>resync {publicationMetrics.resyncRequired} · eventos descartados {publicationMetrics.droppedEvents} · revisión {publicationMetrics.revision}</span>
                            <span>época {publicationMetrics.publicationEpoch.slice(0, 8) || '—'} · última operación {publicationMetrics.lastOperationId?.slice(0, 8) || '—'} · actor {publicationMetrics.lastActorId?.slice(0, 8) || '—'}</span>
                            <span>último cambio: {formatPublicationTimestamp(publicationMetrics.lastChangeAtUnixMs)}</span>
                            <span>telemetría local: {publicationTelemetrySamples} muestras acotadas (sin contenido ni secretos)</span>
                          </div>
                        </>
                      ) : (
                        <div className="notia-settings-empty">La publicación no está activa.</div>
                      )}
                    </SettingsCard>
                  ) : null}
                </>
              ) : null}
            </div>
          </div>
        </div>
      </div>
      <ConfirmationDialogModal
        open={deleteUser !== null}
        title="Eliminar usuario"
        message={`¿Querés eliminar a ${deleteUser?.name ?? 'este usuario'}? Se revocará su vínculo de Telegram y no se puede deshacer.`}
        confirmLabel="Eliminar usuario"
        cancelLabel="Cancelar"
        tone="danger"
        onConfirm={() => {
          if (!deleteUser) return
          const userId = deleteUser.id
          setDeleteUser(null)
          void handleDeleteLibraryUser(userId)
        }}
        onCancel={() => setDeleteUser(null)}
      />
    </NotiaModalShell>
  )
}
