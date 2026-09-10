//! One local welcome space per browser, requested only by the root UI visit.
use axum::{
    Json,
    extract::{Path, State},
};
use axum_wasm_macros::wasm_compat;
use base64::Engine as _;
use dialog_artifacts::{Artifact, ArtifactSelector, Changes, Update};
use dialog_operator::Profile;
use dialog_query::{Output as _, Query, Term};
use dialog_repository::{Blob, RepositoryExt as _};
use futures_util::StreamExt as _;
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
    #[serde(default)]
    welcome_ready: bool,
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
        || progress.welcome_ready
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
    let seed = fetch_snapshot(SEED_URL).await?;
    let snapshot: Snapshot = serde_json::from_str(&seed).map_err(internal)?;
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
    // The marker is committed with the seed, so a crash before credential-save
    // cannot replay the snapshot (or the libraries) over subsequent edits.
    if !imported(&tonk, key, "welcome").await? {
        let scaffold = repository::fetch_standard_library("/library/core.yaml").await?;
        let name =
            repository::repository_name_body(&subject, "Welcome to Tonk").map_err(internal)?;
        repository::seed_standard_library(&tonk, key, "main", &format!("{scaffold}\n{name}"))
            .await?;
        let agent_library =
            repository::fetch_standard_library("/library/onboarding-agent.yaml").await?;
        repository::seed_standard_library(&tonk, key, "main", &agent_library).await?;
        import_snapshot(&tonk, key, snapshot, "welcome").await?;
    }
    repository::set_replica_status(&tonk, &subject, Replica::initialized_status())
        .await
        .map_err(internal)?;
    progress.welcome_ready = true;
    save(&tonk, &registry, &progress).await?;
    Ok(Json(WelcomeResponse {
        path: Some(format!("/space/{key}")),
    }))
}

async fn locally_mounted(tonk: &TonkState, key: &str) -> Result<bool, TonkWorkerError> {
    let session = tonk
        .reactor
        .profile_repository()
        .branch("main")
        .acquire(&tonk.operator)
        .await
        .map_err(internal)?;
    let replicas = session
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
    Ok(replicas
        .iter()
        .any(|replica| replica.subject.0.to_string() == key))
}

/// A branch-local marker shares the snapshot commit, unlike the credential
/// journal. It closes the crash window between data commit and journal save.
async fn imported(tonk: &TonkState, key: &str, shard: &str) -> Result<bool, TonkWorkerError> {
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
    let stream = branch
        .claims()
        .select(
            ArtifactSelector::new()
                .the("xyz.tonk.onboarding/imported".parse().map_err(internal)?)
                .of(format!("id:tonk/onboarding-v2/{shard}")
                    .parse()
                    .map_err(internal)?),
        )
        .perform(&tonk.operator)
        .await
        .map_err(internal)?;
    tokio::pin!(stream);
    Ok(stream.next().await.transpose().map_err(internal)?.is_some())
}

async fn import_snapshot(
    tonk: &TonkState,
    key: &str,
    snapshot: Snapshot,
    shard: &str,
) -> Result<(), TonkWorkerError> {
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
        import_blob(tonk, &branch, &blob.entity, bytes).await?;
    }
    // A user/agent can write an optional entity before its first import. Keep
    // every existing (attribute, entity) pair, including cardinality-many sets;
    // late seed assertions must not replace or merge into those edits.
    let mut existing = std::collections::BTreeSet::new();
    if shard == "demos" {
        let stream = branch
            .claims()
            .select(ArtifactSelector::new().of_starting_with(""))
            .perform(&tonk.operator)
            .await
            .map_err(internal)?;
        tokio::pin!(stream);
        while let Some(row) = stream.next().await {
            let row = row.map_err(internal)?;
            existing.insert((
                row.the_bytes().map_err(internal)?.into_owned(),
                row.of_bytes().map_err(internal)?.into_owned(),
            ));
        }
    }
    let mut changes = Changes::new();
    for artifact in snapshot.artifacts {
        if !existing.contains(&(
            artifact.the.as_str().as_bytes().to_vec(),
            artifact.of.as_str().as_bytes().to_vec(),
        )) {
            changes.associate(artifact.the, artifact.of, artifact.is);
        }
    }
    changes.associate(
        "xyz.tonk.onboarding/imported".parse().map_err(internal)?,
        format!("id:tonk/onboarding-v2/{shard}")
            .parse()
            .map_err(internal)?,
        dialog_artifacts::Value::Boolean(true),
    );
    tonk.reactor
        .repository(key)
        .branch("main")
        .transaction()
        .assert(changes)
        .commit()
        .perform(&tonk.operator)
        .await
        .map_err(internal)?;
    tonk.reactor.run_scheduled_polls(&tonk.operator).await;
    Ok(())
}

/// One optional shard, requested after Welcome paints or pulled forward by
/// navigation. Scope and profile checks also make this a no-op in other spaces,
/// old fully-seeded spaces and copies of the authored application.
#[wasm_compat]
pub async fn prepare(
    State(state): State<AppState>,
    Path(path): Path<super::evaluate::EvaluatePath>,
) -> Result<Json<bool>, TonkWorkerError> {
    let tonk = state.write().await;
    let registry = tonk
        .registry
        .open_profile(&tonk.storage, tonk.registry.initial_profile())
        .await?;
    let bytes = match registry
        .credential()
        .site(JOURNAL)
        .load::<Vec<u8>>()
        .perform(&tonk.storage)
        .await
    {
        Ok(bytes) => bytes,
        Err(error) if crate::credential::is_missing(&error) => return Ok(Json(true)),
        Err(error) => return Err(internal(error)),
    };
    let mut progress: Progress = serde_json::from_slice(&bytes).map_err(internal)?;
    if progress.complete
        || !progress.welcome_ready
        || path.branch != "main"
        || progress.profile.as_deref() != Some(&tonk.profile_name)
        || progress
            .subject
            .as_ref()
            .is_none_or(|subject| subject.repo_key() != path.repo)
    {
        return Ok(Json(true));
    }
    // Removal must not recreate the space or import late content into it.
    if !locally_mounted(&tonk, &path.repo).await? {
        return Ok(Json(true));
    }
    if !imported(&tonk, &path.repo, "demos").await? {
        let seed = fetch_snapshot("/library/onboarding-demos.yaml").await?;
        let snapshot = serde_json::from_str(&seed).map_err(internal)?;
        import_snapshot(&tonk, &path.repo, snapshot, "demos").await?;
    }
    let repository = tonk
        .profile
        .repository(&path.repo)
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
    for media in bundled_media()? {
        ensure_media(&tonk, &branch, &media.entity).await?;
    }
    progress.complete = true;
    save(&tonk, &registry, &progress).await?;
    Ok(Json(true))
}

#[derive(Deserialize)]
struct Media {
    entity: dialog_artifacts::Entity,
    url: String,
}

fn bundled_media() -> Result<Vec<Media>, TonkWorkerError> {
    serde_json::from_str(include_str!(
        "../../../tonk-core/assets/library/onboarding-media.json"
    ))
    .map_err(internal)
}

async fn import_blob(
    tonk: &TonkState,
    branch: &dialog_repository::Branch,
    expected: &dialog_artifacts::Entity,
    bytes: Vec<u8>,
) -> Result<(), TonkWorkerError> {
    // Verify before writing anything, including the branch's blob index.
    if &dialog_artifacts::Entity::from_blob(blake3::hash(&bytes).as_bytes()).map_err(internal)?
        != expected
    {
        return Err(internal("bundled blob hash mismatch"));
    }
    let chunks = futures_util::stream::iter(vec![Ok::<_, dialog_effects::blob::BlobError>(bytes)]);
    let entity = Blob::import(chunks)
        .write(branch.blobs())
        .perform(&tonk.operator)
        .await
        .map_err(internal)?;
    if &entity != expected {
        return Err(internal("bundled blob hash mismatch"));
    }
    Ok(())
}

pub(super) async fn hydrate_media(
    state: &AppState,
    key: &str,
    branch_name: &str,
    entity: &dialog_artifacts::Entity,
) -> Result<(), TonkWorkerError> {
    if !bundled_media()?.iter().any(|media| &media.entity == entity) {
        return Ok(());
    }
    // Blob imports advance the branch index; serialize with other writer routes.
    let tonk = state.write().await;
    let repository = tonk
        .profile
        .repository(key)
        .load()
        .perform(&tonk.operator)
        .await
        .map_err(internal)?;
    let branch = repository
        .branch(branch_name)
        .open()
        .perform(&tonk.operator)
        .await
        .map_err(internal)?;
    ensure_media(&tonk, &branch, entity).await?;
    Ok(())
}

/// Only this immutable, compiled-in allowlist can fill a missing starter image.
/// The caller has already resolved its authorized repository and branch. Bytes
/// become ordinary branch blobs, so export/sync and later offline reads work.
pub(super) async fn ensure_media(
    tonk: &TonkState,
    branch: &dialog_repository::Branch,
    entity: &dialog_artifacts::Entity,
) -> Result<bool, TonkWorkerError> {
    let Some(media) = bundled_media()?
        .into_iter()
        .find(|media| &media.entity == entity)
    else {
        return Ok(false);
    };
    match Blob::from(entity.clone())
        .read(branch.blobs())
        .perform(&tonk.operator)
        .await
    {
        Ok(_) => return Ok(true),
        Err(dialog_repository::CommitError::Blob(dialog_effects::blob::BlobError::NotFound(_))) => {
        }
        Err(error) => return Err(internal(error)),
    }
    let bytes = fetch_media(&media.url).await?;
    import_blob(tonk, branch, entity, bytes).await?;
    Ok(true)
}

async fn fetch_snapshot(url: &str) -> Result<String, TonkWorkerError> {
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    {
        repository::fetch_standard_library(url).await
    }
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    {
        String::from_utf8(fetch_media(url).await?).map_err(internal)
    }
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn fetch_media(url: &str) -> Result<Vec<u8>, TonkWorkerError> {
    use wasm_bindgen::JsCast as _;
    use wasm_bindgen_futures::JsFuture;
    #[wasm_bindgen::prelude::wasm_bindgen]
    extern "C" {
        #[wasm_bindgen(catch, js_name = tonkBundledAsset)]
        async fn bundled_asset(path: &str) -> Result<wasm_bindgen::JsValue, wasm_bindgen::JsValue>;
    }
    let response: web_sys::Response = bundled_asset(url)
        .await
        .and_then(|v| v.dyn_into())
        .map_err(|e| internal(format!("fetch {url}: {e:?}")))?;
    if !response.ok() {
        return Err(internal(format!("fetch {url}: HTTP {}", response.status())));
    }
    let bytes = JsFuture::from(
        response
            .array_buffer()
            .map_err(|e| internal(format!("media body: {e:?}")))?,
    )
    .await
    .map_err(|e| internal(format!("media body: {e:?}")))?;
    Ok(js_sys::Uint8Array::new(&bytes).to_vec())
}

#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
async fn fetch_media(url: &str) -> Result<Vec<u8>, TonkWorkerError> {
    match url {
        "/library/welcome-CmZq5URy2ucFKNyZCFUtHiNvEaUNK4Z8UstvzwbjREFG.webp" => Ok(include_bytes!("../../../tonk-core/assets/library/welcome-CmZq5URy2ucFKNyZCFUtHiNvEaUNK4Z8UstvzwbjREFG.webp").to_vec()),
        "/library/welcome-9hKHdfALCDKRL5z3Xkn2JUM72DWSzsSBwAvdbyaPF2sU.webp" => Ok(include_bytes!("../../../tonk-core/assets/library/welcome-9hKHdfALCDKRL5z3Xkn2JUM72DWSzsSBwAvdbyaPF2sU.webp").to_vec()),
        _ => Err(internal("unknown bundled media")),
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    async fn finish(state: AppState, key: &str) {
        let _ = prepare(
            State(state),
            Path(super::super::evaluate::EvaluatePath {
                repo: key.to_owned(),
                branch: "main".to_owned(),
            }),
        )
        .await
        .unwrap();
    }

    async fn values(tonk: &TonkState, key: &str, attribute: &str) -> Vec<Artifact> {
        let repository = tonk
            .profile
            .repository(key)
            .load()
            .perform(&tonk.operator)
            .await
            .unwrap();
        let branch = repository
            .branch("main")
            .open()
            .perform(&tonk.operator)
            .await
            .unwrap();
        let rows = branch
            .claims()
            .select(ArtifactSelector::new().the(attribute.parse().unwrap()))
            .perform(&tonk.operator)
            .await
            .unwrap();
        rows.map(|row| row.unwrap().to_owned().unwrap())
            .collect()
            .await
    }

    #[dialog_common::test]
    async fn welcome_is_usable_before_demos_and_completion_never_replays_edits() {
        let state = crate::router::command::tests::native::test_state().await;
        let path = welcome(State(state.clone())).await.unwrap().0.path.unwrap();
        let key = path.strip_prefix("/space/").unwrap();
        {
            let tonk = state.read().await;
            assert!(imported(&tonk, key, "welcome").await.unwrap());
            assert!(!imported(&tonk, key, "demos").await.unwrap());
            assert_eq!(
                values(&tonk, key, "xyz.tonk.component/module").await.len(),
                7
            );
            assert!(
                values(&tonk, key, "xyz.tonk.demo.zork/heading")
                    .await
                    .is_empty()
            );
            let mut edit = Changes::new();
            edit.associate(
                "xyz.tonk.vault.welcome-page/heading".parse().unwrap(),
                "did:key:z6MkAMKLz5uDbduG95r9rQriwRit6DE4MAbBSmWqpUz8PxWB"
                    .parse()
                    .unwrap(),
                dialog_artifacts::Value::String("My Welcome".into()),
            );
            edit.associate(
                "xyz.tonk.demo.zork/heading".parse().unwrap(),
                "did:key:z6MkEL6MU8V23gFmRSHcENLEnGnaYpEQFCS6LQWSp4S4MEk5"
                    .parse()
                    .unwrap(),
                dialog_artifacts::Value::String("My early demo edit".into()),
            );
            tonk.reactor
                .repository(key)
                .branch("main")
                .transaction()
                .assert(edit)
                .commit()
                .perform(&tonk.operator)
                .await
                .unwrap();
        }
        let ((), ()) = futures_util::join!(finish(state.clone(), key), finish(state.clone(), key));
        {
            let tonk = state.read().await;
            assert!(imported(&tonk, key, "demos").await.unwrap());
            assert_eq!(
                values(&tonk, key, "xyz.tonk.component/module").await.len(),
                26
            );
            let mut edit = Changes::new();
            edit.associate(
                "xyz.tonk.component/module".parse().unwrap(),
                "id:zork/component/terminal".parse().unwrap(),
                dialog_artifacts::Value::String("user module".into()),
            );
            tonk.reactor
                .repository(key)
                .branch("main")
                .transaction()
                .assert(edit)
                .commit()
                .perform(&tonk.operator)
                .await
                .unwrap();
            // Simulate interruption after both commits but before journal saves.
            let registry = tonk
                .registry
                .open_profile(&tonk.storage, tonk.registry.initial_profile())
                .await
                .unwrap();
            save(
                &tonk,
                &registry,
                &Progress {
                    subject: Some(key.parse().unwrap()),
                    profile: Some(tonk.profile_name.clone()),
                    complete: false,
                    welcome_ready: false,
                },
            )
            .await
            .unwrap();
        }
        assert!(
            welcome(State(state.clone()))
                .await
                .unwrap()
                .0
                .path
                .is_some()
        );
        finish(state.clone(), key).await;
        let tonk = state.read().await;
        assert_eq!(
            values(&tonk, key, "xyz.tonk.demo.zork/heading").await[0].is,
            dialog_artifacts::Value::String("My early demo edit".into())
        );
        assert_eq!(
            values(&tonk, key, "xyz.tonk.vault.welcome-page/heading").await[0].is,
            dialog_artifacts::Value::String("My Welcome".into())
        );
        assert!(
            values(&tonk, key, "xyz.tonk.component/module")
                .await
                .iter()
                .any(|a| a.of.to_string() == "id:zork/component/terminal"
                    && a.is == dialog_artifacts::Value::String("user module".into()))
        );
    }

    #[dialog_common::test]
    async fn bundled_images_are_hash_checked_and_imported_only_on_demand() {
        let state = crate::router::command::tests::native::test_state().await;
        let path = welcome(State(state.clone())).await.unwrap().0.path.unwrap();
        let key = path.strip_prefix("/space/").unwrap();
        let tonk = state.read().await;
        let repository = tonk
            .profile
            .repository(key)
            .load()
            .perform(&tonk.operator)
            .await
            .unwrap();
        let branch = repository
            .branch("main")
            .open()
            .perform(&tonk.operator)
            .await
            .unwrap();
        for media in bundled_media().unwrap() {
            assert!(
                Blob::from(media.entity.clone())
                    .read(branch.blobs())
                    .perform(&tonk.operator)
                    .await
                    .is_err()
            );
            assert!(
                import_blob(&tonk, &branch, &media.entity, b"wrong bytes".to_vec())
                    .await
                    .is_err()
            );
            assert!(ensure_media(&tonk, &branch, &media.entity).await.unwrap());
            let mut reader = Blob::from(media.entity)
                .read(branch.blobs())
                .perform(&tonk.operator)
                .await
                .unwrap();
            let mut bytes = Vec::new();
            while let Some(chunk) = reader.next().await.unwrap() {
                bytes.extend_from_slice(&chunk);
            }
            assert_eq!(bytes, fetch_media(&media.url).await.unwrap());
        }
    }

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
        finish(state.clone(), key).await;
        assert!(!locally_mounted(&*state.read().await, key).await.unwrap());
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
                    welcome_ready: false,
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
        let deferred: Snapshot = serde_json::from_str(include_str!(
            "../../../tonk-core/assets/library/onboarding-demos.yaml"
        ))
        .unwrap();
        for artifact in snapshot.artifacts.into_iter().chain(deferred.artifacts) {
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
