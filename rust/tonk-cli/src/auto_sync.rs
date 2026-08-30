//! Auto-sync — pull-before / push-after around `tonk eval`.
//!
//! When an upstream is configured, a mutating `eval` pulls the
//! upstream in before evaluating (so the write lands on top of the
//! latest shared state) and pushes the resulting commit back out
//! after. This makes data reach the remote without anyone running
//! `tonk push` by hand.
//!
//! Escape hatch: `--no-sync` on the command, or the `TONK_NO_SYNC`
//! environment variable. A branch with no upstream is a silent skip
//! (tonk's pre-auto-sync behavior). Sync failures are warnings on
//! stderr — the local write is already committed, so the command
//! still succeeds and the user can recover with the manual
//! `tonk pull` / `tonk push` flow.

use crate::eval::{self, Options, Outcome, Source};
use crate::site::TonkSite;
use crate::sync::{self, SyncError};

/// Environment variable that opts a tonk invocation out of
/// auto-sync, mirroring the `--no-sync` flag.
pub const NO_SYNC_ENV: &str = "TONK_NO_SYNC";

/// Whether auto-sync should run for this invocation.
///
/// Off when `--no-sync` was passed (`no_sync_flag`) or when
/// [`NO_SYNC_ENV`] is set to a value other than empty / `0` /
/// `false` / `no`.
pub fn enabled(no_sync_flag: bool) -> bool {
    !no_sync_flag && !env_value_opts_out(std::env::var(NO_SYNC_ENV).ok().as_deref())
}

/// Whether an opt-out environment variable's value actually opts out.
/// Empty, `0`, `false` and `no` mean "leave it on"; anything else
/// (including the bare `=1` everyone reaches for) means "turn it off".
///
/// Pure so the truthiness rule is testable without mutating the
/// process environment. Shared by every `TONK_NO_*` switch so they
/// all read the same way.
pub(crate) fn env_value_opts_out(value: Option<&str>) -> bool {
    match value {
        Some(raw) => !matches!(
            raw.trim().to_ascii_lowercase().as_str(),
            "" | "0" | "false" | "no"
        ),
        None => false,
    }
}

/// One local write's optional remote-sync lifecycle.
///
/// [`begin`](Self::begin) completes warning-only pull-before work. The caller
/// then performs and, when relevant, reports its local commit before
/// [`finish`](Self::finish) begins push-after. Keeping that ordering in the
/// type prevents a remote wait from hiding a durable local receipt.
pub struct WriteSession<'a> {
    site: &'a TonkSite,
    enabled: bool,
}

impl<'a> WriteSession<'a> {
    /// Begin a write session, performing best-effort pull-before when enabled.
    pub async fn begin(site: &'a TonkSite, enabled: bool) -> Self {
        if enabled {
            pull_before(site).await;
        }
        Self { site, enabled }
    }

    /// Finish a write after its local outcome has been made observable.
    ///
    /// A non-committing eval (including dry-run) skips both push and account
    /// directory work. Failures are retained in [`SyncReport`] so the caller
    /// can name the durable local action accurately.
    pub async fn finish(self, committed: bool) -> SyncReport {
        if !committed {
            return SyncReport::default();
        }

        let push = if self.enabled {
            push_after(self.site).await
        } else {
            None
        };
        let account_directory = crate::account_spaces::record_current(self.site)
            .await
            .err()
            .map(|error| format!("{error:#}"));
        SyncReport {
            push,
            account_directory,
        }
    }
}

/// Best-effort work that settled after a durable local commit.
#[derive(Debug, Default)]
pub struct SyncReport {
    push: Option<SyncError>,
    account_directory: Option<String>,
}

impl SyncReport {
    /// Render recovery for an eval whose receipt is already on stdout.
    ///
    /// Repeating asserted notation can create a second non-idempotent write,
    /// so recovery only retries remote delivery of the recorded revision.
    pub fn warn_eval(&self) {
        if let Some(error) = &self.push {
            eprintln!("warning: local eval was saved, but auto-sync push failed: {error}");
            eprintln!(
                "do not repeat the eval; inspect the saved revision with `tonk status`, \
                 then retry only remote delivery with `tonk push`"
            );
        }
        if let Some(error) = &self.account_directory {
            eprintln!(
                "warning: local eval was saved, but the account directory update failed: {error}"
            );
        }
    }

    /// Render recovery for a non-eval write after its local commit.
    pub fn warn_write(&self) {
        if let Some(error) = &self.push {
            eprintln!(
                "warning: the local write was saved, but auto-sync push failed: {error}; \
                 inspect with `tonk status`, then retry remote delivery with `tonk push`"
            );
        }
        if let Some(error) = &self.account_directory {
            eprintln!("warning: account directory update failed: {error}");
        }
    }
}

/// Evaluate `source` against `site`, syncing around the write when
/// `sync` is on.
///
/// Pull-before brings the local branch up to date; push-after sends
/// the new commit back. Both are no-ops when no upstream is
/// configured. The eval result is returned unchanged — sync wraps
/// it without altering success or output.
pub async fn run_eval(
    site: &TonkSite,
    source: Source,
    options: Options,
    sync: bool,
) -> Result<Outcome, eval::EvalError> {
    let session = WriteSession::begin(site, sync).await;
    let outcome = eval::run_against_site(site, source, options).await?;
    session.finish(outcome.committed).await.warn_eval();
    Ok(outcome)
}

/// Sync around any committing write, not just an eval.
///
/// [`run_eval`] wraps the one write path that had this from the start.
/// `blob add` commits its metadata transaction the same way and wants the
/// same wrapping, so the pull / push / record-in-the-account-directory
/// sequence lives here rather than being copied.
///
/// `write` is a future rather than a closure so it can borrow `site`
/// alongside the pull that precedes it; it is created before the pull but
/// only polled after it, so the write still sees the pulled branch.
pub async fn around_commit<T, E>(
    site: &TonkSite,
    sync: bool,
    write: impl std::future::Future<Output = Result<T, E>>,
) -> Result<T, E> {
    let session = WriteSession::begin(site, sync).await;
    let outcome = write.await?;
    session.finish(true).await.warn_write();
    Ok(outcome)
}

/// Pull the upstream into the local branch before a write. A
/// missing upstream is a silent skip; any other failure is a
/// warning — the command proceeds either way.
async fn pull_before(site: &TonkSite) {
    match sync::pull(site).await {
        Ok(_) | Err(SyncError::UpstreamNotConfigured { .. }) => {}
        Err(err) => warn("pull", &err),
    }
}

/// Push the local branch to its upstream after a write. A missing
/// upstream is a silent skip; any other failure is a warning — the
/// local write is already committed.
async fn push_after(site: &TonkSite) -> Option<SyncError> {
    match sync::push(site).await {
        Ok(_) | Err(SyncError::UpstreamNotConfigured { .. }) => None,
        Err(error) => Some(error),
    }
}

fn warn(op: &str, err: &SyncError) {
    eprintln!("warning: auto-sync {op} failed: {err}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_browser);

    #[dialog_common::test]
    fn it_treats_an_absent_env_value_as_opt_in() {
        assert!(!env_value_opts_out(None));
    }

    #[dialog_common::test]
    fn it_treats_falsey_env_values_as_opt_in() {
        assert!(!env_value_opts_out(Some("")));
        assert!(!env_value_opts_out(Some("0")));
        assert!(!env_value_opts_out(Some("false")));
        assert!(!env_value_opts_out(Some("no")));
        assert!(!env_value_opts_out(Some("  FALSE  ")));
    }

    #[dialog_common::test]
    fn it_treats_other_env_values_as_opt_out() {
        assert!(env_value_opts_out(Some("1")));
        assert!(env_value_opts_out(Some("true")));
        assert!(env_value_opts_out(Some("yes")));
    }

    #[dialog_common::test]
    fn it_disables_auto_sync_when_the_flag_is_set() {
        // The flag forces off regardless of the environment.
        assert!(!enabled(true));
    }
}
