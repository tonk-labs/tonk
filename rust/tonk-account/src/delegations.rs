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
//!
//! The other direction lives here too: [`adopt_account_upstream`] points a
//! profile's access branch at the account and pulls it, which is how a device
//! that holds the account grant regains access to everything the account
//! knows about.

use dialog_capability::{Fork, Provider};
use dialog_common::ConditionalSync;
use dialog_credentials::Ed25519Signer;
use dialog_effects::archive::{Get, Import, Put};
use dialog_effects::authority::{Attest, Identify};
use dialog_effects::blob::Write as BlobWrite;
use dialog_effects::memory::{Publish, Resolve};
use dialog_repository::{
    Branch, CommitError, PullError, RemoteSite, Revision, SetUpstreamError, Upstream,
    UpstreamBranch,
};
use dialog_ucan::UcanDelegation;
use dialog_ucan_core::subject::Subject as UcanSubject;
use dialog_ucan_core::{DelegationBuilder, DelegationChain};
use dialog_varsig::Did;

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

/// Point a profile's access branch at the account repository and pull.
///
/// This is the read half of the account being the durable home of
/// delegations. The write half ([`retain_space_delegation`]) puts a space's
/// authority into the account; this brings everything the account holds back
/// down to a device, so access is recovered by syncing a branch rather than
/// by fetching an artifact over HTTP.
///
/// Returns the revision the pull landed on, or `None` when the branch was
/// already up to date.
///
/// Two things have to be true before calling, and neither is checked here
/// because both are the caller's to arrange:
///
/// - The device must already hold the `account → profile` grant locally.
///   The pull is itself an authorized fetch, and the operator that authorizes
///   it resolves proofs only from what is already local — so a grant that
///   arrives *in* the pull cannot authorize the pull that carries it.
/// - `account` must be a REMOTE branch resolved against the account's DID
///   (`repository.remote(name).create(site).subject(account_did)`). A local
///   upstream resolves against the pulling branch's own subject, so it can
///   only ever name a sibling branch in the same repository — never another
///   repository's.
///
/// The upstream is set only when absent, so an established one is never
/// silently repointed; a branch already tracking something else is reported
/// rather than overwritten.
pub async fn adopt_account_upstream<Env>(
    access: &Branch,
    account: impl Into<UpstreamBranch>,
    env: &Env,
) -> Result<Option<Revision>, AdoptError>
where
    Env: Provider<Get>
        + Provider<Put>
        + Provider<Import>
        + Provider<Resolve>
        + Provider<Publish>
        + Provider<Identify>
        + Provider<Attest>
        + Provider<Fork<RemoteSite, Get>>
        + Provider<Fork<RemoteSite, Resolve>>
        + ConditionalSync
        + 'static,
{
    match access.upstream() {
        None => access.set_upstream(account.into()).perform(env).await?,
        Some(Upstream::Remote { .. }) => {}
        Some(_) => return Err(AdoptError::ForeignUpstream),
    }
    Ok(access.pull().perform(env).await?)
}

/// Why adopting the account as an access-branch upstream failed.
#[derive(Debug, thiserror::Error)]
pub enum AdoptError {
    /// The access branch already tracks something that is not a remote, so
    /// repointing it would silently change what the profile syncs against.
    #[error("access branch already tracks a non-remote upstream")]
    ForeignUpstream,
    /// Recording the upstream failed.
    #[error("failed to set the account upstream: {0}")]
    SetUpstream(#[from] SetUpstreamError),
    /// The pull itself failed.
    #[error("failed to pull the account: {0}")]
    Pull(#[from] PullError),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    wasm_bindgen_test_configure!(run_in_browser);

    /// The union edge is subject-open and command-open, matching the grant
    /// it mirrors. A narrower return edge would make the union asymmetric,
    /// and the asymmetry would only surface later as a proof that fails for
    /// no visible reason.
    #[dialog_common::test]
    async fn it_mints_a_symmetric_union_edge() {
        use dialog_varsig::Principal as _;

        let profile = Ed25519Signer::generate().await.unwrap();
        let account = Ed25519Signer::generate().await.unwrap();
        let union = mint_account_union(&profile, &account.did()).await.unwrap();

        assert_eq!(union.issuer(), &profile.did());
        assert_eq!(union.audience(), &account.did());
        assert!(
            union.subject().is_none(),
            "the union edge must be a powerline, not scoped to one space"
        );
        assert_eq!(union.proof_cids().len(), 1);
    }

    /// A branch already tracking a local upstream is reported rather than
    /// silently repointed at the account — repointing would change what the
    /// profile syncs against without anyone asking for it.
    #[dialog_common::test]
    fn it_names_a_foreign_upstream_rather_than_repointing_it() {
        let error = AdoptError::ForeignUpstream;
        assert!(
            error.to_string().contains("already tracks"),
            "the error must say what it refused to overwrite, got {error}"
        );
    }
}

/// Mint the profile's half of the account union: `profile → account`.
///
/// The account grants the profile a powerline at sign-in, which is what lets
/// a device act. This is the other direction, and it is what makes the
/// account's authority *complete*: with both edges retained, anything either
/// side can prove, the other can prove too — so a later device pulling the
/// account inherits what this profile holds, not merely what the account was
/// given directly.
///
/// Subject-open and command-open, matching the grant it mirrors: a narrower
/// return edge would silently make the union asymmetric, and the asymmetry
/// would only surface later as a proof that inexplicably fails.
pub async fn mint_account_union(
    profile: &Ed25519Signer,
    account: &Did,
) -> Result<DelegationChain, UnionError> {
    let delegation = DelegationBuilder::new()
        .issuer(profile.clone())
        .audience(account)
        .subject(UcanSubject::Any)
        .command(vec![])
        .try_build()
        .await
        .map_err(|error| UnionError::Mint(format!("{error:?}")))?;
    Ok(DelegationChain::new(delegation))
}

/// Why minting the profile's half of the union failed.
#[derive(Debug, thiserror::Error)]
pub enum UnionError {
    /// The delegation could not be built or signed.
    #[error("failed to mint the profile to account delegation: {0}")]
    Mint(String),
}
