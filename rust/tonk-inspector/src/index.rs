//! `<tonk-notebook-index>` — the notebook directory's search-and-create box.
//!
//! The directory view renders one `.notebook-index__item` link per notebook
//! plus a form at the top. This element makes that form do two jobs at once:
//!
//! - **Filter.** Typing hides the rows that do not match. This is a VIEW
//!   concern: nothing is written, so a search must never produce a fact.
//! - **Create.** Submitting asserts `notebook/create` with the typed title,
//!   which is an ordinary command and goes through the library like any
//!   other.
//!
//! The element wraps the rows rather than generating them, so if its script
//! never loads the directory still renders as a plain, working list of
//! links — the filter is an enhancement, not a dependency.

use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use web_sys::{Element, Event, HtmlElement};

use custom_elements::CustomElement;

/// Whether a notebook titled `title` should show while `query` is typed.
///
/// Case-insensitive substring, not a prefix: a notebook is as often
/// remembered by a word in the middle of its name as by how it starts.
/// A blank query matches everything, so clearing the box restores the list.
pub fn matches(title: &str, query: &str) -> bool {
    let query = query.trim();
    if query.is_empty() {
        return true;
    }
    title.to_lowercase().contains(&query.to_lowercase())
}

/// Whether the typed query names an EXISTING notebook exactly.
///
/// Submitting is "open it if it exists, else create it", and that hinges on
/// an exact (case- and space-insensitive) title match rather than on whether
/// anything is still visible: typing `Not` while `Notes` exists should
/// create `Not`, not open `Notes`.
pub fn exact<'a>(titles: impl Iterator<Item = (&'a str, &'a str)>, query: &str) -> Option<String> {
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return None;
    }
    titles
        .filter(|(title, _)| title.trim().to_lowercase() == needle)
        .map(|(_, href)| href.to_owned())
        .next()
}

/// The directory's search box.
#[derive(Default)]
pub struct TonkNotebookIndexElement;

impl TonkNotebookIndexElement {
    /// The rows the view rendered, as `(title, href)` plus the row element.
    fn rows(host: &HtmlElement) -> Vec<(String, String, HtmlElement)> {
        let Ok(items) = host.query_selector_all(".notebook-index__item") else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for index in 0..items.length() {
            let Some(row) = items
                .item(index)
                .and_then(|node| node.dyn_into::<HtmlElement>().ok())
            else {
                continue;
            };
            let title = row
                .query_selector(".notebook-index__title")
                .ok()
                .flatten()
                .map(|el| el.text_content().unwrap_or_default())
                .unwrap_or_default();
            let href = row.get_attribute("href").unwrap_or_default();
            out.push((title, href, row));
        }
        out
    }

    /// The current query, read off the `wa-input`.
    ///
    /// Through `value` on the element itself, not `FormData`: a `wa-input`
    /// is a form-associated custom element and FormData reads back empty
    /// for it.
    fn query(host: &HtmlElement) -> String {
        host.query_selector(".notebook-index__query")
            .ok()
            .flatten()
            .and_then(|el| js_sys::Reflect::get(&el, &"value".into()).ok())
            .and_then(|value| value.as_string())
            .unwrap_or_default()
    }

    /// Show the rows that match and hide the rest, and say so when none do.
    fn filter(host: &HtmlElement) {
        let query = Self::query(host);
        let mut shown = 0usize;
        for (title, _, row) in Self::rows(host) {
            let visible = matches(&title, &query);
            if visible {
                shown += 1;
            }
            let _ = row.set_attribute("hidden", "");
            if visible {
                let _ = row.remove_attribute("hidden");
            }
        }
        // The "nothing matches" hint belongs to a non-empty query only: an
        // empty space with an empty box is not a failed search.
        if let Ok(Some(empty)) = host.query_selector(".notebook-index__empty") {
            let show = shown == 0 && !query.trim().is_empty();
            let _ = empty.set_attribute("hidden", "");
            if show {
                let _ = empty.remove_attribute("hidden");
            }
        }
    }
}

impl CustomElement for TonkNotebookIndexElement {
    fn shadow() -> bool {
        // Light DOM: the rows are rendered by `<tonk-display>` into this
        // element's children, and the app stylesheet styles them.
        false
    }

    fn inject_children(&mut self, _this: &HtmlElement) {}

    fn connected_callback(&mut self, this: &HtmlElement) {
        let host = this.clone();

        // Filter as the box changes. `input` (not `change`) so the list
        // narrows while typing rather than on blur.
        let typing = host.clone();
        let on_input = Closure::<dyn FnMut(Event)>::new(move |_: Event| {
            TonkNotebookIndexElement::filter(&typing);
        });
        let _ = host.add_event_listener_with_callback("input", on_input.as_ref().unchecked_ref());
        on_input.forget();

        // Submitting opens an exact match instead of creating a duplicate.
        //
        // The library's `notebook/create` fires on this same submit, so when
        // the title already exists we must stop the event reaching it —
        // otherwise looking up a notebook silently creates a second one with
        // the same name.
        let submitting = host.clone();
        let on_submit = Closure::<dyn FnMut(Event)>::new(move |event: Event| {
            let query = TonkNotebookIndexElement::query(&submitting);
            let rows = TonkNotebookIndexElement::rows(&submitting);
            let existing = exact(
                rows.iter()
                    .map(|(title, href, _)| (title.as_str(), href.as_str())),
                &query,
            );
            if let Some(href) = existing {
                event.prevent_default();
                event.stop_propagation();
                if let Some(window) = web_sys::window() {
                    let _ = window.location().set_href(&href);
                }
            }
        });
        // CAPTURE phase: the command handler is bound on the form, and a
        // bubbling listener here would run after it has already created the
        // notebook.
        let _ = host.add_event_listener_with_callback_and_bool(
            "submit",
            on_submit.as_ref().unchecked_ref(),
            true,
        );
        on_submit.forget();

        // The rows arrive from a `<tonk-display>` render that may land after
        // this callback, so re-filter whenever the child list changes —
        // otherwise a query typed before the data lands is ignored.
        let observing = host.clone();
        let on_mutate = Closure::<dyn FnMut(js_sys::Array)>::new(move |_: js_sys::Array| {
            TonkNotebookIndexElement::filter(&observing);
        });
        if let Ok(observer) = web_sys::MutationObserver::new(on_mutate.as_ref().unchecked_ref()) {
            let options = web_sys::MutationObserverInit::new();
            options.set_child_list(true);
            options.set_subtree(true);
            let _ = observer.observe_with_options(host.as_ref() as &Element, &options);
        }
        on_mutate.forget();

        Self::filter(&host);
    }
}

#[cfg(test)]
mod tests {
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    use super::*;

    /// A blank box shows everything, so clearing the query restores the list.
    #[dialog_common::test]
    fn it_matches_everything_on_a_blank_query() {
        assert!(matches("Notes", ""));
        assert!(matches("Notes", "   "));
    }

    /// Substring, not prefix: a notebook is as often remembered by a word in
    /// the middle of its name.
    #[dialog_common::test]
    fn it_matches_a_word_inside_the_title() {
        assert!(matches("Weekly Planning", "plan"));
        assert!(!matches("Weekly Planning", "zzz"));
    }

    #[dialog_common::test]
    fn it_matches_regardless_of_case() {
        assert!(matches("Weekly Planning", "WEEKLY"));
        assert!(matches("weekly planning", "Planning"));
    }

    /// An exact title opens rather than creating a duplicate.
    #[dialog_common::test]
    fn it_finds_an_exact_title() {
        let rows = [("Notes", "notebook/id:a"), ("Plans", "notebook/id:b")];
        assert_eq!(
            exact(rows.iter().copied(), "Notes"),
            Some("notebook/id:a".to_owned())
        );
    }

    /// Exact means exact: a prefix creates a new notebook rather than
    /// opening the one it happens to be a prefix of.
    #[dialog_common::test]
    fn it_finds_no_exact_title_for_a_prefix() {
        let rows = [("Notes", "notebook/id:a")];
        assert_eq!(exact(rows.iter().copied(), "Not"), None);
    }

    /// Case and surrounding space do not make a different notebook.
    #[dialog_common::test]
    fn it_finds_an_exact_title_ignoring_case_and_space() {
        let rows = [("Notes", "notebook/id:a")];
        assert_eq!(
            exact(rows.iter().copied(), "  notes "),
            Some("notebook/id:a".to_owned())
        );
    }

    /// A blank query never opens anything.
    #[dialog_common::test]
    fn it_finds_no_exact_title_for_a_blank_query() {
        let rows = [("Notes", "notebook/id:a")];
        assert_eq!(exact(rows.iter().copied(), "  "), None);
    }
}
