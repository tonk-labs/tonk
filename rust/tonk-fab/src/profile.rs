//! What the bar asks of the profile, over HTTP.
//!
//! The bar renders on the profile's origin, whose worker holds the profile:
//! a claim is a `POST` to that branch's `/transact`, a read a `POST` to its
//! `/query`. The branch is the one the bar's own `with` names.

use serde_json::Value;
use tonk_host::error::ErrorDetail;
use tonk_host::location::Location;
use tonk_host::{post_json, route_of};
use wasm_bindgen_futures::spawn_local;
use web_sys::window;

const DEFAULT_BRANCH: &str = "main";

/// The profile branch the bar in this document is rendered against.
fn branch() -> String {
    window()
        .and_then(|win| win.document())
        .and_then(|document| document.query_selector("tonk-fab[with]").ok().flatten())
        .and_then(|bar| bar.get_attribute("with"))
        .and_then(|with| with.parse::<Location>().ok())
        .and_then(|location| route_of(&location).1)
        .unwrap_or_else(|| DEFAULT_BRANCH.into())
}

fn endpoint(operation: &str) -> String {
    format!(
        "/api/repository/profile:tonk/branch/{}/{operation}",
        branch()
    )
}

/// Claim on the profile branch and say how it went.
pub(crate) async fn claim(request: &Value) -> Result<String, ErrorDetail> {
    post_json(&endpoint("transact"), &request.to_string()).await
}

/// Claim on the profile branch without waiting for the answer.
pub(crate) fn transact(request: &Value) {
    let request = request.clone();
    spawn_local(async move {
        if let Err(error) = claim(&request).await {
            tonk_common::log!("tonk-fab: profile claim failed: {}", error.message);
        }
    });
}

/// Read conclusions from the profile branch. `None` when the read fails.
pub(crate) async fn query(body: &Value) -> Option<Value> {
    let answer = post_json(&endpoint("query"), &body.to_string())
        .await
        .ok()?;
    serde_json::from_str(&answer).ok()
}
