//! `<ui-copy-link>` — the Hub verb that mints and copies a space invite.
//!
//! Profiles retain their seeded Hub view for life, so older profiles still
//! render this element with `url="/space/{subject}"`. The current view names
//! the subject directly with `space={subject}`. Both forms are accepted here:
//! the runtime wraps the shared `<tonk-share>` control, which mints a fresh
//! authorization and settles the clipboard with the resulting invite URL.
//! A member-only `/space/...` route is never itself copied.

use custom_elements::CustomElement;
use web_sys::{Element, HtmlElement, window};

/// The class the Hub stylesheet uses for its action button.
const VERB_CLASS: &str = "copy-verb";

/// The resting label. Overridable for seeded views that supplied `label`.
const DEFAULT_LABEL: &str = "copy link";

/// Per-element state lives in the nested `<tonk-share>` control.
#[derive(Default)]
pub(crate) struct UiCopyLink;

impl CustomElement for UiCopyLink {
    fn shadow() -> bool {
        false
    }

    fn observed_attributes() -> &'static [&'static str] {
        &["label", "space", "url"]
    }

    fn inject_children(&mut self, this: &HtmlElement) {
        if share_of(this).is_some() {
            return;
        }
        let Some(document) = window().and_then(|w| w.document()) else {
            return;
        };
        let Ok(share) = document.create_element("tonk-share") else {
            return;
        };
        let Ok(button) = document.create_element("button") else {
            return;
        };
        let _ = button.set_attribute("type", "button");
        let _ = button.set_attribute("class", VERB_CLASS);

        for (state, text) in [
            ("idle", label_of(this)),
            ("copying", "copying…".to_owned()),
            ("copied", "copied".to_owned()),
            ("failed", "couldn't copy".to_owned()),
        ] {
            let Ok(label) = document.create_element("span") else {
                continue;
            };
            let _ = label.set_attribute("data-share-copy-label", state);
            label.set_text_content(Some(&text));
            let _ = button.append_child(&label);
        }

        let _ = share.append_child(&button);
        let _ = this.append_child(&share);
        sync_space(this);
    }

    fn connected_callback(&mut self, this: &HtmlElement) {
        sync_space(this);
    }

    fn attribute_changed_callback(
        &mut self,
        this: &HtmlElement,
        name: String,
        old: Option<String>,
        new: Option<String>,
    ) {
        if old == new {
            return;
        }
        match name.as_str() {
            "label" => sync_label(this),
            "space" | "url" => sync_space(this),
            _ => {}
        }
    }
}

fn share_of(this: &HtmlElement) -> Option<Element> {
    this.query_selector("tonk-share").ok().flatten()
}

fn label_of(this: &HtmlElement) -> String {
    this.get_attribute("label")
        .filter(|label| !label.is_empty())
        .unwrap_or_else(|| DEFAULT_LABEL.to_owned())
}

fn sync_label(this: &HtmlElement) {
    let Ok(Some(label)) = this.query_selector("[data-share-copy-label=\"idle\"]") else {
        return;
    };
    label.set_text_content(Some(&label_of(this)));
}

/// Resolve the target from the current `space` contract or an older seeded
/// `/space/{subject}` URL. The route is only an address carrier; it is never
/// handed to the clipboard.
fn space_of(this: &HtmlElement) -> Option<String> {
    if let Some(space) = this
        .get_attribute("space")
        .filter(|space| !space.is_empty())
    {
        return Some(space);
    }
    let raw = this.get_attribute("url").filter(|url| !url.is_empty())?;
    let base = window()?.location().href().ok()?;
    let path = web_sys::Url::new_with_base(&raw, &base).ok()?.pathname();
    path.strip_prefix("/space/")
        .and_then(|rest| rest.split('/').next())
        .filter(|subject| !subject.is_empty())
        .map(str::to_owned)
}

fn sync_space(this: &HtmlElement) {
    let Some(share) = share_of(this) else {
        return;
    };
    match space_of(this) {
        Some(space) => {
            let _ = share.set_attribute("space", &space);
        }
        None => {
            let _ = share.remove_attribute("space");
        }
    }
}

/// Register `<ui-copy-link>`. Idempotent.
pub(crate) fn register() {
    let Some(win) = window() else {
        return;
    };
    if win.custom_elements().get("ui-copy-link").is_undefined() {
        UiCopyLink::define("ui-copy-link");
    }
}

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod tests {
    use super::*;
    use wasm_bindgen::JsCast;
    use wasm_bindgen_test::{wasm_bindgen_test, wasm_bindgen_test_configure};

    wasm_bindgen_test_configure!(run_in_browser);

    fn mount(markup: &str) -> HtmlElement {
        // Production registers workspace elements before FABB elements. The
        // nested `<tonk-share>` must therefore survive creation while still
        // unknown and upgrade once its implementation is registered.
        register();
        let document = window().unwrap().document().unwrap();
        let fixture = document.create_element("div").unwrap();
        fixture.set_inner_html(markup);
        document.body().unwrap().append_child(&fixture).unwrap();
        tonk_fab::register();
        fixture.unchecked_into()
    }

    #[wasm_bindgen_test]
    fn it_routes_the_current_space_contract_into_the_invite_control() {
        let fixture = mount(r#"<ui-copy-link space="did:key:z6MkCurrent"></ui-copy-link>"#);
        let share = fixture.query_selector("tonk-share").unwrap().unwrap();
        assert_eq!(
            share.get_attribute("space").as_deref(),
            Some("did:key:z6MkCurrent")
        );
        assert!(share.query_selector("button.copy-verb").unwrap().is_some());
        fixture.remove();
    }

    #[wasm_bindgen_test]
    fn it_upgrades_a_seeded_route_without_copying_that_route() {
        let fixture = mount(
            r#"<ui-copy-link url="/space/did:key:z6MkSeeded" label="copy link"></ui-copy-link>"#,
        );
        let share = fixture.query_selector("tonk-share").unwrap().unwrap();
        assert_eq!(
            share.get_attribute("space").as_deref(),
            Some("did:key:z6MkSeeded")
        );
        assert_eq!(
            fixture
                .query_selector("[data-share-copy-label=\"idle\"]")
                .unwrap()
                .unwrap()
                .text_content()
                .as_deref(),
            Some("copy link")
        );
        fixture.remove();
    }
}
