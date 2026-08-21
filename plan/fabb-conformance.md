# FABB conformance

Bring `rust/tonk-fab` to the FABB spec, then conform the Hub to `hub.html`.
Reference: `~/tonk/gooey/fabb/` — `README.md` (the laws), `fabb.js` (the
reference implementation, v0.8.2, 968 lines), `fabb.html` (the living spec),
`hub.html` (the Hub wireframe), `screens/` (renders).

Order was chosen deliberately: the bar floats over every `/space/{id}`, two of
the Hub's own flows route through it (create → enter → rename in place; the
space stack's `open ▸ … more ↖`), and `hub.html` restates the bar's tokens
verbatim — "the hub is chrome, it runs on the same file as the bar".

## Where things live today

| piece | today | notes |
|---|---|---|
| the bar | `rust/tonk-fab` — `element.rs` (2411), `markup.rs` (379), `fab.css` (912), `logic.rs` (1813) | light DOM + one global stylesheet |
| the Hub | `rust/tonk-core/assets/library/profile.yaml` → `view/directory!: id:tonk:space/directory` | declarative view, Web Awesome |
| shell | `rust/tonk-ui` — `index.html`, `styles.css`, `/account` page | |
| bar's data children | `rust/tonk-workspace` — `<ui-sync-status>`, `<ui-dropdown>`, `<tonk-default-remote>`; `tonk-fab` — `<ui-space-name>`, `<ui-space-switcher>`, `<ui-member-roster>`, `<tonk-share>`, `<ui-profile-name>` | separate custom elements that own their subscriptions |

## Cell map — today vs the spec

FABB: `[circle 36][space 216][changes 432][share 144][fold 24][mode 18]`,
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

## What is already in place

- **Four-corner docking**: `logic.rs::Dock` (`TopLeft`/`TopRight`/`BottomLeft`/
  `BottomRight`), `nearest_dock`, and the `fab-mirror` class already implement
  snap-to-nearest-corner and the right-anchored mirror. The spec's `flip` law
  is largely this, already built and persisted as a profile claim. (The
  comment in `profile.yaml` naming only `tonk:top-left`/`tonk:bottom-left` is
  stale.)
- **Drag + threshold**: `DRAG_THRESHOLD_PX` / `TOUCH_DRAG_THRESHOLD_PX`,
  pointer capture, click-vs-drag suppression.
- **Telescope collapse**: `.fab__tele` wrappers, `max-width` animation.
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
   `[circle 36][space 216][share 144][fold 24][mode 18]`; cell widths and the
   flush-run geometry stay spec-correct, the bar is just shorter. Add the rung
   with the feature it serves.
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
4. **Done** — `bar.rs` + `markup.rs`: cells, separators, fold glyph, mode
   pill, flip, stacks, in-place disclosure, rename in place.
5. **Done** — `element.rs`: the float. Drag/snap/dock kept and adapted to
   read the handle through the composed path (shadow retargets `target` to
   the host). `fab.css` and `tests/fab_stylesheet.rs` deleted with the global
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

10. **Next** — the Hub itself: `profile.yaml`'s `view/directory!:
    id:tonk:space/directory`, conformed to `hub.html`. Scope is the hub page
    only (masthead, hubbar, the 432px spaces stack with hover verbs, the
    remove-confirm cluster); the settings overlay, account stack and usage
    banner stay out. Tokens are settled — reuse the bar's, which `hub.html`
    restates verbatim.

## Knowingly not conformant

- The `changes` rung, and with it `alert` — nothing drives either (Decision 1).
  The alert CSS exists and is correct; no data sets the attribute.
- `tonk-field`, `tonk-toggle`, and the spec's standalone `tonk-fab` (a bare
  sync circle) — no consumer. The tag `tonk-fab` is spent on the bar, which is
  the mount contract.
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
