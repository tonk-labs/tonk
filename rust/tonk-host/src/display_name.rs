//! Result-bearing account display-name write for sealed guests.

use serde::Deserialize;

use crate::error::{ErrorDetail, ErrorKind};

#[derive(Deserialize)]
struct DisplayNameResponse {
    name: String,
}

/// Commit a display name through the worker's authoritative account endpoint.
///
/// The relative fetch uses the sealed guest's existing relay. A non-success
/// response remains an error so callers can restore their last subscribed
/// value instead of displaying a phantom rename.
pub async fn set_account_display_name(name: &str) -> Result<String, ErrorDetail> {
    let body = serde_json::to_string(&serde_json::json!({ "name": name }))
        .map_err(|error| ErrorDetail::new(ErrorKind::Parse, error.to_string()))?;
    let response = crate::http::post_json("/api/account/display-name", &body).await?;
    serde_json::from_str::<DisplayNameResponse>(&response)
        .map(|response| response.name)
        .map_err(|error| {
            ErrorDetail::new(
                ErrorKind::Parse,
                format!("parse account display-name response: {error}"),
            )
        })
}
