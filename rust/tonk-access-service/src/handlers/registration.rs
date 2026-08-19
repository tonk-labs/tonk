//! Worker-side glue for the registration commands.
//!
//! The logic lives in [`crate::registration`], generic over storage and
//! email delivery; this module binds it to D1, Resend, and the wrangler
//! configuration, and shapes the HTTP answers.

use dialog_credentials::Ed25519Signer;
use tonk_account::customer::RegistrationError;
use worker::{Env, Request, Response, RouteContext, console_error};

use crate::service::{did_document, signer_from_hex};

/// Lifetime of the emailed activation delegation when the
/// `EMAIL_TOKEN_TTL` variable is unset, in seconds.
#[cfg(target_arch = "wasm32")]
const DEFAULT_EMAIL_TOKEN_TTL: u64 = 24 * 60 * 60;

/// Answer a registration invocation.
#[cfg(target_arch = "wasm32")]
pub async fn handle(body: &[u8], req: &Request, env: &Env) -> worker::Result<Response> {
    match handle_inner(body, req, env).await {
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
    req: &Request,
    env: &Env,
) -> Result<crate::registration::Answer, RegistrationError> {
    use worker::Date;

    use crate::email::Resend;
    use crate::registration::Registration;
    use crate::store::d1::D1Store;

    let store = D1Store::new(
        env.d1("CONTROL")
            .map_err(|err| internal(format!("control database: {err}")))?,
    );
    let service = service_signer(env)?;
    let api_key = env
        .secret("RESEND_API_KEY")
        .map_err(|err| internal(format!("RESEND_API_KEY: {err}")))?
        .to_string();
    let from = env
        .var("EMAIL_FROM")
        .map_err(|err| internal(format!("EMAIL_FROM: {err}")))?
        .to_string();
    let email = Resend::new(api_key, from);

    let url = req
        .url()
        .map_err(|err| internal(format!("request url: {err}")))?;
    let origin = url.origin().ascii_serialization();
    let activation_ttl = env
        .var("EMAIL_TOKEN_TTL")
        .ok()
        .and_then(|value| value.to_string().parse().ok())
        .unwrap_or(DEFAULT_EMAIL_TOKEN_TTL);

    let registration = Registration {
        store: &store,
        email: &email,
        service: &service,
        origin: &origin,
        activation_ttl,
        now: Date::now().as_millis() / 1_000,
        container: body,
    };
    registration.handle().await
}

/// Wrap a configuration failure as an internal refusal.
fn internal(message: String) -> RegistrationError {
    RegistrationError::Internal { message }
}

/// The service's signing identity, from the `SERVICE_SECRET_KEY` secret.
fn service_signer(env: &Env) -> Result<Ed25519Signer, RegistrationError> {
    let seed = env
        .secret("SERVICE_SECRET_KEY")
        .map_err(|err| internal(format!("SERVICE_SECRET_KEY: {err}")))?
        .to_string();
    signer_from_hex(&seed).map_err(|message| RegistrationError::Internal { message })
}

/// GET `/customer/:did` → the customer's registration state. This is
/// what the enrolling client polls to notice activation, and how it
/// decides whether a wiped or fresh service needs a (re-)enrollment.
///
/// The route is registered by the worker entrypoint, which also compiles
/// natively, so the signature exists on both targets; only the worker
/// body does, since it reads D1. The native twin lives in the helpers
/// server.
#[cfg(not(target_arch = "wasm32"))]
pub async fn handle_customer(_req: Request, _ctx: RouteContext<()>) -> worker::Result<Response> {
    Response::error("Not Found", 404)
}

/// GET `/customer/:did` → the customer's registration state (worker
/// body; see the native twin above).
#[cfg(target_arch = "wasm32")]
pub async fn handle_customer(_req: Request, ctx: RouteContext<()>) -> worker::Result<Response> {
    use tonk_account::customer::Receipt;

    use crate::store::Store;
    use crate::store::d1::D1Store;

    let Some(did) = ctx.param("did") else {
        return Response::error("Not Found", 404);
    };
    let store = match ctx.env.d1("CONTROL") {
        Ok(database) => D1Store::new(database),
        Err(err) => {
            worker::console_error!("customer probe unavailable, no CONTROL binding: {err}");
            return Response::error("Customer registry is not configured", 500);
        }
    };
    match store.customer(did).await {
        Ok(Some(customer)) => {
            let receipt = Receipt {
                customer: customer.did.parse().map_err(|err| {
                    worker::Error::RustError(format!("stored customer did is malformed: {err:?}"))
                })?,
                status: customer.status,
            };
            Response::from_json(&receipt)
        }
        Ok(None) => {
            let refusal = RegistrationError::UnknownCustomer;
            let response = Response::from_json(&serde_json::json!({ "error": refusal }))?;
            Ok(response.with_status(refusal.status()))
        }
        Err(err) => {
            worker::console_error!("customer probe failed: {err}");
            Response::error("Customer registry is unavailable", 500)
        }
    }
}

/// GET `/.well-known/did.json` → the service's DID document.
pub async fn handle_did_document(req: Request, ctx: RouteContext<()>) -> worker::Result<Response> {
    let signer = match service_signer(&ctx.env) {
        Ok(signer) => signer,
        Err(err) => {
            console_error!("did document unavailable: {err}");
            return Response::error("Service identity is not configured", 404);
        }
    };
    let host = req
        .url()?
        .host_str()
        .map(ToString::to_string)
        .unwrap_or_default();
    Response::from_json(&did_document(&host, &signer))
}
