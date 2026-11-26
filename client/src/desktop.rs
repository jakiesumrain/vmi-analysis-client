//! Open a local GUI or sign in over SSH, with optional Agent startup.

use std::time::Duration;

use axum::{extract::State, response::IntoResponse, Json};
use http_body_util::{BodyExt, Full, Limited};
use hyper::{body::Bytes, client::conn::http1, header, HeaderMap, Request, StatusCode};
use hyper_util::rt::TokioIo;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::net::TcpStream;

use crate::config::{Profile, ProfileMode, ProfileStore};
use crate::local_gui;
use crate::server::ApiError;
use crate::{read_status, secrets, AppState, SharedState};

const LOGIN_COOKIE: &str = "vmi_session";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VmiProfile {
    #[serde(default)]
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

fn key_id(
    kind: &str,
    profile: &Profile,
    vmi: &VmiProfile,
    provider: &str,
) -> Result<String, String> {
    let identity = if profile.mode == ProfileMode::Local {
        json!(["local", profile.local_checkout()?]).to_string()
    } else {
        server_identity(profile)
    };
    Ok(format!(
        "vmi:{kind}:{}",
        json!([identity, vmi.account, provider])
    ))
}

fn vmi_profile(profile: &Profile) -> Result<&VmiProfile, String> {
    profile.validate()?;
    let vmi = profile
        .vmi
        .as_ref()
        .ok_or("Enter the VMI connection settings.")?;
    if profile.mode == ProfileMode::Local {
        return if vmi.account.is_empty() {
            Ok(vmi)
        } else {
            Err("Local VMI profiles do not use a VMI account.".into())
        };
    }
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

async fn request(
    port: u16,
    path: &str,
    content_type: &str,
    payload: String,
    cookie: Option<&str>,
) -> Result<(StatusCode, HeaderMap), String> {
    tokio::time::timeout(Duration::from_secs(50), async {
        let tcp = TcpStream::connect(("127.0.0.1", port))
            .await
            .map_err(|_| "Could not reach VMI through the tunnel.")?;
        let (mut sender, connection) = http1::handshake(TokioIo::new(tcp))
            .await
            .map_err(|_| "Could not open the VMI connection.")?;
        let connection = tokio::spawn(connection);
        let mut builder = Request::post(path)
            .header("host", format!("127.0.0.1:{port}"))
            .header("content-type", content_type)
            .header("connection", "close");
        if let Some(cookie) = cookie {
            builder = builder.header("cookie", cookie);
        }
        let result = async {
            let body = Full::new(Bytes::from(payload));
            let outgoing = builder.body(body).map_err(|_| "Invalid desktop request.")?;
            let response = sender
                .send_request(outgoing)
                .await
                .map_err(|_| "Could not send the desktop request.")?;
            let (head, body) = response.into_parts();
            Limited::new(body, 16384)
                .collect()
                .await
                .map_err(|_| "Could not read the VMI response.")?;
            Ok((head.status, head.headers))
        }
        .await;
        connection.abort();
        result.map_err(str::to_string)
    })
    .await
    .map_err(|_| "VMI did not respond in time.".to_owned())?
}

fn session_cookie(headers: &HeaderMap) -> Option<&str> {
    headers
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|value| {
            let (pair, _) = value.to_str().ok()?.split_once(';')?;
            let (name, token) = pair.split_once('=')?;
            (name == LOGIN_COOKIE && valid_token(token)).then_some(pair)
        })
        .next()
}

#[derive(Deserialize)]
pub struct SaveRequest {
    profile: Profile,
    vmi_password: Option<String>,
    api_key: Option<String>,
}

pub async fn save(
    State(state): State<SharedState>,
    Json(mut data): Json<SaveRequest>,
) -> Result<Json<ProfileStore>, ApiError> {
    if let Some(password) = data.vmi_password.filter(|value| !value.is_empty()) {
        if data.profile.mode != ProfileMode::Ssh {
            return Err(ApiError("A local profile needs no VMI password.".into()));
        }
        let vmi = vmi_profile(&data.profile).map_err(ApiError)?;
        if password.chars().count() > 1024 || password.contains('\0') {
            return Err(ApiError("Enter a valid VMI account password.".into()));
        }
        secrets::save(
            &key_id("password", &data.profile, vmi, "").map_err(ApiError)?,
            &password,
        )
        .map_err(ApiError)?;
    }
    if let Some(key) = data.api_key.filter(|key| !key.is_empty()) {
        let vmi = vmi_profile(&data.profile).map_err(ApiError)?;
        let agent = vmi
            .agent
            .as_ref()
            .ok_or_else(|| ApiError("Enter the model endpoint and model ID.".into()))?;
        if key.len() > 16384 || key.contains(['\r', '\n', '\0']) || agent.base_url.is_empty() {
            return Err(ApiError("Enter a valid model endpoint and API key.".into()));
        }
        secrets::save(
            &key_id("model", &data.profile, vmi, &agent.base_url).map_err(ApiError)?,
            &key,
        )
        .map_err(ApiError)?;
    }
    save_profile(&state, &mut data.profile)
        .map(Json)
        .map_err(ApiError)
}

pub async fn open(
    State(state): State<SharedState>,
    Json(profile): Json<Profile>,
) -> Result<impl IntoResponse, ApiError> {
    if profile.mode != ProfileMode::Ssh {
        return Err(ApiError("Use Open VMI for a local profile.".into()));
    }
    let vmi = vmi_profile(&profile).map_err(ApiError)?;
    let _connection = state.connection.lock().await;
    let port = active_port(&state, &profile).map_err(ApiError)?;
    let password = secrets::load(&key_id("password", &profile, vmi, "").map_err(ApiError)?)
        .map_err(ApiError)?
        .ok_or_else(|| {
            ApiError("Save the VMI account password in the client profile first.".into())
        })?;
    let form = form_urlencoded::Serializer::new(String::new())
        .append_pair("username", &vmi.account)
        .append_pair("password", &password)
        .finish();
    let (status, headers) = request(
        port,
        "/login",
        "application/x-www-form-urlencoded",
        form,
        None,
    )
    .await
    .map_err(ApiError)?;
    match status {
        StatusCode::SEE_OTHER => (),
        StatusCode::UNAUTHORIZED => {
            return Err(ApiError(
                "VMI rejected the saved account name or password.".into(),
            ))
        }
        StatusCode::TOO_MANY_REQUESTS => {
            return Err(ApiError(
                "VMI sign-in is temporarily rate-limited. Try again shortly.".into(),
            ))
        }
        _ => {
            return Err(ApiError(
                "The VMI server did not accept the sign-in request.".into(),
            ))
        }
    }
    let session = session_cookie(&headers)
        .ok_or_else(|| ApiError("The VMI server did not return a valid session cookie.".into()))?;
    let (agent, mut warning) = match model_payload(&profile, vmi) {
        Ok(agent) => (agent, None),
        Err(message) => (None, Some(message)),
    };
    if let Some(agent) = agent {
        warning = match request(
            port,
            "/api/agent/session",
            "application/json",
            agent.to_string(),
            Some(session),
        )
        .await
        {
            Ok((StatusCode::OK, _)) => None,
            Ok((StatusCode::FORBIDDEN, _)) => {
                Some("Agent is unavailable in manual-only mode.".into())
            }
            Ok((StatusCode::NOT_FOUND, _)) => {
                Some("This VMI server does not support starting Agent from the client.".into())
            }
            _ => Some("VMI signed in. Agent could not start with the saved settings.".into()),
        };
    }
    // Both listeners use 127.0.0.1. Browsers scope cookies to the host
    // irrespective of port, so the normal VMI session applies after navigation.
    let cookie = format!("{session}; Path=/; HttpOnly; SameSite=Strict");
    Ok((
        [
            (header::SET_COOKIE, cookie),
            (header::CACHE_CONTROL, "no-store".into()),
        ],
        Json(warning),
    ))
}

fn model_payload(profile: &Profile, vmi: &VmiProfile) -> Result<Option<Value>, String> {
    let Some(agent) = &vmi.agent else {
        return Ok(None);
    };
    let key = secrets::load(&key_id("model", profile, vmi, &agent.base_url)?)
        .map_err(|_| "Could not read the model API key from the OS keychain.")?
        .ok_or("Save the model API key for this endpoint in the client profile first.")?;
    Ok(Some(json!({
        "base_url": agent.base_url, "api": agent.api, "model": agent.model,
        "additional_models": agent.additional_models, "image_models": agent.image_models,
        "api_key": key,
    })))
}

pub async fn open_local(
    State(state): State<SharedState>,
    Json(profile): Json<Profile>,
) -> Result<Json<String>, ApiError> {
    if profile.mode != ProfileMode::Local {
        return Err(ApiError("Choose the local connection mode.".into()));
    }
    let checkout = profile.local_checkout().map_err(ApiError)?;
    let vmi = profile
        .vmi
        .as_ref()
        .map(|_| vmi_profile(&profile))
        .transpose()
        .map_err(ApiError)?;
    let model = vmi.and_then(|vmi| vmi.agent.as_ref());
    let mut guis = state.local_guis.lock().await;
    guis.retain_mut(|gui| gui.is_running());
    if let Some(gui) = guis.iter().find(|gui| gui.checkout == checkout) {
        if gui.model.as_ref() != model {
            return Err(ApiError("Close the running VMI GUI before launching this checkout with different Agent settings.".into()));
        }
        return Ok(Json(gui.url()));
    }
    let connection = vmi
        .map(|vmi| model_payload(&profile, vmi))
        .transpose()
        .map_err(ApiError)?
        .flatten();
    let model = model.cloned();
    let gui = tokio::task::spawn_blocking(move || local_gui::launch(checkout, model, connection))
        .await
        .map_err(|_| ApiError("Could not start the local VMI launcher.".into()))?
        .map_err(ApiError)?;
    let url = gui.url();
    guis.push(gui);
    Ok(Json(url))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_the_normal_vmi_session_cookie() {
        let mut headers = HeaderMap::new();
        headers.append(
            header::SET_COOKIE,
            "unrelated=value; Path=/".parse().unwrap(),
        );
        headers.append(
            header::SET_COOKIE,
            "vmi_session=bad; Path=/".parse().unwrap(),
        );
        let token = "A".repeat(43);
        let pair = format!("{LOGIN_COOKIE}={token}");
        headers.append(
            header::SET_COOKIE,
            format!("{pair}; Path=/; HttpOnly").parse().unwrap(),
        );
        assert_eq!(session_cookie(&headers), Some(pair.as_str()));
    }
}
