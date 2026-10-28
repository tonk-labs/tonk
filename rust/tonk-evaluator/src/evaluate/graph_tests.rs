use super::*;
use dialog_peer::helpers::{test_repo, test_session_with_peer};

const PEOPLE: &str = r#"
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

fn syntax(text: &str) -> Syntax {
    let parsed = tonk_notation::parse(text);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    parsed.syntax.expect("valid syntax")
}

#[dialog_common::test]
async fn cyclic_graph_reads_the_transaction_overlay_and_roundtrips_the_worker_dto()
-> anyhow::Result<()> {
    let (operator, profile) = test_session_with_peer().await;
    let repo = test_repo(&operator, &profile).await;
    let branch = repo.branch("main").open().perform(&operator).await?;
    syntax(PEOPLE)
        .evaluate(branch.transaction())
        .perform(&operator)
        .await?
        .commit()
        .publish()
        .perform(&operator)
        .await?;

    let evaluated = syntax(
        r#"
person:
  this: id:alice
person!:
  this: id:bob
  name: "Robert"
"#,
    )
    .evaluate(branch.transaction())
    .perform(&operator)
    .await?;
    let before = evaluated.matches[0].graph.as_ref().expect("typed graph");
    assert_eq!(before.roots, vec!["id:alice".parse::<Entity>()?]);
    assert_eq!(before.nodes.len(), 2, "cycle is finite and normalized");
    let bob: Entity = "id:bob".parse()?;
    assert_eq!(
        before.nodes[&bob].values().next().unwrap()["name"],
        vec![Value::String("Bob".into())]
    );
    let after = evaluated.matches_after(&operator).await?;
    let graph = after[0].graph.as_ref().expect("overlay graph");
    assert_eq!(
        graph.nodes[&bob].values().next().unwrap()["name"],
        vec![Value::String("Robert".into())]
    );
    let wire = serde_json::to_value(&after[0])?;
    let decoded: tonk_worker_api::QueryMatchBlock = serde_json::from_value(wire.clone())?;
    assert_eq!(serde_json::to_value(decoded)?, wire);
    Ok(())
}

#[dialog_common::test]
async fn a_required_typed_edge_filters_the_root() -> anyhow::Result<()> {
    let (operator, profile) = test_session_with_peer().await;
    let repo = test_repo(&operator, &profile).await;
    let branch = repo.branch("main").open().perform(&operator).await?;
    let source = format!(
        r#"{PEOPLE}
concept!: &issue
  with:
    title: {{ the: example.issue/title, as: text, description: "Title" }}
    owner: {{ the: example.issue/owner, as: person, description: "Owner" }}
issue!:
  this: id:good
  title: "Good"
  owner: id:alice
issue!:
  this: id:bad
  title: "Bad"
  owner: id:unknown
"#
    );
    syntax(&source)
        .evaluate(branch.transaction())
        .perform(&operator)
        .await?
        .commit()
        .publish()
        .perform(&operator)
        .await?;
    let evaluated = syntax("issue:\n")
        .evaluate(branch.transaction())
        .perform(&operator)
        .await?;
    assert_eq!(evaluated.matches[0].results.len(), 1);
    assert_eq!(evaluated.matches[0].results[0].this, "id:good");
    assert_eq!(
        evaluated.matches[0].graph.as_ref().unwrap().roots,
        vec!["id:good".parse::<Entity>()?]
    );
    Ok(())
}

#[dialog_common::test]
async fn graph_matching_and_shaping_have_distinct_parent_semantics() -> anyhow::Result<()> {
    let (operator, profile) = test_session_with_peer().await;
    let repo = test_repo(&operator, &profile).await;
    let branch = repo.branch("main").open().perform(&operator).await?;
    let source = format!(
        r#"{PEOPLE}
concept!: &issue
  with:
    title: {{ the: example.issue/title, as: text, description: "Title" }}
    owner: {{ the: example.issue/owner, as: person, description: "Owner" }}
issue!:
  this: id:first
  title: "First"
  owner: id:alice
issue!:
  this: id:second
  title: "Second"
  owner: id:bob
"#
    );
    syntax(&source)
        .evaluate(branch.transaction())
        .perform(&operator)
        .await?
        .commit()
        .publish()
        .perform(&operator)
        .await?;
    let matching = syntax(
        r#"issue:
  graph:
    matching:
      owner: { name: "Alice" }
"#,
    )
    .evaluate(branch.transaction())
    .perform(&operator)
    .await?;
    assert_eq!(matching.matches[0].results.len(), 1);
    assert_eq!(matching.matches[0].results[0].this, "id:first");
    let shaping = syntax(
        r#"issue:
  graph:
    shaping:
      owner: { name: "Alice" }
"#,
    )
    .evaluate(branch.transaction())
    .perform(&operator)
    .await?;
    assert_eq!(
        shaping.matches[0].results.len(),
        2,
        "shaping retains parents"
    );
    let graph = shaping.matches[0].graph.as_ref().unwrap();
    let second: Entity = "id:second".parse()?;
    let bag = graph.nodes[&second].values().next().unwrap();
    assert!(
        bag.get("owner").is_none_or(Vec::is_empty),
        "nonmatching target is excluded from edge"
    );
    Ok(())
}

#[dialog_common::test]
async fn optional_many_edges_keep_empty_parents_and_incomplete_child_bags() -> anyhow::Result<()> {
    let (operator, profile) = test_session_with_peer().await;
    let repo = test_repo(&operator, &profile).await;
    let branch = repo.branch("main").open().perform(&operator).await?;
    let source = r#"
concept!: &comment
  with:
    body: { the: example.comment/body, as: text, description: "Body" }
    author: { the: example.comment/author, as: entity, description: "Author" }
concept!: &issue
  with:
    title: { the: example.issue/title, as: text, description: "Title" }
  maybe:
    comments: { the: example.issue/comments, as: comment, pick: :all, description: "Comments" }
issue!:
  this: id:empty
  title: "Empty"
issue!:
  this: id:partial
  title: "Partial"
  comments: id:comment
example.comment!:
  this: id:comment
  body: "No author yet"
"#;
    syntax(source)
        .evaluate(branch.transaction())
        .perform(&operator)
        .await?
        .commit()
        .publish()
        .perform(&operator)
        .await?;
    let evaluated = syntax("issue:\n")
        .evaluate(branch.transaction())
        .perform(&operator)
        .await?;
    let graph = evaluated.matches[0].graph.as_ref().unwrap();
    assert_eq!(graph.roots.len(), 2);
    let empty: Entity = "id:empty".parse()?;
    assert!(graph.nodes[&empty].values().next().unwrap()["comments"].is_empty());
    let child: Entity = "id:comment".parse()?;
    let bag = graph.nodes[&child].values().next().unwrap();
    assert_eq!(bag["body"], vec![Value::String("No author yet".into())]);
    assert!(!bag.contains_key("author"));
    Ok(())
}

#[dialog_common::test]
async fn replacing_a_pinned_typed_field_with_entity_clears_its_edge() -> anyhow::Result<()> {
    let (operator, profile) = test_session_with_peer().await;
    let repo = test_repo(&operator, &profile).await;
    let branch = repo.branch("main").open().perform(&operator).await?;
    let source = PEOPLE.replace(
        "concept!: &person",
        "concept!: &person\n  this: id:person-schema",
    );
    syntax(&source)
        .evaluate(branch.transaction())
        .perform(&operator)
        .await?
        .commit()
        .publish()
        .perform(&operator)
        .await?;
    syntax(
        r#"concept!:
  this: id:person-schema
  with:
    name: { the: example.person/name, as: text, description: "Name" }
  maybe:
    manager: { the: example.person/manager, as: entity, description: "Manager" }
"#,
    )
    .evaluate(branch.transaction())
    .perform(&operator)
    .await?
    .commit()
    .publish()
    .perform(&operator)
    .await?;
    let evaluated = syntax("person:\n")
        .evaluate(branch.transaction())
        .perform(&operator)
        .await?;
    assert!(
        evaluated.matches[0].graph.is_none(),
        "no stale typed edge survives redefinition"
    );
    assert_eq!(evaluated.matches[0].results.len(), 2);
    Ok(())
}
