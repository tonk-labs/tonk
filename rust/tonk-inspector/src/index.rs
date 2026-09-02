//! `<tonk-notebook-index>` — the notebook index, where the heading is how
//! you get to a notebook.
//!
//! The page mounts an empty `<tonk-prose switcher>` showing `# `. Typing a
//! title suggests the notebooks that match; the list carries an explicit
//! "create this" row, so pressing Enter always has a visible target rather
//! than creating as the silent consequence of matching nothing.
//!
//! The suggestion ranking, the create row, and the keyboard handling all
//! live in the editor bundle (`heading-switcher.ts`, `fuzzy.ts`) because
//! they are ProseMirror concerns. This element is the app half: it supplies
//! the candidate notebooks, renders the panel, and performs the navigation.
//!
//! The rows it reads come from the directory view, which renders one link
//! per notebook. They are also the fallback: with no script at all the
//! index is still a working list of links.

use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use web_sys::{CustomEvent, Element, HtmlElement};

use custom_elements::CustomElement;

/// One notebook the switcher can offer.
struct Candidate {
    title: String,
    href: String,
}

/// The index page.
#[derive(Default)]
pub struct TonkNotebookIndexElement;

impl TonkNotebookIndexElement {
    /// The notebooks the directory rendered, as `(title, href)`.
    fn candidates(host: &HtmlElement) -> Vec<Candidate> {
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
                .and_then(|el| el.text_content())
                .unwrap_or_default()
                .trim()
                .to_owned();
            let href = row.get_attribute("href").unwrap_or_default();
            if !title.is_empty() && !href.is_empty() {
                out.push(Candidate { title, href });
            }
        }
        out
    }

    /// Hand the editor the current candidate list.
    ///
    /// Assigned as a PROPERTY, not an attribute: it is a list read on every
    /// keystroke, and round-tripping it through the DOM would be lossy and
    /// slow.
    fn publish(host: &HtmlElement) {
        let Ok(Some(prose)) = host.query_selector("tonk-prose") else {
            return;
        };
        let list = js_sys::Array::new();
        for candidate in Self::candidates(host) {
            let entry = js_sys::Object::new();
            let _ = js_sys::Reflect::set(&entry, &"title".into(), &candidate.title.into());
            let _ = js_sys::Reflect::set(&entry, &"href".into(), &candidate.href.into());
            list.push(&entry);
        }
        let _ = js_sys::Reflect::set(&prose, &"candidates".into(), &list);
    }

    /// Draw the suggestion list, with the matched characters marked.
    fn render_panel(host: &HtmlElement, rows: &JsValue, active: i32) {
        let Ok(Some(panel)) = host.query_selector(".notebook-switcher__panel") else {
            return;
        };
        let Some(document) = web_sys::window().and_then(|w| w.document()) else {
            return;
        };
        panel.set_inner_html("");
        let Ok(rows) = rows.clone().dyn_into::<js_sys::Array>() else {
            let _ = panel.set_attribute("hidden", "");
            return;
        };
        if rows.length() == 0 {
            let _ = panel.set_attribute("hidden", "");
            return;
        }
        let _ = panel.remove_attribute("hidden");

        for index in 0..rows.length() {
            let row = rows.get(index);
            let title = js_sys::Reflect::get(&row, &"title".into())
                .ok()
                .and_then(|v| v.as_string())
                .unwrap_or_default();
            let create = js_sys::Reflect::get(&row, &"create".into())
                .ok()
                .map(|v| v.is_truthy())
                .unwrap_or(false);
            let Ok(item) = document.create_element("div") else {
                continue;
            };
            let mut class = String::from("notebook-switcher__row");
            if index as i32 == active {
                class.push_str(" notebook-switcher__row--active");
            }
            if create {
                class.push_str(" notebook-switcher__row--create");
            }
            let _ = item.set_attribute("class", &class);
            let _ = item.set_attribute("data-index", &index.to_string());

            // The NAME, then a small verb saying what Enter does with it.
            //
            // Every row is the same shape, so creating is one more thing to
            // pick rather than a differently-worded afterthought. The name
            // is what you read; the verb only tells you which of the two
            // things is about to happen.
            if let Ok(name) = document.create_element("span") {
                let _ = name.set_attribute("class", "notebook-switcher__name");
                if create {
                    // Nothing to highlight: this row IS what was typed.
                    name.set_text_content(Some(&title));
                } else {
                    // Mark the characters the query matched, so the reason a
                    // row is here is visible.
                    let spans = js_sys::Reflect::get(&row, &"spans".into())
                        .ok()
                        .and_then(|v| v.dyn_into::<js_sys::Array>().ok());
                    let marked: Vec<usize> = spans
                        .map(|arr| {
                            (0..arr.length())
                                .filter_map(|i| arr.get(i).as_f64().map(|n| n as usize))
                                .collect()
                        })
                        .unwrap_or_default();
                    // Runs, not characters: a `<mark>` per contiguous
                    // stretch, so the text stays selectable as words.
                    let mut at = 0usize;
                    let chars: Vec<char> = title.chars().collect();
                    while at < chars.len() {
                        let hit = marked.contains(&at);
                        let mut end = at;
                        while end < chars.len() && marked.contains(&end) == hit {
                            end += 1;
                        }
                        let run: String = chars[at..end].iter().collect();
                        let tag = if hit { "mark" } else { "span" };
                        if let Ok(part) = document.create_element(tag) {
                            part.set_text_content(Some(&run));
                            let _ = name.append_child(&part);
                        }
                        at = end;
                    }
                }
                let _ = item.append_child(&name);
            }
            if let Ok(verb) = document.create_element("span") {
                let _ = verb.set_attribute("class", "notebook-switcher__verb");
                verb.set_text_content(Some(if create { "create" } else { "open" }));
                let _ = item.append_child(&verb);
            }
            let _ = panel.append_child(&item);
        }
    }

    /// Go to a notebook, as a route change rather than a page load.
    ///
    /// Through `tonk_host::navigate_to`, not `location.href`: it pushes
    /// history and fires `popstate` so `<tonk-site>` re-resolves, and in a
    /// guest it forwards to the parent — the guest's document is
    /// `about:srcdoc` at an opaque origin, where a real navigation would
    /// load the whole app inside the iframe.
    fn navigate(href: &str) {
        tonk_host::navigate_to(href);
    }
}

impl CustomElement for TonkNotebookIndexElement {
    fn shadow() -> bool {
        // Light DOM: the directory rows are rendered into this element's
        // children by `<tonk-display>`, and the app stylesheet styles them.
        false
    }

    fn inject_children(&mut self, this: &HtmlElement) {
        // Only the suggestion panel. The EDITOR is a real
        // `<tonk-notebook draft>` mounted by the view, not something this
        // element builds: the index is a notebook opened on nothing, so
        // nothing changes shape under the author when the draft is named.
        let Some(document) = web_sys::window().and_then(|w| w.document()) else {
            return;
        };
        if this
            .query_selector(".notebook-switcher__panel")
            .ok()
            .flatten()
            .is_some()
        {
            return;
        }
        if let Ok(panel) = document.create_element("div") {
            let _ = panel.set_attribute("class", "notebook-switcher__panel");
            let _ = panel.set_attribute("hidden", "");
            // After the draft editor, above the rows it matches against.
            let after = this
                .query_selector("tonk-notebook[draft]")
                .ok()
                .flatten()
                .and_then(|el| el.next_sibling());
            let _ = this.insert_before(&panel, after.as_ref());
        }
    }

    fn connected_callback(&mut self, this: &HtmlElement) {
        let host = this.clone();

        // Suggestions: draw the panel.
        let drawing = host.clone();
        let on_suggest = Closure::<dyn FnMut(CustomEvent)>::new(move |event: CustomEvent| {
            let detail = event.detail();
            let rows = js_sys::Reflect::get(&detail, &"rows".into()).unwrap_or(JsValue::NULL);
            let active = js_sys::Reflect::get(&detail, &"active".into())
                .ok()
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0) as i32;
            TonkNotebookIndexElement::render_panel(&drawing, &rows, active);
        });
        let _ =
            host.add_event_listener_with_callback("suggest", on_suggest.as_ref().unchecked_ref());
        on_suggest.forget();

        // Choosing an existing notebook.
        let on_switch = Closure::<dyn FnMut(CustomEvent)>::new(move |event: CustomEvent| {
            if let Some(href) = js_sys::Reflect::get(&event.detail(), &"href".into())
                .ok()
                .and_then(|v| v.as_string())
            {
                TonkNotebookIndexElement::navigate(&href);
            }
        });
        let _ = host.add_event_listener_with_callback("switch", on_switch.as_ref().unchecked_ref());
        on_switch.forget();

        // Creating one. The worker's `CreateNotebook` handler writes the
        // notebook AND performs the redirect: it is the only place that
        // knows the entity the write derives, and the page cannot learn it
        // from a transient that is swept before any subscription sees it.
        let creating = host.clone();
        let on_create = Closure::<dyn FnMut(CustomEvent)>::new(move |event: CustomEvent| {
            let Some(title) = js_sys::Reflect::get(&event.detail(), &"title".into())
                .ok()
                .and_then(|v| v.as_string())
            else {
                return;
            };
            // The draft's whole document rides along: its body is content
            // the author typed, and a create that carried only the title
            // would throw away everything written under the heading.
            let document = js_sys::Reflect::get(&event.detail(), &"document".into())
                .ok()
                .and_then(|v| v.as_string())
                .unwrap_or_default();
            let detail = js_sys::Object::new();
            let _ = js_sys::Reflect::set(&detail, &"createdTitle".into(), &title.into());
            let _ = js_sys::Reflect::set(&detail, &"createdBody".into(), &document.into());
            let init = web_sys::CustomEventInit::new();
            init.set_detail(&detail);
            init.set_bubbles(true);
            init.set_composed(true);
            if let Ok(event) = CustomEvent::new_with_event_init_dict("notebookcreate", &init) {
                let _ = creating.dispatch_event(&event);
            }
        });
        let _ = host.add_event_listener_with_callback("create", on_create.as_ref().unchecked_ref());
        on_create.forget();

        // The rows arrive from a `<tonk-display>` render that may land after
        // this callback, so republish whenever the child list changes.
        let observing = host.clone();
        let on_mutate = Closure::<dyn FnMut(js_sys::Array)>::new(move |_: js_sys::Array| {
            TonkNotebookIndexElement::publish(&observing);
        });
        if let Ok(observer) = web_sys::MutationObserver::new(on_mutate.as_ref().unchecked_ref()) {
            let options = web_sys::MutationObserverInit::new();
            options.set_child_list(true);
            options.set_subtree(true);
            let _ = observer.observe_with_options(host.as_ref() as &Element, &options);
        }
        on_mutate.forget();

        Self::publish(&host);
    }
}
