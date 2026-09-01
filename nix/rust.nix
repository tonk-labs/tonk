# This module contains helpers for building Rust-based artifacts
# It exists because we're using [crane](https://crane.dev) to do
# the building, and correct crane usage is somewhat nuanced compared
# to the built-in Nix tools (such as buildRustPackage). Using the
# helpers here means you can maximize the amount of sharing / re-use
# of dependencies across Rust projects.
{
  pkgs,
  filter,
  crane,
  workspaceRoot,
  buildInputs,
}:

let
  # Cargo dependencies that are Git repositories need to have their
  # expected build hash recorded separately. We make a shared variable so
  # that the same dependencies can be used across all derivations that
  # need them. Crane expects the full git URL as the key.
  cargoGitDependencies = {
    "git+https://github.com/dialog-db/dialog-db.git?tag=tonk-2026-05-19#9c49c9956028119d7d9293b93e329dc1b84e4999" =
      "sha256-jwKq75n7oPjfNglx+mArc79+Y/v3yRZj4teeNs9iZCw=";
  };

  # Filter source to only Rust-relevant files
  # rustSource = craneLib.cleanCargoSource (craneLib.path workspaceRoot);
  rustSource = filter {
    root = workspaceRoot;
    include = [
      ".cargo"
      "Cargo.lock"
      "Cargo.toml"
      "rust-toolchain.toml"
      "rust"
    ];
  };

  rustToolchain = pkgs.rust-bin.fromRustupToolchainFile (workspaceRoot + "/rust-toolchain.toml");
  craneLib = (crane.mkLib pkgs).overrideToolchain (_: rustToolchain);

  wasm-bindgen-cli =
    with pkgs;
    buildWasmBindgenCli rec {
      # Must match the `wasm-bindgen` crate version in the workspace exactly:
      # the CLI generates the JS glue for the wasm the crate emits, and a
      # mismatch produces bindings that silently do not line up.
      src = fetchCrate {
        pname = "wasm-bindgen-cli";
        version = "0.2.126";
        hash = "sha256-H6Is3fiZVxZCfOMWK5dWMSrtn50VGv0sfdnsT+cTtyk=";
      };

      cargoDeps = rustPlatform.fetchCargoVendor {
        inherit src;
        inherit (src) pname version;
        hash = "sha256-VucqkXbCi4qtQzY/HrXiDnbSURsagPsdNVMn1Tw3UiY=";
      };
    };

  enforce-workspace-deps =
    with pkgs;
    rustPlatform.buildRustPackage rec {
      pname = "cargo-enforce-shared-workspace-deps";
      version = "0.1.0";
      buildInputs = [ rustToolchain ];

      src = fetchCrate {
        inherit pname version;
        sha256 = "sha256-XOdKeg9tNt/HT+WO9QKtdX3fUMUssVTlXRV0LOIMMzc=";
      };

      cargoHash = "sha256-O6DQXK8/VVwTLuFlSyh8jtBJyAFMfAUNXnTeMWrXTCM=";
    };

  nativeBuildInputs = buildInputs ++ [
    rustToolchain
  ];

  # Workspace-wide common attributes
  commonAttributes = {

    src = rustSource;
    strictDeps = true;
    inherit nativeBuildInputs;
    buildInputs =
      with pkgs;
      lib.optionals stdenv.isLinux [
        dbus
      ];

    # Git dependencies with hashes for offline evaluation
    # Crane will automatically find Cargo.lock from src
    outputHashes = cargoGitDependencies;
    doCheck = false;
  };

  # Build native dependencies once for entire workspace
  nativeArtifacts = craneLib.buildDepsOnly (
    commonAttributes
    // {
      pname = "tonk-workspace-deps";
    }
  );

  # Build WASM dependencies separately (different target)
  # Exclude native-only crates that can't compile for wasm32-unknown-unknown.
  # If you add a new native-only crate, add it to the --exclude list here.
  wasmCargoExcludeArgs = "--workspace --exclude tonk-cli";

  wasmAttributes = commonAttributes // {
    CARGO_BUILD_TARGET = "wasm32-unknown-unknown";
    cargoExtraArgs = wasmCargoExcludeArgs;
  };

  wasmArtifacts = craneLib.buildDepsOnly (
    wasmAttributes
    // {
      pname = "tonk-workspace-wasm-deps";
    }
  );

  # Generic crate builder using crane
  buildCrate =
    attributes:
    craneLib.buildPackage (
      commonAttributes
      // {
        version = "0.1.0";

        cargoArtifacts = nativeArtifacts;
      }
      // attributes
    );

  buildWasmCrate =
    attributes:
    craneLib.buildPackage (
      wasmAttributes
      // {
        cargoArtifacts = wasmArtifacts;

        # These *_BIN envvars are an implicit part of the `worker-build` API
        WASM_OPT_BIN = "${pkgs.binaryen}/bin/wasm-opt";
        WASM_BINDGEN_BIN = "${wasm-bindgen-cli}/bin/wasm-bindgen";
        ESBUILD_BIN = "${pkgs.esbuild}/bin/esbuild";
      }
      // attributes
    );

  buildTrunkCrate =
    attributes:
    let
      crateRoot = builtins.dirOf attributes.trunkConfig;
    in
    craneLib.buildTrunkPackage (
      wasmAttributes
      // {
        cargoArtifacts = wasmArtifacts;
        preBuild = ''
          cd ${crateRoot}
        '';
        inherit wasm-bindgen-cli;
      }
      // attributes
    );

  # Build cargo-nextest test archive
  buildTestArchive =
    {
      name,
      args ? "",
      target ? null,
      # Extra cargo args for a dedicated dependency-only build. An archive
      # whose feature set pulls dependencies the shared workspace artifacts
      # never compile (e.g. tonk-ui's `integration-tests` WebDriver stack)
      # names them here so they are cached across source changes instead of
      # rebuilt inside every archive derivation.
      depsExtraArgs ? null,
    }:
    let
      targetAttributes = if target == "wasm32-unknown-unknown" then wasmAttributes else commonAttributes;

      sharedArtifacts = if target == "wasm32-unknown-unknown" then wasmArtifacts else nativeArtifacts;

      targetArtifacts =
        if depsExtraArgs == null then
          sharedArtifacts
        else
          craneLib.buildDepsOnly (
            targetAttributes
            // {
              pname = "tonk-workspace-${name}-deps";
              cargoExtraArgs = depsExtraArgs;
            }
          );
    in
    craneLib.mkCargoDerivation (
      targetAttributes
      // {
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
      }
    );

  cargoChecks = {
    clippy = craneLib.cargoClippy (
      commonAttributes
      // {
        pname = "tonk-clippy-lint";
        cargoArtifacts = nativeArtifacts;
        cargoClippyExtraArgs = "--all-targets --all-features -- -D warnings";
      }
    );

    rustfmt = craneLib.cargoFmt {
      src = rustSource;
      pname = "tonk-fmt-check";
    };

    sharedWorkspaceDeps = buildCrate {
      pname = "shared-workspace-deps-check";
      buildPhase = ''
        ${enforce-workspace-deps}/bin/cargo-enforce-shared-workspace-deps
      '';
      installPhase = ''
        touch $out
      '';
    };
  };

in
{
  inherit
    buildCrate
    buildWasmCrate
    buildTrunkCrate
    buildTestArchive
    rustToolchain
    cargoChecks
    wasm-bindgen-cli
    ;
}
