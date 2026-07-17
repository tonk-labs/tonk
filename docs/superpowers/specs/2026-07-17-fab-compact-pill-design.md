# FAB compact pill — design

2026-07-17

## Problem

The FAB (`<tonk-fab>`, `rust/tonk-fab`) is designed for desktop and breaks down elsewhere:

- **Low resolution**: the bar is fixed-width with pixel sizing and no responsive
  layout. Docked to a corner with a 16px inset, a bar wider than the viewport
  overflows it, so some segments cannot be reached.
- **Mobile**: dragging uses pointer events but has no `touch-action` handling,
  defers pointer capture until a 4px threshold, and only the small circle cap is
  draggable. The browser's own touch gestures win the race, making finger drag
  unreliable and cross-screen drags nearly impossible. There is also no viewport
  clamping mid-drag.

Inspiration: the iOS text-selection popup — a pill with a fixed chevron that
pages horizontally on swipe and expands into a vertical menu showing everything.

## Decisions

- **Adaptive, not universal**: wide viewports keep the current expanded bar and
  telescope unchanged. The compact pill only activates when the bar would not
  fit.
- **Free drag stays** on all screen sizes, repaired for touch (not reduced to
  dock-only or removed).
- **Overflow mechanics** (revised 2026-07-17 after device testing): swipe to
  page horizontally + chevron-tap advances one page, wrapping to the start at
  the end. The vertical menu of the first design was cut — buggy on device
  and it added little over the pager. The chevron is nub-sized (a rounded
  terminator that meshes with the pill, with an invisible >=44px coarse-pointer
  hit area) and retracts with the strip when the bar collapses.
- **Page 1 contents**: sync circle (fixed cap) + space name/switcher. Share and
  profile name go to page 2.
- **Compact dropdowns float**: the switcher and roster open over the bar in
  compact mode (`position: fixed`; the bar's drop-shadow filter makes `.fab`
  the containing block, lifting the menu out of the scroll strip's clip),
  spanning the full bar width. Same `is-open` mechanics as desktop.
- **Strip is horizontal-only**: `touch-action: pan-x` + `overscroll-behavior:
  contain`, so a vertical finger on the strip cannot pan the overlay page and
  ride the pill off-screen.
- **Async content re-measures the fit**: the names arrive from live
  subscriptions after connect, so a MutationObserver on the bar (child-list +
  text) and a `document.fonts.ready` pass re-run the compact-fit evaluation —
  without them a phone sits in wide mode measuring the empty bar.
- **Architecture**: scroll-snap pager + CSS restack (approach A). One DOM, no
  new child components, no markup duplication; gesture handling is native
  browser scrolling. Rejected: a Rust-driven pager (re-implements browser
  scrolling inside already-subtle pointer handlers) and a separate
  `<tonk-fab-mini>` element (two components sharing seven children and one
  persistence model).

## Compact mode

A new `fab--compact` class, decided by measurement, not device detection:

- `logic.rs` gains a pure function `is_compact(expanded_width, viewport_width)`
  returning true when the fully expanded bar plus the two 16px insets exceeds
  the viewport width. `expanded_width` comes from the tile widths already
  measured for the telescope.
- Evaluated on connect, on guest-window resize, and after content settles
  (invite links and long names widen the bar).
- Crossing the threshold swaps the class; any open menus close on a mode switch
  so dropdown geometry is never stranded.

A phone and a small desktop window behave identically.

## Compact layout

Left to right:

1. **Left cap — sync circle**, unchanged. Still the drag handle and the
   telescope toggle. Telescope works in compact mode: the pill can collapse to
   just the circle.
2. **Middle — scroll strip.** The existing segments sit inside a horizontal
   `overflow-x` container with CSS `scroll-snap-type: x mandatory`. `markup.rs`
   wraps segments in page groups:
   - Page 1: space name + switcher (`.fab__repo`).
   - Page 2: share (`.fab__share`), then profile name (`.fab__account`).
   Swiping is native scroll (momentum, rubber-banding); snap points land on
   page boundaries. No page-indicator dots — the chevron plus a slight peek of
   the next segment's edge signal there is more. An open dropdown closes when a
   swipe starts.
3. **Right cap — chevron** (revised): a nub-sized paging button. Tapping
   advances the strip one page (smooth scroll), wrapping to the start at the
   end — `strip_page_target` in `logic.rs` owns the arithmetic. The
   dropdowns float over the bar (see Decisions); `.fab__scrim` handles
   tap-away dismissal as on desktop.

Wide viewports see zero change.

## Drag fixes (all modes)

- **`touch-action: none` on the circle cap** so the browser stops competing for
  touch gestures. Scoped to the cap; touch scrolling elsewhere is unaffected.
- **Immediate pointer capture for touch pointers** at `pointerdown`. Mouse keeps
  the current deferred-capture behavior (capture at the drag threshold) so a
  stationary press still clicks; capture does not break tap-to-toggle on touch.
- **Bigger touch target**: an invisible pseudo-element pads the cap's hit area
  to ≥44px without visual change. Drag threshold for touch pointers rises to
  ≈8px so taps don't misregister as drags.
- **Viewport clamping**: a pure `logic.rs` function clamps `left/top` during
  drag so the bar can never leave the viewport, in any mode.
- **Drag/paging exclusivity**: while `.dragging` is set, the scroll strip locks
  (`overflow: hidden`).

Corner docking, mirror flip, and the durable `xyz.tonk.fab/dock` claim are
untouched; compact mode docks and persists identically. Page and menu state are
ephemeral (not persisted).

## Error handling / edge cases

- `prefers-reduced-motion` continues to suppress transitions, including the
  menu restack.
- Mode re-evaluation after settle handles content growth (e.g. minted invite
  link) pushing a previously fitting bar past the viewport.
- Stale-press guard (`buttons == 0` on move) and pointercancel handling remain.

## Testing

- Native tests for the new pure functions in `logic.rs` (`is_compact`,
  `clamp_position`), alongside the existing dock and telescope-timing tests.
- wasm tests (safaridriver or chromedriver locally) for: compact activation
  on narrow viewports, compact segment taps opening the floating dropdowns,
  collapse dismissing dropdowns and dropping `fab--settled`, and
  computed-style pins with the real stylesheet (chevron hidden in wide mode,
  chevron tile clamped away when collapsed).
- Manual pass for feel (swipe momentum, drag responsiveness) in responsive mode
  and on a phone.

## Non-goals

- No change to wide-viewport behavior, dock persistence, or child elements'
  internals.
- No page-indicator dots, no chevron-tap paging.
- No device detection; all adaptation is measurement-driven.
