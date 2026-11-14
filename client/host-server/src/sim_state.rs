//! Planned handover / crash boot decisions for `…/sim_state` (x00ag / mvcc_xpbd §4.8).
//!
//! # Guarantees
//! - Planned path: signed+AEAD SimState → quarantine → PhysicsWorld import (callers).
//! - Crash path: last-good preferred; 1 frame → velocity 0; unknown + no snapshot →
//!   [`BootDecision::FailClosed`] (no Manifest cold start).
//! - Non-finite / overspeed / bad holder rejected by quarantine helpers.
//!
//! # Non-guarantees
//! - App→host last-good IPC; automatic old-host `host_pub` discovery; cross-arch bit match;
//! - `drot` restore; supervisor wiring of [`HostBootMode`]; `…/edit`.

use std::collections::HashMap;

use alchemy_protocol::wire_crypto::sim_entity_is_finite;
use alchemy_protocol::{entity_holder_slot, SimEntity, SimState, RELEASE_LINEAR_SPEED_MAX};

/// Physics tick period used when approximating velocity from frame deltas (60Hz).
pub const SIM_STATE_DT: f32 = 1.0 / 60.0;

/// Soft world bound for import quarantine (demo cell_origin≈0; not Spatial Interest).
pub const SIM_STATE_POS_ABS_MAX: f64 = 1.0e5;

/// How the host process should obtain its initial physics state (x00ag).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostBootMode {
    /// Manifest entries → PhysicsWorld (existing default).
    ColdStart,
    /// Fetch `…/sim_state` from the previous host and resume.
    PlannedHandover,
    /// Crash promote: last-good / snapshot / fail-closed (§4.8).
    CrashPromote,
}

impl HostBootMode {
    /// Parse `ALCHEMY_HOST_BOOT_MODE` (`cold` | `planned_handover` | `crash_promote`).
    /// Empty / unset → [`HostBootMode::ColdStart`]. Unknown values are `Err` (fail-closed).
    pub fn from_env_value(raw: Option<&str>) -> Result<Self, String> {
        match raw.map(str::trim).unwrap_or("") {
            "" | "cold" | "cold_start" => Ok(Self::ColdStart),
            "planned_handover" | "planned" => Ok(Self::PlannedHandover),
            "crash_promote" | "crash" => Ok(Self::CrashPromote),
            other => Err(format!(
                "unknown ALCHEMY_HOST_BOOT_MODE={other:?}; expected cold|planned_handover|crash_promote"
            )),
        }
    }

    pub fn from_env() -> Result<Self, String> {
        Self::from_env_value(std::env::var("ALCHEMY_HOST_BOOT_MODE").ok().as_deref())
    }
}

/// One accepted (x0024) frame pose used as crash last-good material.
#[derive(Debug, Clone)]
pub struct LastGoodFrame {
    pub sim_tick: u64,
    pub net_tick: u64,
    pub entities: Vec<SimEntity>,
}

/// Inputs for crash / planned boot decision (pure; no I/O).
#[derive(Debug, Clone)]
pub enum BootSource {
    Cold,
    Planned(SimState),
    Crash {
        last_good: Vec<LastGoodFrame>,
        /// Snapshot already fetched (None = unavailable / old host dead).
        snapshot: Option<SimState>,
    },
}

/// How [`crate::sim::PhysicsWorld::from_sim_state_with_backend`] treats wire `flags`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SimStateImportKind {
    /// Planned handover: every entity must have non-zero readiness `flags` (DoD).
    Planned,
    /// Crash seed: `flags==0` keeps Manifest/cold readiness (pose-only / velocity 0).
    CrashPoseOnly,
}

#[derive(Debug, Clone)]
pub enum BootDecision {
    /// Start physics from this state (cold uses empty entities → Manifest path).
    Start {
        state: Option<SimState>,
        /// When true, callers must use Manifest cold start (never for crash unknown).
        cold_manifest: bool,
        /// `Some` only when `state` is `Some` (cold Manifest path leaves this `None`).
        import: Option<SimStateImportKind>,
    },
    FailClosed {
        reason: &'static str,
    },
}

/// Decide how to boot physics (§4.8). Never returns cold Manifest for crash-unknown.
pub fn decide_boot(source: BootSource) -> BootDecision {
    match source {
        BootSource::Cold => BootDecision::Start {
            state: None,
            cold_manifest: true,
            import: None,
        },
        BootSource::Planned(state) => BootDecision::Start {
            state: Some(state),
            cold_manifest: false,
            import: Some(SimStateImportKind::Planned),
        },
        BootSource::Crash {
            last_good,
            snapshot,
        } => {
            if last_good.is_empty() {
                match snapshot {
                    Some(state) => BootDecision::Start {
                        state: Some(state_as_crash_pose_seed(state)),
                        cold_manifest: false,
                        import: Some(SimStateImportKind::CrashPoseOnly),
                    },
                    None => BootDecision::FailClosed {
                        reason: "crash promote: no last-good and snapshot unavailable",
                    },
                }
            } else if last_good.len() == 1 {
                let mut state = sim_state_from_last_good(&last_good[0]);
                zero_entity_deltas(&mut state);
                // Pose/velocity from last-good; readiness stays cold (CrashPoseOnly).
                zero_entity_flags(&mut state);
                BootDecision::Start {
                    state: Some(state),
                    cold_manifest: false,
                    import: Some(SimStateImportKind::CrashPoseOnly),
                }
            } else {
                let a = &last_good[last_good.len() - 2];
                let b = &last_good[last_good.len() - 1];
                let mut state = approx_velocity_from_two_frames(a, b);
                zero_entity_flags(&mut state);
                BootDecision::Start {
                    state: Some(state),
                    cold_manifest: false,
                    import: Some(SimStateImportKind::CrashPoseOnly),
                }
            }
        }
    }
}

fn sim_state_from_last_good(frame: &LastGoodFrame) -> SimState {
    SimState {
        protocol_pin: String::new(),
        room_id: String::new(),
        host_generation: 0,
        scene_rev: 0,
        sim_tick: frame.sim_tick,
        net_tick: frame.net_tick,
        entities: frame.entities.clone(),
        host_sig: Vec::new(),
    }
}

/// CrashPoseOnly seed: zero velocities and force flags=0 (pose only; cold readiness kept at apply).
fn state_as_crash_pose_seed(mut state: SimState) -> SimState {
    zero_entity_deltas(&mut state);
    zero_entity_flags(&mut state);
    state
}

fn zero_entity_deltas(state: &mut SimState) {
    for e in &mut state.entities {
        e.dx = 0.0;
        e.dy = 0.0;
        e.dz = 0.0;
        e.drot_x = 0.0;
        e.drot_y = 0.0;
        e.drot_z = 0.0;
    }
}

/// CrashPoseOnly contract: never carry readiness on the wire into apply.
fn zero_entity_flags(state: &mut SimState) {
    for e in &mut state.entities {
        e.flags = 0;
    }
}

/// Approximate per-tick displacement from two last-good poses (dt = publish interval proxy).
fn approx_velocity_from_two_frames(a: &LastGoodFrame, b: &LastGoodFrame) -> SimState {
    let mut state = sim_state_from_last_good(b);
    let tick_span = b.net_tick.saturating_sub(a.net_tick).max(1) as f32;
    // Assume net publishes at ~30Hz when approximating from wire frames.
    let interval = tick_span / 30.0;
    let scale = if interval > 0.0 {
        SIM_STATE_DT / interval
    } else {
        0.0
    };
    let prev_by_id: HashMap<u32, &SimEntity> = a.entities.iter().map(|e| (e.id, e)).collect();
    for e in &mut state.entities {
        if let Some(prev) = prev_by_id.get(&e.id) {
            e.dx = (e.x - prev.x) as f32 * scale;
            e.dy = (e.y - prev.y) as f32 * scale;
            e.dz = (e.z - prev.z) as f32 * scale;
        } else {
            e.dx = 0.0;
            e.dy = 0.0;
            e.dz = 0.0;
        }
        e.drot_x = 0.0;
        e.drot_y = 0.0;
        e.drot_z = 0.0;
    }
    state
}

/// Why [`quarantine_sim_state`] rejected a payload.
#[derive(Debug, Clone, PartialEq)]
pub enum QuarantineError {
    NonFinite {
        entity_id: u32,
    },
    Overspeed {
        entity_id: u32,
        speed: f32,
    },
    PosOutOfRange {
        entity_id: u32,
    },
    BadHolder {
        entity_id: u32,
        holder: u32,
    },
    EmptyEntities,
    /// Planned import: wire readiness flags must be non-zero (not crash `flags==0` sentinel).
    MissingReadinessFlags {
        entity_id: u32,
    },
}

impl std::fmt::Display for QuarantineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NonFinite { entity_id } => write!(f, "non-finite entity {entity_id}"),
            Self::Overspeed { entity_id, speed } => {
                write!(f, "entity {entity_id} speed {speed} exceeds clamp")
            }
            Self::PosOutOfRange { entity_id } => {
                write!(f, "entity {entity_id} position out of quarantine range")
            }
            Self::BadHolder { entity_id, holder } => {
                write!(f, "entity {entity_id} holder {holder} invalid")
            }
            Self::EmptyEntities => write!(f, "sim_state has no entities"),
            Self::MissingReadinessFlags { entity_id } => write!(
                f,
                "planned sim_state entity {entity_id} has flags=0 (readiness required)"
            ),
        }
    }
}

impl std::error::Error for QuarantineError {}

/// Reject non-finite / overspeed / OOB / bad holder before page write (fail-closed).
///
/// [`SimStateImportKind::Planned`] also requires every entity `flags != 0` so a blanked
/// readiness payload cannot silently keep cold masks and break trajectory DoD.
pub fn quarantine_sim_state(
    state: &SimState,
    import: SimStateImportKind,
) -> Result<(), QuarantineError> {
    if state.entities.is_empty() {
        return Err(QuarantineError::EmptyEntities);
    }
    let max_delta = RELEASE_LINEAR_SPEED_MAX * SIM_STATE_DT;
    let max_delta2 = max_delta * max_delta;
    let require_flags = import == SimStateImportKind::Planned;
    for e in &state.entities {
        if !sim_entity_is_finite(e) {
            return Err(QuarantineError::NonFinite { entity_id: e.id });
        }
        if e.x.abs() > SIM_STATE_POS_ABS_MAX
            || e.y.abs() > SIM_STATE_POS_ABS_MAX
            || e.z.abs() > SIM_STATE_POS_ABS_MAX
        {
            return Err(QuarantineError::PosOutOfRange { entity_id: e.id });
        }
        // 0 = none; non-zero must decode to a valid slot (`entity_holder_slot`).
        if e.holder != 0 && entity_holder_slot(e.holder).is_none() {
            return Err(QuarantineError::BadHolder {
                entity_id: e.id,
                holder: e.holder,
            });
        }
        if require_flags && e.flags == 0 {
            return Err(QuarantineError::MissingReadinessFlags { entity_id: e.id });
        }
        let speed2 = e.dx * e.dx + e.dy * e.dy + e.dz * e.dz;
        if speed2 > max_delta2 {
            return Err(QuarantineError::Overspeed {
                entity_id: e.id,
                speed: speed2.sqrt() / SIM_STATE_DT,
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entity(id: u32, x: f64, dx: f32) -> SimEntity {
        SimEntity {
            id,
            x,
            y: 0.0,
            z: 0.0,
            qx: 0.0,
            qy: 0.0,
            qz: 0.0,
            qw: 1.0,
            dx,
            dy: 0.0,
            dz: 0.0,
            drot_x: 0.0,
            drot_y: 0.0,
            drot_z: 0.0,
            holder: 0,
            flags: 0,
        }
    }

    #[test]
    fn boot_mode_env_parse() {
        assert_eq!(
            HostBootMode::from_env_value(Some("planned_handover")).unwrap(),
            HostBootMode::PlannedHandover
        );
        assert_eq!(
            HostBootMode::from_env_value(Some("crash_promote")).unwrap(),
            HostBootMode::CrashPromote
        );
        assert_eq!(
            HostBootMode::from_env_value(None).unwrap(),
            HostBootMode::ColdStart
        );
        assert!(HostBootMode::from_env_value(Some("planned-handover")).is_err());
        assert!(HostBootMode::from_env_value(Some("nope")).is_err());
    }

    #[test]
    fn crash_fail_closed_unknown_without_snapshot() {
        let d = decide_boot(BootSource::Crash {
            last_good: vec![],
            snapshot: None,
        });
        match d {
            BootDecision::FailClosed { reason } => {
                assert!(reason.contains("last-good"));
            }
            BootDecision::Start { cold_manifest, .. } => {
                panic!("must not cold-start; cold_manifest={cold_manifest}");
            }
        }
    }

    #[test]
    fn crash_handover_single_frame_velocity_zero() {
        let frame = LastGoodFrame {
            sim_tick: 3,
            net_tick: 1,
            entities: vec![entity(7, 1.0, 9.0)],
        };
        let d = decide_boot(BootSource::Crash {
            last_good: vec![frame],
            snapshot: None,
        });
        let BootDecision::Start {
            state: Some(state),
            cold_manifest: false,
            import: Some(SimStateImportKind::CrashPoseOnly),
        } = d
        else {
            panic!("expected start from single last-good");
        };
        assert_eq!(state.entities[0].dx, 0.0);
        assert_eq!(state.entities[0].dy, 0.0);
        assert_eq!(state.entities[0].dz, 0.0);
        assert_eq!(state.entities[0].flags, 0);
    }

    #[test]
    fn crash_two_frames_approximates_delta() {
        let a = LastGoodFrame {
            sim_tick: 1,
            net_tick: 1,
            entities: vec![entity(1, 0.0, 0.0)],
        };
        let b = LastGoodFrame {
            sim_tick: 2,
            net_tick: 2,
            entities: vec![entity(1, 1.0, 0.0)],
        };
        let d = decide_boot(BootSource::Crash {
            last_good: vec![a, b],
            snapshot: None,
        });
        let BootDecision::Start {
            state: Some(state), ..
        } = d
        else {
            panic!("expected approx start");
        };
        // 1m over ~1/30s → ~30 m/s; scaled to 60Hz tick ≈ 0.5 m displacement.
        assert!(state.entities[0].dx > 0.0);
    }

    #[test]
    fn quarantine_rejects_nan_and_overspeed() {
        let mut state = SimState {
            protocol_pin: "pin".into(),
            room_id: "local".into(),
            entities: vec![entity(1, 0.0, 0.0)],
            ..Default::default()
        };
        assert!(quarantine_sim_state(&state, SimStateImportKind::CrashPoseOnly).is_ok());
        state.entities[0].dx = f32::NAN;
        assert!(matches!(
            quarantine_sim_state(&state, SimStateImportKind::CrashPoseOnly),
            Err(QuarantineError::NonFinite { .. })
        ));
        state.entities[0].dx = RELEASE_LINEAR_SPEED_MAX; // per-tick ≈ 20m → overspeed
        state.entities[0].dy = 0.0;
        state.entities[0].dz = 0.0;
        assert!(matches!(
            quarantine_sim_state(&state, SimStateImportKind::CrashPoseOnly),
            Err(QuarantineError::Overspeed { .. })
        ));
    }

    #[test]
    fn planned_quarantine_rejects_all_zero_flags() {
        let mut state = SimState {
            protocol_pin: "pin".into(),
            room_id: "local".into(),
            entities: vec![entity(1, 0.0, 0.0)],
            ..Default::default()
        };
        state.entities[0].flags = 0;
        assert!(matches!(
            quarantine_sim_state(&state, SimStateImportKind::Planned),
            Err(QuarantineError::MissingReadinessFlags { .. })
        ));
        state.entities[0].flags = 1; // ALLOCATED-ish non-zero
        assert!(quarantine_sim_state(&state, SimStateImportKind::Planned).is_ok());
    }
}
