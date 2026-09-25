---
name: commands-not-routes
description: How to add an operation to the tonk worker — define a COMMAND (transient concept + registered handler + outcomes as facts), not a new HTTP route in router.rs. Use whenever about to add `.route(` to rust/tonk-worker/src/router.rs, a `reqwest` call to tonk-ui/src/api.rs, or a request/response DTO to tonk-worker-api for something a page or the FAB triggers.
allowed-tools: Read, Bash, Glob, Grep
---

# Commands, not routes

The worker's HTTP surface is pinned: `rust/tonk-worker/src/router/route_table.rs` lists every route, and `it_adds_no_http_routes_without_editing_the_pinned_table` fails when `router.rs` gains one. That is deliberate. The route table is a branch's `query`, `transact` and `evaluate` (on the profile and on a space), plus the two `blob` routes that carry raw bytes; everything else a page asks of the worker is a command it transacts, answered by facts it queries.

## Why

- A route is one handler reachable by one caller over one transport. A command is a fact: the FAB, a YAML view, the CLI, a test, and another command can all assert it, and it rides the same `/transact` path everything else uses.
- A route answers with a response body one caller reads once. A command's outcome lands as facts on a branch the page already subscribes to, so every element showing that state updates, on every tab, with no polling and no "reshape the bar after the POST" code.
- Routes accumulate a parallel API in three places (`router.rs`, `tonk-worker-api`, `tonk-ui/src/api.rs`) that nothing keeps consistent. Commands live in one place: `tonk-schema`.
- Capability is a compile-time gate: a command only registers if `CommandEnv: Provider<C>`. A route has no equivalent.

The account, identity, profile, customer, custody, invite, join, sync, site, export/import, inspect and onboarding routes that used to sit here were all migrated to commands and queries. Reads became queries over facts the worker already holds or publishes on an overlay `state:*` row; actions became commands. Do not reintroduce them.

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

2. **A provider** in the worker: `impl dialog_capability::Provider<C> for CommandEnv` (dual `async_trait` attrs — `Send` off wasm, `?Send` on it). `execute` gets the decoded command and reads everything else off `self`: the `AppState`, the origin repo/branch the transient was committed in (`self.origin()` — and `may_target_space` when the command names a target space), and the originating client when a page effect is needed. Register the TYPE in `profile_commands()` (and `space_commands()` if a space branch may run it on itself) in `rust/tonk-worker/src/router/command.rs` with `.command::<C>()` (or `.migrated::<Current, Legacy>()` while an old shape must keep decoding). The provider must compile for EVERY target — see the `target-agnostic-providers` skill; a hand-written `CommandHandler` impl is no longer used anywhere. Look at `router/members.rs` (`Provider<PromoteMember>`, `Provider<ExpelMember>`) for the smallest complete example; a command that must read raw facts beyond its concept registers a wrapper request type with a hand-written `Decode` (see `CreateSpaceRequest` in `router/repository.rs`).

3. **A trigger.** Two forms, both already wired:
   - Declarative, in a view: `command!: &member/promote` in `rust/tonk-core/assets/library/core.yaml`, bound to a form or `onclick=command`. Fields read `dom.event.*` attributes.
   - Programmatic, from a Rust element: build the same transient as a `TransactRequest` claim and dispatch through `window.tonk.transact(...)`. See `invite_claim_json` / `enable_sync_claim_json` in `rust/tonk-fab/src/logic.rs`.

4. **Outcomes as facts.** The handler asserts durable facts (a `MemberRole` stamp, a `Membership` row, an `Invitation`) or overlay facts (`state:*` entities for per-session, unreplicated state such as a refusal the share control echoes). The page reads them through the subscription it already has. If the page must be told something that no branch can carry (navigate, set title), post to `env.origin().client` as `tonk:join` does; do not invent a response body.

   When the page genuinely waits on one answer (a probe, a ceremony step), use the **answer row**: the command carries an `at` stamp (the page's `Date.now()`), the provider writes a single overlay row (`state:<name>`, e.g. `state:welcome`, `state:branch-inspection`) carrying `answered-at` plus the outcome, and the page queries that row until `answered-at` equals its stamp. `OpenWelcome`/`WelcomeAnswer` and `InspectBranch`/`BranchInspection` are the smallest examples.

## When a route is actually right

- Raw bytes that are not facts: `<img src>` must point at a URL that serves bytes with their content type, and an upload is a body too large to be a claim (`blob`).
- A streaming protocol that is not a query (`/api/language-server`).

A debug surface is not an exception: the inspector asserts `InspectBranch` and queries its answer row. Neither is "the service worker must answer before the page has a branch": the profile branch always exists, so query it. Background work the page needs to nudge (sync) rides a worker message (`{type: "keepalive"}`), not a route.

If you believe you have one of these, add it to `ROUTES` in `route_table.rs` in the same change and say in the PR why a command could not carry it. Reviewers will push back on "the UI needs a response".

## Checklist before writing `.route(`

- Is this triggered by a user or a page? Then it is a command.
- Does it need to answer the caller? Ask what fact the caller would subscribe to instead.
- Does it need the page to do something (navigate, open)? Post to the originating client from the handler.
- Is the same operation needed from the CLI? A command is asserted the same way from `tonk eval`; a route is not.
