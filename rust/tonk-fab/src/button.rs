//! `<tonk-button>` — a block button.
//!
//! A boxy block, no radii: every terminal edge is a line, so the word sits
//! bottom-right like every other cell (law 3). Runs of buttons fuse flush and
//! the fill boundary is the divider — which is why there is no gap and no
//! border between neighbours.
//!
//! `variant="primary"` is solid ink; the fill IS the call to action, so it
//! needs no glyph and takes no color (law 5). `variant="quiet"` drops the
//! ring and the glass. `solid` swaps the floating-chrome glass for the denser
//! panel fill, for buttons sitting on a modal surface where the blur reads as
//! nothing.

use custom_elements::CustomElement;
use wasm_bindgen::JsValue;
use web_sys::{HtmlElement, window};

use crate::shadow::{self, Bound};

/// Component-local rules layered over [`crate::skin::SKIN`].
const CSS: &str = r#"
:host{ display:inline-block; }
.w{ display:contents; }
.b{ height:36px; min-width:144px; border-radius:0; padding:0 10px 9px 24px;
  display:inline-flex; align-items:flex-end; justify-content:flex-end; gap:6px;
  font-size:13px; font-weight:600; line-height:1; text-transform:lowercase; color:var(--_ink);
  background:var(--_bg); -webkit-backdrop-filter:var(--_filter); backdrop-filter:var(--_filter);
  box-shadow:var(--_ring); }
.b:hover{ background:linear-gradient(var(--_hover),var(--_hover)), var(--_bg); }
.b:active{ background:linear-gradient(var(--_press),var(--_press)), var(--_bg); }
:host([solid]) .b{ background:var(--_panel); }
:host([solid]) .b:hover{ background:linear-gradient(var(--_hover),var(--_hover)), var(--_panel); }
:host([solid]) .b:active{ background:linear-gradient(var(--_press),var(--_press)), var(--_panel); }
:host([variant=primary]) .b{ background:var(--_ink); color:var(--_on); -webkit-backdrop-filter:none; backdrop-filter:none; }
:host([variant=primary]) .b:hover{ filter:brightness(.92); }
:host([variant=primary]) .b:active{ filter:brightness(.86); }
:host([variant=quiet]) .b{ background:transparent; -webkit-backdrop-filter:none; backdrop-filter:none; box-shadow:none; }
:host([variant=quiet]) .b:hover{ background:var(--_hover); }
:host([variant=quiet]) .b:active{ background:var(--_press); }
:host([disabled]) .b{ opacity:.4; cursor:not-allowed; }
"#;

const HTML: &str = r#"<div class="w"><button class="b" part="button"><slot></slot></button></div>"#;

/// Per-element state — the listeners, kept alive for the element's lifetime.
#[derive(Default)]
pub(crate) struct TonkButton {
    listeners: Vec<Bound>,
}

impl CustomElement for TonkButton {
    fn shadow() -> bool {
        // Attached in `connected_callback` so the component controls build
        // timing — see `shadow::ensure_shadow`.
        false
    }

    fn observed_attributes() -> &'static [&'static str] {
        &["variant", "disabled", "solid"]
    }

    fn inject_children(&mut self, _this: &HtmlElement) {}

    fn connected_callback(&mut self, this: &HtmlElement) {
        let root = shadow::build(this, CSS, HTML);

        if let Ok(Some(button)) = root.query_selector(".b") {
            let host = this.clone();
            self.listeners.push(shadow::on_click(&button, move || {
                // A disabled button emits nothing. The native `disabled`
                // attribute is also mirrored below, but the guard has to be
                // here too: `disabled` on the HOST is what callers set, and
                // that alone does not disable the shadow button.
                if host.has_attribute("disabled") {
                    return;
                }
                shadow::emit(&host, "fabb-press", &JsValue::NULL);
            }));
        }

        self.listeners.push(shadow::install_visibility_pause(this));
        sync_disabled(this);
    }

    fn disconnected_callback(&mut self, _this: &HtmlElement) {
        self.listeners.clear();
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
        if name == "disabled" {
            sync_disabled(this);
        }
    }
}

/// Mirror the host's `disabled` onto the shadow button, so the control is
/// genuinely inert to the keyboard and not merely dimmed.
fn sync_disabled(this: &HtmlElement) {
    let disabled = this.has_attribute("disabled");
    if let Some(root) = this.shadow_root()
        && let Ok(Some(button)) = root.query_selector(".b")
    {
        if disabled {
            let _ = button.set_attribute("disabled", "");
        } else {
            let _ = button.remove_attribute("disabled");
        }
    }
}

/// Register `<tonk-button>`. Idempotent.
pub(crate) fn register() {
    let Some(win) = window() else { return };
    if win.custom_elements().get("tonk-button").is_undefined() {
        TonkButton::define("tonk-button");
    }
}
