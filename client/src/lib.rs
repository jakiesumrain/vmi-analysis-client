//! VMI Analysis Client.
//!
//! The process is a local service: it owns the SSH tunnel and serves a small
//! control panel on loopback. The panel is opened in the system browser, and
//! the VMI Analysis page itself is opened in the same browser through the
//! tunnel. Nothing is rendered with an embedded webview.

mod config;
mod secrets;
mod server;
mod ssh;
mod tray;
mod tunnel;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::broadcast;
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    pub level: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConnectInfo {
    pub url: String,
    pub local_port: u16,
}

/// Events pushed to the browser over server-sent events.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum UiEvent {
    Status { status: Status },
    Log { level: String, message: String },
}

/// One live tunnel attempt.
pub struct ActiveConnection {
    pub cancel: CancellationToken,
    pub session: Session,
    pub profile: Profile,
    pub url: String,
    pub local_port: u16,
}

/// Credentials of the most recent session, kept in memory so Reconnect works
/// without asking again.
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
    pub events: broadcast::Sender<UiEvent>,
    pub token: String,
}

impl AppState {
    fn new(config_dir: PathBuf, token: String) -> Self {
        let profiles_path = config_dir.join("profiles.json");
        let store = match ProfileStore::load(&profiles_path) {
            Ok(store) => store,
            Err(err) => {
                eprintln!("[warn] {err}");
                ProfileStore::default()
            }
        };
        let (events, _) = broadcast::channel(64);
        Self {
            store: Mutex::new(store),
            status: Mutex::new(Status::default()),
            connection: Mutex::new(None),
            last: Mutex::new(None),
            profiles_path,
            known_hosts_path: config_dir.join("known_hosts"),
            events,
            token,
        }
    }
}

/// Console log line, used where no state is available.
pub(crate) fn log_line(level: &str, message: impl Into<String>) {
    eprintln!("[{level}] {}", message.into());
}

/// Log to the console and to the browser panel.
pub fn emit_log(state: &AppState, level: &str, message: impl Into<String>) {
    let message = message.into();
    log_line(level, &message);
    let _ = state.events.send(UiEvent::Log {
        level: level.to_owned(),
        message,
    });
}

/// Store the status, push it to the browser panel and update the tray tooltip.
pub fn publish(state: &AppState, status: Status) {
    if let Ok(mut guard) = state.status.lock() {
        *guard = status.clone();
    }
    let _ = state.events.send(UiEvent::Status {
        status: status.clone(),
    });
    let tooltip = match status.local_port {
        Some(port) => format!("VMI Analysis Client — {} · port {port}", status.state),
        None => format!("VMI Analysis Client — {}", status.state),
    };
    tray::set_tooltip(&tooltip);
}

pub fn read_status(state: &AppState) -> Status {
    state
        .status
        .lock()
        .map(|guard| guard.clone())
        .unwrap_or_default()
}

pub fn store_snapshot(state: &AppState) -> ProfileStore {
    state
        .store
        .lock()
        .map(|guard| guard.clone())
        .unwrap_or_default()
}

/// Cancel the tunnel and report a disconnected state.
pub fn stop(state: &AppState) {
    let active = state
        .connection
        .lock()
        .ok()
        .and_then(|mut guard| guard.take());
    if let Some(active) = active {
        active.cancel.cancel();
        emit_log(state, "info", "Tunnel closed.");
    }
    publish(state, Status::default());
}

/// Resolve credentials for a profile from the request and the OS keychain.
pub fn resolve_auth(
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
pub async fn start_session(
    state: SharedState,
    profile: Profile,
    auth: AuthMaterial,
) -> Result<ConnectInfo, String> {
    stop(&state);

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

    tokio::spawn(tunnel::accept_loop(
        Arc::clone(&state),
        cancel.clone(),
        listener,
        Arc::clone(&session),
        target,
    ));
    tokio::spawn(tunnel::supervise(
        Arc::clone(&state),
        cancel,
        session,
        spec,
        url.clone(),
        local_port,
    ));

    Ok(ConnectInfo { url, local_port })
}

pub async fn reconnect(state: SharedState) -> Result<ConnectInfo, String> {
    let last = state
        .last
        .lock()
        .map_err(|_| "Internal state lock failed.".to_owned())?
        .clone()
        .ok_or_else(|| "Connect once before using Reconnect.".to_owned())?;
    let mut profile = last.profile;
    profile.local_port = last.local_port;
    start_session(state, profile, last.auth).await
}

// ---------------------------------------------------------------------------
// Process startup
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize)]
struct SessionFile {
    pid: u32,
    port: u16,
    token: String,
}

fn config_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("VMI_CLIENT_CONFIG_DIR") {
        return PathBuf::from(dir);
    }
    if cfg!(windows) {
        std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."))
            .join("com.vmi.analysis.client")
    } else if cfg!(target_os = "macos") {
        home_dir().join("Library/Application Support/com.vmi.analysis.client")
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home_dir().join(".config"))
            .join("com.vmi.analysis.client")
    }
}

fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn new_token() -> String {
    let mut bytes = [0u8; 16];
    if getrandom::fill(&mut bytes).is_err() {
        // Extremely unlikely; fall back to a time-derived value.
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|value| value.as_nanos())
            .unwrap_or(0);
        bytes[..8].copy_from_slice(&(nanos as u64).to_le_bytes());
    }
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn panel_url(port: u16, token: &str) -> String {
    format!("http://127.0.0.1:{port}/?token={token}")
}

/// Return the panel URL if another instance is already serving it.
async fn running_instance_url(session: &SessionFile) -> Option<String> {
    let mut stream = tokio::time::timeout(
        Duration::from_millis(500),
        tokio::net::TcpStream::connect(("127.0.0.1", session.port)),
    )
    .await
    .ok()?
    .ok()?;

    let request = format!(
        "GET /api/ping HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nAuthorization: Bearer {}\r\nConnection: close\r\n\r\n",
        session.port, session.token
    );
    stream.write_all(request.as_bytes()).await.ok()?;

    let mut buffer = [0u8; 64];
    let read = tokio::time::timeout(Duration::from_millis(900), stream.read(&mut buffer))
        .await
        .ok()?
        .ok()?;
    let head = String::from_utf8_lossy(&buffer[..read]);
    head.starts_with("HTTP/1.1 200")
        .then(|| panel_url(session.port, &session.token))
}

/// Entry point used by the binary.
pub fn run() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("could not start the async runtime");
    runtime.block_on(run_async());
}

async fn run_async() {
    let dir = config_dir();
    if let Err(err) = std::fs::create_dir_all(&dir) {
        eprintln!("[error] could not create {}: {err}", dir.display());
        return;
    }
    let session_path = dir.join("session.json");

    if let Ok(text) = std::fs::read_to_string(&session_path) {
        if let Ok(session) = serde_json::from_str::<SessionFile>(&text) {
            if let Some(url) = running_instance_url(&session).await {
                log_line("info", "The client is already running; opening its panel.");
                if let Err(err) = open::that(&url) {
                    eprintln!("[warn] could not open the browser: {err}\nOpen {url}");
                }
                return;
            }
        }
    }

    let token = new_token();
    let state = Arc::new(AppState::new(dir.clone(), token.clone()));

    let listener = match TcpListener::bind(("127.0.0.1", 0)).await {
        Ok(listener) => listener,
        Err(err) => {
            eprintln!("[error] could not bind a loopback port: {err}");
            return;
        }
    };
    let port = match listener.local_addr() {
        Ok(addr) => addr.port(),
        Err(err) => {
            eprintln!("[error] could not read the loopback port: {err}");
            return;
        }
    };

    let session = SessionFile {
        pid: std::process::id(),
        port,
        token: token.clone(),
    };
    if let Ok(text) = serde_json::to_string_pretty(&session) {
        if let Err(err) = std::fs::write(&session_path, text) {
            eprintln!("[warn] could not write {}: {err}", session_path.display());
        }
    }

    let url = panel_url(port, &token);
    log_line("info", format!("Panel: {url}"));
    if let Err(err) = open::that(&url) {
        eprintln!("[warn] could not open the browser automatically: {err}");
    }

    let (tray_tx, mut tray_rx) = tokio::sync::mpsc::unbounded_channel();
    tray::start(dir.clone(), tray_tx);
    {
        let state = Arc::clone(&state);
        let panel = url.clone();
        tokio::spawn(async move {
            while let Some(action) = tray_rx.recv().await {
                match action {
                    tray::TrayAction::OpenPanel => {
                        if let Err(err) = open::that(&panel) {
                            emit_log(&state, "warn", format!("Could not open the panel: {err}"));
                        }
                    }
                    tray::TrayAction::Reconnect => {
                        if let Err(err) = reconnect(Arc::clone(&state)).await {
                            emit_log(&state, "error", err);
                        }
                    }
                    tray::TrayAction::Disconnect => stop(&state),
                    tray::TrayAction::Quit => {
                        stop(&state);
                        std::process::exit(0);
                    }
                }
            }
        });
    }

    let app = server::router(Arc::clone(&state));
    if let Err(err) = axum::serve(listener, app).await {
        eprintln!("[error] server stopped: {err}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_is_hex_and_long_enough() {
        let token = new_token();
        assert_eq!(token.len(), 32);
        assert!(token.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(token, new_token());
    }

    #[test]
    fn panel_url_carries_the_token() {
        assert_eq!(panel_url(14001, "abc"), "http://127.0.0.1:14001/?token=abc");
    }
}
