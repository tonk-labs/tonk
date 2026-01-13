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

        # Source filtering for Rust builds (include .tonk test data files)
        src = pkgs.lib.cleanSourceWith {
          src = ./.;
          filter =
            path: type: (craneLib.filterCargoSources path type) || (builtins.match ".*\\.tonk$" path != null);
        };

        # Vendor dependencies with git dependency hashes
        cargoVendorDir = craneLib.vendorCargoDeps {
          inherit src;
          outputHashes = {
            "git+https://github.com/tonk-labs/samod?branch=wasm-runtime#fe92f4d6fbb53fe107b1f4d9eea3fe5da7a30322" =
              "sha256-0mr/mtsnm+BZHlQLPEfe+wmzWjPldcULSvOzCOf5yMc=";
            "git+https://github.com/tonk-labs/rs-ucan.git?branch=jackddouglas/feat/check#671a0256621eb4656b42d9e631108da3ec18158b" =
              "sha256-5KQ7wIXv7PHgd6y1pq0+aUU/VFW7BLxECmVUNk1JfGw=";
            "git+https://github.com/dialog-db/dialog-db.git?branch=tonk-ecs#b533146c83451cd94fe356c56eb845fd1f0a5586" =
              "sha256-PFyP3BbNCq0WiLD0Z8TKHXv7LJtAMsv3kXPrvvakjlw=";
            "git+https://github.com/dialog-db/dialog-db.git?branch=feat/s3-presign-crate#bb05dcad94a343d121f8870a5360706aeeb71632" =
              "sha256-K5wDWTWGUfQ23jAv9NDB0AgTdEPaUJIDx0Yf1KBCqww=";
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
            wrangler
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

          "wasm:test" = {
            description = "Builds WASM and runs Node.js tests";
            command = ''
              set -e
              echo "Building WASM for Node.js..."
              cd rust/tonk-core
              wasm-pack build --target nodejs --out-dir pkg-node -- --features wasm-node

              echo ""
              echo "Installing test dependencies..."
              (cd examples/node && bun install --frozen-lockfile)

              echo ""
              echo "Running integration tests..."
              (cd examples/node && bun run test)
            '';
          };

          "wasm:test:sync" = {
            description = "Runs sync protocol tests";
            command = ''
              set -e
              echo "Building WASM for Node.js..."
              cd rust/tonk-core
              wasm-pack build --target nodejs --out-dir pkg-node -- --features wasm-node

              echo ""
              echo "Installing dependencies..."
              (cd examples/server && bun install --frozen-lockfile)
              (cd tests/node-sync && bun install --frozen-lockfile)

              echo ""
              echo "Running sync tests..."
              (cd tests/node-sync && bun run test)
            '';
          };

          "test:all" = {
            description = "Runs the full test suite";
            command = ''
              set -e
              echo "Installing Node.js dependencies..."
              (cd rust/tonk-core/examples/server && bun install --frozen-lockfile)
              (cd rust/tonk-core/examples/node && bun install --frozen-lockfile)
              (cd rust/tonk-core/tests/node-sync && bun install --frozen-lockfile)

              echo ""
              echo "Running native Rust tests..."
              cargo test

              echo ""
              echo "Building WASM for Node.js..."
              (cd rust/tonk-core && wasm-pack build --target nodejs --out-dir pkg-node -- --features wasm-node)

              echo ""
              echo "Running WASM integration tests..."
              (cd rust/tonk-core/examples/node && bun run test)

              echo ""
              echo "All tests passed!"
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

          # Cargo tests
          tests = craneLib.cargoTest (
            commonArgs
            // {
              inherit cargoArtifacts;
              cargoTestExtraArgs = "-- --test-threads=1";
            }
          );
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

          tonk-access-service = craneLib.buildPackage (
            commonArgs
            // {
              inherit cargoArtifacts;
              pname = "tonk-access-service";
              version = "0.1.0";
            }
          );
        };
      }
    );
}
