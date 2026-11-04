use crate::dto::speech::{
    DiarizedTranscriptDto, SpeechCapabilitiesDto, SpeechErrorDto, SpeechModelStatusDto,
    SpeechPartialEventDto, SpeechSegmentsEventDto, SpeechSessionEventDto, SpeechSessionStateDto,
    SpeechTranscriptSegmentDto,
};
use std::sync::Mutex;
#[cfg(any(target_os = "windows", target_os = "android"))]
use std::sync::{Arc, Mutex as StdMutex};
#[cfg(any(target_os = "windows", target_os = "android"))]
use std::time::Instant;
#[cfg(any(target_os = "windows", target_os = "android"))]
use crate::host::{AppHandle, Emitter, Manager};
#[cfg(any(target_os = "windows", target_os = "android"))]
use crate::services::whisper_recognizer::{WhisperAsrConfig, WhisperVadRecognizer};

pub const MAX_SPEECH_SESSION_SECONDS: u32 = 12 * 60 * 60;
#[cfg(any(target_os = "windows", target_os = "android"))]
const DIARIZATION_WINDOW_SAMPLES: usize = 16_000 * 15 * 60;
/// How far before the 15 minutes a window may end to cut in a pause.
#[cfg(any(target_os = "windows", target_os = "android"))]
const WINDOW_PAUSE_SEARCH_SAMPLES: usize = 16_000 * 120;
/// A last window shorter than this joins the previous one.
#[cfg(any(target_os = "windows", target_os = "android"))]
const WINDOW_MIN_TAIL_SAMPLES: usize = 16_000 * 60;
/// Silence between two lines that counts as a pause to cut at.
#[cfg(any(target_os = "windows", target_os = "android"))]
const WINDOW_MIN_PAUSE_SAMPLES: u64 = 16_000 / 5;
#[cfg(any(target_os = "windows", target_os = "android"))]
const GLOBAL_SPEAKER_MATCH_THRESHOLD: f32 = 0.72;
#[cfg(any(target_os = "windows", target_os = "android"))]
pub(crate) type PreloadedRecognizer = Arc<StdMutex<Option<WhisperVadRecognizer>>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpeechPhase {
    Idle,
    Preparing,
    Recording,
    Paused,
    Finalizing,
}

const ACTIVE_SPEECH_PHASES: [SpeechPhase; 3] = [
    SpeechPhase::Recording,
    SpeechPhase::Paused,
    SpeechPhase::Finalizing,
];

pub struct SpeechRuntimeState {
    pub phase: Mutex<SpeechPhase>,
    #[cfg(any(target_os = "windows", target_os = "android"))]
    active_session: Mutex<Option<ActivePlatformSpeechSession>>,
    #[cfg(any(target_os = "windows", target_os = "android"))]
    preloaded_recognizer: PreloadedRecognizer,
    /// Serializes the startup preload and interface requests so the same
    /// model is never loaded twice at once.
    #[cfg(any(target_os = "windows", target_os = "android"))]
    preparation: StdMutex<()>,
    /// Audio check before recording: the sources open only to measure them.
    #[cfg(any(target_os = "windows", target_os = "android"))]
    monitor: Mutex<Option<AudioMonitor>>,
    /// Session whose speaker separation the person asked to skip.
    skip_diarization: Mutex<Option<String>>,
    /// Session that stopped recording and is separating its speakers.
    #[cfg(any(target_os = "windows", target_os = "android"))]
    finalizing_session: Mutex<Option<String>>,
}

/// Longest an audio check stays open without being stopped.
#[cfg(any(target_os = "windows", target_os = "android"))]
const AUDIO_MONITOR_LIFETIME: std::time::Duration = std::time::Duration::from_secs(120);

#[cfg(any(target_os = "windows", target_os = "android"))]
struct AudioMonitor {
    id: String,
    // Dropped first: it stops emitting before the capture closes.
    _levels: crate::services::speech_levels::LevelReporter,
    _capture: crate::services::speech_audio::PlatformAudioCapture,
}

#[cfg(any(target_os = "windows", target_os = "android"))]
pub(crate) fn recognizer_cache(state: &SpeechRuntimeState) -> PreloadedRecognizer {
    Arc::clone(&state.preloaded_recognizer)
}

#[cfg(any(target_os = "windows", target_os = "android"))]
pub fn transcribe_external_audio(
    app: &AppHandle,
    cache: &PreloadedRecognizer,
    samples: &[f32],
) -> Result<String, String> {
    use crate::services::speech_worker::StreamingRecognizer;

    let model = preferred_asr_model(app)?;
    let mut recognizer = match cache
        .lock()
        .map_err(|_| "No se pudo acceder al modelo precargado.".to_string())?
        .take()
    {
        Some(recognizer) if recognizer.matches(&model) => recognizer,
        Some(_) | None => load_recognizer(app, &model)?,
    };
    let result = (|| {
        let mut text = String::new();
        for chunk in samples.chunks(3_200) {
            let update = recognizer.accept_waveform(chunk)?;
            if commit_external_update(&mut text, &update, false) {
                recognizer.reset_after_endpoint()?;
            }
        }
        let final_update = recognizer.finish()?;
        commit_external_update(&mut text, &final_update, true);
        let text = text.trim().to_string();
        if text.is_empty() {
            return Err("No se detecto voz en el audio de Telegram.".to_string());
        }
        Ok(text)
    })();
    let reset_result = recognizer.reset_session();
    if reset_result.is_ok() {
        if let Ok(mut slot) = cache.lock() {
            *slot = Some(recognizer);
        }
    }
    reset_result?;
    result
}

#[cfg(any(target_os = "windows", target_os = "android"))]
fn load_recognizer(
    app: &AppHandle,
    model: &WhisperAsrConfig,
) -> Result<WhisperVadRecognizer, String> {
    let runtime = crate::services::whisper_runtime::runtime_path(app)?;
    // Silero VAD runs on the sherpa-onnx runtime that diarization also uses.
    let sherpa_runtime = crate::services::sherpa_runtime::resolve_platform_runtime_path(app)?;
    WhisperVadRecognizer::load(&runtime, &sherpa_runtime, model)
}

/// The recognition model with the saved language, for audio that does not
/// come from a session started by the interface.
#[cfg(any(target_os = "windows", target_os = "android"))]
fn preferred_asr_model(app: &AppHandle) -> Result<WhisperAsrConfig, String> {
    let selection = SavedAsrSelection::read(app);
    crate::services::speech_model_repository::resolve_asr_model(app, &selection.language)
}

/// The `speechRecognition` device preference, already normalized by the backend.
#[cfg(any(target_os = "windows", target_os = "android"))]
struct SavedAsrSelection {
    language: String,
    enabled: bool,
}

#[cfg(any(target_os = "windows", target_os = "android"))]
impl SavedAsrSelection {
    fn read(app: &AppHandle) -> Self {
        let preferences = crate::device_preferences::section(app, "speechRecognition");
        Self {
            language: preferences["language"].as_str().unwrap_or_default().to_string(),
            enabled: preferences["enabled"].as_bool() != Some(false),
        }
    }
}

/// Loads the saved recognition model while Notia starts, so dictation and
/// Meeting find it resident. Runs on its own thread: the window never waits.
pub(crate) fn init_preload() -> crate::host::plugin::TauriPlugin<crate::host::Wry> {
    crate::host::plugin::Builder::new("notia-speech-preload")
        .setup(|app, _api| {
            #[cfg(any(target_os = "windows", target_os = "android"))]
            preload_at_startup(app.clone());
            #[cfg(not(any(target_os = "windows", target_os = "android")))]
            let _ = app;
            Ok(())
        })
        .build()
}

#[cfg(any(target_os = "windows", target_os = "android"))]
fn preload_at_startup(app: AppHandle) {
    let selection = SavedAsrSelection::read(&app);
    if !selection.enabled {
        log::info!("[notia:speech] startup preload skipped: speech recognition disabled");
        return;
    }
    let spawned = std::thread::Builder::new()
        .name("notia-speech-preload".to_string())
        .spawn(move || {
            let started_at = Instant::now();
            match prepare_recognizer(&app, &selection.language) {
                Ok(()) => log::info!(
                    "[notia:speech] startup preload ready elapsed_ms={}",
                    started_at.elapsed().as_millis()
                ),
                Err(message) => log::warn!("[notia:speech] startup preload failed: {message}"),
            }
        });
    if let Err(error) = spawned {
        log::warn!("[notia:speech] startup preload thread not started: {error}");
    }
}

fn commit_external_update(
    target: &mut String,
    update: &crate::services::speech_worker::RecognitionUpdate,
    is_final: bool,
) -> bool {
    if update.endpoint_detected || is_final {
        append_text(target, &update.text);
    }
    update.endpoint_detected
}

#[cfg(any(target_os = "windows", target_os = "android"))]
struct ActivePlatformSpeechSession {
    session_id: String,
    // Dropped first: it stops emitting before the capture closes.
    _levels: Option<crate::services::speech_levels::LevelReporter>,
    audio: SessionAudio,
    meter: crate::services::speech_audio::SharedCaptureMeter,
    worker: crate::services::speech_worker::SpeechWorker,
    confirmed: Arc<StdMutex<ConfirmedSpeech>>,
}

/// Where a session's audio comes from.
#[cfg(any(target_os = "windows", target_os = "android"))]
enum SessionAudio {
    /// The microphone and the computer's audio, recorded live.
    Capture(crate::services::speech_audio::PlatformAudioCapture),
    /// A file decoded into the session; it cannot be paused. The feeder is
    /// held so dropping the session stops it.
    File { _feeder: crate::services::speech_file::FileFeeder },
}

#[cfg(any(target_os = "windows", target_os = "android"))]
impl SessionAudio {
    fn pause(&self) -> Result<(), String> {
        match self {
            Self::Capture(capture) => capture.pause(),
            Self::File { .. } => Err(FILE_SESSION_CANNOT_PAUSE.to_string()),
        }
    }

    fn resume(&self) -> Result<(), String> {
        match self {
            Self::Capture(capture) => capture.resume(),
            Self::File { .. } => Err(FILE_SESSION_CANNOT_PAUSE.to_string()),
        }
    }
}

#[cfg(any(target_os = "windows", target_os = "android"))]
const FILE_SESSION_CANNOT_PAUSE: &str = "La transcripción de un archivo no se puede pausar.";

/// What the session confirmed while recording: the whole text and each line
/// with the samples it spans, which the speaker separation reuses.
#[cfg(any(target_os = "windows", target_os = "android"))]
#[derive(Default)]
struct ConfirmedSpeech {
    text: String,
    lines: Vec<ConfirmedLine>,
}

#[cfg(any(target_os = "windows", target_os = "android"))]
#[derive(Debug, Clone, PartialEq)]
struct ConfirmedLine {
    span: crate::services::speech_worker::SampleSpan,
    text: String,
}

#[cfg(any(target_os = "windows", target_os = "android"))]
impl ConfirmedSpeech {
    /// Adds a confirmed utterance and returns the text added. An utterance
    /// without its samples is placed where the previous one ended.
    fn confirm(&mut self, text: &str, span: Option<crate::services::speech_worker::SampleSpan>) -> String {
        let appended = append_text(&mut self.text, text);
        if !appended.is_empty() {
            let span = span.unwrap_or_else(|| {
                let end = self.lines.last().map_or(0, |line| line.span.end);
                crate::services::speech_worker::SampleSpan { start: end, end }
            });
            self.lines.push(ConfirmedLine { span, text: appended.clone() });
        }
        appended
    }
}

impl Default for SpeechRuntimeState {
    fn default() -> Self {
        Self {
            phase: Mutex::new(SpeechPhase::Idle),
            #[cfg(any(target_os = "windows", target_os = "android"))]
            active_session: Mutex::new(None),
            #[cfg(any(target_os = "windows", target_os = "android"))]
            preloaded_recognizer: Arc::new(StdMutex::new(None)),
            #[cfg(any(target_os = "windows", target_os = "android"))]
            preparation: StdMutex::new(()),
            #[cfg(any(target_os = "windows", target_os = "android"))]
            monitor: Mutex::new(None),
            skip_diarization: Mutex::new(None),
            #[cfg(any(target_os = "windows", target_os = "android"))]
            finalizing_session: Mutex::new(None),
        }
    }
}

#[cfg(any(target_os = "windows", target_os = "android"))]
pub fn prepare_recognizer(app: &AppHandle, language: &str) -> Result<(), String> {
    let state = app.state::<SpeechRuntimeState>();
    // A request that arrives while the startup preload is loading waits here
    // and then finds the model already resident.
    let _preparing = state
        .preparation
        .lock()
        .map_err(|_| "No se pudo coordinar la preparación del modelo de voz.".to_string())?;
    // A recording holds the model and gives it back when it ends; a view that
    // opens meanwhile must not load a second copy.
    if state.active_session.lock().is_ok_and(|slot| slot.is_some()) {
        return Ok(());
    }
    let resolved = crate::services::speech_model_repository::resolve_asr_model(app, language)?;
    {
        let cache = state
            .preloaded_recognizer
            .lock()
            .map_err(|_| "No se pudo acceder al modelo precargado.".to_string())?;
        if cache
            .as_ref()
            .is_some_and(|recognizer| recognizer.matches(&resolved))
        {
            return Ok(());
        }
    }
    let recognizer = load_recognizer(app, &resolved)?;
    *state
        .preloaded_recognizer
        .lock()
        .map_err(|_| "No se pudo guardar el modelo precargado.".to_string())? = Some(recognizer);
    log::info!("[notia:speech] offline recognizer prepared");
    Ok(())
}

#[cfg(not(any(target_os = "windows", target_os = "android")))]
pub fn prepare_recognizer(_app: &crate::host::AppHandle, _language: &str) -> Result<(), String> {
    Err(not_integrated_error())
}

pub fn runtime_integrated() -> bool {
    cfg!(any(target_os = "windows", target_os = "android"))
}

/// What a session records besides the recognizer model.
#[cfg(any(target_os = "windows", target_os = "android"))]
pub struct SessionCapture {
    pub sources: crate::services::speech_audio::CaptureSources,
    pub max_duration_seconds: u32,
    pub expected_speakers: Option<u32>,
    /// The session records a Meeting: it emits `speech://levels`, and its
    /// lines reach the interface through the meeting, so `speech://partial`
    /// leaves out the confirmed text, which grows for hours.
    pub meeting: bool,
}

#[cfg(any(target_os = "windows", target_os = "android"))]
pub fn start_platform_session(
    app: &AppHandle,
    state: &SpeechRuntimeState,
    session_id: String,
    model: WhisperAsrConfig,
    diarization_model: Option<crate::services::speech_model_repository::ResolvedDiarizationModel>,
    capture: SessionCapture,
) -> Result<(), String> {
    let mut slot = state
        .active_session
        .lock()
        .map_err(|_| "No se pudo bloquear la sesion de voz.".to_string())?;
    if slot.is_some() {
        return Err("Ya existe una sesion de voz activa.".to_string());
    }
    stop_any_audio_monitor(state);
    if let Ok(mut skip) = state.skip_diarization.lock() {
        *skip = None;
    }
    let SessionCapture { sources, max_duration_seconds, expected_speakers, meeting } = capture;
    let buffer = crate::services::speech_audio::create_shared_pcm_buffer();
    let meter: crate::services::speech_audio::SharedCaptureMeter = Arc::default();
    let audio_capture = crate::services::speech_audio::PlatformAudioCapture::start(
        Some(Arc::clone(&buffer)),
        sources,
        Some(Arc::clone(&meter)),
        &crate::services::audio_devices::for_capture(app),
    )?;
    let levels = meeting
        .then(|| {
            crate::services::speech_levels::LevelReporter::start(
                app.clone(),
                session_id.clone(),
                Arc::clone(&meter),
                sources,
                None,
            )
        })
        .transpose()?;
    let (worker, confirmed) = start_session_worker(
        app,
        state,
        &session_id,
        model,
        diarization_model,
        buffer,
        max_duration_seconds,
        SessionKind { expected_speakers, meeting, file: false },
    )?;
    *slot = Some(ActivePlatformSpeechSession {
        session_id,
        _levels: levels,
        audio: SessionAudio::Capture(audio_capture),
        meter,
        worker,
        confirmed,
    });
    Ok(())
}

/// A file a session transcribes: its decodable copy and its duration.
#[cfg(any(target_os = "windows", target_os = "android"))]
pub struct FileCapture {
    pub path: std::path::PathBuf,
    pub duration_ms: u64,
    pub expected_speakers: Option<u32>,
}

/// Starts a session that transcribes a file for a Meeting. It reports
/// `finalizing` with the share of the file transcribed, then separates the
/// speakers as a recording does.
#[cfg(any(target_os = "windows", target_os = "android"))]
pub fn start_file_session(
    app: &AppHandle,
    state: &SpeechRuntimeState,
    session_id: String,
    model: WhisperAsrConfig,
    diarization_model: Option<crate::services::speech_model_repository::ResolvedDiarizationModel>,
    file: FileCapture,
) -> Result<(), String> {
    let mut slot = state
        .active_session
        .lock()
        .map_err(|_| "No se pudo bloquear la sesion de voz.".to_string())?;
    if slot.is_some() {
        return Err("Ya existe una sesion de voz activa.".to_string());
    }
    stop_any_audio_monitor(state);
    if let Ok(mut skip) = state.skip_diarization.lock() {
        *skip = None;
    }
    // The archive holds the whole file, with room for a decoder that reads
    // a little more than the container announced.
    let max_duration_seconds = u32::try_from(file.duration_ms / 1_000 + 60)
        .unwrap_or(MAX_SPEECH_SESSION_SECONDS)
        .min(MAX_SPEECH_SESSION_SECONDS);
    let buffer = crate::services::speech_audio::create_shared_pcm_buffer();
    let meter: crate::services::speech_audio::SharedCaptureMeter = Arc::default();
    let (worker, confirmed) = start_session_worker(
        app,
        state,
        &session_id,
        model,
        diarization_model,
        Arc::clone(&buffer),
        max_duration_seconds,
        SessionKind { expected_speakers: file.expected_speakers, meeting: true, file: true },
    )?;
    // The slot stays locked until the session is in it, so a short file
    // cannot finish before its session exists.
    let feeder = crate::services::speech_file::FileFeeder::start(
        app.clone(),
        session_id.clone(),
        file.path,
        file.duration_ms,
        buffer,
        Arc::clone(&meter),
    )?;
    // Processing from now on, so an interface that asks for the session's
    // state right away can follow it (the worker's Ready reports it too).
    if let Ok(mut phase) = state.phase.lock() {
        *phase = SpeechPhase::Finalizing;
    }
    set_finalizing_session(state, Some(session_id.clone()));
    *slot = Some(ActivePlatformSpeechSession {
        session_id,
        _levels: None,
        audio: SessionAudio::File { _feeder: feeder },
        meter,
        worker,
        confirmed,
    });
    Ok(())
}

/// The whole file reached the recognizer: the session ends as a stopped
/// recording does, recognizing what is left and separating the speakers.
/// Nothing happens when the session was cancelled meanwhile.
#[cfg(any(target_os = "windows", target_os = "android"))]
pub(crate) fn finish_file_session(app: &AppHandle, session_id: &str) {
    let state = app.state::<SpeechRuntimeState>();
    let session = state.active_session.lock().ok().and_then(|mut slot| {
        if slot.as_ref().is_some_and(|session| session.session_id == session_id) {
            slot.take()
        } else {
            None
        }
    });
    let Some(ActivePlatformSpeechSession { worker, audio, .. }) = session else {
        return;
    };
    drop(audio);
    set_finalizing_session(&state, Some(session_id.to_string()));
    let finished = worker.stop().and_then(|()| worker.join());
    set_finalizing_session(&state, None);
    if finished.is_err() {
        if let Ok(mut phase) = state.phase.lock() {
            *phase = SpeechPhase::Idle;
        }
    }
    #[cfg(target_os = "android")]
    crate::mobile_continuity::end_android_work(
        app.state::<crate::mobile_continuity::ContinuityState>().inner(),
    );
}

/// What kind of session a worker serves.
#[cfg(any(target_os = "windows", target_os = "android"))]
struct SessionKind {
    expected_speakers: Option<u32>,
    meeting: bool,
    /// The audio comes from a file: the session reports `finalizing` with
    /// the file's progress instead of `recording`.
    file: bool,
}

/// Starts the recognizer worker of a session over `buffer`.
#[cfg(any(target_os = "windows", target_os = "android"))]
#[allow(clippy::too_many_arguments)]
fn start_session_worker(
    app: &AppHandle,
    state: &SpeechRuntimeState,
    session_id: &str,
    model: WhisperAsrConfig,
    diarization_model: Option<crate::services::speech_model_repository::ResolvedDiarizationModel>,
    buffer: crate::services::speech_audio::SharedPcmBuffer,
    max_duration_seconds: u32,
    kind: SessionKind,
) -> Result<(crate::services::speech_worker::SpeechWorker, Arc<StdMutex<ConfirmedSpeech>>), String> {
    let SessionKind { expected_speakers, meeting, file } = kind;
    let diarization_runtime_path = diarization_model
        .as_ref()
        .map(|_| crate::services::sherpa_runtime::resolve_platform_runtime_path(app))
        .transpose()?;
    let recognizer_app = app.clone();
    let final_pass_asr = model.clone();
    let recognizer_cache = Arc::clone(&state.preloaded_recognizer);
    let recycler_cache = Arc::clone(&recognizer_cache);
    let session_id = session_id.to_string();
    let started_at = Instant::now();
    let confirmed = Arc::new(StdMutex::new(ConfirmedSpeech::default()));
    let callback_app = app.clone();
    let callback_session_id = session_id.clone();
    let callback_confirmed = Arc::clone(&confirmed);
    let worker = crate::services::speech_worker::SpeechWorker::start_with_recycler(
        buffer,
        max_duration_seconds,
        move || {
            let cached = recognizer_cache
                .lock()
                .map_err(|_| "No se pudo acceder al modelo precargado.".to_string())?
                .take()
                .filter(|recognizer| recognizer.matches(&model));
            let mut recognizer = match cached {
                Some(recognizer) => recognizer,
                None => load_recognizer(&recognizer_app, &model)?,
            };
            recognizer.enable_live_partials();
            Ok(recognizer)
        },
        move |event| {
            handle_worker_event(
                &callback_app,
                &callback_session_id,
                started_at,
                &callback_confirmed,
                WorkerEventTarget { meeting, file },
                DiarizationSetup {
                    runtime_path: diarization_runtime_path.as_deref(),
                    model: diarization_model.as_ref(),
                    expected_speakers,
                    asr: &final_pass_asr,
                },
                event,
            );
        },
        move |recognizer| {
            if let Ok(mut slot) = recycler_cache.lock() {
                *slot = Some(recognizer);
            }
        },
    )?;
    Ok((worker, confirmed))
}

/// Position of the recording of `session_id`, without its pauses.
#[cfg(any(target_os = "windows", target_os = "android"))]
pub fn session_position_ms(state: &SpeechRuntimeState, session_id: &str) -> Result<u64, String> {
    let slot = state
        .active_session
        .lock()
        .map_err(|_| "No se pudo bloquear la sesion de voz.".to_string())?;
    slot.as_ref()
        .filter(|session| session.session_id == session_id)
        .map(|session| session.meter.position_ms())
        .ok_or_else(|| "La grabación ya terminó.".to_string())
}

#[cfg(not(any(target_os = "windows", target_os = "android")))]
pub fn session_position_ms(_state: &SpeechRuntimeState, _session_id: &str) -> Result<u64, String> {
    Err(not_integrated_error())
}

/// Opens `sources` only to measure them and emits their levels under a new
/// id until stopped, a recording starts or two minutes pass.
#[cfg(any(target_os = "windows", target_os = "android"))]
pub fn start_audio_monitor(
    app: &AppHandle,
    state: &SpeechRuntimeState,
    sources: crate::services::speech_audio::CaptureSources,
) -> Result<String, String> {
    if state
        .active_session
        .lock()
        .map_err(|_| "No se pudo bloquear la sesion de voz.".to_string())?
        .is_some()
    {
        return Err("No se puede probar el audio mientras se graba.".to_string());
    }
    stop_any_audio_monitor(state);
    let id = uuid::Uuid::new_v4().to_string();
    let meter: crate::services::speech_audio::SharedCaptureMeter = Arc::default();
    let devices = crate::services::audio_devices::for_capture(app);
    let capture = crate::services::speech_audio::PlatformAudioCapture::start(None, sources, Some(Arc::clone(&meter)), &devices)?;
    let expiry_app = app.clone();
    let expiry_id = id.clone();
    let levels = crate::services::speech_levels::LevelReporter::start(
        app.clone(),
        id.clone(),
        meter,
        sources,
        Some((
            AUDIO_MONITOR_LIFETIME,
            Box::new(move || {
                let _ = stop_audio_monitor(&expiry_app.state::<SpeechRuntimeState>(), &expiry_id);
            }),
        )),
    )?;
    *state
        .monitor
        .lock()
        .map_err(|_| "No se pudo bloquear la prueba de audio.".to_string())? =
        Some(AudioMonitor { id: id.clone(), _levels: levels, _capture: capture });
    Ok(id)
}

#[cfg(not(any(target_os = "windows", target_os = "android")))]
pub fn start_audio_monitor(
    _app: &crate::host::AppHandle,
    _state: &SpeechRuntimeState,
    _sources: crate::services::speech_audio::CaptureSources,
) -> Result<String, String> {
    Err(not_integrated_error())
}

/// Closes the audio check `id`; another check or none is left as is.
pub fn stop_audio_monitor(state: &SpeechRuntimeState, id: &str) -> Result<(), String> {
    #[cfg(any(target_os = "windows", target_os = "android"))]
    {
        let stopped = {
            let mut slot = state
                .monitor
                .lock()
                .map_err(|_| "No se pudo bloquear la prueba de audio.".to_string())?;
            if slot.as_ref().is_some_and(|monitor| monitor.id == id) { slot.take() } else { None }
        };
        drop(stopped);
    }
    #[cfg(not(any(target_os = "windows", target_os = "android")))]
    let _ = (state, id);
    Ok(())
}

#[cfg(any(target_os = "windows", target_os = "android"))]
fn stop_any_audio_monitor(state: &SpeechRuntimeState) {
    let stopped = state.monitor.lock().ok().and_then(|mut slot| slot.take());
    drop(stopped);
}

/// Asks to finish `session_id` without separating its speakers. Only while
/// it is finalizing.
pub fn skip_diarization(state: &SpeechRuntimeState, session_id: &str) -> Result<(), String> {
    let finalizing = state
        .phase
        .lock()
        .map(|phase| *phase == SpeechPhase::Finalizing)
        .unwrap_or(false);
    if !finalizing {
        return Err("La grabación no está separando hablantes.".to_string());
    }
    *state
        .skip_diarization
        .lock()
        .map_err(|_| "No se pudo bloquear el estado de voz.".to_string())? = Some(session_id.to_string());
    Ok(())
}

#[cfg(any(target_os = "windows", target_os = "android"))]
fn diarization_skipped(app: &AppHandle, session_id: &str) -> bool {
    app.state::<SpeechRuntimeState>()
        .skip_diarization
        .lock()
        .map(|skip| skip.as_deref() == Some(session_id))
        .unwrap_or(false)
}

#[cfg(any(target_os = "windows", target_os = "android"))]
pub fn consume_platform_turn(
    state: &SpeechRuntimeState,
    session_id: &str,
) -> Result<String, String> {
    let slot = state
        .active_session
        .lock()
        .map_err(|_| "No se pudo bloquear la sesion de voz.".to_string())?;
    let session = slot
        .as_ref()
        .filter(|session| session.session_id == session_id)
        .ok_or_else(|| "La sesion de voz no coincide con la sesion activa.".to_string())?;
    session.audio.pause()?;
    session.worker.pause()?;
    let mut confirmed = session
        .confirmed
        .lock()
        .map_err(|_| "No se pudo obtener el turno reconocido.".to_string())?;
    let text = std::mem::take(&mut confirmed.text).trim().to_string();
    confirmed.lines.clear();
    drop(confirmed);
    if text.is_empty() {
        session.worker.resume()?;
        session.audio.resume()?;
        return Err("No se detecto voz en este turno.".to_string());
    }
    Ok(text)
}

#[cfg(not(any(target_os = "windows", target_os = "android")))]
pub fn consume_platform_turn(
    _state: &SpeechRuntimeState,
    _session_id: &str,
) -> Result<String, String> {
    Err("La captura de voz todavia no esta integrada en esta plataforma.".to_string())
}

#[cfg(not(any(target_os = "windows", target_os = "android")))]
pub fn start_platform_session(
    _app: &crate::host::AppHandle,
    _state: &SpeechRuntimeState,
    _session_id: String,
    _diarization_enabled: bool,
    _max_duration_seconds: u32,
    _capture_system_audio: bool,
) -> Result<(), String> {
    Err("La captura de voz todavia no esta integrada en esta plataforma.".to_string())
}

#[cfg(any(target_os = "windows", target_os = "android"))]
pub fn pause_platform_audio(state: &SpeechRuntimeState) -> Result<(), String> {
    let slot = state
        .active_session
        .lock()
        .map_err(|_| "No se pudo bloquear la captura de voz.".to_string())?;
    let session = slot
        .as_ref()
        .ok_or_else(|| "No hay una captura de voz activa.".to_string())?;
    session.audio.pause()?;
    session.worker.pause()
}

#[cfg(not(any(target_os = "windows", target_os = "android")))]
pub fn pause_platform_audio(_state: &SpeechRuntimeState) -> Result<(), String> {
    Err("La captura de voz todavia no esta integrada en esta plataforma.".to_string())
}

#[cfg(any(target_os = "windows", target_os = "android"))]
pub fn resume_platform_audio(state: &SpeechRuntimeState) -> Result<(), String> {
    let slot = state
        .active_session
        .lock()
        .map_err(|_| "No se pudo bloquear la captura de voz.".to_string())?;
    let session = slot
        .as_ref()
        .ok_or_else(|| "No hay una captura de voz activa.".to_string())?;
    session.audio.resume()?;
    session.worker.resume()
}

#[cfg(not(any(target_os = "windows", target_os = "android")))]
pub fn resume_platform_audio(_state: &SpeechRuntimeState) -> Result<(), String> {
    Err("La captura de voz todavia no esta integrada en esta plataforma.".to_string())
}

pub fn cancel_platform_audio(state: &SpeechRuntimeState) -> Result<(), String> {
    #[cfg(any(target_os = "windows", target_os = "android"))]
    {
        let session = state
            .active_session
            .lock()
            .map_err(|_| "No se pudo bloquear la captura de voz.".to_string())?
            .take();
        if let Some(session) = session {
            session.worker.cancel()?;
            session.worker.join()?;
        }
    }
    let mut phase = state
        .phase
        .lock()
        .map_err(|_| "No se pudo bloquear el estado de voz.".to_string())?;
    *phase = SpeechPhase::Idle;
    Ok(())
}

#[cfg(any(target_os = "windows", target_os = "android"))]
pub fn stop_platform_session(state: &SpeechRuntimeState) -> Result<(), String> {
    let session = state
        .active_session
        .lock()
        .map_err(|_| "No se pudo bloquear la sesion de voz.".to_string())?
        .take()
        .ok_or_else(|| "No hay una sesion de voz activa.".to_string())?;
    // The worker must finish even if the capture cannot pause: the phase is
    // already finalizing and only the worker's end returns it to idle.
    if let Err(error) = session.audio.pause() {
        log::warn!("[notia:speech] the capture did not pause before finishing: {error}");
    }
    // Nothing to measure while the speakers are separated.
    drop(session._levels);
    set_finalizing_session(state, Some(session.session_id.clone()));
    let finished = session.worker.stop().and_then(|()| session.worker.join());
    set_finalizing_session(state, None);
    if finished.is_err() {
        // A worker that panicked never reported its end.
        if let Ok(mut phase) = state.phase.lock() {
            *phase = SpeechPhase::Idle;
        }
    }
    finished
}

/// Releases `session_id` when its worker ended by itself (duration limit or
/// error) instead of through `stop` or `cancel`, which already took it. Until
/// then the microphone stayed open and no other session could start. Runs on
/// the worker thread, so the worker is detached instead of joined. Returns
/// whether the session was still held.
#[cfg(any(target_os = "windows", target_os = "android"))]
fn release_ended_session(app: &AppHandle, session_id: &str) -> bool {
    let state = app.state::<SpeechRuntimeState>();
    let session = state.active_session.lock().ok().and_then(|mut slot| {
        if slot.as_ref().is_some_and(|session| session.session_id == session_id) {
            slot.take()
        } else {
            None
        }
    });
    let Some(ActivePlatformSpeechSession { _levels, audio, worker, .. }) = session else {
        return false;
    };
    drop(_levels);
    drop(audio);
    worker.detach();
    #[cfg(target_os = "android")]
    crate::mobile_continuity::end_android_work(
        app.state::<crate::mobile_continuity::ContinuityState>().inner(),
    );
    true
}

#[cfg(any(target_os = "windows", target_os = "android"))]
fn set_finalizing_session(state: &SpeechRuntimeState, session_id: Option<String>) {
    if let Ok(mut finalizing) = state.finalizing_session.lock() {
        *finalizing = session_id;
    }
}

/// State of `session_id` while it records or separates its speakers, so an
/// interface that opens again can follow it. The clock is the recording
/// position without pauses.
#[cfg(any(target_os = "windows", target_os = "android"))]
pub fn session_state(state: &SpeechRuntimeState, session_id: &str) -> Result<SpeechSessionStateDto, String> {
    const ENDED: &str = "La grabación ya terminó.";
    let phase = *state
        .phase
        .lock()
        .map_err(|_| "No se pudo bloquear el estado de voz.".to_string())?;
    if phase == SpeechPhase::Finalizing {
        let finalizing = state
            .finalizing_session
            .lock()
            .map_err(|_| "No se pudo bloquear el estado de voz.".to_string())?;
        return match finalizing.as_deref() {
            Some(id) if id == session_id => Ok(SpeechSessionStateDto::Finalizing { progress: None, stage: None }),
            _ => Err(ENDED.to_string()),
        };
    }
    let elapsed_ms = session_position_ms(state, session_id).map_err(|_| ENDED.to_string())?;
    match phase {
        // The session exists but its worker has not reported that it is ready.
        SpeechPhase::Preparing => Ok(SpeechSessionStateDto::Preparing { progress: None }),
        SpeechPhase::Recording => Ok(SpeechSessionStateDto::Recording { elapsed_ms, has_speech: true }),
        SpeechPhase::Paused => Ok(SpeechSessionStateDto::Paused { elapsed_ms }),
        _ => Err(ENDED.to_string()),
    }
}

#[cfg(not(any(target_os = "windows", target_os = "android")))]
pub fn session_state(_state: &SpeechRuntimeState, _session_id: &str) -> Result<SpeechSessionStateDto, String> {
    Err(not_integrated_error())
}

#[cfg(not(any(target_os = "windows", target_os = "android")))]
pub fn stop_platform_session(_state: &SpeechRuntimeState) -> Result<(), String> {
    Err("La captura de voz todavia no esta integrada en esta plataforma.".to_string())
}

#[cfg(any(target_os = "windows", target_os = "android"))]
pub fn validate_active_session(
    state: &SpeechRuntimeState,
    session_id: &str,
) -> Result<u64, String> {
    let slot = state
        .active_session
        .lock()
        .map_err(|_| "No se pudo bloquear la sesion de voz.".to_string())?;
    let session = slot
        .as_ref()
        .filter(|session| session.session_id == session_id)
        .ok_or_else(|| "La sesion de voz no coincide con la sesion activa.".to_string())?;
    Ok(session.meter.position_ms())
}

#[cfg(not(any(target_os = "windows", target_os = "android")))]
pub fn validate_active_session(
    _state: &SpeechRuntimeState,
    _session_id: &str,
) -> Result<u64, String> {
    Err("No hay una sesion de voz activa.".to_string())
}

/// Whether `session_id` is gone: no session holds it and none is finishing
/// it, as when its worker ended without reporting it. A recording phase left
/// without any session returns to idle, so another recording can start.
#[cfg(any(target_os = "windows", target_os = "android"))]
pub fn release_lost_session(state: &SpeechRuntimeState, session_id: &str) -> bool {
    let Ok(slot) = state.active_session.lock() else { return false };
    if slot.as_ref().is_some_and(|session| session.session_id == session_id) {
        return false;
    }
    let finishing = state
        .finalizing_session
        .lock()
        .map(|finalizing| finalizing.as_deref() == Some(session_id))
        .unwrap_or(true);
    if finishing {
        return false;
    }
    let Ok(mut phase) = state.phase.lock() else { return false };
    log::error!(
        "[notia:speech] a session was asked to end but is gone (phase {:?}, another session active: {})",
        *phase,
        slot.is_some()
    );
    if slot.is_none() && matches!(*phase, SpeechPhase::Recording | SpeechPhase::Paused) {
        *phase = SpeechPhase::Idle;
    }
    true
}

#[cfg(not(any(target_os = "windows", target_os = "android")))]
pub fn release_lost_session(_state: &SpeechRuntimeState, _session_id: &str) -> bool {
    false
}

/// How a finished session separates its speakers.
#[cfg(any(target_os = "windows", target_os = "android"))]
struct DiarizationSetup<'a> {
    runtime_path: Option<&'a std::path::Path>,
    model: Option<&'a crate::services::speech_model_repository::ResolvedDiarizationModel>,
    expected_speakers: Option<u32>,
    /// The session's recognition model and language, for the final pass.
    asr: &'a WhisperAsrConfig,
}

/// Returned when the person skipped the speaker separation.
#[cfg(any(target_os = "windows", target_os = "android"))]
const DIARIZATION_SKIPPED: &str = "La separación de hablantes se omitió.";

#[cfg(any(target_os = "windows", target_os = "android"))]
/// Who a worker's events are for: a Meeting (its lines go through the
/// meeting) and whether its audio is a file.
#[cfg(any(target_os = "windows", target_os = "android"))]
#[derive(Clone, Copy)]
struct WorkerEventTarget {
    meeting: bool,
    file: bool,
}

#[cfg(any(target_os = "windows", target_os = "android"))]
fn handle_worker_event(
    app: &AppHandle,
    session_id: &str,
    started_at: Instant,
    confirmed: &Arc<StdMutex<ConfirmedSpeech>>,
    target: WorkerEventTarget,
    diarization: DiarizationSetup<'_>,
    event: crate::services::speech_worker::SpeechWorkerEvent,
) {
    use crate::services::speech_worker::SpeechWorkerEvent;
    let meeting = target.meeting;
    match event {
        // A file is not recorded: its session is processing from the start.
        SpeechWorkerEvent::Ready if target.file => {
            set_runtime_phase(app, SpeechPhase::Finalizing);
            set_finalizing_session(&app.state::<SpeechRuntimeState>(), Some(session_id.to_string()));
            emit_state(
                app,
                session_id,
                SpeechSessionStateDto::Finalizing { progress: Some(0.0), stage: Some("transcribing") },
            );
        }
        SpeechWorkerEvent::Ready => {
            set_runtime_phase(app, SpeechPhase::Recording);
            emit_state(
                app,
                session_id,
                SpeechSessionStateDto::Recording {
                    elapsed_ms: elapsed_ms(started_at),
                    has_speech: false,
                },
            );
        }
        SpeechWorkerEvent::Partial(update) => {
            let Ok(mut confirmed) = confirmed.lock() else {
                emit_error(
                    app,
                    session_id,
                    "internal",
                    "No se pudo actualizar la transcripcion.",
                );
                return;
            };
            let partial = if update.endpoint_detected {
                let appended = confirmed.confirm(&update.text, update.span);
                let span = update.span.map(|span| (span.start_ms(), span.end_ms()));
                crate::meeting::on_line(app, session_id, span, &appended);
                String::new()
            } else {
                unconfirmed_suffix(&confirmed.text, &update.text)
            };
            let _ = app.emit(
                "speech://partial",
                SpeechPartialEventDto {
                    session_id: session_id.to_string(),
                    confirmed_text: if meeting { String::new() } else { confirmed.text.clone() },
                    partial_text: partial,
                },
            );
        }
        SpeechWorkerEvent::Finished { update, audio } => {
            // Reaching the duration limit finishes the session like `stop`.
            if release_ended_session(app, session_id) {
                set_runtime_phase(app, SpeechPhase::Finalizing);
                set_finalizing_session(&app.state::<SpeechRuntimeState>(), Some(session_id.to_string()));
                emit_state(
                    app,
                    session_id,
                    SpeechSessionStateDto::Finalizing { progress: None, stage: Some("transcribing") },
                );
            }
            let span = update.span.map(|span| (span.start_ms(), span.end_ms()));
            let (text, lines) = match confirmed.lock() {
                Ok(mut confirmed) => {
                    let appended = confirmed.confirm(&update.text, update.span);
                    crate::meeting::on_line(app, session_id, span, &appended);
                    (confirmed.text.clone(), std::mem::take(&mut confirmed.lines))
                }
                Err(_) => (update.text, Vec::new()),
            };
            crate::meeting::on_processing(app, session_id, audio.duration_ms());
            let transcript = match diarization.model {
                Some(model) => match diarization
                    .runtime_path
                    .ok_or_else(|| "No se encontró el runtime de diarización.".to_string())
                    .and_then(|runtime_path| {
                        diarize_recorded_audio(
                            app,
                            session_id,
                            runtime_path,
                            model,
                            diarization.expected_speakers,
                            diarization.asr,
                            &audio,
                            &lines,
                        )
                    }) {
                    Ok(transcript) => transcript,
                    Err(message) if message == DIARIZATION_SKIPPED => {
                        log::info!("[notia:speech] speaker separation skipped by the person");
                        transcript_without_diarization(&text, audio.duration_ms())
                    }
                    Err(message) => {
                        log::warn!(
                            "[notia:speech] diarization failed; preserving ASR transcript: {message}"
                        );
                        transcript_without_diarization(&text, audio.duration_ms())
                    }
                },
                None => transcript_without_diarization(&text, audio.duration_ms()),
            };
            crate::meeting::on_completed(app, session_id, &transcript, audio.duration_ms());
            let _ = app.emit(
                "speech://segments",
                SpeechSegmentsEventDto {
                    session_id: session_id.to_string(),
                    transcript: transcript.clone(),
                },
            );
            emit_state(
                app,
                session_id,
                SpeechSessionStateDto::Completed { transcript },
            );
            set_finalizing_session(&app.state::<SpeechRuntimeState>(), None);
            set_runtime_phase(app, SpeechPhase::Idle);
        }
        SpeechWorkerEvent::Error(message) => {
            release_ended_session(app, session_id);
            crate::meeting::on_interrupted(app, session_id);
            emit_error(app, session_id, "internal", &message);
            set_runtime_phase(app, SpeechPhase::Idle);
        }
    }
}

#[cfg(any(target_os = "windows", target_os = "android"))]
fn emit_finalizing(app: &AppHandle, session_id: &str, stage: &'static str, progress: f32) {
    emit_state(
        app,
        session_id,
        SpeechSessionStateDto::Finalizing {
            progress: Some(progress.clamp(0.0, 1.0)),
            stage: Some(stage),
        },
    );
}

/// Separates the speakers of the recording window by window. On a GPU the
/// final pass transcribes each window again and gives every word its speaker;
/// elsewhere, or when that pass fails, each live line takes its speaker.
/// Reports the stage and progress of each window and stops when the person
/// skips it.
#[cfg(any(target_os = "windows", target_os = "android"))]
#[allow(clippy::too_many_arguments)]
fn diarize_recorded_audio(
    app: &AppHandle,
    session_id: &str,
    runtime_path: &std::path::Path,
    model: &crate::services::speech_model_repository::ResolvedDiarizationModel,
    expected_speakers: Option<u32>,
    asr: &WhisperAsrConfig,
    audio: &crate::services::speech_worker::RecordedAudio,
    lines: &[ConfirmedLine],
) -> Result<DiarizedTranscriptDto, String> {
    let spans = lines.iter().map(|line| line.span).collect::<Vec<_>>();
    let window_ends = diarization_window_ends(&spans, audio.sample_count());
    let window_count = window_ends.len().max(1) as f32;
    let embedding_extractor =
        match crate::services::sherpa_diarization::SpeakerEmbeddingExtractor::new(
            runtime_path,
            model,
        ) {
            Ok(extractor) => Some(extractor),
            Err(message) => {
                log::warn!(
                "[notia:speech] speaker embedding matching unavailable; keeping window-local speakers: {message}"
            );
                None
            }
        };
    let mut final_pass = load_final_pass(app, asr);
    let mut speaker_registry = GlobalSpeakerRegistry::new();
    let mut transcript = DiarizedTranscriptDto {
        text: String::new(),
        segments: Vec::new(),
        speaker_count: 0,
    };
    let mut window_index = 0_usize;
    audio.for_each_window(&window_ends, |window_start, samples| {
        let window = window_index as f32;
        if diarization_skipped(app, session_id) {
            return Err(DIARIZATION_SKIPPED.to_string());
        }
        emit_finalizing(app, session_id, "detecting-speakers", window / window_count);
        let diarization = crate::services::sherpa_diarization::process(
            runtime_path,
            model,
            samples,
            expected_speakers,
        )?;
        if diarization_skipped(app, session_id) {
            return Err(DIARIZATION_SKIPPED.to_string());
        }
        let mut stage = if final_pass.is_some() { "second-pass" } else { "assigning-turns" };
        emit_finalizing(app, session_id, stage, (window + 0.5) / window_count);
        let embeddings = match embedding_extractor.as_ref() {
            Some(extractor) => match extractor.extract(samples, &diarization) {
                Ok(embeddings) => embeddings,
                Err(message) => {
                    log::warn!(
                        "[notia:speech] speaker embeddings failed for diarization window; keeping unmatched speakers local: {message}"
                    );
                    Vec::new()
                }
            },
            None => Vec::new(),
        };
        let speaker_mapping = speaker_registry.remap_chunk(&diarization, &embeddings);
        // Each line belongs to the window that holds its middle.
        let window_end = (window_start + samples.len()) as u64;
        let last_window = window_start + samples.len() >= audio.sample_count();
        let window_lines = lines
            .iter()
            .filter(|line| {
                let middle = (line.span.start + line.span.end) / 2;
                middle >= window_start as u64 && (middle < window_end || last_window)
            })
            .collect::<Vec<_>>();
        let mut last_reported = -1.0_f32;
        let mut report = |stage: &'static str, done: usize, total: usize| {
            if diarization_skipped(app, session_id) {
                return Err(DIARIZATION_SKIPPED.to_string());
            }
            let progress = (window + 0.5 + 0.5 * done as f32 / total.max(1) as f32) / window_count;
            if progress - last_reported >= 0.01 {
                last_reported = progress;
                emit_finalizing(app, session_id, stage, progress);
            }
            Ok(())
        };
        let retranscribed = match final_pass.as_mut() {
            Some(pass) => match final_pass_chunk(
                pass,
                samples,
                window_start,
                &diarization,
                &window_lines,
                &mut |done, total| report("second-pass", done, total),
            ) {
                Ok(chunk) => Some(chunk),
                Err(message) if message == DIARIZATION_SKIPPED => return Err(message),
                Err(message) => {
                    log::warn!("[notia:speech] the final pass failed; the recording keeps its live text: {message}");
                    final_pass = None;
                    stage = "assigning-turns";
                    None
                }
            },
            None => None,
        };
        let chunk = match retranscribed {
            Some(chunk) => chunk,
            None => attribute_lines(app, samples, window_start, &diarization, &window_lines, &mut |done, total| {
                report(stage, done, total)
            })?,
        };
        append_diarized_chunk(
            &mut transcript,
            chunk,
            crate::services::speech_worker::samples_to_ms(window_start as u64),
            window_index,
            &speaker_mapping,
            speaker_registry.len() as u32,
        );
        window_index = window_index.saturating_add(1);
        Ok(())
    })?;
    if transcript.segments.is_empty() {
        return Err("La diarización no produjo segmentos de voz.".to_string());
    }
    Ok(transcript)
}

/// The final pass: Whisper large-v3 on a GPU. `None` on a CPU or without the
/// model, where the live lines keep their text: a second pass with the live
/// model (large-v3-turbo) wrote worse text than the live one on the probe
/// meetings (3.4 % of words wrong against 3.1 %; large-v3, 2.6 %).
#[cfg(any(target_os = "windows", target_os = "android"))]
fn load_final_pass(app: &AppHandle, asr: &WhisperAsrConfig) -> Option<crate::services::whisper_recognizer::WhisperFinalPass> {
    #[cfg(target_os = "windows")]
    {
        let model = crate::services::speech_model_repository::resolve_final_pass_model(app)?;
        let loaded = crate::services::whisper_runtime::runtime_path(app).and_then(|runtime| {
            crate::services::whisper_recognizer::WhisperFinalPass::load(&runtime, &model, "large-v3", &asr.language)
        });
        match loaded {
            Ok(pass) => pass,
            Err(message) => {
                log::warn!("[notia:speech] the final pass is unavailable; the recording keeps its live text: {message}");
                None
            }
        }
    }
    #[cfg(target_os = "android")]
    {
        let _ = (app, asr);
        None
    }
}

/// A window transcribed again by the final pass. Its live lines are decoded
/// again in blocks of up to 28 s with the time of each word, and each word
/// replaces the live text of the line that holds it: the lines keep the cuts
/// the voice detector made at each pause, which separated speakers better on
/// the probe conversations than the word times (98 % of the words with their
/// speaker against 95 %). A line keeps its speakers as `attribute_lines`
/// gives them; across a speaker change it splits at the first word said
/// after the change, without transcribing again. A line the pass heard
/// less than half of its live words in keeps its live text: Whisper may skip
/// a whole sentence of a block. `on_block` hears the progress before each
/// block and stops the pass with an error.
#[cfg(any(target_os = "windows", target_os = "android"))]
fn final_pass_chunk(
    pass: &mut crate::services::whisper_recognizer::WhisperFinalPass,
    samples: &[f32],
    window_start: usize,
    diarization: &crate::services::sherpa_diarization::DiarizationResult,
    lines: &[&ConfirmedLine],
    on_block: &mut dyn FnMut(usize, usize) -> Result<(), String>,
) -> Result<DiarizedTranscriptDto, String> {
    use crate::services::speech_worker::samples_to_ms;
    use notia_backend_core::timed_transcript::{words_by_span, TimedWord};

    let turns = speaker_turns(&diarization.segments);
    // Samples of each line inside this window.
    let local = |line: &ConfirmedLine| {
        let start = (line.span.start.saturating_sub(window_start as u64) as usize).min(samples.len());
        start..(line.span.end.saturating_sub(window_start as u64) as usize).clamp(start, samples.len())
    };
    let ranges = lines.iter().map(|line| local(line)).collect::<Vec<_>>();
    let blocks = crate::services::whisper_recognizer::final_pass_blocks(samples, &ranges);
    let mut words = Vec::<TimedWord>::new();
    for (index, block) in blocks.iter().enumerate() {
        on_block(index, blocks.len())?;
        words.extend(pass.transcribe(&samples[block.clone()], samples_to_ms(block.start as u64))?);
    }
    let spans = ranges.iter().map(|range| (samples_to_ms(range.start as u64), samples_to_ms(range.end as u64))).collect::<Vec<_>>();
    let mut segments = Vec::<SpeechTranscriptSegmentDto>::with_capacity(lines.len());
    let mut push = |start_ms: u64, end_ms: u64, speaker: Option<i32>, text: String| {
        if !text.is_empty() {
            segments.push(SpeechTranscriptSegmentDto {
                id: format!("segment-{}", segments.len() + 1),
                start_ms,
                end_ms,
                speaker_id: speaker.map(|speaker| format!("speaker-{}", speaker + 1)),
                text,
                is_final: true,
            });
        }
    };
    let joined = |words: &[TimedWord]| pass.clean(&words.iter().map(|word| word.text.as_str()).collect::<Vec<_>>().join(" "));
    for ((line, range), line_words) in lines.iter().zip(&ranges).zip(words_by_span(&words, &spans)) {
        let pieces = line_pieces(&turns, samples_to_seconds(range.start), samples_to_seconds(range.end));
        let (start_ms, end_ms) = (samples_to_ms(range.start as u64), samples_to_ms(range.end as u64));
        let text = joined(&line_words);
        // Whisper sometimes skips a whole sentence of a block: a line that
        // lost more than half of its live words keeps them.
        let skipped = text.split_whitespace().count() * 2 < line.text.split_whitespace().count();
        if pieces.len() == 1 || skipped {
            let speaker = pieces
                .iter()
                .max_by(|left, right| left.duration().total_cmp(&right.duration()))
                .and_then(|piece| piece.speaker);
            push(start_ms, end_ms, speaker, if skipped { line.text.clone() } else { text });
            continue;
        }
        for piece in &pieces {
            let (piece_start, piece_end) = piece.sample_range();
            let (piece_start, piece_end) = (samples_to_ms(piece_start as u64), samples_to_ms(piece_end as u64));
            let inside = line_words
                .iter()
                .filter(|word| (piece_start..piece_end).contains(&word.start_ms.clamp(start_ms, end_ms.saturating_sub(1))))
                .cloned()
                .collect::<Vec<_>>();
            push(piece_start, piece_end, piece.speaker, joined(&inside));
        }
    }
    let text = segments.iter().map(|segment| segment.text.as_str()).collect::<Vec<_>>().join(" ");
    Ok(DiarizedTranscriptDto {
        text,
        segments,
        speaker_count: diarization.speaker_count,
    })
}

/// Where each diarization window ends: about every 15 minutes, in the pause
/// between two confirmed lines closest before that mark (up to two minutes
/// earlier), so no word is split between two windows. Without a pause there
/// it cuts at the mark. A last window under a minute joins the previous one.
#[cfg(any(target_os = "windows", target_os = "android"))]
fn diarization_window_ends(
    spans: &[crate::services::speech_worker::SampleSpan],
    total_samples: usize,
) -> Vec<usize> {
    let mut sorted = spans.to_vec();
    sorted.sort_by_key(|span| span.start);
    let mut pauses = Vec::new();
    let mut spoken_until = 0_u64;
    for span in &sorted {
        if span.start >= spoken_until.saturating_add(WINDOW_MIN_PAUSE_SAMPLES) {
            pauses.push(spoken_until..span.start);
        }
        spoken_until = spoken_until.max(span.end);
    }
    pauses.push(spoken_until..u64::MAX);
    let mut ends = Vec::new();
    let mut start = 0_usize;
    while total_samples.saturating_sub(start) > DIARIZATION_WINDOW_SAMPLES + WINDOW_MIN_TAIL_SAMPLES {
        let target = (start + DIARIZATION_WINDOW_SAMPLES) as u64;
        let earliest = target - WINDOW_PAUSE_SEARCH_SAMPLES as u64;
        let cut = pauses
            .iter()
            .filter_map(|pause| {
                let low = pause.start.max(earliest);
                let high = pause.end.min(target);
                (low <= high).then(|| (pause.start / 2 + pause.end / 2).clamp(low, high))
            })
            .max()
            .unwrap_or(target);
        ends.push(cut as usize);
        start = cut as usize;
    }
    ends.push(total_samples);
    ends
}

#[cfg(any(target_os = "windows", target_os = "android"))]
struct GlobalSpeakerRegistry {
    profiles: Vec<GlobalSpeakerProfile>,
    observed_speakers: std::collections::BTreeSet<String>,
    next_speaker_index: usize,
}

#[cfg(any(target_os = "windows", target_os = "android"))]
struct GlobalSpeakerProfile {
    id: String,
    centroid: Vec<f32>,
    observations: usize,
}

#[cfg(any(target_os = "windows", target_os = "android"))]
impl GlobalSpeakerRegistry {
    fn new() -> Self {
        Self {
            profiles: Vec::new(),
            observed_speakers: std::collections::BTreeSet::new(),
            next_speaker_index: 0,
        }
    }

    fn len(&self) -> usize {
        self.observed_speakers.len()
    }

    fn remap_chunk(
        &mut self,
        diarization: &crate::services::sherpa_diarization::DiarizationResult,
        embeddings: &[crate::services::sherpa_diarization::SpeakerEmbedding],
    ) -> std::collections::BTreeMap<String, String> {
        let local_speakers = diarization
            .segments
            .iter()
            .map(|segment| format!("speaker-{}", segment.speaker + 1))
            .collect::<std::collections::BTreeSet<_>>();
        let embedding_by_speaker = embeddings
            .iter()
            .map(|embedding| {
                (
                    format!("speaker-{}", embedding.speaker + 1),
                    embedding.vector.as_slice(),
                )
            })
            .collect::<std::collections::BTreeMap<_, _>>();
        let mut candidates = Vec::new();
        for (local_speaker, embedding) in &embedding_by_speaker {
            for (profile_index, profile) in self.profiles.iter().enumerate() {
                if let Some(score) = cosine_similarity(embedding, &profile.centroid) {
                    candidates.push((score, local_speaker.as_str(), profile_index));
                }
            }
        }
        candidates.sort_by(|left, right| {
            right
                .0
                .partial_cmp(&left.0)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let mut mapping = std::collections::BTreeMap::new();
        let mut claimed_profiles = std::collections::BTreeSet::new();
        for (score, local_speaker, profile_index) in candidates {
            if score < GLOBAL_SPEAKER_MATCH_THRESHOLD
                || mapping.contains_key(local_speaker)
                || !claimed_profiles.insert(profile_index)
            {
                continue;
            }
            let global_speaker = self.profiles[profile_index].id.clone();
            mapping.insert(local_speaker.to_string(), global_speaker);
            self.observed_speakers
                .insert(self.profiles[profile_index].id.clone());
            self.update_profile(profile_index, embedding_by_speaker[local_speaker]);
        }

        for local_speaker in local_speakers {
            if mapping.contains_key(&local_speaker) {
                continue;
            }
            let global_speaker = self.allocate_speaker_id();
            if let Some(embedding) = embedding_by_speaker.get(&local_speaker) {
                self.profiles.push(GlobalSpeakerProfile {
                    id: global_speaker.clone(),
                    centroid: normalize_embedding(embedding),
                    observations: 1,
                });
            } else {
                // A very short turn may not yield an embedding. Keep a unique
                // ID for this occurrence without poisoning the match registry.
            }
            mapping.insert(local_speaker, global_speaker);
        }
        mapping
    }

    fn allocate_speaker_id(&mut self) -> String {
        let id = global_speaker_id(self.next_speaker_index);
        self.next_speaker_index = self.next_speaker_index.saturating_add(1);
        self.observed_speakers.insert(id.clone());
        id
    }

    fn update_profile(&mut self, profile_index: usize, embedding: &[f32]) {
        let profile = &mut self.profiles[profile_index];
        if profile.centroid.len() != embedding.len() {
            return;
        }
        let previous_weight = profile.observations as f32;
        for (centroid, value) in profile.centroid.iter_mut().zip(embedding) {
            *centroid = (*centroid * previous_weight + *value) / (previous_weight + 1.0);
        }
        profile.centroid = normalize_embedding(&profile.centroid);
        profile.observations = profile.observations.saturating_add(1);
    }
}

#[cfg(any(target_os = "windows", target_os = "android"))]
fn global_speaker_id(profile_index: usize) -> String {
    format!("speaker-{}", profile_index + 1)
}

#[cfg(any(target_os = "windows", target_os = "android"))]
fn normalize_embedding(embedding: &[f32]) -> Vec<f32> {
    let norm = embedding
        .iter()
        .map(|value| value * value)
        .sum::<f32>()
        .sqrt();
    if norm <= f32::EPSILON || !norm.is_finite() {
        return embedding.to_vec();
    }
    embedding.iter().map(|value| value / norm).collect()
}

#[cfg(any(target_os = "windows", target_os = "android"))]
fn cosine_similarity(left: &[f32], right: &[f32]) -> Option<f32> {
    if left.len() != right.len() || left.is_empty() {
        return None;
    }
    let left_norm = left.iter().map(|value| value * value).sum::<f32>().sqrt();
    let right_norm = right.iter().map(|value| value * value).sum::<f32>().sqrt();
    if !left_norm.is_finite()
        || !right_norm.is_finite()
        || left_norm <= f32::EPSILON
        || right_norm <= f32::EPSILON
    {
        return None;
    }
    Some(
        left.iter()
            .zip(right)
            .map(|(left, right)| left * right)
            .sum::<f32>()
            / (left_norm * right_norm),
    )
}

#[cfg(any(target_os = "windows", target_os = "android"))]
fn append_diarized_chunk(
    target: &mut DiarizedTranscriptDto,
    mut chunk: DiarizedTranscriptDto,
    chunk_start_ms: u64,
    chunk_index: usize,
    speaker_mapping: &std::collections::BTreeMap<String, String>,
    speaker_count: u32,
) {
    if !target.text.is_empty() && !chunk.text.is_empty() {
        target.text.push(' ');
    }
    target.text.push_str(&chunk.text);
    target.speaker_count = speaker_count;
    for segment in &mut chunk.segments {
        segment.id = format!("segment-{}", target.segments.len() + 1);
        segment.start_ms = segment.start_ms.saturating_add(chunk_start_ms);
        segment.end_ms = segment.end_ms.saturating_add(chunk_start_ms);
        if let Some(speaker_id) = &segment.speaker_id {
            segment.speaker_id = Some(
                speaker_mapping
                    .get(speaker_id)
                    .cloned()
                    .unwrap_or_else(|| format!("chunk-{chunk_index}-{speaker_id}")),
            );
        }
    }
    target.segments.extend(chunk.segments);
}

#[cfg(any(target_os = "windows", target_os = "android"))]
fn elapsed_ms(started_at: Instant) -> u64 {
    started_at.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
}

/// Appends a confirmed utterance and returns it. Utterances never share audio,
/// so words that repeat the end of `target` ("Sí." answered after "Sí.") were
/// spoken again and are kept.
fn append_text(target: &mut String, text: &str) -> String {
    let words = text.split_whitespace().collect::<Vec<_>>();
    let Some(first_word) = words.first() else {
        return String::new();
    };
    normalize_transcript_chunk_boundary(target, first_word);
    if !target.is_empty() {
        target.push(' ');
    }
    let appended = words.join(" ");
    target.push_str(&appended);
    appended
}

fn normalize_transcript_chunk_boundary(target: &mut String, next_word: &str) {
    let starts_lowercase = next_word.chars().next().is_some_and(char::is_lowercase);
    if !starts_lowercase || !target.ends_with('.') {
        return;
    }
    target.pop();
    if matches!(
        next_word.to_lowercase().as_str(),
        "aunque" | "pero" | "porque" | "pues" | "sino"
    ) {
        target.push(',');
    }
}

fn unconfirmed_suffix(confirmed: &str, partial: &str) -> String {
    let confirmed_words = confirmed.split_whitespace().collect::<Vec<_>>();
    let partial_words = partial.split_whitespace().collect::<Vec<_>>();
    let overlap = matching_boundary_words(&confirmed_words, &partial_words);
    partial_words[overlap..].join(" ")
}

fn matching_boundary_words(left_words: &[&str], right_words: &[&str]) -> usize {
    let exact_overlap = (1..=left_words.len().min(right_words.len()))
        .rev()
        .find(|&count| {
            left_words[left_words.len() - count..]
                .iter()
                .zip(&right_words[..count])
                .all(|(left, right)| words_match(left, right))
        })
        .unwrap_or(0);
    if exact_overlap > 0 {
        return exact_overlap;
    }

    // A preview starts a little before the utterance VAD detected, so it can
    // repeat the end of the confirmed text, rendered slightly differently
    // (for example, `Espartinas` / `las partinas`). Reconcile a sufficiently
    // long fuzzy boundary so that one changed word does not show the overlap.
    const MAX_BOUNDARY_WORDS: usize = 12;
    const MIN_FUZZY_BOUNDARY_WORDS: usize = 4;
    const MAX_BOUNDARY_EDITS: usize = 2;
    let max_left = left_words.len().min(MAX_BOUNDARY_WORDS);
    let max_right = right_words.len().min(MAX_BOUNDARY_WORDS);
    let mut best: Option<(usize, usize, usize)> = None;

    for left_count in MIN_FUZZY_BOUNDARY_WORDS..=max_left {
        for right_count in MIN_FUZZY_BOUNDARY_WORDS..=max_right {
            let span = left_count.max(right_count);
            let allowed_edits = (span / 3).min(MAX_BOUNDARY_EDITS);
            if left_count.abs_diff(right_count) > allowed_edits {
                continue;
            }
            let distance = boundary_edit_distance(
                &left_words[left_words.len() - left_count..],
                &right_words[..right_count],
            );
            if distance == 0 || distance > allowed_edits {
                continue;
            }
            let candidate = (span, usize::MAX - distance, right_count);
            if best.is_none_or(|current| candidate > current) {
                best = Some(candidate);
            }
        }
    }

    best.map_or(0, |(_, _, right_count)| right_count)
}

fn boundary_edit_distance(left: &[&str], right: &[&str]) -> usize {
    let mut previous = (0..=right.len()).collect::<Vec<_>>();
    let mut current = vec![0; right.len() + 1];
    for (left_index, left_word) in left.iter().enumerate() {
        current[0] = left_index + 1;
        for (right_index, right_word) in right.iter().enumerate() {
            let substitution =
                previous[right_index] + usize::from(!words_match(left_word, right_word));
            current[right_index + 1] = substitution
                .min(previous[right_index + 1] + 1)
                .min(current[right_index] + 1);
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[right.len()]
}

fn words_match(left: &str, right: &str) -> bool {
    left.trim_matches(|character: char| !character.is_alphanumeric())
        .eq_ignore_ascii_case(right.trim_matches(|character: char| !character.is_alphanumeric()))
}

#[cfg(any(target_os = "windows", target_os = "android"))]
fn transcript_without_diarization(text: &str, elapsed_ms: u64) -> DiarizedTranscriptDto {
    let text = text.trim().to_string();
    let segments = (!text.is_empty())
        .then(|| SpeechTranscriptSegmentDto {
            id: "segment-1".to_string(),
            start_ms: 0,
            end_ms: elapsed_ms,
            speaker_id: None,
            text: text.clone(),
            is_final: true,
        })
        .into_iter()
        .collect();
    DiarizedTranscriptDto {
        text,
        segments,
        speaker_count: 0,
    }
}

/// Gives each live line of a window its speaker. A line inside one speaker's
/// turn keeps the text recognized while recording; only a line spoken across
/// a speaker change is transcribed again, piece by piece. Before, every turn
/// of the recording was transcribed again. `on_line` hears the progress
/// before each line and stops the pass with an error.
#[cfg(any(target_os = "windows", target_os = "android"))]
fn attribute_lines(
    app: &AppHandle,
    samples: &[f32],
    window_start: usize,
    diarization: &crate::services::sherpa_diarization::DiarizationResult,
    lines: &[&ConfirmedLine],
    on_line: &mut dyn FnMut(usize, usize) -> Result<(), String>,
) -> Result<DiarizedTranscriptDto, String> {
    use crate::services::speech_worker::samples_to_ms;

    let turns = speaker_turns(&diarization.segments);
    let mut recognizer = BorrowedRecognizer::new(app);
    let mut segments = Vec::<SpeechTranscriptSegmentDto>::with_capacity(lines.len());
    let mut push = |start: u64, end: u64, speaker: Option<i32>, text: String| {
        segments.push(SpeechTranscriptSegmentDto {
            id: format!("segment-{}", segments.len() + 1),
            start_ms: samples_to_ms(start),
            end_ms: samples_to_ms(end),
            speaker_id: speaker.map(|speaker| format!("speaker-{}", speaker + 1)),
            text,
            is_final: true,
        });
    };
    for (index, line) in lines.iter().enumerate() {
        on_line(index, lines.len())?;
        // Samples of the line inside this window.
        let start = (line.span.start.saturating_sub(window_start as u64) as usize).min(samples.len());
        let end = (line.span.end.saturating_sub(window_start as u64) as usize).clamp(start, samples.len());
        let pieces = line_pieces(&turns, samples_to_seconds(start), samples_to_seconds(end));
        let retranscribed = if pieces.len() > 1 { recognizer.transcribe(samples, &pieces) } else { None };
        match retranscribed {
            Some(parts) => {
                for (piece, text) in parts {
                    let (piece_start, piece_end) = piece.sample_range();
                    push(piece_start as u64, piece_end as u64, piece.speaker, text);
                }
            }
            None => {
                let speaker = pieces
                    .iter()
                    .max_by(|left, right| left.duration().total_cmp(&right.duration()))
                    .and_then(|piece| piece.speaker);
                push(start as u64, end as u64, speaker, line.text.clone());
            }
        }
    }
    let text = segments.iter().map(|segment| segment.text.as_str()).collect::<Vec<_>>().join(" ");
    Ok(DiarizedTranscriptDto {
        text,
        segments,
        speaker_count: diarization.speaker_count,
    })
}

#[cfg(any(target_os = "windows", target_os = "android"))]
fn samples_to_seconds(samples: usize) -> f32 {
    samples as f32 / crate::services::speech_audio::SPEECH_SAMPLE_RATE as f32
}

/// Part of a line spoken by one speaker (`None` when the window found none).
#[cfg(any(target_os = "windows", target_os = "android"))]
#[derive(Debug, Clone, Copy, PartialEq)]
struct LinePiece {
    start_seconds: f32,
    end_seconds: f32,
    speaker: Option<i32>,
}

#[cfg(any(target_os = "windows", target_os = "android"))]
impl LinePiece {
    fn duration(&self) -> f32 {
        self.end_seconds - self.start_seconds
    }

    fn sample_range(&self) -> (usize, usize) {
        let rate = crate::services::speech_audio::SPEECH_SAMPLE_RATE as f32;
        (
            (self.start_seconds * rate).round() as usize,
            (self.end_seconds * rate).round() as usize,
        )
    }
}

/// The speakers of the line `start..end` (seconds of the window). Only a
/// speaker heard for at least half a second counts: a line with one such
/// speaker stays whole with the speaker heard the most, and a line with
/// several splits halfway through the gap between their turns. A line no turn
/// touches takes the speaker of the nearest turn.
#[cfg(any(target_os = "windows", target_os = "android"))]
fn line_pieces(
    turns: &[crate::services::sherpa_diarization::DiarizationSegment],
    start: f32,
    end: f32,
) -> Vec<LinePiece> {
    const MIN_SPEAKER_SECONDS: f32 = 0.5;
    struct Run {
        speaker: i32,
        start: f32,
        end: f32,
        heard: f32,
    }
    let whole = |speaker| vec![LinePiece { start_seconds: start, end_seconds: end, speaker }];
    let mut runs = Vec::<Run>::new();
    for turn in turns.iter().filter(|turn| turn.end_seconds > start && turn.start_seconds < end) {
        let heard = turn.end_seconds.min(end) - turn.start_seconds.max(start);
        match runs.last_mut() {
            Some(run) if run.speaker == turn.speaker => {
                run.end = turn.end_seconds;
                run.heard += heard;
            }
            _ => runs.push(Run { speaker: turn.speaker, start: turn.start_seconds, end: turn.end_seconds, heard }),
        }
    }
    if runs.is_empty() {
        let nearest = turns
            .iter()
            .min_by(|left, right| {
                let distance = |turn: &crate::services::sherpa_diarization::DiarizationSegment| {
                    (turn.start_seconds - end).max(start - turn.end_seconds)
                };
                distance(left).total_cmp(&distance(right))
            })
            .map(|turn| turn.speaker);
        return whole(nearest);
    }
    let mut kept = Vec::<Run>::new();
    for run in runs.iter().filter(|run| run.heard >= MIN_SPEAKER_SECONDS) {
        match kept.last_mut() {
            Some(last) if last.speaker == run.speaker => {
                last.end = run.end;
                last.heard += run.heard;
            }
            _ => kept.push(Run { speaker: run.speaker, start: run.start, end: run.end, heard: run.heard }),
        }
    }
    if kept.len() <= 1 {
        let mut heard = std::collections::BTreeMap::<i32, f32>::new();
        for run in &runs {
            *heard.entry(run.speaker).or_default() += run.heard;
        }
        let speaker = heard
            .into_iter()
            .max_by(|left, right| left.1.total_cmp(&right.1))
            .map(|(speaker, _)| speaker);
        return whole(speaker);
    }
    let boundaries = kept
        .windows(2)
        .map(|pair| ((pair[0].end + pair[1].start) / 2.0).clamp(start, end))
        .collect::<Vec<_>>();
    kept.iter()
        .enumerate()
        .map(|(index, run)| LinePiece {
            start_seconds: if index == 0 { start } else { boundaries[index - 1] },
            end_seconds: boundaries.get(index).copied().unwrap_or(end),
            speaker: Some(run.speaker),
        })
        .collect()
}

/// The resident recognizer, taken from its cache the first time a line must
/// be transcribed again and returned to it when dropped. Without it the line
/// keeps its live text.
#[cfg(any(target_os = "windows", target_os = "android"))]
struct BorrowedRecognizer {
    cache: PreloadedRecognizer,
    recognizer: Option<WhisperVadRecognizer>,
    unavailable: bool,
}

#[cfg(any(target_os = "windows", target_os = "android"))]
impl BorrowedRecognizer {
    fn new(app: &AppHandle) -> Self {
        Self {
            cache: recognizer_cache(&app.state::<SpeechRuntimeState>()),
            recognizer: None,
            unavailable: false,
        }
    }

    /// Text of each piece that has any; `None` when nothing could be
    /// transcribed and the line should keep its live text.
    fn transcribe(&mut self, samples: &[f32], pieces: &[LinePiece]) -> Option<Vec<(LinePiece, String)>> {
        if self.recognizer.is_none() && !self.unavailable {
            self.recognizer = self.cache.lock().ok().and_then(|mut slot| slot.take());
            if self.recognizer.is_none() {
                self.unavailable = true;
                log::warn!("[notia:speech] recognizer unavailable; lines across a speaker change keep their live text");
            }
        }
        let recognizer = self.recognizer.as_mut()?;
        let mut parts = Vec::with_capacity(pieces.len());
        for piece in pieces {
            let (start, end) = piece.sample_range();
            let audio = &samples[start.min(samples.len())..end.min(samples.len())];
            match transcribe_piece(recognizer, audio) {
                Ok(text) if !text.is_empty() => parts.push((*piece, text)),
                Ok(_) => {}
                Err(message) => {
                    log::warn!("[notia:speech] a line across a speaker change keeps its live text: {message}");
                    return None;
                }
            }
        }
        (!parts.is_empty()).then_some(parts)
    }
}

#[cfg(any(target_os = "windows", target_os = "android"))]
impl Drop for BorrowedRecognizer {
    fn drop(&mut self) {
        use crate::services::speech_worker::StreamingRecognizer;

        if let Some(mut recognizer) = self.recognizer.take() {
            if recognizer.reset_session().is_ok() {
                if let Ok(mut slot) = self.cache.lock() {
                    *slot = Some(recognizer);
                }
            }
        }
    }
}

/// Recognizes one piece of audio on its own.
#[cfg(any(target_os = "windows", target_os = "android"))]
fn transcribe_piece(recognizer: &mut WhisperVadRecognizer, samples: &[f32]) -> Result<String, String> {
    use crate::services::speech_worker::StreamingRecognizer;

    const ASR_CHUNK_SAMPLES: usize = 3_200;
    // Silero VAD needs some audio to close a segment; short pieces are padded with silence.
    const MIN_PIECE_SAMPLES: usize = 12_800;
    let mut padded = samples.to_vec();
    if padded.len() < MIN_PIECE_SAMPLES {
        padded.resize(MIN_PIECE_SAMPLES, 0.0);
    }
    recognizer.reset_session()?;
    let mut text = String::new();
    for chunk in padded.chunks(ASR_CHUNK_SAMPLES) {
        let update = recognizer.accept_waveform(chunk)?;
        if update.endpoint_detected {
            append_text(&mut text, &update.text);
            recognizer.reset_after_endpoint()?;
        }
    }
    let final_update = recognizer.finish()?;
    append_text(&mut text, &final_update.text);
    Ok(text.trim().to_string())
}

/// Speaker turns that never share audio. Diarization gives overlapping speech
/// to both speakers, and transcribing it twice repeated the words of whoever
/// kept talking inside the other turn (a "sí" said over someone). The overlap
/// stays with the turn that started first; a leftover too short to hold a
/// word is dropped, and segments of one speaker less than 0.5 s apart join.
#[cfg(any(target_os = "windows", target_os = "android"))]
fn speaker_turns(
    source: &[crate::services::sherpa_diarization::DiarizationSegment],
) -> Vec<crate::services::sherpa_diarization::DiarizationSegment> {
    const MAX_JOIN_GAP_SECONDS: f32 = 0.5;
    const MIN_TURN_SECONDS: f32 = 0.25;
    let mut turns: Vec<crate::services::sherpa_diarization::DiarizationSegment> = Vec::new();
    let mut covered_until = 0.0_f32;
    for segment in source {
        let start = segment.start_seconds.max(covered_until);
        let end = segment.end_seconds;
        covered_until = covered_until.max(end);
        if end - start < MIN_TURN_SECONDS {
            continue;
        }
        match turns.last_mut() {
            Some(previous)
                if previous.speaker == segment.speaker
                    && start <= previous.end_seconds + MAX_JOIN_GAP_SECONDS =>
            {
                previous.end_seconds = end;
            }
            _ => turns.push(crate::services::sherpa_diarization::DiarizationSegment {
                start_seconds: start,
                end_seconds: end,
                speaker: segment.speaker,
            }),
        }
    }
    turns
}

#[cfg(test)]
fn nearest_sentence_boundary(words: &[&str], minimum: usize, ideal: usize) -> usize {
    const MAX_BOUNDARY_SHIFT_WORDS: usize = 12;
    let lower = ideal
        .saturating_sub(MAX_BOUNDARY_SHIFT_WORDS)
        .max(minimum.saturating_add(1));
    let upper = ideal
        .saturating_add(MAX_BOUNDARY_SHIFT_WORDS)
        .min(words.len().saturating_sub(1));
    (lower..=upper)
        .filter(|&boundary| {
            words[boundary - 1]
                .trim_end_matches(|character: char| matches!(character, '\"' | '\'' | ')' | ']'))
                .ends_with(['.', '?', '!'])
        })
        .min_by_key(|&boundary| boundary.abs_diff(ideal))
        .unwrap_or_else(|| ideal.clamp(minimum.saturating_add(1), words.len()))
}

#[cfg(any(target_os = "windows", target_os = "android"))]
fn emit_state(app: &AppHandle, session_id: &str, state: SpeechSessionStateDto) {
    let _ = app.emit(
        "speech://state",
        SpeechSessionEventDto {
            session_id: session_id.to_string(),
            state,
        },
    );
}

#[cfg(any(target_os = "windows", target_os = "android"))]
pub fn emit_session_state(app: &AppHandle, session_id: &str, state: SpeechSessionStateDto) {
    emit_state(app, session_id, state);
}

#[cfg(any(target_os = "windows", target_os = "android"))]
fn emit_error(app: &AppHandle, session_id: &str, code: &str, message: &str) {
    emit_state(
        app,
        session_id,
        SpeechSessionStateDto::Error {
            error: SpeechErrorDto {
                code: code.to_string(),
                message: message.to_string(),
            },
        },
    );
}

#[cfg(any(target_os = "windows", target_os = "android"))]
fn set_runtime_phase(app: &AppHandle, phase: SpeechPhase) {
    if let Ok(mut current) = app.state::<SpeechRuntimeState>().phase.lock() {
        *current = phase;
    }
}

impl SpeechPhase {
    pub fn is_active(self) -> bool {
        self == Self::Preparing || ACTIVE_SPEECH_PHASES.contains(&self)
    }

    pub fn can_transition_to(self, next: Self) -> bool {
        matches!(
            (self, next),
            (Self::Idle, Self::Preparing)
                | (Self::Preparing, Self::Recording)
                | (Self::Preparing, Self::Idle)
                | (Self::Recording, Self::Paused)
                | (Self::Recording, Self::Finalizing)
                | (Self::Recording, Self::Idle)
                | (Self::Paused, Self::Recording)
                | (Self::Paused, Self::Finalizing)
                | (Self::Paused, Self::Idle)
                | (Self::Finalizing, Self::Idle)
        )
    }
}

pub fn current_capabilities(
    model_status: &SpeechModelStatusDto,
    _diarization_runtime_compatible: bool,
    _audio_available: bool,
    permission: &str,
) -> SpeechCapabilitiesDto {
    let platform = if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "android") {
        "android"
    } else {
        "other"
    };

    let platform_supported = cfg!(any(target_os = "windows", target_os = "android"));
    let asr_ready = model_status
        .profiles
        .iter()
        .any(|profile| profile.asr_ready);
    let diarization_ready = model_status
        .profiles
        .iter()
        .any(|profile| profile.diarization_ready);
    let supported = platform_supported;
    SpeechCapabilitiesDto {
        supported,
        platform: platform.to_string(),
        architecture: std::env::consts::ARCH.to_string(),
        permission: permission.to_string(),
        asr_model_installed: asr_ready,
        diarization_model_installed: diarization_ready,
        system_audio_supported: cfg!(target_os = "windows"),
        unavailable_reason: (!supported).then(|| {
            if platform_supported {
                "not-integrated"
            } else {
                "unsupported-platform"
            }
            .to_string()
        }),
    }
}

pub fn validate_start_input(language: &str, max_duration_seconds: u32) -> Result<(), String> {
    if language.trim().is_empty() || language.len() > 16 {
        return Err("El idioma solicitado para el dictado no es valido.".to_string());
    }
    if !(1..=MAX_SPEECH_SESSION_SECONDS).contains(&max_duration_seconds) {
        return Err(format!(
            "La duracion del dictado debe estar entre 1 y {MAX_SPEECH_SESSION_SECONDS} segundos."
        ));
    }
    Ok(())
}

pub fn not_integrated_error() -> String {
    "El reconocimiento de voz nativo todavía no está disponible en esta plataforma.".to_string()
}

#[cfg(test)]
mod tests {
    #[cfg(any(target_os = "windows", target_os = "android"))]
    use super::{
        diarization_window_ends, line_pieces, speaker_turns, ConfirmedSpeech, GlobalSpeakerRegistry,
        LinePiece, DIARIZATION_WINDOW_SAMPLES,
    };
    #[cfg(any(target_os = "windows", target_os = "android"))]
    use crate::services::speech_worker::SampleSpan;
    use super::{
        append_text, commit_external_update, current_capabilities, nearest_sentence_boundary,
        unconfirmed_suffix, validate_start_input, SpeechPhase, MAX_SPEECH_SESSION_SECONDS,
    };
    use crate::dto::speech::SpeechModelStatusDto;
    #[cfg(any(target_os = "windows", target_os = "android"))]
    use crate::services::sherpa_diarization::{
        DiarizationResult, DiarizationSegment, SpeakerEmbedding,
    };
    use crate::services::speech_worker::RecognitionUpdate;

    #[cfg(any(target_os = "windows", target_os = "android"))]
    #[test]
    fn only_a_session_nobody_holds_or_finishes_is_lost() {
        use super::{release_lost_session, set_finalizing_session, SpeechRuntimeState};
        let state = SpeechRuntimeState::default();
        *state.phase.lock().expect("phase") = SpeechPhase::Finalizing;
        set_finalizing_session(&state, Some("a".to_string()));
        assert!(!release_lost_session(&state, "a"));
        // Another session finishing keeps its phase.
        assert!(release_lost_session(&state, "b"));
        assert_eq!(*state.phase.lock().expect("phase"), SpeechPhase::Finalizing);

        set_finalizing_session(&state, None);
        *state.phase.lock().expect("phase") = SpeechPhase::Paused;
        assert!(release_lost_session(&state, "a"));
        assert_eq!(*state.phase.lock().expect("phase"), SpeechPhase::Idle);
    }

    #[test]
    fn speech_phase_rejects_invalid_transitions() {
        assert!(!SpeechPhase::Idle.is_active());
        assert!(SpeechPhase::Recording.is_active());
        assert!(SpeechPhase::Idle.can_transition_to(SpeechPhase::Preparing));
        assert!(!SpeechPhase::Idle.can_transition_to(SpeechPhase::Recording));
        assert!(SpeechPhase::Paused.can_transition_to(SpeechPhase::Finalizing));
        assert!(!SpeechPhase::Finalizing.can_transition_to(SpeechPhase::Paused));
    }

    #[test]
    fn start_input_is_bounded() {
        assert!(validate_start_input("es", MAX_SPEECH_SESSION_SECONDS).is_ok());
        assert!(validate_start_input("", 10).is_err());
        assert!(validate_start_input("es", 0).is_err());
        assert!(validate_start_input("es", MAX_SPEECH_SESSION_SECONDS + 1).is_err());
    }

    #[cfg(any(target_os = "windows", target_os = "android"))]
    #[test]
    fn global_speaker_registry_matches_local_ids_across_windows() {
        let mut registry = GlobalSpeakerRegistry::new();
        let first_window = DiarizationResult {
            speaker_count: 2,
            segments: vec![
                DiarizationSegment {
                    start_seconds: 0.0,
                    end_seconds: 3.0,
                    speaker: 0,
                },
                DiarizationSegment {
                    start_seconds: 3.0,
                    end_seconds: 6.0,
                    speaker: 1,
                },
            ],
        };
        let first_mapping = registry.remap_chunk(
            &first_window,
            &[
                SpeakerEmbedding {
                    speaker: 0,
                    vector: vec![1.0, 0.0],
                },
                SpeakerEmbedding {
                    speaker: 1,
                    vector: vec![0.0, 1.0],
                },
            ],
        );
        let second_window = DiarizationResult {
            speaker_count: 2,
            segments: vec![
                DiarizationSegment {
                    start_seconds: 0.0,
                    end_seconds: 3.0,
                    speaker: 8,
                },
                DiarizationSegment {
                    start_seconds: 3.0,
                    end_seconds: 6.0,
                    speaker: 4,
                },
            ],
        };
        let second_mapping = registry.remap_chunk(
            &second_window,
            &[
                SpeakerEmbedding {
                    speaker: 8,
                    vector: vec![0.02, 0.99],
                },
                SpeakerEmbedding {
                    speaker: 4,
                    vector: vec![0.98, 0.04],
                },
            ],
        );

        assert_eq!(registry.len(), 2);
        assert_eq!(first_mapping["speaker-1"], second_mapping["speaker-5"]);
        assert_eq!(first_mapping["speaker-2"], second_mapping["speaker-9"]);
        assert_ne!(second_mapping["speaker-5"], second_mapping["speaker-9"]);
    }

    #[cfg(any(target_os = "windows", target_os = "android"))]
    #[test]
    fn a_line_inside_one_turn_keeps_its_speaker_and_a_line_across_a_change_splits() {
        let turn = |start_seconds, end_seconds, speaker| DiarizationSegment { start_seconds, end_seconds, speaker };
        let piece = |start_seconds, end_seconds, speaker| LinePiece { start_seconds, end_seconds, speaker };
        let turns = [turn(0.0, 10.0, 0), turn(10.4, 11.0, 1), turn(11.2, 20.0, 0), turn(21.0, 30.0, 1)];
        // Inside one turn: reused whole.
        assert_eq!(line_pieces(&turns, 1.0, 9.0), vec![piece(1.0, 9.0, Some(0))]);
        // Speaker 1 heard for 0.4 s inside the line does not split it.
        assert_eq!(line_pieces(&turns, 9.0, 10.8), vec![piece(9.0, 10.8, Some(0))]);
        // Across a real change: split halfway through the gap between turns.
        assert_eq!(
            line_pieces(&turns, 15.0, 25.0),
            vec![piece(15.0, 20.5, Some(0)), piece(20.5, 25.0, Some(1))]
        );
        // Outside every turn: the nearest speaker.
        assert_eq!(line_pieces(&turns, 30.5, 31.0), vec![piece(30.5, 31.0, Some(1))]);
        assert_eq!(line_pieces(&[], 1.0, 2.0), vec![piece(1.0, 2.0, None)]);
    }

    #[cfg(any(target_os = "windows", target_os = "android"))]
    #[test]
    fn diarization_windows_end_in_the_pause_closest_before_15_minutes() {
        const MINUTE: u64 = 16_000 * 60;
        let window = DIARIZATION_WINDOW_SAMPLES as u64;
        // Speech everywhere except pauses at 14:00 and 14:50.
        let spans = [
            SampleSpan { start: 0, end: 14 * MINUTE },
            SampleSpan { start: 14 * MINUTE + 16_000, end: 14 * MINUTE + 50 * 16_000 },
            SampleSpan { start: 14 * MINUTE + 52 * 16_000, end: 40 * MINUTE },
        ];
        let ends = diarization_window_ends(&spans, (40 * MINUTE) as usize);
        assert_eq!(ends[0] as u64, 14 * MINUTE + 51 * 16_000);
        // No pause near the next mark: cut at it.
        assert_eq!(ends[1] as u64, ends[0] as u64 + window);
        assert_eq!(*ends.last().unwrap(), (40 * MINUTE) as usize);
        assert!(ends.windows(2).all(|pair| pair[0] < pair[1]));
        // Under 16 minutes stays one window.
        assert_eq!(diarization_window_ends(&[], (16 * MINUTE) as usize), vec![(16 * MINUTE) as usize]);
        // Silence at the mark: cut at the mark.
        assert_eq!(diarization_window_ends(&[], (20 * MINUTE) as usize)[0] as u64, window);
    }

    #[cfg(any(target_os = "windows", target_os = "android"))]
    #[test]
    fn confirmed_lines_keep_their_samples() {
        let mut confirmed = ConfirmedSpeech::default();
        assert_eq!(confirmed.confirm("Hola.", Some(SampleSpan { start: 10, end: 20 })), "Hola.");
        assert_eq!(confirmed.confirm("   ", Some(SampleSpan { start: 30, end: 40 })), "");
        assert_eq!(confirmed.confirm("Sí.", None), "Sí.");
        assert_eq!(confirmed.text, "Hola. Sí.");
        assert_eq!(confirmed.lines.len(), 2);
        assert_eq!(confirmed.lines[1].span, SampleSpan { start: 20, end: 20 });
    }

    #[cfg(any(target_os = "windows", target_os = "android"))]
    #[test]
    fn speaker_turns_transcribe_overlapping_speech_once() {
        let segment = |start_seconds, end_seconds, speaker| DiarizationSegment {
            start_seconds,
            end_seconds,
            speaker,
        };
        let turns = speaker_turns(&[
            segment(0.0, 10.0, 0),
            // A "sí" over speaker 0 and the rest of speaker 0.
            segment(9.5, 9.9, 1),
            segment(10.2, 20.0, 0),
            // Speaker 1 starts before speaker 0 finishes.
            segment(19.0, 30.0, 1),
            segment(29.9, 30.1, 0),
        ]);
        assert_eq!(turns, vec![segment(0.0, 20.0, 0), segment(20.0, 30.0, 1)]);
        assert!(turns.windows(2).all(|pair| pair[0].end_seconds <= pair[1].start_seconds));
    }

    #[test]
    fn capabilities_do_not_claim_unvalidated_models() {
        let capabilities = current_capabilities(
            &SpeechModelStatusDto {
                schema_version: 1,
                profiles: Vec::new(),
            },
            false,
            false,
            "unavailable",
        );
        assert_eq!(
            capabilities.supported,
            cfg!(any(target_os = "windows", target_os = "android"))
        );
        assert!(!capabilities.asr_model_installed);
        assert!(!capabilities.diarization_model_installed);
    }

    #[test]
    fn confirmed_utterances_keep_words_spoken_again() {
        let mut transcript = "¿Estamos de acuerdo? Sí.".to_string();
        assert_eq!(append_text(&mut transcript, "Sí."), "Sí.");
        assert_eq!(append_text(&mut transcript, "  Sí,   claro. "), "Sí, claro.");
        assert_eq!(transcript, "¿Estamos de acuerdo? Sí. Sí. Sí, claro.");
        assert_eq!(append_text(&mut transcript, "   "), "");
    }

    #[test]
    fn external_audio_does_not_append_repeated_partial_hypotheses() {
        let mut transcript = String::new();
        for _ in 0..3 {
            assert!(!commit_external_update(
                &mut transcript,
                &RecognitionUpdate {
                    text: "Que es Spring Boot.".to_string(),
                    endpoint_detected: false,
                    span: None,
                },
                false,
            ));
        }
        commit_external_update(
            &mut transcript,
            &RecognitionUpdate {
                text: "Que es Spring Boot.".to_string(),
                endpoint_detected: false,
                span: None,
            },
            true,
        );
        assert_eq!(transcript, "Que es Spring Boot.");
    }

    #[test]
    fn partial_text_hides_a_fuzzy_repetition_of_the_confirmed_end() {
        assert_eq!(
            unconfirmed_suffix(
                "Hola, buenas tardes. Hoy estamos en Espartinas para hacer unas preguntas",
                "las partinas para hacer unas preguntas sobre el ahorro",
            ),
            "sobre el ahorro"
        );
    }

    #[test]
    fn confirmed_utterances_keep_distinct_text() {
        let mut transcript = "Hola equipo".to_string();
        append_text(&mut transcript, "empecemos la reunión");
        assert_eq!(transcript, "Hola equipo empecemos la reunión");
    }

    #[test]
    fn partial_text_hides_audio_overlap_already_confirmed() {
        assert_eq!(
            unconfirmed_suffix(
                "La transcripcion de la reunion",
                "de la reunion continua ahora"
            ),
            "continua ahora"
        );
    }

    #[test]
    fn confirmed_chunks_repair_false_sentence_boundaries() {
        let mut transcript = "preguntas sobre el ahorro.".to_string();
        append_text(&mut transcript, "y contaminación del agua");
        assert_eq!(
            transcript,
            "preguntas sobre el ahorro y contaminación del agua"
        );

        let mut transcript = "Hay muchos factores.".to_string();
        append_text(&mut transcript, "pero sobre todo está el ser humano");
        assert_eq!(
            transcript,
            "Hay muchos factores, pero sobre todo está el ser humano"
        );
    }

    #[test]
    fn diarization_moves_word_allocation_to_a_sentence_boundary() {
        let words =
            "Cuáles son las causas de la contaminación del agua? Hombre hay muchos factores"
                .split_whitespace()
                .collect::<Vec<_>>();
        let boundary = nearest_sentence_boundary(&words, 0, 6);
        assert_eq!(
            words[..boundary].join(" "),
            "Cuáles son las causas de la contaminación del agua?"
        );
        assert_eq!(words[boundary..].join(" "), "Hombre hay muchos factores");
    }

    /// Runs every 16 kHz mono WAV of `NOTIA_FINAL_PROBE_DIR` through the live
    /// recognizer, the diarization and both ways of giving the text its
    /// speakers: live lines (`.attr.txt`, re-transcribing the lines across a
    /// speaker change) and the final pass with turbo (`.final-turbo.txt`) and
    /// large-v3 (`.final-large.txt`). Each line is `start_ms end_ms speaker
    /// text`. `NOTIA_FINAL_EXPECTED` passes the count of `<stem>.speakers.txt`.
    #[cfg(target_os = "windows")]
    #[test]
    #[ignore = "requires the speech models, the whisper.cpp and sherpa-onnx runtimes and a GPU"]
    fn final_pass_probe_recordings() {
        use super::{final_pass_chunk, transcribe_piece};
        use crate::dto::speech::SpeechTranscriptSegmentDto;
        use crate::services::speech_model_repository::{ResolvedDiarizationModel, DIARIZATION_CLUSTERING_THRESHOLD};
        use crate::services::speech_worker::{samples_to_ms, StreamingRecognizer};
        use crate::services::whisper_recognizer::{WhisperAsrConfig, WhisperFinalPass, WhisperVadRecognizer};
        use std::path::PathBuf;

        let Ok(dir) = std::env::var("NOTIA_FINAL_PROBE_DIR") else {
            return;
        };
        let root = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/.."));
        let whisper_runtime = root.join("resources/whisper/runtime/windows-x86_64/notia_whisper.dll");
        let sherpa_runtime = root.join("resources/speech/runtime/windows-x86_64/sherpa-onnx-c-api.dll");
        let models = root.join("resources/speech/models");
        let turbo = models.join("whisper-large-v3-turbo/ggml-large-v3-turbo-q8_0.bin");
        let large = models.join("whisper-large-v3/ggml-large-v3-q5_0.bin");
        let asr = WhisperAsrConfig {
            model: turbo.clone(),
            vad: models.join("whisper-large-v3-turbo/silero_vad.onnx"),
            num_threads: 4,
            language: "es".to_string(),
        };
        let diarization_model = ResolvedDiarizationModel {
            segmentation: models.join("speaker-diarization-v2/segmentation.onnx"),
            embedding: models.join("speaker-diarization-v2/embedding.onnx"),
            num_threads: 4,
            clustering_threshold: DIARIZATION_CLUSTERING_THRESHOLD,
        };
        let format = |segments: &[SpeechTranscriptSegmentDto]| {
            segments
                .iter()
                .map(|segment| {
                    format!("{} {} {} {}", segment.start_ms, segment.end_ms, segment.speaker_id.as_deref().unwrap_or("-"), segment.text)
                })
                .collect::<Vec<_>>()
                .join("\n")
        };
        let mut wavs = std::fs::read_dir(&dir)
            .expect("probe dir")
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| path.extension().is_some_and(|extension| extension == "wav"))
            .collect::<Vec<_>>();
        wavs.sort();
        for wav in wavs {
            let samples = hound::WavReader::open(&wav)
                .expect("wav")
                .into_samples::<i16>()
                .map(|sample| f32::from(sample.expect("sample")) / f32::from(i16::MAX))
                .collect::<Vec<_>>();
            let stem = wav.file_stem().and_then(|stem| stem.to_str()).unwrap_or_default().to_string();
            let output = |suffix: &str, text: String| std::fs::write(wav.with_file_name(format!("{stem}.{suffix}.txt")), text).expect("write");

            let mut recognizer = WhisperVadRecognizer::load(&whisper_runtime, &sherpa_runtime, &asr).expect("recognizer");
            recognizer.enable_live_partials();
            let mut confirmed = ConfirmedSpeech::default();
            for batch in samples.chunks(3_200) {
                let update = recognizer.accept_waveform(batch).expect("accept");
                if update.endpoint_detected {
                    confirmed.confirm(&update.text, update.span);
                }
            }
            let update = recognizer.finish().expect("finish");
            confirmed.confirm(&update.text, update.span);
            let lines = confirmed.lines.iter().collect::<Vec<_>>();

            let expected = std::env::var("NOTIA_FINAL_EXPECTED")
                .ok()
                .and_then(|_| std::fs::read_to_string(wav.with_extension("speakers.txt")).ok())
                .and_then(|text| text.trim().parse().ok());
            let diarization =
                crate::services::sherpa_diarization::process(&sherpa_runtime, &diarization_model, &samples, expected).expect("diarize");

            // Live lines, as `attribute_lines` gives them their speakers.
            let turns = speaker_turns(&diarization.segments);
            let mut keep_live = |lines: &[&super::ConfirmedLine]| -> Result<Vec<SpeechTranscriptSegmentDto>, String> {
                let mut segments = Vec::new();
                for line in lines {
                    let (start, end) = (line.span.start as usize, (line.span.end as usize).min(samples.len()));
                    let pieces = line_pieces(&turns, start as f32 / 16_000.0, end as f32 / 16_000.0);
                    let mut push = |start: usize, end: usize, speaker: Option<i32>, text: String| {
                        segments.push(SpeechTranscriptSegmentDto {
                            id: String::new(),
                            start_ms: samples_to_ms(start as u64),
                            end_ms: samples_to_ms(end as u64),
                            speaker_id: speaker.map(|speaker| format!("speaker-{}", speaker + 1)),
                            text,
                            is_final: true,
                        })
                    };
                    if pieces.len() > 1 {
                        for piece in pieces {
                            let (piece_start, piece_end) = piece.sample_range();
                            let text = transcribe_piece(&mut recognizer, &samples[piece_start..piece_end.min(samples.len())])?;
                            if !text.is_empty() {
                                push(piece_start, piece_end, piece.speaker, text);
                            }
                        }
                    } else {
                        push(start, end, pieces[0].speaker, line.text.clone());
                    }
                }
                Ok(segments)
            };
            output("attr", format(&keep_live(&lines).expect("attribute")));
            drop(keep_live);
            drop(recognizer);

            for (label, model, alignment) in [("final-turbo", &turbo, "large-v3-turbo"), ("final-large", &large, "large-v3")] {
                let started = std::time::Instant::now();
                let mut pass = WhisperFinalPass::load(&whisper_runtime, model, alignment, "es").expect("final pass").expect("a GPU");
                let chunk = final_pass_chunk(&mut pass, &samples, 0, &diarization, &lines, &mut |_, _| Ok(())).expect("final pass");
                output(label, format(&chunk.segments));
                eprintln!("{stem} {label}: {} segments in {:.1} s", chunk.segments.len(), started.elapsed().as_secs_f32());
            }
            eprintln!("{stem}: {} live lines, {} speakers", lines.len(), diarization.speaker_count);
        }
    }
}
