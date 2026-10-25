//! Read JWT `sub` from payload without signature verification.
//!
//! Host verifies Room Token; client only needs `sub` to match PresenceState.

use anyhow::{bail, Context, Result};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct ClaimsSub {
    sub: String,
}

/// Decode JWT payload and return `sub`. Does **not** verify the signature.
pub fn sub_unverified(token: &str) -> Result<String> {
    let mut parts = token.split('.');
    let _header = parts.next().context("JWT missing header")?;
    let payload_b64 = parts.next().context("JWT missing payload")?;
    if parts.next().is_none() {
        bail!("JWT missing signature segment");
    }
    let json = base64url_decode(payload_b64).context("JWT payload base64")?;
    let claims: ClaimsSub = serde_json::from_slice(&json).context("JWT payload JSON")?;
    if claims.sub.is_empty() {
        bail!("JWT sub is empty");
    }
    Ok(claims.sub)
}

fn base64url_decode(input: &str) -> Result<Vec<u8>> {
    let mut s = input.replace('-', "+").replace('_', "/");
    while !s.len().is_multiple_of(4) {
        s.push('=');
    }
    base64_std_decode(&s)
}

fn base64_std_decode(input: &str) -> Result<Vec<u8>> {
    const TABLE: &[u8; 128] = &{
        let mut t = [0xffu8; 128];
        let mut i = 0u8;
        while i < 26 {
            t[(b'A' + i) as usize] = i;
            t[(b'a' + i) as usize] = 26 + i;
            i += 1;
        }
        i = 0;
        while i < 10 {
            t[(b'0' + i) as usize] = 52 + i;
            i += 1;
        }
        t[b'+' as usize] = 62;
        t[b'/' as usize] = 63;
        t
    };

    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len() * 3 / 4);
    let mut buf = 0u32;
    let mut bits = 0u32;
    for &c in bytes {
        if c == b'=' {
            break;
        }
        if c as usize >= TABLE.len() || TABLE[c as usize] == 0xff {
            bail!("invalid base64 byte 0x{c:02x}");
        }
        buf = (buf << 6) | u32::from(TABLE[c as usize]);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
            buf &= (1 << bits) - 1;
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_known_payload() {
        // {"sub":"user-1"} — header/sig are dummy
        let token = "eyJhbGciOiJub25lIn0.eyJzdWIiOiJ1c2VyLTEifQ.sig";
        assert_eq!(sub_unverified(token).unwrap(), "user-1");
    }
}
