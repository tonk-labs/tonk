//! The profile library's ranked cases: `account/status`, `space/presence`
//! and `space/sync` each list their cases best first, rules derive one
//! case each, and a read elects the best case any rule derives. The
//! precedence must not depend on the order the facts were written in.
//!
//! Each concept is read as the library stores it on the branch, so this
//! covers the lowering of a ranked attribute (a cardinality-many
//! attribute read by its field's `top`) and its round trip through the
//! stored definition.

// The test commits with the one-step publish, which fails on a lost
// race: the branch is its own, with no other writer.
#![allow(deprecated)]
#![cfg(not(target_arch = "wasm32"))]

use dialog_capability::Subject;
use dialog_effects::storage::Location;
use dialog_peer::helpers::{open_peer, test_storage, unique_name};
use dialog_query::query::Output as _;
use dialog_query::{ConceptQuery, Parameters, Term, Value};
use dialog_repository::Branch;
use dialog_repository::RepositoryExt as _;
use tonk_evaluator::evaluate::SyntaxEvaluateExt as _;
use tonk_notation::{expand, parse, parse_at};
use tonk_schema::concept::{Concept, QueryPlan};
use tonk_schema::query_source::Source;

/// What the test asks of an environment: everything the evaluator and a
/// commit do.
trait Env:
    tonk_evaluator::evaluate::EvaluateEnv
    + dialog_capability::Provider<dialog_effects::blob::Size>
    + dialog_capability::Provider<dialog_effects::blob::Import>
    + dialog_capability::Provider<dialog_effects::archive::Import>
    + dialog_capability::Provider<dialog_effects::authority::Attest>
{
}
impl<T> Env for T where
    T: tonk_evaluator::evaluate::EvaluateEnv
        + dialog_capability::Provider<dialog_effects::blob::Size>
        + dialog_capability::Provider<dialog_effects::blob::Import>
        + dialog_capability::Provider<dialog_effects::archive::Import>
        + dialog_capability::Provider<dialog_effects::authority::Attest>
{
}

const PROFILE_LIBRARY: &str = include_str!("../../tonk-core/assets/library/profile.yaml");

async fn commit<E: Env>(branch: &Branch, operator: &E, text: &str) {
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

async fn seed_profile<E: Env>(branch: &Branch, operator: &E) {
    let parsed = parse_at(tonk_library::location("profile.yaml"), PROFILE_LIBRARY);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let mut syntax = parsed.syntax.expect("syntax");
    let unexpanded = expand(&mut syntax, &tonk_library::Bundled).await;
    assert!(unexpanded.is_empty(), "{unexpanded:?}");
    syntax
        .evaluate(branch.transaction())
        .perform(operator)
        .await
        .expect("the profile library evaluates")
        .commit()
        .publish()
        .perform(operator)
        .await
        .expect("the profile library commits");
}

/// The case the stored concept `concept` elects for `this`, in `field`.
async fn case_of<E: Env>(
    branch: &Branch,
    operator: &E,
    concept: &str,
    field: &str,
    this: &str,
) -> Vec<String> {
    let stored = Concept::by_entity(concept.parse().expect("an entity"))
        .resolve(&Source::from(branch), operator)
        .await
        .expect("the concept resolves")
        .expect("the concept is stored");
    let mut terms = Parameters::new();
    terms.insert(
        "this".to_string(),
        Term::Constant(Value::Entity(this.parse().expect("an entity"))),
    );
    terms.insert(field.to_string(), Term::var(field));
    let rows = branch
        .select(QueryPlan::from(ConceptQuery {
            predicate: stored.descriptor,
            terms,
        }))
        .perform(operator)
        .try_vec()
        .await
        .expect("the concept reads");
    rows.iter()
        .filter_map(|row| match row.source().value_of(field) {
            Some(Value::Entity(case)) => Some(case.to_string()),
            _ => None,
        })
        .collect()
}

/// Each write order of `facts`, one commit per fact, on a branch of its
/// own with the profile library seeded: the elected case is checked
/// after every order.
async fn in_every_order(
    facts: &[&str],
    concept: &str,
    field: &str,
    this: &str,
    expected: &str,
    prelude: &str,
) {
    let orders: Vec<Vec<&str>> = {
        let forward = facts.to_vec();
        let mut backward = facts.to_vec();
        backward.reverse();
        let mut rotated = facts.to_vec();
        rotated.rotate_left(1);
        vec![forward, backward, rotated]
    };
    for order in orders {
        let peer = open_peer(test_storage().await, Location::profile(unique_name("ranked")))
            .await
            .expect("a peer");
        let operator = peer
            .session(b"ranked")
            .space(peer.state())
            .allow(Subject::any())
            .await
            .expect("a session");
        let repo = peer
            .space(unique_name("repo"))
            .open()
            .perform(&operator)
            .await
            .expect("a repository");
        let branch = repo
            .branch("main")
            .open()
            .perform(&operator)
            .await
            .expect("a branch");
        seed_profile(&branch, &operator).await;
        let prelude = prelude.replace("{device}", &device(&branch, &operator).await);
        if !prelude.is_empty() {
            commit(&branch, &operator, &prelude).await;
        }
        for fact in &order {
            let fact = fact.replace("{device}", &device(&branch, &operator).await);
            commit(&branch, &operator, &fact).await;
        }
        assert_eq!(
            case_of(&branch, &operator, concept, field, this).await,
            vec![expected.to_string()],
            "{concept} of {this}, written in the order {order:?}",
        );
    }
}

/// The device profile the session reports: what a replica on this
/// device is keyed by.
async fn device<E: Env>(branch: &Branch, operator: &E) -> String {
    let descriptor = serde_json::from_value(serde_json::json!({
        "with": { "profile": { "the": "dialog.session/profile", "as": "entity:" } }
    }))
    .expect("descriptor");
    let mut terms = Parameters::new();
    terms.insert("this".to_string(), Term::var("this"));
    terms.insert("profile".to_string(), Term::var("profile"));
    let rows = branch
        .select(QueryPlan::from(ConceptQuery {
            predicate: descriptor,
            terms,
        }))
        .perform(operator)
        .try_vec()
        .await
        .expect("the session reads");
    match rows.first().and_then(|row| row.source().value_of("profile")) {
        Some(Value::Entity(profile)) => profile.to_string(),
        other => panic!("no session profile: {other:?}"),
    }
}

const REGISTERED: &str = "account/registered!:\n  this: id:account\n  registered-at: 1\n  email: \"account@example.com\"\n  provider: \"https://access.example/\"\n";
const ACTIVE: &str = "account/active!:\n  this: id:account\n  activated-at: 2\n";
const SUSPENDED: &str =
    "account/suspended!:\n  this: id:account\n  suspended-at: 3\n  reason: \"test\"\n";
const ONBOARDING: &str = "account/onboarding!:\n  this: id:account\n  minted-at: 0\n";

/// Suspended outranks active, which outranks registered, whatever order
/// the facts arrive in.
#[dialog_common::test]
async fn an_accounts_status_is_the_best_case_any_rule_derives() {
    let status = |facts: &'static [&'static str], expected: &'static str| async move {
        in_every_order(
            facts,
            "tonk:account/status",
            "status",
            "id:account",
            expected,
            "",
        )
        .await
    };
    status(&[REGISTERED, ACTIVE, SUSPENDED], "case:suspended").await;
    status(&[REGISTERED, ACTIVE], "case:active").await;
    status(&[ONBOARDING, REGISTERED], "case:registered").await;
}

const SPACE: &str = "space!:\n  this: id:space\n  subject: id:subject\n";
const REPLICATING: &str = "space/replicating!:\n  this: id:replicating\n  subject: id:subject\n  replicating: true\n";
const REPLICA: &str =
    "space/replica!:\n  this: id:replica\n  subject: id:subject\n  profile: {device}\n";
const SEEDING: &str = "space/replica-status!:\n  this: id:replica\n  status: tonk:blank\n";

/// A replica still seeding reads as seeding although it is also a
/// replica, and a pull in flight reads as replicating although a replica
/// is held: the library lists seeding, replicating, replicated, remote.
#[dialog_common::test]
async fn a_spaces_presence_is_the_best_case_any_rule_derives() {
    let presence = |facts: &'static [&'static str], expected: &'static str| async move {
        in_every_order(
            facts,
            "tonk:space/presence",
            "presence",
            "id:space",
            expected,
            SPACE,
        )
        .await
    };
    presence(&[REPLICA, SEEDING], "case:seeding").await;
    presence(&[REPLICATING, REPLICA], "case:replicating").await;
    presence(&[REPLICA], "case:replicated").await;
    presence(&[REPLICATING], "case:replicating").await;
}

const SYNCED: &str = "space/synced!:\n  this: id:space\n  home-address: \"https://sync.example/\"\n";
const PAUSED: &str = "replica/sync-enabled!:\n  this: id:replica\n  enabled: false\n";

/// A synced space paused on this device reads as paused; synced
/// outranks local.
#[dialog_common::test]
async fn a_spaces_sync_is_the_best_case_any_rule_derives() {
    let sync = |facts: &'static [&'static str], expected: &'static str| async move {
        in_every_order(facts, "tonk:space/sync", "sync", "id:space", expected, SPACE).await
    };
    sync(&[SYNCED, REPLICA, PAUSED], "case:paused").await;
    sync(&[SYNCED], "case:synced").await;
    sync(&[REPLICA], "case:local").await;
}
