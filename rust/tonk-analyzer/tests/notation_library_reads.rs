//! A notation library is lowered to claims with no branch
//! (`notation_library_claims`): nothing it reads is there. A write whose
//! premises read stored facts, or a retract by entity alone, which reads
//! the entity's claims to know what to retract, would write nothing. Such
//! a form is refused, naming itself, rather than installed as nothing. A
//! write whose premises read no stored fact, as the notebook's routes read
//! only `id`, still writes.

#![cfg(not(target_arch = "wasm32"))]

use tonk_analyzer::notation::{NotationLibrary, notation_library_claims};

const CORE: &str = include_str!("../../tonk-core/assets/library/core.clj");

const THING: &str = "(concept (thing ?t name: ?n)
  (probe.thing/name of: ?t is: ?n as: :text cardinality: :one))
";

async fn lowered(library: &str) -> NotationLibrary {
    let (operator, _) = dialog_peer::helpers::test_session_with_peer().await;
    notation_library_claims(CORE, &format!("{THING}{library}"), &operator)
        .await
        .expect("the library lowers")
}

fn refusal(library: &NotationLibrary) -> Option<&str> {
    library
        .forms
        .iter()
        .find_map(|form| form.refused.as_deref())
}

#[dialog_common::test]
async fn a_write_whose_premises_read_stored_facts_is_refused() {
    let library = lowered(
        "(assert (thing thing:a name: \"A\"))
(assert (thing ?t name: \"B\") (thing ?t name: \"A\"))
",
    )
    .await;
    let refused = refusal(&library).expect("the write is refused");
    assert!(
        refused.contains("(thing ?t name: \"A\")"),
        "the refusal names the form: {refused}"
    );
}

#[dialog_common::test]
async fn a_retract_by_entity_alone_is_refused() {
    let library = lowered(
        "(assert (thing thing:a name: \"A\"))
(retract (thing thing:a))
",
    )
    .await;
    let refused = refusal(&library).expect("the retract is refused");
    assert!(
        refused.contains("(retract (thing thing:a))"),
        "the refusal names the form: {refused}"
    );
}

#[dialog_common::test]
async fn a_write_whose_premises_read_no_stored_fact_still_writes() {
    let library = lowered(
        "(assert (thing ?t name: \"B\") (id ?t name: \"B\"))
",
    )
    .await;
    assert_eq!(refusal(&library), None);
    assert!(
        library
            .forms
            .iter()
            .any(|form| form.kind != "concept"
                && !form.durable.clone().into_instructions().is_empty()),
        "the write wrote"
    );
}
