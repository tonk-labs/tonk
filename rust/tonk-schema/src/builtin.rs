//! Built-in concept registry.
//!
//! Some concepts are not user-defined facts on the branch — they
//! describe the meta-schema itself ([`attribute`], [`concept`]) or
//! repository state written by the worker as native Rust types
//! ([`branch`], [`replica`], [`remote`], [`tracking-branch`]). The
//! resolution surface can't fetch them because nothing on the
//! branch tags them as concepts; instead the analyzer consults
//! this registry first and falls through to the live environment
//! only when no built-in matches.
//!
//! Built-ins win over branch-defined concepts of the same name.
//! In-document `concept!` definitions still win over built-ins
//! within their own document, so users can shadow a built-in
//! locally for testing.
//!
//! # Result type
//!
//! The registry stores [`crate::resolution::ConceptDefinition`] —
//! entity plus a durability-tagged descriptor. Every built-in is
//! durable.

use std::sync::OnceLock;

use dialog_artifacts::Entity;
use dialog_query::ConceptDescriptor as DialogConceptDescriptor;

use crate::concept::{command_of_command_descriptor, concept_of_concept_descriptor};
use crate::resolution::ConceptDefinition;
use crate::rule_query::rule_of_rule_descriptor;
use crate::{Branch, Remote, Replica, TrackingBranch};
use tonk_core::claim::ConceptDescriptor;
use tonk_core::meta::{AnonymousAttribute, Name};

/// Look up a built-in concept by head-name as a durability-tagged
/// [`ConceptDefinition`]. Returns `None` for names that fall
/// through to the live environment. Built-ins are always durable.
pub fn lookup_concept(name: &str) -> Option<ConceptDefinition> {
    REGISTRY
        .get_or_init(build_registry)
        .iter()
        .find(|(key, _)| *key == name)
        .map(|(_, concept)| concept.clone())
}

/// Iterate every built-in concept as `(name, ConceptDefinition)`
/// pairs. Used by the concept-of-concept query path to surface
/// built-ins in a `concept:` query result.
pub fn concept_registry() -> &'static [(&'static str, ConceptDefinition)] {
    REGISTRY.get_or_init(build_registry).as_slice()
}

static REGISTRY: OnceLock<Vec<(&'static str, ConceptDefinition)>> = OnceLock::new();

fn build_registry() -> Vec<(&'static str, ConceptDefinition)> {
    vec![
        ("attribute", builtin::<AnonymousAttribute>("attribute")),
        ("concept", concept_descriptor()),
        ("command", command_descriptor()),
        ("event", event_descriptor()),
        ("rule", rule_descriptor()),
        ("name", builtin::<Name>("name")),
        ("branch", builtin::<Branch>("branch")),
        ("replica", builtin::<Replica>("replica")),
        ("remote", builtin::<Remote>("remote")),
        (
            "tracking-branch",
            builtin::<TrackingBranch>("tracking-branch"),
        ),
    ]
}

/// Built-in `concept` view — the concept-of-concept descriptor.
///
/// Resolves to the sentinel descriptor whose `this()` triggers
/// dispatch to [`crate::concept::AnonymousConceptQuery`] in
/// [`crate::concept::QueryPlan::from`], so a `concept:` head at
/// query time enumerates *every* concept (built-in + branch) with
/// a synthesised `source` field.
///
/// Kept as a hand-built descriptor (rather than `derive(Concept)`)
/// because the concept-of-concept's `with:` is a dictionary — an
/// arbitrary map of names to attribute references — not a fixed
/// record of named fields. Rust struct derives can't express that
/// shape, so this one stays JSON.
fn concept_descriptor() -> ConceptDefinition {
    ConceptDefinition {
        entity: "db:concept"
            .parse()
            .expect("`db:concept` is a valid entity URI"),
        descriptor: ConceptDescriptor::Durable(concept_of_concept_descriptor().clone()),
    }
}

/// Built-in `command` view — the command-of-command descriptor.
///
/// Resolves to the sentinel descriptor whose `this()` triggers
/// dispatch to [`crate::concept::AnonymousConceptQuery::commands`]
/// in [`crate::concept::QueryPlan::from`], so a `command:` head at
/// query time enumerates every *transient* concept (the commands)
/// on the branch — the transient-only sibling of
/// [`concept_descriptor`].
///
/// Kept as a hand-built descriptor for the same reason
/// [`concept_descriptor`] is: its `with:` synthesises fields with
/// no fixed-record Rust shape the derive can express.
fn command_descriptor() -> ConceptDefinition {
    ConceptDefinition {
        entity: "db:command"
            .parse()
            .expect("`db:command` is a valid entity URI"),
        descriptor: ConceptDescriptor::Durable(command_of_command_descriptor().clone()),
    }
}

/// Built-in `event` concept — how a platform event fills a command's
/// fields.
///
/// A peer of `command`: an `event!:` declaration is schema an author
/// writes, so it must resolve on any branch rather than only where the
/// standard library has been seeded. Its instances (`on/click`, and a
/// terminal's own set) are ordinary data and stay in the library.
///
/// Hand-built rather than `derive(Concept)` for the same reason
/// [`command_descriptor`] and [`rule_descriptor`] are: `where` is a
/// keyed dictionary, which no Rust fixed-record shape expresses — the
/// same reason the `view` concept is still declared in the library.
fn event_descriptor() -> ConceptDefinition {
    static DESCRIPTOR: std::sync::OnceLock<DialogConceptDescriptor> = std::sync::OnceLock::new();
    let descriptor = DESCRIPTOR.get_or_init(|| {
        serde_json::from_value(serde_json::json!({
            "description": "How a platform event fills a command's fields.",
            "with": {
                "type": {
                    "the": "xyz.tonk.event/type",
                    "as": "Text",
                    "cardinality": "one",
                    "description": "The platform event name to listen for"
                },
                // A keyed collection: the `the` names a domain and the
                // key supplies the name half, so each source lands as
                // its own fact and a space can supersede one without
                // restating the map. Its own domain, so a command field
                // called `type` cannot collide with `type` above.
                "where": {
                    "the": { "domain": "xyz.tonk.event.where", "keyed": "dictionary" },
                    "as": "Text",
                    "cardinality": "one",
                    "description": "Command field name to source"
                },
                // Optional: a declaration that suppresses neither must
                // still resolve, so these cannot be required.
                "prevent-default": {
                    "the": "xyz.tonk.event/prevent-default",
                    "as": "Boolean",
                    "cardinality": "one",
                    "optional": true,
                    "description": "Suppress the platform's default handling"
                },
                "stop-propagation": {
                    "the": "xyz.tonk.event/stop-propagation",
                    "as": "Boolean",
                    "cardinality": "one",
                    "optional": true,
                    "description": "Stop the event propagating further"
                }
            }
        }))
        .expect("event descriptor is well-formed")
    });
    ConceptDefinition {
        entity: "db:event"
            .parse()
            .expect("`db:event` is a valid entity URI"),
        descriptor: ConceptDescriptor::Durable(descriptor.clone()),
    }
}

/// Built-in `rule` view — the rule-of-rule descriptor.
///
/// Resolves to the sentinel descriptor whose `this()` triggers
/// dispatch to [`crate::rule_query::AnonymousRuleQuery`] in
/// [`crate::concept::QueryPlan::from`], so a `rule:` head at
/// query time enumerates *every* installed inductive rule with a
/// synthesised `definition` field. The rule-side parallel of
/// [`concept_descriptor`].
///
/// Kept as a hand-built descriptor (rather than `derive(Concept)`)
/// for the same reason `concept_descriptor` is: its synthesised
/// fields have no fixed-record Rust shape the derive can express.
fn rule_descriptor() -> ConceptDefinition {
    ConceptDefinition {
        entity: "db:rule".parse().expect("`db:rule` is a valid entity URI"),
        descriptor: ConceptDescriptor::Durable(rule_of_rule_descriptor().clone()),
    }
}

/// Build a built-in [`ConceptDefinition`] from a
/// `#[derive(Concept)]` Rust struct's static descriptor.
///
/// The descriptor is read through the derive-generated
/// [`Descriptor<ConceptDescriptor>`](dialog_query::Descriptor) impl,
/// so this is a pure schema-only path. The descriptor's `this()`
/// would be a content-derived hash; built-ins instead live at the
/// stable `db:<name>` URI so the `db:` scheme protection covers them
/// and the row remains identifiable without knowing the hash.
fn builtin<S>(name: &str) -> ConceptDefinition
where
    S: dialog_query::Descriptor<DialogConceptDescriptor>,
{
    let entity: Entity = format!("db:{name}")
        .parse()
        .expect("`db:<builtin>` is a valid entity URI");
    ConceptDefinition {
        entity,
        descriptor: ConceptDescriptor::Durable(S::descriptor().clone()),
    }
}
