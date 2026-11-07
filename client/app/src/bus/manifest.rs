//! SceneManifest subscriber (x0026 seal + x0025 attestation, x0027 classification).
//!
//! Guarantees:
//! - Manifest `world_seal` + `host_attestation` verified via world JWKS before adopting `host_pub`
//! - `(host_generation, revision)` monotonic gate; regression → discard (x003a)
//! - Verify / decode / limits / AEAD failure → discard and keep joined; leave only on explicit
//!   Leave (x0038)
//! - PROTOCOL_PIN mismatch rejects apply (x0023 / x0038 discard)
//! - Entries drive mesh blake3 / Grab flags / box half-extents (x0027); mesh blake3 is sent to the
//!   remount channel **before** snapshot GET (x003c / x007d); new hex re-sends, same hex skips;
//!   `last_blake3_hex` advances only after a successful remount `try_send`
//! - Accepted `world_seal` is stored for PlaceRead (x006c)
//!
//! Non-guarantees: assets download / GPU upload (see `assets_fetch` / `SceneGpu::replace_world_mesh`);
//! reliable delivery of Manifest puts; remount channel Full eventually recovers on a later put
//! with a different hex (same hex retries after Full because `last_blake3_hex` was not updated).

use std::sync::{mpsc, Arc};
use std::thread;
use std::time::Instant;

use alchemy_client_trust::{
    process_manifest_plaintext, DiscardReason, HostAttestationVerifier, ManifestSealVerifier,
    TrustOutcome, TrustWatermarks,
};
use alchemy_protocol::wire_aead::AeadKind;
use alchemy_protocol::{scene_entry_quat_xyzw, scene_entry_scale};
use alchemy_shared_sim::{scene_entry_color_rgba, ManifestIndex};
use anyhow::{Context, Result};
use zenoh::bytes::ZBytes;
use zenoh::Wait;

use super::frame::fetch_snapshot;
use super::wire::open_zenoh_payload;
use super::GATE_LOG_INTERVAL;
use crate::scene::{EntityPose, SharedState};

pub(crate) fn spawn_manifest_subscriber(
    session: zenoh::Session,
    key: String,
    shared: Arc<SharedState>,
    // Ok(hex) = mesh blake3; Err(reason) = sealed Manifest applied but no mesh blake3.
    blake3_tx: mpsc::SyncSender<Result<String, String>>,
    seal: Arc<ManifestSealVerifier>,
    attest: Arc<HostAttestationVerifier>,
) -> Result<()> {
    let subscriber = session
        .declare_subscriber(key.clone())
        .wait()
        .map_err(|e| anyhow::anyhow!("declare_subscriber manifest failed: {e}"))?;

    thread::Builder::new()
        .name("zenoh-manifest".into())
        .spawn(move || {
            log::info!("subscribed to {key}");
            // Last blake3 hex pushed to remount worker; re-send only when hex changes (x007d).
            let mut last_blake3_hex: Option<String> = None;
            // `true` only after a successful snapshot apply for the current `host_pub`.
            let mut snapshot_ok = false;
            // x0038: drop counters (host input-gate style); never leave on verify fail.
            let mut dropped_identity: u64 = 0;
            let mut dropped_seal: u64 = 0;
            let mut dropped_attest: u64 = 0;
            let mut dropped_gen_rev: u64 = 0;
            let mut dropped_decode: u64 = 0;
            let mut dropped_limits: u64 = 0;
            let mut dropped_aead: u64 = 0;
            let mut last_gate_log = Instant::now();
            while let Ok(sample) = subscriber.recv() {
                // x003b: early drops must still reach the interval aggregate log below.
                'sample: {
                let bytes: ZBytes = sample.payload().clone();
                let payload = match open_zenoh_payload(&shared, AeadKind::Manifest, &bytes) {
                    Ok(pt) => pt,
                    Err(err) => {
                        dropped_aead += 1;
                        log::warn!("drop SceneManifest: AEAD: {err}");
                        break 'sample;
                    }
                };
                // Hold host_pub + gen-rev locks across gate+apply (x003a / x0070).
                let (generation_changed, manifest, accepted_seal) =
                    match (shared.host_pub.lock(), shared.manifest_gen_rev.lock()) {
                        (Ok(mut host_slot), Ok(mut gen_slot)) => {
                            // Gate only needs host_pub + gen-rev; seal is written on Accept.
                            let mut watermarks = TrustWatermarks {
                                host_pub: host_slot.clone(),
                                manifest_gen_rev: *gen_slot,
                                last_world_seal: None,
                            };
                            let outcome = process_manifest_plaintext(
                                payload.as_ref(),
                                shared.get_room_id().as_ref(),
                                &mut watermarks,
                                seal.as_ref(),
                                attest.as_ref(),
                            );
                            match outcome {
                                TrustOutcome::Discard { reason, detail } => {
                                    match reason {
                                        // AEAD is opened before this gate; Aead here is a bug.
                                        DiscardReason::Aead => {
                                            log::error!(
                                                "bug: AEAD discard after open_zenoh_payload: {detail}"
                                            );
                                            dropped_aead += 1;
                                        }
                                        DiscardReason::Limits => dropped_limits += 1,
                                        DiscardReason::Decode => dropped_decode += 1,
                                        DiscardReason::Identity => dropped_identity += 1,
                                        DiscardReason::Seal => dropped_seal += 1,
                                        DiscardReason::Attest => dropped_attest += 1,
                                        DiscardReason::GenRev => dropped_gen_rev += 1,
                                    }
                                    log::warn!("drop SceneManifest: {reason:?}: {detail}");
                                    break 'sample;
                                }
                                TrustOutcome::Accept {
                                    generation_changed,
                                    manifest,
                                } => {
                                    *host_slot = watermarks.host_pub;
                                    *gen_slot = watermarks.manifest_gen_rev;
                                    // x007a: always log Accept; room_jti=missing if token not ready yet.
                                    let room_jti = shared
                                        .auth
                                        .lock()
                                        .ok()
                                        .and_then(|g| g.room_token.clone())
                                        .and_then(|tok| {
                                            crate::jwt_sub::sub_jti_unverified(&tok)
                                                .ok()
                                                .map(|(_, j)| j)
                                        });
                                    let jti_label =
                                        room_jti.as_deref().unwrap_or("missing");
                                    match *gen_slot {
                                        Some(gr) => log::info!(
                                            "manifest accept: room_jti={jti_label} gen={} rev={}",
                                            gr.host_generation,
                                            gr.revision
                                        ),
                                        None => log::info!(
                                            "manifest accept: room_jti={jti_label}"
                                        ),
                                    }
                                    (generation_changed, manifest, watermarks.last_world_seal)
                                }
                            }
                        }
                        _ => {
                            log::error!(
                                "host_pub/manifest_gen_rev lock poisoned; skip Manifest apply"
                            );
                            break 'sample;
                        }
                    };
                if generation_changed {
                    log::info!(
                        "SceneManifest host_generation advanced to {}; reset world + renew Intent session",
                        manifest.host_generation
                    );
                    shared.reset_for_host_generation();
                    snapshot_ok = false;
                    last_blake3_hex = None;
                }
                // Accepted Manifest only — PlaceRead needs this seal (x006c).
                // After generation reset (clears seal), re-apply the accepted seal.
                if let Ok(mut slot) = shared.last_world_seal.lock() {
                    *slot = accepted_seal;
                }
                // Classification: ManifestIndex (shared-sim / x0030; priority = x0027).
                // x003c: send mesh blake3 BEFORE snapshot GET — boot wait must not
                // depend on snapshot latency (auth path bails on Manifest timeout).
                let index = ManifestIndex::from_entries(&manifest.entries);
                let mesh_id = index.mesh_entity_id.unwrap_or(0);
                let mut mesh_blake3: Option<String> = None;
                if let Some(mid) = index.mesh_entity_id {
                    if let Some(entry) = manifest.entries.iter().find(|e| e.entity_id == mid)
                    {
                        if entry.half_extent > 0.0 {
                            log::warn!(
                                "scene entry {} has blake3 + half_extent; mesh wins (box ignored)",
                                entry.entity_id
                            );
                        }
                        mesh_blake3 = Some(entry.blake3.clone());
                    }
                }
                // x006e put-back list: 64 ascii-hex blake3 only (CAS key).
                // shared-sim treats any non-empty blake3 as Mesh; short/non-hex
                // hashes may still draw but are intentionally omitted here.
                let scene_meshes: Vec<(u32, String)> = manifest
                    .entries
                    .iter()
                    .filter(|e| {
                        !alchemy_protocol::scene_entry_is_user_root(e.entity_id)
                            && e.blake3.len() == 64
                            && e.blake3.bytes().all(|b| b.is_ascii_hexdigit())
                    })
                    .map(|e| (e.entity_id, e.blake3.clone()))
                    .collect();
                shared.set_scene_meshes(scene_meshes);
                if let Ok(mut world) = shared.world.lock() {
                    for entry in &manifest.entries {
                        let (r, g, b, a) = scene_entry_color_rgba(entry);
                        match world.entities.entry(entry.entity_id) {
                            std::collections::hash_map::Entry::Vacant(v) => {
                                let [qx, qy, qz, qw] = scene_entry_quat_xyzw(entry);
                                let [sx, sy, sz] = scene_entry_scale(entry);
                                v.insert(EntityPose {
                                    x: entry.x as f32,
                                    y: entry.y as f32,
                                    z: entry.z as f32,
                                    qx: qx as f32,
                                    qy: qy as f32,
                                    qz: qz as f32,
                                    qw: qw as f32,
                                    sx: sx as f32,
                                    sy: sy as f32,
                                    sz: sz as f32,
                                    r,
                                    g,
                                    b,
                                    a,
                                });
                            }
                            std::collections::hash_map::Entry::Occupied(mut o) => {
                                // Frame owns pose (quat/scale); Manifest revises tint only
                                // — no TRS reseed on revision (x002c).
                                if entry.a > 0.0 {
                                    let p = o.get_mut();
                                    p.r = r;
                                    p.g = g;
                                    p.b = b;
                                    p.a = a;
                                }
                            }
                        }
                    }
                }
                shared.apply_manifest_defs(index.grabbable, index.half_extents, mesh_id);
                let msg = match &mesh_blake3 {
                    Some(hex) => Ok(hex.clone()),
                    None => Err("SceneManifest has no mesh entity blake3".to_string()),
                };
                let should_send = match (&msg, &last_blake3_hex) {
                    (Ok(hex), Some(prev)) if prev == hex => false,
                    (Ok(_), _) => true,
                    (Err(_), None) => true, // First Manifest without mesh — notify remount once.
                    (Err(_), Some(_)) => false,
                };
                if should_send {
                    // Update last_blake3_hex only after try_send succeeds so Full/Disconnected
                    // does not permanently suppress remount for this hex (x007d).
                    let mark = match &msg {
                        Ok(hex) => Some(hex.clone()),
                        Err(_) => Some(String::new()),
                    };
                    match blake3_tx.try_send(msg) {
                        Ok(()) => last_blake3_hex = mark,
                        Err(err) => {
                            log::warn!("Manifest mesh blake3 remount channel full/closed: {err}")
                        }
                    }
                }
                if !snapshot_ok {
                    if let Some(cap) = shared.wire_cap() {
                        snapshot_ok = fetch_snapshot(
                            &session,
                            shared.get_room_id().as_ref(),
                            &cap,
                            &shared,
                        );
                    }
                }
                log::info!(
                    "SceneManifest generation={} revision={} entries={} pin_ok attested snapshot_ok={snapshot_ok}",
                    manifest.host_generation,
                    manifest.revision,
                    manifest.entries.len()
                );
                } // 'sample
                if last_gate_log.elapsed() >= GATE_LOG_INTERVAL {
                    if dropped_identity
                        + dropped_seal
                        + dropped_attest
                        + dropped_gen_rev
                        + dropped_decode
                        + dropped_limits
                        + dropped_aead
                        > 0
                    {
                        log::info!(
                            "manifest gate: dropped_identity={dropped_identity} dropped_seal={dropped_seal} dropped_attest={dropped_attest} dropped_gen_rev={dropped_gen_rev} dropped_decode={dropped_decode} dropped_limits={dropped_limits} dropped_aead={dropped_aead} (last ~5s)"
                        );
                    }
                    dropped_identity = 0;
                    dropped_seal = 0;
                    dropped_attest = 0;
                    dropped_gen_rev = 0;
                    dropped_decode = 0;
                    dropped_limits = 0;
                    dropped_aead = 0;
                    last_gate_log = Instant::now();
                }
            }
        })
        .context("spawn manifest thread")?;
    Ok(())
}
