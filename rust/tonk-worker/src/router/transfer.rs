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
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use dialog_csv::{CsvExporter, CsvImporter};
use serde::{Deserialize, Serialize};
use std::io::Cursor;
use tonk_common::log;
use tonk_macros::handler;

use super::AppState;
use super::evaluate::EvaluatePath;
use crate::TonkWorkerError;

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
#[handler]
pub async fn export(
    State(state): State<AppState>,
    Path(path): Path<EvaluatePath>,
    headers: HeaderMap,
) -> Result<Response, TonkWorkerError> {
    if super::names_profile(&state, &path.repo).await {
        let path = ProfileExportPath {
            branch: path.branch,
        };
        return export_profile(State(state), Path(path), headers).await;
    }
    log!("export repo={}, branch={}", path.repo, path.branch);
    let tonk_state = state.read().await;
    let tonk_branch = tonk_state
        .reactor
        .repository(&path.repo)
        .branch(&path.branch);
    let csv = export_branch_csv(&tonk_state, tonk_branch).await?;
    Ok(csv_response(
        &format!("{}-{}.csv", path.repo, path.branch),
        csv,
    ))
}

/// `GET /api/repository/profile:tonk/branch/{branch}/export`
///
/// The profile's counterpart to [`export`]. The profile is a singleton,
/// so there is no `repo` segment and the branch resolves through
/// `profile_repository()` -- the same split `/query`, `/evaluate` and
/// `/transact` already make.
///
/// This is what lets the profile branch be captured as a fixture: its
/// content (delegations, device links, definitions) is otherwise
/// reachable only one query at a time.
#[handler]
pub async fn export_profile(
    State(state): State<AppState>,
    Path(path): Path<ProfileExportPath>,
    headers: HeaderMap,
) -> Result<Response, TonkWorkerError> {
    log!("export profile branch={}", path.branch);
    let wants_snapshot = headers
        .get(header::ACCEPT)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|accept| accept.contains(SNAPSHOT_MEDIA_TYPE));

    let tonk_state = state.read().await.clone();
    let tonk_branch = tonk_state.reactor.profile_repository().branch(&path.branch);

    // CSV ships FACTS; the snapshot ships the tree itself -- every block
    // and every blob the revision references, which is what a fixture
    // needs to reproduce a real download (delegation envelopes are
    // blobs, and they travel their own channel). The media type names
    // dialog because the format is dialog's snapshot, not tonk's: the
    // bytes are `dialog_repository::Item`s and anything speaking that
    // format can read them.
    if wants_snapshot {
        let body = export_branch_snapshot(&tonk_state, tonk_branch).await?;
        return Ok(snapshot_response(
            &format!("profile-{}.car", path.branch),
            body,
        ));
    }

    let csv = export_branch_csv(&tonk_state, tonk_branch).await?;
    Ok(csv_response(&format!("profile-{}.csv", path.branch), csv))
}

/// Export one branch as CSV, minus governance rows. Shared by the
/// repository and profile routes so the two cannot drift.
async fn export_branch_csv(
    tonk_state: &crate::worker::TonkState,
    tonk_branch: dialog_reactor::BranchReference<'_>,
) -> Result<String, TonkWorkerError> {
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
    Ok(strip_governance_rows(&csv))
}

/// Wrap CSV bytes as a downloadable `text/csv` response.
fn csv_response(filename: &str, csv: String) -> Response {
    let mut response = (StatusCode::OK, Body::from(csv)).into_response();
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, "text/csv".parse().unwrap());
    if let Ok(value) = format!("attachment; filename=\"{filename}\"").parse::<header::HeaderValue>()
    {
        headers.insert(header::CONTENT_DISPOSITION, value);
    }
    response
}

/// Media type selecting the whole-tree snapshot instead of CSV.
///
/// The format is dialog's, and so is the codec: see
/// [`dialog_repository::codec`]. Serving it from here rather
/// than re-implementing the framing means the app and dialog's own tests
/// read the same bytes.
pub use dialog_repository::codec::MEDIA_TYPE as SNAPSHOT_MEDIA_TYPE;

/// Export a branch's whole revision -- blocks and blobs -- as a CARv1.
async fn export_branch_snapshot(
    tonk_state: &crate::worker::TonkState,
    tonk_branch: dialog_reactor::BranchReference<'_>,
) -> Result<Vec<u8>, TonkWorkerError> {
    let branch = tonk_branch
        .acquire(&tonk_state.operator)
        .await
        .map_err(|e| TonkWorkerError::NotFound(e.to_string()))?;
    let revision = branch
        .handle()
        .revision()
        .ok_or_else(|| TonkWorkerError::NotFound("branch has no revision".into()))?;
    let root = dialog_common::Blake3Hash::from(*revision.tree.hash());
    let repository = dialog_repository::Repository::from(tonk_state.profile.did());

    // The profile is a PARTIAL replica: login materializes the
    // operational regions and leaves history and coverage by reference,
    // so a plain export dies on the first absent block. Reach for the
    // rest rather than skipping it -- `sparse` would succeed by silently
    // omitting whatever this replica happens not to hold, and a snapshot
    // with invisible holes is worse than one that fails loudly.
    //
    // With no remote configured there is nothing to reach for; sparse is
    // then the honest answer.
    let export = repository.snapshot(revision).export();
    let upstream = match (
        super::identity::local_root(tonk_state).await,
        super::account_state::account_remote(tonk_state).await,
    ) {
        (Ok(root), Ok(remote)) => tonk_account::peer::connect(
            dialog_repository::SiteAddress::from(dialog_remote_ucan::UcanAddress::new(
                remote.as_str(),
            )),
            root.root_did,
            &tonk_state.operator,
        )
        .await
        .ok(),
        _ => None,
    };
    let export = match upstream {
        Some(upstream) => export.download(upstream),
        None => export.sparse(),
    };

    dialog_repository::codec::encode(export.perform(&tonk_state.operator), vec![root])
        .await
        .map_err(|e| TonkWorkerError::Internal(e.to_string()))
}

/// Wrap snapshot bytes as a downloadable response.
fn snapshot_response(filename: &str, body: Vec<u8>) -> Response {
    let mut response = (StatusCode::OK, Body::from(body)).into_response();
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, SNAPSHOT_MEDIA_TYPE.parse().unwrap());
    if let Ok(value) = format!("attachment; filename=\"{filename}\"").parse::<header::HeaderValue>()
    {
        headers.insert(header::CONTENT_DISPOSITION, value);
    }
    response
}

/// Path parameters for the profile export route. The profile is a
/// singleton -- no `repo` segment.
#[derive(Debug, Deserialize)]
pub struct ProfileExportPath {
    /// The branch name.
    pub branch: String,
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
#[handler]
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
    // The import commits under the branch's write lock, so a read guard is
    // enough.
    let tonk_state = state.read().await;
    let tonk_branch = tonk_state
        .reactor
        .repository(&path.repo)
        .branch(&path.branch);

    // The import publishes against the head it read, and nothing holds
    // other writers of the branch off meanwhile, so one that loses the
    // head's race reads the head again and imports onto it.
    const IMPORT_ATTEMPTS: usize = 4;
    let mut attempt = 0;
    let revision = loop {
        attempt += 1;
        let importer = CsvImporter::from(Cursor::new(body.to_vec()));
        match tonk_branch
            .import(importer)
            .perform(&tonk_state.operator)
            .await
        {
            Ok(revision) => break revision,
            Err(dialog_reactor::ImportError::Commit(dialog_repository::CommitError::Publish(
                dialog_repository::PublishError::VersionMismatch { .. },
            ))) if attempt < IMPORT_ATTEMPTS => {
                let session = tonk_branch
                    .acquire(&tonk_state.operator)
                    .await
                    .map_err(|e| TonkWorkerError::Router(e.to_string()))?;
                session
                    .handle()
                    .refresh(&tonk_state.operator)
                    .await
                    .map_err(|e| TonkWorkerError::Router(e.to_string()))?;
            }
            Err(e) => return Err(TonkWorkerError::Router(e.to_string())),
        }
    };

    // Announce the moved head on the branch channel so subscribed
    // UIs refresh their revision/sync badges, the same as
    // `evaluate`. The reactor already re-polled SSE subscriptions.

    Ok(axum::Json(ImportResponse { revision }))
}
