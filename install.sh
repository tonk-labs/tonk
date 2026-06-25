#!/bin/sh
# Install the tonk CLI. Detects your platform, downloads the matching
# release archive, and drops the `tonk` binary on your PATH.
#
# Stable (default):
#   curl -fsSL https://github.com/tonk-labs/tonk/releases/latest/download/install.sh | sh
#
# Pre-release (staging channel):
#   curl -fsSL https://github.com/tonk-labs/tonk/releases/latest/download/install.sh | TONK_CHANNEL=staging sh
#
# Environment:
#   TONK_CHANNEL=staging   install the pre-release channel (default: stable)
#   TONK_RELEASE=<tag>     pin an explicit release tag (wins over TONK_CHANNEL)
#   TONK_INSTALL_DIR       where to install (default: $HOME/.local/bin)
#
# The macOS binary is not Apple-signed (see BUG: Apple code signing). This
# script clears the Gatekeeper quarantine and ad-hoc signs the binary so it
# runs; a hand-downloaded binary needs the same `xattr -c` + `codesign`.
set -eu

REPO="tonk-labs/tonk"
INSTALL_DIR="${TONK_INSTALL_DIR:-$HOME/.local/bin}"

say() { printf 'install: %s\n' "$1" >&2; }
die() { printf 'install: error: %s\n' "$1" >&2; exit 1; }

# Channel selection. Only the exact value `staging` opts into the
# pre-release; TONK_RELEASE pins an explicit tag and wins over the channel.
if [ "${TONK_CHANNEL:-}" = "staging" ]; then
  channel="staging"
else
  channel="stable"
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

chmod +x "$tmp/tonk"

mkdir -p "$INSTALL_DIR"
mv "$tmp/tonk" "$INSTALL_DIR/tonk"
dest="$INSTALL_DIR/tonk"

# macOS-only Gatekeeper handling. The tonk binary is not Apple-signed, so
# Gatekeeper would otherwise quarantine it (and, on recent macOS, can kill
# it on first launch). Two steps make an unsigned binary runnable:
#   1. clear every extended attribute, including com.apple.quarantine;
#   2. apply an ad-hoc signature, which arm64 binaries require to execute
#      and which is re-established after the move/clear.
# Both are no-ops off macOS (the tools are absent), so the guard is `Darwin`.
if [ "$os" = "Darwin" ]; then
  xattr -c "$dest" 2>/dev/null || true
  if command -v codesign >/dev/null 2>&1; then
    codesign --force --sign - "$dest" 2>/dev/null || true
  fi
fi

say "installed tonk to $dest"

case ":$PATH:" in
  *":$INSTALL_DIR:"*) ;;
  *) say "note: $INSTALL_DIR is not on your PATH; add it, e.g. export PATH=\"$INSTALL_DIR:\$PATH\"" ;;
esac

# Confirm the binary actually runs; if macOS still blocks it, surface the
# exact recovery command instead of leaving a silent broken install.
if ! "$dest" --version >/dev/null 2>&1; then
  say "warning: '$dest --version' did not run cleanly."
  if [ "$os" = "Darwin" ]; then
    say "if macOS blocked it, run: xattr -c \"$dest\" && codesign --force --sign - \"$dest\""
    say "or allow it once under System Settings > Privacy & Security."
  fi
else
  "$dest" --version 2>/dev/null || true
fi
