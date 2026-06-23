# Reactor as capability router — `Reactor<S>` + `State<S>` + per-fetch effects

## Status

Design captured, **not yet implemented**. A provisional stepping-stone is
committed (`CommandOrigin` on `CommandEnv`, threaded through `dispatch`);
this plan supersedes that approach. Deferred — invite (the motivating
feature) can ship on the current router-dispatch seam first.

## Motivation

Command handlers currently can't discover which repo/branch their
triggering commit happened in. `dispatch(state, transients)` drops the
context the `/transact` route holds (`path.repo`/`path.branch`), so every
handler that operates on "the repo I fired in" re-carries it as a command
field stamped on a DOM `data-subject` (e.g. `tonk/rename-repository`).
That's the workaround `db/origin` was meant to remove — `db/origin` and
`db/branch` are declared in `core.yaml` but unwired.

The deeper observation: **the reactor is the natural home for command
dispatch — it's the capability router, the way the HTTP router is the
request router.** The reactor already owns the mechanism (`CommandRegistry`,
`CommandHandler`, `TypedCommand`, `match_transients`, decode); only the
dispatch *loop* and the concrete `Env` live up in the worker. Moving
dispatch into the reactor's commit cycle gives origin for free (from the
committing `BranchReference`) and makes every commit path fire commands
uniformly (today only `/transact` does; `slide` never does).

## Design

### `Reactor<S>` — generic over command state, like axum `Router<S>`

```rust
pub struct Reactor<S = ()> {
    profile: Profile,
    repos: RwLock<HashMap<String, Arc<RepositoryState>>>,
    profile_repo: RwLock<Option<Arc<RepositoryState>>>,
    commands: CommandRegistry<S>,   // moved in from TonkState
    state: S,                       // the .with_state value
}

impl Reactor<()> { pub fn new(profile) -> Reactor<()> }
impl<S> Reactor<S> {
    pub fn with_state<S2>(self, s: S2) -> Reactor<S2>;   // axum-style
    pub fn command<C>(self) -> Self where S: Provider<C>; // register
}
```

`S` is the command capability bundle (what handlers need to *act*):
`{ profile, operator }`. It must be `Clone + 'static` (handlers get a
clone for a detached `'static` future). It must **not** hold the reactor
back (cycle); re-queries go through a re-acquired `BranchReference`.

### `State<S>` — the handler-facing surface, like axum `State<S>`

Handlers implement `Provider<C> for reactor::State<S>`, not a bare env.
`State<S>` is built *per dispatched command* (so it carries that command's
origin) and is `'static` (it goes into the detached future):

```rust
pub struct State<S> { state: S, context: Context }

pub struct Context {
    pub origin: Origin,        // { repo: String, branch: String } — owned, re-acquirable
    pub transients: Changes,   // the triggering transient batch (owned)
}

impl<S> State<S> {
    pub fn state(&self) -> &S;
    pub fn profile(&self) -> &Profile;       // reactor-native
    pub fn origin(&self) -> &Origin;
    pub fn context(&self) -> &Context;        // { origin, transients }
    pub fn branch(&self) -> BranchReference<'_>;  // re-acquired live, scoped to origin
    pub fn spawn<F>(&self, future: F);        // follow-on work; joins the commit's Effect
}
```

**Why `State<S>` not `Reactor<S>` as receiver:** origin differs per
invocation, so the receiver must be a per-dispatch value; and handlers
should see capabilities + context, not the whole reactor (over-broad +
cycle).

**Why origin is owned, not a borrowed `BranchReference<'a>`:** the handler
future is `'static` and runs *after* the commit consumes the transaction's
`BranchReference`. So `context()` carries repo/branch by value;
`branch()` re-acquires a fresh live ref (cheap — warm cache) when the
handler actually queries.

### Effects: commit returns `{ revision, effect }`; route owns `waitUntil`

Constraint: the HTTP route must respond **when the durable commit lands**,
not after effects; but effects (and anything they spawn) must still **run
to completion** even though the response already returned (service-worker
`waitUntil` semantics). Today `/transact` `.await`s dispatch before
responding — a latent bug (slow/failing effect blocks the response).

**No separate `dispatch()` step.** The reactor's commit absorbs dispatch:
it commits durably, drives the matched handlers, and returns the effects as
a future the caller awaits. There is no worker-level `dispatch(...)`.

```rust
// reactor
pub struct Committed {
    pub revision: Revision,
    pub effect: Effect,   // 'static future(s) — the dispatched handlers' work
}

let Committed { revision, effect } = branch
    .commit()                 // spawns matched handlers' work into `effect`
    .perform(&operator)
    .await?;                  // resolves when the DURABLE write lands
```

The route uses each half for its purpose:

```
on_fetch(event):
  handle_via_router inserts event (clone) into request extensions   // like ClientId
  -> transact route:
       let { revision, effect } = branch.commit().perform(op).await  // commit done
       event = req.extensions().get::<FetchEvent>()
       event.wait_until(effect)                                       // route owns waitUntil
       return response(revision)                                      // returns at commit
```

- The reactor commit cycle drives matched handlers and collects their work
  into `effect` (internally a `TaskQueue` / `join_all` of the `'static`
  handler futures). Handlers spawn follow-on work via `State::spawn`, which
  joins into the same `effect`; nested spawns drain transitively.
- The route gets `revision` for the response *now*; `effect` goes to
  `waitUntil`, decoupled from the response.

**`FetchEvent` reaches the route via axum request extensions** — inserted
in `handle_via_router` beside the existing
`request.extensions_mut().insert(ClientId(...))`, extracted in the route.
It is the *route that committed* that calls `wait_until`, so the wiring
stays where the commit is, not threaded down to handlers. (`!Send` on
native is fine: this path is wasm-only worker code.)

**web-sys gap:** `FetchEvent::wait_until` is not in the generated binding
(it's inherited from `ExtendableEvent`). Add a small manual extern, as the
worker already does for `resultingClientId` (`event_resulting_client_id`).

`dialog_common::r#async::TaskQueue` (`spawn` aggregates fire-and-forget
work; `join` drains it transitively) is the natural internal carrier for
`Effect`.

## Ownership split

| Lives in `Reactor<S>` (capability router) | Lives in `TonkState` (SW/HTTP session) |
|---|---|
| repo/branch handle cache, profile | `view_bindings` (Client-ID → repo/branch routing) |
| `CommandRegistry<S>` (moved in) | `bridges` (per-client MessagePorts) |
| `S` = `{ profile, operator }` | the `Reactor<S>` |
| commit cycle: spawn effects, origin from `BranchReference` | — |

`profile_name` is **dropped**: it only provided a degraded-case display
label and is the worker's own `PROFILE_NAME` constant (the loaded `Profile`
holds only the credential — verified: name is a transient storage address,
not retained at the dialog/credential layer). Use `PROFILE_NAME` or
`profile.did()` at the two call sites.

`operator` is the **dialog/storage env** (`CommitProvider`/`SelectProvider`),
distinct from `S` (the **command/effect env**, `Provider<C>`). It stays
owned by the worker and passed into reactor effects per-call, as today.

## Consequence for invite

With this in place, `tonk/invite` carries **no `subject` field**: the
handler reads `state.origin().repo`, loads that repository, delegates to
`cmd.audience`, and asserts the `invitation` fact — all from the commit's
own context. (Drop the provisional `subject` field on the `Invite` schema
type.) `db/origin` is no longer required for invite, though wiring it as a
queryable fact remains the right general follow-up for views.

## Scope / blast radius

- `dialog-reactor`: `Reactor<S>`, `State<S>`, commit absorbs dispatch and
  returns `{ revision, effect }`, `CommandRegistry`/`Provider` receiver
  becomes `State<S>`. Shared with `slide` — slide passes `()` / no commands
  → no behavior change (it just ignores `effect`).
- `dialog-common`: reuse `TaskQueue` (no change) as the internal `Effect`
  carrier.
- `tonk-worker`: delete the standalone `dispatch()`; transact route
  consumes `{ revision, effect }` and calls `event.wait_until(effect)` (the
  `FetchEvent` arrives via request extensions); `CommandEnv` becomes `S`;
  drop `profile_name`; the manual `wait_until` extern.
- Invite handler rides on top.

This is foundational and spans three crates; do it on its own branch,
after invite ships on the simpler seam (or alongside, if we choose to fold
invite into it).
