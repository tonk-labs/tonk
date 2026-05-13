//! `<tonk-display>` custom-element implementation.
//!
//! Coordinates three live flows for one rendered entity:
//! 1. One-shot resolve of the `model` concept descriptor + entity.
//! 2. SSE subscription on the matching `view` row → template HTML.
//! 3. SSE subscription on the entity → field conclusion.
//!
//! Both subscriptions feed the [`crate::render::Renderer`], which
//! caches the last entity conclusion so either input can fire
//! first or change later without losing the other.

use std::cell::RefCell;
use std::rc::Rc;

use custom_elements::CustomElement;
use tonk_concept::error::{ErrorDetail, ErrorKind};
use tonk_concept::resolve::{ParsedSource, parse_source, phase1_query};
use tonk_concept::sse::open_sse;
use tonk_schema::conclusion::Conclusion;
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use wasm_bindgen_futures::{JsFuture, spawn_local};
use web_sys::{
    AbortController, CustomEvent, CustomEventInit, Element, Headers, HtmlElement, Request,
    RequestInit, Response, window,
};

use crate::render::Renderer;
use crate::resolve::{entity_query, looks_like_uri, view_query};
use crate::state::{self, State};

/// Internal lifecycle state shared across async closures.
struct Inner {
    /// Renderer, once the initial `display` HTML has been resolved.
    renderer: Option<Renderer>,
    /// Aborts the view subscription on disconnect / attribute
    /// change.
    view_abort: Option<AbortController>,
    /// Aborts the entity subscription on disconnect / attribute
    /// change.
    entity_abort: Option<AbortController>,
    /// Last entity conclusion seen; replayed when the template
    /// arrives or swaps.
    last_conclusion: Option<Conclusion>,
    /// Last template HTML seen; held in case the entity stream
    /// races ahead of the view stream.
    pending_template: Option<String>,
}

impl Inner {
    fn new() -> Self {
        Self {
            renderer: None,
            view_abort: None,
            entity_abort: None,
            last_conclusion: None,
            pending_template: None,
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
            s.renderer = None;
            s.last_conclusion = None;
            s.pending_template = None;
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
    let host = host.clone();
    spawn_local(async move {
        if let Err(err) = run(&host, state).await {
            state::set(&host, State::Error);
            dispatch_error(&host, err);
        }
    });
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

    // v1 requires both `model` and `view`. Fallback rendering for
    // omitted attributes is tracked in plan/tonk-display.md and
    // lands as a follow-up.
    let model = host
        .get_attribute("model")
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            ErrorDetail::new(
                ErrorKind::Descriptor,
                "<tonk-display> requires `model` (fallback rendering deferred)",
            )
        })?;
    let view = host
        .get_attribute("view")
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            ErrorDetail::new(
                ErrorKind::Descriptor,
                "<tonk-display> requires `view` (fallback rendering deferred)",
            )
        })?;

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

    // Open both subscriptions.
    let view_q = view_query(&model_entity, &view)
        .map_err(|e| ErrorDetail::new(ErrorKind::Descriptor, format!("view query: {e}")))?;
    let view_body = serde_json::to_string(&view_q)
        .map_err(|e| ErrorDetail::new(ErrorKind::Parse, format!("view body: {e}")))?;

    let entity_q = entity_query(&descriptor_json, &entity)
        .map_err(|e| ErrorDetail::new(ErrorKind::Descriptor, format!("entity query: {e}")))?;
    let entity_body = serde_json::to_string(&entity_q)
        .map_err(|e| ErrorDetail::new(ErrorKind::Parse, format!("entity body: {e}")))?;

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
                    dispatch_error(
                        &host_for_frame,
                        ErrorDetail::new(ErrorKind::Parse, format!("view frame: {e}")),
                    );
                    return;
                }
            };
            handle_view_frame(&host_for_frame, &state, conclusions);
        },
        move |err: ErrorDetail| {
            state::set(&host_for_err, State::Error);
            dispatch_error(&host_for_err, err);
            state_for_err.borrow_mut().renderer = None;
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
                    dispatch_error(
                        &host_for_frame,
                        ErrorDetail::new(ErrorKind::Parse, format!("entity frame: {e}")),
                    );
                    return;
                }
            };
            handle_entity_frame(&host_for_frame, &state, conclusions);
        },
        move |err: ErrorDetail| {
            state::set(&host_for_err, State::Error);
            dispatch_error(&host_for_err, err);
            state_for_err.borrow_mut().renderer = None;
        },
    )
    .await
}

/// Apply a view frame: pick out `display`, build/swap the renderer,
/// and re-apply the cached entity conclusion if one is held.
fn handle_view_frame(host: &Element, state: &Rc<RefCell<Inner>>, conclusions: Vec<Conclusion>) {
    let display = conclusions.into_iter().next().and_then(|c| {
        c.fields
            .get("display")
            .and_then(|v| v.as_str())
            .map(str::to_owned)
    });
    let Some(display) = display else {
        // View row vanished or has no `display` text. Leave the
        // current DOM in place; subsequent entity frames keep
        // patching against it. Authors can react by removing the
        // view assertion entirely → entity stream will continue to
        // fire if the entity is still on the branch.
        return;
    };

    let mut s = state.borrow_mut();
    let cached = s.last_conclusion.clone();
    if let Some(r) = s.renderer.as_mut() {
        r.swap_template(&display);
    } else if let Some(r) = Renderer::new(host.clone(), &display) {
        s.renderer = Some(r);
        if let Some(c) = cached.as_ref() {
            if let Some(r) = s.renderer.as_mut() {
                r.apply(c);
            }
            state::set(host, State::Ready);
        }
    } else {
        s.pending_template = Some(display);
        return;
    }
    dispatch_event(host, "tonk-display:template", Some(JsValue::from_str("ok")));
}

/// Apply an entity frame: empty → empty state + clear; non-empty →
/// cache + apply (if renderer exists yet) and mark ready.
fn handle_entity_frame(host: &Element, state: &Rc<RefCell<Inner>>, conclusions: Vec<Conclusion>) {
    let Some(conclusion) = conclusions.into_iter().next() else {
        let mut s = state.borrow_mut();
        s.last_conclusion = None;
        if let Some(r) = s.renderer.as_mut() {
            r.clear();
        }
        state::set(host, State::Empty);
        return;
    };

    let mut s = state.borrow_mut();
    s.last_conclusion = Some(conclusion.clone());
    if let Some(r) = s.renderer.as_mut() {
        r.apply(&conclusion);
        state::set(host, State::Ready);
        let detail = serde_wasm_bindgen::to_value(&conclusion).unwrap_or(JsValue::NULL);
        dispatch_event(host, "tonk-display:result", Some(detail));
    }
    // If renderer isn't built yet, the view stream will pick up the
    // cached conclusion when it arrives.
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
