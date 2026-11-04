use crate::dto::speech::{
    AudioMonitorPayload, AudioMonitorResultDto, AudioMonitorStopPayload, SherpaRuntimeStatusDto,
    SpeechAudioInputStatusDto, SpeechCapabilitiesDto, SpeechModelStatusDto, SpeechSessionPayload,
    StartSpeechSessionPayload, StartSpeechSessionResultDto,
};
use crate::mobile_continuity;
use crate::mobile_speech_permission::AndroidSpeechPermissionState;
use crate::services::sherpa_runtime;
use crate::services::speech_audio;
use crate::services::speech_model_repository;
use crate::services::speech_service;
use crate::services::speech_service::{SpeechPhase, SpeechRuntimeState};
use crate::host::{AppHandle, Manager, State};

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PrepareSpeechModelPayload {
    language: String,
}

pub async fn prepare_speech_model(
    payload: PrepareSpeechModelPayload,
    app: AppHandle,
) -> Result<(), String> {
    let language = payload.language.trim().to_string();
    if language.is_empty() {
        return Err("El idioma del reconocimiento de voz no puede estar vacío.".to_string());
    }
    let worker_app = app.clone();
    crate::host::async_runtime::spawn_blocking(move || {
        speech_service::prepare_recognizer(&worker_app, &language)
    })
    .await
    .map_err(|error| format!("Falló la preparación del reconocimiento de voz: {error}"))?
}

pub fn get_speech_capabilities(
    app: AppHandle,
    permission_state: State<'_, AndroidSpeechPermissionState>,
) -> Result<SpeechCapabilitiesDto, String> {
    // La comprobación de capacidades es informativa y no debe quedar inutilizable
    // por un directorio de modelos ausente (por ejemplo, en el primer arranque).
    // La resolución estricta se hace al iniciar la sesión y allí se devuelve el
    // error accionable de modelo faltante/inválido.
    let model_status =
        speech_model_repository::inspect_installed_models(&app).unwrap_or_else(|_| {
            SpeechModelStatusDto {
                schema_version: 1,
                profiles: Vec::new(),
            }
        });
    let runtime = sherpa_runtime::probe(&app);
    let permission = microphone_permission(&permission_state);
    let audio_available = if permission == "granted" {
        speech_audio::probe_audio_input().available
    } else {
        cfg!(target_os = "android")
    };
    Ok(speech_service::current_capabilities(
        &model_status,
        runtime.compatible,
        audio_available,
        &permission,
    ))
}

pub fn get_speech_model_status(app: AppHandle) -> Result<SpeechModelStatusDto, String> {
    speech_model_repository::inspect_installed_models(&app)
}

pub fn probe_speech_audio_input(
    permission_state: State<'_, AndroidSpeechPermissionState>,
) -> SpeechAudioInputStatusDto {
    let permission = microphone_permission(&permission_state);
    if cfg!(target_os = "android") && permission != "granted" {
        return SpeechAudioInputStatusDto {
            supported: true,
            available: true,
            device_label: None,
            sample_rate: None,
            channels: None,
            error_message: None,
        };
    }
    speech_audio::probe_audio_input()
}

pub fn probe_sherpa_runtime(app: AppHandle) -> SherpaRuntimeStatusDto {
    sherpa_runtime::probe(&app)
}

pub async fn start_speech_session(
    payload: StartSpeechSessionPayload,
    app: AppHandle,
    state: State<'_, SpeechRuntimeState>,
    permission_state: State<'_, AndroidSpeechPermissionState>,
) -> Result<StartSpeechSessionResultDto, String> {
    speech_service::validate_start_input(&payload.language, payload.max_duration_seconds)?;
    let sources = speech_audio::CaptureSources::resolve(payload.capture_microphone, payload.capture_system_audio)?;
    if payload.expected_speakers.is_some_and(|count| !(2..=MAX_EXPECTED_SPEAKERS).contains(&count)) {
        return Err(format!("La cantidad de hablantes debe estar entre 2 y {MAX_EXPECTED_SPEAKERS}."));
    }
    let mut payload = payload;
    if let Some(meeting) = payload.meeting.as_mut() {
        meeting.ai_context = meeting
            .ai_context
            .take()
            .map(notia_backend_core::meeting_ai::MeetingAiContext::normalized)
            .transpose()
            .map_err(|error| error.message)?;
    }
    let phase = *state
        .phase
        .lock()
        .map_err(|_| "No se pudo bloquear el estado de la sesion de voz.".to_string())?;
    if phase.is_active() || !phase.can_transition_to(SpeechPhase::Preparing) {
        return Err("Ya existe una sesion de voz activa.".to_string());
    }
    if !speech_service::runtime_integrated() {
        return Err(speech_service::not_integrated_error());
    }
    #[cfg(any(target_os = "windows", target_os = "android"))]
    {
        #[cfg(target_os = "android")]
        crate::mobile_speech_permission::ensure_microphone_permission(&permission_state)?;
        #[cfg(not(target_os = "android"))]
        let _ = permission_state;
        // La captura de voz continúa aunque el WebView pase a segundo plano:
        // se declara el trabajo en un servicio en primer plano con el tipo
        // `microphone`. Si el sistema rechaza el servicio, la sesión sigue
        // igualmente y solo se pierde la garantía de continuidad.
        #[cfg(target_os = "android")]
        let continuity_started = {
            let continuity_state = app.state::<mobile_continuity::ContinuityState>();
            mobile_continuity::begin_android_work(continuity_state.inner(), Some("microphone"))
        };
        #[cfg(not(target_os = "android"))]
        let continuity_started = true;
        let session_id = uuid::Uuid::new_v4().to_string();
        {
            let mut phase = state
                .phase
                .lock()
                .map_err(|_| "No se pudo bloquear el estado de voz.".to_string())?;
            *phase = SpeechPhase::Preparing;
        }
        speech_service::emit_session_state(
            &app,
            &session_id,
            crate::dto::speech::SpeechSessionStateDto::Preparing { progress: None },
        );
        let language = payload.language;
        let diarization_enabled = payload.diarization_enabled;
        let capture = speech_service::SessionCapture {
            sources,
            max_duration_seconds: payload.max_duration_seconds,
            expected_speakers: payload.expected_speakers,
            meeting: payload.meeting.is_some(),
        };
        let worker_app = app.clone();
        let worker_session_id = session_id.clone();
        // The meeting exists before the first line can arrive.
        if let Some(options) = &payload.meeting {
            crate::meeting::begin(&app, &session_id, sources, options);
        }
        let result = match crate::host::async_runtime::spawn_blocking(move || {
            let model = speech_model_repository::resolve_asr_model(&worker_app, &language)?;
            let diarization_model = diarization_enabled
                .then(|| speech_model_repository::resolve_diarization_model(&worker_app, &language))
                .transpose()?;
            let worker_state = worker_app.state::<SpeechRuntimeState>();
            speech_service::start_platform_session(&worker_app, &worker_state, worker_session_id, model, diarization_model, capture)
        })
        .await
        {
            Ok(result) => result,
            Err(error) => {
                if let Ok(mut phase) = state.phase.lock() {
                    *phase = SpeechPhase::Idle;
                }
                crate::meeting::discard_session(&app, &session_id);
                if continuity_started {
                    mobile_continuity::end_android_work(
                        app.state::<mobile_continuity::ContinuityState>().inner(),
                    );
                }
                return Err(format!(
                    "El worker de inicio de voz finalizo inesperadamente: {error}"
                ));
            }
        };
        if let Err(error) = result {
            if let Ok(mut phase) = state.phase.lock() {
                *phase = SpeechPhase::Idle;
            }
            crate::meeting::discard_session(&app, &session_id);
            if continuity_started {
                mobile_continuity::end_android_work(
                    app.state::<mobile_continuity::ContinuityState>().inner(),
                );
            }
            return Err(error);
        }
        Ok(StartSpeechSessionResultDto { session_id })
    }
    #[cfg(not(any(target_os = "windows", target_os = "android")))]
    {
        let _ = (app, state, sources, payload.diarization_enabled, payload.meeting);
        Err(speech_service::not_integrated_error())
    }
}

/// Most speakers the person can ask the diarization to find.
const MAX_EXPECTED_SPEAKERS: u32 = 10;

/// Opens the chosen sources only to show their levels before recording.
pub fn start_audio_monitor(
    payload: AudioMonitorPayload,
    app: AppHandle,
    state: State<'_, SpeechRuntimeState>,
    permission_state: State<'_, AndroidSpeechPermissionState>,
) -> Result<AudioMonitorResultDto, String> {
    let sources = speech_audio::CaptureSources::resolve(payload.microphone, payload.system)?;
    #[cfg(target_os = "android")]
    crate::mobile_speech_permission::ensure_microphone_permission(&permission_state)?;
    #[cfg(not(target_os = "android"))]
    let _ = permission_state;
    let phase = *state
        .phase
        .lock()
        .map_err(|_| "No se pudo bloquear el estado de voz.".to_string())?;
    if phase != SpeechPhase::Idle {
        return Err("No se puede probar el audio mientras se graba.".to_string());
    }
    let monitor_id = speech_service::start_audio_monitor(&app, &state, sources)?;
    Ok(AudioMonitorResultDto { monitor_id })
}

pub fn stop_audio_monitor(
    payload: AudioMonitorStopPayload,
    state: State<'_, SpeechRuntimeState>,
) -> Result<(), String> {
    speech_service::stop_audio_monitor(&state, &payload.monitor_id)
}

/// Finishes a session that is separating speakers with its text alone.
/// State of a session that is still recording or separating its speakers.
pub fn speech_session_state(
    payload: SpeechSessionPayload,
    state: State<'_, SpeechRuntimeState>,
) -> Result<crate::dto::speech::SpeechSessionStateDto, String> {
    validate_session_command(&payload)?;
    speech_service::session_state(&state, &payload.session_id)
}

pub fn skip_speech_diarization(
    payload: SpeechSessionPayload,
    state: State<'_, SpeechRuntimeState>,
) -> Result<(), String> {
    validate_session_command(&payload)?;
    speech_service::skip_diarization(&state, &payload.session_id)
}

fn microphone_permission(_state: &AndroidSpeechPermissionState) -> String {
    #[cfg(target_os = "android")]
    {
        return match crate::mobile_speech_permission::check_microphone_permission(_state) {
            Ok(crate::host::plugin::PermissionState::Granted) => "granted",
            Ok(crate::host::plugin::PermissionState::Denied) => "denied",
            Ok(crate::host::plugin::PermissionState::Prompt)
            | Ok(crate::host::plugin::PermissionState::PromptWithRationale) => "prompt",
            Err(_) => "unavailable",
        }
        .to_string();
    }
    #[cfg(not(target_os = "android"))]
    {
        "granted".to_string()
    }
}

fn validate_session_command(payload: &SpeechSessionPayload) -> Result<(), String> {
    if payload.session_id.trim().is_empty() || payload.session_id.len() > 128 {
        return Err("El identificador de la sesion de voz no es valido.".to_string());
    }
    Ok(())
}

pub fn pause_speech_session(
    payload: SpeechSessionPayload,
    app: AppHandle,
    state: State<'_, SpeechRuntimeState>,
) -> Result<(), String> {
    validate_session_command(&payload)?;
    if !speech_service::runtime_integrated() {
        return Err(speech_service::not_integrated_error());
    }
    let elapsed_ms = speech_service::validate_active_session(&state, &payload.session_id)?;
    {
        let mut phase = state
            .phase
            .lock()
            .map_err(|_| "No se pudo bloquear el estado de voz.".to_string())?;
        if !phase.can_transition_to(SpeechPhase::Paused) {
            return Err("La sesion no puede pausarse en su estado actual.".to_string());
        }
        *phase = SpeechPhase::Paused;
    }
    if let Err(error) = speech_service::pause_platform_audio(&state) {
        if let Ok(mut phase) = state.phase.lock() {
            *phase = SpeechPhase::Recording;
        }
        return Err(error);
    }
    #[cfg(any(target_os = "windows", target_os = "android"))]
    speech_service::emit_session_state(
        &app,
        &payload.session_id,
        crate::dto::speech::SpeechSessionStateDto::Paused { elapsed_ms },
    );
    Ok(())
}

pub fn resume_speech_session(
    payload: SpeechSessionPayload,
    app: AppHandle,
    state: State<'_, SpeechRuntimeState>,
) -> Result<(), String> {
    validate_session_command(&payload)?;
    if !speech_service::runtime_integrated() {
        return Err(speech_service::not_integrated_error());
    }
    let elapsed_ms = speech_service::validate_active_session(&state, &payload.session_id)?;
    {
        let mut phase = state
            .phase
            .lock()
            .map_err(|_| "No se pudo bloquear el estado de voz.".to_string())?;
        if !phase.can_transition_to(SpeechPhase::Recording) {
            return Err("La sesion no puede reanudarse en su estado actual.".to_string());
        }
        *phase = SpeechPhase::Recording;
    }
    if let Err(error) = speech_service::resume_platform_audio(&state) {
        if let Ok(mut phase) = state.phase.lock() {
            *phase = SpeechPhase::Paused;
        }
        return Err(error);
    }
    #[cfg(any(target_os = "windows", target_os = "android"))]
    speech_service::emit_session_state(
        &app,
        &payload.session_id,
        crate::dto::speech::SpeechSessionStateDto::Recording {
            elapsed_ms,
            has_speech: true,
        },
    );
    Ok(())
}

pub async fn stop_speech_session(
    payload: SpeechSessionPayload,
    app: AppHandle,
    state: State<'_, SpeechRuntimeState>,
) -> Result<(), String> {
    validate_session_command(&payload)?;
    if let Err(error) = speech_service::validate_active_session(&state, &payload.session_id) {
        // A meeting whose session is gone finishes with the lines it has.
        if speech_service::release_lost_session(&state, &payload.session_id)
            && crate::meeting::finish_lost_session(&app, &payload.session_id)
        {
            return Ok(());
        }
        return Err(error);
    }
    {
        let mut phase = state
            .phase
            .lock()
            .map_err(|_| "No se pudo bloquear el estado de voz.".to_string())?;
        if !phase.can_transition_to(SpeechPhase::Finalizing) {
            return Err("La sesion no puede finalizarse en su estado actual.".to_string());
        }
        *phase = SpeechPhase::Finalizing;
    }
    #[cfg(any(target_os = "windows", target_os = "android"))]
    speech_service::emit_session_state(
        &app,
        &payload.session_id,
        crate::dto::speech::SpeechSessionStateDto::Finalizing {
            progress: None,
            stage: Some("transcribing"),
        },
    );
    let worker_app = app.clone();
    let stopped = crate::host::async_runtime::spawn_blocking(move || {
        let worker_state = worker_app.state::<SpeechRuntimeState>();
        speech_service::stop_platform_session(&worker_state)
    })
    .await
    .map_err(|error| {
        format!("El worker de finalizacion de voz finalizo inesperadamente: {error}")
    })
    .and_then(|stopped| stopped);
    #[cfg(target_os = "android")]
    {
        let continuity_state = app.state::<mobile_continuity::ContinuityState>();
        let _ = mobile_continuity::end_android_work(continuity_state.inner());
    }
    stopped
}

pub fn consume_speech_turn(
    payload: SpeechSessionPayload,
    app: AppHandle,
    state: State<'_, SpeechRuntimeState>,
) -> Result<String, String> {
    validate_session_command(&payload)?;
    let elapsed_ms = speech_service::validate_active_session(&state, &payload.session_id)?;
    let text = speech_service::consume_platform_turn(&state, &payload.session_id)?;
    if text.is_empty() {
        return Err("No se detecto voz en este turno.".to_string());
    }
    if let Ok(mut phase) = state.phase.lock() {
        *phase = SpeechPhase::Paused;
    }
    #[cfg(any(target_os = "windows", target_os = "android"))]
    speech_service::emit_session_state(
        &app,
        &payload.session_id,
        crate::dto::speech::SpeechSessionStateDto::Paused { elapsed_ms },
    );
    Ok(text)
}

pub fn cancel_speech_session(
    payload: SpeechSessionPayload,
    app: AppHandle,
    state: State<'_, SpeechRuntimeState>,
) -> Result<(), String> {
    validate_session_command(&payload)?;
    if let Err(error) = speech_service::validate_active_session(&state, &payload.session_id) {
        // A meeting whose session is gone is dropped all the same.
        if speech_service::release_lost_session(&state, &payload.session_id)
            && crate::meeting::discard_session(&app, &payload.session_id)
        {
            return Ok(());
        }
        return Err(error);
    }
    let result = speech_service::cancel_platform_audio(&state);
    crate::meeting::discard_session(&app, &payload.session_id);
    #[cfg(target_os = "android")]
    {
        let continuity_state = app.state::<mobile_continuity::ContinuityState>();
        let _ = mobile_continuity::end_android_work(continuity_state.inner());
    }
    result
}
