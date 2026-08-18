//! Worker-side glue for the registration commands.
//!
//! The logic lives in [`crate::registration`], generic over storage and
//! email delivery; this module binds it to D1, Resend, and the wrangler
//! configuration, and shapes the HTTP answers.

use dialog_credentials::Ed25519Signer;
use tonk_account::customer::RegistrationError;
use worker::{Request, Response, RouteContext, console_error};

use crate::service::{did_document, signer_from_hex};

/// Lifetime of the emailed activation delegation when the
/// `EMAIL_TOKEN_TTL` variable is unset, in seconds.
#[cfg(target_arch = "wasm32")]
const DEFAULT_EMAIL_TOKEN_TTL: u64 = 24 * 60 * 60;

/// Answer a registration invocation.
#[cfg(target_arch = "wasm32")]
pub async fn handle(
    body: &[u8],
    req: &Request,
    ctx: &RouteContext<()>,
) -> worker::Result<Response> {
    match handle_inner(body, req, ctx).await {
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
    ctx: &RouteContext<()>,
) -> Result<tonk_account::customer::Receipt, RegistrationError> {
    use worker::Date;

    use crate::email::Resend;
    use crate::registration::Registration;
    use crate::store::d1::D1Store;

    let store = D1Store::new(
        ctx.env
            .d1("CONTROL")
            .map_err(|err| internal(format!("control database: {err}")))?,
    );
    let service = service_signer(ctx)?;
    let api_key = ctx
        .secret("RESEND_API_KEY")
        .map_err(|err| internal(format!("RESEND_API_KEY: {err}")))?
        .to_string();
    let from = ctx
        .var("EMAIL_FROM")
        .map_err(|err| internal(format!("EMAIL_FROM: {err}")))?
        .to_string();
    let email = Resend::new(api_key, from);

    let url = req
        .url()
        .map_err(|err| internal(format!("request url: {err}")))?;
    let origin = url.origin().ascii_serialization();
    let activation_ttl = ctx
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
fn service_signer(ctx: &RouteContext<()>) -> Result<Ed25519Signer, RegistrationError> {
    let seed = ctx
        .secret("SERVICE_SECRET_KEY")
        .map_err(|err| internal(format!("SERVICE_SECRET_KEY: {err}")))?
        .to_string();
    signer_from_hex(&seed).map_err(|message| RegistrationError::Internal { message })
}

/// GET `/.well-known/did.json` → the service's DID document.
pub async fn handle_did_document(req: Request, ctx: RouteContext<()>) -> worker::Result<Response> {
    let signer = match service_signer(&ctx) {
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
