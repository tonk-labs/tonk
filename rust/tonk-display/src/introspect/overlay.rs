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
use super::inspect::default_subject;
use super::mode::{Input, Machine, TargetId};
use super::panel::{self, Panel};
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
    /// Everything painted over the page, below the chrome. Markers
    /// live here so the panel is never buried under a badge.
    marks: Element,
    /// A transparent, clickable surface over the tracked display.
    /// Clicking it pins — which is why the overlay needs to take no
    /// gesture from the page at all: a click here never reaches it.
    shield: Element,
    /// The outline around the tracked display.
    outline: Element,
    /// The pin affordance. The one thing on the layer that takes
    /// pointer events.
    pin: Element,
    /// The concept panel.
    panel: Panel,
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
    /// The repeat row the pointer is over, by its stamped subject. In
    /// a directory this is what the panel follows: point at a card,
    /// read that card's values.
    hovered_subject: Option<String>,
    /// The field (or command) a part of the panel is asking to
    /// highlight, while the pointer rests on it. Both halves of the
    /// panel key on the same name, so one value serves a concept row
    /// and a marked span in the template alike.
    focus: Option<String>,
    /// Where the user dragged the panel to, if they did. Otherwise it
    /// places itself away from whatever is being observed.
    panel_at: Option<(f64, f64)>,
    /// A drag in progress: the pointer's offset inside the panel.
    dragging: Option<(f64, f64)>,
}

/// The painted state for one observed display.
struct Painted {
    target: TargetId,
    /// Everything the display and its renderer reported, kept so the
    /// panel can redraw for another subject without a full rebuild.
    snapshot: super::slot::Snapshot,
    slots: Vec<Marker>,
    commands: Vec<Marker>,
    /// A fingerprint of what was described, so an unchanged snapshot
    /// reuses its markers and keeps their identity intact.
    signature: String,
    /// Whether the marker cap truncated what is drawn. The panel says
    /// so, since the page itself cannot show what is not painted.
    truncated: bool,
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
    /// What the panel would name to highlight this marker: the fields
    /// a slot reads, or the command an interaction posts.
    keys: Vec<String>,
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
        style.set_text_content(Some(&format!("{CSS}{}", super::panel::CSS)));
        let _ = root.append_child(&style);

        let layer = element(document, "div", "layer")?;
        let outline = element(document, "div", "outline")?;
        let pin = element(document, "button", "pin")?;
        pin.set_text_content(Some("pin"));
        let marks = element(document, "div", "marks")?;
        let shield = element(document, "div", "shield")?;
        let _ = shield.set_attribute("title", "click to pin this display");
        // Order is z-order: marks under the chrome, the panel over
        // everything. A badge drawn across the panel was the reported
        // symptom of getting this wrong.
        let _ = layer.append_child(&marks);
        let _ = layer.append_child(&shield);
        let _ = layer.append_child(&outline);
        let _ = layer.append_child(&pin);
        let _ = root.append_child(&layer);
        let panel = Panel::build(document, &layer)?;

        Some(Self {
            host: host.clone(),
            marks,
            shield,
            outline,
            pin,
            panel,
            machine: Machine::default(),
            targets: Vec::new(),
            painted: None,
            frame: None,
            tick: None,
            age: 0,
            painting: false,
            hovered_subject: None,
            focus: None,
            panel_at: None,
            dragging: None,
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
        hide(&self.outline);
        hide(&self.pin);
        hide(&self.shield);
        self.panel.hide();
        self.focus = None;
        self.hovered_subject = None;
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
            // A drag owns the pointer outright.
            if overlay.borrow().dragging.is_some() {
                drag_panel(overlay, mouse);
                return;
            }
            // The whole cost of a closed hood. Everything below this
            // hit-tests the document, so nothing below it may run on
            // the mousemoves of a page nobody is inspecting. Alt up
            // with nothing painted means there is neither anything to
            // start nor anything to stop.
            if !mouse.alt_key() && !overlay.borrow().painting {
                return;
            }
            // Resting on the panel or the pin is not leaving the
            // display; without this the outline vanishes the moment
            // the pointer crosses onto either.
            if on_chrome(overlay, mouse) {
                return;
            }
            // Sticky: the subject only changes when the pointer is
            // over a row, so walking off a card towards the panel
            // keeps the panel on the card you came from — which is
            // the reason you were walking towards it.
            if let Some(subject) = subject_under(overlay, mouse) {
                overlay.borrow_mut().hovered_subject = Some(subject);
            }
            let over = display_under(overlay, mouse).map(|host| {
                let mut state = overlay.borrow_mut();
                state.target_of(&host)
            });
            let input = Input::Pointer {
                alt: mouse.alt_key(),
                over,
                at: js_sys::Date::now(),
            };
            overlay.borrow_mut().machine.apply(input);
        },
    ));

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

    // The shield: the whole tracked display, clickable. This is the
    // big hit target — the pin is only its label — and because it is
    // overlay chrome the click never reaches the page, so no gesture
    // has to be taken from it or swallowed.
    let shield = overlay.borrow().shield.clone();
    bound.push(listen(
        shield.as_ref(),
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

    // Dragging the panel by its header.
    let head = overlay.borrow().panel.head().clone();
    bound.push(listen(
        head.as_ref(),
        "mousedown",
        false,
        overlay,
        |overlay, event| {
            let Some(mouse) = event.dyn_ref::<MouseEvent>() else {
                return;
            };
            if mouse
                .target()
                .and_then(|target| target.dyn_into::<Element>().ok())
                .is_some_and(|element| element.matches(".close").unwrap_or(false))
            {
                return;
            }
            event.prevent_default();
            let rect = overlay.borrow().panel.root().get_bounding_client_rect();
            overlay.borrow_mut().dragging = Some((
                mouse.client_x() - rect.left(),
                mouse.client_y() - rect.top(),
            ));
        },
    ));
    bound.push(listen(
        &target,
        "mouseup",
        true,
        overlay,
        |overlay, _event| {
            overlay.borrow_mut().dragging = None;
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

    // The panel. Its rows highlight the slots they name while the
    // pointer rests on them, and its close button stops observing.
    let root = overlay.borrow().panel.root().clone();
    bound.push(listen(
        root.as_ref(),
        "mouseover",
        false,
        overlay,
        |overlay, event| {
            let focus = event
                .target()
                .and_then(|target| target.dyn_into::<Element>().ok())
                .and_then(|element| panel::field_of(&element));
            overlay.borrow_mut().focus = focus;
        },
    ));
    bound.push(listen(
        root.as_ref(),
        "mouseleave",
        false,
        overlay,
        |overlay, _event| {
            overlay.borrow_mut().focus = None;
        },
    ));
    bound.push(listen(
        root.as_ref(),
        "click",
        false,
        overlay,
        |overlay, event| {
            event.stop_propagation();
            let Some(target) = event
                .target()
                .and_then(|target| target.dyn_into::<Element>().ok())
            else {
                return;
            };
            if target.matches(".close").unwrap_or(false) {
                overlay.borrow_mut().machine.apply(Input::Clear);
                return;
            }
            if let Some(tab) = Panel::tab_of(&target) {
                overlay.borrow_mut().panel.select(tab);
            }
        },
    ));

    bound
}

/// The repeat row's subject under a pointer event. The renderer stamps
/// `data-this` on every row root, so this is a read of what the render
/// pass already wrote.
fn subject_under(overlay: &Rc<RefCell<Overlay>>, event: &MouseEvent) -> Option<String> {
    under(overlay, event)
        .and_then(|element| element.closest("[data-this]").ok().flatten())
        .and_then(|row| row.get_attribute("data-this"))
        .filter(|subject| !subject.is_empty())
}

/// Whether the pointer is over the panel or the pin — the chrome that
/// must be reachable without the observation evaporating.
///
/// Tested by coordinate rather than by event target. Everything in the
/// overlay's shadow root retargets to one host, so a target test
/// cannot tell the panel from the shield — and the shield covers the
/// display, so treating it as chrome would blind the machine to the
/// pointer crossing onto a display nested inside.
fn on_chrome(overlay: &Rc<RefCell<Overlay>>, mouse: &MouseEvent) -> bool {
    let state = overlay.borrow();
    let x = mouse.client_x();
    let y = mouse.client_y();
    [&state.panel.root().clone(), &state.pin]
        .into_iter()
        .any(|element| {
            let rect = element.get_bounding_client_rect();
            rect.width() > 0.0
                && x >= rect.left()
                && x <= rect.right()
                && y >= rect.top()
                && y <= rect.bottom()
        })
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

/// The topmost page element under a pointer event, ignoring the
/// overlay's own chrome.
///
/// Hit-testing by point rather than by `event.target`, because the
/// shield sits over the tracked display and every event target inside
/// it retargets to the overlay host. Point-testing sees past it, which
/// is what lets a shielded display still tell you when the pointer has
/// crossed onto the display nested inside it.
fn under(overlay: &Rc<RefCell<Overlay>>, event: &MouseEvent) -> Option<Element> {
    let document = window()?.document()?;
    let host = overlay.borrow().host.clone();
    let stack = document.elements_from_point(event.client_x() as f32, event.client_y() as f32);
    for index in 0..stack.length() {
        let Ok(element) = stack.get(index).dyn_into::<Element>() else {
            continue;
        };
        if element.is_same_node(Some(host.as_ref())) {
            continue;
        }
        return Some(element);
    }
    None
}

/// The `<tonk-display>` under a pointer event, if any. `closest` gives
/// the innermost one, which is the right answer: a display nested
/// inside another's template is its own thing to inspect.
fn display_under(overlay: &Rc<RefCell<Overlay>>, event: &MouseEvent) -> Option<Element> {
    under(overlay, event).and_then(|element| element.closest("tonk-display").ok().flatten())
}

/// Move the panel under a dragging pointer, clamped to the viewport
/// so it cannot be dropped somewhere unreachable.
fn drag_panel(overlay: &Rc<RefCell<Overlay>>, mouse: &MouseEvent) {
    let Some(win) = window() else {
        return;
    };
    let (offset_x, offset_y) = match overlay.borrow().dragging {
        Some(offset) => offset,
        None => return,
    };
    let rect = overlay.borrow().panel.root().get_bounding_client_rect();
    let width = win
        .inner_width()
        .ok()
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let height = win
        .inner_height()
        .ok()
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let left = (mouse.client_x() - offset_x).clamp(0.0, (width - rect.width()).max(0.0));
    let top = (mouse.client_y() - offset_y).clamp(0.0, (height - rect.height()).max(0.0));
    overlay.borrow_mut().panel_at = Some((left, top));
}

/// Where the panel sits: where it was dragged, else the corner
/// furthest from what is being observed.
///
/// A fixed corner is wrong as often as it is right — half the time it
/// covers the very thing you asked it about. Choosing the diagonally
/// opposite corner is not a layout engine, but it is right far more
/// often, and dragging covers the rest.
fn panel_position(overlay: &Overlay, display: Option<&Element>) -> String {
    if let Some((left, top)) = overlay.panel_at {
        return format!("left:{left}px;top:{top}px;right:auto;bottom:auto");
    }
    let Some(win) = window() else {
        return "right:8px;bottom:8px".to_owned();
    };
    let width = win
        .inner_width()
        .ok()
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let height = win
        .inner_height()
        .ok()
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let Some(rect) = display.map(|display| display.get_bounding_client_rect()) else {
        return "right:8px;bottom:8px".to_owned();
    };
    let horizontal = if rect.left() + rect.width() / 2.0 < width / 2.0 {
        "right:8px;left:auto"
    } else {
        "left:8px;right:auto"
    };
    let vertical = if rect.top() + rect.height() / 2.0 < height / 2.0 {
        "bottom:8px;top:auto"
    } else {
        "top:8px;bottom:auto"
    };
    format!("{horizontal};{vertical}")
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
            let mut state = overlay.borrow_mut();
            state.drop_painted();
            state.panel.hide();
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
    let box_style = format!(
        "display:block;left:{}px;top:{}px;width:{}px;height:{}px",
        rect.left(),
        rect.top(),
        rect.width(),
        rect.height()
    );
    let _ = state.outline.set_attribute("style", &box_style);
    let _ = state.shield.set_attribute("style", &box_style);
    let latched = state.machine.is_latched();
    let _ = state
        .outline
        .set_attribute("class", if latched { "outline pinned" } else { "outline" });
    let _ = state.shield.set_attribute(
        "title",
        if latched {
            "click to stop pinning this display"
        } else {
            "click to pin this display"
        },
    );

    // Inside the outline's top-left, overlapping the display rather
    // than floating above it. Outside, the walk to reach it crosses
    // page that is not a display, and every step of that walk used to
    // read as giving up — which made the pin, and so pinning at all,
    // unreachable. The leave grace covers the rest of the gap; this
    // removes most of it.
    let top = rect.top();
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
    draw_panel(overlay);
    flash_changes(overlay);
    bounce_dispatches(overlay);
}

/// Show the panel for whichever subject the pointer is nearest.
fn draw_panel(overlay: &Rc<RefCell<Overlay>>) {
    let Some(document) = window().and_then(|w| w.document()) else {
        return;
    };
    let mut state = overlay.borrow_mut();
    let hovered = state.hovered_subject.clone();
    let Some(painted) = state.painted.take() else {
        return;
    };
    let subject = default_subject(&painted.snapshot, hovered.as_deref()).map(str::to_owned);
    let display = state.element(painted.target).cloned();
    let position = panel_position(&state, display.as_ref());
    state.panel.show(
        &document,
        &painted.snapshot,
        &painted.signature,
        subject.as_deref(),
        painted.truncated,
        &position,
    );
    state.painted = Some(painted);
}

/// Re-ask the display and its renderer what is there, and rebuild the
/// markers if the set has actually changed shape.
fn rebuild(overlay: &Rc<RefCell<Overlay>>, target: TargetId, host: &Element) {
    let slots = registry::slots_under(host);
    let commands = registry::commands_under(host);
    let signature = signature(&slots, &commands);
    // The display reports the concept half and the renderer the slot
    // half; the panel needs them as one value, so they are joined
    // here rather than either side reaching across.
    let mut snapshot = registry::display_facts(host).unwrap_or_default();
    snapshot.slots = slots.iter().map(|(slot, _)| slot.clone()).collect();

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
    let layer = overlay.borrow().marks.clone();

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
        if let Some(marker) = build_marker(
            &document,
            &layer,
            &classes,
            slot.label(),
            node,
            style,
            slot.fields.clone(),
        ) {
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
            vec![command.command.clone()],
        ) {
            command_markers.push(marker);
            budget -= 1;
        }
    }

    let truncated = budget == 0;
    let mut state = overlay.borrow_mut();
    state.drop_painted();
    state.painted = Some(Painted {
        target,
        snapshot,
        truncated,
        slots: slot_markers,
        commands: command_markers,
        signature,
    });
}

fn build_marker(
    document: &Document,
    layer: &Element,
    classes: &str,
    label: String,
    anchor: Node,
    style: MarkerStyle,
    keys: Vec<String>,
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
        keys,
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
        apply_focus(marker, state.focus.as_deref());
    }
}

/// While a panel row is under the pointer, the slots it names come
/// forward and everything else recedes. This is the other half of the
/// panel: a row says which fields exist, and the page says where they
/// went.
fn apply_focus(marker: &Marker, focus: Option<&str>) {
    let state = match focus {
        None => "",
        Some(focus) if marker.keys.iter().any(|key| key == focus) => "on",
        Some(_) => "off",
    };
    let _ = marker.mark.set_attribute("data-focus", state);
    let _ = marker.badge.set_attribute("data-focus", state);
    let _ = marker.leader.set_attribute("data-focus", state);
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
    let _ = overlay.borrow().marks.append_child(&marker);
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
.marks { position: fixed; inset: 0; pointer-events: none; z-index: 1; }
.shield { position: fixed; display: none; z-index: 2; pointer-events: auto; cursor: pointer;
          background: transparent; }
.shield:hover { background: color-mix(in srgb, #4f8cff 7%, transparent); }
.outline { position: fixed; display: none; z-index: 3; pointer-events: none; box-sizing: border-box;
           border: 1px solid #4f8cff; background: color-mix(in srgb, #4f8cff 5%, transparent);
           border-radius: 2px; }
.outline.pinned { border-style: dashed; border-width: 2px; }
.pin { position: fixed; display: none; z-index: 4; pointer-events: auto; cursor: pointer;
       height: 20px; padding: 0 10px; font: inherit; line-height: 20px; color: #fff;
       background: #4f8cff; border: 0; border-radius: 0 0 3px 0; }
.pin:hover { background: #1f6feb; }
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
[data-focus=off] { opacity: .18; }
.mark[data-focus=on] { outline: 1px solid #fff; outline-offset: 1px; }
.badge[data-focus=on] { box-shadow: 0 0 0 1px #fff; }
";
