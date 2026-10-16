#!/bin/sh
# Package a tonk-agent-peer binary as a Claude Desktop extension (.mcpb).
#
#   plugins/tonk/package-mcpb.sh BINARY PLATFORM VERSION OUT.mcpb
#
# PLATFORM is darwin, linux or win32. The extension runs the same peer as the
# plugin and shares its data directory, so both act as one peer. Desktop chat
# renders the peer's live views (MCP Apps); plugins cannot reach chat.
set -eu
binary="$1"
platform="$2"
version="$3"
out="$4"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
mkdir -p "$work/server"
install -m 0755 "$binary" "$work/server/tonk-agent-peer"
cat > "$work/manifest.json" <<EOF
{
  "manifest_version": "0.3",
  "name": "tonk-agent-peer",
  "display_name": "Tonk",
  "version": "$version",
  "description": "Work in shared Tonk spaces through a local peer, with live views in the conversation.",
  "long_description": "Paste a Tonk space's share link and Claude joins it under this computer's own peer identity. Claude can then read and write the space, and show live views of it that update as the space changes.",
  "author": { "name": "Tonk Labs", "url": "https://github.com/tonk-labs" },
  "homepage": "https://github.com/tonk-labs/tonk/tree/main/plugins/tonk",
  "repository": { "type": "git", "url": "https://github.com/tonk-labs/tonk" },
  "license": "MIT",
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
    { "name": "tonk_show", "description": "Show a live view of a space" },
    { "name": "tonk_guide", "description": "Read Tonk's authoring manuals" }
  ],
  "compatibility": { "platforms": ["$platform"] }
}
EOF
case "$out" in /*) ;; *) out="$PWD/$out" ;; esac
npx --yes @anthropic-ai/mcpb@2.1.2 pack "$work" "$out" >&2
