//! Reviewable, root-authorized account deletion orchestration.

use std::collections::{BTreeMap, BTreeSet};

use axum::{
    Json,
    extract::{Extension, State},
};
use axum_wasm_macros::wasm_compat;
use serde::Deserialize;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;
use tonk_worker_api::{
    AccountDeletionPlan, AccountDeletionRequest, AccountDeletionResult, AccountDeletionSpace,
    AccountSpaceDeletionRequest, HostedSpaceDeletionResult,
};
use url::Url;

use super::AppState;
use crate::TonkWorkerError;
use crate::axum::RequestOrigin;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AccessPlan {
    customer: String,
    spaces: Vec<AccessSpace>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AccessSpace {
    space: String,
    #[serde(default)]
    deleting_since: Option<u64>,
}

async fn load_plan(
    state: &crate::worker::TonkState,
    origin: &Url,
) -> Result<AccountDeletionPlan, TonkWorkerError> {
    let link = super::account::account_link(state).await.ok_or_else(|| {
        TonkWorkerError::NotFound("this profile is not linked to an account".into())
    })?;
    let root_did = link.issuer().to_string();
    let email = super::account_devices::account_summary(state)
        .await?
        .email
        .ok_or_else(|| TonkWorkerError::Conflict("the account has no verified email".into()))?;
    let device = state.profile.signer().signer().clone();
    let inventory_invocation = tonk_identity::request::build_device_invocation(
        device.clone(),
        &link,
        vec!["customer".into(), "deletion".into(), "plan".into()],
        BTreeMap::new(),
    )
    .await
    .map_err(|error| TonkWorkerError::Internal(format!("build deletion inventory: {error}")))?;
    let ucan = origin
        .join("ucan/")
        .map_err(|error| TonkWorkerError::Internal(format!("access endpoint: {error}")))?;
    let access: AccessPlan = match super::http::post_cbor(&ucan, &inventory_invocation).await {
        Ok(response) => serde_json::from_slice(&response.body).map_err(|error| {
            TonkWorkerError::Internal(format!("access deletion inventory is invalid: {error}"))
        })?,
        Err(super::http::HttpError::Upstream(failure)) if failure.status == 404 => AccessPlan {
            customer: root_did.clone(),
            spaces: Vec::new(),
        },
        Err(error) => return Err(error.into()),
    };
    if access.customer != root_did {
        return Err(TonkWorkerError::Forbidden(
            "access deletion inventory belongs to another account".into(),
        ));
    }

    // The account directory — plain facts on profile main — is the
    // inventory: display names, deletion grants, and the joined-space
    // count all come from the synced account DB rather than any
    // service-side artifact store.
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
    let owned: BTreeSet<_> = access
        .spaces
        .iter()
        .map(|space| space.space.as_str())
        .collect();
    let joined_spaces = directory
        .iter()
        .filter(|space| !owned.contains(space.subject.to_string().as_str()))
        .count();
    let names: BTreeMap<String, String> = directory
        .iter()
        .filter_map(|space| {
            space
                .name
                .clone()
                .map(|name| (space.subject.to_string(), name))
        })
        .collect();
    let mut spaces = Vec::with_capacity(access.spaces.len());
    for hosted in access.spaces {
        spaces.push(AccountDeletionSpace {
            name: names.get(&hosted.space).cloned(),
            subject: hosted.space,
            deleting_since: hosted.deleting_since,
        });
    }
    Ok(AccountDeletionPlan {
        root_did,
        email,
        spaces,
        joined_spaces,
    })
}

/// GET `/api/account/deletion/plan` returns the exact destructive scope.
#[wasm_compat]
pub async fn plan(
    State(state): State<AppState>,
    Extension(origin): Extension<RequestOrigin>,
) -> Result<Json<AccountDeletionPlan>, TonkWorkerError> {
    let state = state.read().await;
    Ok(Json(load_plan(&state, origin.url()).await?))
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
        load_plan(&state, origin.url()).await?
    };
    // The lookup is the ownership check: a space this account does not
    // own is not in its plan.
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

/// POST `/api/account/delete` executes a previously reviewed passkey ceremony.
///
/// Whole-account deletion is fail-closed while its passkey ceremony is only a
/// UI gesture and the worker can otherwise mint destructive device-signed
/// requests on a direct POST. This handler deliberately has no state, origin,
/// or body extractor: Axum returns the refusal before profile activation,
/// inventory reads, or any local/remote effect. Keep [`delete`] intact for the
/// root-signed, replay-safe follow-up.
#[wasm_compat]
pub async fn delete_unavailable() -> Result<Json<AccountDeletionResult>, TonkWorkerError> {
    Err(TonkWorkerError::AccountStateUnavailable(
        "Secure account deletion is temporarily unavailable. No account, spaces, or local data were changed."
            .into(),
    ))
}

/// Preserved deletion orchestration, not routed until the worker can verify
/// and consume a root-signed authorization bound to its exact plan.
#[wasm_compat]
pub async fn delete(
    State(state): State<AppState>,
    Extension(origin): Extension<RequestOrigin>,
    Json(request): Json<AccountDeletionRequest>,
) -> Result<Json<AccountDeletionResult>, TonkWorkerError> {
    let current = {
        let state = state.read().await;
        load_plan(&state, origin.url()).await?
    };
    let required: BTreeSet<_> = current
        .spaces
        .iter()
        .map(|space| space.subject.clone())
        .collect();
    let supplied: BTreeSet<_> = request
        .spaces
        .iter()
        .map(|space| space.subject.clone())
        .collect();
    if supplied != required {
        return Err(TonkWorkerError::Forbidden(
            "the reviewed space set does not match the account's owned hosted spaces".into(),
        ));
    }

    if request.confirmed_email != current.email {
        return Err(TonkWorkerError::Forbidden(
            "the confirmed email does not match the account's verified email".into(),
        ));
    }

    // Deprovision every reviewed space, then finalize both services.
    // All of it signs with this device's delegated authority: the
    // account's chain reaches this device, and possession of that
    // chain is the deletion policy. The passkey assertion the UI
    // performed is a user-verification gate, not a signing ceremony.
    for space in &request.spaces {
        let subject: dialog_varsig::Did = space.subject.parse().map_err(|error| {
            TonkWorkerError::Internal(format!("reviewed space DID became invalid: {error:?}"))
        })?;
        let state = state.read().await;
        super::customer::deprovision_consumer(&state, origin.url(), &subject).await?;
    }
    let (link, device) = {
        let state = state.read().await;
        let link = super::account::account_link(&state).await.ok_or_else(|| {
            TonkWorkerError::NotFound("this profile is not linked to an account".into())
        })?;
        (link, state.profile.signer().signer().clone())
    };
    let ucan = origin
        .url()
        .join("ucan/")
        .map_err(|error| TonkWorkerError::Internal(format!("access endpoint: {error}")))?;
    let customer = tonk_identity::request::build_device_invocation(
        device.clone(),
        &link,
        vec!["customer".into(), "delete".into()],
        BTreeMap::new(),
    )
    .await
    .map_err(|error| TonkWorkerError::Internal(format!("build customer deletion: {error}")))?;
    match super::http::post_cbor(&ucan, &customer).await {
        Ok(_) => {}
        Err(super::http::HttpError::Upstream(failure)) if failure.status == 404 => {
            // A previous attempt may have completed access-service cleanup
            // before the account-service call failed. Continue that retry.
        }
        Err(error) => return Err(error.into()),
    }

    // The access service's customer row is keyed on the account and
    // its email is unique, so leaving it behind keeps the address taken
    // after the account that held it is gone.
    delete_customer(&state).await?;

    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    for space in &request.spaces {
        let subject = space.subject.parse().map_err(|error| {
            TonkWorkerError::Internal(format!("reviewed space DID became invalid: {error}"))
        })?;
        super::repository::remove_space_inner(&state, &subject).await?;
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
        deleted_spaces: request.spaces.len(),
        retained_joined_spaces: current.joined_spaces,
    }))
}

/// Tell the access service to drop the customer row, releasing the
/// address it holds.
async fn delete_customer(state: &AppState) -> Result<(), TonkWorkerError> {
    let (device, link) = {
        let tonk = state.read().await;
        let link = super::account::account_link(&tonk).await.ok_or_else(|| {
            TonkWorkerError::NotFound("this profile is not linked to an account".to_string())
        })?;
        (tonk.profile.signer().signer().clone(), link)
    };
    let body = tonk_identity::request::build_device_invocation(
        device,
        &link,
        vec!["customer".to_string(), "delete".to_string()],
        BTreeMap::new(),
    )
    .await
    .map_err(|error| TonkWorkerError::Internal(format!("build customer deletion: {error}")))?;
    let endpoint = super::customer::ucan_endpoint(&super::customer::service_origin()?)?;
    super::http::post_cbor(&endpoint, &body).await?;
    Ok(())
}
