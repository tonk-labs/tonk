{
  description = "Tonk";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    fenix = {
      url = "github:nix-community/fenix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    crane.url = "github:ipetkov/crane";
  };

  outputs =
    {
      self,
      nixpkgs,
      flake-utils,
      fenix,
      crane,
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
        fenixPkgs = fenix.packages.${system};

        rustToolchainStable = fenixPkgs.fromToolchainFile {
          file = ./rust-toolchain.toml;
          sha256 = "sha256-sqSWJDUxc+zaz1nBWMAJKTAGBuGWP25GCftIOlCEAtA=";
        };

        rustToolchainNightly = fenixPkgs.fromToolchainFile {
          file = ./rust-toolchain-nightly.toml;
          sha256 = pkgs.lib.fakeHash;
        };

        # Set up crane with the fenix toolchain
        craneLib = (crane.mkLib pkgs).overrideToolchain (_: rustToolchainStable);

        # Source filtering for Rust builds
        src = craneLib.cleanCargoSource ./.;

        # Vendor dependencies with git dependency hashes
        cargoVendorDir = craneLib.vendorCargoDeps {
          inherit src;
          outputHashes = {
            "git+https://github.com/tonk-labs/samod?branch=wasm-runtime#fe92f4d6fbb53fe107b1f4d9eea3fe5da7a30322" =
              "sha256-0mr/mtsnm+BZHlQLPEfe+wmzWjPldcULSvOzCOf5yMc=";
          };
        };

        # Shared build dependencies for both Nix builds and dev shells
        sharedBuildInputs = with pkgs; [
          openssl
        ];

        sharedNativeBuildInputs = with pkgs; [
          pkg-config
        ];

        # Common arguments for crane builds
        commonArgs = {
          inherit src cargoVendorDir;
          pname = "tonk";
          version = "0.1.0";
          strictDeps = true;

          nativeBuildInputs = sharedNativeBuildInputs;
          buildInputs = sharedBuildInputs;
        };

        # Build dependencies only (for caching)
        cargoArtifacts = craneLib.buildDepsOnly commonArgs;

        # Common build inputs for all dev shells
        commonBuildInputs =
          with pkgs;
          [
            rustToolchainStable
            bun
          ]
          ++ sharedBuildInputs
          ++ sharedNativeBuildInputs
          ++ lib.optionals stdenv.isLinux [
            # Linux-specific inputs
          ]
          ++ lib.optionals stdenv.isDarwin [
            # MacOS-specific inputs
          ];

        commands = {
          "build" = {
            description = "Builds all of Tonk";
            command = "cargo build";
          };
          "build:web" = {
            description = "Builds the Tonk web application";
            command = "echo 'TODO'";
          };
          "test:all" = {
            description = "Runs the full test suite";
            command = ''
              echo "Installing Node.js dependencies for sync tests..."
              (cd rust/tonk-core/examples/server && bun install --frozen-lockfile)
              (cd rust/tonk-core/examples/node && bun install --frozen-lockfile)
              (cd rust/tonk-core/tests/node-sync && bun install --frozen-lockfile)
              echo "Running cargo test..."
              cargo test
            '';
          };
        };

        menu = (import ./menu.nix { inherit pkgs; }).makeMenu commands;
      in
      {

        # Default dev shell - uses basic relay
        devShells = {
          default = pkgs.mkShell {
            buildInputs = commonBuildInputs;
            nativeBuildInputs = menu.commands;
            shellHook = ''
              clear
              ${menu.header}
            '';
          };

          ci = pkgs.mkShell {
            buildInputs = commonBuildInputs;
          };
        };

        checks = {
          # Run clippy on the crate source
          clippy = craneLib.cargoClippy (
            commonArgs
            // {
              inherit cargoArtifacts;
              cargoClippyExtraArgs = "--all-targets -- --deny warnings";
            }
          );

          # Check formatting
          rustfmt = craneLib.cargoFmt {
            inherit src;
            pname = "tonk";
          };
        };

        packages = {
          tonk-core = craneLib.buildPackage (
            commonArgs
            // {
              inherit cargoArtifacts;
            }
          );
        };
      }
    );
}
