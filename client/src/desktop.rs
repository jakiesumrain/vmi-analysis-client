//! Optional account linking and Agent launch over the active SSH tunnel.

use std::time::Duration;

use axum::{extract::State, Json};
use http_body_util::{BodyExt, Full, Limited};
use hyper::{body::Bytes, client::conn::http1, Request, StatusCode};
use hyper_util::rt::TokioIo;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::json;
use tokio::net::TcpStream;

use crate::config::{Profile, ProfileStore};
use crate::server::ApiError;
use crate::{read_status, secrets, AppState, SharedState};

const PREFIX: &str = "/client/v1";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VmiProfile {
    pub account: String,
    pub agent: Option<AgentProfile>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentProfile {
    pub base_url: String,
    pub api: String,
    pub model: String,
    #[serde(default)]
    pub additional_models: Vec<String>,
    #[serde(default)]
    pub image_models: Vec<String>,
}

fn server_identity(profile: &Profile) -> String {
    json!([
        profile.host.trim().to_ascii_lowercase(),
        profile.port,
        profile.user.trim(),
        profile.remote_host.trim(),
        profile.remote_port
    ])
    .to_string()
}

fn key_id(kind: &str, profile: &Profile, vmi: &VmiProfile, provider: &str) -> String {
    format!(
        "vmi:{kind}:{}",
        json!([server_identity(profile), vmi.account, provider])
    )
}

fn vmi_profile(profile: &Profile) -> Result<&VmiProfile, String> {
    profile.validate()?;
    let vmi = profile.vmi.as_ref().ok_or("Enter a VMI account name.")?;
    let mut bytes = vmi.account.bytes();
    if vmi.account.len() > 32
        || !bytes.next().is_some_and(|byte| byte.is_ascii_lowercase())
        || !bytes
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"_-".contains(&byte))
    {
        return Err("Enter a valid VMI account name.".into());
    }
    Ok(vmi)
}

pub(crate) fn save_profile(
    state: &AppState,
    profile: &mut Profile,
) -> Result<ProfileStore, String> {
    profile.name = profile.display_name();
    profile.validate()?;
    if profile.vmi.is_some() {
        vmi_profile(profile)?;
    }
    let mut store = state
        .store
        .lock()
        .map_err(|_| "Internal state lock failed.")?;
    let mut updated = store.clone();
    updated.upsert(profile.clone());
    updated.save(&state.profiles_path)?;
    *store = updated.clone();
    Ok(updated)
}

fn active_port(state: &AppState, profile: &Profile) -> Result<u16, String> {
    let status = read_status(state);
    let last = state
        .last
        .lock()
        .map_err(|_| "Internal state lock failed.")?;
    match (status.state.as_str(), status.local_port, last.as_ref()) {
        ("connected", Some(port), Some(active))
            if server_identity(profile) == server_identity(&active.profile) =>
        {
            Ok(port)
        }
        _ => Err("Connect this profile’s SSH tunnel first.".into()),
    }
}

fn valid_token(token: &str) -> bool {
    token.len() == 43
        && token
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_-".contains(&byte))
}

async fn request<T: DeserializeOwned>(
    port: u16,
    path: &str,
    payload: serde_json::Value,
    credential: Option<&str>,
) -> Result<T, String> {
    tokio::time::timeout(Duration::from_secs(50), async {
        let tcp = TcpStream::connect(("127.0.0.1", port))
            .await
            .map_err(|_| "Could not reach VMI through the tunnel.")?;
        let (mut sender, connection) = http1::handshake(TokioIo::new(tcp))
            .await
            .map_err(|_| "Could not open the VMI connection.")?;
        let connection = tokio::spawn(connection);
        let mut builder = Request::post(format!("{PREFIX}{path}"))
            .header("host", format!("127.0.0.1:{port}"))
            .header("content-type", "application/json")
            .header("connection", "close");
        if let Some(token) = credential {
            builder = builder.header("authorization", format!("Bearer {token}"));
        }
        let result = async {
            let body = Full::new(Bytes::from(payload.to_string()));
            let outgoing = builder.body(body).map_err(|_| "Invalid desktop request.")?;
            let response = sender
                .send_request(outgoing)
                .await
                .map_err(|_| "Could not send the desktop request.")?;
            let status = response.status();
            if !status.is_success() {
                return Err(match status {
                    StatusCode::NOT_FOUND => "This VMI server does not support desktop sign-in.",
                    StatusCode::UNAUTHORIZED => "Link this VMI account again.",
                    StatusCode::FORBIDDEN => {
                        "VMI refused the link. Generate a new link code in VMI Settings."
                    }
                    _ => "The VMI server could not complete the desktop request.",
                });
            }
            let body = Limited::new(response.into_body(), 16384)
                .collect()
                .await
                .map_err(|_| "Could not read the desktop response.")?
                .to_bytes();
            serde_json::from_slice(&body)
                .map_err(|_| "The VMI server returned an invalid response.")
        }
        .await;
        connection.abort();
        result.map_err(str::to_string)
    })
    .await
    .map_err(|_| "VMI did not respond in time.".to_owned())?
}

#[derive(Deserialize)]
pub struct PairRequest {
    profile: Profile,
    code: String,
}

#[derive(Deserialize)]
struct PairResponse {
    credential: String,
}

pub async fn pair(
    State(state): State<SharedState>,
    Json(mut data): Json<PairRequest>,
) -> Result<Json<ProfileStore>, ApiError> {
    let vmi = vmi_profile(&data.profile).map_err(ApiError)?;
    if !valid_token(&data.code) {
        return Err(ApiError("Paste the link code from VMI Settings.".into()));
    }
    let _connection = state.connection.lock().await;
    let port = active_port(&state, &data.profile).map_err(ApiError)?;
    let reply: PairResponse = request(
        port,
        "/pair",
        json!({"account": vmi.account, "code": data.code}),
        None,
    )
    .await
    .map_err(ApiError)?;
    if !valid_token(&reply.credential) {
        return Err(ApiError(
            "The server returned an invalid desktop credential.".into(),
        ));
    }
    secrets::save(&key_id("device", &data.profile, vmi, ""), &reply.credential)
        .map_err(ApiError)?;
    save_profile(&state, &mut data.profile)
        .map(Json)
        .map_err(ApiError)
}

#[derive(Deserialize)]
pub struct SaveRequest {
    profile: Profile,
    api_key: Option<String>,
}

pub async fn save(
    State(state): State<SharedState>,
    Json(mut data): Json<SaveRequest>,
) -> Result<Json<ProfileStore>, ApiError> {
    if let Some(key) = data.api_key.filter(|key| !key.is_empty()) {
        let vmi = vmi_profile(&data.profile).map_err(ApiError)?;
        let agent = vmi
            .agent
            .as_ref()
            .ok_or_else(|| ApiError("Enter the model endpoint and model ID.".into()))?;
        if key.len() > 16384 || key.contains(['\r', '\n', '\0']) || agent.base_url.is_empty() {
            return Err(ApiError("Enter a valid model endpoint and API key.".into()));
        }
        secrets::save(&key_id("model", &data.profile, vmi, &agent.base_url), &key)
            .map_err(ApiError)?;
    }
    save_profile(&state, &mut data.profile)
        .map(Json)
        .map_err(ApiError)
}

#[derive(Deserialize)]
struct LaunchResponse {
    ticket: String,
    warning: Option<String>,
}

pub async fn open(
    State(state): State<SharedState>,
    Json(profile): Json<Profile>,
) -> Result<Json<Option<String>>, ApiError> {
    let vmi = vmi_profile(&profile).map_err(ApiError)?;
    let _connection = state.connection.lock().await;
    let port = active_port(&state, &profile).map_err(ApiError)?;
    let credential = secrets::load(&key_id("device", &profile, vmi, ""))
        .map_err(ApiError)?
        .ok_or_else(|| ApiError("Link your VMI account before using desktop sign-in.".into()))?;
    let mut warning = None;
    let agent = if let Some(settings) = &vmi.agent {
        match secrets::load(&key_id("model", &profile, vmi, &settings.base_url))
            .map_err(ApiError)?
        {
            Some(key) => Some(json!({
                "base_url": settings.base_url, "api": settings.api, "model": settings.model,
                "additional_models": settings.additional_models, "image_models": settings.image_models,
                "api_key": key,
            })),
            None => {
                warning = Some("No API key saved for this model endpoint. VMI was opened without starting Agent.".into());
                None
            }
        }
    } else {
        None
    };
    let reply: LaunchResponse = request(
        port,
        "/launch",
        json!({"account": vmi.account, "agent": agent}),
        Some(&credential),
    )
    .await
    .map_err(ApiError)?;
    if !valid_token(&reply.ticket) {
        return Err(ApiError(
            "The server returned an invalid sign-in ticket.".into(),
        ));
    }
    let address = format!(
        "http://127.0.0.1:{port}{PREFIX}/open#account={}&ticket={}",
        vmi.account, reply.ticket
    );
    open::that(&address)
        .map_err(|_| ApiError("Could not open VMI in the default browser.".into()))?;
    Ok(Json(reply.warning.or(warning)))
}
