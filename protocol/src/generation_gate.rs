//! Monotonic `(host_generation, revision)` gate (x003a).
//!
//! Guarantees: lexicographic non-decrease; generation advance only when caller presents
//! a world-proven higher generation (HostAttestation). Presence must not raise generation.
//! Non-guarantees: Zenoh delivery order; full host handoff UX.

/// Accepted watermark for Manifest or Presence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GenRev {
    pub host_generation: u64,
    pub revision: u64,
}

/// Decision after comparing incoming wire values to a known watermark.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GenRevDecision {
    /// First accepted sample for this room session.
    AcceptFirst,
    /// Same generation; revision non-decreasing.
    AcceptSameOrForward,
    /// Higher generation (world-proven handoff). Caller must reset host session state.
    AcceptNewGeneration,
    /// Older generation or same-generation revision regression → discard.
    RejectRegression,
    /// `host_generation == 0` is never valid on the wire after x003a.
    RejectInvalidGeneration,
}

/// Decide whether to adopt `(host_generation, revision)`.
///
/// Call only after cryptographic verification (seal / attestation / host_sig).
pub fn decide_gen_rev(known: Option<GenRev>, incoming: GenRev) -> GenRevDecision {
    if incoming.host_generation == 0 {
        return GenRevDecision::RejectInvalidGeneration;
    }
    match known {
        None => GenRevDecision::AcceptFirst,
        Some(k) if incoming.host_generation < k.host_generation => GenRevDecision::RejectRegression,
        Some(k) if incoming.host_generation > k.host_generation => {
            GenRevDecision::AcceptNewGeneration
        }
        Some(k) if incoming.revision < k.revision => GenRevDecision::RejectRegression,
        Some(_) => GenRevDecision::AcceptSameOrForward,
    }
}

/// Presence may only match the accepted Manifest generation (never raise it).
pub fn decide_presence_gen_rev(
    accepted_manifest_generation: Option<u64>,
    known_presence: Option<GenRev>,
    incoming: GenRev,
) -> GenRevDecision {
    if incoming.host_generation == 0 {
        return GenRevDecision::RejectInvalidGeneration;
    }
    let Some(manifest_gen) = accepted_manifest_generation else {
        // No Manifest yet → cannot apply Presence against an attested host.
        return GenRevDecision::RejectRegression;
    };
    if incoming.host_generation != manifest_gen {
        return GenRevDecision::RejectRegression;
    }
    decide_gen_rev(known_presence, incoming)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gr(g: u64, r: u64) -> GenRev {
        GenRev {
            host_generation: g,
            revision: r,
        }
    }

    #[test]
    fn first_and_forward() {
        assert_eq!(decide_gen_rev(None, gr(1, 1)), GenRevDecision::AcceptFirst);
        assert_eq!(
            decide_gen_rev(Some(gr(1, 1)), gr(1, 1)),
            GenRevDecision::AcceptSameOrForward
        );
        assert_eq!(
            decide_gen_rev(Some(gr(1, 1)), gr(1, 2)),
            GenRevDecision::AcceptSameOrForward
        );
    }

    #[test]
    fn reject_revision_and_generation_regression() {
        assert_eq!(
            decide_gen_rev(Some(gr(1, 5)), gr(1, 4)),
            GenRevDecision::RejectRegression
        );
        assert_eq!(
            decide_gen_rev(Some(gr(2, 1)), gr(1, 99)),
            GenRevDecision::RejectRegression
        );
    }

    #[test]
    fn accept_new_generation() {
        assert_eq!(
            decide_gen_rev(Some(gr(1, 9)), gr(2, 0)),
            GenRevDecision::AcceptNewGeneration
        );
    }

    #[test]
    fn reject_generation_zero() {
        assert_eq!(
            decide_gen_rev(None, gr(0, 1)),
            GenRevDecision::RejectInvalidGeneration
        );
    }

    #[test]
    fn presence_must_match_manifest_generation() {
        assert_eq!(
            decide_presence_gen_rev(None, None, gr(1, 1)),
            GenRevDecision::RejectRegression
        );
        assert_eq!(
            decide_presence_gen_rev(Some(1), None, gr(2, 1)),
            GenRevDecision::RejectRegression
        );
        assert_eq!(
            decide_presence_gen_rev(Some(1), None, gr(1, 1)),
            GenRevDecision::AcceptFirst
        );
        assert_eq!(
            decide_presence_gen_rev(Some(1), Some(gr(1, 3)), gr(1, 2)),
            GenRevDecision::RejectRegression
        );
    }
}
