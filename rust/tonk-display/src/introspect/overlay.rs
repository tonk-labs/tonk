//! `<tonk-introspect>` — the overlay that paints what the machine
//! decides.
//!
//! One instance per document, auto-mounted by [`register`]. It is
//! inert until Alt goes down: the only thing installed while the hood
//! is closed is a document `mousemove` listener whose first act is to
//! read `altKey` and return.
//!
//! Alt state is read off the *pointer* event rather than remembered
//! from a `keydown`. A guest iframe that never had focus receives no
//! key events at all, but every mouse event it does receive carries
//! the modifier flags — so hovering works in a frame that has never
//! been clicked, which is the common case for a page of sealed views.
//! `keyup` is still listened for, as the one way to notice Alt going
//! up while the pointer sits perfectly still.

use std::cell::RefCell;
use std::rc::Rc;

use custom_elements::CustomElement;
use js_sys::Function;
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use web_sys::{
    Document, Element, Event, HtmlElement, KeyboardEvent, MouseEvent, Node, Range, ShadowRootMode,
    window,
};

use super::mode::{Input, Machine, TargetId};
use super::registry;
use super::slot::{Origin, Slot, SlotKind};

/// How long a change flash stays up.
const FLASH_MS: i32 = 700;

/// How often the snapshot is rebuilt while observing, in frames.
/// Positions are recomputed every frame regardless — this is only
/// about re-asking the renderer what its slots are, which catches
/// rows appearing and vanishing.
const RESNAPSHOT_FRAMES: u32 = 30;

/// The element.
#[derive(Default)]
pub struct TonkIntrospect {
    inner: RefCell<Option<Rc<RefCell<Overlay>>>>,
    listeners: RefCell<Vec<Bound>>,
}

/// A document listener plus the closure owning its JS memory.
struct Bound {
    target: web_sys::EventTarget,
    event: String,
    closure: Closure<dyn FnMut(Event)>,
}

impl Drop for Bound {
    fn drop(&mut self) {
        let _ = self.target.remove_event_listener_with_callback_and_bool(
            &self.event,
            self.closure.as_ref().unchecked_ref(),
            true,
        );
    }
}

/// Everything painted, and the state deciding what to paint.
struct Overlay {
    /// The fixed-position layer every box lives in, inside the
    /// element's shadow root so page CSS cannot reach it.
    layer: Element,
    /// The outline around the tracked display.
    outline: Element,
    /// The corner readout: concept, facet, subject and slot counts.
    hud: Element,
    machine: Machine,
    /// Displays seen so far, indexed by [`TargetId`]. Only grows
    /// while Alt is held over new displays.
    targets: Vec<Element>,
    /// What is currently painted.
    painted: Option<Painted>,
    /// `requestAnimationFrame` handle while the loop runs.
    frame: Option<i32>,
    /// The rAF callback, kept alive for as long as the overlay is.
    tick: Option<Function>,
    /// Frames since the snapshot was last rebuilt.
    age: u32,
    /// Whether anything is currently drawn. Together with the
    /// machine's phase this is what keeps an idle pointer free: a
    /// `mousemove` with Alt up over a page that has nothing painted
    /// asks for no frame at all, so it costs the `altKey` read and
    /// nothing else.
    painting: bool,
}

/// The painted state for one observed display.
struct Painted {
    target: TargetId,
    /// One box per slot, with the node it tracks. The `Slot` is kept
    /// so a rebuild can tell whether anything actually changed shape.
    boxes: Vec<(Element, Node, Slot)>,
    /// A cheap fingerprint of the slot set, so an unchanged snapshot
    /// reuses its boxes and keeps their identity (and any running
    /// animation) intact.
    signature: String,
}

impl CustomElement for TonkIntrospect {
    fn shadow() -> bool {
        false
    }

    fn observed_attributes() -> &'static [&'static str] {
        &[]
    }

    fn inject_children(&mut self, _this: &HtmlElement) {}

    fn connected_callback(&mut self, this: &HtmlElement) {
        let host: Element = this.clone().into();
        let Some(document) = window().and_then(|w| w.document()) else {
            return;
        };
        let Some(overlay) = Overlay::build(&host, &document) else {
            return;
        };
        let overlay = Rc::new(RefCell::new(overlay));
        *self.inner.borrow_mut() = Some(overlay.clone());
        install_tick(&overlay);
        *self.listeners.borrow_mut() = install_listeners(&document, &overlay);
    }

    fn disconnected_callback(&mut self, _this: &HtmlElement) {
        self.listeners.borrow_mut().clear();
        if let Some(overlay) = self.inner.borrow_mut().take() {
            overlay.borrow_mut().clear();
        }
        registry::set_armed(false);
    }

    fn attribute_changed_callback(
        &mut self,
        _this: &HtmlElement,
        _name: String,
        _old: Option<String>,
        _new: Option<String>,
    ) {
    }
}

/// Register `<tonk-introspect>` and mount one into the document.
///
/// Mounting is automatic because the whole point is that the hood
/// opens without preparation — you hold Alt over something that looks
/// wrong, in whatever frame it happens to be rendering in. Remove the
/// element to opt out; nothing re-adds it.
pub fn register() {
    let Some(win) = window() else {
        return;
    };
    if win.custom_elements().get(NAME).is_undefined() {
        TonkIntrospect::define(NAME);
    }
    let Some(document) = win.document() else {
        return;
    };
    if document.query_selector(NAME).ok().flatten().is_some() {
        return;
    }
    let Some(body) = document.body() else {
        return;
    };
    if let Ok(element) = document.create_element(NAME) {
        let _ = body.append_child(&element);
    }
}

/// The element name, in one place.
const NAME: &str = "tonk-introspect";

impl Overlay {
    fn build(host: &Element, document: &Document) -> Option<Self> {
        let root = host.shadow_root().or_else(|| {
            host.attach_shadow(&web_sys::ShadowRootInit::new(ShadowRootMode::Open))
                .ok()
        })?;
        let style = document.create_element("style").ok()?;
        style.set_text_content(Some(CSS));
        let _ = root.append_child(&style);

        let layer = document.create_element("div").ok()?;
        let _ = layer.set_attribute("part", "layer");
        let _ = layer.set_attribute("class", "layer");
        let outline = document.create_element("div").ok()?;
        let _ = outline.set_attribute("class", "outline");
        let hud = document.create_element("div").ok()?;
        let _ = hud.set_attribute("class", "hud");
        let _ = layer.append_child(&outline);
        let _ = layer.append_child(&hud);
        let _ = root.append_child(&layer);

        Some(Self {
            layer,
            outline,
            hud,
            machine: Machine::default(),
            targets: Vec::new(),
            painted: None,
            frame: None,
            tick: None,
            age: 0,
            painting: false,
        })
    }

    /// The id for `host`, assigning one if this is the first sighting.
    fn target_of(&mut self, host: &Element) -> TargetId {
        if let Some(index) = self
            .targets
            .iter()
            .position(|seen| seen.is_same_node(Some(host.as_ref())))
        {
            return index as TargetId;
        }
        self.targets.push(host.clone());
        (self.targets.len() - 1) as TargetId
    }

    fn element(&self, target: TargetId) -> Option<&Element> {
        self.targets.get(target as usize)
    }

    /// Tear every painted thing down and disarm.
    fn clear(&mut self) {
        if !self.painting {
            return;
        }
        self.painting = false;
        self.drop_painted();
        // Nothing holds a `TargetId` now, so the table can be
        // renumbered: drop the displays that have since detached
        // rather than keep their subtrees alive for the session.
        self.targets.retain(|target| target.is_connected());
        let _ = self.outline.set_attribute("style", "display:none");
        let _ = self.hud.set_attribute("style", "display:none");
        registry::set_armed(false);
        if let Some(handle) = self.frame.take()
            && let Some(win) = window()
        {
            let _ = win.cancel_animation_frame(handle);
        }
    }

    fn drop_painted(&mut self) {
        if let Some(painted) = self.painted.take() {
            for (element, _, _) in painted.boxes {
                element.remove();
            }
        }
    }
}

/// A slot set's fingerprint: enough to notice a row appearing, a
/// binding changing shape, or the observed display swapping template.
fn signature(slots: &[(Slot, Option<Node>)]) -> String {
    let mut out = String::new();
    for (slot, _) in slots {
        out.push_str(&slot.label());
        out.push('\u{1f}');
    }
    out
}

fn install_listeners(document: &Document, overlay: &Rc<RefCell<Overlay>>) -> Vec<Bound> {
    let mut bound = Vec::new();
    let target: web_sys::EventTarget = document.clone().into();

    bound.push(listen(&target, "mousemove", overlay, |overlay, event| {
        let Some(mouse) = event.dyn_ref::<MouseEvent>() else {
            return;
        };
        let over = display_under(mouse).map(|host| overlay.borrow_mut().target_of(&host));
        let input = Input::Pointer {
            alt: mouse.alt_key(),
            over,
            at: js_sys::Date::now(),
        };
        overlay.borrow_mut().machine.apply(input);
    }));

    // Alt-click pins the observation. Swallowed in the capture phase
    // so the view's own click handler does not also fire — inspecting
    // a button must never dispatch the command that button carries.
    bound.push(listen(&target, "click", overlay, |overlay, event| {
        let Some(mouse) = event.dyn_ref::<MouseEvent>() else {
            return;
        };
        if !mouse.alt_key() {
            return;
        }
        let Some(host) = display_under(mouse) else {
            return;
        };
        event.prevent_default();
        event.stop_propagation();
        let over = overlay.borrow_mut().target_of(&host);
        overlay.borrow_mut().machine.apply(Input::Toggle { over });
    }));

    bound.push(listen(&target, "keyup", overlay, |overlay, event| {
        let Some(key) = event.dyn_ref::<KeyboardEvent>() else {
            return;
        };
        if key.key() == "Alt" {
            overlay.borrow_mut().machine.apply(Input::AltReleased);
        }
    }));

    bound.push(listen(&target, "keydown", overlay, |overlay, event| {
        let Some(key) = event.dyn_ref::<KeyboardEvent>() else {
            return;
        };
        if key.key() == "Escape" {
            overlay.borrow_mut().machine.apply(Input::Clear);
        }
    }));

    bound
}

fn listen(
    target: &web_sys::EventTarget,
    event: &str,
    overlay: &Rc<RefCell<Overlay>>,
    handler: impl Fn(&Rc<RefCell<Overlay>>, &Event) + 'static,
) -> Bound {
    let overlay = overlay.clone();
    let closure = Closure::wrap(Box::new(move |event: Event| {
        handler(&overlay, &event);
        schedule(&overlay);
    }) as Box<dyn FnMut(Event)>);
    let _ = target.add_event_listener_with_callback_and_bool(
        event,
        closure.as_ref().unchecked_ref(),
        true,
    );
    Bound {
        target: target.clone(),
        event: event.to_owned(),
        closure,
    }
}

/// The `<tonk-display>` under a pointer event, if any. `closest`
/// gives the innermost one, which is the right answer: a display
/// nested inside another's template is its own thing to inspect.
fn display_under(event: &MouseEvent) -> Option<Element> {
    event
        .target()
        .and_then(|target| target.dyn_into::<Element>().ok())
        .and_then(|element| element.closest("tonk-display").ok().flatten())
}

/// Build the rAF callback once and stash it on the overlay.
fn install_tick(overlay: &Rc<RefCell<Overlay>>) {
    let weak = Rc::downgrade(overlay);
    let closure = Closure::wrap(Box::new(move || {
        let Some(overlay) = weak.upgrade() else {
            return;
        };
        overlay.borrow_mut().frame = None;
        paint(&overlay);
    }) as Box<dyn FnMut()>);
    let function: Function = closure.as_ref().unchecked_ref::<Function>().clone();
    closure.forget();
    overlay.borrow_mut().tick = Some(function);
}

/// Ask for a frame, unless one is already pending.
fn schedule(overlay: &Rc<RefCell<Overlay>>) {
    let Some(win) = window() else {
        return;
    };
    let mut state = overlay.borrow_mut();
    if state.frame.is_some() {
        return;
    }
    // Nothing tracked and nothing drawn — there is no frame to paint.
    if state.machine.highlighted().is_none() && !state.painting {
        return;
    }
    let Some(tick) = state.tick.clone() else {
        return;
    };
    state.frame = win.request_animation_frame(tick.unchecked_ref()).ok();
}

/// One frame: advance the dwell timer, reconcile what is painted with
/// what the machine wants painted, and reposition everything.
fn paint(overlay: &Rc<RefCell<Overlay>>) {
    {
        let mut state = overlay.borrow_mut();
        let at = js_sys::Date::now();
        state.machine.apply(Input::Tick { at });
    }

    let (highlighted, observed) = {
        let state = overlay.borrow();
        (state.machine.highlighted(), state.machine.observed())
    };

    registry::set_armed(observed.is_some());

    if highlighted.is_none() {
        overlay.borrow_mut().clear();
        return;
    }
    overlay.borrow_mut().painting = true;

    paint_outline(overlay, highlighted);
    match observed {
        Some(target) => paint_observation(overlay, target),
        None => {
            overlay.borrow_mut().drop_painted();
            let _ = overlay.borrow().hud.set_attribute("style", "display:none");
        }
    }

    // Keep the loop alive while anything is tracked: positions follow
    // scrolling and layout, and a latched observation has no pointer
    // events to drive it.
    schedule(overlay);
}

fn paint_outline(overlay: &Rc<RefCell<Overlay>>, target: Option<TargetId>) {
    let state = overlay.borrow();
    let Some(element) = target.and_then(|target| state.element(target)) else {
        let _ = state.outline.set_attribute("style", "display:none");
        return;
    };
    let rect = element.get_bounding_client_rect();
    let _ = state.outline.set_attribute(
        "style",
        &format!(
            "display:block;left:{}px;top:{}px;width:{}px;height:{}px",
            rect.left(),
            rect.top(),
            rect.width(),
            rect.height()
        ),
    );
    let latched = state.machine.is_latched();
    let _ = state
        .outline
        .set_attribute("class", if latched { "outline pinned" } else { "outline" });
}

fn paint_observation(overlay: &Rc<RefCell<Overlay>>, target: TargetId) {
    let host = {
        let state = overlay.borrow();
        state.element(target).cloned()
    };
    let Some(host) = host else {
        return;
    };

    let stale = {
        let state = overlay.borrow();
        state.age >= RESNAPSHOT_FRAMES
            || state
                .painted
                .as_ref()
                .is_none_or(|painted| painted.target != target)
    };

    if stale {
        rebuild(overlay, target, &host);
    } else {
        overlay.borrow_mut().age += 1;
    }

    reposition(overlay);
    flash_changes(overlay);
}

/// Re-ask the renderer for its slots and rebuild the boxes if the set
/// has actually changed shape.
fn rebuild(overlay: &Rc<RefCell<Overlay>>, target: TargetId, host: &Element) {
    let slots = registry::slots_under(host);
    let facts = registry::display_facts(host);
    let signature = signature(&slots);

    {
        let mut state = overlay.borrow_mut();
        state.age = 0;
        if state
            .painted
            .as_ref()
            .is_some_and(|painted| painted.target == target && painted.signature == signature)
        {
            return;
        }
    }

    let Some(document) = window().and_then(|w| w.document()) else {
        return;
    };

    let mut boxes = Vec::new();
    for (slot, node) in slots {
        let Some(node) = node else {
            continue;
        };
        let Ok(element) = document.create_element("div") else {
            continue;
        };
        let _ = element.set_attribute("class", &format!("slot {}", origin_class(slot.origin)));
        let label = document.create_element("span");
        if let Ok(label) = label {
            let _ = label.set_attribute("class", "tag");
            label.set_text_content(Some(&slot.label()));
            let _ = element.append_child(&label);
        }
        let _ = overlay.borrow().layer.append_child(&element);
        boxes.push((element, node, slot));
    }

    let mut state = overlay.borrow_mut();
    state.drop_painted();
    let count = boxes.len();
    state.painted = Some(Painted {
        target,
        boxes,
        signature,
    });
    write_hud(&state.hud, facts.as_ref(), count);
}

fn write_hud(hud: &Element, facts: Option<&super::slot::Snapshot>, slots: usize) {
    let Some(facts) = facts else {
        let _ = hud.set_attribute("style", "display:none");
        return;
    };
    let model = facts
        .model
        .clone()
        .or_else(|| facts.model_entity.clone())
        .unwrap_or_else(|| "?".to_owned());
    let facet = facts.facet.clone().unwrap_or_else(|| "?".to_owned());
    let mode = if facts.directory {
        "directory"
    } else {
        "detail"
    };
    let mut text = format!(
        "{model} · {facet} · {mode} · {} subject(s) · {slots} slot(s)",
        facts.subjects.len()
    );
    let unbound = facts.unbound_fields();
    if !unbound.is_empty() {
        text.push_str(&format!("\nunrendered: {}", unbound.join(", ")));
    }
    let undeclared = facts.undeclared_fields();
    if !undeclared.is_empty() {
        text.push_str(&format!("\nnot on the concept: {}", undeclared.join(", ")));
    }
    hud.set_text_content(Some(&text));
    let _ = hud.set_attribute("style", "display:block");
}

/// Move every slot box onto its node's current rect.
fn reposition(overlay: &Rc<RefCell<Overlay>>) {
    let state = overlay.borrow();
    let Some(painted) = state.painted.as_ref() else {
        return;
    };
    for (element, node, slot) in &painted.boxes {
        match rect_of(node, slot) {
            Some((left, top, width, height)) => {
                let _ = element.set_attribute(
                    "style",
                    &format!(
                        "display:block;left:{left}px;top:{top}px;width:{width}px;height:{height}px"
                    ),
                );
            }
            None => {
                let _ = element.set_attribute("style", "display:none");
            }
        }
    }
}

/// The viewport rect a slot occupies.
///
/// A text slot has no element of its own, so its box comes from a
/// `Range` over the text node — which is also why a slot that
/// rendered an empty string has no box at all and is hidden rather
/// than drawn as a hairline. An attribute slot has no region in
/// principle; it borrows its element's, which is the honest answer to
/// "where did `with={repo}` land".
fn rect_of(node: &Node, slot: &Slot) -> Option<(f64, f64, f64, f64)> {
    let rect = match (&slot.kind, node.dyn_ref::<Element>()) {
        (SlotKind::Attribute { .. }, Some(element)) => element.get_bounding_client_rect(),
        _ => {
            let range = Range::new().ok()?;
            range.select_node_contents(node).ok()?;
            range.get_bounding_client_rect()
        }
    };
    if rect.width() <= 0.0 && rect.height() <= 0.0 {
        return None;
    }
    Some((rect.left(), rect.top(), rect.width(), rect.height()))
}

/// Drain the renderer's change queue and flash where each write
/// landed. A flash is its own transient element rather than a class
/// on the slot box, so a value that changes twice in quick succession
/// shows twice instead of restarting one animation.
fn flash_changes(overlay: &Rc<RefCell<Overlay>>) {
    let changed = registry::drain_changes();
    if changed.is_empty() {
        return;
    }
    let Some(win) = window() else {
        return;
    };
    let Some(document) = win.document() else {
        return;
    };
    let layer = overlay.borrow().layer.clone();
    for node in changed {
        let Some((left, top, width, height)) = rect_of_node(&node) else {
            continue;
        };
        let Ok(element) = document.create_element("div") else {
            continue;
        };
        let _ = element.set_attribute("class", "flash");
        let _ = element.set_attribute(
            "style",
            &format!("left:{left}px;top:{top}px;width:{width}px;height:{height}px"),
        );
        let _ = layer.append_child(&element);
        let doomed = element.clone();
        let retire = Closure::once_into_js(move || {
            doomed.remove();
        });
        let _ = win.set_timeout_with_callback_and_timeout_and_arguments_0(
            retire.unchecked_ref(),
            FLASH_MS,
        );
    }
}

/// The rect of a node whose slot kind is not known — element if it is
/// one, otherwise the text range.
fn rect_of_node(node: &Node) -> Option<(f64, f64, f64, f64)> {
    let rect = match node.dyn_ref::<Element>() {
        Some(element) => element.get_bounding_client_rect(),
        None => {
            let range = Range::new().ok()?;
            range.select_node_contents(node).ok()?;
            range.get_bounding_client_rect()
        }
    };
    if rect.width() <= 0.0 && rect.height() <= 0.0 {
        return None;
    }
    Some((rect.left(), rect.top(), rect.width(), rect.height()))
}

fn origin_class(origin: Origin) -> &'static str {
    match origin {
        Origin::Concept => "from-concept",
        Origin::Subject => "from-subject",
        Origin::Host => "from-host",
        Origin::Key => "from-key",
    }
}

/// Everything the overlay draws. Scoped by the shadow root, so page
/// styles cannot reach it and it cannot reach the page.
const CSS: &str = "\
:host { position: fixed; inset: 0; pointer-events: none; z-index: 2147483000; }
.layer { position: fixed; inset: 0; pointer-events: none; font: 11px/1.4 ui-monospace, monospace; }
.outline { position: fixed; display: none; box-sizing: border-box;
           border: 1px solid color-mix(in srgb, currentColor 30%, #4f8cff);
           background: color-mix(in srgb, #4f8cff 6%, transparent); border-radius: 2px; }
.outline.pinned { border-style: dashed; border-width: 2px; }
.slot { position: fixed; display: none; box-sizing: border-box; border: 1px solid; border-radius: 2px; }
.slot .tag { position: absolute; left: 0; bottom: 100%; padding: 0 3px;
             white-space: nowrap; color: #fff; border-radius: 2px 2px 0 0; }
.slot.from-concept { border-color: #22a06b; background: color-mix(in srgb, #22a06b 10%, transparent); }
.slot.from-concept .tag { background: #22a06b; }
.slot.from-subject { border-color: #8250df; background: color-mix(in srgb, #8250df 10%, transparent); }
.slot.from-subject .tag { background: #8250df; }
.slot.from-host { border-color: #bf8700; background: color-mix(in srgb, #bf8700 10%, transparent); }
.slot.from-host .tag { background: #bf8700; }
.slot.from-key { border-color: #0969da; background: color-mix(in srgb, #0969da 10%, transparent); }
.slot.from-key .tag { background: #0969da; }
.flash { position: fixed; box-sizing: border-box; border: 2px solid #e8590c; border-radius: 2px;
         background: color-mix(in srgb, #e8590c 30%, transparent);
         animation: tonk-introspect-flash 700ms ease-out forwards; }
@keyframes tonk-introspect-flash {
  from { opacity: 1; transform: scale(1.06); }
  to   { opacity: 0; transform: scale(1); }
}
.hud { position: fixed; display: none; right: 8px; bottom: 8px; max-width: 46ch;
       padding: 6px 8px; white-space: pre-wrap; color: #fff; background: rgba(20,20,24,.92);
       border-radius: 3px; }
";
