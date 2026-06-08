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
transiently* and also implements
[`dialog_capability::Command`](https://github.com/dialog-db/dialog-db)
(`Input = Self`, `Output = ()`). The transient channel (see effects.md)
makes the trigger edge-triggered: the fact exists only for the commit
that asserted it, so the command fires exactly once and leaves no durable
trace.

What *runs* a command is a `Provider<C>` — there is no handler function.
The provider is a tonk-owned env, `CommandEnv`, a cheap handle over
`AppState`:

```rust
// the command concept (tonk-schema)
#[derive(Concept)]
struct CreateSpace { this: Entity, name: SpaceName }
impl dialog_capability::Command for CreateSpace {
    type Input = Self;
    type Output = ();
}

// the provider (tonk-worker) — capability + behaviour in one impl
#[async_trait(?Send)]
impl Provider<CreateSpace> for CommandEnv {
    async fn execute(&self, cmd: CreateSpace) {
        // re-lock through self.state() to reach the operator/reactor,
        // do the IO, commit outcomes. Self-contained; returns ().
    }
}
```

A command is *self-contained*: `execute` does its own IO and commits its
own outcomes through the env. There is no outcome buffer and no
dispatcher-side commit. Capability is **structural** — a command can run
only if `CommandEnv: Provider<C>` is implemented, and registering a
command requires that bound, so an unsupported command won't even
register. (The runtime UCAN-style gate — the operator actually *holding*
the capability — layers on top of this later; see "Future direction".)

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
   against the registry and calls each matched command's
   `Provider::execute` on a clone of the `CommandEnv` —
   **concurrently and independently** (one command's IO or failure
   doesn't block another).

4. **Outcome.** The provider commits its own outcomes through the env, so
   UIs react over their subscriptions like any other state change. Errors
   are facts too: a provider asserts a `Failed`/`status` outcome rather
   than surfacing an error.

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

2. **Impl `Command`** for the concept (`Input = Self`, `Output = ()`),
   and **impl `Provider<C>` for `CommandEnv`** (`tonk-worker`). The
   `execute` body re-locks through `self.state()` to reach the
   operator/reactor, does the IO, and commits its outcomes. Use
   `#[cfg_attr(not(wasm32), async_trait)] #[cfg_attr(wasm32,
   async_trait(?Send))]` (or plain `?Send` for a wasm-only provider).

3. **Register the type** in `router::command_registry()`:
   `CommandRegistry::new().command::<CreateSpace>()`. This compiles only
   if `CommandEnv: Provider<CreateSpace>` — the capability gate.

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

Commands have no read-set isolation. A provider reads durable state
through the env, decides, and commits — without checking that what it
read still holds; concurrent commands in one batch can both read and
write the same state. The goal (marked `TODO(stm)` on
`reactor::command::TypedCommand::run` and `dispatch`) is STM-like
optimistic concurrency: track the observed revision/read-set and
commit-or-conflict, re-running a command whose reads went stale rather
than committing on them.

## Future direction — runtime capability gating

Commands are already `dialog_capability::Command`s run by a
`Provider<C>`, so the *compile-time* capability gate is in place: a
command can't be registered unless the env provides it. The next step is
the *runtime* UCAN gate — `execute` succeeding only if the operator
actually **holds** the capability for each action it attempts (create a
repo, commit to a branch), rather than the env unconditionally doing the
work. Moving providers from `CommandEnv` toward the operator's own
capability chain (where orphan rules allow) is the path there. The decode
bridge and the transient-trigger dispatch stay; only the authorization
inside `execute` changes.
