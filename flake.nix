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

          # Copy relay binary to platform-specific JS package
          RELAY_BINARY="packages/relay/target/release/tonk-relay"
          if [ -f "$RELAY_BINARY" ]; then
            ARCH=$(uname -m)
            case "$ARCH" in
              arm64|aarch64) PLATFORM_PKG="packages/relay-js-darwin-arm64" ;;
              x86_64) PLATFORM_PKG="packages/relay-js-darwin-x64" ;;
              *) PLATFORM_PKG="" ;;
            esac

            if [ -n "$PLATFORM_PKG" ] && [ -d "$PLATFORM_PKG" ]; then
              mkdir -p "$PLATFORM_PKG/bin"
              if [ ! -f "$PLATFORM_PKG/bin/tonk-relay" ] || [ "$RELAY_BINARY" -nt "$PLATFORM_PKG/bin/tonk-relay" ]; then
                echo ""
                echo "📦 Copying relay binary to $PLATFORM_PKG..."
                cp "$RELAY_BINARY" "$PLATFORM_PKG/bin/tonk-relay"
                chmod +x "$PLATFORM_PKG/bin/tonk-relay"
              else
                echo "✓ Relay binary already in $PLATFORM_PKG"
              fi
            fi
          fi

          echo ""
          echo "✅ Development environment ready!"
          echo ""
        '';

        # Common shell hook footer
        shellFooter = ''
          echo "Project Structure:"
          echo "  packages/core        - Tonk CRDT core (Rust)"
          echo "  packages/core-js     - JavaScript bindings"
          echo "  packages/relay       - Basic relay server"
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
