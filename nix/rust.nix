# This module contains helpers for building Rust-based artifacts
# It exists because we're using [crane](https://crane.dev) to do
# the building, and correct crane usage is somewhat nuanced compared
# to the built-in Nix tools (such as buildRustPackage). Using the
# helpers here means you can maximize the amount of sharing / re-use
# of dependencies across Rust projects.
{ pkgs
, filter
, crane
, workspaceRoot
, buildInputs
}:

let
  # Cargo dependencies that are Git repositories need to have their
  # expected build hash recorded separately. We make a shared variable so
  # that the same dependencies can be used across all derivations that
  # need them. Crane expects the full git URL as the key.
  cargoGitDependencies = {
    "git+https://github.com/dialog-db/dialog-db.git?branch=tonk-rewire#5804dd37c3e492ccd7ddd6f96e5d29edc87c9efb" = "sha256-wVImYyZnww23y4ebLV7zGO38O4HDED31Z8BDIChJwBg=";
    "git+https://github.com/tonk-labs/rs-ucan.git?branch=jackddouglas/feat/check#671a0256621eb4656b42d9e631108da3ec18158b" = "sha256-5KQ7wIXv7PHgd6y1pq0+aUU/VFW7BLxECmVUNk1JfGw=";
  };

  # Filter source to only Rust-relevant files
  # rustSource = craneLib.cleanCargoSource (craneLib.path workspaceRoot);
  rustSource = filter {
    root = workspaceRoot;
    include = [
      "Cargo.lock"
      "Cargo.toml"
      "rust-toolchain.toml"
      "rust"
    ];
  };

  rustToolchain = pkgs.rust-bin.fromRustupToolchainFile (workspaceRoot + "/rust-toolchain.toml");
  craneLib = (crane.mkLib pkgs).overrideToolchain (_: rustToolchain);

  nativeBuildInputs = buildInputs ++ [
    rustToolchain
  ];

  # Workspace-wide common attributes
  commonAttributes = {

    src = rustSource;
    strictDeps = true;
    inherit nativeBuildInputs;

    # Git dependencies with hashes for offline evaluation
    # Crane will automatically find Cargo.lock from src
    outputHashes = cargoGitDependencies;
    doCheck = false;
  };

  # Build native dependencies once for entire workspace
  nativeArtifacts = craneLib.buildDepsOnly (commonAttributes // {
    pname = "tonk-workspace-deps";
  });

  # Build WASM dependencies separately (different target)
  wasmAttributes = commonAttributes // {
    CARGO_BUILD_TARGET = "wasm32-unknown-unknown";
  };

  wasmArtifacts = craneLib.buildDepsOnly (wasmAttributes // {
    pname = "tonk-workspace-wasm-deps";
  });

  # Generic crate builder using crane
  buildCrate = attributes: craneLib.buildPackage (commonAttributes // {
    version = "0.1.0";
    cargoArtifacts = nativeArtifacts;
  } // attributes);


  buildTrunkCrate = attributes:
    let
      crateRoot = builtins.dirOf attributes.trunkConfig;
    in
    craneLib.buildTrunkPackage (
      wasmAttributes // {
        cargoArtifacts = wasmArtifacts;
        preBuild = ''
          cd ${crateRoot}
        '';
      } // attributes
    );

  # Build cargo-nextest test archive
  buildTestArchive = { name, args ? "", target ? null }:
    let
      targetAttributes =
        if target == "wasm32-unknown-unknown" then wasmAttributes else commonAttributes;

      targetArtifacts = if target == "wasm32-unknown-unknown" then wasmArtifacts else nativeArtifacts;
    in
    craneLib.mkCargoDerivation (targetAttributes // {
      pname = "tests-${name}";
      cargoArtifacts = targetArtifacts;

      buildPhaseCargoCommand = ''
        cargo nextest archive \
          ${args} \
          --archive-file ./tests-${name}.tar.zst
      '';

      installPhaseCommand = ''
        mkdir -p $out
        cp ./*.tar.zst $out/
      '';

      doInstallCargoArtifacts = false;
      nativeBuildInputs = (targetAttributes.nativeBuildInputs or [ ]) ++ [ pkgs.cargo-nextest ];
    });

  cargoChecks = {
    clippy = craneLib.cargoClippy (commonAttributes // {
      pname = "tonk-clippy-lint";
      cargoArtifacts = nativeArtifacts;
      cargoClippyExtraArgs = "--all-targets --all-features -- -D warnings";
    });

    rustfmt = craneLib.cargoFmt {
      src = rustSource;
      pname = "tonk-fmt-check";
    };
  };

in
{
  inherit buildCrate buildTrunkCrate buildTestArchive rustToolchain cargoChecks;
}
