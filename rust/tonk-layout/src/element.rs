//! `<tonk-layout>` custom-element implementation.
//!
//! Read-path (step 5): opens three live SSE subscriptions —
//! workspace (filtered by `name`), tiles (unfiltered; the fold drops
//! orphans), and columns (filtered by the resolved workspace entity,
//! opened lazily on the first workspace frame). Every incoming frame
//! is stashed in [`Inner`], re-folded through [`fold_layout`], and
//! used to flip `data-state`. The element doesn't render column /
//! tile DOM yet — the reconciler in step 6 does that.

use std::cell::RefCell;
use std::rc::Rc;

use custom_elements::CustomElement;
use tonk_concept::error::{ErrorDetail, ErrorKind};
use tonk_concept::sse::{SubscriptionAbort, open_sse};
use tonk_schema::conclusion::Conclusion;
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use wasm_bindgen::closure::Closure;
use wasm_bindgen_futures::spawn_local;
use web_sys::{
    CustomEvent, CustomEventInit, Document, Element, Event, HtmlElement, KeyboardEvent,
    MouseEvent, PointerEvent, window,
};

use crate::interact;
use crate::model::{Layout, fold_layout};
use crate::reconcile::reconcile_layout;
use crate::resolve::{columns_query, evaluate_url, query_url, tiles_query, workspace_query};
use crate::state::{self, State};
use crate::ulid;
use crate::writer;

/// Internal lifecycle state shared across async closures.
struct Inner {
    /// Set true by `disconnected_callback` so any frame handler
    /// still running can bail before mutating a host that has
    /// detached.
    disposed: bool,
    /// Bumped every time `start` kicks off a new lifecycle. Each
    /// spawned async chain captures the value at spawn time and
    /// bails on any step that runs after the generation has moved
    /// on.
    generation: u64,
    /// Live workspace subscription (cancelled on drop).
    workspace_abort: Option<SubscriptionAbort>,
    /// Live columns subscription. `None` until the first
    /// non-empty workspace frame opens it.
    columns_abort: Option<SubscriptionAbort>,
    /// Live tiles subscription. Opened alongside columns.
    tiles_abort: Option<SubscriptionAbort>,
    /// True once we've started spawning the children (columns +
    /// tiles) subscriptions for this generation. Prevents a burst
    /// of workspace frames from racing to spawn multiple children
    /// opens.
    children_pending: bool,
    /// Latest frame of each subscription.
    workspace_frame: Vec<Conclusion>,
    columns_frame: Vec<Conclusion>,
    tiles_frame: Vec<Conclusion>,
    /// Latest folded layout — stashed so step 6's reconciler can
    /// diff against it without re-folding from scratch.
    layout: Option<Layout>,
    /// `layout.focus` from the previous refold. Compared against the
    /// new value so `scrollIntoView` only fires on actual focus
    /// changes (avoids fighting a user who scrolled the strip
    /// after a remote-driven refold left focus untouched).
    last_focus: Option<String>,
    /// Live keyboard listener. Held so the closure stays alive for
    /// the duration of this element's connection.
    keydown_listener: Option<Closure<dyn FnMut(KeyboardEvent)>>,
    /// Live click listener — click-to-focus delegation.
    click_listener: Option<Closure<dyn FnMut(MouseEvent)>>,
    /// Pointerdown listener — starts a resize drag if the event
    /// lands on a `.niri-resize` handle, no-op otherwise.
    pointerdown_listener: Option<Closure<dyn FnMut(PointerEvent)>>,
    /// Pointermove listener — applies the running drag (if any).
    pointermove_listener: Option<Closure<dyn FnMut(PointerEvent)>>,
    /// Pointerup / pointercancel listener — ends the drag and
    /// flushes the debouncer.
    pointerup_listener: Option<Closure<dyn FnMut(PointerEvent)>>,
    /// State of the currently-running drag, if any. Replaced on
    /// pointerdown of a fresh handle and cleared on pointerup.
    active_drag: Option<interact::ActiveDrag>,
    /// Debounced `/evaluate` poster — coalesces pointermove flushes
    /// into one POST ~200ms after the pointer goes idle.
    debouncer: writer::Debouncer,
    /// The open-tile dialog, mounted as a sibling of `.niri-strip`
    /// inside the host. Held so we don't open a second instance and
    /// can clean it up on disconnect.
    add_tile_dialog: Option<Element>,
    /// Submit listener for the open-tile dialog's form — held to
    /// keep the closure alive while the listener is attached.
    add_tile_submit: Option<Closure<dyn FnMut(Event)>>,
    /// `wa-after-hide` listener that cleans up after dismissal.
    add_tile_close: Option<Closure<dyn FnMut(Event)>>,
}

impl Inner {
    fn new() -> Self {
        Self {
            disposed: false,
            generation: 0,
            workspace_abort: None,
            columns_abort: None,
            tiles_abort: None,
            children_pending: false,
            workspace_frame: Vec::new(),
            columns_frame: Vec::new(),
            tiles_frame: Vec::new(),
            layout: None,
            last_focus: None,
            keydown_listener: None,
            click_listener: None,
            pointerdown_listener: None,
            pointermove_listener: None,
            pointerup_listener: None,
            active_drag: None,
            debouncer: writer::Debouncer::new(),
            add_tile_dialog: None,
            add_tile_submit: None,
            add_tile_close: None,
        }
    }

    fn abort_all(&mut self) {
        self.workspace_abort.take();
        self.columns_abort.take();
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
            detach_listeners(&host, &inner);
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
        detach_listeners(&host, &inner);
        {
            let mut i = inner.borrow_mut();
            i.abort_all();
            i.children_pending = false;
            i.workspace_frame.clear();
            i.columns_frame.clear();
            i.tiles_frame.clear();
            i.layout = None;
        }
        host.set_inner_html("");
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

/// Bump generation, paint a loading skeleton, attach interaction
/// listeners, kick off the workspace subscription.
fn start(host: &Element, inner: Rc<RefCell<Inner>>) {
    let generation = {
        let mut i = inner.borrow_mut();
        i.generation = i.generation.wrapping_add(1);
        i.generation
    };
    state::set(host, State::Loading);
    mount_skeleton(host);
    attach_listeners(host, &inner);

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

/// Empty `<div class="niri-strip">` shell — the reconciler in step 6
/// will populate column / tile children. Idempotent: clears any
/// prior content first.
fn mount_skeleton(host: &Element) {
    host.set_inner_html("");
    if let Some(document) = window().and_then(|w| w.document())
        && let Ok(strip) = document.create_element("div")
    {
        let _ = strip.set_attribute("class", "niri-strip");
        let _ = host.append_child(&strip);
    }
}

/// Open the workspace SSE subscription. The frame handler stashes
/// each frame, lazily opens the children subscriptions on the first
/// non-empty workspace frame, and re-folds.
async fn run_workspace(
    host: Element,
    inner: Rc<RefCell<Inner>>,
    generation: u64,
) -> Result<(), ErrorDetail> {
    let workspace_name = host
        .get_attribute("workspace")
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "default".to_owned());
    let url = query_url(
        host.get_attribute("space").as_deref(),
        host.get_attribute("branch").as_deref(),
    );

    let q = workspace_query(&workspace_name)
        .map_err(|e| ErrorDetail::new(ErrorKind::Parse, format!("workspace query: {e}")))?;
    let body = serde_json::to_value(&q)
        .map_err(|e| ErrorDetail::new(ErrorKind::Parse, format!("workspace body: {e}")))?;

    let abort = open_frame_stream(
        &url,
        &body,
        host.clone(),
        inner.clone(),
        generation,
        "workspace",
        {
            let url = url.clone();
            move |host, inner, generation, frame| {
                handle_workspace_frame(host, inner, generation, frame, url.clone());
            }
        },
    )
    .await?;

    install_abort(&inner, generation, abort, |i, a| {
        i.workspace_abort = Some(a)
    })?;
    Ok(())
}

/// Common SSE-opening glue: parses each emitted frame into a
/// `Vec<Conclusion>`, runs the per-stream handler, and routes
/// transport errors through `fail`.
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

/// Workspace frame handler: store, lazily open children, refold.
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
        // Open children once, on the first non-empty workspace
        // frame. A workspace re-name elsewhere would change the
        // entity URI; v1 doesn't reopen — the element restarts via
        // `attribute_changed_callback` on `workspace=` changes,
        // which covers the common case.
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

/// Open columns + tiles subscriptions in parallel. Tiles has no
/// workspace dependency, so the only async input is the entity URI
/// the workspace frame just delivered.
async fn run_children(
    host: Element,
    inner: Rc<RefCell<Inner>>,
    generation: u64,
    workspace_entity: String,
    url: String,
) -> Result<(), ErrorDetail> {
    let columns_q = columns_query(&workspace_entity)
        .map_err(|e| ErrorDetail::new(ErrorKind::Parse, format!("columns query: {e}")))?;
    let columns_body = serde_json::to_value(&columns_q)
        .map_err(|e| ErrorDetail::new(ErrorKind::Parse, format!("columns body: {e}")))?;
    let tiles_q = tiles_query()
        .map_err(|e| ErrorDetail::new(ErrorKind::Parse, format!("tiles query: {e}")))?;
    let tiles_body = serde_json::to_value(&tiles_q)
        .map_err(|e| ErrorDetail::new(ErrorKind::Parse, format!("tiles body: {e}")))?;

    let columns_abort = open_frame_stream(
        &url,
        &columns_body,
        host.clone(),
        inner.clone(),
        generation,
        "columns",
        |host, inner, _gen, frame| {
            inner.borrow_mut().columns_frame = frame;
            refold(&host, &inner);
        },
    )
    .await?;
    install_abort(&inner, generation, columns_abort, |i, a| {
        i.columns_abort = Some(a)
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

/// Re-fold the three latest frames into a [`Layout`], patch the
/// DOM via the reconciler, and update `data-state`. Also scrolls
/// the focused column into view when focus changed since the last
/// refold.
fn refold(host: &Element, inner: &Rc<RefCell<Inner>>) {
    let mut i = inner.borrow_mut();
    let layout = fold_layout(&i.workspace_frame, &i.columns_frame, &i.tiles_frame);
    let next_state = match &layout {
        Some(l) if !l.columns.is_empty() => State::Ready,
        // No workspace, or workspace with zero columns: both flow
        // into the `empty` state — the SPEC documents `empty` as
        // covering both.
        _ => State::Empty,
    };
    if let Some(l) = layout.as_ref() {
        reconcile_layout(host, l);
    } else {
        // Workspace not found yet — clear any prior column / tile
        // DOM so the host shows the bare strip skeleton.
        mount_skeleton(host);
    }
    state::set(host, next_state);

    // Scroll on focus change. Done after reconcile so the focused
    // tile's column node exists in the DOM (the reconciler may
    // have just inserted it).
    let new_focus = layout.as_ref().and_then(|l| l.focus.clone());
    if new_focus != i.last_focus
        && let Some(focus) = new_focus.as_deref()
    {
        scroll_focused_into_view(host, focus);
        // Also dispatch the focus event the SPEC promises.
        let detail = serde_wasm_bindgen::to_value(&serde_json::json!({ "tile": focus }))
            .unwrap_or(JsValue::NULL);
        dispatch(host, "tonk-layout:focus", Some(detail));
    }
    i.last_focus = new_focus;

    i.layout = layout;
    dispatch(host, "tonk-layout:layout", None);
}

/// Find the focused tile's column element and scroll it into view
/// with niri's "nearest edge" semantics — only scrolls if the
/// column isn't already fully visible, never moves vertically.
fn scroll_focused_into_view(host: &Element, focus_tile: &str) {
    let selector = format!(".niri-tile[data-entity=\"{focus_tile}\"]");
    let Some(tile_el) = host.query_selector(&selector).ok().flatten() else {
        return;
    };
    let Some(column_el) = tile_el.parent_element() else {
        return;
    };
    let opts = web_sys::ScrollIntoViewOptions::new();
    opts.set_behavior(web_sys::ScrollBehavior::Smooth);
    opts.set_inline(web_sys::ScrollLogicalPosition::Nearest);
    opts.set_block(web_sys::ScrollLogicalPosition::Nearest);
    column_el.scroll_into_view_with_scroll_into_view_options(&opts);
}

/// Transition to error state and dispatch the failure event.
fn fail(host: &Element, inner: &Rc<RefCell<Inner>>, err: ErrorDetail) {
    {
        let mut i = inner.borrow_mut();
        i.abort_all();
    }
    state::set(host, State::Error);
    let detail = serde_wasm_bindgen::to_value(&err).unwrap_or(JsValue::NULL);
    dispatch(host, "tonk-layout:error", Some(detail));
}

/// True iff the lifecycle this generation belongs to hasn't been
/// disposed or superseded.
fn is_current(inner: &Rc<RefCell<Inner>>, generation: u64) -> bool {
    let i = inner.borrow();
    !i.disposed && i.generation == generation
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

/// Wire keyboard + pointer listeners to the host. Closures are
/// stored in [`Inner`] so they outlive the registration; they're
/// dropped (and the listeners removed) by `detach_listeners` on
/// disconnect or attribute change.
fn attach_listeners(host: &Element, inner: &Rc<RefCell<Inner>>) {
    // Make the host focusable so it can receive keydown events.
    // `tabindex="0"` adds it to the natural tab order — users can
    // tab into the strip; once focused, arrow keys / R / Q fire.
    let _ = host.set_attribute("tabindex", "0");

    let host_for_key = host.clone();
    let inner_for_key = inner.clone();
    let keydown = Closure::wrap(Box::new(move |ev: KeyboardEvent| {
        let layout = inner_for_key.borrow().layout.clone();
        let Some(layout) = layout else {
            return;
        };
        match interact::handle_keydown(&layout, &ev) {
            Some(interact::KeydownAction::PostDoc(doc)) => {
                ev.prevent_default();
                spawn_evaluate_post(&host_for_key, &inner_for_key, doc);
            }
            Some(interact::KeydownAction::OpenAddTileDialog) => {
                ev.prevent_default();
                open_add_tile_dialog(&host_for_key, &inner_for_key);
            }
            None => {}
        }
    }) as Box<dyn FnMut(KeyboardEvent)>);
    let _ = host.add_event_listener_with_callback("keydown", keydown.as_ref().unchecked_ref());

    let host_for_click = host.clone();
    let inner_for_click = inner.clone();
    let click = Closure::wrap(Box::new(move |ev: MouseEvent| {
        // Suppress click-to-focus if the click was the end of a
        // resize drag — pointerup synthesises a click on the
        // handle, which would otherwise yank focus to nothing.
        if inner_for_click.borrow().active_drag.is_some() {
            return;
        }
        let layout = inner_for_click.borrow().layout.clone();
        let Some(layout) = layout else {
            return;
        };
        if let Some(doc) = interact::handle_click(&layout, &ev) {
            spawn_evaluate_post(&host_for_click, &inner_for_click, doc);
        }
    }) as Box<dyn FnMut(MouseEvent)>);
    let _ = host.add_event_listener_with_callback("click", click.as_ref().unchecked_ref());

    // Pointerdown — start a resize drag if the event landed on a
    // handle. Otherwise the click listener above takes over.
    let host_for_pd = host.clone();
    let inner_for_pd = inner.clone();
    let pointerdown = Closure::wrap(Box::new(move |ev: PointerEvent| {
        let layout = inner_for_pd.borrow().layout.clone();
        let Some(layout) = layout else {
            return;
        };
        if let Some(drag) = interact::start_resize_drag(&host_for_pd, &layout, &ev) {
            ev.prevent_default();
            inner_for_pd.borrow_mut().active_drag = Some(drag);
        }
    }) as Box<dyn FnMut(PointerEvent)>);
    let _ = host
        .add_event_listener_with_callback("pointerdown", pointerdown.as_ref().unchecked_ref());

    // Pointermove — bail unless a drag is running. Computes the
    // new width, updates inline CSS directly (instant feedback),
    // and schedules a debounced /evaluate POST.
    let host_for_pm = host.clone();
    let inner_for_pm = inner.clone();
    let pointermove = Closure::wrap(Box::new(move |ev: PointerEvent| {
        let drag = inner_for_pm.borrow().active_drag.clone();
        let Some(drag) = drag else {
            return;
        };
        let new_width = interact::update_resize_drag(&drag, &ev);
        let url = evaluate_url(
            host_for_pm.get_attribute("space").as_deref(),
            host_for_pm.get_attribute("branch").as_deref(),
        );
        let doc = writer::resize_column_doc(&drag.column_entity, new_width);
        inner_for_pm.borrow().debouncer.schedule(url, doc, 200);
    }) as Box<dyn FnMut(PointerEvent)>);
    let _ = host
        .add_event_listener_with_callback("pointermove", pointermove.as_ref().unchecked_ref());

    // Pointerup / pointercancel — end the drag, release pointer
    // capture, and flush the debouncer so the commit lands on
    // release without waiting for the trailing 200ms.
    let inner_for_pu = inner.clone();
    let pointerup = Closure::wrap(Box::new(move |_ev: PointerEvent| {
        let drag = inner_for_pu.borrow_mut().active_drag.take();
        if let Some(drag) = drag {
            interact::end_resize_drag(&drag);
            inner_for_pu.borrow().debouncer.flush();
        }
    }) as Box<dyn FnMut(PointerEvent)>);
    let _ =
        host.add_event_listener_with_callback("pointerup", pointerup.as_ref().unchecked_ref());
    let _ = host
        .add_event_listener_with_callback("pointercancel", pointerup.as_ref().unchecked_ref());

    let mut i = inner.borrow_mut();
    i.keydown_listener = Some(keydown);
    i.click_listener = Some(click);
    i.pointerdown_listener = Some(pointerdown);
    i.pointermove_listener = Some(pointermove);
    i.pointerup_listener = Some(pointerup);
}

/// Remove the listeners we attached and drop the Closures that
/// back them. Must run before the host's event-target identity
/// gets reused — leaving a dangling listener referencing a freed
/// Closure crashes on dispatch.
fn detach_listeners(host: &Element, inner: &Rc<RefCell<Inner>>) {
    let mut i = inner.borrow_mut();
    if let Some(keydown) = i.keydown_listener.take() {
        let _ =
            host.remove_event_listener_with_callback("keydown", keydown.as_ref().unchecked_ref());
    }
    if let Some(click) = i.click_listener.take() {
        let _ = host.remove_event_listener_with_callback("click", click.as_ref().unchecked_ref());
    }
    if let Some(pointerdown) = i.pointerdown_listener.take() {
        let _ = host.remove_event_listener_with_callback(
            "pointerdown",
            pointerdown.as_ref().unchecked_ref(),
        );
    }
    if let Some(pointermove) = i.pointermove_listener.take() {
        let _ = host.remove_event_listener_with_callback(
            "pointermove",
            pointermove.as_ref().unchecked_ref(),
        );
    }
    if let Some(pointerup) = i.pointerup_listener.take() {
        let _ =
            host.remove_event_listener_with_callback("pointerup", pointerup.as_ref().unchecked_ref());
        let _ = host.remove_event_listener_with_callback(
            "pointercancel",
            pointerup.as_ref().unchecked_ref(),
        );
    }
    // Drop any in-flight debounced POST — element is going away.
    i.debouncer.cancel();
    i.active_drag = None;
    // Tear down the open-tile dialog if it's still mounted.
    if let Some(dialog) = i.add_tile_dialog.take() {
        dialog.remove();
    }
    i.add_tile_submit = None;
    i.add_tile_close = None;
}

/// Mount a `<wa-dialog>` prompting for the new tile's `entity` /
/// `model` / `view`. Submitting builds a `kind: "display"` tile
/// row, POSTs it, and closes the dialog; the new tile appears via
/// the subscription frame, no manual reconciliation needed.
fn open_add_tile_dialog(host: &Element, inner: &Rc<RefCell<Inner>>) {
    if inner.borrow().add_tile_dialog.is_some() {
        return;
    }
    let Some(document) = window().and_then(|w| w.document()) else {
        return;
    };
    let Ok(dialog) = document.create_element("wa-dialog") else {
        return;
    };
    let _ = dialog.set_attribute("label", "Open tile");
    let _ = dialog.set_attribute("open", "");
    let _ = dialog.set_attribute("class", "tonk-layout-add-tile");

    let Ok(form) = document.create_element("form") else {
        return;
    };
    let _ = form.set_attribute("class", "tonk-layout-add-tile-form");
    let _ = form.set_attribute(
        "style",
        "display: flex; flex-direction: column; gap: var(--wa-space-m, 12px);",
    );

    let entity_input = make_input(&document, "entity", "Entity URI", true);
    let model_input = make_input(&document, "model", "Model name", false);
    let view_input = make_input(&document, "view", "View name", false);
    let _ = form.append_child(&entity_input);
    let _ = form.append_child(&model_input);
    let _ = form.append_child(&view_input);

    if let Ok(submit_btn) = document.create_element("wa-button") {
        let _ = submit_btn.set_attribute("type", "submit");
        let _ = submit_btn.set_attribute("variant", "brand");
        let _ = submit_btn.set_attribute("appearance", "filled");
        submit_btn.set_text_content(Some("Open"));
        let _ = form.append_child(&submit_btn);
    }

    let _ = dialog.append_child(&form);
    let _ = host.append_child(&dialog);

    // Submit — read inputs, build doc, POST, close dialog.
    let host_for_submit = host.clone();
    let inner_for_submit = inner.clone();
    let dialog_for_submit = dialog.clone();
    let entity_for_submit = entity_input.clone();
    let model_for_submit = model_input.clone();
    let view_for_submit = view_input.clone();
    let submit = Closure::wrap(Box::new(move |ev: Event| {
        ev.prevent_default();
        let entity = read_value(&entity_for_submit);
        if entity.is_empty() {
            return;
        }
        let model = read_value(&model_for_submit);
        let view = read_value(&view_for_submit);
        submit_add_tile(
            &host_for_submit,
            &inner_for_submit,
            &dialog_for_submit,
            &entity,
            &model,
            &view,
        );
    }) as Box<dyn FnMut(Event)>);
    let _ = form.add_event_listener_with_callback("submit", submit.as_ref().unchecked_ref());

    // wa-after-hide fires on dismissal (X button, ESC, programmatic
    // close). Clean up our refs and remove the element from the DOM.
    let inner_for_close = inner.clone();
    let dialog_for_close = dialog.clone();
    let close = Closure::wrap(Box::new(move |_ev: Event| {
        let mut i = inner_for_close.borrow_mut();
        i.add_tile_dialog = None;
        i.add_tile_submit = None;
        i.add_tile_close = None;
        dialog_for_close.remove();
    }) as Box<dyn FnMut(Event)>);
    let _ = dialog
        .add_event_listener_with_callback("wa-after-hide", close.as_ref().unchecked_ref());

    let mut i = inner.borrow_mut();
    i.add_tile_dialog = Some(dialog);
    i.add_tile_submit = Some(submit);
    i.add_tile_close = Some(close);
}

/// Build a `<wa-input>` with name/label and (optionally) required.
fn make_input(document: &Document, name: &str, label: &str, required: bool) -> Element {
    let input = document
        .create_element("wa-input")
        .expect("wa-input always creates");
    let _ = input.set_attribute("name", name);
    let _ = input.set_attribute("label", label);
    if required {
        let _ = input.set_attribute("required", "");
    }
    input
}

/// Read a `<wa-input>`'s `.value` JS property. Plain `get_attribute`
/// doesn't see the value the user typed — Web Awesome's inputs
/// expose it as a property, not as a reflected attribute.
fn read_value(input: &Element) -> String {
    use js_sys::Reflect;
    Reflect::get(input.as_ref(), &JsValue::from_str("value"))
        .ok()
        .and_then(|v| v.as_string())
        .unwrap_or_default()
}

/// Build + POST the doc that opens a new tile. Handles all three
/// states the branch can be in:
///
/// - **Layout has columns** — append to the focused column (or the
///   first column if no focus); one `tile!:` block.
/// - **Workspace exists, no columns** — mint a column ULID, emit
///   `column!:` + `tile!:` in one document so they commit together.
/// - **No workspace yet** — mint workspace + column ULIDs from the
///   `workspace` attribute's name; emit `workspace!:` + `column!:` +
///   `tile!:` in one document for full lazy bootstrap.
fn submit_add_tile(
    host: &Element,
    inner: &Rc<RefCell<Inner>>,
    dialog: &Element,
    display_entity: &str,
    model: &str,
    view: &str,
) {
    let layout = inner.borrow().layout.clone();
    let target = interact::pick_new_tile_target(layout.as_ref());

    let Some(tile_ulid) = ulid::new_ulid() else {
        return;
    };
    let tile_id = format!("id:{tile_ulid}");

    let doc = match target {
        interact::NewTileTarget::AppendToColumn {
            column_entity,
            tile_order,
        } => writer::create_display_tile_doc(
            &tile_id,
            &column_entity,
            &tile_order,
            1.0,
            display_entity,
            view,
            model,
        ),
        interact::NewTileTarget::NewColumn { workspace_entity } => {
            let Some(col_ulid) = ulid::new_ulid() else {
                return;
            };
            let col_id = format!("id:{col_ulid}");
            writer::column_creation_block(&col_id, &workspace_entity, "n", 1.0)
                + "\n"
                + &writer::create_display_tile_doc(
                    &tile_id,
                    &col_id,
                    "n",
                    1.0,
                    display_entity,
                    view,
                    model,
                )
        }
        interact::NewTileTarget::NewWorkspace => {
            let workspace_name = host
                .get_attribute("workspace")
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "default".to_owned());
            let Some(ws_ulid) = ulid::new_ulid() else {
                return;
            };
            let Some(col_ulid) = ulid::new_ulid() else {
                return;
            };
            let ws_id = format!("id:{ws_ulid}");
            let col_id = format!("id:{col_ulid}");
            writer::workspace_creation_block(&ws_id, &workspace_name)
                + "\n"
                + &writer::column_creation_block(&col_id, &ws_id, "n", 1.0)
                + "\n"
                + &writer::create_display_tile_doc(
                    &tile_id,
                    &col_id,
                    "n",
                    1.0,
                    display_entity,
                    view,
                    model,
                )
        }
    };

    spawn_evaluate_post(host, inner, doc);
    // Programmatically close the dialog — wa-after-hide fires
    // and our close listener tears down the refs.
    let _ = dialog.remove_attribute("open");
}

/// Spawn an async task that POSTs `doc` to `/evaluate`. On failure
/// routes the error through [`fail`], surfacing it on the host's
/// `data-state` and via the `tonk-layout:error` event.
fn spawn_evaluate_post(host: &Element, inner: &Rc<RefCell<Inner>>, doc: String) {
    let url = evaluate_url(
        host.get_attribute("space").as_deref(),
        host.get_attribute("branch").as_deref(),
    );
    let host_clone = host.clone();
    let inner_clone = inner.clone();
    spawn_local(async move {
        match writer::post_evaluate(&url, &doc).await {
            Ok(()) => {
                // Subscription frame will reflect the new state.
            }
            Err(err) => {
                if !inner_clone.borrow().disposed {
                    fail(&host_clone, &inner_clone, err);
                }
            }
        }
    });
}
