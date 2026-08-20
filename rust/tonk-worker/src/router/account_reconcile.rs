//! Bounded, retryable browser enrollment for locally mounted spaces.

use std::collections::HashSet;
use std::sync::{LazyLock, Mutex};

use dialog_repository::RepositoryExt as _;
use dialog_varsig::Did;
use serde::{Deserialize, Serialize};
use tonk_common::log;
use tonk_worker_api::AccountSpaceEnrollment;

use crate::TonkWorkerError;
use crate::worker::TonkState;

static RECONCILE_IN_FLIGHT: LazyLock<Mutex<HashSet<String>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));
const ENROLLMENT_PREFIX: &str = "tonk-account-enrollment-v1/";

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct EnrollmentRecord {
    version: u8,
    subject: String,
    phase: AccountSpaceEnrollment,
    #[serde(default)]
    connected: bool,
    error: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RemotePlan {
    Preserve,
    AddDefaultRelay,
    ProvisionDefaults,
}

fn remote_plan(
    execution: &super::create_invite::ConfiguredRemoteRequirement,
    has_default_relay: bool,
) -> Result<RemotePlan, super::create_invite::RemoteRefusal> {
    match execution {
        super::create_invite::ConfiguredRemoteRequirement::Ready(remote)
            if remote.revocation_url.is_none() && has_default_relay =>
        {
            Ok(RemotePlan::AddDefaultRelay)
        }
        super::create_invite::ConfiguredRemoteRequirement::Ready(_) => Ok(RemotePlan::Preserve),
        super::create_invite::ConfiguredRemoteRequirement::Refused(
            super::create_invite::RemoteRefusal::NotSynced,
        ) => Ok(RemotePlan::ProvisionDefaults),
        super::create_invite::ConfiguredRemoteRequirement::Refused(reason) => Err(*reason),
    }
}

async fn save_status(
    tonk: &TonkState,
    subject: &str,
    phase: AccountSpaceEnrollment,
    error: Option<&str>,
) -> Result<(), TonkWorkerError> {
    let bytes = serde_json::to_vec(&EnrollmentRecord {
        version: 1,
        subject: subject.to_owned(),
        phase,
        connected: phase == AccountSpaceEnrollment::Connected,
        error: error.map(str::to_owned),
    })
    .map_err(|error| TonkWorkerError::Internal(format!("serialize enrollment state: {error}")))?;
    tonk.profile
        .credential()
        .site(format!("{ENROLLMENT_PREFIX}{subject}"))
        .save(bytes)
        .perform(&tonk.operator)
        .await
        .map_err(|error| TonkWorkerError::Internal(format!("save enrollment state: {error}")))
}

async fn reconcile_one(
    tonk: &TonkState,
    account: &Did,
    account_key: &str,
    key: &str,
    default_remote: &str,
    default_relay: Option<&str>,
) -> Result<(), TonkWorkerError> {
    let repository = tonk
        .profile
        .repository(key)
        .load()
        .perform(&tonk.operator)
        .await
        .map_err(|error| TonkWorkerError::Internal(format!("load enrollment target: {error}")))?;
    let subject = repository.did();
    let subject_text = subject.to_string();
    save_status(
        tonk,
        &subject_text,
        AccountSpaceEnrollment::Provisioning,
        None,
    )
    .await?;

    let mut execution =
        super::create_invite::resolve_configured_remote_url(tonk, &repository).await?;
    match remote_plan(&execution, default_relay.is_some()) {
        Ok(RemotePlan::AddDefaultRelay) => {
            let super::create_invite::ConfiguredRemoteRequirement::Ready(remote) = &execution
            else {
                unreachable!("relay augmentation requires a configured remote")
            };
            super::repository::ensure_account_remote(
                tonk,
                key,
                remote.access_url.as_str(),
                default_relay,
            )
            .await?;
            execution =
                super::create_invite::resolve_configured_remote_url(tonk, &repository).await?;
        }
        Ok(RemotePlan::Preserve) => {}
        Ok(RemotePlan::ProvisionDefaults) => {
            super::repository::ensure_account_remote(tonk, key, default_remote, default_relay)
                .await?;
            execution =
                super::create_invite::resolve_configured_remote_url(tonk, &repository).await?;
        }
        Err(reason) => {
            return Err(TonkWorkerError::Conflict(format!(
                "existing upstream is not eligible for automatic enrollment: {}",
                reason.code()
            )));
        }
    }
    let super::create_invite::ConfiguredRemoteRequirement::Ready(remote) = execution else {
        return Err(TonkWorkerError::Conflict(
            "space has no usable remote after enrollment".to_string(),
        ));
    };

    save_status(
        tonk,
        &subject_text,
        AccountSpaceEnrollment::PendingPush,
        None,
    )
    .await?;
    let content = tonk
        .reactor
        .repository(key)
        .branch(tonk_account::MAIN_BRANCH)
        .acquire(&tonk.operator)
        .await
        .map_err(|error| TonkWorkerError::Internal(format!("open content branch: {error}")))?;
    tonk.reactor
        .repository(key)
        .branch(tonk_account::MAIN_BRANCH)
        .push()
        .perform(&tonk.operator)
        .await
        .map_err(|error| TonkWorkerError::Internal(format!("push content branch: {error}")))?;
    let tree = content
        .handle()
        .revision()
        .ok_or_else(|| TonkWorkerError::Internal("content branch has no confirmed head".into()))?
        .tree
        .to_string();

    let account_branch = tonk
        .reactor
        .repository(account_key)
        .branch(tonk_account::MAIN_BRANCH)
        .acquire(&tonk.operator)
        .await
        .map_err(|error| TonkWorkerError::Internal(format!("open account branch: {error}")))?;
    let prefix = super::repository::space_root_prefix(tonk, &subject).await?;
    let retained = tonk_account::delegations::retain_space_delegation(
        account_branch.handle(),
        &prefix,
        &tonk.operator,
    )
    .await
    .map_err(|error| TonkWorkerError::Internal(format!("retain space authority: {error}")))?;
    if retained {
        #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
        tonk.sync_queue.mark_dirty(account_key, js_sys::Date::now());
    }
    let name = super::repository::repository_label(tonk, &repository, key).await;
    let remote_url = remote.access_url.to_string();
    let revocation_url = remote.revocation_url.as_ref().map(ToString::to_string);
    tonk_schema::account::record_active_account_space(
        account_branch.handle(),
        tonk_schema::account::AccountSpaceInput {
            account: account.clone(),
            subject: subject.clone(),
            name: Some(name.clone()),
            remote_url: Some(remote_url.clone()),
            revocation_url: revocation_url.clone(),
            confirmed_revision: Some(tree.clone()),
        },
        &tonk.operator,
    )
    .await
    .map_err(|error| TonkWorkerError::Conflict(error.to_string()))?;
    let configuration = super::repository::space_config(&remote_url, revocation_url.as_deref())
        .map_err(|error| TonkWorkerError::Internal(error.to_string()))?;
    super::repository::record_space_mount(tonk, &subject, &configuration, Some(name.as_str()))
        .await;
    tonk.reactor
        .repository(account_key)
        .branch(tonk_account::MAIN_BRANCH)
        .push()
        .perform(&tonk.operator)
        .await
        .map_err(|error| TonkWorkerError::Internal(format!("push account membership: {error}")))?;
    save_status(tonk, &subject_text, AccountSpaceEnrollment::Connected, None).await?;
    Ok(())
}

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod planning_tests {
    use super::*;
    use url::Url;
    use wasm_bindgen_test::wasm_bindgen_test_configure;

    wasm_bindgen_test_configure!(run_in_service_worker);

    fn ready(
        url: &str,
        relay: Option<&str>,
    ) -> crate::router::create_invite::ConfiguredRemoteRequirement {
        crate::router::create_invite::ConfiguredRemoteRequirement::Ready(
            crate::router::create_invite::ConfiguredRemoteExecutionUrls {
                access_url: Url::parse(url).unwrap(),
                revocation_url: relay.map(|relay| Url::parse(relay).unwrap()),
            },
        )
    }

    #[dialog_common::test]
    async fn it_converges_pre_account_local_spaces_after_attachment() {
        let custom = ready(
            "https://custom.example/ucan/",
            Some("https://custom.example/revocations/"),
        );
        assert_eq!(remote_plan(&custom, true), Ok(RemotePlan::Preserve));

        let custom_without_relay = ready("https://custom.example/ucan/", None);
        assert_eq!(
            remote_plan(&custom_without_relay, true),
            Ok(RemotePlan::AddDefaultRelay),
            "the existing content upstream is preserved while only missing metadata is added"
        );
        assert_eq!(
            remote_plan(
                &crate::router::create_invite::ConfiguredRemoteRequirement::Refused(
                    crate::router::create_invite::RemoteRefusal::NotSynced,
                ),
                true,
            ),
            Ok(RemotePlan::ProvisionDefaults),
            "only a space with no upstream is provisioned from deployment defaults"
        );
    }
}

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod retry_tests {
    use super::*;
    use dialog_repository::RepositoryExt as _;
    use wasm_bindgen_test::wasm_bindgen_test_configure;

    wasm_bindgen_test_configure!(run_in_service_worker);

    #[dialog_common::test]
    async fn it_retries_remote_attachment_after_create_failure() {
        let state = crate::router::tests::test_state().await;
        let (app, shared, _lsp) = crate::router::api_router_with_state(state);
        let key = crate::router::tests::put_repo(&app, "retry-enrollment").await;
        let tonk = shared.read().await;
        let repository = tonk
            .profile
            .repository(&key)
            .load()
            .perform(&tonk.operator)
            .await
            .unwrap();
        let subject = repository.did().to_string();
        let revision = tonk
            .reactor
            .repository(&key)
            .branch(tonk_account::MAIN_BRANCH)
            .acquire(&tonk.operator)
            .await
            .unwrap()
            .handle()
            .revision();

        save_status(
            &tonk,
            &subject,
            AccountSpaceEnrollment::Error,
            Some("provision: create remote failed"),
        )
        .await
        .unwrap();
        save_status(&tonk, &subject, AccountSpaceEnrollment::Provisioning, None)
            .await
            .unwrap();
        save_status(&tonk, &subject, AccountSpaceEnrollment::PendingPush, None)
            .await
            .unwrap();

        let retry_repository = tonk
            .profile
            .repository(&key)
            .load()
            .perform(&tonk.operator)
            .await
            .unwrap();
        assert_eq!(retry_repository.did().to_string(), subject);
        assert_eq!(
            tonk.reactor
                .repository(&key)
                .branch(tonk_account::MAIN_BRANCH)
                .acquire(&tonk.operator)
                .await
                .unwrap()
                .handle()
                .revision(),
            revision,
            "retry state must not create or rewrite the local space"
        );
        let bytes = tonk
            .profile
            .credential()
            .site(format!("{ENROLLMENT_PREFIX}{subject}"))
            .load::<Vec<u8>>()
            .perform(&tonk.operator)
            .await
            .unwrap();
        let record: EnrollmentRecord = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(record.subject, subject);
        assert_eq!(record.phase, AccountSpaceEnrollment::PendingPush);
        assert!(!record.connected);
    }
}

/// Run one coalesced, bounded pass for the currently active profile.
pub(crate) async fn reconcile(tonk: &TonkState) {
    {
        let mut in_flight = RECONCILE_IN_FLIGHT
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !in_flight.insert(tonk.profile_name.clone()) {
            return;
        }
    }
    let result = async {
        let Some(defaults) = super::account::deployment_defaults(tonk).await? else {
            return Ok::<(), TonkWorkerError>(());
        };
        let Some(default_remote) = defaults.access_remote.as_deref() else {
            return Ok(());
        };
        let ready = super::account_state::require_ready_account_state(tonk).await?;
        for key in super::profile_name::real_space_keys(tonk)
            .await
            .into_iter()
            .take(32)
        {
            if let Err(error) = reconcile_one(
                tonk,
                &ready.subject,
                &ready.key,
                &key,
                default_remote,
                defaults.revocation_relay.as_deref(),
            )
            .await
            {
                let subject = tonk
                    .profile
                    .repository(&key)
                    .load()
                    .perform(&tonk.operator)
                    .await
                    .map(|repository| repository.did().to_string())
                    .unwrap_or(key.clone());
                let message = error.to_string();
                let _ = save_status(
                    tonk,
                    &subject,
                    AccountSpaceEnrollment::Error,
                    Some(&message),
                )
                .await;
                log!("account enrollment of '{key}' remains pending: {error}");
            }
        }
        Ok(())
    }
    .await;
    if let Err(error) = result {
        log!("account enrollment pass skipped: {error}");
    }
    RECONCILE_IN_FLIGHT
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .remove(&tonk.profile_name);
}
