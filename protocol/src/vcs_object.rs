//! Spatial VCS Tree / Commit codec — deterministic CBOR Alchemy profile (x00c8).
//!
//! # Guarantees
//! - Same logical Tree/Commit → same canonical bytes → same blake3 id.
//! - Strict decode rejects non-canonical heads, unknown keys, trailing bytes, and
//!   semantic violations (unsorted/duplicate `local_id`, bad TRS, etc.).
//! - TRS v1 = IEEE754 f32 LE 40 bytes; NaN / Inf / -0.0 rejected on decode;
//!   encode normalizes -0.0 → +0.0.
//! - Alchemy CBOR profile: no negative ints (major 1), no float/tag/null; `created_at_ms`
//!   is a non-negative uint. Root commits require `generation == 1`.
//! - Hostile length args are bounded by remaining input bytes (no OOM on `with_capacity`).
//!
//! # Non-guarantees
//! - assets-server Port pool wiring; DB content-addressed migration; push/fork/PR.
//! - Parent→child generation consistency beyond root (`generation == 1`); → x00c9.
//! - World/Scene trees; f64 TRS; protobuf / JSON Prefab.Payload compatibility.

use std::fmt;

/// Maximum encoded object size (draft §5.4).
pub const MAX_OBJECT_BYTES: usize = 1 << 20;
/// Maximum children in a Tree (SceneBudget order).
pub const MAX_CHILDREN: usize = 4096;
/// Maximum UTF-8 commit message length.
pub const MAX_MESSAGE_BYTES: usize = 4096;
/// Blake3 digest length.
pub const HASH_LEN: usize = 32;
/// TRS v1 packed size: translation[3] + rotation[4] + scale[3] as f32 LE.
pub const TRS_BYTES: usize = 40;

pub const KIND_TREE: u64 = 1;
pub const KIND_COMMIT: u64 = 2;

/// 32-byte content hash (blake3 of canonical object bytes, or a child blob id).
pub type ObjectHash = [u8; HASH_LEN];

#[derive(Debug, Clone, PartialEq)]
pub struct Tree {
    pub children: Vec<TreeChild>,
    pub license: Option<String>,
    pub license_text: Option<ObjectHash>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TreeChild {
    pub local_id: String,
    pub blake3: ObjectHash,
    pub trs: Option<[f32; 10]>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Commit {
    pub tree: ObjectHash,
    pub parents: Vec<ObjectHash>,
    pub generation: u64,
    pub author_sub: String,
    /// Non-negative Unix epoch ms (CBOR unsigned; major 1 forbidden by profile).
    pub created_at_ms: u64,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ObjectKind {
    Tree,
    Commit,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DecodedObject {
    Tree(Tree),
    Commit(Commit),
}

/// Summary for Port / future DB index columns.
#[derive(Debug, Clone, PartialEq)]
pub struct ObjectSummary {
    pub kind: ObjectKind,
    pub hash: ObjectHash,
    pub tree: Option<ObjectHash>,
    pub parents: Vec<ObjectHash>,
    pub generation: Option<u64>,
    pub author_sub: Option<String>,
    /// Commit only (x00cc push: copy into `vcs_commits`).
    pub created_at_ms: Option<u64>,
    /// Commit only (x00cc push: copy into `vcs_commits`).
    pub message: Option<String>,
    pub child_blake3s: Vec<ObjectHash>,
    pub license: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VcsObjectError {
    Empty,
    Oversize { len: usize },
    TrailingBytes,
    UnexpectedEof,
    UnsupportedMajor { major: u8 },
    NonCanonicalHead,
    UnexpectedType { expected: &'static str },
    UnknownKey { key: u64 },
    DuplicateKey { key: u64 },
    MapKeysOutOfOrder,
    InvalidKind { kind: u64 },
    InvalidUtf8,
    LocalIdEmpty,
    LocalIdTooLong,
    LocalIdUnsortedOrDuplicate,
    TooManyChildren { count: usize },
    BadHashLen { len: usize },
    BadHex,
    BadTrsLen { len: usize },
    BadTrsFloat,
    TooManyParents { count: usize },
    DuplicateParent,
    AuthorSubEmpty,
    AuthorSubTooLong,
    AuthorSubForbiddenChar,
    MessageTooLong { len: usize },
    LicenseEmpty,
    LicenseTooLong { len: usize },
    MissingRequired { field: &'static str },
    InvalidGeneration { generation: u64 },
    LengthArgTooLarge { arg: u64 },
    NonCanonicalForm,
    EncodeFailed(&'static str),
}

impl fmt::Display for VcsObjectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "empty object"),
            Self::Oversize { len } => write!(f, "oversize len={len}"),
            Self::TrailingBytes => write!(f, "trailing bytes"),
            Self::UnexpectedEof => write!(f, "unexpected eof"),
            Self::UnsupportedMajor { major } => write!(f, "unsupported major={major}"),
            Self::NonCanonicalHead => write!(f, "non-canonical cbor head"),
            Self::UnexpectedType { expected } => write!(f, "unexpected type (expected {expected})"),
            Self::UnknownKey { key } => write!(f, "unknown key={key}"),
            Self::DuplicateKey { key } => write!(f, "duplicate key={key}"),
            Self::MapKeysOutOfOrder => write!(f, "map keys out of order"),
            Self::InvalidKind { kind } => write!(f, "invalid kind={kind}"),
            Self::InvalidUtf8 => write!(f, "invalid utf-8"),
            Self::LocalIdEmpty => write!(f, "empty local_id"),
            Self::LocalIdTooLong => write!(f, "local_id too long"),
            Self::LocalIdUnsortedOrDuplicate => write!(f, "local_id unsorted or duplicate"),
            Self::TooManyChildren { count } => write!(f, "too many children count={count}"),
            Self::BadHashLen { len } => write!(f, "bad hash len={len}"),
            Self::BadHex => write!(f, "bad hex"),
            Self::BadTrsLen { len } => write!(f, "bad trs len={len}"),
            Self::BadTrsFloat => write!(f, "bad trs float"),
            Self::TooManyParents { count } => write!(f, "too many parents count={count}"),
            Self::DuplicateParent => write!(f, "duplicate parent"),
            Self::AuthorSubEmpty => write!(f, "empty author_sub"),
            Self::AuthorSubTooLong => write!(f, "author_sub too long"),
            Self::AuthorSubForbiddenChar => write!(f, "author_sub contains '|'"),
            Self::MessageTooLong { len } => write!(f, "message too long len={len}"),
            Self::LicenseEmpty => write!(f, "empty license (omit key instead)"),
            Self::LicenseTooLong { len } => write!(f, "license too long len={len}"),
            Self::MissingRequired { field } => write!(f, "missing required {field}"),
            Self::InvalidGeneration { generation } => {
                write!(f, "invalid generation={generation}")
            }
            Self::LengthArgTooLarge { arg } => write!(f, "length arg too large arg={arg}"),
            Self::NonCanonicalForm => write!(f, "non-canonical object form"),
            Self::EncodeFailed(msg) => write!(f, "encode failed: {msg}"),
        }
    }
}

impl std::error::Error for VcsObjectError {}

/// Blake3 of canonical object bytes.
pub fn object_id(bytes: &[u8]) -> ObjectHash {
    *blake3::hash(bytes).as_bytes()
}

pub fn object_id_hex(bytes: &[u8]) -> String {
    hex::encode(object_id(bytes))
}

pub fn hash_to_hex(hash: &ObjectHash) -> String {
    hex::encode(hash)
}

pub fn hash_from_hex(s: &str) -> Result<ObjectHash, VcsObjectError> {
    let down = s.trim().to_ascii_lowercase();
    if down.len() != HASH_LEN * 2 {
        return Err(VcsObjectError::BadHashLen {
            len: down.len() / 2,
        });
    }
    let mut out = [0u8; HASH_LEN];
    hex::decode_to_slice(&down, &mut out).map_err(|_| VcsObjectError::BadHex)?;
    Ok(out)
}

// ---------------------------------------------------------------------------
// Encode
// ---------------------------------------------------------------------------

pub fn encode_tree(tree: &Tree) -> Result<Vec<u8>, VcsObjectError> {
    validate_tree(tree)?;
    let mut w = CborWriter::new();
    let mut pairs: Vec<(u64, Vec<u8>)> = Vec::with_capacity(4);

    pairs.push((0, {
        let mut tmp = CborWriter::new();
        tmp.write_uint(KIND_TREE);
        tmp.into_bytes()
    }));

    pairs.push((1, {
        let mut tmp = CborWriter::new();
        tmp.write_array_header(tree.children.len())?;
        for child in &tree.children {
            encode_child(&mut tmp, child)?;
        }
        tmp.into_bytes()
    }));

    if let Some(license) = &tree.license {
        pairs.push((2, {
            let mut tmp = CborWriter::new();
            tmp.write_tstr(license)?;
            tmp.into_bytes()
        }));
    }
    if let Some(lt) = &tree.license_text {
        pairs.push((3, {
            let mut tmp = CborWriter::new();
            tmp.write_bstr(lt);
            tmp.into_bytes()
        }));
    }

    w.write_map_pairs(&pairs)?;
    let out = w.into_bytes();
    if out.len() > MAX_OBJECT_BYTES {
        return Err(VcsObjectError::Oversize { len: out.len() });
    }
    Ok(out)
}

pub fn encode_commit(commit: &Commit) -> Result<Vec<u8>, VcsObjectError> {
    validate_commit(commit)?;
    let mut w = CborWriter::new();
    let mut pairs: Vec<(u64, Vec<u8>)> = Vec::with_capacity(7);

    pairs.push((0, {
        let mut tmp = CborWriter::new();
        tmp.write_uint(KIND_COMMIT);
        tmp.into_bytes()
    }));
    pairs.push((1, {
        let mut tmp = CborWriter::new();
        tmp.write_bstr(&commit.tree);
        tmp.into_bytes()
    }));
    pairs.push((2, {
        let mut tmp = CborWriter::new();
        tmp.write_array_header(commit.parents.len())?;
        for p in &commit.parents {
            tmp.write_bstr(p);
        }
        tmp.into_bytes()
    }));
    pairs.push((3, {
        let mut tmp = CborWriter::new();
        tmp.write_uint(commit.generation);
        tmp.into_bytes()
    }));
    pairs.push((4, {
        let mut tmp = CborWriter::new();
        tmp.write_tstr(&commit.author_sub)?;
        tmp.into_bytes()
    }));
    pairs.push((5, {
        let mut tmp = CborWriter::new();
        tmp.write_uint(commit.created_at_ms);
        tmp.into_bytes()
    }));
    pairs.push((6, {
        let mut tmp = CborWriter::new();
        tmp.write_tstr(&commit.message)?;
        tmp.into_bytes()
    }));

    w.write_map_pairs(&pairs)?;
    let out = w.into_bytes();
    if out.len() > MAX_OBJECT_BYTES {
        return Err(VcsObjectError::Oversize { len: out.len() });
    }
    Ok(out)
}

fn encode_child(w: &mut CborWriter, child: &TreeChild) -> Result<(), VcsObjectError> {
    let mut pairs: Vec<(u64, Vec<u8>)> = Vec::with_capacity(3);
    pairs.push((0, {
        let mut tmp = CborWriter::new();
        tmp.write_tstr(&child.local_id)?;
        tmp.into_bytes()
    }));
    pairs.push((1, {
        let mut tmp = CborWriter::new();
        tmp.write_bstr(&child.blake3);
        tmp.into_bytes()
    }));
    if let Some(trs) = &child.trs {
        let packed = pack_trs(trs)?;
        pairs.push((2, {
            let mut tmp = CborWriter::new();
            tmp.write_bstr(&packed);
            tmp.into_bytes()
        }));
    }
    w.write_map_pairs(&pairs)
}

// ---------------------------------------------------------------------------
// Decode (strict)
// ---------------------------------------------------------------------------

pub fn strict_decode(bytes: &[u8]) -> Result<DecodedObject, VcsObjectError> {
    if bytes.is_empty() {
        return Err(VcsObjectError::Empty);
    }
    if bytes.len() > MAX_OBJECT_BYTES {
        return Err(VcsObjectError::Oversize { len: bytes.len() });
    }

    let mut r = CborReader::new(bytes);
    let map = r.read_uint_key_map()?;
    if !r.is_eof() {
        return Err(VcsObjectError::TrailingBytes);
    }

    let kind = match map.get(&0) {
        Some(v) => v.as_uint().ok_or(VcsObjectError::UnexpectedType {
            expected: "kind uint",
        })?,
        None => return Err(VcsObjectError::MissingRequired { field: "kind" }),
    };

    let decoded = match kind {
        KIND_TREE => DecodedObject::Tree(decode_tree_map(&map)?),
        KIND_COMMIT => DecodedObject::Commit(decode_commit_map(&map)?),
        other => return Err(VcsObjectError::InvalidKind { kind: other }),
    };

    // Canonical form: re-encode must equal input bytes.
    let reencoded = match &decoded {
        DecodedObject::Tree(t) => encode_tree(t)?,
        DecodedObject::Commit(c) => encode_commit(c)?,
    };
    if reencoded.as_slice() != bytes {
        return Err(VcsObjectError::NonCanonicalForm);
    }
    Ok(decoded)
}

pub fn verify(bytes: &[u8]) -> Result<ObjectSummary, VcsObjectError> {
    let decoded = strict_decode(bytes)?;
    let hash = object_id(bytes);
    Ok(match decoded {
        DecodedObject::Tree(t) => ObjectSummary {
            kind: ObjectKind::Tree,
            hash,
            tree: None,
            parents: Vec::new(),
            generation: None,
            author_sub: None,
            created_at_ms: None,
            message: None,
            child_blake3s: t.children.iter().map(|c| c.blake3).collect(),
            license: t.license,
        },
        DecodedObject::Commit(c) => ObjectSummary {
            kind: ObjectKind::Commit,
            hash,
            tree: Some(c.tree),
            parents: c.parents,
            generation: Some(c.generation),
            author_sub: Some(c.author_sub),
            created_at_ms: Some(c.created_at_ms),
            message: Some(c.message),
            child_blake3s: Vec::new(),
            license: None,
        },
    })
}

fn decode_tree_map(map: &UintKeyMap) -> Result<Tree, VcsObjectError> {
    for key in map.keys() {
        if *key > 3 {
            return Err(VcsObjectError::UnknownKey { key: *key });
        }
    }
    let children_val = map
        .get(&1)
        .ok_or(VcsObjectError::MissingRequired { field: "children" })?;
    let children_arr = children_val
        .as_array()
        .ok_or(VcsObjectError::UnexpectedType {
            expected: "children array",
        })?;
    if children_arr.len() > MAX_CHILDREN {
        return Err(VcsObjectError::TooManyChildren {
            count: children_arr.len(),
        });
    }
    let mut children = Vec::with_capacity(children_arr.len());
    for item in children_arr {
        let child_map = item.as_map().ok_or(VcsObjectError::UnexpectedType {
            expected: "child map",
        })?;
        children.push(decode_child_map(child_map)?);
    }
    let license = match map.get(&2) {
        Some(v) => Some(v.as_tstr()?.to_string()),
        None => None,
    };
    let license_text = match map.get(&3) {
        Some(v) => Some(v.as_hash()?),
        None => None,
    };
    let tree = Tree {
        children,
        license,
        license_text,
    };
    validate_tree(&tree)?;
    Ok(tree)
}

fn decode_child_map(map: &UintKeyMap) -> Result<TreeChild, VcsObjectError> {
    for key in map.keys() {
        if *key > 2 {
            return Err(VcsObjectError::UnknownKey { key: *key });
        }
    }
    let local_id = map
        .get(&0)
        .ok_or(VcsObjectError::MissingRequired { field: "local_id" })?
        .as_tstr()?
        .to_string();
    let blake3 = map
        .get(&1)
        .ok_or(VcsObjectError::MissingRequired { field: "blake3" })?
        .as_hash()?;
    let trs = match map.get(&2) {
        Some(v) => Some(unpack_trs(v.as_bstr()?)?),
        None => None,
    };
    Ok(TreeChild {
        local_id,
        blake3,
        trs,
    })
}

fn decode_commit_map(map: &UintKeyMap) -> Result<Commit, VcsObjectError> {
    for key in map.keys() {
        if *key > 6 {
            return Err(VcsObjectError::UnknownKey { key: *key });
        }
    }
    let tree = map
        .get(&1)
        .ok_or(VcsObjectError::MissingRequired { field: "tree" })?
        .as_hash()?;
    let parents_val = map
        .get(&2)
        .ok_or(VcsObjectError::MissingRequired { field: "parents" })?;
    let parents_arr = parents_val
        .as_array()
        .ok_or(VcsObjectError::UnexpectedType {
            expected: "parents array",
        })?;
    if parents_arr.len() > 2 {
        return Err(VcsObjectError::TooManyParents {
            count: parents_arr.len(),
        });
    }
    let mut parents = Vec::with_capacity(parents_arr.len());
    for p in parents_arr {
        parents.push(p.as_hash()?);
    }
    let generation = map
        .get(&3)
        .ok_or(VcsObjectError::MissingRequired {
            field: "generation",
        })?
        .as_uint()
        .ok_or(VcsObjectError::UnexpectedType {
            expected: "generation uint",
        })?;
    let author_sub = map
        .get(&4)
        .ok_or(VcsObjectError::MissingRequired {
            field: "author_sub",
        })?
        .as_tstr()?
        .to_string();
    let created_at_ms = map
        .get(&5)
        .ok_or(VcsObjectError::MissingRequired {
            field: "created_at_ms",
        })?
        .as_uint()
        .ok_or(VcsObjectError::UnexpectedType {
            expected: "created_at_ms uint",
        })?;
    let message = map
        .get(&6)
        .ok_or(VcsObjectError::MissingRequired { field: "message" })?
        .as_tstr()?
        .to_string();
    let commit = Commit {
        tree,
        parents,
        generation,
        author_sub,
        created_at_ms,
        message,
    };
    validate_commit(&commit)?;
    Ok(commit)
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

fn validate_tree(tree: &Tree) -> Result<(), VcsObjectError> {
    if tree.children.len() > MAX_CHILDREN {
        return Err(VcsObjectError::TooManyChildren {
            count: tree.children.len(),
        });
    }
    let mut prev: Option<&str> = None;
    for child in &tree.children {
        if child.local_id.is_empty() {
            return Err(VcsObjectError::LocalIdEmpty);
        }
        if child.local_id.len() > 255 {
            return Err(VcsObjectError::LocalIdTooLong);
        }
        if let Some(p) = prev {
            if child.local_id.as_bytes() <= p.as_bytes() {
                return Err(VcsObjectError::LocalIdUnsortedOrDuplicate);
            }
        }
        prev = Some(&child.local_id);
        if let Some(trs) = &child.trs {
            let _ = pack_trs(trs)?;
        }
    }
    if let Some(lic) = &tree.license {
        // Optional fields: absent ⇒ omit key. Empty string would dual-encode with omission (§6.5).
        if lic.is_empty() {
            return Err(VcsObjectError::LicenseEmpty);
        }
        if lic.len() > 255 {
            return Err(VcsObjectError::LicenseTooLong { len: lic.len() });
        }
    }
    Ok(())
}

fn validate_commit(commit: &Commit) -> Result<(), VcsObjectError> {
    if commit.parents.len() > 2 {
        return Err(VcsObjectError::TooManyParents {
            count: commit.parents.len(),
        });
    }
    if commit.parents.len() == 2 && commit.parents[0] == commit.parents[1] {
        return Err(VcsObjectError::DuplicateParent);
    }
    // draft §5.3: root generation = 1; generation 0 is never valid.
    if commit.generation == 0 {
        return Err(VcsObjectError::InvalidGeneration {
            generation: commit.generation,
        });
    }
    if commit.parents.is_empty() && commit.generation != 1 {
        return Err(VcsObjectError::InvalidGeneration {
            generation: commit.generation,
        });
    }
    if commit.author_sub.is_empty() {
        return Err(VcsObjectError::AuthorSubEmpty);
    }
    if commit.author_sub.len() > 255 {
        return Err(VcsObjectError::AuthorSubTooLong);
    }
    if commit.author_sub.contains('|') {
        return Err(VcsObjectError::AuthorSubForbiddenChar);
    }
    if commit.message.len() > MAX_MESSAGE_BYTES {
        return Err(VcsObjectError::MessageTooLong {
            len: commit.message.len(),
        });
    }
    Ok(())
}

fn pack_trs(trs: &[f32; 10]) -> Result<[u8; TRS_BYTES], VcsObjectError> {
    let mut out = [0u8; TRS_BYTES];
    for (i, &f) in trs.iter().enumerate() {
        let n = normalize_f32(f)?;
        out[i * 4..(i + 1) * 4].copy_from_slice(&n.to_le_bytes());
    }
    Ok(out)
}

fn unpack_trs(bytes: &[u8]) -> Result<[f32; 10], VcsObjectError> {
    if bytes.len() != TRS_BYTES {
        return Err(VcsObjectError::BadTrsLen { len: bytes.len() });
    }
    let mut out = [0f32; 10];
    for i in 0..10 {
        let mut buf = [0u8; 4];
        buf.copy_from_slice(&bytes[i * 4..(i + 1) * 4]);
        let f = f32::from_le_bytes(buf);
        // Reject non-canonical bits (including -0.0) on decode.
        if !f.is_finite() || f.to_bits() == (-0.0f32).to_bits() {
            return Err(VcsObjectError::BadTrsFloat);
        }
        out[i] = f;
    }
    Ok(out)
}

fn normalize_f32(f: f32) -> Result<f32, VcsObjectError> {
    if !f.is_finite() {
        return Err(VcsObjectError::BadTrsFloat);
    }
    if f.to_bits() == (-0.0f32).to_bits() {
        return Ok(0.0);
    }
    Ok(f)
}

// ---------------------------------------------------------------------------
// Minimal CBOR (Alchemy profile)
// ---------------------------------------------------------------------------

struct CborWriter {
    buf: Vec<u8>,
}

impl CborWriter {
    fn new() -> Self {
        Self {
            buf: Vec::with_capacity(256),
        }
    }

    fn into_bytes(self) -> Vec<u8> {
        self.buf
    }

    fn write_uint(&mut self, v: u64) {
        self.write_head(0, v);
    }

    fn write_bstr(&mut self, bytes: &[u8]) {
        self.write_head(2, bytes.len() as u64);
        self.buf.extend_from_slice(bytes);
    }

    fn write_tstr(&mut self, s: &str) -> Result<(), VcsObjectError> {
        if s.len() > u32::MAX as usize {
            return Err(VcsObjectError::EncodeFailed("tstr too long"));
        }
        self.write_head(3, s.len() as u64);
        self.buf.extend_from_slice(s.as_bytes());
        Ok(())
    }

    fn write_array_header(&mut self, len: usize) -> Result<(), VcsObjectError> {
        self.write_head(4, len as u64);
        Ok(())
    }

    fn write_map_pairs(&mut self, pairs: &[(u64, Vec<u8>)]) -> Result<(), VcsObjectError> {
        // Caller supplies ascending unique keys.
        for w in pairs.windows(2) {
            if w[0].0 >= w[1].0 {
                return Err(VcsObjectError::MapKeysOutOfOrder);
            }
        }
        self.write_head(5, pairs.len() as u64);
        for (k, v) in pairs {
            self.write_uint(*k);
            self.buf.extend_from_slice(v);
        }
        Ok(())
    }

    fn write_head(&mut self, major: u8, arg: u64) {
        let mt = major << 5;
        if arg <= 23 {
            self.buf.push(mt | (arg as u8));
        } else if arg <= u8::MAX as u64 {
            self.buf.push(mt | 24);
            self.buf.push(arg as u8);
        } else if arg <= u16::MAX as u64 {
            self.buf.push(mt | 25);
            self.buf.extend_from_slice(&(arg as u16).to_be_bytes());
        } else if arg <= u32::MAX as u64 {
            self.buf.push(mt | 26);
            self.buf.extend_from_slice(&(arg as u32).to_be_bytes());
        } else {
            self.buf.push(mt | 27);
            self.buf.extend_from_slice(&arg.to_be_bytes());
        }
    }
}

#[derive(Debug)]
enum CborValue {
    Uint(u64),
    Bstr(Vec<u8>),
    Tstr(String),
    Array(Vec<CborValue>),
    Map(UintKeyMap),
}

type UintKeyMap = std::collections::BTreeMap<u64, CborValue>;

impl CborValue {
    fn as_uint(&self) -> Option<u64> {
        match self {
            Self::Uint(v) => Some(*v),
            _ => None,
        }
    }

    fn as_bstr(&self) -> Result<&[u8], VcsObjectError> {
        match self {
            Self::Bstr(b) => Ok(b),
            _ => Err(VcsObjectError::UnexpectedType { expected: "bstr" }),
        }
    }

    fn as_hash(&self) -> Result<ObjectHash, VcsObjectError> {
        let b = self.as_bstr()?;
        if b.len() != HASH_LEN {
            return Err(VcsObjectError::BadHashLen { len: b.len() });
        }
        let mut out = [0u8; HASH_LEN];
        out.copy_from_slice(b);
        Ok(out)
    }

    fn as_tstr(&self) -> Result<&str, VcsObjectError> {
        match self {
            Self::Tstr(s) => Ok(s),
            _ => Err(VcsObjectError::UnexpectedType { expected: "tstr" }),
        }
    }

    fn as_array(&self) -> Option<&[CborValue]> {
        match self {
            Self::Array(a) => Some(a),
            _ => None,
        }
    }

    fn as_map(&self) -> Option<&UintKeyMap> {
        match self {
            Self::Map(m) => Some(m),
            _ => None,
        }
    }
}

struct CborReader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> CborReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    fn is_eof(&self) -> bool {
        self.pos >= self.data.len()
    }

    fn read_uint_key_map(&mut self) -> Result<UintKeyMap, VcsObjectError> {
        match self.read_value()? {
            CborValue::Map(m) => Ok(m),
            _ => Err(VcsObjectError::UnexpectedType { expected: "map" }),
        }
    }

    fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.pos)
    }

    /// Bound a CBOR length/count arg by remaining bytes (`min_bytes_per` × n).
    /// bstr/tstr/array items need ≥1 byte; map entries need key+value ≥2 bytes.
    fn bounded_len(&self, arg: u64, min_bytes_per: usize) -> Result<usize, VcsObjectError> {
        let n = usize::try_from(arg).map_err(|_| VcsObjectError::LengthArgTooLarge { arg })?;
        let need = n
            .checked_mul(min_bytes_per)
            .ok_or(VcsObjectError::LengthArgTooLarge { arg })?;
        if need > self.remaining() {
            return Err(VcsObjectError::LengthArgTooLarge { arg });
        }
        Ok(n)
    }

    fn read_value(&mut self) -> Result<CborValue, VcsObjectError> {
        let (major, arg) = self.read_head()?;
        match major {
            0 => Ok(CborValue::Uint(arg)),
            // Alchemy profile (§6.5): negative integers forbidden.
            1 => Err(VcsObjectError::UnsupportedMajor { major: 1 }),
            2 => {
                let n = self.bounded_len(arg, 1)?;
                let bytes = self.read_exact(n)?;
                Ok(CborValue::Bstr(bytes.to_vec()))
            }
            3 => {
                let n = self.bounded_len(arg, 1)?;
                let bytes = self.read_exact(n)?;
                let s = std::str::from_utf8(bytes).map_err(|_| VcsObjectError::InvalidUtf8)?;
                Ok(CborValue::Tstr(s.to_string()))
            }
            4 => {
                let n = self.bounded_len(arg, 1)?;
                let mut items = Vec::with_capacity(n);
                for _ in 0..n {
                    items.push(self.read_value()?);
                }
                Ok(CborValue::Array(items))
            }
            5 => {
                let n = self.bounded_len(arg, 2)?;
                let mut map = UintKeyMap::new();
                let mut last_key: Option<u64> = None;
                for _ in 0..n {
                    let key_val = self.read_value()?;
                    let key = key_val.as_uint().ok_or(VcsObjectError::UnexpectedType {
                        expected: "uint map key",
                    })?;
                    if let Some(prev) = last_key {
                        if key <= prev {
                            return Err(VcsObjectError::MapKeysOutOfOrder);
                        }
                    }
                    last_key = Some(key);
                    let value = self.read_value()?;
                    if map.insert(key, value).is_some() {
                        return Err(VcsObjectError::DuplicateKey { key });
                    }
                }
                Ok(CborValue::Map(map))
            }
            other => Err(VcsObjectError::UnsupportedMajor { major: other }),
        }
    }

    fn read_head(&mut self) -> Result<(u8, u64), VcsObjectError> {
        if self.pos >= self.data.len() {
            return Err(VcsObjectError::UnexpectedEof);
        }
        let b = self.data[self.pos];
        self.pos += 1;
        let major = b >> 5;
        let ai = b & 0x1f;
        let arg = match ai {
            0..=23 => ai as u64,
            24 => {
                let x = self.read_exact(1)?[0] as u64;
                if x <= 23 {
                    return Err(VcsObjectError::NonCanonicalHead);
                }
                x
            }
            25 => {
                let raw = self.read_exact(2)?;
                let x = u16::from_be_bytes([raw[0], raw[1]]) as u64;
                if x <= u8::MAX as u64 {
                    return Err(VcsObjectError::NonCanonicalHead);
                }
                x
            }
            26 => {
                let raw = self.read_exact(4)?;
                let x = u32::from_be_bytes([raw[0], raw[1], raw[2], raw[3]]) as u64;
                if x <= u16::MAX as u64 {
                    return Err(VcsObjectError::NonCanonicalHead);
                }
                x
            }
            27 => {
                let raw = self.read_exact(8)?;
                let mut buf = [0u8; 8];
                buf.copy_from_slice(raw);
                let x = u64::from_be_bytes(buf);
                if x <= u32::MAX as u64 {
                    return Err(VcsObjectError::NonCanonicalHead);
                }
                x
            }
            _ => return Err(VcsObjectError::NonCanonicalHead), // indefinite / reserved
        };
        Ok((major, arg))
    }

    fn read_exact(&mut self, n: usize) -> Result<&'a [u8], VcsObjectError> {
        let end = self
            .pos
            .checked_add(n)
            .ok_or(VcsObjectError::UnexpectedEof)?;
        if end > self.data.len() {
            return Err(VcsObjectError::UnexpectedEof);
        }
        let slice = &self.data[self.pos..end];
        self.pos = end;
        Ok(slice)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_hash(seed: u8) -> ObjectHash {
        [seed; HASH_LEN]
    }

    fn sample_tree() -> Tree {
        Tree {
            children: vec![
                TreeChild {
                    local_id: "hat".into(),
                    blake3: sample_hash(0x11),
                    trs: Some([1.0, 2.0, 3.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0]),
                },
                TreeChild {
                    local_id: "pin".into(),
                    blake3: sample_hash(0x22),
                    trs: None,
                },
            ],
            license: Some("CC-BY-4.0".into()),
            license_text: None,
        }
    }

    fn sample_commit(tree_hash: ObjectHash) -> Commit {
        Commit {
            tree: tree_hash,
            parents: vec![],
            generation: 1,
            author_sub: "user-123".into(),
            created_at_ms: 1_763_172_000_000,
            message: "帽子の色を変更".into(),
        }
    }

    #[test]
    fn tree_round_trip_stable_hash() {
        let tree = sample_tree();
        let bytes = encode_tree(&tree).expect("encode");
        let hash1 = object_id_hex(&bytes);
        let decoded = strict_decode(&bytes).expect("decode");
        match decoded {
            DecodedObject::Tree(t) => {
                assert_eq!(t, tree);
                let bytes2 = encode_tree(&t).unwrap();
                assert_eq!(bytes, bytes2);
                assert_eq!(hash1, object_id_hex(&bytes2));
            }
            _ => panic!("expected tree"),
        }
    }

    #[test]
    fn commit_round_trip() {
        let tree_bytes = encode_tree(&sample_tree()).unwrap();
        let tree_hash = object_id(&tree_bytes);
        let commit = sample_commit(tree_hash);
        let bytes = encode_commit(&commit).unwrap();
        match strict_decode(&bytes).unwrap() {
            DecodedObject::Commit(c) => assert_eq!(c, commit),
            _ => panic!("expected commit"),
        }
    }

    #[test]
    fn rejects_unsorted_children() {
        let mut tree = sample_tree();
        tree.children.swap(0, 1);
        assert!(matches!(
            encode_tree(&tree),
            Err(VcsObjectError::LocalIdUnsortedOrDuplicate)
        ));
    }

    #[test]
    fn rejects_non_canonical_map_head() {
        let bytes = encode_tree(&sample_tree()).unwrap();
        assert_eq!(bytes[0], 0xa3, "sample tree is a 3-key map");
        let mut bad = bytes.clone();
        // Non-shortest length: 0xb8 0x03 instead of 0xa3.
        bad[0] = 0xb8;
        bad.insert(1, 0x03);
        assert!(matches!(
            strict_decode(&bad),
            Err(VcsObjectError::NonCanonicalHead)
        ));
    }

    #[test]
    fn rejects_unknown_tree_key() {
        // map(3): kind=1, children=[], key 99 = 0
        let bytes = hex::decode("a300010180186300").unwrap();
        assert!(matches!(
            strict_decode(&bytes),
            Err(VcsObjectError::UnknownKey { key: 99 })
        ));
    }

    #[test]
    fn rejects_negative_int_major() {
        // map(1): key 0 = negative int -1 (major 1, arg 0) — profile forbidden
        let bytes = hex::decode("a10020").unwrap();
        assert!(matches!(
            strict_decode(&bytes),
            Err(VcsObjectError::UnsupportedMajor { major: 1 })
        ));
    }

    #[test]
    fn rejects_huge_array_length_without_oom() {
        // kind=1, children = array claiming 0xffffffff items (no payload)
        let bytes = hex::decode("a20001019affffffff").unwrap();
        assert!(matches!(
            strict_decode(&bytes),
            Err(VcsObjectError::LengthArgTooLarge { .. })
        ));
    }

    #[test]
    fn rejects_trailing_bytes() {
        let mut bytes = encode_tree(&sample_tree()).unwrap();
        bytes.push(0x00);
        assert!(matches!(
            strict_decode(&bytes),
            Err(VcsObjectError::TrailingBytes)
        ));
    }

    #[test]
    fn rejects_root_generation_not_one() {
        let mut c = sample_commit(sample_hash(1));
        c.generation = 2;
        assert!(matches!(
            encode_commit(&c),
            Err(VcsObjectError::InvalidGeneration { generation: 2 })
        ));
    }

    #[test]
    fn rejects_generation_zero() {
        let mut c = sample_commit(sample_hash(1));
        c.parents = vec![sample_hash(2)];
        c.generation = 0;
        assert!(matches!(
            encode_commit(&c),
            Err(VcsObjectError::InvalidGeneration { generation: 0 })
        ));
    }

    #[test]
    fn rejects_nan_trs_on_encode() {
        let mut tree = sample_tree();
        tree.children[0].trs = Some([f32::NAN; 10]);
        assert!(matches!(
            encode_tree(&tree),
            Err(VcsObjectError::BadTrsFloat)
        ));
    }

    #[test]
    fn encode_normalizes_neg_zero() {
        let mut tree = sample_tree();
        let mut trs = [0.0f32; 10];
        trs[0] = -0.0;
        trs[6] = 1.0;
        trs[7] = 1.0;
        trs[8] = 1.0;
        trs[9] = 1.0;
        tree.children[0].trs = Some(trs);
        let bytes = encode_tree(&tree).unwrap();
        match strict_decode(&bytes).unwrap() {
            DecodedObject::Tree(t) => {
                let packed = pack_trs(t.children[0].trs.as_ref().unwrap()).unwrap();
                assert_eq!(&packed[0..4], &0.0f32.to_le_bytes());
            }
            _ => panic!("tree"),
        }
    }

    #[test]
    fn rejects_author_pipe() {
        let mut c = sample_commit(sample_hash(1));
        c.author_sub = "a|b".into();
        assert!(matches!(
            encode_commit(&c),
            Err(VcsObjectError::AuthorSubForbiddenChar)
        ));
    }

    #[test]
    fn verify_summary_tree() {
        let bytes = encode_tree(&sample_tree()).unwrap();
        let summary = verify(&bytes).unwrap();
        assert_eq!(summary.kind, ObjectKind::Tree);
        assert_eq!(summary.child_blake3s.len(), 2);
        assert_eq!(summary.license.as_deref(), Some("CC-BY-4.0"));
        assert_eq!(summary.hash, object_id(&bytes));
    }

    fn load_golden(name: &str) -> (Vec<u8>, String) {
        let dir = format!("{}/testdata/vcs_object", env!("CARGO_MANIFEST_DIR"));
        let hex = std::fs::read_to_string(format!("{dir}/{name}.cbor.hex"))
            .unwrap()
            .trim()
            .to_string();
        let hash = std::fs::read_to_string(format!("{dir}/{name}.blake3"))
            .unwrap()
            .trim()
            .to_string();
        let bytes = hex::decode(&hex).expect("golden hex");
        (bytes, hash)
    }

    #[test]
    fn golden_tree_minimal() {
        let (bytes, expect_hash) = load_golden("tree_minimal");
        assert_eq!(object_id_hex(&bytes), expect_hash);
        match strict_decode(&bytes).unwrap() {
            DecodedObject::Tree(t) => {
                assert_eq!(t.children.len(), 1);
                assert_eq!(t.children[0].local_id, "a");
                assert_eq!(encode_tree(&t).unwrap(), bytes);
            }
            _ => panic!("expected tree"),
        }
    }

    #[test]
    fn golden_tree_licensed() {
        let (bytes, expect_hash) = load_golden("tree_licensed");
        assert_eq!(object_id_hex(&bytes), expect_hash);
        match strict_decode(&bytes).unwrap() {
            DecodedObject::Tree(t) => {
                assert_eq!(t.license.as_deref(), Some("CC-BY-4.0"));
                assert_eq!(t.children.len(), 2);
                assert_eq!(encode_tree(&t).unwrap(), bytes);
            }
            _ => panic!("expected tree"),
        }
    }

    #[test]
    fn golden_commit_root() {
        let (bytes, expect_hash) = load_golden("commit_root");
        assert_eq!(object_id_hex(&bytes), expect_hash);
        match strict_decode(&bytes).unwrap() {
            DecodedObject::Commit(c) => {
                assert_eq!(c.generation, 1);
                assert_eq!(c.author_sub, "user-123");
                assert_eq!(c.message, "init");
                assert!(c.parents.is_empty());
                assert_eq!(encode_commit(&c).unwrap(), bytes);
            }
            _ => panic!("expected commit"),
        }
    }

    #[test]
    fn rejects_duplicate_parent() {
        let mut c = sample_commit(sample_hash(1));
        let p = sample_hash(9);
        c.parents = vec![p, p];
        assert!(matches!(
            encode_commit(&c),
            Err(VcsObjectError::DuplicateParent)
        ));
    }

    #[test]
    fn rejects_empty_object() {
        assert!(matches!(strict_decode(&[]), Err(VcsObjectError::Empty)));
    }

    #[test]
    fn rejects_empty_license() {
        let mut tree = sample_tree();
        tree.license = Some(String::new());
        assert!(matches!(
            encode_tree(&tree),
            Err(VcsObjectError::LicenseEmpty)
        ));
    }

    #[test]
    fn rejects_huge_map_length_with_two_byte_floor() {
        // Canonical map head for 64 pairs (0xb8 0x40) + only 10 payload bytes.
        // 1-byte floor would allow n=64; 2-byte floor requires ≥128 → LengthArgTooLarge.
        let mut bytes = vec![0xb8, 0x40];
        bytes.extend_from_slice(&[0u8; 10]);
        assert!(matches!(
            strict_decode(&bytes),
            Err(VcsObjectError::LengthArgTooLarge { arg: 64 })
        ));
    }
}
