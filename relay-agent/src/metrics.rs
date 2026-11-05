//! Minimal localhost Prometheus text metrics (x007a / P2-12).
//!
//! Guarantees: optional `RELAY_AGENT_METRICS_ADDR` or `RELAY_AGENT_METRICS=1`
//! (default `127.0.0.1:9106`) serves `GET /metrics` on loopback only.
//! Non-guarantees: OTLP; public scrape; `jti`/`sub`/`room_id` labels.

use std::io::{Read, Write};
use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

const DEFAULT_ADDR: &str = "127.0.0.1:9106";

/// Process-wide allocate counters.
#[derive(Debug, Default)]
pub struct RelayMetrics {
    allocate_total: AtomicU64,
    allocate_errors_total: AtomicU64,
}

impl RelayMetrics {
    pub fn inc_allocate_ok(&self) {
        self.allocate_total.fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_allocate_err(&self) {
        self.allocate_errors_total.fetch_add(1, Ordering::Relaxed);
    }

    pub fn render_prometheus(&self) -> String {
        let mut out = String::with_capacity(256);
        out.push_str("# HELP alchemy_relay_allocate_total Allocate successes\n");
        out.push_str("# TYPE alchemy_relay_allocate_total counter\n");
        out.push_str(&format!(
            "alchemy_relay_allocate_total {}\n",
            self.allocate_total.load(Ordering::Relaxed)
        ));
        out.push_str("# HELP alchemy_relay_allocate_errors_total Allocate failures\n");
        out.push_str("# TYPE alchemy_relay_allocate_errors_total counter\n");
        out.push_str(&format!(
            "alchemy_relay_allocate_errors_total {}\n",
            self.allocate_errors_total.load(Ordering::Relaxed)
        ));
        out
    }
}

/// Parse metrics bind addr; reject non-loopback.
pub fn parse_metrics_addr(raw: &str) -> Result<SocketAddr, String> {
    let addr: SocketAddr = raw
        .parse()
        .map_err(|e| format!("invalid RELAY_AGENT_METRICS_ADDR={raw:?}: {e}"))?;
    if !is_loopback_ip(addr.ip()) {
        return Err(format!(
            "RELAY_AGENT_METRICS_ADDR must be loopback (got {})",
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

fn serve_one(stream: &mut TcpStream, metrics: &RelayMetrics) {
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

pub fn spawn_metrics_server(addr: SocketAddr, metrics: Arc<RelayMetrics>) {
    thread::Builder::new()
        .name("relay-metrics".into())
        .spawn(move || {
            let listener = match TcpListener::bind(addr) {
                Ok(l) => l,
                Err(err) => {
                    log::error!("relay metrics bind {addr} failed: {err}");
                    return;
                }
            };
            log::info!("relay metrics listening on http://{addr}/metrics (x007a)");
            for stream in listener.incoming() {
                match stream {
                    Ok(mut stream) => serve_one(&mut stream, &metrics),
                    Err(err) => log::warn!("relay metrics accept: {err}"),
                }
            }
        })
        .expect("spawn relay-metrics");
}

/// Start metrics if `RELAY_AGENT_METRICS_ADDR` or `RELAY_AGENT_METRICS=1`.
pub fn maybe_spawn_from_env(metrics: Arc<RelayMetrics>) {
    let raw = match std::env::var("RELAY_AGENT_METRICS_ADDR") {
        Ok(v) if !v.is_empty() => Some(v),
        _ => match std::env::var("RELAY_AGENT_METRICS") {
            Ok(v) if v == "1" || v.eq_ignore_ascii_case("true") => Some(DEFAULT_ADDR.to_string()),
            _ => None,
        },
    };
    match raw {
        Some(addr_raw) => match parse_metrics_addr(&addr_raw) {
            Ok(addr) => spawn_metrics_server(addr, metrics),
            Err(err) => log::error!("relay metrics disabled: {err}"),
        },
        None => log::info!(
            "relay metrics off (set RELAY_AGENT_METRICS_ADDR=127.0.0.1:9106 or RELAY_AGENT_METRICS=1)"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_rejects_non_loopback() {
        assert!(parse_metrics_addr("0.0.0.0:9106").is_err());
        assert!(parse_metrics_addr("192.168.1.1:9106").is_err());
        assert!(parse_metrics_addr("127.0.0.1:9106").is_ok());
        assert!(parse_metrics_addr("[::1]:9106").is_ok());
    }

    #[test]
    fn render_contains_required_names() {
        let m = RelayMetrics::default();
        m.inc_allocate_ok();
        m.inc_allocate_err();
        let text = m.render_prometheus();
        assert!(text.contains("alchemy_relay_allocate_total 1"));
        assert!(text.contains("alchemy_relay_allocate_errors_total 1"));
    }

    #[test]
    fn get_metrics_path_only() {
        assert!(is_get_metrics_request("GET /metrics HTTP/1.1\r\n"));
        assert!(is_get_metrics_request("GET /metrics?x=1 HTTP/1.1\r\n"));
        assert!(!is_get_metrics_request("GET /health HTTP/1.1\r\n"));
        assert!(!is_get_metrics_request("POST /metrics HTTP/1.1\r\n"));
    }
}
