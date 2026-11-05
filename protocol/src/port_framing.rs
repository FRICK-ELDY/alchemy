//! Length-prefixed Port framing for Elixir `{:packet, 4}` workers (x0075).
//!
//! # Guarantees
//! - u32 big-endian length prefix, then that many payload bytes (Erlang Port packet 4).
//! - Soft oversize / empty responses stay process-alive when callers use [`ServeReply`].
//!
//! # Non-guarantees
//! - Concurrent clients on one stdin; request multiplexing; streaming hash API.

use std::io::{self, Read, Write};

/// Soft-error response prefix (`!` is never a hex digit; success is exactly 64 hex).
pub const SOFT_ERR_PREFIX: u8 = b'!';

#[derive(Debug)]
pub enum FramingError {
    Io(io::Error),
    /// Declared length exceeds the configured ceiling (hard: stop reading this peer).
    Oversize { len: u32, max: u32 },
}

impl From<io::Error> for FramingError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

impl std::fmt::Display for FramingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "io: {e}"),
            Self::Oversize { len, max } => write!(f, "oversize len={len} max={max}"),
        }
    }
}

/// Success = 64-byte ASCII hex; soft error = `e` + reason (Port stays alive).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServeReply {
    Hex([u8; 64]),
    SoftError(String),
}

impl ServeReply {
    pub fn ok_hex(hex: &str) -> Result<Self, String> {
        let down = hex.trim().to_ascii_lowercase();
        if down.len() != 64 || !down.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(format!("expected 64 hex chars, got len={}", down.len()));
        }
        let mut buf = [0u8; 64];
        buf.copy_from_slice(down.as_bytes());
        Ok(Self::Hex(buf))
    }

    pub fn soft_error(reason: impl Into<String>) -> Self {
        let msg = reason.into();
        // Keep packet small for Port callers (char boundary safe).
        let msg = match msg.char_indices().nth(200) {
            Some((idx, _)) => msg[..idx].to_string(),
            None => msg,
        };
        Self::SoftError(msg)
    }

    pub fn as_bytes(&self) -> Vec<u8> {
        match self {
            Self::Hex(h) => h.to_vec(),
            Self::SoftError(msg) => {
                let mut out = Vec::with_capacity(1 + msg.len());
                out.push(SOFT_ERR_PREFIX);
                out.extend_from_slice(msg.as_bytes());
                out
            }
        }
    }
}

/// Read one `{:packet, 4}` frame. `Ok(None)` on clean EOF before a length header.
pub fn read_packet<R: Read>(reader: &mut R, max_len: u32) -> Result<Option<Vec<u8>>, FramingError> {
    let mut len_buf = [0u8; 4];
    match reader.read_exact(&mut len_buf) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(FramingError::Io(e)),
    }
    let len = u32::from_be_bytes(len_buf);
    if len > max_len {
        return Err(FramingError::Oversize { len, max: max_len });
    }
    let mut payload = vec![0u8; len as usize];
    reader.read_exact(&mut payload)?;
    Ok(Some(payload))
}

/// Write one `{:packet, 4}` frame.
pub fn write_packet<W: Write>(writer: &mut W, payload: &[u8]) -> Result<(), FramingError> {
    let len = u32::try_from(payload.len()).map_err(|_| FramingError::Oversize {
        len: u32::MAX,
        max: u32::MAX - 1,
    })?;
    writer.write_all(&len.to_be_bytes())?;
    writer.write_all(payload)?;
    writer.flush()?;
    Ok(())
}

/// Serve loop: read packets, map to replies, write packets until EOF.
pub fn serve_loop<R, W, F>(
    reader: &mut R,
    writer: &mut W,
    max_len: u32,
    mut handle: F,
) -> Result<(), FramingError>
where
    R: Read,
    W: Write,
    F: FnMut(&[u8]) -> ServeReply,
{
    loop {
        match read_packet(reader, max_len)? {
            None => return Ok(()),
            Some(payload) => {
                let reply = handle(&payload);
                write_packet(writer, &reply.as_bytes())?;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn round_trip_empty_and_bytes() {
        let mut buf = Vec::new();
        write_packet(&mut buf, b"").unwrap();
        write_packet(&mut buf, b"abc").unwrap();
        let mut cur = Cursor::new(buf);
        assert_eq!(read_packet(&mut cur, 100).unwrap(), Some(vec![]));
        assert_eq!(read_packet(&mut cur, 100).unwrap(), Some(b"abc".to_vec()));
        assert_eq!(read_packet(&mut cur, 100).unwrap(), None);
    }

    #[test]
    fn oversize_is_hard_error() {
        let mut buf = Vec::new();
        write_packet(&mut buf, &[0u8; 8]).unwrap();
        let mut cur = Cursor::new(buf);
        match read_packet(&mut cur, 4) {
            Err(FramingError::Oversize { len: 8, max: 4 }) => {}
            other => panic!("expected oversize, got {other:?}"),
        }
    }

    #[test]
    fn serve_reply_hex_and_soft_error() {
        let ok = ServeReply::ok_hex("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")
            .unwrap();
        assert_eq!(ok.as_bytes().len(), 64);
        let err = ServeReply::soft_error("bad");
        assert_eq!(err.as_bytes(), b"!bad");
    }

    #[test]
    fn soft_error_truncates_on_char_boundary() {
        // 201 chars of multi-byte 'あ' (3 bytes each) must not panic on truncate.
        let long: String = std::iter::repeat('あ').take(201).collect();
        let reply = ServeReply::soft_error(long);
        let bytes = reply.as_bytes();
        assert_eq!(bytes[0], SOFT_ERR_PREFIX);
        assert!(std::str::from_utf8(&bytes[1..]).is_ok());
        assert_eq!(std::str::from_utf8(&bytes[1..]).unwrap().chars().count(), 200);
    }

    #[test]
    fn serve_loop_maps_payload() {
        let mut req = Vec::new();
        write_packet(&mut req, b"hi").unwrap();
        let mut out = Vec::new();
        let mut cur = Cursor::new(req);
        serve_loop(&mut cur, &mut out, 16, |p| {
            assert_eq!(p, b"hi");
            ServeReply::soft_error("x")
        })
        .unwrap();
        let mut resp = Cursor::new(out);
        assert_eq!(read_packet(&mut resp, 16).unwrap(), Some(b"!x".to_vec()));
    }
}
