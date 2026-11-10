//! Shared blocking HTTP agent for credential / seal / scene calls (credential worker only; x003d).
//!
//! Guarantees: fixed connect/read timeout; canonical blake3 hex shape check.
//! x0081: host HTTP also carries `cnf.spk` PoP (see `http_pop_client`).
//! Non-guarantees: retries, auth, TLS configuration.

use std::time::Duration;

/// Auth / Room Token / HostAttestation / ManifestSeal HTTP (credential worker only; x003d).
const CREDENTIAL_HTTP_TIMEOUT: Duration = Duration::from_secs(5);

pub(super) fn credential_agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(CREDENTIAL_HTTP_TIMEOUT)
        .timeout_read(CREDENTIAL_HTTP_TIMEOUT)
        .build()
}

pub(super) fn is_blake3_hex(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blake3_hex_accepts_64_hex_rejects_charset() {
        assert!(is_blake3_hex(&"ab".repeat(32)));
        assert!(is_blake3_hex(&"AB".repeat(32)));
        assert!(!is_blake3_hex("short"));
        assert!(!is_blake3_hex(&"zz".repeat(32)));
        assert!(!is_blake3_hex(&format!("{}!", "a".repeat(63))));
    }
}
