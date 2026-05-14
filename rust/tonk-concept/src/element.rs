//! `<tonk-concept>` custom-element implementation.
//!
//! Wires the static-side modules ([`crate::resolve`],
//! [`crate::template`], [`crate::render`], [`crate::sse`]) into a
//! lifecycle: snapshot the row template once, resolve `source`
//! into a wire query, open an SSE subscription, and diff each
//! frame into the live DOM.

use std::cell::RefCell;
use std::rc::Rc;

use custom_elements::CustomElement;
use wasm_bindgen::JsValue;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::spawn_local;
use web_sys::{AbortController, CustomEvent, CustomEventInit, Element, HtmlElement, window};

use crate::error::{ErrorDetail, ErrorKind};
use crate::render::Renderer;
use crate::resolve::{ParsedSource, parse_source, phase1_query, phase2_query};
use crate::sse::open_sse;
use crate::template::{BindingPlan, extract_plan, snapshot_template};

/// Internal lifecycle state shared across async closures.
struct Inner {
    /// Snapshot from the host's child template — `None` until
    /// `connected_callback` ran successfully.
    plan: Option<BindingPlan>,
    /// The cloneable template fragment.
    template: Option<web_sys::DocumentFragment>,
    /// Where rendered rows are appended.
    container: Option<Element>,
    /// Active diff renderer once Phase 2 has started.
    renderer: Option<Renderer>,
    /// Aborts the in-flight Phase-2 SSE fetch when the element
    /// disconnects or an attribute changes.
    abort: Option<AbortController>,
}

impl Inner {
    fn new() -> Self {
        Self {
            plan: None,
            template: None,
            container: None,
            renderer: None,
            abort: None,
        }
    }
}

/// The custom element. `custom-elements` requires `Default` so we
/// hold no fields directly — the real state lives in `Inner` and
/// is only allocated on `connected_callback` (the only point at
/// which we have a host element to attach to).
#[derive(Default)]
pub struct TonkConcept {
    inner: RefCell<Option<Rc<RefCell<Inner>>>>,
}

impl CustomElement for TonkConcept {
    fn shadow() -> bool {
        false
    }

    fn observed_attributes() -> &'static [&'static str] {
        &["source", "space", "branch"]
    }

    fn inject_children(&mut self, _this: &HtmlElement) {}

    fn connected_callback(&mut self, this: &HtmlElement) {
        let host_element: Element = this.clone().into();
        let snapshot = match snapshot_template(&host_element) {
            Ok(s) => s,
            Err(err) => {
                dispatch_error(&host_element, err);
                return;
            }
        };
        let plan = extract_plan(&snapshot.fragment);

        let state = Rc::new(RefCell::new(Inner::new()));
        {
            let mut s = state.borrow_mut();
            s.plan = Some(plan);
            s.template = Some(snapshot.fragment);
            s.container = Some(snapshot.container);
        }
        *self.inner.borrow_mut() = Some(state.clone());

        start_subscription(&host_element, state);
    }

    fn disconnected_callback(&mut self, _this: &HtmlElement) {
        if let Some(state) = self.inner.borrow_mut().take()
            && let Some(abort) = state.borrow_mut().abort.take()
        {
            abort.abort();
        }
    }

    fn attribute_changed_callback(
        &mut self,
        this: &HtmlElement,
        _name: String,
        _old: Option<String>,
        _new: Option<String>,
    ) {
        let host_element: Element = this.clone().into();
        let Some(state) = self.inner.borrow().clone() else {
            return;
        };
        // Cancel any in-flight stream, drop existing rows, restart.
        {
            let mut s = state.borrow_mut();
            if let Some(abort) = s.abort.take() {
                abort.abort();
            }
            if let Some(mut renderer) = s.renderer.take() {
                renderer.clear();
            }
        }
        start_subscription(&host_element, state);
    }
}

/// Public entry point — registers the element with the page.
pub fn register() {
    if already_registered() {
        return;
    }
    TonkConcept::define("tonk-concept");
}

fn already_registered() -> bool {
    let Some(win) = window() else {
        return false;
    };
    !win.custom_elements().get("tonk-concept").is_undefined()
}

/// Kick off Phase 1 → Phase 2.
fn start_subscription(host: &Element, state: Rc<RefCell<Inner>>) {
    let host = host.clone();
    spawn_local(async move {
        if let Err(err) = subscribe(&host, state).await {
            dispatch_error(&host, err);
        }
    });
}

async fn subscribe(host: &Element, state: Rc<RefCell<Inner>>) -> Result<(), ErrorDetail> {
    let source_attr = host.get_attribute("source").unwrap_or_default();
    if source_attr.is_empty() {
        return Err(ErrorDetail::new(
            ErrorKind::Descriptor,
            "<tonk-concept> requires a `source` attribute",
        ));
    }
    let parsed: ParsedSource = parse_source(&source_attr);

    // Either-or override: if the author sets `space` or `branch`,
    // build an absolute `/api/repository/{space}/branch/{branch}/query`
    // URL — missing half defaults to `home`/`main`. With neither
    // attribute set, a relative `/query` URL lets the service
    // worker's guest-iframe rewrite resolve the request under
    // whatever branch the iframe is actually rooted at, which is
    // rarely `home`/`main`.
    let space_attr = host.get_attribute("space");
    let branch_attr = host.get_attribute("branch");
    let url = match (&space_attr, &branch_attr) {
        (None, None) => "/query".to_owned(),
        _ => format!(
            "/api/repository/{}/branch/{}/query",
            space_attr.as_deref().unwrap_or("home"),
            branch_attr.as_deref().unwrap_or("main"),
        ),
    };

    // Phase 1 — one-shot resolve via plain JSON request.
    let phase1_body = serde_json::to_string(&phase1_query(&parsed))
        .map_err(|e| ErrorDetail::new(ErrorKind::Parse, format!("phase1 body: {e}")))?;
    let descriptor_json = phase1_lookup(&url, &phase1_body).await?;

    // Phase 2 — build the actual subscription query and stream it.
    let phase2 = phase2_query(&descriptor_json, &parsed.filters)
        .map_err(|e| ErrorDetail::new(ErrorKind::Descriptor, format!("phase2: {e}")))?;
    let phase2_body = serde_json::to_string(&phase2)
        .map_err(|e| ErrorDetail::new(ErrorKind::Parse, format!("phase2 body: {e}")))?;

    // Initialise the renderer now that we know the descriptor.
    {
        let mut s = state.borrow_mut();
        let plan = s
            .plan
            .clone()
            .ok_or_else(|| ErrorDetail::new(ErrorKind::Descriptor, "no template plan"))?;
        let template = s
            .template
            .clone()
            .ok_or_else(|| ErrorDetail::new(ErrorKind::Descriptor, "no template fragment"))?;
        let container = s
            .container
            .clone()
            .ok_or_else(|| ErrorDetail::new(ErrorKind::Descriptor, "no render container"))?;
        s.renderer = Some(Renderer::new(plan, template, container));
    }

    let host_for_frame = host.clone();
    let host_for_err = host.clone();
    let state_for_frame = state.clone();
    let state_for_err = state.clone();
    let abort = open_sse(
        &url,
        &phase2_body,
        move |frame: &str| {
            let conclusions: Vec<tonk_schema::conclusion::Conclusion> =
                match serde_json::from_str(frame) {
                    Ok(v) => v,
                    Err(e) => {
                        dispatch_error(
                            &host_for_frame,
                            ErrorDetail::new(ErrorKind::Parse, format!("frame: {e}")),
                        );
                        return;
                    }
                };
            if let Some(renderer) = state_for_frame.borrow_mut().renderer.as_mut() {
                renderer.apply(&conclusions);
            }
            dispatch_event(
                &host_for_frame,
                "tonk-concept:result",
                Some(serde_wasm_bindgen::to_value(&conclusions).unwrap_or(JsValue::NULL)),
            );
        },
        move |err: ErrorDetail| {
            dispatch_error(&host_for_err, err);
            // Drop renderer on persistent stream error.
            state_for_err.borrow_mut().renderer = None;
        },
    )
    .await?;

    state.borrow_mut().abort = Some(abort);
    dispatch_event(host, "tonk-concept:connected", None);
    Ok(())
}

/// Phase-1 helper — POST the query as JSON, parse one
/// `Conclusion`, return its `source` field.
async fn phase1_lookup(url: &str, body: &str) -> Result<String, ErrorDetail> {
    use wasm_bindgen_futures::JsFuture;
    use web_sys::{Headers, Request, RequestInit, Response};

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
    let conclusions: Vec<tonk_schema::conclusion::Conclusion> = serde_json::from_str(&body_text)
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
            ErrorDetail::new(
                ErrorKind::Descriptor,
                "phase1 row missing `source` field — worker may not be on the AnonymousConceptQuery build",
            )
        })?;
    Ok(source)
}

fn dispatch_error(host: &Element, err: ErrorDetail) {
    let detail = serde_wasm_bindgen::to_value(&err).unwrap_or(JsValue::NULL);
    dispatch_event(host, "tonk-concept:error", Some(detail));
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
