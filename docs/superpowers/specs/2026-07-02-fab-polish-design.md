# FAB polish — font, dropdown geometry, pause-sync removal

Five UI/UX fixes to the FAB (the floating segmented control bar). Everything
lives in two places: the `tonk:profile/fab` view (markup + CSS) in
`rust/tonk-core/assets/library/profile.yaml`, and gesture logic in
`rust/tonk-fab/src/element.rs`.

## 1. IBM Plex Sans Condensed for all FAB text

- Set `font-family: "IBM Plex Sans Condensed", var(--wa-font-family-heading, sans-serif)`
  on `.fab` (replacing the `--wa-font-family-body` / Space Grotesk stack).
- Remove the `font-family: var(--wa-font-family-code, monospace)` overrides on
  `.fab__menu-item` and `.fab__menu-item--action` so dropdown rows inherit the
  same face.
- No new assets: the woff2s (400/500/600/700) already ship in
  `rust/tonk-ui/assets/fonts/` and the portal bridge inlines `/fonts/*.woff2`
  into the sealed iframe as data URLs.
- **Amended (2026-07-02, live review):** text is UNIFORM across the control,
  matching the wireframe — one size, weight, and ink everywhere. Drop the
  `font-weight: 500` on the profile name, the `font-size: 13px` on menu rows
  and `14px` on the menu glyph (all inherit the bar's 16px), and the 70%-ink
  dim on the "all repos" / "+ new" action rows (full `--fab-ink`).

## 2. 7px gaps around dropdowns

- `.fab__menu { gap: 7px }` (between rows; was 6px).
- Dock-direction offsets `.fab-dock-top .fab__menu { margin-top: 7px }` and
  `.fab-dock-bottom .fab__menu { margin-bottom: 7px }` (was 6px).
- The invisible hover-bridge `::before` blocks grow to match (7px tall,
  offset −7px) so the pointer can still cross the gap without the hover
  dropping.

## 3. Equal rung/dropdown widths (both menus)

Applies to the repo segment + space switcher AND the share segment + member
roster.

- CSS: `.fab__menu { width: 100% }` (drop `width: max-content`,
  `min-width: 100%`, and the 280px `max-width` cap) so an open menu is exactly
  its segment's width; rows keep `overflow: hidden; text-overflow: ellipsis`
  and clip inside it.
- Rust (`toggle_menu` in element.rs): when opening, measure the menu's natural
  max-content width (momentarily unclamp, read, restore — same trick as
  `measure_tile_widths`). If it exceeds the segment's current width, stamp an
  inline `min-width` on the segment so the rung widens and whitespace fills
  around the label. Clear the inline `min-width` when the menu closes (or when
  the other menu's toggle closes it), so the resting bar shrink-wraps as today.
- Jumpiness guard: `transition: min-width 0.2s ease` on `.fab__repo` and
  `.fab__share` so the widening eases open instead of snapping.
  `prefers-reduced-motion: reduce` disables it alongside the existing telescope
  exception. If it still feels jumpy in practice, the fallback design is a
  MutationObserver per menu that keeps the rung at the widest-row width
  permanently (no on-open change).
- **Amended (2026-07-02, live review):** clear-on-close proved jumpy — segment
  widths visibly changed depending on which dropdown was open. The equalized
  `min-width` now RATCHETS: it is stamped on open when the menu's natural width
  exceeds the segment's current rendered width, and is never cleared — a
  column's width only grows, and only when a wider element enters (re-measured
  on each open). The `clear_menu_width` helper is removed.
- The telescope's settled state already leaves tiles at `max-width: none`, so
  a widened segment reflows the bar along its docked direction — the same
  behavior as the growing invite link.

## 4. Name caps — no change

Profile and repo names stay clamped at `16ch` with ellipsis. The width
equalization above absorbs the rest.

**Amended (2026-07-02, live review):** the caps differ per segment — profile
name clamps at `15ch`, repo name at `24ch` (both the static label and its
inline editable).

## 5. Remove pause-sync (for now)

The double-click-to-pause gesture is unintuitive; remove it entirely. The sync
circle remains a status indicator + fold toggle + drag handle.

- element.rs: delete the `dblclick` listener, `trigger_pause_toggle`,
  `is_sync_paused`, `open_pause_dialog`, `submit_pause_form`, and the
  `detail() <= 1` guard in the click handler (no longer needed once no
  dblclick exists). Update the module docs.
- profile.yaml: delete the `#fab-pause-sync` dialog, the
  `tonk:view/fab-pause` display mount, and comment references.
- core.yaml: remove the `tonk:view/fab-pause` view instance (and its view-kind
  concept) if nothing else references them; the `tonk:pause-sync` command
  itself stays — it is space-branch machinery a future affordance rewires.

## 6. Drag robustness (amendment, 2026-07-02 live review)

Fast flicks lose the FAB: the `pointermove`/`pointerup` listeners sit on the
element and capture is only taken after a 4px move event reaches the element,
so a fast pointer outruns the FAB before any move event fires, and the
`pointerup` lands outside it — `fabPressing` is never cleared and a later
hover resumes a phantom drag.

- Move the `pointermove` / `pointerup` listeners (plus a new `pointercancel`)
  to the guest WINDOW; `pointerdown` stays on the element (circle only).
- Stale-press guard: a `pointermove` with `fabPressing` set but
  `e.buttons() == 0` means the release was lost — finish the drag (snap to the
  nearest dock if it had moved, else just clear the press).
- `pointercancel` finishes the drag the same way.
- The threshold-time `set_pointer_capture` stays (it keeps events flowing when
  the pointer leaves the browser window; captured events still bubble to the
  window listeners).

## 7. Right-dock mirroring (amendment, 2026-07-02 live review)

When docked right, the whole control mirrors along the x-axis. The
`row-reverse` + cap-radius CSS for this already exists but is DEAD — keyed on
`.fab--dock-right`, a class nothing sets (element.rs stamps `fab-dock-right`
on the `<tonk-fab>` host). Re-key those three rules to `.fab-dock-right .fab`
descendants, and complete the mirror: the share roster (now the bar's left
end) anchors left, action rows justify flex-start, member cards align left,
and telescope tiles anchor content toward the circle
(`justify-content: flex-start`) so the unfold reads from the circle outward.

## 8. Preloaded menu widths (amendment, 2026-07-02 live review)

Opening a dropdown should never change the bar's widths — the ratchet stamps
should already be there. On connect, measure each menu's natural width even
while closed (inline `display: flex; visibility: hidden; width: max-content`,
read, restore — synchronous, so nothing paints) and stamp the ratcheted
segment `min-width`. Re-measure and re-ratchet when a menu's content mutates
(one MutationObserver per menu — rows render asynchronously) and once when
`document.fonts.ready` resolves (a font swap changes metrics but fires no
mutation). The on-open equalize stays as a cheap no-op fallback.

**Corrected (final review):** measurements taken before the font loads use the
FALLBACK face, which is typically WIDER than condensed Plex — they
over-report, and a never-shrink ratchet would bake the over-wide stamp in for
the session. The `fonts.ready` pass is therefore AUTHORITATIVE, not ratcheted:
it restamps each segment from a fresh real-metrics measurement in both
directions (the correction rides the font swap's own reflow, eased by the
min-width transition). All other passes (connect, mutations, open) stay
ratcheted; after the fonts land every measurement uses real metrics, so
over-stamps cannot recur. Hardening from the same review: `disconnected_callback`
clears the press flags (`fabPressing`/`fabMoved`) so a mid-press clone remount
cannot let a stale window listener persist a phantom dock.

## 9. Live drag mirroring + drag-time menu hygiene (amendment, 2026-07-02 live review)

Two drag-time defects, one root cause: the mirror and the menus' open-direction
are keyed on the `fab-dock-*` classes, which a drag REMOVES (they pin corners
and would fight the inline drag position) — so the mirror can only flip on
drop, and an open menu loses its `top: 100%` anchor and falls back to its
static position, floating mid-bar.

- Split the mirror into its own host class, `fab-mirror`, carrying ONLY the
  visual flips (row-reverse, cap radii, menu horizontal anchors, row
  alignment, tele justify) — the `fab-dock-*` classes keep only positioning
  and the menus' vertical open-direction.
- element.rs drives `fab-mirror` continuously: from the horizontal dock at
  rest (apply_dock), and during a drag from the viewport x-midline — the bar
  mirrors the moment it crosses the middle.
- The drop corner is decided by the same signal the mirror uses, so the live
  mirror is always a truthful preview of the snap (was: pointer release
  point).
- **Corrected (task review):** the drag-time signal is the HANDLE'S (circle
  cap's) center, not the bar's. The pointer-compensation below holds the
  handle fixed across a flip, so a bar-center predicate would be shifted back
  across the midline by its own compensation (≈ a bar-width per flip) and
  oscillate; the handle is flip-invariant by construction, so no hysteresis
  is needed. `finish_drag` docks by the handle's center on both axes — the
  corner you dock is where the handle is.
- A drag promotion closes any open menu (`is-open` dropped; the ratcheted
  widths stay). Dragging with a dropdown open is not a state the chrome
  supports.
- **Amended (2026-07-02, live review):** the flip must keep the pointer on the
  part of the bar it grabbed. Row-reversing inside a fixed box teleports the
  circle to the bar's other end; instead, on a mid-drag mirror toggle the bar
  SHIFTS by the grab handle's measured displacement (circle rect before vs
  after the class flip), and the shift is folded into the drag's stored
  start-left so subsequent pointer deltas don't undo it. Net effect: the
  circle — and the pointer holding it — stays put while the bar swings around
  it to the other side.

## Testing

- `cargo clippy --all -- -D warnings` (native lint gate).
- Existing tonk-fab unit tests in `logic.rs` unaffected.
- Live check in tonk-ui: fold/unfold; both dropdowns open with equal widths at
  top and bottom docks; rung eases wider when the menu is wider than the label;
  menu rows render in Plex Sans Condensed; double-click does nothing.
