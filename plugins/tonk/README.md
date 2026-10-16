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

## Configuration

Set these in the environment the host starts MCP servers with:

| Variable | Default | Meaning |
| --- | --- | --- |
| `TONK_CHANNEL` | `stable` | `stable`, `staging`, or `dev` (latest `main` build) |
| `TONK_RELEASE` | | Pin an exact release tag |
| `TONK_AGENT_PEER_HOME` | `~/.tonk/agent-peer` | Peer data directory |
| `TONK_AGENT_PEER_BINARY` | | Run a local build instead (development) |

## Not yet

- Signing in to an account, so peers on several machines act for one account.
- Keeping the peer key in the OS keychain; it is a file in the private data
  directory for now.
- Sync while no session is open.
