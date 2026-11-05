//! Room Token verification against world-server JWKS + auth-gated token fetch.
//!
//! Guarantees: RS256 verify + iss/aud/exp/room_id checks; Room Token issued only with auth JWT.
//! x0021: verify never does HTTP on the input path (SWR + background refresh).
//! x0040: JWKS soft TTL (SWR) + **hard stale TTL** — beyond hard, bind verify fails closed
//! until background refresh succeeds (revoked world keys cannot be used forever).
//! Long-running host: `HostCredentialChain` (auth → Room Token → HostAttestation) plus
//! `ManifestSealCache` (body blake3 → world seal).
//! x0039: seal request includes `host_attestation`; world rejects non-canonical blake3.
//! x003d: credential / seal HTTP runs on a dedicated worker; physics reads
//! [`HostPublishSnapshot`] only. Soft-deadline refresh failure retries with backoff and
//! keeps the last good snapshot. **Fail-closed** means stop publishing after hard expiry
//! (full TTL), not kill the process on a transient 502.
//! Non-guarantees: auth UI, session MAC (P2); Zenoh put path is x0042 network worker.
//! client `world_jwks` soft/hard + unknown-kid cooldown: x0044 (parity with this module).
//! Fuzz: `host-server/fuzz` `room_token_verify` (x004c; feature `fuzzing`).
//!
//! Module layout (x0076, mechanical split of the former single `room_token.rs`):
//! `verify` (claims + verifier type), `jwks` (cache / refresh), `timing` (TTL math),
//! `http` (shared agent), `credential_chain` (auth -> host token -> attestation; still the
//! largest unit — further split is a follow-up), `seal_cache` (ManifestSeal),
//! `room_scene` (scene fetch/append/delete), `credential_worker`.
//!
//! [`RoomTokenVerifier`]: type + claim checks live in `verify`; JWKS cache fields are
//! `pub(super)` so `jwks` owns fetch / SWR / hard-stale / background kick impls.

mod credential_chain;
mod credential_worker;
mod http;
mod jwks;
mod room_scene;
mod seal_cache;
mod timing;
mod verify;

pub use credential_chain::{
    default_auth_url, fetch_auth_session, fetch_host_attestation, fetch_host_token,
    plan_credential_refresh, refresh_auth_session, CredentialRefreshPlan, HostAuthSession,
    HostCredentialChain, HostTokenBundle, WorldMtls,
};
pub use credential_worker::{
    build_manifest_from_scene, ensure_sealed_manifest, force_seal_retry, spawn_credential_worker,
    write_snapshot_on_success, CredentialWorkerConfig, HostPublishSnapshot,
};
pub use room_scene::{
    append_room_scene_entry, delete_room_scene_entry, fetch_room_scene,
    install_shared_scene_if_newer, RoomScene,
};
pub use seal_cache::{fetch_manifest_seal, ManifestSealCache};
pub use timing::{
    earliest_hard_expire, hard_expire_deadline, publish_allowed_at, refresh_deadline,
    TOKEN_REFRESH_FRACTION,
};
pub use verify::{RoomClaims, RoomCnf, RoomTokenVerifier};
