//! Installing and uninstalling a library component from an agent chat.
//!
//! A `library/install!:` or `library/uninstall!:` is a command, and a
//! command's effects do not show in a dry run, so the generic propose
//! path refuses it. These two are known ahead: the catalog on the space
//! says what a component adds (`library/component`, `library/manifest`),
//! so the ask lists that, and the writer runs the command itself when the
//! person answers ([`ROUTE`]), awaiting its outcome rather than answering
//! before it ran, as a command's dispatch would.
//!
//! A receipt's undo runs the inverse: an install is undone by uninstalling
//! the component, and an uninstall by installing it again ([`UNDO`]).

use serde_json::{Value, json};

use super::card::{Card, Look};
use super::propose::{APPLY, Context, DISMISS};
pub use crate::router::library::{Action, Catalog};

/// The writer route that runs one library command and answers its
/// outcome: `POST`, `{"action", "component"}`.
pub const ROUTE: &str = "/api/library";

/// The app-only tool a receipt's undo calls, with the change in `_meta`:
/// `{space, component, action}`, `action` being the one the undo runs.
pub const UNDO: &str = "tonk_library_undo";

/// The changes an ask lists before it says how many more there are.
const LISTED: usize = 5;

/// A library command a document asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Command {
    /// Install or uninstall.
    pub action: Action,
    /// `tonk:library/<name>`.
    pub component: String,
}

impl Command {
    /// The body [`ROUTE`] takes.
    pub fn body(&self) -> String {
        json!({ "action": self.action.word(), "component": self.component }).to_string()
    }

    /// The command a [`ROUTE`] body names.
    pub fn from_body(body: &Value) -> Option<Self> {
        let action = Action::from_word(body["action"].as_str()?)?;
        let component = body["component"].as_str()?;
        component.starts_with("tonk:library/").then(|| Self {
            action,
            component: component.to_owned(),
        })
    }
}

/// The library command `document` is, when it is one and nothing else:
/// one `library/install!:` or `library/uninstall!:` naming a component.
pub fn command_of(document: &str) -> Option<Command> {
    use tonk_notation::{Expression, FieldValue};

    let syntax = tonk_notation::parse(document).syntax?;
    let [Expression::Claim(claim)] = syntax.expressions.as_slice() else {
        return None;
    };
    let action = match claim.inner.predicate.source.as_str() {
        "library/install" => Action::Install,
        "library/uninstall" => Action::Uninstall,
        _ => return None,
    };
    let component = claim
        .inner
        .fields
        .iter()
        .find(|field| field.name == "component")
        .and_then(|field| match &field.value {
            FieldValue::Uri(uri) | FieldValue::Symbol(uri) => Some(uri.clone()),
            _ => None,
        })?;
    component
        .starts_with("tonk:library/")
        .then_some(Command { action, component })
}

/// What [`ROUTE`] answers: the outcome the command recorded, and the
/// catalog entry of its component.
pub fn answered(outcome: &tonk_schema::LibraryOutcome, catalog: &Catalog) -> Value {
    json!({
        "library": {
            "action": outcome.action.0,
            "component": outcome.component.0.to_string(),
            "status": outcome.status.0,
            "version": outcome.version.as_ref().map(|version| version.0.clone()),
            "lines": outcome.lines.as_ref().map(|lines| lines.0.clone()),
            "reason": outcome.reason.as_ref().map(|reason| reason.0.clone()),
            "catalog": catalog,
        }
    })
}

/// What the propose route answers for a library command: the held id, and
/// the catalog in place of a dry run.
pub fn held(id: &str, expires: u64, command: &Command, catalog: &Catalog) -> Value {
    json!({
        "proposal": { "id": id, "expires": expires },
        "library": {
            "action": command.action.word(),
            "component": command.component,
            "catalog": catalog,
        },
    })
}

/// A part line of the manifest as an ask lists it: (name, kind).
fn part(line: &str) -> Option<(String, String)> {
    let mut words = line.split_whitespace();
    let kind = words.next()?;
    let name = words.next()?;
    Some((name.to_owned(), kind.to_owned()))
}

fn catalog_of(library: &Value) -> Catalog {
    serde_json::from_value(library["catalog"].clone()).unwrap_or_default()
}

fn action_of(library: &Value) -> Action {
    library["action"]
        .as_str()
        .and_then(Action::from_word)
        .unwrap_or(Action::Install)
}

/// The ask for a library command held as `id`; `library` is the held
/// answer's `library`.
pub fn ask(context: &Context, id: &str, library: &Value) -> Card {
    let catalog = catalog_of(library);
    let action = action_of(library);
    let space = context.place.label().to_owned();
    let label = &catalog.label;
    let (say, act, decline, mark, line) = match action {
        Action::Install => (
            format!("install {label} in {space}"),
            "install",
            "not now",
            "+",
            "+",
        ),
        Action::Uninstall => (
            format!("uninstall {label} from {space}"),
            "uninstall",
            "keep it",
            "−",
            "-",
        ),
    };
    let arguments = json!({ "space": context.place.did, "id": id });
    let mut card = Card::ask(
        context.place.clone(),
        context.role,
        say,
        context.agent.clone(),
    );
    // Views first, as the mock lists them, each named once.
    let mut parts: Vec<(String, String)> = Vec::new();
    for kind in ["view", "concept", "command"] {
        for (name, of) in catalog.adds.iter().filter_map(|line| part(line)) {
            if of == kind && !parts.contains(&(name.clone(), of.clone())) {
                parts.push((name, of));
            }
        }
    }
    for (name, kind) in parts.iter().take(LISTED) {
        card = card.change(mark, name.clone(), Some(kind.clone()));
    }
    if parts.len() > LISTED {
        card = card.change("…", format!("{} more", parts.len() - LISTED), None);
    }
    if !catalog.agents.is_empty() {
        let count = catalog.agents.len();
        let lines = if count == 1 { "line" } else { "lines" };
        card = card.change("~", "AGENTS.md", Some(format!("{mark} {count} {lines}")));
        card.lines = Some(
            catalog
                .agents
                .iter()
                .map(|agent| format!("{line} {agent}"))
                .collect::<Vec<_>>()
                .join("\n"),
        );
    }
    let version = match &catalog.version {
        Some(version) => format!("{} {version}", catalog.name),
        None => catalog.name.clone(),
    };
    let who = if context.home {
        "Only you and this agent see it.".to_owned()
    } else {
        match action {
            Action::Install => format!("Everyone in {space} gets the view."),
            Action::Uninstall => format!("It goes for everyone in {space}."),
        }
    };
    card.fine(format!("From the Tonk library, {version}. {who}"))
        .answer(act, Look::Main, APPLY, arguments.clone())
        .answer(decline, Look::Ring, DISMISS, arguments)
}

/// The change a receipt's undo is called with: the inverse command.
pub fn change(space: &str, library: &Value) -> Value {
    json!({
        "space": space,
        "component": library["component"],
        "action": action_of(library).inverse().word(),
    })
}

/// The receipt for a library command whose answer is `library` (the
/// [`ROUTE`] answer's `library`), with undo where it changed the space,
/// or a tell when it failed. `undoing` when the command ran as an undo.
pub fn receipt(context: &Context, library: &Value, undoing: bool) -> Card {
    let catalog = catalog_of(library);
    let label = &catalog.label;
    let space = context.place.label();
    let strip = |say: String| {
        Card::strip(
            Some(context.place.clone()),
            context.role,
            say,
            context.agent.clone(),
        )
    };
    match library["status"].as_str().unwrap_or_default() {
        "installed" if undoing => strip(format!("undone · {label} is back in {space}")),
        "uninstalled" if undoing => strip(format!("undone · {label} is no longer in {space}")),
        "installed" => strip(format!("installed {label} in {space}"))
            .by_agent()
            .undo(UNDO),
        "uninstalled" => strip(format!("uninstalled {label} from {space}"))
            .by_agent()
            .undo(UNDO),
        "current" => strip(format!("{label} is in {space} already · nothing changed")),
        _ => {
            let (title, say) = match action_of(library) {
                Action::Install => (
                    "not installed",
                    format!("I couldn’t install {label} in {space}, so nothing changed."),
                ),
                Action::Uninstall => (
                    "not uninstalled",
                    format!("I couldn’t uninstall {label} from {space}, so nothing changed."),
                ),
            };
            let mut card = Card::tell(
                Some(context.place.clone()),
                context.role,
                title,
                say,
                context.agent.clone(),
            );
            if let Some(reason) = library["reason"].as_str() {
                card = card.fine(reason.to_owned());
            }
            card
        }
    }
}

/// The receipt for a library command the person declined: "not
/// installed · nothing changed in Launch".
pub fn declined(context: &Context, library: &Value) -> Card {
    let space = context.place.label();
    let not = match action_of(library) {
        Action::Install => "not installed",
        Action::Uninstall => "not uninstalled",
    };
    Card::strip(
        Some(context.place.clone()),
        context.role,
        format!("{not} · nothing changed in {space}"),
        context.agent.clone(),
    )
}

/// What the model reads of a library command's answer.
pub fn told(context: &Context, library: &Value) -> String {
    let catalog = catalog_of(library);
    let space = context.place.label();
    let mut text = match library["status"].as_str().unwrap_or_default() {
        "installed" => format!(
            "Installed {} in {space}: its views, concepts and commands are in the space now.",
            catalog.label
        ),
        "uninstalled" => format!(
            "Uninstalled {} from {space}: what it added is gone; what the space wrote with it stays.",
            catalog.label
        ),
        "current" => format!(
            "{} is installed in {space} already and up to date; nothing changed.",
            catalog.label
        ),
        _ => format!(
            "The {} of {} in {space} failed: {}. Nothing changed.",
            action_of(library).word(),
            catalog.label,
            library["reason"].as_str().unwrap_or("no reason given")
        ),
    };
    if let Some(lines) = library["lines"].as_str() {
        text.push_str(&format!(" AGENTS.md lines changed:\n{lines}"));
    }
    text
}

/// A tell that a change runs a command whose effects cannot be shown
/// before it runs, so it cannot be confirmed first: nothing changed.
pub fn not_previewable(context: &Context) -> Card {
    let space = context.place.label();
    Card::tell(
        Some(context.place.clone()),
        context.role,
        "can’t preview",
        format!(
            "This change runs a command, and I can’t show what it does before it runs, so I \
             didn’t ask you to confirm it. Nothing changed in {space}."
        ),
        context.agent.clone(),
    )
}

/// The tool's `tools/list` entry: visible to the app alone.
pub fn tool() -> Value {
    json!({
        "name": UNDO,
        "description": "Undo a library install or uninstall a receipt shows. Called by the card.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "space": { "type": "string", "pattern": "^did:key:[A-Za-z0-9]+$" },
                "component": { "type": "string", "pattern": "^tonk:library/[a-z0-9-]+$" },
                "action": { "type": "string", "enum": ["install", "uninstall"] },
            },
            "required": ["space", "component", "action"],
            "additionalProperties": false,
        },
        "annotations": { "destructiveHint": true, "openWorldHint": false },
        "_meta": { "ui": { "visibility": ["app"] } },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_service_worker);

    #[dialog_common::test]
    fn it_reads_a_library_command_and_nothing_else() {
        assert_eq!(
            command_of("library/install!:\n  component: tonk:library/table\n  time: 1.0\n"),
            Some(Command {
                action: Action::Install,
                component: "tonk:library/table".to_owned()
            })
        );
        assert_eq!(
            command_of("library/uninstall!:\n  component: tonk:library/prose\n")
                .map(|command| command.action),
            Some(Action::Uninstall)
        );
        for other in [
            "library/install:\n  component: tonk:library/table\n",
            "note!:\n  this: urn:x\n  title: \"x\"\n",
            "library/install!:\n  component: urn:elsewhere\n",
            "library/install!:\n  component: tonk:library/table\nnote!:\n  this: urn:x\n  title: \"x\"\n",
        ] {
            assert_eq!(command_of(other), None, "{other}");
        }
    }

    fn context(home: bool) -> Context {
        use super::super::card::{Agent, Place};
        Context {
            place: Place {
                did: "did:key:zLaunch".into(),
                name: Some("Launch".into()),
                item: None,
            },
            role: None,
            agent: Agent::default(),
            home,
            origin: "https://tonk.example".into(),
        }
    }

    /// The ask says what the mock's does: the request in one line, the
    /// views, concepts and commands it adds, its AGENTS.md lines, and
    /// where it is from.
    #[dialog_common::test]
    fn it_asks_to_install_with_what_the_component_adds() {
        let library = json!({
            "action": "install",
            "component": "tonk:library/decision-log",
            "catalog": {
                "name": "decision-log",
                "label": "Decision log",
                "version": "1.2",
                "agents": ["Log decisions in Decision log with log-decision.", "Link each decision to the proposal it closes."],
                "adds": ["concept decision", "command log-decision", "view decision ui", "view decision directory"],
            },
        });
        let card = ask(&context(false), "id", &library).to_value();
        assert_eq!(card["say"], "install Decision log in Launch");
        let changes: Vec<(String, String, String)> = card["changes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|change| {
                (
                    change["mark"].as_str().unwrap().to_owned(),
                    change["name"].as_str().unwrap().to_owned(),
                    change["kind"].as_str().unwrap_or_default().to_owned(),
                )
            })
            .collect();
        let expected: Vec<(String, String, String)> = [
            ("+", "decision", "view"),
            ("+", "decision", "concept"),
            ("+", "log-decision", "command"),
            ("~", "AGENTS.md", "+ 2 lines"),
        ]
        .iter()
        .map(|(a, b, c)| ((*a).to_owned(), (*b).to_owned(), (*c).to_owned()))
        .collect();
        assert_eq!(changes, expected);
        assert_eq!(
            card["lines"],
            "+ Log decisions in Decision log with log-decision.\n+ Link each decision to the proposal it closes."
        );
        assert_eq!(
            card["fine"],
            "From the Tonk library, decision-log 1.2. Everyone in Launch gets the view."
        );
        assert_eq!(card["answers"][0]["label"], "install");
        assert_eq!(card["answers"][1]["label"], "not now");
    }

    #[dialog_common::test]
    fn it_receipts_an_install_and_its_undo_as_the_mock_does() {
        let library = json!({
            "action": "install",
            "component": "tonk:library/decision-log",
            "status": "installed",
            "catalog": { "name": "decision-log", "label": "Decision log", "agents": [], "adds": [] },
        });
        let card = receipt(&context(false), &library, false).to_value();
        assert_eq!(card["say"], "installed Decision log in Launch");
        assert_eq!(card["byAgent"], json!(true));
        assert_eq!(card["undo"]["tool"], UNDO);
        assert_eq!(change("did:key:zLaunch", &library)["action"], "uninstall");

        let undone = json!({ "action": "uninstall", "status": "uninstalled", "catalog": library["catalog"] });
        let card = receipt(&context(false), &undone, true).to_value();
        assert_eq!(card["say"], "undone · Decision log is no longer in Launch");
        assert!(card.get("undo").is_none());

        let declined = declined(&context(false), &json!({ "action": "install" })).to_value();
        assert_eq!(declined["say"], "not installed · nothing changed in Launch");

        let failed = json!({ "action": "install", "status": "failed", "reason": "no such file", "catalog": library["catalog"] });
        let card = receipt(&context(false), &failed, false).to_value();
        assert_eq!(card["kind"], "tell");
        assert_eq!(card["label"], "not installed");
        assert_eq!(card["fine"], "no such file");
    }

    #[dialog_common::test]
    fn it_round_trips_the_route_body() {
        let command = Command {
            action: Action::Uninstall,
            component: "tonk:library/table".to_owned(),
        };
        let body: Value = serde_json::from_str(&command.body()).unwrap();
        assert_eq!(Command::from_body(&body), Some(command));
        assert_eq!(
            Command::from_body(&json!({ "action": "install", "component": "urn:x" })),
            None
        );
    }
}
