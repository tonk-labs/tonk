//! The document commands, host-neutral.
//!
//! A command names an intent; the runtime fulfils it. `document/replace`
//! asserted from a page and from `tonk assert` is the same command, and
//! this module is the one place its behaviour lives. A host contributes
//! only the moment it calls [`run`]: the worker from its command
//! dispatcher, the CLI right after it commits.
//!
//! The command concepts are declared in
//! `tonk-core/assets/library/document.yaml` and typed in
//! `tonk_schema::command`.

use std::collections::HashMap;

use dialog_artifacts::{Changes, Entity, Instruction};
use dialog_reactor::{Decode, EntityFacts};
use dialog_repository::Branch;
use tonk_schema::command::{
    DocumentInsert, DocumentPut, DocumentRemove, DocumentReplace, DocumentRestore, DocumentSplice,
};

use crate::engine::{Edit, Stamp};
use crate::formula::parse_heads;
use crate::session::{self, DocumentEnv, SessionError, Written};

/// One document command, decoded: which document, from which heads,
/// which edits.
#[derive(Clone, Debug, PartialEq)]
pub struct Request {
    /// The command's own entity — where a refusal is reported.
    pub command: Entity,
    /// The document to edit.
    pub document: Entity,
    /// The heads the edit was computed against; `None` = the branch's.
    pub base: Option<Vec<String>>,
    /// The edits, applied as one change.
    pub edits: Vec<Edit>,
}

/// A `put` value as the table shape stores it: sizes are numbers,
/// everything else is the text as given.
fn put_value(path: &str, value: String) -> serde_json::Value {
    let sized = path.contains("/widths/") || path.contains("/heights/");
    match value.parse::<f64>() {
        Ok(number) if sized => serde_json::json!(number),
        _ => serde_json::Value::String(value),
    }
}

impl From<DocumentReplace> for Request {
    fn from(c: DocumentReplace) -> Self {
        Self {
            command: c.this,
            document: c.document.0,
            base: None,
            edits: vec![Edit::Replace {
                find: c.find.0,
                with: c.with.0,
            }],
        }
    }
}

impl From<DocumentInsert> for Request {
    fn from(c: DocumentInsert) -> Self {
        Self {
            command: c.this,
            document: c.document.0,
            base: None,
            edits: vec![Edit::Insert {
                after: c.after.0,
                text: c.text.0,
            }],
        }
    }
}

impl From<DocumentSplice> for Request {
    fn from(c: DocumentSplice) -> Self {
        Self {
            command: c.this,
            document: c.document.0,
            base: Some(parse_heads(&c.heads.0)),
            edits: vec![Edit::Splice {
                at: c.at.0 as usize,
                delete: c.delete.0 as usize,
                text: c.text.0,
            }],
        }
    }
}

impl From<DocumentPut> for Request {
    fn from(c: DocumentPut) -> Self {
        Self {
            command: c.this,
            document: c.document.0,
            base: None,
            edits: vec![Edit::Put {
                value: put_value(&c.path.0, c.value.0),
                path: c.path.0,
            }],
        }
    }
}

impl From<DocumentRemove> for Request {
    fn from(c: DocumentRemove) -> Self {
        Self {
            command: c.this,
            document: c.document.0,
            base: None,
            edits: vec![Edit::Remove { path: c.path.0 }],
        }
    }
}

impl From<DocumentRestore> for Request {
    fn from(c: DocumentRestore) -> Self {
        Self {
            command: c.this,
            document: c.document.0,
            base: None,
            edits: vec![Edit::Restore {
                heads: parse_heads(&c.heads.0),
            }],
        }
    }
}

/// Every document command in a batch of transient facts. A host that
/// has no command registry of its own — the CLI — dispatches with this.
pub fn requests(transients: &Changes) -> Vec<Request> {
    let mut by_entity: HashMap<Entity, EntityFacts> = HashMap::new();
    for instruction in transients.clone().into_instructions() {
        if let Instruction::Assert(artifact) | Instruction::Replace(artifact) = instruction {
            by_entity.entry(artifact.of.clone()).or_default().push(artifact);
        }
    }
    let mut out = Vec::new();
    for (entity, facts) in by_entity {
        if let Some(c) = DocumentReplace::decode(entity.clone(), &facts) {
            out.push(c.into());
        } else if let Some(c) = DocumentInsert::decode(entity.clone(), &facts) {
            out.push(c.into());
        } else if let Some(c) = DocumentSplice::decode(entity.clone(), &facts) {
            out.push(c.into());
        } else if let Some(c) = DocumentPut::decode(entity.clone(), &facts) {
            out.push(c.into());
        } else if let Some(c) = DocumentRemove::decode(entity.clone(), &facts) {
            out.push(c.into());
        } else if let Some(c) = DocumentRestore::decode(entity, &facts) {
            out.push(c.into());
        }
    }
    out
}

/// Run one document command on `branch`. A refused edit — no match for
/// `find`, two matches, a bad path — changes nothing and comes back as
/// the error; the host decides how to report it.
pub async fn run<Env: DocumentEnv>(
    branch: &Branch,
    env: &Env,
    stamp: &Stamp,
    request: &Request,
) -> Result<Written, SessionError> {
    session::write(
        branch,
        &request.document,
        None,
        request.base.as_deref(),
        &request.edits,
        stamp,
        env,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{Content, Format};
    use dialog_artifacts::Statement as _;
    use dialog_operator::helpers::{test_operator_with_profile, test_repo};
    use dialog_query::the;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_browser);

    fn replace(find: &str, with: &str) -> Changes {
        let mut changes = Changes::new();
        let this: Entity = "cmd:replace".parse().unwrap();
        the!("xyz.tonk.document.replace/document")
            .of(this.clone())
            .is("id:prose/doc".parse::<Entity>().unwrap())
            .assert(&mut changes);
        the!("xyz.tonk.document.replace/find")
            .of(this.clone())
            .is(find.to_string())
            .assert(&mut changes);
        the!("xyz.tonk.document.replace/with")
            .of(this)
            .is(with.to_string())
            .assert(&mut changes);
        changes
    }

    #[dialog_common::test]
    async fn it_decodes_and_runs_a_command_from_transient_facts() -> anyhow::Result<()> {
        let (operator, profile) = test_operator_with_profile().await;
        let repo = test_repo(&operator, &profile).await;
        let branch = repo.branch("main").open().perform(&operator).await?;
        let doc: Entity = "id:prose/doc".parse().unwrap();
        let stamp = Stamp::default();
        session::write(&branch, &doc, Some(Format::Text), None, &[Edit::SetText { text: "hello world".into() }], &stamp, &operator).await?;

        let decoded = requests(&replace("world", "there"));
        assert_eq!(decoded.len(), 1);
        assert_eq!(decoded[0].document, doc);
        let written = run(&branch, &operator, &stamp, &decoded[0]).await?;
        assert_eq!(written.snapshot.content, Content::Text("hello there".into()));

        let refused = run(&branch, &operator, &stamp, &requests(&replace("absent", "x"))[0]).await;
        assert!(refused.is_err(), "no match changes nothing");
        assert_eq!(session::read(&branch, &doc, None, &operator).await?.content, Content::Text("hello there".into()));

        assert!(requests(&Changes::new()).is_empty());
        Ok(())
    }

    #[dialog_common::test]
    fn it_stores_sizes_as_numbers_and_cells_as_text() {
        assert_eq!(put_value("sheets/s/widths/B", "120".into()), serde_json::json!(120.0));
        assert_eq!(put_value("sheets/s/cells/B2", "120".into()), serde_json::json!("120"));
    }
}
