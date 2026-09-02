//! Worker-side glue for the registration commands.
//!
//! The logic lives in [`crate::registration`], generic over storage and
//! email delivery; this module binds it to D1, Resend, and the wrangler
//! configuration, and shapes the HTTP answers.

use dialog_credentials::Ed25519Signer;
use tonk_account::customer::RegistrationError;
use worker::{Env, Request, Response, RouteContext};

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
        Ok(receipt) => {
            replicate_verdicts(&receipt, env).await;
            Response::from_json(&receipt)
        }
        Err(err) => {
            use crate::observability::{
                AccessFailureKind, AccessFailureLog, AccessOperation, AccessOutcome, AccessSite,
            };
            use crate::registration::RegistrationCommand;
            let operation = match crate::registration::registration_command(body) {
                Some(RegistrationCommand::Enroll) => AccessOperation::Enrollment,
                Some(RegistrationCommand::Activate) => AccessOperation::Activation,
                Some(RegistrationCommand::Resend) => AccessOperation::Resend,
                Some(
                    RegistrationCommand::ProviderAdd
                    | RegistrationCommand::Suspend
                    | RegistrationCommand::Resume
                    | RegistrationCommand::Archive,
                )
                | None => AccessOperation::Provisioning,
            };
            let status = err.status();
            let failure_kind = match status {
                401 | 403 => AccessFailureKind::AccessDenied,
                404 => AccessFailureKind::NotFound,
                409 => AccessFailureKind::Conflict,
                429 => AccessFailureKind::RateLimited,
                500..=599 => AccessFailureKind::Unavailable,
                _ => AccessFailureKind::Invalid,
            };
            AccessFailureLog::new(
                operation,
                if status >= 500 {
                    AccessOutcome::Unavailable
                } else {
                    AccessOutcome::Refused
                },
                failure_kind,
                status,
                status >= 500 || status == 429,
                AccessSite::Registration,
            )
            .emit();
            let response = Response::from_json(&serde_json::json!({ "error": err }))?;
            Ok(response.with_status(err.status()))
        }
    }
}

/// Write the fresh servability verdicts for every subject a completed
/// registration command changed, so the change propagates to the
/// presign path as itself rather than waiting out a cached entry's
/// validity. `/provider/add` touches the provisioned consumer;
/// `/customer/enroll` and `/customer/activate` touch the customer and
/// every consumer it funds. Best-effort: a failed write costs at most
/// one stale validity window, which the verdicts are sized for.
#[cfg(target_arch = "wasm32")]
async fn replicate_verdicts(answer: &crate::registration::Answer, env: &Env) {
    use worker::Date;

    use crate::handlers::ucan::{derive_verdict, servability_kv};
    use crate::registration::Answer;
    use crate::store::Store;
    use crate::store::d1::D1Store;

    let customer = match answer {
        Answer::Subscription(receipt) => {
            let now = Date::now().as_millis() / 1_000;
            let _ = derive_verdict(
                receipt.consumer.as_str(),
                now,
                env,
                servability_kv(env).as_ref(),
            )
            .await;
            return;
        }
        Answer::Customer(receipt) => receipt.customer.as_str().to_string(),
        // The operator commands answer nothing; their consumer rides
        // out the cached verdict's validity instead.
        Answer::Done => return,
    };
    let kv = servability_kv(env);
    let now = Date::now().as_millis() / 1_000;
    let mut subjects = vec![customer.clone()];
    match env.d1("CONTROL") {
        Ok(control) => {
            let store = D1Store::new(control);
            match store.subscriptions_by_provider(&customer).await {
                Ok(subscriptions) => subjects.extend(
                    subscriptions
                        .into_iter()
                        .map(|subscription| subscription.consumer)
                        .filter(|consumer| *consumer != customer),
                ),
                Err(_) => worker::console_error!("consumer verdicts were not refreshed"),
            }
            // The row itself, for the probe and the email lookup: both
            // poll it, and the write-through is how a poll notices the
            // enrollment or activation that just happened.
            if let Some(kv) = &kv {
                match store.customer(&customer).await {
                    Ok(Some(row)) => crate::store::replica::replicate(kv, &row, now).await,
                    Ok(None) => {}
                    Err(_) => worker::console_error!("customer replica was not written"),
                }
            }
        }
        Err(_) => worker::console_error!("verdicts were not refreshed: CONTROL is unavailable"),
    }
    for subject in subjects {
        let _ = derive_verdict(&subject, now, env, kv.as_ref()).await;
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
    use crate::store::Store;
    use crate::store::d1::D1Store;
    use crate::store::replica;

    let Some(did) = ctx.param("did") else {
        return Response::error("Not Found", 404);
    };

    // This is a poll — the enrolling client watches it for activation —
    // so it reads the KV replica first and reaches D1 only on a miss,
    // backfilling what it finds. Enrollment and activation write the
    // replica through, which is how the poll notices them.
    let now = worker::Date::now().as_millis() / 1_000;
    let kv = super::ucan::servability_kv(&ctx.env);
    if let Some(kv) = &kv
        && let Some(cached) = replica::load(kv, &replica::did_key(did), now).await
    {
        return answer_probe(cached.customer, &req);
    }

    let store = match ctx.env.d1("CONTROL") {
        Ok(database) => D1Store::new(database),
        Err(_) => {
            crate::observability::AccessFailureLog::new(
                crate::observability::AccessOperation::CustomerProbe,
                crate::observability::AccessOutcome::Unavailable,
                crate::observability::AccessFailureKind::Unavailable,
                500,
                true,
                crate::observability::AccessSite::ControlStore,
            )
            .emit();
            return Response::error("Customer registry is not configured", 500);
        }
    };
    match store.customer(did).await {
        Ok(row) => {
            if let Some(kv) = &kv {
                replica::backfill(kv, &replica::did_key(did), row.clone(), now).await;
            }
            answer_probe(row, &req)
        }
        Err(_) => {
            crate::observability::AccessFailureLog::new(
                crate::observability::AccessOperation::CustomerProbe,
                crate::observability::AccessOutcome::Unavailable,
                crate::observability::AccessFailureKind::Unavailable,
                500,
                true,
                crate::observability::AccessSite::ControlStore,
            )
            .emit();
            Response::error("Customer registry is unavailable", 500)
        }
    }
}

/// The probe's answer for a customer row, or for its absence.
#[cfg(target_arch = "wasm32")]
fn answer_probe(
    customer: Option<crate::store::Customer>,
    req: &Request,
) -> worker::Result<Response> {
    use tonk_account::customer::{CustomerStatus, Receipt};

    match customer {
        Some(customer) => {
            let receipt = Receipt {
                customer: customer.account.parse().map_err(|_| {
                    worker::Error::RustError("stored customer DID is malformed".to_owned())
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
        None => {
            let refusal = RegistrationError::UnknownCustomer;
            let response = Response::from_json(&serde_json::json!({ "error": refusal }))?;
            Ok(response.with_status(refusal.status()))
        }
    }
}

/// GET `/.well-known/did.json` → the service's DID document.
pub async fn handle_did_document(req: Request, ctx: RouteContext<()>) -> worker::Result<Response> {
    let signer = match service_signer(&ctx.env) {
        Ok(signer) => signer,
        Err(_) => {
            crate::observability::AccessFailureLog::new(
                crate::observability::AccessOperation::Provisioning,
                crate::observability::AccessOutcome::Unavailable,
                crate::observability::AccessFailureKind::Unavailable,
                404,
                false,
                crate::observability::AccessSite::Entry,
            )
            .emit();
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
