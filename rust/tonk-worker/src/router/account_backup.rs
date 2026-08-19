//! Best-effort account backup and semantic inventory for synced spots.

#[cfg(test)]
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;

use dialog_credentials::Ed25519Signer;
use dialog_query::{Output as _, Query, Term};
use dialog_repository::{Repository, RepositoryExt as _};
use dialog_ucan_core::DelegationChain;
use dialog_ucan_core::promise::Promised;
use dialog_varsig::Did;
use tonk_account::backup::{ACCOUNT_SPOTS_CAPABILITY_V1, AccountSpotBackup, AccountSpotSummary};
use tonk_common::log;
use tonk_schema::RepositoryName;
use tonk_schema::prelude::DidExt as _;

use crate::TonkWorkerError;
use crate::worker::TonkState;

#[cfg(test)]
thread_local! {
    static BACKUP_DISPATCHES: Cell<usize> = const { Cell::new(0) };
    static BACKUP_ARTIFACTS: RefCell<Vec<AccountSpotBackup>> = const { RefCell::new(Vec::new()) };
}

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
pub(crate) fn backup_dispatch_count() -> usize {
    BACKUP_DISPATCHES.with(Cell::get)
}

#[cfg(test)]
fn capture_backup_dispatch() {
    BACKUP_DISPATCHES.with(|count| count.set(count.get() + 1));
}

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
pub(crate) fn take_backup_artifacts() -> Vec<AccountSpotBackup> {
    BACKUP_ARTIFACTS.with(|artifacts| std::mem::take(&mut *artifacts.borrow_mut()))
}

/// Resolve the account-service base URL attached to this profile.
pub(crate) async fn account_service_url(tonk: &TonkState) -> Option<String> {
    crate::router::account::provider(tonk).await
}

fn endpoint(value: String) -> Result<url::Url, TonkWorkerError> {
    url::Url::parse(&value)
        .map_err(|error| TonkWorkerError::Internal(format!("invalid service endpoint: {error}")))
}

async fn invocation(
    device: &Ed25519Signer,
    link: &DelegationChain,
    command: &str,
    arguments: BTreeMap<String, Promised>,
) -> Result<Vec<u8>, TonkWorkerError> {
    tonk_identity::request::build_device_invocation(
        device.clone(),
        link,
        vec!["account".into(), "chain".into(), command.into()],
        arguments,
    )
    .await
    .map_err(|error| TonkWorkerError::Internal(format!("build {command} invocation: {error}")))
}

struct BackedUpChains {
    keys: Vec<String>,
    account_spots_capability: bool,
}

async fn list_backed_up_chains(
    device: &Ed25519Signer,
    link: &DelegationChain,
    service: &str,
) -> Result<BackedUpChains, TonkWorkerError> {
    let body = invocation(device, link, "list", BTreeMap::new()).await?;
    let endpoint = endpoint(format!("{}/chains/list", service.trim_end_matches('/')))?;
    let response = super::http::post_cbor(&endpoint, &body).await?;
    let keys = serde_json::from_slice(&response.body)
        .map_err(|error| TonkWorkerError::Internal(format!("parse chain keys: {error}")))?;
    Ok(BackedUpChains {
        keys,
        account_spots_capability: response.account_spots_capability.as_deref()
            == Some(ACCOUNT_SPOTS_CAPABILITY_V1),
    })
}

async fn get_backed_up_bytes(
    device: &Ed25519Signer,
    link: &DelegationChain,
    service: &str,
    key: &str,
) -> Result<Vec<u8>, TonkWorkerError> {
    let arguments = [("key".to_owned(), Promised::String(key.to_owned()))]
        .into_iter()
        .collect();
    let body = invocation(device, link, "get", arguments).await?;
    let endpoint = endpoint(format!("{}/chains/get", service.trim_end_matches('/')))?;
    Ok(super::http::post_cbor(&endpoint, &body).await?.body)
}

async fn legacy_inventory(
    device: &Ed25519Signer,
    link: &DelegationChain,
    service: &str,
    keys: Vec<String>,
) -> Result<Vec<AccountSpotSummary>, TonkWorkerError> {
    let mut groups: BTreeMap<String, Vec<(String, AccountSpotBackup, Did)>> = BTreeMap::new();
    for key in keys {
        let bytes = match get_backed_up_bytes(device, link, service, &key).await {
            Ok(bytes) => bytes,
            Err(error) => {
                log!("legacy backup '{key}' could not be fetched: {error}");
                continue;
            }
        };
        let Ok(backup) = serde_json::from_slice::<AccountSpotBackup>(&bytes) else {
            continue;
        };
        let Ok(validated) = backup.validate_for(link.issuer()).await else {
            continue;
        };
        groups
            .entry(validated.subject.to_string())
            .or_default()
            .push((key, backup, validated.subject));
    }

    let mut rows = Vec::with_capacity(groups.len());
    for candidates in groups.into_values() {
        let (first_key, first, subject) = &candidates[0];
        if candidates
            .iter()
            .skip(1)
            .any(|(_, candidate, _)| candidate != first)
        {
            rows.push(AccountSpotSummary {
                subject: subject.to_string(),
                key: None,
                name: None,
                remote_url: None,
                revocation_url: None,
                ambiguous: true,
                deletion_ready: false,
            });
        } else {
            rows.push(AccountSpotSummary {
                subject: subject.to_string(),
                key: Some(first_key.clone()),
                name: first.name.clone(),
                remote_url: first.remote_url.clone(),
                revocation_url: first.revocation_url.clone(),
                ambiguous: false,
                deletion_ready: first.deletion_grant_hex.is_some(),
            });
        }
    }
    Ok(rows)
}

/// List one semantic row per spot backed up by this account.
pub(crate) async fn list_backed_up_spots(
    device: &Ed25519Signer,
    link: &DelegationChain,
    service: &str,
) -> Result<Vec<AccountSpotSummary>, TonkWorkerError> {
    let listed = list_backed_up_chains(device, link, service).await?;
    if !listed.account_spots_capability {
        return legacy_inventory(device, link, service, listed.keys).await;
    }

    let body = invocation(device, link, "spots", BTreeMap::new()).await?;
    let endpoint = endpoint(format!("{}/chains/spots", service.trim_end_matches('/')))?;
    let response = super::http::post_cbor(&endpoint, &body).await?;
    serde_json::from_slice(&response.body)
        .map_err(|error| TonkWorkerError::Internal(format!("parse account spots: {error}")))
}

/// Fetch and deserialize one selected account spot artifact.
pub(crate) async fn get_backed_up_spot(
    device: &Ed25519Signer,
    link: &DelegationChain,
    service: &str,
    key: &str,
) -> Result<AccountSpotBackup, TonkWorkerError> {
    let bytes = get_backed_up_bytes(device, link, service, key).await?;
    serde_json::from_slice(&bytes)
        .map_err(|error| TonkWorkerError::Internal(format!("bad backup artifact: {error}")))
}

/// Build, sign, and upload one immutable backup artifact.
struct BackupArtifactInput {
    chain: DelegationChain,
    deletion_grant: Option<DelegationChain>,
    remote_url: Option<String>,
    revocation_url: Option<String>,
    name: Option<String>,
}

async fn run_backup(
    device: Ed25519Signer,
    link: DelegationChain,
    service: String,
    input: BackupArtifactInput,
) -> Result<(), TonkWorkerError> {
    let chain_bytes = input
        .chain
        .to_bytes()
        .map_err(|error| TonkWorkerError::Internal(format!("serialize claimed chain: {error}")))?;
    let artifact = AccountSpotBackup {
        chain_hex: hex::encode(chain_bytes),
        deletion_grant_hex: input
            .deletion_grant
            .map(|grant| grant.to_bytes())
            .transpose()
            .map_err(|error| {
                TonkWorkerError::Internal(format!("serialize deletion grant: {error}"))
            })?
            .map(hex::encode),
        remote_url: input.remote_url,
        revocation_url: input.revocation_url,
        name: input.name,
    };
    artifact
        .validate_for(link.issuer())
        .await
        .map_err(|error| TonkWorkerError::Internal(format!("validate backup artifact: {error}")))?;
    #[cfg(test)]
    BACKUP_ARTIFACTS.with(|artifacts| artifacts.borrow_mut().push(artifact.clone()));
    let artifact_bytes = serde_json::to_vec(&artifact).map_err(|error| {
        TonkWorkerError::Internal(format!("serialize backup artifact: {error}"))
    })?;
    let arguments = [(
        "chain".to_owned(),
        Promised::String(hex::encode(artifact_bytes)),
    )]
    .into_iter()
    .collect();
    let body = invocation(&device, &link, "put", arguments).await?;
    let endpoint = endpoint(format!("{}/chains/put", service.trim_end_matches('/')))?;
    super::http::post_cbor(&endpoint, &body).await?;
    Ok(())
}

async fn dispatch_backup(
    tonk: &TonkState,
    context: &'static str,
    chain: DelegationChain,
    remote_url: Option<String>,
    revocation_url: Option<String>,
    name: Option<String>,
) -> Result<(), TonkWorkerError> {
    let Some(link) = crate::router::account::account_link(tonk).await else {
        return Ok(());
    };
    let Some(service) = account_service_url(tonk).await else {
        return Ok(());
    };
    let subject = chain.subject().ok_or_else(|| {
        TonkWorkerError::Internal("backup chain has no repository subject".to_string())
    })?;
    let deletion_grant =
        crate::router::repository::space_deletion_grant(tonk, subject, link.issuer()).await?;
    // Re-provision during every backup sweep. This upgrades an original
    // direct legacy owner proof to the access service's narrowly tagged
    // `legacy-direct` deletion mode, or installs the exact creation grant.
    // Indirect joined-space chains are refused by the service and remain
    // ordinary backups; discovery never grants destructive authority.
    if let Err(error) =
        super::customer::provision_consumer(tonk, subject, &chain, deletion_grant.as_ref()).await
    {
        log!("{context} deletion-authority upgrade skipped: {error}");
    }
    let device = tonk.profile.signer().signer().clone();
    run_backup(
        device,
        link,
        service,
        BackupArtifactInput {
            chain,
            deletion_grant,
            remote_url,
            revocation_url,
            name,
        },
    )
    .await
    .map_err(|error| TonkWorkerError::Internal(format!("{context} backup failed: {error}")))
}

async fn content_name<R>(tonk: &TonkState, repository: &Repository<R>) -> Option<String>
where
    R: dialog_varsig::Principal + Clone,
{
    let branch = repository
        .branch("main")
        .open()
        .perform(&tonk.operator)
        .await
        .ok()?;
    branch
        .query()
        .select(Query::<RepositoryName> {
            this: Term::from(repository.did().this()),
            name: Term::var("name"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .ok()?
        .into_iter()
        .next()
        .map(|row| row.name.0)
}

/// Refresh one mounted subject's backup from its persisted root prefix,
/// synced content name, and configured sync remote.
pub(crate) async fn back_up_subject(
    tonk: &TonkState,
    subject: &Did,
) -> Result<(), TonkWorkerError> {
    let repository = tonk
        .profile
        .repository(subject.repo_key())
        .load()
        .perform(&tonk.operator)
        .await
        .map_err(|error| TonkWorkerError::Internal(format!("load backup subject: {error}")))?;
    let remote =
        match crate::router::create_invite::resolve_configured_remote_url(tonk, &repository).await?
        {
            crate::router::create_invite::ConfiguredRemoteRequirement::Ready(remote) => remote,
            crate::router::create_invite::ConfiguredRemoteRequirement::Refused(_) => return Ok(()),
        };
    let prefix = crate::router::repository::space_root_prefix(tonk, subject).await?;
    let name = content_name(tonk, &repository).await;
    #[cfg(test)]
    capture_backup_dispatch();
    dispatch_backup(
        tonk,
        "spot",
        prefix,
        Some(remote.access_url.to_string()),
        remote.revocation_url.map(|url| url.to_string()),
        name,
    )
    .await?;
    Ok(())
}

/// Back up every mounted spot whose actual upstream is usable.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) async fn back_up_existing_spaces(tonk: &TonkState) {
    for key in crate::router::profile_name::real_space_keys(tonk).await {
        let Ok(repository) = tonk
            .profile
            .repository(&key)
            .load()
            .perform(&tonk.operator)
            .await
        else {
            continue;
        };
        if let Err(error) = back_up_subject(tonk, &repository.did()).await {
            log!("existing-space backup skipped: {error}");
        }
    }
}

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod tests {
    use super::*;

    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use dialog_remote_ucan_s3::UcanAddress;
    use dialog_repository::SiteAddress;
    use tower::ServiceExt as _;
    use wasm_bindgen::{JsCast as _, JsValue};
    use wasm_bindgen_test::wasm_bindgen_test_configure;

    use crate::router::repository::{
        BranchConfiguration, RemoteConfiguration, RepositoryConfiguration,
    };
    use crate::router::tests::{GlobalPropertyGuard, attach_remote, put_repo, test_state};
    use crate::router::{api_router_with_state, tests};

    wasm_bindgen_test_configure!(run_in_service_worker);

    async fn next_artifact() -> Option<AccountSpotBackup> {
        for _ in 0..20 {
            wasm_bindgen_futures::JsFuture::from(js_sys::Promise::resolve(&JsValue::NULL))
                .await
                .unwrap();
            if let Some(artifact) = take_backup_artifacts().into_iter().last() {
                return Some(artifact);
            }
        }
        None
    }

    async fn next_task() {
        let promise: js_sys::Promise =
            js_sys::Function::new_no_args("return new Promise(resolve => setTimeout(resolve, 0));")
                .call0(&JsValue::UNDEFINED)
                .unwrap()
                .dyn_into()
                .unwrap();
        wasm_bindgen_futures::JsFuture::from(promise).await.unwrap();
    }

    #[dialog_common::test]
    async fn inventory_discovers_spots_before_using_the_new_route() {
        let state = test_state().await;
        let link = crate::router::account::account_link(&state).await.unwrap();
        let device = state.profile.signer().signer().clone();
        let requests = js_sys::Array::new();
        let _requests = GlobalPropertyGuard::replace("__tonkBackupRequests", requests.as_ref());
        let advertised = JsValue::FALSE;
        let _advertised = GlobalPropertyGuard::replace("__tonkAdvertiseAccountSpots", &advertised);
        let fetch = js_sys::Function::new_with_args(
            "request",
            r#"
            globalThis.__tonkBackupRequests.push(request.url);
            if (request.url.endsWith('/chains/list')) {
                const headers = globalThis.__tonkAdvertiseAccountSpots
                    ? { 'X-Tonk-Account-Spots': 'v1' }
                    : {};
                return Promise.resolve(new Response('[]', { status: 200, headers }));
            }
            if (request.url.endsWith('/chains/spots')) {
                return Promise.resolve(new Response('[]', { status: 200 }));
            }
            return Promise.resolve(new Response('{}', { status: 500 }));
            "#,
        );
        let _fetch = GlobalPropertyGuard::replace("fetch", fetch.as_ref());

        let rows = list_backed_up_spots(
            &device,
            &link,
            crate::router::account::TEST_ACCOUNT_PROVIDER,
        )
        .await
        .unwrap();
        assert!(rows.is_empty());
        let urls: Vec<String> = requests
            .iter()
            .filter_map(|value| value.as_string())
            .collect();
        assert_eq!(urls.len(), 1);
        assert!(urls[0].ends_with("/chains/list"));
        assert!(
            urls.iter().all(|url| !url.ends_with("/chains/spots")),
            "an old service must never receive a spots request: {urls:?}"
        );

        requests.set_length(0);
        js_sys::Reflect::set(
            &js_sys::global(),
            &JsValue::from_str("__tonkAdvertiseAccountSpots"),
            &JsValue::TRUE,
        )
        .unwrap();
        let rows = list_backed_up_spots(
            &device,
            &link,
            crate::router::account::TEST_ACCOUNT_PROVIDER,
        )
        .await
        .unwrap();
        assert!(rows.is_empty());
        let urls: Vec<String> = requests
            .iter()
            .filter_map(|value| value.as_string())
            .collect();
        assert_eq!(urls.len(), 2);
        assert!(urls[0].ends_with("/chains/list"));
        assert!(urls[1].ends_with("/chains/spots"));
    }

    #[dialog_common::test]
    async fn backup_accepts_a_synced_spot_without_weakening_invite_relay_requirements() {
        let (app, state, _lsp) = api_router_with_state(test_state().await);
        let fetch = js_sys::Function::new_with_args(
            "_request",
            "return Promise.resolve(new Response('{}', { status: 200 }));",
        );
        let _fetch = GlobalPropertyGuard::replace("fetch", fetch.as_ref());

        let key = put_repo(&app, "backup-without-relay").await;
        let subject: Did = key.parse().unwrap();
        let remote = RepositoryConfiguration::default()
            .remote(
                "origin",
                RemoteConfiguration::new(SiteAddress::from(UcanAddress::new(
                    "https://sync.example.test/ucan/",
                )))
                .subject(subject.clone()),
            )
            .branch(
                "main",
                BranchConfiguration::default().upstream("origin", "main"),
            );
        let attached = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repository/{key}/remote"))
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&remote).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(attached.status(), StatusCode::OK);
        for _ in 0..5 {
            wasm_bindgen_futures::JsFuture::from(js_sys::Promise::resolve(&JsValue::NULL))
                .await
                .unwrap();
        }
        take_backup_artifacts();

        {
            let tonk = state.read().await;
            let repository = tonk
                .profile
                .repository(&key)
                .load()
                .perform(&tonk.operator)
                .await
                .unwrap();
            assert!(matches!(
                crate::router::create_invite::resolve_remote_url(&tonk, &repository)
                    .await
                    .unwrap(),
                crate::router::create_invite::RemoteRequirement::Refused(
                    crate::router::create_invite::RemoteRefusal::MissingRevocationRelay
                )
            ));
            back_up_subject(&tonk, &subject).await.unwrap();
        }
        let artifact = next_artifact()
            .await
            .expect("a configured UCAN upstream should produce a backup");
        assert_eq!(
            artifact.remote_url.as_deref(),
            Some("https://sync.example.test/ucan/")
        );
        assert_eq!(artifact.revocation_url, None);
        let root = {
            let tonk = state.read().await;
            crate::router::identity::root_did(&tonk).await.unwrap()
        };
        let validated = artifact.validate_for(&root).await.unwrap();
        assert_eq!(validated.subject, subject);
        assert!(
            validated.deletion_grant.is_some(),
            "a worker-created owned space backup carries its exact deletion grant"
        );
    }

    #[dialog_common::test]
    async fn backup_does_not_finish_before_the_account_service_accepts_it() {
        use std::cell::Cell;
        use std::rc::Rc;

        let (app, state, _lsp) = api_router_with_state(test_state().await);
        let immediate = js_sys::Function::new_with_args(
            "_request",
            "return Promise.resolve(new Response('{}', { status: 200 }));",
        );
        let _initial_fetch = GlobalPropertyGuard::replace("fetch", immediate.as_ref());
        let key = put_repo(&app, "backup-awaits-upload").await;
        attach_remote(&app, &key, "https://sync.example.test/ucan/").await;

        let started = JsValue::FALSE;
        let _started = GlobalPropertyGuard::replace("__tonkUploadStarted", &started);
        let resolver = JsValue::UNDEFINED;
        let _resolver = GlobalPropertyGuard::replace("__tonkResolveUpload", &resolver);
        let pending = js_sys::Function::new_with_args(
            "_request",
            r#"
            globalThis.__tonkUploadStarted = true;
            return new Promise(resolve => {
                globalThis.__tonkResolveUpload = () =>
                    resolve(new Response('{}', { status: 200 }));
            });
            "#,
        );
        let _pending_fetch = GlobalPropertyGuard::replace("fetch", pending.as_ref());

        let completed = Rc::new(Cell::new(false));
        let completed_task = Rc::clone(&completed);
        let state_task = state.clone();
        let subject: Did = key.parse().unwrap();
        wasm_bindgen_futures::spawn_local(async move {
            let tonk = state_task.read().await;
            back_up_subject(&tonk, &subject).await.unwrap();
            completed_task.set(true);
        });

        let mut upload_started = false;
        for _ in 0..100 {
            next_task().await;
            if js_sys::Reflect::get(&js_sys::global(), &"__tonkUploadStarted".into())
                .unwrap()
                .is_truthy()
            {
                upload_started = true;
                break;
            }
        }
        assert!(upload_started, "backup reached the mocked account service");
        assert!(
            !completed.get(),
            "backup returned before its upload settled"
        );

        let resolve: js_sys::Function =
            js_sys::Reflect::get(&js_sys::global(), &"__tonkResolveUpload".into())
                .unwrap()
                .dyn_into()
                .unwrap();
        resolve.call0(&JsValue::UNDEFINED).unwrap();
        for _ in 0..100 {
            next_task().await;
            if completed.get() {
                break;
            }
        }
        assert!(completed.get(), "backup finishes after upload acceptance");
    }

    #[dialog_common::test]
    async fn backup_skips_a_spot_without_an_actual_upstream() {
        let (app, state, _lsp) = api_router_with_state(test_state().await);
        let fetch = js_sys::Function::new_with_args(
            "_request",
            "return Promise.resolve(new Response('{}', { status: 200 }));",
        );
        let _fetch = GlobalPropertyGuard::replace("fetch", fetch.as_ref());
        take_backup_artifacts();

        let key = tests::put_repo(&app, "backup-without-upstream").await;
        let subject: Did = key.parse().unwrap();
        {
            let tonk = state.read().await;
            back_up_subject(&tonk, &subject).await.unwrap();
        }
        for _ in 0..5 {
            wasm_bindgen_futures::JsFuture::from(js_sys::Promise::resolve(&JsValue::NULL))
                .await
                .unwrap();
        }
        assert!(
            take_backup_artifacts().is_empty(),
            "a spot without an actual upstream must be skipped"
        );
    }
}
