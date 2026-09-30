//! A space's key in the account's custody — the CLI half of the custody
//! the worker follows, which tonk and dialog share.
//!
//! A space's key must survive this machine: the account holds it sealed,
//! and the profile keeps a copy of its own. Creating a space puts it in
//! that custody; a space from before, or one linked from elsewhere, is
//! adopted into it. Seeds an earlier release sealed in tonk's own custody
//! rows move into it when the onboarding account's secret is here to open
//! them.

use anyhow::{Context, Result};
use dialog_credentials::key::ExtractableKey as _;
use dialog_credentials::{Ed25519Signer, Extractable};
use dialog_peer::{Peer, Session};
use dialog_repository::Branch;
use dialog_storage::provider::storage::NativeSpace;
use dialog_varsig::Did;
use tonk_schema::SeedKind;
use zeroize::Zeroizing;

/// Whether the account `profile` acts for holds the key of `subject`.
pub async fn held_for_account(profile: &Peer<NativeSpace>, subject: &Did) -> Result<bool> {
    let account = profile
        .authority()
        .await
        .context("the profile acts for no account")?;
    let held = dialog_repository::secrets::held_principal(profile.state(), subject, profile)
        .await
        .map_err(|error| anyhow::anyhow!("failed to read the held keys: {error}"))?;
    Ok(held.is_some_and(|held| held.to == account))
}

/// Take the space whose key `seed` is into the custody of the account
/// `profile` acts for.
pub async fn adopt_space_seed(
    profile: &Peer<NativeSpace>,
    seed: &Zeroizing<[u8; 32]>,
) -> Result<()> {
    let key = <Ed25519Signer<Extractable>>::import(&**seed)
        .await
        .map_err(|error| anyhow::anyhow!("failed to import the space key: {error:?}"))?;
    profile
        .adopt_principal(SeedKind::Space.held(), key)
        .await
        .context("failed to take the space into the account's custody")
}

/// Move what the onboarding account sealed in tonk's own custody rows on
/// `branch` into the custody of the account `profile` acts for, retracting
/// the rows once each key is held there. `space` runs for each space moved,
/// before its rows go; an invite's membership is re-issued to the account
/// with the chain proving it. Answers the principals that stayed, with why.
pub async fn migrate_onboarding<F, Fut>(
    profile: &Peer<NativeSpace>,
    operator: &Peer<NativeSpace, Session>,
    branch: &Branch,
    secret: &tonk_identity::envelope::AccountSecret,
    space: F,
) -> Result<Vec<(Did, String)>>
where
    F: Fn(Ed25519Signer) -> Fut + Copy + dialog_common::ConditionalSend,
    Fut: std::future::Future<Output = Result<(), String>> + dialog_common::ConditionalSend,
{
    let outcome = tonk_schema::custody::migrate(
        branch,
        secret.secret(),
        operator,
        |kind, key, principal, message| async move {
            let signer = Ed25519Signer::import(
                &key.export()
                    .await
                    .map_err(|error| format!("{error:?}"))
                    .map(|export| match export {
                        dialog_credentials::KeyExport::Extractable(seed) => seed,
                    })?
                    .as_slice()
                    .try_into()
                    .map_err(|_| "the space key is not a key".to_string())?,
            )
            .await
            .map_err(|error| format!("{error:?}"))?;
            profile
                .adopt_principal(kind.held(), key)
                .await
                .map_err(|error| format!("custody: {error}"))?;
            if kind == SeedKind::Space {
                space(signer).await?;
            }
            // One fresh handle: taking custody advanced the branch.
            open_local_account_branch(profile, operator)
                .await
                .map_err(|error| format!("open: {error:#}"))?
                .transaction()
                .retract(principal)
                .retract(message)
                .commit()
                .publish()
                .perform(operator)
                .await
                .map(|_| ())
                .map_err(|error| format!("retract the custody rows: {error}"))
        },
    )
    .await
    .map_err(|error| anyhow::anyhow!("the old custody could not be read: {error}"))?;
    Ok(outcome.failures)
}

/// The profile repository's `main` branch, opened locally — the same
/// branch the account mounts with a remote after sign-in, reachable
/// before any account exists. Where an unlinked device's custody rows
/// live, so they ride straight into the account when it arrives.
pub async fn open_local_account_branch(
    profile: &Peer<NativeSpace>,
    operator: &Peer<NativeSpace, Session>,
) -> Result<Branch> {
    dialog_repository::Repository::from(profile.credential().clone())
        .branch(tonk_account::MAIN_BRANCH)
        .open()
        .perform(operator)
        .await
        .context("failed to open the local account branch")
}

/// The signing seed a locally created space's stored credential carries.
/// `None` for a space this machine only ever held a verifier for — a
/// joined or delegated space, whose seed is someone else's to custody.
pub async fn site_seed(site: &crate::site::TonkSite) -> Result<Option<Zeroizing<[u8; 32]>>> {
    if site.is_scoped() {
        return Ok(None);
    }
    let Some(signer) = crate::site::space_signer(site).await? else {
        return Ok(None);
    };
    let exported = signer
        .export()
        .await
        .map_err(|error| anyhow::anyhow!("failed to export the space signer: {error:?}"))?;
    let dialog_credentials::KeyExport::Extractable(bytes) = exported;
    let seed: [u8; 32] = bytes
        .as_slice()
        .try_into()
        .context("the exported space seed is not 32 bytes")?;
    Ok(Some(Zeroizing::new(seed)))
}

/// One space's outcome under [`rotate_local_spaces`].
#[derive(Debug)]
pub enum SpaceRotation {
    /// Authority and custody now reach the account.
    Moved,
    /// Nothing to do: the account already holds this space's custody.
    Already,
    /// Skipped, with the reason: no signer (a joined space), or a
    /// founder row naming a different account.
    Skipped(String),
}

/// Move custody of every registered local space to the signed-in
/// account. Two passes share the work: [`rotate_from_onboarding`] runs
/// the shared core over seeds the onboarding account sealed, and this
/// walk covers spaces from before the onboarding account existed, whose
/// only seed source is the signer credential this machine stored.
/// Authority (`space → root`, retained into the account) and the sealed
/// seed move; hosting does not: a space gains its remote and
/// provisioning through `tonk space link`.
///
/// Best-effort per space: a failure is reported and the rest continue,
/// and running again converges.
pub async fn rotate_local_spaces(
    store: &crate::space::SpaceStore,
    config: &crate::site::SiteConfig,
) -> Result<Vec<(String, SpaceRotation)>> {
    let registry = store.load()?;
    let Some(account) = registry.account.clone() else {
        return Ok(Vec::new());
    };
    let account_root: Did = account
        .root
        .parse()
        .context("the signed-in account root is invalid")?;

    let mut outcomes = Vec::new();
    for (name, entry) in &registry.spaces {
        let mut site_config = config.clone();
        site_config.require_account = false;
        let site = match crate::site::TonkSite::open_with(&entry.site, site_config).await {
            Ok(site) => site,
            Err(error) => {
                outcomes.push((
                    name.clone(),
                    SpaceRotation::Skipped(format!("could not open: {error:#}")),
                ));
                continue;
            }
        };
        match rotate_site(&site, &account_root, store).await {
            Ok(outcome) => outcomes.push((name.clone(), outcome)),
            Err(error) => outcomes.push((
                name.clone(),
                SpaceRotation::Skipped(format!("failed: {error:#}")),
            )),
        }
    }
    Ok(outcomes)
}

async fn rotate_site(
    site: &crate::site::TonkSite,
    account_root: &Did,
    store: &crate::space::SpaceStore,
) -> Result<SpaceRotation> {
    if site.is_scoped() {
        return Ok(SpaceRotation::Skipped(
            "scoped connection access does not transfer ownership".to_string(),
        ));
    }
    let subject = site.repository.did();
    // Ownership is the space's own answer: a founder row naming another
    // account is final — a synced space stays with its owner.
    let roster = crate::inventory::read_roster(site).await?;
    if let Some(founder) = roster.founder()
        && founder.did != account_root.to_string()
    {
        return Ok(SpaceRotation::Skipped(format!("owned by {}", founder.did)));
    }
    let Some(seed) = site_seed(site).await? else {
        return Ok(SpaceRotation::Skipped(
            "no local signer (a joined space)".to_string(),
        ));
    };

    let operator =
        crate::account_state::credential_operator_for_store(&site.profile, store).await?;

    let prefix = crate::site::adopt_account_root_prefix_for(
        &site.profile,
        site.operator.local(),
        &subject,
        account_root,
    )
    .await?;
    crate::account_state::retain_space_delegation_in(&site.profile, &operator, store, &prefix)
        .await?;

    if held_for_account(&site.profile, &subject).await? {
        return Ok(SpaceRotation::Already);
    }
    adopt_space_seed(&site.profile, &seed).await?;
    Ok(SpaceRotation::Moved)
}

/// Move everything the onboarding account custodies into the custody of
/// the signed-in account, then retire the onboarding account.
///
/// Each space's key is taken into the account's custody, and its
/// `space -> root` is minted, the prefix persisted and the chain retained
/// into the account. An invite's key is held for the account too, and its
/// membership re-issued with the chain proving it. The retirement waits
/// for every principal to move.
pub async fn rotate_from_onboarding(
    store: &crate::space::SpaceStore,
    config: &crate::site::SiteConfig,
) -> Result<Vec<(Did, String)>> {
    use dialog_varsig::Principal as _;

    let registry = store.load()?;
    let Some(account) = registry.account.clone() else {
        return Ok(Vec::new());
    };
    let account_root: Did = account
        .root
        .parse()
        .context("the signed-in account root is invalid")?;
    let profile = crate::site::open_profile(
        config.profile_name.clone(),
        config.profile_directory.clone(),
        true,
    )
    .await
    .with_context(|| format!("failed to open profile '{}'", config.profile_name))?;
    let operator = crate::account_state::credential_operator_for_store(&profile, store).await?;
    let Some(secret) = crate::onboarding::read_if_openable_in(&profile, &operator).await? else {
        return Ok(Vec::new());
    };

    let branch =
        match crate::account_state::open_account_branch_in(&profile, &operator, store).await? {
            Some(branch) => branch,
            None => open_local_account_branch(&profile, &operator).await?,
        };
    // Bound as references OUTSIDE the closure: each `async move` block
    // it produces captures a copy of the reference, so the closure can run
    // once per seed without consuming the values.
    let operator_ref = &operator;
    let profile_ref = &profile;
    let account_root_ref = &account_root;
    let failures = migrate_onboarding(
        &profile,
        &operator,
        &branch,
        &secret,
        |signer: Ed25519Signer| async move {
            let subject = signer.did();
            let minter = dialog_repository::Repository::from(signer);
            let chain = minter
                .access()
                .claim(&minter)
                .delegate(account_root_ref.clone())
                .perform(operator_ref)
                .await
                .map_err(|error| format!("{subject}: delegate: {error}"))?
                .into_chain();
            let bytes = chain
                .to_bytes()
                .map_err(|error| format!("{subject}: serialize: {error}"))?;
            profile_ref
                .secrets()
                .site(tonk_account::prefix::space_root_site(
                    &subject,
                    account_root_ref,
                ))
                .save(bytes)
                .perform(profile_ref)
                .await
                .map_err(|error| format!("{subject}: prefix: {error}"))?;
            let commit_branch = open_local_account_branch(profile_ref, operator_ref)
                .await
                .map_err(|error| format!("{subject}: open: {error:#}"))?;
            tonk_account::delegations::retain_space_delegation(&commit_branch, &chain, operator_ref)
                .await
                .map(|_| ())
                .map_err(|error| format!("{subject}: retain: {error}"))
        },
    )
    .await?;

    if failures.is_empty() {
        crate::onboarding::retire(&profile, &operator).await?;
    }
    Ok(failures)
}
