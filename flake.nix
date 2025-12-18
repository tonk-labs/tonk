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
    bun2nix = {
      url = "github:nix-community/bun2nix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      self,
      nixpkgs,
      flake-utils,
      fenix,
      crane,
      bun2nix,
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
        fenixPkgs = fenix.packages.${system};
        bun2nixPkg = bun2nix.packages.${system}.default;

        rustToolchain = fenixPkgs.fromToolchainFile {
          file = ./rust-toolchain.toml;
          sha256 = "sha256-sqSWJDUxc+zaz1nBWMAJKTAGBuGWP25GCftIOlCEAtA=";
        };

        # Crane setup with fenix toolchain
        craneLib = (crane.mkLib pkgs).overrideToolchain rustToolchain;

        # Source filtering - only include Rust-relevant files for better caching
        src = pkgs.lib.cleanSourceWith {
          src = ./.;
          filter =
            path: type: (craneLib.filterCargoSources path type) || (builtins.match ".*\\.toml$" path != null);
        };

        # Full source for WASM/Node tests (includes JS, JSON, HTML files)
        testSrc = pkgs.lib.cleanSourceWith {
          src = ./.;
          filter =
            path: type:
            (craneLib.filterCargoSources path type)
            || (builtins.match ".*\\.toml$" path != null)
            || (builtins.match ".*\\.tonk$" path != null)
            || (builtins.match ".*\\.js$" path != null)
            || (builtins.match ".*\\.ts$" path != null)
            || (builtins.match ".*\\.json$" path != null)
            || (builtins.match ".*\\.html$" path != null)
            || (builtins.match ".*bun\\.lock$" path != null)
            || (builtins.match ".*bun\\.nix$" path != null);
        };

        # Common arguments for all Crane builds
        commonArgs = {
          inherit src;
          pname = "tonk";
          version = "0.1.0";
          strictDeps = true;
          # Build inputs needed for compilation
          buildInputs =
            pkgs.lib.optionals pkgs.stdenv.isLinux [
              pkgs.openssl
            ]
            ++ pkgs.lib.optionals pkgs.stdenv.isDarwin [
              pkgs.apple-sdk_15
              pkgs.libiconv
            ];
          nativeBuildInputs = [
            rustToolchain
          ]
          ++ pkgs.lib.optionals pkgs.stdenv.isLinux [
            pkgs.pkg-config
          ];
        };

        # Build dependencies only - this is cached and reused
        cargoArtifacts = craneLib.buildDepsOnly commonArgs;

        wasmCargoArtifacts = craneLib.buildDepsOnly (
          commonArgs
          // {
            CARGO_BUILD_TARGET = "wasm32-unknown-unknown";
            cargoExtraArgs = "-p tonk-core --features wasm-browser";
            doCheck = false;
          }
        );

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

        # Common build inputs for all dev shells
        commonBuildInputs =
          with pkgs;
          [
            rustToolchain
            wasm-bindgen-cli
            wasm-pack
            bun
            bun2nixPkg # For generating bun.nix files
            wrangler
            geckodriver # For WASM headless tests
          ]
          ++ lib.optionals stdenv.isLinux [
            # Linux-specific inputs
            firefox-esr # For WASM headless tests
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
          # Clippy check
          clippy = craneLib.cargoClippy (
            commonArgs
            // {
              inherit cargoArtifacts;
              cargoClippyExtraArgs = "--all-targets -- -D warnings";
            }
          );

          # Format check
          rustfmt = craneLib.cargoFmt {
            inherit src;
            pname = "tonk";
          };

          # Run tests
          tests = craneLib.cargoTest (
            commonArgs
            // {
              inherit cargoArtifacts;
              src = testSrc;
              cargoTestExtraArgs = "-- --test-threads=1";
            }
          );

          # WASM compilation check
          # NOTE: Browser tests fail in Nix sandbox
          # Run browser tests manually with: wasm-pack test --headless --firefox -- --features wasm-browser
          wasm-tests = craneLib.mkCargoDerivation {
            src = testSrc;
            pname = "tonk-wasm-tests";
            version = "0.1.0";
            cargoArtifacts = wasmCargoArtifacts;
            doInstallCargoArtifacts = false;

            nativeBuildInputs = [
              pkgs.wasm-pack
              wasm-bindgen-cli
              pkgs.cacert
            ];

            buildPhaseCargoCommand = ''
              export HOME=$TMPDIR
              export SSL_CERT_FILE=${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt

              cd rust/tonk-core

              echo "Building WASM target..."

              # Build WASM package for Node.js target
              RUSTFLAGS="--cfg getrandom_backend=\"wasm_js\"" \
                wasm-pack build --target nodejs --features wasm-node

              # Verify browser target compiles
              RUSTFLAGS="--cfg getrandom_backend=\"wasm_js\"" \
                wasm-pack build --target web --features wasm-browser
            '';

            installPhase = ''
              touch $out
            '';
          };

          # Node.js integration tests
          node-tests =
            let
              # Pre-fetch dependencies for each test directory using bun2nix
              examplesSharedDeps = bun2nixPkg.fetchBunDeps {
                bunNix = ./rust/tonk-core/examples/shared/bun.nix;
              };
              examplesNodeDeps = bun2nixPkg.fetchBunDeps {
                bunNix = ./rust/tonk-core/examples/node/bun.nix;
              };
              nodeSyncDeps = bun2nixPkg.fetchBunDeps {
                bunNix = ./rust/tonk-core/tests/node-sync/bun.nix;
              };
            in
            craneLib.mkCargoDerivation {
              src = testSrc;
              pname = "tonk-node-tests";
              version = "0.1.0";
              cargoArtifacts = wasmCargoArtifacts;
              doInstallCargoArtifacts = false;

              nativeBuildInputs = [
                pkgs.wasm-pack
                wasm-bindgen-cli
                pkgs.bun
                pkgs.nodejs
                pkgs.cacert
              ];

              buildPhaseCargoCommand = ''
                # Set up writable directories for bun (following bun2nix hook pattern)
                export HOME=$(mktemp -d)
                export SSL_CERT_FILE=${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt

                cd rust/tonk-core

                echo "Building WASM for Node.js..."
                RUSTFLAGS="--cfg getrandom_backend=\"wasm_js\"" \
                  wasm-pack build --target nodejs --out-dir pkg-node -- --features wasm-node

                # Helper function to install bun deps by copying cache to writable location
                install_bun_deps() {
                  local cache_src="$1"
                  local writable_cache=$(mktemp -d)
                  cp -r "$cache_src"/share/bun-cache/. "$writable_cache"
                  BUN_INSTALL_CACHE_DIR="$writable_cache" bun install --frozen-lockfile --backend=copyfile --ignore-scripts
                }

                echo "Installing dependencies for examples/shared..."
                cd examples/shared
                install_bun_deps "${examplesSharedDeps}"

                echo "Running Node.js integration tests..."
                cd ../node
                install_bun_deps "${examplesNodeDeps}"
                # Use --exit to force mocha to terminate after tests complete, even with dangling handles
                ./node_modules/.bin/mocha --exit --timeout 10000 integration/*.test.js

                echo "Running Node sync tests..."
                cd ../../tests/node-sync
                install_bun_deps "${nodeSyncDeps}"
                bun run test
              '';

              installPhase = ''
                touch $out
              '';
            };
        };

        packages = {
          # Build tonk-core library
          tonk-core = craneLib.buildPackage (
            commonArgs
            // {
              inherit cargoArtifacts;
              pname = "tonk-core";
              version = "0.1.0";
              cargoExtraArgs = "-p tonk-core";
            }
          );

          # Build tonk-space library
          tonk-space = craneLib.buildPackage (
            commonArgs
            // {
              inherit cargoArtifacts;
              pname = "tonk-space";
              version = "0.1.0";
              cargoExtraArgs = "-p tonk-space";
            }
          );

          # Build tonk-access-service
          tonk-access-service = craneLib.buildPackage (
            commonArgs
            // {
              inherit cargoArtifacts;
              pname = "tonk-access-service";
              version = "0.1.0";
              cargoExtraArgs = "-p tonk-access-service";
            }
          );

          # Build entire workspace
          default = craneLib.buildPackage (
            commonArgs
            // {
              inherit cargoArtifacts;
              pname = "tonk";
              version = "0.1.0";
            }
          );
        };
      }
    );
}
