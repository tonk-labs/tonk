{
  description = "Tonk";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    fenix = {
      url = "github:nix-community/fenix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    { self
    , nixpkgs
    , flake-utils
    , fenix
    ,
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
            useFetchCargoVendor = true;
          };

        # Common build inputs for all dev shells
        commonBuildInputs =
          with pkgs;
          [
            rustToolchainStable
            wasm-bindgen-cli
            tailwindcss_4
            trunk
          ]
          ++ lib.optionals stdenv.isLinux [
            # Linux-specific inputs
            openssl
            pkg-config
            chromium
            chromedriver
          ]
          ++ lib.optionals stdenv.isDarwin [
            # MacOS-specific inputs
          ];

        # Cargo dependencies that are Git repositories need to have their
        # expected build hash # recorded separately. We make a shared variable so
        # that the same dependencies can be # used across all derivations that
        # need them.
        cargoGitDependencies = {
          "dialog-artifacts-0.1.0" = "sha256-wVImYyZnww23y4ebLV7zGO38O4HDED31Z8BDIChJwBg=";
          "ucan-0.5.0" = "sha256-CCQar9nU3KhBn1Kl5RsRJUASX8bO77pu7wbzzoLccBs=";
        };

        commands = {
          "build" = {
            description = "Builds all of Tonk";
            command = "cargo build";
          };
          "build:web" = {
            description = "Builds the Tonk web application";
            command = "trunk build --config ./rust/tonk-ui/Trunk.toml";
          };
          "dev:web" =
            {
              description = "Start a dev server for the Tonk web application";
              command = "trunk serve --config ./rust/tonk-ui/Trunk.toml";
            };
          "lint" = {
            description = "Run lints against the current tree";
            command = "nix flake check";
          };
          "test:all" = {
            description = "Run the full test suite (all configurations, grab a coffee)";
            command = ''
              test:nat:dbg
              test:nat:rls
              test:web:dbg
              test:web:rls
            '';
          };

          "test:nat:dbg" = {
            description = "Unit and integration tests (${system}, debug)";
            command = "cargo test --features integration-tests";
          };

          "test:nat:rls" = {
            description = "Unit and integration tests (${system}, release)";
            command = "cargo test --features integration-tests --release";
          };

          "test:web:dbg" = {
            description = "Unit tests (wasm32-unknown-unknown, debug)";
            command = "cargo test --target wasm32-unknown-unknown";
          };

          "test:web:rls" = {
            description = "Unit tests (wasm32-unknown-unknown, release)";
            command = "cargo test --target wasm32-unknown-unknown --release";
          };
        };

        menu = (import ./menu.nix { inherit pkgs; }).makeMenu commands;
      in
      {

        # Default dev shell - uses basic relay
        devShells = with pkgs; {
          default = mkShell {
            buildInputs = commonBuildInputs;
            nativeBuildInputs = menu.commands;
            env = lib.optionals stdenv.isLinux {
              "CHROMEDRIVER" = "${chromedriver}/bin/chromedriver";
            };
            shellHook = ''
              clear
              ${menu.header}
            '';
          };

          ci = mkShell {
            buildInputs = commonBuildInputs;
            nativeBuildInputs = menu.commands;
            env = lib.optionals stdenv.isLinux {
              "CHROMEDRIVER" = "${chromedriver}/bin/chromedriver";
              "CHROME" = "${chromium}/bin/chromium";
            };
          };
        };

        checks = {
          clippy = pkgs.rustPlatform.buildRustPackage {
            pname = "tonk-clippy-lint";
            version = "0.1.0";
            src = ./.;
            cargoLock = {
              lockFile = ./Cargo.lock;
              outputHashes = cargoGitDependencies;
            };
            nativeBuildInputs = [ rustToolchainStable ];
            buildPhase = ''
              cargo clippy --all-targets --all-features -- -D warnings
            '';
            installPhase = ''
              touch $out
            '';
          };

          rustfmt =
            pkgs.runCommand "tonk-fmt-check"
              {
                nativeBuildInputs = [ rustToolchainStable ];
              }
              ''
                cd ${./.}
                cargo fmt --check
                touch $out
              '';
        };

        packages =
          {
            tonk-ui = pkgs.rustPlatform.buildRustPackage {
              pname = "tonk-ui";
              version = "0.1.0";
              src = ./.;
              cargoLock = {
                lockFile = ./Cargo.lock;
                outputHashes = cargoGitDependencies;
              };
              nativeBuildInputs = [ rustToolchainStable ] ++ commonBuildInputs;
              buildPhase = ''
                trunk build --config ./rust/tonk-ui/Trunk.toml
              '';
              installPhase = ''
                mkdir -p $out
                cp -r ./rust/tonk-ui/dist/* $out/
              '';
            };
          };
      }
    );
}
