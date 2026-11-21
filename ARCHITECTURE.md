# Architecture

## Clients and services

| Entry point | How it reaches a space |
| --- | --- |
| Web | [`tonk-ui`](rust/tonk-ui) is a Leptos app compiled to WebAssembly with Trunk. Its API requests go to the [service worker](rust/tonk-worker), which opens the browser’s local replica. |
| MCP | The [access service](rust/tonk-access-service) handles OAuth and forwards `/mcp` to [`tonk-host-worker`](rust/tonk-host-worker). A Durable Object handles writes for each space and MCP session. [`tonk-view-peer`](rust/tonk-view-peer) runs a replica in a Web Worker for views embedded in chat. |
| Desktop | The [macOS app](desktop) uses SwiftUI for its shell, WebKit for space views and a native worker for data access. |
| CLI | [`tonk-cli`](rust/tonk-cli) opens a local replica selected by name or directory binding. It evaluates notation against the space and can pull from or push to an upstream. |

A local coding agent can also run [`tonk-agent-peer`](rust/tonk-agent-peer)
over stdio. It redeems a share link once and keeps its own replica. The separate
[agent-link adapter](mcp/AGENT-LINKS.md) accepts a scoped link on each call.

## Components of a space

### Links

Share links carry [UCAN](https://github.com/ucan-wg/spec) delegations that
specify the recipient’s capabilities. [`tonk-invite`](rust/tonk-invite)
encodes the invitation. The access service verifies delegated authority before
granting access to remote storage.

### Local copies and sync

A space is a [Dialog DB](https://github.com/dialog-db/dialog-db) repository.
Replicas pull from an upstream, merge changes and push new revisions.
The access service provides the remote storage used for sync.

### Facts and schemas

A fact states that an entity has an attribute with a value. A *concept*
groups typed attributes into a shape you can assert or query. For example, a
task can have a title and a done flag. Different concepts can read the same
facts without migrating stored data.

### Views

A view binds an HTML template to a concept. [`tonk-display`](rust/tonk-display)
updates the page when query results change. [`tonk-render`](rust/tonk-render)
uses the same template planner to produce HTML without a browser.

### Rules

A control such as a checkbox can issue a command, represented as transient
facts. When those facts and the stored data satisfy a rule’s premises, the
[evaluator](rust/tonk-evaluator) can assert a durable change such as marking
the task done. The [data model](guide/src/the-model.md) follows this loop
from a button click to an updated view.

### History

Each write appends a signed revision to the repository’s content-addressed
history. Retracting a claim records a new entry and preserves earlier states.

## Crate map

Crates are grouped by the component they implement. All crates are in
[`rust/`](rust); the macOS app is in [`desktop/`](desktop).

| Part | Crates |
| --- | --- |
| **Data, schemas and rules** | [`tonk-notation`](rust/tonk-notation) (the notation you write), [`tonk-analyzer`](rust/tonk-analyzer) (checks a document), [`tonk-evaluator`](rust/tonk-evaluator) (runs it against a space), [`tonk-schema`](rust/tonk-schema) (typed facts Tonk itself stores), [`tonk-core`](rust/tonk-core) (shared primitives and the standard library), [`tonk-library`](rust/tonk-library), [`dialog-reactor`](rust/dialog-reactor) (live queries and transactions over [Dialog DB](https://github.com/dialog-db/dialog-db)), [`tonk-macros`](rust/tonk-macros) |
| **Views** | [`tonk-display`](rust/tonk-display), [`tonk-template`](rust/tonk-template), [`tonk-render`](rust/tonk-render) (server-side HTML), [`tonk-router`](rust/tonk-router), [`tonk-host`](rust/tonk-host), [`tonk-portal`](rust/tonk-portal), [`tonk-guest`](rust/tonk-guest), and the built-in elements [`tonk-table`](rust/tonk-table), [`tonk-prose`](rust/tonk-prose), [`tonk-code`](rust/tonk-code), [`tonk-inspector`](rust/tonk-inspector), [`tonk-fab`](rust/tonk-fab); the command palette [`tonk-intent`](rust/tonk-intent) and [`tonk-lingo`](rust/tonk-lingo) |
| **Links, access and accounts** | [`tonk-invite`](rust/tonk-invite) (link formats), [`tonk-access-service`](rust/tonk-access-service) (checks permissions, gives storage access), [`tonk-identity`](rust/tonk-identity) (passkey-based identity), [`tonk-account`](rust/tonk-account), [`tonk-email`](rust/tonk-email) |
| **Clients** | [`tonk-worker`](rust/tonk-worker) (shared worker for browser and native hosts), [`tonk-worker-api`](rust/tonk-worker-api), [`tonk-ui`](rust/tonk-ui) (web app), [`tonk-cli`](rust/tonk-cli), [`desktop/`](desktop) (macOS app), [`tonk-language-server`](rust/tonk-language-server) |
| **Agents** | [`tonk-agent-peer`](rust/tonk-agent-peer) (local MCP server), [`tonk-host-worker`](rust/tonk-host-worker) and [`tonk-view-peer`](rust/tonk-view-peer) (hosted MCP and embedded views), [`mcp/`](mcp) |
| **Support** | [`tonk-common`](rust/tonk-common), [`tonk-handler-macros`](rust/tonk-handler-macros), [`tonk-analytics`](rust/tonk-analytics), [`tonk-perf`](rust/tonk-perf), [`bench/`](bench) |

## Further reading

- [How Tonk works](guide/src/introduction.md), [the model](guide/src/the-model.md)
  and the [notation reference](guide/src/reference.md) in the guide.
- [`manual/`](manual): design notes for spaces, accounts and terms.
- `tonk help -g`: the CLI's built-in guides (notation, views, events, sync).
