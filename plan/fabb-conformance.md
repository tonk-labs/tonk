# FABB conformance

Bring `rust/tonk-fab` to the FABB spec, then conform the Hub to `hub.html`.
Reference: `~/tonk/gooey/fabb/` — `README.md` (the laws), `fabb.js` (the
reference implementation, v0.8.2, 968 lines), `fabb.html` (the living spec),
`hub.html` (the Hub wireframe), `screens/` (renders).

Order was chosen deliberately: the bar floats over every `/space/{id}`, two of
the Hub's own flows route through it (create → enter → rename in place; the
space stack's `open ▸ … more ↖`), and `hub.html` restates the bar's tokens
verbatim — "the hub is chrome, it runs on the same file as the bar".

## Revised mobile product decision (2026-08-24)

The product no longer treats the reference telescope, fold control, or strip
panning as current laws. Those behaviors remain mentioned below only as the
historical baseline from which the component was ported.

The current bar uses one fit-driven partition after safe-area-aware left and
right float insets are removed from the available width:

- At 414px or more it renders the complete 36px-high run:
  `sync · space · share · appearance`. There is no fold, overflow, or collapse
  affordance in this layout.
- Below 414px it renders a 44px-high compact run. Sync, the ellipsizing space
  name, and overflow remain visible. Share stays in the run when at least
  352px is usable; appearance is always in the vertical overflow menu.
- Compact overflow is `share ▸ · appearance` when share does not fit, and
  `appearance` when it does. The one canonical share menu is re-anchored
  rather than cloned; `back ◂` returns to overflow.
- The 44px sync disc toggles the compact run directly. It closes any expanded
  dropdown before collapsing, and expands the run when it is already
  collapsed. The state is local to the mounted element and a full-width resize
  clears it; there is no duplicate collapse row in overflow.
- Dropdown blocks and their bar use one fixed 7px visible gap. Opening fades
  the dropdown without translating it through that gap.

This revision preserves real-DOM mirroring and focus order, edge docking,
`max(16px, safe-area + 8px)` on every side, the visual-viewport keyboard lift,
the `new · open ▸ · rename` space stack, and the canonical share roster.

## Where things live today

| piece | today | notes |
|---|---|---|
| the bar | `rust/tonk-fab` — `element.rs` (2411), `markup.rs` (379), `fab.css` (912), `logic.rs` (1813) | light DOM + one global stylesheet |
| the Hub | `rust/tonk-core/assets/library/profile.yaml` → `view/directory!: id:tonk:space/directory` | declarative view, Web Awesome |
| shell | `rust/tonk-ui` — `index.html`, `styles.css`, `/account` page | |
| bar's data children | `rust/tonk-workspace` — `<ui-sync-status>`, `<ui-dropdown>`, `<tonk-default-remote>`; `tonk-fab` — `<ui-space-name>`, `<ui-space-switcher>`, `<ui-member-roster>`, `<tonk-share>`, `<ui-profile-name>` | separate custom elements that own their subscriptions |

## Historical cell map — original port vs the reference

Historical reference: `[circle 36][space 216][changes 432][share 144][fold 24][mode 18]`,
flush cells on one surface, 1px weighted separators, bookends swapping on
right-snap.

| spec cell | today | gap |
|---|---|---|
| circle (sync disc) | `.fab__cap-l` + `<ui-sync-status onpause>` | disc geometry: filled / hollow / 135° half-fill; alert blink |
| space 216 | `.fab__repo` — `<ui-space-name>` + `<ui-dropdown>`/`<ui-space-switcher>` | needs the stack IA `new · open ▸ · rename · settings`, rename in place via block cursor |
| changes 432 | — | **does not exist** (see Open decisions) |
| share 144 | `.fab__share` + `<tonk-share>` + `<ui-member-roster>` | reskin; stack grammar |
| fold 24 | `.fab__more` chevron + `.fab__strip`/`.fab__page` pager | spec wants ▸/◂ directional fold + swipe pan, not a pager |
| mode 18 | — | **does not exist** — the app follows OS only (`index.html`) |
| account | `.fab__account` — `<ui-profile-name>` + `/account` link | **not in the spec's bar** — account is Hub chrome |

## Historical baseline before the mobile revision

- **Edge docking**: release now glides to the nearest edge while keeping its
  free coordinate along that edge, matching `fabb.js::_snap`. `logic.rs::Dock`
  (`TopLeft`/`TopRight`/`BottomLeft`/`BottomRight`) remains the persisted
  fallback seat restored on the next page load, matching `hub.html`; the live
  page is no longer pinned to those four corners.
- **Drag + threshold**: `DRAG_THRESHOLD_PX` / `TOUCH_DRAG_THRESHOLD_PX`,
  pointer capture, click-vs-drag suppression.
- **Historical telescope collapse**: `.fab__tele` wrappers and their
  `max-width` animation existed before the revised mobile product decision.
- Rename dispatch (`ui-space-name::dispatch_rename`), pause-sync dispatch,
  share mint with its two repair prompts.

So the structural laws are less far off than the crate's size suggests. The
real work is material, type, cell geometry, the stack grammar, and the
component family.

## Components to add

`fabb.js` is eight shadow-DOM elements. `tonk-fab` today is one light-DOM
element. Porting means:

- `tonk-fab` (the circle alone), `tonk-bar`, `tonk-menu`, `tonk-mi`,
  `tonk-dialog`, `tonk-button`, `tonk-field`, `tonk-toggle`.
- The shared `SKIN` token block (`--_ink`, `--_soft`, `--_bg`, `--_ring`,
  `--_filter`, …) with the light/dark twin, plus the `--fabb-*` public API.
- Internal tokens must stay `--_`-prefixed: a host page's own `--ink` will
  arrive uninvited (the reference README records this happening).

**Shadow DOM is the decision.** Law 6 ("the chrome themes itself, never the
view") is a platform guarantee under shadow and a convention without it, and
`element.rs` currently returns `shadow() == false` with a globally injected
stylesheet. The existing data children (`<ui-sync-status>` etc.) stay
light-DOM and are slotted, exactly as `fabb.js` slots `<tonk-menu>`.

## Decisions

1. **The `changes` rung — omitted.** 432px, the widest cell, and the one the
   alert law is built around, but nothing in the repo implements proposals or
   history points (`grep -rn proposal rust/` returns nothing). Building it
   would be dead chrome. The bar is therefore
   `[circle 36][space 216][share 144][mode 18]`; the revised compact product
   route is recorded above. Add the changes rung with the feature it serves.
2. **Templates — dropped entirely.** The wireframe's `new` creates
   `untitled`, enters the space, and arms rename in place
   (`bar.editSpace()`); there is no wizard. Templates are not retained
   unlinked — they go. This reaches beyond the create flow:
   - `rust/tonk-core/assets/library/{sheets,wiki,board}.yaml` (~5.1k lines)
   - the three `copy-file` links in `rust/tonk-ui/index.html`
   - `template_from_facts`, `SHEETS_/WIKI_/BOARD_LIBRARY_URL` and the
     template seeding in `rust/tonk-worker/src/router/repository.rs`
   - the template assertions in `rust/tonk-worker/tests/standard_library.rs`
   - the `template` field on `space/create` in `profile.yaml`
   - `bench/scenarios/wiki-conversion` (already unwinnable as checkpointed,
     so little is lost)

   `prose.yaml` / `table.yaml` / `issue.yaml` are already referenced by
   nothing. Sequenced as its own commit — it touches the worker and bench and
   is orthogonal to the bar's material.
3. **The `account` cell — dropped from the bar**, per the spec: account is Hub
   chrome. `/account` stays reachable. A removal from the space view.
4. **Shadow DOM** — see above; law 6 becomes a guarantee rather than a
   convention.

## Sequence

1. **Done** — `skin.rs`: the `SKIN` token block, light/dark twin, the disc,
   the block cursor, the blink keyframes.
2. **Done** — `shadow.rs`: shadow attach, mode plumbing reactive to
   `prefers-color-scheme`, the composed event emitter, `mount_edit`.
3. **Done** — `menu.rs` / `mi.rs`: the stack grammar, including the single
   masked underlay ("one filter per stack") and the clip-aware flyout.
4. **Done, revised** — `bar.rs` + `markup.rs`: cells, separators, mode pill,
   real-DOM flip, stacks, in-place disclosure, rename in place, fit-driven
   full/compact action layouts, vertical overflow, and sync-disc collapse.
5. **Done** — `element.rs`: the float. Drag/snap/dock kept and adapted to
   read the handle through the composed path (shadow retargets `target` to
   the host). Run-width changes remain anchored to the sync disc. `fab.css`
   and `tests/fab_stylesheet.rs` deleted with the global
   stylesheet they tested.
6. **Done** — `dialog.rs` / `button.rs`, and the two share refusal prompts
   re-authored in FABB grammar, mounted on `<body>`.
7. **Done** — the data children went headless and the stack verbs are wired.
   `<ui-space-name headless>` and `<ui-sync-status headless>` write
   `label` / `state` onto the bar; `<ui-space-switcher>` and
   `<ui-member-roster>` render rows; `fabb-pick` now dispatches new / open /
   rename / settings / more / copy link.

   Three things this surfaced:
   - **Rows must be stack SIBLINGS.** `tonk-menu` masks its glass to direct
     `tonk-mi` children and lays them out with 7px gaps, so rows rendered
     inside the switcher or roster get no glass and add a stray gap where the
     producer sits. `stack_rows.rs` inserts them before the producer and tags
     them by owner so a rebuild clears only its own.
   - **`new` still needs a remote.** The deleted wizard got it from
     `<tonk-default-remote auto>`; without it the worker happily creates a
     silently local-only space. A hidden create form carries it.
   - **The flip mirrors the CELLS too**, departing from the reference's law
     10 ("content order never changes"). Holding the order fixed made space
     and share trade places relative to the circle when the bar changed
     sides. Mirroring is what preserves the shape. The mid-drag pivot
     compensation — shifting the element by the handle's displacement so the
     circle stays under the pointer — was in the pre-rewrite `element.rs`,
     got dropped, and is restored.
8. **Done** — the conformance sweep against the laws, which found four gaps:
   - **Nothing detached a listener.** Dropping a `Closure` invalidates it but
     leaves the registration, so the next fire throws "closure invoked after
     being dropped". Live, not theoretical: `mi`/`menu` re-wire on reconnect
     and the in-place sub-stack disclosure moves a stack between parents.
     `shadow::Bound` now detaches on drop, and every listener uses it.
   - **A stack could not be dismissed** without picking from it — no
     click-away, no Escape (law 10's stack behaviour). Both are now
     document-scoped; Escape defers to a rename in flight.
   - **The mode pill did not persist** (law 8). Stored per device under
     `tonk-mode`, the key `hub.html` uses, so the Hub's switch will agree.
     Absent means follow the system — the pill overrides, it does not replace.
   - **No keyboard lift.** The bar seats bottom-right on a phone, which is
     where the keyboard goes. Ported from the reference's `_initLift`.

9. **Done** — the templates teardown (`949aebb10`). Libraries, Trunk rules,
   `template_from_facts`, `seed_library_urls`, the field on `space/create` /
   `CreateSpaceRequest` / `PendingIntent`, and the Hub's create wizard, which
   is now a direct submit. A space always seeds `core.yaml` alone.

   `PendingIntent` is persisted across an account round trip, so an intent
   parked before this still carries `template`; serde ignores unknown fields,
   and the wire-shape test keeps sending it to prove the upgrade is safe.

10. **Done** — the Hub: `profile.yaml`'s `view/directory!:
    id:tonk:space/directory`, conformed to `hub.html`. Masthead logotype,
    hubbar, the 432px spaces stack with hover verbs, the remove-confirm
    cluster, the empty state, one breakpoint at 640. Two supporting elements
    in `tonk-workspace`: `<ui-mode-switch>` (the ink cap) and
    `<ui-copy-link>` (a verb that answers in place).

## The sealed guest, and what it costs

Everything a view renders happens inside the portal guest —
`sandbox="allow-scripts"`, an **opaque origin**. `tonk_workspace`,
`tonk_display` and `tonk_fab` are registered only in `tonk-guest`; the top
page registers `tonk_portal::register_site()` and the account/activate
routes. Consequences that shaped the Hub:

- **`localStorage` throws there.** The mode persistence added to the bar in
  step 8 was therefore dead on arrival — `window().local_storage()` returns
  `Err`, `.ok().flatten()` swallowed it, and nothing was ever stored. Removed
  from both the bar and the cap rather than left pretending. The override now
  lasts the session, which is what the sandbox actually allows.
- **Clipboard works**, because `tonk-portal::shared` delegates the
  `clipboard-write` Permissions Policy into the guest. `<ui-copy-link>` still
  reports "couldn't copy" on a rejected write rather than claiming success.
- **The theme class is propagated**: the bridge stamps the page's `rootClass`
  into the guest and keeps `wa-dark`/`wa-light` live off
  `prefers-color-scheme`. So the Hub's dark twin keys off `:root.wa-dark`,
  the app's existing signal, rather than a second one of its own.

### Follow-up: persisting the mode

Two viable routes, both their own change:

1. A `mode` page effect in `tonk-host::page_effect`, beside `navigate` and
   `title`. The PAGE stores it, and `index.html` can then apply it before
   first paint — no flash. Most correct.
2. A profile claim through `window.tonk.transact`, exactly as the FAB stores
   its dock. Already proven to work from inside a guest, but it lands after
   the guest has painted, so the theme would flip on load.

## Hub deltas from the wireframe

- **One account rung, not two.** `hub.html` has `account ▸` (a switcher) and
  `settings` (an overlay). Both are out of scope, and both live at `/account`
  today — it already IS the settings surface. Two cells to one destination
  would be chrome pretending to offer a choice.
- **No per-row sync disc.** `hub.html` has none, so this follows it — but the
  previous launcher did, via `<ui-sync-status>` per row. Rows still dim while
  seeding (`data-status`) and when not replicated (`data-local`); finer sync
  state (paused, offline, conflict) is now only visible on the bar inside the
  space. Worth a second look if that reads as a loss.
- **`copy link` copies the space URL**, which is a bookmark for existing
  members, not an invite. Sharing with someone new is the bar's `share`,
  which mints and delegates.

## Knowingly not conformant

- The `changes` rung, and with it `alert` — nothing drives either (Decision 1).
  The alert CSS exists and is correct; no data sets the attribute.
- `tonk-field`, `tonk-toggle`, and the spec's standalone `tonk-fab` (a bare
  sync circle) — no consumer. The tag `tonk-fab` is spent on the bar, which is
  the mount contract.
- The space stack's `settings` row — there are no space settings for it to
  open, so it reads `new · open ▸ · rename`. Same reasoning as the `changes`
  rung; it comes back with the surface it leads to.
- The flip mirrors the cells, departing from law 10's fixed content order —
  a deliberate revision, see `bar::apply_flip`.
- The resting seat defaults to bottom-right on every device; the reference
  defaults desktop to top-right and only coarse pointers to bottom-right.
  Pre-existing, and the dock persists after the first drag either way.
- The disc renders three shapes for eight sync states; the precise one rides
  as `data-sync-status` for the accessible name.

`tonk-field` and `tonk-toggle` are deliberately NOT built: the space rename
uses the bar cell's own block cursor, and the settings pane is out of scope,
so both would be dead components. Add them with the surface that needs them.

Tests: 96 native pass, including 15 new `markup` / `skin` cases asserting the
laws (fixed cell widths, ink-only alerts, lowercase chrome vs. user words,
stack gap, the absent `changes` rung, geometry-only glyphs).
`tests/frame_delivery.rs`, `tests/space_name_element.rs` and
`rust/tonk-worker/tests/fab_drift.rs` still to be revisited in step 7.
