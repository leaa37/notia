//! Tauri host of Notia. The application lives in `notia-app`; this crate only
//! builds it with the paths and platform ports of the window, forwards the
//! single `app_invoke` command of the WebView to its registry, and keeps what belongs to the
//! window itself (controls, dragging, exit, Windows tray) and the native
//! Android plugins.

use std::sync::Arc;

use notia_app::host::dialog::{DialogFilter, DialogPort, FilePath, Url};
use notia_app::host::plugin::{AndroidPluginRegistrar, PluginApi};
use notia_app::host::{Asset, AssetSource, AppContext, AppPaths, EventSink, HostPorts};
use notia_app::registry::{dispatch_app_invoke, Dispatch, APP_INVOKE};
use serde::Deserialize;
use tauri::ipc::{InvokeBody, InvokeError};
use tauri::{Emitter, Manager, Wry};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WindowControlPayload {
    action: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NotiaLogPayload {
    level: String,
    module: String,
    message: String,
    #[serde(default)]
    data: Option<String>,
}

#[tauri::command]
fn notia_log(payload: NotiaLogPayload) {
    let log_level = match payload.level.as_str() {
        "error" => log::Level::Error,
        "warn" => log::Level::Warn,
        "info" => log::Level::Info,
        "perf" => log::Level::Info,
        _ => log::Level::Debug,
    };
    let data_suffix = payload.data.unwrap_or_default();
    if payload.module == "telegram-ai" {
        log::log!(
            target: "notia_telegram_ai",
            log_level,
            "[notia:js:telegram-ai] {} {}",
            payload.message,
            data_suffix
        );
    } else {
        log::log!(
            log_level,
            "[notia:js:{}] {} {}",
            payload.module,
            payload.message,
            data_suffix
        );
    }
}

#[tauri::command]
#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn window_control(window: tauri::Window, payload: WindowControlPayload) {
    match payload.action.as_str() {
        "minimize" => {
            let _ = window.minimize();
        }
        "maximize" => {
            if window.is_maximized().unwrap_or(false) {
                let _ = window.unmaximize();
            } else {
                let _ = window.maximize();
            }
        }
        "fullscreen" => {
            let is_fullscreen = window.is_fullscreen().unwrap_or(false);
            let _ = window.set_fullscreen(!is_fullscreen);
        }
        "close" => {
            let _ = window.close();
        }
        _ => {}
    }
}

#[tauri::command]
#[cfg(any(target_os = "android", target_os = "ios"))]
fn window_control(_window: tauri::Window, payload: WindowControlPayload) {
    let _ = payload.action;
}

#[tauri::command]
fn exit_application(app: tauri::AppHandle) {
    app.exit(0);
}

#[tauri::command]
#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn start_window_dragging(window: tauri::Window) {
    let _ = window.start_dragging();
}

#[tauri::command]
#[cfg(any(target_os = "android", target_os = "ios"))]
fn start_window_dragging(_window: tauri::Window) {}

#[tauri::command]
#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn start_window_dragging_with_restore(window: tauri::Window) {
    if window.is_fullscreen().unwrap_or(false) {
        let _ = window.set_fullscreen(false);
    }
    if window.is_maximized().unwrap_or(false) {
        let _ = window.unmaximize();
    }
    let _ = window.start_dragging();
}

#[tauri::command]
#[cfg(any(target_os = "android", target_os = "ios"))]
fn start_window_dragging_with_restore(_window: tauri::Window) {}

/// Delivers the events of the application to the WebView.
struct WebviewEvents(tauri::AppHandle);

impl EventSink for WebviewEvents {
    fn emit(&self, event: &str, payload: serde_json::Value) -> Result<(), String> {
        self.0.emit(event, payload).map_err(|error| error.to_string())
    }
}

/// Serves the interface bundle embedded in the executable.
struct EmbeddedAssets(tauri::AppHandle);

impl AssetSource for EmbeddedAssets {
    fn get(&self, path: &str) -> Option<Asset> {
        self.0
            .asset_resolver()
            .get(path.to_string())
            .map(|asset| Asset::new(asset.bytes().to_vec(), asset.mime_type().to_string()))
    }
}

/// Native pickers of the dialog plugin.
struct NativeDialogs(tauri::AppHandle);

impl NativeDialogs {
    fn builder(&self, title: Option<&str>) -> tauri_plugin_dialog::FileDialogBuilder<Wry> {
        use tauri_plugin_dialog::DialogExt;
        let builder = self.0.dialog().file();
        match title {
            Some(title) => builder.set_title(title),
            None => builder,
        }
    }
}

fn picked(path: tauri_plugin_dialog::FilePath) -> FilePath {
    match path {
        tauri_plugin_dialog::FilePath::Path(path) => FilePath::Path(path),
        tauri_plugin_dialog::FilePath::Url(url) => FilePath::Url(Url::new(url.as_str())),
    }
}

impl DialogPort for NativeDialogs {
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    fn pick_folder(&self, title: Option<&str>) -> Option<FilePath> {
        self.builder(title).blocking_pick_folder().map(picked)
    }

    /// Android picks library folders through the SAF plugin, not the dialog.
    #[cfg(any(target_os = "android", target_os = "ios"))]
    fn pick_folder(&self, _title: Option<&str>) -> Option<FilePath> {
        None
    }

    fn pick_file(&self, title: Option<&str>, filters: &[DialogFilter]) -> Option<FilePath> {
        let mut builder = self.builder(title);
        for filter in filters {
            let extensions: Vec<&str> = filter.extensions.iter().map(String::as_str).collect();
            builder = builder.add_filter(filter.name.clone(), &extensions);
        }
        builder.blocking_pick_file().map(picked)
    }
}

/// A Kotlin plugin registered through Tauri.
#[cfg(target_os = "android")]
struct NativePlugin(tauri::plugin::PluginHandle<Wry>);

#[cfg(target_os = "android")]
impl notia_app::host::plugin::MobilePlugin for NativePlugin {
    fn run(&self, command: &str, payload: serde_json::Value) -> Result<serde_json::Value, String> {
        self.0
            .run_mobile_plugin::<serde_json::Value>(command, payload)
            .map_err(|error| error.to_string())
    }

    fn run_async(&self, command: String, payload: serde_json::Value) -> notia_app::host::plugin::MobileFuture {
        let handle = self.0.clone();
        Box::pin(async move {
            handle
                .run_mobile_plugin_async::<serde_json::Value>(&command, payload)
                .await
                .map_err(|error| error.to_string())
        })
    }
}

#[cfg(target_os = "android")]
fn android_registrar(api: tauri::plugin::PluginApi<Wry, ()>) -> Option<AndroidPluginRegistrar> {
    Some(Box::new(move |package: &str, class: &str| {
        api.register_android_plugin(package, class)
            .map(|handle| notia_app::host::plugin::PluginHandle::new(Arc::new(NativePlugin(handle))))
            .map_err(|error| error.to_string())
    }))
}

#[cfg(not(target_os = "android"))]
fn android_registrar(_api: tauri::plugin::PluginApi<Wry, ()>) -> Option<AndroidPluginRegistrar> {
    None
}

/// Lets native plugins (Android AI stream) push messages to the application.
fn install_channel_factory() {
    notia_app::host::ipc::set_channel_factory(Box::new(|callback| {
        let channel = tauri::ipc::Channel::<serde_json::Value>::new(move |body| {
            let body = match body {
                tauri::ipc::InvokeResponseBody::Json(text) => notia_app::host::ipc::InvokeResponseBody::Json(text),
                tauri::ipc::InvokeResponseBody::Raw(bytes) => notia_app::host::ipc::InvokeResponseBody::Raw(bytes),
            };
            if let Err(error) = callback(body) {
                log::warn!("[notia:ipc] channel message rejected: {error}");
            }
            Ok(())
        });
        let identity = serde_json::to_value(&channel).unwrap_or(serde_json::Value::Null);
        let guard: Arc<dyn std::any::Any + Send + Sync> = Arc::new(channel);
        (identity, guard)
    }));
}

/// Takes the data folder for this process. If the headless server (or
/// another window) already uses it, tells the person and closes: two
/// processes over the same databases would corrupt them.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn hold_data_dir(app: &tauri::AppHandle, data_dir: &std::path::Path) {
    match notia_app::host::DataDirLock::acquire(data_dir) {
        Ok(lock) => {
            app.manage(lock);
        }
        Err(message) => {
            use tauri_plugin_dialog::{DialogExt, MessageDialogKind};
            log::error!("[notia] data folder unavailable: {message}");
            app.dialog()
                .message(message)
                .title("Notia ya está en uso")
                .kind(MessageDialogKind::Error)
                .blocking_show();
            std::process::exit(1);
        }
    }
}

/// First plugin: builds the application with the paths and ports of Tauri.
fn host_plugin() -> tauri::plugin::TauriPlugin<Wry> {
    tauri::plugin::Builder::new("notia-host")
        .setup(|app, _api| {
            let data_dir = app.path().app_data_dir().ok();
            #[cfg(not(any(target_os = "android", target_os = "ios")))]
            if let Some(data_dir) = &data_dir {
                hold_data_dir(app, data_dir);
            }
            let paths = AppPaths::new(data_dir, app.path().resource_dir().ok());
            let ports = HostPorts {
                events: Some(Arc::new(WebviewEvents(app.clone()))),
                assets: Some(Arc::new(EmbeddedAssets(app.clone()))),
                dialogs: Some(Arc::new(NativeDialogs(app.clone()))),
            };
            let context = notia_app::create_app(paths, ports);
            notia_app::start_device_services(&context);
            app.manage(context);
            Ok(())
        })
        .build()
}

/// Runs a startup hook of the application as a Tauri plugin with its name,
/// so its Kotlin plugin is registered under the name the WebView expects.
fn startup_plugin(hook: notia_app::host::plugin::TauriPlugin) -> tauri::plugin::TauriPlugin<Wry> {
    tauri::plugin::Builder::new(hook.name())
        .setup(move |app, api| {
            let context = app.state::<AppContext>().inner().clone();
            hook.run_setup(&context, PluginApi::new(android_registrar(api)))
        })
        .build()
}

type Command = Box<dyn FnOnce() + Send>;

/// Runs the commands of the interface on their own thread, in the order
/// they arrive. The thread that delivers them is the WebView's: on Android
/// the page waits for it (`postMessage` of the JavaScript bridge), and on
/// Windows it is the window's. A command that reads or writes the library
/// would freeze typing and scrolling while it runs. One thread keeps the
/// order the commands had (shared notes and recording chunks rely on it);
/// a command that returns a task still runs that task on the async runtime.
fn run_in_order(command: impl FnOnce() + Send + 'static) {
    static COMMANDS: std::sync::OnceLock<std::sync::Mutex<std::sync::mpsc::Sender<Command>>> = std::sync::OnceLock::new();
    let sender = COMMANDS.get_or_init(|| {
        let (sender, receiver) = std::sync::mpsc::channel::<Command>();
        let spawned = std::thread::Builder::new().name("notia-commands".into()).spawn(move || {
            for command in receiver {
                // A command that panics answers nothing; the next ones still run.
                let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(command));
            }
        });
        if spawned.is_err() {
            log::error!("[notia:host] the command thread did not start");
        }
        std::sync::Mutex::new(sender)
    });
    let command: Command = Box::new(command);
    let rejected = match sender.lock() {
        Ok(sender) => sender.send(command).err().map(|error| error.0),
        Err(_) => Some(command),
    };
    // Without the thread the command runs here, as it did before.
    if let Some(command) = rejected {
        command();
    }
}

fn invoke_body(body: &InvokeBody) -> serde_json::Value {
    match body {
        InvokeBody::Json(value) => value.clone(),
        InvokeBody::Raw(_) => serde_json::Value::Object(Default::default()),
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::async_runtime::set(notia_app::host::async_runtime::handle());
    install_channel_factory();

    // The dialog plugin goes first: the host plugin may need it to report
    // that the data folder is in use.
    let mut builder = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(host_plugin());
    for hook in notia_app::startup_hooks() {
        builder = builder.plugin(startup_plugin(hook));
    }

    #[cfg(target_os = "windows")]
    let builder = crate::windows_tray::configure(builder);

    let window_commands: fn(tauri::ipc::Invoke<Wry>) -> bool = tauri::generate_handler![
        notia_log,
        window_control,
        exit_application,
        start_window_dragging,
        start_window_dragging_with_restore,
    ];

    builder
        .invoke_handler(move |invoke| {
            // The interface reaches the application only through `app_invoke`;
            // any other command belongs to the window.
            if invoke.message.command() != APP_INVOKE {
                return window_commands(invoke);
            }
            let webview = invoke.message.webview();
            let Some(app) = webview.try_state::<AppContext>() else {
                invoke.resolver.reject("Notia todavía se está iniciando.");
                return true;
            };
            let body = invoke_body(invoke.message.payload());
            let label = webview.window().label().to_string();
            let app = app.inner().clone();
            let resolver = invoke.resolver;
            run_in_order(move || match dispatch_app_invoke(&app, &label, &body) {
                Dispatch::Ready(reply) => resolver.respond(reply.map_err(InvokeError)),
                Dispatch::Pending(task) => resolver.respond_async(async move { task.await.map_err(InvokeError) }),
            });
            true
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
