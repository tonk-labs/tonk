# Installation

## Quick Install (recommended)

The fastest way to install Carry is with the install script. It detects your platform, downloads the right binary, and installs shell completions automatically:

```bash
curl -fsSL https://raw.githubusercontent.com/tonk-labs/tonk/feat/carry/install.sh | sh
```

This installs the `carry` binary to `/usr/local/bin` (you may be prompted for your password) and sets up shell completions for your current shell (zsh, bash, or fish).

It's always good practice to inspect a script before running it on your machine. You can view it [here](https://raw.githubusercontent.com/tonk-labs/tonk/feat/carry/install.sh).

### Uninstall

To remove Carry and its shell completions:

```bash
curl -fsSL https://raw.githubusercontent.com/tonk-labs/tonk/feat/carry/install.sh | sh -s -- uninstall
```

## From Nix

If you have [Nix](https://nixos.org/) installed, you can build Carry from the Tonk flake:

```bash
nix build github:tonk-labs/tonk#carry
```

This produces a standalone binary at `./result/bin/carry`. Copy it to somewhere on your `$PATH`:

```bash
cp ./result/bin/carry ~/.local/bin/
```

## From Source

Clone the repository and build with Cargo:

```bash
git clone https://github.com/tonk-labs/tonk.git
cd tonk
cargo build --release --package carry
```

The binary will be at `target/release/carry`.

### Using the Nix development shell

If you're working on the Tonk codebase, the Nix flake provides a complete development environment:

```bash
cd tonk
nix develop
cargo build --package carry
```

## Verify Installation

```bash
carry --help
```

You should see the Carry help output describing available commands and key concepts.

## Shell Completions

The install script sets up shell completions automatically for zsh, bash, and fish. If you installed via another method, Carry supports completions via `clap_complete`. Generate them by running:

```bash
COMPLETE=zsh carry    # for zsh
COMPLETE=bash carry   # for bash
COMPLETE=fish carry   # for fish
```

Redirect the output to the appropriate completions directory for your shell.
