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
use dialog_artifacts::{Artifact, Datum, Key, State, Value};
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

use crate::{Conclusion, Query, SelectProvider};

/// A decoded tree node, instantiated for the artifact key/value types.
type TreeNode = PersistentNode<Key, State<Datum>>;

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
            Some(key) => Ok(vec![key_row(key)?]),
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
    let decode = |e: DialogSearchTreeError| FormulaError::Decode(e.to_string());
    let body = node.body().map_err(decode)?;

    let (kind, count) = match body {
        ArchivedNodeBody::Index(index) => ("index", index.len()),
        ArchivedNodeBody::Segment(segment) => ("segment", segment.len()),
    };
    let size = node.buffer().as_ref().len();

    let mut fields: BTreeMap<String, Ipld> = BTreeMap::new();
    fields.insert("kind".into(), Ipld::String(kind.into()));
    fields.insert("size".into(), Ipld::Integer(size as i128));
    fields.insert("count".into(), Ipld::Integer(count as i128));
    // Only a segment carries a full upper-bound key; an index's table
    // holds separators, not whole keys, so it reports no bound.
    if let Some(bound) = node.upper_bound().map_err(decode)? {
        let manifest = node.manifest().map_err(decode)?;
        fields.insert("bound".into(), Ipld::String(bytes_hex(&bound)));
        // The node's rank — the boundary level its upper-bound key
        // falls on (geometric over the key bytes). Higher rank ⇒
        // higher in the tree; it's what determines the tree's shape.
        fields.insert(
            "rank".into(),
            Ipld::Integer(Geometric::rank(&bound, &manifest) as i128),
        );
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
    let decode = |e: DialogSearchTreeError| FormulaError::Decode(e.to_string());
    let parent = read_node(branch, env, hash).await?;
    let body = parent.body().map_err(decode)?;

    let ArchivedNodeBody::Index(index) = body else {
        return Ok(vec![]); // a segment has no children
    };
    let manifest = parent.manifest().map_err(decode)?;

    // Collect each child's hash and separator up front so we can drop the
    // borrow on `parent` before the awaits that read each child. The link
    // carries the separator, so a not-cached child can still show its bound.
    let children: Vec<(Blake3Hash, Vec<u8>)> = index
        .links()
        .map_err(decode)?
        .into_iter()
        .map(|link| (*link.node.as_bytes(), link.separator))
        .collect();

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
                fields.insert("bound".into(), Ipld::String(bytes_hex(&bound)));
                fields.insert(
                    "rank".into(),
                    Ipld::Integer(Geometric::rank(&bound, &manifest) as i128),
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
/// Each row carries the entry's composite key (hex of its raw bytes —
/// `tree/key` decomposes it into components), its position in the leaf
/// (`at`), the asserted/retracted `state`, and, for an asserted entry, the
/// entity / attribute / value reconstructed from the key. An index node
/// has no entries and yields no rows.
async fn entry_rows<Env: SelectProvider>(
    branch: &Branch,
    env: &Env,
    hash: Blake3Hash,
) -> Result<Vec<Conclusion>, FormulaError> {
    let decode = |e: DialogSearchTreeError| FormulaError::Decode(e.to_string());
    let leaf = read_node(branch, env, hash).await?;
    let body = leaf.body().map_err(decode)?;

    let ArchivedNodeBody::Segment(segment) = body else {
        return Ok(vec![]); // an index has no entries
    };
    let manifest = leaf.manifest().map_err(decode)?;

    // The columnar leaf stores keys and values in separate tables; stream
    // the keys (in entry order) and pair each with its value slot by index.
    // Copy each key out of the borrowed streamer so it survives the
    // per-entry `value_at` borrow.
    let mut keys = segment.keys::<Key>().map_err(decode)?;
    let mut owned_keys: Vec<Vec<u8>> = Vec::with_capacity(segment.len());
    while let Some((_, key)) = keys.next_key().map_err(decode)? {
        owned_keys.push(key.to_vec());
    }

    let mut rows = Vec::with_capacity(owned_keys.len());
    for (at, key_bytes) in owned_keys.into_iter().enumerate() {
        let state: State<Datum> =
            into_owned(segment.value_at(at).map_err(decode)?).map_err(decode)?;

        let key_hex = bytes_hex(&key_bytes);
        let mut fields: BTreeMap<String, Ipld> = BTreeMap::new();
        fields.insert("key".into(), Ipld::String(key_hex.clone()));
        fields.insert("at".into(), Ipld::Integer(at as i128));
        fields.insert(
            "rank".into(),
            Ipld::Integer(Geometric::rank(&key_bytes, &manifest) as i128),
        );

        // A segment holds asserted facts; a retraction is a tombstone. The
        // entity/attribute/value all live IN the key now (the datum carries
        // only causal metadata), so reconstruct the fact from the key —
        // standing in a placeholder for a spilled value, since the inspector
        // has no store to fetch the block.
        match state {
            State::Added(datum) => {
                let key = Key::from(key_bytes);
                let artifact = Artifact::from_key_datum_placeholder(&key, &datum)
                    .map_err(|e| FormulaError::Decode(e.to_string()))?;
                fields.insert("entity".into(), Ipld::String(artifact.of.to_string()));
                fields.insert("attribute".into(), Ipld::String(artifact.the.to_string()));
                fields.insert(
                    "type".into(),
                    Ipld::String(artifact.is.data_type().to_string()),
                );
                if let Some(ipld) = value_to_ipld(&artifact.is) {
                    fields.insert("value".into(), ipld);
                }
            }
            State::Removed => {
                fields.insert("retracted".into(), Ipld::Bool(true));
            }
        }

        rows.push(Conclusion {
            this: key_hex,
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

/// Key tags, per dialog-artifacts/src/key.rs. The M3 key format is
/// variable-length and columnar — a fixed byte layout no longer applies —
/// so the components are recovered by decoding, not slicing.
const ENTITY_KEY_TAG: u8 = 0;
const ATTRIBUTE_KEY_TAG: u8 = 1;
const VALUE_KEY_TAG: u8 = 2;

/// Resolve the required `key` input: a `0x`-prefixed hex string of the raw
/// composite key bytes.
fn key_input(query: &Query, param: &str, formula: &str) -> Result<Option<Key>, FormulaError> {
    let bad = |reason: String| FormulaError::BadInput {
        formula: formula.into(),
        reason,
    };
    match query.terms.get(param) {
        Some(Term::Constant(Value::String(s))) => {
            let raw = s.strip_prefix("0x").unwrap_or(s);
            if raw.len() % 2 != 0 {
                return Err(bad(format!("hex key {s:?} has an odd digit count")));
            }
            let bytes = (0..raw.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&raw[i..i + 2], 16))
                .collect::<Result<Vec<u8>, _>>()
                .map_err(|e| bad(format!("invalid hex key {s:?}: {e}")))?;
            Ok(Some(Key::from(bytes)))
        }
        Some(_) => Err(bad(format!("`{param}` must be a key string"))),
        None => Err(bad(format!("`{param}` is required"))),
    }
}

/// One `tree/key` conclusion: the key's decoded components. The tag names
/// the index ordering (entity / attribute / value); the entity, attribute,
/// value type, and (for an inline key) value are reconstructed from the
/// columnar key. A spilled value shows the placeholder.
fn key_row(key: Key) -> Result<Conclusion, FormulaError> {
    let tag = match key.tag() {
        ENTITY_KEY_TAG => "entity",
        ATTRIBUTE_KEY_TAG => "attribute",
        VALUE_KEY_TAG => "value",
        _ => "unknown",
    };

    let mut fields: BTreeMap<String, Ipld> = BTreeMap::new();
    fields.insert("tag".into(), Ipld::String(tag.into()));

    // A bare key carries no datum, so reconstruct the fact against an empty
    // one (only its `cause` is read, and that is absent here).
    let datum = Datum::for_artifact(&Artifact {
        the: "x/x".parse().expect("placeholder attribute parses"),
        of: dialog_artifacts::Entity::new().expect("placeholder entity mints"),
        is: Value::Boolean(false),
        cause: None,
    });
    if let Ok(artifact) = Artifact::from_key_datum_placeholder(&key, &datum) {
        fields.insert("entity".into(), Ipld::String(artifact.of.to_string()));
        fields.insert("attribute".into(), Ipld::String(artifact.the.to_string()));
        fields.insert(
            "value-type".into(),
            Ipld::String(artifact.is.data_type().to_string()),
        );
        if let Some(ipld) = value_to_ipld(&artifact.is) {
            fields.insert("value".into(), ipld);
        }
    }

    Ok(Conclusion {
        this: bytes_hex(key.as_ref()),
        fields,
    })
}

/// Format a node hash as the `#<base58>` string used across `tree/*`.
fn to_base58(hash: &Blake3Hash) -> String {
    format!("#{}", hash.to_base58())
}

/// Encode raw key bytes as a `0x`-prefixed hex string. Keys are
/// variable-length and can exceed the `base58` crate's fixed decode buffer,
/// so they travel as hex, which the client decodes without a length cap.
/// (Node hashes are 32 bytes and stay base58.)
fn bytes_hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(2 + bytes.len() * 2);
    s.push_str("0x");
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}
