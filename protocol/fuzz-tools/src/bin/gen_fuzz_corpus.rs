//! One-shot seed writer for `protocol/fuzz/corpus` (x004c).
//!
//! Not a cargo-fuzz target — lives in `protocol/fuzz-tools` so
//! `cargo fuzz list` / `cargo fuzz run` cannot pick it up by mistake.
//!
//! ```bash
//! cargo run --bin gen_fuzz_corpus --manifest-path protocol/fuzz-tools/Cargo.toml
//! ```

use alchemy_protocol::{
    InputEvent, Message, PresenceState, RenderFrame, SceneManifest, PROTOCOL_PIN,
};
use std::fs;
use std::path::PathBuf;

fn main() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fuzz/corpus");
    for dir in [
        "decode_manifest",
        "decode_frame",
        "decode_presence",
        "decode_input",
    ] {
        fs::create_dir_all(root.join(dir)).expect("mkdir");
    }

    let manifest = SceneManifest {
        room_id: "local".into(),
        revision: 1,
        protocol_pin: PROTOCOL_PIN.into(),
        host_generation: 1,
        ..Default::default()
    };
    let frame = RenderFrame {
        tick: 1,
        ..Default::default()
    };
    let presence = PresenceState {
        revision: 1,
        protocol_pin: PROTOCOL_PIN.into(),
        host_generation: 1,
        ..Default::default()
    };
    let input = InputEvent {
        room_token: "a.b.c".into(),
        session_pub: vec![0u8; 32],
        intent_sig: vec![0u8; 64],
        ..Default::default()
    };

    write(
        &root,
        "decode_manifest/seed_empty.pb",
        &SceneManifest::default().encode_to_vec(),
    );
    write(
        &root,
        "decode_manifest/seed_minimal.pb",
        &manifest.encode_to_vec(),
    );
    write(
        &root,
        "decode_frame/seed_empty.pb",
        &RenderFrame::default().encode_to_vec(),
    );
    write(&root, "decode_frame/seed_minimal.pb", &frame.encode_to_vec());
    write(
        &root,
        "decode_presence/seed_empty.pb",
        &PresenceState::default().encode_to_vec(),
    );
    write(
        &root,
        "decode_presence/seed_minimal.pb",
        &presence.encode_to_vec(),
    );
    write(
        &root,
        "decode_input/seed_empty.pb",
        &InputEvent::default().encode_to_vec(),
    );
    write(&root, "decode_input/seed_minimal.pb", &input.encode_to_vec());
    write(&root, "decode_frame/seed_junk.bin", b"\xff\x00\xabalchemy");
    write(
        &root,
        "decode_manifest/seed_junk.bin",
        b"\x08\xff\xff\xff\xff\x0f",
    );
    let shown = root.canonicalize().unwrap_or_else(|_| root.clone());
    eprintln!("wrote seeds under {}", shown.display());
}

fn write(root: &PathBuf, rel: &str, bytes: &[u8]) {
    fs::write(root.join(rel), bytes).unwrap_or_else(|e| panic!("write {rel}: {e}"));
}
