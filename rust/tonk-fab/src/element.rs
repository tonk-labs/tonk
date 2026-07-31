//! The `<tonk-fab>` custom element — a floating, draggable container.
//!
//! Generic affordance: it renders its content as a `position: fixed` box on a
//! high z-index (so it floats over whatever is below) and lets the user drag it
//! around the viewport. It is NOT a portal and uses no iframe — it lives in the
//! same document as its content and moves itself directly. The FAB chrome uses
//! it to float the profile pill over the space content, but nothing here is
//! FAB-specific beyond the `.fab` class names [`crate::markup::fab_html`]
//! authors.
//!
//! - Telescope collapse/expand: the bar rests EXPANDED (all segments shown).
//!   A plain click on the circle toggles it — the segments after the cap
//!   animate their `max-width` open/closed, staggered, so the bar unfolds from
//!   / retracts into the circle.
//! - Drag: `pointerdown` (not on an interactive descendant) starts a free drag,
//!   capturing the grab offset; promotion closes any open menu and drops the
//!   `fab-dock-*` classes; `pointermove` sets the element's own `left`/`top`
//!   and, every move, resyncs the `fab-mirror` host class LIVE from the drag
//!   HANDLE's current center (the circle cap — held fixed across mirror
//!   flips), so the visual right-anchored flip previews the eventual snap
//!   continuously, not just at drop; `pointerup` SNAPS the FAB to the corner
//!   nearest the HANDLE'S CENTER (the same anchor, so the drop always
//!   matches what the live mirror was just showing) — by swapping its
//!   `fab-dock-*` classes, resyncing `fab-mirror` from the dock, and
//!   persisting the dock as a profile claim via `window.tonk.transact(...)`.
//!   The view stylesheet (profile.yaml) owns the resting pixel position and
//!   the menus' vertical open-direction (both keyed off `fab-dock-*`, which
//!   only exist at rest); the visual right-anchored flips key off
//!   `fab-mirror` instead, since that is the class still present mid-drag. A
//!   press that never moves past a small threshold is a click.
//! - `inject_children` authors the bar (already wrapped for the telescope —
//!   see `fab_html`); connect restores the persisted dock (or a default
//!   bottom-right) and applies its classes.
//!
//! The element does NOT use Shadow DOM — it is a transparent wrapper.

use crate::logic::{
    DOCK_CLASSES, Dock, clamp_position, corrected_min_width, create_space_claim_json,
    dock_claim_json, dock_from_conclusions, is_compact, membership_endpoint, mirrored,
    nearest_dock, pause_claim_json, ratchet_min_width, strip_at_end, strip_page_target,
    telescope_delay_ms, telescope_settle_ms,
};
use custom_elements::CustomElement;
use js_sys::Promise;
use js_sys::{Function, Object, Reflect};
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::{JsFuture, spawn_local};
use web_sys::{
    Element, HtmlElement, MutationObserver, MutationObserverInit, PointerEvent, Request,
    RequestInit, Response, window,
};

// web-sys doesn't expose a typed `clearTimeout`/`setTimeout` wrapper in the
// features we have, so we call them via js_sys::Function from the global.
#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = globalThis, js_name = setTimeout)]
    fn set_timeout(handler: &Function, delay: i32) -> i32;

    #[wasm_bindgen(js_namespace = globalThis, js_name = clearTimeout)]
    fn clear_timeout(id: i32);
}

/// The z-index the floating FAB sits at — above page content (and the repo
/// content portal) so it never gets covered. Near `MAX_SAFE_INTEGER` to beat
/// any app stacking context.
const FAB_Z_INDEX: &str = "2147483646";

/// The id of the injected stylesheet, so injection is idempotent.
const STYLE_ID: &str = "tonk-fab-styles";

/// Inject the FAB stylesheet once per document.
///
/// The element has no shadow root (`shadow()` below returns `false`), so the
/// CSS is global rather than scoped. It must be guarded: the element re-binds
/// on every clone (`tonk-display` clones the chrome view and mounts the
/// clone), and an unguarded injection would append a duplicate `<style>` per
/// mount. Keyed off a stable element id rather than a JS expando, since a
/// clone landing in a FRESH document still needs the stylesheet — an expando
/// guard (like `__tonkFabBound` below) would follow the clone and skip it.
fn ensure_stylesheet() {
    let Some(document) = window().and_then(|w| w.document()) else {
        return;
    };
    if document.get_element_by_id(STYLE_ID).is_some() {
        return;
    }
    let Ok(style) = document.create_element("style") else {
        return;
    };
    let _ = style.set_attribute("id", STYLE_ID);
    style.set_text_content(Some(include_str!("fab.css")));
    if let Some(head) = document.head() {
        let _ = head.append_child(style.as_ref());
    }
}

/// How far (CSS px) the pointer must travel from the press origin before it
/// counts as a drag rather than a click. Below this the press toggles the
/// telescope; above it the FAB moves and the click is suppressed. Touch pointers
/// use `TOUCH_DRAG_THRESHOLD_PX` instead.
const DRAG_THRESHOLD_PX: f64 = 4.0;

/// The drag threshold for TOUCH pointers. Wider than the mouse threshold: a
/// finger wobbles a few px during a plain tap, and promoting that to a drag
/// would eat the tap-to-toggle gesture.
const TOUCH_DRAG_THRESHOLD_PX: f64 = 8.0;

/// The `<tonk-fab>` custom element.
#[derive(Default)]
pub struct TonkFab;

impl CustomElement for TonkFab {
    fn shadow() -> bool {
        false
    }

    fn observed_attributes() -> &'static [&'static str] {
        &[]
    }

    /// Author the FAB's own DOM. The `space` attribute is stamped by the
    /// mounting view (`<tonk-fab with="main@profile:tonk" space={id}>`) and
    /// is already resolved by the time `inject_children` runs — per the
    /// `custom-elements` crate, this is deferred to (and runs before) the
    /// first `connectedCallback`, i.e. after HTML parsing has set the
    /// element's attributes.
    fn inject_children(&mut self, this: &HtmlElement) {
        let space = this.get_attribute("space").unwrap_or_default();
        this.set_inner_html(&crate::markup::fab_html(&space));
    }

    fn connected_callback(&mut self, this: &HtmlElement) {
        // Outside the `__tonkFabBound` guard below: a clone landing in a
        // fresh document still needs the stylesheet, and this is itself
        // idempotent (keyed off a stable element id, not the expando).
        ensure_stylesheet();

        // Float the element: fixed-position, high z-index. Its left/top come
        // from the restored position below.
        let style = this.style();
        let _ = style.set_property("position", "fixed");
        let _ = style.set_property("margin", "0");
        let _ = style.set_property("z-index", FAB_Z_INDEX);

        // Guard against double-binding when the SAME element reconnects.
        //
        // The marker is a JS expando PROPERTY, not a `data-*` attribute, on
        // purpose: `<tonk-display>` snapshots its view by `cloneNode`-ing the
        // authored subtree and mounting the clone. `cloneNode` copies
        // attributes but NOT event listeners or JS properties — so an
        // attribute guard would ride along on the clone (marking it "bound")
        // while the listeners stayed on the discarded original, leaving the
        // live element inert. A property is dropped by `cloneNode`, so the
        // mounted clone re-binds; it still persists across a genuine
        // disconnect/reconnect of the same node, so reconnects don't double-bind.
        let already_bound = Reflect::get(this.as_ref(), &"__tonkFabBound".into())
            .map(|v| v.is_truthy())
            .unwrap_or(false);
        if !already_bound {
            let _ = Reflect::set(this.as_ref(), &"__tonkFabBound".into(), &JsValue::TRUE);
            attach_drag(this);
            attach_gestures(this);
            attach_create_space_form(this);
            attach_profile_name_commit(this);
            attach_membership(this);
            preload_menu_widths(this);
            attach_resize(this);
            attach_strip_scroll(this);
            observe_bar_content(this);
            update_compact_mode(this);
        }
        // Restore the persisted position and apply it to our own style.
        restore_position(this);
    }

    fn disconnected_callback(&mut self, this: &HtmlElement) {
        // Cancel any pending timers so their closures don't fire against a
        // detached element.
        if let Some(id_str) = this.dataset().get("settleTimer") {
            if let Ok(id) = id_str.parse::<i32>() {
                clear_timeout(id);
            }
            this.dataset().delete("settleTimer");
        }

        // Drop any in-flight press: the window-scoped drag listeners outlive
        // a clone remount, and a press left armed on the old element would
        // let its stale `finish_drag` persist a phantom dock on the next
        // buttons-up move.
        this.dataset().delete("fabPressing");
        this.dataset().delete("fabMoved");
        this.dataset().delete("fabTouch");
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

/// The membership status the worker reports for a guest visit.
const MEMBERSHIP_GUEST: &str = "guest";

/// Host attribute marking share as unavailable, styled to dim the control.
/// Advisory: the control stays clickable, because the worker's refusal is what
/// carries the reason and the offer to join.
const SHARE_UNAVAILABLE_ATTR: &str = "data-share-unavailable";

/// Put the bar in the shape this replica's membership calls for.
///
/// Both effects follow from the same answer, so they live in one place rather
/// than drifting apart: a guest gets the join action and a share control marked
/// unavailable (the worker refuses a guest's mint), a durable member gets
/// neither. Idempotent, and separate from the fetch in [`attach_membership`] so
/// the shape is testable without a service worker to answer.
fn apply_membership(host: &HtmlElement, status: &str) {
    let guest = status == MEMBERSHIP_GUEST;
    if let Ok(Some(join)) = host.query_selector(".fab__join") {
        if guest {
            let _ = join.remove_attribute("hidden");
        } else {
            let _ = join.set_attribute("hidden", "");
        }
    }
    if guest {
        let _ = host.set_attribute(SHARE_UNAVAILABLE_ATTR, "");
    } else {
        let _ = host.remove_attribute(SHARE_UNAVAILABLE_ATTR);
    }
}

/// Show a guest-only durable-join action and promote through the worker.
fn attach_membership(host: &HtmlElement) {
    let Some(space) = host.get_attribute("space") else {
        return;
    };
    let Ok(endpoint) = membership_endpoint(&space) else {
        return;
    };
    let Ok(Some(button)) = host.query_selector(".fab__join") else {
        return;
    };
    let check_host = host.clone();
    let check_endpoint = endpoint.clone();
    spawn_local(async move {
        let Some(window) = window() else { return };
        let Ok(value) = JsFuture::from(window.fetch_with_str(&check_endpoint)).await else {
            return;
        };
        let Ok(response) = value.dyn_into::<Response>() else {
            return;
        };
        let Ok(json) = response.json() else { return };
        let Ok(value) = JsFuture::from(json).await else {
            return;
        };
        let Some(status) = Reflect::get(&value, &"status".into())
            .ok()
            .and_then(|value| value.as_string())
        else {
            return;
        };
        apply_membership(&check_host, &status);
    });

    let action_button = button.clone();
    let closure = Closure::<dyn FnMut(web_sys::Event)>::new(move |_event| {
        let endpoint = endpoint.clone();
        let button = action_button.clone();
        // Held for the whole round trip, synchronously, before anything can
        // await: promotion is a network call, and a second click would post a
        // second promotion. Every path below either hides the button (success,
        // which supersedes the disabled state) or clears it again.
        let _ = button.set_attribute("disabled", "");
        spawn_local(async move {
            let released = button.clone();
            let release = move || {
                let _ = released.remove_attribute("disabled");
            };
            let Some(window) = window() else {
                return release();
            };
            let init = RequestInit::new();
            init.set_method("POST");
            let Ok(request) = Request::new_with_str_and_init(&endpoint, &init) else {
                return release();
            };
            let Ok(value) = JsFuture::from(window.fetch_with_request(&request)).await else {
                return release();
            };
            let Ok(response) = value.dyn_into::<Response>() else {
                return release();
            };
            if response.ok() {
                let _ = button.set_attribute("hidden", "");
            } else {
                release();
            }
        });
    });
    let _ = button.add_event_listener_with_callback("click", closure.as_ref().unchecked_ref());
    closure.forget();
}

/// The click-away curtain (`.fab__scrim`) and the `.fab__tele` telescope
/// wrappers used to be retrofitted here at runtime — `inject_scrim` and
/// `wrap_telescope_tiles` — because the view-rendered markup this element
/// used to wrap had no chance to shape its own DOM: the view renderer
/// dropped empty elements (so an authored `<div class="fab__scrim"></div>`
/// never reached the DOM) and the scrim had to land as a runtime-inserted
/// SIBLING of `.fab`, never a child, or the child-order inference that found
/// the circle cap would mistake it for a collapsible tile. Now that
/// [`crate::markup::fab_html`] authors the whole subtree directly (see
/// `inject_children` above), both are emitted as real markup instead:
/// `.fab__scrim` is a literal sibling of `.fab`, and every collapsible
/// segment already comes wrapped in its own `.fab__tele` div with the
/// resting `fab--anim fab--settled` classes stamped on `.fab` itself.

/// Attach the FAB's NATIVE click gesture listener. Because only the circle is
/// draggable (see `attach_drag`), the pointer is never captured over a
/// segment, so the browser's own `click` fires normally — no manual tap
/// detection, no timers. The listener sits on the `<tonk-fab>` host and
/// routes by the event target:
///
/// - CIRCLE cap: `click` folds/expands the bar. Alt/option-`click` is the pause
///   gesture, handled by the cap's own `<ui-sync-status onpause=…>` — so this
///   handler leaves an alt-click alone (no fold) and lets it toggle sync there.
/// - SPOT segment: `click` toggles the switcher menu.
/// - SHARE segment: `click` toggles the roster menu.
///
/// The name/spot editables edit on their OWN native `dblclick` (editable.rs).
fn attach_gestures(element: &HtmlElement) {
    let el_click = element.clone();
    let on_click = Closure::<dyn FnMut(web_sys::MouseEvent)>::new(move |e: web_sys::MouseEvent| {
        let Some(t) = e.target().and_then(|t| t.dyn_into::<Element>().ok()) else {
            return;
        };
        if t.closest(".fab__scrim").ok().flatten().is_some() {
            // The click-away curtain. It only has a hit area while a dropdown is
            // open (CSS: `.fab:has(.is-open) .fab__scrim`), so reaching here
            // means the user clicked outside every menu — retract both. The
            // curtain lies behind the bar, so this never fires for a click on
            // the bar itself or on a menu row.
            close_menus(&el_click);
        } else if t.closest(".fab__more").ok().flatten().is_some() {
            advance_strip(&el_click);
        } else if let Some(cap) = t.closest(".fab__cap-l").ok().flatten() {
            // Alt/option-click toggles sync pause; a plain click folds/expands.
            // Pause is dispatched here (on the FAB, which reliably receives the
            // click — the cloned `<ui-sync-status>` inside the cap cannot own a
            // live DOM listener) reading the target space + command off the
            // cap's `<ui-sync-status with=… onpause=…>`.
            if e.alt_key() {
                dispatch_pause_from_cap(&cap);
            } else {
                toggle_telescope(&el_click);
            }
        } else if t
            .closest(".fab__menu, .fab__share-menu")
            .ok()
            .flatten()
            .is_some()
        {
            // A click inside an open menu acts on that menu's own row. If
            // it hit an actionable item (a space link, "all spots", "new"),
            // the interaction is complete — retract the dropdown so it
            // doesn't sit open over the next view.
            if t.closest(".fab__menu a, .fab__menu button")
                .ok()
                .flatten()
                .is_some()
                && let Some(seg) = el_click.query_selector(".fab__repo.is-open").ok().flatten()
            {
                let _ = seg.class_list().remove_1("is-open");
            }
        } else if let Some(seg) = t.closest(".fab__repo").ok().flatten() {
            toggle_menu(&el_click, &seg, ".fab__share");
        } else if let Some(seg) = t.closest(".fab__share").ok().flatten() {
            toggle_menu(&el_click, &seg, ".fab__repo");
        }
    });

    let target: &web_sys::EventTarget = element.unchecked_ref();
    target
        .add_event_listener_with_callback("click", on_click.as_ref().unchecked_ref())
        .ok();
    on_click.forget();
}

/// Toggle sync pause for the active space. Reads the target space and the
/// command URI off the cap's `<ui-sync-status with="branch@repo" onpause=…>`
/// (`markup::fab_html` stamps `with="main@{space}"` there), builds the
/// `tonk:pause-sync` transient, and dispatches it through
/// `window.tonk.transact` — routeless, so it lands on the FAB portal's own
/// context (`main@profile:tonk`, where the command is defined and its
/// handler reads the target space from the command). Nothing space-side is
/// required, and this runs from the FAB's own click listener, which — unlike
/// a listener on the cloned `<ui-sync-status>` — reliably receives the click.
fn dispatch_pause_from_cap(cap: &Element) {
    let Some(status) = cap.query_selector("ui-sync-status").ok().flatten() else {
        return;
    };
    let command = status
        .get_attribute("onpause")
        .filter(|c| !c.is_empty())
        .unwrap_or_else(|| "tonk:pause-sync".to_owned());
    // The cap's `with` is a `branch@repo` location — take the repo half.
    let Some(space) = status
        .get_attribute("with")
        .filter(|w| !w.is_empty() && !w.contains('{'))
        .map(|w| w.rsplit('@').next().unwrap_or(&w).to_owned())
        .filter(|s| !s.is_empty())
    else {
        return;
    };

    transact(&pause_claim_json(&command, &space, js_sys::Date::now()));
}

/// Dispatch a `TransactRequest` JSON body via `window.tonk.transact(...)`,
/// routeless. Shared by every FAB-owned command dispatch (pause, dock
/// persistence, create-space, profile-rename) — each builds its own claim
/// JSON via `crate::logic` and hands it here.
fn transact(claim: &serde_json::Value) {
    let json_str = match serde_json::to_string(claim) {
        Ok(s) => s,
        Err(_) => return,
    };
    let Some(win) = window() else { return };
    let Some(tonk) = Reflect::get(&win, &"tonk".into())
        .ok()
        .and_then(|v| v.dyn_into::<Object>().ok())
    else {
        return;
    };
    let Some(transact_fn) = Reflect::get(&tonk, &"transact".into())
        .ok()
        .and_then(|v| v.dyn_into::<Function>().ok())
    else {
        return;
    };
    if let Some(obj) = js_sys::JSON::parse(&json_str).ok() {
        transact_fn.call1(&tonk, &obj).ok();
    }
}

/// Attach the create-space wizard's `submit` handler directly to
/// `#fab-space-create-form`.
///
/// This markup used to live inside a `<tonk-display model="tonk:profile/fab">`
/// wrapper, whose own render pass rewrote `onsubmit=space/create` into
/// `data-onsubmit` and installed a `tonk-display::events::delegate::Delegate`
/// that resolved the concept descriptor and dispatched the claim (see
/// `markup.rs`'s module docs). `<tonk-fab>` sets this markup via
/// `set_inner_html` directly, so no such delegate exists — this reimplements,
/// from Rust, the three things that delegate did for this one form:
/// `preventDefault()` (else the browser falls through to a native GET submit
/// and reloads the page with `?name=` — see `tonk-display/src/events/
/// extract.rs` around line 631), reading the submitted fields, and dismissing
/// the dialog (`maybe_dismiss_overlay` in `tonk-display/src/events/
/// delegate.rs`).
///
/// The form is static markup present at connect time (unlike the profile-name
/// chip, which may render asynchronously), so a direct listener — rather than
/// delegation on the host — is enough.
fn attach_create_space_form(element: &HtmlElement) {
    let Some(form) = element
        .query_selector("#fab-space-create-form")
        .ok()
        .flatten()
    else {
        return;
    };
    let on_submit = Closure::<dyn FnMut(web_sys::Event)>::new(move |event: web_sys::Event| {
        // Unconditional and first: the browser has already run native
        // constraint validation by the time `submit` fires, so this only
        // ever suppresses the reload, never a legitimate validation error.
        event.prevent_default();
        let Some(target) = event.target().and_then(|t| t.dyn_into::<Element>().ok()) else {
            return;
        };
        let name = form_field(&target, "name");
        let remote = form_field(&target, "remote");
        // Filled by `<tonk-default-remote relay-field="revocation">` from
        // this deployment's config; blank when it declares no relay.
        let revocation = form_field(&target, "revocation");
        let template = form_field(&target, "template");
        transact(&create_space_claim_json(
            &name,
            &remote,
            &revocation,
            &template,
        ));
        dismiss_overlay(&target);
    });
    let target: &web_sys::EventTarget = form.unchecked_ref();
    let _ = target.add_event_listener_with_callback("submit", on_submit.as_ref().unchecked_ref());
    on_submit.forget();
}

/// Read `form.elements[field].value` the way `dom.event.current-target.
/// elements.<field>/value` reads it in the browser: a plain `Reflect` walk,
/// not a typed `HtmlFormElement`/`HtmlInputElement` cast. That matters for
/// `template`: three radios share `name="template"`, so
/// `form.elements.template` is a `RadioNodeList`, not a single control —
/// its own `.value` getter already resolves to the checked radio's value,
/// exactly like a single input's. A typed cast to `HtmlInputElement` would
/// fail on that shape; the untyped walk handles both uniformly.
fn form_field(form: &Element, field: &str) -> String {
    Reflect::get(form, &JsValue::from_str("elements"))
        .and_then(|elements| Reflect::get(&elements, &JsValue::from_str(field)))
        .and_then(|item| Reflect::get(&item, &JsValue::from_str("value")))
        .ok()
        .and_then(|v| v.as_string())
        .unwrap_or_default()
}

/// Reimplements `tonk_display::events::delegate::maybe_dismiss_overlay` for
/// FAB-owned markup, where no `Delegate` is installed to run the original.
/// `target` is the element the effect fired on (the form, for a submit).
///
/// Two markers, each a no-op unless present — see the original for why:
/// - `[data-close-dialog]` closes the nearest `<wa-dialog>` (sets `open =
///   false`).
/// - `[data-close-radio="<id>"]` re-checks the CSS-paging radio with that id
///   and, when the marked element is itself a form, resets it. The FAB's own
///   create form doesn't carry this marker (only the Hub's onboarding
///   overlay and remove-confirm forms do), so this branch is currently a
///   no-op here — kept for parity with the original and in case a future
///   FAB-owned form adds one.
fn dismiss_overlay(target: &Element) {
    if let Some(marked) = target.closest("[data-close-dialog]").ok().flatten()
        && let Some(dialog) = marked.closest("wa-dialog").ok().flatten()
    {
        let _ = Reflect::set(&dialog, &JsValue::from_str("open"), &JsValue::FALSE);
    }
    if let Some(marked) = target.closest("[data-close-radio]").ok().flatten()
        && let Some(id) = marked.get_attribute("data-close-radio")
        && let Some(doc) = marked.owner_document()
        && let Some(radio) = doc.get_element_by_id(&id)
    {
        let _ = Reflect::set(&radio, &JsValue::from_str("checked"), &JsValue::TRUE);
        if let Ok(reset_fn) = Reflect::get(&marked, &JsValue::from_str("reset"))
            .and_then(|v| v.dyn_into::<Function>())
        {
            let _ = reset_fn.call0(&marked);
        }
    }
}

/// Attach a delegated `change` listener on the whole `<tonk-fab>` host for
/// the profile-name chip's commit, mirroring `attach_gestures`'s click
/// delegation (not `dispatch_pause_from_cap`'s direct-child lookup): the
/// chip's `<tonk-editable data-rename="tonk:profile">` renders inside a
/// nested `<tonk-display model="tonk:profile/name">` — asynchronously,
/// after that display's own subscribe resolves — so a listener attached
/// once at connect time to a `query_selector` result would silently find
/// nothing. Delegation on the host catches the bubbling `change`
/// (`tonk-workspace::editable` dispatches a bubbling native `Event`)
/// whenever it eventually appears.
///
/// Filters on `[data-rename="tonk:profile"]` specifically: the create-space
/// dialog's own `change`-firing radios also live under this host and must
/// not be mistaken for a name commit.
fn attach_profile_name_commit(element: &HtmlElement) {
    let on_change = Closure::<dyn FnMut(web_sys::Event)>::new(move |event: web_sys::Event| {
        let Some(target) = event.target().and_then(|t| t.dyn_into::<Element>().ok()) else {
            return;
        };
        let Some(editable) = target
            .closest("[data-rename=\"tonk:profile\"]")
            .ok()
            .flatten()
        else {
            return;
        };
        let name = editable.text_content().unwrap_or_default();
        if name.trim().is_empty() {
            if let Some(profile_name) = editable.closest("ui-profile-name").ok().flatten()
                && let Some(previous) = profile_name.get_attribute("data-subscribed-name")
            {
                editable.set_text_content(Some(&previous));
            }
            return;
        }
        let profile_name = editable.closest("ui-profile-name").ok().flatten();
        wasm_bindgen_futures::spawn_local(async move {
            if let Err(error) = tonk_host::set_account_display_name(name.trim()).await {
                web_sys::console::warn_1(&JsValue::from_str(&format!(
                    "account display-name write failed: {error:?}"
                )));
                if let Some(previous) = profile_name
                    .as_ref()
                    .and_then(|host| host.get_attribute("data-subscribed-name"))
                {
                    editable.set_text_content(Some(&previous));
                }
                if let Some(window) = web_sys::window() {
                    let _ = window.alert_with_message(
                        "Name wasn’t changed. Open /account to finish or retry account setup, then try again.",
                    );
                }
            }
        });
    });
    let target: &web_sys::EventTarget = element.unchecked_ref();
    let _ = target.add_event_listener_with_callback("change", on_change.as_ref().unchecked_ref());
    on_change.forget();
}

/// Open (or close) the dropdown owned by `seg` by toggling its `is-open` class,
/// closing the other menu (matched by `other_sel`) so only one is open at a time.
/// The open-direction is CSS, keyed off the FAB's `fab-dock-*` class.
///
/// On open the segment is widened (an eased inline `min-width`) to the menu's
/// natural width when the menu is the wider of the two — the stylesheet's
/// `width: 100%` then makes menu and rung exactly equal. The stamped
/// `min-width` RATCHETS: it is never cleared, so a column keeps its width
/// across open/close and across the other menu's toggles, and only grows —
/// re-measured on each open — when a wider element has entered the menu.
/// (Clearing on close made the bar's columns visibly resize depending on
/// which dropdown was open.)
fn toggle_menu(element: &HtmlElement, seg: &Element, other_sel: &str) {
    // No menu work while the bar is mid-drag (a second pointer's click): the
    // dock classes that anchor an open menu are stripped during a drag, so an
    // open here would float unanchored mid-bar.
    if element
        .query_selector(".fab.dragging")
        .ok()
        .flatten()
        .is_some()
    {
        return;
    }
    if let Some(other) = element.query_selector(other_sel).ok().flatten() {
        other.class_list().remove_1("is-open").ok();
    }
    let opening = !seg.class_list().contains("is-open");
    seg.class_list().toggle_with_force("is-open", opening).ok();
    if opening {
        equalize_menu_width(seg);
    }
}

/// Advance the compact pager one page, wrapping to the start at the end —
/// the tap alternative to swiping the strip (the compact bar's only other
/// horizontal gesture). Smooth so the slide reads as paging, not a jump;
/// the strip's own `scroll` listener dismisses any open dropdown as the
/// segments move out from under it.
fn advance_strip(element: &HtmlElement) {
    let Some(strip) = element.query_selector(".fab__strip").ok().flatten() else {
        return;
    };
    let target = strip_page_target(
        strip.scroll_left() as f64,
        strip.client_width() as f64,
        strip.scroll_width() as f64,
    );
    let options = web_sys::ScrollToOptions::new();
    options.set_left(target);
    options.set_behavior(web_sys::ScrollBehavior::Smooth);
    strip.scroll_to_with_scroll_to_options(&options);
}

/// Measure the menu's natural (max-content) width — momentarily overriding
/// the stylesheet's `width: 100%`, reading the box, restoring — and stamp the
/// segment's inline `min-width` to the RATCHETED target (never below a prior
/// stamp — see `ratchet_min_width`). Works whether the menu is open or
/// closed: a closed menu (`display: none`) is forced measurable — laid out at
/// its natural width, invisible and out of the paint — then restored, all
/// within one task, so nothing flashes. Called on open (`toggle_menu`), at
/// connect and on content mutation (`preload_menu_widths`, `observe_menu`),
/// and once fonts finish loading (`refresh_on_fonts_ready`). On the
/// first-ever stamp, pins the segment's current rendered width and flushes
/// layout before the target, so the 0.2s `min-width` ease has a numeric start
/// instead of animating from the unanimatable `auto`.
fn equalize_menu_width(seg: &Element) {
    // Not while compact: the compact dropdown is bar-width by rule, not
    // rung-equalized, so a stamp buys nothing there — and mid-compact the
    // segment rect under-reports, making any ratchet taken here junk. The
    // stamps that already exist stay (the pager CSS neutralizes them with
    // `min-width: 0 !important`), so the wide layout — and the compact-fit
    // measurement, which unclamps to the wide layout — is identical before
    // and after a trip through compact.
    if seg.closest(".fab--compact").ok().flatten().is_some() {
        return;
    }
    let Some(menu) = seg.query_selector(".fab__menu").ok().flatten() else {
        return;
    };
    let natural = menu_natural_width(seg, &menu);
    let seg_el = seg.unchecked_ref::<HtmlElement>();
    let segment = seg.get_bounding_client_rect().width();
    // A prior ratchet stamp, read back off the inline style ("260px" → 260.0).
    let stamped = seg_el
        .style()
        .get_property_value("min-width")
        .ok()
        .and_then(|v| v.strip_suffix("px").and_then(|n| n.parse::<f64>().ok()));
    if let Some(min_width) = ratchet_min_width(natural, segment, stamped) {
        // Give the 0.2s ease a NUMERIC start on the first stamp: `min-width`
        // rests at `auto` (not animatable), so pin the current rendered width
        // and flush layout before stamping the target — otherwise the first
        // (and, ratcheted, usually only) widening snaps instead of easing.
        if stamped.is_none() {
            let _ = seg_el
                .style()
                .set_property("min-width", &format!("{segment}px"));
            let _ = seg_el.offset_width();
        }
        let _ = seg_el
            .style()
            .set_property("min-width", &format!("{min_width}px"));
    }
}

/// Measure the menu's natural (max-content) width, open or closed — a closed
/// menu (`display: none`) is momentarily forced measurable, invisible and out
/// of the paint (`visibility: hidden`); everything is restored before return.
/// Synchronous within one task, so nothing flashes.
fn menu_natural_width(seg: &Element, menu: &Element) -> f64 {
    let style = menu.unchecked_ref::<HtmlElement>().style();
    // A closed menu is `display: none` (no boxes). Force it measurable —
    // invisible and out of the paint (`visibility: hidden`), laid out at its
    // natural width — then restore. All within one task, so no flash.
    let closed = !seg.class_list().contains("is-open");
    if closed {
        let _ = style.set_property("display", "flex");
        let _ = style.set_property("visibility", "hidden");
    }
    let _ = style.set_property("width", "max-content");
    let natural = menu.get_bounding_client_rect().width();
    let _ = style.remove_property("width");
    if closed {
        let _ = style.remove_property("display");
        let _ = style.remove_property("visibility");
    }
    natural
}

/// Restamp `seg`'s width from a FRESH measurement, replacing any ratcheted
/// stamp in both directions — the one-time correction for stamps taken
/// against fallback-font metrics before the Plex face landed. The min-width
/// transition eases the correction, riding the font swap's own reflow.
fn restamp_menu_width(seg: &Element) {
    // Same compact suspension as `equalize_menu_width`, and for the same
    // reason: inline stamps must not fight the pager's `min-width: 0`.
    if seg.closest(".fab--compact").ok().flatten().is_some() {
        return;
    }
    let Some(menu) = seg.query_selector(".fab__menu").ok().flatten() else {
        return;
    };
    let natural = menu_natural_width(seg, &menu);
    if let Some(min_width) = corrected_min_width(natural) {
        let _ = seg
            .unchecked_ref::<HtmlElement>()
            .style()
            .set_property("min-width", &format!("{min_width}px"));
    }
}

/// The two dropdown-owning segments.
const MENU_SEGMENTS: [&str; 2] = [".fab__repo", ".fab__share"];

/// The `fab-mirror` host class carrying the visual right-anchored flips —
/// separate from the `fab-dock-*` classes (position + menu vertical
/// direction) because a drag removes those while the mirror must track the
/// bar LIVE across the midline.
const MIRROR_CLASS: &str = "fab-mirror";

/// The grab handle's (circle cap's) viewport center, or `None` if the bar
/// hasn't rendered one. The handle is the drag's anchor: the flip
/// compensation holds it fixed, so decisions keyed on it are stable across
/// mirror flips (the bar's own center moves by nearly a bar-width per flip
/// and would oscillate).
fn handle_center(el: &HtmlElement) -> Option<(f64, f64)> {
    el.query_selector(".fab__cap-l").ok().flatten().map(|c| {
        let r = c.get_bounding_client_rect();
        (r.left() + r.width() / 2.0, r.top() + r.height() / 2.0)
    })
}

/// Set the mirror from the drag HANDLE's current center: mirrored on the
/// right half of the viewport, upright on the left. Called per drag move
/// (apply_dock owns the resting sync). Falls back to the bar-rect center
/// only if the handle is missing.
///
/// A flip row-reverses the bar inside its fixed box, which would teleport
/// the circle — the grab handle under the pointer — to the bar's other end.
/// So a mid-drag flip SHIFTS the bar by the handle's measured displacement
/// (its center before vs after the class toggle): the handle stays put, the
/// bar swings around it. The shift is folded into the drag's stored
/// `fabStartLeft` so the next pointer delta doesn't undo it. Because the
/// flip decision is keyed on the handle — the very point the compensation
/// holds fixed — a compensated flip cannot re-cross the threshold that
/// triggered it: no oscillation, no hysteresis needed. (Keying on the bar's
/// own center would oscillate: the compensation moves it by nearly a
/// bar-width, straight back across the midline.)
fn apply_mirror_from_handle(el: &HtmlElement) {
    let rect = el.get_bounding_client_rect();
    let before = handle_center(el);
    let anchor_x = before.map_or(rect.left() + rect.width() / 2.0, |(x, _)| x);
    let want = mirrored(anchor_x, viewport_width());
    if el.class_list().contains(MIRROR_CLASS) == want {
        return;
    }
    el.class_list().toggle_with_force(MIRROR_CLASS, want).ok();
    // Compensate only while actually dragging: at promotion (and any
    // non-drag call) the bar is at rest geometry and no pointer is anchored.
    if el.dataset().get("fabMoved").is_none() {
        return;
    }
    let (Some(before), Some(after)) = (before, handle_center(el)) else {
        return;
    };
    let shift = before.0 - after.0;
    if shift != 0.0 {
        let left = rect.left() + shift;
        let _ = el.style().set_property("left", &format!("{left}px"));
        let start = read_data_f64(el, "fabStartLeft") + shift;
        el.dataset().set("fabStartLeft", &start.to_string()).ok();
    }
}

/// Dismiss both dropdowns (the ratcheted widths stay). The single dismissal
/// point, called by any interaction that invalidates an open menu's
/// anchoring: a drag (which drops the `fab-dock-*` classes anchoring the
/// menu to the bar, so an open menu would float unanchored mid-bar), mode
/// switches, pager swipes and taps, or click-away.
fn close_menus(el: &HtmlElement) {
    for sel in MENU_SEGMENTS {
        if let Some(seg) = el.query_selector(sel).ok().flatten() {
            seg.class_list().remove_1("is-open").ok();
        }
    }
}

/// Stamp both segments' ratcheted widths up front and keep them fresh, so a
/// dropdown OPEN never changes the bar: rows render asynchronously (a
/// MutationObserver per menu re-ratchets as content lands) and the Plex face
/// loads asynchronously (a font swap changes metrics but fires no mutation,
/// so `document.fonts.ready` triggers one more pass).
fn preload_menu_widths(element: &HtmlElement) {
    for sel in MENU_SEGMENTS {
        if let Some(seg) = element.query_selector(sel).ok().flatten() {
            equalize_menu_width(&seg);
            observe_menu(&seg);
        }
    }
    refresh_on_fonts_ready(element);
}

/// Re-ratchet `seg`'s width whenever its menu's content changes (rows arrive
/// from live subscriptions well after connect). The observer lives as long as
/// the page (closure forgotten) — one FAB, two menus, so no accounting.
fn observe_menu(seg: &Element) {
    let Some(menu) = seg.query_selector(".fab__menu").ok().flatten() else {
        return;
    };
    let seg_for_cb = seg.clone();
    let cb = Closure::<dyn FnMut(js_sys::Array, web_sys::MutationObserver)>::new(
        move |_records: js_sys::Array, _obs: web_sys::MutationObserver| {
            equalize_menu_width(&seg_for_cb);
        },
    );
    if let Ok(observer) = MutationObserver::new(cb.as_ref().unchecked_ref()) {
        let init = MutationObserverInit::new();
        init.set_child_list(true);
        init.set_subtree(true);
        init.set_character_data(true);
        observer.observe_with_options(&menu, &init).ok();
    }
    cb.forget();
}

/// One authoritative restamp once the fonts land: measurements taken before
/// this point (connect, mutation) used the fallback face, which typically
/// OVER-reports condensed Plex's metrics — and the ratchet those passes use
/// can only widen, never correct an over-wide stamp back down. This pass
/// restamps from a fresh measurement in both directions instead of ratcheting.
fn refresh_on_fonts_ready(element: &HtmlElement) {
    let Some(document) = window().and_then(|w| w.document()) else {
        return;
    };
    let ready = match document.fonts().ready() {
        Ok(p) => p,
        Err(_) => return,
    };
    let el = element.clone();
    spawn_local(async move {
        let _ = wasm_bindgen_futures::JsFuture::from(ready).await;
        for sel in MENU_SEGMENTS {
            if let Some(seg) = el.query_selector(sel).ok().flatten() {
                restamp_menu_width(&seg);
            }
        }
        // The face swap changes every label's metrics — the same reflow
        // that invalidates the stamps invalidates the compact-fit call.
        update_compact_mode(&el);
    });
}

/// Re-evaluate compact mode whenever the bar's CONTENT changes: the profile
/// and space names arrive asynchronously from live subscriptions, so the
/// width measured at connect is the EMPTY bar's — without this, a phone
/// sits in wide mode until some unrelated trigger (a resize, a telescope
/// settle) happens to re-measure. Child-list/text mutations only: the
/// class flips `update_compact_mode` itself performs are attribute
/// mutations and cannot re-fire the observer.
fn observe_bar_content(element: &HtmlElement) {
    let Some(fab) = element.query_selector(".fab").ok().flatten() else {
        return;
    };
    let el = element.clone();
    let cb = Closure::<dyn FnMut(js_sys::Array, MutationObserver)>::new(
        move |_records: js_sys::Array, _obs: MutationObserver| {
            update_compact_mode(&el);
        },
    );
    if let Ok(observer) = MutationObserver::new(cb.as_ref().unchecked_ref()) {
        let init = MutationObserverInit::new();
        init.set_child_list(true);
        init.set_subtree(true);
        init.set_character_data(true);
        observer.observe_with_options(&fab, &init).ok();
    }
    cb.forget();
}

/// Toggle the telescope open/closed: flip `fab--collapsed` on `.fab` and drive
/// each `.fab__tele` tile's `max-width` / `margin-left` / staggered
/// `transition-delay`, then schedule the post-animation `settled` state that
/// unclamps `max-width` so expanded content can reflow freely.
fn toggle_telescope(element: &HtmlElement) {
    let Some(fab) = element.query_selector(".fab").ok().flatten() else {
        return;
    };
    let collapsing = !fab.class_list().contains("fab--collapsed");
    set_telescope(element, &fab, collapsing);
}

/// Drive the telescope to the given state. `collapsing = true` retracts the
/// tiles into the circle; `false` unfolds them to their measured widths.
fn set_telescope(element: &HtmlElement, fab: &Element, collapsing: bool) {
    // Compact collapse is CSS-driven: the strip and the chevron cap
    // transition their own max-width (see `.fab--compact.fab--collapsed`
    // in fab.css). Driving per-tile inline max-widths here would zero out
    // the pages the strip lays out.
    if fab.class_list().contains("fab--compact") {
        // A collapse retracts everything: a floating dropdown left standing
        // would hover over a bar that is shrinking to a bare circle, with
        // its scrim still armed.
        close_menus(element);
        // Collapsed and settled are mutually exclusive, exactly as in the
        // wide branch below: `fab--settled`'s unclamp (`max-width: none` on
        // every shown tile, and a higher-specificity rule than the collapse
        // clamp) would hold the chevron's tile open while the strip
        // retracts — the collapsed pill kept its arrow.
        fab.class_list()
            .toggle_with_force("fab--settled", !collapsing)
            .ok();
        fab.class_list()
            .toggle_with_force("fab--collapsed", collapsing)
            .ok();
        return;
    }

    let tiles = telescope_tiles(fab);
    let count = tiles.len();

    // Clear any prior settle timer + `settled` class: while animating, tiles
    // must be clamped (overflow hidden) so `max-width` can drive them.
    if let Some(id_str) = element.dataset().get("settleTimer") {
        if let Ok(id) = id_str.parse::<i32>() {
            clear_timeout(id);
        }
        element.dataset().delete("settleTimer");
    }
    fab.class_list().remove_1("fab--settled").ok();

    // Measure natural widths BEFORE mutating the state class, so an
    // already-expanded tile reports its true width (see `measure_tile_widths`).
    let widths = if collapsing {
        Vec::new()
    } else {
        measure_tile_widths(&tiles)
    };

    // Collapsing from the settled state, shown tiles rest at `max-width: none`
    // (see `schedule_settle`), and a `none → 0` transition does not animate.
    // Pin each tile to its current rendered width and flush layout, so the
    // clamp-to-zero below animates from a concrete start value.
    if collapsing {
        for tile in &tiles {
            let w = tile.get_bounding_client_rect().width();
            let style = tile.unchecked_ref::<HtmlElement>().style();
            let _ = style.set_property("max-width", &format!("{w}px"));
        }
        let _ = fab.unchecked_ref::<HtmlElement>().offset_width();
    }

    for (i, tile) in tiles.iter().enumerate() {
        let style = tile.unchecked_ref::<HtmlElement>().style();
        let delay = telescope_delay_ms(i, count, collapsing);
        let _ = style.set_property("transition-delay", &format!("{delay}ms"));
        // A tile is hidden only while the whole bar is folding; when expanded
        // every segment shows (no per-section disclosure gate).
        let hidden = collapsing;
        // Mark hidden tiles so the post-settle `overflow: visible; max-width:
        // none` unclamp SKIPS them while collapsed.
        tile.class_list()
            .toggle_with_force("fab__tele--hidden", hidden)
            .ok();
        if hidden {
            let _ = style.set_property("max-width", "0px");
            let _ = style.set_property("margin-left", "-2px");
        } else {
            let w = widths.get(i).copied().unwrap_or(0.0);
            let _ = style.set_property("max-width", &format!("{w}px"));
            let _ = style.set_property("margin-left", "0px");
        }
    }

    if collapsing {
        fab.class_list().add_1("fab--collapsed").ok();
    } else {
        fab.class_list().remove_1("fab--collapsed").ok();
        // After the sweep, mark settled so `max-width` unclamps (`none`) and
        // the expanded content can reflow (e.g. a growing invite link).
        schedule_settle(element, fab, count);
    }
}

/// Collect the `.fab__tele` wrapper tiles, sorted into VISUAL order. The
/// DOM groups tiles by compact page (repo before share/account) while CSS
/// `order` restores the wide bar's visual order — the telescope stagger
/// must follow what the eye sees, not the DOM. A child scan no longer
/// works: the tiles live inside `.fab__strip` > `.fab__page` wrappers.
fn telescope_tiles(fab: &Element) -> Vec<Element> {
    let mut out = Vec::new();
    if let Ok(list) = fab.query_selector_all(".fab__tele") {
        for i in 0..list.length() {
            if let Some(node) = list.item(i)
                && let Ok(el) = node.dyn_into::<Element>()
            {
                out.push(el);
            }
        }
    }
    out.sort_by_key(tile_rank);
    out
}

/// The wide bar's visual position of a tile, RELATIVE to the others — must
/// mirror the visual order the CSS `order` rules in `fab.css` establish
/// (account, then repo, then share, then end).
fn tile_rank(tile: &Element) -> u8 {
    let cl = tile.class_list();
    if cl.contains("fab__tele--account") {
        0
    } else if cl.contains("fab__tele--repo") {
        1
    } else if cl.contains("fab__tele--share") {
        2
    } else {
        3
    }
}

/// Measure each tile's natural width by momentarily unclamping it (max-width
/// none, overflow visible, no negative margin), reading the box, then
/// restoring the inline styles. Mirrors the wireframe's `measure()`.
fn measure_tile_widths(tiles: &[Element]) -> Vec<f64> {
    let mut widths = Vec::with_capacity(tiles.len());
    for tile in tiles {
        let style = tile.unchecked_ref::<HtmlElement>().style();
        let saved_mw = style.get_property_value("max-width").unwrap_or_default();
        let saved_ov = style.get_property_value("overflow").unwrap_or_default();
        let saved_ml = style.get_property_value("margin-left").unwrap_or_default();
        let _ = style.set_property("max-width", "none");
        let _ = style.set_property("overflow", "visible");
        let _ = style.set_property("margin-left", "0px");
        let w = tile.get_bounding_client_rect().width().ceil() + 1.0;
        // Restore (empty string removes the inline prop).
        let _ = style.set_property("max-width", &saved_mw);
        let _ = style.set_property("overflow", &saved_ov);
        let _ = style.set_property("margin-left", &saved_ml);
        widths.push(w);
    }
    widths
}

/// Schedule the `fab--settled` class after the telescope finishes expanding, so
/// each tile's `max-width` unclamps and the content can reflow past its
/// measured width. Stashes the timer id so a re-toggle (or disconnect) cancels it.
fn schedule_settle(element: &HtmlElement, fab: &Element, count: usize) {
    let fab_for_settle = fab.clone();
    let el_for_settle = element.clone();
    let settle_once = Closure::<dyn Fn()>::new(move || {
        fab_for_settle.class_list().add_1("fab--settled").ok();
        // Unclamp shown tiles: drop the inline `max-width` pinned during the
        // expand animation so each tile now sizes to its content. Inline styles
        // beat the stylesheet's `max-width: none`, so the clamp must be lifted
        // here in JS — otherwise content that grows AFTER the expand (a minted
        // invite link, a longer edited name) overflows its measured box and,
        // with the tile's `justify-content: flex-end`, spills leftward over the
        // neighbouring segment instead of widening the bar.
        for tile in telescope_tiles(&fab_for_settle) {
            if tile.class_list().contains("fab__tele--hidden") {
                continue;
            }
            let style = tile.unchecked_ref::<HtmlElement>().style();
            let _ = style.set_property("max-width", "none");
        }
        // Content settling is the moment the bar reaches its true width —
        // the one growth path (invite link, long rename) a resize never
        // sees. Re-check the fit here.
        update_compact_mode(&el_for_settle);
    });
    let settle_fn = settle_once
        .as_ref()
        .unchecked_ref::<js_sys::Function>()
        .clone();
    settle_once.forget();
    let id = set_timeout(&settle_fn, telescope_settle_ms(count) as i32);
    element.dataset().set("settleTimer", &id.to_string()).ok();
}

/// Re-evaluate compact mode from the WOULD-BE expanded bar width — the same
/// input whichever mode we are in, so the threshold cannot flap. Called on
/// connect, on guest-window resize, and when the telescope settles (content
/// like a minted invite link can widen the bar without a resize).
fn update_compact_mode(element: &HtmlElement) {
    let Some(fab) = element.query_selector(".fab").ok().flatten() else {
        return;
    };
    let compact = is_compact(expanded_bar_width(&fab), viewport_width());
    if fab.class_list().contains("fab--compact") == compact {
        return;
    }
    // Crossing modes resets transient UI state: menus close (their anchors
    // change shape entirely) and the telescope re-opens expanded with no
    // stale per-tile clamps — a wide-mode collapse leaves inline
    // `max-width: 0` on every tile, which would zero out the compact pages.
    close_menus(element);
    fab.class_list().remove_1("fab--collapsed").ok();
    fab.class_list().add_1("fab--settled").ok();
    for tile in telescope_tiles(&fab) {
        let style = tile.unchecked_ref::<HtmlElement>().style();
        let _ = style.remove_property("max-width");
        let _ = style.remove_property("margin-left");
        let _ = style.remove_property("transition-delay");
        tile.class_list().remove_1("fab__tele--hidden").ok();
    }
    fab.class_list()
        .toggle_with_force("fab--compact", compact)
        .ok();
    // A fresh mode starts the pager at page 1 with a forward arrow.
    sync_pager_arrow(element);
}

/// The bar's expanded width. Measured directly when expanded; a compact bar
/// is momentarily unclamped within one task — no paint can happen before the
/// classes are restored — the same trick `menu_natural_width` uses on closed
/// menus. Known gap, accepted: a WIDE bar collapsed to its circle
/// under-reports here (its tiles hold inline `max-width: 0` that no class
/// removal lifts), so shrinking the viewport while collapsed-wide defers the
/// flip to compact until the next expand's settle pass re-evaluates — and a
/// collapsed circle always fits anyway.
fn expanded_bar_width(fab: &Element) -> f64 {
    let cl = fab.class_list();
    let was_compact = cl.contains("fab--compact");
    if was_compact {
        cl.remove_1("fab--compact").ok();
    }
    let width = fab.get_bounding_client_rect().width();
    if was_compact {
        cl.add_1("fab--compact").ok();
    }
    width
}

/// Re-evaluate compact mode whenever the guest window resizes. The overlay
/// iframe is pinned full-viewport, so its window size IS the app viewport.
fn attach_resize(element: &HtmlElement) {
    let el = element.clone();
    let on_resize = Closure::<dyn FnMut()>::new(move || update_compact_mode(&el));
    if let Some(win) = window() {
        let target: &web_sys::EventTarget = win.unchecked_ref();
        let _ =
            target.add_event_listener_with_callback("resize", on_resize.as_ref().unchecked_ref());
    }
    on_resize.forget();
}

/// A swipe on the compact strip moves the segments out from under their
/// anchored dropdowns — dismiss them rather than drag them along.
fn attach_strip_scroll(element: &HtmlElement) {
    let Some(strip) = element.query_selector(".fab__strip").ok().flatten() else {
        return;
    };
    let el = element.clone();
    let on_scroll = Closure::<dyn FnMut()>::new(move || {
        close_menus(&el);
        sync_pager_arrow(&el);
    });
    let target: &web_sys::EventTarget = strip.unchecked_ref();
    let _ = target.add_event_listener_with_callback("scroll", on_scroll.as_ref().unchecked_ref());
    on_scroll.forget();
}

/// Point the pager arrow at what its next tap DOES: forward mid-strip,
/// back (`fab__more--back`, a 180° glyph flip) once the strip rests at its
/// end and the next tap wraps to the start. Driven from the strip's scroll
/// events and from mode changes, so swipes and taps both keep it honest.
fn sync_pager_arrow(element: &HtmlElement) {
    let Some(strip) = element.query_selector(".fab__strip").ok().flatten() else {
        return;
    };
    let Some(more) = element.query_selector(".fab__more").ok().flatten() else {
        return;
    };
    let at_end = strip_at_end(
        strip.scroll_left() as f64,
        strip.client_width() as f64,
        strip.scroll_width() as f64,
    );
    more.class_list()
        .toggle_with_force("fab__more--back", at_end)
        .ok();
}

/// Attach pointer event listeners for free drag-and-drop. The element moves
/// itself (its own `position: fixed` `left`/`top`); there is no iframe to relay
/// to.
///
/// `pointerdown` stays on the element (only a press starting on the circle cap
/// should arm a drag), but `pointermove`, `pointerup`, and `pointercancel` are
/// attached to the guest `window` instead: a fast flick can outrun the element
/// before its first `pointermove` fires (capture is only taken once the press
/// passes the drag threshold), so element-scoped listeners can lose the
/// pointer mid-drag and never see the release, leaving `fabPressing` stuck set
/// so a later hover resumes a phantom drag. The FAB's overlay iframe is pinned
/// full-viewport while dragging, so the window sees every pointer event;
/// captured events (post-threshold) still bubble there too. `on_move` also
/// carries a stale-press guard: if a move event arrives with no buttons held,
/// the release was already lost, so the drag is finished right there instead
/// of waiting for a `pointerup` that will never come.
fn attach_drag(element: &HtmlElement) {
    let el_down = element.clone();
    let on_down = Closure::<dyn FnMut(PointerEvent)>::new(move |e: PointerEvent| {
        // Only the primary button drags, and only from the CIRCLE cap — that is
        // the sole draggable handle. A press anywhere else on the bar (a
        // segment, an editable, a menu) is left entirely to native click.
        if e.button() != 0 {
            return;
        }
        let Some(cap) = e
            .target()
            .and_then(|t| t.dyn_into::<Element>().ok())
            .and_then(|el| el.closest(".fab__cap-l").ok().flatten())
        else {
            return;
        };
        // TOUCH presses capture IMMEDIATELY, and on the CAP (not the host):
        // a fast flick outruns even the window listeners' first delivery on
        // some mobile browsers, and deferred capture is the desktop
        // compromise that lets a stationary mouse press click — a touch tap
        // still clicks with capture held, because capture retargets pointer
        // events to the cap, which is exactly where the tap's click routes
        // anyway. Capturing on the host instead would retarget the click to
        // the host and break tap-to-toggle (`attach_gestures` walks
        // `closest(".fab__cap-l")` from the click target).
        if e.pointer_type() == "touch" {
            el_down.dataset().set("fabTouch", "1").ok();
            cap.set_pointer_capture(e.pointer_id()).ok();
        } else {
            el_down.dataset().delete("fabTouch");
        }
        // DELTA-based drag: remember the pointer's start AND the element's start
        // `left`/`top`, then translate by the pointer delta. No grab-offset or
        // rect math — the element moves 1:1 with the cursor and drops exactly
        // where released. We do NOT capture or `preventDefault` here so a plain
        // press still fires native click/dblclick; capture is taken in
        // `pointermove` only once the press passes the drag threshold.
        let rect = el_down.get_bounding_client_rect();
        el_down
            .dataset()
            .set("fabStartLeft", &rect.left().to_string())
            .ok();
        el_down
            .dataset()
            .set("fabStartTop", &rect.top().to_string())
            .ok();
        el_down
            .dataset()
            .set("fabDownX", &(e.client_x() as f64).to_string())
            .ok();
        el_down
            .dataset()
            .set("fabDownY", &(e.client_y() as f64).to_string())
            .ok();
        el_down.dataset().set("fabPressing", "1").ok();
        el_down.dataset().delete("fabMoved");
    });

    let el_move = element.clone();
    let on_move = Closure::<dyn FnMut(PointerEvent)>::new(move |e: PointerEvent| {
        if el_move.dataset().get("fabPressing").is_none() {
            return;
        }
        // A press with NO button still held means the pointerup was lost
        // (fast flick released outside the element before capture was taken):
        // finish the drag here so a later hover can't resume a phantom press.
        if e.buttons() == 0 {
            finish_drag(&el_move, e.pointer_id());
            return;
        }
        let dx = e.client_x() as f64 - read_data_f64(&el_move, "fabDownX");
        let dy = e.client_y() as f64 - read_data_f64(&el_move, "fabDownY");
        // Promote to a DRAG once past the dead zone; take capture only then, so a
        // stationary press stays a plain native click.
        if el_move.dataset().get("fabMoved").is_none() {
            let touch = el_move.dataset().get("fabTouch").is_some();
            let threshold = if touch {
                TOUCH_DRAG_THRESHOLD_PX
            } else {
                DRAG_THRESHOLD_PX
            };
            if dx.hypot(dy) < threshold {
                return;
            }
            el_move.dataset().set("fabMoved", "1").ok();
            // A touch press already holds capture on the cap (see
            // `on_down`); re-capturing on the host would retarget the
            // post-drag click mid-gesture.
            if !touch {
                el_move.set_pointer_capture(e.pointer_id()).ok();
            }
            if let Some(fab) = el_move.query_selector(".fab").ok().flatten() {
                fab.class_list().add_1("dragging").ok();
            }
            // A drag can't support an open menu (see `close_menus`): the
            // dock classes it's about to drop are the menu's vertical anchor.
            close_menus(&el_move);
            // Drop the dock class so the stylesheet's `.fab-dock-*` position
            // (which pins `bottom`/`top`) stops fighting the inline `left`/`top`
            // that now tracks the pointer 1:1.
            let cl = el_move.class_list();
            for c in DOCK_CLASSES {
                cl.remove_1(c).ok();
            }
            // The dock classes just vanished — resync the mirror from the
            // live handle immediately so it doesn't flash upright for one frame.
            apply_mirror_from_handle(&el_move);
        }
        e.prevent_default();
        let left = read_data_f64(&el_move, "fabStartLeft") + dx;
        let top = read_data_f64(&el_move, "fabStartTop") + dy;
        track_position(&el_move, left, top);
        apply_mirror_from_handle(&el_move);
    });

    let el_up = element.clone();
    let on_up = Closure::<dyn FnMut(PointerEvent)>::new(move |e: PointerEvent| {
        if el_up.dataset().get("fabPressing").is_none() {
            return;
        }
        finish_drag(&el_up, e.pointer_id());
    });

    let el_cancel = element.clone();
    let on_cancel = Closure::<dyn FnMut(PointerEvent)>::new(move |e: PointerEvent| {
        if el_cancel.dataset().get("fabPressing").is_none() {
            return;
        }
        finish_drag(&el_cancel, e.pointer_id());
    });

    let target: &web_sys::EventTarget = element.unchecked_ref();
    target
        .add_event_listener_with_callback("pointerdown", on_down.as_ref().unchecked_ref())
        .ok();
    // WINDOW-scoped move/up/cancel: a fast flick outruns the element before
    // its first pointermove fires (capture is only taken past the drag
    // threshold), so element-scoped listeners lose the pointer mid-drag and
    // never see the release. The overlay iframe is pinned full-viewport while
    // dragging, so the window sees every event. Captured events (post
    // threshold) still bubble here.
    if let Some(win) = window() {
        let wtarget: &web_sys::EventTarget = win.unchecked_ref();
        for (name, cb) in [
            ("pointermove", on_move.as_ref()),
            ("pointerup", on_up.as_ref()),
            ("pointercancel", on_cancel.as_ref()),
        ] {
            wtarget
                .add_event_listener_with_callback(name, cb.unchecked_ref())
                .ok();
        }
    }
    on_down.forget();
    on_move.forget();
    on_up.forget();
    on_cancel.forget();
}

/// Finish a press: clear the press flags and — if the press had been promoted
/// to a drag — release capture, drop the dragging class, and snap/persist the
/// dock nearest the HANDLE'S CENTER (falling back to the bar-rect center if
/// the handle is missing). The handle is what you drag, and it is the anchor
/// the mirror preview keys on (`apply_mirror_from_handle`), so the snap
/// always agrees with the live preview the bar has been showing throughout
/// the drag. Shared by `pointerup`, `pointercancel`, and the stale-press
/// guard in `pointermove`.
fn finish_drag(el: &HtmlElement, pointer_id: i32) {
    el.dataset().delete("fabPressing");
    let touch = el.dataset().get("fabTouch").is_some();
    el.dataset().delete("fabTouch");
    let moved = el.dataset().get("fabMoved").is_some();
    if !moved {
        return;
    }
    if touch {
        // Touch capture lives on the cap (see `attach_drag`). Explicit
        // release is belt-and-braces — pointerup implicitly releases — but
        // pointercancel paths keep it honest.
        if let Some(cap) = el.query_selector(".fab__cap-l").ok().flatten() {
            cap.release_pointer_capture(pointer_id).ok();
        }
    } else {
        el.release_pointer_capture(pointer_id).ok();
    }
    if let Some(fab) = el.query_selector(".fab").ok().flatten() {
        fab.class_list().remove_1("dragging").ok();
    }
    let rect = el.get_bounding_client_rect();
    let (center_x, center_y) = handle_center(el).unwrap_or((
        rect.left() + rect.width() / 2.0,
        rect.top() + rect.height() / 2.0,
    ));
    let dock = nearest_dock(center_x, center_y, viewport_width(), viewport_height());
    apply_dock(el, dock);
    persist_dock(dock);
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
fn read_data_f64(el: &HtmlElement, key: &str) -> f64 {
    el.dataset()
        .get(key)
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(0.0)
}

/// Track the FAB at `(left, top)` (viewport top-left) with plain `left`/`top`
/// during a drag — no corner anchoring, so it follows the cursor 1:1 without
/// jumping as it crosses the viewport midlines. Clamped so the bar can never
/// leave the viewport, whatever the pointer does. (The mirror-flip
/// compensation in `apply_mirror_from_handle` writes `left` directly and is
/// not clamped — the very next pointer move re-clamps, so any excursion
/// lasts one frame at most.)
fn track_position(el: &HtmlElement, left: f64, top: f64) {
    let rect = el.get_bounding_client_rect();
    let (left, top) = clamp_position(
        left,
        top,
        rect.width(),
        rect.height(),
        viewport_width(),
        viewport_height(),
    );
    let style = el.style();
    let _ = style.remove_property("right");
    let _ = style.remove_property("bottom");
    let _ = style.set_property("left", &format!("{}px", left));
    let _ = style.set_property("top", &format!("{}px", top));
}

/// Dock the FAB by swapping its `fab-dock-*` classes and clearing any drag-time
/// inline offsets, so the view stylesheet's `.fab-dock-*` rules own the resting
/// pixel position (and the submenu open-direction), and sync the `fab-mirror`
/// class from the dock. Used at drop and on restore. Anchoring by class — not
/// a fixed pixel offset — keeps the FAB pinned to its corner when the
/// viewport resizes.
fn apply_dock(el: &HtmlElement, dock: Dock) {
    let style = el.style();
    let _ = style.remove_property("left");
    let _ = style.remove_property("top");
    let _ = style.remove_property("right");
    let _ = style.remove_property("bottom");
    let cl = el.class_list();
    for c in DOCK_CLASSES {
        cl.remove_1(c).ok();
    }
    for c in dock.css_classes() {
        cl.add_1(c).ok();
    }
    // Sync the mirror from the dock, not the rect — at rest the dock IS the
    // truth (a drag drives it from the live handle instead; see
    // `apply_mirror_from_handle`).
    cl.toggle_with_force(MIRROR_CLASS, dock.css_classes()[1] == "fab-dock-right")
        .ok();
}

/// Persist `dock` by calling `window.tonk.transact(request)`. The request is the
/// `TransactRequest` JSON produced by `dock_claim_json`.
fn persist_dock(dock: Dock) {
    transact(&dock_claim_json(dock));
}

/// On connect, query the persisted FAB dock from `window.tonk.query(...)` and
/// apply its class. Falls back to the default (bottom-right) dock if none is stored.
fn restore_position(this: &HtmlElement) {
    // Position at the default dock immediately so the FAB is placed on first
    // paint; the async query below swaps in the persisted dock if one exists.
    apply_dock(this, Dock::BottomRight);

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

    let json_str = match serde_json::to_string(&query_body) {
        Ok(s) => s,
        Err(_) => return,
    };

    let Some(win) = window() else {
        return;
    };

    let tonk = match Reflect::get(&win, &"tonk".into())
        .ok()
        .and_then(|v| v.dyn_into::<Object>().ok())
    {
        Some(t) => t,
        None => {
            default_position(this);
            return;
        }
    };

    let query_fn = match Reflect::get(&tonk, &"query".into())
        .ok()
        .and_then(|v| v.dyn_into::<Function>().ok())
    {
        Some(f) => f,
        None => {
            default_position(this);
            return;
        }
    };

    let js_body = match js_sys::JSON::parse(&json_str).ok() {
        Some(v) => v,
        None => {
            default_position(this);
            return;
        }
    };

    let result = match query_fn.call1(&tonk, &js_body).ok() {
        Some(v) => v,
        None => {
            default_position(this);
            return;
        }
    };

    // `window.tonk.query` returns a Promise<Conclusion[]>. Await it and apply
    // the persisted dock if present.
    if let Ok(promise) = result.dyn_into::<Promise>() {
        let this = this.clone();
        spawn_local(async move {
            match wasm_bindgen_futures::JsFuture::from(promise).await {
                Ok(rows) => match read_dock_from_rows(&rows) {
                    Some(dock) => apply_dock(&this, dock),
                    None => default_position(&this),
                },
                Err(_) => default_position(&this),
            }
        });
    } else {
        default_position(this);
    }
}

/// Extract the persisted dock from a `Conclusion[]` value returned by
/// `window.tonk.query(...)`. Decodes the JS value to JSON and delegates the
/// row-shape parsing to [`dock_from_conclusions`], which is unit-tested
/// against the `{ this, fields: { dock } }` conclusion shape.
fn read_dock_from_rows(rows: &JsValue) -> Option<Dock> {
    let json = js_sys::JSON::stringify(rows).ok()?.as_string()?;
    let value: serde_json::Value = serde_json::from_str(&json).ok()?;
    dock_from_conclusions(&value)
}

/// Apply the default dock (bottom-right) to the element.
fn default_position(this: &HtmlElement) {
    apply_dock(this, Dock::BottomRight);
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

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod tests {
    use super::*;
    use wasm_bindgen_test::{wasm_bindgen_test, wasm_bindgen_test_configure};
    use web_sys::window;

    wasm_bindgen_test_configure!(run_in_browser);

    /// A FAB whose two dropdown segments are both open, plus the click-away
    /// curtain — the shape the profile view renders.
    fn open_fab() -> HtmlElement {
        let document = window().expect("window").document().expect("document");
        let fab: HtmlElement = document
            .create_element("div")
            .expect("create fab")
            .unchecked_into();
        fab.set_class_name("fab");
        fab.set_inner_html(
            r#"<div class="fab__scrim"></div>
               <span class="fab__seg fab__repo is-open"></span>
               <span class="fab__seg fab__share is-open"></span>"#,
        );
        fab
    }

    fn is_open(fab: &HtmlElement, selector: &str) -> bool {
        fab.query_selector(selector)
            .ok()
            .flatten()
            .map(|seg| seg.class_list().contains("is-open"))
            .unwrap_or(false)
    }

    /// The shape of the rule that made `hidden` inert on the bar's join
    /// action: Web Awesome's `@layer wa-native` skins native form controls,
    /// and its nested `&:not(input[type="file"])` desugars to a selector
    /// matching every `<button>`. An author `display` declaration outranks
    /// the UA's `[hidden] { display: none }` whatever its specificity or
    /// layer, so the attribute stops hiding anything.
    ///
    /// Reproduced rather than loaded: the real sheet is a hashed build
    /// artifact, and the invariant under test is "a layered author `display`
    /// rule on `button` must not defeat `hidden`" — true of whatever
    /// selector Web Awesome ships next.
    const WEB_AWESOME_NATIVE_BUTTON: &str = r#"
@layer wa-native {
  button, input[type="button"], input[type="reset"], input[type="submit"],
  input[type="file"], a.wa-button {
    &:not(input[type="file"]), &::file-selector-button {
      display: inline-flex;
    }
  }
}"#;

    /// Mount the real bar under the stylesheets a sealed guest actually
    /// applies, in the order it applies them: Web Awesome, then the app
    /// stylesheet (the guest concatenates those two), then `fab.css`, which
    /// `ensure_stylesheet` appends to `<head>` later. That ordering is the
    /// hazard — `fab.css` losing a same-specificity tie by winning it — so
    /// the fixture pins it instead of inheriting whatever order earlier
    /// tests left behind.
    ///
    /// Returns the host and the sheets, both of which the caller removes.
    fn guest_styled_fab() -> (HtmlElement, Vec<Element>) {
        let document = window().expect("window").document().expect("document");
        let head = document.head().expect("head");
        let mut sheets = Vec::new();
        for css in [
            WEB_AWESOME_NATIVE_BUTTON,
            include_str!("../../tonk-ui/styles.css"),
            include_str!("fab.css"),
        ] {
            let style = document.create_element("style").expect("create style");
            style.set_text_content(Some(css));
            head.append_child(&style).expect("append style");
            sheets.push(style);
        }

        let host: HtmlElement = document
            .create_element("tonk-fab")
            .expect("create host")
            .unchecked_into();
        host.set_inner_html(&crate::markup::fab_html("did:key:zJoinFixture"));
        document
            .body()
            .expect("body")
            .append_child(&host)
            .expect("mount");
        (host, sheets)
    }

    fn teardown(host: &HtmlElement, sheets: &[Element]) {
        host.remove();
        for sheet in sheets {
            sheet.remove();
        }
    }

    fn computed(element: &Element, property: &str) -> String {
        window()
            .expect("window")
            .get_computed_style(element)
            .expect("computed style")
            .expect("style declaration")
            .get_property_value(property)
            .expect("property")
    }

    /// The join action is guest-only: `attach_membership` unhides it just for
    /// a replica the worker reports as a guest. The `hidden` attribute it
    /// ships with has to actually hide it, or every owner sees an invitation
    /// to join a spot they created.
    #[dialog_common::test]
    fn it_keeps_the_join_action_hidden_under_a_layered_button_display_rule() {
        let (host, sheets) = guest_styled_fab();
        let join = host
            .query_selector(".fab__join")
            .expect("query")
            .expect("join action");
        assert!(
            join.has_attribute("hidden"),
            "the bar must author the join action hidden",
        );

        let display = computed(&join, "display");
        teardown(&host, &sheets);

        assert_eq!(
            display, "none",
            "`hidden` must survive an author `display` rule on `button`",
        );
    }

    /// Shown, it has to read as bar copy beside the member name — the same
    /// treatment `.fab__share-trigger` gets. Unstyled it inherits Web
    /// Awesome's native-button skin, which lands as an opaque chip with its
    /// own border and box height, breaking the pill.
    #[dialog_common::test]
    fn it_styles_the_join_action_as_bar_copy_rather_than_a_native_button() {
        let (host, sheets) = guest_styled_fab();
        let join = host
            .query_selector(".fab__join")
            .expect("query")
            .expect("join action");
        join.remove_attribute("hidden").expect("unhide");

        let display = computed(&join, "display");
        let background = computed(&join, "background-color");
        let border = computed(&join, "border-top-width");
        teardown(&host, &sheets);

        // Not a keyword assertion: the action is a flex item of
        // `.fab__account`, and flex items blockify, so an authored
        // `inline-flex` computes as `flex`. What matters is that it lays out
        // at all once the attribute is gone.
        assert_ne!(display, "none", "shown, it lays out");
        assert_eq!(
            background, "rgba(0, 0, 0, 0)",
            "the segment supplies the surface; the action is transparent",
        );
        assert_eq!(border, "0px", "bar copy carries no button border");
    }

    /// A guest's share attempt is refused by the worker, so the bar can say so
    /// before the click rather than after a round trip. Same membership answer
    /// that reveals the join action, so it costs no extra request.
    ///
    /// Advisory only — the control stays clickable, because the refusal is
    /// what carries the reason and the offer to join.
    #[dialog_common::test]
    fn it_marks_share_unavailable_for_a_guest_replica() {
        let document = window().expect("window").document().expect("document");
        let host: HtmlElement = document
            .create_element("tonk-fab")
            .expect("create host")
            .unchecked_into();
        host.set_inner_html(&crate::markup::fab_html("did:key:zGuest"));

        apply_membership(&host, "guest");
        let guest_marked = host.has_attribute(SHARE_UNAVAILABLE_ATTR);
        let join_shown = host
            .query_selector(".fab__join")
            .expect("query")
            .map(|join| !join.has_attribute("hidden"))
            .unwrap_or(false);

        apply_membership(&host, "durable");
        let member_marked = host.has_attribute(SHARE_UNAVAILABLE_ATTR);
        let join_hidden = host
            .query_selector(".fab__join")
            .expect("query")
            .map(|join| join.has_attribute("hidden"))
            .unwrap_or(false);

        assert!(guest_marked, "a guest's bar marks share unavailable");
        assert!(join_shown, "and reveals the join action");
        assert!(!member_marked, "a durable member's share is available");
        assert!(join_hidden, "and carries no join action");
    }

    /// Promotion is a network round trip. Without a disabled state the click
    /// reads as dead and a second click posts a second promotion.
    #[dialog_common::test]
    fn it_disables_the_join_action_while_the_promotion_is_in_flight() {
        let document = window().expect("window").document().expect("document");
        let host: HtmlElement = document
            .create_element("tonk-fab")
            .expect("create host")
            .unchecked_into();
        host.set_attribute("space", "did:key:zJoinClick")
            .expect("space");
        host.set_inner_html(&crate::markup::fab_html("did:key:zJoinClick"));
        document
            .body()
            .expect("body")
            .append_child(&host)
            .expect("mount");
        attach_membership(&host);

        let join: HtmlElement = host
            .query_selector(".fab__join")
            .expect("query")
            .expect("join action")
            .unchecked_into();
        join.click();
        let disabled = join.has_attribute("disabled");
        host.remove();

        assert!(
            disabled,
            "the click must disable the action for the round trip",
        );
    }

    /// A `<tonk-fab>` holding the bar, the way `markup::fab_html` authors it —
    /// the scrim as a sibling of `.fab`, cap first, then the two dropdown
    /// segments.
    fn fab_host() -> HtmlElement {
        let document = window().expect("window").document().expect("document");
        let host: HtmlElement = document
            .create_element("tonk-fab")
            .expect("create host")
            .unchecked_into();
        host.set_inner_html(
            r#"<div class="fab__scrim"></div>
               <div class="fab">
                 <span class="fab__seg fab__cap-l"></span>
                 <div class="fab__strip">
                   <div class="fab__page fab__page--main">
                     <div class="fab__tele fab__tele--repo"><span class="fab__seg fab__repo"></span></div>
                   </div>
                   <div class="fab__page fab__page--more">
                     <div class="fab__tele fab__tele--share"><span class="fab__seg fab__share"></span></div>
                     <div class="fab__tele fab__tele--account"><span class="fab__seg fab__account"></span></div>
                   </div>
                 </div>
                 <div class="fab__tele fab__tele--end">
                   <span class="fab__seg fab__cap-r fab__end" aria-hidden="true"></span>
                   <button type="button" class="fab__seg fab__cap-r fab__more"></button>
                 </div>
               </div>"#,
        );
        host
    }

    #[wasm_bindgen_test]
    fn it_dismisses_the_menus_when_the_curtain_is_clicked() {
        // End-to-end through the real gesture handler: a click on the curtain
        // must reach `close_menus`. Testing `close_menus` alone would still pass
        // if the handler's curtain branch were deleted.
        let document = window().expect("window").document().expect("document");
        let host = fab_host();
        attach_gestures(&host);
        // The handler walks up from `event.target`, so the element has to be in
        // the document for the click to dispatch and bubble.
        document
            .body()
            .expect("body")
            .append_child(&host)
            .expect("mount");
        for selector in [".fab__repo", ".fab__share"] {
            host.query_selector(selector)
                .ok()
                .flatten()
                .expect("segment")
                .class_list()
                .add_1("is-open")
                .expect("open");
        }

        let scrim = host
            .query_selector(".fab__scrim")
            .ok()
            .flatten()
            .expect("curtain");
        // Must bubble: the listener lives on <tonk-fab>, not on the curtain.
        let init = web_sys::MouseEventInit::new();
        init.set_bubbles(true);
        let click = web_sys::MouseEvent::new_with_mouse_event_init_dict("click", &init)
            .expect("click event");
        scrim.dispatch_event(&click).expect("dispatch");

        assert!(
            !is_open(&host, ".fab__repo") && !is_open(&host, ".fab__share"),
            "clicking the curtain must dismiss both dropdowns",
        );
        host.remove();
    }

    #[wasm_bindgen_test]
    fn it_closes_both_dropdowns_at_once() {
        // The curtain dismisses EVERY menu, not just the one that opened it —
        // a click outside is a click outside for both.
        let fab = open_fab();
        assert!(is_open(&fab, ".fab__repo") && is_open(&fab, ".fab__share"));

        close_menus(&fab);

        assert!(
            !is_open(&fab, ".fab__repo"),
            "the repo switcher should close"
        );
        assert!(
            !is_open(&fab, ".fab__share"),
            "the share roster should close"
        );
    }

    #[wasm_bindgen_test]
    fn it_is_a_no_op_when_nothing_is_open() {
        // The curtain has no hit area unless a menu is open, but closing an
        // already-closed bar must not throw or disturb the segments.
        let fab = open_fab();
        close_menus(&fab);
        close_menus(&fab);
        assert!(!is_open(&fab, ".fab__repo"));
        assert!(!is_open(&fab, ".fab__share"));
    }

    #[wasm_bindgen_test]
    fn it_compacts_when_the_expanded_bar_cannot_fit() {
        let document = window().expect("window").document().expect("document");
        // `expanded_bar_width` only measures correctly because it removes
        // `fab--compact` before reading the rect (then restores it) — a
        // naive measurement while the class is still applied would read the
        // clamped-down size instead of the bar's true would-be-expanded
        // size. The fixture otherwise loads no stylesheet, so nothing here
        // would catch a regression that measured before unclamping. Inject
        // the one rule that actually exercises that: a real `.fab--compact`
        // clamp, matching fab.css's compact-mode intent, that would corrupt
        // a naive measurement if `expanded_bar_width` didn't remove the
        // class first.
        const CLAMP_STYLE_ID: &str = "it-compacts-when-the-expanded-bar-cannot-fit-clamp";
        let clamp_style = document.create_element("style").expect("create style");
        let _ = clamp_style.set_attribute("id", CLAMP_STYLE_ID);
        clamp_style.set_text_content(Some(
            ".fab--compact { max-inline-size: 50px; overflow: hidden; }",
        ));
        if let Some(head) = document.head() {
            head.append_child(&clamp_style).expect("mount clamp style");
        }
        let host = fab_host();
        document
            .body()
            .expect("body")
            .append_child(&host)
            .expect("mount");
        let fab = host
            .query_selector(".fab")
            .ok()
            .flatten()
            .expect("bar")
            .unchecked_into::<HtmlElement>();
        // The fixture loads no stylesheet, so every element here is plain
        // block/inline layout. `.fab`'s own rect would otherwise just fill
        // its container regardless of content (a block `<div>`'s `width:
        // auto` fills its containing block; it does not shrink-to-fit or
        // grow with an overflowing child) — insensitive to the oversized
        // segment below and unable to exercise `expanded_bar_width` at all.
        // Reproduce the three fab.css facts the measurement actually
        // depends on: `.fab` itself is `inline-flex` (shrink-to-fit, so an
        // unshrinkable child can push its rect past the viewport);
        // `.fab__strip`/`.fab__page` are `display: contents` (they
        // disappear from the box tree, so their `.fab__tele` descendants
        // become direct flex items of `.fab`, exactly as fab.css flattens
        // them); and `.fab__tele` is `flex: 0 0 auto` (no shrink, so an
        // oversized tile isn't compressed back down by the flex algorithm).
        let _ = fab.style().set_property("display", "inline-flex");
        for sel in [".fab__strip", ".fab__page"] {
            if let Ok(list) = host.query_selector_all(sel) {
                for i in 0..list.length() {
                    if let Some(node) = list.item(i) {
                        let _ = node
                            .unchecked_into::<HtmlElement>()
                            .style()
                            .set_property("display", "contents");
                    }
                }
            }
        }
        if let Ok(list) = host.query_selector_all(".fab__tele") {
            for i in 0..list.length() {
                if let Some(node) = list.item(i) {
                    let _ = node
                        .unchecked_into::<HtmlElement>()
                        .style()
                        .set_property("flex", "0 0 auto");
                }
            }
        }
        let wide = host
            .query_selector(".fab__repo")
            .ok()
            .flatten()
            .expect("repo segment")
            .unchecked_into::<HtmlElement>();
        // Per-property, not a `cssText` blob: `CSSStyleDeclaration.setProperty
        // ("cssText", …)` is a Blink/Gecko-only quirk that WebKit/Safari does
        // not honor (confirmed empirically — the style attribute never gets
        // created), which would silently leave this element at its default
        // zero size under Safari's local wasm-test route.
        let _ = wide.style().set_property("display", "inline-block");
        let _ = wide.style().set_property("width", "9999px");

        update_compact_mode(&host);
        assert!(
            fab.class_list().contains("fab--compact"),
            "a bar wider than any viewport must compact"
        );

        // The bar is still oversized and now genuinely carries
        // `fab--compact` (with the clamp rule above in effect). Re-evaluate
        // while clamped: a correct `expanded_bar_width` removes the class
        // before measuring, so it still sees the true oversized width and
        // stays compact. A regression that measured before removing the
        // class would read the clamped ~50px, decide the bar fits, and
        // wrongly drop the class here.
        update_compact_mode(&host);
        assert!(
            fab.class_list().contains("fab--compact"),
            "re-evaluating while clamped must measure the unclamped width"
        );

        let _ = wide.style().set_property("width", "10px");
        update_compact_mode(&host);
        assert!(
            !fab.class_list().contains("fab--compact"),
            "a bar that fits again must leave compact mode"
        );
        host.remove();
        clamp_style.remove();
    }

    fn bubbling_click() -> web_sys::MouseEvent {
        let init = web_sys::MouseEventInit::new();
        init.set_bubbles(true);
        web_sys::MouseEvent::new_with_mouse_event_init_dict("click", &init).expect("click event")
    }

    #[wasm_bindgen_test]
    fn it_opens_the_floating_dropdown_from_a_compact_segment_tap() {
        // Compact segment taps open the SAME dropdown as desktop — floated
        // over the bar by CSS (position: fixed escapes the strip's clip) —
        // not a separate mobile menu. Through the real gesture path.
        let document = window().expect("window").document().expect("document");
        let host = fab_host();
        attach_gestures(&host);
        document
            .body()
            .expect("body")
            .append_child(&host)
            .expect("mount");

        host.query_selector(".fab")
            .ok()
            .flatten()
            .expect("bar")
            .class_list()
            .add_1("fab--compact")
            .expect("compact");

        let repo = host
            .query_selector(".fab__repo")
            .ok()
            .flatten()
            .expect("repo segment");
        repo.dispatch_event(&bubbling_click()).expect("dispatch");

        assert!(
            is_open(&host, ".fab__repo"),
            "a compact segment tap must open its dropdown, same as desktop"
        );
        host.remove();
    }

    #[wasm_bindgen_test]
    fn it_collapses_the_compact_bar_with_a_dropdown_open() {
        // A collapse retracts everything: the strip clamps to nothing, so a
        // floating dropdown left standing would hover over a bare circle
        // with its scrim still armed. The cap click must dismiss it.
        let document = window().expect("window").document().expect("document");
        let host = fab_host();
        attach_gestures(&host);
        document
            .body()
            .expect("body")
            .append_child(&host)
            .expect("mount");

        let fab = host.query_selector(".fab").ok().flatten().expect("bar");
        fab.class_list().add_1("fab--compact").expect("compact");
        fab.class_list().add_1("fab--settled").expect("settled");
        host.query_selector(".fab__repo")
            .ok()
            .flatten()
            .expect("repo segment")
            .class_list()
            .add_1("is-open")
            .expect("open dropdown");

        let cap = host
            .query_selector(".fab__cap-l")
            .ok()
            .flatten()
            .expect("cap");
        // A plain bubbling click — not the alt-click pause gesture.
        cap.dispatch_event(&bubbling_click()).expect("dispatch");

        assert!(
            fab.class_list().contains("fab--collapsed"),
            "the cap click must collapse the compact bar"
        );
        assert!(
            !fab.class_list().contains("fab--settled"),
            "collapsing must drop fab--settled — its unclamp rule outranks \
             the collapse clamp and would keep the chevron's tile open"
        );
        assert!(
            !is_open(&host, ".fab__repo"),
            "collapsing must dismiss the open dropdown"
        );
        host.remove();
    }
}
