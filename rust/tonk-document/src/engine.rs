//! The document engine: everything that touches automerge.
//!
//! A [`Document`] is the cell's bytes loaded into memory. It holds the
//! changes of EVERY branch at once — the cell is an object store — and
//! every read and edit names the heads it is relative to, which is what
//! a branch's `document/heads` claim supplies. Nothing here knows about
//! dialog, cells or branches; the rest of the crate composes this with
//! them.
//!
//! Positions are UTF-16 code units, the unit of JavaScript strings. In
//! automerge the index unit is a build-time choice, not document state,
//! so every document is created and loaded with
//! [`TextEncoding::Utf16CodeUnit`] to agree with the elements.

use std::collections::BTreeMap;

use automerge::transaction::{CommitOptions, Transactable};
use automerge::{
    ActorId, AutoCommit, AutomergeError, ChangeHash, LoadOptions, ObjId, ObjType, PatchAction,
    ROOT, ReadDoc, ScalarValue, TextEncoding, Value,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Format name of a text document (`xyz.tonk.document/format`).
pub const TEXT_FORMAT: &str = "automerge/text@1";
/// Format name of a table (workbook) document.
pub const TABLE_FORMAT: &str = "automerge/table@1";

const TEXT_KEY: &str = "text";
const SHEETS_KEY: &str = "sheets";
const SHEET_MAPS: [&str; 4] = ["cells", "styles", "widths", "heights"];

/// The shape of a document's root.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Format {
    /// `{ text: Text }` holding markdown.
    Text,
    /// `{ sheets: Map<SheetId, Sheet> }`.
    Table,
}

impl Format {
    /// The `xyz.tonk.document/format` value for this shape.
    pub fn name(self) -> &'static str {
        match self {
            Format::Text => TEXT_FORMAT,
            Format::Table => TABLE_FORMAT,
        }
    }

    /// Parse a format claim. An unknown name — a newer format — is
    /// `None`, and the caller opens the document read-only.
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            TEXT_FORMAT => Some(Format::Text),
            TABLE_FORMAT => Some(Format::Table),
            _ => None,
        }
    }
}

/// Failures of the engine.
#[derive(Debug, Error)]
pub enum DocumentError {
    /// Automerge rejected the bytes or the operation.
    #[error("automerge: {0}")]
    Automerge(#[from] AutomergeError),
    /// A head was not 32 bytes of hex.
    #[error("bad head {0:?}")]
    BadHead(String),
    /// The document does not hold every change the heads name yet.
    #[error("the document is missing changes the heads name: {0:?}")]
    MissingChanges(Vec<String>),
    /// The bytes do not have the root this format requires.
    #[error("the document is not {0}")]
    WrongShape(&'static str),
    /// `find` / `after` matched nothing.
    #[error("no match for {0:?}")]
    NoMatch(String),
    /// `find` / `after` matched more than once.
    #[error("{count} matches for {text:?}; quote a longer passage")]
    AmbiguousMatch {
        /// The quoted passage.
        text: String,
        /// How often it occurs.
        count: usize,
    },
    /// A splice range runs past the end of the text.
    #[error("range {at}+{delete} is outside the text (length {length})")]
    OutOfRange {
        /// Start, UTF-16 units.
        at: usize,
        /// Deleted length, UTF-16 units.
        delete: usize,
        /// Text length, UTF-16 units.
        length: usize,
    },
    /// A path the table shape does not have.
    #[error("bad path {0:?}")]
    BadPath(String),
    /// The edit does not apply to this document's shape.
    #[error("{edit} does not apply to a {format} document")]
    WrongEdit {
        /// The edit kind.
        edit: &'static str,
        /// The document's format.
        format: &'static str,
    },
}

/// Who made a change and when. Recorded in the change's message so the
/// JS elements, which have no author API, can write the same form.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stamp {
    /// The writer's profile DID. Advisory: automerge changes are unsigned.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    /// Unix seconds.
    #[serde(default)]
    pub time: i64,
}

/// One edit, applied on top of given heads.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Edit {
    /// Replace the one occurrence of `find`.
    Replace {
        /// The quoted passage; must occur exactly once.
        find: String,
        /// Its replacement.
        with: String,
    },
    /// Insert `text` right after the one occurrence of `after`.
    Insert {
        /// The quoted passage; must occur exactly once.
        after: String,
        /// What to insert.
        text: String,
    },
    /// Exact range edit, UTF-16 units at the edit's heads.
    Splice {
        /// Start.
        at: usize,
        /// Units to delete.
        delete: usize,
        /// Text to insert.
        text: String,
    },
    /// Make the text equal `text` (diffed into splices).
    SetText {
        /// The whole new text.
        text: String,
    },
    /// Write one table value, e.g. `sheets/<id>/cells/B2`.
    Put {
        /// Slash-separated path.
        path: String,
        /// A string, number, boolean or null.
        value: serde_json::Value,
    },
    /// Remove one table value, or a whole sheet (`sheets/<id>`).
    Remove {
        /// Slash-separated path.
        path: String,
    },
    /// Make the content equal to what it was at `heads`.
    Restore {
        /// The past version.
        heads: Vec<String>,
    },
}

/// One sheet of a workbook at some heads.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Sheet {
    /// The sheet's key under `sheets`.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Fractional order key.
    pub order: String,
    /// Raw cell input by A1 address.
    pub cells: BTreeMap<String, String>,
    /// Style JSON by A1 address.
    pub styles: BTreeMap<String, String>,
    /// Column width px by column letters.
    pub widths: BTreeMap<String, f64>,
    /// Row height px by row number.
    pub heights: BTreeMap<String, f64>,
    /// Addresses whose cell holds concurrent values.
    pub conflicts: Vec<String>,
}

/// A workbook at some heads, sheets in `order`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Table {
    /// The sheets, sorted by `(order, id)`.
    pub sheets: Vec<Sheet>,
}

/// A document's content at some heads.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Content {
    /// Markdown.
    Text(String),
    /// A workbook.
    Table(Table),
}

/// One automerge change, for history.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangeInfo {
    /// The change hash, hex.
    pub change: String,
    /// The author DID the writer recorded, if any.
    pub author: Option<String>,
    /// Unix seconds.
    pub time: i64,
    /// The hashes this change builds on, hex.
    pub parents: Vec<String>,
}

/// One difference between two versions, in tonk's own shape.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "lowercase")]
pub enum DiffOp {
    /// Text inserted at `at` (UTF-16 units, in the evolving text).
    Insert {
        /// Position.
        at: usize,
        /// Inserted text.
        text: String,
    },
    /// `length` units deleted at `at`.
    Delete {
        /// Position.
        at: usize,
        /// Deleted length.
        length: usize,
    },
    /// A table value written.
    Put {
        /// Slash-separated path.
        path: String,
        /// The new value.
        value: serde_json::Value,
    },
    /// A table value removed.
    Remove {
        /// Slash-separated path.
        path: String,
    },
}

/// A cell's bytes, loaded.
pub struct Document {
    doc: AutoCommit,
    format: Format,
}

impl std::fmt::Debug for Document {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Document")
            .field("format", &self.format)
            .finish_non_exhaustive()
    }
}

/// Parse hex heads.
pub fn parse_heads(heads: &[String]) -> Result<Vec<ChangeHash>, DocumentError> {
    heads
        .iter()
        .map(|head| {
            let bytes = hex::decode(head).map_err(|_| DocumentError::BadHead(head.clone()))?;
            ChangeHash::try_from(bytes.as_slice()).map_err(|_| DocumentError::BadHead(head.clone()))
        })
        .collect()
}

/// Format heads as sorted hex, the form the heads claim stores.
pub fn format_heads(heads: &[ChangeHash]) -> Vec<String> {
    let mut out: Vec<String> = heads.iter().map(|h| hex::encode(h.0)).collect();
    out.sort();
    out
}

fn fixed_actor(label: &[u8]) -> ActorId {
    let hash = blake3::hash(label);
    ActorId::from(&hash.as_bytes()[..16])
}

fn fresh(format: Format) -> Result<AutoCommit, DocumentError> {
    // Constant genesis: a fixed actor and time, so two replicas that
    // start a document with no contact share the same root objects.
    // Without it each would create its own `text` object and one side's
    // edits would vanish on merge.
    let label = format!("tonk-document/genesis/{}", format.name());
    let mut doc =
        AutoCommit::new_with_encoding(TextEncoding::Utf16CodeUnit).with_actor(fixed_actor(label.as_bytes()));
    match format {
        Format::Text => {
            doc.put_object(ROOT, TEXT_KEY, ObjType::Text)?;
        }
        Format::Table => {
            doc.put_object(ROOT, SHEETS_KEY, ObjType::Map)?;
        }
    }
    doc.commit_with(CommitOptions::default().with_time(0));
    Ok(doc)
}

impl Document {
    /// A new, empty document of `format`. Deterministic.
    pub fn genesis(format: Format) -> Result<Self, DocumentError> {
        let mut doc = fresh(format)?;
        doc.set_actor(ActorId::random());
        Ok(Self { doc, format })
    }

    /// The heads of [`Document::genesis`]: what a branch is at before it
    /// holds any `document/heads` claim.
    pub fn genesis_heads(format: Format) -> Result<Vec<String>, DocumentError> {
        let mut doc = fresh(format)?;
        Ok(format_heads(&doc.get_heads()))
    }

    /// Start a text document from existing markdown. Deterministic: two
    /// replicas that convert the same markdown produce the same change,
    /// so merging them does not double the text.
    pub fn from_text(markdown: &str) -> Result<Self, DocumentError> {
        let mut doc = fresh(Format::Text)?;
        if !markdown.is_empty() {
            let mut label = b"tonk-document/import/text/".to_vec();
            label.extend_from_slice(markdown.as_bytes());
            doc.set_actor(fixed_actor(&label));
            let text = text_object(&doc, None)?;
            doc.splice_text(&text, 0, 0, markdown)?;
            doc.commit_with(CommitOptions::default().with_time(0));
        }
        doc.set_actor(ActorId::random());
        Ok(Self {
            doc,
            format: Format::Text,
        })
    }

    /// Start a table document from an existing workbook. Deterministic
    /// for equal input: sheets and keys are written in sorted order.
    pub fn from_table(table: &Table) -> Result<Self, DocumentError> {
        let mut doc = fresh(Format::Table)?;
        if !table.sheets.is_empty() {
            let canonical = serde_json::to_vec(&canonical_table(table)).unwrap_or_default();
            let mut label = b"tonk-document/import/table/".to_vec();
            label.extend_from_slice(&canonical);
            doc.set_actor(fixed_actor(&label));
            let sheets = sheets_object(&doc, None)?;
            let mut sorted: Vec<&Sheet> = table.sheets.iter().collect();
            sorted.sort_by(|a, b| a.id.cmp(&b.id));
            for sheet in sorted {
                let object = create_sheet(&mut doc, &sheets, &sheet.id)?;
                doc.put(&object, "name", sheet.name.as_str())?;
                doc.put(&object, "order", sheet.order.as_str())?;
                let cells = child_map(&doc, &object, "cells", None)?;
                for (at, content) in &sheet.cells {
                    doc.put(&cells, at.as_str(), content.as_str())?;
                }
                let styles = child_map(&doc, &object, "styles", None)?;
                for (at, style) in &sheet.styles {
                    doc.put(&styles, at.as_str(), style.as_str())?;
                }
                let widths = child_map(&doc, &object, "widths", None)?;
                for (at, width) in &sheet.widths {
                    doc.put(&widths, at.as_str(), *width)?;
                }
                let heights = child_map(&doc, &object, "heights", None)?;
                for (at, height) in &sheet.heights {
                    doc.put(&heights, at.as_str(), *height)?;
                }
            }
            doc.commit_with(CommitOptions::default().with_time(0));
        }
        doc.set_actor(ActorId::random());
        Ok(Self {
            doc,
            format: Format::Table,
        })
    }

    /// Load a cell's bytes. The shape is read off the root.
    pub fn load(bytes: &[u8]) -> Result<Self, DocumentError> {
        let doc = AutoCommit::load_with_options(
            bytes,
            LoadOptions::new().text_encoding(TextEncoding::Utf16CodeUnit),
        )?;
        let format = if matches!(doc.get(ROOT, TEXT_KEY)?, Some((Value::Object(ObjType::Text), _))) {
            Format::Text
        } else if matches!(doc.get(ROOT, SHEETS_KEY)?, Some((Value::Object(ObjType::Map), _))) {
            Format::Table
        } else {
            return Err(DocumentError::WrongShape("a tonk document"));
        };
        Ok(Self { doc, format })
    }

    /// The document's shape.
    pub fn format(&self) -> Format {
        self.format
    }

    /// Merge another copy of the cell's bytes in. Commutative and
    /// idempotent. Returns whether anything new arrived.
    pub fn merge(&mut self, bytes: &[u8]) -> Result<bool, DocumentError> {
        let before = self.doc.get_heads();
        self.doc.load_incremental(bytes)?;
        Ok(self.doc.get_heads() != before)
    }

    /// The bytes to store in the cell.
    pub fn save(&mut self) -> Vec<u8> {
        self.doc.save()
    }

    /// The heads of the whole store — every branch's changes together.
    /// NOT what a branch shows; compare with a marker to tell whether
    /// the store changed.
    pub fn store_heads(&mut self) -> Vec<String> {
        format_heads(&self.doc.get_heads())
    }

    /// The heads among `heads` whose changes this document lacks.
    pub fn missing(&mut self, heads: &[String]) -> Result<Vec<String>, DocumentError> {
        let parsed = parse_heads(heads)?;
        let mut missing = Vec::new();
        for (head, text) in parsed.iter().zip(heads) {
            if self.doc.get_change_by_hash(head).is_none() {
                missing.push(text.clone());
            }
        }
        Ok(missing)
    }

    fn resolve(&mut self, heads: &[String]) -> Result<Vec<ChangeHash>, DocumentError> {
        let missing = self.missing(heads)?;
        if !missing.is_empty() {
            return Err(DocumentError::MissingChanges(missing));
        }
        parse_heads(heads)
    }

    /// Drop every head that is an ancestor of another.
    pub fn normalize(&mut self, heads: &[String]) -> Result<Vec<String>, DocumentError> {
        let parsed = self.resolve(heads)?;
        if parsed.len() < 2 {
            return Ok(format_heads(&parsed));
        }
        let mut fork = self.doc.fork_at(&parsed)?;
        Ok(format_heads(&fork.get_heads()))
    }

    /// The content at `heads`.
    pub fn content(&mut self, heads: &[String]) -> Result<Content, DocumentError> {
        let at = self.resolve(heads)?;
        match self.format {
            Format::Text => Ok(Content::Text(self.text_at(&at)?)),
            Format::Table => Ok(Content::Table(self.table_at(&at)?)),
        }
    }

    /// The markdown at `heads`.
    pub fn text(&mut self, heads: &[String]) -> Result<String, DocumentError> {
        let at = self.resolve(heads)?;
        self.text_at(&at)
    }

    /// The workbook at `heads`.
    pub fn table(&mut self, heads: &[String]) -> Result<Table, DocumentError> {
        let at = self.resolve(heads)?;
        self.table_at(&at)
    }

    fn text_at(&mut self, at: &[ChangeHash]) -> Result<String, DocumentError> {
        if self.format != Format::Text {
            return Err(DocumentError::WrongShape(TEXT_FORMAT));
        }
        let text = text_object(&self.doc, Some(at))?;
        Ok(self.doc.text_at(&text, at)?)
    }

    fn table_at(&mut self, at: &[ChangeHash]) -> Result<Table, DocumentError> {
        if self.format != Format::Table {
            return Err(DocumentError::WrongShape(TABLE_FORMAT));
        }
        let doc = &self.doc;
        let sheets = sheets_object(doc, Some(at))?;
        let mut out = Vec::new();
        for id in doc.keys_at(&sheets, at) {
            let Some((Value::Object(ObjType::Map), object)) = doc.get_at(&sheets, id.as_str(), at)? else {
                continue;
            };
            let mut sheet = Sheet {
                id: id.clone(),
                name: string_at(doc, &object, "name", at)?.unwrap_or_default(),
                order: string_at(doc, &object, "order", at)?.unwrap_or_default(),
                ..Sheet::default()
            };
            if let Ok(cells) = child_map(doc, &object, "cells", Some(at)) {
                for at_key in doc.keys_at(&cells, at) {
                    if let Some(content) = string_at(doc, &cells, &at_key, at)? {
                        if doc.get_all_at(&cells, at_key.as_str(), at)?.len() > 1 {
                            sheet.conflicts.push(at_key.clone());
                        }
                        sheet.cells.insert(at_key, content);
                    }
                }
            }
            if let Ok(styles) = child_map(doc, &object, "styles", Some(at)) {
                for key in doc.keys_at(&styles, at) {
                    if let Some(style) = string_at(doc, &styles, &key, at)? {
                        sheet.styles.insert(key, style);
                    }
                }
            }
            if let Ok(widths) = child_map(doc, &object, "widths", Some(at)) {
                for key in doc.keys_at(&widths, at) {
                    if let Some(width) = number_at(doc, &widths, &key, at)? {
                        sheet.widths.insert(key, width);
                    }
                }
            }
            if let Ok(heights) = child_map(doc, &object, "heights", Some(at)) {
                for key in doc.keys_at(&heights, at) {
                    if let Some(height) = number_at(doc, &heights, &key, at)? {
                        sheet.heights.insert(key, height);
                    }
                }
            }
            out.push(sheet);
        }
        out.sort_by(|a, b| (a.order.as_str(), a.id.as_str()).cmp(&(b.order.as_str(), b.id.as_str())));
        Ok(Table { sheets: out })
    }

    /// Apply `edit` on top of `heads` and return the new heads for that
    /// line. Other branches' changes in the store are untouched and
    /// unseen. An edit that changes nothing returns `heads` normalized.
    pub fn edit(&mut self, heads: &[String], stamp: &Stamp, edit: &Edit) -> Result<Vec<String>, DocumentError> {
        let base = self.normalize(heads)?;
        let at = parse_heads(&base)?;
        self.doc.isolate(&at);
        let applied = self.apply(&at, edit);
        let outcome = match applied {
            Ok(()) => {
                let message = serde_json::to_string(stamp).unwrap_or_default();
                if self.doc.pending_ops() > 0 {
                    self.doc
                        .commit_with(CommitOptions::default().with_message(message).with_time(stamp.time));
                }
                Ok(format_heads(&self.doc.get_heads()))
            }
            Err(error) => {
                self.doc.rollback();
                Err(error)
            }
        };
        self.doc.integrate();
        outcome
    }

    fn apply(&mut self, at: &[ChangeHash], edit: &Edit) -> Result<(), DocumentError> {
        match (self.format, edit) {
            (Format::Text, Edit::Replace { find, with }) => {
                let (start, length) = self.locate(at, find)?;
                let text = text_object(&self.doc, None)?;
                self.doc.splice_text(&text, start, length as isize, with)?;
            }
            (Format::Text, Edit::Insert { after, text: inserted }) => {
                let (start, length) = self.locate(at, after)?;
                let text = text_object(&self.doc, None)?;
                self.doc.splice_text(&text, start + length, 0, inserted)?;
            }
            (Format::Text, Edit::Splice { at: start, delete, text: inserted }) => {
                let text = text_object(&self.doc, None)?;
                let length = self.doc.length(&text);
                if start + delete > length {
                    return Err(DocumentError::OutOfRange {
                        at: *start,
                        delete: *delete,
                        length,
                    });
                }
                self.doc.splice_text(&text, *start, *delete as isize, inserted)?;
            }
            (Format::Text, Edit::SetText { text: new_text }) => {
                let text = text_object(&self.doc, None)?;
                self.doc.update_text(&text, new_text)?;
            }
            (Format::Table, Edit::Put { path, value }) => self.put(path, value)?,
            (Format::Table, Edit::Remove { path }) => self.remove(path)?,
            (Format::Text, Edit::Restore { heads }) => {
                let past = self.resolve(heads)?;
                // Reading at other heads while isolated is fine: reads
                // take explicit heads.
                let text = text_object(&self.doc, None)?;
                let old = self.doc.text_at(&text, &past)?;
                self.doc.update_text(&text, old)?;
            }
            (Format::Table, Edit::Restore { heads }) => {
                let past = self.resolve(heads)?;
                let target = self.table_at(&past)?;
                let current = self.table_at(at)?;
                for op in table_diff(&current, &target) {
                    match op {
                        DiffOp::Put { path, value } => self.put(&path, &value)?,
                        DiffOp::Remove { path } => self.remove(&path)?,
                        DiffOp::Insert { .. } | DiffOp::Delete { .. } => {}
                    }
                }
            }
            (Format::Text, Edit::Put { .. }) => return Err(wrong_edit("put", Format::Text)),
            (Format::Text, Edit::Remove { .. }) => return Err(wrong_edit("remove", Format::Text)),
            (Format::Table, Edit::Replace { .. }) => return Err(wrong_edit("replace", Format::Table)),
            (Format::Table, Edit::Insert { .. }) => return Err(wrong_edit("insert", Format::Table)),
            (Format::Table, Edit::Splice { .. }) => return Err(wrong_edit("splice", Format::Table)),
            (Format::Table, Edit::SetText { .. }) => return Err(wrong_edit("set-text", Format::Table)),
        }
        Ok(())
    }

    /// Find the one occurrence of `quote`; `(start, length)` in UTF-16.
    fn locate(&mut self, at: &[ChangeHash], quote: &str) -> Result<(usize, usize), DocumentError> {
        let text = self.text_at(at)?;
        if quote.is_empty() {
            return Err(DocumentError::NoMatch(String::new()));
        }
        let mut found = text.match_indices(quote);
        let Some((byte, _)) = found.next() else {
            return Err(DocumentError::NoMatch(quote.to_string()));
        };
        let more = found.count();
        if more > 0 {
            return Err(DocumentError::AmbiguousMatch {
                text: quote.to_string(),
                count: more + 1,
            });
        }
        let start = text[..byte].encode_utf16().count();
        Ok((start, quote.encode_utf16().count()))
    }

    fn put(&mut self, path: &str, value: &serde_json::Value) -> Result<(), DocumentError> {
        let (sheet, rest) = table_path(path)?;
        let sheets = sheets_object(&self.doc, None)?;
        let object = match self.doc.get(&sheets, sheet)? {
            Some((Value::Object(ObjType::Map), object)) => object,
            _ => create_sheet(&mut self.doc, &sheets, sheet)?,
        };
        let scalar = scalar(value).ok_or_else(|| DocumentError::BadPath(path.to_string()))?;
        match rest.as_slice() {
            [field @ ("name" | "order")] => {
                self.doc.put(&object, *field, scalar)?;
            }
            [map, key] if SHEET_MAPS.contains(map) => {
                let target = child_map(&self.doc, &object, map, None)?;
                self.doc.put(&target, *key, scalar)?;
            }
            _ => return Err(DocumentError::BadPath(path.to_string())),
        }
        Ok(())
    }

    fn remove(&mut self, path: &str) -> Result<(), DocumentError> {
        let (sheet, rest) = table_path(path)?;
        let sheets = sheets_object(&self.doc, None)?;
        let Some((Value::Object(ObjType::Map), object)) = self.doc.get(&sheets, sheet)? else {
            return Ok(());
        };
        match rest.as_slice() {
            [] => {
                self.doc.delete(&sheets, sheet)?;
            }
            [map, key] if SHEET_MAPS.contains(map) => {
                let target = child_map(&self.doc, &object, map, None)?;
                if self.doc.get(&target, *key)?.is_some() {
                    self.doc.delete(&target, *key)?;
                }
            }
            _ => return Err(DocumentError::BadPath(path.to_string())),
        }
        Ok(())
    }

    /// Every change not reachable from `since`, oldest first.
    pub fn changes(&mut self, since: &[String]) -> Result<Vec<ChangeInfo>, DocumentError> {
        let since = self.resolve(since)?;
        Ok(self
            .doc
            .get_changes_meta(&since)
            .into_iter()
            .map(|meta| {
                let stamp: Option<Stamp> = meta
                    .message
                    .as_deref()
                    .and_then(|message| serde_json::from_str(message).ok());
                ChangeInfo {
                    change: hex::encode(meta.hash.0),
                    author: stamp.and_then(|stamp| stamp.author),
                    time: meta.timestamp,
                    parents: format_heads(&meta.deps),
                }
            })
            .collect())
    }

    /// What turns the content at `from` into the content at `to`.
    pub fn diff(&mut self, from: &[String], to: &[String]) -> Result<Vec<DiffOp>, DocumentError> {
        let from = self.resolve(from)?;
        let to = self.resolve(to)?;
        match self.format {
            Format::Table => {
                let before = self.table_at(&from)?;
                let after = self.table_at(&to)?;
                Ok(table_diff(&before, &after))
            }
            Format::Text => {
                let mut out = Vec::new();
                for patch in self.doc.diff(&from, &to) {
                    match patch.action {
                        PatchAction::SpliceText { index, value, .. } => out.push(DiffOp::Insert {
                            at: index,
                            text: value.make_string(),
                        }),
                        PatchAction::DeleteSeq { index, length } => out.push(DiffOp::Delete { at: index, length }),
                        _ => {}
                    }
                }
                Ok(out)
            }
        }
    }
}

fn wrong_edit(edit: &'static str, format: Format) -> DocumentError {
    DocumentError::WrongEdit {
        edit,
        format: format.name(),
    }
}

fn text_object(doc: &AutoCommit, at: Option<&[ChangeHash]>) -> Result<ObjId, DocumentError> {
    let found = match at {
        Some(at) => doc.get_at(ROOT, TEXT_KEY, at)?,
        None => doc.get(ROOT, TEXT_KEY)?,
    };
    match found {
        Some((Value::Object(ObjType::Text), id)) => Ok(id),
        _ => Err(DocumentError::WrongShape(TEXT_FORMAT)),
    }
}

fn sheets_object(doc: &AutoCommit, at: Option<&[ChangeHash]>) -> Result<ObjId, DocumentError> {
    let found = match at {
        Some(at) => doc.get_at(ROOT, SHEETS_KEY, at)?,
        None => doc.get(ROOT, SHEETS_KEY)?,
    };
    match found {
        Some((Value::Object(ObjType::Map), id)) => Ok(id),
        _ => Err(DocumentError::WrongShape(TABLE_FORMAT)),
    }
}

fn child_map(
    doc: &AutoCommit,
    parent: &ObjId,
    key: &str,
    at: Option<&[ChangeHash]>,
) -> Result<ObjId, DocumentError> {
    let found = match at {
        Some(at) => doc.get_at(parent, key, at)?,
        None => doc.get(parent, key)?,
    };
    match found {
        Some((Value::Object(ObjType::Map), id)) => Ok(id),
        _ => Err(DocumentError::WrongShape(TABLE_FORMAT)),
    }
}

/// A sheet and its four child maps, in one change with the write that
/// needed it.
fn create_sheet(doc: &mut AutoCommit, sheets: &ObjId, id: &str) -> Result<ObjId, DocumentError> {
    let object = doc.put_object(sheets, id, ObjType::Map)?;
    doc.put(&object, "name", "")?;
    doc.put(&object, "order", "")?;
    for map in SHEET_MAPS {
        doc.put_object(&object, map, ObjType::Map)?;
    }
    Ok(object)
}

fn string_at(doc: &AutoCommit, object: &ObjId, key: &str, at: &[ChangeHash]) -> Result<Option<String>, DocumentError> {
    Ok(match doc.get_at(object, key, at)? {
        Some((Value::Scalar(scalar), _)) => match scalar.as_ref() {
            ScalarValue::Str(text) => Some(text.to_string()),
            other => Some(other.to_string()),
        },
        _ => None,
    })
}

fn number_at(doc: &AutoCommit, object: &ObjId, key: &str, at: &[ChangeHash]) -> Result<Option<f64>, DocumentError> {
    Ok(match doc.get_at(object, key, at)? {
        Some((Value::Scalar(scalar), _)) => match scalar.as_ref() {
            ScalarValue::F64(n) => Some(*n),
            ScalarValue::Int(n) => Some(*n as f64),
            ScalarValue::Uint(n) => Some(*n as f64),
            ScalarValue::Str(text) => text.parse().ok(),
            _ => None,
        },
        _ => None,
    })
}

fn scalar(value: &serde_json::Value) -> Option<ScalarValue> {
    match value {
        serde_json::Value::String(text) => Some(ScalarValue::Str(text.as_str().into())),
        serde_json::Value::Bool(flag) => Some(ScalarValue::Boolean(*flag)),
        serde_json::Value::Null => Some(ScalarValue::Null),
        serde_json::Value::Number(number) => number.as_f64().map(ScalarValue::F64),
        _ => None,
    }
}

/// Split `sheets/<id>/...` into the sheet id and the rest.
fn table_path(path: &str) -> Result<(&str, Vec<&str>), DocumentError> {
    let mut parts = path.split('/');
    match (parts.next(), parts.next()) {
        (Some(SHEETS_KEY), Some(sheet)) if !sheet.is_empty() => Ok((sheet, parts.collect())),
        _ => Err(DocumentError::BadPath(path.to_string())),
    }
}

fn canonical_table(table: &Table) -> Table {
    let mut sheets = table.sheets.clone();
    sheets.sort_by(|a, b| a.id.cmp(&b.id));
    for sheet in &mut sheets {
        sheet.conflicts.clear();
    }
    Table { sheets }
}

fn table_diff(before: &Table, after: &Table) -> Vec<DiffOp> {
    let mut out = Vec::new();
    let old: BTreeMap<&str, &Sheet> = before.sheets.iter().map(|s| (s.id.as_str(), s)).collect();
    let new: BTreeMap<&str, &Sheet> = after.sheets.iter().map(|s| (s.id.as_str(), s)).collect();
    let empty = Sheet::default();
    for (id, sheet) in &new {
        let was = old.get(id).copied().unwrap_or(&empty);
        if was.name != sheet.name || !old.contains_key(id) {
            out.push(DiffOp::Put {
                path: format!("sheets/{id}/name"),
                value: sheet.name.clone().into(),
            });
        }
        if was.order != sheet.order || !old.contains_key(id) {
            out.push(DiffOp::Put {
                path: format!("sheets/{id}/order"),
                value: sheet.order.clone().into(),
            });
        }
        diff_strings(&mut out, id, "cells", &was.cells, &sheet.cells);
        diff_strings(&mut out, id, "styles", &was.styles, &sheet.styles);
        diff_numbers(&mut out, id, "widths", &was.widths, &sheet.widths);
        diff_numbers(&mut out, id, "heights", &was.heights, &sheet.heights);
    }
    for id in old.keys() {
        if !new.contains_key(id) {
            out.push(DiffOp::Remove {
                path: format!("sheets/{id}"),
            });
        }
    }
    out
}

fn diff_strings(
    out: &mut Vec<DiffOp>,
    sheet: &str,
    map: &str,
    before: &BTreeMap<String, String>,
    after: &BTreeMap<String, String>,
) {
    for (key, value) in after {
        if before.get(key) != Some(value) {
            out.push(DiffOp::Put {
                path: format!("sheets/{sheet}/{map}/{key}"),
                value: value.clone().into(),
            });
        }
    }
    for key in before.keys() {
        if !after.contains_key(key) {
            out.push(DiffOp::Remove {
                path: format!("sheets/{sheet}/{map}/{key}"),
            });
        }
    }
}

fn diff_numbers(
    out: &mut Vec<DiffOp>,
    sheet: &str,
    map: &str,
    before: &BTreeMap<String, f64>,
    after: &BTreeMap<String, f64>,
) {
    for (key, value) in after {
        if before.get(key) != Some(value) {
            out.push(DiffOp::Put {
                path: format!("sheets/{sheet}/{map}/{key}"),
                value: serde_json::json!(value),
            });
        }
    }
    for key in before.keys() {
        if !after.contains_key(key) {
            out.push(DiffOp::Remove {
                path: format!("sheets/{sheet}/{map}/{key}"),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_browser);

    fn stamp() -> Stamp {
        Stamp {
            author: Some("did:key:zTest".into()),
            time: 1_700_000_000,
        }
    }

    fn text_edit(text: &str) -> Edit {
        Edit::SetText { text: text.into() }
    }

    #[dialog_common::test]
    fn it_starts_every_replica_from_the_same_genesis() {
        let mut a = Document::genesis(Format::Text).unwrap();
        let mut b = Document::genesis(Format::Text).unwrap();
        assert_eq!(a.store_heads(), b.store_heads());
        assert_eq!(a.store_heads(), Document::genesis_heads(Format::Text).unwrap());

        // Two replicas edit a new document with no contact, then merge.
        let genesis = a.store_heads();
        let ha = a.edit(&genesis, &stamp(), &text_edit("hello")).unwrap();
        let hb = b.edit(&genesis, &stamp(), &text_edit("world")).unwrap();
        let bytes = b.save();
        a.merge(&bytes).unwrap();
        let mut both = ha.clone();
        both.extend(hb);
        let merged = a.text(&both).unwrap();
        assert!(merged.contains("hello") && merged.contains("world"), "{merged:?}");
    }

    #[dialog_common::test]
    fn it_converts_the_same_markdown_to_the_same_change() {
        let mut a = Document::from_text("# Title\n\nbody").unwrap();
        let mut b = Document::from_text("# Title\n\nbody").unwrap();
        assert_eq!(a.store_heads(), b.store_heads());
        let heads = a.store_heads();
        let bytes = b.save();
        assert!(!a.merge(&bytes).unwrap(), "merging an identical import adds nothing");
        assert_eq!(a.text(&heads).unwrap(), "# Title\n\nbody", "the text appears once");
    }

    #[dialog_common::test]
    fn it_isolates_branches_inside_one_store() {
        let mut doc = Document::from_text("base").unwrap();
        let base = doc.store_heads();
        let main = doc.edit(&base, &stamp(), &text_edit("base main")).unwrap();
        let feature = doc.edit(&base, &stamp(), &text_edit("base feature")).unwrap();

        assert_eq!(doc.text(&main).unwrap(), "base main");
        assert_eq!(doc.text(&feature).unwrap(), "base feature");
        assert_eq!(doc.text(&base).unwrap(), "base", "an old revision shows the old text");

        // A branch merge is the union of both heads.
        let mut union = main.clone();
        union.extend(feature.clone());
        let merged = doc.text(&union).unwrap();
        assert!(merged.contains("main") && merged.contains("feature"), "{merged:?}");

        // The next edit on the merged line collapses the heads to one.
        let next = doc
            .edit(&union, &stamp(), &Edit::Insert { after: "base".into(), text: "!".into() })
            .unwrap();
        assert_eq!(next.len(), 1);
    }

    #[dialog_common::test]
    fn it_survives_a_save_and_load_with_every_branch() {
        let mut doc = Document::from_text("base").unwrap();
        let base = doc.store_heads();
        let main = doc.edit(&base, &stamp(), &text_edit("main")).unwrap();
        let feature = doc.edit(&base, &stamp(), &text_edit("feature")).unwrap();
        let mut loaded = Document::load(&doc.save()).unwrap();
        assert_eq!(loaded.format(), Format::Text);
        assert_eq!(loaded.text(&main).unwrap(), "main");
        assert_eq!(loaded.text(&feature).unwrap(), "feature");
    }

    #[dialog_common::test]
    fn it_replaces_one_quoted_passage_and_refuses_the_rest() {
        let mut doc = Document::from_text("one two two").unwrap();
        let heads = doc.store_heads();
        let after = doc
            .edit(&heads, &stamp(), &Edit::Replace { find: "one".into(), with: "1".into() })
            .unwrap();
        assert_eq!(doc.text(&after).unwrap(), "1 two two");

        let none = doc.edit(&after, &stamp(), &Edit::Replace { find: "nine".into(), with: "9".into() });
        assert!(matches!(none, Err(DocumentError::NoMatch(_))));
        let many = doc.edit(&after, &stamp(), &Edit::Replace { find: "two".into(), with: "2".into() });
        assert!(matches!(many, Err(DocumentError::AmbiguousMatch { count: 2, .. })));
        assert_eq!(doc.text(&after).unwrap(), "1 two two", "a refused edit changes nothing");
    }

    #[dialog_common::test]
    fn it_lands_a_splice_computed_at_old_heads_in_the_intended_place() {
        let mut doc = Document::from_text("hello world").unwrap();
        let old = doc.store_heads();
        // Someone else prepends text after `old` was read.
        let newer = doc
            .edit(&old, &stamp(), &Edit::Splice { at: 0, delete: 0, text: ">>> ".into() })
            .unwrap();
        // An agent computed "replace `world`" as index 6 against `old`.
        let agent = doc
            .edit(&old, &stamp(), &Edit::Splice { at: 6, delete: 5, text: "there".into() })
            .unwrap();
        let mut union = newer;
        union.extend(agent);
        assert_eq!(doc.text(&union).unwrap(), ">>> hello there");
    }

    #[dialog_common::test]
    fn it_counts_positions_in_utf16_units() {
        let mut doc = Document::from_text("a😀b").unwrap();
        let heads = doc.store_heads();
        // The emoji is two UTF-16 units, so `b` sits at 3.
        let after = doc
            .edit(&heads, &stamp(), &Edit::Splice { at: 3, delete: 1, text: "c".into() })
            .unwrap();
        assert_eq!(doc.text(&after).unwrap(), "a😀c");
        let replaced = doc
            .edit(&after, &stamp(), &Edit::Replace { find: "c".into(), with: "d".into() })
            .unwrap();
        assert_eq!(doc.text(&replaced).unwrap(), "a😀d");
    }

    #[dialog_common::test]
    fn it_reports_heads_whose_changes_have_not_arrived() {
        let mut ahead = Document::from_text("base").unwrap();
        let base = ahead.store_heads();
        let mut behind = Document::load(&ahead.save()).unwrap();
        let newer = ahead.edit(&base, &stamp(), &text_edit("newer")).unwrap();
        assert_eq!(behind.missing(&newer).unwrap(), newer);
        assert!(matches!(behind.text(&newer), Err(DocumentError::MissingChanges(_))));
        behind.merge(&ahead.save()).unwrap();
        assert!(behind.missing(&newer).unwrap().is_empty());
        assert_eq!(behind.text(&newer).unwrap(), "newer");
    }

    #[dialog_common::test]
    fn it_lists_changes_with_author_and_restores_a_version() {
        let mut doc = Document::from_text("v1").unwrap();
        let v1 = doc.store_heads();
        let v2 = doc.edit(&v1, &stamp(), &text_edit("v2")).unwrap();
        let changes = doc.changes(&v1).unwrap();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].author.as_deref(), Some("did:key:zTest"));
        assert_eq!(changes[0].time, 1_700_000_000);
        assert_eq!(changes[0].parents, v1);

        let diff = doc.diff(&v1, &v2).unwrap();
        assert!(!diff.is_empty());

        let restored = doc.edit(&v2, &stamp(), &Edit::Restore { heads: v1.clone() }).unwrap();
        assert_eq!(doc.text(&restored).unwrap(), "v1");
        assert_ne!(restored, v1, "a restore is a new change");
        assert_eq!(doc.text(&v2).unwrap(), "v2", "the old versions still read");
    }

    #[dialog_common::test]
    fn it_keeps_one_cell_and_flags_the_conflict_when_two_fill_it() {
        let mut a = Document::genesis(Format::Table).unwrap();
        let genesis = a.store_heads();
        let with_sheet = a
            .edit(&genesis, &stamp(), &Edit::Put { path: "sheets/s1/name".into(), value: "Sheet1".into() })
            .unwrap();
        let mut b = Document::load(&a.save()).unwrap();

        let ha = a
            .edit(&with_sheet, &stamp(), &Edit::Put { path: "sheets/s1/cells/B2".into(), value: "from a".into() })
            .unwrap();
        let hb = b
            .edit(&with_sheet, &stamp(), &Edit::Put { path: "sheets/s1/cells/B2".into(), value: "from b".into() })
            .unwrap();
        a.merge(&b.save()).unwrap();
        b.merge(&a.save()).unwrap();

        let mut union = ha;
        union.extend(hb);
        let ta = a.table(&union).unwrap();
        let tb = b.table(&union).unwrap();
        assert_eq!(ta, tb, "both replicas converge on one value");
        assert_eq!(ta.sheets.len(), 1);
        assert_eq!(ta.sheets[0].cells.len(), 1);
        assert_eq!(ta.sheets[0].conflicts, vec!["B2".to_string()]);
    }

    #[dialog_common::test]
    fn it_diffs_and_restores_a_workbook() {
        let mut doc = Document::genesis(Format::Table).unwrap();
        let g = doc.store_heads();
        let v1 = doc
            .edit(&g, &stamp(), &Edit::Put { path: "sheets/s1/cells/A1".into(), value: "1".into() })
            .unwrap();
        let v2 = doc
            .edit(&v1, &stamp(), &Edit::Put { path: "sheets/s1/cells/A1".into(), value: "2".into() })
            .unwrap();
        let v3 = doc
            .edit(&v2, &stamp(), &Edit::Remove { path: "sheets/s1/cells/A1".into() })
            .unwrap();
        assert_eq!(
            doc.diff(&v1, &v2).unwrap(),
            vec![DiffOp::Put { path: "sheets/s1/cells/A1".into(), value: "2".into() }]
        );
        assert_eq!(
            doc.diff(&v2, &v3).unwrap(),
            vec![DiffOp::Remove { path: "sheets/s1/cells/A1".into() }]
        );
        let back = doc.edit(&v3, &stamp(), &Edit::Restore { heads: v1.clone() }).unwrap();
        assert_eq!(doc.table(&back).unwrap().sheets[0].cells["A1"], "1");
    }

    #[dialog_common::test]
    fn it_converts_the_same_workbook_to_the_same_change() {
        let mut sheet = Sheet {
            id: "s1".into(),
            name: "Sheet1".into(),
            order: "m".into(),
            ..Sheet::default()
        };
        sheet.cells.insert("A1".into(), "Item".into());
        sheet.cells.insert("D2".into(), "=B2*C2".into());
        let table = Table { sheets: vec![sheet] };
        let mut a = Document::from_table(&table).unwrap();
        let mut b = Document::from_table(&table).unwrap();
        assert_eq!(a.store_heads(), b.store_heads());
        let heads = a.store_heads();
        assert_eq!(a.table(&heads).unwrap(), table);
    }

    #[dialog_common::test]
    fn it_refuses_an_edit_of_the_wrong_shape() {
        let mut doc = Document::genesis(Format::Text).unwrap();
        let heads = doc.store_heads();
        let result = doc.edit(&heads, &stamp(), &Edit::Put { path: "sheets/s/cells/A1".into(), value: "x".into() });
        assert!(matches!(result, Err(DocumentError::WrongEdit { .. })));
    }
}
