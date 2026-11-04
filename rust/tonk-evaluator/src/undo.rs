//! Undoing one change: staging the inverse of one revision.
//!
//! A revision's history lists every claim it recorded, each an assertion
//! or a retraction, and each assertion names the claims on the same
//! entity and attribute it superseded (its cause). The inverse of the
//! revision is then:
//!
//! - each assertion retracted;
//! - each claim an assertion superseded asserted again, with the policy
//!   that supersedes, so a cardinality-one value goes back to what it was;
//! - each retraction asserted again.
//!
//! # When a later change touched the same facts
//!
//! The undo is refused, as a whole, when any revision after it recorded a
//! claim on an entity and attribute it touched, or when what it wrote no
//! longer stands as it left it. A later revision that was itself undone,
//! and the undo that undid it, do not count: they cancel out.
//!
//! Undoing only the untouched facts would leave a state nobody chose: half
//! of an edit, a restored title beside a description someone wrote for the
//! new one. A refusal changes nothing, says which fact moved, and is
//! resolved by undoing the later change first.
//!
//! "After" is the branch log walked from its head down to the revision:
//! every revision met first is a descendant or a concurrent one, and
//! either counts as touching. Only the newest [`HISTORY_LIMIT`] revisions
//! are walked; an older change is refused as too old rather than walked
//! without bound.
//!
//! The outcome is recorded as a [`tonk_schema::Undo`] on the entity named
//! for the revision, in the undo's own commit when it undoes, or alone when
//! it refuses. The undo's commit is a revision like any other, so undoing
//! it redoes the change.

use std::collections::{BTreeSet, HashMap, HashSet};

use dialog_artifacts::history::{HistorySelector, Version};
use dialog_artifacts::{ArtifactSelector, Entity, Pick, Relation, Statement, Value};
use dialog_repository::{Branch, Transaction};
use futures_util::StreamExt as _;
use tonk_schema::{Undo, UndoReason, decode_revision};

use crate::evaluate::EvaluateEnv;

/// How many revisions back from the head an undo looks for the change.
pub const HISTORY_LIMIT: usize = 512;

/// Why a change was not undone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// The revision does not decode.
    Malformed,
    /// The revision is not among the branch's newest [`HISTORY_LIMIT`].
    NotFound,
    /// The revision recorded nothing an undo can reverse.
    Empty,
    /// The change was undone already.
    AlreadyUndone,
    /// A later change, or the branch as it stands, moved a fact the
    /// change wrote.
    ChangedSince {
        /// The entity whose fact moved.
        of: Entity,
        /// The attribute that moved.
        the: Relation,
    },
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refusal::Malformed => write!(f, "This does not name a change."),
            Refusal::NotFound => write!(
                f,
                "This change is not in the space's recent history, so it cannot be undone."
            ),
            Refusal::Empty => write!(f, "This change wrote nothing to undo."),
            Refusal::AlreadyUndone => write!(f, "This change was undone already."),
            Refusal::ChangedSince { of, the } => write!(
                f,
                "{the} of {of} changed after this change; undo the later change first."
            ),
        }
    }
}

/// What staging an undo found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UndoOutcome {
    /// The inverse is staged: `claims` claims, with the outcome record.
    Undone {
        /// How many inverse claims were staged.
        claims: usize,
        /// Each entity the undo takes back, in the order the change first
        /// touched it.
        reverted: Vec<Reverted>,
    },
    /// Nothing but the refusal is staged.
    Refused(Refusal),
}

/// One entity an undo takes back, as a receipt names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reverted {
    /// The entity.
    pub of: Entity,
    /// Its name, title, label or text, as it stands once undone (or, for
    /// what the change added, as the change wrote it). `None` when it has
    /// none of them.
    pub name: Option<String>,
    /// What the change did to it.
    pub was: Was,
}

/// What a change did to an entity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Was {
    /// Every claim on it was new: the change added it.
    Added,
    /// Every claim was a retraction: the change removed it.
    Removed,
    /// Anything else.
    Changed,
}

/// The attributes a receipt names an entity by, the first it has first.
const NAMES: &[&str] = &["name", "title", "label", "text"];

/// An undo staged on a transaction the caller commits.
pub struct StagedUndo<'a> {
    /// The transaction holding the inverse and the outcome record. It is
    /// empty for [`Refusal::AlreadyUndone`], whose record already stands,
    /// and for [`Refusal::Malformed`], which has no revision to record on.
    pub transaction: Transaction<&'a Branch>,
    /// What was staged.
    pub outcome: UndoOutcome,
}

impl StagedUndo<'_> {
    /// Whether committing the transaction writes anything.
    pub fn writes(&self) -> bool {
        !matches!(
            self.outcome,
            UndoOutcome::Refused(Refusal::Malformed | Refusal::AlreadyUndone)
        )
    }
}

/// An error reading the branch, as opposed to a refusal.
#[derive(Debug, thiserror::Error)]
#[error("undo: {0}")]
pub struct UndoError(String);

/// One claim of an inverse, written with its policy.
#[derive(Clone)]
struct Fact {
    the: Relation,
    of: Entity,
    is: Value,
    pick: Pick,
}

impl Statement for Fact {
    fn assert(self, update: &mut impl dialog_artifacts::Update) {
        update.associate(self.the, self.of, self.is, self.pick);
    }

    fn retract(self, update: &mut impl dialog_artifacts::Update) {
        update.dissociate(self.the, self.of, self.is);
    }
}

/// One record of the revision being undone.
struct Recorded {
    assertion: bool,
    the: Relation,
    of: Entity,
    is: Value,
    supersedes: Vec<Version>,
}

/// Stage the undo of the change `revision` names (encoded as
/// [`tonk_schema::encode_revision`] encodes it) on a transaction of
/// `branch`. The caller commits it when [`StagedUndo::writes`].
pub async fn stage_undo<'a, Env: EvaluateEnv>(
    branch: &'a Branch,
    revision: &str,
    env: &Env,
) -> Result<StagedUndo<'a>, UndoError> {
    let transaction = branch.transaction();
    let Some(version) = decode_revision(revision) else {
        return Ok(StagedUndo {
            transaction,
            outcome: UndoOutcome::Refused(Refusal::Malformed),
        });
    };
    let outcome = Undo::entity(revision);
    let status = values(branch, env, &outcome, "xyz.tonk.undo/status").await?;
    if status.contains(&Value::String(Undo::UNDONE.to_owned())) {
        return Ok(StagedUndo {
            transaction,
            outcome: UndoOutcome::Refused(Refusal::AlreadyUndone),
        });
    }
    let reasons = values(branch, env, &outcome, "xyz.tonk.undo/reason").await?;
    match inverse(branch, &version, env).await? {
        Err(refusal) => {
            let transaction = transaction
                .assert(Undo::new(revision, Undo::REFUSED))
                .assert(UndoReason::new(revision, refusal.to_string()));
            Ok(StagedUndo {
                transaction,
                outcome: UndoOutcome::Refused(refusal),
            })
        }
        Ok((facts, reverted)) => {
            let claims = facts.len();
            let mut transaction = transaction;
            for (assert, fact) in facts {
                transaction = if assert {
                    transaction.assert(fact)
                } else {
                    transaction.retract(fact)
                };
            }
            transaction = transaction.assert(Undo::new(revision, Undo::UNDONE));
            // A refusal from an earlier attempt no longer explains anything.
            for reason in reasons {
                if let Value::String(reason) = reason {
                    transaction = transaction.retract(UndoReason::new(revision, reason));
                }
            }
            Ok(StagedUndo {
                transaction,
                outcome: UndoOutcome::Undone { claims, reverted },
            })
        }
    }
}

/// The inverse of `version` on `branch`, as `(assert, fact)` pairs, with
/// the entities it takes back, or why it may not be applied.
async fn inverse<Env: EvaluateEnv>(
    branch: &Branch,
    version: &Version,
    env: &Env,
) -> Result<Result<(Vec<(bool, Fact)>, Vec<Reverted>), Refusal>, UndoError> {
    let Some(later) = later_revisions(branch, version, env).await? else {
        return Ok(Err(Refusal::NotFound));
    };
    let recorded = records_at(branch, version, env).await?;
    if recorded.is_empty() {
        return Ok(Err(Refusal::Empty));
    }
    let touched: BTreeSet<(String, String)> = recorded
        .iter()
        .map(|record| (record.of.to_string(), record.the.to_string()))
        .collect();
    let mut after = Vec::new();
    for later in &later {
        after.push((*later, records_at(branch, later, env).await?));
    }
    for record in standing(after).into_iter().flatten() {
        if touched.contains(&(record.of.to_string(), record.the.to_string())) {
            return Ok(Err(Refusal::ChangedSince {
                of: record.of,
                the: record.the,
            }));
        }
    }

    let reverted = reverted(branch, &recorded, env).await?;
    let mut facts = Vec::new();
    for record in recorded {
        let now = values(branch, env, &record.of, record.the.as_str()).await?;
        let stands = now.contains(&record.is);
        // What the change left must still be there: a later revision the
        // log did not reach (one merged in from below the limit) is caught
        // here rather than overwritten.
        if stands != record.assertion {
            return Ok(Err(Refusal::ChangedSince {
                of: record.of,
                the: record.the,
            }));
        }
        if !record.assertion {
            facts.push((
                true,
                Fact {
                    the: record.the,
                    of: record.of,
                    is: record.is,
                    pick: Pick::All,
                },
            ));
            continue;
        }
        facts.push((
            false,
            Fact {
                the: record.the.clone(),
                of: record.of.clone(),
                is: record.is.clone(),
                pick: Pick::All,
            },
        ));
        for prior in &record.supersedes {
            for earlier in records_at(branch, prior, env).await? {
                if earlier.assertion && earlier.of == record.of && earlier.the == record.the {
                    facts.push((
                        true,
                        Fact {
                            the: earlier.the,
                            of: earlier.of,
                            is: earlier.is,
                            pick: Pick::Last,
                        },
                    ));
                }
            }
        }
    }
    Ok(Ok((facts, reverted)))
}

/// The entities `recorded` (one revision's records) touched, each with
/// what the revision did to it and its name. The record of an undo
/// (`tonk:undo/…`) is the undo's own, not something it takes back.
async fn reverted<Env: EvaluateEnv>(
    branch: &Branch,
    recorded: &[Recorded],
    env: &Env,
) -> Result<Vec<Reverted>, UndoError> {
    let mut entities: Vec<&Entity> = Vec::new();
    for record in recorded {
        if !entities.contains(&&record.of) && !record.of.to_string().starts_with("tonk:undo/") {
            entities.push(&record.of);
        }
    }
    let mut reverted = Vec::new();
    for of in entities {
        let records: Vec<&Recorded> = recorded.iter().filter(|record| &record.of == of).collect();
        // Added when every claim it has is one the change wrote, removed
        // when the change retracted the last of them.
        let standing = claims_of(branch, of, env).await?;
        let was = if records
            .iter()
            .all(|record| record.assertion && record.supersedes.is_empty())
            && standing == records.len()
        {
            Was::Added
        } else if records.iter().all(|record| !record.assertion) && standing == 0 {
            Was::Removed
        } else {
            Was::Changed
        };
        reverted.push(Reverted {
            of: of.clone(),
            name: name(branch, of, &records, was, env).await?,
            was,
        });
    }
    Ok(reverted)
}

/// How many claims `of` has on `branch`.
async fn claims_of<Env: EvaluateEnv>(
    branch: &Branch,
    of: &Entity,
    env: &Env,
) -> Result<usize, UndoError> {
    let claims = branch
        .claims()
        .select(ArtifactSelector::new().of(of.clone()))
        .perform(env)
        .await
        .map_err(|error| UndoError(format!("read the branch: {error}")))?;
    futures_util::pin_mut!(claims);
    let mut count = 0;
    while let Some(claim) = claims.next().await {
        claim.map_err(|error| UndoError(format!("read the branch: {error}")))?;
        count += 1;
    }
    Ok(count)
}

/// The name of `of`, which `records` touched: what the change added it
/// with, or else what stands once it is undone, read from each domain the
/// records name before the undo lands (so a name the change wrote is
/// skipped there, and a name it superseded is read from what it was).
async fn name<Env: EvaluateEnv>(
    branch: &Branch,
    of: &Entity,
    records: &[&Recorded],
    was: Was,
    env: &Env,
) -> Result<Option<String>, UndoError> {
    let named = |record: &&&Recorded| {
        let the = record.the.to_string();
        let attribute = the.rsplit('/').next().unwrap_or_default().to_owned();
        NAMES.iter().position(|name| *name == attribute)
    };
    let text = |value: &Value| match value {
        Value::String(text) if !text.is_empty() => Some(text.clone()),
        _ => None,
    };
    // Added or removed, the change's own claims say it.
    if was != Was::Changed {
        let mut names: Vec<(usize, &&Recorded)> = records
            .iter()
            .filter_map(|record| named(&record).map(|rank| (rank, record)))
            .collect();
        names.sort_by_key(|(rank, _)| *rank);
        return Ok(names.into_iter().find_map(|(_, record)| text(&record.is)));
    }
    // Changed: what will stand, attribute by attribute, in each domain.
    let mut domains: Vec<String> = Vec::new();
    for record in records {
        let the = record.the.to_string();
        if let Some((domain, _)) = the.rsplit_once('/')
            && !domains.iter().any(|known| known == domain)
        {
            domains.push(domain.to_owned());
        }
    }
    for attribute in NAMES {
        for domain in &domains {
            let the = format!("{domain}/{attribute}");
            let touched: Vec<&&Recorded> = records
                .iter()
                .filter(|record| record.the.to_string() == the)
                .collect();
            let mut stands = values(branch, env, of, &the).await?;
            for record in &touched {
                if record.assertion {
                    stands.retain(|value| value != &record.is);
                    for prior in &record.supersedes {
                        for earlier in records_at(branch, prior, env).await? {
                            if earlier.assertion && &earlier.of == of && earlier.the == record.the {
                                stands.push(earlier.is);
                            }
                        }
                    }
                } else {
                    stands.push(record.is.clone());
                }
            }
            if let Some(name) = stands.iter().find_map(text) {
                return Ok(Some(name));
            }
        }
    }
    Ok(None)
}

/// The records of the `later` revisions that still stand.
///
/// A revision that was undone, and the undo that undid it, cancel out when
/// both are among them, unless that undo was itself undone: a redo leaves
/// the undo and the redo cancelled and the original change standing.
fn standing(later: Vec<(Version, Vec<Recorded>)>) -> Vec<Vec<Recorded>> {
    let versions: HashSet<Version> = later.iter().map(|(version, _)| *version).collect();
    // Each undo among them, by what it undid.
    let mut undoers: HashMap<Version, Vec<Version>> = HashMap::new();
    let mut targets: HashMap<Version, Version> = HashMap::new();
    for (version, records) in &later {
        for undid in records.iter().filter_map(undid) {
            if versions.contains(&undid) {
                undoers.entry(undid).or_default().push(*version);
                targets.insert(*version, undid);
            }
        }
    }
    let mut undone = HashMap::new();
    let mut cancelled = HashSet::new();
    for version in &versions {
        if is_undone(version, &undoers, &mut undone) {
            cancelled.insert(*version);
        }
    }
    // An undo in force cancels with what it undid.
    for (undo, target) in &targets {
        if !is_undone(undo, &undoers, &mut undone) && cancelled.contains(target) {
            cancelled.insert(*undo);
        }
    }
    later
        .into_iter()
        .filter(|(version, _)| !cancelled.contains(version))
        .map(|(_, records)| records)
        .collect()
}

/// Whether `version` was undone by an undo that is itself in force. An
/// undo is always later than what it undid, so the chain ends.
fn is_undone(
    version: &Version,
    undoers: &HashMap<Version, Vec<Version>>,
    undone: &mut HashMap<Version, bool>,
) -> bool {
    if let Some(known) = undone.get(version) {
        return *known;
    }
    let answer = undoers
        .get(version)
        .is_some_and(|by| by.iter().any(|undo| !is_undone(undo, undoers, undone)));
    undone.insert(*version, answer);
    answer
}

/// The revision `record` says was undone, when it is an undo's record of
/// its outcome.
fn undid(record: &Recorded) -> Option<Version> {
    if !record.assertion
        || record.the.as_str() != "xyz.tonk.undo/status"
        || record.is != Value::String(Undo::UNDONE.to_owned())
    {
        return None;
    }
    let entity = record.of.to_string();
    decode_revision(entity.strip_prefix("tonk:undo/")?)
}

/// The revisions the branch log walks from its head before reaching
/// `version`, or `None` when it does not reach it within
/// [`HISTORY_LIMIT`].
async fn later_revisions<Env: EvaluateEnv>(
    branch: &Branch,
    version: &Version,
    env: &Env,
) -> Result<Option<Vec<Version>>, UndoError> {
    let log = branch
        .log(env, HISTORY_LIMIT)
        .await
        .map_err(|error| UndoError(format!("read the branch log: {error}")))?;
    let mut later = Vec::new();
    for (logged, _) in log {
        if &logged == version {
            return Ok(Some(later));
        }
        later.push(logged);
    }
    Ok(None)
}

/// Every claim `version` recorded.
async fn records_at<Env: EvaluateEnv>(
    branch: &Branch,
    version: &Version,
    env: &Env,
) -> Result<Vec<Recorded>, UndoError> {
    let history = branch.history(env);
    let records = history.select(HistorySelector::At(*version));
    futures_util::pin_mut!(records);
    let mut recorded = Vec::new();
    while let Some(record) = records.next().await {
        let (_, record) =
            record.map_err(|error| UndoError(format!("read the change's history: {error}")))?;
        let assertion = record.is_assertion();
        let claim = record.claim();
        recorded.push(Recorded {
            assertion,
            the: claim.the.clone(),
            of: claim.of.clone(),
            is: claim.is.clone(),
            supersedes: claim.cause.versions().to_vec(),
        });
    }
    Ok(recorded)
}

/// The values `of` holds for `the` on the branch as it stands.
async fn values<Env: EvaluateEnv>(
    branch: &Branch,
    env: &Env,
    of: &Entity,
    the: &str,
) -> Result<Vec<Value>, UndoError> {
    let the: Relation = the
        .parse()
        .map_err(|error| UndoError(format!("attribute {the}: {error}")))?;
    let claims = branch
        .claims()
        .select(ArtifactSelector::new().of(of.clone()).the(the))
        .perform(env)
        .await
        .map_err(|error| UndoError(format!("read the branch: {error}")))?;
    futures_util::pin_mut!(claims);
    let mut values = Vec::new();
    while let Some(claim) = claims.next().await {
        let claim = claim.map_err(|error| UndoError(format!("read the branch: {error}")))?;
        values.push(
            claim
                .value()
                .map_err(|error| UndoError(format!("decode a value: {error}")))?,
        );
    }
    Ok(values)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_browser);

    use dialog_peer::helpers::{test_repo, test_session_with_peer};
    use tonk_notation::parse;
    use tonk_schema::encode_revision;

    use crate::evaluate::SyntaxEvaluateExt as _;

    /// The session the tests act as.
    type Operator =
        dialog_peer::Peer<dialog_storage::provider::storage::VolatileSpace, dialog_peer::Session>;

    /// A note: a cardinality-one title and a many-valued tag.
    const SCHEMA: &str = "attribute!: &n/title
  description: The title.
  the: io.undo.note/title
  as: text
  cardinality: :one
attribute!: &n/tag
  description: A tag.
  the: io.undo.note/tag
  as: text
  cardinality: :many
concept!: &note
  description: A note.
  with:
    title: n/title
  maybe:
    tag: n/tag
";

    const NOTE: &str = "urn:undo:note";

    /// Evaluate `document` on `branch` and commit it: the revision it
    /// made, encoded as an undo names it.
    async fn write(branch: &Branch, env: &Operator, document: &str) -> String {
        let syntax = parse(document).syntax.expect("syntax");
        let revision = syntax
            .evaluate(branch.transaction())
            .perform(env)
            .await
            .unwrap_or_else(|error| panic!("evaluate {document:?}: {error}"))
            .commit()
            .publish()
            .perform(env)
            .await
            .unwrap_or_else(|error| panic!("commit {document:?}: {error}"));
        encode_revision(&revision.version())
    }

    /// Undo `revision` and commit what was staged.
    async fn undo(branch: &Branch, env: &Operator, revision: &str) -> UndoOutcome {
        let staged = stage_undo(branch, revision, env).await.expect("stage");
        let outcome = staged.outcome.clone();
        if staged.writes() {
            staged
                .transaction
                .commit()
                .publish()
                .perform(env)
                .await
                .expect("commit the undo");
        }
        outcome
    }

    async fn of(branch: &Branch, env: &Operator, the: &str) -> Vec<Value> {
        let mut found = values(branch, env, &NOTE.parse().unwrap(), the)
            .await
            .unwrap();
        found.sort_by_key(|value| format!("{value:?}"));
        found
    }

    /// What an undone outcome names: each entity, its name, and what the
    /// change did to it.
    fn reverted(outcome: &UndoOutcome) -> Vec<(String, Option<String>, Was)> {
        match outcome {
            UndoOutcome::Undone { reverted, .. } => reverted
                .iter()
                .map(|reverted| (reverted.of.to_string(), reverted.name.clone(), reverted.was))
                .collect(),
            UndoOutcome::Refused(refusal) => panic!("refused: {refusal}"),
        }
    }

    fn text(value: &str) -> Value {
        Value::String(value.to_owned())
    }

    async fn status(branch: &Branch, env: &Operator, revision: &str) -> Vec<Value> {
        values(branch, env, &Undo::entity(revision), "xyz.tonk.undo/status")
            .await
            .unwrap()
    }

    #[dialog_common::test]
    async fn it_undoes_an_assertion() {
        let (operator, profile) = test_session_with_peer().await;
        let repo = test_repo(&operator, &profile).await;
        let branch = repo.branch("main").open().perform(&operator).await.unwrap();
        write(&branch, &operator, SCHEMA).await;
        let added = write(
            &branch,
            &operator,
            &format!("note!:\n  this: {NOTE}\n  title: \"one\"\n  tag: \"a\"\n"),
        )
        .await;

        let outcome = undo(&branch, &operator, &added).await;

        assert!(
            matches!(outcome, UndoOutcome::Undone { claims: 2, .. }),
            "{outcome:?}"
        );
        assert_eq!(
            reverted(&outcome),
            vec![(NOTE.to_owned(), Some("one".to_owned()), Was::Added)]
        );
        assert!(
            of(&branch, &operator, "io.undo.note/title")
                .await
                .is_empty()
        );
        assert!(of(&branch, &operator, "io.undo.note/tag").await.is_empty());
        assert_eq!(
            status(&branch, &operator, &added).await,
            vec![text(Undo::UNDONE)]
        );
    }

    #[dialog_common::test]
    async fn it_undoes_a_retraction() {
        let (operator, profile) = test_session_with_peer().await;
        let repo = test_repo(&operator, &profile).await;
        let branch = repo.branch("main").open().perform(&operator).await.unwrap();
        write(&branch, &operator, SCHEMA).await;
        write(
            &branch,
            &operator,
            &format!("note!:\n  this: {NOTE}\n  title: \"one\"\n  tag: \"a\"\n"),
        )
        .await;
        let removed = write(
            &branch,
            &operator,
            &format!("note!:\n  this: {NOTE}\n  tag: _\n"),
        )
        .await;
        assert!(of(&branch, &operator, "io.undo.note/tag").await.is_empty());

        let outcome = undo(&branch, &operator, &removed).await;

        assert!(matches!(outcome, UndoOutcome::Undone { .. }), "{outcome:?}");
        // A tag came off a note that stays: changed, named by its title.
        assert_eq!(
            reverted(&outcome),
            vec![(NOTE.to_owned(), Some("one".to_owned()), Was::Changed)]
        );
        assert_eq!(
            of(&branch, &operator, "io.undo.note/tag").await,
            vec![text("a")]
        );
        assert_eq!(
            of(&branch, &operator, "io.undo.note/title").await,
            vec![text("one")]
        );
    }

    #[dialog_common::test]
    async fn it_restores_what_a_cardinality_one_write_superseded() {
        let (operator, profile) = test_session_with_peer().await;
        let repo = test_repo(&operator, &profile).await;
        let branch = repo.branch("main").open().perform(&operator).await.unwrap();
        write(&branch, &operator, SCHEMA).await;
        write(
            &branch,
            &operator,
            &format!("note!:\n  this: {NOTE}\n  title: \"one\"\n"),
        )
        .await;
        let renamed = write(
            &branch,
            &operator,
            &format!("note!:\n  this: {NOTE}\n  title: \"two\"\n"),
        )
        .await;
        assert_eq!(
            of(&branch, &operator, "io.undo.note/title").await,
            vec![text("two")]
        );

        let outcome = undo(&branch, &operator, &renamed).await;

        assert!(matches!(outcome, UndoOutcome::Undone { .. }), "{outcome:?}");
        // Named as it stands once undone.
        assert_eq!(
            reverted(&outcome),
            vec![(NOTE.to_owned(), Some("one".to_owned()), Was::Changed)]
        );
        assert_eq!(
            of(&branch, &operator, "io.undo.note/title").await,
            vec![text("one")]
        );
    }

    #[dialog_common::test]
    async fn it_refuses_a_change_a_later_one_touched_until_that_is_undone() {
        let (operator, profile) = test_session_with_peer().await;
        let repo = test_repo(&operator, &profile).await;
        let branch = repo.branch("main").open().perform(&operator).await.unwrap();
        write(&branch, &operator, SCHEMA).await;
        let first = write(
            &branch,
            &operator,
            &format!("note!:\n  this: {NOTE}\n  title: \"one\"\n"),
        )
        .await;
        let later = write(
            &branch,
            &operator,
            &format!("note!:\n  this: {NOTE}\n  title: \"two\"\n"),
        )
        .await;

        let outcome = undo(&branch, &operator, &first).await;

        assert!(
            matches!(
                &outcome,
                UndoOutcome::Refused(Refusal::ChangedSince { the, .. })
                    if the.as_str() == "io.undo.note/title"
            ),
            "{outcome:?}"
        );
        // Nothing moved, and the refusal is on record with its reason.
        assert_eq!(
            of(&branch, &operator, "io.undo.note/title").await,
            vec![text("two")]
        );
        assert_eq!(
            status(&branch, &operator, &first).await,
            vec![text(Undo::REFUSED)]
        );
        let reasons = values(
            &branch,
            &operator,
            &Undo::entity(&first),
            "xyz.tonk.undo/reason",
        )
        .await
        .unwrap();
        assert_eq!(reasons.len(), 1, "{reasons:?}");

        // Undoing the later change first clears the way.
        let outcome = undo(&branch, &operator, &later).await;
        assert!(matches!(outcome, UndoOutcome::Undone { .. }), "{outcome:?}");
        let outcome = undo(&branch, &operator, &first).await;
        assert!(matches!(outcome, UndoOutcome::Undone { .. }), "{outcome:?}");
        assert!(
            of(&branch, &operator, "io.undo.note/title")
                .await
                .is_empty()
        );
        assert_eq!(
            status(&branch, &operator, &first).await,
            vec![text(Undo::UNDONE)]
        );
        let reasons = values(
            &branch,
            &operator,
            &Undo::entity(&first),
            "xyz.tonk.undo/reason",
        )
        .await
        .unwrap();
        assert!(reasons.is_empty(), "{reasons:?}");
    }

    /// A later change that was undone and then redone stands again, so it
    /// still blocks undoing an earlier change to the same attribute.
    #[dialog_common::test]
    async fn it_refuses_a_change_a_redone_later_one_touched() {
        let (operator, profile) = test_session_with_peer().await;
        let repo = test_repo(&operator, &profile).await;
        let branch = repo.branch("main").open().perform(&operator).await.unwrap();
        write(&branch, &operator, SCHEMA).await;
        let first = write(
            &branch,
            &operator,
            &format!("note!:\n  this: {NOTE}\n  title: \"one\"\n  tag: \"a\"\n"),
        )
        .await;
        let later = write(
            &branch,
            &operator,
            &format!("note!:\n  this: {NOTE}\n  tag: \"b\"\n"),
        )
        .await;
        let staged = stage_undo(&branch, &later, &operator).await.unwrap();
        let undone = staged
            .transaction
            .commit()
            .publish()
            .perform(&operator)
            .await
            .unwrap();
        let undone = encode_revision(&undone.version());
        let redone = undo(&branch, &operator, &undone).await;
        assert!(matches!(redone, UndoOutcome::Undone { .. }), "{redone:?}");
        assert_eq!(
            of(&branch, &operator, "io.undo.note/tag").await,
            vec![text("a"), text("b")]
        );

        let outcome = undo(&branch, &operator, &first).await;

        assert!(
            matches!(
                &outcome,
                UndoOutcome::Refused(Refusal::ChangedSince { the, .. })
                    if the.as_str() == "io.undo.note/tag"
            ),
            "{outcome:?}"
        );
        assert_eq!(
            of(&branch, &operator, "io.undo.note/title").await,
            vec![text("one")]
        );
    }

    #[dialog_common::test]
    async fn it_leaves_a_change_to_another_attribute_alone() {
        let (operator, profile) = test_session_with_peer().await;
        let repo = test_repo(&operator, &profile).await;
        let branch = repo.branch("main").open().perform(&operator).await.unwrap();
        write(&branch, &operator, SCHEMA).await;
        let titled = write(
            &branch,
            &operator,
            &format!("note!:\n  this: {NOTE}\n  title: \"one\"\n"),
        )
        .await;
        write(
            &branch,
            &operator,
            &format!("note!:\n  this: {NOTE}\n  tag: \"a\"\n"),
        )
        .await;

        let outcome = undo(&branch, &operator, &titled).await;

        assert!(matches!(outcome, UndoOutcome::Undone { .. }), "{outcome:?}");
        assert!(
            of(&branch, &operator, "io.undo.note/title")
                .await
                .is_empty()
        );
        assert_eq!(
            of(&branch, &operator, "io.undo.note/tag").await,
            vec![text("a")]
        );
    }

    #[dialog_common::test]
    async fn it_undoes_a_change_once_and_redoes_it_by_undoing_the_undo() {
        let (operator, profile) = test_session_with_peer().await;
        let repo = test_repo(&operator, &profile).await;
        let branch = repo.branch("main").open().perform(&operator).await.unwrap();
        write(&branch, &operator, SCHEMA).await;
        let added = write(
            &branch,
            &operator,
            &format!("note!:\n  this: {NOTE}\n  title: \"one\"\n"),
        )
        .await;

        let staged = stage_undo(&branch, &added, &operator).await.unwrap();
        let undone = staged
            .transaction
            .commit()
            .publish()
            .perform(&operator)
            .await
            .unwrap();
        let undone = encode_revision(&undone.version());
        assert_eq!(
            undo(&branch, &operator, &added).await,
            UndoOutcome::Refused(Refusal::AlreadyUndone)
        );

        let outcome = undo(&branch, &operator, &undone).await;

        assert!(matches!(outcome, UndoOutcome::Undone { .. }), "{outcome:?}");
        assert_eq!(
            of(&branch, &operator, "io.undo.note/title").await,
            vec![text("one")]
        );
        assert!(status(&branch, &operator, &added).await.is_empty());
        // The redo names the note it brings back, not the undo's record.
        assert_eq!(
            reverted(&outcome),
            vec![(NOTE.to_owned(), Some("one".to_owned()), Was::Removed)]
        );
    }

    #[dialog_common::test]
    async fn it_names_a_removed_note_by_what_it_was() {
        let (operator, profile) = test_session_with_peer().await;
        let repo = test_repo(&operator, &profile).await;
        let branch = repo.branch("main").open().perform(&operator).await.unwrap();
        write(&branch, &operator, SCHEMA).await;
        write(
            &branch,
            &operator,
            &format!("note!:\n  this: {NOTE}\n  title: \"one\"\n  tag: \"a\"\n"),
        )
        .await;
        let removed = write(
            &branch,
            &operator,
            &format!("note!:\n  this: {NOTE}\n  title: _\n  tag: _\n"),
        )
        .await;

        let outcome = undo(&branch, &operator, &removed).await;

        assert_eq!(
            reverted(&outcome),
            vec![(NOTE.to_owned(), Some("one".to_owned()), Was::Removed)]
        );
        assert_eq!(
            of(&branch, &operator, "io.undo.note/title").await,
            vec![text("one")]
        );
    }

    #[dialog_common::test]
    async fn it_refuses_a_revision_it_cannot_find() {
        let (operator, profile) = test_session_with_peer().await;
        let repo = test_repo(&operator, &profile).await;
        let branch = repo.branch("main").open().perform(&operator).await.unwrap();
        write(&branch, &operator, SCHEMA).await;
        let other = test_repo(&operator, &profile).await;
        let elsewhere = other
            .branch("main")
            .open()
            .perform(&operator)
            .await
            .unwrap();
        let foreign = write(&elsewhere, &operator, SCHEMA).await;

        assert_eq!(
            undo(&branch, &operator, &foreign).await,
            UndoOutcome::Refused(Refusal::NotFound)
        );
        assert_eq!(
            undo(&branch, &operator, "not a revision").await,
            UndoOutcome::Refused(Refusal::Malformed)
        );
    }
}
