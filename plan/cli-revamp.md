# CLI revamp: a dialog client with tonk as one peer

Status: sketch, not decided. Nothing here is implemented.

## What is being asked for

1. No account. A profile on the device is enough to create, query, write and share.
2. Expose dialog itself (repositories, branches, query, transact, sync), not
   tonk's account and space model.
3. The tonk service is a remote you replicate through, like any other peer.
4. Peer-to-peer sync the way sapling does it (iroh, UCAN chains, invites).
5. A daemon (sapling's process model), so caches stay warm and concurrent
   invocations do not race.
6. Serve a replica locally, with no tonk service in between.
7. The same command palette that spaces have.
8. An MCP mode for agents.

## What exists today (facts, with where to look)

- The CLI already opens dialog directly and syncs with `branch.push()` /
  `branch.pull()` (`rust/tonk-cli/src/sync.rs:225,263`). Sync is dialog
  replication, not tonk HTTP routes.
- The account coupling is in the default site config, not in the commands:
  `require_account` is on (`site.rs:1300-1309`), `AccountBoundOperator` gates
  every network fork (`account_authority.rs`), and `bootstrap_repository`
  always resolves an owner and provisions with the access service
  (`site.rs:640-760`, `customer::provision_in`).
- `remote add` accepts only a UCAN access-service address (`remote.rs:122-160`).
  dialog's `SiteAddress` already has `S3 | Ucan | Fs | Iroh` variants.
- Palette commands are data: `command!:` concepts with `action:` (the words
  that say it) and fields with `role:` (`core.yaml:2677-2700`). 16 declare
  `action:`, 13 of them in `profile.yaml`. Their handlers are
  `Provider<C> for CommandEnv` in **tonk-worker**
  (`router/command.rs:661-742`). The CLI does not depend on tonk-worker and
  runs none of them.
- MCP exists twice: `mcp/` (Node, goes through tonk-worker) and
  `rust/tonk-agent-peer` (stdio rmcp over `tonk_worker::LocalPeer`, with its
  own data dir `~/.tonk/agent-peer` and identity, separate from the CLI's).
  `tonk-agent-peer/src/owner.rs` already has an owner-process pattern: the
  first process to take the directory lock syncs every 5s and serves the
  others over `peer.sock`.
- sapling's daemon (PDD001/PDD007): one binary, HTTP/1.1 over a Unix socket in
  the runtime dir, auto-start by detached re-exec, `flock` instance lock plus
  data lock, leases plus exit-when-idle (10s), per-repo `Turns` mutex for the
  daemon's own writes. Plus sandboxed Wasm workers per (domain, persona, space).
- sapling's p2p (PDD006/PDD011) lives mostly **in sapling**, not dialog-db:
  responder policy (read-only, invoker bound to the QUIC connection), routes
  store, invites, member branches, watch over QUIC, the stranger caps. sapling
  pins a dialog-db fork branch (rev `28a910b7`), not dialog main.
- dialog-db has the server halves as libraries with no non-test caller:
  `dialog_remote_ucan::server::Access` (HTTP-framework-agnostic) and
  `dialog_peer_iroh::{serve::Responder, transport::accept}`.
- `Branch::replicate(Download).pull().push().every(ticks)` is a long-running
  sync stream (`dialog-repository/src/repository/branch/replicate.rs:80-100`).
  A natural thing for a daemon to hold.
- `dialog-stack` (`Stack::invoke`, `stack.provide`, `report.effect()`) is the
  command mechanism; the tonk CLI does not use it yet.

## Where the request's assumptions do not hold

These need an answer before the command surface is final.

1. **"The daemon avoids races."** The race that actually loses data is
   dialog-db#705: one key writing from two replicas on the same head mints
   the same version and one write disappears (`SyncError::RivalWriter`). A
   daemon serializes writers *on one machine, for one data dir*. It does not
   help when the CLI and tonk-agent-peer use separate data dirs (they do
   today), or across devices. What fixes #705 is one writer key per writer:
   a daemon that hands each client (CLI, MCP session) its own derived session
   key, plus folding agent-peer's data dir into the daemon's. Name which race
   you mean.
2. **"Warm caches."** Hypothesis, not a fact. Nobody has measured what a cold
   `tonk query` spends on opening the site, deriving keys, resolving remotes.
   If it is 50ms, a daemon is a lot of machinery for it. The stronger reasons
   for a daemon are things a one-shot process *cannot* do: hold a `replicate`
   stream, keep an iroh endpoint reachable so others can pull from you, keep
   QUIC connections open, run providers whose effects outlive a command.
3. **"Borrow sapling's daemon."** Two different things are braided there: the
   process model (socket, auto-start, locks, leases) and the worker/guest
   kernel sandbox. Only the first fits. And the p2p code you want is sapling
   code on a dialog fork. Sharing it means moving it into dialog-db
   (`dialog-peer-iroh`) first, or tonk forks it too. That is a prerequisite,
   not a detail.
4. **"Tonk service as just a peer, no account."** The access service stores
   bytes for whoever is provisioned (`customer::provision_in`). With no
   account, who is the customer? Options: the service accepts any `did:key`
   with a quota; provisioning comes from an invite (someone else's storage);
   or using the tonk service is the one thing that needs an account. Pick one.
   "No account" and "free hosted storage for any key" are not the same claim.
5. **"The same palette as spaces."** Most palette commands are
   tonk-product commands (`account/add`, `space/publish`, `agent/connect`,
   `space/invite-email`), and their handlers live in tonk-worker. Exposing them
   means depending on tonk-worker, which is the opposite of "less tonk
   opinionated". What generalizes is the *mechanism*: list the `command!:`
   concepts a repository declares, parse text against `action:`/`role:`, and
   invoke through the stack. Which providers ship is a separate question, and
   the answer is probably "dialog's generic ones built in, tonk's as an opt-in
   provider set".
6. **"Is it still `tonk`?"** If the CLI is a dialog client, the binary name,
   the on-disk location (`data_dir()/tonk/`, profile `dialog/tonk`) and the
   `tonk.<cmd>.vN` JSON envelopes are product choices. Decide before the
   surface ships, not after.

## Command surface (sketch)

Conventions kept from `plan/cli-structure.md` and `plan/cli-consistency.md`:
nouns list when bare; `add`/`rm`; `--json` on every command, wrapped in a
versioned envelope (`{"schema": "tonk.<cmd>.v1", ...}`); write verbs take
`--dry-run --no-sync -q`; `eval` stays the one write primitive and other write
verbs are sugar over it.

### Global flags

| flag | meaning |
|---|---|
| `--profile <name>` | which device profile (default `default`, env `TONK_PROFILE`) |
| `--repo <name\|did>` | which repository (default: bound to cwd, else `repo use`) |
| `--branch <name>` | default `main` |
| `--json` | machine output |
| `--no-daemon` | open storage in-process (refused if a daemon holds the data lock) |
| `--expect-daemon` | fail instead of auto-starting |

Output for errors, everywhere: `{"error": {"code": "...", "message": "...", "retry": bool}}`
with a stable `code` (`not-found`, `unauthorized`, `rival-writer`, `timeout`,
`conflict`, `unhandled`).

### daemon

| command | parameters | output |
|---|---|---|
| `daemon start` | `[--foreground] [--idle-timer <dur>] [--serve iroh\|http=<addr>]... [--relay <url>]` | foreground: logs; else `{pid, socket, build}` |
| `daemon stop` | | `{stopped: bool, pid}` |
| `daemon status` | | `{pid, build, uptime, profiles, repos_open, replicating: [{repo, branch, remote, state, last}], serving: [{transport, address}], clients}` |

### profile (replaces account / identity)

| command | parameters | output |
|---|---|---|
| `profile` | | rows `{name, did, default}` |
| `profile add <name>` | | `{name, did}` |
| `profile show [<name>]` | | `{name, did, iroh_node, repos: n, contacts: n}` |
| `profile use <name>` | | `{name}` |
| `profile rm <name>` | `-y` | `{name, removed}` |

No passkey, no access service. An account becomes optional later: delegating a
profile to a recovery key is a `share`, not a login.

### repo (replaces space)

| command | parameters | output |
|---|---|---|
| `repo` | | rows `{name, did, branch, upstream, bound_dir}` |
| `repo add <name>` | `[--library core\|none\|<file>]` | `{name, did}` |
| `repo show [<repo>]` | | `{name, did, branches, remotes, layers, members}` |
| `repo use <repo>` / `repo bind [<dir>]` | | `{name}` |
| `repo rm <repo>` | `[--keep-data] -y` | `{name}` |

`--library core` seeds tonk's `core.yaml` (today's behaviour); `none` gives a
bare dialog repository. That flag is where "less opinionated" is decided.

### branch

| command | parameters | output |
|---|---|---|
| `branch` | | rows `{name, revision, upstream}` |
| `branch add <name>` | `[--from <branch\|rev>]` | `{name, revision}` |
| `branch log` | `[--limit n]` | rows `{revision, parents, author, time, changes}` |
| `branch reset <rev>` | `-y` | `{from, to}` |
| `branch merge <from>` | `[--dry-run]` | `{revision, merged}` |

### data

| command | parameters | output |
|---|---|---|
| `query <doc\|-\|CONCEPT>` | `[--where F=V]... [--at <rev>] [--layer <name>] [--limit n]` | rows of the concept/variables; `{revision, rows, complete}` |
| `subscribe <doc\|->` | same as query | NDJSON stream of `{revision, added, removed}` (needs daemon) |
| `eval <doc\|-\|-c ..>` | `WriteArgs` | `{revision, ticks, asserted, retracted, commands: [{id, concept, outcome}]}` |
| `assert CONCEPT [ENTITY] --<field>..` | `WriteArgs [--notation]` | same receipt as eval; `--notation` prints the doc instead |
| `retract CONCEPT ENTITY [--field f]` | `WriteArgs [--notation]` | same receipt |
| `import <file>` | `[--format csv\|notation\|car]` | `{revision, claims}` |
| `export` | `[--format notation\|car] [--out f] [--at rev]` | file or stdout |
| `blob add <file>` / `blob cat <uri>` / `blob info <uri>` | `[--type mime]` | `{uri, size, type}` / bytes / metadata |

The schema verbs (`concept`, `view`, `element`, `render`) stay, but as tonk
library verbs: available when the repository has the tonk library, listed under
`tonk help library`, not in the core index.

### command (the palette)

| command | parameters | output |
|---|---|---|
| `command` | `[--all]` | rows `{concept, actions, fields: [{name, role, type}], provider: built-in\|tonk\|none}`; without `--all`, only those with a provider |
| `command suggest "<text>"` | `[--limit n]` | rows `{concept, reading, filled: {field: value}, missing: [field]}`, ranked; the same parse the palette does (tonk-lingo + tonk-intent) |
| `command run CONCEPT --<field> ..` | `[--args <json>] [--wait <dur>] WriteArgs` | `{invocation, revision, outcome: answered\|failed\|unhandled, answer, progress}` |
| `command run "<text>"` | `[--pick n] -y` | runs the top suggestion when every field is filled; otherwise prints the suggestions and exits non-zero |

`command run` is `stack.invoke(..)` then `report.effect().perform(..)`. An
`unhandled` outcome is an answer, not a crash: it means this process has no
provider for the concept (for example a tonk-worker command in a CLI built
without the tonk provider set).

### remote and sync

Remote addresses are URIs over dialog's `SiteAddress` variants:
`https://<host>` (UCAN, direct), `ucan+permit://...`, `s3://<bucket>?endpoint=..&region=..`,
`file:///path`, `did:key:z...?route=ip:1.2.3.4:5678` (iroh).

| command | parameters | output |
|---|---|---|
| `remote` | | rows `{name, address, subject, kind}` |
| `remote add <name> <address>` | `[--subject <did>] [--credential <name>]` | `{name, address, subject}` |
| `remote rm <name>` | | `{name}` |
| `upstream set <remote>[/<branch>]` | `[--pull-only\|--push-only]` | `{branch, upstream}` |
| `fetch` / `pull` / `push` | `[<remote>] [--download minimal\|queried\|everything]` | `{remote, before, after, pulled, pushed}` |
| `sync` | `[--watch] [--every <dur>]` | one-shot receipt, or with `--watch` the daemon keeps `replicate()` running and this prints `Replicated` events as NDJSON |
| `status` | | `{repo, branch, revision, upstreams: [{remote, ahead, behind, reachable}], daemon}` |

The tonk service becomes `remote add tonk https://tonk.network` with no special
code path. Whatever provisioning it needs (assumption 4) is a capability the
remote asks for and the CLI reports as `unauthorized`, with how to get it.

### share and peers (p2p)

| command | parameters | output |
|---|---|---|
| `share <did\|contact>` | `[--read\|--write] [--expires <dur>] [--branch]` | `{invite: "<one-line ticket>", subject, audience, expires}`; the ticket carries the UCAN chain and routes, no secret (sapling's format) |
| `join <ticket\|url>` | `[--name <repo name>]` | `{repo, did, remote, access}` |
| `member` | | rows `{did, name, access, since, expires}` |
| `member rm <did>` | | `{did, revoked}` |
| `contact` / `contact add <name> <did>` / `contact rm` | `[--route ..]` | rows `{name, did, routes}` |
| `peer` | | rows `{did, name, connected, last_seen, repos}` (daemon state) |

`join` of a tonk invite URL stays supported by recognizing the URL form; it
lands as a remote pointing at the tonk service.

### serve

| command | parameters | output |
|---|---|---|
| `serve` | `[--iroh] [--http <addr>] [--repo <r>].. [--read-only] [--relay <url>]` | `{transports: [{kind, address}], node: did, repos: [did]}`, then logs; or with a daemon running, registers the listeners with it and returns |

`--iroh` uses `dialog_peer_iroh::serve::Responder` with sapling's policy
(read-only by default, invoker bound to the connection). `--http` wraps
`dialog_remote_ucan::server::Access`, so anything that can talk to the tonk
service can talk to a laptop. Local serving is this command; there is no
separate "local mode".

### mcp

| command | parameters | output |
|---|---|---|
| `mcp` | `[--stdio] [--http <addr>] [--repo <r>].. [--access read\|read-write] [--agent <name>]` | MCP server on stdio (default) or HTTP |

Tools, matching `mcp/tools.json` where they already exist so hosts see the same
names:

| tool | input | output |
|---|---|---|
| `tonk_list_spaces` | `{}` | `[{subject, name, access, home}]` |
| `tonk_query` | `{space, notation}` | rows, as `query --json` |
| `tonk_evaluate` | `{space, notation}` | receipt, as `eval --json` |
| `tonk_commands` (new) | `{space, text?}` | as `command` / `command suggest` |
| `tonk_run_command` (new) | `{space, concept, args}` | as `command run --json` |
| `tonk_guide` | `{topic?}` | text |

Each MCP session gets its own derived writer key (`peer.session(context)`), so
an agent and a person writing the same repository are two writers, not one key
in two places (#705). `mcp` is a daemon client, which retires tonk-agent-peer's
separate data dir and owner process.

### tonk-specific (kept, demoted)

`tonk link` (move a repo under a tonk account via the browser), `tonk invite`
(short links, email invites), `render`, `view`, `element`, `concept`, `update`,
`telemetry`. Under `tonk help tonk`, not the core index. `migrate` converts
`spaces.json` + `spaces/<name>/` into profiles and repos.

## Daemon shape

Borrow from sapling (PDD001): HTTP/1.1 on a Unix socket in the runtime dir,
auto-start by detached re-exec, instance `flock` + data `flock`, leases and
exit-when-idle, `X-<Build>` header with a warning on mismatch, per-repo
`Turns` mutex for writes. Do not borrow the Wasm worker/guest-kernel split:
providers run in the daemon.

The daemon holds: open repositories and stacks, the credential store, one iroh
endpoint per profile that serves or syncs, `replicate()` streams for branches
with `sync --watch`, subscriptions, registered providers.

The CLI is a thin client: every command above is a request on the socket, and
`--no-daemon` runs the same handler in-process. One handler, two transports, so
there is no daemon-only behaviour except what needs a long-lived process
(`subscribe`, `sync --watch`, `serve`, `peer`).

## Order of work

0. **Prerequisites outside tonk.** Move sapling's responder policy, invite
   format and routes into dialog-db (`dialog-peer-iroh`), and get sapling off
   its fork. Decide assumption 4 with whoever runs the access service.
1. Daemon skeleton + `profile`, `repo`, `branch`, `query`, `eval`, with
   `--no-daemon` parity. Account off by default; `AccountBoundOperator` only on
   tonk remotes.
2. `remote` with all address kinds, `pull/push/fetch/sync`, `status`.
3. `serve` (iroh, then http), `share`/`join`/`member`/`contact`/`peer`.
4. `command` on the stack: generic providers built in, tonk's provider set
   extracted from tonk-worker behind a feature.
5. `mcp`, then retire `tonk-agent-peer`'s separate store.
6. `migrate`, demote tonk-specific verbs, delete `account_*` modules no longer
   reached.

## Open questions

- Which race, concretely, motivates the daemon? (assumption 1)
- Is cold-start cost measured? A number decides whether `--no-daemon` is the
  default for one-shot commands. (assumption 2)
- Who provisions storage on the tonk service for an account-less key? (4)
- Which palette providers must work outside the browser, and can they leave
  tonk-worker? (5)
- Binary name and data dir. (6)
- Does a repo without the tonk library still count as a "space" the web app can
  open? If not, `repo add --library none` makes repositories the web app
  cannot show.
- Is `share` to a `did:key` enough, or do people need a name to address
  (sapling has no discovery either; kanban b066a7)?
