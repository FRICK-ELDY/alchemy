//! Refresh-token OS vault (x00cf / P1-6).
//!
//! Guarantees: `(service=alchemy.refresh, key_id)` set/get/delete via OS credential
//! store (keyring); failures are explicit `Result`s for session-only fallback.
//! Non-guarantees: vault strength / sync / cross-process isolation; in-process memory
//! secrecy after load; availability on headless CI without a Secret Service.

#[cfg(test)]
use std::collections::HashMap;
#[cfg(test)]
use std::sync::Mutex;

pub const SERVICE: &str = "alchemy.refresh";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VaultError {
    Unavailable(String),
    Io(String),
}

impl std::fmt::Display for VaultError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable(msg) => write!(f, "refresh vault unavailable: {msg}"),
            Self::Io(msg) => write!(f, "refresh vault io: {msg}"),
        }
    }
}

impl std::error::Error for VaultError {}

/// Backend for remember-me refresh secrets (OS or in-memory for tests).
pub trait RefreshVault: Send + Sync {
    fn store(&self, key_id: &str, secret: &str) -> Result<(), VaultError>;
    fn load(&self, key_id: &str) -> Result<Option<String>, VaultError>;
    fn delete(&self, key_id: &str) -> Result<(), VaultError>;
}

/// Production backend: Windows Credential Manager / macOS Keychain / Secret Service.
pub struct OsRefreshVault;

impl RefreshVault for OsRefreshVault {
    fn store(&self, key_id: &str, secret: &str) -> Result<(), VaultError> {
        let entry = keyring::Entry::new(SERVICE, key_id).map_err(map_keyring_err)?;
        entry.set_password(secret).map_err(map_keyring_err)
    }

    fn load(&self, key_id: &str) -> Result<Option<String>, VaultError> {
        let entry = keyring::Entry::new(SERVICE, key_id).map_err(map_keyring_err)?;
        match entry.get_password() {
            Ok(secret) => Ok(Some(secret)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(err) => Err(map_keyring_err(err)),
        }
    }

    fn delete(&self, key_id: &str) -> Result<(), VaultError> {
        let entry = keyring::Entry::new(SERVICE, key_id).map_err(map_keyring_err)?;
        match entry.delete_credential() {
            Ok(()) => Ok(()),
            Err(keyring::Error::NoEntry) => Ok(()),
            Err(err) => Err(map_keyring_err(err)),
        }
    }
}

fn map_keyring_err(err: keyring::Error) -> VaultError {
    match err {
        keyring::Error::NoDefaultStore => {
            VaultError::Unavailable("no default credential store".into())
        }
        other => VaultError::Io(other.to_string()),
    }
}

/// Stable key id stored in prefs (`refresh_vault_key`).
pub fn key_id_for_username(username: &str) -> String {
    format!("user:{username}")
}

/// In-memory vault for unit tests (CI does not require OS Secret Service).
#[cfg(test)]
#[derive(Default)]
pub struct MemoryRefreshVault {
    map: Mutex<HashMap<String, String>>,
}

#[cfg(test)]
impl MemoryRefreshVault {
    pub fn new() -> Self {
        Self::default()
    }
}

#[cfg(test)]
impl RefreshVault for MemoryRefreshVault {
    fn store(&self, key_id: &str, secret: &str) -> Result<(), VaultError> {
        self.map
            .lock()
            .map_err(|_| VaultError::Io("memory vault lock poisoned".into()))?
            .insert(key_id.to_string(), secret.to_string());
        Ok(())
    }

    fn load(&self, key_id: &str) -> Result<Option<String>, VaultError> {
        Ok(self
            .map
            .lock()
            .map_err(|_| VaultError::Io("memory vault lock poisoned".into()))?
            .get(key_id)
            .cloned())
    }

    fn delete(&self, key_id: &str) -> Result<(), VaultError> {
        self.map
            .lock()
            .map_err(|_| VaultError::Io("memory vault lock poisoned".into()))?
            .remove(key_id);
        Ok(())
    }
}

/// Always-failing vault (session-only path tests).
#[cfg(test)]
pub struct UnavailableRefreshVault;

#[cfg(test)]
impl RefreshVault for UnavailableRefreshVault {
    fn store(&self, _key_id: &str, _secret: &str) -> Result<(), VaultError> {
        Err(VaultError::Unavailable("test unavailable".into()))
    }

    fn load(&self, _key_id: &str) -> Result<Option<String>, VaultError> {
        Err(VaultError::Unavailable("test unavailable".into()))
    }

    fn delete(&self, _key_id: &str) -> Result<(), VaultError> {
        Err(VaultError::Unavailable("test unavailable".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_roundtrip_and_delete() {
        let vault = MemoryRefreshVault::new();
        vault.store("user:a", "rt-1").unwrap();
        assert_eq!(vault.load("user:a").unwrap().as_deref(), Some("rt-1"));
        vault.delete("user:a").unwrap();
        assert_eq!(vault.load("user:a").unwrap(), None);
    }

    #[test]
    fn unavailable_is_session_only_signal() {
        let vault = UnavailableRefreshVault;
        assert!(matches!(
            vault.store("k", "s"),
            Err(VaultError::Unavailable(_))
        ));
    }
}
