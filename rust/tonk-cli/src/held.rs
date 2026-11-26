//! A write refused because another process holds the cell it writes.
//!
//! Dialog's filesystem provider guards each cell with a lock file naming
//! its holder. Writers inside one process take turns at it; a holder in
//! another process (the desktop app beside this CLI, say) is not waited
//! for: the write is refused at once as
//! [`Rejection::Unavailable`], with nothing written, and is safe to try
//! again as it was. This module recognises that refusal by its type and
//! retries the write a bounded number of times with a short backoff, so
//! two tonk processes writing one space at once cost a short wait rather
//! than a failed command.

use std::future::Future;
use std::time::Duration;

use dialog_effects::Rejection;
use dialog_effects::memory::MemoryError;
use dialog_repository::PublishError;
use dialog_storage::DialogStorageError;

/// How many times a write refused as unavailable is tried in all,
/// the first try included.
pub const ATTEMPTS: u32 = 5;

/// The wait before the first retry; each later retry waits twice as
/// long as the one before. With [`ATTEMPTS`] at 5 the waits come to
/// 100 + 200 + 400 + 800 ms, so the whole bound is about a second and a
/// half.
pub const FIRST_BACKOFF: Duration = Duration::from_millis(100);

/// The reason a write was refused as temporarily unavailable, when the
/// chain under `error` carries such a refusal.
///
/// Dialog's wrappers hold their [`Rejection`] in `#[error(transparent)]`
/// variants, which forward `source()` past the rejection itself, so a
/// source-walk alone never reaches it: every wrapper a write's refusal
/// travels in has to be named here. A wrapper not listed degrades to
/// "not a held cell", which is the failure as it was reported before.
pub fn unavailable<'a>(error: &'a (dyn std::error::Error + 'static)) -> Option<&'a Rejection> {
    let mut current = Some(error);
    while let Some(error) = current {
        if let Some(rejection) = rejection(error)
            && matches!(rejection, Rejection::Unavailable { .. })
        {
            return Some(rejection);
        }
        current = error.source();
    }
    None
}

/// The rejection one link of the chain carries, if it is one.
fn rejection<'a>(error: &'a (dyn std::error::Error + 'static)) -> Option<&'a Rejection> {
    if let Some(rejection) = error.downcast_ref::<Rejection>() {
        return Some(rejection);
    }
    if let Some(MemoryError::Rejected(rejection)) = error.downcast_ref::<MemoryError>() {
        return Some(rejection);
    }
    if let Some(PublishError::Rejected(rejection)) = error.downcast_ref::<PublishError>() {
        return Some(rejection);
    }
    if let Some(DialogStorageError::Rejected(rejection)) =
        error.downcast_ref::<DialogStorageError>()
    {
        return Some(rejection);
    }
    None
}

/// What a write that stayed refused after every attempt reports.
pub fn message(rejection: &Rejection) -> String {
    format!(
        "another tonk process is writing to this space ({rejection}); \
         nothing was written, try again once it finishes"
    )
}

/// Run `attempt` until it succeeds, fails with an error `is_held` does
/// not recognise, or has been tried [`ATTEMPTS`] times. Each attempt is
/// passed its number, from 1. The last error is returned as it is.
///
/// `attempt` must be safe to run again after a held refusal, which a
/// refusal guarantees of the write it refused: nothing was written.
pub async fn retry<T, E, F, Fut>(is_held: impl Fn(&E) -> bool, mut attempt: F) -> Result<T, E>
where
    F: FnMut(u32) -> Fut,
    Fut: Future<Output = Result<T, E>>,
{
    let mut backoff = FIRST_BACKOFF;
    let mut number = 1;
    loop {
        match attempt(number).await {
            Err(error) if is_held(&error) && number < ATTEMPTS => {
                tokio::time::sleep(backoff).await;
                backoff *= 2;
                number += 1;
            }
            outcome => return outcome,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dialog_repository::CommitError;

    #[dialog_common::test]
    fn it_reads_a_held_cell_out_of_a_commit_error() {
        let held = Rejection::unavailable("Concurrent write in progress (lock /x.lock held)");
        let error = CommitError::Publish(PublishError::from(MemoryError::Rejected(held.clone())));
        assert_eq!(unavailable(&error), Some(&held));
    }

    #[dialog_common::test]
    fn it_does_not_read_other_failures_as_a_held_cell() {
        let unsupported = Rejection::Unsupported {
            reason: "watch".to_owned(),
        };
        let error = CommitError::Publish(PublishError::Rejected(unsupported));
        assert_eq!(unavailable(&error), None);
        let error = CommitError::Publish(PublishError::Storage("disk full".to_owned()));
        assert_eq!(unavailable(&error), None);
    }

    #[dialog_common::test]
    async fn it_stops_retrying_at_the_bound() {
        let mut tries = 0;
        let outcome: Result<(), &str> = retry(
            |_| true,
            |_| {
                tries += 1;
                async { Err("held") }
            },
        )
        .await;
        assert_eq!(outcome, Err("held"));
        assert_eq!(tries, ATTEMPTS);
    }

    #[dialog_common::test]
    async fn it_does_not_retry_an_error_it_does_not_recognise() {
        let mut tries = 0;
        let outcome: Result<(), &str> = retry(
            |error: &&str| *error == "held",
            |_| {
                tries += 1;
                async { Err("other") }
            },
        )
        .await;
        assert_eq!(outcome, Err("other"));
        assert_eq!(tries, 1);
    }
}
