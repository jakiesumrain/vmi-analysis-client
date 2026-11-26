//! Profiles and their on-disk store.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The server web application always listens on loopback; this is its port.
pub const DEFAULT_REMOTE_HOST: &str = "127.0.0.1";
pub const DEFAULT_REMOTE_PORT: u16 = 13517;
pub const DEFAULT_SSH_PORT: u16 = 22;

fn default_ssh_port() -> u16 {
    DEFAULT_SSH_PORT
}

fn default_remote_host() -> String {
    DEFAULT_REMOTE_HOST.to_owned()
}

fn default_remote_port() -> u16 {
    DEFAULT_REMOTE_PORT
}

/// How the client authenticates to the SSH server.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AuthMethod {
    /// Password authentication. The password itself is never stored in the
    /// profile; it lives in the OS keychain when the user asks to remember it.
    #[default]
    Password,
    /// Public-key authentication from a key file on this computer.
    KeyFile { path: String },
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProfileMode {
    #[default]
    Ssh,
    Local,
}

/// A saved connection profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    /// Display name. Defaults to `user@host` when the user leaves it empty.
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub mode: ProfileMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkout: Option<String>,
    #[serde(default)]
    pub host: String,
    #[serde(default = "default_ssh_port")]
    pub port: u16,
    #[serde(default)]
    pub user: String,
    #[serde(default)]
    pub auth: AuthMethod,
    /// Host to connect to from the server, normally loopback.
    #[serde(default = "default_remote_host")]
    pub remote_host: String,
    /// Port of the VMI web application on the server.
    #[serde(default = "default_remote_port")]
    pub remote_port: u16,
    /// Local loopback port, or 0 to let the operating system choose one.
    #[serde(default)]
    pub local_port: u16,
    /// Keep the SSH password in the OS keychain.
    #[serde(default)]
    pub remember_password: bool,
    /// Keep an encrypted key file's passphrase in the OS keychain.
    #[serde(default)]
    pub remember_passphrase: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vmi: Option<crate::desktop::VmiProfile>,
}

impl Profile {
    pub fn display_name(&self) -> String {
        if self.name.trim().is_empty() {
            match self.mode {
                ProfileMode::Ssh => format!("{}@{}", self.user, self.host),
                ProfileMode::Local => "Local VMI".into(),
            }
        } else {
            self.name.trim().to_owned()
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.mode == ProfileMode::Local {
            self.local_checkout()?;
            return Ok(());
        }
        if self.remote_port == 0 {
            return Err("The VMI web port must be between 1 and 65535.".into());
        }
        let host = self.host.trim();
        if host.is_empty() {
            return Err("A server host name or address is required.".into());
        }
        if host.chars().any(char::is_whitespace) {
            return Err("The server host must not contain spaces.".into());
        }
        if self.user.trim().is_empty() {
            return Err("An SSH user name is required.".into());
        }
        if self.port == 0 {
            return Err("The SSH port must be between 1 and 65535.".into());
        }
        if self.remote_host.trim().is_empty() {
            return Err("The server target address is required.".into());
        }
        if let AuthMethod::KeyFile { path } = &self.auth {
            if path.trim().is_empty() {
                return Err("Choose an SSH key file, or switch to password authentication.".into());
            }
        }
        Ok(())
    }

    pub fn local_checkout(&self) -> Result<PathBuf, String> {
        let path = expand_path(self.checkout.as_deref().unwrap_or_default());
        if !path.is_absolute() {
            return Err("Choose the folder containing your VMI Analysis GUI checkout.".into());
        }
        let path = path
            .canonicalize()
            .map_err(|_| "The VMI Analysis GUI folder could not be opened.")?;
        if !path.join("pyproject.toml").is_file()
            || !path.join("src/vmi_analysis/gui/app.py").is_file()
        {
            return Err("Choose the VMI Analysis GUI repository root.".into());
        }
        Ok(path)
    }

    /// The local URL used by SSH and local launches.
    pub fn local_url(&self, local_port: u16) -> String {
        format!("http://127.0.0.1:{local_port}")
    }
}

/// The saved profile collection.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProfileStore {
    #[serde(default)]
    pub profiles: Vec<Profile>,
    #[serde(default)]
    pub last_used: Option<String>,
}

impl ProfileStore {
    pub fn load(path: &Path) -> Result<Self, String> {
        match std::fs::read_to_string(path) {
            Ok(text) => serde_json::from_str(&text)
                .map_err(|err| format!("Could not read profiles from {}: {err}", path.display())),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(err) => Err(format!(
                "Could not read profiles from {}: {err}",
                path.display()
            )),
        }
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|err| format!("Could not create {}: {err}", parent.display()))?;
        }
        let text = serde_json::to_string_pretty(self)
            .map_err(|err| format!("Could not serialize profiles: {err}"))?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, text)
            .map_err(|err| format!("Could not write {}: {err}", tmp.display()))?;
        std::fs::rename(&tmp, path)
            .map_err(|err| format!("Could not replace {}: {err}", path.display()))?;
        Ok(())
    }

    /// Insert or replace a profile by name.
    pub fn upsert(&mut self, profile: Profile) {
        if let Some(existing) = self
            .profiles
            .iter_mut()
            .find(|item| item.display_name() == profile.display_name())
        {
            *existing = profile;
        } else {
            self.profiles.push(profile);
        }
    }

    pub fn remove(&mut self, name: &str) -> bool {
        let before = self.profiles.len();
        self.profiles.retain(|item| item.display_name() != name);
        self.profiles.len() != before
    }

    pub fn get(&self, name: &str) -> Option<&Profile> {
        self.profiles
            .iter()
            .find(|item| item.display_name() == name)
    }
}

/// Expand a leading `~` in a key path typed by the user.
pub fn expand_path(path: &str) -> PathBuf {
    let path = path.trim();
    let rest = path.strip_prefix("~/").or_else(|| path.strip_prefix("~\\"));
    match rest {
        Some(rest) => match home_dir() {
            Some(home) => home.join(rest),
            None => PathBuf::from(path),
        },
        None => PathBuf::from(path),
    }
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Profile {
        Profile {
            name: "Lab server".into(),
            mode: ProfileMode::Ssh,
            checkout: None,
            host: "vmi.example.org".into(),
            port: 22,
            user: "alice".into(),
            auth: AuthMethod::Password,
            remote_host: DEFAULT_REMOTE_HOST.into(),
            remote_port: DEFAULT_REMOTE_PORT,
            local_port: 0,
            remember_password: false,
            remember_passphrase: false,
            vmi: None,
        }
    }

    #[test]
    fn defaults_target_the_vmi_port() {
        let profile: Profile = serde_json::from_str(r#"{"host":"h","user":"u"}"#).expect("parse");
        assert_eq!(profile.mode, ProfileMode::Ssh);
        assert_eq!(profile.port, DEFAULT_SSH_PORT);
        assert_eq!(profile.remote_host, DEFAULT_REMOTE_HOST);
        assert_eq!(profile.remote_port, DEFAULT_REMOTE_PORT);
        assert_eq!(profile.local_port, 0);
        assert_eq!(profile.auth, AuthMethod::Password);
    }

    #[test]
    fn local_profile_requires_a_checkout() {
        let profile: Profile = serde_json::from_str(r#"{"mode":"local"}"#).expect("parse");
        assert_eq!(profile.display_name(), "Local VMI");
        assert!(profile.validate().is_err());
    }

    #[test]
    fn display_name_falls_back_to_user_and_host() {
        let mut profile = sample();
        profile.name = "   ".into();
        assert_eq!(profile.display_name(), "alice@vmi.example.org");
    }

    #[test]
    fn validation_rejects_missing_fields() {
        let mut profile = sample();
        profile.host = " ".into();
        assert!(profile.validate().is_err());

        let mut profile = sample();
        profile.user = "".into();
        assert!(profile.validate().is_err());

        let mut profile = sample();
        profile.auth = AuthMethod::KeyFile { path: " ".into() };
        assert!(profile.validate().is_err());

        assert!(sample().validate().is_ok());
    }

    #[test]
    fn upsert_replaces_by_display_name() {
        let mut store = ProfileStore::default();
        let mut first = sample();
        first.local_port = 13517;
        store.upsert(first);

        let mut second = sample();
        second.local_port = 14000;
        store.upsert(second);

        assert_eq!(store.profiles.len(), 1);
        assert_eq!(store.profiles[0].local_port, 14000);
        assert!(store.remove("Lab server"));
        assert!(store.profiles.is_empty());
    }

    #[test]
    fn store_round_trips_through_disk() {
        let dir = std::env::temp_dir().join(format!("vmi-client-test-{}", std::process::id()));
        let path = dir.join("profiles.json");
        let mut store = ProfileStore::default();
        store.upsert(sample());
        store.last_used = Some("Lab server".into());
        store.save(&path).expect("save");
        let loaded = ProfileStore::load(&path).expect("load");
        assert_eq!(loaded.profiles.len(), 1);
        assert_eq!(loaded.last_used.as_deref(), Some("Lab server"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_store_is_empty() {
        let path = std::env::temp_dir().join("vmi-client-does-not-exist.json");
        let _ = std::fs::remove_file(&path);
        let store = ProfileStore::load(&path).expect("load");
        assert!(store.profiles.is_empty());
    }

    #[test]
    fn local_url_is_loopback() {
        assert_eq!(sample().local_url(14001), "http://127.0.0.1:14001");
    }
}
