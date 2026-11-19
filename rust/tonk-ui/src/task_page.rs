//! What Tonk's task pages in the top document share: gooey's in-browser
//! system (`/in-browser/fabb-pages.css`, `fabb-forms.css`, `<tonk-message>`,
//! loaded by `index.html`), the logotype, and how a button waits.

/// The logotype at the top of a task page, gooey's `.fabb-logo`.
pub(crate) const LOGOTYPE: &str = include_str!("logotype.svg");

/// A task page's markup with its `<!-- logotype -->` placeholder filled.
pub(crate) fn page(markup: &str) -> String {
    markup.replace("<!-- logotype -->", LOGOTYPE)
}

/// How long a request may take before its button says it is waiting, so a
/// quick answer never flickers.
const WAIT_WORDS_AFTER_MS: i32 = 400;

/// A request is out: past 400ms the button says so in its `data-wait`
/// words, with `aria-busy` and `aria-disabled`, at its width (fabb-forms.css
/// keeps the longer of the two words' widths). It stays where it is and is
/// never greyed out. The caller blocks a second press itself, at once.
pub(crate) fn wait(button: &web_sys::Element) {
    use wasm_bindgen::JsCast;
    use wasm_bindgen::closure::Closure;
    let Some(window) = web_sys::window() else {
        return;
    };
    if button.get_attribute("data-label").is_none() {
        let _ = button.set_attribute("data-label", &button.text_content().unwrap_or_default());
    }
    let target = button.clone();
    let show = Closure::once_into_js(move || {
        if target.get_attribute("data-waiting").is_none() {
            return;
        }
        // fabb-forms.css reserves the waiting words' width; a longer label
        // is held here, so the button keeps the width it was pressed at.
        if let Ok(element) = target.clone().dyn_into::<web_sys::HtmlElement>() {
            let width = element.get_bounding_client_rect().width();
            let _ = element
                .style()
                .set_property("min-width", &format!("{width}px"));
        }
        if let Some(words) = target.get_attribute("data-wait") {
            target.set_text_content(Some(&words));
        }
        let _ = target.set_attribute("aria-busy", "true");
        let _ = target.set_attribute("aria-disabled", "true");
    });
    let _ = button.set_attribute("data-waiting", "");
    let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(
        show.unchecked_ref(),
        WAIT_WORDS_AFTER_MS,
    );
}

/// The request answered: the button says its own words again. With `done`
/// it stays unpressable (`aria-disabled`), for an answer that can't be
/// given twice.
pub(crate) fn settle(button: &web_sys::Element, done: bool) {
    use wasm_bindgen::JsCast;
    let _ = button.remove_attribute("data-waiting");
    let _ = button.remove_attribute("aria-busy");
    if let Ok(element) = button.clone().dyn_into::<web_sys::HtmlElement>() {
        let _ = element.style().remove_property("min-width");
    }
    if let Some(label) = button.get_attribute("data-label") {
        button.set_text_content(Some(&label));
    }
    if done {
        let _ = button.set_attribute("aria-disabled", "true");
    } else {
        let _ = button.remove_attribute("aria-disabled");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_browser);

    #[dialog_common::test]
    fn it_puts_the_logotype_where_the_page_asks_for_it() {
        let markup = page(r#"<header class="fabb-top"><!-- logotype --></header>"#);
        assert!(markup.starts_with(r#"<header class="fabb-top"><svg class="fabb-logo""#));
        assert!(markup.contains(r#"aria-label="Tonk""#));
        assert!(!markup.contains("<!-- logotype -->"));
    }
}
