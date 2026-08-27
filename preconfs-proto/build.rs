fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Vendored protoc: building this crate must not need protobuf installed.
    let protoc = protoc_bin_vendored::protoc_bin_path()?;
    // SAFETY: build scripts run single-threaded before any user code.
    unsafe {
        std::env::set_var("PROTOC", protoc);
    }
    println!("cargo:rerun-if-changed=proto/preconfs.proto");
    tonic_prost_build::configure()
        // bytes::Bytes for the transaction payloads: no copy on decode.
        .bytes(".")
        .build_client(true)
        .build_server(false)
        .compile_protos(&["proto/preconfs.proto"], &["proto"])?;
    Ok(())
}
