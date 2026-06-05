//! `<tonk-inspector source="…">` custom element.
//!
//! Thin web-component wrapper around the [`TonkInspector`] Leptos
//! component (defined in `super::space`). Lets the inspector be
//! dropped into any DOM context — board tiles, demos, embeds —
//! by writing `<tonk-inspector source="home">` instead of
//! building a Leptos sub-tree.
//!
//! Mount strategy: the element creates a fresh Leptos root via
//! `leptos::mount::mount_to` on its own host node. The inspector
//! consults no page-level signal, so it renders standalone in any
//! host (board tile, demo, embed) without provided context. (The
//! share affordance now lives in the workspace top bar as
//! `<tonk-share>`, which bridges to the shell via a `tonk:share`
//! window event — see `super::invite` / `TonkShell`.)

use std::any::Any;
use std::cell::RefCell;

use crate::components::TonkInspector;
use custom_elements::CustomElement;
use leptos::prelude::*;
use web_sys::{HtmlElement, window};

/// Per-instance state for one `<tonk-inspector>` element.
#[derive(Default)]
pub(crate) struct TonkInspectorElement {
    /// Reactive source the inner [`TonkInspector`] reads. Updated
    /// from `attribute_changed_callback` so the inspector
    /// re-resolves when the host's `source` attribute flips.
    source: RwSignal<Option<String>, LocalStorage>,
    /// Boxed [`leptos::mount::UnmountHandle`]. Type-erased because
    /// the concrete `N::State` is the giant view's mountable type
    /// and we don't want to name it. Dropping the box unmounts
    /// the sub-tree.
    mount: RefCell<Option<Box<dyn Any>>>,
}

impl CustomElement for TonkInspectorElement {
    fn shadow() -> bool {
        false
    }

    fn observed_attributes() -> &'static [&'static str] {
        &["source"]
    }

    fn inject_children(&mut self, _this: &HtmlElement) {}

    fn connected_callback(&mut self, this: &HtmlElement) {
        // Seed the reactive source from the current attribute
        // value before mounting so the first render already sees
        // the right space name.
        let initial = this.get_attribute("source").filter(|s| !s.is_empty());
        self.source.set(initial);

        let source = self.source;
        let handle = leptos::mount::mount_to(this.clone(), move || {
            view! {
                <TonkInspector source=source />
            }
        });
        *self.mount.borrow_mut() = Some(Box::new(handle));
    }

    fn disconnected_callback(&mut self, _this: &HtmlElement) {
        // Drop the mount handle so Leptos can tear down the
        // sub-tree's reactive graph cleanly.
        self.mount.borrow_mut().take();
    }

    fn attribute_changed_callback(
        &mut self,
        _this: &HtmlElement,
        _name: String,
        old: Option<String>,
        new: Option<String>,
    ) {
        // No-op when the attribute was rewritten to the same
        // value (Leptos reactive bindings can replay
        // setAttribute with an unchanged value).
        if old == new {
            return;
        }
        self.source.set(new.filter(|s| !s.is_empty()));
    }
}

/// Register `<tonk-inspector>`. Idempotent — calling more than
/// once is harmless.
pub fn register() {
    if already_registered() {
        return;
    }
    TonkInspectorElement::define("tonk-inspector");
}

fn already_registered() -> bool {
    let Some(win) = window() else { return false };
    !win.custom_elements().get("tonk-inspector").is_undefined()
}
