//! Parent→host access JWT inbox (x009f).
//!
//! Guarantees: latest push coalesces; credential worker takes ownership of the newest
//! payload. Non-guarantees: JWT signature checks (parent IPC trust boundary).

use alchemy_protocol::host_ipc::AuthPush;
use std::sync::{Arc, Mutex};

/// Shared inbox for parent `AUTH_PUSH` lines.
#[derive(Clone, Default)]
pub struct AuthBrokerInbox {
    inner: Arc<Mutex<Option<AuthPush>>>,
}

impl AuthBrokerInbox {
    pub fn new() -> Self {
        Self::default()
    }

    /// Store the latest push. `Err` on poisoned mutex — caller must not ACK success to parent.
    pub fn store(&self, push: AuthPush) -> Result<(), AuthBrokerStoreError> {
        match self.inner.lock() {
            Ok(mut g) => {
                *g = Some(push);
                Ok(())
            }
            Err(_) => {
                log::error!("AuthBrokerInbox poisoned; dropping AUTH_PUSH (do not ACK OK)");
                Err(AuthBrokerStoreError::Poisoned)
            }
        }
    }

    /// Take the latest pending push (overwrite coalesces; only newest kept).
    pub fn take(&self) -> Option<AuthPush> {
        self.inner.lock().ok().and_then(|mut g| g.take())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthBrokerStoreError {
    Poisoned,
}

impl std::fmt::Display for AuthBrokerStoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Poisoned => write!(f, "AuthBrokerInbox mutex poisoned"),
        }
    }
}

impl std::error::Error for AuthBrokerStoreError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn store_take_coalesces_to_latest() {
        let inbox = AuthBrokerInbox::new();
        inbox
            .store(AuthPush {
                expires_in: 10,
                access_jwt: "a".into(),
            })
            .unwrap();
        inbox
            .store(AuthPush {
                expires_in: 20,
                access_jwt: "b".into(),
            })
            .unwrap();
        let got = inbox.take().expect("push");
        assert_eq!(got.expires_in, 20);
        assert_eq!(got.access_jwt, "b");
        assert!(inbox.take().is_none());
    }
}
