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
use js_sys::Reflect;
use tonk_concept::error::{ErrorDetail, ErrorKind};
use tonk_concept::sse::{SubscriptionAbort, open_sse};
use tonk_schema::conclusion::Conclusion;
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use wasm_bindgen::closure::Closure;
use wasm_bindgen_futures::spawn_local;
use web_sys::{CustomEvent, CustomEventInit, Element, HtmlElement, window};

use crate::model::{Layout, fold_universal};
use crate::resolve::{evaluate_url, focus_query, query_url, tiles_query, workspace_query};
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
    /// Live subscriptions (cancelled on drop).
    workspace_abort: Option<SubscriptionAbort>,
    focus_abort: Option<SubscriptionAbort>,
    tiles_abort: Option<SubscriptionAbort>,
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
}

impl Inner {
    fn new() -> Self {
        Self {
            disposed: false,
            generation: 0,
            workspace_abort: None,
            focus_abort: None,
            tiles_abort: None,
            children_pending: false,
            workspace_frame: Vec::new(),
            focus_frame: Vec::new(),
            tiles_frame: Vec::new(),
            layout: None,
            last_focus: None,
            effect_listeners: Vec::new(),
        }
    }

    fn abort_all(&mut self) {
        self.workspace_abort.take();
        self.focus_abort.take();
        self.tiles_abort.take();
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
        &["workspace", "space", "branch"]
    }

    fn inject_children(&mut self, _this: &HtmlElement) {}

    fn connected_callback(&mut self, this: &HtmlElement) {
        let host: Element = this.clone().into();
        let inner = Rc::new(RefCell::new(Inner::new()));
        *self.inner.borrow_mut() = Some(inner.clone());
        start(&host, inner);
    }

    fn disconnected_callback(&mut self, this: &HtmlElement) {
        if let Some(inner) = self.inner.borrow_mut().take() {
            let host: Element = this.clone().into();
            detach_effect_listeners(&host, &inner);
            let mut i = inner.borrow_mut();
            i.disposed = true;
            i.abort_all();
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
}

fn already_registered() -> bool {
    let Some(win) = window() else {
        return false;
    };
    !win.custom_elements().get("tonk-layout").is_undefined()
}

/// Bump generation, attach effect listeners, kick off the workspace
/// subscription.
fn start(host: &Element, inner: Rc<RefCell<Inner>>) {
    let generation = {
        let mut i = inner.borrow_mut();
        i.generation = i.generation.wrapping_add(1);
        i.generation
    };
    attach_effect_listeners(host, &inner);

    let host_for_run = host.clone();
    let inner_for_run = inner.clone();
    spawn_local(async move {
        if let Err(err) =
            run_workspace(host_for_run.clone(), inner_for_run.clone(), generation).await
            && is_current(&inner_for_run, generation)
        {
            fail(&host_for_run, &inner_for_run, err);
        }
    });
}

/// Open the workspace SSE subscription. Each frame stashes itself,
/// lazily opens focus + tiles subscriptions on the first non-empty
/// workspace frame, and refolds.
async fn run_workspace(
    host: Element,
    inner: Rc<RefCell<Inner>>,
    generation: u64,
) -> Result<(), ErrorDetail> {
    let url = query_url(
        host.get_attribute("space").as_deref(),
        host.get_attribute("branch").as_deref(),
    );
    let q = workspace_query(&workspace_name(&host))
        .map_err(|e| ErrorDetail::new(ErrorKind::Parse, format!("workspace query: {e}")))?;
    let body = serde_json::to_value(&q)
        .map_err(|e| ErrorDetail::new(ErrorKind::Parse, format!("workspace body: {e}")))?;

    let url_for_children = url.clone();
    let abort = open_frame_stream(
        &url,
        &body,
        host.clone(),
        inner.clone(),
        generation,
        "workspace",
        move |host, inner, generation, frame| {
            handle_workspace_frame(host, inner, generation, frame, url_for_children.clone());
        },
    )
    .await?;

    install_abort(&inner, generation, abort, |i, a| {
        i.workspace_abort = Some(a)
    })?;
    Ok(())
}

/// Common SSE-opening glue: parse each emitted frame into a
/// `Vec<Conclusion>`, run the per-stream handler, route transport
/// errors through `fail`.
async fn open_frame_stream<H>(
    url: &str,
    body: &serde_json::Value,
    host: Element,
    inner: Rc<RefCell<Inner>>,
    generation: u64,
    label: &'static str,
    handler: H,
) -> Result<SubscriptionAbort, ErrorDetail>
where
    H: Fn(Element, Rc<RefCell<Inner>>, u64, Vec<Conclusion>) + 'static,
{
    let host_for_frame = host.clone();
    let inner_for_frame = inner.clone();
    let host_for_err = host.clone();
    let inner_for_err = inner.clone();
    open_sse(
        url,
        body,
        move |frame: &str| {
            if !is_current(&inner_for_frame, generation) {
                return;
            }
            let conclusions: Vec<Conclusion> = match serde_json::from_str(frame) {
                Ok(v) => v,
                Err(e) => {
                    fail(
                        &host_for_frame,
                        &inner_for_frame,
                        ErrorDetail::new(ErrorKind::Parse, format!("{label} frame: {e}")),
                    );
                    return;
                }
            };
            handler(
                host_for_frame.clone(),
                inner_for_frame.clone(),
                generation,
                conclusions,
            );
        },
        move |err: ErrorDetail| {
            if !is_current(&inner_for_err, generation) {
                return;
            }
            fail(&host_for_err, &inner_for_err, err);
        },
    )
    .await
}

/// Workspace frame handler: store, lazily open focus + tiles, refold.
fn handle_workspace_frame(
    host: Element,
    inner: Rc<RefCell<Inner>>,
    generation: u64,
    frame: Vec<Conclusion>,
    url: String,
) {
    let workspace_entity = frame.first().map(|c| c.this.clone());
    let should_open_children = {
        let mut i = inner.borrow_mut();
        i.workspace_frame = frame;
        workspace_entity.is_some() && !i.children_pending
    };
    if should_open_children && let Some(ws_entity) = workspace_entity {
        inner.borrow_mut().children_pending = true;
        let host_for_open = host.clone();
        let inner_for_open = inner.clone();
        spawn_local(async move {
            if let Err(err) = run_children(
                host_for_open.clone(),
                inner_for_open.clone(),
                generation,
                ws_entity,
                url,
            )
            .await
                && is_current(&inner_for_open, generation)
            {
                fail(&host_for_open, &inner_for_open, err);
            }
        });
    }
    refold(&host, &inner);
}

/// Open focus + tiles subscriptions in parallel once the workspace
/// URI is known.
async fn run_children(
    host: Element,
    inner: Rc<RefCell<Inner>>,
    generation: u64,
    workspace_entity: String,
    url: String,
) -> Result<(), ErrorDetail> {
    let focus_q = focus_query(&workspace_entity)
        .map_err(|e| ErrorDetail::new(ErrorKind::Parse, format!("focus query: {e}")))?;
    let focus_body = serde_json::to_value(&focus_q)
        .map_err(|e| ErrorDetail::new(ErrorKind::Parse, format!("focus body: {e}")))?;
    let tiles_q = tiles_query()
        .map_err(|e| ErrorDetail::new(ErrorKind::Parse, format!("tiles query: {e}")))?;
    let tiles_body = serde_json::to_value(&tiles_q)
        .map_err(|e| ErrorDetail::new(ErrorKind::Parse, format!("tiles body: {e}")))?;

    let focus_abort = open_frame_stream(
        &url,
        &focus_body,
        host.clone(),
        inner.clone(),
        generation,
        "focus",
        |host, inner, _gen, frame| {
            inner.borrow_mut().focus_frame = frame;
            refold(&host, &inner);
        },
    )
    .await?;
    install_abort(&inner, generation, focus_abort, |i, a| {
        i.focus_abort = Some(a)
    })?;

    let tiles_abort = open_frame_stream(
        &url,
        &tiles_body,
        host.clone(),
        inner.clone(),
        generation,
        "tiles",
        |host, inner, _gen, frame| {
            inner.borrow_mut().tiles_frame = frame;
            refold(&host, &inner);
        },
    )
    .await?;
    install_abort(&inner, generation, tiles_abort, |i, a| {
        i.tiles_abort = Some(a)
    })?;
    Ok(())
}

/// Stash an abort handle if the lifecycle hasn't moved on, dropping
/// it otherwise (which cancels the just-opened transport).
fn install_abort<F>(
    inner: &Rc<RefCell<Inner>>,
    generation: u64,
    abort: SubscriptionAbort,
    install: F,
) -> Result<(), ErrorDetail>
where
    F: FnOnce(&mut Inner, SubscriptionAbort),
{
    let mut i = inner.borrow_mut();
    if i.disposed || i.generation != generation {
        drop(abort);
        return Err(ErrorDetail::new(ErrorKind::Descriptor, "superseded"));
    }
    install(&mut i, abort);
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

/// Spawn an async task that POSTs `doc` to `/evaluate`. On failure
/// routes the error through [`fail`]. No-ops if the element's
/// generation has moved on between spawn and post — keeps stale
/// effects from leaking writes to a superseded workspace.
fn spawn_evaluate_post(host: &Element, inner: &Rc<RefCell<Inner>>, doc: String) {
    let url = evaluate_url(
        host.get_attribute("space").as_deref(),
        host.get_attribute("branch").as_deref(),
    );
    let generation = inner.borrow().generation;
    let host_clone = host.clone();
    let inner_clone = inner.clone();
    spawn_local(async move {
        if !is_current(&inner_clone, generation) {
            return;
        }
        if let Err(err) = writer::post_evaluate(&url, &doc).await
            && is_current(&inner_clone, generation)
        {
            fail(&host_clone, &inner_clone, err);
        }
    });
}
