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

        rustToolchain = fenixPkgs.fromToolchainFile {
          file = ./rust-toolchain.toml;
          sha256 = "sha256-iksnAJGL0yvaXLqz2iX8TqG+4GuyTvJNHfiQmX7zWlE=";
        };

        wasm-bindgen-cli =
          with pkgs;
          rustPlatform.buildRustPackage rec {
            pname = "wasm-bindgen-cli";
            version = "0.2.100";
            buildInputs = [
              rustToolchain
            ];

            src = fetchCrate {
              inherit pname version;
              sha256 = "sha256-3RJzK7mkYFrs7C/WkhW9Rr4LdP5ofb2FdYGz1P7Uxog=";
            };

            cargoHash = "sha256-qsO12332HSjWCVKtf1cUePWWb9IdYUmT+8OPj/XP2WE=";
          };

        # Set up crane with the fenix toolchain
        craneLib = (crane.mkLib pkgs).overrideToolchain (_: rustToolchain);

        # Source filtering for Rust builds
        src = craneLib.cleanCargoSource ./.;

        # Vendor dependencies with git dependency hashes
        cargoVendorDir = craneLib.vendorCargoDeps {
          inherit src;
          outputHashes = {
            "git+https://github.com/tonk-labs/samod?branch=wasm-runtime#fe92f4d6fbb53fe107b1f4d9eea3fe5da7a30322" =
              "sha256-0mr/mtsnm+BZHlQLPEfe+wmzWjPldcULSvOzCOf5yMc=";
            "git+https://github.com/tonk-labs/rs-ucan.git?branch=fix/wasm-compile#25b4a5a02a89b9f9332bc61e5c3d7ddebc7e058f" =
              "sha256-ZUqBvqG0hvhpcR1uXjAh9TbL/zLKw8Tv2TweSKD1f48=";
            "git+https://github.com/dialog-db/dialog-db.git?branch=tonk-ecs#288cff4a36e83fb7ce892b37c5132f4ab519a479" =
              "sha256-veYCuACVZEIveVuwh9O3XuoJtrihE/t+cWQTe7zWYsg=";
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
            rustToolchain
            wasm-pack
            wasm-bindgen-cli
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

          "wasm:build" = {
            description = "Builds WASM for browser and Node.js";
            command = ''
              set -e
              cd rust/tonk-core
              rm -rf pkg pkg-node pkg-browser
              echo "Building WASM for browser target..."
              wasm-pack build --target web --out-dir pkg-browser -- --features wasm-browser
              echo ""
              echo "Building WASM for Node.js target..."
              wasm-pack build --target nodejs --out-dir pkg-node -- --features wasm-node
              echo ""
              echo "WASM build complete!"
              echo "  Browser: rust/tonk-core/pkg-browser/"
              echo "  Node.js: rust/tonk-core/pkg-node/"
            '';
          };

          "wasm:build:browser" = {
            description = "Builds WASM for browser target";
            command = ''
              set -e
              cd rust/tonk-core
              rm -rf pkg-browser
              wasm-pack build --target web --out-dir pkg-browser -- --features wasm-browser
            '';
          };

          "wasm:build:node" = {
            description = "Builds WASM for Node.js target";
            command = ''
              set -e
              cd rust/tonk-core
              rm -rf pkg-node
              wasm-pack build --target nodejs --out-dir pkg-node -- --features wasm-node
            '';
          };

          "wasm:clean" = {
            description = "Cleans WASM build artifacts";
            command = ''
              rm -rf rust/tonk-core/pkg rust/tonk-core/pkg-node rust/tonk-core/pkg-browser
              echo "WASM artifacts cleaned"
            '';
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
              pname = "tonk-core";
              version = "0.1.0";
            }
          );

          tonk-space = craneLib.buildPackage (
            commonArgs
            // {
              inherit cargoArtifacts;
              pname = "tonk-space";
              version = "0.1.0";
            }
          );
        };
      }
    );
}
