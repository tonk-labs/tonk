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
    { self
    , crane
    , nixpkgs
    , flake-utils
    , rust-overlay
    , nix-filter
    , wrangler-flake

    ,
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ (import rust-overlay) ];
        };
        filter = nix-filter.lib;

        # Use nixpkgs#wasm-bindgen-cli to avoid building it (it is slow!)
        wasm-bindgen-cli = pkgs.wasm-bindgen-cli_0_2_100;

        # We get wrangler from a 3P crate because nixpkgs#wrangler lags
        # the latest release
        wrangler = wrangler-flake.packages.${system}.wrangler;


        # Common build inputs for all dev shells
        commonBuildInputs =
          with pkgs;
          [
            tailwindcss_4
            trunk
            cachix
            cargo-nextest
            wasm-bindgen-cli
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

        # Import rust helpers
        rustHelpers = import ./nix/rust.nix {
          inherit pkgs filter crane;
          buildInputs = commonBuildInputs;
          workspaceRoot = ./.;
        };

        inherit (rustHelpers) buildTrunkCrate buildTestArchive cargoChecks rustToolchain;

        # Include the Rust toolchain and some extras in build inputs
        # for dev shells
        devShellBuildInputs = commonBuildInputs ++ [
          wrangler
          rustToolchain
        ];

        # Import menu helpers (e.g., colorful Tonk Shell commands)
        menuHelpers = (import ./nix/menu.nix { inherit pkgs; });

        inherit (menuHelpers) makeMenu makeDevShellHook menuTestCommand;

        commands = {
          "build:web" = {
            description = "Build the Tonk web application";
            command = "nix build .#tonk-ui";
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
        # Building 3P wrangler is slow; this configures pulling from a cache
        # SEE: https://github.com/emrldnix/wrangler?tab=readme-ov-file#using-the-nar-cache
        nix.settings = {
          substituters = [ "https://wrangler.cachix.org" ];
          trusted-public-keys = [ "wrangler.cachix.org-1:N/FIcG2qBQcolSpklb2IMDbsfjZKWg+ctxx0mSMXdSs=" ];
        };

        checks = cargoChecks;


        devShells = with pkgs; {
          default = mkShell {
            buildInputs = devShellBuildInputs;
            nativeBuildInputs = menu.commands;
            env = lib.optionalAttrs stdenv.isLinux {
              "CHROMEDRIVER" = "${chromedriver}/bin/chromedriver";
            };
            shellHook = makeDevShellHook menu;
          };

          ci = mkShell {
            buildInputs = devShellBuildInputs;
            nativeBuildInputs = menu.commands;
            env = lib.optionalAttrs stdenv.isLinux {
              "CHROME" = "${chromium}/bin/chromium";
              "CHROMEDRIVER" = "${chromedriver}/bin/chromedriver";
            };
          };
        };

        packages =
          {
            tests-native-debug = buildTestArchive {
              name = "native-debug";
              args = "--features integration-tests";
            };

            tests-native-release = buildTestArchive {
              name = "native-release";
              args = "--features integration-tests";
            };

            tests-web-debug = buildTestArchive {
              name = "web-debug";
              target = "wasm32-unknown-unknown";
            };

            tests-web-release = buildTestArchive {
              name = "web-release";
              target = "wasm32-unknown-unknown";
            };

            tests = pkgs.runCommand "tests-all" { } ''
              mkdir -p $out
              cp ${self.packages.${system}.tests-native-debug}/*.tar.zst $out/
              cp ${self.packages.${system}.tests-native-release}/*.tar.zst $out/
              cp ${self.packages.${system}.tests-web-debug}/*.tar.zst $out/
              cp ${self.packages.${system}.tests-web-release}/*.tar.zst $out/
            '';

            tonk-ui = buildTrunkCrate {
              pname = "tonk-ui";
              trunkConfig = "./rust/tonk-ui/Trunk.toml";

              inherit wasm-bindgen-cli;
            };

            # This package is used by integration tests to run a web server
            # over a local deployment of tonk-ui
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
