//! Node.js packages Notia installs with the host PC's npm the first time a
//! tool needs them (Playwright for the browser, Baileys for WhatsApp): a
//! folder of the app data with Notia's script, a minimal `package.json` and
//! the pinned packages. Neither is bundled with the app; without Node.js or
//! npm the tool answers how to install them.

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::backend::{BackendError, BackendErrorCode};

const INSTALL_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const POLL_INTERVAL: Duration = Duration::from_millis(100);

/// A script of Notia and the exact packages it runs with.
pub(crate) struct NodePackage {
    /// Name of the folder's `package.json`.
    pub(crate) name: &'static str,
    pub(crate) script_name: &'static str,
    pub(crate) script: &'static str,
    /// `(package, exact version)`; the first one tells whether the folder
    /// is installed.
    pub(crate) packages: &'static [(&'static str, &'static str)],
    pub(crate) min_node_major: u32,
    /// What needs Node.js, for the messages («El navegador», «WhatsApp»).
    pub(crate) needed_by: &'static str,
}

impl NodePackage {
    fn specs(&self) -> Vec<String> {
        self.packages.iter().map(|(name, version)| format!("{name}@{version}")).collect()
    }

    pub(crate) fn node_missing(&self) -> BackendError {
        BackendError::new(
            BackendErrorCode::Unsupported,
            format!(
                "{} necesita Node.js {} o más nuevo instalado en la PC host (nodejs.org); después probá de nuevo.",
                self.needed_by, self.min_node_major
            ),
            false,
        )
    }

    fn install_failed(&self, folder: &Path, code: Option<i32>) -> BackendError {
        let code = code.map(|code| format!(" (código {code})")).unwrap_or_default();
        BackendError::new(
            BackendErrorCode::Unsupported,
            format!(
                "No se pudo instalar {} con npm{code}. Revisá la conexión, o instalalo a mano en la PC host con «npm install {}» dentro de {}.",
                self.packages.first().map_or("", |(name, _)| *name),
                self.specs().join(" "),
                folder.display()
            ),
            true,
        )
    }

    /// Writes the script when it changed and installs the packages when
    /// they are missing or another version.
    pub(crate) fn install_into(&self, folder: &Path) -> Result<(), BackendError> {
        let storage = |_| BackendError::new(BackendErrorCode::Storage, format!("No se pudo preparar la carpeta de {}.", self.needed_by), true);
        std::fs::create_dir_all(folder).map_err(storage)?;
        let script = folder.join(self.script_name);
        if std::fs::read_to_string(&script).ok().as_deref() != Some(self.script) {
            std::fs::write(&script, self.script).map_err(storage)?;
        }
        let manifest = folder.join("package.json");
        if !manifest.is_file() {
            std::fs::write(&manifest, format!("{{\n  \"name\": \"{}\",\n  \"private\": true\n}}\n", self.name)).map_err(storage)?;
        }
        if self.is_installed(folder) {
            return Ok(());
        }
        self.check_node()?;
        self.run_npm_install(folder)?;
        if !self.is_installed(folder) {
            return Err(self.install_failed(folder, None));
        }
        Ok(())
    }

    fn is_installed(&self, folder: &Path) -> bool {
        self.packages.iter().all(|(name, version)| installed_version(folder, name).as_deref() == Some(*version))
    }

    fn check_node(&self) -> Result<(), BackendError> {
        let mut command = Command::new("node");
        command.arg("--version").stdin(Stdio::null()).stderr(Stdio::null());
        hide_console(&mut command);
        let output = command.output().map_err(|_| self.node_missing())?;
        let version = String::from_utf8_lossy(&output.stdout);
        let major = version.trim().trim_start_matches('v').split('.').next().and_then(|major| major.parse::<u32>().ok());
        match major {
            Some(major) if output.status.success() && major >= self.min_node_major => Ok(()),
            _ => Err(self.node_missing()),
        }
    }

    fn run_npm_install(&self, folder: &Path) -> Result<(), BackendError> {
        let mut command = Command::new(if cfg!(target_os = "windows") { "npm.cmd" } else { "npm" });
        command
            .args(["install", "--no-audit", "--no-fund", "--save-exact"])
            .args(self.specs())
            .current_dir(folder)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        hide_console(&mut command);
        let mut child = command.spawn().map_err(|_| self.node_missing())?;
        let started = Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(status)) if status.success() => return Ok(()),
                Ok(Some(status)) => return Err(self.install_failed(folder, status.code())),
                Ok(None) if started.elapsed() >= INSTALL_TIMEOUT => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(self.install_failed(folder, None));
                }
                Ok(None) => std::thread::sleep(POLL_INTERVAL),
                Err(_) => return Err(self.install_failed(folder, None)),
            }
        }
    }
}

fn installed_version(folder: &Path, package: &str) -> Option<String> {
    let mut manifest = folder.join("node_modules");
    for part in package.split('/') {
        manifest.push(part);
    }
    let manifest = std::fs::read_to_string(manifest.join("package.json")).ok()?;
    serde_json::from_str::<Value>(&manifest).ok()?.get("version")?.as_str().map(str::to_string)
}

pub(crate) fn hide_console(command: &mut Command) {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt as _;
        // No console window over the app; the process opens its own windows.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(target_os = "windows"))]
    let _ = command;
}
