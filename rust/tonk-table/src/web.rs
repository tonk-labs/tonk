use wasm_bindgen::JsCast;
use web_sys::{Document, HtmlScriptElement};

/// Inject the `<tonk-table>` JS module into `<head>` exactly once.
///
/// The bundled module self-registers the custom element on first
/// import; calling [`install`] more than once is a no-op so it's
/// safe to put in `bin/ui.rs` alongside the other element installs.
///
/// `src` is the URL the bundle has been copied to by Trunk —
/// pass `/tonk-table/tonk-table.js` if you've used the matching
/// `copy-dir` link in `index.html`. Routing the URL through the
/// caller (rather than hardcoding it here) keeps deployment-path
/// concerns in one place: whichever crate owns `index.html` decides
/// where assets live.
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

const SCRIPT_ID: &str = "tonk-table-loader";

fn already_injected(document: &Document) -> bool {
    document.get_element_by_id(SCRIPT_ID).is_some()
}
