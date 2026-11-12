//! Headless mode: `notia --headless` runs the application without a window
//! and serves it over HTTPS + WebSocket to remote clients on the network.
//! The routes are those of `server::api`, shared with the Host mode of the
//! app window; this module parses the command line and starts the server.

use std::net::{SocketAddr, TcpListener};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::api::{ApiServer, ServerKind};
use super::assets::DirectoryAssets;
use super::{network, owner, tls};
use crate::host::plugin::PluginApi;
use crate::host::{AppHandle, AppPaths, AssetSource, DataDirLock, HostPorts, Manager};
use crate::library_catalog::CatalogLibrary;

/// Identifier of the application; the headless server uses the same data
/// folder the window uses.
const APP_IDENTIFIER: &str = "com.gabriel.notia";
pub const DEFAULT_PORT: u16 = crate::backend::connection::DEFAULT_HOST_PORT;
const OWNER_PASSWORD_VARIABLE: &str = "NOTIA_OWNER_PASSWORD";
/// Under `--data-root`: Notia's data and the libraries, side by side.
const DATA_ROOT_APP_DIR: &str = "app";
const DATA_ROOT_LIBRARIES_DIR: &str = "bibliotecas";
/// The library `--data-root` creates when it has none.
const DEFAULT_LIBRARY_NAME: &str = "Biblioteca";

const USAGE: &str = "Uso: notia --headless [opciones]

  --data-root <carpeta>     Todo lo que Notia guarda queda en esta carpeta: sus
                            datos en <carpeta>/app y las bibliotecas en
                            <carpeta>/bibliotecas, donde cada subcarpeta es una
                            biblioteca (si no hay ninguna, crea «Biblioteca»).
                            No se combina con --data-dir ni --add-library.
  --data-dir <carpeta>      Datos de Notia (por defecto, los de la aplicación).
  --resource-dir <carpeta>  Recursos (modelos de voz, runtimes). Por defecto, la carpeta del ejecutable.
  --static-dir <carpeta>    Interfaz compilada (dist) para los clientes remotos.
  --bind <dirección:puerto> Dirección de escucha (por defecto 0.0.0.0:52480).
  --set-owner-password      Guarda la contraseña del dueño y termina. La lee de
                            NOTIA_OWNER_PASSWORD o de la entrada estándar.
  --add-library <carpeta>   Agrega una carpeta de este equipo como biblioteca y
                            termina (la opción se puede repetir).";

/// Whether the command line asks for the headless mode.
pub fn requested(mut args: impl Iterator<Item = String>) -> bool {
    args.any(|argument| argument == "--headless")
}

#[derive(Debug, Clone, PartialEq)]
struct Options {
    data_dir: PathBuf,
    /// With `--data-root`: every folder in it is a library of the server.
    libraries_dir: Option<PathBuf>,
    resource_dir: Option<PathBuf>,
    static_dir: Option<PathBuf>,
    bind: SocketAddr,
    set_owner_password: bool,
    add_libraries: Vec<PathBuf>,
}

fn default_data_dir() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    let base = std::env::var_os("APPDATA").map(PathBuf::from);
    #[cfg(not(target_os = "windows"))]
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local").join("share")));
    base.map(|base| base.join(APP_IDENTIFIER))
}

fn executable_dir() -> Option<PathBuf> {
    std::env::current_exe().ok()?.parent().map(Path::to_path_buf)
}

fn parse_options(args: &[String], default_data: Option<PathBuf>, executable: Option<PathBuf>) -> Result<Options, String> {
    let mut data_root: Option<PathBuf> = None;
    let mut data_dir = None;
    let mut resource_dir = None;
    let mut static_dir = None;
    let mut bind = SocketAddr::from(([0, 0, 0, 0], DEFAULT_PORT));
    let mut set_owner_password = false;
    let mut add_libraries = Vec::new();
    let mut arguments = args.iter();
    while let Some(argument) = arguments.next() {
        let (name, inline) = match argument.split_once('=') {
            Some((name, value)) => (name, Some(value.to_string())),
            None => (argument.as_str(), None),
        };
        let mut value = || {
            inline
                .clone()
                .or_else(|| arguments.next().cloned())
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| format!("Falta el valor de {name}."))
        };
        match name {
            "--headless" => {}
            "--set-owner-password" => set_owner_password = true,
            "--data-root" => data_root = Some(PathBuf::from(value()?)),
            "--data-dir" => data_dir = Some(PathBuf::from(value()?)),
            "--resource-dir" => resource_dir = Some(PathBuf::from(value()?)),
            "--static-dir" => static_dir = Some(PathBuf::from(value()?)),
            "--add-library" => add_libraries.push(PathBuf::from(value()?)),
            "--bind" => {
                bind = value()?
                    .parse()
                    .map_err(|_| "--bind espera dirección:puerto, por ejemplo 0.0.0.0:52480.".to_string())?
            }
            other => return Err(format!("Opción desconocida: {other}.")),
        }
    }
    if data_root.is_some() && (data_dir.is_some() || !add_libraries.is_empty()) {
        return Err(
            "--data-root no se combina con --data-dir ni con --add-library: cada carpeta de <data-root>/bibliotecas ya es una biblioteca."
                .to_string(),
        );
    }
    let libraries_dir = data_root.as_ref().map(|root| root.join(DATA_ROOT_LIBRARIES_DIR));
    let data_dir = data_root
        .map(|root| root.join(DATA_ROOT_APP_DIR))
        .or(data_dir)
        .or(default_data)
        .ok_or_else(|| "No se pudo determinar la carpeta de datos; indicá --data-dir.".to_string())?;
    let resource_dir = resource_dir.or_else(|| executable.clone());
    let static_dir = static_dir.or_else(|| executable.map(|dir| dir.join("dist")).filter(|dir| dir.is_dir()));
    Ok(Options { data_dir, libraries_dir, resource_dir, static_dir, bind, set_owner_password, add_libraries })
}

/// Runs the headless mode and returns the process exit code.
pub fn run(args: Vec<String>) -> i32 {
    attach_parent_console();
    if args.iter().any(|argument| argument == "--help" || argument == "-h") {
        println!("{USAGE}");
        return 0;
    }
    let options = match parse_options(&args, default_data_dir(), executable_dir()) {
        Ok(options) => options,
        Err(error) => {
            eprintln!("{error}\n\n{USAGE}");
            return 2;
        }
    };
    let server_dir = options.data_dir.join("headless-server");
    if options.set_owner_password {
        return store_owner_password(&server_dir);
    }
    if !options.add_libraries.is_empty() {
        return add_libraries(&options);
    }
    if !owner::has_owner_password(&server_dir) {
        eprintln!(
            "El servidor no tiene contraseña de dueño. Definila corriendo el mismo comando con --set-owner-password."
        );
        return 2;
    }
    match serve(options, server_dir) {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("{error}");
            1
        }
    }
}

fn store_owner_password(server_dir: &Path) -> i32 {
    let password = match std::env::var(OWNER_PASSWORD_VARIABLE) {
        Ok(password) => password,
        Err(_) => {
            println!("Contraseña del dueño (entre 8 y 256 caracteres):");
            let mut line = String::new();
            if std::io::stdin().read_line(&mut line).is_err() {
                eprintln!("No se pudo leer la contraseña.");
                return 1;
            }
            line.trim_end_matches(['\r', '\n']).to_string()
        }
    };
    match owner::set_owner_password(server_dir, &password) {
        Ok(()) => {
            println!("Contraseña guardada.");
            0
        }
        Err(error) => {
            eprintln!("{error}");
            1
        }
    }
}

/// Registers local folders as libraries, with the data the server and the
/// window use; neither may be running meanwhile (the data folder lock).
fn add_libraries(options: &Options) -> i32 {
    let _lock = match DataDirLock::acquire(&options.data_dir) {
        Ok(lock) => lock,
        Err(error) => {
            eprintln!("{error}");
            return 1;
        }
    };
    let app = crate::create_app(
        AppPaths::new(Some(options.data_dir.clone()), options.resource_dir.clone()),
        HostPorts::default(),
    );
    crate::library_registry::load_persisted_bindings(&app);
    let mut code = 0;
    for folder in &options.add_libraries {
        match register_library(&app, folder) {
            Ok(library) => println!("Biblioteca «{}» lista ({}).", library.name, library.path),
            Err(error) => {
                eprintln!("{}: {error}", folder.display());
                code = 1;
            }
        }
    }
    code
}

/// Adds `folder` to the catalog (once) and leaves its `.notia/` database
/// ready, so the first sign-in of a folder Notia never opened works.
fn register_library(app: &AppHandle, folder: &Path) -> Result<CatalogLibrary, String> {
    let library = crate::library_catalog::add_desktop_library(app, folder).map_err(|error| error.message)?;
    let database = crate::database::initialize_desktop_library_database(&library.path);
    if !database.ok {
        return Err(database
            .error
            .unwrap_or_else(|| "No se pudo preparar la base de datos de la biblioteca.".to_string()));
    }
    Ok(library)
}

/// Every folder of `libraries_dir` is a library of the server: registered
/// once and with its database ready. Without any, it creates «Biblioteca».
/// Hidden folders and links are skipped, so every library stays inside it.
fn register_root_libraries(app: &AppHandle, libraries_dir: &Path) -> Result<Vec<CatalogLibrary>, String> {
    std::fs::create_dir_all(libraries_dir)
        .map_err(|_| format!("No se pudo crear la carpeta de bibliotecas {}.", libraries_dir.display()))?;
    let mut folders = library_folders(libraries_dir)?;
    if folders.is_empty() {
        let folder = libraries_dir.join(DEFAULT_LIBRARY_NAME);
        std::fs::create_dir_all(&folder)
            .map_err(|_| format!("No se pudo crear la biblioteca {}.", folder.display()))?;
        folders.push(folder);
    }
    let mut libraries = Vec::new();
    for folder in folders {
        match register_library(app, &folder) {
            Ok(library) => libraries.push(library),
            Err(error) => eprintln!("{}: {error}", folder.display()),
        }
    }
    Ok(libraries)
}

fn library_folders(libraries_dir: &Path) -> Result<Vec<PathBuf>, String> {
    let entries = std::fs::read_dir(libraries_dir)
        .map_err(|_| format!("No se pudo leer la carpeta de bibliotecas {}.", libraries_dir.display()))?;
    let mut folders: Vec<PathBuf> = entries
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .filter(|entry| !entry.file_name().to_string_lossy().starts_with('.'))
        .map(|entry| entry.path())
        .collect();
    folders.sort();
    Ok(folders)
}

fn serve(options: Options, server_dir: PathBuf) -> Result<(), String> {
    let lock = DataDirLock::acquire(&options.data_dir)?;
    let tls = tls::server_config(&server_dir.join("tls"))?;
    let listener = TcpListener::bind(options.bind)
        .map_err(|_| format!("No se pudo escuchar en {}.", options.bind))?;
    let port = listener.local_addr().map(|address| address.port()).unwrap_or(options.bind.port());

    let assets = options
        .static_dir
        .clone()
        .map(|dir| Arc::new(DirectoryAssets::new(dir)) as Arc<dyn AssetSource>);
    // The application delivers its events to the shared hub (`create_app`).
    let app = crate::create_app(
        AppPaths::new(Some(options.data_dir.clone()), options.resource_dir.clone()),
        HostPorts { events: None, assets: assets.clone(), dialogs: None },
    );
    if let Some(libraries_dir) = &options.libraries_dir {
        // Before the startup hooks, which open the catalog's libraries.
        crate::library_registry::load_persisted_bindings(&app);
        for library in register_root_libraries(&app, libraries_dir)? {
            println!("Biblioteca «{}» lista ({}).", library.name, library.path);
        }
    }
    // The server runs the AI actions of its library ahead of the app.
    crate::ai_actions::run_as_server();
    for hook in crate::startup_hooks() {
        let name = hook.name();
        if let Err(error) = hook.run_setup(&app, PluginApi::new(None)) {
            log::error!("[notia:headless] startup hook {name} failed: {error}");
        }
    }
    let hub = app.state::<super::SharedEventHub>().inner().0.clone();
    let has_assets = assets.is_some();
    let server = Arc::new(ApiServer::new(app, hub, assets, ServerKind::Headless, Some(server_dir), Some(lock)));
    println!("Notia headless escuchando en https://{}:{port}/", network::local_network_ip());
    if !has_assets {
        println!("Sin interfaz: indicá --static-dir para servir la aplicación web.");
    }
    super::api::run(listener, server, tls);
    Ok(())
}

/// A release build on Windows has no console of its own; attach to the one
/// that launched the headless server so its messages are visible.
#[cfg(target_os = "windows")]
fn attach_parent_console() {
    use windows::Win32::System::Console::{AttachConsole, ATTACH_PARENT_PROCESS};
    // SAFETY: plain Win32 call without pointers; failure only means there is
    // no parent console (or one is already attached).
    unsafe {
        let _ = AttachConsole(ATTACH_PARENT_PROCESS);
    }
}

#[cfg(not(target_os = "windows"))]
fn attach_parent_console() {}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn parses_the_headless_options() {
        let options = parse_options(
            &args(&["--headless", "--data-dir", "D", "--bind=127.0.0.1:9000", "--static-dir", "web"]),
            None,
            Some(PathBuf::from("exe")),
        )
        .expect("options");
        assert_eq!(options.data_dir, PathBuf::from("D"));
        assert_eq!(options.bind, "127.0.0.1:9000".parse::<SocketAddr>().unwrap());
        assert_eq!(options.static_dir, Some(PathBuf::from("web")));
        assert_eq!(options.resource_dir, Some(PathBuf::from("exe")));
        assert!(!options.set_owner_password);
        assert!(options.add_libraries.is_empty());
        let adding = parse_options(
            &args(&["--headless", "--add-library", "A", "--add-library=B"]),
            Some(PathBuf::from("d")),
            None,
        )
        .expect("options");
        assert_eq!(adding.add_libraries, vec![PathBuf::from("A"), PathBuf::from("B")]);
    }

    #[test]
    fn uses_the_application_data_folder_by_default_and_rejects_unknown_options() {
        let options = parse_options(&args(&["--headless"]), Some(PathBuf::from("appdata")), None).expect("options");
        assert_eq!(options.data_dir, PathBuf::from("appdata"));
        assert_eq!(options.bind.port(), DEFAULT_PORT);
        assert!(parse_options(&args(&["--headless"]), None, None).is_err());
        assert!(parse_options(&args(&["--headless", "--port", "1"]), Some(PathBuf::from("d")), None).is_err());
        assert!(parse_options(&args(&["--headless", "--bind"]), Some(PathBuf::from("d")), None).is_err());
        assert!(requested(args(&["notia", "--headless"]).into_iter()));
        assert!(!requested(args(&["notia"]).into_iter()));
    }

    #[test]
    fn the_data_root_holds_the_data_and_the_libraries() {
        let options = parse_options(&args(&["--headless", "--data-root", "/datos"]), Some(PathBuf::from("appdata")), None)
            .expect("options");
        assert_eq!(options.data_dir, PathBuf::from("/datos").join("app"));
        assert_eq!(options.libraries_dir, Some(PathBuf::from("/datos").join("bibliotecas")));
        let owner = parse_options(&args(&["--headless", "--data-root=/datos", "--set-owner-password"]), None, None)
            .expect("options");
        assert!(owner.set_owner_password);
        assert_eq!(owner.data_dir, PathBuf::from("/datos").join("app"));
        let plain = parse_options(&args(&["--headless"]), Some(PathBuf::from("appdata")), None).expect("options");
        assert_eq!(plain.libraries_dir, None);
        // A library or data folder elsewhere would live outside the root.
        assert!(parse_options(&args(&["--headless", "--data-root", "/datos", "--data-dir", "D"]), None, None).is_err());
        assert!(parse_options(&args(&["--headless", "--data-root", "/datos", "--add-library", "A"]), None, None).is_err());
        assert!(parse_options(&args(&["--headless", "--data-root"]), None, None).is_err());
    }

    #[test]
    fn the_folders_of_the_data_root_become_ready_libraries() {
        let root = std::env::temp_dir().join(format!("notia-data-root-{}", uuid::Uuid::new_v4()));
        let libraries_dir = root.join(DATA_ROOT_LIBRARIES_DIR);
        let app = crate::create_app(AppPaths::new(Some(root.join(DATA_ROOT_APP_DIR)), None), HostPorts::default());
        // A startup hook manages it in the server; the sign-in status reads it.
        app.manage(crate::mobile_directory_picker::AndroidDirectoryPickerState::empty());
        crate::library_registry::load_persisted_bindings(&app);

        // An empty root gets «Biblioteca», with its database, and the
        // Owner's first sign-in can start (it failed without `.notia/`).
        let created = register_root_libraries(&app, &libraries_dir).expect("libraries");
        assert_eq!(created.len(), 1);
        assert_eq!(created[0].name, DEFAULT_LIBRARY_NAME);
        assert!(libraries_dir.join(DEFAULT_LIBRARY_NAME).join(".notia").join("notia.db").is_file());
        let payload = serde_json::from_value(serde_json::json!({})).expect("payload");
        let status = crate::app_auth::app_auth_status(&app, payload).expect("sign-in status");
        assert_eq!(serde_json::to_value(status).expect("json")["state"], "setup");

        // New folders join on the next start; hidden ones are not libraries,
        // and the ones already there keep their id.
        std::fs::create_dir_all(libraries_dir.join("Trabajo")).expect("folder");
        std::fs::create_dir_all(libraries_dir.join(".papelera")).expect("hidden folder");
        let again = register_root_libraries(&app, &libraries_dir).expect("libraries");
        let names: Vec<&str> = again.iter().map(|library| library.name.as_str()).collect();
        assert_eq!(names, vec![DEFAULT_LIBRARY_NAME, "Trabajo"]);
        assert_eq!(again[0].id, created[0].id);
        assert!(libraries_dir.join("Trabajo").join(".notia").join("notia.db").is_file());
        assert!(!libraries_dir.join(".papelera").join(".notia").exists());
        let _ = std::fs::remove_dir_all(&root);
    }
}
