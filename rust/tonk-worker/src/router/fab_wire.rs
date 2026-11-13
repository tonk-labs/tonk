//! The wire the bar's elements speak, read off the profile library for the
//! tests that hold the worker to it.
//!
//! `<tonk-fab>`, `<tonk-share>` and `<tonk-agent-panel>` are `element!:`
//! definitions in `profile.yaml`. Each keeps the queries it subscribes to
//! and the commands it dispatches in one `wire` method, a JSON table
//! returned from `() => (...)`. These helpers read that table out of the
//! same text the profile is seeded with, so a test runs the very query
//! the element sends rather than a copy that could drift from it.

use serde_json::Value;

/// The library the profile is seeded with. Embedded, since a nextest
/// archive carries no data files beside the binary.
const PROFILE_LIBRARY: &str = include_str!("../../../tonk-core/assets/library/profile.yaml");

/// The source of method `key` of `element!: &{tag}`, dedented.
pub(crate) fn element_method(tag: &str, key: &str) -> String {
    let heading = format!("element!: &{tag}");
    let mut lines = PROFILE_LIBRARY
        .lines()
        .skip_while(|line| *line != heading)
        .skip(1)
        .take_while(|line| line.is_empty() || line.starts_with(' '))
        .skip_while(|line| *line != "  method:")
        .skip(1)
        .skip_while(|line| *line != format!("    {key}: |"))
        .skip(1)
        .peekable();
    let mut source = String::new();
    while let Some(line) = lines.next_if(|line| line.is_empty() || line.starts_with("      ")) {
        source.push_str(line.strip_prefix("      ").unwrap_or(""));
        source.push('\n');
    }
    assert!(!source.is_empty(), "<{tag}> declares no method `{key}`");
    source
}

/// The `wire` table of `element!: &{tag}`.
pub(crate) fn wire(tag: &str) -> Value {
    let source = element_method(tag, "wire");
    let table = source
        .trim()
        .strip_prefix("() => (")
        .and_then(|rest| rest.strip_suffix(')'))
        .unwrap_or_else(|| panic!("<{tag}>'s wire is not `() => (<json>)`"));
    serde_json::from_str(table)
        .unwrap_or_else(|error| panic!("<{tag}>'s wire is not JSON: {error}"))
}

/// Query `name` of `tag`'s wire as the element sends it, with `this` bound
/// to `subject` when given, serialized for the wire.
pub(crate) fn query(tag: &str, name: &str, subject: Option<&str>) -> String {
    let mut query = wire(tag)["queries"][name].clone();
    assert!(query.is_object(), "<{tag}> has no query `{name}`");
    if let Some(subject) = subject {
        query["terms"]["this"] = Value::String(subject.to_owned());
    }
    query.to_string()
}

#[cfg(test)]
mod tests {
    use dialog_reactor::Decode;
    use tonk_schema::command::{
        AgentHandoff, EnableSync, ForgetInvite, Invite, PauseSync, PublishSpace, UnpublishSpace,
    };

    use super::*;

    /// The attributes command `name` of `tag` declares.
    fn declared(tag: &str, name: &str) -> Vec<String> {
        let command = &wire(tag)["commands"][name];
        command["with"]
            .as_object()
            .unwrap_or_else(|| panic!("<{tag}> has no command `{name}`"))
            .values()
            .map(|field| field["the"].as_str().expect("an attribute").to_owned())
            .collect()
    }

    /// Every command the bar dispatches carries each attribute its handler
    /// decodes from.
    ///
    /// The worker reads these commands raw, by attribute. If the element's
    /// claim and the handler drift apart the command decodes as nothing,
    /// the handler never runs, and the bar still looks as if it worked:
    /// the failure this design has to rule out.
    fn assert_carries<C: Decode>(tag: &str, name: &str) {
        let triggers = C::trigger_attributes();
        assert!(
            !triggers.is_empty(),
            "the command must declare trigger attributes"
        );
        let declared = declared(tag, name);
        for attribute in &triggers {
            assert!(
                declared.contains(attribute),
                "<{tag}>'s `{name}` claim must carry {attribute}, which its handler decodes from",
            );
        }
    }

    #[dialog_common::test]
    fn it_builds_invite_claims_carrying_every_attribute_the_handler_triggers_on() {
        assert_carries::<Invite>("tonk-share", "invite");
    }

    #[dialog_common::test]
    fn it_builds_forget_invite_claims_carrying_every_attribute_the_handler_triggers_on() {
        assert_carries::<ForgetInvite>("tonk-share", "forget-invite");
    }

    #[dialog_common::test]
    fn it_builds_enable_sync_claims_carrying_every_attribute_the_handler_triggers_on() {
        assert_carries::<EnableSync>("tonk-share", "enable-sync");
    }

    #[dialog_common::test]
    fn it_builds_pause_claims_carrying_every_attribute_the_handler_triggers_on() {
        assert_carries::<PauseSync>("tonk-fab", "pause-sync");
    }

    #[dialog_common::test]
    fn it_builds_publication_claims_carrying_every_attribute_the_handlers_trigger_on() {
        assert_carries::<PublishSpace>("tonk-fab", "publish");
        assert_carries::<UnpublishSpace>("tonk-fab", "unpublish");
    }

    #[dialog_common::test]
    fn it_builds_agent_handoff_claims_carrying_every_attribute_the_handler_triggers_on() {
        assert_carries::<AgentHandoff>("tonk-agent-panel", "handoff");
    }

    #[dialog_common::test]
    fn it_reads_the_repo_name_attribute_the_schema_writes() {
        // The bar reads a raw attribute; the worker writes facts under the
        // schema's domain type. If the two diverge the name silently blanks.
        let body = query("tonk-fab", "space-name", Some("did:key:z6Mk"));
        assert!(body.contains(&tonk_schema::domain::repo::Name::the().to_string()));
        assert!(body.contains("did:key:z6Mk"));
    }

    #[dialog_common::test]
    fn it_reads_every_method_it_is_asked_for() {
        assert!(element_method("tonk-fab", "connected").starts_with("(self) =>"));
        assert!(element_method("tonk-share", "press").contains("'invite'"));
    }
}
