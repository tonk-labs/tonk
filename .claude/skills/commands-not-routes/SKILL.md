---
name: commands-not-routes
description: How to add an operation to the tonk worker — define a COMMAND (transient concept + registered handler + outcomes as facts), not a new HTTP route in router.rs. Use whenever about to add `.route(` to rust/tonk-worker/src/router.rs, a `reqwest` call to tonk-ui/src/api.rs, or a request/response DTO to tonk-worker-api for something a page or the FAB triggers.
allowed-tools: Read, Bash, Glob, Grep
---

# Commands, not routes

The worker's HTTP surface is pinned: `rust/tonk-worker/src/router/route_table.rs` lists every route, and `it_adds_no_http_routes_without_editing_the_pinned_table` fails when `router.rs` gains one. That is deliberate. The route table is the data plane (branch `query`, `transact`, `evaluate`, `blob`, `sync`, and a few identity/site probes); everything a user *does* is a command.

## Why

- A route is one handler reachable by one caller over one transport. A command is a fact: the FAB, a YAML view, the CLI, a test, and another command can all assert it, and it rides the same `/transact` path everything else uses.
- A route answers with a response body one caller reads once. A command's outcome lands as facts on a branch the page already subscribes to, so every element showing that state updates, on every tab, with no polling and no "reshape the bar after the POST" code.
- Routes accumulate a parallel API in three places (`router.rs`, `tonk-worker-api`, `tonk-ui/src/api.rs`) that nothing keeps consistent. Commands live in one place: `tonk-schema`.
- Capability is a compile-time gate: a command only registers if `CommandEnv: Provider<C>`. A route has no equivalent.

Roughly half of today's 61 routes are commands wearing HTTP (`/api/account/*`, `/api/profile/{join,visit}`, `/api/repository/{repo}/{invite,membership}`, `/api/custody/*`, `/api/customer/*`). Do not add to that half; migrate from it when you touch it.

## What a command is

1. **A transient concept** in `rust/tonk-schema/src/command.rs`, with its attributes in `domain.rs` under `command::<name>`. It is asserted with `kind: transient`, so it never persists; the commit sweeps it after the handler fires. Implement `Command` on it.

   ```rust
   #[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
   pub struct PromoteMember {
       pub this: Entity,
       pub member: crate::domain::command::promote::Promote,
   }
   impl Command for PromoteMember { type Input = Self; type Output = (); }
   ```

2. **A handler** in the worker implementing `crate::reactor::CommandHandler<CommandEnv>`: `trigger_attributes` (from `Decode::trigger_attributes()`), `matches`, and `run`. `run` gets a `CommandEnv`: the `AppState` plus the origin repo/branch the transient was committed in, and the originating client when a page effect is needed. Register it in `command_registry()` in `rust/tonk-worker/src/router/command.rs`. Look at `router/members.rs` (`PromoteMemberHandler`, `ExpelMemberHandler`) for the smallest complete example.

3. **A trigger.** Two forms, both already wired:
   - Declarative, in a view: `command!: &member/promote` in `rust/tonk-core/assets/library/core.yaml`, bound to a form or `onclick=command`. Fields read `dom.event.*` attributes.
   - Programmatic, from a Rust element: build the same transient as a `TransactRequest` claim and dispatch through `window.tonk.transact(...)`. See `invite_claim_json` / `enable_sync_claim_json` in `rust/tonk-fab/src/logic.rs`.

4. **Outcomes as facts.** The handler asserts durable facts (a `MemberRole` stamp, a `Membership` row, an `Invitation`) or overlay facts (`state:*` entities for per-session, unreplicated state such as a refusal the share control echoes). The page reads them through the subscription it already has. If the page must be told something that no branch can carry (navigate, set title), post to `env.origin().client` as `tonk:join` does; do not invent a response body.

## When a route is actually right

- Raw data plane: bytes in, bytes out, no user intent (`blob`, `import`/`export`, `sync/*`).
- Something the service worker must answer *before* the page has a branch to subscribe to (`/api/identify`, `/api/site`, `/api/identity/root`).
- A debug/inspection surface (`/api/inspect/*`).

If you believe you have one of these, add it to `ROUTES` in `route_table.rs` in the same change and say in the PR why a command could not carry it. Reviewers will push back on "the UI needs a response".

## Checklist before writing `.route(`

- Is this triggered by a user or a page? Then it is a command.
- Does it need to answer the caller? Ask what fact the caller would subscribe to instead.
- Does it need the page to do something (navigate, open)? Post to the originating client from the handler.
- Is the same operation needed from the CLI? A command is asserted the same way from `tonk eval`; a route is not.
