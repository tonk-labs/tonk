//! The `<tonk-tree>` custom element: a two-pane inspector for the
//! dialog-search-tree index behind a branch. Left pane is a lazy
//! `<wa-tree>` outline of index/segment nodes; right pane is the node
//! inspector. It resolves its repository from its own `with="branch@repo"`
//! attribute (forwarded onto it by the mounting `<tonk-display>`) and
//! drives itself from the worker's `tree/*` query
//! formulas.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use custom_elements::CustomElement;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::spawn_local;
use web_sys::{Element, Event, HtmlElement};

use crate::dom::{ElExt, clear, document, el};
use crate::inspector;
use crate::key;
use crate::model::{Kind, Loader, NodeHash, TreeNode};

/// Shared, mutable inspector state. Held in an `Rc<RefCell<…>>` so async
/// loads and event handlers can read and mutate it.
pub struct State {
    pub loader: Loader,
    /// Cached nodes by hash.
    pub nodes: HashMap<NodeHash, TreeNode>,
    /// Loaded child-hash lists, by index-node hash.
    pub children: HashMap<NodeHash, Vec<NodeHash>>,
    pub root: Option<NodeHash>,
    pub selected: Option<NodeHash>,
    pub max_size: u64,
    /// The shadow root we render into.
    pub shadow: web_sys::ShadowRoot,
}

impl State {
    fn put(&mut self, node: TreeNode) {
        self.max_size = self.max_size.max(node.size);
        self.nodes.entry(node.hash.clone()).or_insert(node);
    }
}

pub type Shared = Rc<RefCell<State>>;

#[derive(Default)]
pub struct TonkTreeElement {
    state: RefCell<Option<Shared>>,
}

impl CustomElement for TonkTreeElement {
    fn shadow() -> bool {
        // We manage our own shadow root (with a stylesheet), so opt out
        // of the framework attaching one.
        false
    }

    fn observed_attributes() -> &'static [&'static str] {
        &["repo", "branch"]
    }

    fn inject_children(&mut self, _this: &HtmlElement) {}

    fn connected_callback(&mut self, this: &HtmlElement) {
        // Defer a tick so the display's forwarded `with` context is set.
        let this = this.clone();
        let slot = self.state.clone();
        spawn_local(async move {
            ensure_wa_tree().await;
            start(&this, &slot);
        });
    }

    fn attribute_changed_callback(
        &mut self,
        this: &HtmlElement,
        _name: String,
        old: Option<String>,
        new: Option<String>,
    ) {
        if old == new {
            return;
        }
        let this = this.clone();
        let slot = self.state.clone();
        spawn_local(async move {
            ensure_wa_tree().await;
            start(&this, &slot);
        });
    }

    fn disconnected_callback(&mut self, _this: &HtmlElement) {
        *self.state.borrow_mut() = None;
    }
}

/// Resolve the repo/branch, build the panes, and kick off the root load.
fn start(this: &HtmlElement, slot: &RefCell<Option<Shared>>) {
    let Some(location) = resolve(this) else {
        mount_error(
            this,
            "no repository in context (set with=\"branch@repo\" or mount inside a routed <tonk-display>)",
        );
        return;
    };

    let shadow = ensure_shadow(this);
    let state = Rc::new(RefCell::new(State {
        loader: Loader::new(&location),
        nodes: HashMap::new(),
        children: HashMap::new(),
        root: None,
        selected: None,
        max_size: 1,
        shadow,
    }));
    *slot.borrow_mut() = Some(state.clone());

    render_shell(&state);

    spawn_local(async move {
        let loader = state.borrow().loader.clone();
        let root = loader.root().await;
        match root {
            Ok(Some(node)) => {
                {
                    let mut s = state.borrow_mut();
                    s.root = Some(node.hash.clone());
                    s.selected = Some(node.hash.clone());
                    s.put(node);
                }
                render_outline(&state);
                inspector::render(&state);
            }
            Ok(None) => set_outline_status(&state, "empty tree"),
            Err(e) => set_outline_status(&state, &e),
        }
    });
}

/// Resolve the routing [`Location`] the tree queries against. An
/// explicit `repo`/`branch` attribute pair names a space directly;
/// otherwise the element's own `with` context (a named space or the
/// profile endpoint) drives it. Returns `None` only when neither is
/// present, so the tree can inspect either a space or the profile DB.
fn resolve(this: &HtmlElement) -> Option<tonk_host::location::Location> {
    use tonk_host::location::{Location, Repo};
    if let Some(repo) = this.get_attribute("repo") {
        let branch = this.get_attribute("branch");
        return Some(Location {
            repo: Repo::Named(repo),
            branch,
        });
    }
    own_with(this)
}

/// This element's OWN parsed `with` context, skipping an unstamped `{…}`
/// placeholder. Routing is never inferred from ancestors — the mounting
/// `<tonk-display>` forwards its context onto this element (see
/// `forward_with`), and absent that the guest's pinned site context
/// applies.
fn own_with(this: &HtmlElement) -> Option<tonk_host::location::Location> {
    this.get_attribute("with")
        .filter(|v| !v.is_empty() && !v.contains('{'))
        .and_then(|v| v.parse().ok())
}

/// Attach (once) a shadow root and return it.
fn ensure_shadow(this: &HtmlElement) -> web_sys::ShadowRoot {
    if let Some(root) = this.shadow_root() {
        return root;
    }
    let init = web_sys::ShadowRootInit::new(web_sys::ShadowRootMode::Open);
    this.attach_shadow(&init).unwrap()
}

/// Build the two-pane shell with styles. Idempotent per render.
fn render_shell(state: &Shared) {
    let shadow = state.borrow().shadow.clone();
    clear(shadow.unchecked_ref::<Element>());

    let style = el("style").text(STYLE);
    let _ = shadow.append_child(&style);

    let outline_pane = el("div").class("pane left");
    let outline = el("dialog-tree-outline").class("outline");
    // Column header — one place for the value labels instead of repeating
    // units on every row. It aligns with the row columns because the rows now
    // fill the full width (see the `::part(label)` rule).
    let header = el("div").class("col-header");
    let _ = header.append_child(&el("span").class("keystr").text("node"));
    let _ = header.append_child(&el("span").class("col size").text("size"));
    let _ = header.append_child(&el("span").class("col count").text("count"));
    let _ = outline.append_child(&header);
    let tree = el("wa-tree").attr("selection", "single");
    let _ = outline.append_child(&tree);
    let _ = outline_pane.append_child(&outline);

    let inspector_pane = el("div").class("pane right");
    let inspector = el("div").class("inspector");
    let _ = inspector_pane.append_child(&inspector);

    let _ = shadow.append_child(&outline_pane);
    let _ = shadow.append_child(&inspector_pane);

    wire_selection(state);
}

fn wa_tree(state: &Shared) -> Element {
    state
        .borrow()
        .shadow
        .query_selector("wa-tree")
        .unwrap()
        .unwrap()
}

fn set_outline_status(state: &Shared, msg: &str) {
    let tree = wa_tree(state);
    clear(&tree);
    let _ = tree.append_child(&el("div").class("status").text(msg));
}

/// Render the outline from the root.
fn render_outline(state: &Shared) {
    let tree = wa_tree(state);
    clear(&tree);
    let root = state.borrow().root.clone();
    if let Some(root) = root {
        let item = build_item(state, &root, None, None);
        let _ = tree.append_child(&item);
    }
}

/// Build a `<wa-tree-item>` for a node. `prev`/`next` are the neighboring
/// siblings' bound keys, for front-coding the row's key.
fn build_item(state: &Shared, hash: &str, prev: Option<String>, next: Option<String>) -> Element {
    let node = state.borrow().nodes.get(hash).cloned();
    let item = el("wa-tree-item").attr("data-hash", hash);

    if let Some(node) = &node {
        // Expandable when it is a cached index with children, or any node we
        // have not fetched yet (we do not know its kind until pulled — the
        // same lazy expansion pulls it from the remote). The expand toggle is
        // our dot, which also encodes locality: filled when cached locally, a
        // hollow ring when not yet fetched.
        let expandable = (node.kind == Kind::Index && node.count > 0) || !node.cached;
        if expandable {
            let _ = item.append_child(&dot(node.cached, "").attr("slot", "expand-icon"));
            let _ = item.append_child(&dot(node.cached, "").attr("slot", "collapse-icon"));
        }

        let _ = item.append_child(&build_row(state, node, prev.as_deref(), next.as_deref()));

        if !expandable {
            // Leaf dot: a direct child of the item (not the row), absolutely
            // positioned at the connector anchor so it tracks `--indent` and
            // lines up with the elbow — an in-flow dot in the row does not.
            let _ = item.append_child(&dot(node.cached, "dot-leaf"));
        }

        if expandable {
            item.set_attribute("lazy", "").ok();
            // The parent's lower bound (`prev`) is the first child's lower
            // bound too, so its pivot is measured against the parent's left
            // edge rather than treated as all-bright.
            attach_lazy(state, &item, hash, prev);
        }
    }
    item
}

/// A node dot: a filled disc when the node is cached locally, a hollow ring
/// when it is not yet fetched. `extra` adds positioning classes.
fn dot(cached: bool, extra: &str) -> Element {
    let locality = if cached { "dot-local" } else { "dot-remote" };
    let cls = format!("dot {locality} {extra}");
    el("span").class(cls.trim_end())
}

/// Render one node row: front-coded key, count, size bar, remote. A branch
/// node's dot lives in the expand-icon slot (it toggles); a leaf's dot is a
/// separate positioned element on the item (see `build_item`).
fn build_row(state: &Shared, node: &TreeNode, prev: Option<&str>, next: Option<&str>) -> Element {
    let row = el("span").class(if node.cached { "row" } else { "row remote" });

    // The key, front-coded against the neighboring siblings.
    let keystr = el("span").class("keystr").attr("title", &node.hash);
    match (&node.bound, node.bound_parts.is_empty()) {
        (Some(bound), false) => append_key(&keystr, &node.bound_parts, bound, prev, next),
        // A bound with no decoded parts (or none at all) falls back to the
        // node hash — a leaf we have not fetched, or an undecodable bound.
        _ => keystr.set_text_content(Some(&short(&node.hash, 8))),
    }
    let _ = row.append_child(&keystr);

    // Size column: a proportional bar plus the byte count, in a fixed-width
    // right-aligned cell so the values line up down the right edge.
    let max = state.borrow().max_size.max(1);
    let frac = (node.size as f64 / max as f64).clamp(0.02, 1.0);
    let sizewrap = el("span").class("col size");
    let bar = el("span")
        .class("sizebar")
        .style(&format!("width: calc(70px * {frac})"));
    let num = el("span").class("sizenum").text(&human_size(node.size));
    let _ = sizewrap.append_child(&bar);
    let _ = sizewrap.append_child(&num);
    let _ = row.append_child(&sizewrap);

    // Count column: the child/entry count as a bare number in its own cell.
    let _ = row.append_child(&el("span").class("col count").text(&node.count.to_string()));

    if !node.cached {
        let _ = row.append_child(
            &el("wa-icon")
                .class("remote-icon")
                .attr("name", "cloud")
                .attr("label", "not cached locally"),
        );
    }
    row
}

thread_local! {
    /// Monotonic id source for wa-tooltip `for=` anchoring.
    static SEG_ID: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

fn next_seg_id() -> String {
    SEG_ID.with(|c| {
        let n = c.get() + 1;
        c.set(n);
        format!("kseg-{n}")
    })
}

/// Append a key's component segments to `parent` for the outline. Each
/// segment is a colored chip with a `wa-tooltip` naming the part; the first
/// is the index-type chip.
///
/// Dimming follows the *routing pivot* — the byte through which this bound
/// must stay bright to be distinguishable from both its `prev` and `next`
/// siblings. Bytes up to and including the pivot decide where a lookup
/// branches, so they stay bright; the tail past the pivot is dimmed. The
/// chip containing the pivot shows the hex bright through the pivot digit
/// then a short dim tail.
pub fn append_key(
    parent: &Element,
    parts: &[crate::model::KeyPart],
    key_hex: &str,
    prev: Option<&str>,
    next: Option<&str>,
) {
    let comps = key::components(parts);
    let pivot = key::pivot_byte(key_hex, prev, next);
    for (i, c) in comps.iter().enumerate() {
        let mut base = format!("key-seg {}", c.part.class());
        if i == 0 {
            base.push_str(" seg-index-type");
        }

        // `emit` appends one pill (chip) with the part's tooltip. A chip is
        // its own background, so dimming a chip dims its background too.
        let emit = |text: &str, dim: bool| {
            let id = next_seg_id();
            let cls = if dim {
                format!("{base} dim")
            } else {
                base.clone()
            };
            let chip = el("span")
                .attr("id", &id)
                .child(&el("span").class("seg-text").text(text));
            chip.set_class_name(&cls);
            let _ = parent.append_child(&chip);
            let _ = parent.append_child(&el("wa-tooltip").attr("for", &id).text(&c.label));
        };

        // Dimming is per-chip now that chips carry decoded TEXT (not hex): a
        // component whose bytes begin past the routing pivot is noise for this
        // row's placement, so it dims whole; a component containing or before
        // the pivot stays bright. (The old per-hex-digit split relied on the
        // 2-chars-per-byte hex mapping, which no longer holds.)
        match pivot {
            Some(p) if c.bytes.start > p => emit(&elide(&c.text), true),
            // Chip is at or before the pivot, or no pivot → bright, truncated.
            _ => emit(&elide(&c.text), false),
        }
    }
}

/// Shorten a chip's text for the dense outline: keep the head, elide the
/// middle of anything long (a `did:key:…` entity, a long value). The inspector
/// pane shows the full text; here we keep rows compact.
fn elide(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= 18 {
        return text.to_owned();
    }
    let head: String = chars[..12].iter().collect();
    let tail: String = chars[chars.len() - 4..].iter().collect();
    format!("{head}…{tail}")
}

/// Append a key's segments in full (the inspector pane): every chip shows
/// its complete hex, nothing dimmed or truncated, so the row may wrap.
pub fn append_key_full(parent: &Element, parts: &[crate::model::KeyPart]) {
    for (i, c) in key::components(parts).iter().enumerate() {
        let mut cls = format!("key-seg {}", c.part.class());
        if i == 0 {
            cls.push_str(" seg-index-type");
        }
        // Every chip shows its full decoded text (the worker already
        // textualized it — entity URI, attribute name, typed value).
        let text = &c.full;
        let id = next_seg_id();
        let chip = el("span")
            .attr("id", &id)
            .child(&el("span").class("seg-text").text(text));
        chip.set_class_name(&cls);
        let _ = parent.append_child(&chip);

        let tip = el("wa-tooltip").attr("for", &id).text(&c.label);
        let _ = parent.append_child(&tip);
    }
}

/// Wire `wa-lazy-load` on a branch item to load + append its children.
/// `parent_lower` is the parent's lower bound — the first child's lower
/// bound, used to measure its routing pivot.
fn attach_lazy(state: &Shared, item: &Element, hash: &str, parent_lower: Option<String>) {
    let state = state.clone();
    let hash = hash.to_owned();
    let item_c = item.clone();
    let cb = Closure::<dyn FnMut(Event)>::new(move |e: Event| {
        e.stop_propagation();
        let state = state.clone();
        let hash = hash.clone();
        let item = item_c.clone();
        let parent_lower = parent_lower.clone();
        // Mark the item loading so its dot spins (our loader) while the
        // children — possibly a network fetch — are in flight.
        let _ = item.set_attribute("data-loading", "");
        spawn_local(async move {
            let loader = state.borrow().loader.clone();
            let kids = loader.children(&hash).await;
            match kids {
                Ok(kids) => {
                    let mut hashes = Vec::with_capacity(kids.len());
                    {
                        let mut s = state.borrow_mut();
                        for k in &kids {
                            s.put(k.clone());
                            hashes.push(k.hash.clone());
                        }
                        s.children.insert(hash.clone(), hashes.clone());
                    }
                    // Each child's pivot is measured against both its
                    // neighbors: the previous sibling (the first child uses
                    // the parent's lower bound) and the next sibling.
                    let bounds: Vec<Option<String>> = {
                        let s = state.borrow();
                        hashes
                            .iter()
                            .map(|h| s.nodes.get(h).and_then(|n| n.bound.clone()))
                            .collect()
                    };
                    for (idx, h) in hashes.iter().enumerate() {
                        let prev = if idx == 0 {
                            parent_lower.clone()
                        } else {
                            bounds[idx - 1].clone()
                        };
                        let next = bounds.get(idx + 1).cloned().flatten();
                        let child = build_item(&state, h, prev, next);
                        let _ = item.append_child(&child);
                    }
                    finish_loading(&item);
                    // Expanding pulled the node from the remote (if it was not
                    // cached), so re-read its own fields — now cached, with a
                    // real kind/size/count — and refresh its row: the dot
                    // fills, the cloud drops, the stats update.
                    refresh_row(&state, &item, &hash).await;
                    // A newly-grown max-size could rescale bars, but we
                    // leave existing bars as-is to avoid a full re-render.
                }
                Err(e) => {
                    web_sys::console::error_1(&format!("tree/child: {e}").into());
                    finish_loading(&item);
                }
            }
        });
    });
    let _ = item.add_event_listener_with_callback("wa-lazy-load", cb.as_ref().unchecked_ref());
    cb.forget();
}

/// Clear a lazy item's loading state once its children resolved. Removing
/// the `lazy` attribute alone leaves wa-tree-item's `loading` property set
/// (its spinner spins forever) when the load returned nothing — a fetched
/// leaf has no children — so the property is reset explicitly.
fn finish_loading(item: &Element) {
    let _ = item.remove_attribute("lazy");
    let _ = item.remove_attribute("data-loading");
    let _ = js_sys::Reflect::set(item.as_ref(), &"loading".into(), &JsValue::FALSE);
}

/// Re-read a node by hash and rebuild its row + dot in place. Called after a
/// node is expanded (and thus fetched), so a previously-remote node picks up
/// its now-cached state: a filled dot, no cloud, real size/count.
async fn refresh_row(state: &Shared, item: &Element, hash: &str) {
    let loader = state.borrow().loader.clone();
    let Ok(Some(node)) = loader.node(hash).await else {
        return;
    };
    {
        let mut s = state.borrow_mut();
        s.nodes.insert(node.hash.clone(), node.clone());
    }
    // Swap the existing row for a fresh one (updated stats, no cloud).
    if let Some(old) = item.query_selector(":scope > .row").ok().flatten() {
        let row = build_row(state, &node, None, None);
        let _ = item.replace_child(&row, &old);
    }

    // Now that the node is fetched we know whether it is expandable (a cached
    // index with children) or a leaf. Update the locality dots *in place* —
    // flipping their filled/hollow class — rather than removing and
    // re-appending, which would briefly detach the dot (a floating dot on a
    // line) and reorder it after the children.
    let expandable = node.kind == Kind::Index && node.count > 0;
    let want_remote = !node.cached;
    let set_locality = |el: &Element| {
        let cls = el.class_name();
        let base = cls.replace(" dot-remote", "").replace(" dot-local", "");
        el.set_class_name(&format!(
            "{base} {}",
            if want_remote {
                "dot-remote"
            } else {
                "dot-local"
            }
        ));
    };

    let slotted: Vec<Element> = ["expand-icon", "collapse-icon"]
        .into_iter()
        .filter_map(|s| {
            item.query_selector(&format!(":scope > [slot=\"{s}\"].dot"))
                .ok()
                .flatten()
        })
        .collect();
    let leaf = item.query_selector(":scope > .dot-leaf").ok().flatten();

    // In both cases keep a dot in the expand-icon/collapse-icon slots: a
    // filled slotted dot suppresses wa-tree's default chevron (its fallback
    // when the slot is empty), so a fetched leaf does not show an arrow.
    slotted.iter().for_each(set_locality);
    if let Some(l) = leaf {
        // Migrate any inline leaf dot back into the slot so there is exactly
        // one marker and no chevron.
        l.remove();
    }
    if slotted.is_empty() {
        let _ = item.append_child(&dot(node.cached, "").attr("slot", "expand-icon"));
        let _ = item.append_child(&dot(node.cached, "").attr("slot", "collapse-icon"));
    }
    if !expandable {
        // A leaf has nothing to expand: drop the lazy contract and collapse
        // so the (now dotted) toggle does not reveal an empty group.
        let _ = item.remove_attribute("lazy");
        let _ = js_sys::Reflect::set(item.as_ref(), &"expanded".into(), &JsValue::FALSE);
    }
}

/// Install a selection listener on the wa-tree (once, in render_shell's
/// tree). Called after the shell is built.
pub fn wire_selection(state: &Shared) {
    let tree = wa_tree(state);
    let state_c = state.clone();
    let cb = Closure::<dyn FnMut(Event)>::new(move |e: Event| {
        // The selected item is event.detail.selection[0].
        let detail = js_sys::Reflect::get(&e, &"detail".into()).ok();
        let selection = detail
            .and_then(|d| js_sys::Reflect::get(&d, &"selection".into()).ok())
            .and_then(|s| js_sys::Reflect::get(&s, &0.into()).ok());
        if let Some(item) = selection
            && let Ok(item) = item.dyn_into::<Element>()
            && let Some(hash) = item.get_attribute("data-hash")
        {
            state_c.borrow_mut().selected = Some(hash);
            inspector::render(&state_c);
        }
    });
    let _ =
        tree.add_event_listener_with_callback("wa-selection-change", cb.as_ref().unchecked_ref());
    cb.forget();
}

/// Web Awesome's auto-loader does not scan shadow DOM, so import the
/// `<wa-tree>` modules ourselves from the served WA dist.
async fn ensure_wa_tree() {
    if window_has("wa-tree-item") {
        return;
    }
    let _ = import_module("/webawesome/components/tree/tree.js").await;
    let _ = import_module("/webawesome/components/tree-item/tree-item.js").await;
    let _ = import_module("/webawesome/components/tooltip/tooltip.js").await;
}

fn window_has(tag: &str) -> bool {
    document()
        .default_view()
        .map(|w| {
            let registry = w.custom_elements();
            !registry.get(tag).is_undefined()
        })
        .unwrap_or(false)
}

async fn import_module(src: &str) -> Result<(), String> {
    let promise =
        js_sys::eval(&format!("import('{src}')")).map_err(|e| format!("import eval: {e:?}"))?;
    let promise: js_sys::Promise = promise.dyn_into().map_err(|_| "not a promise")?;
    wasm_bindgen_futures::JsFuture::from(promise)
        .await
        .map(|_| ())
        .map_err(|e| format!("import: {e:?}"))
}

fn mount_error(this: &HtmlElement, msg: &str) {
    let shadow = ensure_shadow(this);
    clear(shadow.unchecked_ref::<Element>());
    let _ = shadow.append_child(&el("style").text(STYLE));
    let _ = shadow.append_child(&el("div").class("err").text(msg));
}

pub fn short(s: &str, n: usize) -> String {
    let raw = s.strip_prefix('#').unwrap_or(s);
    raw.chars().take(n).collect()
}

pub fn human_size(n: u64) -> String {
    if n < 1024 {
        format!("{n} B")
    } else if n < 1024 * 1024 {
        format!("{:.0} KB", n as f64 / 1024.0)
    } else {
        format!("{:.1} MB", n as f64 / (1024.0 * 1024.0))
    }
}

/// Register the `<tonk-tree>` element. Idempotent.
pub fn register() {
    if window_has("tonk-tree") {
        return;
    }
    TonkTreeElement::define("tonk-tree");
}

const STYLE: &str = r#"
/* The element is mounted bare inside a view fragment, so `height: 100%`
   has nothing to resolve against — the ancestor chain is auto-height and
   the panes grow to fit their content, scrolling the whole page instead
   of themselves. Styling the route's `tonk-view > *` to fix that would
   clobber other views' layout, so the bound lives here.

   The route slot gives this element a definite height (see the
   `.display-view-slot:has(tonk-tree)` rule in `tonk-ui/styles.css`),
   so filling the parent is enough. `flex: 1 1 0` claims the space when
   the parent is a flex column — which the guest body and the route
   chain both are — and `min-height: 0` overrides a flex item's
   automatic content-height minimum, the thing that otherwise lets the
   panes push past the bottom edge. Each pane then has a definite
   height to scroll within, independently of the other. */
:host {
  display: grid; grid-template-columns: 1.4fr 1fr;
  flex: 1 1 0; min-height: 0; height: 100%;
  box-sizing: border-box;
  color: var(--wa-color-text-normal);
  font-family: var(--wa-font-family-code, ui-monospace, monospace);
  font-size: var(--wa-font-size-s, 13px);
}
/* `min-height: 0` overrides the grid item's `auto` minimum, which would
   otherwise floor each pane at its content height and defeat the scroll. */
.pane { overflow: auto; min-height: 0; padding: var(--wa-space-m, 12px); }
.pane.left { border-right: 1px solid var(--wa-color-border-quiet); }
/* The outline runs at a smaller, denser size — closer to a D3 indented
   tree. The connectors are sized in `em`, so they tighten with it. The
   inspector pane keeps the host's normal size. */
dialog-tree-outline { font-size: var(--wa-font-size-xs, 11px); }
/* D3-style indented-tree connectors, scoped to this element's wa-tree.
   wa-tree's built-in vertical spine runs the full height of a subtree
   (past the last child, to nowhere), so it is disabled and the spine is
   drawn per-child instead: a continuous vertical line that each child's
   horizontal elbow forks off of. The vertical spans the whole row for
   every child so it runs unbroken through each elbow point; the last child
   stops its vertical at its own dot, terminating the spine cleanly. The
   dot center sits at (0.1875em + --indent + 1em) from the item's content
   box; the spine is one level (2em) left of the dot, so the elbow spans 2em
   to meet the dot exactly. Lines use the quiet text grey, like the labels. */
wa-tree { --indent-guide-color: transparent; }
wa-tree-item { position: relative;
  /* The selected row is marked by wa-tree's brand accent bar alone — drop
     its fill and the loud blue focus ring; the focused row gets a quiet
     fill on its own row instead (below). */
  --wa-color-neutral-fill-quiet: transparent;
  --wa-focus-ring: 0 0 0 0 transparent; }
/* Focused (keyboard-navigated) row: a quiet translucent fill spanning the
   full row width like wa-tree's native highlight. Painted on the host (which
   is full width, unlike the inset label) as a sized gradient band so it is
   only the node's own row tall and does not bleed into the subtree below. */
wa-tree-item:focus-visible, wa-tree-item:focus {
  background-image: linear-gradient(
    color-mix(in srgb, var(--wa-color-text-normal) 10%, transparent),
    color-mix(in srgb, var(--wa-color-text-normal) 10%, transparent));
  background-repeat: no-repeat; background-size: 100% 2em; background-position: 0 0; }
/* z-index keeps the lines above a focused row's fill so they stay crisp. */
wa-tree-item::before, wa-tree-item::after {
  content: ''; position: absolute; z-index: 3;
  inset-inline-start: calc(0.1875em + var(--indent) - 1em);
  border-color: var(--wa-color-border-loud, rgb(56 24 42 / 55%)); }
/* Horizontal elbow forking off the spine. It stops at the dot's left edge
   (~5px short of the center) so the line never enters the circle.
   (width/style only — a `border-top` shorthand would reset the color set
   above to currentColor, turning the line bright.) */
wa-tree-item::before { top: 1em; width: calc(2em - 5px);
  border-top-width: 1px; border-top-style: solid; }
/* Continuous vertical spine. It starts 1em above this row's top so it
   reaches up to the parent's dot (which sits 1em into the parent row, just
   above the first child) — otherwise the spine would float a row below the
   parent. The last child stops its spine at its own dot. */
wa-tree-item::after { top: -1em; bottom: 0; width: 0;
  border-inline-start-width: 1px; border-inline-start-style: solid; }
wa-tree-item:last-child::after { bottom: auto; height: 2em; }
/* The root has no parent spine. */
wa-tree > wa-tree-item::before, wa-tree > wa-tree-item::after { display: none; }
/* Make the label fill the item's full width (it is content-sized by default)
   so every row — whatever its indent — shares the same right edge. The value
   columns then right-pack to a common column, aligned across all depths. */
wa-tree-item::part(label) { flex: 1 1 auto; min-width: 0; }
.row { display: inline-flex; align-items: center; gap: var(--wa-space-s, 8px); width: 100%; }
.row.remote { opacity: 0.5; }
/* The node dot replaces wa-tree's expand chevron and encodes locality:
   a filled disc when cached locally, a hollow ring when not yet fetched.
   A branch dot sits in the expand-icon slot (and is the toggle); a leaf dot
   is absolutely positioned on the item at the connector anchor. */
/* The dot sits above the connector lines (z-index > the lines' 3); the
   elbow stops at its edge and the spine is one level left, so no line
   enters the circle and no opaque fill is needed. */
.dot { flex: none; width: 8px; height: 8px; border-radius: 50%; box-sizing: border-box;
  position: relative; z-index: 5; }
.dot-local { background: var(--wa-color-border-loud, rgb(56 24 42 / 55%)); border: 1.5px solid var(--wa-color-border-loud, rgb(56 24 42 / 55%)); }
.dot-remote { background: transparent; border: 1.5px solid var(--wa-color-border-loud, rgb(56 24 42 / 55%)); }
/* A branch dot lives in the 2em-wide expand slot; keep it from inheriting
   the chevron's rotate-on-expand. */
[slot="expand-icon"].dot, [slot="collapse-icon"].dot { rotate: none !important; }
/* While a node loads (a network fetch on expand) the dot goes hollow and a
   single higher-contrast arc — exactly the dot's size, one side open —
   spins in its place. The base border is dropped so there is no second
   ring; wa-tree's own spinner is hidden. */
@keyframes tonk-dot-spin { to { rotate: 360deg; } }
/* wa-tree reveals BOTH the expand- and collapse-icon slots during its
   loading state, so our two slotted dots would both show (a stacked
   figure-8). Show the spinner on the expand-icon dot only. */
dialog-tree-outline wa-tree-item[data-loading] [slot="collapse-icon"].dot { display: none; }
dialog-tree-outline wa-tree-item[data-loading] .dot { border-color: transparent; }
dialog-tree-outline wa-tree-item[data-loading] .dot::after {
  content: ''; position: absolute; top: 50%; left: 50%;
  width: 8px; height: 8px; margin: -4px 0 0 -4px; border-radius: 50%; box-sizing: border-box;
  border: 1.5px solid var(--wa-color-text-quiet, #5b4953);
  border-top-color: transparent;
  animation: tonk-dot-spin 0.7s linear infinite; }
dialog-tree-outline wa-tree-item::part(spinner) { display: none; }
/* A loading node has no children yet: hide its elbow so nothing enters the
   spinner, and stop its downward spine at the dot. */
dialog-tree-outline wa-tree-item[data-loading]::before { opacity: 0; }
dialog-tree-outline wa-tree-item[data-loading]::after { bottom: auto; height: 1em; }
/* A leaf dot: placed at the same anchor as the elbow end
   (0.1875em + --indent + 1em), so it lines up with the connectors. */
wa-tree-item > .dot-leaf { position: absolute; z-index: 5;
  inset-inline-start: calc(0.1875em + var(--indent) + 1em); top: 1em;
  transform: translate(-50%, -50%); }
.keystr { white-space: nowrap; display: inline-flex; align-items: center; gap: 3px;
  flex: 1 1 auto; min-width: 0; }
/* In the outline, key segments are colored TEXT (no background fill) so
   the row stays compact. The component class sets the color. A thin divider
   separates adjacent chips so the decoded parts read as a delimited key
   (entity · attribute · value) rather than running together. */
.key-seg { font-weight: var(--wa-font-weight-semibold, 600);
  display: inline-flex; align-items: center; }
.keystr .key-seg + .key-seg::before {
  content: "\2009\00B7\2009"; color: var(--wa-color-text-quiet, #5b4953);
  font-weight: 400; }
/* A NUL / control byte rendered as a glyph (␀ / ·) reads as structure, not
   text; dim it so the real content stands out. */
.key-seg .seg-text { unicode-bidi: plaintext; }
.key-seg.seg-entity { color: var(--tonk-circle, #3d6da8); }
.key-seg.seg-attribute { color: var(--tonk-triangle, #c89a2b); }
.key-seg.seg-value { color: var(--tonk-square, #b94a3d); }
/* The value-TYPE tag belongs to the value it precedes, so it carries the
   value color rather than a hue of its own — violet sat outside the
   Bauhaus palette. Reduced opacity and the lighter weight keep it
   subordinate to the value itself. */
.key-seg.seg-vtype { color: var(--tonk-square, #b94a3d); opacity: 0.7;
  font-weight: var(--wa-font-weight-normal, 400); }
.key-seg.seg-unknown { color: var(--tonk-closure, #7a7268); }
/* The index-type segment is neutral (the page's normal text color). */
.key-seg.seg-index-type { color: var(--wa-color-text-normal); }
/* Past the routing pivot: dim the segment — text and color. */
.key-seg.dim { opacity: 0.6; font-weight: var(--wa-font-weight-normal, 400); }
/* Value columns (size, count): fixed-width right-aligned cells anchored to
   the pane's right edge, so they line up across rows regardless of the key's
   width or indent depth — D3 indented-tree style. The key flexes; the first
   column gets a margin so it sits a comfortable distance from the key. */
.col { flex: none; color: var(--wa-color-text-quiet); white-space: nowrap;
  text-align: right; }
.col.size { width: 120px; margin-left: var(--wa-space-l, 24px);
  display: inline-flex; align-items: center; justify-content: flex-end;
  gap: var(--wa-space-xs, 6px); }
.col.count { width: 48px; }
/* Column header: same column widths as the rows, so the labels sit over
   their values. The key cell flexes like a row's, indented to clear the
   tree's dot gutter. */
.col-header { display: flex; align-items: center; gap: var(--wa-space-s, 8px);
  padding: 0 0 4px; margin-bottom: 4px;
  color: var(--wa-color-text-quiet); border-bottom: 1px solid var(--wa-color-border-quiet);
  text-transform: uppercase; letter-spacing: 0.05em; font-size: 0.85em; opacity: 0.8; }
.col-header .keystr { flex: 1 1 auto; padding-left: 1.4em; }
.sizebar { height: 7px; background: var(--tonk-closure, #7a7268); border-radius: var(--wa-border-radius-s, 2px); min-width: 2px; }
.row.remote .sizebar { background: var(--wa-color-neutral-fill-loud, #666); }
.sizenum { width: 48px; text-align: right; }
.remote-icon { color: var(--wa-color-text-quiet, #5b4953); flex: none; }
.status, .err { color: var(--wa-color-text-quiet); font-style: italic; padding: 4px 0; }
.err { color: var(--tonk-alarm, #a8302a); }

/* Inspector pane */
.inspector h2 { margin: 0 0 var(--wa-space-s, 8px); font-size: var(--wa-font-size-xs, 12px);
  font-weight: 600; color: var(--tonk-circle, #3d6da8); text-transform: uppercase; letter-spacing: 0.04em; }
.inspector .kv { display: flex; gap: var(--wa-space-s, 8px); margin: 2px 0; }
.inspector .kv .k { color: var(--wa-color-text-quiet); min-width: 64px; }
.inspector .kv .v { word-break: break-all; }
.inspector .sizebar { display: inline-block; vertical-align: middle; margin-left: 6px; }
/* Inspector key: full hex as solid chips (background fill, mode-inverse
   text), flowing as inline text so a long part wraps mid-chip and the next
   part follows immediately. box-decoration-break paints both line fragments
   of a wrapped chip. */
.keybytes { margin: 4px 0 6px; line-height: 2; }
.keybytes .key-seg { display: inline; white-space: normal; word-break: break-all;
  -webkit-box-decoration-break: clone; box-decoration-break: clone;
  color: light-dark(#000, #fff); border-radius: var(--wa-border-radius-s, 2px);
  padding: 1px 4px; margin-right: 3px; }
.keybytes .key-seg.seg-entity { background: var(--tonk-circle, #3d6da8); color: light-dark(#000, #fff); }
.keybytes .key-seg.seg-attribute { background: var(--tonk-triangle, #c89a2b); color: light-dark(#000, #fff); }
.keybytes .key-seg.seg-value { background: var(--tonk-square, #b94a3d); color: light-dark(#000, #fff); }
.keybytes .key-seg.seg-vtype { background: var(--tonk-square, #b94a3d);
  color: light-dark(#000, #fff); opacity: 0.7; }
.keybytes .key-seg.seg-unknown { background: var(--tonk-closure, #7a7268); color: light-dark(#000, #fff); }
.keybytes .key-seg.seg-index-type { background: light-dark(#000, #fff);
  color: light-dark(#fff, #000); }
.keybytes .key-seg .seg-text { white-space: normal; word-break: break-all; }
.entries { margin-top: var(--wa-space-m, 12px); }
.entries .k { color: var(--wa-color-text-quiet); }
table { width: 100%; border-collapse: collapse; font-size: var(--wa-font-size-xs, 12px); }
th { text-align: left; color: var(--wa-color-text-quiet); font-weight: 600;
  padding: 3px 8px 3px 0; border-bottom: 1px solid var(--wa-color-border-normal); }
td { padding: 3px 8px 3px 0; border-bottom: 1px solid var(--wa-color-border-quiet); vertical-align: top; }
tr.entry { cursor: pointer; }
tr.entry:hover { background: var(--wa-color-surface-raised, rgba(255,255,255,0.04)); }
tr.entry.removed td { color: var(--wa-color-text-quiet); }
/* Columns + value types reuse the app's Bauhaus code palette so the
   inspector reads like the notation editor / query tree. */
.col-attr { color: var(--tonk-triangle, #c89a2b); }
/* Entities are underlined wherever they appear (URIs), so the entity
   column matches `.val-entity`. */
.col-ent { color: var(--tonk-circle, #3d6da8); text-decoration: underline; }
.val-entity { color: var(--tonk-circle, #3d6da8); text-decoration: underline; }
/* Numbers/bool/bytes stay neutral so they never read as entities (blue). */
.val-boolean, .val-unsignedint, .val-signedint, .val-float,
.val-bytes, .val-record { color: var(--tonk-closure, #7a7268); }
.val-symbol { color: var(--tonk-triangle, #c89a2b); }
tr.detail td { padding: 4px 0 8px 16px; }
.entry-detail .keybytes { margin: 4px 0; }
"#;
