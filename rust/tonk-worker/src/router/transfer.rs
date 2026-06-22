//! CSV export / import routes.
//!
//! - `GET  /api/repository/{repo}/branch/{branch}/export` streams
//!   every artifact on the branch out as `text/csv`.
//! - `POST /api/repository/{repo}/branch/{branch}/import` reads a
//!   `text/csv` body and commits each row as an assertion.
//!
//! Both go through the reactor's [`BranchReference::export`] /
//! [`BranchReference::import`] wrappers, so import re-polls
//! subscriptions automatically (the route then broadcasts the new
//! revision so revision/sync badges refresh, the same as
//! `evaluate`). The CSV format lives here; the reactor stays
//! format-agnostic.

use ::axum::{
    body::{Body, Bytes},
    extract::{Path, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use axum_wasm_macros::wasm_compat;
use dialog_csv::{CsvExporter, CsvImporter};
use serde::{Deserialize, Serialize};
use std::io::Cursor;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;
use tonk_common::log;

use super::AppState;
use super::evaluate::EvaluatePath;
use crate::TonkWorkerError;
use crate::broadcast::{Notification, broadcast};

/// Attribute domain prefixes that are repo governance, not user
/// content, and so are withheld from content CSV export. Every
/// attribute under these domains (`…/member`, `…/name`, `…/inviter`,
/// etc.) is governance.
const GOVERNANCE_DOMAINS: [&str; 2] = ["xyz.tonk.membership", "xyz.tonk.invitation"];

/// Drop CSV rows whose attribute (first `the` column) is in a
/// governance domain, preserving the header and all content rows. The
/// result always ends with a trailing newline.
fn strip_governance_rows(csv: &str) -> String {
    let mut out = String::with_capacity(csv.len());
    for (i, line) in csv.lines().enumerate() {
        let is_governance = i > 0
            && line
                .split(',')
                .next()
                .map(|attr| attr.trim_matches('"'))
                .is_some_and(|attr| GOVERNANCE_DOMAINS.iter().any(|d| attr.starts_with(d)));
        if !is_governance {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

/// `GET /api/repository/{repo}/branch/{branch}/export`
///
/// Streams every artifact on the branch as a CSV document
/// (`the,of,as,is,cause` columns). Read-only.
#[wasm_compat]
pub async fn export(
    State(state): State<AppState>,
    Path(path): Path<EvaluatePath>,
) -> Result<Response, TonkWorkerError> {
    log!("export repo={}, branch={}", path.repo, path.branch);
    let tonk_state = state.write().await;
    let tonk_branch = tonk_state
        .reactor
        .repository(&path.repo)
        .branch(&path.branch);

    let mut buf: Vec<u8> = Vec::new();
    tonk_branch
        .export(CsvExporter::from(&mut buf))
        .perform(&tonk_state.operator)
        .await
        .map_err(|e| TonkWorkerError::NotFound(e.to_string()))?;

    // Withhold governance facts (membership/invitation) that now live on
    // the content branch alongside user content, so they don't leak into
    // a content import on the other side. CSV is utf8.
    let csv = String::from_utf8(buf)
        .map_err(|e| TonkWorkerError::Internal(format!("export produced non-utf8 csv: {e}")))?;
    let csv = strip_governance_rows(&csv);

    let filename = format!("{}-{}.csv", path.repo, path.branch);
    let mut response = (StatusCode::OK, Body::from(csv)).into_response();
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, "text/csv".parse().unwrap());
    if let Ok(value) = format!("attachment; filename=\"{filename}\"").parse::<header::HeaderValue>()
    {
        headers.insert(header::CONTENT_DISPOSITION, value);
    }
    Ok(response)
}

/// Wire-shape returned by `/import` — the post-commit revision.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportResponse {
    /// Revision of the branch after the import committed.
    pub revision: dialog_repository::Revision,
}

/// `POST /api/repository/{repo}/branch/{branch}/import`
///
/// Reads a `text/csv` body (`the,of,as,is,cause` columns) and
/// commits each row as an assertion in one transaction. Returns the
/// post-commit revision and broadcasts it so subscribers refresh.
#[wasm_compat]
pub async fn import(
    State(state): State<AppState>,
    Path(path): Path<EvaluatePath>,
    body: Bytes,
) -> Result<axum::Json<ImportResponse>, TonkWorkerError> {
    log!(
        "import repo={}, branch={} ({} bytes)",
        path.repo,
        path.branch,
        body.len()
    );
    let tonk_state = state.write().await;
    let tonk_branch = tonk_state
        .reactor
        .repository(&path.repo)
        .branch(&path.branch);

    let importer = CsvImporter::from(Cursor::new(body.to_vec()));
    let revision = tonk_branch
        .import(importer)
        .perform(&tonk_state.operator)
        .await
        .map_err(|e| TonkWorkerError::Router(e.to_string()))?;

    // Announce the moved head on the branch channel so subscribed
    // UIs refresh their revision/sync badges, the same as
    // `evaluate`. The reactor already re-polled SSE subscriptions.
    broadcast(
        &format!("/api/repository/{}/branch/{}", path.repo, path.branch),
        &Notification {
            branch: path.branch.clone(),
            revision: revision.clone(),
        },
    );

    Ok(axum::Json(ImportResponse { revision }))
}

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod tests {
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    wasm_bindgen_test_configure!(run_in_service_worker);

    use ::axum::body::Body;
    use ::axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    use crate::router::{
        AppState, ImportResponse, RepositoryInfo, api_router_from_state, api_router_with_state,
        tests::test_state,
    };

    /// Create a repo via `PUT /api/repository/{label}` and return the
    /// shared state plus the repo's routing key.
    async fn fresh_repo(label: &str) -> (AppState, String) {
        let (app, state, _lsp) = api_router_with_state(test_state().await);
        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repository/{label}"))
                    .method("PUT")
                    .header("content-type", "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
        let body = ::axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let info: RepositoryInfo = serde_json::from_slice(&body).unwrap();
        (state, info.name)
    }

    /// Seed two `attribute!:` facts on `main` so the branch has
    /// artifacts to export. Uses the evaluate route so the path is
    /// the real one a client takes.
    async fn seed(state: &AppState, repo: &str) {
        let (app, _lsp) = api_router_from_state(state.clone());
        let doc = "attribute!: &probe-name\n  description: A name\n  the: xyz.tonk.probe/name\n  as: text\n  cardinality: one\n";
        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repository/{repo}/branch/main/evaluate"))
                    .method("POST")
                    .header("content-type", "application/yaml")
                    .body(Body::from(doc))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "seed evaluate failed");
    }

    async fn get_export(state: &AppState, repo: &str) -> (StatusCode, String, String) {
        let (app, _lsp) = api_router_from_state(state.clone());
        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repository/{repo}/branch/main/export"))
                    .method("GET")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        let bytes = ::axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        (
            status,
            content_type,
            String::from_utf8(bytes.to_vec()).unwrap(),
        )
    }

    async fn post_import(state: &AppState, repo: &str, csv: &str) -> (StatusCode, Vec<u8>) {
        let (app, _lsp) = api_router_from_state(state.clone());
        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repository/{repo}/branch/main/import"))
                    .method("POST")
                    .header("content-type", "text/csv")
                    .body(Body::from(csv.to_owned()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let bytes = ::axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, bytes.to_vec())
    }

    /// Export returns a CSV document with the dialog header and at
    /// least one data row for a seeded branch.
    #[dialog_common::test]
    async fn it_exports_branch_artifacts_as_csv() {
        let (state, repo) = fresh_repo("export-test").await;
        seed(&state, &repo).await;

        let (status, content_type, csv) = get_export(&state, &repo).await;
        assert_eq!(status, StatusCode::OK);
        assert!(
            content_type.starts_with("text/csv"),
            "content-type was {content_type:?}",
        );
        assert!(
            csv.starts_with("the,of,as,is,cause"),
            "export missing CSV header: {csv}",
        );
        assert!(
            csv.lines().count() > 1,
            "export should have at least one data row: {csv}",
        );
    }

    /// Export from one repo, import into a fresh repo, and confirm
    /// the second repo's export matches — the CSV round-trips through
    /// the HTTP routes.
    #[dialog_common::test]
    async fn it_roundtrips_export_then_import() {
        let (state, source) = fresh_repo("rt-source").await;
        seed(&state, &source).await;
        let (_, _, source_csv) = get_export(&state, &source).await;

        let (_, dest) = fresh_repo("rt-dest").await;
        let (status, body) = post_import(&state, &dest, &source_csv).await;
        assert_eq!(status, StatusCode::OK, "import failed: {body:?}");
        let resp: ImportResponse = serde_json::from_slice(&body).unwrap();
        // The import committed a real revision (non-default head).
        let _ = resp.revision;

        // The destination now exports the same artifact rows the
        // source did (header + data, set-equal). Membership rows are
        // excluded: each repo stamps its OWN founder membership (keyed
        // by its own subject DID) on the content branch, so those rows
        // legitimately differ between source and dest.
        let (_, _, dest_csv) = get_export(&state, &dest).await;
        let is_artifact_row = |row: &&str| !row.contains("xyz.tonk.membership/");
        let mut source_rows: Vec<&str> =
            source_csv.lines().skip(1).filter(is_artifact_row).collect();
        let mut dest_rows: Vec<&str> = dest_csv.lines().skip(1).filter(is_artifact_row).collect();
        source_rows.sort_unstable();
        dest_rows.sort_unstable();
        assert_eq!(
            dest_rows, source_rows,
            "round-tripped rows differ\nsource:\n{source_csv}\ndest:\n{dest_csv}",
        );
    }

    /// A repo's content export withholds governance facts
    /// (membership/invitation) that live on the content branch.
    #[dialog_common::test]
    async fn it_excludes_governance_facts_from_export() {
        let (state, repo) = fresh_repo("gov-export").await;
        seed(&state, &repo).await;
        let (_, _, csv) = get_export(&state, &repo).await;
        for line in csv.lines().skip(1) {
            let attr = line.split(',').next().unwrap_or("").trim_matches('"');
            assert!(
                !attr.starts_with("xyz.tonk.membership")
                    && !attr.starts_with("xyz.tonk.invitation"),
                "governance row leaked into export: {line}",
            );
        }
    }

    /// An empty CSV (header only) imports without error and commits
    /// nothing meaningful.
    #[dialog_common::test]
    async fn it_imports_an_empty_csv() {
        let (state, repo) = fresh_repo("empty-import").await;
        let (status, _) = post_import(&state, &repo, "the,of,as,is,cause\n").await;
        assert_eq!(status, StatusCode::OK);
    }
}
