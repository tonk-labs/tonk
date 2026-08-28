# FABB mobile experience implementation plan

> **Implementation amendment — 2026-08-24:** The sync disc is now the sole
> compact collapse/expand control. Tapping it collapses an expanded compact
> FABB even while a dropdown is open, and tapping it again expands the run.
> There is no `collapse` row in overflow; this supersedes the explicit
> collapse-row wording retained below as the original implementation record.
> Dropdown rows and the dropdown-to-FABB boundary use the same fixed 7px
> visible gap, with opacity-only disclosure so motion never narrows that gap.

**Goal:** Keep the FABB fully visible whenever it fits, and replace its current
fold/drop behavior on narrow screens with a touch-safe compact bar, vertical
overflow, and an explicit compact-only collapse to the sync disc.

**Approach:** Make responsive layout a pure, fit-driven partition of the
existing top-level actions, then have the DOM render that partition without
duplicating action state or menus. Preserve the FABB's edge docking, shadow-DOM
isolation, stack grammar, and existing space/share workflows. Treat collapse
as a deliberate overflow action on compact screens, use the collapsed disc as
the expand control, and harden the existing safe-area and visual-viewport
behavior for phones.

**Constraints:**

- The mount contract remains `<tonk-fab with="main@profile:tonk" space={id}>`
  in `rust/tonk-core/assets/library/profile.yaml`. The imperative `open(cell)`,
  `close()`, and `editSpace()` methods remain available.
- The full bar is always fully visible. It has no collapse, expand, or fold
  control, and it ignores any stale compact collapse state.
- Responsiveness is based on usable width, not a device or pointer label. A
  phone in landscape or a narrow desktop window gets whichever layout fits.
- The compact bar always keeps the sync disc and space name visible. `share`
  remains horizontal when it fits; appearance always moves into overflow in
  compact mode. The arrow exists only while at least one action is overflowed.
- `collapse` is the final row in the compact overflow menu. It retracts the
  action run to the 44px sync disc; tapping that collapsed disc expands the
  compact bar. An expanded-disc tap does not collapse it.
- Collapse is local element state, not profile state. A newly mounted FABB
  starts expanded, and a viewport resize that can fit the full bar forces it
  expanded.
- Hidden actions are not cloned. The visible cell and overflow row must invoke
  the same canonical action and, for share, the same canonical `<tonk-menu>`.
- Compact controls and menu rows have at least a 44px touch target. The 14px
  sync mark remains visually unchanged inside its larger cell.
- The bar remains usable below the preferred compact space-name width: the
  name truncates with an ellipsis before either bookend disappears.
- The existing space stack remains `new · open ▸ · rename`; the share
  stack remains `copy link` plus the member roster. This pass changes their
  route into view, not their underlying operations or authority.
- Right-edge docking continues to mirror the real DOM and focus order so the
  sync disc stays at the viewport edge.
- Keep the existing safe-area policy: every dock edge is
  `max(16px, safe-area-inset + 8px)`. Reuse the existing visual-viewport
  keyboard lift instead of adding a second positioning system.
- The FABB still runs in a sealed opaque-origin guest. Do not add localStorage,
  a new persistence mechanism, a framework, a motion library, or another icon
  dependency.
- Internal CSS tokens remain `--_`-prefixed and public tokens remain
  `--fabb-*`; page CSS must not cross the shadow boundary.
- Keep the absent `changes` rung absent. Adding proposals/history remains tied
  to the feature that can actually drive it.
- Do not add sync details or pause/resume controls to the mobile FABB in this
  pass. The disc continues to communicate state visually and through its
  accessible name; detailed controls belong in a broader space-status design.
- Update the old reference/conformance wording where this design deliberately
  supersedes the telescope and fold laws.

## Approved layout policy

The current product bar is
`[sync 36][space 216][share 144][fold 24][mode 18]`. Removing the fold makes
the full natural width 414px:

```text
full, left anchored:   [sync 36][space 216][share 144][mode 18]
full, right anchored:  [mode 18][share 144][space 216][sync 36]
```

The layout calculation receives usable width after subtracting the resolved
left and right float insets:

- `usable >= 414`: full. Show sync, space, share, and mode; do not render the
  overflow arrow.
- `usable < 414`: compact. Use a 44px-high bar, a 44px sync cell, a 44px
  overflow cell, and a space cell capped at 216px.
- `usable >= 352` while compact: keep the 144px share cell visible. The 352px
  threshold is `44 sync + 120 preferred space minimum + 144 share + 44 more`.
- `usable < 352`: move share into overflow. Give space the remaining
  `usable - 88` pixels, capped at 216px; below 120px it may shrink further and
  ellipsize rather than dropping the bar.
- Appearance is always an overflow row in compact mode, so compact always has
  at least one overflow action and therefore always shows the arrow.
- Compact starts expanded. Choosing `collapse` changes only its presentation;
  the same `BarLayout` still determines which cells return on expansion.

At the default 16px side insets this means a 390px viewport has 358px usable
and can retain share, while a 375px viewport has 343px usable and moves share
into the vertical menu. Exact-fit comparisons are inclusive so resize cannot
oscillate at a threshold.

## Interaction model

- The compact arrow points toward the menu's opening direction: up when the
  bar is docked at the bottom, down when it is docked at the top. It reflects
  `aria-expanded`; it does not horizontally expand the bar.
- Overflow is a 216px stack, capped to the usable viewport width and aligned
  to the arrow's outer edge. It opens above or below according to the existing
  `up` placement and never flies sideways on a coarse pointer.
- When share is hidden, overflow contains `share ▸` followed by the
  appearance action and `collapse` as the final row. Choosing share replaces
  overflow in place with the one canonical share stack and exposes `back ◂`
  as its first row. Back restores overflow at the same anchor. When share is
  visible, its stack opens from the share cell and has no back row; overflow
  reads `appearance · collapse`.
- The appearance row performs the same app-wide theme change as the desktop
  half-pill, then closes the menu. Its label describes the action (`dark mode`
  or `light mode`), not merely the current state.
- Choosing collapse closes the menu, restores any hoisted sub-stack, and
  retracts the compact action run toward the docked disc. The collapsed state
  is exactly one 44px disc cell; it does not retain a separate arrow.
- Tapping the collapsed disc restores the compact action run. Its accessible
  label is `expand FABB · sync: <exact status> · drag to move`; expanded and
  full labels report sync status and drag affordance without advertising a
  tap action.
- The existing Option-click sync pause shortcut is unchanged but receives no
  new mobile UI. Sync details, transaction feedback, and pause/resume IA are
  outside this pass.
- Pointer travel below the existing mouse/touch threshold resolves as a tap;
  on a collapsed compact bar that tap expands. An expanded-disc tap is inert.
  Travel beyond the threshold is a drag, closes any menu, suppresses the
  trailing click, and preserves the current edge-snap behavior.
- Click-away and Escape dismiss a menu and return focus to its opener when the
  opener still exists. If a resize removes the opener, close without focusing
  a hidden control and move focus to the space cell.

## File map

- `rust/tonk-fab/src/logic.rs`: pure responsive partition and extracted
  viewport-lift calculations with native tests.
- `rust/tonk-fab/src/markup.rs`: full/compact/collapsed bar markup, overflow
  stack markup, responsive geometry, touch targets, and disclosure motion.
- `rust/tonk-fab/src/bar.rs`: action/panel state, layout application, canonical
  stack anchoring, overflow/back/collapse navigation, focus restoration, and
  mode-row updates.
- `rust/tonk-fab/src/element.rs`: compact collapsed-state tap versus drag
  behavior, responsive observer ownership, safe-area-aware usable-width input,
  and viewport lifecycle wiring.
- `rust/tonk-fab/src/menu.rs`: compact propagation and capped menu width.
- `rust/tonk-fab/src/mi.rs`: 44px compact row targets.
- `rust/tonk-fab/tests/responsive_overflow.rs`: real-DOM responsive,
  overflow, focus, compact collapse, and resize regressions; replaces the
  current desktop-capable telescope browser test.
- `rust/tonk-fab/tests/drag_snap.rs`: touch tap/drag separation and menu-close
  behavior without weakening the existing edge-snap regression.
- `rust/tonk-fab/tests/telescope.rs`: delete after equivalent no-collapse and
  responsive-overflow coverage exists.
- `plan/fabb-conformance.md`: record that mobile overflow deliberately
  supersedes telescope collapse, fold, and strip panning.

### Task 1: Define the fit-driven layout policy

**Files:**

- Modify: `rust/tonk-fab/src/logic.rs:is_compact` and its `compact` test module
- Test: `rust/tonk-fab/src/logic.rs`

**Interfaces:**

- Produces:

  ```rust
  pub const FULL_BAR_WIDTH_PX: f64 = 414.0;
  pub const COMPACT_CELL_PX: f64 = 44.0;
  pub const COMPACT_SPACE_MIN_PX: f64 = 120.0;
  pub const SPACE_CELL_PX: f64 = 216.0;
  pub const SHARE_CELL_PX: f64 = 144.0;

  #[derive(Clone, Copy, Debug, PartialEq)]
  pub struct BarLayout {
      pub compact: bool,
      pub space_width_px: f64,
      pub show_share: bool,
      pub show_mode: bool,
      pub show_overflow: bool,
  }

  pub fn bar_layout(usable_width_px: f64) -> BarLayout;
  ```

- Replaces: `is_compact(expanded_width, viewport_width)`. No production caller
  currently uses `is_compact`, so replace it rather than keeping two layout
  policies.
- Consumes: usable width already reduced by the resolved left/right
  `EdgeInsets`; DOM measurement stays outside this pure module.

- [ ] Add native tests for usable widths `414`, `413.9`, `352`, `351.9`,
  `216`, and `80`. Expect full at 414; compact with share at 352; compact
  without share below 352; space capped at 216 and never negative; overflow
  present exactly when compact; mode visible exactly when full.
- [ ] Run `nix develop . -c cargo test -p tonk-fab --lib`; expect compilation
  to fail because `BarLayout` and `bar_layout` do not exist.
- [ ] Implement the constants and pure functions. Clamp a negative or
  sub-bookend usable width to zero before calculating `space_width_px`; do not
  introduce a second breakpoint constant in DOM code.
- [ ] Run `nix develop . -c cargo test -p tonk-fab --lib`; expect all
  `tonk-fab` native tests to pass.

### Task 2: Replace collapse and fold with full and compact action layouts

**Files:**

- Modify: `rust/tonk-fab/src/markup.rs:BAR_CSS`, `BAR_HTML`, `STACKS_HTML`, and
  markup tests
- Modify: `rust/tonk-fab/src/logic.rs:strip_at_end`, `strip_page_target`,
  telescope timing helpers, and their tests
- Modify: `rust/tonk-fab/src/bar.rs:BarState`, `build`, `apply_flip`, `open`,
  `close`, `sync_expanded`, and `apply_responsive`
- Modify: `rust/tonk-fab/src/element.rs:observed_attributes`, `attach_drag`,
  and `attach_responsive`
- Create: `rust/tonk-fab/tests/responsive_overflow.rs`
- Delete: `rust/tonk-fab/tests/telescope.rs`

**Interfaces:**

- Consumes: `logic::bar_layout(usable_width_px)` from Task 1 and the existing
  `float_insets` values.
- Produces:

  ```rust
  #[derive(Clone, Copy, Debug, PartialEq, Eq)]
  enum Cell { Sync, Space, Share, More }

  #[derive(Clone, Copy, Debug, PartialEq, Eq)]
  enum Panel { Space, Share, Overflow }

  struct OpenPanel {
      panel: Panel,
      anchor: Cell,
      return_to: Option<Panel>,
  }

  fn apply_responsive(this: &HtmlElement, usable_width_px: f64, state: &Shared);
  fn open_panel(
      this: &HtmlElement,
      state: &Shared,
      panel: Panel,
      anchor: Cell,
      return_to: Option<Panel>,
  );
  ```

- Compatibility: the imperative `open("space")` and `open("share")` map to
  `Panel::Space` and `Panel::Share`. Unknown names remain no-ops. No code
  outside `tonk-fab` consumes `fabb-collapse` or `fabb-fold` at the current
  branch head, so remove those events together with their controls.

- [ ] Add a browser test that mounts the bar in a sized parent and asserts:
  at 500px parent width the full bar shows share and mode with no more arrow;
  at 390px it is compact with share plus more; at 375px it is compact with
  share hidden and more present. Account for the computed 16px side insets,
  and assert the exact-fit cases do not flap after two observer deliveries.
- [ ] Add a browser assertion that clicking the sync disc in full or expanded
  compact mode never adds the legacy `collapsed` attribute and that
  `data-cell=fold` no longer exists. Expect this test to fail against the
  current telescope behavior.
- [ ] Run
  `nix develop . -c test:web:debug -E 'package(tonk-fab)'`; expect the new
  full/compact DOM assertions to fail because the current bar still folds,
  collapses, and drops its strip.
- [ ] Replace `.tele` with a non-scrolling `.run`; remove the fold cell,
  `.folded`/`.rfold`/`.xopen`/`.rd` classes, `collapsed` CSS, telescope
  transition, strip panning, `set_fold_glyph`, and the fold click listener.
  Delete the now-unreferenced `strip_at_end`, `strip_page_target`,
  `TELESCOPE_MS`, `TELESCOPE_STAGGER_MS`, `telescope_delay_ms`, and
  `telescope_settle_ms` helpers and their native tests; repository search at
  the current head finds no consumer outside this obsolete interaction.
  Remove `collapsed` from observed attributes and remove the ordinary
  sync-disc telescope toggle. Task 4 adds a separate internal
  `BarState.compact_collapsed` state rather than reviving this public
  attribute.
- [ ] Render a `data-cell=more` button that is hidden in full mode and shown in
  compact mode. Set `compact` and `--_space-w` from the single `BarLayout`
  result; hide/show share and mode from that same result so CSS and action
  inventory cannot disagree.
- [ ] Update `apply_flip` to order full left as
  `sync · space · share · mode`, full right as
  `mode · share · space · sync`, compact left as
  `sync · space · [share] · more`, and compact right as
  `more · [share] · space · sync`. Reorder real nodes so visual,
  tab, and accessibility order continue to match.
- [ ] Make the responsive observer subtract `float_insets.left` and
  `float_insets.right` before calling `bar_layout`. Store both the
  `ResizeObserver` and its callback on `TonkFab` and disconnect/drop them in
  `disconnected_callback`; do not retain the current `callback.forget()` leak.
- [ ] On a layout transition, close a panel whose opener becomes hidden,
  restore a hoisted sub-stack, and focus the space cell if focus was inside
  the disappearing control. A repeated observer delivery of the same layout
  must make no DOM or focus change.
- [ ] Delete `tests/telescope.rs` only after the new browser test proves the
  full bar and expanded compact bar do not collapse from an ordinary disc tap
  and the compact layout remains reachable. Task 4 adds replacement coverage
  for the new deliberate compact-only collapse route.
- [ ] Run `nix develop . -c cargo test -p tonk-fab --lib` and
  `nix develop . -c test:web:debug -E 'package(tonk-fab)'`; expect success.

### Task 3: Route hidden actions through one vertical overflow stack

**Files:**

- Modify: `rust/tonk-fab/src/markup.rs:STACKS_HTML` and `STACKS_CSS`
- Modify: `rust/tonk-fab/src/bar.rs:open_panel`, `close`, and panel navigation
- Modify: `rust/tonk-fab/src/element.rs:attach_stack_verbs`
- Test: `rust/tonk-fab/tests/responsive_overflow.rs`

**Interfaces:**

- Consumes: `OpenPanel`, `Panel`, `Cell`, and the current `BarLayout` from
  Task 2.
- Produces: one `tonk-menu[data-for="overflow"]`; a share menu that can be
  anchored either to `Cell::Share` or `Cell::More`; `data-mi-back`,
  `data-overflow-share`, and `data-overflow-mode` row hooks.
- Menu width policy: space and visible-share stacks inherit their rung width;
  overflow and a share stack reached from overflow use
  `min(216px, usable viewport width)` and align their outer edge to the anchor.

- [ ] Extend the browser test at 375px: opening more shows `share ▸` then the
  appearance action; choosing share replaces the overflow stack in the same
  coordinates; `back ◂` restores overflow; only one canonical share menu
  exists in light DOM throughout.
- [ ] Add the complementary 390px assertion: because share is visible, the
  overflow menu omits the share row and contains only appearance; the visible
  share cell opens the same canonical share stack with no back row.
- [ ] Add mode assertions in both layouts. The desktop half-pill and compact
  overflow row must both update the host `mode`, call the existing app-wide
  theme path, update `aria-checked`/action copy, and close their panel.
- [ ] Run
  `nix develop . -c test:web:debug -E 'package(tonk-fab)'`; expect failure
  because there is no overflow stack or alternate share anchor.
- [ ] Generalize stack opening so panel identity and anchor identity are
  separate. Keep one visible stack at a time; do not clone or duplicate the
  member roster or copy-link state. Set the share back row visible only when
  `OpenPanel.return_to == Some(Panel::Overflow)`.
- [ ] Reuse the existing in-place disclosure machinery for the share roster
  and space switcher. Closing, resizing, dragging, or pressing Escape must
  restore every hoisted sub-stack before changing panel.
- [ ] Set stable `aria-controls` ids on the more/share cells, synchronize
  `aria-expanded`, focus the first actionable row on open, and return focus to
  the opener on close/back. Click-away retains the current composed-path
  behavior.
- [ ] Run `nix develop . -c cargo test -p tonk-fab --lib` and
  `nix develop . -c test:web:debug -E 'package(tonk-fab)'`; expect success.

### Task 4: Add an explicit compact-only collapse to the sync disc

**Files:**

- Modify: `rust/tonk-fab/src/markup.rs:BAR_CSS` and `STACKS_HTML`
- Modify: `rust/tonk-fab/src/bar.rs:BarState`, overflow row handling,
  `apply_responsive`, `apply_flip`, and `update`
- Modify: `rust/tonk-fab/src/element.rs:attach_drag`
- Test: `rust/tonk-fab/tests/responsive_overflow.rs`
- Test: `rust/tonk-fab/tests/drag_snap.rs`

**Interfaces:**

- Consumes: compact/full `BarLayout` from Task 1 and the overflow stack from
  Task 3.
- Produces:

  ```rust
  // New field on the existing BarState.
  pub compact_collapsed: bool;

  pub(crate) fn collapse_compact(this: &HtmlElement, state: &Shared);
  pub(crate) fn expand_compact(this: &HtmlElement, state: &Shared);
  ```

- DOM state: `.w.compact-collapsed` is internal presentation state. Do not
  restore the public `collapsed` attribute or `fabb-collapse` event removed in
  Task 2.
- Produces: `tonk-mi[data-overflow-collapse]` as the final compact overflow
  row, regardless of whether share is also overflowed.

- [ ] Add a 375px browser test asserting the final overflow order is
  `share ▸ · appearance · collapse`; add the 390px counterpart asserting
  `appearance · collapse`. Choosing collapse must close the stack, restore any
  hoisted sub-stack, focus the sync cell rather than the disappearing more
  cell, and leave exactly the 44px sync cell visible.
- [ ] Assert the collapsed disc's accessible name is
  `expand FABB · sync: <exact status> · drag to move`, while the expanded and
  full disc labels do not advertise collapse or expansion.
- [ ] Assert one collapsed-disc tap restores the same compact layout computed
  before collapse: 390px returns share, while 375px returns share to overflow.
  A disc tap while already expanded is inert.
- [ ] Resize a collapsed 375px parent to 500px and expect the full bar to
  appear automatically with `.compact-collapsed` removed. Resize back to
  375px and expect expanded compact, proving collapse is neither persisted
  nor revived by a later narrow layout.
- [ ] Extend `drag_snap.rs` with touch pointer events: a stationary tap on a
  collapsed disc expands it; 9px travel promotes to drag without expanding,
  suppresses the trailing click, preserves the collapsed atom through the
  drag, and emits exactly one `fabb-snap`.
- [ ] Run
  `nix develop . -c test:web:debug -E 'package(tonk-fab)'`; expect the new
  assertions to fail because overflow has no collapse action and Task 2 made
  expanded-disc taps inert.
- [ ] Append `data-overflow-collapse` after every conditional overflow row.
  `collapse_compact` no-ops outside compact layout; inside compact it commits
  a live rename, closes the current panel, restores sub-stacks, sets
  `compact_collapsed`, and applies `.compact-collapsed`.
- [ ] Make `.compact-collapsed .run` retract toward the sync cell at either
  edge without retaining a separate arrow or focusable hidden control. Use an
  interruptible `max-width 200ms var(--_ease)` transition plus an opacity fade
  that finishes within the same 200ms; name the transitioned properties
  explicitly and use no `transition: all`. When reduced motion is requested,
  settle immediately. Hidden cells must be removed from focus and hit testing
  for the whole collapsed interval.
- [ ] In the disc click path, call `expand_compact` only when
  `compact_collapsed` is true. Preserve the existing Option-click pause
  shortcut without adding mobile copy, status panels, or transaction changes.
- [ ] Make `apply_responsive` clear `compact_collapsed` before applying a full
  layout. Do not write a dock claim or any other persistence when collapsing
  or expanding.
- [ ] Run `nix develop . -c cargo test -p tonk-fab --lib` and
  `nix develop . -c test:web:debug -E 'package(tonk-fab)'`; expect success.

### Task 5: Apply touch geometry, disclosure motion, and viewport resilience

**Files:**

- Modify: `rust/tonk-fab/src/markup.rs:BAR_CSS`
- Modify: `rust/tonk-fab/src/menu.rs:propagate`
- Modify: `rust/tonk-fab/src/mi.rs:CSS` and observed attributes
- Modify: `rust/tonk-fab/src/logic.rs` for extracted viewport calculations
- Modify: `rust/tonk-fab/src/element.rs:attach_keyboard_lift`, viewport resize,
  and drag clamping
- Test: `rust/tonk-fab/src/logic.rs`
- Test: `rust/tonk-fab/tests/responsive_overflow.rs`
- Test: `rust/tonk-fab/tests/drag_snap.rs`

**Interfaces:**

- Produces:

  ```rust
  pub fn keyboard_lift_px(
      resting_bottom: f64,
      visual_offset_top: f64,
      visual_height: f64,
      gap_px: f64,
  ) -> f64;
  ```

- Compact propagation: a compact bar stamps `compact` on its slotted menus;
  `tonk-menu` passes it to direct `tonk-mi` rows, and `tonk-mi[compact] .row`
  uses `min-height:44px`. Full-mode rows remain 36px.

- [ ] Add native tests showing `keyboard_lift_px` returns zero when the resting
  bottom fits, returns `occlusion + 8` when covered, and is stable when called
  repeatedly with the same resting bottom.
- [ ] Add browser computed-style assertions: compact bar height, sync, and
  more targets are 44px; compact menu rows are at least 44px; full bar remains
  36px; the visible disc remains 14px; compact collapse settles at a 44px host
  width with no hidden focus target.
- [ ] Add computed-style assertions that disclosure transitions name only
  `opacity` and `transform`, use no `transition: all`, and disappear under
  `prefers-reduced-motion`. The closed menu must be non-focusable and
  non-clickable during its hidden state.
- [ ] Run `nix develop . -c cargo test -p tonk-fab --lib` and
  `nix develop . -c test:web:debug -E 'package(tonk-fab)'`; expect the new
  geometry and extracted-lift tests to fail.
- [ ] Raise only compact geometry to 44px. Preserve the full bar's 36px
  proportions and the FABB's existing border radius/material. Do not scale
  individual fused cells on press: doing so opens visible seams between
  rungs; retain the current ink wash as the pressed response.
- [ ] Replace `.mw`'s `display:none/block` switch with an interruptible
  opacity plus 4px translate transition using the existing `--_ease`, while
  governing hit testing with `visibility` and `pointer-events`. Reverse the
  translation when the panel opens upward. Disable duration under
  `prefers-reduced-motion`.
- [ ] Extract the keyboard-lift arithmetic and keep using the resting bottom,
  not the already transformed rectangle. On visual-viewport resize/scroll,
  update the lift and re-evaluate whether an open menu fits; never apply the
  lift mid-drag.
- [ ] On parent/visual-viewport resize and orientation change, recompute usable
  width, close or re-anchor stale panels, and clamp the resting bar to the
  resolved safe area. Preserve its stored nearest-corner fallback; responsive
  layout changes must not write a new dock claim.
- [ ] Run `nix develop . -c cargo test -p tonk-fab --lib` and
  `nix develop . -c test:web:debug -E 'package(tonk-fab)'`; expect success.

### Task 6: Verify the complete mobile slice and update the governing spec

**Files:**

- Modify: `plan/fabb-conformance.md` decisions, completed sequence, and known
  deltas
- Verify: all files above

**Interfaces:**

- Produces no new runtime interface. This task proves the integrated behavior
  in the component harness and the real sealed-guest product boundary.

- [ ] Update `plan/fabb-conformance.md` to remove telescope/fold/pan as current
  laws and record the replacement: full when 414px fits; otherwise compact,
  share opportunistically visible, appearance always overflowed, one vertical
  menu for hidden actions, and an explicit compact-only collapse row whose
  expand target is the 44px disc. Keep the historical reference decision
  distinguishable from the product's revised mobile design.
- [ ] Search for stale runtime and test references with
  `rg -n 'collapsed|folded|rfold|xopen|fabb-collapse|fabb-fold|telescope|strip_page' rust/tonk-fab plan/fabb-conformance.md`.
  Expect no legacy public `collapsed` attribute/event, fold state, pager, or
  desktop collapse. The only live collapse symbol must be the internal
  compact-only `compact_collapsed` path; historical prose must be explicitly
  labelled historical.
- [ ] Run `nix develop . -c cargo fmt --all -- --check` and
  `git diff --check`; expect success.
- [ ] Run `nix develop . -c cargo test -p tonk-fab --lib`; expect all pure and
  markup tests to pass.
- [ ] Run
  `nix develop . -c test:web:debug -E 'package(tonk-fab)'`; expect every FABB
  DOM, subscription, responsive, drag, and share test to pass in headless
  Chrome.
- [ ] Run `nix develop . -c test:native:debug` and
  `nix develop . -c test:web:debug`; expect the relevant broader suites to
  pass. If localhost, Chrome, or Nix cache access is denied by the sandbox,
  rerun the unchanged command with normal host permissions before classifying
  it as a product failure.
- [ ] Run `nix flake check` and `nix develop . -c build:web`; expect the lint
  gate and production web bundle to pass. Ensure the new browser test file is
  tracked before the flake snapshot build.
- [ ] Start `nix develop . -c dev:web` and make one clean live reproduction in
  the actual sealed FABB guest at viewport widths 320, 375, 390, 768, and
  1024px, in both left and right docks. For each width record: visible cell
  order, overflow row order, computed target sizes, stack direction, share
  back navigation, compact collapse/expand, full-width forced expansion,
  click-away/Escape, collapsed-disc drag snap, orientation resize,
  software-keyboard lift, and whether any console error or failed request
  occurred.
- [ ] Repeat the 375px live check with `prefers-reduced-motion: reduce` and a
  simulated bottom safe area. Confirm no disclosure animation runs, the bar
  clears the inset, and an open upward menu remains inside the visual viewport.
- [ ] Re-read the final diff and confirm there is one source of responsive
  truth (`bar_layout`), one canonical menu per action, no localStorage or new
  dependency, no change to space/share authority, and no claim that a source
  check alone proves deployed mobile behavior.

## Explicitly deferred

- Proposal/history `changes` actions and alert routing. There is still no
  product feature to drive them.
- Account switching or account settings in the FABB. Those remain Hub/account
  chrome.
- Sync details and mobile pause/resume. The disc keeps its existing visual and
  accessible status, and Option-click remains unchanged; any discoverable
  detailed controls belong in a broader space-status/settings design.
- Persisting the appearance override. The sealed guest still has an opaque
  origin; persistence needs the separate page-effect or profile-claim design
  already recorded in `plan/fabb-conformance.md`.
- A bottom sheet, horizontal action paging, gesture-only shortcuts, or more
  than one submenu level. The approved interaction is one anchored vertical
  overflow stack with in-place replacement.
- Changing share semantics, invite authority, membership promotion, docking
  persistence, or the FABB's default bottom-right seat.
- Reworking desktop dimensions or visual material beyond removing the fold
  cell. The 44px geometry is compact-mode-only.
