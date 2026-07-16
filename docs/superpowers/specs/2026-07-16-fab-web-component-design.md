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
| Share button | `tonk:repository/fab-share` (`core.yaml:755`) |
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
dispatches a `tonk-display:result` carrying `{link}`. The shell asserts its own
furniture into spaces whose creators are supposed to own them.

## Why they ended up there

Not a decision — a constraint. `<tonk-display>` has exactly one branch dial. The
model descriptor (`element.rs:849`), the view row (`:955`), the entity facts
(`:959`), the command descriptor (`refresh_delegate` → `resolve_model`, `:1715`)
and the claim dispatch (`events/delegate.rs:127`) all resolve through the host's
own `with=`. Routing is never inferred from DOM ancestors (`context.rs:1-9`);
the single propagation boundary is attribute stamping via `forward_with`
(`element.rs:2061`).

So one `with` selects the branch for template *and* data simultaneously. A
profile-branch template cannot render space-branch data through
`<tonk-display>`. To show a space's members, the template had to live on the
space.

## The two mechanisms

Both avoid frozen descriptors. One is proven in the FAB; the other is proven
elsewhere and must be carried into it.

**Writes — inline descriptor, target as a parameter. Proven in `tonk-fab`.**
`pause_claim_json` (`tonk-fab/src/logic.rs:281`) inlines the whole concept in
the claim and names the target space in `parameters.space`. Dispatch is
routeless via `window.tonk.transact`, landing on the FAB's own
`main@profile:tonk`. `PauseSyncHandler` (`repository.rs:943`) is registered
globally (`command.rs:123`), decodes the target off the facts, and acts on that
replica. `logic.rs:277`: "nothing space-side is required."

This works because handlers and rules match commands by **attribute URI**, not
by seeded-descriptor identity (`dialog-reactor/src/command.rs:52-83`), with
`it_decodes_create_space_from_name_only_facts` as the standing regression test.
That is the load-bearing property of this whole design.

**Reads — inline predicate over raw attributes. Proven in `tonk-workspace`, not
here.** `<ui-sync-status>` (`tonk-workspace/src/ui_sync_status.rs:161`)
subscribes without naming a concept:

```json
{ "predicate": { "with": { "status": {
      "the": "xyz.tonk.sync/status", "as": "Entity", "cardinality": "one" } } },
  "terms": { "this": "state:here", "status": { "?": { "name": "status" } } } }
```

The field shape rides in the query, so nothing need be seeded. It reaches its
space through its **own `with="main@{did}"` attribute** and plain
`consumer::subscribe` (`ui_sync_status.rs:130`).

`tonk-fab` issues zero subscriptions today. The read direction is therefore
*unproven in this element* — it is the same `apply_route` → query path, but no
existing `ui-` element has been mounted this way from Rust-built markup. Treat
the first zone as a spike.

**We use `with=` per element, not `subscribe_with_route`.** The `*_with_route`
family exists but has exactly one caller in the codebase — the portal bridge
(`tonk-portal/src/bridge.rs:1378`). Since `resolve_with` reads the element's own
attribute with no ancestor walk (`context.rs:23`), the FAB stamps
`with="main@{did}"` on each `ui-` child as it builds the markup, and each child
uses plain `subscribe`. This is the `<ui-sync-status>` precedent exactly, and it
keeps the design on the one path that already has a worked example.

`<ui-sync-status>`'s module doc already articulates the principle this design
generalises:

> Host chrome, NOT space content: it renders the same wireframe "disc"
> indicator regardless of what a space asserts, so a space choosing wild UI can
> never redefine or break it (unlike a stdlib `tonk:view/*` view, which lives on
> the space branch and would need per-space seeding). It is defined in Rust —
> the `ui-` prefix marks it as a host UI primitive, distinct from the `tonk-`
> data elements.

## Design

`<tonk-fab>` grows from a transparent drag/telescope wrapper into the component
that owns the entire FAB. All markup and the ~593-line stylesheet move from
`profile.yaml:812-1688` into `rust/tonk-fab/`. The space becomes a place the
shell reads facts from, never a place it renders from.

### Mount

`tonk:space/chrome`'s view (`profile.yaml:2071-2076`) becomes:

```html
<tonk-site with="main@{id}" allow="main@{id}" path={rest}></tonk-site>
<tonk-fab with="main@profile:tonk" space="{id}"></tonk-fab>
```

`{id}` is the space DID. The FAB stays inside a tonk-display view — that is what
substitutes `{id}` and clones the element — but it is no longer a *display mount
point* for its own content. `with` carries the profile branch for the FAB's own
reads; `space` is stamped onto each `ui-` child's `with` as `main@{did}`.

This retires the `tonk:profile/fab` concept and its view, and with them the
FAB's reliance on `augment_frame`'s zero-instance host-only conclusion
(`element.rs:2325`) and the `{dom.host/data-space}` escape hatch. That special
case stays — the empty-repo launchpad and other chrome use it — but the FAB
stops depending on it.

The chrome view's stable `this:` means the re-seed replaces the mount in place;
existing profiles get no double-FAB.

### Zones

| Zone | Today | Becomes |
|---|---|---|
| Sync disc | `<ui-sync-status with={dom.host/data-space}>` | unchanged element; FAB now performs the `with=` substitution tonk-display's template engine did |
| Profile name | `<tonk-display model=tonk:profile/name>` + `onchange=profile/rename` | Rust markup; inlined `profile/rename` claim; Rust-attached `<tonk-editable>` change listener |
| Space switcher | `view=tonk:view/fab-menu` + `<ui-dropdown exclude=>` | Rust markup; profile-branch subscription; must reproduce self-replica hiding (`kind=tonk:profile`), `status` reflection, and active-space exclusion |
| Create wizard | `profile.yaml:933-1090`, `onsubmit=space/create` | Rust markup; inlined `space/create` claim. See "Wizard" below — this is the largest single piece |
| Repo name chip | `tonk:repository/name-view` (space) | `<ui-space-name>` — subscribes `xyz.tonk.repo/name`; inlined rename claim |
| Share button | `tonk:repository/fab-share` (space) | folded into `<tonk-share>`; inlined mint claim. See "Share" below |
| Invite link | `tonk:view/fab-invite` (space) | subscribes `xyz.tonk.credential/link`. See "Invite lifetime" below |
| Member roster | `tonk:view/fab-roster` (space) | `<ui-member-roster>` — **one** inline descriptor, directory mode (unbound `this`), three `with` fields on the same entity |

The roster is a single predicate carrying `xyz.tonk.membership/name`, `/member`
and `/role` — not three sync-status-shaped subscriptions, which would require
client-side row-joining no existing element does. Those fields are `with`, not
`maybe`, so a member missing a synced name or role is invisible. That matches
today's behaviour, but it becomes the FAB's own choice and should be a
deliberate one.

The switcher could read `xyz.tonk.replica/name` from the profile branch instead
of each space's own name, which would avoid N cross-repo subscriptions. We keep
`<ui-space-name>`: the space's own repo name is the cross-device source of truth
(`profile.yaml:186-190` states this for the Hub), and the profile-side replica
name goes stale. Named here because it is a real freshness-vs-cost trade.

### Read only asserted facts, never rule-derived conclusions

Rules are seeded and frozen exactly like views, so reading a rule's conclusion
reintroduces the dependency one layer down. `tonk:agent-invite` is *derived* by
a rule (`core.yaml:400`) joining `tonk:repository` with `tonk:invitation`,
purely to compose the agent-prompt view. The FAB must not subscribe to it. It
subscribes to `tonk:invitation`'s own `xyz.tonk.credential/link` and already has
the repo name from `<ui-space-name>`, so it needs no join.

The rule and `tonk:agent-invite` stay for the agent-prompt view, which is space
content and correctly space-rendered.

### Invite lifetime

The link is **not** a durable space fact. `InviteHandler` writes it via
`.overlay().assert(Credential{..})` + `.write()` (`repository.rs:768-777`); it
never commits durably and never replicates (`core.yaml:270-272` says so
outright, and the secret is the point — the URL carries the seed in its `#`
fragment). It exists only in the minting session's overlay.

The design works because overlay writes schedule a branch poll and re-fire
subscriptions, so the same-session mint→display flow behaves exactly as today.
But "the shell reads facts the space owns" does not describe this zone, and the
spec should not pretend otherwise. `Credential` is cardinality-one on the repo
subject, so a re-mint supersedes in place — which answers "which of several
invites?" for free.

### Share

"Folded into `<tonk-share>`" hides a rewrite of both of its inputs. Today it
gets the URL by listening for `tonk-display:result` from the `fab-invite` child,
and triggers the mint by letting the click fall through to the child form's
delegate-resolved `onsubmit`. This design deletes both children, breaking both
paths.

The rewrite must preserve the `ClipboardItem`-promise trick (`share.rs:8-22`):
the clipboard write opens synchronously while the click's user activation is
live, and resolves when the mint lands. The mint claim must therefore dispatch
inside that same click handler.

### What the delegate did that Rust must now do

`onsubmit=`/`onchange=` are resolved by tonk-display's delegate and dispatched
on the display host. Rust markup has no delegate. Building the claim is the
easy part; these are the side effects that will bite:

- **`prevent-default`.** Without it the create form does a native submit and
  reloads with `?name=` — the exact bug `extract.rs:631-635` warns about.
- **Overlay dismissal** (`delegate.rs:148-175`). `data-close-dialog` sets the
  `<wa-dialog>`'s `open = false`; `data-close-radio` re-checks the paging radio
  and calls `form.reset()`. The create form carries `data-close-dialog`
  (`profile.yaml:942`), so without this the wizard never closes after Create.
- **`<tonk-editable>` commit wiring.** Both rename fields' change-event→claim
  path is delegate-resolved. Rust must attach the listeners.
- **Empty fields must be omitted, not sent as `""`.** The extractor drops empty
  fields, and a rule premise requiring all fields matches zero rows otherwise.
- **Attribute URIs verbatim**, kebab-cased as declared
  (`dom.event.current-target.elements.name/value`), exactly as
  `pause_claim_json` does.

### Wizard

The zone table understates this by an order of magnitude. Reproducing it in
Rust markup means: CSS-radio paging (`#wiz-start`/`#wiz-template`), the hidden
`Untitled` name sentinel (non-empty or the field is dropped and the command
never fires), four template radios, the submit-inside-`<label>` trick, and
`<tonk-default-remote field="remote" auto>` filling the hidden `remote` input.
All of it must survive verbatim.

### Structure and CSS

`<tonk-fab>` has no shadow root (`element.rs:33`, `shadow() -> false`); the
whole chain is light DOM. So "the stylesheet moves into the crate" needs a
mechanism. Two options exist: the app stylesheet (the `<ui-sync-status>`
precedent — its `.sync`/`.disc` CSS lives there) or a Rust-injected global
`<style>`.

**Choose `include_str!("fab.css")` injected once into the document head**, keyed
by a stable id. Both ship with the app and so both fix drift, but the crate-local
file keeps the component self-contained — the stated goal — and lets the
stylesheet be reviewed and diffed alongside the markup that uses it. It must be
guarded against re-injection: the element re-binds on clone (`__tonkFabBound` is
dropped by `cloneNode`, `element.rs:100-113`), and tonk-display clones the
chrome view.

Owning the markup also *simplifies* the element. `inject_scrim` and
`wrap_telescope_tiles` (`element.rs:196-208`) exist only to retrofit structure
onto view-supplied markup — inferring the cap from `.fab` child 0 and wrapping
the rest as tiles, with the scrim forced to be a sibling of `.fab` because the
view renderer drops empty elements. When the FAB emits its own DOM it can build
the wrappers and scrim directly. The structural-inference code should go, not be
satisfied.

## Consequences

**Rename changes from a rule to a handler.** `tonk/rename-repository`
(`core.yaml:809`) is consumed by a rule seeded on the *space* branch
(`core.yaml:825`). A profile-dispatched claim has no rule there to consume it,
so rename needs a worker handler shaped like `PauseSyncHandler`. This is the one
place the design changes semantics.

**`tonk:invite` mints against its dispatch origin.** Dispatched routeless from
the profile, the origin repo is empty — a known trap. `InviteHandler` needs the
same `space`-parameter treatment `PauseSync` got in #572. `RemoveSpaceHandler`
already demonstrates the empty-origin property this relies on.

**Space-parameter handlers no-op silently on an absent replica.**
`PauseSyncHandler` logs and returns when `.acquire()` fails
(`repository.rs:992-994`). Rename and invite inherit this: a profile-dispatched
rename of a space whose replica isn't local does nothing while the UI looks
successful — precisely the failure class this design attacks. The `ui-`
components must reflect the handler outcome rather than updating optimistically.

**Stale views on existing spaces go inert, not broken.** Once the FAB stops
mounting them, the `fab__` views simply stop being read. No migration needed.

## Deletions

- `core.yaml`: `tonk:view/fab-roster` (:670, :691), `tonk:view/fab-invite`
  (:708, :727), `tonk:repository/fab-share` (:755) — and with them every `fab__`
  chrome reference (10 lines, 15 occurrences, all inside these three views).
- **Keep `tonk:repository/name-view` (`core.yaml:844`).** It is the default
  entity view for `tonk:repository` and the workspace topbar's title chip. Only
  the FAB's implicit no-`view=` mount of it (`profile.yaml:859`) goes.
- **Keep `tonk:view/label` (`core.yaml:903`).** Used by the Hub cards
  (`profile.yaml:511`), the Hub's delete-confirm overlay (`:526`) and
  `wiki.yaml:624`.
- `profile.yaml`: the FAB view (:812-1688), `tonk:profile/fab` (:677),
  `tonk:view/fab-menu` (:699, :718), `tonk:profile/name` view (:795).
- Vestigial, worth removing while here: `<tonk-fab-portal>`
  (`tonk-portal/src/fab.rs`, ~350 lines) is exported at `lib.rs:43` but called
  from nowhere and appears in no view. The FAB is not portaled.
- Stale comment to fix in passing: `repository.rs:968` calls the repo key the
  DID's "suffix"; `repo_key()` returns the full DID (`prelude.rs:80-93`).

## Error handling

A failed subscription does **not** quietly idle. Repo-load failure →
`RepositoryNotFound` → HTTP 404 → `frame_stream` error → `ops.rs:551-561` calls
the consumer's `error` hook and then `schedule_resubscribe` — an unbounded
reconnect loop. The switcher spawns one `<ui-space-name>` per listed space, so a
single stale or never-synced entry becomes a forever-retrying SSE, and a
profile with several becomes N of them.

The `ui-` components therefore need an explicit give-up story: bounded retries
with backoff, then a terminal state the zone renders honestly ("unavailable"),
rather than inheriting the default loop. This is a requirement of the design,
not an implementation detail — it is new load the current FAB does not generate.

Absent facts, by contrast, are normal: they render the existing fallbacks
("Untitled" for an unnamed repo, an empty roster). The FAB's chrome must render
regardless of what any space does or doesn't assert — the whole point of the
`ui-` boundary.

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
  decoding the `space` parameter, mirroring `PauseSyncHandler` — including the
  absent-replica path, which must surface rather than no-op.
- Browser-level verification needs the Chrome + matched chromedriver route;
  wasm tests run `nextest -j1`.

## Sequencing

Land `<ui-space-name>` first, end to end: read (`xyz.tonk.repo/name` via `with=`
+ plain `subscribe`) and write (inlined rename claim + new handler). It is the
only zone exercising both directions, and it validates the one mechanism with no
existing worked example — a top-level `ui-` element subscribing from Rust-built
markup. If that holds, the rest is repetition.

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
