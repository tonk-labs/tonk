{
  description = "Tonk";

  inputs = {
    crane.url = "github:ipetkov/crane";
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    nixpkgs-chromedriver.url = "github:NixOS/nixpkgs/master";
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
      nixpkgs-chromedriver,
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

        chromedriverDarwin = nixpkgs-chromedriver.legacyPackages.${system}.chromedriver;

        # Common build inputs for all dev shells
        commonBuildInputs =
          with pkgs;
          [
            binaryen
            caddy
            cachix
            cargo-nextest
            esbuild
            imagemagick
            jq
            mdbook
            mdbook-mermaid
            python3
            tailwindcss_4
            trunk
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

        # PostHog project API key, baked into release binaries at compile
        # time (rust/tonk-analytics reads it via option_env!). Project API
        # keys are public-by-design client keys, so committing one here is
        # fine. An empty string compiles analytics to a no-op. The EU
        # ingestion host is the crate default, so no host var is needed.
        # See docs/telemetry.md.
        posthogKey = "";

        # Rewrite Nix store libiconv to the macOS system equivalent
        # so the binary works on machines without Nix installed
        darwinBinaryFixup = pkgs.lib.optionalString pkgs.stdenv.isDarwin ''
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
          }
          // lib.optionalAttrs stdenv.isDarwin {
            "CHROMEDRIVER" = "${chromedriverDarwin}/bin/chromedriver";
          };

        # Import menu helpers (e.g., colorful Tonk Shell commands)
        menuHelpers = (import ./nix/menu.nix { inherit pkgs chromedriverDarwin; });

        inherit (menuHelpers) makeMenu makeDevShellHook menuTestCommand;

        commands = {
          "build:web" = {
            description = "Build the Tonk web application";
            command = "nix build .#tonk-ui";
          };
          "dev:web" = {
            description = "Start a dev server (set UCAN_ENDPOINT to override /ucan/ proxy)";
            command = ''
              ENDPOINT="''${UCAN_ENDPOINT:-https://staging.tonk.xyz/ucan/}"
              # Serve the user guide at /guide/ via mdbook's own live-reload
              # server, proxied by trunk (see the [[proxies]] entry in
              # Trunk.toml).
              #
              # Call mdbook and mdbook-mermaid by absolute nix-store path. A
              # `~/.cargo/bin/mdbook` on PATH would otherwise shadow the nix
              # one, and a version-mismatched mdbook driving the mermaid
              # preprocessor silently kills `mdbook serve` mid-session.
              # Putting the matched mermaid binary first on PATH lets mdbook
              # find the preprocessor without disturbing the rest of PATH.
              #
              # Reclaim port 3001 from any mdbook left by a previous run,
              # otherwise the new mdbook fails to bind and /guide/ goes dead.
              GUIDE_PORT=3001
              pkill -f "mdbook serve ./guide" 2>/dev/null || true
              PATH="${pkgs.mdbook-mermaid}/bin:$PATH" \
                ${pkgs.mdbook}/bin/mdbook serve ./guide --port "$GUIDE_PORT" --hostname 127.0.0.1 &
              GUIDE_PID=$!
              trap 'kill "$GUIDE_PID" 2>/dev/null; pkill -f "mdbook serve ./guide" 2>/dev/null' EXIT INT TERM
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
            args = "--workspace --exclude tonk-cli";
          };

          tests-web-release = buildTestArchive {
            name = "web-release";
            target = "wasm32-unknown-unknown";
            args = "--workspace --exclude tonk-cli --release";
          };

          tests = pkgs.runCommand "tests-all" { } ''
            mkdir -p $out
            cp ${self.packages.${system}.tests-native-debug}/*.tar.zst $out/
            cp ${self.packages.${system}.tests-native-release}/*.tar.zst $out/
            cp ${self.packages.${system}.tests-web-debug}/*.tar.zst $out/
            cp ${self.packages.${system}.tests-web-release}/*.tar.zst $out/
          '';

          tonk-cli = buildCrate {
            pname = "tonk-cli";
            cargoExtraArgs = "--package tonk-cli";
            TONK_POSTHOG_KEY = posthogKey;
            fixupPhase = darwinBinaryFixup;
          };

          tonk-ui = buildTrunkCrate {
            pname = "tonk-ui";
            trunkConfig = "./rust/tonk-ui/Trunk.toml";
            TONK_POSTHOG_KEY = posthogKey;
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

          # The user guide (mdBook), built to static HTML. Served under
          # /guide/ on the deployed site, so `site-url` in book.toml must
          # match that prefix.
          tonk-guide = pkgs.stdenv.mkDerivation {
            pname = "tonk-guide";
            version = "0.1.0";
            src = filter {
              root = ./guide;
            };
            nativeBuildInputs = [
              pkgs.mdbook
              pkgs.mdbook-mermaid
            ];
            buildPhase = ''
              mdbook build --dest-dir ./book
            '';
            installPhase = ''
              mkdir -p $out
              cp -r ./book/* $out/
            '';
          };

          tonk-cloudflare-artifacts = buildWasmCrate {
            pname = "tonk-cloudflare-assets";
            buildPhase = ''
              mkdir -p ./build
              cp -r ${tonk-access-service} ./build/tonk-access-service
              cp -r ${tonk-ui} ./build/tonk-ui
              # Files copied from the read-only nix store keep their
              # read-only perms, so make the tonk-ui tree writable before
              # adding the guide subdirectory into it.
              chmod -R u+w ./build/tonk-ui
              # Ship the guide as static assets under tonk-ui/guide so the
              # Cloudflare asset layer serves it at /guide/ directly.
              mkdir -p ./build/tonk-ui/guide
              cp -r ${tonk-guide}/* ./build/tonk-ui/guide/
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
