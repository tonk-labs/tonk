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
          sha256 = "sha256-lA+P/gO+FPBym55ZoYV9nZiIxCEXAW4tYUi5OQnj/10=";
        };

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

          # Check if all packages are already built
          if [ -f "packages/core/pkg-browser/tonk_core_bg.wasm" ] && \
             [ -f "packages/core-js/dist/index.js" ] && \
             [ -f "packages/keepsync/dist/index.js" ] && \
             [ -f "packages/cli/dist/tonk.js" ] && \
             [ -f "packages/create/dist/create.js" ] && \
             [ -f "packages/host-web/dist/index.html" ]; then
            echo "✓ All packages already built (skipping pnpm build)"
          else
            echo ""
            echo "🔨 Building remaining packages..."
            pnpm build
          fi

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
              echo ""
              echo "This uses the open-source relay included in this repo"
              echo "For development, testing, and local use"
              echo ""
            ''
            + initBuild
            + shellFooter
            + ''
              echo "Internal Developers:"
              echo "  To use the proprietary relay:"
              echo "    nix develop .#withKnot"
              echo "  (Configure TONK_RELAY_BINARY in .envrc.local)"
              echo ""
            '';
        };

        # Internal dev shell
        devShells.withKnot = pkgs.mkShell {
          buildInputs = commonBuildInputs;

          shellHook =
            shellHeader
            + ''
              if [ -n "$TONK_RELAY_BINARY" ] && [ -f "$TONK_RELAY_BINARY" ]; then
                echo "Relay: Proprietary"
                echo ""
                echo "Using proprietary relay from environment variable"
                echo ""
              else
                echo "Relay: Basic"
                echo ""
                echo "WARNING: TONK_RELAY_BINARY not properly configured"
                echo "To use proprietary relay, set the environment variable:"
                echo "  export TONK_RELAY_BINARY=/path/to/proprietary/tonk-relay"
                echo ""
              fi
            ''
            + initBuild
            + shellFooter;
        };
      }
    );
}
