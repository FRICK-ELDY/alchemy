//! Remote negotiate / blob upload / push (x00cd).

use serde::{Deserialize, Serialize};

use crate::error::VcsStoreError;

#[derive(Debug, Clone, Serialize)]
pub struct NegotiateRequest {
    pub ref_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub old_commit_hash: Option<String>,
    pub new_commit_hash: String,
    pub object_hashes: Vec<String>,
    pub blob_blake3s: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NegotiateResult {
    pub missing_object_hashes: Vec<String>,
    pub missing_blob_blake3s: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PushObject {
    pub hash_hex: String,
    pub bytes_b64: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PushRequest {
    pub ref_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub old_commit_hash: Option<String>,
    pub new_commit_hash: String,
    pub objects: Vec<PushObject>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RemoteRef {
    pub ref_id: String,
    pub repo_id: String,
    pub name: String,
    pub kind: String,
    pub commit_hash: String,
    pub protected: bool,
    pub is_frozen: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PushResult {
    #[serde(rename = "ref")]
    pub vref: RemoteRef,
}

#[derive(Debug, Clone, Deserialize)]
struct ErrorBody {
    error: String,
}

/// Transport used by [`crate::Store::push`].
pub trait Remote {
    fn negotiate(
        &self,
        token: &str,
        repo_id: &str,
        req: &NegotiateRequest,
    ) -> Result<NegotiateResult, VcsStoreError>;

    fn upload_blob(&self, token: &str, bytes: &[u8]) -> Result<String, VcsStoreError>;

    fn push(
        &self,
        token: &str,
        repo_id: &str,
        req: &PushRequest,
    ) -> Result<PushResult, VcsStoreError>;
}

/// ureq client against assets-server `/api/v1`.
pub struct HttpRemote {
    base_url: String,
}

impl HttpRemote {
    pub fn new(base_url: impl Into<String>) -> Self {
        let mut base = base_url.into();
        while base.ends_with('/') {
            base.pop();
        }
        Self { base_url: base }
    }

    fn auth_request(&self, method: &str, path: &str, token: &str) -> ureq::Request {
        let url = format!("{}{}", self.base_url, path);
        ureq::request(method, &url).set("Authorization", &format!("Bearer {token}"))
    }

    fn read_json<T: for<'de> Deserialize<'de>>(
        resp: Result<ureq::Response, ureq::Error>,
    ) -> Result<T, VcsStoreError> {
        match resp {
            Ok(r) => r.into_json::<T>().map_err(|e| VcsStoreError::Remote(e.to_string())),
            Err(ureq::Error::Status(code, resp)) => {
                let body = resp.into_string().unwrap_or_default();
                let msg = serde_json::from_str::<ErrorBody>(&body)
                    .map(|e| e.error)
                    .unwrap_or(body);
                Err(VcsStoreError::HttpStatus {
                    status: code,
                    body: msg,
                })
            }
            Err(e) => Err(VcsStoreError::Remote(e.to_string())),
        }
    }
}

impl Remote for HttpRemote {
    fn negotiate(
        &self,
        token: &str,
        repo_id: &str,
        req: &NegotiateRequest,
    ) -> Result<NegotiateResult, VcsStoreError> {
        let path = format!("/api/v1/vcs/repos/{repo_id}/negotiate");
        let resp = self
            .auth_request("POST", &path, token)
            .set("Content-Type", "application/json")
            .send_json(ureq::json!(req));
        Self::read_json(resp)
    }

    fn upload_blob(&self, token: &str, bytes: &[u8]) -> Result<String, VcsStoreError> {
        let resp = self
            .auth_request("POST", "/api/v1/blobs", token)
            .set("Content-Type", "application/octet-stream")
            .send_bytes(bytes);
        #[derive(Deserialize)]
        struct UploadResp {
            blake3: String,
        }
        let parsed: UploadResp = Self::read_json(resp)?;
        Ok(parsed.blake3)
    }

    fn push(
        &self,
        token: &str,
        repo_id: &str,
        req: &PushRequest,
    ) -> Result<PushResult, VcsStoreError> {
        let path = format!("/api/v1/vcs/repos/{repo_id}/push");
        let resp = self
            .auth_request("POST", &path, token)
            .set("Content-Type", "application/json")
            .send_json(ureq::json!(req));
        Self::read_json(resp)
    }
}

/// In-memory remote for unit tests (no network).
#[derive(Debug, Default)]
pub struct MockRemote {
    pub negotiate_calls: std::cell::RefCell<usize>,
    pub push_calls: std::cell::RefCell<usize>,
    pub uploaded_blobs: std::cell::RefCell<Vec<Vec<u8>>>,
    pub last_push: std::cell::RefCell<Option<PushRequest>>,
    pub missing_objects: Vec<String>,
    pub missing_blobs: Vec<String>,
    /// When set, push response tip differs from request (tests RemoteTipMismatch).
    pub override_push_tip: Option<String>,
}

impl Remote for MockRemote {
    fn negotiate(
        &self,
        _token: &str,
        _repo_id: &str,
        _req: &NegotiateRequest,
    ) -> Result<NegotiateResult, VcsStoreError> {
        *self.negotiate_calls.borrow_mut() += 1;
        Ok(NegotiateResult {
            missing_object_hashes: self.missing_objects.clone(),
            missing_blob_blake3s: self.missing_blobs.clone(),
        })
    }

    fn upload_blob(&self, _token: &str, bytes: &[u8]) -> Result<String, VcsStoreError> {
        self.uploaded_blobs.borrow_mut().push(bytes.to_vec());
        let dig = blake3::hash(bytes);
        Ok(dig.to_hex().to_string())
    }

    fn push(
        &self,
        _token: &str,
        repo_id: &str,
        req: &PushRequest,
    ) -> Result<PushResult, VcsStoreError> {
        *self.push_calls.borrow_mut() += 1;
        *self.last_push.borrow_mut() = Some(req.clone());
        let commit_hash = self
            .override_push_tip
            .clone()
            .unwrap_or_else(|| req.new_commit_hash.clone());
        Ok(PushResult {
            vref: RemoteRef {
                ref_id: "ref-test".into(),
                repo_id: repo_id.into(),
                name: req.ref_name.clone(),
                kind: "branch".into(),
                commit_hash,
                protected: false,
                is_frozen: false,
            },
        })
    }
}
