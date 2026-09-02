//! Account deletion: a review read from the account db, one root-signed
//! purge presented to the access service, then local cleanup. The purge
//! is a command the hub asserts; the passkey that signs it is asked for
//! through the custody relay, and the worker signs with the root it
//! recovers.

use std::collections::BTreeSet;

use axum::{
    Json,
    extract::{Extension, State},
};
use axum_wasm_macros::wasm_compat;
use dialog_query::{Output as _, Query, Term};
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tonk_common::log;
use tonk_schema::SpaceProvider;
use tonk_schema::domain::space::Provider;
use tonk_schema::prelude::DidExt as _;
use tonk_worker_api::{
    AccountDeletionPlan, AccountDeletionSpace, AccountSpaceDeletionRequest,
    HostedSpaceDeletionResult,
};

use super::AppState;
use crate::TonkWorkerError;
use crate::axum::RequestOrigin;
use crate::worker::TonkState;

/// The destructive scope, from the account db alone: the directory is
/// the inventory, and a space this account provides carries a
/// `SpaceProvider` fact naming the account. Everything else listed is
/// joined and stays.
async fn load_plan(state: &TonkState) -> Result<AccountDeletionPlan, TonkWorkerError> {
    let link = super::account::account_link(state).await.ok_or_else(|| {
        TonkWorkerError::NotFound("this profile is not linked to an account".into())
    })?;
    let root = link.issuer().clone();
    let email = super::account_devices::account_summary(state)
        .await?
        .email
        .ok_or_else(|| TonkWorkerError::Conflict("the account has no verified email".into()))?;
    let main = state
        .reactor
        .profile_repository()
        .branch(tonk_account::MAIN_BRANCH)
        .acquire(&state.operator)
        .await
        .map_err(|error| {
            TonkWorkerError::Internal(format!("open the account directory: {error}"))
        })?;
    let directory = tonk_schema::directory::spaces(main.handle(), &state.operator)
        .await
        .map_err(|error| {
            TonkWorkerError::Internal(format!("account directory query failed: {error:?}"))
        })?;
    let provided: Vec<SpaceProvider> = main
        .handle()
        .query()
        .select(Query::<SpaceProvider> {
            this: Term::var("this"),
            provider: Term::from(Provider(root.this())),
        })
        .perform(&state.operator)
        .try_vec()
        .await
        .map_err(|error| {
            TonkWorkerError::Internal(format!("space provider query failed: {error:?}"))
        })?;
    let owned: BTreeSet<_> = provided.into_iter().map(|row| row.this).collect();
    let (spaces, joined): (Vec<_>, Vec<_>) = directory
        .into_iter()
        .partition(|space| owned.contains(&space.subject.this()));
    Ok(AccountDeletionPlan {
        root_did: root.to_string(),
        email,
        spaces: spaces
            .into_iter()
            .map(|space| AccountDeletionSpace {
                subject: space.subject.to_string(),
                name: space.name,
            })
            .collect(),
        joined_spaces: joined.len(),
    })
}

/// GET `/api/account/deletion/plan` returns the exact destructive scope.
#[wasm_compat]
pub async fn plan(
    State(state): State<AppState>,
) -> Result<Json<AccountDeletionPlan>, TonkWorkerError> {
    let state = state.read().await;
    Ok(Json(load_plan(&state).await?))
}

/// POST `/api/account/spaces/delete` deletes one reviewed owned hosted space
/// while leaving the account and every other space intact. The worker
/// signs the `/provider/remove` itself: deletion is the account ending
/// its hosting relationship, and this linked device holds that
/// authority.
#[wasm_compat]
pub async fn delete_space(
    State(state): State<AppState>,
    Extension(origin): Extension<RequestOrigin>,
    Json(request): Json<AccountSpaceDeletionRequest>,
) -> Result<Json<HostedSpaceDeletionResult>, TonkWorkerError> {
    let current = {
        let state = state.read().await;
        load_plan(&state).await?
    };
    // The lookup is the ownership check: a space this account does not
    // provide is not in its plan.
    current
        .spaces
        .iter()
        .find(|space| space.subject == request.subject)
        .ok_or_else(|| TonkWorkerError::Forbidden("space is not owned by this account".into()))?;
    let subject: dialog_varsig::Did = request.subject.parse().map_err(|error| {
        TonkWorkerError::Internal(format!("reviewed space DID became invalid: {error:?}"))
    })?;
    {
        let state = state.read().await;
        super::customer::deprovision_consumer(&state, origin.url(), &subject).await?;
    }

    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    super::repository::remove_space_inner(&state, &subject).await?;
    Ok(Json(HostedSpaceDeletionResult {
        subject: request.subject,
    }))
}

/// `tonk:delete-account`: check the retyped address, then ask the page
/// for the passkey. The purge itself runs in [`purge`] once the
/// handles arrive.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) struct DeleteAccountHandler {
    attributes: Vec<String>,
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
impl DeleteAccountHandler {
    pub(crate) fn new() -> Self {
        use crate::reactor::Decode as _;
        Self {
            attributes: tonk_schema::command::DeleteAccount::trigger_attributes(),
        }
    }
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn decode_deletion(facts: &crate::reactor::EntityFacts) -> Option<String> {
    use crate::reactor::Decode as _;
    facts
        .first()
        .map(|artifact| artifact.of.clone())
        .and_then(|entity| tonk_schema::command::DeleteAccount::decode(entity, facts))
        .map(|command| command.email.0)
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
impl crate::reactor::CommandHandler<crate::router::CommandEnv> for DeleteAccountHandler {
    fn trigger_attributes(&self) -> &[String] {
        &self.attributes
    }

    fn matches(&self, facts: &crate::reactor::EntityFacts) -> bool {
        decode_deletion(facts).is_some()
    }

    fn run(
        &self,
        facts: &crate::reactor::EntityFacts,
        env: &crate::router::CommandEnv,
    ) -> crate::reactor::RunFuture {
        use tonk_schema::{ceremony, ceremony_state};

        let email = decode_deletion(facts);
        let env = env.clone();
        Box::pin(async move {
            let Some(email) = email else {
                log!("delete-account: the transient carried no address; skipping");
                return;
            };
            log!(
                "delete-account: asked from client {:?}",
                env.client().map(|c| c.0.clone())
            );
            {
                let tonk = env.state().read().await;
                let plan = match load_plan(&tonk).await {
                    Ok(plan) => plan,
                    Err(error) => {
                        super::ceremony::report(
                            &tonk,
                            ceremony::DELETE_ACCOUNT,
                            ceremony_state::REFUSED,
                            &error.to_string(),
                        )
                        .await;
                        return;
                    }
                };
                if email.trim() != plan.email {
                    super::ceremony::report(
                        &tonk,
                        ceremony::DELETE_ACCOUNT,
                        ceremony_state::REFUSED,
                        "the confirmation email does not match this account",
                    )
                    .await;
                    return;
                }
            }
            super::ceremony::ask_for_passkey(
                &env,
                ceremony::DELETE_ACCOUNT,
                tonk_worker_api::CustodyIntent::PurgeAccount(Default::default()),
            )
            .await;
        })
    }
}

/// Purge the account the passkey holds.
///
/// One invocation: the root the custody cell opens signs
/// `/void/customer/purge`, the access service denies every consumer the
/// account provides in a single write and takes the data and rows from
/// there, and this device removes its replicas, retires the profile,
/// and moves onto a fresh one. Presenting the purge again after it
/// succeeded is still a purge of a customer that is gone, which the
/// service answers the same way.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) async fn purge(
    state: &AppState,
    custodian: &tonk_identity::custodian::Custodian,
) -> Result<(), String> {
    use tonk_schema::{ceremony, ceremony_state};

    let outcome = purge_inner(state, custodian).await;
    let tonk = state.read().await;
    match &outcome {
        Ok(()) => {
            super::ceremony::report(&tonk, ceremony::DELETE_ACCOUNT, ceremony_state::DONE, "/")
                .await
        }
        Err(error) => {
            super::ceremony::report(
                &tonk,
                ceremony::DELETE_ACCOUNT,
                ceremony_state::FAILED,
                error,
            )
            .await
        }
    }
    outcome
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn purge_inner(
    state: &AppState,
    custodian: &tonk_identity::custodian::Custodian,
) -> Result<(), String> {
    use dialog_varsig::Principal as _;
    use tonk_schema::{ceremony, ceremony_state};

    let current = {
        let tonk = state.read().await;
        super::ceremony::report(&tonk, ceremony::DELETE_ACCOUNT, ceremony_state::WORKING, "").await;
        load_plan(&tonk).await.map_err(|error| error.to_string())?
    };
    let account = super::custody::held_account(custodian).await?;
    let root = account
        .signer()
        .await
        .map_err(|error| format!("the account signer did not derive: {error:#}"))?;
    if root.did().to_string() != current.root_did {
        return Err("this passkey belongs to a different account".into());
    }
    let invocation = tonk_identity::request::build_purge_invocation(root)
        .await
        .map_err(|error| format!("the purge did not sign: {error:#}"))?;
    let ucan = super::customer::ucan_endpoint(
        &super::customer::service_origin().map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    super::http::post_cbor(&ucan, &invocation)
        .await
        .map_err(|error| format!("the service did not purge the account: {error}"))?;

    // The service has nothing of the account's any more; neither should
    // this device.
    for space in &current.spaces {
        let subject: dialog_varsig::Did = space
            .subject
            .parse()
            .map_err(|error| format!("reviewed space DID became invalid: {error:?}"))?;
        super::repository::remove_space_inner(state, &subject)
            .await
            .map_err(|error| format!("the local replica of {subject} was not removed: {error}"))?;
        let tonk = state.read().await;
        super::customer::retract_space_provider(&tonk, &subject).await;
    }
    let retired = {
        let tonk = state.read().await;
        super::customer::clear_customer(&tonk)
            .await
            .map_err(|error| error.to_string())?;
        tonk.profile.did()
    };
    let _ = super::account::unlink(State(state.clone()))
        .await
        .map_err(|error| format!("the profile did not unlink: {error}"))?;
    // Finish on a fresh profile so the released email can immediately
    // create a genuinely new account.
    let _ = super::profiles::add(State(state.clone()))
        .await
        .map_err(|error| format!("a fresh profile did not open: {error}"))?;
    // Permanent deletion retires this account's profile rather than
    // rebinding its retained joined spaces and delegations to another
    // root. A retired profile that holds nothing is forgotten outright;
    // one that still holds joined spaces stays listed as a local
    // workspace so they remain reachable. After the rotation: moving
    // profiles re-records the outgoing one so it stays reachable, which
    // would undo a removal made before it.
    if current.joined_spaces == 0 {
        let tonk = state.read().await;
        if let Err(error) = tonk
            .registry
            .remove_roster(&tonk.storage, &tonk.operator, &retired)
            .await
        {
            log!("delete-account: the retired profile stays listed: {error}");
        }
    }
    Ok(())
}
