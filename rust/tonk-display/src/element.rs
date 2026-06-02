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
use std::collections::BTreeMap;
use std::rc::Rc;

use custom_elements::CustomElement;
use ipld_core::ipld::Ipld;
use js_sys::{Function, Reflect};
use tonk_concept::resolve::{ParsedSource, parse_source, phase1_query};
use tonk_host::consumer::{self as host_consumer, Subscription as HostSubscription};
use tonk_host::error::{ErrorDetail, ErrorKind};
use tonk_host::install_depth_annotator;
use tonk_schema::conclusion::Conclusion;
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use wasm_bindgen_futures::spawn_local;
use web_sys::{CustomEvent, CustomEventInit, Element, HtmlElement, Node, window};

use crate::resolve::{entity_query, looks_like_uri, view_by_model_query, view_predicate};
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
    /// Cancels the view (or views-for-model) subscription on
    /// disconnect / attribute change. Dropping the handle calls
    /// the host's `cancel()` and dispatches `tonk-unsubscribe`.
    view_sub: Option<HostSubscription>,
    /// Cancels the entity subscription on disconnect / attribute
    /// change.
    entity_sub: Option<HostSubscription>,
    /// Last entity conclusion seen; replayed when a fresh slide
    /// is mounted so it picks up the current data without waiting
    /// for the next entity frame.
    last_conclusion: Option<Conclusion>,
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
}

impl Inner {
    fn new() -> Self {
        Self {
            disposed: false,
            generation: 0,
            view_sub: None,
            entity_sub: None,
            last_conclusion: None,
            carousel: None,
            slides: BTreeMap::new(),
            notation_source: None,
            notation_item: None,
            delegate: None,
            delegate_generation: 0,
            depth_annotator: None,
        }
    }

    fn abort_all(&mut self) {
        // Dropping the subscriptions cancels via the host and
        // dispatches `tonk-unsubscribe`.
        self.view_sub.take();
        self.entity_sub.take();
        // Drop the delegate — its impl removes listeners from the
        // host on Drop.
        self.delegate.take();
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
        &["entity", "model", "view"]
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
        _name: String,
        _old: Option<String>,
        _new: Option<String>,
    ) {
        let host: Element = this.clone().into();
        let Some(state) = self.inner.borrow().clone() else {
            return;
        };
        {
            let mut s = state.borrow_mut();
            s.abort_all();
            s.last_conclusion = None;
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
    match tag.as_deref() {
        Some("view") => handle_view_frame(host, state, conclusions),
        Some("entity") => handle_entity_frame(host, state, conclusions),
        _ => {
            // Unknown tag — log and drop.
        }
    }
}

/// `update(delta, { tag })` — incremental change. V1 SW does not
/// emit this; we'd treat it as `reset` on whatever the frame
/// implies. For now log and drop.
fn on_update(_host: &Element, _state: &Rc<RefCell<Inner>>, _payload: JsValue, _opts: JsValue) {
    // V1 SW emits only `reset`. Once delta semantics arrive,
    // this handler applies the delta to the slide state.
}

/// `error(detail, { tag })` — transport / parse error on a
/// subscription. Surface as a host-level failure.
fn on_error(host: &Element, state: &Rc<RefCell<Inner>>, payload: JsValue, _opts: JsValue) {
    let message = Reflect::get(&payload, &"message".into())
        .ok()
        .and_then(|v| v.as_string())
        .unwrap_or_else(|| format!("{payload:?}"));
    fail(host, state, ErrorDetail::new(ErrorKind::Network, message));
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
    state::set_error(host, state::error_title(err.kind), &err.message);
    dispatch_error(host, err);
}

/// Empty the host's DOM and forget every mounted slide / chrome.
fn clear_host(host: &Element, inner: &mut Inner) {
    inner.slides.clear();
    inner.carousel = None;
    inner.notation_source = None;
    inner.notation_item = None;
    host.set_inner_html("");
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

async fn run(
    host: &Element,
    state: Rc<RefCell<Inner>>,
    generation: u64,
) -> Result<(), ErrorDetail> {
    let entity = host
        .get_attribute("entity")
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            ErrorDetail::new(ErrorKind::Descriptor, "<tonk-display> requires `entity`")
        })?;
    if !looks_like_uri(&entity) {
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
    let (model_entity, descriptor_json) = resolve_model(host, &model).await?;
    check_generation(&state, generation)?;

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

    // Resolve the view *concept*'s descriptor — the query predicate.
    // When `view` is omitted, the built-in `view` concept's
    // descriptor is known (`view_predicate`), so no resolve is
    // needed; a named/URI view concept is resolved from the branch.
    // The descriptor maps the `display` field name to whatever
    // attribute that view kind declares it under, so `view_by_model`
    // reads the right template regardless of the kind.
    let view_descriptor: serde_json::Value = match view.as_deref() {
        None => view_predicate(),
        Some(view_ref) => {
            let (_, view_descriptor_json) = resolve_model(host, view_ref).await?;
            check_generation(&state, generation)?;
            serde_json::from_str(&view_descriptor_json).map_err(|e| {
                ErrorDetail::new(ErrorKind::Descriptor, format!("view descriptor: {e}"))
            })?
        }
    };
    let view_q = view_by_model_query(&view_descriptor, &model_entity).map_err(|e| {
        ErrorDetail::new(ErrorKind::Descriptor, format!("view-by-model query: {e}"))
    })?;
    let view_body = to_body(&view_q)?;

    let entity_q = entity_query(&descriptor_json, &entity)
        .map_err(|e| ErrorDetail::new(ErrorKind::Descriptor, format!("entity query: {e}")))?;
    let entity_body = to_body(&entity_q)?;

    // The view query is model-constrained, so it resolves to one
    // presentation. Mount in single mode.
    ensure_carousel(host, &state, true);

    // Open the two subscriptions via the host. Frames arrive
    // through our `__tonkReset` delegate, which routes to
    // `handle_view_frame` or `handle_entity_frame` by `opts.tag`.
    let view_tag = JsValue::from_str("view");
    let view_sub = host_consumer::subscribe(host, &view_body, Some(&view_tag))?;
    if check_generation(&state, generation).is_err() {
        return Err(ErrorDetail::new(ErrorKind::Descriptor, "superseded"));
    }
    let entity_tag = JsValue::from_str("entity");
    let entity_sub = host_consumer::subscribe(host, &entity_body, Some(&entity_tag))?;

    {
        let mut s = state.borrow_mut();
        // Final generation check — if a newer flow ran between
        // opening the two subscriptions, drop them so we don't
        // orphan them (the newer flow's subscriptions are already
        // stored).
        if s.generation != generation {
            return Err(ErrorDetail::new(ErrorKind::Descriptor, "superseded"));
        }
        s.view_sub = Some(view_sub);
        s.entity_sub = Some(entity_sub);
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
fn to_body(query: &tonk_schema::query::Query) -> Result<JsValue, ErrorDetail> {
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
/// `(entity, descriptor_json)` via the Phase-1 concept-of-concepts
/// query.
async fn resolve_model(host: &Element, source: &str) -> Result<(String, String), ErrorDetail> {
    let parsed: ParsedSource = parse_source(source);
    let phase1_q = phase1_query(&parsed);
    let result = host_consumer::query(host, &to_body(&phase1_q)?).await?;
    extract_phase1(&result)
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
    let cached = s.last_conclusion.clone();
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
            // Push the cached entity conclusion if we have one,
            // so the new slide renders immediately rather than
            // waiting for the next entity frame.
            if let Some(c) = cached.as_ref() {
                call_render(&new_slide.view_el, &serialize_conclusion(c));
            }
            s.slides.insert(name, new_slide);
        }
    }

    // In single mode, mark Ready once the slide is mounted (with
    // or without an entity frame yet — empty content is fine).
    // In carousel mode, ensure the notation slide is fed.
    if cached.is_some() && !s.slides.is_empty() {
        state::set(host, State::Ready);
    }
    if let Some(c) = cached.as_ref() {
        update_notation(host, &s, c);
    }

    // Drop the borrow before spawning the delegate refresh — its
    // descriptor resolves are async and reacquire the state.
    drop(s);
    schedule_delegate_refresh(host, state);

    dispatch_event(host, "tonk-display:template", Some(JsValue::from_str("ok")));
}

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
    let host = host.clone();
    let state = state.clone();
    spawn_local(async move {
        refresh_delegate(&host, &state, delegate_generation).await;
    });
}

async fn refresh_delegate(host: &Element, state: &Rc<RefCell<Inner>>, delegate_generation: u64) {
    use crate::events::delegate::{Delegate, Descriptors};
    use std::collections::BTreeSet;

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
        let parsed = parse_source(name);
        let q = phase1_query(&parsed);
        let body_val = match serde_wasm_bindgen::to_value(&q) {
            Ok(v) => v,
            Err(_) => continue,
        };
        match host_consumer::query(host, &body_val).await {
            Ok(result) => {
                if let Ok((_entity, descriptor_json)) = extract_phase1(&result) {
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
}

/// Apply an entity frame: empty → empty state + clear slides;
/// non-empty → fold rows + cache + render on every slide.
///
/// The fold collapses N rows for the same entity (one per tuple
/// the worker emits for cardinality-many attributes) into a single
/// conclusion whose differing fields become `Array` values. The
/// template renderer's iteration-aware walk does the per-value
/// cloning from there.
fn handle_entity_frame(host: &Element, state: &Rc<RefCell<Inner>>, conclusions: Vec<Conclusion>) {
    let Some(conclusion) = crate::fold::fold_rows(conclusions) else {
        let mut s = state.borrow_mut();
        s.last_conclusion = None;
        clear_host(host, &mut s);
        state::set(host, State::Empty);
        return;
    };

    let mut s = state.borrow_mut();
    s.last_conclusion = Some(conclusion.clone());
    let detail = serialize_conclusion(&conclusion);
    for slide in s.slides.values() {
        call_render(&slide.view_el, &detail);
    }
    update_notation(host, &s, &conclusion);
    if !s.slides.is_empty() || s.notation_source.is_some() {
        state::set(host, State::Ready);
    }
    dispatch_event(host, "tonk-display:result", Some(event_detail(&conclusion)));
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
fn mount_view_slide(host: &Element, inner: &mut Inner, display: &str) -> Option<Slide> {
    let document = window()?.document()?;

    let view_el = document.create_element("tonk-view").ok()?;
    view_el.set_inner_html(display);

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
fn call_render(el: &Element, detail: &JsValue) {
    let Ok(draw) = Reflect::get(el.as_ref(), &"draw".into()) else {
        return;
    };
    let Ok(func) = draw.dyn_into::<Function>() else {
        return;
    };
    let _ = func.call1(&JsValue::NULL, detail);
}

/// Serialize a `Conclusion` into a JsValue with the shape
/// `<tonk-view>` / `<tonk-inspector>` expect.
fn serialize_conclusion(conclusion: &Conclusion) -> JsValue {
    serde_wasm_bindgen::to_value(conclusion).unwrap_or(JsValue::NULL)
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
}
