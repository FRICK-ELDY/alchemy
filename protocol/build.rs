fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=proto/render_frame.proto");
    println!("cargo:rerun-if-changed=proto/input_events.proto");
    println!("cargo:rerun-if-changed=proto/scene_manifest.proto");
    println!("cargo:rerun-if-changed=proto/presence.proto");
    let protoc = protoc_bin_vendored::protoc_bin_path()?;
    std::env::set_var("PROTOC", protoc);
    prost_build::compile_protos(
        &[
            "proto/render_frame.proto",
            "proto/input_events.proto",
            "proto/scene_manifest.proto",
            "proto/presence.proto",
        ],
        &["proto/"],
    )?;
    Ok(())
}
