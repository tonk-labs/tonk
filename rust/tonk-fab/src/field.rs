//! `<tonk-field>` — an entry or settled record row.
//!
//! The row keeps the edge grammar's two readings on one 36px block: the
//! noun sits bottom-left in soft ink and the value sits bottom-right in ink.
//! Entry rows carry the shared terminal cursor; settled records do not.

#[cfg_attr(not(any(test, target_arch = "wasm32")), allow(dead_code))]
const CSS: &str = r#"
:host{ display:block; min-width:0; }
.w{ display:block; }
.row{ height:36px; min-width:0; padding:0 10px 8px 12px; display:flex;
  align-items:flex-end; gap:12px; background:var(--_bg);
  -webkit-backdrop-filter:var(--_filter); backdrop-filter:var(--_filter);
  box-shadow:var(--_ring); overflow:hidden; }
.noun{ position:relative; flex:none; min-width:0; color:var(--_soft);
  font-size:12px; font-weight:600; line-height:1; text-transform:lowercase;
  white-space:nowrap; }
.noun-change{ display:none; color:var(--_ink); text-decoration:underline;
  text-underline-offset:2px; }
.noun .cur{ vertical-align:-2px; margin-left:3px; mix-blend-mode:normal; }
.value{ flex:1 1 auto; min-width:0; padding:0; border:0; outline:0;
  color:var(--_ink); background:transparent; font:inherit; font-size:13px;
  font-weight:600; line-height:1; text-align:right; caret-color:transparent; }
.value-cur{ margin-bottom:0; }
:host([filter=digits]) .value{ font-variant-numeric:tabular-nums; }
:host([autolen="6"]) .value{ letter-spacing:.14em; }
:host([settled]) .value{ pointer-events:none; }
:host([settled]) .value-cur{ display:none; }
:host([changeable]) .noun{ cursor:pointer; min-height:20px; display:inline-flex;
  align-items:flex-end; }
:host([changeable]) .noun:hover .noun-current,
:host([changeable]) .noun:focus-visible .noun-current{ display:none; }
:host([changeable]) .noun:hover .noun-change,
:host([changeable]) .noun:focus-visible .noun-change{ display:inline-flex; align-items:flex-end; }
.row.rejecting{ animation:fabb-wash .45s var(--_ease) 2; }
/* the skin gives `input.value` the native block caret where the engine
   draws one; the tail block would double the cursor, so it stands down */
@supports (caret-shape: block){
  .value-cur{ display:none; }
}
@media (pointer:coarse){
  :host([changeable]) .noun-current{ display:none; }
  :host([changeable]) .noun-change{ display:inline-flex; align-items:flex-end; }
}
@media (max-width:519px), (pointer:coarse){
  .row{ height:44px; }
  .value{ font-size:16px; }
}
@media (prefers-reduced-motion:reduce){
  .row.rejecting{ animation:none; }
  .cur{ animation:none; }
}
"#;

#[cfg_attr(not(any(test, target_arch = "wasm32")), allow(dead_code))]
const HTML: &str = r#"<div class="w"><label class="row">
  <span class="noun"><span class="noun-current"></span><span class="noun-change">change<i class="cur" aria-hidden="true"></i></span></span>
  <input class="value" type="text" autocomplete="off" spellcheck="false">
  <i class="cur value-cur" aria-hidden="true"></i>
</label></div>"#;

#[cfg(target_arch = "wasm32")]
mod element {
    use std::cell::RefCell;
    use std::rc::Rc;

    use custom_elements::CustomElement;
    use js_sys::{Object, Reflect};
    use wasm_bindgen::JsCast;
    use wasm_bindgen::prelude::*;
    use web_sys::{HtmlElement, HtmlInputElement, KeyboardEvent, window};

    use crate::shadow::{self, Bound};

    use super::{CSS, HTML};

    #[derive(Default)]
    pub(super) struct TonkField {
        listeners: Vec<Bound>,
        last_auto_commit: Rc<RefCell<Option<String>>>,
    }

    impl CustomElement for TonkField {
        fn shadow() -> bool {
            false
        }

        fn observed_attributes() -> &'static [&'static str] {
            &[
                "mode",
                "noun",
                "value",
                "settled",
                "filter",
                "autolen",
                "changeable",
            ]
        }

        fn inject_children(&mut self, _this: &HtmlElement) {}

        fn connected_callback(&mut self, this: &HtmlElement) {
            let root = shadow::build(this, CSS, HTML);
            install_reject_api(this);

            if let Ok(Some(value)) = root.query_selector(".value") {
                let host = this.clone();
                let last = self.last_auto_commit.clone();
                self.listeners.push(shadow::bind(&value, "input", move |_| {
                    normalize_and_maybe_commit(&host, &last);
                }));

                let host = this.clone();
                self.listeners
                    .push(shadow::bind(&value, "keydown", move |event| {
                        let Some(event) = event.dyn_ref::<KeyboardEvent>() else {
                            return;
                        };
                        if event.key() == "Enter" {
                            event.prevent_default();
                            emit_commit(&host);
                        }
                    }));
            }

            if let Ok(Some(noun)) = root.query_selector(".noun") {
                let host = this.clone();
                self.listeners
                    .push(shadow::bind(&noun, "click", move |event| {
                        if host.has_attribute("changeable") {
                            event.prevent_default();
                            shadow::emit(&host, "fabb-change-noun", &JsValue::NULL);
                        }
                    }));

                let host = this.clone();
                self.listeners
                    .push(shadow::bind(&noun, "keydown", move |event| {
                        let Some(event) = event.dyn_ref::<KeyboardEvent>() else {
                            return;
                        };
                        if host.has_attribute("changeable")
                            && matches!(event.key().as_str(), "Enter" | " ")
                        {
                            event.prevent_default();
                            shadow::emit(&host, "fabb-change-noun", &JsValue::NULL);
                        }
                    }));
            }

            if let Ok(Some(row)) = root.query_selector(".row") {
                let row_for_end = row.clone();
                self.listeners
                    .push(shadow::bind(&row, "animationend", move |_| {
                        let _ = row_for_end.class_list().remove_1("rejecting");
                    }));
            }

            self.listeners.push(shadow::install_visibility_pause(this));
            if let Some(listener) = shadow::install_system_mode(this) {
                self.listeners.push(listener);
            }
            sync(this);
        }

        fn disconnected_callback(&mut self, _this: &HtmlElement) {
            self.listeners.clear();
            *self.last_auto_commit.borrow_mut() = None;
        }

        fn attribute_changed_callback(
            &mut self,
            this: &HtmlElement,
            _name: String,
            old: Option<String>,
            new: Option<String>,
        ) {
            if old != new {
                sync(this);
            }
        }
    }

    fn input(this: &HtmlElement) -> Option<HtmlInputElement> {
        this.shadow_root()?
            .query_selector(".value")
            .ok()
            .flatten()?
            .dyn_into::<HtmlInputElement>()
            .ok()
    }

    fn sync(this: &HtmlElement) {
        shadow::apply_mode(this);
        let Some(root) = this.shadow_root() else {
            return;
        };
        if let Ok(Some(noun)) = root.query_selector(".noun-current") {
            noun.set_text_content(Some(&this.get_attribute("noun").unwrap_or_default()));
        }
        if let Some(input) = input(this) {
            let next = this.get_attribute("value").unwrap_or_default();
            if input.value() != next {
                input.set_value(&next);
            }
            input.set_read_only(this.has_attribute("settled"));
            input.set_tab_index(if this.has_attribute("settled") { -1 } else { 0 });
            if this.get_attribute("filter").as_deref() == Some("digits") {
                let _ = input.set_attribute("inputmode", "numeric");
            } else {
                let _ = input.remove_attribute("inputmode");
            }
            if let Some(length) = this.get_attribute("autolen") {
                let _ = input.set_attribute("maxlength", &length);
            } else {
                let _ = input.remove_attribute("maxlength");
            }
        }
        if let Ok(Some(noun)) = root.query_selector(".noun") {
            if this.has_attribute("changeable") {
                let _ = noun.set_attribute("role", "button");
                let _ = noun.set_attribute("tabindex", "0");
                let _ = noun.set_attribute("aria-label", "change");
            } else {
                let _ = noun.remove_attribute("role");
                let _ = noun.remove_attribute("tabindex");
                let _ = noun.remove_attribute("aria-label");
            }
        }
    }

    fn normalize_and_maybe_commit(
        this: &HtmlElement,
        last_auto_commit: &Rc<RefCell<Option<String>>>,
    ) {
        let Some(input) = input(this) else {
            return;
        };
        let mut value = input.value();
        if this.get_attribute("filter").as_deref() == Some("digits") {
            value.retain(|character| character.is_ascii_digit());
        }
        let limit = this
            .get_attribute("autolen")
            .and_then(|length| length.parse::<usize>().ok())
            .filter(|length| *length > 0);
        if let Some(limit) = limit
            && value.chars().count() > limit
        {
            value = value.chars().take(limit).collect();
        }
        if input.value() != value {
            input.set_value(&value);
        }
        let _ = this.set_attribute("value", &value);

        if limit.is_some_and(|limit| value.chars().count() == limit) {
            if last_auto_commit.borrow().as_deref() != Some(value.as_str()) {
                *last_auto_commit.borrow_mut() = Some(value);
                emit_commit(this);
            }
        } else {
            *last_auto_commit.borrow_mut() = None;
        }
    }

    fn emit_commit(this: &HtmlElement) {
        let value = input(this).map(|input| input.value()).unwrap_or_default();
        let detail = Object::new();
        let _ = Reflect::set(&detail, &"value".into(), &value.into());
        shadow::emit(this, "fabb-commit", &detail.into());
    }

    fn install_reject_api(this: &HtmlElement) {
        let host = this.clone();
        let reject = Closure::<dyn FnMut()>::new(move || reject(&host));
        let _ = Reflect::set(this, &"reject".into(), reject.as_ref());
        reject.forget();
    }

    fn reject(this: &HtmlElement) {
        if let Some(root) = this.shadow_root()
            && let Ok(Some(row)) = root.query_selector(".row")
        {
            let _ = row.class_list().remove_1("rejecting");
            let _ = row.get_bounding_client_rect();
            let _ = row.class_list().add_1("rejecting");
        }
        if let Some(input) = input(this) {
            let _ = input.focus();
            input.select();
        }
    }

    pub(crate) fn register() {
        let Some(win) = window() else { return };
        if win.custom_elements().get("tonk-field").is_undefined() {
            TonkField::define("tonk-field");
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub(crate) use element::register;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markup_has_the_edge_row_anatomy() {
        assert!(HTML.contains("class=\"noun\""));
        assert!(HTML.contains("class=\"value\""));
        assert!(HTML.contains("class=\"cur\""));
        assert!(CSS.contains("height:36px"));
        assert!(CSS.contains(":host([settled])"));
        assert!(CSS.contains(":host([filter=digits])"));
        assert!(CSS.contains("@media (max-width:519px), (pointer:coarse)"));
        assert!(CSS.contains(".row{ height:44px; }"));
        assert!(CSS.contains(".value{ font-size:16px; }"));
    }

    #[test]
    fn component_css_uses_only_skin_tokens_for_colour() {
        assert!(!CSS.contains('#'));
        assert!(!CSS.contains("rgb("));
        assert!(!CSS.contains("rgba("));
        assert!(CSS.contains("var(--_ink)"));
        assert!(CSS.contains("var(--_soft)"));
    }
}
