{
  description = "Tonk";

  inputs = {
    crane.url = "github:ipetkov/crane";
    dialog-db-src = {
      url = "github:dialog-db/dialog-db/tonk-2026-08-28";
      flake = false;
    };
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
      dialog-db-src,
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
            (
              final: prev:
              prev.lib.optionalAttrs prev.stdenv.isDarwin {
                # Remove when nixpkgs remarshal passes with Python 3.14 on Darwin.
                remarshal = final.python313Packages.remarshal;
              }
            )
          ];
        };
        filter = nix-filter.lib;

        # We get wrangler from a 3P crate because nixpkgs#wrangler lags
        # the latest release
        wrangler = wrangler-flake.packages.${system}.wrangler;

        # The official PostHog CLI moves faster than nixpkgs. Pin its release
        # archives directly so `posthog-cli login` and the API client are
        # reproducible on every system this flake supports.
        posthogCliVersion = "0.16.0";
        posthogCliRelease =
          {
            x86_64-linux = {
              target = "x86_64-unknown-linux-gnu";
              hash = "sha256-9ucLvHdq8B6UxLEy7BDDtlljY/UBrR+pSx/qgjNlg1w=";
            };
            aarch64-linux = {
              target = "aarch64-unknown-linux-gnu";
              hash = "sha256-zgSh26cn4Ty7IZ1Ua/yKO2JGrAk2Lmywu4VTJ03Sjgg=";
            };
            x86_64-darwin = {
              target = "x86_64-apple-darwin";
              hash = "sha256-lfG8QsSq+ywwMUkQEeUMsUbo5cKCnIr6K3obsQrKa1k=";
            };
            aarch64-darwin = {
              target = "aarch64-apple-darwin";
              hash = "sha256-J4rAW5COg4jXbvPjnEcipYN7Rc0mXEmgVURj36DqVnI=";
            };
          }
          .${system};
        posthogCli = pkgs.stdenvNoCC.mkDerivation {
          pname = "posthog-cli";
          version = posthogCliVersion;
          src = pkgs.fetchurl {
            url = "https://github.com/PostHog/posthog/releases/download/posthog-cli%2Fv${posthogCliVersion}/posthog-cli-${posthogCliRelease.target}.tar.gz";
            hash = posthogCliRelease.hash;
          };
          nativeBuildInputs = pkgs.lib.optionals pkgs.stdenv.isLinux [ pkgs.autoPatchelfHook ];
          buildInputs = pkgs.lib.optionals pkgs.stdenv.isLinux [
            pkgs.stdenv.cc.cc.lib
            pkgs.zlib
          ];
          installPhase = ''
            runHook preInstall
            install -Dm755 posthog-cli $out/bin/posthog-cli
            mkdir -p $out/lib
            cp -R lib/. $out/lib/
            install -Dm644 LICENSE $out/share/licenses/posthog-cli/LICENSE
            runHook postInstall
          '';
          dontStrip = true;
        };

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

        wbg-pool = import ./nix/wbg-pool.nix {
          inherit
            pkgs
            crane
            nix-filter
            rustToolchain
            dialog-db-src
            ;
        };

        # PostHog project API key, baked into release binaries at compile
        # time (rust/tonk-analytics reads it via option_env!). Project API
        # keys are public-by-design client keys, so committing one here is
        # fine. An empty string compiles analytics to a no-op. The EU
        # ingestion host is the crate default, so no host var is needed.
        # See docs/telemetry.md.
        posthogKey = "phc_dPEh0Tb5GFMZtykYV6Yg8VEHqJeAutrL7frEMYKmRuW";

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
          # Devshell only: `commonBuildInputs` becomes nativeBuildInputs
          # for every crate derivation, so a release tool in there would
          # change 34 hashes and force a cold cache rebuild.
          pkgs.cargo-release
          pkgs.nodejs
          posthogCli
          wrangler
          rustToolchain
          wasm-bindgen-cli
          wbg-pool
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
            "WBG_POOL_FALLBACK_RUNNER" = "${wasm-bindgen-cli}/bin/wasm-bindgen-test-runner";
          }
          // lib.optionalAttrs stdenv.isLinux {
            "CHROME" = "${chromium}/bin/chromium";
            "CHROMEDRIVER" = "${chromedriver}/bin/chromedriver";
            "WBG_POOL_NO_SANDBOX" = "1";
          }
          // lib.optionalAttrs stdenv.isDarwin {
            "CHROMEDRIVER" = "${chromedriverDarwin}/bin/chromedriver";
            "WBG_POOL_BROWSER" = "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
          };

        # Import menu helpers (e.g., colorful Tonk Shell commands)
        menuHelpers = (import ./nix/menu.nix { inherit pkgs chromedriverDarwin; });

        inherit (menuHelpers) makeMenu makeDevShellHook menuTestCommand;

        commands = {
          "build:web" = {
            description = "Build the Tonk web application";
            command = "nix build .#tonk-ui";
          };
          "dev:storybook" = {
            description = "Serve the visual product Storybook at http://127.0.0.1:4173/docs/storybook/app/";
            command = ''
              ${pkgs.python3}/bin/python3 docs/storybook/scripts/build.py --check
              exec ${pkgs.python3}/bin/python3 -m http.server 4173 --bind 127.0.0.1
            '';
          };
          "dev:web" = {
            description = "Start a dev server with a local access service (set UCAN_ENDPOINT to proxy /ucan/ to a remote instead)";
            command = ''
              # Resolve the /ucan/ sync backend. By default, spin up a local,
              # blob-aware access service: the native `tonk-access-local` helper
              # mirrors the deployed Cloudflare Worker over a local in-memory S3,
              # so local dev never depends on a deployed service being in sync
              # with this checkout (a stale remote rejects newer commands — e.g.
              # blob import — with a 403). Set UCAN_ENDPOINT to point /ucan/ at a
              # real remote (staging, prod, a teammate's tunnel) instead.
              ACCESS_PID=""
              SHORTCUT_ORIGIN=""
              if [ -n "''${UCAN_ENDPOINT:-}" ]; then
                ENDPOINT="$UCAN_ENDPOINT"
                echo "dev:web: proxying /ucan/ to $ENDPOINT (from UCAN_ENDPOINT)"
              else
                echo "dev:web: starting a local access service (set UCAN_ENDPOINT to use a remote)..."
                ACCESS_LOG="$(mktemp)"
                # Activation links must open on the page origin (trunk),
                # not on the access service's own port, or the /activate
                # route 404s.
                export ACCESS_PUBLIC_ORIGIN="http://localhost:''${TRUNK_SERVE_PORT:-8080}"
                # Regular runs are ephemeral: the access service keeps its
                # state in memory and a restart starts clean. Export
                # ACCESS_STATE_DIR (e.g. "$PWD/.tonk-dev/access") before
                # dev:web to persist customers, the service key, and a
                # blob snapshot between runs — without it, restarting the
                # service orphans every client holding credentials
                # against it, so clear site data after a restart.
                if [ -n "''${ACCESS_STATE_DIR:-}" ]; then
                  mkdir -p "$ACCESS_STATE_DIR"
                  echo "dev:web: persisting access-service state in $ACCESS_STATE_DIR"
                fi
                # stdout carries the `ACCESS_SERVICE_URL=` line; stderr (build
                # progress) stays on the terminal.
                cargo run --bin tonk-access-local --features helpers >"$ACCESS_LOG" &
                ACCESS_PID=$!
                ENDPOINT=""
                tries=0
                while [ "$tries" -lt 600 ]; do
                  ENDPOINT="$(sed -n 's|^ACCESS_SERVICE_URL=||p' "$ACCESS_LOG" 2>/dev/null | head -n1)"
                  if [ -n "$ENDPOINT" ]; then
                    break
                  fi
                  if ! kill -0 "$ACCESS_PID" 2>/dev/null; then
                    echo "dev:web: the local access service exited before printing its URL" >&2
                    exit 1
                  fi
                  tries=$((tries + 1))
                  sleep 0.5
                done
                if [ -z "$ENDPOINT" ]; then
                  echo "dev:web: timed out waiting for the local access service" >&2
                  kill "$ACCESS_PID" 2>/dev/null || true
                  exit 1
                fi
                # `/@` (the invite shortcut) is served by this same process, so
                # it proxies to the same origin. Kept before `/ucan/` is
                # appended to $ENDPOINT.
                SHORTCUT_ORIGIN="$ENDPOINT"
                ENDPOINT="$ENDPOINT/ucan/"
                # Activation emails are captured, never sent, so sign-up is
                # only completable if the links reach the terminal.
                tail -f "$ACCESS_LOG" | grep --line-buffered "ACCESS_ACTIVATION_EMAIL" &
                echo "dev:web: local access service ready; proxying /ucan/ and /@ to $SHORTCUT_ORIGIN"
              fi
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

              # Trunk takes only one `--proxy-backend`, and its TOML proxies do
              # not interpolate, so `/@` (the invite shortcut) cannot be written
              # into Trunk.toml — the access service's port is only known now.
              # Generate a config that appends it, next to the original so the
              # relative `watch` / target paths still resolve.
              #
              # `no_redirect` (underscore; the hyphen spelling is ignored) hands
              # the shortcut's 301 to the BROWSER, which resolves the relative
              # `Location: /join?…` against the dev origin and carries the short
              # link's `#seed` onto it. Left to follow the redirect itself, trunk
              # would resolve that Location against the access service, which has
              # no `/join` route, and serve its 405.
              TRUNK_CONFIG_GENERATED="./rust/tonk-ui/.Trunk.dev.toml"
              cp ./rust/tonk-ui/Trunk.toml "$TRUNK_CONFIG_GENERATED"
              if [ -n "$SHORTCUT_ORIGIN" ]; then
                # printf, not a heredoc: a heredoc's body has to sit at column
                # zero, which nixfmt then reflows the whole surrounding Nix
                # string around. Beyond `/@`: `/.well-known/tonk` is where the
                # browser reads its service endpoints (trunk otherwise serves
                # index.html for it, and the JSON parse fails as "deployment
                # configuration is invalid"), `/.well-known/did.json` is the
                # service's own DID document — unproxied it answered
                # index.html too, so anything resolving the service identity
                # got HTML where it expected JSON — and `/customer/` is the
                # registration state the worker and account panel read.
                {
                  printf '\n[[proxies]]\nbackend = "%s/@"\nno_redirect = true\n' \
                    "$SHORTCUT_ORIGIN"
                  printf '\n[[proxies]]\nbackend = "%s/.well-known/tonk"\n' \
                    "$SHORTCUT_ORIGIN"
                  printf '\n[[proxies]]\nbackend = "%s/.well-known/did.json"\n' \
                    "$SHORTCUT_ORIGIN"
                  printf '\n[[proxies]]\nbackend = "%s/customer/"\n' \
                    "$SHORTCUT_ORIGIN"
                } >>"$TRUNK_CONFIG_GENERATED"
              else
                echo "dev:web: no local access service, so /@ is unproxied; invite links stay long"
              fi
              trap 'kill "$GUIDE_PID" "$ACCESS_PID" 2>/dev/null; pkill -f "mdbook serve ./guide" 2>/dev/null; rm -f "$TRUNK_CONFIG_GENERATED"' EXIT INT TERM

              trunk serve --config "$TRUNK_CONFIG_GENERATED" --proxy-backend "$ENDPOINT"
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
              test:sw
            '';
          };

          "test:sw" = {
            description = "Service-worker lifecycle tests (update, rollback, caching)";
            # Runs against the SHIPPED `assets/service_worker.js` with
            # stubbed service-worker globals, so it pins the artifact
            # that actually deploys. No browser and no wasm needed —
            # these cover the update/rollback paths whose failure mode
            # is a user stranded on a stale build.
            # An explicit glob, not the directory: node's directory
            # discovery skips `.mjs`, so `--test <dir>` silently runs
            # nothing and still exits non-zero.
            command = "${pkgs.nodejs}/bin/node --test 'rust/tonk-ui/tests/*.test.mjs'";
          };

          "test:e2e" = {
            description = "Run serialized real-browser account integration tests";
            command = ''
              # Both installables come from the Nix store, so cachix serves
              # them warm; rebuilding them in-place with cargo cost every CI
              # run a from-scratch compile, since runners keep no cargo
              # target directory between runs.
              nix build .#tonk-cli .#tests-e2e

              TONK_BIN="$(nix eval .#tonk-cli.outPath --raw)/bin/tonk"
              export TONK_BIN
              # The store-built CLI bakes in the release PostHog key; the
              # debug build the suite spawned before had none. Keep test
              # runs out of the analytics.
              export DO_NOT_TRACK=1

              TESTS_PATH="$(nix eval .#tests-e2e.outPath --raw)"

              # The `e2e` profile (.config/nextest.toml) serializes the
              # suite and holds the quarantine list of known-broken tests.
              cargo nextest run \
                --profile e2e \
                --workspace-remap ./ \
                --archive-file "$TESTS_PATH/tests-e2e.tar.zst" \
                "$@"
            '';
          };

          "test:storybook" = {
            description = "Validate the visual product Storybook and its local links";
            command = ''
              ${pkgs.python3}/bin/python3 docs/storybook/scripts/build.py --check
              ${pkgs.python3}/bin/python3 docs/storybook/scripts/check-links.py docs/storybook
            '';
          };

          "test:storage" = {
            description = "Run the focused real-browser storage regression";
            command = "bash scripts/test-e2e-storage.sh";
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
            description = "Unit tests (wasm32-unknown-unknown, debug, pooled runner)";
            package = "tests-web-debug";
            runner = "${wbg-pool}/bin/wbg-pool";
          };

          "test:web:debug:stock" = menuTestCommand {
            description = "Unit tests (wasm32-unknown-unknown, debug, stock runner)";
            package = "tests-web-debug";
            runner = "${wasm-bindgen-cli}/bin/wasm-bindgen-test-runner";
            clearPoolEnv = true;
          };

          "test:web:release" = menuTestCommand {
            description = "Unit tests (wasm32-unknown-unknown, release, pooled runner)";
            package = "tests-web-release";
            runner = "${wbg-pool}/bin/wbg-pool";
          };

          "test:web:release:stock" = menuTestCommand {
            description = "Unit tests (wasm32-unknown-unknown, release, stock runner)";
            package = "tests-web-release";
            runner = "${wasm-bindgen-cli}/bin/wasm-bindgen-test-runner";
            clearPoolEnv = true;
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

            nix-source-refs = runCommand "nix-source-refs" { nativeBuildInputs = [ ripgrep ]; } ''
              cd ${self}
              bash scripts/check-nix-source-refs.sh .
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
          inherit wbg-pool;

          # Several crates define a `helpers` feature, and a bare
          # `--features helpers` would switch on all of them — including
          # tonk-ui's, which pulls a WebDriver client. `integration-tests`
          # names the ones that stand up a live service, which the
          # access service's HTTP-level tests are `#![cfg]`'d on.
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

          # The real-browser suite `test:e2e` runs. Its `integration-tests`
          # feature pulls the WebDriver stack the shared workspace artifacts
          # never compile, hence its own dependency-only build.
          tests-e2e = buildTestArchive {
            name = "e2e";
            args = "--package tonk-ui --features integration-tests";
            depsExtraArgs = "--package tonk-ui --features integration-tests";
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
            postFixup = ''
              ${./rust/tonk-ui/scripts/stamp-service-worker.sh} "$out"
            '';
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

          # The dependency-free product Storybook, validated and shipped as
          # static assets. Its source map remains in docs/storybook; only the
          # browser explorer is included in the deployed asset bundle.
          tonk-storybook = pkgs.runCommand "tonk-storybook" { nativeBuildInputs = [ pkgs.python3 ]; } ''
            cd ${self}
            python3 docs/storybook/scripts/build.py --check
            python3 docs/storybook/scripts/check-links.py docs/storybook
            mkdir -p $out
            cp -r docs/storybook/app/* $out/
          '';

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
              # Keep the same reviewed Storybook available to the whole team
              # from the deployed Tonk asset origin.
              mkdir -p ./build/tonk-ui/storybook
              cp -r ${tonk-storybook}/* ./build/tonk-ui/storybook/
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
              SERVICE_WORKER_ROOT=''${3:-}

              if [ -n "$SERVICE_WORKER_ROOT" ]; then
                  mkdir -p "$SERVICE_WORKER_ROOT"
                  cp ${self.packages.${system}.tonk-ui}/service_worker.js \
                      "$SERVICE_WORKER_ROOT/service_worker.js"
                  # The package lives in the read-only Nix store and `cp`
                  # preserves that mode. Only this per-harness fixture is
                  # mutable: the upgrade regression rewrites its build stamp
                  # to make the browser discover a successor worker.
                  chmod u+w "$SERVICE_WORKER_ROOT/service_worker.js"
                  SERVICE_WORKER_HANDLE="handle /service_worker.js {
                      root * \"$SERVICE_WORKER_ROOT\"
                      file_server
                  }"
              else
                  SERVICE_WORKER_HANDLE=""
              fi

              echo "Test server live at https://tonk.network:$PORT"
              # `nix run` execs this script, and this exec in turn makes Caddy
              # the process owned by the test helper. Killing its `Child` then
              # cannot orphan a grandchild. Stdin avoids leaking a temp config.
              exec ${caddy}/bin/caddy run --config - --adapter caddyfile << EOF
              {
                  skip_install_trust
                  auto_https disable_redirects
                  servers {
                      protocols h1 h2
                  }
              }
              https://tonk.network:$PORT {
                  tls internal
                  handle /.well-known/tonk {
                      reverse_proxy localhost:$ACCESS_SERVICE_PORT
                  }
                  handle /ucan/* {
                      reverse_proxy localhost:$ACCESS_SERVICE_PORT
                  }
                  handle /customer/* {
                      reverse_proxy localhost:$ACCESS_SERVICE_PORT
                  }
                  $SERVICE_WORKER_HANDLE
                  handle {
                      root * ${self.packages.${system}.tonk-ui}
                      try_files {path} /index.html
                      file_server
                  }
              }
              EOF
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
