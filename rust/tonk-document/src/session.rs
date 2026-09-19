//! A document on a branch: open it, read it, edit it.
//!
//! The cell holds every branch's changes; the branch holds the pointer,
//! the many-valued `document/heads` claim. Everything here reads and
//! writes RELATIVE TO THAT CLAIM, which is what isolates one branch's
//! document from another's inside the same cell.
//!
//! Writes go bytes first, pointer second. The cell only grows, so bytes
//! that no pointer names yet are harmless; a pointer naming bytes that
//! are not stored would not be. Since dialog #488 a commit stages and
//! `publish` moves the branch head with one compare-and-swap, so a lost
//! publish re-runs only the pointer transaction.
//!
//! Three rules for the heads claim follow from dialog's observed-remove
//! merge, where a retraction covers only the claim versions its author
//! saw:
//!
//! 1. Never re-assert a head the branch already claims — the new claim
//!    version would outlive other replicas' retractions.
//! 2. Retract only heads read from the local tree — a retract of an
//!    absent value is a silent no-op.
//! 3. Tolerate extra heads — an ancestor head is harmless to automerge,
//!    and the next write drops it.

use std::collections::BTreeSet;

use dialog_artifacts::{ArtifactSelector, Attribute, Entity, Statement, Update, Value};
use dialog_capability::Subject;
use dialog_reactor::{CommitProvider, PushProvider, SelectProvider};
use dialog_repository::{Branch, RepositoryMemoryExt as _, Upstream};
use futures_util::StreamExt as _;
use thiserror::Error;

use crate::cell::{self, CellError, LocalCell, RETRY_LIMIT, RemoteCell, Transport as _};
use crate::engine::{Content, Document, DocumentError, Edit, Format, Sheet, Stamp, Table};
use crate::sync::{Marker, Outcome, sync_pass};

/// `xyz.tonk.document/format`
pub const FORMAT: &str = "xyz.tonk.document/format";
/// `xyz.tonk.document/heads`
pub const HEADS: &str = "xyz.tonk.document/heads";
/// `xyz.tonk.document/text` — mirror, overlay-only.
pub const TEXT: &str = "xyz.tonk.document/text";
/// `xyz.tonk.document/failure` — overlay-only, on a refused command.
pub const FAILURE: &str = "xyz.tonk.document/failure";

const LEGACY_PROSE_CONTENT: &str = "io.gozala.prose/content";
const SHEET_TABLE: &str = "xyz.tonk.table.sheet/table";
const SHEET_NAME: &str = "xyz.tonk.table.sheet/name";
const SHEET_ORDER: &str = "xyz.tonk.table.sheet/order";
const CELL_SHEET: &str = "xyz.tonk.table.cell/sheet";
const CELL_AT: &str = "xyz.tonk.table.cell/at";
const CELL_CONTENT: &str = "xyz.tonk.table.cell/content";
const CELL_STYLE: &str = "xyz.tonk.table.cell/style";
const COLUMN_SHEET: &str = "xyz.tonk.table.column/sheet";
const COLUMN_AT: &str = "xyz.tonk.table.column/at";
const COLUMN_WIDTH: &str = "xyz.tonk.table.column/width";
const ROW_SHEET: &str = "xyz.tonk.table.row/sheet";
const ROW_AT: &str = "xyz.tonk.table.row/at";
const ROW_HEIGHT: &str = "xyz.tonk.table.row/height";

/// The env a document operation needs: queries, commits and the memory
/// effects (which [`CommitProvider`] already carries).
pub trait DocumentEnv: SelectProvider + CommitProvider {}
impl<T: SelectProvider + CommitProvider> DocumentEnv for T {}

/// Failures of a document operation.
#[derive(Debug, Error)]
pub enum SessionError {
    /// The entity is not a document and the caller named no format to
    /// create one with.
    #[error("{0} is not a document")]
    NotADocument(String),
    /// The format claim names a format this build does not know: a newer
    /// app wrote it. The document must not be edited.
    #[error("{0} has the unknown format {1:?}; update tonk to edit it")]
    UnknownFormat(String, String),
    /// The cell failed.
    #[error(transparent)]
    Cell(#[from] CellError),
    /// The engine refused.
    #[error(transparent)]
    Document(#[from] DocumentError),
    /// A query or commit on the branch failed.
    #[error("branch: {0}")]
    Branch(String),
    /// The branch head kept moving under the pointer write.
    #[error("the branch stayed contended after {RETRY_LIMIT} tries")]
    Contended,
}

/// A document's state on one branch.
#[derive(Clone, Debug, PartialEq)]
pub struct Snapshot {
    /// The document's shape.
    pub format: Format,
    /// The heads this branch is at.
    pub heads: Vec<String>,
    /// The content at those heads.
    pub content: Content,
}

/// The result of a write.
#[derive(Clone, Debug, PartialEq)]
pub struct Written {
    /// The heads of the writer's own line after the edit: what the
    /// content it sent now corresponds to.
    pub local: Vec<String>,
    /// The branch's state after the write, which may include changes the
    /// writer had not seen.
    pub snapshot: Snapshot,
}

/// One `(entity, attribute, value)` statement.
#[derive(Clone)]
struct Fact {
    the: Attribute,
    of: Entity,
    is: Value,
    unique: bool,
}

impl Fact {
    fn many(the: &str, of: &Entity, is: impl Into<Value>) -> Result<Self, SessionError> {
        Ok(Self {
            the: attribute(the)?,
            of: of.clone(),
            is: is.into(),
            unique: false,
        })
    }

    fn one(the: &str, of: &Entity, is: impl Into<Value>) -> Result<Self, SessionError> {
        Ok(Self {
            unique: true,
            ..Self::many(the, of, is)?
        })
    }
}

impl Statement for Fact {
    fn assert(self, update: &mut impl Update) {
        if self.unique {
            update.associate_unique(self.the, self.of, self.is);
        } else {
            update.associate(self.the, self.of, self.is);
        }
    }

    fn retract(self, update: &mut impl Update) {
        update.dissociate(self.the, self.of, self.is);
    }
}

fn attribute(name: &str) -> Result<Attribute, SessionError> {
    name.parse()
        .map_err(|_| SessionError::Branch(format!("bad attribute {name}")))
}

fn branch_error(error: impl std::fmt::Display) -> SessionError {
    SessionError::Branch(error.to_string())
}

/// Every `(entity, value)` of `the`, optionally pinned to one entity or
/// one value.
async fn select<Env: SelectProvider>(
    branch: &Branch,
    the: &str,
    of: Option<&Entity>,
    is: Option<Value>,
    env: &Env,
) -> Result<Vec<(Entity, Value)>, SessionError> {
    let mut selector = ArtifactSelector::new().the(attribute(the)?);
    if let Some(of) = of {
        selector = selector.of(of.clone());
    }
    if let Some(is) = is {
        selector = selector.is(is);
    }
    let stream = branch
        .claims()
        .select(selector)
        .perform(env)
        .await
        .map_err(branch_error)?;
    futures_util::pin_mut!(stream);
    let mut out = Vec::new();
    while let Some(next) = stream.next().await {
        let artifact = next.map_err(branch_error)?;
        let artifact = artifact.to_owned().map_err(branch_error)?;
        out.push((artifact.of, artifact.is));
    }
    Ok(out)
}

async fn texts<Env: SelectProvider>(
    branch: &Branch,
    the: &str,
    of: &Entity,
    env: &Env,
) -> Result<Vec<String>, SessionError> {
    Ok(select(branch, the, Some(of), None, env)
        .await?
        .into_iter()
        .filter_map(|(_, value)| String::try_from(value).ok())
        .collect())
}

async fn text<Env: SelectProvider>(
    branch: &Branch,
    the: &str,
    of: &Entity,
    env: &Env,
) -> Result<Option<String>, SessionError> {
    Ok(texts(branch, the, of, env).await?.into_iter().next())
}

/// The heads the branch claims, sorted. Empty when it claims none.
pub async fn claimed_heads<Env: SelectProvider>(
    branch: &Branch,
    entity: &Entity,
    env: &Env,
) -> Result<Vec<String>, SessionError> {
    let mut heads = texts(branch, HEADS, entity, env).await?;
    heads.sort();
    heads.dedup();
    Ok(heads)
}

/// The format the branch claims for `entity`, if any.
pub async fn claimed_format<Env: SelectProvider>(
    branch: &Branch,
    entity: &Entity,
    env: &Env,
) -> Result<Option<Format>, SessionError> {
    match text(branch, FORMAT, entity, env).await? {
        None => Ok(None),
        Some(name) => Format::parse(&name)
            .map(Some)
            .ok_or_else(|| SessionError::UnknownFormat(entity.to_string(), name)),
    }
}

/// Every document entity the branch declares, with its format name.
/// Cells cannot be enumerated, so this claim is how documents are found.
pub async fn documents<Env: SelectProvider>(
    branch: &Branch,
    env: &Env,
) -> Result<Vec<(Entity, String)>, SessionError> {
    Ok(select(branch, FORMAT, None, None, env)
        .await?
        .into_iter()
        .filter_map(|(entity, value)| String::try_from(value).ok().map(|name| (entity, name)))
        .collect())
}

/// The markdown inside a `<tonk-prose>` content envelope, or the text
/// itself when it is bare markdown.
fn prose_body(content: &str) -> &str {
    const HEADER: &str = "tonk-prose-version:";
    let is_envelope = content
        .get(..HEADER.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(HEADER));
    if !is_envelope {
        return content;
    }
    let crlf = content.find("\r\n\r\n").map(|at| (at, at + 4));
    let lf = content.find("\n\n").map(|at| (at, at + 2));
    match (crlf, lf) {
        (Some(a), Some(b)) => &content[if a.0 <= b.0 { a.1 } else { b.1 }..],
        (Some(a), None) => &content[a.1..],
        (None, Some(b)) => &content[b.1..],
        (None, None) => content,
    }
}

/// What a conversion found: the document, and the old claims to retract.
struct Converted {
    document: Document,
    retract: Vec<Fact>,
}

async fn convert_prose<Env: SelectProvider>(
    branch: &Branch,
    entity: &Entity,
    env: &Env,
) -> Result<Converted, SessionError> {
    let bodies = texts(branch, LEGACY_PROSE_CONTENT, entity, env).await?;
    let document = Document::from_text(bodies.first().map_or("", |body| prose_body(body)))?;
    let mut retract = Vec::new();
    for body in bodies {
        retract.push(Fact::many(LEGACY_PROSE_CONTENT, entity, body)?);
    }
    Ok(Converted { document, retract })
}

/// The id a claims-era sheet entity gets inside the document. Entity
/// URIs contain `/`, which the table paths split on.
fn sheet_id(sheet: &Entity) -> String {
    cell::cell_id(sheet)
}

async fn convert_table<Env: SelectProvider>(
    branch: &Branch,
    entity: &Entity,
    env: &Env,
) -> Result<Converted, SessionError> {
    let mut retract = Vec::new();
    let mut table = Table::default();
    let mut sheets = select(
        branch,
        SHEET_TABLE,
        None,
        Some(Value::Entity(entity.clone())),
        env,
    )
    .await?;
    sheets.sort_by_key(|(sheet, _)| sheet.to_string());
    for (sheet_entity, _) in sheets {
        let mut sheet = Sheet {
            id: sheet_id(&sheet_entity),
            ..Sheet::default()
        };
        retract.push(Fact::many(
            SHEET_TABLE,
            &sheet_entity,
            Value::Entity(entity.clone()),
        )?);
        if let Some(name) = text(branch, SHEET_NAME, &sheet_entity, env).await? {
            retract.push(Fact::many(SHEET_NAME, &sheet_entity, name.clone())?);
            sheet.name = name;
        }
        if let Some(order) = text(branch, SHEET_ORDER, &sheet_entity, env).await? {
            retract.push(Fact::many(SHEET_ORDER, &sheet_entity, order.clone())?);
            sheet.order = order;
        }

        // Cells. Two claims-era entities can hold one address (the
        // defect document mode removes); the smallest entity id wins so
        // every replica converts to the same document. Dialog does not
        // elect between them: they are two entities, not two values.
        let mut cells = select(
            branch,
            CELL_SHEET,
            None,
            Some(Value::Entity(sheet_entity.clone())),
            env,
        )
        .await?;
        cells.sort_by_key(|(cell, _)| cell.to_string());
        for (cell_entity, _) in cells {
            retract.push(Fact::many(
                CELL_SHEET,
                &cell_entity,
                Value::Entity(sheet_entity.clone()),
            )?);
            let at = text(branch, CELL_AT, &cell_entity, env).await?;
            let content = text(branch, CELL_CONTENT, &cell_entity, env).await?;
            let style = text(branch, CELL_STYLE, &cell_entity, env).await?;
            if let Some(at) = &at {
                retract.push(Fact::many(CELL_AT, &cell_entity, at.clone())?);
            }
            if let Some(content) = &content {
                retract.push(Fact::many(CELL_CONTENT, &cell_entity, content.clone())?);
            }
            if let Some(style) = &style {
                retract.push(Fact::many(CELL_STYLE, &cell_entity, style.clone())?);
            }
            let (Some(at), Some(content)) = (at, content) else {
                continue;
            };
            if sheet.cells.contains_key(&at) {
                continue;
            }
            if let Some(style) = style.filter(|style| !style.is_empty()) {
                sheet.styles.insert(at.clone(), style);
            }
            sheet.cells.insert(at, content);
        }

        for (parent, key, value, target) in [
            (COLUMN_SHEET, COLUMN_AT, COLUMN_WIDTH, true),
            (ROW_SHEET, ROW_AT, ROW_HEIGHT, false),
        ] {
            let mut lines = select(
                branch,
                parent,
                None,
                Some(Value::Entity(sheet_entity.clone())),
                env,
            )
            .await?;
            lines.sort_by_key(|(line, _)| line.to_string());
            for (line, _) in lines {
                retract.push(Fact::many(
                    parent,
                    &line,
                    Value::Entity(sheet_entity.clone()),
                )?);
                let at = text(branch, key, &line, env).await?;
                let size = text(branch, value, &line, env).await?;
                if let Some(at) = &at {
                    retract.push(Fact::many(key, &line, at.clone())?);
                }
                if let Some(size) = &size {
                    retract.push(Fact::many(value, &line, size.clone())?);
                }
                let (Some(at), Some(size)) = (at, size.and_then(|size| size.parse::<f64>().ok()))
                else {
                    continue;
                };
                let map = if target {
                    &mut sheet.widths
                } else {
                    &mut sheet.heights
                };
                map.entry(at).or_insert(size);
            }
        }
        table.sheets.push(sheet);
    }
    Ok(Converted {
        document: Document::from_table(&table)?,
        retract,
    })
}

/// Open `entity`'s document on `branch`, creating or converting it on
/// first use. `create` is the format to create with when the entity is
/// not a document yet; with `None` a missing document is an error.
///
/// Conversion lives here, not in the elements: a document-mode view no
/// longer binds the old claims, so an element cannot see them. It is
/// deterministic, so two replicas that convert the same claims produce
/// the same change and merging them doubles nothing.
pub async fn open<Env: DocumentEnv>(
    branch: &Branch,
    entity: &Entity,
    create: Option<Format>,
    env: &Env,
) -> Result<(Document, Format), SessionError> {
    let claimed = claimed_format(branch, entity, env).await?;
    let local = LocalCell::new(&branch.subject(), entity, env);
    if let Some((document, _)) = cell::load(&local).await? {
        let format = document.format();
        if claimed.is_none() {
            declare(branch, entity, format, &[], Vec::new(), env).await?;
        }
        return Ok((document, format));
    }

    // No local bytes. The format is the claimed one (declared elsewhere,
    // or by a seed) or the one the caller creates with. Either way any
    // claims-era body still on the branch is converted: the conversion
    // is deterministic, so a replica that does it here and another that
    // did it there hold the same change. With nothing to convert this is
    // genesis, which is constant too — bytes declared elsewhere arrive
    // with the next sync pass and merge onto it.
    let format = claimed
        .or(create)
        .ok_or_else(|| SessionError::NotADocument(entity.to_string()))?;
    let Converted {
        mut document,
        retract,
    } = match format {
        Format::Text => convert_prose(branch, entity, env).await?,
        Format::Table => convert_table(branch, entity, env).await?,
    };
    cell::save(&local, &mut document, None).await?;
    let heads = document.store_heads();
    let imported = if heads == Document::genesis_heads(format)? {
        Vec::new()
    } else {
        heads
    };
    if claimed.is_none() || !retract.is_empty() || !imported.is_empty() {
        declare(branch, entity, format, &imported, retract, env).await?;
    }
    Ok((document, format))
}

/// Convert every claims-era document the branch still holds: prose
/// entities whose body is a `content` claim, and workbooks whose sheets
/// and cells are claims. A document-mode view matches on the format
/// claim, so until this runs an old entity renders nothing. Returns how
/// many were converted. Hosts call it once per branch.
pub async fn adopt_legacy<Env: DocumentEnv>(
    branch: &Branch,
    env: &Env,
) -> Result<usize, SessionError> {
    let mut found: Vec<(Entity, Format)> = Vec::new();
    for (entity, _) in select(branch, LEGACY_PROSE_CONTENT, None, None, env).await? {
        found.push((entity, Format::Text));
    }
    for (_, value) in select(branch, SHEET_TABLE, None, None, env).await? {
        if let Value::Entity(workbook) = value {
            found.push((workbook, Format::Table));
        }
    }
    found.sort_by_key(|(entity, _)| entity.to_string());
    found.dedup_by(|a, b| a.0 == b.0);
    let mut converted = 0;
    for (entity, format) in found {
        let local = LocalCell::new(&branch.subject(), &entity, env);
        if cell::load(&local).await?.is_some() {
            continue;
        }
        open(branch, &entity, Some(format), env).await?;
        converted += 1;
    }
    Ok(converted)
}

/// Assert the format claim (and an import's heads), retracting the
/// claims a conversion replaced, in one commit.
async fn declare<Env: DocumentEnv>(
    branch: &Branch,
    entity: &Entity,
    format: Format,
    heads: &[String],
    retract: Vec<Fact>,
    env: &Env,
) -> Result<(), SessionError> {
    for attempt in 0..RETRY_LIMIT {
        let claimed: BTreeSet<String> = claimed_heads(branch, entity, env)
            .await?
            .into_iter()
            .collect();
        let mut transaction =
            branch
                .transaction()
                .assert(Fact::one(FORMAT, entity, format.name().to_string())?);
        for head in heads.iter().filter(|head| !claimed.contains(*head)) {
            transaction = transaction.assert(Fact::many(HEADS, entity, head.clone())?);
        }
        for fact in retract.iter().cloned() {
            transaction = transaction.retract(fact);
        }
        match transaction.commit().publish().perform(env).await {
            Ok(_) => return Ok(()),
            Err(error) if attempt + 1 == RETRY_LIMIT => return Err(branch_error(error)),
            Err(_) => branch.refresh(env).await.map_err(branch_error)?,
        }
    }
    Err(SessionError::Contended)
}

async fn heads_or_genesis<Env: SelectProvider>(
    branch: &Branch,
    entity: &Entity,
    format: Format,
    env: &Env,
) -> Result<(Vec<String>, Vec<String>), SessionError> {
    let claimed = claimed_heads(branch, entity, env).await?;
    let effective = if claimed.is_empty() {
        Document::genesis_heads(format)?
    } else {
        claimed.clone()
    };
    Ok((claimed, effective))
}

/// The document as this branch sees it.
pub async fn read<Env: DocumentEnv>(
    branch: &Branch,
    entity: &Entity,
    create: Option<Format>,
    env: &Env,
) -> Result<Snapshot, SessionError> {
    let (mut document, format) = open(branch, entity, create, env).await?;
    let (_, heads) = heads_or_genesis(branch, entity, format, env).await?;
    let heads = document.normalize(&heads)?;
    let content = document.content(&heads)?;
    Ok(Snapshot {
        format,
        heads,
        content,
    })
}

/// Apply `edits` as one change on top of `base` — the heads the writer
/// last saw, or the branch's own when `None` — and advance the branch's
/// heads claim to include the result.
pub async fn write<Env: DocumentEnv>(
    branch: &Branch,
    entity: &Entity,
    create: Option<Format>,
    base: Option<&[String]>,
    edits: &[Edit],
    stamp: &Stamp,
    env: &Env,
) -> Result<Written, SessionError> {
    let (_, format) = open(branch, entity, create, env).await?;
    let local_cell = LocalCell::new(&branch.subject(), entity, env);
    let mut local: Option<Vec<String>> = None;

    for _ in 0..RETRY_LIMIT {
        // Reload each round: the cell is shared with other sessions.
        let (mut document, version) = match cell::load(&local_cell).await? {
            Some((document, version)) => (document, Some(version)),
            None => (Document::genesis(format)?, None),
        };
        let (claimed, effective) = heads_or_genesis(branch, entity, format, env).await?;

        // Bytes first. The edit is applied once; later rounds only retry
        // the pointer.
        let line = match &local {
            Some(line) => line.clone(),
            None => {
                let at = base.map_or_else(|| effective.clone(), <[String]>::to_vec);
                let line = document.edit_all(&at, stamp, edits)?;
                cell::save(&local_cell, &mut document, version).await?;
                local = Some(line.clone());
                line
            }
        };

        let mut union = effective.clone();
        union.extend(line.iter().cloned());
        let next = document.normalize(&union)?;

        let claimed_set: BTreeSet<&String> = claimed.iter().collect();
        let next_set: BTreeSet<&String> = next.iter().collect();
        let mut transaction = branch.transaction();
        let mut changed = false;
        // Rule 2: retract only what was read from the tree.
        for head in claimed.iter().filter(|head| !next_set.contains(head)) {
            transaction = transaction.retract(Fact::many(HEADS, entity, head.clone())?);
            changed = true;
        }
        // Rule 1: never re-assert a head the branch already claims.
        for head in next.iter().filter(|head| !claimed_set.contains(head)) {
            transaction = transaction.assert(Fact::many(HEADS, entity, head.clone())?);
            changed = true;
        }
        if changed && transaction.commit().publish().perform(env).await.is_err() {
            // The branch head moved under us: refresh and redo only
            // the pointer.
            branch.refresh(env).await.map_err(branch_error)?;
            continue;
        }
        let content = document.content(&next)?;
        return Ok(Written {
            local: line,
            snapshot: Snapshot {
                format,
                heads: next,
                content,
            },
        });
    }
    Err(SessionError::Contended)
}

/// The overlay fact a host records when a document command is refused:
/// why, on the COMMAND's own entity, so the page that asked can read it.
/// Overlay-only — a refusal is not worth a commit.
pub fn failure(command: &Entity, document: &Entity, reason: &str) -> impl Statement + Clone {
    Fact {
        the: FAILURE.parse().expect("a static attribute"),
        of: command.clone(),
        is: format!("{document}: {reason}").into(),
        unique: true,
    }
}

/// Run the sync pass for `entity` against the remote `branch` tracks.
/// A branch that tracks no remote has nothing to sync with.
pub async fn sync<Env>(branch: &Branch, entity: &Entity, env: &Env) -> Result<Outcome, SessionError>
where
    Env: DocumentEnv + PushProvider,
{
    let Some(Upstream::Remote { remote: name, .. }) = branch.upstream() else {
        return Ok(Outcome::default());
    };
    let subject = branch.subject();
    let remote = subject
        .remote(name.clone())
        .load()
        .perform(env)
        .await
        .map_err(branch_error)?;
    let address = remote.address();
    let local = LocalCell::new(&subject, entity, env);
    let marker = LocalCell::marker(&subject, &name, entity, env);
    let there = RemoteCell::new(
        &Subject::from(address.subject().clone()),
        entity,
        address.site().clone(),
        env,
    );
    Ok(sync_pass(&local, &there, &marker).await?)
}

/// Whether `entity` holds changes its remote has not seen, judged from
/// local state alone: the store's heads against the sync marker's.
pub async fn is_dirty<Env>(
    branch: &Branch,
    entity: &Entity,
    env: &Env,
) -> Result<bool, SessionError>
where
    Env: DocumentEnv,
{
    let Some(Upstream::Remote { remote: name, .. }) = branch.upstream() else {
        return Ok(false);
    };
    let subject = branch.subject();
    let Some((mut document, _)) = cell::load(&LocalCell::new(&subject, entity, env)).await? else {
        return Ok(false);
    };
    let marker = LocalCell::marker(&subject, &name, entity, env);
    let known: Marker = match marker.resolve().await? {
        Some((bytes, _)) => serde_json::from_slice(&bytes).unwrap_or_default(),
        None => Marker::default(),
    };
    Ok(document.store_heads() != known.heads)
}

/// The entity a mirrored sheet or cell sits on. Mirror facts own their
/// entities, so replacing them is "drop the entity, assert again".
fn mirror_entity(document: &Entity, rest: &str) -> Result<Entity, SessionError> {
    format!("mirror:{}/{rest}", cell::cell_id(document))
        .parse()
        .map_err(|_| SessionError::Branch(format!("bad mirror entity for {rest}")))
}

/// Mirror the document, as this branch sees it, into the branch's
/// session overlay: `document/text` for a text document, `table/sheet`
/// and `table/cell` facts for a workbook. In memory only — never
/// committed, never synced; every replica derives the same facts.
///
/// An overlay retract is a tombstone, not a removal, so an update drops
/// the mirror's entities and asserts again. Returns whether the overlay
/// changed; the host then polls the branch's subscriptions.
pub async fn mirror<Env: DocumentEnv>(
    branch: &Branch,
    entity: &Entity,
    env: &Env,
) -> Result<bool, SessionError> {
    let snapshot = read(branch, entity, None, env).await?;
    let prefix = format!("mirror:{}/", cell::cell_id(entity));
    let overlay = branch.overlay();
    match &snapshot.content {
        Content::Text(text) => {
            // The text sits on the document entity itself, so a query
            // joins it with the entity's other claims. Only this one
            // attribute is ever overlaid there.
            overlay.retain_entities(|overlaid| overlaid != entity);
            overlay.assert(Fact::one(TEXT, entity, text.clone())?);
        }
        Content::Table(table) => {
            overlay.retain_entities(|overlaid| !overlaid.to_string().starts_with(&prefix));
            for sheet in &table.sheets {
                let sheet_entity = mirror_entity(entity, &sheet.id)?;
                overlay.assert(Fact::one(
                    SHEET_TABLE,
                    &sheet_entity,
                    Value::Entity(entity.clone()),
                )?);
                overlay.assert(Fact::one(SHEET_NAME, &sheet_entity, sheet.name.clone())?);
                overlay.assert(Fact::one(SHEET_ORDER, &sheet_entity, sheet.order.clone())?);
                for (at, content) in &sheet.cells {
                    let cell_entity = mirror_entity(entity, &format!("{}/{at}", sheet.id))?;
                    overlay.assert(Fact::one(
                        CELL_SHEET,
                        &cell_entity,
                        Value::Entity(sheet_entity.clone()),
                    )?);
                    overlay.assert(Fact::one(CELL_AT, &cell_entity, at.clone())?);
                    overlay.assert(Fact::one(CELL_CONTENT, &cell_entity, content.clone())?);
                    if let Some(style) = sheet.styles.get(at) {
                        overlay.assert(Fact::one(CELL_STYLE, &cell_entity, style.clone())?);
                    }
                }
            }
        }
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dialog_operator::helpers::{test_operator_with_profile, test_repo};
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_browser);

    fn entity(text: &str) -> Entity {
        text.parse().unwrap()
    }

    fn stamp() -> Stamp {
        Stamp {
            author: Some("did:key:zTest".into()),
            time: 1,
        }
    }

    fn set(text: &str) -> Vec<Edit> {
        vec![Edit::SetText { text: text.into() }]
    }

    /// Every `(entity, value)` of `the` as a QUERY sees it: tree plus the
    /// session overlay. `select` above reads the tree alone.
    async fn queried<Env: SelectProvider>(
        branch: &Branch,
        the: &str,
        env: &Env,
    ) -> anyhow::Result<Vec<(Entity, Value)>> {
        use dialog_query::{DynamicAttributeQuery, Output as _, Term, The};
        let the: The = the.parse().map_err(|_| anyhow::anyhow!("bad attribute"))?;
        let query = DynamicAttributeQuery::new(
            Term::from(the),
            Term::var("of"),
            Term::var("is"),
            Term::var("cause"),
            None,
        );
        let claims = branch.query().select(query).perform(env).try_vec().await?;
        Ok(claims
            .into_iter()
            .map(|claim| (claim.of, claim.is))
            .collect())
    }

    async fn queried_texts<Env: SelectProvider>(
        branch: &Branch,
        the: &str,
        env: &Env,
    ) -> anyhow::Result<Vec<String>> {
        Ok(queried(branch, the, env)
            .await?
            .into_iter()
            .filter_map(|(_, value)| String::try_from(value).ok())
            .collect())
    }

    fn text_of(snapshot: &Snapshot) -> String {
        match &snapshot.content {
            Content::Text(text) => text.clone(),
            Content::Table(_) => panic!("expected text"),
        }
    }

    #[dialog_common::test]
    async fn it_creates_edits_and_reads_a_document() -> anyhow::Result<()> {
        let (operator, profile) = test_operator_with_profile().await;
        let repo = test_repo(&operator, &profile).await;
        let branch = repo.branch("main").open().perform(&operator).await?;
        let doc = entity("id:prose/doc");

        assert!(matches!(
            read(&branch, &doc, None, &operator).await,
            Err(SessionError::NotADocument(_))
        ));

        let created = read(&branch, &doc, Some(Format::Text), &operator).await?;
        assert_eq!(text_of(&created), "");
        assert_eq!(
            claimed_format(&branch, &doc, &operator).await?,
            Some(Format::Text)
        );
        assert!(
            claimed_heads(&branch, &doc, &operator).await?.is_empty(),
            "genesis needs no pointer"
        );

        let written = write(
            &branch,
            &doc,
            None,
            None,
            &set("hello"),
            &stamp(),
            &operator,
        )
        .await?;
        assert_eq!(text_of(&written.snapshot), "hello");
        assert_eq!(
            claimed_heads(&branch, &doc, &operator).await?,
            written.snapshot.heads
        );

        let again = write(
            &branch,
            &doc,
            None,
            None,
            &set("hello world"),
            &stamp(),
            &operator,
        )
        .await?;
        let claimed = claimed_heads(&branch, &doc, &operator).await?;
        assert_eq!(
            claimed, again.snapshot.heads,
            "the old head is retracted, the new one asserted"
        );
        assert_eq!(claimed.len(), 1);
        assert_eq!(
            text_of(&read(&branch, &doc, None, &operator).await?),
            "hello world"
        );
        Ok(())
    }

    #[dialog_common::test]
    async fn it_writes_no_pointer_when_the_heads_did_not_change() -> anyhow::Result<()> {
        let (operator, profile) = test_operator_with_profile().await;
        let repo = test_repo(&operator, &profile).await;
        let branch = repo.branch("main").open().perform(&operator).await?;
        let doc = entity("id:prose/doc");
        write(
            &branch,
            &doc,
            Some(Format::Text),
            None,
            &set("same"),
            &stamp(),
            &operator,
        )
        .await?;
        let revision = branch.revision();
        write(&branch, &doc, None, None, &set("same"), &stamp(), &operator).await?;
        assert_eq!(
            branch.revision(),
            revision,
            "an edit that changes nothing commits nothing"
        );
        Ok(())
    }

    #[dialog_common::test]
    async fn it_reconciles_a_writer_that_kept_typing() -> anyhow::Result<()> {
        let (operator, profile) = test_operator_with_profile().await;
        let repo = test_repo(&operator, &profile).await;
        let branch = repo.branch("main").open().perform(&operator).await?;
        let doc = entity("id:prose/doc");
        let start = write(
            &branch,
            &doc,
            Some(Format::Text),
            None,
            &set("start"),
            &stamp(),
            &operator,
        )
        .await?;

        // Someone else edits the branch after the element last looked.
        write(
            &branch,
            &doc,
            None,
            None,
            &[Edit::Insert {
                after: "start".into(),
                text: " + remote".into(),
            }],
            &stamp(),
            &operator,
        )
        .await?;

        // The element sends its text against the heads it knew.
        let stale = start.snapshot.heads.clone();
        let sent = write(
            &branch,
            &doc,
            None,
            Some(&stale),
            &set("local + start"),
            &stamp(),
            &operator,
        )
        .await?;
        let merged = text_of(&sent.snapshot);
        assert!(
            merged.contains("local + ") && merged.contains(" + remote"),
            "{merged:?}"
        );

        // It typed more during the round trip and sends again from the
        // heads the reply said its text corresponds to: nothing doubles.
        let next = write(
            &branch,
            &doc,
            None,
            Some(&sent.local),
            &set("more local + start"),
            &stamp(),
            &operator,
        )
        .await?;
        let text = text_of(&next.snapshot);
        assert_eq!(text.matches("local").count(), 1, "{text:?}");
        assert!(
            text.contains("more ") && text.contains(" + remote"),
            "{text:?}"
        );
        Ok(())
    }

    #[dialog_common::test]
    async fn it_isolates_a_document_between_branches() -> anyhow::Result<()> {
        let (operator, profile) = test_operator_with_profile().await;
        let repo = test_repo(&operator, &profile).await;
        let main = repo.branch("main").open().perform(&operator).await?;
        let feature = repo.branch("feature").open().perform(&operator).await?;
        let doc = entity("id:prose/doc");

        write(
            &main,
            &doc,
            Some(Format::Text),
            None,
            &set("on main"),
            &stamp(),
            &operator,
        )
        .await?;
        write(
            &feature,
            &doc,
            Some(Format::Text),
            None,
            &set("on feature"),
            &stamp(),
            &operator,
        )
        .await?;

        assert_eq!(
            text_of(&read(&main, &doc, None, &operator).await?),
            "on main"
        );
        assert_eq!(
            text_of(&read(&feature, &doc, None, &operator).await?),
            "on feature"
        );
        Ok(())
    }

    #[dialog_common::test]
    async fn it_converts_an_existing_prose_body_once() -> anyhow::Result<()> {
        let (operator, profile) = test_operator_with_profile().await;
        let repo = test_repo(&operator, &profile).await;
        let branch = repo.branch("main").open().perform(&operator).await?;
        let doc = entity("id:prose/doc");
        let envelope = "Tonk-Prose-Version: 1\r\nETag: \"42\"\r\nContent-Type: text/markdown\r\n\r\n# Hello\n\nbody";
        branch
            .transaction()
            .assert(Fact::one(LEGACY_PROSE_CONTENT, &doc, envelope.to_string())?)
            .commit()
            .publish()
            .perform(&operator)
            .await?;

        let snapshot = read(&branch, &doc, Some(Format::Text), &operator).await?;
        assert_eq!(text_of(&snapshot), "# Hello\n\nbody");
        assert!(
            texts(&branch, LEGACY_PROSE_CONTENT, &doc, &operator)
                .await?
                .is_empty(),
            "the old body claim is retracted"
        );
        assert_eq!(
            snapshot.heads,
            claimed_heads(&branch, &doc, &operator).await?
        );
        assert_eq!(
            text_of(&read(&branch, &doc, Some(Format::Text), &operator).await?),
            "# Hello\n\nbody"
        );
        Ok(())
    }

    #[dialog_common::test]
    async fn it_mirrors_a_document_into_the_overlay() -> anyhow::Result<()> {
        let (operator, profile) = test_operator_with_profile().await;
        let repo = test_repo(&operator, &profile).await;
        let branch = repo.branch("main").open().perform(&operator).await?;
        let doc = entity("id:prose/doc");
        write(
            &branch,
            &doc,
            Some(Format::Text),
            None,
            &set("first"),
            &stamp(),
            &operator,
        )
        .await?;
        let revision = branch.revision();
        mirror(&branch, &doc, &operator).await?;
        assert_eq!(
            queried_texts(&branch, TEXT, &operator).await?,
            vec!["first".to_string()]
        );
        assert!(
            texts(&branch, TEXT, &doc, &operator).await?.is_empty(),
            "the tree holds no mirror fact"
        );

        write(
            &branch,
            &doc,
            None,
            None,
            &set("second"),
            &stamp(),
            &operator,
        )
        .await?;
        let after_write = branch.revision();
        mirror(&branch, &doc, &operator).await?;
        assert_eq!(
            queried_texts(&branch, TEXT, &operator).await?,
            vec!["second".to_string()],
            "an update leaves only the new value"
        );
        assert_ne!(revision, after_write);
        assert_eq!(branch.revision(), after_write, "the mirror commits nothing");
        Ok(())
    }

    #[dialog_common::test]
    async fn it_mirrors_a_workbook_as_sheet_and_cell_facts() -> anyhow::Result<()> {
        let (operator, profile) = test_operator_with_profile().await;
        let repo = test_repo(&operator, &profile).await;
        let branch = repo.branch("main").open().perform(&operator).await?;
        let book = entity("id:table/book");
        let edits = vec![
            Edit::Put {
                path: "sheets/s1/name".into(),
                value: "Sheet1".into(),
            },
            Edit::Put {
                path: "sheets/s1/cells/B2".into(),
                value: "=A1*2".into(),
            },
        ];
        write(
            &branch,
            &book,
            Some(Format::Table),
            None,
            &edits,
            &stamp(),
            &operator,
        )
        .await?;
        mirror(&branch, &book, &operator).await?;

        let sheets = queried(&branch, SHEET_TABLE, &operator).await?;
        assert_eq!(sheets.len(), 1);
        assert_eq!(sheets[0].1, Value::Entity(book.clone()));
        assert_eq!(
            queried_texts(&branch, CELL_CONTENT, &operator).await?,
            vec!["=A1*2".to_string()]
        );

        write(
            &branch,
            &book,
            None,
            None,
            &[Edit::Remove {
                path: "sheets/s1/cells/B2".into(),
            }],
            &stamp(),
            &operator,
        )
        .await?;
        mirror(&branch, &book, &operator).await?;
        assert!(
            queried_texts(&branch, CELL_CONTENT, &operator)
                .await?
                .is_empty(),
            "a removed cell leaves the mirror"
        );
        Ok(())
    }

    #[dialog_common::test]
    async fn it_adopts_claims_era_documents_in_one_sweep() -> anyhow::Result<()> {
        let (operator, profile) = test_operator_with_profile().await;
        let repo = test_repo(&operator, &profile).await;
        let branch = repo.branch("main").open().perform(&operator).await?;
        let prose = entity("id:prose/old");
        let book = entity("id:table/book");
        let sheet = entity("id:table/book/sheet1");
        let first = entity("id:table/book/a");
        let twin = entity("id:table/book/b");
        let mut transaction = branch
            .transaction()
            .assert(Fact::one(
                LEGACY_PROSE_CONTENT,
                &prose,
                "old body".to_string(),
            )?)
            .assert(Fact::one(SHEET_TABLE, &sheet, Value::Entity(book.clone()))?)
            .assert(Fact::one(SHEET_NAME, &sheet, "Sheet1".to_string())?)
            .assert(Fact::one(SHEET_ORDER, &sheet, "m".to_string())?);
        // Two claims-era cell entities at ONE address: the defect
        // document mode removes. The smallest entity id wins.
        for (cell, content) in [(&first, "kept"), (&twin, "dropped")] {
            transaction = transaction
                .assert(Fact::one(CELL_SHEET, cell, Value::Entity(sheet.clone()))?)
                .assert(Fact::one(CELL_AT, cell, "B2".to_string())?)
                .assert(Fact::one(CELL_CONTENT, cell, content.to_string())?);
        }
        transaction.commit().publish().perform(&operator).await?;

        assert_eq!(adopt_legacy(&branch, &operator).await?, 2);
        assert_eq!(
            adopt_legacy(&branch, &operator).await?,
            0,
            "a second sweep finds nothing"
        );

        assert_eq!(
            text_of(&read(&branch, &prose, None, &operator).await?),
            "old body"
        );
        let Content::Table(table) = read(&branch, &book, None, &operator).await?.content else {
            panic!("expected a workbook");
        };
        assert_eq!(table.sheets.len(), 1);
        assert_eq!(table.sheets[0].name, "Sheet1");
        assert_eq!(table.sheets[0].cells["B2"], "kept");
        assert!(
            select(&branch, CELL_CONTENT, None, None, &operator)
                .await?
                .is_empty(),
            "the old cell claims are retracted"
        );
        Ok(())
    }

    #[dialog_common::test]
    async fn it_merges_a_document_when_dialog_merges_two_branches() -> anyhow::Result<()> {
        let (operator, profile) = test_operator_with_profile().await;
        let repo = test_repo(&operator, &profile).await;
        let main = repo.branch("main").open().perform(&operator).await?;
        let doc = entity("id:prose/doc");
        write(
            &main,
            &doc,
            Some(Format::Text),
            None,
            &set("base"),
            &stamp(),
            &operator,
        )
        .await?;

        let feature = repo.branch("feature").open().perform(&operator).await?;
        feature.set_upstream(&main).perform(&operator).await?;
        feature.pull().perform(&operator).await?;
        assert_eq!(
            text_of(&read(&feature, &doc, None, &operator).await?),
            "base"
        );

        // Both sides edit without seeing each other. Each write is the
        // exact pair this design emits: retract the head it saw, assert
        // its own, in one transaction.
        let ours = write(
            &feature,
            &doc,
            None,
            None,
            &[Edit::Insert {
                after: "base".into(),
                text: " feature".into(),
            }],
            &stamp(),
            &operator,
        )
        .await?;
        let theirs = write(
            &main,
            &doc,
            None,
            None,
            &[Edit::Splice {
                at: 0,
                delete: 0,
                text: "main ".into(),
            }],
            &stamp(),
            &operator,
        )
        .await?;
        assert_eq!(
            text_of(&read(&feature, &doc, None, &operator).await?),
            "base feature",
            "a branch sees only its own line"
        );

        // Dialog merges the branches. No document code runs: the merge
        // of the many-valued heads claim IS the automerge merge.
        feature
            .pull()
            .perform(&operator)
            .await?
            .expect("pull merges");
        let mut expected = ours.snapshot.heads.clone();
        expected.extend(theirs.snapshot.heads.clone());
        expected.sort();
        assert_eq!(
            claimed_heads(&feature, &doc, &operator).await?,
            expected,
            "both heads are claimed and the shared base head is gone"
        );
        assert_eq!(
            text_of(&read(&feature, &doc, None, &operator).await?),
            "main base feature"
        );

        // The next write on the merged branch collapses the heads to one.
        let next = write(
            &feature,
            &doc,
            None,
            None,
            &[Edit::Insert {
                after: "feature".into(),
                text: "!".into(),
            }],
            &stamp(),
            &operator,
        )
        .await?;
        assert_eq!(claimed_heads(&feature, &doc, &operator).await?.len(), 1);
        assert_eq!(text_of(&next.snapshot), "main base feature!");
        Ok(())
    }

    #[dialog_common::test]
    fn it_reads_the_markdown_out_of_an_envelope() {
        assert_eq!(prose_body("plain *markdown*"), "plain *markdown*");
        assert_eq!(
            prose_body("Tonk-Prose-Version: 1\nETag: \"1\"\n\nbody\n\nmore"),
            "body\n\nmore"
        );
        assert_eq!(
            prose_body("tonk-prose-version: 1\r\nETag: \"1\"\r\n\r\nbody"),
            "body"
        );
    }
}
