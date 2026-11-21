//! The local TCP listener and the SSH session supervisor.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use russh::client::Handle;
use tokio::net::{TcpListener, TcpStream};
use tokio_util::sync::CancellationToken;

use crate::ssh::{self, ClientHandler, ConnectSpec, HostKeyOutcome};
use crate::{emit_log, publish, Status};

/// How often the supervisor checks whether the SSH session is still alive.
const LIVENESS_INTERVAL: Duration = Duration::from_secs(2);
/// Delay before the first reconnect attempt.
const RECONNECT_DELAY: Duration = Duration::from_secs(3);
/// Give up after this many consecutive failed attempts.
const MAX_ATTEMPTS: u32 = 5;

/// The current SSH session, or `None` while (re)connecting.
pub type Session = Arc<tokio::sync::Mutex<Option<Handle<ClientHandler>>>>;

/// Where the tunnel sends traffic on the server side.
#[derive(Debug, Clone)]
pub struct Target {
    pub remote_host: String,
    pub remote_port: u16,
}

/// Accept browser connections on the local listener and forward each one
/// through the current SSH session.
pub async fn accept_loop(
    state: crate::SharedState,
    cancel: CancellationToken,
    listener: TcpListener,
    session: Session,
    target: Arc<Target>,
) {
    loop {
        tokio::select! {
            _ = cancel.cancelled() => break,
            accepted = listener.accept() => match accepted {
                Ok((tcp, peer)) => {
                    let session = Arc::clone(&session);
                    let target = Arc::clone(&target);
                    tokio::spawn(async move {
                        forward(tcp, peer, session, target).await;
                    });
                }
                Err(err) => {
                    emit_log(&state, "warn", format!("Local listener error: {err}"));
                    tokio::time::sleep(Duration::from_millis(250)).await;
                }
            }
        }
    }
    emit_log(&state, "info", "Local listener closed.");
}

async fn forward(mut tcp: TcpStream, peer: SocketAddr, session: Session, target: Arc<Target>) {
    // Hold the session lock only while opening the channel. If the tunnel is
    // reconnecting there is no session yet, so the browser connection is
    // dropped and the page reload will try again.
    let channel = {
        let guard = session.lock().await;
        match guard.as_ref() {
            Some(handle) if !handle.is_closed() => handle
                .channel_open_direct_tcpip(
                    target.remote_host.clone(),
                    u32::from(target.remote_port),
                    "127.0.0.1",
                    u32::from(peer.port()),
                )
                .await
                .ok(),
            _ => None,
        }
    };

    let Some(channel) = channel else {
        return;
    };

    let mut stream = channel.into_stream();
    let _ = tokio::io::copy_bidirectional(&mut tcp, &mut stream).await;
}

/// Keep one SSH session alive for the lifetime of the tunnel: connect,
/// authenticate, detect loss, and reconnect with a bounded number of attempts.
pub async fn supervise(
    state: crate::SharedState,
    cancel: CancellationToken,
    session: Session,
    spec: ConnectSpec,
    url: String,
    local_port: u16,
) {
    let mut first_attempt = true;
    let mut attempt: u32 = 0;

    loop {
        if cancel.is_cancelled() {
            break;
        }

        publish(
            &state,
            Status {
                state: if first_attempt {
                    "connecting".into()
                } else {
                    "reconnecting".into()
                },
                message: if first_attempt {
                    "Opening the SSH tunnel…".into()
                } else {
                    "Reconnecting to the server…".into()
                },
                url: Some(url.clone()),
                local_port: Some(local_port),
            },
        );

        match ssh::connect(&spec, &state.known_hosts_path).await {
            Ok((handle, host_key)) => {
                first_attempt = false;
                *session.lock().await = Some(handle);

                if host_key == HostKeyOutcome::Learned {
                    emit_log(
                        &state,
                        "info",
                        "New server host key recorded; future connections verify it.",
                    );
                }
                emit_log(
                    &state,
                    "info",
                    format!("Tunnel up: {url} -> {}:{}", spec.host, spec.port),
                );
                publish(
                    &state,
                    Status {
                        state: "connected".into(),
                        message: "Tunnel is up.".into(),
                        url: Some(url.clone()),
                        local_port: Some(local_port),
                    },
                );

                // Wait for the session to close or for the user to disconnect.
                loop {
                    tokio::time::sleep(LIVENESS_INTERVAL).await;
                    if cancel.is_cancelled() {
                        break;
                    }
                    let closed = {
                        let guard = session.lock().await;
                        guard
                            .as_ref()
                            .map(|handle| handle.is_closed())
                            .unwrap_or(true)
                    };
                    if closed {
                        break;
                    }
                }

                if cancel.is_cancelled() {
                    break;
                }

                *session.lock().await = None;
                attempt = 0;
                emit_log(&state, "warn", "The SSH session closed. Reconnecting…");
                publish(
                    &state,
                    Status {
                        state: "reconnecting".into(),
                        message: "The SSH session closed. Reconnecting…".into(),
                        url: Some(url.clone()),
                        local_port: Some(local_port),
                    },
                );
                tokio::time::sleep(RECONNECT_DELAY).await;
            }
            Err(err) => {
                if cancel.is_cancelled() {
                    break;
                }

                first_attempt = false;

                if !err.retryable() {
                    emit_log(&state, "error", err.message.clone());
                    publish(
                        &state,
                        Status {
                            state: "failed".into(),
                            message: err.message,
                            url: Some(url.clone()),
                            local_port: Some(local_port),
                        },
                    );
                    abandon(&state, &session);
                    return;
                }

                attempt += 1;
                if attempt >= MAX_ATTEMPTS {
                    let message = format!("{} Gave up after {MAX_ATTEMPTS} attempts.", err.message);
                    emit_log(&state, "error", message.clone());
                    publish(
                        &state,
                        Status {
                            state: "failed".into(),
                            message,
                            url: Some(url.clone()),
                            local_port: Some(local_port),
                        },
                    );
                    abandon(&state, &session);
                    return;
                }

                let message = format!(
                    "{} Retrying in {}s (attempt {attempt}/{MAX_ATTEMPTS}).",
                    err.message,
                    RECONNECT_DELAY.as_secs()
                );
                emit_log(&state, "warn", message.clone());
                publish(
                    &state,
                    Status {
                        state: "reconnecting".into(),
                        message,
                        url: Some(url.clone()),
                        local_port: Some(local_port),
                    },
                );
                tokio::time::sleep(RECONNECT_DELAY).await;
            }
        }
    }

    *session.lock().await = None;
}

/// Drop the active connection slot if it still belongs to this supervisor.
fn abandon(state: &crate::AppState, session: &Session) {
    if let Ok(mut guard) = state.connection.lock() {
        let ours = guard
            .as_ref()
            .map(|active| Arc::ptr_eq(&active.session, session))
            .unwrap_or(false);
        if ours {
            *guard = None;
        }
    }
}

#[cfg(test)]
mod tests;
