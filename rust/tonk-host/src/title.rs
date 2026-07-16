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
