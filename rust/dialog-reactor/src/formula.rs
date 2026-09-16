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
    BlobEntrySummary, EntrySummary, KeyComponent, KeySummary, NodeSummary, SpanSummary,
    inspect_blob_records, inspect_entries, inspect_keys, inspect_manifest, inspect_node,
    inspect_spans, key_components, separator_components,
};
use dialog_artifacts::{
    ATTRIBUTE_KEY_TAG, Artifact, BLOB_KEY_TAG, COVERAGE_KEY_TAG, Datum, DialogArtifactsError,
    ENTITY_KEY_TAG, Entity, HISTORY_KEY_TAG, Key, VALUE_KEY_TAG, Value,
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
    // The subtree's advisory scale code (a log-scale entry-count estimate)
    // and the hitchhiker ops buffered on this node — the two numbers that
    // explain the tree's SHAPE rather than its contents. Novelty is always
    // 0 for a segment; it is reported anyway so the field never vanishes.
    fields.insert("scale".into(), Ipld::Integer(summary.scale as i128));
    fields.insert("novelty".into(), Ipld::Integer(summary.novelty as i128));
    // Every node embeds the manifest it was written under, so the
    // configuration that produced this shape is readable off the node
    // itself rather than assumed from the current defaults.
    if let Ok(manifest) = inspect_manifest(bytes.to_vec()) {
        let mut m: BTreeMap<String, Ipld> = BTreeMap::new();
        m.insert("version".into(), Ipld::Integer(manifest.version as i128));
        m.insert("fanout".into(), Ipld::Integer(1i128 << manifest.fanout_n));
        m.insert(
            "max-separator".into(),
            Ipld::Integer(manifest.max_separator as i128),
        );
        m.insert("inline".into(), Ipld::Integer(manifest.inline_n as i128));
        m.insert(
            "spill-prefix".into(),
            Ipld::Integer(manifest.spill_prefix as i128),
        );
        m.insert(
            "max-segment".into(),
            Ipld::Integer(manifest.max_segment as i128),
        );
        fields.insert("manifest".into(), Ipld::Map(m));
    }
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
/// row's `this`), its sibling position (`at`), the child's own node
/// fields (kind/size/count), read from the child block, and the ops the
/// parent still buffers against that span (`pending`). A segment node has
/// no children and yields no rows.
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
                // The parent's span records the subtree's advisory scale, so
                // an unfetched child still says how much sits beneath it.
                fields.insert("scale".into(), Ipld::Integer(span.scale as i128));
                fields
            }
        };
        // Ops buffered against this span IN THE PARENT — work destined for
        // this subtree that has not been written down into it. Distinct
        // from the child's own `novelty` (what it buffers for its own
        // children), and known whether or not the child is cached.
        fields.insert("pending".into(), Ipld::Integer(span.novelty as i128));
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
    segment_rows(read_node(branch, env, hash).await?)
}

/// The `tree/entry` rows a segment node's bytes decode to — the whole of
/// what `tree/entry` does once the block is in hand, so the decoding is
/// exercisable over a node built in a test rather than only through a
/// branch.
fn segment_rows(bytes: Vec<u8>) -> Result<Vec<Conclusion>, FormulaError> {
    if inspect_node(bytes.clone())?.kind != "segment" {
        return Ok(vec![]); // an index has no entries
    }
    let keys: Vec<KeySummary> = inspect_keys(bytes.clone())?;
    let entries: Vec<EntrySummary> = inspect_entries(bytes.clone())?;

    let mut rows = Vec::with_capacity(entries.len());
    for entry in entries {
        let key_hex = bytes_hex(&entry.key);
        // Decode the key ONCE: the components are both the row's
        // self-describing `key-parts` and where the value type is read
        // from when the value itself is not in the key (a spill).
        let components = key_components(&entry.key);
        let mut fields: BTreeMap<String, Ipld> = BTreeMap::new();
        fields.insert("key".into(), Ipld::String(key_hex.clone()));
        // The decoded, self-describing key components for the entry's key row.
        fields.insert(
            "key-parts".into(),
            Ipld::List(components.iter().map(component_part).collect()),
        );
        fields.insert("at".into(), Ipld::Integer(entry.at as i128));
        if let Some(key) = keys.get(entry.at as usize) {
            fields.insert("rank".into(), Ipld::Integer(key.rank as i128));
        }

        // The index a key belongs to, read off its leading tag. A segment
        // holds more than entity-attribute-value facts: history and
        // coverage records, blob-index records. Naming the ordering lets
        // the inspector render each for what it is instead of trying to
        // read them all as facts.
        let ordering = entry
            .key
            .first()
            .copied()
            .map(tag_name)
            .unwrap_or("unknown");
        fields.insert("ordering".into(), Ipld::String(ordering.into()));

        // Claim metadata: which version wrote the entry, what it descends
        // from, and how much was folded into it. This is what makes a
        // history or coverage record legible — a covering record is the
        // one that supersedes prior versions.
        if !entry.origin.is_empty() {
            fields.insert("origin".into(), Ipld::String(bytes_hex(&entry.origin)));
        }
        fields.insert("edition".into(), Ipld::Integer(entry.edition as i128));
        fields.insert("cause".into(), Ipld::Integer(entry.cause as i128));
        fields.insert("collapsed".into(), Ipld::Integer(entry.collapsed as i128));
        fields.insert("supersedes".into(), Ipld::Integer(entry.supersedes as i128));
        fields.insert("retraction".into(), Ipld::Bool(entry.retraction));
        if let Some(spill) = &entry.spill {
            fields.insert("spill".into(), Ipld::String(to_base58(spill)));
        }

        // A segment holds asserted facts; a retraction is a tombstone. The
        // entity/attribute/value all live IN the key (the datum carries only
        // causal metadata), so reconstruct the fact from the key.
        //
        // A history or coverage key carries the same fact behind a version
        // prefix, so it reconstructs too (see [`fact_key`]) — without that,
        // those rows reported no entity, attribute or value at all and the
        // whole record read as blank. A blob key names content rather than a
        // fact and reconstructs nothing; it keeps its key components and
        // metadata, and its own fields are merged in below.
        if entry.state == "removed" {
            fields.insert("retracted".into(), Ipld::Bool(true));
        } else {
            fields.extend(fact_fields(&entry.key, &components, entry.spill.is_some()));
        }

        rows.push(Conclusion {
            this: key_hex,
            fields,
        });
    }

    // Blob-index entries carry no claim metadata, so they come back from
    // the entry walk with everything empty; their real payload (the
    // referenced hash, its size and record version) lives in a parallel
    // index. Merge it in by segment position so a blob row is a blob row
    // rather than an entry that appears to say nothing.
    //
    // A record that does not decode (an encoding version this build does
    // not know) is not fatal: the rest of the segment still reads, and the
    // undecodable rows keep their keys. Propagating the error instead
    // blanked the whole segment over one unknown record.
    for record in inspect_blob_records(bytes).unwrap_or_default() {
        let Some(row) = rows
            .iter_mut()
            .find(|row| row.fields.get("at") == Some(&Ipld::Integer(record.at as i128)))
        else {
            continue;
        };
        row.fields.extend(blob_fields(&record));
    }
    Ok(rows)
}

/// The fields a blob-index row carries: the referenced hash, the entity it
/// is addressed by, its size, and the record's encoding version.
///
/// The hash reads as the `#<base58>` every other hash in `tree/*` uses, and
/// as the `blob:<base58>` ENTITY the same bytes are named by everywhere
/// else in the branch — so a blob row names the subject its metadata facts
/// (`xyz.tonk.blob/name` and friends) hang off, rather than a bare hex run
/// that matches nothing else on screen.
fn blob_fields(record: &BlobEntrySummary) -> BTreeMap<String, Ipld> {
    let mut fields: BTreeMap<String, Ipld> = BTreeMap::new();
    match Blake3Hash::try_from(record.blob.as_slice()) {
        Ok(hash) => {
            fields.insert("blob".into(), Ipld::String(to_base58(&hash)));
            if let Ok(entity) = Entity::from_blob(&hash) {
                fields.insert("entity".into(), Ipld::String(entity.to_string()));
            }
        }
        // Not 32 bytes: not a hash we can render as one.
        Err(_) => {
            fields.insert("blob".into(), Ipld::String(bytes_hex(&record.blob)));
        }
    }
    fields.insert("blob-size".into(), Ipld::Integer(record.size as i128));
    fields.insert("blob-version".into(), Ipld::Integer(record.version as i128));
    fields
}

/// The fact an entry's key carries, as the row's `entity` / `attribute` /
/// `type` / `value` fields. Empty for a key that carries no fact (a blob
/// record) or does not decode under its ordering.
///
/// `components` is the key's already-decoded components (the value type is
/// read from there when the value itself is not in the key), and `spilled`
/// says whether the value spilled to the archive.
fn fact_fields(key: &[u8], components: &[KeyComponent], spilled: bool) -> BTreeMap<String, Ipld> {
    let mut fields: BTreeMap<String, Ipld> = BTreeMap::new();
    let Some(key) = fact_key(key) else {
        return fields;
    };
    // The entity, attribute and value all live in the key; the datum
    // carries only causal metadata, so a placeholder one is enough to
    // reconstruct with.
    let datum = Datum {
        cause: None,
        blob: None,
        version: None,
        collapsed: Vec::new(),
        supersedes: Vec::new(),
        retraction: false,
    };
    let Ok(artifact) = Artifact::from_key_datum_placeholder(&key, &datum) else {
        return fields;
    };
    fields.insert("entity".into(), Ipld::String(artifact.of.to_string()));
    fields.insert("attribute".into(), Ipld::String(artifact.the.to_string()));
    if spilled {
        // A spilled value's bytes live in an archive block the inspector has
        // no store to fetch, and the placeholder artifact stands a
        // `<spilled value>` STRING in for them — whose type would misreport
        // the value's own. Report the type the key records and no value; the
        // row's `spill` reference is what there is to show.
        if let Some(vtype) = components.iter().find(|part| part.kind == "vtype") {
            fields.insert("type".into(), Ipld::String(vtype.text.clone()));
        }
    } else {
        fields.insert(
            "type".into(),
            Ipld::String(artifact.is.data_type().to_string()),
        );
        if let Some(ipld) = value_to_ipld(&artifact.is) {
            fields.insert("value".into(), ipld);
        }
    }
    fields
}

/// The EAV-shaped key an entry's fact reconstructs from, or `None` for an
/// entry that carries no fact.
///
/// An entity / attribute / value key is one already. A history or coverage
/// key is the *same* fact behind a fixed-width version prefix — tag ‖
/// origin(32) ‖ edition(8) ‖ entity ‖ attribute ‖ value slot — so dropping
/// the prefix and re-tagging the tail yields exactly the entity-ordered key
/// the record's claim reconstructs from. (This mirrors what dialog's own
/// `key_components` does to decode those keys.) A blob key names content,
/// not a fact, and has nothing to reconstruct.
fn fact_key(bytes: &[u8]) -> Option<Key> {
    /// Width of the version prefix a history/coverage key carries after its
    /// tag: a 32-byte origin and an 8-byte big-endian edition.
    const VERSION_PREFIX: usize = 32 + 8;

    match bytes.first().copied()? {
        ENTITY_KEY_TAG | ATTRIBUTE_KEY_TAG | VALUE_KEY_TAG => Some(Key::from(bytes.to_vec())),
        HISTORY_KEY_TAG | COVERAGE_KEY_TAG => {
            let tail = bytes.get(1 + VERSION_PREFIX..)?;
            if tail.is_empty() {
                return None;
            }
            let mut synthetic = Vec::with_capacity(1 + tail.len());
            synthetic.push(ENTITY_KEY_TAG);
            synthetic.extend_from_slice(tail);
            Some(Key::from(synthetic))
        }
        _ => None,
    }
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
    separator_components(bytes)
        .iter()
        .flat_map(|component| match component.kind {
            "prefix" => prefix_fields(bytes.first().copied(), component),
            _ => vec![component_part(component)],
        })
        .collect()
}

/// Split a separator's front-coded `prefix` into the key FIELDS it spans.
///
/// Dialog reports everything after the tag as one opaque `prefix`, but a
/// separator is a truncated key: its bytes are the leading fields of a
/// real key, NUL-delimited, cut wherever the prefix ends. Painting the
/// whole run one colour mislabels it — `db:concept␀db.meta/concept␀d`
/// showed as a single entity when it is an entity, an attribute, and the
/// first byte of the next entity. Split on the delimiters and colour each
/// field by its position in the ordering, so a separator reads with the
/// same colour code as a full key.
///
/// The final field is usually truncated mid-value (that is the point of
/// front-coding), so it is reported as-is; the key's own components would
/// be wrong to synthesize here.
fn prefix_fields(tag: Option<u8>, component: &KeyComponent) -> Vec<Ipld> {
    // Field order per index, mirroring dialog's `key_components`: the
    // ordering's leading sort column comes first.
    let order: &[&str] = match tag {
        Some(ENTITY_KEY_TAG) => &["entity", "attribute", "vtype", "value"],
        Some(ATTRIBUTE_KEY_TAG) => &["attribute", "entity", "vtype", "value"],
        Some(VALUE_KEY_TAG) => &["vtype", "value", "attribute", "entity"],
        // A history or coverage separator opens with a 32-byte origin
        // and an 8-byte big-endian edition — raw binary, which renders
        // as a run of overlapping control glyphs if passed through as
        // text. Report them as the version they encode, then read what
        // follows as the entity-ordered fact fields it is.
        Some(HISTORY_KEY_TAG) | Some(COVERAGE_KEY_TAG) => {
            return version_fields(&component.bytes);
        }
        // A blob separator's prefix is part of a 32-byte content hash:
        // raw binary with no text in it at all, which as utf8-lossy text
        // renders as a run of replacement characters. Report it as the
        // hash prefix it is.
        Some(BLOB_KEY_TAG) => {
            return vec![component_part(&KeyComponent {
                kind: "blob",
                text: format!(
                    "blob:{}",
                    bytes_hex(&component.bytes).trim_start_matches("0x")
                ),
                bytes: component.bytes.clone(),
            })];
        }
        // An unknown tag has no field layout at all, so leave it opaque
        // rather than colouring it by a layout it does not have.
        _ => return vec![component_part(component)],
    };

    // In value-ordering the key opens with a one-byte VALUE TYPE tag
    // rather than a NUL-delimited field, so it has to be peeled off
    // before splitting — otherwise it fuses with the value that follows
    // and every later field lands one slot early (an attribute showing
    // up where the value belongs).
    let peel_vtype = tag == Some(VALUE_KEY_TAG);
    field_parts(&component.bytes, order, peel_vtype)
}

/// Split a run of NUL-delimited key fields into components, labelling each
/// by its position in `order`. `peel_vtype` takes the leading byte as a
/// value-type tag first (the value ordering opens with one rather than
/// with a delimited field).
fn field_parts(bytes: &[u8], order: &[&'static str], peel_vtype: bool) -> Vec<Ipld> {
    let mut parts = Vec::new();
    let mut rest = bytes;
    let mut next = 0usize;
    if peel_vtype && let Some((vtype, tail)) = rest.split_first() {
        parts.push(component_part(&KeyComponent {
            kind: "vtype",
            text: value_type_name(*vtype),
            bytes: vec![*vtype],
        }));
        rest = tail;
        next = 1;
    }

    parts.extend(
        rest.split(|b| *b == 0)
            .filter(|field| !field.is_empty())
            .enumerate()
            .map(|(index, field)| {
                component_part(&KeyComponent {
                    kind: order.get(next + index).copied().unwrap_or("opaque"),
                    text: String::from_utf8_lossy(field).into_owned(),
                    bytes: field.to_vec(),
                })
            }),
    );
    parts
}

/// Decode a history/coverage separator's leading bytes into readable
/// version fields: a 32-byte origin and an 8-byte big-endian edition.
///
/// Front-coding truncates anywhere, so each field is emitted only if
/// wholly present; whatever follows the version is the EAV tail, which
/// is reported as one component rather than guessed at.
fn version_fields(bytes: &[u8]) -> Vec<Ipld> {
    const ORIGIN: usize = 32;
    const EDITION: usize = 8;

    let mut parts = Vec::new();
    let Some(origin) = bytes.get(..ORIGIN) else {
        // Too short to carry a whole origin — nothing decodable.
        return vec![component_part(&KeyComponent {
            kind: "opaque",
            text: bytes_hex(bytes),
            bytes: bytes.to_vec(),
        })];
    };
    parts.push(component_part(&KeyComponent {
        kind: "origin",
        text: format!("origin:{}", bytes_hex(origin).trim_start_matches("0x")),
        bytes: origin.to_vec(),
    }));

    let rest = &bytes[ORIGIN..];
    if let Some(edition) = rest.get(..EDITION) {
        let n = edition.iter().fold(0u64, |acc, b| (acc << 8) | *b as u64);
        parts.push(component_part(&KeyComponent {
            kind: "edition",
            text: format!("@{n}"),
            bytes: edition.to_vec(),
        }));
        // What follows the version is the fact, in the entity ordering —
        // NUL-delimited entity, attribute, value slot, cut wherever the
        // front-coding ended. Split it like any other separator tail
        // rather than painting the whole run as one entity.
        parts.extend(field_parts(
            &rest[EDITION..],
            &["entity", "attribute", "vtype", "value"],
            false,
        ));
    } else if !rest.is_empty() {
        parts.push(component_part(&KeyComponent {
            kind: "opaque",
            text: bytes_hex(rest),
            bytes: rest.to_vec(),
        }));
    }
    parts
}

/// The human name of a value-type tag, matching what dialog's own
/// `key_components` renders for a `vtype` component (which formats the
/// decoded [`ValueDataType`]). The tags are the enum's discriminants.
fn value_type_name(tag: u8) -> String {
    match tag {
        0 => "Bytes",
        1 => "Entity",
        2 => "Boolean",
        3 => "String",
        4 => "UnsignedInt",
        5 => "SignedInt",
        6 => "Float",
        7 => "Record",
        8 => "Symbol",
        _ => return format!("type {tag}"),
    }
    .to_owned()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    wasm_bindgen_test_configure!(run_in_browser);

    /// Read a component list back as `(kind, text)` pairs.
    fn kinds(parts: &[Ipld]) -> Vec<(String, String)> {
        parts
            .iter()
            .map(|part| {
                let Ipld::Map(m) = part else {
                    panic!("component must be a map")
                };
                let get = |k: &str| match m.get(k) {
                    Some(Ipld::String(s)) => s.clone(),
                    other => panic!("{k} must be a string, got {other:?}"),
                };
                (get("kind"), get("text"))
            })
            .collect()
    }

    /// A separator is a front-coded prefix spanning SEVERAL key fields,
    /// NUL-delimited. Painting the run one colour mislabels it — the
    /// reported symptom was `db:concept␀db.meta/concept␀d` rendering as
    /// a single entity when it is an entity, an attribute, and the first
    /// byte of the next field.
    #[dialog_common::test]
    fn it_splits_an_entity_separator_into_its_fields() {
        let mut bytes = vec![ENTITY_KEY_TAG];
        bytes.extend_from_slice(b"concept:J4J64\0db.concept.with/n");

        let parts = kinds(&separator_parts(&bytes));

        assert_eq!(
            parts,
            vec![
                ("index".to_owned(), "entity".to_owned()),
                ("entity".to_owned(), "concept:J4J64".to_owned()),
                ("attribute".to_owned(), "db.concept.with/n".to_owned()),
            ]
        );
    }

    /// Value ordering opens with a one-byte VALUE TYPE tag rather than a
    /// NUL-delimited field. Splitting without peeling it off fuses it to
    /// the value that follows and shifts every later field one slot
    /// early — the attribute then lands where the value belongs.
    #[dialog_common::test]
    fn it_peels_the_value_type_tag_before_splitting() {
        let mut bytes = vec![VALUE_KEY_TAG];
        bytes.push(1); // ValueDataType::Entity
        bytes.extend_from_slice(b"db:concept\0db.meta/concept\0d");

        let parts = kinds(&separator_parts(&bytes));

        assert_eq!(
            parts,
            vec![
                ("index".to_owned(), "value".to_owned()),
                ("vtype".to_owned(), "Entity".to_owned()),
                ("value".to_owned(), "db:concept".to_owned()),
                ("attribute".to_owned(), "db.meta/concept".to_owned()),
                ("entity".to_owned(), "d".to_owned()),
            ]
        );
    }

    /// A history separator opens with a 32-byte origin and an 8-byte
    /// big-endian edition. Passed through as text those render as a run
    /// of overlapping control glyphs, so they decode to the version they
    /// encode instead.
    #[dialog_common::test]
    fn it_decodes_a_history_separator_as_a_version() {
        let mut bytes = vec![HISTORY_KEY_TAG];
        bytes.extend_from_slice(&[0xab; 32]);
        bytes.extend_from_slice(&7u64.to_be_bytes());

        let parts = kinds(&separator_parts(&bytes));

        assert_eq!(parts[0], ("index".to_owned(), "history".to_owned()));
        assert_eq!(parts[1].0, "origin");
        assert!(
            parts[1].1.starts_with("origin:abab"),
            "origin renders as hex, got {:?}",
            parts[1].1
        );
        assert_eq!(parts[2], ("edition".to_owned(), "@7".to_owned()));
    }

    /// Front-coding truncates anywhere, so a separator may carry only
    /// part of the version. A partial origin stays opaque rather than
    /// being decoded from bytes that are not all there.
    #[dialog_common::test]
    fn it_leaves_a_truncated_history_separator_opaque() {
        let mut bytes = vec![HISTORY_KEY_TAG];
        bytes.extend_from_slice(&[0xab; 8]); // short of a whole origin

        let parts = kinds(&separator_parts(&bytes));

        assert_eq!(parts[0], ("index".to_owned(), "history".to_owned()));
        assert_eq!(parts[1].0, "opaque");
    }

    /// Build the `(origin, edition)` version a history/coverage key is
    /// written under.
    fn version(edition: u64) -> dialog_artifacts::history::Version {
        dialog_artifacts::history::Version::new(
            dialog_artifacts::history::Origin([0xab; 32]),
            dialog_artifacts::history::Edition::new(edition),
        )
    }

    /// Read a fields map's string field.
    fn text(fields: &BTreeMap<String, Ipld>, key: &str) -> Option<String> {
        match fields.get(key) {
            Some(Ipld::String(s)) => Some(s.clone()),
            _ => None,
        }
    }

    /// A history record carries the SAME fact as the entry it records,
    /// behind a version prefix. The inspector used to reconstruct only
    /// EAV-shaped keys, so every history row rendered with a blank entity
    /// and value and its ordering name squatting in the attribute column.
    #[dialog_common::test]
    fn it_reconstructs_the_fact_a_history_record_carries() {
        let of: dialog_artifacts::Entity = "test:subject".parse().expect("entity parses");
        let the: dialog_artifacts::Attribute = "test/name".parse().expect("attribute parses");
        let is = Value::String("Zaphod".into());
        let key = dialog_artifacts::history_key(
            &version(7),
            &of,
            &the,
            &is,
            &dialog_search_tree::Manifest::default(),
        );

        let fields = fact_fields(key.as_ref(), &key_components(key.as_ref()), false);

        assert_eq!(text(&fields, "entity").as_deref(), Some("test:subject"));
        assert_eq!(text(&fields, "attribute").as_deref(), Some("test/name"));
        assert_eq!(text(&fields, "type").as_deref(), Some("String"));
        assert_eq!(fields.get("value"), Some(&Ipld::String("Zaphod".into())));
    }

    /// A coverage entry keeps the claim it covers in its key but carries
    /// the value only as a spilled reference (that is what keeps the
    /// region value-free). The row reports the entity and attribute, the
    /// value type the KEY records — not the `<spilled value>` placeholder's
    /// `String` — and no value.
    #[dialog_common::test]
    fn it_reconstructs_a_coverage_record_without_inventing_a_value() {
        let of: dialog_artifacts::Entity = "test:subject".parse().expect("entity parses");
        let the: dialog_artifacts::Attribute = "test/age".parse().expect("attribute parses");
        let key = dialog_artifacts::coverage_key(&version(3), &of, &the, &Value::UnsignedInt(42));

        let fields = fact_fields(key.as_ref(), &key_components(key.as_ref()), true);

        assert_eq!(text(&fields, "entity").as_deref(), Some("test:subject"));
        assert_eq!(text(&fields, "attribute").as_deref(), Some("test/age"));
        assert_eq!(text(&fields, "type").as_deref(), Some("Bytes"));
        assert_eq!(fields.get("value"), None, "a spilled value is not invented");
    }

    /// An ordinary fact key still reconstructs, in every ordering.
    #[dialog_common::test]
    fn it_reconstructs_a_fact_in_every_ordering() {
        let artifact = dialog_artifacts::Artifact {
            the: "test/name".parse().expect("attribute parses"),
            of: "test:subject".parse().expect("entity parses"),
            is: Value::String("Trillian".into()),
            cause: None,
        };
        let manifest = dialog_search_tree::Manifest::default();
        use dialog_artifacts::KeyType as _;
        for key in [
            dialog_artifacts::EntityKey::<Key>::from_artifact(&artifact, &manifest)
                .bytes()
                .to_vec(),
            dialog_artifacts::AttributeKey::<Key>::from_artifact(&artifact, &manifest)
                .bytes()
                .to_vec(),
            dialog_artifacts::ValueKey::<Key>::from_artifact(&artifact, &manifest)
                .bytes()
                .to_vec(),
        ] {
            let fields = fact_fields(&key, &key_components(&key), false);
            assert_eq!(text(&fields, "entity").as_deref(), Some("test:subject"));
            assert_eq!(text(&fields, "attribute").as_deref(), Some("test/name"));
            assert_eq!(fields.get("value"), Some(&Ipld::String("Trillian".into())));
        }
    }

    /// A blob key names content, not a fact: it reconstructs nothing, and
    /// the row is furnished by its blob record instead.
    #[dialog_common::test]
    fn it_reports_no_fact_for_a_blob_key() {
        let key = dialog_artifacts::BlobKey::new(&[0x11; 32]).into_key();

        let fields = fact_fields(key.as_ref(), &key_components(key.as_ref()), false);

        assert!(fields.is_empty(), "a blob key carries no fact: {fields:?}");
    }

    /// A blob row names its blob the way the rest of the branch does: the
    /// `#<base58>` content hash, and the `blob:<base58>` entity the same
    /// bytes carry their metadata facts under. A raw hex run named nothing
    /// the inspector could cross-reference.
    #[dialog_common::test]
    fn it_names_a_blob_row_by_its_hash_and_entity() {
        let hash = [0x11u8; 32];
        let record = BlobEntrySummary {
            at: 0,
            blob: hash.to_vec(),
            version: 1,
            size: 4096,
        };

        let fields = blob_fields(&record);

        assert_eq!(
            text(&fields, "blob").as_deref(),
            Some(to_base58(&hash).as_str())
        );
        assert_eq!(
            text(&fields, "entity"),
            Some(
                dialog_artifacts::Entity::from_blob(&hash)
                    .expect("blob entity")
                    .to_string()
            )
        );
        assert_eq!(fields.get("blob-size"), Some(&Ipld::Integer(4096)));
        assert_eq!(fields.get("blob-version"), Some(&Ipld::Integer(1)));
    }

    /// A blob hash that is not 32 bytes is not renderable as one, and
    /// falls back to hex rather than being dropped.
    #[dialog_common::test]
    fn it_falls_back_to_hex_for_an_unrenderable_blob_hash() {
        let record = BlobEntrySummary {
            at: 0,
            blob: vec![0x01, 0x02],
            version: 1,
            size: 2,
        };

        let fields = blob_fields(&record);

        assert_eq!(text(&fields, "blob").as_deref(), Some("0x0102"));
        assert_eq!(fields.get("entity"), None);
    }

    /// Build a segment node holding `entries`, the way the tree stores
    /// them, so the decode runs over real node bytes.
    fn segment(entries: Vec<(Key, dialog_artifacts::State<Datum>)>) -> Vec<u8> {
        use dialog_search_tree::{Entry, PersistentNodeBody};
        let entries: Vec<Entry<Key, dialog_artifacts::State<Datum>>> = entries
            .into_iter()
            .map(|(key, value)| Entry { key, value })
            .collect();
        PersistentNodeBody::<dialog_artifacts::State<Datum>>::segment_from_entries::<Key>(
            entries,
            dialog_search_tree::Manifest::default(),
        )
        .expect("segment builds")
        .as_bytes()
        .expect("segment serializes")
        .as_ref()
        .to_vec()
    }

    /// The row for the entry at `ordering`, of the rows `segment_rows`
    /// produced.
    fn row_of<'a>(rows: &'a [Conclusion], ordering: &str) -> &'a Conclusion {
        rows.iter()
            .find(|row| row.fields.get("ordering") == Some(&Ipld::String(ordering.into())))
            .unwrap_or_else(|| panic!("a {ordering} row among {rows:?}"))
    }

    /// A segment holds more than facts, and every kind of entry in one has
    /// to read as what it is. Over a real node carrying a fact, a history
    /// record, a coverage record and a blob reference: each row names its
    /// index, and the three that carry a fact report its entity, attribute
    /// and value — the history row used to report none of them.
    #[dialog_common::test]
    fn it_decodes_every_region_of_a_mixed_segment() {
        use dialog_artifacts::{KeyType as _, State};
        let manifest = dialog_search_tree::Manifest::default();
        let of: dialog_artifacts::Entity = "test:subject".parse().expect("entity parses");
        let the: dialog_artifacts::Attribute = "test/name".parse().expect("attribute parses");
        let is = Value::String("Ford".into());
        let at = version(9);

        let fact = dialog_artifacts::Artifact {
            the: the.clone(),
            of: of.clone(),
            is: is.clone(),
            cause: None,
        };
        let datum = |version| {
            State::Added(Datum {
                cause: None,
                blob: None,
                version,
                collapsed: Vec::new(),
                supersedes: Vec::new(),
                retraction: false,
            })
        };
        let blob_hash = [0x22u8; 32];
        let mut record = vec![1u8];
        record.extend_from_slice(&1234u64.to_be_bytes());

        let mut entries = vec![
            (
                Key::from(
                    dialog_artifacts::EntityKey::<Key>::from_artifact(&fact, &manifest)
                        .bytes()
                        .to_vec(),
                ),
                datum(Some(at)),
            ),
            (
                dialog_artifacts::history_key(&at, &of, &the, &is, &manifest),
                datum(Some(at)),
            ),
            (
                dialog_artifacts::coverage_key(&at, &of, &the, &is),
                datum(Some(at)),
            ),
            (
                dialog_artifacts::BlobKey::new(&blob_hash).into_key(),
                State::Added(Datum {
                    cause: None,
                    blob: Some(record),
                    version: None,
                    collapsed: Vec::new(),
                    supersedes: Vec::new(),
                    retraction: false,
                }),
            ),
        ];
        // A segment stores its entries in key order.
        entries.sort_by(|a, b| a.0.as_ref().cmp(b.0.as_ref()));

        let rows = segment_rows(segment(entries)).expect("segment decodes");

        assert_eq!(rows.len(), 4, "every entry yields a row: {rows:?}");

        // The fact and the history record both read as the fact they carry.
        for ordering in ["entity", "history"] {
            let fields = &row_of(&rows, ordering).fields;
            assert_eq!(
                fields.get("entity"),
                Some(&Ipld::String("test:subject".into())),
                "{ordering} row names its entity: {fields:?}"
            );
            assert_eq!(
                fields.get("attribute"),
                Some(&Ipld::String("test/name".into())),
                "{ordering} row names its attribute: {fields:?}"
            );
            assert_eq!(
                fields.get("value"),
                Some(&Ipld::String("Ford".into())),
                "{ordering} row carries its value: {fields:?}"
            );
        }

        // The history row also carries the revision that wrote it.
        let history = &row_of(&rows, "history").fields;
        assert_eq!(history.get("edition"), Some(&Ipld::Integer(9)));

        // Coverage keeps the claim but not its value (that is what keeps
        // the region cheap to diff), so it names the claim and its spill.
        let coverage = &row_of(&rows, "coverage").fields;
        assert_eq!(
            coverage.get("entity"),
            Some(&Ipld::String("test:subject".into()))
        );
        assert_eq!(coverage.get("value"), None, "coverage carries no value");
        assert!(
            matches!(coverage.get("spill"), Some(Ipld::String(s)) if s.starts_with('#')),
            "coverage names the value it covers by reference: {coverage:?}"
        );

        // The blob row names its content, by hash and by the entity the
        // same bytes carry their metadata under, plus the size.
        let blob = &row_of(&rows, "blob").fields;
        assert_eq!(blob.get("blob"), Some(&Ipld::String(to_base58(&blob_hash))));
        assert_eq!(
            blob.get("entity"),
            Some(&Ipld::String(
                Entity::from_blob(&blob_hash)
                    .expect("blob entity")
                    .to_string()
            ))
        );
        assert_eq!(blob.get("blob-size"), Some(&Ipld::Integer(1234)));
    }

    /// A blob separator is part of a raw content hash — no text in it at
    /// all. Passing it through as utf8-lossy text painted the outline with
    /// replacement characters; it reads as the hash prefix it is.
    #[dialog_common::test]
    fn it_renders_a_blob_separator_as_a_hash_prefix() {
        let mut bytes = vec![BLOB_KEY_TAG];
        bytes.extend_from_slice(&[0xde, 0xad, 0xbe, 0xef]);

        let parts = kinds(&separator_parts(&bytes));

        assert_eq!(parts[0], ("index".to_owned(), "blob".to_owned()));
        assert_eq!(parts[1], ("blob".to_owned(), "blob:deadbeef".to_owned()));
    }

    /// Past a history separator's version prefix the bytes are the fact,
    /// NUL-delimited under the entity ordering. They split into their own
    /// fields rather than being painted as one long entity.
    #[dialog_common::test]
    fn it_splits_the_fact_behind_a_history_separator() {
        let mut bytes = vec![HISTORY_KEY_TAG];
        bytes.extend_from_slice(&[0xab; 32]);
        bytes.extend_from_slice(&4u64.to_be_bytes());
        bytes.extend_from_slice(b"test:subject\0test/name");

        let parts = kinds(&separator_parts(&bytes));

        assert_eq!(
            parts,
            vec![
                ("index".to_owned(), "history".to_owned()),
                ("origin".to_owned(), format!("origin:{}", "ab".repeat(32))),
                ("edition".to_owned(), "@4".to_owned()),
                ("entity".to_owned(), "test:subject".to_owned()),
                ("attribute".to_owned(), "test/name".to_owned()),
            ]
        );
    }
}
