use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use dialog_artifacts::{ArtifactSelector, Value};
use dialog_effects::storage::Directory;
use dialog_repository::{RepositoryExt as _, Revision, codec};
use futures_util::StreamExt as _;
use tonk_worker::{StateLock, TonkState, api_router_from_state};
use tower::ServiceExt as _;

use super::local_spaces;

/// The registry profile a native worker opens, as `TonkState::open_native`
/// names it.
const NATIVE_REGISTRY: &str = "hosted";

const NOTE: &str = r#"concept!: &space-name
  description: A space's name, as a space keeps it.
  with:
    name:
      description: The name.
      the: xyz.tonk.repo/name
      as: text

concept!: &note
  description: A note.
  with:
    body:
      description: What the note says.
      the: xyz.tonk.test-note/body
      as: text
"#;

async fn call(app: &axum::Router, method: &str, uri: &str, body: &str) -> (StatusCode, String) {
    let request = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(body.to_owned()))
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

/// A space created through the worker under `label`, its routing key.
async fn space(app: &axum::Router, label: &str) -> String {
    let (status, body) = call(app, "PUT", &format!("/api/repository/{label}"), "{}").await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let info: serde_json::Value = serde_json::from_str(&body).unwrap();
    info["name"].as_str().unwrap().to_owned()
}

async fn evaluate(app: &axum::Router, key: &str, document: &str) {
    let (status, body) = call(
        app,
        "POST",
        &format!("/api/repository/{key}/branch/main/evaluate?transact=true"),
        document,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

/// The directories a native worker keeps its profiles and spaces in.
fn kept_in(root: &std::path::Path) -> (Directory, Directory) {
    let root = root.canonicalize().unwrap();
    (
        Directory::At(root.to_string_lossy().into_owned()),
        Directory::At(root.join("spaces").to_string_lossy().into_owned()),
    )
}

#[dialog_common::test]
async fn it_reads_a_space_kept_on_this_device_as_a_copy_is_made_from() {
    let root = tempfile::tempdir().unwrap();
    let key = {
        let state = TonkState::open_native(root.path()).await.unwrap();
        let (app, _lsp) = api_router_from_state(Arc::new(StateLock::new(state)));
        let key = space(&app, "Kept%20notes").await;
        let subject = if key.starts_with("did:") {
            key.clone()
        } else {
            format!("did:key:{key}")
        };
        evaluate(&app, &key, NOTE).await;
        evaluate(
            &app,
            &key,
            &format!("space-name!:\n  this: {subject}\n  name: \"Kept notes\"\n"),
        )
        .await;
        evaluate(
            &app,
            &key,
            "note!: &kept\n  body: \"kept on this device\"\n",
        )
        .await;
        key
    };

    let (directory, base) = kept_in(root.path());
    let found = local_spaces(directory, base, NATIVE_REGISTRY)
        .await
        .unwrap();
    assert_eq!(found.len(), 1, "the one space is found: {found:?}");
    let kept = &found[0];
    assert!(key.ends_with(kept.subject.as_str()) || kept.subject.as_str().ends_with(&key));
    assert_eq!(kept.name, "Kept notes");

    // What is read is what a copy is made from: imported into another
    // worker's space, its revision holds the note.
    let other = tempfile::tempdir().unwrap();
    let state = TonkState::open_native(other.path()).await.unwrap();
    let (app, _lsp) = api_router_from_state(Arc::new(StateLock::new(state.clone())));
    let copy = space(&app, "Copy").await;
    let repository = state
        .profile
        .space(copy.as_str())
        .load()
        .perform(&state.operator)
        .await
        .unwrap();
    let items = codec::decode(&kept.content).unwrap();
    repository
        .import(futures_util::stream::iter(items))
        .perform(&state.operator)
        .await
        .unwrap();
    let revision: Revision = serde_json::from_slice(&kept.revision).unwrap();
    let bodies: Vec<Value> = repository
        .snapshot(revision)
        .claims()
        .select(ArtifactSelector::new().the("xyz.tonk.test-note/body".parse().unwrap()))
        .perform(&state.operator)
        .await
        .unwrap()
        .map(|artifact| artifact.unwrap().to_owned().unwrap().is)
        .collect()
        .await;
    assert_eq!(bodies, vec![Value::String("kept on this device".into())]);
}

#[dialog_common::test]
async fn it_finds_nothing_where_no_space_was_made() {
    let root = tempfile::tempdir().unwrap();
    drop(TonkState::open_native(root.path()).await.unwrap());
    let (directory, base) = kept_in(root.path());
    let found = local_spaces(directory, base, NATIVE_REGISTRY)
        .await
        .unwrap();
    assert!(
        found.is_empty(),
        "the profile itself is no space to bring: {found:?}"
    );
}
