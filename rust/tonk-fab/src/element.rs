//! `<tonk-fab>` — the floating bar.
//!
//! The element is the FABB bar (see [`crate::bar`] for the cells and
//! [`crate::markup`] for their geometry) plus the float: a `position: fixed`
//! box on a high z-index that can be dragged to any point along a viewport
//! edge.
//!
//! It keeps the tag `<tonk-fab>` because that is the mount contract — the
//! space route in `profile.yaml` renders `<tonk-fab with="main@profile:tonk"
//! space={id}>`, and the DID it passes addresses every subscription. The
//! spec's standalone `tonk-fab` (a bare sync circle) is a documentation
//! component with no product use, so the name is spent here on the thing that
//! actually floats.
//!
//! ## Shadow DOM
//!
//! Law 6 — the chrome themes itself, never the view. Under shadow that is a
//! platform guarantee rather than a convention: a space is free to ship any
//! CSS it likes and cannot reach the bar, and the bar cannot leak into the
//! space. This replaced a globally injected stylesheet, which had neither
//! property and had to be guarded against duplicate injection on every clone.
//!
//! ## Drag and dock
//!
//! A press on the circle that travels past the dead zone becomes a drag: the
//! bar tracks the pointer 1:1 on inline `left`/`top`, and on release eases to
//! the nearest edge while preserving its position along that edge. A
//! right-edge landing becomes right-anchored after the glide, so later
//! responsive run-width changes still grow leftward. The nearest corner remains the
//! persisted fallback seat used on a subsequent page load.
//!
//! A press that never travels is a click: it toggles the action run, closing
//! any open stack before collapse; alt/option keeps the existing sync-pause
//! shortcut.

use std::cell::RefCell;
use std::rc::Rc;

use custom_elements::CustomElement;
use js_sys::{Function, Object, Promise, Reflect};
use tonk_common::log;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::{JsFuture, spawn_local};
use web_sys::{
    Element, HtmlElement, PointerEvent, ResizeObserver, Response, VisibilityState, window,
};

use crate::bar;
use crate::logic::{
    DOCK_CLASSES, Dock, Edge, EdgeInsets, EdgeSnap, clamp_position, collapsed_claim_json,
    collapsed_from_conclusions, dock_claim_json, dock_from_conclusions, nearest_dock,
    pause_claim_json, repository_endpoint, snap_to_nearest_edge,
};
use crate::shadow::Bound;

/// The z-index the floating bar sits at — above page content (and the repo
/// content portal) so it never gets covered. Near `MAX_SAFE_INTEGER` to beat
/// any app stacking context.
const FAB_Z_INDEX: &str = "2147483646";

/// How far (CSS px) the pointer must travel before a press counts as a drag.
/// Below this the press remains a tap.
const DRAG_THRESHOLD_PX: f64 = 4.0;

/// The drag threshold for TOUCH pointers. Wider than the mouse threshold: a
/// finger wobbles a few px during a plain tap, and promoting that to a drag
/// would eat the tap-to-toggle gesture.
const TOUCH_DRAG_THRESHOLD_PX: f64 = 8.0;

/// The reference snap duration is 400ms; the right anchor is swapped in just
/// after it settles so changing from `left` to `right` cannot cancel the glide.
const EDGE_ANCHOR_DELAY_MS: i32 = 440;
/// Marks the bar when this device does not hold the addressed space.
///
/// The bar stays mounted because its space switcher is the way out, but the
/// controls that act on the absent replica must not be offered.
const UNKNOWN_SPACE_ATTR: &str = "data-unknown-space";

/// Marks a local profile that has not created or logged into an account yet.
const ACCOUNT_REQUIRED_ATTR: &str = "data-account-required";

/// Where the bar sits until it is dragged: bottom-right, under the thumb on
/// a phone and out of the way of page content on a desktop. A persisted dock
/// overrides it.
const DEFAULT_DOCK: Dock = Dock::BottomRight;

/// The class marking a right-anchored bar mid-drag. The `flip` ATTRIBUTE is
/// the resting truth (it reorders the bookends); this class is what survives
/// while the dock classes are dropped during a drag.
const MIRROR_CLASS: &str = "fab-mirror";

/// The `<tonk-fab>` custom element.
#[derive(Default)]
pub(crate) struct TonkFab {
    state: bar::Shared,
    listeners: Rc<RefCell<Vec<Bound>>>,
    responsive_observer: Option<ResizeObserver>,
    responsive_callback: Option<Closure<dyn FnMut(JsValue, JsValue)>>,
    activation_watch: Option<crate::activation::ActivationWatch>,
}

impl CustomElement for TonkFab {
    fn shadow() -> bool {
        // Attached by `bar::build`, so the component controls build timing.
        false
    }

    fn observed_attributes() -> &'static [&'static str] {
        &[
            "space",
            "label",
            "state",
            "alert",
            "up",
            "flip",
            "data-sync-status",
        ]
    }

    fn inject_children(&mut self, this: &HtmlElement) {
        this.set_inner_html(&crate::markup::stacks_html(
            &this.get_attribute("space").unwrap_or_default(),
        ));
    }

    fn connected_callback(&mut self, this: &HtmlElement) {
        float(this);
        ensure_stacks_stylesheet();
        // Safe before the asynchronous account answer: never flash a live
        // copy action on a profile that may not have an account.
        apply_account_ready(this, false);

        let mut listeners = bar::build(this, &self.state);
        listeners.extend(attach_drag(this, &self.state));
        *self.listeners.borrow_mut() = listeners;

        install_imperative_api(this, &self.state);
        self.listeners
            .borrow_mut()
            .extend(attach_stack_verbs(this, &self.state));
        let (observer, callback) = attach_responsive(this, &self.state);
        self.responsive_observer = observer;
        self.responsive_callback = callback;
        self.listeners
            .borrow_mut()
            .extend(attach_keyboard_lift(this));
        mount_refusal_dialogs();
        restore_position(this);
        restore_collapse(this, &self.state);
        self.listeners.borrow_mut().extend(attach_presence(this));
        self.activation_watch = crate::activation::watch(this);
    }

    fn disconnected_callback(&mut self, _this: &HtmlElement) {
        if let Some(observer) = self.responsive_observer.take() {
            observer.disconnect();
        }
        self.responsive_callback = None;
        self.activation_watch = None;
        self.listeners.borrow_mut().clear();
        bar::remove_conditions();
        crate::activation::remove();
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
        match name.as_str() {
            "flip" => bar::apply_flip(this),
            "data-sync-status" => bar::update(this),
            // The space is what every subscription is addressed to. The
            // subtree is authored ONCE, so a bar whose `space` was blank at
            // authoring time — an unsubstituted first projection — carries
            // children pointed at nothing. Restamp them, and re-ask for the
            // presence answer, when it finally lands.
            "space" => {
                restamp_space(this, new.as_deref().unwrap_or_default());
                // Probe this host directly. A document lookup would select the
                // first bar, which need not be the one whose binding changed.
                if let Some(endpoint) = host_repository_endpoint(this) {
                    spawn_local(check_presence(this.clone(), endpoint));
                }
            }
            _ => bar::update(this),
        }
    }
}

/// The id of the injected stack stylesheet, so injection is idempotent.
const STACKS_STYLE_ID: &str = "tonk-fab-stacks";

/// Inject the slotted stacks' styles once per document.
///
/// These rules cannot live in shadow CSS: slotted content is styled by the
/// document, and document styles beat `::slotted()`. Keyed off a stable
/// element id rather than an expando, so a bar landing in a fresh document
/// still gets them.
fn ensure_stacks_stylesheet() {
    let Some(document) = window().and_then(|w| w.document()) else {
        return;
    };
    if document.get_element_by_id(STACKS_STYLE_ID).is_some() {
        return;
    }
    let Some(head) = document.head() else { return };
    let Ok(style) = document.create_element("style") else {
        return;
    };
    style.set_id(STACKS_STYLE_ID);
    style.set_text_content(Some(crate::markup::STACKS_CSS));
    let _ = head.append_child(&style);
}

/// Re-stamp the space onto every child derived from it.
///
/// The children are built to heal — each observes its own attribute and
/// re-opens its subscriptions when it changes — but nothing else ever
/// re-delivers the space to them. Attributes are rewritten in place rather
/// than the subtree being re-authored, so every listener bound at connect
/// stays alive.
fn restamp_space(this: &HtmlElement, space: &str) {
    if space.is_empty() {
        return;
    }
    for (selector, attribute, value) in [
        ("ui-space-name", "space", space.to_string()),
        ("ui-member-roster", "space", space.to_string()),
        ("ui-space-switcher", "current", space.to_string()),
        ("ui-sync-status", "with", format!("main@{space}")),
    ] {
        if let Ok(Some(child)) = this.query_selector(selector) {
            // Only when it changes: every one of these targets observes
            // the attribute, and a redundant write still fires its
            // `attributeChangedCallback`. Restamping runs from inside
            // this element's own callback, so a needless write is a
            // needless re-entry.
            if child.get_attribute(attribute).as_deref() != Some(value.as_str()) {
                let _ = child.set_attribute(attribute, &value);
            }
        }
    }
}

/// Mount the share flow's refusal prompts on `<body>`, once per document.
///
/// They live outside the bar deliberately: they are modals, and an unslotted
/// light-DOM child of a shadow host never renders, so a dialog parked inside
/// `<tonk-fab>` could not be shown at all. Keyed off a stable id rather than
/// an expando, so a bar landing in a fresh document still gets them.
pub(crate) fn mount_refusal_dialogs() {
    let Some(document) = window().and_then(|w| w.document()) else {
        return;
    };
    if document.get_element_by_id("fabb-connect-cluster").is_some() {
        return;
    }
    let Some(body) = document.body() else { return };
    let Ok(holder) = document.create_element("div") else {
        return;
    };
    holder.set_inner_html(crate::markup::REFUSAL_DIALOGS_HTML);
    while let Some(child) = holder.first_element_child() {
        let _ = body.append_child(&child);
    }
}

/// Make the host a floating, fixed-position box.
fn float(this: &HtmlElement) {
    let style = this.style();
    let _ = style.set_property("position", "fixed");
    let _ = style.set_property("z-index", FAB_Z_INDEX);
}

/// The circle — the bar's only drag handle, and the target of the collapse
/// and pause gestures.
///
/// Reached through the composed path rather than `event.target`: the circle
/// lives in the shadow root, and an event crossing that boundary is
/// retargeted to the host, so `target` is always `<tonk-fab>` itself.
fn pressed_the_circle(this: &HtmlElement, event: &PointerEvent) -> Option<Element> {
    let circle = this.shadow_root()?.query_selector(".fab").ok().flatten()?;
    let path = event.composed_path();
    for index in 0..path.length() {
        if let Ok(element) = path.get(index).dyn_into::<Element>()
            && element == circle
        {
            return Some(circle);
        }
    }
    None
}

/// The circle's centre in viewport coordinates — the anchor both the live
/// mirror preview and the drop snap key on, so the two always agree.
fn handle_center(this: &HtmlElement) -> Option<(f64, f64)> {
    let circle = this.shadow_root()?.query_selector(".fab").ok().flatten()?;
    let rect = circle.get_bounding_client_rect();
    Some((
        rect.left() + rect.width() / 2.0,
        rect.top() + rect.height() / 2.0,
    ))
}

/// Preview the eventual snap continuously during a drag, rather than only at
/// the drop — and pivot the bar around the pointer when it flips.
///
/// A flip mirrors the run inside the element's fixed box, which on its own
/// would teleport the circle — the handle under the pointer — to the bar's
/// other end. So a mid-drag flip SHIFTS the whole element by the handle's
/// measured displacement (its centre before the toggle versus after): the
/// handle stays put and the bar swings around it. The shift is folded into
/// the drag's stored origin so the next pointer delta does not undo it.
///
/// Keying the decision on the HANDLE — the very point the compensation holds
/// fixed — is what makes this stable: a compensated flip cannot re-cross the
/// threshold that triggered it, so there is no oscillation and no hysteresis
/// is needed. Keying on the bar's own centre would oscillate, because the
/// compensation moves that centre by nearly a bar-width, straight back over
/// the midline.
fn apply_mirror_from_handle(this: &HtmlElement) {
    let rect = this.get_bounding_client_rect();
    let before = handle_center(this);
    let anchor_x = before.map_or(rect.left() + rect.width() / 2.0, |(x, _)| x);
    let want = crate::logic::mirrored(anchor_x, viewport_width());
    if this.class_list().contains(MIRROR_CLASS) == want {
        return;
    }
    let _ = this.class_list().toggle_with_force(MIRROR_CLASS, want);
    set_flip(this, want);

    // Compensate only while actually dragging: at promotion, and on any
    // resting call, the bar is at dock geometry and no pointer is anchored.
    if this.dataset().get("fabMoved").is_none() {
        return;
    }
    let (Some(before), Some(after)) = (before, handle_center(this)) else {
        return;
    };
    let shift = before.0 - after.0;
    if shift == 0.0 {
        return;
    }
    let left = rect.left() + shift;
    let _ = this.style().set_property("left", &format!("{left}px"));
    let start = read_data_f64(this, "fabStartLeft") + shift;
    let _ = this.dataset().set("fabStartLeft", &start.to_string());
}

/// Flip the bookends, keeping the attribute and the DOM order in step.
fn set_flip(this: &HtmlElement, flipped: bool) {
    if flipped == this.has_attribute("flip") {
        return;
    }
    if flipped {
        let _ = this.set_attribute("flip", "");
    } else {
        let _ = this.remove_attribute("flip");
    }
    bar::apply_flip(this);
}

/// Wire the press / drag / release gestures. Returns the listeners to keep
/// alive.
fn attach_drag(this: &HtmlElement, state: &bar::Shared) -> Vec<Bound> {
    let mut listeners: Vec<Bound> = Vec::new();

    // The press starts on the host: it is the only element that both sees the
    // pointer and survives the shadow retarget.
    {
        let host = this.clone();
        let shared = state.clone();
        listeners.push(crate::shadow::bind(this, "pointerdown", move |event| {
            let Some(event) = event.dyn_ref::<PointerEvent>() else {
                return;
            };
            // Only the primary button drags, and only from the circle. A
            // press anywhere else on the bar is left entirely to native
            // click, which is what the cells are wired to.
            if event.button() != 0 {
                return;
            }
            let Some(circle) = pressed_the_circle(&host, event) else {
                return;
            };
            invalidate_edge_anchor(&host);
            bar::commit_edit(&host, &shared);

            // Touch presses capture IMMEDIATELY, and on the circle rather
            // than the host: a fast flick outruns even the window listeners'
            // first delivery on some mobile browsers. Deferred capture is
            // the desktop compromise that lets a stationary mouse press
            // still produce a click.
            let dataset = host.dataset();
            if event.pointer_type() == "touch" {
                let _ = dataset.set("fabTouch", "1");
                let _ = circle.set_pointer_capture(event.pointer_id());
            } else {
                dataset.delete("fabTouch");
            }

            // Delta-based: remember where the pointer and the element each
            // started, then translate by the difference. The bar moves 1:1
            // with the cursor and drops exactly where released.
            let rect = host.get_bounding_client_rect();
            let _ = dataset.set("fabStartLeft", &rect.left().to_string());
            let _ = dataset.set("fabStartTop", &rect.top().to_string());
            let _ = dataset.set("fabDownX", &event.client_x().to_string());
            let _ = dataset.set("fabDownY", &event.client_y().to_string());
            let _ = dataset.set("fabPressing", "1");
            dataset.delete("fabMoved");
        }));
    }

    // Move / up / cancel are WINDOW-scoped: a fast flick outruns the element
    // before its first pointermove fires (capture is only taken past the
    // threshold), so element-scoped listeners lose the pointer mid-drag and
    // never see the release.
    let Some(win) = window() else {
        return listeners;
    };

    {
        let host = this.clone();
        let shared = state.clone();
        listeners.push(crate::shadow::bind(&win, "pointermove", move |event| {
            let Some(event) = event.dyn_ref::<PointerEvent>() else {
                return;
            };
            if host.dataset().get("fabPressing").is_none() {
                return;
            }
            // A press with no button still held means the pointerup was lost
            // — finish here so a later hover cannot resume a phantom press.
            if event.buttons() == 0 {
                finish_drag(&host, event.pointer_id());
                return;
            }
            let dx = event.client_x() - read_data_f64(&host, "fabDownX");
            let dy = event.client_y() - read_data_f64(&host, "fabDownY");

            if host.dataset().get("fabMoved").is_none() {
                let touch = host.dataset().get("fabTouch").is_some();
                let threshold = if touch {
                    TOUCH_DRAG_THRESHOLD_PX
                } else {
                    DRAG_THRESHOLD_PX
                };
                if dx.hypot(dy) < threshold {
                    return;
                }
                let _ = host.dataset().set("fabMoved", "1");
                // Touch already holds capture on the circle; re-capturing on
                // the host would retarget the post-drag click mid-gesture.
                if !touch {
                    let _ = host.set_pointer_capture(event.pointer_id());
                }
                let _ = host.set_attribute("dragging", "");
                // A drag cannot support an open stack: the dock classes it is
                // about to drop are the stack's vertical anchor.
                bar::close(&host, &shared);
                // Drop the dock classes so their resting position stops
                // fighting the inline left/top now tracking the pointer.
                for class in DOCK_CLASSES {
                    let _ = host.class_list().remove_1(class);
                }
                // They just vanished — resync the mirror from the live handle
                // so it does not flash upright for one frame.
                apply_mirror_from_handle(&host);
            }
            event.prevent_default();
            let left = read_data_f64(&host, "fabStartLeft") + dx;
            let top = read_data_f64(&host, "fabStartTop") + dy;
            track_position(&host, left, top);
            apply_mirror_from_handle(&host);
        }));
    }

    for event_name in ["pointerup", "pointercancel"] {
        let host = this.clone();
        listeners.push(crate::shadow::bind(&win, event_name, move |event| {
            let Some(event) = event.dyn_ref::<PointerEvent>() else {
                return;
            };
            if host.dataset().get("fabPressing").is_none() {
                return;
            }
            finish_drag(&host, event.pointer_id());
        }));
    }

    // The click the press resolves to, when it never became a drag.
    {
        let host = this.clone();
        let shared = state.clone();
        listeners.push(crate::shadow::bind(this, "click", move |event| {
            let Some(mouse) = event.dyn_ref::<web_sys::MouseEvent>() else {
                return;
            };
            // A drag's trailing click is suppressed by the same flag the
            // release leaves behind.
            if host.dataset().get("fabJustDragged").is_some() {
                host.dataset().delete("fabJustDragged");
                return;
            }
            let on_circle = host
                .shadow_root()
                .and_then(|root| root.query_selector(".fab").ok().flatten())
                .is_some_and(|circle| {
                    let path = event.composed_path();
                    (0..path.length()).any(|index| {
                        path.get(index)
                            .dyn_into::<Element>()
                            .is_ok_and(|element| element == circle)
                    })
                });
            if !on_circle {
                return;
            }
            if mouse.alt_key() {
                dispatch_pause(&host);
            } else if shared.borrow().collapsed {
                bar::expand(&host, &shared);
                persist_collapsed(false);
            } else {
                bar::collapse(&host, &shared);
                persist_collapsed(true);
            }
        }));
    }

    listeners
}

/// Wire what the stack rows actually do.
///
/// Every row reports itself as `fabb-pick`; this is the one place that turns
/// a pick into an action. Without it the stacks render and answer to hover
/// but nothing happens on click — which is exactly how `rename` and `open`
/// were inert.
fn attach_stack_verbs(this: &HtmlElement, state: &bar::Shared) -> Vec<Bound> {
    let host = this.clone();
    let shared = state.clone();
    vec![crate::shadow::bind(this, "fabb-pick", move |event| {
        let Some(row) = event
            .dyn_ref::<web_sys::CustomEvent>()
            .map(|e| e.detail())
            .and_then(|detail| Reflect::get(&detail, &"item".into()).ok())
            .and_then(|item| item.dyn_into::<Element>().ok())
        else {
            return;
        };

        // A space row: go there — unless it is the space you are on, where
        // the pick has nothing to do but put the stack away.
        if let Some(subject) = row.get_attribute("data-space") {
            if !row.has_attribute("current") {
                navigate(&format!("/space/{subject}"));
            }
            return;
        }
        if row.has_attribute("data-mi-home") {
            navigate("/");
            return;
        }
        if row.has_attribute("data-mi-cfg") {
            // Settings is a page: the /settings route serves the hub chrome
            // with the settings section open — the wireframes'
            // showHub-then-openSettings move, as a plain navigation.
            navigate("/settings");
            return;
        }
        if row.has_attribute("data-mi-rename") {
            // Close first: the stack is about to be replaced by a cursor
            // blinking in the cell the stack hangs from.
            bar::close(&host, &shared);
            bar::edit_space(&host, &shared);
            return;
        }
        if row.has_attribute("data-mi-new") {
            bar::close(&host, &shared);
            create_space();
            return;
        }
        if row.has_attribute("data-share-account") {
            bar::close(&host, &shared);
            // Raise the ceremony over the space rather than navigating to
            // `/settings`: leaving the space loses what the click was for,
            // and the share cannot finish somewhere else. The space rides
            // along so the interrupted share mints once an account exists.
            //
            // The top page does the raising — WebAuthn needs a `window`
            // and a user gesture, and this frame has neither — so this
            // asks through the portal bridge.
            let space = host.get_attribute("space").unwrap_or_default();
            tonk_host::request_registration(
                &serde_json::json!({
                    "reason": tonk_worker_api::share::BLOCKED_NEEDS_ACCOUNT,
                    "space": space,
                })
                .to_string(),
            );
            return;
        }
        if let (Some(member), Some(space)) = (
            row.get_attribute("data-member-promote"),
            row.get_attribute("data-promote-space"),
        ) {
            bar::close(&host, &shared);
            spawn_local(async move {
                match delegate(&space, "/", &member).await {
                    Ok(chain) => {
                        transact(&crate::logic::promote_claim_json(&space, &member, &chain))
                    }
                    Err(error) => log!("member/promote: the page did not delegate: {error:?}"),
                }
            });
            return;
        }
        if row.has_attribute("data-share-link") {
            // Forward into the headless `<tonk-share>`, which owns the mint
            // and the clipboard write. Synchronously, and in this same click
            // task: the clipboard write is only permitted while the user
            // activation from the original click is still live, and anything
            // deferred here spends it.
            if let Ok(Some(share)) = host.query_selector("tonk-share")
                && let Ok(share) = share.dyn_into::<HtmlElement>()
            {
                share.click();
            }
        }
    })]
}

/// Leave for `path` in the top document.
///
/// Through the host, never `location.assign`: the bar renders in a sealed
/// guest — `sandbox="allow-scripts"`, no `allow-top-navigation` — where
/// assigning `location` either moves the IFRAME or is blocked outright. The
/// page effect walks the message up to the real page, which is the only
/// frame that can navigate. This is why `more ↖` did nothing.
fn navigate(path: &str) {
    tonk_host::navigate_to(path);
}

/// Create a space and go straight into it.
///
/// No wizard and no template picker: the space starts `Untitled` (the worker
/// uniquifies it against the existing labels) and is renamed in place, with
/// the block cursor already blinking, the moment you land. Naming it costs no
/// navigation, which is the whole reason there is no form to fill in first.
///
/// The sentinel must be non-empty — the event extractor omits blank fields,
/// and with no `name` fact the transient would never reach the handler.
///
/// No remote is supplied by the page: the worker resolves where the space
/// syncs from the account's provider registration.
fn create_space() {
    let claim = crate::logic::create_space_claim_json("Untitled");
    transact(&claim);
}

/// Finish a press: clear the flags and, if it had been promoted to a drag,
/// release capture and glide to the nearest viewport edge.
fn finish_drag(this: &HtmlElement, pointer_id: i32) {
    let dataset = this.dataset();
    dataset.delete("fabPressing");
    let touch = dataset.get("fabTouch").is_some();
    dataset.delete("fabTouch");
    if dataset.get("fabMoved").is_none() {
        return;
    }
    dataset.delete("fabMoved");
    // Suppress the click this release is about to produce.
    let _ = dataset.set("fabJustDragged", "1");

    if touch {
        if let Some(circle) = this
            .shadow_root()
            .and_then(|root| root.query_selector(".fab").ok().flatten())
        {
            let _ = circle.release_pointer_capture(pointer_id);
        }
    } else {
        let _ = this.release_pointer_capture(pointer_id);
    }
    let _ = this.remove_attribute("dragging");

    let rect = this.get_bounding_client_rect();
    let (vw, vh) = (viewport_width(), viewport_height());
    let snap = snap_to_nearest_edge(
        rect.left(),
        rect.top(),
        rect.width(),
        rect.height(),
        vw,
        vh,
        float_insets(this),
    );
    apply_edge_snap(this, snap, rect.width(), rect.height(), vw, vh);

    // Persistence intentionally stays compatible with the Hub wireframe: the
    // exact along-edge point lasts for this page, while the nearest corner is
    // the stable seat restored on the next load.
    let dock = nearest_dock(
        snap.left + rect.width() / 2.0,
        snap.top + rect.height() / 2.0,
        vw,
        vh,
    );
    persist_dock(dock);
    let detail = Object::new();
    let _ = Reflect::set(&detail, &"dock".into(), &dock.symbol().into());
    let _ = Reflect::set(&detail, &"edge".into(), &snap.edge.symbol().into());
    let _ = Reflect::set(&detail, &"left".into(), &snap.left.into());
    let _ = Reflect::set(&detail, &"top".into(), &snap.top.into());
    crate::shadow::emit(this, "fabb-snap", &detail);
}

/// Toggle sync pause for the active space.
///
/// Dispatched routelessly through `window.tonk.transact`, so it lands on the
/// profile context where `tonk:pause-sync` is defined and its handler reads
/// the target space out of the command. Nothing space-side is required, which
/// is what keeps the affordance working on spaces seeded before it existed.
fn dispatch_pause(this: &HtmlElement) {
    let Some(space) = this.get_attribute("space") else {
        return;
    };
    if space.is_empty() {
        return;
    }
    let time = window()
        .and_then(|w| w.performance())
        .map(|p| p.now())
        .unwrap_or_default();
    transact(&pause_claim_json("tonk:pause-sync", &space, time));
}

/// Call `window.tonk.transact(request)` with a claim.
fn transact(claim: &serde_json::Value) {
    let Ok(json) = serde_json::to_string(claim) else {
        return;
    };
    let Some(win) = window() else { return };
    let Some(tonk) = Reflect::get(&win, &"tonk".into())
        .ok()
        .and_then(|v| v.dyn_into::<Object>().ok())
    else {
        return;
    };
    let Some(transact) = Reflect::get(&tonk, &"transact".into())
        .ok()
        .and_then(|v| v.dyn_into::<Function>().ok())
    else {
        return;
    };
    let Ok(body) = js_sys::JSON::parse(&json) else {
        return;
    };
    let _ = transact.call1(&tonk, &body);
}

/// Expose the bar's imperative surface as own properties on the instance.
///
/// `open` / `close` / `editSpace` are the handles a host page drives the bar
/// with — the create flow lands in a new space and calls `editSpace()` so the
/// block cursor is already blinking on the name.
fn install_imperative_api(this: &HtmlElement, state: &bar::Shared) {
    let host = this.clone();
    let shared = state.clone();
    let open = Closure::<dyn FnMut(String)>::new(move |cell: String| {
        bar::open(&host, &shared, &cell);
    });
    let _ = Reflect::set(this, &"open".into(), open.as_ref());
    open.forget();

    let host = this.clone();
    let shared = state.clone();
    let close = Closure::<dyn FnMut()>::new(move || bar::close(&host, &shared));
    let _ = Reflect::set(this, &"close".into(), close.as_ref());
    close.forget();

    let host = this.clone();
    let shared = state.clone();
    let edit = Closure::<dyn FnMut()>::new(move || bar::edit_space(&host, &shared));
    let _ = Reflect::set(this, &"editSpace".into(), edit.as_ref());
    edit.forget();
}

/// Ride above the software keyboard, and settle back when it recedes.
///
/// A bar seated at the bottom is exactly where a phone puts its keyboard, so
/// without this it is simply covered whenever anything is being typed. The
/// visual viewport is the only thing that reports the occlusion: the layout
/// viewport does not change when the keyboard opens, so a `fixed` element
/// keeps its coordinates and disappears underneath.
///
/// The lift is folded back into the measurement (`bottom + lift`) so the
/// calculation always describes the RESTING position — otherwise each event
/// would measure the already-lifted bar and creep upward.
fn attach_keyboard_lift(this: &HtmlElement) -> Vec<Bound> {
    let Some(viewport) = window().and_then(|w| w.visual_viewport()) else {
        return Vec::new();
    };
    let lift = Rc::new(RefCell::new(0.0_f64));
    let mut bindings = Vec::new();
    for event in ["resize", "scroll"] {
        let host = this.clone();
        let measured = viewport.clone();
        let lift = lift.clone();
        bindings.push(crate::shadow::bind(&viewport, event, move |_| {
            // Mid-drag the bar is following the pointer; a transform would
            // fight it.
            if host.has_attribute("dragging") {
                return;
            }
            let current = *lift.borrow();
            let base = host.get_bounding_client_rect().bottom() + current;
            let next =
                crate::logic::keyboard_lift_px(base, measured.offset_top(), measured.height(), 8.0);
            if next == current {
                return;
            }
            *lift.borrow_mut() = next;
            let style = host.style();
            if next > 0.0 {
                let _ = style.set_property("transform", &format!("translateY(-{next}px)"));
            } else {
                let _ = style.remove_property("transform");
            }
        }));
    }
    bindings
}

/// Apply the fit policy against the bar's parent after resolving both float
/// insets. The returned observer and callback are owned by `TonkFab`.
fn attach_responsive(
    this: &HtmlElement,
    state: &bar::Shared,
) -> (
    Option<ResizeObserver>,
    Option<Closure<dyn FnMut(JsValue, JsValue)>>,
) {
    let Some(parent) = this.parent_element() else {
        return (None, None);
    };
    let host = this.clone();
    let shared = state.clone();
    let callback = Closure::<dyn FnMut(JsValue, JsValue)>::new(move |_: JsValue, _: JsValue| {
        let parent_width = host
            .parent_element()
            .map(|p| p.client_width() as f64)
            .unwrap_or_default();
        let insets = float_insets(&host);
        bar::apply_responsive(
            &host,
            (parent_width - insets.left - insets.right).max(0.0),
            &shared,
        );
    });
    let observer = if let Ok(observer) = ResizeObserver::new(callback.as_ref().unchecked_ref()) {
        observer.observe(&parent);
        Some(observer)
    } else {
        None
    };
    let insets = float_insets(this);
    bar::apply_responsive(
        this,
        (parent.client_width() as f64 - insets.left - insets.right).max(0.0),
        state,
    );
    (observer, Some(callback))
}

/// The viewport height in CSS px, defaulting if unavailable.
fn viewport_height() -> f64 {
    window()
        .and_then(|w| w.inner_height().ok())
        .and_then(|v| v.as_f64())
        .unwrap_or(768.0)
}

/// The viewport width in CSS px, defaulting if unavailable.
fn viewport_width() -> f64 {
    window()
        .and_then(|w| w.inner_width().ok())
        .and_then(|v| v.as_f64())
        .unwrap_or(1024.0)
}

/// Read a numeric `data-*` value off the element, defaulting to 0.
fn read_data_f64(this: &HtmlElement, key: &str) -> f64 {
    this.dataset()
        .get(key)
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(0.0)
}

/// Track the bar at `(left, top)` during a drag with plain `left`/`top` — no
/// corner anchoring, so it follows the cursor 1:1 without jumping as it
/// crosses the viewport midlines. Clamped so it can never leave the viewport.
fn track_position(this: &HtmlElement, left: f64, top: f64) {
    let rect = this.get_bounding_client_rect();
    let (left, top) = clamp_position(
        left,
        top,
        rect.width(),
        rect.height(),
        viewport_width(),
        viewport_height(),
    );
    let style = this.style();
    let _ = style.remove_property("right");
    let _ = style.remove_property("bottom");
    let _ = style.set_property("left", &format!("{left}px"));
    let _ = style.set_property("top", &format!("{top}px"));
}

/// Read the safe-area-aware float margins from the shadow skin.
fn float_insets(this: &HtmlElement) -> EdgeInsets {
    let fallback = EdgeInsets {
        top: 16.0,
        right: 16.0,
        bottom: 16.0,
        left: 16.0,
    };
    let Some(wrapper) = this
        .shadow_root()
        .and_then(|root| root.query_selector(".w").ok().flatten())
    else {
        return fallback;
    };
    let Some(style) = window().and_then(|win| win.get_computed_style(&wrapper).ok().flatten())
    else {
        return fallback;
    };
    let inset = |property: &str| {
        style
            .get_property_value(property)
            .ok()
            .and_then(|value| value.trim().trim_end_matches("px").parse::<f64>().ok())
            .map_or(16.0, |safe_area| (safe_area + 8.0).max(16.0))
    };
    EdgeInsets {
        top: inset("--_sat"),
        right: inset("--_sar"),
        bottom: inset("--_sab"),
        left: inset("--_sal"),
    }
}

/// Move to an edge point with the reference 400ms position transition.
///
/// Right-edge landings become `right`-anchored only after that transition,
/// otherwise replacing `left` immediately would skip the visible glide.
fn apply_edge_snap(this: &HtmlElement, snap: EdgeSnap, width: f64, height: f64, vw: f64, vh: f64) {
    invalidate_edge_anchor(this);
    let classes = this.class_list();
    for class in DOCK_CLASSES {
        let _ = classes.remove_1(class);
    }

    let style = this.style();
    let _ = style.remove_property("right");
    let _ = style.remove_property("bottom");
    let _ = style.set_property("left", &format!("{}px", snap.left));
    let _ = style.set_property("top", &format!("{}px", snap.top));

    let flipped = if width >= vw {
        false
    } else {
        match snap.edge {
            Edge::Right => true,
            Edge::Left => false,
            Edge::Top | Edge::Bottom => snap.left + width / 2.0 >= vw / 2.0,
        }
    };
    let _ = classes.toggle_with_force(MIRROR_CLASS, flipped);
    set_flip(this, flipped);

    let opens_up = match snap.edge {
        Edge::Bottom => true,
        Edge::Top => false,
        Edge::Left | Edge::Right => snap.top + height / 2.0 >= vh / 2.0,
    };
    if opens_up {
        let _ = this.set_attribute("up", "");
    } else {
        let _ = this.remove_attribute("up");
    }

    if snap.edge == Edge::Right {
        schedule_right_anchor(this, (vw - (snap.left + width)).max(0.0));
    }
}

/// Invalidate any pending post-glide anchor without cancelling its callback.
/// Letting an obsolete one fire and no-op allows its one-shot closure to be
/// reclaimed instead of leaking every time a new drag interrupts the glide.
fn invalidate_edge_anchor(this: &HtmlElement) -> u32 {
    let next = this
        .dataset()
        .get("fabAnchorVersion")
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or_default()
        .wrapping_add(1);
    let _ = this.dataset().set("fabAnchorVersion", &next.to_string());
    next
}

/// Convert a settled right-edge point from `left` positioning to a stable
/// right anchor so responsive run-width changes grow away from the handle.
fn schedule_right_anchor(this: &HtmlElement, right: f64) {
    let Some(win) = window() else { return };
    let version = invalidate_edge_anchor(this);
    let host = this.clone();
    let anchor = Closure::once_into_js(move || {
        let current = host
            .dataset()
            .get("fabAnchorVersion")
            .and_then(|value| value.parse::<u32>().ok());
        if current != Some(version) || host.has_attribute("dragging") {
            return;
        }
        let style = host.style();
        let _ = style.set_property("right", &format!("{right}px"));
        let _ = style.set_property("left", "auto");
    });
    let _ = win.set_timeout_with_callback_and_timeout_and_arguments_0(
        anchor.unchecked_ref(),
        EDGE_ANCHOR_DELAY_MS,
    );
}

/// Dock the bar by swapping its `fab-dock-*` classes and clearing drag-time
/// inline offsets.
///
/// Anchoring by class — not a fixed pixel offset — keeps the bar pinned to
/// its corner when the viewport resizes.
fn apply_dock(this: &HtmlElement, dock: Dock) {
    let style = this.style();
    for property in ["left", "top", "right", "bottom"] {
        let _ = style.remove_property(property);
    }
    let classes = this.class_list();
    for class in DOCK_CLASSES {
        let _ = classes.remove_1(class);
    }
    for class in dock.css_classes() {
        let _ = classes.add_1(class);
    }
    // At rest the dock IS the truth (a drag drives the mirror from the live
    // handle instead).
    let right = dock.css_classes()[1] == "fab-dock-right";
    let _ = classes.toggle_with_force(MIRROR_CLASS, right);
    set_flip(this, right);
    // Seated at the bottom, stacks open upward.
    let bottom = dock.css_classes()[0] == "fab-dock-bottom";
    if bottom {
        let _ = this.set_attribute("up", "");
    } else {
        let _ = this.remove_attribute("up");
    }
    position_for_dock(this, dock);
}

/// Seat the bar in its corner.
///
/// Every side wears `max(16px, safe-area + 8px)`: 16 in a plain browser tab,
/// and the OS's own inset plus a little air wherever there is a notch or a
/// home indicator.
fn position_for_dock(this: &HtmlElement, dock: Dock) {
    let inset = |side: &str| format!("max(16px, calc(env(safe-area-inset-{side}) + 8px))");
    let style = this.style();
    let classes = dock.css_classes();
    let (vertical, horizontal) = (classes[0], classes[1]);
    if vertical == "fab-dock-top" {
        let _ = style.set_property("top", &inset("top"));
        let _ = style.set_property("bottom", "auto");
    } else {
        let _ = style.set_property("bottom", &inset("bottom"));
        let _ = style.set_property("top", "auto");
    }
    if horizontal == "fab-dock-left" {
        let _ = style.set_property("left", &inset("left"));
        let _ = style.set_property("right", "auto");
    } else {
        let _ = style.set_property("right", &inset("right"));
        let _ = style.set_property("left", "auto");
    }
}

/// Persist `dock` as a profile claim.
fn persist_dock(dock: Dock) {
    transact(&dock_claim_json(dock));
}

/// Persist whether the bar is collapsed, beside the dock.
fn persist_collapsed(collapsed: bool) {
    transact(&collapsed_claim_json(collapsed));
}

/// Run a `window.tonk.query` and hand the settled result to `then` on a
/// later task. `then` never runs when the bridge is missing — callers
/// leave their defaults standing — and runs with `None` when the query
/// itself fails.
fn profile_query(query_body: serde_json::Value, then: impl FnOnce(Option<JsValue>) + 'static) {
    let Ok(json) = serde_json::to_string(&query_body) else {
        return;
    };
    let Some(win) = window() else { return };
    let Some(tonk) = Reflect::get(&win, &"tonk".into())
        .ok()
        .and_then(|v| v.dyn_into::<Object>().ok())
    else {
        return;
    };
    let Some(query) = Reflect::get(&tonk, &"query".into())
        .ok()
        .and_then(|v| v.dyn_into::<Function>().ok())
    else {
        return;
    };
    let Ok(body) = js_sys::JSON::parse(&json) else {
        return;
    };
    let Ok(result) = query.call1(&tonk, &body) else {
        return;
    };
    let Ok(promise) = result.dyn_into::<Promise>() else {
        return;
    };
    spawn_local(async move {
        then(JsFuture::from(promise).await.ok());
    });
}

/// Restore the persisted dock, defaulting to bottom-right.
///
/// The default is queued for the first microtask so the bar is seated before
/// first paint without recursively entering its connection callback; the
/// async query swaps in the stored corner if there is one.
fn restore_position(this: &HtmlElement) {
    // `flip` and `up` are observed attributes. Writing either while the
    // custom element's connected callback still owns its component mutex
    // recursively enters that same callback lock in wasm. A resolved promise
    // resumes in the next microtask: after connection, but still before the
    // browser paints the initial dock.
    let host = this.clone();
    spawn_local(async move {
        let _ = JsFuture::from(Promise::resolve(&JsValue::UNDEFINED)).await;
        apply_dock(&host, DEFAULT_DOCK);
    });

    let query_body = serde_json::json!({
        "terms": {
            "this": "state:fab",
            "dock": { "?": { "name": "dock" } }
        },
        "predicate": {
            "description": "Persisted FAB dock (profile claim).",
            "with": {
                "dock": { "the": "xyz.tonk.fab/dock", "cardinality": "one", "as": "Entity" }
            }
        }
    });
    let host = this.clone();
    profile_query(query_body, move |rows| {
        // Fall back explicitly rather than by omission: a query that answers
        // with nothing, or fails, must still land the bar in its default
        // corner instead of wherever a half-applied earlier state left it.
        let dock = rows
            .as_ref()
            .and_then(read_dock_from_rows)
            .unwrap_or(DEFAULT_DOCK);
        apply_dock(&host, dock);
    });
}

/// Restore the persisted collapse.
///
/// Expanded is both the default and the DOM's starting state, so only a
/// stored `true` does anything — and it seats the bar without the focus
/// move a user's own collapse carries.
fn restore_collapse(this: &HtmlElement, state: &bar::Shared) {
    let query_body = serde_json::json!({
        "terms": {
            "this": "state:fab",
            "collapsed": { "?": { "name": "collapsed" } }
        },
        "predicate": {
            "description": "Persisted FAB collapse (profile claim).",
            "with": {
                "collapsed": { "the": "xyz.tonk.fab/collapsed", "cardinality": "one", "as": "Boolean" }
            }
        }
    });
    let host = this.clone();
    let state = state.clone();
    profile_query(query_body, move |rows| {
        if rows
            .as_ref()
            .and_then(read_collapsed_from_rows)
            .unwrap_or(false)
        {
            bar::seat_collapsed(&host, &state);
        }
    });
}

/// Extract the persisted dock from a `Conclusion[]` value.
fn read_dock_from_rows(rows: &JsValue) -> Option<Dock> {
    let json = js_sys::JSON::stringify(rows).ok()?.as_string()?;
    let value: serde_json::Value = serde_json::from_str(&json).ok()?;
    dock_from_conclusions(&value)
}

/// Extract the persisted collapse from a `Conclusion[]` value.
fn read_collapsed_from_rows(rows: &JsValue) -> Option<bool> {
    let json = js_sys::JSON::stringify(rows).ok()?.as_string()?;
    let value: serde_json::Value = serde_json::from_str(&json).ok()?;
    collapsed_from_conclusions(&value)
}

/// Mark the space present and clear any earlier absence stamps.
fn apply_present(this: &HtmlElement) {
    let _ = this.remove_attribute(UNKNOWN_SPACE_ATTR);
}

/// Mark a space that this device does not hold.
fn apply_unknown_space(this: &HtmlElement) {
    let _ = this.set_attribute(UNKNOWN_SPACE_ATTR, "");
}

/// Swap the share menu between its safe account handoff and copy action.
pub(crate) fn apply_account_ready(this: &HtmlElement, ready: bool) {
    if ready {
        let _ = this.remove_attribute(ACCOUNT_REQUIRED_ATTR);
    } else {
        let _ = this.set_attribute(ACCOUNT_REQUIRED_ATTR, "");
    }
    if let Ok(Some(account)) = this.query_selector("[data-share-account]") {
        if ready {
            let _ = account.set_attribute("hidden", "");
        } else {
            let _ = account.remove_attribute("hidden");
        }
    }
    if let Ok(Some(copy)) = this.query_selector("[data-share-link]") {
        if ready {
            let _ = copy.remove_attribute("hidden");
        } else {
            let _ = copy.set_attribute("hidden", "");
        }
    }
}

/// Return this bar's repository endpoint once its space binding is resolved.
fn host_repository_endpoint(this: &HtmlElement) -> Option<String> {
    repository_endpoint(&this.get_attribute("space")?).ok()
}

/// Ask the worker whether this device holds the space.
async fn check_presence(this: HtmlElement, endpoint: String) {
    let Some(win) = window() else { return };
    let Ok(value) = JsFuture::from(win.fetch_with_str(&endpoint)).await else {
        return;
    };
    let Ok(response) = value.dyn_into::<Response>() else {
        return;
    };
    if response.status() == 404 {
        apply_unknown_space(&this);
    } else if response.ok() {
        apply_present(&this);
    }
}

/// Probe presence now and whenever this tab returns to the foreground.
fn attach_presence(this: &HtmlElement) -> Vec<Bound> {
    if let Some(endpoint) = host_repository_endpoint(this) {
        spawn_local(check_presence(this.clone(), endpoint));
    }
    let Some(document) = window().and_then(|window| window.document()) else {
        return Vec::new();
    };
    let host = this.clone();
    vec![crate::shadow::bind(
        document.unchecked_ref(),
        "visibilitychange",
        move |_| {
            let hidden = window()
                .and_then(|window| window.document())
                .is_some_and(|document| document.visibility_state() == VisibilityState::Hidden);
            if !hidden && host.is_connected() {
                if let Some(endpoint) = host_repository_endpoint(&host) {
                    spawn_local(check_presence(host.clone(), endpoint));
                }
            }
        },
    )]
}

/// Ask the top page to mint a root-signed delegation hop for `audience`.
async fn delegate(subject: &str, command: &str, audience: &str) -> Result<String, JsValue> {
    let win = window().ok_or_else(|| JsValue::from_str("no window"))?;
    let tonk = Reflect::get(&win, &"tonk".into())?
        .dyn_into::<Object>()
        .map_err(|_| JsValue::from_str("no window.tonk"))?;
    let delegate = Reflect::get(&tonk, &"delegate".into())?
        .dyn_into::<Function>()
        .map_err(|_| JsValue::from_str("window.tonk.delegate is missing"))?;
    let request = Object::new();
    Reflect::set(&request, &"subject".into(), &JsValue::from_str(subject))?;
    Reflect::set(&request, &"command".into(), &JsValue::from_str(command))?;
    Reflect::set(&request, &"audience".into(), &JsValue::from_str(audience))?;
    let promise: Promise = delegate.call1(&tonk, &request)?.dyn_into()?;
    JsFuture::from(promise)
        .await?
        .as_string()
        .ok_or_else(|| JsValue::from_str("the page answered without a chain"))
}

/// Register `<tonk-fab>` with the page's custom element registry. Idempotent.
pub fn register() {
    let Some(win) = window() else {
        return;
    };
    if !win.custom_elements().get("tonk-fab").is_undefined() {
        return;
    }
    TonkFab::define("tonk-fab");
}
