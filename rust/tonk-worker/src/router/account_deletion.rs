//! Account deletion: a review read from the account db, one root-signed
//! purge presented to the access service, then local cleanup.

use std::collections::BTreeSet;

use axum::{
    Json,
    extract::{Extension, State},
};
use axum_wasm_macros::wasm_compat;
use dialog_query::{Output as _, Query, Term};
use dialog_ucan_core::InvocationChain;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;
use tonk_identity::request::PURGE_COMMAND;
use tonk_schema::SpaceProvider;
use tonk_schema::domain::space::Provider;
use tonk_schema::prelude::DidExt as _;
use tonk_worker_api::{
    AccountDeletionPlan, AccountDeletionRequest, AccountDeletionResult, AccountDeletionSpace,
    AccountSpaceDeletionRequest, HostedSpaceDeletionResult,
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

/// POST `/api/account/delete` presents the page's root-signed purge.
///
/// One invocation: the access service denies every consumer the account
/// provides in a single write and takes the data and the rows from
/// there, so nothing here loops over spaces or orders service calls.
/// Presenting it again after it succeeded is still a purge of a
/// customer that is gone, which the service answers the same way.
#[wasm_compat]
pub async fn delete(
    State(state): State<AppState>,
    Extension(origin): Extension<RequestOrigin>,
    Json(request): Json<AccountDeletionRequest>,
) -> Result<Json<AccountDeletionResult>, TonkWorkerError> {
    let current = {
        let state = state.read().await;
        load_plan(&state).await?
    };
    if request.confirmed_email != current.email {
        return Err(TonkWorkerError::Forbidden(
            "the confirmed email does not match the account's verified email".into(),
        ));
    }
    // The page signed it; the worker only checks it is the purge of
    // THIS account before forwarding, so a stray invocation cannot be
    // laundered through this route.
    let invocation = hex::decode(&request.invocation_hex)
        .map_err(|error| TonkWorkerError::Forbidden(format!("purge is not hex: {error}")))?;
    let chain = InvocationChain::try_from(invocation.as_slice())
        .map_err(|error| TonkWorkerError::Forbidden(format!("purge is malformed: {error}")))?;
    if chain.subject().to_string() != current.root_did
        || chain.command().0 != PURGE_COMMAND.map(str::to_string)
    {
        return Err(TonkWorkerError::Forbidden(
            "the signed purge does not name this account".into(),
        ));
    }
    let ucan = super::customer::ucan_endpoint(origin.url())?;
    super::http::post_cbor(&ucan, &invocation).await?;

    // The service has nothing of the account's any more; neither should
    // this device.
    for space in &current.spaces {
        let subject: dialog_varsig::Did = space.subject.parse().map_err(|error| {
            TonkWorkerError::Internal(format!("reviewed space DID became invalid: {error:?}"))
        })?;
        #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
        super::repository::remove_space_inner(&state, &subject).await?;
        let state = state.read().await;
        super::customer::retract_space_provider(&state, &subject).await;
    }
    {
        let current_state = state.read().await;
        super::customer::clear_customer(&current_state).await?;
    }
    let _ = super::account::unlink(State(state.clone())).await?;
    // Permanent deletion retires this account's profile rather than
    // rebinding its retained joined spaces and delegations to another root.
    // Unlike ordinary sign-out, finish on a fresh profile so the released
    // email can immediately create a genuinely new account.
    let _ = super::profiles::add(State(state.clone())).await?;

    Ok(Json(AccountDeletionResult {
        deleted_spaces: current.spaces.len(),
        retained_joined_spaces: current.joined_spaces,
    }))
}
