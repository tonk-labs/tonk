//! Account deletion: a review read from the account db, one root-signed
//! purge presented to the access service, then local cleanup. The purge
//! is a command the hub asserts; the passkey that signs it is asked for
//! through the custody relay, and the worker signs with the root it
//! recovers.

use std::collections::BTreeSet;

#[cfg(target_arch = "wasm32")]
use axum::extract::Extension;
use axum::{Json, extract::State};
use dialog_query::{Output as _, Query, Term};
use tonk_common::log;
use tonk_macros::handler;
use tonk_schema::SpaceProvider;
use tonk_schema::domain::space::Provider;
use tonk_schema::prelude::DidExt as _;
use tonk_worker_api::{AccountDeletionPlan, AccountDeletionSpace};

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

/// GET `/api/account/deletion/plan` returns the exact destructive scope.
#[handler]
pub async fn plan(
    State(state): State<AppState>,
) -> Result<Json<AccountDeletionPlan>, TonkWorkerError> {
    let state = state.read().await;
    Ok(Json(load_plan(&state).await?))
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
            // The purge is signed by the root the custody cell opens, and
            // the service keeps that cell closed until the emailed link is
            // opened. Asking for the passkey now would only fail after it.
            let registration = super::customer::registration(&tonk).await;
            if let Some(reason) = not_deletable_yet(&registration) {
                super::ceremony::report(
                    &tonk,
                    ceremony::DELETE_ACCOUNT,
                    ceremony_state::REFUSED,
                    &reason,
                )
                .await;
                return;
            }
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

/// What to tell someone deleting an account that is still waiting on its
/// emailed link to be opened.
const AWAITING_ACTIVATION: &str = "this account is not confirmed yet. Open the link in your \
     activation email, then delete it. If you already opened it, try again in a moment.";

/// Why this account cannot be deleted yet, or `None` when it can.
///
/// Only an account awaiting activation is held back: its custody cell
/// stays closed until the link is opened, so no passkey can sign the
/// purge.
fn not_deletable_yet(registration: &super::customer::Registration) -> Option<String> {
    match registration {
        super::customer::Registration::AwaitingActivation { .. } => {
            Some(AWAITING_ACTIVATION.to_owned())
        }
        super::customer::Registration::Unregistered
        | super::customer::Registration::Served { .. }
        | super::customer::Registration::Suspended => None,
    }
}

/// A purge failure as the settings page shows it.
///
/// A custody refusal travels as `code`, a unit separator, then the
/// message, so the custody reply can carry the code. The status row is
/// read by a person, so it carries the sentence and never the code.
#[cfg(any(test, all(target_arch = "wasm32", feature = "persona")))]
pub(super) fn readable_failure(error: &str) -> String {
    use tonk_identity::custody::CustodyDenial;

    match error.split_once('\u{1f}') {
        Some((code, message)) => match CustodyDenial::from_code(code, message) {
            Some(CustodyDenial::AwaitingActivation) => AWAITING_ACTIVATION.to_owned(),
            _ => message.to_owned(),
        },
        None => error.to_owned(),
    }
}

/// Present `invocation`, a `/void/customer/purge` of the account `root`, to
/// the access service, and clear this device of the account: its replicas,
/// its customer record and its link.
#[cfg(target_arch = "wasm32")]
pub(crate) async fn purged(
    state: &AppState,
    source: Option<&super::ClientId>,
    root: &str,
    invocation: &[u8],
) -> Result<(), String> {
    let current = {
        let tonk = state.read().await;
        load_plan(&tonk).await.map_err(|error| error.to_string())?
    };
    if current.root_did != root {
        return Err("this passkey belongs to a different account".into());
    }
    let ucan = super::customer::ucan_endpoint(
        &super::customer::service_origin().map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    super::http::post_cbor(&ucan, invocation)
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
    let _ = super::account::unlink(State(state.clone()), source.cloned().map(Extension))
        .await
        .map_err(|error| format!("the profile did not unlink: {error}"))?;
    if current.joined_spaces == 0 {
        let tonk = state.read().await;
        super::profile::forget_branch(&tonk, &retired).await;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[dialog_common::test]
    fn it_names_the_missing_activation_rather_than_its_code() {
        let refused =
            readable_failure("awaiting-activation\u{1f}the account is awaiting email confirmation");
        assert_eq!(refused, AWAITING_ACTIVATION);
        assert!(!refused.contains("awaiting-activation"));
    }

    #[dialog_common::test]
    fn it_keeps_the_sentence_of_any_other_refusal() {
        assert_eq!(
            readable_failure("suspended\u{1f}the account is suspended: unpaid"),
            "the account is suspended: unpaid"
        );
        assert_eq!(
            readable_failure("this passkey belongs to a different account"),
            "this passkey belongs to a different account"
        );
    }

    #[dialog_common::test]
    fn it_holds_back_only_an_account_awaiting_activation() {
        use super::super::customer::Registration;

        assert!(
            not_deletable_yet(&Registration::AwaitingActivation {
                email: "person@example.test".into(),
            })
            .is_some()
        );
        for registration in [
            Registration::Unregistered,
            Registration::Served {
                provider: "https://example.test/ucan/".into(),
            },
            Registration::Suspended,
        ] {
            assert_eq!(not_deletable_yet(&registration), None, "{registration:?}");
        }
    }

    /// A pending account is refused before the passkey is asked for,
    /// with a sentence a person can act on.
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    #[dialog_common::test]
    async fn it_refuses_to_delete_an_account_awaiting_activation() {
        use dialog_credentials::Ed25519Signer;
        use dialog_varsig::Principal as _;
        use tonk_schema::{CeremonyStatus, ceremony, ceremony_state};

        let state = crate::router::command::tests::native::test_state().await;
        {
            let tonk = state.read().await;
            let root = Ed25519Signer::import(&[91; 32]).await.unwrap();
            let grant = tonk_identity::delegation::mint_device_delegation(
                root.clone(),
                &tonk.profile.did(),
            )
            .await
            .unwrap();
            super::super::identity::persist_root(
                &tonk,
                tonk_worker_api::SaveRootRequest {
                    credential_id: "pending-test".into(),
                    delegation_hex: hex::encode(grant.to_bytes().unwrap()),
                    passkey: None,
                    encryption_key: None,
                },
            )
            .await
            .unwrap();
            tonk.reactor
                .profile_repository()
                .branch(&tonk.active_branch)
                .transaction()
                .assert(tonk_schema::AccountRegistered::new(
                    root.did().this(),
                    "pending@example.test".into(),
                    "https://example.test/ucan/".into(),
                    1,
                ))
                .commit()
                .perform(&tonk.operator)
                .await
                .unwrap();
        }

        let env =
            crate::router::CommandEnv::new(state.clone(), crate::router::CommandOrigin::default());
        dialog_capability::Provider::<tonk_schema::command::DeleteAccount>::execute(
            &env,
            tonk_schema::command::DeleteAccount {
                this: "cmd:delete-account".parse().expect("entity"),
                email: tonk_schema::domain::command::delete_account::Email(
                    "pending@example.test".into(),
                ),
            },
        )
        .await;

        let tonk = state.read().await;
        let session = tonk
            .reactor
            .profile_repository()
            .branch(&tonk.active_branch)
            .acquire(&tonk.operator)
            .await
            .unwrap();
        let rows: Vec<CeremonyStatus> = session
            .handle()
            .query()
            .select(Query::<CeremonyStatus>::default())
            .perform(&tonk.operator)
            .try_vec()
            .await
            .unwrap();
        let row = rows
            .iter()
            .find(|row| row.ceremony.0 == ceremony::DELETE_ACCOUNT)
            .expect("the refusal is reported");
        assert_eq!(row.state.0, ceremony_state::REFUSED);
        assert_eq!(row.detail.0, AWAITING_ACTIVATION);
    }
}
