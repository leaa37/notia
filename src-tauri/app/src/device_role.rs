//! Which computer runs the library's services (Telegram, WhatsApp, the
//! webhook, the autonomous agent, the AI actions and tasks, the Google sync
//! and the backups): the one with the most processing among the computers
//! that see each other (`backend::connection::runs_services_among`).
//!
//! - A host notes each computer that syncs with it (`note_client`, from the
//!   sync's manifest, which only the Owner's session reaches); a client
//!   notes its host on each health check (`note_host`).
//! - A computer counts while it was seen in the last [`SEEN_FOR`]; after
//!   that the others decide without it, and a computer alone runs them.
//! - A client that has more processing than its host takes the services
//!   only once the host knows it (its last sync is recent): until then the
//!   host keeps them, so nobody answers Telegram twice.
//! - A client waits for its first check of the host (at most
//!   [`FIRST_CHECK`]) before taking them at start.
//! - Android never runs them; the computer of the library does.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::backend::connection::{runs_services_among, DevicePower, RunMode};
use crate::host::{AppHandle, Manager};

/// A computer not seen for this long is away.
const SEEN_FOR: Duration = Duration::from_secs(90);
/// How long a client waits at start for the first check of its host.
const FIRST_CHECK: Duration = Duration::from_secs(20);

pub(crate) struct DeviceRoleState {
    started: Instant,
    /// Computers that synced with this host, by id.
    clients: Mutex<HashMap<String, (DevicePower, Instant)>>,
    /// The host of this client, as it last answered.
    host: Mutex<Option<(DevicePower, Instant)>>,
    /// When this client last synced: the host knows it since then.
    synced: Mutex<Option<Instant>>,
    host_checked: AtomicBool,
}

impl Default for DeviceRoleState {
    fn default() -> Self {
        Self {
            started: Instant::now(),
            clients: Mutex::default(),
            host: Mutex::default(),
            synced: Mutex::default(),
            host_checked: AtomicBool::new(false),
        }
    }
}

fn state(app: &AppHandle) -> Option<crate::host::State<'static, DeviceRoleState>> {
    app.try_state::<DeviceRoleState>()
}

fn recent(at: Instant) -> bool {
    at.elapsed() < SEEN_FOR
}

/// How much this computer processes, as it tells the others.
pub(crate) fn local_power(app: &AppHandle) -> DevicePower {
    let settings = crate::connection::settings(app);
    DevicePower {
        device_id: settings.device_id,
        threads: std::thread::available_parallelism().map_or(1, |threads| threads.get() as u32),
        memory_mib: total_memory_mib(),
        host: settings.mode == RunMode::Host,
    }
}

#[cfg(target_os = "windows")]
fn total_memory_mib() -> u64 {
    use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};

    let mut status = MEMORYSTATUSEX { dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
    match unsafe { GlobalMemoryStatusEx(&mut status) } {
        Ok(()) => status.ullTotalPhys / (1024 * 1024),
        Err(_) => 0,
    }
}

#[cfg(target_os = "linux")]
fn total_memory_mib() -> u64 {
    std::fs::read_to_string("/proc/meminfo")
        .ok()
        .and_then(|text| {
            let line = text.lines().find(|line| line.starts_with("MemTotal:"))?;
            line.split_whitespace().nth(1)?.parse::<u64>().ok()
        })
        .map_or(0, |kib| kib / 1024)
}

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
fn total_memory_mib() -> u64 {
    0
}

/// A computer synced with this host (Android does not count).
pub(crate) fn note_client(app: &AppHandle, power: DevicePower) {
    let Some(state) = state(app) else {
        return;
    };
    if power.device_id.trim().is_empty() {
        return;
    }
    if let Ok(mut clients) = state.clients.lock() {
        clients.retain(|_, (_, at)| recent(*at));
        clients.insert(power.device_id.clone(), (power, Instant::now()));
    };
}

/// The host of this client answered its health check.
pub(crate) fn note_host(app: &AppHandle, power: DevicePower) {
    if let Some(state) = state(app) {
        if let Ok(mut host) = state.host.lock() {
            *host = Some((power, Instant::now()));
        };
    }
}

/// The monitor of this client checked its host (whether it answered or not).
pub(crate) fn host_checked(app: &AppHandle) {
    if let Some(state) = state(app) {
        state.host_checked.store(true, Ordering::SeqCst);
    }
}

/// This client synced: its host knows it now.
pub(crate) fn note_synced(app: &AppHandle) {
    if let Some(state) = state(app) {
        if let Ok(mut synced) = state.synced.lock() {
            *synced = Some(Instant::now());
        };
    }
}

/// Whether this device runs the library's services now. It also keeps the
/// guard of Telegram in line: a device that does not run them never calls
/// the Telegram API.
pub(crate) fn runs_services(app: &AppHandle) -> bool {
    let runs = decide(app);
    crate::services::telegram_service::set_client_device(!runs);
    runs
}

fn decide(app: &AppHandle) -> bool {
    if cfg!(any(target_os = "android", target_os = "ios")) {
        return false;
    }
    let client = crate::connection::is_client(app);
    let Some(state) = state(app) else {
        return !client;
    };
    let local = local_power(app);
    if !client {
        let clients: Vec<DevicePower> = state
            .clients
            .lock()
            .map(|clients| clients.values().filter(|(_, at)| recent(*at)).map(|(power, _)| power.clone()).collect())
            .unwrap_or_default();
        return runs_services_among(&local, &clients);
    }
    if !state.host_checked.load(Ordering::SeqCst) && state.started.elapsed() < FIRST_CHECK {
        return false;
    }
    let host = state.host.lock().ok().and_then(|host| host.clone()).filter(|(_, at)| recent(*at));
    match host {
        None => true,
        Some((host, _)) => {
            let known = state.synced.lock().ok().and_then(|synced| *synced).is_some_and(recent);
            runs_services_among(&local, &[host]) && known
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::connection::ConnectionSettings;
    use crate::host::{AppContext, AppPaths, HostPorts};

    fn app(mode: RunMode) -> (AppContext, std::path::PathBuf) {
        let directory = std::env::temp_dir().join(format!("notia-role-{}", uuid::Uuid::new_v4()));
        let app = AppContext::new(AppPaths::new(Some(directory.clone()), None), HostPorts::default());
        app.manage(crate::connection::ConnectionState::default());
        app.manage(DeviceRoleState::default());
        let settings = ConnectionSettings { mode, host_address: "casa:6000".into(), device_id: "este".into(), ..ConnectionSettings::default() };
        crate::connection::store(&app, &settings).expect("settings");
        (app, directory)
    }

    fn other(threads: u32) -> DevicePower {
        DevicePower { device_id: "otro".into(), threads, memory_mib: 0, host: false }
    }

    #[test]
    fn a_host_gives_the_services_to_a_stronger_computer_that_syncs() {
        let (host, directory) = app(RunMode::Host);
        assert!(decide(&host), "alone");
        note_client(&host, other(1));
        assert!(decide(&host), "a weaker one");
        note_client(&host, other(u32::MAX));
        assert!(!decide(&host), "a stronger one");
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn a_stronger_client_takes_the_services_once_its_host_knows_it() {
        let (client, directory) = app(RunMode::Client);
        assert!(!decide(&client), "before the first check");
        host_checked(&client);
        assert!(decide(&client), "the host does not answer");
        note_host(&client, other(u32::MAX));
        assert!(!decide(&client), "a stronger host");
        note_host(&client, other(1));
        assert!(!decide(&client), "the host does not know it yet");
        note_synced(&client);
        assert!(decide(&client), "a weaker host that knows it");
        let _ = std::fs::remove_dir_all(directory);
    }
}
