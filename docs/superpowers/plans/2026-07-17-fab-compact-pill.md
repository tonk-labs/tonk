# FAB Compact Pill Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `<tonk-fab>` usable on small screens and touch devices: an adaptive compact pill (scroll-snap pages + chevron vertical menu) plus repaired touch dragging and viewport clamping.

**Architecture:** One DOM, no new components. Compact mode is a measured layout state (`fab--compact` on `.fab`): the existing segments move inside a horizontal scroll-snap strip grouped into pages, a chevron cap toggles a CSS-restacked vertical menu (`fab--menu`). Wide viewports render pixel-identically to today via `display: contents` wrappers + flex `order`. All new geometry is pure functions in `logic.rs` (native-tested); DOM wiring lives in `element.rs`; markup in `markup.rs`; styling in `fab.css`.

**Tech Stack:** Rust/WASM (`custom-elements` crate), web-sys pointer events, CSS scroll-snap. Spec: `docs/superpowers/specs/2026-07-17-fab-compact-pill-design.md`.

## Global Constraints

- No emojis in code, commits, or output.
- Conventional Commits: `type(tonk-fab): subject`, imperative, lowercase, no trailing period.
- Native tests: plain `#[test]` in behaviour-named `#[cfg(test)] mod`s with `it_does_x` / descriptive-sentence names, matching `logic.rs`'s existing style.
- `logic.rs` must stay DOM-free (compiles and tests on the native target).
- Wide-viewport rendering must not change: same visual order (circle, account, repo, share, end cap), same telescope behaviour, same dock persistence.
- The lint gate is workspace-wide: `cargo clippy --workspace --all-targets --all-features` (native) plus `cargo fmt --check` must pass at the end.
- Wasm tests run in a browser: `cargo test -p tonk-fab --target wasm32-unknown-unknown` with a major-version-matched chromedriver on PATH and Chrome at the default `/Applications` path (or the safaridriver route). If neither driver is available locally, note it in the final report rather than skipping silently.

## File Structure

No new files. All work lands in `rust/tonk-fab/src/`:

- `logic.rs` — new pure functions: `DOCK_INSET_PX`, `is_compact`, `clamp_position` (+ tests).
- `element.rs` — clamping in `track_position`; touch-aware drag arming; `telescope_tiles` re-scoped and visually ranked; `update_compact_mode` / `expanded_bar_width` / resize listener; chevron + strip-scroll gestures; `close_menus` extension; wasm tests.
- `markup.rs` — bar restructure: `.fab__strip` > `.fab__page`s, tile modifier classes, chevron button (+ tests).
- `fab.css` — touch-action + coarse-pointer hit target; wide-mode parity (`display: contents`, `order`); compact strip/pages/chevron; vertical-menu restack; drag scroll lock.

---

### Task 1: Pure geometry — `is_compact` and `clamp_position`

**Files:**
- Modify: `rust/tonk-fab/src/logic.rs` (add after `mirrored`, around line 200)

**Interfaces:**
- Produces: `pub const DOCK_INSET_PX: f64`, `pub fn is_compact(expanded_width: f64, viewport_width: f64) -> bool`, `pub fn clamp_position(left: f64, top: f64, width: f64, height: f64, vw: f64, vh: f64) -> (f64, f64)`. Tasks 2 and 5 consume these from `crate::logic`.

- [ ] **Step 1: Write the failing tests**

Append to `logic.rs` alongside the existing test modules:

```rust
#[cfg(test)]
mod compact {
    use super::*;

    #[test]
    fn a_bar_that_fits_with_both_insets_is_not_compact() {
        assert!(!is_compact(300.0, 400.0));
    }

    #[test]
    fn a_bar_wider_than_the_viewport_minus_insets_is_compact() {
        assert!(is_compact(380.0, 400.0));
    }

    #[test]
    fn the_exact_fit_is_not_compact() {
        // 368 + 2*16 == 400: still fits; only strictly-greater flips it, so
        // the threshold is identical in both directions and cannot flap.
        assert!(!is_compact(368.0, 400.0));
    }
}

#[cfg(test)]
mod clamp {
    use super::*;

    #[test]
    fn an_inside_position_is_untouched() {
        assert_eq!(
            clamp_position(100.0, 50.0, 300.0, 36.0, 1000.0, 800.0),
            (100.0, 50.0)
        );
    }

    #[test]
    fn it_clamps_every_edge() {
        // Past the origin pins to 0.
        assert_eq!(
            clamp_position(-20.0, -5.0, 300.0, 36.0, 1000.0, 800.0),
            (0.0, 0.0)
        );
        // Right/bottom overflow pins to viewport minus the bar.
        assert_eq!(
            clamp_position(900.0, 790.0, 300.0, 36.0, 1000.0, 800.0),
            (700.0, 764.0)
        );
    }

    #[test]
    fn a_bar_wider_than_the_viewport_pins_to_the_origin() {
        // vw - width is negative; the origin wins (max runs last) so the
        // bar's left edge — and the circle cap on it — stays reachable.
        assert_eq!(
            clamp_position(50.0, 10.0, 500.0, 36.0, 400.0, 800.0),
            (0.0, 10.0)
        );
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p tonk-fab compact clamp` (from the repo root; native target)
Expected: FAIL to compile — `is_compact` / `clamp_position` / `DOCK_INSET_PX` not found.

- [ ] **Step 3: Write the implementation**

Add to `logic.rs` after `mirrored` (before the telescope constants):

```rust
/// The stylesheet's dock inset — `tonk-fab.fab-dock-* { …: 16px }` in
/// `fab.css`. The compact-mode fit test must account for it on both sides.
pub const DOCK_INSET_PX: f64 = 16.0;

/// Whether the bar must render compact: the fully EXPANDED bar plus both
/// dock insets no longer fits the viewport width. Keyed on the would-be
/// expanded width (not the current rendered width), so the threshold is the
/// same entering and leaving compact and cannot oscillate.
pub fn is_compact(expanded_width: f64, viewport_width: f64) -> bool {
    expanded_width + 2.0 * DOCK_INSET_PX > viewport_width
}

/// Clamp a dragged bar's top-left corner so the bar stays fully inside the
/// viewport. The origin clamp runs LAST: a bar wider or taller than the
/// viewport pins to the left/top edge, keeping the grab handle reachable.
pub fn clamp_position(
    left: f64,
    top: f64,
    width: f64,
    height: f64,
    vw: f64,
    vh: f64,
) -> (f64, f64) {
    (
        left.min(vw - width).max(0.0),
        top.min(vh - height).max(0.0),
    )
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p tonk-fab`
Expected: PASS (all existing modules plus `compact` and `clamp`).

- [ ] **Step 5: Commit**

```bash
git add rust/tonk-fab/src/logic.rs
git commit -m "feat(tonk-fab): add compact-fit and viewport-clamp geometry"
```

---

### Task 2: Clamp the drag to the viewport

**Files:**
- Modify: `rust/tonk-fab/src/element.rs` (imports around line 37; `track_position` around line 1072)

**Interfaces:**
- Consumes: `logic::clamp_position` from Task 1.
- Produces: `track_position` now clamps internally; no signature change, no caller changes.

- [ ] **Step 1: Update the import**

In the `use crate::logic::{…}` list at the top of `element.rs`, add `clamp_position`:

```rust
use crate::logic::{
    DOCK_CLASSES, Dock, clamp_position, corrected_min_width, create_space_claim_json,
    dock_claim_json, dock_from_conclusions, mirrored, nearest_dock, pause_claim_json,
    profile_rename_claim_json, ratchet_min_width, telescope_delay_ms, telescope_settle_ms,
};
```

- [ ] **Step 2: Clamp inside `track_position`**

Replace the existing `track_position`:

```rust
/// Track the FAB at `(left, top)` (viewport top-left) with plain `left`/`top`
/// during a drag — no corner anchoring, so it follows the cursor 1:1 without
/// jumping as it crosses the viewport midlines. Clamped so the bar can never
/// leave the viewport, whatever the pointer does. (The mirror-flip
/// compensation in `apply_mirror_from_handle` writes `left` directly and is
/// not clamped — the very next pointer move re-clamps, so any excursion
/// lasts one frame at most.)
fn track_position(el: &HtmlElement, left: f64, top: f64) {
    let rect = el.get_bounding_client_rect();
    let (left, top) = clamp_position(
        left,
        top,
        rect.width(),
        rect.height(),
        viewport_width(),
        viewport_height(),
    );
    let style = el.style();
    let _ = style.remove_property("right");
    let _ = style.remove_property("bottom");
    let _ = style.set_property("left", &format!("{}px", left));
    let _ = style.set_property("top", &format!("{}px", top));
}
```

- [ ] **Step 3: Verify it builds and existing tests pass**

Run: `cargo check -p tonk-fab --target wasm32-unknown-unknown && cargo test -p tonk-fab`
Expected: clean check, all native tests PASS (the clamp function itself is covered by Task 1's tests).

- [ ] **Step 4: Commit**

```bash
git add rust/tonk-fab/src/element.rs
git commit -m "fix(tonk-fab): clamp the drag so the bar cannot leave the viewport"
```

---

### Task 3: Repair touch dragging

**Files:**
- Modify: `rust/tonk-fab/src/element.rs` (`DRAG_THRESHOLD_PX` around line 96; `attach_drag` around line 883; `finish_drag` around line 1025; `disconnected_callback` around line 164)
- Modify: `rust/tonk-fab/src/fab.css` (the `.fab__cap-l` block around line 193)

**Interfaces:**
- Consumes: nothing new.
- Produces: touch presses arm with `data-fab-touch`, capture immediately on the cap, and use an 8px threshold. No API changes.

- [ ] **Step 1: Add the touch threshold constant**

Below `DRAG_THRESHOLD_PX` in `element.rs`:

```rust
/// The drag threshold for TOUCH pointers. Wider than the mouse threshold: a
/// finger wobbles a few px during a plain tap, and promoting that to a drag
/// would eat the tap-to-toggle gesture.
const TOUCH_DRAG_THRESHOLD_PX: f64 = 8.0;
```

- [ ] **Step 2: Arm touch presses with immediate capture**

In `attach_drag`'s `on_down` closure, after the `if !on_circle { return; }` check and before the rect bookkeeping, the cap element is needed — restructure the circle check to keep it:

Replace:

```rust
        let on_circle = e
            .target()
            .and_then(|t| t.dyn_into::<Element>().ok())
            .and_then(|el| el.closest(".fab__cap-l").ok().flatten())
            .is_some();
        if !on_circle {
            return;
        }
```

with:

```rust
        let Some(cap) = e
            .target()
            .and_then(|t| t.dyn_into::<Element>().ok())
            .and_then(|el| el.closest(".fab__cap-l").ok().flatten())
        else {
            return;
        };
        // TOUCH presses capture IMMEDIATELY, and on the CAP (not the host):
        // a fast flick outruns even the window listeners' first delivery on
        // some mobile browsers, and deferred capture is the desktop
        // compromise that lets a stationary mouse press click — a touch tap
        // still clicks with capture held, because capture retargets pointer
        // events to the cap, which is exactly where the tap's click routes
        // anyway. Capturing on the host instead would retarget the click to
        // the host and break tap-to-toggle (`attach_gestures` walks
        // `closest(".fab__cap-l")` from the click target).
        if e.pointer_type() == "touch" {
            el_down.dataset().set("fabTouch", "1").ok();
            cap.set_pointer_capture(e.pointer_id()).ok();
        } else {
            el_down.dataset().delete("fabTouch");
        }
```

- [ ] **Step 3: Use the touch threshold and skip re-capture in `on_move`**

In the promotion block of `on_move`, replace:

```rust
        if el_move.dataset().get("fabMoved").is_none() {
            if dx.hypot(dy) < DRAG_THRESHOLD_PX {
                return;
            }
            el_move.dataset().set("fabMoved", "1").ok();
            el_move.set_pointer_capture(e.pointer_id()).ok();
```

with:

```rust
        if el_move.dataset().get("fabMoved").is_none() {
            let touch = el_move.dataset().get("fabTouch").is_some();
            let threshold = if touch {
                TOUCH_DRAG_THRESHOLD_PX
            } else {
                DRAG_THRESHOLD_PX
            };
            if dx.hypot(dy) < threshold {
                return;
            }
            el_move.dataset().set("fabMoved", "1").ok();
            // A touch press already holds capture on the cap (see
            // `on_down`); re-capturing on the host would retarget the
            // post-drag click mid-gesture.
            if !touch {
                el_move.set_pointer_capture(e.pointer_id()).ok();
            }
```

- [ ] **Step 4: Release the cap's capture and clear the flag in `finish_drag` and `disconnected_callback`**

In `finish_drag`, after `el.dataset().delete("fabPressing");` add:

```rust
    let touch = el.dataset().get("fabTouch").is_some();
    el.dataset().delete("fabTouch");
```

and replace `el.release_pointer_capture(pointer_id).ok();` with:

```rust
    if touch {
        // Touch capture lives on the cap (see `attach_drag`). Explicit
        // release is belt-and-braces — pointerup implicitly releases — but
        // pointercancel paths keep it honest.
        if let Some(cap) = el.query_selector(".fab__cap-l").ok().flatten() {
            cap.release_pointer_capture(pointer_id).ok();
        }
    } else {
        el.release_pointer_capture(pointer_id).ok();
    }
```

In `disconnected_callback`, alongside the `fabPressing`/`fabMoved` deletes add:

```rust
        this.dataset().delete("fabTouch");
```

- [ ] **Step 5: CSS — stop the browser competing, widen the touch target**

In `fab.css`, extend the grab-handle cap block (the `.fab__cap-l { flex: none; … }` rule around line 193) by adding one declaration to it:

```css
  /* The browser must not race us for touch gestures on the drag handle —
     without this, mobile scroll/zoom recognizers usually win and the drag
     stutters or dies. Scoped to the cap so touch behaviour anywhere else
     on the bar (and the compact strip's own pan) is untouched. */
  touch-action: none;
```

and add after that rule:

```css
/* On coarse pointers the 36px cap grows an invisible 52px hit area
   (>=44px Apple/Android guidance) without visual change. The cap is
   `position: relative` via `.fab__seg`, so the inset anchors to it; the
   pseudo-element is part of the cap for hit-testing, so both the drag
   arming and the tap gesture route through `closest(".fab__cap-l")`
   exactly as before. */
@media (pointer: coarse) {
  .fab__cap-l::after {
    content: "";
    position: absolute;
    inset: -8px;
  }
}
```

- [ ] **Step 6: Verify it builds; native tests pass**

Run: `cargo check -p tonk-fab --target wasm32-unknown-unknown && cargo test -p tonk-fab`
Expected: clean check, tests PASS. (`pointer_type()` is part of web-sys's `PointerEvent`, already a dependency feature.)

- [ ] **Step 7: Commit**

```bash
git add rust/tonk-fab/src/element.rs rust/tonk-fab/src/fab.css
git commit -m "fix(tonk-fab): make touch dragging reliable"
```

---

### Task 4: Restructure the markup — strip, pages, chevron (wide mode pixel-identical)

**Files:**
- Modify: `rust/tonk-fab/src/markup.rs` (`fab_html`, lines 75–181, and its tests)
- Modify: `rust/tonk-fab/src/element.rs` (`telescope_tiles` around line 800; wasm `fab_host` fixture around line 1259)
- Modify: `rust/tonk-fab/src/fab.css` (wide-mode parity rules; stylesheet test selector)
- Modify: `rust/tonk-fab/src/logic.rs` (stylesheet test around line 1307)

**Interfaces:**
- Consumes: nothing new.
- Produces: DOM contract used by Tasks 5–6 — `.fab__strip` (direct child of `.fab`) containing `.fab__page.fab__page--main` (repo tile) and `.fab__page.fab__page--more` (share, then account tiles); tiles carry `fab__tele--account` / `--repo` / `--share` / `--end` modifiers; the end tile (outside the strip) holds both the decorative `.fab__end` nub and a `<button class="fab__seg fab__cap-r fab__more">`.

- [ ] **Step 1: Write the failing markup tests**

Add to `markup.rs`'s test module:

```rust
    #[test]
    fn it_groups_the_tiles_into_compact_pages() {
        let html = fab_html("did:key:z6Mk");
        // Page 1 holds the space name + switcher; page 2 share then account.
        // The strip and pages are `display: contents` on wide viewports, so
        // this grouping is invisible there; compact mode makes them the
        // scroll-snap pager.
        let strip = html.find("fab__strip").expect("strip present");
        let main = html.find("fab__page--main").expect("main page present");
        let more = html.find("fab__page--more").expect("more page present");
        let repo = html.find("fab__tele--repo").expect("repo tile present");
        let share = html.find("fab__tele--share").expect("share tile present");
        let account = html.find("fab__tele--account").expect("account tile present");
        assert!(strip < main && main < more, "strip wraps the pages in order");
        assert!(main < repo && repo < more, "the repo tile is page 1's content");
        assert!(more < share && share < account, "page 2 is share, then account");
    }

    #[test]
    fn it_authors_the_chevron_beside_the_end_nub() {
        let html = fab_html("did:key:z6Mk");
        // Both live in the end tile, OUTSIDE the strip: the chevron is a
        // fixed right cap (like the circle on the left), never scrolled away
        // with the pages. CSS shows exactly one of the pair per mode.
        let end_tile = html.find("fab__tele--end").expect("end tile present");
        let nub = html.find("fab__end").expect("nub present");
        let more = html.find("fab__more").expect("chevron present");
        assert!(end_tile < nub && end_tile < more);
        assert!(html.contains(r#"<button type="button" class="fab__seg fab__cap-r fab__more""#));
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p tonk-fab --lib markup`
Expected: FAIL — `fab__strip` etc. not found in the HTML.

- [ ] **Step 3: Rewrite the bar portion of `fab_html`**

Replace the bar `<div class="fab …">…</div>` portion of the format string in `fab_html` (everything from `<div class="fab fab--anim fab--settled">` through its closing `</div>`, leaving the scrim line and the whole `<wa-dialog>` unchanged):

```html
<div class="fab fab--anim fab--settled">
  <span class="fab__seg fab__cap-l fab__circle"><ui-sync-status with="main@{space}" onpause="tonk:pause-sync"></ui-sync-status></span>
  <div class="fab__strip">
    <div class="fab__page fab__page--main">
      <div class="fab__tele fab__tele--repo">
        <span class="fab__seg fab__repo">
          <span class="fab__space"><ui-space-name space="{space}"></ui-space-name></span>
          <ui-dropdown class="fab__menu" exclude="{space}">
            <ui-space-switcher exclude="{space}"></ui-space-switcher>
          </ui-dropdown>
        </span>
      </div>
    </div>
    <div class="fab__page fab__page--more">
      <div class="fab__tele fab__tele--share">
        <span class="fab__seg fab__share">
          <tonk-share space="{space}">
            <form class="fab__share-form">
              <button type="submit" class="fab__share-trigger">
                <span class="fab__share-label fab__share-label--idle">share</span>
                <span class="fab__share-label fab__share-label--copying">
                  <span class="fab__share-spinner"></span>copying…
                </span>
                <span class="fab__share-label fab__share-label--copied">
                  <wa-icon name="check"></wa-icon>copied
                </span>
                <span class="fab__share-label fab__share-label--failed">
                  <wa-icon name="triangle-exclamation"></wa-icon>failed
                </span>
              </button>
            </form>
          </tonk-share>
          <nav class="fab__menu fab__share-menu">
            <ui-member-roster space="{space}"></ui-member-roster>
          </nav>
        </span>
      </div>
      <div class="fab__tele fab__tele--account">
        <span class="fab__seg fab__account">
          <span class="fab__name"><ui-profile-name></ui-profile-name></span>
        </span>
      </div>
    </div>
  </div>
  <div class="fab__tele fab__tele--end">
    <span class="fab__seg fab__cap-r fab__end" aria-hidden="true"></span>
    <button type="button" class="fab__seg fab__cap-r fab__more" aria-label="More controls"><wa-icon name="chevron-right"></wa-icon></button>
  </div>
</div>
```

Also update the module doc's "Structure is authored, not inferred" section with one sentence noting the strip/page grouping exists for the compact pager and is `display: contents` on wide viewports.

- [ ] **Step 4: Wide-mode parity CSS**

In `fab.css`, after the `.fab { … }` block, add:

```css
/* ── Compact grouping (inert on wide viewports) ───────────────────
   The strip and pages exist for the compact pager (see `.fab--compact`
   below). On wide viewports they generate NO boxes, so the tiles are
   direct flex items of `.fab` exactly as before — and flex `order`
   restores the wide bar's visual order (account, repo, share, end),
   which the DOM gives up to group page 2 (share, account) together.
   The circle cap keeps default order 0, so it stays first. */
.fab__strip,
.fab__page { display: contents; }
.fab__tele--account { order: 1; }
.fab__tele--repo    { order: 2; }
.fab__tele--share   { order: 3; }
.fab__tele--end     { order: 4; }
/* The chevron cap is compact-only; wide mode shows the decorative nub. */
.fab__more { display: none; }
```

- [ ] **Step 5: Re-scope and rank `telescope_tiles`**

In `element.rs`, replace `telescope_tiles`:

```rust
/// Collect the `.fab__tele` wrapper tiles, sorted into VISUAL order. The
/// DOM groups tiles by compact page (repo before share/account) while CSS
/// `order` restores the wide bar's visual order — the telescope stagger
/// must follow what the eye sees, not the DOM. A child scan no longer
/// works: the tiles live inside `.fab__strip` > `.fab__page` wrappers.
fn telescope_tiles(fab: &Element) -> Vec<Element> {
    let mut out = Vec::new();
    if let Ok(list) = fab.query_selector_all(".fab__tele") {
        for i in 0..list.length() {
            if let Some(node) = list.item(i)
                && let Ok(el) = node.dyn_into::<Element>()
            {
                out.push(el);
            }
        }
    }
    out.sort_by_key(tile_rank);
    out
}

/// The wide bar's visual position of a tile — must match the CSS `order`
/// values in `fab.css` (account 1, repo 2, share 3, end 4).
fn tile_rank(tile: &Element) -> u8 {
    let cl = tile.class_list();
    if cl.contains("fab__tele--account") {
        0
    } else if cl.contains("fab__tele--repo") {
        1
    } else if cl.contains("fab__tele--share") {
        2
    } else {
        3
    }
}
```

- [ ] **Step 6: Update the wasm fixture and the stylesheet test**

In `element.rs`'s wasm test module, update `fab_host` to the new shape (the drag/gesture tests walk this structure):

```rust
        host.set_inner_html(
            r#"<div class="fab__scrim"></div>
               <div class="fab">
                 <span class="fab__seg fab__cap-l"></span>
                 <div class="fab__strip">
                   <div class="fab__page fab__page--main">
                     <div class="fab__tele fab__tele--repo"><span class="fab__seg fab__repo"></span></div>
                   </div>
                   <div class="fab__page fab__page--more">
                     <div class="fab__tele fab__tele--share"><span class="fab__seg fab__share"></span></div>
                     <div class="fab__tele fab__tele--account"><span class="fab__seg fab__account"></span></div>
                   </div>
                 </div>
                 <div class="fab__tele fab__tele--end">
                   <span class="fab__seg fab__cap-r fab__end" aria-hidden="true"></span>
                   <button type="button" class="fab__seg fab__cap-r fab__more"></button>
                 </div>
               </div>"#,
        );
```

In `logic.rs`'s `mod stylesheet` test, add one line so a stale CSS copy fails:

```rust
        assert!(css.contains(".fab__strip"));
```

- [ ] **Step 7: Run all native tests**

Run: `cargo test -p tonk-fab && cargo check -p tonk-fab --target wasm32-unknown-unknown`
Expected: PASS — new markup tests, existing markup tests (space stamping, telescope wrappers, no tonk-display), stylesheet test.

- [ ] **Step 8: Commit**

```bash
git add rust/tonk-fab/src/markup.rs rust/tonk-fab/src/element.rs rust/tonk-fab/src/fab.css rust/tonk-fab/src/logic.rs
git commit -m "refactor(tonk-fab): group bar tiles into compact pages behind display:contents"
```

---

### Task 5: Compact mode — activation and the scroll-snap pager

**Files:**
- Modify: `rust/tonk-fab/src/element.rs` (new `update_compact_mode`, `expanded_bar_width`, `attach_resize`, `attach_strip_scroll`; `connected_callback` around line 125; `set_telescope` around line 732; `schedule_settle` around line 839)
- Modify: `rust/tonk-fab/src/fab.css` (compact layout rules)

**Interfaces:**
- Consumes: `logic::is_compact` (Task 1), DOM contract (Task 4).
- Produces: `fn update_compact_mode(element: &HtmlElement)` — Task 6's tests may call it; the `fab--compact` class on `.fab` that all compact CSS keys off.

- [ ] **Step 1: Implement mode evaluation in `element.rs`**

Add `is_compact` to the `crate::logic` import list, then add:

```rust
/// Re-evaluate compact mode from the WOULD-BE expanded bar width — the same
/// input whichever mode we are in, so the threshold cannot flap. Called on
/// connect, on guest-window resize, and when the telescope settles (content
/// like a minted invite link can widen the bar without a resize).
fn update_compact_mode(element: &HtmlElement) {
    let Some(fab) = element.query_selector(".fab").ok().flatten() else {
        return;
    };
    let compact = is_compact(expanded_bar_width(&fab), viewport_width());
    if fab.class_list().contains("fab--compact") == compact {
        return;
    }
    // Crossing modes resets transient UI state: menus close (their anchors
    // change shape entirely) and the telescope re-opens expanded with no
    // stale per-tile clamps — a wide-mode collapse leaves inline
    // `max-width: 0` on every tile, which would zero out the compact pages.
    close_menus(element);
    fab.class_list().remove_1("fab--collapsed").ok();
    fab.class_list().add_1("fab--settled").ok();
    for tile in telescope_tiles(&fab) {
        let style = tile.unchecked_ref::<HtmlElement>().style();
        let _ = style.remove_property("max-width");
        let _ = style.remove_property("margin-left");
        let _ = style.remove_property("transition-delay");
        tile.class_list().remove_1("fab__tele--hidden").ok();
    }
    fab.class_list()
        .toggle_with_force("fab--compact", compact)
        .ok();
}

/// The bar's expanded width. Measured directly when expanded; a compact bar
/// is momentarily unclamped within one task — no paint can happen before the
/// classes are restored — the same trick `menu_natural_width` uses on closed
/// menus. Known gap, accepted: a WIDE bar collapsed to its circle
/// under-reports here (its tiles hold inline `max-width: 0` that no class
/// removal lifts), so shrinking the viewport while collapsed-wide defers the
/// flip to compact until the next expand's settle pass re-evaluates — and a
/// collapsed circle always fits anyway.
fn expanded_bar_width(fab: &Element) -> f64 {
    let cl = fab.class_list();
    let was_compact = cl.contains("fab--compact");
    if was_compact {
        cl.remove_1("fab--compact").ok();
    }
    let width = fab.get_bounding_client_rect().width();
    if was_compact {
        cl.add_1("fab--compact").ok();
    }
    width
}

/// Re-evaluate compact mode whenever the guest window resizes. The overlay
/// iframe is pinned full-viewport, so its window size IS the app viewport.
fn attach_resize(element: &HtmlElement) {
    let el = element.clone();
    let on_resize = Closure::<dyn FnMut()>::new(move || update_compact_mode(&el));
    if let Some(win) = window() {
        let target: &web_sys::EventTarget = win.unchecked_ref();
        let _ = target
            .add_event_listener_with_callback("resize", on_resize.as_ref().unchecked_ref());
    }
    on_resize.forget();
}

/// A swipe on the compact strip moves the segments out from under their
/// anchored dropdowns — dismiss them rather than drag them along.
fn attach_strip_scroll(element: &HtmlElement) {
    let Some(strip) = element.query_selector(".fab__strip").ok().flatten() else {
        return;
    };
    let el = element.clone();
    let on_scroll = Closure::<dyn FnMut()>::new(move || close_menus(&el));
    let target: &web_sys::EventTarget = strip.unchecked_ref();
    let _ = target.add_event_listener_with_callback("scroll", on_scroll.as_ref().unchecked_ref());
    on_scroll.forget();
}
```

- [ ] **Step 2: Wire the calls**

In `connected_callback`'s bind block, after `preload_menu_widths(this);` add:

```rust
            attach_resize(this);
            attach_strip_scroll(this);
            update_compact_mode(this);
```

In `schedule_settle`, the settle closure needs the host element for the mode pass. Change its signature and body:

```rust
fn schedule_settle(element: &HtmlElement, fab: &Element, count: usize) {
    let fab_for_settle = fab.clone();
    let el_for_settle = element.clone();
    let settle_once = Closure::<dyn Fn()>::new(move || {
        fab_for_settle.class_list().add_1("fab--settled").ok();
        for tile in telescope_tiles(&fab_for_settle) {
            if tile.class_list().contains("fab__tele--hidden") {
                continue;
            }
            let style = tile.unchecked_ref::<HtmlElement>().style();
            let _ = style.set_property("max-width", "none");
        }
        // Content settling is the moment the bar reaches its true width —
        // the one growth path (invite link, long rename) a resize never
        // sees. Re-check the fit here.
        update_compact_mode(&el_for_settle);
    });
    let settle_fn = settle_once
        .as_ref()
        .unchecked_ref::<js_sys::Function>()
        .clone();
    settle_once.forget();
    let id = set_timeout(&settle_fn, telescope_settle_ms(count) as i32);
    element.dataset().set("settleTimer", &id.to_string()).ok();
}
```

(The existing per-tile unclamp comment block stays; only the `el_for_settle` capture and the trailing call are new.)

- [ ] **Step 3: Compact collapse is CSS-driven — guard `set_telescope`**

At the top of `set_telescope`, before the settle-timer clearing:

```rust
    // Compact collapse is CSS-driven: the strip and the chevron cap
    // transition their own max-width (see `.fab--compact.fab--collapsed`
    // in fab.css). Driving per-tile inline max-widths here would zero out
    // the pages the strip lays out.
    if fab.class_list().contains("fab--compact") {
        fab.class_list()
            .toggle_with_force("fab--collapsed", collapsing)
            .ok();
        return;
    }
```

- [ ] **Step 4: Compact layout CSS**

Add to `fab.css` after the wide-mode parity block from Task 4:

```css
/* ── Compact mode ─────────────────────────────────────────────────
   `fab--compact` is set on `.fab` by element.rs when the EXPANDED bar
   plus both 16px dock insets would overflow the viewport (measured, not
   device-sniffed — a narrow desktop window compacts too). Layout: the
   circle cap stays the fixed left cap, the chevron replaces the end nub
   as the fixed right cap, and the segments page horizontally between
   them in a native scroll-snap strip — swipe momentum for free, and no
   gesture code to fight the drag (which arms only on the circle cap). */
.fab--compact { max-inline-size: calc(100vw - 32px); }
.fab--compact .fab__strip {
  display: flex;
  gap: 2px;
  flex: 0 1 auto;
  min-width: 0;
  /* Numeric so the collapse transition below can interpolate to 0:
     viewport minus circle cap (36) + chevron cap (36) + insets (32) +
     two 2px bar gaps. */
  max-width: calc(100vw - 108px);
  overflow-x: auto;
  scroll-snap-type: x mandatory;
  scrollbar-width: none;
  transition: max-width 0.4s ease;
}
.fab--compact .fab__strip::-webkit-scrollbar { display: none; }
.fab--compact .fab__page {
  display: flex;
  gap: 2px;
  flex: 0 0 auto;
  /* Just under the strip width: the next page's leading edge peeks
     ~24px, signalling there is more to swipe to (no page dots). */
  min-width: calc(100% - 24px);
  scroll-snap-align: start;
}
/* Pages own the sizing now; tiles and segments shrink into them.
   Scoped to `.fab__page` descendants: the circle cap and the chevron
   cap are `.fab__seg`s OUTSIDE the strip and must keep their fixed
   36px sizing — an unscoped `.fab--compact .fab__seg` outranks their
   single-class rules and breaks both caps. */
.fab--compact .fab__page .fab__tele { flex: 1 1 auto; min-width: 0; }
.fab--compact .fab__page .fab__seg { min-width: 0; flex: 1 1 auto; padding: 0 14px; }
/* Order is DOM order inside the strip (page 1 first); clear the
   wide-mode reordering. */
.fab--compact .fab__tele--account,
.fab--compact .fab__tele--repo,
.fab--compact .fab__tele--share { order: 0; }
/* The right cap swaps: nub out, chevron in. */
.fab--compact .fab__end { display: none; }
.fab--compact .fab__more {
  display: inline-flex;
  flex: none;
  inline-size: 36px;
  padding: 0;
  align-items: center;
  justify-content: center;
  border: 0;
  cursor: pointer;
  font: inherit;
}
.fab--compact .fab__more wa-icon { transition: transform 0.2s ease; }
/* Collapse in compact: the strip and chevron retract into the circle
   (CSS transitions, not per-tile inline styles — see set_telescope). */
.fab--compact .fab__tele--end {
  transition: max-width 0.4s ease, margin-left 0.4s ease;
  max-width: 40px;
}
.fab--compact.fab--collapsed .fab__strip,
.fab--compact.fab--collapsed .fab__tele--end {
  max-width: 0;
  margin-left: -2px;
  overflow: hidden;
}
/* Dragging locks the pager so paging and dragging cannot interleave. */
.fab.dragging .fab__strip { overflow: hidden; }
@media (prefers-reduced-motion: reduce) {
  .fab--compact .fab__strip,
  .fab--compact .fab__tele--end,
  .fab--compact .fab__more wa-icon { transition: none; }
}
```

- [ ] **Step 5: Wasm test — compact activation is measured**

Add to `element.rs`'s wasm test module. The test browser's viewport can't be resized from inside the test, so drive the OTHER input: an oversized segment makes the measured expanded width exceed any real viewport (no stylesheet is injected in the fixture, so the class itself changes no geometry and the measurement is deterministic).

```rust
    #[wasm_bindgen_test]
    fn it_compacts_when_the_expanded_bar_cannot_fit() {
        let document = window().expect("window").document().expect("document");
        let host = fab_host();
        document.body().expect("body").append_child(&host).expect("mount");
        let wide = host
            .query_selector(".fab__repo")
            .ok()
            .flatten()
            .expect("repo segment")
            .unchecked_into::<HtmlElement>();
        let _ = wide
            .style()
            .set_property("cssText", "display:inline-block;width:9999px");

        update_compact_mode(&host);
        let fab = host.query_selector(".fab").ok().flatten().expect("bar");
        assert!(
            fab.class_list().contains("fab--compact"),
            "a bar wider than any viewport must compact"
        );

        let _ = wide.style().set_property("cssText", "display:inline-block;width:10px");
        update_compact_mode(&host);
        assert!(
            !fab.class_list().contains("fab--compact"),
            "a bar that fits again must leave compact mode"
        );
        host.remove();
    }
```

- [ ] **Step 6: Build, run tests**

Run: `cargo test -p tonk-fab && cargo test -p tonk-fab --target wasm32-unknown-unknown`
Expected: native PASS; wasm PASS including the new activation test (chromedriver route — if no driver is available locally, flag it in the final report).

- [ ] **Step 7: Manual smoke check (responsive mode)**

Build and serve the app the usual way for this branch, open the browser dev tools responsive mode:
- Wide viewport: bar renders exactly as before (order: circle, account, repo, share, nub); telescope and dropdowns unchanged.
- Narrow (~390px phone width): bar compacts — circle + space name + chevron; swiping the middle pages to share/profile with snap; next page edge peeks.
- Resize across the threshold both ways: mode flips, no flapping, menus closed on the flip.
Record any visual rough edges to iterate on after Task 6 (padding, peek width, snap feel are tuning knobs, not plan changes).

- [ ] **Step 8: Commit**

```bash
git add rust/tonk-fab/src/element.rs rust/tonk-fab/src/fab.css
git commit -m "feat(tonk-fab): adaptive compact mode with a scroll-snap pager"
```

---

### Task 6: The chevron's vertical menu

**Files:**
- Modify: `rust/tonk-fab/src/element.rs` (`attach_gestures` around line 219; `close_menus` around line 648; new `toggle_more_menu`; wasm tests)
- Modify: `rust/tonk-fab/src/fab.css` (menu restack rules; scrim `:has` rule around line 658)

**Interfaces:**
- Consumes: `fab--compact` (Task 5), DOM contract (Task 4).
- Produces: `fab--menu` class on `.fab`; `close_menus` now also clears it (drag promotion and the scrim get dismissal for free).

- [ ] **Step 1: Write the failing wasm tests**

Add to `element.rs`'s wasm test module (the fixture already has `.fab__more` from Task 4):

```rust
    fn has_menu(host: &HtmlElement) -> bool {
        host.query_selector(".fab")
            .ok()
            .flatten()
            .map(|fab| fab.class_list().contains("fab--menu"))
            .unwrap_or(false)
    }

    fn bubbling_click() -> web_sys::MouseEvent {
        let init = web_sys::MouseEventInit::new();
        init.set_bubbles(true);
        web_sys::MouseEvent::new_with_mouse_event_init_dict("click", &init).expect("click event")
    }

    #[wasm_bindgen_test]
    fn it_toggles_the_vertical_menu_from_the_chevron() {
        let document = window().expect("window").document().expect("document");
        let host = fab_host();
        attach_gestures(&host);
        document.body().expect("body").append_child(&host).expect("mount");

        let chevron = host.query_selector(".fab__more").ok().flatten().expect("chevron");
        chevron.dispatch_event(&bubbling_click()).expect("dispatch");
        assert!(has_menu(&host), "a chevron click opens the vertical menu");

        chevron.dispatch_event(&bubbling_click()).expect("dispatch");
        assert!(!has_menu(&host), "a second click closes it again");
        host.remove();
    }

    #[wasm_bindgen_test]
    fn it_dismisses_the_vertical_menu_from_the_curtain() {
        let document = window().expect("window").document().expect("document");
        let host = fab_host();
        attach_gestures(&host);
        document.body().expect("body").append_child(&host).expect("mount");

        host.query_selector(".fab")
            .ok()
            .flatten()
            .expect("bar")
            .class_list()
            .add_1("fab--menu")
            .expect("open");
        let scrim = host.query_selector(".fab__scrim").ok().flatten().expect("curtain");
        scrim.dispatch_event(&bubbling_click()).expect("dispatch");
        assert!(!has_menu(&host), "the click-away curtain closes the vertical menu");
        host.remove();
    }
```

- [ ] **Step 2: Run wasm tests to verify the new ones fail**

Run: `cargo test -p tonk-fab --target wasm32-unknown-unknown`
(Chromedriver/Chrome route; if no driver is available locally, proceed and flag it in the final report — the native suite still gates.)
Expected: the two new tests FAIL (no `fab--menu` handling yet); the three existing wasm tests PASS.

- [ ] **Step 3: Implement the gesture and dismissal**

In `attach_gestures`'s click closure, insert a chevron branch right after the scrim branch (before the `.fab__cap-l` branch):

```rust
        } else if t.closest(".fab__more").ok().flatten().is_some() {
            toggle_more_menu(&el_click);
```

Add the function near `toggle_menu`:

```rust
/// Toggle the compact vertical menu (`fab--menu` on `.fab`): every control
/// stacked full-width, anchored to the bar like the dropdowns. Opening it
/// first closes any open dropdown — the vertical menu owns the whole field,
/// and the dropdowns re-open INSIDE it as inline accordions (CSS).
fn toggle_more_menu(element: &HtmlElement) {
    let Some(fab) = element.query_selector(".fab").ok().flatten() else {
        return;
    };
    let opening = !fab.class_list().contains("fab--menu");
    close_menus(element);
    fab.class_list()
        .toggle_with_force("fab--menu", opening)
        .ok();
}
```

Extend `close_menus` (this gives the scrim, drag promotion, mode switches, and strip scrolls dismissal of the vertical menu for free — every existing caller wants it):

```rust
fn close_menus(el: &HtmlElement) {
    for sel in MENU_SEGMENTS {
        if let Some(seg) = el.query_selector(sel).ok().flatten() {
            seg.class_list().remove_1("is-open").ok();
        }
    }
    if let Some(fab) = el.query_selector(".fab").ok().flatten() {
        fab.class_list().remove_1("fab--menu").ok();
    }
}
```

- [ ] **Step 4: Menu restack CSS**

Add to `fab.css` after the compact block, and extend the scrim rule:

```css
/* ── The chevron's vertical menu (compact only in practice) ───────
   `fab--menu` pops the strip out of the bar as an absolutely-anchored
   vertical stack — the same anchoring the dropdowns use (away from the
   docked edge, dock-keyed), so it always unfolds into view. The bar
   itself shrinks to its two caps while the menu is up, iOS-style. */
.fab--menu .fab__strip {
  position: absolute;
  left: 0;
  right: 0;
  display: flex;
  flex-direction: column;
  gap: 7px;
  max-width: none;
  overflow: visible;
  z-index: 100;
}
.fab-dock-top .fab--menu .fab__strip { top: 100%; margin-top: 7px; }
.fab-dock-bottom .fab--menu .fab__strip { bottom: 100%; margin-bottom: 7px; }
/* Pages dissolve; every tile is its own full-width row. Scoped to the
   strip: the end tile (chevron cap) sits outside it and keeps its bar
   styling while the menu is up. */
.fab--menu .fab__page { display: contents; }
.fab--menu .fab__strip .fab__tele {
  max-width: none !important;
  overflow: visible;
  min-width: 0;
}
.fab--menu .fab__strip .fab__seg {
  width: 100%;
  box-sizing: border-box;
  block-size: 36px;
  border-radius: 2px;
  flex: none;
}
/* An open dropdown renders INLINE as an accordion row under its
   segment instead of floating: the segment's tile stacks them. */
.fab--menu .fab__tele--repo,
.fab--menu .fab__tele--share { display: flex; flex-direction: column; gap: 7px; }
.fab--menu .fab__repo.is-open .fab__menu,
.fab--menu .fab__share.is-open .fab__share-menu {
  position: static;
  margin: 7px 0 0;
  width: 100%;
}
/* The chevron flips while the menu is up. */
.fab--menu .fab__more wa-icon { transform: rotate(90deg); }
```

And extend the existing scrim activation rule:

```css
tonk-fab:has(.is-open) .fab__scrim,
tonk-fab:has(.fab--menu) .fab__scrim { pointer-events: auto; }
```

- [ ] **Step 5: Run all tests**

Run: `cargo test -p tonk-fab && cargo test -p tonk-fab --target wasm32-unknown-unknown`
Expected: native PASS; all five wasm tests PASS.

- [ ] **Step 6: Manual smoke check**

In responsive mode at phone width: chevron tap opens the vertical stack (rows: space name, share, profile), chevron rotates; tapping the space-name row unfolds the switcher inline; tap-away on the page closes everything; dragging the circle still works with the menu auto-closing. Note polish items for the iteration pass.

- [ ] **Step 7: Commit**

```bash
git add rust/tonk-fab/src/element.rs rust/tonk-fab/src/fab.css
git commit -m "feat(tonk-fab): chevron-toggled vertical menu for the compact pill"
```

---

### Task 7: Gate and device pass

**Files:**
- None expected (fixes only if the gate finds issues).

- [ ] **Step 1: Run the full lint gate**

Run: `cargo clippy --workspace --all-targets --all-features` and `cargo fmt --check` (both native, from the repo root).
Expected: clean. Fix any findings in place (the `--all-features` gate compiles integration tests that per-crate clippy misses).

- [ ] **Step 2: Full test sweep**

Run: `cargo test -p tonk-fab && cargo test -p tonk-fab --target wasm32-unknown-unknown`
Expected: PASS.

- [ ] **Step 3: Manual verification checklist**

Serve the app; verify and record results honestly:
- Desktop wide: pixel parity with `main` behaviour (order, telescope, dropdowns, drag + corner snap, dock persistence across reload).
- Desktop narrow window: compact activates; drag cannot push the bar off-screen; corners still snap and persist.
- Phone (real device or touch emulation): drag from the circle is reliable (no scroll fights, ≥44px target, taps still toggle); swipe pages with momentum; chevron menu opens/dismisses; reduced-motion honoured.

- [ ] **Step 4: Commit any gate fixes**

```bash
git add -u
git commit -m "chore(tonk-fab): appease the workspace lint gate"
```

(Skip if the gate was already clean.)
