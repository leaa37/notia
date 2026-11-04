//! HTTPS + WebSocket API of a Notia server, shared by the headless mode
//! (`notia --headless`) and the Host mode of the app window.
//!
//! Routes:
//! - `GET /api/health`: liveness, protocol version and the library served
//!   (public), what a client's «Probar conexión» calls.
//! - `GET /api/auth/status`: whether the library served is locked or waits
//!   for the Owner's first password (public).
//! - `POST /api/auth/login` `{ username, password }` (the library's Owner)
//!   or `{ password }` (the headless server's own password),
//!   `POST /api/auth/logout`, `GET /api/session`: session in an `HttpOnly`
//!   cookie.
//! - `POST /api/auth/first-login|create-password|change-password`: the
//!   Owner's first password and password change (public, rate limited).
//! - `GET /api/capabilities`: commands a remote client may call.
//! - `POST /api/invoke` `{ command, args }`: runs a command of the registry
//!   and answers `{ result }` or `{ error }` (the backend error as is).
//! - `GET /api/file?path=`: a file of a registered library.
//! - `GET /api/events[?since=N]` (WebSocket): every event of the
//!   application as `{ seq, event, payload }`.
//! - `POST /api/browser/*` and `POST /api/meeting-call/*`: the ColdPass
//!   and NotIA Chrome extensions (`browser`, `meeting_call`), also over
//!   plain HTTP from this same computer.
//! - Anything else: files of the interface, when the server has them.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::RecvTimeoutError;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rand::RngCore;
use rustls::{ServerConfig, ServerConnection, StreamOwned};
use serde_json::Value;
use tungstenite::{accept_with_config, protocol::WebSocketConfig, Message};

use super::events::EventHub;
use super::http::{
    http_body, http_redirect, is_websocket_upgrade, json_response, json_response_with_headers, read_http_request_limited,
    request_cookie, request_line_parts, request_origin_is_expected, request_query_param, response,
    response_with_headers, serve_http_redirect, text_response, websocket_timeout, PrefixedStream,
};
use super::owner;
use super::rate::RateWindows;
use crate::host::{AppContext, AssetSource, DataDirLock};
use crate::registry::{dispatch_app_invoke, is_remote_command, Dispatch, COMMAND_NAMES, LOCAL_ONLY_COMMANDS};

mod browser;
mod extension;
mod meeting_call;

pub(crate) const PROTOCOL_VERSION: u16 = crate::backend::connection::HOST_PROTOCOL_VERSION;
const SESSION_COOKIE: &str = "notia_session";
const SESSION_TTL: Duration = Duration::from_secs(12 * 60 * 60);
const MAX_SESSIONS: usize = 64;
const MAX_CONNECTIONS: usize = 128;
/// Chat attachments travel inside commands, so requests may be large.
const MAX_REQUEST_BYTES: usize = 32 * 1024 * 1024;
const MAX_EVENT_CLIENT_MESSAGE_BYTES: usize = 64 * 1024;
/// The extension sends a page address and a sign-in, never files.
const MAX_BROWSER_REQUEST_BYTES: usize = 64 * 1024;
const REQUEST_READ_TIMEOUT: Duration = Duration::from_secs(5);
const EVENT_POLL_INTERVAL: Duration = Duration::from_millis(50);
const EVENT_PING_INTERVAL: Duration = Duration::from_secs(30);
const ACCEPT_POLL_INTERVAL: Duration = Duration::from_millis(100);
const INVOKES_PER_MINUTE: usize = 3_000;
/// Largest library file served to the interface (images, attachments).
const MAX_LIBRARY_FILE_BYTES: u64 = 64 * 1024 * 1024;
/// Policy of the interface pages: the same the app window uses, with the
/// server origin for requests and WebSocket, and no framing by other sites.
/// Inline and evaluated scripts are needed by XGraph and the editors.
const INTERFACE_CONTENT_SECURITY_POLICY: &str = "Content-Security-Policy: default-src 'self'; \
    script-src 'self' 'unsafe-inline' 'unsafe-eval' 'wasm-unsafe-eval'; style-src 'self' 'unsafe-inline'; \
    img-src 'self' data: blob: https:; media-src 'self' data: blob:; font-src 'self' data:; \
    worker-src 'self' blob:; connect-src 'self'; frame-src 'self' blob: data:; object-src 'none'; \
    base-uri 'self'; form-action 'self'; frame-ancestors 'none'";
/// Label of the "window" remote clients act from.
const REMOTE_WINDOW_LABEL: &str = "remote";

/// What the server is, as `/api/health` reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ServerKind {
    Headless,
    Host,
}

impl ServerKind {
    fn id(self) -> &'static str {
        match self {
            Self::Headless => "headless",
            Self::Host => "host",
        }
    }
}

pub(crate) struct ApiServer {
    pub(crate) app: AppContext,
    pub(crate) hub: Arc<EventHub>,
    pub(crate) assets: Option<Arc<dyn AssetSource>>,
    pub(crate) kind: ServerKind,
    /// Folder of the headless server's own password; the Host mode signs in
    /// only the library's Owner.
    pub(crate) server_dir: Option<PathBuf>,
    pub(crate) stop: Arc<AtomicBool>,
    sessions: Mutex<HashMap<String, Instant>>,
    /// Vaults the ColdPass extension opened (Host mode).
    browser: crate::coldpass::BrowserVaults,
    /// Sessions of the NotIA extension (Host mode).
    call_sessions: extension::ExtensionSessions,
    rates: Mutex<RateWindows>,
    connections: AtomicUsize,
    _lock: Option<DataDirLock>,
}

impl ApiServer {
    pub(crate) fn new(
        app: AppContext,
        hub: Arc<EventHub>,
        assets: Option<Arc<dyn AssetSource>>,
        kind: ServerKind,
        server_dir: Option<PathBuf>,
        lock: Option<DataDirLock>,
    ) -> Self {
        Self {
            app,
            hub,
            assets,
            kind,
            server_dir,
            stop: Arc::new(AtomicBool::new(false)),
            sessions: Mutex::new(HashMap::new()),
            browser: crate::coldpass::BrowserVaults::default(),
            call_sessions: extension::ExtensionSessions::default(),
            rates: Mutex::new(RateWindows::default()),
            connections: AtomicUsize::new(0),
            _lock: lock,
        }
    }

    fn stopped(&self) -> bool {
        self.stop.load(Ordering::SeqCst)
    }

    fn allow(&self, key: String, limit: usize) -> bool {
        self.rates
            .lock()
            .is_ok_and(|mut rates| rates.allow(key, limit, Duration::from_secs(60)))
    }

    fn session_active(&self, token: &str) -> bool {
        if self.stopped() {
            return false;
        }
        let Ok(mut sessions) = self.sessions.lock() else {
            return false;
        };
        match sessions.get(token) {
            Some(expires) if *expires > Instant::now() => true,
            Some(_) => {
                sessions.remove(token);
                false
            }
            None => false,
        }
    }

    fn open_session(&self) -> Option<String> {
        let mut bytes = [0_u8; 32];
        rand::thread_rng().fill_bytes(&mut bytes);
        let token: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
        let mut sessions = self.sessions.lock().ok()?;
        let now = Instant::now();
        sessions.retain(|_, expires| *expires > now);
        if sessions.len() >= MAX_SESSIONS {
            let oldest = sessions.iter().min_by_key(|(_, expires)| **expires).map(|(token, _)| token.clone())?;
            sessions.remove(&oldest);
        }
        sessions.insert(token.clone(), now + SESSION_TTL);
        Some(token)
    }

    fn close_session(&self, token: &str) {
        if let Ok(mut sessions) = self.sessions.lock() {
            sessions.remove(token);
        }
    }

    /// Ends every session (the Owner's password changed).
    fn close_all_sessions(&self) {
        if let Ok(mut sessions) = self.sessions.lock() {
            sessions.clear();
        }
    }
}

/// Accepts connections until the server is stopped.
pub(crate) fn run(listener: TcpListener, server: Arc<ApiServer>, tls: Arc<ServerConfig>) {
    let _ = listener.set_nonblocking(true);
    while !server.stopped() {
        let stream = match listener.accept() {
            Ok((stream, _)) => stream,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(ACCEPT_POLL_INTERVAL);
                continue;
            }
            Err(_) => continue,
        };
        let _ = stream.set_nonblocking(false);
        if server.connections.fetch_add(1, Ordering::SeqCst) >= MAX_CONNECTIONS {
            server.connections.fetch_sub(1, Ordering::SeqCst);
            continue;
        }
        let spawned = std::thread::Builder::new().name("notia-api-connection".into()).spawn({
            let server = Arc::clone(&server);
            let tls = Arc::clone(&tls);
            move || {
                serve_connection(stream, &server, tls);
                server.connections.fetch_sub(1, Ordering::SeqCst);
            }
        });
        if spawned.is_err() {
            server.connections.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

fn serve_connection(stream: TcpStream, server: &ApiServer, tls: Arc<ServerConfig>) {
    let _ = stream.set_read_timeout(Some(REQUEST_READ_TIMEOUT));
    let peer = stream.peer_addr().map(|address| address.ip().to_string()).unwrap_or_else(|_| "unknown".into());
    let mut first_byte = [0_u8; 1];
    let is_tls = matches!(stream.peek(&mut first_byte), Ok(1)) && first_byte[0] == 22;
    if !is_tls {
        let loopback = stream.peer_addr().is_ok_and(|address| address.ip().is_loopback());
        if loopback {
            serve_loopback_plain(stream, server, &peer);
        } else {
            serve_http_redirect(stream);
        }
        return;
    }
    let Ok(raw) = stream.try_clone() else {
        return;
    };
    let Ok(connection) = ServerConnection::new(tls) else {
        return;
    };
    serve_request(StreamOwned::new(connection, stream), &raw, server, &peer);
}

/// Plain HTTP from this same computer: the routes of the extensions are
/// served (the traffic never leaves the machine, and Chrome does not let an
/// extension reach a self-signed HTTPS address); anything else is sent to
/// HTTPS as from any other address.
fn serve_loopback_plain(mut stream: TcpStream, server: &ApiServer, peer: &str) {
    let response = match read_http_request_limited(&mut stream, MAX_BROWSER_REQUEST_BYTES) {
        Ok(request) => match request_line_parts(&request) {
            Some((method, path)) if path.starts_with("/api/browser/") => browser::route(&request, method, &path, server, peer),
            Some((method, path)) if path.starts_with("/api/meeting-call/") => {
                meeting_call::route(&request, method, &path, server, peer)
            }
            _ => http_redirect(&request),
        },
        Err(status) => text_response(status, "Solicitud HTTP inválida."),
    };
    let _ = stream.write_all(&response);
}

fn session_cookie(token: &str, max_age: u64) -> String {
    format!("Set-Cookie: {SESSION_COOKIE}={token}; HttpOnly; Secure; SameSite=Strict; Path=/; Max-Age={max_age}")
}

fn error_body(status: &str, message: &str) -> Vec<u8> {
    json_response(status, serde_json::json!({ "error": message }))
}

fn serve_request<S: Read + Write>(mut stream: S, raw: &TcpStream, server: &ApiServer, peer: &str) {
    let request = match read_http_request_limited(&mut stream, MAX_REQUEST_BYTES) {
        Ok(request) => request,
        Err(status) => {
            let _ = stream.write_all(&text_response(status, "Solicitud HTTP inválida."));
            return;
        }
    };
    let Some((method, path)) = request_line_parts(&request) else {
        let _ = stream.write_all(&text_response("400 Bad Request", "Solicitud inválida."));
        return;
    };
    let session = request_cookie(&request, SESSION_COOKIE)
        .filter(|token| server.session_active(token))
        .map(str::to_owned);

    if method == "GET" && path == "/api/events" {
        let Some(session) = session else {
            let _ = stream.write_all(&error_body("401 Unauthorized", "Iniciá sesión para continuar."));
            return;
        };
        if !is_websocket_upgrade(&request) {
            let _ = stream.write_all(&text_response("426 Upgrade Required", "Esta ruta requiere WebSocket."));
            return;
        }
        let _ = raw.set_read_timeout(Some(EVENT_POLL_INTERVAL));
        let since = request_query_param(&request, "since").and_then(|value| value.parse::<u64>().ok());
        serve_events(PrefixedStream::new(request, stream), server, &session, since);
        return;
    }

    let response = route(&request, method, &path, session.as_deref(), server, peer);
    let _ = stream.write_all(&response);
}

fn route(request: &[u8], method: &str, path: &str, session: Option<&str>, server: &ApiServer, peer: &str) -> Vec<u8> {
    let is_api = path == "/api" || path.starts_with("/api/");
    // The extensions' own origin and token; see `extension`.
    if path.starts_with("/api/browser/") {
        return browser::route(request, method, path, server, peer);
    }
    if path.starts_with("/api/meeting-call/") {
        return meeting_call::route(request, method, path, server, peer);
    }
    if method == "POST" && !request_origin_is_expected(request) {
        return error_body("403 Forbidden", "Origen no autorizado.");
    }
    match (method, path) {
        ("GET", "/api/health") => health(server),
        ("GET", "/api/auth/status") => auth_status(server),
        ("POST", "/api/auth/login") => login(http_body(request), server, peer),
        ("POST", "/api/auth/logout") => {
            if let Some(token) = session {
                server.close_session(token);
            }
            json_response_with_headers("200 OK", serde_json::json!({ "ok": true }), &[&session_cookie("", 0)])
        }
        ("POST", "/api/auth/first-login" | "/api/auth/create-password" | "/api/auth/change-password") => {
            owner_password_flow(path, http_body(request), server, peer)
        }
        ("GET", "/api/session") => json_response("200 OK", serde_json::json!({ "authenticated": session.is_some() })),
        _ if is_api && session.is_none() => error_body("401 Unauthorized", "Iniciá sesión para continuar."),
        ("GET", "/api/capabilities") => json_response("200 OK", capabilities()),
        ("GET", "/api/file") => serve_library_file(request, &server.app),
        ("POST", "/api/invoke") => {
            let session = session.unwrap_or_default();
            // Collaborative editing sends a message every few keystrokes.
            if !server.allow(format!("invoke:{session}"), INVOKES_PER_MINUTE) {
                return json_response_with_headers(
                    "429 Too Many Requests",
                    serde_json::json!({ "error": "Demasiadas operaciones. Esperá unos segundos.", "retryable": true }),
                    &["Retry-After: 30"],
                );
            }
            invoke(http_body(request), server)
        }
        _ if is_api => error_body("404 Not Found", "No existe."),
        ("GET", _) | ("HEAD", _) => serve_static(server.assets.as_deref(), path),
        _ => error_body("405 Method Not Allowed", "Método no permitido."),
    }
}

fn health(server: &ApiServer) -> Vec<u8> {
    let library = crate::library_catalog::selected_library(&server.app);
    json_response(
        "200 OK",
        serde_json::json!({
            "ok": true,
            "app": "notia",
            "protocolVersion": PROTOCOL_VERSION,
            "mode": server.kind.id(),
            "platform": std::env::consts::OS,
            "libraryId": library.as_ref().map(|library| library.id.clone()),
            "library": library.map(|library| library.name),
            // How much this computer processes: a computer that syncs with
            // it decides with it which one runs the library's services.
            "device": crate::device_role::local_power(&server.app),
        }),
    )
}

fn auth_status(server: &ApiServer) -> Vec<u8> {
    match crate::app_auth::host_status(&server.app) {
        Ok(status) => json_response("200 OK", status),
        Err(error) => json_response("400 Bad Request", serde_json::json!({ "error": error })),
    }
}

pub(crate) fn capabilities() -> Value {
    let remote: Vec<&str> = COMMAND_NAMES.iter().copied().filter(|command| is_remote_command(command)).collect();
    serde_json::json!({
        "protocolVersion": PROTOCOL_VERSION,
        "platform": std::env::consts::OS,
        "commands": remote,
        "localOnlyCommands": LOCAL_ONLY_COMMANDS,
    })
}

fn too_many_attempts() -> Vec<u8> {
    json_response_with_headers(
        "429 Too Many Requests",
        serde_json::json!({ "error": "Demasiados intentos. Esperá un minuto.", "retryable": true }),
        &["Retry-After: 60"],
    )
}

fn login(body: &[u8], server: &ApiServer, peer: &str) -> Vec<u8> {
    if !server.allow(format!("login:{peer}"), 30) {
        return too_many_attempts();
    }
    let input = serde_json::from_slice::<Value>(body).unwrap_or(Value::Null);
    let text = |key: &str| input.get(key).and_then(Value::as_str).map(str::to_owned);
    let Some(password) = text("password") else {
        return error_body("400 Bad Request", "Falta la contraseña.");
    };
    let signed_in = match (text("username"), &server.server_dir) {
        // The library's Owner: the same sign-in as the app, which also
        // unlocks the library here.
        (Some(username), _) => crate::app_auth::host_sign_in(&server.app, &username, &password).map_err(|error| error.message),
        (None, Some(server_dir)) if owner::verify_owner_password(server_dir, &password) => Ok(()),
        (None, Some(_)) => Err("Contraseña incorrecta.".to_string()),
        (None, None) => Err("Iniciá sesión con el usuario Owner de la biblioteca.".to_string()),
    };
    if let Err(message) = signed_in {
        return error_body("401 Unauthorized", &message);
    }
    match server.open_session() {
        Some(token) => json_response_with_headers(
            "200 OK",
            serde_json::json!({ "ok": true }),
            &[&session_cookie(&token, SESSION_TTL.as_secs())],
        ),
        None => error_body("500 Internal Server Error", "No se pudo abrir la sesión."),
    }
}

/// «Primer inicio» and «Cambiar contraseña» of the library's Owner, as the
/// app's sign-in window runs them, for the library this server shows.
fn owner_password_flow(path: &str, body: &[u8], server: &ApiServer, peer: &str) -> Vec<u8> {
    if !server.allow(format!("password:{peer}"), 10) {
        return too_many_attempts();
    }
    let input = serde_json::from_slice::<Value>(body).unwrap_or(Value::Null);
    let text = |key: &str| input.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
    let result = match path {
        "/api/auth/first-login" => crate::app_auth::host_first_login(&server.app, &text("username")),
        "/api/auth/create-password" => crate::app_auth::host_create_password(&server.app, &text("username"), &text("password")),
        _ => crate::app_auth::host_change_password(&server.app, &text("username"), &text("current"), &text("new")).inspect(|_| {
            // Sessions opened with the old password end.
            server.close_all_sessions();
            server.browser.close_all();
            server.call_sessions.close_all();
        }),
    };
    match result {
        Ok(()) => json_response("200 OK", serde_json::json!({ "ok": true })),
        Err(error) => json_response("400 Bad Request", serde_json::json!({ "error": error })),
    }
}

fn invoke(body: &[u8], server: &ApiServer) -> Vec<u8> {
    let Ok(request) = serde_json::from_slice::<Value>(body) else {
        return error_body("400 Bad Request", "Solicitud inválida.");
    };
    let command = request.get("command").and_then(Value::as_str).unwrap_or_default();
    if !is_remote_command(command) {
        return error_body("403 Forbidden", "Esta operación solo está disponible en el equipo que ejecuta Notia.");
    }
    // The clients of a host act with their own label: some settings belong
    // to the host alone (Telegram).
    let window_label = match server.kind {
        ServerKind::Headless => REMOTE_WINDOW_LABEL,
        ServerKind::Host => crate::registry::CLIENT_WINDOW_LABEL,
    };
    let reply = match dispatch_app_invoke(&server.app, window_label, &request) {
        Dispatch::Ready(reply) => reply,
        Dispatch::Pending(task) => crate::host::async_runtime::block_on(task),
    };
    match reply {
        Ok(result) => json_response("200 OK", serde_json::json!({ "result": result })),
        Err(error) => json_response("400 Bad Request", serde_json::json!({ "error": error })),
    }
}

/// A file of a registered library, for the interface to show (images).
/// Served sandboxed so an SVG or HTML file cannot run scripts on this origin.
fn serve_library_file(request: &[u8], app: &AppContext) -> Vec<u8> {
    use crate::host::Manager;
    let Some(path) = request_query_param(request, "path").filter(|path| !path.trim().is_empty()) else {
        return error_body("400 Bad Request", "Falta la ruta del archivo.");
    };
    let path = Path::new(&path);
    let registry = app.state::<crate::library_registry::LibraryBindingRegistry>();
    if !registry.contains_desktop_path(path) {
        return error_body("403 Forbidden", "La ruta está fuera de las bibliotecas registradas.");
    }
    let Ok(metadata) = std::fs::metadata(path) else {
        return error_body("404 Not Found", "No existe.");
    };
    if !metadata.is_file() {
        return error_body("404 Not Found", "No existe.");
    }
    if metadata.len() > MAX_LIBRARY_FILE_BYTES {
        return error_body("413 Payload Too Large", "El archivo supera el tamaño permitido.");
    }
    match std::fs::read(path) {
        Ok(bytes) => response_with_headers(
            "200 OK",
            super::assets::mime_type(&path.to_string_lossy()),
            &bytes,
            &["Content-Security-Policy: sandbox"],
        ),
        Err(_) => error_body("404 Not Found", "No existe."),
    }
}

fn serve_static(assets: Option<&dyn AssetSource>, path: &str) -> Vec<u8> {
    let Some(assets) = assets else {
        return text_response("200 OK", "Servidor Notia activo.");
    };
    let requested = if path == "/" { "index.html" } else { path.trim_start_matches('/') };
    // Unknown routes belong to the single-page application.
    match assets.get(requested).or_else(|| assets.get("index.html")) {
        Some(asset) if asset.mime_type().starts_with("text/html") => response_with_headers(
            "200 OK",
            asset.mime_type(),
            asset.bytes(),
            &[INTERFACE_CONTENT_SECURITY_POLICY],
        ),
        Some(asset) => response("200 OK", asset.mime_type(), asset.bytes()),
        None => text_response("404 Not Found", "No existe."),
    }
}

fn serve_events<S: Read + Write>(stream: PrefixedStream<S>, server: &ApiServer, session: &str, since: Option<u64>) {
    let config = WebSocketConfig {
        max_message_size: Some(MAX_EVENT_CLIENT_MESSAGE_BYTES),
        max_frame_size: Some(MAX_EVENT_CLIENT_MESSAGE_BYTES),
        ..WebSocketConfig::default()
    };
    let Ok(mut socket) = accept_with_config(stream, Some(config)) else {
        return;
    };
    let (receiver, replay) = server.hub.subscribe(since);
    for message in replay {
        if socket.send(Message::text(message)).is_err() {
            return;
        }
    }
    let mut last_ping = Instant::now();
    loop {
        if !server.session_active(session) {
            let _ = socket.close(None);
            let _ = socket.flush();
            return;
        }
        match receiver.recv_timeout(EVENT_POLL_INTERVAL) {
            Ok(message) => {
                if socket.send(Message::text(message)).is_err() {
                    return;
                }
                continue;
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        match socket.read() {
            Ok(Message::Close(_)) => return,
            Ok(_) => {}
            Err(error) if websocket_timeout(&error) => {}
            Err(_) => return,
        }
        if last_ping.elapsed() >= EVENT_PING_INTERVAL {
            if socket.send(Message::Ping(Vec::new().into())).is_err() {
                return;
            }
            last_ping = Instant::now();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::assets::DirectoryAssets;
    use crate::host::{AppPaths, HostPorts};

    #[test]
    fn capabilities_leave_out_local_only_commands() {
        let value = capabilities();
        let commands = value["commands"].as_array().expect("commands");
        assert!(commands.iter().any(|command| command == "task_manager_board_view"));
        assert!(!commands.iter().any(|command| command == "start_speech_session"));
        assert!(commands.iter().any(|command| command == "speech_remote_audio"));
        assert_eq!(value["platform"], std::env::consts::OS);
    }

    #[test]
    fn library_files_are_served_only_inside_registered_libraries() {
        let root = std::env::temp_dir().join(format!("notia-file-route-{}", uuid::Uuid::new_v4()));
        let library = root.join("biblioteca");
        std::fs::create_dir_all(&library).expect("library folder");
        std::fs::write(library.join("foto.png"), b"png").expect("image");
        std::fs::write(root.join("secreto.txt"), b"no").expect("outside file");
        let app = crate::create_app(AppPaths::new(Some(root.join("data")), None), HostPorts::default());
        crate::library_catalog::add_desktop_library(&app, &library).expect("library registered");
        let request = |path: &std::path::Path| {
            let encoded: String = path
                .to_string_lossy()
                .bytes()
                .map(|byte| if byte.is_ascii_alphanumeric() { (byte as char).to_string() } else { format!("%{byte:02X}") })
                .collect();
            format!("GET /api/file?path={encoded} HTTP/1.1\r\nHost: h\r\n\r\n").into_bytes()
        };
        let served = String::from_utf8_lossy(&serve_library_file(&request(&library.join("foto.png")), &app)).into_owned();
        assert!(served.starts_with("HTTP/1.1 200"));
        assert!(served.contains("Content-Type: image/png"));
        assert!(served.contains("Content-Security-Policy: sandbox"));
        let outside = serve_library_file(&request(&root.join("secreto.txt")), &app);
        assert!(String::from_utf8_lossy(&outside).starts_with("HTTP/1.1 403"));
        let traversal = serve_library_file(&request(&library.join("..").join("secreto.txt")), &app);
        assert!(String::from_utf8_lossy(&traversal).starts_with("HTTP/1.1 403"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn interface_pages_carry_the_content_security_policy() {
        let root = std::env::temp_dir().join(format!("notia-static-csp-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("assets")).expect("static folder");
        std::fs::write(root.join("index.html"), b"<html></html>").expect("index");
        std::fs::write(root.join("assets/app.js"), b"ok").expect("script");
        let assets = DirectoryAssets::new(root.clone());
        let page = String::from_utf8_lossy(&serve_static(Some(&assets), "/biblioteca")).into_owned();
        assert!(page.contains("Content-Security-Policy: default-src 'self';"));
        assert!(page.contains("frame-ancestors 'none'"));
        let script = String::from_utf8_lossy(&serve_static(Some(&assets), "/assets/app.js")).into_owned();
        assert!(!script.contains("Content-Security-Policy"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn remote_invoke_refuses_local_only_and_runs_registry_commands() {
        let app = crate::create_app(AppPaths::default(), HostPorts::default());
        let server = ApiServer::new(app, Arc::new(EventHub::default()), None, ServerKind::Headless, None, None);
        let refused = invoke(br#"{"command":"start_speech_session","args":{}}"#, &server);
        assert!(String::from_utf8_lossy(&refused).starts_with("HTTP/1.1 403"));
        let missing = invoke(br#"{"command":"finance_overview"}"#, &server);
        let text = String::from_utf8_lossy(&missing);
        assert!(text.starts_with("HTTP/1.1 400"));
        assert!(text.contains("missing required key payload"));
    }

    #[test]
    fn health_reports_the_server_and_its_library() {
        let root = std::env::temp_dir().join(format!("notia-health-{}", uuid::Uuid::new_v4()));
        let app = crate::create_app(AppPaths::new(Some(root.join("data")), None), HostPorts::default());
        let server = ApiServer::new(app, Arc::new(EventHub::default()), None, ServerKind::Host, None, None);
        let text = String::from_utf8_lossy(&health(&server)).into_owned();
        assert!(text.starts_with("HTTP/1.1 200") && text.contains("\"app\":\"notia\"") && text.contains("\"mode\":\"host\""));
        let _ = std::fs::remove_dir_all(&root);
    }
}
