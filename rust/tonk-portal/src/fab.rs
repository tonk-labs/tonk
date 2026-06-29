//! The `<tonk-fab-portal>` custom element.
//!
//! A [`TonkPortal`] variant that renders a sealed iframe as a small
//! fixed-position box positioned top-centre (default 64 × 64 px,
//! z-index near `MAX_SAFE_INTEGER - 1`). It shares all bridge and
//! lifecycle logic with `<tonk-portal>` through [`crate::shared`]; the
//! only difference is the iframe styling and the `__tonkFab` geometry
//! channel that lets the guest drive its own box dimensions/position.
//!
//! Attributes: `content` (guest HTML string), `runtime` (boolean —
//! injects the guest element runtime before mounting `content`).
//!
//! ## Geometry protocol
//!
//! Guest → host (JSON over `postMessage`, unguarded origin):
//! ```json
//! { "__tonkFab": { "type": "resize", "w": 320, "h": 64 } }
//! { "__tonkFab": { "type": "dragstart" } }
//! { "__tonkFab": { "type": "dragmove", "x": 200, "y": 400 } }
//! { "__tonkFab": { "type": "drop", "x": 200, "y": 400 } }
//! { "__tonkFab": { "type": "overlay" } }
//! ```
//!
//! `overlay` expands the iframe to the full viewport (so a modal dialog in the
//! guest renders unclipped) without touching the stored resting box; the guest
//! restores the FAB by posting a `resize` when the modal closes.
//!
//! Host → guest: none (the guest computes submenu direction from its own rect).

use std::cell::RefCell;
use std::rc::Rc;

use custom_elements::CustomElement;
use js_sys::Reflect;
use tonk_fab::logic::{FabIntent, FabState, geometry_box};
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use wasm_bindgen::closure::Closure;
use web_sys::{Element, HtmlElement, MessageEvent, window};

use crate::bridge::{self, PortalState};
use crate::shared::{connect_portal, install_method_shims, reload_portal};

/// Geometry state for one FAB portal instance. Starts at a default 64×64
/// position (top-centre equivalent, x/y will be set by the first real
/// geometry message).
#[derive(Clone, Copy, Debug)]
struct FabGeom {
    state: FabState,
    /// Whether a real position has been received (suppresses the initial
    /// centering transform once the first box arrives).
    has_position: bool,
}

impl Default for FabGeom {
    fn default() -> Self {
        Self {
            state: FabState {
                x: 0.0,
                y: 12.0,
                w: 64.0,
                h: 64.0,
                dragging: false,
            },
            has_position: false,
        }
    }
}

/// The FAB portal custom element. Holds the shared [`PortalState`] and
/// the per-instance [`FabGeom`]; both are `None`/default until
/// `connected_callback` builds them.
#[derive(Default)]
pub struct TonkFabPortal {
    inner: RefCell<Option<Rc<RefCell<PortalState>>>>,
    geom: RefCell<FabGeom>,
    /// The `message` listener installed by `connected_callback`. Stored so
    /// `disconnected_callback` can remove it and drop the closure.
    message_listener: RefCell<Option<Closure<dyn FnMut(MessageEvent)>>>,
}

impl CustomElement for TonkFabPortal {
    fn shadow() -> bool {
        false
    }

    fn observed_attributes() -> &'static [&'static str] {
        &["content", "runtime"]
    }

    fn inject_children(&mut self, _this: &HtmlElement) {}

    fn connected_callback(&mut self, this: &HtmlElement) {
        // `true`: the FAB is trusted first-party chrome (placed only by the
        // shell view in the top document), so its guest may relay a per-query
        // repository route — letting `<tonk-fab>`'s `<tonk-repository name=…>`
        // resolve other spaces' labels. This privilege is NOT extended to the
        // generic `<tonk-portal>`, which renders synced/untrusted content.
        connect_portal(this, &self.inner, true, |iframe| {
            // Initial fixed small box: top-centre, above all content.
            // Once the first `__tonkFab` geometry message arrives, the
            // centering transform is dropped and absolute px values take
            // over.
            let style = iframe.style();
            let _ = style.set_property("position", "fixed");
            let _ = style.set_property("top", "12px");
            let _ = style.set_property("left", "50%");
            let _ = style.set_property("transform", "translateX(-50%)");
            let _ = style.set_property("width", "64px");
            let _ = style.set_property("height", "64px");
            let _ = style.set_property("border", "0");
            let _ = style.set_property("background", "transparent");
            let _ = style.set_property("z-index", "2147483646");
            let _ = style.set_property("color-scheme", "normal");
        });

        // Install the per-instance `__tonkFab` geometry listener. It
        // captures the inner state (to reach the iframe) and the geom
        // state (to build intents). Matching by `event.source` mirrors
        // the `__tonkRuntime` pattern in `bridge.rs`.
        let inner = self.inner.borrow().clone();
        let geom = self.geom.clone();
        let listener: Closure<dyn FnMut(MessageEvent)> =
            Closure::wrap(Box::new(move |event: MessageEvent| {
                let data = event.data();

                // Only process messages that carry `__tonkFab`.
                let fab_payload =
                    Reflect::get(&data, &"__tonkFab".into()).unwrap_or(JsValue::UNDEFINED);
                if fab_payload.is_undefined() || fab_payload.is_null() {
                    return;
                }

                // Authenticate: the message must come from THIS portal's
                // iframe contentWindow (mirrors __tonkRuntime source-matching).
                let Some(ref state_rc) = inner else {
                    return;
                };
                let iframe_opt = state_rc.borrow().iframe.clone();
                let Some(iframe) = iframe_opt else {
                    return;
                };
                let source = Reflect::get(&event, &"source".into()).unwrap_or(JsValue::NULL);
                let cw: JsValue = match iframe.content_window() {
                    Some(w) => w.into(),
                    None => return,
                };
                if cw != source {
                    return;
                }

                // Read viewport dimensions.
                let (vw, vh) = window()
                    .and_then(|w| {
                        let vw = w.inner_width().ok()?.as_f64()?;
                        let vh = w.inner_height().ok()?.as_f64()?;
                        Some((vw, vh))
                    })
                    .unwrap_or((1024.0, 768.0));

                let msg_type = Reflect::get(&fab_payload, &"type".into())
                    .ok()
                    .and_then(|v| v.as_string());

                let intent = {
                    let g = *geom.borrow();
                    match msg_type.as_deref() {
                        Some("dragstart") => FabIntent::DragStart,
                        Some("overlay") => FabIntent::Overlay,
                        Some("dragmove") => {
                            let x = Reflect::get(&fab_payload, &"x".into())
                                .ok()
                                .and_then(|v| v.as_f64())
                                .unwrap_or(g.state.x);
                            let y = Reflect::get(&fab_payload, &"y".into())
                                .ok()
                                .and_then(|v| v.as_f64())
                                .unwrap_or(g.state.y);
                            FabIntent::DragMove {
                                x,
                                y,
                                state: g.state,
                            }
                        }
                        Some("drop") => {
                            let x = Reflect::get(&fab_payload, &"x".into())
                                .ok()
                                .and_then(|v| v.as_f64())
                                .unwrap_or(g.state.x);
                            let y = Reflect::get(&fab_payload, &"y".into())
                                .ok()
                                .and_then(|v| v.as_f64())
                                .unwrap_or(g.state.y);
                            FabIntent::Drop {
                                x,
                                y,
                                state: g.state,
                            }
                        }
                        Some("resize") => {
                            let w = Reflect::get(&fab_payload, &"w".into())
                                .ok()
                                .and_then(|v| v.as_f64())
                                .unwrap_or(g.state.w);
                            let h = Reflect::get(&fab_payload, &"h".into())
                                .ok()
                                .and_then(|v| v.as_f64())
                                .unwrap_or(g.state.h);
                            FabIntent::Resize {
                                w,
                                h,
                                state: g.state,
                            }
                        }
                        _ => return,
                    }
                };

                let fab_box = geometry_box(&intent, vw, vh);

                // Update persisted FabState so subsequent intents compose.
                {
                    let mut g = geom.borrow_mut();
                    match &intent {
                        FabIntent::DragStart => {
                            g.state.dragging = true;
                        }
                        FabIntent::DragMove { x, y, .. } => {
                            g.state.x = *x;
                            g.state.y = *y;
                            // dragging stays true — mid-drag
                        }
                        FabIntent::Drop { x, y, .. } => {
                            g.state.x = *x;
                            g.state.y = *y;
                            g.state.dragging = false;
                        }
                        FabIntent::Resize { w, h, .. } => {
                            g.state.w = *w;
                            g.state.h = *h;
                        }
                        // The overlay is transient chrome: it must NOT clobber
                        // the resting box, so the following `resize` can put the
                        // FAB back exactly where it was.
                        FabIntent::Overlay => {}
                    }
                    g.has_position = true;
                }

                // Apply the computed box to the iframe style.
                let style = iframe.style();
                let _ = style.set_property("left", &format!("{}px", fab_box.left));
                let _ = style.set_property("top", &format!("{}px", fab_box.top));
                let _ = style.set_property("width", &format!("{}px", fab_box.width));
                let _ = style.set_property("height", &format!("{}px", fab_box.height));
                // Drop the initial centering transform once a real position arrives.
                let _ = style.remove_property("transform");
            }) as Box<dyn FnMut(MessageEvent)>);

        if let Some(win) = window() {
            let _ =
                win.add_event_listener_with_callback("message", listener.as_ref().unchecked_ref());
        }
        // Store the closure so it stays alive while connected and can be
        // removed in `disconnected_callback`.
        *self.message_listener.borrow_mut() = Some(listener);
    }

    fn disconnected_callback(&mut self, _this: &HtmlElement) {
        // Remove the per-instance geometry listener before dropping it so
        // re-connects don't accumulate stale listeners.
        if let Some(listener) = self.message_listener.borrow_mut().take() {
            if let Some(win) = window() {
                let _ = win.remove_event_listener_with_callback(
                    "message",
                    listener.as_ref().unchecked_ref(),
                );
            }
            drop(listener);
        }

        if let Some(state) = self.inner.borrow_mut().take() {
            let mut s = state.borrow_mut();
            s.disposed = true;
            s.clear_subs();
            if let Some(iframe) = s.iframe.take() {
                bridge::unregister_portal(&iframe);
                if let Some(parent) = iframe.parent_node() {
                    let _ = parent.remove_child(&iframe);
                }
            }
        }
    }

    fn attribute_changed_callback(
        &mut self,
        this: &HtmlElement,
        name: String,
        _old: Option<String>,
        _new: Option<String>,
    ) {
        let host: Element = this.clone().into();
        // Pre-connect callbacks (during upgrade) have no state yet; the
        // initial values are read live in `connected_callback`.
        let Some(state) = self.inner.borrow().clone() else {
            return;
        };
        if name == "content" {
            reload_portal(&host, &state);
        }
    }
}

/// Register `<tonk-fab-portal>` with the page. Idempotent. Installs the
/// page-level `hello` message listener (safe to call multiple times —
/// it is guarded by a thread-local), defines the element, and installs
/// the `reset` / `error` prototype shims.
pub fn register_fab_portal() {
    bridge::install_message_listener();
    if already_registered() {
        return;
    }
    TonkFabPortal::define("tonk-fab-portal");
    install_method_shims("tonk-fab-portal");
}

fn already_registered() -> bool {
    let Some(win) = window() else {
        return false;
    };
    !win.custom_elements().get("tonk-fab-portal").is_undefined()
}
