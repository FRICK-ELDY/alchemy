//! Minimal localhost Prometheus text metrics (x0049 / P3-4).
//!
//! Guarantees: optional `ALCHEMY_METRICS_ADDR` (loopback only) serves `GET /metrics`;
//! frame age + JWKS gauges/counters with fixed low-cardinality names.
//! Non-guarantees: OTLP; public scrape; histogram buckets; `sub`/`room_id` labels.

use std::io::{Read, Write};
use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// Process-wide client metrics (frame + JWKS).
#[derive(Debug, Default)]
pub struct ClientMetrics {
    last_frame_at: Mutex<Option<Instant>>,
    jwks_fetched_at: Mutex<Option<Instant>>,
    jwks_hard_stale_total: AtomicU64,
}

impl ClientMetrics {
    pub fn note_frame_accepted(&self) {
        if let Ok(mut slot) = self.last_frame_at.lock() {
            *slot = Some(Instant::now());
        }
    }

    /// 1 if at least one Frame was accepted; 0 otherwise (distinguishes from age=0).
    pub fn frame_seen(&self) -> u64 {
        self.last_frame_at
            .lock()
            .ok()
            .and_then(|g| g.as_ref().map(|_| 1))
            .unwrap_or(0)
    }

    /// Ms since last accept; 0 when never seen (pair with `frame_seen`).
    pub fn frame_age_ms(&self) -> u64 {
        self.last_frame_at
            .lock()
            .ok()
            .and_then(|g| g.map(|t| t.elapsed().as_millis() as u64))
            .unwrap_or(0)
    }

    /// Store last successful JWKS refresh instant (age computed live on scrape).
    pub fn set_jwks_fetched_at(&self, at: Instant) {
        if let Ok(mut slot) = self.jwks_fetched_at.lock() {
            *slot = Some(at);
        }
    }

    /// Live seconds since last JWKS refresh (0 if never).
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

    pub fn render_prometheus(&self) -> String {
        let mut out = String::with_capacity(640);
        out.push_str(
            "# HELP alchemy_client_frame_seen 1 after first accepted RenderFrame, else 0\n",
        );
        out.push_str("# TYPE alchemy_client_frame_seen gauge\n");
        out.push_str(&format!(
            "alchemy_client_frame_seen {}\n",
            self.frame_seen()
        ));
        out.push_str(
            "# HELP alchemy_client_frame_age_ms Ms since last accepted RenderFrame (0 if unseen)\n",
        );
        out.push_str("# TYPE alchemy_client_frame_age_ms gauge\n");
        out.push_str(&format!(
            "alchemy_client_frame_age_ms {}\n",
            self.frame_age_ms()
        ));
        out.push_str(
            "# HELP alchemy_jwks_cache_age_secs Seconds since last successful JWKS refresh\n",
        );
        out.push_str("# TYPE alchemy_jwks_cache_age_secs gauge\n");
        out.push_str(&format!(
            "alchemy_jwks_cache_age_secs{{service=\"client\"}} {}\n",
            self.jwks_cache_age_secs()
        ));
        out.push_str("# HELP alchemy_jwks_hard_stale_total JWKS hard-stale verify rejections\n");
        out.push_str("# TYPE alchemy_jwks_hard_stale_total counter\n");
        out.push_str(&format!(
            "alchemy_jwks_hard_stale_total{{service=\"client\"}} {}\n",
            self.jwks_hard_stale_total.load(Ordering::Relaxed)
        ));
        out
    }
}

/// Parse `ALCHEMY_METRICS_ADDR`; reject non-loopback. `None` path = caller skips spawn.
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

fn serve_one(stream: &mut TcpStream, metrics: &ClientMetrics) {
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

pub fn spawn_metrics_server(addr: SocketAddr, metrics: Arc<ClientMetrics>) {
    thread::Builder::new()
        .name("client-metrics".into())
        .spawn(move || {
            let listener = match TcpListener::bind(addr) {
                Ok(l) => l,
                Err(err) => {
                    log::error!("client metrics bind {addr} failed: {err}");
                    return;
                }
            };
            log::info!("client metrics listening on http://{addr}/metrics (x0049)");
            for stream in listener.incoming() {
                match stream {
                    Ok(mut stream) => serve_one(&mut stream, &metrics),
                    Err(err) => log::warn!("client metrics accept: {err}"),
                }
            }
        })
        .expect("spawn client-metrics");
}

pub fn maybe_spawn_from_env(metrics: Arc<ClientMetrics>) {
    match std::env::var("ALCHEMY_METRICS_ADDR") {
        Ok(raw) if !raw.is_empty() => match parse_metrics_addr(&raw) {
            Ok(addr) => spawn_metrics_server(addr, metrics),
            Err(err) => log::error!("client metrics disabled: {err}"),
        },
        _ => log::info!("client metrics off (set ALCHEMY_METRICS_ADDR=127.0.0.1:9102 to enable)"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_rejects_non_loopback() {
        assert!(parse_metrics_addr("0.0.0.0:9102").is_err());
        assert!(parse_metrics_addr("127.0.0.1:9102").is_ok());
    }

    #[test]
    fn frame_seen_distinguishes_unreceived() {
        let m = ClientMetrics::default();
        assert_eq!(m.frame_seen(), 0);
        assert_eq!(m.frame_age_ms(), 0);
        m.note_frame_accepted();
        assert_eq!(m.frame_seen(), 1);
        assert!(m.frame_age_ms() < 5_000);
    }

    #[test]
    fn jwks_age_is_live_from_instant() {
        let m = ClientMetrics::default();
        m.set_jwks_fetched_at(Instant::now() - Duration::from_secs(9));
        assert!(m.jwks_cache_age_secs() >= 9);
        let text = m.render_prometheus();
        assert!(text.contains("alchemy_jwks_cache_age_secs{service=\"client\"}"));
        assert!(text.contains("alchemy_client_frame_seen 0"));
    }

    #[test]
    fn render_and_hard_stale() {
        let m = ClientMetrics::default();
        m.note_frame_accepted();
        m.set_jwks_fetched_at(Instant::now());
        m.inc_jwks_hard_stale();
        let text = m.render_prometheus();
        assert!(text.contains("alchemy_client_frame_age_ms "));
        assert!(text.contains("alchemy_client_frame_seen 1"));
        assert!(text.contains("alchemy_jwks_hard_stale_total{service=\"client\"} 1"));
    }

    #[test]
    fn get_metrics_path_only() {
        assert!(is_get_metrics_request("GET /metrics HTTP/1.1\r\n"));
        assert!(!is_get_metrics_request("GET / HTTP/1.1\r\n"));
    }
}
