//! Auth-server HTTP session shell (x0018 / x0021).
//!
//! Guarantees: login / register / logout / refresh against existing `/api/v1/auth/*`;
//! refresh deadlines at ~80% of `expires_in` for access and Room Token.
//! Non-guarantees: email verify UI, OAuth, disk credential vault, OpenXR keyboard.

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::time::{Duration, Instant};

/// Fraction of TTL at which client re-fetches (P1-1).
pub const TOKEN_REFRESH_FRACTION: f64 = 0.8;

#[derive(Debug, Clone, Deserialize)]
pub struct AuthUser {
    pub user_id: String,
    pub username: String,
    pub email: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AuthSession {
    pub access_token: String,
    pub token_type: String,
    pub expires_in: u64,
    #[serde(default)]
    pub refresh_token: Option<String>,
    pub user: AuthUser,
}

#[derive(Debug, Clone, Default)]
pub struct AuthGate {
    pub access_token: Option<String>,
    pub refresh_token: Option<String>,
    pub username: Option<String>,
    pub room_token: Option<String>,
    pub local_sub: Option<String>,
    /// When `Instant::now() >= deadline`, refresh access (None = do not auto-refresh).
    pub access_deadline: Option<Instant>,
    /// When `Instant::now() >= deadline`, re-issue Room Token.
    pub room_deadline: Option<Instant>,
    /// After Room Token refresh: declare new `…/alive/{sub}/{jti}` after the next Intent put (x0022).
    pub liveliness_rebind: bool,
    /// After rebind declare: drop the previous liveliness token on the *following* Intent put.
    /// Keeps old key alive one more put; host also defers liveliness reclaim with grace.
    pub liveliness_retire_prev: bool,
}

impl AuthGate {
    pub fn is_authenticated(&self) -> bool {
        self.access_token.is_some()
    }

    pub fn is_joined(&self) -> bool {
        self.room_token.is_some()
    }

    pub fn clear_session(&mut self) {
        self.access_token = None;
        self.refresh_token = None;
        self.username = None;
        self.room_token = None;
        self.local_sub = None;
        self.access_deadline = None;
        self.room_deadline = None;
        self.liveliness_rebind = false;
        self.liveliness_retire_prev = false;
    }

    pub fn apply_auth_session(&mut self, session: AuthSession) {
        self.access_deadline = refresh_deadline(session.expires_in);
        self.access_token = Some(session.access_token);
        self.refresh_token = session.refresh_token;
        self.username = Some(session.user.username);
    }

    pub fn apply_room(&mut self, room_token: String, local_sub: String, expires_in: u64) {
        self.room_token = Some(room_token);
        self.local_sub = Some(local_sub);
        self.room_deadline = refresh_deadline(expires_in);
        // Initial join declares liveliness explicitly; refresh sets rebind via `apply_room_refresh`.
        self.liveliness_rebind = false;
        self.liveliness_retire_prev = false;
    }

    /// Room Token refresh: keep join, schedule liveliness rebind after next Intent put.
    pub fn apply_room_refresh(&mut self, room_token: String, local_sub: String, expires_in: u64) {
        self.room_token = Some(room_token);
        self.local_sub = Some(local_sub);
        self.room_deadline = refresh_deadline(expires_in);
        self.liveliness_rebind = true;
        self.liveliness_retire_prev = false;
    }

    pub fn clear_room(&mut self) {
        self.room_token = None;
        self.local_sub = None;
        self.room_deadline = None;
        self.liveliness_rebind = false;
        self.liveliness_retire_prev = false;
    }
}

/// Instant at which a token with `expires_in` seconds should be refreshed, or `None` if TTL is 0.
pub fn refresh_deadline(expires_in: u64) -> Option<Instant> {
    if expires_in == 0 {
        return None;
    }
    let secs = ((expires_in as f64) * TOKEN_REFRESH_FRACTION).floor() as u64;
    Some(Instant::now() + Duration::from_secs(secs.max(1)))
}

pub fn login(
    auth_url: &str,
    identifier: &str,
    password: &str,
    remember_me: bool,
) -> Result<AuthSession> {
    let base = auth_url.trim_end_matches('/');
    let url = format!("{base}/api/v1/auth/login");
    let resp = ureq::post(&url)
        .set("content-type", "application/json")
        .send_json(serde_json::json!({
            "identifier": identifier,
            "password": password,
            "remember_me": remember_me,
        }))
        .context("auth login failed")?;
    parse_session(resp)
}

pub fn register(
    auth_url: &str,
    username: &str,
    email: &str,
    password: &str,
    birthday: &str,
    remember_me: bool,
) -> Result<AuthSession> {
    let base = auth_url.trim_end_matches('/');
    let url = format!("{base}/api/v1/auth/register");
    let resp = ureq::post(&url)
        .set("content-type", "application/json")
        .send_json(serde_json::json!({
            "username": username,
            "email": email,
            "password": password,
            "birthday": birthday,
            "tos_agreed": true,
            "remember_me": remember_me,
        }))
        .context("auth register failed")?;
    parse_session(resp)
}

pub fn logout(auth_url: &str, access_token: &str, refresh_token: Option<&str>) -> Result<()> {
    let base = auth_url.trim_end_matches('/');
    let url = format!("{base}/api/v1/auth/logout");
    let body = if let Some(rt) = refresh_token {
        serde_json::json!({ "refresh_token": rt })
    } else {
        serde_json::json!({})
    };
    match ureq::post(&url)
        .set("content-type", "application/json")
        .set("authorization", &format!("Bearer {access_token}"))
        .send_json(body)
    {
        Ok(_) => Ok(()),
        Err(err) => {
            // Local clear still required by Slice contract; treat network/API failure as soft.
            log::warn!("auth logout API failed (local session still cleared): {err}");
            Ok(())
        }
    }
}

pub fn refresh(auth_url: &str, refresh_token: &str) -> Result<AuthSession> {
    let base = auth_url.trim_end_matches('/');
    let url = format!("{base}/api/v1/auth/refresh");
    let resp = ureq::post(&url)
        .set("content-type", "application/json")
        .send_json(serde_json::json!({ "refresh_token": refresh_token }))
        .context("auth refresh failed")?;
    parse_session(resp)
}

fn parse_session(resp: ureq::Response) -> Result<AuthSession> {
    let status = resp.status();
    let session: AuthSession = resp.into_json().context("parse auth session JSON")?;
    if session.access_token.is_empty() {
        bail!("auth session missing access_token (HTTP {status})");
    }
    Ok(session)
}

/// CI / smoke: env token or AUTH_USER login (no UI).
///
/// Release (x001e): `AUTH_USER` requires `AUTH_PASSWORD` (no seed default).
/// Debug: may default password to `Password1` when only `AUTH_USER` is set.
pub fn try_silent_session(auth_url: &str) -> Result<Option<AuthSession>> {
    if let Ok(token) = std::env::var("ALCHEMY_AUTH_TOKEN") {
        if !token.is_empty() {
            return Ok(Some(AuthSession {
                access_token: token,
                token_type: "Bearer".into(),
                expires_in: 0,
                refresh_token: None,
                user: AuthUser {
                    user_id: "env".into(),
                    username: std::env::var("AUTH_USER").unwrap_or_else(|_| "env".into()),
                    email: String::new(),
                },
            }));
        }
    }
    let Ok(user) = std::env::var("AUTH_USER") else {
        return Ok(None);
    };
    if user.is_empty() {
        return Ok(None);
    }
    let password = match std::env::var("AUTH_PASSWORD") {
        Ok(p) if !p.is_empty() => p,
        _ => {
            #[cfg(debug_assertions)]
            {
                "Password1".into()
            }
            #[cfg(not(debug_assertions))]
            {
                bail!(
                    "AUTH_PASSWORD is required in release when AUTH_USER is set (seed defaults are debug-only)"
                );
            }
        }
    };
    // remember_me so refresh_token is issued (x0021 P1-1 access refresh on silent/smoke).
    Ok(Some(login(auth_url, &user, &password, true)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_gate_clear_resets_join() {
        let mut g = AuthGate::default();
        g.apply_auth_session(AuthSession {
            access_token: "a".into(),
            token_type: "Bearer".into(),
            expires_in: 1,
            refresh_token: Some("r".into()),
            user: AuthUser {
                user_id: "u".into(),
                username: "alice".into(),
                email: "a@b.c".into(),
            },
        });
        g.apply_room("rt".into(), "sub".into(), 300);
        assert!(g.is_authenticated());
        assert!(g.is_joined());
        assert!(g.room_deadline.is_some());
        assert!(g.access_deadline.is_some());
        g.clear_session();
        assert!(!g.is_authenticated());
        assert!(!g.is_joined());
        assert!(g.room_deadline.is_none());
        assert!(g.access_deadline.is_none());
    }

    #[test]
    fn refresh_deadline_none_when_ttl_zero() {
        assert!(refresh_deadline(0).is_none());
    }

    #[test]
    fn refresh_deadline_at_80_percent() {
        let before = Instant::now();
        let d = refresh_deadline(100).expect("deadline");
        let after = Instant::now();
        let wait = d.saturating_duration_since(before);
        assert!(
            wait.as_secs() >= 79 && wait.as_secs() <= 81,
            "wait={wait:?}"
        );
        let _ = after;
    }
}
