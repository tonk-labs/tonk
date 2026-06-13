use wasm_bindgen::JsCast;
use web_sys::{Document, HtmlScriptElement};

const SCRIPT_ID: &str = "tonk-tree-loader";

/// Inject the `<tonk-tree>` JS module into `<head>` exactly once.
///
/// The bundled module self-registers the custom element on first
/// import; calling [`install`] more than once is a no-op, so it's safe
/// to place alongside the other element installs in `bin/ui.rs`.
///
/// `src` is the URL the bundle has been copied to by Trunk — pass
/// `/tonk-tree/tonk-tree.js` for the matching `copy-dir` link in
/// `index.html`. Routing the URL through the caller keeps deployment-
/// path concerns with whichever crate owns `index.html`.
pub fn install(src: &str) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let Some(document) = window.document() else {
        return;
    };
    if already_injected(&document) {
        return;
    }
    let Ok(script) = document.create_element("script") else {
        return;
    };
    let Ok(script) = script.dyn_into::<HtmlScriptElement>() else {
        return;
    };
    script.set_type("module");
    script.set_src(src);
    script.set_id(SCRIPT_ID);
    if let Some(head) = document.head() {
        let _ = head.append_child(&script);
    }
}

fn already_injected(document: &Document) -> bool {
    document.get_element_by_id(SCRIPT_ID).is_some()
}
