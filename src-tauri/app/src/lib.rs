//! Notia application layer: use cases, service state, platform adapters and
//! the command registry. It does not depend on Tauri; a host (the Tauri
//! window, later the headless server) builds the application with
//! [`create_app`], runs [`startup_hooks`] and routes commands through
//! [`registry::dispatch`].

pub mod host;
pub mod registry;
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub mod server;

pub mod backend;
pub mod backend_ollama;
mod backend_runtime;
pub mod backend_tauri;
mod backup;
mod database;
mod finance;
mod finance_agent_inputs;
mod finance_edits;
mod finance_matching;
mod finance_migration;
mod finance_reconciliation;
mod finance_records;
mod finance_screen;
mod agenda;
mod agenda_view;
mod agenda_sync;
mod holidays;
mod routine;
mod routine_dashboard;
mod routine_tools;
mod agenda_tools;
mod coldpass;
mod secret_clipboard;
mod config_crypto;
mod client_status;
mod connection;
mod device_role;
mod host_client;
mod host_mirror;
mod host_sync;
mod row_sync;
#[cfg(not(any(target_os = "android", target_os = "ios")))]
mod host_server;
mod config_vault;
mod device_secret;
mod agent_autonomy;
mod ai_actions;
mod ai_task_runner;
mod recipes;
mod gym;
mod health;
mod app_auth;
mod agent_history;
mod agent_knowledge;
mod agent_pending;
mod agent_scripts;
mod agent_workspace;
mod ai_chat;
mod ai_tasks;
mod chat_history;
mod device_preferences;
mod telegram_worker;
mod folder_contexts;
mod public_spaces;
mod library_documents;
mod library_catalog;
mod library_config;
mod page_links;
mod note_page_mode;
mod note_ink;
mod note_preview;
mod mail_accounts;
mod mail_tools;
mod gitbook_blocks;
mod library_graph;
mod library_inventory;
mod library_session;
mod published_library;
mod chat_agents;
mod library_document_adapter;
mod library_registry;
mod library_users;
mod local_time;
mod meeting;
mod meeting_history;
mod mermaid_editor;
mod meeting_media;
mod home;
mod terminal;
mod browser;
mod node_packages;
mod webhooks;
mod whatsapp;
mod weather;
mod image_generation;
mod recent_documents;
mod user_auth;

mod commands {
    pub mod bluetooth;
    pub mod qwen3_tts;
    pub mod remote_speech;
    pub mod speech;
    pub mod telegram;
}
mod dto {
    pub mod bluetooth;
    pub mod speech;
}
mod filesystem;
mod mobile_ai_bridge;
mod mobile_biometric;
mod mobile_continuity;
mod mobile_directory_picker;
mod mobile_speech_permission;
mod notia_timer;
mod task_manager_commands;
mod task_manager_fs;
mod task_manager_publication;
mod task_manager_publication_source;
mod task_manager_store;
mod services {
    pub mod ai_service;
    pub mod bluetooth_service;
    #[cfg(all(feature = "bluetooth", not(any(target_os = "android", target_os = "ios"))))]
    pub mod coldpass_secure_link;
    pub mod finance_extraction;
    pub mod finance_external;
    pub mod local_agreement;
    pub mod media_decoder;
    pub mod qwen3_tts_service;
    pub mod sherpa_diarization;
    pub mod sherpa_runtime;
    pub mod sherpa_vad;
    pub mod spanish_transcript;
    pub mod audio_devices;
    pub mod speech_audio;
    #[cfg(any(target_os = "windows", target_os = "android"))]
    pub mod speech_levels;
    pub mod speech_model_repository;
    pub mod speech_service;
    #[cfg(any(target_os = "windows", target_os = "android"))]
    pub mod speech_file;
    pub mod speech_worker;
    pub mod telegram_audio;
    pub mod telegram_service;
    pub mod whisper_recognizer;
    pub mod whisper_runtime;
}
mod state {
    pub mod bluetooth_state;
}

use host::plugin::TauriPlugin;
use host::{AppContext, AppPaths, HostPorts, Manager};

/// Builds the application with the state of every service registered.
pub fn create_app(paths: AppPaths, ports: HostPorts) -> AppContext {
    // On desktop every event also reaches the hub the Host mode and the
    // headless server deliver to their clients.
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    let hub = std::sync::Arc::new(server::events::EventHub::default());
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    let ports = HostPorts {
        events: Some(std::sync::Arc::new(server::events::TeeEvents { first: ports.events, hub: hub.clone() })),
        ..ports
    };
    let app = AppContext::new(paths, ports);
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    app.manage(server::SharedEventHub(hub));
    app.manage(state::bluetooth_state::ColdPassBluetoothState::default());
    app.manage(services::speech_service::SpeechRuntimeState::default());
    app.manage(services::qwen3_tts_service::Qwen3TtsRuntimeState::default());
    app.manage(filesystem::watch::LibraryTreeWatchState::default());
    app.manage(task_manager_publication::TaskManagerPublicationState::default());
    app.manage(task_manager_commands::TaskManagerBackendState::default());
    app.manage(backend_runtime::BackendRuntimeState::default());
    app.manage(browser::BrowserState::default());
    app.manage(whatsapp::WhatsAppState::default());
    app.manage(webhooks::WebhookState::default());
    app.manage(library_registry::LibraryBindingRegistry::default());
    app.manage(library_session::LibrarySessionState::default());
    app.manage(ai_chat::AiChatState::default());
    app.manage(ai_tasks::AiTasksState::default());
    app.manage(meeting::MeetingState::default());
    app.manage(meeting_media::MeetingMediaState::default());
    app.manage(coldpass::ColdPassState::default());
    app.manage(config_vault::ConfigVaultState::default());
    app.manage(library_catalog::LibraryCatalogState::default());
    app.manage(agent_workspace::AgentWorkspaceState::default());
    app.manage(agent_history::AgentHistoryState::default());
    app.manage(chat_history::ChatHistoryState::default());
    app.manage(device_preferences::DevicePreferencesState::default());
    app.manage(telegram_worker::TelegramWorkerState::default());
    app.manage(library_graph::LibraryGraphState::default());
    app.manage(backup::service::BackupState::default());
    app.manage(commands::remote_speech::RemoteSpeechState::default());
    app.manage(connection::ConnectionState::default());
    app.manage(device_role::DeviceRoleState::default());
    app.manage(host_client::HostClientState::default());
    app.manage(host_mirror::MirrorState::default());
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    app.manage(host_server::HostServerState::default());
    app.manage(public_spaces::PublicSpacesState::default());
    app
}

/// Starts what the mode of this device needs (Settings → General → «Modo
/// de ejecución»): the server of a host or the link of a client. Only the
/// app window calls it; the headless server serves on its own.
pub fn start_device_services(app: &AppContext) {
    connection::start(app);
}

/// Startup hooks, in the order they must run. Each hook keeps the name of
/// its native plugin so the host registers the Kotlin side under it.
pub fn startup_hooks() -> Vec<TauriPlugin> {
    vec![
        library_registry::init(),
        backup::service::init(),
        telegram_worker::init(),
        whatsapp::init(),
        webhooks::init(),
        agent_autonomy::init(),
        ai_actions::init(),
        ai_task_runner::init(),
        agenda_sync::init(),
        task_manager_publication_source::init(),
        public_spaces::init(),
        database::init(),
        mobile_ai_bridge::init(),
        mobile_continuity::init(),
        mobile_biometric::init(),
        mobile_directory_picker::init(),
        mobile_speech_permission::init(),
        services::speech_service::init_preload(),
    ]
}
