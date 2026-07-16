# Spot Tab Title Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Title the browser tab with the focused spot's name — `<Spot name> — Tonk`, live-updating on rename.

**Architecture:** `document.title` lives only on the top page, but the spot's name lives on the spot's own branch and is rendered by chrome inside a sealed guest. The guest asks the page to retitle over the existing portal `MessagePort` bridge, adding a fifth message type (`title`) beside `navigate`/`fetch`/`subscribe`/`unsubscribe`. A headless `<tonk-title>` element pushes its `text` attribute; a view template supplies `{name}`; the mount sits in the depth-1 profile chrome so the bridge dispatcher — which runs in the guest's **parent** — lands on the real document.

**Tech Stack:** Rust → wasm (`wasm-bindgen`, `web-sys`, `custom-elements`), asserted-notation YAML libraries, `dialog_common::test` + `wasm-bindgen-test`.

**Spec:** `docs/superpowers/specs/2026-07-16-spot-tab-title-design.md`

## Global Constraints

- **Repo:** `/Users/jackdouglas/tonk/tonk/.wt/spot-improv`, branch `feat/spot-improv`.
- **Use `git`, NOT `jj`.** The `jj` root is `/Users/jackdouglas/tonk/tonk` (the main checkout) and does not track this worktree; `jj` commands here would operate on the wrong workspace.
- **Commit style:** Conventional Commits — `type(scope): subject`, imperative, lowercase, no trailing period, subject under ~72 chars.
- **No emojis** in code, comments, or commit messages.
- **Test style:** always `#[dialog_common::test]`; name tests `it_does_x`; group by behaviour. Every assertion carries a message saying what should hold.
- **Lint gate:** workspace `cargo clippy --all-targets --all-features` + `cargo fmt --check`. Run per the commands in each task.
- **Comments state constraints, not narration.** Do not write "Phase X", "per the RFC", or references to this plan in code.
- **Exact title copy:** `<name> — Tonk` and `Untitled — Tonk`. The separator is an em dash (`—`, U+2014) surrounded by single spaces.

---

## File Structure

| File | Responsibility |
|---|---|
| `rust/tonk-host/src/title.rs` (create) | `set_title` — the page-side assignment of `document.title`, with the empty-string guard. |
| `rust/tonk-host/src/lib.rs` (modify) | Declare `mod title` and re-export `set_title`. |
| `rust/tonk-portal/src/bridge.rs` (modify) | The `title` wire message: guest `setTitle` in the bootstrap JS; `title_text` parse + `handle_title` dispatch on the host side. |
| `rust/tonk-portal/src/title.rs` (create) | The `<tonk-title>` element — pushes its `text` over the bridge. Renders nothing. |
| `rust/tonk-portal/src/lib.rs` (modify) | Declare `mod title` and export `register_title`. |
| `rust/tonk-guest/src/bin/guest.rs` (modify) | Register `<tonk-title>` in the guest's element surface. |
| `rust/tonk-core/assets/library/core.yaml` (modify) | The `tonk:view/title` view kind and the `tonk:repository` title-view row carrying `{name}`. |
| `rust/tonk-core/assets/library/profile.yaml` (modify) | Mount the title view in the depth-1 space chrome. |

Data flows one way: element → bridge → `set_title`. Tasks 1-3 build it bottom-up so each compiles against the one before; tasks 4-5 supply the data that drives it.

---

### Task 1: `set_title` on the host page

**Files:**
- Create: `rust/tonk-host/src/title.rs`
- Modify: `rust/tonk-host/src/lib.rs`

**Interfaces:**
- Consumes: nothing.
- Produces: `tonk_host::set_title(title: &str)` — sets `document.title`; no-ops on an empty string. Used by Task 2.

- [ ] **Step 1: Write the failing test**

Create `rust/tonk-host/src/title.rs` containing ONLY the test module for now (the implementation lands in Step 3):

```rust
#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod tests {
    use super::*;
    use wasm_bindgen_test::wasm_bindgen_test_configure;

    wasm_bindgen_test_configure!(run_in_browser);

    fn document_title() -> String {
        web_sys::window()
            .expect("a window in the test harness")
            .document()
            .expect("a document in the test harness")
            .title()
    }

    /// A non-empty title reaches the document. An empty one is ignored:
    /// a view renders a blank `{name}` before the fact resolves, and
    /// letting that through would wipe a title that was already right.
    #[dialog_common::test]
    async fn it_sets_a_non_empty_title_and_ignores_an_empty_one() {
        set_title("Notes — Tonk");
        assert_eq!(
            document_title(),
            "Notes — Tonk",
            "a non-empty title should reach the document"
        );

        set_title("");
        assert_eq!(
            document_title(),
            "Notes — Tonk",
            "an empty title should leave the previous title in place"
        );
    }
}
```

Add to `rust/tonk-host/src/lib.rs`, directly after the existing `mod navigate;` / `pub use navigate::navigate_to;` pair (currently near line 70):

```rust
#[cfg(target_arch = "wasm32")]
mod title;
#[cfg(target_arch = "wasm32")]
pub use title::set_title;
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo check -p tonk-host --target wasm32-unknown-unknown`
Expected: FAIL — `cannot find function 'set_title' in this scope` (and an unresolved `title::set_title` re-export).

This is a compile-level failure, which is the honest "red" for a Rust function that does not exist yet.

- [ ] **Step 3: Write minimal implementation**

Prepend to `rust/tonk-host/src/title.rs`, above the test module:

```rust
//! Setting the host page's tab title on a guest's behalf.
//!
//! `document.title` exists only on the top page. The chrome that knows
//! a spot's name renders inside a sealed guest, which cannot reach the
//! top document, so it posts a `title` message over the portal bridge;
//! the bridge dispatcher runs in the parent and calls this.
//!
//! Mirrors `navigate.rs` — a page capability a guest asks for. Unlike
//! navigate there is no provider to install: nothing is pushed from the
//! service worker, so this is a plain function, not a listener.

use web_sys::window;

/// Set the page's tab title.
///
/// An empty title is a no-op, not a blank tab: a view renders a blank
/// `{name}` until the fact resolves, and letting that through would
/// wipe a title that was already correct.
pub fn set_title(title: &str) {
    if title.is_empty() {
        return;
    }
    let Some(document) = window().and_then(|w| w.document()) else {
        return;
    };
    document.set_title(title);
}
```

`web-sys`'s `Document` feature is already enabled in `rust/tonk-host/Cargo.toml`; no dependency change is needed.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo check -p tonk-host --target wasm32-unknown-unknown`
Expected: PASS (compiles clean).

Then run the browser test:
Run: `cargo test -p tonk-host --target wasm32-unknown-unknown title`
Expected: PASS — `it_sets_a_non_empty_title_and_ignores_an_empty_one`.

If the wasm test runner cannot start (no Chrome at the default path, or a chromedriver major mismatch), do NOT treat that as a failure of this task — the harness is a known local gap. Record that the test was not run and continue; the browser check in Task 6 is the backstop.

- [ ] **Step 5: Commit**

```bash
git add rust/tonk-host/src/title.rs rust/tonk-host/src/lib.rs
git commit -m "feat(tonk-host): set the page tab title for a guest"
```

---

### Task 2: The `title` message on the portal bridge

**Files:**
- Modify: `rust/tonk-portal/src/bridge.rs` (bootstrap JS near line 237; `make_dispatcher` near line 1177; handlers near line 1423; tests module near line 2142)

**Interfaces:**
- Consumes: `tonk_host::set_title` (Task 1).
- Produces:
  - Guest-side JS: `window.tonk.setTitle(text)` — fire-and-forget, posts `{v:1, type:"title", text}`. Used by Task 3.
  - Host-side Rust: `title_text(data: &JsValue) -> Option<String>` and `handle_title(data: &JsValue)` (both private to the module).

- [ ] **Step 1: Write the failing test**

Add to the existing `mod tests` in `rust/tonk-portal/src/bridge.rs` (near line 2142):

```rust
    fn title_message(kind: &str, text: &str) -> JsValue {
        let object = js_sys::Object::new();
        let _ = Reflect::set(
            &object,
            &JsValue::from_str("type"),
            &JsValue::from_str(kind),
        );
        let _ = Reflect::set(
            &object,
            &JsValue::from_str("text"),
            &JsValue::from_str(text),
        );
        object.into()
    }

    /// `title_text` accepts only a `{ type: "title", text }` shape with
    /// non-empty text; everything else yields `None`, so an unrelated
    /// message never retitles the tab and an unresolved `{name}` never
    /// blanks it. We assert the parse, not the assignment — performing
    /// it would retitle the test harness itself.
    #[dialog_common::test]
    async fn it_reads_text_only_from_a_title_message() {
        assert_eq!(
            title_text(&title_message("title", "Notes — Tonk")),
            Some("Notes — Tonk".to_owned()),
            "a title message with text should yield it"
        );
        assert_eq!(
            title_text(&title_message("title", "")),
            None,
            "an empty text should yield None"
        );
        assert_eq!(
            title_text(&title_message("other", "Notes — Tonk")),
            None,
            "a non-title message should yield None"
        );
        assert_eq!(
            title_text(&JsValue::from_str("not an object")),
            None,
            "a non-object payload should yield None"
        );
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo check -p tonk-portal --target wasm32-unknown-unknown --all-targets`
Expected: FAIL — `cannot find function 'title_text' in this scope`.

- [ ] **Step 3: Write minimal implementation**

Three edits to `rust/tonk-portal/src/bridge.rs`.

**3a.** In the guest bootstrap JS, immediately after the `navigate:function(href){…},` entry (near line 237), add:

```javascript
    // Retitle the HOST page's tab: the opaque guest can't touch
    // parent.document.title. `<tonk-title>` posts its text here and the
    // parent performs the real assignment. Fire-and-forget (no response).
    setTitle:function(text){
      ready.then(function(){port.postMessage({v:1,type:"title",text:text});});
    },
```

This is hand-written JS inside a Rust string literal — the compiler cannot check it. Keep it a near-copy of `navigate` directly above it, and mind the trailing comma.

**3b.** In `make_dispatcher`'s match (near line 1177), add the arm after `"navigate"`:

```rust
            "title" => handle_title(&data),
```

**3c.** Beside `handle_navigate` (near line 1423), add:

```rust
/// Set the host page's tab title on the guest's behalf. The guest's
/// `<tonk-title>` posts `{v:1, type:"title", text}`; this runs in the
/// parent document, which is where `document.title` lives.
fn handle_title(data: &JsValue) {
    let Some(text) = title_text(data) else {
        return;
    };
    tonk_host::set_title(&text);
}

/// Read `text` out of a `{ type: "title", text }` message, or `None` when
/// the message isn't a title or carries no usable text. The dispatcher
/// has already matched on `type`; re-checking it here keeps the parse
/// independently testable, as `navigate_href` does in `tonk-host`.
fn title_text(data: &JsValue) -> Option<String> {
    if get_str(data, "type")? != "title" {
        return None;
    }
    get_str(data, "text").filter(|text| !text.is_empty())
}
```

`get_str` (line 2096), `Reflect`, and `JsValue` are already in scope in this module. `tonk-host` is already a dependency of `tonk-portal`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo check -p tonk-portal --target wasm32-unknown-unknown --all-targets`
Expected: PASS.

Run: `cargo test -p tonk-portal --target wasm32-unknown-unknown title_text`
Expected: PASS — `it_reads_text_only_from_a_title_message`.

Same harness caveat as Task 1: if the wasm runner cannot start, record it and continue.

- [ ] **Step 5: Commit**

```bash
git add rust/tonk-portal/src/bridge.rs
git commit -m "feat(tonk-portal): carry a title message over the guest bridge"
```

---

### Task 3: The `<tonk-title>` element

**Files:**
- Create: `rust/tonk-portal/src/title.rs`
- Modify: `rust/tonk-portal/src/lib.rs`
- Modify: `rust/tonk-guest/src/bin/guest.rs`

**Interfaces:**
- Consumes: `window.tonk.setTitle(text)` (Task 2).
- Produces: `tonk_portal::register_title()` — defines `<tonk-title>`. The element reads one attribute, `text`.

- [ ] **Step 1: Write the failing test**

Create `rust/tonk-portal/src/title.rs` containing ONLY the test module for now:

```rust
#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod tests {
    use super::*;
    use wasm_bindgen::closure::Closure;
    use wasm_bindgen_test::wasm_bindgen_test_configure;

    wasm_bindgen_test_configure!(run_in_browser);

    /// Install a `window.tonk.setTitle` stub recording its argument on
    /// `window.__title`, clearing any previous record. There is no real
    /// bridge in the test harness, so the stub stands in for the parent.
    fn install_stub() {
        let win = window().expect("a window in the test harness");
        let _ = Reflect::set(&win, &JsValue::from_str("__title"), &JsValue::UNDEFINED);
        let tonk = Object::new();
        let capture = Closure::<dyn FnMut(JsValue)>::new(move |value: JsValue| {
            let win = window().expect("a window in the test harness");
            let _ = Reflect::set(&win, &JsValue::from_str("__title"), &value);
        });
        let _ = Reflect::set(&tonk, &JsValue::from_str("setTitle"), capture.as_ref());
        capture.forget();
        let _ = Reflect::set(&win, &JsValue::from_str("tonk"), &tonk);
    }

    fn captured() -> Option<String> {
        let win = window().expect("a window in the test harness");
        Reflect::get(&win, &JsValue::from_str("__title"))
            .ok()?
            .as_string()
    }

    fn element_with_text(text: Option<&str>) -> HtmlElement {
        let document = window()
            .expect("a window in the test harness")
            .document()
            .expect("a document in the test harness");
        let element = document
            .create_element("tonk-title")
            .expect("creates an element")
            .dyn_into::<HtmlElement>()
            .expect("an html element");
        if let Some(text) = text {
            let _ = element.set_attribute("text", text);
        }
        element
    }

    /// Only a non-empty `text` rides the bridge. A blank or absent one
    /// is dropped, so a view that has not resolved `{name}` yet never
    /// blanks the tab.
    #[dialog_common::test]
    async fn it_pushes_only_a_non_empty_text() {
        install_stub();
        push_title(&element_with_text(Some("Notes — Tonk")));
        assert_eq!(
            captured(),
            Some("Notes — Tonk".to_owned()),
            "a non-empty text should reach the bridge"
        );

        install_stub();
        push_title(&element_with_text(Some("")));
        assert_eq!(
            captured(),
            None,
            "an empty text should not reach the bridge"
        );

        install_stub();
        push_title(&element_with_text(None));
        assert_eq!(
            captured(),
            None,
            "a missing text attribute should not reach the bridge"
        );
    }

    /// Without a bridge installed the push is a silent no-op, not a
    /// panic: the element may connect before the bootstrap does. A
    /// panic would fail this outright; the assertion additionally
    /// pins that nothing was pushed.
    #[dialog_common::test]
    async fn it_does_nothing_without_a_bridge() {
        install_stub();
        let win = window().expect("a window in the test harness");
        let _ = Reflect::set(&win, &JsValue::from_str("__title"), &JsValue::UNDEFINED);
        let _ = Reflect::set(&win, &JsValue::from_str("tonk"), &JsValue::UNDEFINED);

        push_title(&element_with_text(Some("Notes — Tonk")));

        assert_eq!(
            captured(),
            None,
            "an absent bridge should push nothing at all"
        );
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo check -p tonk-portal --target wasm32-unknown-unknown --all-targets`
Expected: FAIL — `cannot find function 'push_title' in this scope`.

- [ ] **Step 3: Write minimal implementation**

Prepend to `rust/tonk-portal/src/title.rs`, above the test module:

```rust
//! `<tonk-title>` — a headless element that names the browser tab.
//!
//! It renders nothing. Its only job is to push its `text` attribute to
//! the host page, which owns `document.title`: a sealed guest cannot
//! touch the top document, so the text rides the bridge's `title`
//! message (`window.tonk.setTitle`) and the parent assigns it.
//!
//! DEPTH CONSTRAINT: the bridge dispatcher runs in the guest's PARENT
//! document, so this titles the real tab only when mounted in a
//! depth-1 guest — the profile chrome's space view. Mounted deeper it
//! silently retitles an intermediate iframe instead.

use custom_elements::CustomElement;
use js_sys::{Function, Object, Reflect};
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use web_sys::{HtmlElement, window};

#[derive(Default)]
struct TitleElement;

impl CustomElement for TitleElement {
    fn shadow() -> bool {
        false
    }

    fn observed_attributes() -> &'static [&'static str] {
        &["text"]
    }

    fn inject_children(&mut self, _this: &HtmlElement) {}

    fn connected_callback(&mut self, this: &HtmlElement) {
        push_title(this);
    }

    fn attribute_changed_callback(
        &mut self,
        this: &HtmlElement,
        _name: String,
        _old: Option<String>,
        _new: Option<String>,
    ) {
        push_title(this);
    }
}

/// Push the element's `text` to the host page over `window.tonk.setTitle`.
///
/// Best-effort at every step: a blank text (a view whose `{name}` has
/// not resolved) or an absent bridge (the element connected before the
/// bootstrap) leaves the tab exactly as it is.
fn push_title(this: &HtmlElement) {
    let Some(text) = this.get_attribute("text").filter(|text| !text.is_empty()) else {
        return;
    };
    let Some(tonk) = window_tonk() else {
        return;
    };
    let Some(set_title) = get_fn(&tonk, "setTitle") else {
        return;
    };
    let _ = set_title.call1(&tonk, &JsValue::from_str(&text));
}

/// `window.tonk`, if the portal bootstrap installed it.
///
/// A deliberate local copy of the same helper in `tonk-guest`'s
/// `guest_host.rs`: `tonk-guest` depends on this crate, not the other
/// way round, so it cannot be imported. Twelve lines of boilerplate is
/// a smaller cost than hoisting a shared module through `tonk-host`
/// for a second caller. Hoist if a third appears.
fn window_tonk() -> Option<Object> {
    let win = window()?;
    Reflect::get(&win, &JsValue::from_str("tonk"))
        .ok()
        .and_then(|value| value.dyn_into::<Object>().ok())
}

/// A callable property off `window.tonk`.
fn get_fn(tonk: &Object, name: &str) -> Option<Function> {
    Reflect::get(tonk, &JsValue::from_str(name))
        .ok()
        .and_then(|value| value.dyn_into::<Function>().ok())
}

/// Register `<tonk-title>`. Call once from the guest's element surface.
pub fn register() {
    TitleElement::define("tonk-title");
}
```

Add to `rust/tonk-portal/src/lib.rs`, beside the existing `mod site;` declaration:

```rust
#[cfg(target_arch = "wasm32")]
mod title;
```

and beside the existing `pub use site::register as register_site;`:

```rust
#[cfg(target_arch = "wasm32")]
pub use title::register as register_title;
```

Add to `rust/tonk-guest/src/bin/guest.rs` in `start()`, immediately after the existing `tonk_portal::register_site();` call:

```rust
    // `<tonk-title>` names the browser tab. Headless: it renders nothing
    // and pushes its text to the host page, which owns `document.title`.
    tonk_portal::register_title();
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo check -p tonk-portal -p tonk-guest --target wasm32-unknown-unknown --all-targets`
Expected: PASS.

Run: `cargo test -p tonk-portal --target wasm32-unknown-unknown push_title`
Expected: PASS — `it_pushes_only_a_non_empty_text`, `it_does_nothing_without_a_bridge`.

Same harness caveat as Task 1.

- [ ] **Step 5: Commit**

```bash
git add rust/tonk-portal/src/title.rs rust/tonk-portal/src/lib.rs rust/tonk-guest/src/bin/guest.rs
git commit -m "feat(tonk-portal): add the headless <tonk-title> element"
```

---

### Task 4: The title view kind and its repository row

**Files:**
- Modify: `rust/tonk-core/assets/library/core.yaml` (insert after the `view/label!:` row ending at line 928, before the `# BOARD` banner at line 930)

**Interfaces:**
- Consumes: `<tonk-title>` (Task 3).
- Produces: the `tonk:view/title` view kind (template under `xyz.tonk.view/title`) and the `tonk:repository/title-view` row. Selected by Task 5 with `view=tonk:view/title`.

**Why a new view kind rather than reusing `tonk:view/label`:** `tonk:repository` already carries an editable name banner under the default `tonk:view` resolution, and a plain-text label under `xyz.tonk.view/label`. A third view needs its own attribute or it collides. `tonk:view/label`'s repository row renders `{name}` as bare text, which cannot carry an element.

- [ ] **Step 1: Add the view kind and row**

Insert into `rust/tonk-core/assets/library/core.yaml` between the `view/label!:` row (ends line 928) and the `# BOARD` banner (line 930):

```yaml
# A tab-title view kind. Its `display` template lives under a distinct
# attribute (`xyz.tonk.view/title`) so a model can carry the editable
# `tonk:view` banner, the plain `tonk:view/label` text, AND a title
# template without the three colliding under the default resolution.
# `<tonk-display>` selects it explicitly with `view=tonk:view/title`.
concept!: &view/title
  this: tonk:view/title
  description: A browser-tab-title view selected via `view=tonk:view/title`.
  with:
    model:
      description: Concept this view renders
      the: xyz.tonk.view/model
      cardinality: one
      as: entity
    display:
      description: HTML template for the title view
      the: xyz.tonk.view/title
      cardinality: one
      as: text

# The repository's name as the browser tab's title. The space chrome
# mounts this via `<tonk-display model=tonk:repository entity={id}
# view=tonk:view/title>` inside a per-space `with={id}` routing context,
# so the tab names the focused spot (read from that space's
# `tonk/repository`, the cross-device source of truth). `<tonk-title>`
# renders nothing — it pushes the text to the host page, which owns
# `document.title`.
view/title!:
  this: id:tonk:repository/title-view
  model: tonk:repository
  display: |
    <tonk-title text="{name} — Tonk"></tonk-title>
```

- [ ] **Step 2: Verify the notation still analyzes**

Run: `cargo check -p tonk-worker --target wasm32-unknown-unknown`
Expected: PASS — `core.yaml` is embedded with `include_str!`, so this catches a file that cannot be read, but NOT a notation error.

Be honest about the gap: the test that actually runs `core.yaml` through `parse → analyze → commit` is `it_seeds_blank_scaffold` in `rust/tonk-worker/src/router/repository.rs`, and its module is **wasm-gated** (`run_in_service_worker`, `repository.rs:3579`) — it does not run under native `cargo test`.

Run it if the wasm harness is available:
Run: `cargo test -p tonk-worker --target wasm32-unknown-unknown it_seeds_blank_scaffold`
Expected: PASS.

If the harness cannot start, the real gate is Task 6's browser check: seeding runs at spot creation, so a malformed scaffold breaks creating a new spot loudly and immediately. Do not skip Task 6.

- [ ] **Step 3: Commit**

```bash
git add rust/tonk-core/assets/library/core.yaml
git commit -m "feat(tonk-core): add a tab-title view kind for the repository name"
```

---

### Task 5: Mount the title in the space chrome

**Files:**
- Modify: `rust/tonk-core/assets/library/profile.yaml:2075-2079` (the `&space-view` `display:` block)

**Interfaces:**
- Consumes: `tonk:view/title` + `tonk:repository/title-view` (Task 4).
- Produces: the live tab title. Nothing depends on this.

- [ ] **Step 1: Add the mount**

In `rust/tonk-core/assets/library/profile.yaml`, replace the `display:` block of `view!: &space-view` (lines 2075-2079) with:

```yaml
view!: &space-view
  this: id:tonk:space/view
  model: tonk:space/chrome
  display: |
    <tonk-site with="main@{id}" allow="main@{id}" path={rest}></tonk-site>
    <tonk-display with="main@profile:tonk" model="tonk:profile/fab" data-space="{id}"></tonk-display>
    <tonk-display with={id} entity={id} model=tonk:repository view=tonk:view/title>
      <tonk-title slot="no-model" text="Untitled — Tonk"></tonk-title>
      <tonk-title slot="no-entity" text="Untitled — Tonk"></tonk-title>
    </tonk-display>
```

Three things about that mount are deliberate:

- **It must stay in this view.** This view renders in a depth-1 guest, whose parent is the real page. Moving the mount into a spot's own route table puts it at depth 2, where it would retitle an intermediate iframe instead — silently.
- **No `slot="loading"`.** While the name is in flight the tab should keep what it already reads rather than flash "Untitled".
- **No `slot="no-view"`.** A spot created before Task 4 has no title-view row and resolves no view; giving it a title would pin every pre-existing spot to a permanent "Untitled — Tonk", which is worse than the honest "Tonk".

The `with={id} entity={id} … view=` shape mirrors the FAB's own name chip at `profile.yaml:859` and the hub card at `profile.yaml:722`.

- [ ] **Step 2: Verify the notation still analyzes**

Run: `cargo check -p tonk-worker --target wasm32-unknown-unknown`
Expected: PASS.

Same gating caveat as Task 4, Step 2 — Task 6 is the real gate.

- [ ] **Step 3: Commit**

```bash
git add rust/tonk-core/assets/library/profile.yaml
git commit -m "feat(tonk-core): title the tab with the focused spot's name"
```

---

### Task 6: Verify in the browser

**Files:** none — this is the gate the wasm-gated tests cannot be.

The whole rendered chain (name fact → view template → attribute → bridge → `document.title`) only exists in a real browser with a real service worker. Everything before this task is unverified end-to-end.

- [ ] **Step 1: Confirm the workspace is green**

Run: `cargo clippy --all-targets --all-features` then `cargo fmt --check`
Expected: PASS, no warnings.

Note `--all-features` matters: it compiles integration tests, so a per-crate or no-features clippy can be green while the gate fails.

- [ ] **Step 2: Build and serve the UI**

Run: `cd rust/tonk-ui && trunk serve`
Expected: a served dev build; open the printed URL.

- [ ] **Step 3: Walk the four states**

Check each, in order. The first is also the gate on Task 4's notation — a malformed scaffold fails spot creation outright.

1. **A fresh spot titles its tab.** Create a new spot. Its tab should read `<name> — Tonk`. If creation itself fails, the `core.yaml` edit is malformed — fix Task 4 before going on.
2. **Rename retitles live.** Rename the spot from the FAB name chip. The tab should follow on commit (Enter or blur), with no reload.
3. **An unnamed spot reads "Untitled — Tonk".** A spot whose name fact has not landed.
4. **A pre-existing spot still reads "Tonk".** A spot created before this change has no title-view row. This is the documented limitation, not a bug.

- [ ] **Step 4: Confirm the depth assumption held**

This is the assumption most likely to be wrong, and it fails silently rather than loudly.

In devtools, confirm the **top-level** document's title changed — not an iframe's. Check the browser tab itself, and run `document.title` in the console with the execution context set to the **top** frame.

If the tab is unchanged but the feature seems otherwise wired, the message is landing on an intermediate document: re-check that the mount is in `&space-view` in `profile.yaml` and nowhere deeper.

- [ ] **Step 5: Record the outcome**

State plainly which of the four states passed and which did not. If a wasm test was skipped in an earlier task because the harness could not start, say so here rather than letting it read as passing.

---

## Notes for the implementer

- **Order matters.** Tasks 1-3 are bottom-up; each compiles against the one before. Tasks 4-5 supply the data. Nothing is observable until Task 5, and nothing is verified until Task 6.
- **The wasm test harness is a known local gap.** Per the repo's history, running wasm tests on this machine needs Safari automation enabled or Chrome at the default `/Applications` path with a major-matched chromedriver. A harness that will not start is not a red test — but it does mean Task 6 is load-bearing, not optional.
- **Two guards, deliberately.** Both `title_text` (parse) and `set_title` (assignment) reject an empty string, and `push_title` drops one before it is sent. Belt and braces: a blank `{name}` renders routinely, and wiping a correct title is a visible regression.
