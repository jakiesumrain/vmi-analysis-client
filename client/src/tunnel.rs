//! The local TCP listener and the SSH session supervisor.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use russh::client::Handle;
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

use crate::ssh::{self, ClientHandler, ConnectSpec, HostKeyOutcome};
use crate::{emit_log, publish, Status};

/// How often the supervisor checks whether the SSH session is still alive.
const LIVENESS_INTERVAL: Duration = Duration::from_secs(2);
/// Delay before the first reconnect attempt.
const RECONNECT_DELAY: Duration = Duration::from_secs(3);
/// Give up after this many consecutive failed attempts.
const MAX_ATTEMPTS: u32 = 5;
const CHANNEL_TIMEOUT: Duration = Duration::from_secs(10);

/// The current SSH session, or `None` while (re)connecting.
pub type Session = Arc<tokio::sync::Mutex<Option<Handle<ClientHandler>>>>;

/// Where the tunnel sends traffic on the server side.
#[derive(Debug, Clone)]
pub struct Target {
    pub remote_host: String,
    pub remote_port: u16,
}

impl Target {
    async fn open(
        &self,
        handle: &Handle<ClientHandler>,
        local_port: u16,
    ) -> Result<russh::Channel<russh::client::Msg>, String> {
        tokio::time::timeout(
            CHANNEL_TIMEOUT,
            handle.channel_open_direct_tcpip(
                self.remote_host.clone(),
                u32::from(self.remote_port),
                "127.0.0.1",
                u32::from(local_port),
            ),
        )
        .await
        .map_err(|_| "Timed out opening the SSH forwarding channel.".to_owned())?
        .map_err(|err| err.to_string())
    }
}

/// Accept browser connections on the local listener and forward each one
/// through the current SSH session.
pub async fn accept_loop(
    state: &crate::AppState,
    cancel: &CancellationToken,
    listener: TcpListener,
    session: &Session,
    target: &Arc<Target>,
) {
    let mut forwards = JoinSet::new();
    loop {
        tokio::select! {
            biased;
            _ = cancel.cancelled() => break,
            _ = forwards.join_next(), if !forwards.is_empty() => (),
            accepted = listener.accept() => match accepted {
                Ok((tcp, peer)) => {
                    forwards.spawn(forward(tcp, peer, Arc::clone(session), Arc::clone(target)));
                }
                Err(err) => {
                    emit_log(state, "warn", format!("Local listener error: {err}"));
                    tokio::select! {
                        _ = cancel.cancelled() => break,
                        _ = tokio::time::sleep(Duration::from_millis(250)) => (),
                    }
                }
            }
        }
    }
    drop(listener);
    forwards.shutdown().await;
    emit_log(state, "info", "Local listener closed.");
}

async fn forward(mut tcp: TcpStream, peer: SocketAddr, session: Session, target: Arc<Target>) {
    // Hold the session lock only while opening the channel. If the tunnel is
    // reconnecting there is no session yet, so the browser connection is
    // dropped and the page reload will try again.
    let channel = {
        let guard = session.lock().await;
        match guard.as_ref() {
            Some(handle) if !handle.is_closed() => target.open(handle, peer.port()).await.ok(),
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
    state: &crate::AppState,
    session: &Session,
    target: &Target,
    spec: ConnectSpec,
    url: String,
    local_port: u16,
) {
    let mut first_attempt = true;
    let mut attempt: u32 = 0;

    loop {
        publish(
            state,
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

        let connected = async {
            let (handle, host_key) = ssh::connect(&spec, &state.known_hosts_path).await?;
            let channel = target.open(&handle, local_port).await.map_err(|err| {
                ssh::SshError::forwarding(format!(
                    "SSH connected, but forwarding to {}:{} failed: {err}",
                    target.remote_host, target.remote_port
                ))
            })?;
            let _ = tokio::time::timeout(CHANNEL_TIMEOUT, channel.close()).await;
            Ok::<_, ssh::SshError>((handle, host_key))
        };
        match connected.await {
            Ok((handle, host_key)) => {
                first_attempt = false;
                *session.lock().await = Some(handle);

                if host_key == HostKeyOutcome::Learned {
                    emit_log(
                        state,
                        "info",
                        "New server host key recorded; future connections verify it.",
                    );
                }
                emit_log(
                    state,
                    "info",
                    format!("Tunnel up: {url} -> {}:{}", spec.host, spec.port),
                );
                publish(
                    state,
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

                *session.lock().await = None;
                attempt = 0;
                emit_log(state, "warn", "The SSH session closed. Reconnecting…");
                publish(
                    state,
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
                first_attempt = false;

                if !err.retryable() {
                    emit_log(state, "error", err.message.clone());
                    publish(
                        state,
                        Status {
                            state: "failed".into(),
                            message: err.message,
                            url: None,
                            local_port: None,
                        },
                    );
                    return;
                }

                attempt += 1;
                if attempt >= MAX_ATTEMPTS {
                    let message = format!("{} Gave up after {MAX_ATTEMPTS} attempts.", err.message);
                    emit_log(state, "error", message.clone());
                    publish(
                        state,
                        Status {
                            state: "failed".into(),
                            message,
                            url: None,
                            local_port: None,
                        },
                    );
                    return;
                }

                let message = format!(
                    "{} Retrying in {}s (attempt {attempt}/{MAX_ATTEMPTS}).",
                    err.message,
                    RECONNECT_DELAY.as_secs()
                );
                emit_log(state, "warn", message.clone());
                publish(
                    state,
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
}

#[cfg(test)]
mod tests;
