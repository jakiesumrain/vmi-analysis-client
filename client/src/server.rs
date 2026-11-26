//! Loopback HTTP server: the control panel, its JSON API and the event stream.

use std::convert::Infallible;
use std::time::Duration;

use axum::extract::{Path, Request, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::middleware::{self, Next};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use serde::Deserialize;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;

use crate::config::{Profile, ProfileStore};
use crate::{
    emit_log, read_status, reconnect, resolve_auth, start_session, stop, store_snapshot, AppState,
    ConnectInfo, LogEntry, SharedState, Status, UiEvent,
};

const INDEX_HTML: &str = include_str!("../../ui/index.html");
const STYLE_CSS: &str = include_str!("../../ui/style.css");
const APP_JS: &str = include_str!("../../ui/app.js");

pub fn router(state: SharedState) -> Router {
    let desktop = Router::new()
        .route("/save", post(crate::desktop::save))
        .route("/open", post(crate::desktop::open))
        .route("/open-local", post(crate::desktop::open_local))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            require_bearer,
        ));
    let api = Router::new()
        .route("/ping", get(ping))
        .route("/status", get(status))
        .route("/events", get(events))
        .route("/profiles", get(list_profiles).post(save_profile))
        .route("/profiles/{name}", delete(delete_profile))
        .route("/connect", post(connect))
        .route("/reconnect", post(reconnect_handler))
        .route("/disconnect", post(disconnect))
        .route("/pick-key", post(pick_key))
        .route("/open", post(open_url))
        .route("/quit", post(quit))
        .nest("/desktop", desktop)
        .layer(middleware::from_fn_with_state(state.clone(), require_token));

    Router::new()
        .route("/", get(index))
        .route("/style.css", get(style))
        .route("/app.js", get(script))
        .nest("/api", api)
        .with_state(state)
}

// ---------------------------------------------------------------------------
// Authorization
// ---------------------------------------------------------------------------

fn constant_time_eq(left: &str, right: &str) -> bool {
    let left = left.as_bytes();
    let right = right.as_bytes();
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0u8, |acc, (a, b)| acc | (a ^ b))
        == 0
}

fn authorized(state: &AppState, headers: &HeaderMap, query: Option<&str>) -> bool {
    if let Some(value) = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
    {
        if let Some(token) = value.strip_prefix("Bearer ") {
            if constant_time_eq(token.trim(), &state.token) {
                return true;
            }
        }
    }
    if let Some(query) = query {
        for pair in query.split('&') {
            if let Some(value) = pair.strip_prefix("token=") {
                if constant_time_eq(value, &state.token) {
                    return true;
                }
            }
        }
    }
    false
}

async fn require_token(State(state): State<SharedState>, request: Request, next: Next) -> Response {
    if authorized(&state, request.headers(), request.uri().query()) {
        next.run(request).await
    } else {
        (StatusCode::UNAUTHORIZED, "Unauthorized").into_response()
    }
}

async fn require_bearer(
    State(state): State<SharedState>,
    request: Request,
    next: Next,
) -> Response {
    if authorized(&state, request.headers(), None) {
        next.run(request).await
    } else {
        (StatusCode::UNAUTHORIZED, "Unauthorized").into_response()
    }
}

// ---------------------------------------------------------------------------
// Static panel
// ---------------------------------------------------------------------------

async fn index() -> Html<&'static str> {
    Html(INDEX_HTML)
}

async fn style() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
        STYLE_CSS,
    )
}

async fn script() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        APP_JS,
    )
}

// ---------------------------------------------------------------------------
// API
// ---------------------------------------------------------------------------

pub(crate) struct ApiError(pub(crate) String);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (StatusCode::BAD_REQUEST, self.0).into_response()
    }
}

async fn ping() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "ok": true }))
}

async fn status(State(state): State<SharedState>) -> Json<Status> {
    Json(read_status(&state))
}

async fn list_profiles(State(state): State<SharedState>) -> Json<ProfileStore> {
    Json(store_snapshot(&state))
}

async fn save_profile(
    State(state): State<SharedState>,
    Json(mut profile): Json<Profile>,
) -> Result<Json<ProfileStore>, ApiError> {
    crate::desktop::save_profile(&state, &mut profile)
        .map(Json)
        .map_err(ApiError)
}

async fn delete_profile(
    State(state): State<SharedState>,
    Path(name): Path<String>,
) -> Result<Json<ProfileStore>, ApiError> {
    let mut store = state
        .store
        .lock()
        .map_err(|_| ApiError("Internal state lock failed.".into()))?;
    store.remove(&name);
    if store.last_used.as_deref() == Some(name.as_str()) {
        store.last_used = None;
    }
    store.save(&state.profiles_path).map_err(ApiError)?;
    Ok(Json(store.clone()))
}

#[derive(Deserialize)]
struct ConnectRequest {
    profile: Profile,
    #[serde(default)]
    password: Option<String>,
    #[serde(default)]
    passphrase: Option<String>,
}

async fn connect(
    State(state): State<SharedState>,
    Json(request): Json<ConnectRequest>,
) -> Result<Json<ConnectInfo>, ApiError> {
    request.profile.validate().map_err(ApiError)?;
    if request.profile.mode != crate::config::ProfileMode::Ssh {
        return Err(ApiError("Use Open VMI for a local profile.".into()));
    }
    let auth =
        resolve_auth(&request.profile, request.password, request.passphrase).map_err(ApiError)?;
    start_session(state, request.profile, auth)
        .await
        .map(Json)
        .map_err(ApiError)
}

async fn reconnect_handler(
    State(state): State<SharedState>,
) -> Result<Json<ConnectInfo>, ApiError> {
    reconnect(state).await.map(Json).map_err(ApiError)
}

async fn disconnect(State(state): State<SharedState>) -> Json<Status> {
    stop(&state).await;
    Json(read_status(&state))
}

async fn pick_key() -> Json<Option<String>> {
    let picked = tokio::task::spawn_blocking(|| rfd::FileDialog::new().pick_file())
        .await
        .ok()
        .flatten();
    Json(picked.map(|path| path.to_string_lossy().into_owned()))
}

#[derive(Deserialize)]
struct OpenRequest {
    url: String,
}

async fn open_url(Json(request): Json<OpenRequest>) -> Result<Json<()>, ApiError> {
    if !request.url.starts_with("http://127.0.0.1:") {
        return Err(ApiError("Refusing to open a non-loopback URL.".into()));
    }
    open::that(&request.url)
        .map_err(|err| ApiError(format!("Could not open the browser: {err}")))?;
    Ok(Json(()))
}

async fn quit(State(state): State<SharedState>) -> Json<()> {
    stop(&state).await;
    emit_log(&state, "info", "Stopping the client.");
    tokio::spawn(async {
        tokio::time::sleep(Duration::from_millis(250)).await;
        std::process::exit(0);
    });
    Json(())
}

async fn events(
    State(state): State<SharedState>,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, Infallible>>> {
    let stream = BroadcastStream::new(state.events.subscribe())
        .then(|item| async move {
            match item {
                Ok(UiEvent::Status { status }) => Event::default()
                    .event("status")
                    .json_data(status)
                    .ok()
                    .map(Ok),
                Ok(UiEvent::Log { level, message }) => Event::default()
                    .event("log")
                    .json_data(LogEntry { level, message })
                    .ok()
                    .map(Ok),
                Err(_) => None,
            }
        })
        .filter_map(|item| item);
    Sse::new(stream).keep_alive(KeepAlive::default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constant_time_compare_matches_only_equal_strings() {
        assert!(constant_time_eq("abc", "abc"));
        assert!(!constant_time_eq("abc", "abd"));
        assert!(!constant_time_eq("abc", "ab"));
    }
}
