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

        # Crane setup with fenix toolchain
        craneLib = (crane.mkLib pkgs).overrideToolchain rustToolchainStable;

        # Source filtering - only include Rust-relevant files for better caching
        src = pkgs.lib.cleanSourceWith {
          src = ./.;
          filter =
            path: type: (craneLib.filterCargoSources path type) || (builtins.match ".*\\.toml$" path != null);
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
            rustToolchainStable
          ]
          ++ pkgs.lib.optionals pkgs.stdenv.isLinux [
            pkgs.pkg-config
          ];
        };

        # Build dependencies only - this is cached and reused
        cargoArtifacts = craneLib.buildDepsOnly commonArgs;

        wasm-bindgen-cli =
          with pkgs;
          rustPlatform.buildRustPackage rec {
            pname = "wasm-bindgen-cli";
            version = "0.2.100";
            buildInputs = [
              rustToolchainStable
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
            rustToolchainStable
            wasm-bindgen-cli
            bun
            wrangler
          ]
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
            command = "cargo test";
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
              cargoClippyExtraArgs = "--all-targets --all-features -- -D warnings";
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
            }
          );
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
