//! The `<tonk-tree>` custom element: a two-pane inspector for the
//! dialog-search-tree index behind a branch. Left pane is a lazy
//! `<wa-tree>` outline of index/segment nodes; right pane is the node
//! inspector. It resolves its repository from the `<tonk-repository>`
//! routing ancestor and drives itself from the worker's `tree/*` query
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
        // Defer a tick so the <tonk-repository> ancestor's `name` is set.
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
    let Some((repo, branch)) = resolve(this) else {
        mount_error(
            this,
            "no repository in context (nest under <tonk-repository>)",
        );
        return;
    };

    let shadow = ensure_shadow(this);
    let state = Rc::new(RefCell::new(State {
        loader: Loader::new(&repo, &branch),
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

/// Resolve `repo`/`branch` from attributes or the `<tonk-repository>` /
/// `<tonk-branch>` ancestors.
fn resolve(this: &HtmlElement) -> Option<(String, String)> {
    let repo = this
        .get_attribute("repo")
        .or_else(|| ancestor_attr(this, "tonk-repository", "name"))?;
    let branch = this
        .get_attribute("branch")
        .or_else(|| ancestor_attr(this, "tonk-branch", "name"))
        .unwrap_or_else(|| "main".to_owned());
    Some((repo, branch))
}

fn ancestor_attr(this: &HtmlElement, tag: &str, attr: &str) -> Option<String> {
    let mut node: Option<Element> = this.dyn_ref::<Element>().cloned();
    while let Some(current) = node {
        if current.local_name() == tag
            && let Some(v) = current.get_attribute(attr)
            && !v.is_empty()
        {
            return Some(v);
        }
        node = current.parent_element();
    }
    None
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
    if let Some(bound) = &node.bound {
        append_key(&keystr, bound, prev, next);
    } else {
        keystr.set_text_content(Some(&short(&node.hash, 8)));
    }
    let _ = row.append_child(&keystr);

    let noun = if node.kind == Kind::Index {
        "children"
    } else {
        "entries"
    };
    let _ = row.append_child(
        &el("span")
            .class("count")
            .text(&format!("{} {noun}", node.count)),
    );

    // Size bar.
    let max = state.borrow().max_size.max(1);
    let frac = (node.size as f64 / max as f64).clamp(0.02, 1.0);
    let sizewrap = el("span").class("sizewrap");
    let bar = el("span")
        .class("sizebar")
        .style(&format!("width: calc(120px * {frac})"));
    let num = el("span").class("sizenum").text(&human_size(node.size));
    let _ = sizewrap.append_child(&bar);
    let _ = sizewrap.append_child(&num);
    let _ = row.append_child(&sizewrap);

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
pub fn append_key(parent: &Element, key_str: &str, prev: Option<&str>, next: Option<&str>) {
    let comps = key::components(key_str);
    let pivot = key::pivot_byte(key_str, prev, next);
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

        match pivot {
            // Chip lies entirely past the pivot → one dim pill, kept short.
            Some(p) if c.bytes.start > p => emit(&c.text, true),
            // Chip straddles the pivot → a bright pill through the pivot
            // byte and a separate dim pill for the short tail, so the tail's
            // background dims with its text (the rest is routing noise).
            Some(p) if c.bytes.end > p + 1 && p + 1 > c.bytes.start => {
                let bright_chars = (p + 1 - c.bytes.start) * 2;
                let full: Vec<char> = c.full.chars().collect();
                let head: String = full.iter().take(bright_chars).collect();
                let rest = full.len() - bright_chars;
                let tail: String = if rest > 6 {
                    let t: String = full.iter().skip(full.len() - 4).collect();
                    format!("…{t}")
                } else {
                    full.iter().skip(bright_chars).collect()
                };
                emit(&head, false);
                if !tail.is_empty() {
                    emit(&tail, true);
                }
            }
            // Chip is at or before the pivot, or no pivot → bright, truncated.
            _ => emit(&c.text, false),
        }
    }
}

/// Append a key's segments in full (the inspector pane): every chip shows
/// its complete hex, nothing dimmed or truncated, so the row may wrap.
pub fn append_key_full(parent: &Element, key_str: &str) {
    for (i, c) in key::components(key_str).iter().enumerate() {
        let mut cls = format!("key-seg {}", c.part.class());
        if i == 0 {
            cls.push_str(" seg-index-type");
        }
        // Single-byte chips (index type, value type) show just the byte;
        // the multi-byte parts show their full hex.
        let text = if c.bytes.len() <= 1 { &c.text } else { &c.full };
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

    if expandable {
        // Keep / fill the slotted dots; a stray leaf dot (node was first a
        // leaf) is removed.
        slotted.iter().for_each(set_locality);
        if let Some(l) = leaf {
            l.remove();
        }
    } else {
        // A leaf: no expand toggle. Drop the lazy contract and collapse so
        // wa-tree stops drawing an expand button, remove the slotted dots,
        // and ensure a single inline leaf dot.
        let _ = item.remove_attribute("lazy");
        let _ = js_sys::Reflect::set(item.as_ref(), &"expanded".into(), &JsValue::FALSE);
        slotted.iter().for_each(|s| s.remove());
        match leaf {
            Some(l) => set_locality(&l),
            None => {
                let _ = item.append_child(&dot(node.cached, "dot-leaf"));
            }
        }
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
:host {
  display: grid; grid-template-columns: 1.4fr 1fr; height: 100%;
  min-height: 240px; color: var(--wa-color-text-normal);
  font-family: var(--wa-font-family-code, ui-monospace, monospace);
  font-size: var(--wa-font-size-s, 13px);
}
.pane { overflow: auto; padding: var(--wa-space-m, 12px); }
.pane.left { border-right: 1px solid var(--wa-color-border-quiet); }
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
  border-color: var(--wa-color-border-normal, #43454d); }
/* Horizontal elbow forking off the spine, reaching the node's dot.
   (width/style only — a `border-top` shorthand would reset the color set
   above to currentColor, turning the line bright.) */
wa-tree-item::before { top: 1em; width: 2em; border-top-width: 1px; border-top-style: solid; }
/* Continuous vertical spine. It starts 1em above this row's top so it
   reaches up to the parent's dot (which sits 1em into the parent row, just
   above the first child) — otherwise the spine would float a row below the
   parent. The last child stops its spine at its own dot. */
wa-tree-item::after { top: -1em; bottom: 0; width: 0;
  border-inline-start-width: 1px; border-inline-start-style: solid; }
wa-tree-item:last-child::after { bottom: auto; height: 2em; }
/* The root has no parent spine. */
wa-tree > wa-tree-item::before, wa-tree > wa-tree-item::after { display: none; }
.row { display: inline-flex; align-items: center; gap: var(--wa-space-s, 8px); width: 100%; }
.row.remote { opacity: 0.5; }
/* The node dot replaces wa-tree's expand chevron and encodes locality:
   a filled disc when cached locally, a hollow ring when not yet fetched.
   A branch dot sits in the expand-icon slot (and is the toggle); a leaf dot
   is absolutely positioned on the item at the connector anchor. */
.dot { flex: none; width: 8px; height: 8px; border-radius: 50%; box-sizing: border-box; }
.dot-local { background: var(--wa-color-border-normal, #43454d); border: 1.5px solid var(--wa-color-border-normal, #43454d); }
.dot-remote { background: transparent; border: 1.5px solid var(--wa-color-border-normal, #43454d); }
/* A branch dot lives in the 2em-wide expand slot; keep it from inheriting
   the chevron's rotate-on-expand. */
[slot="expand-icon"].dot, [slot="collapse-icon"].dot { rotate: none !important; }
/* A leaf dot: placed at the same anchor as the elbow end
   (0.1875em + --indent + 1em), so it lines up with the connectors. */
wa-tree-item > .dot-leaf { position: absolute; z-index: 4;
  inset-inline-start: calc(0.1875em + var(--indent) + 1em); top: 1em;
  transform: translate(-50%, -50%); }
.keystr { white-space: nowrap; display: inline-flex; align-items: center; gap: 3px;
  flex: 1 1 auto; min-width: 0; }
/* In the outline, key segments are colored TEXT (no background fill) so
   the row stays compact. The component class sets the color. */
.key-seg { font-weight: var(--wa-font-weight-semibold, 600);
  display: inline-flex; align-items: center; }
.key-seg.seg-entity { color: var(--tonk-circle, #3d6da8); }
.key-seg.seg-attribute { color: var(--tonk-triangle, #c89a2b); }
.key-seg.seg-value { color: var(--tonk-square, #b94a3d); }
.key-seg.seg-vtype { color: var(--tonk-square, #b94a3d); }
.key-seg.seg-unknown { color: var(--tonk-closure, #7a7268); }
/* The index-type segment is neutral (the page's normal text color). */
.key-seg.seg-index-type { color: var(--wa-color-text-normal); }
/* Past the routing pivot: dim the segment — text and color. */
.key-seg.dim { opacity: 0.6; font-weight: var(--wa-font-weight-normal, 400); }
/* count and size are fixed-width right-aligned columns so they line up
   across rows regardless of the key's width or the row's indent depth
   (the columns hug the right edge of the pane, D3 value-column style). */
.count { color: var(--wa-color-text-quiet); font-size: var(--wa-font-size-xs, 11px);
  flex: none; width: 84px; text-align: right; white-space: nowrap; }
.sizewrap { display: inline-flex; align-items: center; justify-content: flex-end;
  gap: var(--wa-space-xs, 6px); flex: none; }
.sizebar { height: 7px; background: var(--tonk-closure, #7a7268); border-radius: var(--wa-border-radius-s, 2px); min-width: 2px; }
.row.remote .sizebar { background: var(--wa-color-neutral-fill-loud, #666); }
.sizenum { color: var(--wa-color-text-quiet); font-size: var(--wa-font-size-xs, 11px); width: 56px; text-align: right; }
.remote-icon { color: var(--tonk-circle, #3d6da8); flex: none; }
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
.keybytes .key-seg.seg-vtype { background: var(--tonk-square, #b94a3d); color: light-dark(#000, #fff); }
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
