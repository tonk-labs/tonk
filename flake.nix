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

        # Common build inputs for all dev shells
        commonBuildInputs = with pkgs; [
          rustToolchainStable
          bun
        ] ++ lib.optionals stdenv.isLinux [
          # Linux-specific inputs
        ] ++ lib.optionals stdenv.isDarwin [
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
          clippy = pkgs.rustPlatform.buildRustPackage {
            pname = "tonk-clippy-lint";
            version = "0.1.0";
            src = ./.;
            cargoLock = {
              lockFile = ./Cargo.lock;
            };
            nativeBuildInputs = [ rustToolchainStable ];
            buildPhase = ''
              cargo clippy --all-targets --all-features -- -D warnings
            '';
            installPhase = ''
              touch $out
            '';
          };

          rustfmt = pkgs.runCommand "tonk-fmt-check"
            {
              nativeBuildInputs = [ rustToolchainStable ];
            } ''
            cd ${./.}
            cargo fmt --check
            touch $out
          '';
        };

        packages = {
          tonk-core = pkgs.rustPlatform.buildRustPackage {
            pname = "tonk-core";
            version = "0.1.0";
            src = ./.;
            cargoLock = {
              lockFile = ./Cargo.lock;
            };
            nativeBuildInputs = [ rustToolchainStable ];
            buildPhase = ''
              cargo clippy --all-targets --all-features -- -D warnings
            '';
            installPhase = ''
              touch $out
            '';
          };
        };
      }
    );
}
