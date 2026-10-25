# Crane-based checks: dependencies are built once in their own derivation, so
# changing the crates' code doesn't rebuild them. The crates are libraries, so
# there is no package to build, only checks.
{
  lib,
  craneLib,
  # A toolchain whose rustfmt understands the unstable options of rustfmt.toml.
  fmtToolchain,
}:

let
  # There is no root `[package]` to read these from.
  pname = "botconf";
  version = "0.1.0";

  # Rust sources, TOML files (incl. rustfmt.toml) and Cargo.lock only.
  src = craneLib.cleanCargoSource ../.;

  commonArgs = {
    inherit src pname version;
    strictDeps = true;
  };

  cargoArtifacts = craneLib.buildDepsOnly commonArgs;
in
{
  checks = {
    clippy = craneLib.cargoClippy (commonArgs // {
      inherit cargoArtifacts;
      cargoClippyExtraArgs = "--workspace --all-targets --all-features -- --deny warnings";
    });

    tests = craneLib.cargoTest (commonArgs // {
      inherit cargoArtifacts;
      cargoTestExtraArgs = "--workspace --all-features";
    });

    rustfmt = (craneLib.overrideToolchain fmtToolchain).cargoFmt {
      inherit src pname version;
    };

    taplo = craneLib.taploFmt {
      inherit pname version;
      src = lib.sources.sourceFilesBySuffices src [ ".toml" ];
    };
  };
}
