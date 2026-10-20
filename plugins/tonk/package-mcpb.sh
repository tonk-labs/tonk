#!/bin/sh
# Package the Tonk Desktop extension (tonk-peer.mcpb): the plugin's launcher in
# Claude Desktop extension packaging.
#
#   plugins/tonk/package-mcpb.sh VERSION RELEASE_TAG OUT.mcpb
#
# Releases publish it next to the plugin, for Claude Desktop chat: Desktop
# renders MCP Apps views only from servers it runs as extensions (or from
# claude_desktop_config.json), never from a plugin's server, even one the
# plugin bundles as a .mcpb. See docs/claude-desktop-plugin-internals.md.
#
# Desktop runs the launcher itself (it is executable, with a #!/bin/sh line),
# as it runs a bundled binary: the views of an extension it started as
# `/bin/sh <launcher>` did not render in our tests, while the same launcher
# started directly did.
#
# The bundle carries the launcher, not the peer itself: on every start it
# fetches the newest verified peer build (and the guest runtime for live
# views), so one install keeps itself current, and the downloaded binary is
# never quarantined.
set -eu
version="$1"
release="$2"
out="$3"
here="$(cd "$(dirname "$0")" && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
mkdir -p "$work/server"
cp "$here/icon.png" "$work/icon.png"
sed -e "s#^PINNED_RELEASE=\"\"#PINNED_RELEASE=\"$release\"#" \
  "$here/scripts/tonk-agent-peer" > "$work/server/tonk-agent-peer"
chmod 0755 "$work/server/tonk-agent-peer"
grep -q "^PINNED_RELEASE=\"$release\"" "$work/server/tonk-agent-peer"
cat > "$work/manifest.json" <<EOF
{
  "manifest_version": "0.3",
  "name": "tonk-agent-peer",
  "display_name": "Tonk",
  "version": "$version",
  "description": "Work in shared Tonk spaces through a local peer, with live views in the conversation.",
  "long_description": "Paste a Tonk space's share link and Claude joins it under this computer's own peer identity. Claude can then read and write the space, and show its things in the conversation rendered by Tonk itself, live. It updates itself from ${release:-the latest main build} each time it starts.",
  "author": { "name": "Tonk Labs", "url": "https://github.com/tonk-labs" },
  "homepage": "https://github.com/tonk-labs/tonk/tree/main/plugins/tonk",
  "repository": { "type": "git", "url": "https://github.com/tonk-labs/tonk" },
  "license": "MIT",
  "icon": "icon.png",
  "server": {
    "type": "binary",
    "entry_point": "server/tonk-agent-peer",
    "mcp_config": { "command": "\${__dirname}/server/tonk-agent-peer", "args": [], "env": {} }
  },
  "tools": [
    { "name": "tonk_join", "description": "Join a Tonk space from a share link" },
    { "name": "tonk_list_spaces", "description": "List spaces this peer has joined" },
    { "name": "tonk_query", "description": "Read a space with Tonk notation" },
    { "name": "tonk_evaluate", "description": "Write to a space with Tonk notation" },
    { "name": "tonk_display", "description": "Render entities with their own Tonk displays" },
    { "name": "tonk_show", "description": "Show a live view of a query" },
    { "name": "tonk_view_log", "description": "Diagnose live views" },
    { "name": "tonk_install_extension", "description": "Open Claude Desktop's install screen for this extension" },
    { "name": "tonk_guide", "description": "Read Tonk's authoring manuals" }
  ],
  "compatibility": { "platforms": ["darwin", "linux"] }
}
EOF
case "$out" in /*) ;; *) out="$PWD/$out" ;; esac
npx --yes @anthropic-ai/mcpb@2.1.2 pack "$work" "$out" >&2
