//! Copy current content into a new identity, without adopting source history.
use super::*;
use dialog_artifacts::{ArtifactSelector, BlobIndexExt as _, Changes, Update};
use dialog_repository::{Blob, Index, NetworkedIndex, RemoteFallback, Snapshot};
use futures_util::StreamExt as _;

fn error(error: impl std::fmt::Display) -> RepositoryError {
    RepositoryError::Internal(format!("duplicate space: {error}"))
}

pub(super) struct Copy {
    snapshot: Snapshot,
    content: Changes,
    blobs: Vec<dialog_artifacts::Entity>,
}

// These are space lifecycle records, even when stored on main. Definitions
// describing their attributes are ordinary application content and remain.
//
// Everything under `dialog.` is dialog's own record, which an application
// commit may not write: a revision, or an asset's size, which the copy
// records again by importing the asset's bytes.
fn is_metadata(attribute: &str) -> bool {
    attribute.starts_with("dialog.")
        || [
            "xyz.tonk.repo/",
            "xyz.tonk.membership/",
            "xyz.tonk.invitation/",
            "xyz.tonk.invitation-execution/",
            "xyz.tonk.credential/",
            "xyz.tonk.authorization/",
            "xyz.tonk.secret/",
            "xyz.tonk.seed/",
            "xyz.tonk.transplant/",
            "xyz.tonk.replica/",
            "xyz.tonk.branch/",
            "xyz.tonk.remote/",
            "xyz.tonk.remote-execution/",
            "xyz.tonk.agent-handoff/",
        ]
        .iter()
        .any(|prefix| attribute.starts_with(prefix))
}

pub(super) async fn prepare(tonk: &TonkState, source: &str) -> Result<Copy, RepositoryError> {
    // Loading an existing repository must succeed; do not manufacture an empty
    // source for a typo, an unmounted space, or an inaccessible subject.
    let repository = tonk
        .profile
        .space(source)
        .load()
        .perform(&tonk.operator)
        .await
        .map_err(error)?;
    require_real_space(tonk, &repository.did())
        .await
        .map_err(error)?;
    let session = tonk
        .reactor
        .repository(source)
        .branch(CONTENT_BRANCH)
        .acquire(&tonk.operator)
        .await
        .map_err(error)?;
    let branch = session.handle();
    let snapshot = branch
        .snapshot()
        .ok_or_else(|| error("source main branch is empty"))?;
    // Materialize the pinned revision before reading it. A pulled branch may
    // only have its root locally; failure must precede destination creation.
    let mut export = snapshot.clone().export();
    if let Some(Upstream::Remote { remote, .. }) = tonk_account::peer::upstream(branch) {
        export = export.download(remote);
    }
    let stream = export.perform(&tonk.operator);
    tokio::pin!(stream);
    while let Some(item) = stream.next().await {
        item.map_err(error)?;
    }

    collect(tonk, &snapshot).await
}

/// Read what a copy takes from `snapshot`: its content without the records of
/// the space it was taken from, and every blob it holds. Every block the
/// snapshot reaches has to be in this worker's store already.
pub(super) async fn collect(
    tonk: &TonkState,
    snapshot: &Snapshot,
) -> Result<Copy, RepositoryError> {
    let stream = snapshot
        .claims()
        .select(ArtifactSelector::new().of_starting_with(""))
        .perform(&tonk.operator)
        .await
        .map_err(error)?;
    tokio::pin!(stream);
    let mut content = Changes::new();
    // The blobs to copy: every asset the branch records, and every blob a
    // tree written before assets recorded in the blob index.
    let mut blobs = Vec::new();
    while let Some(artifact) = stream.next().await {
        let artifact = artifact.map_err(error)?.to_owned().map_err(error)?;
        if artifact.the.as_str() == dialog_artifacts::ASSET_SIZE {
            blobs.push(artifact.of.clone());
        }
        if !is_metadata(artifact.the.as_str()) {
            content.associate(artifact.the, artifact.of, artifact.is);
        }
    }
    let store = NetworkedIndex::new(
        &tonk.operator,
        snapshot.archive().index(),
        RemoteFallback::None,
    );
    let stream = Index::from_hash((*snapshot.revision().tree.hash()).into()).list_blobs(store);
    tokio::pin!(stream);
    while let Some(blob) = stream.next().await {
        let (hash, _) = blob.map_err(error)?;
        let entity = dialog_artifacts::Entity::from_blob(&hash).map_err(error)?;
        if !blobs.contains(&entity) {
            blobs.push(entity);
        }
    }
    Ok(Copy {
        snapshot: snapshot.clone(),
        content,
        blobs,
    })
}

pub(super) async fn create(
    state: &AppState,
    name: &str,
    copy: Copy,
) -> Result<String, RepositoryError> {
    let tonk = state.write().await;
    let configuration =
        RepositoryConfiguration::default().branch(CONTENT_BRANCH, BranchConfiguration::default());
    let repository = create_repository(&tonk, name, &configuration).await?;
    let subject = repository.did();
    let key = subject.repo_key();
    write(&tonk, &subject, name, copy).await?;
    write_replica_status(&tonk, &subject, Replica::initialized_status(), None).await?;
    Ok(key.to_owned())
}

/// Write `copy` onto `subject`'s content branch, under `name`: its blobs,
/// then its content and the new space's own name in one commit.
pub(super) async fn write(
    tonk: &TonkState,
    subject: &Did,
    name: &str,
    copy: Copy,
) -> Result<(), RepositoryError> {
    let key = subject.repo_key();
    let session = tonk
        .reactor
        .repository(key)
        .branch(CONTENT_BRANCH)
        .acquire(&tonk.operator)
        .await
        .map_err(error)?;
    for entity in copy.blobs {
        let bytes = Blob::from(entity.clone())
            .read(copy.snapshot.blobs())
            .perform(&tonk.operator)
            .await
            .map_err(error)?;
        let chunks = futures_util::stream::try_unfold(bytes, |mut reader| async move {
            Ok(reader.next().await?.map(|chunk| (chunk, reader)))
        });
        tokio::pin!(chunks);
        let copied = Blob::import(chunks)
            .write(session.handle().blobs())
            .perform(&tonk.operator)
            .await
            .map_err(error)?;
        if copied != entity {
            return Err(error("blob identity changed"));
        }
    }
    // No standard-library seed: that could overwrite copied definitions/routes.
    tonk.reactor
        .repository(key)
        .branch(CONTENT_BRANCH)
        .transaction()
        .assert(copy.content)
        .assert(RepositoryName {
            this: subject.this(),
            name: tonk_schema::domain::repo::Name(name.to_owned()),
        })
        .commit()
        .perform(&tonk.operator)
        .await
        .map_err(error)?;
    Ok(())
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use dialog_artifacts::{Artifact, Entity, Value};

    async fn facts(tonk: &TonkState, key: &str, branch: &str) -> Vec<Artifact> {
        let session = tonk
            .reactor
            .repository(key)
            .branch(branch)
            .acquire(&tonk.operator)
            .await
            .unwrap();
        let stream = session
            .handle()
            .claims()
            .select(ArtifactSelector::new().of_starting_with(""))
            .perform(&tonk.operator)
            .await
            .unwrap();
        tokio::pin!(stream);
        let mut facts = Vec::new();
        while let Some(row) = stream.next().await {
            facts.push(row.unwrap().to_owned().unwrap());
        }
        facts
    }

    #[dialog_common::test]
    async fn it_duplicates_main_content_and_blobs_with_fresh_identity_and_metadata() {
        let state = crate::router::command::tests::native::test_state().await;
        let source = create_space_inner(&state, "Original", Some("Old description"))
            .await
            .unwrap();
        let blob;
        let prepared;
        let before;
        {
            let tonk = state.read().await;
            let branch = tonk
                .reactor
                .repository(&source)
                .branch(CONTENT_BRANCH)
                .acquire(&tonk.operator)
                .await
                .unwrap();
            blob = Blob::import(futures_util::stream::iter([Ok::<
                _,
                dialog_effects::blob::BlobError,
            >(
                b"image bytes".to_vec()
            )]))
            .write(branch.handle().blobs())
            .perform(&tonk.operator)
            .await
            .unwrap();
            let mut changes = Changes::new();
            for (attribute, entity, value) in [
                (
                    "example/text",
                    "id:note",
                    Value::String("Original text".into()),
                ),
                ("example/attachment", "id:note", Value::Entity(blob.clone())),
                ("example/binary", "id:note", Value::Bytes(vec![0, 255, 42])),
                (
                    "xyz.tonk.membership/member",
                    "id:old-member",
                    Value::Entity("did:key:zOldMember".parse().unwrap()),
                ),
                (
                    "xyz.tonk.authorization/proof",
                    "id:old-proof",
                    Value::String("source delegation".into()),
                ),
                (
                    "xyz.tonk.invitation/audience",
                    "id:invite",
                    Value::String("old grant".into()),
                ),
            ] {
                changes.associate(attribute.parse().unwrap(), entity.parse().unwrap(), value);
            }
            tonk.reactor
                .repository(&source)
                .branch(CONTENT_BRANCH)
                .transaction()
                .assert(changes)
                .commit()
                .perform(&tonk.operator)
                .await
                .unwrap();
            let mut side = Changes::new();
            side.associate(
                "example/private".parse().unwrap(),
                "id:other".parse().unwrap(),
                Value::Boolean(true),
            );
            tonk.reactor
                .repository(&source)
                .branch("draft")
                .transaction()
                .assert(side)
                .commit()
                .perform(&tonk.operator)
                .await
                .unwrap();
            before = facts(&tonk, &source, CONTENT_BRANCH).await;
            prepared = prepare(&tonk, &source).await.unwrap();
        }
        let destination = create(&state, "Copy of Original", prepared).await.unwrap();
        assert_ne!(destination, source);
        let tonk = state.read().await;
        let copied = facts(&tonk, &destination, CONTENT_BRANCH).await;
        for fact in before.iter().filter(|fact| !is_metadata(fact.the.as_str())) {
            assert!(
                copied
                    .iter()
                    .any(|copy| copy.the == fact.the && copy.of == fact.of && copy.is == fact.is),
                "missing {fact:?}"
            );
        }
        assert!(
            !copied.iter().any(|fact| fact.of.as_str() == "id:old-member"
                || fact.of.as_str() == "id:invite"
                || fact.of.as_str() == "id:old-proof"
                || fact.the.as_str() == "example/private")
        );
        assert!(
            !copied
                .iter()
                .any(|fact| fact.the.as_str().starts_with("xyz.tonk.seed/"))
        );
        assert!(
            !copied
                .iter()
                .any(|fact| fact.the.as_str() == "xyz.tonk.repo/description")
        );
        let members: Vec<_> = copied
            .iter()
            .filter(|fact| fact.the.as_str() == "xyz.tonk.membership/subject")
            .collect();
        assert_eq!(members.len(), 1);
        assert_eq!(members[0].is, Value::Entity(destination.parse().unwrap()));
        assert!(
            copied
                .iter()
                .any(|fact| fact.the.as_str() == "xyz.tonk.repo/name"
                    && fact.is == Value::String("Copy of Original".into()))
        );
        let meta = facts(&tonk, &destination, META_BRANCH).await;
        assert!(
            !meta
                .iter()
                .any(|fact| fact.is == Value::String("draft".into())
                    || fact.the.as_str().starts_with("xyz.tonk.remote/"))
        );
        let branch = tonk
            .reactor
            .repository(&destination)
            .branch(CONTENT_BRANCH)
            .acquire(&tonk.operator)
            .await
            .unwrap();
        let mut reader = Blob::from(blob)
            .read(branch.handle().blobs())
            .perform(&tonk.operator)
            .await
            .unwrap();
        let mut bytes = Vec::new();
        while let Some(chunk) = reader.next().await.unwrap() {
            bytes.extend(chunk);
        }
        assert_eq!(bytes, b"image bytes");
        let mut edit = Changes::new();
        edit.associate(
            "example/text".parse().unwrap(),
            "id:copy-only".parse::<Entity>().unwrap(),
            Value::String("Independent".into()),
        );
        tonk.reactor
            .repository(&destination)
            .branch(CONTENT_BRANCH)
            .transaction()
            .assert(edit)
            .commit()
            .perform(&tonk.operator)
            .await
            .unwrap();
        let after = facts(&tonk, &source, CONTENT_BRANCH).await;
        assert_eq!(
            before, after,
            "duplicating and editing the copy must leave the source unchanged"
        );
    }

    #[dialog_common::test]
    async fn it_refuses_a_missing_source_before_creating_a_space() {
        let state = crate::router::command::tests::native::test_state().await;
        let tonk = state.read().await;
        assert!(prepare(&tonk, "did:key:zMissing").await.is_err());
    }
}
