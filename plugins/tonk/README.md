# Tonk plugin

Work in shared Tonk spaces from Claude through a local peer.

Paste a space's share link into a conversation and Claude joins the space
under this computer's own peer identity. After that the link is no longer
needed: any session on this computer can read and write the space, show its
things in the conversation rendered by Tonk itself, and the peer keeps the
space in sync while a session is open.

## Install

```
claude plugin marketplace add tonk-labs/tonk
claude plugin install tonk@tonk
```

In Claude Desktop, add the `tonk-labs/tonk` marketplace under Customize →
Plugins and install Tonk from it. macOS (Apple Silicon) and Linux (x86_64) are
supported.

That's all there is to install. The first time you ask Claude to show
something from a space in Claude Desktop chat, it opens Desktop's install
screen for the small Tonk extension that live views need there; choose
Install, and start a new chat. See [Live views](#live-views) for why views
need it.

To try a pull request's build, use the one-line `claude --plugin-url …` from
the **Agent peer preview** comment on the pull request.

## What this plugin runs and connects to

The plugin itself is a launcher script and a skill. The optional Tonk extension
(`tonk-peer.mcpb`, published with each release) holds only a copy of the same
launcher. Everything they run, fetch or send:

- **It downloads the Tonk peer.** On every start the launcher
  (`scripts/tonk-agent-peer`) fetches `checksums.txt` from the Tonk release it
  follows on GitHub (`github.com/tonk-labs/tonk/releases`), and when that lists
  a newer archive for your platform, downloads it, verifies its SHA-256
  against `checksums.txt`, and installs it in `~/.tonk/bin/<channel>`. The
  archive holds the `tonk-agent-peer` binary, built from this repository by
  its CI, the guest runtime that renders views, and the Tonk extension for
  Claude Desktop (`tonk-peer.mcpb`). Offline, it runs the copy
  it already has. It then runs the binary as the plugin's stdio MCP server.
  Tagged releases are Developer ID signed and notarized; builds of `main`
  are not.
- **The peer keeps its identity and data locally,** in `~/.tonk/agent-peer`:
  a key it generates on first use (a file in that private directory, not the
  OS keychain yet), the spaces it joined, and their replicas.
- **It talks to the Tonk service of each space you join.** A share link names
  the service (for example `tonk.foundation`); the peer redeems the link
  there, then pulls and pushes that space's changes to the same service,
  every few seconds while a session is open. It contacts no other service,
  sends nothing to Anthropic or to the plugin's authors, and joins only links
  you give it in the conversation.
- **It opens Claude Desktop's extension installer when you ask to see
  something.** On macOS, when you ask Claude to display or show something and
  the Tonk extension isn't installed, `tonk_install_extension` opens that
  verified `tonk-peer.mcpb` with Claude Desktop, which shows its own install
  screen for you to confirm or cancel. It opens at most once per conversation,
  changes nothing else, and does nothing when the extension is installed.
- **It serves views on this computer only.** For live views in the
  conversation, the peer serves its own API on `127.0.0.1` at a random port.
  Every request must carry a random token that the peer creates when it
  starts and gives only to the views the host loads from it, so other
  programs and web pages on the computer cannot use it.

## How it works

- **One peer per computer.** Every session on the computer is the same peer.
- **One owner at a time.** Hosts start an MCP server per session. The first to
  need the peer owns it: it keeps the worker open and syncs. Other sessions
  forward their calls to it. When the owner's session ends, the next call
  anywhere takes over, and a newer build takes over from an older one.
- **Any Tonk service.** Like the browser, the peer joins a space on whichever
  service its link names.

## Live views

Two tools show a space in the conversation where the host renders MCP Apps:

- `tonk_display` renders things with their concept's own display, through
  Tonk's guest runtime: the same `<tonk-display>` the web app uses, live and
  interactive.
- `tonk_show` renders a query's result and keeps it current.

Claude Desktop renders these views only from servers it runs as extensions,
not from a plugin's server, so in Desktop chat they need the Tonk extension,
[`tonk-peer.mcpb`](https://github.com/tonk-labs/tonk/releases/download/tonk-agent-peer-dev/tonk-peer.mcpb).
It runs the same self-updating launcher, as the same peer: one identity and
one set of spaces whichever copy Claude calls. With only the plugin installed,
the tools work and return their results as text. Claude Code always shows
text. `tonk_view_log` explains a view that stays blank.

The directory no longer lists extensions, which is why the extension is a
separate download. Details in
[docs/claude-desktop-plugin-internals.md](https://github.com/tonk-labs/tonk/blob/main/docs/claude-desktop-plugin-internals.md).

## Configuration

Set these in the environment the host starts MCP servers with:

| Variable | Default | Meaning |
| --- | --- | --- |
| `TONK_CHANNEL` | `dev` | `dev` (latest `main` build), `staging`, or `stable` |
| `TONK_RELEASE` | | Pin an exact release tag |
| `TONK_AGENT_PEER_HOME` | `~/.tonk/agent-peer` | Peer data directory |
| `TONK_AGENT_PEER_RUNTIME` | `runtime/` beside the binary | Guest runtime served to views |
| `TONK_AGENT_PEER_BINARY` | | Run a local build instead (development) |

## Not yet

- Signing in to an account, so peers on several computers act for one
  account.
- Keeping the peer key in the OS keychain.
- Sync while no session is open.

## License

MIT
