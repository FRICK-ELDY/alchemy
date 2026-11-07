//! Zenoh liveliness token declaration for the current Room Token `sub`+`jti` (x0022 / x0019).
//!
//! Lives under `bus/` (not `room`) so `bus::input` can rebind liveliness without a
//! `room` ↔ `bus` dependency cycle; `room` calls into this module.
//!
//! Guarantees:
//! - Returns `Err` when not joined / missing `wire_cap`; token keyed by
//!   `room_id` + `wire_cap` + `sub` + `jti`
//! - `retain_previous = false` drops any previous token immediately (join / re-login);
//!   `true` stashes it in `liveliness_prev` until the next Intent put (Room Token refresh)
//!
//! Non-guarantees: undeclare on Leave / session close (see `room::leave_room`), host-side
//! liveliness observation.

use alchemy_protocol::liveliness_key;
use anyhow::{Context, Result};
use zenoh::Wait;

use crate::jwt_sub;
use crate::scene::SharedState;

/// Declare Zenoh liveliness for current Room Token `sub`+`jti` (x0022). No-op if not joined.
///
/// `retain_previous`: only for Room Token **refresh** rebind. Stashes the old token in
/// `liveliness_prev` until the next Intent put. Join / re-login pass `false` and drop any
/// previous key immediately (no dual-key leak until leave).
pub(crate) fn declare_room_alive(
    shared: &SharedState,
    session: &zenoh::Session,
    retain_previous: bool,
) -> Result<()> {
    let room_token = shared
        .auth
        .lock()
        .ok()
        .and_then(|g| g.room_token.clone())
        .context("declare liveliness: not joined")?;
    let (sub, jti) =
        jwt_sub::sub_jti_unverified(&room_token).context("declare liveliness: read sub/jti")?;
    let wire_cap = shared
        .wire_cap()
        .ok_or_else(|| anyhow::anyhow!("liveliness requires wire_cap (join first; x004e)"))?;
    let key = liveliness_key(shared.get_room_id().as_ref(), &wire_cap, &sub, &jti);
    let token = session
        .liveliness()
        .declare_token(key.clone())
        .wait()
        .map_err(|e| anyhow::anyhow!("declare liveliness token failed: {e}"))?;
    if let Ok(mut slot) = shared.liveliness.lock() {
        let old = slot.replace(token);
        if let Some(old) = old {
            if retain_previous {
                if let Ok(mut prev) = shared.liveliness_prev.lock() {
                    *prev = Some(old);
                }
            }
            // else: drop `old` immediately (join / re-login)
        }
    }
    log::info!("liveliness declared key={key} retain_previous={retain_previous}");
    Ok(())
}
