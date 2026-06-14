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
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
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
        let item = build_item(state, &root, None);
        let _ = tree.append_child(&item);
    }
}

/// Build a `<wa-tree-item>` for a node. `prev` is the previous sibling's
/// bound key, for front-coding the row's key.
fn build_item(state: &Shared, hash: &str, prev: Option<String>) -> Element {
    let node = state.borrow().nodes.get(hash).cloned();
    let item = el("wa-tree-item").attr("data-hash", hash);

    if let Some(node) = &node {
        let _ = item.append_child(&build_row(state, node, prev.as_deref()));
        if node.kind == Kind::Index && node.count > 0 {
            item.set_attribute("lazy", "").ok();
            attach_lazy(state, &item, hash);
        }
    }
    item
}

/// Render one node row: front-coded key, count, size bar, remote. Whether a
/// node is an index or a segment reads from its unfold arrow (index nodes
/// have children), so no kind icon is drawn.
fn build_row(state: &Shared, node: &TreeNode, prev: Option<&str>) -> Element {
    let row = el("span").class(if node.cached { "row" } else { "row remote" });

    // The key, front-coded against the previous sibling.
    let keystr = el("span").class("keystr").attr("title", &node.hash);
    if let Some(bound) = &node.bound {
        append_key(&keystr, bound, prev);
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

/// Append a key's component segments to `parent`. Every segment is a chip
/// whose background color comes from its component class (`seg-entity`,
/// `seg-value`, …) — no inline styles. The first segment is the index-type
/// chip. Leading segments shared with the previous sibling are dimmed
/// (front coding). Each chip carries a `wa-tooltip` (anchored by id) naming
/// the part.
pub fn append_key(parent: &Element, key_str: &str, prev: Option<&str>) {
    let comps = key::components(key_str);
    let shared = key::shared_prefix_len(key_str, prev);
    for (i, c) in comps.iter().enumerate() {
        let mut cls = format!("key-seg {}", c.part.class());
        if i == 0 {
            cls.push_str(" seg-index-type");
        }
        if i < shared {
            cls.push_str(" shared");
        }

        let id = next_seg_id();
        let chip = el("span")
            .attr("id", &id)
            .child(&el("span").class("seg-text").text(&c.text));
        chip.set_class_name(&cls);
        let _ = parent.append_child(&chip);

        let tip = el("wa-tooltip").attr("for", &id).text(&c.label);
        let _ = parent.append_child(&tip);
    }
}

/// Wire `wa-lazy-load` on a branch item to load + append its children.
fn attach_lazy(state: &Shared, item: &Element, hash: &str) {
    let state = state.clone();
    let hash = hash.to_owned();
    let item_c = item.clone();
    let cb = Closure::<dyn FnMut(Event)>::new(move |e: Event| {
        e.stop_propagation();
        let state = state.clone();
        let hash = hash.clone();
        let item = item_c.clone();
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
                    // Front-code each child against the previous sibling.
                    let mut prev: Option<String> = None;
                    for h in &hashes {
                        let child = build_item(&state, h, prev.clone());
                        let _ = item.append_child(&child);
                        prev = state.borrow().nodes.get(h).and_then(|n| n.bound.clone());
                    }
                    item.remove_attribute("lazy").ok();
                    // A newly-grown max-size could rescale bars, but we
                    // leave existing bars as-is to avoid a full re-render.
                }
                Err(e) => {
                    web_sys::console::error_1(&format!("tree/child: {e}").into());
                    item.remove_attribute("lazy").ok();
                }
            }
        });
    });
    let _ = item.add_event_listener_with_callback("wa-lazy-load", cb.as_ref().unchecked_ref());
    cb.forget();
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
.row { display: inline-flex; align-items: center; gap: var(--wa-space-s, 8px); width: 100%; }
.row.remote { opacity: 0.5; }
.keystr { white-space: nowrap; display: inline-flex; align-items: center; gap: 3px; }
/* Every key segment is a solid chip: its background comes from the
   component class below; text is mode-inverse (black on light, white on
   dark) so it reads on any of the Bauhaus backgrounds. */
.key-seg { font-weight: var(--wa-font-weight-semibold, 600); padding: 0 5px;
  border-radius: var(--wa-border-radius-s, 2px); color: light-dark(#000, #fff);
  display: inline-flex; align-items: center; }
.key-seg.seg-entity { background: var(--tonk-circle, #3d6da8); }
.key-seg.seg-attribute { background: var(--tonk-triangle, #c89a2b); }
.key-seg.seg-value { background: var(--tonk-square, #b94a3d); }
.key-seg.seg-vtype { background: var(--tonk-square, #b94a3d); }
.key-seg.seg-unknown { background: var(--tonk-closure, #7a7268); }
/* The index-type chip is neutral: a mode-inverse background (black in
   light, white in dark) with the opposite text color. */
.key-seg.seg-index-type { background: light-dark(#000, #fff); color: light-dark(#fff, #000); }
.key-seg.shared { opacity: 0.35; font-weight: var(--wa-font-weight-normal, 400); }
.count { color: var(--wa-color-text-quiet); font-size: var(--wa-font-size-xs, 11px); flex: none; }
.sizewrap { display: inline-flex; align-items: center; gap: var(--wa-space-xs, 6px); margin-left: auto; flex: none; }
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
.keybytes { margin: 4px 0 6px; line-height: 1.7; }
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
.col-ent { color: var(--tonk-circle, #3d6da8); }
.val-entity { color: var(--tonk-circle, #3d6da8); text-decoration: underline; }
.val-text { color: var(--tonk-square, #b94a3d); }
/* Numbers/bool/bytes stay neutral so they never read as entities (blue). */
.val-boolean, .val-unsignedint, .val-signedint, .val-float,
.val-bytes, .val-record { color: var(--tonk-closure, #7a7268); }
.val-symbol { color: var(--tonk-triangle, #c89a2b); }
tr.detail td { padding: 4px 0 8px 16px; }
.entry-detail .keybytes { margin: 4px 0; }
"#;
