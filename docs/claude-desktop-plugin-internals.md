# How Claude Desktop runs the Tonk plugin

Notes from reading Claude Desktop 2.42304.2 (`@ant/desktop`, its `app.asar`)
and the Claude Code engine 2.1.304 it bundles, October 2026. None of this is
documented. It is minified code: function names below are the minifier's and
change between builds, so search for the quoted strings, not the names.

## Why live views render from the extension and not from the plugin

A thread in Claude Desktop that works with the computer reaches local MCP
servers through Desktop's device bridge (tools named
`mcp__remote-devices__<server>__<tool>`). The bridge announces each local tool
with its own `_meta`, and the chat renders a tool's `ui://` view only when that
`_meta` carries `anthropic/hasUiResource: true`.

The bridge collects servers from two managers
(`index.chunk-BgWXEy-f.js`):

- the shared manager: installed extensions (`.mcpb`) and
  `claude_desktop_config.json` servers, `isPlugin: false`;
- the plugin manager (`getPluginMcpInstance`): servers of enabled plugins,
  stdio and `.mcpb`, keyed `plugin:<plugin>:<server>`, `isPlugin: true`.

And announces them like this:

```js
_meta: { "anthropic/approvalHash": h, "anthropic/serverId": n,
  "anthropic/kind": a ? "plugin" : "local",
  "anthropic/toolName": m.name, "anthropic/readOnlyHint": ...,
  ...!a && r.n(m) && { "anthropic/hasUiResource": !0 } }   // a = isPlugin
```

So a plugin's server never gets the view flag, whatever it declares. Our
`tonk_display` and `tonk_show` still run and return their result as text. The
same server installed as an extension, or listed in
`claude_desktop_config.json`, renders. Putting the server in a `.mcpb` inside
the plugin does not change this: it is still run by the plugin manager.

Tracked upstream in
[anthropics/claude-ai-mcp#274](https://github.com/anthropics/claude-ai-mcp/issues/274)
(and its duplicate #830), which predate this finding.

## Other paths that can render plugin views, all behind Anthropic's flags

- **Local Code-tab sessions.** Desktop starts the bundled engine with
  `CLAUDE_CODE_MCP_APPS_HOST=1` only when feature gate `513873720` is on
  (or `CLAUDE_E2E_COPPER_HARBOR` is set). With it, the engine advertises
  `io.modelcontextprotocol/ui` to the servers it dials, reports each tool's
  `_meta.ui`, and reads `ui://` resources for Desktop
  (`announceCliMcpAppServers`, which accepts servers of source `plugin`).
  Without it the engine's `initialize` omits the extension.
- **Cowork VM sessions.** The same variable is set behind gate `3184956269`,
  off by default.
- **The plugin bridge itself** (running plugin servers on the host for remote
  threads) is gated by `3555657854`.

Nothing a plugin declares turns any of these on.

## What a plugin cannot do

- **Set its own icon.** The plugin objects Desktop gives the Customize page have
  no icon field. Desktop reads only `displayName`, `description`, `version`,
  `author` from `plugin.json`, and `version`, `displayName`, `description` from a
  marketplace entry. A `.mcpb` icon is resolved only for installed extensions;
  inside a plugin the bundle manifest is dropped before the page sees it. The
  page is part of the claude.ai web app, so the icon presumably comes from the
  directory listing. (`clis.<name>.icon` exists but only labels a flag-gated CLI
  card.)
- **Register a localhost connector.** claude.ai connectors are dialed from
  Anthropic's side (`mcp-proxy.anthropic.com`), so they cannot reach
  `127.0.0.1`. A plugin's `http` server is either dialed by the engine (no
  views, as above) or swapped for an existing claude.ai connector with a
  matching name or host; a plugin with a `.mcpb` server is never swapped.
  Desktop's own client dials only extensions, config-file servers and, on
  third-party deployments, admin-provisioned servers.
- **Have a server marked as an extension.** No manifest field makes the shared
  manager own a plugin's server.

## What a plugin can do that the docs do not mention

Fields Desktop or the bundled engine accept beyond skills and MCP servers:
`hooks` (including hooks modules, `register(on)`, which can add UI surfaces
beside the chat: the "mods" mechanism), `agents`, `userConfig` (exposed as
`${user_config.KEY}` and `CLAUDE_PLUGIN_OPTION_<KEY>`), `monitors`, `channels`,
`lspServers`, `outputStyles`, `themes`, `workflows`, `dependencies`,
`defaultEnabled`, `binaries{name:{sha256}}`, and `clis` blocks.

Limits of the plugin bridge for remote threads: a server is dropped if it
references `${user_config.*}` or has a required `.mcpb` `user_config` field
without a default; only `CLAUDE_PLUGIN_ROOT`, `PATH`, `HOME` and similar are
expanded (`CLAUDE_PLUGIN_DATA` is not); the command must resolve inside the
plugin directory.

## What we tested (macOS, Desktop 2.42304.2, October 2026)

All in threads that reach the computer through the Desktop link, each in a
chat started after the install:

| Delivered as | Server command | Views |
| --- | --- | --- |
| Extension, binary bundled | `${__dirname}/server/tonk-agent-peer` | render |
| Extension, launcher script | `${__dirname}/server/tonk-agent-peer` (executable, `#!/bin/sh`) | render |
| Extension, launcher script | `/bin/sh ${__dirname}/server/tonk-agent-peer` | did not render (tested only in chats started before the install, so unconfirmed) |
| Plugin, the same launcher bundled as `.mcpb` | `${__dirname}/server/tonk-agent-peer` | text only |

Two rules that cost us a day:

- **Views render only in chats started after the extension was installed or
  updated.** An older chat keeps the tool list it started with and never
  requests the view, even though the tools still answer. Test in a new chat.
- **Right after a plugin or extension is installed or updated, Desktop stops
  and restarts its server (about 30 seconds).** A chat started in that window
  sees none of its tools.

Views from Anthropic's built-in apps render in the same threads. Mod panes
don't: a mod in these threads has no surface (`$.session.surfaces()` is `[]`,
`$.ui.open` resolves `isPlaced: false, reason: "no-surface"`).

## Options for Tonk today

The directory no longer accepts extensions (`.mcpb`) and asks for local servers
to be shipped in a plugin, which is exactly where views can't render. And a
plugin that declares its server as a bundled `.mcpb` is held for a reviewer
("Bundled MCP server not inspected").

1. **What we ship:** the plugin (skill, auto-update, Code and Cowork) declares
   a plain stdio server, and releases publish the extension separately as
   `tonk-peer.mcpb`. The skill tells the agent to offer the extension when the
   user wants views in Desktop chat and only the plugin's copy of the tools is
   there. Two installs, works now.
2. Have the launcher add a `tonk-peer` entry to `claude_desktop_config.json`
   on first run, with consent. One install, views render, but Desktop needs a
   restart, chat sees the tools twice, and directory review will likely object.
3. A mod (hooks module UI surface) showing Tonk displays beside the chat.
   Untested whether mods load in remote threads.
4. Ask Anthropic to drop the `!isPlugin` condition (#274). The real fix.
