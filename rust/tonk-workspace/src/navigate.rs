//! `<tonk-navigate>` — performs a client-side navigation from a fact.
//!
//! The page-bound half of Elm's `pushUrl`: a worker handler asserts a
//! `tonk:navigate` fact carrying a destination, a view renders it onto this
//! element's `href`, and the element navigates the page there. Navigation
//! is a *page* capability the service worker lacks (no `window`), so the
//! intent travels as data on the branch and the act runs here.
//!
//! ```html
//! <tonk-navigate href="/space/did:key:…"></tonk-navigate>
//! ```
//!
//! It is the mirror of `<tonk-page>` (page location → data): this is data →
//! page location. Navigating once an `href` is present, it assigns
//! `window.location` so the destination becomes a real page load (the SPA
//! router then resolves the route).
//!
//! Fires once per non-empty `href`. The element may be created with an
//! empty/absent `href` (the fact not yet resolved) and have it set later by
//! the display reconcile; it navigates the first time `href` is non-empty,
//! then guards against re-navigating.

use custom_elements::CustomElement;
use web_sys::{HtmlElement, window};

/// Per-element state: a fired-once guard so a later attribute reconcile
/// doesn't re-navigate.
#[derive(Default)]
pub(crate) struct TonkNavigate {
    navigated: bool,
}

impl TonkNavigate {
    /// Navigate to `this`'s `href` if it is a resolved, non-empty value and
    /// we haven't navigated yet. An `href` still carrying a `{…}` template
    /// placeholder means the display binding hasn't substituted the fact's
    /// value yet — navigating then would land on a literal `/{href}`, so we
    /// wait for the real value.
    fn navigate_if_ready(&mut self, this: &HtmlElement) {
        if self.navigated {
            return;
        }
        let href = this.get_attribute("href").unwrap_or_default();
        if href.is_empty() || href.contains('{') {
            return;
        }
        self.navigated = true;
        if let Some(location) = window().map(|w| w.location()) {
            let _ = location.assign(&href);
        }
    }
}

impl CustomElement for TonkNavigate {
    fn shadow() -> bool {
        false
    }

    fn observed_attributes() -> &'static [&'static str] {
        // React to `href` being set after connect (the fact resolves
        // asynchronously, so the element often connects href-less).
        &["href"]
    }

    fn inject_children(&mut self, _this: &HtmlElement) {}

    fn connected_callback(&mut self, this: &HtmlElement) {
        self.navigate_if_ready(this);
    }

    fn attribute_changed_callback(
        &mut self,
        this: &HtmlElement,
        _name: String,
        _old: Option<String>,
        _new: Option<String>,
    ) {
        self.navigate_if_ready(this);
    }
}

/// Register `<tonk-navigate>`. Idempotent.
pub(crate) fn register() {
    if already_registered() {
        return;
    }
    TonkNavigate::define("tonk-navigate");
}

fn already_registered() -> bool {
    let Some(win) = window() else {
        return false;
    };
    !win.custom_elements().get("tonk-navigate").is_undefined()
}

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod tests {
    use super::*;
    use wasm_bindgen::JsCast as _;
    use wasm_bindgen_test::wasm_bindgen_test_configure;

    wasm_bindgen_test_configure!(run_in_browser);

    /// A `<tonk-navigate>` with no `href`, or an empty one, must not arm —
    /// otherwise an href-less connect (the fact not yet resolved) would
    /// navigate to nowhere. We exercise only the no-navigation cases: a
    /// positive case would assign `window.location` and tear the harness
    /// out from under the test.
    #[dialog_common::test]
    async fn it_does_not_navigate_without_an_href() {
        let document = window().unwrap().document().unwrap();
        let host: HtmlElement = document
            .create_element("tonk-navigate")
            .unwrap()
            .dyn_into()
            .unwrap();

        let mut state = TonkNavigate::default();

        // No href attribute — must not arm.
        state.navigate_if_ready(&host);
        assert!(!state.navigated, "must not navigate without an href");

        // Empty href — still must not arm.
        host.set_attribute("href", "").unwrap();
        state.navigate_if_ready(&host);
        assert!(!state.navigated, "must not navigate on an empty href");

        // An unsubstituted `{…}` template placeholder — must not arm, or we
        // would navigate to a literal `/{href}`.
        host.set_attribute("href", "/{href}").unwrap();
        state.navigate_if_ready(&host);
        assert!(
            !state.navigated,
            "must not navigate on an unsubstituted placeholder"
        );
    }
}
