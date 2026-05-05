# `TonkReactor`: query subscriptions over branches

`TonkReactor` is the worker's reactive layer over dialog
branches. It holds (a) the worker's `operator` + `profile`
handles — same things `TonkState` carries today — and (b) a set
of standing query subscriptions keyed by branch.

A subscription names "this query, on this branch." Whenever the
branch changes (a transaction commits, a sync pulls in artifacts)
the reactor re-runs each subscription's query and broadcasts
the new conclusions to every downstream subscriber — but only
when those conclusions differ from the previous broadcast.
Unchanged results are silent: dedup is a property of the
subscription, not the subscriber.

This document defines the public shape of `TonkReactor`,
`Subscription`, and the `POST /api/repository/{repo}/branch/
{branch}/query` endpoint that uses them.

## Cost model

The naive shape is: when a branch changes, re-run every
subscription's query and broadcast the result to every
subscriber, dedupping when the result hasn't changed. That's
what this spec describes. It's fine for the working-set sizes
we expect (10s of subscriptions per branch, queries that
complete in milliseconds) and keeps the implementation small.

Two pieces of cleverness are *deliberately* deferred:

- **Skip evaluation when no relevant attributes were touched.**
  In principle the query's `ConceptDescriptor` enumerates the
  attribute URIs it reads, and a commit's touched-attribute set
  could pre-filter which subscriptions even need to run. In
  practice "the attributes a query reads" is non-trivial when
  the query is a join, and a wrong filter silently misses
  updates. Skipping evaluation is a future optimization once
  re-evaluation cost actually shows up in profiles.
- **Per-fact diffs.** The subscription tells subscribers "here
  is the full result"; subscribers compute their own diff if
  they want one. Sending diffs is more bytes of protocol
  surface than the current consumers need.

What we *do* do:

- **Hash-based broadcast dedup.** Each subscription remembers
  the blake3 hash of the last broadcast. A re-run that
  produces the same hash skips the broadcast entirely. Holding
  the 32-byte hash instead of the full bytes keeps the working
  set small even for subscriptions whose result is large.
- **One subscription per `(branch, query)` pair.** A second
  subscriber for the same query attaches to the existing
  subscription rather than allocating a parallel one. When
  every subscriber disconnects, the subscription is dropped on
  the next change pass.

---

## Goals

- **Push-style query results.** A client subscribes once and
  receives the latest result whenever the branch changes,
  without polling.
- **Coalesce redundant broadcasts.** Two transactions that don't
  affect a query's matches don't generate two broadcasts.
- **Coalesce redundant subscriptions.** N clients subscribing to
  the same query share one re-evaluation; the reactor doesn't
  run the query N times per change.
- **Same endpoint serves one-shot queries.** Without
  `Accept: text/event-stream` the route runs the query once and
  returns conclusions inline, no subscription created.

## Non-goals

- **Differential update payloads.** The reactor sends the full
  conclusion set every broadcast, not a diff. Diffing belongs to
  whatever consumer wants it.
- **Per-subscriber filtering.** A subscription's broadcasts go
  to every subscriber. Two clients wanting different filters
  open two subscriptions.
- **Cross-branch joins.** A subscription scopes to one branch.
  A client wanting to react to changes on multiple branches
  opens one subscription per branch.
- **Persistence.** Subscriptions live in worker memory and
  vanish when the worker is replaced. The endpoint is meant to
  be idempotent — clients re-subscribe after a worker upgrade
  and the reactor rebuilds its state from scratch.

---

## Public types

### `TonkReactor`

```rust
pub struct TonkReactor {
    profile: Profile,
    repos: Mutex<HashMap<String, RepoEntry>>,
}
```

Owned by the worker (lives on `TonkState`). The reactor doesn't
own the operator — every effect takes `&Env` at `perform` time,
matching dialog's pattern. That keeps the reactor agnostic to
*which* operator runs the effect (test stubs, the worker's
`DefaultOperator`, etc.) and avoids holding a long-lived handle
that would need teardown coordination.

The `repos` map is populated lazily by `perform`: a chain that
references a repo + branch resolves each level against the
cache; on miss it opens the underlying handle (using `env`) and
inserts. One open per repo + one per branch is the floor; every
subsequent `perform` for the same chain skips both opens.

## API surface

The public API is a builder chain. Each method on the chain
returns a description; nothing touches the reactor's caches or
the network until `perform`.

```rust
reactor
    .repository("home")          // RepositoryHandle, builder
    .branch("meta")              // BranchHandle, builder
    .transaction()               // TransactionBuilder
    .assert(...)
    .commit()                    // Commit, an effect
    .perform(&operator).await?;
```

Three leaf effects exist: `commit` (mutating), `query` (one-
shot read), `subscribe` (subscription read). All take `&Env`
at perform time:

```rust
reactor.repository("home").branch("meta").query(q).perform(&op).await?
// → Vec<ConceptConclusion>

reactor.repository("home").branch("meta").subscribe(q).perform(&op).await?
// → broadcast::Receiver<Bytes>, with initial snapshot already enqueued

reactor.repository("home").branch("meta").transaction().assert(...).commit().perform(&op).await?
// → commit result; on success the perform path re-evaluates every
//   subscription on this branch against the same `&op`.
```

`perform` semantics for the chain:

1. Look up `RepoEntry` for the repository name. On miss, open
   the repository via `env` and insert.
2. Look up `BranchEntry` for the branch name in
   `repo.branches`. On miss, open the branch via `env` and
   insert.
3. Run the leaf effect (`commit` / `query` / `subscribe`)
   against the resolved branch.
4. For `commit`: on success, walk `branch.subscriptions` and
   re-evaluate each one against `env`. Failures here log and
   are swallowed — the commit already succeeded; subscription
   broadcasts are best-effort.

### `RepoEntry`

```rust
struct RepoEntry {
    repository: Repository,
    branches: HashMap<String, BranchEntry>,
}
```

Caches the open `Repository` handle so subsequent branches under
the same repo skip the repository load. `Repository` is the
result of `state.profile.repository(name).load().perform(op)`
and stays valid across requests.

### `BranchEntry`

```rust
struct BranchEntry {
    branch: Branch,
    subscriptions: HashMap<QueryHash, Subscription>,
}
```

Caches the open `Branch` handle and the subscriptions registered
against it. `Branch` internally holds a `Cell<Revision>` that
tracks the revision automatically as the branch advances, so a
cached handle stays current — re-evaluation doesn't need to
re-open between commits.

### Builder types

The chain is built from the following types, each holding the
state accumulated so far. Every method on a chain returns a new
chain value — nothing async, nothing touches the reactor's
caches until `perform`.

```rust
pub struct RepositoryHandle<'a> {
    reactor: &'a TonkReactor,
    repo: String,
}

impl RepositoryHandle<'_> {
    pub fn branch(self, name: impl Into<String>) -> BranchHandle<'_>;
}

pub struct BranchHandle<'a> {
    reactor: &'a TonkReactor,
    repo: String,
    branch: String,
}

impl BranchHandle<'_> {
    pub fn transaction(self) -> TransactionBuilder<'_>;
    pub fn query(self, q: ConceptQuery) -> Query<'_>;
    pub fn subscribe(self, q: ConceptQuery) -> Subscribe<'_>;
}

pub struct TransactionBuilder<'a> {
    /* repo + branch + asserts + retracts */
}

impl TransactionBuilder<'_> {
    pub fn assert<S: Statement>(self, s: S) -> Self;
    pub fn retract<S: Statement>(self, s: S) -> Self;
    pub fn commit(self) -> Commit<'_>;
}
```

The leaf effect types — `Commit`, `Query`, `Subscribe` — each
have a single `perform` method:

```rust
impl Commit<'_> {
    pub async fn perform<Env>(self, env: &Env) -> Result<CommitResult, ReactorError>
    where
        Env: /* dialog operator bound */;
}

impl Query<'_> {
    pub async fn perform<Env>(self, env: &Env) -> Result<Vec<ConceptConclusion>, ReactorError>
    where Env: /* … */;
}

impl Subscribe<'_> {
    pub async fn perform<Env>(self, env: &Env) -> Result<broadcast::Receiver<Bytes>, ReactorError>
    where Env: /* … */;
}
```

The bare `Branch::transaction()` / `Branch::pull` / etc. paths
are not exposed on routes. Mutation flows through the reactor's
chain so the perform path can re-evaluate subscriptions before
returning.

### `QueryHash`

```rust
struct QueryHash(blake3::Hash);
```

Content hash of the canonical-CBOR encoding of the
`ConceptQuery` (via `serde_ipld_dagcbor`). CBOR gives a
deterministic byte layout without writing a custom canonicalizer
— map keys sort, integers pack consistently. Same machinery
dialog uses for content addressing.

The repo + branch don't go into this hash because the
subscription is keyed *inside* its branch's `BranchEntry` —
two different branches with the same query naturally land in
different sub-maps. Within one branch, two clients sending
identical `ConceptQuery` values collide on `QueryHash` and
share one subscription.

`ConceptQuery` doesn't implement `Hash`, but it implements
`PartialEq`, so the worker verifies on a hash collision (a
distinct query producing the same hash — negligible with
blake3, but cheap to check) and rejects the second.

### `Subscription`

```rust
struct Subscription {
    query: ConceptQuery,
    last_hash: Option<blake3::Hash>,
    subscribers: Vec<Subscriber>,
}

struct Subscriber {
    sender: mpsc::UnboundedSender<Bytes>,
    status: Status,
}

enum Status {
    /// Just attached — hasn't received the current snapshot yet.
    Pending,
    /// Has received bytes whose hash matches the subscription's
    /// `last_hash`.
    Established,
}
```

One subscription per `(branch, query)` pair, regardless of how
many subscribers have attached. The branch handle isn't carried
here — the poll path already has a `&BranchEntry.branch` at the
point it iterates subscriptions.

- **`query`** — the `ConceptQuery` to re-run.
- **`last_hash`** — blake3 of the most recent serialization of
  the query result. `None` until the first poll completes.
- **`subscribers`** — open downstream channels with their
  delivery status. New subscribers join as `Pending`; the next
  poll either promotes them to `Established` (if it broadcast
  to them) or drops them (if their channel closed).

Broadcast bytes are framed as JSON (`Vec<ConceptConclusion>`
serialized via serde_json) so the SSE event payload is directly
parseable by clients.

---

### Polling subscriptions

A single `poll` routine handles both first-delivery to a new
subscriber and broadcast-on-change. It's invoked from
`Subscribe::perform` (after attaching the new subscriber) and
from the success path of `Commit::perform`, `Pull::perform`,
and `Sync::perform`.

For each subscription on the branch (commit/pull/sync polls
all subscriptions; subscribe polls only the affected one):

1. **Re-evaluate.** Run `subscription.query` against the
   branch using the `&env` the caller passed. Collect
   `Vec<ConceptConclusion>`.
2. **Serialize.** Encode as JSON bytes (broadcast format) and
   blake3-hash the bytes → `new_hash`.
3. **Decide who receives.**
   - If `Some(new_hash) != last_hash`: send `bytes` to *every*
     subscriber (Pending + Established). Set
     `last_hash = Some(new_hash)`. Mark all Pending →
     Established.
   - Else: send `bytes` only to Pending subscribers; mark them
     Established. Established subscribers skip — they already
     received bytes that matched this hash.
4. **Cleanup.** Drop any subscriber whose
   `sender.send(...)` returned `Err` (downstream receiver
   closed). If `subscribers.is_empty()` afterward, drop the
   subscription.

Errors during re-evaluation (branch closed, query rejected)
log and skip — a single broken subscription doesn't bring down
the whole branch's poll pass.

This routine isn't on the public surface: the reactor doesn't
expose a `notify_changed` method, because every legitimate way
to mutate a branch already goes through the chain and the poll
is bundled into the leaf effect's perform.

#### Why two-status instead of per-subscriber hashes

A simpler design tracks a `last_seen: Option<Hash>` per
subscriber and skips any subscriber whose `last_seen` already
matches `new_hash`. That works but adds 32 bytes per subscriber
and runs an `Option<Hash>` comparison per subscriber per poll.

Pending/Established collapses the same information into one
bit per subscriber: "have you received the current `last_hash`
yet?" — which is all the poll needs to decide who to send to.
Per-subscriber state stays at one byte regardless of how many
subscriptions accumulate.

### `TonkReactor::shutdown`

```rust
pub async fn shutdown(&self);
```

Drops every subscription's senders so all open SSE response
bodies finish. Subscription map clears. Called from the SW
`onupdatefound` path alongside `LspHub::shutdown`.

### `ReactorError`

```rust
pub enum ReactorError {
    BranchNotFound { repo: String, branch: String },
    QueryRejected(String),
    QueryHashCollision,
    Internal(String),
}
```

Surfaced through the route's error mapping into the existing
`TonkWorkerError` shape.

---

## Endpoint

```
POST /api/repository/{repo}/branch/{branch}/query
Content-Type: application/json
Body: ConceptQuery (serialized as { terms, predicate })
```

### Without `Accept: text/event-stream`

```
200 OK
Content-Type: application/json
Body: Vec<ConceptConclusion>
```

Builds and performs:
`reactor.repository(repo).branch(branch).query(q).perform(&op)`.
One-shot.

### With `Accept: text/event-stream`

```
200 OK
Content-Type: text/event-stream
Cache-Control: no-cache
Connection: keep-alive
Body: stream of `data: <Vec<ConceptConclusion> as JSON>\n\n`
```

Builds and performs:
`reactor.repository(repo).branch(branch).subscribe(q).perform(&op)`.
The leaf returns the per-subscriber `mpsc::UnboundedReceiver<Bytes>`;
the route wraps it as an SSE body, framing each item as
`data: <bytes>\n\n`. The stream terminates when the
subscription's sender is dropped (worker shutdown or
subscription GC).

The first event is the current snapshot — `subscribe` triggers
the poll routine before returning, which sends the current
result to the freshly-attached Pending subscriber.

### Error responses

- `400 Bad Request` — request body isn't a valid `ConceptQuery`.
- `404 Not Found` — repository or branch doesn't exist.
- `500 Internal Server Error` — query execution failed.

The error body is the existing JSON envelope
(`{ "error": { "kind", "message" } }`) the rest of the API uses.

---

## Migration

Every route that currently calls
`state.profile.repository(repo).load().perform(op).await?
.branch(b).open().perform(op).await?` and then mutates through
`Branch::transaction()` / `Branch::pull()` / etc. swaps to
`state.reactor.repository(repo).branch(b)` followed by the
chain methods. The reactor's cached handles eliminate the
load + open overhead per request as a side benefit.

A notification failure during commit/pull/sync logs and returns
to the caller; the request still succeeds. The reactor is a
background concern — clients shouldn't see request errors
because of a subscription glitch.

---

## Open questions

- **Per-branch lock granularity.** The reactor holds one
  `Mutex` over the whole repos map. Re-evaluation for branch A
  holds the lock while running A's queries, blocking reads for
  branch B. If subscription counts grow we'll need per-branch
  locks; for now the simpler shape is fine.
- **Hash collision detection.** Blake3 collisions are
  vanishingly unlikely, but the spec mentions verifying with
  `PartialEq` on subscribe to be safe. Cost is one
  `ConceptQuery == ConceptQuery` comparison per subscribe; cheap
  and prevents a class of footgun. Worth keeping.
