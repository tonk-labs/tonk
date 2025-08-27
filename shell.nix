{
  pkgs ? import <nixpkgs> { },
}:

pkgs.mkShell {
  buildInputs = with pkgs; [
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
    echo ""
    echo "Project Structure:"
    echo "  packages/cli     - Tonk CLI tool"
    echo "  packages/server  - Tonk server package"
    echo "  packages/keepsync - Reactive sync engine"
    echo "  packages/create  - Project bootstrapping tool"
    echo "  examples/        - Example applications"
    echo ""
  '';
}
