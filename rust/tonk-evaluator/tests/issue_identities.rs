//! `issue.yaml` and `issue.clj` side by side: what each stores for the
//! issue library, and whether a space whose issues were written under
//! the YAML still answers through the notation, and the other way.
//!
//! The YAML is lowered as the worker installs a component
//! (`library_claims_after` in `tonk-worker/src/router/repository.rs`):
//! `core.yaml` and the component analyzed as one document, the
//! component's expressions planned on their own. The notation is lowered
//! by `tonk_analyzer::notation::notation_library_claims`, which the
//! worker installs a `.clj` component with (`notation_claims`). Each side
//! ends in the claims a branch transaction is handed.
//!
//! The test pins every identity the conversion keeps and every one it
//! moves, so a migration knows what it has to rewrite:
//!
//! - kept: the concept's entity, its five attributes' entities, and the
//!   name `issue` the concept is bound to; every record written under
//!   either reads back under the other;
//! - moved in text only: the concept's and three attributes'
//!   descriptions, which the notation keeps from comments, so it drops a
//!   blank line and a trailing newline the YAML's `|` blocks hold;
//! - lost: `io.gozala.issue/blocks`, an attribute no concept reads, which
//!   the notation cannot declare on its own, and the names
//!   `issue/title` … `issue/blocks` the YAML's `attribute!:` anchors bind
//!   to the six attributes (`db.name/referent`), which the notation has
//!   no spelling for.
//!
//! The library declares no rule, view or command, so it says nothing of
//! their identities.

#![allow(deprecated)]
#![cfg(not(target_arch = "wasm32"))]

mod support;

use std::collections::BTreeSet;

use dialog_capability::Subject;
use dialog_corpus::Value;
use dialog_effects::storage::Location;
use dialog_peer::helpers::{open_peer, test_storage, unique_name};
use dialog_query::query::Output as _;
use dialog_query::{ConceptQuery, Parameters, Term};
use dialog_repository::{Branch, RepositoryExt as _};
use support::{Env, attribute, notation_claims, plain, referent, yaml_claims};
use tonk_analyzer::notation::tour::{Cell, evaluate_on_branch};
use tonk_evaluator::evaluate::SyntaxEvaluateExt as _;
use tonk_notation::parse;
use tonk_schema::concept::{Concept, QueryPlan};
use tonk_schema::query_source::Source;

const ISSUE_YAML: &str = include_str!("../../tonk-core/assets/library/issue.yaml");
const ISSUE_CLJ: &str = include_str!("../../tonk-core/assets/library/issue.clj");

/// The five fields of `issue`, as the YAML names their attributes.
const FIELDS: [&str; 5] = ["title", "description", "type", "priority", "status"];

#[dialog_common::test]
async fn the_issue_library_keeps_its_concept_and_attribute_identities() {
    let yaml = yaml_claims("issue.yaml", ISSUE_YAML).await;
    let notation = notation_claims(ISSUE_CLJ).await;

    // The concept: one entity, bound to the same name on both sides.
    let concept = referent(&yaml, "issue").expect("the YAML names the concept");
    assert_eq!(referent(&notation, "issue").as_ref(), Some(&concept));
    assert_eq!(plain(&yaml[&concept]), plain(&notation[&concept]));

    // Its attributes: one entity each.
    let mut kept = BTreeSet::from([concept.clone(), "id:issue".to_owned()]);
    for field in FIELDS {
        let relation = format!("io.gozala.issue/{field}");
        let entity = attribute(&yaml, &relation).expect("the YAML declares it");
        assert_eq!(
            attribute(&notation, &relation).as_ref(),
            Some(&entity),
            "{relation}"
        );
        assert_eq!(
            plain(&yaml[&entity]),
            plain(&notation[&entity]),
            "{relation}"
        );
        kept.insert(entity);
    }

    // What differs only in the descriptions' blank lines and closing
    // newline: the concept and three attributes.
    let reworded: BTreeSet<String> = kept
        .iter()
        .filter(|entity| yaml[*entity] != notation[*entity])
        .cloned()
        .collect();
    let described = |relation: &str| attribute(&yaml, relation).expect("declared");
    assert_eq!(
        reworded,
        BTreeSet::from([
            concept.clone(),
            described("io.gozala.issue/type"),
            described("io.gozala.issue/priority"),
            described("io.gozala.issue/status"),
        ]),
    );

    // What the YAML stores and the notation does not: `blocks`, and the
    // six names the YAML's `attribute!:` anchors bind.
    let lost: BTreeSet<String> = yaml
        .keys()
        .filter(|entity| !notation.contains_key(*entity))
        .cloned()
        .collect();
    let blocks = described("io.gozala.issue/blocks");
    let mut expected: BTreeSet<String> = FIELDS
        .iter()
        .chain(&["blocks"])
        .map(|field| format!("id:issue/{field}"))
        .collect();
    expected.insert(blocks.clone());
    assert_eq!(lost, expected);
    assert_eq!(attribute(&notation, "io.gozala.issue/blocks"), None);
    for field in FIELDS.iter().chain(&["blocks"]) {
        assert_eq!(
            referent(&notation, &format!("issue/{field}")),
            None,
            "the notation binds no name to an attribute"
        );
    }

    // And nothing the notation stores that the YAML does not.
    let added: Vec<&String> = notation
        .keys()
        .filter(|entity| !yaml.contains_key(*entity))
        .collect();
    assert!(added.is_empty(), "only in the notation: {added:?}");
}

/// Evaluate tonk's YAML `text` on `branch` and commit it.
async fn commit_yaml<E: Env>(branch: &Branch, operator: &E, text: &str) {
    let parsed = parse(text);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    parsed
        .syntax
        .expect("syntax")
        .evaluate(branch.transaction())
        .perform(operator)
        .await
        .expect("evaluates")
        .commit()
        .publish()
        .perform(operator)
        .await
        .expect("commits");
}

/// Run a notation cell on `branch`, after `issue.clj`.
async fn cell<E: Env>(
    branch: &Branch,
    operator: &E,
    source: &str,
) -> tonk_analyzer::notation::tour::Answer {
    let answer = evaluate_on_branch(
        branch,
        &Cell {
            source: source.to_owned(),
            context: vec![ISSUE_CLJ.to_owned()],
        },
        operator,
    )
    .await;
    assert!(answer.error.is_none(), "{:?}", answer.error);
    for form in &answer.forms {
        assert!(form.refused.is_none(), "{:?}", form.refused);
    }
    answer
}

const ISSUE_1: &str = "issue!:\n  this: id:issue-1\n  title: \"It crashes\"\n  description: \"On start\"\n  type: id:bug\n  priority: id:high\n  status: id:open\n";

#[dialog_common::test]
async fn issues_written_under_the_yaml_answer_through_the_notation() {
    let peer = open_peer(
        test_storage().await,
        Location::profile(unique_name("issue")),
    )
    .await
    .expect("a peer");
    let operator = peer
        .session(b"issue")
        .space(peer.state())
        .allow(Subject::any())
        .await
        .expect("a session");
    let branch: Branch = peer
        .space(unique_name("repo"))
        .open()
        .perform(&operator)
        .await
        .expect("a repository")
        .branch("main")
        .open()
        .perform(&operator)
        .await
        .expect("a branch");
    commit_yaml(&branch, &operator, ISSUE_YAML).await;
    commit_yaml(&branch, &operator, ISSUE_1).await;

    let answer = cell(
        &branch,
        &operator,
        "(select (issue ?i title: ?title description: ?description type: ?type \
         priority: ?priority status: ?status))",
    )
    .await;
    let printed = &answer.forms[0].printed;
    assert_eq!(printed.len(), 1, "{printed:?}");
    for value in [
        "id:issue-1",
        "title: \"It crashes\"",
        "description: \"On start\"",
        "type: id:bug",
        "priority: id:high",
        "status: id:open",
    ] {
        assert!(printed[0].contains(value), "{value} in {}", printed[0]);
    }
}

#[dialog_common::test]
async fn an_issue_written_through_the_notation_reads_back_through_the_yaml_concept() {
    let peer = open_peer(
        test_storage().await,
        Location::profile(unique_name("issue")),
    )
    .await
    .expect("a peer");
    let operator = peer
        .session(b"issue")
        .space(peer.state())
        .allow(Subject::any())
        .await
        .expect("a session");
    let branch: Branch = peer
        .space(unique_name("repo"))
        .open()
        .perform(&operator)
        .await
        .expect("a repository")
        .branch("main")
        .open()
        .perform(&operator)
        .await
        .expect("a branch");
    commit_yaml(&branch, &operator, ISSUE_YAML).await;
    cell(
        &branch,
        &operator,
        "(assert (issue id:issue-2 title: \"Write the docs\" description: \"For the tour\" \
         type: id:task priority: id:low status: id:open))",
    )
    .await;

    // The concept as the YAML stored it, read as tonk reads a concept.
    let concept = referent(&yaml_claims("issue.yaml", ISSUE_YAML).await, "issue")
        .expect("the YAML names the concept");
    let stored = Concept::by_entity(concept.parse().expect("an entity"))
        .resolve(&Source::from(&branch), &operator)
        .await
        .expect("the concept resolves")
        .expect("the concept is stored");
    let mut terms = Parameters::new();
    terms.insert(
        "this".to_owned(),
        Term::Constant(Value::Entity("id:issue-2".parse().expect("an entity"))),
    );
    for field in FIELDS {
        terms.insert(field.to_owned(), Term::var(field));
    }
    let rows = branch
        .select(QueryPlan::from(ConceptQuery {
            predicate: stored.descriptor,
            terms,
        }))
        .perform(&operator)
        .try_vec()
        .await
        .expect("the concept reads");
    assert_eq!(rows.len(), 1);
    let row = rows[0].source();
    assert_eq!(
        row.value_of("title"),
        Some(&Value::String("Write the docs".into()))
    );
    assert_eq!(
        row.value_of("type"),
        Some(&Value::Entity("id:task".parse().expect("an entity")))
    );
}
