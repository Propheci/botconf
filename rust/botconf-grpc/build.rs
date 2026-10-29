fn main() -> Result<(), Box<dyn std::error::Error>> {
    // The proto is outside the crate: git dependencies check out the whole
    // repository, so the relative path works for them. Publishing the crate
    // would need the proto copied into it.
    tonic_build::configure()
        .build_server(true)
        // For the tests, and for hosts that call a settings service.
        .build_client(true)
        .compile_protos(&["../../proto/botconf/v1/settings.proto"], &["../../proto"])?;
    println!("cargo:rerun-if-changed=../../proto");
    Ok(())
}
