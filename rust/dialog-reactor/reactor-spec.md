# `TonkReactor`: query subscriptions over branches

`TonkReactor` is the worker's reactive layer over dialog
branches. It holds (a) the worker's `profile` handle and (b) a
two-tier cache of open repository and branch handles, with the
subscriptions registered against each branch.

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
  the next change pass (its dead channel is detected the next
  time the poll attempts to send).

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
    repos: Mutex<HashMap<String, Arc<RepositoryState>>>,
}
```

Owned by the worker (lives on `TonkState`). The reactor doesn't
own an operator — every effect takes `&Env` at `perform` time,
matching dialog's pattern. That keeps the reactor agnostic to
*which* operator runs the effect (test stubs, the worker's
`DefaultOperator`, etc.) and avoids holding a long-lived handle
that would need teardown coordination.

The `repos` map is populated lazily. A chain that references a
repo + branch resolves each level against the cache; on miss it
opens the underlying handle (using `env`) and inserts. One open
per repo + one per branch is the floor; every subsequent
`acquire` for the same chain skips both opens.

The cache is two-tier `Arc`-shared: `Arc<RepositoryState>` and
`Arc<BranchState>`. Once a chain has acquired a session it can
operate on the branch (subscribe, poll) without re-locking the
reactor's outer map.

`parking_lot::Mutex` covers both maps' critical sections — they
are short and synchronous.

## API surface

The public API is a builder chain. Each method on the chain
returns a description; nothing touches the reactor's caches or
the network until `acquire` (for the inner handle) or `perform`
(for a leaf effect).

```rust
reactor
    .repository("home")          // RepositoryReference
    .branch("meta")              // BranchReference
    .transaction()               // TransactionBuilder
    .assert(...)
    .commit()                    // Commit, an effect
    .perform(&operator).await?;
```

Leaf effects: `commit` (transaction), `subscribe` (subscription
read), `pull`, `push`. The one-shot read uses dialog's native
`Branch::select(...)` directly, after acquiring the session,
because it returns a stream rather than a materialized `Vec`.
Two-phase keeps the underlying lazy stream visible to the caller
(who can `try_next` for streaming or `try_vec` to collect):

```rust
let session = reactor.repository("home").branch("meta")
    .acquire(&op).await?;

let conclusions = session.handle()
    .select(q).perform(&op).try_vec().await?;
// Vec<ConceptConclusion>

let subscriber = reactor.repository("home").branch("meta")
    .subscribe(q).perform(&op).await?;
// Subscriber { hash, receiver: UnboundedReceiver<Bytes> }
// — first event is the current snapshot, enqueued before
// `perform` returns

reactor.repository("home").branch("meta")
    .transaction().assert(...).commit().perform(&op).await?
// — on success the perform path re-evaluates every
//   subscription on this branch against the same `&op`.

reactor.repository("home").branch("meta")
    .pull().perform(&op).await?
// — on success the perform path re-polls every subscription.
```

Mutation chain semantics:

1. Walk the chain to a leaf effect.
2. Leaf `perform` calls `branch_reference.acquire(env)` —
   reusing a cached `BranchSession` if present, otherwise
   opening (and caching) the repository and branch.
3. Run the underlying dialog operation (`commit` / `pull` /
   `push`).
4. On success — for `commit` and `pull` — call
   `branch_session.state.poll(env)`, walking the subscription
   map and re-running each query. Failures here log and are
   swallowed; the mutation already succeeded.

`push` doesn't re-poll because it doesn't change local branch
state.

### `RepositoryState`

```rust
pub struct RepositoryState {
    repository: Arc<Repository>,
    branches: Mutex<HashMap<String, Arc<BranchState>>>,
}
```

Caches the open `Repository` handle so subsequent branches
under the same repo skip the repository load. `Arc<Repository>`
because the repo handle is cloned into per-branch chains.

### `BranchState`

```rust
pub struct BranchState {
    pub branch: Branch,
    subscriptions: Mutex<HashMap<QueryHash, Subscription>>,
}
```

Caches the open `Branch` handle and the subscriptions
registered against it. `Branch` internally holds a
`Cell<Revision>` that tracks the revision automatically as the
branch advances, so a cached handle stays current —
re-evaluation doesn't need to re-open between commits.

### `BranchSession`

```rust
pub struct BranchSession {
    pub state: Arc<BranchState>,
}

impl BranchSession {
    pub fn handle(&self) -> &Branch;
    pub fn subscription(&self, hash: QueryHash) -> SubscriptionReference<'_>;
}
```

Returned by `BranchReference::acquire(&env)`. Holds the
`Arc<BranchState>` so callers operate on the branch (subscribe,
select, poll) without re-locking the reactor's outer map.

### Builder types

```rust
pub struct RepositoryReference<'a> {
    pub reactor: &'a TonkReactor,
    pub name: &'a str,
}

impl RepositoryReference<'_> {
    pub fn branch(self, name: &str) -> BranchReference<'_>;
    pub async fn acquire<Env>(&self, env: &Env) -> Result<Arc<RepositoryState>, ReactorError>
        where Env: LoadProvider;
}

pub struct BranchReference<'a> {
    pub repository: RepositoryReference<'a>,
    pub name: &'a str,
}

impl BranchReference<'_> {
    pub async fn acquire<Env>(&self, env: &Env) -> Result<BranchSession, ReactorError>
        where Env: LoadProvider + BranchOpenProvider;

    pub fn subscribe(self, q: ConceptQuery) -> Subscribe<'_>;
    pub fn transaction(self) -> TransactionBuilder<'_>;
    pub fn pull(self) -> Pull<'_>;
    pub fn push(self) -> Push<'_>;
}

pub struct TransactionBuilder<'a> { /* repo + branch + asserts + retracts */ }

impl TransactionBuilder<'_> {
    pub fn assert(self, c: Claim) -> Self;
    pub fn retract(self, c: Claim) -> Self;
    pub fn commit(self) -> Commit<'_>;
}
```

There is no `Query` (one-shot) effect type. Use the dialog
`Branch::select(...)` chain on a `BranchSession::handle()` for
that.

### Per-operation env traits

The chain bounds `Env` per leaf, not via a single umbrella
trait, so each operation states the provider capabilities it
actually needs:

- `LoadProvider` — `RepositoryReference::acquire`
- `BranchOpenProvider` — `BranchReference::acquire`
- `SelectProvider` — re-poll path (subscription evaluation)
- `CommitProvider` — `Commit::perform`
- `PullProvider` — `Pull::perform`
- `PushProvider` — `Push::perform`

### `QueryHash`

```rust
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct QueryHash(Blake3Hash);

impl From<&ConceptQuery> for QueryHash { /* … */ }
```

Blake3 of the canonical-JSON encoding of a `Query` (the
serializable wire shape of `ConceptQuery`). JSON is
deterministic enough within one Rust process: `Parameters` and
`NamedAttributes` both serialize keys in `BTreeMap` order via
their custom serializers.

The repo + branch don't go into this hash because the
subscription is keyed *inside* its branch's `BranchState` —
two different branches with the same query naturally land in
different sub-maps. Within one branch, two clients sending
identical `ConceptQuery` values collide on `QueryHash` and
share one subscription.

`ConceptQuery` doesn't implement `Hash`, but it implements
`PartialEq`, so the worker verifies on subscribe (a hash
collision being a distinct query producing the same hash —
negligible with blake3, but cheap to check) and rejects the
second.

### `Subscription` and friends

```rust
pub(crate) struct Subscription {
    pub query: ConceptQuery,
    pub last_hash: Option<Blake3Hash>,
    pub subscribers: Vec<SubscriberSession>,
}

pub(crate) struct SubscriberSession {
    pub sender: UnboundedSender<Bytes>,
    pub status: Status,
}

pub(crate) enum Status {
    Pending,      // attached, hasn't received the current snapshot
    Established,  // received bytes whose hash matches `last_hash`
}
```

One subscription per `(branch, query)` pair, regardless of how
many subscribers have attached. The branch handle isn't carried
here — the poll path already has access via the parent
`BranchState`.

The public-facing handle a subscriber holds is:

```rust
pub struct Subscriber {
    pub hash: QueryHash,
    pub receiver: UnboundedReceiver<Bytes>,
}
```

`SubscriberSession` (in the subscription map) and `Subscriber`
(returned to the caller) deliberately have distinct names: the
former is internal channel state, the latter is the caller's
read handle.

Broadcast bytes are framed as JSON `Vec<Conclusion>` (the wire
projection of `ConceptConclusion`) so SSE event payloads parse
directly client-side.

### Wire shapes: `Query` and `Conclusion`

```rust
#[derive(Serialize, Deserialize)]
pub struct Query {
    pub terms: Parameters,
    pub predicate: ConceptDescriptor,
}

#[derive(Serialize)]
pub struct Conclusion {
    pub this: String,                                    // entity URI
    pub fields: BTreeMap<String, serde_json::Value>,     // term → value
}
```

`Query` is the serializable projection of `ConceptQuery` (used
as the `/query` request body and the canonical input to the
subscription hash). `Conclusion` is the serializable projection
of `ConceptConclusion` (used as the `/query` response and the
broadcast frame).

`Conclusion::project(c, terms)` walks the query's `terms` map,
looks each name up against the conclusion's underlying `Match`,
and serializes the resulting `dialog_artifacts::Value` via its
existing `serde::Serialize` impl. So `fields` carries one entry
per term named in the originating query: `this`, plus any other
variables the query bound.

---

### Polling subscriptions

A single `poll` routine handles both first-delivery to a new
subscriber and broadcast-on-change. It's invoked from
`Subscribe::perform` (after attaching the new subscriber) and
from the success path of `Commit::perform` and `Pull::perform`.

For each subscription on the branch (commit/pull poll all
subscriptions; subscribe polls only the affected one):

1. **Snapshot the query** out of the lock so re-evaluation
   doesn't hold the subscription mutex across an await.
2. **Re-evaluate.** Run the query against the branch using the
   `&env` the caller passed. Collect `Vec<ConceptConclusion>`.
3. **Project + serialize.** Render every conclusion through
   `Conclusion::project(c, &terms)`, encode the `Vec<Conclusion>`
   as JSON bytes, blake3-hash the bytes → `new_hash`.
4. **Decide who receives.**
   - If `Some(new_hash) != last_hash`: send `bytes` to *every*
     subscriber (Pending + Established). Set
     `last_hash = Some(new_hash)`. Mark all Pending →
     Established.
   - Else: send `bytes` only to Pending subscribers; mark them
     Established. Established subscribers skip — they already
     received bytes that matched this hash.
5. **Cleanup.** Drop any subscriber whose `sender.send(...)`
   returned `Err` (downstream receiver closed). If
   `subscribers.is_empty()` afterward, drop the subscription.

Errors during re-evaluation (branch closed, query rejected)
log and skip — a single broken subscription doesn't bring down
the whole branch's poll pass.

This routine isn't on the public surface as a notify hook: the
reactor doesn't expose a `notify_changed` method, because every
legitimate way to mutate a branch already goes through the
chain and the poll is bundled into the leaf effect's perform.
A `BranchState::poll(&env)` method *is* public, used by the
chain effects internally and by tests asserting on cache state.

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

#### Dead-subscriber pruning

Pruning is piggy-backed on the send attempt — there is no
separate reaper. A dropped SSE body closes the receiver; the
next change-driven poll's `send(...)` to that subscriber
returns `Err`, `retain_mut` drops it, and (if it was the last
subscriber) the subscription itself is removed.

This means an idle subscription whose sole subscriber dropped
will linger until the *next* commit or pull on the branch. In
practice, polls only fire on real change, and the first such
change reclaims the slot. We don't probe `is_closed()` on every
poll because that adds work to every iteration for a problem
that resolves itself the next time anything actually happens.

### `TonkReactor::shutdown`

```rust
pub fn shutdown(&self);
```

Drops every cached handle (and therefore every subscription's
sender) so all open SSE response bodies finish. Subscription
maps clear via the `Arc` reference count dropping to zero.
Called from the SW `onupdatefound` path alongside
`LspHub::shutdown`.

### `ReactorError`

```rust
pub enum ReactorError {
    RepositoryNotFound { repo: String, reason: String },
    BranchNotFound { repo: String, branch: String, reason: String },
    QueryFailed(#[from] EvaluationError),
    Commit(#[from] CommitError),
    Pull(#[from] PullError),
    Push(#[from] PushError),
    QueryHashCollision,
}
```

Surfaced through the route's error mapping into the existing
`TonkWorkerError` shape.

---

## Endpoint

```
POST /api/repository/{repo}/branch/{branch}/query
Content-Type: application/json
Body: Query (serialized as { terms, predicate })
```

### Without `Accept: text/event-stream`

```
200 OK
Content-Type: application/json
Body: Vec<Conclusion>
```

Acquires the `BranchSession` via the reactor chain, then calls
dialog's `branch.select(q).perform(&op).try_vec()` and projects
each `ConceptConclusion` through `Conclusion::project`.

### With `Accept: text/event-stream`

```
200 OK
Content-Type: text/event-stream
Cache-Control: no-cache
Connection: keep-alive
Body: stream of `data: <Vec<Conclusion> as JSON>\n\n`
```

Builds and performs:
`reactor.repository(repo).branch(branch).subscribe(q).perform(&op)`.
The leaf returns the `Subscriber { hash, receiver }`; the route
wraps `receiver` as an SSE body, framing each item as
`data: <bytes>\n\n`. The stream terminates when the
subscription's sender is dropped (worker shutdown or
subscription GC).

The first event is the current snapshot — `subscribe` triggers
the poll routine before returning, which sends the current
result to the freshly-attached Pending subscriber.

### Error responses

- `400 Bad Request` — request body isn't a valid `Query`.
- `404 Not Found` — repository or branch doesn't exist.
- `500 Internal Server Error` — query execution failed.

The error body is the existing JSON envelope
(`{ "error": { "kind", "message" } }`) the rest of the API uses.

---

## Migration

Routes that mutate (transaction/commit, pull, push, sync) flow
through the reactor's chain. The reactor's cached handles
eliminate per-request load + open overhead as a side benefit,
and — load-bearing — every successful mutation re-polls
subscriptions before returning so SSE clients see fresh frames.

Routes that only *read* (one-shot select, inspect endpoints)
acquire a `BranchSession` for the cache benefit, then call
dialog's native APIs on the underlying handle.

A re-poll failure during commit/pull logs and is swallowed; the
mutation request still succeeds. The reactor is a background
concern — clients shouldn't see request errors because of a
subscription glitch.

---

## Open questions

- **Per-branch lock granularity.** The reactor holds one
  `Mutex` over the whole repos map. Re-evaluation for branch A
  holds the lock briefly to enumerate; the actual query runs
  outside the lock. If subscription counts grow we may want
  finer-grained locks; for now the simpler shape is fine.
- **Hash collision detection.** Blake3 collisions are
  vanishingly unlikely, but the spec mentions verifying with
  `PartialEq` on subscribe to be safe. Cost is one
  `ConceptQuery == ConceptQuery` comparison per subscribe; cheap
  and prevents a class of footgun. Worth keeping.
- **Cross-tab broadcast.** Phase 3 work: a `BroadcastChannel`
  between SW instances so a commit in one tab fans out to
  subscribers in others. Not in this spec.
