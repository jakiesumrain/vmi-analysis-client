//! OS-keychain storage for SSH passwords and key-file passphrases.
//!
//! Nothing here is written to the profiles file. On Windows the entries live in
//! Credential Manager, on macOS in Keychain, and on Linux in the Secret Service.

use keyring::{Entry, Error as KeyringError};

use crate::config::Profile;

/// Keychain service name. Keep it stable: changing it orphans saved entries.
const SERVICE: &str = "com.vmi.analysis.client";

fn password_account(profile: &Profile) -> String {
    format!(
        "ssh-password:{}@{}:{}",
        profile.user,
        profile.host.trim(),
        profile.port
    )
}

fn passphrase_account(key_path: &str) -> String {
    format!("ssh-key-passphrase:{key_path}")
}

fn entry(account: &str) -> Result<Entry, String> {
    Entry::new(SERVICE, account).map_err(|err| format!("Keychain is unavailable: {err}"))
}

pub(crate) fn save(account: &str, secret: &str) -> Result<(), String> {
    entry(account)?
        .set_password(secret)
        .map_err(|err| format!("Could not save to the keychain: {err}"))
}

pub(crate) fn load(account: &str) -> Result<Option<String>, String> {
    match entry(account)?.get_password() {
        Ok(secret) => Ok(Some(secret)),
        Err(KeyringError::NoEntry) => Ok(None),
        Err(err) => Err(format!("Could not read from the keychain: {err}")),
    }
}

fn delete(account: &str) -> Result<(), String> {
    match entry(account)?.delete_credential() {
        Ok(()) => Ok(()),
        Err(KeyringError::NoEntry) => Ok(()),
        Err(err) => Err(format!("Could not remove the keychain entry: {err}")),
    }
}

pub fn save_password(profile: &Profile, password: &str) -> Result<(), String> {
    save(&password_account(profile), password)
}

pub fn load_password(profile: &Profile) -> Result<Option<String>, String> {
    load(&password_account(profile))
}

pub fn delete_password(profile: &Profile) -> Result<(), String> {
    delete(&password_account(profile))
}

pub fn save_passphrase(key_path: &str, passphrase: &str) -> Result<(), String> {
    save(&passphrase_account(key_path), passphrase)
}

pub fn load_passphrase(key_path: &str) -> Result<Option<String>, String> {
    load(&passphrase_account(key_path))
}

pub fn delete_passphrase(key_path: &str) -> Result<(), String> {
    delete(&passphrase_account(key_path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AuthMethod;

    fn profile() -> Profile {
        Profile {
            name: "Lab".into(),
            mode: crate::config::ProfileMode::Ssh,
            host: "vmi.example.org".into(),
            port: 2222,
            user: "alice".into(),
            auth: AuthMethod::Password,
            remote_host: "127.0.0.1".into(),
            remote_port: 13517,
            local_port: 0,
            remember_password: true,
            remember_passphrase: false,
            vmi: None,
        }
    }

    #[test]
    fn password_account_is_scoped_to_user_and_host() {
        let account = password_account(&profile());
        assert_eq!(account, "ssh-password:alice@vmi.example.org:2222");
    }

    #[test]
    fn passphrase_account_is_scoped_to_the_key_path() {
        assert_eq!(
            passphrase_account("C:/keys/id_ed25519"),
            "ssh-key-passphrase:C:/keys/id_ed25519"
        );
    }
}
