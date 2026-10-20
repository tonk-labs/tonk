#!/bin/sh
# Package the plugin as a zip that `claude --plugin-url`, `--plugin-dir` and
# Claude Desktop's plugin upload accept, optionally pinning its launcher, and
# the MCP bundle that carries it, to one release.
#
#   plugins/tonk/package.sh OUT.zip [RELEASE_TAG]
set -eu
out="$1"
release="${2:-}"
here="$(cd "$(dirname "$0")" && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
cp -R "$here" "$work/tonk"
rm -f "$work/tonk/package.sh" "$work/tonk/package-mcpb.sh"
launcher="$work/tonk/scripts/tonk-agent-peer"
sed -i.bak \
  -e "s#^PINNED_RELEASE=\"\"#PINNED_RELEASE=\"$release\"#" \
  "$launcher"
rm -f "$launcher.bak"
chmod 0755 "$launcher"
# The server the plugin declares is the bundled launcher: pin that copy too.
version="$(sed -n 's/^  "version": "\(.*\)",$/\1/p' "$here/.claude-plugin/plugin.json")"
"$here/package-mcpb.sh" "$version" "$release" "$work/tonk/tonk.mcpb"
case "$out" in /*) ;; *) out="$PWD/$out" ;; esac
(cd "$work/tonk" && zip -qr "$out" .)
