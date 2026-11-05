//! Account-bound email delivery, driven by profile commands and overlay outcomes.
use super::{CommandEnv, identity::LocalRoot};
use crate::TonkWorkerError;
use dialog_capability::Provider;
use dialog_ucan_core::{InvocationBuilder, InvocationChain, promise::Promised, time::Timestamp};
use dialog_varsig::Did;
use serde_json::Value;
use std::collections::BTreeMap;
use tonk_schema::command::{ClaimEmailInvitation, EmailInvitationStatus, SendEmailInvitation};
use tonk_schema::domain::email_invitation_status::{Message, State};
use tonk_schema::invocation::{Done, Failure, Outcome};
use tonk_schema::prelude::DidExt as _;
use url::Url;

/// The refusal for a command fired anywhere but the profile, which is the
/// only place an account's mail may be sent from.
fn not_from_profile() -> Outcome {
    Err(Failure::new(
        "email invitations are sent from the profile, not from a space",
    ))
}

fn unavailable() -> TonkWorkerError {
    TonkWorkerError::Conflict("Delivery could not be confirmed. Retry the same invitation.".into())
}

async fn origin(tonk: &crate::TonkState) -> Result<Url, TonkWorkerError> {
    // A browser link belongs to its own deployment; native workers use the
    // selected account's attached provider. Never accept an arbitrary mail URL.
    if let Some(origin) = super::repository::worker_origin() {
        return format!("{origin}/").parse().map_err(|_| unavailable());
    }
    super::customer::home_service_origin(tonk).await
}

async fn call(
    tonk: &crate::TonkState,
    root: &LocalRoot,
    origin: &Url,
    op: &str,
    arguments: BTreeMap<String, Promised>,
) -> Result<Value, TonkWorkerError> {
    let current = super::identity::local_root(tonk).await?;
    if current.root_did != root.root_did || current.bytes != root.bytes {
        return Err(TonkWorkerError::Forbidden(
            "The selected account changed. Try again.".into(),
        ));
    }
    let response = super::http::get(&origin.join(".well-known/tonk").map_err(|_| unavailable())?)
        .await
        .map_err(|_| unavailable())?;
    let config: tonk_worker_api::DeploymentConfig =
        serde_json::from_slice(&response.body).map_err(|_| unavailable())?;
    let service: Did = config
        .service_did
        .ok_or_else(unavailable)?
        .parse()
        .map_err(|_| unavailable())?;
    let invocation = InvocationBuilder::new()
        .issuer(tonk.profile.credential().signer().clone())
        .audience(&service)
        .subject(&root.root_did)
        .command(vec!["email-invitation".into(), op.into()])
        .arguments(arguments)
        .proofs(root.delegation.proof_cids().to_vec())
        .expiration(Timestamp::five_minutes_from_now())
        .try_build()
        .await
        .map_err(|_| unavailable())?;
    let bytes = InvocationChain::new(invocation, root.delegation.export().collect())
        .to_bytes()
        .map_err(|_| unavailable())?;
    let response = super::http::post_cbor(&super::customer::ucan_endpoint(origin)?, &bytes)
        .await
        .map_err(|error| {
            // Sending and cancelling are the sender's; claiming is the recipient's.
            let sender = op != "claim";
            let message = match error {
                super::http::HttpError::Upstream(failure) => match failure.status {
                    403 if sender => {
                        "This space can't be shared by email from this account right now. Check that it is syncing, then try again."
                    }
                    403 => tonk_worker_api::email_invitation::WRONG_ACCOUNT,
                    409 if sender => {
                        "This address already has a Tonk account, or the invitation changed. Look up the address again."
                    }
                    409 => tonk_worker_api::email_invitation::UNCONFIRMED,
                    410 => tonk_worker_api::email_invitation::GONE,
                    429 => {
                        "Delivery is limited. Wait a minute before retrying the same invitation."
                    }
                    _ => "Delivery could not be confirmed. Retry the same invitation.",
                },
                _ => "Delivery could not be confirmed. Retry the same invitation.",
            };
            TonkWorkerError::Conflict(message.into())
        })?;
    serde_json::from_slice(&response.body).map_err(|_| unavailable())
}

async fn account(env: &CommandEnv, expected: &str) -> Result<LocalRoot, TonkWorkerError> {
    let tonk = env.state().read().await;
    let root = super::identity::local_root(&tonk).await?;
    if !env.from_profile()
        || env.origin().branch != tonk.active_branch
        || root.root_did.as_str() != expected
    {
        return Err(TonkWorkerError::Forbidden(
            "The selected account changed. Try again.".into(),
        ));
    }
    Ok(root)
}

async fn publish(env: &CommandEnv, this: &dialog_artifacts::Entity, state: &str, message: &str) {
    let tonk = env.state().read().await;
    let _ = tonk
        .reactor
        .profile_repository()
        .branch(&env.origin().branch)
        .overlay()
        .assert(EmailInvitationStatus {
            this: this.clone(),
            state: State(state.into()),
            message: Message(message.into()),
        })
        .write()
        .perform(&tonk.operator)
        .await;
}

async fn send(env: &CommandEnv, command: &SendEmailInvitation) -> Result<(), TonkWorkerError> {
    let id = &command.id.0;
    if !tonk_invite::delivery::valid_id(id) {
        return Err(unavailable());
    }
    let root = account(env, &command.account.0.to_string()).await?;
    let tonk = env.state().read().await;
    let origin = origin(&tonk).await?;
    let mut args = BTreeMap::from([
        ("invitation".into(), Promised::String(id.clone())),
        ("email".into(), Promised::String(command.email.0.clone())),
        (
            "space".into(),
            Promised::String(command.space.0.to_string()),
        ),
    ]);
    let prepared = call(&tonk, &root, &origin, "prepare", args.clone()).await?;
    drop(tonk);
    // The service retains the exact original before attempting delivery. A
    // lost acknowledgement or process restart therefore retries the same grant.
    let original = if let Some(original) = prepared["original"].as_str() {
        original.to_owned()
    } else {
        let audience: Did = prepared["recipientRoot"]
            .as_str()
            .ok_or_else(unavailable)?
            .parse()
            .map_err(|_| unavailable())?;
        let request = tonk_worker_api::CreateInviteRequest {
            recipient_root: Some(audience),
            ..Default::default()
        };
        let request_origin =
            crate::axum::RequestOrigin::parse(origin.as_str()).map_err(|_| unavailable())?;
        let axum::Json(minted) = super::create_invite::mint_invite(
            env.state().clone(),
            command.space.0.to_string(),
            request_origin,
            request,
            Some(&root),
            Some(id),
        )
        .await?;
        let invite = tonk_invite::Invite::parse_url(minted.url().as_str())
            .await
            .map_err(|_| unavailable())?;
        bs58::encode(invite.chain.to_bytes().map_err(|_| unavailable())?).into_string()
    };
    account(env, root.root_did.as_str()).await?;
    args.clear();
    args.insert("invitation".into(), Promised::String(id.clone()));
    args.insert("original".into(), Promised::String(original));
    let tonk = env.state().read().await;
    let answer = call(&tonk, &root, &origin, "send", args).await?;
    if answer["state"] != "sent" {
        return Err(unavailable());
    }
    Ok(())
}

#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl Provider<SendEmailInvitation> for CommandEnv {
    async fn execute(&self, command: SendEmailInvitation) -> Outcome {
        if !self.from_profile() {
            return not_from_profile();
        }
        publish(self, &command.this, "pending", "Sending invitation…").await;
        match send(self, &command).await {
            Ok(()) => {
                publish(self, &command.this, "sent", "Invitation sent. They can create an account, confirm their email, and join this space.").await;
                Done::with(command.space.0.clone())
            }
            Err(error) => {
                publish(self, &command.this, "failed", &error.to_string()).await;
                Err(Failure::new(error))
            }
        }
    }
}

#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl Provider<ClaimEmailInvitation> for CommandEnv {
    async fn execute(&self, command: ClaimEmailInvitation) -> Outcome {
        if !self.from_profile() {
            return not_from_profile();
        }
        publish(self, &command.this, "pending", "Checking your invitation…").await;
        let result = async {
            if !tonk_invite::delivery::valid_id(&command.id.0) {
                return Err(unavailable());
            }
            let root = account(self, &command.account.0.to_string()).await?;
            let tonk = self.state().read().await;
            let origin = origin(&tonk).await?;
            let answer = call(
                &tonk,
                &root,
                &origin,
                "claim",
                BTreeMap::from([("invitation".into(), Promised::String(command.id.0.clone()))]),
            )
            .await?;
            let envelope: tonk_invite::delivery::DeliveryEnvelope =
                serde_json::from_value(answer).map_err(|_| unavailable())?;
            let invite = envelope
                .into_invite(&root.root_did, &command.id.0)
                .await
                .map_err(|_| unavailable())?;
            let url = invite
                .to_url(origin.join("join").map_err(|_| unavailable())?.as_str())
                .map_err(|_| unavailable())?;
            // Keep the account read lock through import. A profile switch cannot
            // install a completion into a different account between these steps.
            let outcome = super::join::join_invite(&tonk, &url)
                .await
                .map_err(TonkWorkerError::from)?;
            Ok::<_, TonkWorkerError>(outcome)
        }
        .await;
        match result {
            Ok(outcome) => {
                publish(self, &command.this, "joined", "Space joined.").await;
                super::navigate::notify_navigate(self.client(), &format!("/space/{}", outcome.key));
                Done::with(outcome.subject.this())
            }
            Err(error) => {
                publish(self, &command.this, "failed", &error.to_string()).await;
                Err(Failure::new(error))
            }
        }
    }
}

#[cfg(all(test, not(all(target_arch = "wasm32", target_os = "unknown"))))]
mod tests {
    use super::*;

    fn command(id: &str) -> SendEmailInvitation {
        SendEmailInvitation {
            this: "id:send-email-invitation".parse().unwrap(),
            id: tonk_schema::domain::send_email_invitation::Id(id.into()),
            email: tonk_schema::domain::send_email_invitation::Email("friend@example.com".into()),
            space: tonk_schema::domain::send_email_invitation::Space(
                "did:key:zSpace".parse().unwrap(),
            ),
            account: tonk_schema::domain::send_email_invitation::Account(
                "did:key:zAccount".parse().unwrap(),
            ),
        }
    }

    /// An invitation id the service could never have issued fails the
    /// send before any account is asked to sign it.
    #[dialog_common::test]
    async fn it_fails_an_invitation_with_an_unusable_id() {
        let state = crate::router::command::tests::native::test_state().await;
        let env = CommandEnv::new(state, crate::router::CommandOrigin::default());
        let outcome = Provider::<SendEmailInvitation>::execute(&env, command("")).await;
        assert!(
            matches!(&outcome, Err(failure) if failure.0.contains("Delivery could not be confirmed")),
            "{outcome:?}"
        );
    }

    /// Mail goes out from the profile only. The same command fired in a
    /// space is refused, and publishes no invitation status.
    #[dialog_common::test]
    async fn it_refuses_an_invitation_fired_in_a_space() {
        let state = crate::router::command::tests::native::test_state().await;
        let branch = state.read().await.active_branch.clone();
        let env = CommandEnv::new(
            state.clone(),
            crate::router::CommandOrigin {
                repo: "did:key:zSomeSpace".into(),
                branch: branch.clone(),
                client: None,
            },
        );
        let outcome = Provider::<SendEmailInvitation>::execute(&env, command("")).await;
        assert!(
            matches!(&outcome, Err(failure) if failure.0.contains("from the profile")),
            "{outcome:?}"
        );

        let tonk = state.read().await;
        let session = tonk
            .reactor
            .profile_repository()
            .branch(&branch)
            .acquire(&tonk.operator)
            .await
            .unwrap();
        let published = session
            .handle()
            .overlay()
            .export()
            .iter()
            .any(|(_, attribute, _)| {
                attribute
                    .to_string()
                    .starts_with("xyz.tonk.email-invitation-status/")
            });
        assert!(!published, "a refused invitation must publish no status");
    }
}
