fn main() -> Result<(), Box<dyn std::error::Error>> {
    // The proto lives in the crate, so the crate builds wherever it is
    // copied on its own (a vendored dependency, a published crate).
    tonic_build::configure()
        .build_server(true)
        // For clients that call a settings service, and botconf-grpc's tests.
        .build_client(true)
        .compile_protos(&["proto/botconf/v1/settings.proto"], &["proto"])?;
    println!("cargo:rerun-if-changed=proto");
    Ok(())
}
