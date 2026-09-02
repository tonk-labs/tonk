//! `<tonk-cluster>` — the ceremony shell for a wall or condition.

#[cfg_attr(not(any(test, target_arch = "wasm32")), allow(dead_code))]
const CSS: &str = r#"
:host{ position:fixed; inset:0; z-index:2147483647; display:block; }
:host([hidden]){ display:none; }
.w{ position:absolute; inset:0; }
.dim{ position:absolute; inset:0; display:grid; place-items:center; padding:24px;
  background:color-mix(in srgb, var(--_ink) 32%, transparent); }
.cluster{ width:min(432px, calc(100vw - 48px)); display:flex; flex-direction:column;
  gap:7px; color:var(--_ink); }
.statement,.narrator{ min-height:36px; padding:12px 14px; display:flex;
  align-items:flex-end; background:var(--_panel); box-shadow:var(--_ring); }
.statement{ font-family:'IBM Plex Sans',system-ui,sans-serif; font-size:13.5px;
  font-weight:600; line-height:1.55; text-wrap:balance; }
.narrator{ color:var(--_soft); font-family:'IBM Plex Sans',system-ui,sans-serif;
  font-size:13px; font-weight:400; line-height:1.55; text-wrap:pretty; }
.fields{ display:flex; flex-direction:column; gap:7px; }
.run{ display:flex; gap:0; align-items:stretch; }
.run ::slotted(*){ flex:1 1 0; min-width:0; }
.ghost{ align-self:flex-start; min-height:40px; padding:10px 0; color:var(--_on);
  font-size:13px; line-height:20px; text-decoration:underline;
  text-underline-offset:2px; cursor:pointer; }
@media (max-width:519px){
  .dim{ padding:16px; }
  .cluster{ width:min(432px, calc(100vw - 32px)); }
  .run{ display:grid; grid-template-columns:1fr; gap:0; }
}
"#;

#[cfg_attr(not(any(test, target_arch = "wasm32")), allow(dead_code))]
const HTML: &str = r#"<div class="w"><div class="dim" part="dim">
  <section class="cluster" role="dialog" aria-modal="true">
    <div class="statement"><slot name="statement"></slot></div>
    <div class="fields"><slot></slot></div>
    <div class="narrator"><slot name="narrator"></slot></div>
    <div class="run"><slot name="run"></slot></div>
    <span class="ghost" role="button" tabindex="0"><span aria-hidden="true">&#9666; </span><slot name="ghost"></slot></span>
  </section>
</div></div>"#;

#[cfg(target_arch = "wasm32")]
mod element {
    use custom_elements::CustomElement;
    use wasm_bindgen::JsCast;
    use wasm_bindgen::prelude::*;
    use web_sys::{Element, HtmlElement, KeyboardEvent, window};

    use crate::shadow::{self, Bound};

    use super::{CSS, HTML};

    #[derive(Default)]
    pub(super) struct TonkCluster {
        listeners: Vec<Bound>,
    }

    impl CustomElement for TonkCluster {
        fn shadow() -> bool {
            false
        }

        fn observed_attributes() -> &'static [&'static str] {
            &[]
        }

        fn inject_children(&mut self, _this: &HtmlElement) {}

        fn connected_callback(&mut self, this: &HtmlElement) {
            let root = shadow::build(this, CSS, HTML);

            if let Ok(Some(dim)) = root.query_selector(".dim") {
                let dim_target = dim.clone();
                self.listeners
                    .push(shadow::bind(&dim, "click", move |event| {
                        if event
                            .target()
                            .and_then(|target| target.dyn_into::<Element>().ok())
                            .is_some_and(|target| target == dim_target)
                        {
                            event.prevent_default();
                            event.stop_propagation();
                        }
                    }));
            }

            if let Ok(Some(ghost)) = root.query_selector(".ghost") {
                let host = this.clone();
                self.listeners.push(shadow::on_click(&ghost, move || {
                    shadow::emit(&host, "fabb-bail", &JsValue::NULL)
                }));

                let host = this.clone();
                self.listeners
                    .push(shadow::bind(&ghost, "keydown", move |event| {
                        let Some(event) = event.dyn_ref::<KeyboardEvent>() else {
                            return;
                        };
                        if matches!(event.key().as_str(), "Enter" | " ") {
                            event.prevent_default();
                            shadow::emit(&host, "fabb-bail", &JsValue::NULL);
                        }
                    }));
            }

            let host = this.clone();
            self.listeners
                .push(shadow::bind(this, "keydown", move |event| {
                    let Some(key) = event.dyn_ref::<KeyboardEvent>() else {
                        return;
                    };
                    match key.key().as_str() {
                        "Escape" => {
                            key.prevent_default();
                            shadow::emit(&host, "fabb-bail", &JsValue::NULL);
                        }
                        "Tab" => loop_focus(&host, key),
                        _ => {}
                    }
                }));

            self.listeners.push(shadow::install_visibility_pause(this));
        }

        fn disconnected_callback(&mut self, _this: &HtmlElement) {
            self.listeners.clear();
        }

        fn attribute_changed_callback(
            &mut self,
            _this: &HtmlElement,
            _name: String,
            _old: Option<String>,
            _new: Option<String>,
        ) {
        }
    }

    fn light_focusables(this: &HtmlElement) -> Vec<Element> {
        let Ok(nodes) = this.query_selector_all(
            "button:not([disabled]),a[href],input:not([disabled]),select:not([disabled]),textarea:not([disabled]),[tabindex]:not([tabindex='-1']),tonk-button:not([disabled]),tonk-field:not([settled])",
        ) else {
            return Vec::new();
        };
        (0..nodes.length())
            .filter_map(|index| nodes.item(index))
            .filter_map(|node| node.dyn_into::<Element>().ok())
            .collect()
    }

    fn event_hit(event: &KeyboardEvent, wanted: &Element) -> bool {
        let path = event.composed_path();
        (0..path.length()).any(|index| {
            path.get(index)
                .dyn_into::<Element>()
                .ok()
                .is_some_and(|element| element == *wanted)
        })
    }

    fn focus(element: &Element) {
        let target = element
            .shadow_root()
            .and_then(|root| {
                root.query_selector("button,input,[tabindex]:not([tabindex='-1'])")
                    .ok()
                    .flatten()
            })
            .unwrap_or_else(|| element.clone());
        if let Ok(target) = target.dyn_into::<HtmlElement>() {
            let _ = target.focus();
        }
    }

    fn loop_focus(this: &HtmlElement, event: &KeyboardEvent) {
        let focusables = light_focusables(this);
        let Some(first) = focusables.first() else {
            return;
        };
        let Some(ghost) = this
            .shadow_root()
            .and_then(|root| root.query_selector(".ghost").ok().flatten())
        else {
            return;
        };
        if !event.shift_key() && event_hit(event, &ghost) {
            event.prevent_default();
            focus(first);
        } else if event.shift_key() && event_hit(event, first) {
            event.prevent_default();
            focus(&ghost);
        }
    }

    pub(crate) fn register() {
        let Some(win) = window() else { return };
        if win.custom_elements().get("tonk-cluster").is_undefined() {
            TonkCluster::define("tonk-cluster");
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub(crate) use element::register;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markup_has_one_dim_and_all_ceremony_slots() {
        assert_eq!(HTML.matches("class=\"dim\"").count(), 1);
        for slot in ["statement", "narrator", "run", "ghost"] {
            assert!(HTML.contains(&format!("name=\"{slot}\"")));
        }
        assert!(!HTML.contains("<button class=\"ghost\""));
    }

    #[test]
    fn run_is_fused_and_the_column_uses_the_shared_rhythm() {
        assert!(CSS.contains("width:min(432px"));
        assert!(CSS.contains("gap:7px"));
        assert!(CSS.contains(".run{"));
        assert!(CSS.contains("gap:0"));
    }
}
