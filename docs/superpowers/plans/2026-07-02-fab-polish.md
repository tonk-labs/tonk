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
