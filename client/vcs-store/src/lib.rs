//! Local Spatial VCS store + HTTP push client (x00cd / draft §7.1·§18.6).
//!
//! # Guarantees
//! - Content-addressed local objects / blobs; hash mismatch → error.
//! - `commit` encodes Tree/Commit via `alchemy-protocol::vcs_object`.
//! - `push` runs negotiate → upload missing blobs → push (Bearer JWT).
//! - Push CAS `old_commit_hash` is remote-tracking tip (`refs/remotes/origin/<branch>`),
//!   updated after success; not HEAD's first parent. After a successful server
//!   push, tip mismatch → [`VcsStoreError::RemoteTipMismatch`] (repair tracking
//!   to **request** tip / `expected`, not the mismatched response body) and
//!   tracking IO failure → [`VcsStoreError::TrackingUpdateFailed`].
//!
//! # Non-guarantees
//! - clone / fetch / pull / branch UI; Save UI; shared-sim conversion; SSH.

#![forbid(unsafe_code)]

mod error;
mod remote;
mod store;

pub use error::VcsStoreError;
pub use remote::{HttpRemote, MockRemote, NegotiateRequest, NegotiateResult, PushObject, PushRequest, PushResult, Remote, RemoteRef};
pub use store::{CommitRequest, CommitResult, Store, TreeChildSpec};
