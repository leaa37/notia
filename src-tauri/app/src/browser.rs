//! The web browser of the host PC for the Owner's agent (`browser_*`):
//! installs Playwright in the app data folder the first time (with the
//! PC's Node.js and npm), keeps one Playwright process (`browser_agent.mjs`)
//! with a Chrome or Edge window and a profile of Notia's own, and hands it
//! each step as one JSON line. The window stays open between requests, so
//! a task can go on in the next message, until the agent or the person
//! closes it. Arguments, limits and every text the model reads live in
//! `backend::browser`.
//!
//! Only a desktop host browses: Android has no browser for the agent, and a
//! client device works against its host (a connected client's chats already
//! run there).

use std::io::{BufRead as _, BufReader, Cursor, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{mpsc, Mutex};
use std::time::{Duration, Instant};

use base64::Engine as _;
use serde_json::Value;

use crate::backend::browser::{self, AgentAnswer, BrowserCommand, GridSpec, SeenPages};
use crate::backend::{BackendError, BackendErrorCode, BackendRequestContext, RequestControl};
use crate::host::{AppHandle, Manager};
use crate::node_packages::{hide_console, NodePackage};

/// The Playwright Notia installs and drives (its public `ariaSnapshot`
/// with `mode: "ai"` gives the references).
const AGENT_PACKAGE: NodePackage = NodePackage {
    name: "notia-browser-agent",
    script_name: "browser_agent.mjs",
    script: include_str!("browser_agent.mjs"),
    packages: &[("playwright-core", "1.64.1")],
    min_node_major: 18,
    needed_by: "El navegador",
};
const POLL_INTERVAL: Duration = Duration::from_millis(100);
/// A screenshot reaches the model at most this many pixels on its longest
/// side.
const SCREENSHOT_MAX_SIDE: u32 = 1600;
const SCREENSHOT_QUALITY: u8 = 70;

/// The browser process of this device, the last pages it showed whole and
/// the grid of its last screenshot with one.
#[derive(Default)]
pub(crate) struct BrowserState {
    session: Mutex<Option<Session>>,
    /// Read by the confirmations while another step may be running.
    last: Mutex<Remembered>,
}

#[derive(Default, Clone)]
struct Remembered {
    pages: SeenPages,
    grid: Option<GridSpec>,
}

impl BrowserState {
    fn remembered(&self) -> Remembered {
        self.last.lock().map(|last| last.clone()).unwrap_or_default()
    }

    fn update(&self, change: impl FnOnce(&mut Remembered)) {
        if let Ok(mut last) = self.last.lock() {
            change(&mut last);
        }
    }

    /// A new window or none: its references and grid are gone.
    fn forget(&self) {
        self.update(|last| *last = Remembered::default());
    }
}

struct Session {
    child: Child,
    stdin: ChildStdin,
    answers: mpsc::Receiver<String>,
    next_id: u64,
}

impl Session {
    fn is_running(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        // Chrome ends with the Playwright process that drives it.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Whether this device runs the agent's browser: any computer (Windows,
/// Linux or macOS), a client too, never Android.
pub(crate) fn is_available(_app: &AppHandle) -> bool {
    cfg!(not(any(target_os = "android", target_os = "ios")))
}

fn unavailable(_app: &AppHandle) -> BackendError {
    BackendError::new(BackendErrorCode::Unsupported, "El navegador existe solo en las computadoras con Notia, no en Android.", false)
}

/// The confirmation of a click, a text, a key, an option or a drag: the
/// element as the last snapshot shows it, or its cells, and the page. A
/// reference the last snapshot lacks, or a cell outside the last grid,
/// goes back to the model before asking the person.
pub(crate) fn preview(app: &AppHandle, name: &str, arguments: &Value) -> Result<(String, String), BackendError> {
    if !is_available(app) {
        return Err(unavailable(app));
    }
    let command = BrowserCommand::from_call(name, arguments)?;
    let last = app.state::<BrowserState>().remembered();
    command.check_pointers(&last.pages, last.grid.as_ref())?;
    Ok((command.summary(), command.detail(&last.pages)))
}

/// Runs one browser step and returns what the model reads. A screenshot
/// for the person goes with the request's images (Telegram sends it before
/// the next question).
pub(crate) fn execute_tool(
    app: &AppHandle,
    control: &RequestControl,
    context: &BackendRequestContext,
    name: &str,
    arguments: &Value,
) -> Result<Value, BackendError> {
    if !is_available(app) {
        return Err(unavailable(app));
    }
    let command = BrowserCommand::from_call(name, arguments)?;
    let state = app.state::<BrowserState>();
    let last = state.remembered();
    command.check_pointers(&last.pages, last.grid.as_ref())?;
    let described = command.described_target(&last.pages);
    let mut session = state.session.lock().map_err(|_| BackendError::new(BackendErrorCode::Internal, "El navegador quedó bloqueado.", true))?;
    if command == BrowserCommand::Close {
        let was_open = session.as_mut().is_some_and(Session::is_running);
        if was_open {
            if let Some(running) = session.as_mut() {
                // The process closes the window and ends; what it answers no longer matters.
                let _ = exchange(running, &command, None, None, control);
            }
        }
        *session = None;
        state.forget();
        return Ok(browser::closed_result(was_open));
    }
    if !session.as_mut().is_some_and(Session::is_running) {
        state.forget();
        *session = None;
        let folder = install(app)?;
        *session = Some(start(&folder, window_mode())?);
    }
    let running = session.as_mut().expect("the session was just started");
    let answer = match exchange(running, &command, last.grid.as_ref(), described.as_deref(), control) {
        Ok(answer) => answer,
        Err(error) => {
            // A browser that stopped answering or ended starts again on the next step.
            if error.code != BackendErrorCode::Cancelled {
                *session = None;
                state.forget();
            }
            return Err(error);
        }
    };
    drop(session);
    if !answer.ok {
        return Err(answer.failure());
    }
    let page = answer.page.clone().unwrap_or_default();
    state.update(|last| {
        if command.returns_whole_snapshot() {
            last.pages.remember(page.clone());
        }
        if answer.grid.is_some() {
            last.grid = answer.grid;
        } else if !command.keeps_grid() {
            last.grid = None;
        }
    });
    // Only how it went is logged: the pages and what is typed may be private.
    log::info!("[notia:browser] paso {name} hecho");
    Ok(match (&command, answer.image) {
        (BrowserCommand::Screenshot { for_person, .. }, Some(image)) => {
            let image = scaled_screenshot(&image)?;
            if *for_person {
                crate::image_generation::keep_for_reply(&context.request_id, "captura-navegador", "jpg", "image/jpeg", image.clone());
            }
            browser::screenshot_result(&page, image, answer.grid.as_ref(), *for_person)
        }
        (BrowserCommand::Snapshot { offset, .. }, _) => browser::tool_result(&page, *offset),
        _ => browser::tool_result(&page, 0),
    })
}

/// Sends one step and waits for its answer, its time or a cancel. Answers
/// of an earlier step that was cancelled are skipped. `described` is the
/// element as a snapshot showed it, to find it again if the page drew it
/// again.
fn exchange(
    session: &mut Session,
    command: &BrowserCommand,
    grid: Option<&GridSpec>,
    described: Option<&str>,
    control: &RequestControl,
) -> Result<AgentAnswer, BackendError> {
    session.next_id += 1;
    let id = session.next_id;
    let mut request = command.agent_request(id, grid)?;
    if let (Some(described), Some(object)) = (described, request.as_object_mut()) {
        object.insert("described".into(), serde_json::json!(described));
    }
    let line = format!("{request}\n");
    session
        .stdin
        .write_all(line.as_bytes())
        .and_then(|()| session.stdin.flush())
        .map_err(|_| BackendError::new(BackendErrorCode::Conflict, "El navegador se cerró: probá de nuevo para abrirlo.", true))?;
    let deadline = Instant::now() + command.answer_timeout();
    loop {
        if control.is_cancelled() {
            return Err(BackendError::cancelled());
        }
        match session.answers.recv_timeout(POLL_INTERVAL) {
            Ok(line) => match serde_json::from_str::<AgentAnswer>(&line) {
                Ok(answer) if answer.id == id => return Ok(answer),
                Ok(_) => {}
                Err(_) => log::error!("[notia:browser] respuesta ilegible del proceso de Playwright"),
            },
            Err(mpsc::RecvTimeoutError::Timeout) if Instant::now() >= deadline => {
                return Err(BackendError::new(
                    BackendErrorCode::Conflict,
                    "El navegador no respondió a tiempo: se cierra y se abre de nuevo en el próximo paso.",
                    true,
                ));
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err(BackendError::new(BackendErrorCode::Conflict, "El navegador se cerró: probá de nuevo para abrirlo.", true));
            }
        }
    }
}

/// A window the person sees on a desktop; without a display (a Linux
/// server) the browser runs hidden.
fn window_mode() -> &'static str {
    let display = std::env::var_os("DISPLAY").is_some() || std::env::var_os("WAYLAND_DISPLAY").is_some();
    if cfg!(any(target_os = "windows", target_os = "macos")) || display {
        "headed"
    } else {
        "headless"
    }
}

fn start(folder: &Path, mode: &str) -> Result<Session, BackendError> {
    let mut command = Command::new("node");
    command
        .arg(folder.join(AGENT_PACKAGE.script_name))
        .arg(folder.join("profile"))
        .arg(mode)
        .current_dir(folder)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    hide_console(&mut command);
    let mut child = command.spawn().map_err(|_| AGENT_PACKAGE.node_missing())?;
    let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
        let _ = child.kill();
        return Err(BackendError::new(BackendErrorCode::Internal, "No se pudo conectar con el proceso del navegador.", true));
    };
    let (sender, answers) = mpsc::channel();
    let spawned = std::thread::Builder::new().name("notia-browser-read".into()).spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            if sender.send(line).is_err() {
                break;
            }
        }
    });
    if spawned.is_err() {
        let _ = child.kill();
        return Err(BackendError::new(BackendErrorCode::Internal, "No se pudo leer el proceso del navegador.", true));
    }
    Ok(Session { child, stdin, answers, next_id: 0 })
}

/// The folder with the script and Playwright, installed with npm when it
/// is missing or another version.
fn install(app: &AppHandle) -> Result<PathBuf, BackendError> {
    let folder = app
        .path()
        .app_data_dir()
        .map(|directory| directory.join("browser-agent"))
        .map_err(|_| BackendError::new(BackendErrorCode::Storage, "No se pudo resolver la carpeta del navegador.", true))?;
    AGENT_PACKAGE.install_into(&folder)?;
    Ok(folder)
}

/// The screenshot as the model reads it: JPEG, at most
/// [`SCREENSHOT_MAX_SIDE`] pixels on its longest side.
fn scaled_screenshot(base64_jpeg: &str) -> Result<String, BackendError> {
    let invalid = || BackendError::new(BackendErrorCode::Conflict, "La captura del navegador no es una imagen válida.", true);
    let engine = base64::engine::general_purpose::STANDARD;
    let bytes = engine.decode(base64_jpeg.as_bytes()).map_err(|_| invalid())?;
    let image = image::load_from_memory(&bytes).map_err(|_| invalid())?;
    if image.width().max(image.height()) <= SCREENSHOT_MAX_SIDE {
        return Ok(base64_jpeg.to_string());
    }
    let image = image.resize(SCREENSHOT_MAX_SIDE, SCREENSHOT_MAX_SIDE, image::imageops::FilterType::Triangle);
    let mut jpeg = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(Cursor::new(&mut jpeg), SCREENSHOT_QUALITY)
        .encode_image(&image.to_rgb8())
        .map_err(|_| invalid())?;
    Ok(engine.encode(jpeg))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_large_screenshot_is_scaled_down_and_a_small_one_kept() {
        let encode = |width, height| {
            let mut jpeg = Vec::new();
            image::codecs::jpeg::JpegEncoder::new_with_quality(Cursor::new(&mut jpeg), 60)
                .encode_image(&image::RgbImage::new(width, height))
                .expect("jpeg");
            base64::engine::general_purpose::STANDARD.encode(jpeg)
        };
        let small = encode(1280, 800);
        assert_eq!(scaled_screenshot(&small).expect("small"), small);
        let large = scaled_screenshot(&encode(3200, 1800)).expect("large");
        let decoded = image::load_from_memory(&base64::engine::general_purpose::STANDARD.decode(large).expect("base64")).expect("image");
        assert_eq!((decoded.width(), decoded.height()), (1600, 900));
        assert!(scaled_screenshot("no es una imagen").is_err());
    }

    /// The reference on the first snapshot line that names `role`.
    fn reference_of(snapshot: &str, role: &str, last: bool) -> String {
        let mut lines = snapshot.lines().filter(|line| line.trim_start().starts_with(&format!("- {role}")));
        let line = if last { lines.last() } else { lines.next() }.unwrap_or_else(|| panic!("no {role} in {snapshot}"));
        line.split("[ref=").nth(1).and_then(|rest| rest.split(']').next()).expect("reference").to_string()
    }

    /// Installs Playwright in a temporary folder (npm and the network) and
    /// drives the PC's Chrome or Edge, hidden, through the real script:
    /// `cargo test -p notia-app browser -- --ignored`. With
    /// `NOTIA_BROWSER_SHOT` set to a file, it saves there the page's
    /// screenshot with its grid, to look at it.
    #[test]
    #[ignore = "needs Node.js, npm, the network and Chrome or Edge"]
    fn drives_a_real_browser_through_the_script() {
        let folder = std::env::temp_dir().join(format!("notia-browser-{}", std::process::id()));
        AGENT_PACKAGE.install_into(&folder).expect("install");
        let mut session = start(&folder, "headless").expect("start");
        let control = RequestControl::new(None);
        let mut grid = None;
        let mut step = |session: &mut Session, name: &str, arguments: Value| {
            let command = BrowserCommand::from_call(name, &arguments).expect("command");
            let answer = exchange(session, &command, grid.as_ref(), None, &control).expect("answer");
            assert!(answer.ok, "{name}: {:?}", answer.error);
            if answer.grid.is_some() {
                grid = answer.grid;
            }
            answer
        };
        let page = step(&mut session, browser::NAVIGATE_TOOL, serde_json::json!({ "url": "https://example.com" })).page.expect("page");
        assert!(page.title.contains("Example"), "{}", page.title);
        let link = reference_of(&page.snapshot, "link", false);
        // A grid over the link alone, and a click on its first cell, as the
        // person would say after reading the screenshot.
        let shot = step(&mut session, browser::SCREENSHOT_TOOL, serde_json::json!({ "ref": link, "grid": true, "forPerson": true }));
        let over_link = shot.grid.expect("grid");
        assert!(over_link.columns >= 1 && over_link.rows == 1, "{over_link:?}");
        assert!(scaled_screenshot(&shot.image.expect("image")).is_ok());
        let page = step(&mut session, browser::CLICK_TOOL, serde_json::json!({ "cells": ["A1"], "element": "enlace" })).page.expect("page");
        assert!(page.url.contains("iana.org"), "{}", page.url);
        // A drag on the page's own grid runs without error.
        let whole = step(&mut session, browser::SCREENSHOT_TOOL, serde_json::json!({ "grid": true }));
        if let Some(target) = std::env::var_os("NOTIA_BROWSER_SHOT") {
            std::fs::write(target, base64::engine::general_purpose::STANDARD.decode(whole.image.expect("image")).expect("base64")).expect("saved");
        }
        step(&mut session, browser::DRAG_TOOL, serde_json::json!({ "from": "A1", "to": "B2", "element": "arrastre de prueba" }));
        // Low in a scrolled page, the screenshot covers the element.
        let page = step(&mut session, browser::PRESS_KEY_TOOL, serde_json::json!({ "key": "End" })).page.expect("page");
        let low = reference_of(&page.snapshot, "link", true);
        let shot = step(&mut session, browser::SCREENSHOT_TOOL, serde_json::json!({ "ref": low, "grid": true }));
        let image = shot.image.expect("image");
        let decoded = image::load_from_memory(&base64::engine::general_purpose::STANDARD.decode(&image).expect("base64")).expect("jpeg");
        let low_grid = shot.grid.expect("grid");
        // The clip of a box with fractional pixels may lose one.
        assert!((f64::from(decoded.width()) - low_grid.width).abs() <= 1.0, "{} vs {}", decoded.width(), low_grid.width);
        assert!((f64::from(decoded.height()) - low_grid.height).abs() <= 1.0, "{} vs {}", decoded.height(), low_grid.height);
        let missing = step_error(&mut session, &control);
        assert!(missing.contains("la referencia e9999 ya no está"), "{missing}");
        let _ = exchange(&mut session, &BrowserCommand::Close, None, None, &control);
        drop(session);
        let _ = std::fs::remove_dir_all(&folder);
    }

    fn step_error(session: &mut Session, control: &RequestControl) -> String {
        let command = BrowserCommand::from_call(browser::CLICK_TOOL, &serde_json::json!({ "ref": "e9999", "element": "nada" })).expect("command");
        let answer = exchange(session, &command, None, None, control).expect("answer");
        assert!(!answer.ok);
        answer.failure().message
    }
}
