#!/bin/sh
# Install the tonk CLI. Detects your platform, downloads the matching
# release archive, and drops the `tonk` binary on your PATH.
#
# Stable (default):
#   curl -fsSL https://github.com/tonk-labs/tonk/releases/latest/download/install.sh | sh
#
# Pre-release (staging channel) — either way:
#   curl -fsSL https://github.com/tonk-labs/tonk/releases/latest/download/install.sh | sh -s -- --staging
#   curl -fsSL https://github.com/tonk-labs/tonk/releases/latest/download/install.sh | TONK_CHANNEL=staging sh
#
# Flags:
#   --stable   install the latest stable release (default)
#   --staging  install the latest staging pre-release (alias: --pre)
#
# Environment overrides (env wins over flags):
#   TONK_CHANNEL=staging   install the pre-release channel
#   TONK_RELEASE=<tag>     pin an explicit release tag (wins over everything)
#   TONK_INSTALL_DIR       where to install (default: $HOME/.local/bin)
set -eu

REPO="tonk-labs/tonk"
INSTALL_DIR="${TONK_INSTALL_DIR:-$HOME/.local/bin}"

say() { printf 'install: %s\n' "$1" >&2; }
die() { printf 'install: error: %s\n' "$1" >&2; exit 1; }

# Channel selection, lowest to highest precedence:
#   default stable -> --staging/--stable flag -> TONK_CHANNEL env -> TONK_RELEASE tag.
channel="stable"
while [ $# -gt 0 ]; do
  case "$1" in
    --staging|--pre|--prerelease) channel="staging" ;;
    --stable|--latest) channel="stable" ;;
    -h|--help)
      say "usage: install.sh [--stable | --staging]"
      exit 0
      ;;
    *) die "unknown argument: $1 (expected --stable or --staging)" ;;
  esac
  shift
done

# Env channel overrides the flag. Only the exact value `staging` opts into
# the pre-release; any other value (or unset) leaves the flag's choice.
if [ "${TONK_CHANNEL:-}" = "staging" ]; then
  channel="staging"
fi

if [ -n "${TONK_RELEASE:-}" ]; then
  RELEASE="$TONK_RELEASE"
elif [ "$channel" = "staging" ]; then
  RELEASE="tonk-staging"
else
  RELEASE="latest"
fi
say "channel: $channel (release: $RELEASE)"

# Map uname output to a release asset platform slug.
os="$(uname -s)"
arch="$(uname -m)"
case "$os" in
  Darwin)
    case "$arch" in
      arm64|aarch64) platform="macos-arm64" ;;
      *) die "unsupported macOS architecture: $arch (only Apple Silicon is published)" ;;
    esac
    ;;
  Linux)
    case "$arch" in
      x86_64|amd64) platform="linux-x86_64" ;;
      *) die "unsupported Linux architecture: $arch (only x86_64 is published)" ;;
    esac
    ;;
  *)
    die "unsupported OS: $os"
    ;;
esac

asset="tonk-${platform}.tar.gz"
if [ "$RELEASE" = "latest" ]; then
  url="https://github.com/${REPO}/releases/latest/download/${asset}"
else
  url="https://github.com/${REPO}/releases/download/${RELEASE}/${asset}"
fi

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

say "downloading $asset"
if command -v curl >/dev/null 2>&1; then
  curl -fsSL "$url" -o "$tmp/$asset" || die "download failed: $url"
elif command -v wget >/dev/null 2>&1; then
  wget -qO "$tmp/$asset" "$url" || die "download failed: $url"
else
  die "need curl or wget to download"
fi

say "extracting"
tar -xzf "$tmp/$asset" -C "$tmp" || die "extract failed"
[ -f "$tmp/tonk" ] || die "archive did not contain a 'tonk' binary"

# Clear the macOS quarantine attribute so Gatekeeper doesn't block the
# unsigned binary on first run. No-op (and absent) off macOS.
if command -v xattr >/dev/null 2>&1; then
  xattr -d com.apple.quarantine "$tmp/tonk" 2>/dev/null || true
fi
chmod +x "$tmp/tonk"

mkdir -p "$INSTALL_DIR"
mv "$tmp/tonk" "$INSTALL_DIR/tonk"
say "installed tonk to $INSTALL_DIR/tonk"

case ":$PATH:" in
  *":$INSTALL_DIR:"*) ;;
  *) say "note: $INSTALL_DIR is not on your PATH; add it, e.g. export PATH=\"$INSTALL_DIR:\$PATH\"" ;;
esac

"$INSTALL_DIR/tonk" --version 2>/dev/null || true
