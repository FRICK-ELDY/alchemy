//! Local content-addressed VCS store (x00cd).

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use alchemy_protocol::vcs_object::{
    self, hash_from_hex, hash_to_hex, Commit, ObjectKind, Tree, TreeChild,
};
use serde::{Deserialize, Serialize};

use crate::error::VcsStoreError;
use crate::remote::{NegotiateRequest, PushObject, PushRequest, PushResult, Remote};

/// Match assets-server `@max_object_hashes` (x00cc / x00cd).
const MAX_OBJECT_HASHES: usize = 512;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Config {
    #[serde(default)]
    remote_base_url: Option<String>,
    #[serde(default)]
    repo_id: Option<String>,
    #[serde(default = "default_branch")]
    default_branch: String,
}

fn default_branch() -> String {
    "main".into()
}

#[derive(Debug)]
pub struct Store {
    root: PathBuf,
    config: Config,
}

#[derive(Debug, Clone)]
pub struct TreeChildSpec {
    pub local_id: String,
    pub blake3_hex: String,
}

#[derive(Debug, Clone)]
pub struct CommitRequest {
    pub children: Vec<TreeChildSpec>,
    pub message: String,
    pub author_sub: String,
    pub created_at_ms: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct CommitResult {
    pub commit_hash: String,
    pub tree_hash: String,
    pub generation: u64,
}

impl Store {
    pub fn init(path: impl AsRef<Path>) -> Result<Self, VcsStoreError> {
        let root = path.as_ref().to_path_buf();
        fs::create_dir_all(root.join("objects"))?;
        fs::create_dir_all(root.join("blobs"))?;
        fs::create_dir_all(root.join("refs").join("heads"))?;
        fs::create_dir_all(root.join("refs").join("remotes").join("origin"))?;
        let config = Config {
            remote_base_url: None,
            repo_id: None,
            default_branch: default_branch(),
        };
        write_json(&root.join("config.json"), &config)?;
        fs::write(root.join("HEAD"), b"ref: refs/heads/main\n")?;
        Ok(Self { root, config })
    }

    pub fn open(path: impl AsRef<Path>) -> Result<Self, VcsStoreError> {
        let root = path.as_ref().to_path_buf();
        let config: Config = read_json(&root.join("config.json"))?;
        Ok(Self { root, config })
    }

    pub fn set_remote(
        &mut self,
        base_url: impl Into<String>,
        repo_id: impl Into<String>,
    ) -> Result<(), VcsStoreError> {
        self.config.remote_base_url = Some(base_url.into());
        self.config.repo_id = Some(repo_id.into());
        write_json(&self.root.join("config.json"), &self.config)
    }

    pub fn repo_id(&self) -> Result<&str, VcsStoreError> {
        self.config
            .repo_id
            .as_deref()
            .ok_or(VcsStoreError::MissingConfig("repo_id"))
    }

    pub fn remote_base_url(&self) -> Result<&str, VcsStoreError> {
        self.config
            .remote_base_url
            .as_deref()
            .ok_or(VcsStoreError::MissingConfig("remote_base_url"))
    }

    pub fn default_branch(&self) -> &str {
        &self.config.default_branch
    }

    pub fn put_object(&self, hash_hex: &str, bytes: &[u8]) -> Result<(), VcsStoreError> {
        let expected = hash_hex.to_ascii_lowercase();
        let actual = hash_to_hex(&vcs_object::object_id(bytes));
        if actual != expected {
            return Err(VcsStoreError::HashMismatch { expected, actual });
        }
        write_cas(&self.root.join("objects"), &expected, bytes)
    }

    pub fn get_object(&self, hash_hex: &str) -> Result<Vec<u8>, VcsStoreError> {
        let hash = hash_hex.to_ascii_lowercase();
        read_cas(&self.root.join("objects"), &hash)
            .ok_or_else(|| VcsStoreError::MissingObject { hash })
    }

    pub fn put_blob(&self, blake3_hex: &str, bytes: &[u8]) -> Result<(), VcsStoreError> {
        let expected = blake3_hex.to_ascii_lowercase();
        let actual = blake3_hex_of(bytes);
        if actual != expected {
            return Err(VcsStoreError::HashMismatch { expected, actual });
        }
        write_cas(&self.root.join("blobs"), &expected, bytes)
    }

    pub fn get_blob(&self, blake3_hex: &str) -> Result<Vec<u8>, VcsStoreError> {
        let hash = blake3_hex.to_ascii_lowercase();
        read_cas(&self.root.join("blobs"), &hash)
            .ok_or_else(|| VcsStoreError::MissingBlob { blake3: hash })
    }

    pub fn head(&self) -> Result<Option<String>, VcsStoreError> {
        let branch = self.current_branch()?;
        let path = self.root.join("refs").join("heads").join(&branch);
        if !path.exists() {
            return Ok(None);
        }
        let raw = fs::read_to_string(path)?;
        let hash = raw.trim();
        if hash.is_empty() {
            Ok(None)
        } else {
            Ok(Some(hash.to_ascii_lowercase()))
        }
    }

    pub fn set_head(&self, commit_hash: &str) -> Result<(), VcsStoreError> {
        let lower = normalize_commit_hash(commit_hash)?;
        let branch = self.current_branch()?;
        let path = self.root.join("refs").join("heads").join(&branch);
        fs::create_dir_all(path.parent().expect("refs/heads"))?;
        fs::write(path, format!("{lower}\n"))?;
        Ok(())
    }

    /// Last known remote tip for `branch` (`refs/remotes/origin/<branch>`).
    ///
    /// Used as push `old_commit_hash` (server CAS). Seed after promote / clone
    /// with [`Self::set_remote_tip`]; updated automatically after successful push.
    pub fn remote_tip(&self, branch: &str) -> Result<Option<String>, VcsStoreError> {
        let path = self.remote_tip_path(branch)?;
        if !path.exists() {
            return Ok(None);
        }
        let raw = fs::read_to_string(path)?;
        let hash = raw.trim();
        if hash.is_empty() {
            Ok(None)
        } else {
            Ok(Some(hash.to_ascii_lowercase()))
        }
    }

    /// Record remote tip for CAS (`None` clears → create-on-push old=nil).
    pub fn set_remote_tip(
        &self,
        branch: &str,
        commit_hash: Option<&str>,
    ) -> Result<(), VcsStoreError> {
        let path = self.remote_tip_path(branch)?;
        match commit_hash {
            None => {
                if path.exists() {
                    fs::remove_file(path)?;
                }
                Ok(())
            }
            Some(hash) => {
                let lower = normalize_commit_hash(hash)?;
                fs::create_dir_all(path.parent().expect("refs/remotes/origin"))?;
                fs::write(path, format!("{lower}\n"))?;
                Ok(())
            }
        }
    }

    fn remote_tip_path(&self, branch: &str) -> Result<PathBuf, VcsStoreError> {
        validate_branch_name(branch)?;
        Ok(self
            .root
            .join("refs")
            .join("remotes")
            .join("origin")
            .join(branch))
    }

    pub fn commit(&self, req: CommitRequest) -> Result<CommitResult, VcsStoreError> {
        let mut children: Vec<TreeChild> = Vec::with_capacity(req.children.len());
        for child in &req.children {
            let blake3 = hash_from_hex(&child.blake3_hex)?;
            children.push(TreeChild {
                local_id: child.local_id.clone(),
                blake3,
                trs: None,
            });
        }
        children.sort_by(|a, b| a.local_id.cmp(&b.local_id));

        let tree_bytes = vcs_object::encode_tree(&Tree {
            children,
            license: None,
            license_text: None,
        })?;
        let tree_hash = hash_to_hex(&vcs_object::object_id(&tree_bytes));
        self.put_object(&tree_hash, &tree_bytes)?;

        let (parents, generation) = match self.head()? {
            Some(parent_hash) => {
                let parent_bytes = self.get_object(&parent_hash)?;
                let summary = vcs_object::verify(&parent_bytes)?;
                let gen = summary.generation.unwrap_or(0).saturating_add(1);
                (vec![hash_from_hex(&parent_hash)?], gen)
            }
            None => (Vec::new(), 1),
        };

        let created_at_ms = req.created_at_ms.unwrap_or_else(now_ms);
        let commit = Commit {
            tree: hash_from_hex(&tree_hash)?,
            parents,
            generation,
            author_sub: req.author_sub,
            created_at_ms,
            message: req.message,
        };
        let commit_bytes = vcs_object::encode_commit(&commit)?;
        let commit_hash = hash_to_hex(&vcs_object::object_id(&commit_bytes));
        self.put_object(&commit_hash, &commit_bytes)?;
        self.set_head(&commit_hash)?;

        Ok(CommitResult {
            commit_hash,
            tree_hash,
            generation,
        })
    }

    /// Negotiate → upload missing blobs → push tip objects.
    ///
    /// `old_commit_hash` is the remote-tracking tip for the current branch
    /// ([`Self::remote_tip`]), not HEAD's first parent. Multi-commit local
    /// stacks therefore CAS against the last successful remote tip.
    pub fn push<R: Remote>(&self, remote: &R, token: &str) -> Result<PushResult, VcsStoreError> {
        let repo_id = self.repo_id()?.to_string();
        let branch = self.current_branch()?;
        let tip = self.head()?.ok_or(VcsStoreError::EmptyHead)?;
        let old_commit_hash = self.remote_tip(&branch)?;
        let (objects, blob_blake3s) =
            self.collect_push_payload(&tip, old_commit_hash.as_deref())?;
        let object_hashes: Vec<String> = objects.iter().map(|(h, _)| h.clone()).collect();

        let neg = remote.negotiate(
            token,
            &repo_id,
            &NegotiateRequest {
                ref_name: branch.clone(),
                old_commit_hash: old_commit_hash.clone(),
                new_commit_hash: tip.clone(),
                object_hashes,
                blob_blake3s: blob_blake3s.clone(),
            },
        )?;

        for blake3 in &neg.missing_blob_blake3s {
            let bytes = self.get_blob(blake3)?;
            let returned = remote.upload_blob(token, &bytes)?;
            if returned.to_ascii_lowercase() != blake3.to_ascii_lowercase() {
                return Err(VcsStoreError::HashMismatch {
                    expected: blake3.clone(),
                    actual: returned,
                });
            }
        }

        let missing: std::collections::HashSet<String> = neg
            .missing_object_hashes
            .iter()
            .map(|h| h.to_ascii_lowercase())
            .collect();

        let push_objects: Vec<PushObject> = objects
            .into_iter()
            .filter(|(hash, _)| missing.contains(&hash.to_ascii_lowercase()))
            .map(|(hash_hex, bytes)| PushObject {
                hash_hex,
                bytes_b64: {
                    use base64::{engine::general_purpose::STANDARD, Engine};
                    STANDARD.encode(&bytes)
                },
            })
            .collect();

        let result = remote.push(
            token,
            &repo_id,
            &PushRequest {
                ref_name: branch.clone(),
                old_commit_hash,
                new_commit_hash: tip.clone(),
                objects: push_objects,
            },
        )?;

        let server_tip = result.vref.commit_hash.to_ascii_lowercase();
        if server_tip != tip {
            return Err(VcsStoreError::RemoteTipMismatch {
                expected: tip,
                actual: server_tip,
            });
        }

        // Server already advanced; tracking IO failure is a distinct error so
        // callers can repair via `set_remote_tip` instead of silent cas_mismatch.
        if let Err(err) = self.set_remote_tip(&branch, Some(&server_tip)) {
            return Err(VcsStoreError::TrackingUpdateFailed {
                tip: server_tip,
                detail: err.to_string(),
            });
        }

        Ok(result)
    }

    /// Objects from `tip` down to (but excluding) `remote_tip`. Missing local
    /// objects fail closed. Caps at [`MAX_OBJECT_HASHES`].
    fn collect_push_payload(
        &self,
        tip: &str,
        remote_tip: Option<&str>,
    ) -> Result<(Vec<(String, Vec<u8>)>, Vec<String>), VcsStoreError> {
        let stop = remote_tip.map(|h| h.to_ascii_lowercase());
        let mut objects = Vec::new();
        let mut blobs = Vec::new();
        let mut queue = vec![tip.to_ascii_lowercase()];
        let mut seen = std::collections::HashSet::new();

        while let Some(hash) = queue.pop() {
            if stop.as_ref() == Some(&hash) {
                continue;
            }
            if !seen.insert(hash.clone()) {
                continue;
            }
            let bytes = self.get_object(&hash)?;
            let summary = vcs_object::verify(&bytes)?;
            objects.push((hash.clone(), bytes));
            if objects.len() > MAX_OBJECT_HASHES {
                return Err(VcsStoreError::TooManyHashes {
                    count: objects.len(),
                });
            }

            match summary.kind {
                ObjectKind::Commit => {
                    if let Some(tree) = summary.tree {
                        queue.push(hash_to_hex(&tree));
                    }
                    for parent in summary.parents {
                        let ph = hash_to_hex(&parent);
                        if stop.as_ref() != Some(&ph) {
                            queue.push(ph);
                        }
                    }
                }
                ObjectKind::Tree => {
                    for child in summary.child_blake3s {
                        blobs.push(hash_to_hex(&child));
                    }
                }
            }
        }

        blobs.sort();
        blobs.dedup();
        if blobs.len() > MAX_OBJECT_HASHES {
            return Err(VcsStoreError::TooManyHashes { count: blobs.len() });
        }
        Ok((objects, blobs))
    }

    fn current_branch(&self) -> Result<String, VcsStoreError> {
        let head = fs::read_to_string(self.root.join("HEAD"))?;
        let line = head.trim();
        let name = if let Some(rest) = line.strip_prefix("ref: refs/heads/") {
            rest.to_string()
        } else {
            self.config.default_branch.clone()
        };
        validate_branch_name(&name)?;
        Ok(name)
    }
}

fn validate_branch_name(name: &str) -> Result<(), VcsStoreError> {
    let ok = !name.is_empty()
        && name.len() <= 255
        && !name.contains("..")
        && !name.contains('/')
        && !name.contains('\\')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.');
    if ok {
        Ok(())
    } else {
        Err(VcsStoreError::InvalidBranch(name.to_string()))
    }
}

fn normalize_commit_hash(hash: &str) -> Result<String, VcsStoreError> {
    let lower = hash.to_ascii_lowercase();
    if lower.len() == 64 && lower.chars().all(|c| c.is_ascii_hexdigit()) {
        Ok(lower)
    } else {
        Err(VcsStoreError::InvalidHash(lower))
    }
}

fn write_cas(root: &Path, hash_hex: &str, bytes: &[u8]) -> Result<(), VcsStoreError> {
    let (dir, file) = cas_parts(hash_hex)?;
    let dir_path = root.join(dir);
    fs::create_dir_all(&dir_path)?;
    let path = dir_path.join(file);
    if path.exists() {
        let existing = fs::read(&path)?;
        if existing != bytes {
            return Err(VcsStoreError::HashMismatch {
                expected: hash_hex.to_string(),
                actual: "content-mismatch".into(),
            });
        }
        return Ok(());
    }
    fs::write(path, bytes)?;
    Ok(())
}

fn read_cas(root: &Path, hash_hex: &str) -> Option<Vec<u8>> {
    let (dir, file) = cas_parts(hash_hex).ok()?;
    fs::read(root.join(dir).join(file)).ok()
}

fn cas_parts(hash_hex: &str) -> Result<(&str, &str), VcsStoreError> {
    if hash_hex.len() != 64 {
        return Err(VcsStoreError::InvalidHash(hash_hex.to_string()));
    }
    Ok((&hash_hex[..2], &hash_hex[2..]))
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), VcsStoreError> {
    let text = serde_json::to_string_pretty(value)?;
    fs::write(path, text)?;
    Ok(())
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, VcsStoreError> {
    let text = fs::read_to_string(path)?;
    Ok(serde_json::from_str(&text)?)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn blake3_hex_of(bytes: &[u8]) -> String {
    let dig = blake3::hash(bytes);
    let mut arr = [0u8; 32];
    arr.copy_from_slice(dig.as_bytes());
    hash_to_hex(&arr)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote::MockRemote;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_root(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("alchemy-vcs-store-{label}-{nanos}"));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    fn author() -> String {
        "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa".into()
    }

    fn commit_mesh(store: &Store, mesh_hex: &str, message: &str, ms: u64) -> CommitResult {
        store
            .commit(CommitRequest {
                children: vec![TreeChildSpec {
                    local_id: "c0".into(),
                    blake3_hex: mesh_hex.to_string(),
                }],
                message: message.into(),
                author_sub: author(),
                created_at_ms: Some(ms),
            })
            .expect("commit")
    }

    #[test]
    fn multi_commit_push_uses_remote_tracking_old() {
        let root = temp_root("multi");
        let mut store = Store::init(&root).expect("init");
        store
            .set_remote("http://127.0.0.1:9", "repo-test")
            .expect("remote");

        let mesh = b"mesh-bytes-for-push";
        let mesh_hex = blake3_hex_of(mesh);
        store.put_blob(&mesh_hex, mesh).expect("blob");

        let c1 = commit_mesh(&store, &mesh_hex, "c1", 1_700_000_000_000);
        let remote = MockRemote {
            missing_objects: vec![c1.tree_hash.clone(), c1.commit_hash.clone()],
            missing_blobs: vec![mesh_hex.clone()],
            ..MockRemote::default()
        };
        store.push(&remote, "token").expect("push1");
        let p1 = remote.last_push.borrow().clone().expect("p1");
        assert_eq!(p1.old_commit_hash, None);
        assert_eq!(p1.new_commit_hash, c1.commit_hash);
        assert_eq!(
            store.remote_tip("main").unwrap().as_deref(),
            Some(c1.commit_hash.as_str())
        );

        let c2 = commit_mesh(&store, &mesh_hex, "c2", 1_700_000_000_001);
        let c3 = commit_mesh(&store, &mesh_hex, "c3", 1_700_000_000_002);
        let remote2 = MockRemote {
            missing_objects: vec![
                c2.tree_hash.clone(),
                c2.commit_hash.clone(),
                c3.tree_hash.clone(),
                c3.commit_hash.clone(),
            ],
            missing_blobs: vec![],
            ..MockRemote::default()
        };
        store.push(&remote2, "token").expect("push2");
        let p2 = remote2.last_push.borrow().clone().expect("p2");
        assert_eq!(p2.old_commit_hash.as_deref(), Some(c1.commit_hash.as_str()));
        assert_eq!(p2.new_commit_hash, c3.commit_hash);
        assert_eq!(p2.ref_name, "main");
        // Joining delta: c2+c3 commits (+ shared tree). c1 excluded by remote tip.
        assert_eq!(p2.objects.len(), 3);
        assert_eq!(
            store.remote_tip("main").unwrap().as_deref(),
            Some(c3.commit_hash.as_str())
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn put_object_rejects_hash_mismatch() {
        let root = temp_root("mismatch");
        let store = Store::init(&root).expect("init");
        let err = store
            .put_object(&"00".repeat(32), b"nope")
            .expect_err("mismatch");
        assert!(matches!(err, VcsStoreError::HashMismatch { .. }));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn rejects_path_traversal_branch_name() {
        assert!(matches!(
            validate_branch_name("../evil"),
            Err(VcsStoreError::InvalidBranch(_))
        ));
        assert!(matches!(validate_branch_name("main"), Ok(())));
    }

    #[test]
    fn set_head_rejects_invalid_hash() {
        let root = temp_root("bad-head");
        let store = Store::init(&root).expect("init");
        let err = store.set_head("short").expect_err("bad hash");
        assert!(matches!(err, VcsStoreError::InvalidHash(_)));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn push_remote_tip_mismatch_does_not_update_tracking() {
        let root = temp_root("tip-mismatch");
        let mut store = Store::init(&root).expect("init");
        store
            .set_remote("http://127.0.0.1:9", "repo-test")
            .expect("remote");
        let mesh = b"mesh-mismatch";
        let mesh_hex = blake3_hex_of(mesh);
        store.put_blob(&mesh_hex, mesh).expect("blob");
        let c1 = commit_mesh(&store, &mesh_hex, "c1", 1);
        let fake_tip = "ff".repeat(32);
        let remote = MockRemote {
            missing_objects: vec![c1.tree_hash.clone(), c1.commit_hash.clone()],
            missing_blobs: vec![mesh_hex.clone()],
            override_push_tip: Some(fake_tip.clone()),
            ..MockRemote::default()
        };
        let err = store.push(&remote, "token").expect_err("mismatch");
        let msg = err.to_string();
        assert!(msg.contains("Some(expected)"));
        assert!(msg.contains("do not trust response tip"));
        assert!(msg.contains(&fake_tip));
        match err {
            VcsStoreError::RemoteTipMismatch { expected, actual } => {
                assert_eq!(expected, c1.commit_hash);
                assert_eq!(actual, fake_tip);
            }
            other => panic!("unexpected {other:?}"),
        }
        assert!(store.remote_tip("main").unwrap().is_none());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn push_tracking_update_failed_reports_server_tip() {
        let root = temp_root("track-fail");
        let mut store = Store::init(&root).expect("init");
        store
            .set_remote("http://127.0.0.1:9", "repo-test")
            .expect("remote");
        let mesh = b"mesh-track-fail";
        let mesh_hex = blake3_hex_of(mesh);
        store.put_blob(&mesh_hex, mesh).expect("blob");
        let c1 = commit_mesh(&store, &mesh_hex, "c1", 1);

        // Turn remotes/origin into a file so set_remote_tip cannot mkdir.
        let origin = root.join("refs").join("remotes").join("origin");
        let _ = fs::remove_dir_all(&origin);
        fs::write(&origin, b"not-a-directory").expect("file");

        let remote = MockRemote {
            missing_objects: vec![c1.tree_hash.clone(), c1.commit_hash.clone()],
            missing_blobs: vec![mesh_hex.clone()],
            ..MockRemote::default()
        };
        let err = store.push(&remote, "token").expect_err("tracking");
        match err {
            VcsStoreError::TrackingUpdateFailed { tip, .. } => {
                assert_eq!(tip, c1.commit_hash);
            }
            other => panic!("unexpected {other:?}"),
        }
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn base64_padding_roundtrip_for_elixir() {
        let samples: &[&[u8]] = &[b"", b"a", b"ab", b"abc", b"mesh-bytes-for-push"];
        for s in samples {
            let enc = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, s);
            let dec = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &enc)
                .expect("decode");
            assert_eq!(&dec, s);
            assert_eq!(enc.len() % 4, 0);
        }
    }
}
