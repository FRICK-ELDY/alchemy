//! Launcher discovery endpoint (x0072 / P1-5).
//!
//! Guarantees: bind `127.0.0.1:0` for the launcher process lifetime; answer
//! authenticated `LOOKUP <token>` with current host generation + IPC addr;
//! [`DiscoveryHub::publish`] monotonically increases generation;
//! [`DiscoveryHub::invalidate`] clears IPC so LOOKUP returns `ERR` until the
//! next publish (avoids stale success during host restart).
//!
//! Non-guarantees: well-known fixed ports; multi-launcher collision; push/notify
//! to clients (clients pull on Exit Client+Host).

use alchemy_protocol::host_ipc::{
    authenticates_lookup, format_lookup_err, format_lookup_ok, is_loopback_addr,
};
use anyhow::{bail, Context, Result};
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

/// Shared discovery state published to clients.
#[derive(Debug, Clone, Default)]
pub struct DiscoveryState {
    pub gen: u64,
    pub ipc_addr: Option<String>,
}

/// Session-fixed discovery hub (loopback listener + mutable host IPC snapshot).
#[derive(Clone)]
pub struct DiscoveryHub {
    inner: Arc<Mutex<DiscoveryState>>,
    token: Arc<str>,
    /// Bound `127.0.0.1:port` string for client env.
    pub addr: String,
}

fn lock_state(inner: &Mutex<DiscoveryState>) -> MutexGuard<'_, DiscoveryState> {
    // Recover from poison: a prior panic in another holder must not diverge
    // publish / LOOKUP / current_ipc behavior.
    inner.lock().unwrap_or_else(PoisonError::into_inner)
}

impl DiscoveryHub {
    /// Bind discovery on `127.0.0.1:0` and spawn the accept loop.
    pub fn spawn(token: impl Into<String>) -> Result<Self> {
        let token: Arc<str> = Arc::from(token.into());
        let listener =
            TcpListener::bind("127.0.0.1:0").context("bind launcher discovery 127.0.0.1:0")?;
        let addr = listener
            .local_addr()
            .context("discovery local_addr")?
            .to_string();
        if !is_loopback_addr(&addr) {
            bail!("discovery bound non-loopback: {addr}");
        }
        let inner = Arc::new(Mutex::new(DiscoveryState::default()));
        let hub = Self {
            inner: Arc::clone(&inner),
            token: Arc::clone(&token),
            addr: addr.clone(),
        };
        let accept_hub = hub.clone();
        std::thread::Builder::new()
            .name("launcher-discovery".into())
            .spawn(move || accept_loop(listener, accept_hub))
            .context("spawn launcher-discovery thread")?;
        log::info!("launcher discovery listening on {addr}");
        Ok(hub)
    }

    /// Clear published IPC (LOOKUP → `ERR`) without bumping generation.
    /// Call at host restart start so clients do not SHUTDOWN a dead port.
    pub fn invalidate(&self) {
        let mut g = lock_state(&self.inner);
        if g.ipc_addr.take().is_some() {
            log::info!("discovery invalidated ipc (gen={} kept)", g.gen);
        }
    }

    /// Publish a new host IPC address; generation increases by 1.
    pub fn publish(&self, ipc_addr: &str) -> Result<()> {
        if !is_loopback_addr(ipc_addr) {
            bail!("discovery publish refused (not loopback): {ipc_addr}");
        }
        let mut g = lock_state(&self.inner);
        g.gen = g.gen.saturating_add(1);
        g.ipc_addr = Some(ipc_addr.to_string());
        log::info!("discovery published gen={} ipc={}", g.gen, ipc_addr);
        Ok(())
    }

    /// Snapshot for launcher-local Ctrl+C shutdown (same state clients LOOKUP).
    pub fn current_ipc(&self) -> Option<String> {
        lock_state(&self.inner).ipc_addr.clone()
    }

    #[cfg(test)]
    fn snapshot(&self) -> DiscoveryState {
        lock_state(&self.inner).clone()
    }
}

fn accept_loop(listener: TcpListener, hub: DiscoveryHub) {
    for conn in listener.incoming() {
        match conn {
            Ok(stream) => {
                // Handle each LOOKUP on its own thread so a slow/silent peer
                // cannot block the next Exit Client+Host for the full read timeout.
                let hub = hub.clone();
                if let Err(err) = std::thread::Builder::new()
                    .name("launcher-discovery-conn".into())
                    .spawn(move || {
                        if let Err(err) = handle_client(stream, &hub) {
                            log::warn!("discovery client: {err:#}");
                        }
                    })
                {
                    log::warn!("discovery conn spawn failed: {err}");
                }
            }
            Err(err) => log::warn!("discovery accept: {err}"),
        }
    }
}

fn handle_client(stream: TcpStream, hub: &DiscoveryHub) -> Result<()> {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
    let mut reader = BufReader::new(&stream);
    let mut line = String::new();
    reader
        .read_line(&mut line)
        .context("discovery read LOOKUP")?;
    let reply = if authenticates_lookup(&line, &hub.token) {
        let g = lock_state(&hub.inner);
        match &g.ipc_addr {
            Some(ipc) => format_lookup_ok(g.gen, ipc),
            None => format_lookup_err(),
        }
    } else {
        log::warn!("discovery: ignored unauthenticated or malformed LOOKUP");
        format_lookup_err()
    };
    let mut writer = stream;
    writer
        .write_all(reply.as_bytes())
        .context("discovery write reply")?;
    writer.flush().ok();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use alchemy_protocol::host_ipc::{request_lookup, LookupOk};
    use std::thread;
    use std::time::{Duration, Instant};

    fn wait_until_lookup_ok(hub: &DiscoveryHub, token: &str) -> LookupOk {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            match request_lookup(&hub.addr, token) {
                Ok(ok) => return ok,
                Err(_) if Instant::now() < deadline => {
                    thread::sleep(Duration::from_millis(5));
                }
                Err(err) => panic!("lookup did not become OK: {err}"),
            }
        }
    }

    /// Wait until discovery answers the protocol `ERR` line (connect must succeed).
    fn wait_until_lookup_err_reply(hub: &DiscoveryHub, token: &str) {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            match request_lookup(&hub.addr, token) {
                Err(err) if err.is_err_reply() => return,
                Ok(_) if Instant::now() < deadline => {
                    thread::sleep(Duration::from_millis(5));
                }
                Err(_) if Instant::now() < deadline => {
                    // Connect / accept race — retry until we see a protocol ERR.
                    thread::sleep(Duration::from_millis(5));
                }
                Ok(ok) => panic!("expected protocol ERR, got {ok:?}"),
                Err(err) => panic!("expected protocol ERR, got non-reply error: {err}"),
            }
        }
    }

    #[test]
    fn publish_bumps_gen_and_lookup_returns_ipc() {
        let hub = DiscoveryHub::spawn("secret-tok").expect("spawn discovery");
        assert!(hub.snapshot().ipc_addr.is_none());
        wait_until_lookup_err_reply(&hub, "secret-tok");

        hub.publish("127.0.0.1:41001").unwrap();
        assert_eq!(hub.snapshot().gen, 1);
        hub.publish("127.0.0.1:41002").unwrap();
        assert_eq!(hub.snapshot().gen, 2);
        assert_eq!(hub.current_ipc().as_deref(), Some("127.0.0.1:41002"));

        let ok = wait_until_lookup_ok(&hub, "secret-tok");
        assert_eq!(
            ok,
            LookupOk {
                gen: 2,
                ipc_addr: "127.0.0.1:41002".into(),
            }
        );
    }

    #[test]
    fn invalidate_clears_ipc_without_bumping_gen() {
        let hub = DiscoveryHub::spawn("tok").expect("spawn");
        hub.publish("127.0.0.1:41010").unwrap();
        assert_eq!(hub.snapshot().gen, 1);
        wait_until_lookup_ok(&hub, "tok");

        hub.invalidate();
        assert_eq!(hub.snapshot().gen, 1);
        assert!(hub.snapshot().ipc_addr.is_none());
        wait_until_lookup_err_reply(&hub, "tok");

        hub.publish("127.0.0.1:41011").unwrap();
        let ok = wait_until_lookup_ok(&hub, "tok");
        assert_eq!(ok.gen, 2);
        assert_eq!(ok.ipc_addr, "127.0.0.1:41011");
    }

    #[test]
    fn bad_token_and_not_ready_return_err() {
        let hub = DiscoveryHub::spawn("good").expect("spawn discovery");
        wait_until_lookup_err_reply(&hub, "good");
        hub.publish("127.0.0.1:41003").unwrap();
        let bad = request_lookup(&hub.addr, "bad");
        assert!(
            bad.as_ref().err().is_some_and(|e| e.is_err_reply()),
            "bad token must be protocol ERR, got {bad:?}"
        );
        assert!(wait_until_lookup_ok(&hub, "good").ipc_addr == "127.0.0.1:41003");
    }

    #[test]
    fn publish_rejects_non_loopback() {
        let hub = DiscoveryHub::spawn("tok").expect("spawn");
        assert!(hub.publish("192.168.1.1:9").is_err());
        assert_eq!(hub.snapshot().gen, 0);
    }
}
