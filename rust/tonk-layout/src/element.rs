//! `<tonk-layout>` custom-element implementation — wasm-side.
//!
//! Opens three live SSE subscriptions (workspace by name, focus by
//! workspace entity, tiles unfiltered) and folds them into a
//! [`Layout`] snapshot. Listens for the seven effect [`CustomEvent`]s
//! bubbling up from its subtree and translates each into one atomic
//! `/evaluate` POST. Dispatches outbound `tonk-layout:changed` /
//! `tonk-layout:focus` / `tonk-layout:error` events.
//!
//! The element renders nothing into its own subtree. UIs that
//! present the workspace ship as `<tonk-display>` view documents
//! wrapping this element.

use std::cell::RefCell;
use std::rc::Rc;

use custom_elements::CustomElement;
use js_sys::{Function, Reflect};
use tonk_host::consumer::{self as host_consumer, Subscription as HostSubscription};
use tonk_host::error::{ErrorDetail, ErrorKind};
use tonk_host::{DepthAnnotator, install_depth_annotator};
use tonk_schema::conclusion::Conclusion;
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use wasm_bindgen::closure::Closure;
use wasm_bindgen_futures::spawn_local;
use web_sys::{CustomEvent, CustomEventInit, Element, HtmlElement, window};

use crate::model::{Layout, fold_universal};
use crate::resolve::{focus_query, tiles_query, workspace_query};
use crate::ulid;
use crate::writer::{self, Direction, WorkspaceTarget};

/// The seven CustomEvent names this element catches. Single source
/// of truth — listener attachment and detach iterate this list.
const EFFECT_NAMES: &[&str] = &[
    "tonk-layout/focus-tile",
    "tonk-layout/focus-prev",
    "tonk-layout/focus-next",
    "tonk-layout/open-tile",
    "tonk-layout/close-tile",
    "tonk-layout/reorder-tile",
    "tonk-layout/update-tile-content",
];

/// Internal lifecycle state shared across async closures.
struct Inner {
    /// Set true by `disconnected_callback` so any frame handler
    /// still running bails before mutating a host that's detached.
    disposed: bool,
    /// Bumped every time `start` kicks off a new lifecycle. Each
    /// spawned async chain captures the value at spawn time and
    /// bails on any step that runs after the generation has moved.
    generation: u64,
    /// Live subscriptions via the host. Dropping cancels via the
    /// host's registry.
    workspace_sub: Option<HostSubscription>,
    focus_sub: Option<HostSubscription>,
    tiles_sub: Option<HostSubscription>,
    /// True once children subscriptions have started for this
    /// generation. Prevents a burst of workspace frames from racing
    /// to spawn duplicate child opens.
    children_pending: bool,
    /// Latest frame of each subscription.
    workspace_frame: Vec<Conclusion>,
    focus_frame: Vec<Conclusion>,
    tiles_frame: Vec<Conclusion>,
    /// Latest folded layout. `None` either before the first workspace
    /// frame arrives or when no workspace matches `name`.
    layout: Option<Layout>,
    /// `layout.focus` from the previous refold. Compared against the
    /// new value so `tonk-layout:focus` only fires on actual changes.
    last_focus: Option<String>,
    /// Event listeners for the seven effect CustomEvents. Held so
    /// the Closures outlive their registration.
    effect_listeners: Vec<Closure<dyn FnMut(CustomEvent)>>,
    /// Depth annotator installed at `connected_callback`; dropped
    /// on disconnect to detach the listeners.
    depth_annotator: Option<DepthAnnotator>,
}

impl Inner {
    fn new() -> Self {
        Self {
            disposed: false,
            generation: 0,
            workspace_sub: None,
            focus_sub: None,
            tiles_sub: None,
            children_pending: false,
            workspace_frame: Vec::new(),
            focus_frame: Vec::new(),
            tiles_frame: Vec::new(),
            layout: None,
            last_focus: None,
            effect_listeners: Vec::new(),
            depth_annotator: None,
        }
    }

    fn abort_all(&mut self) {
        self.workspace_sub.take();
        self.focus_sub.take();
        self.tiles_sub.take();
    }
}

/// The custom element.
#[derive(Default)]
pub struct TonkLayout {
    inner: RefCell<Option<Rc<RefCell<Inner>>>>,
}

impl CustomElement for TonkLayout {
    fn shadow() -> bool {
        false
    }

    fn observed_attributes() -> &'static [&'static str] {
        &["workspace"]
    }

    fn inject_children(&mut self, _this: &HtmlElement) {}

    fn connected_callback(&mut self, this: &HtmlElement) {
        let host: Element = this.clone().into();
        let inner = Rc::new(RefCell::new(Inner::new()));
        inner.borrow_mut().depth_annotator = Some(install_depth_annotator(&host));
        install_method_delegates(&host, &inner);
        *self.inner.borrow_mut() = Some(inner.clone());
        start(&host, inner);
    }

    fn disconnected_callback(&mut self, this: &HtmlElement) {
        if let Some(inner) = self.inner.borrow_mut().take() {
            let host: Element = this.clone().into();
            detach_effect_listeners(&host, &inner);
            {
                let mut i = inner.borrow_mut();
                i.disposed = true;
                i.abort_all();
                i.depth_annotator.take();
            }
            host_consumer::dispatch_unsubscribe(&host);
        }
    }

    fn attribute_changed_callback(
        &mut self,
        this: &HtmlElement,
        _name: String,
        _old: Option<String>,
        _new: Option<String>,
    ) {
        let host: Element = this.clone().into();
        let Some(inner) = self.inner.borrow().clone() else {
            return;
        };
        detach_effect_listeners(&host, &inner);
        {
            let mut i = inner.borrow_mut();
            i.abort_all();
            i.children_pending = false;
            i.workspace_frame.clear();
            i.focus_frame.clear();
            i.tiles_frame.clear();
            i.layout = None;
            i.last_focus = None;
        }
        start(&host, inner);
    }
}

/// Public entry point — registers the element with the page.
pub fn register() {
    if already_registered() {
        return;
    }
    TonkLayout::define("tonk-layout");
    install_method_shims();
}

/// Install the `reset` / `update` / `error` method shims on the
/// `<tonk-layout>` prototype.
fn install_method_shims() {
    let Some(win) = window() else { return };
    let constructor = win.custom_elements().get("tonk-layout");
    if constructor.is_undefined() {
        return;
    }
    let Ok(proto) = Reflect::get(&constructor, &"prototype".into()) else {
        return;
    };
    let reset_fn = Function::new_with_args(
        "payload, opts",
        "if (typeof this.__tonkReset === 'function') this.__tonkReset(payload, opts);",
    );
    let update_fn = Function::new_with_args(
        "payload, opts",
        "if (typeof this.__tonkUpdate === 'function') this.__tonkUpdate(payload, opts);",
    );
    let error_fn = Function::new_with_args(
        "payload, opts",
        "if (typeof this.__tonkError === 'function') this.__tonkError(payload, opts);",
    );
    let _ = Reflect::set(&proto, &"reset".into(), &reset_fn);
    let _ = Reflect::set(&proto, &"update".into(), &update_fn);
    let _ = Reflect::set(&proto, &"error".into(), &error_fn);
}

/// Per-instance: write closures to `__tonkReset` / `__tonkUpdate`
/// / `__tonkError` on the element so the prototype shims can
/// invoke them.
fn install_method_delegates(host: &Element, inner: &Rc<RefCell<Inner>>) {
    let host_for_reset = host.clone();
    let inner_for_reset = inner.clone();
    let reset: Closure<dyn FnMut(JsValue, JsValue)> =
        Closure::wrap(Box::new(move |payload, opts| {
            on_reset(&host_for_reset, &inner_for_reset, payload, opts);
        }));
    let _ = Reflect::set(host, &"__tonkReset".into(), reset.as_ref());
    reset.forget();

    let update: Closure<dyn FnMut(JsValue, JsValue)> =
        Closure::wrap(Box::new(move |_payload, _opts| {
            // V1 SW emits only `reset`; delta semantics arrive
            // with DBSP integration.
        }));
    let _ = Reflect::set(host, &"__tonkUpdate".into(), update.as_ref());
    update.forget();

    let host_for_error = host.clone();
    let inner_for_error = inner.clone();
    let error: Closure<dyn FnMut(JsValue, JsValue)> =
        Closure::wrap(Box::new(move |payload, _opts| {
            let message = Reflect::get(&payload, &"message".into())
                .ok()
                .and_then(|v| v.as_string())
                .unwrap_or_else(|| format!("{payload:?}"));
            fail(
                &host_for_error,
                &inner_for_error,
                ErrorDetail::new(ErrorKind::Network, message),
            );
        }));
    let _ = Reflect::set(host, &"__tonkError".into(), error.as_ref());
    error.forget();
}

/// `reset(conclusions, { tag })` — dispatch on tag to the
/// per-stream handler. All three subscriptions deliver via this
/// one entry point.
fn on_reset(host: &Element, inner: &Rc<RefCell<Inner>>, payload: JsValue, opts: JsValue) {
    let tag = Reflect::get(&opts, &"tag".into())
        .ok()
        .and_then(|v| v.as_string());
    if !is_current_borrow(inner) {
        return;
    }
    let conclusions: Vec<Conclusion> = match serde_wasm_bindgen::from_value(payload) {
        Ok(v) => v,
        Err(e) => {
            fail(
                host,
                inner,
                ErrorDetail::new(ErrorKind::Parse, format!("reset payload: {e}")),
            );
            return;
        }
    };
    let generation = inner.borrow().generation;
    match tag.as_deref() {
        Some("workspace") => {
            handle_workspace_frame(host.clone(), inner.clone(), generation, conclusions)
        }
        Some("focus") => {
            inner.borrow_mut().focus_frame = conclusions;
            refold(host, inner);
        }
        Some("tiles") => {
            inner.borrow_mut().tiles_frame = conclusions;
            refold(host, inner);
        }
        _ => {}
    }
}

fn is_current_borrow(inner: &Rc<RefCell<Inner>>) -> bool {
    let i = inner.borrow();
    !i.disposed
}

fn already_registered() -> bool {
    let Some(win) = window() else {
        return false;
    };
    !win.custom_elements().get("tonk-layout").is_undefined()
}

/// Bump generation, attach effect listeners, kick off the workspace
/// subscription via the host.
fn start(host: &Element, inner: Rc<RefCell<Inner>>) {
    let _generation = {
        let mut i = inner.borrow_mut();
        i.generation = i.generation.wrapping_add(1);
        i.generation
    };
    attach_effect_listeners(host, &inner);

    if let Err(err) = open_workspace_subscription(host, &inner) {
        fail(host, &inner, err);
    }
}

/// Open the workspace subscription via the host. Frames arrive
/// at `on_reset` with tag `"workspace"`, which routes them to
/// `handle_workspace_frame` below. The host's annotators stamp
/// `space` and `branch` from ancestors as the subscribe event
/// bubbles up.
fn open_workspace_subscription(
    host: &Element,
    inner: &Rc<RefCell<Inner>>,
) -> Result<(), ErrorDetail> {
    let q = workspace_query(&workspace_name(host))
        .map_err(|e| ErrorDetail::new(ErrorKind::Parse, format!("workspace query: {e}")))?;
    let body = serde_wasm_bindgen::to_value(&q)
        .map_err(|e| ErrorDetail::new(ErrorKind::Parse, format!("workspace body: {e}")))?;
    let tag = JsValue::from_str("workspace");
    let sub = host_consumer::subscribe(host, &body, Some(&tag))?;
    inner.borrow_mut().workspace_sub = Some(sub);
    Ok(())
}

/// Workspace frame handler: store, lazily open focus + tiles
/// subscriptions on the first non-empty workspace frame, refold.
fn handle_workspace_frame(
    host: Element,
    inner: Rc<RefCell<Inner>>,
    _generation: u64,
    frame: Vec<Conclusion>,
) {
    let workspace_entity = frame.first().map(|c| c.this.clone());
    let should_open_children = {
        let mut i = inner.borrow_mut();
        i.workspace_frame = frame;
        workspace_entity.is_some() && !i.children_pending
    };
    if should_open_children && let Some(ws_entity) = workspace_entity {
        inner.borrow_mut().children_pending = true;
        if let Err(err) = open_children_subscriptions(&host, &inner, &ws_entity) {
            fail(&host, &inner, err);
        }
    }
    refold(&host, &inner);
}

/// Open focus + tiles subscriptions once the workspace entity URI
/// is known.
fn open_children_subscriptions(
    host: &Element,
    inner: &Rc<RefCell<Inner>>,
    workspace_entity: &str,
) -> Result<(), ErrorDetail> {
    let focus_q = focus_query(workspace_entity)
        .map_err(|e| ErrorDetail::new(ErrorKind::Parse, format!("focus query: {e}")))?;
    let focus_body = serde_wasm_bindgen::to_value(&focus_q)
        .map_err(|e| ErrorDetail::new(ErrorKind::Parse, format!("focus body: {e}")))?;
    let focus_tag = JsValue::from_str("focus");
    let focus_sub = host_consumer::subscribe(host, &focus_body, Some(&focus_tag))?;
    inner.borrow_mut().focus_sub = Some(focus_sub);

    let tiles_q = tiles_query()
        .map_err(|e| ErrorDetail::new(ErrorKind::Parse, format!("tiles query: {e}")))?;
    let tiles_body = serde_wasm_bindgen::to_value(&tiles_q)
        .map_err(|e| ErrorDetail::new(ErrorKind::Parse, format!("tiles body: {e}")))?;
    let tiles_tag = JsValue::from_str("tiles");
    let tiles_sub = host_consumer::subscribe(host, &tiles_body, Some(&tiles_tag))?;
    inner.borrow_mut().tiles_sub = Some(tiles_sub);
    Ok(())
}

/// Re-fold the three latest frames into a [`Layout`], dispatch
/// outbound events.
fn refold(host: &Element, inner: &Rc<RefCell<Inner>>) {
    let layout = {
        let i = inner.borrow();
        fold_universal(&i.workspace_frame, &i.focus_frame, &i.tiles_frame)
    };

    let new_focus = layout.as_ref().and_then(|l| l.focus.clone());
    let focus_changed = {
        let i = inner.borrow();
        new_focus != i.last_focus
    };
    {
        let mut i = inner.borrow_mut();
        i.last_focus = new_focus.clone();
        i.layout = layout.clone();
    }

    if focus_changed && let Some(focus) = new_focus.as_deref() {
        let detail = serde_wasm_bindgen::to_value(&serde_json::json!({ "tile": focus }))
            .unwrap_or(JsValue::NULL);
        dispatch(host, "tonk-layout:focus", Some(detail));
    }

    let changed_detail = match layout.as_ref() {
        Some(l) => serde_json::json!({
            "workspace": l.workspace,
            "focus": l.focus,
            "tile_count": l.tiles.len(),
        }),
        None => serde_json::json!({
            "workspace": null,
            "focus": null,
            "tile_count": 0,
        }),
    };
    let detail = serde_wasm_bindgen::to_value(&changed_detail).unwrap_or(JsValue::NULL);
    dispatch(host, "tonk-layout:changed", Some(detail));
}

/// Transition to error state and dispatch the failure event.
fn fail(host: &Element, inner: &Rc<RefCell<Inner>>, err: ErrorDetail) {
    {
        let mut i = inner.borrow_mut();
        i.abort_all();
    }
    let detail = serde_wasm_bindgen::to_value(&err).unwrap_or(JsValue::NULL);
    dispatch(host, "tonk-layout:error", Some(detail));
}

/// True iff the lifecycle this generation belongs to hasn't been
/// disposed or superseded.
fn is_current(inner: &Rc<RefCell<Inner>>, generation: u64) -> bool {
    let i = inner.borrow();
    !i.disposed && i.generation == generation
}

/// Resolved value of the `workspace=` attribute.
fn workspace_name(host: &Element) -> String {
    host.get_attribute("workspace")
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "default".to_owned())
}

fn dispatch(host: &Element, name: &str, detail: Option<JsValue>) {
    let init = CustomEventInit::new();
    if let Some(d) = detail {
        init.set_detail(&d);
    }
    init.set_bubbles(true);
    init.set_composed(true);
    let Ok(event) = CustomEvent::new_with_event_init_dict(name, &init) else {
        return;
    };
    let _ = host.dispatch_event(&event);
}

// -- Effect dispatch -----------------------------------------------

/// Attach one listener per effect event. Each catches the `detail`
/// payload, builds the notation document, and POSTs it.
fn attach_effect_listeners(host: &Element, inner: &Rc<RefCell<Inner>>) {
    let mut listeners = Vec::with_capacity(EFFECT_NAMES.len());
    for &name in EFFECT_NAMES {
        let host_for_handler = host.clone();
        let inner_for_handler = inner.clone();
        let listener = Closure::wrap(Box::new(move |ev: CustomEvent| {
            handle_effect_event(&host_for_handler, &inner_for_handler, name, &ev);
        }) as Box<dyn FnMut(CustomEvent)>);
        let _ = host.add_event_listener_with_callback(name, listener.as_ref().unchecked_ref());
        listeners.push(listener);
    }
    inner.borrow_mut().effect_listeners = listeners;
}

/// Remove every effect listener and drop the Closures backing them.
fn detach_effect_listeners(host: &Element, inner: &Rc<RefCell<Inner>>) {
    let listeners = std::mem::take(&mut inner.borrow_mut().effect_listeners);
    for (name, listener) in EFFECT_NAMES.iter().zip(listeners.iter()) {
        let _ = host.remove_event_listener_with_callback(name, listener.as_ref().unchecked_ref());
    }
}

/// Translate one effect CustomEvent into an `/evaluate` POST.
fn handle_effect_event(
    host: &Element,
    inner: &Rc<RefCell<Inner>>,
    event_name: &str,
    ev: &CustomEvent,
) {
    let detail = ev.detail();
    let layout = inner.borrow().layout.clone();
    let result = match event_name {
        "tonk-layout/focus-tile" => effect_focus_tile(layout.as_ref(), &detail),
        "tonk-layout/focus-prev" => effect_focus_step(layout.as_ref(), Direction::Prev),
        "tonk-layout/focus-next" => effect_focus_step(layout.as_ref(), Direction::Next),
        "tonk-layout/open-tile" => effect_open_tile(host, layout.as_ref(), &detail),
        "tonk-layout/close-tile" => effect_close_tile(layout.as_ref(), &detail),
        "tonk-layout/reorder-tile" => effect_reorder_tile(layout.as_ref(), &detail),
        "tonk-layout/update-tile-content" => effect_update_tile_content(&detail),
        _ => return,
    };
    match result {
        Ok(Some(doc)) => spawn_evaluate_post(host, inner, doc),
        Ok(None) => {} // valid no-op (e.g., focus-prev at the boundary).
        Err(msg) => fail(
            host,
            inner,
            ErrorDetail::new(ErrorKind::Descriptor, format!("{event_name}: {msg}")),
        ),
    }
}

fn effect_focus_tile(layout: Option<&Layout>, detail: &JsValue) -> Result<Option<String>, String> {
    let workspace = layout
        .map(|l| l.workspace.clone())
        .ok_or("no workspace loaded")?;
    let target = detail_string(detail, "target").ok_or("missing target")?;
    Ok(Some(writer::focus_tile_doc(&workspace, &target)))
}

fn effect_focus_step(
    layout: Option<&Layout>,
    direction: Direction,
) -> Result<Option<String>, String> {
    let Some(layout) = layout else {
        // Without a layout we don't know where focus would land.
        // Treat as no-op rather than error — view JS may fire
        // focus-next before the first frame arrives.
        return Ok(None);
    };
    Ok(writer::resolve_focus_step(layout, direction)
        .map(|target| writer::focus_tile_doc(&layout.workspace, &target)))
}

fn effect_open_tile(
    host: &Element,
    layout: Option<&Layout>,
    detail: &JsValue,
) -> Result<Option<String>, String> {
    let view = detail_string(detail, "view").ok_or("missing view")?;
    let model = detail_string(detail, "model").ok_or("missing model")?;
    let entity = detail_string(detail, "entity");
    let before = detail_string(detail, "before");
    let after = detail_string(detail, "after");

    let new_tile_id = format!("id:{}", ulid::new_ulid().ok_or("ulid mint failed")?);

    let (workspace_target, order) = match layout {
        Some(layout) => {
            let order = writer::resolve_position_order(layout, before.as_deref(), after.as_deref())
                .map_err(|e| e.to_owned())?;
            (WorkspaceTarget::Existing(layout.workspace.clone()), order)
        }
        None => {
            // Lazy bootstrap: mint workspace + use `n` as the first
            // tile's order key (no peers to bisect against).
            let new_ws_id = format!("id:{}", ulid::new_ulid().ok_or("ulid mint failed")?);
            (
                WorkspaceTarget::Bootstrap {
                    id: new_ws_id,
                    name: workspace_name(host),
                },
                "n".to_owned(),
            )
        }
    };

    Ok(Some(writer::open_tile_doc(
        workspace_target,
        &new_tile_id,
        &order,
        &view,
        &model,
        entity.as_deref(),
    )))
}

fn effect_close_tile(layout: Option<&Layout>, detail: &JsValue) -> Result<Option<String>, String> {
    let layout = layout.ok_or("no workspace loaded")?;
    let target = detail_string(detail, "target").ok_or("missing target")?;
    let focus_action = writer::resolve_close_focus(layout, &target);
    Ok(Some(writer::close_tile_doc(
        &target,
        &layout.workspace,
        focus_action,
    )))
}

fn effect_reorder_tile(
    layout: Option<&Layout>,
    detail: &JsValue,
) -> Result<Option<String>, String> {
    let layout = layout.ok_or("no workspace loaded")?;
    let target = detail_string(detail, "target").ok_or("missing target")?;
    let before = detail_string(detail, "before");
    let after = detail_string(detail, "after");
    let order = writer::resolve_position_order(layout, before.as_deref(), after.as_deref())
        .map_err(|e| e.to_owned())?;
    Ok(Some(writer::reorder_tile_doc(&target, &order)))
}

fn effect_update_tile_content(detail: &JsValue) -> Result<Option<String>, String> {
    let target = detail_string(detail, "target").ok_or("missing target")?;
    let entity = detail_string(detail, "entity");
    let view = detail_string(detail, "view");
    let model = detail_string(detail, "model");
    let doc = writer::update_tile_content_doc(
        &target,
        entity.as_deref(),
        view.as_deref(),
        model.as_deref(),
    );
    if doc.is_empty() {
        // No field supplied — silent no-op rather than an error doc.
        Ok(None)
    } else {
        Ok(Some(doc))
    }
}

/// Read `detail.<key>` as a string. Returns `None` if `detail` is
/// null, the key is missing, or the value isn't a string.
fn detail_string(detail: &JsValue, key: &str) -> Option<String> {
    if detail.is_null() || detail.is_undefined() {
        return None;
    }
    Reflect::get(detail, &JsValue::from_str(key))
        .ok()
        .and_then(|v| v.as_string())
}

/// Spawn an async task that POSTs `doc` to `/evaluate` via the
/// host's `tonk-evaluate` event. On failure routes the error
/// through [`fail`]. No-ops if the element's generation has
/// moved on between spawn and post — keeps stale effects from
/// leaking writes to a superseded workspace.
fn spawn_evaluate_post(host: &Element, inner: &Rc<RefCell<Inner>>, doc: String) {
    let generation = inner.borrow().generation;
    let host_clone = host.clone();
    let inner_clone = inner.clone();
    spawn_local(async move {
        if !is_current(&inner_clone, generation) {
            return;
        }
        if let Err(err) = host_consumer::evaluate(&host_clone, &doc).await
            && is_current(&inner_clone, generation)
        {
            fail(&host_clone, &inner_clone, err);
        }
    });
}
