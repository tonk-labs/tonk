# FAB Polish Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Apply five FAB UI/UX fixes — IBM Plex Sans Condensed for all FAB text, 7px dropdown gaps, rung/dropdown width equalization on open, and removal of the double-click pause-sync gesture.

**Architecture:** The FAB is the `<tonk-fab>` custom element (`rust/tonk-fab/src/element.rs`, wasm-only, with pure geometry in `logic.rs`, native-tested). All FAB markup + CSS live in the `tonk:profile/fab` view inside `rust/tonk-core/assets/library/profile.yaml`; the pause-sync form/view kind live in `rust/tonk-core/assets/library/core.yaml`. Spec: `docs/superpowers/specs/2026-07-02-fab-polish-design.md`.

**Tech Stack:** Rust (wasm-bindgen / web-sys), tonk notation YAML seed libraries, CSS.

## Global Constraints

- Repo is jj/git colocated: commit with `jj commit -m "..."` (never `git add`; the working copy IS the change). After `jj commit` you are on a fresh empty change.
- Conventional commit subjects: `type(scope): subject`, imperative, lowercase, no trailing period.
- No emojis anywhere. No "Phase X" / "per the spec" references in code or comments.
- Comment style: match the existing dense `//!`/`///`/inline-comment voice in these files; comments explain constraints, not narration.
- Lint gate: `cargo clippy --all -- -D warnings` (run from `/Users/jackdouglas/tonk/tonk/rust`). element.rs is wasm-gated, so also check `cargo clippy -p tonk-fab --target wasm32-unknown-unknown -- -D warnings`.
- Seed yaml gate: `cargo test -p tonk-worker --test standard_library` (native; parses + analyzes + lowers both yamls).
- All paths below are relative to `/Users/jackdouglas/tonk/tonk` unless absolute. Cargo commands run from `/Users/jackdouglas/tonk/tonk/rust`.

---

### Task 1: IBM Plex Sans Condensed + 7px gaps (CSS only)

**Files:**
- Modify: `rust/tonk-core/assets/library/profile.yaml` (the `<style>` block of the `id:tonk:profile/fab/view` display template, roughly lines 459–973)
- Test: `rust/tonk-worker/tests/standard_library.rs` (existing; just run it)

**Interfaces:**
- Consumes: nothing from other tasks.
- Produces: nothing other tasks reference by name; purely presentational CSS.

- [ ] **Step 1: Set the FAB font to IBM Plex Sans Condensed**

In `profile.yaml`, in the `.fab` rule, replace the line:

```css
          font-family: var(--wa-font-family-body, sans-serif);
```

with:

```css
          /* The FAB is set in IBM Plex Sans Condensed throughout — the app's
             display face (already shipped + inlined into the sealed iframe by
             the portal bridge), condensed so the bar stays tight. */
          font-family: "IBM Plex Sans Condensed", var(--wa-font-family-heading, sans-serif);
```

- [ ] **Step 2: Drop the monospace overrides on menu rows**

In the `.fab__menu-item` rule, delete the line:

```css
          font-family: var(--wa-font-family-code, monospace);
```

In the `.fab__menu-item--action` rule, delete the line:

```css
          font-family: var(--wa-font-family-code, monospace);
```

(Each rule keeps its other properties; `--action` keeps `font: inherit`, which now resolves to the Plex stack set on `.fab`.)

- [ ] **Step 3: Bump dropdown gaps from 6px to 7px**

In the `.fab__menu` rule change `gap: 6px;` to `gap: 7px;`.

Change the dock-direction offsets:

```css
        .fab-dock-top .fab__menu { top: 100%; margin-top: 7px; }
        .fab-dock-bottom .fab__menu { bottom: 100%; margin-bottom: 7px; }
```

Change BOTH hover-bridge blocks (the `::before` rules that let the pointer cross the gap) from 6px to 7px:

```css
        .fab-dock-top .fab__menu::before {
          content: "";
          position: absolute;
          left: 0;
          right: 0;
          top: -7px;
          height: 7px;
        }
        .fab-dock-bottom .fab__menu::before {
          content: "";
          position: absolute;
          left: 0;
          right: 0;
          bottom: -7px;
          height: 7px;
        }
```

Update the two comments that name the old value: the hover-bridge comment ("Invisible hover-bridge over the 6px gap …" → "…the 7px gap…") and the roster comment above `.fab__menu > tonk-display` if it mentions the gap.

- [ ] **Step 4: Validate the seed yaml still lowers**

Run: `cargo test -p tonk-worker --test standard_library`
Expected: PASS (both `standard library (core.yaml)` and `profile library (profile.yaml)` lower).

- [ ] **Step 5: Commit**

```bash
jj commit -m "feat(fab): set IBM Plex Sans Condensed and 7px dropdown gaps"
```

---

### Task 2: Pure width-equalization helper in logic.rs (TDD)

**Files:**
- Modify: `rust/tonk-fab/src/logic.rs`

**Interfaces:**
- Consumes: nothing.
- Produces: `pub fn menu_min_width(menu_natural: f64, segment: f64) -> Option<f64>` — Task 3's `element.rs` imports this from `crate::logic`.

- [ ] **Step 1: Write the failing tests**

Append to `logic.rs` (alongside the existing `mod telescope` etc. — this file uses plain `#[test]` with BDD-style names; keep that):

```rust
#[cfg(test)]
mod menu {
    use super::*;

    #[test]
    fn a_wider_menu_widens_the_segment() {
        // Fractional natural widths round UP so the stamped min-width never
        // undershoots the menu by a subpixel.
        assert_eq!(menu_min_width(220.4, 120.0), Some(221.0));
    }

    #[test]
    fn a_narrower_or_equal_menu_leaves_the_segment_alone() {
        assert_eq!(menu_min_width(80.0, 120.0), None);
        assert_eq!(menu_min_width(120.0, 120.0), None);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p tonk-fab menu`
Expected: FAIL to compile — `menu_min_width` not found.

- [ ] **Step 3: Implement the helper**

Add to `logic.rs` (near `telescope_settle_ms`, before the test modules):

```rust
/// The inline `min-width` (px) to stamp on a bar segment when its dropdown
/// opens: the menu's natural (max-content) width when that EXCEEDS the
/// segment, so the rung widens — whitespace filling around its label — and
/// the menu (styled `width: 100%`) lands exactly as wide as the rung. `None`
/// when the segment is already at least as wide (the menu's `width: 100%`
/// alone matches them). Only ever widens; a menu narrower than its segment
/// never shrinks the bar.
pub fn menu_min_width(menu_natural: f64, segment: f64) -> Option<f64> {
    (menu_natural > segment).then(|| menu_natural.ceil())
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p tonk-fab`
Expected: PASS (new `menu` module plus all existing dock/telescope/geometry/persist tests).

- [ ] **Step 5: Commit**

```bash
jj commit -m "feat(fab): add menu_min_width equalization helper"
```

---

### Task 3: Equalize rung/dropdown widths on open

**Files:**
- Modify: `rust/tonk-fab/src/element.rs` (`toggle_menu`, imports)
- Modify: `rust/tonk-core/assets/library/profile.yaml` (`.fab__menu` sizing + segment transition)

**Interfaces:**
- Consumes: `crate::logic::menu_min_width` from Task 2.
- Produces: nothing later tasks reference.

- [ ] **Step 1: Make the menu exactly the segment's width in CSS**

In `profile.yaml`, in the `.fab__menu` rule, replace these three lines:

```css
          min-width: 100%;
          width: max-content;
          max-width: 280px;
```

with:

```css
          width: 100%;
```

and replace the comment above them (the block starting "At LEAST as wide as the trigger…") with:

```css
          /* Exactly as wide as the trigger segment. When the menu's natural
             width exceeds the segment's, element.rs stamps a matching inline
             `min-width` on the segment as the menu opens (cleared on close),
             so rung and dropdown always read as one column; rows ellipsise
             within it. Anchored at `left: 0`; the share roster re-anchors
             right (`.fab__share-menu`). */
```

- [ ] **Step 2: Ease the widening**

Add after the `.fab__space` rule block (or anywhere adjacent to the segment rules) in the same `<style>`:

```css
        /* Ease the equalize-on-open widening (the inline `min-width`
           element.rs stamps when a dropdown opens) so the rung unfolds
           rather than snaps; background keeps its existing hover fade. */
        .fab__repo, .fab__share { transition: background 0.15s ease, min-width 0.2s ease; }
```

And inside the existing `@media (prefers-reduced-motion: reduce)` block add:

```css
          .fab__repo, .fab__share { transition: background 0.15s ease; }
```

- [ ] **Step 3: Stamp / clear the segment min-width in element.rs**

In `element.rs`, extend the `crate::logic` import to include `menu_min_width`:

```rust
use crate::logic::{
    DOCK_CLASSES, Dock, dock_claim_json, menu_min_width, nearest_dock, telescope_delay_ms,
    telescope_settle_ms,
};
```

Replace `toggle_menu` with:

```rust
/// Open (or close) the dropdown owned by `seg` by toggling its `is-open` class,
/// closing the other menu (matched by `other_sel`) so only one is open at a time.
/// The open-direction is CSS, keyed off the FAB's `fab-dock-*` class.
///
/// On open the segment is widened (an eased inline `min-width`) to the menu's
/// natural width when the menu is the wider of the two — the stylesheet's
/// `width: 100%` then makes menu and rung exactly equal. The inline
/// `min-width` is cleared on close so the resting bar shrink-wraps.
fn toggle_menu(element: &HtmlElement, seg: &Element, other_sel: &str) {
    if let Some(other) = element.query_selector(other_sel).ok().flatten() {
        other.class_list().remove_1("is-open").ok();
        clear_menu_width(&other);
    }
    let opening = !seg.class_list().contains("is-open");
    seg.class_list().toggle_with_force("is-open", opening).ok();
    if opening {
        equalize_menu_width(seg);
    } else {
        clear_menu_width(seg);
    }
}

/// Measure the just-opened menu's natural (max-content) width — momentarily
/// overriding the stylesheet's `width: 100%`, reading the box, restoring —
/// and stamp the segment's inline `min-width` when the menu is wider. Runs
/// after `is-open` lands (the menu must be laid out to measure) but within
/// the same task, so nothing paints at the unmeasured width.
fn equalize_menu_width(seg: &Element) {
    let Some(menu) = seg.query_selector(".fab__menu").ok().flatten() else {
        return;
    };
    let style = menu.unchecked_ref::<HtmlElement>().style();
    let _ = style.set_property("width", "max-content");
    let natural = menu.get_bounding_client_rect().width();
    let _ = style.remove_property("width");
    let segment = seg.get_bounding_client_rect().width();
    if let Some(min_width) = menu_min_width(natural, segment) {
        let _ = seg
            .unchecked_ref::<HtmlElement>()
            .style()
            .set_property("min-width", &format!("{min_width}px"));
    }
}

/// Drop the equalized inline `min-width` so the closed segment shrink-wraps
/// its label again (the `min-width` transition eases it back).
fn clear_menu_width(seg: &Element) {
    let _ = seg
        .unchecked_ref::<HtmlElement>()
        .style()
        .remove_property("min-width");
}
```

- [ ] **Step 4: Verify both targets compile clean**

Run: `cargo clippy -p tonk-fab --target wasm32-unknown-unknown -- -D warnings`
Expected: clean.
Run: `cargo test -p tonk-fab && cargo test -p tonk-worker --test standard_library`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
jj commit -m "feat(fab): equalize rung and dropdown widths on menu open"
```

---

### Task 4: Remove the double-click pause-sync gesture

**Files:**
- Modify: `rust/tonk-fab/src/element.rs` (module docs, `attach_gestures`, delete four functions, `disconnected_callback`)
- Modify: `rust/tonk-core/assets/library/profile.yaml` (delete pause dialog + mount)
- Modify: `rust/tonk-core/assets/library/core.yaml` (delete `view/fab-pause` concept + instance, update the `tonk:pause-sync` comment)

**Interfaces:**
- Consumes: nothing from other tasks.
- Produces: nothing. The `tonk:pause-sync` command and its worker handler stay untouched (space-branch machinery for a future affordance).

- [ ] **Step 1: Strip the gesture from element.rs**

In the module doc (`//!` block at the top), replace the telescope bullet's double-click sentence — delete "A DOUBLE click toggles pause/resume of sync (the circle is the pause switch, matching the control-panel wireframe)." so the bullet ends after describing the single-click toggle.

In `attach_gestures`:
- Update the doc comment: drop the "`dblclick` pauses/resumes sync" wording from the CIRCLE bullet (it becomes "- CIRCLE cap: `click` folds/expands the bar.") and any remaining reference to a double-click pause. Keep the note that the name/spot editables edit on their own native `dblclick` (that behavior is editable.rs's, unchanged).
- In the click closure, replace the `detail() <= 1`-guarded branch:

```rust
        if t.closest(".fab__cap-l").ok().flatten().is_some() {
            toggle_telescope(&el_click);
        } else if t
```

(the guard existed only to cooperate with the deleted `dblclick` handler).
- Delete the `el_dbl` binding, the whole `on_dbl` closure, the `dblclick` `add_event_listener_with_callback` call, and `on_dbl.forget()`.

Delete these four functions and their doc comments entirely: `trigger_pause_toggle`, `is_sync_paused`, `open_pause_dialog`, `submit_pause_form`.

In `disconnected_callback`, shrink the timer-key loop to the only key still written (`settleTimer`; `tapTimer`/`editTimer` were never set since the native-gesture rewrite):

```rust
        if let Some(id_str) = this.dataset().get("settleTimer") {
            if let Ok(id) = id_str.parse::<i32>() {
                clear_timeout(id);
            }
            this.dataset().delete("settleTimer");
        }
```

If `Promise` or `Function` imports become unused after the deletions, remove them (clippy `-D warnings` will say; `Function` is still used by `persist_dock`/`restore_position`, `Promise` by `restore_position` — expect both to stay).

- [ ] **Step 2: Strip the dialog + mount from profile.yaml**

Inside the `id:tonk:profile/fab/view` display template, delete the entire block from the comment `<!-- Pause-sync confirm dialog + form — opened by DOUBLE-CLICKING the FAB circle …>` through the closing `</wa-dialog>` of `<wa-dialog id="fab-pause-sync" …>` — that is BOTH the `<tonk-host><tonk-repository …><tonk-display model="tonk:repository" view="tonk:view/fab-pause">…</tonk-host>` mount and the dialog element.

- [ ] **Step 3: Strip the view kind + form from core.yaml**

Delete two blocks (each with its leading comment):
- `concept!: &view/fab-pause` (the "A named view kind for the FAB's pause-sync form…" block).
- `view/fab-pause!:` / `this: id:tonk:repository/fab-pause` (the "The FAB's pause-sync FORM, authored on the SPACE branch…" block, ending at the `display:` line with the `<form id="fab-pause-sync-form" …></form>` template).

Update the comment above `command!: &tonk/pause-sync` — replace the first two sentences ("Toggle auto-sync for the space in scope. The sync chip submits this (via the confirm dialog when pausing, directly via the resume overlay when paused).") with:

```yaml
# Toggle auto-sync for the space in scope. No chrome currently submits this —
# the FAB's double-click pause gesture was removed as unintuitive — but the
# command and its worker handler stay for a future, visible affordance.
```

Leave the `tonk:invite` marker comment that mentions `tonk:pause-sync` (the command still exists, so the decode-disambiguation note stays true).

- [ ] **Step 4: Verify**

Run: `cargo test -p tonk-worker --test standard_library`
Expected: PASS (both yamls still lower).
Run: `cargo clippy -p tonk-fab --target wasm32-unknown-unknown -- -D warnings`
Expected: clean (catches any now-unused imports).
Run: `cargo clippy --all -- -D warnings`
Expected: clean.

- [ ] **Step 5: Commit**

```bash
jj commit -m "feat(fab): remove double-click pause-sync gesture"
```

---

### Task 5: Live verification

**Files:** none (verification only).

**Interfaces:** consumes the built app; produces a pass/fail report against the checklist.

- [ ] **Step 1: Run the app and exercise the FAB**

Launch tonk-ui (the `run` skill covers this; otherwise `trunk serve` from `rust/tonk-ui` per `Trunk.toml`). Seed yamls apply at repo creation, so use a FRESH profile (fresh browser profile / cleared site data) so the edited profile.yaml + core.yaml actually seed.

Checklist, in a space (`/space/<id>`):
- All FAB text — name, repo, share, menu rows — renders in IBM Plex Sans Condensed (menu rows no longer monospace).
- Dropdown rows sit 7px apart and 7px off the bar; hover can travel from segment into the menu without it closing.
- Open the space switcher and the share roster at a top dock and a bottom dock: the open menu is exactly the segment's width; when the menu's content is wider, the rung eases wider (0.2s) and snaps nothing; closing eases it back.
- Double-clicking the sync circle does nothing (no dialog, no fold-flicker); single click still folds/unfolds; drag still docks to corners.

- [ ] **Step 2: Report**

State plainly what passed and anything that didn't (with what was observed). If the widening still reads jumpy in practice, note it — the spec's fallback is the MutationObserver always-equal variant, a separate follow-up.

---

### Task 6: Ratchet segment widths + per-segment name caps (live-review amendment)

Live review found segment widths visibly changing depending on which dropdown
was open — the clear-on-close behavior from Task 3. Amendment (spec §3/§4
amendments, 2026-07-02): the equalized `min-width` ratchets (never cleared;
only grows, re-measured on each open), and the name caps become 15ch (profile)
/ 24ch (repo).

**Files:**
- Modify: `rust/tonk-fab/src/element.rs` (`toggle_menu`, delete `clear_menu_width`)
- Modify: `rust/tonk-core/assets/library/profile.yaml` (menu comment + four max-width caps)

**Interfaces:**
- Consumes: `crate::logic::menu_min_width` (unchanged — its "only widen" contract is exactly the ratchet comparison against the segment's current rendered width).
- Produces: nothing later tasks reference.

- [ ] **Step 1: Ratchet in element.rs**

Replace `toggle_menu` with:

```rust
/// Open (or close) the dropdown owned by `seg` by toggling its `is-open` class,
/// closing the other menu (matched by `other_sel`) so only one is open at a time.
/// The open-direction is CSS, keyed off the FAB's `fab-dock-*` class.
///
/// On open the segment is widened (an eased inline `min-width`) to the menu's
/// natural width when the menu is the wider of the two — the stylesheet's
/// `width: 100%` then makes menu and rung exactly equal. The stamped
/// `min-width` RATCHETS: it is never cleared, so a column keeps its width
/// across open/close and across the other menu's toggles, and only grows —
/// re-measured on each open — when a wider element has entered the menu.
/// (Clearing on close made the bar's columns visibly resize depending on
/// which dropdown was open.)
fn toggle_menu(element: &HtmlElement, seg: &Element, other_sel: &str) {
    if let Some(other) = element.query_selector(other_sel).ok().flatten() {
        other.class_list().remove_1("is-open").ok();
    }
    let opening = !seg.class_list().contains("is-open");
    seg.class_list().toggle_with_force("is-open", opening).ok();
    if opening {
        equalize_menu_width(seg);
    }
}
```

Delete the `clear_menu_width` function and its doc comment entirely. Leave
`equalize_menu_width` as is — measuring against the segment's CURRENT rendered
width (which includes any previously stamped `min-width`) is what makes the
stamp a ratchet.

- [ ] **Step 2: Update the menu comment + caps in profile.yaml**

In the `.fab__menu` comment, replace the parenthetical "(cleared on close)"
with "(never cleared — a column only ratchets wider)".

Change four `max-width` values:
- `.fab__name` rule: `max-width: 16ch;` → `max-width: 15ch;`
- `.fab__name-input` rule: `max-width: 16ch;` → `max-width: 15ch;`
- `.fab__space` rule: `max-width: 16ch;` → `max-width: 24ch;`
- `.fab__space tonk-editable` rule: `max-width: 16ch;` → `max-width: 24ch;`

- [ ] **Step 3: Verify**

Run: `cargo clippy -p tonk-fab --target wasm32-unknown-unknown -- -D warnings`
Expected: clean (catches a now-unused function if the delete missed a call).
Run: `cargo test -p tonk-fab && cargo test -p tonk-worker --test standard_library`
Expected: PASS.
Run: `cargo clippy --all -- -D warnings`
Expected: clean.

- [ ] **Step 4: Commit**

```bash
jj commit -m "feat(fab): ratchet equalized segment widths and retune name caps"
```

---

### Task 7: Uniform text across the control (live-review amendment)

The wireframe sets every label — bar segments and dropdown rows — in one size,
weight, and ink. The live FAB deviates three ways (spec §1 amendment,
2026-07-02). All edits are in the FAB `<style>` block of
`rust/tonk-core/assets/library/profile.yaml`.

**Files:**
- Modify: `rust/tonk-core/assets/library/profile.yaml`

**Interfaces:** none — presentational CSS only.

- [ ] **Step 1: Drop the three deviations**

In the `.fab__name` rule, delete the line:

```css
          font-weight: 500;
```

In the `.fab__menu-item` rule, delete the line (rows then inherit the bar's
16px from `.fab`):

```css
          font-size: 13px;
```

In the `.fab__menu-glyph` rule, delete the line (the glyph inherits the row
size):

```css
          font-size: 14px;
```

In the `.fab__menu-item--action` rule, delete the line (actions then inherit
`.fab__menu-item`'s full `--fab-ink`):

```css
          color: color-mix(in oklab, var(--fab-ink) 70%, transparent);
```

Update the comment above `.fab__menu-item--action` — it currently reads
"actions, not spaces: right-aligned and quieter" — drop the "and quieter"
claim (they are no longer dimmed).

- [ ] **Step 2: Verify**

Run: `cargo test -p tonk-worker --test standard_library`
Expected: PASS (YAML still parses/lowers).

- [ ] **Step 3: Commit**

```bash
jj commit -m "feat(fab): unify text size, weight, and ink across the control"
```

---

### Task 8: Window-scoped drag listeners + stale-press guard (spec §6)

**Files:**
- Modify: `rust/tonk-fab/src/element.rs` (`attach_drag`)

**Interfaces:** none new; `nearest_dock`/`apply_dock`/`persist_dock` are consumed unchanged.

- [ ] **Step 1: Restructure `attach_drag`**

Keep the `on_down` closure exactly as is, attached to the ELEMENT. Extract the
shared drag-finish into a helper and attach `on_move`, `on_up`, and a new
`pointercancel` listener to the guest WINDOW:

```rust
/// Finish a press at viewport point `(x, y)`: clear the press flags and — if
/// the press had been promoted to a drag — release capture, drop the dragging
/// class, and snap/persist the nearest dock. Shared by `pointerup`,
/// `pointercancel`, and the stale-press guard in `pointermove`.
fn finish_drag(el: &HtmlElement, pointer_id: i32, x: f64, y: f64) {
    el.dataset().delete("fabPressing");
    let moved = el.dataset().get("fabMoved").is_some();
    if !moved {
        return;
    }
    el.release_pointer_capture(pointer_id).ok();
    if let Some(fab) = el.query_selector(".fab").ok().flatten() {
        fab.class_list().remove_1("dragging").ok();
    }
    let dock = nearest_dock(x, y, viewport_width(), viewport_height());
    apply_dock(el, dock);
    persist_dock(dock);
}
```

In `on_move`, right after the existing `fabPressing` early-return, add the
stale-press guard:

```rust
        // A press with NO button still held means the pointerup was lost
        // (fast flick released outside the element before capture was taken):
        // finish the drag here so a later hover can't resume a phantom press.
        if e.buttons() == 0 {
            finish_drag(&el_move, e.pointer_id(), e.client_x() as f64, e.client_y() as f64);
            return;
        }
```

Rewrite `on_up` to delegate:

```rust
    let el_up = element.clone();
    let on_up = Closure::<dyn FnMut(PointerEvent)>::new(move |e: PointerEvent| {
        if el_up.dataset().get("fabPressing").is_none() {
            return;
        }
        finish_drag(&el_up, e.pointer_id(), e.client_x() as f64, e.client_y() as f64);
    });
```

Add an `on_cancel` closure with the identical body (a cancelled pointer ends
the drag where it stands).

Attach: `pointerdown` on the element (unchanged); `pointermove`, `pointerup`,
`pointercancel` on the window —

```rust
    let target: &web_sys::EventTarget = element.unchecked_ref();
    target
        .add_event_listener_with_callback("pointerdown", on_down.as_ref().unchecked_ref())
        .ok();
    // WINDOW-scoped move/up/cancel: a fast flick outruns the element before
    // its first pointermove fires (capture is only taken past the drag
    // threshold), so element-scoped listeners lose the pointer mid-drag and
    // never see the release. The overlay iframe is pinned full-viewport while
    // dragging, so the window sees every event. Captured events (post
    // threshold) still bubble here.
    if let Some(win) = window() {
        let wtarget: &web_sys::EventTarget = win.unchecked_ref();
        for (name, cb) in [
            ("pointermove", on_move.as_ref()),
            ("pointerup", on_up.as_ref()),
            ("pointercancel", on_cancel.as_ref()),
        ] {
            wtarget
                .add_event_listener_with_callback(name, cb.unchecked_ref())
                .ok();
        }
    }
    on_down.forget();
    on_move.forget();
    on_up.forget();
    on_cancel.forget();
```

Update `attach_drag`'s doc comment: listeners' new homes and why (fast-flick
loss + lost-release phantom drag), and document the guard.

- [ ] **Step 2: Verify**

Run: `cargo clippy -p tonk-fab --target wasm32-unknown-unknown -- -D warnings`
Expected: clean.
Run: `cargo test -p tonk-fab`
Expected: 23/23 pass (no logic changes).

- [ ] **Step 3: Commit**

```bash
jj commit -m "fix(fab): survive fast drags with window-scoped pointer listeners"
```

---

### Task 9: Right-dock mirroring (spec §7, CSS only)

**Files:**
- Modify: `rust/tonk-core/assets/library/profile.yaml` (FAB `<style>` block)

**Interfaces:** none — presentational CSS keyed off the existing host `fab-dock-right` class.

- [ ] **Step 1: Re-key the dead `.fab--dock-right` rules**

The three rules around line 545 use `.fab--dock-right`, a class nothing sets.
Re-key them to the host class (the comment above them stays):

```css
        .fab-dock-right .fab { flex-direction: row-reverse; }
        .fab-dock-right .fab__cap-l { border-radius: 0 36px 36px 0; }
        .fab-dock-right .fab__cap-r { border-radius: 36px 0 0 36px; }
```

- [ ] **Step 2: Complete the mirror**

AFTER the existing `.fab__share.is-open .fab__share-menu { display: flex; }`
rule (so the flip wins its specificity/source-order ties), add:

```css
        /* Mirrored on a right dock: the bar is row-reversed, so the share
           zone is the LEFT end — its roster flips to anchor left (the repo
           switcher already re-anchors right via `.fab-dock-right .fab__menu`).
           Row alignment mirrors with it: actions and member cards read from
           the bar's outer edge inward on both docks. */
        .fab-dock-right .fab__share-menu { left: 0; right: auto; }
        .fab-dock-right .fab__menu-item--action { justify-content: flex-start; }
        .fab-dock-right .fab__menu-item--member { text-align: left; }
        /* Telescope tiles clip toward the circle, so the unfold reads from
           the circle outward on a right dock too. */
        .fab-dock-right .fab--anim .fab__tele { justify-content: flex-start; }
```

- [ ] **Step 3: Verify**

Run: `cargo test -p tonk-worker --test standard_library`
Expected: PASS 3/3.

- [ ] **Step 4: Commit**

```bash
jj commit -m "feat(fab): mirror the bar and menus on a right dock"
```

---

### Task 10: Preload menu widths (spec §8)

**Files:**
- Modify: `rust/tonk-fab/Cargo.toml` (web-sys features: `MutationObserver`, `MutationObserverInit`, `FontFaceSet`)
- Modify: `rust/tonk-fab/src/element.rs`

**Interfaces:**
- Consumes: `crate::logic::ratchet_min_width` (unchanged) via the existing `equalize_menu_width`.
- Produces: nothing later tasks reference.

- [ ] **Step 1: Make `equalize_menu_width` measurable while closed**

In `equalize_menu_width`, the menu is `display: none` unless open. Before the
`width: max-content` measurement, force it measurable when closed, and restore
after (all inline, synchronous — nothing paints):

```rust
    let style = menu.unchecked_ref::<HtmlElement>().style();
    // A closed menu is `display: none` (no boxes). Force it measurable —
    // invisible and out of the paint (`visibility: hidden`), laid out at its
    // natural width — then restore. All within one task, so no flash.
    let closed = !seg.class_list().contains("is-open");
    if closed {
        let _ = style.set_property("display", "flex");
        let _ = style.set_property("visibility", "hidden");
    }
    let _ = style.set_property("width", "max-content");
    let natural = menu.get_bounding_client_rect().width();
    let _ = style.remove_property("width");
    if closed {
        let _ = style.remove_property("display");
        let _ = style.remove_property("visibility");
    }
```

Update its doc comment (measures open or closed; preload + observer callers).

- [ ] **Step 2: Preload on connect, observe mutations, refresh on font load**

Add to `connected_callback`, inside the `!already_bound` block after
`attach_gestures(this)`: `preload_menu_widths(this);`

```rust
/// The two dropdown-owning segments.
const MENU_SEGMENTS: [&str; 2] = [".fab__repo", ".fab__share"];

/// Stamp both segments' ratcheted widths up front and keep them fresh, so a
/// dropdown OPEN never changes the bar: rows render asynchronously (a
/// MutationObserver per menu re-ratchets as content lands) and the Plex face
/// loads asynchronously (a font swap changes metrics but fires no mutation,
/// so `document.fonts.ready` triggers one more pass).
fn preload_menu_widths(element: &HtmlElement) {
    for sel in MENU_SEGMENTS {
        if let Some(seg) = element.query_selector(sel).ok().flatten() {
            equalize_menu_width(&seg);
            observe_menu(&seg);
        }
    }
    refresh_on_fonts_ready(element);
}
```

`observe_menu`: one observer per menu, re-ratcheting on any content change.
Mirror the MutationObserver construction pattern already used in
`rust/tonk-display/src/fallback.rs` (same web-sys version):

```rust
/// Re-ratchet `seg`'s width whenever its menu's content changes (rows arrive
/// from live subscriptions well after connect). The observer lives as long as
/// the page (closure forgotten) — one FAB, two menus, so no accounting.
fn observe_menu(seg: &Element) {
    let Some(menu) = seg.query_selector(".fab__menu").ok().flatten() else {
        return;
    };
    let seg_for_cb = seg.clone();
    let cb = Closure::<dyn FnMut(js_sys::Array, web_sys::MutationObserver)>::new(
        move |_records: js_sys::Array, _obs: web_sys::MutationObserver| {
            equalize_menu_width(&seg_for_cb);
        },
    );
    if let Ok(observer) = web_sys::MutationObserver::new(cb.as_ref().unchecked_ref()) {
        let init = web_sys::MutationObserverInit::new();
        init.set_child_list(true);
        init.set_subtree(true);
        init.set_character_data(true);
        observer.observe_with_options(&menu, &init).ok();
    }
    cb.forget();
}
```

(If the workspace web-sys exposes builder-style `child_list(&mut ...)` instead
of `set_child_list`, follow whichever tonk-display uses — match, don't fight,
the pinned version.)

```rust
/// One more ratchet pass once the fonts land: measurements taken against the
/// fallback face under-report the condensed Plex metrics.
fn refresh_on_fonts_ready(element: &HtmlElement) {
    let Some(document) = window().and_then(|w| w.document()) else {
        return;
    };
    let ready = match document.fonts().ready() {
        Ok(p) => p,
        Err(_) => return,
    };
    let el = element.clone();
    spawn_local(async move {
        let _ = wasm_bindgen_futures::JsFuture::from(ready).await;
        for sel in MENU_SEGMENTS {
            if let Some(seg) = el.query_selector(sel).ok().flatten() {
                equalize_menu_width(&seg);
            }
        }
    });
}
```

Add the three web-sys features to `rust/tonk-fab/Cargo.toml` alphabetically in
the existing feature list: `"FontFaceSet"`, `"MutationObserver"`,
`"MutationObserverInit"`.

- [ ] **Step 3: Verify**

Run: `cargo clippy -p tonk-fab --target wasm32-unknown-unknown -- -D warnings`
Expected: clean.
Run: `cargo test -p tonk-fab && cargo test -p tonk-worker --test standard_library && cargo clippy --all -- -D warnings`
Expected: all pass/clean.

- [ ] **Step 4: Commit**

```bash
jj commit -m "feat(fab): preload ratcheted menu widths at connect"
```

---

### Task 11: Authoritative fonts-ready restamp + press-flag cleanup (final-review fixes)

Final review found the preload's font interaction inverted: pre-font
measurements use the fallback face, which is typically WIDER than condensed
Plex, so the never-shrink ratchet bakes in over-wide columns the fonts-ready
pass can only fail to correct. Fix: the fonts-ready pass restamps
authoritatively (both directions); everything else stays ratcheted. Plus a
lifecycle hardening: clear press flags on disconnect.

**Files:**
- Modify: `rust/tonk-fab/src/logic.rs` (new pure helper + tests)
- Modify: `rust/tonk-fab/src/element.rs`

**Interfaces:**
- Consumes: existing `ratchet_min_width` (unchanged).
- Produces: `pub fn corrected_min_width(menu_natural: f64) -> Option<f64>` in `crate::logic`.

- [ ] **Step 1 (TDD): pure helper in logic.rs**

Tests first (own `#[cfg(test)] mod corrected` block, plain `#[test]`, run
`cargo test -p tonk-fab corrected` to see them fail, then implement):

```rust
#[cfg(test)]
mod corrected {
    use super::*;

    #[test]
    fn a_rendered_menu_restamps_to_its_ceiled_width() {
        assert_eq!(corrected_min_width(220.4), Some(221.0));
    }

    #[test]
    fn an_empty_menu_clears_nothing() {
        // Zero means the menu has no rendered rows yet — leave the current
        // stamp alone rather than collapsing the column.
        assert_eq!(corrected_min_width(0.0), None);
    }
}
```

```rust
/// The AUTHORITATIVE `min-width` for the one fonts-ready restamp: the menu's
/// fresh real-metrics width, ceiled, replacing any ratcheted stamp in BOTH
/// directions — measurements taken before the font landed used the fallback
/// face (typically wider than condensed Plex), and the never-shrink ratchet
/// cannot correct an over-wide stamp downward. `None` (an unrendered, empty
/// menu) leaves the existing stamp untouched.
pub fn corrected_min_width(menu_natural: f64) -> Option<f64> {
    (menu_natural > 0.0).then(|| menu_natural.ceil())
}
```

- [ ] **Step 2: element.rs — measure helper, restamp path, disconnect cleanup**

Extract the measurement block of `equalize_menu_width` (the closed-menu
forcing + `width: max-content` read + restore) into:

```rust
/// Measure the menu's natural (max-content) width, open or closed — a closed
/// menu (`display: none`) is momentarily forced measurable, invisible and out
/// of the paint (`visibility: hidden`); everything is restored before return.
/// Synchronous within one task, so nothing flashes.
fn menu_natural_width(seg: &Element, menu: &Element) -> f64 {
    // (body moved verbatim from equalize_menu_width)
}
```

`equalize_menu_width` keeps its ratchet behavior, now via the helper. Add the
authoritative sibling used only by the fonts-ready pass:

```rust
/// Restamp `seg`'s width from a FRESH measurement, replacing any ratcheted
/// stamp in both directions — the one-time correction for stamps taken
/// against fallback-font metrics before the Plex face landed. The min-width
/// transition eases the correction, riding the font swap's own reflow.
fn restamp_menu_width(seg: &Element) {
    let Some(menu) = seg.query_selector(".fab__menu").ok().flatten() else {
        return;
    };
    let natural = menu_natural_width(seg, &menu);
    if let Some(min_width) = corrected_min_width(natural) {
        let _ = seg
            .unchecked_ref::<HtmlElement>()
            .style()
            .set_property("min-width", &format!("{min_width}px"));
    }
}
```

In `refresh_on_fonts_ready`, call `restamp_menu_width(&seg)` instead of
`equalize_menu_width(&seg)`, and fix its doc comment (the fallback face
typically OVER-reports condensed Plex, hence an authoritative restamp rather
than a ratchet). Extend the `crate::logic` import with `corrected_min_width`.

In `disconnected_callback`, after the `settleTimer` cleanup add:

```rust
        // Drop any in-flight press: the window-scoped drag listeners outlive
        // a clone remount, and a press left armed on the old element would
        // let its stale `finish_drag` persist a phantom dock on the next
        // buttons-up move.
        this.dataset().delete("fabPressing");
        this.dataset().delete("fabMoved");
```

- [ ] **Step 3: Verify**

Run: `cargo test -p tonk-fab` (25/25 with the two new tests),
`cargo clippy -p tonk-fab --target wasm32-unknown-unknown -- -D warnings`,
`cargo test -p tonk-worker --test standard_library`,
`cargo clippy --all -- -D warnings`
Expected: all pass/clean.

- [ ] **Step 4: Commit**

```bash
jj commit -m "fix(fab): restamp menu widths authoritatively once fonts load"
```
