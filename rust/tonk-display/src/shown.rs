//! Whether a display is shown, and waiting until it is.
//!
//! A display marked `loading="lazy"` that the browser does not render —
//! `hidden`, inside a closed dialog, under `display: none` — has nothing to
//! show, so it stays inert: it resolves no model and opens no subscription
//! until it is laid out. Panels, menus and dialogs marked lazy then cost
//! nothing until someone opens them. A display is not lazy unless marked:
//! a hidden one is often what a page reads its data from.

use js_sys::{Array, Function, Reflect};
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use web_sys::{Element, IntersectionObserver, IntersectionObserverEntry, IntersectionObserverInit};

/// A wait for a display to be shown. Dropping it stops waiting.
pub(crate) struct Waiting {
    observer: IntersectionObserver,
    _callback: Closure<dyn FnMut(Array)>,
}

impl Drop for Waiting {
    fn drop(&mut self) {
        self.observer.disconnect();
    }
}

/// Whether the browser renders `host`. A browser that cannot say counts as
/// rendering it.
pub(crate) fn shown(host: &Element) -> bool {
    let Some(target) = laid_out(host) else {
        return true;
    };
    match Reflect::get(&target, &"checkVisibility".into()) {
        Ok(check) if check.is_function() => check
            .unchecked_into::<Function>()
            .call0(&target)
            .map_or(true, |visible| visible.is_truthy()),
        _ => true,
    }
}

/// Call `start` once `host` is shown. The browser reports an element
/// intersecting once it is laid out; the margin is wide enough that one laid
/// out anywhere on the page counts, so a display waits for being rendered,
/// not for being scrolled to.
pub(crate) fn when_shown(host: &Element, start: impl FnOnce() + 'static) -> Option<Waiting> {
    let target = laid_out(host)?;
    let mut start = Some(start);
    let callback = Closure::<dyn FnMut(Array)>::new(move |entries: Array| {
        let intersecting = entries.iter().any(|entry| {
            entry
                .dyn_into::<IntersectionObserverEntry>()
                .is_ok_and(|entry| entry.is_intersecting())
        });
        if intersecting && let Some(start) = start.take() {
            start();
        }
    });
    let options = IntersectionObserverInit::new();
    options.set_root_margin("100000px");
    let observer =
        IntersectionObserver::new_with_options(callback.as_ref().unchecked_ref(), &options).ok()?;
    observer.observe(&target);
    Some(Waiting {
        observer,
        _callback: callback,
    })
}

/// The element whose box shows `host`: itself, or for a `display: contents`
/// host, which has no box of its own, the nearest ancestor that has one.
/// None where no ancestor in reach does.
fn laid_out(host: &Element) -> Option<Element> {
    let window = web_sys::window()?;
    let mut element = host.clone();
    loop {
        let display = window
            .get_computed_style(&element)
            .ok()
            .flatten()
            .and_then(|style| style.get_property_value("display").ok())
            .unwrap_or_default();
        if display != "contents" {
            return Some(element);
        }
        element = element.parent_element()?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::rc::Rc;
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    use web_sys::window;

    wasm_bindgen_test_configure!(run_in_browser);

    async fn sleep_ms(ms: i32) {
        let promise = js_sys::Promise::new(&mut |resolve, _| {
            let _ = window()
                .expect("window")
                .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms);
        });
        let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
    }

    /// An element under a hidden container, mounted in the document.
    fn hidden_element() -> (Element, Element) {
        let document = window().expect("window").document().expect("document");
        let container = document.create_element("div").expect("a container");
        container.set_attribute("hidden", "").expect("hidden");
        let element = document.create_element("span").expect("an element");
        element.set_text_content(Some("shown"));
        container.append_child(&element).expect("append");
        document
            .body()
            .expect("body")
            .append_child(&container)
            .expect("mount");
        (container, element)
    }

    #[dialog_common::test]
    async fn it_says_an_element_under_a_hidden_container_is_not_shown() {
        let (container, element) = hidden_element();
        assert!(!shown(&element), "hidden");
        container.remove_attribute("hidden").expect("show");
        assert!(shown(&element), "shown");
        container.remove();
    }

    #[dialog_common::test]
    async fn it_waits_until_an_element_is_shown() {
        let (container, element) = hidden_element();
        let started = Rc::new(Cell::new(false));
        let _waiting = when_shown(&element, {
            let started = started.clone();
            move || started.set(true)
        })
        .expect("a wait");

        sleep_ms(100).await;
        assert!(!started.get(), "nothing starts while it is hidden");

        container.remove_attribute("hidden").expect("show");
        for _ in 0..40 {
            if started.get() {
                break;
            }
            sleep_ms(25).await;
        }
        container.remove();
        assert!(started.get(), "it starts once it is shown");
    }
}
