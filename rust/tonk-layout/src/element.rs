//! `<tonk-layout>` custom-element implementation.
//!
//! A tiling window manager: an infinite horizontal scrollable
//! strip of columns, each column a vertical stack of tiles. The
//! layout itself is persisted to the branch as normalized
//! entities (see `/plan/tonk-layout.md`).
//!
//! This module owns the element lifecycle and the read path: a
//! cascade of three SSE subscriptions (workspace → columns +
//! tiles) whose frames fold into a [`Layout`]. DOM rendering of
//! that layout lands in the next step; for now each fold reflects
//! `data-state` and dispatches a `tonk-layout:layout` event.

use std::cell::RefCell;
use std::rc::Rc;

use custom_elements::CustomElement;
use tonk_concept::error::{ErrorDetail, ErrorKind};
use tonk_concept::sse::open_sse;
use tonk_schema::conclusion::Conclusion;
use wasm_bindgen_futures::spawn_local;
use web_sys::{AbortController, CustomEvent, CustomEventInit, Element, HtmlElement, window};

use crate::model::Layout;
use crate::reconcile::Reconciler;
use crate::resolve::{columns_query, tiles_query, workspace_query};
use crate::state::{self, State};

/// Default workspace name when the `workspace` attribute is absent.
const DEFAULT_WORKSPACE: &str = "default";

/// Internal lifecycle state shared across async closures.
///
/// `custom-elements` requires the element struct itself to be
/// `Default`, and there is no host element until
/// `connected_callback`. So the real state lives here and is only
/// allocated once we are connected.
struct Inner {
    /// Aborts the workspace subscription.
    workspace_abort: Option<AbortController>,
    /// Aborts the columns subscription (opened once the workspace
    /// entity resolves).
    columns_abort: Option<AbortController>,
    /// Aborts the tiles subscription.
    tiles_abort: Option<AbortController>,
    /// Latest workspace row, if its first frame has arrived.
    workspace: Option<Conclusion>,
    /// Latest columns frame.
    columns: Vec<Conclusion>,
    /// Latest tiles frame.
    tiles: Vec<Conclusion>,
    /// Patches the strip DOM to match each folded layout. Created
    /// lazily on the first fold, dropped on `abort`.
    reconciler: Option<Reconciler>,
    /// Monotonic counter. Every spawned flow captures the value
    /// at spawn; a flow whose generation no longer matches has
    /// been superseded by an `attribute_changed_callback` and
    /// bails instead of touching the host.
    generation: u64,
}

impl Inner {
    fn new() -> Self {
        Self {
            workspace_abort: None,
            columns_abort: None,
            tiles_abort: None,
            workspace: None,
            columns: Vec::new(),
            tiles: Vec::new(),
            reconciler: None,
            generation: 0,
        }
    }

    /// Cancel every in-flight subscription and forget the cached
    /// frames. Safe to call when nothing is open.
    fn abort(&mut self) {
        for handle in [
            self.workspace_abort.take(),
            self.columns_abort.take(),
            self.tiles_abort.take(),
        ]
        .into_iter()
        .flatten()
        {
            handle.abort();
        }
        self.workspace = None;
        self.columns.clear();
        self.tiles.clear();
        if let Some(reconciler) = &mut self.reconciler {
            reconciler.clear();
        }
        self.reconciler = None;
    }
}

/// The custom element. Holds no fields directly — see [`Inner`].
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
        state::set(&host, State::Loading);

        let inner = Rc::new(RefCell::new(Inner::new()));
        *self.inner.borrow_mut() = Some(inner.clone());

        start(&host, inner);
    }

    fn disconnected_callback(&mut self, _this: &HtmlElement) {
        if let Some(inner) = self.inner.borrow_mut().take() {
            inner.borrow_mut().abort();
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
        // Cancel the current subscription cascade, bump the
        // generation so any in-flight flow bails, and restart
        // against the new attributes.
        {
            let mut s = inner.borrow_mut();
            s.abort();
            s.generation = s.generation.wrapping_add(1);
        }
        state::set(&host, State::Loading);
        start(&host, inner);
    }
}

/// Resolve the workspace name from the host's `workspace`
/// attribute, falling back to [`DEFAULT_WORKSPACE`].
fn workspace_name(host: &Element) -> String {
    match host.get_attribute("workspace") {
        Some(name) if !name.is_empty() => name,
        _ => DEFAULT_WORKSPACE.to_string(),
    }
}

/// The `space` attribute, defaulting to `"home"`.
fn space(host: &Element) -> String {
    host.get_attribute("space")
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "home".to_string())
}

/// The `branch` attribute, defaulting to `"main"`.
fn branch(host: &Element) -> String {
    host.get_attribute("branch")
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "main".to_string())
}

/// Build the worker `/query` URL from the host's `space` /
/// `branch` attributes.
fn query_url(host: &Element) -> String {
    format!(
        "/api/repository/{}/branch/{}/query",
        space(host),
        branch(host)
    )
}

/// Begin (or restart) the element's read path.
///
/// Opens the workspace subscription; once the workspace entity
/// resolves, [`open_layout_streams`] opens the columns and tiles
/// subscriptions. Each frame triggers a [`refold`].
fn start(host: &Element, inner: Rc<RefCell<Inner>>) {
    let url = query_url(host);
    let name = workspace_name(host);
    let generation = inner.borrow().generation;

    let host = host.clone();
    spawn_local(async move {
        let body = match serde_json::to_string(&workspace_query(&name)) {
            Ok(body) => body,
            Err(error) => {
                fail(&host, ErrorKind::Parse, format!("workspace query: {error}"));
                return;
            }
        };

        let host_for_frame = host.clone();
        let inner_for_frame = inner.clone();
        let url_for_frame = url.clone();
        let host_for_error = host.clone();
        let inner_for_error = inner.clone();

        let on_frame = move |payload: &str| {
            let rows = match parse_frame(payload) {
                Ok(rows) => rows,
                Err(error) => {
                    fail(&host_for_frame, error.kind, error.message);
                    return;
                }
            };
            if !generation_current(&inner_for_frame, generation) {
                return;
            }
            // The workspace query matches at most one row.
            let workspace = rows.into_iter().next();
            let entity = workspace.as_ref().map(|row| row.this.clone());
            inner_for_frame.borrow_mut().workspace = workspace;

            match entity {
                Some(entity) => open_layout_streams(
                    &url_for_frame,
                    &entity,
                    &host_for_frame,
                    &inner_for_frame,
                    generation,
                ),
                // No workspace row yet — nothing to render.
                None => {
                    refold(&host_for_frame, &inner_for_frame);
                }
            }
        };

        let on_error = move |error: ErrorDetail| {
            if generation_current(&inner_for_error, generation) {
                fail(&host_for_error, error.kind, error.message);
            }
        };

        match open_sse(&url, &body, on_frame, on_error).await {
            Ok(abort) => {
                if generation_current(&inner, generation) {
                    inner.borrow_mut().workspace_abort = Some(abort);
                } else {
                    // Superseded while the fetch was in flight.
                    abort.abort();
                }
            }
            Err(error) => {
                if generation_current(&inner, generation) {
                    fail(&host, error.kind, error.message);
                }
            }
        }
    });
}

/// Open the columns and tiles subscriptions for a resolved
/// workspace entity. Replaces any streams from a prior workspace
/// frame so a `workspace` rename re-points cleanly.
fn open_layout_streams(
    url: &str,
    workspace_entity: &str,
    host: &Element,
    inner: &Rc<RefCell<Inner>>,
    generation: u64,
) {
    // Drop streams from a previous workspace entity.
    {
        let mut s = inner.borrow_mut();
        if let Some(abort) = s.columns_abort.take() {
            abort.abort();
        }
        if let Some(abort) = s.tiles_abort.take() {
            abort.abort();
        }
    }

    spawn_stream(
        url,
        workspace_entity,
        host,
        inner,
        generation,
        StreamKind::Columns,
    );
    spawn_stream(
        url,
        workspace_entity,
        host,
        inner,
        generation,
        StreamKind::Tiles,
    );
}

/// Which of the two layout subscriptions a [`spawn_stream`] call
/// drives.
#[derive(Clone, Copy)]
enum StreamKind {
    Columns,
    Tiles,
}

/// Open one layout subscription (columns or tiles), routing each
/// frame into the matching `Inner` field and triggering a
/// [`refold`].
fn spawn_stream(
    url: &str,
    workspace_entity: &str,
    host: &Element,
    inner: &Rc<RefCell<Inner>>,
    generation: u64,
    kind: StreamKind,
) {
    let query = match kind {
        StreamKind::Columns => columns_query(workspace_entity),
        StreamKind::Tiles => tiles_query(workspace_entity),
    };
    let url = url.to_string();
    let host = host.clone();
    let inner = inner.clone();

    spawn_local(async move {
        let body = match serde_json::to_string(&query) {
            Ok(body) => body,
            Err(error) => {
                fail(&host, ErrorKind::Parse, format!("layout query: {error}"));
                return;
            }
        };

        let host_for_frame = host.clone();
        let inner_for_frame = inner.clone();
        let host_for_error = host.clone();
        let inner_for_error = inner.clone();

        let on_frame = move |payload: &str| {
            let rows = match parse_frame(payload) {
                Ok(rows) => rows,
                Err(error) => {
                    fail(&host_for_frame, error.kind, error.message);
                    return;
                }
            };
            if !generation_current(&inner_for_frame, generation) {
                return;
            }
            {
                let mut s = inner_for_frame.borrow_mut();
                match kind {
                    StreamKind::Columns => s.columns = rows,
                    StreamKind::Tiles => s.tiles = rows,
                }
            }
            refold(&host_for_frame, &inner_for_frame);
        };

        let on_error = move |error: ErrorDetail| {
            if generation_current(&inner_for_error, generation) {
                fail(&host_for_error, error.kind, error.message);
            }
        };

        match open_sse(&url, &body, on_frame, on_error).await {
            Ok(abort) => {
                if generation_current(&inner, generation) {
                    let mut s = inner.borrow_mut();
                    match kind {
                        StreamKind::Columns => s.columns_abort = Some(abort),
                        StreamKind::Tiles => s.tiles_abort = Some(abort),
                    }
                } else {
                    abort.abort();
                }
            }
            Err(error) => {
                if generation_current(&inner, generation) {
                    fail(&host, error.kind, error.message);
                }
            }
        }
    });
}

/// Fold the latest cached frames into a [`Layout`], patch the
/// strip DOM to match, reflect the resulting `data-state`, and
/// dispatch `tonk-layout:layout`.
fn refold(host: &Element, inner: &Rc<RefCell<Inner>>) {
    let layout = {
        let s = inner.borrow();
        Layout::fold(s.workspace.as_ref(), &s.columns, &s.tiles)
    };

    {
        let mut s = inner.borrow_mut();
        let reconciler = s
            .reconciler
            .get_or_insert_with(|| Reconciler::new(host.clone(), space(host), branch(host)));
        reconciler.apply(&layout);
    }

    let next = if layout.is_empty() {
        State::Empty
    } else {
        State::Ready
    };
    state::set(host, next);
    dispatch_layout(host, &layout);
}

/// Parsed-frame error wrapper so frame handlers can early-return.
struct FrameError {
    kind: ErrorKind,
    message: String,
}

/// Parse an SSE frame payload (`[{…}, …]`) into conclusions.
fn parse_frame(payload: &str) -> Result<Vec<Conclusion>, FrameError> {
    serde_json::from_str(payload).map_err(|error| FrameError {
        kind: ErrorKind::Parse,
        message: format!("frame: {error}"),
    })
}

/// True while `generation` still matches `Inner`'s — i.e. this
/// flow has not been superseded by an attribute change.
fn generation_current(inner: &Rc<RefCell<Inner>>, generation: u64) -> bool {
    inner.borrow().generation == generation
}

/// Reflect an error state on the host and dispatch
/// `tonk-layout:error`.
fn fail(host: &Element, kind: ErrorKind, message: impl Into<String>) {
    let detail = ErrorDetail::new(kind, message);
    state::set(host, State::Error);
    dispatch(host, "tonk-layout:error", &detail);
}

/// Dispatch `tonk-layout:layout` with the column and tile counts
/// for diagnostics.
fn dispatch_layout(host: &Element, layout: &Layout) {
    let tile_count: usize = layout.columns.iter().map(|c| c.tiles.len()).sum();
    let detail = serde_json::json!({
        "columns": layout.columns.len(),
        "tiles": tile_count,
    });
    dispatch(host, "tonk-layout:layout", &detail);
}

/// Dispatch a bubbling, composed `CustomEvent` carrying `detail`
/// serialized to a JS value.
fn dispatch(host: &Element, name: &str, detail: &impl serde::Serialize) {
    let init = CustomEventInit::new();
    init.set_bubbles(true);
    init.set_composed(true);
    if let Ok(value) = serde_wasm_bindgen::to_value(detail) {
        init.set_detail(&value);
    }
    if let Ok(event) = CustomEvent::new_with_event_init_dict(name, &init) {
        let _ = host.dispatch_event(&event);
    }
}

/// Register `<tonk-layout>` with the custom-element registry.
/// Idempotent — repeated calls after the first are no-ops.
pub fn register() {
    if already_registered() {
        return;
    }
    TonkLayout::define("tonk-layout");
}

/// True once `<tonk-layout>` is in the registry.
fn already_registered() -> bool {
    window()
        .map(|w| !w.custom_elements().get("tonk-layout").is_undefined())
        .unwrap_or(false)
}
