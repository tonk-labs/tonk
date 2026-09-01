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

/// An [`crate::email::EmailSender`] that may not be configured: `Ready`
/// delegates to Resend, `Missing` fails at the SEND with the reason.
///
/// Only enrollment and resend mail anything, so an environment without
/// email secrets — every preview lacked them at some point — still
/// answers activation, provisioning, and the operator commands, instead
/// of refusing every registration invocation up front over a credential
/// most of them never use.
#[cfg(target_arch = "wasm32")]
enum MaybeEmail {
    /// Configured; sends deliver.
    Ready(crate::email::Resend),
    /// Not configured; sends fail with this explanation.
    Missing(String),
}

#[cfg(target_arch = "wasm32")]
#[async_trait::async_trait(?Send)]
impl crate::email::EmailSender for MaybeEmail {
    async fn send_activation(
        &self,
        email: &str,
        link: &str,
    ) -> Result<(), crate::email::EmailError> {
        match self {
            Self::Ready(resend) => resend.send_activation(email, link).await,
            Self::Missing(reason) => Err(crate::email::EmailError::Send(reason.clone())),
        }
    }
}

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
    let service_seed = env
        .secret("SERVICE_SECRET_KEY")
        .map_err(|err| internal(format!("SERVICE_SECRET_KEY: {err}")))?
        .to_string();
    let service = signer_from_hex(&service_seed)
        .map_err(|message| RegistrationError::Internal { message })?;
    // Missing email credentials fail at the SEND, not up front: only
    // enrollment and resend mail anything, and refusing activation over
    // an email secret it never uses stranded every preview environment
    // that lacked one.
    let email = match (env.secret("RESEND_API_KEY"), env.var("EMAIL_FROM")) {
        (Ok(api_key), Ok(from)) => {
            MaybeEmail::Ready(Resend::new(api_key.to_string(), from.to_string()))
        }
        (api_key, from) => MaybeEmail::Missing(format!(
            "email delivery is not configured: RESEND_API_KEY {}, EMAIL_FROM {}",
            if api_key.is_ok() { "set" } else { "missing" },
            if from.is_ok() { "set" } else { "missing" },
        )),
    };

    let url = req
        .url()
        .map_err(|err| internal(format!("request url: {err}")))?;
    let origin = url.origin().ascii_serialization();
    let activation_ttl = env
        .var("EMAIL_TOKEN_TTL")
        .ok()
        .and_then(|value| value.to_string().parse().ok())
        .unwrap_or(DEFAULT_EMAIL_TOKEN_TTL);

    // Bound per request, like the presign path's: the index wraps a KV
    // handle taken from this request's `Env`, which is not ours to keep.
    let revocations = {
        use crate::revocation::{checker::IndexedRevocations, index::kv::KvRevocationIndex};
        let kv = env
            .kv("REVOCATIONS_KV")
            .map_err(|err| internal(format!("REVOCATIONS_KV: {err}")))?;
        IndexedRevocations(KvRevocationIndex::new(kv))
    };

    // The same authorizer the presign path runs. Enrollment redeems the
    // recovery invocation through it, so the custody cell is written to
    // exactly the object that invocation names.
    let vault = crate::vault::AuthorizedVault(WorkerRedeemer(
        super::ucan::create_authorizer(env).map_err(|refusal| internal(format!("{refusal:?}")))?,
    ));

    let registration = Registration {
        store: &store,
        email: &email,
        vault: &vault,
        service: &service,
        service_seed: &service_seed,
        origin: &origin,
        activation_ttl,
        now: Date::now().as_millis() / 1_000,
        container: body,
        revocations: &revocations,
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
pub async fn handle_customer(req: Request, ctx: RouteContext<()>) -> worker::Result<Response> {
    use tonk_account::customer::{CustomerStatus, Receipt};

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
                customer: customer.account.parse().map_err(|err| {
                    worker::Error::RustError(format!("stored customer did is malformed: {err:?}"))
                })?,
                status: customer.status,
                ledger: None,
                // Only for a customer this service actually serves. The
                // probe is what notices activation, so it must answer
                // the address then — but naming one for a customer
                // still awaiting email confirmation would let a client
                // record an endpoint that refuses every presign.
                provider: (customer.status == CustomerStatus::Active)
                    .then(|| {
                        req.url()
                            .ok()
                            .map(|url| format!("{}/ucan/", url.origin().ascii_serialization()))
                    })
                    .flatten(),
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
    // The request's own origin: on the Worker this is the deployment the
    // browser actually reached, which is what a client should sync to.
    let origin = req.url()?.origin().ascii_serialization();
    let document = did_document(&host, &origin, &signer);
    // Pretty-printed: a DID document is something people open and read,
    // and `from_json` would emit it on one line.
    let body = serde_json::to_string_pretty(&document)
        .map_err(|error| worker::Error::RustError(error.to_string()))?;
    Response::ok(body).map(|response| {
        response.with_headers({
            let headers = worker::Headers::new();
            let _ = headers.set("content-type", "application/json");
            headers
        })
    })
}

/// The worker's [`Redeemer`]: its own authorizer, asked directly rather
/// than over HTTP.
#[cfg(target_arch = "wasm32")]
struct WorkerRedeemer(dialog_remote_ucan_s3::UcanAuthorizer);

#[cfg(target_arch = "wasm32")]
#[async_trait::async_trait(?Send)]
impl crate::vault::Redeemer for WorkerRedeemer {
    async fn redeem(
        &self,
        container: &[u8],
    ) -> Result<dialog_remote_s3::Permit, crate::vault::VaultError> {
        self.0
            .authorize(container)
            .await
            .map_err(|error| crate::vault::VaultError::Unavailable(error.to_string()))
    }
}
