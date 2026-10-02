//! `intent/suggest` end to end: a space seeded with the core library,
//! asked over the `/query` route exactly as the `<command-palette>` element
//! asks (through the portal, which relays `window.tonk.query` there), and
//! the reading it returns transacted.
//!
//! Native, on the same fixture as the other native router tests.

use axum::Router;
use axum::body::Body;
use axum::http::Request;
use serde_json::{Value, json};
use tower::ServiceExt;

use super::api_router_with_state;
use super::repository::profile_library_tests::test_state;

const CORE: &str = include_str!("../../../tonk-core/assets/library/core.yaml");

/// The core library leaves the space's rename unsaid (the profile's says
/// it); say it here, so a rule-handled command runs end to end.
const RENAME: &str = r#"
intent/action!:
  this: tonk/rename-repository
  name: "rename"

intent/argument!:
  command: tonk/rename-repository
  field: rename-repository/subject
  role: intent/object
  noun: tonk/repository

intent/argument!:
  command: tonk/rename-repository
  field: rename-repository/name
  role: intent/goal
"#;

async fn send(app: &Router, method: &str, uri: &str, kind: &str, body: String) -> Value {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(uri)
                .method(method)
                .header("content-type", kind)
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    assert!(
        status.is_success(),
        "{method} {uri}: {status} {}",
        String::from_utf8_lossy(&bytes)
    );
    serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

/// Ask `intent/suggest` what `input` could mean, as the element does.
async fn suggest(app: &Router, key: &str, input: &str, this: &str) -> Vec<Value> {
    let rows = send(
        app,
        "POST",
        &format!("/api/repository/{key}/branch/main/query"),
        "application/json",
        json!({
            "predicate": "intent/suggest",
            "terms": { "input": input, "this": this, "now": 1.0 }
        })
        .to_string(),
    )
    .await;
    rows.as_array().cloned().unwrap_or_default()
}

fn field<'a>(row: &'a Value, name: &str) -> &'a Value {
    &row["fields"][name]
}

#[dialog_common::test]
async fn it_suggests_from_a_seeded_space_and_runs_what_it_suggests() {
    let (app, _state, _lsp) = api_router_with_state(test_state().await);
    let created = send(
        &app,
        "PUT",
        "/api/repository/palette",
        "application/json",
        "{}".into(),
    )
    .await;
    let key = created["name"].as_str().unwrap().to_owned();
    let subject = created["subject"].as_str().unwrap().to_owned();
    let evaluate = format!("/api/repository/{key}/branch/main/evaluate");
    send(&app, "POST", &evaluate, "application/yaml", CORE.into()).await;
    send(&app, "POST", &evaluate, "application/yaml", RENAME.into()).await;
    send(
        &app,
        "POST",
        &evaluate,
        "application/yaml",
        format!("tonk/repository!:\n  this: {subject}\n  name: \"Budget\"\n"),
    )
    .await;

    // An empty argument is labelled by its noun, and nothing runs yet.
    let rows = suggest(&app, &key, "expel", &subject).await;
    let expel = rows
        .iter()
        .find(|row| field(row, "text") == "expel (member)")
        .unwrap_or_else(|| panic!("expel is suggested: {rows:?}"));
    assert_eq!(field(expel, "claim"), &Value::Null);
    assert_eq!(field(expel, "rank"), 0);

    // Typing a verb's start completes it, up to the first empty argument.
    let rows = suggest(&app, &key, "ren", &subject).await;
    assert_eq!(field(&rows[0], "completion"), "rename Budget to ");

    // "this" is the space; the goal is typed text.
    let rows = suggest(&app, &key, "rename this to Q3", &subject).await;
    let top = &rows[0];
    assert_eq!(field(top, "text"), "rename [Budget] to [Q3]");
    let claim = field(top, "claim")
        .as_str()
        .expect("a complete reading runs");
    send(
        &app,
        "POST",
        &format!("/api/repository/{key}/branch/main/transact"),
        "application/json",
        claim.into(),
    )
    .await;

    let renamed = send(
        &app,
        "POST",
        &format!("/api/repository/{key}/branch/main/query"),
        "application/json",
        json!({
            "predicate": { "with": {
                "name": { "the": "xyz.tonk.repo/name", "as": "Text", "cardinality": "one" }
            } },
            "terms": { "this": subject, "name": { "?": { "name": "name" } } }
        })
        .to_string(),
    )
    .await;
    assert_eq!(
        renamed[0]["fields"]["name"], "Q3",
        "the rename rule applied the suggested claim: {renamed}"
    );

    // With nothing typed: what can be done without saying more. Rename
    // wants a name and expel a member, so neither is offered.
    let menu = suggest(&app, &key, "", &subject).await;
    assert!(
        menu.iter().all(|row| field(row, "claim").is_string()),
        "the menu offers only what runs as it is: {menu:?}"
    );
}

/// A command whose fields say their own role, with no `intent/argument`
/// facts: the field's attribute carries `role: goal`, so typed text after
/// "to" fills it.
const RETITLE: &str = r#"
command!: &test/retitle
  description: "Retitle the test thing"
  with:
    title:
      description: "The new title"
      the: io.test.retitle/title
      as: text
      role: goal

intent/action!:
  this: test/retitle
  name: "retitle"
"#;

#[dialog_common::test]
async fn it_fills_a_field_by_the_role_on_its_attribute() {
    let (app, _state, _lsp) = api_router_with_state(test_state().await);
    let created = send(
        &app,
        "PUT",
        "/api/repository/palette",
        "application/json",
        "{}".into(),
    )
    .await;
    let key = created["name"].as_str().unwrap().to_owned();
    let subject = created["subject"].as_str().unwrap().to_owned();
    let evaluate = format!("/api/repository/{key}/branch/main/evaluate");
    send(&app, "POST", &evaluate, "application/yaml", CORE.into()).await;
    send(&app, "POST", &evaluate, "application/yaml", RETITLE.into()).await;

    let rows = suggest(&app, &key, "retitle to Plans", &subject).await;
    let top = rows
        .iter()
        .find(|row| field(row, "text") == "retitle to [Plans]")
        .unwrap_or_else(|| panic!("retitle is read with its goal: {rows:?}"));
    let claim: Value = serde_json::from_str(
        field(top, "claim")
            .as_str()
            .expect("the only field is filled, so it runs"),
    )
    .unwrap();
    assert!(
        claim.to_string().contains("io.test.retitle/title") && claim.to_string().contains("Plans"),
        "the claim sets the title: {claim}"
    );
}

/// How `intent/suggest` scales with a noun's rows, store reads included.
/// Run with `cargo test --release -p tonk-worker --lib -- --ignored
/// --nocapture intent::it_scales`.
#[dialog_common::test]
#[ignore = "timing, not a check"]
async fn it_scales_with_candidates() {
    for count in [10usize, 100, 1_000, 5_000] {
        let (app, _state, _lsp) = api_router_with_state(test_state().await);
        let created = send(
            &app,
            "PUT",
            "/api/repository/scale",
            "application/json",
            "{}".into(),
        )
        .await;
        let key = created["name"].as_str().unwrap().to_owned();
        let subject = created["subject"].as_str().unwrap().to_owned();
        let evaluate = format!("/api/repository/{key}/branch/main/evaluate");
        send(&app, "POST", &evaluate, "application/yaml", CORE.into()).await;
        send(&app, "POST", &evaluate, "application/yaml", RENAME.into()).await;
        let seeded = std::time::Instant::now();
        let mut yaml = String::new();
        for n in 0..count {
            yaml.push_str(&format!(
                "tonk/repository!:\n  this: did:key:z6Mkscale{n}\n  name: \"Budget {n}\"\n\n"
            ));
        }
        send(&app, "POST", &evaluate, "application/yaml", yaml).await;
        let seeded = seeded.elapsed();
        for input in ["ex", "rename budget 42 to Q3"] {
            let runs = 3;
            let start = std::time::Instant::now();
            for _ in 0..runs {
                std::hint::black_box(suggest(&app, &key, input, &subject).await);
            }
            println!(
                "{count:>6} rows  {input:<24} {:>9.1} ms  (seeded in {:.1} s)",
                start.elapsed().as_secs_f64() * 1000.0 / f64::from(runs),
                seeded.as_secs_f64()
            );
        }
    }
}

/// How one `/evaluate` of `count` rows scales. Run with `cargo test
/// --release -p tonk-worker --lib -- --ignored --nocapture
/// intent::it_evaluates`.
#[dialog_common::test]
#[ignore = "timing, not a check"]
async fn it_evaluates_many_rows() {
    let counts: Vec<usize> = std::env::var("ROWS")
        .ok()
        .map(|rows| rows.split(',').filter_map(|n| n.parse().ok()).collect())
        .unwrap_or_else(|| vec![250, 500, 1_000, 2_000]);
    for count in counts {
        let (app, _state, _lsp) = api_router_with_state(test_state().await);
        let created = send(
            &app,
            "PUT",
            "/api/repository/scale",
            "application/json",
            "{}".into(),
        )
        .await;
        let key = created["name"].as_str().unwrap().to_owned();
        let evaluate = format!("/api/repository/{key}/branch/main/evaluate");
        send(&app, "POST", &evaluate, "application/yaml", CORE.into()).await;
        let mut yaml = String::new();
        for n in 0..count {
            yaml.push_str(&format!(
                "tonk/repository!:\n  this: did:key:z6Mkscale{n}\n  name: \"Budget {n}\"\n\n"
            ));
        }
        let start = std::time::Instant::now();
        send(&app, "POST", &evaluate, "application/yaml", yaml).await;
        let elapsed = start.elapsed().as_secs_f64();
        println!(
            "{count:>6} rows  {elapsed:>7.2} s  {:>6.2} ms/row",
            elapsed * 1000.0 / count as f64
        );
    }
}
