{
  description = "Tonk Monorepo";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    fenix = {
      url = "github:nix-community/fenix";
      inputs.nixpkgs.follows = "nixpkgs";
    };

    # Optional: Reference to knot repo for proprietary relay binary
    # Override locally with: nix develop .#withKnot --override-input knot path:../knot
    knot = {
      url = "git+ssh://git@github.com/tonk-labs/knot.git";
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

        # Get knot relay if available
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

        # Build initialization script
        initBuild = ''
          echo "Initializing development environment..."
          echo ""

          # Install dependencies
          if [ ! -d "node_modules" ]; then
            echo "📦 Installing dependencies..."
            pnpm install --frozen-lockfile
          else
            echo "✓ Dependencies already installed"
          fi

          # Build Rust WASM package
          if [ ! -d "packages/core/pkg-browser" ]; then
            echo ""
            echo "🦀 Building Rust WASM package (packages/core)..."
            cd packages/core && pnpm run build:browser && cd ../..
          else
            echo "✓ WASM package already built"
          fi

          # Build TypeScript wrapper
          if [ ! -d "packages/core-js/dist" ]; then
            echo ""
            echo "📘 Building TypeScript wrapper (packages/core-js)..."
            cd packages/core-js && pnpm run build && cd ../..
          else
            echo "✓ TypeScript wrapper already built"
          fi

          # Build relay binary
          if [ ! -f "packages/relay/target/release/tonk-relay" ]; then
            echo ""
            echo "🚀 Building relay server (packages/relay)..."
            cd packages/relay && cargo build --release && cd ../..
          else
            echo "✓ Relay server already built"
          fi

          # Build all packages
          echo ""
          echo "🔨 Building all packages..."
          pnpm build

          echo ""
          echo "✅ Development environment ready!"
          echo ""
        '';

        # Common shell hook footer
        shellFooter = ''
          echo "Project Structure:"
          echo "  packages/cli         - Tonk CLI tool"
          echo "  packages/core        - Tonk CRDT core (Rust)"
          echo "  packages/core-js     - JavaScript bindings"
          echo "  packages/keepsync    - Reactive sync engine"
          echo "  packages/host-web    - Web host environment"
          echo "  packages/relay       - Basic relay server"
          echo "  packages/create      - Project bootstrapping"
          echo "  examples/            - Example applications"
          echo ""
          echo "Quick Start:"
          echo "  • pnpm install       - Install dependencies"
          echo "  • pnpm build         - Build all packages"
          echo "  • pnpm test          - Run tests"
          echo "  • cd examples/demo && pnpm run dev  - Start demo"
          echo ""
        '';
      in
      {
        # Default dev shell - uses basic relay
        devShells.default = pkgs.mkShell {
          buildInputs = commonBuildInputs;

          shellHook =
            shellHeader
            + ''
              echo "Relay: Basic"
              echo "Location: packages/relay/target/release/tonk-relay"
              echo ""
              echo "This uses the open-source relay included in this repo"
              echo "For development, testing, and small deployments"
              echo ""
            ''
            + initBuild
            + shellFooter
            + ''
              echo "Internal Developers:"
              echo "  To use the proprietary relay from knot:"
              echo "    nix develop .#withKnot --override-input knot path:../knot"
              echo ""
            '';
        };

        # Internal dev shell - uses proprietary relay from knot
        devShells.withKnot = pkgs.mkShell {
          buildInputs = commonBuildInputs ++ pkgs.lib.optional (knotRelay != null) knotRelay;

          shellHook =
            shellHeader
            + (
              if knotRelay != null then
                ''
                  echo "Relay: Proprietary (from knot)"
                  echo "Location: ${knotRelay}/bin/tonk-relay"
                  echo ""
                  echo "Using proprietary relay from knot repository"
                  echo "This version includes performance optimisations and additional security"
                  echo ""

                  # Set environment variable for proprietary relay binary
                  export TONK_RELAY_BINARY="${knotRelay}/bin/tonk-relay"
                ''
              else
                ''
                  echo "Relay: Basic (fallback - knot not available)"
                  echo "Location: packages/relay/target/release/tonk-relay"
                  echo ""
                  echo "WARNING: knot repository not available"
                  echo "Falling back to local OSS relay"
                  echo ""
                  echo "To use proprietary relay:"
                  echo "  nix develop .#withKnot --override-input knot path:../knot"
                  echo ""
                ''
            )
            + initBuild
            + shellFooter;
        };
      }
    );
}
