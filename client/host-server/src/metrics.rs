//! Minimal localhost Prometheus text metrics (x0049 / P3-4).
//!
//! Guarantees: optional `ALCHEMY_METRICS_ADDR` (loopback only) serves `GET /metrics`;
//! tick lateness + JWKS gauges/counters with fixed low-cardinality names.
//! Non-guarantees: OTLP; public scrape; histogram buckets; `sub`/`room_id` labels.

use std::io::{Read, Write};
use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// Ticks late by more than this many ms count toward `alchemy_host_tick_late_total`.
/// 60Hz period ≈16.7ms; 5ms ignores sleep-wake jitter while flagging real schedule slip.
pub const TICK_LATE_THRESHOLD_MS: u64 = 5;

/// Process-wide host metrics (physics + JWKS snapshots).
#[derive(Debug, Default)]
pub struct HostMetrics {
    tick_lateness_ms: AtomicU64,
    tick_late_total: AtomicU64,
    jwks_fetched_at: Mutex<Option<Instant>>,
    jwks_hard_stale_total: AtomicU64,
}

impl HostMetrics {
    pub fn set_tick_lateness_ms(&self, ms: u64) {
        self.tick_lateness_ms.store(ms, Ordering::Relaxed);
        if ms > TICK_LATE_THRESHOLD_MS {
            self.tick_late_total.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Last successful JWKS refresh instant (age computed live on scrape).
    pub fn set_jwks_fetched_at(&self, at: Instant) {
        if let Ok(mut slot) = self.jwks_fetched_at.lock() {
            *slot = Some(at);
        }
    }

    pub fn jwks_cache_age_secs(&self) -> u64 {
        self.jwks_fetched_at
            .lock()
            .ok()
            .and_then(|g| g.map(|t| t.elapsed().as_secs()))
            .unwrap_or(0)
    }

    pub fn inc_jwks_hard_stale(&self) {
        self.jwks_hard_stale_total.fetch_add(1, Ordering::Relaxed);
    }

    #[cfg(test)]
    pub fn jwks_hard_stale_total(&self) -> u64 {
        self.jwks_hard_stale_total.load(Ordering::Relaxed)
    }

    #[cfg(test)]
    pub fn tick_late_total(&self) -> u64 {
        self.tick_late_total.load(Ordering::Relaxed)
    }

    pub fn render_prometheus(&self) -> String {
        let mut out = String::with_capacity(512);
        out.push_str(
            "# HELP alchemy_host_tick_lateness_ms Physics tick lateness vs schedule (ms)\n",
        );
        out.push_str("# TYPE alchemy_host_tick_lateness_ms gauge\n");
        out.push_str(&format!(
            "alchemy_host_tick_lateness_ms {}\n",
            self.tick_lateness_ms.load(Ordering::Relaxed)
        ));
        out.push_str(&format!(
            "# HELP alchemy_host_tick_late_total Physics ticks late by more than {TICK_LATE_THRESHOLD_MS}ms\n"
        ));
        out.push_str("# TYPE alchemy_host_tick_late_total counter\n");
        out.push_str(&format!(
            "alchemy_host_tick_late_total {}\n",
            self.tick_late_total.load(Ordering::Relaxed)
        ));
        out.push_str(
            "# HELP alchemy_jwks_cache_age_secs Seconds since last successful JWKS refresh\n",
        );
        out.push_str("# TYPE alchemy_jwks_cache_age_secs gauge\n");
        out.push_str(&format!(
            "alchemy_jwks_cache_age_secs{{service=\"host\"}} {}\n",
            self.jwks_cache_age_secs()
        ));
        out.push_str("# HELP alchemy_jwks_hard_stale_total JWKS hard-stale verify rejections\n");
        out.push_str("# TYPE alchemy_jwks_hard_stale_total counter\n");
        out.push_str(&format!(
            "alchemy_jwks_hard_stale_total{{service=\"host\"}} {}\n",
            self.jwks_hard_stale_total.load(Ordering::Relaxed)
        ));
        out
    }
}

/// Parse `ALCHEMY_METRICS_ADDR`; reject non-loopback. `None` = metrics off.
pub fn parse_metrics_addr(raw: &str) -> Result<SocketAddr, String> {
    let addr: SocketAddr = raw
        .parse()
        .map_err(|e| format!("invalid ALCHEMY_METRICS_ADDR={raw:?}: {e}"))?;
    if !is_loopback_ip(addr.ip()) {
        return Err(format!(
            "ALCHEMY_METRICS_ADDR must be loopback (got {})",
            addr.ip()
        ));
    }
    Ok(addr)
}

fn is_loopback_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_loopback(),
        IpAddr::V6(v6) => v6.is_loopback(),
    }
}

fn is_get_metrics_request(head: &str) -> bool {
    let line = head.lines().next().unwrap_or("").trim();
    let mut parts = line.split_whitespace();
    match (parts.next(), parts.next()) {
        (Some("GET"), Some(path)) => path == "/metrics" || path.starts_with("/metrics?"),
        _ => false,
    }
}

fn write_http(stream: &mut TcpStream, status: &str, body: &str) {
    let resp = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/plain; version=0.0.4; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(resp.as_bytes());
}

fn serve_one(stream: &mut TcpStream, metrics: &HostMetrics) {
    let Ok(peer) = stream.peer_addr() else {
        let _ = stream.shutdown(std::net::Shutdown::Both);
        return;
    };
    if !is_loopback_ip(peer.ip()) {
        let _ = stream.shutdown(std::net::Shutdown::Both);
        return;
    }
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let mut buf = [0u8; 1024];
    let n = stream.read(&mut buf).unwrap_or(0);
    let head = std::str::from_utf8(&buf[..n]).unwrap_or("");
    if !is_get_metrics_request(head) {
        write_http(stream, "404 Not Found", "not found\n");
        return;
    }
    write_http(stream, "200 OK", &metrics.render_prometheus());
}

/// Spawn a daemon thread serving `GET /metrics` on `addr` (must be loopback).
pub fn spawn_metrics_server(addr: SocketAddr, metrics: Arc<HostMetrics>) {
    thread::Builder::new()
        .name("host-metrics".into())
        .spawn(move || {
            let listener = match TcpListener::bind(addr) {
                Ok(l) => l,
                Err(err) => {
                    log::error!("host metrics bind {addr} failed: {err}");
                    return;
                }
            };
            log::info!("host metrics listening on http://{addr}/metrics (x0049)");
            for stream in listener.incoming() {
                match stream {
                    Ok(mut stream) => serve_one(&mut stream, &metrics),
                    Err(err) => log::warn!("host metrics accept: {err}"),
                }
            }
        })
        .expect("spawn host-metrics");
}

/// Start metrics if `ALCHEMY_METRICS_ADDR` is set.
pub fn maybe_spawn_from_env(metrics: Arc<HostMetrics>) {
    match std::env::var("ALCHEMY_METRICS_ADDR") {
        Ok(raw) if !raw.is_empty() => match parse_metrics_addr(&raw) {
            Ok(addr) => spawn_metrics_server(addr, metrics),
            Err(err) => log::error!("host metrics disabled: {err}"),
        },
        _ => log::info!("host metrics off (set ALCHEMY_METRICS_ADDR=127.0.0.1:9101 to enable)"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_rejects_non_loopback() {
        assert!(parse_metrics_addr("0.0.0.0:9101").is_err());
        assert!(parse_metrics_addr("192.168.1.1:9101").is_err());
        assert!(parse_metrics_addr("127.0.0.1:9101").is_ok());
        assert!(parse_metrics_addr("[::1]:9101").is_ok());
    }

    #[test]
    fn late_total_uses_threshold() {
        let m = HostMetrics::default();
        m.set_tick_lateness_ms(TICK_LATE_THRESHOLD_MS);
        assert_eq!(m.tick_late_total(), 0);
        m.set_tick_lateness_ms(TICK_LATE_THRESHOLD_MS + 1);
        assert_eq!(m.tick_late_total(), 1);
    }

    #[test]
    fn render_contains_required_names() {
        let m = HostMetrics::default();
        m.set_tick_lateness_ms(TICK_LATE_THRESHOLD_MS + 2);
        m.set_jwks_fetched_at(Instant::now() - Duration::from_secs(12));
        m.inc_jwks_hard_stale();
        let text = m.render_prometheus();
        assert!(text.contains(&format!(
            "alchemy_host_tick_lateness_ms {}",
            TICK_LATE_THRESHOLD_MS + 2
        )));
        assert!(text.contains(&format!(
            "# HELP alchemy_host_tick_late_total Physics ticks late by more than {TICK_LATE_THRESHOLD_MS}ms"
        )));
        assert!(text.contains("alchemy_host_tick_late_total 1"));
        assert!(text.contains("alchemy_jwks_cache_age_secs{service=\"host\"}"));
        assert!(m.jwks_cache_age_secs() >= 12);
        assert!(text.contains("alchemy_jwks_hard_stale_total{service=\"host\"} 1"));
    }

    #[test]
    fn get_metrics_path_only() {
        assert!(is_get_metrics_request("GET /metrics HTTP/1.1\r\n"));
        assert!(is_get_metrics_request("GET /metrics?x=1 HTTP/1.1\r\n"));
        assert!(!is_get_metrics_request("GET /health HTTP/1.1\r\n"));
        assert!(!is_get_metrics_request("POST /metrics HTTP/1.1\r\n"));
    }
}
