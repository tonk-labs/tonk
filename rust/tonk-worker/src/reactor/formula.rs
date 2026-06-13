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
//!
//! Node hashes travel as `#<base58>` strings: that is what each row's
//! `this` carries, and what a `hash`/`child` input term is parsed from,
//! so a row from one operator feeds the next operator's input directly.

use std::collections::BTreeMap;

use base58::{FromBase58, ToBase58};
use dialog_artifacts::{Datum, KeyBytes, State, Value};
use dialog_common::Blake3Hash as NodeHash;
use dialog_query::Term;
use dialog_repository::{Branch, LocalIndex, RepositoryArchiveExt};
use dialog_search_tree::{ArchivedNodeBody, Buffer, DialogSearchTreeError, Node, into_owned};
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

    /// A required input term was missing or not a node-hash string.
    #[error("bad input for {formula}: {reason}")]
    BadInput {
        /// The formula that was being resolved.
        formula: String,
        /// Why the input was rejected.
        reason: String,
    },

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
        // Describe one node. `hash` is optional and defaults to the
        // tree root, so a bare `tree/node` query is the entry point.
        "tree/node" => match node_input(branch, query, "hash", name)? {
            Some(hash) => Ok(vec![node_row(branch, env, hash).await?]),
            None => Ok(vec![]), // empty tree — no root node
        },

        // Children of a branch node. `hash` is required (no node-hash
        // index to scan from). One self-contained row per child.
        "tree/child" => {
            let Some(hash) = node_input(branch, query, "hash", name)? else {
                return Ok(vec![]);
            };
            child_rows(branch, env, hash).await
        }

        // Entries of a leaf node. `hash` is required. One row per
        // stored entry, carrying the key and its decoded datum.
        "tree/entry" => {
            let Some(hash) = node_input(branch, query, "hash", name)? else {
                return Ok(vec![]);
            };
            entry_rows(branch, env, hash).await
        }

        // Decompose a composite key into its components. Pure: no
        // block read. `key` is required.
        "tree/key" => match key_input(query, "key", name)? {
            Some(key) => Ok(vec![key_row(key)]),
            None => Ok(vec![]),
        },

        other => Err(FormulaError::Unknown(other.into())),
    }
}

/// Resolve the node-hash input named `param`. A `#<base58>` constant is
/// parsed; an absent or unbound term falls back to the tree root (so a
/// bare query targets the root). `Ok(None)` means an empty tree.
fn node_input(
    branch: &Branch,
    query: &Query,
    param: &str,
    formula: &str,
) -> Result<Option<Blake3Hash>, FormulaError> {
    match query.terms.get(param) {
        Some(Term::Constant(Value::String(s))) => parse_hash(s, formula).map(Some),
        Some(Term::Constant(other)) => Err(FormulaError::BadInput {
            formula: formula.into(),
            reason: format!("`{param}` must be a node-hash string, got {other:?}"),
        }),
        // Unbound variable or absent term: default to the root.
        _ => Ok(root_hash(branch)),
    }
}

/// Parse a `#<base58>` node-hash string into raw bytes.
fn parse_hash(s: &str, formula: &str) -> Result<Blake3Hash, FormulaError> {
    let bad = |reason: String| FormulaError::BadInput {
        formula: formula.into(),
        reason,
    };
    let raw = s.strip_prefix('#').unwrap_or(s);
    let bytes = raw
        .from_base58()
        .map_err(|e| bad(format!("invalid base58 hash {s:?}: {e:?}")))?;
    bytes
        .try_into()
        .map_err(|v: Vec<u8>| bad(format!("hash {s:?} is {} bytes, want 32", v.len())))
}

/// The branch's current root node hash, or `None` for an empty tree.
fn root_hash(branch: &Branch) -> Option<Blake3Hash> {
    let revision = branch.revision()?;
    let hash = *revision.tree.hash();
    (hash != [0u8; 32]).then_some(hash)
}

/// Read and decode one node from the live archive.
async fn read_node<Env: GetPutProvider>(
    branch: &Branch,
    env: &Env,
    hash: Blake3Hash,
) -> Result<TreeNode, FormulaError> {
    let index = LocalIndex::new(env, branch.archive().index());
    let bytes = index
        .get(&hash)
        .await
        .map_err(|e| FormulaError::Read(e.to_string()))?
        .ok_or_else(|| FormulaError::Read(format!("node {} not found", to_base58(&hash))))?;
    Ok(TreeNode::new(Buffer::from(bytes)))
}

/// The scalar fields describing a node: kind, byte size, child/entry
/// count, leaf flag. Shared by `tree/node` and `tree/child` so a child
/// row carries the child's own node fields (no join needed).
fn node_fields(node: &TreeNode) -> Result<BTreeMap<String, Ipld>, FormulaError> {
    let body = node
        .body()
        .map_err(|e: DialogSearchTreeError| FormulaError::Decode(e.to_string()))?;

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
    Ok(fields)
}

/// One `tree/node` conclusion for the node at `hash`.
async fn node_row<Env: GetPutProvider>(
    branch: &Branch,
    env: &Env,
    hash: Blake3Hash,
) -> Result<Conclusion, FormulaError> {
    let node = read_node(branch, env, hash).await?;
    Ok(Conclusion {
        this: to_base58(&hash),
        fields: node_fields(&node)?,
    })
}

/// One `tree/child` conclusion per child of the branch node at `hash`.
///
/// Each row is self-contained: it names the child (`child` field + the
/// row's `this`), its sibling position (`at`), and the child's own node
/// fields (kind/size/count/leaf), read from the child block. A leaf
/// node has no children and yields no rows.
async fn child_rows<Env: GetPutProvider>(
    branch: &Branch,
    env: &Env,
    hash: Blake3Hash,
) -> Result<Vec<Conclusion>, FormulaError> {
    let parent = read_node(branch, env, hash).await?;
    let body = parent
        .body()
        .map_err(|e: DialogSearchTreeError| FormulaError::Decode(e.to_string()))?;

    let ArchivedNodeBody::Index(index) = body else {
        return Ok(vec![]); // a leaf has no children
    };

    // Collect child hashes first so we can drop the borrow on `parent`
    // before the awaits that read each child.
    let children: Vec<Blake3Hash> = index
        .links
        .iter()
        .map(|link| *<&NodeHash>::from(&link.node).as_bytes())
        .collect();

    let mut rows = Vec::with_capacity(children.len());
    for (at, child) in children.into_iter().enumerate() {
        let node = read_node(branch, env, child).await?;
        let mut fields = node_fields(&node)?;
        fields.insert("child".into(), Ipld::String(to_base58(&child)));
        fields.insert("at".into(), Ipld::Integer(at as i128));
        rows.push(Conclusion {
            this: to_base58(&child),
            fields,
        });
    }
    Ok(rows)
}

/// One `tree/entry` conclusion per entry in the leaf node at `hash`.
///
/// Each row carries the entry's composite key (base58 of its 162 bytes,
/// for now — `tree/key` decomposes it into components), its position in
/// the leaf (`at`), the asserted/retracted `state`, and, for an
/// asserted entry, the decoded datum's `entity` / `attribute` /
/// `value-type`. A branch node has no entries and yields no rows.
async fn entry_rows<Env: GetPutProvider>(
    branch: &Branch,
    env: &Env,
    hash: Blake3Hash,
) -> Result<Vec<Conclusion>, FormulaError> {
    let leaf = read_node(branch, env, hash).await?;
    let body = leaf
        .body()
        .map_err(|e: DialogSearchTreeError| FormulaError::Decode(e.to_string()))?;

    let ArchivedNodeBody::Segment(segment) = body else {
        return Ok(vec![]); // a branch has no entries
    };

    let mut rows = Vec::with_capacity(segment.entries.len());
    for (at, entry) in segment.entries.iter().enumerate() {
        let key: KeyBytes =
            into_owned(&entry.key).map_err(|e| FormulaError::Decode(e.to_string()))?;
        let state: State<Datum> =
            into_owned(&entry.value).map_err(|e| FormulaError::Decode(e.to_string()))?;

        let mut fields: BTreeMap<String, Ipld> = BTreeMap::new();
        fields.insert("key".into(), Ipld::String(format!("#{}", key.to_base58())));
        fields.insert("at".into(), Ipld::Integer(at as i128));
        match state {
            State::Added(datum) => {
                fields.insert("state".into(), Ipld::String("added".into()));
                fields.insert("entity".into(), Ipld::String(datum.entity));
                fields.insert("attribute".into(), Ipld::String(datum.attribute));
                fields.insert("value-type".into(), Ipld::Integer(datum.value_type as i128));
            }
            State::Removed => {
                fields.insert("state".into(), Ipld::String("removed".into()));
            }
        }

        rows.push(Conclusion {
            this: format!("#{}", key.to_base58()),
            fields,
        });
    }
    Ok(rows)
}

/// Composite index-key layout (bytes), per dialog-artifacts/src/key.rs:
/// `[ Tag:1 ][ Entity:64 ][ Attribute:64 ][ ValueType:1 ][ ValueRef:32 ]`.
const KEY_TAG: usize = 0;
const KEY_ENTITY: std::ops::Range<usize> = 1..65;
const KEY_ATTRIBUTE: std::ops::Range<usize> = 65..129;
const KEY_VALUE_TYPE: usize = 129;
const KEY_VALUE_REF: std::ops::Range<usize> = 130..162;
const KEY_LEN: usize = 162;

/// Resolve the required `key` input: a `#<base58>` string of the 162-byte
/// composite key.
fn key_input(query: &Query, param: &str, formula: &str) -> Result<Option<KeyBytes>, FormulaError> {
    let bad = |reason: String| FormulaError::BadInput {
        formula: formula.into(),
        reason,
    };
    match query.terms.get(param) {
        Some(Term::Constant(Value::String(s))) => {
            let raw = s.strip_prefix('#').unwrap_or(s);
            let bytes = raw
                .from_base58()
                .map_err(|e| bad(format!("invalid base58 key {s:?}: {e:?}")))?;
            let key: KeyBytes = bytes
                .try_into()
                .map_err(|v: Vec<u8>| bad(format!("key is {} bytes, want {KEY_LEN}", v.len())))?;
            Ok(Some(key))
        }
        Some(_) => Err(bad(format!("`{param}` must be a key string"))),
        None => Err(bad(format!("`{param}` is required"))),
    }
}

/// One `tree/key` conclusion: the key's components, each base58-encoded.
/// The tag byte names the index ordering (entity / attribute / value);
/// the other components are the raw entity / attribute / value-reference
/// slots. Human-readable resolution (entity → did:key, attribute →
/// domain/name) is a later refinement.
fn key_row(key: KeyBytes) -> Conclusion {
    let tag = match key[KEY_TAG] {
        0 => "entity",
        1 => "attribute",
        2 => "value",
        _ => "unknown",
    };

    let mut fields: BTreeMap<String, Ipld> = BTreeMap::new();
    fields.insert("tag".into(), Ipld::String(tag.into()));
    fields.insert("entity".into(), Ipld::String(key[KEY_ENTITY].to_base58()));
    fields.insert(
        "attribute".into(),
        Ipld::String(key[KEY_ATTRIBUTE].to_base58()),
    );
    fields.insert(
        "value-type".into(),
        Ipld::Integer(key[KEY_VALUE_TYPE] as i128),
    );
    fields.insert(
        "value-ref".into(),
        Ipld::String(key[KEY_VALUE_REF].to_base58()),
    );

    Conclusion {
        this: format!("#{}", key.to_base58()),
        fields,
    }
}

/// Format a node hash as the `#<base58>` string used across `tree/*`.
fn to_base58(hash: &Blake3Hash) -> String {
    format!("#{}", hash.to_base58())
}
