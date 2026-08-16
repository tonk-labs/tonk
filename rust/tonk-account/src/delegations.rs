//! Retaining space authority into the account repository.
//!
//! The account repository is the durable home of delegations: a device
//! regains access by pulling it, because the delegations are just facts in a
//! branch it syncs. That only works if something writes them there, which is
//! what this module is for.
//!
//! Both adapters mint the same `space → account-root` prefix when a space is
//! created — the worker in its repository route, the CLI in `site` — and both
//! reach the account branch as an ordinary [`Branch`]. So the policy lives
//! here once rather than being spelled out on each side, where the two could
//! drift into retaining different things.

use dialog_capability::{Fork, Provider};
use dialog_common::ConditionalSync;
use dialog_effects::archive::{Get, Import, Put};
use dialog_effects::authority::{Attest, Identify};
use dialog_effects::blob::Write as BlobWrite;
use dialog_effects::memory::{Publish, Resolve};
use dialog_repository::{Branch, CommitError, RemoteSite};
use dialog_ucan::UcanDelegation;
use dialog_ucan_core::DelegationChain;

/// Retain a `space → account-root` delegation into the account repository's
/// branch, so the authority it carries replicates with the account.
///
/// Returns whether anything was written. Retaining is content-addressed, so a
/// chain already present commits nothing and returns `false` — callers may run
/// this unconditionally on every space creation rather than checking first.
///
/// The caller decides how to treat a failure. Both adapters treat it as
/// best-effort: a space is fully usable the moment its delegation reaches the
/// profile's own access branch, and retaining into the account is what makes
/// it recoverable on the *next* device, so failing space creation over a
/// hidden system repository would trade a working space for a recoverable one.
pub async fn retain_space_delegation<Env>(
    account: &Branch,
    chain: &DelegationChain,
    env: &Env,
) -> Result<bool, CommitError>
where
    Env: Provider<Get>
        + Provider<Put>
        + Provider<Import>
        + Provider<Resolve>
        + Provider<Publish>
        + Provider<Identify>
        + Provider<Attest>
        + Provider<BlobWrite>
        + Provider<Fork<RemoteSite, Get>>
        + Provider<Fork<RemoteSite, Resolve>>
        + ConditionalSync
        + 'static,
{
    let retained = account
        .delegations()
        .retain(UcanDelegation(chain.clone()))
        .perform(env)
        .await?;
    Ok(!retained.is_empty())
}
