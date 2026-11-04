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

use super::card::{Agent, Card, Disc, Place, Role};

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
    expires: u64,
}

/// Why a held change cannot be answered.
#[derive(Debug, PartialEq, Eq)]
pub struct Gone;

impl Proposals {
    /// Hold `document`, previewed at `revision`, until `now + LIFETIME`.
    /// Answers its id and when it lapses.
    pub fn hold(&self, document: String, revision: Value, now: u64) -> (String, u64) {
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
    pub fn take(&self, id: &str, now: u64) -> Result<(String, Value), Gone> {
        let mut held = self.held.lock();
        held.retain(|held| held.expires > now);
        let at = held.iter().position(|held| held.id == id).ok_or(Gone)?;
        let held = held.remove(at);
        Ok((held.document, held.revision))
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
}

/// One line of what a dry run or a write changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Changed {
    /// `+` added, `−` removed, `~` changed.
    pub mark: &'static str,
    /// The entity's name for a person.
    pub name: String,
    /// What it is: its expression's label, or `command`.
    pub kind: String,
}

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
        for result in &now {
            // A match that reads the same before and after is a query's
            // answer, or a write of what was there already: no change.
            if was.contains(result) {
                continue;
            }
            let existed = was.iter().any(|old| this(old) == this(result));
            let kind = if result["transient"].as_bool().unwrap_or(false) {
                "command".to_owned()
            } else {
                label.clone()
            };
            changed.push(Changed {
                mark: if existed { "~" } else { "+" },
                name: name_of(result, &label),
                kind,
            });
        }
        for result in &was {
            if !now.iter().any(|new| this(new) == this(result)) {
                changed.push(Changed {
                    mark: "−",
                    name: name_of(result, &label),
                    kind: label.clone(),
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

/// How many things `evaluation` changes, in words: "3 items", or "1
/// change" for a write that touches no listed entity.
fn count(changes: usize, claims: u64) -> String {
    match (changes, claims) {
        (1, _) => "1 item".to_owned(),
        (0, 1) => "1 change".to_owned(),
        (0, claims) => format!("{claims} changes"),
        (changes, _) => format!("{changes} items"),
    }
}

fn claims(evaluation: &Value) -> u64 {
    evaluation["commits"]["claims"].as_u64().unwrap_or(0)
}

/// The ask for a change held as `id`, from its dry run `evaluation`.
pub fn ask(context: &Context, id: &str, evaluation: &Value) -> Card {
    let changes = changed(evaluation);
    let what = count(changes.len(), claims(evaluation));
    let space = context.place.label().to_owned();
    let arguments = json!({ "space": context.place.did, "id": id });
    let mut card = Card::ask(
        context.place.clone(),
        context.role,
        format!("change {what} in {space}"),
        context.agent.clone(),
    );
    for change in changes.iter().take(LISTED) {
        card = card.change(change.mark, change.name.clone(), Some(change.kind.clone()));
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
    let agent = context
        .agent
        .name
        .clone()
        .unwrap_or_else(|| "this agent".to_owned());
    card.entry_bot("made by", agent)
        .fine("Nothing changes until you answer.")
        .answer("not now", false, DISMISS, arguments.clone())
        .answer(format!("change {what}"), true, APPLY, arguments)
}

/// The receipt for a write whose answer is `evaluation`.
pub fn receipt(context: &Context, evaluation: &Value) -> Card {
    let space = context.place.label();
    let changes = changed(evaluation).len();
    let claims = claims(evaluation);
    if claims == 0 {
        return Card::strip(
            Some(context.place.clone()),
            context.role,
            format!("nothing changed in {space}"),
            context.agent.clone(),
        );
    }
    Card::strip(
        Some(context.place.clone()),
        context.role,
        format!("saved {} in {space}", count(changes, claims)),
        context.agent.clone(),
    )
    .by_agent()
}

/// The receipt for a change the person declined.
pub fn dismissed(context: &Context) -> Card {
    Card::strip(
        Some(context.place.clone()),
        context.role,
        format!("kept as is · nothing changed in {}", context.place.label()),
        context.agent.clone(),
    )
}

/// A tell that a change was not saved, because of `reason`.
pub fn not_saved(context: &Context, reason: &str) -> Card {
    Card::tell(
        Some(context.place.clone()),
        context.role,
        "not saved",
        format!(
            "This change isn’t saved in {}. Ask me to try again.",
            context.place.label()
        ),
        context.agent.clone(),
    )
    .disc(Disc::Hollow)
    .fine(reason.to_owned())
}

/// A tell that the space moved after the preview the person answered.
pub fn moved(context: &Context) -> Card {
    Card::tell(
        Some(context.place.clone()),
        context.role,
        "changed since",
        format!(
            "{} changed after I showed you this, so nothing is overwritten. Ask me to show it again.",
            context.place.label()
        ),
        context.agent.clone(),
    )
}

/// A tell that a held change can no longer be answered.
pub fn gone(context: &Context) -> Card {
    Card::tell(
        Some(context.place.clone()),
        context.role,
        "not applied",
        "This was already answered, or waited too long. Nothing changed. Ask me again.",
        context.agent.clone(),
    )
}

#[cfg(test)]
mod tests;
