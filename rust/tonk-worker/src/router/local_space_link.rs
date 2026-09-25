//! Browser half of local-space adoption.
//!
//! Each step is a command the settings page asserts; the worker signs
//! consent with the already-linked browser device and runs the ordinary
//! targeted-invite join path, then answers on the `state:local-space-link`
//! overlay row with what the step produced for the waiting terminal. None
//! of it exposes an account key or account-wide grant to the CLI.

use dialog_ucan_core::time::Timestamp;
use tonk_invite::local_space_link;

use crate::TonkWorkerError;
use crate::worker::TonkState;

/// What a local-space link request asks, as the page shows it before
/// anyone approves.
struct Description {
    name: String,
    subject: Option<String>,
    callback: String,
    correlation: String,
}

fn invalid(error: impl std::fmt::Display) -> TonkWorkerError {
    TonkWorkerError::Forbidden(error.to_string())
}

async fn trusted_service() -> Result<local_space_link::TrustedService, TonkWorkerError> {
    let origin = super::customer::service_origin()?;
    let endpoint = origin.join(".well-known/tonk").map_err(|error| {
        TonkWorkerError::Internal(format!("deployment discovery URL is invalid: {error}"))
    })?;
    let response = super::http::get(&endpoint).await.map_err(|error| {
        TonkWorkerError::Internal(format!("deployment discovery failed: {error}"))
    })?;
    let config: tonk_worker_api::DeploymentConfig = serde_json::from_slice(&response.body)
        .map_err(|error| {
            TonkWorkerError::Internal(format!("deployment discovery was malformed: {error}"))
        })?;
    let service_did = config
        .service_did
        .ok_or_else(|| TonkWorkerError::Internal("deployment has no service identity".into()))?
        .parse()
        .map_err(|error| TonkWorkerError::Internal(format!("service DID is invalid: {error:?}")))?;
    let remote = super::customer::ucan_endpoint(&origin)?;
    local_space_link::TrustedService::new(service_did, remote).map_err(invalid)
}

fn decode(value: &str, label: &str) -> Result<Vec<u8>, TonkWorkerError> {
    local_space_link::decode_transport(value)
        .map_err(|_| TonkWorkerError::Router(format!("{label} is not valid base58")))
}

fn encode(bytes: &[u8]) -> Result<String, TonkWorkerError> {
    local_space_link::encode_transport(bytes).map_err(invalid)
}

/// Decode what a request asks. The page cannot read the transport
/// encoding itself; nothing is validated or signed here, that is
/// [`approve`]'s.
fn describe(request: &str) -> Result<Description, TonkWorkerError> {
    let request = local_space_link::LocalSpaceLinkRequest::from_bytes(&decode(
        request,
        "local-space link request",
    )?)
    .map_err(invalid)?;
    Ok(Description {
        name: request.name().to_owned(),
        subject: request.subject_hint().map(ToString::to_string),
        callback: request.callback().to_string(),
        correlation: request.correlation().to_owned(),
    })
}

/// Approve a request with this browser's device, answering the encoded
/// approval.
async fn approve(tonk: &TonkState, request: &str) -> Result<String, TonkWorkerError> {
    let service = trusted_service().await?;
    let request = local_space_link::LocalSpaceLinkRequest::from_bytes(&decode(
        request,
        "local-space link request",
    )?)
    .map_err(invalid)?
    .validate(&service, Timestamp::now())
    .await
    .map_err(invalid)?;
    let root = super::identity::local_root(tonk).await?;
    let device = tonk.profile.signer().signer().clone();
    let approval = local_space_link::LocalSpaceLinkApproval::issue_from_device(
        &request,
        root.delegation,
        &device,
        Timestamp::now(),
    )
    .await
    .map_err(invalid)?;
    encode(&approval.to_bytes().map_err(invalid)?)
}

/// Join the space the terminal published, answering the encoded
/// completion.
async fn complete(
    tonk: &TonkState,
    command: &tonk_schema::command::CompleteLocalSpaceLink,
) -> Result<String, TonkWorkerError> {
    let service = trusted_service().await?;
    let request = local_space_link::LocalSpaceLinkRequest::from_bytes(&decode(
        &command.request.0,
        "local-space link request",
    )?)
    .map_err(invalid)?
    .validate(&service, Timestamp::now())
    .await
    .map_err(invalid)?;

    let root = super::identity::local_root(tonk).await?;
    let approval = local_space_link::LocalSpaceLinkApproval::from_bytes(&decode(
        &command.approval.0,
        "local-space link approval",
    )?)
    .map_err(invalid)?
    .validate(&request, Some(&root.root_did), Timestamp::now())
    .await
    .map_err(invalid)?;
    let provisioned = local_space_link::LocalSpaceLinkCompletion::from_bytes(&decode(
        &command.provisioned.0,
        "local-space provisioning receipt",
    )?)
    .map_err(invalid)?
    .validate(&approval, Timestamp::now())
    .await
    .map_err(invalid)?;
    if provisioned.publication != "provisioned" {
        return Err(TonkWorkerError::Forbidden(
            "local-space provisioning receipt has the wrong stage".into(),
        ));
    }
    let outcome =
        super::join::join_for_local_space_link(tonk, &command.invite.0, &request.space).await?;
    let device = tonk.profile.signer().signer().clone();
    let completion = local_space_link::LocalSpaceLinkCompletion::issue_from_device(
        &approval,
        root.delegation,
        &device,
        format!("{}:{}", outcome.key, outcome.subject),
        Timestamp::now(),
    )
    .await
    .map_err(invalid)?;
    encode(&completion.to_bytes().map_err(invalid)?)
}

/// Provision the approved space at the account's access service,
/// answering the encoded provisioning receipt.
async fn provision(
    tonk: &TonkState,
    command: &tonk_schema::command::ProvisionLocalSpaceLink,
) -> Result<String, TonkWorkerError> {
    let service = trusted_service().await?;
    let request = local_space_link::LocalSpaceLinkRequest::from_bytes(&decode(
        &command.request.0,
        "local-space link request",
    )?)
    .map_err(invalid)?
    .validate(&service, Timestamp::now())
    .await
    .map_err(invalid)?;

    let root = super::identity::local_root(tonk).await?;
    let approval = local_space_link::LocalSpaceLinkApproval::from_bytes(&decode(
        &command.approval.0,
        "local-space link approval",
    )?)
    .map_err(invalid)?
    .validate(&request, Some(&root.root_did), Timestamp::now())
    .await
    .map_err(invalid)?;
    let consent_bytes = decode(&command.consent.0, "local-space link consent")?;
    let consent = tonk_account::prefix::validate_prefix(&consent_bytes, &approval.account)
        .await
        .map_err(invalid)?;
    if consent.subject != request.space || consent.chain.proofs().count() != 1 {
        return Err(TonkWorkerError::Forbidden(
            "local-space link consent is not direct authority for this space".into(),
        ));
    }
    super::customer::provision_consumer(tonk, &request.space, &consent.chain, None).await?;
    super::join::save_local_space_root_authority(tonk, &request.space, consent.chain).await?;
    let device = tonk.profile.signer().signer().clone();
    let provisioned = local_space_link::LocalSpaceLinkCompletion::issue_from_device(
        &approval,
        root.delegation,
        &device,
        "provisioned".into(),
        Timestamp::now(),
    )
    .await
    .map_err(invalid)?;
    encode(&provisioned.to_bytes().map_err(invalid)?)
}

/// Answer the step `step` asked at `at` on the `state:local-space-link`
/// row, replacing whatever the last step left there. A describe also
/// writes what the request asks beside it.
async fn answer(
    tonk: &TonkState,
    step: &str,
    at: u64,
    result: Result<String, TonkWorkerError>,
    description: Option<Description>,
) {
    use tonk_schema::domain::local_space_link as field;
    use tonk_schema::{LocalSpaceLinkAnswer, LocalSpaceLinkRequest};

    let Ok(this) = LocalSpaceLinkAnswer::ENTITY.parse::<dialog_artifacts::Entity>() else {
        return;
    };
    let (outcome, product) = match result {
        Ok(product) => ("done", product),
        Err(error) => {
            tonk_common::log!("local-space link {step}: {error}");
            ("failed", error.to_string())
        }
    };
    let branch = match tonk
        .reactor
        .profile_repository()
        .branch(&tonk.active_branch)
        .acquire(&tonk.operator)
        .await
    {
        Ok(branch) => branch,
        Err(error) => {
            tonk_common::log!("local-space link {step}: open the active branch: {error}");
            return;
        }
    };
    branch
        .state
        .retain_overlay_entities(|overlaid| overlaid != &this);
    if let Some(description) = description {
        branch.state.assert_overlay(LocalSpaceLinkRequest {
            this: this.clone(),
            name: field::Name(description.name),
            subject: field::Subject(description.subject.unwrap_or_default()),
            callback: field::Callback(description.callback),
            correlation: field::Correlation(description.correlation),
        });
    }
    branch.state.assert_overlay(LocalSpaceLinkAnswer {
        this,
        step: field::Step(step.to_owned()),
        answered_at: field::AnsweredAt(at),
        outcome: field::Outcome(outcome.to_owned()),
        product: field::Product(product),
    });
    tonk.reactor
        .schedule_poll(std::sync::Arc::clone(&branch.state));
    tonk.reactor.run_scheduled_polls(&tonk.operator).await;
}

/// Run [`DescribeLocalSpaceLink`].
///
/// [`DescribeLocalSpaceLink`]: tonk_schema::command::DescribeLocalSpaceLink
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl dialog_capability::Provider<tonk_schema::command::DescribeLocalSpaceLink>
    for crate::router::CommandEnv
{
    async fn execute(&self, command: tonk_schema::command::DescribeLocalSpaceLink) {
        let tonk = self.state().read().await;
        match describe(&command.request.0) {
            Ok(description) => {
                answer(
                    &tonk,
                    "describe",
                    command.at.0,
                    Ok(String::new()),
                    Some(description),
                )
                .await
            }
            Err(error) => answer(&tonk, "describe", command.at.0, Err(error), None).await,
        }
    }
}

/// Run [`ApproveLocalSpaceLink`].
///
/// [`ApproveLocalSpaceLink`]: tonk_schema::command::ApproveLocalSpaceLink
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl dialog_capability::Provider<tonk_schema::command::ApproveLocalSpaceLink>
    for crate::router::CommandEnv
{
    async fn execute(&self, command: tonk_schema::command::ApproveLocalSpaceLink) {
        let tonk = self.state().read().await;
        let result = approve(&tonk, &command.request.0).await;
        answer(&tonk, "approve", command.at.0, result, None).await;
    }
}

/// Run [`ProvisionLocalSpaceLink`].
///
/// [`ProvisionLocalSpaceLink`]: tonk_schema::command::ProvisionLocalSpaceLink
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl dialog_capability::Provider<tonk_schema::command::ProvisionLocalSpaceLink>
    for crate::router::CommandEnv
{
    async fn execute(&self, command: tonk_schema::command::ProvisionLocalSpaceLink) {
        let tonk = self.state().write().await;
        let result = provision(&tonk, &command).await;
        answer(&tonk, "provision", command.at.0, result, None).await;
    }
}

/// Run [`CompleteLocalSpaceLink`].
///
/// [`CompleteLocalSpaceLink`]: tonk_schema::command::CompleteLocalSpaceLink
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl dialog_capability::Provider<tonk_schema::command::CompleteLocalSpaceLink>
    for crate::router::CommandEnv
{
    async fn execute(&self, command: tonk_schema::command::CompleteLocalSpaceLink) {
        let tonk = self.state().write().await;
        let result = complete(&tonk, &command).await;
        answer(&tonk, "complete", command.at.0, result, None).await;
    }
}

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod tests {
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    wasm_bindgen_test_configure!(run_in_service_worker);

    use dialog_credentials::Ed25519Signer;
    use dialog_ucan_core::time::Timestamp;
    use dialog_varsig::Principal as _;
    use tonk_invite::local_space_link::{LocalSpaceLinkRequest, TrustedService, encode_transport};

    use crate::router::tests::{profile_rows, test_state};
    use crate::router::{AppState, CommandEnv};

    fn answer_query(names: &[&str]) -> serde_json::Value {
        let mut with = serde_json::Map::new();
        let mut terms = serde_json::Map::new();
        terms.insert(
            "this".into(),
            tonk_schema::LocalSpaceLinkAnswer::ENTITY.into(),
        );
        for name in names {
            let as_ = if *name == "answered-at" {
                "UnsignedInteger"
            } else {
                "Text"
            };
            with.insert(
                (*name).into(),
                serde_json::json!({
                    "the": format!("xyz.tonk.local-space-link/{name}"),
                    "as": as_,
                    "cardinality": "one"
                }),
            );
            terms.insert((*name).into(), serde_json::json!({ "?": { "name": name } }));
        }
        serde_json::json!({ "predicate": { "with": with }, "terms": terms })
    }

    async fn describe(state: &AppState, request: String, at: u64) {
        let env = CommandEnv::new(state.clone(), Default::default());
        let command = tonk_schema::command::DescribeLocalSpaceLink {
            this: "command:describe".parse().unwrap(),
            request: tonk_schema::domain::command::local_space_link::describe::Request(request),
            at: tonk_schema::domain::command::local_space_link::describe::At(at),
        };
        dialog_capability::Provider::<tonk_schema::command::DescribeLocalSpaceLink>::execute(
            &env, command,
        )
        .await;
    }

    /// Describing a request answers on the overlay with what it asks — the
    /// space's name and DID and the terminal's callback — stamped with the
    /// asker's `at`, where the page reads it instead of a response body.
    #[dialog_common::test]
    async fn it_describes_a_request_on_the_overlay() {
        let state: AppState = std::sync::Arc::new(tokio::sync::RwLock::new(test_state().await));
        let owner = Ed25519Signer::generate().await.unwrap();
        let recipient = Ed25519Signer::generate().await.unwrap();
        let service_signer = Ed25519Signer::generate().await.unwrap();
        let service = TrustedService::new(
            service_signer.did(),
            "https://access.example/ucan/".parse().unwrap(),
        )
        .unwrap();
        let request = LocalSpaceLinkRequest::issue(
            &owner,
            &recipient.did(),
            "http://127.0.0.1:43210/".parse().unwrap(),
            "0123456789abcdef0123456789abcdef".into(),
            "garden".into(),
            &service,
            Timestamp::now(),
        )
        .await
        .unwrap();
        describe(
            &state,
            encode_transport(&request.to_bytes().unwrap()).unwrap(),
            7,
        )
        .await;

        let answer = profile_rows(&state, answer_query(&["step", "outcome", "answered-at"])).await;
        assert_eq!(answer.len(), 1, "{answer:?}");
        assert_eq!(answer[0]["fields"]["step"], "describe");
        assert_eq!(answer[0]["fields"]["outcome"], "done");
        assert_eq!(answer[0]["fields"]["answered-at"], 7);
        let asked = profile_rows(&state, answer_query(&["name", "subject", "callback"])).await;
        assert_eq!(asked[0]["fields"]["name"], "garden");
        assert_eq!(asked[0]["fields"]["subject"], owner.did().to_string());
        assert_eq!(asked[0]["fields"]["callback"], "http://127.0.0.1:43210/");
    }

    /// A request that does not decode answers `failed`, saying why, and
    /// leaves no description behind from an earlier request.
    #[dialog_common::test]
    async fn it_answers_an_unreadable_request_as_failed() {
        let state: AppState = std::sync::Arc::new(tokio::sync::RwLock::new(test_state().await));
        describe(&state, "not base58 at all!".into(), 9).await;

        let answer =
            profile_rows(&state, answer_query(&["outcome", "product", "answered-at"])).await;
        assert_eq!(answer[0]["fields"]["outcome"], "failed");
        assert_eq!(answer[0]["fields"]["answered-at"], 9);
        assert!(
            answer[0]["fields"]["product"]
                .as_str()
                .is_some_and(|reason| reason.contains("base58")),
            "{answer:?}"
        );
        assert!(
            profile_rows(&state, answer_query(&["name"]))
                .await
                .is_empty()
        );
    }
}
