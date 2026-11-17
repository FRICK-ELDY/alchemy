use std::fmt;

use alchemy_protocol::vcs_object::VcsObjectError;

#[derive(Debug)]
pub enum VcsStoreError {
    Io(std::io::Error),
    Json(serde_json::Error),
    Codec(VcsObjectError),
    HashMismatch { expected: String, actual: String },
    MissingObject { hash: String },
    MissingBlob { blake3: String },
    MissingConfig(&'static str),
    InvalidHash(String),
    EmptyHead,
    TooManyHashes { count: usize },
    InvalidBranch(String),
    /// Push response `commit_hash` (`actual`) ≠ local new tip (`expected`).
    /// Tracking was **not** updated. Honest assets-server advances the ref to the
    /// request `new_commit_hash` (= `expected`); repair with
    /// `set_remote_tip(branch, Some(expected))` (or re-fetch tip later). Do **not**
    /// trust `actual` for tracking — a corrupt/mismatched body must not rewrite it.
    RemoteTipMismatch { expected: String, actual: String },
    /// Server tip advanced to `tip`, but local remote-tracking write failed.
    /// Caller must repair with `set_remote_tip(branch, Some(tip))` or next push
    /// will `cas_mismatch`.
    TrackingUpdateFailed { tip: String, detail: String },
    Remote(String),
    HttpStatus { status: u16, body: String },
}

impl fmt::Display for VcsStoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "io: {e}"),
            Self::Json(e) => write!(f, "json: {e}"),
            Self::Codec(e) => write!(f, "codec: {e}"),
            Self::HashMismatch { expected, actual } => {
                write!(f, "hash mismatch expected={expected} actual={actual}")
            }
            Self::MissingObject { hash } => write!(f, "missing object {hash}"),
            Self::MissingBlob { blake3 } => write!(f, "missing blob {blake3}"),
            Self::MissingConfig(k) => write!(f, "missing config {k}"),
            Self::InvalidHash(h) => write!(f, "invalid hash {h}"),
            Self::EmptyHead => write!(f, "empty HEAD"),
            Self::TooManyHashes { count } => write!(f, "too many object hashes ({count})"),
            Self::InvalidBranch(name) => write!(f, "invalid branch name {name:?}"),
            Self::RemoteTipMismatch { expected, actual } => {
                write!(
                    f,
                    "push response tip {actual} ≠ local new tip {expected}; \
                     tracking not updated — repair via set_remote_tip(…, Some(expected)) \
                     (honest server advanced to request tip; do not trust response tip)"
                )
            }
            Self::TrackingUpdateFailed { tip, detail } => {
                write!(
                    f,
                    "server tip advanced to {tip} but tracking update failed: {detail}"
                )
            }
            Self::Remote(msg) => write!(f, "remote: {msg}"),
            Self::HttpStatus { status, body } => write!(f, "http {status}: {body}"),
        }
    }
}

impl std::error::Error for VcsStoreError {}

impl From<std::io::Error> for VcsStoreError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<serde_json::Error> for VcsStoreError {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value)
    }
}

impl From<VcsObjectError> for VcsStoreError {
    fn from(value: VcsObjectError) -> Self {
        Self::Codec(value)
    }
}
