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
//!
//! ## Pinning without taking a gesture
//!
//! Observation is pinned by clicking the overlay's own pin affordance,
//! not by a modifier-click on the page. A modifier-click would have to
//! be swallowed — inspecting a button must never dispatch the command
//! that button carries — and that means taking the gesture away from
//! every app for as long as the overlay is mounted. The pin is overlay
//! chrome with `pointer-events: auto`, so it costs the page nothing.
//! `<tonk-introspect alt-click>` restores alt-click pinning for a page
//! that wants it and knows what it is giving up.
//!
//! Moving the pointer onto the overlay's own chrome does not count as
//! moving off the display: the machine ignores pointer events whose
//! target retargets to the overlay host, which is what makes reaching
//! for the pin possible at all.
//!
//! ## Marking something with no extent
//!
//! A slot that rendered an empty string has nothing to box, and that
//! is exactly the case an author most wants to see. So a marker is not
//! always a box:
//!
//! - **Extent** — the slot rendered glyphs. Box them.
//! - **Point** — the slot is empty. Find the caret position it would
//!   have occupied (the trailing edge of the previous sibling, the
//!   leading edge of the next, or the parent's content corner) and
//!   draw a tick there.
//! - **Edge** — the slot wrote a property of an element rather than
//!   text. Tick the element's top edge instead of filling it: the
//!   element is where the value went, but the element is not the value.
//!
//! Every marker carries a label badge whatever its placement, so an
//! empty slot is still named. Badges that would collide are pushed
//! down and joined to their anchor by a leader line.

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

use super::command::Command;
use super::mode::{Input, Machine, TargetId};
use super::registry;
use super::slot::{Origin, Slot, SlotKind};

/// How long a change flash or a dispatch bounce stays up.
const FLASH_MS: i32 = 700;

/// How often the snapshot is rebuilt while observing, in frames.
/// Positions are recomputed every frame regardless — this is only
/// about re-asking the renderer what its slots are, which catches rows
/// appearing and vanishing.
const RESNAPSHOT_FRAMES: u32 = 30;

/// The most markers painted at once. A directory of a few hundred rows
/// would otherwise put thousands of elements on the layer and make the
/// frame budget the thing being debugged. The readout says when this
/// bit.
const MARKER_CAP: usize = 160;

/// Badge geometry, in CSS pixels. The font is monospace at 11px, so a
/// label's width is arithmetic rather than a layout read — which keeps
/// the collision pass off the critical path.
const BADGE_HEIGHT: f64 = 13.0;
const BADGE_CHAR: f64 = 6.2;
const BADGE_PADDING: f64 = 6.0;

/// The element name, in one place.
const NAME: &str = "tonk-introspect";

/// Set this attribute on the element to restore alt-click pinning. Off
/// by default: see the note on pinning above.
const ALT_CLICK: &str = "alt-click";

/// The element.
#[derive(Default)]
pub struct TonkIntrospect {
    inner: RefCell<Option<Rc<RefCell<Overlay>>>>,
    listeners: RefCell<Vec<Bound>>,
}

/// A listener plus the closure owning its JS memory.
struct Bound {
    target: web_sys::EventTarget,
    event: String,
    capture: bool,
    closure: Closure<dyn FnMut(Event)>,
}

impl Drop for Bound {
    fn drop(&mut self) {
        let _ = self.target.remove_event_listener_with_callback_and_bool(
            &self.event,
            self.closure.as_ref().unchecked_ref(),
            self.capture,
        );
    }
}

/// Everything painted, and the state deciding what to paint.
struct Overlay {
    /// The overlay's own host element, so a pointer event that
    /// retargets to it can be told apart from one on the page.
    host: Element,
    /// The fixed-position layer every marker lives in, inside the
    /// element's shadow root so page CSS cannot reach it.
    layer: Element,
    /// The outline around the tracked display.
    outline: Element,
    /// The pin affordance. The one thing on the layer that takes
    /// pointer events.
    pin: Element,
    /// The corner readout.
    hud: Element,
    machine: Machine,
    /// Displays seen so far, indexed by [`TargetId`].
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
    /// asks for no frame at all.
    painting: bool,
}

/// The painted state for one observed display.
struct Painted {
    target: TargetId,
    slots: Vec<Marker>,
    commands: Vec<Marker>,
    /// A fingerprint of what was described, so an unchanged snapshot
    /// reuses its markers and keeps their identity intact.
    signature: String,
}

/// One painted marker: the tick or box on the thing, the badge naming
/// it, and the leader joining them when the badge had to move.
struct Marker {
    mark: Element,
    badge: Element,
    leader: Element,
    /// What the marker tracks. A slot tracks the node it wrote into; a
    /// command tracks the element that binds it.
    anchor: Node,
    /// How to place it, decided by what it is.
    style: MarkerStyle,
    /// The badge text, fixed at build time.
    label: String,
}

/// What kind of thing a marker is tracking.
enum MarkerStyle {
    /// A text slot: box its glyphs, or tick its caret when empty.
    Text,
    /// A slot that wrote an element property: tick the element's edge.
    Property,
    /// A bound interaction: outline the element that binds it.
    Interaction,
}

/// Where a marker goes this frame.
enum Placement {
    /// Box a region.
    Extent {
        left: f64,
        top: f64,
        width: f64,
        height: f64,
    },
    /// Tick a caret position — something with no extent of its own.
    Point { left: f64, top: f64, height: f64 },
    /// Tick an element's top edge.
    Edge { left: f64, top: f64, width: f64 },
    /// Nothing on screen.
    Offscreen,
}

impl Placement {
    /// Where the badge wants to sit, and where its leader would start.
    fn anchor(&self) -> Option<(f64, f64)> {
        match *self {
            Placement::Extent { left, top, .. } | Placement::Point { left, top, .. } => {
                Some((left, top))
            }
            Placement::Edge { left, top, .. } => Some((left, top)),
            Placement::Offscreen => None,
        }
    }
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

impl Overlay {
    fn build(host: &Element, document: &Document) -> Option<Self> {
        let root = host.shadow_root().or_else(|| {
            host.attach_shadow(&web_sys::ShadowRootInit::new(ShadowRootMode::Open))
                .ok()
        })?;
        let style = document.create_element("style").ok()?;
        style.set_text_content(Some(CSS));
        let _ = root.append_child(&style);

        let layer = element(document, "div", "layer")?;
        let outline = element(document, "div", "outline")?;
        let pin = element(document, "button", "pin")?;
        pin.set_text_content(Some("pin"));
        let hud = element(document, "div", "hud")?;
        let _ = layer.append_child(&outline);
        let _ = layer.append_child(&pin);
        let _ = layer.append_child(&hud);
        let _ = root.append_child(&layer);

        Some(Self {
            host: host.clone(),
            layer,
            outline,
            pin,
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

    /// Whether this page asked for alt-click pinning as well.
    fn alt_click_pins(&self) -> bool {
        self.host.has_attribute(ALT_CLICK)
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
        hide(&self.outline);
        hide(&self.pin);
        hide(&self.hud);
        registry::set_armed(false);
        // Nothing holds a `TargetId` now, so the table can be
        // renumbered: drop the displays that have since detached
        // rather than keep their subtrees alive for the session.
        self.targets.retain(|target| target.is_connected());
        if let Some(handle) = self.frame.take()
            && let Some(win) = window()
        {
            let _ = win.cancel_animation_frame(handle);
        }
    }

    fn drop_painted(&mut self) {
        if let Some(painted) = self.painted.take() {
            for marker in painted.slots.into_iter().chain(painted.commands) {
                marker.mark.remove();
                marker.badge.remove();
                marker.leader.remove();
            }
        }
    }
}

fn element(document: &Document, tag: &str, class: &str) -> Option<Element> {
    let element = document.create_element(tag).ok()?;
    let _ = element.set_attribute("class", class);
    Some(element)
}

fn hide(element: &Element) {
    let _ = element.set_attribute("style", "display:none");
}

fn install_listeners(document: &Document, overlay: &Rc<RefCell<Overlay>>) -> Vec<Bound> {
    let mut bound = Vec::new();
    let target: web_sys::EventTarget = document.clone().into();

    bound.push(listen(
        &target,
        "mousemove",
        true,
        overlay,
        |overlay, event| {
            let Some(mouse) = event.dyn_ref::<MouseEvent>() else {
                return;
            };
            // Reaching for the overlay's own chrome is not leaving the
            // display. Without this the outline vanishes the moment the
            // pointer crosses onto the pin.
            if on_overlay(overlay, event) {
                return;
            }
            let over = display_under(mouse).map(|host| overlay.borrow_mut().target_of(&host));
            let input = Input::Pointer {
                alt: mouse.alt_key(),
                over,
                at: js_sys::Date::now(),
            };
            overlay.borrow_mut().machine.apply(input);
        },
    ));

    // Alt-click pinning, only for a page that asked for it. It has to
    // be swallowed in the capture phase — inspecting a button must
    // never dispatch the command that button carries — which is
    // precisely why it is not the default.
    bound.push(listen(&target, "click", true, overlay, |overlay, event| {
        if !overlay.borrow().alt_click_pins() {
            return;
        }
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

    bound.push(listen(&target, "keyup", true, overlay, |overlay, event| {
        let Some(key) = event.dyn_ref::<KeyboardEvent>() else {
            return;
        };
        if key.key() == "Alt" {
            overlay.borrow_mut().machine.apply(Input::AltReleased);
        }
    }));

    bound.push(listen(
        &target,
        "keydown",
        true,
        overlay,
        |overlay, event| {
            let Some(key) = event.dyn_ref::<KeyboardEvent>() else {
                return;
            };
            if key.key() == "Escape" {
                overlay.borrow_mut().machine.apply(Input::Clear);
            }
        },
    ));

    // The pin itself. Bubble phase on the button, so nothing on the
    // page is involved at all.
    let pin = overlay.borrow().pin.clone();
    bound.push(listen(
        pin.as_ref(),
        "click",
        false,
        overlay,
        |overlay, event| {
            event.stop_propagation();
            let over = overlay.borrow().machine.highlighted();
            if let Some(over) = over {
                overlay.borrow_mut().machine.apply(Input::Toggle { over });
            }
        },
    ));

    bound
}

/// Whether an event landed on the overlay's own chrome. Shadow content
/// retargets to the host, so comparing against the host covers the pin
/// and everything else on the layer.
fn on_overlay(overlay: &Rc<RefCell<Overlay>>, event: &Event) -> bool {
    let Some(target) = event.target().and_then(|t| t.dyn_into::<Node>().ok()) else {
        return false;
    };
    target.is_same_node(Some(overlay.borrow().host.as_ref()))
}

fn listen(
    target: &web_sys::EventTarget,
    event: &str,
    capture: bool,
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
        capture,
    );
    Bound {
        target: target.clone(),
        event: event.to_owned(),
        capture,
        closure,
    }
}

/// The `<tonk-display>` under a pointer event, if any. `closest` gives
/// the innermost one, which is the right answer: a display nested
/// inside another's template is its own thing to inspect.
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

    paint_frame(overlay, highlighted);
    match observed {
        Some(target) => paint_observation(overlay, target),
        None => {
            overlay.borrow_mut().drop_painted();
            hide(&overlay.borrow().hud);
        }
    }

    // Keep the loop alive while anything is tracked: positions follow
    // scrolling and layout, and a pinned observation has no pointer
    // events to drive it.
    schedule(overlay);
}

/// The outline around the tracked display, and the pin hanging off it.
fn paint_frame(overlay: &Rc<RefCell<Overlay>>, target: Option<TargetId>) {
    let state = overlay.borrow();
    let Some(element) = target.and_then(|target| state.element(target)) else {
        hide(&state.outline);
        hide(&state.pin);
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

    // The pin sits above the outline's top-left, or just inside when
    // the display is against the top of the viewport.
    let top = if rect.top() >= BADGE_HEIGHT + 2.0 {
        rect.top() - BADGE_HEIGHT - 2.0
    } else {
        rect.top() + 2.0
    };
    state
        .pin
        .set_text_content(Some(if latched { "pinned" } else { "pin" }));
    let _ = state
        .pin
        .set_attribute("class", if latched { "pin pinned" } else { "pin" });
    let _ = state.pin.set_attribute(
        "style",
        &format!("display:block;left:{}px;top:{top}px", rect.left()),
    );
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
    bounce_dispatches(overlay);
}

/// Re-ask the display and its renderer what is there, and rebuild the
/// markers if the set has actually changed shape.
fn rebuild(overlay: &Rc<RefCell<Overlay>>, target: TargetId, host: &Element) {
    let slots = registry::slots_under(host);
    let commands = registry::commands_under(host);
    let facts = registry::display_facts(host);
    let signature = signature(&slots, &commands);

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
    let layer = overlay.borrow().layer.clone();

    let mut budget = MARKER_CAP;
    let mut slot_markers = Vec::new();
    for (slot, node) in slots {
        if budget == 0 {
            break;
        }
        let Some(node) = node else {
            continue;
        };
        let style = match slot.kind {
            SlotKind::Text => MarkerStyle::Text,
            SlotKind::Attribute { .. } => MarkerStyle::Property,
        };
        let classes = format!("mark slot {}", origin_class(slot.origin));
        if let Some(marker) = build_marker(&document, &layer, &classes, slot.label(), node, style) {
            slot_markers.push(marker);
            budget -= 1;
        }
    }

    let mut command_markers = Vec::new();
    for (command, element) in commands {
        if budget == 0 {
            break;
        }
        let classes = if command.is_live() {
            "mark command".to_owned()
        } else {
            "mark command inert".to_owned()
        };
        if let Some(marker) = build_marker(
            &document,
            &layer,
            &classes,
            command.label(),
            element.into(),
            MarkerStyle::Interaction,
        ) {
            command_markers.push(marker);
            budget -= 1;
        }
    }

    let truncated = budget == 0;
    let mut state = overlay.borrow_mut();
    state.drop_painted();
    let counts = (slot_markers.len(), command_markers.len());
    state.painted = Some(Painted {
        target,
        slots: slot_markers,
        commands: command_markers,
        signature,
    });
    write_hud(&state.hud, facts.as_ref(), counts, truncated);
}

fn build_marker(
    document: &Document,
    layer: &Element,
    classes: &str,
    label: String,
    anchor: Node,
    style: MarkerStyle,
) -> Option<Marker> {
    let mark = element(document, "div", classes)?;
    let badge = element(document, "div", &classes.replace("mark", "badge"))?;
    badge.set_text_content(Some(&label));
    let leader = element(document, "div", &classes.replace("mark", "leader"))?;
    let _ = layer.append_child(&mark);
    let _ = layer.append_child(&leader);
    let _ = layer.append_child(&badge);
    Some(Marker {
        mark,
        badge,
        leader,
        anchor,
        style,
        label,
    })
}

/// A fingerprint of what is being shown: enough to notice a row
/// appearing, a binding changing shape, or the display swapping
/// template.
fn signature(slots: &[(Slot, Option<Node>)], commands: &[(Command, Element)]) -> String {
    let mut out = String::new();
    for (slot, _) in slots {
        out.push_str(&slot.label());
        out.push('\u{1f}');
    }
    out.push('\u{1e}');
    for (command, _) in commands {
        out.push_str(&command.label());
        out.push('\u{1f}');
    }
    out
}

fn write_hud(
    hud: &Element,
    facts: Option<&super::slot::Snapshot>,
    counts: (usize, usize),
    truncated: bool,
) {
    let Some(facts) = facts else {
        hide(hud);
        return;
    };
    let (slots, commands) = counts;
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
        "{model} · {facet} · {mode} · {} subject(s) · {slots} slot(s) · {commands} command(s)",
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
    if truncated {
        text.push_str(&format!("\nshowing the first {MARKER_CAP} markers"));
    }
    hud.set_text_content(Some(&text));
    let _ = hud.set_attribute("style", "display:block");
}

/// Place every marker on its anchor's current geometry, then lay the
/// badges out so they do not sit on top of each other.
fn reposition(overlay: &Rc<RefCell<Overlay>>) {
    let state = overlay.borrow();
    let Some(painted) = state.painted.as_ref() else {
        return;
    };
    // Badges already placed this frame, as (left, top, right). A new
    // badge that would overlap one is pushed below it and joined to
    // its anchor by a leader.
    let mut placed: Vec<(f64, f64, f64)> = Vec::new();
    for marker in painted.slots.iter().chain(painted.commands.iter()) {
        let placement = place(&marker.anchor, &marker.style);
        apply_mark(marker, &placement);
        apply_badge(marker, &placement, &mut placed);
    }
}

fn apply_mark(marker: &Marker, placement: &Placement) {
    match *placement {
        Placement::Extent {
            left,
            top,
            width,
            height,
        } => {
            let _ = marker.mark.set_attribute(
                "style",
                &format!(
                    "display:block;left:{left}px;top:{top}px;width:{width}px;height:{height}px"
                ),
            );
            let _ = marker.mark.set_attribute("data-shape", "extent");
        }
        Placement::Point { left, top, height } => {
            let _ = marker.mark.set_attribute(
                "style",
                &format!("display:block;left:{left}px;top:{top}px;width:2px;height:{height}px"),
            );
            let _ = marker.mark.set_attribute("data-shape", "point");
        }
        Placement::Edge { left, top, width } => {
            let _ = marker.mark.set_attribute(
                "style",
                &format!("display:block;left:{left}px;top:{top}px;width:{width}px;height:2px"),
            );
            let _ = marker.mark.set_attribute("data-shape", "edge");
        }
        Placement::Offscreen => {
            hide(&marker.mark);
            hide(&marker.badge);
            hide(&marker.leader);
        }
    }
}

fn apply_badge(marker: &Marker, placement: &Placement, placed: &mut Vec<(f64, f64, f64)>) {
    let Some((anchor_left, anchor_top)) = placement.anchor() else {
        return;
    };
    let width = marker.label.chars().count() as f64 * BADGE_CHAR + BADGE_PADDING;
    // Above the anchor by default, which is where a label reads
    // without covering the thing it names.
    let mut top = anchor_top - BADGE_HEIGHT - 1.0;
    if top < 0.0 {
        top = anchor_top + 1.0;
    }
    let right = anchor_left + width;
    while placed.iter().any(|(other_left, other_top, other_right)| {
        (top - other_top).abs() < BADGE_HEIGHT && anchor_left < *other_right && right > *other_left
    }) {
        top += BADGE_HEIGHT + 1.0;
    }
    placed.push((anchor_left, top, right));

    let _ = marker.badge.set_attribute(
        "style",
        &format!("display:block;left:{anchor_left}px;top:{top}px"),
    );

    // A badge that stayed put needs no leader; one that was pushed
    // down gets a dashed line back to what it names.
    let settled = top + BADGE_HEIGHT + 1.0;
    if (settled - anchor_top).abs() < 1.5 {
        hide(&marker.leader);
        return;
    }
    let (line_top, line_height) = if top > anchor_top {
        (anchor_top, top - anchor_top)
    } else {
        (settled, anchor_top - settled)
    };
    let _ = marker.leader.set_attribute(
        "style",
        &format!("display:block;left:{anchor_left}px;top:{line_top}px;height:{line_height}px"),
    );
}

/// Decide where a marker goes from what its anchor currently measures.
fn place(anchor: &Node, style: &MarkerStyle) -> Placement {
    match style {
        MarkerStyle::Interaction => match anchor.dyn_ref::<Element>() {
            Some(element) => from_rect(&element.get_bounding_client_rect())
                .map(|(left, top, width, height)| Placement::Extent {
                    left,
                    top,
                    width,
                    height,
                })
                .unwrap_or(Placement::Offscreen),
            None => Placement::Offscreen,
        },
        // The element is where the value went, but the element is not
        // the value — so tick its edge rather than filling it, which
        // would read as "this whole region is the value".
        MarkerStyle::Property => match anchor.dyn_ref::<Element>() {
            Some(element) => from_rect(&element.get_bounding_client_rect())
                .map(|(left, top, width, _)| Placement::Edge { left, top, width })
                .unwrap_or(Placement::Offscreen),
            None => Placement::Offscreen,
        },
        MarkerStyle::Text => match text_extent(anchor) {
            Some((left, top, width, height)) => Placement::Extent {
                left,
                top,
                width,
                height,
            },
            // No glyphs: the value is empty, which is the case an
            // author most wants to see. Point at where it would be.
            None => caret(anchor)
                .map(|(left, top, height)| Placement::Point { left, top, height })
                .unwrap_or(Placement::Offscreen),
        },
    }
}

/// The box a text node's glyphs occupy, or `None` when it has none.
fn text_extent(node: &Node) -> Option<(f64, f64, f64, f64)> {
    let range = Range::new().ok()?;
    range.select_node_contents(node).ok()?;
    from_rect(&range.get_bounding_client_rect())
}

/// Where an empty text node's value would appear.
///
/// The caret position is the trailing edge of whatever precedes it,
/// else the leading edge of whatever follows, else the parent's
/// content corner. `<p>Hello {name}</p>` with `name` absent therefore
/// ticks immediately after `Hello `, which is the answer a reader
/// wants: not "this is missing somewhere" but "it would be here".
fn caret(node: &Node) -> Option<(f64, f64, f64)> {
    if let Some(previous) = node.previous_sibling()
        && let Some((left, top, width, height)) = extent_of(&previous)
    {
        return Some((left + width, top, height));
    }
    if let Some(next) = node.next_sibling()
        && let Some((left, top, _, height)) = extent_of(&next)
    {
        return Some((left, top, height));
    }
    let parent = node.parent_element()?;
    let (left, top, _, height) = from_rect(&parent.get_bounding_client_rect())?;
    Some((left, top, height.min(16.0)))
}

/// The box any node occupies — element rect or text range.
fn extent_of(node: &Node) -> Option<(f64, f64, f64, f64)> {
    match node.dyn_ref::<Element>() {
        Some(element) => from_rect(&element.get_bounding_client_rect()),
        None => text_extent(node),
    }
}

/// A rect, unless it is degenerate.
fn from_rect(rect: &web_sys::DomRect) -> Option<(f64, f64, f64, f64)> {
    if rect.width() <= 0.0 && rect.height() <= 0.0 {
        return None;
    }
    Some((rect.left(), rect.top(), rect.width(), rect.height()))
}

/// Drain the renderer's change queue and flash where each write
/// landed. A flash is its own transient element rather than a class on
/// the marker, so a value that changes twice in quick succession shows
/// twice instead of restarting one animation.
fn flash_changes(overlay: &Rc<RefCell<Overlay>>) {
    for node in registry::drain_changes() {
        let placement = extent_of(&node)
            .or_else(|| caret(&node).map(|(left, top, height)| (left, top, 2.0, height)));
        if let Some((left, top, width, height)) = placement {
            transient(overlay, "flash", left, top, width.max(2.0), height);
        }
    }
}

/// Drain the dispatch queue and bounce the element that posted.
fn bounce_dispatches(overlay: &Rc<RefCell<Overlay>>) {
    for (element, _command) in registry::drain_dispatches() {
        if let Some((left, top, width, height)) = from_rect(&element.get_bounding_client_rect()) {
            transient(overlay, "bounce", left, top, width, height);
        }
    }
}

/// Drop a self-retiring marker on the layer.
fn transient(overlay: &Rc<RefCell<Overlay>>, class: &str, left: f64, top: f64, w: f64, h: f64) {
    let Some(win) = window() else {
        return;
    };
    let Some(document) = win.document() else {
        return;
    };
    let Some(marker) = element(&document, "div", class) else {
        return;
    };
    let _ = marker.set_attribute(
        "style",
        &format!("left:{left}px;top:{top}px;width:{w}px;height:{h}px"),
    );
    let _ = overlay.borrow().layer.append_child(&marker);
    let doomed = marker.clone();
    let retire = Closure::once_into_js(move || {
        doomed.remove();
    });
    let _ =
        win.set_timeout_with_callback_and_timeout_and_arguments_0(retire.unchecked_ref(), FLASH_MS);
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
.layer { position: fixed; inset: 0; pointer-events: none;
         font: 11px/1.2 ui-monospace, SFMono-Regular, Menlo, monospace; }
.outline { position: fixed; display: none; box-sizing: border-box;
           border: 1px solid #4f8cff; background: color-mix(in srgb, #4f8cff 5%, transparent);
           border-radius: 2px; }
.outline.pinned { border-style: dashed; border-width: 2px; }
.pin { position: fixed; display: none; pointer-events: auto; cursor: pointer;
       height: 13px; padding: 0 5px; font: inherit; line-height: 13px; color: #fff;
       background: #4f8cff; border: 0; border-radius: 2px 2px 0 0; }
.pin.pinned { background: #1f6feb; }
.mark { position: fixed; display: none; box-sizing: border-box; border-radius: 1px; }
.mark[data-shape=extent] { border: 1px solid var(--ink); background: var(--wash); }
.mark[data-shape=point] { background: var(--ink); }
.mark[data-shape=edge] { background: var(--ink); }
.badge { position: fixed; display: none; height: 13px; line-height: 13px; padding: 0 3px;
         white-space: nowrap; color: #fff; background: var(--ink); border-radius: 2px; }
.leader { position: fixed; display: none; width: 0; border-left: 1px dashed var(--ink); }
.from-concept { --ink: #22a06b; --wash: color-mix(in srgb, #22a06b 10%, transparent); }
.from-subject { --ink: #8250df; --wash: color-mix(in srgb, #8250df 10%, transparent); }
.from-host    { --ink: #bf8700; --wash: color-mix(in srgb, #bf8700 10%, transparent); }
.from-key     { --ink: #0969da; --wash: color-mix(in srgb, #0969da 10%, transparent); }
.command      { --ink: #d6336c; --wash: color-mix(in srgb, #d6336c 8%, transparent); }
.command.inert { --ink: #c92a2a; }
.mark.command[data-shape=extent] { border-style: dashed; }
.mark.command.inert[data-shape=extent] { border-style: dotted; border-width: 2px; }
.flash { position: fixed; box-sizing: border-box; border: 2px solid #e8590c; border-radius: 2px;
         background: color-mix(in srgb, #e8590c 30%, transparent);
         animation: tonk-introspect-flash 700ms ease-out forwards; }
.bounce { position: fixed; box-sizing: border-box; border: 2px solid #d6336c; border-radius: 3px;
          background: color-mix(in srgb, #d6336c 22%, transparent);
          animation: tonk-introspect-bounce 700ms cubic-bezier(.2,.9,.3,1) forwards; }
@keyframes tonk-introspect-flash {
  from { opacity: 1; transform: scale(1.06); }
  to   { opacity: 0; transform: scale(1); }
}
@keyframes tonk-introspect-bounce {
  0%   { opacity: 1; transform: scale(1); }
  35%  { opacity: 1; transform: scale(1.09); }
  to   { opacity: 0; transform: scale(1); }
}
.hud { position: fixed; display: none; right: 8px; bottom: 8px; max-width: 52ch;
       padding: 6px 8px; white-space: pre-wrap; line-height: 1.4; color: #fff;
       background: rgba(20,20,24,.92); border-radius: 3px; }
";
