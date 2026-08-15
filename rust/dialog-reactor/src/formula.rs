//! Worker-resolved query formulas — named procedures the worker
//! answers itself rather than handing to dialog's planner.
//!
//! A `/query` body whose `predicate` is a bare string (rather than a
//! concept object) names a formula; [`resolve_formula`] dispatches by
//! name and returns [`Conclusion`] rows in the same shape concept
//! queries produce, so the host / display path is unchanged.
//!
//! The first family is `tree/*` — introspection of the branch's index
//! tree (node structure, sizes, entries). The decoding itself is
//! dialog's own [`dialog_artifacts::inspect`] surface (the same one
//! the native `tree/*` query resolvers are built on): node bytes are
//! fetched by content hash and summarized by `inspect_*`, and keys
//! decompose through `key_components` / `separator_components` — the
//! upstreamed form of the split this inspector originally proved out.
//!
//! Node hashes travel as `#<base58>` strings: that is what each row's
//! `this` carries, and what a `hash`/`child` input term is parsed from,
//! so a row from one operator feeds the next operator's input directly.

use std::collections::BTreeMap;

use base58::{FromBase58, ToBase58};
use dialog_artifacts::inspect::{
    EntrySummary, KeyComponent, KeySummary, NodeSummary, SpanSummary, inspect_entries,
    inspect_keys, inspect_node, inspect_spans, key_components, separator_components,
};
use dialog_artifacts::{
    ATTRIBUTE_KEY_TAG, Artifact, Datum, DialogArtifactsError, ENTITY_KEY_TAG, Key, VALUE_KEY_TAG,
    Value,
};
use dialog_query::Term;
use dialog_repository::{
    Branch, LocalIndex, NetworkedIndex, RepositoryArchiveExt, RepositoryMemoryExt, Upstream,
};
use dialog_storage::{Blake3Hash, StorageBackend};
use ipld_core::ipld::Ipld;
use thiserror::Error;

use crate::{Conclusion, Query, SelectProvider};

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

impl From<DialogArtifactsError> for FormulaError {
    fn from(error: DialogArtifactsError) -> Self {
        Self::Decode(error.to_string())
    }
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

/// Read one node's raw bytes, falling back to the remote when the block is
/// not cached locally — so expanding a not-yet-fetched node transparently
/// pulls it (and caches it) the same way a normal lazy expansion does.
async fn read_node<Env: SelectProvider>(
    branch: &Branch,
    env: &Env,
    hash: Blake3Hash,
) -> Result<Vec<u8>, FormulaError> {
    let index = NetworkedIndex::new(env, branch.archive().index(), remote(branch, env).await);
    index
        .get(&hash)
        .await
        .map_err(|e| FormulaError::Read(e.to_string()))?
        .ok_or_else(|| FormulaError::Read(format!("node {} not found", to_base58(&hash))))
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

/// Read a node's bytes from the *local* archive only (no remote fallback),
/// returning `None` when the block is not cached. This is what the dot's
/// locality reflects (a local hit is cached, a miss would have to be fetched)
/// and lets `tree/child` list a not-cached child from the parent's link
/// without pulling it — the pull happens only when that child is expanded.
async fn read_local<Env: SelectProvider>(
    branch: &Branch,
    env: &Env,
    hash: Blake3Hash,
) -> Result<Option<Vec<u8>>, FormulaError> {
    let index = LocalIndex::new(env, branch.archive().index());
    index
        .get(&hash)
        .await
        .map_err(|e| FormulaError::Read(e.to_string()))
}

/// The scalar fields describing a node: `kind` (`index` for a node of
/// child links, `segment` for a node of entries), byte size, child/entry
/// count, and — for a segment — its upper-bound key (`bound`, raw hex; the
/// decoded components ride `bound-parts`). An index's table holds
/// separators, not whole keys, so it reports no bound of its own (its
/// outline boundary is the link separator `child_rows` stamps).
fn node_fields(bytes: &[u8]) -> Result<BTreeMap<String, Ipld>, FormulaError> {
    let summary: NodeSummary = inspect_node(bytes.to_vec())?;

    let mut fields: BTreeMap<String, Ipld> = BTreeMap::new();
    fields.insert("kind".into(), Ipld::String(summary.kind.into()));
    fields.insert("size".into(), Ipld::Integer(summary.size as i128));
    fields.insert("count".into(), Ipld::Integer(summary.count as i128));
    if summary.kind == "segment" {
        let keys: Vec<KeySummary> = inspect_keys(bytes.to_vec())?;
        if let Some(last) = keys.last() {
            fields.insert("bound".into(), Ipld::String(bytes_hex(&last.key)));
            // The decoded, self-describing components of the bound key — the
            // inspector renders these as textual/colored chips.
            fields.insert("bound-parts".into(), Ipld::List(key_parts(&last.key)));
            // The key's leaf-coin rank under the node's embedded manifest —
            // what decides whether a boundary forms after it. Higher rank ⇒
            // higher in the tree; it's what determines the tree's shape.
            fields.insert("rank".into(), Ipld::Integer(last.rank as i128));
        }
    }
    Ok(fields)
}

/// One `tree/node` conclusion for the node at `hash`.
async fn node_row<Env: SelectProvider>(
    branch: &Branch,
    env: &Env,
    hash: Blake3Hash,
) -> Result<Conclusion, FormulaError> {
    let bytes = read_node(branch, env, hash).await?;
    Ok(Conclusion {
        this: to_base58(&hash),
        fields: node_fields(&bytes)?,
    })
}

/// One `tree/child` conclusion per child of the index node at `hash`.
///
/// Each row is self-contained: it names the child (`child` field + the
/// row's `this`), its sibling position (`at`), and the child's own node
/// fields (kind/size/count), read from the child block. A segment
/// node has no children and yields no rows.
async fn child_rows<Env: SelectProvider>(
    branch: &Branch,
    env: &Env,
    hash: Blake3Hash,
) -> Result<Vec<Conclusion>, FormulaError> {
    let parent = read_node(branch, env, hash).await?;
    if inspect_node(parent.clone())?.kind != "index" {
        return Ok(vec![]); // a segment has no children
    }
    let spans: Vec<SpanSummary> = inspect_spans(parent)?;

    let mut rows = Vec::with_capacity(spans.len());
    for span in spans {
        let child = span.node;
        // Local-only read: a hit is cached, a miss is remote (the block
        // would have to be fetched). A cached child carries its full node
        // fields (size/count/kind); a remote one carries only what the
        // parent's span knows, flagged `cached: false`.
        let mut fields = match read_local(branch, env, child).await? {
            Some(bytes) => {
                let mut fields = node_fields(&bytes)?;
                fields.insert("cached".into(), Ipld::Bool(true));
                fields
            }
            None => {
                let mut fields: BTreeMap<String, Ipld> = BTreeMap::new();
                fields.insert("cached".into(), Ipld::Bool(false));
                fields
            }
        };
        // The child's boundary in the outline is its SPAN SEPARATOR — the
        // left-edge key of the subtree it roots — for both cached and remote
        // children. This is the right thing to show on the left: an index node
        // has no whole upper-bound key of its own (its table holds
        // separators), so without this a cached index row falls back to its
        // opaque hash fragment. The separator is a front-coded PREFIX, so it
        // may decode only partially; `separator_parts` still surfaces its tag
        // and as many leading bytes as the prefix carries. Overrides any
        // `bound`/`bound-parts` a segment child's `node_fields` set from its
        // own upper key, so the whole outline is keyed uniformly by separator.
        fields.insert("bound".into(), Ipld::String(bytes_hex(&span.separator)));
        fields.insert(
            "bound-parts".into(),
            Ipld::List(separator_parts(&span.separator)),
        );
        // The separator's seam rank: the level coin that made this boundary
        // exist (0 for the leftmost span and forced seams).
        fields.insert("rank".into(), Ipld::Integer(span.rank as i128));
        fields.insert("child".into(), Ipld::String(to_base58(&child)));
        fields.insert("at".into(), Ipld::Integer(span.at as i128));
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
    let bytes = read_node(branch, env, hash).await?;
    if inspect_node(bytes.clone())?.kind != "segment" {
        return Ok(vec![]); // an index has no entries
    }
    let keys: Vec<KeySummary> = inspect_keys(bytes.clone())?;
    let entries: Vec<EntrySummary> = inspect_entries(bytes)?;

    let mut rows = Vec::with_capacity(entries.len());
    for entry in entries {
        let key_hex = bytes_hex(&entry.key);
        let mut fields: BTreeMap<String, Ipld> = BTreeMap::new();
        fields.insert("key".into(), Ipld::String(key_hex.clone()));
        // The decoded, self-describing key components for the entry's key row.
        fields.insert("key-parts".into(), Ipld::List(key_parts(&entry.key)));
        fields.insert("at".into(), Ipld::Integer(entry.at as i128));
        if let Some(key) = keys.get(entry.at as usize) {
            fields.insert("rank".into(), Ipld::Integer(key.rank as i128));
        }

        // A segment holds asserted facts; a retraction is a tombstone. The
        // entity/attribute/value all live IN the key (the datum carries only
        // causal metadata), so reconstruct the fact from the key — standing
        // in a placeholder for a spilled value, since the inspector has no
        // store to fetch the block. The synthesized datum carries no cause;
        // the inspector doesn't render one.
        if entry.state == "removed" {
            fields.insert("retracted".into(), Ipld::Bool(true));
        } else {
            let key = Key::from(entry.key.clone());
            let datum = Datum {
                cause: None,
                blob: None,
                version: None,
                collapsed: Vec::new(),
                supersedes: Vec::new(),
                retraction: false,
            };
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

/// One decoded component as the wire's `{ kind, text, hex }` map. `kind`
/// selects the UI's color/glyph, `text` is the human rendering, and `hex`
/// is the raw component bytes for the detail/tooltip.
fn component_part(component: &KeyComponent) -> Ipld {
    let mut m: BTreeMap<String, Ipld> = BTreeMap::new();
    m.insert("kind".into(), Ipld::String(component.kind.into()));
    m.insert("text".into(), Ipld::String(component.text.clone()));
    m.insert("hex".into(), Ipld::String(bytes_hex(&component.bytes)));
    Ipld::Map(m)
}

/// Decode a raw key into structured, self-describing parts for the tree
/// inspector, via dialog's own `key_components`.
fn key_parts(bytes: &[u8]) -> Vec<Ipld> {
    key_components(bytes).iter().map(component_part).collect()
}

/// Decode an index node's SPAN SEPARATOR into parts, via dialog's own
/// `separator_components`. Dialog reports the post-tag prefix as one
/// structural `prefix` component; recolor it by the ordering's *leading*
/// sort column (entity for EAV/history, attribute for AEV, value for VAE)
/// so the outline's left edge reads in the right hue — `\0concept:J4J64…`
/// shows as `concept:J4J64…` in entity-blue rather than structural gray.
fn separator_parts(bytes: &[u8]) -> Vec<Ipld> {
    let lead_kind = match bytes.first() {
        Some(&ENTITY_KEY_TAG) => "entity",
        Some(&ATTRIBUTE_KEY_TAG) => "attribute",
        Some(&VALUE_KEY_TAG) => "value",
        _ => "opaque",
    };
    separator_components(bytes)
        .iter()
        .map(|component| match component.kind {
            "prefix" => component_part(&KeyComponent {
                kind: lead_kind,
                text: component.text.clone(),
                bytes: component.bytes.clone(),
            }),
            _ => component_part(component),
        })
        .collect()
}

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

/// The human name of a key's index ordering, from its leading tag byte.
fn tag_name(tag: u8) -> &'static str {
    match tag {
        ENTITY_KEY_TAG => "entity",
        ATTRIBUTE_KEY_TAG => "attribute",
        VALUE_KEY_TAG => "value",
        dialog_artifacts::HISTORY_KEY_TAG => "history",
        dialog_artifacts::BLOB_KEY_TAG => "blob",
        dialog_artifacts::COVERAGE_KEY_TAG => "coverage",
        _ => "unknown",
    }
}

/// One `tree/key` conclusion: the key's decoded components. The tag names
/// the index ordering (entity / attribute / value); the entity, attribute,
/// value type, and (for an inline key) value are reconstructed from the
/// key. A spilled value shows the placeholder.
fn key_row(key: Key) -> Conclusion {
    let mut fields: BTreeMap<String, Ipld> = BTreeMap::new();
    fields.insert("tag".into(), Ipld::String(tag_name(key.tag()).into()));
    // The decoded, self-describing components — the single source of truth for
    // the inspector's key rendering (entity/attribute/value-type/value chips).
    fields.insert("parts".into(), Ipld::List(key_parts(key.as_ref())));

    Conclusion {
        this: bytes_hex(key.as_ref()),
        fields,
    }
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
