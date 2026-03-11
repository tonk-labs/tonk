{
  description = "Tonk";

  inputs = {
    crane.url = "github:ipetkov/crane";
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    nix-filter.url = "github:numtide/nix-filter";
    wrangler-flake = {
      url = "github:emrldnix/wrangler";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      self,
      crane,
      nixpkgs,
      flake-utils,
      rust-overlay,
      nix-filter,
      wrangler-flake,
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [
            (import rust-overlay)
            (import ./nix/esbuild.nix)
          ];
        };
        filter = nix-filter.lib;

        # We get wrangler from a 3P crate because nixpkgs#wrangler lags
        # the latest release
        wrangler = wrangler-flake.packages.${system}.wrangler;

        # Common build inputs for all dev shells
        commonBuildInputs =
          with pkgs;
          [
            tailwindcss_4
            trunk
            binaryen
            cachix
            cargo-nextest
            esbuild
            worker-build
          ]
          ++ lib.optionals stdenv.isLinux [
            # Linux-specific inputs
            openssl
            pkg-config
            dbus
            chromium
            chromedriver
          ];

        # Import rust helpers
        rustHelpers = import ./nix/rust.nix {
          inherit pkgs filter crane;
          buildInputs = commonBuildInputs;
          workspaceRoot = ./.;
        };

        inherit (rustHelpers)
          buildCrate
          buildWasmCrate
          buildTrunkCrate
          buildTestArchive
          cargoChecks
          rustToolchain
          wasm-bindgen-cli
          ;

        # Include the Rust toolchain in build inputs for dev shells
        devShellBuildInputs = commonBuildInputs ++ [
          pkgs.cachix
          wrangler
          rustToolchain
          wasm-bindgen-cli
        ];

        devShellEnvVars =
          with pkgs;
          {
            # These *_BIN envvars are an implicit part of the `worker-build` API
            # Noting that successfully building inside the Nix sandbox depends on
            # specific version ranges of `wasm-bindgen-cli`, `esbuild` and the Cargo
            # `web-sys` crate.
            "WASM_BINDGEN_BIN" = "${wasm-bindgen-cli}/bin/wasm-bindgen";
            "ESBUILD_BIN" = "${esbuild}/bin/esbuild";
            "WASM_OPT_BIN" = "${binaryen}/bin/wasm-opt";
          }
          // lib.optionalAttrs stdenv.isLinux {
            "CHROME" = "${chromium}/bin/chromium";
            "CHROMEDRIVER" = "${chromedriver}/bin/chromedriver";
          };

        # Import menu helpers (e.g., colorful Tonk Shell commands)
        menuHelpers = (import ./nix/menu.nix { inherit pkgs; });

        inherit (menuHelpers) makeMenu makeDevShellHook menuTestCommand;

        commands = {
          "build:web" = {
            description = "Build the Tonk web application";
            command = "nix build .#tonk-ui";
          };
          "dev:web" = {
            description = "Start a dev server (set UCAN_ENDPOINT to override /ucan/ proxy)";
            command = ''
              ENDPOINT="''${UCAN_ENDPOINT:-https://tonk-access-service.tonk.workers.dev/ucan/}"
              trunk serve --config ./rust/tonk-ui/Trunk.toml --proxy-backend "$ENDPOINT"
            '';
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
          };

          "test:native:debug" = menuTestCommand {
            description = "Unit and integration tests (${system}, debug)";
            package = "tests-native-debug";
          };

          "test:native:release" = menuTestCommand {
            description = "Unit and integration tests (${system}, release)";
            package = "tests-native-release";
          };

          "test:web:debug" = menuTestCommand {
            description = "Unit tests (wasm32-unknown-unknown, debug)";
            package = "tests-web-debug";
          };

          "test:web:release" = menuTestCommand {
            description = "Unit tests (wasm32-unknown-unknown, release)";
            package = "tests-web-release";
          };

          "menu" = {
            description = "Display all Tonk Shell commands";
            command = "showTonkMenu";
          };
        };

        menu = makeMenu commands;
      in
      {

        checks =
          cargoChecks
          // (with pkgs; {
            nixfmt-check = runCommand "nixfmt-check" { } ''
              cd ${self}
              echo "Checking Nix file formatting..."
              ${nixfmt}/bin/nixfmt --check $(find . -name '*.nix' -type f)
              touch $out
            '';
          });

        devShells = with pkgs; {
          default = mkShell {
            buildInputs = devShellBuildInputs;
            nativeBuildInputs = menu.commands;
            env = devShellEnvVars;
            shellHook = makeDevShellHook menu;
          };

          ci = mkShell {
            buildInputs = devShellBuildInputs;
            nativeBuildInputs = menu.commands;
            env = devShellEnvVars;
          };
        };

        packages = rec {
          tests-native-debug = buildTestArchive {
            name = "native-debug";
            args = "--workspace --exclude tonk-ui --exclude tonk-core --features integration-tests";
          };

          tests-native-release = buildTestArchive {
            name = "native-release";
            args = "--workspace --exclude tonk-ui --exclude tonk-core --features integration-tests --release";
          };

          tests-web-debug = buildTestArchive {
            name = "web-debug";
            target = "wasm32-unknown-unknown";
          };

          tests-web-release = buildTestArchive {
            name = "web-release";
            target = "wasm32-unknown-unknown";
            args = "--release";
          };

          tests-cli-integration = buildTestArchive {
            name = "cli-integration";
            args = "--package tonk-cli --test cli_integration --bin carry";
          };

          tests = pkgs.runCommand "tests-all" { } ''
            mkdir -p $out
            cp ${self.packages.${system}.tests-native-debug}/*.tar.zst $out/
            cp ${self.packages.${system}.tests-native-release}/*.tar.zst $out/
            cp ${self.packages.${system}.tests-web-debug}/*.tar.zst $out/
            cp ${self.packages.${system}.tests-web-release}/*.tar.zst $out/
          '';

          carry-cli = buildCrate {
            pname = "carry-cli";
            cargoExtraArgs = "--package tonk-cli";
            # Rewrite Nix store libiconv to the macOS system equivalent
            # so the binary works on machines without Nix installed
            fixupPhase = pkgs.lib.optionalString pkgs.stdenv.isDarwin ''
              for bin in $out/bin/*; do
                if [ -f "$bin" ]; then
                  NIX_ICONV=$(otool -L "$bin" | grep '/nix/store.*libiconv' | awk '{print $1}' || true)
                  if [ -n "$NIX_ICONV" ]; then
                    install_name_tool -change "$NIX_ICONV" /usr/lib/libiconv.2.dylib "$bin"
                  fi
                  /usr/bin/codesign --force --sign - "$bin"
                fi
              done
            '';
          };

          tonk-assess = buildCrate {
            pname = "tonk-assess";
            cargoExtraArgs = "--package tonk-assess";
            nativeBuildInputs = pkgs.lib.optionals pkgs.stdenv.isDarwin [ pkgs.darwin.sigtool ];
            fixupPhase = pkgs.lib.optionalString pkgs.stdenv.isDarwin ''
              for bin in $out/bin/*; do
                if [ -f "$bin" ]; then
                  NIX_ICONV=$(otool -L "$bin" | grep '/nix/store.*libiconv' | awk '{print $1}' || true)
                  if [ -n "$NIX_ICONV" ]; then
                    install_name_tool -change "$NIX_ICONV" /usr/lib/libiconv.2.dylib "$bin"
                  fi
                  codesign -f -s - "$bin"
                fi
              done
            '';
          };

          tonk-ui = buildTrunkCrate {
            pname = "tonk-ui";
            trunkConfig = "./rust/tonk-ui/Trunk.toml";
          };

          tonk-access-service = buildWasmCrate {
            pname = "tonk-access-service";

            buildPhase = ''
              cd rust/tonk-access-service
              worker-build --release
              echo "fin"
            '';

            installPhase = ''
              mkdir -p $out
              cp -r ./build/* $out/
            '';
          };

          tonk-cloudflare-artifacts = buildWasmCrate {
            pname = "tonk-cloudflare-assets";
            buildPhase = ''
              mkdir -p ./build
              cp -r ${tonk-access-service} ./build/tonk-access-service
              cp -r ${tonk-ui} ./build/tonk-ui
            '';
            installPhase = ''
              mkdir -p $out
              cp -r ./build/* $out/
            '';
          };

          # This package is used by integration tests to run a web server
          # over a local deployment of tonk-ui with Caddy as reverse proxy
          # to route /ucan/* to the access service
          tonk-ui-test-server =
            with pkgs;
            writeScriptBin "tonk-ui-test-server" ''
              #!${bash}/bin/bash
              PORT=''${1:-8080}
              ACCESS_SERVICE_PORT=''${2:-8090}
              CONFIG_FILE=$(mktemp)
              trap 'rm -f "$CONFIG_FILE"' EXIT

              cat > "$CONFIG_FILE" << EOF
              :$PORT {
                  handle /ucan/* {
                      reverse_proxy localhost:$ACCESS_SERVICE_PORT
                  }
                  handle {
                      root * ${self.packages.${system}.tonk-ui}
                      file_server
                  }
              }
              EOF

              echo "Test server live at http://127.0.0.1:$PORT"
              ${caddy}/bin/caddy run --config "$CONFIG_FILE" --adapter caddyfile
            '';
        };
      }
    );

  # Building 3P wrangler is slow; this configures pulling from a cache
  # SEE: https://github.com/emrldnix/wrangler?tab=readme-ov-file#using-the-nar-cache
  nixConfig = {
    extra-substituters = [
      "https://tonk-test-cache.cachix.org"
      "https://wrangler.cachix.org"
    ];
    extra-trusted-public-keys = [
      "tonk-test-cache.cachix.org-1:H6CaKCO7CeGEq3NTQsHDPuC0+aaxwI1sDXZWloIWqEo="
      "wrangler.cachix.org-1:N/FIcG2qBQcolSpklb2IMDbsfjZKWg+ctxx0mSMXdSs="
    ];
  };
}
