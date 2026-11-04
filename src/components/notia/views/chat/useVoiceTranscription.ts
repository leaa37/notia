import { useCallback, useEffect, useRef, useState } from 'react'
import { useAppSelector } from '../../../../store/hooks'
import { selectSpeechRecognitionSettings } from '../../../../features/preferences/preferencesSelectors'
import {
  cancelSpeechSession,
  consumeSpeechTurn,
  getSpeechCapabilities,
  getSpeechSessionState,
  listenSpeechPartial,
  listenSpeechSegments,
  listenSpeechState,
  pauseSpeechSession,
  probeSpeechAudioInput,
  probeSherpaRuntime,
  prepareSpeechModel,
  resumeSpeechSession,
  startSpeechSession,
  stopSpeechSession,
} from '../../../../services/speech/speechService'
import { mergeVoiceTextIntoDraft } from '../../../../services/speech/speechTranscript'
import { backendKind, backendSupports } from '../../../../services/transport'
import { useRemoteVoiceTranscription } from './useRemoteVoiceTranscription'
import type {
  MeetingSessionOptions,
  SpeechAudioInputStatus,
  SpeechCapabilities,
  SpeechSessionState,
  SherpaRuntimeStatus,
} from '../../../../services/speech/speechTypes'

const INITIAL_STATE: SpeechSessionState = { status: 'idle' }
const ACTIVE_STATUSES = new Set<SpeechSessionState['status']>(['preparing', 'recording', 'paused', 'finalizing'])

interface UseVoiceTranscriptionInput {
  draft: string
  setDraft: (value: string) => void
  pauseDetectionMs?: number | null
  continuousSession?: boolean
  maxDurationSeconds?: number
  onCompleted?: (text: string) => void
  captureSystemAudio?: boolean
  captureMicrophone?: boolean
  /** Speakers the diarization must find; `null` lets it decide. */
  expectedSpeakers?: number | null
  /** The session records a Meeting. */
  meeting?: MeetingSessionOptions | null
}

export function hasNewRecognizedSpeech(previous: string, next: string): boolean {
  return Boolean(next.trim()) && next.trim() !== previous
}

/** An empty preview means nothing is being spoken: the utterance was confirmed or discarded. */
export function stabilizePartialTranscript(previous: string, next: string): string {
  const normalizedPrevious = previous.trim()
  const normalizedNext = next.trim()
  if (!normalizedNext) return ''
  if (!normalizedPrevious || normalizedNext.startsWith(normalizedPrevious)) return normalizedNext
  const previousWords = normalizedPrevious.split(/\s+/).length
  const nextWords = normalizedNext.split(/\s+/).length
  return nextWords > previousWords ? normalizedNext : normalizedPrevious
}

function useLocalVoiceTranscription({
  draft,
  setDraft,
  pauseDetectionMs = null,
  continuousSession = false,
  maxDurationSeconds = 900,
  onCompleted,
  captureSystemAudio = false,
  captureMicrophone = true,
  expectedSpeakers = null,
  meeting = null,
}: UseVoiceTranscriptionInput) {
  const speechRecognition = useAppSelector(selectSpeechRecognitionSettings)
  // The backend preloads the saved model at startup; asking before the saved
  // preferences arrive would prepare the default model instead.
  const devicePreferencesLoaded = useAppSelector((appState) => appState.preferences.devicePreferencesLoaded)
  const [capabilities, setCapabilities] = useState<SpeechCapabilities | null>(null)
  const [audioInput, setAudioInput] = useState<SpeechAudioInputStatus | null>(null)
  const [sherpaRuntime, setSherpaRuntime] = useState<SherpaRuntimeStatus | null>(null)
  const [state, setState] = useState<SpeechSessionState>(INITIAL_STATE)
  const [visiblePartialText, setVisiblePartialText] = useState('')
  const [isModelReady, setIsModelReady] = useState(false)
  const [modelPreparationError, setModelPreparationError] = useState<string | null>(null)
  const sessionIdRef = useRef<string | null>(null)
  const consumingTurnRef = useRef<string | null>(null)
  const baseDraftRef = useRef('')
  const silenceTimerRef = useRef<number | null>(null)
  const lastObservedSpeechRef = useRef('')
  const confirmedTextRef = useRef('')
  const visiblePartialTextRef = useRef('')
  const onCompletedRef = useRef(onCompleted)
  const environmentErrorRef = useRef<string | null>(null)
  // A Meeting session belongs to the backend: it keeps recording when the
  // view closes and the view follows it again with `attach`.
  const keepsSessionRef = useRef(meeting !== null)
  const stateEventsRef = useRef(0)

  useEffect(() => {
    onCompletedRef.current = onCompleted
  }, [onCompleted])

  useEffect(() => {
    keepsSessionRef.current = meeting !== null
  }, [meeting])

  useEffect(() => {
    setIsModelReady(false)
    setModelPreparationError(null)
  }, [speechRecognition])

  useEffect(() => {
    let current = true
    if (!devicePreferencesLoaded || !speechRecognition.enabled) {
      setIsModelReady(false)
      return () => { current = false }
    }

    void prepareSpeechModel(speechRecognition, 'device').then(() => {
      if (!current) return
      setIsModelReady(true)
      setModelPreparationError(null)
    }).catch((error: unknown) => {
      if (!current) return
      setIsModelReady(false)
      setModelPreparationError(error instanceof Error ? error.message : 'No se pudo preparar el reconocimiento de voz.')
    })

    return () => { current = false }
  }, [devicePreferencesLoaded, speechRecognition])

  useEffect(() => {
    let current = true
    void Promise.allSettled([
      getSpeechCapabilities(),
      probeSpeechAudioInput(),
      probeSherpaRuntime(),
    ]).then(([capabilitiesResult, audioResult, sherpaResult]) => {
      if (!current) return
      if (capabilitiesResult.status === 'fulfilled') setCapabilities(capabilitiesResult.value)
      else environmentErrorRef.current = capabilitiesResult.reason instanceof Error
        ? capabilitiesResult.reason.message
        : 'No se pudo comprobar las capacidades de voz.'
      if (audioResult.status === 'fulfilled') setAudioInput(audioResult.value)
      else if (!environmentErrorRef.current) environmentErrorRef.current = audioResult.reason instanceof Error
        ? audioResult.reason.message
        : 'No se pudo comprobar el micrófono.'
      if (sherpaResult.status === 'fulfilled') setSherpaRuntime(sherpaResult.value)
    })
    return () => { current = false }
  }, [])

  useEffect(() => {
    let disposed = false
    const unlisteners: Array<() => void> = []
    const completeDetectedTurn = (sessionId: string) => {
      if (!continuousSession) {
        void stopSpeechSession(sessionId).catch(() => undefined)
        return
      }
      // Partial-text and diarization events can observe the same silence. Only
      // one consume request may be in flight for a session; a second request
      // would race and report a misleading generic speech error.
      if (consumingTurnRef.current === sessionId) return
      consumingTurnRef.current = sessionId
      void consumeSpeechTurn(sessionId).then((text) => {
        if (sessionId !== sessionIdRef.current) return
        baseDraftRef.current = ''
        lastObservedSpeechRef.current = ''
        setDraft('')
        // Keep UI callbacks outside the consume promise: an exception in the
        // chat/TTS flow must not turn a successfully recognized turn into a
        // misleading "No se pudo completar" speech error.
        queueMicrotask(() => onCompletedRef.current?.(text))
      }).catch((error) => {
        if (sessionId === sessionIdRef.current) {
          setState({ status: 'error', error: { code: 'internal', message: error instanceof Error ? error.message : 'No se pudo completar el turno hablado.' } })
          consumingTurnRef.current = null
        }
      })
    }
    const register = async () => {
      const listeners = await Promise.all([
        listenSpeechState((event) => {
          if (event.sessionId !== sessionIdRef.current) return
          stateEventsRef.current += 1
          setState(event.state)
          if (event.state.status === 'completed') {
            const transcript = event.state.transcript.formattedText
            setDraft(mergeVoiceTextIntoDraft(baseDraftRef.current, transcript))
            sessionIdRef.current = null
            consumingTurnRef.current = null
            setVisiblePartialText('')
            queueMicrotask(() => onCompletedRef.current?.(transcript))
          } else if (event.state.status === 'error') {
            sessionIdRef.current = null
            setVisiblePartialText('')
          }
        }),
        listenSpeechPartial((event) => {
          if (event.sessionId !== sessionIdRef.current) return
          if (event.confirmedText !== confirmedTextRef.current) {
            confirmedTextRef.current = event.confirmedText
            visiblePartialTextRef.current = ''
          }
          visiblePartialTextRef.current = stabilizePartialTranscript(
            visiblePartialTextRef.current,
            event.partialText,
          )
          setVisiblePartialText(visiblePartialTextRef.current)
          const observedSpeech = `${confirmedTextRef.current} ${visiblePartialTextRef.current}`.trim()
          if (!observedSpeech) return
          setDraft(mergeVoiceTextIntoDraft(baseDraftRef.current, observedSpeech))
          // In conversation mode only arm the silence timer after sherpa has
          // confirmed an endpoint. A partial hypothesis is not yet available
          // to consume_speech_turn and would otherwise produce a misleading
          // generic turn error for short questions.
          if (pauseDetectionMs && confirmedTextRef.current.trim() && hasNewRecognizedSpeech(lastObservedSpeechRef.current, observedSpeech)) {
            lastObservedSpeechRef.current = observedSpeech
            if (silenceTimerRef.current !== null) window.clearTimeout(silenceTimerRef.current)
            silenceTimerRef.current = window.setTimeout(() => {
              silenceTimerRef.current = null
              completeDetectedTurn(event.sessionId)
            }, pauseDetectionMs)
          }
        }),
        listenSpeechSegments((event) => {
          if (event.sessionId !== sessionIdRef.current) return
          const observedSpeech = event.transcript.formattedText.trim()
          setDraft(mergeVoiceTextIntoDraft(baseDraftRef.current, observedSpeech))
          if (pauseDetectionMs && hasNewRecognizedSpeech(lastObservedSpeechRef.current, observedSpeech)) {
            lastObservedSpeechRef.current = observedSpeech
            if (silenceTimerRef.current !== null) window.clearTimeout(silenceTimerRef.current)
            silenceTimerRef.current = window.setTimeout(() => {
              silenceTimerRef.current = null
              completeDetectedTurn(event.sessionId)
            }, pauseDetectionMs)
          }
        }),
      ])
      if (disposed) listeners.forEach((unlisten) => unlisten())
      else unlisteners.push(...listeners)
    }
    void register().catch(() => {
      if (!disposed) setState({ status: 'error', error: { code: 'internal', message: 'No se pudieron escuchar los eventos de voz.' } })
    })
    return () => {
      disposed = true
      unlisteners.forEach((unlisten) => unlisten())
      if (silenceTimerRef.current !== null) window.clearTimeout(silenceTimerRef.current)
      const sessionId = sessionIdRef.current
      sessionIdRef.current = null
      consumingTurnRef.current = null
      if (sessionId && !keepsSessionRef.current) void cancelSpeechSession(sessionId).catch(() => undefined)
    }
  }, [continuousSession, pauseDetectionMs, setDraft])

  useEffect(() => {
    if (state.status !== 'recording') return
    const timer = window.setInterval(() => {
      setState((current) => current.status === 'recording'
        ? { ...current, elapsedMs: current.elapsedMs + 1_000 }
        : current)
    }, 1_000)
    return () => window.clearInterval(timer)
  }, [state.status])

  const start = useCallback(async () => {
    if (!speechRecognition.enabled) {
      setState({
        status: 'error',
        error: {
          code: 'internal',
          message: 'Activa el reconocimiento de voz en Configuraciones → Voz.',
        },
      })
      return false
    }
    setState({ status: 'preparing' })
    try {
      await prepareSpeechModel(speechRecognition, 'device')
      setIsModelReady(true)
      setModelPreparationError(null)
    } catch (error) {
      const message = error instanceof Error ? error.message : 'No se pudo preparar el reconocimiento de voz.'
      setIsModelReady(false)
      setModelPreparationError(message)
      setState({ status: 'error', error: { code: 'internal', message } })
      return false
    }
    let currentCapabilities = capabilities
    let currentAudioInput = audioInput
    if (!currentCapabilities || !currentAudioInput) {
      setState({ status: 'preparing' })
      try {
        const [capabilitiesResult, audioResult] = await Promise.allSettled([
          getSpeechCapabilities(),
          probeSpeechAudioInput(),
        ])
        if (capabilitiesResult.status === 'rejected') {
          throw capabilitiesResult.reason
        }
        currentCapabilities = capabilitiesResult.value
        setCapabilities(currentCapabilities)
        if (audioResult.status === 'fulfilled') {
          currentAudioInput = audioResult.value
          setAudioInput(currentAudioInput)
        }
      } catch (error) {
        setState({
          status: 'error',
          error: {
            code: 'internal',
            message: error instanceof Error ? error.message : environmentErrorRef.current || (typeof error === 'string' ? error : 'No se pudo comprobar el runtime de voz.'),
          },
        })
        return false
      }
    }
    if (!currentCapabilities.supported && currentCapabilities.platform !== 'windows' && currentCapabilities.platform !== 'android') {
      setState({
        status: 'error',
        error: {
          code: 'unsupported-platform',
          message: environmentErrorRef.current || 'El reconocimiento de voz no está disponible en esta plataforma.',
        },
      })
      return false
    }
    if (currentCapabilities.permission === 'denied') {
      setState({
        status: 'error',
        error: {
          code: 'permission-denied',
          message: 'Habilita el permiso de micrófono de Notia en la configuración del sistema.',
        },
      })
      return false
    }
    // Recording only the computer audio does not need a microphone.
    if (captureMicrophone && !currentAudioInput) {
      setState({
        status: 'error',
        error: { code: 'microphone-unavailable', message: 'No se pudo comprobar el micrófono.' },
      })
      return false
    }
    if (captureMicrophone && currentAudioInput && currentCapabilities.permission === 'granted' && !currentAudioInput.available) {
      setState({
        status: 'error',
        error: {
          code: 'microphone-unavailable',
          message: currentAudioInput.errorMessage || 'No hay un micrófono disponible.',
        },
      })
      return false
    }
    baseDraftRef.current = draft
    lastObservedSpeechRef.current = ''
    confirmedTextRef.current = ''
    visiblePartialTextRef.current = ''
    setVisiblePartialText('')
    if (silenceTimerRef.current !== null) {
      window.clearTimeout(silenceTimerRef.current)
      silenceTimerRef.current = null
    }
    setState({ status: 'preparing' })
    try {
      const result = await startSpeechSession({
        language: speechRecognition.language,
        diarizationEnabled: !continuousSession,
        maxDurationSeconds,
        captureSystemAudio,
        captureMicrophone,
        ...(expectedSpeakers ? { expectedSpeakers } : {}),
        ...(meeting ? { meeting } : {}),
      })
      sessionIdRef.current = result.sessionId
      setState({ status: 'recording', elapsedMs: 0, hasSpeech: false })
      return true
    } catch (error) {
      const message = error instanceof Error
        ? error.message
        : typeof error === 'string'
          ? error
          : (typeof error === 'object' && error !== null && 'message' in error && typeof error.message === 'string'
              ? error.message
              : 'No se pudo iniciar el micrófono.')
      setState({ status: 'error', error: { code: 'internal', message } })
      return false
    }
  }, [audioInput, capabilities, captureMicrophone, captureSystemAudio, continuousSession, draft, expectedSpeakers, maxDurationSeconds, meeting, speechRecognition])

  /**
   * Follows `sessionId` from now on without reading its state: its events
   * reach this hook and pause, stop and cancel act on it. Whether it is the
   * followed session; `false` when another one is.
   */
  const holdSession = useCallback((sessionId: string) => {
    if (sessionIdRef.current && sessionIdRef.current !== sessionId) return false
    if (!sessionIdRef.current) {
      sessionIdRef.current = sessionId
      confirmedTextRef.current = ''
      visiblePartialTextRef.current = ''
    }
    return true
  }, [])

  /**
   * Follows a session that kept recording or separating speakers while its
   * view was closed, and reads its state. Following it already (a dismissed
   * error leaves the session here) reads the state again. Resolves whether
   * the session is followed; `false` when it ended or another one is.
   */
  const attach = useCallback(async (sessionId: string) => {
    if (!holdSession(sessionId)) return false
    const eventsBefore = stateEventsRef.current
    try {
      const current = await getSpeechSessionState(sessionId)
      if (sessionIdRef.current !== sessionId) return false
      // An event that arrived meanwhile is newer than this answer.
      if (stateEventsRef.current === eventsBefore) setState(current)
      return true
    } catch {
      // The session ended meanwhile; the meeting shows its result.
      if (sessionIdRef.current === sessionId) sessionIdRef.current = null
      return false
    }
  }, [holdSession])

  const invokeForCurrentSession = useCallback(async (operation: (sessionId: string) => Promise<void>) => {
    const sessionId = sessionIdRef.current
    if (!sessionId) return
    try {
      await operation(sessionId)
    } catch (error) {
      setState({ status: 'error', error: { code: 'internal', message: error instanceof Error ? error.message : 'Fallo la sesion de voz.' } })
    }
  }, [])

  const resume = useCallback(async () => {
    const sessionId = sessionIdRef.current
    if (!sessionId) return false
    try {
      await resumeSpeechSession(sessionId)
      if (consumingTurnRef.current === sessionId) consumingTurnRef.current = null
      return true
    } catch (error) {
      setState({ status: 'error', error: { code: 'internal', message: error instanceof Error ? error.message : 'No se pudo reanudar el micrófono.' } })
      throw error
    }
  }, [])

  const cancel = useCallback(async () => {
    const sessionId = sessionIdRef.current
    sessionIdRef.current = null
    lastObservedSpeechRef.current = ''
    confirmedTextRef.current = ''
    visiblePartialTextRef.current = ''
    setVisiblePartialText('')
    if (sessionId) await cancelSpeechSession(sessionId).catch(() => undefined)
    setDraft(baseDraftRef.current)
    setState(INITIAL_STATE)
  }, [setDraft])

  return {
    capabilities,
    audioInput,
    sherpaRuntime,
    state,
    visiblePartialText,
    isModelReady,
    modelPreparationError,
    isActive: ACTIVE_STATUSES.has(state.status),
    start,
    pause: () => invokeForCurrentSession(pauseSpeechSession),
    resume,
    stop: () => invokeForCurrentSession(stopSpeechSession),
    cancel,
    attach,
    holdSession,
    dismissError: () => setState(INITIAL_STATE),
  }
}

/**
 * Dictation hook of this interface. The app records and recognizes through
 * the native speech session of its own device, also as a client of a host
 * (a tablet dictates and transcribes its meetings itself). A browser
 * connected to a Notia server, whose backend cannot record, records here and
 * sends the audio; a published Task Manager, which offers no dictation,
 * gets that hook idle. The transport is installed before the interface loads
 * and never changes, so the choice is fixed per page.
 */
export const useVoiceTranscription: typeof useLocalVoiceTranscription =
  backendKind() !== 'local' && !backendSupports('start_speech_session')
    ? useRemoteVoiceTranscription
    : useLocalVoiceTranscription
