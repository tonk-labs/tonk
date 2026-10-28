use crate::BranchState;
use dialog_artifacts::{Entity, Value};
use dialog_peer::helpers::{test_repo, test_session_with_peer};
use dialog_query::{ConceptQuery, Parameters, Term, the};
use std::sync::Arc;
use tonk_evaluator::evaluate::SyntaxEvaluateExt;
use tonk_schema::resolution::{ConceptReference, NamedReference};

#[dialog_common::test]
async fn graph_subscribers_receive_snapshot_then_nested_delta_and_reconnect_snapshot()
-> anyhow::Result<()> {
    let (operator, profile) = test_session_with_peer().await;
    let repo = test_repo(&operator, &profile).await;
    let branch = repo.branch("main").open().perform(&operator).await?;
    let source = r#"
concept!: &person
  with:
    name: { the: example.person/name, as: text, description: "Name" }
  maybe:
    manager: { the: example.person/manager, as: person, description: "Manager" }
person!:
  this: id:alice
  name: "Alice"
  manager: id:bob
person!:
  this: id:bob
  name: "Bob"
  manager: id:alice
"#;
    tonk_notation::parse(source)
        .syntax
        .unwrap()
        .evaluate(branch.transaction())
        .perform(&operator)
        .await?
        .commit()
        .publish()
        .perform(&operator)
        .await?;
    let concept = ConceptReference::from(NamedReference("person".into()))
        .resolve(&branch)
        .perform(&operator)
        .await?
        .unwrap();
    let mut terms = Parameters::new();
    terms.insert(
        "this".into(),
        Term::Constant(Value::Entity("id:alice".parse()?)),
    );
    let root = ConceptQuery {
        predicate: concept.descriptor.concept().clone(),
        terms,
    };
    let graph = tonk_schema::graph::resolve_graph(root, "person", &(&branch).into(), &operator)
        .await?
        .unwrap();
    let state = Arc::new(BranchState::new(branch));
    let mut subscriber = state.subscribe_graph(graph.clone(), Some("test".into()), 0)?;
    state.poll(&operator).await;
    let snapshot: serde_json::Value = serde_json::from_slice(&subscriber.receiver.try_recv()?)?;
    assert_eq!(snapshot["kind"], "graph_snapshot");
    assert_eq!(snapshot["graph"]["nodes"].as_object().unwrap().len(), 2);
    let bob: Entity = "id:bob".parse()?;
    state
        .branch
        .transaction()
        .assert(the!("example.person/name").of(bob).is("Robert".to_owned()))
        .commit()
        .publish()
        .perform(&operator)
        .await?;
    state.poll(&operator).await;
    let delta: serde_json::Value = serde_json::from_slice(&subscriber.receiver.try_recv()?)?;
    assert_eq!(delta["kind"], "graph_delta");
    assert!(
        delta["delta"]["fields"]
            .as_array()
            .unwrap()
            .iter()
            .any(|field| field["field"] == "name")
    );
    let mut second = state.subscribe_graph(graph, Some("reconnect".into()), 0)?;
    state.poll(&operator).await;
    let snapshot: serde_json::Value = serde_json::from_slice(&second.receiver.try_recv()?)?;
    assert_eq!(snapshot["kind"], "graph_snapshot");
    assert!(
        subscriber.receiver.try_recv().is_err(),
        "unchanged poll sends no duplicate delta"
    );
    state.retain_subscribers(|_| false);
    assert!(
        second.receiver.is_closed(),
        "client cleanup closes graph stream"
    );
    Ok(())
}

#[dialog_common::test]
async fn constrained_graph_stream_tracks_root_membership_edge_moves_and_cycle_cuts()
-> anyhow::Result<()> {
    let (operator, profile) = test_session_with_peer().await;
    let repo = test_repo(&operator, &profile).await;
    let branch = repo.branch("main").open().perform(&operator).await?;
    let source = r#"concept!: &person
  with:
    name: { the: example.person/name, as: text, description: "Name" }
  maybe:
    manager: { the: example.person/manager, as: person, description: "Manager" }
concept!: &issue
  with:
    title: { the: example.issue/title, as: text, description: "Title" }
    owner: { the: example.issue/owner, as: person, description: "Owner" }
person!:
  this: id:alice
  name: "Alice"
  manager: id:bob
person!:
  this: id:bob
  name: "Bob"
issue!:
  this: id:issue
  title: "Issue"
  owner: id:alice
"#;
    tonk_notation::parse(source)
        .syntax
        .unwrap()
        .evaluate(branch.transaction())
        .perform(&operator)
        .await?
        .commit()
        .publish()
        .perform(&operator)
        .await?;
    let concept = ConceptReference::from(NamedReference("issue".into()))
        .resolve(&branch)
        .perform(&operator)
        .await?
        .unwrap();
    let root = ConceptQuery {
        predicate: concept.descriptor.concept().clone(),
        terms: {
            let mut terms = Parameters::new();
            terms.insert("this".into(), Term::var("this"));
            terms
        },
    };
    let mut graph = tonk_schema::graph::resolve_graph(root, "issue", &(&branch).into(), &operator)
        .await?
        .unwrap();
    let constraints = tonk_schema::graph::GraphConstraints {
        matching: [(
            "owner".into(),
            [("name".into(), Value::String("Alice".into()))].into(),
        )]
        .into(),
        ..Default::default()
    };
    constraints.apply(&mut graph)?;
    let state = Arc::new(BranchState::new(branch));
    let mut stream = state.subscribe_graph(graph, None, 0)?;
    state.poll(&operator).await;
    let initial: serde_json::Value = serde_json::from_slice(&stream.receiver.try_recv()?)?;
    assert_eq!(initial["graph"]["roots"], serde_json::json!(["id:issue"]));
    for (mutation, field, root_added, root_removed) in [
        (
            "person!:\n  this: id:bob\n  manager: id:alice\n",
            "manager",
            false,
            false,
        ),
        (
            "person!:\n  this: id:bob\n  manager: _\n",
            "manager",
            false,
            false,
        ),
        (
            "issue!:\n  this: id:issue\n  owner: id:bob\n",
            "",
            false,
            true,
        ),
        (
            "person!:\n  this: id:bob\n  name: \"Alice\"\n",
            "",
            true,
            false,
        ),
        ("issue!:\n  this: id:issue\n  title: _\n", "", false, true),
    ] {
        tonk_notation::parse(mutation)
            .syntax
            .unwrap()
            .evaluate(state.branch.transaction())
            .perform(&operator)
            .await?
            .commit()
            .publish()
            .perform(&operator)
            .await?;
        state.poll(&operator).await;
        let wire: serde_json::Value = serde_json::from_slice(&stream.receiver.try_recv()?)?;
        assert_eq!(wire["kind"], "graph_delta");
        let delta = &wire["delta"];
        if !field.is_empty() {
            assert!(
                delta["fields"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|change| change["field"] == field)
            );
        }
        if root_added {
            assert_eq!(delta["roots_added"], serde_json::json!(["id:issue"]));
        }
        if root_removed {
            assert_eq!(delta["roots_removed"], serde_json::json!(["id:issue"]));
        }
    }
    Ok(())
}
