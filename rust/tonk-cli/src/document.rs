//! Automerge documents in the CLI.
//!
//! The behaviour is the host-neutral `tonk-document` crate, the same
//! code the service worker runs; this module only decides WHEN the CLI
//! calls it, so an agent gets the same result from `tonk assert
//! document/replace` and `tonk query` as a page gets from
//! `tonk.transact` and `tonk.query`:
//!
//! - **commands** run right after the write commits, from the transient
//!   facts the evaluator already hands back. The worker does this in its
//!   command dispatcher; the CLI has none, so it dispatches here.
//! - **the mirror** is built before an evaluation. The worker keeps it
//!   up to date as documents change; a CLI process is short-lived, so it
//!   derives it on demand.
//! - **the sync pass** runs inside auto-sync, after the branch pull and
//!   after the branch push.
//! - **history formulas** answer `tonk query document/<formula>`.

use dialog_artifacts::Changes;
use dialog_repository::Branch;
use tonk_document::engine::Stamp;
use tonk_document::{command, formula, session};

use crate::site::TonkSite;

/// The writer's stamp: this profile, now.
fn stamp(site: &TonkSite) -> Stamp {
    let time = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs() as i64);
    Stamp {
        author: Some(site.profile.did().to_string()),
        time,
    }
}

/// Derive the mirror facts of every document on `branch`, so a query in
/// the evaluation that follows can read document text and table cells.
/// Best effort: a document whose bytes have not arrived yet is skipped.
pub async fn mirror_all(site: &TonkSite, branch: &Branch) {
    let Ok(documents) = session::documents(branch, &site.operator).await else {
        return;
    };
    for (entity, _) in documents {
        let _ = session::mirror(branch, &entity, &site.operator).await;
    }
}

/// Run every document command among `transients`. Returns one line per
/// refused command; a refused command changed nothing.
pub async fn dispatch(site: &TonkSite, branch: &Branch, transients: &Changes) -> Vec<String> {
    let mut refused = Vec::new();
    for request in command::requests(transients) {
        if let Err(error) = command::run(branch, &site.operator, &stamp(site), &request).await {
            refused.push(format!("{}: {error}", request.document));
        }
    }
    refused
}

/// Run the sync pass for every document of the site's branch. Failures
/// are warnings: the local state is already durable.
pub async fn sync_all(site: &TonkSite) {
    let Ok(session) = site.branch().await else {
        return;
    };
    let branch = session.handle();
    let Ok(documents) = session::documents(branch, &site.operator).await else {
        return;
    };
    for (entity, _) in documents {
        if let Err(error) = session::sync(branch, &entity, &site.operator).await {
            eprintln!("warning: document {entity} did not sync: {error}");
        }
    }
}

/// Answer a `document/*` history formula: the same rows the worker's
/// `/query` route returns, as pretty JSON.
pub async fn query_formula(
    site: &TonkSite,
    name: &str,
    terms: &[(String, String)],
) -> Result<String, String> {
    let terms: serde_json::Map<String, serde_json::Value> = terms
        .iter()
        .map(|(key, value)| (key.clone(), serde_json::Value::String(value.clone())))
        .collect();
    let query: dialog_reactor::Query =
        serde_json::from_value(serde_json::json!({ "predicate": name, "terms": terms }))
            .map_err(|error| format!("bad formula query: {error}"))?;
    let session = site
        .branch()
        .await
        .map_err(|error| format!("acquire branch: {error}"))?;
    let rows = formula::resolve(session.handle(), &site.operator, &query)
        .await
        .map_err(|error| error.to_string())?;
    let mut out = serde_json::to_string_pretty(&rows).map_err(|error| error.to_string())?;
    out.push('\n');
    Ok(out)
}
