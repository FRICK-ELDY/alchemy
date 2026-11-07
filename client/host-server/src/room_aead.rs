//! Shared Room AEAD key material with one-generation grace (x007e).
//!
//! Guarantees: seal always uses current `(key, epoch)`; open tries current then
//! previous while `prev` is within grace (≈ Room Token TTL). Fail-closed after grace.
//! Prev may be seeded from host Token `room_key_prev*` (world `RoomKeyGrace`) when
//! this process did not observe the rotate locally (restart / late sync).

use alchemy_protocol::wire_aead::{self, AeadKind, WireAeadError, ROOM_KEY_LEN};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

/// Default prev grace when Token refresh omits a usable TTL (debug / bypass).
pub const DEFAULT_PREV_GRACE: Duration = Duration::from_secs(300);

type RoomKeyBytes = [u8; ROOM_KEY_LEN];
type PrevKeyEpoch = (RoomKeyBytes, u64);

#[derive(Debug, Clone)]
pub struct RoomAeadKeys {
    current_key: [u8; ROOM_KEY_LEN],
    current_epoch: u64,
    prev_key: Option<[u8; ROOM_KEY_LEN]>,
    prev_epoch: Option<u64>,
    prev_expires_at: Option<Instant>,
}

impl RoomAeadKeys {
    pub fn new(current_key: [u8; ROOM_KEY_LEN], current_epoch: u64) -> Self {
        Self {
            current_key,
            current_epoch,
            prev_key: None,
            prev_epoch: None,
            prev_expires_at: None,
        }
    }

    pub fn current_epoch(&self) -> u64 {
        self.current_epoch
    }

    pub fn current_key(&self) -> &[u8; ROOM_KEY_LEN] {
        &self.current_key
    }

    /// Seal with the live key/epoch (callers clone bytes under the read lock).
    pub fn seal_material(&self) -> ([u8; ROOM_KEY_LEN], u64) {
        (self.current_key, self.current_epoch)
    }

    /// Snapshot for dual-open without mutating (expired prev is skipped, not cleared).
    fn open_material(&self, now: Instant) -> (RoomKeyBytes, u64, Option<PrevKeyEpoch>) {
        let prev = match (self.prev_key, self.prev_epoch, self.prev_expires_at) {
            // Inclusive end matches world `RoomKeyGrace.get` (`now <= exp`).
            (Some(key), Some(epoch), Some(exp)) if now <= exp => Some((key, epoch)),
            _ => None,
        };
        (self.current_key, self.current_epoch, prev)
    }

    /// Clear expired prev under a write lock (best-effort; open path does not require it).
    pub fn gc_expired_prev(&mut self, now: Instant) {
        match self.prev_expires_at {
            Some(exp) if now >= exp => {
                self.prev_key = None;
                self.prev_epoch = None;
                self.prev_expires_at = None;
            }
            _ => {}
        }
    }

    /// Apply Token key/epoch. Returns `true` when current epoch advanced (caller should
    /// force Manifest AEAD re-put).
    ///
    /// `world_prev`: optional grace pair from host Token (`RoomKeyGrace`). Used when
    /// seeding prev after a jump/restart, or when local had no prior generation.
    pub fn apply_token_key(
        &mut self,
        new_key: [u8; ROOM_KEY_LEN],
        new_epoch: u64,
        grace: Duration,
        world_prev: Option<([u8; ROOM_KEY_LEN], u64)>,
    ) -> bool {
        if new_epoch < self.current_epoch {
            log::warn!(
                "room AEAD Token epoch went backwards ({} → {}); ignoring",
                self.current_epoch,
                new_epoch
            );
            return false;
        }
        if new_epoch == self.current_epoch {
            if new_key != self.current_key {
                log::warn!(
                    "room AEAD key bytes changed at same epoch={new_epoch}; adopting new bytes"
                );
                self.current_key = new_key;
            }
            self.maybe_seed_world_prev(world_prev, grace);
            return false;
        }
        // Epoch advanced: current → prev (preferred), else world_prev if contiguous.
        self.prev_key = Some(self.current_key);
        self.prev_epoch = Some(self.current_epoch);
        self.prev_expires_at = Some(Instant::now() + grace);
        log::info!(
            "room AEAD rotated epoch {} → {} (prev grace {}s)",
            self.current_epoch,
            new_epoch,
            grace.as_secs()
        );
        self.current_key = new_key;
        self.current_epoch = new_epoch;
        // If we jumped more than +1, prefer world_prev when it matches current-1.
        if let Some((pk, pe)) = world_prev {
            if pe + 1 == new_epoch {
                self.prev_key = Some(pk);
                self.prev_epoch = Some(pe);
                self.prev_expires_at = Some(Instant::now() + grace);
            }
        }
        true
    }

    fn maybe_seed_world_prev(
        &mut self,
        world_prev: Option<([u8; ROOM_KEY_LEN], u64)>,
        grace: Duration,
    ) {
        let Some((pk, pe)) = world_prev else {
            return;
        };
        if self.prev_key.is_some() {
            return;
        }
        if pe + 1 != self.current_epoch {
            return;
        }
        self.prev_key = Some(pk);
        self.prev_epoch = Some(pe);
        self.prev_expires_at = Some(Instant::now() + grace);
        log::info!(
            "room AEAD seeded prev epoch={pe} from Token RoomKeyGrace (current={})",
            self.current_epoch
        );
    }

    pub fn open(
        &self,
        room_id: &str,
        kind: AeadKind,
        envelope: &[u8],
    ) -> Result<Vec<u8>, WireAeadError> {
        let (cur_key, cur_epoch, prev) = self.open_material(Instant::now());
        wire_aead::open_current_or_prev(
            &cur_key,
            cur_epoch,
            prev.as_ref().map(|(k, e)| (k.as_slice(), *e)),
            room_id,
            kind,
            envelope,
        )
    }
}

pub type SharedRoomAead = Arc<RwLock<RoomAeadKeys>>;

pub fn shared_room_aead(key: [u8; ROOM_KEY_LEN], epoch: u64) -> SharedRoomAead {
    Arc::new(RwLock::new(RoomAeadKeys::new(key, epoch)))
}

/// Seal under a short read lock (copy key material out first).
pub fn seal_shared(
    keys: &SharedRoomAead,
    room_id: &str,
    kind: AeadKind,
    plaintext: &[u8],
) -> Result<Vec<u8>, WireAeadError> {
    let (key, epoch) = keys
        .read()
        .map_err(|_| WireAeadError::CryptographicFailure)?
        .seal_material();
    wire_aead::seal(&key, room_id, epoch, kind, plaintext)
}

/// Open with current→prev grace under a **read** lock (no write contention with seal).
pub fn open_shared(
    keys: &SharedRoomAead,
    room_id: &str,
    kind: AeadKind,
    envelope: &[u8],
) -> Result<Vec<u8>, WireAeadError> {
    let out = {
        let guard = keys
            .read()
            .map_err(|_| WireAeadError::CryptographicFailure)?;
        guard.open(room_id, kind, envelope)
    };
    // Best-effort GC of expired prev (ignore contended / poisoned write).
    if let Ok(mut g) = keys.try_write() {
        g.gc_expired_prev(Instant::now());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotate_keeps_prev_openable() {
        let mut keys = RoomAeadKeys::new([1u8; ROOM_KEY_LEN], 0);
        let env = wire_aead::seal(&[1u8; ROOM_KEY_LEN], "r", 0, AeadKind::Input, b"hi").unwrap();
        assert!(keys.apply_token_key([2u8; ROOM_KEY_LEN], 1, Duration::from_secs(60), None));
        assert_eq!(keys.current_epoch(), 1);
        let pt = keys.open("r", AeadKind::Input, &env).unwrap();
        assert_eq!(pt, b"hi");
        let env2 = wire_aead::seal(&[2u8; ROOM_KEY_LEN], "r", 1, AeadKind::Input, b"ok").unwrap();
        assert_eq!(keys.open("r", AeadKind::Input, &env2).unwrap(), b"ok");
    }

    #[test]
    fn expired_prev_rejected() {
        let mut keys = RoomAeadKeys::new([1u8; ROOM_KEY_LEN], 0);
        let env = wire_aead::seal(&[1u8; ROOM_KEY_LEN], "r", 0, AeadKind::Input, b"hi").unwrap();
        assert!(keys.apply_token_key([2u8; ROOM_KEY_LEN], 1, Duration::from_millis(1), None));
        std::thread::sleep(Duration::from_millis(5));
        assert!(keys.open("r", AeadKind::Input, &env).is_err());
    }

    #[test]
    fn prev_accepted_at_exact_expiry_instant() {
        // Inclusive end (`now <= exp`) matches world RoomKeyGrace.
        let mut keys = RoomAeadKeys::new([1u8; ROOM_KEY_LEN], 0);
        let env = wire_aead::seal(&[1u8; ROOM_KEY_LEN], "r", 0, AeadKind::Input, b"hi").unwrap();
        assert!(keys.apply_token_key([2u8; ROOM_KEY_LEN], 1, Duration::from_secs(60), None));
        let exp = keys.prev_expires_at.expect("prev grace set");
        let (cur_key, cur_epoch, prev) = keys.open_material(exp);
        assert_eq!(cur_epoch, 1);
        assert!(prev.is_some());
        let pt = wire_aead::open_current_or_prev(
            &cur_key,
            cur_epoch,
            prev.as_ref().map(|(k, e)| (k.as_slice(), *e)),
            "r",
            AeadKind::Input,
            &env,
        )
        .unwrap();
        assert_eq!(pt, b"hi");
    }

    #[test]
    fn world_prev_seeds_on_same_epoch_bootstrap() {
        let mut keys = RoomAeadKeys::new([2u8; ROOM_KEY_LEN], 1);
        let env = wire_aead::seal(&[1u8; ROOM_KEY_LEN], "r", 0, AeadKind::Input, b"old").unwrap();
        assert!(!keys.apply_token_key(
            [2u8; ROOM_KEY_LEN],
            1,
            Duration::from_secs(60),
            Some(([1u8; ROOM_KEY_LEN], 0)),
        ));
        assert_eq!(keys.open("r", AeadKind::Input, &env).unwrap(), b"old");
    }
}
