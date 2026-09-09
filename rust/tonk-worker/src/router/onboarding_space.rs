//! One local welcome space per browser, requested only by the root UI visit.
use axum::{Json, extract::State};
use axum_wasm_macros::wasm_compat;
use base64::Engine as _;
use dialog_artifacts::{Artifact, Changes, Update};
use dialog_operator::Profile;
use dialog_query::{Output as _, Query, Term};
use dialog_repository::{Blob, RepositoryExt as _};
use serde::{Deserialize, Serialize};
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;
use tonk_schema::{Replica, prelude::DidExt as _};

use super::{AppState, BranchConfiguration, RepositoryConfiguration, repository};
use crate::{TonkWorkerError, worker::TonkState};

const JOURNAL: &str = "tonk-onboarding-space-v1";
const SEED_URL: &str = "/library/onboarding.yaml";

#[derive(Default, Serialize, Deserialize)]
struct Progress {
    subject: Option<dialog_varsig::Did>,
    complete: bool,
    profile: Option<String>,
}

#[derive(Deserialize)]
struct Snapshot {
    #[serde(deserialize_with = "decode_artifacts")]
    artifacts: Vec<Artifact>,
    blobs: Vec<SeedBlob>,
}

/// Dialog's legacy Value parser uses a size-limited base58 decoder. Exported
/// compiled rules exceed that limit, so decode binary values with bs58.
fn decode_artifacts<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<Artifact>, D::Error> {
    #[derive(Deserialize)]
    struct Row {
        the: String,
        of: String,
        is: String,
    }
    Vec::<Row>::deserialize(deserializer)?
        .into_iter()
        .map(|row| {
            use serde::de::Error as _;
            let value = if let Some(bytes) = row.is.strip_prefix("bytes:") {
                dialog_artifacts::Value::Bytes(
                    bs58::decode(bytes).into_vec().map_err(D::Error::custom)?,
                )
            } else {
                row.is.parse().map_err(D::Error::custom)?
            };
            Ok(Artifact {
                the: row.the.parse().map_err(D::Error::custom)?,
                of: row.of.parse().map_err(D::Error::custom)?,
                is: value,
                cause: None,
            })
        })
        .collect()
}

#[derive(Deserialize)]
struct SeedBlob {
    entity: dialog_artifacts::Entity,
    data: String,
}

#[derive(Debug, Serialize)]
pub struct WelcomeResponse {
    path: Option<String>,
}

fn internal(error: impl std::fmt::Display) -> TonkWorkerError {
    TonkWorkerError::Internal(format!("onboarding space: {error}"))
}

async fn save(
    tonk: &TonkState,
    registry: &Profile,
    progress: &Progress,
) -> Result<(), TonkWorkerError> {
    registry
        .credential()
        .site(JOURNAL)
        .save(serde_json::to_vec(progress).map_err(internal)?)
        .perform(&tonk.storage)
        .await
        .map_err(internal)
}

#[wasm_compat]
pub async fn welcome(
    State(state): State<AppState>,
) -> Result<Json<WelcomeResponse>, TonkWorkerError> {
    // Hold the write guard through setup: concurrent root visits and profile
    // replacement must observe the completed journal, never create a second copy.
    let tonk = state.write().await;
    let registry = tonk
        .registry
        .open_profile(&tonk.storage, tonk.registry.initial_profile())
        .await?;
    let mut progress: Progress = match registry
        .credential()
        .site(JOURNAL)
        .load::<Vec<u8>>()
        .perform(&tonk.storage)
        .await
    {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(internal)?,
        Err(error) if crate::credential::is_missing(&error) => Progress::default(),
        Err(error) => return Err(internal(error)),
    };
    if progress.complete
        || progress
            .profile
            .as_ref()
            .is_some_and(|profile| profile != &tonk.profile_name)
    {
        return Ok(Json(WelcomeResponse { path: None }));
    }

    let session = tonk
        .reactor
        .profile_repository()
        .branch("main")
        .acquire(&tonk.operator)
        .await
        .map_err(internal)?;
    let replicas: Vec<Replica> = session
        .handle()
        .query()
        .select(Query::<Replica> {
            this: Term::var("this"),
            subject: Term::var("subject"),
            profile: Term::var("profile"),
            kind: Term::var("kind"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .map_err(internal)?;
    if let Some(subject) = &progress.subject {
        if !replicas
            .iter()
            .any(|replica| replica.subject.0.to_string() == subject.to_string())
        {
            // Removal while a previous document was interrupted is intentional.
            progress.complete = true;
            save(&tonk, &registry, &progress).await?;
            return Ok(Json(WelcomeResponse { path: None }));
        }
    } else {
        let has_account = match super::identity::local_root(&tonk).await {
            Ok(_) => true,
            Err(TonkWorkerError::RootRequired) => false,
            Err(error) => return Err(error),
        };
        if has_account
            || replicas
                .iter()
                .any(|replica| replica.kind == Replica::repository_kind())
        {
            progress.complete = true;
            save(&tonk, &registry, &progress).await?;
            return Ok(Json(WelcomeResponse { path: None }));
        }
    }

    // Validate the entire asset before creating anything. JSON is the YAML
    // subset used for this typed snapshot; Artifact preserves bytes and entities.
    let seed = repository::fetch_standard_library(SEED_URL).await?;
    let snapshot: Snapshot = serde_json::from_str(&seed).map_err(internal)?;
    let agent_library =
        repository::fetch_standard_library("/library/onboarding-agent.yaml").await?;
    let subject = match &progress.subject {
        Some(subject) => subject.clone(),
        None => {
            let configuration =
                RepositoryConfiguration::default().branch("main", BranchConfiguration::default());
            let repository =
                repository::create_repository(&tonk, "Welcome to Tonk", &configuration)
                    .await
                    .map_err(internal)?;
            let subject = repository.did();
            progress.subject = Some(subject.clone());
            progress.profile = Some(tonk.profile_name.clone());
            save(&tonk, &registry, &progress).await?;
            subject
        }
    };
    let key = subject.repo_key();
    let scaffold = repository::fetch_standard_library("/library/core.yaml").await?;
    let name = repository::repository_name_body(&subject, "Welcome to Tonk").map_err(internal)?;
    repository::seed_standard_library(&tonk, key, "main", &format!("{scaffold}\n{name}")).await?;
    let repository = tonk
        .profile
        .repository(key)
        .load()
        .perform(&tonk.operator)
        .await
        .map_err(internal)?;
    let branch = repository
        .branch("main")
        .open()
        .perform(&tonk.operator)
        .await
        .map_err(internal)?;
    for blob in snapshot.blobs {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(blob.data)
            .map_err(internal)?;
        let chunks =
            futures_util::stream::iter(vec![Ok::<_, dialog_effects::blob::BlobError>(bytes)]);
        let entity = Blob::import(chunks)
            .write(branch.blobs())
            .perform(&tonk.operator)
            .await
            .map_err(internal)?;
        if entity != blob.entity {
            return Err(internal("bundled blob hash mismatch"));
        }
    }
    let mut changes = Changes::new();
    for artifact in snapshot.artifacts {
        changes.associate(artifact.the, artifact.of, artifact.is);
    }
    tonk.reactor
        .repository(key)
        .branch("main")
        .transaction()
        .assert(changes)
        .commit()
        .perform(&tonk.operator)
        .await
        .map_err(internal)?;
    repository::seed_standard_library(&tonk, key, "main", &agent_library).await?;
    repository::set_replica_status(&tonk, &subject, Replica::initialized_status())
        .await
        .map_err(internal)?;
    progress.complete = true;
    save(&tonk, &registry, &progress).await?;
    Ok(Json(WelcomeResponse {
        path: Some(format!("/space/{key}")),
    }))
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    #[dialog_common::test]
    async fn first_visit_seeds_once_even_with_concurrent_requests() {
        let state = crate::router::command::tests::native::test_state().await;
        let (first, second) =
            futures_util::join!(welcome(State(state.clone())), welcome(State(state.clone())));
        let paths = [first.unwrap().0.path, second.unwrap().0.path];
        assert_eq!(paths.iter().filter(|path| path.is_some()).count(), 1);
        let path = paths.into_iter().flatten().next().unwrap();
        let key = path.strip_prefix("/space/").unwrap();
        let tonk = state.read().await;
        let response = super::super::evaluate::evaluate_body(
            &tonk,
            key,
            "main",
            "vault/welcome-page:\n  heading: ?heading\n".into(),
            false,
        )
        .await
        .unwrap();
        assert_eq!(response.matches_after[0].results.len(), 1);
        // The imported legacy invitation schema must not prevent the playground
        // from joining a current account-scoped handoff with the space name.
        let handoff = super::super::evaluate::evaluate_body(
            &tonk,
            key,
            "main",
            format!(
                "tonk/agent-handoff-state!:\n  this: {key}\n  status: \"ready\"\n  link: \"https://example.test/invite\"\n  account: {key}\n\nonboarding/agent-invite:\n  this: {key}\n  link: ?link\n  account: ?account\n"
            ),
            true,
        )
        .await
        .unwrap();
        assert_eq!(handoff.matches_after[0].results.len(), 1);
        drop(tonk);
        let subject = key.parse().unwrap();
        repository::remove_space_inner(&state, &subject)
            .await
            .unwrap();
        assert!(
            welcome(State(state)).await.unwrap().0.path.is_none(),
            "removing the welcome space must not reset first use"
        );
    }

    #[dialog_common::test]
    async fn resumes_a_pending_space_without_creating_another() {
        let state = crate::router::command::tests::native::test_state().await;
        let subject = {
            let tonk = state.write().await;
            let config =
                RepositoryConfiguration::default().branch("main", BranchConfiguration::default());
            let subject = repository::create_repository(&tonk, "Welcome to Tonk", &config)
                .await
                .unwrap()
                .did();
            let registry = tonk
                .registry
                .open_profile(&tonk.storage, tonk.registry.initial_profile())
                .await
                .unwrap();
            save(
                &tonk,
                &registry,
                &Progress {
                    subject: Some(subject.clone()),
                    profile: Some(tonk.profile_name.clone()),
                    complete: false,
                },
            )
            .await
            .unwrap();
            subject
        };
        let response = welcome(State(state.clone())).await.unwrap().0;
        assert_eq!(
            response.path,
            Some(format!("/space/{}", subject.repo_key()))
        );
        assert!(welcome(State(state)).await.unwrap().0.path.is_none());
    }

    #[dialog_common::test]
    async fn existing_spaces_do_not_trigger_onboarding() {
        let state = crate::router::command::tests::native::test_state().await;
        {
            let tonk = state.write().await;
            let config =
                RepositoryConfiguration::default().branch("main", BranchConfiguration::default());
            repository::create_repository(&tonk, "Existing work", &config)
                .await
                .unwrap();
        }
        assert!(welcome(State(state)).await.unwrap().0.path.is_none());
    }

    #[dialog_common::test]
    fn bundled_snapshot_has_welcome_and_no_governance_or_history() {
        let snapshot: Snapshot = serde_json::from_str(include_str!(
            "../../../tonk-core/assets/library/onboarding.yaml"
        ))
        .unwrap();
        assert!(
            snapshot
                .artifacts
                .iter()
                .any(|a| a.the.to_string() == "xyz.tonk.vault.welcome-page/heading")
        );
        for artifact in snapshot.artifacts {
            let attribute = artifact.the.to_string();
            assert!(
                ![
                    "dialog.ucan/",
                    "dialog.db/",
                    "xyz.tonk.membership/",
                    "xyz.tonk.invitation/",
                    "xyz.tonk.invitation-execution/",
                    "xyz.tonk.authorization/",
                    "xyz.tonk.transplant/",
                    "xyz.tonk.repo/"
                ]
                .iter()
                .any(|prefix| attribute.starts_with(prefix)),
                "{attribute}"
            );
        }
    }
}
