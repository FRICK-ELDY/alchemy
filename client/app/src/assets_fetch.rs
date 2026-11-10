//! Assets HTTP / blob / mesh helpers.
//!
//! **Wired:**
//! - Boot / MainHub: empty shell via [`crate::mesh::empty_shell_mesh`] (not this module)
//! - Join remount: [`load_auth_mesh`] + [`try_place_read_for_mesh`] from `room` mesh-remount worker (x007d)
//! - Unauthenticated Coin fixture: [`load_unauth_home_mesh`] (debug / seed tooling; not MainHub boot)
//!
//! **Helper contract when called** ([`load_auth_mesh`]):
//! - Assets GET (Grant or PlaceRead, x006c): connect/read timeout; failure → error placeholder
//!   + dash status (not Coin fixture)
//! - Fixture fallback only with debug build + `ALCHEMY_ALLOW_FIXTURE_FALLBACK` (x0048 / P3-3)
//! - Verifies blake3 before GLB decode (mismatch → placeholder / debug fixture)
//!
//! Non-guarantees: cancel mid-stream beyond HTTP timeout; Manifest subscription
//! (see `bus::manifest`); retry/backoff on download failure.

use std::io::Read;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};

use crate::asset_decode_budget::{AssetDecodeBudget, MAX_INPUT_BYTES};
use crate::mesh::{
    blake3_hex, load_mesh_from_glb, placeholder_error_mesh, verify_blake3, CpuMesh, COIN_BLAKE3_HEX,
};
use crate::place_read;
use crate::scene::SharedState;

/// Assets blob GET: connect bound (hung TCP / dead assets).
const ASSETS_HTTP_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// Assets blob GET: read bound (cancels stalled body / R2 transfer — x0048).
const ASSETS_HTTP_READ_TIMEOUT: Duration = Duration::from_secs(30);

fn assets_http_agent() -> ureq::Agent {
    // Follow 302 → R2 presigned GET (x001d). ureq strips Authorization on cross-host redirect.
    // `timeout_read` is per-read / idle stall (not a wall-clock cap on chunked transfer).
    // Hung connect or stalled body is cancelled; cooperative mid-stream cancel is non-goal.
    ureq::AgentBuilder::new()
        .timeout_connect(ASSETS_HTTP_CONNECT_TIMEOUT)
        .timeout_read(ASSETS_HTTP_READ_TIMEOUT)
        .redirects(5)
        .build()
}

/// Short Home-facing line (top-level `Display` only — full chain stays in logs).
fn asset_status_line(kind: &str, err: &anyhow::Error) -> String {
    format!("{kind} — placeholder ({err})")
}

fn try_coin_fixture_mesh() -> Result<CpuMesh> {
    let fixture = load_fixture_bytes()?;
    verify_blake3(&fixture, COIN_BLAKE3_HEX).context("fixture blake3")?;
    load_mesh_from_glb(&fixture).context("decode fixture GLB")
}

fn placeholder_from_err(kind: &str, err: &anyhow::Error) -> AuthMeshLoad {
    let status = asset_status_line(kind, err);
    log::error!("{status}; detail={err:#}");
    AuthMeshLoad::Placeholder {
        mesh: placeholder_error_mesh(),
        status,
    }
}

fn fetch_blob_bytes(
    assets_url: &str,
    blake3: &str,
    auth_token: &str,
    place_read: Option<&str>,
) -> Result<Vec<u8>> {
    let base = assets_url.trim_end_matches('/');
    let url = format!("{base}/api/v1/blobs/{blake3}");
    let authz = format!("Bearer {auth_token}");
    let mut req = assets_http_agent().get(&url).set("authorization", &authz);
    if let Some(token) = place_read.filter(|t| !t.is_empty()) {
        req = req.set("x-alchemy-place-read", token);
    }
    let resp = req.call().context("assets blob download failed")?;
    // x0085: stop reading past decode input budget (OOM / zip-bomb body).
    let mut limited = resp
        .into_reader()
        .take((MAX_INPUT_BYTES as u64).saturating_add(1));
    let mut bytes = Vec::new();
    limited.read_to_end(&mut bytes).context("read blob body")?;
    AssetDecodeBudget::standard()
        .check_input_bytes(bytes.len())
        .context("blob exceeds GLB decode input budget")?;
    Ok(bytes)
}

/// Debug-only Coin fixture escape hatch (x003c / x0048). Release never returns true.
fn fixture_fallback_allowed() -> bool {
    cfg!(debug_assertions) && std::env::var_os("ALCHEMY_ALLOW_FIXTURE_FALLBACK").is_some()
}

fn load_blob_bytes(
    assets_url: &str,
    blake3: &str,
    auth_token: &str,
    place_read: Option<&str>,
) -> Result<Vec<u8>> {
    match fetch_blob_bytes(assets_url, blake3, auth_token, place_read) {
        Ok(bytes) => {
            log::info!(
                "downloaded blob via assets GET ({} bytes, blake3={}; may follow R2 redirect)",
                bytes.len(),
                blake3_hex(&bytes)
            );
            Ok(bytes)
        }
        Err(err) => {
            // x003c / x0048: Grant/PlaceRead path requires assets. Fixture is debug-only escape hatch.
            if fixture_fallback_allowed() {
                log::warn!(
                    "assets download failed ({err:#}); ALCHEMY_ALLOW_FIXTURE_FALLBACK → fixture"
                );
                return load_fixture_bytes();
            }
            Err(err).context(
                "assets blob download required (Grant or PlaceRead; upload demo coin / ensure CAS; debug: ALCHEMY_ALLOW_FIXTURE_FALLBACK=1)",
            )
        }
    }
}

/// Authenticated mesh resolution: assets GET success, debug fixture, or error placeholder.
pub(crate) enum AuthMeshLoad {
    Ready(CpuMesh),
    /// Distinct cube + dash status — never Coin fixture (x0048).
    Placeholder {
        mesh: CpuMesh,
        status: String,
    },
}

fn debug_fixture_or_placeholder(kind: &str, err: &anyhow::Error) -> AuthMeshLoad {
    if fixture_fallback_allowed() {
        log::warn!("{kind} ({err:#}); ALCHEMY_ALLOW_FIXTURE_FALLBACK → fixture");
        match try_coin_fixture_mesh() {
            Ok(mesh) => AuthMeshLoad::Ready(mesh),
            Err(fix_err) => {
                placeholder_from_err(&format!("{kind}; fixture fallback failed"), &fix_err)
            }
        }
    } else {
        placeholder_from_err(kind, err)
    }
}

/// Authenticated Manifest mesh download (x003c / x007d remount).
pub(crate) fn load_auth_mesh(
    assets_url: &str,
    blake3: &str,
    auth_token: &str,
    place_read: Option<&str>,
) -> AuthMeshLoad {
    let primary = match load_blob_bytes(assets_url, blake3, auth_token, place_read) {
        Ok(bytes) => bytes,
        Err(err) => {
            // load_blob_bytes already applied debug fixture escape on download failure.
            return placeholder_from_err("Asset download failed", &err);
        }
    };
    match verify_blake3(&primary, blake3) {
        Ok(()) => match load_mesh_from_glb(&primary) {
            Ok(mesh) => AuthMeshLoad::Ready(mesh),
            Err(err) => debug_fixture_or_placeholder("Asset GLB decode failed", &err),
        },
        Err(err) => debug_fixture_or_placeholder("Asset blake3 mismatch", &err),
    }
}

/// x006c: Room Token + last accepted `world_seal` → PlaceRead for assets GET.
pub(crate) fn try_place_read_for_mesh(shared: &SharedState) -> Option<String> {
    let room_token = shared.auth.lock().ok().and_then(|g| g.room_token.clone())?;
    let world_seal = shared
        .last_world_seal
        .lock()
        .ok()
        .and_then(|g| g.clone())
        .filter(|s| !s.is_empty())?;
    let world_url = shared.get_world_url();
    match place_read::fetch_place_read(
        &world_url,
        shared.get_room_id().as_ref(),
        &room_token,
        &world_seal,
    ) {
        Ok(token) => {
            log::info!("PlaceRead issued for assets mesh GET (x006c)");
            Some(token)
        }
        Err(err) => {
            // Owner may still succeed via BlobGrant alone; guests need PlaceRead.
            log::warn!("PlaceRead issue failed (assets GET may rely on Grant only): {err:#}");
            None
        }
    }
}

fn load_fixture_bytes() -> Result<Vec<u8>> {
    let path = fixture_path()?;
    std::fs::read(&path).with_context(|| format!("read fixture {}", path.display()))
}

fn fixture_path() -> Result<PathBuf> {
    // Used for: (1) ensure-demo-coin seed source beside ZIP, (2) debug fixture fallback.
    // Not the MainHub boot mesh (empty shell) or authenticated remount path.
    const REL: &str = "fixtures/Coin_PolygonalMind.glb";
    if let Ok(p) = std::env::var("ALCHEMY_FIXTURE_GLB") {
        return Ok(PathBuf::from(p));
    }
    if let Ok(mut exe) = std::env::current_exe() {
        exe.pop();
        let beside_exe = exe.join(REL);
        if beside_exe.is_file() {
            return Ok(beside_exe);
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        let beside_cwd = cwd.join(REL);
        if beside_cwd.is_file() {
            return Ok(beside_cwd);
        }
    }
    // Dev tree: client/app → ../fixtures
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("../fixtures/Coin_PolygonalMind.glb");
    Ok(path)
}

/// Coin fixture for seed / debug tooling (not MainHub boot; x007d uses empty shell).
#[cfg(test)]
pub(crate) fn load_unauth_home_mesh() -> AuthMeshLoad {
    match try_coin_fixture_mesh() {
        Ok(mesh) => AuthMeshLoad::Ready(mesh),
        Err(err) => placeholder_from_err("Home fixture unavailable", &err),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_unauth_home_mesh_loads_coin_or_placeholder() {
        // Fixture present in client/fixtures during unit tests.
        let out = load_unauth_home_mesh();
        assert!(matches!(
            out,
            AuthMeshLoad::Ready(_) | AuthMeshLoad::Placeholder { .. }
        ));
    }
}
