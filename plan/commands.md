# Commands — typed-Rust effect handlers for tonk

Status: implemented. Describes how a transient concept (a *command*)
fires a typed-Rust handler in the service worker after a commit, and
how to wire a new command end to end.

Commands are the imperative sibling of the declarative `rule!:` effects
in [effects.md](./effects.md). A rule reacts to a transient by deriving
more facts (body query → head assertion); a command reacts by running
arbitrary `async` Rust — repository creation, key generation, network
IO — which a rule body can't express. Both share the same trigger: a
transient concept asserted in a commit, swept before the durable write.

## The shape

A command is an ordinary `#[derive(Concept)]` that is *asserted
transiently*. The transient channel (see effects.md) makes the trigger
edge-triggered: the fact exists only for the commit that asserted it, so
the handler fires exactly once and the command leaves no durable trace.

A handler is an `async fn` whose parameters declare what it needs, axum
style:

```rust
// pure — no capability
async fn record(cmd: SomeCommand, tx: Transaction) -> Transaction { … }

// needs IO — declares the capability
async fn create_space(
    cmd: CreateSpace,
    State(state): State<AppState>,   // declared capability
    tx: Transaction,
) -> Transaction { … }
```

- The first parameter is the decoded command (owned).
- Any `State<…>` parameters are *declared capabilities* — the
  dispatcher supplies them; a pure handler declares none.
- `Transaction` is the outcome buffer (the Bevy `Commands` analog): the
  handler `assert`/`retract`s into it and returns it. The dispatcher
  commits it to the branch the command arrived on.

The handler's only fact-write path is the returned `Transaction`, and it
always lands on the command's own branch — so a handler can't "commit
whatever wherever". (A privileged handler that declares `State<AppState>`
can still do multi-branch IO through the reactor, e.g. `create_space`
creating a repo; that is an acknowledged exception, gated today only by
holding the capability. See "Future direction".)

## How a command flows

1. **Trigger.** Something asserts a transient command. Two paths:
   - **From a DOM event** (the common case): a `<form onsubmit=cmd>` or
     `<button onclick=cmd>` in a view template. The notation event layer
     ([event-handling.md](./event-handling.md)) builds a `TransactRequest`
     with one transient claim and POSTs it to `/transact`. The command's
     fields read from the event (e.g. `the:
     dom.event.current-target.elements.name/value` for a form input).
   - **From code**: build a transient `TransactRequest` and POST it, or
     assert through the reactor with the transient bucket.

2. **Commit + capture.** The `/transact` path
   (`tonk-worker/src/router/transact.rs`) applies the claims. Transient
   claims land in the transaction's transient bucket. Before committing,
   the path captures `builder.transients.clone()` — the commit sweeps
   transients from durable storage, so this snapshot is the only
   post-commit view of which commands arrived.

3. **Dispatch.** After the commit and once the state lock is released,
   `router::command::dispatch` runs. It matches the captured transients
   against the registry, runs each matched handler, and commits each
   outcome to the command's branch — **concurrently and independently**
   (one handler's IO or failure doesn't block another).

4. **Outcome.** The handler's `Transaction` commits durably, so UIs
   react over their subscriptions like any other state change. Errors
   are facts too: a handler asserts a `Failed`/`status` outcome rather
   than returning an error.

## The decode bridge (dynamic → static)

The dispatcher matches *untyped* committed facts; a `Provider`/handler
wants a *typed* command. The bridge is `decode_concept`
(`tonk-worker/src/reactor/command.rs`): it reuses the derived
`Query::<C>::default()` + `realize` — the same decode read-subscriptions
use — so a plain `#[derive(Concept)]` is decodable with no extra code. A
missing or mistyped required field makes decode fail, which is the
natural "this concept doesn't match these facts" signal.

Matching is reverse-indexed by the command's attribute names (mirroring
the `dialog.effect/on` index the rule fixpoint walks), so only candidate
handlers are decoded. **All matching handlers fire** — commands are
subscription-like, no tiebreak.

## Wiring a new command — checklist

1. **Define the concept** (`tonk-schema/src/command.rs`), a plain
   `#[derive(Concept)]` with `this` + the fields the handler needs. If
   it's form-driven, the field `the:` is a `dom.event.*` read-path so
   the form populates it and the handler decodes the same attribute
   (see [event-handling.md](./event-handling.md) and the kebab-case note
   below).

2. **Write the handler** (`tonk-worker/src/router/…`), an
   `async fn(C, [State<…>,] Transaction) -> Transaction`. Use the
   reactor's `State` (`crate::reactor::State`), NOT axum's — both are in
   scope in router modules, and only the reactor one is a command
   capability.

3. **Register it** in `router::command_registry()`:
   `CommandRegistry::new().command(create_space)`. The builder infers
   the command type and capabilities from the fn signature.

4. **Define the trigger.** For a form/click, add the `command!` to the
   library (the concept must be present on the branch so the view's
   mount finds its descriptor) and the `<form onsubmit=…>` /
   `<button onclick=…>` to the view template. The command's fields and
   the library `command!` must agree on every `the:`.

### Gotchas

- **Attribute `the:` is kebab-case.** The analyzer rejects uppercase in a
  relation domain. Write `dom.event.current-target.elements.name/value`;
  the event layer converts each segment to camelCase
  (`event.currentTarget.elements.name.value`) at read time. The stored
  attribute is the literal kebab string, and the handler decodes by that
  same string — form-assert and handler-decode agree.
- **Reading a form field**: use `dom.event.current-target.elements.<name>/value`
  (the form's named control), NOT `FormData` (which returns empty for a
  Web Awesome `wa-input`).
- **Validate library edits** with the native
  `it_lowers_the_profile_library` / `it_lowers_the_standard_library`
  tests in `tonk-worker/tests/standard_library.rs` — a `the:` typo or an
  unresolved anchor fails there rather than silently in the browser.

## Transactional caveat (TODO)

Handlers have no read-set isolation. A handler reads durable state via
`State<…>`, decides an outcome, and that outcome commits later without
checking that what it read still holds; concurrent handlers can both
read and write the same state. The goal (marked `TODO(stm)` on
`dispatch`/`commit_outcome`/`State`) is STM-like optimistic concurrency:
track the observed revision/read-set and commit-or-conflict, re-running a
handler whose reads went stale rather than committing on them.

## Future direction — capability-based commands

The handler layer is bespoke today. The intended end state is to express
commands as `dialog_capability::Command` (or `Effect`) and replace
handlers with `impl Provider<CreateSpace> for <env>`: the operator's (or
a tonk command env's) capability set decides whether a command may run,
which is UCAN-shaped — a command attempted without the capability simply
isn't provided. The router would then register command *types* and look
up the provider, rather than registering handler functions. The decode
bridge and the transient-trigger dispatch above stay; only the "what
runs" layer changes.
