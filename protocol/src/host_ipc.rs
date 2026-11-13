//! Localhost TCP host shutdown + auth broker + ready-file helpers
//! (x0032; x009f AUTH_PUSH; discovery LOOKUP retired).
//!
//! Not a Zenoh wire schema — shared line format / client request / ready-file
//! parsing so host and client (`HostSupervisor`) stay in sync.
//!
//! Host IPC lines on TCP to the host listener:
//! - `SHUTDOWN <token>\n`
//! - `AUTH_PUSH <token> <expires_in> <access_jwt>\n` (x009f parent credential broker)
//!
//! Ready file (optional second line): `ipc=127.0.0.1:<port>`.
//! Legacy discovery LOOKUP helpers remain for protocol tests / transitional callers.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::time::Duration;

/// Env: shared secret; when set, host binds `127.0.0.1:0` for IPC.
pub const ENV_TOKEN: &str = "ALCHEMY_HOST_IPC_TOKEN";

/// Env: loopback `host:port` for clients (launcher fills after reading ready file).
pub const ENV_ADDR: &str = "ALCHEMY_HOST_IPC_ADDR";

/// Env: launcher discovery loopback `host:port` (session-fixed; x0072).
pub const ENV_DISCOVERY_ADDR: &str = "ALCHEMY_LAUNCHER_DISCOVERY_ADDR";

/// Env: when `1`, host treats `ALCHEMY_AUTH_TOKEN` as parent-brokered (x009f).
pub const ENV_AUTH_BROKER: &str = "ALCHEMY_AUTH_BROKER";

/// Env: remaining access JWT TTL seconds for ParentBroker bootstrap (x009f).
pub const ENV_AUTH_EXPIRES_IN: &str = "ALCHEMY_AUTH_EXPIRES_IN";

/// Ready-file line prefix announcing the bound IPC address.
pub const READY_IPC_PREFIX: &str = "ipc=";

/// Format the shutdown request line (includes trailing newline).
pub fn shutdown_line(token: &str) -> String {
    format!("SHUTDOWN {token}\n")
}

/// Format parent→host access JWT push (x009f). `expires_in` is remaining seconds (≥1).
///
/// `access_jwt` must not contain whitespace (JWT compact form).
pub fn auth_push_line(token: &str, expires_in: u64, access_jwt: &str) -> String {
    format!("AUTH_PUSH {token} {expires_in} {access_jwt}\n")
}

/// Format the discovery LOOKUP request line (includes trailing newline).
pub fn lookup_line(token: &str) -> String {
    format!("LOOKUP {token}\n")
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

/// Parsed `AUTH_PUSH` payload after IPC token authentication (x009f).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthPush {
    pub expires_in: u64,
    pub access_jwt: String,
}

/// Parse `AUTH_PUSH <token> <expires_in> <access_jwt>`; `None` when malformed or token mismatch.
pub fn parse_auth_push(line: &str, expected_token: &str) -> Option<AuthPush> {
    let line = line.trim();
    let rest = line.strip_prefix("AUTH_PUSH")?;
    let mut parts = rest.split_whitespace();
    let got = parts.next()?;
    if got.is_empty() || got != expected_token {
        return None;
    }
    let expires_in: u64 = parts.next()?.parse().ok()?;
    if expires_in == 0 {
        return None;
    }
    let access_jwt = parts.next()?;
    if access_jwt.is_empty() || parts.next().is_some() {
        return None;
    }
    Some(AuthPush {
        expires_in,
        access_jwt: access_jwt.to_string(),
    })
}

/// Parse a single discovery request line; true only when LOOKUP and token matches.
pub fn authenticates_lookup(line: &str, expected_token: &str) -> bool {
    let line = line.trim();
    let Some(rest) = line.strip_prefix("LOOKUP") else {
        return false;
    };
    let got = rest.trim();
    !got.is_empty() && got == expected_token
}

/// Format a successful discovery reply (`OK gen=… ipc=…\n`).
pub fn format_lookup_ok(gen: u64, ipc_addr: &str) -> String {
    format!("OK gen={gen} ipc={ipc_addr}\n")
}

/// Error reply for discovery (bad token / not ready).
pub fn format_lookup_err() -> String {
    "ERR\n".to_string()
}

/// Parsed discovery OK reply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LookupOk {
    pub gen: u64,
    pub ipc_addr: String,
}

/// LOOKUP failure after (or before) talking to discovery.
#[derive(Debug)]
pub enum LookupError {
    /// Server replied `ERR\n` (not ready / bad token) — connect succeeded.
    ErrReply,
    /// TCP / read / write failure (includes accept-not-ready races).
    Io(std::io::Error),
    /// Connected but reply was neither OK nor ERR.
    InvalidReply(String),
}

impl std::fmt::Display for LookupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ErrReply => write!(f, "discovery returned ERR"),
            Self::Io(err) => write!(f, "{err}"),
            Self::InvalidReply(line) => write!(f, "discovery reply not OK: {line:?}"),
        }
    }
}

impl std::error::Error for LookupError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(err) => Some(err),
            Self::ErrReply | Self::InvalidReply(_) => None,
        }
    }
}

impl LookupError {
    /// True when the server answered the protocol `ERR` line (not a connect failure).
    pub fn is_err_reply(&self) -> bool {
        matches!(self, Self::ErrReply)
    }
}

/// Parse `OK gen=<u64> ipc=<addr>` (with or without trailing newline).
/// `ipc` must be loopback ([`is_loopback_addr`]); non-loopback → `None`.
pub fn parse_lookup_ok(line: &str) -> Option<LookupOk> {
    let line = line.trim();
    let rest = line.strip_prefix("OK")?;
    let rest = rest.trim();
    let mut gen: Option<u64> = None;
    let mut ipc_addr: Option<String> = None;
    for part in rest.split_whitespace() {
        if let Some(g) = part.strip_prefix("gen=") {
            gen = g.parse().ok();
        } else if let Some(a) = part.strip_prefix("ipc=") {
            let a = a.trim();
            if !a.is_empty() {
                ipc_addr = Some(a.to_string());
            }
        }
    }
    let ipc_addr = ipc_addr?;
    if !is_loopback_addr(&ipc_addr) {
        return None;
    }
    Some(LookupOk {
        gen: gen?,
        ipc_addr,
    })
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

/// Connect, send [`auth_push_line`], expect `OK\n` (x009f). Caller should enforce loopback.
pub fn request_auth_push(
    addr: &str,
    token: &str,
    expires_in: u64,
    access_jwt: &str,
) -> Result<(), std::io::Error> {
    let mut stream = TcpStream::connect(addr)?;
    stream.set_write_timeout(Some(Duration::from_secs(2))).ok();
    stream.set_read_timeout(Some(Duration::from_secs(2))).ok();
    stream.write_all(auth_push_line(token, expires_in, access_jwt).as_bytes())?;
    stream.flush().ok();
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    if line.trim() == "OK" {
        Ok(())
    } else {
        Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("auth push reply not OK: {line:?}"),
        ))
    }
}

/// Connect to launcher discovery, send LOOKUP, parse OK. Caller should enforce loopback.
pub fn request_lookup(discovery_addr: &str, token: &str) -> Result<LookupOk, LookupError> {
    let mut stream = TcpStream::connect(discovery_addr).map_err(LookupError::Io)?;
    stream.set_write_timeout(Some(Duration::from_secs(2))).ok();
    stream.set_read_timeout(Some(Duration::from_secs(2))).ok();
    stream
        .write_all(lookup_line(token).as_bytes())
        .map_err(LookupError::Io)?;
    stream.flush().ok();
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).map_err(LookupError::Io)?;
    if line.trim() == "ERR" {
        return Err(LookupError::ErrReply);
    }
    parse_lookup_ok(&line).ok_or_else(|| LookupError::InvalidReply(line))
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
    fn auth_push_line_roundtrip() {
        let line = auth_push_line("tok", 900, "hdr.payload.sig");
        assert_eq!(
            parse_auth_push(&line, "tok"),
            Some(AuthPush {
                expires_in: 900,
                access_jwt: "hdr.payload.sig".into(),
            })
        );
        assert!(parse_auth_push(&line, "other").is_none());
        assert!(parse_auth_push("AUTH_PUSH tok 0 jwt\n", "tok").is_none());
        assert!(parse_auth_push("AUTH_PUSH tok 9\n", "tok").is_none());
        assert!(parse_auth_push("SHUTDOWN tok\n", "tok").is_none());
    }

    #[test]
    fn lookup_line_roundtrip_auth() {
        let line = lookup_line("tok");
        assert!(authenticates_lookup(&line, "tok"));
        assert!(!authenticates_lookup(&line, "other"));
        assert!(!authenticates_lookup("SHUTDOWN tok\n", "tok"));
        assert!(!authenticates_lookup("LOOKUP\n", "tok"));
    }

    #[test]
    fn lookup_ok_roundtrip() {
        let line = format_lookup_ok(3, "127.0.0.1:45555");
        assert_eq!(
            parse_lookup_ok(&line),
            Some(LookupOk {
                gen: 3,
                ipc_addr: "127.0.0.1:45555".into(),
            })
        );
        assert!(parse_lookup_ok("ERR\n").is_none());
        assert!(parse_lookup_ok("OK ipc=127.0.0.1:1\n").is_none());
        assert!(parse_lookup_ok("OK gen=1\n").is_none());
        assert!(parse_lookup_ok("OK gen=1 ipc=192.168.0.1:9\n").is_none());
    }

    #[test]
    fn lookup_error_err_reply_flag() {
        assert!(LookupError::ErrReply.is_err_reply());
        assert!(!LookupError::InvalidReply("x".into()).is_err_reply());
        assert!(!LookupError::Io(std::io::Error::other("connect")).is_err_reply());
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
