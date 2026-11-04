//! The Python support scripts of `.agent/scripts` on the host PC: reads the
//! folder (the agent's scripts and the person's), shows them to the model
//! in the description of `run_agent_script`, saves the scripts the agent
//! writes and runs them with the PC's Python 3, in the library's folder,
//! with their arguments as JSON on stdin. Headers, arguments, files and
//! what the model reads live in `backend::agent_scripts`.
//!
//! Only a desktop host runs scripts, like the terminal: Android has no
//! Python and a client device works against its host. Logs say how a run
//! ended, never its arguments or output.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{mpsc, Mutex};
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::backend::agent_scripts::{self as scripts, AgentScript, RunEnd, RunRequest, SaveRequest};
use crate::backend::{BackendError, BackendErrorCode, BackendRequestContext, RequestControl, ToolDefinition};
use crate::host::{AppHandle, Manager};
use crate::library_registry::{LibraryBindingRegistry, LibraryBindingRoot};
use crate::mobile_directory_picker::AndroidDirectoryPickerState;

/// What a run keeps of stdout and of stderr; the model reads their end.
const MAX_CAPTURED_BYTES: usize = 2 * 1024 * 1024;
const POLL_INTERVAL: Duration = Duration::from_millis(100);

/// Whether this device runs the scripts.
pub(crate) fn is_available(app: &AppHandle) -> bool {
    crate::terminal::is_available(app)
}

fn unavailable(_app: &AppHandle) -> BackendError {
    BackendError::new(BackendErrorCode::Unsupported, "Los scripts de soporte corren solo en las computadoras con Notia, no en Android.", false)
}

fn library_root(app: &AppHandle, library_id: &str) -> Result<PathBuf, BackendError> {
    app.state::<LibraryBindingRegistry>()
        .lookup(library_id)
        .ok()
        .and_then(|binding| match binding.root {
            Some(LibraryBindingRoot::Desktop { canonical_root }) => Some(canonical_root),
            _ => None,
        })
        .filter(|root| root.is_dir())
        .ok_or_else(|| BackendError::new(BackendErrorCode::Unsupported, "La biblioteca no está en una carpeta de esta PC: los scripts no pueden correr.", false))
}

fn scripts_folder(root: &Path) -> PathBuf {
    scripts::SCRIPTS_DIRECTORY.split('/').fold(root.to_path_buf(), |path, part| path.join(part))
}

/// The scripts of the folder, by name, and the files that are not one
/// (with why).
fn load(app: &AppHandle, library_id: &str) -> (Vec<AgentScript>, Vec<String>) {
    let Ok(root) = library_root(app, library_id) else {
        return (Vec::new(), Vec::new());
    };
    let Ok(entries) = std::fs::read_dir(scripts_folder(&root)) else {
        return (Vec::new(), Vec::new());
    };
    let mut files = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_file())
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|file| file.to_ascii_lowercase().ends_with(".py"))
        .collect::<Vec<_>>();
    files.sort();
    let (mut found, mut unreadable) = (Vec::new(), Vec::new());
    for file in files {
        if found.len() >= scripts::MAX_SCRIPTS {
            unreadable.push(format!("{file}: hay más de {} scripts y este quedó afuera", scripts::MAX_SCRIPTS));
            continue;
        }
        let path = scripts_folder(&root).join(&file);
        if std::fs::metadata(&path).map_or(true, |metadata| metadata.len() as usize > scripts::MAX_SCRIPT_BYTES) {
            unreadable.push(format!("{file}: pesa más de {} KB", scripts::MAX_SCRIPT_BYTES / 1024));
            continue;
        }
        match std::fs::read_to_string(&path) {
            Ok(content) => match scripts::parse_script(&file, &content) {
                Some(script) => found.push(script),
                None => unreadable.push(format!("{file}: el nombre va en minúsculas, con letras, números y _, empezando con una letra")),
            },
            Err(_) => unreadable.push(format!("{file}: no se pudo leer como texto UTF-8")),
        }
    }
    (found, unreadable)
}

/// Lists the folder's scripts in the description of `run_agent_script`,
/// so the model sees them like tools of their own.
pub(crate) fn describe_scripts(app: &AppHandle, library_id: &str, tools: &mut [ToolDefinition]) {
    let Some(tool) = tools.iter_mut().find(|tool| tool.name == scripts::RUN_TOOL) else { return };
    if !is_available(app) {
        return;
    }
    let (found, _) = load(app, library_id);
    tool.description = scripts::run_tool_description(&tool.description, &found);
}

/// Keeps the folder's guide as Notia ships it, `#System` (the person reads
/// it but cannot change it): written again when it is missing or differs.
/// Whether it was missing.
pub(crate) fn ensure_readme(documents: &crate::library_documents::Documents<'_>) -> Result<bool, BackendError> {
    let current = documents.read(scripts::README_PATH)?;
    documents.write(scripts::README_PATH, current.as_deref(), &crate::backend::agent_workspace::with_system_context(scripts::README))?;
    Ok(current.is_none())
}

/// The confirmation of a script the agent saves: where and the whole file.
pub(crate) fn preview(app: &AppHandle, context: &BackendRequestContext, arguments: &Value) -> Result<(String, String), BackendError> {
    if !is_available(app) {
        return Err(unavailable(app));
    }
    let request = SaveRequest::from_arguments(arguments)?;
    let exists = scripts_folder(&library_root(app, &context.library_id)?).join(format!("{}.py", request.name)).is_file();
    Ok((request.summary(exists), request.detail()))
}

pub(crate) fn execute_tool(app: &AppHandle, control: &RequestControl, context: &BackendRequestContext, name: &str, arguments: &Value) -> Result<Value, BackendError> {
    if !is_available(app) {
        return Err(unavailable(app));
    }
    match name {
        scripts::LIST_TOOL => {
            let (found, unreadable) = load(app, &context.library_id);
            Ok(scripts::list_result(&found, &unreadable))
        }
        scripts::SAVE_TOOL => save(app, &context.library_id, &SaveRequest::from_arguments(arguments)?),
        scripts::RUN_TOOL => run(app, control, &context.library_id, &RunRequest::from_arguments(arguments)?),
        _ => Err(BackendError::invalid_input("La herramienta no es de los scripts de soporte.")),
    }
}

fn save(app: &AppHandle, library_id: &str, request: &SaveRequest) -> Result<Value, BackendError> {
    let registry = app.state::<LibraryBindingRegistry>();
    let picker = app.state::<AndroidDirectoryPickerState>();
    crate::filesystem::commands::ensure_library_folders(registry.inner(), picker.inner(), library_id, scripts::SCRIPTS_DIRECTORY)?;
    let path = scripts::script_path(&request.name);
    let replaced = crate::library_documents::with_documents(app, library_id, |documents| {
        let current = documents.read(&path)?;
        documents.write(&path, current.as_deref(), &request.source())?;
        Ok(current.is_some())
    })?;
    log::info!("[notia:scripts] script {}", if replaced { "reemplazado" } else { "creado" });
    Ok(scripts::saved_result(request, replaced))
}

fn run(app: &AppHandle, control: &RequestControl, library_id: &str, request: &RunRequest) -> Result<Value, BackendError> {
    let root = library_root(app, library_id)?;
    let (found, _) = load(app, library_id);
    let script = found.iter().find(|script| script.name == request.script).ok_or_else(|| {
        BackendError::invalid_input(format!("No hay un script {} en .agent/scripts: mirá list_agent_scripts.", request.script))
    })?;
    script.check_arguments(&request.arguments)?;
    let python = python()?;
    let folder = scripts_folder(&root);
    let mut command = Command::new(python.program);
    command
        .args(python.args)
        .args(["-X", "utf8"])
        .arg(folder.join(&script.file))
        .current_dir(&root)
        .env("NOTIA_LIBRARY", &root)
        .env("NOTIA_SCRIPTS", &folder)
        .env("PYTHONIOENCODING", "utf-8")
        .env("PYTHONUTF8", "1")
        // No __pycache__ in the library.
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    crate::node_packages::hide_console(&mut command);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;
        // Its own process group, so a stop reaches what it started.
        command.process_group(0);
    }
    let started = Instant::now();
    let mut child = command
        .spawn()
        .map_err(|_| BackendError::new(BackendErrorCode::Internal, "No se pudo iniciar Python en la PC host.", true))?;
    if let Some(mut stdin) = child.stdin.take() {
        // A script that does not read its arguments closes stdin first.
        let _ = stdin.write_all(request.arguments.to_string().as_bytes());
    }
    let stdout = capture(child.stdout.take());
    let stderr = capture(child.stderr.take());
    let end = wait(&mut child, request.timeout, control);
    let (stdout, stderr) = (collected(stdout), collected(stderr));
    let Some(end) = end else {
        log::info!("[notia:scripts] script cancelado");
        return Err(BackendError::cancelled());
    };
    let elapsed = started.elapsed();
    log::info!("[notia:scripts] script terminado ({:?}) en {} s", end, elapsed.as_secs());
    Ok(scripts::run_result(&script.name, end, elapsed, &stdout, &stderr))
}

/// Waits for the script until it ends, its time is up (it is stopped) or
/// the request is cancelled (`None`).
fn wait(child: &mut Child, timeout: Duration, control: &RequestControl) -> Option<RunEnd> {
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(RunEnd::Exited(status.code())),
            Ok(None) if control.is_cancelled() => {
                crate::terminal::stop(child);
                return None;
            }
            Ok(None) if Instant::now() >= deadline => {
                crate::terminal::stop(child);
                return Some(RunEnd::TimedOut);
            }
            Ok(None) => std::thread::sleep(POLL_INTERVAL),
            Err(_) => return Some(RunEnd::Exited(None)),
        }
    }
}

/// Reads a stream on its own thread, keeping its first
/// [`MAX_CAPTURED_BYTES`].
fn capture(stream: Option<impl std::io::Read + Send + 'static>) -> mpsc::Receiver<Vec<u8>> {
    let (sender, receiver) = mpsc::channel();
    if let Some(mut stream) = stream {
        let _ = std::thread::Builder::new().name("notia-script-read".into()).spawn(move || {
            let mut kept = Vec::new();
            let mut buffer = [0; 8 * 1024];
            loop {
                match stream.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(read) => {
                        let room = MAX_CAPTURED_BYTES.saturating_sub(kept.len());
                        kept.extend_from_slice(&buffer[..read.min(room)]);
                    }
                }
            }
            let _ = sender.send(kept);
        });
    }
    receiver
}

fn collected(receiver: mpsc::Receiver<Vec<u8>>) -> String {
    // A process the script left running may keep the stream open.
    let bytes = receiver.recv_timeout(Duration::from_secs(2)).unwrap_or_default();
    String::from_utf8_lossy(&bytes).into_owned()
}

/// The Python 3 of the PC and how it is started.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Python {
    program: &'static str,
    args: &'static [&'static str],
}

/// Finds Python 3 the first time it is needed; a Python installed later
/// is found on the next run, since only a success is kept.
fn python() -> Result<Python, BackendError> {
    static FOUND: Mutex<Option<Python>> = Mutex::new(None);
    if let Some(found) = FOUND.lock().ok().and_then(|found| *found) {
        return Ok(found);
    }
    let candidates: &[Python] = if cfg!(target_os = "windows") {
        // `python3` on Windows is often the Microsoft Store's shortcut, which
        // fails `--version`.
        &[Python { program: "py", args: &["-3"] }, Python { program: "python", args: &[] }, Python { program: "python3", args: &[] }]
    } else {
        &[Python { program: "python3", args: &[] }, Python { program: "python", args: &[] }]
    };
    let found = candidates.iter().copied().find(|candidate| is_python3(*candidate)).ok_or_else(|| {
        BackendError::new(
            BackendErrorCode::Unsupported,
            "Los scripts de soporte necesitan Python 3 instalado en la PC host (python.org, con «Add python.exe to PATH»); después probá de nuevo.",
            false,
        )
    })?;
    if let Ok(mut kept) = FOUND.lock() {
        *kept = Some(found);
    }
    Ok(found)
}

fn is_python3(candidate: Python) -> bool {
    let mut command = Command::new(candidate.program);
    command.args(candidate.args).arg("--version").stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    crate::node_packages::hide_console(&mut command);
    command.output().is_ok_and(|output| {
        let text = format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
        output.status.success() && text.trim_start().starts_with("Python 3")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs a real script with the PC's Python: its arguments arrive on
    /// stdin, it runs in the given folder and its JSON comes back.
    #[test]
    #[ignore = "needs Python 3 on the PC"]
    fn a_script_runs_with_its_arguments_and_returns_its_json() {
        let python = python().expect("python");
        let folder = std::env::temp_dir().join(format!("notia-script-{}", std::process::id()));
        std::fs::create_dir_all(&folder).expect("folder");
        let request = SaveRequest::from_arguments(&serde_json::json!({
            "name": "sumar",
            "description": "Suma números.",
            "parameters": [{ "name": "numeros", "type": "array", "required": true }],
            "code": "import json, os, sys\nargs = json.load(sys.stdin)\nprint(json.dumps({'total': sum(args['numeros']), 'carpeta': os.environ['NOTIA_LIBRARY'], 'acá': 'ñ'}))\n",
        }))
        .expect("request");
        std::fs::write(folder.join("sumar.py"), request.source()).expect("script");
        let script = scripts::parse_script("sumar.py", &request.source()).expect("parsed");
        let mut child = Command::new(python.program)
            .args(python.args)
            .args(["-X", "utf8"])
            .arg(folder.join("sumar.py"))
            .current_dir(&folder)
            .env("NOTIA_LIBRARY", &folder)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn");
        child.stdin.take().expect("stdin").write_all(br#"{"numeros":[1,2,3.5]}"#).expect("arguments");
        let (stdout, stderr) = (capture(child.stdout.take()), capture(child.stderr.take()));
        let end = wait(&mut child, Duration::from_secs(30), &RequestControl::new(None)).expect("ended");
        let result = scripts::run_result(&script.name, end, Duration::ZERO, &collected(stdout), &collected(stderr));
        assert_eq!(result["ok"], true, "{result}");
        assert_eq!(result["result"]["total"], 6.5);
        assert_eq!(result["result"]["acá"], "ñ");
        assert_eq!(result["result"]["carpeta"], folder.to_string_lossy().as_ref());
        // A script that runs too long is stopped.
        std::fs::write(folder.join("lento.py"), "import time\ntime.sleep(30)\n").expect("slow");
        let mut slow = Command::new(python.program).args(python.args).arg(folder.join("lento.py")).stdin(Stdio::null()).spawn().expect("spawn");
        assert_eq!(wait(&mut slow, Duration::from_millis(500), &RequestControl::new(None)), Some(RunEnd::TimedOut));
        let _ = std::fs::remove_dir_all(&folder);
    }
}
