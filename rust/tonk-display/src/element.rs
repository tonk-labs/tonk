//! `<tonk-display>` custom-element implementation.
//!
//! Coordinates live data flows for a single rendered entity and
//! mounts dumb-renderer children (`<tonk-view>`, `<tonk-inspector>`)
//! as slides. Two modes:
//!
//! - **Single mode** (when the `view` attribute is set): one
//!   `<tonk-view>` is mounted with the resolved view's `display`
//!   HTML as its children. Every entity frame calls `.render(conclusion)`
//!   on it. View-text edits replace the `<tonk-view>` with a fresh
//!   one carrying the new children.
//!
//! - **Carousel mode** (when no `view` attribute is set): a
//!   `<wa-carousel>` hosts one slide per view (each containing a
//!   `<tonk-view>`) plus a final `<tonk-inspector>` slide. The view
//!   subscription becomes a "views for model" query whose frames
//!   carry every view row; we diff by name. Every entity frame
//!   walks all slides and calls `.render(...)` on each.
//!
//! Subscriptions live in `<tonk-display>` only — slide elements
//! never open their own. One model resolution + one views
//! subscription + one entity subscription per attribute set,
//! regardless of slide count.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use custom_elements::CustomElement;
use js_sys::{Function, Reflect};
use tonk_concept::error::{ErrorDetail, ErrorKind};
use tonk_concept::resolve::{ParsedSource, parse_source, phase1_query};
use tonk_concept::sse::open_sse;
use tonk_schema::conclusion::Conclusion;
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use wasm_bindgen_futures::{JsFuture, spawn_local};
use web_sys::{
    AbortController, CustomEvent, CustomEventInit, Element, Headers, HtmlElement, Node, Request,
    RequestInit, Response, window,
};

use crate::resolve::{entity_query, looks_like_uri, view_query, views_for_model_query};
use crate::state::{self, State};

/// One mounted slide in carousel mode (or the sole slide in
/// single mode). Keyed by view name; `display` is the template
/// HTML the slide's `<tonk-view>` was built with so we can detect
/// content changes and rebuild.
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
    /// Aborts the view (or views-for-model) subscription on
    /// disconnect / attribute change.
    view_abort: Option<AbortController>,
    /// Aborts the entity subscription on disconnect / attribute
    /// change.
    entity_abort: Option<AbortController>,
    /// Last entity conclusion seen; replayed when a fresh slide
    /// is mounted so it picks up the current data without waiting
    /// for the next entity frame.
    last_conclusion: Option<Conclusion>,
    /// `<wa-carousel>` element when in carousel mode, `None` in
    /// single mode.
    carousel: Option<Element>,
    /// Slides keyed by view name. In single mode there's exactly
    /// one entry with whatever name the `view` attribute holds.
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
}

impl Inner {
    fn new() -> Self {
        Self {
            view_abort: None,
            entity_abort: None,
            last_conclusion: None,
            carousel: None,
            slides: BTreeMap::new(),
            notation_source: None,
            notation_item: None,
        }
    }

    fn abort_all(&mut self) {
        if let Some(a) = self.view_abort.take() {
            a.abort();
        }
        if let Some(a) = self.entity_abort.take() {
            a.abort();
        }
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
        &["entity", "model", "view", "space", "branch"]
    }

    fn inject_children(&mut self, _this: &HtmlElement) {}

    fn connected_callback(&mut self, this: &HtmlElement) {
        let host: Element = this.clone().into();
        state::set(&host, State::Loading);

        let state = Rc::new(RefCell::new(Inner::new()));
        *self.inner.borrow_mut() = Some(state.clone());
        start_flows(&host, state);
    }

    fn disconnected_callback(&mut self, _this: &HtmlElement) {
        if let Some(state) = self.inner.borrow_mut().take() {
            state.borrow_mut().abort_all();
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
}

fn already_registered() -> bool {
    let Some(win) = window() else {
        return false;
    };
    !win.custom_elements().get("tonk-display").is_undefined()
}

fn start_flows(host: &Element, state: Rc<RefCell<Inner>>) {
    let host_clone = host.clone();
    let state_clone = state.clone();
    spawn_local(async move {
        if let Err(err) = run(&host_clone, state_clone.clone()).await {
            fail(&host_clone, &state_clone, err);
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
    state::set_error(host, error_title(err.kind), &err.message);
    dispatch_error(host, err);
}

/// Short label shown as the error callout's `<strong>` heading.
fn error_title(kind: ErrorKind) -> &'static str {
    match kind {
        ErrorKind::UnknownSource => "Not found",
        ErrorKind::Network => "Connection failed",
        ErrorKind::Parse => "Couldn't read response",
        ErrorKind::Descriptor => "Invalid configuration",
    }
}

/// Empty the host's DOM and forget every mounted slide / chrome.
fn clear_host(host: &Element, inner: &mut Inner) {
    inner.slides.clear();
    inner.carousel = None;
    inner.notation_source = None;
    inner.notation_item = None;
    host.set_inner_html("");
}

async fn run(host: &Element, state: Rc<RefCell<Inner>>) -> Result<(), ErrorDetail> {
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

    // `model` is still required for v1 — without a concept
    // descriptor we don't know which fields to project on the
    // entity. A future fallback could query every claim on the
    // entity directly; for now, require it.
    let model = host
        .get_attribute("model")
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            ErrorDetail::new(
                ErrorKind::Descriptor,
                "<tonk-display> requires `model` (fallback for missing model is deferred)",
            )
        })?;
    // `view` is optional — its absence triggers carousel mode.
    let view = host.get_attribute("view").filter(|s| !s.is_empty());

    let space = host
        .get_attribute("space")
        .unwrap_or_else(|| "home".to_owned());
    let branch = host
        .get_attribute("branch")
        .unwrap_or_else(|| "main".to_owned());
    let url = format!("/api/repository/{space}/branch/{branch}/query");

    // Phase 1 — resolve the model concept's entity + descriptor.
    let parsed: ParsedSource = parse_source(&model);
    let phase1_body = serde_json::to_string(&phase1_query(&parsed))
        .map_err(|e| ErrorDetail::new(ErrorKind::Parse, format!("phase1 body: {e}")))?;
    let (model_entity, descriptor_json) = phase1_lookup(&url, &phase1_body).await?;

    // Build the view subscription query: pin-by-name in single
    // mode, all-views-for-model in carousel mode.
    let view_q = match view.as_deref() {
        Some(name) => view_query(&model_entity, name)
            .map_err(|e| ErrorDetail::new(ErrorKind::Descriptor, format!("view query: {e}")))?,
        None => views_for_model_query(&model_entity).map_err(|e| {
            ErrorDetail::new(ErrorKind::Descriptor, format!("views-for-model query: {e}"))
        })?,
    };
    let view_body = serde_json::to_string(&view_q)
        .map_err(|e| ErrorDetail::new(ErrorKind::Parse, format!("view body: {e}")))?;

    let entity_q = entity_query(&descriptor_json, &entity)
        .map_err(|e| ErrorDetail::new(ErrorKind::Descriptor, format!("entity query: {e}")))?;
    let entity_body = serde_json::to_string(&entity_q)
        .map_err(|e| ErrorDetail::new(ErrorKind::Parse, format!("entity body: {e}")))?;

    // Always mount the `<wa-carousel>` so the slot geometry is
    // identical regardless of mode. The only user-visible
    // difference is whether navigation arrows show and whether
    // the trailing inspector slide is present — both controlled
    // by whether `view` is set.
    let single_mode = view.is_some();
    ensure_carousel(host, &state, single_mode);

    let view_abort = open_view_stream(&url, &view_body, host.clone(), state.clone()).await?;
    let entity_abort = open_entity_stream(&url, &entity_body, host.clone(), state.clone()).await?;

    {
        let mut s = state.borrow_mut();
        s.view_abort = Some(view_abort);
        s.entity_abort = Some(entity_abort);
    }
    dispatch_event(host, "tonk-display:connected", None);
    Ok(())
}

async fn open_view_stream(
    url: &str,
    body: &str,
    host: Element,
    state: Rc<RefCell<Inner>>,
) -> Result<AbortController, ErrorDetail> {
    let host_for_frame = host.clone();
    let host_for_err = host.clone();
    let state_for_err = state.clone();
    open_sse(
        url,
        body,
        move |frame: &str| {
            let conclusions: Vec<Conclusion> = match serde_json::from_str(frame) {
                Ok(v) => v,
                Err(e) => {
                    fail(
                        &host_for_frame,
                        &state,
                        ErrorDetail::new(ErrorKind::Parse, format!("view frame: {e}")),
                    );
                    return;
                }
            };
            handle_view_frame(&host_for_frame, &state, conclusions);
        },
        move |err: ErrorDetail| {
            fail(&host_for_err, &state_for_err, err);
        },
    )
    .await
}

async fn open_entity_stream(
    url: &str,
    body: &str,
    host: Element,
    state: Rc<RefCell<Inner>>,
) -> Result<AbortController, ErrorDetail> {
    let host_for_frame = host.clone();
    let host_for_err = host.clone();
    let state_for_err = state.clone();
    open_sse(
        url,
        body,
        move |frame: &str| {
            let conclusions: Vec<Conclusion> = match serde_json::from_str(frame) {
                Ok(v) => v,
                Err(e) => {
                    fail(
                        &host_for_frame,
                        &state,
                        ErrorDetail::new(ErrorKind::Parse, format!("entity frame: {e}")),
                    );
                    return;
                }
            };
            handle_entity_frame(&host_for_frame, &state, conclusions);
        },
        move |err: ErrorDetail| {
            fail(&host_for_err, &state_for_err, err);
        },
    )
    .await
}

/// Diff the incoming view frame against currently mounted slides.
/// Slides are keyed by view name; we add/remove/replace as needed,
/// then push the cached entity conclusion into any fresh slide so
/// it has data to render right away.
fn handle_view_frame(host: &Element, state: &Rc<RefCell<Inner>>, conclusions: Vec<Conclusion>) {
    let mut s = state.borrow_mut();

    // Extract (name, display) pairs from the incoming frame. When
    // the `view` attribute is set, the subscription pinned `name`
    // as a constant — so the server doesn't project it back. Fall
    // back to the attribute value for that case. When `view` is
    // absent, the views-for-model query left `name` as a variable
    // and the server fills it.
    let view_attr = host.get_attribute("view").unwrap_or_default();
    let incoming: BTreeMap<String, String> = conclusions
        .into_iter()
        .filter_map(|c| {
            let display = c
                .fields
                .get("display")
                .and_then(|v| v.as_str())
                .map(str::to_owned)?;
            let name = c
                .fields
                .get("name")
                .and_then(|v| v.as_str())
                .map(str::to_owned)
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| view_attr.clone());
            if name.is_empty() {
                None
            } else {
                Some((name, display))
            }
        })
        .collect();

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

    dispatch_event(host, "tonk-display:template", Some(JsValue::from_str("ok")));
}

/// Apply an entity frame: empty → empty state + clear slides;
/// non-empty → cache + render on every slide.
fn handle_entity_frame(host: &Element, state: &Rc<RefCell<Inner>>, conclusions: Vec<Conclusion>) {
    let Some(conclusion) = conclusions.into_iter().next() else {
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
    let event_detail = serde_wasm_bindgen::to_value(&conclusion).unwrap_or(JsValue::NULL);
    dispatch_event(host, "tonk-display:result", Some(event_detail));
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
    let text = crate::notation_format::format(conclusion, &head, None);
    script.set_text_content(Some(&text));
}

/// Ensure the `<wa-carousel>` chrome is mounted. Idempotent —
/// does nothing if the carousel is already set up.
///
/// `single_mode` selects between the two presentation modes:
/// - `true` (when `view` attribute is set): no navigation arrows,
///   no trailing notation slide. The user sees exactly one view
///   rendering.
/// - `false`: navigation arrows enabled, and a trailing
///   `<tonk-notation>` slide so the user can flip to a
///   syntax-highlighted dump of the entity in dialog-yaml
///   notation form.
///
/// The carousel host is mounted either way so the slot geometry
/// is identical between the modes — the only user-visible
/// difference is whether arrows + the notation slide are
/// present.
fn ensure_carousel(host: &Element, state: &Rc<RefCell<Inner>>, single_mode: bool) {
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
    if !single_mode {
        let _ = carousel.set_attribute("navigation", "");

        // Trailing notation slide — only present in carousel
        // mode. Lets the user fall back to a syntax-highlighted
        // entity dump regardless of how many views the model has
        // published. The `<script type="text/tonk-notation">`
        // child carries the source; updating its `textContent` is
        // what `<tonk-notation>` watches via its MutationObserver.
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
    }

    let _ = host.append_child(&carousel);
    s.carousel = Some(carousel);
}

/// Create a `<wa-carousel-item>` wrapping a fresh `<tonk-view>`
/// (initialized with `display` as inner HTML) and insert it into
/// the carousel. In carousel mode it lands *before* the trailing
/// inspector slide so the inspector stays last; in single-view
/// mode there's no inspector to keep behind, so the item just
/// gets appended.
fn mount_view_slide(_host: &Element, inner: &mut Inner, display: &str) -> Option<Slide> {
    let document = window()?.document()?;
    let carousel = inner.carousel.as_ref()?;

    let view_el = document.create_element("tonk-view").ok()?;
    view_el.set_inner_html(display);

    let item = document.create_element("wa-carousel-item").ok()?;
    let _ = item.append_child(&view_el);
    if let Some(trailing) = inner.notation_item.as_ref() {
        let _ = carousel.insert_before(&item, Some(trailing));
    } else {
        let _ = carousel.append_child(&item);
    }

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

/// One-shot Phase-1 lookup. Returns `(this, source)` from the first
/// matching row — `this` is the concept entity URI, `source` is the
/// raw descriptor JSON the worker put in the row's `source` field.
async fn phase1_lookup(url: &str, body: &str) -> Result<(String, String), ErrorDetail> {
    let init = RequestInit::new();
    init.set_method("POST");
    let headers = Headers::new()
        .map_err(|e| ErrorDetail::new(ErrorKind::Network, format!("Headers: {e:?}")))?;
    headers
        .append("content-type", "application/json")
        .map_err(|e| ErrorDetail::new(ErrorKind::Network, format!("content-type: {e:?}")))?;
    headers
        .append("accept", "application/json")
        .map_err(|e| ErrorDetail::new(ErrorKind::Network, format!("accept: {e:?}")))?;
    init.set_headers(&headers);
    init.set_body(&JsValue::from_str(body));

    let request = Request::new_with_str_and_init(url, &init)
        .map_err(|e| ErrorDetail::new(ErrorKind::Network, format!("Request: {e:?}")))?;
    let win = window().ok_or_else(|| ErrorDetail::new(ErrorKind::Network, "no window"))?;
    let resp_value = JsFuture::from(win.fetch_with_request(&request))
        .await
        .map_err(|e| ErrorDetail::new(ErrorKind::Network, format!("fetch: {e:?}")))?;
    let resp: Response = resp_value
        .dyn_into()
        .map_err(|_| ErrorDetail::new(ErrorKind::Network, "fetch did not return Response"))?;
    if !resp.ok() {
        return Err(ErrorDetail::new(
            ErrorKind::Network,
            format!("phase1 HTTP {}", resp.status()),
        ));
    }
    let text = JsFuture::from(
        resp.text()
            .map_err(|e| ErrorDetail::new(ErrorKind::Network, format!("text: {e:?}")))?,
    )
    .await
    .map_err(|e| ErrorDetail::new(ErrorKind::Network, format!("read body: {e:?}")))?;
    let body_text = text
        .as_string()
        .ok_or_else(|| ErrorDetail::new(ErrorKind::Parse, "body was not a string"))?;
    let conclusions: Vec<Conclusion> = serde_json::from_str(&body_text)
        .map_err(|e| ErrorDetail::new(ErrorKind::Parse, format!("parse: {e}")))?;
    let first = conclusions
        .into_iter()
        .next()
        .ok_or_else(|| ErrorDetail::new(ErrorKind::UnknownSource, "no concept matched"))?;
    let source = first
        .fields
        .get("source")
        .and_then(|v| v.as_str())
        .map(str::to_owned)
        .ok_or_else(|| {
            ErrorDetail::new(ErrorKind::Descriptor, "phase1 row missing `source` field")
        })?;
    Ok((first.this, source))
}

fn dispatch_error(host: &Element, err: ErrorDetail) {
    let detail = serde_wasm_bindgen::to_value(&err).unwrap_or(JsValue::NULL);
    dispatch_event(host, "tonk-display:error", Some(detail));
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
