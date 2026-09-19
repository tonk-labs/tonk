//! Export and import of document cells.
//!
//! A branch export walks the search tree, and a document's bytes are
//! not in it: only the `document/format` and `document/heads` claims
//! are. An export of the tree alone would carry heads that point at
//! changes the importing side never gets.
//!
//! So an export appends one artifact per document — the attribute
//! [`BYTES`], the document entity, the document saved AT THE BRANCH'S
//! HEADS, so nothing of another branch leaves with it. The value is
//! base64 TEXT, not a bytes value: dialog's CSV decodes bytes with a
//! base58 reader that stops at 132 bytes, so a bytes value of any real
//! size does not survive the round trip. The artifact
//! exists only inside an export: [`take_documents`] removes it from an
//! import before anything is committed, and [`restore`] merges the bytes
//! into the local cell. The bytes never become a claim, for the same
//! reason they are not one to begin with.
//!
//! An import restores the cells FIRST and commits the claims after, the
//! same order a write uses: a crash in between leaves bytes no branch
//! points at, never heads without their changes.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD_NO_PAD as BASE64;
use dialog_artifacts::{Artifact, DialogArtifactsError, Entity, Exporter, Value};
use dialog_common::ConditionalSend;
use dialog_repository::Branch;

use crate::cell::{self, LocalCell};
use crate::engine::{Document, DocumentError, Format};
use crate::session::{self, DocumentEnv, SessionError};

/// The export-only attribute that carries a document's bytes.
pub const BYTES: &str = "xyz.tonk.document/bytes";

/// One artifact per document of `branch`, each the document as the
/// branch sees it. A document whose cell this replica does not hold
/// (never opened here, not yet synced) is skipped and named in the
/// second list, so a caller can say the export is incomplete.
pub async fn documents<Env: DocumentEnv>(
    branch: &Branch,
    env: &Env,
) -> Result<(Vec<Artifact>, Vec<Entity>), SessionError> {
    let the = BYTES
        .parse()
        .map_err(|error| SessionError::Branch(format!("{BYTES}: {error}")))?;
    let mut artifacts = Vec::new();
    let mut missing = Vec::new();
    for (entity, name) in session::documents(branch, env).await? {
        let Some(format) = Format::parse(&name) else {
            // A format this build does not know: its claims still export.
            missing.push(entity);
            continue;
        };
        let local = LocalCell::new(&branch.subject(), &entity, env);
        let Some((mut document, _)) = cell::load(&local).await? else {
            missing.push(entity);
            continue;
        };
        let (_, heads) = session::heads_or_genesis(branch, &entity, format, env).await?;
        match document.save_at(&heads) {
            Ok(bytes) => artifacts.push(Artifact {
                the: Clone::clone(&the),
                of: entity,
                is: Value::String(BASE64.encode(bytes)),
                cause: None,
            }),
            Err(DocumentError::MissingChanges(_)) => missing.push(entity),
            Err(error) => return Err(error.into()),
        }
    }
    Ok((artifacts, missing))
}

/// An exporter that writes `documents` after the branch's own artifacts.
pub struct WithDocuments<E> {
    inner: E,
    documents: Vec<Artifact>,
}

impl<E> WithDocuments<E> {
    /// Wrap `inner`; `documents` comes from [`documents`].
    pub fn new(inner: E, documents: Vec<Artifact>) -> Self {
        Self { inner, documents }
    }
}

#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl<E: Exporter + ConditionalSend> Exporter for WithDocuments<E> {
    async fn write(&mut self, artifact: &Artifact) -> Result<(), DialogArtifactsError> {
        self.inner.write(artifact).await
    }

    async fn close(&mut self) -> Result<(), DialogArtifactsError> {
        for document in std::mem::take(&mut self.documents) {
            self.inner.write(&document).await?;
        }
        self.inner.close().await
    }
}

/// One row of an import, as an importer yields it.
pub type Row = Result<Artifact, DialogArtifactsError>;

/// Split the rows of an import: the document bytes out, everything else
/// (errors included, for the importer to treat as it always has) left.
/// A [`BYTES`] row that is not base64 is turned into an error row: it
/// must never be committed as a claim.
pub fn take_documents(rows: Vec<Row>) -> (Vec<Row>, Vec<(Entity, Vec<u8>)>) {
    let mut rest = Vec::with_capacity(rows.len());
    let mut documents = Vec::new();
    for row in rows {
        match row {
            Ok(Artifact { the, of, is, .. }) if the.to_string() == BYTES => {
                let decoded = match &is {
                    Value::String(text) => BASE64.decode(text).ok(),
                    _ => None,
                };
                match decoded {
                    Some(bytes) => documents.push((of, bytes)),
                    None => rest.push(Err(DialogArtifactsError::InvalidValue(format!(
                        "{BYTES} of {of} is not base64 text"
                    )))),
                }
            }
            other => rest.push(other),
        }
    }
    (rest, documents)
}

/// Merge imported bytes into this replica's cells. A merge, so importing
/// over a document that already exists here loses nothing of either.
/// Touches no branch: the heads arrive with the import's own claims.
pub async fn restore<Env: DocumentEnv>(
    branch: &Branch,
    documents: &[(Entity, Vec<u8>)],
    env: &Env,
) -> Result<(), SessionError> {
    for (entity, bytes) in documents {
        let incoming = Document::load(bytes)?;
        let local = LocalCell::new(&branch.subject(), entity, env);
        let (mut document, version) = match cell::load(&local).await? {
            Some((mut document, version)) => {
                if document.format() != incoming.format() {
                    return Err(SessionError::Document(DocumentError::WrongShape(
                        document.format().name(),
                    )));
                }
                document.merge(bytes)?;
                (document, Some(version))
            }
            None => (incoming, None),
        };
        cell::save(&local, &mut document, version).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{Edit, Stamp};
    use dialog_operator::helpers::{test_operator_with_profile, test_repo};
    use futures_util::stream;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_browser);

    /// An exporter that keeps what it is given.
    #[derive(Default)]
    struct Collect(Vec<Artifact>);

    #[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
    #[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
    impl Exporter for &mut Collect {
        async fn write(&mut self, artifact: &Artifact) -> Result<(), DialogArtifactsError> {
            self.0.push(artifact.clone());
            Ok(())
        }
        async fn close(&mut self) -> Result<(), DialogArtifactsError> {
            Ok(())
        }
    }

    #[dialog_common::test]
    async fn it_keeps_a_document_across_an_export_and_an_import() -> anyhow::Result<()> {
        let (operator, profile) = test_operator_with_profile().await;
        let stamp = Stamp {
            author: None,
            time: 1,
        };
        let doc: Entity = "id:prose/doc".parse()?;

        let source = test_repo(&operator, &profile).await;
        let main = source.branch("main").open().perform(&operator).await?;
        session::write(
            &main,
            &doc,
            Some(Format::Text),
            None,
            &[Edit::SetText {
                text: "kept".into(),
            }],
            &stamp,
            &operator,
        )
        .await?;
        // Another branch's edit shares the cell, and must not leave with main.
        let other = source.branch("other").open().perform(&operator).await?;
        session::write(
            &other,
            &doc,
            Some(Format::Text),
            None,
            &[Edit::SetText {
                text: "private".into(),
            }],
            &stamp,
            &operator,
        )
        .await?;

        let (documents, missing) = documents(&main, &operator).await?;
        assert!(missing.is_empty());
        let mut exported = Collect::default();
        main.export(WithDocuments::new(&mut exported, documents))
            .perform(&operator)
            .await?;
        assert!(
            exported.0.iter().any(|row| row.the.to_string() == BYTES),
            "the export carries the document"
        );

        let target = test_repo(&operator, &profile).await;
        let branch = target.branch("main").open().perform(&operator).await?;
        // dialog's own revision records belong to the repo that minted
        // them; an import refuses the reserved namespace.
        let rows = exported
            .0
            .into_iter()
            .filter(|row| !row.the.to_string().starts_with("dialog."))
            .map(Ok)
            .collect();
        let (rest, bytes) = take_documents(rows);
        assert!(
            rest.iter()
                .flatten()
                .all(|row| row.the.to_string() != BYTES)
        );
        restore(&branch, &bytes, &operator).await?;
        branch.import(stream::iter(rest)).perform(&operator).await?;

        let snapshot = session::read(&branch, &doc, None, &operator).await?;
        assert!(
            matches!(snapshot.content, crate::engine::Content::Text(ref text) if text == "kept")
        );
        let mut restored = Document::load(&session::bytes(&branch, &doc, &operator).await?)?;
        assert_eq!(
            restored.store_heads(),
            snapshot.heads,
            "only the exported branch's changes came along"
        );
        Ok(())
    }
}
