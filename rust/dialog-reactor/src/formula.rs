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
    EntrySummary, KeyComponent, KeySummary, NodeSummary, SpanSummary, inspect_blob_records,
    inspect_entries, inspect_keys, inspect_manifest, inspect_node, inspect_spans, key_components,
    separator_components,
};
use dialog_artifacts::{
    ATTRIBUTE_KEY_TAG, Artifact, COVERAGE_KEY_TAG, Datum, DialogArtifactsError, ENTITY_KEY_TAG,
    HISTORY_KEY_TAG, Key, VALUE_KEY_TAG, Value,
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
    let entries: Vec<EntrySummary> = inspect_entries(bytes.clone())?;

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
        // causal metadata), so reconstruct the fact from the key — standing
        // in a placeholder for a spilled value, since the inspector has no
        // store to fetch the block.
        //
        // Only EAV-shaped keys reconstruct. A history, coverage or blob
        // key decodes to no artifact, and treating that as fatal used to
        // abort the WHOLE entry list — one history record and the segment
        // rendered empty. Such an entry keeps its key components and
        // metadata and simply reports no fact.
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
            if let Ok(artifact) = Artifact::from_key_datum_placeholder(&key, &datum) {
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
    for record in inspect_blob_records(bytes)? {
        if let Some(row) = rows
            .iter_mut()
            .find(|row| row.fields.get("at") == Some(&Ipld::Integer(record.at as i128)))
        {
            row.fields
                .insert("blob".into(), Ipld::String(bytes_hex(&record.blob)));
            row.fields
                .insert("blob-size".into(), Ipld::Integer(record.size as i128));
            row.fields
                .insert("blob-version".into(), Ipld::Integer(record.version as i128));
        }
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
        // text. Report them as the version they encode.
        Some(HISTORY_KEY_TAG) | Some(COVERAGE_KEY_TAG) => {
            return version_fields(&component.bytes);
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
    let mut parts = Vec::new();
    let mut rest = component.bytes.as_slice();
    let mut next = 0usize;
    if tag == Some(VALUE_KEY_TAG)
        && let Some((vtype, tail)) = rest.split_first()
    {
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
        let tail = &rest[EDITION..];
        if !tail.is_empty() {
            parts.push(component_part(&KeyComponent {
                kind: "entity",
                text: String::from_utf8_lossy(tail).into_owned(),
                bytes: tail.to_vec(),
            }));
        }
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
}
