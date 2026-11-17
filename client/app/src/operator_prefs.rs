//! Operator / onboarding prefs (system_ui Step1–3 shell).
//!
//! Guarantees: OFFICIAL vs CUSTOM endpoint expansion; disk prefs for operator choice,
//! optional `refresh_vault_key` (OS vault id only), last_login_at (7-day gate);
//! no secrets in logs; prefs JSON never serializes refresh secret (x00cf).
//! Non-guarantees: Federation directory; host-server env sync; vault availability
//! (headless → session-only).
//!
//! Security note (x00cf / P1-6): refresh token body lives in the OS vault via
//! `refresh_vault`. Legacy plaintext `refresh_token` is deserialize-only and stripped
//! on load/migrate. Vault failure → session-only (no plaintext left on disk).

use crate::refresh_vault::{self, RefreshVault};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

/// In-process lock for prefs load/modify/save so soft-refresh cannot resurrect
/// remember-me over a concurrent clear (x00cf).
static PREFS_IO_LOCK: Mutex<()> = Mutex::new(());

/// Bumped whenever remember-me durability is cleared (logout / OFF / scrub / rotate fail).
/// `persist_remember_me` refuses to install a key if this changes across vault `store`.
static REMEMBER_CLEAR_EPOCH: AtomicU64 = AtomicU64::new(0);

fn prefs_io_lock() -> MutexGuard<'static, ()> {
    PREFS_IO_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn bump_remember_clear_epoch() {
    REMEMBER_CLEAR_EPOCH.fetch_add(1, Ordering::SeqCst);
}

/// Aligns with auth-server `refresh_token_inactivity_days: 7`.
pub const SESSION_MAX_AGE_SECS: u64 = 7 * 24 * 60 * 60;

pub const OFFICIAL_AUTH_URL: &str = "https://auth.frick-eldy.com";
pub const OFFICIAL_WORLD_URL: &str = "https://world.frick-eldy.com";
pub const OFFICIAL_ASSETS_URL: &str = "https://assets.frick-eldy.com";
pub const OFFICIAL_ZENOH_CONNECT: &str = "tls/relay.frick-eldy.com:7447";

pub const CUSTOM_DEFAULT_HOST: &str = "127.0.0.1";
pub const CUSTOM_AUTH_PORT: u16 = 4002;
pub const CUSTOM_WORLD_PORT: u16 = 4003;
pub const CUSTOM_ASSETS_PORT: u16 = 4004;
pub const CUSTOM_ZENOH_PORT: u16 = 7447;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperatorKind {
    #[default]
    Official,
    Custom,
    /// Selectable in UI but not connectable yet.
    Federation,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Endpoints {
    pub auth_url: String,
    pub world_url: String,
    pub assets_url: String,
    pub zenoh_connect: String,
}

impl Endpoints {
    pub fn official() -> Self {
        Self {
            auth_url: OFFICIAL_AUTH_URL.into(),
            world_url: OFFICIAL_WORLD_URL.into(),
            assets_url: OFFICIAL_ASSETS_URL.into(),
            zenoh_connect: OFFICIAL_ZENOH_CONNECT.into(),
        }
    }

    pub fn custom_localhost() -> Self {
        expand_custom(CUSTOM_DEFAULT_HOST)
    }
}

/// Expand CUSTOM host or Auth base URL into the four service endpoints.
pub fn expand_custom(input: &str) -> Endpoints {
    let raw = input.trim();
    if raw.is_empty() {
        return Endpoints::custom_localhost();
    }
    if let Some(rest) = raw
        .strip_prefix("https://")
        .or_else(|| raw.strip_prefix("http://"))
    {
        let https = raw.starts_with("https://");
        let host_port = rest.split('/').next().unwrap_or(rest);
        let host = host_port.split(':').next().unwrap_or(host_port);
        let port = host_port.split(':').nth(1);
        // :443 / :80 count as "no explicit port" (same-origin reverse-proxy style).
        let default_port = matches!((https, port), (true, Some("443")) | (false, Some("80")));
        let explicit_non_default_port = port.is_some() && !default_port;
        let zenoh_scheme = if https { "tls" } else { "tcp" };
        if explicit_non_default_port {
            // Explicit non-default port: keep auth URL; sibling ports for world/assets.
            let auth_url = raw.trim_end_matches('/').to_string();
            return Endpoints {
                auth_url,
                world_url: format!(
                    "http{}://{host}:{CUSTOM_WORLD_PORT}",
                    if https { "s" } else { "" }
                ),
                assets_url: format!(
                    "http{}://{host}:{CUSTOM_ASSETS_PORT}",
                    if https { "s" } else { "" }
                ),
                zenoh_connect: format!("{zenoh_scheme}/{host}:{CUSTOM_ZENOH_PORT}"),
            };
        }
        // Host-only (or default 443/80): http keeps local-dev ports; https = same origin.
        // Custom does not invent auth./world./assets. subdomains (Official-only layout).
        if https {
            return Endpoints {
                auth_url: format!("https://{host}"),
                world_url: format!("https://{host}"),
                assets_url: format!("https://{host}"),
                zenoh_connect: format!("{zenoh_scheme}/{host}:{CUSTOM_ZENOH_PORT}"),
            };
        }
        return Endpoints {
            auth_url: format!("http://{host}:{CUSTOM_AUTH_PORT}"),
            world_url: format!("http://{host}:{CUSTOM_WORLD_PORT}"),
            assets_url: format!("http://{host}:{CUSTOM_ASSETS_PORT}"),
            zenoh_connect: format!("{zenoh_scheme}/{host}:{CUSTOM_ZENOH_PORT}"),
        };
    }
    // Bare host or `host:port` (no scheme). Explicit port is kept on auth only;
    // world/assets/zenoh still use CUSTOM_* ports on the same host.
    if let Some((host, port)) = raw.split_once(':') {
        if !port.is_empty() && port.chars().all(|c| c.is_ascii_digit()) {
            return Endpoints {
                auth_url: format!("http://{host}:{port}"),
                world_url: format!("http://{host}:{CUSTOM_WORLD_PORT}"),
                assets_url: format!("http://{host}:{CUSTOM_ASSETS_PORT}"),
                zenoh_connect: format!("tcp/{host}:{CUSTOM_ZENOH_PORT}"),
            };
        }
    }
    let host = raw.split(':').next().unwrap_or(raw);
    Endpoints {
        auth_url: format!("http://{host}:{CUSTOM_AUTH_PORT}"),
        world_url: format!("http://{host}:{CUSTOM_WORLD_PORT}"),
        assets_url: format!("http://{host}:{CUSTOM_ASSETS_PORT}"),
        zenoh_connect: format!("tcp/{host}:{CUSTOM_ZENOH_PORT}"),
    }
}

pub fn endpoints_for(operator: OperatorKind, custom_host: &str) -> Endpoints {
    match operator {
        OperatorKind::Official => Endpoints::official(),
        OperatorKind::Custom => expand_custom(custom_host),
        // Federation is stub — fall back to official constants if somehow applied.
        OperatorKind::Federation => Endpoints::official(),
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct OnboardingPrefs {
    pub operator_chosen: bool,
    pub operator: OperatorKind,
    pub custom_host: String,
    pub auth_url: String,
    pub world_url: String,
    pub assets_url: String,
    pub zenoh_connect: String,
    /// OS vault entry id (`alchemy.refresh` / keyring username). Secret body is not here.
    #[serde(default)]
    pub refresh_vault_key: Option<String>,
    /// Legacy plaintext refresh (pre-x00cf). Deserialize only — never written back.
    #[serde(default, skip_serializing)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub username: Option<String>,
    /// Unix seconds of last successful login/register/refresh restore.
    #[serde(default)]
    pub last_login_at: Option<u64>,
    /// Local host mode (HostSupervisor / Organizing). Distinct from [`OperatorKind`].
    /// Default off — Participation-only until enabled in Settings.
    #[serde(default)]
    pub server_operator: bool,
}

impl Default for OnboardingPrefs {
    fn default() -> Self {
        let ep = Endpoints::custom_localhost();
        Self {
            operator_chosen: false,
            operator: OperatorKind::Official,
            custom_host: CUSTOM_DEFAULT_HOST.into(),
            auth_url: ep.auth_url,
            world_url: ep.world_url,
            assets_url: ep.assets_url,
            zenoh_connect: ep.zenoh_connect,
            refresh_vault_key: None,
            refresh_token: None,
            username: None,
            last_login_at: None,
            server_operator: false,
        }
    }
}

impl OnboardingPrefs {
    pub fn endpoints(&self) -> Endpoints {
        Endpoints {
            auth_url: self.auth_url.clone(),
            world_url: self.world_url.clone(),
            assets_url: self.assets_url.clone(),
            zenoh_connect: self.zenoh_connect.clone(),
        }
    }

    pub fn apply_endpoints(&mut self, ep: &Endpoints) {
        self.auth_url = ep.auth_url.clone();
        self.world_url = ep.world_url.clone();
        self.assets_url = ep.assets_url.clone();
        self.zenoh_connect = ep.zenoh_connect.clone();
    }

    /// Clear remember-me + username fields. Returns the prior vault key (delete **outside**
    /// the prefs IO lock). Does not touch the OS vault itself.
    ///
    /// [`REMEMBER_CLEAR_EPOCH`] advances only when durable remember-me material was
    /// present (vault key or legacy plaintext) so a no-op logout cannot orphan a
    /// concurrent [`persist_remember_me`].
    pub fn clear_session_fields(&mut self) -> Option<String> {
        if self.refresh_vault_key.is_some() || self.refresh_token.is_some() {
            bump_remember_clear_epoch();
        }
        let key = self.refresh_vault_key.take();
        self.refresh_token = None;
        self.username = None;
        self.last_login_at = None;
        key
    }

    /// Clear fields then delete vault (vault I/O outside prefs lock when possible).
    pub fn clear_session_with_vault(&mut self, vault: &dyn RefreshVault) {
        if let Some(key) = self.clear_session_fields() {
            if let Err(err) = vault.delete(&key) {
                log::warn!("refresh vault delete failed ({err}); prefs key cleared anyway");
            }
        }
    }

    pub fn session_still_valid(&self, now_secs: u64) -> bool {
        match self.last_login_at {
            Some(at) => now_secs.saturating_sub(at) <= SESSION_MAX_AGE_SECS,
            None => false,
        }
    }

    /// True when remember-me restore may attempt vault (or legacy plaintext migrate).
    pub fn has_remembered_refresh(&self) -> bool {
        self.refresh_vault_key.is_some() || self.refresh_token.is_some()
    }
}

/// Move legacy plaintext `refresh_token` into `vault` and drop it from prefs.
///
/// Returns `true` if prefs were mutated (caller should `save`). On vault failure the
/// plaintext is still stripped (session-only) so it never remains on disk after save.
pub fn migrate_legacy_refresh(prefs: &mut OnboardingPrefs, vault: &dyn RefreshVault) -> bool {
    let Some(plain) = prefs.refresh_token.take() else {
        return false;
    };
    let key = prefs.refresh_vault_key.clone().unwrap_or_else(|| {
        let id = refresh_vault::key_id_for_username(prefs.username.as_deref().unwrap_or("legacy"));
        prefs.refresh_vault_key = Some(id.clone());
        id
    });
    match vault.store(&key, &plain) {
        Ok(()) => {
            log::info!("onboarding: migrated plaintext refresh_token into OS vault");
            true
        }
        Err(err) => {
            log::warn!(
                "onboarding: vault migrate failed ({err}); stripping plaintext (session-only)"
            );
            prefs.refresh_vault_key = None;
            prefs.last_login_at = None;
            true
        }
    }
}

/// Production remember-me persist: brief prefs read → vault I/O → locked prefs RMW.
///
/// If logout / remember-me OFF bumps [`REMEMBER_CLEAR_EPOCH`] between `store` and the
/// prefs write, the key is **not** installed and the just-stored vault entry is deleted
/// (same orphan policy as [`sync_rotated_refresh`]).
pub fn persist_remember_me(
    username: &str,
    refresh: &str,
    vault: &dyn RefreshVault,
    mut apply_non_session: impl FnMut(&mut OnboardingPrefs),
) -> bool {
    let key = refresh_vault::key_id_for_username(username);
    let epoch = REMEMBER_CLEAR_EPOCH.load(Ordering::SeqCst);
    let old_key = {
        let _g = prefs_io_lock();
        load_unlocked().refresh_vault_key
    };
    if let Some(ref old) = old_key {
        if old.as_str() != key {
            if let Err(err) = vault.delete(old) {
                log::warn!(
                    "onboarding: prior refresh vault delete failed ({err}); continuing store"
                );
            }
        }
    }
    match vault.store(&key, refresh) {
        Ok(()) => {
            let orphaned = update_prefs(|prefs| {
                apply_non_session(prefs);
                if REMEMBER_CLEAR_EPOCH.load(Ordering::SeqCst) != epoch {
                    // Clear won the race — do not resurrect refresh_vault_key.
                    prefs.refresh_token = None;
                    prefs.username = Some(username.to_string());
                    true
                } else {
                    prefs.refresh_vault_key = Some(key.clone());
                    prefs.refresh_token = None;
                    prefs.username = Some(username.to_string());
                    prefs.last_login_at = Some(now_unix_secs());
                    false
                }
            });
            if orphaned {
                log::info!(
                    "onboarding: persist_remember_me raced past remember-me clear; deleting entry"
                );
                let _ = vault.delete(&key);
                false
            } else {
                true
            }
        }
        Err(err) => {
            log::warn!("onboarding: refresh vault store failed ({err}); session-only");
            if let Some(old) = old_key {
                let _ = vault.delete(&old);
            }
            update_prefs(|prefs| {
                apply_non_session(prefs);
                prefs.refresh_vault_key = None;
                prefs.refresh_token = None;
                prefs.username = Some(username.to_string());
                prefs.last_login_at = None;
            });
            false
        }
    }
}

/// Clear session under prefs lock; delete returned vault key **outside** the lock.
pub fn clear_session_rmw() -> Option<String> {
    update_prefs(|prefs| prefs.clear_session_fields())
}

/// Best-effort vault delete for a key taken from [`clear_session_rmw`] / field clear.
pub fn delete_refresh_vault_key(key: Option<String>, vault: &dyn RefreshVault) {
    if let Some(key) = key {
        if let Err(err) = vault.delete(&key) {
            log::warn!("refresh vault delete failed ({err}); prefs key already cleared");
        }
    }
}

/// Load refresh secret from vault, or legacy plaintext when **no** vault key exists.
///
/// If `refresh_vault_key` is set, an empty/missing vault entry is treated as absent
/// (no legacy fallback) so callers can scrub the stale key id.
pub fn load_refresh_secret(prefs: &OnboardingPrefs, vault: &dyn RefreshVault) -> Option<String> {
    if let Some(key) = prefs.refresh_vault_key.as_deref() {
        return match vault.load(key) {
            Ok(Some(secret)) if !secret.is_empty() => Some(secret),
            Ok(None) | Ok(Some(_)) => {
                log::warn!("onboarding: refresh vault key present but secret missing");
                None
            }
            Err(err) => {
                log::warn!("onboarding: refresh vault load failed ({err})");
                None
            }
        };
    }
    prefs
        .refresh_token
        .as_ref()
        .filter(|s| !s.is_empty())
        .cloned()
}

/// Drop remember-me fields (key id + last_login); keep username. No vault I/O.
///
/// Epoch bumps only when a vault key or legacy plaintext was present.
pub fn clear_remember_me_fields(prefs: &mut OnboardingPrefs) -> Option<String> {
    if prefs.refresh_vault_key.is_some() || prefs.refresh_token.is_some() {
        bump_remember_clear_epoch();
    }
    let key = prefs.refresh_vault_key.take();
    prefs.refresh_token = None;
    prefs.last_login_at = None;
    key
}

fn drop_remember_me_fields(prefs: &mut OnboardingPrefs) -> Option<String> {
    clear_remember_me_fields(prefs)
}

/// Soft-refresh rotation sync (x00cf).
///
/// Spec: current session keeps the rotated refresh in memory; vault must not keep a
/// **revoked** prior token. On store failure → drop remember-me (delete vault + clear
/// key id). On success → bump `last_login_at` only under the prefs IO lock when the
/// key id is still present (remember-me OFF / logout cannot be overwritten; a store
/// that raced past clear is deleted).
pub fn sync_rotated_refresh(refresh: &str, vault: &dyn RefreshVault) {
    let key = {
        let _g = prefs_io_lock();
        load_unlocked().refresh_vault_key
    };
    let Some(key) = key else {
        return;
    };
    // Vault I/O stays outside the prefs lock (avoid blocking UI / deadlock with test
    // vaults that touch prefs).
    match vault.store(&key, refresh) {
        Ok(()) => {
            let orphaned = {
                let _g = prefs_io_lock();
                let mut prefs = load_unlocked();
                if prefs.refresh_vault_key.as_deref() == Some(key.as_str()) {
                    prefs.last_login_at = Some(now_unix_secs());
                    save_unlocked(&prefs);
                    false
                } else {
                    true
                }
            };
            if orphaned {
                log::info!("onboarding: vault store raced past remember-me clear; deleting entry");
                let _ = vault.delete(&key);
            }
        }
        Err(err) => {
            log::warn!(
                "onboarding: refresh vault rotate failed ({err}); dropping remember-me (session continues)"
            );
            // Remove revoked prior secret outside the prefs lock.
            let _ = vault.delete(&key);
            let _g = prefs_io_lock();
            let mut prefs = load_unlocked();
            if prefs.refresh_vault_key.as_deref() == Some(key.as_str()) {
                let _ = drop_remember_me_fields(&mut prefs);
                save_unlocked(&prefs);
            }
        }
    }
}

/// Clear a stale remember-me pointer when vault secret is missing (boot scrub).
pub fn scrub_missing_refresh_secret(prefs: &mut OnboardingPrefs, vault: &dyn RefreshVault) {
    if let Some(key) = drop_remember_me_fields(prefs) {
        delete_refresh_vault_key(Some(key), vault);
    }
}

/// Atomic load → mutate → save under the prefs IO lock (remember-me clear vs soft-refresh).
pub fn update_prefs<R>(f: impl FnOnce(&mut OnboardingPrefs) -> R) -> R {
    let _g = prefs_io_lock();
    let mut prefs = load_unlocked();
    let out = f(&mut prefs);
    save_unlocked(&prefs);
    out
}

pub fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn prefs_path() -> PathBuf {
    if let Ok(appdata) = std::env::var("APPDATA") {
        return PathBuf::from(appdata)
            .join("Alchemy")
            .join("onboarding.json");
    }
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home)
            .join(".config")
            .join("alchemy")
            .join("onboarding.json");
    }
    PathBuf::from("alchemy_onboarding.json")
}

fn load_unlocked() -> OnboardingPrefs {
    let path = prefs_path();
    match fs::read_to_string(&path) {
        Ok(raw) => serde_json::from_str(&raw).unwrap_or_default(),
        Err(_) => OnboardingPrefs::default(),
    }
}

fn save_unlocked(prefs: &OnboardingPrefs) {
    let path = prefs_path();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    match serde_json::to_string_pretty(prefs) {
        Ok(raw) => {
            if let Err(err) = fs::write(&path, raw) {
                log::warn!("onboarding prefs save failed: {err}");
            }
        }
        Err(err) => log::warn!("onboarding prefs encode failed: {err}"),
    }
}

pub fn load() -> OnboardingPrefs {
    let _g = prefs_io_lock();
    load_unlocked()
}

pub fn save(prefs: &OnboardingPrefs) {
    let _g = prefs_io_lock();
    save_unlocked(prefs);
}

/// Resolve boot endpoints: process env wins per-field, else saved operator, else localhost.
///
/// Note: UI `ApplyOperatorEndpoints` later overrides SharedState and process env
/// (`AUTH_URL` / `WORLD_URL` / `ASSETS_URL` / `ZENOH_CONNECT`) so Official/Custom
/// selection is not stuck on a boot-time localhost env. A Running host-server must
/// be restarted to pick up the new URLs.
pub fn boot_endpoints(prefs: &OnboardingPrefs) -> Endpoints {
    let env_auth = std::env::var("AUTH_URL").ok();
    let env_world = std::env::var("WORLD_URL").ok();
    let env_assets = std::env::var("ASSETS_URL").ok();
    let env_zenoh = std::env::var("ZENOH_CONNECT").ok();
    if env_auth.is_some() || env_world.is_some() || env_assets.is_some() || env_zenoh.is_some() {
        let fallback = if prefs.operator_chosen {
            prefs.endpoints()
        } else {
            Endpoints::custom_localhost()
        };
        return Endpoints {
            auth_url: env_auth.unwrap_or(fallback.auth_url),
            world_url: env_world.unwrap_or(fallback.world_url),
            assets_url: env_assets.unwrap_or(fallback.assets_url),
            zenoh_connect: env_zenoh.unwrap_or(fallback.zenoh_connect),
        };
    }
    if prefs.operator_chosen {
        prefs.endpoints()
    } else {
        Endpoints::custom_localhost()
    }
}

/// In-memory vault helper for unit tests only (no epoch / disk RMW).
/// Production code must use [`persist_remember_me`].
#[cfg(test)]
fn persist_refresh_secret(
    prefs: &mut OnboardingPrefs,
    username: &str,
    refresh: &str,
    vault: &dyn RefreshVault,
) -> bool {
    let key = refresh_vault::key_id_for_username(username);
    if let Some(old_key) = prefs.refresh_vault_key.as_deref() {
        if old_key != key {
            let _ = vault.delete(old_key);
        }
    }
    match vault.store(&key, refresh) {
        Ok(()) => {
            prefs.refresh_vault_key = Some(key);
            prefs.refresh_token = None;
            prefs.username = Some(username.to_string());
            prefs.last_login_at = Some(now_unix_secs());
            true
        }
        Err(_) => {
            if let Some(old_key) = prefs.refresh_vault_key.take() {
                let _ = vault.delete(&old_key);
            }
            prefs.refresh_token = None;
            prefs.username = Some(username.to_string());
            prefs.last_login_at = None;
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Serializes tests that rewrite `APPDATA` / `HOME` for prefs_path isolation.
    static PREFS_ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn expand_bare_host() {
        let ep = expand_custom("127.0.0.1");
        assert_eq!(ep.auth_url, "http://127.0.0.1:4002");
        assert_eq!(ep.world_url, "http://127.0.0.1:4003");
        assert_eq!(ep.zenoh_connect, "tcp/127.0.0.1:7447");
    }

    #[test]
    fn expand_bare_host_keeps_explicit_auth_port() {
        let ep = expand_custom("192.168.1.5:9999");
        assert_eq!(ep.auth_url, "http://192.168.1.5:9999");
        assert_eq!(ep.world_url, "http://192.168.1.5:4003");
        assert_eq!(ep.zenoh_connect, "tcp/192.168.1.5:7447");
    }

    #[test]
    fn expand_http_auth_url() {
        let ep = expand_custom("http://192.168.1.5:4002");
        assert_eq!(ep.auth_url, "http://192.168.1.5:4002");
        assert_eq!(ep.world_url, "http://192.168.1.5:4003");
        assert_eq!(ep.zenoh_connect, "tcp/192.168.1.5:7447");
    }

    #[test]
    fn expand_https_host_omits_dev_ports() {
        let ep = expand_custom("https://example.com");
        assert_eq!(ep.auth_url, "https://example.com");
        assert_eq!(ep.world_url, "https://example.com");
        assert_eq!(ep.zenoh_connect, "tls/example.com:7447");
    }

    #[test]
    fn expand_https_443_matches_host_only() {
        let a = expand_custom("https://example.com");
        let b = expand_custom("https://example.com:443");
        assert_eq!(a, b);
    }

    #[test]
    fn boot_endpoints_honors_zenoh_env_alone() {
        let prev = std::env::var("ZENOH_CONNECT").ok();
        std::env::set_var("ZENOH_CONNECT", "tls/relay.example:7447");
        let ep = boot_endpoints(&OnboardingPrefs::default());
        assert_eq!(ep.zenoh_connect, "tls/relay.example:7447");
        assert_eq!(ep.auth_url, "http://127.0.0.1:4002");
        match prev {
            Some(v) => std::env::set_var("ZENOH_CONNECT", v),
            None => std::env::remove_var("ZENOH_CONNECT"),
        }
    }

    #[test]
    fn session_age_gate() {
        let mut p = OnboardingPrefs::default();
        assert!(!p.session_still_valid(1_000_000));
        p.last_login_at = Some(1_000_000);
        assert!(p.session_still_valid(1_000_000 + SESSION_MAX_AGE_SECS));
        assert!(!p.session_still_valid(1_000_000 + SESSION_MAX_AGE_SECS + 1));
    }

    #[test]
    fn official_constants() {
        let ep = Endpoints::official();
        assert!(ep.auth_url.contains("frick-eldy.com"));
        assert!(ep.zenoh_connect.starts_with("tls/"));
    }

    #[test]
    fn save_json_omits_refresh_token_field() {
        let vault = refresh_vault::MemoryRefreshVault::new();
        let mut prefs = OnboardingPrefs::default();
        assert!(persist_refresh_secret(
            &mut prefs,
            "alice",
            "rt-secret",
            &vault
        ));
        let raw = serde_json::to_string_pretty(&prefs).unwrap();
        assert!(
            !raw.contains("refresh_token"),
            "prefs must not serialize refresh_token: {raw}"
        );
        assert!(raw.contains("refresh_vault_key"));
        assert!(!raw.contains("rt-secret"));
        assert_eq!(
            load_refresh_secret(&prefs, &vault).as_deref(),
            Some("rt-secret")
        );
    }

    #[test]
    fn migrate_legacy_plaintext_into_vault() {
        let vault = refresh_vault::MemoryRefreshVault::new();
        let mut prefs = OnboardingPrefs {
            username: Some("bob".into()),
            refresh_token: Some("legacy-rt".into()),
            last_login_at: Some(1),
            ..OnboardingPrefs::default()
        };
        assert!(migrate_legacy_refresh(&mut prefs, &vault));
        assert!(prefs.refresh_token.is_none());
        assert_eq!(prefs.refresh_vault_key.as_deref(), Some("user:bob"));
        assert_eq!(
            load_refresh_secret(&prefs, &vault).as_deref(),
            Some("legacy-rt")
        );
        let raw = serde_json::to_string(&prefs).unwrap();
        assert!(!raw.contains("legacy-rt"));
        assert!(!raw.contains("\"refresh_token\""));
    }

    #[test]
    fn vault_failure_is_session_only_no_plaintext() {
        let vault = refresh_vault::UnavailableRefreshVault;
        let mut prefs = OnboardingPrefs::default();
        assert!(!persist_refresh_secret(
            &mut prefs,
            "carol",
            "rt-secret",
            &vault
        ));
        assert!(prefs.refresh_vault_key.is_none());
        assert!(prefs.refresh_token.is_none());
        assert!(prefs.last_login_at.is_none());
        assert_eq!(prefs.username.as_deref(), Some("carol"));
        let raw = serde_json::to_string(&prefs).unwrap();
        assert!(!raw.contains("rt-secret"));
    }

    #[test]
    fn migrate_vault_failure_strips_plaintext() {
        let vault = refresh_vault::UnavailableRefreshVault;
        let mut prefs = OnboardingPrefs {
            username: Some("dave".into()),
            refresh_token: Some("plain-rt".into()),
            last_login_at: Some(42),
            ..OnboardingPrefs::default()
        };
        assert!(migrate_legacy_refresh(&mut prefs, &vault));
        assert!(prefs.refresh_token.is_none());
        assert!(prefs.refresh_vault_key.is_none());
        assert!(prefs.last_login_at.is_none());
    }

    #[test]
    fn clear_session_deletes_vault_entry() {
        let vault = refresh_vault::MemoryRefreshVault::new();
        let mut prefs = OnboardingPrefs::default();
        assert!(persist_refresh_secret(
            &mut prefs,
            "erin",
            "rt-secret",
            &vault
        ));
        prefs.clear_session_with_vault(&vault);
        assert!(prefs.refresh_vault_key.is_none());
        assert_eq!(vault.load("user:erin").unwrap(), None);
    }

    #[test]
    fn persist_deletes_prior_user_vault_entry() {
        let vault = refresh_vault::MemoryRefreshVault::new();
        let mut prefs = OnboardingPrefs::default();
        assert!(persist_refresh_secret(
            &mut prefs, "olduser", "rt-old", &vault
        ));
        assert!(persist_refresh_secret(
            &mut prefs, "newuser", "rt-new", &vault
        ));
        assert_eq!(vault.load("user:olduser").unwrap(), None);
        assert_eq!(
            vault.load("user:newuser").unwrap().as_deref(),
            Some("rt-new")
        );
        assert_eq!(prefs.refresh_vault_key.as_deref(), Some("user:newuser"));
    }

    #[test]
    fn load_with_vault_key_does_not_fall_back_to_legacy() {
        let vault = refresh_vault::MemoryRefreshVault::new();
        let prefs = OnboardingPrefs {
            refresh_vault_key: Some("user:ghost".into()),
            refresh_token: Some("legacy-should-not-use".into()),
            ..OnboardingPrefs::default()
        };
        assert!(load_refresh_secret(&prefs, &vault).is_none());
    }

    #[test]
    fn clear_without_remember_me_does_not_bump_epoch() {
        let before = REMEMBER_CLEAR_EPOCH.load(Ordering::SeqCst);
        let mut prefs = OnboardingPrefs::default();
        assert!(prefs.clear_session_fields().is_none());
        assert_eq!(REMEMBER_CLEAR_EPOCH.load(Ordering::SeqCst), before);
        assert!(clear_remember_me_fields(&mut prefs).is_none());
        assert_eq!(REMEMBER_CLEAR_EPOCH.load(Ordering::SeqCst), before);

        prefs.refresh_vault_key = Some("user:x".into());
        assert_eq!(prefs.clear_session_fields().as_deref(), Some("user:x"));
        assert_eq!(REMEMBER_CLEAR_EPOCH.load(Ordering::SeqCst), before + 1);
    }

    #[test]
    fn persist_remember_me_deletes_orphan_when_clear_races_store() {
        let _guard = PREFS_ENV_LOCK.lock().unwrap();
        struct ClearOnStoreVault {
            inner: refresh_vault::MemoryRefreshVault,
        }
        impl refresh_vault::RefreshVault for ClearOnStoreVault {
            fn store(&self, key_id: &str, secret: &str) -> Result<(), refresh_vault::VaultError> {
                self.inner.store(key_id, secret)?;
                let _ = clear_session_rmw();
                Ok(())
            }
            fn load(&self, key_id: &str) -> Result<Option<String>, refresh_vault::VaultError> {
                self.inner.load(key_id)
            }
            fn delete(&self, key_id: &str) -> Result<(), refresh_vault::VaultError> {
                self.inner.delete(key_id)
            }
        }

        let vault = ClearOnStoreVault {
            inner: refresh_vault::MemoryRefreshVault::new(),
        };
        let dir = std::env::temp_dir().join(format!(
            "alchemy-prefs-persist-race-{}-kara",
            std::process::id()
        ));
        let _ = fs::create_dir_all(&dir);
        let prev_appdata = std::env::var("APPDATA").ok();
        let prev_home = std::env::var("HOME").ok();
        std::env::set_var("APPDATA", &dir);
        std::env::remove_var("HOME");
        // Seed an existing key so mid-store clear bumps REMEMBER_CLEAR_EPOCH.
        let mut seeded = OnboardingPrefs::default();
        seeded.refresh_vault_key = Some("user:kara".into());
        seeded.last_login_at = Some(1);
        save(&seeded);

        assert!(!persist_remember_me("kara", "rt-new", &vault, |_| {}));
        assert!(load().refresh_vault_key.is_none());
        assert_eq!(vault.inner.load("user:kara").unwrap(), None);

        match prev_appdata {
            Some(v) => std::env::set_var("APPDATA", v),
            None => std::env::remove_var("APPDATA"),
        }
        match prev_home {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn unrelated_prefs_rmw_does_not_resurrect_cleared_remember_me() {
        let _guard = PREFS_ENV_LOCK.lock().unwrap();
        let vault = refresh_vault::MemoryRefreshVault::new();
        let mut prefs = OnboardingPrefs::default();
        assert!(persist_refresh_secret(
            &mut prefs,
            "judy",
            "rt-secret",
            &vault
        ));
        let dir = std::env::temp_dir().join(format!(
            "alchemy-prefs-unrelated-{}-judy",
            std::process::id()
        ));
        let _ = fs::create_dir_all(&dir);
        let prev_appdata = std::env::var("APPDATA").ok();
        let prev_home = std::env::var("HOME").ok();
        std::env::set_var("APPDATA", &dir);
        std::env::remove_var("HOME");
        save(&prefs);

        let key = clear_session_rmw();
        delete_refresh_vault_key(key, &vault);
        assert!(load().refresh_vault_key.is_none());

        // Simulate ServerOperatorChanged / ApplyOperatorEndpoints RMW.
        update_prefs(|p| {
            p.server_operator = true;
        });
        let after = load();
        assert!(
            after.refresh_vault_key.is_none(),
            "unrelated prefs save must not resurrect remember-me"
        );
        assert!(after.server_operator);
        assert_eq!(vault.load("user:judy").unwrap(), None);

        match prev_appdata {
            Some(v) => std::env::set_var("APPDATA", v),
            None => std::env::remove_var("APPDATA"),
        }
        match prev_home {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn scrub_missing_refresh_secret_clears_key_and_vault() {
        let vault = refresh_vault::MemoryRefreshVault::new();
        let mut prefs = OnboardingPrefs::default();
        assert!(persist_refresh_secret(
            &mut prefs,
            "ivan",
            "rt-secret",
            &vault
        ));
        // Simulate boot: key present, vault empty.
        vault.delete("user:ivan").unwrap();
        assert!(load_refresh_secret(&prefs, &vault).is_none());
        scrub_missing_refresh_secret(&mut prefs, &vault);
        assert!(prefs.refresh_vault_key.is_none());
        assert!(prefs.last_login_at.is_none());
        assert!(!prefs.has_remembered_refresh());
        assert_eq!(vault.load("user:ivan").unwrap(), None);
    }

    #[test]
    fn sync_rotated_refresh_updates_vault_and_last_login() {
        let _guard = PREFS_ENV_LOCK.lock().unwrap();
        let vault = refresh_vault::MemoryRefreshVault::new();
        let mut prefs = OnboardingPrefs::default();
        assert!(persist_refresh_secret(
            &mut prefs, "frank", "rt-old", &vault
        ));
        // Isolate prefs path so sync_rotated_refresh load/save does not touch real disk.
        let dir =
            std::env::temp_dir().join(format!("alchemy-prefs-test-{}-frank", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let prev_appdata = std::env::var("APPDATA").ok();
        let prev_home = std::env::var("HOME").ok();
        std::env::set_var("APPDATA", &dir);
        std::env::remove_var("HOME");
        save(&prefs);

        sync_rotated_refresh("rt-new", &vault);

        let after = load();
        assert_eq!(
            load_refresh_secret(&after, &vault).as_deref(),
            Some("rt-new")
        );
        assert!(after.last_login_at.is_some());
        assert_eq!(after.refresh_vault_key.as_deref(), Some("user:frank"));

        match prev_appdata {
            Some(v) => std::env::set_var("APPDATA", v),
            None => std::env::remove_var("APPDATA"),
        }
        match prev_home {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn sync_rotated_refresh_failure_drops_remember_me() {
        let _guard = PREFS_ENV_LOCK.lock().unwrap();
        // Seed prefs on a temp path, then rotate against an unavailable vault.
        let mem = refresh_vault::MemoryRefreshVault::new();
        let mut prefs = OnboardingPrefs::default();
        assert!(persist_refresh_secret(&mut prefs, "gina", "rt-old", &mem));
        let dir =
            std::env::temp_dir().join(format!("alchemy-prefs-drop-{}-gina", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let prev_appdata = std::env::var("APPDATA").ok();
        let prev_home = std::env::var("HOME").ok();
        std::env::set_var("APPDATA", &dir);
        std::env::remove_var("HOME");
        save(&prefs);

        sync_rotated_refresh("rt-new", &refresh_vault::UnavailableRefreshVault);

        let after = load();
        assert!(after.refresh_vault_key.is_none());
        assert!(after.last_login_at.is_none());
        assert_eq!(after.username.as_deref(), Some("gina"));
        // Memory still had the old secret; unavailable path deletes via UnavailableRefreshVault
        // (no-op delete err ignored). Ensure prefs no longer points at it.
        assert!(!after.has_remembered_refresh());

        match prev_appdata {
            Some(v) => std::env::set_var("APPDATA", v),
            None => std::env::remove_var("APPDATA"),
        }
        match prev_home {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
        let _ = fs::remove_dir_all(&dir);
    }

    /// Clears prefs during `store` to emulate logout racing soft-refresh.
    struct ClearPrefsOnStoreVault {
        inner: refresh_vault::MemoryRefreshVault,
    }

    impl refresh_vault::RefreshVault for ClearPrefsOnStoreVault {
        fn store(&self, key_id: &str, secret: &str) -> Result<(), refresh_vault::VaultError> {
            self.inner.store(key_id, secret)?;
            let mut prefs = load();
            prefs.clear_session_fields();
            prefs.username = Some("helen".into());
            save(&prefs);
            Ok(())
        }

        fn load(&self, key_id: &str) -> Result<Option<String>, refresh_vault::VaultError> {
            self.inner.load(key_id)
        }

        fn delete(&self, key_id: &str) -> Result<(), refresh_vault::VaultError> {
            self.inner.delete(key_id)
        }
    }

    #[test]
    fn sync_rotated_refresh_does_not_resurrect_cleared_remember_me() {
        let _guard = PREFS_ENV_LOCK.lock().unwrap();
        let vault = ClearPrefsOnStoreVault {
            inner: refresh_vault::MemoryRefreshVault::new(),
        };
        let dir =
            std::env::temp_dir().join(format!("alchemy-prefs-race-{}-helen", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let prev_appdata = std::env::var("APPDATA").ok();
        let prev_home = std::env::var("HOME").ok();
        std::env::set_var("APPDATA", &dir);
        std::env::remove_var("HOME");

        let mut prefs = OnboardingPrefs::default();
        assert!(persist_refresh_secret(
            &mut prefs,
            "helen",
            "rt-old",
            &vault.inner
        ));
        save(&prefs);

        sync_rotated_refresh("rt-new", &vault);

        let after = load();
        assert!(
            after.refresh_vault_key.is_none(),
            "cleared remember-me must not be resurrected"
        );
        assert_eq!(
            vault.inner.load("user:helen").unwrap(),
            None,
            "orphaned vault entry from raced store must be deleted"
        );

        match prev_appdata {
            Some(v) => std::env::set_var("APPDATA", v),
            None => std::env::remove_var("APPDATA"),
        }
        match prev_home {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
        let _ = fs::remove_dir_all(&dir);
    }
}
