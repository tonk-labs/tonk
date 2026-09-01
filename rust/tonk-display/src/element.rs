//! `<tonk-display>` custom-element implementation.
//!
//! Coordinates live data flows for a single rendered entity and
//! mounts a dumb-renderer `<tonk-view>` as a slide.
//!
//! The `view` attribute names a view *concept* (named or URI); the
//! `model` attribute names the subject's model concept. Resolution
//! queries the view concept constrained to that model and reads the
//! row's `display` field as the template — the view concept's
//! descriptor maps the `display` field name to whatever attribute
//! that view kind declares it under, so any conforming view kind
//! works. When `view` is omitted, the built-in `view` concept's
//! descriptor is used. Every entity frame calls `.render(conclusion)`
//! on the mounted `<tonk-view>`; view-text edits replace it with a
//! fresh one carrying the new children.
//!
//! `view="about:blank"` is reserved for a future carousel mode
//! (enumerate every view for the model); it currently errors.
//!
//! Subscriptions live in `<tonk-display>` only — slide elements
//! never open their own. One model resolution + one view
//! subscription + one entity subscription per attribute set.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

use crate::resolve::{ParsedSource, name_query, parse_source, phase1_query};
use custom_elements::CustomElement;
use ipld_core::ipld::Ipld;
use js_sys::{Array, Function, Promise, Reflect};
use tonk_host::consumer::{self as host_consumer, Subscription as HostSubscription};
use tonk_host::error::{ErrorDetail, ErrorKind};
use tonk_host::install_depth_annotator;
use tonk_schema::conclusion::Conclusion;
use tonk_schema::query::Query;
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use wasm_bindgen::closure::Closure;
use wasm_bindgen_futures::spawn_local;
use web_sys::{
    CustomEvent, CustomEventInit, Element, HtmlElement, MutationObserver, MutationObserverInit,
    MutationRecord, Node, window,
};

use crate::resolve::{
    directory_view_predicate, entity_query, instances_query, looks_like_uri, view_by_model_query,
    view_predicate,
};
use crate::state::{self, State};

/// Sentinel `view` value that selects carousel mode — enumerate
/// every view defined for the model instead of rendering one. Uses
/// `about:blank` to match the "blank view" convention (a new
/// artifact's `view` is `about:blank` until a presentation is
/// chosen).
const CAROUSEL_VIEW: &str = "about:blank";

/// One mounted slide. Keyed by the view entity URI (the `this` of
/// the resolved view row); `display` is the template HTML the
/// slide's `<tonk-view>` was built with so we can detect content
/// changes and rebuild.
struct Slide {
    /// The `display` HTML the slide's `<tonk-view>` was built
    /// with. Used to detect template-content changes so we rebuild
    /// the slide instead of just re-rendering the conclusion.
    display: String,
    /// The `<wa-carousel-item>` (carousel mode) or container `<div>`
    /// (single mode) that wraps the `<tonk-view>`. Carousel-item
    /// kept here so we can remove the slide cleanly when its view
    /// vanishes.
    item: Element,
    /// The `<tonk-view>` instance inside `item`. Receives
    /// `.render(conclusion)` on every entity frame.
    view_el: Element,
}

/// Internal lifecycle state shared across async closures.
struct Inner {
    /// Set to true by `disconnected_callback` so any async
    /// chain still running against this state (typically one
    /// blocked on phase-1 fetch when the element detached) can
    /// bail before mutating the host. Without this guard a
    /// stale chain's view-frame handler can mount a slide into
    /// a host whose `<tonk-display>` instance has since been
    /// re-attached and is running a *different* state — yielding
    /// duplicated `<tonk-view>` children under one host.
    disposed: bool,
    /// Monotonic counter incremented every time `start_flows`
    /// kicks off a new lifecycle on the *current* state. Each
    /// spawned async chain captures the value at spawn time and
    /// bails on any step that runs after the generation has
    /// moved on. Generation is per-`Inner`; the disposed flag
    /// covers the cross-`Inner` race where a custom element
    /// detaches and re-attaches.
    generation: u64,
    /// Cancels the phase-1 **model** subscription on disconnect /
    /// attribute change. The model resolve is a live subscription (not
    /// a one-shot) so a concept seeded *after* the element mounts
    /// pushes a frame and the display recovers from `no-model` without
    /// a reload. Each model frame (re)starts the downstream view +
    /// entity flow via `handle_model_frame`.
    model_sub: Option<HostSubscription>,
    /// Bumped every time a model frame (re)starts the downstream flow.
    /// A downstream async chain captures this at spawn and bails if a
    /// newer model frame has superseded it, mirroring `generation` but
    /// scoped to the model→downstream restart so a late model frame
    /// doesn't race an in-flight downstream setup.
    downstream_generation: u64,
    /// The `(model_entity, descriptor_json)` the current downstream flow
    /// was started for, recorded **synchronously** in `handle_model_frame`
    /// before the async `start_downstream` spawns. The model subscription
    /// re-pushes a frame on every branch revision (including unrelated
    /// data writes); this lets the handler skip restarting the downstream
    /// flow when the resolved concept is unchanged, so a plain write
    /// updates the entity rows in place instead of tearing the whole host
    /// down and remounting. Must be set here (not in `start_downstream`'s
    /// tail) so back-to-back model frames during one write don't all see
    /// a stale value and each trigger a remount.
    ///
    /// The descriptor is stored **parsed** (`serde_json::Value`), not as
    /// the raw `source` string: the worker re-serializes the descriptor
    /// with non-deterministic object-key ordering, so two byte-different
    /// strings can describe the identical concept. Comparing parsed
    /// values is order-independent, so an unrelated branch write (which
    /// re-pushes the same concept with shuffled keys) no longer counts as
    /// a change and the list is not torn down.
    resolved_model: Option<(String, serde_json::Value)>,
    /// Cancels the view (or views-for-model) subscription on
    /// disconnect / attribute change. Dropping the handle calls
    /// the host's `cancel()` and dispatches `tonk-unsubscribe`.
    view_sub: Option<HostSubscription>,
    /// Cancels the entity subscription on disconnect / attribute
    /// change.
    entity_sub: Option<HostSubscription>,
    /// Last folded entity frame seen; replayed in full when a fresh
    /// slide mounts so it picks up the current data without waiting for
    /// the next entity frame. The whole frame matters in directory mode
    /// (one conclusion per instance) — replaying only the lead would
    /// drop every instance but the first. The lead conclusion (for
    /// notation / the result event) is `last_frame.first()`.
    last_frame: Vec<Conclusion>,
    /// The last full row set seen per subscription tag
    /// (`model` / `view` / `entity`), retained so an incremental
    /// `update` delta can be applied to the correct tag's set and the
    /// merged full set routed to that tag's handler. A `reset` replaces
    /// the tag's entry; a `delta` mutates it. Without this a model/view
    /// delta would merge into the entity `last_frame` and resolve the
    /// wrong (or empty) set — the "Model not found" after an empty
    /// snapshot then a stamping delta.
    retained: BTreeMap<String, Vec<Conclusion>>,
    /// `<wa-carousel>` element when in carousel mode, `None` in
    /// single mode.
    carousel: Option<Element>,
    /// Slides keyed by the `view` attribute (single mode — exactly
    /// one entry) or the view entity URI (carousel mode).
    slides: BTreeMap<String, Slide>,
    /// The trailing carousel slide's source `<script
    /// type="text/tonk-notation">` element. We update its
    /// `textContent` on every entity frame; the enclosing
    /// `<tonk-notation>` re-renders via its MutationObserver. Only
    /// present in carousel mode.
    notation_source: Option<Element>,
    /// The `<wa-carousel-item>` that wraps the `<tonk-notation>`
    /// slide, kept so `mount_view_slide` can insert new view slides
    /// before it.
    notation_item: Option<Element>,
    /// Event-handler delegation listeners installed on the host.
    /// One [`Delegate`] per `<tonk-display>` instance; dropped on
    /// disconnect or attribute change so listeners cycle with the
    /// generation. `None` when no `on<event>` bindings appear in
    /// the mounted templates.
    delegate: Option<crate::events::delegate::Delegate>,
    /// Bumps every time `schedule_delegate_refresh` runs. A scheduled
    /// refresh captures the current value at start and bails before
    /// writing if it has moved on. Without this guard a view-frame
    /// swap that lands while a previous refresh is still resolving
    /// concept descriptors would write a stale [`Delegate`] over the
    /// newer one.
    delegate_generation: u64,
    /// Depth annotator installed at `connected_callback` — its
    /// listeners increment `event.detail.depth` on every operation
    /// event that bubbles up through this element. Dropped on
    /// disconnect.
    depth_annotator: Option<tonk_host::DepthAnnotator>,
    /// Bumped on every entity frame. A reconnect-empty hold captures the
    /// serial and applies the emptiness after the grace period only if no
    /// further frame superseded it.
    entity_serial: u64,
    /// Watches the host's own attributes that mounted views consume via
    /// `{dom.host/<attr>}` (advertised by each slide as
    /// `data-host-bindings`). A genuine value change replays the cached
    /// frame through every slide's binding diff, so `dom.host/*` is a
    /// live render input rather than a mount-time snapshot.
    host_watch: Option<HostAttrWatch>,
    /// The model concept's descriptor JSON, handed to a `<tonk-portal>`
    /// so its no-argument `tonk.subscribe()` can build the scoped-entity
    /// query. Set on every connect; only read when a view frame routes
    /// to portal mode (its projected `type` is `text/html`).
    portal_descriptor: Option<String>,
    /// The resolved model entity, surfaced to the portal as its `model`
    /// attribute (the bridge's `context.model`).
    portal_model: Option<String>,
    /// The view-concept descriptor used to resolve a view by model.
    /// Retained so the `_:_` default-view fallback can re-query against
    /// the same view concept when the model-specific view frame is
    /// empty.
    view_descriptor: Option<serde_json::Value>,
    /// The resolved model entity (`model_entity`), retained for the
    /// `_:_` fallback query and for comparison.
    model_entity: Option<String>,
    /// True when the currently-mounted slide came from the `_:_`
    /// default view rather than a model-specific view. A non-empty
    /// model-specific view frame replaces it (the specific view takes
    /// over); set false then.
    default_slide: bool,
    /// Directory mode: `true` when the host has no `entity`, so the
    /// entity subscription matches every instance of the model (`this`
    /// unbound) and the frame is grouped by `this` (`select_rows`)
    /// rather than folded to one conclusion.
    directory: bool,
}

impl Inner {
    fn new() -> Self {
        Self {
            disposed: false,
            generation: 0,
            model_sub: None,
            downstream_generation: 0,
            resolved_model: None,
            view_sub: None,
            entity_sub: None,
            last_frame: Vec::new(),
            retained: BTreeMap::new(),
            carousel: None,
            slides: BTreeMap::new(),
            notation_source: None,
            notation_item: None,
            delegate: None,
            delegate_generation: 0,
            depth_annotator: None,
            entity_serial: 0,
            host_watch: None,
            portal_descriptor: None,
            portal_model: None,
            view_descriptor: None,
            model_entity: None,
            default_slide: false,
            directory: false,
        }
    }

    fn abort_all(&mut self) {
        // Dropping the subscriptions cancels via the host and
        // dispatches `tonk-unsubscribe`.
        self.model_sub.take();
        self.view_sub.take();
        self.entity_sub.take();
        // Forget the resolved concept so a restart (attribute change /
        // reconnect) re-resolves and remounts even if the new model
        // happens to resolve to the same concept.
        self.resolved_model = None;
        // Drop the delegate — its impl removes listeners from the
        // host on Drop.
        self.delegate.take();
        // Disconnect the host-attribute watcher.
        self.host_watch.take();
    }
}

/// The custom element.
#[derive(Default)]
pub struct TonkDisplay {
    inner: RefCell<Option<Rc<RefCell<Inner>>>>,
}

impl CustomElement for TonkDisplay {
    fn shadow() -> bool {
        false
    }

    fn observed_attributes() -> &'static [&'static str] {
        // `entity`/`model`/`view` are the subject inputs: a change to any
        // restarts the resolve/subscribe flow. `data-active` / `data-base`
        // are host-context attributes a parent threads in (read by a
        // template as `{dom.host/<attr>}`); a change to one does not alter
        // what this display resolves, only the value projected into the
        // already-mounted view, so it is propagated in place rather than
        // restarting. `data-base` lets a wrapping `<tonk-origin>` deliver
        // the invite-URL base (`{origin}/join`) into the view after mount.
        // See `attribute_changed_callback`.
        &["entity", "model", "view", "data-active", "data-base"]
    }

    fn inject_children(&mut self, _this: &HtmlElement) {}

    fn connected_callback(&mut self, this: &HtmlElement) {
        let host: Element = this.clone().into();
        state::set(&host, State::Loading);

        let state = Rc::new(RefCell::new(Inner::new()));
        // Install the depth annotator so any descendant consumer
        // events bubbling through us are credited one level of
        // nesting. The handle's `Drop` detaches the listeners.
        state.borrow_mut().depth_annotator = Some(install_depth_annotator(&host));
        // Install reset / update / error JS methods on the host
        // so the tonk-host invokes them by name when frames arrive.
        install_method_delegates(&host, &state);
        *self.inner.borrow_mut() = Some(state.clone());
        start_flows(&host, state);
    }

    fn disconnected_callback(&mut self, _this: &HtmlElement) {
        if let Some(state) = self.inner.borrow_mut().take() {
            let mut inner = state.borrow_mut();
            inner.disposed = true;
            inner.abort_all();
        }
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
        let host: Element = this.clone().into();
        let Some(state) = self.inner.borrow().clone() else {
            return;
        };

        // Only a subject input — `entity`/`model`/`view` — changes what
        // this display resolves (a different entity, model, or view
        // template), so only those tear down and restart the
        // resolve/subscribe flow. Every other observed attribute is
        // host-context (e.g. `data-active`, threaded in for a template to
        // read as `{dom.host/<attr>}`): it leaves the resolved template
        // unchanged and only alters a value projected into the mounted
        // view, so it is propagated in place — re-augment the cached
        // frame and replay it through the existing slides, which the
        // `<tonk-view>` renderer diffs into the DOM without a teardown.
        if !resolves_template(&name) {
            replay_host_attributes(&host, &state);
            return;
        }

        {
            let mut s = state.borrow_mut();
            s.abort_all();
            s.last_frame = Vec::new();
            // Drop retained per-tag sets too: a restart re-subscribes and
            // the first frame per tag is a fresh snapshot, so stale rows
            // from the prior context must not survive as a delta base.
            s.retained.clear();
            // Tear down any mounted slide / carousel chrome so the
            // restart starts from a clean host.
            clear_host(&host, &mut s);
        }
        state::set(&host, State::Loading);
        start_flows(&host, state);
    }
}

/// Public entry point — registers the element with the page.
pub fn register() {
    if already_registered() {
        return;
    }
    TonkDisplay::define("tonk-display");
    install_method_shims();
}

/// Install the `reset` / `update` / `error` method shims on the
/// `<tonk-display>` prototype. Each shim reads the per-instance
/// `__tonkReset` / `__tonkUpdate` / `__tonkError` property
/// (a Rust closure attached in `connected_callback`) and calls
/// it with the payload and opts. Installing the shim on the
/// prototype rather than on each instance keeps `this`-binding
/// correct (a `wasm_bindgen::Closure` would have given us a
/// plain function with no `this`).
fn install_method_shims() {
    let Some(win) = window() else {
        return;
    };
    let constructor = win.custom_elements().get("tonk-display");
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
/// / `__tonkError` on the element. The closures capture the
/// element's `Inner` state and dispatch on `opts.tag` to the
/// right handler.
fn install_method_delegates(host: &Element, state: &Rc<RefCell<Inner>>) {
    use wasm_bindgen::closure::Closure;
    let host_for_reset = host.clone();
    let state_for_reset = state.clone();
    let reset: Closure<dyn FnMut(JsValue, JsValue)> =
        Closure::wrap(Box::new(move |payload, opts| {
            on_reset(&host_for_reset, &state_for_reset, payload, opts);
        }));
    let _ = Reflect::set(host, &"__tonkReset".into(), reset.as_ref());
    reset.forget();

    let host_for_update = host.clone();
    let state_for_update = state.clone();
    let update: Closure<dyn FnMut(JsValue, JsValue)> =
        Closure::wrap(Box::new(move |payload, opts| {
            on_update(&host_for_update, &state_for_update, payload, opts);
        }));
    let _ = Reflect::set(host, &"__tonkUpdate".into(), update.as_ref());
    update.forget();

    let host_for_error = host.clone();
    let state_for_error = state.clone();
    let error: Closure<dyn FnMut(JsValue, JsValue)> =
        Closure::wrap(Box::new(move |payload, opts| {
            on_error(&host_for_error, &state_for_error, payload, opts);
        }));
    let _ = Reflect::set(host, &"__tonkError".into(), error.as_ref());
    error.forget();
}

/// `reset(conclusions, { tag })` — first / full-snapshot frame.
/// V1 SW always emits `reset`; deltas (`update`) come later with
/// DBSP integration.
fn on_reset(host: &Element, state: &Rc<RefCell<Inner>>, payload: JsValue, opts: JsValue) {
    let tag = read_tag(&opts);
    let conclusions: Vec<Conclusion> = match serde_wasm_bindgen::from_value(payload) {
        Ok(v) => v,
        Err(e) => {
            fail(
                host,
                state,
                ErrorDetail::new(ErrorKind::Parse, format!("reset payload: {e}")),
            );
            return;
        }
    };
    let reconnect = opts.is_object()
        && Reflect::get(&opts, &"reconnect".into())
            .map(|v| v.is_truthy())
            .unwrap_or(false);
    // Retain this tag's full set so a later `update` delta applies to the
    // right base (a `reset` replaces it wholesale).
    if let Some(tag) = &tag {
        state
            .borrow_mut()
            .retained
            .insert(tag.clone(), conclusions.clone());
    }
    match tag.as_deref() {
        Some("model") => handle_model_frame(host, state, conclusions),
        Some("view") => handle_view_frame(host, state, conclusions),
        Some("entity") => handle_entity_frame(host, state, conclusions, reconnect),
        _ => {
            // Unknown tag — log and drop.
        }
    }
}

/// `update({ asserted, retracted }, { tag })` — incremental change.
///
/// Apply the delta to the retained flat frame (`last_frame`) by
/// conclusion identity — drop each `retracted` row, append each
/// `asserted` row — then route the merged full set through the same
/// tag handler `reset` uses. Reusing the reset path keeps a delta
/// exactly equivalent to the snapshot it would have produced, so no
/// rendering logic is duplicated.
fn on_update(host: &Element, state: &Rc<RefCell<Inner>>, payload: JsValue, opts: JsValue) {
    let tag = read_tag(&opts);
    let asserted = read_conclusions(&payload, "asserted");
    let retracted = read_conclusions(&payload, "retracted");
    let (asserted, retracted) = match (asserted, retracted) {
        (Ok(a), Ok(r)) => (a, r),
        (Err(e), _) | (_, Err(e)) => {
            fail(
                host,
                state,
                ErrorDetail::new(ErrorKind::Parse, format!("update payload: {e}")),
            );
            return;
        }
    };

    // Apply the delta to THIS tag's retained set (not `last_frame`, which
    // is the entity frame only) and re-store it, so a model/view delta
    // resolves against model/view rows rather than the entity frame.
    let Some(tag) = tag else {
        // A delta with no tag has no base to apply to — drop it.
        return;
    };
    let merged = {
        let mut s = state.borrow_mut();
        let base = s.retained.get(&tag).cloned().unwrap_or_default();
        let merged = apply_delta(base, &asserted, retracted);
        s.retained.insert(tag.clone(), merged.clone());
        merged
    };

    match tag.as_str() {
        "model" => handle_model_frame(host, state, merged),
        "view" => handle_view_frame(host, state, merged),
        "entity" => handle_entity_frame(host, state, merged, false),
        _ => {
            // Unknown tag — log and drop.
        }
    }
}

/// Apply a delta to a retained frame, healing over base drift while
/// preserving legitimate multi-row (directory) entities.
///
/// The reactor emits a *per-entity diff*: for each entity it touched
/// it asserts the rows that entered and retracts the rows that left
/// that entity's projection (`dialog-repository` subscription
/// `maintain` — over-delete the entity's retained rows, re-derive,
/// diff). An entity's *unchanged* rows are neither asserted nor
/// retracted, so a directory frame legitimately keeps N rows for one
/// `this` (one per tuple of a cardinality-many field) across a delta
/// that only supersedes one of them.
///
/// Two things must hold at once:
/// - **Value-equality retract** (the common case): drop each
///   `retracted` row from the base by value. When the base still
///   holds what the retract names, this exactly supersedes a
///   cardinality-one field and preserves the untouched tuples of a
///   multi-row entity (the retract *matches* the changed tuple only).
/// - **Drift heal** (the counter bug): if the consumer missed or
///   reset a frame, its retained row for an entity can have *drifted*
///   from what the delta retracts (base `count:5`, delta retracts
///   `count:4`, asserts `count:6`). The value retract then matches
///   nothing and a naive append leaves two rows for one `this`, which
///   the downstream group-by-`this` fold collapses into a garbled
///   multi-valued field. So: for any `this` that the delta *both*
///   asserts a row for *and* has an unmatched retract for (the
///   supersession-under-drift signal), drop that entity's surviving
///   base rows before appending — the assert carries the entity's
///   fresh row-set.
///
/// The heal is scoped by the *unmatched* retract: an entity whose
/// retracts all matched (a directory tuple supersession, or a pure
/// addition with no retract) keeps its other rows untouched, so
/// multi-row directory mode is preserved. The result is the full set
/// the equivalent `Snapshot` frame would carry, keeping a delta and a
/// snapshot interchangeable downstream.
fn apply_delta(
    mut rows: Vec<Conclusion>,
    asserted: &[Conclusion],
    retracted: Vec<Conclusion>,
) -> Vec<Conclusion> {
    // Value-equality retract, recording which retracts found no
    // matching base row. An unmatched retract for a `this` the delta
    // also asserts is the drift signal: the base row for that entity
    // is stale relative to the delta's view of it.
    let mut drifted: BTreeSet<String> = retracted.iter().map(|r| r.this.clone()).collect();
    rows.retain(|row| {
        if retracted.contains(row) {
            // This retract matched a live row — not drift for this
            // entity (a clean supersession or a directory-tuple
            // removal), so it must not trigger a heal that would drop
            // the entity's other rows.
            drifted.remove(&row.this);
            false
        } else {
            true
        }
    });

    // Entities the delta asserts a fresh row-set for. A drifted base
    // row for one of these is superseded by the assert.
    let asserted_entities: BTreeSet<&str> = asserted.iter().map(|c| c.this.as_str()).collect();
    rows.retain(|row| {
        !(drifted.contains(&row.this) && asserted_entities.contains(row.this.as_str()))
    });

    rows.extend(asserted.iter().cloned());
    rows
}

/// Read `payload[field]` as a `Vec<Conclusion>` for a delta frame.
/// `JSON.parse`-produced values carry `fields` as a plain object (not
/// a JS `Map`), so `serde_wasm_bindgen` reads them back correctly.
fn read_conclusions(payload: &JsValue, field: &str) -> Result<Vec<Conclusion>, String> {
    let value = Reflect::get(payload, &field.into()).map_err(|e| format!("{e:?}"))?;
    if value.is_undefined() || value.is_null() {
        return Ok(Vec::new());
    }
    serde_wasm_bindgen::from_value(value).map_err(|e| e.to_string())
}

/// `error(detail, { tag })` — transport / parse error on a
/// subscription.
///
/// A transport interruption (the SW restarting or releasing streams on
/// update, a network blip) is NOT a render failure: the rendered content
/// stays — replacing it with a callout would throw away a perfectly good
/// view over a hiccup — and the host is stamped `data-state="offline"` so
/// chrome can dim or badge it. The host-side retry reconnects the
/// subscription, and the next frame heals the state to `ready`. Only a
/// real refusal (HTTP 403 → `unauthorized`) replaces content via the loud
/// path.
fn on_error(host: &Element, state: &Rc<RefCell<Inner>>, payload: JsValue, _opts: JsValue) {
    let message = Reflect::get(&payload, &"message".into())
        .ok()
        .and_then(|v| v.as_string())
        .unwrap_or_else(|| format!("{payload:?}"));
    // Carry the HTTP status across the boundary when the producer set
    // one: it is what separates a retryable hiccup from a settled
    // answer, and rebuilding the detail from `message` alone would
    // discard it and read every failure as `offline`.
    let err = match Reflect::get(&payload, &"status".into())
        .ok()
        .and_then(|v| v.as_f64())
    {
        Some(status) => ErrorDetail::http(status as u16, message),
        None => ErrorDetail::new(ErrorKind::Network, message),
    };
    if loud_state(&err) == State::Offline {
        state::set(host, State::Offline);
        dispatch_error(host, err);
        return;
    }
    fail(host, state, err);
}

/// Read `opts.tag` as a string. Returns `None` if absent.
fn read_tag(opts: &JsValue) -> Option<String> {
    if !opts.is_object() {
        return None;
    }
    Reflect::get(opts, &"tag".into())
        .ok()
        .and_then(|v| v.as_string())
}

fn already_registered() -> bool {
    let Some(win) = window() else {
        return false;
    };
    !win.custom_elements().get("tonk-display").is_undefined()
}

fn start_flows(host: &Element, state: Rc<RefCell<Inner>>) {
    // Bump the generation so any already-spawned flow chains
    // running concurrently bail at their next generation check
    // instead of overwriting our state.
    let generation = {
        let mut s = state.borrow_mut();
        s.generation = s.generation.wrapping_add(1);
        s.generation
    };
    let host_clone = host.clone();
    let state_clone = state.clone();
    spawn_local(async move {
        if let Err(err) = run(&host_clone, state_clone.clone(), generation).await {
            // Only surface the error if we're still the current
            // generation — a stale flow's failure (e.g. aborted
            // fetch) shouldn't overwrite a healthy newer flow's
            // state.
            if state_clone.borrow().generation == generation {
                fail(&host_clone, &state_clone, err);
            }
        }
    });
}

/// Transition the host into the error state: tear down any mounted
/// slides (so the error callout isn't sitting beside a half-rendered
/// template), surface the failure as a `<wa-callout>` inside the
/// host, and dispatch the `tonk-display:error` event so listeners
/// (page-level diagnostics, analytics, etc.) still get the
/// structured detail.
fn fail(host: &Element, state: &Rc<RefCell<Inner>>, err: ErrorDetail) {
    {
        let mut inner = state.borrow_mut();
        clear_host(host, &mut inner);
    }
    let loud = loud_state(&err);
    state::set_error(host, loud, state::state_title(loud, err.kind), &err.message);
    dispatch_error(host, err);
}

/// Classify an `ErrorDetail` into the loud state it drives. A network
/// error carries the HTTP status when one was reported, and the status
/// says which failure this is: `403` is `unauthorized` (the branch
/// exists, this device may not read it) and `404` is `unknown` (there
/// is no such repository here at all). Anything else — a dropped
/// connection, a restarting worker, a `500` — is `offline`, the
/// state that keeps retrying.
///
/// Falls back to matching the message for a status-less `403` so an
/// error minted before the status was carried still classifies.
fn loud_state(err: &ErrorDetail) -> State {
    if err.kind != ErrorKind::Network {
        return state::state_for(err.kind);
    }
    match err.status {
        Some(403) => State::Unauthorized,
        Some(404) => State::Unknown,
        Some(_) => State::Offline,
        None if err.message.contains("HTTP 403") => State::Unauthorized,
        None => State::Offline,
    }
}

/// Empty the host's rendered output and forget every mounted slide /
/// chrome — but **preserve the embedder's state-slot children**
/// (`[slot]`), which are authored input, not rendered output. Without
/// this guard `set_inner_html("")` would delete the embedder's
/// `slot="no-model"` etc. on a flow restart, so a later absence state
/// couldn't find them and would fall back to the built-in callout even
/// though the embedder provided content.
fn clear_host(host: &Element, inner: &mut Inner) {
    inner.slides.clear();
    inner.carousel = None;
    inner.notation_source = None;
    inner.notation_item = None;
    remove_rendered_children(host);
}

/// Remove every direct child of `host` that the renderer mounted,
/// keeping the embedder's authored `[slot]` children. Replaces a blanket
/// `set_inner_html("")` so state-slot content survives a restart.
fn remove_rendered_children(host: &Element) {
    // Walk the element children once, collecting the rendered ones (no
    // `slot` attribute) before removing — removing mid-walk would break
    // the sibling chain.
    let mut to_remove: Vec<Element> = Vec::new();
    let mut cursor = host.first_element_child();
    while let Some(child) = cursor {
        cursor = child.next_element_sibling();
        if !child.has_attribute("slot") {
            to_remove.push(child);
        }
    }
    for child in to_remove {
        child.remove();
    }
}

/// Bail if this flow's generation has been superseded or the
/// element this state belongs to has been disconnected. Either
/// condition means our async chain has lost the right to mutate
/// the host — a newer flow is in charge (or no flow is, if the
/// element is gone). Returning Err here causes the caller to
/// short-circuit without touching the DOM.
fn check_generation(state: &Rc<RefCell<Inner>>, generation: u64) -> Result<(), ErrorDetail> {
    let s = state.borrow();
    if s.disposed || s.generation != generation {
        Err(ErrorDetail::new(ErrorKind::Descriptor, "superseded"))
    } else {
        Ok(())
    }
}

/// Like [`check_generation`] but for the downstream flow restarted by
/// each model frame: bail if the element was disposed or a newer model
/// frame bumped `downstream_generation` while this chain awaited.
fn check_downstream(
    state: &Rc<RefCell<Inner>>,
    downstream_generation: u64,
) -> Result<(), ErrorDetail> {
    let s = state.borrow();
    if s.disposed || s.downstream_generation != downstream_generation {
        Err(ErrorDetail::new(ErrorKind::Descriptor, "superseded"))
    } else {
        Ok(())
    }
}

/// Phase 1: validate the attributes and open the **model**
/// subscription. The model resolve is a live subscription (not a
/// one-shot) so a concept seeded *after* the element mounts pushes a
/// frame and the display leaves `no-model` without a reload. Each
/// model frame routes to `handle_model_frame`, which (re)starts the
/// downstream view + entity flow ([`start_downstream`]).
///
/// Attribute errors (bad `entity`, missing `model`, carousel) surface
/// here as `malformed`; an *absent* model concept is not an error —
/// it is the `no-model` steady state the subscription recovers from.
async fn run(
    host: &Element,
    state: Rc<RefCell<Inner>>,
    generation: u64,
) -> Result<(), ErrorDetail> {
    // `entity` is optional. Present → single mode: render that one
    // entity. Absent → directory mode: render every instance of the
    // model (`this` unbound) through the model's directory view.
    let entity = host.get_attribute("entity").filter(|s| !s.is_empty());
    if let Some(entity) = &entity
        && !looks_like_uri(entity)
    {
        return Err(ErrorDetail::new(
            ErrorKind::Descriptor,
            "`entity` must be an entity URI (contain `:`)",
        ));
    }

    // The `view` attribute names a view *concept* (named or URI);
    // when omitted, the built-in `view` concept (`tonk:view`) is
    // used. The `model` attribute names the subject's model concept
    // and constrains which view row to render.
    let view = host.get_attribute("view").filter(|s| !s.is_empty());

    // `view="about:blank"` is the carousel sentinel: enumerate every
    // view defined for the model. This is the two-step flow from the
    // design (step 1: query the `{model}` display contract for all
    // conforming view kinds; step 2: resolve each kind's template).
    // Step 2 is not built yet, so carousel surfaces a clear error
    // rather than rendering a half-resolved frame.
    if view.as_deref() == Some(CAROUSEL_VIEW) {
        return Err(ErrorDetail::new(
            ErrorKind::Descriptor,
            "carousel view (view=\"about:blank\") is not implemented yet",
        ));
    }

    // `model` is required: it both projects the subject's fields and
    // constrains the view query.
    let model = host
        .get_attribute("model")
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            ErrorDetail::new(
                ErrorKind::Descriptor,
                "<tonk-display> requires a `model` attribute",
            )
        })?;

    // Resolve the model name to its concept URI (one-shot — a bookmark
    // name rarely lands late; an attribute change restarts the flow),
    // then *subscribe* to its phase-1 concept query so a concept seeded
    // after mount pushes a frame. The empty frame is `no-model`, not a
    // hard error — `handle_model_frame` keeps the subscription and
    // starts the downstream flow once the concept lands.
    let model_q = resolve_model_query(host, &model).await?;
    check_generation(&state, generation)?;
    let model_body = to_body(&model_q)?;
    let model_tag = JsValue::from_str("model");
    let model_sub = host_consumer::subscribe_claimed(host, &model_body, Some(&model_tag)).await?;
    {
        let mut s = state.borrow_mut();
        if s.generation != generation {
            return Err(ErrorDetail::new(ErrorKind::Descriptor, "superseded"));
        }
        s.model_sub = Some(model_sub);
    }
    Ok(())
}

/// Phase 2: with the model concept resolved (`model_entity` +
/// `descriptor_json`), open the view + entity subscriptions. Invoked
/// from `handle_model_frame` on every non-empty model frame, so a
/// branch revision that (re)defines the model — or a view that lands
/// later — re-runs this against the freshest descriptor. Tears down
/// the prior view/entity subscriptions first and bumps
/// `downstream_generation` so a stale chain bails.
async fn start_downstream(
    host: &Element,
    state: Rc<RefCell<Inner>>,
    model_entity: String,
    descriptor_json: String,
    downstream_generation: u64,
) -> Result<(), ErrorDetail> {
    let entity = host.get_attribute("entity").filter(|s| !s.is_empty());
    let directory = entity.is_none();
    let view = host.get_attribute("view").filter(|s| !s.is_empty());

    // Resolve the view *concept*'s descriptor — the query predicate.
    // When `view` is omitted, the built-in `view` concept's
    // descriptor is known (`view_predicate`), so no resolve is
    // needed; a named/URI view concept is resolved from the branch.
    // The descriptor maps the `display` field name to whatever
    // attribute that view kind declares it under, so `view_by_model`
    // reads the right template regardless of the kind.
    let view_descriptor: serde_json::Value = match view.as_deref() {
        // No explicit `view`: the built-in detail view (`tonk:view`) in
        // single mode, or the directory view (`tonk:view/directory`) in
        // directory mode. Both fall back to their `_:_` default when
        // the model has no specific view of that kind.
        None if directory => directory_view_predicate(),
        None => view_predicate(),
        Some(view_ref) => {
            // An explicit `view` whose concept is not on the branch is
            // `no-view`, not a hard error: a recoverable absence the
            // model subscription leaves once the view concept lands and
            // the next model frame re-runs this. Other resolve failures
            // still propagate.
            match resolve_model(host, view_ref).await {
                Ok((_, view_descriptor_json)) => {
                    check_downstream(&state, downstream_generation)?;
                    serde_json::from_str(&view_descriptor_json).map_err(|e| {
                        ErrorDetail::new(ErrorKind::Descriptor, format!("view descriptor: {e}"))
                    })?
                }
                Err(err) if err.kind == ErrorKind::UnknownSource => {
                    let model = host.get_attribute("model").unwrap_or_default();
                    state::set_absence(
                        host,
                        State::NoView,
                        "View not found",
                        &format!(
                            r#"view:
  this: {view_ref}
  model: {model}"#
                        ),
                    );
                    return Ok(());
                }
                Err(err) => return Err(err),
            }
        }
    };
    let view_q = view_by_model_query(&view_descriptor, &model_entity).map_err(|e| {
        ErrorDetail::new(ErrorKind::Descriptor, format!("view-by-model query: {e}"))
    })?;
    let view_body = to_body(&view_q)?;

    // Single mode pins `this` to the entity; directory mode leaves
    // `this` unbound so the query matches every instance of the model.
    let entity_q = match &entity {
        Some(entity) => entity_query(&descriptor_json, entity),
        None => instances_query(&descriptor_json),
    }
    .map_err(|e| ErrorDetail::new(ErrorKind::Descriptor, format!("entity query: {e}")))?;
    let entity_body = to_body(&entity_q)?;

    // A fresh model frame re-runs the downstream flow: tear down the
    // prior view/entity subscriptions and any mounted slide so we
    // remount cleanly against the freshest descriptor.
    {
        let mut s = state.borrow_mut();
        s.view_sub.take();
        s.entity_sub.take();
        clear_host(host, &mut s);
    }

    // The view query is model-constrained, so it resolves to one
    // presentation. Mount in single mode.
    ensure_carousel(host, &state, true);

    // Open the view subscription via the host. Frames arrive through
    // our `__tonkReset` delegate, which routes to `handle_view_frame`
    // (or `handle_entity_frame`) by `opts.tag`.
    let view_tag = JsValue::from_str("view");
    let view_sub = host_consumer::subscribe_claimed(host, &view_body, Some(&view_tag)).await?;
    check_downstream(&state, downstream_generation)?;

    let entity_tag = JsValue::from_str("entity");
    let entity_sub =
        host_consumer::subscribe_claimed(host, &entity_body, Some(&entity_tag)).await?;

    {
        let mut s = state.borrow_mut();
        // Final generation check — if a newer model frame restarted the
        // downstream flow between opening the subscriptions, drop them
        // so we don't orphan them (the newer flow's are already stored).
        if s.downstream_generation != downstream_generation {
            return Err(ErrorDetail::new(ErrorKind::Descriptor, "superseded"));
        }
        s.view_sub = Some(view_sub);
        s.entity_sub = Some(entity_sub);
        s.directory = directory;
        // Retained for the `_:_` default-view fallback: if the
        // model-specific view frame is empty, `handle_view_frame`
        // re-queries the same view concept with `model = _:_`.
        s.view_descriptor = Some(view_descriptor.clone());
        s.model_entity = Some(model_entity.clone());
        // Context handed to a `<tonk-portal>` if a view frame routes
        // here in portal mode: the subject's model entity (the
        // bridge's `context.model`) and its descriptor (so the bridge
        // can build its own entity query). A text/html view's iframe
        // fetches data itself, so the entity subscription above just
        // no-ops against the portal (it has no `draw`).
        s.portal_descriptor = Some(descriptor_json);
        s.portal_model = Some(model_entity);
    }
    dispatch_event(host, "tonk-display:connected", None);
    Ok(())
}

/// Decode the host's phase-1 result (a `Vec<Conclusion>` as a JS
/// value) into the `(entity, descriptor_json)` tuple the rest of
/// the flow expects. Phase-1 returns 0 or 1 row.
fn extract_phase1(value: &JsValue) -> Result<(String, String), ErrorDetail> {
    let conclusions: Vec<Conclusion> = serde_wasm_bindgen::from_value(value.clone())
        .map_err(|e| ErrorDetail::new(ErrorKind::Parse, format!("phase1 result: {e}")))?;
    let first = conclusions
        .into_iter()
        .next()
        .ok_or_else(|| ErrorDetail::new(ErrorKind::UnknownSource, "no concept matched"))?;
    let source = ipld_str(first.fields.get("source"))
        .map(str::to_owned)
        .ok_or_else(|| {
            ErrorDetail::new(ErrorKind::Descriptor, "phase1 row missing `source` field")
        })?;
    Ok((first.this, source))
}

/// Read an `Ipld::String` value as `&str` and return `None`
/// for any other variant (or for `None`).
fn ipld_str(value: Option<&Ipld>) -> Option<&str> {
    match value? {
        Ipld::String(s) => Some(s.as_str()),
        _ => None,
    }
}

/// Serialize a wire query to the JS body the host bridge expects.
fn to_body(query: &Query) -> Result<JsValue, ErrorDetail> {
    serde_wasm_bindgen::to_value(query)
        .map_err(|e| ErrorDetail::new(ErrorKind::Parse, format!("query body: {e}")))
}

/// Pull a string field off the first conclusion of a one-shot query
/// result (a `Vec<Conclusion>` serialized as a JS value).
///
/// `Err` is a decode failure — the result wasn't the
/// `Vec<Conclusion>` shape we expect (a wire/protocol mismatch),
/// which is distinct from a genuinely empty or field-less result.
/// `Ok(None)` means the result was empty or the field is absent /
/// non-string. Keeping the two apart stops a decode bug from
/// masquerading as a "not found" further up the chain.
fn first_field(value: &JsValue, field: &str) -> Result<Option<String>, ErrorDetail> {
    let conclusions: Vec<Conclusion> = serde_wasm_bindgen::from_value(value.clone())
        .map_err(|e| ErrorDetail::new(ErrorKind::Parse, format!("query result: {e}")))?;
    Ok(conclusions
        .into_iter()
        .next()
        .and_then(|c| ipld_str(c.fields.get(field)).map(str::to_owned)))
}

/// Resolve a concept (bookmark name or entity URI) to its
/// `(entity, descriptor_json)`.
///
/// A bare name is first resolved through the **Name concept**
/// (`id:<name>` → `db.name/referent`) into a concept entity URI,
/// then that URI drives the Phase-1 concept-of-concepts lookup by
/// `this`. This is the path that makes a model or view named after a
/// concept pinned to a `this:` URI (e.g. `workspace` → `tonk:workspace`)
/// resolve — the concept carries a `Name` claim but no
/// `db.meta/name`, so a direct `name`-filtered Phase-1 would miss
/// it. A value that is already a URI skips name resolution.
async fn resolve_model(host: &Element, source: &str) -> Result<(String, String), ErrorDetail> {
    let phase1_q = resolve_model_query(host, source).await?;
    let result = host_consumer::query(host, &to_body(&phase1_q)?).await?;
    extract_phase1(&result)
}

/// Build the phase-1 concept query for `source` *without executing it*.
///
/// Splits the front half of [`resolve_model`]: a bare bookmark name is
/// resolved through the Name concept (`id:<name>` →
/// `db.name/referent`) into a concept URI (a one-shot `query`),
/// then `phase1_query` is built from the resolved `ParsedSource`. The
/// caller decides whether to run it once or open a subscription on it —
/// the model link subscribes so a late-seeded concept recovers.
async fn resolve_model_query(host: &Element, source: &str) -> Result<Query, ErrorDetail> {
    let parsed: ParsedSource = parse_source(source);
    let parsed = if parsed.is_uri() {
        parsed
    } else {
        // Resolve the bookmark name to its referent URI via the Name
        // concept. An unresolved name falls through with the original
        // string, so Phase-1 still reports a clean "no concept matched".
        let name_result =
            host_consumer::query(host, &to_body(&name_query(&parsed.name_or_uri))?).await?;
        match first_field(&name_result, "entity")? {
            Some(uri) => ParsedSource {
                name_or_uri: uri,
                filters: parsed.filters,
            },
            None => parsed,
        }
    };
    Ok(phase1_query(&parsed))
}

/// Route a `"model"` subscription frame. The model resolve is a live
/// subscription, so this fires on every branch revision touching the
/// concept-of-concepts view:
///
/// - **empty frame** → `no-model` (the concept is not on the branch
///   *yet*). No teardown; the subscription stays open and recovers the
///   instant the concept lands. This is the fix for the latched red box
///   on a still-seeding fresh space.
/// - **resolved row** → bump `downstream_generation` and (re)start the
///   downstream view + entity flow against the freshest descriptor, so
///   a model (re)definition or a late-landing view is picked up.
fn handle_model_frame(host: &Element, state: &Rc<RefCell<Inner>>, conclusions: Vec<Conclusion>) {
    let resolved = extract_phase1_conclusion(conclusions);
    let Some((model_entity, descriptor_json)) = resolved else {
        // The concept is not on the branch yet. Stay subscribed; the
        // embedder may skin `no-model` via a `slot="no-model"` child,
        // otherwise the built-in fallback names the missing model.
        let model = host.get_attribute("model").unwrap_or_default();
        state::set_absence(
            host,
            State::NoModel,
            "Model not found",
            &format!(
                r#"concept:
  this: {model}"#
            ),
        );
        return;
    };
    let downstream_generation = {
        let mut s = state.borrow_mut();
        if s.disposed {
            return;
        }
        // The model subscription re-pushes on every branch revision,
        // including plain entity-data writes. Restart the downstream
        // flow only when the *resolved concept* actually changed —
        // otherwise an unrelated write would tear down and remount the
        // view/entity subscriptions and wipe the rendered rows. The
        // entity subscription already streams data updates in place.
        //
        // The comparison is against `resolved_model`, set synchronously
        // here (not `start_downstream`'s async tail), so a burst of model
        // frames during one write doesn't each see a stale value and
        // remount.
        //
        // Compare the descriptor as a *parsed* value: the worker emits
        // the same concept's `source` with non-deterministic key ordering
        // between revisions, so a raw-string compare reports a spurious
        // change on every unrelated write and tears the list down. Parsed
        // equality is order-independent. A descriptor that fails to parse
        // falls back to the raw string wrapped as a JSON string, so it
        // still compares.
        let next_descriptor = serde_json::from_str::<serde_json::Value>(&descriptor_json)
            .unwrap_or_else(|_| serde_json::Value::String(descriptor_json.clone()));
        let next = (model_entity.clone(), next_descriptor);
        if s.resolved_model.as_ref() == Some(&next) {
            return;
        }
        s.resolved_model = Some(next);
        s.downstream_generation = s.downstream_generation.wrapping_add(1);
        s.downstream_generation
    };
    let host = host.clone();
    let state = state.clone();
    spawn_local(async move {
        if let Err(err) = start_downstream(
            &host,
            state.clone(),
            model_entity,
            descriptor_json,
            downstream_generation,
        )
        .await
        {
            // Only surface if still current — a stale downstream chain's
            // failure (a `check_downstream` "superseded", or a real
            // error a newer model frame has since replaced) shouldn't
            // overwrite a healthy newer flow's state. The generation
            // guard covers both: a superseded chain fails the check.
            if state.borrow().downstream_generation == downstream_generation {
                fail(&host, &state, err);
            }
        }
    });
}

/// Decode a phase-1 subscription frame (already-deserialized
/// `Vec<Conclusion>`) into `(model_entity, descriptor_json)`. Returns
/// `None` for an empty frame or a row missing the `source` descriptor —
/// both are the `no-model` steady state, not a hard error.
fn extract_phase1_conclusion(conclusions: Vec<Conclusion>) -> Option<(String, String)> {
    let first = conclusions.into_iter().next()?;
    let source = ipld_str(first.fields.get("source")).map(str::to_owned)?;
    Some((first.this, source))
}

/// Key each incoming view-frame conclusion by its view entity
/// (`this`), paired with the `display` template. The view query is
/// model-constrained, so each row is a distinct view instance keyed
/// by its own entity URI. A conclusion with no `display` string is
/// dropped.
fn slide_keys(conclusions: Vec<Conclusion>) -> BTreeMap<String, String> {
    conclusions
        .into_iter()
        .filter_map(|c| {
            let display = ipld_str(c.fields.get("display")).map(str::to_owned)?;
            Some((c.this, display))
        })
        .collect()
}

/// Diff the incoming view frame against currently mounted slides.
/// Slides are keyed by the view entity URI; we add/remove/replace as
/// needed, then push the cached entity conclusion into any fresh
/// slide so it has data to render right away.
fn handle_view_frame(host: &Element, state: &Rc<RefCell<Inner>>, conclusions: Vec<Conclusion>) {
    let mut s = state.borrow_mut();

    // A view whose projected `type` is `text/html` is a full HTML
    // document mounted into a `<tonk-portal>` — whose bridge fetches
    // the entity's own data through the live `tonk` object — rather
    // than interpolated inline. `type` rides the view query only when
    // the view concept declares it (see `view_by_model_query`), so an
    // ordinary view never trips this; a `display` change just reloads
    // the portal's `content` in place.
    // No model-specific view resolved: fall back to the `_:_` default
    // view (a directory carousel, or the notation dump for a detail
    // view). Re-query the same view concept with `model = _:_` and
    // render its result. The model-specific subscription stays live, so
    // if a view for this model is defined later its frame is non-empty
    // and the branch below mounts it, replacing the default. Skip if
    // we're already showing the default (avoid re-querying every empty
    // frame).
    if conclusions.is_empty() {
        let need_default = !s.default_slide;
        drop(s);
        if need_default {
            spawn_default_view(host, state);
        }
        return;
    }
    // A model-specific view arrived: it wins over any default slide.
    // Clearing the flag lets the normal slide reconciliation below
    // replace the default `<tonk-view>` (keyed differently) and the
    // stale default slide is dropped as a vanished key.
    s.default_slide = false;

    if conclusions
        .iter()
        .any(|c| ipld_str(c.fields.get("type")) == Some("text/html"))
    {
        handle_portal_view_frame(host, &mut s, conclusions);
        drop(s);
        dispatch_event(host, "tonk-display:template", Some(JsValue::from_str("ok")));
        return;
    }

    let incoming = slide_keys(conclusions);

    // Remove vanished slides.
    let stale: Vec<String> = s
        .slides
        .keys()
        .filter(|k| !incoming.contains_key(*k))
        .cloned()
        .collect();
    for name in stale {
        if let Some(slide) = s.slides.remove(&name)
            && let Some(parent) = slide.item.parent_node()
        {
            let _: Result<Node, _> = parent.remove_child(&slide.item);
        }
    }

    // Add or replace slides.
    let cached = s.last_frame.clone();
    // Always replay the cached frame to a freshly mounted slide — even
    // when it is empty — so the slide's chrome (e.g. a fallback region)
    // renders right away on a cold-empty collection rather than staying
    // blank until an entity frame that has already passed. The WHOLE
    // frame is replayed: directory mode has one conclusion per instance,
    // so a single-conclusion replay would drop every instance but the
    // first.
    let cached_detail = augmented_detail(host, &cached, s.directory);
    for (name, display) in incoming {
        let existing_matches = s
            .slides
            .get(&name)
            .map(|slide| slide.display == display)
            .unwrap_or(false);
        if existing_matches {
            continue;
        }
        // Drop the prior slide for this name if its template
        // changed — we replace whole `<tonk-view>` rather than
        // template-swap-in-place, per the design.
        if let Some(slide) = s.slides.remove(&name)
            && let Some(parent) = slide.item.parent_node()
        {
            let _: Result<Node, _> = parent.remove_child(&slide.item);
        }
        if let Some(new_slide) = mount_view_slide(host, &mut s, &display) {
            call_render(&new_slide.view_el, &cached_detail);
            s.slides.insert(name, new_slide);
        }
    }

    // In single mode, mark Ready once the slide is mounted (with
    // or without an entity frame yet — empty content is fine).
    // In carousel mode, ensure the notation slide is fed.
    if !cached.is_empty() && !s.slides.is_empty() {
        state::set(host, State::Ready);
    }
    if let Some(c) = cached.first() {
        update_notation(host, &s, c);
    }

    // Drop the borrow before spawning the delegate refresh — its
    // descriptor resolves are async and reacquire the state.
    drop(s);
    schedule_delegate_refresh(host, state);
    refresh_host_watch(host, state);

    dispatch_event(host, "tonk-display:template", Some(JsValue::from_str("ok")));
}

/// The sentinel model entity for a default view: a `view!`/
/// `view/directory!` declared with `model: tonk:_` matches any model
/// that has no specific view of that kind. `tonk:_` is the
/// wildcard-model entity seeded by core.yaml. See
/// `tonk-core/docs/templates.md`.
const DEFAULT_MODEL: &str = "tonk:_";

/// Query the `_:_` default view (same view concept, `model = _:_`) and
/// mount its template as the default slide. Spawned from
/// `handle_view_frame` when the model-specific view frame is empty. The
/// model-specific subscription stays live; a later non-empty frame
/// replaces this default (see the `default_slide` flag).
fn spawn_default_view(host: &Element, state: &Rc<RefCell<Inner>>) {
    let descriptor = {
        let s = state.borrow();
        s.view_descriptor.clone()
    };
    let Some(descriptor) = descriptor else { return };

    let host = host.clone();
    let state = state.clone();
    spawn_local(async move {
        // If the default view can't be built or resolves to nothing,
        // settle to Empty rather than leaving the element on `loading`
        // forever — there is genuinely no presentation for this model
        // (not even the `_:_` default is seeded).
        let resolved = async {
            let query = view_by_model_query(&descriptor, DEFAULT_MODEL).ok()?;
            let body = to_body(&query).ok()?;
            let result = host_consumer::query(&host, &body).await.ok()?;
            first_field(&result, "display").ok().flatten()
        }
        .await;
        let Some(display) = resolved else {
            // No specific view and no `_:_` default view for this model.
            // If the entity has data, fall back to a notation dump so
            // it's still inspectable; otherwise settle (the entity
            // frame decides not-found vs empty).
            let conclusion = {
                let s = state.borrow();
                if s.disposed || s.default_slide || !s.slides.is_empty() {
                    return;
                }
                s.last_frame.first().cloned()
            };
            if let Some(conclusion) = conclusion {
                mount_notation_fallback(&host, &state, &conclusion);
            }
            return;
        };
        // Another frame may have raced ahead while we were querying: if
        // a model-specific view landed (default_slide cleared with a
        // slide present), don't clobber it with the default.
        let mut s = state.borrow_mut();
        if s.disposed || (!s.default_slide && !s.slides.is_empty()) {
            return;
        }
        // Replace any prior slides with the single default slide.
        for (_, slide) in std::mem::take(&mut s.slides) {
            if let Some(parent) = slide.item.parent_node() {
                let _: Result<Node, _> = parent.remove_child(&slide.item);
            }
        }
        if let Some(slide) = mount_view_slide(&host, &mut s, &display) {
            // Replay the whole cached frame (directory mode has one
            // conclusion per instance; a single replay would drop all
            // but the first).
            if !s.last_frame.is_empty() {
                let detail = augmented_detail(&host, &s.last_frame, s.directory);
                call_render(&slide.view_el, &detail);
            }
            s.slides.insert("__default__".to_owned(), slide);
            s.default_slide = true;
            // Rendered through the `_:_` default view, not a
            // model-specific one — observably `default-view` so an
            // embedder can tell "rendered the way I intended" from
            // "rendered through the generic fallback".
            state::set(&host, State::DefaultView);
            drop(s);
            refresh_host_watch(&host, &state);
        }
    });
}

/// Ultimate fallback when an entity has data but its model has no view
/// (neither a specific view nor the `_:_` default): mount a
/// `<tonk-notation>` rendering the conclusion as syntax-highlighted
/// notation, so any entity is at least inspectable. `<tonk-notation>`
/// is a passive renderer of notation *text* (via a
/// `<script type="text/tonk-notation">` child), so we format the
/// conclusion here (the same `notation_format` recipe carousel mode
/// uses) and write it into that script.
fn mount_notation_fallback(host: &Element, state: &Rc<RefCell<Inner>>, conclusion: &Conclusion) {
    let Some(document) = window().and_then(|w| w.document()) else {
        return;
    };
    let (Ok(notation), Ok(script)) = (
        document.create_element("tonk-notation"),
        document.create_element("script"),
    ) else {
        return;
    };
    let _ = script.set_attribute("type", "text/tonk-notation");
    let head = host
        .get_attribute("model")
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "concept".to_owned());
    let text = crate::notation_format::format(&conclusion.this, &conclusion.fields, &head, None);
    script.set_text_content(Some(&text));
    let _ = notation.append_child(&script);

    let mut s = state.borrow_mut();
    if s.disposed || s.default_slide || !s.slides.is_empty() {
        return;
    }
    let _ = host.append_child(&notation);
    s.slides.insert(
        "__notation__".to_owned(),
        Slide {
            display: String::new(),
            item: notation.clone(),
            view_el: notation,
        },
    );
    // Track the fallback's `<script>` so subsequent entity frames re-format it
    // via `update_notation`. A `<tonk-notation>` has no `draw` method, so the
    // `call_render` loop over slides no-ops on it; without this the default
    // `_:_` dump renders the mount-time conclusion once and never reflects a
    // later update (the counter stayed stale on every increment).
    s.notation_source = Some(script);
    s.default_slide = true;
    // The notation dump is the ultimate `_:_` fallback — also
    // `default-view`, not a model-specific render.
    state::set(host, State::DefaultView);
    refresh_host_watch(host, state);
}

/// Diff a portal-mode view frame. Single-mode only, so at most one
/// slide keyed by the view entity URI. A new row mounts a
/// `<tonk-portal>` scoped to the entity; a changed `display` updates
/// the portal's `content` in place (it reloads itself); a vanished row
/// removes the portal.
fn handle_portal_view_frame(host: &Element, s: &mut Inner, conclusions: Vec<Conclusion>) {
    let incoming = slide_keys(conclusions);

    // Remove a vanished portal.
    let stale: Vec<String> = s
        .slides
        .keys()
        .filter(|k| !incoming.contains_key(*k))
        .cloned()
        .collect();
    for name in stale {
        if let Some(slide) = s.slides.remove(&name)
            && let Some(parent) = slide.item.parent_node()
        {
            let _: Result<Node, _> = parent.remove_child(&slide.item);
        }
    }

    for (name, display) in incoming {
        match s.slides.get(&name) {
            Some(slide) if slide.display == display => {}
            // Content changed: patch the portal's `content` attribute;
            // it reloads its iframe internally.
            Some(slide) => {
                let _ = slide.view_el.set_attribute("content", &display);
                if let Some(slide) = s.slides.get_mut(&name) {
                    slide.display = display;
                }
            }
            None => {
                if let Some(slide) = mount_portal_slide(host, s, &display) {
                    s.slides.insert(name, slide);
                }
            }
        }
    }

    if !s.slides.is_empty() {
        state::set(host, State::Ready);
    }
}

/// Mount a `<tonk-portal>` scoped to the displayed entity. Attributes
/// and the descriptor property are set before append so the portal's
/// `connected_callback` builds its bridge against the final context.
fn mount_portal_slide(host: &Element, inner: &Inner, display: &str) -> Option<Slide> {
    let document = window()?.document()?;
    let portal = document.create_element("tonk-portal").ok()?;
    let _ = portal.set_attribute("content", display);
    if let Some(entity) = host.get_attribute("entity") {
        let _ = portal.set_attribute("entity", &entity);
    }
    if let Some(model) = inner.portal_model.as_ref() {
        let _ = portal.set_attribute("model", model);
    }
    if let Some(descriptor) = inner.portal_descriptor.as_ref() {
        let _ = Reflect::set(
            portal.as_ref(),
            &"descriptor".into(),
            &JsValue::from_str(descriptor),
        );
    }
    let _ = host.append_child(&portal);
    Some(Slide {
        display: display.to_owned(),
        item: portal.clone(),
        view_el: portal,
    })
}

/// Marker attribute stamped on a `<tonk-display>` host once its event
/// delegate is installed — the persistent, queryable twin of the one-shot
/// `tonk-display:bound` event. A `<tonk-page onmount=…>` whose command is
/// handled by this display's delegate reads it to learn the delegate is
/// listening, so it can fire `mount` even if it connected (or reconnected
/// across a view reconcile) *after* the announcement and missed the event.
/// Cleared while a fresh delegate refresh is pending, so it never reads
/// ready during the async descriptor-resolve window. This is a DOM contract
/// shared with `<tonk-page>` in the `tonk-workspace` crate — keep the string
/// in sync there.
const BOUND_ATTR: &str = "data-bound";

/// Walk the currently-mounted `<tonk-view>` slides, collect the
/// distinct event types and concept names they reference via
/// their `data-event-bindings` attribute, resolve each concept's
/// descriptor via phase-1 lookup, and install a fresh
/// [`Delegate`](crate::events::delegate::Delegate) on the host.
///
/// Scheduled (not awaited inline) so view-frame handling stays
/// synchronous — the listener install lags one tick behind the
/// slide mount, which is fine because there's no way to click a
/// slide that hasn't reached the DOM yet anyway.
fn schedule_delegate_refresh(host: &Element, state: &Rc<RefCell<Inner>>) {
    // Bump the per-refresh generation so any in-flight task started
    // by a previous frame bails on its final write. Each new frame's
    // task captures this value at start and re-checks before
    // installing.
    let delegate_generation = {
        let mut s = state.borrow_mut();
        s.delegate_generation = s.delegate_generation.wrapping_add(1);
        s.delegate_generation
    };
    // A fresh delegate is about to be (re)built asynchronously; until the
    // install completes the host is not ready to handle events. Drop the
    // readiness marker now (synchronously) so a `<tonk-page>` reading it
    // during the resolve window does not fire into a half-installed
    // delegate. The settling refresh re-stamps it on install.
    let _ = host.remove_attribute(BOUND_ATTR);
    let host = host.clone();
    let state = state.clone();
    spawn_local(async move {
        refresh_delegate(&host, &state, delegate_generation).await;
    });
}

async fn refresh_delegate(host: &Element, state: &Rc<RefCell<Inner>>, delegate_generation: u64) {
    use crate::events::delegate::{Delegate, Descriptors};

    // Snapshot the view elements so we don't hold the borrow
    // across awaits. The delegate's `claim` calls dispatch on the
    // host element itself; the `<tonk-host>` ancestor handles
    // `(space, branch)` annotation, so no URL plumbing here.
    let view_els: Vec<Element> = {
        let s = state.borrow();
        if s.disposed || s.delegate_generation != delegate_generation {
            return;
        }
        s.slides.values().map(|sl| sl.view_el.clone()).collect()
    };

    // Collect distinct event types and concept names across slides.
    let mut event_types: BTreeSet<String> = BTreeSet::new();
    let mut concept_names: BTreeSet<String> = BTreeSet::new();
    for el in &view_els {
        let Some(raw) = el.get_attribute("data-event-bindings") else {
            continue;
        };
        let Ok(value): Result<serde_json::Value, _> = serde_json::from_str(&raw) else {
            continue;
        };
        if let Some(arr) = value.get("events").and_then(|v| v.as_array()) {
            for item in arr {
                if let Some(s) = item.as_str() {
                    event_types.insert(s.to_owned());
                }
            }
        }
        if let Some(arr) = value.get("concepts").and_then(|v| v.as_array()) {
            for item in arr {
                if let Some(s) = item.as_str() {
                    concept_names.insert(s.to_owned());
                }
            }
        }
    }

    if event_types.is_empty() || concept_names.is_empty() {
        // No bindings — drop any existing delegate so detached
        // listeners don't linger after a template-swap. Only do
        // this if no newer refresh has been scheduled in the
        // meantime; otherwise the newer one is responsible for the
        // final state.
        let mut s = state.borrow_mut();
        if !s.disposed && s.delegate_generation == delegate_generation {
            s.delegate = None;
        }
        return;
    }

    // Resolve each concept's descriptor via the host's
    // `tonk-query` event. The host annotates space/branch. The
    // descriptor JSON is parsed once here so click-time event
    // handlers don't pay the parse cost.
    let mut descriptors: Descriptors = Descriptors::new();
    for name in &concept_names {
        // Bail mid-loop if a newer refresh has started — no point
        // resolving descriptors for a snapshot we won't install.
        {
            let s = state.borrow();
            if s.disposed || s.delegate_generation != delegate_generation {
                return;
            }
        }
        // Resolve through the same name-first path as the model/view
        // (`resolve_model`), so an event-handler concept named after a
        // pinned-`this` concept resolves via the Name index too.
        match resolve_model(host, name).await {
            Ok((_entity, descriptor_json)) => {
                match serde_json::from_str::<serde_json::Value>(&descriptor_json) {
                    Ok(value) => {
                        descriptors.insert(name.clone(), value);
                    }
                    Err(e) => {
                        web_sys::console::warn_1(&JsValue::from_str(&format!(
                            "<tonk-display>: descriptor for `{name}` is not valid JSON: {e}",
                        )));
                    }
                }
            }
            Err(_) => {
                // Concept didn't resolve — bindings referencing
                // it will silently no-op on click. Continue with
                // the rest so partial failures don't break the
                // whole delegate.
            }
        }
    }

    // Build the delegate before acquiring the borrow so its
    // `addEventListener` calls don't run inside the lock.
    let delegate = Delegate::install(host.clone(), event_types.into_iter(), descriptors);
    // Re-check the per-refresh generation: if a newer refresh has
    // started while we were resolving descriptors, drop our delegate
    // rather than overwrite the newer one. `delegate`'s `Drop` impl
    // removes the listeners we just attached, so the host doesn't
    // accumulate stale listeners.
    let mut s = state.borrow_mut();
    if s.disposed || s.delegate_generation != delegate_generation {
        drop(s);
        drop(delegate);
        return;
    }
    s.delegate = Some(delegate);
    drop(s);

    // Persist readiness as a queryable marker *before* announcing it, so a
    // `<tonk-page>` that connects — or reconnects across a view reconcile —
    // after this point can detect the delegate is installed without having
    // caught the transient event below. See [`BOUND_ATTR`].
    let _ = host.set_attribute(BOUND_ATTR, "");

    // The delegate's listeners are now attached. Announce it so any
    // mount-triggered element (e.g. `<tonk-page onmount=…>`) that
    // connected *before* this point — its `mount` event would have fired
    // into the void, since the delegate installs asynchronously after the
    // template renders — can now fire knowing a listener exists. Carries
    // the bound event types so a listener only acts when its event is live.
    dispatch_event(host, "tonk-display:bound", Some(JsValue::from_str("ok")));
}

/// Apply an entity frame: empty → empty state + clear slides;
/// non-empty → fold rows + cache + render on every slide.
///
/// The fold collapses N rows for the same entity (one per tuple
/// the worker emits for cardinality-many attributes) into a single
/// conclusion whose differing fields become `Array` values. The
/// template renderer's iteration-aware walk does the per-value
/// cloning from there.
fn handle_entity_frame(
    host: &Element,
    state: &Rc<RefCell<Inner>>,
    conclusions: Vec<Conclusion>,
    reconnect: bool,
) {
    // Everything is a list of folds: group the flat rows by `this` into
    // one folded conclusion per subject. Cardinality-one is just a
    // one-element frame; the renderer iterates `{this}` over the frame.
    let frame = crate::fold::select_rows(conclusions);

    if frame.is_empty() {
        // HOLD a reconnect-empty over previously rendered content: the
        // first frame after a re-opened subscription can reflect state the
        // worker lost across a restart (an overlay-backed record — e.g. the
        // site stamp — before its owner re-asserts it). Clearing on it
        // collapses a perfectly good view into a spinner and tears down
        // nested guests, only for the heal to rebuild them seconds later.
        // Keep the content; if no real frame supersedes this one within the
        // grace period, apply the emptiness for real (a genuine deletion
        // still lands, just unhurriedly).
        let held = {
            let mut s = state.borrow_mut();
            s.entity_serial += 1;
            reconnect && !s.last_frame.is_empty()
        };
        if held {
            let serial = state.borrow().entity_serial;
            let host = host.clone();
            let state = state.clone();
            spawn_local(async move {
                reconnect_grace().await;
                if state.borrow().disposed || state.borrow().entity_serial != serial {
                    return;
                }
                handle_entity_frame(&host, &state, Vec::new(), false);
            });
            return;
        }

        let mut s = state.borrow_mut();
        s.last_frame = Vec::new();
        // An empty frame means no rows matched — zero instances in a
        // collection, or a single entity whose row has not landed yet
        // (e.g. its concept was just seeded and is still syncing). Either
        // way this is non-destructive: keep the mounted view and its
        // subscription, render the empty frame through the slides so the
        // template's chrome stays put and the repeat clears its rows. The
        // same slides reconcile in place when a row later lands — no
        // teardown, no reload, no latched error. The embedder decides
        // what an empty display reads as (a placeholder, nothing) off
        // `data-state`; a single-entity display does NOT hard-fail on a
        // missing entity, so a row that arrives after init still renders.
        //
        // Directory mode (`this` unbound) signals `empty` — a legitimate
        // zero-instance collection that `<tonk-fallback>` keys its
        // launchpad on. Single mode signals `no-entity` — that one row
        // is absent.
        let directory = s.directory;
        // The resolved model concept's descriptor (parsed), so the
        // no-entity diagnostic can name the concept's required attributes
        // and probe which ones the entity is actually missing.
        let descriptor = s.resolved_model.as_ref().map(|(_, value)| value.clone());
        let diagnostic_serial = s.entity_serial;
        let diagnostic_state = state.clone();
        // A directory view with zero instances still renders chrome that may
        // read {dom.host/*} (the FAB reads {dom.host/data-space}); feed a
        // synthetic host-only conclusion so those resolve. Single mode keeps
        // the truly-empty render so its no-entity slot/diagnosis is unaffected.
        let detail = augmented_detail(host, &[], directory);
        for slide in s.slides.values() {
            call_render(&slide.view_el, &detail);
        }
        drop(s);
        if directory {
            // Directory mode: a legitimate zero-instance collection that
            // `<tonk-fallback>` keys its launchpad on. Not an absence to
            // call out — the view's own chrome renders the empty state.
            state::set(host, State::Empty);
        } else {
            // Single mode: the one entity's row is absent. The embedder
            // may slot `no-entity`; otherwise diagnose *why* the entity did
            // not match the concept — probe each required attribute and
            // report which are missing — then show that as the notation.
            let host = host.clone();
            spawn_local(async move {
                diagnose_no_entity(&host, descriptor, &diagnostic_state, diagnostic_serial).await;
            });
        }
        return;
    }

    let mut s = state.borrow_mut();
    // Cache the whole folded frame for the slide-mount replay (directory
    // mode has one conclusion per instance). The lead conclusion drives
    // notation + the result event.
    s.entity_serial += 1;
    let first = frame[0].clone();
    s.last_frame = frame.clone();
    // Each slide sees the whole frame, augmented with the host's own
    // attributes under `dom.host/*`; notation, caching, and the result
    // event keep the unaugmented conclusions.
    let detail = augmented_detail(host, &frame, s.directory);
    for slide in s.slides.values() {
        call_render(&slide.view_el, &detail);
    }
    update_notation(host, &s, &first);
    if !s.slides.is_empty() || s.notation_source.is_some() {
        // A default (`_:_`) slide renders through the generic fallback,
        // so the display is `default-view`, not `ready`. A frame with a
        // model-specific slide is genuinely `ready`.
        let rendered = if s.default_slide {
            State::DefaultView
        } else {
            State::Ready
        };
        state::set(host, rendered);
    }
    dispatch_event(host, "tonk-display:result", Some(event_detail(&first)));
}

/// Explain why the single entity did not match its model concept.
///
/// The entity is on the branch but the concept query (a conjunction over
/// every required attribute) matched nothing — so at least one required
/// attribute is absent. A dialog query is all-or-nothing, so the failing
/// conjunction can't say *which* attribute is missing; this probes each
/// required attribute independently (one single-field query per `with:`
/// entry) and reports the split: which the entity has (with their values)
/// and which it lacks. That split is exactly the manual diagnosis a
/// missing-instance otherwise forces — e.g. "tonk:binder requires
/// `active`, `subject`; entity has `subject` but not `active`".
///
/// The result drives the loud `no-entity` absence: a structured summary
/// on top, the raw present facts as notation below. A descriptor that
/// isn't available (or has no `with:` map) falls back to the bare
/// `model: / this:` notation — there is nothing to diff against.
async fn diagnose_no_entity(
    host: &Element,
    descriptor: Option<serde_json::Value>,
    state: &Rc<RefCell<Inner>>,
    entity_serial: u64,
) {
    let model = host.get_attribute("model").unwrap_or_default();
    let entity = host.get_attribute("entity").unwrap_or_default();

    // Without a descriptor `with:` map there is nothing to probe; keep the
    // old bare notation so the state still renders something meaningful.
    let with = descriptor
        .as_ref()
        .and_then(|d| d.get("with"))
        .and_then(|w| w.as_object());
    let Some(with) = with else {
        if diagnostic_was_superseded(state, entity_serial) {
            return;
        }
        state::set_absence(
            host,
            State::NoEntity,
            "Not found",
            &format!(
                r#"{model}:
  this: {entity}"#
            ),
        );
        return;
    };

    // Probe each required attribute on its own: a one-field predicate
    // pinned to this entity. A non-empty result means the entity carries
    // that attribute (capture its value); an empty result means it is
    // missing — the reason the whole-concept match failed. Missing rows
    // carry the attribute URI (`the:`) so the diagnostic can name it.
    let mut present: Vec<(String, String)> = Vec::new();
    let mut missing: Vec<(String, String)> = Vec::new();
    for (field, spec) in with {
        let one = serde_json::json!({
            "terms": { "this": entity, field: { "?": { "name": field } } },
            "predicate": { "with": { field: spec } },
        });
        let value = match serde_json::from_value::<Query>(one).ok().as_ref() {
            Some(query) => match to_body(query) {
                Ok(body) => host_consumer::query(host, &body)
                    .await
                    .ok()
                    .and_then(|result| first_field(&result, field).ok().flatten()),
                Err(_) => None,
            },
            None => None,
        };
        match value {
            Some(value) => present.push((field.clone(), value)),
            None => {
                // The dialog attribute key (`the:`), so the tooltip can name
                // exactly which attribute the entity lacks. Fall back to the
                // field name if the descriptor omits it.
                let uri = spec
                    .get("the")
                    .and_then(|t| t.as_str())
                    .unwrap_or(field)
                    .to_owned();
                missing.push((field.clone(), uri));
            }
        }
    }

    if diagnostic_was_superseded(state, entity_serial) {
        return;
    }
    state::set_no_entity_diagnostic(host, &model, &entity, &present, &missing);
}

fn diagnostic_was_superseded(state: &Rc<RefCell<Inner>>, entity_serial: u64) -> bool {
    let state = state.borrow();
    state.disposed || state.entity_serial != entity_serial
}

/// Refresh the trailing notation slide's source `<script>` with
/// `conclusion` formatted as dialog-yaml notation. No-op in
/// single mode (where `notation_source` is `None`).
fn update_notation(host: &Element, inner: &Inner, conclusion: &Conclusion) {
    let Some(script) = inner.notation_source.as_ref() else {
        return;
    };
    let head = host
        .get_attribute("model")
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "concept".to_owned());
    let text = crate::notation_format::format(&conclusion.this, &conclusion.fields, &head, None);
    script.set_text_content(Some(&text));
}

/// Ensure the `<wa-carousel>` chrome is mounted — but **only** in
/// the multi-view fallback mode. In single-view mode (when the
/// `view` attribute is set) the orchestrator skips the carousel
/// entirely and mounts the `<tonk-view>` straight into the host,
/// so the rendered template flows in its natural layout and
/// honours its container's sizing instead of being squeezed into
/// the carousel's aspect ratio.
///
/// Idempotent — does nothing if the carousel is already set up.
fn ensure_carousel(host: &Element, state: &Rc<RefCell<Inner>>, single_mode: bool) {
    if single_mode {
        return;
    }
    let mut s = state.borrow_mut();
    if s.carousel.is_some() {
        return;
    }
    let Some(document) = window().and_then(|w| w.document()) else {
        return;
    };
    let Ok(carousel) = document.create_element("wa-carousel") else {
        return;
    };
    let _ = carousel.set_attribute("navigation", "");

    // Trailing notation slide — present whenever a carousel is
    // mounted. Lets the user fall back to a syntax-highlighted
    // entity dump regardless of how many views the model has
    // published. The `<script type="text/tonk-notation">` child
    // carries the source; updating its `textContent` is what
    // `<tonk-notation>` watches via its MutationObserver.
    if let (Ok(item), Ok(notation), Ok(script)) = (
        document.create_element("wa-carousel-item"),
        document.create_element("tonk-notation"),
        document.create_element("script"),
    ) {
        let _ = script.set_attribute("type", "text/tonk-notation");
        let _ = notation.append_child(&script);
        let _ = item.append_child(&notation);
        let _ = carousel.append_child(&item);
        s.notation_source = Some(script);
        s.notation_item = Some(item);
    }

    let _ = host.append_child(&carousel);
    s.carousel = Some(carousel);
}

/// Mount a fresh `<tonk-view>` (initialized with `display` as
/// inner HTML) for this slide. Two paths:
///
/// - **Carousel present** (multi-view fallback): wrap the view in
///   a `<wa-carousel-item>` and insert it before the trailing
///   notation slide so the inspector stays last.
/// - **Carousel absent** (single-view mode): append the
///   `<tonk-view>` straight into the host so the user's template
///   flows in its natural layout — no aspect ratio, no
///   carousel-imposed sizing.
/// Forward the display host's OWN routing context (`with`) onto the view
/// it just mounted — so a view whose content resolves its own context
/// (`<tonk-tree>`, `<tonk-inspector>`, a nested `<ui-sync-status>`) sees
/// the display's context without inferring it from DOM ancestors. This is
/// the ONE propagation boundary: context flows across a `<tonk-display>`,
/// never through an arbitrary element. A view element that carries its own
/// `with` in the template keeps it (stamp only where absent).
///
/// The display host has a `with` only when the template gave it one (route
/// views: `<tonk-display with="{branch}@{repo}">`). With none, the view
/// inherits the site's pinned context like any other consumer, and there
/// is nothing to forward.
fn forward_with(host: &Element, view_el: &Element) {
    let Some(context) = host.get_attribute("with").filter(|v| !v.is_empty()) else {
        return;
    };
    if context.contains('{') {
        return;
    }
    // Stamp the view root and every routing consumer inside it that lacks
    // its own `with`. `:scope, …` selects the view element too, so a view
    // that IS a single routing consumer is covered.
    let selector = "[data-tonk-with], tonk-tree, tonk-inspector, ui-sync-status";
    if view_el
        .matches(&format!("{selector}, [with]"))
        .unwrap_or(false)
        && !view_el.has_attribute("with")
    {
        let _ = view_el.set_attribute("with", &context);
    }
    if let Ok(list) = view_el.query_selector_all(selector) {
        for i in 0..list.length() {
            if let Some(el) = list.item(i).and_then(|n| n.dyn_into::<Element>().ok())
                && !el.has_attribute("with")
            {
                let _ = el.set_attribute("with", &context);
            }
        }
    }
}

fn mount_view_slide(host: &Element, inner: &mut Inner, display: &str) -> Option<Slide> {
    let document = window()?.document()?;

    let view_el = document.create_element("tonk-view").ok()?;
    // Stamp the model concept's `cardinality: one` field names so the view's
    // planner treats them as scalar substitutions, not iteration axes — an
    // optional scalar field used in the template then renders its host once
    // (blank when absent) instead of being cloned zero times and dropped. Set
    // before the element connects (append below), since `<tonk-view>` reads it
    // in `connected_callback`.
    if let Some(descriptor) = inner.portal_descriptor.as_deref() {
        let scalars = tonk_template::resolve::scalar_field_names(descriptor);
        if !scalars.is_empty() {
            let csv = scalars.into_iter().collect::<Vec<_>>().join(",");
            let _ = view_el.set_attribute("data-scalar-fields", &csv);
        }
    }
    view_el.set_inner_html(display);
    forward_with(host, &view_el);

    let item: Element = if let Some(carousel) = inner.carousel.as_ref() {
        let wrapper = document.create_element("wa-carousel-item").ok()?;
        let _ = wrapper.append_child(&view_el);
        if let Some(trailing) = inner.notation_item.as_ref() {
            let _ = carousel.insert_before(&wrapper, Some(trailing));
        } else {
            let _ = carousel.append_child(&wrapper);
        }
        wrapper
    } else {
        // No carousel chrome — the slide *is* the `<tonk-view>`.
        // `item` is the same element as `view_el` so removal /
        // identity checks elsewhere work uniformly.
        let _ = host.append_child(&view_el);
        view_el.clone()
    };

    Some(Slide {
        display: display.to_owned(),
        item,
        view_el,
    })
}

/// Call the per-instance `draw` closure each slide element
/// installs in its `connected_callback`. We invoke `draw` directly
/// rather than going through the prototype `render` method —
/// `wasm_bindgen::Closure::wrap` produces a plain JS function with
/// no `this` binding, so when JS calls `el.render(detail)` the
/// closure sees `(this=detail, detail=undefined)` and silently
/// no-ops. Going straight to `draw` sidesteps that.
/// The installed host-attribute watcher: a `MutationObserver` on the
/// display host filtered to EXACTLY the attributes its mounted views read
/// via `{dom.host/<attr>}`. Dropping it disconnects the observer.
struct HostAttrWatch {
    observer: MutationObserver,
    _closure: Closure<dyn FnMut(Array, MutationObserver)>,
    /// The watched set, so a refresh with an unchanged union skips the
    /// reinstall.
    attrs: std::collections::BTreeSet<String>,
}

impl Drop for HostAttrWatch {
    fn drop(&mut self) {
        self.observer.disconnect();
    }
}

/// (Re)install the host-attribute watcher from the union of every mounted
/// slide's advertised `data-host-bindings`. Deferred one microtask: a
/// slide mounted inside a custom-element reaction has not run its
/// `connected_callback` (which advertises the set) yet.
fn refresh_host_watch(host: &Element, state: &Rc<RefCell<Inner>>) {
    let host = host.clone();
    let state = state.clone();
    spawn_local(async move {
        let _ = wasm_bindgen_futures::JsFuture::from(Promise::resolve(&JsValue::UNDEFINED)).await;
        let attrs: std::collections::BTreeSet<String> = {
            let s = state.borrow();
            if s.disposed {
                return;
            }
            s.slides
                .values()
                .filter_map(|slide| slide.view_el.get_attribute("data-host-bindings"))
                .flat_map(|v| v.split_whitespace().map(str::to_owned).collect::<Vec<_>>())
                .collect()
        };
        if state
            .borrow()
            .host_watch
            .as_ref()
            .is_some_and(|watch| watch.attrs == attrs)
        {
            return;
        }
        let watch = if attrs.is_empty() {
            None
        } else {
            install_host_watch(&host, &state, attrs)
        };
        state.borrow_mut().host_watch = watch;
    });
}

/// Build and attach the watcher. The closure holds a `Weak` back-reference
/// — the watcher lives inside `Inner`, so a strong `Rc` would cycle and
/// leak the display state.
fn install_host_watch(
    host: &Element,
    state: &Rc<RefCell<Inner>>,
    attrs: std::collections::BTreeSet<String>,
) -> Option<HostAttrWatch> {
    let weak = Rc::downgrade(state);
    let host_cb = host.clone();
    let closure = Closure::wrap(
        Box::new(move |records: Array, _observer: MutationObserver| {
            let Some(state) = weak.upgrade() else {
                return;
            };
            // Replay only on a genuine value change: the render diff restamps
            // attributes wholesale, and a same-value write must not re-render.
            let changed = records.iter().any(|record| {
                let Ok(record) = record.dyn_into::<MutationRecord>() else {
                    return false;
                };
                let Some(name) = record.attribute_name() else {
                    return false;
                };
                record.old_value() != host_cb.get_attribute(&name)
            });
            if changed {
                replay_host_frame(&host_cb, &state);
            }
        }) as Box<dyn FnMut(Array, MutationObserver)>,
    );
    let observer = MutationObserver::new(closure.as_ref().unchecked_ref()).ok()?;
    let init = MutationObserverInit::new();
    init.set_attributes(true);
    init.set_attribute_old_value(true);
    let filter = Array::new();
    for attr in &attrs {
        filter.push(&JsValue::from_str(attr));
    }
    init.set_attribute_filter(&filter);
    observer.observe_with_options(host, &init).ok()?;
    Some(HostAttrWatch {
        observer,
        _closure: closure,
        attrs,
    })
}

/// Replay the cached entity frame through every slide against the host's
/// CURRENT attributes. `call_render` reaches each mounted renderer's
/// update path, whose per-binding diff writes only the values that
/// changed — an incremental update, never a remount.
fn replay_host_frame(host: &Element, state: &Rc<RefCell<Inner>>) {
    let s = state.borrow();
    if s.disposed || s.slides.is_empty() {
        return;
    }
    let detail = augmented_detail(host, &s.last_frame, s.directory);
    for slide in s.slides.values() {
        call_render(&slide.view_el, &detail);
    }
}

/// The reconnect-empty grace: long enough for the owner's heal claim to
/// re-stamp (retry jitter + claim round-trip), short enough that genuine
/// emptiness still applies promptly.
async fn reconnect_grace() {
    let promise = js_sys::Promise::new(&mut |resolve, _reject| {
        if let Some(win) = window() {
            let _ = win.set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, 5_000);
        }
    });
    let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
}

fn call_render(el: &Element, detail: &JsValue) {
    let Ok(draw) = Reflect::get(el.as_ref(), &"draw".into()) else {
        return;
    };
    let Ok(func) = draw.dyn_into::<Function>() else {
        return;
    };
    let _ = func.call1(&JsValue::NULL, detail);
}

/// Serialize a frame of conclusions as a JS array — the shape a
/// slide's `draw` accepts (it renders one row per conclusion).
fn serialize_conclusions(conclusions: &[Conclusion]) -> JsValue {
    serde_wasm_bindgen::to_value(conclusions).unwrap_or(JsValue::NULL)
}

/// The host `<tonk-display>`'s own attributes as `dom.host/<attr>` fields,
/// so a template can reference them with `{dom.host/model}`,
/// `{dom.host/data-space}`, etc. — the render-time counterpart of the
/// `dom.event/*` namespace (see `events::path`). Scalars, constant across
/// the render: a directory template threads the outer model into each
/// nested `<tonk-display entity={this} model={dom.host/model}>` this way,
/// since an instance carries no pointer to its own model.
///
/// The augmentation is render-only — the cached/notation/event conclusion
/// stays unaugmented (see [`augment_frame`]). The substituter resolves
/// `{dom.host/X}` by an ordinary field lookup, so no parser or substituter
/// change is needed, only that these entries are present.
fn host_attr_fields(host: &Element) -> BTreeMap<String, Ipld> {
    let mut fields = BTreeMap::new();
    let attrs = host.attributes();
    for i in 0..attrs.length() {
        let Some(attr) = attrs.item(i) else { continue };
        fields.insert(
            format!("dom.host/{}", attr.name()),
            Ipld::String(attr.value()),
        );
    }
    fields
}

/// Augment a render frame with the host's `dom.host/*` fields.
///
/// Normally this just folds the host fields into every conclusion. The
/// special case is an **empty directory frame**: a directory view whose
/// collection has zero instances (e.g. the FAB, whose `tonk:profile/fab`
/// concept is never asserted) still renders its chrome, and that chrome
/// may read `{dom.host/data-space}` and friends. With no conclusion to
/// fold into, those reads would resolve to nothing. So emit a single
/// conclusion carrying only the host fields; its empty `this` keeps it
/// from materializing as a repeat row (see `render::build_repeat_row`).
///
/// Single (non-directory) mode keeps an empty frame empty: an absent
/// entity must stay absent so its `no-entity` slot / diagnosis is
/// unaffected.
fn augment_frame(
    host_fields: &BTreeMap<String, Ipld>,
    frame: &[Conclusion],
    directory: bool,
) -> Vec<Conclusion> {
    if frame.is_empty() {
        if directory {
            return vec![Conclusion {
                this: String::new(),
                fields: host_fields.clone(),
            }];
        }
        return Vec::new();
    }
    frame
        .iter()
        .map(|c| {
            let mut augmented = c.clone();
            for (k, v) in host_fields {
                augmented.fields.insert(k.clone(), v.clone());
            }
            augmented
        })
        .collect()
}

/// Serialize a frame for a slide's `<tonk-view>`, augmented with the
/// host's `dom.host/*` attributes (see [`augment_frame`]).
fn augmented_detail(host: &Element, frame: &[Conclusion], directory: bool) -> JsValue {
    let host_fields = host_attr_fields(host);
    serialize_conclusions(&augment_frame(&host_fields, frame, directory))
}

/// Whether a change to `name` resolves a different view template —
/// `entity`/`model`/`view` are the subject inputs that decide what this
/// display resolves, so a change to any tears down and restarts the
/// flow. Every other observed attribute is host-context (threaded in
/// for a template to read as `{dom.host/<name>}`) and updates in place.
///
/// Conservative for now: a `model` change tears down even when the
/// resolved view template is unchanged (a model only re-projects the
/// field set). The ideal is to tear down solely on a view *template*
/// change and otherwise re-resolve the projection in place; that needs
/// the in-place path to re-run the query without rebuilding the DOM,
/// which is a larger change left for later.
fn resolves_template(name: &str) -> bool {
    matches!(name, "entity" | "model" | "view")
}

/// Re-project the host's current attributes into the mounted view(s)
/// without restarting. Re-augments the cached frame with the host's
/// `dom.host/*` attributes (picking up the changed `data-*` value) and
/// replays it through each slide's `<tonk-view>` renderer, which diffs
/// the new values into the existing DOM in place. A no-op before
/// anything is mounted (no frame, no slides).
fn replay_host_attributes(host: &Element, state: &Rc<RefCell<Inner>>) {
    let s = state.borrow();
    if s.slides.is_empty() {
        return;
    }
    // A directory view's chrome reads {dom.host/*} even with zero instances
    // (the FAB), so replay an empty directory frame too; single mode with no
    // frame has nothing to re-project.
    if s.last_frame.is_empty() && !s.directory {
        return;
    }
    let detail = augmented_detail(host, &s.last_frame, s.directory);
    for slide in s.slides.values() {
        call_render(&slide.view_el, &detail);
    }
}

fn dispatch_error(host: &Element, err: ErrorDetail) {
    dispatch_event(host, "tonk-display:error", Some(event_detail(&err)));
}

/// Serialize an event detail as a plain JS object (not a `Map`) so
/// consumers can read fields via dot access (`event.detail.<field>`).
/// `serde_wasm_bindgen::to_value` renders structs/maps as a JS `Map`,
/// which makes `detail.<field>` come out `undefined`; the
/// json-compatible serializer emits a plain object instead.
fn event_detail<T: serde::Serialize>(value: &T) -> JsValue {
    value
        .serialize(&serde_wasm_bindgen::Serializer::json_compatible())
        .unwrap_or(JsValue::NULL)
}

fn dispatch_event(host: &Element, name: &str, detail: Option<JsValue>) {
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

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_browser);

    fn host_field(key: &str, value: &str) -> BTreeMap<String, Ipld> {
        let mut fields = BTreeMap::new();
        fields.insert(key.to_owned(), Ipld::String(value.to_owned()));
        fields
    }

    fn row(this: &str, display: &str) -> Conclusion {
        Conclusion {
            this: this.to_owned(),
            fields: host_field("display", display),
        }
    }

    /// A delta applied to a retained set yields exactly the full set the
    /// equivalent snapshot would carry: retracted rows leave, asserted
    /// rows join, untouched rows stay. This equivalence is what lets the
    /// consumer route a delta through the same handler as a snapshot.
    #[dialog_common::test]
    fn it_applies_a_delta_equivalently_to_a_snapshot() {
        let retained = vec![row("a", "A"), row("b", "B"), row("c", "C")];

        // Retract b, assert d — the snapshot that would follow.
        let asserted = vec![row("d", "D")];
        let retracted = vec![row("b", "B")];

        let merged = apply_delta(retained, &asserted, retracted);

        assert_eq!(
            merged,
            vec![row("a", "A"), row("c", "C"), row("d", "D")],
            "delta merge must equal the equivalent full snapshot"
        );
    }

    /// A retract whose value differs from the retained row (stale key)
    /// removes nothing — equality is by value, so a mismatch can't drop a
    /// live row.
    #[dialog_common::test]
    fn it_ignores_a_retract_that_does_not_match_a_retained_row() {
        let retained = vec![row("a", "A")];
        let merged = apply_delta(retained, &[], vec![row("a", "STALE")]);
        assert_eq!(merged, vec![row("a", "A")]);
    }

    fn count_row(this: &str, count: i128) -> Conclusion {
        let mut fields = BTreeMap::new();
        fields.insert("count".to_owned(), Ipld::Integer(count));
        Conclusion {
            this: this.to_owned(),
            fields,
        }
    }

    /// Superseding a cardinality-one field on an existing entity (the
    /// counter case): the delta retracts the old value and asserts the
    /// new one for the SAME `this`. The merged set must contain exactly
    /// one row for that entity, carrying the NEW value — not two rows
    /// (old + new) that a downstream group-by-`this` fold would collapse
    /// into a multi-valued/stale field.
    #[dialog_common::test]
    fn it_supersedes_a_cardinality_one_field_for_the_same_entity() {
        let retained = vec![count_row("e", 4)];
        let merged = apply_delta(retained, &[count_row("e", 5)], vec![count_row("e", 4)]);
        assert_eq!(
            merged,
            vec![count_row("e", 5)],
            "supersession must leave exactly one row carrying the new value"
        );
    }

    /// The failure the counter bug actually hit: the retained base has
    /// DRIFTED from what the delta's retract names (the consumer missed
    /// or mis-applied an earlier frame, so its row is `count:5` while the
    /// delta retracts `count:4` and asserts `count:6`). A correct apply
    /// must still end with one row carrying the newest asserted value —
    /// never two rows for one entity.
    #[dialog_common::test]
    fn it_supersedes_even_when_the_retained_base_has_drifted() {
        let retained = vec![count_row("e", 5)];
        let merged = apply_delta(retained, &[count_row("e", 6)], vec![count_row("e", 4)]);
        assert_eq!(
            merged,
            vec![count_row("e", 6)],
            "an asserted row must replace the entity's prior row even when the \
             retract value does not match the (drifted) retained row"
        );
    }

    fn tag_row(this: &str, tag: &str) -> Conclusion {
        let mut fields = BTreeMap::new();
        fields.insert("tag".to_owned(), Ipld::String(tag.to_owned()));
        Conclusion {
            this: this.to_owned(),
            fields,
        }
    }

    /// Directory / multi-row mode: one `this` legitimately carries
    /// several rows (one per tuple of a cardinality-many field). The
    /// reactor's per-entity diff supersedes ONE tuple by asserting the
    /// new tuple and retracting the old — the drift heal must NOT wipe
    /// the entity's other tuples, so the untouched rows survive and
    /// only the superseded tuple is replaced.
    #[dialog_common::test]
    fn it_preserves_other_rows_of_a_multi_valued_entity_through_a_delta() {
        let retained = vec![tag_row("e", "x"), tag_row("e", "y"), tag_row("e", "z")];
        // Supersede tuple `y` → `w`; the retract matches a live row, so
        // it is a clean tuple removal, not drift.
        let merged = apply_delta(retained, &[tag_row("e", "w")], vec![tag_row("e", "y")]);
        assert_eq!(
            merged,
            vec![tag_row("e", "x"), tag_row("e", "z"), tag_row("e", "w")],
            "a multi-valued entity keeps its untouched tuples; only the \
             superseded tuple is replaced"
        );
    }

    /// `forward_with` stamps the display host's OWN `with` onto routing
    /// consumers in the mounted view that lack one, and leaves those with
    /// their own `with` alone — the ONE context-propagation boundary,
    /// replacing DOM-ancestor inference.
    #[cfg(target_arch = "wasm32")]
    #[dialog_common::test]
    fn it_forwards_its_with_onto_routing_consumers() {
        let document = web_sys::window().unwrap().document().unwrap();
        let host = document.create_element("tonk-display").unwrap();
        host.set_attribute("with", "main@did:key:zSpace").unwrap();
        let view = document.create_element("tonk-view").unwrap();
        view.set_inner_html(concat!(
            "<ui-sync-status></ui-sync-status>",
            "<tonk-tree></tonk-tree>",
            r#"<tonk-inspector with="main@did:key:zOther"></tonk-inspector>"#,
            "<span>plain</span>",
        ));
        host.append_child(&view).unwrap();

        forward_with(&host, &view);

        assert_eq!(
            view.query_selector("ui-sync-status")
                .unwrap()
                .unwrap()
                .get_attribute("with")
                .as_deref(),
            Some("main@did:key:zSpace"),
            "a routing consumer without its own with inherits the display's",
        );
        assert_eq!(
            view.query_selector("tonk-tree")
                .unwrap()
                .unwrap()
                .get_attribute("with")
                .as_deref(),
            Some("main@did:key:zSpace"),
        );
        assert_eq!(
            view.query_selector("tonk-inspector")
                .unwrap()
                .unwrap()
                .get_attribute("with")
                .as_deref(),
            Some("main@did:key:zOther"),
            "a consumer with its own with is left untouched",
        );
        assert!(
            view.query_selector("span")
                .unwrap()
                .unwrap()
                .get_attribute("with")
                .is_none(),
            "a plain element gets no routing context",
        );
    }

    /// A display with no `with` of its own forwards nothing — the view
    /// inherits the guest's pinned site context instead.
    #[cfg(target_arch = "wasm32")]
    #[dialog_common::test]
    fn it_forwards_nothing_without_its_own_with() {
        let document = web_sys::window().unwrap().document().unwrap();
        let host = document.create_element("tonk-display").unwrap();
        let view = document.create_element("tonk-view").unwrap();
        view.set_inner_html("<ui-sync-status></ui-sync-status>");
        host.append_child(&view).unwrap();

        forward_with(&host, &view);

        assert!(
            view.query_selector("ui-sync-status")
                .unwrap()
                .unwrap()
                .get_attribute("with")
                .is_none(),
            "no display context -> nothing forwarded",
        );
    }

    /// A host with one rendered child and a mounted slide, plus the shared
    /// state — the shape `on_error` reconciles against.
    #[cfg(target_arch = "wasm32")]
    fn rendered_display() -> (Element, Rc<RefCell<Inner>>, Element) {
        let document = web_sys::window().expect("window").document().expect("doc");
        let host = document.create_element("tonk-display").expect("host");
        document
            .body()
            .expect("body")
            .append_child(&host)
            .expect("attach host");
        let content = document.create_element("span").expect("content");
        content.set_text_content(Some("rendered"));
        host.append_child(&content).expect("attach content");
        let view: Element = document.create_element("tonk-view").expect("view");
        host.append_child(&view).expect("attach view");
        let mut inner = Inner::new();
        inner.slides.insert(
            "v".to_owned(),
            Slide {
                display: String::new(),
                item: view.clone(),
                view_el: view,
            },
        );
        (host, Rc::new(RefCell::new(inner)), content)
    }

    /// A dropped connection must NOT replace the rendered content with a
    /// callout: the DOM stays, the host is stamped `offline`, and the
    /// host-side reconnect heals it with the next frame.
    #[cfg(target_arch = "wasm32")]
    #[dialog_common::test]
    fn it_keeps_rendered_content_on_a_transport_interruption() {
        let (host, state, content) = rendered_display();
        let payload = js_sys::Object::new();
        let _ = Reflect::set(
            &payload,
            &"message".into(),
            &JsValue::from_str("stream read failed: TypeError: network error"),
        );
        on_error(&host, &state, payload.into(), JsValue::UNDEFINED);
        assert!(content.is_connected(), "rendered content must survive");
        assert!(
            !state.borrow().slides.is_empty(),
            "slides must survive an interruption"
        );
        assert_eq!(host.get_attribute("data-state").as_deref(), Some("offline"));
    }

    /// A reconnect-empty frame HOLDS previously rendered content (the
    /// worker may have lost overlay state it is about to re-assert); a
    /// plain empty frame clears as before.
    #[cfg(target_arch = "wasm32")]
    #[dialog_common::test]
    fn it_holds_content_on_a_reconnect_empty_frame() {
        let (host, state, _content) = rendered_display();
        state.borrow_mut().last_frame = vec![Conclusion {
            this: "e:1".to_owned(),
            fields: BTreeMap::new(),
        }];

        handle_entity_frame(&host, &state, Vec::new(), true);
        assert!(
            !state.borrow().last_frame.is_empty(),
            "a reconnect-empty must hold the cached frame"
        );
        assert_ne!(
            host.get_attribute("data-state").as_deref(),
            Some("no-entity"),
            "a reconnect-empty must not surface absence"
        );

        handle_entity_frame(&host, &state, Vec::new(), false);
        assert!(
            state.borrow().last_frame.is_empty(),
            "a plain empty frame clears as before"
        );
    }

    #[cfg(target_arch = "wasm32")]
    #[dialog_common::test]
    async fn it_discards_a_no_entity_diagnostic_superseded_by_a_ready_frame() {
        let (host, state, content) = rendered_display();
        let no_entity = web_sys::window()
            .unwrap()
            .document()
            .unwrap()
            .create_element("span")
            .unwrap();
        no_entity.set_attribute("slot", "no-entity").unwrap();
        no_entity.set_attribute("hidden", "").unwrap();
        host.append_child(&no_entity).unwrap();

        handle_entity_frame(&host, &state, Vec::new(), false);
        let stale_serial = state.borrow().entity_serial;
        handle_entity_frame(
            &host,
            &state,
            vec![Conclusion {
                this: "e:ready".to_owned(),
                fields: BTreeMap::new(),
            }],
            false,
        );
        diagnose_no_entity(&host, None, &state, stale_serial).await;

        assert_eq!(host.get_attribute("data-state").as_deref(), Some("ready"));
        assert!(
            no_entity.has_attribute("hidden"),
            "a stale diagnostic must not reveal the no-entity slot"
        );
        assert!(
            content.is_connected(),
            "the ready frame must remain mounted"
        );
        host.remove();
    }

    /// An authorization refusal is a real failure: the loud path replaces
    /// content as before.
    #[cfg(target_arch = "wasm32")]
    #[dialog_common::test]
    fn it_replaces_content_on_an_authorization_refusal() {
        let (host, state, content) = rendered_display();
        let payload = js_sys::Object::new();
        let _ = Reflect::set(
            &payload,
            &"message".into(),
            &JsValue::from_str("HTTP 403: not a member"),
        );
        on_error(&host, &state, payload.into(), JsValue::UNDEFINED);
        assert!(!content.is_connected(), "refused content is torn down");
        assert!(state.borrow().slides.is_empty());
        assert_eq!(
            host.get_attribute("data-state").as_deref(),
            Some("unauthorized")
        );
    }

    // `<tonk-display>` mounts each view slide with `data-scalar-fields` derived
    // from the model concept's `cardinality: one` fields, so the view's planner
    // keeps an optional scalar field's host element when the value is absent
    // (e.g. `<tonk-site path={rest}>` on the bare space root).
    #[dialog_common::test]
    fn it_stamps_scalar_fields_on_the_mounted_view_from_the_descriptor() {
        let document = web_sys::window().expect("window").document().expect("doc");
        let host = document.create_element("div").expect("host");
        let mut inner = Inner::new();
        inner.portal_descriptor = Some(
            r#"{
                "with":  { "id":   { "the": "xyz.tonk.site/id",   "as": "Text", "cardinality": "one" } },
                "maybe": { "rest": { "the": "xyz.tonk.site/rest", "as": "Text", "cardinality": "one" } }
            }"#
            .to_owned(),
        );
        mount_view_slide(&host, &mut inner, "<a path=\"{rest}\">x</a>").expect("slide mounts");
        let view = host
            .query_selector("tonk-view")
            .unwrap()
            .expect("a <tonk-view> was mounted");
        // BTreeSet → comma-joined in sorted order.
        assert_eq!(
            view.get_attribute("data-scalar-fields").unwrap_or_default(),
            "id,rest",
            "the mounted view should carry the descriptor's cardinality-one fields"
        );
    }

    #[dialog_common::test]
    fn augment_frame_emits_a_host_only_conclusion_for_an_empty_directory() {
        // A zero-instance directory view (the FAB) still needs its chrome to
        // read {dom.host/data-space}, so emit one subjectless conclusion
        // carrying just the host fields.
        let hf = host_field("dom.host/data-space", "acme");
        let out = augment_frame(&hf, &[], true);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].this, "");
        assert_eq!(
            out[0].fields.get("dom.host/data-space"),
            Some(&Ipld::String("acme".to_owned()))
        );
    }

    #[dialog_common::test]
    fn augment_frame_keeps_an_empty_single_frame_empty() {
        // Single (non-directory) mode: an absent entity stays absent so the
        // no-entity slot / diagnosis is unaffected.
        let hf = host_field("dom.host/data-space", "acme");
        assert!(augment_frame(&hf, &[], false).is_empty());
    }

    #[dialog_common::test]
    fn augment_frame_folds_host_fields_into_each_conclusion() {
        let hf = host_field("dom.host/data-space", "acme");
        let frame = vec![Conclusion {
            this: "did:x".to_owned(),
            fields: BTreeMap::new(),
        }];
        let out = augment_frame(&hf, &frame, true);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].this, "did:x");
        assert_eq!(
            out[0].fields.get("dom.host/data-space"),
            Some(&Ipld::String("acme".to_owned()))
        );
    }

    /// Only the subject inputs (`entity`/`model`/`view`) re-resolve what
    /// the display renders, so only those force a teardown. A host-context
    /// attribute like `data-active` (threaded in for a template to read
    /// as `{dom.host/data-active}`) leaves the resolved view unchanged and
    /// must update in place — a teardown there would re-resolve and reload
    /// the whole subtree on every selection change.
    #[dialog_common::test]
    fn it_tears_down_only_on_subject_input_changes() {
        assert!(resolves_template("entity"));
        assert!(resolves_template("model"));
        assert!(resolves_template("view"));
        assert!(
            !resolves_template("data-active"),
            "a host-context attribute must update in place, not tear down",
        );
    }

    /// Outbound `tonk-display:result` / `:error` details must be plain
    /// JS objects so consumers can read `event.detail.<field>`.
    /// `serde_wasm_bindgen::to_value` renders them as a `Map`, where
    /// `Reflect`/dot access reads `undefined`.
    #[dialog_common::test]
    fn it_serializes_event_detail_as_a_dot_accessible_object() {
        #[derive(serde::Serialize)]
        struct Probe {
            label: String,
        }
        let detail = event_detail(&Probe {
            label: "hi".to_owned(),
        });
        let field = Reflect::get(&detail, &JsValue::from_str("label")).expect("reflect get");
        assert_eq!(field.as_string().as_deref(), Some("hi"));
    }

    fn view_row(this: &str, display: Option<&str>) -> Conclusion {
        let mut fields = BTreeMap::new();
        if let Some(d) = display {
            fields.insert("display".to_owned(), Ipld::String(d.to_owned()));
        }
        Conclusion {
            this: this.to_owned(),
            fields,
        }
    }

    // Each slide keys off its own view entity URI (`this`), paired
    // with the view's `display` template. The model-constrained
    // query resolves to one row in the common case, but the keying
    // is uniform regardless of how many rows arrive.
    #[dialog_common::test]
    fn it_keys_slides_by_view_entity() {
        let rows = vec![
            view_row("did:key:zViewA", Some("<p>A</p>")),
            view_row("did:key:zViewB", Some("<p>B</p>")),
        ];
        let keyed = slide_keys(rows);
        assert_eq!(keyed.len(), 2);
        assert_eq!(
            keyed.get("did:key:zViewA").map(String::as_str),
            Some("<p>A</p>")
        );
        assert_eq!(
            keyed.get("did:key:zViewB").map(String::as_str),
            Some("<p>B</p>")
        );
    }

    // A frame row with no `display` field can't render; it's dropped
    // rather than producing a blank slide.
    #[dialog_common::test]
    fn it_drops_a_frame_row_with_no_display_field() {
        let rows = vec![view_row("did:key:zView", None)];
        assert!(slide_keys(rows).is_empty());
    }

    // An empty model frame is the `no-model` steady state, not an error:
    // `extract_phase1_conclusion` returns `None` so the handler keeps the
    // subscription and waits for the concept.
    #[dialog_common::test]
    fn it_reads_no_model_from_an_empty_phase1_frame() {
        assert!(extract_phase1_conclusion(Vec::new()).is_none());
    }

    // A resolved phase-1 row yields the model entity + descriptor.
    #[dialog_common::test]
    fn it_reads_the_model_entity_and_descriptor_from_a_phase1_row() {
        let mut fields = BTreeMap::new();
        fields.insert(
            "source".to_owned(),
            Ipld::String("{\"with\":{}}".to_owned()),
        );
        let row = Conclusion {
            this: "did:key:zModel".to_owned(),
            fields,
        };
        let resolved = extract_phase1_conclusion(vec![row]).expect("a row resolves");
        assert_eq!(resolved.0, "did:key:zModel");
        assert_eq!(resolved.1, "{\"with\":{}}");
    }

    // A row missing the `source` descriptor is treated as unresolved
    // (`no-model`), not a half-resolved model.
    #[dialog_common::test]
    fn it_treats_a_phase1_row_without_source_as_no_model() {
        let row = Conclusion {
            this: "did:key:zModel".to_owned(),
            fields: BTreeMap::new(),
        };
        assert!(extract_phase1_conclusion(vec![row]).is_none());
    }

    // `loud_state` classifies an error into the loud state that drives
    // the danger callout. A network error carrying an HTTP 403 is
    // `unauthorized` (a real wire signal — the worker formats access
    // denials as `HTTP 403: …`); any other network error is `offline`;
    // parse/descriptor errors are `malformed`.
    #[dialog_common::test]
    fn it_maps_a_403_network_error_to_unauthorized() {
        let err = ErrorDetail::new(ErrorKind::Network, "HTTP 403: forbidden");
        assert_eq!(loud_state(&err), State::Unauthorized);
    }

    #[dialog_common::test]
    fn it_maps_a_non_403_network_error_to_offline() {
        let err = ErrorDetail::new(ErrorKind::Network, "HTTP 500: boom");
        assert_eq!(loud_state(&err), State::Offline);
        let dropped = ErrorDetail::new(ErrorKind::Network, "connection reset");
        assert_eq!(loud_state(&dropped), State::Offline);
    }

    /// A `404` is a settled answer, not a transport hiccup: landing on
    /// a space this device never joined must read as `unknown` so the
    /// page can say so, rather than as `offline`, which claims the
    /// network is down and keeps retrying.
    #[dialog_common::test]
    fn it_maps_a_404_to_unknown_rather_than_offline() {
        let err = ErrorDetail::http(404, "HTTP 404: repository not found");
        assert_eq!(loud_state(&err), State::Unknown);
    }

    /// The status is read structurally, so a `403` classifies without
    /// the message having to spell it out.
    #[dialog_common::test]
    fn it_maps_a_403_status_to_unauthorized_without_reading_the_message() {
        let err = ErrorDetail::http(403, "nope");
        assert_eq!(loud_state(&err), State::Unauthorized);
    }

    /// Every other status stays `offline` — the retrying state.
    #[dialog_common::test]
    fn it_maps_other_statuses_to_offline() {
        assert_eq!(loud_state(&ErrorDetail::http(500, "boom")), State::Offline);
        assert_eq!(
            loud_state(&ErrorDetail::http(502, "gateway")),
            State::Offline
        );
    }

    #[dialog_common::test]
    fn it_maps_parse_and_descriptor_errors_to_malformed() {
        assert_eq!(
            loud_state(&ErrorDetail::new(ErrorKind::Parse, "bad json")),
            State::Malformed,
        );
        assert_eq!(
            loud_state(&ErrorDetail::new(ErrorKind::Descriptor, "bad attr")),
            State::Malformed,
        );
    }

    // --- Display hook: routing a `text/html` view to a portal ---
    //
    // Driven through the real `<tonk-display>` flow against a fake host
    // that answers the resolve one-shots in order (model concept → view
    // concept) and captures the subscriptions. Portal mode is decided
    // per view frame: a row whose projected `type` is `text/html`
    // mounts a `<tonk-portal>`; any other row renders inline through
    // `<tonk-view>`. Both runs open the view and entity subscriptions —
    // in portal mode the entity frames simply no-op against the iframe.
    #[cfg(target_arch = "wasm32")]
    mod hook {
        use super::*;
        use ipld_core::ipld::Ipld;
        use js_sys::{Function, Object, Promise};
        use std::cell::RefCell;
        use std::collections::BTreeMap;
        use std::rc::Rc;
        use wasm_bindgen::JsCast;
        use wasm_bindgen::closure::Closure;
        use wasm_bindgen_futures::JsFuture;
        use web_sys::CustomEvent;

        fn document() -> web_sys::Document {
            window().unwrap().document().unwrap()
        }

        async fn sleep(ms: i32) {
            let promise = Promise::new(&mut |resolve, _reject| {
                let _ = window()
                    .unwrap()
                    .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms);
            });
            let _ = JsFuture::from(promise).await;
        }

        /// Poll until `parent` contains an element matching `selector`.
        async fn await_selector(parent: &Element, selector: &str) -> Option<Element> {
            for _ in 0..200 {
                if let Ok(Some(el)) = parent.query_selector(selector) {
                    return Some(el);
                }
                sleep(5).await;
            }
            None
        }

        /// A `Vec<Conclusion>` as a JS value, matching what the host
        /// hands back (one-shot results and subscription frames both
        /// decode through `serde_wasm_bindgen::from_value`).
        fn rows(items: &[(&str, &[(&str, &str)])]) -> JsValue {
            let conclusions: Vec<Conclusion> = items
                .iter()
                .map(|(this, fields)| Conclusion {
                    this: (*this).to_owned(),
                    fields: fields
                        .iter()
                        .map(|(k, v)| (k.to_string(), Ipld::String(v.to_string())))
                        .collect::<BTreeMap<_, _>>(),
                })
                .collect();
            serde_wasm_bindgen::to_value(&conclusions).unwrap()
        }

        struct FakeHost {
            container: Element,
            state: Rc<RefCell<FakeState>>,
            _listeners: Vec<Closure<dyn FnMut(CustomEvent)>>,
        }

        struct FakeState {
            /// One-shot query responses, answered in dispatch order.
            query_responses: Vec<JsValue>,
            answered: usize,
            /// Subscription consumer + tag, keyed by tag.
            subs: BTreeMap<String, Element>,
            /// Tags of every subscription opened.
            subscribe_tags: Vec<String>,
            /// The phase-1 model concept frame, auto-pushed the moment
            /// the `"model"` subscription opens — the model resolve is a
            /// live subscription now, not a one-shot. `None` makes the
            /// model subscription stay empty (the `no-model` state).
            model_frame: Option<JsValue>,
        }

        impl FakeHost {
            fn install(query_responses: Vec<JsValue>) -> FakeHost {
                Self::install_with_model(query_responses, None)
            }

            /// Like [`install`] but with an explicit model concept frame
            /// auto-pushed on the `"model"` subscription. `None` leaves
            /// the model subscription empty so the display sits in
            /// `no-model`.
            fn install_with_model(
                query_responses: Vec<JsValue>,
                model_frame: Option<JsValue>,
            ) -> FakeHost {
                let container = document().create_element("div").unwrap();
                document().body().unwrap().append_child(&container).unwrap();
                let state = Rc::new(RefCell::new(FakeState {
                    query_responses,
                    answered: 0,
                    subs: BTreeMap::new(),
                    subscribe_tags: Vec::new(),
                    model_frame,
                }));
                let mut listeners = Vec::new();

                {
                    let state = state.clone();
                    let cb: Closure<dyn FnMut(CustomEvent)> =
                        Closure::wrap(Box::new(move |ev: CustomEvent| {
                            ev.stop_propagation();
                            ev.prevent_default();
                            let detail: Object = ev.detail().dyn_into().unwrap();
                            let mut s = state.borrow_mut();
                            let i = s.answered;
                            let result = s
                                .query_responses
                                .get(i)
                                .cloned()
                                .unwrap_or(JsValue::from(js_sys::Array::new()));
                            s.answered += 1;
                            let _ =
                                Reflect::set(&detail, &"result".into(), &Promise::resolve(&result));
                        }) as Box<dyn FnMut(CustomEvent)>);
                    let _ = container.add_event_listener_with_callback(
                        "tonk-query",
                        cb.as_ref().unchecked_ref(),
                    );
                    listeners.push(cb);
                }
                {
                    let state = state.clone();
                    let cb: Closure<dyn FnMut(CustomEvent)> =
                        Closure::wrap(Box::new(move |ev: CustomEvent| {
                            ev.stop_propagation();
                            ev.prevent_default();
                            let detail: Object = ev.detail().dyn_into().unwrap();
                            let tag = Reflect::get(&detail, &"tag".into())
                                .ok()
                                .and_then(|v| v.as_string())
                                .unwrap_or_default();
                            let consumer: Element = ev.target().unwrap().dyn_into().unwrap();
                            let model_frame = {
                                let mut s = state.borrow_mut();
                                s.subscribe_tags.push(tag.clone());
                                s.subs.insert(tag.clone(), consumer.clone());
                                // Auto-push the model concept frame as
                                // soon as the model subscription opens, so
                                // the downstream flow starts the way a live
                                // host would on the first revision.
                                if tag == "model" {
                                    s.model_frame.clone()
                                } else {
                                    None
                                }
                            };
                            let sub = Object::new();
                            let noop = Function::new_no_args("");
                            let _ = Reflect::set(&sub, &"cancel".into(), &noop);
                            let _ = Reflect::set(&detail, &"subscription".into(), &sub);
                            if let Some(frame) = model_frame {
                                let opts = Object::new();
                                let _ =
                                    Reflect::set(&opts, &"tag".into(), &JsValue::from_str("model"));
                                if let Ok(reset) = Reflect::get(&consumer, &"reset".into())
                                    && let Ok(reset) = reset.dyn_into::<Function>()
                                {
                                    let _ = reset.call2(&consumer, &frame, &opts);
                                }
                            }
                        }) as Box<dyn FnMut(CustomEvent)>);
                    let _ = container.add_event_listener_with_callback(
                        "tonk-subscribe",
                        cb.as_ref().unchecked_ref(),
                    );
                    listeners.push(cb);
                }

                FakeHost {
                    container,
                    state,
                    _listeners: listeners,
                }
            }

            fn push_frame(&self, tag: &str, conclusions: &JsValue) {
                let consumer = self.state.borrow().subs.get(tag).cloned();
                let Some(consumer) = consumer else { return };
                let opts = Object::new();
                let _ = Reflect::set(&opts, &"tag".into(), &JsValue::from_str(tag));
                let reset: Function = Reflect::get(&consumer, &"reset".into())
                    .unwrap()
                    .dyn_into()
                    .unwrap();
                let _ = reset.call2(&consumer, conclusions, &opts);
            }

            fn subscribe_tags(&self) -> Vec<String> {
                self.state.borrow().subscribe_tags.clone()
            }

            /// Deliver a transport `error` frame on the `tag`
            /// subscription — drives the consumer's `__tonkError` path
            /// (`on_error` → `fail`). `message` is read by `on_error` from
            /// `detail.message`.
            fn push_error(&self, tag: &str, message: &str) {
                let consumer = self.state.borrow().subs.get(tag).cloned();
                let Some(consumer) = consumer else { return };
                let payload = Object::new();
                let _ = Reflect::set(&payload, &"message".into(), &JsValue::from_str(message));
                let opts = Object::new();
                let _ = Reflect::set(&opts, &"tag".into(), &JsValue::from_str(tag));
                if let Ok(error) = Reflect::get(&consumer, &"error".into())
                    && let Ok(error) = error.dyn_into::<Function>()
                {
                    let _ = error.call2(&consumer, &payload, &opts);
                }
            }
        }

        fn mount_display(host: &FakeHost, view: &str, model: &str, entity: &str) -> Element {
            register();
            let display = document().create_element("tonk-display").unwrap();
            display.set_attribute("view", view).unwrap();
            display.set_attribute("model", model).unwrap();
            display.set_attribute("entity", entity).unwrap();
            host.container.append_child(&display).unwrap();
            display
        }

        // The flow resolves two concepts in order: the subject `model`
        // concept (projected for the entity query) then the `view`
        // concept (the query predicate). The view concept declares a
        // `type` attribute, so `view_by_model_query` projects `type` and
        // each view frame carries the value that decides portal mode.
        //
        // `resolve_model` resolves a bare name through the Name concept
        // first (`id:<name>` → `db.name/referent`), then runs the
        // Phase-1 concept query by `this`. So each bare-name resolution
        // is TWO queries: a name lookup, then the concept lookup. The
        // fixture answers in dispatch order, so a name-resolution row
        // precedes each concept row. `mount_display` uses bare names for
        // both `model` and `view`, hence two name/concept pairs.
        fn name_row(entity: &str) -> JsValue {
            rows(&[("did:key:zName", &[("entity", entity)])])
        }
        // The model concept resolves through a live `"model"`
        // subscription now, so its phase-1 row is auto-pushed as the
        // model frame (see `install_with_model`) rather than answered as
        // a one-shot. The remaining one-shots are the model *name*
        // lookup and the view name + concept lookups (the explicit
        // `view=` resolve is still a one-shot inside `start_downstream`).
        fn model_concept_frame() -> JsValue {
            rows(&[(
                "did:key:zModel",
                &[(
                    "source",
                    r#"{"with":{"count":{"the":"counter/count","as":"UnsignedInteger","cardinality":"one"}}}"#,
                )],
            )])
        }
        /// The same concept descriptor as [`model_concept_frame`] but
        /// with the object keys in a *different order* — what the worker
        /// actually emits between revisions (its descriptor serialization
        /// is not key-order-stable). Byte-different, semantically
        /// identical: the model-frame guard must treat it as unchanged.
        fn model_concept_frame_reordered() -> JsValue {
            rows(&[(
                "did:key:zModel",
                &[(
                    "source",
                    r#"{"with":{"count":{"cardinality":"one","as":"UnsignedInteger","the":"counter/count"}}}"#,
                )],
            )])
        }
        fn resolve_responses() -> Vec<JsValue> {
            vec![
                // model name `counter` → did:key:zModel
                name_row("did:key:zModel"),
                // view name `counter` → did:key:zViewConcept
                name_row("did:key:zViewConcept"),
                rows(&[(
                    "did:key:zViewConcept",
                    &[(
                        "source",
                        r#"{"with":{"model":{"the":"xyz.tonk.view/model","as":"Entity","cardinality":"one"},"display":{"the":"xyz.tonk.view/display","as":"Text","cardinality":"one"},"type":{"the":"xyz.tonk.view/type","as":"Text","cardinality":"one"}}}"#,
                    )],
                )]),
            ]
        }

        #[dialog_common::test]
        async fn it_mounts_a_portal_for_a_text_html_view_frame() {
            let host =
                FakeHost::install_with_model(resolve_responses(), Some(model_concept_frame()));
            let display = mount_display(&host, "counter", "counter", "id:demo-counter");

            // Wait for the flow to open its subscriptions (model, then
            // view + entity once the model frame resolves).
            for _ in 0..200 {
                if host.subscribe_tags().len() >= 3 {
                    break;
                }
                sleep(5).await;
            }
            // The view frame carries the projected `type` and the HTML
            // document, which routes the row to a portal.
            host.push_frame(
                "view",
                &rows(&[(
                    "did:key:zView",
                    &[
                        ("display", "<h1>monolith</h1>"),
                        ("type", "text/html"),
                        ("model", "did:key:zModel"),
                    ],
                )]),
            );

            let portal = await_selector(&display, "tonk-portal")
                .await
                .expect("a text/html view should mount a <tonk-portal>");
            assert_eq!(
                portal.get_attribute("content").as_deref(),
                Some("<h1>monolith</h1>"),
                "the view's HTML document becomes the portal content",
            );
            assert_eq!(
                portal.get_attribute("entity").as_deref(),
                Some("id:demo-counter"),
                "the portal is scoped to the displayed entity",
            );
            let descriptor = Reflect::get(portal.as_ref(), &"descriptor".into())
                .ok()
                .and_then(|v| v.as_string());
            assert!(
                descriptor.is_some_and(|d| d.contains("counter/count")),
                "the model descriptor is handed to the portal",
            );
            assert!(
                display.query_selector("tonk-view").unwrap().is_none(),
                "a portal view renders no inline <tonk-view>",
            );
            // The model subscription opens first, then the view + entity
            // subs once the model frame resolves; the entity sub just
            // no-ops against the portal (a `<tonk-portal>` exposes no
            // `draw`).
            assert_eq!(
                host.subscribe_tags(),
                vec!["model".to_owned(), "view".to_owned(), "entity".to_owned()],
            );
        }

        #[dialog_common::test]
        async fn it_renders_inline_and_subscribes_to_the_entity_when_no_type() {
            let host =
                FakeHost::install_with_model(resolve_responses(), Some(model_concept_frame()));
            let display = mount_display(&host, "counter", "counter", "id:demo-counter");

            for _ in 0..200 {
                if host.subscribe_tags().len() >= 3 {
                    break;
                }
                sleep(5).await;
            }
            host.push_frame(
                "view",
                &rows(&[("did:key:zView", &[("display", "<p>{count}</p>")])]),
            );

            assert!(
                await_selector(&display, "tonk-view").await.is_some(),
                "a view frame with no type renders inline through <tonk-view>",
            );
            assert!(
                display.query_selector("tonk-portal").unwrap().is_none(),
                "no portal is mounted for a non-portal view",
            );
            let mut tags = host.subscribe_tags();
            tags.sort();
            assert_eq!(
                tags,
                vec!["entity".to_owned(), "model".to_owned(), "view".to_owned()],
                "inline mode opens the model, view, and entity subscriptions",
            );
        }

        // The fix for the latched red box on a still-seeding space: when
        // the model concept is not on the branch yet, the model resolve
        // is an *empty subscription frame*, not a hard error. The display
        // sits in `no-model` with no callout, stays subscribed, and
        // recovers the instant the concept lands — no reload.
        #[dialog_common::test]
        async fn it_recovers_from_no_model_when_the_concept_lands_late() {
            // No auto model frame: the model subscription opens, and its
            // first frame is empty (the concept has not synced yet).
            let host = FakeHost::install(resolve_responses());
            let display = mount_display(&host, "counter", "counter", "id:demo-counter");

            // Wait for the model subscription, then deliver an empty frame
            // — that lands the display in `no-model`.
            for _ in 0..200 {
                if host.subscribe_tags().contains(&"model".to_owned()) {
                    break;
                }
                sleep(5).await;
            }
            host.push_frame("model", &rows(&[]));
            for _ in 0..200 {
                if display.get_attribute("data-state").as_deref() == Some("no-model") {
                    break;
                }
                sleep(5).await;
            }
            assert_eq!(
                display.get_attribute("data-state").as_deref(),
                Some("no-model"),
                "an absent model concept is `no-model`, not an error",
            );

            // The concept lands: the model subscription pushes a non-empty
            // frame, the downstream view + entity subs open, and a view +
            // entity frame render the row live.
            host.push_frame("model", &model_concept_frame());
            for _ in 0..200 {
                if host.subscribe_tags().contains(&"view".to_owned())
                    && host.subscribe_tags().contains(&"entity".to_owned())
                {
                    break;
                }
                sleep(5).await;
            }
            host.push_frame(
                "view",
                &rows(&[("did:key:zView", &[("display", "<p>{count}</p>")])]),
            );
            host.push_frame("entity", &rows(&[("id:demo-counter", &[("count", "7")])]));

            assert!(
                await_selector(&display, "tonk-view").await.is_some(),
                "the display recovers and renders once the concept lands",
            );
            assert!(
                display.query_selector("wa-callout").unwrap().is_none(),
                "recovery leaves no latched callout behind",
            );
        }

        // A bad `entity` attribute (not a URI) is an author error: `run`
        // rejects it before opening any subscription, so the display goes
        // `malformed` with a loud danger callout.
        #[dialog_common::test]
        async fn it_goes_malformed_on_a_bad_entity_attribute() {
            let host = FakeHost::install(Vec::new());
            // `entity` must be a URI (contain `:`); `oops` is not.
            let display = mount_display(&host, "counter", "counter", "oops");
            for _ in 0..200 {
                if display.get_attribute("data-state").as_deref() == Some("malformed") {
                    break;
                }
                sleep(5).await;
            }
            assert_eq!(
                display.get_attribute("data-state").as_deref(),
                Some("malformed"),
                "a non-URI entity attribute is a malformed author error",
            );
            let callout = display
                .query_selector("wa-callout")
                .unwrap()
                .expect("malformed surfaces a danger callout");
            assert_eq!(callout.get_attribute("variant").as_deref(), Some("danger"));
        }

        // A transport `error` frame on a live subscription drives the
        // display to `offline` QUIETLY: the rendered content stays (no
        // callout replaces it — the interruption is a hiccup, not a render
        // failure), and the host-side reconnect heals the state with the
        // next frame.
        #[dialog_common::test]
        async fn it_goes_offline_on_a_subscription_error() {
            let host =
                FakeHost::install_with_model(resolve_responses(), Some(model_concept_frame()));
            let display = mount_display(&host, "counter", "counter", "id:demo-counter");
            for _ in 0..200 {
                if host.subscribe_tags().contains(&"entity".to_owned()) {
                    break;
                }
                sleep(5).await;
            }
            host.push_error("entity", "HTTP 500: upstream gone");
            for _ in 0..200 {
                if display.get_attribute("data-state").as_deref() == Some("offline") {
                    break;
                }
                sleep(5).await;
            }
            assert_eq!(
                display.get_attribute("data-state").as_deref(),
                Some("offline"),
                "a transport error is `offline`",
            );
            assert!(
                display.query_selector("wa-callout").unwrap().is_none(),
                "an interruption must NOT replace content with a callout"
            );
        }

        // An HTTP 403 on a subscription is access denial, not a transient
        // transport failure: it drives `unauthorized`, not `offline`.
        #[dialog_common::test]
        async fn it_goes_unauthorized_on_a_403() {
            let host =
                FakeHost::install_with_model(resolve_responses(), Some(model_concept_frame()));
            let display = mount_display(&host, "counter", "counter", "id:demo-counter");
            for _ in 0..200 {
                if host.subscribe_tags().contains(&"entity".to_owned()) {
                    break;
                }
                sleep(5).await;
            }
            host.push_error("entity", "HTTP 403: forbidden");
            for _ in 0..200 {
                if display.get_attribute("data-state").as_deref() == Some("unauthorized") {
                    break;
                }
                sleep(5).await;
            }
            assert_eq!(
                display.get_attribute("data-state").as_deref(),
                Some("unauthorized"),
                "an HTTP 403 is `unauthorized`, not `offline`",
            );
        }

        // An explicit `view=` whose concept is absent on the branch is the
        // recoverable `no-view` state — a danger fallback naming the
        // missing view (a missing view concept is a config error, like a
        // missing model). The model resolves (auto-pushed frame); the view
        // name + concept one-shots return empty, so the view resolve fails
        // with `UnknownSource`.
        #[dialog_common::test]
        async fn it_goes_no_view_when_an_explicit_view_is_absent() {
            // One-shot responses: the model name lookup resolves; the view
            // name lookup + view concept lookup are left empty (default
            // empty array), so the explicit view never resolves.
            let host = FakeHost::install_with_model(
                vec![name_row("did:key:zModel")],
                Some(model_concept_frame()),
            );
            let display = mount_display(&host, "missing-view", "counter", "id:demo-counter");
            for _ in 0..200 {
                if display.get_attribute("data-state").as_deref() == Some("no-view") {
                    break;
                }
                sleep(5).await;
            }
            assert_eq!(
                display.get_attribute("data-state").as_deref(),
                Some("no-view"),
                "an absent explicit view is `no-view`",
            );
            let callout = display
                .query_selector("wa-callout")
                .unwrap()
                .expect("no-view names the missing view");
            assert_eq!(
                callout.get_attribute("variant").as_deref(),
                Some("danger"),
                "a missing view concept is a danger, like a missing model",
            );
        }

        // When the model has no model-specific view but a `_:_` default
        // view is seeded, the empty view frame falls back to it: the
        // display renders the default presentation and reports
        // `default-view` (observably distinct from `ready`).
        #[dialog_common::test]
        async fn it_renders_the_default_view_when_no_specific_view_exists() {
            // One-shots in dispatch order: the bare model name `counter`
            // resolves through the Name concept first, THEN the `_:_`
            // fallback query `spawn_default_view` makes returns a `display`
            // template. The model concept itself is the auto-pushed frame,
            // and NO explicit `view=` is set so the built-in view predicate
            // is used (no extra view-concept resolve one-shot).
            let host = FakeHost::install_with_model(
                vec![
                    name_row("did:key:zModel"),
                    rows(&[("did:key:zDefaultView", &[("display", "<p>{count}</p>")])]),
                ],
                Some(model_concept_frame()),
            );
            register();
            let display = document().create_element("tonk-display").unwrap();
            display.set_attribute("model", "counter").unwrap();
            display.set_attribute("entity", "id:demo-counter").unwrap();
            host.container.append_child(&display).unwrap();
            for _ in 0..200 {
                if host.subscribe_tags().contains(&"view".to_owned())
                    && host.subscribe_tags().contains(&"entity".to_owned())
                {
                    break;
                }
                sleep(5).await;
            }
            // The model-specific view subscription pushes an EMPTY frame —
            // no view for this model — which triggers the `_:_` fallback.
            host.push_frame("view", &rows(&[]));
            host.push_frame("entity", &rows(&[("id:demo-counter", &[("count", "9")])]));

            assert!(
                await_selector(&display, "tonk-view").await.is_some(),
                "the `_:_` default view renders when no specific view exists",
            );
            for _ in 0..200 {
                if display.get_attribute("data-state").as_deref() == Some("default-view") {
                    break;
                }
                sleep(5).await;
            }
            assert_eq!(
                display.get_attribute("data-state").as_deref(),
                Some("default-view"),
                "rendering through the `_:_` fallback reports `default-view`",
            );
        }

        // The model subscription re-pushes on EVERY branch revision,
        // including unrelated data writes. An identical model frame must
        // NOT restart the downstream flow — that would `clear_host` and
        // remount, wiping the rendered rows (the Hub list flashing empty
        // while a new space is added). A repeat model frame is a no-op;
        // data updates flow through the entity subscription in place.
        #[dialog_common::test]
        async fn it_does_not_remount_on_a_repeat_model_frame() {
            let host =
                FakeHost::install_with_model(resolve_responses(), Some(model_concept_frame()));
            let display = mount_display(&host, "counter", "counter", "id:demo-counter");

            for _ in 0..200 {
                if host.subscribe_tags().len() >= 3 {
                    break;
                }
                sleep(5).await;
            }
            host.push_frame(
                "view",
                &rows(&[("did:key:zView", &[("display", "<p>{count}</p>")])]),
            );
            host.push_frame("entity", &rows(&[("id:demo-counter", &[("count", "1")])]));
            let view = await_selector(&display, "tonk-view")
                .await
                .expect("renders the row");

            // A second model frame arrives (an unrelated write revised
            // the branch). It carries the SAME concept but with the
            // descriptor's keys in a different order — exactly what the
            // worker emits, since its descriptor serialization is not
            // key-order-stable. The guard must treat it as unchanged: the
            // mounted <tonk-view> survives, no remount, no wiped list.
            let subs_before = host.subscribe_tags().len();
            host.push_frame("model", &model_concept_frame_reordered());
            sleep(50).await;

            assert!(
                view.is_connected(),
                "a key-reordered repeat model frame must not tear down the mounted view",
            );
            assert_eq!(
                host.subscribe_tags().len(),
                subs_before,
                "a key-reordered repeat model frame must not reopen the view/entity subscriptions",
            );

            // A later entity frame still updates the existing slide in
            // place — the subscription was never torn down.
            host.push_frame("entity", &rows(&[("id:demo-counter", &[("count", "2")])]));
            sleep(50).await;
            assert!(
                view.is_connected(),
                "the same view keeps rendering data updates in place",
            );
        }

        // --- Directory mode: empty -> content live flip ---
        //
        // Directory mode (no `entity` attribute) resolves only the
        // subject `model` concept; the built-in directory view predicate
        // needs no branch lookup, so two query responses suffice: the
        // model name lookup and the concept row.
        fn directory_resolve_responses() -> Vec<JsValue> {
            vec![name_row("did:key:zModel")]
        }
        // The directory model concept, auto-pushed on the `"model"`
        // subscription (the built-in directory view predicate needs no
        // branch lookup, so only the model name lookup remains a
        // one-shot).
        fn directory_model_frame() -> JsValue {
            rows(&[(
                "did:key:zModel",
                &[(
                    "source",
                    r#"{"with":{"title":{"the":"item/title","as":"Text","cardinality":"one"}}}"#,
                )],
            )])
        }
        fn install_directory() -> FakeHost {
            FakeHost::install_with_model(
                directory_resolve_responses(),
                Some(directory_model_frame()),
            )
        }

        fn mount_directory(host: &FakeHost, model: &str) -> Element {
            register();
            crate::view::register();
            let display = document().create_element("tonk-display").unwrap();
            display.set_attribute("model", model).unwrap();
            host.container.append_child(&display).unwrap();
            display
        }

        // A directory collection that goes empty -> one instance must
        // render the new instance *live*, with no reload. The empty frame
        // used to tear the mounted view down, so when the first instance
        // landed there were no slides left to render it and the page
        // stayed blank until a refresh.
        #[dialog_common::test]
        async fn it_renders_an_instance_that_lands_after_an_empty_directory_frame() {
            let host = install_directory();
            let display = mount_directory(&host, "item");

            for _ in 0..200 {
                if host.subscribe_tags().len() >= 3 {
                    break;
                }
                sleep(5).await;
            }

            // The directory view: a per-instance row (`{this}` marks the
            // repeat) plus a fallback region that is chrome (no subject
            // reference, so it renders once regardless of instance count).
            host.push_frame(
                "view",
                &rows(&[(
                    "did:key:zView",
                    &[(
                        "display",
                        "<div><ul><li data-id={this}>{title}</li></ul><p class=\"fallback\">empty</p></div>",
                    )],
                )]),
            );

            // The collection starts empty.
            host.push_frame("entity", &rows(&[]));

            // The first instance lands.
            host.push_frame(
                "entity",
                &rows(&[("did:key:zItem1", &[("title", "Hello")])]),
            );

            let row = await_selector(&display, "li[data-id]")
                .await
                .expect("an instance landing after an empty frame should render live");
            assert_eq!(row.text_content().as_deref(), Some("Hello"));
        }

        // On a cold-empty collection the empty entity frame can arrive
        // before the view template. The view must still mount and render
        // its chrome (the fallback region) so the launchpad shows on an
        // empty repo, rather than staying blank until the first instance.
        #[dialog_common::test]
        async fn it_renders_the_fallback_chrome_when_empty_before_the_view_arrives() {
            let host = install_directory();
            let display = mount_directory(&host, "item");

            for _ in 0..200 {
                if host.subscribe_tags().len() >= 3 {
                    break;
                }
                sleep(5).await;
            }

            // The empty collection frame arrives first...
            host.push_frame("entity", &rows(&[]));

            // ...then the view template.
            host.push_frame(
                "view",
                &rows(&[(
                    "did:key:zView",
                    &[(
                        "display",
                        "<div><ul><li data-id={this}>{title}</li></ul><p class=\"fallback\">Nothing yet</p></div>",
                    )],
                )]),
            );

            let fallback = await_selector(&display, "p.fallback")
                .await
                .expect("the fallback chrome should render on an empty collection at mount");
            assert_eq!(fallback.text_content().as_deref(), Some("Nothing yet"));
        }

        // A static element that is a direct *sibling* of the repeat root
        // (the `{this}` element) must survive rendering — it is chrome,
        // not part of the repeat. Regression for the binder's
        // `[slot="empty"]` launchpad: `<tonk-sheet-binder>` holds the
        // repeat `<tonk-display entity={this}>` and a static `<div
        // slot="empty">` as direct siblings; the launchpad was dropped
        // when it sat *after* the repeat element. (A static node BEFORE
        // the repeat element survived, which is what made the bug
        // position-dependent.)
        #[dialog_common::test]
        async fn it_keeps_a_static_sibling_after_the_repeat_element() {
            let host = install_directory();
            let display = mount_directory(&host, "item");

            for _ in 0..200 {
                if host.subscribe_tags().len() >= 3 {
                    break;
                }
                sleep(5).await;
            }

            // The repeat root is the `<span data-id={this}>` element; the
            // `<p class="after">` is its direct following sibling (chrome).
            host.push_frame(
                "view",
                &rows(&[(
                    "did:key:zView",
                    &[(
                        "display",
                        "<div><span data-id={this}>{title}</span><p class=\"after\">keep me</p></div>",
                    )],
                )]),
            );
            host.push_frame(
                "entity",
                &rows(&[("did:key:zItem1", &[("title", "Hello")])]),
            );

            let row = await_selector(&display, "span[data-id]")
                .await
                .expect("the repeat row should render");
            assert_eq!(row.text_content().as_deref(), Some("Hello"));

            let after = await_selector(&display, "p.after")
                .await
                .expect("a static sibling after the repeat element must survive");
            assert_eq!(after.text_content().as_deref(), Some("keep me"));
        }

        // The static sibling must also survive when the repeat element
        // is a *custom element* (the binder's real shape): the renderer
        // must not let the element's own connectedCallback / mutation
        // churn drop chrome that follows it.
        #[dialog_common::test]
        async fn it_keeps_a_static_sibling_after_a_repeat_custom_element() {
            let host = install_directory();
            let display = mount_directory(&host, "item");

            for _ in 0..200 {
                if host.subscribe_tags().len() >= 3 {
                    break;
                }
                sleep(5).await;
            }

            // Repeat root is a nested `<tonk-display entity={this}>` (a
            // custom element); the `<p class="after">` follows it.
            host.push_frame(
                "view",
                &rows(&[(
                    "did:key:zView",
                    &[(
                        "display",
                        "<div><tonk-display entity={this} model=item data-id={this}></tonk-display><p class=\"after\">keep me</p></div>",
                    )],
                )]),
            );
            host.push_frame(
                "entity",
                &rows(&[("did:key:zItem1", &[("title", "Hello")])]),
            );

            let after = await_selector(&display, "p.after")
                .await
                .expect("a static sibling after a custom-element repeat root must survive");
            assert_eq!(after.text_content().as_deref(), Some("keep me"));
        }

        // A chrome element binding a host-context attribute
        // (`{dom.host/data-active}`) must receive the host's `data-active`
        // value, even when `data-active` is set on the host before any
        // frame arrives. The binder relies on this to learn which sheet is
        // active: `<tonk-sheet-binder active={dom.host/data-active}>`.
        #[dialog_common::test]
        async fn it_projects_a_host_attribute_set_before_the_frame_into_chrome() {
            let host = install_directory();
            let display = mount_directory(&host, "item");
            // The host carries `data-active` from the start, mirroring the
            // shell threading it in before the directory view resolves.
            display
                .set_attribute("data-active", "id:active-one")
                .unwrap();

            for _ in 0..200 {
                if host.subscribe_tags().len() >= 3 {
                    break;
                }
                sleep(5).await;
            }

            // Chrome `<span data-x={dom.host/data-active}>` (no subject
            // ref, so it renders once) plus a per-instance row.
            host.push_frame(
                "view",
                &rows(&[(
                    "did:key:zView",
                    &[(
                        "display",
                        "<div><span class=\"probe\" data-x={dom.host/data-active}></span><ul><li data-id={this}>{title}</li></ul></div>",
                    )],
                )]),
            );
            host.push_frame(
                "entity",
                &rows(&[("did:key:zItem1", &[("title", "Hello")])]),
            );

            let probe = await_selector(&display, "span.probe")
                .await
                .expect("the chrome probe should render");
            assert_eq!(
                probe.get_attribute("data-x").as_deref(),
                Some("id:active-one"),
                "chrome must project the host's data-active via {{dom.host/data-active}}",
            );
        }

        // Changing `data-active` after mount reprojects it into the chrome
        // in place (no teardown), so a tab switch's persisted active is
        // reflected without a reload.
        #[dialog_common::test]
        async fn it_reprojects_a_host_attribute_change_into_chrome_in_place() {
            let host = install_directory();
            let display = mount_directory(&host, "item");

            for _ in 0..200 {
                if host.subscribe_tags().len() >= 3 {
                    break;
                }
                sleep(5).await;
            }

            host.push_frame(
                "view",
                &rows(&[(
                    "did:key:zView",
                    &[(
                        "display",
                        "<div><span class=\"probe\" data-x={dom.host/data-active}></span><ul><li data-id={this}>{title}</li></ul></div>",
                    )],
                )]),
            );
            host.push_frame(
                "entity",
                &rows(&[("did:key:zItem1", &[("title", "Hello")])]),
            );

            let probe = await_selector(&display, "span.probe")
                .await
                .expect("the chrome probe should render");

            // The instance content rendered: capture it so we can assert it
            // is NOT torn down by the host-attribute change.
            let row = await_selector(&display, "li[data-id]")
                .await
                .expect("the instance row should render");

            display
                .set_attribute("data-active", "id:active-two")
                .unwrap();

            // The change reprojects into the same chrome node in place.
            for _ in 0..200 {
                if probe.get_attribute("data-x").as_deref() == Some("id:active-two") {
                    break;
                }
                sleep(5).await;
            }
            assert_eq!(
                probe.get_attribute("data-x").as_deref(),
                Some("id:active-two"),
                "a data-active change must reproject into chrome",
            );
            // Same node — an in-place update, not a teardown/remount.
            assert!(
                row.is_connected(),
                "the mounted instance must survive a host-attribute change (no teardown)",
            );
        }
    }
}
