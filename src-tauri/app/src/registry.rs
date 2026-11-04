//! Registry of the application commands.
//!
//! Every command of the application is routed here. Each entry reads its
//! arguments from the JSON object (camelCase keys, as Tauri did), injects the
//! application, a service state or the calling window, and returns the
//! result as JSON. Commands without `async` run on the calling thread;
//! `async` ones return a future the host drives. The entries were generated
//! from the former Tauri handler list; new commands are added by hand with
//! the same shape.
//!
//! Clients reach the registry through one entry point, [`APP_INVOKE`], whose
//! body is `{ command, args }`.

#![allow(clippy::redundant_closure)]

use std::future::Future;
use std::pin::Pin;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::host::{AppHandle, Manager, Window};

/// JSON result of a command: the value or the error it returned.
pub type Reply = Result<Value, Value>;

/// Outcome of routing a command.
pub enum Dispatch {
    Ready(Reply),
    Pending(Pin<Box<dyn Future<Output = Reply> + Send>>),
}

/// Runs `command` with `args` on behalf of the client window
/// `window_label`. Returns `None` when the application has no such command.
pub fn dispatch(app: &AppHandle, window_label: &str, command: &str, args: &Value) -> Option<Dispatch> {
    let route = route(command)?;
    Some(route(app, window_label, command, args).unwrap_or_else(|error| Dispatch::Ready(Err(error))))
}

fn route(command: &str) -> Option<Route> {
    let route: Route = match command {
        "backend_export_markdown_document" => backend_export_markdown_document,
        "backend_export_diagram_sources" => backend_export_diagram_sources,
        "backend_read_library_config" => backend_read_library_config,
        "backend_write_library_config" => backend_write_library_config,
        "backend_ensure_library_config" => backend_ensure_library_config,
        "backend_library_graph" => backend_library_graph,
        "library_link_targets" => library_link_targets,
        "library_link_suggestions" => library_link_suggestions,
        "backend_library_graph_search" => backend_library_graph_search,
        "backend_library_search" => backend_library_search,
        "backend_library_catalog" => backend_library_catalog,
        "app_auth_status" => app_auth_status,
        "app_auth_login" => app_auth_login,
        "app_auth_first_login" => app_auth_first_login,
        "app_auth_create_password" => app_auth_create_password,
        "app_auth_change_password" => app_auth_change_password,
        "app_auth_logout" => app_auth_logout,
        "backend_agent_history" => backend_agent_history,
        "backend_save_pending_clarification" => backend_save_pending_clarification,
        "backend_pending_clarification" => backend_pending_clarification,
        "backend_clear_pending_clarification" => backend_clear_pending_clarification,
        "backend_answer_pending_clarification" => backend_answer_pending_clarification,
        "backend_agent_history_diff" => backend_agent_history_diff,
        "backend_agent_prompts" => backend_agent_prompts,
        "backend_select_agent_prompt" => backend_select_agent_prompt,
        "backend_save_agent_memories" => backend_save_agent_memories,
        "backend_device_preferences" => backend_device_preferences,
        "backend_publish_task_manager" => backend_publish_task_manager,
        "backend_save_device_preferences" => backend_save_device_preferences,
        "backend_create_chat" => backend_create_chat,
        "backend_load_chat" => backend_load_chat,
        "backend_list_chats" => backend_list_chats,
        "backend_match_chat" => backend_match_chat,
        "backend_set_chat_context" => backend_set_chat_context,
        "backend_set_chat_settings" => backend_set_chat_settings,
        "backend_set_chat_pinned" => backend_set_chat_pinned,
        "backend_rename_chat" => backend_rename_chat,
        "backend_save_chat" => backend_save_chat,
        "backend_chat_image_previews" => backend_chat_image_previews,
        "backend_classify_chat_file" => backend_classify_chat_file,
        "backend_save_library_catalog" => backend_save_library_catalog,
        "backend_backup_status" => backend_backup_status,
        "backend_pick_backup_directory" => backend_pick_backup_directory,
        "backend_disable_backups" => backend_disable_backups,
        "backend_migrate_backup_directory" => backend_migrate_backup_directory,
        "backend_sync_page_link" => backend_sync_page_link,
        "markdown_blocks_resolve" => markdown_blocks_resolve,
        "markdown_set_page_mode" => markdown_set_page_mode,
        "backend_preview_editor_preferences" => backend_preview_editor_preferences,
        "markdown_ink_load" => markdown_ink_load,
        "markdown_ink_add" => markdown_ink_add,
        "markdown_ink_remove" => markdown_ink_remove,
        "markdown_ink_restore" => markdown_ink_restore,
        "markdown_ink_replace" => markdown_ink_replace,
        "markdown_ink_select" => markdown_ink_select,
        "markdown_ink_move" => markdown_ink_move,
        "markdown_ink_erase" => markdown_ink_erase,
        "markdown_ink_convert" => markdown_ink_convert,
        "markdown_export_diagram" => markdown_export_diagram,
        "markdown_note_preview" => markdown_note_preview,
        "backend_mail_accounts" => backend_mail_accounts,
        "backend_connect_mail_account" => backend_connect_mail_account,
        "backend_cancel_mail_account_connection" => backend_cancel_mail_account_connection,
        "backend_disconnect_mail_account" => backend_disconnect_mail_account,
        "backend_set_mail_account_type" => backend_set_mail_account_type,
        "backend_save_google_cloud_credentials" => backend_save_google_cloud_credentials,
        "backend_check_google_cloud_credentials" => backend_check_google_cloud_credentials,
        "backend_remove_google_cloud_credentials" => backend_remove_google_cloud_credentials,
        "backend_import_google_cloud_json" => backend_import_google_cloud_json,
        "webhook_view" => webhook_view,
        "webhook_save_server" => webhook_save_server,
        "webhook_save_action" => webhook_save_action,
        "webhook_new_token" => webhook_new_token,
        "webhook_delete_action" => webhook_delete_action,
        "whatsapp_view" => whatsapp_view,
        "whatsapp_connect" => whatsapp_connect,
        "whatsapp_disconnect" => whatsapp_disconnect,
        "whatsapp_set_chat" => whatsapp_set_chat,
        "coldpass_unlock" => coldpass_unlock,
        "coldpass_status" => coldpass_status,
        "coldpass_generate_password" => coldpass_generate_password,
        "coldpass_rate_password" => coldpass_rate_password,
        "mermaid_document" => mermaid_document,
        "mermaid_edit" => mermaid_edit,
        "coldpass_lock" => coldpass_lock,
        "coldpass_save_entry" => coldpass_save_entry,
        "coldpass_delete_entry" => coldpass_delete_entry,
        "coldpass_pick_csv_import" => coldpass_pick_csv_import,
        "coldpass_confirm_import" => coldpass_confirm_import,
        "coldpass_copy_secret" => coldpass_copy_secret,
        "coldpass_biometric_status" => coldpass_biometric_status,
        "coldpass_enable_biometric" => coldpass_enable_biometric,
        "coldpass_disable_biometric" => coldpass_disable_biometric,
        "coldpass_unlock_biometric" => coldpass_unlock_biometric,
        "stop_library_tree_watch" => stop_library_tree_watch,
        "list_library_roles" => list_library_roles,
        "create_library_role" => create_library_role,
        "list_library_users" => list_library_users,
        "create_library_user" => create_library_user,
        "update_library_user_password" => update_library_user_password,
        "delete_library_user" => delete_library_user,
        "update_library_user_name" => update_library_user_name,
        "update_library_user_role" => update_library_user_role,
        "update_library_user_contexts" => update_library_user_contexts,
        "resolve_library_telegram_user" => resolve_library_telegram_user,
        "find_library_user" => find_library_user,
        "link_library_user_telegram" => link_library_user_telegram,
        "unlink_library_user_telegram" => unlink_library_user_telegram,
        "routine_get_dashboard" => routine_get_dashboard,
        "routine_apply_mutation" => routine_apply_mutation,
        "agenda_get_view" => agenda_get_view,
        "agenda_apply_mutation" => agenda_apply_mutation,
        "ai_actions_dashboard" => ai_actions_dashboard,
        "ai_action_preview" => ai_action_preview,
        "ai_action_get" => ai_action_get,
        "ai_action_create" => ai_action_create,
        "ai_action_update" => ai_action_update,
        "ai_action_set_enabled" => ai_action_set_enabled,
        "ai_action_delete" => ai_action_delete,
        "ai_action_runs" => ai_action_runs,
        "ai_action_retry" => ai_action_retry,
        "ai_action_test" => ai_action_test,
        "ai_tasks_list" => ai_tasks_list,
        "ai_task_get" => ai_task_get,
        "ai_task_thinking" => ai_task_thinking,
        "ai_task_answer" => ai_task_answer,
        "ai_task_cancel" => ai_task_cancel,
        "ai_task_resume" => ai_task_resume,
        "ai_task_delete" => ai_task_delete,
        "recipes_grid" => recipes_grid,
        "recipes_detail" => recipes_detail,
        "recipes_photo" => recipes_photo,
        "recipes_create" => recipes_create,
        "recipes_update" => recipes_update,
        "recipes_delete" => recipes_delete,
        "health_dashboard" => health_dashboard,
        "health_apply" => health_apply,
        "health_generate_plan" => health_generate_plan,
        "health_estimate_meal" => health_estimate_meal,
        "gym_view" => gym_view,
        "gym_resume" => gym_resume,
        "gym_apply" => gym_apply,
        "gym_exercise" => gym_exercise,
        "gym_catalog_apply" => gym_catalog_apply,
        "gym_set_media" => gym_set_media,
        "gym_video" => gym_video,
        "gym_equipment_images" => gym_equipment_images,
        "gym_body" => gym_body,
        "finance_dev_list_tables" => finance_dev_list_tables,
        "finance_dev_query_table" => finance_dev_query_table,
        "finance_dev_query_sql" => finance_dev_query_sql,
        "finance_dev_seed_demo_data" => finance_dev_seed_demo_data,
        "finance_clear_all_data" => finance_clear_all_data,
        "finance_overview" => finance_overview,
        "finance_movements" => finance_movements,
        "finance_products" => finance_products,
        "finance_salary_savings" => finance_salary_savings,
        "finance_dollar_quotes" => finance_dollar_quotes,
        "get_speech_capabilities" => get_speech_capabilities,
        "prepare_speech_model" => prepare_speech_model,
        "prepare_device_speech_model" => prepare_speech_model,
        "get_speech_model_status" => get_speech_model_status,
        "probe_speech_audio_input" => probe_speech_audio_input,
        "speech_audio_devices" => speech_audio_devices,
        "probe_sherpa_runtime" => probe_sherpa_runtime,
        "start_speech_session" => start_speech_session,
        "pause_speech_session" => pause_speech_session,
        "resume_speech_session" => resume_speech_session,
        "consume_speech_turn" => consume_speech_turn,
        "speech_remote_audio" => speech_remote_audio,
        "speech_remote_audio_cancel" => speech_remote_audio_cancel,
        "stop_speech_session" => stop_speech_session,
        "cancel_speech_session" => cancel_speech_session,
        "skip_speech_diarization" => skip_speech_diarization,
        "speech_session_state" => speech_session_state,
        "start_audio_monitor" => start_audio_monitor,
        "stop_audio_monitor" => stop_audio_monitor,
        "home_dashboard" => home_dashboard,
        "weather_home" => weather_home,
        "weather_get_location" => weather_get_location,
        "weather_search_locations" => weather_search_locations,
        "weather_set_location" => weather_set_location,
        "backend_image_models_status" => backend_image_models_status,
        "backend_download_image_models" => backend_download_image_models,
        "backend_cancel_image_models_download" => backend_cancel_image_models_download,
        "backend_delete_image_models" => backend_delete_image_models,
        "meeting_media_begin" => meeting_media_begin,
        "meeting_media_chunk" => meeting_media_chunk,
        "meeting_media_finish" => meeting_media_finish,
        "meeting_media_discard" => meeting_media_discard,
        "meeting_start_file_session" => meeting_start_file_session,
        "meeting_snapshot" => meeting_snapshot,
        "meeting_context" => meeting_context,
        "meeting_notes_text" => meeting_notes_text,
        "meeting_discard" => meeting_discard,
        "meeting_add_mark" => meeting_add_mark,
        "meeting_remove_mark" => meeting_remove_mark,
        "meeting_set_notes" => meeting_set_notes,
        "meeting_set_live_answers" => meeting_set_live_answers,
        "meeting_set_ai_notes" => meeting_set_ai_notes,
        "meeting_call_notes_agent" => meeting_call_notes_agent,
        "meeting_ai_context_options" => meeting_ai_context_options,
        "meeting_save_ai_context_choice" => meeting_save_ai_context_choice,
        "meeting_regenerate_answer" => meeting_regenerate_answer,
        "meeting_pin_answer" => meeting_pin_answer,
        "meeting_rename_speaker" => meeting_rename_speaker,
        "meeting_merge_speakers" => meeting_merge_speakers,
        "meeting_generate_insights" => meeting_generate_insights,
        "meeting_save_note" => meeting_save_note,
        "meeting_export" => meeting_export,
        "meeting_task_boards" => meeting_task_boards,
        "meeting_send_tasks" => meeting_send_tasks,
        "meeting_history" => meeting_history,
        "meeting_open_saved" => meeting_open_saved,
        "get_qwen3_tts_status" => get_qwen3_tts_status,
        "reload_qwen3_tts" => reload_qwen3_tts,
        "synthesize_qwen3_tts_speech" => synthesize_qwen3_tts_speech,
        "qwen3_tts_speech_plan" => qwen3_tts_speech_plan,
        "prepare_qwen3_tts" => prepare_qwen3_tts,
        "check_telegram_bot" => check_telegram_bot,
        "revoke_library_binding" => revoke_library_binding,
        #[cfg(all(feature = "bluetooth", not(any(target_os = "android", target_os = "ios"))))]
        "coldpass_bluetooth_status" => coldpass_bluetooth_status,
        #[cfg(any(target_os = "android", target_os = "ios", not(feature = "bluetooth")))]
        "coldpass_bluetooth_status" => coldpass_bluetooth_status,
        #[cfg(all(feature = "bluetooth", not(any(target_os = "android", target_os = "ios"))))]
        "coldpass_bluetooth_connect" => coldpass_bluetooth_connect,
        #[cfg(any(target_os = "android", target_os = "ios", not(feature = "bluetooth")))]
        "coldpass_bluetooth_connect" => coldpass_bluetooth_connect,
        #[cfg(all(feature = "bluetooth", not(any(target_os = "android", target_os = "ios"))))]
        "coldpass_bluetooth_submit_pin" => coldpass_bluetooth_submit_pin,
        #[cfg(any(target_os = "android", target_os = "ios", not(feature = "bluetooth")))]
        "coldpass_bluetooth_submit_pin" => coldpass_bluetooth_submit_pin,
        #[cfg(all(feature = "bluetooth", not(any(target_os = "android", target_os = "ios"))))]
        "coldpass_bluetooth_authenticate" => coldpass_bluetooth_authenticate,
        #[cfg(any(target_os = "android", target_os = "ios", not(feature = "bluetooth")))]
        "coldpass_bluetooth_authenticate" => coldpass_bluetooth_authenticate,
        #[cfg(all(feature = "bluetooth", not(any(target_os = "android", target_os = "ios"))))]
        "coldpass_bluetooth_send_message" => coldpass_bluetooth_send_message,
        #[cfg(any(target_os = "android", target_os = "ios", not(feature = "bluetooth")))]
        "coldpass_bluetooth_send_message" => coldpass_bluetooth_send_message,
        #[cfg(all(feature = "bluetooth", not(any(target_os = "android", target_os = "ios"))))]
        "coldpass_bluetooth_disconnect" => coldpass_bluetooth_disconnect,
        #[cfg(any(target_os = "android", target_os = "ios", not(feature = "bluetooth")))]
        "coldpass_bluetooth_disconnect" => coldpass_bluetooth_disconnect,
        "get_task_manager_publication_url" => get_task_manager_publication_url,
        "get_task_manager_publication_status" => get_task_manager_publication_status,
        "open_task_manager_publication" => open_task_manager_publication,
        "stop_task_manager_publication" => stop_task_manager_publication,
        "begin_task_manager_publication_batch" => begin_task_manager_publication_batch,
        "end_task_manager_publication_batch" => end_task_manager_publication_batch,
        "ai_chat_send" => ai_chat_send,
        "ai_chat_answer" => ai_chat_answer,
        "ai_chat_cancel" => ai_chat_cancel,
        "ai_chat_interject" => ai_chat_interject,
        "ai_check_health" => ai_check_health,
        "ai_list_models" => ai_list_models,
        "ai_resolve_model" => ai_resolve_model,
        "ai_recognize_inkmath" => ai_recognize_inkmath,
        "chat_agents_catalog" => chat_agents_catalog,
        "library_open" => library_open,
        "library_refresh" => library_refresh,
        "library_read_directory" => library_read_directory,
        "library_read_document" => library_read_document,
        "library_write_document" => library_write_document,
        "library_mutate_entry" => library_mutate_entry,
        "library_set_folder_context" => library_set_folder_context,
        "public_spaces_view" => public_spaces_view,
        "public_spaces_save" => public_spaces_save,
        "public_space_candidates" => public_space_candidates,
        "public_space_open" => public_space_open,
        "library_pick_directory" => library_pick_directory,
        "library_list_files" => library_list_files,
        "library_list_folders" => library_list_folders,
        "task_manager_board_view" => task_manager_board_view,
        "task_manager_board_execute" => task_manager_board_execute,
        "task_manager_board_plan_preview" => task_manager_board_plan_preview,
        "task_manager_management_view" => task_manager_management_view,
        "task_manager_pomodoro" => task_manager_pomodoro,
        "task_manager_delete_pomodoro" => task_manager_delete_pomodoro,
        "task_manager_read_ticket_source" => task_manager_read_ticket_source,
        "task_manager_task_detail" => task_manager_task_detail,
        "task_manager_write_ticket_source" => task_manager_write_ticket_source,
        "connection_settings" => connection_settings,
        "save_connection_settings" => save_connection_settings,
        "test_host_connection" => test_host_connection,
        "client_open" => client_open,
        "sync_copy_now" => sync_copy_now,
        "pick_copy_folder" => pick_copy_folder,
        "host_sync_manifest" => host_sync_manifest,
        "host_sync_database" => host_sync_database,
        "host_sync_database_changes" => host_sync_database_changes,
        "host_sync_database_push" => host_sync_database_push,
        "host_sync_write" => host_sync_write,
        "host_sync_delete" => host_sync_delete,
        _ => return None,
    };
    Some(route)
}

type Route = fn(&AppHandle, &str, &str, &Value) -> Result<Dispatch, Value>;

/// Every command of the registry.
pub const COMMAND_NAMES: &[&str] = &[
    "backend_export_markdown_document",
    "backend_export_diagram_sources",
    "backend_read_library_config",
    "backend_write_library_config",
    "backend_ensure_library_config",
    "backend_library_graph",
    "library_link_targets",
    "library_link_suggestions",
    "backend_library_graph_search",
    "backend_library_search",
    "backend_library_catalog",
    "app_auth_status",
    "app_auth_login",
    "app_auth_first_login",
    "app_auth_create_password",
    "app_auth_change_password",
    "app_auth_logout",
    "backend_agent_history",
    "backend_save_pending_clarification",
    "backend_pending_clarification",
    "backend_clear_pending_clarification",
    "backend_answer_pending_clarification",
    "backend_agent_history_diff",
    "backend_agent_prompts",
    "backend_select_agent_prompt",
    "backend_save_agent_memories",
    "backend_device_preferences",
    "backend_publish_task_manager",
    "backend_save_device_preferences",
    "backend_create_chat",
    "backend_load_chat",
    "backend_list_chats",
    "backend_match_chat",
    "backend_set_chat_context",
    "backend_set_chat_settings",
    "backend_set_chat_pinned",
    "backend_rename_chat",
    "backend_save_chat",
    "backend_chat_image_previews",
    "backend_classify_chat_file",
    "backend_save_library_catalog",
    "backend_backup_status",
    "backend_pick_backup_directory",
    "backend_disable_backups",
    "backend_migrate_backup_directory",
    "backend_sync_page_link",
    "markdown_blocks_resolve",
    "markdown_set_page_mode",
    "backend_preview_editor_preferences",
    "markdown_ink_load",
    "markdown_ink_add",
    "markdown_ink_remove",
    "markdown_ink_restore",
    "markdown_ink_replace",
    "markdown_ink_select",
    "markdown_ink_move",
    "markdown_ink_erase",
    "markdown_ink_convert",
    "markdown_export_diagram",
    "markdown_note_preview",
    "backend_mail_accounts",
    "backend_connect_mail_account",
    "backend_cancel_mail_account_connection",
    "backend_disconnect_mail_account",
    "backend_set_mail_account_type",
    "backend_save_google_cloud_credentials",
    "backend_check_google_cloud_credentials",
    "backend_remove_google_cloud_credentials",
    "backend_import_google_cloud_json",
    "webhook_view",
    "webhook_save_server",
    "webhook_save_action",
    "webhook_new_token",
    "webhook_delete_action",
    "whatsapp_view",
    "whatsapp_connect",
    "whatsapp_disconnect",
    "whatsapp_set_chat",
    "coldpass_unlock",
    "coldpass_status",
    "coldpass_generate_password",
    "coldpass_rate_password",
    "mermaid_document",
    "mermaid_edit",
    "coldpass_lock",
    "coldpass_save_entry",
    "coldpass_delete_entry",
    "coldpass_pick_csv_import",
    "coldpass_confirm_import",
    "coldpass_copy_secret",
    "coldpass_biometric_status",
    "coldpass_enable_biometric",
    "coldpass_disable_biometric",
    "coldpass_unlock_biometric",
    "stop_library_tree_watch",
    "list_library_roles",
    "create_library_role",
    "list_library_users",
    "create_library_user",
    "update_library_user_password",
    "delete_library_user",
    "update_library_user_name",
    "update_library_user_role",
    "update_library_user_contexts",
    "resolve_library_telegram_user",
    "find_library_user",
    "link_library_user_telegram",
    "unlink_library_user_telegram",
    "routine_get_dashboard",
    "routine_apply_mutation",
    "agenda_get_view",
    "agenda_apply_mutation",
    "ai_actions_dashboard",
    "ai_action_preview",
    "ai_action_get",
    "ai_action_create",
    "ai_action_update",
    "ai_action_set_enabled",
    "ai_action_delete",
    "ai_action_runs",
    "ai_action_retry",
    "ai_action_test",
    "ai_tasks_list",
    "ai_task_get",
    "ai_task_thinking",
    "ai_task_answer",
    "ai_task_cancel",
    "ai_task_resume",
    "ai_task_delete",
    "recipes_grid",
    "recipes_detail",
    "recipes_photo",
    "recipes_create",
    "recipes_update",
    "recipes_delete",
    "health_dashboard",
    "health_apply",
    "health_generate_plan",
    "health_estimate_meal",
    "gym_view",
    "gym_resume",
    "gym_apply",
    "gym_exercise",
    "gym_catalog_apply",
    "gym_set_media",
    "gym_video",
    "gym_equipment_images",
    "gym_body",
    "finance_dev_list_tables",
    "finance_dev_query_table",
    "finance_dev_query_sql",
    "finance_dev_seed_demo_data",
    "finance_clear_all_data",
    "finance_overview",
    "finance_movements",
    "finance_products",
    "finance_salary_savings",
    "finance_dollar_quotes",
    "get_speech_capabilities",
    "prepare_speech_model",
    "prepare_device_speech_model",
    "get_speech_model_status",
    "probe_speech_audio_input",
    "speech_audio_devices",
    "probe_sherpa_runtime",
    "start_speech_session",
    "pause_speech_session",
    "resume_speech_session",
    "consume_speech_turn",
    "speech_remote_audio",
    "speech_remote_audio_cancel",
    "stop_speech_session",
    "cancel_speech_session",
    "skip_speech_diarization",
    "speech_session_state",
    "start_audio_monitor",
    "stop_audio_monitor",
    "home_dashboard",
    "weather_home",
    "weather_get_location",
    "weather_search_locations",
    "weather_set_location",
    "backend_image_models_status",
    "backend_download_image_models",
    "backend_cancel_image_models_download",
    "backend_delete_image_models",
    "meeting_media_begin",
    "meeting_media_chunk",
    "meeting_media_finish",
    "meeting_media_discard",
    "meeting_start_file_session",
    "meeting_snapshot",
    "meeting_context",
    "meeting_notes_text",
    "meeting_discard",
    "meeting_add_mark",
    "meeting_remove_mark",
    "meeting_set_notes",
    "meeting_set_live_answers",
    "meeting_set_ai_notes",
    "meeting_call_notes_agent",
    "meeting_ai_context_options",
    "meeting_save_ai_context_choice",
    "meeting_regenerate_answer",
    "meeting_pin_answer",
    "meeting_rename_speaker",
    "meeting_merge_speakers",
    "meeting_generate_insights",
    "meeting_save_note",
    "meeting_export",
    "meeting_task_boards",
    "meeting_send_tasks",
    "meeting_history",
    "meeting_open_saved",
    "get_qwen3_tts_status",
    "reload_qwen3_tts",
    "synthesize_qwen3_tts_speech",
    "qwen3_tts_speech_plan",
    "prepare_qwen3_tts",
    "check_telegram_bot",
    "revoke_library_binding",
    "coldpass_bluetooth_status",
    "coldpass_bluetooth_connect",
    "coldpass_bluetooth_submit_pin",
    "coldpass_bluetooth_authenticate",
    "coldpass_bluetooth_send_message",
    "coldpass_bluetooth_disconnect",
    "get_task_manager_publication_url",
    "get_task_manager_publication_status",
    "open_task_manager_publication",
    "stop_task_manager_publication",
    "begin_task_manager_publication_batch",
    "end_task_manager_publication_batch",
    "ai_chat_send",
    "ai_chat_answer",
    "ai_chat_cancel",
    "ai_chat_interject",
    "ai_check_health",
    "ai_list_models",
    "ai_resolve_model",
    "ai_recognize_inkmath",
    "chat_agents_catalog",
    "library_open",
    "library_refresh",
    "library_read_directory",
    "library_read_document",
    "library_write_document",
    "library_mutate_entry",
    "library_set_folder_context",
    "public_spaces_view",
    "public_spaces_save",
    "public_space_candidates",
    "public_space_open",
    "library_pick_directory",
    "library_list_files",
    "library_list_folders",
    "task_manager_board_view",
    "task_manager_board_execute",
    "task_manager_board_plan_preview",
    "task_manager_management_view",
    "task_manager_pomodoro",
    "task_manager_delete_pomodoro",
    "task_manager_read_ticket_source",
    "task_manager_task_detail",
    "task_manager_write_ticket_source",
    "connection_settings",
    "save_connection_settings",
    "test_host_connection",
    "client_open",
    "sync_copy_now",
    "pick_copy_folder",
    "host_sync_manifest",
    "host_sync_database",
    "host_sync_database_changes",
    "host_sync_database_push",
    "host_sync_write",
    "host_sync_delete",
];

/// Commands that act on devices of the computer running Notia (microphone,
/// Bluetooth, native pickers). Only the interface running on that computer
/// may call them; remote clients of the headless server cannot.
pub const LOCAL_ONLY_COMMANDS: &[&str] = &[
    // The image models of this device: a client never downloads ~11 GB on its host.
    "backend_image_models_status",
    "backend_download_image_models",
    "backend_cancel_image_models_download",
    "backend_delete_image_models",
    "probe_speech_audio_input",
    "speech_audio_devices",
    // The recognizer of this device (a client records on its own device).
    "prepare_device_speech_model",
    "start_speech_session",
    "pause_speech_session",
    "resume_speech_session",
    "consume_speech_turn",
    "stop_speech_session",
    "cancel_speech_session",
    "skip_speech_diarization",
    "speech_session_state",
    "start_audio_monitor",
    "stop_audio_monitor",
    "meeting_media_begin",
    "meeting_media_chunk",
    "meeting_media_finish",
    "meeting_media_discard",
    "meeting_start_file_session",
    "meeting_snapshot",
    "meeting_context",
    "meeting_notes_text",
    "meeting_discard",
    "meeting_add_mark",
    "meeting_remove_mark",
    "meeting_set_notes",
    "meeting_set_live_answers",
    "meeting_set_ai_notes",
    "meeting_call_notes_agent",
    "meeting_ai_context_options",
    "meeting_save_ai_context_choice",
    "meeting_regenerate_answer",
    "meeting_pin_answer",
    "meeting_rename_speaker",
    "meeting_merge_speakers",
    "meeting_generate_insights",
    "meeting_save_note",
    "meeting_export",
    "meeting_task_boards",
    "meeting_send_tasks",
    "meeting_history",
    "meeting_open_saved",
    "coldpass_bluetooth_status",
    "coldpass_bluetooth_connect",
    "coldpass_bluetooth_submit_pin",
    "coldpass_bluetooth_authenticate",
    "coldpass_bluetooth_send_message",
    "coldpass_bluetooth_disconnect",
    "library_pick_directory",
    "backend_pick_backup_directory",
    "coldpass_pick_csv_import",
    // The clipboard of this device.
    "coldpass_copy_secret",
    // The fingerprint sensor and Keystore of this device.
    "coldpass_biometric_status",
    "coldpass_enable_biometric",
    "coldpass_disable_biometric",
    "coldpass_unlock_biometric",
    // Opens the browser and listens on the loopback address of this computer.
    "backend_connect_mail_account",
    "backend_cancel_mail_account_connection",
    "backend_import_google_cloud_json",
    // The mode of this device: a client must not change its host's.
    "connection_settings",
    "save_connection_settings",
    "test_host_connection",
    "client_open",
    "sync_copy_now",
    "pick_copy_folder",
];

/// Whether a remote client (headless server) may call `command`.
pub fn is_remote_command(command: &str) -> bool {
    route(command).is_some() && !LOCAL_ONLY_COMMANDS.contains(&command)
}

/// A library user on a published Task Manager: every command runs as that
/// user and only over the published boards. The window and the headless
/// server act as the owner and use [`dispatch`] instead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishedScope {
    pub library_id: String,
    pub library_user_id: String,
    pub board_ids: Vec<String>,
}

/// Commands a published page may call. The rest of the registry is not
/// reachable from a publication.
pub const PUBLISHED_COMMANDS: &[&str] = &[
    "task_manager_board_view",
    "task_manager_board_execute",
    "task_manager_management_view",
    // The notes of the published boards' contexts: explorer, editor and graph.
    "library_read_directory",
    "library_read_document",
    "library_write_document",
    "library_mutate_entry",
    "backend_library_graph",
    "backend_library_graph_search",
    "task_manager_read_ticket_source",
    "task_manager_task_detail",
    "task_manager_write_ticket_source",
    "task_manager_pomodoro",
    // The ticket's editor: its page and pen preferences (kept in the page),
    // page mode and the strokes over the ticket.
    "backend_preview_editor_preferences",
    "markdown_set_page_mode",
    "markdown_ink_load",
    "markdown_ink_add",
    "markdown_ink_remove",
    "markdown_ink_restore",
    "markdown_ink_replace",
    "markdown_ink_select",
    "markdown_ink_move",
    "markdown_ink_erase",
];

pub fn is_published_command(command: &str) -> bool {
    PUBLISHED_COMMANDS.contains(&command)
}

/// Runs a published command for `scope`. Returns the result and whether it
/// changed data, or `None` when publications cannot call `command`.
pub fn dispatch_published(
    app: &AppHandle,
    scope: &PublishedScope,
    command: &str,
    args: &Value,
) -> Option<Result<(Value, bool), crate::backend::BackendError>> {
    use crate::task_manager_commands as task_manager;

    if !is_published_command(command) {
        return None;
    }
    let state = app.state::<task_manager::TaskManagerBackendState>();
    let registry = app.state::<crate::library_registry::LibraryBindingRegistry>();
    Some(task_manager::task_manager_execute_for_publication(
        app,
        &state,
        &registry,
        &scope.library_id,
        &scope.library_user_id,
        scope.board_ids.clone(),
        command,
        args,
    ))
}

/// Single command a host exposes to its clients.
pub const APP_INVOKE: &str = "app_invoke";

/// Label of the calls a Host-mode server receives from its clients: they
/// cannot change what only the host decides, such as Telegram.
pub const CLIENT_WINDOW_LABEL: &str = "client";

/// Body of [`APP_INVOKE`]: a command of the registry and its arguments.
#[derive(Debug, Deserialize)]
struct AppInvoke {
    command: String,
    #[serde(default)]
    args: Value,
}

/// Runs the body of an [`APP_INVOKE`] call. A malformed body or an unknown
/// command is answered with an error instead of reaching any handler.
pub fn dispatch_app_invoke(app: &AppHandle, window_label: &str, body: &Value) -> Dispatch {
    let request = match AppInvoke::deserialize(body) {
        Ok(request) => request,
        Err(error) => return Dispatch::Ready(Err(Value::String(format!("invalid {APP_INVOKE} body: {error}")))),
    };
    let args = if request.args.is_null() { Value::Object(Default::default()) } else { request.args };
    dispatch(app, window_label, &request.command, &args).unwrap_or_else(|| {
        Dispatch::Ready(Err(Value::String(format!("command {} not found", request.command))))
    })
}

fn arg<T: DeserializeOwned>(command: &str, args: &Value, key: &str) -> Result<T, Value> {
    match args.get(key) {
        Some(value) => serde_json::from_value(value.clone()).map_err(|error| {
            Value::String(format!("invalid args `{key}` for command `{command}`: {error}"))
        }),
        None => serde_json::from_value(Value::Null)
            .map_err(|_| Value::String(format!("command {command} missing required key {key}"))),
    }
}

fn reply_value<T: Serialize>(value: T) -> Reply {
    serde_json::to_value(value).map_err(|error| Value::String(error.to_string()))
}

fn reply_result<T: Serialize, E: Serialize>(result: Result<T, E>) -> Reply {
    match result {
        Ok(value) => reply_value(value),
        Err(error) => Err(serde_json::to_value(error).unwrap_or_else(|error| Value::String(error.to_string()))),
    }
}

fn backend_export_markdown_document(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_value(crate::filesystem::commands::backend_export_markdown_document(app.clone(), arg(command, args, "payload")?, app.state(), app.state()))))
}

fn backend_export_diagram_sources(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_result(crate::filesystem::commands::backend_export_diagram_sources(app.clone(), arg(command, args, "payload")?, app.state(), app.state()))))
}

fn backend_read_library_config(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_value(crate::library_config::backend_read_library_config(arg(command, args, "payload")?, app))))
}

fn backend_write_library_config(app: &AppHandle, window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    // A client never turns Telegram on: neither on its host (its calls come
    // with the client label) nor on its own device (its copy).
    let from_client = window_label == CLIENT_WINDOW_LABEL || crate::connection::is_client(app);
    Ok(Dispatch::Ready(reply_value(crate::library_config::backend_write_library_config(arg(command, args, "payload")?, app, from_client))))
}

fn backend_ensure_library_config(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_value(crate::library_config::backend_ensure_library_config(arg(command, args, "payload")?, app))))
}

fn backend_library_graph(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let arg0 = app.clone();
    let arg1 = arg(command, args, "payload")?;
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::library_graph::backend_library_graph(arg0, arg1).await) })))
}

fn library_link_targets(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let arg0 = app.clone();
    let arg1 = arg(command, args, "payload")?;
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::library_graph::library_link_targets(arg0, arg1).await) })))
}

fn library_link_suggestions(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let arg0 = app.clone();
    let arg1 = arg(command, args, "payload")?;
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::library_graph::library_link_suggestions(arg0, arg1).await) })))
}

fn backend_library_graph_search(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let arg0 = app.clone();
    let arg1 = arg(command, args, "payload")?;
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::library_graph::backend_library_graph_search(arg0, arg1).await) })))
}

fn backend_library_search(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let arg0 = app.clone();
    let arg1 = arg(command, args, "payload")?;
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::library_graph::backend_library_search(arg0, arg1).await) })))
}

fn backend_library_catalog(app: &AppHandle, _window_label: &str, _command: &str, _args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_result(crate::library_catalog::backend_library_catalog(app.clone(), app.state()))))
}

fn app_auth_status(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let arg0 = app.clone();
    let arg1 = arg(command, args, "payload")?;
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::app_auth::backend_app_auth_status(arg0, arg1).await) })))
}

fn app_auth_login(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let arg0 = app.clone();
    let arg1 = arg(command, args, "payload")?;
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::app_auth::backend_app_auth_login(arg0, arg1).await) })))
}

fn app_auth_first_login(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let arg0 = app.clone();
    let arg1 = arg(command, args, "payload")?;
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::app_auth::backend_app_auth_first_login(arg0, arg1).await) })))
}

fn app_auth_create_password(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let arg0 = app.clone();
    let arg1 = arg(command, args, "payload")?;
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::app_auth::backend_app_auth_create_password(arg0, arg1).await) })))
}

fn app_auth_change_password(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let arg0 = app.clone();
    let arg1 = arg(command, args, "payload")?;
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::app_auth::backend_app_auth_change_password(arg0, arg1).await) })))
}

fn app_auth_logout(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let arg0 = app.clone();
    let arg1 = arg(command, args, "payload")?;
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::app_auth::backend_app_auth_logout(arg0, arg1).await) })))
}

fn backend_agent_history(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_result(crate::agent_history::backend_agent_history(app.clone(), arg(command, args, "payload")?))))
}

fn backend_save_pending_clarification(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_result(crate::agent_pending::backend_save_pending_clarification(app.clone(), arg(command, args, "payload")?))))
}

fn backend_pending_clarification(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_value(crate::agent_pending::backend_pending_clarification(app.clone(), arg(command, args, "payload")?))))
}

fn backend_clear_pending_clarification(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready({ crate::agent_pending::backend_clear_pending_clarification(app.clone(), arg(command, args, "payload")?); Ok(Value::Null) }))
}

fn backend_answer_pending_clarification(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_value(crate::agent_pending::backend_answer_pending_clarification(app.clone(), arg(command, args, "payload")?))))
}

fn backend_agent_history_diff(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_result(crate::agent_history::backend_agent_history_diff(app.clone(), arg(command, args, "payload")?))))
}

fn backend_agent_prompts(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let arg0 = app.clone();
    let arg1 = arg(command, args, "payload")?;
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::agent_workspace::backend_agent_prompts(arg0, arg1).await) })))
}

fn backend_select_agent_prompt(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let arg0 = app.clone();
    let arg1 = arg(command, args, "payload")?;
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::agent_workspace::backend_select_agent_prompt(arg0, arg1).await) })))
}

fn backend_save_agent_memories(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let arg0 = app.clone();
    let arg1 = arg(command, args, "payload")?;
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::agent_workspace::backend_save_agent_memories(arg0, arg1).await) })))
}

fn backend_device_preferences(app: &AppHandle, _window_label: &str, _command: &str, _args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_value(crate::device_preferences::backend_device_preferences(app.clone()))))
}

fn backend_publish_task_manager(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let arg0 = app.clone();
    let arg1 = arg(command, args, "payload")?;
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::task_manager_publication_source::backend_publish_task_manager(arg0, arg1).await) })))
}

fn backend_save_device_preferences(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_result(crate::device_preferences::backend_save_device_preferences(app.clone(), arg(command, args, "preferences")?))))
}

fn backend_create_chat(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let arg0 = app.clone();
    let arg1 = arg(command, args, "payload")?;
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::chat_history::backend_create_chat(arg0, arg1).await) })))
}

fn backend_load_chat(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let arg0 = app.clone();
    let arg1 = arg(command, args, "payload")?;
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::chat_history::backend_load_chat(arg0, arg1).await) })))
}

fn backend_list_chats(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let arg0 = app.clone();
    let arg1 = arg(command, args, "payload")?;
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::chat_history::backend_list_chats(arg0, arg1).await) })))
}

fn backend_match_chat(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let arg0 = app.clone();
    let arg1 = arg(command, args, "payload")?;
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::chat_history::backend_match_chat(arg0, arg1).await) })))
}

fn backend_set_chat_context(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let arg0 = app.clone();
    let arg1 = arg(command, args, "payload")?;
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::chat_history::backend_set_chat_context(arg0, arg1).await) })))
}

fn backend_set_chat_settings(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let arg0 = app.clone();
    let arg1 = arg(command, args, "payload")?;
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::chat_history::backend_set_chat_settings(arg0, arg1).await) })))
}

fn backend_set_chat_pinned(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let arg0 = app.clone();
    let arg1 = arg(command, args, "payload")?;
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::chat_history::backend_set_chat_pinned(arg0, arg1).await) })))
}

fn backend_rename_chat(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let arg0 = app.clone();
    let arg1 = arg(command, args, "payload")?;
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::chat_history::backend_rename_chat(arg0, arg1).await) })))
}

fn backend_save_chat(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let arg0 = app.clone();
    let arg1 = arg(command, args, "payload")?;
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::chat_history::backend_save_chat(arg0, arg1).await) })))
}

fn backend_chat_image_previews(_app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let arg0 = arg(command, args, "payload")?;
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::chat_history::backend_chat_image_previews(arg0).await) })))
}

fn backend_classify_chat_file(_app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_result(crate::chat_history::backend_classify_chat_file(arg(command, args, "payload")?))))
}

/// The libraries of the host are folders of its disk: a client cannot
/// replace that list or drop a binding (an empty catalog sent by a client
/// emptied the host).
fn refuse_host_libraries_change(window_label: &str) -> Option<Dispatch> {
    (window_label == CLIENT_WINDOW_LABEL).then(|| {
        let error = crate::backend::BackendError::new(
            crate::backend::BackendErrorCode::Forbidden,
            "Las bibliotecas del host se agregan, quitan y eligen en el host.",
            false,
        );
        Dispatch::Ready(reply_result::<(), _>(Err(error)))
    })
}

fn backend_save_library_catalog(app: &AppHandle, window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    if let Some(refused) = refuse_host_libraries_change(window_label) {
        return Ok(refused);
    }
    Ok(Dispatch::Ready(reply_result(crate::library_catalog::backend_save_library_catalog(app.clone(), arg(command, args, "catalog")?, app.state()))))
}

fn backend_backup_status(app: &AppHandle, _window_label: &str, _command: &str, _args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_value(crate::backup::service::backend_backup_status(app.clone()))))
}

fn backend_pick_backup_directory(app: &AppHandle, _window_label: &str, _command: &str, _args: &Value) -> Result<Dispatch, Value> {
    let arg0 = app.clone();
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::backup::service::backend_pick_backup_directory(arg0).await) })))
}

fn backend_disable_backups(app: &AppHandle, _window_label: &str, _command: &str, _args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_result(crate::backup::service::backend_disable_backups(app.clone()))))
}

fn backend_migrate_backup_directory(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_result(crate::backup::service::backend_migrate_backup_directory(app.clone(), arg(command, args, "directoryPath")?))))
}

fn backend_sync_page_link(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let arg0 = app.clone();
    let arg1 = arg(command, args, "payload")?;
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::page_links::backend_sync_page_link(arg0, arg1).await) })))
}

fn coldpass_unlock(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let arg0 = app.clone();
    let arg1 = arg(command, args, "payload")?;
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::coldpass::coldpass_unlock(arg0, arg1).await) })))
}

fn coldpass_biometric_status(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let arg0 = app.clone();
    let arg1 = arg(command, args, "payload")?;
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::coldpass::coldpass_biometric_status(arg0, arg1).await) })))
}

fn coldpass_enable_biometric(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let arg0 = app.clone();
    let arg1 = arg(command, args, "payload")?;
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::coldpass::coldpass_enable_biometric(arg0, arg1).await) })))
}

fn coldpass_disable_biometric(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let arg0 = app.clone();
    let arg1 = arg(command, args, "payload")?;
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::coldpass::coldpass_disable_biometric(arg0, arg1).await) })))
}

fn coldpass_unlock_biometric(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let arg0 = app.clone();
    let arg1 = arg(command, args, "payload")?;
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::coldpass::coldpass_unlock_biometric(arg0, arg1).await) })))
}

fn coldpass_status(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let arg0 = app.clone();
    let arg1 = arg(command, args, "payload")?;
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::coldpass::coldpass_status(arg0, arg1).await) })))
}

fn coldpass_generate_password(_app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_result(crate::coldpass::coldpass_generate_password(arg(command, args, "payload")?))))
}

fn coldpass_rate_password(_app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_result(crate::coldpass::coldpass_rate_password(arg(command, args, "payload")?))))
}

fn mermaid_document(_app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_result(crate::mermaid_editor::mermaid_document(arg(command, args, "payload")?))))
}

fn mermaid_edit(_app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_result(crate::mermaid_editor::mermaid_edit(arg(command, args, "payload")?))))
}

fn coldpass_lock(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_result(crate::coldpass::coldpass_lock(arg(command, args, "payload")?, app.state()))))
}

fn coldpass_save_entry(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let arg0 = app.clone();
    let arg1 = arg(command, args, "payload")?;
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::coldpass::coldpass_save_entry(arg0, arg1).await) })))
}

fn coldpass_delete_entry(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let arg0 = app.clone();
    let arg1 = arg(command, args, "payload")?;
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::coldpass::coldpass_delete_entry(arg0, arg1).await) })))
}

fn coldpass_pick_csv_import(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let arg0 = app.clone();
    let arg1 = arg(command, args, "payload")?;
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::coldpass::coldpass_pick_csv_import(arg0, arg1).await) })))
}

fn coldpass_confirm_import(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let arg0 = app.clone();
    let arg1 = arg(command, args, "payload")?;
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::coldpass::coldpass_confirm_import(arg0, arg1).await) })))
}

fn coldpass_copy_secret(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_result(crate::coldpass::coldpass_copy_secret(app.clone(), arg(command, args, "payload")?))))
}

fn stop_library_tree_watch(app: &AppHandle, window_label: &str, _command: &str, _args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_value(crate::filesystem::watch::stop_library_tree_watch(Window::new(app.clone(), window_label), app.state()))))
}

fn list_library_roles(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_result(crate::library_users::list_library_roles(app.clone(), arg(command, args, "payload")?))))
}

fn create_library_role(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_result(crate::library_users::create_library_role(app.clone(), arg(command, args, "payload")?))))
}

fn list_library_users(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_result(crate::library_users::list_library_users(app.clone(), arg(command, args, "payload")?))))
}

fn create_library_user(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_result(crate::library_users::create_library_user(app.clone(), arg(command, args, "payload")?))))
}

fn update_library_user_password(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_result(crate::library_users::update_library_user_password(app.clone(), app.state(), arg(command, args, "payload")?))))
}

fn delete_library_user(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_result(crate::library_users::delete_library_user(app.clone(), app.state(), arg(command, args, "payload")?))))
}

fn update_library_user_name(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_result(crate::library_users::update_library_user_name(app.clone(), arg(command, args, "payload")?))))
}

fn update_library_user_role(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_result(crate::library_users::update_library_user_role(app.clone(), arg(command, args, "payload")?))))
}

fn update_library_user_contexts(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_result(crate::library_users::update_library_user_contexts(app.clone(), app.state(), arg(command, args, "payload")?))))
}

fn resolve_library_telegram_user(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_result(crate::library_users::resolve_library_telegram_user(app.clone(), arg(command, args, "payload")?))))
}

fn find_library_user(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_result(crate::library_users::find_library_user(app.clone(), arg(command, args, "payload")?))))
}

fn link_library_user_telegram(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_result(crate::library_users::link_library_user_telegram(app.clone(), arg(command, args, "payload")?))))
}

fn unlink_library_user_telegram(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_result(crate::library_users::unlink_library_user_telegram(app.clone(), arg(command, args, "payload")?))))
}

fn routine_get_dashboard(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_result(crate::routine::routine_get_dashboard(app.clone(), arg(command, args, "context")?))))
}

fn routine_apply_mutation(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_result(crate::routine::routine_apply_mutation(app.clone(), arg(command, args, "payload")?))))
}

fn agenda_get_view(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let (arg0, arg1, arg2) = (app.clone(), arg(command, args, "context")?, arg(command, args, "request")?);
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::agenda::agenda_get_view(arg0, arg1, arg2).await) })))
}

fn agenda_apply_mutation(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let (arg0, arg1) = (app.clone(), arg(command, args, "payload")?);
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::agenda::agenda_apply_mutation(arg0, arg1).await) })))
}

fn ai_actions_dashboard(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let (arg0, arg1, arg2, arg3) = (app.clone(), arg(command, args, "context")?, arg(command, args, "filter")?, arg(command, args, "query")?);
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::ai_actions::ai_actions_dashboard(arg0, arg1, arg2, arg3).await) })))
}

fn ai_action_preview(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let (arg0, arg1, arg2, arg3) = (app.clone(), arg(command, args, "context")?, arg(command, args, "actionId")?, arg(command, args, "input")?);
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::ai_actions::ai_action_preview(arg0, arg1, arg2, arg3).await) })))
}

fn ai_action_get(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let (arg0, arg1, arg2) = (app.clone(), arg(command, args, "context")?, arg(command, args, "actionId")?);
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::ai_actions::ai_action_get(arg0, arg1, arg2).await) })))
}

fn ai_action_create(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let (arg0, arg1, arg2) = (app.clone(), arg(command, args, "context")?, arg(command, args, "input")?);
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::ai_actions::ai_action_create(arg0, arg1, arg2).await) })))
}

fn ai_action_update(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let (arg0, arg1, arg2, arg3) = (app.clone(), arg(command, args, "context")?, arg(command, args, "actionId")?, arg(command, args, "input")?);
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::ai_actions::ai_action_update(arg0, arg1, arg2, arg3).await) })))
}

fn ai_action_set_enabled(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let (arg0, arg1, arg2, arg3) = (app.clone(), arg(command, args, "context")?, arg(command, args, "actionId")?, arg(command, args, "enabled")?);
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::ai_actions::ai_action_set_enabled(arg0, arg1, arg2, arg3).await) })))
}

fn ai_action_delete(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let (arg0, arg1, arg2) = (app.clone(), arg(command, args, "context")?, arg(command, args, "actionId")?);
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::ai_actions::ai_action_delete(arg0, arg1, arg2).await) })))
}

fn ai_action_runs(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let (arg0, arg1, arg2) = (app.clone(), arg(command, args, "context")?, arg(command, args, "actionId")?);
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::ai_actions::ai_action_runs(arg0, arg1, arg2).await) })))
}

fn ai_action_retry(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let (arg0, arg1, arg2) = (app.clone(), arg(command, args, "context")?, arg(command, args, "runId")?);
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::ai_actions::ai_action_retry(arg0, arg1, arg2).await) })))
}

fn ai_action_test(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let (arg0, arg1, arg2) = (app.clone(), arg(command, args, "context")?, arg(command, args, "input")?);
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::ai_actions::ai_action_test(arg0, arg1, arg2).await) })))
}

fn ai_tasks_list(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let (arg0, arg1) = (app.clone(), arg(command, args, "context")?);
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::ai_task_runner::ai_tasks_list(arg0, arg1).await) })))
}

fn ai_task_get(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let (arg0, arg1, arg2) = (app.clone(), arg(command, args, "context")?, arg(command, args, "taskId")?);
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::ai_task_runner::ai_task_get(arg0, arg1, arg2).await) })))
}

fn ai_task_thinking(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let (arg0, arg1, arg2) = (app.clone(), arg(command, args, "context")?, arg(command, args, "taskId")?);
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::ai_task_runner::ai_task_thinking(arg0, arg1, arg2).await) })))
}

fn ai_task_answer(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let (arg0, arg1, arg2, arg3, arg4) =
        (app.clone(), arg(command, args, "context")?, arg(command, args, "taskId")?, arg(command, args, "approvalId")?, arg(command, args, "answer")?);
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::ai_task_runner::ai_task_answer(arg0, arg1, arg2, arg3, arg4).await) })))
}

fn ai_task_cancel(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let (arg0, arg1, arg2) = (app.clone(), arg(command, args, "context")?, arg(command, args, "taskId")?);
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::ai_task_runner::ai_task_cancel(arg0, arg1, arg2).await) })))
}

fn ai_task_resume(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let (arg0, arg1, arg2) = (app.clone(), arg(command, args, "context")?, arg(command, args, "taskId")?);
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::ai_task_runner::ai_task_resume(arg0, arg1, arg2).await) })))
}

fn ai_task_delete(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let (arg0, arg1, arg2) = (app.clone(), arg(command, args, "context")?, arg(command, args, "taskId")?);
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::ai_task_runner::ai_task_delete(arg0, arg1, arg2).await) })))
}

fn recipes_grid(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let (arg0, arg1, arg2) = (app.clone(), arg(command, args, "context")?, arg(command, args, "query")?);
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::recipes::recipes_grid(arg0, arg1, arg2).await) })))
}

fn recipes_detail(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let (arg0, arg1, arg2) = (app.clone(), arg(command, args, "context")?, arg(command, args, "id")?);
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::recipes::recipes_detail(arg0, arg1, arg2).await) })))
}

fn recipes_photo(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let (arg0, arg1, arg2) = (app.clone(), arg(command, args, "context")?, arg(command, args, "id")?);
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::recipes::recipes_photo(arg0, arg1, arg2).await) })))
}

fn recipes_create(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let (arg0, arg1, arg2, arg3) = (app.clone(), arg(command, args, "context")?, arg(command, args, "input")?, arg(command, args, "photo")?);
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::recipes::recipes_create(arg0, arg1, arg2, arg3).await) })))
}

fn recipes_update(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let (arg0, arg1, arg2, arg3, arg4) = (app.clone(), arg(command, args, "context")?, arg(command, args, "id")?, arg(command, args, "input")?, arg(command, args, "photo")?);
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::recipes::recipes_update(arg0, arg1, arg2, arg3, arg4).await) })))
}

fn recipes_delete(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let (arg0, arg1, arg2) = (app.clone(), arg(command, args, "context")?, arg(command, args, "id")?);
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::recipes::recipes_delete(arg0, arg1, arg2).await) })))
}

fn health_dashboard(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let (arg0, arg1, arg2) = (app.clone(), arg(command, args, "context")?, arg(command, args, "query")?);
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::health::health_dashboard(arg0, arg1, arg2).await) })))
}

fn health_apply(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let (arg0, arg1, arg2, arg3) = (app.clone(), arg(command, args, "context")?, arg(command, args, "mutation")?, arg(command, args, "query")?);
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::health::health_apply(arg0, arg1, arg2, arg3).await) })))
}

fn health_generate_plan(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let (arg0, arg1, arg2) = (app.clone(), arg(command, args, "context")?, arg(command, args, "query")?);
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::health::health_generate_plan(arg0, arg1, arg2).await) })))
}

fn health_estimate_meal(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let (arg0, arg1, arg2) = (app.clone(), arg(command, args, "context")?, arg(command, args, "description")?);
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::health::health_estimate_meal(arg0, arg1, arg2).await) })))
}

fn gym_view(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let (arg0, arg1, arg2) = (app.clone(), arg(command, args, "context")?, arg(command, args, "query")?);
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::gym::gym_view(arg0, arg1, arg2).await) })))
}

fn gym_resume(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let (arg0, arg1) = (app.clone(), arg(command, args, "context")?);
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::gym::gym_resume(arg0, arg1).await) })))
}

fn gym_apply(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let (arg0, arg1, arg2, arg3) = (app.clone(), arg(command, args, "context")?, arg(command, args, "mutation")?, arg(command, args, "query")?);
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::gym::gym_apply(arg0, arg1, arg2, arg3).await) })))
}

fn gym_exercise(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let (arg0, arg1, arg2) = (app.clone(), arg(command, args, "context")?, arg(command, args, "exerciseId")?);
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::gym::gym_exercise(arg0, arg1, arg2).await) })))
}

fn gym_catalog_apply(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let (arg0, arg1, arg2, arg3) = (app.clone(), arg(command, args, "context")?, arg(command, args, "mutation")?, arg(command, args, "photo")?);
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::gym::gym_catalog_apply(arg0, arg1, arg2, arg3).await) })))
}

fn gym_set_media(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let (arg0, arg1, arg2, arg3) = (app.clone(), arg(command, args, "context")?, arg(command, args, "exerciseId")?, arg(command, args, "media")?);
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::gym::gym_set_media(arg0, arg1, arg2, arg3).await) })))
}

fn gym_video(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let (arg0, arg1, arg2) = (app.clone(), arg(command, args, "context")?, arg(command, args, "exerciseId")?);
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::gym::gym_video(arg0, arg1, arg2).await) })))
}

fn gym_equipment_images(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let (arg0, arg1) = (app.clone(), arg(command, args, "context")?);
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::gym::gym_equipment_images(arg0, arg1).await) })))
}

fn gym_body(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let (arg0, arg1) = (app.clone(), arg(command, args, "context")?);
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::gym::gym_body(arg0, arg1).await) })))
}

fn finance_dev_list_tables(_app: &AppHandle, _window_label: &str, _command: &str, _args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_value(crate::finance::finance_dev_list_tables())))
}

fn finance_dev_query_table(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_result(crate::finance::finance_dev_query_table(app.clone(), arg(command, args, "payload")?))))
}

fn finance_dev_query_sql(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_result(crate::finance::finance_dev_query_sql(app.clone(), arg(command, args, "payload")?))))
}

fn finance_dev_seed_demo_data(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_result(crate::finance::finance_dev_seed_demo_data(app.clone(), arg(command, args, "context")?))))
}

fn finance_clear_all_data(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_result(crate::finance::finance_clear_all_data(app.clone(), arg(command, args, "context")?))))
}

fn finance_overview(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_result(crate::finance_screen::finance_overview(app.clone(), arg(command, args, "payload")?))))
}

fn finance_movements(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_result(crate::finance_screen::finance_movements(app.clone(), arg(command, args, "payload")?))))
}

fn finance_products(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_result(crate::finance_screen::finance_products(app.clone(), arg(command, args, "payload")?))))
}

fn finance_salary_savings(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let arg0 = app.clone();
    let arg1 = arg(command, args, "payload")?;
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::finance_screen::finance_salary_savings(arg0, arg1).await) })))
}

fn finance_dollar_quotes(_app: &AppHandle, _window_label: &str, _command: &str, _args: &Value) -> Result<Dispatch, Value> {

    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::services::finance_external::finance_dollar_quotes().await) })))
}

fn get_speech_capabilities(app: &AppHandle, _window_label: &str, _command: &str, _args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_result(crate::commands::speech::get_speech_capabilities(app.clone(), app.state()))))
}

fn prepare_speech_model(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let arg0 = arg(command, args, "payload")?;
    let arg1 = app.clone();
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::commands::speech::prepare_speech_model(arg0, arg1).await) })))
}

fn get_speech_model_status(app: &AppHandle, _window_label: &str, _command: &str, _args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_result(crate::commands::speech::get_speech_model_status(app.clone()))))
}

fn probe_speech_audio_input(app: &AppHandle, _window_label: &str, _command: &str, _args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_value(crate::commands::speech::probe_speech_audio_input(app.state()))))
}

fn speech_audio_devices(app: &AppHandle, _window_label: &str, _command: &str, _args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_value(crate::services::audio_devices::list(app))))
}

fn probe_sherpa_runtime(app: &AppHandle, _window_label: &str, _command: &str, _args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_value(crate::commands::speech::probe_sherpa_runtime(app.clone()))))
}

fn start_speech_session(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let arg0 = arg(command, args, "payload")?;
    let arg1 = app.clone();
    let arg2 = app.state();
    let arg3 = app.state();
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::commands::speech::start_speech_session(arg0, arg1, arg2, arg3).await) })))
}

fn pause_speech_session(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_result(crate::commands::speech::pause_speech_session(arg(command, args, "payload")?, app.clone(), app.state()))))
}

fn resume_speech_session(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_result(crate::commands::speech::resume_speech_session(arg(command, args, "payload")?, app.clone(), app.state()))))
}

fn consume_speech_turn(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_result(crate::commands::speech::consume_speech_turn(arg(command, args, "payload")?, app.clone(), app.state()))))
}

fn speech_remote_audio(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let arg0 = app.clone();
    let arg1 = app.state();
    let arg2 = arg(command, args, "payload")?;
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::commands::remote_speech::speech_remote_audio(arg0, arg1, arg2).await) })))
}

fn speech_remote_audio_cancel(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_result(crate::commands::remote_speech::speech_remote_audio_cancel(app.state(), arg(command, args, "payload")?))))
}

fn stop_speech_session(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let arg0 = arg(command, args, "payload")?;
    let arg1 = app.clone();
    let arg2 = app.state();
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::commands::speech::stop_speech_session(arg0, arg1, arg2).await) })))
}

fn speech_session_state(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_result(crate::commands::speech::speech_session_state(arg(command, args, "payload")?, app.state()))))
}

fn skip_speech_diarization(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_result(crate::commands::speech::skip_speech_diarization(arg(command, args, "payload")?, app.state()))))
}

fn start_audio_monitor(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_result(crate::commands::speech::start_audio_monitor(arg(command, args, "payload")?, app.clone(), app.state(), app.state()))))
}

fn stop_audio_monitor(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_result(crate::commands::speech::stop_audio_monitor(arg(command, args, "payload")?, app.state()))))
}

fn backend_image_models_status(app: &AppHandle, _window_label: &str, _command: &str, _args: &Value) -> Result<Dispatch, Value> {
    let app = app.clone();
    Ok(Dispatch::Pending(Box::pin(async move {
        reply_result(crate::host::async_runtime::spawn_blocking(move || crate::image_generation::backend_image_models_status(app)).await.unwrap_or_else(|_| Err(crate::backend::BackendError::new(crate::backend::BackendErrorCode::Internal, "No se pudo consultar los modelos de imágenes.", true))))
    })))
}

fn backend_download_image_models(app: &AppHandle, _window_label: &str, _command: &str, _args: &Value) -> Result<Dispatch, Value> {
    let app = app.clone();
    Ok(Dispatch::Pending(Box::pin(async move {
        reply_result(crate::host::async_runtime::spawn_blocking(move || crate::image_generation::backend_download_image_models(app)).await.unwrap_or_else(|_| Err(crate::backend::BackendError::new(crate::backend::BackendErrorCode::Internal, "No se pudo consultar los modelos de imágenes.", true))))
    })))
}

fn backend_cancel_image_models_download(app: &AppHandle, _window_label: &str, _command: &str, _args: &Value) -> Result<Dispatch, Value> {
    let app = app.clone();
    Ok(Dispatch::Pending(Box::pin(async move {
        reply_result(crate::host::async_runtime::spawn_blocking(move || crate::image_generation::backend_cancel_image_models_download(app)).await.unwrap_or_else(|_| Err(crate::backend::BackendError::new(crate::backend::BackendErrorCode::Internal, "No se pudo consultar los modelos de imágenes.", true))))
    })))
}

fn backend_delete_image_models(app: &AppHandle, _window_label: &str, _command: &str, _args: &Value) -> Result<Dispatch, Value> {
    let app = app.clone();
    Ok(Dispatch::Pending(Box::pin(async move {
        reply_result(crate::host::async_runtime::spawn_blocking(move || crate::image_generation::backend_delete_image_models(app)).await.unwrap_or_else(|_| Err(crate::backend::BackendError::new(crate::backend::BackendErrorCode::Internal, "No se pudo consultar los modelos de imágenes.", true))))
    })))
}

fn home_dashboard(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let arg0 = app.clone();
    let arg1 = arg(command, args, "payload")?;
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::home::home_dashboard(arg0, arg1).await) })))
}

fn weather_home(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let arg0 = app.clone();
    let arg1 = arg(command, args, "payload")?;
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::weather::weather_home(arg0, arg1).await) })))
}

fn weather_get_location(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_result(crate::weather::weather_get_location(app.clone(), arg(command, args, "payload")?))))
}

fn weather_search_locations(_app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let arg0 = arg(command, args, "payload")?;
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::weather::weather_search_locations(arg0).await) })))
}

fn weather_set_location(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_result(crate::weather::weather_set_location(app.clone(), arg(command, args, "payload")?))))
}

fn meeting_media_begin(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_result(crate::meeting_media::meeting_media_begin(app.clone(), arg(command, args, "payload")?))))
}

fn meeting_media_chunk(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_result(crate::meeting_media::meeting_media_chunk(app.clone(), arg(command, args, "payload")?))))
}

fn meeting_media_finish(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let arg0 = app.clone();
    let arg1 = arg(command, args, "payload")?;
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::meeting_media::meeting_media_finish(arg0, arg1).await) })))
}

fn meeting_media_discard(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_result(crate::meeting_media::meeting_media_discard(app.clone(), arg(command, args, "payload")?))))
}

fn meeting_start_file_session(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let arg0 = app.clone();
    let arg1 = arg(command, args, "payload")?;
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::meeting_media::meeting_start_file_session(arg0, arg1).await) })))
}

fn meeting_snapshot(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_result(crate::meeting::meeting_snapshot(app.clone(), arg(command, args, "payload")?))))
}

fn meeting_context(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_result(crate::meeting::meeting_context(app.clone(), arg(command, args, "payload")?))))
}

fn meeting_notes_text(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_result(crate::meeting::meeting_notes_text(app.clone(), arg(command, args, "payload")?))))
}

fn meeting_save_ai_context_choice(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_result(crate::meeting::meeting_save_ai_context_choice(app.clone(), arg(command, args, "payload")?))))
}

fn meeting_discard(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_result(crate::meeting::meeting_discard(app.clone(), arg(command, args, "payload")?))))
}

fn meeting_add_mark(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_result(crate::meeting::meeting_add_mark(app.clone(), arg(command, args, "payload")?))))
}

fn meeting_remove_mark(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_result(crate::meeting::meeting_remove_mark(app.clone(), arg(command, args, "payload")?))))
}

fn meeting_set_notes(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_result(crate::meeting::meeting_set_notes(app.clone(), arg(command, args, "payload")?))))
}

fn meeting_set_live_answers(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_result(crate::meeting::meeting_set_live_answers(app.clone(), arg(command, args, "payload")?))))
}

fn meeting_set_ai_notes(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_result(crate::meeting::meeting_set_ai_notes(app.clone(), arg(command, args, "payload")?))))
}

fn meeting_call_notes_agent(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_result(crate::meeting::meeting_call_notes_agent(app.clone(), arg(command, args, "payload")?))))
}

fn meeting_ai_context_options(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let arg0 = app.clone();
    let arg1 = arg(command, args, "payload")?;
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::meeting::meeting_ai_context_options(arg0, arg1).await) })))
}

fn meeting_regenerate_answer(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_result(crate::meeting::meeting_regenerate_answer(app.clone(), arg(command, args, "payload")?))))
}

fn meeting_pin_answer(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_result(crate::meeting::meeting_pin_answer(app.clone(), arg(command, args, "payload")?))))
}

fn meeting_rename_speaker(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_result(crate::meeting::meeting_rename_speaker(app.clone(), arg(command, args, "payload")?))))
}

fn meeting_merge_speakers(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_result(crate::meeting::meeting_merge_speakers(app.clone(), arg(command, args, "payload")?))))
}

fn meeting_generate_insights(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let arg0 = app.clone();
    let arg1 = arg(command, args, "payload")?;
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::meeting::meeting_generate_insights(arg0, arg1).await) })))
}

fn meeting_history(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let arg0 = app.clone();
    let arg1 = arg(command, args, "payload")?;
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::meeting_history::meeting_history(arg0, arg1).await) })))
}

fn meeting_open_saved(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let arg0 = app.clone();
    let arg1 = arg(command, args, "payload")?;
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::meeting_history::meeting_open_saved(arg0, arg1).await) })))
}

fn meeting_save_note(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let arg0 = app.clone();
    let arg1 = arg(command, args, "payload")?;
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::meeting::meeting_save_note(arg0, arg1).await) })))
}

fn meeting_export(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let arg0 = app.clone();
    let arg1 = arg(command, args, "payload")?;
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::meeting::meeting_export(arg0, arg1).await) })))
}

fn meeting_task_boards(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let arg0 = app.clone();
    let arg1 = arg(command, args, "payload")?;
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::meeting::meeting_task_boards(arg0, arg1).await) })))
}

fn meeting_send_tasks(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let arg0 = app.clone();
    let arg1 = arg(command, args, "payload")?;
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::meeting::meeting_send_tasks(arg0, arg1).await) })))
}

fn cancel_speech_session(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_result(crate::commands::speech::cancel_speech_session(arg(command, args, "payload")?, app.clone(), app.state()))))
}

fn get_qwen3_tts_status(app: &AppHandle, _window_label: &str, _command: &str, _args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_value(crate::commands::qwen3_tts::get_qwen3_tts_status(app.state()))))
}

fn reload_qwen3_tts(app: &AppHandle, _window_label: &str, _command: &str, _args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_result(crate::commands::qwen3_tts::reload_qwen3_tts(app.state()))))
}

fn synthesize_qwen3_tts_speech(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let arg0 = app.clone();
    let arg1 = arg(command, args, "input")?;
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::commands::qwen3_tts::synthesize_qwen3_tts_speech(arg0, arg1).await) })))
}

fn qwen3_tts_speech_plan(_app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_value(crate::commands::qwen3_tts::qwen3_tts_speech_plan(arg(command, args, "markdown")?))))
}

fn prepare_qwen3_tts(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let arg0 = app.clone();
    let arg1 = arg(command, args, "input")?;
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::commands::qwen3_tts::prepare_qwen3_tts(arg0, arg1).await) })))
}

fn check_telegram_bot(_app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let arg0 = arg(command, args, "payload")?;
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::commands::telegram::check_telegram_bot(arg0).await) })))
}

fn revoke_library_binding(app: &AppHandle, window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    if let Some(refused) = refuse_host_libraries_change(window_label) {
        return Ok(refused);
    }
    Ok(Dispatch::Ready(reply_result(crate::library_registry::revoke_library_binding(app.clone(), arg(command, args, "libraryId")?, app.state(), app.state(), app.state()))))
}

#[cfg(all(feature = "bluetooth", not(any(target_os = "android", target_os = "ios"))))]
fn coldpass_bluetooth_status(app: &AppHandle, _window_label: &str, _command: &str, _args: &Value) -> Result<Dispatch, Value> {
    let arg0 = app.state();
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::commands::bluetooth::coldpass_bluetooth_status(arg0).await) })))
}

#[cfg(any(target_os = "android", target_os = "ios", not(feature = "bluetooth")))]
fn coldpass_bluetooth_status(_app: &AppHandle, _window_label: &str, _command: &str, _args: &Value) -> Result<Dispatch, Value> {

    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::commands::bluetooth::coldpass_bluetooth_status().await) })))
}

#[cfg(all(feature = "bluetooth", not(any(target_os = "android", target_os = "ios"))))]
fn coldpass_bluetooth_connect(app: &AppHandle, _window_label: &str, _command: &str, _args: &Value) -> Result<Dispatch, Value> {
    let arg0 = app.state();
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::commands::bluetooth::coldpass_bluetooth_connect(arg0).await) })))
}

#[cfg(any(target_os = "android", target_os = "ios", not(feature = "bluetooth")))]
fn coldpass_bluetooth_connect(_app: &AppHandle, _window_label: &str, _command: &str, _args: &Value) -> Result<Dispatch, Value> {

    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::commands::bluetooth::coldpass_bluetooth_connect().await) })))
}

#[cfg(all(feature = "bluetooth", not(any(target_os = "android", target_os = "ios"))))]
fn coldpass_bluetooth_submit_pin(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let arg0 = arg(command, args, "payload")?;
    let arg1 = app.state();
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::commands::bluetooth::coldpass_bluetooth_submit_pin(arg0, arg1).await) })))
}

#[cfg(any(target_os = "android", target_os = "ios", not(feature = "bluetooth")))]
fn coldpass_bluetooth_submit_pin(_app: &AppHandle, _window_label: &str, _command: &str, _args: &Value) -> Result<Dispatch, Value> {

    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::commands::bluetooth::coldpass_bluetooth_submit_pin().await) })))
}

#[cfg(all(feature = "bluetooth", not(any(target_os = "android", target_os = "ios"))))]
fn coldpass_bluetooth_authenticate(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let arg0 = arg(command, args, "payload")?;
    let arg1 = app.state();
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::commands::bluetooth::coldpass_bluetooth_authenticate(arg0, arg1).await) })))
}

#[cfg(any(target_os = "android", target_os = "ios", not(feature = "bluetooth")))]
fn coldpass_bluetooth_authenticate(_app: &AppHandle, _window_label: &str, _command: &str, _args: &Value) -> Result<Dispatch, Value> {

    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::commands::bluetooth::coldpass_bluetooth_authenticate().await) })))
}

#[cfg(all(feature = "bluetooth", not(any(target_os = "android", target_os = "ios"))))]
fn coldpass_bluetooth_send_message(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let arg0 = arg(command, args, "payload")?;
    let arg1 = app.state();
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::commands::bluetooth::coldpass_bluetooth_send_message(arg0, arg1).await) })))
}

#[cfg(any(target_os = "android", target_os = "ios", not(feature = "bluetooth")))]
fn coldpass_bluetooth_send_message(_app: &AppHandle, _window_label: &str, _command: &str, _args: &Value) -> Result<Dispatch, Value> {

    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::commands::bluetooth::coldpass_bluetooth_send_message().await) })))
}

#[cfg(all(feature = "bluetooth", not(any(target_os = "android", target_os = "ios"))))]
fn coldpass_bluetooth_disconnect(app: &AppHandle, _window_label: &str, _command: &str, _args: &Value) -> Result<Dispatch, Value> {
    let arg0 = app.state();
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::commands::bluetooth::coldpass_bluetooth_disconnect(arg0).await) })))
}

#[cfg(any(target_os = "android", target_os = "ios", not(feature = "bluetooth")))]
fn coldpass_bluetooth_disconnect(_app: &AppHandle, _window_label: &str, _command: &str, _args: &Value) -> Result<Dispatch, Value> {

    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::commands::bluetooth::coldpass_bluetooth_disconnect().await) })))
}

fn get_task_manager_publication_url(app: &AppHandle, _window_label: &str, _command: &str, _args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_result(crate::task_manager_publication::get_task_manager_publication_url(app.state()))))
}

fn get_task_manager_publication_status(app: &AppHandle, _window_label: &str, _command: &str, _args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_result(crate::task_manager_publication::get_task_manager_publication_status(app.state()))))
}

fn open_task_manager_publication(app: &AppHandle, _window_label: &str, _command: &str, _args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_result(crate::task_manager_publication::open_task_manager_publication(app.state()))))
}

fn stop_task_manager_publication(app: &AppHandle, _window_label: &str, _command: &str, _args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_result(crate::task_manager_publication::stop_task_manager_publication(app.state()))))
}

fn begin_task_manager_publication_batch(app: &AppHandle, _window_label: &str, _command: &str, _args: &Value) -> Result<Dispatch, Value> {
    let arg0 = app.state();
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::task_manager_publication::begin_task_manager_publication_batch(arg0).await) })))
}

fn end_task_manager_publication_batch(app: &AppHandle, _window_label: &str, _command: &str, _args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_result(crate::task_manager_publication::end_task_manager_publication_batch(app.state()))))
}

fn ai_chat_send(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let arg0 = app.clone();
    let arg1 = arg(command, args, "payload")?;
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::ai_chat::ai_chat_send(arg0, arg1).await) })))
}

fn ai_chat_answer(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_result(crate::ai_chat::ai_chat_answer(app.state(), arg(command, args, "payload")?))))
}

fn ai_chat_cancel(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let arg0 = app.clone();
    let arg1 = arg(command, args, "payload")?;
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::ai_chat::ai_chat_cancel(arg0, arg1).await) })))
}

fn ai_chat_interject(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let arg0 = app.clone();
    let arg1 = arg(command, args, "payload")?;
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::ai_chat::ai_chat_interject(arg0, arg1).await) })))
}

fn ai_check_health(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let arg0 = app.clone();
    let arg1 = arg(command, args, "payload")?;
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::ai_tasks::ai_check_health(arg0, arg1).await) })))
}

fn ai_list_models(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let arg0 = app.clone();
    let arg1 = arg(command, args, "payload")?;
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::ai_tasks::ai_list_models(arg0, arg1).await) })))
}

fn ai_resolve_model(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let arg0 = app.clone();
    let arg1 = arg(command, args, "payload")?;
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::ai_tasks::ai_resolve_model(arg0, arg1).await) })))
}

fn ai_recognize_inkmath(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let arg0 = app.clone();
    let arg1 = arg(command, args, "payload")?;
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::ai_tasks::ai_recognize_inkmath(arg0, arg1).await) })))
}

fn markdown_blocks_resolve(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let arg0 = app.clone();
    let arg1 = arg(command, args, "payload")?;
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::gitbook_blocks::markdown_blocks_resolve(arg0, arg1).await) })))
}

fn backend_preview_editor_preferences(_app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let payload: Value = arg(command, args, "payload")?;
    Ok(Dispatch::Ready(reply_value(crate::device_preferences::editor_preferences_view(payload.get("preferences").unwrap_or(&Value::Null)))))
}

fn markdown_ink_load(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let (app, payload) = (app.clone(), arg(command, args, "payload")?);
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::note_ink::markdown_ink_load(app, payload).await) })))
}

fn markdown_ink_add(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let (app, payload) = (app.clone(), arg(command, args, "payload")?);
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::note_ink::markdown_ink_add(app, payload).await) })))
}

fn markdown_ink_remove(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let (app, payload) = (app.clone(), arg(command, args, "payload")?);
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::note_ink::markdown_ink_remove(app, payload).await) })))
}

fn markdown_ink_restore(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let (app, payload) = (app.clone(), arg(command, args, "payload")?);
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::note_ink::markdown_ink_restore(app, payload).await) })))
}

fn markdown_ink_replace(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let (app, payload) = (app.clone(), arg(command, args, "payload")?);
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::note_ink::markdown_ink_replace(app, payload).await) })))
}

fn markdown_ink_select(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let (app, payload) = (app.clone(), arg(command, args, "payload")?);
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::note_ink::markdown_ink_select(app, payload).await) })))
}

fn markdown_ink_move(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let (app, payload) = (app.clone(), arg(command, args, "payload")?);
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::note_ink::markdown_ink_move(app, payload).await) })))
}

fn markdown_ink_erase(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let (app, payload) = (app.clone(), arg(command, args, "payload")?);
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::note_ink::markdown_ink_erase(app, payload).await) })))
}

fn markdown_ink_convert(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let (app, payload) = (app.clone(), arg(command, args, "payload")?);
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::note_ink::markdown_ink_convert(app, payload).await) })))
}

fn markdown_export_diagram(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let (app, payload) = (app.clone(), arg(command, args, "payload")?);
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::note_ink::markdown_export_diagram(app, payload).await) })))
}

fn markdown_note_preview(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let (app, payload) = (app.clone(), arg(command, args, "payload")?);
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::note_preview::markdown_note_preview(app, payload).await) })))
}

fn markdown_set_page_mode(_app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_result(crate::note_page_mode::markdown_set_page_mode(arg(command, args, "payload")?))))
}

fn backend_mail_accounts(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let arg0 = app.clone();
    let arg1 = arg(command, args, "payload")?;
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::mail_accounts::backend_mail_accounts(arg0, arg1).await) })))
}

fn backend_connect_mail_account(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let arg0 = app.clone();
    let arg1 = arg(command, args, "payload")?;
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::mail_accounts::backend_connect_mail_account(arg0, arg1).await) })))
}

fn backend_cancel_mail_account_connection(_app: &AppHandle, _window_label: &str, _command: &str, _args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_result(crate::mail_accounts::backend_cancel_mail_account_connection())))
}

fn backend_disconnect_mail_account(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let arg0 = app.clone();
    let arg1 = arg(command, args, "payload")?;
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::mail_accounts::backend_disconnect_mail_account(arg0, arg1).await) })))
}

fn backend_set_mail_account_type(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let arg0 = app.clone();
    let arg1 = arg(command, args, "payload")?;
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::mail_accounts::backend_set_mail_account_type(arg0, arg1).await) })))
}

fn backend_save_google_cloud_credentials(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let arg0 = app.clone();
    let arg1 = arg(command, args, "payload")?;
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::mail_accounts::backend_save_google_cloud_credentials(arg0, arg1).await) })))
}

fn webhook_view(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let arg0 = app.clone();
    let arg1 = arg(command, args, "payload")?;
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::webhooks::webhook_view(arg0, arg1).await) })))
}

fn webhook_save_server(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let arg0 = app.clone();
    let arg1 = arg(command, args, "payload")?;
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::webhooks::webhook_save_server(arg0, arg1).await) })))
}

fn webhook_save_action(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let arg0 = app.clone();
    let arg1 = arg(command, args, "payload")?;
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::webhooks::webhook_save_action(arg0, arg1).await) })))
}

fn webhook_new_token(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let arg0 = app.clone();
    let arg1 = arg(command, args, "payload")?;
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::webhooks::webhook_new_token(arg0, arg1).await) })))
}

fn webhook_delete_action(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let arg0 = app.clone();
    let arg1 = arg(command, args, "payload")?;
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::webhooks::webhook_delete_action(arg0, arg1).await) })))
}

fn whatsapp_view(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let arg0 = app.clone();
    let arg1 = arg(command, args, "payload")?;
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::whatsapp::whatsapp_view(arg0, arg1).await) })))
}

fn whatsapp_connect(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let arg0 = app.clone();
    let arg1 = arg(command, args, "payload")?;
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::whatsapp::whatsapp_connect(arg0, arg1).await) })))
}

fn whatsapp_disconnect(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let arg0 = app.clone();
    let arg1 = arg(command, args, "payload")?;
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::whatsapp::whatsapp_disconnect(arg0, arg1).await) })))
}

fn whatsapp_set_chat(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let arg0 = app.clone();
    let arg1 = arg(command, args, "payload")?;
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::whatsapp::whatsapp_set_chat(arg0, arg1).await) })))
}

fn backend_check_google_cloud_credentials(_app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let arg0 = arg(command, args, "payload")?;
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::mail_accounts::backend_check_google_cloud_credentials(arg0).await) })))
}

fn backend_remove_google_cloud_credentials(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let arg0 = app.clone();
    let arg1 = arg(command, args, "payload")?;
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::mail_accounts::backend_remove_google_cloud_credentials(arg0, arg1).await) })))
}

fn backend_import_google_cloud_json(app: &AppHandle, _window_label: &str, _command: &str, _args: &Value) -> Result<Dispatch, Value> {
    let arg0 = app.clone();
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::mail_accounts::backend_import_google_cloud_json(arg0).await) })))
}

fn chat_agents_catalog(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let arg0 = app.clone();
    let arg1 = arg(command, args, "payload")?;
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::chat_agents::chat_agents_catalog(arg0, arg1).await) })))
}

fn library_open(app: &AppHandle, window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let arg0 = Window::new(app.clone(), window_label);
    let arg1 = arg(command, args, "payload")?;
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::library_session::library_open(arg0, arg1).await) })))
}

fn library_refresh(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let arg0 = app.clone();
    let arg1 = arg(command, args, "payload")?;
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::library_session::library_refresh(arg0, arg1).await) })))
}

fn library_read_directory(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let arg0 = app.clone();
    let arg1 = arg(command, args, "payload")?;
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::library_session::library_read_directory(arg0, arg1).await) })))
}

fn library_read_document(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_value(crate::library_session::library_read_document(app.clone(), arg(command, args, "payload")?))))
}

fn library_write_document(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_value(crate::library_session::library_save_document(app.clone(), arg(command, args, "payload")?))))
}

fn library_mutate_entry(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_value(crate::library_session::library_mutate_entry(app.clone(), arg(command, args, "payload")?))))
}

fn library_set_folder_context(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_value(crate::folder_contexts::library_set_folder_context(app.clone(), arg(command, args, "payload")?))))
}

fn public_spaces_view(app: &AppHandle, _window_label: &str, _command: &str, _args: &Value) -> Result<Dispatch, Value> {
    let app = app.clone();
    Ok(Dispatch::Pending(Box::pin(async move {
        reply_result(
            crate::host::async_runtime::spawn_blocking(move || crate::public_spaces::public_spaces_view(app))
                .await
                .map_err(|_| crate::backend::BackendError::new(crate::backend::BackendErrorCode::Internal, "No se pudieron leer los espacios públicos.", true)),
        )
    })))
}

fn public_spaces_save(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let (app, payload) = (app.clone(), arg(command, args, "payload")?);
    Ok(Dispatch::Pending(Box::pin(async move {
        reply_result(
            crate::host::async_runtime::spawn_blocking(move || crate::public_spaces::public_spaces_save(app, payload))
                .await
                .unwrap_or_else(|_| Err(crate::backend::BackendError::new(crate::backend::BackendErrorCode::Internal, "No se pudieron guardar los espacios públicos.", true))),
        )
    })))
}

fn public_space_candidates(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let (app, payload) = (app.clone(), arg(command, args, "payload")?);
    Ok(Dispatch::Pending(Box::pin(async move {
        reply_result(
            crate::host::async_runtime::spawn_blocking(move || crate::public_spaces::public_space_candidates(app, payload))
                .await
                .unwrap_or_else(|_| Err(crate::backend::BackendError::new(crate::backend::BackendErrorCode::Internal, "No se pudo leer la biblioteca.", true))),
        )
    })))
}

fn public_space_open(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_result(crate::public_spaces::public_space_open(app.clone(), arg(command, args, "payload")?))))
}

fn library_pick_directory(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let arg0 = app.clone();
    let arg1 = arg(command, args, "payload")?;
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::library_session::library_pick_directory(arg0, arg1).await) })))
}

fn library_list_files(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let arg0 = app.clone();
    let arg1 = arg(command, args, "payload")?;
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::library_session::library_list_files(arg0, arg1).await) })))
}

fn library_list_folders(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let arg0 = app.clone();
    let arg1 = arg(command, args, "payload")?;
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::library_session::library_list_folders(arg0, arg1).await) })))
}

fn task_manager_board_view(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_result(crate::task_manager_commands::task_manager_board_view(app.clone(), arg(command, args, "payload")?, app.state(), app.state()))))
}

fn task_manager_board_execute(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let arg0 = app.clone();
    let arg1 = arg(command, args, "payload")?;
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::task_manager_commands::task_manager_board_execute(arg0, arg1).await) })))
}

fn task_manager_board_plan_preview(_app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_result(crate::task_manager_commands::task_manager_board_plan_preview(arg(command, args, "payload")?))))
}

fn task_manager_management_view(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let arg0 = app.clone();
    let arg1 = arg(command, args, "payload")?;
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::task_manager_commands::task_manager_management_view(arg0, arg1).await) })))
}

fn task_manager_pomodoro(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let arg0 = app.clone();
    let arg1 = arg(command, args, "payload")?;
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::task_manager_commands::task_manager_pomodoro(arg0, arg1).await) })))
}

fn task_manager_delete_pomodoro(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_result(crate::task_manager_commands::task_manager_delete_pomodoro(app.clone(), arg(command, args, "payload")?, app.state(), app.state()))))
}

fn task_manager_task_detail(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_result(crate::task_manager_commands::task_manager_task_detail(app.clone(), arg(command, args, "payload")?, app.state(), app.state()))))
}

fn task_manager_read_ticket_source(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_result(crate::task_manager_commands::task_manager_read_ticket_source(app.clone(), arg(command, args, "payload")?, app.state(), app.state()))))
}

fn task_manager_write_ticket_source(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_result(crate::task_manager_commands::task_manager_write_ticket_source(app.clone(), arg(command, args, "payload")?, app.state(), app.state()))))
}

fn connection_settings(app: &AppHandle, _window_label: &str, _command: &str, _args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_value(crate::connection::connection_settings(app))))
}

fn save_connection_settings(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_result(crate::connection::save_connection_settings(app, arg(command, args, "payload")?))))
}

fn test_host_connection(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    let arg0 = app.clone();
    let arg1 = arg::<Option<crate::connection::TestHostPayload>>(command, args, "payload")?.unwrap_or_default();
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::connection::test_host_connection(arg0, arg1).await) })))
}

fn client_open(app: &AppHandle, _window_label: &str, _command: &str, _args: &Value) -> Result<Dispatch, Value> {
    let app = app.clone();
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::connection::client_open(app).await) })))
}

fn sync_copy_now(app: &AppHandle, _window_label: &str, _command: &str, _args: &Value) -> Result<Dispatch, Value> {
    let app = app.clone();
    Ok(Dispatch::Pending(Box::pin(async move { reply_result(crate::host_mirror::sync(&app).await) })))
}

fn pick_copy_folder(app: &AppHandle, _window_label: &str, _command: &str, _args: &Value) -> Result<Dispatch, Value> {
    let app = app.clone();
    Ok(Dispatch::Pending(Box::pin(async move {
        let picked = crate::host_mirror::pick_copy_folder(app.clone()).await;
        reply_result(picked.map(|()| crate::connection::connection_settings(&app)))
    })))
}

fn host_sync_manifest(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_result(crate::host_sync::host_sync_manifest(app, arg(command, args, "payload")?))))
}

fn host_sync_database(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_result(crate::host_sync::host_sync_database(app, arg(command, args, "payload")?))))
}

fn host_sync_database_changes(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_result(crate::host_sync::host_sync_database_changes(app, arg(command, args, "payload")?))))
}

fn host_sync_database_push(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_result(crate::host_sync::host_sync_database_push(app, arg(command, args, "payload")?))))
}

fn host_sync_write(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_result(crate::host_sync::host_sync_write(app, arg(command, args, "payload")?))))
}

fn host_sync_delete(app: &AppHandle, _window_label: &str, command: &str, args: &Value) -> Result<Dispatch, Value> {
    Ok(Dispatch::Ready(reply_result(crate::host_sync::host_sync_delete(app, arg(command, args, "payload")?))))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::{AppPaths, HostPorts};

    fn ready_error(dispatch: Dispatch) -> Value {
        match dispatch {
            Dispatch::Ready(Err(error)) => error,
            _ => panic!("expected an immediate error"),
        }
    }

    #[test]
    fn every_listed_command_has_a_route() {
        for command in COMMAND_NAMES {
            assert!(route(command).is_some(), "{command}");
        }
        for command in LOCAL_ONLY_COMMANDS {
            assert!(COMMAND_NAMES.contains(command), "{command}");
            assert!(!is_remote_command(command), "{command}");
        }
        assert!(is_remote_command("task_manager_board_view"));
        assert!(!is_remote_command("window_control"));
    }

    #[test]
    fn publications_reach_only_their_commands_with_the_published_user() {
        let app = crate::create_app(AppPaths::default(), HostPorts::default());
        let scope = PublishedScope {
            library_id: "library-1".into(),
            library_user_id: "user-1".into(),
            board_ids: vec!["board-1".into()],
        };
        for command in PUBLISHED_COMMANDS {
            assert!(COMMAND_NAMES.contains(command), "{command}");
        }
        assert!(!is_published_command("task_manager_apply_mutation"));
        assert!(dispatch_published(&app, &scope, "backend_library_catalog", &serde_json::json!({})).is_none());
        assert!(dispatch_published(&app, &scope, "write_library_file", &serde_json::json!({})).is_none());
        // An unregistered library is refused by the use case, not by the host.
        assert!(dispatch_published(&app, &scope, "task_manager_board_view", &serde_json::json!({}))
            .expect("published command")
            .is_err());
    }

    #[test]
    fn app_invoke_rejects_unknown_commands_and_malformed_bodies() {
        let app = crate::create_app(AppPaths::default(), HostPorts::default());
        let unknown = dispatch_app_invoke(&app, "main", &serde_json::json!({ "command": "nope" }));
        assert_eq!(ready_error(unknown), Value::String("command nope not found".into()));
        let malformed = dispatch_app_invoke(&app, "main", &serde_json::json!({ "args": {} }));
        assert!(ready_error(malformed).as_str().unwrap().starts_with("invalid app_invoke body"));
    }

    #[test]
    fn app_invoke_reports_missing_arguments_like_tauri() {
        let app = crate::create_app(AppPaths::default(), HostPorts::default());
        let body = serde_json::json!({ "command": "finance_overview" });
        assert_eq!(
            ready_error(dispatch_app_invoke(&app, "main", &body)),
            Value::String("command finance_overview missing required key payload".into())
        );
        let invalid = serde_json::json!({ "command": "finance_overview", "args": { "payload": "x" } });
        assert!(ready_error(dispatch_app_invoke(&app, "main", &invalid))
            .as_str()
            .unwrap()
            .starts_with("invalid args `payload` for command `finance_overview`"));
    }
}
