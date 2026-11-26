//! SSH connection, authentication and host-key trust.

use std::fmt;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use russh::client::{self, Config, Handle};
use russh::keys::ssh_key::known_hosts::KnownHosts;
use russh::keys::{self, PrivateKeyWithHashAlg, PublicKeyOrCertificate};

/// How long to wait for the TCP + key-exchange handshake.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);
/// How long to wait for the server to answer an authentication attempt.
const AUTH_TIMEOUT: Duration = Duration::from_secs(20);
/// Keepalive cadence, so dead networks are detected while idle.
const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(20);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SshErrorKind {
    /// The server presented a key that differs from the recorded one.
    HostKeyChanged,
    /// The host key could not be compared or recorded.
    HostKeyRejected,
    /// The server rejected the credentials.
    Authentication,
    /// The key file is missing, unreadable or encrypted with a wrong passphrase.
    KeyFile,
    /// SSH authenticated, but could not open the configured forwarding channel.
    Forwarding,
    /// The connection failed or dropped; retrying may help.
    Network,
}

#[derive(Debug, Clone)]
pub struct SshError {
    pub kind: SshErrorKind,
    pub message: String,
}

impl SshError {
    fn new(kind: SshErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub fn forwarding(message: impl Into<String>) -> Self {
        Self::new(SshErrorKind::Forwarding, message)
    }

    /// Whether reconnecting could reasonably succeed.
    pub fn retryable(&self) -> bool {
        matches!(self.kind, SshErrorKind::Network)
    }
}

impl fmt::Display for SshError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

/// Credentials resolved for one session. Kept in memory only, and reused when
/// the supervisor reconnects.
#[derive(Clone)]
pub enum AuthMaterial {
    Password(String),
    KeyFile {
        path: PathBuf,
        passphrase: Option<String>,
    },
}

impl fmt::Debug for AuthMaterial {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Password(_) => f.write_str("Password(<redacted>)"),
            Self::KeyFile { path, passphrase } => f
                .debug_struct("KeyFile")
                .field("path", path)
                .field("passphrase", &passphrase.is_some())
                .finish(),
        }
    }
}

/// What the client observed about the server's host key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostKeyOutcome {
    /// Trusted, because it matches the recorded key.
    Trusted,
    /// Not recorded yet; accepted once and recorded after authentication.
    Learned,
    /// Differs from the recorded key. The connection is refused.
    Changed,
    /// Could not be checked.
    Unreadable(String),
}

/// Everything needed to open one SSH session.
#[derive(Debug, Clone)]
pub struct ConnectSpec {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub auth: AuthMaterial,
}

pub struct ClientHandler {
    host: String,
    port: u16,
    known_hosts: PathBuf,
    outcome: Arc<Mutex<Option<HostKeyOutcome>>>,
    observed: Arc<Mutex<Option<keys::PublicKey>>>,
}

fn check_host_key(host: &str, port: u16, path: &Path, key: &keys::PublicKey) -> HostKeyOutcome {
    // russh's reader treats any file-open failure as an empty host store.
    let records = match std::fs::read_to_string(path) {
        Ok(records) => records,
        Err(err) if err.kind() == ErrorKind::NotFound => return HostKeyOutcome::Learned,
        Err(err) => return HostKeyOutcome::Unreadable(err.to_string()),
    };
    for entry in KnownHosts::new(&records) {
        match entry {
            Ok(entry) if entry.marker().is_none() => (),
            Ok(_) => {
                return HostKeyOutcome::Unreadable(
                    "Unsupported marker in the client host-key store.".into(),
                )
            }
            Err(err) => return HostKeyOutcome::Unreadable(err.to_string()),
        }
    }
    match keys::known_hosts::known_host_keys_path(host, port, path) {
        Ok(keys) if keys.is_empty() => HostKeyOutcome::Learned,
        Ok(keys)
            if keys
                .iter()
                .any(|(_, recorded)| recorded.key_data() == key.key_data()) =>
        {
            HostKeyOutcome::Trusted
        }
        Ok(_) => HostKeyOutcome::Changed,
        Err(err) => HostKeyOutcome::Unreadable(err.to_string()),
    }
}

impl client::Handler for ClientHandler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        let key = match server_public_key {
            PublicKeyOrCertificate::PublicKey { key, .. } => key.clone(),
            PublicKeyOrCertificate::Certificate(cert) => cert.public_key().clone().into(),
        };

        if let Ok(mut observed) = self.observed.lock() {
            *observed = Some(key.clone());
        }

        let outcome = check_host_key(&self.host, self.port, &self.known_hosts, &key);
        let accepted = matches!(outcome, HostKeyOutcome::Trusted | HostKeyOutcome::Learned);

        let Ok(mut slot) = self.outcome.lock() else {
            return Ok(false);
        };
        *slot = Some(outcome);
        Ok(accepted)
    }
}

/// Open and authenticate an SSH session.
///
/// Returns the session handle and what happened with the host key.
pub async fn connect(
    spec: &ConnectSpec,
    known_hosts: &Path,
) -> Result<(Handle<ClientHandler>, HostKeyOutcome), SshError> {
    if let Some(parent) = known_hosts.parent() {
        std::fs::create_dir_all(parent).map_err(|err| {
            SshError::new(
                SshErrorKind::HostKeyRejected,
                format!("Could not create {}: {err}", parent.display()),
            )
        })?;
    }

    let config = Arc::new(Config {
        keepalive_interval: Some(KEEPALIVE_INTERVAL),
        keepalive_max: 3,
        inactivity_timeout: None,
        nodelay: true,
        ..Default::default()
    });

    let outcome = Arc::new(Mutex::new(None));
    let observed = Arc::new(Mutex::new(None));
    let handler = ClientHandler {
        host: spec.host.clone(),
        port: spec.port,
        known_hosts: known_hosts.to_path_buf(),
        outcome: Arc::clone(&outcome),
        observed: Arc::clone(&observed),
    };

    let mut handle = match tokio::time::timeout(
        CONNECT_TIMEOUT,
        client::connect(config, (spec.host.as_str(), spec.port), handler),
    )
    .await
    {
        Err(_) => {
            return Err(SshError::new(
                SshErrorKind::Network,
                format!(
                    "Timed out connecting to {}:{} after {}s.",
                    spec.host,
                    spec.port,
                    CONNECT_TIMEOUT.as_secs()
                ),
            ))
        }
        Ok(Err(err)) => {
            let seen = outcome.lock().ok().and_then(|slot| slot.clone());
            return Err(match seen {
                Some(HostKeyOutcome::Changed) => SshError::new(
                    SshErrorKind::HostKeyChanged,
                    format!(
                        "The host key for {}:{} does not match the recorded key. The connection was refused.",
                        spec.host, spec.port
                    ),
                ),
                Some(HostKeyOutcome::Unreadable(reason)) => SshError::new(
                    SshErrorKind::HostKeyRejected,
                    format!("Could not verify the host key: {reason}"),
                ),
                _ => SshError::new(
                    SshErrorKind::Network,
                    format!("Could not connect to {}:{}: {err}", spec.host, spec.port),
                ),
            });
        }
        Ok(Ok(handle)) => handle,
    };

    authenticate(&mut handle, spec).await?;

    let seen = outcome
        .lock()
        .ok()
        .and_then(|slot| slot.clone())
        .ok_or_else(|| {
            SshError::new(
                SshErrorKind::HostKeyRejected,
                "Could not verify the server host key.",
            )
        })?;
    if seen == HostKeyOutcome::Learned {
        let key = observed
            .lock()
            .ok()
            .and_then(|slot| slot.clone())
            .ok_or_else(|| {
                SshError::new(
                    SshErrorKind::HostKeyRejected,
                    "The server host key was not recorded.",
                )
            })?;
        keys::known_hosts::learn_known_hosts_path(&spec.host, spec.port, &key, known_hosts)
            .map_err(|err| {
                SshError::new(
                    SshErrorKind::HostKeyRejected,
                    format!("Could not save the server host key: {err}"),
                )
            })?;
    }

    Ok((handle, seen))
}

async fn authenticate(
    handle: &mut Handle<ClientHandler>,
    spec: &ConnectSpec,
) -> Result<(), SshError> {
    let result = match &spec.auth {
        AuthMaterial::Password(password) => tokio::time::timeout(
            AUTH_TIMEOUT,
            handle.authenticate_password(&spec.user, password),
        )
        .await
        .map_err(|_| {
            SshError::new(
                SshErrorKind::Authentication,
                "The SSH server did not answer the password authentication in time.",
            )
        })?
        .map_err(|err| {
            SshError::new(
                SshErrorKind::Network,
                format!("SSH authentication failed: {err}"),
            )
        })?,
        AuthMaterial::KeyFile { path, passphrase } => {
            let key = keys::load_secret_key(path, passphrase.as_deref()).map_err(|err| {
                SshError::new(
                    SshErrorKind::KeyFile,
                    format!(
                        "Could not read the SSH key {}: {err}. If the key is encrypted, enter its passphrase.",
                        path.display()
                    ),
                )
            })?;

            let hash = if key.algorithm().is_rsa() {
                handle
                    .best_supported_rsa_hash()
                    .await
                    .map_err(|err| {
                        SshError::new(
                            SshErrorKind::Network,
                            format!("Could not negotiate an RSA signature algorithm: {err}"),
                        )
                    })?
                    .flatten()
            } else {
                None
            };

            let key = PrivateKeyWithHashAlg::new(Arc::new(key), hash);
            tokio::time::timeout(AUTH_TIMEOUT, handle.authenticate_publickey(&spec.user, key))
                .await
                .map_err(|_| {
                    SshError::new(
                        SshErrorKind::Authentication,
                        "The SSH server did not answer the key authentication in time.",
                    )
                })?
                .map_err(|err| {
                    SshError::new(
                        SshErrorKind::Network,
                        format!("SSH authentication failed: {err}"),
                    )
                })?
        }
    };

    match result {
        russh::client::AuthResult::Success => Ok(()),
        russh::client::AuthResult::Failure { .. } => Err(SshError::new(
            SshErrorKind::Authentication,
            format!(
                "The server rejected the credentials for {}@{}.",
                spec.user, spec.host
            ),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_material_debug_redacts_secrets() {
        let password = AuthMaterial::Password("hunter2".into());
        assert_eq!(format!("{password:?}"), "Password(<redacted>)");

        let key = AuthMaterial::KeyFile {
            path: PathBuf::from("id_ed25519"),
            passphrase: Some("secret".into()),
        };
        let text = format!("{key:?}");
        assert!(!text.contains("secret"));
        assert!(text.contains("id_ed25519"));
    }

    #[test]
    fn network_errors_are_retryable_and_others_are_not() {
        assert!(SshError::new(SshErrorKind::Network, "reset").retryable());
        assert!(!SshError::new(SshErrorKind::Authentication, "denied").retryable());
        assert!(!SshError::new(SshErrorKind::HostKeyChanged, "changed").retryable());
    }
}
