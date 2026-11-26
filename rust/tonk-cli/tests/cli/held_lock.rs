//! A commit refused because another process holds the branch's head cell.
//!
//! Dialog's filesystem provider guards a cell with the lock file
//! `{cell}.lock`, holding `{pid} {token}`. Writers of one process take
//! turns; a holder those turns know nothing of, such as another process,
//! is refused at once as `Rejection::Unavailable` with nothing written.
//! These tests write that lock file as a holder writes it, naming a
//! running process (this one, whose turns do not know the file, so it
//! reads as held exactly as another process's would), and check that the
//! CLI recognises the refusal by its type, retries it, and gives up with a
//! clear message when the lock stays held.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use dialog_effects::Rejection;
use dialog_repository::{CommitError, PublishError, Revision};
use tonk_cli::eval::EvalError;
use tonk_cli::{Coded, ExitCode, held};
use tonk_schema::RepositoryName;
use tonk_schema::prelude::DidExt as _;

use crate::common::{ATTRIBUTE_DECL, NOTE_ATTRIBUTE_DECL, TestSite};

/// The file holding the active branch's head: the `revision` cell of the
/// branch's space, the only one under `root` for a fresh site once the
/// profile's own directory (`_profile`, see `isolated_config`) is left out.
fn head_cell(root: &Path, branch: &str) -> Result<PathBuf> {
    fn walk(dir: &Path, found: &mut Vec<PathBuf>) -> std::io::Result<()> {
        for entry in std::fs::read_dir(dir)? {
            let path = entry?.path();
            if path.is_dir() {
                if path.file_name().is_some_and(|name| name == "_profile") {
                    continue;
                }
                walk(&path, found)?;
            } else if path.file_name().is_some_and(|name| name == "revision") {
                found.push(path);
            }
        }
        Ok(())
    }
    let mut found = Vec::new();
    walk(root, &mut found)?;
    found.retain(|path| path.to_string_lossy().contains(branch));
    match found.as_slice() {
        [cell] => Ok(cell.clone()),
        other => anyhow::bail!("expected one head cell for {branch}, found {other:?}"),
    }
}

/// Hold `cell` as a writer in another process would: a lock file beside it
/// naming a running process and a token this process's turns never took.
fn hold(cell: &Path) -> Result<(PathBuf, String)> {
    let mut lock = cell.as_os_str().to_owned();
    lock.push(".lock");
    let lock = PathBuf::from(lock);
    let holder = format!("{} 7", std::process::id());
    std::fs::write(&lock, &holder)?;
    Ok((lock, holder))
}

/// The head the branch holds on disk now, read past the handle's cache.
async fn head_on_disk(test: &TestSite) -> Result<Option<Revision>> {
    let session = test.site.branch().await?;
    let branch = session.handle();
    branch.refresh(&test.site.operator).await?;
    Ok(branch.revision())
}

/// Commit a write while the head cell is held: the refusal is the typed
/// `Rejection::Unavailable`, the CLI's recogniser reads it out of the
/// commit error, nothing is written, and the retry, once the holder lets
/// go, lands the write.
#[dialog_common::test]
async fn it_retries_a_commit_refused_by_a_held_head() -> Result<()> {
    let test = TestSite::new().await?;
    test.eval_inline(ATTRIBUTE_DECL).await?;
    let session = test.site.branch().await?;
    let name = session.handle().name().to_owned();
    drop(session);
    let subject = test.site.repository.did();
    let cell = head_cell(&test.parent, &name)?;
    let before = head_on_disk(&test).await?;
    let (lock, holder) = hold(&cell)?;

    let mut attempts = Vec::new();
    let landed = held::retry(
        |error: &CommitError| held::unavailable(error).is_some(),
        |attempt| {
            attempts.push(attempt);
            let test = &test;
            let lock = lock.clone();
            let holder = holder.clone();
            let subject = subject.clone();
            let before = before.clone();
            async move {
                let session = test.site.branch().await.expect("branch");
                let branch = session.handle();
                let outcome = branch
                    .transaction()
                    .assert(RepositoryName {
                        this: subject.this(),
                        name: tonk_schema::domain::repo::Name("held".to_owned()),
                    })
                    .commit()
                    .publish()
                    .perform(&test.site.operator)
                    .await;
                if attempt == 1 {
                    let refused = outcome.as_ref().err();
                    assert!(
                        matches!(
                            refused,
                            Some(CommitError::Publish(PublishError::Rejected(
                                Rejection::Unavailable { .. }
                            )))
                        ),
                        "a held head refuses the commit as unavailable: {outcome:?}"
                    );
                    assert_eq!(
                        head_on_disk(test).await.expect("head"),
                        before,
                        "the refused commit wrote nothing"
                    );
                    assert_eq!(
                        std::fs::read_to_string(&lock).expect("lock"),
                        holder,
                        "the holder's lock file is left as it was"
                    );
                    // The holder lets go before the retry.
                    std::fs::remove_file(&lock).expect("release");
                }
                outcome
            }
        },
    )
    .await
    .context("the retry lands once the holder lets go")?;

    assert_eq!(attempts, vec![1, 2], "one refusal, one retry");
    let after = head_on_disk(&test).await?;
    assert_ne!(after, before, "the retry wrote the commit");
    assert_eq!(after.as_ref(), Some(&landed));
    Ok(())
}

/// `tonk eval` against a head that stays held retries up to the bound,
/// then reports that another tonk process is writing, with the I/O exit
/// code, and writes nothing.
#[dialog_common::test]
async fn it_gives_up_on_a_head_that_stays_held() -> Result<()> {
    let test = TestSite::new().await?;
    test.eval_inline(ATTRIBUTE_DECL).await?;
    let session = test.site.branch().await?;
    let name = session.handle().name().to_owned();
    drop(session);
    let cell = head_cell(&test.parent, &name)?;
    let before = head_on_disk(&test).await?;
    let (lock, holder) = hold(&cell)?;

    let started = std::time::Instant::now();
    let outcome = test.eval_inline(NOTE_ATTRIBUTE_DECL).await;
    let waited = started.elapsed();

    let Err(error @ EvalError::Unavailable(message)) = &outcome else {
        anyhow::bail!("expected the commit to give up as unavailable, got {outcome:?}");
    };
    assert!(
        message.contains("another tonk process is writing to this space")
            && message.contains("try again"),
        "the message says who holds the space and what to do: {message}"
    );
    assert_eq!(error.exit_code(), ExitCode::IoError);
    // The backoff between the attempts is 100 + 200 + 400 + 800 ms.
    assert!(
        waited >= std::time::Duration::from_millis(1500),
        "every attempt was made before giving up: waited {waited:?}"
    );
    assert_eq!(head_on_disk(&test).await?, before, "nothing was written");
    assert_eq!(std::fs::read_to_string(&lock)?, holder);
    std::fs::remove_file(&lock)?;
    Ok(())
}
