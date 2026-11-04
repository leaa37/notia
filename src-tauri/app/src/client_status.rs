//! How a client's sync with its host is, said outside the window: on
//! Android, a fixed notification while Notia is open (looking for the host,
//! synced, syncing, a failed sync, or without the host). The window always
//! works on the copy; the status only says how far the copy is from the
//! host.
//!
//! The state comes from `host_client` (the link and the session) and
//! `host_mirror` (the sync); both call [`refresh`] when it changes. The
//! notification only shows what [`describe`] decides.

use serde::Serialize;

use crate::host::AppHandle;
use crate::host_client::LinkState;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum StatusKind {
    Connecting,
    Connected,
    Syncing,
    /// The host answers but the copy did not sync.
    Failed,
    /// The host does not answer.
    Offline,
}

impl StatusKind {
    #[cfg_attr(not(target_os = "android"), allow(dead_code))]
    fn as_str(self) -> &'static str {
        match self {
            Self::Connecting => "connecting",
            Self::Connected => "connected",
            Self::Syncing => "syncing",
            Self::Failed => "failed",
            Self::Offline => "offline",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ClientStatus {
    pub(crate) kind: StatusKind,
    pub(crate) title: String,
    pub(crate) text: String,
}

/// What the status is made of.
#[derive(Debug, Clone, Default)]
pub(crate) struct StatusInput {
    /// This device is a client (`false`: no status).
    pub(crate) client: bool,
    pub(crate) host_address: String,
    pub(crate) link: LinkState,
    pub(crate) library: Option<String>,
    pub(crate) signed_in: bool,
    pub(crate) link_message: Option<String>,
    /// The sync is moving files or the database.
    pub(crate) transferring: bool,
    /// Files done and to do of the running sync.
    pub(crate) progress: Option<(usize, usize)>,
    /// The error of the last sync.
    pub(crate) sync_error: Option<String>,
    /// Files that did not travel in the last sync.
    pub(crate) skipped: usize,
}

fn files(count: usize) -> String {
    if count == 1 { "1 archivo".to_string() } else { format!("{count} archivos") }
}

fn status(kind: StatusKind, title: &str, text: String) -> Option<ClientStatus> {
    Some(ClientStatus { kind, title: title.to_string(), text })
}

/// The status of the sync, or `None` when this device is not a client.
pub(crate) fn describe(input: &StatusInput) -> Option<ClientStatus> {
    if !input.client {
        return None;
    }
    let library = input.library.as_deref().filter(|name| !name.trim().is_empty()).unwrap_or("la biblioteca");
    let host = input.host_address.as_str();
    match input.link {
        LinkState::Unknown => status(StatusKind::Connecting, "Buscando el host", format!("Trabajás con la copia local mientras Notia busca {host}.")),
        LinkState::Offline => {
            let text = input
                .link_message
                .clone()
                .unwrap_or_else(|| format!("Trabajás con la copia local. Los cambios se sincronizan cuando {host} responda."));
            status(StatusKind::Offline, "Sin conexión con el host", text)
        }
        LinkState::Online if !input.signed_in => {
            status(StatusKind::Connecting, "Host disponible", format!("Iniciá sesión en Notia para sincronizar {library}."))
        }
        LinkState::Online if input.transferring => {
            let text = match input.progress {
                Some((done, total)) if total > 0 => format!("{library} con {host}: {done} de {}.", files(total)),
                _ => format!("{library} con {host}."),
            };
            status(StatusKind::Syncing, "Sincronizando", text)
        }
        LinkState::Online => match &input.sync_error {
            Some(error) => status(StatusKind::Failed, "No se pudo sincronizar", error.clone()),
            None if input.skipped > 0 => status(
                StatusKind::Connected,
                "Sincronizado con el host",
                format!("{library} con {host} · {} sin copiar; se vuelve a intentar.", files(input.skipped)),
            ),
            None => status(StatusKind::Connected, "Sincronizado con el host", format!("{library} con {host} · al día.")),
        },
    }
}

/// The status of this device now.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
pub(crate) fn current(app: &AppHandle) -> Option<ClientStatus> {
    if !crate::connection::is_client(app) {
        return None;
    }
    let link = crate::host_client::link_view(app);
    describe(&StatusInput {
        client: true,
        host_address: crate::connection::settings(app).host_address,
        link: link.state,
        library: link.library.or_else(|| crate::host_mirror::copied_library(app).map(|(_, name)| name)),
        signed_in: link.signed_in,
        link_message: link.message,
        transferring: crate::host_mirror::is_transferring(app),
        progress: crate::host_mirror::progress(app),
        sync_error: crate::host_mirror::last_error(app),
        skipped: crate::host_mirror::last_skipped(app),
    })
}

/// Shows the status again when it changed (Android); elsewhere the window
/// already shows it.
pub(crate) fn refresh(app: &AppHandle) {
    #[cfg(target_os = "android")]
    android::show(app, current(app));
    #[cfg(not(target_os = "android"))]
    let _ = app;
}

#[cfg(target_os = "android")]
mod android {
    use std::sync::mpsc::{self, Sender};
    use std::sync::{Mutex, OnceLock};

    use super::ClientStatus;
    use crate::host::{AppHandle, Manager};
    use crate::mobile_continuity::{clear_android_connection_status, show_android_connection_status, ContinuityState};

    /// The plugin is called from its own thread, in order: the callers hold
    /// locks of the link or the copy, and only the last status matters.
    static NOTIFIER: OnceLock<Mutex<Sender<Option<ClientStatus>>>> = OnceLock::new();

    pub(super) fn show(app: &AppHandle, status: Option<ClientStatus>) {
        let sender = NOTIFIER.get_or_init(|| {
            let (sender, receiver) = mpsc::channel::<Option<ClientStatus>>();
            let app = app.clone();
            let spawned = std::thread::Builder::new().name("notia-link-status".into()).spawn(move || {
                // `None` inside means «no notification»; the outer one, not
                // shown yet (a notification left by an earlier run goes).
                let mut shown: Option<Option<ClientStatus>> = None;
                while let Ok(mut next) = receiver.recv() {
                    while let Ok(newer) = receiver.try_recv() {
                        next = newer;
                    }
                    if shown.as_ref() == Some(&next) {
                        continue;
                    }
                    let Some(state) = app.try_state::<ContinuityState>() else {
                        continue;
                    };
                    let done = match &next {
                        Some(status) => show_android_connection_status(&state, status.kind.as_str(), &status.title, &status.text),
                        None => clear_android_connection_status(&state),
                    };
                    if done {
                        shown = Some(next);
                    }
                }
            });
            if spawned.is_err() {
                log::error!("[notia:client] the link status notifier did not start");
            }
            Mutex::new(sender)
        });
        if let Ok(sender) = sender.lock() {
            let _ = sender.send(status);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn client(link: LinkState) -> StatusInput {
        StatusInput {
            client: true,
            host_address: "casa:52480".into(),
            link,
            library: Some("Notas".into()),
            signed_in: true,
            ..StatusInput::default()
        }
    }

    #[test]
    fn a_host_shows_no_status() {
        assert_eq!(describe(&StatusInput::default()), None);
    }

    #[test]
    fn the_status_follows_the_link_and_the_sync() {
        let kind = |input: &StatusInput| describe(input).expect("status").kind;
        assert_eq!(kind(&client(LinkState::Unknown)), StatusKind::Connecting);
        let synced = describe(&client(LinkState::Online)).expect("synced");
        assert_eq!((synced.kind, synced.text.as_str()), (StatusKind::Connected, "Notas con casa:52480 · al día."));
        assert_eq!(kind(&StatusInput { transferring: true, ..client(LinkState::Online) }), StatusKind::Syncing);
        let copying = describe(&StatusInput { transferring: true, progress: Some((340, 1508)), ..client(LinkState::Online) }).expect("copying");
        assert_eq!(copying.text, "Notas con casa:52480: 340 de 1508 archivos.");
        let partial = describe(&StatusInput { skipped: 1, ..client(LinkState::Online) }).expect("partial");
        assert_eq!((partial.kind, partial.text.as_str()), (StatusKind::Connected, "Notas con casa:52480 · 1 archivo sin copiar; se vuelve a intentar."));
        let failed = describe(&StatusInput { sync_error: Some("Sin espacio.".into()), ..client(LinkState::Online) }).expect("failed");
        assert_eq!((failed.kind, failed.text.as_str()), (StatusKind::Failed, "Sin espacio."));
        assert_eq!(kind(&StatusInput { signed_in: false, ..client(LinkState::Online) }), StatusKind::Connecting);

        let offline = describe(&client(LinkState::Offline)).expect("offline");
        assert_eq!(
            (offline.kind, offline.text.as_str()),
            (StatusKind::Offline, "Trabajás con la copia local. Los cambios se sincronizan cuando casa:52480 responda.")
        );
        let changed = describe(&StatusInput { link_message: Some("El certificado del host cambió.".into()), ..client(LinkState::Offline) }).expect("changed");
        assert_eq!(changed.text, "El certificado del host cambió.");
    }
}
