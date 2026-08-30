//! Setting the host page's tab title on a guest's behalf.
//!
//! `document.title` exists only on the top page. The chrome that knows
//! a space's name renders inside a sealed guest, which cannot reach the
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
/// wipe a title that was already correct. The guard runs before
/// forwarding so a blank render dies at its source rather than being
/// posted up the frame chain for each parent to re-drop. (`navigate_to`
/// forwards as its very first statement — it has no such guard, so the
/// two differ deliberately.)
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

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod tests {
    use super::*;
    use js_sys::{Array, Function, Object, Reflect};
    use wasm_bindgen::JsCast;
    use wasm_bindgen::JsValue;
    use wasm_bindgen::closure::Closure;
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
        let _ = Reflect::delete_property(win.unchecked_ref::<Object>(), &JsValue::from_str("tonk"));
    }

    /// In a guest, a title is posted to the parent rather than written to the
    /// guest's own (invisible) document title.
    #[dialog_common::test]
    async fn it_forwards_a_title_from_a_guest_instead_of_setting_it() {
        set_title("Before — Tonk");
        let calls = install_title_stub();

        set_title("Forwarded — Tonk");

        let title_after = document_title();
        // Restore BEFORE asserting — a panic unwinds past any cleanup placed
        // after the assertions, leaking the stub into every later test in this
        // binary (it would make the test above forward its title instead of
        // setting it). `bridge.rs:2643` does the same, for the same reason.
        clear_tonk();

        assert_eq!(calls.length(), 1, "the parent should have been called once");
        assert_eq!(
            calls.get(0).as_string(),
            Some("Forwarded — Tonk".to_owned()),
            "the text should reach the parent verbatim"
        );
        assert_eq!(
            title_after, "Before — Tonk",
            "a forwarded title must not retitle this document"
        );
    }

    /// The empty guard runs BEFORE forwarding: a blank render is dropped at
    /// its source rather than posted up the frame chain for each parent to
    /// re-drop.
    #[dialog_common::test]
    async fn it_does_not_forward_an_empty_title() {
        let calls = install_title_stub();

        set_title("");

        clear_tonk(); // before asserting — see above
        assert_eq!(calls.length(), 0, "an empty title should not be forwarded");
    }
}
