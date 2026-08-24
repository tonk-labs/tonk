//! Worker glue for `/ucan/revoke`.
//!
//! The decision lives in [`crate::revoke`], generic over the control
//! store and the revocation index; this binds it to D1 and KV and
//! shapes the HTTP answer.

#[cfg(target_arch = "wasm32")]
use tonk_account::customer::RegistrationError;
#[cfg(target_arch = "wasm32")]
use worker::{Env, Response};

/// Answer a revocation invocation.
///
/// Worker-only: it binds D1 and KV, neither of which exists natively.
/// The native server reaches [`crate::revoke::revoke`] directly.
#[cfg(target_arch = "wasm32")]
pub async fn handle(body: &[u8], env: &Env) -> worker::Result<Response> {
    match handle_inner(body, env).await {
        Ok(receipt) => Response::from_json(&receipt),
        Err(err) => {
            let response = Response::from_json(&serde_json::json!({ "error": err }))?;
            Ok(response.with_status(err.status()))
        }
    }
}

#[cfg(target_arch = "wasm32")]
async fn handle_inner(
    body: &[u8],
    env: &Env,
) -> Result<tonk_account::customer::RevokeReceipt, RegistrationError> {
    use crate::revocation::index::kv::KvRevocationIndex;
    use crate::store::d1::D1Store;

    let store = D1Store::new(
        env.d1("CONTROL")
            .map_err(|err| RegistrationError::Internal {
                message: format!("control database: {err}"),
            })?,
    );
    let index = KvRevocationIndex::new(env.kv("REVOCATIONS_KV").map_err(|err| {
        RegistrationError::Internal {
            message: format!("revocation index: {err}"),
        }
    })?);
    crate::revoke::revoke(&store, &index, body).await
}
