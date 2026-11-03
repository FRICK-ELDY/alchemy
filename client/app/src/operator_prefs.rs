//! Operator / onboarding prefs (system_ui Step1–3 shell).
//!
//! Guarantees: OFFICIAL vs CUSTOM endpoint expansion; disk prefs for operator choice,
//! optional refresh_token, last_login_at (7-day gate); no secrets in logs.
//! Non-guarantees: encrypted vault, OS keychain, Federation directory, host-server env sync.
//!
//! Security note: `refresh_token` is stored plaintext under the user config dir
//! (`%APPDATA%/Alchemy/onboarding.json` on Windows). Treat the machine profile as the
//! trust boundary; failed restore / logout clears the token from disk.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

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
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub username: Option<String>,
    /// Unix seconds of last successful login/register/refresh restore.
    #[serde(default)]
    pub last_login_at: Option<u64>,
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
            refresh_token: None,
            username: None,
            last_login_at: None,
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

    pub fn clear_session(&mut self) {
        self.refresh_token = None;
        self.username = None;
        self.last_login_at = None;
    }

    pub fn session_still_valid(&self, now_secs: u64) -> bool {
        match self.last_login_at {
            Some(at) => now_secs.saturating_sub(at) <= SESSION_MAX_AGE_SECS,
            None => false,
        }
    }
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

pub fn load() -> OnboardingPrefs {
    let path = prefs_path();
    match fs::read_to_string(&path) {
        Ok(raw) => serde_json::from_str(&raw).unwrap_or_default(),
        Err(_) => OnboardingPrefs::default(),
    }
}

pub fn save(prefs: &OnboardingPrefs) {
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

/// Resolve boot endpoints: process env wins per-field, else saved operator, else localhost.
///
/// Note: UI `ApplyOperatorEndpoints` later overrides SharedState + `ZENOH_CONNECT` for this
/// process so Official/Custom selection is not stuck on a boot-time localhost env.
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
