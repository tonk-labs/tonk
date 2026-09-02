//! `<tonk-upload>` — a headless native blob-upload primitive.
//!
//! The mechanism (file input, ingest fetch, state machine, preview) lives in
//! Shadow DOM; the author restyles it via the `trigger` slot, `::part()`
//! (`base`/`button`/`preview`/`status`), `--tonk-upload-*` CSS vars, and the
//! reflected `data-state` attribute. Bytes are ingested through the worker
//! `POST …/blob` route (relayed by the guest's `window.fetch`), and a
//! successful upload dispatches a bubbling `tonk-upload` CustomEvent carrying
//! the resulting `blob:<hash>`.

use std::cell::RefCell;
use std::rc::Rc;

use custom_elements::CustomElement;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::{JsFuture, spawn_local};
use web_sys::{Element, HtmlElement, ShadowRoot};

/// Per-instance state: the shadow root plus the change/click listener closures,
/// kept alive for the element's lifetime.
#[derive(Default)]
struct Inner {
    shadow: Option<ShadowRoot>,
    listeners: Vec<Closure<dyn FnMut(web_sys::Event)>>,
}

type Shared = Rc<RefCell<Inner>>;

#[derive(Default)]
pub struct TonkUploadElement {
    state: Shared,
}

impl CustomElement for TonkUploadElement {
    fn shadow() -> bool {
        // We attach our own shadow root so we control build timing.
        false
    }

    fn observed_attributes() -> &'static [&'static str] {
        &["with", "accept"]
    }

    fn inject_children(&mut self, _this: &HtmlElement) {}

    fn connected_callback(&mut self, this: &HtmlElement) {
        set_state(this, "idle");
        build_shadow(this, &self.state);
    }

    fn disconnected_callback(&mut self, _this: &HtmlElement) {
        *self.state.borrow_mut() = Inner::default();
    }

    fn attribute_changed_callback(
        &mut self,
        _this: &HtmlElement,
        name: String,
        old: Option<String>,
        new: Option<String>,
    ) {
        if old == new {
            return;
        }
        // `accept` propagates to the input; `with` only affects the next upload.
        if name == "accept"
            && let Some(root) = self.state.borrow().shadow.as_ref()
            && let Ok(Some(input)) = root.query_selector("input[type=file]")
        {
            let _ = input.set_attribute("accept", new.as_deref().unwrap_or(""));
        }
    }
}

/// Reflect the state machine onto the host for `data-state` CSS.
fn set_state(this: &HtmlElement, state: &str) {
    let _ = this.set_attribute("data-state", state);
}

fn ensure_shadow(this: &HtmlElement) -> ShadowRoot {
    if let Some(root) = this.shadow_root() {
        return root;
    }
    let init = web_sys::ShadowRootInit::new(web_sys::ShadowRootMode::Open);
    this.attach_shadow(&init).unwrap()
}

fn doc() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

fn el(tag: &str) -> Element {
    doc().create_element(tag).unwrap()
}

/// Build the default shadow UI once, and wire the trigger + file input.
fn build_shadow(this: &HtmlElement, state: &Shared) {
    let root = ensure_shadow(this);

    let style = el("style");
    style.set_text_content(Some(STYLE));
    let _ = root.append_child(&style);

    let base = el("div");
    let _ = base.set_attribute("part", "base");

    // Hidden real input.
    let input = el("input");
    let _ = input.set_attribute("type", "file");
    let _ = input.set_attribute("hidden", "");
    if let Some(accept) = this.get_attribute("accept") {
        let _ = input.set_attribute("accept", &accept);
    }
    let _ = base.append_child(&input);

    // Trigger slot (fallback = default button).
    let slot = el("slot");
    let _ = slot.set_attribute("name", "trigger");
    let button = el("button");
    let _ = button.set_attribute("part", "button");
    let _ = button.set_attribute("type", "button");
    button.set_text_content(Some("Choose file…"));
    let _ = slot.append_child(&button);
    let _ = base.append_child(&slot);

    // Preview (element-owned part; hidden until an upload).
    let preview = el("img");
    let _ = preview.set_attribute("part", "preview");
    let _ = preview.set_attribute("hidden", "");
    let _ = base.append_child(&preview);

    // Status (element-owned part).
    let status = el("span");
    let _ = status.set_attribute("part", "status");
    let _ = base.append_child(&status);

    let _ = root.append_child(&base);
    state.borrow_mut().shadow = Some(root);

    // Disable the trigger when there is no usable space context.
    let with_ok =
        crate::blob_url::branch_repo(&this.get_attribute("with").unwrap_or_default()).is_some();
    if !with_ok {
        let _ = button.set_attribute("disabled", "");
        status.set_text_content(Some("no space context"));
    }

    // Clicking anywhere on the trigger slot (default button or author-slotted
    // content) opens the native picker — unless `with` is missing.
    {
        let input_for_click = input.clone();
        let click = Closure::wrap(Box::new(move |_e: web_sys::Event| {
            if with_ok {
                input_for_click.unchecked_ref::<HtmlElement>().click();
            }
        }) as Box<dyn FnMut(web_sys::Event)>);
        let _ = slot.add_event_listener_with_callback("click", click.as_ref().unchecked_ref());
        state.borrow_mut().listeners.push(click);
    }

    // A picked file drives the upload flow.
    {
        let host = this.clone();
        let change = Closure::wrap(Box::new(move |e: web_sys::Event| {
            if let Some(target) = e.target()
                && let Ok(input) = target.dyn_into::<web_sys::HtmlInputElement>()
                && let Some(file) = input.files().and_then(|l| l.get(0))
            {
                ingest(&host, &file);
            }
        }) as Box<dyn FnMut(web_sys::Event)>);
        let _ = input.add_event_listener_with_callback("change", change.as_ref().unchecked_ref());
        state.borrow_mut().listeners.push(change);
    }
}

/// Read `file` to bytes, POST it to the ingest route, swap the preview to the
/// read route, and emit `tonk-upload` on success. All async; drives `data-state`.
fn ingest(host: &HtmlElement, file: &web_sys::File) {
    let host = host.clone();
    let file = file.clone();
    spawn_local(async move {
        let with = host.get_attribute("with").unwrap_or_default();
        let Some((branch, repo)) = crate::blob_url::branch_repo(&with) else {
            fail(&host, "no space context");
            return;
        };
        let name = file.name();
        let mime = {
            let t = file.type_();
            if t.is_empty() {
                "application/octet-stream".to_string()
            } else {
                t
            }
        };

        set_state(&host, "reading");
        if mime.starts_with("image/")
            && let Ok(url) = web_sys::Url::create_object_url_with_blob(&file)
        {
            show_preview(&host, &url);
        }

        let buf = match JsFuture::from(file.array_buffer()).await {
            Ok(b) => b,
            Err(_) => return fail(&host, "could not read file"),
        };
        let bytes = js_sys::Uint8Array::new(&buf);

        set_state(&host, "uploading");
        let url = format!("/api/repository/{repo}/branch/{branch}/blob");
        let headers = web_sys::Headers::new().unwrap();
        let _ = headers.append("content-type", &mime);
        let _ = headers.append("x-tonk-blob-name", &name);
        let init = web_sys::RequestInit::new();
        init.set_method("POST");
        init.set_headers(&headers);
        init.set_body(&bytes.into());
        let win = web_sys::window().unwrap();
        let resp_val = match JsFuture::from(win.fetch_with_str_and_init(&url, &init)).await {
            Ok(v) => v,
            Err(_) => return fail(&host, "upload failed"),
        };
        let resp: web_sys::Response = match resp_val.dyn_into() {
            Ok(r) => r,
            Err(_) => return fail(&host, "upload failed"),
        };
        if !resp.ok() {
            return fail(&host, &format!("upload failed ({})", resp.status()));
        }
        let text = match resp.text() {
            Ok(p) => match JsFuture::from(p).await {
                Ok(t) => t.as_string().unwrap_or_default(),
                Err(_) => return fail(&host, "bad response"),
            },
            Err(_) => return fail(&host, "bad response"),
        };
        let json: serde_json::Value = match serde_json::from_str(&text) {
            Ok(j) => j,
            Err(_) => return fail(&host, "bad response"),
        };
        let entity = json["entity"].as_str().unwrap_or_default().to_string();
        if entity.is_empty() {
            return fail(&host, "bad response");
        }
        let content_type = json["contentType"].as_str().unwrap_or(&mime).to_string();
        let size = json["size"].as_u64().unwrap_or(0);

        // Read the image back FROM THE DB and preview those bytes — the whole
        // round-trip in one go. Fetch the worker blob route through the relayed
        // `window.fetch` and swap the preview to that object URL (a native
        // `<img src>` load of the route would bypass the relay + service worker
        // inside the sealed guest). On failure the reading-phase local preview
        // stays put. Non-images get a status line instead.
        if content_type.starts_with("image/") {
            if let Some(read) = crate::blob_url::blob_read_url(&with, &entity)
                && let Some(obj) = crate::blob_url::fetch_object_url(&read).await
            {
                show_preview(&host, &obj);
            }
        } else {
            set_status(&host, &format!("{name} · {size} bytes"));
        }

        set_state(&host, "done");
        emit(&host, &entity, &content_type, &name, size);
    });
}

fn show_preview(host: &HtmlElement, src: &str) {
    if let Some(root) = host.shadow_root()
        && let Ok(Some(img)) = root.query_selector("[part=preview]")
    {
        let prior = img.get_attribute("src");
        let _ = img.set_attribute("src", src);
        let _ = img.remove_attribute("hidden");
        // Free the object URL we just replaced (the reading-phase local
        // preview when swapping to the DB read-back), once nothing references it.
        if let Some(prior) = prior
            && prior.starts_with("blob:")
            && prior != src
        {
            let _ = web_sys::Url::revoke_object_url(&prior);
        }
    }
}

fn set_status(host: &HtmlElement, msg: &str) {
    if let Some(root) = host.shadow_root()
        && let Ok(Some(s)) = root.query_selector("[part=status]")
    {
        s.set_text_content(Some(msg));
    }
}

fn fail(host: &HtmlElement, msg: &str) {
    set_state(host, "error");
    set_status(host, msg);
    if let Some(root) = host.shadow_root()
        && let Ok(Some(img)) = root.query_selector("[part=preview]")
    {
        let _ = img.set_attribute("hidden", "");
    }
}

fn emit(host: &HtmlElement, blob: &str, content_type: &str, name: &str, size: u64) {
    let detail = js_sys::Object::new();
    let _ = js_sys::Reflect::set(&detail, &"blob".into(), &blob.into());
    let _ = js_sys::Reflect::set(&detail, &"contentType".into(), &content_type.into());
    let _ = js_sys::Reflect::set(&detail, &"name".into(), &name.into());
    let _ = js_sys::Reflect::set(&detail, &"size".into(), &JsValue::from_f64(size as f64));
    let init = web_sys::CustomEventInit::new();
    init.set_bubbles(true);
    init.set_composed(true);
    init.set_detail(&detail);
    if let Ok(ev) = web_sys::CustomEvent::new_with_event_init_dict("tonk-upload", &init) {
        let _ = host.dispatch_event(&ev);
    }
}

/// Test-only hook: inject a `File` as if the user had picked it (browsers don't
/// let script set `input.files`, and there's no real OS picker in tests).
#[cfg(all(test, target_arch = "wasm32"))]
pub(crate) fn __test_ingest(host: &HtmlElement, file: &web_sys::File) {
    ingest(host, file);
}

const STYLE: &str = r#"
:host { display: inline-flex; }
[part=base] {
  display: inline-flex; align-items: center; gap: var(--tonk-upload-gap, var(--wa-space-s, 8px));
  font-family: var(--wa-font-family-body, system-ui, sans-serif);
  color: var(--wa-color-text-normal, #38182a);
}
/* A neutral bordered control matching the sheets/board/wiki templates:
   a surface fill with a subtle border that brightens on hover — no loud
   brand accent. `--tonk-upload-accent` still lets an author tint the
   border. */
[part=button] {
  display: inline-flex; align-items: center; gap: var(--wa-space-xs, 6px);
  padding: var(--wa-space-xs, 6px) var(--wa-space-m, 12px);
  font: inherit; font-size: var(--wa-font-size-s, 14px);
  color: var(--wa-color-text-normal, #38182a);
  background: var(--wa-color-surface-raised, #fcfbfb);
  border: var(--wa-border-width-s, 1px) solid
    var(--tonk-upload-accent, var(--wa-color-surface-border, rgb(56 24 42 / 28%)));
  border-radius: var(--tonk-upload-radius, var(--wa-border-radius-s, 0));
  cursor: pointer;
  transition-property: color, background-color, border-color;
  transition-duration: 120ms;
  transition-timing-function: cubic-bezier(0.2, 0, 0, 1);
}
[part=button]:hover {
  border-color: var(--wa-color-text-normal, #38182a);
  background: var(--wa-color-neutral-fill-quiet, #f4f4f4);
}
[part=button][disabled] { opacity: 0.5; cursor: not-allowed; }
[part=button][disabled]:hover {
  border-color: var(--wa-color-surface-border, rgb(56 24 42 / 28%));
  background: var(--wa-color-surface-raised, #fcfbfb);
}
[part=preview] {
  max-width: 96px; max-height: 96px;
  border-radius: var(--wa-border-radius-s, 0);
  border: var(--wa-border-width-s, 1px) solid var(--wa-color-surface-border, rgb(56 24 42 / 28%));
}
[part=preview][hidden] { display: none; }
[part=status] {
  font-size: var(--wa-font-size-s, 13px);
  color: var(--wa-color-text-quiet, #5b4953);
}
/* One ink (DESIGN.md): the error is worded, not colored. */
:host([data-state=error]) [part=status] { color: var(--wa-color-text-normal, #38182a); }
"#;

/// Register the `<tonk-upload>` element. Idempotent.
pub fn register() {
    if already_registered() {
        return;
    }
    TonkUploadElement::define("tonk-upload");
}

fn already_registered() -> bool {
    doc()
        .default_view()
        .map(|w| !w.custom_elements().get("tonk-upload").is_undefined())
        .unwrap_or(false)
}

#[cfg(all(test, target_arch = "wasm32"))]
mod tests {
    use wasm_bindgen::closure::Closure;
    use wasm_bindgen::{JsCast, JsValue};
    use wasm_bindgen_futures::JsFuture;
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    wasm_bindgen_test_configure!(run_in_browser);

    use std::cell::RefCell;
    use std::rc::Rc;

    fn document() -> web_sys::Document {
        web_sys::window().unwrap().document().unwrap()
    }

    async fn sleep(ms: i32) {
        let p = js_sys::Promise::new(&mut |res, _| {
            let _ = web_sys::window()
                .unwrap()
                .set_timeout_with_callback_and_timeout_and_arguments_0(&res, ms);
        });
        let _ = JsFuture::from(p).await;
    }

    async fn await_input(host: &web_sys::HtmlElement) -> web_sys::ShadowRoot {
        for _ in 0..200 {
            if let Some(r) = host.shadow_root() {
                if r.query_selector("input[type=file]").unwrap().is_some() {
                    return r;
                }
            }
            sleep(5).await;
        }
        panic!("shadow input never appeared");
    }

    fn make_file(bytes: &[u8], name: &str, mime: &str) -> web_sys::File {
        let parts = js_sys::Array::new();
        parts.push(&js_sys::Uint8Array::from(bytes).into());
        let opts = web_sys::FilePropertyBag::new();
        opts.set_type(mime);
        web_sys::File::new_with_u8_array_sequence_and_options(&parts, name, &opts).unwrap()
    }

    /// Replace `window.fetch` with a stub returning `status` + `json`.
    fn stub_fetch(status: u16, json: &'static str) {
        stub_fetch_capturing(status, json, Rc::new(RefCell::new(Vec::new())));
    }

    /// Like [`stub_fetch`], but records every requested URL into `urls` — so a
    /// test can assert which routes were hit (e.g. the read-back GET).
    fn stub_fetch_capturing(status: u16, json: &'static str, urls: Rc<RefCell<Vec<String>>>) {
        let stub = Closure::wrap(Box::new(move |url: JsValue, _init: JsValue| {
            if let Some(u) = url.as_string() {
                urls.borrow_mut().push(u);
            }
            let init = web_sys::ResponseInit::new();
            init.set_status(status);
            let resp = web_sys::Response::new_with_opt_str_and_init(Some(json), &init).unwrap();
            js_sys::Promise::resolve(&JsValue::from(resp))
        })
            as Box<dyn FnMut(JsValue, JsValue) -> js_sys::Promise>);
        js_sys::Reflect::set(
            &web_sys::window().unwrap(),
            &"fetch".into(),
            stub.as_ref().unchecked_ref(),
        )
        .unwrap();
        stub.forget();
    }

    #[dialog_common::test]
    async fn it_builds_a_default_shadow_ui() {
        super::register();
        let el = document().create_element("tonk-upload").unwrap();
        el.set_attribute("with", "main@did:key:zSpace").unwrap();
        document().body().unwrap().append_child(&el).unwrap();
        let host: web_sys::HtmlElement = el.dyn_into().unwrap();
        let root = await_input(&host).await;
        assert!(root.query_selector("[part=button]").unwrap().is_some());
        assert!(root.query_selector("[part=preview]").unwrap().is_some());
        assert!(root.query_selector("[part=status]").unwrap().is_some());
        assert_eq!(host.get_attribute("data-state").as_deref(), Some("idle"));
    }

    #[dialog_common::test]
    async fn it_uploads_and_emits_on_pick() {
        super::register();
        let urls: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
        stub_fetch_capturing(
            200,
            r#"{"entity":"blob:zH","contentType":"image/png","name":"a.png","size":3}"#,
            urls.clone(),
        );

        let el = document().create_element("tonk-upload").unwrap();
        el.set_attribute("with", "main@repo").unwrap();
        document().body().unwrap().append_child(&el).unwrap();
        let host: web_sys::HtmlElement = el.dyn_into().unwrap();

        let got: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
        {
            let got = got.clone();
            let cb = Closure::wrap(Box::new(move |ev: web_sys::CustomEvent| {
                let blob = js_sys::Reflect::get(&ev.detail(), &"blob".into())
                    .ok()
                    .and_then(|v| v.as_string());
                *got.borrow_mut() = blob;
            }) as Box<dyn FnMut(web_sys::CustomEvent)>);
            host.add_event_listener_with_callback("tonk-upload", cb.as_ref().unchecked_ref())
                .unwrap();
            cb.forget();
        }

        let _ = await_input(&host).await;
        super::__test_ingest(&host, &make_file(b"abc", "a.png", "image/png"));

        for _ in 0..200 {
            if got.borrow().is_some() {
                break;
            }
            sleep(5).await;
        }
        assert_eq!(got.borrow().as_deref(), Some("blob:zH"));
        assert_eq!(host.get_attribute("data-state").as_deref(), Some("done"));
        let preview = host
            .shadow_root()
            .unwrap()
            .query_selector("[part=preview]")
            .unwrap()
            .unwrap();
        // After upload the preview is read back FROM THE DB: the bytes are
        // fetched from the worker blob route through the relayed `window.fetch`
        // and shown as an object URL (a native `<img src>` load of the route
        // would bypass the relay + service worker inside the sealed guest).
        let src = preview.get_attribute("src").expect("preview has a src");
        assert!(
            src.starts_with("blob:") && src.contains('/'),
            "preview src is an object URL: {src}",
        );
        assert!(
            urls.borrow()
                .iter()
                .any(|u| u == "/api/repository/repo/branch/main/blob/blob:zH"),
            "the preview was read back from the DB via the blob route, got: {:?}",
            urls.borrow(),
        );
    }

    #[dialog_common::test]
    async fn it_errors_without_emitting_on_failure() {
        super::register();
        stub_fetch(500, r#"{"error":"boom"}"#);

        let el = document().create_element("tonk-upload").unwrap();
        el.set_attribute("with", "main@repo").unwrap();
        document().body().unwrap().append_child(&el).unwrap();
        let host: web_sys::HtmlElement = el.dyn_into().unwrap();

        let fired = Rc::new(RefCell::new(false));
        {
            let fired = fired.clone();
            let cb = Closure::wrap(Box::new(move |_ev: web_sys::CustomEvent| {
                *fired.borrow_mut() = true;
            }) as Box<dyn FnMut(web_sys::CustomEvent)>);
            host.add_event_listener_with_callback("tonk-upload", cb.as_ref().unchecked_ref())
                .unwrap();
            cb.forget();
        }

        let _ = await_input(&host).await;
        super::__test_ingest(&host, &make_file(b"abc", "a.png", "image/png"));

        for _ in 0..200 {
            if host.get_attribute("data-state").as_deref() == Some("error") {
                break;
            }
            sleep(5).await;
        }
        assert_eq!(host.get_attribute("data-state").as_deref(), Some("error"));
        assert!(!*fired.borrow(), "no tonk-upload event on failure");
    }

    #[dialog_common::test]
    async fn it_projects_a_slotted_trigger() {
        super::register();
        let el = document().create_element("tonk-upload").unwrap();
        el.set_attribute("with", "main@repo").unwrap();
        let btn = document().create_element("button").unwrap();
        btn.set_attribute("slot", "trigger").unwrap();
        btn.set_text_content(Some("Pick"));
        el.append_child(&btn).unwrap();
        document().body().unwrap().append_child(&el).unwrap();
        let host: web_sys::HtmlElement = el.dyn_into().unwrap();
        let _ = await_input(&host).await;

        // The author's light-DOM trigger is present and assigned to the slot.
        let slotted = host
            .query_selector("button[slot=trigger]")
            .unwrap()
            .unwrap();
        assert_eq!(slotted.text_content().as_deref(), Some("Pick"));
    }
}
