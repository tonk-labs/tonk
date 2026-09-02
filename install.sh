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
#   TONK_INSTALL_DIR       where to install (default: /usr/local/bin if
#                          writable, else $HOME/.local/bin)
#
# macOS binaries from the current release workflows are Developer ID signed and
# notarized. Installation must preserve those exact bytes for Gatekeeper.
set -eu

REPO="tonk-labs/tonk"

say() { printf 'install: %s\n' "$1" >&2; }
die() { printf 'install: error: %s\n' "$1" >&2; exit 1; }

# Install location: honor the override, else prefer a writable /usr/local/bin
# (usually already on PATH) and fall back to the per-user bin.
if [ -n "${TONK_INSTALL_DIR:-}" ]; then
  INSTALL_DIR="$TONK_INSTALL_DIR"
elif [ -w /usr/local/bin ]; then
  INSTALL_DIR="/usr/local/bin"
else
  INSTALL_DIR="$HOME/.local/bin"
fi

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

# Resolve the checksums.txt URL alongside the asset on the same release.
if [ "$RELEASE" = "latest" ]; then
  sums_url="https://github.com/${REPO}/releases/latest/download/checksums.txt"
else
  sums_url="https://github.com/${REPO}/releases/download/${RELEASE}/checksums.txt"
fi

fetch() {
  # fetch <url> <output> -> 0 on success, non-zero otherwise.
  if command -v curl >/dev/null 2>&1; then
    curl -fsSL "$1" -o "$2"
  elif command -v wget >/dev/null 2>&1; then
    wget -qO "$2" "$1"
  else
    die "need curl or wget to download"
  fi
}

say "downloading $asset"
fetch "$url" "$tmp/$asset" || die "download failed: $url"

# Verify the archive against the release's checksums.txt. Refuse to install
# on mismatch; this protects the archive independently of platform signing.
sha256_of() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | awk '{print $1}'
  elif command -v openssl >/dev/null 2>&1; then
    openssl dgst -sha256 "$1" | awk '{print $NF}'
  else
    return 1
  fi
}

if fetch "$sums_url" "$tmp/checksums.txt"; then
  expected="$(awk -v f="$asset" '{name=$2; sub(/^\*/, "", name); if (name == f) {print $1; exit}}' "$tmp/checksums.txt")"
  [ -n "$expected" ] || die "no checksum entry for $asset in checksums.txt"
  actual="$(sha256_of "$tmp/$asset")" || die "no sha256 tool found (need sha256sum, shasum, or openssl)"
  [ "$expected" = "$actual" ] || die "checksum mismatch for $asset (expected $expected, got $actual)"
  say "checksum verified"
else
  die "could not download checksums.txt from $sums_url; refusing to install unverified binary"
fi

say "extracting"
tar -xzf "$tmp/$asset" -C "$tmp" || die "extract failed"
[ -f "$tmp/tonk" ] || die "archive did not contain a 'tonk' binary"

chmod +x "$tmp/tonk"

mkdir -p "$INSTALL_DIR"
mv "$tmp/tonk" "$INSTALL_DIR/tonk"
dest="$INSTALL_DIR/tonk"

say "installed tonk to $dest"

# Record what we installed so `tonk update` preserves this channel and
# can answer "already current" without downloading an archive.
# Best-effort: an older release has no manifest.json, and a receipt we
# cannot write is not a reason to fail an install.
#
# Mirrors `update::receipt::Receipt`; TONK_UPDATE_STATE overrides the
# directory for tests, matching the CLI.
if [ -n "${TONK_UPDATE_STATE:-}" ]; then
  state_dir="$TONK_UPDATE_STATE"
elif [ "$os" = "Darwin" ]; then
  state_dir="$HOME/Library/Application Support/tonk"
else
  state_dir="${XDG_DATA_HOME:-$HOME/.local/share}/tonk"
fi

# Escape a value for use inside a JSON string: backslashes first, then
# quotes. An install dir containing either would otherwise emit JSON the
# CLI silently fails to parse (it reads the receipt with `.ok()`).
json_escape() {
  printf '%s' "$1" | sed -e 's/\\/\\\\/g' -e 's/"/\\"/g'
}

# Everything that can fail lives here, and it is only ever called as an
# `if` condition, so a failure skips the receipt instead of aborting an
# install whose binary is already in place. `mkdir -p` succeeds on an
# existing directory even when it is unwritable, so the write itself has
# to be guarded, not just the mkdir.
write_receipt() {
  mkdir -p "$state_dir" 2>/dev/null || return 1
  cat > "$state_dir/install.json" 2>/dev/null <<EOF || return 1
{
  "channel": "$channel",
  "version": "$m_version",
  "commit": "$m_commit",
  "install_dir": "$(json_escape "$INSTALL_DIR")",
  "installed_at": "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
}
EOF
  return 0
}

if fetch "${url%/*}/manifest.json" "$tmp/manifest.json" 2>/dev/null; then
  # Pull two string fields out without requiring jq.
  m_version="$(sed -n 's/.*"version"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$tmp/manifest.json")"
  m_commit="$(sed -n 's/.*"commit"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$tmp/manifest.json")"
  if [ -n "$m_version" ] && [ -n "$m_commit" ]; then
    # 2>/dev/null on the call, not just inside: the shell opens the
    # redirect target before `cat` runs, so `cat`'s own redirect cannot
    # suppress a "Permission denied" for an unwritable state dir. This is
    # best-effort, so it stays silent.
    if write_receipt 2>/dev/null; then
      say "recorded install receipt in $state_dir"
    fi
  fi
fi

case ":$PATH:" in
  *":$INSTALL_DIR:"*) ;;
  *) say "note: $INSTALL_DIR is not on your PATH; add it, e.g. export PATH=\"$INSTALL_DIR:\$PATH\"" ;;
esac

# Confirm the binary actually runs instead of leaving a silent broken install.
if ! "$dest" --version >/dev/null 2>&1; then
  say "warning: '$dest --version' did not run cleanly."
  if [ "$os" = "Darwin" ]; then
    say "check your network, then allow it under System Settings > Privacy & Security if macOS blocked it."
  fi
else
  "$dest" --version 2>/dev/null || true
fi
