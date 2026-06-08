//! `<tonk-fallback>` — a launchpad region shown only while its
//! enclosing `<tonk-display>` is empty.
//!
//! A directory `<tonk-display>` reflects its lifecycle as `data-state`
//! on itself: `loading` while resolving, `empty` when the collection
//! has zero instances, `ready` once at least one renders. This element
//! is sibling *chrome* inside a directory view (it references no subject
//! field, so the renderer keeps it mounted regardless of instance
//! count). It finds its nearest `<tonk-display>` ancestor, mirrors that
//! host's emptiness onto its own `hidden` state, and watches the host so
//! the flip is live: the launchpad shows on an empty repo and vanishes
//! the moment the first instance lands — no author CSS, no reload.

#[cfg(target_arch = "wasm32")]
use std::cell::RefCell;

#[cfg(target_arch = "wasm32")]
use custom_elements::CustomElement;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::{JsCast, JsValue, closure::Closure};
#[cfg(target_arch = "wasm32")]
use web_sys::{HtmlElement, MutationObserver, MutationObserverInit};

/// The `data-state` value a `<tonk-display>` carries when its
/// collection has zero instances.
#[cfg(target_arch = "wasm32")]
const EMPTY_STATE: &str = "empty";

/// Retained observer + its callback closure, dropped on disconnect.
#[cfg(target_arch = "wasm32")]
#[derive(Default)]
struct Inner {
    observer: Option<MutationObserver>,
    _callback: Option<Closure<dyn FnMut()>>,
}

/// The custom element.
#[cfg(target_arch = "wasm32")]
#[derive(Default)]
pub struct TonkFallback {
    inner: RefCell<Inner>,
}

#[cfg(target_arch = "wasm32")]
impl CustomElement for TonkFallback {
    fn shadow() -> bool {
        false
    }

    fn observed_attributes() -> &'static [&'static str] {
        &[]
    }

    fn inject_children(&mut self, _this: &HtmlElement) {}

    fn connected_callback(&mut self, this: &HtmlElement) {
        sync(this);
        observe_host(this, &self.inner);
    }

    fn disconnected_callback(&mut self, _this: &HtmlElement) {
        let mut inner = self.inner.borrow_mut();
        if let Some(observer) = inner.observer.take() {
            observer.disconnect();
        }
        inner._callback = None;
    }
}

/// Reflect the enclosing display's emptiness onto this element's
/// `hidden` state: visible only when the nearest `<tonk-display>`
/// ancestor is `data-state="empty"`. With no such ancestor (the element
/// used outside a display) it stays hidden.
#[cfg(target_arch = "wasm32")]
fn sync(this: &HtmlElement) {
    let empty = this
        .closest("tonk-display")
        .ok()
        .flatten()
        .and_then(|host| host.get_attribute("data-state"))
        .map(|state| state == EMPTY_STATE)
        .unwrap_or(false);
    this.set_hidden(!empty);
}

/// Observe the enclosing display's `data-state` so the fallback's
/// visibility tracks it live — the launchpad hides the moment the first
/// instance lands (state flips `empty` -> `ready`) and reappears if the
/// collection drains back to zero. The observer + its closure are
/// retained on the element and dropped in `disconnected_callback`.
#[cfg(target_arch = "wasm32")]
fn observe_host(this: &HtmlElement, inner: &RefCell<Inner>) {
    let Some(host) = this.closest("tonk-display").ok().flatten() else {
        return;
    };
    let element = this.clone();
    let callback = Closure::wrap(Box::new(move || sync(&element)) as Box<dyn FnMut()>);
    let Ok(observer) = MutationObserver::new(callback.as_ref().unchecked_ref()) else {
        return;
    };
    let init = MutationObserverInit::new();
    init.set_attributes(true);
    init.set_attribute_filter(&js_sys::Array::of1(&JsValue::from_str("data-state")));
    let _ = observer.observe_with_options(&host, &init);

    let mut slot = inner.borrow_mut();
    slot.observer = Some(observer);
    slot._callback = Some(callback);
}

/// Register the `<tonk-fallback>` custom element. Idempotent.
#[cfg(target_arch = "wasm32")]
pub fn register() {
    if already_registered() {
        return;
    }
    TonkFallback::define("tonk-fallback");
}

#[cfg(target_arch = "wasm32")]
fn already_registered() -> bool {
    let Some(win) = web_sys::window() else {
        return false;
    };
    !win.custom_elements().get("tonk-fallback").is_undefined()
}

#[cfg(all(test, target_arch = "wasm32"))]
mod tests {
    use super::*;
    use wasm_bindgen::JsCast;
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    wasm_bindgen_test_configure!(run_in_browser);

    async fn tick() {
        // Let queued MutationObserver microtasks flush.
        let promise = js_sys::Promise::new(&mut |resolve, _| {
            let _ = web_sys::window()
                .unwrap()
                .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, 0);
        });
        let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
    }

    #[dialog_common::test]
    async fn it_shows_only_when_the_ancestor_display_is_empty() {
        register();
        let document = web_sys::window().unwrap().document().unwrap();

        // A bare `<tonk-display>` ancestor carrying a `data-state`.
        let host = document.create_element("tonk-display").unwrap();
        host.set_attribute("data-state", "empty").unwrap();
        document.body().unwrap().append_child(&host).unwrap();

        let fallback = document.create_element("tonk-fallback").unwrap();
        fallback.set_inner_html("<p>nothing yet</p>");
        host.append_child(&fallback).unwrap();
        tick().await;

        let fb: HtmlElement = fallback.dyn_into().unwrap();
        assert!(
            !fb.hidden(),
            "fallback should be visible while the display is empty",
        );

        // Content lands -> the host flips to `ready`, the fallback hides.
        host.set_attribute("data-state", "ready").unwrap();
        tick().await;
        assert!(
            fb.hidden(),
            "fallback should hide once the display is no longer empty",
        );

        // Collection drains back to empty -> the fallback reappears.
        host.set_attribute("data-state", "empty").unwrap();
        tick().await;
        assert!(
            !fb.hidden(),
            "fallback should reappear when the display goes empty again",
        );
    }
}
