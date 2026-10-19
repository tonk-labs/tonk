# Tonk plugin

Work in shared Tonk spaces from Claude Code and Cowork through a local peer.

Paste a space's share link into a conversation and the agent joins the space
under this machine's own peer identity. After that the link is no longer
needed: any session on this machine can read and write the space, and the
peer keeps it in sync in the background while a session is open.

## Install

```
claude plugin marketplace add tonk-labs/tonk
claude plugin install tonk@tonk
```

The first session downloads the signed `tonk-agent-peer` binary for your platform
from the latest Tonk release and verifies it against the release's
`checksums.txt`. macOS (Apple Silicon) and Linux (x86_64) are supported.

To try a pull request's build, use the one-line `claude --plugin-url …` from the
**Agent peer preview** comment on the pull request.

## How it works

- **One peer per machine.** The peer's identity and data live in
  `~/.tonk/agent-peer`. Every session on the machine is the same peer.
- **One owner at a time.** Hosts start an MCP server per session. The first to
  need the peer owns it: it keeps the worker open and syncs every 30 seconds.
  Other sessions forward their calls to it. When the owner's session ends, the
  next call anywhere takes over.
- **Any Tonk service.** Like the browser, the peer joins a space on whichever
  service its link names. It joins only links you give it in the conversation.

## Live views (experimental)

`tonk_show` renders the result of a query as a live view inside the
conversation, using MCP Apps. The view keeps itself current by long-polling the
peer (`tonk_watch`, which only the view can call), so changes made anywhere,
including other people's, appear within seconds.

Views render where the host supports MCP Apps from a local server: Claude
Desktop chat, through the `tonk.mcpb` extension from a release, and possibly
Cowork. Claude Code shows the result as text. The extension carries the same
launcher as the plugin, pinned to its release: it installs once and updates
itself on every start, and a newer build takes over from an older peer still
running in another session. The extension and the plugin run the same peer
and share its data directory.

`tonk_display` renders entities in the conversation with their concept's own
display, through Tonk's real guest runtime: the same `<tonk-display>` the web
app uses, live and interactive. The peer that owns the data directory serves
its worker API on a loopback port, and the view relays the guest's requests
there, as the browser relays them to its service worker. The guest runtime
(`space-runtime.json`) ships in the extension under `server/runtime`.
`tonk_view_log` shows what recent views requested and reported, for when one
stays blank.

The loopback API is not authenticated yet: while a peer runs, any local
process or page that finds its port can act as that peer.

## Configuration

Set these in the environment the host starts MCP servers with:

| Variable | Default | Meaning |
| --- | --- | --- |
| `TONK_CHANNEL` | `dev` | `dev` (latest `main` build), `staging`, or `stable` |
| `TONK_RELEASE` | | Pin an exact release tag |
| `TONK_AGENT_PEER_HOME` | `~/.tonk/agent-peer` | Peer data directory |
| `TONK_AGENT_PEER_RUNTIME` | `runtime/` beside the binary | Guest runtime served to space views |
| `TONK_AGENT_PEER_BINARY` | | Run a local build instead (development) |

## Not yet

- Signing in to an account, so peers on several machines act for one account.
- Keeping the peer key in the OS keychain; it is a file in the private data
  directory for now.
- Sync while no session is open.
