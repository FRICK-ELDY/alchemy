//! Identity / pin gates (x0038). Failure → discard sample (never leave).

use alchemy_protocol::protocol_pin_matches;

/// x0038: Manifest identity checks. Failure → discard sample (never leave).
pub fn gate_manifest_identity(
    protocol_pin: &str,
    room_id: &str,
    host_pub_len: usize,
    expected_room: &str,
) -> Result<(), &'static str> {
    if !protocol_pin_matches(protocol_pin) {
        return Err("protocol_pin mismatch");
    }
    if room_id != expected_room {
        return Err("room_id mismatch");
    }
    if host_pub_len != 32 {
        return Err("host_pub length");
    }
    Ok(())
}

/// x0038: Presence pin check. Failure → discard sample (never leave).
pub fn gate_presence_pin(protocol_pin: &str) -> Result<(), &'static str> {
    if !protocol_pin_matches(protocol_pin) {
        return Err("protocol_pin mismatch");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use alchemy_protocol::PROTOCOL_PIN;

    #[test]
    fn manifest_identity_gate_rejects_bad_pin_room_and_host_pub_len() {
        assert!(gate_manifest_identity(PROTOCOL_PIN, "local", 32, "local").is_ok());
        assert_eq!(
            gate_manifest_identity("bogus-pin", "local", 32, "local"),
            Err("protocol_pin mismatch")
        );
        assert_eq!(
            gate_manifest_identity(PROTOCOL_PIN, "other", 32, "local"),
            Err("room_id mismatch")
        );
        assert_eq!(
            gate_manifest_identity(PROTOCOL_PIN, "local", 16, "local"),
            Err("host_pub length")
        );
    }

    #[test]
    fn presence_pin_gate_rejects_mismatch() {
        assert!(gate_presence_pin(PROTOCOL_PIN).is_ok());
        assert_eq!(gate_presence_pin("bogus-pin"), Err("protocol_pin mismatch"));
    }
}
