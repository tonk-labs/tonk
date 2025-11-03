{
  description = "Tonk Monorepo";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    fenix = {
      url = "github:nix-community/fenix";
      inputs.nixpkgs.follows = "nixpkgs";
    };

    # Optional: Reference to private knot repo for proprietary relay binary
    # Only needed for internal developers
    # Override locally with: nix develop .#withKnot --override-input knot path:../knot
    knot = {
      url = "git+ssh://git@github.com/tonk-labs/knot.git";
      # Mark as optional - won't fail if unavailable
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      self,
      nixpkgs,
      flake-utils,
      fenix,
      knot,
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
        fenixPkgs = fenix.packages.${system};

        rustToolchain = fenixPkgs.fromToolchainFile {
          file = ./packages/core/rust-toolchain.toml;
          sha256 = "sha256-lA+P/gO+FPBym55ZoYV9nZiIxCEXAW4tYUi5OQnj/10=";
        };

        # Local OSS relay path
        localRelay = ./packages/relay/target/release/tonk-relay;

        # Proprietary relay from knot (if available)
        knotRelay = knot.packages.${system}.tonk-relay or knot.packages.${system}.default or null;

        # Common build inputs for all dev shells
        commonBuildInputs = with pkgs; [
          # Rust toolchain with rust-src and wasm target
          rustToolchain

          # Node.js and package managers
          nodejs_22
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

        # Common shell hook header
        shellHeader = ''
          echo "═══════════════════════════════════════════════════"
          echo "  Tonk Development Environment"
          echo "═══════════════════════════════════════════════════"
          echo ""
          echo "Node.js:     $(node --version)"
          echo "pnpm:        $(pnpm --version)"
          echo "TypeScript:  $(tsc --version)"
          echo "Rust:        $(rustc --version)"
          echo ""
        '';

        # Common shell hook footer
        shellFooter = ''
          echo ""
          echo "Project Structure:"
          echo "  packages/cli         - Tonk CLI tool"
          echo "  packages/core        - Tonk CRDT core (Rust)"
          echo "  packages/core-js     - JavaScript bindings"
          echo "  packages/keepsync    - Reactive sync engine"
          echo "  packages/host-web    - Web host environment"
          echo "  packages/relay       - Open-source relay server"
          echo "  packages/create      - Project bootstrapping"
          echo "  examples/            - Example applications"
          echo ""
          echo "Quick Start:"
          echo "  • pnpm install                           - Install dependencies"
          echo "  • cd packages/relay && cargo build       - Build OSS relay"
          echo "  • cd examples/demo && pnpm run dev       - Start demo"
          echo "  • cd packages/sprinkles && pnpm run dev  - Dev environment"
          echo ""
        '';
      in
      {
        # Default dev shell - uses local OSS relay
        devShells.default = pkgs.mkShell {
          buildInputs = commonBuildInputs;

          shellHook = shellHeader + ''
            echo "Relay: Open Source (local build)"
            echo "Location: packages/relay/target/release/tonk-relay"
            echo ""
            echo "This uses the FREE open-source relay included in this repo."
            echo "Perfect for development, testing, and small deployments!"
          '' + shellFooter + ''
            echo "Internal Developers:"
            echo "  To use the proprietary optimized relay from knot:"
            echo "    nix develop .#withKnot --override-input knot path:../knot"
            echo ""
          '';
        };

        # Internal dev shell - uses proprietary relay from knot
        devShells.withKnot = pkgs.mkShell {
          buildInputs = commonBuildInputs ++ pkgs.lib.optional (knotRelay != null) knotRelay;

          shellHook = shellHeader + (if knotRelay != null then ''
            echo "Relay: Proprietary (from knot)"
            echo "Location: ${knotRelay}/bin/tonk-relay"
            echo ""
            echo "Using PROPRIETARY optimized relay from knot repository."
            echo "This version includes performance optimizations and enterprise features."
            echo ""

            # Set environment variable for proprietary relay binary
            export TONK_RELAY_BINARY="${knotRelay}/bin/tonk-relay"
          '' else ''
            echo "Relay: Open Source (fallback - knot not available)"
            echo "Location: packages/relay/target/release/tonk-relay"
            echo ""
            echo "WARNING: knot repository not available."
            echo "Falling back to local OSS relay."
            echo ""
            echo "To use proprietary relay:"
            echo "  nix develop .#withKnot --override-input knot path:../knot"
            echo ""
          '') + shellFooter;
        };
      }
    );
}
