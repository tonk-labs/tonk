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
//! The author-facing object:
//!
//! ```text
//! window.tonk = {
//!   context: { this, model },
//!   preview(request)   -> Promise<value>,
//!   delegate(request)  -> Promise<delegation>,
//!   navigate(href)     -> void,
//!   reload()           -> void,
//!   setTitle(text)     -> void,
//!   open(href)         -> void,
//!   analytics(event)   -> void,
//!   register(reason, cb) -> void,
//!   task(payload, cb)  -> void,
//!   fetch(path, req)   -> Promise<Response>,
//!   ready: Promise<void>,
//! }
//! ```
//!
//! `tonk` is defined synchronously when the bootstrap runs; each method
//! `await`s `ready` internally before posting. Data is not relayed
//! through this object: a guest reads and writes with plain `fetch`.
//!
//! The parent is a pure **port relay**. One page-level `message`
//! listener (installed once) authenticates a `hello` by matching
//! `event.source` against the registered iframes' live `contentWindow`
//! (never by `event.origin`, which is `"null"` at an opaque origin).
//! On a match it binds the transferred port to that portal's
//! [`PortalState`] and posts `ready { context }` back. The per-port
//! dispatcher then answers each inbound envelope (`preview`,
//! `navigate`, `reload`, `title`, `open`, `analytics`, `register`,
//! `task`, `fetch`, `delegate`, `key`) on the trusted page.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use js_sys::{Object, Reflect};
use tonk_host::bridge::context_field;
use tonk_host::location::{Allow, Location};
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use wasm_bindgen::closure::Closure;
use wasm_bindgen_futures::spawn_local;
use web_sys::{
    AbortController, Element, HtmlIFrameElement, MessageEvent, MessageEventInit, MessagePort,
    window,
};

use crate::space_origin::{
    broker_port, clear_unreachable, deliver_document, grant_relayed_port, relay_port,
    requested_location,
};

/// Per-portal bridge + iframe state. Held behind `Rc<RefCell<…>>` so
/// it is reachable from the element lifecycle and the page-level
/// message listener.
pub(crate) struct PortalState {
    /// The single child iframe. Owned here so attribute callbacks can
    /// reload it and `disconnected_callback` can detach it.
    pub iframe: Option<HtmlIFrameElement>,
    /// Set by `disconnected_callback`; mirrors `<tonk-display>`.
    pub disposed: bool,
    /// Abort handles for every fetch this portal relayed. Aborted (and
    /// drained) on teardown so no response keeps streaming into a
    /// destroyed guest realm.
    relays: Vec<AbortController>,
    /// The one trusted-page task leased to this guest, if any.
    active_task: Option<crate::task::Request>,
    /// The port bound by the latest `hello` handshake, used to relay
    /// results back to the iframe. `None` until the iframe says hello.
    port: Option<MessagePort>,
    /// The current port's `onmessage` dispatcher, kept alive for the
    /// port's lifetime. Replaced on each handshake.
    _dispatcher: Option<Closure<dyn FnMut(MessageEvent)>>,
    /// The top document's listener that forwards the command palette's
    /// chord down to this guest (see [`relay_chord_down`]). Replaced on
    /// each handshake; dropping it removes the listener.
    chord: Option<ChordRelay>,
    /// The portal's own routing context (its `with`). `allow`'s `self`
    /// entry resolves to it.
    with: Option<Location>,
    /// Which locations this portal permits its guest to reach. A
    /// **privilege of the trusted portal element**, set host-side at
    /// construction — NOT something the guest can assert. `<tonk-site>`
    /// derives it from its `allow` attribute; `<tonk-fab-portal>` grants
    /// `*`; the generic `<tonk-portal>` grants `self`, so a
    /// synced/untrusted content guest's fetch of another location is
    /// denied with a typed error. See `handle_host_fetch`.
    allow: Allow,
    /// The space's real origin when this portal renders it there (a
    /// `<tonk-site origin>`), rather than in an opaque `srcdoc` frame.
    origin: Option<String>,
    /// The bootstrap document a real-origin frame asks for once its space
    /// worker is in control: the markup a sealed frame gets as `srcdoc`.
    document: Option<String>,
    /// The authority real-origin sites render under, handed down to the
    /// guest so the sites it nests render on origins of their own too.
    pub(crate) site_pattern: Option<String>,
    /// Which load of the real-origin frame this is, and whether its shell
    /// has announced itself. A frame that finishes loading without its
    /// shell could not be reached (see `space_origin::watch_shell`).
    pub(crate) shell: Cell<Shell>,
}

/// One load of a real-origin frame, counted so that a check scheduled for an
/// earlier load does not judge a later one.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Shell {
    /// The load this is, starting at 1.
    pub(crate) load: u32,
    /// Whether the shell of this load has announced itself.
    pub(crate) seen: bool,
}

impl PortalState {
    pub(crate) fn new() -> Self {
        Self {
            iframe: None,
            disposed: false,
            relays: Vec::new(),
            active_task: None,
            port: None,
            _dispatcher: None,
            chord: None,
            with: None,
            allow: Allow::none(),
            origin: None,
            document: None,
            site_pattern: None,
            shell: Cell::default(),
        }
    }

    /// Render this portal's space at `origin`, handing the frame `document`
    /// once it asks. Called host-side by `connect_portal` and on reload.
    pub(crate) fn set_origin_document(
        &mut self,
        origin: String,
        document: String,
        site_pattern: Option<String>,
    ) {
        self.origin = Some(origin);
        self.document = Some(document);
        self.site_pattern = site_pattern;
        self.shell.set(Shell {
            load: self.shell.get().load + 1,
            seen: false,
        });
    }

    /// The space's real origin, when this portal renders it there.
    pub(crate) fn origin(&self) -> Option<&str> {
        self.origin.as_deref()
    }

    /// Set this portal's routing context and reach. Called once, host-side,
    /// by the trusted portal element during `connect_portal`.
    pub(crate) fn set_route(&mut self, with: Option<Location>, allow: Allow) {
        self.with = with;
        self.allow = allow;
    }

    /// This portal's target space (a `did:key` string), or `None` when it
    /// targets the profile/Hub. Drives the guest's synthetic `<base>` origin.
    pub(crate) fn route_space(&self) -> Option<String> {
        self.with
            .as_ref()
            .and_then(|w| w.space())
            .map(str::to_owned)
    }

    /// Whether this portal's routing context and reach match exactly.
    /// `<tonk-site>` uses this to re-route a pure path change in place —
    /// same reach, live iframe — instead of rebuilding the guest.
    pub(crate) fn same_route(&self, with: &Location, allow: &Allow) -> bool {
        self.with.as_ref() == Some(with) && self.allow == *allow
    }

    /// Cancel and forget every relayed fetch, and close the bridge port.
    /// Aborting each relay cancels the underlying fetch — including a
    /// streaming response whose body was TRANSFERRED into the guest. A
    /// torn-down guest must not leave live pipes into its destroyed
    /// realm: orphaned transferred streams are the prime suspect for the
    /// renderer crash on space→hub navigation.
    pub(crate) fn sever(&mut self) {
        for relay in self.relays.drain(..) {
            relay.abort();
        }
        // Close the bridge port too: a torn-down (or reloading) guest must
        // leave NO live browser-brokered endpoints behind — the in-flight
        // chunk drains terminate via the aborts above and close their own
        // ports.
        if let Some(port) = self.port.take() {
            port.close();
        }
    }

    /// Track a relayed fetch's abort handle for the portal's lifetime, so
    /// teardown can cancel it. Bounded by the portal's own lifetime — a
    /// navigation rebuild drains the lot.
    pub(crate) fn track_relay(&mut self, controller: AbortController) {
        self.relays.push(controller);
    }

    fn accept_task(&mut self, request: &crate::task::Request) -> Result<(), &'static str> {
        use crate::task::Action;

        match request.action {
            Action::Open => {
                if self.active_task.is_some() {
                    return Err("busy");
                }
                self.active_task = Some(request.clone());
            }
            Action::Reseat | Action::Suspend | Action::Show => {
                let Some(active) = self.active_task.as_ref() else {
                    return Err("stale");
                };
                if active.request_id != request.request_id {
                    return Err("stale");
                }
                self.active_task = Some(request.clone());
            }
            Action::Dismiss => {
                let Some(active) = self.active_task.as_ref() else {
                    return Err("stale");
                };
                if active.request_id != request.request_id {
                    return Err("stale");
                }
                self.active_task = None;
            }
        }
        Ok(())
    }

    fn finish_task(&mut self, request_id: &str) {
        if self
            .active_task
            .as_ref()
            .is_some_and(|request| request.request_id == request_id)
        {
            self.active_task = None;
        }
    }

    fn take_task_dismissal(&mut self) -> Option<crate::task::Request> {
        self.active_task.take().map(|mut request| {
            request.action = crate::task::Action::Dismiss;
            request.presentation = None;
            request
        })
    }
}

/// The bootstrap script prepended into the iframe's `srcdoc`. It defines
/// `window.tonk` synchronously, opens a `MessageChannel`, and hands one
/// port to the parent via `parent.postMessage(hello, "*", [port2])`.
/// Posting to `"*"` is unavoidable from a null origin; the parent
/// authenticates by `event.source`, not `event.origin`.
const BOOTSTRAP_JS: &str = include_str!("bootstrap.js");

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
const RUNTIME_BOOTSTRAP_JS: &str = include_str!("runtime_bootstrap.js");
const PREVIEW_CAPTURE_JS: &str = include_str!("preview_capture.js");

#[wasm_bindgen::prelude::wasm_bindgen(module = "/src/preview_cache.js")]
extern "C" {
    #[wasm_bindgen::prelude::wasm_bindgen(js_name = previewRequest)]
    fn preview_request(request: &JsValue, context: &JsValue, host: &Element) -> js_sys::Promise;
}

/// A `<base href>` element pinning the guest's document base to the
/// per-space synthetic origin, so the BROWSER resolves every relative URL
/// (links, forms, `new URL`, `<page-mount>` location reads) under it. Empty
/// when there is no space origin (the profile/Hub), leaving the guest's
/// inherited base untouched. Prepended before everything so it applies from
/// the first parsed node.
fn base_tag(base: &str) -> String {
    if base.is_empty() {
        String::new()
    } else {
        // `base` is a same-origin literal we built (`https://{label}.tonk.network/`),
        // so there is nothing to escape, but keep it minimal and attribute-safe.
        format!("<base href=\"{base}\">")
    }
}

/// Prepend the bootstrap script that wires `window.tonk` to this
/// portal's bridge over a `MessagePort`. `base` is the per-space synthetic
/// origin the guest should resolve URLs against (empty = leave inherited).
pub(crate) fn bootstrap_srcdoc(content: &str, base: &str, head: &str) -> String {
    format!(
        "{}{head}<script>{BOOTSTRAP_JS}</script><script>{PREVIEW_CAPTURE_JS}</script>{content}",
        base_tag(base)
    )
}

/// Like [`bootstrap_srcdoc`], plus the runtime-injection bootstrap: the
/// guest will ask the parent (`runtime-ready`) for the element runtime and
/// bring it up before `content`'s custom elements upgrade.
pub(crate) fn bootstrap_srcdoc_with_runtime(content: &str, base: &str, head: &str) -> String {
    format!(
        "{}{head}<script>{BOOTSTRAP_JS}</script><script>{RUNTIME_BOOTSTRAP_JS}</script><script>{PREVIEW_CAPTURE_JS}</script>{content}",
        base_tag(base)
    )
}

/// Fetch the element runtime + app CSS (the parent is trusted + networked)
/// and post an `inject` envelope to the sealed `iframe`'s window. Called
/// when the guest signals `runtime-ready`. The guest fetches nothing; every
/// byte crosses here.
///
/// A frame `on_origin` fetches for itself instead: it is told which build to
/// load and where the app stylesheet is, and loads them from its own origin,
/// where its worker and the HTTP cache keep them.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) fn inject_runtime(iframe: &HtmlIFrameElement, on_origin: bool) {
    let Some(content_window) = iframe.content_window() else {
        return;
    };
    spawn_local(async move {
        let built = if on_origin {
            build_origin_payload().await
        } else {
            build_inject_payload().await
        };
        let (payload, transfer) = match built {
            Ok(p) => p,
            Err(e) => {
                tonk_common::log!("portal runtime: failed to assemble payload: {e}");
                return;
            }
        };
        // Post to the iframe window (not the data port): runtime setup is a
        // one-time window-channel handoff, distinct from the tonk data port.
        // The large binary payloads (wasm + WA bundle) are TRANSFERRED by
        // ownership via the transfer list, not structured-clone-copied.
        let _ = content_window.post_message_with_transfer(&payload, "*", &transfer);
    });
}

/// The hashed guest-asset basenames the `hash-guest.sh` post_build hook
/// writes into `guest/manifest.json`. Each names a content-hashed file under
/// the guest dir, so those assets cache immutably while the manifest itself
/// is fetched fresh on every load.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
#[derive(serde::Deserialize)]
struct GuestManifest {
    js: String,
    wasm: String,
    #[serde(rename = "waJs")]
    wa_js: String,
    #[serde(rename = "waCss")]
    wa_css: String,
}

/// Build the envelope for a guest on its own origin: which build's assets to
/// load (the guest manifest), the app stylesheet's URL, and the root classes.
/// The `<tonk-prose>` and `<tonk-table>` registration shells still ride along,
/// as in [`build_inject_payload`]; the lazy editor cores stay on `need-*`.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn build_origin_payload() -> Result<(JsValue, JsValue), String> {
    let manifest = fetch_text("/guest/manifest.json").await?;
    let manifest = js_sys::JSON::parse(&manifest).map_err(|e| format!("guest manifest: {e:?}"))?;
    let payload = Object::new();
    let _ = Reflect::set(&payload, &"__tonkRuntime".into(), &"inject".into());
    let _ = Reflect::set(&payload, &"fromOrigin".into(), &JsValue::TRUE);
    let _ = Reflect::set(&payload, &"manifest".into(), &manifest);
    if let Some(href) = app_stylesheet_href() {
        let _ = Reflect::set(&payload, &"cssHref".into(), &JsValue::from_str(&href));
    }
    let prose = bundle_graph_entries(fetch_tonk_prose_shell().await);
    let table = bundle_graph_entries(fetch_tonk_table_shell().await);
    let _ = Reflect::set(&payload, &"prose".into(), &prose);
    let _ = Reflect::set(&payload, &"table".into(), &table);
    let root_class = window()
        .and_then(|w| w.document())
        .and_then(|d| d.document_element())
        .map(|e| e.class_name())
        .unwrap_or_default();
    let _ = Reflect::set(
        &payload,
        &"rootClass".into(),
        &JsValue::from_str(&root_class),
    );
    Ok((payload.into(), js_sys::Array::new().into()))
}

/// Build the runtime-inject envelope by fetching the served guest bundle +
/// app stylesheet. Returns `(payload, transfer)` for
/// `post_message_with_transfer`: the payload carries the glue/css/snippets as
/// strings plus the wasm + WA bundle as ArrayBuffers, and `transfer` lists
/// those buffers so they hand off by ownership instead of being copied.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn build_inject_payload() -> Result<(JsValue, JsValue), String> {
    use wasm_bindgen::JsValue;

    // The manifest names the current build's hashed assets. It rides the
    // SW's stale-while-revalidate cache like everything else, so a sealed
    // `/space` works OFFLINE — never `no-store`, which the SW refuses to
    // cache (an offline guest could then never resolve its assets). Serving
    // a stale manifest is safe: it points at the PREVIOUS build's hashed
    // assets, which are still cached (immutable, never evicted within a cache
    // version), so the guest loads fully; SWR refreshes the manifest in the
    // background and the next load picks up the new build.
    let manifest: GuestManifest = {
        let text = fetch_text("/guest/manifest.json").await?;
        serde_json::from_str(&text).map_err(|e| format!("guest manifest: {e}"))?
    };

    let glue = fetch_text(&format!("/guest/{}", manifest.js)).await?;
    let wasm = fetch_array_buffer(&format!("/guest/{}", manifest.wasm)).await?;
    // App stylesheet — its hashed filename is discovered from the parent
    // document's own `<link rel=stylesheet href=/styles-*.css>`. The Web
    // Awesome CSS + the self-contained WA component bundle ride along so
    // `<wa-*>` elements style + upgrade inside the sealed guest with no
    // network of its own.
    let mut css = fetch_text(&format!("/guest/{}", manifest.wa_css))
        .await
        .unwrap_or_default();
    if let Some(app_css) = app_stylesheet_css().await {
        css.push('\n');
        css.push_str(&app_css);
    }
    // Inline `@font-face url("/fonts/*")` as `data:` URLs: a null-origin guest
    // can't fetch the fonts (CORS-blocked), so the host (same-origin) fetches
    // each face and base64-embeds it. Handles woff2/woff/otf/ttf — the launcher
    // ships Gestalte as `.otf`, so limiting this to woff2 left it unstyled.
    css = inline_fonts(&css).await;
    // The bundled Web Awesome components (esbuild, no dynamic/relative
    // imports), imported by the guest before its content upgrades. Fetched as
    // an ArrayBuffer (not text): the guest never manipulates it as a string,
    // it just blobs + imports it, so we transfer the bytes (ownership moved,
    // no structured-clone copy) and the guest wraps them in a Blob zero-copy.
    let wa = fetch_array_buffer(&format!("/guest/{}", manifest.wa_js))
        .await
        .unwrap_or(JsValue::UNDEFINED);

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

    // The `<tonk-code>` editor bundle is NOT in the boot payload — not even a
    // shell, because `tonk-code.js` is itself the element definition and there
    // is nothing smaller to register. Its ~659 kB graph (main + dialog-yaml
    // pack + shared chunks) crosses the boundary over `need-code`, the first
    // time a `<tonk-code>`/`<tonk-diagnostics-provider>` appears in the
    // guest's DOM (see the observer in `BOOTSTRAP_JS` and `inject_code_core`).
    // Most guests — the Hub, settings, join, a space page — never mount an
    // editor, and now never pay for one.

    // The `<tonk-prose>` markdown editor SHELL only (~4 kB): enough to
    // register the element so guest markup upgrades. The editor core stays
    // out of the boot payload — the guest requests it over `need-prose` the
    // first time an element actually connects (see the listener in
    // `install_message_listener`), so guests that never render an editor
    // never pay for one. Its code blocks embed the `<tonk-code>` element
    // injected above.
    let prose = bundle_graph_entries(fetch_tonk_prose_shell().await);

    // The `<tonk-table>` spreadsheet SHELL only: same lazy contract as
    // tonk-prose above — the grid core and the multi-megabyte IronCalc
    // engine bytes stay out of the boot payload; the guest requests them
    // over `need-table` the first time an element actually connects.
    let table = bundle_graph_entries(fetch_tonk_table_shell().await);

    let payload = Object::new();
    let _ = Reflect::set(&payload, &"__tonkRuntime".into(), &"inject".into());
    let _ = Reflect::set(&payload, &"glue".into(), &JsValue::from_str(&glue));
    let _ = Reflect::set(&payload, &"snippets".into(), &snippets);
    let _ = Reflect::set(&payload, &"prose".into(), &prose);
    let _ = Reflect::set(&payload, &"table".into(), &table);
    let _ = Reflect::set(&payload, &"wasm".into(), &wasm);
    let _ = Reflect::set(&payload, &"css".into(), &JsValue::from_str(&css));
    let _ = Reflect::set(&payload, &"wa".into(), &wa);
    // Mirror the outer document's root classes (the WA theme/palette/dark
    // classes) so the guest themes identically — recomputing from
    // matchMedia inside the guest can disagree with the parent.
    let root_class = window()
        .and_then(|w| w.document())
        .and_then(|d| d.document_element())
        .map(|e| e.class_name())
        .unwrap_or_default();
    let _ = Reflect::set(
        &payload,
        &"rootClass".into(),
        &JsValue::from_str(&root_class),
    );

    // Transfer the two large binary payloads (the guest wasm + the WA bundle)
    // by OWNERSHIP rather than letting `postMessage` structured-clone-copy
    // them across the window boundary. `glue`/`css`/snippets stay as strings:
    // the guest manipulates them as text, so there's nothing to transfer.
    let transfer = js_sys::Array::new();
    if !wasm.is_undefined() {
        transfer.push(&wasm);
    }
    if !wa.is_undefined() {
        transfer.push(&wa);
    }
    Ok((payload.into(), transfer.into()))
}

/// Map a `/fonts/*` file extension to its `data:` MIME type. Any file under
/// `/fonts/` is inlined regardless of extension; this only picks a precise
/// MIME for the known font formats and falls back to a generic font MIME for
/// anything else, so a new face drops in without touching this code.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn font_mime(path: &str) -> &'static str {
    match path
        .rsplit('.')
        .next()
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("woff2") => "font/woff2",
        Some("woff") => "font/woff",
        Some("otf") => "font/otf",
        Some("ttf") => "font/ttf",
        Some("eot") => "application/vnd.ms-fontobject",
        // Unknown extension: a generic font MIME still renders (browsers sniff
        // the actual format from the bytes), so an arbitrary face still works.
        _ => "font/otf",
    }
}

/// Collect the distinct `/fonts/*` paths referenced as `url(...)` arguments
/// in `css`, in first-appearance order. Only genuine `url()` arguments
/// qualify: scanning for the raw `/fonts/` substring also matches prose in
/// comments — a comment in styles.css mentioning `` `/fonts/` `` used to
/// produce a junk `GET /fonts/%60%20(copied…` 404 on every guest boot.
fn find_font_paths(css: &str) -> Vec<String> {
    let mut paths: Vec<String> = Vec::new();
    let mut rest = css;
    while let Some(i) = rest.find("url(") {
        rest = &rest[i + "url(".len()..];
        let Some(close) = rest.find(')') else { break };
        let arg = rest[..close].trim().trim_matches(['"', '\'']);
        // Skip a bare `/fonts/` with no filename.
        if let Some(name) = arg.strip_prefix("/fonts/")
            && !name.is_empty()
            && !paths.iter().any(|p| p == arg)
        {
            paths.push(arg.to_owned());
        }
        rest = &rest[close + 1..];
    }
    paths
}

/// Replace every `url("/fonts/<name>.<ext>")` in `css` with a
/// `url("data:<mime>;base64,…")` so the sealed guest needs no font fetch.
/// Inlines ANY file under `/fonts/`, not a fixed set of extensions. Fonts
/// whose fetch/encode fails are left as-is (degrade to a fallback face).
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn inline_fonts(css: &str) -> String {
    let mut paths = find_font_paths(css);
    // Substitute longest-first so a path that is a prefix of another
    // (`a.woff` next to `a.woff2`) isn't corrupted by the shorter one's
    // replacement landing inside it.
    paths.sort_by_key(|p| std::cmp::Reverse(p.len()));

    let mut out = css.to_owned();
    for path in paths {
        if let Ok(buffer) = fetch_array_buffer(&path).await
            && let Some(b64) = array_buffer_to_base64(&buffer)
        {
            let data_url = format!("data:{};base64,{b64}", font_mime(&path));
            // Replace the path wherever it appears as a url argument.
            out = out.replace(&path, &data_url);
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
            if let Some(q) = quote
                && let Some(end) = after[1..].find(q)
            {
                let spec = &after[1..1 + end];
                // statement without a trailing `;`-only tail variance:
                // keep the trimmed line up to and including the close quote
                let stmt_end = from_idx + 6 + 1 + end + 1;
                let stmt = trimmed[..stmt_end].to_owned();
                out.push((stmt, spec.to_owned()));
            }
        }
    }
    out
}

/// Find the relative `./…` ESM import specifiers in a module's source — both
/// static (`from"./x"`) and dynamic (`import("./x")`). Used to walk an editor
/// bundle's chunk graph (tonk-code, tonk-prose) so every referenced file can be
/// fetched and injected (the sealed guest can't fetch siblings at its opaque
/// origin).
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn find_relative_imports(src: &str) -> Vec<String> {
    let mut out = Vec::new();
    // Match `"./<name>"` and `'./<name>'` occurrences anywhere — covers
    // `from"./x.js"`, `import("./x.js")`, and the language-pack URL template.
    for quote in ['"', '\''] {
        let needle = format!("{quote}./");
        let mut rest = src;
        while let Some(i) = rest.find(&needle) {
            let after = &rest[i + needle.len()..];
            if let Some(end) = after.find(quote) {
                let name = &after[..end];
                // Skip the language-pack template literal (`./tonk-code-lang-…`
                // contains a `${…}` placeholder, handled separately).
                if !name.contains("${") && !out.contains(&name.to_owned()) {
                    out.push(name.to_owned());
                }
                rest = &after[end + 1..];
            } else {
                break;
            }
        }
    }
    out
}

/// Fetch the `<tonk-code>` editor bundle graph from `/tonk-code/` for guest
/// injection: the main element bundle, the dialog-yaml language pack, and every
/// `chunk-*.js` either transitively imports.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn fetch_tonk_code_bundles() -> Vec<(String, String)> {
    // Entry points with stable (unhashed) names served at `/tonk-code/`.
    fetch_bundle_graph(
        "/tonk-code",
        &["tonk-code.js", "tonk-code-lang-dialog-yaml.js"],
    )
    .await
}

/// Fetch ONLY the `<tonk-prose>` registration shell for the guest boot
/// payload. Deliberately not `fetch_bundle_graph`: the shell's source
/// mentions `"./tonk-prose-editor.js"` (its default-resolution fallback),
/// and the graph walk would follow it — eagerly shipping the ~400 kB core
/// to every guest, which is exactly what the lazy split avoids.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn fetch_tonk_prose_shell() -> Vec<(String, String)> {
    match fetch_text("/tonk-prose/tonk-prose.js").await {
        Ok(src) => vec![("tonk-prose.js".to_owned(), src)],
        Err(e) => {
            web_sys::console::warn_1(&JsValue::from_str(&format!(
                "/tonk-prose inject: skipping tonk-prose.js: {e}"
            )));
            Vec::new()
        }
    }
}

/// Fetch the `<tonk-prose>` editor-core graph (the core chunk plus anything
/// it transitively imports) for the on-demand `need-prose` reply.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn fetch_tonk_prose_core() -> Vec<(String, String)> {
    fetch_bundle_graph("/tonk-prose", &["tonk-prose-editor.js"]).await
}

/// Reply to a guest's `need-prose` request: fetch the editor-core graph
/// (the parent is trusted + networked; the sealed guest can't fetch) and
/// post it back as an `inject-prose` envelope on the guest's window. Called
/// from the page-level message listener when the first `<tonk-prose>` in
/// that guest connects. Best-effort like the boot inject — an empty graph
/// makes the guest's promise reject and the element render empty rather
/// than wedging the runtime.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn inject_prose_core(iframe: &HtmlIFrameElement) {
    let Some(content_window) = iframe.content_window() else {
        return;
    };
    spawn_local(async move {
        let prose = bundle_graph_entries(fetch_tonk_prose_core().await);
        let payload = Object::new();
        let _ = Reflect::set(&payload, &"__tonkRuntime".into(), &"inject-prose".into());
        let _ = Reflect::set(&payload, &"prose".into(), &prose);
        let _ = content_window.post_message(&payload, "*");
    });
}

/// Reply to a guest's `need-code` request: fetch the `<tonk-code>` editor
/// bundle graph (the parent is trusted + networked; the sealed guest can't
/// fetch) and post it back as an `inject-code` envelope on the guest's
/// window. Called from the page-level message listener when a
/// `<tonk-code>` or `<tonk-diagnostics-provider>` first appears in that
/// guest's DOM.
///
/// Unlike prose/table this is not a *core* top-up over an already-registered
/// shell: the element is undefined until this reply lands, because
/// `tonk-code.js` is the definition. Best-effort all the same — an empty
/// graph leaves the element undefined, so a consumer's
/// `whenDefined("tonk-code")` stays pending and it renders without an
/// editor rather than wedging the runtime.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn inject_code_core(iframe: &HtmlIFrameElement) {
    let Some(content_window) = iframe.content_window() else {
        return;
    };
    spawn_local(async move {
        let code = bundle_graph_entries(fetch_tonk_code_bundles().await);
        let payload = Object::new();
        let _ = Reflect::set(&payload, &"__tonkRuntime".into(), &"inject-code".into());
        let _ = Reflect::set(&payload, &"code".into(), &code);
        let _ = content_window.post_message(&payload, "*");
    });
}

/// Fetch ONLY the `<tonk-table>` registration shell for the guest boot
/// payload. Deliberately not `fetch_bundle_graph`: the shell's source
/// mentions `"./tonk-table-grid.js"` (its default-resolution fallback),
/// and the graph walk would follow it — eagerly shipping the grid and
/// the multi-megabyte engine-bytes leaf to every guest, which is
/// exactly what the lazy split avoids.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn fetch_tonk_table_shell() -> Vec<(String, String)> {
    match fetch_text("/tonk-table/tonk-table.js").await {
        Ok(src) => vec![("tonk-table.js".to_owned(), src)],
        Err(e) => {
            web_sys::console::warn_1(&JsValue::from_str(&format!(
                "/tonk-table inject: skipping tonk-table.js: {e}"
            )));
            Vec::new()
        }
    }
}

/// Fetch the `<tonk-table>` grid core for the on-demand `need-table`
/// reply: the grid chunk plus the engine-bytes leaf, BY NAME rather
/// than via `fetch_bundle_graph`. The grid chunk embeds the wasm
/// IMPORT-OBJECT key `"./wasm_bg.js"` — a string the engine wasm names
/// its import module by, not a real file — and the graph walk would
/// chase it into the SPA's HTML fallback, after which the guest-side
/// blob rewrite would corrupt the key and `WebAssembly.instantiate`
/// would reject the engine. The build pins the file set (three fixed
/// entries, no code splitting), so the explicit list is an invariant,
/// not a guess.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn fetch_tonk_table_core() -> Vec<(String, String)> {
    fetch_bundle_files(
        "/tonk-table",
        &["tonk-table-grid.js", "tonk-table-engine.js"],
    )
    .await
}

/// Fetch an explicit list of bundle files (no graph walk) from `base`
/// for guest injection. Best-effort like `fetch_bundle_graph` — a
/// missing file is skipped, so the feature degrades rather than
/// failing the whole inject.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn fetch_bundle_files(base: &str, names: &[&str]) -> Vec<(String, String)> {
    let mut files: Vec<(String, String)> = Vec::new();
    for name in names {
        match fetch_text(&format!("{base}/{name}")).await {
            Ok(src) => files.push(((*name).to_owned(), src)),
            Err(e) => {
                web_sys::console::warn_1(&JsValue::from_str(&format!(
                    "{base} inject: skipping {name}: {e}"
                )));
            }
        }
    }
    files
}

/// Reply to a guest's `need-table` request: fetch the grid core (the
/// parent is trusted + networked; the sealed guest can't fetch) and
/// post it back as an `inject-table` envelope on the guest's window.
/// Called from the page-level message listener when the first
/// `<tonk-table>` in that guest connects. Best-effort like the boot
/// inject — an empty graph makes the guest's promise reject and the
/// element render empty rather than wedging the runtime.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn inject_table_core(iframe: &HtmlIFrameElement) {
    let Some(content_window) = iframe.content_window() else {
        return;
    };
    spawn_local(async move {
        let table = bundle_graph_entries(fetch_tonk_table_core().await);
        let payload = Object::new();
        let _ = Reflect::set(&payload, &"__tonkRuntime".into(), &"inject-table".into());
        let _ = Reflect::set(&payload, &"table".into(), &table);
        let _ = content_window.post_message(&payload, "*");
    });
}

/// Fetch a code-split editor bundle graph (`entries` + every `./…` chunk they
/// transitively import) from `base` for guest injection. Returns
/// `(name, src)` pairs the guest blobs + import-rewrites. Best-effort — a
/// missing file is skipped, so the editor degrades rather than failing the
/// whole inject.
///
/// These bundles are code-split (esbuild `splitting:true`, required for a
/// single module identity per shared dependency), so they can't be one
/// self-contained ESM like the WA bundle; instead the guest mints a blob per
/// file and rewrites relative imports to those blobs.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn fetch_bundle_graph(base: &str, entries: &[&str]) -> Vec<(String, String)> {
    let mut files: Vec<(String, String)> = Vec::new();
    let mut queue: Vec<String> = entries.iter().map(|s| s.to_string()).collect();

    while let Some(name) = queue.pop() {
        if files.iter().any(|(n, _)| n == &name) {
            continue;
        }
        // Cache-first, like every other guest-boot asset. This used to
        // `reload` (force a network fetch, bypassing the cache) because the
        // entry points have STABLE names and a rebuilt editor must reach
        // the guest. But forcing the network made a cached load on a slow
        // connection pay the full download every time — the tonk-code graph
        // is ~3 MB, so a 3G reload took seconds to fetch bytes it already had
        // cached, while an offline reload (which can't reach the network) was
        // instant. The SW's stale-while-revalidate serves the cached copy
        // immediately and refreshes in the background, so a content change
        // reaches the guest on the NEXT load — acceptable: the chunks are
        // content-hashed (immutable), only the entry points can change,
        // and dev hot-reload already does a full page reload on a real code
        // change.
        let src = match fetch_text(&format!("{base}/{name}")).await {
            Ok(src) => src,
            Err(e) => {
                web_sys::console::warn_1(&JsValue::from_str(&format!(
                    "{base} inject: skipping {name}: {e}"
                )));
                continue;
            }
        };
        // Enqueue every chunk this file imports (e.g. a language pack also
        // pulls shared chunks).
        for spec in find_relative_imports(&src) {
            if !files.iter().any(|(n, _)| n == &spec) && !queue.contains(&spec) {
                queue.push(spec);
            }
        }
        files.push((name, src));
    }
    files
}

/// Package `(name, src)` bundle files as a JS array of `{name, src}` objects
/// for the inject payload.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn bundle_graph_entries(files: Vec<(String, String)>) -> js_sys::Array {
    let array = js_sys::Array::new();
    for (name, src) in files {
        let entry = Object::new();
        let _ = Reflect::set(&entry, &"name".into(), &JsValue::from_str(&name));
        let _ = Reflect::set(&entry, &"src".into(), &JsValue::from_str(&src));
        array.push(&entry);
    }
    array
}

/// The app stylesheet's URL, from this document's own
/// `<link rel=stylesheet href=/styles-*.css>`: the top document's, or the one
/// a guest on its own origin linked when it loaded.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn app_stylesheet_href() -> Option<String> {
    let links = window()?
        .document()?
        .query_selector_all("link[rel=stylesheet]")
        .ok()?;
    (0..links.length()).find_map(|i| {
        let el: Element = links.item(i)?.dyn_into().ok()?;
        el.get_attribute("href")
            .filter(|href| href.contains("/styles-") || href.ends_with("styles.css"))
    })
}

/// The app stylesheet CSS to inject into a guest, read from the document that is
/// bringing the guest up.
///
/// Two cases, because a guest can nest:
/// - **Top document**: it links the app CSS as `<link rel=stylesheet
///   href=/styles-*.css>`; fetch that href's content.
/// - **A guest bringing up a NESTED guest**: it has NO such `<link>` — its own
///   app CSS was injected as an inline `<style data-tonk-app-css>` (it was itself
///   a guest). Read that style's text content directly, so the app CSS
///   propagates down every nesting level instead of stopping at level one.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn app_stylesheet_css() -> Option<String> {
    let document = window()?.document()?;

    // Top document: a `<link rel=stylesheet href=/styles-*.css>`.
    if let Ok(links) = document.query_selector_all("link[rel=stylesheet]") {
        for i in 0..links.length() {
            let Some(node) = links.item(i) else { continue };
            let Ok(el) = node.dyn_into::<Element>() else {
                continue;
            };
            if let Some(href) = el.get_attribute("href")
                && (href.contains("/styles-") || href.ends_with("styles.css"))
            {
                return fetch_text(&href).await.ok();
            }
        }
    }

    // A guest bringing up a nested guest: its injected app CSS is inline.
    if let Ok(Some(style)) = document.query_selector("style[data-tonk-app-css]") {
        return style.text_content().filter(|c| !c.is_empty());
    }

    None
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn fetch_text(url: &str) -> Result<String, String> {
    resp_text(fetch(url).await?).await
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn resp_text(resp: web_sys::Response) -> Result<String, String> {
    let text =
        wasm_bindgen_futures::JsFuture::from(resp.text().map_err(|e| format!("text(): {e:?}"))?)
            .await
            .map_err(|e| format!("await text: {e:?}"))?;
    text.as_string().ok_or_else(|| "text not a string".into())
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn fetch_array_buffer(url: &str) -> Result<JsValue, String> {
    let resp = fetch(url).await?;
    // A missing hashed asset 200s with the SPA fallback (`text/html`). Reject
    // that here so HTML bytes never reach `WebAssembly.instantiate` as a
    // bogus magic word — surface a clear error instead.
    if let Some(ct) = resp.headers().get("content-type").ok().flatten()
        && ct.contains("text/html")
    {
        return Err(format!("fetch {url}: got HTML (asset missing?)"));
    }
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
    // Default cache mode (no override): these URLs are content-hashed (named
    // by the guest manifest), so the SW's stale-while-revalidate shell cache
    // can hold them immutably — a sealed `/space` works OFFLINE (populated on
    // the first online load, served from cache after) and a content change
    // is a NEW URL (cache miss → fresh), never a stale hit. The manifest rides
    // the same SWR cache so an offline guest can still resolve its assets.
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

            // A real-origin site frame: its shell asks for the document once
            // its worker is in control, and its broker asks for a port to the
            // host worker whenever that worker has none. A request naming a
            // location was relayed up from a frame nested in it, and is granted
            // only if this portal's `allow` reaches that location. All of it is
            // answered only for a registered frame, only at its own origin.
            if let Some(kind) = get_str(&data, "__tonkOrigin") {
                let source = Reflect::get(&event, &"source".into()).unwrap_or(JsValue::NULL);
                if kind == "relay-port" {
                    pass_relayed_port(&registry, &event, &source);
                    return;
                }
                let matched = registry.borrow().iter().find_map(|entry| {
                    let cw: JsValue = entry.iframe.content_window()?.into();
                    (cw == source).then(|| (entry.iframe.clone(), entry.state.clone()))
                });
                let Some((iframe, state)) = matched else {
                    return;
                };
                let state = state.borrow();
                let Some(origin) = state.origin().filter(|origin| *origin == event.origin()) else {
                    return;
                };
                match kind.as_str() {
                    "shell" => {
                        state.shell.set(Shell {
                            seen: true,
                            ..state.shell.get()
                        });
                        clear_unreachable(&iframe);
                    }
                    "shell-ready" => {
                        if let Some(document) = state.document.as_deref() {
                            deliver_document(&iframe, origin, document);
                        }
                    }
                    // The profile's worker told the page that asked it
                    // something (go here, run this passkey ceremony), and
                    // its own frame heard it. The page hears it as though
                    // its own worker had said it. Only from the profile:
                    // a space's frame holds author code.
                    "worker-message" => {
                        if state.with.as_ref().is_some_and(Location::profile) {
                            hear_worker_message(&data);
                        }
                    }
                    "need-port" => match requested_location(&data) {
                        Some(requested) if state.allow.permits(&requested) => {
                            grant_relayed_port(&iframe, origin, &requested);
                        }
                        Some(_) => {}
                        None => {
                            if let Some(with) = state.with.as_ref() {
                                broker_port(&iframe, origin, with);
                            }
                        }
                    },
                    _ => {}
                }
                return;
            }

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
                            let on_origin = entry.state.borrow().origin().is_some();
                            (cw == source).then(|| (entry.iframe.clone(), on_origin))
                        });
                        #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
                        if let Some((iframe, on_origin)) = matched {
                            inject_runtime(&iframe, on_origin);
                        }
                    }
                    // Lazy `<tonk-prose>` editor core: the boot payload only
                    // carries the registration shell; the guest asks for the
                    // core when the first element connects.
                    "need-prose" => {
                        let matched = registry.borrow().iter().find_map(|entry| {
                            let cw: JsValue = entry.iframe.content_window()?.into();
                            (cw == source).then(|| entry.iframe.clone())
                        });
                        #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
                        if let Some(iframe) = matched {
                            inject_prose_core(&iframe);
                        }
                    }
                    // Lazy `<tonk-code>` editor bundle: nothing rides the boot
                    // payload, so this is the ONLY path by which the element
                    // is ever defined in a guest. The guest asks when a
                    // `<tonk-code>`/`<tonk-diagnostics-provider>` first
                    // appears in its DOM.
                    "need-code" => {
                        let matched = registry.borrow().iter().find_map(|entry| {
                            let cw: JsValue = entry.iframe.content_window()?.into();
                            (cw == source).then(|| entry.iframe.clone())
                        });
                        #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
                        if let Some(iframe) = matched {
                            inject_code_core(&iframe);
                        }
                    }
                    // Lazy `<tonk-table>` grid core (grid + engine bytes):
                    // same contract as `need-prose` above.
                    "need-table" => {
                        let matched = registry.borrow().iter().find_map(|entry| {
                            let cw: JsValue = entry.iframe.content_window()?.into();
                            (cw == source).then(|| entry.iframe.clone())
                        });
                        #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
                        if let Some(iframe) = matched {
                            inject_table_core(&iframe);
                        }
                    }
                    "error" => {
                        tonk_common::log!(
                            "portal guest runtime error: {}",
                            get_str(&data, "error").unwrap_or_default()
                        );
                    }
                    "warn" => {
                        tonk_common::log!(
                            "portal guest runtime warn: {}",
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

/// Pass a port the parent minted down to the real-origin frame rendering the
/// location it names. Only the parent document, at the host's origin, may send
/// one.
fn pass_relayed_port(
    registry: &Rc<RefCell<Vec<PortalEntry>>>,
    event: &MessageEvent,
    source: &JsValue,
) {
    let Some(window) = window() else {
        return;
    };
    let from_parent = window
        .parent()
        .ok()
        .flatten()
        .is_some_and(|parent| JsValue::from(parent) == *source);
    let from_host = tonk_host::bridge::context_origin().is_some_and(|host| host == event.origin());
    if !from_parent || !from_host {
        return;
    }
    let (Some(requested), Some(port)) = (requested_location(&event.data()), read_first_port(event))
    else {
        return;
    };
    let registry = registry.borrow();
    let target = registry.iter().find_map(|entry| {
        let state = entry.state.borrow();
        let with = state.with.as_ref()?;
        let origin = state.origin()?;
        with.same_reach(&requested)
            .then(|| (entry.iframe.clone(), origin.to_owned(), with.clone()))
    });
    if let Some((iframe, origin, with)) = target {
        relay_port(&iframe, &origin, &with, port);
    }
}

/// Whether a portal in this document renders the site at `with` on an origin
/// of its own.
pub(crate) fn renders(with: &Location) -> bool {
    REGISTRY.with(|registry| {
        registry.borrow().iter().any(|entry| {
            let state = entry.state.borrow();
            state.origin().is_some()
                && state
                    .with
                    .as_ref()
                    .is_some_and(|rendered| rendered.same_reach(with))
        })
    })
}

/// Dispatch what a site's worker said on this page's own service worker
/// container, where the page listens for its worker. Only the top document
/// does: a nested one is not the page.
fn hear_worker_message(data: &JsValue) {
    let Some(window) = window() else {
        return;
    };
    let nested = window
        .parent()
        .ok()
        .flatten()
        .is_some_and(|parent| JsValue::from(parent) != JsValue::from(window.clone()));
    if nested {
        return;
    }
    let Ok(message) = Reflect::get(data, &"message".into()) else {
        return;
    };
    let init = MessageEventInit::new();
    init.set_data(&message);
    if let Ok(event) = MessageEvent::new_with_event_init_dict("message", &init) {
        let _ = window.navigator().service_worker().dispatch_event(&event);
    }
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

    state.borrow_mut().chord = relay_chord_down(host, &port, state);

    let ready = Object::new();
    set_v1(&ready, "ready");
    let _ = Reflect::set(&ready, &"context".into(), &build_context(host, state));
    let _ = port.post_message(&ready);
    let init = web_sys::CustomEventInit::new();
    init.set_bubbles(true);
    init.set_composed(true);
    if let Ok(event) = web_sys::CustomEvent::new_with_event_init_dict("tonk:guest-ready", &init) {
        let _ = host.dispatch_event(&event);
    }
}

/// Update URL context before a reused guest receives the next route frame.
pub(crate) fn refresh_context(host: &Element, state: &Rc<RefCell<PortalState>>) {
    let port = state.borrow().port.clone();
    if let Some(port) = port {
        let envelope = Object::new();
        set_v1(&envelope, "context");
        let _ = Reflect::set(&envelope, &"context".into(), &build_context(host, state));
        let _ = port.post_message(&envelope);
    }
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
            "preview" => {
                let Some(id) = get_str(&data, "id") else {
                    return;
                };
                let context = build_context(&host, &state);
                let request = Reflect::get(&data, &"request".into()).unwrap_or(JsValue::NULL);
                let promise = preview_request(&request, &context, &host);
                let port = port.clone();
                spawn_local(async move {
                    let value = wasm_bindgen_futures::JsFuture::from(promise)
                        .await
                        .unwrap_or(JsValue::NULL);
                    post_result(&port, "preview-result", &id, "value", &value);
                });
            }
            "navigate" => handle_navigate(&state, &data),
            "reload" => tonk_host::reload_page(),
            "title" => handle_title(&data),
            "open" => handle_open(&state, &data),
            "analytics" => handle_analytics(&data),
            "register" => handle_register(&state, &port, &data),
            "task" => handle_task(&state, &port, &data),
            "fetch" => handle_host_fetch(&state, &port, &data),
            "delegate" => handle_delegate(&port, &data),
            "key" => handle_key(&host, &data),
            _ => {}
        }
    }) as Box<dyn FnMut(MessageEvent)>)
}

/// A request the portal refuses to relay. A distinct type rather than a
/// collapse to `None`: it is the seam for a future capability-request
/// flow, where an un-listed request prompts to extend `allow` rather
/// than simply failing.
#[derive(Debug)]
enum Refused {
    /// The requested location is not in the portal's `allow`.
    Denied { requested: Location },
}

impl Refused {
    fn message(&self) -> String {
        match self {
            Refused::Denied { requested } => {
                format!("denied: route {requested} is not permitted by this site's allow")
            }
        }
    }
}

/// Navigate the host page to `href`. The sealed guest can't touch its
/// parent's location, so a link click inside it posts the href here and the
/// trusted parent performs the navigation — as a client-side route change
/// (`pushState` + `popstate`), never a reload: the top `<tonk-site>` re-routes
/// its path in place and the running guest re-renders via its `tonk:site`
/// subscription. With `replace` the current history entry changes address
/// instead of a new one being added, and with `delta` in place of `href` the
/// page moves that many entries through its history.
fn handle_navigate(state: &Rc<RefCell<PortalState>>, data: &JsValue) {
    if let Some(delta) = Reflect::get(data, &"delta".into())
        .ok()
        .and_then(|delta| delta.as_f64())
    {
        tonk_host::traverse(delta as i32);
        return;
    }
    let Some(href) = get_str(data, "href").filter(|h| !h.is_empty()) else {
        return;
    };
    let href = real_href(state, &href);
    let replace = Reflect::get(data, &"replace".into()).is_ok_and(|replace| replace.is_truthy());
    if replace {
        tonk_host::replace_to(&href);
    } else {
        tonk_host::navigate_to(&href);
    }
}

/// Forward a closed analytics envelope toward the top page.
fn handle_analytics(data: &JsValue) {
    let Some(event) = get_str(data, "event").filter(|event| !event.is_empty()) else {
        return;
    };
    tonk_host::analytics::relay(&event);
}

/// Translate a guest-world href into the REAL route the host navigates to.
///
/// The guest resolves links against its synthetic per-space origin
/// (`https://{label}.tonk.network/`), so an in-space link arrives as a bare
/// absolute path (`/activity`). The document is really served at
/// `/space/{did}/...`, so prefix the space segment. A guest with no space
/// context (profile/Hub), or an already-`/space/...` path, is left as-is.
fn real_href(state: &Rc<RefCell<PortalState>>, href: &str) -> String {
    let Some(space) = state.borrow().route_space() else {
        return href.to_owned();
    };
    // Root of the space ("/") maps to the space's own route.
    if href == "/" {
        return format!("/space/{space}");
    }
    // A leading-slash in-space path; anything else (already absolute host
    // path, or a fragment/query) is passed through untouched.
    if let Some(rest) = href.strip_prefix('/') {
        if rest.starts_with("space/") || is_top_level_route(rest) {
            href.to_owned()
        } else {
            format!("/space/{space}/{rest}")
        }
    } else {
        href.to_owned()
    }
}

/// Routes that belong to the PROFILE, not to any space.
///
/// The guest resolves every link against its synthetic per-space origin, so a
/// link to one of these arrives looking exactly like an in-space path and would
/// be rewritten to `/space/{did}/join` — a route no space defines. The page then
/// tries to boot the whole app inside the sealed frame, where the origin is
/// opaque: no service worker, every asset CORS-blocked, and the renderer dies.
///
/// These names are the profile's own route table (`profile.yaml`), which no
/// space route shadows, so passing them through is unambiguous.
fn is_top_level_route(rest: &str) -> bool {
    let head = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    matches!(head, "join" | "account" | "inspector" | "diagnose")
}

/// Set the host page's tab title on the guest's behalf. The guest's
/// `<tab-title>` posts `{v:1, type:"title", text}`; this runs in the
/// parent document, which is where `document.title` lives.
/// Raise the host's registration dialog for a share that needs an
/// account.
///
/// The dialog itself lives in `tonk-ui`, which depends on this crate, so
/// it cannot be called by name from here. The top page registers a
/// handler at boot instead — the same shape as the other page effects,
/// where this crate carries the transport and the shell supplies the
/// behaviour.
fn handle_register(state: &Rc<RefCell<PortalState>>, port: &MessagePort, data: &JsValue) {
    let Some((reason, token)) = register_request(data) else {
        return;
    };
    let focus_return = token.map(|token| RegisterFocusReturn {
        port: port.clone(),
        frame: state.borrow().iframe.clone(),
        token,
        handled: false,
    });
    REGISTER_HANDLER.with(|handler| {
        if let Some(handler) = handler.borrow().as_ref() {
            handler(&reason, focus_return);
        } else if let Some(window) = window()
            && let Ok(tonk) = Reflect::get(&window, &"tonk".into())
            && let Ok(register) = Reflect::get(&tonk, &"register".into())
            && let Some(register) = register.dyn_ref::<js_sys::Function>()
        {
            // A sealed guest may itself host portals. Only the outer shell
            // owns account UI; relay through this guest's established port.
            let _ = relay_register(register, &tonk, &reason, focus_return);
        }
    });
}

fn relay_register(
    register: &js_sys::Function,
    receiver: &JsValue,
    reason: &str,
    focus_return: Option<RegisterFocusReturn>,
) -> Result<(), JsValue> {
    let held = Rc::new(RefCell::new(focus_return));
    let callback = Closure::<dyn FnMut(String)>::new(move |kind: String| {
        if kind == "custody-open" {
            if let Some(reply) = held.borrow().as_ref() {
                reply.show_custody();
            }
        } else if let Some(reply) = held.borrow_mut().take() {
            match kind.as_str() {
                "register-focus" => reply.restore(),
                "custody-focus" => reply.restore_custody(),
                // Dropping an unhandled reply discards only this child's token.
                _ => {}
            }
        }
    })
    .into_js_value();
    register.call2(receiver, &reason.into(), &callback)?;
    Ok(())
}

/// A one-shot return path to the exact control in a sealed guest that asked
/// the top page to open registration.
pub struct RegisterFocusReturn {
    port: MessagePort,
    frame: Option<HtmlIFrameElement>,
    token: String,
    handled: bool,
}

impl RegisterFocusReturn {
    /// Return focus to the still-connected guest opener and consume its token.
    pub fn restore(mut self) {
        if let Some(frame) = self.frame.as_ref()
            && frame.is_connected()
        {
            let _ = frame.focus();
        }
        self.post("register-focus");
        self.handled = true;
    }

    /// Replace the guest approval rows once the top-page prompt is ready.
    pub fn show_custody(&self) {
        self.post("custody-open");
    }

    /// Finish an account custody screen without closing Hub registration.
    pub fn restore_custody(mut self) {
        self.post("custody-focus");
        self.handled = true;
    }

    fn post(&self, kind: &str) {
        let envelope = Object::new();
        set_v1(&envelope, kind);
        let _ = Reflect::set(
            &envelope,
            &"focusToken".into(),
            &JsValue::from_str(&self.token),
        );
        let _ = self.port.post_message(&envelope);
    }
}

impl Drop for RegisterFocusReturn {
    fn drop(&mut self) {
        if !self.handled {
            self.post("register-focus-discard");
        }
    }
}

/// What a page does when a guest asks it to raise registration.
type RegisterHandler = Box<dyn Fn(&str, Option<RegisterFocusReturn>)>;

thread_local! {
    /// What to do when a guest asks for registration. `None` until the
    /// shell installs one. Nested sealed guests relay through their existing
    /// parent port; a page with neither handler nor bridge drops the request.
    static REGISTER_HANDLER: std::cell::RefCell<Option<RegisterHandler>> =
        const { std::cell::RefCell::new(None) };
}

/// Install what runs when a guest asks the host to register an account.
///
/// Called once by the shell at boot. Later calls replace the handler,
/// which keeps a hot reload from stacking dialogs.
pub fn on_register(handler: impl Fn(&str, Option<RegisterFocusReturn>) + 'static) {
    REGISTER_HANDLER.with(|slot| {
        *slot.borrow_mut() = Some(Box::new(handler));
    });
}

/// Read `reason` out of a `{ type: "register", reason }` message, or
/// `None` when the message is not one. Split out so the parse is
/// testable on its own, the way [`title_text`] is.
fn register_request(data: &JsValue) -> Option<(String, Option<String>)> {
    if get_str(data, "type")? != "register" {
        return None;
    }
    let reason = get_str(data, "reason").filter(|reason| !reason.is_empty())?;
    let token = get_str(data, "focusToken").filter(|token| !token.is_empty());
    Some((reason, token))
}

/// A one-shot result path from the trusted page to the exact sealed guest
/// that requested a contained task.
pub struct ContainedTaskReturn {
    port: MessagePort,
    frame: Option<HtmlIFrameElement>,
    state: Weak<RefCell<PortalState>>,
    request_id: Option<String>,
    token: String,
    handled: bool,
}

impl ContainedTaskReturn {
    /// Finish the request, restore the connected guest frame and consume the
    /// return token.
    pub fn finish(mut self, result: &str) {
        if let Some(frame) = self.frame.as_ref()
            && frame.is_connected()
        {
            let _ = frame.focus();
        }
        self.release_task();
        self.post(result);
        self.handled = true;
    }

    fn release_task(&self) {
        let Some(request_id) = self.request_id.as_deref() else {
            return;
        };
        if let Some(state) = self.state.upgrade() {
            state.borrow_mut().finish_task(request_id);
        }
    }

    fn post(&self, result: &str) {
        let envelope = Object::new();
        set_v1(&envelope, "task-result");
        let _ = Reflect::set(
            &envelope,
            &"focusToken".into(),
            &JsValue::from_str(&self.token),
        );
        let _ = Reflect::set(&envelope, &"result".into(), &JsValue::from_str(result));
        let _ = self.port.post_message(&envelope);
    }
}

impl Drop for ContainedTaskReturn {
    fn drop(&mut self) {
        if !self.handled {
            self.release_task();
            self.post("disconnected");
        }
    }
}

type TaskHandler = Box<dyn Fn(crate::task::Request, Option<ContainedTaskReturn>)>;

thread_local! {
    static TASK_HANDLER: std::cell::RefCell<Option<TaskHandler>> =
        const { std::cell::RefCell::new(None) };
}

/// Install what runs when a sealed guest asks for a trusted-page contained
/// task. Later calls replace the handler so hot reload cannot stack hosts.
pub fn on_task(handler: impl Fn(crate::task::Request, Option<ContainedTaskReturn>) + 'static) {
    TASK_HANDLER.with(|slot| {
        *slot.borrow_mut() = Some(Box::new(handler));
    });
}

fn handle_task(state: &Rc<RefCell<PortalState>>, port: &MessagePort, data: &JsValue) {
    let Some((payload, token)) = task_request(data) else {
        return;
    };
    let make_return = |request_id: Option<&str>| {
        token.as_ref().map(|token| ContainedTaskReturn {
            port: port.clone(),
            frame: state.borrow().iframe.clone(),
            state: Rc::downgrade(state),
            request_id: request_id.map(str::to_owned),
            token: token.clone(),
            handled: false,
        })
    };
    let request = match crate::task::Request::parse(&payload).and_then(|request| {
        let offset = state
            .borrow()
            .iframe
            .as_ref()
            .map(|frame| {
                let element: &Element = frame.unchecked_ref();
                let rect = element.get_bounding_client_rect();
                (rect.left(), rect.top())
            })
            .unwrap_or((0.0, 0.0));
        request.translated(offset.0, offset.1)
    }) {
        Ok(request) => request,
        Err(_) => {
            if let Some(reply) = make_return(None) {
                reply.finish("invalid");
            }
            return;
        }
    };

    if let Err(result) = state.borrow_mut().accept_task(&request) {
        if let Some(reply) = make_return(None) {
            reply.finish(result);
        }
        return;
    }

    let request_id = request.request_id.clone();
    dispatch_task(request, make_return(Some(&request_id)), true);
}

fn dispatch_task(
    request: crate::task::Request,
    focus_return: Option<ContainedTaskReturn>,
    relay: bool,
) {
    let mut request = Some(request);
    let mut focus_return = focus_return;
    TASK_HANDLER.with(|handler| {
        if let Some(handler) = handler.borrow().as_ref() {
            handler(request.take().expect("task request"), focus_return.take());
        }
    });
    if request.is_none() {
        return;
    }
    if !relay {
        return;
    }
    if let Some(window) = window()
        && let Ok(tonk) = Reflect::get(&window, &"tonk".into())
        && let Ok(task) = Reflect::get(&tonk, &"task".into())
        && let Some(task) = task.dyn_ref::<js_sys::Function>()
        && let Ok(payload) = request.as_ref().expect("unhandled request").to_json()
    {
        let _ = relay_task(task, &tonk, &payload, focus_return.take());
    }
}

/// Dismiss the task leased to a guest before its port and iframe disappear.
pub(crate) fn disconnect_task(state: &Rc<RefCell<PortalState>>) {
    let request = state.borrow_mut().take_task_dismissal();
    if let Some(request) = request {
        dispatch_task(request, None, true);
    }
}

fn relay_task(
    task: &js_sys::Function,
    receiver: &JsValue,
    payload: &str,
    focus_return: Option<ContainedTaskReturn>,
) -> Result<(), JsValue> {
    let held = Rc::new(RefCell::new(focus_return));
    let callback = Closure::<dyn FnMut(String)>::new(move |result: String| {
        if let Some(reply) = held.borrow_mut().take() {
            reply.finish(&result);
        }
    })
    .into_js_value();
    task.call2(receiver, &payload.into(), &callback)?;
    Ok(())
}

fn task_request(data: &JsValue) -> Option<(String, Option<String>)> {
    if get_str(data, "type")? != "task" {
        return None;
    }
    let payload = get_str(data, "payload").filter(|payload| !payload.is_empty())?;
    let token = get_str(data, "focusToken").filter(|token| !token.is_empty());
    Some((payload, token))
}

/// A keyboard chord pressed inside the guest (the bootstrap forwards only
/// the command palette's): re-dispatch it from this portal element, so it
/// bubbles through the document the portal lives in as if pressed there.
fn handle_key(host: &Element, data: &JsValue) {
    let flag = |name: &str| {
        Reflect::get(data, &name.into())
            .ok()
            .and_then(|value| value.as_bool())
            .unwrap_or(false)
    };
    let init = web_sys::KeyboardEventInit::new();
    init.set_key(&get_str(data, "key").unwrap_or_default());
    init.set_ctrl_key(flag("ctrlKey"));
    init.set_meta_key(flag("metaKey"));
    init.set_shift_key(flag("shiftKey"));
    init.set_alt_key(flag("altKey"));
    init.set_bubbles(true);
    init.set_composed(true);
    init.set_cancelable(true);
    if let Ok(event) = web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init) {
        let _ = host.dispatch_event(&event);
    }
}

/// A document `keydown` listener, removed when dropped.
pub(crate) struct ChordRelay {
    document: web_sys::Document,
    listener: Closure<dyn FnMut(web_sys::KeyboardEvent)>,
}

impl Drop for ChordRelay {
    fn drop(&mut self) {
        let _ = self
            .document
            .remove_event_listener_with_callback("keydown", self.listener.as_ref().unchecked_ref());
    }
}

/// Whether `event` is the command palette's chord: Ctrl/Cmd+K or
/// Ctrl/Cmd+Shift+P.
fn is_chord(event: &web_sys::KeyboardEvent) -> bool {
    let key = event.key().to_lowercase();
    (event.meta_key() || event.ctrl_key()) && (key == "k" || (event.shift_key() && key == "p"))
}

/// Forward the command palette's chord from the top document down into
/// this portal's guest.
///
/// The palette lives in the profile's frame, but on a fresh load focus is
/// in the top document, whose keys never reach a frame; the chord did
/// nothing until the page was clicked. The guest bootstrap forwards a
/// chord UP (see `handle_key`); this is the other direction. Only the top
/// document relays down, so a chord pressed in the profile frame is not
/// also pushed into the space frame below it. Only a trusted, unhandled
/// chord is relayed (the re-dispatched upward copy is untrusted, so a
/// chord never bounces), and not while a modal dialog of the top page
/// owns the keyboard. The guest's frame is focused first: a sandboxed
/// guest cannot take focus from its parent, and the palette focuses its
/// line. Only the site (the page's own frame) is relayed to, never a
/// content portal.
fn relay_chord_down(
    host: &Element,
    port: &MessagePort,
    state: &Rc<RefCell<PortalState>>,
) -> Option<ChordRelay> {
    if !host.tag_name().eq_ignore_ascii_case("tonk-site") {
        return None;
    }
    let window = window()?;
    let top = window.top().ok().flatten()?;
    if !Object::is(&top, &window) {
        return None;
    }
    let document = window.document()?;
    let port = port.clone();
    let weak = Rc::downgrade(state);
    let modal_host = document.clone();
    let listener = Closure::wrap(Box::new(move |event: web_sys::KeyboardEvent| {
        if !event.is_trusted() || event.default_prevented() || !is_chord(&event) {
            return;
        }
        if modal_host
            .query_selector("dialog:modal")
            .ok()
            .flatten()
            .is_some()
        {
            return;
        }
        let Some(state) = weak.upgrade() else {
            return;
        };
        if state.borrow().disposed {
            return;
        }
        event.prevent_default();
        if let Some(iframe) = state.borrow().iframe.clone() {
            let _ = iframe.focus();
        }
        let envelope = Object::new();
        set_v1(&envelope, "key");
        let _ = Reflect::set(&envelope, &"key".into(), &event.key().into());
        for (name, value) in [
            ("ctrlKey", event.ctrl_key()),
            ("metaKey", event.meta_key()),
            ("shiftKey", event.shift_key()),
            ("altKey", event.alt_key()),
        ] {
            let _ = Reflect::set(&envelope, &name.into(), &value.into());
        }
        let _ = port.post_message(&envelope);
    }) as Box<dyn FnMut(web_sys::KeyboardEvent)>);
    document
        .add_event_listener_with_callback("keydown", listener.as_ref().unchecked_ref())
        .ok()?;
    Some(ChordRelay { document, listener })
}

fn handle_title(data: &JsValue) {
    let Some(text) = title_text(data) else {
        return;
    };
    tonk_host::set_title(&text);
}

/// Read `text` out of a `{ type: "title", text }` message, or `None` when
/// the message isn't a title or carries no usable text. The dispatcher
/// has already matched on `type`; re-checking it here keeps the parse
/// independently testable, as `navigate_href` does in `tonk-host`.
fn title_text(data: &JsValue) -> Option<String> {
    if get_str(data, "type")? != "title" {
        return None;
    }
    get_str(data, "text").filter(|text| !text.is_empty())
}

/// Open a link on the guest's behalf. The sealed guest has no `allow-popups`
/// and no `allow-top-navigation`, so it cannot open anything itself; it posts
/// the raw href and `tonk_host::open_external` — running on the page, which is
/// the only place that can both resolve and open it — decides what happens.
/// Mint a delegation under the passkey on the guest's behalf.
///
/// The guest asks `{ subject, command, audience }`; the account root that
/// signs it lives behind the passkey, which exists only on this top-level
/// window and only inside a user gesture. The guest's click propagates its
/// activation to this frame, so the ceremony runs here immediately and the
/// prompt is the user's own gesture. The hop minted is `root -> audience`
/// over `subject` at `command`; the guest carries it to the worker, which
/// checks it against what it composes it with. Answered with
/// `delegate-result` carrying the base58 chain, or `delegate-error`.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn handle_delegate(port: &MessagePort, data: &JsValue) {
    let Some(id) = get_str(data, "id") else {
        return;
    };
    let request = (
        get_str(data, "subject").unwrap_or_default(),
        get_str(data, "command").unwrap_or_default(),
        get_str(data, "audience").unwrap_or_default(),
    );
    let port = port.clone();
    wasm_bindgen_futures::spawn_local(async move {
        match mint_delegation(&request.0, &request.1, &request.2).await {
            Ok(encoded) => post_result(
                &port,
                "delegate-result",
                &id,
                "delegation",
                &JsValue::from_str(&encoded),
            ),
            Err(error) => post_error(&port, "delegate-error", &id, &format!("{error:#}")),
        }
    });
}

/// Run the passkey ceremony and mint `root -> audience` over `subject` at
/// `command`, returning the serialized chain as base58.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn mint_delegation(subject: &str, command: &str, audience: &str) -> anyhow::Result<String> {
    use dialog_ucan_core::command::Command;
    use dialog_ucan_core::subject::Subject as UcanSubject;
    use dialog_ucan_core::{DelegationBuilder, DelegationChain};
    use dialog_varsig::Did;

    let subject: Did = subject
        .parse()
        .map_err(|error| anyhow::anyhow!("the subject is not a DID: {error:?}"))?;
    let audience: Did = audience
        .parse()
        .map_err(|error| anyhow::anyhow!("the audience is not a DID: {error:?}"))?;
    let command = Command::parse(command)
        .map_err(|error| anyhow::anyhow!("the command does not parse: {error}"))?;
    // The custody endpoint the page's other ceremonies use: the account
    // service is served under `/ucan/` on the page's own origin.
    let origin = web_sys::window()
        .and_then(|window| window.location().origin().ok())
        .ok_or_else(|| anyhow::anyhow!("window origin is unavailable"))?;
    let endpoint = format!("{}/ucan/", origin.trim_end_matches('/'));
    let root = tonk_identity::ceremony::unlock_root(&endpoint).await?;
    let delegation = DelegationBuilder::new()
        .issuer(dialog_credentials::Signer::from(root))
        .audience(&audience)
        .subject(UcanSubject::Specific(subject))
        .command(command.segments().clone())
        .try_build()
        .await
        .map_err(|error| anyhow::anyhow!("failed to mint the delegation: {error}"))?;
    let bytes = DelegationChain::new(delegation).to_bytes()?;
    Ok(bs58::encode(bytes).into_string())
}

fn handle_open(state: &Rc<RefCell<PortalState>>, data: &JsValue) {
    let Some(href) = open_href(data) else {
        return;
    };
    // `open` is for hrefs that escaped the guest's synthetic origin, so the
    // href is normally a full external URL and passes through. Defensively map
    // a bare in-space path too (`real_href` no-ops on external URLs, which
    // don't start with a single `/`).
    tonk_host::open_external(&real_href(state, &href));
}

/// Read `href` out of an `{ type: "open", href }` message, or `None` when the
/// message isn't an open or carries no usable href. Mirrors `title_text`.
fn open_href(data: &JsValue) -> Option<String> {
    if get_str(data, "type")? != "open" {
        return None;
    }
    get_str(data, "href").filter(|href| !href.is_empty())
}

/// Perform a same-origin fetch on the host and stream the response back. The
/// opaque guest can't reach a same-origin, SW-routed `/api/...` endpoint
/// itself, so it asks the host (which IS same-origin). Restricted to
/// host-relative paths (`/…`, not `//`) so the guest can't drive the host to
/// fetch arbitrary cross-origin URLs.
///
/// The host does its own `fetch`, then posts a `fetch-result` envelope back
/// over the port carrying the status, status text, headers, and the response
/// body's `ReadableStream` — TRANSFERRED (not copied) so the bytes never
/// round-trip through wasm. The guest rebuilds a real streaming `Response`
/// from those, so its overridden `window.fetch` is faithful (`.text()`,
/// `.blob()`, `.arrayBuffer()`, `.body` all work) and binary-safe.
///
/// Branch data-plane paths are gated by this portal's `with`/`allow`
/// before the fetch runs — with the guest's IO riding plain `fetch`,
/// this relay IS the reach chokepoint, for the elements' requests and
/// for raw guest `fetch()` calls alike.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn handle_host_fetch(state: &Rc<RefCell<PortalState>>, port: &MessagePort, data: &JsValue) {
    let Some(id) = get_str(data, "id") else {
        return;
    };
    let Some(path) = get_str(data, "path") else {
        return post_error(port, "fetch-error", &id, "missing path");
    };
    if !path.starts_with('/') || path.starts_with("//") {
        return post_error(port, "fetch-error", &id, "path must be host-relative");
    };
    if let Some(requested) = data_plane_location(&path, state) {
        let s = state.borrow();
        let permitted = s
            .with
            .as_ref()
            .is_some_and(|own| own.same_reach(&requested))
            || s.allow.permits(&requested);
        if !permitted {
            let denied = Refused::Denied { requested };
            tonk_common::log!("portal fetch {}", denied.message());
            return post_error(port, "fetch-error", &id, &denied.message());
        }
    }
    // The guest forwards the full request so POST query/subscribe/transact work,
    // not just GET. Build the `RequestInit` (method, headers, body) and fetch the
    // bare relative path as a STRING — never a `Request`, which would resolve the
    // path against this document's baseURI. When the host is itself a sealed guest
    // (a NESTED portal), that baseURI is the real origin, so a `Request` would
    // make the path a cross-origin absolute URL its OWN `window.fetch` override
    // can't relay (origin `null` → CORS). The string path lets each level's
    // override catch the host-relative `/…` and relay up to its parent.
    let init = match build_relayed_request(data) {
        Ok(init) => init,
        Err(e) => return post_error(port, "fetch-error", &id, &e),
    };
    // Every relay is abortable and tracked on the portal: teardown aborts
    // the lot, so a torn-down guest's streams (transferred response bodies
    // included) are cancelled instead of piping into a destroyed realm.
    if let Ok(controller) = AbortController::new() {
        init.set_signal(Some(&controller.signal()));
        state.borrow_mut().track_relay(controller);
    }
    let port = port.clone();
    spawn_local(async move {
        match fetch_path(&path, &init).await {
            Ok(resp) => post_fetch_response(&port, &id, &resp).await,
            Err(e) => post_error(&port, "fetch-error", &id, &e),
        }
    });
}

/// The repository reach a relayed path targets, if any:
/// `/api/repository/{repo}` or `/api/repository/{repo}/branch/{branch}/…`.
/// Non-data-plane paths (assets, the guest bundle, `/api/sync`, and
/// repository control routes) return `None`.
///
/// A `profile:<name>` repository segment names the profile's own
/// repository. The worker serves one profile whatever name the segment
/// carries, so it canonicalizes to the portal's own profile name when the
/// portal is profile-pinned, else the worker's default (`tonk`).
fn data_plane_location(path: &str, state: &Rc<RefCell<PortalState>>) -> Option<Location> {
    use tonk_host::location::Repo;
    let path = path.split_once('?').map_or(path, |(path, _)| path);
    let rest = path.strip_prefix("/api/repository/")?;
    let mut segments = rest.split('/');
    let repo = segments.next().filter(|s| !s.is_empty())?;
    let profile = repo.starts_with("profile:")
        || repo
            .get(..10)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("profile%3a"));
    let repo = if profile {
        let name = state
            .borrow()
            .with
            .as_ref()
            .and_then(|own| match &own.repo {
                Repo::Profile(name) => Some(name.clone()),
                Repo::Named(_) => None,
            })
            .unwrap_or_else(|| "tonk".to_owned());
        Repo::Profile(name)
    } else {
        Repo::Named(repo.to_owned())
    };
    let branch = match segments.next() {
        None => "main",
        Some("branch") => segments.next().filter(|s| !s.is_empty())?,
        _ => return None,
    };
    Some(Location {
        repo,
        branch: Some(branch.to_owned()),
    })
}

/// Build a `Request` for a relayed guest fetch from the envelope's
/// `method`/`headers`/`body`. `headers` is an array of `[name, value]` pairs;
/// `body` is a string (our `/api` bodies are JSON) or absent.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn build_relayed_request(data: &JsValue) -> Result<web_sys::RequestInit, String> {
    let init = web_sys::RequestInit::new();
    let method = get_str(data, "method").unwrap_or_else(|| "GET".to_owned());
    init.set_method(&method);

    let headers = web_sys::Headers::new().map_err(|e| format!("Headers: {e:?}"))?;
    if let Ok(pairs) = Reflect::get(data, &"headers".into())
        && let Ok(pairs) = pairs.dyn_into::<js_sys::Array>()
    {
        for pair in pairs.iter() {
            let pair: js_sys::Array = match pair.dyn_into() {
                Ok(p) => p,
                Err(_) => continue,
            };
            if let (Some(name), Some(value)) = (pair.get(0).as_string(), pair.get(1).as_string()) {
                let _ = headers.append(&name, &value);
            }
        }
    }
    init.set_headers(&headers);

    // Body — only for methods that carry one. A bodyless GET/HEAD with a body
    // set throws, so only attach when present and non-null.
    let body = Reflect::get(data, &"body".into()).unwrap_or(JsValue::UNDEFINED);
    if !body.is_undefined() && !body.is_null() {
        init.set_body(&body);
    }

    Ok(init)
}

/// Perform a host-side `fetch(path, init)` and return the `Response`. The path is
/// passed as a STRING (not a `Request`) so a nested-guest host's overridden
/// `window.fetch` catches the host-relative `/…` and relays it up — see
/// [`handle_host_fetch`].
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn fetch_path(path: &str, init: &web_sys::RequestInit) -> Result<web_sys::Response, String> {
    let win = window().ok_or("no window")?;
    let resp_value = wasm_bindgen_futures::JsFuture::from(win.fetch_with_str_and_init(path, init))
        .await
        .map_err(|e| format!("fetch: {e:?}"))?;
    resp_value
        .dyn_into::<web_sys::Response>()
        .map_err(|_| "fetch: not a Response".to_string())
}

/// Post a `fetch-result` envelope carrying the response status + headers and
/// the body, streamed to the guest.
///
/// Body delivery has two paths, chosen by whether the browser can transfer a
/// `ReadableStream` over `postMessage`:
///
/// - **Fast path** (Chrome, Firefox, Safari 27+): transfer `response.body`
///   itself — one transfer, native streaming, zero plumbing.
/// - **Fallback** (Safari before 27, which throws `DataCloneError` on a
///   stream transfer): transfer one end of a fresh `MessageChannel` and drain
///   the body into it as chunks, with credit-based backpressure (see
///   [`drain_body_to_port`]). The guest rebuilds a `ReadableStream` fed by
///   that port.
///
/// Either way the guest gets a real streaming `Response`. A bodyless response
/// (e.g. 204) sends neither.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn post_fetch_response(port: &MessagePort, id: &str, resp: &web_sys::Response) {
    let env = Object::new();
    set_v1(&env, "fetch-result");
    let _ = Reflect::set(&env, &"id".into(), &JsValue::from_str(id));
    let _ = Reflect::set(
        &env,
        &"status".into(),
        &JsValue::from_f64(resp.status() as f64),
    );
    let _ = Reflect::set(
        &env,
        &"statusText".into(),
        &JsValue::from_str(&resp.status_text()),
    );
    // The final URL the host fetched (post-redirect). The guest can't
    // recover it — `new Response(...)` leaves `url` as `""` and the
    // property is readonly — so it travels on the envelope and the guest
    // shadows the getter with it. Without it, a guest consumer that parses
    // `response.url` fails on every relayed fetch.
    let _ = Reflect::set(&env, &"url".into(), &JsValue::from_str(&resp.url()));
    // Headers as an array of [name, value] pairs — structured-clonable and
    // re-hydrated into a `Headers` on the guest side.
    let _ = Reflect::set(&env, &"headers".into(), &headers_to_array(&resp.headers()));

    let Some(body) = resp.body() else {
        // Bodyless response — send the head with no body.
        let _ = port.post_message(&env);
        return;
    };

    // Fast path: attempt to transfer the stream itself. We only learn whether
    // the browser supports it by trying — a probe post on a throwaway channel,
    // so a `DataCloneError` here never reaches the guest.
    if streams_are_transferable() {
        let transfer = js_sys::Array::new();
        let _ = Reflect::set(&env, &"body".into(), &body);
        transfer.push(&body);
        match port.post_message_with_transferable(&env, &transfer) {
            Ok(()) => return,
            // Shouldn't happen once the probe passed, but if it does, fall
            // through to the chunked path rather than dropping the response.
            Err(e) => {
                tonk_common::log!("portal fetch: stream transfer failed post-probe: {e:?}");
            }
        }
    }

    // Fallback: drain the body into a MessageChannel with credit-based
    // backpressure. Strip the (untransferable) stream off the head envelope
    // and hand the guest a port instead.
    let _ = Reflect::delete_property(&env, &"body".into());
    drain_body_to_port(port, env, &body);
}

/// Whether this browser can transfer a `ReadableStream` over `postMessage`.
/// Detected once by probing a throwaway `MessageChannel` (the result is
/// cached): Safari before 27 throws `DataCloneError`, every other current
/// browser succeeds.
///
/// Transfers were briefly disabled while chasing a browser-process crash;
/// the real trigger was the SYNCHRONOUS destruction of a live nested guest
/// (now a two-phase teardown: unload to `about:blank`, remove a tick
/// later, with every relay aborted and every port closed first). With that
/// fixed, the zero-copy transfer path is back.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn streams_are_transferable() -> bool {
    thread_local! {
        static SUPPORTED: std::cell::OnceCell<bool> = const { std::cell::OnceCell::new() };
    }
    SUPPORTED.with(|cell| {
        *cell.get_or_init(|| {
            let Ok(channel) = web_sys::MessageChannel::new() else {
                return false;
            };
            let stream = web_sys::ReadableStream::new().unwrap_or_else(|_| JsValue::NULL.into());
            let transfer = js_sys::Array::new();
            transfer.push(&stream);
            channel
                .port1()
                .post_message_with_transferable(&JsValue::NULL, &transfer)
                .is_ok()
        })
    })
}

/// Drain `body` into a fresh `MessageChannel`, transferring the guest's end on
/// the `head` envelope (as `streamPort`). Credit-based backpressure: the guest
/// posts `{type:"credit", n}` and the host reads + posts up to `n` more chunks
/// (`{type:"chunk", buffer}` transferred), then `{type:"close"}` on EOF or
/// `{type:"error", error}` on a read failure. A guest `{type:"cancel"}`
/// cancels the reader. Used only when `ReadableStream` transfer is unavailable.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn drain_body_to_port(port: &MessagePort, head: Object, body: &web_sys::ReadableStream) {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    let Ok(channel) = web_sys::MessageChannel::new() else {
        return post_error_obj(port, &head, "host could not open a stream channel");
    };
    let host_port = channel.port1();
    let guest_port = channel.port2();

    // Hand the guest its end on the head envelope (transferred).
    let _ = Reflect::set(&head, &"streamPort".into(), &guest_port);
    let transfer = js_sys::Array::new();
    transfer.push(&guest_port);
    if let Err(e) = port.post_message_with_transferable(&head, &transfer) {
        tonk_common::log!("portal fetch: failed to hand off stream port: {e:?}");
        return;
    }

    let Ok(reader_val) = body
        .get_reader()
        .dyn_into::<web_sys::ReadableStreamDefaultReader>()
    else {
        return;
    };
    let reader = Rc::new(reader_val);
    // Available credit + a "pump in flight" guard so concurrent credit grants
    // don't launch overlapping reader loops (a reader allows one read at a
    // time).
    let credit = Rc::new(Cell::new(0u32));
    let pumping = Rc::new(Cell::new(false));
    let host_port = Rc::new(host_port);
    let cancelled = Rc::new(Cell::new(false));

    // The pump: while there's credit and we're not already reading, read one
    // chunk and post it, decrementing credit. Re-entrant-safe via `pumping`.
    // The pump closure re-invokes itself (to drain remaining credit) and is
    // also invoked by the credit handler, so it lives behind a shared cell.
    type PumpCell = Rc<RefCell<Option<Closure<dyn FnMut()>>>>;
    let pump: PumpCell = Rc::new(RefCell::new(None));
    {
        let reader = reader.clone();
        let credit = credit.clone();
        let pumping = pumping.clone();
        let host_port = host_port.clone();
        let cancelled = cancelled.clone();
        let pump_ref = pump.clone();
        let closure = Closure::wrap(Box::new(move || {
            if pumping.get() || cancelled.get() || credit.get() == 0 {
                return;
            }
            pumping.set(true);
            let reader = reader.clone();
            let credit = credit.clone();
            let pumping = pumping.clone();
            let host_port = host_port.clone();
            let cancelled = cancelled.clone();
            let pump_ref = pump_ref.clone();
            spawn_local(async move {
                let result = wasm_bindgen_futures::JsFuture::from(reader.read()).await;
                pumping.set(false);
                if cancelled.get() {
                    return;
                }
                match result {
                    Ok(chunk) => {
                        let done = Reflect::get(&chunk, &"done".into())
                            .ok()
                            .and_then(|v| v.as_bool())
                            .unwrap_or(true);
                        if done {
                            let env = Object::new();
                            let _ = Reflect::set(&env, &"type".into(), &"close".into());
                            let _ = host_port.post_message(&env);
                            // The stream is over — close the host end NOW.
                            // Every chunk channel is a browser-brokered pair
                            // of ports; leaving them to the GC keeps live
                            // endpoints into guest frames that may be mid-
                            // teardown, which is exactly the churn the
                            // renderer has crashed under.
                            host_port.close();
                            return;
                        }
                        // `value` is a `Uint8Array`, possibly a view windowed
                        // into a larger backing buffer (byteOffset/byteLength).
                        // Transfer the backing buffer (zero-copy — the whole
                        // point of a transfer) and carry the window offsets so
                        // the guest reconstructs a view over exactly this
                        // chunk's bytes, not the sibling bytes that may share
                        // the buffer.
                        let view = js_sys::Uint8Array::new(
                            &Reflect::get(&chunk, &"value".into()).unwrap_or(JsValue::NULL),
                        );
                        let buffer = view.buffer();
                        let env = Object::new();
                        let _ = Reflect::set(&env, &"type".into(), &"chunk".into());
                        let _ = Reflect::set(&env, &"chunk".into(), &buffer);
                        let _ = Reflect::set(
                            &env,
                            &"byteOffset".into(),
                            &JsValue::from_f64(view.byte_offset() as f64),
                        );
                        let _ = Reflect::set(
                            &env,
                            &"byteLength".into(),
                            &JsValue::from_f64(view.byte_length() as f64),
                        );
                        let transfer = js_sys::Array::new();
                        transfer.push(&buffer);
                        let _ = host_port.post_message_with_transferable(&env, &transfer);
                        credit.set(credit.get().saturating_sub(1));
                        // More credit may remain — keep pumping.
                        if let Some(cb) = pump_ref.borrow().as_ref() {
                            let _ =
                                js_sys::Function::from(cb.as_ref().clone()).call0(&JsValue::NULL);
                        }
                    }
                    Err(e) => {
                        let env = Object::new();
                        let _ = Reflect::set(&env, &"type".into(), &"error".into());
                        let _ = Reflect::set(
                            &env,
                            &"error".into(),
                            &JsValue::from_str(&format!("{e:?}")),
                        );
                        let _ = host_port.post_message(&env);
                        // Terminal — free the port pair (see the EOF arm).
                        host_port.close();
                    }
                }
            });
        }) as Box<dyn FnMut()>);
        *pump.borrow_mut() = Some(closure);
    }

    // The host port's message handler: grant credit, or cancel.
    let onmessage = {
        let credit = credit.clone();
        let cancelled = cancelled.clone();
        let reader = reader.clone();
        let pump = pump.clone();
        let host_port = host_port.clone();
        Closure::wrap(Box::new(move |event: MessageEvent| {
            let data = event.data();
            match get_str(&data, "type").as_deref() {
                Some("credit") => {
                    let n = Reflect::get(&data, &"n".into())
                        .ok()
                        .and_then(|v| v.as_f64())
                        .unwrap_or(0.0) as u32;
                    credit.set(credit.get().saturating_add(n));
                    if let Some(cb) = pump.borrow().as_ref() {
                        let _ = js_sys::Function::from(cb.as_ref().clone()).call0(&JsValue::NULL);
                    }
                }
                Some("cancel") => {
                    cancelled.set(true);
                    let _ = reader.cancel();
                    // Terminal — free the port pair (see the EOF arm).
                    host_port.close();
                }
                _ => {}
            }
        }) as Box<dyn FnMut(MessageEvent)>)
    };
    host_port.set_onmessage(Some(onmessage.as_ref().unchecked_ref()));
    // Keep everything alive for the stream's lifetime. `onmessage` is leaked
    // (it's the only owner the browser-side port references). It holds an `Rc`
    // clone of `pump` (the `RefCell<Option<Closure>>`), which keeps the pump
    // closure itself alive — so the credit handler can still invoke it. Do NOT
    // take the pump out of the cell: that would empty it and the handler would
    // find nothing to pump.
    onmessage.forget();
}

/// Post a `fetch-error` derived from a partially built `fetch-result` head
/// (reusing its `id`).
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn post_error_obj(port: &MessagePort, head: &Object, message: &str) {
    if let Some(id) = get_str(head, "id") {
        post_error(port, "fetch-error", &id, message);
    }
}

/// Serialize a `Headers` into a `[[name, value], …]` array. `Headers`
/// isn't structured-clonable, but this pair array is, and the guest
/// reconstructs a `Headers` from it.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn headers_to_array(headers: &web_sys::Headers) -> js_sys::Array {
    let out = js_sys::Array::new();
    let iter = js_sys::try_iter(headers).ok().flatten();
    if let Some(iter) = iter {
        for entry in iter.flatten() {
            // Each entry is a `[name, value]` array already.
            out.push(&entry);
        }
    }
    out
}

/// Build the `context` object (`{ this, model, origin, repo, branch }`) the
/// iframe receives in its `ready` envelope. `this`/`model` come from the
/// host's attributes; `origin` is the host page's real origin (the opaque
/// guest's is `"null"`); `repo`/`branch` come from the portal's `with`
/// context — which lives OUTSIDE the iframe, so a guest control that would
/// normally resolve its repo via a `with` ancestor reads it from the context
/// instead. Anything needing a same-origin URL or the scoped repo reads it
/// from `window.tonk.context` rather than the DOM/`window.location`.
fn build_context(host: &Element, state: &Rc<RefCell<PortalState>>) -> Object {
    let context = Object::new();
    let this = host.get_attribute("entity").unwrap_or_default();
    let model = host.get_attribute("model").unwrap_or_default();
    let location = window().map(|w| w.location());
    // The host's real origin. Read it via `context_origin()`, not
    // `location.origin()` directly: when THIS portal is itself inside a sealed
    // guest (a NESTED `<tonk-site>`), the host document is `about:srcdoc` and
    // `location.origin()` is the opaque string `"null"`. `context_origin()`
    // prefers the origin the parent portal already forwarded in
    // `window.tonk.context.origin`, so the real origin propagates down every
    // nesting level; it falls back to `location.origin` at the top document
    // (no parent portal, so `window.tonk` is absent).
    let origin = tonk_host::bridge::context_origin().unwrap_or_default();
    // A guest's own `window.location` is not where the page is: a sealed
    // one reads `about:srcdoc`, and one on a site origin reads that site's
    // document. The page's location is the top document's, which every host
    // hands down: this host takes it from its own context when it is itself
    // a guest, and from `window.location` when it is the page. The guest
    // stamps it on its requests (the worker routes by it), and a control
    // that reads the location (e.g. `<page-mount>`, which couriers an
    // invite's `?access` + `#seed` into the join command) sees the real one.
    let (path, search, hash) = match context_field("path") {
        Some(path) => (
            path,
            context_field("search").unwrap_or_default(),
            context_field("hash").unwrap_or_default(),
        ),
        None => (
            location
                .as_ref()
                .and_then(|l| l.pathname().ok())
                .unwrap_or_default(),
            location
                .as_ref()
                .and_then(|l| l.search().ok())
                .unwrap_or_default(),
            location
                .as_ref()
                .and_then(|l| l.hash().ok())
                .unwrap_or_default(),
        ),
    };
    let (repo, branch, with) = state
        .borrow()
        .with
        .as_ref()
        .map(|with| {
            (
                with.space().unwrap_or_default().to_owned(),
                with.effective_branch().to_owned(),
                with.to_string(),
            )
        })
        .unwrap_or_default();
    // The host's per-tab `site` entity (`site:<uuid>`). The guest's data queries
    // are ultimately issued by the installed host over HTTP, which stamps
    // THIS site on `X-Tonk-Site` — so the SW keys this tab's `tonk:site` facts by
    // it, not by the guest's own `guest:…` id. Guest content that renders the
    // routing indirection binds `entity` to this so it resolves the facts the SW
    // actually stamped.
    //
    // A routed portal is hosted by a `<tonk-site>`, which names its own site
    // (`data-site`, the entity its route is stamped on, on the branch it
    // shows). That is the guest's site: what its page reports (its
    // selection) and what the palette interprets against.
    let site = host
        .closest("[data-site]")
        .ok()
        .flatten()
        .and_then(|site| site.get_attribute("data-site"))
        .filter(|site| !site.is_empty())
        .unwrap_or_else(tonk_host::bridge::site_id);
    let _ = Reflect::set(&context, &"this".into(), &JsValue::from_str(&this));
    let _ = Reflect::set(&context, &"model".into(), &JsValue::from_str(&model));
    let _ = Reflect::set(&context, &"origin".into(), &JsValue::from_str(&origin));
    // The per-space SYNTHETIC origin this guest believes it lives at
    // (`https://{label}.tonk.network/`), so in-guest navigation resolves like an
    // ordinary page: in-space routes are plain absolute paths under it, and an
    // href that escapes it is external. Distinct from `origin` (the REAL host
    // origin, which propagates down nesting and keys the `/api` relay strip).
    // Absent for the profile/Hub (no space) — those links are genuinely
    // top-level and want the real origin.
    // A space rendered at its real origin resolves against that origin
    // instead: there is no illusion left to maintain.
    let base = match state.borrow().origin() {
        Some(origin) => format!("{origin}/"),
        None => tonk_host::space_origin::space_origin_for(&repo).unwrap_or_default(),
    };
    let _ = Reflect::set(&context, &"base".into(), &JsValue::from_str(&base));
    if let Some(site_pattern) = state.borrow().site_pattern.as_deref() {
        let _ = Reflect::set(
            &context,
            &"sitePattern".into(),
            &JsValue::from_str(site_pattern),
        );
    }
    // A `<tonk-site>`'s own site entity and in-site path, which a guest on a
    // real origin claims `tonk:load` for against its own worker. Distinct from
    // `site` above, the tab's site the service worker assigned.
    if let Some(site_entity) = host.get_attribute("data-site") {
        let _ = Reflect::set(
            &context,
            &"siteEntity".into(),
            &JsValue::from_str(&site_entity),
        );
    }
    if let Some(site_path) = host.get_attribute("path") {
        let _ = Reflect::set(&context, &"sitePath".into(), &JsValue::from_str(&site_path));
    }
    let _ = Reflect::set(&context, &"path".into(), &JsValue::from_str(&path));
    let _ = Reflect::set(&context, &"search".into(), &JsValue::from_str(&search));
    let _ = Reflect::set(&context, &"hash".into(), &JsValue::from_str(&hash));
    // Only the product-owned home site opts in. Nested content portals inherit
    // its space identity; a routed non-home site explicitly clears it.
    let preview = if host.has_attribute("data-space-preview") {
        let path = host.get_attribute("path").unwrap_or_default();
        if path.is_empty() || path == "/" {
            repo.clone()
        } else {
            String::new()
        }
    } else {
        context_field("preview")
            .filter(|space| repo.is_empty() || space == &repo)
            .unwrap_or_default()
    };
    let _ = Reflect::set(&context, &"preview".into(), &JsValue::from_str(&preview));
    let _ = Reflect::set(&context, &"repo".into(), &JsValue::from_str(&repo));
    let _ = Reflect::set(&context, &"branch".into(), &JsValue::from_str(&branch));
    // The pinned context as one `branch@repo` location: the guest host's
    // fallback route for consumers with no `with` of their own.
    let _ = Reflect::set(&context, &"with".into(), &JsValue::from_str(&with));
    let _ = Reflect::set(&context, &"site".into(), &JsValue::from_str(&site));
    context
}

// --- Small helpers -------------------------------------------------

fn get_str(obj: &JsValue, key: &str) -> Option<String> {
    Reflect::get(obj, &key.into())
        .ok()
        .and_then(|v| v.as_string())
}

fn read_first_port(event: &MessageEvent) -> Option<MessagePort> {
    let ports = Reflect::get(event, &"ports".into()).ok()?;
    let ports: js_sys::Array = ports.dyn_into().ok()?;
    ports.get(0).dyn_into::<MessagePort>().ok()
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
mod runtime_bootstrap_tests {
    use super::RUNTIME_BOOTSTRAP_JS;

    /// A nested guest fills its parent's whole viewport, so once content
    /// renders in one, no click reaches the frame the FABB lives in and its
    /// open stack cannot be dismissed by clicking away. The guest reports
    /// the press upward and every ancestor redispatches it, so the dismiss
    /// listeners already on those documents fire.
    #[test]
    fn a_press_in_a_guest_reaches_every_ancestor() {
        assert!(RUNTIME_BOOTSTRAP_JS.contains(r#"__tonkRuntime:"press""#));
        // Capture phase: content that stops propagation must not also
        // stop an ancestor's overlay from closing.
        assert!(RUNTIME_BOOTSTRAP_JS.contains(r#"document.addEventListener("pointerdown""#));
        // Relayed onward, so the press climbs past the first ancestor.
        assert!(
            RUNTIME_BOOTSTRAP_JS
                .contains(r#"document.dispatchEvent(new PointerEvent("pointerdown""#)
        );
    }

    /// Only the FACT of the press travels. Coordinates or a target would
    /// let an ancestor observe what was pressed inside a sealed guest.
    #[test]
    fn the_relayed_press_carries_nothing_about_what_was_pressed() {
        let at = RUNTIME_BOOTSTRAP_JS
            .find(r#"parent.postMessage({__tonkRuntime:"press"}"#)
            .expect("the relay");
        let message = &RUNTIME_BOOTSTRAP_JS[at..at + 60];
        for leak in ["clientX", "clientY", "target", "path"] {
            assert!(
                !message.contains(leak),
                "press relay leaks {leak}: {message}"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use js_sys::{Array, Function, Promise};
    use wasm_bindgen_futures::JsFuture;
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    use web_sys::{Document, HtmlDialogElement, HtmlElement, MessageChannel};

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

    /// The guest resolves every link against its synthetic per-space
    /// origin, so a link to a PROFILE route (`/join`) arrives looking
    /// exactly like an in-space path. Rewriting it to `/space/{did}/join`
    /// names a route no space defines, and the app then tries to boot
    /// inside the sealed frame — opaque origin, no service worker, every
    /// asset CORS-blocked, renderer crash. These must pass through.
    #[dialog_common::test]
    fn it_treats_profile_routes_as_top_level() {
        assert!(is_top_level_route("join"));
        assert!(is_top_level_route("account"));
        assert!(is_top_level_route("inspector"));
        assert!(is_top_level_route("diagnose"));
        // A query or sub-path does not disguise the route.
        assert!(is_top_level_route("join?x=1"));
        assert!(is_top_level_route("account/devices"));
        assert!(is_top_level_route("diagnose/abc123"));
    }

    /// Everything else is genuinely in-space and still gets the space
    /// prefix — the behaviour the pass-through must not swallow.
    #[dialog_common::test]
    fn it_leaves_in_space_paths_to_the_space_prefix() {
        assert!(!is_top_level_route("activity"));
        assert!(!is_top_level_route("board"));
        assert!(!is_top_level_route(""));
        // A path that merely STARTS with a top-level name is not one.
        assert!(!is_top_level_route("joinery"));
        assert!(!is_top_level_route("accounts-payable"));
    }

    // --- find_font_paths: url() argument extraction --------------------

    #[dialog_common::test]
    fn it_finds_font_paths_in_url_args_only() {
        // The comment reproduces styles.css prose that the old raw-substring
        // scan turned into `GET /fonts/%60%20(copied…` — a 404 on every
        // guest boot. Prose mentions of `/fonts/` must not count; quoted,
        // single-quoted, and bare url() arguments must; duplicates collapse.
        let css = r#"
            /* Files live under `/fonts/` (copied into the dist by Trunk's
               `copy-dir` on `./assets/fonts`). */
            @font-face { src: url("/fonts/space-grotesk-400.woff2") format("woff2"); }
            @font-face { src: url('/fonts/gestalte-400.otf'); }
            .bare { mask: url(/fonts/unquoted.ttf); }
            .other { background: url("/images/mark-white.svg"); }
            .dup { src: url("/fonts/space-grotesk-400.woff2"); }
            .empty { background: url("/fonts/"); }
        "#;
        assert_eq!(
            find_font_paths(css),
            vec![
                "/fonts/space-grotesk-400.woff2",
                "/fonts/gestalte-400.otf",
                "/fonts/unquoted.ttf",
            ]
        );
    }

    // --- data_plane_location: what a relayed fetch reaches -------------

    /// A portal state pinned to `with` and granting `allow`.
    fn routed_state(with: Option<&str>, allow: &str) -> Rc<RefCell<PortalState>> {
        let state = Rc::new(RefCell::new(PortalState::new()));
        state.borrow_mut().set_route(
            with.map(|w| w.parse().expect("with parses")),
            allow.parse().expect("allow parses"),
        );
        state
    }

    #[dialog_common::test]
    fn it_classifies_repository_metadata_under_the_portal_reach() {
        let named = routed_state(Some("main@did:key:zSpace"), "main@did:key:zSpace");
        assert_eq!(
            data_plane_location("/api/repository/did:key:zSpace", &named),
            Some("main@did:key:zSpace".parse().unwrap()),
        );

        let profile = routed_state(Some("main@profile:tonk"), "main@profile:tonk");
        assert_eq!(
            data_plane_location("/api/repository/profile:tonk", &profile),
            Some("main@profile:tonk".parse().unwrap()),
        );
    }

    #[dialog_common::test]
    fn it_does_not_misclassify_repository_control_routes() {
        let state = routed_state(Some("main@did:key:zSpace"), "main@did:key:zSpace");
        assert_eq!(
            data_plane_location("/api/repository/did:key:zSpace/remote", &state),
            None,
        );
        assert_eq!(
            data_plane_location("/api/repository/did:key:zSpace/invite", &state),
            None,
        );
    }

    // --- FakeHost: where the tests mount portals ---------------------

    /// A container in the document that consumers and portals attach
    /// under.
    struct FakeHost {
        container: Element,
    }

    impl FakeHost {
        fn install() -> FakeHost {
            let container = document().create_element("div").expect("div");
            document()
                .body()
                .expect("body")
                .append_child(&container)
                .expect("attach container");
            FakeHost { container }
        }
    }

    /// A consumer element that dispatches the bridge's events: a `<div>`
    /// under the fake host carrying the scoped `entity` / `model`.
    fn relay_consumer(host: &FakeHost, entity: Option<&str>, model: Option<&str>) -> Element {
        let consumer = document().create_element("div").expect("div");
        if let Some(e) = entity {
            consumer.set_attribute("entity", e).expect("entity");
        }
        if let Some(m) = model {
            consumer.set_attribute("model", m).expect("model");
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

        /// Await the first message of any shape (for raw envelopes that
        /// carry no `type` field, e.g. a bare transferred-body probe).
        async fn wait_for_any(&self) -> JsValue {
            for _ in 0..200 {
                let found = self.messages.borrow().first().cloned();
                if let Some(found) = found {
                    return found;
                }
                sleep(5).await;
            }
            JsValue::UNDEFINED
        }

        /// How many messages have arrived so far.
        fn count(&self) -> usize {
            self.messages.borrow().len()
        }

        /// Drop collected messages so `wait_for_any` returns the next one.
        fn clear(&self) {
            self.messages.borrow_mut().clear();
        }
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

    // --- Handshake tests ---------------------------------------------

    #[dialog_common::test]
    async fn it_posts_ready_with_context_on_bind() {
        let host = FakeHost::install();
        let consumer = relay_consumer(&host, Some("id:demo-counter"), Some("counter"));
        let state = Rc::new(RefCell::new(PortalState::new()));
        let (listener, _port) = bind(&consumer, &state);

        let ready = listener.wait_for("ready").await;
        let context = Reflect::get(&ready, &"context".into()).expect("context");
        assert_eq!(
            get_str(&context, "this").as_deref(),
            Some("id:demo-counter")
        );
        assert_eq!(get_str(&context, "model").as_deref(), Some("counter"));
        // The host forwards its real `search` (the `?query`) into the guest
        // context — a sealed guest can't read it off its own `about:srcdoc`
        // location, and `<page-mount>` needs it to courier an invite's `?access`.
        assert!(
            get_str(&context, "search").is_some(),
            "context carries a `search` field forwarded from the host location",
        );
    }

    #[dialog_common::test]
    async fn it_refreshes_location_in_a_reused_guest() {
        let host = FakeHost::install();
        let consumer = relay_consumer(&host, None, None);
        let state = Rc::new(RefCell::new(PortalState::new()));
        let (listener, _port) = bind(&consumer, &state);
        listener.wait_for("ready").await;
        let win = window().unwrap();
        let original = win.location().href().unwrap();
        win.history()
            .unwrap()
            .push_state_with_url(
                &JsValue::NULL,
                "",
                Some("/settings?delete-space=did%3Akey%3AzOwned#delete-account"),
            )
            .unwrap();
        refresh_context(&consumer, &state);
        let update = listener.wait_for("context").await;
        let context = Reflect::get(&update, &"context".into()).unwrap();
        win.history()
            .unwrap()
            .replace_state_with_url(&JsValue::NULL, "", Some(&original))
            .unwrap();
        assert_eq!(get_str(&context, "path").as_deref(), Some("/settings"));
        assert_eq!(
            get_str(&context, "search").as_deref(),
            Some("?delete-space=did%3Akey%3AzOwned")
        );
        assert_eq!(
            get_str(&context, "hash").as_deref(),
            Some("#delete-account")
        );
    }

    /// When THIS portal is itself a nested guest, its host document is
    /// `about:srcdoc` and `location.origin` is `"null"`; the real origin lives
    /// in the parent-forwarded `window.tonk.context.origin`. The ready envelope
    /// must carry that forwarded origin (not `"null"`), so a further-nested
    /// guest can build a same-origin invite link. Simulated by installing a
    /// `window.tonk.context.origin` before bind.
    #[dialog_common::test]
    async fn it_forwards_the_parent_context_origin() {
        let win = window().expect("window");
        let tonk = Object::new();
        let ctx = Object::new();
        let _ = Reflect::set(&ctx, &"origin".into(), &"https://forwarded.test".into());
        let _ = Reflect::set(&ctx, &"path".into(), &"/space/x/notes".into());
        let _ = Reflect::set(&ctx, &"search".into(), &"?access=1".into());
        let _ = Reflect::set(&tonk, &"context".into(), &ctx);
        let _ = Reflect::set(&win, &"tonk".into(), &tonk);

        let host = FakeHost::install();
        let consumer = relay_consumer(&host, Some("id:demo-counter"), Some("counter"));
        let state = Rc::new(RefCell::new(PortalState::new()));
        let (listener, _port) = bind(&consumer, &state);

        let ready = listener.wait_for("ready").await;
        let context = Reflect::get(&ready, &"context".into()).expect("context");

        // Restore before asserting so a failure doesn't leak `window.tonk`
        // into a later test running in the same page.
        let _ = Reflect::set(&win, &"tonk".into(), &JsValue::UNDEFINED);

        assert_eq!(
            get_str(&context, "origin").as_deref(),
            Some("https://forwarded.test"),
            "a nested portal forwards the parent context origin, not `about:srcdoc`'s null",
        );
        assert_eq!(
            get_str(&context, "path").as_deref(),
            Some("/space/x/notes"),
            "a nested portal forwards the page's path, not its own document's",
        );
        assert_eq!(get_str(&context, "search").as_deref(), Some("?access=1"));
        assert_eq!(get_str(&context, "hash").as_deref(), Some(""));
    }

    // --- End-to-end smoke tests --------------------------------------

    /// Mount a real `<tonk-portal>` (opaque-origin iframe) under the
    /// fake host with the given attributes.
    fn mount_portal(
        host: &FakeHost,
        content: &str,
        entity: Option<&str>,
        model: Option<&str>,
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
    async fn it_presents_and_reseats_a_typed_task_across_a_real_opaque_portal() {
        let host = FakeHost::install();
        let probe = WindowProbe::install("task");
        let standing = Rc::new(RefCell::new(None::<HtmlDialogElement>));
        let reply = Rc::new(RefCell::new(None::<ContainedTaskReturn>));
        let latest = Rc::new(RefCell::new(None::<crate::task::Request>));
        let standing_for_handler = standing.clone();
        let reply_for_handler = reply.clone();
        let latest_for_handler = latest.clone();
        on_task(move |request, focus_return| {
            match request.action {
                crate::task::Action::Open => {
                    let dialog = document()
                        .create_element("dialog")
                        .expect("dialog")
                        .dyn_into::<HtmlDialogElement>()
                        .expect("native dialog");
                    dialog.set_text_content(Some("trusted task probe"));
                    document()
                        .body()
                        .expect("body")
                        .append_child(&dialog)
                        .expect("mount dialog");
                    seat_probe(&dialog, &request);
                    dialog.show_modal().expect("show modal");
                    *standing_for_handler.borrow_mut() = Some(dialog);
                    *reply_for_handler.borrow_mut() = focus_return;
                }
                crate::task::Action::Reseat => {
                    if let Some(dialog) = standing_for_handler.borrow().as_ref() {
                        seat_probe(dialog, &request);
                    }
                }
                _ => {}
            }
            *latest_for_handler.borrow_mut() = Some(request);
        });

        let open = task_payload(crate::task::Action::Open, 10.0, 12.0);
        let reseat = task_payload(crate::task::Action::Reseat, 30.0, 36.0);
        let content = format!(
            r#"<button id="opener">open task</button><main id="surface">space</main><script>
            var opener=document.getElementById('opener'),surface=document.getElementById('surface');
            opener.focus();surface.hidden=true;
            window.addEventListener('tonk:task-closed',function(event){{
              var wasHidden=surface.hidden;surface.hidden=false;
              queueMicrotask(function(){{parent.postMessage({{__test:'task',result:event.detail.result,wasHidden:wasHidden,focused:document.activeElement===opener}},'*');}});
            }},{{once:true}});
            tonk.task({open:?});tonk.task({reseat:?});
            </script>"#
        );
        let portal = mount_portal(&host, &content, None, None);
        portal
            .set_attribute(
                "style",
                "display:block;margin:29px 0 0 37px;width:320px;height:220px",
            )
            .expect("portal geometry");

        for _ in 0..400 {
            if standing
                .borrow()
                .as_ref()
                .is_some_and(HtmlDialogElement::open)
                && latest
                    .borrow()
                    .as_ref()
                    .is_some_and(|request| request.action == crate::task::Action::Reseat)
            {
                break;
            }
            sleep(5).await;
        }
        let dialog = standing.borrow().clone().expect("standing top-page modal");
        assert!(dialog.open(), "the native modal blocks the trusted page");
        assert!(
            dialog.matches(":modal").expect(":modal selector"),
            "the request is modal in the top page rather than only the guest"
        );
        let iframe = portal
            .query_selector("iframe")
            .expect("iframe selector")
            .expect("portal iframe")
            .dyn_into::<HtmlElement>()
            .expect("HTML iframe");
        let frame = iframe.get_bounding_client_rect();
        let latest_request = latest.borrow().clone().expect("reseat request");
        let anchor = &latest_request.presentation.as_ref().unwrap().anchor;
        assert!((anchor.left - (frame.left() + 30.0)).abs() < 0.5);
        assert!((anchor.top - (frame.top() + 36.0)).abs() < 0.5);
        assert_eq!(
            latest_request.presentation.as_ref().unwrap().horizontal,
            crate::task::Horizontal::Right
        );
        assert_eq!(
            latest_request.presentation.as_ref().unwrap().vertical,
            crate::task::Vertical::Bottom
        );
        assert!((dialog.get_bounding_client_rect().right() - anchor.right).abs() < 0.5);
        assert!((dialog.get_bounding_client_rect().bottom() - anchor.bottom).abs() < 0.5);

        reply
            .borrow_mut()
            .take()
            .expect("task return")
            .finish("completed");
        let message = probe.wait().await;
        assert_eq!(get_str(&message, "result").as_deref(), Some("completed"));
        assert_eq!(
            Reflect::get(&message, &"wasHidden".into())
                .ok()
                .and_then(|value| value.as_bool()),
            Some(true),
            "only the trusted task surface is visible while it is open"
        );
        assert_eq!(
            Reflect::get(&message, &"focused".into())
                .ok()
                .and_then(|value| value.as_bool()),
            Some(true),
            "focus returns through the existing port token"
        );
        dialog.close();
        dialog.remove();
        portal.remove();
    }

    #[dialog_common::test]
    async fn it_dismisses_the_trusted_task_when_its_guest_disconnects() {
        let host = FakeHost::install();
        let standing = Rc::new(RefCell::new(None::<HtmlDialogElement>));
        let held_return = Rc::new(RefCell::new(None::<ContainedTaskReturn>));
        let dismissed = Rc::new(std::cell::Cell::new(false));
        let standing_for_handler = standing.clone();
        let held_return_for_handler = held_return.clone();
        let dismissed_for_handler = dismissed.clone();
        on_task(move |request, focus_return| match request.action {
            crate::task::Action::Open => {
                let dialog = document()
                    .create_element("dialog")
                    .expect("dialog")
                    .dyn_into::<HtmlDialogElement>()
                    .expect("native dialog");
                document()
                    .body()
                    .expect("body")
                    .append_child(&dialog)
                    .expect("mount dialog");
                dialog.show_modal().expect("show modal");
                *standing_for_handler.borrow_mut() = Some(dialog);
                *held_return_for_handler.borrow_mut() = focus_return;
            }
            crate::task::Action::Dismiss => {
                if let Some(dialog) = standing_for_handler.borrow_mut().take() {
                    dialog.close();
                    dialog.remove();
                }
                held_return_for_handler.borrow_mut().take();
                dismissed_for_handler.set(true);
            }
            _ => {}
        });

        let open = task_payload(crate::task::Action::Open, 10.0, 12.0);
        let portal = mount_portal(
            &host,
            &format!("<script>tonk.task({open:?})</script>"),
            None,
            None,
        );
        for _ in 0..400 {
            if standing
                .borrow()
                .as_ref()
                .is_some_and(HtmlDialogElement::open)
            {
                break;
            }
            sleep(5).await;
        }
        assert!(standing.borrow().is_some(), "trusted task opened");

        portal.remove();
        for _ in 0..100 {
            if dismissed.get() {
                break;
            }
            sleep(5).await;
        }
        assert!(
            dismissed.get(),
            "guest teardown dispatches one typed dismissal"
        );
        assert!(standing.borrow().is_none(), "trusted modal is released");
    }

    #[dialog_common::test]
    async fn it_ignores_a_hello_from_an_unregistered_source() {
        // A registered portal whose iframe never speaks: the registry is
        // non-empty, but only its live `contentWindow` may complete a
        // handshake.
        install_message_listener();
        let host = FakeHost::install();
        let consumer = relay_consumer(&host, None, None);
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
        mount_portal(&host, &report("a"), Some("id:alpha"), Some("counter"));
        mount_portal(&host, &report("b"), Some("id:beta"), Some("counter"));

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

    /// The credit-based fallback (`drain_body_to_port`, used when a browser
    /// can't transfer a `ReadableStream`) drains a response body into a
    /// `MessageChannel`: it hands over a `streamPort`, then posts `chunk`
    /// messages only as the consumer grants credit, and `close` at EOF. This
    /// drives that protocol by hand (standing in for the guest's
    /// ReadableStream) and asserts the bytes reassemble AND that no chunk
    /// arrives before credit is granted (backpressure holds).
    #[dialog_common::test]
    async fn it_drains_a_body_to_a_port_with_credit_backpressure() {
        use web_sys::{Response, ResponseInit};

        // A body that yields a few chunks. A Response from a string gives one
        // chunk; that's enough to exercise the credit gate + close.
        let init = ResponseInit::new();
        let resp = Response::new_with_opt_str_and_init(Some("sigil-bytes"), &init)
            .expect("construct response");
        let body = resp.body().expect("response body");

        // The "guest" side: the head envelope is posted to `client`, carrying
        // the transferred stream port.
        let head_channel = MessageChannel::new().expect("head channel");
        let host_to_guest = head_channel.port1();
        let guest_in = head_channel.port2();
        let head_listener = PortListener::attach(&guest_in);

        let head = Object::new();
        set_v1(&head, "fetch-result");
        let _ = Reflect::set(&head, &"id".into(), &JsValue::from_str("r1"));
        drain_body_to_port(&host_to_guest, head, &body);

        // Receive the head + the stream port.
        let received = head_listener.wait_for("fetch-result").await;
        let stream_port: MessagePort = Reflect::get(&received, &"streamPort".into())
            .expect("streamPort")
            .dyn_into()
            .expect("a MessagePort");
        let chunk_listener = PortListener::attach(&stream_port);

        // Backpressure: before granting credit, no chunk must arrive.
        sleep(30).await;
        assert!(
            chunk_listener.count() == 0,
            "no chunk may be sent before credit is granted",
        );

        // Grant credit and collect chunks until `close`.
        let mut collected: Vec<u8> = Vec::new();
        let mut closed = false;
        for _ in 0..50 {
            let grant = Object::new();
            let _ = Reflect::set(&grant, &"type".into(), &"credit".into());
            let _ = Reflect::set(&grant, &"n".into(), &JsValue::from_f64(1.0));
            stream_port.post_message(&grant).expect("grant credit");

            let msg = chunk_listener.wait_for_any().await;
            chunk_listener.clear();
            match get_str(&msg, "type").as_deref() {
                Some("chunk") => {
                    // Reconstruct the view over exactly the chunk's window, the
                    // way the guest does — the buffer is transferred whole but
                    // the bytes live in [byteOffset, byteOffset+byteLength).
                    let chunk = Reflect::get(&msg, &"chunk".into()).expect("chunk");
                    let offset = Reflect::get(&msg, &"byteOffset".into())
                        .ok()
                        .and_then(|v| v.as_f64())
                        .unwrap_or(0.0) as u32;
                    let length = Reflect::get(&msg, &"byteLength".into())
                        .ok()
                        .and_then(|v| v.as_f64())
                        .map(|n| n as u32)
                        .unwrap_or_else(|| js_sys::ArrayBuffer::from(chunk.clone()).byte_length());
                    let bytes =
                        js_sys::Uint8Array::new_with_byte_offset_and_length(&chunk, offset, length)
                            .to_vec();
                    collected.extend_from_slice(&bytes);
                }
                Some("close") => {
                    closed = true;
                    break;
                }
                other => panic!("unexpected stream message: {other:?}"),
            }
        }
        assert!(closed, "the stream must close after the body is drained");
        assert_eq!(
            String::from_utf8(collected).unwrap(),
            "sigil-bytes",
            "the drained bytes must reassemble to the body",
        );
    }

    /// The relay forwards the FULL request (method, headers, body), not just a
    /// GET path, so POST query/subscribe/transact route through it. This
    /// verifies `build_relayed_request` reconstructs each from the envelope.
    #[dialog_common::test]
    async fn it_builds_a_relayed_request_with_method_headers_body() {
        // Envelope shaped like what the guest posts: method + [[name,value]]
        // header pairs + a string body.
        let data = Object::new();
        let _ = Reflect::set(&data, &"method".into(), &"POST".into());
        let headers = Array::new();
        let pair = Array::new();
        pair.push(&"content-type".into());
        pair.push(&"application/json".into());
        headers.push(&pair);
        let _ = Reflect::set(&data, &"headers".into(), &headers);
        let _ = Reflect::set(&data, &"body".into(), &"{\"q\":1}".into());

        // `build_relayed_request` reconstructs the `RequestInit`; the path is
        // fetched separately as a bare string (see `handle_host_fetch`).
        // Materialize a real `Request` from the init to read the fields back.
        let init = build_relayed_request(&data).expect("request");
        let request =
            web_sys::Request::new_with_str_and_init("/api/repository/x/branch/main/query", &init)
                .expect("request from init");

        assert_eq!(request.method(), "POST");
        assert!(
            request
                .url()
                .ends_with("/api/repository/x/branch/main/query"),
            "url: {}",
            request.url(),
        );
        assert_eq!(
            request
                .headers()
                .get("content-type")
                .ok()
                .flatten()
                .as_deref(),
            Some("application/json"),
        );
        let body = JsFuture::from(request.text().expect("text()"))
            .await
            .expect("await body");
        assert_eq!(body.as_string().as_deref(), Some("{\"q\":1}"));
    }

    /// A relayed response must carry the URL the host fetched.
    ///
    /// The guest rebuilds the `Response` from the envelope, and
    /// `new Response(...)` cannot set `url` — it reads back `""` unless the
    /// shim restores it. An empty `url` is not cosmetic: reqwest's wasm
    /// client parses it while converting every response and throws
    /// `url parse`, so a Rust component fetching from inside a sealed guest
    /// (`<tonk-default-remote>` reading `/.well-known/tonk`) dies mid-await
    /// with the request already served.
    #[dialog_common::test]
    async fn it_gives_the_guest_a_response_carrying_the_fetched_url() {
        let host = FakeHost::install();
        let probe = WindowProbe::install("u");

        // Author code at the opaque origin fetches through the relayed
        // `window.fetch` and reports what `url` the response carries. The
        // path need not exist — a 404 is still a Response with a URL.
        let content = "<script>\
            fetch('/.well-known/tonk')\
              .then(function(r){parent.postMessage({__test:'u',url:r.url},'*');})\
              .catch(function(err){parent.postMessage({__test:'u',error:String(err)},'*');});\
            </script>";
        mount_portal(&host, content, None, None);

        let msg = probe.wait().await;
        assert!(
            !msg.is_undefined(),
            "author iframe should post the relayed response back",
        );
        assert!(
            Reflect::get(&msg, &"error".into())
                .ok()
                .filter(|v| !v.is_undefined())
                .is_none(),
            "the relayed fetch should not error; got: {:?}",
            Reflect::get(&msg, &"error".into()).ok(),
        );
        let url = get_str(&msg, "url").unwrap_or_default();
        assert!(
            url.ends_with("/.well-known/tonk"),
            "the rebuilt response must report the fetched URL, got {url:?}",
        );
    }

    fn title_message(kind: &str, text: &str) -> JsValue {
        let object = js_sys::Object::new();
        let _ = Reflect::set(
            &object,
            &JsValue::from_str("type"),
            &JsValue::from_str(kind),
        );
        let _ = Reflect::set(
            &object,
            &JsValue::from_str("text"),
            &JsValue::from_str(text),
        );
        object.into()
    }

    /// `title_text` accepts only a `{ type: "title", text }` shape with
    /// non-empty text; everything else yields `None`, so an unrelated
    /// message never retitles the tab and an unresolved `{name}` never
    /// blanks it. We assert the parse, not the assignment — performing
    /// it would retitle the test harness itself.
    #[dialog_common::test]
    async fn it_reads_text_only_from_a_title_message() {
        assert_eq!(
            title_text(&title_message("title", "Notes — Tonk")),
            Some("Notes — Tonk".to_owned()),
            "a title message with text should yield it"
        );
        assert_eq!(
            title_text(&title_message("title", "")),
            None,
            "an empty text should yield None"
        );
        assert_eq!(
            title_text(&title_message("other", "Notes — Tonk")),
            None,
            "a non-title message should yield None"
        );
        assert_eq!(
            title_text(&JsValue::from_str("not an object")),
            None,
            "a non-object payload should yield None"
        );
    }

    #[dialog_common::test]
    async fn it_parses_only_non_empty_registration_focus_tokens() {
        let message = Object::new();
        let _ = Reflect::set(&message, &"type".into(), &"register".into());
        let _ = Reflect::set(&message, &"reason".into(), &"needs-account".into());
        let _ = Reflect::set(&message, &"focusToken".into(), &"focus-1".into());
        assert_eq!(
            register_request(&message.clone().into()),
            Some(("needs-account".into(), Some("focus-1".into())))
        );

        let _ = Reflect::set(&message, &"focusToken".into(), &"".into());
        assert_eq!(
            register_request(&message.clone().into()),
            Some(("needs-account".into(), None)),
            "an empty token must never create a guest focus handle"
        );
        let _ = Reflect::set(&message, &"reason".into(), &"".into());
        assert_eq!(register_request(&message.into()), None);
    }

    #[dialog_common::test]
    async fn it_returns_registration_focus_through_the_request_port() {
        let state = Rc::new(RefCell::new(PortalState::new()));
        let channel = MessageChannel::new().expect("message channel");
        let listener = PortListener::attach(&channel.port2());
        let held = Rc::new(RefCell::new(None));
        let captured = held.clone();
        on_register(move |reason, focus_return| {
            assert_eq!(reason, "needs-account");
            *captured.borrow_mut() = focus_return;
        });

        let request = Object::new();
        let _ = Reflect::set(&request, &"type".into(), &"register".into());
        let _ = Reflect::set(&request, &"reason".into(), &"needs-account".into());
        let _ = Reflect::set(&request, &"focusToken".into(), &"focus-2".into());
        handle_register(&state, &channel.port1(), &request.into());
        held.borrow_mut()
            .take()
            .expect("focus return handle")
            .restore();

        let returned = listener.wait_for("register-focus").await;
        assert_eq!(get_str(&returned, "focusToken").as_deref(), Some("focus-2"));
    }

    fn task_payload(action: crate::task::Action, left: f64, top: f64) -> String {
        crate::task::Request {
            version: crate::task::VERSION,
            request_id: "probe-1".into(),
            purpose: crate::task::Purpose::Probe,
            action,
            account: None,
            presentation: Some(crate::task::Presentation {
                anchor: crate::task::Anchor {
                    left,
                    top,
                    right: left + 120.0,
                    bottom: top + 48.0,
                    width: 120.0,
                    height: 48.0,
                },
                horizontal: crate::task::Horizontal::Right,
                vertical: crate::task::Vertical::Bottom,
                dismissal: crate::task::Dismissal::Optional,
            }),
        }
        .to_json()
        .expect("task JSON")
    }

    fn seat_probe(dialog: &HtmlDialogElement, request: &crate::task::Request) {
        let presentation = request.presentation.as_ref().expect("presentation");
        let anchor = &presentation.anchor;
        let width = 160.0;
        let height = 96.0;
        let left = match presentation.horizontal {
            crate::task::Horizontal::Left => anchor.left,
            crate::task::Horizontal::Right => anchor.right - width,
        };
        let top = match presentation.vertical {
            crate::task::Vertical::Top => anchor.top,
            crate::task::Vertical::Bottom => anchor.bottom - height,
        };
        let _ = dialog.style().set_property("margin", "0");
        let _ = dialog.style().set_property("box-sizing", "border-box");
        let _ = dialog.style().set_property("border", "0");
        let _ = dialog.style().set_property("padding", "0");
        let _ = dialog.style().set_property("width", &format!("{width}px"));
        let _ = dialog
            .style()
            .set_property("height", &format!("{height}px"));
        let _ = dialog.style().set_property("left", &format!("{left}px"));
        let _ = dialog.style().set_property("top", &format!("{top}px"));
    }

    #[dialog_common::test]
    async fn it_returns_a_contained_task_result_through_the_request_port() {
        let state = Rc::new(RefCell::new(PortalState::new()));
        let channel = MessageChannel::new().expect("message channel");
        let listener = PortListener::attach(&channel.port2());
        let held = Rc::new(RefCell::new(None));
        let captured = held.clone();
        on_task(move |request, focus_return| {
            assert_eq!(request.purpose, crate::task::Purpose::Probe);
            assert_eq!(request.action, crate::task::Action::Open);
            *captured.borrow_mut() = focus_return;
        });

        let request = Object::new();
        let _ = Reflect::set(&request, &"type".into(), &"task".into());
        let _ = Reflect::set(
            &request,
            &"payload".into(),
            &task_payload(crate::task::Action::Open, 10.0, 20.0).into(),
        );
        let _ = Reflect::set(&request, &"focusToken".into(), &"task-focus-1".into());
        handle_task(&state, &channel.port1(), &request.into());
        held.borrow_mut()
            .take()
            .expect("task return handle")
            .finish("completed");

        let returned = listener.wait_for("task-result").await;
        assert_eq!(
            get_str(&returned, "focusToken").as_deref(),
            Some("task-focus-1")
        );
        assert_eq!(get_str(&returned, "result").as_deref(), Some("completed"));
    }

    #[dialog_common::test]
    async fn it_rejects_invalid_task_geometry_before_the_presenter_runs() {
        let state = Rc::new(RefCell::new(PortalState::new()));
        let channel = MessageChannel::new().expect("message channel");
        let listener = PortListener::attach(&channel.port2());
        let handled = Rc::new(std::cell::Cell::new(false));
        let seen = handled.clone();
        on_task(move |_, _| seen.set(true));

        let malformed = task_payload(crate::task::Action::Open, 10.0, 20.0)
            .replace("\"width\":120.0", "\"width\":90.0");
        let request = Object::new();
        let _ = Reflect::set(&request, &"type".into(), &"task".into());
        let _ = Reflect::set(&request, &"payload".into(), &malformed.into());
        let _ = Reflect::set(&request, &"focusToken".into(), &"task-focus-2".into());
        handle_task(&state, &channel.port1(), &request.into());

        let returned = listener.wait_for("task-result").await;
        assert!(
            !handled.get(),
            "invalid metadata never reaches the host presenter"
        );
        assert_eq!(get_str(&returned, "result").as_deref(), Some("invalid"));
    }

    /// A render can replace the guest control that opened registration
    /// before registration is requested, before focus returns, or just
    /// after. Focus follows the replacement.
    #[dialog_common::test]
    async fn it_returns_registration_focus_to_a_replaced_opener() {
        let scenario = Function::new_with_args(
            "bootstrap, replaced",
            r#"return (async () => {
                const frame = document.createElement("iframe");
                document.body.append(frame);
                const port = await new Promise(resolve => {
                    const hello = event => {
                        if (event.source !== frame.contentWindow || event.data?.type !== "hello") return;
                        window.removeEventListener("message", hello);
                        resolve(event.ports[0]);
                    };
                    window.addEventListener("message", hello);
                    const doc = frame.contentDocument;
                    doc.open();
                    doc.write(`<button data-opener="account" data-state="ready">open</button><script>${bootstrap}<\/script>`);
                    doc.close();
                });
                const doc = frame.contentDocument;
                const requested = new Promise(resolve => {
                    port.onmessage = event => {
                        if (event.data?.type === "register") resolve(event.data.focusToken);
                    };
                });
                port.postMessage({ v: 1, type: "ready", context: {} });
                const replace = () => {
                    const old = doc.querySelector("[data-opener]");
                    const next = old.cloneNode(true);
                    next.setAttribute("data-state", "loading");
                    old.replaceWith(next);
                    return next;
                };
                const settle = () => new Promise(resolve => setTimeout(resolve, 20));
                doc.querySelector("[data-opener]").focus();
                let next = replaced === "before-request" ? replace() : null;
                frame.contentWindow.tonk.register("needs-account");
                const token = await requested;
                if (replaced === "before-return") next = replace();
                port.postMessage({ v: 1, type: "register-focus", focusToken: token });
                await settle();
                if (replaced === "after-return") next = replace();
                await settle();
                const focused = doc.activeElement === next;
                frame.remove();
                return focused;
            })();"#,
        );
        for replaced in ["before-request", "before-return", "after-return"] {
            let promise = scenario
                .call2(
                    &JsValue::NULL,
                    &JsValue::from_str(BOOTSTRAP_JS),
                    &JsValue::from_str(replaced),
                )
                .expect("run the scenario");
            let focused = JsFuture::from(Promise::from(promise))
                .await
                .expect("scenario settles");
            assert_eq!(
                focused.as_bool(),
                Some(true),
                "focus did not follow an opener replaced {replaced}"
            );
        }
    }

    #[dialog_common::test]
    async fn it_relays_nested_registration_focus_and_discard_to_the_child_port() {
        for terminal in ["register-focus", "custody-focus", "register-focus-discard"] {
            let channel = MessageChannel::new().expect("message channel");
            let listener = PortListener::attach(&channel.port2());
            let reply = RegisterFocusReturn {
                port: channel.port1(),
                frame: None,
                token: "inner-opener".into(),
                handled: false,
            };
            let outer_register = js_sys::Function::new_with_args(
                "reason, relay",
                &format!(
                    "if(reason!=='needs-account')throw Error('wrong reason');relay('custody-open');relay('{terminal}');"
                ),
            );
            relay_register(
                &outer_register,
                &JsValue::NULL,
                "needs-account",
                Some(reply),
            )
            .unwrap();
            let opened = listener.wait_for("custody-open").await;
            assert_eq!(
                get_str(&opened, "focusToken").as_deref(),
                Some("inner-opener")
            );
            let returned = listener.wait_for(terminal).await;
            assert_eq!(
                get_str(&returned, "focusToken").as_deref(),
                Some("inner-opener")
            );
        }
    }

    #[dialog_common::test]
    async fn it_replaces_and_restores_custody_through_the_request_port() {
        let channel = MessageChannel::new().expect("message channel");
        let listener = PortListener::attach(&channel.port2());
        let reply = RegisterFocusReturn {
            port: channel.port1(),
            frame: None,
            token: "custody-1".into(),
            handled: false,
        };
        reply.show_custody();
        let opened = listener.wait_for("custody-open").await;
        assert_eq!(get_str(&opened, "focusToken").as_deref(), Some("custody-1"));
        reply.restore_custody();
        let closed = listener.wait_for("custody-focus").await;
        assert_eq!(get_str(&closed, "focusToken").as_deref(), Some("custody-1"));
    }

    /// `open_href` accepts only a well-formed `{type:"open", href}`. The
    /// dispatcher has already matched on `type`; re-checking here keeps the
    /// parse independently testable, as `title_text` does.
    #[dialog_common::test]
    async fn it_reads_href_only_from_an_open_message() {
        let message = js_sys::Object::new();
        let _ = js_sys::Reflect::set(
            &message,
            &JsValue::from_str("type"),
            &JsValue::from_str("open"),
        );
        let _ = js_sys::Reflect::set(
            &message,
            &JsValue::from_str("href"),
            &JsValue::from_str("https://example.com/"),
        );
        assert_eq!(
            open_href(&message.into()),
            Some("https://example.com/".to_owned()),
            "an open message with an href should yield it"
        );

        let empty = js_sys::Object::new();
        let _ = js_sys::Reflect::set(
            &empty,
            &JsValue::from_str("type"),
            &JsValue::from_str("open"),
        );
        let _ = js_sys::Reflect::set(&empty, &JsValue::from_str("href"), &JsValue::from_str(""));
        assert_eq!(
            open_href(&empty.into()),
            None,
            "an empty href should yield None"
        );

        let other = js_sys::Object::new();
        let _ = js_sys::Reflect::set(
            &other,
            &JsValue::from_str("type"),
            &JsValue::from_str("navigate"),
        );
        let _ = js_sys::Reflect::set(
            &other,
            &JsValue::from_str("href"),
            &JsValue::from_str("https://example.com/"),
        );
        assert_eq!(
            open_href(&other.into()),
            None,
            "a non-open message should yield None"
        );

        assert_eq!(
            open_href(&JsValue::from_str("not an object")),
            None,
            "a non-object payload should yield None"
        );
    }
}
