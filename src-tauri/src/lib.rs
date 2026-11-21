//! VMI Analysis Client: a desktop launcher that opens an SSH local forward to
//! the VMI Analysis server and hands the browser a loopback URL.

mod config;
mod secrets;
mod ssh;
mod tunnel;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, State, WindowEvent};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

use config::{AuthMethod, Profile, ProfileStore};
use ssh::{AuthMaterial, ConnectSpec};
use tunnel::Session;

pub type SharedState = Arc<AppState>;

#[derive(Debug, Clone, Serialize)]
pub struct Status {
    pub state: String,
    pub message: String,
    pub url: Option<String>,
    pub local_port: Option<u16>,
}

impl Default for Status {
    fn default() -> Self {
        Self {
            state: "disconnected".into(),
            message: "Not connected.".into(),
            url: None,
            local_port: None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct LogEntry {
    pub level: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConnectInfo {
    pub url: String,
    pub local_port: u16,
}

/// One live tunnel attempt.
pub struct ActiveConnection {
    pub cancel: CancellationToken,
    pub session: Session,
    pub profile: Profile,
    pub url: String,
    pub local_port: u16,
}

/// Credentials of the most recent session, kept in memory so the tray can
/// reconnect without asking again.
#[derive(Clone)]
pub struct LastSession {
    pub profile: Profile,
    pub auth: AuthMaterial,
    pub local_port: u16,
}

pub struct AppState {
    pub store: Mutex<ProfileStore>,
    pub status: Mutex<Status>,
    pub connection: Mutex<Option<ActiveConnection>>,
    pub last: Mutex<Option<LastSession>>,
    pub profiles_path: PathBuf,
    pub known_hosts_path: PathBuf,
}

impl AppState {
    fn new(config_dir: PathBuf) -> Self {
        let profiles_path = config_dir.join("profiles.json");
        let store = match ProfileStore::load(&profiles_path) {
            Ok(store) => store,
            Err(err) => {
                eprintln!("[warn] {err}");
                ProfileStore::default()
            }
        };
        Self {
            store: Mutex::new(store),
            status: Mutex::new(Status::default()),
            connection: Mutex::new(None),
            last: Mutex::new(None),
            profiles_path,
            known_hosts_path: config_dir.join("known_hosts"),
        }
    }
}

/// Console log line, used where no app handle is available.
pub(crate) fn log_line(level: &str, message: impl Into<String>) {
    eprintln!("[{level}] {}", message.into());
}

/// Log to the console and to the window.
pub(crate) fn emit_log(app: &AppHandle, level: &str, message: impl Into<String>) {
    let message = message.into();
    log_line(level, &message);
    let _ = app.emit(
        "client-log",
        LogEntry {
            level: level.to_owned(),
            message,
        },
    );
}

/// Store the status and notify the window.
pub(crate) fn publish(app: &AppHandle, state: &AppState, status: Status) {
    if let Ok(mut guard) = state.status.lock() {
        *guard = status.clone();
    }
    let _ = app.emit("client-status", status);
}

fn read_status(state: &AppState) -> Status {
    state
        .status
        .lock()
        .map(|guard| guard.clone())
        .unwrap_or_default()
}

fn store_snapshot(state: &AppState) -> ProfileStore {
    state
        .store
        .lock()
        .map(|guard| guard.clone())
        .unwrap_or_default()
}

fn stop(app: &AppHandle, state: &AppState) {
    let active = state
        .connection
        .lock()
        .ok()
        .and_then(|mut guard| guard.take());
    if let Some(active) = active {
        active.cancel.cancel();
        emit_log(app, "info", "Tunnel closed.");
    }
    publish(app, state, Status::default());
}

/// Resolve the credentials for a profile from the request and the keychain.
fn resolve_auth(
    profile: &Profile,
    password: Option<String>,
    passphrase: Option<String>,
) -> Result<AuthMaterial, String> {
    match &profile.auth {
        AuthMethod::Password => {
            let secret = match password.filter(|value| !value.is_empty()) {
                Some(supplied) => {
                    if profile.remember_password {
                        secrets::save_password(profile, &supplied)?;
                    } else {
                        secrets::delete_password(profile)?;
                    }
                    supplied
                }
                None => secrets::load_password(profile)?
                    .ok_or_else(|| "Enter the SSH password for this profile.".to_owned())?,
            };
            Ok(AuthMaterial::Password(secret))
        }
        AuthMethod::KeyFile { path } => {
            let expanded = config::expand_path(path);
            let account = expanded.to_string_lossy().into_owned();
            let secret = match passphrase.filter(|value| !value.is_empty()) {
                Some(supplied) => {
                    if profile.remember_passphrase {
                        secrets::save_passphrase(&account, &supplied)?;
                    } else {
                        secrets::delete_passphrase(&account)?;
                    }
                    Some(supplied)
                }
                None => secrets::load_passphrase(&account)?,
            };
            Ok(AuthMaterial::KeyFile {
                path: expanded,
                passphrase: secret,
            })
        }
    }
}

/// Bind the local listener and start the supervisor for one session.
async fn start(
    app: AppHandle,
    state: SharedState,
    profile: Profile,
    auth: AuthMaterial,
) -> Result<ConnectInfo, String> {
    stop(&app, &state);

    let requested_port = profile.local_port;
    let listener = TcpListener::bind(("127.0.0.1", requested_port))
        .await
        .map_err(|err| {
            if requested_port == 0 {
                format!("Could not open a local loopback port: {err}")
            } else {
                format!("Local port {requested_port} is not available: {err}")
            }
        })?;
    let local_port = listener
        .local_addr()
        .map_err(|err| format!("Could not read the local port: {err}"))?
        .port();
    let url = profile.local_url(local_port);

    let cancel = CancellationToken::new();
    let session: Session = Arc::new(tokio::sync::Mutex::new(None));

    {
        let mut guard = state
            .connection
            .lock()
            .map_err(|_| "Internal state lock failed.".to_owned())?;
        *guard = Some(ActiveConnection {
            cancel: cancel.clone(),
            session: Arc::clone(&session),
            profile: profile.clone(),
            url: url.clone(),
            local_port,
        });
    }
    if let Ok(mut guard) = state.last.lock() {
        *guard = Some(LastSession {
            profile: profile.clone(),
            auth: auth.clone(),
            local_port,
        });
    }

    let spec = ConnectSpec {
        host: profile.host.trim().to_owned(),
        port: profile.port,
        user: profile.user.trim().to_owned(),
        auth,
    };
    let target = Arc::new(tunnel::Target {
        remote_host: profile.remote_host.trim().to_owned(),
        remote_port: profile.remote_port,
    });

    tauri::async_runtime::spawn(tunnel::accept_loop(
        app.clone(),
        cancel.clone(),
        listener,
        Arc::clone(&session),
        target,
    ));
    tauri::async_runtime::spawn(tunnel::supervise(
        app,
        Arc::clone(&state),
        cancel,
        session,
        spec,
        url.clone(),
        local_port,
    ));

    Ok(ConnectInfo { url, local_port })
}

async fn do_reconnect(app: AppHandle, state: SharedState) -> Result<ConnectInfo, String> {
    let last = state
        .last
        .lock()
        .map_err(|_| "Internal state lock failed.".to_owned())?
        .clone()
        .ok_or_else(|| "Connect once before using Reconnect.".to_owned())?;
    let mut profile = last.profile;
    profile.local_port = last.local_port;
    start(app, state, profile, last.auth).await
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

#[tauri::command]
async fn connect(
    app: AppHandle,
    state: State<'_, SharedState>,
    profile: Profile,
    password: Option<String>,
    passphrase: Option<String>,
) -> Result<ConnectInfo, String> {
    profile.validate()?;
    let auth = resolve_auth(&profile, password, passphrase)?;
    let state = Arc::clone(&state);
    start(app, state, profile, auth).await
}

#[tauri::command]
async fn reconnect(app: AppHandle, state: State<'_, SharedState>) -> Result<ConnectInfo, String> {
    let state = Arc::clone(&state);
    do_reconnect(app, state).await
}

#[tauri::command]
fn disconnect(app: AppHandle, state: State<'_, SharedState>) -> Status {
    stop(&app, &state);
    read_status(&state)
}

#[tauri::command]
fn current_status(state: State<'_, SharedState>) -> Status {
    read_status(&state)
}

#[tauri::command]
fn load_profiles(state: State<'_, SharedState>) -> ProfileStore {
    store_snapshot(&state)
}

#[tauri::command]
fn save_profile(
    state: State<'_, SharedState>,
    mut profile: Profile,
) -> Result<ProfileStore, String> {
    profile.name = profile.display_name();
    profile.validate()?;
    let mut store = state
        .store
        .lock()
        .map_err(|_| "Internal state lock failed.".to_owned())?;
    store.upsert(profile);
    store.save(&state.profiles_path)?;
    Ok(store.clone())
}

#[tauri::command]
fn delete_profile(state: State<'_, SharedState>, name: String) -> Result<ProfileStore, String> {
    let mut store = state
        .store
        .lock()
        .map_err(|_| "Internal state lock failed.".to_owned())?;
    store.remove(&name);
    if store.last_used.as_deref() == Some(name.as_str()) {
        store.last_used = None;
    }
    store.save(&state.profiles_path)?;
    Ok(store.clone())
}

#[tauri::command]
fn select_profile(state: State<'_, SharedState>, name: String) -> Result<ProfileStore, String> {
    let mut store = state
        .store
        .lock()
        .map_err(|_| "Internal state lock failed.".to_owned())?;
    store.last_used = Some(name);
    store.save(&state.profiles_path)?;
    Ok(store.clone())
}

#[tauri::command]
fn open_url(app: AppHandle, url: String) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|err| format!("Could not open the browser: {err}"))
}

#[tauri::command]
async fn pick_key_file(app: AppHandle) -> Option<String> {
    use tauri_plugin_dialog::DialogExt;
    let picked = app.dialog().file().blocking_pick_file();
    picked
        .and_then(|path| path.into_path().ok())
        .map(|path| path.to_string_lossy().into_owned())
}

#[tauri::command]
fn app_version() -> String {
    env!("CARGO_PKG_VERSION").to_owned()
}

// ---------------------------------------------------------------------------
// Tray and window
// ---------------------------------------------------------------------------

fn show_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

fn open_current_url(app: &AppHandle) {
    let state = app.state::<SharedState>().inner().clone();
    if let Some(url) = read_status(&state).url {
        if let Err(err) = open_url(app.clone(), url) {
            emit_log(app, "error", err);
        }
    } else {
        emit_log(app, "warn", "No tunnel is running yet.");
    }
}

fn build_tray(app: &mut tauri::App) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "Show window", true, None::<&str>)?;
    let open = MenuItem::with_id(app, "open", "Open VMI in browser", true, None::<&str>)?;
    let reconnect = MenuItem::with_id(app, "reconnect", "Reconnect", true, None::<&str>)?;
    let disconnect = MenuItem::with_id(app, "disconnect", "Disconnect", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &open, &reconnect, &disconnect, &quit])?;

    let mut builder = TrayIconBuilder::with_id("main")
        .tooltip("VMI Analysis Client")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => show_window(app),
            "open" => open_current_url(app),
            "disconnect" => {
                let state = app.state::<SharedState>().inner().clone();
                stop(app, &state);
            }
            "reconnect" => {
                let state = app.state::<SharedState>().inner().clone();
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(err) = do_reconnect(app.clone(), state).await {
                        emit_log(&app, "error", err);
                        show_window(&app);
                    }
                });
            }
            "quit" => {
                let state = app.state::<SharedState>().inner().clone();
                stop(app, &state);
                app.exit(0);
            }
            _ => {}
        });

    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let config_dir = app.path().app_config_dir()?;
            std::fs::create_dir_all(&config_dir)?;

            // WebView2 keeps a cache; it belongs under local app data, not the
            // roaming profile, so give it an explicit directory.
            let data_dir = app.path().app_local_data_dir()?.join("webview");
            std::fs::create_dir_all(&data_dir)?;

            app.manage(Arc::new(AppState::new(config_dir)));

            tauri::WebviewWindowBuilder::new(
                app,
                "main",
                tauri::WebviewUrl::App("index.html".into()),
            )
            .title("VMI Analysis Client")
            .inner_size(960.0, 700.0)
            .min_inner_size(760.0, 560.0)
            .center()
            .data_directory(data_dir)
            .build()?;

            build_tray(app)?;
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                // The tunnel should survive closing the window; quit from the tray.
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            connect,
            reconnect,
            disconnect,
            current_status,
            load_profiles,
            save_profile,
            delete_profile,
            select_profile,
            open_url,
            pick_key_file,
            app_version
        ])
        .run(tauri::generate_context!())
        .expect("error while running VMI Analysis Client");
}
