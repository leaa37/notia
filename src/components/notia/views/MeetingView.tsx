import { memo, useCallback, useEffect, useMemo, useRef, useState, type DragEvent } from 'react'
import { shallowEqual } from 'react-redux'
import { ArrowUpFromLine, Check, ChevronDown, CircleStop, Clock, Download, EllipsisVertical, FileText, Flag, Lock, Mic, Pause, Play, RotateCcw, Search, X } from 'lucide-react'
import { useVoiceTranscription } from './chat/useVoiceTranscription'
import { useNarrowContainer } from '../../../hooks/useNarrowContainer'
import { useAppDispatch, useAppSelector } from '../../../store/hooks'
import { selectAiSettings, selectSpeechRecognitionSettings } from '../../../features/preferences/preferencesSelectors'
import { setSpeechRecognitionSettings } from '../../../features/preferences/preferencesSlice'
import { selectActiveLibrary } from '../../../features/library/librarySelectors'
import { useNotiaAction } from '../../../context/notiaActions/useNotiaAction'
import { clearMeetingTranscriptContext, setMeetingTranscriptContext } from '../../../services/meeting/meetingTranscriptContext'
import {
  addMeetingMark,
  callMeetingNotesAgent,
  discardMeeting,
  exportMeeting,
  meetingAiSettings,
  pinMeetingAnswer,
  regenerateMeetingAnswer,
  removeMeetingMark,
  saveMeetingNote,
  setMeetingAiNotes,
  setMeetingLiveAnswers,
  setMeetingNotes,
} from '../../../services/meeting/meetingService'
import { skipSpeechDiarization, startAudioMonitor, stopAudioMonitor } from '../../../services/speech/speechService'
import { discardMeetingMedia, startMeetingFileSession, uploadMeetingMedia } from '../../../services/meeting/meetingMediaService'
import { loadLibraryFolderOptions } from '../../../services/chat/chatAttachmentRuntime'
import type { MeetingExportFormat, MeetingFilter } from '../../../services/meeting/meetingTypes'
import { MeetingReadyPanel, type MeetingLiveSetupProps, type MeetingSource } from './meeting/MeetingReadyPanel'
import { DEFAULT_MEETING_FOLDER, type MeetingOptionsProps } from './meeting/MeetingOptions'
import { MeetingSourceTabs, type MeetingSourceTab } from './meeting/MeetingSourceTabs'
import { MeetingUploadPanel, type MeetingFileSetupProps, type MeetingFileState } from './meeting/MeetingUploadPanel'
import { MeetingRecordingPanel } from './meeting/MeetingRecordingPanel'
import { MeetingProcessingPanel, type MeetingProcessingPanelProps } from './meeting/MeetingProcessingPanel'
import { MeetingCompletedPanel } from './meeting/MeetingCompletedPanel'
import { MeetingPhoneSetup } from './meeting/MeetingPhoneSetup'
import { MeetingPhoneRecording, type MeetingPhoneSheet } from './meeting/MeetingPhoneRecording'
import { MeetingPhoneProcessing } from './meeting/MeetingPhoneProcessing'
import { MeetingPhoneCompleted } from './meeting/MeetingPhoneCompleted'
import { MeetingPhoneMenu } from './meeting/MeetingPhoneMenu'
import { useMeetingSnapshot } from './meeting/useMeetingSnapshot'
import { useMeetingHistory } from './meeting/useMeetingHistory'
import { MeetingHistoryPanel, MeetingPhoneHistory } from './meeting/MeetingHistoryPanel'
import { useFollowMeetingSession } from './meeting/useFollowMeetingSession'
import { useSpeechLevels } from './meeting/useSpeechLevels'
import { useMeetingAiContext } from './meeting/useMeetingAiContext'
import { useAudioDevices } from './meeting/useAudioDevices'
import { MeetingMarkComposer, type MeetingAiNotesActions } from './meeting/MeetingAiNotesPanel'
import { formatClock } from './meeting/meetingDisplay'
import './meeting/meetingPhone.css'
import './meeting/meetingAi.css'
import './meeting/meetingFinished.css'

const MEETING_MAX_DURATION_SECONDS = 12 * 60 * 60
const LEVEL_HISTORY = 90
const NO_FILTER: MeetingFilter = { query: '', speakerId: null }
/** Width of the view below which the phone boards of the canvas apply. */
const PHONE_MAX_WIDTH = 600

type MeetingStage = 'ready' | 'recording' | 'processing' | 'completed'

// The meeting keeps its transcript in the backend; the voice hook's draft,
// which would copy the whole text on every preview, is not used.
const NO_DRAFT = ''
const ignoreDraft = () => undefined

const errorText = (error: unknown, fallback: string) => (error instanceof Error ? error.message : fallback)

function MeetingViewComponent() {
  const dispatch = useAppDispatch()
  const aiPreferences = useAppSelector(selectAiSettings, shallowEqual)
  const speechRecognition = useAppSelector(selectSpeechRecognitionSettings)
  const library = useAppSelector(selectActiveLibrary)
  const openFile = useNotiaAction('openFile')
  const announceTreeChange = useNotiaAction('chatWorkspaceTreeChanged')

  const [sources, setSources] = useState<Record<MeetingSource, boolean>>({ microphone: true, system: true })
  const [expectedSpeakers, setExpectedSpeakers] = useState<number | null>(null)
  const [folder, setFolder] = useState(DEFAULT_MEETING_FOLDER)
  const [folderOptions, setFolderOptions] = useState<string[]>([])
  // Off until the person turns them on: they send the transcript to the AI provider.
  const [liveAnswers, setLiveAnswers] = useState(false)
  // Notas IA starts on (canvas «Grabando»); the backend keeps it off without an AI configured.
  const [aiNotes, setAiNotes] = useState(true)
  const { state: aiContextState, aiContext } = useMeetingAiContext(library)
  const audioDevices = useAudioDevices()
  // «Nueva marca» open, with the minute it was opened at.
  const [markDraft, setMarkDraft] = useState<{ atMs: number | undefined } | null>(null)
  const [monitorId, setMonitorId] = useState<string | null>(null)
  const [filter, setFilter] = useState<MeetingFilter>(NO_FILTER)
  const [actionError, setActionError] = useState<string | null>(null)
  const [notice, setNotice] = useState<string | null>(null)
  const [busyAction, setBusyAction] = useState<'save' | 'export' | 'new' | null>(null)
  const [exportMenuOpen, setExportMenuOpen] = useState(false)
  const [isSkipping, setIsSkipping] = useState(false)
  const monitorRef = useRef<string | null>(null)
  // A file uploaded to transcribe instead of recording.
  const [sourceTab, setSourceTab] = useState<MeetingSourceTab>('live')
  const [fileState, setFileState] = useState<MeetingFileState>({ status: 'empty' })
  const [isDraggingFile, setIsDraggingFile] = useState(false)
  const [isStartingFile, setIsStartingFile] = useState(false)
  const uploadRef = useRef<AbortController | null>(null)
  const readyMediaIdRef = useRef<string | null>(null)
  readyMediaIdRef.current = fileState.status === 'ready' ? fileState.media.mediaId : null
  // The phone boards follow the width of the view, not the window's.
  const [root, setRoot] = useState<HTMLElement | null>(null)
  const phone = useNarrowContainer(root, PHONE_MAX_WIDTH)
  const [phoneSheet, setPhoneSheet] = useState<MeetingPhoneSheet | null>(null)
  const [phoneSearchOpen, setPhoneSearchOpen] = useState(false)
  const [phoneHistoryOpen, setPhoneHistoryOpen] = useState(false)

  const meetingOptions = useMemo(
    () => ({ liveAnswers, aiNotes, settings: meetingAiSettings(aiPreferences), aiContext }),
    [aiContext, aiNotes, aiPreferences, liveAnswers],
  )
  const voice = useVoiceTranscription({
    draft: NO_DRAFT,
    setDraft: ignoreDraft,
    maxDurationSeconds: MEETING_MAX_DURATION_SECONDS,
    captureMicrophone: sources.microphone,
    captureSystemAudio: sources.system,
    expectedSpeakers,
    meeting: meetingOptions,
  })
  const systemAudioSupported = voice.capabilities?.systemAudioSupported ?? false
  const effectiveSources = useMemo(
    () => ({ microphone: sources.microphone, system: sources.system && systemAudioSupported }),
    [sources, systemAudioSupported],
  )
  const { snapshot, error: snapshotError, refresh: refreshSnapshot } = useMeetingSnapshot(filter)

  const status = voice.state.status
  // A meeting that kept recording while the view was closed shows as
  // recording while the hook attaches to its session again.
  const liveElsewhere = snapshot?.status === 'live' && status === 'idle'
  const stage: MeetingStage = status === 'recording' || status === 'paused' || liveElsewhere ? 'recording'
    : status === 'finalizing' || snapshot?.status === 'processing' ? 'processing'
      : snapshot?.status === 'completed' ? 'completed'
        : 'ready'

  const attachVoice = voice.attach
  const runningId = snapshot && (snapshot.status === 'live' || snapshot.status === 'processing') ? snapshot.id : null
  useFollowMeetingSession(attachVoice, runningId, status, refreshSnapshot)
  // A view that opened again acts on the meeting's session without waiting
  // to read its state; the backend answers, or finishes a meeting whose
  // session is gone, and an error shows in the banner.
  const onSession = (action: () => Promise<unknown>) => () => {
    if (status === 'idle' && runningId) voice.holdSession(runningId)
    void action()
  }
  const levels = useSpeechLevels(stage === 'recording' ? snapshot?.id ?? null : monitorId, LEVEL_HISTORY)
  const history = useMeetingHistory(library?.id ?? null, stage === 'ready')

  const contextMeetingId = snapshot?.id ?? null
  const hasTranscript = Boolean(snapshot && (snapshot.lines.length > 0 || snapshot.totalTurns > 0 || snapshot.notes.trim()))
  useEffect(() => {
    setMeetingTranscriptContext(contextMeetingId, hasTranscript)
  }, [contextMeetingId, hasTranscript])
  useEffect(() => clearMeetingTranscriptContext, [])

  useEffect(() => {
    if (stage !== 'processing') setIsSkipping(false)
    setExportMenuOpen(false)
    setPhoneSheet(null)
    setPhoneSearchOpen(false)
    setPhoneHistoryOpen(false)
    setMarkDraft(null)
  }, [stage])

  // A speaker merged into another no longer exists to filter by.
  const filteredSpeakerGone = Boolean(filter.speakerId && snapshot && !snapshot.speakers.some((speaker) => speaker.id === filter.speakerId))
  useEffect(() => {
    if (filteredSpeakerGone) setFilter((current) => ({ ...current, speakerId: null }))
  }, [filteredSpeakerGone])

  useEffect(() => {
    let current = true
    setFolderOptions([])
    if (!library) return
    void loadLibraryFolderOptions(library).then((options) => {
      if (current) setFolderOptions(options.map((option) => option.relativePath).filter(Boolean))
    }).catch(() => undefined)
    return () => { current = false }
  }, [library])

  const stopMonitor = useCallback(() => {
    const id = monitorRef.current
    monitorRef.current = null
    setMonitorId(null)
    if (id) void stopAudioMonitor(id).catch(() => undefined)
  }, [])
  useEffect(() => stopMonitor, [stopMonitor])

  const toggleMonitor = async () => {
    if (monitorRef.current) {
      stopMonitor()
      return
    }
    setActionError(null)
    try {
      const id = await startAudioMonitor(effectiveSources)
      monitorRef.current = id
      setMonitorId(id)
    } catch (error) {
      setActionError(errorText(error, 'No se pudo probar el audio.'))
    }
  }

  const toggleSource = (source: MeetingSource) => {
    stopMonitor()
    setSources((current) => ({ ...current, [source]: !current[source] }))
  }

  // A file left uploaded when the view closes is dropped.
  useEffect(() => () => {
    uploadRef.current?.abort()
    const mediaId = readyMediaIdRef.current
    if (mediaId) void discardMeetingMedia(mediaId).catch(() => undefined)
  }, [])

  const removeFile = useCallback(() => {
    uploadRef.current?.abort()
    uploadRef.current = null
    const mediaId = readyMediaIdRef.current
    if (mediaId) void discardMeetingMedia(mediaId).catch(() => undefined)
    setFileState({ status: 'empty' })
  }, [])

  const chooseFile = async (file: File) => {
    removeFile()
    setSourceTab('file')
    setActionError(null)
    setNotice(null)
    const controller = new AbortController()
    uploadRef.current = controller
    setFileState({ status: 'uploading', name: file.name, byteLength: file.size, progress: 0 })
    try {
      const media = await uploadMeetingMedia(file, {
        signal: controller.signal,
        onProgress: (progress) => {
          if (!controller.signal.aborted) setFileState({ status: 'uploading', name: file.name, byteLength: file.size, progress })
        },
        onReading: () => {
          if (!controller.signal.aborted) setFileState({ status: 'reading', name: file.name, byteLength: file.size })
        },
      })
      if (controller.signal.aborted) {
        void discardMeetingMedia(media.mediaId).catch(() => undefined)
        return
      }
      setFileState({ status: 'ready', media })
    } catch (error) {
      if (controller.signal.aborted) return
      setFileState({ status: 'empty' })
      setActionError(errorText(error, 'No se pudo cargar el archivo.'))
    } finally {
      if (uploadRef.current === controller) uploadRef.current = null
    }
  }

  const handleDragOver = (event: DragEvent<HTMLElement>) => {
    if (stage !== 'ready' || !Array.from(event.dataTransfer.types).includes('Files')) return
    event.preventDefault()
    event.dataTransfer.dropEffect = 'copy'
    if (!isDraggingFile) setIsDraggingFile(true)
  }
  const handleDragLeave = (event: DragEvent<HTMLElement>) => {
    // Moving between children of the view is not leaving it.
    if (event.currentTarget.contains(event.relatedTarget as Node | null)) return
    setIsDraggingFile(false)
  }
  const handleDrop = (event: DragEvent<HTMLElement>) => {
    if (stage !== 'ready') return
    event.preventDefault()
    setIsDraggingFile(false)
    const dropped = event.dataTransfer.files?.[0]
    if (dropped) void chooseFile(dropped)
  }

  const transcribeFile = async () => {
    if (fileState.status !== 'ready') return
    setIsStartingFile(true)
    setActionError(null)
    setNotice(null)
    setFilter(NO_FILTER)
    try {
      const sessionId = await startMeetingFileSession({
        mediaId: fileState.media.mediaId,
        language: speechRecognition.language,
        expectedSpeakers,
        settings: meetingAiSettings(aiPreferences),
      })
      // The session owns the file now; the view follows it as a meeting.
      readyMediaIdRef.current = null
      setFileState({ status: 'empty' })
      await attachVoice(sessionId)
    } catch (error) {
      setActionError(errorText(error, 'No se pudo empezar a transcribir el archivo.'))
    } finally {
      setIsStartingFile(false)
    }
  }

  const canStart = voice.isModelReady && (effectiveSources.microphone || effectiveSources.system)
  const startVoice = voice.start
  const start = useCallback(async () => {
    stopMonitor()
    setActionError(null)
    setNotice(null)
    setFilter(NO_FILTER)
    await startVoice()
  }, [startVoice, stopMonitor])

  useEffect(() => {
    if (stage !== 'ready' || sourceTab !== 'live' || !canStart || status === 'preparing') return
    const handleShortcut = (event: KeyboardEvent) => {
      if (!event.ctrlKey || !event.shiftKey || event.altKey || event.metaKey || event.key.toLowerCase() !== 'r') return
      event.preventDefault()
      void start()
    }
    window.addEventListener('keydown', handleShortcut)
    return () => window.removeEventListener('keydown', handleShortcut)
  }, [canStart, sourceTab, stage, start, status])

  const run = async (action: () => Promise<unknown>, fallback: string) => {
    setActionError(null)
    try {
      await action()
    } catch (error) {
      setActionError(errorText(error, fallback))
    }
  }

  const meetingId = snapshot?.id ?? null
  const recordingElapsedMs = voice.state.status === 'recording' || voice.state.status === 'paused' ? voice.state.elapsedMs : undefined
  const openMarkComposer = () => {
    if (meetingId) setMarkDraft({ atMs: recordingElapsedMs })
  }
  const saveMark = (label: string) => {
    const draft = markDraft
    setMarkDraft(null)
    if (meetingId && draft) {
      void run(() => addMeetingMark(meetingId, { label: label.trim() || undefined, atMs: draft.atMs }), 'No se pudo marcar el momento.')
    }
  }
  const toggleLiveAnswers = (enabled: boolean) => {
    setLiveAnswers(enabled)
    if (meetingId) void run(() => setMeetingLiveAnswers(meetingId, enabled, aiPreferences), 'No se pudieron cambiar las respuestas en vivo.')
  }
  const notesActions: MeetingAiNotesActions = {
    libraryId: library?.id ?? null,
    onToggleAiNotes: (enabled) => {
      setAiNotes(enabled)
      if (meetingId) void run(() => setMeetingAiNotes(meetingId, enabled, aiPreferences), 'No se pudo cambiar Notas IA.')
    },
    onCallNotesAgent: () => {
      if (meetingId) void run(() => callMeetingNotesAgent(meetingId, aiPreferences), 'No se pudo llamar al agente.')
    },
    onAddOwnNote: async (text) => {
      if (!meetingId) return false
      setActionError(null)
      try {
        await addMeetingMark(meetingId, { label: text })
        return true
      } catch (error) {
        setActionError(errorText(error, 'No se pudo agregar la nota.'))
        return false
      }
    },
    onRemoveMark: (markId) => meetingId && void run(() => removeMeetingMark(meetingId, markId), 'No se pudo quitar el momento.'),
    onError: setActionError,
  }
  const saveNotes = useCallback((notes: string) => {
    if (meetingId) void setMeetingNotes(meetingId, notes).catch(() => undefined)
  }, [meetingId])

  const skipSeparation = () => {
    if (!meetingId) return
    setIsSkipping(true)
    void skipSpeechDiarization(meetingId).catch((error) => {
      setIsSkipping(false)
      setActionError(errorText(error, 'No se pudo cancelar la separación.'))
    })
  }

  const saveNote = async () => {
    if (!meetingId || !library) return
    setBusyAction('save')
    setActionError(null)
    try {
      const saved = await saveMeetingNote(meetingId, library.id, folder)
      announceTreeChange(saved.path)
      setNotice('La reunión quedó guardada como nota.')
    } catch (error) {
      setActionError(errorText(error, 'No se pudo guardar la nota.'))
    } finally {
      setBusyAction(null)
    }
  }

  const exportAs = async (format: MeetingExportFormat) => {
    setExportMenuOpen(false)
    if (!meetingId || !library) return
    setBusyAction('export')
    setActionError(null)
    try {
      const exported = await exportMeeting(meetingId, library.id, folder, format)
      announceTreeChange(exported.path)
      setNotice(`Se exportó la reunión (${format === 'pdf' ? 'PDF' : 'Word'}) junto a su nota.`)
    } catch (error) {
      setActionError(errorText(error, 'No se pudo exportar la reunión.'))
    } finally {
      setBusyAction(null)
    }
  }

  const newRecording = async () => {
    if (!meetingId) return
    setBusyAction('new')
    setNotice(null)
    await run(() => discardMeeting(meetingId), 'No se pudo empezar una nueva grabación.')
    setFilter(NO_FILTER)
    setBusyAction(null)
    voice.dismissError()
  }

  const elapsedMs = recordingElapsedMs ?? 0
  const speakerCount = snapshot?.speakers.length ?? 0
  const modelLabel = voice.modelPreparationError ? 'No se pudo preparar el modelo de voz'
    : !voice.isModelReady ? 'Preparando voz al iniciar Notia…'
      : null
  // The phone bar says «Lista» where the desktop says «Lista para grabar».
  const readyLabelFor = (idleLabel: string) => (sourceTab === 'file'
    ? (fileState.status === 'uploading' ? `Cargando archivo… ${Math.round(fileState.progress * 100)}%`
      : fileState.status === 'reading' ? 'Leyendo el archivo…'
        : modelLabel ?? (fileState.status === 'ready' ? 'Archivo listo' : 'Esperando un archivo'))
    : status === 'preparing' ? 'Iniciando captura de audio…'
      : modelLabel ?? idleLabel)
  const readyLabel = readyLabelFor('Lista para grabar')
  const fileReady = sourceTab === 'file' && fileState.status === 'ready'
  const transcribingFile = Boolean(snapshot?.sourceFile)
    && (voice.state.status !== 'finalizing' || (voice.state.stage ?? 'transcribing') === 'transcribing')
  const errorMessage = voice.state.status === 'error' ? voice.state.error.message
    : voice.modelPreparationError ?? actionError ?? snapshotError

  const optionProps: MeetingOptionsProps = {
    language: speechRecognition.language,
    onLanguageChange: (language) => dispatch(setSpeechRecognitionSettings({ ...speechRecognition, language })),
    expectedSpeakers,
    onExpectedSpeakersChange: setExpectedSpeakers,
    folder,
    folderOptions,
    libraryName: library?.name ?? null,
    onFolderChange: setFolder,
  }
  const liveSetup: MeetingLiveSetupProps = {
    canStart,
    isStarting: status === 'preparing',
    onStart: () => void start(),
    systemAudioSupported,
    sources: effectiveSources,
    onToggleSource: toggleSource,
    isChecking: monitorId !== null,
    onToggleCheck: () => void toggleMonitor(),
    levels,
  }
  const fileSetup: MeetingFileSetupProps = {
    file: fileState,
    isDragging: isDraggingFile,
    canTranscribe: voice.isModelReady && status === 'idle',
    isStarting: isStartingFile,
    onChooseFile: (file) => void chooseFile(file),
    onRemoveFile: removeFile,
    onTranscribe: () => void transcribeFile(),
  }
  const answerActions = {
    onToggleLiveAnswers: toggleLiveAnswers,
    onRegenerateAnswer: (answerId: string, shorter: boolean) => meetingId && void run(
      () => regenerateMeetingAnswer(meetingId, answerId, shorter, aiPreferences),
      'No se pudo generar la respuesta.',
    ),
    onPinAnswer: (answerId: string, pinned: boolean) => meetingId && void run(
      () => pinMeetingAnswer(meetingId, answerId, pinned),
      'No se pudo fijar la respuesta.',
    ),
    onSaveNotes: saveNotes,
    onRemoveMark: (markId: string) => meetingId && void run(() => removeMeetingMark(meetingId, markId), 'No se pudo quitar el momento.'),
  }
  const processingProps: MeetingProcessingPanelProps = {
    durationMs: snapshot?.durationMs ?? 0,
    progress: voice.state.status === 'finalizing' ? voice.state.progress : undefined,
    stage: voice.state.status === 'finalizing' ? voice.state.stage : undefined,
    lines: snapshot?.lines ?? [],
    sourceFile: snapshot?.sourceFile,
    isSkipping,
    onSkip: skipSeparation,
    onCancelFile: () => void voice.cancel(),
  }
  const rootProps = {
    ref: setRoot,
    'data-stage': stage,
    'data-dragging': isDraggingFile ? 'true' : undefined,
    onDragOver: handleDragOver,
    onDragLeave: handleDragLeave,
    onDrop: handleDrop,
  }

  const banners = (
    <>
      {errorMessage ? (
        <div className="notia-meeting-banner notia-meeting-banner--error" role="alert">
          <span>{errorMessage}</span>
          {status === 'error' || actionError ? (
            <button
              type="button"
              className="notia-meeting-icon-button"
              aria-label="Cerrar aviso"
              onClick={() => {
                setActionError(null)
                if (status === 'error') voice.dismissError()
              }}
            >
              <X size={15} aria-hidden="true" />
            </button>
          ) : null}
        </div>
      ) : null}
      {notice ? (
        <div className="notia-meeting-banner" role="status">
          <span>{notice}</span>
          <button type="button" className="notia-meeting-icon-button" aria-label="Cerrar aviso" onClick={() => setNotice(null)}>
            <X size={15} aria-hidden="true" />
          </button>
        </div>
      ) : null}
    </>
  )

  if (phone) {
    const closePhoneSearch = () => {
      setPhoneSearchOpen(false)
      setFilter(NO_FILTER)
    }
    return (
      <main className="notia-main notia-meeting-view notia-meeting-view--phone" {...rootProps}>
        <header className="notia-meeting-phone-bar" data-stage={stage}>
          {stage === 'ready' ? (
            <>
              <span className="notia-meeting-phone-title">Meeting</span>
              <span className="notia-meeting-phone-pill" role="status" aria-live="polite" data-ready={fileReady ? 'true' : undefined}>
                <span className="notia-meeting-pill-dot" aria-hidden="true" />
                <span className="notia-meeting-phone-pill-text">{readyLabelFor('Lista')}</span>
              </span>
              <button
                type="button"
                className="notia-meeting-phone-icon"
                aria-label="Reuniones anteriores"
                aria-pressed={phoneHistoryOpen}
                onClick={() => setPhoneHistoryOpen((open) => !open)}
              >
                <Clock size={19} aria-hidden="true" />
              </button>
            </>
          ) : stage === 'recording' ? (
            <>
              <h1 className="notia-meeting-phone-title">Meeting</h1>
              <span className="notia-meeting-phone-pill notia-meeting-phone-pill--recording" role="status" data-paused={status === 'paused' ? 'true' : undefined}>
                <span className="notia-meeting-pill-dot" aria-hidden="true" />
                <span className={status === 'paused' ? undefined : 'notia-meeting-sr'}>{status === 'paused' ? 'En pausa' : 'Grabando'}</span>
                <span className="notia-meeting-mono">{formatClock(elapsedMs)}</span>
              </span>
              <MeetingPhoneMenu
                label="Más opciones"
                icon={<EllipsisVertical size={18} aria-hidden="true" />}
                items={[
                  { label: 'Notas IA completas', onSelect: () => setPhoneSheet('ai-notes'), disabled: !snapshot },
                  { label: 'Notas rápidas', onSelect: () => setPhoneSheet('notes'), disabled: !snapshot },
                  { label: `Momentos marcados (${snapshot?.marks.length ?? 0})`, onSelect: () => setPhoneSheet('marks'), disabled: !snapshot },
                  { label: 'Cancelar grabación', onSelect: () => void onSession(voice.cancel)() },
                ]}
              />
            </>
          ) : stage === 'processing' ? (
            <>
              <h1 className="notia-meeting-phone-title">Meeting</h1>
              <span className="notia-meeting-phone-bar-detail" title={snapshot?.sourceFile?.name}>
                {snapshot?.sourceFile ? snapshot.sourceFile.name : formatClock(snapshot?.durationMs ?? 0)}
              </span>
            </>
          ) : (
            <>
              <h1 className="notia-meeting-phone-title">Meeting</h1>
              <div className="notia-meeting-phone-bar-actions">
                <button
                  type="button"
                  className="notia-meeting-phone-icon"
                  aria-label="Buscar en la transcripción"
                  aria-pressed={phoneSearchOpen}
                  disabled={!snapshot}
                  onClick={() => (phoneSearchOpen ? closePhoneSearch() : setPhoneSearchOpen(true))}
                >
                  <Search size={18} aria-hidden="true" />
                </button>
                <MeetingPhoneMenu
                  label={busyAction === 'export' ? 'Exportando…' : 'Exportar'}
                  icon={<ArrowUpFromLine size={18} aria-hidden="true" />}
                  disabled={!library || busyAction !== null}
                  items={[
                    { label: 'PDF', onSelect: () => void exportAs('pdf') },
                    { label: 'Word (.docx)', onSelect: () => void exportAs('docx') },
                  ]}
                />
              </div>
            </>
          )}
        </header>
        {errorMessage || notice ? <div className="notia-meeting-phone-banners">{banners}</div> : null}
        {stage === 'ready' && phoneHistoryOpen ? (
          <MeetingPhoneHistory history={history} hasLibrary={Boolean(library)} onNewMeeting={() => setPhoneHistoryOpen(false)} />
        ) : stage === 'ready' ? (
          <MeetingPhoneSetup
            tab={sourceTab}
            onSelectTab={setSourceTab}
            options={optionProps}
            live={liveSetup}
            upload={fileSetup}
            aiContext={aiContextState}
          />
        ) : stage === 'recording' ? (
          <MeetingPhoneRecording
            snapshot={snapshot}
            partialText={voice.visiblePartialText}
            levels={levels}
            isPaused={status === 'paused'}
            canMark={Boolean(meetingId) && status === 'recording'}
            onMark={openMarkComposer}
            onPause={() => void onSession(voice.pause)()}
            onResume={() => void voice.resume().catch(() => undefined)}
            onStop={() => void onSession(voice.stop)()}
            {...answerActions}
            notesActions={notesActions}
            sheet={phoneSheet}
            onCloseSheet={() => setPhoneSheet(null)}
          />
        ) : stage === 'processing' ? (
          <MeetingPhoneProcessing {...processingProps} />
        ) : snapshot ? (
          <MeetingPhoneCompleted
            key={snapshot.id}
            snapshot={snapshot}
            filter={filter}
            onFilterChange={setFilter}
            aiPreferences={aiPreferences}
            library={library}
            searchOpen={phoneSearchOpen}
            onCloseSearch={closePhoneSearch}
            isBusy={busyAction !== null}
            isSaving={busyAction === 'save'}
            onNewRecording={() => void newRecording()}
            onSaveNote={() => void saveNote()}
            onOpenNote={() => void openFile(snapshot.savedNotePath ?? '')}
          />
        ) : null}
        {markDraft && stage === 'recording' ? (
          <MeetingMarkComposer phone atMs={markDraft.atMs ?? elapsedMs} onSave={saveMark} onCancel={() => setMarkDraft(null)} />
        ) : null}
      </main>
    )
  }

  return (
    <main className="notia-main notia-meeting-view" {...rootProps}>
      {stage === 'ready' ? (
        <div className="notia-meeting-ready-layout">
          <div className="notia-meeting-ready-main">
            <header className="notia-meeting-intro">
              <div>
                <span className="notia-meeting-eyebrow"><Lock size={14} aria-hidden="true" /> Transcripción local</span>
                <h1>Meeting</h1>
                <p>{sourceTab === 'file'
                  ? 'Subí una grabación que ya tengas y Notia la transcribe entera, separada por hablante.'
                  : 'Nombrá la reunión y elegí las fuentes. Al finalizar, Notia separa las intervenciones por hablante.'}</p>
          </div>
          <span className="notia-meeting-pill" role="status" aria-live="polite" data-ready={fileReady ? 'true' : undefined}>
            <span className="notia-meeting-pill-dot" aria-hidden="true" />{readyLabel}
          </span>
        </header>
        {banners}
        <MeetingSourceTabs selected={sourceTab} onSelect={setSourceTab} />
        {sourceTab === 'file' ? (
          <MeetingUploadPanel {...fileSetup} {...optionProps} />
        ) : (
          <MeetingReadyPanel
            {...liveSetup}
            {...optionProps}
            aiContext={aiContextState}
            audioDevices={audioDevices}
            microphoneLabel={voice.audioInput?.deviceLabel ?? 'Micrófono predeterminado'}
          />
        )}
          </div>
          <MeetingHistoryPanel history={history} hasLibrary={Boolean(library)} />
        </div>
      ) : (
        <>
          <header className="notia-meeting-bar">
            <div className="notia-meeting-bar-title">
              <h1>Meeting</h1>
              {stage === 'recording' ? (
                <span className="notia-meeting-pill notia-meeting-pill--recording" role="status">
                  <span className="notia-meeting-pill-dot" aria-hidden="true" />
                  {status === 'paused' ? 'En pausa' : 'Grabando'}
                  <span className="notia-meeting-mono">{formatClock(elapsedMs)}</span>
                </span>
              ) : stage === 'processing' ? (
                <span className="notia-meeting-pill" role="status">
                  <span className="notia-meeting-pill-dot" aria-hidden="true" />{transcribingFile ? 'Transcribiendo archivo' : 'Separando hablantes'}
                </span>
              ) : (
                <span className="notia-meeting-pill notia-meeting-pill--done" role="status">
                  <Check size={13} aria-hidden="true" />Finalizada
                  <span className="notia-meeting-pill-detail">
                    · {formatClock(snapshot?.durationMs ?? 0)}{speakerCount > 0 ? ` · ${speakerCount} ${speakerCount === 1 ? 'hablante' : 'hablantes'}` : ''}
                  </span>
                </span>
              )}
            </div>
            {stage === 'recording' ? (
              <div className="notia-meeting-actions">
                <div className="notia-meeting-menu">
                  <button
                    type="button"
                    className="notia-meeting-secondary-button"
                    aria-expanded={markDraft !== null}
                    aria-controls={markDraft ? 'meeting-mark-popover' : undefined}
                    data-open={markDraft ? 'true' : undefined}
                    onClick={() => (markDraft ? setMarkDraft(null) : openMarkComposer())}
                    disabled={!meetingId || status !== 'recording'}
                  >
                    <Flag size={14} aria-hidden="true" /> Marcar momento
                  </button>
                  {markDraft ? (
                    <MeetingMarkComposer atMs={markDraft.atMs ?? elapsedMs} onSave={saveMark} onCancel={() => setMarkDraft(null)} />
                  ) : null}
                </div>
                {status === 'paused' ? (
                  <button type="button" className="notia-meeting-secondary-button" onClick={() => void voice.resume().catch(() => undefined)}>
                    <Play size={14} aria-hidden="true" /> Reanudar
                  </button>
                ) : (
                  <button type="button" className="notia-meeting-secondary-button" onClick={() => void onSession(voice.pause)()}>
                    <Pause size={14} aria-hidden="true" /> Pausar
                  </button>
                )}
                <button type="button" className="notia-meeting-primary-button" onClick={() => void onSession(voice.stop)()}>
                  <CircleStop size={14} aria-hidden="true" /> Finalizar
                </button>
                <button type="button" className="notia-meeting-ghost-button" onClick={() => void onSession(voice.cancel)()}>
                  <RotateCcw size={14} aria-hidden="true" /> Cancelar
                </button>
              </div>
            ) : stage === 'completed' ? (
              <div className="notia-meeting-actions">
                <button type="button" className="notia-meeting-secondary-button" onClick={() => void newRecording()} disabled={busyAction !== null}>
                  <Mic size={14} aria-hidden="true" /> Nueva grabación
                </button>
                <div className="notia-meeting-menu">
                  <button
                    type="button"
                    className="notia-meeting-secondary-button"
                    aria-haspopup="menu"
                    aria-expanded={exportMenuOpen}
                    onClick={() => setExportMenuOpen((open) => !open)}
                    disabled={!library || busyAction !== null}
                  >
                    <Download size={14} aria-hidden="true" /> {busyAction === 'export' ? 'Exportando…' : 'Exportar'}
                    <ChevronDown size={13} aria-hidden="true" />
                  </button>
                  {exportMenuOpen ? (
                    <div className="notia-meeting-menu-list" role="menu">
                      <button type="button" role="menuitem" onClick={() => void exportAs('pdf')}>PDF</button>
                      <button type="button" role="menuitem" onClick={() => void exportAs('docx')}>Word (.docx)</button>
                    </div>
                  ) : null}
                </div>
                {snapshot?.savedNotePath ? (
                  <button type="button" className="notia-meeting-secondary-button" onClick={() => void openFile(snapshot.savedNotePath ?? '')}>
                    <FileText size={14} aria-hidden="true" /> Abrir nota
                  </button>
                ) : null}
                <button
                  type="button"
                  className="notia-meeting-primary-button"
                  onClick={() => void saveNote()}
                  disabled={!library || busyAction !== null}
                  title={library ? undefined : 'Abrí una biblioteca para guardar la reunión'}
                >
                  <FileText size={14} aria-hidden="true" />
                  {busyAction === 'save' ? 'Guardando…' : snapshot?.savedNotePath ? 'Actualizar nota' : 'Guardar como nota'}
                </button>
              </div>
            ) : null}
          </header>

        {banners}

        {stage === 'recording' ? (
          <MeetingRecordingPanel
            snapshot={snapshot}
            partialText={voice.visiblePartialText}
            levels={levels}
            onToggleLiveAnswers={answerActions.onToggleLiveAnswers}
            onRegenerateAnswer={answerActions.onRegenerateAnswer}
            onPinAnswer={answerActions.onPinAnswer}
            notesActions={notesActions}
            canAddNote={Boolean(meetingId)}
          />
        ) : stage === 'processing' ? (
          <MeetingProcessingPanel {...processingProps} />
        ) : snapshot ? (
          <MeetingCompletedPanel
            snapshot={snapshot}
            filter={filter}
            onFilterChange={setFilter}
            aiPreferences={aiPreferences}
            library={library}
          />
        ) : null}
        </>
      )}
    </main>
  )
}

export const MeetingView = memo(MeetingViewComponent)
MeetingView.displayName = 'MeetingView'
