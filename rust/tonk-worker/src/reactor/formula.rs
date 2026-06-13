//! Worker-resolved query formulas — named procedures the worker
//! answers itself rather than handing to dialog's planner.
//!
//! A `/query` body whose `predicate` is a bare string (rather than a
//! concept object) names a formula; [`resolve_formula`] dispatches by
//! name and returns [`Conclusion`] rows in the same shape concept
//! queries produce, so the host / display path is unchanged.
//!
//! The first family is `tree/*` — introspection of the branch's index
//! tree (node structure, sizes, entries). See `plan/tree-inspector.md`.

use std::collections::BTreeMap;

use dialog_artifacts::{Datum, KeyBytes, State};
use dialog_repository::{Branch, LocalIndex, RepositoryArchiveExt};
use dialog_search_tree::{ArchivedNodeBody, Buffer, Node};
use dialog_storage::{Blake3Hash, StorageBackend};
use ipld_core::ipld::Ipld;
use thiserror::Error;

use crate::reactor::{Conclusion, GetPutProvider, Query};

/// A decoded tree node, instantiated for the artifact key/value types.
type TreeNode = Node<KeyBytes, State<Datum>>;

/// Failure modes for [`resolve_formula`].
#[derive(Debug, Error)]
pub enum FormulaError {
    /// The query named no formula (a concept query reached here — a
    /// router bug).
    #[error("not a formula query")]
    NotFormula,

    /// No formula is registered under this name.
    #[error("unknown formula: {0}")]
    Unknown(String),

    /// Reading a node block from the archive failed.
    #[error("archive read failed: {0}")]
    Read(String),

    /// Decoding a node block failed.
    #[error("node decode failed: {0}")]
    Decode(String),
}

/// Resolve a formula [`Query`] against `branch`, returning its rows.
pub async fn resolve_formula<Env: GetPutProvider>(
    branch: &Branch,
    env: &Env,
    query: &Query,
) -> Result<Vec<Conclusion>, FormulaError> {
    let name = query.formula().ok_or(FormulaError::NotFormula)?;

    match name {
        // For now `tree/node` with no `hash` term reports the root
        // node. Arbitrary-hash input and the other operators land in
        // the next increment.
        "tree/node" => match root_hash(branch) {
            Some(root) => Ok(vec![node_row(branch, env, root).await?]),
            None => Ok(vec![]), // empty tree — no root node
        },
        "tree/child" | "tree/entry" | "tree/key" => Ok(vec![]),
        other => Err(FormulaError::Unknown(other.into())),
    }
}

/// The branch's current root node hash, or `None` for an empty tree.
fn root_hash(branch: &Branch) -> Option<Blake3Hash> {
    let revision = branch.revision()?;
    let hash = *revision.tree.hash();
    (hash != [0u8; 32]).then_some(hash)
}

/// Read and decode one node, projecting its scalar fields into a
/// `tree/node` conclusion: kind, size, level-marker, child/entry count.
async fn node_row<Env: GetPutProvider>(
    branch: &Branch,
    env: &Env,
    hash: Blake3Hash,
) -> Result<Conclusion, FormulaError> {
    let index = LocalIndex::new(env, branch.archive().index());
    let bytes = index
        .get(&hash)
        .await
        .map_err(|e| FormulaError::Read(e.to_string()))?
        .ok_or_else(|| FormulaError::Read(format!("node {} not found", to_base58(&hash))))?;

    let node: TreeNode = TreeNode::new(Buffer::from(bytes));
    let body = node
        .body()
        .map_err(|e: dialog_search_tree::DialogSearchTreeError| {
            FormulaError::Decode(e.to_string())
        })?;

    let (kind, count, is_leaf) = match body {
        ArchivedNodeBody::Index(index) => ("branch", index.links.len(), false),
        ArchivedNodeBody::Segment(segment) => ("leaf", segment.entries.len(), true),
    };

    let size = node.buffer().as_ref().len();

    let mut fields: BTreeMap<String, Ipld> = BTreeMap::new();
    fields.insert("kind".into(), Ipld::String(kind.into()));
    fields.insert("size".into(), Ipld::Integer(size as i128));
    fields.insert("count".into(), Ipld::Integer(count as i128));
    fields.insert("leaf".into(), Ipld::Bool(is_leaf));

    Ok(Conclusion {
        this: to_base58(&hash),
        fields,
    })
}

fn to_base58(hash: &Blake3Hash) -> String {
    use base58::ToBase58;
    format!("#{}", hash.to_base58())
}
