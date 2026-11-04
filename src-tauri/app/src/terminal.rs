//! The terminal of the host PC for the Owner's agent (`run_terminal_command`):
//! runs the command in a shell, streams its output as request events (the
//! chat shows it live, Telegram follows it) and stops the whole process tree
//! on a cancel or when its time is up. While it runs, a light agent reads
//! the live output and tells its status (`terminal-status`). Arguments,
//! decoding, the transcript and every text live in `backend::terminal`.
//!
//! Only a desktop host runs commands: Android has no terminal for the agent,
//! and a client device works against its host (a connected client's chats
//! already run there), so an offline copy never runs them on the client.

use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::backend::terminal::{self, OutputChunk, OutputDecoder, TerminalOutcome, TerminalRequest, Transcript};
use crate::backend::{BackendError, BackendErrorCode, BackendEvent, BackendEventSink, BackendRequestContext, RequestControl};
use crate::host::{AppHandle, Manager};
use crate::library_registry::{LibraryBindingRegistry, LibraryBindingRoot};

/// The output reaches the chat and Telegram at most this often.
const FLUSH_INTERVAL: Duration = Duration::from_millis(300);
const POLL_INTERVAL: Duration = Duration::from_millis(100);
/// After the shell ends, a process it left running may keep its output
/// open; the command counts as ended after this.
const DRAIN_AFTER_EXIT: Duration = Duration::from_secs(1);
const READ_BUFFER: usize = 8 * 1024;
/// How often the status thread looks whether the command ended.
const STATUS_POLL: Duration = Duration::from_millis(250);

/// Whether this device runs the agent's commands: any computer (Windows,
/// Linux or macOS), a client too, never Android.
pub(crate) fn is_available(_app: &AppHandle) -> bool {
    cfg!(not(any(target_os = "android", target_os = "ios")))
}

fn unavailable(_app: &AppHandle) -> BackendError {
    BackendError::new(BackendErrorCode::Unsupported, "La terminal existe solo en las computadoras con Notia, no en Android.", false)
}

/// The confirmation's title and the command whole, with the folder it runs in.
pub(crate) fn preview(app: &AppHandle, context: &BackendRequestContext, arguments: &Value) -> Result<(String, String), BackendError> {
    if !is_available(app) {
        return Err(unavailable(app));
    }
    if let Some(scan) = terminal::DiskUsageRequest::from_arguments(arguments)? {
        return Ok((scan.summary(), scan.detail()));
    }
    let request = TerminalRequest::from_arguments(arguments)?;
    let cwd = working_folder(app, &context.library_id, request.cwd.as_deref())?;
    Ok((request.summary(), request.detail(&cwd.to_string_lossy())))
}

/// Runs the command until it ends, its time is up or the request is
/// cancelled, streaming its output to `events`; with `live_status`, the
/// light agent tells its status meanwhile. What the model reads.
pub(crate) fn execute_tool(
    app: &AppHandle,
    control: &RequestControl,
    events: Option<Arc<dyn BackendEventSink>>,
    live_status: bool,
    context: &BackendRequestContext,
    call_id: &str,
    arguments: &Value,
) -> Result<Value, BackendError> {
    if !is_available(app) {
        return Err(unavailable(app));
    }
    if let Some(scan) = terminal::DiskUsageRequest::from_arguments(arguments)? {
        let (usage, elapsed) = scan_disk(&scan, control)?;
        log::info!("[notia:terminal] uso de disco medido en {} s", elapsed.as_secs());
        return Ok(terminal::disk_usage_result(&scan, usage, elapsed));
    }
    let request = TerminalRequest::from_arguments(arguments)?;
    let cwd = working_folder(app, &context.library_id, request.cwd.as_deref())?;
    let cwd_text = cwd.to_string_lossy().into_owned();
    let publish = |event: BackendEvent| publish_event(events.as_deref(), event);
    let mut child = spawn(&request, &cwd)?;
    publish(BackendEvent::TerminalStarted {
        request_id: context.request_id.clone(),
        call_id: call_id.to_string(),
        command: request.command.clone(),
        shell: request.shell.id().to_string(),
        cwd: cwd_text.clone(),
    });
    let live = Arc::new(Mutex::new(LiveOutput::default()));
    let done = Arc::new(AtomicBool::new(false));
    if let (true, Some(events)) = (live_status, events.clone()) {
        watch_status(app, context, call_id, &request, Arc::clone(&live), Arc::clone(&done), events);
    }
    let (outcome, elapsed, transcript) = follow(&mut child, request.timeout, control, &mut |chunk| {
        if let Ok(mut live) = live.lock() {
            live.push(&chunk);
        }
        publish(BackendEvent::TerminalOutput {
            request_id: context.request_id.clone(),
            call_id: call_id.to_string(),
            text: chunk.text,
            partial: chunk.partial,
        })
    });
    done.store(true, Ordering::SeqCst);
    // Only how it ended is logged: the command and its output may be private.
    log::info!("[notia:terminal] comando {} en {} s", outcome.status(), elapsed.as_secs());
    publish(BackendEvent::TerminalFinished {
        request_id: context.request_id.clone(),
        call_id: call_id.to_string(),
        status: outcome.status().to_string(),
        exit_code: outcome.exit_code(),
        duration_ms: elapsed.as_millis().min(u128::from(u64::MAX)) as u64,
        summary: outcome.text(),
        markdown: terminal::transcript_markdown(&request, &cwd_text, outcome, elapsed, &transcript),
    });
    if outcome == TerminalOutcome::Cancelled {
        return Err(BackendError::cancelled());
    }
    Ok(terminal::tool_result(&request, &cwd_text, outcome, elapsed, &transcript))
}

fn publish_event(events: Option<&dyn BackendEventSink>, event: BackendEvent) {
    if let Some(events) = events {
        if let Err(error) = events.publish(event) {
            log::warn!("[notia:terminal] evento no emitido: {}", error.message);
        }
    }
}

/// The end of the output the status agent reads, and how many times it
/// changed.
#[derive(Default)]
struct LiveOutput {
    text: String,
    partial: String,
    version: u64,
}

impl LiveOutput {
    fn push(&mut self, chunk: &OutputChunk) {
        self.text.push_str(&chunk.text);
        if self.text.len() > terminal::STATUS_TAIL_CHARS * 4 {
            let keep = self.text.char_indices().rev().nth(terminal::STATUS_TAIL_CHARS).map_or(0, |(index, _)| index);
            self.text.drain(..keep);
        }
        self.partial.clone_from(&chunk.partial);
        self.version += 1;
    }
}

/// The light agent of the live status: after [`terminal::STATUS_FIRST_AFTER`]
/// and then every [`terminal::STATUS_EVERY`] while the command printed more,
/// one call to the configured model without thinking nor tools reads the
/// end of the output (and its previous status) and the status goes out as
/// a `terminal-status` event. A failed or slow call is skipped; nothing is
/// published once the command ended.
fn watch_status(
    app: &AppHandle,
    context: &BackendRequestContext,
    call_id: &str,
    request: &TerminalRequest,
    live: Arc<Mutex<LiveOutput>>,
    done: Arc<AtomicBool>,
    events: Arc<dyn BackendEventSink>,
) {
    let app = app.clone();
    let (library_id, request_id, call_id) = (context.library_id.clone(), context.request_id.clone(), call_id.to_string());
    let (command, shell) = (request.command.clone(), request.shell.label());
    let started = Instant::now();
    let spawned = std::thread::Builder::new().name("notia-terminal-status".into()).spawn(move || {
        let mut next = started + terminal::STATUS_FIRST_AFTER;
        let mut asked_version = None;
        let mut previous: Option<String> = None;
        loop {
            while Instant::now() < next {
                if done.load(Ordering::SeqCst) {
                    return;
                }
                std::thread::sleep(STATUS_POLL);
            }
            next = Instant::now() + terminal::STATUS_EVERY;
            let Some((output, version)) = live.lock().ok().map(|live| (format!("{}{}", live.text, live.partial), live.version)) else {
                return;
            };
            // Nothing new since the last status: it still stands.
            if asked_version == Some(version) {
                continue;
            }
            asked_version = Some(version);
            let user = terminal::status_request(&command, shell, started.elapsed(), &output, previous.as_deref());
            match crate::backend_runtime::complete_text(&app, &library_id, terminal::LIVE_STATUS_SYSTEM, &user, terminal::STATUS_TIMEOUT) {
                Ok(answer) => {
                    let Some(status) = terminal::status_answer(&answer) else { continue };
                    if done.load(Ordering::SeqCst) {
                        return;
                    }
                    publish_event(
                        Some(events.as_ref()),
                        BackendEvent::TerminalStatus { request_id: request_id.clone(), call_id: call_id.clone(), status: status.clone() },
                    );
                    previous = Some(status);
                }
                // Only the code: the message could quote the output.
                Err(error) => log::error!("[notia:terminal] el estado en vivo falló ({:?})", error.code),
            }
        }
    });
    if spawned.is_err() {
        log::error!("[notia:terminal] no se pudo iniciar el estado en vivo");
    }
}

/// Follows a started command until it ends, its time is up or `control`
/// is cancelled, handing its output to `on_output` as it arrives.
fn follow(
    child: &mut Child,
    timeout: Duration,
    control: &RequestControl,
    on_output: &mut dyn FnMut(OutputChunk),
) -> (TerminalOutcome, Duration, Transcript) {
    let output = read_output(child);
    let started = Instant::now();
    let mut decoder = OutputDecoder::default();
    let mut transcript = Transcript::default();
    let mut send = |chunk: Option<OutputChunk>, transcript: &mut Transcript| {
        if let Some(chunk) = chunk {
            transcript.push(&chunk.text);
            on_output(chunk);
        }
    };
    let mut last_flush = Instant::now();
    let mut readers_done = false;
    let mut exited: Option<(Option<i32>, Instant)> = None;
    let outcome = loop {
        if readers_done {
            std::thread::sleep(POLL_INTERVAL);
        } else {
            match output.recv_timeout(POLL_INTERVAL) {
                Ok(bytes) => decoder.push(&bytes),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => readers_done = true,
            }
        }
        if last_flush.elapsed() >= FLUSH_INTERVAL {
            send(decoder.take(), &mut transcript);
            last_flush = Instant::now();
        }
        if exited.is_none() {
            match child.try_wait() {
                Ok(Some(status)) => exited = Some((status.code(), Instant::now())),
                Ok(None) => {}
                Err(_) => exited = Some((None, Instant::now())),
            }
        }
        if let Some((code, at)) = exited {
            if readers_done || at.elapsed() >= DRAIN_AFTER_EXIT {
                break TerminalOutcome::Exited(code);
            }
            continue;
        }
        if control.is_cancelled() {
            stop(child);
            break TerminalOutcome::Cancelled;
        }
        if started.elapsed() >= timeout {
            stop(child);
            break TerminalOutcome::TimedOut;
        }
    };
    // What arrived before the end goes too.
    while let Ok(bytes) = output.try_recv() {
        decoder.push(&bytes);
    }
    send(decoder.finish(), &mut transcript);
    (outcome, started.elapsed(), transcript)
}

/// The folder the command runs in: the one asked for, which must exist;
/// otherwise the library's folder, or the user's home.
/// Measures `request.path` in one pass: what each folder or file directly
/// inside it holds, and the largest files. Links and junctions are not
/// followed, so nothing is counted twice; what cannot be read is counted
/// apart. It stops at its time limit or a cancel with what it measured.
fn scan_disk(request: &terminal::DiskUsageRequest, control: &RequestControl) -> Result<(terminal::DiskUsage, Duration), BackendError> {
    let root = PathBuf::from(&request.path);
    let children = std::fs::read_dir(&root).map_err(|_| BackendError::invalid_input(format!("No se puede leer la carpeta {} en la PC host.", request.path)))?;
    let started = Instant::now();
    let mut usage = terminal::DiskUsage::default();
    let mut seen = 0_u64;
    for child in children {
        let Ok(child) = child else {
            usage.unreadable += 1;
            continue;
        };
        let Ok(kind) = child.file_type() else {
            usage.unreadable += 1;
            continue;
        };
        if kind.is_symlink() {
            continue;
        }
        let mut entry = terminal::DiskEntry { name: child.file_name().to_string_lossy().into_owned(), bytes: 0, files: 0, folder: kind.is_dir() };
        let mut pending = vec![child.path()];
        if !kind.is_dir() {
            pending.clear();
            entry.files = 1;
            entry.bytes = child.metadata().map(|metadata| metadata.len()).unwrap_or(0);
            keep_largest(&mut usage.largest, child.path(), entry.bytes);
        }
        while let Some(folder) = pending.pop() {
            seen += 1;
            if seen % 500 == 0 {
                if control.is_cancelled() {
                    return Err(BackendError::cancelled());
                }
                if started.elapsed() >= terminal::DISK_SCAN_TIMEOUT {
                    usage.partial = true;
                    break;
                }
            }
            let Ok(items) = std::fs::read_dir(&folder) else {
                usage.unreadable += 1;
                continue;
            };
            for item in items {
                let Ok(item) = item else {
                    usage.unreadable += 1;
                    continue;
                };
                match item.file_type() {
                    Ok(kind) if kind.is_symlink() => {}
                    Ok(kind) if kind.is_dir() => pending.push(item.path()),
                    Ok(_) => match item.metadata() {
                        Ok(metadata) => {
                            entry.bytes += metadata.len();
                            entry.files += 1;
                            keep_largest(&mut usage.largest, item.path(), metadata.len());
                        }
                        Err(_) => usage.unreadable += 1,
                    },
                    Err(_) => usage.unreadable += 1,
                }
            }
        }
        usage.entries.push(entry);
        if usage.partial {
            break;
        }
    }
    Ok((usage, started.elapsed()))
}

/// Keeps the `DISK_LARGEST_FILES` largest files seen.
fn keep_largest(largest: &mut Vec<(String, u64)>, path: PathBuf, bytes: u64) {
    if largest.len() < terminal::DISK_LARGEST_FILES {
        largest.push((path.to_string_lossy().into_owned(), bytes));
    } else if let Some(smallest) = largest.iter_mut().min_by_key(|(_, size)| *size).filter(|(_, size)| *size < bytes) {
        *smallest = (path.to_string_lossy().into_owned(), bytes);
    }
}

fn working_folder(app: &AppHandle, library_id: &str, asked: Option<&str>) -> Result<PathBuf, BackendError> {
    if let Some(asked) = asked {
        let path = PathBuf::from(asked);
        if !path.is_dir() {
            return Err(BackendError::invalid_input(format!("La carpeta {asked} no existe en la PC host.")));
        }
        return Ok(path);
    }
    let library = app
        .state::<LibraryBindingRegistry>()
        .lookup(library_id)
        .ok()
        .and_then(|binding| match binding.root {
            Some(LibraryBindingRoot::Desktop { canonical_root }) => Some(canonical_root),
            _ => None,
        })
        .filter(|path| path.is_dir());
    library
        .or_else(|| std::env::var_os(if cfg!(target_os = "windows") { "USERPROFILE" } else { "HOME" }).map(PathBuf::from).filter(|path| path.is_dir()))
        .or_else(|| std::env::current_dir().ok())
        .ok_or_else(|| BackendError::new(BackendErrorCode::Internal, "No hay una carpeta donde correr el comando.", true))
}

fn spawn(request: &TerminalRequest, cwd: &Path) -> Result<Child, BackendError> {
    let launch = request.shell.launch(&request.command);
    let mut command = Command::new(launch.program);
    command
        .args(&launch.args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // Plain output: the chat and Telegram show text, not colours.
        .env("NO_COLOR", "1")
        .env("TERM", "dumb");
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt as _;
        // No console window over the app.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
        if let Some(tail) = &launch.raw_tail {
            command.raw_arg(tail);
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;
        // Its own process group, so a stop reaches what it started.
        command.process_group(0);
    }
    command.spawn().map_err(|error| {
        BackendError::new(
            BackendErrorCode::Internal,
            format!("No se pudo iniciar {} en la PC host ({}).", request.shell.label(), error.kind()),
            true,
        )
    })
}

/// Reads stdout and stderr as they arrive, in one stream; the channel
/// closes once both ended.
fn read_output(child: &mut Child) -> mpsc::Receiver<Vec<u8>> {
    let (sender, receiver) = mpsc::channel();
    let readers: [Option<Box<dyn std::io::Read + Send>>; 2] = [
        child.stdout.take().map(|stream| Box::new(stream) as Box<dyn std::io::Read + Send>),
        child.stderr.take().map(|stream| Box::new(stream) as Box<dyn std::io::Read + Send>),
    ];
    for mut reader in readers.into_iter().flatten() {
        let sender = sender.clone();
        let spawned = std::thread::Builder::new().name("notia-terminal-read".into()).spawn(move || {
            let mut buffer = vec![0; READ_BUFFER];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(read) => {
                        if sender.send(buffer[..read].to_vec()).is_err() {
                            break;
                        }
                    }
                }
            }
        });
        if spawned.is_err() {
            log::error!("[notia:terminal] no se pudo leer la salida del comando");
        }
    }
    receiver
}

/// Stops the shell and everything it started (also a support script).
pub(crate) fn stop(child: &mut Child) {
    let pid = child.id();
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt as _;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let _ = Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(CREATE_NO_WINDOW)
            .status();
    }
    #[cfg(unix)]
    {
        let _ = Command::new("kill").args(["-KILL", &format!("-{pid}")]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status();
    }
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(all(test, target_os = "windows"))]
mod tests {
    use super::*;

    fn run(command: &str, timeout: u64) -> (Value, Vec<String>) {
        let request = TerminalRequest::from_arguments(&serde_json::json!({ "command": command, "timeoutSeconds": timeout })).expect("request");
        let cwd = std::env::temp_dir();
        let mut child = spawn(&request, &cwd).expect("spawn");
        let mut chunks = Vec::new();
        let (outcome, elapsed, transcript) = follow(&mut child, request.timeout, &RequestControl::new(None), &mut |chunk| chunks.push(chunk.text));
        (terminal::tool_result(&request, &cwd.to_string_lossy(), outcome, elapsed, &transcript), chunks)
    }

    #[test]
    fn powershell_prints_utf8_and_keeps_the_native_exit_code() {
        let (result, chunks) = run("Write-Output 'año ñandú'; cmd /c exit 3", 60);
        assert_eq!(chunks.concat(), "año ñandú
");
        assert_eq!(result["output"], "año ñandú\n");
        assert_eq!(result["exitCode"], 3);
        assert_eq!(result["status"], "failed");
    }

    #[test]
    fn a_command_past_its_time_is_stopped_with_what_it_started() {
        let started = Instant::now();
        let (result, _) = run("Write-Output 'empieza'; ping -n 30 127.0.0.1 > $null; Write-Output 'nunca'", 5);
        assert_eq!(result["status"], "timed-out");
        assert!(started.elapsed() < Duration::from_secs(20));
        assert_eq!(result["output"], "empieza\n");
    }

    #[test]
    fn cmd_runs_quoted_commands_untouched() {
        let request = TerminalRequest::from_arguments(&serde_json::json!({ "command": "echo \"hola mundo\"& exit 0", "shell": "cmd" })).expect("request");
        let mut child = spawn(&request, &std::env::temp_dir()).expect("spawn");
        let (outcome, _, transcript) = follow(&mut child, request.timeout, &RequestControl::new(None), &mut |_| {});
        assert_eq!(transcript.for_model(), "\"hola mundo\"
");
        assert_eq!(outcome, TerminalOutcome::Exited(Some(0)));
    }

    #[test]
    fn the_live_output_keeps_its_end_and_counts_changes() {
        let mut live = LiveOutput::default();
        for index in 0..5_000 {
            live.push(&OutputChunk { text: format!("línea {index}\n"), partial: "50%".into() });
        }
        assert_eq!(live.version, 5_000);
        assert!(live.text.ends_with("línea 4999\n"));
        assert!(live.text.chars().count() <= terminal::STATUS_TAIL_CHARS * 4 + 20);
        assert_eq!(live.partial, "50%");
    }

    #[test]
    fn a_cancelled_request_stops_the_command() {
        let request = TerminalRequest::from_arguments(&serde_json::json!({ "command": "ping -n 30 127.0.0.1" })).expect("request");
        let mut child = spawn(&request, &std::env::temp_dir()).expect("spawn");
        let control = RequestControl::new(None);
        let cancel = control.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(1));
            cancel.cancel();
        });
        let started = Instant::now();
        let (outcome, _, _) = follow(&mut child, request.timeout, &control, &mut |_| {});
        assert_eq!(outcome, TerminalOutcome::Cancelled);
        assert!(started.elapsed() < Duration::from_secs(15));
    }
}
