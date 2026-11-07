//! Credential worker thread + physics-facing publish snapshot (x003d).
//!
//! Guarantees: credential / seal HTTP runs on a dedicated thread; failure keeps the last good
//! snapshot and retries with backoff; publish stops after hard expiry (process stays up);
//! host Room Token key sync ≈15s advances only after a successful refresh (failure re-arms
//! with backoff, capped by the sync interval).
//! Non-guarantees: Zenoh put path (x0042), auth UI.

use super::credential_chain::{
    plan_credential_refresh, CredentialRefreshPlan, HostCredentialChain,
};
use super::room_scene::RoomScene;
use super::seal_cache::ManifestSealCache;
use super::timing::{earliest_hard_expire, publish_allowed_at};
use super::verify::RoomTokenVerifier;
use alchemy_protocol::wire_crypto::manifest_seal_blake3_hex;
use alchemy_protocol::{
    LocomotionConfig, LocomotionMode, Message, SceneManifest, DEFAULT_MOVE_SPEED, PROTOCOL_PIN,
};
use anyhow::{anyhow, Context, Result};
use std::sync::atomic::Ordering;
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::{Duration, Instant};

/// Credential worker idle poll when nothing is soft-due.
const CREDENTIAL_WORKER_IDLE: Duration = Duration::from_millis(200);
const CREDENTIAL_BACKOFF_START: Duration = Duration::from_secs(1);
const CREDENTIAL_BACKOFF_MAX: Duration = Duration::from_secs(30);
/// x007e: re-fetch host Token often enough that world rotate is adopted before
/// guests Rejoin on the new epoch (soft TTL alone can leave a minutes-long window).
const ROOM_KEY_SYNC_INTERVAL: Duration = Duration::from_secs(15);

/// Physics-facing credential / sealed-Manifest surface (x003d).
#[derive(Debug, Clone)]
pub struct HostPublishSnapshot {
    pub publish_allowed: bool,
    pub hard_expire_at: Option<Instant>,
    pub host_generation: u64,
    pub manifest_bytes: Vec<u8>,
    pub manifest_revision: u64,
    /// Bumped when the worker rebuilds a sealed Manifest (attest / seal rotate).
    pub manifest_epoch: u64,
}

impl HostPublishSnapshot {
    pub fn from_sealed(
        manifest: &SceneManifest,
        hard_expire_at: Option<Instant>,
        manifest_epoch: u64,
    ) -> Self {
        Self {
            publish_allowed: true,
            hard_expire_at,
            host_generation: manifest.host_generation,
            manifest_bytes: manifest.encode_to_vec(),
            manifest_revision: manifest.revision,
            manifest_epoch,
        }
    }
}

/// Inputs owned by the credential worker after bootstrap (x003d).
///
/// `scene` is shared with the physics thread so x006d take-out can append entries
/// and force a ManifestSeal rebuild (`force_reseal`).
pub struct CredentialWorkerConfig {
    pub auth_url: String,
    pub world_url: String,
    pub room_id: String,
    pub host_pub: Vec<u8>,
    pub scene: Arc<RwLock<RoomScene>>,
    pub force_reseal: Arc<std::sync::atomic::AtomicBool>,
    /// Shared with AEAD seal/open paths; updated when host Token epoch advances (x007e).
    pub room_aead: crate::room_aead::SharedRoomAead,
}

fn combined_hard_expire(
    creds: &HostCredentialChain,
    seal_cache: &ManifestSealCache,
) -> Option<Instant> {
    earliest_hard_expire(&[
        creds.auth_hard_expire_at,
        Some(creds.host_hard_expire_at),
        Some(creds.attest_hard_expire_at),
        if seal_cache.seal.is_empty() {
            None
        } else {
            Some(seal_cache.hard_expire_at)
        },
    ])
}

/// Apply a successful sealed Manifest into the shared snapshot (testable helper).
pub fn write_snapshot_on_success(
    snapshot: &mut HostPublishSnapshot,
    manifest: &SceneManifest,
    hard_expire_at: Option<Instant>,
) {
    let epoch = snapshot.manifest_epoch.wrapping_add(1);
    *snapshot = HostPublishSnapshot::from_sealed(manifest, hard_expire_at, epoch);
}

pub fn build_manifest_from_scene(
    room_id: &str,
    scene: &RoomScene,
    host_pub: Vec<u8>,
    host_attestation: String,
    world_seal: String,
    host_generation: u64,
) -> SceneManifest {
    let loco = scene.locomotion.unwrap_or(LocomotionConfig {
        mode: LocomotionMode::Noclip.into(),
        move_speed: DEFAULT_MOVE_SPEED,
    });
    SceneManifest {
        room_id: room_id.to_string(),
        revision: scene.revision,
        entries: scene.entries.clone(),
        locomotion: Some(loco),
        protocol_pin: PROTOCOL_PIN.to_string(),
        host_pub,
        host_attestation,
        world_seal,
        host_generation,
    }
}

/// Build Manifest body (empty seal), obtain/cache world seal, return publishable Manifest.
pub fn ensure_sealed_manifest(
    room_id: &str,
    scene: &RoomScene,
    host_pub: &[u8],
    creds: &HostCredentialChain,
    world_url: &str,
    seal_cache: &mut ManifestSealCache,
) -> Result<SceneManifest> {
    let mut manifest = build_manifest_from_scene(
        room_id,
        scene,
        host_pub.to_vec(),
        creds.host_attestation.clone(),
        String::new(),
        creds.host_generation,
    );
    let body_hex = manifest_seal_blake3_hex(&manifest);
    seal_cache
        .ensure(
            world_url,
            room_id,
            &creds.host_token,
            manifest.revision,
            &body_hex,
            &creds.host_attestation,
        )
        .context("ensure ManifestSeal")?;
    manifest.world_seal = seal_cache.seal.clone();
    Ok(manifest)
}

/// Mark seal soft-due so the next worker loop retries even if credential soft
/// deadlines already advanced (attest succeeded, seal HTTP failed).
pub fn force_seal_retry(seal_cache: &mut ManifestSealCache, pending_seal: &mut bool) {
    *pending_seal = true;
    seal_cache.refresh_at = Instant::now();
}

/// Spawn background auth → Room Token → attestation → seal refresh (x003d).
pub fn spawn_credential_worker(
    snapshot: Arc<RwLock<HostPublishSnapshot>>,
    mut creds: HostCredentialChain,
    mut seal_cache: ManifestSealCache,
    verifier: Arc<RoomTokenVerifier>,
    cfg: CredentialWorkerConfig,
) {
    thread::Builder::new()
        .name("credential-refresh".into())
        .spawn(move || {
            let mut backoff = CREDENTIAL_BACKOFF_START;
            let mut pending_seal = false;
            let mut next_room_key_sync = Instant::now() + ROOM_KEY_SYNC_INTERVAL;
            loop {
                let now = Instant::now();
                {
                    let Ok(mut snap) = snapshot.write() else {
                        log::error!("credential snapshot RwLock poisoned; stopping worker");
                        return;
                    };
                    if snap.publish_allowed
                        && !publish_allowed_at(now, snap.hard_expire_at, true)
                    {
                        log::error!(
                            "credential hard expiry reached; stopping Manifest/Frame/snapshot publish (host process stays up)"
                        );
                        snap.publish_allowed = false;
                    }
                }

                let mut plan = plan_credential_refresh(
                    now,
                    creds.auth_refresh_at,
                    creds.host_refresh_at,
                    creds.attest_refresh_at,
                );
                // Advance `next_room_key_sync` only after a successful refresh (below);
                // on failure, backoff + short re-arm keep the ≈15s guarantee closer.
                let key_sync_due = now >= next_room_key_sync;
                if key_sync_due {
                    plan = Some(match plan {
                        Some(mut p) => {
                            p.refresh_host = true;
                            p
                        }
                        None => CredentialRefreshPlan {
                            refresh_auth: false,
                            refresh_host: true,
                            refresh_attest: false,
                        },
                    });
                }
                let external_reseal = cfg
                    .force_reseal
                    .swap(false, Ordering::SeqCst);
                let seal_due = pending_seal || external_reseal || now >= seal_cache.refresh_at;
                if plan.is_none() && !seal_due {
                    thread::sleep(CREDENTIAL_WORKER_IDLE);
                    continue;
                }

                let refresh_result = (|| -> Result<bool> {
                    let mut attest_rotated = false;
                    let mut room_aead_advanced = false;
                    if let Some(plan) = plan {
                        let refreshed_host = plan.refresh_host;
                        attest_rotated = creds.apply_credential_plan(
                            plan,
                            &cfg.auth_url,
                            &cfg.world_url,
                            &cfg.room_id,
                            &cfg.host_pub,
                            verifier.as_ref(),
                        )?;
                        if refreshed_host {
                            // Grace ≈ Token TTL (hard expiry delta); fall back to 300s.
                            let grace = creds
                                .host_hard_expire_at
                                .checked_duration_since(Instant::now())
                                .unwrap_or(crate::room_aead::DEFAULT_PREV_GRACE);
                            // Poison must fail the refresh so `next_room_key_sync` does not
                            // advance as if the new epoch were applied.
                            let mut keys = cfg
                                .room_aead
                                .write()
                                .map_err(|_| anyhow!("room_aead RwLock poisoned"))?;
                            room_aead_advanced = keys.apply_token_key(
                                creds.room_key,
                                creds.room_key_epoch,
                                grace,
                                creds.room_key_prev,
                            );
                        }
                    }
                    if attest_rotated || seal_due || seal_cache.seal.is_empty() {
                        let scene = cfg
                            .scene
                            .read()
                            .map_err(|_| anyhow!("scene RwLock poisoned"))?
                            .clone();
                        let manifest = ensure_sealed_manifest(
                            &cfg.room_id,
                            &scene,
                            &cfg.host_pub,
                            &creds,
                            &cfg.world_url,
                            &mut seal_cache,
                        )
                        .inspect_err(|_| {
                            force_seal_retry(&mut seal_cache, &mut pending_seal);
                            // Retry take-out reseal after transient seal failure.
                            if external_reseal {
                                cfg.force_reseal.store(true, Ordering::SeqCst);
                            }
                        })?;
                        pending_seal = false;
                        let hard = combined_hard_expire(&creds, &seal_cache);
                        let Ok(mut snap) = snapshot.write() else {
                            return Err(anyhow!("credential snapshot RwLock poisoned"));
                        };
                        write_snapshot_on_success(&mut snap, &manifest, hard);
                        if attest_rotated {
                            log::info!(
                                "credential worker: sealed Manifest ready (HostAttestation rotated, epoch={})",
                                snap.manifest_epoch
                            );
                        } else {
                            log::info!(
                                "credential worker: sealed Manifest ready (ManifestSeal refresh, epoch={})",
                                snap.manifest_epoch
                            );
                        }
                    } else {
                        // Auth/host soft refresh without attest: extend hard expiry only.
                        let hard = combined_hard_expire(&creds, &seal_cache);
                        let Ok(mut snap) = snapshot.write() else {
                            return Err(anyhow!("credential snapshot RwLock poisoned"));
                        };
                        snap.hard_expire_at = hard;
                        snap.publish_allowed = true;
                        snap.host_generation = creds.host_generation;
                        if room_aead_advanced {
                            // Force physics to re-put Manifest under the new AEAD epoch
                            // without waiting for MANIFEST_REPUBLISH (x007e).
                            snap.manifest_epoch = snap.manifest_epoch.saturating_add(1);
                            log::info!(
                                "credential worker: room_key epoch advanced; bump manifest_epoch={} for AEAD re-put",
                                snap.manifest_epoch
                            );
                        }
                    }
                    if room_aead_advanced
                        && (attest_rotated || seal_due || seal_cache.seal.is_empty())
                    {
                        // Seal path already bumped epoch via write_snapshot_on_success;
                        // still log for operators.
                        log::info!("credential worker: room_key epoch advanced with seal rebuild");
                    }
                    Ok(attest_rotated)
                })();

                match refresh_result {
                    Ok(_) => {
                        backoff = CREDENTIAL_BACKOFF_START;
                        if key_sync_due {
                            next_room_key_sync = Instant::now() + ROOM_KEY_SYNC_INTERVAL;
                        }
                        thread::sleep(CREDENTIAL_WORKER_IDLE);
                    }
                    Err(err) => {
                        log::warn!(
                            "credential refresh failed (retry with backoff; keeping last good snapshot): {err:#}"
                        );
                        if key_sync_due {
                            // Fail-closed soft retry: do not wait a full 15s after a sync miss.
                            next_room_key_sync = Instant::now() + backoff.min(ROOM_KEY_SYNC_INTERVAL);
                        }
                        thread::sleep(backoff);
                        backoff = backoff.saturating_mul(2).min(CREDENTIAL_BACKOFF_MAX);
                    }
                }
            }
        })
        .expect("spawn credential-refresh");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_snapshot_on_success_bumps_epoch_and_enables_publish() {
        let mut snap = HostPublishSnapshot {
            publish_allowed: false,
            hard_expire_at: Some(Instant::now() - Duration::from_secs(1)),
            host_generation: 1,
            manifest_bytes: vec![],
            manifest_revision: 0,
            manifest_epoch: 3,
        };
        let manifest = SceneManifest {
            room_id: "local".into(),
            revision: 7,
            entries: vec![],
            locomotion: None,
            protocol_pin: PROTOCOL_PIN.to_string(),
            host_pub: vec![0; 32],
            host_attestation: "attest".into(),
            world_seal: "seal".into(),
            host_generation: 2,
        };
        let hard = Instant::now() + Duration::from_secs(30);
        write_snapshot_on_success(&mut snap, &manifest, Some(hard));
        assert!(snap.publish_allowed);
        assert_eq!(snap.manifest_epoch, 4);
        assert_eq!(snap.manifest_revision, 7);
        assert_eq!(snap.host_generation, 2);
        assert!(!snap.manifest_bytes.is_empty());
    }

    #[test]
    fn force_seal_retry_marks_pending_and_soft_due() {
        let mut seal = ManifestSealCache::empty();
        seal.refresh_at = Instant::now() + Duration::from_secs(3600);
        let mut pending = false;
        force_seal_retry(&mut seal, &mut pending);
        assert!(pending);
        assert!(seal.refresh_at <= Instant::now());
    }

    #[test]
    fn seal_failure_path_does_not_write_snapshot() {
        // Contract: on ensure_sealed_manifest Err, worker calls force_seal_retry and
        // must not call write_snapshot_on_success (snapshot stays at previous epoch).
        let before = HostPublishSnapshot {
            publish_allowed: true,
            hard_expire_at: Some(Instant::now() + Duration::from_secs(10)),
            host_generation: 9,
            manifest_bytes: vec![1, 2, 3],
            manifest_revision: 1,
            manifest_epoch: 5,
        };
        let mut snap = before.clone();
        let mut pending = false;
        let mut seal = ManifestSealCache::empty();
        seal.refresh_at = Instant::now() + Duration::from_secs(60);
        // Simulate Err branch only:
        force_seal_retry(&mut seal, &mut pending);
        assert_eq!(snap.manifest_epoch, before.manifest_epoch);
        assert_eq!(snap.manifest_bytes, before.manifest_bytes);
        assert!(pending);
        // Success path would bump epoch:
        let manifest = SceneManifest {
            room_id: "local".into(),
            revision: 1,
            entries: vec![],
            locomotion: None,
            protocol_pin: PROTOCOL_PIN.to_string(),
            host_pub: vec![0; 32],
            host_attestation: "a".into(),
            world_seal: "s".into(),
            host_generation: 9,
        };
        write_snapshot_on_success(
            &mut snap,
            &manifest,
            Some(Instant::now() + Duration::from_secs(30)),
        );
        assert_eq!(snap.manifest_epoch, 6);
    }
}
