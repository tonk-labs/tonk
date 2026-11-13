# Architecture

This document describes how Tonk works and where each part is implemented.
For an introduction, see the [README](README.md).

Tonk provisions and syncs **spaces**. A space is a data store that people and
programs read and write through links with object capabilities attached. Each
client that opens a link keeps a local copy of the space and syncs its changes
with the other copies. A space contains its data, the schemas that describe
the data, views (web pages that show and edit the data), rules, and a signed
history of all changes.

## Components of a space

Each section gives a short description, then the technical basis and the
location of the code.

### Links

A space is shared with a link. The link contains a permission, for example
read access or read-write access. A client that opens the link gets a local
copy of the space. The owner can revoke a link.

Technical basis: the link carries a [UCAN](https://github.com/ucan-wg/spec)
delegation, which is a chain of signed capabilities. The access service
verifies the chain before it gives access to storage. Code:
[`tonk-invite`](rust/tonk-invite) (link formats),
[`tonk-access-service`](rust/tonk-access-service) (verification and storage
access).

### Local copies and sync

Each client (browser, desktop app, CLI or agent) works on a local copy of the
space. A client can make changes when it is offline. When it connects, it
sends its changes and receives the changes from other copies. The sync
service stores and forwards changes. It does not run application code.

Technical basis: each copy is a replica of a
[Dialog DB](https://github.com/dialog-db/dialog-db) repository. A replica
pulls from its upstream, merges the facts of both sides, and pushes new
revisions. See [How Tonk works](guide/src/introduction.md#local-copies-and-sync).

### Facts and schemas

A space stores data as facts. A fact states that an entity has an attribute
with a value. A schema selects a set of attributes to read together, for
example "a task has a title and a done flag". Schemas apply when data is
read, not when it is written. A schema can be added or changed at any time,
and two schemas can read the same facts. Stored data is not migrated.

Technical basis: a fact is an (entity, attribute, value) claim. A schema is a
*concept*: a named set of typed attributes. Code and reference:
[the model](guide/src/the-model.md), [`tonk-notation`](rust/tonk-notation),
and the standard library in
[`rust/tonk-core/assets/library`](rust/tonk-core/assets/library).

### Views

A view is a web page (HTML, CSS and JavaScript) that is stored in the space.
It shows data from the space and lets a user change it. The same view can
open in a browser, in the desktop app, or in an agent chat client.

Technical basis: a view is a template that is bound to a concept. A WASM
runtime in the client queries the space, renders the template, and updates
it when the facts change. Code: [`tonk-display`](rust/tonk-display),
[`tonk-template`](rust/tonk-template), [`tonk-worker`](rust/tonk-worker).

### Rules

A rule defines what follows from an action, in a way similar to a
spreadsheet formula. Example: when a user ticks a task, the rule marks the
task as done.

Technical basis: a page control posts a *command*, which is a transient fact.
A rule is a deduction. When the premises of the rule match the command and
the stored facts, the rule writes its conclusion as durable facts. Rules run
when a command is evaluated. They are not background jobs. Code:
[the model](guide/src/the-model.md), [`tonk-evaluator`](rust/tonk-evaluator).

### History

Each change is signed by the key that made it. History is append-only, so
earlier states remain available.

Technical basis: each write appends a signed revision to the replica's
content-addressed history. A retraction is a new entry; it does not remove
earlier entries. See
[How Tonk works](guide/src/introduction.md#history).

## Storage: Dialog DB

Tonk stores data in [Dialog DB](https://github.com/dialog-db/dialog-db), an
embeddable database that keeps a local replica on each device and syncs
replicas. Dialog stores claims, which are (entity, attribute, value)
triples, in an append-only, content-addressed repository. A change asserts a
new claim or retracts an existing one; stored claims are not changed in
place. Concepts group attributes into queryable shapes, and rules derive new
claims from existing ones. A new concept or rule does not change stored
claims.

## Crate map

Crates are grouped by the component they implement. Each crate's README
starts with a short description of the crate. All crates are in
[`rust/`](rust); the macOS app is in [`desktop/`](desktop).

| Part | Crates |
| --- | --- |
| **Data, schemas and rules** | [`tonk-notation`](rust/tonk-notation) (the notation you write), [`tonk-analyzer`](rust/tonk-analyzer) (checks a document), [`tonk-evaluator`](rust/tonk-evaluator) (runs it against a space), [`tonk-schema`](rust/tonk-schema) (typed facts Tonk itself stores), [`tonk-core`](rust/tonk-core) (shared primitives and the standard library), [`tonk-library`](rust/tonk-library), [`dialog-reactor`](rust/dialog-reactor) (live queries and transactions over [Dialog DB](https://github.com/dialog-db/dialog-db)), [`tonk-macros`](rust/tonk-macros) |
| **Views** | [`tonk-display`](rust/tonk-display), [`tonk-template`](rust/tonk-template), [`tonk-render`](rust/tonk-render) (server-side HTML), [`tonk-router`](rust/tonk-router), [`tonk-host`](rust/tonk-host), [`tonk-site`](rust/tonk-site), [`tonk-guest`](rust/tonk-guest), the editor builds behind library components ([`tonk-code`](rust/tonk-code) for `<code-editor>`, [`tonk-prose`](rust/tonk-prose) for `<prose-editor>`, and [`tonk-table`](rust/tonk-table) for `<table-editor>`), [`tonk-inspector`](rust/tonk-inspector) (the notebook); the command palette [`tonk-intent`](rust/tonk-intent) and [`tonk-lingo`](rust/tonk-lingo) |
| **Links, access and accounts** | [`tonk-invite`](rust/tonk-invite) (link formats), [`tonk-access-service`](rust/tonk-access-service) (checks permissions, gives storage access), [`tonk-identity`](rust/tonk-identity) (passkey-based identity), [`tonk-account`](rust/tonk-account), [`tonk-email`](rust/tonk-email) |
| **Clients** | [`tonk-worker`](rust/tonk-worker) (the engine behind every client: browser service worker and native hosts), [`tonk-worker-api`](rust/tonk-worker-api), [`tonk-ui`](rust/tonk-ui) (web app), [`tonk-cli`](rust/tonk-cli), [`desktop/`](desktop) (macOS app), [`tonk-language-server`](rust/tonk-language-server) |
| **Agents** | [`tonk-agent-peer`](rust/tonk-agent-peer) (local MCP server), [`tonk-host-worker`](rust/tonk-host-worker) and [`tonk-view-peer`](rust/tonk-view-peer) (hosted MCP), [`mcp/`](mcp) |
| **Support** | [`tonk-common`](rust/tonk-common), [`tonk-macros`](rust/tonk-macros), [`tonk-analytics`](rust/tonk-analytics), [`tonk-perf`](rust/tonk-perf), [`bench/`](bench) |

## Further reading

- [How Tonk works](guide/src/introduction.md), [the model](guide/src/the-model.md)
  and the [notation reference](guide/src/reference.md) in the guide.
- [`manual/`](manual): design notes for spaces, accounts and terms.
- `tonk help -g`: the CLI's built-in guides (notation, views, events, sync).
