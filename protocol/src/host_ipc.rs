//! Localhost TCP host shutdown helpers (x0032 launcher IPC).
//!
//! Not a Zenoh wire schema — shared line format / client request / ready-file
//! parsing so host, client, and launcher stay in sync.
//!
//! Protocol: one line `SHUTDOWN <token>\n` on TCP to the host listener.
//! Ready file (optional second line): `ipc=127.0.0.1:<port>`.

use std::io::Write;
use std::net::TcpStream;
use std::time::Duration;

/// Env: shared secret; when set, host binds `127.0.0.1:0` for IPC.
pub const ENV_TOKEN: &str = "ALCHEMY_HOST_IPC_TOKEN";

/// Env: loopback `host:port` for clients (launcher fills after reading ready file).
pub const ENV_ADDR: &str = "ALCHEMY_HOST_IPC_ADDR";

/// Ready-file line prefix announcing the bound IPC address.
pub const READY_IPC_PREFIX: &str = "ipc=";

/// Format the shutdown request line (includes trailing newline).
pub fn shutdown_line(token: &str) -> String {
    format!("SHUTDOWN {token}\n")
}

/// True when `addr` is loopback host:port (`127.0.0.1:` or `localhost:`).
pub fn is_loopback_addr(addr: &str) -> bool {
    let addr = addr.trim();
    addr.starts_with("127.0.0.1:") || addr.starts_with("localhost:")
}

/// Parse a single request line; true only when command is SHUTDOWN and token matches.
pub fn authenticates_shutdown(line: &str, expected_token: &str) -> bool {
    let line = line.trim();
    let Some(rest) = line.strip_prefix("SHUTDOWN") else {
        return false;
    };
    let got = rest.trim();
    !got.is_empty() && got == expected_token
}

/// Connect and send [`shutdown_line`]. Caller should enforce [`is_loopback_addr`].
pub fn request_shutdown(addr: &str, token: &str) -> Result<(), std::io::Error> {
    let mut stream = TcpStream::connect(addr)?;
    stream.set_write_timeout(Some(Duration::from_secs(2))).ok();
    stream.set_read_timeout(Some(Duration::from_secs(2))).ok();
    stream.write_all(shutdown_line(token).as_bytes())?;
    stream.flush().ok();
    Ok(())
}

/// Build ready-file body: `ready\n` plus optional `ipc=…\n`.
pub fn ready_file_body(ipc_addr: Option<&str>) -> String {
    match ipc_addr {
        Some(addr) if !addr.is_empty() => format!("ready\n{READY_IPC_PREFIX}{addr}\n"),
        _ => "ready\n".to_string(),
    }
}

/// Extract `ipc=host:port` from a ready-file body, if present.
pub fn parse_ready_ipc_addr(body: &str) -> Option<String> {
    for line in body.lines() {
        let line = line.trim();
        if let Some(addr) = line.strip_prefix(READY_IPC_PREFIX) {
            let addr = addr.trim();
            if !addr.is_empty() {
                return Some(addr.to_string());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shutdown_line_roundtrip_auth() {
        let line = shutdown_line("tok");
        assert!(authenticates_shutdown(&line, "tok"));
        assert!(!authenticates_shutdown(&line, "other"));
        assert!(!authenticates_shutdown("PING tok\n", "tok"));
    }

    #[test]
    fn loopback_check() {
        assert!(is_loopback_addr("127.0.0.1:9"));
        assert!(is_loopback_addr("localhost:9"));
        assert!(!is_loopback_addr("192.168.0.1:9"));
        assert!(!is_loopback_addr(""));
    }

    #[test]
    fn ready_file_ipc_roundtrip() {
        let body = ready_file_body(Some("127.0.0.1:45555"));
        assert!(body.starts_with("ready\n"));
        assert_eq!(
            parse_ready_ipc_addr(&body).as_deref(),
            Some("127.0.0.1:45555")
        );
        assert!(parse_ready_ipc_addr("ready\n").is_none());
    }
}
