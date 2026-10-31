//! Home-node Connect endpoints (x0066).
//!
//! Guarantees: resolve priority env > `connect.json` > compile defaults; persist without
//! secrets; simple domain expands to auth/assets/world + one relay; selected relay is one
//! `ZENOH_CONNECT` (+ optional `RELAY_ALLOCATE_URL` for non-local).
//! Non-guarantees: federation session overrides; multi-relay failover; TLS CA UI.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

pub const DEFAULT_AUTH_URL: &str = "http://127.0.0.1:4002";
pub const DEFAULT_ASSETS_URL: &str = "http://127.0.0.1:4004";
pub const DEFAULT_WORLD_URL: &str = "http://127.0.0.1:4003";
pub const DEFAULT_ZENOH_CONNECT: &str = "tcp/127.0.0.1:7447";
pub const PROFILE_VERSION: u32 = 1;

const ENV_AUTH: &str = "AUTH_URL";
const ENV_ASSETS: &str = "ASSETS_URL";
const ENV_WORLD: &str = "WORLD_URL";
const ENV_ZENOH: &str = "ZENOH_CONNECT";
const ENV_RELAY_ALLOCATE: &str = "RELAY_ALLOCATE_URL";
const ENV_CONNECT_PATH: &str = "ALCHEMY_CONNECT_CONFIG";

/// Persisted / resolved home-node endpoints (no passwords).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectProfile {
    #[serde(default = "profile_version_default")]
    pub version: u32,
    /// When set and matches expanded endpoints, UI can stay in simple mode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub simple_domain: Option<String>,
    pub auth_url: String,
    pub assets_url: String,
    pub world_url: String,
    /// Zenoh connect endpoints (`tcp/…` or `tls/…`).
    pub relays: Vec<String>,
    pub selected_relay: usize,
    /// Parallel to `relays` when known; empty string = leave `RELAY_ALLOCATE_URL` unset.
    #[serde(default)]
    pub relay_allocate_urls: Vec<String>,
}

fn profile_version_default() -> u32 {
    PROFILE_VERSION
}

impl Default for ConnectProfile {
    fn default() -> Self {
        Self {
            version: PROFILE_VERSION,
            simple_domain: None,
            auth_url: DEFAULT_AUTH_URL.to_string(),
            assets_url: DEFAULT_ASSETS_URL.to_string(),
            world_url: DEFAULT_WORLD_URL.to_string(),
            relays: vec![DEFAULT_ZENOH_CONNECT.to_string()],
            selected_relay: 0,
            relay_allocate_urls: vec![String::new()],
        }
    }
}

impl ConnectProfile {
    pub fn selected_zenoh(&self) -> &str {
        self.relays
            .get(self.selected_relay)
            .map(String::as_str)
            .unwrap_or(DEFAULT_ZENOH_CONNECT)
    }

    pub fn selected_allocate_url(&self) -> Option<&str> {
        let url = self
            .relay_allocate_urls
            .get(self.selected_relay)
            .map(String::as_str)
            .unwrap_or("");
        if url.is_empty() {
            None
        } else {
            Some(url)
        }
    }

    /// Normalize indices and ensure at least one relay (no hard cap — operators may list many).
    pub fn sanitize(&mut self) {
        if self.relays.is_empty() {
            self.relays.push(DEFAULT_ZENOH_CONNECT.to_string());
        }
        while self.relay_allocate_urls.len() < self.relays.len() {
            self.relay_allocate_urls.push(String::new());
        }
        self.relay_allocate_urls.truncate(self.relays.len());
        if self.selected_relay >= self.relays.len() {
            self.selected_relay = 0;
        }
        self.auth_url = self.auth_url.trim().trim_end_matches('/').to_string();
        self.assets_url = self.assets_url.trim().trim_end_matches('/').to_string();
        self.world_url = self.world_url.trim().trim_end_matches('/').to_string();
        for r in &mut self.relays {
            *r = r.trim().to_string();
        }
        for a in &mut self.relay_allocate_urls {
            *a = a.trim().trim_end_matches('/').to_string();
        }
    }

    /// Fill empty allocate slots from zenoh endpoint hints (`tls/host:7447` → `https://host:7450`).
    pub fn sync_empty_allocate_hints(&mut self) {
        for i in 0..self.relays.len() {
            if self
                .relay_allocate_urls
                .get(i)
                .map(|s| s.is_empty())
                .unwrap_or(true)
            {
                let hint = allocate_hint_from_zenoh(&self.relays[i]);
                if self.relay_allocate_urls.len() <= i {
                    self.relay_allocate_urls.push(hint);
                } else {
                    self.relay_allocate_urls[i] = hint;
                }
            }
        }
    }

    pub fn matches_simple_expand(&self, domain: &str) -> bool {
        let expanded = expand_simple_domain(domain);
        self.auth_url == expanded.auth_url
            && self.assets_url == expanded.assets_url
            && self.world_url == expanded.world_url
            && self.relays == expanded.relays
            && self.relay_allocate_urls == expanded.relay_allocate_urls
            && self.selected_relay == expanded.selected_relay
    }
}

/// Derive Relay Agent URL from a Zenoh connect string, or empty for local tcp.
///
/// `tls/relay.example.tld:7447` → `https://relay.example.tld:7450`  
/// `tcp/127.0.0.1:7447` → `""` (no allocate)
pub fn allocate_hint_from_zenoh(zenoh: &str) -> String {
    let z = zenoh.trim();
    let Some(rest) = z.strip_prefix("tls/") else {
        return String::new();
    };
    let host = rest.split(':').next().unwrap_or("").trim();
    if host.is_empty() || host == "127.0.0.1" || host == "localhost" {
        return String::new();
    }
    format!("https://{host}:7450")
}

/// Expand `example.tld` → https auth/assets/world + tls relay (+ allocate https :7450).
pub fn expand_simple_domain(domain: &str) -> ConnectProfile {
    let d = domain
        .trim()
        .trim_start_matches("https://")
        .trim_start_matches("http://");
    let d = d.trim_end_matches('/');
    let d = d.trim();
    ConnectProfile {
        version: PROFILE_VERSION,
        simple_domain: Some(d.to_string()),
        auth_url: format!("https://auth.{d}"),
        assets_url: format!("https://assets.{d}"),
        world_url: format!("https://world.{d}"),
        relays: vec![format!("tls/relay.{d}:7447")],
        selected_relay: 0,
        relay_allocate_urls: vec![format!("https://relay.{d}:7450")],
    }
}

/// Default config file path (`ALCHEMY_CONNECT_CONFIG` overrides).
pub fn default_config_path() -> PathBuf {
    if let Ok(p) = env::var(ENV_CONNECT_PATH) {
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    config_dir().join("connect.json")
}

fn config_dir() -> PathBuf {
    if cfg!(windows) {
        if let Ok(base) = env::var("LOCALAPPDATA") {
            return PathBuf::from(base).join("alchemy");
        }
    }
    if let Ok(xdg) = env::var("XDG_CONFIG_HOME") {
        if !xdg.is_empty() {
            return PathBuf::from(xdg).join("alchemy");
        }
    }
    if let Ok(home) = env::var("HOME") {
        return PathBuf::from(home).join(".config").join("alchemy");
    }
    // Last resort (Windows without LOCALAPPDATA, etc.)
    PathBuf::from("alchemy-config")
}

pub fn load_file(path: &Path) -> Result<Option<ConnectProfile>> {
    if !path.is_file() {
        return Ok(None);
    }
    let raw = fs::read_to_string(path)
        .with_context(|| format!("read connect config {}", path.display()))?;
    let mut profile: ConnectProfile = serde_json::from_str(&raw).context("parse connect.json")?;
    profile.sanitize();
    Ok(Some(profile))
}

pub fn save_file(path: &Path, profile: &ConnectProfile) -> Result<()> {
    let mut p = profile.clone();
    p.sanitize();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create connect config dir {}", parent.display()))?;
    }
    let body = serde_json::to_string_pretty(&p).context("serialize connect.json")?;
    fs::write(path, body).with_context(|| format!("write connect config {}", path.display()))?;
    Ok(())
}

/// Resolve effective profile: per-field env wins over file over defaults.
pub fn resolve() -> Result<ConnectProfile> {
    resolve_with_path(&default_config_path())
}

pub fn resolve_with_path(path: &Path) -> Result<ConnectProfile> {
    let mut profile = load_file(path)?.unwrap_or_default();
    overlay_env(&mut profile);
    profile.sanitize();
    Ok(profile)
}

fn overlay_env(profile: &mut ConnectProfile) {
    if let Ok(v) = env::var(ENV_AUTH) {
        if !v.is_empty() {
            profile.auth_url = v;
            profile.simple_domain = None;
        }
    }
    if let Ok(v) = env::var(ENV_ASSETS) {
        if !v.is_empty() {
            profile.assets_url = v;
            profile.simple_domain = None;
        }
    }
    if let Ok(v) = env::var(ENV_WORLD) {
        if !v.is_empty() {
            profile.world_url = v;
            profile.simple_domain = None;
        }
    }
    if let Ok(v) = env::var(ENV_ZENOH) {
        if !v.is_empty() {
            ensure_relay_slot(profile, 0);
            profile.relays[0] = v;
            profile.selected_relay = 0;
            profile.simple_domain = None;
        }
    }
    if let Ok(v) = env::var(ENV_RELAY_ALLOCATE) {
        ensure_relay_slot(profile, profile.selected_relay);
        profile.relay_allocate_urls[profile.selected_relay] = v;
        profile.simple_domain = None;
    }
}

fn ensure_relay_slot(profile: &mut ConnectProfile, idx: usize) {
    while profile.relays.len() <= idx {
        profile.relays.push(DEFAULT_ZENOH_CONNECT.to_string());
    }
    while profile.relay_allocate_urls.len() <= idx {
        profile.relay_allocate_urls.push(String::new());
    }
}

/// Write selected endpoints into process env (launcher / host / **client main before threads**).
///
/// **Do not call from Connect Apply** while token-refresh / JWKS / Zenoh threads run —
/// concurrent `set_var` is unsound. Runtime Apply updates `SharedState` + `connect.json` only.
pub fn apply_to_env(profile: &ConnectProfile) {
    let mut p = profile.clone();
    p.sanitize();
    // SAFETY: caller guarantees no other threads are reading/writing the process environment
    // (startup of launcher, host-server, or client `main` before background workers).
    unsafe {
        env::set_var(ENV_AUTH, &p.auth_url);
        env::set_var(ENV_ASSETS, &p.assets_url);
        env::set_var(ENV_WORLD, &p.world_url);
        env::set_var(ENV_ZENOH, p.selected_zenoh());
        match p.selected_allocate_url() {
            Some(url) => env::set_var(ENV_RELAY_ALLOCATE, url),
            None => env::remove_var(ENV_RELAY_ALLOCATE),
        }
    }
}

/// Load file (if any) and apply only keys that are not already set in the environment.
/// Used by launcher / host so CI env still wins.
pub fn apply_file_defaults_to_env() -> Result<()> {
    apply_file_defaults_to_env_at(&default_config_path())
}

pub fn apply_file_defaults_to_env_at(path: &Path) -> Result<()> {
    let Some(file) = load_file(path)? else {
        return Ok(());
    };
    set_if_unset(ENV_AUTH, &file.auth_url);
    set_if_unset(ENV_ASSETS, &file.assets_url);
    set_if_unset(ENV_WORLD, &file.world_url);
    set_if_unset(ENV_ZENOH, file.selected_zenoh());
    if let Some(url) = file.selected_allocate_url() {
        set_if_unset(ENV_RELAY_ALLOCATE, url);
    }
    Ok(())
}

fn set_if_unset(key: &str, value: &str) {
    match env::var(key) {
        Ok(v) if !v.is_empty() => {}
        _ => {
            // SAFETY: same startup-only contract as [`apply_to_env`].
            unsafe {
                env::set_var(key, value);
            }
        }
    }
}

/// Validate UI draft before persist.
///
/// Checks non-empty auth/assets/world, non-empty relays, `http(s)://` on HTTP bases,
/// and `tcp/` or `tls/` prefix on Zenoh endpoints. Does **not** DNS-resolve or probe.
pub fn validate_for_apply(profile: &ConnectProfile) -> Result<()> {
    let mut p = profile.clone();
    p.sanitize();
    if p.auth_url.is_empty() || p.assets_url.is_empty() || p.world_url.is_empty() {
        bail!("auth / assets / world URLs must be non-empty");
    }
    for (name, url) in [
        ("auth", p.auth_url.as_str()),
        ("assets", p.assets_url.as_str()),
        ("world", p.world_url.as_str()),
    ] {
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            bail!("{name} URL must start with http:// or https://");
        }
    }
    if p.relays.iter().any(|r| r.is_empty()) {
        bail!("relay endpoints must be non-empty");
    }
    for r in &p.relays {
        if !(r.starts_with("tcp/") || r.starts_with("tls/")) {
            bail!("relay must start with tcp/ or tls/ (got {r:?})");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // Env mutations serialize across tests in this module.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn expand_simple_domain_shapes() {
        let p = expand_simple_domain("example.tld");
        assert_eq!(p.auth_url, "https://auth.example.tld");
        assert_eq!(p.assets_url, "https://assets.example.tld");
        assert_eq!(p.world_url, "https://world.example.tld");
        assert_eq!(p.relays, vec!["tls/relay.example.tld:7447"]);
        assert_eq!(
            p.relay_allocate_urls,
            vec!["https://relay.example.tld:7450"]
        );
        assert!(p.matches_simple_expand("example.tld"));
    }

    #[test]
    fn allocate_hint_from_zenoh_tls_and_tcp() {
        assert_eq!(
            allocate_hint_from_zenoh("tls/relay.example.tld:7447"),
            "https://relay.example.tld:7450"
        );
        assert!(allocate_hint_from_zenoh("tcp/127.0.0.1:7447").is_empty());
        assert!(allocate_hint_from_zenoh("tcp/localhost:7447").is_empty());
    }

    #[test]
    fn validate_rejects_bad_schemes() {
        let bad_auth = ConnectProfile {
            auth_url: "ftp://nope".into(),
            ..ConnectProfile::default()
        };
        assert!(validate_for_apply(&bad_auth).is_err());
        let bad_relay = ConnectProfile {
            relays: vec!["http://not-zenoh".into()],
            ..ConnectProfile::default()
        };
        assert!(validate_for_apply(&bad_relay).is_err());
    }

    #[test]
    fn serde_roundtrip() {
        let p = expand_simple_domain("acme.test");
        let raw = serde_json::to_string(&p).unwrap();
        let back: ConnectProfile = serde_json::from_str(&raw).unwrap();
        assert_eq!(p, back);
    }

    #[test]
    fn resolve_env_overrides_file() {
        let _g = ENV_LOCK.lock().unwrap();
        let dir = std::env::temp_dir().join(format!("alchemy-connect-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("connect.json");
        let file = expand_simple_domain("file.tld");
        save_file(&path, &file).unwrap();

        unsafe {
            env::set_var(ENV_AUTH, "http://env-auth:9");
            env::remove_var(ENV_ASSETS);
            env::remove_var(ENV_WORLD);
            env::remove_var(ENV_ZENOH);
            env::remove_var(ENV_RELAY_ALLOCATE);
        }
        let resolved = resolve_with_path(&path).unwrap();
        assert_eq!(resolved.auth_url, "http://env-auth:9");
        assert_eq!(resolved.world_url, file.world_url);
        unsafe {
            env::remove_var(ENV_AUTH);
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn apply_file_defaults_skips_set_env() {
        let _g = ENV_LOCK.lock().unwrap();
        let dir =
            std::env::temp_dir().join(format!("alchemy-connect-defaults-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("connect.json");
        save_file(&path, &expand_simple_domain("def.tld")).unwrap();
        unsafe {
            env::set_var(ENV_AUTH, "http://keep-me");
            env::remove_var(ENV_WORLD);
        }
        apply_file_defaults_to_env_at(&path).unwrap();
        assert_eq!(env::var(ENV_AUTH).unwrap(), "http://keep-me");
        assert_eq!(env::var(ENV_WORLD).unwrap(), "https://world.def.tld");
        unsafe {
            env::remove_var(ENV_AUTH);
            env::remove_var(ENV_WORLD);
            env::remove_var(ENV_ASSETS);
            env::remove_var(ENV_ZENOH);
            env::remove_var(ENV_RELAY_ALLOCATE);
        }
        let _ = fs::remove_dir_all(&dir);
    }
}
