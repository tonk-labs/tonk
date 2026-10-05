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
const NOTEBOOK: &str = include_str!("../../../tonk-core/assets/library/notebook.yaml");
const PROFILE: &str = include_str!("../../../tonk-core/assets/library/profile.yaml");

/// The core library leaves the space's rename unsaid (the profile's says
/// it); say it here, so a rule-handled command runs end to end: the
/// command's name, the roles of its fields (re-declaring an attribute
/// with a role is the same attribute), and a rule offering the space as
/// what is renamed.
const RENAME: &str = r#"
attribute!: &rename-repository/subject
  description: The repository being renamed.
  the: xyz.tonk.rename-repository/subject
  as: entity
  role: object

attribute!: &rename-repository/name
  description: The new name.
  the: xyz.tonk.rename-repository/name
  as: text
  role: goal

intent/action!:
  this: tonk/rename-repository
  name: "rename"

rule!:
  description: The space is what a rename from its palette renames.
  assert: rename-repository/subject
  when:
    - assert: intent
      where: { this: ?this, command: tonk/rename-repository }
    - assert: tonk/repository
      where: { this: ?subject }
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

    // The palette interprets each line in a tab (its site), then reads the
    // readings back.
    let space = format!("/api/repository/{key}/branch/main");
    let page = "site:0b6e9a7c-1d2f-4e3a-8b5c-6d7e8f9a0b1c";
    let expression = "intent:test-suggest";
    let read = |input: &'static str, time: f64| {
        let (app, space, subject) = (&app, &space, &subject);
        async move {
            interpreted(app, space, expression, input, page, time).await;
            readings(app, space, expression, subject).await
        }
    };

    // An empty argument is labelled by its field, and nothing runs yet:
    // the space has no members for a rule to offer.
    let rows = read("expel", 1.0).await;
    let expel = rows
        .iter()
        .find(|row| field(row, "text") == "expel (member)")
        .unwrap_or_else(|| panic!("expel is suggested: {rows:?}"));
    assert_eq!(field(expel, "claim"), &Value::Null);
    assert_eq!(field(expel, "rank"), 0);

    // Typing a verb's start completes it, up to the first empty argument.
    let rows = read("ren", 2.0).await;
    assert_eq!(field(&rows[0], "completion"), "rename Budget to ");

    // "this" is the space; the goal is typed text.
    let rows = read("rename this to Q3", 3.0).await;
    let top = &rows[0];
    assert_eq!(field(top, "text"), "rename [Budget] to [Q3]");
    let claim = field(top, "claim")
        .as_str()
        .expect("a complete reading runs");
    send(
        &app,
        "POST",
        &format!("{space}/transact"),
        "application/json",
        claim.into(),
    )
    .await;

    let renamed = send(
        &app,
        "POST",
        &format!("{space}/query"),
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
    let menu = read("", 4.0).await;
    assert!(
        menu.iter().all(|row| field(row, "claim").is_string()),
        "the menu offers only what runs as it is: {menu:?}"
    );
}

/// A command whose fields say their own role: the field's attribute
/// carries `role: goal`, so typed text after "to" fills it.
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
    // With text selected in the page, the bare verb takes it as the title.
    let site = "site:0b5c8e7a-6f0e-4b1d-8c2e-7d9a1f3e5b40";
    send(
        &app,
        "POST",
        &format!("/api/repository/{key}/branch/main/transact"),
        "application/json",
        select(site, "Roadmap", 1.0),
    )
    .await;
    let space = format!("/api/repository/{key}/branch/main");
    selection(&app, &space, site, Some("Roadmap")).await;
    let rows = send(
        &app,
        "POST",
        &format!("{space}/query"),
        "application/json",
        json!({
            "predicate": "intent/suggest",
            "terms": { "input": "retitle", "this": subject, "now": 1.0, "site": site }
        })
        .to_string(),
    )
    .await;
    let rows = rows.as_array().cloned().unwrap_or_default();
    assert!(
        rows.iter()
            .any(|row| field(row, "text") == "retitle to [Roadmap]"),
        "the selection fills the title: {rows:?}"
    );
}

/// A `site/select` report, as `bootstrap.js` sends it.
fn select(site: &str, text: &str, time: f64) -> String {
    json!({
        "claims": [{
            "op": "assert",
            "application": {
                "predicate": {
                    "kind": "transient",
                    "concept": {
                        "description": "Report what the page in a tab has selected.",
                        "with": {
                            "site": { "the": "xyz.tonk.command.site-select/site", "as": "Entity" },
                            "text": { "the": "xyz.tonk.command.site-select/text", "as": "Text" },
                            "time": { "the": "xyz.tonk.command.site-select/time", "as": "Float" }
                        }
                    }
                },
                "parameters": { "site": site, "text": text, "time": time }
            }
        }]
    })
    .to_string()
}

/// The selection recorded on `site`, on the branch at `prefix`, once it
/// is `expected` (the command runs after the transact returns).
async fn selection(
    app: &Router,
    prefix: &str,
    site: &str,
    expected: Option<&str>,
) -> Option<String> {
    let mut seen = None;
    for _ in 0..200 {
        let rows = send(
            app,
            "POST",
            &format!("{prefix}/query"),
            "application/json",
            json!({
                "predicate": { "with": {
                    "selection": { "the": "xyz.tonk.site/selection", "as": "Text", "cardinality": "one" }
                } },
                "terms": { "this": site, "selection": { "?": { "name": "selection" } } }
            })
            .to_string(),
        )
        .await;
        seen = rows
            .as_array()
            .and_then(|rows| rows.first())
            .and_then(|row| row["fields"]["selection"].as_str())
            .map(str::to_owned);
        if seen.as_deref() == expected {
            return seen;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    seen
}

#[dialog_common::test]
async fn it_records_a_pages_selection_on_its_site() {
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
    let space = format!("/api/repository/{key}/branch/main");
    let profile = "/api/repository/profile:tonk/branch/main";
    send(
        &app,
        "POST",
        &format!("{space}/evaluate"),
        "application/yaml",
        CORE.into(),
    )
    .await;
    let site = "site:6d3c3f5e-0b0e-4f2a-9a55-3c1d2f0e9a11";

    // Reported from the space: recorded on the space, and on the profile,
    // whose commands the palette proposes too.
    send(
        &app,
        "POST",
        &format!("{space}/transact"),
        "application/json",
        select(site, "Plans", 1.0),
    )
    .await;
    assert_eq!(
        selection(&app, &space, site, Some("Plans"))
            .await
            .as_deref(),
        Some("Plans")
    );
    assert_eq!(
        selection(&app, profile, site, Some("Plans"))
            .await
            .as_deref(),
        Some("Plans")
    );

    // A new selection replaces the old one.
    send(
        &app,
        "POST",
        &format!("{space}/transact"),
        "application/json",
        select(site, "Roadmap", 2.0),
    )
    .await;
    assert_eq!(
        selection(&app, &space, site, Some("Roadmap"))
            .await
            .as_deref(),
        Some("Roadmap")
    );

    // An empty one clears it, on both.
    send(
        &app,
        "POST",
        &format!("{space}/transact"),
        "application/json",
        select(site, "", 3.0),
    )
    .await;
    assert_eq!(selection(&app, &space, site, None).await, None);
    assert_eq!(selection(&app, profile, site, None).await, None);
}

/// An `intent/interpret`, as the palette sends it.
fn interpret(expression: &str, input: &str, site: &str, time: f64) -> String {
    json!({
        "claims": [{
            "op": "assert",
            "application": {
                "predicate": {
                    "kind": "transient",
                    "concept": {
                        "description": "Interpret what was typed in the command palette, where it was opened.",
                        "with": {
                            "expression": { "the": "tonk.dialog.intent.interpret/expression", "as": "Entity" },
                            "input": { "the": "tonk.dialog.intent.interpret/input", "as": "Text" },
                            "site": { "the": "tonk.dialog.intent.interpret/site", "as": "Entity" },
                            "time": { "the": "tonk.dialog.intent.interpret/time", "as": "Float" }
                        }
                    }
                },
                "parameters": { "expression": expression, "input": input, "site": site, "time": time }
            }
        }]
    })
    .to_string()
}

/// Send `input` for `expression` and wait until the handler recorded it.
async fn interpreted(
    app: &Router,
    prefix: &str,
    expression: &str,
    input: &str,
    site: &str,
    time: f64,
) {
    send(
        app,
        "POST",
        &format!("{prefix}/transact"),
        "application/json",
        interpret(expression, input, site, time),
    )
    .await;
    for _ in 0..200 {
        let rows = send(
            app,
            "POST",
            &format!("{prefix}/query"),
            "application/json",
            json!({
                "predicate": { "with": {
                    "input": { "the": "tonk.dialog.intent.expression/input", "as": "Text", "cardinality": "one" }
                } },
                "terms": { "this": expression, "input": { "?": { "name": "input" } } }
            })
            .to_string(),
        )
        .await;
        if rows
            .as_array()
            .and_then(|rows| rows.first())
            .and_then(|row| row["fields"]["input"].as_str())
            == Some(input)
        {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("{input:?} was never interpreted for {expression}");
}

/// The readings of an interpreted expression.
async fn readings(app: &Router, prefix: &str, expression: &str, this: &str) -> Vec<Value> {
    let rows = send(
        app,
        "POST",
        &format!("{prefix}/query"),
        "application/json",
        json!({
            "predicate": "intent/suggest",
            "terms": { "expression": expression, "this": this, "now": 1.0, "max": 8 }
        })
        .to_string(),
    )
    .await;
    rows.as_array().cloned().unwrap_or_default()
}

/// A site on a notebook's page, as the route stamp would leave it.
fn notebook_page(site: &str, key: &str, notebook: &str) -> String {
    format!(
        r#"
site!:
  this: {site}
  path: "/notebook/{notebook}"
  anchor: ""
  space: "{key}"
  branch: "main"
  replica: replica:test
  branch-entity: branch:test
  profile-branch: "main"
  route: route:notebook
  concept: notebook/route

notebook/route!:
  this: {site}
  entity: {notebook}
  repo: "{key}"
  branch: "main"
"#
    )
}

#[dialog_common::test]
async fn it_renames_the_notebook_a_page_shows() {
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
    let space = format!("/api/repository/{key}/branch/main");
    let evaluate = format!("{space}/evaluate");
    send(&app, "POST", &evaluate, "application/yaml", CORE.into()).await;
    send(&app, "POST", &evaluate, "application/yaml", NOTEBOOK.into()).await;
    let page = "site:4a1f0c9e-2b7d-4e8a-9c3f-5d6e7f8a9b0c";
    let notebook = "notebook:plans";
    send(
        &app,
        "POST",
        &evaluate,
        "application/yaml",
        notebook_page(page, &key, notebook),
    )
    .await;

    // "rename to Plans" on the notebook's page: the page's notebook is the
    // subject, the typed text the title.
    let expression = "intent:test-expression-1";
    interpreted(&app, &space, expression, "rename to Plans", page, 1.0).await;
    let rows = readings(&app, &space, expression, &subject).await;
    let retitle = rows
        .iter()
        .find(|row| {
            field(row, "text").as_str() == Some(format!("rename [{notebook}] to [Plans]").as_str())
        })
        .unwrap_or_else(|| panic!("the page's notebook is renamed: {rows:?}"));
    let claim = field(retitle, "claim")
        .as_str()
        .expect("both fields are filled");
    assert!(
        claim.contains(notebook) && claim.contains("Plans"),
        "{claim}"
    );

    // With "Roadmap" selected in the page, bare "rename" takes it as the
    // title.
    send(
        &app,
        "POST",
        &format!("{space}/transact"),
        "application/json",
        select(page, "Roadmap", 2.0),
    )
    .await;
    selection(&app, &space, page, Some("Roadmap")).await;
    interpreted(&app, &space, expression, "rename", page, 2.0).await;
    let rows = readings(&app, &space, expression, &subject).await;
    assert!(
        rows.iter().any(|row| field(row, "text").as_str()
            == Some(format!("rename [{notebook}] to [Roadmap]").as_str())
            && field(row, "claim").is_string()),
        "the selection is the title: {rows:?}"
    );

    // On a page that isn't a notebook's, nothing names a notebook, so the
    // retitle can't run as it is.
    let elsewhere = "site:7c2e1d0b-3a4f-4b5c-8d6e-9f0a1b2c3d4e";
    let other = "intent:test-expression-2";
    interpreted(&app, &space, other, "rename to Plans", elsewhere, 3.0).await;
    let rows = readings(&app, &space, other, &subject).await;
    assert!(
        rows.iter().all(|row| !field(row, "claim")
            .as_str()
            .is_some_and(|claim| claim.contains("notebook.retitle"))),
        "no notebook is renamed off a notebook's page: {rows:?}"
    );
}

/// Send `body` as the tab `client` would: the service worker tags every
/// request a page makes with its client id, which is how a command's
/// handler knows which tab asked.
async fn send_as(app: &Router, client: &str, uri: &str, body: String) -> Value {
    let mut request = Request::builder()
        .uri(uri)
        .method("POST")
        .header("content-type", "application/json")
        .body(Body::from(body))
        .unwrap();
    request
        .extensions_mut()
        .insert(super::ClientId(client.to_owned()));
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    assert!(
        status.is_success(),
        "POST {uri}: {status} {}",
        String::from_utf8_lossy(&bytes)
    );
    serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

/// Every act in the bar's menu, said in the palette as the menu says it,
/// runs as it is and asks the tab that said it to perform that act: the
/// request the bar's `<ui-site-request>` presses the control for.
#[dialog_common::test]
async fn it_runs_every_menu_act_from_the_palette() {
    let (app, state, _lsp) = api_router_with_state(test_state().await);
    let branch = state.read().await.active_branch.clone();
    let profile = format!("/api/repository/profile:tonk/branch/{branch}");
    let client = "tab";
    let site = format!("site:{client}");
    for library in [CORE, PROFILE] {
        send(
            &app,
            "POST",
            &format!("{profile}/evaluate"),
            "application/yaml",
            library.into(),
        )
        .await;
    }

    for (said, request) in [
        ("add an account", "account"),
        ("copy share link", "share"),
        ("view members", "members"),
        ("copy agent link", "agent"),
        ("connect this space", "connect"),
        ("confirm your email", "connect"),
    ] {
        let rows = send(
            &app,
            "POST",
            &format!("{profile}/query"),
            "application/json",
            json!({
                "predicate": "intent/suggest",
                "terms": { "input": said, "this": "did:key:z6MkpTHR8VNsBxYAAWHut2Geadd9jSwuBV8xRoAnwWsdvktH", "now": 1.0 }
            })
            .to_string(),
        )
        .await;
        let rows = rows.as_array().cloned().unwrap_or_default();
        let top = rows
            .first()
            .unwrap_or_else(|| panic!("{said:?} is suggested: {rows:?}"));
        assert_eq!(field(top, "text"), said, "{rows:?}");
        let claim = field(top, "claim")
            .as_str()
            .unwrap_or_else(|| panic!("{said:?} runs as it is: {top:?}"))
            .to_owned();
        send_as(&app, client, &format!("{profile}/transact"), claim).await;

        // The handler runs after the transact returns.
        let mut seen = Value::Null;
        for _ in 0..200 {
            let rows = send(
                &app,
                "POST",
                &format!("{profile}/query"),
                "application/json",
                json!({
                    "predicate": { "with": {
                        "request": { "the": "xyz.tonk.site/request", "as": "Text", "cardinality": "one" }
                    } },
                    "terms": { "this": site, "request": { "?": { "name": "request" } } }
                })
                .to_string(),
            )
            .await;
            seen = rows[0]["fields"]["request"].clone();
            if seen == request {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert_eq!(seen, request, "{said:?} asks the tab for {request:?}");
    }
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

/// "install note" completes to the notebook component, and running the
/// reading installs it beside the space's seed: the space records its
/// install under the component's source.
#[dialog_common::test]
async fn it_installs_a_component_from_the_palette() {
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
    let space = format!("/api/repository/{key}/branch/main");
    send(
        &app,
        "POST",
        &format!("{space}/evaluate"),
        "application/yaml",
        CORE.into(),
    )
    .await;

    let page = "site:6f0d2c8e-4b1a-4c3d-9e2f-1a2b3c4d5e6f";
    let expression = "intent:test-install";
    interpreted(&app, &space, expression, "install note", page, 1.0).await;
    let rows = readings(&app, &space, expression, &subject).await;
    let top = rows
        .first()
        .unwrap_or_else(|| panic!("install is suggested: {rows:?}"));
    assert_eq!(field(top, "text"), "install [notebook]", "{rows:?}");
    let claim = field(top, "claim")
        .as_str()
        .unwrap_or_else(|| panic!("the reading runs as it is: {top:?}"))
        .to_owned();
    send(
        &app,
        "POST",
        &format!("{space}/transact"),
        "application/json",
        claim,
    )
    .await;

    // The handler runs after the transact returns.
    let mut sources = Value::Null;
    for _ in 0..500 {
        sources = send(
            &app,
            "POST",
            &format!("{space}/query"),
            "application/json",
            json!({
                "predicate": { "with": {
                    "source": { "the": "xyz.tonk.seed/source", "as": "Text", "cardinality": "one" }
                } },
                "terms": {
                    "this": { "?": { "name": "this" } },
                    "source": { "?": { "name": "source" } }
                }
            })
            .to_string(),
        )
        .await;
        if sources.as_array().is_some_and(|rows| {
            rows.iter()
                .any(|row| row["fields"]["source"] == "/library/notebook.yaml")
        }) {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("the notebook component was never installed: {sources}");
}
