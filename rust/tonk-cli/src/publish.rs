//! `tonk publish` — assert a directory of notation documents and
//! deliver the result to the upstream.
//!
//! The directory is the source of truth for what it asserts: every
//! `*.yaml` / `*.yml` file under it is evaluated in path order into a
//! single commit, and
//! files they reference with `!include` / `!include/blob` come along
//! as values and blobs. Re-publishing an unchanged directory commits
//! nothing, because asserting a fact that already holds leaves the
//! tree as it was and blobs are content-addressed.
//!
//! Publishing asserts; it never retracts. A file or field removed from
//! the directory leaves its facts in the space until something
//! retracts them, the same as deleting a document someone already ran
//! through `tonk eval`.
//!
//! Delivery is a push that integrates concurrent writers instead of
//! failing on them: when the upstream moved since the pull, it pulls
//! (merging both sides' facts) and pushes again, up to a bounded
//! number of attempts.

use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::ExitCode;
use crate::eval::{self, EvalError, Source};
use crate::site::TonkSite;
use crate::sync::{self, SyncError};
use tonk_schema::SyncState;

/// Per-invocation knobs for [`run`].
#[derive(Debug, Clone)]
pub struct Options {
    /// Push attempts before giving up on an upstream that keeps moving.
    /// Each attempt after the first pulls before it pushes.
    pub attempts: u32,
    /// Evaluate every document without committing or syncing.
    pub dry_run: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            attempts: 5,
            dry_run: false,
        }
    }
}

/// One evaluated document.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Document {
    /// Path relative to the published directory.
    pub path: PathBuf,
    /// Claims the document asserted. A claim that already held counts
    /// too; [`Outcome::changed`] says whether anything was new.
    pub claims: usize,
}

/// What [`run`] did.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Outcome {
    /// Every document, in the order it was evaluated.
    pub documents: Vec<Document>,
    /// Whether the documents moved the local branch: `false` when the
    /// space already held everything they assert.
    pub changed: bool,
    /// Whether the upstream advanced. `false` with no upstream, on a
    /// dry run, or when it already had everything.
    pub pushed: bool,
    /// Push attempts made, `0` when there was nothing to deliver.
    pub attempts: u32,
    /// The local branch's tree after publishing, if it has a revision.
    pub tree: Option<String>,
}

/// Failure modes for [`run`].
#[derive(Debug, Error)]
pub enum PublishError {
    /// The directory could not be read.
    #[error("cannot read {path}: {reason}")]
    Io {
        /// The path that failed.
        path: PathBuf,
        /// Why.
        reason: String,
    },
    /// Committing the documents failed.
    #[error("commit failed: {0}")]
    Commit(String),
    /// The directory holds no notation documents.
    #[error("no *.yaml or *.yml documents under {0}")]
    Empty(PathBuf),
    /// A document was rejected. Nothing was committed or pushed.
    #[error("{}", document_failure(path, source))]
    Document {
        /// The document that failed.
        path: PathBuf,
        /// What evaluation said.
        source: EvalError,
    },
    /// The upstream refused the delivery or could not be reached. The
    /// documents are committed locally; `tonk push` retries delivery.
    #[error("published locally, but delivery failed: {0}")]
    Sync(#[from] SyncError),
    /// Whether an upstream is configured could not be read.
    #[error("{0}")]
    Remote(#[from] crate::remote::RemoteError),
    /// The upstream kept moving for every attempt.
    #[error(
        "published locally, but the upstream moved on each of {0} attempts; \
         retry with `tonk push`"
    )]
    Contended(u32),
}

/// A parse error already names its document at each diagnostic; the
/// others are prefixed with it here.
fn document_failure(path: &Path, source: &EvalError) -> String {
    match source {
        EvalError::Parse(_) => source.to_string(),
        _ => format!("{}: {source}", path.display()),
    }
}

impl crate::Coded for PublishError {
    /// CLI exit code for this failure mode.
    fn exit_code(&self) -> ExitCode {
        match self {
            PublishError::Io { .. } | PublishError::Empty(_) => ExitCode::IoError,
            PublishError::Document { source, .. } => source.exit_code(),
            PublishError::Sync(error) => error.exit_code(),
            PublishError::Remote(error) => error.exit_code(),
            PublishError::Commit(_) | PublishError::Contended(_) => ExitCode::CommitError,
        }
    }
}

/// The notation documents under `root`, in the order [`run`] evaluates
/// them: sorted by relative path, so `00-schema.yaml` precedes the
/// documents that use its concepts. Hidden files and directories
/// (`.git`, `.github`) are skipped.
pub fn documents(root: &Path) -> Result<Vec<PathBuf>, PublishError> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), PublishError> {
        let io = |reason: std::io::Error| PublishError::Io {
            path: dir.to_path_buf(),
            reason: reason.to_string(),
        };
        for entry in std::fs::read_dir(dir).map_err(io)? {
            let entry = entry.map_err(io)?;
            if entry.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            let path = entry.path();
            if entry.file_type().map_err(io)?.is_dir() {
                walk(&path, out)?;
            } else if path
                .extension()
                .is_some_and(|extension| extension == "yaml" || extension == "yml")
            {
                out.push(path);
            }
        }
        Ok(())
    }

    let mut found = Vec::new();
    walk(root, &mut found)?;
    if found.is_empty() {
        return Err(PublishError::Empty(root.to_path_buf()));
    }
    found.sort();
    Ok(found)
}

/// Evaluate every document under `root` against `site` in one
/// transaction, commit it, then deliver.
///
/// Pulls first when an upstream is configured, so the documents are
/// asserted on top of what the space already holds and an unchanged
/// directory is recognised as one. That pull is best effort, like the
/// one before any other write: a stale base costs a retry at delivery,
/// where a failure that persists is reported.
///
/// All documents share one transaction, so a later document sees what
/// an earlier one declared, and a rejected document leaves nothing
/// committed. A dry run evaluates the same way and drops the
/// transaction.
pub async fn run(site: &TonkSite, root: &Path, options: Options) -> Result<Outcome, PublishError> {
    let paths = documents(root)?;
    let upstream = !options.dry_run && crate::remote::upstream_configured(site).await?;
    if upstream && let Err(error) = sync::pull(site).await {
        eprintln!("warning: pull before publishing failed: {error}");
    }

    let session = site.branch().await.map_err(|error| PublishError::Io {
        path: site.root.clone(),
        reason: format!("acquire branch: {error}"),
    })?;
    let branch = session.handle();
    let before = branch.revision();

    let mut txn = branch.transaction();
    let mut writes = false;
    let mut documents = Vec::with_capacity(paths.len());
    for path in paths {
        let applied = eval::evaluate_into(site, txn, Source::File(path.clone()))
            .await
            .map_err(|source| PublishError::Document {
                path: path.clone(),
                source,
            })?;
        txn = applied.txn;
        writes |= applied.writes;
        documents.push(Document {
            path: path.strip_prefix(root).unwrap_or(&path).to_path_buf(),
            claims: applied.claims,
        });
    }

    let after = if options.dry_run || !writes {
        before.clone()
    } else {
        let revision = txn
            .commit()
            .publish()
            .perform(&site.operator)
            .await
            .map_err(|error| PublishError::Commit(error.to_string()))?;
        session.poll(&site.operator).await;
        Some(revision)
    };

    let (pushed, attempts) = if upstream {
        deliver(site, options.attempts.max(1)).await?
    } else {
        (false, 0)
    };

    Ok(Outcome {
        documents,
        changed: before != after,
        pushed,
        attempts,
        tree: after.map(|revision| revision.tree.to_string()),
    })
}

/// Push, and on a moved upstream pull and push again, up to `attempts`
/// times. Returns whether the upstream advanced and the attempts made.
///
/// Asks first whether the upstream already holds the local head, since
/// a push reports success either way: an unchanged directory then makes
/// no push attempt at all.
pub async fn deliver(site: &TonkSite, attempts: u32) -> Result<(bool, u32), PublishError> {
    if let Ok(SyncState::Synced | SyncState::Behind) = sync::status(site).await {
        return Ok((false, 0));
    }
    for attempt in 1..=attempts {
        if attempt > 1 {
            sync::pull(site).await?;
        }
        match sync::push(site).await {
            Ok(outcome) => return Ok((outcome.advanced, attempt)),
            Err(SyncError::NonFastForward) => continue,
            Err(error) => return Err(error.into()),
        }
    }
    Err(PublishError::Contended(attempts))
}
