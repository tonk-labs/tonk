# Clicking links inside a spot — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make links inside a spot's sandboxed iframe clickable — external `http(s)`, `mailto:`/`tel:`, `target="_blank"`, and modified (cmd/ctrl/shift/middle) clicks — opening in a new tab, with external destinations gated by a confirm dialog rendered by the trusted top page.

**Architecture:** Two stacked PRs. **PR 1** generalizes page-only effects so they forward up the frame chain until they reach the top page, fixing a latent depth-2 `navigate` bug and building the registry `navigate.rs:9-14` predicted. **PR 2** adds `open` as the third page effect: the guest classifies a click and relays the raw href; the top page — the single policy point — resolves it, allowlists the scheme with the browser's URL parser, and either opens a tab silently (same origin) or confirms first (external).

**Tech Stack:** Rust + `wasm-bindgen` / `web-sys`, custom elements, `MessageChannel` port bridge, native `<dialog>`, Web Awesome CSS custom properties (theme only, no components).

**Spec:** `docs/superpowers/specs/2026-07-16-spot-link-clicks-design.md`

## Global Constraints

- **All tests use `#[dialog_common::test]`**, named `it_does_x`, grouped by behaviour. Never `#[wasm_bindgen_test]` directly.
- **Browser-only test modules** are gated `#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]` with `wasm_bindgen_test_configure!(run_in_browser);`, matching `navigate.rs:141-146` and `title.rs:29-34`.
- **No `mod.rs`** — use `foo.rs` + `foo/` form.
- **No emojis** in code, commits, or output.
- **Conventional Commits**: `type(scope): subject`, imperative, lowercase, no trailing period, subject under ~72 chars.
- **Never interpolate a URL or host into HTML.** `set_text_content` only. These strings are attacker-controlled and the dialog renders on the real origin.
- **Never prefix-match a scheme.** Use `web_sys::Url` only.
- **PRs target `origin/staging`**, not `main`.
- **Dialog copy is exactly** `Open in a new tab?` / `Cancel` / `Open`. Not "Leave this spot?" — with `_blank` + `noopener` the user does not leave.
- **Lint gate** is workspace `cargo clippy --all-targets --all-features` plus `cargo fmt --check` (via `nix flake check`). `--all-features` compiles integration tests, so a per-crate check can pass while the gate fails.

## Terminology warning

`rust/tonk-host/src/depth.rs` already exists and means something **completely different** — DOM consumer-element nesting within one document, annotated onto event details. It has nothing to do with iframe nesting. **Do not name anything in this plan `depth`**, and do not modify `depth.rs`.

---

## File Structure

**PR 1 — forwarding**

| File | Responsibility |
|---|---|
| Create `rust/tonk-host/src/page_effect.rs` | The one rule: "am I a guest? then forward." Nothing else. |
| Modify `rust/tonk-host/src/lib.rs:71-77` | Register the module; export `open_external` (PR 2). |
| Modify `rust/tonk-host/src/navigate.rs:98` | `navigate_to` forwards before performing. |
| Modify `rust/tonk-host/src/title.rs:19` | `set_title` forwards before performing. |

**PR 2 — the `open` effect**

| File | Responsibility |
|---|---|
| Create `rust/tonk-host/src/open.rs` | Resolve, classify, allowlist, dialog, open. The single policy point. |
| Modify `rust/tonk-host/Cargo.toml` | Add `console` + `HtmlDialogElement` web-sys features. |
| Modify `rust/tonk-portal/src/bridge.rs:241` | `window.tonk.open` in the guest bootstrap. |
| Modify `rust/tonk-portal/src/bridge.rs:1189` | Dispatch `"open"`; add `handle_open` / `open_href`. |
| Modify `rust/tonk-guest/src/guest_host.rs:81-106` | Classify a click into Navigate / Open / Ignore; add `auxclick`. |
| Modify `docs/superpowers/specs/...-design.md` | Correct the error-reporting claim (see Task 9). |

**Why forwarding lives in the effect functions, not the dispatcher.** `handle_navigate` (`bridge.rs:1430`) stays five lines and unchanged. Putting the rule inside `tonk_host::navigate_to` / `set_title` / `open_external` means *every* caller inherits it — the bridge dispatcher, the service-worker message listener (`navigate.rs:39`), and any future one — and the chain composes for free: each hop's dispatcher calls the effect fn, which forwards again until a document without `window.tonk` performs it.

---

## PR 1 — page-effect forwarding

### Task 1: The forwarding rule

**Files:**
- Create: `rust/tonk-host/src/page_effect.rs`
- Modify: `rust/tonk-host/src/lib.rs`

**Interfaces:**
- Produces: `pub(crate) fn forward(method: &str, arg: &str) -> bool` — `true` when this document is a portal guest and the effect was posted to the parent (caller must return early); `false` when this document is the page and must perform the effect itself.

- [ ] **Step 1: Register the module**

In `rust/tonk-host/src/lib.rs`, immediately before the `mod navigate;` block at line 70-71, add:

```rust
#[cfg(target_arch = "wasm32")]
mod page_effect;
```

- [ ] **Step 2: Write the failing test**

Create `rust/tonk-host/src/page_effect.rs` containing ONLY the test module for now (the implementation lands in Step 4):

```rust
//! Placeholder — implementation lands in Step 4.

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod tests {
    use super::*;
    use js_sys::{Array, Function, Object, Reflect};
    use wasm_bindgen::JsValue;
    use wasm_bindgen_test::wasm_bindgen_test_configure;

    wasm_bindgen_test_configure!(run_in_browser);

    /// Install a stub `window.tonk` whose `method` records its argument into
    /// a JS array, and hand back that array. Every test that calls this MUST
    /// call `clear_tonk()` before returning: `window` is shared across every
    /// test in this wasm module, and a leaked `window.tonk` would make each
    /// page effect silently forward into the void for every test that runs
    /// afterwards.
    fn install_tonk(method: &str) -> Array {
        let calls = Array::new();
        let recorder = {
            let calls = calls.clone();
            Closure::wrap(Box::new(move |value: JsValue| {
                calls.push(&value);
            }) as Box<dyn FnMut(JsValue)>)
        };
        let tonk = Object::new();
        let _ = Reflect::set(
            &tonk,
            &JsValue::from_str(method),
            recorder.as_ref().unchecked_ref::<Function>(),
        );
        // The stub must outlive this fn; the test clears `window.tonk` instead.
        recorder.forget();
        let win = web_sys::window().expect("a window in the test harness");
        let _ = Reflect::set(&win, &JsValue::from_str("tonk"), &tonk);
        calls
    }

    fn clear_tonk() {
        let win = web_sys::window().expect("a window in the test harness");
        let _ = Reflect::delete_property(
            win.unchecked_ref::<Object>(),
            &JsValue::from_str("tonk"),
        );
    }

    /// A document with a `window.tonk` is a portal guest: the effect is posted
    /// to the parent and the caller is told to stop.
    #[dialog_common::test]
    async fn it_forwards_when_this_document_is_a_guest() {
        let calls = install_tonk("navigate");

        let forwarded = forward("navigate", "/space/abc");

        assert!(forwarded, "a guest should forward the effect");
        assert_eq!(calls.length(), 1, "the parent should have been called once");
        assert_eq!(
            calls.get(0).as_string(),
            Some("/space/abc".to_owned()),
            "the href should reach the parent verbatim"
        );
        clear_tonk();
    }

    /// The top page has no `window.tonk` — it must perform the effect itself.
    #[dialog_common::test]
    async fn it_does_not_forward_when_this_document_is_the_page() {
        clear_tonk();

        assert!(
            !forward("navigate", "/space/abc"),
            "the top page should perform the effect, not forward it"
        );
    }

    /// A guest whose bridge lacks the method cannot forward. Reporting `false`
    /// would make the caller perform a page effect inside an iframe; reporting
    /// `true` drops it. Dropping is correct — performing is the bug this whole
    /// module exists to prevent.
    #[dialog_common::test]
    async fn it_drops_rather_than_performs_when_the_bridge_lacks_the_method() {
        let _calls = install_tonk("navigate");

        assert!(
            forward("setTitle", "Notes — Tonk"),
            "a guest missing the method should still not perform locally"
        );
        clear_tonk();
    }
}
```

- [ ] **Step 3: Run the test to verify it fails**

Run: `cargo test -p tonk-host --target wasm32-unknown-unknown page_effect`
Expected: FAIL — `cannot find function 'forward' in this scope`.

> **Note on running wasm tests locally:** per `project_wasm_tests_need_safari_automation` / `project_wasm_tests_chrome_route`, these need either `safaridriver` enabled (run with `-j1`) or Chrome at the default `/Applications` path with a major-matched chromedriver. If neither is available, run the compile check (`cargo clippy -p tonk-host --target wasm32-unknown-unknown --all-targets`) and defer execution to CI.

- [ ] **Step 4: Write the implementation**

Replace the placeholder doc comment at the top of `rust/tonk-host/src/page_effect.rs` with the implementation, keeping the test module below it:

```rust
//! Forwarding for page-only effects.
//!
//! Some effects can only happen on the top page: moving `location`,
//! setting `document.title`, opening a tab. The chrome that wants them
//! runs in a sealed guest, so it posts a message over the portal bridge
//! — and the bridge dispatcher runs in the guest's PARENT, which is not
//! necessarily the page.
//!
//! Spot content is a guest inside the profile chrome, which is itself a
//! guest. A message from spot content is dispatched one hop up, in an
//! opaque-origin `about:srcdoc` document, where performing the effect
//! either throws or corrupts the wrong frame. See the design spec.
//!
//! So every page effect asks this first: am I myself a guest? If so the
//! effect is re-posted to the parent, which asks the same question. The
//! recursion terminates at the page, in O(frames) hops. It is the shape
//! `bridge::context_origin` already uses for "the real value lives N
//! frames up".
//!
//! `navigate.rs:9-14` called for exactly this registry when a second page
//! effect appeared. `title` arrived as a parallel special case instead;
//! this is the generalization, and `open` is the third member.
//!
//! NOTE: unrelated to `depth.rs`, which counts DOM consumer nesting
//! inside a single document.

use js_sys::{Function, Reflect};
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use web_sys::window;

/// Post a page effect to the parent when this document is a portal guest.
///
/// Returns `true` when the caller must stop — either the effect was
/// forwarded, or this is a guest whose bridge could not carry it (in which
/// case dropping it is still correct: a guest must never perform a page
/// effect on its own document). Returns `false` only for a real page, which
/// must perform the effect itself.
///
/// The discriminator is `window.tonk`, assigned at exactly one place —
/// `BOOTSTRAP_JS` in `tonk-portal/src/bridge.rs`, which only ever runs in a
/// guest's `srcdoc`. No Rust assigns it and the top page never has one, so
/// its presence means precisely "I am a portal guest with a bridge to my
/// parent". Deliberately NOT `window === window.top`, which encodes "I am
/// the outermost frame" — a different claim that would break if the Tonk
/// page were ever itself embedded.
///
/// If `window.tonk` is ever installed on the top page, every page effect
/// silently no-ops. That is the assumption to check first if they all stop
/// working at once.
pub(crate) fn forward(method: &str, arg: &str) -> bool {
    let Some(win) = window() else {
        return false;
    };
    let Ok(tonk) = Reflect::get(&win, &JsValue::from_str("tonk")) else {
        return false;
    };
    if tonk.is_undefined() || tonk.is_null() {
        return false;
    }
    // A guest from here on: whatever happens, do not perform locally.
    if let Ok(method) = Reflect::get(&tonk, &JsValue::from_str(method))
        && let Ok(method) = method.dyn_into::<Function>()
    {
        let _ = method.call1(&tonk, &JsValue::from_str(arg));
    }
    true
}
```

Add the `Closure` import the test module needs — at the top of the `mod tests` block, alongside the existing imports:

```rust
    use wasm_bindgen::closure::Closure;
    use wasm_bindgen::JsCast;
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p tonk-host --target wasm32-unknown-unknown page_effect`
Expected: PASS — 3 tests.

- [ ] **Step 6: Commit**

```bash
git add rust/tonk-host/src/page_effect.rs rust/tonk-host/src/lib.rs
git commit -m "feat(tonk-host): forward page-only effects to the page

A guest's bridge dispatcher runs in its parent, which is not necessarily
the top page. Spot content is a guest inside the profile chrome, so its
page effects were performed one hop up, in an opaque-origin document.

Ask 'am I a guest?' and re-post if so, terminating at the page in
O(frames) hops. Builds the registry navigate.rs called for."
```

---

### Task 2: `navigate_to` forwards

**Files:**
- Modify: `rust/tonk-host/src/navigate.rs:98-134` (the fn) and `:141-146` (test module)

**Interfaces:**
- Consumes: `page_effect::forward` from Task 1.
- Produces: no signature change — `pub fn navigate_to(href: &str)` still.

- [ ] **Step 1: Write the failing test**

In `rust/tonk-host/src/navigate.rs`, inside the existing `mod tests` block, add these imports at the top of the block:

```rust
    use js_sys::{Array, Function, Object, Reflect};
    use wasm_bindgen::JsCast;
    use wasm_bindgen::closure::Closure;
```

and add this test after `it_recognises_only_a_sync_message`:

```rust
    /// Install a stub `window.tonk.navigate` recording its argument. See the
    /// note in `page_effect.rs`: `window` is shared across the whole wasm
    /// test module, so this MUST be cleared before the test returns.
    fn install_navigate_stub() -> Array {
        let calls = Array::new();
        let recorder = {
            let calls = calls.clone();
            Closure::wrap(Box::new(move |value: JsValue| {
                calls.push(&value);
            }) as Box<dyn FnMut(JsValue)>)
        };
        let tonk = Object::new();
        let _ = Reflect::set(
            &tonk,
            &JsValue::from_str("navigate"),
            recorder.as_ref().unchecked_ref::<Function>(),
        );
        recorder.forget();
        let win = window().expect("a window in the test harness");
        let _ = Reflect::set(&win, &JsValue::from_str("tonk"), &tonk);
        calls
    }

    fn clear_tonk() {
        let win = window().expect("a window in the test harness");
        let _ = Reflect::delete_property(
            win.unchecked_ref::<Object>(),
            &JsValue::from_str("tonk"),
        );
    }

    /// In a guest, `navigate_to` posts to the parent instead of touching this
    /// document's history. This is the one navigation we CAN assert directly:
    /// forwarding means nothing actually navigates, so the harness survives.
    #[dialog_common::test]
    async fn it_forwards_a_navigation_from_a_guest_instead_of_performing_it() {
        let before = window()
            .expect("a window in the test harness")
            .location()
            .href()
            .expect("a location href");
        let calls = install_navigate_stub();

        navigate_to("/space/forwarded");

        assert_eq!(calls.length(), 1, "the parent should have been called once");
        assert_eq!(
            calls.get(0).as_string(),
            Some("/space/forwarded".to_owned()),
            "the href should reach the parent verbatim"
        );
        let after = window()
            .expect("a window in the test harness")
            .location()
            .href()
            .expect("a location href");
        assert_eq!(
            before, after,
            "a forwarded navigation must not move this document"
        );
        clear_tonk();
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p tonk-host --target wasm32-unknown-unknown navigate`
Expected: FAIL — `calls.length()` is 0, and the assertion message reads "the parent should have been called once". (The unmodified `navigate_to` calls `pushState` instead of forwarding.)

- [ ] **Step 3: Write the implementation**

In `rust/tonk-host/src/navigate.rs`, replace the doc comment and opening lines of `navigate_to` (currently at `:90-103`) so the fn begins:

```rust
/// Navigate to `href` WITHOUT reloading: push it onto history and fire
/// `popstate` so the top-level `<tonk-site>` re-resolves. The path change then
/// updates the tab's site in the overlay, whose subscription re-renders the
/// view — the route change propagates as a data change, not a page load. Falls
/// back to a real `location.assign` only if history isn't available.
///
/// In a guest this forwards to the parent instead (see `page_effect`): a
/// guest's document is `about:srcdoc` at an opaque origin, where `pushState`
/// to a real URL throws and the `location.assign` fallback below would load
/// the whole app INSIDE the iframe.
///
/// Public: the portal bridge performs a guest's relayed link click through
/// this too, so an in-guest navigation stays a client-side route change.
pub fn navigate_to(href: &str) {
    use wasm_bindgen::JsValue;
    if crate::page_effect::forward("navigate", href) {
        return;
    }
    let Some(win) = window() else {
        return;
    };
```

Leave the rest of the function (the same-URL guard and the `pushState`/`assign` body, `:110-133`) exactly as it is.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p tonk-host --target wasm32-unknown-unknown navigate`
Expected: PASS — 3 tests (the two existing parse tests plus the new forwarding test).

- [ ] **Step 5: Commit**

```bash
git add rust/tonk-host/src/navigate.rs
git commit -m "fix(tonk-host): forward navigation out of a guest

navigate_to ran wherever the dispatcher lived. From a depth-2 guest that
is an opaque about:srcdoc document: pushState throws SecurityError, which
.is_ok() folded into the 'no history access' branch, and the fallback
location.assign loaded the whole app inside the chrome iframe.

Unreachable today only because every anchor in the product sits one hop
from the page. Forward instead."
```

---

### Task 3: `set_title` forwards

**Files:**
- Modify: `rust/tonk-host/src/title.rs:19-27` (the fn) and `:29-63` (test module)

**Interfaces:**
- Consumes: `page_effect::forward` from Task 1.
- Produces: no signature change — `pub fn set_title(title: &str)` still.

- [ ] **Step 1: Write the failing test**

In `rust/tonk-host/src/title.rs`, inside the existing `mod tests` block, add these imports at the top of the block:

```rust
    use js_sys::{Array, Function, Object, Reflect};
    use wasm_bindgen::JsCast;
    use wasm_bindgen::JsValue;
    use wasm_bindgen::closure::Closure;
```

and add this test after `it_sets_a_non_empty_title_and_ignores_an_empty_one`:

```rust
    /// Install a stub `window.tonk.setTitle` recording its argument. MUST be
    /// cleared before the test returns — `window` is shared across the whole
    /// wasm test module, and a leaked stub would make the test above forward
    /// its title instead of setting it.
    fn install_title_stub() -> Array {
        let calls = Array::new();
        let recorder = {
            let calls = calls.clone();
            Closure::wrap(Box::new(move |value: JsValue| {
                calls.push(&value);
            }) as Box<dyn FnMut(JsValue)>)
        };
        let tonk = Object::new();
        let _ = Reflect::set(
            &tonk,
            &JsValue::from_str("setTitle"),
            recorder.as_ref().unchecked_ref::<Function>(),
        );
        recorder.forget();
        let win = web_sys::window().expect("a window in the test harness");
        let _ = Reflect::set(&win, &JsValue::from_str("tonk"), &tonk);
        calls
    }

    fn clear_tonk() {
        let win = web_sys::window().expect("a window in the test harness");
        let _ = Reflect::delete_property(
            win.unchecked_ref::<Object>(),
            &JsValue::from_str("tonk"),
        );
    }

    /// In a guest, a title is posted to the parent rather than written to the
    /// guest's own (invisible) document title.
    #[dialog_common::test]
    async fn it_forwards_a_title_from_a_guest_instead_of_setting_it() {
        set_title("Before — Tonk");
        let calls = install_title_stub();

        set_title("Forwarded — Tonk");

        assert_eq!(calls.length(), 1, "the parent should have been called once");
        assert_eq!(
            calls.get(0).as_string(),
            Some("Forwarded — Tonk".to_owned()),
            "the text should reach the parent verbatim"
        );
        assert_eq!(
            document_title(),
            "Before — Tonk",
            "a forwarded title must not retitle this document"
        );
        clear_tonk();
    }

    /// The empty guard runs BEFORE forwarding: a blank render is dropped at
    /// its source rather than posted up the frame chain for each parent to
    /// re-drop.
    #[dialog_common::test]
    async fn it_does_not_forward_an_empty_title() {
        let calls = install_title_stub();

        set_title("");

        assert_eq!(calls.length(), 0, "an empty title should not be forwarded");
        clear_tonk();
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p tonk-host --target wasm32-unknown-unknown title`
Expected: FAIL — `it_forwards_a_title_from_a_guest_instead_of_setting_it` reports "the parent should have been called once" (length 0). `it_does_not_forward_an_empty_title` already passes.

- [ ] **Step 3: Write the implementation**

In `rust/tonk-host/src/title.rs`, replace `set_title` (`:14-27`) with:

```rust
/// Set the page's tab title.
///
/// An empty title is a no-op, not a blank tab: a view renders a blank
/// `{name}` until the fact resolves, and letting that through would
/// wipe a title that was already correct. The guard runs before
/// forwarding so a blank render dies at its source rather than being
/// posted up the frame chain for each parent to re-drop.
///
/// In a guest this forwards to the parent instead (see `page_effect`):
/// `document.title` in a sealed iframe is invisible, so writing it there
/// would silently do nothing.
pub fn set_title(title: &str) {
    if title.is_empty() {
        return;
    }
    if crate::page_effect::forward("setTitle", title) {
        return;
    }
    let Some(document) = window().and_then(|w| w.document()) else {
        return;
    };
    document.set_title(title);
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p tonk-host --target wasm32-unknown-unknown title`
Expected: PASS — 3 tests.

- [ ] **Step 5: Run the whole crate's tests and the lint gate**

```bash
cargo test -p tonk-host --target wasm32-unknown-unknown
cargo clippy -p tonk-host --target wasm32-unknown-unknown --all-targets -- -D warnings
cargo fmt --check
```
Expected: all PASS. If `it_sets_a_non_empty_title_and_ignores_an_empty_one` fails here but passed alone, a test leaked `window.tonk` — find the missing `clear_tonk()`.

- [ ] **Step 6: Commit**

```bash
git add rust/tonk-host/src/title.rs
git commit -m "feat(tonk-host): forward titles out of a guest

Unblocks a title set from a nested guest, which the tab-title spec
deferred as needing a relay. Sub-route titles remain unbuilt."
```

**PR 1 ends here.** Open it against `origin/staging` with a body explaining the latent depth-2 bug (spec section "Latent bug: depth-2 navigation is already broken").

---

## PR 2 — the `open` effect

### Task 4: Destination classification

**Files:**
- Create: `rust/tonk-host/src/open.rs`
- Modify: `rust/tonk-host/src/lib.rs`
- Modify: `rust/tonk-host/Cargo.toml`

**Interfaces:**
- Consumes: `page_effect::forward` (Task 1) — used in Task 5, not here.
- Produces:
  - `pub(crate) enum Destination { SameOrigin(String), External { url: String, host: String }, Rejected }`
  - `pub(crate) fn classify(href: &str, base: &str, page_origin: &str) -> Destination`

- [ ] **Step 1: Add the web-sys features**

In `rust/tonk-host/Cargo.toml`, add to the `web-sys` `features` list, keeping it alphabetical within its existing grouping: `"console"` (after `"CustomEventInit"`) and `"HtmlDialogElement"` (after `"HtmlElement"`).

The list must end up containing at least: `"console"`, `"Document"`, `"Element"`, `"HtmlDialogElement"`, `"HtmlElement"`, `"HtmlHeadElement"`, `"Location"`, `"Node"`, `"Url"`, `"Window"`.

`HtmlHeadElement` is required by `Document::head()`, which Task 5's `ensure_styles` calls — it is easy to miss because `head()` looks like it needs nothing.

`HtmlAnchorElement` is deliberately NOT added — Task 5 creates the anchor via `create_element("a")` + `set_attribute`, and clicks it through `HtmlElement::click()`, which is already enabled.

- [ ] **Step 2: Register the module**

In `rust/tonk-host/src/lib.rs`, after the `mod navigate; pub use navigate::navigate_to;` block (lines 70-73), add:

```rust
#[cfg(target_arch = "wasm32")]
mod open;
#[cfg(target_arch = "wasm32")]
pub use open::open_external;
```

- [ ] **Step 3: Write the failing test**

Create `rust/tonk-host/src/open.rs`:

```rust
//! Placeholder — implementation lands in Step 5.

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod tests {
    use super::*;
    use wasm_bindgen_test::wasm_bindgen_test_configure;

    wasm_bindgen_test_configure!(run_in_browser);

    const BASE: &str = "https://tonk.example/space/abc";
    const ORIGIN: &str = "https://tonk.example";

    fn classified(href: &str) -> Destination {
        classify(href, BASE, ORIGIN)
    }

    /// A path resolves against the page and lands on our own origin, so it
    /// opens with no dialog — there is nothing to warn about.
    #[dialog_common::test]
    async fn it_treats_our_own_origin_as_same_origin() {
        assert_eq!(
            classified("/space/def"),
            Destination::SameOrigin("https://tonk.example/space/def".to_owned()),
            "an in-app path should resolve against the page origin"
        );
        assert_eq!(
            classified("https://tonk.example/space/def"),
            Destination::SameOrigin("https://tonk.example/space/def".to_owned()),
            "an absolute URL on our origin is still same-origin"
        );
    }

    /// A different origin is announced before it opens.
    #[dialog_common::test]
    async fn it_treats_another_origin_as_external() {
        assert_eq!(
            classified("https://example.com/docs/x"),
            Destination::External {
                url: "https://example.com/docs/x".to_owned(),
                host: "example.com".to_owned(),
            },
            "a cross-origin https URL should be announced"
        );
        assert_eq!(
            classified("//example.com/docs/x"),
            Destination::External {
                url: "https://example.com/docs/x".to_owned(),
                host: "example.com".to_owned(),
            },
            "a protocol-relative URL inherits the page scheme and is external"
        );
        assert_eq!(
            classified("http://example.com/"),
            Destination::External {
                url: "http://example.com/".to_owned(),
                host: "example.com".to_owned(),
            },
            "plain http is allowed, and is external even on the same host"
        );
    }

    /// `mailto:`/`tel:` have no origin. They are external by construction, and
    /// the address stands in for the host in the dialog.
    #[dialog_common::test]
    async fn it_shows_the_address_as_the_host_for_mail_and_tel() {
        assert_eq!(
            classified("mailto:someone@example.com"),
            Destination::External {
                url: "mailto:someone@example.com".to_owned(),
                host: "someone@example.com".to_owned(),
            },
            "a mailto address should be what the dialog names"
        );
        assert_eq!(
            classified("tel:+15551234567"),
            Destination::External {
                url: "tel:+15551234567".to_owned(),
                host: "+15551234567".to_owned(),
            },
            "a tel number should be what the dialog names"
        );
    }

    /// THE security test. A relayed href reaches the trusted origin, so a
    /// scheme outside the allowlist must never become an openable URL — a
    /// `javascript:` URL opened here would execute on the real origin and
    /// defeat the sandbox the whole architecture exists to maintain.
    ///
    /// Every evasion below defeats a `starts_with` check and is normalised
    /// away by the URL parser, which is exactly why we parse.
    #[dialog_common::test]
    async fn it_rejects_every_scheme_outside_the_allowlist() {
        for href in [
            "javascript:alert(1)",
            "JaVaScRiPt:alert(1)",
            "  javascript:alert(1)",
            "java\nscript:alert(1)",
            "java\tscript:alert(1)",
            "\u{0000}javascript:alert(1)",
            "data:text/html,<script>alert(1)</script>",
            "blob:https://tonk.example/abc",
            "file:///etc/passwd",
            "vbscript:msgbox(1)",
            "about:blank",
            "ws://tonk.example/socket",
        ] {
            assert_eq!(
                classified(href),
                Destination::Rejected,
                "`{href}` must never be openable"
            );
        }
    }

    /// An unparseable href is rejected rather than guessed at.
    #[dialog_common::test]
    async fn it_rejects_an_unparseable_href() {
        assert_eq!(
            classified("http://"),
            Destination::Rejected,
            "an href the URL parser rejects should be rejected"
        );
    }
}
```

- [ ] **Step 4: Run the test to verify it fails**

Run: `cargo test -p tonk-host --target wasm32-unknown-unknown open`
Expected: FAIL — `cannot find type 'Destination' in this scope`.

- [ ] **Step 5: Write the implementation**

Replace the placeholder doc comment at the top of `rust/tonk-host/src/open.rs` with:

```rust
//! Opening a link on a guest's behalf — the single policy point.
//!
//! A click inside a sealed guest cannot open anything: the sandbox is
//! `allow-scripts allow-forms`, with no `allow-popups` and no
//! `allow-top-navigation`. The guest relays the raw href and the page
//! decides here.
//!
//! The href is ATTACKER-CONTROLLED. Spot content is data: views and
//! components are facts a collaborator or an agent can assert into a
//! space. This module is where an untrusted string meets the real
//! origin, so two rules are absolute:
//!
//! 1. Parse, never prefix-match. `javascript:` reaching an anchor would
//!    execute on the real origin and defeat the sandbox entirely, and
//!    `JaVaScRiPt:` / leading whitespace / embedded newlines all defeat
//!    string comparison. The URL parser normalises every one of them
//!    into a canonical `protocol`.
//! 2. Never interpolate the URL or host into HTML. Text nodes only.
//!
//! The dialog gates LEAVING THE ORIGIN, not opening a tab: a cmd-clicked
//! in-app link opens silently, an external link is always announced.

use crate::page_effect;
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use wasm_bindgen::closure::Closure;
use web_sys::{Document, Element, HtmlDialogElement, HtmlElement, Url, window};

/// Schemes a relayed href may carry. Everything else is rejected.
///
/// `http`/`https` are the point. `mailto`/`tel` are here because they are
/// inert handoffs to an external handler and carry no script. Nothing else
/// has a use case, and every addition is a new way to reach the real origin.
const ALLOWED_SCHEMES: [&str; 4] = ["http:", "https:", "mailto:", "tel:"];

/// What the page decided a relayed href is.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Destination {
    /// Our own origin — open a tab with no dialog.
    SameOrigin(String),
    /// Off-origin, on the allowlist — confirm first. `host` is what the
    /// dialog names; for `mailto:`/`tel:` it is the address, which is the
    /// only meaningful thing to show.
    External { url: String, host: String },
    /// Not openable.
    Rejected,
}

/// Resolve `href` against `base` and decide what it is.
///
/// Split out from the DOM so it can be tested exhaustively — this is the
/// security boundary, and it is worth more tests per line than anything
/// else in the crate.
pub(crate) fn classify(href: &str, base: &str, page_origin: &str) -> Destination {
    let Ok(url) = Url::new_with_base(href, base) else {
        return Destination::Rejected;
    };
    let protocol = url.protocol();
    if !ALLOWED_SCHEMES.contains(&protocol.as_str()) {
        return Destination::Rejected;
    }
    // `origin` is `"null"` for `mailto:`/`tel:` (opaque path, no host), so
    // they can never collide with a real page origin and are always external.
    if protocol == "mailto:" || protocol == "tel:" {
        return Destination::External {
            host: url.pathname(),
            url: url.href(),
        };
    }
    if url.origin() == page_origin {
        return Destination::SameOrigin(url.href());
    }
    // `host` comes from the parser, so an IDN homograph is already punycoded
    // (`аpple.com` shows as `xn--pple-43d.com`) — one reason to display the
    // parsed host rather than anything from the href.
    Destination::External {
        host: url.host(),
        url: url.href(),
    }
}
```

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test -p tonk-host --target wasm32-unknown-unknown open`
Expected: PASS — 5 tests.

> If `it_rejects_an_unparseable_href` fails because `Url::new_with_base("http://", …)` parses in this engine, replace the href with `"http://["` (an invalid IPv6 literal, rejected by every conforming parser) and keep the test.

- [ ] **Step 7: Commit**

```bash
git add rust/tonk-host/src/open.rs rust/tonk-host/src/lib.rs rust/tonk-host/Cargo.toml
git commit -m "feat(tonk-host): classify a relayed href against a scheme allowlist

The href is attacker-controlled and this is where it meets the real
origin. Parse with the URL parser rather than prefix-matching: it
normalises JaVaScRiPt:, leading whitespace and embedded newlines into a
canonical protocol, which is the whole reason a prefix check is unsafe."
```

---

### Task 5: Opening, and the confirm dialog

**Files:**
- Modify: `rust/tonk-host/src/open.rs`

**Interfaces:**
- Consumes: `classify`, `Destination` (Task 4); `page_effect::forward` (Task 1); `crate::navigate_to` (Task 2).
- Produces: `pub fn open_external(href: &str)` — the entry point `tonk-portal`'s dispatcher calls in Task 6.

- [ ] **Step 1: Write the implementation**

Append to `rust/tonk-host/src/open.rs`, above the `mod tests` block. (This task is DOM-side and browser-verified in Task 8 rather than unit-tested; `classify` in Task 4 carries the logic that can be asserted in isolation.)

```rust
/// Open `href` on behalf of a guest.
///
/// Forwards until it reaches the page (see `page_effect`), then resolves the
/// href against the page's own URL — which is why a guest can send a bare
/// `/path` without knowing its own origin (it has none; it is opaque).
pub fn open_external(href: &str) {
    if page_effect::forward("open", href) {
        return;
    }
    let Some(win) = window() else {
        return;
    };
    let (Ok(base), Ok(page_origin)) = (win.location().href(), win.location().origin()) else {
        return;
    };
    match classify(href, &base, &page_origin) {
        Destination::SameOrigin(url) => open_same_origin(&url),
        Destination::External { url, host } => confirm_then_open(&url, &host),
        Destination::Rejected => {
            // The top page IS the real console, so warn directly. (The
            // `__tonkRuntime` warn channel exists to lift GUEST errors out of
            // an opaque origin that sanitizes them; nothing to lift here.)
            web_sys::console::warn_1(&JsValue::from_str(&format!(
                "tonk: refused to open `{href}` — scheme is not one of {ALLOWED_SCHEMES:?}"
            )));
        }
    }
}

/// Open our own origin in a new tab, with no dialog — there is nothing to
/// warn about.
///
/// Deliberately WITHOUT `noopener`, for two reasons that point the same way:
/// the destination is our own origin, so an opener reference is harmless and
/// ordinary; and `window.open` with `noopener` returns null unconditionally,
/// which would destroy the only signal we have that the popup was blocked.
///
/// Blocking is a live possibility here: unlike the dialog path there is no
/// confirm press, so this depends on the click's transient user activation
/// surviving the relay from the guest. A same-origin destination degrades to
/// a same-tab route change, which is a reasonable outcome — silently doing
/// nothing, the bug this whole change exists to fix, is not.
fn open_same_origin(url: &str) {
    let Some(win) = window() else {
        return;
    };
    match win.open_with_url_and_target(url, "_blank") {
        Ok(Some(_)) => {}
        _ => crate::navigate_to(url),
    }
}

/// Announce an off-origin destination, and open it if the user agrees.
///
/// The Open press is itself a user activation IN THE TOP DOCUMENT, so this
/// path never gambles on activation surviving the relay. The affordance and
/// the mechanism reinforce each other.
fn confirm_then_open(url: &str, host: &str) {
    let Some(document) = window().and_then(|w| w.document()) else {
        return;
    };
    let Some(body) = document.body() else {
        return;
    };
    ensure_styles(&document);

    let Some(dialog) = build_dialog(&document, host, url) else {
        return;
    };
    let Some(confirm) = dialog.query_selector(".tonk-open__confirm").ok().flatten() else {
        return;
    };
    let Some(cancel) = dialog.query_selector(".tonk-open__cancel").ok().flatten() else {
        return;
    };

    // One `close` listener owns teardown, so Esc, Cancel and Open all unwind
    // through the same path and the buttons only have to decide intent.
    on_event(dialog.unchecked_ref::<Element>(), "close", {
        let dialog = dialog.clone();
        move || {
            dialog.remove();
        }
    });
    on_event(&cancel, "click", {
        let dialog = dialog.clone();
        move || {
            dialog.close();
        }
    });
    on_event(&confirm, "click", {
        let dialog = dialog.clone();
        let document = document.clone();
        let url = url.to_owned();
        move || {
            open_in_new_tab(&document, &url);
            dialog.close();
        }
    });

    let _ = body.append_child(&dialog);
    let _ = dialog.show_modal();
}

/// Attach a listener that ignores its event. Leaked deliberately: the dialog
/// is removed on `close`, which drops the last reference to the element the
/// listeners are attached to, so nothing outlives the dialog.
fn on_event<F: FnMut() + 'static>(target: &Element, event: &str, mut handler: F) {
    let closure = Closure::wrap(Box::new(move |_: web_sys::Event| handler()) as Box<dyn FnMut(_)>);
    let _ = target.add_event_listener_with_callback(event, closure.as_ref().unchecked_ref());
    closure.forget();
}

/// Build the dialog.
///
/// Every attacker-controlled string goes in via `set_text_content`. There is
/// no `set_inner_html` here and there must never be: this renders on the real
/// origin, so interpolating a host or URL into markup would be a scripting
/// hole in the trusted document — the exact thing the scheme allowlist exists
/// to prevent, reintroduced one layer down.
fn build_dialog(document: &Document, host: &str, url: &str) -> Option<HtmlDialogElement> {
    let dialog: HtmlDialogElement = document
        .create_element("dialog")
        .ok()?
        .dyn_into::<HtmlDialogElement>()
        .ok()?;
    let _ = dialog.set_attribute("class", "tonk-open");

    let heading = document.create_element("h2").ok()?;
    heading.set_text_content(Some("Open in a new tab?"));

    let host_line = document.create_element("p").ok()?;
    let _ = host_line.set_attribute("class", "tonk-open__host");
    host_line.set_text_content(Some(host)); // text, never HTML

    let url_line = document.create_element("p").ok()?;
    let _ = url_line.set_attribute("class", "tonk-open__url");
    url_line.set_text_content(Some(url)); // text, never HTML

    let actions = document.create_element("div").ok()?;
    let _ = actions.set_attribute("class", "tonk-open__actions");

    let cancel = document.create_element("button").ok()?;
    let _ = cancel.set_attribute("class", "tonk-open__cancel");
    cancel.set_text_content(Some("Cancel"));

    let confirm = document.create_element("button").ok()?;
    let _ = confirm.set_attribute("class", "tonk-open__confirm");
    confirm.set_text_content(Some("Open"));

    let _ = actions.append_child(&cancel);
    let _ = actions.append_child(&confirm);
    let _ = dialog.append_child(&heading);
    let _ = dialog.append_child(&host_line);
    let _ = dialog.append_child(&url_line);
    let _ = dialog.append_child(&actions);
    Some(dialog)
}

/// Open `url` in a new tab by synthesizing an anchor and clicking it.
///
/// An anchor rather than `window.open` because it handles all four allowed
/// schemes uniformly: `window.open("mailto:…")` can strand a blank tab, while
/// an anchor click hands off to the mail client the way a real link does.
///
/// `noopener noreferrer` because this destination is off-origin: `noopener`
/// denies it a handle on our window (reverse tabnabbing), `noreferrer` keeps
/// the spot's URL out of its logs.
fn open_in_new_tab(document: &Document, url: &str) {
    let Ok(anchor) = document.create_element("a") else {
        return;
    };
    let _ = anchor.set_attribute("href", url);
    let _ = anchor.set_attribute("target", "_blank");
    let _ = anchor.set_attribute("rel", "noopener noreferrer");
    let Some(body) = document.body() else {
        return;
    };
    // Some engines only dispatch a click on a connected element.
    let _ = body.append_child(&anchor);
    if let Some(anchor) = anchor.dyn_ref::<HtmlElement>() {
        anchor.click();
    }
    anchor.remove();
}

/// Inject the dialog's stylesheet once.
///
/// Plain DOM and plain CSS, NOT `<wa-dialog>`: the Web Awesome loader is
/// idle-injected rather than eager (see `tonk-ui/index.html`), because its
/// statically-imported chunks would otherwise starve the boot data plane. A
/// `wa-*` component could still be undefined when an early click lands. Every
/// value is `var(--wa-token, literal)` so it matches the theme when loaded and
/// still looks right before it is — the same technique the boot shell uses,
/// and it keeps index.html's "nothing on the top page uses a wa-* component"
/// true.
fn ensure_styles(document: &Document) {
    const STYLE_ID: &str = "tonk-open-style";
    if document.get_element_by_id(STYLE_ID).is_some() {
        return;
    }
    let Ok(style) = document.create_element("style") else {
        return;
    };
    let _ = style.set_attribute("id", STYLE_ID);
    style.set_text_content(Some(
        r#"
dialog.tonk-open {
  border: 1px solid var(--wa-color-neutral-border-normal, #d4d4d8);
  border-radius: var(--wa-border-radius-l, 8px);
  background: var(--wa-color-surface-raised, #fff);
  color: var(--wa-color-text-normal, #18181b);
  font-family: var(--wa-font-family-body, system-ui, sans-serif);
  padding: 1.25rem;
  max-width: min(28rem, calc(100vw - 2rem));
}
dialog.tonk-open::backdrop { background: rgb(0 0 0 / 0.4); }
.tonk-open h2 {
  margin: 0 0 0.75rem;
  font-size: var(--wa-font-size-l, 1.125rem);
}
.tonk-open__host {
  margin: 0 0 0.25rem;
  font-weight: 600;
}
/* The URL is attacker-chosen: it must wrap rather than widen the dialog,
   and it must not be able to push the buttons off-screen. */
.tonk-open__url {
  margin: 0 0 1.25rem;
  color: var(--wa-color-text-quiet, #71717a);
  font-size: var(--wa-font-size-s, 0.875rem);
  overflow-wrap: anywhere;
  max-height: 4.5rem;
  overflow-y: auto;
}
.tonk-open__actions {
  display: flex;
  justify-content: flex-end;
  gap: 0.5rem;
}
.tonk-open button {
  border-radius: var(--wa-border-radius-m, 6px);
  border: 1px solid var(--wa-color-neutral-border-normal, #d4d4d8);
  background: var(--wa-color-neutral-fill-quiet, #f4f4f5);
  color: inherit;
  font: inherit;
  padding: 0.4rem 0.9rem;
  cursor: pointer;
}
.tonk-open__confirm {
  background: var(--wa-color-brand-fill-loud, #3b4a0a);
  border-color: var(--wa-color-brand-fill-loud, #3b4a0a);
  color: var(--wa-color-brand-on-loud, #f4f7e4);
}
"#,
    ));
    if let Some(head) = document.head() {
        let _ = head.append_child(&style);
    }
}
```

- [ ] **Step 2: Verify it compiles and lints**

```bash
cargo clippy -p tonk-host --target wasm32-unknown-unknown --all-targets -- -D warnings
cargo fmt --check
```
Expected: clean. If `HtmlDialogElement` is unresolved, Step 1 of Task 4 (the Cargo feature) was skipped.

- [ ] **Step 3: Run the crate's tests**

Run: `cargo test -p tonk-host --target wasm32-unknown-unknown`
Expected: PASS — Task 4's classification tests still pass; no new tests here.

- [ ] **Step 4: Commit**

```bash
git add rust/tonk-host/src/open.rs
git commit -m "feat(tonk-host): open a relayed link, confirming off-origin ones

The dialog gates leaving the origin, not opening a tab: a same-origin
destination opens silently, an external one is announced first. The
confirm press doubles as the user activation that lets the tab open, so
that path never depends on activation surviving the relay."
```

---

### Task 6: The `open` bridge message

**Files:**
- Modify: `rust/tonk-portal/src/bridge.rs:241-243` (guest bootstrap), `:1189` (dispatch), `:1440` (handlers)

**Interfaces:**
- Consumes: `tonk_host::open_external` (Task 5).
- Produces: `window.tonk.open(href)` in every guest; `{v:1, type:"open", href}` on the port.

- [ ] **Step 1: Write the failing test**

In `rust/tonk-portal/src/bridge.rs`, find the test module containing the existing envelope-parsing tests and add:

```rust
    /// `open_href` accepts only a well-formed `{type:"open", href}`. The
    /// dispatcher has already matched on `type`; re-checking here keeps the
    /// parse independently testable, as `title_text` does.
    #[dialog_common::test]
    async fn it_reads_href_only_from_an_open_message() {
        let message = js_sys::Object::new();
        let _ = js_sys::Reflect::set(
            &message,
            &JsValue::from_str("type"),
            &JsValue::from_str("open"),
        );
        let _ = js_sys::Reflect::set(
            &message,
            &JsValue::from_str("href"),
            &JsValue::from_str("https://example.com/"),
        );
        assert_eq!(
            open_href(&message.into()),
            Some("https://example.com/".to_owned()),
            "an open message with an href should yield it"
        );

        let empty = js_sys::Object::new();
        let _ = js_sys::Reflect::set(
            &empty,
            &JsValue::from_str("type"),
            &JsValue::from_str("open"),
        );
        let _ = js_sys::Reflect::set(&empty, &JsValue::from_str("href"), &JsValue::from_str(""));
        assert_eq!(
            open_href(&empty.into()),
            None,
            "an empty href should yield None"
        );

        let other = js_sys::Object::new();
        let _ = js_sys::Reflect::set(
            &other,
            &JsValue::from_str("type"),
            &JsValue::from_str("navigate"),
        );
        let _ = js_sys::Reflect::set(
            &other,
            &JsValue::from_str("href"),
            &JsValue::from_str("https://example.com/"),
        );
        assert_eq!(
            open_href(&other.into()),
            None,
            "a non-open message should yield None"
        );

        assert_eq!(
            open_href(&JsValue::from_str("not an object")),
            None,
            "a non-object payload should yield None"
        );
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p tonk-portal --target wasm32-unknown-unknown open_href`
Expected: FAIL — `cannot find function 'open_href' in this scope`.

- [ ] **Step 3: Add the handlers**

In `rust/tonk-portal/src/bridge.rs`, immediately after `title_text` (which ends at `:1456`), add:

```rust
/// Open a link on the guest's behalf. The sealed guest has no `allow-popups`
/// and no `allow-top-navigation`, so it cannot open anything itself; it posts
/// the raw href and `tonk_host::open_external` — running on the page, which is
/// the only place that can both resolve and open it — decides what happens.
fn handle_open(data: &JsValue) {
    let Some(href) = open_href(data) else {
        return;
    };
    tonk_host::open_external(&href);
}

/// Read `href` out of an `{ type: "open", href }` message, or `None` when the
/// message isn't an open or carries no usable href. Mirrors `title_text`.
fn open_href(data: &JsValue) -> Option<String> {
    if get_str(data, "type")? != "open" {
        return None;
    }
    get_str(data, "href").filter(|href| !href.is_empty())
}
```

- [ ] **Step 4: Dispatch it**

In `make_dispatcher` (`bridge.rs:1182-1192`), add a line after `"title" => handle_title(&data),`:

```rust
            "open" => handle_open(&data),
```

- [ ] **Step 5: Expose it to guests**

In `BOOTSTRAP_JS`, immediately after the `setTitle` entry (`bridge.rs:241-243`), add:

```js
    // Open a link from the HOST: the opaque guest has neither `allow-popups`
    // nor `allow-top-navigation`, so a click on an external link posts its
    // raw href here and the parent decides — resolving it against the real
    // origin, allowlisting the scheme, and confirming anything off-origin.
    // Fire-and-forget (no response).
    open:function(href){
      ready.then(function(){port.postMessage({v:1,type:"open",href:href});});
    },
```

Also add `open(href)` to the `window.tonk` shape documented in the module header at `bridge.rs:15`, beside `navigate(href)` and `setTitle(text)`.

> This is hand-written JS inside a Rust string literal, so a typo is a runtime failure the compiler cannot catch. Keep it a character-for-character analogue of `navigate` above it, and exercise it in Task 8.

- [ ] **Step 6: Run the tests to verify they pass**

```bash
cargo test -p tonk-portal --target wasm32-unknown-unknown open_href
cargo clippy -p tonk-portal --target wasm32-unknown-unknown --all-targets -- -D warnings
```
Expected: PASS, clean.

- [ ] **Step 7: Commit**

```bash
git add rust/tonk-portal/src/bridge.rs
git commit -m "feat(tonk-portal): relay a guest's link open to the page

A sixth port message, mirroring navigate and title. The guest sends the
raw href; all policy lives at the page, so there is no intermediate gate
for spot content to forge past."
```

---

### Task 7: The guest click classifier

**Files:**
- Modify: `rust/tonk-guest/src/guest_host.rs:32-115`

**Interfaces:**
- Consumes: `window.tonk.open` (Task 6).
- Produces: no public API change — `pub fn install()` still.

**Context:** the listener currently relays only `/…` paths and bails on modified clicks (`:86-90`), so external links, `mailto:`, `target="_blank"`, and cmd/middle-clicks are all inert. This replaces the bail with a classifier.

- [ ] **Step 1: Add the `auxclick` listener**

Middle-click does **not** fire `click` in modern browsers — it fires `auxclick`. The existing `mouse.button() != 0` guard inside a `click` listener is therefore unreachable for middle-clicks. Register both events.

In `install()` (`guest_host.rs:53-63`), replace the navigation-relay block with:

```rust
    // Navigation relay: a link click inside the opaque guest can't move the
    // parent or open a tab, so catch it at the document and post the href over
    // the bridge for the host to perform. Capture phase so it runs before any
    // app handler and before the (blocked-anyway) native action.
    //
    // `auxclick` as well as `click`: a middle-click does not fire `click` at
    // all, so a `click`-only listener can never see one.
    let mut listeners = Vec::with_capacity(2);
    for event in ["click", "auxclick"] {
        let listener = make_nav_listener();
        let _ = document.add_event_listener_with_callback_and_bool(
            event,
            listener.as_ref().unchecked_ref(),
            true,
        );
        listeners.push(listener);
    }
    INSTALLED.with(|cell| *cell.borrow_mut() = Some(listeners));
```

and change the thread-local (`:29-35`) to hold both:

```rust
/// Installed listeners, kept alive for the page's lifetime.
type Listener = Closure<dyn FnMut(Event)>;

thread_local! {
    /// The installed navigation listeners. `Some` once [`install`] ran.
    static INSTALLED: RefCell<Option<Vec<Listener>>> = const { RefCell::new(None) };
}
```

- [ ] **Step 2: Write the classifier**

Replace `make_nav_listener` (`guest_host.rs:79-106`) with:

```rust
/// What a click on a link should do.
#[derive(Debug, PartialEq, Eq)]
enum Intent {
    /// An in-app route change, performed by the host in place.
    Navigate(String),
    /// A new tab, decided and performed by the host. Whether it is announced
    /// first is the HOST's call, not ours — this guest is untrusted, so its
    /// classification is routing, never policy.
    Open(String),
    /// Not ours. Left to the browser.
    Ignore,
}

/// Build the document click listener that relays link activation to the host.
fn make_nav_listener() -> Listener {
    Closure::wrap(Box::new(move |event: Event| match classify_click(&event) {
        Intent::Ignore => {}
        Intent::Navigate(href) => {
            event.prevent_default();
            call_bridge("navigate", &href);
        }
        Intent::Open(href) => {
            event.prevent_default();
            call_bridge("open", &href);
        }
    }) as Box<dyn FnMut(Event)>)
}

/// Decide what a click means.
///
/// Anything that isn't a plain in-app navigation is handed to `open`, INCLUDING
/// schemes we expect the host to refuse. Filtering them here would be security
/// theatre: a component can call `window.tonk.open` directly, so the guest is
/// never the control — and relaying gets a console warning out of the host
/// instead of a silently dead click, which is the bug this change exists to fix.
fn classify_click(event: &Event) -> Intent {
    let Some(mouse) = event.dyn_ref::<web_sys::MouseEvent>() else {
        return Intent::Ignore;
    };
    // `auxclick` fires for every non-primary button. Middle (1) means "new
    // tab"; right (2) belongs to the context menu.
    if event.type_() == "auxclick" && mouse.button() != 1 {
        return Intent::Ignore;
    }
    let Some(anchor) = event.target().and_then(closest_anchor) else {
        return Intent::Ignore;
    };
    let Some(href) = anchor.get_attribute("href").filter(|h| !h.is_empty()) else {
        return Intent::Ignore;
    };
    // A fragment is same-document and needs no host: the browser handles it
    // inside the guest, where the scrolling actually has to happen.
    if href.starts_with('#') {
        return Intent::Ignore;
    }

    let wants_new_tab = mouse.meta_key()
        || mouse.ctrl_key()
        || mouse.shift_key()
        || mouse.button() == 1
        || anchor
            .get_attribute("target")
            .is_some_and(|target| target == "_blank");
    let in_app = href.starts_with('/') && !href.starts_with("//");

    if in_app && !wants_new_tab {
        Intent::Navigate(href)
    } else {
        Intent::Open(href)
    }
}

/// Walk up from an event target to the nearest `<a href>`.
///
/// The raw `href` ATTRIBUTE is what callers read, never the resolved `.href`
/// property, which an opaque origin mangles to `null/…`. Resolution is the
/// host's job — it is the only frame with a real base URL.
fn closest_anchor(target: web_sys::EventTarget) -> Option<Element> {
    target
        .dyn_into::<Element>()
        .ok()?
        .closest("a[href]")
        .ok()
        .flatten()
}

/// Call a fire-and-forget method on the bridge, if it is installed.
fn call_bridge(method: &str, arg: &str) {
    if let Some(tonk) = window_tonk()
        && let Some(function) = get_fn(&tonk, method)
    {
        let _ = function.call1(&tonk, &JsValue::from_str(arg));
    }
}
```

Delete `closest_anchor_href` (`:111-115`) — `closest_anchor` replaces it.

- [ ] **Step 3: Update the module header**

In `guest_host.rs:15-17`, replace the navigation bullet:

```rust
//! - the link click relay — the opaque guest can neither move the
//!   parent nor open a tab (no `allow-top-navigation`, no
//!   `allow-popups`), so link clicks post their raw href over
//!   `window.tonk.navigate` or `window.tonk.open` for the host to
//!   resolve and perform;
```

- [ ] **Step 4: Verify it compiles and lints**

```bash
cargo clippy -p tonk-guest --target wasm32-unknown-unknown --all-targets -- -D warnings
cargo fmt --check
```
Expected: clean. No new web-sys features are needed — `target` is read with `get_attribute`, so `HtmlAnchorElement` is not required.

- [ ] **Step 5: Commit**

```bash
git add rust/tonk-guest/src/guest_host.rs
git commit -m "feat(tonk-guest): classify link clicks instead of dropping them

External links, mailto:/tel:, target=_blank and modified clicks all fell
through to a native action the sandbox blocks, so the click did nothing
at all. Route them to the host's open effect.

Listen for auxclick too: a middle-click never fires click, so the
existing button() guard could not have seen one."
```

---

### Task 8: Build, and verify in a browser

**Files:** none — this is the verification gate.

The rendered chain crosses two iframes, a service worker, and a popup blocker. None of that is reachable from a unit test, and per `project_wasm_tests_need_safari_automation` local wasm tests need Safari automation or a major-matched chromedriver. So this task is manual and mandatory.

- [ ] **Step 1: Full workspace gate**

```bash
cargo fmt --check
nix flake check
```
Expected: clean. Note `--all-features` compiles integration tests, so a green per-crate clippy does not imply a green gate.

- [ ] **Step 2: Build and serve the UI**

Use the repo's normal dev loop for `tonk-ui` (see `.claude/skills/run` / the crate's README). The app must be served over a real origin — the service worker will not register otherwise, and none of this works without it.

- [ ] **Step 3: Seed a spot with links to click**

Create a spot and assert a view carrying every case. Per `project_slide_guide_this_mapping_bug` and the tonk skill, use asserted notation against the live synced branch (`project_slide_dont_use_no_sync` — do not pass `--no-sync`).

The view's template needs:

```html
<a href="https://example.com/docs/x">external https</a>
<a href="mailto:someone@example.com">mail</a>
<a href="tel:+15551234567">phone</a>
<a href="/">in-app path</a>
<a href="/" target="_blank">in-app, new tab</a>
<a href="javascript:alert(1)">rejected scheme</a>
<a href="#section">fragment</a>
```

- [ ] **Step 4: Walk the matrix**

| Action | Expected |
|---|---|
| Click `external https` | Dialog: "Open in a new tab?", host `example.com`, full URL beneath |
| Dialog → Cancel | Dialog closes. No tab. Spot untouched. |
| Dialog → Esc | Same as Cancel. |
| Dialog → Open | New tab loads example.com. Spot still there, still working. |
| Click `mail` | Dialog names `someone@example.com`; Open hands off to the mail client with no stranded blank tab |
| Click `tel` | Dialog names `+15551234567` |
| Click `in-app path` | Navigates in place. No dialog. No reload — the spot's iframe is reused. |
| Cmd/ctrl-click `in-app path` | New tab at `/`. No dialog. |
| Middle-click `in-app path` | New tab at `/`. No dialog. |
| Click `in-app, new tab` | New tab at `/`. No dialog. |
| Click `rejected scheme` | Nothing opens. Console warns `tonk: refused to open ...`. **No alert fires.** |
| Click `fragment` | Scrolls within the spot. No dialog, no navigation. |

- [ ] **Step 5: Verify the depth-2 fix specifically**

This is PR 1's whole point and the matrix above only exercises it implicitly.

With `in-app path` clicked inside the spot, confirm the URL bar changed and the **top page** re-routed. The failure this fixes is unmistakable: the entire Tonk app re-rendering *inside* the spot's frame, with the URL bar unchanged and service-worker errors in the console.

- [ ] **Step 6: Verify tab titles still work**

`set_title` changed in Task 3. Confirm the tab still reads `<Spot name> — Tonk` on `/space/{id}`, and that renaming the spot still retitles it live.

- [ ] **Step 7: Check the popup fallback**

Enable the browser's popup blocker for the origin, then cmd-click `in-app path`. Expected: it navigates in the **same tab** rather than doing nothing — `open_same_origin`'s fallback. If it silently does nothing, `window.open` returned a truthy handle for a blocked popup, or `noopener` crept back into the features string and forced a null return.

- [ ] **Step 8: Commit any fixes, then open PR 2**

Base it on `origin/staging`, stacked on PR 1.

---

### Task 9: Correct the spec

**Files:**
- Modify: `docs/superpowers/specs/2026-07-16-spot-link-clicks-design.md`

The spec's "Errors" section says a rejected scheme is reported over the `__tonkRuntime:"warn"` channel. That is wrong, and the implementation in Task 5 does not do it: rejection happens at the top page, which **is** the real console. The `__tonkRuntime` channel exists to lift errors out of an opaque guest origin that sanitizes them — there is nothing to lift.

- [ ] **Step 1: Replace the "Errors" section**

```markdown
## Errors

A rejected scheme is dropped and `console.warn`ed by the top page, naming the
href and the allowlist. No bridge channel is involved: rejection happens on the
page, which is already the real console. (The `__tonkRuntime:"warn"` relay at
`bridge.rs:447-458` exists to lift GUEST errors out of an opaque origin that
sanitizes them out of the parent console — that is a different problem.)

A silently-dropped click is the bug this change exists to fix; the fix must not
reproduce it in a narrower form.
```

- [ ] **Step 2: Commit**

```bash
git add docs/superpowers/specs/2026-07-16-spot-link-clicks-design.md
git commit -m "docs: correct the error-reporting path in the link-click spec

Rejection happens on the top page, which is the real console. The
__tonkRuntime warn relay is for lifting guest errors out of an opaque
origin and has no role here."
```

---

## Deliberately not in this plan

Carried from the spec's Scope, so nobody adds them mid-flight:

- **In-page fragments** — left native (Task 7 ignores them).
- **Sub-route titles** — PR 1 unblocks them; nobody has asked.
- **Deleting `fab.rs`** (354 lines of unreachable code) and the stale comments at `profile.yaml:807-808`, `fab.rs:5-8`, `shared.rs:3`. Worth doing, but not inside a security-sensitive change.
- **A real fix for `navigate_to`'s `SecurityError` misread** (`navigate.rs:131` reads it as "no history access"). PR 1 makes it unreachable rather than correct.
- **Hover destination chips** — considered and dropped: a chatty message stream that shows nothing on touch devices.
- **Per-host "don't ask again"** — speculative until the dialog proves annoying.
