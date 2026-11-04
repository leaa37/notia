//! The HTTP side of the webhook: accepts the connections of the endpoint,
//! answers `POST /webhook` and queues the calls the core accepts.

use std::io::Write as _;
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};

use super::{lock, now_ms, read_config, WebhookState};
use crate::backend::webhooks::{self as core, CallRecord, CallRejection, CallStatus, WebhookCallPrompt, CALLS_PER_MINUTE, MAX_BODY_BYTES, WEBHOOK_PATH};
use crate::host::{AppHandle, Manager};
use crate::server::http::{http_body, json_response, json_response_with_headers, read_http_request_limited, request_header_value, request_line_parts};
use crate::telegram_worker::{enqueue_webhook, ActionEnqueue, WebhookJob};

pub(super) type Rates = crate::server::rate::RateWindows;

const ACCEPT_POLL_INTERVAL: Duration = Duration::from_millis(100);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_CONNECTIONS: usize = 16;
/// Headers on top of the body.
const MAX_HEADER_BYTES: usize = 16 * 1024;

/// The addresses of this computer other devices of the network call.
pub(super) fn network_urls(port: u16) -> Vec<String> {
    crate::server::network::local_ipv4_addresses().iter().map(|address| format!("http://{address}:{port}{WEBHOOK_PATH}")).collect()
}

pub(super) fn accept(listener: TcpListener, app: &AppHandle, library_id: &str, stop: &AtomicBool) {
    let _ = listener.set_nonblocking(true);
    let connections = Arc::new(AtomicUsize::new(0));
    while !stop.load(Ordering::SeqCst) {
        let stream = match listener.accept() {
            Ok((stream, _)) => stream,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(ACCEPT_POLL_INTERVAL);
                continue;
            }
            Err(_) => continue,
        };
        if connections.fetch_add(1, Ordering::SeqCst) >= MAX_CONNECTIONS {
            connections.fetch_sub(1, Ordering::SeqCst);
            continue;
        }
        let spawned = std::thread::Builder::new().name("notia-webhook-call".into()).spawn({
            let app = app.clone();
            let library_id = library_id.to_string();
            let connections = Arc::clone(&connections);
            move || {
                serve(stream, &app, &library_id);
                connections.fetch_sub(1, Ordering::SeqCst);
            }
        });
        if spawned.is_err() {
            connections.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

fn serve(mut stream: TcpStream, app: &AppHandle, library_id: &str) {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(REQUEST_TIMEOUT));
    let _ = stream.set_write_timeout(Some(REQUEST_TIMEOUT));
    let response = match read_http_request_limited(&mut stream, MAX_BODY_BYTES + MAX_HEADER_BYTES) {
        Ok(request) => match request_line_parts(&request) {
            Some((_, path)) if path != WEBHOOK_PATH => failure("404 Not Found", &format!("La única ruta es POST {WEBHOOK_PATH}.")),
            Some(("POST", _)) => {
                let token = request_header_value(&request, "authorization");
                let (status, body) = handle_call(app, library_id, http_body(&request), core::bearer_token(token.as_deref()));
                json_response(status, body)
            }
            Some(_) => json_response_with_headers(
                "405 Method Not Allowed",
                json!({ "ok": false, "error": format!("Usá POST {WEBHOOK_PATH}.") }),
                &["Allow: POST"],
            ),
            None => failure("400 Bad Request", "Solicitud HTTP inválida."),
        },
        Err(status) => failure(status, "Solicitud HTTP inválida o demasiado grande."),
    };
    let _ = stream.write_all(&response);
}

fn failure(status: &str, message: &str) -> Vec<u8> {
    json_response(status, json!({ "ok": false, "error": message }))
}

fn rejected(status: &'static str, message: String) -> (&'static str, Value) {
    (status, json!({ "ok": false, "error": message }))
}

/// Checks a call and queues its run; the status and body of the answer.
fn handle_call(app: &AppHandle, library_id: &str, body: &[u8], token: Option<&str>) -> (&'static str, Value) {
    let call = match core::parse_call(body) {
        Ok(call) => call,
        Err(rejection) => return rejected(rejection.status(), rejection.message()),
    };
    let config = match read_config(app, library_id) {
        Ok(config) if config.enabled => config,
        Ok(_) => return rejected("503 Service Unavailable", "El webhook está apagado en Notia.".into()),
        Err(_) => return rejected("503 Service Unavailable", "Notia no puede leer la configuración ahora (¿la biblioteca está bloqueada?).".into()),
    };
    let state = app.state::<WebhookState>();
    let refuse = |action: &str, rejection: CallRejection| {
        lock(&state.calls).push(CallRecord {
            id: uuid::Uuid::new_v4().to_string(),
            library_id: library_id.to_string(),
            action: action.to_string(),
            received_at_ms: now_ms(),
            status: CallStatus::Rejected,
            prompt: String::new(),
            detail: rejection.message(),
        });
        rejected(rejection.status(), rejection.message())
    };
    let action = match core::authorize(&config, &call, token) {
        Ok(action) => action,
        Err(rejection) => return refuse(&call.action, rejection),
    };
    if !lock(&state.rates).allow(format!("{library_id}:{}", action.id), CALLS_PER_MINUTE, Duration::from_secs(60)) {
        return refuse(&action.name, CallRejection::TooManyCalls);
    }
    if !crate::telegram_worker::bot_runs_for(app, library_id) {
        return refuse(&action.name, CallRejection::Unavailable("Telegram no está activo en esta PC: Notia responde los webhooks por Telegram.".into()));
    }
    let id = uuid::Uuid::new_v4().to_string();
    lock(&state.calls).push(CallRecord {
        id: id.clone(),
        library_id: library_id.to_string(),
        action: action.name.clone(),
        received_at_ms: now_ms(),
        status: CallStatus::Queued,
        prompt: core::prompt_excerpt(&call.prompt),
        detail: String::new(),
    });
    let job = WebhookJob {
        call_id: id.clone(),
        prompt: WebhookCallPrompt { action: action.name.clone(), instructions: action.instructions.clone() },
        request: core::webhook_request(&action.name, &call.prompt, &crate::local_time::local_now().1),
    };
    if enqueue_webhook(app, library_id, job) == ActionEnqueue::Unavailable {
        let message = "El Owner no vinculó su Telegram con el bot de la biblioteca.".to_string();
        lock(&state.calls).finish(&id, Err(message.clone()));
        return rejected("503 Service Unavailable", message);
    }
    ("202 Accepted", json!({ "ok": true, "callId": id, "action": action.name, "status": "queued" }))
}
