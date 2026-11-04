import { useCallback, useEffect, useRef, useState } from 'react'
import { useAppSelector } from '../../../../store/hooks'
import { selectSpeechRecognitionSettings } from '../../../../features/preferences/preferencesSelectors'
import { backendSupports } from '../../../../services/transport'
import { cancelRemoteSpeech, prepareSpeechModel, sendRemoteSpeechChunk } from '../../../../services/speech/speechService'
import { bytesToBase64, startRemoteSpeechCapture, type RemoteSpeechCapture } from '../../../../services/speech/remoteSpeechCapture'
import { mergeVoiceTextIntoDraft } from '../../../../services/speech/speechTranscript'
import type { SpeechAudioInputStatus, SpeechCapabilities, SpeechSessionState, SherpaRuntimeStatus } from '../../../../services/speech/speechTypes'

const INITIAL_STATE: SpeechSessionState = { status: 'idle' }
const ACTIVE_STATUSES = new Set<SpeechSessionState['status']>(['preparing', 'recording', 'paused', 'finalizing'])

interface UseRemoteVoiceTranscriptionInput {
  draft: string
  setDraft: (value: string) => void
  onCompleted?: (text: string) => void
}

function messageOf(error: unknown, fallback: string): string {
  if (error instanceof Error) return error.message
  if (typeof error === 'string' && error.trim()) return error
  if (error && typeof error === 'object' && 'message' in error && typeof error.message === 'string') return error.message
  return fallback
}

/**
 * Dictation from a browser connected to a Notia server: the microphone of
 * this device records, the server recognizes the text when recording stops.
 * Same shape as the local hook; there are no partial results nor speakers.
 */
export function useRemoteVoiceTranscription({ draft, setDraft, onCompleted }: UseRemoteVoiceTranscriptionInput) {
  const speechRecognition = useAppSelector(selectSpeechRecognitionSettings)
  const devicePreferencesLoaded = useAppSelector((appState) => appState.preferences.devicePreferencesLoaded)
  const [state, setState] = useState<SpeechSessionState>(INITIAL_STATE)
  const [isModelReady, setIsModelReady] = useState(false)
  const [modelPreparationError, setModelPreparationError] = useState<string | null>(null)
  const captureRef = useRef<RemoteSpeechCapture | null>(null)
  const sessionIdRef = useRef<string | null>(null)
  const sequenceRef = useRef(0)
  const queueRef = useRef<Promise<string | null>>(Promise.resolve(null))
  const baseDraftRef = useRef('')
  const onCompletedRef = useRef(onCompleted)
  const supported = backendSupports('speech_remote_audio')

  useEffect(() => {
    onCompletedRef.current = onCompleted
  }, [onCompleted])

  useEffect(() => {
    let current = true
    if (!supported || !devicePreferencesLoaded || !speechRecognition.enabled) {
      setIsModelReady(false)
      return () => { current = false }
    }
    void prepareSpeechModel(speechRecognition).then(() => {
      if (!current) return
      setIsModelReady(true)
      setModelPreparationError(null)
    }).catch((error: unknown) => {
      if (!current) return
      setIsModelReady(false)
      setModelPreparationError(messageOf(error, 'No se pudo preparar el reconocimiento de voz.'))
    })
    return () => { current = false }
  }, [devicePreferencesLoaded, speechRecognition, supported])

  useEffect(() => {
    if (state.status !== 'recording') return
    const timer = window.setInterval(() => {
      setState((current) => current.status === 'recording' ? { ...current, elapsedMs: current.elapsedMs + 1_000 } : current)
    }, 1_000)
    return () => window.clearInterval(timer)
  }, [state.status])

  const release = useCallback(async (): Promise<Uint8Array> => {
    const capture = captureRef.current
    captureRef.current = null
    return capture ? capture.stop() : new Uint8Array()
  }, [])

  const fail = useCallback((error: unknown) => {
    const sessionId = sessionIdRef.current
    sessionIdRef.current = null
    void release()
    if (sessionId) void cancelRemoteSpeech(sessionId).catch(() => undefined)
    setState({ status: 'error', error: { code: 'internal', message: messageOf(error, 'Falló el dictado.') } })
  }, [release])

  /** Chunks go out one at a time, in order, as the server requires. */
  const send = useCallback((pcm: Uint8Array, sampleRate: number, last: boolean) => {
    const sessionId = sessionIdRef.current
    if (!sessionId) return Promise.resolve(null)
    const sequence = sequenceRef.current
    sequenceRef.current += 1
    queueRef.current = queueRef.current.then(() => sendRemoteSpeechChunk({
      sessionId,
      sequence,
      sampleRate,
      last,
      dataBase64: bytesToBase64(pcm),
    }))
    return queueRef.current
  }, [])

  const start = useCallback(async () => {
    if (!supported) {
      setState({ status: 'error', error: { code: 'unsupported-platform', message: 'El servidor no ofrece dictado.' } })
      return false
    }
    if (!speechRecognition.enabled) {
      setState({ status: 'error', error: { code: 'internal', message: 'Activa el reconocimiento de voz en Configuraciones → Voz.' } })
      return false
    }
    setState({ status: 'preparing' })
    baseDraftRef.current = draft
    sessionIdRef.current = crypto.randomUUID()
    sequenceRef.current = 0
    queueRef.current = Promise.resolve(null)
    try {
      captureRef.current = await startRemoteSpeechCapture((pcm, sampleRate) => {
        send(pcm, sampleRate, false).catch(fail)
      })
      setState({ status: 'recording', elapsedMs: 0, hasSpeech: false })
      return true
    } catch (error) {
      sessionIdRef.current = null
      setState({ status: 'error', error: { code: 'microphone-unavailable', message: messageOf(error, 'No se pudo usar el micrófono.') } })
      return false
    }
  }, [draft, fail, speechRecognition.enabled, send, supported])

  const stop = useCallback(async () => {
    const capture = captureRef.current
    if (!capture || !sessionIdRef.current) return
    setState({ status: 'finalizing' })
    try {
      const tail = await release()
      const text = (await send(tail, capture.sampleRate, true)) ?? ''
      sessionIdRef.current = null
      setDraft(mergeVoiceTextIntoDraft(baseDraftRef.current, text))
      setState({ status: 'completed', transcript: { text, segments: [], speakerCount: text ? 1 : 0, formattedText: text } })
      queueMicrotask(() => onCompletedRef.current?.(text))
    } catch (error) {
      fail(error)
    }
  }, [fail, release, send, setDraft])

  const pause = useCallback(async () => {
    captureRef.current?.pause()
    setState((current) => current.status === 'recording' ? { status: 'paused', elapsedMs: current.elapsedMs } : current)
  }, [])

  const resume = useCallback(async () => {
    if (!captureRef.current) return false
    captureRef.current.resume()
    setState((current) => current.status === 'paused' ? { status: 'recording', elapsedMs: current.elapsedMs, hasSpeech: false } : current)
    return true
  }, [])

  const cancel = useCallback(async () => {
    const sessionId = sessionIdRef.current
    sessionIdRef.current = null
    await release()
    if (sessionId) await cancelRemoteSpeech(sessionId).catch(() => undefined)
    setDraft(baseDraftRef.current)
    setState(INITIAL_STATE)
  }, [release, setDraft])

  useEffect(() => () => {
    const sessionId = sessionIdRef.current
    sessionIdRef.current = null
    void captureRef.current?.stop()
    captureRef.current = null
    if (sessionId) void cancelRemoteSpeech(sessionId).catch(() => undefined)
  }, [])

  return {
    capabilities: null as SpeechCapabilities | null,
    audioInput: null as SpeechAudioInputStatus | null,
    sherpaRuntime: null as SherpaRuntimeStatus | null,
    state,
    visiblePartialText: '',
    isModelReady,
    modelPreparationError,
    isActive: ACTIVE_STATUSES.has(state.status),
    start,
    pause,
    resume,
    stop,
    cancel,
    // Meeting, the only session that outlives its view, is not offered in a
    // browser connected to a server.
    attach: async () => false,
    holdSession: () => false,
    dismissError: () => setState(INITIAL_STATE),
  }
}
