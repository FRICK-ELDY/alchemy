//! One-shot helper to print golden Tree/Commit hex + blake3 (x00c8).
//! Not part of the public API — used when refreshing testdata.

fn main() {
    use alchemy_protocol::vcs_object::*;

    let tree = Tree {
        children: vec![TreeChild {
            local_id: "a".into(),
            blake3: [0u8; 32],
            trs: None,
        }],
        license: None,
        license_text: None,
    };
    let tree_bytes = encode_tree(&tree).unwrap();
    println!("TREE_HEX={}", hex::encode(&tree_bytes));
    println!("TREE_HASH={}", object_id_hex(&tree_bytes));

    let tree2 = Tree {
        children: vec![
            TreeChild {
                local_id: "hat".into(),
                blake3: [0x11; 32],
                trs: Some([1.0, 2.0, 3.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0]),
            },
            TreeChild {
                local_id: "pin".into(),
                blake3: [0x22; 32],
                trs: None,
            },
        ],
        license: Some("CC-BY-4.0".into()),
        license_text: None,
    };
    let t2 = encode_tree(&tree2).unwrap();
    let th = object_id(&t2);
    println!("TREE2_HEX={}", hex::encode(&t2));
    println!("TREE2_HASH={}", object_id_hex(&t2));

    let commit = Commit {
        tree: th,
        parents: vec![],
        generation: 1,
        author_sub: "user-123".into(),
        created_at_ms: 1_763_172_000_000,
        message: "init".into(),
    };
    let cb = encode_commit(&commit).unwrap();
    println!("COMMIT_HEX={}", hex::encode(&cb));
    println!("COMMIT_HASH={}", object_id_hex(&cb));
}
