//! A view written in the notation, kept current on a space's branch.
//!
//! The notation's driver (`dialog_notation::live`) does the work: it
//! lays the rules each layer of a view reads on the branch's session
//! overlay, subscribes to each layer's query, and on each poll says what
//! the rows that came and went change in the view's tree. This module is
//! the seam: it takes tonk's inputs (library texts, a branch, an entity)
//! and hands back tonk's own patch type and HTML, so nothing outside it
//! names a type of `dialog-notation`.
//!
//! What is not here yet: a view is not read from the branch. At this
//! revision of the notation a query is compiled from the library's text
//! and nothing of it is stored, so [`View::compile`] takes the texts the
//! space installed. Once definitions are stored on the branch, that is
//! the one call that changes.

use dialog_corpus::{Entity, Value};
use dialog_notation::live::{Live, Reader, Watched as Driven};
use dialog_notation::view::tree::Slot;
use dialog_notation::{
    Analyzer, Content, Patch, Plan, Rendered, Shown, Step, compile, parse, read,
};
use dialog_query::query::Output as _;
use dialog_repository::Branch;

/// Where an instance stands in a view's tree: for each level from the
/// root, the slot of its parent it is in and the key that tells it apart
/// from its siblings.
pub type Address = Vec<(usize, Vec<Value>)>;

/// A change to a rendered view, in tonk's terms: what a page does to
/// its DOM to follow the space.
#[derive(Clone, Debug, PartialEq)]
pub enum ViewPatch {
    /// An instance arrives: `html` goes into slot `slot` of the instance
    /// at `parent` (a root when `parent` is empty), at `index` among the
    /// instances there.
    Insert {
        /// The parent instance.
        parent: Address,
        /// The parent's slot.
        slot: usize,
        /// The place among the slot's instances.
        index: usize,
        /// The instance, rendered.
        html: String,
    },
    /// An instance leaves.
    Remove {
        /// The instance.
        at: Address,
    },
    /// A text of an instance changes.
    SetText {
        /// The instance.
        at: Address,
        /// The slot that holds the text.
        slot: usize,
        /// The text now.
        text: String,
    },
    /// An attribute of an instance changes.
    SetAttribute {
        /// The instance.
        at: Address,
        /// The attribute.
        name: String,
        /// Its value now.
        value: String,
    },
    /// What an event of an instance posts changes.
    SetEvent {
        /// The instance.
        at: Address,
        /// Which of its events, in the order the template writes them.
        index: usize,
        /// The command it posts now.
        command: String,
    },
}

/// The answers of `call`, asked once on `branch` after `documents`
/// (their definitions only), each printed in the notation: what a person
/// or an agent asking gets, `(notebook/named ?n title: ?title)`.
pub async fn select<E: Reader>(
    branch: &Branch,
    documents: &[&str],
    call: &str,
    env: &E,
) -> Result<Vec<String>, String> {
    let tree = analyzed(documents, call)?;
    match compile(&tree)
        .map_err(|error| error.to_string())?
        .as_slice()
    {
        [Step::Select(selection)] => Ok(Live::new(branch.clone())
            .select(selection, env)
            .await
            .map_err(|error| error.to_string())?
            .iter()
            .map(ToString::to_string)
            .collect()),
        [Step::Query(query)] => {
            let mut layer = branch.query();
            if let Some(rule) = &query.rule {
                layer = layer.with(rule.clone());
            }
            let rows: Vec<dialog_query::ConceptConclusion> = layer
                .select(query.select.clone())
                .perform(env)
                .try_vec()
                .await
                .map_err(|error| error.to_string())?;
            rows.iter()
                .map(|row| query.row(row).map(|row| row.to_string()))
                .collect::<Result<_, _>>()
                .map_err(|error| error.to_string())
        }
        other => Err(format!("{call} is no selection: {} steps", other.len())),
    }
}

/// `(select call)` after the definitions of `documents`, analyzed.
fn analyzed(documents: &[&str], call: &str) -> Result<dialog_notation::Syntax, String> {
    // A view reads definitions only: a document's asserts and retracts
    // were written when it was installed, and are left out.
    let mut definitions = Vec::new();
    for document in documents {
        for form in read(document).map_err(|error| error.to_string())? {
            if !matches!(form.head(), Some("assert" | "retract")) {
                definitions.push(form.to_string());
            }
        }
    }
    let text = format!("{}\n(select {call})", definitions.join("\n"));
    let trees = parse(&text).map_err(|error| error.to_string())?;
    let analysis = Analyzer::new()
        .map_err(|error| error.to_string())?
        .analyze(&trees);
    if let Some(refused) = analysis
        .diagnostics
        .iter()
        .find_map(|diagnostic| diagnostic.refusal.as_ref())
    {
        return Err(format!("the notation refused the documents: {refused}"));
    }
    analysis
        .analyzed
        .last()
        .cloned()
        .ok_or_else(|| format!("{call} compiled to nothing"))
}

/// A view compiled from a library's texts, ready to be watched for an
/// entity.
#[derive(Clone)]
pub struct View {
    plan: Plan,
}

impl View {
    /// The view `call` names, compiled after `documents` (core, then the
    /// library, then its views, in that order): `call` is the query
    /// whose answer is markup, as it would be asked,
    /// `(notebook/ui ?n is: ?ui)`.
    pub fn compile(documents: &[&str], call: &str) -> Result<View, String> {
        let tree = analyzed(documents, call)?;
        match compile(&tree)
            .map_err(|error| error.to_string())?
            .as_slice()
        {
            [Step::View(plan)] => Ok(View {
                plan: (**plan).clone(),
            }),
            _ => Err(format!("{call} is no query whose answer is markup")),
        }
    }

    /// The view kept current on `branch` for `entity`, the entity the
    /// view answers of (`None` for one of no entity, a directory). Its
    /// rules are laid on the branch's session overlay and nothing is
    /// committed.
    pub fn watch(&self, branch: &Branch, entity: Option<Entity>) -> Result<Watched, String> {
        let driven = Live::new(branch.clone())
            .watch(self.plan.clone(), entity.map(Value::Entity))
            .map_err(|error| error.to_string())?;
        Ok(Watched {
            driven,
            tree: Rendered::default(),
        })
    }
}

/// A view kept current. Made by [`View::watch`].
pub struct Watched {
    driven: Driven,
    /// The tree as the patches so far make it: what a page holds.
    tree: Rendered,
}

impl Watched {
    /// What moved since the last poll, as patches. The first poll gives
    /// the whole view as inserts. Empty when nothing it reads moved.
    pub async fn poll<E: Reader>(&mut self, env: &E) -> Result<Vec<ViewPatch>, String> {
        let patches = self
            .driven
            .poll(env)
            .await
            .map_err(|error| error.to_string())?;
        let mut out = Vec::with_capacity(patches.len());
        for patch in patches {
            self.tree.apply(&patch)?;
            out.push(match patch {
                Patch::Insert {
                    parent,
                    slot,
                    index,
                    instance,
                } => ViewPatch::Insert {
                    parent,
                    slot,
                    index,
                    html: instance_html(&instance),
                },
                Patch::Remove { at } => ViewPatch::Remove { at },
                Patch::SetText { at, slot, text } => ViewPatch::SetText { at, slot, text },
                Patch::SetAttribute { at, name, value } => {
                    ViewPatch::SetAttribute { at, name, value }
                }
                Patch::SetEvent { at, index, event } => ViewPatch::SetEvent {
                    at,
                    index,
                    command: event.command,
                },
            });
        }
        Ok(out)
    }

    /// The view as the patches so far leave it, as HTML.
    pub fn html(&self) -> String {
        self.tree.roots.iter().map(instance_html).collect()
    }

    /// The view as it would render afresh from what the driver holds,
    /// as HTML: equal to [`Watched::html`] when the patches are right.
    pub fn maintained_html(&self) -> String {
        self.driven.tree().roots.iter().map(instance_html).collect()
    }

    /// How many times each layer's subscription evaluated its query
    /// whole, so far.
    pub fn recomputes(&self) -> Vec<usize> {
        self.driven.recomputes()
    }

    /// How many rows each layer delivered at the last poll, come and
    /// gone.
    pub fn delivered(&self) -> Vec<usize> {
        self.driven.delivered().to_vec()
    }
}

/// A rendered tree as HTML, in the same spelling [`Watched::html`] gives.
pub fn rendered_html(tree: &Rendered) -> String {
    tree.roots.iter().map(instance_html).collect()
}

/// An instance as HTML. An event bound on it is written as
/// `data-on-<type>="<command>"`, which is this module's own spelling
/// and not yet what `tonk-display` listens for.
fn instance_html(shown: &Shown) -> String {
    let Content::Element {
        tag,
        attributes,
        events,
        slots,
    } = &shown.content;
    let mut out = format!("<{tag}");
    for (name, value) in attributes {
        out.push_str(&format!(" {name}=\"{}\"", escape(value)));
    }
    for event in events {
        out.push_str(&format!(
            " data-on-{}=\"{}\"",
            event.kind,
            escape(&event.command)
        ));
    }
    out.push('>');
    for slot in slots {
        match slot {
            Slot::Text(text) => out.push_str(&escape(text)),
            Slot::Instances(list) => list
                .iter()
                .for_each(|child| out.push_str(&instance_html(child))),
        }
    }
    out.push_str(&format!("</{tag}>"));
    out
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
