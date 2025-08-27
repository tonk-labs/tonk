{
  description = "Tonk Monorepo";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    fenix = {
      url = "github:nix-community/fenix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      self,
      nixpkgs,
      flake-utils,
      fenix,
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
        fenixPkgs = fenix.packages.${system};

        rustToolchain = fenixPkgs.fromToolchainFile {
          file = ./packages/core/rust-toolchain.toml;
          sha256 = "sha256-X41CCI3xmeeqwWoJ6f+KOjAgiWFHX3SJnWIOgVxKJOo=";
        };
      in
      {
        devShells.default = pkgs.mkShell {
          buildInputs = with pkgs; [
            # Rust toolchain with rust-src and wasm target
            rustToolchain

            # Node.js and package managers
            nodejs_20
            pnpm_9

            # TypeScript and build tools
            typescript

            # Development tools
            git

            # Docker
            docker
            docker-compose

            # Rust + WASM
            wasm-pack
            llvmPackages.bintools
          ];

          shellHook = ''
            echo "Tonk Monorepo"
            echo "================================"
            echo "Node.js version: $(node --version)"
            echo "pnpm version: $(pnpm --version)"
            echo "TypeScript version: $(tsc --version)"
            echo "Rust version: $(rustc --version)"
            echo ""
            echo "Project Structure:"
            echo "  packages/cli     - Tonk CLI tool"
            echo "  packages/server  - Tonk server package"
            echo "  packages/keepsync - Reactive sync engine"
            echo "  packages/create  - Project bootstrapping tool"
            echo "  examples/        - Example applications"
            echo ""
          '';
        };
      }
    );
}
