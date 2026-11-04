//! Inventory take-out spawn control plane (x006d).
//!
//! Guarantees: Zenoh `…/inventory/spawn` with Room Token **host** role; host never
//! receives Drive `node_id` — only blake3 + pose; world scene append then local
//! Rapier + shared scene update for re-seal.
//! Non-guarantees: guest take-out; put-back; Prefab multi-child; Intent path.

use alchemy_protocol::inventory_spawn_key;
use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread;
use zenoh::Wait;

/// One accepted spawn request ready for the physics / seal path.
#[derive(Debug, Clone)]
pub struct SpawnRequest {
    /// Verified host Room Token (also used for world scene append).
    pub room_token: String,
    pub blake3: String,
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub qx: f64,
    pub qy: f64,
    pub qz: f64,
    pub qw: f64,
    pub sx: f64,
    pub sy: f64,
    pub sz: f64,
}

#[derive(Debug, Deserialize)]
struct SpawnPayloadJson {
    room_token: String,
    blake3: String,
    #[serde(default)]
    x: f64,
    #[serde(default = "default_y")]
    y: f64,
    #[serde(default = "default_z")]
    z: f64,
    #[serde(default)]
    qx: f64,
    #[serde(default)]
    qy: f64,
    #[serde(default)]
    qz: f64,
    #[serde(default = "default_qw")]
    qw: f64,
    #[serde(default = "default_one")]
    sx: f64,
    #[serde(default = "default_one")]
    sy: f64,
    #[serde(default = "default_one")]
    sz: f64,
}

fn default_y() -> f64 {
    1.5
}
fn default_z() -> f64 {
    2.0
}
fn default_qw() -> f64 {
    1.0
}
fn default_one() -> f64 {
    1.0
}

/// Channel of pending spawn requests (physics thread drains).
pub struct SpawnInbox {
    rx: Mutex<Receiver<SpawnRequest>>,
}

impl SpawnInbox {
    pub fn try_recv(&self) -> Option<SpawnRequest> {
        match self.rx.lock().ok()?.try_recv() {
            Ok(req) => Some(req),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => None,
        }
    }
}

fn is_blake3_hex(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

fn parse_payload(bytes: &[u8]) -> Result<SpawnPayloadJson> {
    let parsed: SpawnPayloadJson = serde_json::from_slice(bytes).context("spawn JSON")?;
    if parsed.room_token.trim().is_empty() {
        return Err(anyhow!("missing room_token"));
    }
    if !is_blake3_hex(&parsed.blake3) {
        return Err(anyhow!("invalid blake3"));
    }
    Ok(parsed)
}

/// Subscribe to inventory spawn; verify host role via `verify_host` before enqueue.
///
/// `verify_host(token) -> Result<()>` must check Room Token role=host + room match.
pub fn spawn_listener<F>(
    session: zenoh::Session,
    room_id: &str,
    wire_cap: &str,
    verify_host: F,
) -> Result<SpawnInbox>
where
    F: Fn(&str) -> Result<()> + Send + 'static,
{
    let key = inventory_spawn_key(room_id, wire_cap);
    let subscriber = session
        .declare_subscriber(key.clone())
        .wait()
        .map_err(|e| anyhow!("declare_subscriber inventory/spawn: {e}"))?;

    let (tx, rx): (Sender<SpawnRequest>, Receiver<SpawnRequest>) = mpsc::channel();
    let tx = Arc::new(Mutex::new(tx));

    thread::Builder::new()
        .name("inventory-spawn".into())
        .spawn(move || {
            let _session = session;
            log::info!("subscribed to {key} (x006d inventory spawn)");
            while let Ok(sample) = subscriber.recv() {
                let bytes = sample.payload().to_bytes();
                match handle_sample(bytes.as_ref(), &verify_host) {
                    Ok(req) => {
                        if let Ok(guard) = tx.lock() {
                            let _ = guard.send(req);
                        }
                    }
                    Err(err) => log::warn!("inventory spawn rejected: {err:#}"),
                }
            }
        })
        .context("spawn inventory-spawn thread")?;

    Ok(SpawnInbox {
        rx: Mutex::new(rx),
    })
}

fn handle_sample<F>(bytes: &[u8], verify_host: &F) -> Result<SpawnRequest>
where
    F: Fn(&str) -> Result<()>,
{
    let parsed = parse_payload(bytes)?;
    verify_host(parsed.room_token.trim())?;
    Ok(SpawnRequest {
        room_token: parsed.room_token.trim().to_string(),
        blake3: parsed.blake3.to_ascii_lowercase(),
        x: parsed.x,
        y: parsed.y,
        z: parsed.z,
        qx: parsed.qx,
        qy: parsed.qy,
        qz: parsed.qz,
        qw: parsed.qw,
        sx: parsed.sx,
        sy: parsed.sy,
        sz: parsed.sz,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_payload_requires_blake3_and_token() {
        let ok = br#"{"room_token":"tok","blake3":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","y":2.0}"#;
        let p = parse_payload(ok).unwrap();
        assert_eq!(p.y, 2.0);
        assert!(parse_payload(br#"{"room_token":"t","blake3":"short"}"#).is_err());
        assert!(parse_payload(br#"{"blake3":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}"#).is_err());
    }
}
