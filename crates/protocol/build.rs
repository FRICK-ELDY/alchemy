fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=proto/render_frame.proto");
    let protoc = protoc_bin_vendored::protoc_bin_path()?;
    std::env::set_var("PROTOC", protoc);
    prost_build::compile_protos(&["proto/render_frame.proto"], &["proto/"])?;
    Ok(())
}
