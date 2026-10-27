//! Shared Zenoh session open helpers for host / client / e2e-probe (x0022).
//!
//! Guarantees: `ZENOH_CONNECT` endpoint + optional `ZENOH_TLS_ROOT_CA` via JSON5 insert.
//! Non-guarantees: mTLS client certs, endpoint validation beyond string insert.

use anyhow::{anyhow, Result};
use zenoh::Config;
use zenoh::Wait;

/// Insert client mode + connect endpoint. `endpoint` is e.g. `tcp/127.0.0.1:7447` or `tls/host:7447`.
pub fn apply_connect_endpoint(config: &mut Config, endpoint: &str) -> Result<()> {
    config
        .insert_json5("mode", r#""client""#)
        .map_err(|e| anyhow!("zenoh mode: {e}"))?;
    let endpoints = serde_json::to_string(&vec![endpoint])
        .map_err(|e| anyhow!("zenoh connect endpoints json: {e}"))?;
    config
        .insert_json5("connect/endpoints", &endpoints)
        .map_err(|e| anyhow!("zenoh connect: {e}"))?;
    Ok(())
}

/// If `ZENOH_TLS_ROOT_CA` is set, insert `transport/link/tls/root_ca_certificate`.
pub fn apply_tls_root_ca_from_env(config: &mut Config) -> Result<()> {
    let Ok(ca) = std::env::var("ZENOH_TLS_ROOT_CA") else {
        return Ok(());
    };
    if ca.is_empty() {
        return Ok(());
    }
    let value = serde_json::to_string(&ca).map_err(|e| anyhow!("zenoh tls root ca json: {e}"))?;
    config
        .insert_json5("transport/link/tls/root_ca_certificate", &value)
        .map_err(|e| anyhow!("zenoh tls root ca: {e}"))?;
    log::info!("zenoh tls root_ca_certificate={ca}");
    Ok(())
}

/// Open a client session for `endpoint` (applies TLS root CA env when set).
pub fn open_client(endpoint: &str) -> Result<zenoh::Session> {
    let mut config = Config::default();
    apply_connect_endpoint(&mut config, endpoint)?;
    apply_tls_root_ca_from_env(&mut config)?;
    log::info!("zenoh connect={endpoint}");
    zenoh::open(config)
        .wait()
        .map_err(|e| anyhow!("zenoh open failed: {e}"))
}
