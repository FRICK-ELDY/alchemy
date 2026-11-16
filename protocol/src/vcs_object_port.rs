//! JSON Port contract for `vcs-object` (x00c8).
//!
//! # Guarantees
//! - `verify` / `encode_tree` / `encode_commit` share [`crate::vcs_object`] encode/verify.
//! - TRS in JSON is **base64 of f32 LE 40 bytes only** (no JSON number arrays — avoids
//!   f64→f32 bit drift vs canonical CBOR).
//!
//! # Non-guarantees
//! - assets-server Port pool wiring.

use crate::vcs_object::{
    encode_commit, encode_tree, hash_from_hex, hash_to_hex, object_id_hex, verify, Commit,
    ObjectHash, ObjectKind, Tree, TreeChild, MAX_OBJECT_BYTES, TRS_BYTES,
};
use serde_json::{json, Value};

/// Handle one JSON request body (oneshot or `--serve` payload).
pub fn handle_json(raw: &str) -> Result<String, String> {
    let v: Value = serde_json::from_str(raw.trim()).map_err(|e| format!("json: {e}"))?;
    let op = v
        .get("op")
        .and_then(|x| x.as_str())
        .ok_or_else(|| "missing op".to_string())?;
    match op {
        "verify" => op_verify(&v),
        "encode_tree" => op_encode_tree(&v),
        "encode_commit" => op_encode_commit(&v),
        other => Err(format!("unknown op={other}")),
    }
}

fn op_verify(v: &Value) -> Result<String, String> {
    let b64 = v
        .get("bytes_b64")
        .and_then(|x| x.as_str())
        .ok_or_else(|| "missing bytes_b64".to_string())?;
    let bytes = b64_decode(b64)?;
    if bytes.len() > MAX_OBJECT_BYTES {
        return Err(format!("oversize len={}", bytes.len()));
    }
    let summary = verify(&bytes).map_err(|e| e.to_string())?;
    let kind = match summary.kind {
        ObjectKind::Tree => "tree",
        ObjectKind::Commit => "commit",
    };
    Ok(json!({
        "ok": true,
        "kind": kind,
        "hash_hex": hash_to_hex(&summary.hash),
        "tree_hex": summary.tree.as_ref().map(hash_to_hex),
        "parents_hex": summary.parents.iter().map(hash_to_hex).collect::<Vec<_>>(),
        "generation": summary.generation,
        "author_sub": summary.author_sub,
        "child_blake3s_hex": summary.child_blake3s.iter().map(hash_to_hex).collect::<Vec<_>>(),
        "license": summary.license,
    })
    .to_string())
}

fn op_encode_tree(v: &Value) -> Result<String, String> {
    let tree = parse_tree(v)?;
    let bytes = encode_tree(&tree).map_err(|e| e.to_string())?;
    Ok(json!({
        "ok": true,
        "kind": "tree",
        "bytes_b64": b64_encode(&bytes),
        "hash_hex": object_id_hex(&bytes),
    })
    .to_string())
}

fn op_encode_commit(v: &Value) -> Result<String, String> {
    let commit = parse_commit(v)?;
    let bytes = encode_commit(&commit).map_err(|e| e.to_string())?;
    Ok(json!({
        "ok": true,
        "kind": "commit",
        "bytes_b64": b64_encode(&bytes),
        "hash_hex": object_id_hex(&bytes),
    })
    .to_string())
}

fn parse_tree(v: &Value) -> Result<Tree, String> {
    let children_v = v
        .get("children")
        .and_then(|x| x.as_array())
        .ok_or_else(|| "missing children".to_string())?;
    let mut children = Vec::with_capacity(children_v.len());
    for c in children_v {
        let local_id = c
            .get("local_id")
            .and_then(|x| x.as_str())
            .ok_or_else(|| "child.local_id".to_string())?
            .to_string();
        let blake3 = parse_hash_field(c, "blake3")?;
        let trs = match c.get("trs") {
            None | Some(Value::Null) => None,
            Some(Value::String(b64)) => {
                let raw = b64_decode(b64)?;
                if raw.len() != TRS_BYTES {
                    return Err(format!("trs b64 len={}", raw.len()));
                }
                let mut out = [0f32; 10];
                for i in 0..10 {
                    let mut buf = [0u8; 4];
                    buf.copy_from_slice(&raw[i * 4..(i + 1) * 4]);
                    out[i] = f32::from_le_bytes(buf);
                }
                Some(out)
            }
            Some(Value::Array(_)) => {
                return Err(
                    "trs must be base64 of f32 LE 40 bytes (JSON number arrays rejected)".into(),
                );
            }
            Some(_) => return Err("trs must be base64 string".into()),
        };
        children.push(TreeChild {
            local_id,
            blake3,
            trs,
        });
    }
    let license = match v.get("license") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => Some(s.clone()),
        Some(_) => return Err("license must be string or omitted".into()),
    };
    let license_text = match v.get("license_text") {
        None | Some(Value::Null) => None,
        Some(Value::String(_)) => Some(parse_hash_field(v, "license_text")?),
        Some(_) => return Err("license_text must be hex string or omitted".into()),
    };
    Ok(Tree {
        children,
        license,
        license_text,
    })
}

fn parse_commit(v: &Value) -> Result<Commit, String> {
    let tree = parse_hash_field(v, "tree")?;
    let parents = match v.get("parents") {
        None => Vec::new(),
        Some(Value::Array(arr)) => {
            let mut out = Vec::with_capacity(arr.len());
            for p in arr {
                let hex = p.as_str().ok_or_else(|| "parent not string".to_string())?;
                out.push(hash_from_hex(hex).map_err(|e| e.to_string())?);
            }
            out
        }
        Some(_) => return Err("parents must be array".into()),
    };
    let generation = v
        .get("generation")
        .and_then(|x| x.as_u64())
        .ok_or_else(|| "missing generation".to_string())?;
    let author_sub = v
        .get("author_sub")
        .and_then(|x| x.as_str())
        .ok_or_else(|| "missing author_sub".to_string())?
        .to_string();
    let created_at_ms = v
        .get("created_at_ms")
        .and_then(|x| x.as_u64())
        .ok_or_else(|| "missing created_at_ms (non-negative uint)".to_string())?;
    let message = v
        .get("message")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    Ok(Commit {
        tree,
        parents,
        generation,
        author_sub,
        created_at_ms,
        message,
    })
}

fn parse_hash_field(v: &Value, key: &str) -> Result<ObjectHash, String> {
    let s = v
        .get(key)
        .and_then(|x| x.as_str())
        .ok_or_else(|| format!("missing {key}"))?;
    hash_from_hex(s).map_err(|e| e.to_string())
}

fn b64_encode(bytes: &[u8]) -> String {
    base64::Engine::encode(&base64::engine::general_purpose::STANDARD, bytes)
}

fn b64_decode(s: &str) -> Result<Vec<u8>, String> {
    base64::Engine::decode(&base64::engine::general_purpose::STANDARD, s)
        .map_err(|e| format!("b64: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vcs_object::{object_id_hex, Tree, TreeChild};

    #[test]
    fn encode_tree_roundtrip_via_json() {
        let zero = "0".repeat(64);
        let req =
            format!(r#"{{"op":"encode_tree","children":[{{"local_id":"a","blake3":"{zero}"}}]}}"#);
        let out = handle_json(&req).unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["ok"], true);
        assert_eq!(v["kind"], "tree");
        assert_eq!(
            v["hash_hex"].as_str().unwrap(),
            "632fa5342fbe640eed8d3a7903b2fa23b7e7e5fc4a8899021021c4422295d324"
        );
        let b64 = v["bytes_b64"].as_str().unwrap();
        let verify_req = format!(r#"{{"op":"verify","bytes_b64":"{b64}"}}"#);
        let verified = handle_json(&verify_req).unwrap();
        let vv: Value = serde_json::from_str(&verified).unwrap();
        assert_eq!(vv["ok"], true);
        assert_eq!(vv["kind"], "tree");
        assert_eq!(vv["hash_hex"], v["hash_hex"]);
    }

    #[test]
    fn rejects_trs_json_array() {
        let zero = "0".repeat(64);
        let req = format!(
            r#"{{"op":"encode_tree","children":[{{"local_id":"a","blake3":"{zero}","trs":[1,0,0,0,0,0,1,1,1,1]}}]}}"#
        );
        let err = handle_json(&req).unwrap_err();
        assert!(err.contains("base64"), "{err}");
    }

    #[test]
    fn encode_tree_trs_b64_matches_library() {
        let mut trs = [0f32; 10];
        trs[0] = 1.0;
        trs[6] = 1.0;
        trs[7] = 1.0;
        trs[8] = 1.0;
        trs[9] = 1.0;
        let mut packed = [0u8; TRS_BYTES];
        for (i, f) in trs.iter().enumerate() {
            packed[i * 4..(i + 1) * 4].copy_from_slice(&f.to_le_bytes());
        }
        let trs_b64 = b64_encode(&packed);
        let blake = "11".repeat(32);
        let req = format!(
            r#"{{"op":"encode_tree","children":[{{"local_id":"hat","blake3":"{blake}","trs":"{trs_b64}"}}]}}"#
        );
        let out = handle_json(&req).unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        let lib = encode_tree(&Tree {
            children: vec![TreeChild {
                local_id: "hat".into(),
                blake3: hash_from_hex(&blake).unwrap(),
                trs: Some(trs),
            }],
            license: None,
            license_text: None,
        })
        .unwrap();
        assert_eq!(v["hash_hex"].as_str().unwrap(), object_id_hex(&lib));
    }

    #[test]
    fn rejects_bad_op() {
        assert!(handle_json(r#"{"op":"nope"}"#)
            .unwrap_err()
            .contains("unknown op"));
    }

    #[test]
    fn rejects_non_string_license() {
        let zero = "0".repeat(64);
        let req = format!(
            r#"{{"op":"encode_tree","children":[{{"local_id":"a","blake3":"{zero}"}}],"license":1}}"#
        );
        let err = handle_json(&req).unwrap_err();
        assert!(err.contains("license must be string"), "{err}");
    }
}
