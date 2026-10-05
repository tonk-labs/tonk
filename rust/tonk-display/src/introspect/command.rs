//! What a template binds an interaction to, described without the DOM.
//!
//! A template wires an interaction two ways. The older form writes the
//! platform event into the attribute name — `onclick="space/create"`,
//! which the renderer rewrites to `data-onclick` so the browser does
//! not try to evaluate it as inline JS. The newer form names an
//! `event!:` declaration instead — `on:click="space/create"` — and the
//! platform event lives on that declaration, which is what lets two
//! declarations read the same event differently.
//!
//! For introspection the difference matters in exactly one way: the
//! older form tells you its trigger by looking at it, and the newer one
//! does not. A declaration that resolved to nothing installs no
//! listener and reports nothing — the element is simply inert — so an
//! unresolved trigger is a finding worth painting, not a blank to hide.

use serde::{Deserialize, Serialize};

/// One interaction a rendered element binds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Command {
    /// A stable-per-snapshot identifier.
    pub id: u32,
    /// The command posted when it fires — a bare name
    /// (`space/create`) or a URI (`tonk:invite`).
    pub command: String,
    /// The platform event that triggers it. `None` when the binding
    /// names a declaration that did not resolve, which means nothing
    /// is listening and the element is inert.
    pub event_type: Option<String>,
    /// The `event!:` declaration the binding names (`on/click`), or
    /// `None` for the older `on<event>=` form, which carries its
    /// trigger in the attribute name instead.
    pub declaration: Option<String>,
    /// The attribute as it appears in the rendered DOM, so a panel can
    /// point at what an author actually wrote.
    pub attribute: String,
}

impl Command {
    /// The short label for the indicator badge: `click → space/create`,
    /// or `?? → space/create` when nothing is listening.
    pub fn label(&self) -> String {
        let trigger = self.event_type.as_deref().unwrap_or("??");
        format!("{trigger} \u{2192} {}", self.command)
    }

    /// Whether this binding is live. A binding whose declaration did
    /// not resolve has no listener installed for it and will never
    /// fire, however correct the template looks.
    pub fn is_live(&self) -> bool {
        self.event_type.is_some()
    }
}

/// Classify one element's attributes into the commands it binds.
///
/// `event_type_of` resolves a declaration name (`on/click`) to its
/// platform event type, which is the [`EventTable`] the display built
/// for its delegate. It returns `None` for a declaration that is not
/// in the table — a dangling reference, or a table that has not been
/// built yet.
///
/// [`EventTable`]: tonk_template::event::EventTable
pub fn commands_on(
    attributes: &[(String, String)],
    event_type_of: impl Fn(&str) -> Option<String>,
    next_id: &mut u32,
) -> Vec<Command> {
    let mut out = Vec::new();
    for (name, value) in attributes {
        let command = value.trim();
        if command.is_empty() {
            continue;
        }
        let (event_type, declaration) =
            if let Some(declaration) = tonk_template::event::event_name_for_attribute(name) {
                let event_type = event_type_of(&declaration);
                (event_type, Some(declaration))
            } else if let Some(event_type) = name.strip_prefix(RENDERED_PREFIX) {
                if event_type.is_empty() {
                    continue;
                }
                (Some(event_type.to_owned()), None)
            } else {
                continue;
            };
        let id = *next_id;
        *next_id += 1;
        out.push(Command {
            id,
            command: command.to_owned(),
            event_type,
            declaration,
            attribute: name.clone(),
        });
    }
    out
}

/// What the renderer rewrites an `on<event>=` attribute to before the
/// browser can read it as inline JS. See `events::preprocess`.
const RENDERED_PREFIX: &str = "data-on";

#[cfg(test)]
mod tests {
    use super::*;

    fn attributes(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect()
    }

    fn resolving(event_type: &'static str) -> impl Fn(&str) -> Option<String> {
        move |_| Some(event_type.to_owned())
    }

    fn resolving_nothing(_: &str) -> Option<String> {
        None
    }

    #[test]
    fn a_rewritten_handler_carries_its_trigger_in_the_attribute_name() {
        let mut id = 0;
        let found = commands_on(
            &attributes(&[("data-onclick", "space/create")]),
            resolving_nothing,
            &mut id,
        );
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].command, "space/create");
        assert_eq!(found[0].event_type.as_deref(), Some("click"));
        assert_eq!(found[0].declaration, None);
        assert!(found[0].is_live());
    }

    #[test]
    fn a_declaration_binding_takes_its_trigger_from_the_table() {
        let mut id = 0;
        let found = commands_on(
            &attributes(&[("on:click", "space/create")]),
            resolving("pointerdown"),
            &mut id,
        );
        assert_eq!(found[0].declaration.as_deref(), Some("on/click"));
        assert_eq!(
            found[0].event_type.as_deref(),
            Some("pointerdown"),
            "the declaration decides the trigger, not the attribute's spelling"
        );
    }

    #[test]
    fn a_binding_whose_declaration_did_not_resolve_is_not_live() {
        let mut id = 0;
        let found = commands_on(
            &attributes(&[("on:click", "space/create")]),
            resolving_nothing,
            &mut id,
        );
        assert_eq!(found.len(), 1, "an inert binding is still worth showing");
        assert!(!found[0].is_live());
        assert_eq!(found[0].label(), "?? \u{2192} space/create");
    }

    #[test]
    fn it_ignores_attributes_that_bind_nothing() {
        let mut id = 0;
        let found = commands_on(
            &attributes(&[
                ("class", "card"),
                ("data-this", "did:key:a"),
                ("data-onclick", "   "),
                ("on:click", ""),
            ]),
            resolving("click"),
            &mut id,
        );
        assert!(found.is_empty());
    }

    #[test]
    fn it_numbers_bindings_across_elements() {
        let mut id = 0;
        let first = commands_on(
            &attributes(&[("data-onclick", "a")]),
            resolving_nothing,
            &mut id,
        );
        let second = commands_on(
            &attributes(&[("data-onsubmit", "b")]),
            resolving_nothing,
            &mut id,
        );
        assert_eq!(first[0].id, 0);
        assert_eq!(second[0].id, 1);
    }

    #[test]
    fn one_element_can_bind_several_triggers() {
        let mut id = 0;
        let found = commands_on(
            &attributes(&[("data-onclick", "a"), ("data-onkeydown", "b")]),
            resolving_nothing,
            &mut id,
        );
        let labels: Vec<String> = found.iter().map(Command::label).collect();
        assert_eq!(labels, ["click \u{2192} a", "keydown \u{2192} b"]);
    }
}
