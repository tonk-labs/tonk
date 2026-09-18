//! How the overlay reaches a display's own state.
//!
//! The overlay needs facts that only the elements hold: which concept
//! a `<tonk-display>` resolved, which template it mounted, and which
//! slots a `<tonk-view>`'s renderer filled. None of that is legible
//! from the rendered DOM — a rendered `with="main@repo"` cannot tell
//! you whether `repo` was a field or a literal, and an attribute
//! binding applied as a JS property leaves no attribute at all.
//!
//! So each element registers itself here on connect, as a `Weak`
//! handle behind a narrow trait. The overlay looks up the host element
//! it is pointing at and asks. Weak because the registry must never be
//! what keeps a detached element alive; dead entries are pruned on
//! every lookup, which is also what makes a `disconnected_callback`
//! unnecessary.
//!
//! The registries are thread-locals, and every frame's wasm instance
//! has its own — which is exactly right, since an overlay only ever
//! introspects displays in its own document.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use web_sys::{Element, Node};

use super::command::Command;
use super::slot::{Slot, Snapshot};

/// What a `<tonk-display>` can say about itself: everything in a
/// [`Snapshot`] except the slots, which belong to its mounted view.
pub trait DisplayFacts {
    /// The concept, facet, mode, subjects and template text this
    /// display currently has resolved. `host` is the element itself,
    /// for the attributes the author wrote on it.
    fn facts(&self, host: &Element) -> Snapshot;

    /// Every interaction the rendered markup binds, paired with the
    /// element that binds it. The display owns this rather than the
    /// view because only it holds the resolved event declarations —
    /// an `on:<name>` attribute names a declaration, and only the
    /// table says which platform event that declaration reads.
    fn commands(&self, host: &Element) -> Vec<(Command, Element)>;
}

/// What a `<tonk-view>` can say about itself: the slots its renderer
/// filled, each paired with the node it wrote into. The node is
/// `None` when the binding's target has gone missing — rare, but the
/// slot is still worth listing.
pub trait ViewFacts {
    /// Every mounted slot, in plan order.
    fn slots(&self) -> Vec<(Slot, Option<Node>)>;
}

struct Entry<T: ?Sized> {
    host: Element,
    state: Weak<RefCell<T>>,
}

thread_local! {
    /// True while an overlay is observing something. Read on the
    /// renderer's hot path, so it stays a plain `Cell<bool>`.
    static ARMED: Cell<bool> = const { Cell::new(false) };
    /// Nodes whose slot value changed since the overlay last drained
    /// this. Only filled while [`armed`].
    static CHANGES: RefCell<Vec<Node>> = const { RefCell::new(Vec::new()) };
    /// Elements that posted a command since the last drain, with the
    /// command they posted. Only filled while [`armed`].
    static DISPATCHES: RefCell<Vec<(Element, String)>> = const { RefCell::new(Vec::new()) };
    static DISPLAYS: RefCell<Vec<Entry<dyn DisplayFacts>>> =
        const { RefCell::new(Vec::new()) };
    static VIEWS: RefCell<Vec<Entry<dyn ViewFacts>>> = const { RefCell::new(Vec::new()) };
}

/// How many pending changes are kept. A flash the overlay never got
/// to is not worth remembering, and an armed overlay that stalls must
/// not grow this without bound.
const CHANGE_CAP: usize = 256;

/// Whether an overlay is currently observing. The renderer checks
/// this before doing any introspection work, so everything here costs
/// one bool read when the hood is closed.
pub fn armed() -> bool {
    ARMED.with(Cell::get)
}

/// Turn change recording on or off. Turning it off drops whatever was
/// queued — those flashes belong to an observation that has ended.
pub fn set_armed(on: bool) {
    ARMED.with(|armed| armed.set(on));
    if !on {
        CHANGES.with(|changes| changes.borrow_mut().clear());
        DISPATCHES.with(|dispatches| dispatches.borrow_mut().clear());
    }
}

/// Record that a slot's value changed at `node`. Called from the
/// renderer, already behind an [`armed`] check.
pub fn note_change(node: &Node) {
    CHANGES.with(|changes| {
        let mut changes = changes.borrow_mut();
        if changes.len() < CHANGE_CAP {
            changes.push(node.clone());
        }
    });
}

/// Take every change recorded since the last drain.
pub fn drain_changes() -> Vec<Node> {
    CHANGES.with(|changes| std::mem::take(&mut *changes.borrow_mut()))
}

/// Record that `bound` posted `command`. Called from the dispatch
/// path at the point the winning binding is known — which is not
/// always the element that was clicked, since dispatch walks up until
/// a binding resolves. Already behind an [`armed`] check.
pub fn note_dispatch(bound: &Element, command: &str) {
    DISPATCHES.with(|dispatches| {
        let mut dispatches = dispatches.borrow_mut();
        if dispatches.len() < CHANGE_CAP {
            dispatches.push((bound.clone(), command.to_owned()));
        }
    });
}

/// Take every dispatch recorded since the last drain.
pub fn drain_dispatches() -> Vec<(Element, String)> {
    DISPATCHES.with(|dispatches| std::mem::take(&mut *dispatches.borrow_mut()))
}

/// Register a `<tonk-display>`'s state against its host element.
pub fn register_display(host: &Element, state: &Rc<RefCell<impl DisplayFacts + 'static>>) {
    DISPLAYS.with(|displays| {
        let mut displays = displays.borrow_mut();
        prune(&mut displays);
        displays.push(Entry {
            host: host.clone(),
            state: Rc::downgrade(state) as Weak<RefCell<dyn DisplayFacts>>,
        });
    });
}

/// Register a `<tonk-view>`'s state against its host element.
pub fn register_view(host: &Element, state: &Rc<RefCell<impl ViewFacts + 'static>>) {
    VIEWS.with(|views| {
        let mut views = views.borrow_mut();
        prune(&mut views);
        views.push(Entry {
            host: host.clone(),
            state: Rc::downgrade(state) as Weak<RefCell<dyn ViewFacts>>,
        });
    });
}

/// The facts `host` reports, or `None` if it is not a registered
/// display (or has been torn down).
pub fn display_facts(host: &Element) -> Option<Snapshot> {
    DISPLAYS.with(|displays| {
        let mut displays = displays.borrow_mut();
        prune(&mut displays);
        let entry = displays
            .iter()
            .find(|entry| entry.host.is_same_node(Some(host.as_ref())))?;
        let state = entry.state.upgrade()?;
        let state = state.try_borrow().ok()?;
        Some(state.facts(host))
    })
}

/// The slots of every `<tonk-view>` whose nearest `<tonk-display>`
/// ancestor is `host` — the views this display owns, and not the ones
/// belonging to a display nested inside it.
pub fn slots_under(host: &Element) -> Vec<(Slot, Option<Node>)> {
    let mut out = Vec::new();
    VIEWS.with(|views| {
        let mut views = views.borrow_mut();
        prune(&mut views);
        for entry in views.iter() {
            if !owns(host, &entry.host) {
                continue;
            }
            let Some(state) = entry.state.upgrade() else {
                continue;
            };
            // A view element re-entered through its own render path
            // would already hold this borrow; skip rather than panic.
            let Ok(state) = state.try_borrow() else {
                continue;
            };
            out.extend(state.slots());
        }
    });
    // Each renderer numbers its slots from zero, and a display can
    // have more than one view mounted. Renumber across the whole set
    // so an id identifies exactly one slot — which is what lets a
    // panel row name the slots it should highlight.
    for (index, (slot, _)) in out.iter_mut().enumerate() {
        slot.id = index as u32;
    }
    out
}

/// The interactions bound inside `host`, paired with their elements.
pub fn commands_under(host: &Element) -> Vec<(Command, Element)> {
    DISPLAYS.with(|displays| {
        let mut displays = displays.borrow_mut();
        prune(&mut displays);
        let Some(entry) = displays
            .iter()
            .find(|entry| entry.host.is_same_node(Some(host.as_ref())))
        else {
            return Vec::new();
        };
        let Some(state) = entry.state.upgrade() else {
            return Vec::new();
        };
        let Ok(state) = state.try_borrow() else {
            return Vec::new();
        };
        state.commands(host)
    })
}

/// Whether `display` is the nearest `<tonk-display>` above `view`.
fn owns(display: &Element, view: &Element) -> bool {
    view.closest("tonk-display")
        .ok()
        .flatten()
        .is_some_and(|nearest| nearest.is_same_node(Some(display.as_ref())))
}

/// Drop entries whose element has been torn down.
fn prune<T: ?Sized>(entries: &mut Vec<Entry<T>>) {
    entries.retain(|entry| entry.state.strong_count() > 0);
}
