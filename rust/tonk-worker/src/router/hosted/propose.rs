//! Propose, then apply: a change the model asks for that the person
//! confirms in the chat first.
//!
//! - A model-visible tool proposes: the writer of the space for the MCP
//!   session dry-runs the change and holds it under a fresh id
//!   ([`Proposals`]). The tool's result is an ask card ([`super::card`])
//!   with what the dry run would change, and changes nothing.
//! - The card's answers call app-only tools: [`APPLY`] writes the held
//!   change, once; [`DISMISS`] drops it. Each answers the card that
//!   replaces the ask, and the card tells the model what happened.
//!
//! An id is good for one answer, on the writer that holds it. A writer
//! belongs to one space and one MCP session (its key is derived from the
//! session id, and the Host Worker names its object by that key), so an
//! id from another session, or for another space, is not found there. A
//! held change lives in the writer's memory: a writer that is evicted or
//! recycled forgets it, and the person is told to ask again.
//!
//! `tonk_evaluate` with `confirm: true` is the one proposer today.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use serde_json::{Value, json};

use super::card::{Agent, Card, Disc, Look, Place, Role};

/// The app-only tool that applies a held change.
pub const APPLY: &str = "tonk_proposal_apply";

/// The app-only tool that drops a held change.
pub const DISMISS: &str = "tonk_proposal_dismiss";

/// The writer's route that dry-runs a document and holds it (`POST`, the
/// document as `text/plain`), answering [`held`].
pub const PROPOSE_ROUTE: &str = "/api/proposals";

/// The writer's route that applies a held change (`POST`, `{"id"}`),
/// answering the evaluation.
pub const APPLY_ROUTE: &str = "/api/proposals/apply";

/// The writer's route that drops a held change (`POST`, `{"id"}`).
pub const DISMISS_ROUTE: &str = "/api/proposals/dismiss";

/// How long a change is held, in seconds.
pub const LIFETIME: u64 = 30 * 60;

/// The most changes one writer holds; a new one drops the oldest.
const MOST: usize = 16;

/// The changes an ask lists before it says how many more there are.
const LISTED: usize = 5;

/// Changes a writer holds until the person answers.
#[derive(Default)]
pub struct Proposals {
    held: parking_lot::Mutex<Vec<Held>>,
}

struct Held {
    id: String,
    document: String,
    /// The revision the document was previewed at, as the worker
    /// answered it (`null` for an empty branch).
    revision: Value,
    /// What the preview showed the person: its matches before and after.
    seen: Value,
    expires: u64,
}

/// A held change, taken to be answered.
#[derive(Debug, PartialEq)]
pub struct Taken {
    /// The document.
    pub document: String,
    /// The revision it was previewed at.
    pub revision: Value,
    /// What its preview showed: [`seen`] of the preview's evaluation.
    pub seen: Value,
}

/// What a preview `evaluation` shows the person: its matches before and
/// after, which an answer compares with a fresh preview.
pub fn seen(evaluation: &Value) -> Value {
    json!({
        "before": evaluation["matches_before"],
        "after": evaluation["matches_after"],
    })
}

/// Why a held change cannot be answered.
#[derive(Debug, PartialEq, Eq)]
pub struct Gone;

impl Proposals {
    /// Hold `document`, previewed at `revision` and showing `seen`, until
    /// `now + LIFETIME`. Answers its id and when it lapses.
    pub fn hold(&self, document: String, revision: Value, seen: Value, now: u64) -> (String, u64) {
        let id = B64.encode(rand::random::<[u8; 16]>());
        let expires = now + LIFETIME;
        let mut held = self.held.lock();
        held.retain(|held| held.expires > now);
        if held.len() >= MOST {
            held.remove(0);
        }
        held.push(Held {
            id: id.clone(),
            document,
            revision,
            seen,
            expires,
        });
        (id, expires)
    }

    /// The change held as `id` and the revision it was previewed at, no
    /// longer held: each is answered once.
    ///
    /// # Errors
    ///
    /// [`Gone`] when no change is held as `id`: it was answered, it lapsed,
    /// or another writer holds it.
    pub fn take(&self, id: &str, now: u64) -> Result<Taken, Gone> {
        let mut held = self.held.lock();
        held.retain(|held| held.expires > now);
        let at = held.iter().position(|held| held.id == id).ok_or(Gone)?;
        let held = held.remove(at);
        Ok(Taken {
            document: held.document,
            revision: held.revision,
            seen: held.seen,
        })
    }
}

/// What [`PROPOSE_ROUTE`] answers: the held change's id and expiry, and
/// the dry run's evaluation.
pub fn held(id: &str, expires: u64, evaluation: Value) -> Value {
    json!({ "proposal": { "id": id, "expires": expires }, "evaluation": evaluation })
}

/// Tools appended to `tools/list`.
pub fn tools() -> Vec<Value> {
    let arguments = json!({
        "type": "object",
        "properties": {
            "space": {
                "type": "string",
                "pattern": "^did:key:[A-Za-z0-9]+$",
            },
            "id": { "type": "string", "minLength": 1, "maxLength": 64 },
        },
        "required": ["space", "id"],
        "additionalProperties": false,
    });
    let app_only = json!({
        "ui": { "visibility": ["app"] },
        "openai/visibility": "private",
    });
    vec![
        json!({
            "name": APPLY,
            "description": "Apply a change the person confirmed. Called by the card.",
            "inputSchema": arguments,
            "_meta": app_only,
        }),
        json!({
            "name": DISMISS,
            "description": "Drop a change the person declined. Called by the card.",
            "inputSchema": arguments,
            "_meta": app_only,
        }),
    ]
}

/// What a card says about a space: where, the role there, and who acts.
#[derive(Debug, Clone)]
pub struct Context {
    /// The space.
    pub place: Place,
    /// The role there; `None` where the connection cannot read it.
    pub role: Option<Role>,
    /// The agent acting.
    pub agent: Agent,
    /// Whether the space is the agent's own.
    pub home: bool,
    /// The provider's origin, where Tonk opens in the person's browser.
    pub origin: String,
}

impl Context {
    /// The space in Tonk.
    fn in_tonk(&self) -> String {
        format!(
            "{}/space/{}",
            self.origin.trim_end_matches('/'),
            self.place.did
        )
    }

    /// Where the person manages their agents' access in Tonk.
    fn agents_in_tonk(&self) -> String {
        format!("{}/settings", self.origin.trim_end_matches('/'))
    }
}

/// One line of what a dry run or a write changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Changed {
    /// `+` added, `−` removed, `~` changed.
    pub mark: &'static str,
    /// The entity's name for a person.
    pub name: String,
    /// What kind of thing it is: a part of the space's schema (`view`,
    /// `concept`, ...), a `command`, or an `item` for a record.
    pub kind: Option<String>,
    /// The entity's URI.
    pub this: String,
    /// The label of the expression it came from.
    pub of: String,
    /// Whether it is a command the document runs, not a durable fact.
    pub command: bool,
}

/// The labels of the space's own parts, which a change list names as
/// they are; any other label is a record's concept.
const PARTS: &[&str] = &[
    "view",
    "concept",
    "attribute",
    "rule",
    "component",
    "effect",
];

/// What `evaluation` (a worker `/evaluate` answer) changes: for each
/// expression, each entity it matches after and not before is added,
/// each it matches before and after with other fields is changed, and
/// each it matched only before is removed. A match the same before and
/// after changed nothing, so an expression that only reads lists none.
pub fn changed(evaluation: &Value) -> Vec<Changed> {
    let blocks = |key: &str| evaluation[key].as_array().cloned().unwrap_or_default();
    let before = blocks("matches_before");
    let after = blocks("matches_after");
    let results = |block: Option<&Value>| {
        block
            .and_then(|block| block["results"].as_array().cloned())
            .unwrap_or_default()
    };
    let mut changed = Vec::new();
    for (index, block) in after.iter().enumerate() {
        let label = block["label"].as_str().unwrap_or("item").to_owned();
        let was = results(before.get(index));
        let now = results(Some(block));
        let this = |result: &Value| result["this"].as_str().unwrap_or_default().to_owned();
        let part = PARTS
            .iter()
            .find(|part| label.trim_end_matches('!') == **part)
            .map(|part| (*part).to_owned());
        for result in &now {
            // A match that reads the same before and after is a query's
            // answer, or a write of what was there already: no change.
            if was.contains(result) {
                continue;
            }
            let existed = was.iter().any(|old| this(old) == this(result));
            let command = result["transient"].as_bool().unwrap_or(false);
            changed.push(Changed {
                mark: if existed { "~" } else { "+" },
                name: name_of(result, &label),
                kind: Some(if command {
                    "command".to_owned()
                } else {
                    part.clone().unwrap_or_else(|| "item".to_owned())
                }),
                this: this(result),
                of: label.clone(),
                command,
            });
        }
        for result in &was {
            if !now.iter().any(|new| this(new) == this(result)) {
                changed.push(Changed {
                    mark: "−",
                    name: name_of(result, &label),
                    kind: Some(part.clone().unwrap_or_else(|| "item".to_owned())),
                    this: this(result),
                    of: label.clone(),
                    command: false,
                });
            }
        }
    }
    changed
}

/// A match's name for a person: its name, title or label field, else
/// the last part of its URI.
fn name_of(result: &Value, label: &str) -> String {
    for field in ["name", "title", "label", "text"] {
        if let Some(text) = result["fields"][field]
            .as_str()
            .filter(|text| !text.is_empty())
        {
            return text.to_owned();
        }
    }
    let this = result["this"].as_str().unwrap_or_default();
    let tail = this
        .rsplit(['/', ':', '#'])
        .find(|part| !part.is_empty())
        .unwrap_or(label);
    if tail.chars().count() > 16 {
        format!("{}…", tail.chars().take(12).collect::<String>())
    } else {
        tail.to_owned()
    }
}

/// What a change does, as one verb and its object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Summary {
    verb: Verb,
    /// The one thing it changes by name, or how many: "Sun hat", "3 items".
    object: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Verb {
    Add,
    Remove,
    Change,
    Run,
}

impl Summary {
    /// What `changes` do: durable changes first (all added, all removed,
    /// or otherwise changed), else the command they run. `None` when they
    /// do nothing.
    pub fn of(changes: &[Changed]) -> Option<Self> {
        let durable: Vec<&Changed> = changes.iter().filter(|change| !change.command).collect();
        if durable.is_empty() {
            let command = changes.iter().find(|change| change.command)?;
            return Some(Self {
                verb: Verb::Run,
                object: command.of.replace(['/', '-', '_'], " "),
            });
        }
        let verb = if durable.iter().all(|change| change.mark == "+") {
            Verb::Add
        } else if durable.iter().all(|change| change.mark == "−") {
            Verb::Remove
        } else {
            Verb::Change
        };
        let object = match durable.as_slice() {
            [one] => one.name.clone(),
            many => format!("{} items", many.len()),
        };
        Some(Self { verb, object })
    }

    /// The request, as an ask says it: "add 3 items to Family trip".
    pub fn ask(&self, space: &str) -> String {
        let object = &self.object;
        match self.verb {
            Verb::Add => format!("add {object} to {space}"),
            Verb::Remove => format!("remove {object} from {space}"),
            Verb::Change => format!("change {object} in {space}"),
            Verb::Run => format!("run {object} in {space}"),
        }
    }

    /// The main answer's words: "add 3 items".
    pub fn act(&self) -> String {
        let object = &self.object;
        match self.verb {
            Verb::Add => format!("add {object}"),
            Verb::Remove => format!("remove {object}"),
            Verb::Change => format!("change {object}"),
            Verb::Run => format!("run {object}"),
        }
    }

    /// The safe answer's words.
    fn decline(&self) -> &'static str {
        match self.verb {
            Verb::Add | Verb::Run => "not now",
            Verb::Change | Verb::Remove => "keep as is",
        }
    }

    /// The receipt, as a strip says it: "added 3 items to Family trip".
    pub fn done(&self, space: &str) -> String {
        let object = &self.object;
        match self.verb {
            Verb::Add => format!("added {object} to {space}"),
            Verb::Remove => format!("removed {object} from {space}"),
            Verb::Change => format!("changed {object} in {space}"),
            Verb::Run => format!("ran {object} in {space}"),
        }
    }
}

/// The ask for a change held as `id`, from its preview `evaluation`.
pub fn ask(context: &Context, id: &str, evaluation: &Value) -> Card {
    let changes = changed(evaluation);
    let space = context.place.label().to_owned();
    let summary = Summary::of(&changes);
    let (say, act, decline) = match &summary {
        Some(summary) => (summary.ask(&space), summary.act(), summary.decline()),
        // A write the preview lists nothing for, such as a rule.
        None => (
            format!("change {space}"),
            "change it".to_owned(),
            "keep as is",
        ),
    };
    let arguments = json!({ "space": context.place.did, "id": id });
    let mut card = Card::ask(
        context.place.clone(),
        context.role,
        say,
        context.agent.clone(),
    );
    for change in changes.iter().take(LISTED) {
        card = card.change(change.mark, change.name.clone(), change.kind.clone());
    }
    if changes.len() > LISTED {
        card = card.change("…", format!("{} more", changes.len() - LISTED), None);
    }
    card = card.entry("in", space.clone());
    card = if context.home {
        card.entry("who sees it", "you and this agent")
    } else {
        card.entry("who sees it", format!("everyone in {space}"))
    };
    card.entry("undo", "from the receipt, or from Activity in Tonk")
        .answer(act, Look::Main, APPLY, arguments.clone())
        .answer(decline, Look::Ring, DISMISS, arguments)
}

/// The receipt for a write whose answer is `evaluation`; `moved` says
/// whether it moved the branch's head.
pub fn receipt(context: &Context, evaluation: &Value, moved: bool) -> Card {
    let space = context.place.label();
    let strip = |say: String| {
        Card::strip(
            Some(context.place.clone()),
            context.role,
            say,
            context.agent.clone(),
        )
    };
    match Summary::of(&changed(evaluation)) {
        Some(summary) => strip(summary.done(space)).by_agent(),
        // A write the matches do not show, such as a rule or a schema.
        None if moved => strip(format!("changed {space}")).by_agent(),
        None => strip(format!("nothing changed in {space}")),
    }
}

/// The strip for a change that would change nothing, so is not asked.
pub fn nothing(context: &Context) -> Card {
    Card::strip(
        Some(context.place.clone()),
        context.role,
        format!("nothing changed in {}", context.place.label()),
        context.agent.clone(),
    )
}

/// The receipt for a change the person declined.
pub fn dismissed(context: &Context) -> Card {
    Card::strip(
        Some(context.place.clone()),
        context.role,
        format!("nothing changed · {} stays as it is", context.place.label()),
        context.agent.clone(),
    )
}

/// A tell that a change is not saved because the space could not be
/// reached: the hollow disc, and try again or drop it.
pub fn not_saved(context: &Context) -> Card {
    let space = context.place.label();
    Card::tell(
        Some(context.place.clone()),
        context.role,
        "not saved",
        format!("This change isn’t saved because Tonk can’t reach {space}. Nothing in it changed."),
        context.agent.clone(),
    )
    .disc(Disc::Hollow)
    .answer_saying(
        "try again",
        Look::Main,
        "asked to try again",
        "The person asked you to try the same change again. It was not saved: query the space \
         first, then write it once.",
    )
    .answer_saying(
        "drop it",
        Look::Link,
        format!("dropped · nothing changed in {space}"),
        "The person dropped the change that was not saved. Do not try it again unless they ask.",
    )
}

/// A tell that the space refused a change as written: it was reached,
/// so the disc stays filled, and nothing changed.
pub fn rejected(context: &Context, proposing: bool) -> Card {
    let space = context.place.label();
    let say = if proposing {
        format!(
            "I couldn’t prepare this change for {space}: it doesn’t fit the space as I wrote it."
        )
    } else {
        format!("I didn’t change {space}: the change doesn’t fit the space as I wrote it.")
    };
    Card::tell(
        Some(context.place.clone()),
        context.role,
        "not changed",
        say,
        context.agent.clone(),
    )
}

/// A tell that the space moved after the preview the person answered.
/// `again` is the same change held afresh at the space's new head, with
/// its preview, when it could be: its matches before are what the space
/// holds now (theirs), its matches after what the change would leave
/// (yours). Keep theirs drops it, use mine applies it. Who made the other
/// change is not known, so the sentence says "Someone".
pub fn moved(context: &Context, again: Option<(&str, &Value)>) -> Card {
    let changes = again
        .map(|(_, evaluation)| changed(evaluation))
        .unwrap_or_default();
    let one = match changes.as_slice() {
        [one] => Some(one),
        _ => None,
    };
    let mut place = context.place.clone();
    place.item = one.map(|one| one.name.clone());
    let label = format!(
        "{} changed",
        one.map_or(context.place.label(), |one| one.name.as_str())
    );
    let Some((id, evaluation)) = again else {
        return Card::tell(
            Some(place),
            context.role,
            label,
            "Someone changed this after I read it, so nothing is overwritten. Ask me to show it again.",
            context.agent.clone(),
        );
    };
    let arguments = json!({ "space": context.place.did, "id": id });
    let mut card = Card::tell(
        Some(place),
        context.role,
        label,
        "Someone changed this after I read it. Nothing is overwritten yet.",
        context.agent.clone(),
    );
    match one {
        Some(one) => {
            let theirs = fields(evaluation, "matches_before", &one.this);
            let yours = fields(evaluation, "matches_after", &one.this);
            let (theirs, yours) = differing(theirs.as_ref(), yours.as_ref());
            card = card.entry("theirs", theirs.unwrap_or_else(|| "not there".to_owned()));
            card = card.entry("yours", yours.unwrap_or_else(|| "removed".to_owned()));
        }
        None => {
            if let Some(summary) = Summary::of(&changes) {
                card = card.entry("yours", summary.act());
            }
        }
    }
    card.answer("keep theirs", Look::Main, DISMISS, arguments.clone())
        .answer("use mine", Look::Ring, APPLY, arguments)
}

/// The fields `evaluation`'s `key` matches hold for the entity `this`.
fn fields(evaluation: &Value, key: &str, this: &str) -> Option<serde_json::Map<String, Value>> {
    evaluation[key]
        .as_array()?
        .iter()
        .filter_map(|block| block["results"].as_array())
        .flatten()
        .find(|result| result["this"].as_str() == Some(this))
        .and_then(|result| result["fields"].as_object().cloned())
}

/// The fields that differ between `theirs` and `yours`, each side's
/// values in words: "4 nights · needs parking". All of a side's fields
/// where the other has none.
fn differing(
    theirs: Option<&serde_json::Map<String, Value>>,
    yours: Option<&serde_json::Map<String, Value>>,
) -> (Option<String>, Option<String>) {
    let words = |value: &Value| match value {
        Value::String(text) => text.clone(),
        Value::Null => "none".to_owned(),
        other => other.to_string(),
    };
    let side = |this: Option<&serde_json::Map<String, Value>>,
                other: Option<&serde_json::Map<String, Value>>| {
        let this = this?;
        let shown: Vec<String> = this
            .iter()
            .filter(|(field, value)| other.is_none_or(|other| other.get(*field) != Some(*value)))
            .map(|(_, value)| words(value))
            .collect();
        Some(if shown.is_empty() {
            this.values().map(words).collect::<Vec<_>>().join(" · ")
        } else {
            shown.join(" · ")
        })
    };
    (side(theirs, yours), side(yours, theirs))
}

/// A tell that a held change can no longer be answered.
pub fn gone(context: &Context) -> Card {
    Card::tell(
        Some(context.place.clone()),
        context.role,
        "not applied",
        "This was already answered, or waited too long, so nothing changed. Ask me again.",
        context.agent.clone(),
    )
}

/// A tell that this connection may not reach the space: neither disc nor
/// role, since the agent is not in it. `lapsed` when its access ran out.
/// The alternative is to grant it in Tonk.
pub fn no_access(context: &Context, lapsed: bool) -> Card {
    let space = context.place.label();
    let (label, say, act) = if lapsed {
        (
            format!("access ended in {space}"),
            format!(
                "My access to {space} has ended. You can grant it again from your agents in Tonk."
            ),
            "grant it again ↗",
        )
    } else {
        (
            format!("no access to {space}"),
            format!(
                "I can’t open {space}: this agent has no access to it. You can grant it from your agents in Tonk."
            ),
            "grant it in tonk ↗",
        )
    };
    Card::tell(
        Some(context.place.clone()),
        None,
        label,
        say,
        context.agent.clone(),
    )
    .answer_opening(act, Look::Ring, context.agents_in_tonk())
}

/// A tell that the connection may only read the space it was asked to
/// change. The alternatives: make the change in Tonk, or skip it.
pub fn read_only(context: &Context) -> Card {
    let space = context.place.label();
    Card::tell(
        Some(context.place.clone()),
        context.role,
        format!("read only in {space}"),
        format!("I can only read {space}, so I didn’t change it. You can make the change in Tonk, or let me write from your agents."),
        context.agent.clone(),
    )
    .entry("this agent", "read · sees and searches only")
    .entry("to let it write", "your agents in Tonk")
    .answer_opening("open in tonk ↗", Look::Main, context.in_tonk())
    .answer_saying(
        "skip",
        Look::Ring,
        format!("nothing changed · {space} stays as it is"),
        "The person skipped the change: this connection may only read the space. Do not try \
         to write it again unless they grant write access.",
    )
}

#[cfg(test)]
mod tests;
