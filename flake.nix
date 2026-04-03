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

        wrangler = wrangler-flake.packages.${system}.wrangler;

        commonBuildInputs =
          with pkgs;
          [
            binaryen
            cachix
            cargo-nextest
            esbuild
            mdbook
            worker-build
          ]
          ++ lib.optionals stdenv.isLinux [
            openssl
            pkg-config
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
          buildTestArchive
          cargoChecks
          rustToolchain
          wasm-bindgen-cli
          ;

        devShellBuildInputs = commonBuildInputs ++ [
          pkgs.cachix
          wrangler
          rustToolchain
          wasm-bindgen-cli
        ];

        devShellEnvVars = with pkgs; {
          "WASM_BINDGEN_BIN" = "${wasm-bindgen-cli}/bin/wasm-bindgen";
          "ESBUILD_BIN" = "${esbuild}/bin/esbuild";
          "WASM_OPT_BIN" = "${binaryen}/bin/wasm-opt";
        };

        menuHelpers = (import ./nix/menu.nix { inherit pkgs; });

        inherit (menuHelpers) makeMenu makeDevShellHook menuTestCommand;

        commands = {
          "lint" = {
            description = "Lint the full source tree";
            command = "nix flake check";
          };
          "test:native:debug" = menuTestCommand {
            description = "Unit and integration tests (${system}, debug)";
            package = "tests-native-debug";
          };
          "test:native:release" = menuTestCommand {
            description = "Unit and integration tests (${system}, release)";
            package = "tests-native-release";
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

        packages = {
          tests-native-debug = buildTestArchive {
            name = "native-debug";
            args = "--workspace --exclude tonk-access-service";
          };

          tests-native-release = buildTestArchive {
            name = "native-release";
            args = "--workspace --exclude tonk-access-service --release";
          };

          tests-cli-integration = buildTestArchive {
            name = "cli-integration";
            args = "--package carry --test cli_integration --bin carry";
          };

          carry = buildCrate {
            pname = "carry";
            cargoExtraArgs = "--package carry";
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
        };
      }
    );

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
