//! Review-only adversarial regressions. Assertions state the required behavior.
#![cfg(not(target_arch = "wasm32"))]

use dialog_artifacts::Entity;
use dialog_capability::{Capability, Command, Provider};
use dialog_effects::memory::{Resolve, Version};
use dialog_operator::helpers::{test_operator_with_profile, test_repo};
use dialog_query::the;
use std::any::Any;
use std::sync::Mutex;
use tonk_document::{
    CellError, Document, DocumentEnv, Edit, Format, LocalCell, Stamp, Transport, cell, formula,
    session, sync_pass,
};

fn set(text: &str) -> Vec<Edit> {
    vec![Edit::SetText { text: text.into() }]
}
fn body(snapshot: session::Snapshot) -> String {
    match snapshot.content {
        tonk_document::Content::Text(text) => text,
        _ => panic!("text expected"),
    }
}

#[test]
fn review_text_diff_positions_refer_to_the_from_version() -> anyhow::Result<()> {
    let mut doc = Document::from_text("ab")?;
    let before = doc.store_heads();
    let after = doc.edit_all(&before, &Stamp::default(), &set("XabY"))?;
    let operations = doc.diff(&before, &after)?;
    assert_eq!(
        operations,
        vec![
            tonk_document::DiffOp::Insert {
                at: 0,
                text: "X".into()
            },
            tonk_document::DiffOp::Insert {
                at: 2,
                text: "Y".into()
            },
        ],
        "the second position must not be shifted by the first insertion"
    );
    Ok(())
}

#[test]
fn diffs_apply_in_from_coordinates_across_replacements_and_unicode() -> anyhow::Result<()> {
    let samples = [
        "",
        "ab",
        "XabY",
        "a🙂bc🚀d",
        "🙂🚀",
        "aaa",
        "bbb",
        "abcabc",
        "cab",
        "X🙂aY🚀",
    ];
    for from in samples {
        for to in samples {
            let mut doc = Document::from_text(from)?;
            let before = doc.store_heads();
            let after = doc.edit_all(&before, &Stamp::default(), &set(to))?;
            let mut output = from.encode_utf16().collect::<Vec<_>>();
            // Apply in reverse, so indices all still refer to the original.
            for operation in doc.diff(&before, &after)?.into_iter().rev() {
                match operation {
                    tonk_document::DiffOp::Insert { at, text } => {
                        output.splice(at..at, text.encode_utf16());
                    }
                    tonk_document::DiffOp::Delete { at, length } => {
                        output.drain(at..at + length);
                    }
                    _ => panic!("text diff expected"),
                }
            }
            assert_eq!(String::from_utf16(&output)?, to, "{from:?} -> {to:?}");
        }
    }
    Ok(())
}

#[test]
fn identified_requests_converge_even_when_replayed_on_independent_replicas() -> anyhow::Result<()> {
    let mut initial = Document::from_text("base")?;
    let heads = initial.store_heads();
    let bytes = initial.save();
    let mut left = Document::load(&bytes)?;
    let mut right = Document::load(&bytes)?;
    let request = tonk_document::engine::EditRequest {
        id: "same-gesture".into(),
        time: 123,
    };
    let first = left.edit_request(
        &heads,
        &Stamp {
            author: Some("did:example:alice".into()),
            time: 124,
        },
        &set("base hello"),
        &request,
    )?;
    let second = right.edit_request(
        &heads,
        &Stamp {
            author: Some("did:example:alice".into()),
            time: 999,
        },
        &set("base hello"),
        &request,
    )?;
    assert_eq!(first, second);
    left.merge(&right.save())?;
    let merged = left.store_heads();
    assert_eq!(left.text(&merged)?, "base hello");
    Ok(())
}

#[derive(Default)]
struct Memory {
    state: Mutex<Option<(Vec<u8>, u64)>>,
    race: Mutex<Option<Vec<u8>>>,
}

#[async_trait::async_trait]
impl Transport for Memory {
    async fn resolve(&self) -> Result<Option<(Vec<u8>, Version)>, CellError> {
        Ok(self
            .state
            .lock()
            .unwrap()
            .as_ref()
            .map(|(b, v)| (b.clone(), Version::from(v.to_string()))))
    }
    async fn publish(&self, bytes: Vec<u8>, when: Option<Version>) -> Result<Version, CellError> {
        let mut state = self.state.lock().unwrap();
        if let Some(bytes) = self.race.lock().unwrap().take() {
            let next = state.as_ref().map_or(1, |(_, v)| v + 1);
            *state = Some((bytes, next));
        }
        if state.as_ref().map(|(_, v)| Version::from(v.to_string())) != when {
            return Err(CellError::Conflict);
        }
        let next = state.as_ref().map_or(1, |(_, v)| v + 1);
        *state = Some((bytes, next));
        Ok(Version::from(next.to_string()))
    }
}

#[tokio::test]
async fn review_sync_keeps_a_write_racing_the_local_save_dirty() -> anyhow::Result<()> {
    let mut base = Document::from_text("base")?;
    let heads = base.store_heads();
    let local = Memory::default();
    let remote = Memory::default();
    let marker = Memory::default();
    local.publish(base.save(), None).await?;
    let mut other = Document::load(&base.save())?;
    other.edit_all(&heads, &Stamp::default(), &set("base remote"))?;
    remote.publish(other.save(), None).await?;
    base.edit_all(&heads, &Stamp::default(), &set("local base"))?;
    *local.race.lock().unwrap() = Some(base.save());
    for _ in 0..4 {
        sync_pass(&local, &remote, &marker).await?;
    }
    let (mut mine, _) = cell::load(&local).await?.unwrap();
    let (mut theirs, _) = cell::load(&remote).await?.unwrap();
    assert_eq!(
        mine.store_heads(),
        theirs.store_heads(),
        "a concurrent local edit must eventually reach the remote"
    );
    Ok(())
}

struct RaceEnv<E> {
    inner: E,
    subject: dialog_capability::Subject,
    entity: Entity,
    inject: Mutex<Option<Vec<u8>>>,
}

#[async_trait::async_trait]
impl<C, E> Provider<C> for RaceEnv<E>
where
    C: Command + 'static,
    C::Input: Send + 'static,
    C::Output: Send + 'static,
    E: Provider<C> + DocumentEnv,
{
    async fn execute(&self, input: C::Input) -> C::Output {
        let is_document_read = (&input as &dyn Any)
            .downcast_ref::<Capability<Resolve>>()
            .is_some_and(|cap| format!("{cap:?}").contains(&cell::space(&self.entity)));
        let result = <E as Provider<C>>::execute(&self.inner, input).await;
        let injection = if is_document_read {
            self.inject.lock().unwrap().take()
        } else {
            None
        };
        if let Some(bytes) = injection {
            let local = LocalCell::new(&self.subject, &self.entity, &self.inner);
            let (_, version) = local.resolve().await.unwrap().unwrap();
            local.publish(bytes, Some(version)).await.unwrap();
        }
        result
    }
}

#[tokio::test]
async fn review_raw_bytes_merge_cannot_erase_a_concurrent_cell_write() -> anyhow::Result<()> {
    let (operator, profile) = test_operator_with_profile().await;
    let repo = test_repo(&operator, &profile).await;
    let branch = repo.branch("main").open().perform(&operator).await?;
    let entity: Entity = "id:review/raw".parse()?;
    let written = session::write(
        &branch,
        &entity,
        Some(Format::Text),
        None,
        &set("base"),
        &Stamp::default(),
        &operator,
    )
    .await?;
    let bytes = session::bytes(&branch, &entity, &operator).await?;
    let mut incoming = Document::load(&bytes)?;
    incoming.edit_all(&written.local, &Stamp::default(), &set("base incoming"))?;
    let mut concurrent = Document::load(&bytes)?;
    let concurrent_heads =
        concurrent.edit_all(&written.local, &Stamp::default(), &set("concurrent base"))?;
    let env = RaceEnv {
        inner: operator,
        subject: branch.subject(),
        entity: entity.clone(),
        inject: Mutex::new(Some(concurrent.save())),
    };
    session::merge_bytes(&branch, &entity, &incoming.save(), &env).await?;
    assert!(env.inject.lock().unwrap().is_none(), "race was exercised");
    let mut saved = Document::load(&session::bytes(&branch, &entity, &env.inner).await?)?;
    assert!(
        saved.missing(&concurrent_heads)?.is_empty(),
        "raw bytes write erased the concurrent change from the object store"
    );
    Ok(())
}

#[tokio::test]
async fn review_each_legacy_branch_converts_its_own_body() -> anyhow::Result<()> {
    let (operator, profile) = test_operator_with_profile().await;
    let repo = test_repo(&operator, &profile).await;
    let main = repo.branch("main").open().perform(&operator).await?;
    let other = repo.branch("other").open().perform(&operator).await?;
    let entity: Entity = "id:review/legacy".parse()?;
    for (branch, text) in [(&main, "main body"), (&other, "other body")] {
        branch
            .transaction()
            .assert(
                the!("io.gozala.prose/content")
                    .of(entity.clone())
                    .is(text.to_string()),
            )
            .commit()
            .publish()
            .perform(&operator)
            .await?;
    }
    session::adopt_legacy(&main, &operator).await?;
    session::adopt_legacy(&other, &operator).await?;
    assert_eq!(
        body(session::read(&other, &entity, Some(Format::Text), &operator).await?),
        "other body"
    );
    Ok(())
}

#[tokio::test]
async fn review_retry_after_lost_reply_does_not_duplicate_text() -> anyhow::Result<()> {
    let (operator, profile) = test_operator_with_profile().await;
    let repo = test_repo(&operator, &profile).await;
    let branch = repo.branch("main").open().perform(&operator).await?;
    let entity: Entity = "id:review/retry".parse()?;
    let before = session::read(&branch, &entity, Some(Format::Text), &operator).await?;
    // The first request commits, but its reply never reaches the element.
    let request = tonk_document::engine::EditRequest {
        id: "gesture-1".into(),
        time: 17,
    };
    session::write_request(
        &branch,
        &entity,
        None,
        &before.heads,
        &set("hello"),
        &Stamp::default(),
        &request,
        &operator,
    )
    .await?;
    // Element retries with the same known heads and text, as DocumentSession does.
    let after = session::write_request(
        &branch,
        &entity,
        None,
        &before.heads,
        &set("hello"),
        &Stamp::default(),
        &request,
        &operator,
    )
    .await?;
    assert_eq!(body(after.snapshot), "hello");
    Ok(())
}

#[tokio::test]
async fn review_versions_heads_describe_the_complete_saved_version() -> anyhow::Result<()> {
    let (operator, profile) = test_operator_with_profile().await;
    let repo = test_repo(&operator, &profile).await;
    let branch = repo.branch("main").open().perform(&operator).await?;
    let entity: Entity = "id:review/versions".parse()?;
    let before = session::read(&branch, &entity, Some(Format::Text), &operator).await?;
    session::write(
        &branch,
        &entity,
        None,
        Some(&before.heads),
        &set("one"),
        &Stamp::default(),
        &operator,
    )
    .await?;
    let saved = session::write(
        &branch,
        &entity,
        None,
        Some(&before.heads),
        &set("two"),
        &Stamp::default(),
        &operator,
    )
    .await?;
    assert_eq!(saved.snapshot.heads.len(), 2);
    let query = serde_json::from_value(
        serde_json::json!({"predicate":"document/versions", "terms":{"document":entity.to_string()}}),
    )?;
    let versions = formula::resolve(&branch, &operator, &query).await?;
    let ipld_core::ipld::Ipld::String(heads) = &versions[0].fields["heads"] else {
        panic!("expected heads")
    };
    assert_eq!(
        formula::parse_heads(heads),
        saved.snapshot.heads,
        "versions must include surviving heads, not only heads asserted in this revision"
    );
    Ok(())
}

#[tokio::test]
async fn versions_find_a_save_behind_many_unrelated_commits() -> anyhow::Result<()> {
    let (operator, profile) = test_operator_with_profile().await;
    let repo = test_repo(&operator, &profile).await;
    let branch = repo.branch("main").open().perform(&operator).await?;
    let entity: Entity = "id:review/old-save".parse()?;
    let saved = session::write(
        &branch,
        &entity,
        Some(Format::Text),
        None,
        &set("old"),
        &Stamp::default(),
        &operator,
    )
    .await?;
    for n in 0..25 {
        branch
            .transaction()
            .assert(
                the!("review/unrelated")
                    .of(entity.clone())
                    .is(n.to_string()),
            )
            .commit()
            .publish()
            .perform(&operator)
            .await?;
    }
    let query = serde_json::from_value(
        serde_json::json!({"predicate":"document/versions", "terms":{"document":entity.to_string(), "limit":"1"}}),
    )?;
    let versions = formula::resolve(&branch, &operator, &query).await?;
    assert_eq!(versions.len(), 1);
    assert_eq!(
        versions[0].fields["heads"],
        ipld_core::ipld::Ipld::String(formula::format_heads(&saved.snapshot.heads))
    );
    Ok(())
}

#[tokio::test]
async fn versions_include_the_complete_heads_of_a_branch_merge() -> anyhow::Result<()> {
    let (operator, profile) = test_operator_with_profile().await;
    let repo = test_repo(&operator, &profile).await;
    let main = repo.branch("main").open().perform(&operator).await?;
    let entity: Entity = "id:review/merge-version".parse()?;
    session::write(
        &main,
        &entity,
        Some(Format::Text),
        None,
        &set("base"),
        &Stamp::default(),
        &operator,
    )
    .await?;
    let other = repo.branch("other").open().perform(&operator).await?;
    other.set_upstream(&main).perform(&operator).await?;
    other.pull().perform(&operator).await?;
    session::write(
        &main,
        &entity,
        None,
        None,
        &set("left base"),
        &Stamp::default(),
        &operator,
    )
    .await?;
    session::write(
        &other,
        &entity,
        None,
        None,
        &set("base right"),
        &Stamp::default(),
        &operator,
    )
    .await?;
    other.pull().perform(&operator).await?;
    let saved = session::read(&other, &entity, None, &operator).await?;
    let query = serde_json::from_value(
        serde_json::json!({"predicate":"document/versions", "terms":{"document":entity.to_string(), "limit":"1"}}),
    )?;
    let versions = formula::resolve(&other, &operator, &query).await?;
    assert_eq!(
        versions[0].fields["heads"],
        ipld_core::ipld::Ipld::String(formula::format_heads(&saved.heads))
    );
    Ok(())
}
