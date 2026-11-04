//! One card for asks, tells and strips, which a host shows inline (MCP
//! Apps) from a tool result's `structuredContent.card` ([`Card`]).
//!
//! - an ask (`.tonk--ask`): the request in one line, what will change, a
//!   button for each answer. Never a view.
//! - a tell (`.tonk--tell`): one sentence naming the problem, then what to
//!   do instead.
//! - a strip (`.tonk-strip`): one line, the receipt of a change. An
//!   answered ask becomes one.
//!
//! The card draws only what the server wrote: every word, and for each
//! answer the app-only tool it calls. It holds no state of its own, so
//! what an answer does is checked where the tool runs (see
//! [`super::propose`]).

use serde::Serialize;
use serde_json::{Value, json};

/// The card's resource.
pub const CARD_URI: &str = "ui://tonk/card-v1.html";

/// The MIME type MCP Apps hosts render.
const CARD_MIME: &str = "text/html;profile=mcp-app";

/// The card, with the in-agent stylesheet's mark in its head.
const CARD: &str = include_str!("card/card.html");

/// Resources served by `resources/list`.
pub fn resources() -> Vec<Value> {
    vec![json!({
        "uri": CARD_URI,
        "name": "tonk-card",
        "title": "Tonk",
        "mimeType": CARD_MIME,
    })]
}

/// The `resources/read` result for `uri`, or `None` when it is not the
/// card. The card talks only to the host; from `origin`, the provider the
/// host reached, it loads nothing but Tonk's fonts.
pub fn read_resource(uri: &str, origin: &str) -> Option<Value> {
    if uri != CARD_URI {
        return None;
    }
    let origin = origin.trim_end_matches('/');
    Some(json!({
        "contents": [{
            "uri": CARD_URI,
            "mimeType": CARD_MIME,
            "text": super::in_agent::splice(CARD, origin),
            "_meta": {
                "ui": {
                    "csp": { "connectDomains": [], "resourceDomains": [origin] },
                    "prefersBorder": true,
                },
                "openai/widgetCSP": { "connect_domains": [], "resource_domains": [origin] },
            },
        }],
    }))
}

/// The `_meta` of a tool whose result the card draws. The card calls
/// app-only tools, so ChatGPT must let the widget call tools.
pub fn tool_meta() -> Value {
    json!({
        "ui": { "resourceUri": CARD_URI },
        "openai/outputTemplate": CARD_URI,
        "openai/widgetAccessible": true,
    })
}

/// What a card draws.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Card {
    /// Which of the three it is.
    pub kind: Kind,
    /// The space, and the item in it, that the card is about.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub space: Option<Place>,
    /// The role in the space. `None` where the person is not in it: the
    /// head then shows neither the disc nor a role.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<Role>,
    /// The disc's state, where there is a role.
    pub disc: Disc,
    /// The one line: the request of an ask, the problem of a tell, the
    /// receipt of a strip.
    pub say: String,
    /// What a tell is about, for its label ("Tonk: not saved").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// That the agent made the change a strip receipts, which it ends
    /// with: "by you" and the bot tag.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub by_agent: bool,
    /// What an ask will change, one line each.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub changes: Vec<Change>,
    /// Lines of a text the space keeps, as a diff (`+ `, `- `, `  `).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lines: Option<String>,
    /// Terms and what they are, as a ledger.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub ledger: Vec<Entry>,
    /// The fine print.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fine: Option<String>,
    /// A button for each answer, the main one first: the card draws them
    /// left to right, as the in-agent mocks do.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub answers: Vec<Answer>,
    /// A strip's undo: the link at the end of its receipt, and the
    /// app-only tool that takes the change back. Its arguments are the
    /// change in the result's `_meta`, which the model does not read.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub undo: Option<Undo>,
    /// The agent: its account's name, shown with the bot tag.
    pub agent: Agent,
    /// The person the agent works for.
    pub person: Person,
}

/// Which card.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// A request, with an answer for each choice.
    Ask,
    /// A problem, and what to do instead.
    Tell,
    /// One line: a receipt.
    Strip,
}

/// The place a card names: a space, and an item in it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Place {
    /// The space's DID.
    pub did: String,
    /// Its name, when it has one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The item, shown as `› item`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item: Option<String>,
}

impl Place {
    /// The name to write in a sentence.
    pub fn label(&self) -> &str {
        self.name.as_deref().unwrap_or("this space")
    }
}

/// A role in a space. Today what a connection may do; admin is not
/// modelled yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// Sees and searches the space.
    Read,
    /// Also adds and changes content.
    Write,
}

/// The disc's three states.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Disc {
    /// Connected and synced.
    Filled,
    /// Local only.
    Hollow,
    /// Paused on purpose.
    Paused,
}

/// One line of what an ask will change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Change {
    /// `+` adds, `−` removes, `~` changes.
    pub mark: String,
    /// What changes.
    pub name: String,
    /// What kind of thing it is, or how it changes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
}

/// One term of a ledger.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Entry {
    /// The term, on the left.
    pub term: String,
    /// What it is.
    pub value: String,
    /// Ends the value with the bot tag, for an agent's name.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub bot: bool,
}

/// One answer, and what it does.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Answer {
    /// The button's words, lowercase.
    pub label: String,
    /// How it is drawn.
    pub kind: Look,
    /// What pressing it does.
    #[serde(flatten)]
    pub does: Does,
}

/// How an answer is drawn: the main one solid and first, the others a
/// ring or a link after it, as the in-agent asks and tells draw them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Look {
    /// A solid button: the answer the card leads with.
    Main,
    /// A ringed button.
    Ring,
    /// An underlined link, for the least weighty answer.
    Link,
}

/// What an answer does.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum Does {
    /// Calls the app-only `tool` with `arguments`; its result is the next
    /// card, and its text is what the model is told.
    Call {
        /// The tool.
        tool: String,
        /// Its arguments.
        arguments: Value,
    },
    /// Opens `url` in Tonk, through the agent app (`ui/open-link`); the
    /// card stays as it is.
    Open {
        /// Where, on the provider's origin.
        url: String,
    },
    /// Calls nothing: the card becomes the strip `receipt`, and the model
    /// is told `context`.
    Say {
        /// The strip's words.
        receipt: String,
        /// What the model is told.
        context: String,
    },
}

/// A strip's undo link.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Undo {
    /// The link's words, lowercase.
    pub label: String,
    /// The app-only tool it calls, with the change in the result's
    /// `_meta`.
    pub tool: String,
}

/// The agent a card names.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Agent {
    /// The agent space's DID.
    pub did: String,
    /// Its account's name.
    pub name: Option<String>,
}

/// The person a card is shown to.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Person {
    /// Their name. Nothing the agent space holds records it yet, so it is
    /// absent and the card says "you".
    pub name: Option<String>,
}

impl Card {
    fn new(
        kind: Kind,
        space: Option<Place>,
        role: Option<Role>,
        say: String,
        agent: Agent,
    ) -> Self {
        Self {
            kind,
            space,
            role,
            disc: Disc::Filled,
            say,
            label: None,
            by_agent: false,
            changes: Vec::new(),
            lines: None,
            ledger: Vec::new(),
            fine: None,
            answers: Vec::new(),
            undo: None,
            agent,
            person: Person::default(),
        }
    }

    /// An ask in `space`, requesting `say`.
    pub fn ask(space: Place, role: Option<Role>, say: impl Into<String>, agent: Agent) -> Self {
        Self::new(Kind::Ask, Some(space), role, say.into(), agent)
    }

    /// A tell about `label`, saying `say`.
    pub fn tell(
        space: Option<Place>,
        role: Option<Role>,
        label: impl Into<String>,
        say: impl Into<String>,
        agent: Agent,
    ) -> Self {
        let mut card = Self::new(Kind::Tell, space, role, say.into(), agent);
        card.label = Some(label.into());
        card
    }

    /// A strip with `receipt`.
    pub fn strip(
        space: Option<Place>,
        role: Option<Role>,
        receipt: impl Into<String>,
        agent: Agent,
    ) -> Self {
        Self::new(Kind::Strip, space, role, receipt.into(), agent)
    }

    /// The receipt names the agent as who made the change.
    pub fn by_agent(mut self) -> Self {
        self.by_agent = true;
        self
    }

    /// A line of what will change.
    pub fn change(mut self, mark: &str, name: impl Into<String>, kind: Option<String>) -> Self {
        self.changes.push(Change {
            mark: mark.to_owned(),
            name: name.into(),
            kind,
        });
        self
    }

    /// A term of the ledger.
    pub fn entry(mut self, term: impl Into<String>, value: impl Into<String>) -> Self {
        self.ledger.push(Entry {
            term: term.into(),
            value: value.into(),
            bot: false,
        });
        self
    }

    /// A term of the ledger whose value is an agent's name.
    pub fn entry_bot(mut self, term: impl Into<String>, value: impl Into<String>) -> Self {
        self.ledger.push(Entry {
            term: term.into(),
            value: value.into(),
            bot: true,
        });
        self
    }

    /// The fine print.
    pub fn fine(mut self, fine: impl Into<String>) -> Self {
        self.fine = Some(fine.into());
        self
    }

    /// An answer that calls `tool`, in order: the main one first.
    pub fn answer(
        mut self,
        label: impl Into<String>,
        kind: Look,
        tool: &str,
        arguments: Value,
    ) -> Self {
        self.answers.push(Answer {
            label: label.into(),
            kind,
            does: Does::Call {
                tool: tool.to_owned(),
                arguments,
            },
        });
        self
    }

    /// An answer that calls nothing: the card becomes the strip
    /// `receipt`, and the model is told `context`.
    pub fn answer_saying(
        mut self,
        label: impl Into<String>,
        kind: Look,
        receipt: impl Into<String>,
        context: impl Into<String>,
    ) -> Self {
        self.answers.push(Answer {
            label: label.into(),
            kind,
            does: Does::Say {
                receipt: receipt.into(),
                context: context.into(),
            },
        });
        self
    }

    /// Ends a strip's receipt with undo, which calls `tool` with the
    /// change the result carries in `_meta`.
    pub fn undo(mut self, tool: &str) -> Self {
        self.undo = Some(Undo {
            label: "undo".to_owned(),
            tool: tool.to_owned(),
        });
        self
    }

    /// An answer that opens `url` in Tonk.
    pub fn answer_opening(mut self, label: impl Into<String>, kind: Look, url: String) -> Self {
        self.answers.push(Answer {
            label: label.into(),
            kind,
            does: Does::Open { url },
        });
        self
    }

    /// The disc's state.
    pub fn disc(mut self, disc: Disc) -> Self {
        self.disc = disc;
        self
    }

    /// As `structuredContent.card`.
    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).expect("a card serializes")
    }
}

#[cfg(test)]
mod tests;
