//! Reviewable, root-authorized account deletion orchestration.

use std::collections::{BTreeMap, BTreeSet};

use axum::{
    Json,
    extract::{Extension, State},
};
use axum_wasm_macros::wasm_compat;
use dialog_ucan_core::InvocationChain;
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
    deletion_ready: bool,
    deletion_kind: Option<String>,
    deletion_state: String,
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
    let root = link.issuer().clone();

    let mut spaces = Vec::with_capacity(access.spaces.len());
    let mut blocked_spaces = Vec::new();
    for hosted in access.spaces {
        let mut name = names.get(&hosted.space).cloned();
        let mut proof_hex = None;
        if hosted.deletion_state != "deleted"
            && let Ok(subject) = hosted.space.parse::<dialog_varsig::Did>()
        {
            name = name.or_else(|| names.get(&hosted.space).cloned());
            proof_hex = match hosted.deletion_kind.as_deref() {
                Some("exact") => {
                    // The creator's local credential answers first; the
                    // synced directory fact covers every other device on
                    // the account.
                    match super::repository::space_deletion_grant(state, &subject, &root).await? {
                        Some(chain) => chain.to_bytes().ok().map(hex::encode),
                        None => {
                            let recorded = tonk_schema::directory::deletion_grant(
                                main.handle(),
                                &subject,
                                &state.operator,
                            )
                            .await
                            .map_err(|error| {
                                TonkWorkerError::Internal(format!(
                                    "account directory query failed: {error:?}"
                                ))
                            })?;
                            match recorded {
                                Some(grant_hex) => {
                                    let bytes = hex::decode(&grant_hex).unwrap_or_default();
                                    tonk_account::deletion::validate_deletion_grant(
                                        &bytes, &subject, &root,
                                    )
                                    .await
                                    .ok()
                                    .map(|_| grant_hex)
                                }
                                None => None,
                            }
                        }
                    }
                }
                // The original direct `space → root` proof, persisted at
                // creation on the creating device. Another device on the
                // account cannot present it yet; the plan reports the
                // space as blocked there rather than guessing.
                Some("legacy-direct") => super::repository::space_root_prefix(state, &subject)
                    .await
                    .ok()
                    .and_then(|chain| chain.to_bytes().ok())
                    .map(hex::encode),
                _ => None,
            };
        }
        if hosted.deletion_state != "deleted" && (!hosted.deletion_ready || proof_hex.is_none()) {
            blocked_spaces.push(hosted.space.clone());
        }
        spaces.push(AccountDeletionSpace {
            subject: hosted.space,
            name,
            state: hosted.deletion_state,
            proof_kind: hosted.deletion_kind,
            proof_hex,
        });
    }
    Ok(AccountDeletionPlan {
        root_did,
        email,
        spaces,
        blocked_spaces,
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

fn invocation_subject(encoded: &str) -> Result<String, TonkWorkerError> {
    let bytes = hex::decode(encoded)
        .map_err(|error| TonkWorkerError::Router(format!("invalid invocation hex: {error}")))?;
    let chain = InvocationChain::try_from(bytes.as_slice()).map_err(|error| {
        TonkWorkerError::Router(format!("invalid deletion invocation: {error}"))
    })?;
    Ok(chain.subject().to_string())
}

/// POST `/api/account/spaces/delete` deletes one reviewed owned hosted space
/// while leaving the account and every other space intact.
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
    let selected = current
        .spaces
        .iter()
        .find(|space| space.subject == request.subject)
        .ok_or_else(|| TonkWorkerError::Forbidden("space is not owned by this account".into()))?;
    if selected.state == "deleted" {
        return Ok(Json(HostedSpaceDeletionResult {
            subject: request.subject,
        }));
    }
    if selected.proof_hex.is_none()
        || current
            .blocked_spaces
            .iter()
            .any(|space| space == &request.subject)
        || invocation_subject(&request.invocation_hex)? != request.subject
    {
        return Err(TonkWorkerError::Forbidden(
            "space deletion does not match recoverable registered authority".into(),
        ));
    }
    let ucan = origin
        .url()
        .join("ucan/")
        .map_err(|error| TonkWorkerError::Internal(format!("access endpoint: {error}")))?;
    let bytes = hex::decode(&request.invocation_hex)
        .map_err(|error| TonkWorkerError::Router(format!("invalid invocation hex: {error}")))?;
    super::http::post_cbor(&ucan, &bytes).await?;

    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    {
        let subject = request.subject.parse().map_err(|error| {
            TonkWorkerError::Internal(format!("reviewed space DID became invalid: {error}"))
        })?;
        super::repository::remove_space_inner(&state, &subject).await?;
    }
    Ok(Json(HostedSpaceDeletionResult {
        subject: request.subject,
    }))
}

/// POST `/api/account/delete` executes a previously reviewed passkey ceremony.
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
    if !current.blocked_spaces.is_empty() {
        return Err(TonkWorkerError::Conflict(format!(
            "deletion proof is unavailable for: {}",
            current.blocked_spaces.join(", ")
        )));
    }
    let required: BTreeSet<_> = current
        .spaces
        .iter()
        .filter(|space| space.state != "deleted")
        .map(|space| space.subject.clone())
        .collect();
    let supplied: BTreeSet<_> = request
        .spaces
        .iter()
        .map(|space| space.subject.clone())
        .collect();
    let subjects_match = request.spaces.iter().all(|space| {
        invocation_subject(&space.invocation_hex).is_ok_and(|subject| subject == space.subject)
    });
    if supplied != required || !subjects_match {
        return Err(TonkWorkerError::Forbidden(
            "prepared deletion invocations do not match the reviewed space set".into(),
        ));
    }

    let ucan = origin
        .url()
        .join("ucan/")
        .map_err(|error| TonkWorkerError::Internal(format!("access endpoint: {error}")))?;
    for space in &request.spaces {
        let bytes = hex::decode(&space.invocation_hex)
            .map_err(|error| TonkWorkerError::Router(format!("invalid invocation hex: {error}")))?;
        super::http::post_cbor(&ucan, &bytes).await?;
    }
    let customer = hex::decode(&request.customer_invocation_hex).map_err(|error| {
        TonkWorkerError::Router(format!("invalid customer deletion invocation: {error}"))
    })?;
    match super::http::post_cbor(&ucan, &customer).await {
        Ok(_) => {}
        Err(super::http::HttpError::Upstream(failure)) if failure.status == 404 => {
            // A previous attempt may have completed access-service cleanup
            // before the account-service call failed. Continue that retry.
        }
        Err(error) => return Err(error.into()),
    }

    let account_service = {
        let state = state.read().await;
        super::account_devices::account_service_url(&state)
            .await
            .ok_or_else(|| TonkWorkerError::Conflict("no account service is attached".into()))?
    };
    let account_endpoint = Url::parse(&format!(
        "{}/account/delete",
        account_service.trim_end_matches('/')
    ))
    .map_err(|error| TonkWorkerError::Internal(format!("account deletion endpoint: {error}")))?;
    let account = hex::decode(&request.account_invocation_hex).map_err(|error| {
        TonkWorkerError::Router(format!("invalid account deletion invocation: {error}"))
    })?;
    super::http::post_cbor(&account_endpoint, &account).await?;

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

    Ok(Json(AccountDeletionResult {
        deleted_spaces: request.spaces.len(),
        retained_joined_spaces: current.joined_spaces,
    }))
}
