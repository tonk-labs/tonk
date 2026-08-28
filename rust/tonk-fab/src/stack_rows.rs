//! Rendering subscription results as rows of a stack.
//!
//! A stack ([`crate::menu`]) wears its glass ONCE, as an underlay masked to
//! its rows, and lays them out as a flex column with 7px gaps. Both of those
//! read the stack's DIRECT `tonk-mi` children — so an element that renders
//! rows inside ITSELF produces rows the mask never cuts a band for (no
//! glass behind them) and, being a child of its own, an extra 7px gap where
//! it sits.
//!
//! So a row-producing element renders its rows as SIBLINGS, inserted just
//! before itself, and is itself laid out away. It tags what it inserts so a
//! rebuild removes only its own rows and leaves the stack's authored ones —
//! `more ↖`, `copy link` — alone.

use wasm_bindgen::JsCast;
use web_sys::{Element, HtmlElement, window};

/// Marks a row as belonging to the element named by the attribute's value,
/// so a rebuild can find exactly its own previous output.
const OWNER_ATTR: &str = "data-row-owner";

/// The stack this element renders into.
pub(crate) fn host_menu(this: &HtmlElement) -> Option<Element> {
    this.closest("tonk-menu").ok().flatten()
}

/// Remove the rows this element inserted last time.
pub(crate) fn clear_rows(this: &HtmlElement, owner: &str) {
    let Some(menu) = host_menu(this) else { return };
    let Ok(previous) = menu.query_selector_all(&format!("[{OWNER_ATTR}=\"{owner}\"]")) else {
        return;
    };
    for index in 0..previous.length() {
        if let Some(node) = previous.item(index)
            && let Ok(element) = node.dyn_into::<Element>()
        {
            element.remove();
        }
    }
}

/// Create a `tonk-mi` row owned by `owner`, ready to be filled and inserted.
pub(crate) fn new_row(owner: &str) -> Option<Element> {
    let document = window()?.document()?;
    let row = document.create_element("tonk-mi").ok()?;
    let _ = row.set_attribute(OWNER_ATTR, owner);
    Some(row)
}

/// Insert `row` into the stack immediately before this element, so the rows
/// appear where the element sits among the authored ones.
pub(crate) fn insert_row(this: &HtmlElement, row: &Element) {
    let Some(menu) = host_menu(this) else { return };
    let _ = menu.insert_before(row, Some(this));
}
