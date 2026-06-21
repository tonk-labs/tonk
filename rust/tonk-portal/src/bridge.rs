//! The live-data bridge injected into a portal's iframe.
//!
//! A portal mounts an **opaque-origin** iframe (`sandbox="allow-scripts"`)
//! and prepends a small bootstrap script to its document. That script
//! defines `window.tonk` synchronously, opens a [`MessageChannel`], and
//! posts a `hello` envelope to its parent transferring one port. The
//! iframe keeps the other port; thereafter author code and the parent
//! communicate only over that port.
//!
//! [`MessageChannel`]: https://developer.mozilla.org/docs/Web/API/MessageChannel
//!
//! The author-facing object is unchanged:
//!
//! ```text
//! window.tonk = {
//!   context: { this, model },
//!   query(body?)      -> Promise<Conclusion[]>,
//!   subscribe(body?)  -> ReadableStream<Conclusion[]>,
//!   transact(request) -> Promise<receipt>,
//!   ready: Promise<void>,
//! }
//! ```
//!
//! `tonk` is defined synchronously when the bootstrap runs, so author
//! top-level `tonk.query()` keeps working; each method `await`s `ready`
//! internally before posting.
//!
//! The parent is a pure **port relay**. One page-level `message`
//! listener (installed once) authenticates a `hello` by matching
//! `event.source` against the registered iframes' live `contentWindow`
//! — never by `event.origin`, which is `"null"` at an opaque origin.
//! On a match it binds the transferred port to that portal's
//! [`PortalState`] and posts `ready { context }` back. The per-port
//! dispatcher then translates each inbound envelope into the existing
//! `tonk-query` / `tonk-subscribe` / `tonk-claim` consumer events on the
//! `<tonk-portal>` element, which bubble through the routing ancestors
//! to `<tonk-host>`. Subscription frames arrive back through the
//! portal's `reset` / `error` methods (the same seam `<tonk-display>`
//! uses) and are posted to the iframe as `subscribe-event` /
//! `subscribe-error` envelopes.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use js_sys::{Object, Reflect};
use tonk_host::consumer::{self as host_consumer, Subscription as HostSubscription};
use tonk_schema::conclusion::Conclusion;
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use wasm_bindgen::closure::Closure;
use wasm_bindgen_futures::spawn_local;
use web_sys::{Element, HtmlIFrameElement, MessageEvent, MessagePort, window};

/// Per-portal bridge + iframe state. Held behind `Rc<RefCell<…>>` so
/// it is reachable from the element lifecycle, the prototype `reset`
/// delegate, and the page-level message listener.
pub(crate) struct PortalState {
    /// The single child iframe. Owned here so attribute callbacks can
    /// reload it and `disconnected_callback` can detach it.
    pub iframe: Option<HtmlIFrameElement>,
    /// Set by `disconnected_callback`; mirrors `<tonk-display>`.
    pub disposed: bool,
    /// Monotonic counter minting unique host subscription tags.
    next_tag: u64,
    /// Live subscriptions keyed by the host tag we minted. Dropping an
    /// entry cancels its host subscription.
    subs: BTreeMap<String, BridgeSub>,
    /// The port bound by the latest `hello` handshake, used to relay
    /// results back to the iframe. `None` until the iframe says hello.
    port: Option<MessagePort>,
    /// The current port's `onmessage` dispatcher, kept alive for the
    /// port's lifetime. Replaced on each handshake.
    _dispatcher: Option<Closure<dyn FnMut(MessageEvent)>>,
}

/// One live subscription: the iframe's correlation id (so frames are
/// addressed to the right author stream) plus the host subscription
/// handle (whose `Drop` cancels upstream). The port to relay frames on
/// is always the portal's current [`PortalState::port`], never a stored
/// clone — a subscription cannot outlive the port it was opened under,
/// since `reload` clears the subs before the next handshake rebinds.
struct BridgeSub {
    iframe_id: String,
    _host_sub: HostSubscription,
}

impl PortalState {
    pub(crate) fn new() -> Self {
        Self {
            iframe: None,
            disposed: false,
            next_tag: 0,
            subs: BTreeMap::new(),
            port: None,
            _dispatcher: None,
        }
    }

    /// Cancel and forget every live subscription. Dropping each
    /// `BridgeSub` cancels its host subscription, so a reload or
    /// teardown never leaves a dangling SSE.
    pub(crate) fn clear_subs(&mut self) {
        self.subs.clear();
    }
}

/// The bootstrap script prepended into the iframe's `srcdoc`. It defines
/// `window.tonk` synchronously, opens a `MessageChannel`, and hands one
/// port to the parent via `parent.postMessage(hello, "*", [port2])`.
/// Posting to `"*"` is unavoidable from a null origin; the parent
/// authenticates by `event.source`, not `event.origin`.
const BOOTSTRAP_JS: &str = r#"(function(){
  var nextId=0, pending=new Map(), streams=new Map();
  var resolveReady; var ready=new Promise(function(r){resolveReady=r;});
  var ch=new MessageChannel(), port=ch.port1;
  function mint(){return "r"+(++nextId);}
  function call(type,extra){
    return ready.then(function(){
      return new Promise(function(resolve,reject){
        var id=mint(); pending.set(id,{resolve:resolve,reject:reject});
        port.postMessage(Object.assign({v:1,type:type,id:id},extra));
      });
    });
  }
  var tonk={
    context:{this:"",model:""},
    ready:ready,
    query:function(body){return call("query",{body:body});},
    transact:function(request){return call("transact",{request:request});},
    subscribe:function(body){
      var id=mint();
      return new ReadableStream({
        start:function(controller){
          streams.set(id,controller);
          ready.then(function(){port.postMessage({v:1,type:"subscribe",id:id,body:body});},
                     function(err){streams.delete(id);controller.error(err);});
        },
        cancel:function(){
          streams.delete(id);
          port.postMessage({v:1,type:"unsubscribe",id:id});
        }
      });
    }
  };
  port.onmessage=function(event){
    var env=event.data; if(!env) return;
    switch(env.type){
      case "ready": tonk.context=env.context; resolveReady(); return;
      case "query-result": case "transact-result": {
        var h=pending.get(env.id); if(!h) return; pending.delete(env.id);
        h.resolve("rows" in env ? env.rows : env.receipt); return;
      }
      case "query-error": case "transact-error": {
        var h=pending.get(env.id); if(!h) return; pending.delete(env.id);
        h.reject(new Error(env.error)); return;
      }
      case "subscribe-event": {
        var c=streams.get(env.id); if(!c) return;
        try{c.enqueue(env.rows);}catch(e){streams.delete(env.id);} return;
      }
      case "subscribe-error": {
        var c=streams.get(env.id); if(!c) return; streams.delete(env.id);
        c.error(new Error(env.error)); return;
      }
    }
  };
  window.tonk=tonk;
  parent.postMessage({v:1,type:"hello"},"*",[ch.port2]);
})();"#;

/// Runtime-injection bootstrap, appended after [`BOOTSTRAP_JS`] when the
/// portal is in `runtime` mode. It receives the element runtime from the
/// parent (over `window` `postMessage`, NOT the data port) and brings it up
/// inside the sealed guest: inject CSS, mint blob URLs for the glue +
/// snippet modules, rewrite the glue's relative snippet imports to those
/// blobs, import the glue, instantiate the wasm from bytes (no fetch), and
/// call `start()` to register the custom elements. The `content` markup
/// (e.g. `<tonk-display>`) is already in the document and upgrades the
/// moment the elements are defined.
///
/// The guest fetches NOTHING — the parent (trusted, networked) hands over
/// every byte. `runtime-ready` tells the parent to send.
const RUNTIME_BOOTSTRAP_JS: &str = r#"(function(){
  window.addEventListener("message", async function(e){
    var d=e.data; if(!d||d.__tonkRuntime!=="inject") return;
    try {
      // Web Awesome theme classes on the root (mirrors index.html), so the
      // injected WA CSS + palette resolve their custom properties.
      document.documentElement.classList.add("wa-theme-default","wa-palette-shoelace");
      try {
        var dark=window.matchMedia&&window.matchMedia("(prefers-color-scheme: dark)").matches;
        document.documentElement.classList.toggle("wa-dark",!!dark);
        document.documentElement.classList.toggle("wa-light",!dark);
      } catch(_) {}
      if (d.css) {
        var style=document.createElement("style");
        style.textContent=d.css;
        document.head.appendChild(style);
      }
      // Web Awesome component bundle: a self-contained ESM (no dynamic or
      // relative imports), imported from a guest-minted blob so the <wa-*>
      // elements upgrade with no network.
      if (d.wa) {
        var waUrl=URL.createObjectURL(new Blob([d.wa],{type:"text/javascript"}));
        await import(waUrl);
      }
      // Rewrite each snippet import statement to a guest-minted blob URL.
      var glue=d.glue;
      for (var i=0;i<d.snippets.length;i++){
        var s=d.snippets[i];
        var url=URL.createObjectURL(new Blob([s.src],{type:"text/javascript"}));
        glue=glue.replace(s.stmt, s.stmt.replace(/from\s*['"][^'"]*['"]/, 'from "'+url+'"'));
      }
      var glueUrl=URL.createObjectURL(new Blob([glue],{type:"text/javascript"}));
      var mod=await import(glueUrl);
      await mod.default({ module_or_path: d.wasm });
      mod.start();
    } catch(err) {
      parent.postMessage({__tonkRuntime:"error",error:String(err)+(err&&err.stack?"\n"+err.stack:"")},"*");
    }
  });
  parent.postMessage({__tonkRuntime:"runtime-ready"},"*");
})();"#;

/// Prepend the bootstrap script that wires `window.tonk` to this
/// portal's bridge over a `MessagePort`.
pub(crate) fn bootstrap_srcdoc(content: &str) -> String {
    format!("<script>{BOOTSTRAP_JS}</script>{content}")
}

/// Like [`bootstrap_srcdoc`], plus the runtime-injection bootstrap: the
/// guest will ask the parent (`runtime-ready`) for the element runtime and
/// bring it up before `content`'s custom elements upgrade.
pub(crate) fn bootstrap_srcdoc_with_runtime(content: &str) -> String {
    format!("<script>{BOOTSTRAP_JS}</script><script>{RUNTIME_BOOTSTRAP_JS}</script>{content}")
}

/// Fetch the element runtime + app CSS (the parent is trusted + networked)
/// and post an `inject` envelope to the sealed `iframe`'s window. Called
/// when the guest signals `runtime-ready`. The guest fetches nothing; every
/// byte crosses here.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) fn inject_runtime(iframe: &HtmlIFrameElement) {
    use wasm_bindgen::JsValue;

    let Some(content_window) = iframe.content_window() else {
        return;
    };
    spawn_local(async move {
        let payload = match build_inject_payload().await {
            Ok(p) => p,
            Err(e) => {
                tonk_common::log!("portal runtime: failed to assemble payload: {e}");
                return;
            }
        };
        // Post to the iframe window (not the data port): runtime setup is a
        // one-time window-channel handoff, distinct from the tonk data port.
        let _ = content_window.post_message(&payload, "*");
    });
}

/// Build the `inject` envelope: `{ __tonkRuntime:"inject", glue, snippets:[
/// {stmt,src} ], wasm: ArrayBuffer, css }`. Fetches the served guest bundle
/// + app stylesheet.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn build_inject_payload() -> Result<JsValue, String> {
    use wasm_bindgen::JsValue;

    let glue = fetch_text("/guest/guest.js").await?;
    let wasm = fetch_array_buffer("/guest/guest_bg.wasm").await?;
    // App stylesheet — its hashed filename is discovered from the parent
    // document's own `<link rel=stylesheet href=/styles-*.css>`. The Web
    // Awesome CSS + the self-contained WA component bundle ride along so
    // `<wa-*>` elements style + upgrade inside the sealed guest with no
    // network of its own.
    let mut css = fetch_text("/guest/wa.css").await.unwrap_or_default();
    if let Some(href) = app_stylesheet_href() {
        css.push('\n');
        css.push_str(&fetch_text(&href).await.unwrap_or_default());
    }
    // Inline `@font-face url("/fonts/*.woff2")` as `data:` URLs: a
    // null-origin guest can't fetch the fonts (CORS-blocked), so the host
    // (same-origin) fetches each woff2 and base64-embeds it.
    css = inline_fonts(&css).await;
    // The bundled Web Awesome components (esbuild, no dynamic/relative
    // imports), imported by the guest before its content upgrades.
    let wa = fetch_text("/guest/wa.js").await.unwrap_or_default();

    // Find every `import … from '…/snippets/…'` statement in the glue and
    // fetch each snippet file, so the guest can rewrite them to blob URLs.
    let snippets = js_sys::Array::new();
    for (stmt, spec) in find_snippet_imports(&glue) {
        let path = format!("/guest/{}", spec.trim_start_matches("./"));
        let src = fetch_text(&path).await?;
        let entry = Object::new();
        let _ = Reflect::set(&entry, &"stmt".into(), &JsValue::from_str(&stmt));
        let _ = Reflect::set(&entry, &"src".into(), &JsValue::from_str(&src));
        snippets.push(&entry);
    }

    let payload = Object::new();
    let _ = Reflect::set(&payload, &"__tonkRuntime".into(), &"inject".into());
    let _ = Reflect::set(&payload, &"glue".into(), &JsValue::from_str(&glue));
    let _ = Reflect::set(&payload, &"snippets".into(), &snippets);
    let _ = Reflect::set(&payload, &"wasm".into(), &wasm);
    let _ = Reflect::set(&payload, &"css".into(), &JsValue::from_str(&css));
    let _ = Reflect::set(&payload, &"wa".into(), &JsValue::from_str(&wa));
    Ok(payload.into())
}

/// Replace every `url("/fonts/<name>.woff2")` in `css` with a
/// `url("data:font/woff2;base64,…")` so the sealed guest needs no font
/// fetch. Fonts whose fetch/encode fails are left as-is (degrade to a
/// fallback face).
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn inline_fonts(css: &str) -> String {
    // Collect distinct `/fonts/*.woff2` paths.
    let mut paths: Vec<String> = Vec::new();
    let mut rest = css;
    while let Some(i) = rest.find("/fonts/") {
        let tail = &rest[i..];
        if let Some(end) = tail.find(".woff2") {
            let path = tail[..end + 6].to_owned();
            if !paths.contains(&path) {
                paths.push(path);
            }
            rest = &tail[end + 6..];
        } else {
            break;
        }
    }

    let mut out = css.to_owned();
    for path in paths {
        if let Ok(buffer) = fetch_array_buffer(&path).await {
            if let Some(b64) = array_buffer_to_base64(&buffer) {
                let data_url = format!("data:font/woff2;base64,{b64}");
                // Replace both quoted forms `"<path>"` (the CSS uses
                // double quotes around the url argument).
                out = out.replace(&path, &data_url);
            }
        }
    }
    out
}

/// Base64-encode an `ArrayBuffer` via `btoa` over a binary string. Returns
/// `None` on any JS error.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn array_buffer_to_base64(buffer: &JsValue) -> Option<String> {
    let bytes = js_sys::Uint8Array::new(buffer);
    let len = bytes.length() as usize;
    // Build a binary string (each char = one byte) for `btoa`.
    let mut binary = String::with_capacity(len);
    let vec = bytes.to_vec();
    for b in vec {
        binary.push(b as char);
    }
    window()?.btoa(&binary).ok()
}

/// Parse `import … from '<spec>'` statements whose spec contains
/// `/snippets/`, returning `(full statement, spec)` pairs.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn find_snippet_imports(glue: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for line in glue.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with("import") || !trimmed.contains("/snippets/") {
            continue;
        }
        // spec is the quoted string after `from`
        if let Some(from_idx) = trimmed.find(" from ") {
            let after = &trimmed[from_idx + 6..];
            let quote = after.chars().next();
            if let Some(q) = quote {
                if let Some(end) = after[1..].find(q) {
                    let spec = &after[1..1 + end];
                    // statement without a trailing `;`-only tail variance:
                    // keep the trimmed line up to and including the close quote
                    let stmt_end = from_idx + 6 + 1 + end + 1;
                    let stmt = trimmed[..stmt_end].to_owned();
                    out.push((stmt, spec.to_owned()));
                }
            }
        }
    }
    out
}

/// The parent document's app stylesheet href (the hashed `/styles-*.css`),
/// read off its own `<link rel="stylesheet">`.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn app_stylesheet_href() -> Option<String> {
    let document = window()?.document()?;
    let links = document.query_selector_all("link[rel=stylesheet]").ok()?;
    for i in 0..links.length() {
        let node = links.item(i)?;
        let el: Element = node.dyn_into().ok()?;
        if let Some(href) = el.get_attribute("href") {
            if href.contains("/styles-") || href.ends_with("styles.css") {
                return Some(href);
            }
        }
    }
    None
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn fetch_text(url: &str) -> Result<String, String> {
    let resp = fetch(url).await?;
    let text = wasm_bindgen_futures::JsFuture::from(
        resp.text().map_err(|e| format!("text(): {e:?}"))?,
    )
    .await
    .map_err(|e| format!("await text: {e:?}"))?;
    text.as_string().ok_or_else(|| "text not a string".into())
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn fetch_array_buffer(url: &str) -> Result<JsValue, String> {
    let resp = fetch(url).await?;
    wasm_bindgen_futures::JsFuture::from(
        resp.array_buffer()
            .map_err(|e| format!("array_buffer(): {e:?}"))?,
    )
    .await
    .map_err(|e| format!("await array_buffer: {e:?}"))
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn fetch(url: &str) -> Result<web_sys::Response, String> {
    let win = window().ok_or("no window")?;
    let resp_value = wasm_bindgen_futures::JsFuture::from(win.fetch_with_str(url))
        .await
        .map_err(|e| format!("fetch {url}: {e:?}"))?;
    resp_value
        .dyn_into::<web_sys::Response>()
        .map_err(|_| format!("fetch {url}: not a Response"))
}

// --- Page-level `hello` listener + registry -----------------------

struct PortalEntry {
    iframe: HtmlIFrameElement,
    host: Element,
    state: Rc<RefCell<PortalState>>,
}

thread_local! {
    static REGISTRY: Rc<RefCell<Vec<PortalEntry>>> = Rc::new(RefCell::new(Vec::new()));
    static LISTENER_INSTALLED: RefCell<bool> = const { RefCell::new(false) };
}

/// Install the single page-level `message` listener that completes the
/// handshake for every portal. Idempotent.
pub(crate) fn install_message_listener() {
    let already = LISTENER_INSTALLED.with(|c| {
        let was = *c.borrow();
        *c.borrow_mut() = true;
        was
    });
    if already {
        return;
    }
    let Some(win) = window() else {
        return;
    };
    let registry = REGISTRY.with(|r| r.clone());
    let listener: Closure<dyn FnMut(MessageEvent)> =
        Closure::wrap(Box::new(move |event: MessageEvent| {
            let data = event.data();

            // Runtime-injection handshake: the guest's runtime bootstrap
            // asks for the element runtime; match its source iframe and
            // fetch+post the bundle. Distinct from the `hello`/data-port
            // handshake below.
            let runtime_kind = get_str(&data, "__tonkRuntime");
            if let Some(kind) = runtime_kind.as_deref() {
                let source = Reflect::get(&event, &"source".into()).unwrap_or(JsValue::NULL);
                match kind {
                    "runtime-ready" => {
                        let matched = registry.borrow().iter().find_map(|entry| {
                            let cw: JsValue = entry.iframe.content_window()?.into();
                            (cw == source).then(|| entry.iframe.clone())
                        });
                        #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
                        if let Some(iframe) = matched {
                            inject_runtime(&iframe);
                        }
                    }
                    "error" => {
                        tonk_common::log!(
                            "portal guest runtime error: {}",
                            get_str(&data, "error").unwrap_or_default()
                        );
                    }
                    _ => {}
                }
                return;
            }

            if get_str(&data, "type").as_deref() != Some("hello") {
                return;
            }
            // Authenticate by source identity: the message must come
            // from one of our iframes' live `contentWindow`.
            let source = Reflect::get(&event, &"source".into()).unwrap_or(JsValue::NULL);
            let port = read_first_port(&event);
            let Some(port) = port else {
                return;
            };
            let matched = registry.borrow().iter().find_map(|entry| {
                let cw: JsValue = entry.iframe.content_window()?.into();
                (cw == source).then(|| (entry.host.clone(), entry.state.clone()))
            });
            if let Some((host, state)) = matched {
                bind_port(&host, &state, port);
            }
        }) as Box<dyn FnMut(MessageEvent)>);
    let _ = win.add_event_listener_with_callback("message", listener.as_ref().unchecked_ref());
    // Lives for the page's lifetime — there is exactly one.
    listener.forget();
}

/// Register `(iframe, host, state)` so the `hello` listener can resolve
/// the portal from the iframe's live `contentWindow`.
pub(crate) fn register_portal(
    iframe: &HtmlIFrameElement,
    host: &Element,
    state: &Rc<RefCell<PortalState>>,
) {
    REGISTRY.with(|r| {
        r.borrow_mut().push(PortalEntry {
            iframe: iframe.clone(),
            host: host.clone(),
            state: state.clone(),
        })
    });
}

/// Drop the registry entry for `iframe` on teardown.
pub(crate) fn unregister_portal(iframe: &HtmlIFrameElement) {
    REGISTRY.with(|r| {
        r.borrow_mut()
            .retain(|e| !e.iframe.is_same_node(Some(iframe.as_ref())))
    });
}

/// Bind a freshly handshaked `port` to `host`/`state`: install the
/// envelope dispatcher, stash the port, and post `ready { context }`.
/// Called from the `hello` listener (and directly from tests, which
/// supply a `MessageChannel` port in place of a real iframe handshake).
pub(crate) fn bind_port(host: &Element, state: &Rc<RefCell<PortalState>>, port: MessagePort) {
    let dispatcher = make_dispatcher(host.clone(), state.clone(), port.clone());
    // Setting onmessage auto-starts the port; no port.start() needed.
    port.set_onmessage(Some(dispatcher.as_ref().unchecked_ref()));

    {
        let mut s = state.borrow_mut();
        s.port = Some(port.clone());
        s._dispatcher = Some(dispatcher);
    }

    let ready = Object::new();
    set_v1(&ready, "ready");
    let _ = Reflect::set(&ready, &"context".into(), &build_context(host));
    let _ = port.post_message(&ready);
}

// --- Envelope dispatch (parent side) ------------------------------

fn make_dispatcher(
    host: Element,
    state: Rc<RefCell<PortalState>>,
    port: MessagePort,
) -> Closure<dyn FnMut(MessageEvent)> {
    Closure::wrap(Box::new(move |event: MessageEvent| {
        let data = event.data();
        let Some(kind) = get_str(&data, "type") else {
            return;
        };
        match kind.as_str() {
            "query" => handle_query(&host, &port, &data),
            "transact" => handle_transact(&host, &port, &data),
            "subscribe" => handle_subscribe(&host, &state, &port, &data),
            "unsubscribe" => handle_unsubscribe(&state, &data),
            _ => {}
        }
    }) as Box<dyn FnMut(MessageEvent)>)
}

fn handle_query(host: &Element, port: &MessagePort, data: &JsValue) {
    let Some(id) = get_str(data, "id") else {
        return;
    };
    let body = match query_body(host, &get_body(data)) {
        Ok(b) => b,
        Err(msg) => return post_error(port, "query-error", &id, &msg),
    };
    let host = host.clone();
    let port = port.clone();
    spawn_local(async move {
        match host_consumer::query(&host, &body).await {
            Ok(rows) => post_result(&port, "query-result", &id, "rows", &rows),
            Err(e) => post_error(&port, "query-error", &id, &e.message),
        }
    });
}

fn handle_transact(host: &Element, port: &MessagePort, data: &JsValue) {
    let Some(id) = get_str(data, "id") else {
        return;
    };
    let request = Reflect::get(data, &"request".into()).unwrap_or(JsValue::UNDEFINED);
    let host = host.clone();
    let port = port.clone();
    spawn_local(async move {
        match host_consumer::claim(&host, &request).await {
            Ok(receipt) => post_result(&port, "transact-result", &id, "receipt", &receipt),
            Err(e) => post_error(&port, "transact-error", &id, &e.message),
        }
    });
}

fn handle_subscribe(
    host: &Element,
    state: &Rc<RefCell<PortalState>>,
    port: &MessagePort,
    data: &JsValue,
) {
    let Some(id) = get_str(data, "id") else {
        return;
    };
    let body = match query_body(host, &get_body(data)) {
        Ok(b) => b,
        Err(msg) => return post_error(port, "subscribe-error", &id, &msg),
    };

    let tag = {
        let mut s = state.borrow_mut();
        s.next_tag = s.next_tag.wrapping_add(1);
        format!("portal-sub-{}", s.next_tag)
    };
    let tag_js = JsValue::from_str(&tag);
    match host_consumer::subscribe(host, &body, Some(&tag_js)) {
        Ok(host_sub) => {
            state.borrow_mut().subs.insert(
                tag,
                BridgeSub {
                    iframe_id: id,
                    _host_sub: host_sub,
                },
            );
        }
        // No host ancestor / dispatch failure: surface to the author's
        // stream; nothing is tracked.
        Err(e) => post_error(port, "subscribe-error", &id, &e.message),
    }
}

fn handle_unsubscribe(state: &Rc<RefCell<PortalState>>, data: &JsValue) {
    let Some(id) = get_str(data, "id") else {
        return;
    };
    let mut s = state.borrow_mut();
    let tag = s
        .subs
        .iter()
        .find(|(_, sub)| sub.iframe_id == id)
        .map(|(tag, _)| tag.clone());
    if let Some(tag) = tag {
        // Dropping the `BridgeSub` cancels its host subscription.
        s.subs.remove(&tag);
    }
}

// --- Query-body construction --------------------------------------

/// Build the query body for a bridge call: no argument streams the
/// scoped entity; an explicit body is forwarded verbatim.
fn query_body(host: &Element, arg: &JsValue) -> Result<JsValue, String> {
    if arg.is_undefined() || arg.is_null() {
        no_arg_entity_query(host)
    } else {
        Ok(arg.clone())
    }
}

fn no_arg_entity_query(host: &Element) -> Result<JsValue, String> {
    let entity = host
        .get_attribute("entity")
        .filter(|s| !s.is_empty())
        .ok_or("tonk.subscribe()/query() with no argument requires a scoped `entity`")?;
    let descriptor = read_descriptor(host)
        .ok_or("tonk.subscribe()/query() with no argument requires a model descriptor")?;
    let query = crate::query::entity_query(&descriptor, &entity)
        .map_err(|e| format!("entity query: {e}"))?;
    serde_wasm_bindgen::to_value(&query).map_err(|e| format!("query body: {e}"))
}

fn read_descriptor(host: &Element) -> Option<String> {
    Reflect::get(host, &"descriptor".into())
        .ok()
        .and_then(|v| v.as_string())
}

/// Build the `context` object (`{ this, model }`) the iframe receives in
/// its `ready` envelope, from the host's current attributes.
fn build_context(host: &Element) -> Object {
    let context = Object::new();
    let this = host.get_attribute("entity").unwrap_or_default();
    let model = host.get_attribute("model").unwrap_or_default();
    let _ = Reflect::set(&context, &"this".into(), &JsValue::from_str(&this));
    let _ = Reflect::set(&context, &"model".into(), &JsValue::from_str(&model));
    context
}

// --- Frame routing (called by the element's reset / error shims) --

/// `reset(conclusions, { tag })` — a subscription frame from the host.
/// The host serializes conclusions with `serde-wasm-bindgen`, which
/// renders maps as JS `Map`s (and integers as `BigInt`). Round-trip
/// through JSON so the wire shape is identical to what `query()` yields
/// (the host `JSON.parse`s one-shot results) — numbers, not `BigInt`s,
/// plain objects, not `Map`s — which `postMessage`'s structured clone
/// would not otherwise guarantee. The plain rows are posted to the
/// iframe as a `subscribe-event` addressed to the author's stream.
pub(crate) fn route_reset(state: &Rc<RefCell<PortalState>>, payload: JsValue, opts: JsValue) {
    let Some(tag) = read_tag(&opts) else {
        return;
    };
    let conclusions: Vec<Conclusion> = match serde_wasm_bindgen::from_value(payload) {
        Ok(v) => v,
        Err(_) => return,
    };
    let plain = match serde_json::to_string(&conclusions) {
        Ok(json) => js_sys::JSON::parse(&json).unwrap_or(JsValue::NULL),
        Err(_) => return,
    };
    let Some((port, iframe_id)) = lookup_sub(state, &tag) else {
        return;
    };
    let env = Object::new();
    set_v1(&env, "subscribe-event");
    let _ = Reflect::set(&env, &"id".into(), &JsValue::from_str(&iframe_id));
    let _ = Reflect::set(&env, &"rows".into(), &plain);
    let _ = port.post_message(&env);
}

/// `error(detail, { tag })` — a transport error on a subscription.
/// Posts a `subscribe-error` so the matching author stream errors.
pub(crate) fn route_error(state: &Rc<RefCell<PortalState>>, payload: JsValue, opts: JsValue) {
    let Some(tag) = read_tag(&opts) else {
        return;
    };
    let Some((port, iframe_id)) = lookup_sub(state, &tag) else {
        return;
    };
    post_error(
        &port,
        "subscribe-error",
        &iframe_id,
        &error_message(&payload),
    );
}

/// Resolve `(current port, iframe correlation id)` for a live tag. The
/// port is read from `state` at call time, so frames always go to the
/// portal's current handshake — never a port captured when the
/// subscription opened.
fn lookup_sub(state: &Rc<RefCell<PortalState>>, tag: &str) -> Option<(MessagePort, String)> {
    let s = state.borrow();
    let iframe_id = s.subs.get(tag)?.iframe_id.clone();
    let port = s.port.clone()?;
    Some((port, iframe_id))
}

// --- Small helpers -------------------------------------------------

fn read_tag(opts: &JsValue) -> Option<String> {
    if !opts.is_object() {
        return None;
    }
    get_str(opts, "tag")
}

fn get_str(obj: &JsValue, key: &str) -> Option<String> {
    Reflect::get(obj, &key.into())
        .ok()
        .and_then(|v| v.as_string())
}

fn get_body(data: &JsValue) -> JsValue {
    Reflect::get(data, &"body".into()).unwrap_or(JsValue::UNDEFINED)
}

fn read_first_port(event: &MessageEvent) -> Option<MessagePort> {
    let ports = Reflect::get(event, &"ports".into()).ok()?;
    let ports: js_sys::Array = ports.dyn_into().ok()?;
    ports.get(0).dyn_into::<MessagePort>().ok()
}

/// Read a human message out of an error payload: a string verbatim,
/// otherwise its `message` field, otherwise its debug form.
fn error_message(payload: &JsValue) -> String {
    if let Some(s) = payload.as_string() {
        return s;
    }
    get_str(payload, "message").unwrap_or_else(|| format!("{payload:?}"))
}

fn set_v1(env: &Object, ty: &str) {
    let _ = Reflect::set(env, &"v".into(), &JsValue::from_f64(1.0));
    let _ = Reflect::set(env, &"type".into(), &JsValue::from_str(ty));
}

fn post_result(port: &MessagePort, ty: &str, id: &str, field: &str, value: &JsValue) {
    let env = Object::new();
    set_v1(&env, ty);
    let _ = Reflect::set(&env, &"id".into(), &JsValue::from_str(id));
    let _ = Reflect::set(&env, &field.into(), value);
    let _ = port.post_message(&env);
}

fn post_error(port: &MessagePort, ty: &str, id: &str, error: &str) {
    let env = Object::new();
    set_v1(&env, ty);
    let _ = Reflect::set(&env, &"id".into(), &JsValue::from_str(id));
    let _ = Reflect::set(&env, &"error".into(), &JsValue::from_str(error));
    let _ = port.post_message(&env);
}

#[cfg(test)]
mod tests {
    use super::*;
    use js_sys::{Array, Function, Promise};
    use wasm_bindgen_futures::JsFuture;
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    use web_sys::{CustomEvent, Document, MessageChannel};

    wasm_bindgen_test_configure!(run_in_browser);

    fn document() -> Document {
        window().expect("window").document().expect("document")
    }

    /// Sleep `ms` milliseconds, yielding to the event loop.
    async fn sleep(ms: i32) {
        let promise = Promise::new(&mut |resolve, _reject| {
            let _ = window()
                .expect("window")
                .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms);
        });
        let _ = JsFuture::from(promise).await;
    }

    const DESCRIPTOR: &str = r#"{"with":{
        "count": { "the": "counter/count", "as": "UnsignedInteger", "cardinality": "one" }
    }}"#;

    // --- FakeHost: a stand-in `<tonk-host>` ancestor ----------------

    /// A minimal stand-in for `<tonk-host>`: a container that answers the
    /// consumer events the relay dispatches with canned data, captures
    /// the live subscription's consumer + tag, and records cancellation.
    struct FakeHost {
        container: Element,
        state: Rc<RefCell<FakeState>>,
        _listeners: Vec<Closure<dyn FnMut(CustomEvent)>>,
    }

    #[derive(Default)]
    struct FakeState {
        query_result: Option<JsValue>,
        claim_result: Option<JsValue>,
        last_query_body: Option<JsValue>,
        last_claim_body: Option<JsValue>,
        sub_consumer: Option<Element>,
        sub_tag: Option<JsValue>,
        last_subscribe_body: Option<JsValue>,
        cancelled: bool,
    }

    impl FakeHost {
        fn install() -> FakeHost {
            let container = document().create_element("div").expect("div");
            document()
                .body()
                .expect("body")
                .append_child(&container)
                .expect("attach container");
            let state = Rc::new(RefCell::new(FakeState::default()));
            let mut listeners = Vec::new();

            {
                let state = state.clone();
                let cb: Closure<dyn FnMut(CustomEvent)> =
                    Closure::wrap(Box::new(move |ev: CustomEvent| {
                        ev.stop_propagation();
                        ev.prevent_default();
                        let detail: Object = ev.detail().dyn_into().unwrap();
                        let query = Reflect::get(&detail, &"query".into()).unwrap();
                        state.borrow_mut().last_query_body = Some(query);
                        let result = state
                            .borrow()
                            .query_result
                            .clone()
                            .unwrap_or(JsValue::from(Array::new()));
                        let _ = Reflect::set(&detail, &"result".into(), &Promise::resolve(&result));
                    }) as Box<dyn FnMut(CustomEvent)>);
                let _ = container
                    .add_event_listener_with_callback("tonk-query", cb.as_ref().unchecked_ref());
                listeners.push(cb);
            }
            {
                let state = state.clone();
                let cb: Closure<dyn FnMut(CustomEvent)> =
                    Closure::wrap(Box::new(move |ev: CustomEvent| {
                        ev.stop_propagation();
                        ev.prevent_default();
                        let detail: Object = ev.detail().dyn_into().unwrap();
                        let request = Reflect::get(&detail, &"request".into()).unwrap();
                        state.borrow_mut().last_claim_body = Some(request);
                        let result = state
                            .borrow()
                            .claim_result
                            .clone()
                            .unwrap_or(JsValue::from_str("ok"));
                        let _ = Reflect::set(&detail, &"result".into(), &Promise::resolve(&result));
                    }) as Box<dyn FnMut(CustomEvent)>);
                let _ = container
                    .add_event_listener_with_callback("tonk-claim", cb.as_ref().unchecked_ref());
                listeners.push(cb);
            }
            {
                let state = state.clone();
                let cb: Closure<dyn FnMut(CustomEvent)> =
                    Closure::wrap(Box::new(move |ev: CustomEvent| {
                        ev.stop_propagation();
                        ev.prevent_default();
                        let detail: Object = ev.detail().dyn_into().unwrap();
                        let query = Reflect::get(&detail, &"query".into()).unwrap();
                        let tag = Reflect::get(&detail, &"tag".into()).ok();
                        let consumer: Element = ev.target().unwrap().dyn_into().unwrap();
                        {
                            let mut s = state.borrow_mut();
                            s.last_subscribe_body = Some(query);
                            s.sub_consumer = Some(consumer);
                            s.sub_tag = tag;
                        }
                        let sub = Object::new();
                        let state_for_cancel = state.clone();
                        let cancel: Closure<dyn FnMut()> = Closure::wrap(Box::new(move || {
                            state_for_cancel.borrow_mut().cancelled = true;
                        })
                            as Box<dyn FnMut()>);
                        let cancel_fn: Function = cancel.into_js_value().unchecked_into();
                        let _ = Reflect::set(&sub, &"cancel".into(), &cancel_fn);
                        let _ = Reflect::set(&detail, &"subscription".into(), &sub);
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

        fn set_query_result(&self, value: JsValue) {
            self.state.borrow_mut().query_result = Some(value);
        }
        fn set_claim_result(&self, value: JsValue) {
            self.state.borrow_mut().claim_result = Some(value);
        }
        fn last_query_body(&self) -> Option<JsValue> {
            self.state.borrow().last_query_body.clone()
        }
        fn last_claim_body(&self) -> Option<JsValue> {
            self.state.borrow().last_claim_body.clone()
        }
        fn sub_tag(&self) -> Option<JsValue> {
            self.state.borrow().sub_tag.clone()
        }
        fn cancelled(&self) -> bool {
            self.state.borrow().cancelled
        }

        /// Push a subscription frame to the captured consumer, mirroring
        /// how the real host calls `consumer.reset(conclusions, { tag })`.
        fn push_frame(&self, conclusions: &JsValue) {
            let (consumer, tag) = {
                let s = self.state.borrow();
                (s.sub_consumer.clone(), s.sub_tag.clone())
            };
            let Some(consumer) = consumer else { return };
            let opts = Object::new();
            if let Some(t) = tag {
                let _ = Reflect::set(&opts, &"tag".into(), &t);
            }
            let reset = Reflect::get(&consumer, &"reset".into()).unwrap();
            let reset: Function = reset.dyn_into().expect("reset method");
            let _ = reset.call2(&consumer, conclusions, &opts);
        }
    }

    /// A consumer element that dispatches the bridge's events: a `<div>`
    /// under the fake host carrying the scoped `entity` / `model` and the
    /// model `descriptor` the relay reads for no-argument calls.
    fn relay_consumer(
        host: &FakeHost,
        entity: Option<&str>,
        model: Option<&str>,
        descriptor: Option<&str>,
    ) -> Element {
        let consumer = document().create_element("div").expect("div");
        if let Some(e) = entity {
            consumer.set_attribute("entity", e).expect("entity");
        }
        if let Some(m) = model {
            consumer.set_attribute("model", m).expect("model");
        }
        if let Some(d) = descriptor {
            let _ = Reflect::set(
                consumer.as_ref(),
                &"descriptor".into(),
                &JsValue::from_str(d),
            );
        }
        host.container.append_child(&consumer).expect("attach");
        consumer
    }

    // --- Port plumbing for relay tests ------------------------------

    /// Collects messages arriving on a port and lets a test await the
    /// first one of a given `type`.
    struct PortListener {
        messages: Rc<RefCell<Vec<JsValue>>>,
        _cb: Closure<dyn FnMut(MessageEvent)>,
    }

    impl PortListener {
        fn attach(port: &MessagePort) -> Self {
            let messages = Rc::new(RefCell::new(Vec::new()));
            let sink = messages.clone();
            let cb: Closure<dyn FnMut(MessageEvent)> =
                Closure::wrap(Box::new(move |e: MessageEvent| {
                    sink.borrow_mut().push(e.data());
                }) as Box<dyn FnMut(MessageEvent)>);
            // Setting onmessage auto-starts the port.
            port.set_onmessage(Some(cb.as_ref().unchecked_ref()));
            PortListener { messages, _cb: cb }
        }

        async fn wait_for(&self, ty: &str) -> JsValue {
            for _ in 0..200 {
                let found = self
                    .messages
                    .borrow()
                    .iter()
                    .find(|d| get_str(d, "type").as_deref() == Some(ty))
                    .cloned();
                if let Some(found) = found {
                    return found;
                }
                sleep(5).await;
            }
            JsValue::UNDEFINED
        }
    }

    /// Build a `{ v, type, id, ...extra }` envelope to post from the
    /// test side of the channel.
    fn envelope(ty: &str, id: &str) -> Object {
        let env = Object::new();
        set_v1(&env, ty);
        let _ = Reflect::set(&env, &"id".into(), &JsValue::from_str(id));
        env
    }

    /// Wire a fresh `MessageChannel`: bind one end to the portal relay
    /// (as a `hello` would) and return the other end's listener + port
    /// for the test to drive.
    fn bind(consumer: &Element, state: &Rc<RefCell<PortalState>>) -> (PortListener, MessagePort) {
        let channel = MessageChannel::new().expect("MessageChannel");
        let test_port = channel.port1();
        let portal_port = channel.port2();
        let listener = PortListener::attach(&test_port);
        bind_port(consumer, state, portal_port);
        (listener, test_port)
    }

    /// A host-shaped subscription frame: `Vec<Conclusion>` serialized
    /// with `serde-wasm-bindgen` (which renders maps as JS `Map`s), as
    /// `<tonk-host>` delivers them.
    fn host_frame(this: &str, count: i128) -> JsValue {
        use ipld_core::ipld::Ipld;
        let mut fields: BTreeMap<String, Ipld> = BTreeMap::new();
        fields.insert("count".to_owned(), Ipld::Integer(count));
        let conclusions = vec![Conclusion {
            this: this.to_owned(),
            fields,
        }];
        serde_wasm_bindgen::to_value(&conclusions).expect("serialize frame")
    }

    fn get_num(obj: &JsValue, key: &str) -> Option<f64> {
        Reflect::get(obj, &key.into()).ok().and_then(|v| v.as_f64())
    }

    // --- Relay tests (seam 1) ---------------------------------------

    #[dialog_common::test]
    async fn it_posts_ready_with_context_on_bind() {
        let host = FakeHost::install();
        let consumer = relay_consumer(&host, Some("id:demo-counter"), Some("counter"), None);
        let state = Rc::new(RefCell::new(PortalState::new()));
        let (listener, _port) = bind(&consumer, &state);

        let ready = listener.wait_for("ready").await;
        let context = Reflect::get(&ready, &"context".into()).expect("context");
        assert_eq!(
            get_str(&context, "this").as_deref(),
            Some("id:demo-counter")
        );
        assert_eq!(get_str(&context, "model").as_deref(), Some("counter"));
    }

    #[dialog_common::test]
    async fn it_relays_a_query_envelope_and_returns_rows() {
        let host = FakeHost::install();
        let canned = Array::new();
        canned.push(&JsValue::from_str("row"));
        host.set_query_result(canned.into());
        let consumer = relay_consumer(&host, None, None, None);
        let state = Rc::new(RefCell::new(PortalState::new()));
        let (listener, port) = bind(&consumer, &state);

        // Explicit body is forwarded verbatim.
        let env = envelope("query", "r1");
        let body = Object::new();
        let _ = Reflect::set(&body, &"marker".into(), &JsValue::from_str("explicit"));
        let _ = Reflect::set(&env, &"body".into(), &body);
        port.post_message(&env).expect("post query");

        let result = listener.wait_for("query-result").await;
        assert_eq!(get_str(&result, "id").as_deref(), Some("r1"));
        let rows: Array = Reflect::get(&result, &"rows".into())
            .expect("rows")
            .dyn_into()
            .expect("array");
        assert_eq!(rows.get(0).as_string().as_deref(), Some("row"));

        let dispatched = host.last_query_body().expect("query dispatched");
        assert_eq!(
            get_str(&dispatched, "marker").as_deref(),
            Some("explicit"),
            "explicit body forwarded verbatim",
        );
    }

    #[dialog_common::test]
    async fn it_builds_the_no_arg_query_from_descriptor_and_entity() {
        let host = FakeHost::install();
        let consumer = relay_consumer(
            &host,
            Some("id:demo-counter"),
            Some("counter"),
            Some(DESCRIPTOR),
        );
        let state = Rc::new(RefCell::new(PortalState::new()));
        let (listener, port) = bind(&consumer, &state);

        // No `body` field — the relay must build the scoped-entity query.
        port.post_message(&envelope("query", "r1")).expect("post");
        let _ = listener.wait_for("query-result").await;

        let body = host.last_query_body().expect("query dispatched");
        let terms = Reflect::get(&body, &"terms".into()).expect("terms");
        // `serde-wasm-bindgen` renders the body as nested `Map`s.
        let this = {
            let map: js_sys::Map = terms.dyn_into().expect("terms is a Map");
            map.get(&"this".into())
        };
        assert_eq!(this.as_string().as_deref(), Some("id:demo-counter"));
    }

    #[dialog_common::test]
    async fn it_relays_a_transact_envelope_to_claim() {
        let host = FakeHost::install();
        host.set_claim_result(JsValue::from_str("receipt"));
        let consumer = relay_consumer(&host, None, None, None);
        let state = Rc::new(RefCell::new(PortalState::new()));
        let (listener, port) = bind(&consumer, &state);

        let env = envelope("transact", "r1");
        let request = Object::new();
        let _ = Reflect::set(&request, &"assert".into(), &JsValue::from_str("something"));
        let _ = Reflect::set(&env, &"request".into(), &request);
        port.post_message(&env).expect("post transact");

        let result = listener.wait_for("transact-result").await;
        assert_eq!(get_str(&result, "id").as_deref(), Some("r1"));
        assert_eq!(
            Reflect::get(&result, &"receipt".into())
                .ok()
                .and_then(|v| v.as_string())
                .as_deref(),
            Some("receipt"),
        );
        let body = host.last_claim_body().expect("claim dispatched");
        assert_eq!(get_str(&body, "assert").as_deref(), Some("something"));
    }

    #[dialog_common::test]
    async fn it_opens_a_host_subscription_and_posts_reset_frames() {
        let host = FakeHost::install();
        let consumer = relay_consumer(
            &host,
            Some("id:demo-counter"),
            Some("counter"),
            Some(DESCRIPTOR),
        );
        let state = Rc::new(RefCell::new(PortalState::new()));
        let (listener, port) = bind(&consumer, &state);

        port.post_message(&envelope("subscribe", "r1"))
            .expect("post subscribe");

        // Wait for the host subscription to open and capture its tag.
        let mut tag = JsValue::UNDEFINED;
        for _ in 0..200 {
            if let Some(t) = host.sub_tag() {
                tag = t;
                break;
            }
            sleep(5).await;
        }
        assert!(!tag.is_undefined(), "subscribe should reach the host");

        // A host frame for that tag must come back as a subscribe-event
        // addressed to the iframe's correlation id, dot-accessible.
        route_reset(&state, host_frame("id:demo-counter", 5), tag_opts(&tag));
        let event = listener.wait_for("subscribe-event").await;
        assert_eq!(get_str(&event, "id").as_deref(), Some("r1"));
        let rows: Array = Reflect::get(&event, &"rows".into())
            .expect("rows")
            .dyn_into()
            .expect("array");
        let me = rows.get(0);
        assert_eq!(get_str(&me, "this").as_deref(), Some("id:demo-counter"));
        let fields = Reflect::get(&me, &"fields".into()).expect("fields");
        assert_eq!(
            get_num(&fields, "count"),
            Some(5.0),
            "integer field is a plain number, not a BigInt",
        );
    }

    #[dialog_common::test]
    async fn it_errors_the_stream_on_a_reset_error_frame() {
        let host = FakeHost::install();
        let consumer = relay_consumer(
            &host,
            Some("id:demo-counter"),
            Some("counter"),
            Some(DESCRIPTOR),
        );
        let state = Rc::new(RefCell::new(PortalState::new()));
        let (listener, port) = bind(&consumer, &state);

        port.post_message(&envelope("subscribe", "r1"))
            .expect("post subscribe");
        let mut tag = JsValue::UNDEFINED;
        for _ in 0..200 {
            if let Some(t) = host.sub_tag() {
                tag = t;
                break;
            }
            sleep(5).await;
        }

        route_error(&state, JsValue::from_str("upstream gone"), tag_opts(&tag));
        let event = listener.wait_for("subscribe-error").await;
        assert_eq!(get_str(&event, "id").as_deref(), Some("r1"));
        assert_eq!(get_str(&event, "error").as_deref(), Some("upstream gone"));
    }

    #[dialog_common::test]
    async fn it_cancels_the_host_subscription_on_unsubscribe() {
        let host = FakeHost::install();
        let consumer = relay_consumer(
            &host,
            Some("id:demo-counter"),
            Some("counter"),
            Some(DESCRIPTOR),
        );
        let state = Rc::new(RefCell::new(PortalState::new()));
        let (_listener, port) = bind(&consumer, &state);

        port.post_message(&envelope("subscribe", "r1"))
            .expect("post subscribe");
        for _ in 0..200 {
            if host.sub_tag().is_some() {
                break;
            }
            sleep(5).await;
        }
        assert!(!host.cancelled(), "not cancelled before unsubscribe");

        port.post_message(&envelope("unsubscribe", "r1"))
            .expect("post unsubscribe");
        for _ in 0..200 {
            if host.cancelled() {
                break;
            }
            sleep(5).await;
        }
        assert!(
            host.cancelled(),
            "unsubscribe drops the BridgeSub, cancelling the host subscription",
        );
    }

    #[dialog_common::test]
    async fn it_returns_a_query_error_when_there_is_no_host_ancestor() {
        // A consumer attached to the body with no FakeHost ancestor:
        // `tonk-query` is not default-prevented, so the relay errors.
        let consumer = document().create_element("div").expect("div");
        document()
            .body()
            .expect("body")
            .append_child(&consumer)
            .expect("attach");
        let state = Rc::new(RefCell::new(PortalState::new()));
        let (listener, port) = bind(&consumer, &state);

        let env = envelope("query", "r1");
        let _ = Reflect::set(&env, &"body".into(), &Object::new());
        port.post_message(&env).expect("post query");

        let error = listener.wait_for("query-error").await;
        assert_eq!(get_str(&error, "id").as_deref(), Some("r1"));
        assert!(
            get_str(&error, "error").is_some(),
            "an error message should be relayed",
        );
    }

    fn tag_opts(tag: &JsValue) -> JsValue {
        let opts = Object::new();
        let _ = Reflect::set(&opts, &"tag".into(), tag);
        opts.into()
    }

    // --- End-to-end smoke tests (seam 2) ----------------------------

    /// Mount a real `<tonk-portal>` (opaque-origin iframe) under the
    /// fake host with the given attributes + descriptor property.
    fn mount_portal(
        host: &FakeHost,
        content: &str,
        entity: Option<&str>,
        model: Option<&str>,
        descriptor: Option<&str>,
    ) -> Element {
        crate::register();
        let portal = document()
            .create_element("tonk-portal")
            .expect("tonk-portal");
        portal.set_attribute("content", content).expect("content");
        if let Some(e) = entity {
            portal.set_attribute("entity", e).expect("entity");
        }
        if let Some(m) = model {
            portal.set_attribute("model", m).expect("model");
        }
        if let Some(d) = descriptor {
            let _ = Reflect::set(portal.as_ref(), &"descriptor".into(), &JsValue::from_str(d));
        }
        host.container.append_child(&portal).expect("attach portal");
        portal
    }

    /// Listen on `window` for the author iframe's `{ __test: tag, ... }`
    /// message posted back across the opaque-origin boundary.
    struct WindowProbe {
        message: Rc<RefCell<Option<JsValue>>>,
        _cb: Closure<dyn FnMut(MessageEvent)>,
    }

    impl WindowProbe {
        fn install(tag: &'static str) -> Self {
            let message = Rc::new(RefCell::new(None));
            let sink = message.clone();
            let cb: Closure<dyn FnMut(MessageEvent)> =
                Closure::wrap(Box::new(move |e: MessageEvent| {
                    let data = e.data();
                    if get_str(&data, "__test").as_deref() == Some(tag) {
                        *sink.borrow_mut() = Some(data);
                    }
                }) as Box<dyn FnMut(MessageEvent)>);
            let _ = window()
                .expect("window")
                .add_event_listener_with_callback("message", cb.as_ref().unchecked_ref());
            WindowProbe { message, _cb: cb }
        }

        async fn wait(&self) -> JsValue {
            for _ in 0..400 {
                if let Some(v) = self.message.borrow().clone() {
                    return v;
                }
                sleep(5).await;
            }
            JsValue::UNDEFINED
        }
    }

    #[dialog_common::test]
    async fn it_runs_a_real_query_across_the_opaque_origin_boundary() {
        let host = FakeHost::install();
        let canned = Array::new();
        canned.push(&JsValue::from_str("row"));
        host.set_query_result(canned.into());
        let probe = WindowProbe::install("q");

        // Author code runs at the opaque origin, calls tonk.query(), and
        // posts the result back to the parent (this test's window).
        let content = "<script>\
            tonk.query()\
              .then(function(rows){parent.postMessage({__test:'q',rows:rows},'*');})\
              .catch(function(err){parent.postMessage({__test:'q',error:String(err)},'*');});\
            </script>";
        mount_portal(
            &host,
            content,
            Some("id:demo-counter"),
            Some("counter"),
            Some(DESCRIPTOR),
        );

        let msg = probe.wait().await;
        assert!(
            !msg.is_undefined(),
            "author iframe should post a result back across the boundary",
        );
        assert!(
            Reflect::get(&msg, &"error".into())
                .ok()
                .filter(|v| !v.is_undefined())
                .is_none(),
            "query should not error; got: {:?}",
            Reflect::get(&msg, &"error".into()).ok(),
        );
        let rows: Array = Reflect::get(&msg, &"rows".into())
            .expect("rows")
            .dyn_into()
            .expect("array");
        assert_eq!(rows.get(0).as_string().as_deref(), Some("row"));
    }

    #[dialog_common::test]
    async fn it_delivers_subscription_frames_across_the_opaque_origin_boundary() {
        let host = FakeHost::install();
        let probe = WindowProbe::install("s");

        // Author subscribes, reads one frame, posts it back.
        let content = "<script>\
            var reader = tonk.subscribe().getReader();\
            reader.read().then(function(r){parent.postMessage({__test:'s',value:r.value},'*');});\
            </script>";
        mount_portal(
            &host,
            content,
            Some("id:demo-counter"),
            Some("counter"),
            Some(DESCRIPTOR),
        );

        // Wait for the host subscription to open, then push a frame.
        for _ in 0..400 {
            if host.sub_tag().is_some() {
                break;
            }
            sleep(5).await;
        }
        assert!(host.sub_tag().is_some(), "subscribe should reach the host");
        host.push_frame(&host_frame("id:demo-counter", 7));

        let msg = probe.wait().await;
        assert!(!msg.is_undefined(), "author should post a frame back");
        let rows: Array = Reflect::get(&msg, &"value".into())
            .expect("value")
            .dyn_into()
            .expect("Conclusion[]");
        let me = rows.get(0);
        assert_eq!(get_str(&me, "this").as_deref(), Some("id:demo-counter"));
        let fields = Reflect::get(&me, &"fields".into()).expect("fields");
        assert_eq!(get_num(&fields, "count"), Some(7.0));
    }

    #[dialog_common::test]
    async fn it_ignores_a_hello_from_an_unregistered_source() {
        // A registered portal whose iframe never speaks: the registry is
        // non-empty, but only its live `contentWindow` may complete a
        // handshake.
        install_message_listener();
        let host = FakeHost::install();
        let consumer = relay_consumer(&host, None, None, None);
        let iframe = document()
            .create_element("iframe")
            .expect("iframe")
            .dyn_into::<HtmlIFrameElement>()
            .expect("iframe cast");
        host.container.append_child(&iframe).expect("attach iframe");
        let state = Rc::new(RefCell::new(PortalState::new()));
        register_portal(&iframe, &consumer, &state);

        // Forge a `hello` from this window — not the iframe's
        // `contentWindow` — transferring a port. Source identity, not
        // the presence of a port, must reject it.
        let channel = MessageChannel::new().expect("MessageChannel");
        let listener = PortListener::attach(&channel.port1());
        let env = Object::new();
        set_v1(&env, "hello");
        let transfer = Array::new();
        transfer.push(&channel.port2());
        window()
            .expect("window")
            .post_message_with_transfer(&env, "*", &transfer)
            .expect("post foreign hello");

        // `wait_for` polls for ~1s; an unmatched hello yields nothing.
        let ready = listener.wait_for("ready").await;
        assert!(
            ready.is_undefined(),
            "a hello from an unregistered source must not be answered",
        );
        assert!(
            state.borrow().port.is_none(),
            "no port should bind for an unmatched source",
        );
    }

    #[dialog_common::test]
    async fn it_routes_each_portals_hello_to_its_own_context() {
        // Two portals share the single page-level listener. Each reports
        // the `this` it received in its handshake; the listener must
        // route each hello to its own portal's context, not cross-wire.
        let host = FakeHost::install();
        let probe_a = WindowProbe::install("a");
        let probe_b = WindowProbe::install("b");
        let report = |tag: &str| {
            format!(
                "<script>tonk.ready.then(function(){{\
                   parent.postMessage({{__test:'{tag}',this:tonk.context.this}},'*');}});\
                 </script>"
            )
        };
        mount_portal(
            &host,
            &report("a"),
            Some("id:alpha"),
            Some("counter"),
            Some(DESCRIPTOR),
        );
        mount_portal(
            &host,
            &report("b"),
            Some("id:beta"),
            Some("counter"),
            Some(DESCRIPTOR),
        );

        let a = probe_a.wait().await;
        let b = probe_b.wait().await;
        assert_eq!(
            get_str(&a, "this").as_deref(),
            Some("id:alpha"),
            "portal A's hello must bind A's context",
        );
        assert_eq!(
            get_str(&b, "this").as_deref(),
            Some("id:beta"),
            "portal B's hello must bind B's context",
        );
    }

    #[dialog_common::test]
    async fn it_cancels_live_subscriptions_when_content_reloads() {
        let host = FakeHost::install();
        let content = "<script>tonk.subscribe().getReader().read();</script>";
        let portal = mount_portal(
            &host,
            content,
            Some("id:demo-counter"),
            Some("counter"),
            Some(DESCRIPTOR),
        );

        // Wait for the subscription to reach the host.
        for _ in 0..400 {
            if host.sub_tag().is_some() {
                break;
            }
            sleep(5).await;
        }
        assert!(host.sub_tag().is_some(), "subscribe should reach the host");
        assert!(!host.cancelled(), "not cancelled before reload");

        // New content reloads the iframe; `reload` clears the subs
        // first, dropping the `BridgeSub` and cancelling the host
        // subscription so the discarded window leaves no dangling SSE.
        portal
            .set_attribute("content", "<p>reloaded</p>")
            .expect("set content");
        for _ in 0..400 {
            if host.cancelled() {
                break;
            }
            sleep(5).await;
        }
        assert!(
            host.cancelled(),
            "a reload cancels the live host subscription",
        );
    }
}
