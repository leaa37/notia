//! How this installation runs (a device setting, not a library one):
//!
//! - **Host**: it keeps the library and serves it on a port to the devices
//!   that sync with it (Windows, Linux and macOS; Android has no server).
//! - **Client**: it keeps a copy of a host's library and works on it as
//!   its own, with or without connection; the copy and the host sync
//!   whenever they see each other.
//!
//! The library's services (Telegram, WhatsApp, the webhook, the autonomous
//! agent, the AI actions and tasks, the Google sync and the backups) run on
//! one computer only: the one with the most processing among the computers
//! that see each other ([`runs_services_among`]). Android never runs them.
//!
//! The adapter stores these settings in the app data folder; the
//! certificate the client trusts is pinned on the first connection. On
//! Android the copy lives in a folder the person chooses (SAF).

use serde::{Deserialize, Serialize};

use crate::error::BackendError;

pub const DEFAULT_HOST_PORT: u16 = 52_480;
/// Version of the API between a client and its host (`/api/health`).
pub const HOST_PROTOCOL_VERSION: u16 = 1;
const MIN_PORT: u16 = 1_024;
const MAX_HOST_CHARS: usize = 253;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RunMode {
    #[default]
    Host,
    Client,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionSettings {
    #[serde(default)]
    pub mode: RunMode,
    /// Id of this installation, made on first use. The computers that see
    /// each other use it to break a tie of processing.
    #[serde(default)]
    pub device_id: String,
    /// `host:port` of the host, as the person wrote it (normalized).
    #[serde(default)]
    pub host_address: String,
    /// Port this installation listens on as a host.
    #[serde(default = "default_port")]
    pub port: u16,
    /// SHA-256 of the host certificate trusted on the first connection
    /// (hex); a different certificate is refused until the address changes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_certificate: Option<String>,
    /// Folder of the copy chosen on Android (a SAF tree). Desktop keeps
    /// the copy in the app data folder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub copy_folder: Option<CopyFolder>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CopyFolder {
    /// The tree URI of the grant, kept as received (opaque).
    pub uri: String,
    /// Name of the folder, to show.
    pub name: String,
}

fn default_port() -> u16 {
    DEFAULT_HOST_PORT
}

impl Default for ConnectionSettings {
    fn default() -> Self {
        Self {
            mode: RunMode::Host,
            device_id: String::new(),
            host_address: String::new(),
            port: DEFAULT_HOST_PORT,
            host_certificate: None,
            copy_folder: None,
        }
    }
}

/// A host address split into host and port.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostAddress {
    pub host: String,
    pub port: u16,
}

impl HostAddress {
    /// `host:port`, with brackets around an IPv6 address.
    pub fn display(&self) -> String {
        if self.host.contains(':') { format!("[{}]:{}", self.host, self.port) } else { format!("{}:{}", self.host, self.port) }
    }

    /// Base URL of the host's API.
    pub fn base_url(&self) -> String {
        format!("https://{}", self.display())
    }
}

fn invalid_address() -> BackendError {
    BackendError::invalid_input("Escribí la dirección del host como equipo:puerto, por ejemplo 192.168.0.10:52480.")
}

/// Parses `equipo`, `equipo:puerto` or `[ipv6]:puerto` (without scheme or
/// path); the port defaults to 52480.
pub fn parse_host_address(text: &str) -> Result<HostAddress, BackendError> {
    let text = text.trim();
    let text = text.strip_prefix("https://").unwrap_or(text).trim_end_matches('/');
    if text.is_empty() || text.contains(['/', '?', '#', '@', ' ']) {
        return Err(invalid_address());
    }
    let (host, port) = if let Some(rest) = text.strip_prefix('[') {
        let (host, after) = rest.split_once(']').ok_or_else(invalid_address)?;
        let port = match after.strip_prefix(':') {
            Some(port) => Some(port),
            None if after.is_empty() => None,
            None => return Err(invalid_address()),
        };
        (host.to_string(), port)
    } else {
        match text.rsplit_once(':') {
            Some((host, port)) if !host.contains(':') => (host.to_string(), Some(port)),
            Some(_) => return Err(invalid_address()),
            None => (text.to_string(), None),
        }
    };
    let port = match port {
        Some(port) => port.parse::<u16>().ok().filter(|port| *port >= MIN_PORT).ok_or_else(invalid_address)?,
        None => DEFAULT_HOST_PORT,
    };
    let valid_host = !host.is_empty()
        && host.chars().count() <= MAX_HOST_CHARS
        && host.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':' | '%'));
    if !valid_host {
        return Err(invalid_address());
    }
    Ok(HostAddress { host: host.to_ascii_lowercase(), port })
}

/// Settings as stored, with a valid port and a normalized address. A pin
/// that belongs to another address is dropped by the adapter on change.
pub fn normalize_connection(settings: ConnectionSettings) -> ConnectionSettings {
    let host_address = parse_host_address(&settings.host_address).map(|address| address.display()).unwrap_or_default();
    ConnectionSettings {
        port: if settings.port >= MIN_PORT { settings.port } else { DEFAULT_HOST_PORT },
        host_certificate: settings
            .host_certificate
            .filter(|pin| pin.len() == 64 && pin.chars().all(|c| c.is_ascii_hexdigit()) && !host_address.is_empty()),
        copy_folder: settings.copy_folder.filter(|folder| crate::AndroidTreeUriDto::new(&folder.uri).is_ok()),
        host_address,
        ..settings
    }
}

/// How much a computer can process, as it reports itself to the others.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DevicePower {
    pub device_id: String,
    /// Threads the processor runs at once.
    pub threads: u32,
    /// Physical memory, in MiB (0 when the system does not say).
    pub memory_mib: u64,
    /// The computer serves the library (Host mode).
    #[serde(default)]
    pub host: bool,
}

impl DevicePower {
    /// Processing first (threads, then memory); on a tie the computer in
    /// Host mode, and last the id, so every computer decides the same.
    fn rank(&self) -> (u32, u64, bool, &str) {
        (self.threads, self.memory_mib, self.host, self.device_id.as_str())
    }
}

/// Whether `local` runs the library's services among the computers it
/// sees now (`others`, without itself): it does unless one of them has
/// more processing. A computer that sees no other one runs them.
pub fn runs_services_among(local: &DevicePower, others: &[DevicePower]) -> bool {
    others
        .iter()
        .filter(|other| other.device_id != local.device_id)
        .all(|other| other.rank() < local.rank())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_addresses_are_parsed_with_their_port() {
        assert_eq!(parse_host_address(" 192.168.0.10:52480 ").expect("ip").display(), "192.168.0.10:52480");
        assert_eq!(parse_host_address("Notebook-Gabi").expect("name"), HostAddress { host: "notebook-gabi".into(), port: DEFAULT_HOST_PORT });
        assert_eq!(parse_host_address("https://casa.local:6000/").expect("scheme").display(), "casa.local:6000");
        assert_eq!(parse_host_address("[fe80::1]:6000").expect("ipv6").display(), "[fe80::1]:6000");
        assert_eq!(parse_host_address("[fe80::1]").expect("ipv6").base_url(), "https://[fe80::1]:52480");
        for bad in ["", "casa:80", "casa:puerto", "a/b", "fe80::1", "user@casa", "casa:70000"] {
            assert!(parse_host_address(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn stored_settings_are_normalized() {
        let settings = normalize_connection(ConnectionSettings {
            mode: RunMode::Client,
            device_id: "equipo".into(),
            host_address: "Casa:6000".into(),
            port: 80,
            host_certificate: Some("ab".repeat(32)),
            copy_folder: Some(CopyFolder { uri: "content://com.android.externalstorage.documents/tree/primary%3ACopia".into(), name: "Copia".into() }),
        });
        assert!(settings.copy_folder.is_some());
        let bad_folder = normalize_connection(ConnectionSettings {
            copy_folder: Some(CopyFolder { uri: "file:///sdcard/Copia".into(), name: "Copia".into() }),
            ..settings.clone()
        });
        assert_eq!(bad_folder.copy_folder, None);
        assert_eq!(settings.host_address, "casa:6000");
        assert_eq!(settings.port, DEFAULT_HOST_PORT);
        assert!(settings.host_certificate.is_some());
        let without_host = normalize_connection(ConnectionSettings { host_address: "no válido".into(), ..settings.clone() });
        assert_eq!(without_host.host_address, "");
        assert_eq!(without_host.host_certificate, None);
        let parsed: ConnectionSettings = serde_json::from_str("{}").expect("defaults");
        assert_eq!(parsed, ConnectionSettings::default());
        // A file from before keeps its mode; the old kind of client is ignored.
        let older: ConnectionSettings = serde_json::from_str(r#"{"mode":"client","clientKind":"remote","hostAddress":"casa:6000"}"#).expect("older");
        assert_eq!((older.mode, older.host_address.as_str()), (RunMode::Client, "casa:6000"));
    }

    fn power(device_id: &str, threads: u32, memory_mib: u64, host: bool) -> DevicePower {
        DevicePower { device_id: device_id.into(), threads, memory_mib, host }
    }

    #[test]
    fn the_computer_with_the_most_processing_runs_the_services() {
        let desktop = power("pc", 24, 32_768, true);
        let laptop = power("notebook", 8, 16_384, false);
        assert!(runs_services_among(&desktop, &[laptop.clone()]));
        assert!(!runs_services_among(&laptop, &[desktop.clone()]));
        // Alone, each computer runs them.
        assert!(runs_services_among(&laptop, &[]));
        // Memory breaks a tie of threads, then Host mode, then the id.
        let bigger = power("notebook", 24, 65_536, false);
        assert!(runs_services_among(&bigger, &[desktop.clone()]) && !runs_services_among(&desktop, &[bigger]));
        let twin = power("otra", 24, 32_768, false);
        assert!(runs_services_among(&desktop, &[twin.clone()]) && !runs_services_among(&twin, &[desktop.clone()]));
        let (a, b) = (power("a", 4, 0, false), power("b", 4, 0, false));
        assert!(runs_services_among(&a, &[b.clone()]) != runs_services_among(&b, &[a]));
        // The computer itself among the others does not count.
        assert!(runs_services_among(&desktop, &[desktop.clone()]));
    }
}
