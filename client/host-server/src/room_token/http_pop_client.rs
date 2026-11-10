//! Client-side HTTP PoP headers for world host APIs (x0081).
//!
//! Guarantees: signs `alchemy/pop/v1|…` with host Token `cnf.spk` key; path is
//! `/api/…` without query.
//! Non-guarantees: clock sync beyond world skew window.

use alchemy_protocol::http_pop::{sign_pop, POP_SIG_HEADER, POP_TS_HEADER};
use alchemy_protocol::wire_crypto::SigningKey;
use base64::Engine;
use std::time::{SystemTime, UNIX_EPOCH};

/// Extract request path (`/api/…`) from an absolute URL (no query).
pub(super) fn request_path(url: &str) -> &str {
    let path = if let Some(i) = url.find("/api/") {
        &url[i..]
    } else if let Some(scheme) = url.find("://") {
        let after = &url[scheme + 3..];
        match after.find('/') {
            Some(j) => &after[j..],
            None => "/",
        }
    } else {
        url
    };
    path.split('?').next().unwrap_or(path)
}

pub(super) fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Attach PoP headers to a ureq request (body must match bytes that will be sent).
pub(super) fn with_pop(
    req: ureq::Request,
    key: &SigningKey,
    method: &str,
    path: &str,
    body: &[u8],
) -> ureq::Request {
    let ts = now_unix_secs();
    let sig = sign_pop(key, method, path, ts, body);
    let sig_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(sig);
    req.set(POP_TS_HEADER, &ts.to_string())
        .set(POP_SIG_HEADER, &sig_b64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_path_strips_origin_and_query() {
        assert_eq!(
            request_path("http://127.0.0.1:4003/api/v1/rooms/r/host_keys"),
            "/api/v1/rooms/r/host_keys"
        );
        assert_eq!(
            request_path("http://127.0.0.1:4003/api/v1/rooms/r/scene?x=1"),
            "/api/v1/rooms/r/scene"
        );
    }
}
