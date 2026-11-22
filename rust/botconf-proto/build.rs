fn main() -> Result<(), Box<dyn std::error::Error>> {
    // The proto is outside the crate: git dependencies check out the whole
    // repository, so the relative path works for them. Publishing the crate
    // would need the proto copied into it.
    tonic_build::configure()
        .build_server(true)
        // For clients that call a settings service, and botconf-grpc's tests.
        .build_client(true)
        .compile_protos(&["../../proto/botconf/v1/settings.proto"], &["../../proto"])?;
    println!("cargo:rerun-if-changed=../../proto");
    Ok(())
}
