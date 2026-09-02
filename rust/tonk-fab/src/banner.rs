//! `<tonk-banner>` — a transient condition banner.

#[cfg_attr(not(any(test, target_arch = "wasm32")), allow(dead_code))]
const CSS: &str = r#"
:host{ position:fixed; left:50%; bottom:40px; width:min(680px, calc(100vw - 48px));
  z-index:2147483645; transform:translateX(-50%); display:block; }
:host([hidden]){ display:none; }
/* The banner follows the PAGE theme, unlike the bar: it floats over the
   page itself, not over a space, so the one-bright-twin law does not
   apply (the edges study themes its banner the same way). Restated
   locally per the skin's own rule; a page without theme tokens falls
   back to the pinned bright `--fabb-*` API. */
.w{
  --_ink:  var(--ink, var(--fabb-ink));
  --_soft: var(--soft, var(--fabb-ink-soft));
  --_on:   var(--on-ink, var(--fabb-on-ink));
  --_bg:   var(--frost, var(--fabb-bg));
  --_ring: 0 0 0 1px var(--ring, var(--fabb-ring));
}
.w{ min-height:44px; display:grid; grid-template-columns:minmax(0,1fr) auto;
  opacity:0; transform:translateY(70px); transition-property:transform,opacity;
  transition-duration:300ms; transition-timing-function:var(--_ease);
  -webkit-backdrop-filter:var(--_filter); backdrop-filter:var(--_filter); }
.w.live{ opacity:1; transform:translateY(0); }
.w.retiring{ opacity:0; transform:translateY(-12px); transition-duration:150ms; }
.message{ min-width:0; padding:11px 16px; display:flex; align-items:center;
  color:var(--_soft); background:var(--_bg); box-shadow:var(--_ring);
  font-size:13px; line-height:1.45; text-wrap:pretty; }
.door{ min-width:144px; min-height:44px; padding:10px 14px; display:flex;
  align-items:flex-end; justify-content:flex-end; color:var(--_on);
  background:var(--_ink); font-size:13px; line-height:1; text-transform:lowercase;
  transition-property:scale,filter; transition-duration:150ms; }
.door:hover{ filter:brightness(.92); }
.door:active{ filter:brightness(.86); scale:.96; }
@media (max-width:519px){
  :host{ bottom:max(76px, env(safe-area-inset-bottom) + 68px);
    width:min(680px, calc(100vw - 32px)); }
  .w{ grid-template-columns:minmax(0,1fr); }
  .door{ min-width:0; justify-content:flex-end; }
}
@media (prefers-reduced-motion:reduce){ .w{ transition-duration:0ms; } }
"#;

#[cfg_attr(not(any(test, target_arch = "wasm32")), allow(dead_code))]
const HTML: &str = r#"<div class="w" role="status">
  <div class="message"><slot></slot></div>
  <button class="door"><slot name="door"></slot></button>
</div>"#;

#[cfg(target_arch = "wasm32")]
mod element {
    use custom_elements::CustomElement;
    use js_sys::Reflect;
    use wasm_bindgen::JsCast;
    use wasm_bindgen::prelude::*;
    use web_sys::{HtmlElement, window};

    use crate::shadow::{self, Bound};

    use super::{CSS, HTML};

    const BEAT_MS: i32 = 450;
    const RETIRE_MS: i32 = 150;

    #[derive(Default)]
    pub(super) struct TonkBanner {
        listeners: Vec<Bound>,
    }

    impl CustomElement for TonkBanner {
        fn shadow() -> bool {
            false
        }

        fn observed_attributes() -> &'static [&'static str] {
            &[]
        }

        fn inject_children(&mut self, _this: &HtmlElement) {}

        fn connected_callback(&mut self, this: &HtmlElement) {
            let root = shadow::build(this, CSS, HTML);
            install_retire_api(this);
            if let Ok(Some(door)) = root.query_selector(".door") {
                let host = this.clone();
                self.listeners.push(shadow::on_click(&door, move || {
                    shadow::emit(&host, "fabb-open", &JsValue::NULL)
                }));
            }
            self.listeners.push(shadow::install_visibility_pause(this));

            let host = this.clone();
            let reveal = Closure::once_into_js(move || {
                if let Some(root) = host.shadow_root()
                    && let Ok(Some(wrapper)) = root.query_selector(".w")
                {
                    let _ = wrapper.class_list().add_1("live");
                }
            });
            if let Some(win) = window() {
                let _ = win.set_timeout_with_callback_and_timeout_and_arguments_0(
                    reveal.unchecked_ref(),
                    BEAT_MS,
                );
            }
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

    fn install_retire_api(this: &HtmlElement) {
        let host = this.clone();
        let retire = Closure::<dyn FnMut()>::new(move || retire(&host));
        let _ = Reflect::set(this, &"retire".into(), retire.as_ref());
        retire.forget();
    }

    fn retire(this: &HtmlElement) {
        if let Some(root) = this.shadow_root()
            && let Ok(Some(wrapper)) = root.query_selector(".w")
        {
            let _ = wrapper.class_list().add_1("retiring");
        }
        let host = this.clone();
        let remove = Closure::once_into_js(move || host.remove());
        if let Some(win) = window() {
            let _ = win.set_timeout_with_callback_and_timeout_and_arguments_0(
                remove.unchecked_ref(),
                RETIRE_MS,
            );
        }
    }

    pub(crate) fn register() {
        let Some(win) = window() else { return };
        if win.custom_elements().get("tonk-banner").is_undefined() {
            TonkBanner::define("tonk-banner");
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub(crate) use element::register;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markup_has_one_soft_message_and_one_solid_door() {
        assert_eq!(HTML.matches("class=\"message\"").count(), 1);
        assert_eq!(HTML.matches("class=\"door\"").count(), 1);
        assert!(CSS.contains("color:var(--_soft)"));
        assert!(CSS.contains("background:var(--_ink)"));
        assert!(!CSS.contains('#'));
        assert!(!CSS.contains("rgb("));
    }

    #[test]
    fn banner_pins_the_desktop_and_phone_seats() {
        assert!(CSS.contains("bottom:40px"));
        assert!(CSS.contains("width:min(680px, calc(100vw - 48px))"));
        assert!(CSS.contains("bottom:max(76px, env(safe-area-inset-bottom) + 68px)"));
    }
}
