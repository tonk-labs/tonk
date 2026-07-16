# Extract the FAB into a web component

Status: approved, not implemented
Date: 2026-07-16

## Problem

Tonk ships shell updates by editing the standard library. That works for the
profile branch and fails for every space.

`profile.yaml` is re-seeded on every worker boot (`repository.rs:2445`
`bootstrap_profile` → `seed_profile_library`, called from `worker.rs:960`). The
seed is additive and content-addressed, so re-asserting dedupes and a view with
a stable `this:` overwrites. Shell updates on the profile branch ride along with
the app.

`core.yaml` is seeded exactly once, at repository creation
(`repository.rs:1712` `seed_and_initialize`). There is no version stamp, no
library hash, and no re-seed path. Every space's descriptors are frozen at the
version of `core.yaml` that created it.

The FAB straddles both. It renders from the profile branch but reaches into each
space for four views seeded from `core.yaml`:

| Zone | Space-side dependency |
|---|---|
| Repo name chip | `tonk:repository/name-view` (`core.yaml:844`) |
| Share button | `tonk:repository/fab-share` (`core.yaml:757`) |
| Invite link | `tonk:view/fab-invite` (`core.yaml:727`) |
| Member roster | `tonk:view/fab-roster` (`core.yaml:691`) |

Ship a FAB that wants a new or changed space-side view and every older spot
renders a placeholder. Two properties make this worse than it sounds:

- **The failure is quiet.** `state.rs:76-92` `is_loud()` excludes
  `NoModel`/`NoView`/`NoEntity`, so drift renders as a neutral grey box that
  reads as "still loading" rather than an error.
- **A frozen command fails silently while looking successful.**
  `command.rs:44` notes the dialog still closes via `data-dialog`, so the user
  sees a successful interaction that did nothing.

The codebase compensates by convention alone — `command.rs:41` states the
invariant that a command concept must keep decoding against the descriptor an
older version seeded. `docs/space-sync-remotes-and-launchpad.md` §3.1 records
the near-miss: adding a required `remote` field to `CreateSpace` would have
broken space creation for every existing user.

There is a second, subtler cost. Those four views are shell chrome, not space
content. `fab-roster` is `<span class="fab__menu-item fab__menu-item--member">`
— markup meaningless without the profile branch's stylesheet. `fab-invite` is a
*hidden* span whose only job is to make `<tonk-display>` reach `ready` so it
dispatches a `tonk-display:result` carrying `{link}`. Ten `fab__` chrome
references sit on the space branch today. The shell asserts its own furniture
into spaces whose creators are supposed to own them.

## Why they ended up there

Not a decision — a constraint. `<tonk-display>` has exactly one branch dial. The
model descriptor (`element.rs:849`), the view row (`:955`), the entity facts
(`:959`), the command descriptor (`refresh_delegate` → `resolve_model`, `:1731`)
and the claim dispatch (`events/delegate.rs:127` → `ambient_route`) all resolve
through the host's own `with=`. Routing is never inferred from DOM ancestors
(`context.rs:1-9`); the single propagation boundary is attribute stamping via
`forward_with` (`element.rs:2061`).

So one `with` selects the branch for template *and* data simultaneously. A
profile-branch template cannot render space-branch data through
`<tonk-display>`. To show a space's members, the template had to live on the
space.

## The escape hatch already exists

#572 solved this once for sync-pause, and the FAB is the worked example. Both
directions are already proven in `rust/tonk-fab`, and neither touches a frozen
descriptor.

**Reads — inline predicate over raw attributes.** `<ui-sync-status>`
(`ui_sync_status.rs:161`) subscribes without naming a concept:

```json
{ "predicate": { "with": { "status": {
      "the": "xyz.tonk.sync/status", "as": "Entity", "cardinality": "one" } } },
  "terms": { "this": "state:here", "status": { "?": { "name": "status" } } } }
```

The field shape rides in the query. `consumer::subscribe_with_route`
(`consumer.rs:190`) takes an explicit cross-repo `space`/`branch`, so the read
is live and needs nothing seeded.

**Writes — inline descriptor, target as a parameter.** `pause_claim_json`
(`logic.rs:281`) inlines the whole concept in the claim and names the target
space in `parameters.space`. Dispatch is routeless via `window.tonk.transact`,
landing on the FAB's own `main@profile:tonk`. `PauseSyncHandler`
(`repository.rs:943`) is registered globally (`command.rs:123`), decodes the
target off the facts, derives the repo key from the DID, and acts on that
replica. `logic.rs:277` states the result: "nothing space-side is required."

`<ui-sync-status>`'s module doc already articulates the principle:

> Host chrome, NOT space content: it renders the same wireframe "disc"
> indicator regardless of what a space asserts, so a space choosing wild UI can
> never redefine or break it (unlike a stdlib `tonk:view/*` view, which lives on
> the space branch and would need per-space seeding). It is defined in Rust —
> the `ui-` prefix marks it as a host UI primitive, distinct from the `tonk-`
> data elements.

This design applies that principle to the rest of the FAB.

## Design

`<tonk-fab>` grows from a transparent drag/telescope wrapper into the component
that owns the entire FAB. All markup and the ~580-line stylesheet move from
`profile.yaml:812-1688` into `rust/tonk-fab/`. The space becomes a place the
shell reads facts from, never a place it renders from.

### Mount

`tonk:space/chrome`'s view (`profile.yaml:2071-2076`) becomes:

```html
<tonk-site with="main@{id}" allow="main@{id}" path={rest}></tonk-site>
<tonk-fab with="main@profile:tonk" space="{id}"></tonk-fab>
```

`with` carries the profile branch for the FAB's own reads; `space` names the
active space for cross-repo subscriptions and claim parameters. The FAB no
longer mounts through `<tonk-display>` at all.

This retires the `tonk:profile/fab` concept and its view, and with them the
FAB's reliance on `augment_frame`'s zero-instance host-only conclusion
(`element.rs:2325`) and the `{dom.host/data-space}` escape hatch. That special
case stays — the empty-repo launchpad and other chrome use it — but the FAB
stops depending on it.

### Zones

| Zone | Today | Becomes |
|---|---|---|
| Sync disc | `<ui-sync-status>` | unchanged — already correct |
| Profile name | `<tonk-display model=tonk:profile/name>` + `onchange=profile/rename` | Rust markup; inlined `profile/rename` claim |
| Space switcher | `view=tonk:view/fab-menu` | Rust markup; profile-branch subscription |
| Create wizard | `profile.yaml:933-1090`, `onsubmit=space/create` | Rust markup; inlined `space/create` claim |
| Repo name chip | `tonk:repository/name-view` (space) | `<ui-space-name>` — subscribes `xyz.tonk.repo/name`; inlined rename claim |
| Share button | `tonk:repository/fab-share` (space) | folded into `<tonk-share>`; inlined mint claim |
| Invite link | `tonk:view/fab-invite` (space) | subscribes `xyz.tonk.credential/link` |
| Member roster | `tonk:view/fab-roster` (space) | `<ui-member-roster>` — subscribes `xyz.tonk.membership/name`, `/member`, `/role` |

New elements follow the `ui-` convention: Rust-owned markup, inline-predicate
subscription via `subscribe_with_route`, dispatch via inlined claim.

The switcher rows currently read each space's name through the space-side
`tonk:view/label` (`profile.yaml:722`) — the same drift vector as the bar. Each
row reuses `<ui-space-name space={did}>` instead, so the switcher depends on
nothing seeded either.

### Read only asserted facts, never rule-derived conclusions

Rules are seeded and frozen exactly like views, so reading a rule's conclusion
reintroduces the dependency one layer down. The invite is the live example:
`tonk:agent-invite` is *derived* by a rule (`core.yaml:400`) joining
`tonk:repository` with `tonk:invitation`, purely to compose the agent-prompt
view. The FAB must not subscribe to it. It subscribes to `tonk:invitation`'s own
`xyz.tonk.credential/link` — a real fact the worker's `InviteHandler` asserts —
and already has the repo name from `<ui-space-name>`, so it needs no join.

The rule and `tonk:agent-invite` stay for the agent-prompt view, which is space
content and correctly space-rendered.

### What stays on the space branch

Only facts the spot genuinely owns: `xyz.tonk.repo/name`, membership records,
invitations — plus every concept, rule and view the creator writes. The shell
asserts nothing into a space.

## Consequences

Two zones change behaviour rather than just moving. Both are load-bearing.

**Rename is a declarative rule, not a handler.** `tonk/rename-repository`
(`core.yaml:809`) is consumed by a rule seeded on the *space* branch
(`core.yaml:825`). A profile-dispatched claim has no rule there to consume it,
so rename needs a worker handler shaped like `PauseSyncHandler`: decode `space`
→ repo key → act on that replica's `main`. This is the one place the design
changes semantics.

**`tonk:invite` mints against its dispatch origin.** Dispatched routeless from
the profile, the origin repo is empty — a known trap. `InviteHandler` needs the
same `space`-parameter treatment `PauseSync` got in #572.

**Command binding is lost with the delegate.** `onsubmit=`/`onchange=` are
resolved by tonk-display's delegate and dispatched on the display host. Rust
markup has no delegate, so every FAB command becomes an explicit inlined claim —
including `space/create` and `profile/rename`, which are safe today. Their
claim parameters must use the same attribute URIs the descriptors declare
(`dom.event.current-target.elements.name/value` and friends), exactly as
`pause_claim_json` does. Their handlers (`CreateSpaceHandler`,
`ProfileRenameHandler`) decode from facts and match on trigger attributes, so
they need no change.

## Deletions

- `core.yaml`: `tonk:view/fab-roster` (:670, :691), `tonk:view/fab-invite`
  (:708, :727), `tonk:repository/fab-share` (:757), and the FAB's use of
  `tonk:repository/name-view` (:844) — all 10 `fab__` references.
- `profile.yaml`: the FAB view (:812-1688), `tonk:profile/fab` (:677),
  `tonk:view/fab-menu` (:699, :718), `tonk:profile/name` view (:795).
- `tonk:view/label` (`core.yaml:903`) stays — the Hub still uses it
  (`profile.yaml:511`). See Out of scope.
- Vestigial and worth removing while here: `<tonk-fab-portal>`
  (`tonk-portal/src/fab.rs`, ~350 lines) is exported at `lib.rs:43` but called
  from nowhere and appears in no view. The FAB is not portaled.

## Error handling

Follow `<ui-sync-status>`: a subscribe failure logs and leaves the zone in its
initial state rather than throwing. Absent facts render the existing fallbacks
("Untitled" for an unnamed repo, an empty roster) — the FAB's chrome must render
regardless of what any space does or doesn't assert, which is the whole point of
the `ui-` boundary.

Failure to reach a space is now visible rather than quiet: unlike a missing
seeded view, a failed subscription is the component's own state and can be
styled honestly.

## Testing

- Pure logic (claim JSON builders, query bodies) is native-testable in
  `logic.rs`, as `pause_claim_json` already is.
- Guard tests in `rust/tonk-worker/tests/standard_library.rs` parse →
  `analyze_local` → lower each real asset; they must stay green after the
  deletions.
- The load-bearing regression test: **seed a branch from an OLD `core.yaml` and
  assert the FAB renders fully against it.** This is the property the whole
  design buys and nothing currently tests it.
  `plan/tonk-layout-headless-split.md:343-363` is the nearest existing pattern
  for pre-seeding an old-schema branch.
- New worker handlers (rename, invite) get `#[dialog_common::test]` coverage for
  decoding the `space` parameter, mirroring `PauseSyncHandler`.
- Browser-level verification needs the Chrome + matched chromedriver route;
  wasm tests run `nextest -j1`.

## Out of scope

- Versioning or re-seeding `core.yaml`. This design routes around the frozen
  seed for the shell; it does not fix drift for anything else.
- **The Hub keeps the same drift vector.** Its cards read each space's name
  through the space-side `tonk:view/label` (`profile.yaml:511`), exactly as the
  FAB does today. `<ui-space-name>` is the drop-in fix and the Hub should adopt
  it next, but that is a separate change with its own layout risk. Worth naming
  plainly: this design makes the *FAB* seed-independent, not the whole shell.
- The stale `tonk:workspace/shell` doc comment (`core.yaml:1501-1509`), which
  describes a two-portal composition that no longer exists.
- The ~25 stale `topbar` references across `tonk-schema` and `tonk-worker`.
