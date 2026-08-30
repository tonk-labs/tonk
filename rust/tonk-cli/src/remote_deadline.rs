//! Shared deadline boundary for repository operations that can wait on a remote.

use std::future::Future;
use std::time::Duration;

use thiserror::Error;

/// Environment override for remote-operation deadlines.
pub const ENV: &str = "TONK_REMOTE_TIMEOUT_SECONDS";

/// Default deadline for one remote operation.
pub const DEFAULT_SECONDS: u64 = 120;

/// Largest accepted deadline for one remote operation.
pub const MAX_SECONDS: u64 = 300;

/// A remote operation exceeded its configured deadline.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error(
    "{operation} to {target} timed out after {seconds} seconds; \
     the remote outcome may be unknown"
)]
pub struct Timeout {
    /// Human-readable operation phase, such as `push main`.
    pub operation: String,
    /// Human-readable remote target, such as `origin/main`.
    pub target: String,
    /// Whole seconds allowed for the operation.
    pub seconds: u64,
}

/// The deadline environment override could not be used safely.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("TONK_REMOTE_TIMEOUT_SECONDS must be a positive whole number from 1 to 300 seconds")]
pub struct Configuration;

/// Result boundary around a remote future.
#[derive(Debug, PartialEq, Eq)]
pub enum RunError<E> {
    /// The remote future completed with its own error.
    Operation(E),
    /// The deadline expired before the future completed.
    Timeout(Timeout),
    /// The process-level deadline override was invalid.
    Configuration(Configuration),
}

/// Run one remote future with the process's configured deadline.
///
/// An absent override uses [`DEFAULT_SECONDS`]. Invalid values fail before the
/// remote future is polled, so a typo cannot silently disable or lengthen the
/// safety boundary.
pub async fn run<T, E>(
    operation: impl Into<String>,
    target: impl Into<String>,
    future: impl Future<Output = Result<T, E>>,
) -> Result<T, RunError<E>> {
    let duration =
        configured_duration(std::env::var_os(ENV).as_deref()).map_err(RunError::Configuration)?;
    run_with(duration, operation, target, future).await
}

/// Run one remote future with an explicitly injected duration.
///
/// Expiry drops the in-flight future. That cancels the local wait, but cannot
/// prove whether a remote accepted the request before its response was lost;
/// callers must retain the typed [`Timeout`] and report that uncertainty.
pub async fn run_with<T, E>(
    duration: Duration,
    operation: impl Into<String>,
    target: impl Into<String>,
    future: impl Future<Output = Result<T, E>>,
) -> Result<T, RunError<E>> {
    let operation = operation.into();
    let target = target.into();
    match tokio::time::timeout(duration, future).await {
        Ok(result) => result.map_err(RunError::Operation),
        Err(_) => Err(RunError::Timeout(Timeout {
            operation,
            target,
            seconds: duration.as_secs(),
        })),
    }
}

fn configured_duration(value: Option<&std::ffi::OsStr>) -> Result<Duration, Configuration> {
    match value {
        None => Ok(Duration::from_secs(DEFAULT_SECONDS)),
        Some(value) => {
            let seconds = value
                .to_str()
                .and_then(|value| value.parse::<u64>().ok())
                .filter(|seconds| (1..=MAX_SECONDS).contains(seconds))
                .ok_or(Configuration)?;
            Ok(Duration::from_secs(seconds))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::pin::Pin;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::task::{Context, Poll};

    use super::*;

    struct NeverCompletes {
        dropped: Arc<AtomicBool>,
    }

    impl Future for NeverCompletes {
        type Output = Result<(), &'static str>;

        fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
            Poll::Pending
        }
    }

    impl Drop for NeverCompletes {
        fn drop(&mut self) {
            self.dropped.store(true, Ordering::SeqCst);
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_never_resolving_remote_is_dropped_at_the_injected_deadline() {
        let dropped = Arc::new(AtomicBool::new(false));
        let deadline = Duration::from_secs(7);
        let bounded = run_with(
            deadline,
            "push main",
            "origin/main",
            NeverCompletes {
                dropped: dropped.clone(),
            },
        );

        let result = tokio::time::timeout(Duration::from_secs(8), bounded)
            .await
            .expect("the remote wrapper must finish at its injected deadline")
            .expect_err("a pending remote must time out");

        assert_eq!(
            result,
            RunError::Timeout(Timeout {
                operation: "push main".to_owned(),
                target: "origin/main".to_owned(),
                seconds: 7,
            })
        );
        assert!(
            dropped.load(Ordering::SeqCst),
            "timing out must cancel and drop the in-flight remote future"
        );
    }

    #[dialog_common::test]
    fn the_default_and_both_documented_bounds_are_accepted() {
        assert_eq!(
            configured_duration(None),
            Ok(Duration::from_secs(DEFAULT_SECONDS))
        );
        assert_eq!(
            configured_duration(Some(OsStr::new("1"))),
            Ok(Duration::from_secs(1))
        );
        assert_eq!(
            configured_duration(Some(OsStr::new("300"))),
            Ok(Duration::from_secs(MAX_SECONDS))
        );
    }

    #[dialog_common::test]
    fn invalid_overrides_fail_without_mutating_process_environment() {
        for value in ["", "0", "-1", "1.5", "five", "301", " 2 "] {
            assert_eq!(
                configured_duration(Some(OsStr::new(value))),
                Err(Configuration),
                "{value:?} must not weaken the configured boundary"
            );
        }
    }
}
