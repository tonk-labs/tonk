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
use dialog_artifacts::{Datum, KeyBytes, State, Value, ValueDataType};
use dialog_common::Blake3Hash as NodeHash;
use dialog_query::Term;
use dialog_repository::{
    Branch, LocalIndex, NetworkedIndex, RepositoryArchiveExt, RepositoryMemoryExt, Upstream,
};
use dialog_search_tree::{
    ArchivedNodeBody, Buffer, DialogSearchTreeError, Distribution, Geometric, PersistentNode,
    into_owned,
};
use dialog_storage::{Blake3Hash, StorageBackend};
use ipld_core::ipld::Ipld;
use thiserror::Error;

use crate::reactor::{Conclusion, Query, SelectProvider};

/// A decoded tree node, instantiated for the artifact key/value types.
type TreeNode = PersistentNode<KeyBytes, State<Datum>>;

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
pub async fn resolve_formula<Env: SelectProvider>(
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

        // Children of an index node. `hash` is required (no node-hash
        // index to scan from). One self-contained row per child.
        "tree/child" => {
            let Some(hash) = node_input(branch, query, "hash", name)? else {
                return Ok(vec![]);
            };
            child_rows(branch, env, hash).await
        }

        // Entries of a segment node. `hash` is required. One row per
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

/// Read and decode one node, falling back to the remote when the block is
/// not cached locally — so expanding a not-yet-fetched node transparently
/// pulls it (and caches it) the same way a normal lazy expansion does.
async fn read_node<Env: SelectProvider>(
    branch: &Branch,
    env: &Env,
    hash: Blake3Hash,
) -> Result<TreeNode, FormulaError> {
    let index = NetworkedIndex::new(env, branch.archive().index(), remote(branch, env).await);
    let bytes = index
        .get(&hash)
        .await
        .map_err(|e| FormulaError::Read(e.to_string()))?
        .ok_or_else(|| FormulaError::Read(format!("node {} not found", to_base58(&hash))))?;
    Ok(TreeNode::new(Buffer::from(bytes)))
}

/// Load the branch's upstream remote, if it tracks one, so a networked read
/// can fall back to it. A failure to load (e.g. no credentials) is non-fatal
/// — the local archive alone may still satisfy the read. Mirrors
/// dialog-repository's `Select::perform`.
async fn remote<Env: SelectProvider>(
    branch: &Branch,
    env: &Env,
) -> Option<dialog_repository::RemoteRepository> {
    match branch.upstream() {
        Some(Upstream::Remote { remote: name, .. }) => {
            branch.subject().remote(name).load().perform(env).await.ok()
        }
        _ => None,
    }
}

/// Read a node from the *local* archive only (no remote fallback), returning
/// `None` when the block is not cached. This is what the dot's locality
/// reflects (a local hit is cached, a miss would have to be fetched) and lets
/// `tree/child` list a not-cached child from the parent's link without
/// pulling it — the pull happens only when that child is expanded.
async fn read_local<Env: SelectProvider>(
    branch: &Branch,
    env: &Env,
    hash: Blake3Hash,
) -> Result<Option<TreeNode>, FormulaError> {
    let index = LocalIndex::new(env, branch.archive().index());
    let bytes = index
        .get(&hash)
        .await
        .map_err(|e| FormulaError::Read(e.to_string()))?;
    Ok(bytes.map(|b| TreeNode::new(Buffer::from(b))))
}

/// The scalar fields describing a node: `kind` (`index` for a node of
/// child links, `segment` for a node of entries), byte size, child/entry
/// count, and the node's upper-bound key (`bound`, as a `#<base58>` of
/// the raw 162 key bytes — the inspector slices it into tag-colored
/// segments client-side). Shared by `tree/node` and `tree/child` so a
/// child row carries the child's own node fields.
fn node_fields(node: &TreeNode) -> Result<BTreeMap<String, Ipld>, FormulaError> {
    let body = node
        .body()
        .map_err(|e: DialogSearchTreeError| FormulaError::Decode(e.to_string()))?;

    let (kind, count, upper_bound) = match body {
        ArchivedNodeBody::Index(index) => (
            "index",
            index.links.len(),
            index.links.last().map(|link| &link.upper_bound),
        ),
        ArchivedNodeBody::Segment(segment) => (
            "segment",
            segment.entries.len(),
            segment.entries.last().map(|entry| &entry.key),
        ),
    };
    let size = node.buffer().as_ref().len();

    let mut fields: BTreeMap<String, Ipld> = BTreeMap::new();
    fields.insert("kind".into(), Ipld::String(kind.into()));
    fields.insert("size".into(), Ipld::Integer(size as i128));
    fields.insert("count".into(), Ipld::Integer(count as i128));
    if let Some(archived_key) = upper_bound {
        let key: KeyBytes =
            into_owned(archived_key).map_err(|e| FormulaError::Decode(e.to_string()))?;
        fields.insert("bound".into(), Ipld::String(key_hex(&key)));
        // The node's rank — the boundary level its upper-bound key
        // falls on (geometric over the key's hash). Higher rank ⇒
        // higher in the tree; it's what determines the tree's shape.
        fields.insert("rank".into(), Ipld::Integer(Geometric::rank(&key) as i128));
    }
    Ok(fields)
}

/// One `tree/node` conclusion for the node at `hash`.
async fn node_row<Env: SelectProvider>(
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

/// One `tree/child` conclusion per child of the index node at `hash`.
///
/// Each row is self-contained: it names the child (`child` field + the
/// row's `this`), its sibling position (`at`), and the child's own node
/// fields (kind/size/count/leaf), read from the child block. A leaf
/// node has no children and yields no rows.
async fn child_rows<Env: SelectProvider>(
    branch: &Branch,
    env: &Env,
    hash: Blake3Hash,
) -> Result<Vec<Conclusion>, FormulaError> {
    let parent = read_node(branch, env, hash).await?;
    let body = parent
        .body()
        .map_err(|e: DialogSearchTreeError| FormulaError::Decode(e.to_string()))?;

    let ArchivedNodeBody::Index(index) = body else {
        return Ok(vec![]); // a segment has no children
    };

    // Collect each child's hash and upper-bound key up front so we can drop
    // the borrow on `parent` before the awaits that read each child. The
    // link carries the bound, so a not-cached child can still show its key.
    let children: Vec<(Blake3Hash, KeyBytes)> = index
        .links
        .iter()
        .map(|link| {
            let hash = *<&NodeHash>::from(&link.node).as_bytes();
            let bound: KeyBytes =
                into_owned(&link.upper_bound).map_err(|e| FormulaError::Decode(e.to_string()))?;
            Ok((hash, bound))
        })
        .collect::<Result<_, FormulaError>>()?;

    let mut rows = Vec::with_capacity(children.len());
    for (at, (child, bound)) in children.into_iter().enumerate() {
        // Local-only read: a hit is cached, a miss is remote (the block
        // would have to be fetched). A cached child carries its full node
        // fields; a remote one carries only what the parent's link knows —
        // its bound key and rank — and is flagged `cached: false`.
        let mut fields = match read_local(branch, env, child).await? {
            Some(node) => {
                let mut fields = node_fields(&node)?;
                fields.insert("cached".into(), Ipld::Bool(true));
                fields
            }
            None => {
                let mut fields: BTreeMap<String, Ipld> = BTreeMap::new();
                fields.insert("cached".into(), Ipld::Bool(false));
                fields.insert("bound".into(), Ipld::String(key_hex(&bound)));
                fields.insert(
                    "rank".into(),
                    Ipld::Integer(Geometric::rank(&bound) as i128),
                );
                fields
            }
        };
        fields.insert("child".into(), Ipld::String(to_base58(&child)));
        fields.insert("at".into(), Ipld::Integer(at as i128));
        rows.push(Conclusion {
            this: to_base58(&child),
            fields,
        });
    }
    Ok(rows)
}

/// One `tree/entry` conclusion per entry in the segment node at `hash`.
///
/// Each row carries the entry's composite key (base58 of its 162 bytes,
/// for now — `tree/key` decomposes it into components), its position in
/// the leaf (`at`), the asserted/retracted `state`, and, for an
/// asserted entry, the decoded datum's `entity` / `attribute` /
/// `value-type`. An index node has no entries and yields no rows.
async fn entry_rows<Env: SelectProvider>(
    branch: &Branch,
    env: &Env,
    hash: Blake3Hash,
) -> Result<Vec<Conclusion>, FormulaError> {
    let leaf = read_node(branch, env, hash).await?;
    let body = leaf
        .body()
        .map_err(|e: DialogSearchTreeError| FormulaError::Decode(e.to_string()))?;

    let ArchivedNodeBody::Segment(segment) = body else {
        return Ok(vec![]); // an index has no entries
    };

    let mut rows = Vec::with_capacity(segment.entries.len());
    for (at, entry) in segment.entries.iter().enumerate() {
        let key: KeyBytes =
            into_owned(&entry.key).map_err(|e| FormulaError::Decode(e.to_string()))?;
        let state: State<Datum> =
            into_owned(&entry.value).map_err(|e| FormulaError::Decode(e.to_string()))?;

        let mut fields: BTreeMap<String, Ipld> = BTreeMap::new();
        fields.insert("key".into(), Ipld::String(key_hex(&key)));
        fields.insert("at".into(), Ipld::Integer(at as i128));
        fields.insert("rank".into(), Ipld::Integer(Geometric::rank(&key) as i128));

        // A segment holds asserted facts; a retraction is a tombstone.
        // Surface the datum's own columns — entity, attribute, value
        // type (by name), and the decoded value — not the Added/Removed
        // wrapper as a column.
        if let State::Added(datum) = state {
            fields.insert("entity".into(), Ipld::String(datum.entity));
            fields.insert("attribute".into(), Ipld::String(datum.attribute));
            let value_type = ValueDataType::from(datum.value_type);
            fields.insert("type".into(), Ipld::String(value_type.to_string()));
            if let Ok(value) = Value::try_from((value_type, datum.value))
                && let Some(ipld) = value_to_ipld(&value)
            {
                fields.insert("value".into(), ipld);
            }
        } else {
            fields.insert("retracted".into(), Ipld::Bool(true));
        }

        rows.push(Conclusion {
            this: key_hex(&key),
            fields,
        });
    }
    Ok(rows)
}

/// Convert a decoded [`Value`] to [`Ipld`] for the wire. Mirrors
/// `tonk_core::conclusion`'s handling: `u128` is special-cased since
/// `ipld_core`'s serde path rejects it.
fn value_to_ipld(value: &Value) -> Option<Ipld> {
    Some(match value {
        Value::Bytes(b) => Ipld::Bytes(b.clone()),
        Value::Entity(e) => Ipld::String(e.to_string()),
        Value::Boolean(b) => Ipld::Bool(*b),
        Value::String(s) => Ipld::String(s.clone()),
        Value::Symbol(s) => Ipld::String(s.to_string()),
        Value::UnsignedInt(u) => match i128::try_from(*u) {
            Ok(i) => Ipld::Integer(i),
            Err(_) => Ipld::String(u.to_string()),
        },
        Value::SignedInt(i) => Ipld::Integer(*i),
        Value::Float(f) => Ipld::Float(*f),
        Value::Record(b) => Ipld::Bytes(b.clone()),
    })
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
            let raw = s.strip_prefix("0x").unwrap_or(s);
            let bytes = (0..raw.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&raw[i..i + 2], 16))
                .collect::<Result<Vec<u8>, _>>()
                .map_err(|e| bad(format!("invalid hex key {s:?}: {e}")))?;
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
        this: key_hex(&key),
        fields,
    }
}

/// Format a node hash as the `#<base58>` string used across `tree/*`.
fn to_base58(hash: &Blake3Hash) -> String {
    format!("#{}", hash.to_base58())
}

/// Encode a composite key as a `0x`-prefixed hex string. Keys are 162
/// bytes — too long for the `base58` crate's fixed decode buffer — so
/// they travel as hex, which the client decodes without a length cap.
/// (Node hashes are 32 bytes and stay base58.)
fn key_hex(key: &KeyBytes) -> String {
    let mut s = String::with_capacity(2 + key.len() * 2);
    s.push_str("0x");
    for b in key.iter() {
        s.push_str(&format!("{b:02x}"));
    }
    s
}
