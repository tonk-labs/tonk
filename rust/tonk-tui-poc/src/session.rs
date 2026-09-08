//! The host state machine: a frame, a focus, and what a keypress does
//! to them.
//!
//! Deliberately terminal-free. Everything a terminal contributes —
//! reading a key, writing cells, owning the alternate screen — is on the
//! other side of [`Key`] and [`Effect`], so the whole interaction model
//! can be driven from a test with no tty (`plan/tui-views.md` §12).
//! What is left here is the part that can actually be wrong: which
//! element focus lands on, and what command an activation posts.
//!
//! It holds no widget state. Carets and scroll offsets are §5.2's
//! problem and are not needed to make a view *reachable*, which is what
//! M3 is.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;
use tonk_render::{Conclusion, Element, Node};
use tonk_template::event::EventDescriptor;

use crate::activate::{self, Activation};
use crate::focus::{self, Axis, Focusable};

/// A key, in the vocabulary the interaction model is written in rather
/// than the one a terminal delivers. Translating crossterm into this is
/// the binary's job and is the only part a test cannot reach.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    /// Next focusable in document order.
    Tab,
    /// Previous.
    BackTab,
    /// Within the focused element's `nav=` container, if it has one.
    Arrow(Direction),
    /// `Enter` or `Space`.
    Activate,
    /// A printable key, which may match a `key=` accelerator.
    Char(char),
    /// Leave.
    Quit,
}

/// Which way an arrow points.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Up,
    Down,
    Left,
    Right,
}

impl Direction {
    fn axis(self) -> Axis {
        match self {
            Self::Up | Self::Down => Axis::Vertical,
            Self::Left | Self::Right => Axis::Horizontal,
        }
    }

    fn forward(self) -> bool {
        matches!(self, Self::Down | Self::Right)
    }
}

/// What a keypress asks the host to do.
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    /// Nothing changed; no repaint is owed.
    Idle,
    /// Focus moved. Repaint.
    Moved,
    /// Post this transact body, then repaint when the subscription
    /// brings the new frame back.
    Post(Value),
    /// The activation resolved to nothing to post — a source did not
    /// apply, so the binding does not fire. Distinct from [`Effect::Idle`]
    /// because it is worth saying out loud: the control looked live.
    Declined,
    /// Leave.
    Quit,
}

/// One keybar chip: what to press, and what it does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chip {
    /// The accelerator.
    pub key: String,
    /// Its caption.
    pub label: String,
}

/// A view, its data, and where focus is.
pub struct Session {
    /// The compiled binding table, straight out of the view's artifact.
    events: BTreeMap<String, EventDescriptor>,
    /// Command name -> its dialog descriptor, as the host resolved them.
    commands: BTreeMap<String, Value>,
    /// The resolved tree this frame.
    tree: Vec<Node>,
    /// Its focusables, in tab order.
    focusables: Vec<Focusable>,
    /// The conclusions the tree was rendered from, for reading `Field`
    /// sources back off the focused row.
    frame: Vec<Conclusion>,
    /// Index into `focusables`, or `None` when the view has none.
    focus: Option<usize>,
    /// What chrome reads when there are no conclusions at all — the same
    /// empty lead the renderer bound the chrome's own holes against, so
    /// a form on an empty view still posts.
    empty: Conclusion,
}

impl Session {
    /// Build a session over an already-resolved tree.
    pub fn new(
        tree: Vec<Node>,
        frame: Vec<Conclusion>,
        events: BTreeMap<String, EventDescriptor>,
        commands: BTreeMap<String, Value>,
    ) -> Self {
        let focusables = focus::collect(&tree, &events);
        let focus = (!focusables.is_empty()).then_some(0);
        Self {
            events,
            commands,
            tree,
            focusables,
            frame,
            focus,
            empty: Conclusion {
                this: String::new(),
                fields: BTreeMap::new(),
            },
        }
    }

    /// The focusables in tab order.
    pub fn focusables(&self) -> &[Focusable] {
        &self.focusables
    }

    /// Which one has focus.
    pub fn focused(&self) -> Option<&Focusable> {
        self.focus.map(|index| &self.focusables[index])
    }

    /// The tree to render this frame: the focused element stamped so
    /// the lowering promotes its `focused-*` decorations, and an empty
    /// `<keybar>` filled with the chips the bindings imply.
    ///
    /// A clone rather than a mutation of the held tree. Focus moves far
    /// more often than the data changes, and a stamp that accumulated
    /// would leave a trail of focus rings behind the cursor.
    pub fn frame_tree(&self) -> Vec<Node> {
        let mut tree = self.tree.clone();
        if let Some(focusable) = self.focused() {
            focus::mark(&mut tree, &focusable.path);
        }
        fill_keybar(&mut tree, &self.keybar());
        tree
    }

    /// The keybar, generated from the bindings rather than maintained
    /// beside them (§5.4).
    ///
    /// Every focusable naming a `key=` contributes a chip, so a chip
    /// cannot advertise something the view does not handle — the two are
    /// the same fact read twice.
    pub fn keybar(&self) -> Vec<Chip> {
        let mut seen: BTreeSet<String> = BTreeSet::new();
        let mut chips: Vec<Chip> = Vec::new();
        for focusable in &self.focusables {
            let Some(key) = &focusable.key else {
                continue;
            };
            // A repeat puts the same chip on every row. One chip.
            if !seen.insert(key.clone()) {
                continue;
            }
            chips.push(Chip {
                key: key.clone(),
                label: focusable.label.clone(),
            });
        }
        chips
    }

    /// Apply a keypress.
    pub fn press(&mut self, key: Key) -> Effect {
        match key {
            Key::Quit => Effect::Quit,
            Key::Tab => self.step(1),
            Key::BackTab => self.step(-1),
            Key::Arrow(direction) => self.arrow(direction),
            Key::Activate => match self.focus {
                Some(index) => self.fire(index),
                None => Effect::Idle,
            },
            Key::Char(character) => self.accelerator(character),
        }
    }

    /// Tab traversal: document order, wrapping.
    ///
    /// Wrapping rather than stopping at the ends because a terminal has
    /// no scrollbar to tell you there is more, and a focus that silently
    /// refuses to move reads as a hang.
    fn step(&mut self, delta: isize) -> Effect {
        let count = self.focusables.len();
        if count == 0 {
            return Effect::Idle;
        }
        let current = self.focus.unwrap_or(0) as isize;
        let next = (current + delta).rem_euclid(count as isize) as usize;
        self.focus = Some(next);
        Effect::Moved
    }

    /// Arrow traversal, scoped to the focused element's `nav=`
    /// container.
    ///
    /// An arrow outside such a container does nothing, rather than
    /// falling back to `Tab`. A terminal user reaches for arrows
    /// constantly for scrolling and history; making them move focus
    /// everywhere would make focus jump under views that never asked for
    /// it.
    fn arrow(&mut self, direction: Direction) -> Effect {
        let Some(index) = self.focus else {
            return Effect::Idle;
        };
        let Some(nav) = self.focusables[index].nav.clone() else {
            return Effect::Idle;
        };
        if nav.axis != direction.axis() {
            return Effect::Idle;
        }
        let siblings: Vec<usize> = self
            .focusables
            .iter()
            .enumerate()
            .filter(|(_, focusable)| focusable.nav.as_ref().map(|n| &n.scope) == Some(&nav.scope))
            .map(|(position, _)| position)
            .collect();
        let Some(at) = siblings.iter().position(|position| *position == index) else {
            return Effect::Idle;
        };
        let delta: isize = if direction.forward() { 1 } else { -1 };
        let next = (at as isize + delta).rem_euclid(siblings.len() as isize) as usize;
        if siblings[next] == index {
            return Effect::Idle;
        }
        self.focus = Some(siblings[next]);
        Effect::Moved
    }

    /// A printable key matching a `key=` accelerator activates that
    /// element without focusing it first — which is what makes the
    /// keybar an affordance rather than a legend.
    fn accelerator(&mut self, character: char) -> Effect {
        let wanted = character.to_string();
        let Some(index) = self
            .focusables
            .iter()
            .position(|focusable| focusable.key.as_deref() == Some(wanted.as_str()))
        else {
            return Effect::Idle;
        };
        self.fire(index)
    }

    /// Build the transact body the focused element's activation posts.
    fn fire(&self, index: usize) -> Effect {
        let focusable = &self.focusables[index];
        let Some(bound) = focusable.activation(&self.events) else {
            return Effect::Idle;
        };
        let Some(descriptor) = self.events.get(&bound.event_name) else {
            return Effect::Idle;
        };
        let Some(command) = self.commands.get(&bound.command) else {
            // The analyzer fails a lowering for a command name that
            // resolves to nothing, so this is a host that was handed a
            // table it did not fill — worth declining loudly rather than
            // posting nothing quietly.
            return Effect::Declined;
        };
        let Some(row) = self.row_of(focusable) else {
            return Effect::Declined;
        };
        let activation = Activation {
            row,
            attributes: &focusable.attributes,
        };
        match activate::build_body(descriptor, command, &activation) {
            Some(body) => Effect::Post(body),
            None => Effect::Declined,
        }
    }

    /// The conclusion a focusable reads its `Field` sources off.
    ///
    /// A focusable inside a repeat clone reads its own row; chrome
    /// outside the repeat reads the lead conclusion, which is what the
    /// renderer bound its own holes against.
    fn row_of(&self, focusable: &Focusable) -> Option<&Conclusion> {
        match &focusable.subject {
            Some(subject) => self.frame.iter().find(|row| &row.this == subject),
            None => Some(self.frame.first().unwrap_or(&self.empty)),
        }
    }
}

/// Fill the first *empty* `<keybar>` with one `<key>` per chip.
///
/// Empty on purpose: a keybar an author has written chips into is theirs
/// to maintain, and silently replacing it would make the template lie
/// about what it renders. An empty one is a request — "put the chips
/// here" — which is the placement decision the host cannot make and the
/// author can.
fn fill_keybar(nodes: &mut [Node], chips: &[Chip]) -> bool {
    for node in nodes {
        let Node::Element(element) = node else {
            continue;
        };
        if element.tag == "keybar" && !element.children.iter().any(|c| c.tag().is_some()) {
            element.children = chips.iter().map(chip_node).collect();
            return true;
        }
        if fill_keybar(&mut element.children, chips) {
            return true;
        }
    }
    false
}

fn chip_node(chip: &Chip) -> Node {
    Node::Element(Element {
        tag: "key".to_owned(),
        attrs: Vec::new(),
        children: vec![Node::Text(format!("{} {}", chip.key, chip.label))],
        void: false,
    })
}
