//! Authoritative sim backend selection (x00ad / MVCC Phase 3 着手).
//!
//! Guarantees: `ALCHEMY_SIM_BACKEND` unset → [`SimBackendKind::EpochXpbd`];
//! known values parse; unknown values are **Err** (no silent fallback).
//! Non-guarantees: flipping production default to `mvcc_xpbd`; dirty codec / `sim_state`.

#![forbid(unsafe_code)]

/// Host 60Hz authority backend (x00ad).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SimBackendKind {
    /// EpochSheet + XPBD（既定）。接触オン（`bounds: Some`）。
    #[default]
    EpochXpbd,
    /// HotPhysicsPage + [`alchemy_shared_sim::PageXpbdAdapter`]（接触オン）。
    /// Wire sample は本スライスでは Epoch Curr へ同期してから既存経路を使う。
    MvccXpbd,
}

impl SimBackendKind {
    pub const ENV_KEY: &'static str = "ALCHEMY_SIM_BACKEND";

    /// Parse env. Unset / empty → [`Self::EpochXpbd`]. Unknown → `Err`.
    pub fn from_env() -> Result<Self, String> {
        match std::env::var(Self::ENV_KEY) {
            Err(std::env::VarError::NotPresent) => Ok(Self::EpochXpbd),
            Err(e) => Err(format!("{} read failed: {e}", Self::ENV_KEY)),
            Ok(raw) => Self::parse(raw.trim()),
        }
    }

    pub fn parse(s: &str) -> Result<Self, String> {
        if s.is_empty() {
            return Ok(Self::EpochXpbd);
        }
        match s {
            "epoch_xpbd" => Ok(Self::EpochXpbd),
            "mvcc_xpbd" => Ok(Self::MvccXpbd),
            other => Err(format!(
                "invalid {}={other:?}; expected epoch_xpbd|mvcc_xpbd",
                Self::ENV_KEY
            )),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::EpochXpbd => "epoch_xpbd",
            Self::MvccXpbd => "mvcc_xpbd",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_known_and_default_empty() {
        assert_eq!(
            SimBackendKind::parse("").unwrap(),
            SimBackendKind::EpochXpbd
        );
        assert_eq!(
            SimBackendKind::parse("epoch_xpbd").unwrap(),
            SimBackendKind::EpochXpbd
        );
        assert_eq!(
            SimBackendKind::parse("mvcc_xpbd").unwrap(),
            SimBackendKind::MvccXpbd
        );
    }

    #[test]
    fn parse_rejects_unknown() {
        let err = SimBackendKind::parse("rapier").unwrap_err();
        assert!(err.contains("invalid"), "{err}");
        assert!(err.contains("epoch_xpbd"), "{err}");
    }
}
