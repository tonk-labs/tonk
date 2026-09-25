//! Account deletion: a review read from the account db, one root-signed
//! purge presented to the access service, then local cleanup. The purge
//! is a command the hub asserts; the passkey that signs it is asked for
//! through the custody relay, and the worker signs with the root it
//! recovers.

use std::collections::BTreeSet;

use dialog_query::{Output as _, Query, Term};
use tonk_common::log;
use tonk_schema::SpaceProvider;
use tonk_schema::domain::space::Provider;
use tonk_schema::prelude::DidExt as _;
use tonk_worker_api::{AccountDeletionPlan, AccountDeletionSpace};

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use super::AppState;
use crate::TonkWorkerError;
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
        .branch(&state.active_branch)
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

#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl dialog_capability::Provider<tonk_schema::command::DeleteAccount>
    for crate::router::CommandEnv
{
    async fn execute(&self, command: tonk_schema::command::DeleteAccount) {
        use tonk_schema::{ceremony, ceremony_state};

        let email = command.email.0;
        log!(
            "delete-account: asked from client {:?}",
            self.client().map(|c| c.0.clone())
        );
        {
            let tonk = self.state().read().await;
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
                    "the reviewed email does not match this account",
                )
                .await;
                return;
            }
        }
        super::ceremony::ask_for_passkey(
            self,
            ceremony::DELETE_ACCOUNT,
            tonk_worker_api::CustodyIntent::PurgeAccount(Default::default()),
        )
        .await;
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
    source: Option<&super::ClientId>,
    custodian: &tonk_identity::custodian::Custodian,
) -> Result<(), String> {
    use tonk_schema::{ceremony, ceremony_state};

    let outcome = purge_inner(state, source, custodian).await;
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
    source: Option<&super::ClientId>,
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
        tonk.active_branch.clone()
    };
    // Signing out moves the profile onto an empty branch and reloads the
    // page onto it, which is also where a genuinely new account can be
    // created. The deleted account's branch stays behind as data: with
    // its account gone there is nothing to return to unless spaces were
    // joined through it, so a branch holding none is forgotten rather
    // than left listed as a ghost, and one that still holds joined
    // spaces stays listed so they remain reachable.
    super::profiles::sign_out(state, source)
        .await
        .map_err(|error| format!("the profile did not unlink: {error}"))?;
    if current.joined_spaces == 0 {
        let tonk = state.read().await;
        super::profile::forget_branch(&tonk, &retired).await;
    }
    Ok(())
}
