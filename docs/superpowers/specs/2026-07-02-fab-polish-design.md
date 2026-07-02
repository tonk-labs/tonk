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
- The telescope's settled state already leaves tiles at `max-width: none`, so
  a widened segment reflows the bar along its docked direction — the same
  behavior as the growing invite link.

## 4. Name caps — no change

Profile and repo names stay clamped at `16ch` with ellipsis. The width
equalization above absorbs the rest.

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

## Testing

- `cargo clippy --all -- -D warnings` (native lint gate).
- Existing tonk-fab unit tests in `logic.rs` unaffected.
- Live check in tonk-ui: fold/unfold; both dropdowns open with equal widths at
  top and bottom docks; rung eases wider when the menu is wider than the label;
  menu rows render in Plex Sans Condensed; double-click does nothing.
