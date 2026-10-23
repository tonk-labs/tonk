//! Bind authenticated email delivery to the existing control database and email transport.

#[cfg(target_arch = "wasm32")]
use crate::invitation::{InvitationEmailSender, InvitationError, InvitationService};
#[cfg(target_arch = "wasm32")]
use worker::{Env, Request, Response};

#[cfg(target_arch = "wasm32")]
struct OptionalEmail(Option<crate::email::Resend>);
#[cfg(target_arch = "wasm32")]
#[async_trait::async_trait(?Send)]
impl InvitationEmailSender for OptionalEmail {
    async fn send_invitation(
        &self,
        email: &str,
        link: &str,
        id: &str,
    ) -> Result<(), InvitationError> {
        self.0
            .as_ref()
            .ok_or(InvitationError::Unavailable)?
            .send_invitation(email, link, id)
            .await
    }
}

/// Dispatch from `/ucan/`, preserving its body limit and CORS handling.
#[cfg(target_arch = "wasm32")]
pub async fn handle(body: &[u8], req: &Request, env: &Env) -> worker::Result<Response> {
    let answer = inner(body, req, env).await;
    match answer {
        Ok(value) => Response::from_json(&value),
        Err(error) => Ok(
            Response::from_json(&serde_json::json!({"error":error.to_string()}))?
                .with_status(error.status()),
        ),
    }
}

#[cfg(target_arch = "wasm32")]
async fn inner(
    body: &[u8],
    req: &Request,
    env: &Env,
) -> Result<serde_json::Value, InvitationError> {
    use crate::revocation::{checker::IndexedRevocations, index::kv::KvRevocationIndex};
    let store = crate::store::d1::D1Store::new(
        env.d1("CONTROL")
            .map_err(|_| InvitationError::Unavailable)?,
    );
    let seed = crate::secret::secret(env, "SERVICE_SECRET_KEY")
        .map_err(|_| InvitationError::Unavailable)?;
    let service =
        crate::service::signer_from_hex(&seed).map_err(|_| InvitationError::Unavailable)?;
    let email = OptionalEmail(
        match (
            crate::secret::secret(env, "RESEND_API_KEY"),
            env.var("EMAIL_FROM"),
        ) {
            (Ok(key), Ok(from)) => Some(crate::email::Resend::new(key, from.to_string())),
            _ => None,
        },
    );
    let revocations = IndexedRevocations(KvRevocationIndex::new(
        env.kv("REVOCATIONS_KV")
            .map_err(|_| InvitationError::Unavailable)?,
    ));
    let origin = req
        .url()
        .map_err(|_| InvitationError::Invalid)?
        .origin()
        .ascii_serialization();
    InvitationService {
        store: &store,
        email: &email,
        service: &service,
        service_seed: &seed,
        origin: &origin,
        now: worker::Date::now().as_millis() / 1000,
        revocations: &revocations,
    }
    .handle(body)
    .await
}
