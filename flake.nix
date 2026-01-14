{
  description = "Tonk";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    fenix = {
      url = "github:nix-community/fenix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    nix-filter.url = "github:numtide/nix-filter";
  };

  outputs =
    { self
    , nixpkgs
    , flake-utils
    , fenix
    , nix-filter
    ,
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
        fenixPkgs = fenix.packages.${system};
        filter = nix-filter.lib;

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
            cachix
            cargo-nextest
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

        rustSource = filter {
          root = ./.;
          include = [
            "Cargo.lock"
            "Cargo.toml"
            "rust-toolchain.toml"
            "rust"
          ];
        };

        # Builds one or more test archives for use with `cargo-nextest`
        # The final package name will be `tests-$name`.
        # SEE: https://nexte.st/docs/ci-features/archiving/
        rustTestPackage = { name, command }: pkgs.rustPlatform.buildRustPackage {
          pname = "tests-${name}";
          version = "0.1.0";
          src = rustSource;
          cargoLock = {
            lockFile = ./Cargo.lock;
            outputHashes = cargoGitDependencies;
          };
          nativeBuildInputs = [ rustToolchainStable ] ++ commonBuildInputs;
          buildPhase = command;
          installPhase = ''
            mkdir -p $out
            cp -r ./*.tar.zst $out/
          '';
        };

        # Helpers for stamping out test commands (which share a lot in common)
        menuTestCommand = target: ''
          nix build .#${target}

          TESTS_PATH=$(nix eval .#${target}.outPath --raw)

          cargo nextest run \
            --no-capture \
            --workspace-remap ./ \
            --archive-file "$TESTS_PATH/${target}.tar.zst" \
        '';

        menuTestEnv = with pkgs; lib.optionalAttrs stdenv.isLinux {
          "CHROME" = "${chromium}/bin/chromium";
          "CHROMEDRIVER" = "${chromedriver}/bin/chromedriver";
        };

        # Cargo dependencies that are Git repositories need to have their
        # expected build hash # recorded separately. We make a shared variable so
        # that the same dependencies can be # used across all derivations that
        # need them.
        cargoGitDependencies = {
          "dialog-artifacts-0.1.0" = "sha256-wVImYyZnww23y4ebLV7zGO38O4HDED31Z8BDIChJwBg=";
          "ucan-0.5.0" = "sha256-CCQar9nU3KhBn1Kl5RsRJUASX8bO77pu7wbzzoLccBs=";
        };

        commands = {
          "build:web" = {
            description = "Build the Tonk web application";
            command = "trunk build --config ./rust/tonk-ui/Trunk.toml";
          };
          "dev:web" =
            {
              description = "Start a dev server for the Tonk web application";
              command = "trunk serve --config ./rust/tonk-ui/Trunk.toml";
            };
          "lint" = {
            description = "Lint the full source tree";
            command = "nix flake check";
          };
          "test:all" = {
            description = "Run the full test suite (all configurations, grab a coffee)";
            command = ''
              test:native:debug
              test:native:release
              test:web:debug
              test:web:release
            '';
            env = menuTestEnv;
          };

          "test:native:debug" = {
            description = "Unit and integration tests (${system}, debug)";
            command = menuTestCommand "tests-native-debug";
            env = menuTestEnv;
          };

          "test:native:release" = {
            description = "Unit and integration tests (${system}, release)";
            command = menuTestCommand "tests-native-release";
            env = menuTestEnv;
          };

          "test:web:debug" = {
            description = "Unit tests (wasm32-unknown-unknown, debug)";
            command = menuTestCommand "tests-web-debug";
            env = menuTestEnv;
          };

          "test:web:release" = {
            description = "Unit tests (wasm32-unknown-unknown, release)";
            command = menuTestCommand "tests-web-release";
            env = menuTestEnv;
          };

          "menu" = {
            description = "Display all Tonk Shell commands";
            command = ''showTonkMenu'';
          };
        };

        menu = (import ./menu.nix { inherit pkgs; }).makeMenu commands;
      in
      {
        devShells = with pkgs; {
          default = mkShell {
            buildInputs = commonBuildInputs;
            nativeBuildInputs = menu.commands;
            env = lib.optionalAttrs stdenv.isLinux {
              "CHROMEDRIVER" = "${chromedriver}/bin/chromedriver";
            };
            shellHook = ''
              clear
              ${menu.header}

              function showTonkMenu() {
                ${menu.menuText}
              }

              export -f showTonkMenu
            '';
          };

          ci = mkShell {
            buildInputs = commonBuildInputs;
            nativeBuildInputs = menu.commands;
            env = lib.optionalAttrs stdenv.isLinux {
              "CHROME" = "${chromium}/bin/chromium";
              "CHROMEDRIVER" = "${chromedriver}/bin/chromedriver";
            };
          };
        };

        checks = {
          clippy = pkgs.rustPlatform.buildRustPackage {
            pname = "tonk-clippy-lint";
            version = "0.1.0";
            src = rustSource;
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
            tests-native-debug = rustTestPackage {
              name = "native-debug";
              command = ''
                cargo nextest archive \
                  --features integration-tests \
                  --archive-file ./tests-native-debug.tar.zst
              '';
            };

            tests-native-release = rustTestPackage {
              name = "native-release";
              command = ''
                cargo nextest archive \
                  --release \
                  --features integration-tests \
                  --archive-file ./tests-native-release.tar.zst
              '';
            };

            tests-web-debug = rustTestPackage {
              name = "web-debug";
              command = ''
                cargo nextest archive \
                  --target wasm32-unknown-unknown \
                  --archive-file ./tests-web-debug.tar.zst
              '';
            };

            tests-web-release = rustTestPackage {
              name = "web-release";
              command = ''
                cargo nextest archive \
                  --release \
                  --target wasm32-unknown-unknown \
                  --archive-file ./tests-web-release.tar.zst
              '';
            };

            tests = rustTestPackage
              {
                name = "all";
                command = ''
                  cp ${self.packages.${system}.tests-native-debug}/*.tar.zst ./
                  cp ${self.packages.${system}.tests-native-release}/*.tar.zst ./
                  cp ${self.packages.${system}.tests-web-debug}/*.tar.zst ./
                  cp ${self.packages.${system}.tests-web-release}/*.tar.zst ./
                '';
              };

            tonk-ui = pkgs.rustPlatform.buildRustPackage {
              pname = "tonk-ui";
              version = "0.1.0";
              src = rustSource;
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

            tonk-ui-test-server = with pkgs; writeScriptBin "tonk-ui-test-server" ''
              #!${bash}/bin/bash
              PORT=''${1:-8080}
              echo "Test server live at http://127.0.0.1:$PORT"
              ${static-web-server}/bin/static-web-server --port $PORT -d ${self.packages.${system}.tonk-ui}
            '';
          };
      }
    );
}
