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
    if sync {
        pull_before(site).await;
    }
    let outcome = eval::run_against_site(site, source, options).await?;
    if sync && outcome.committed {
        push_after(site).await;
    }
    if outcome.committed
        && let Err(error) = crate::account_spots::back_up_current(site).await
    {
        eprintln!("warning: account spot backup failed: {error:#}");
    }
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
async fn push_after(site: &TonkSite) {
    match sync::push(site).await {
        Ok(_) | Err(SyncError::UpstreamNotConfigured { .. }) => {}
        Err(err) => warn("push", &err),
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
