//! Sealing a space's seed to the account — the CLI half of the custody
//! model the worker follows.
//!
//! A space's signing seed must survive this machine: any device on the
//! account can then re-issue the space after a passkey ceremony opens
//! the seed. The account publishes an X25519 recipient
//! ([`AccountSealedInbox`]) exactly so that sealing needs no passkey —
//! the CLI reads the public half from the account branch it already
//! syncs, seals, and records a [`SecretMessage`] and the
//! [`SecretPrincipal`] naming it beside the
//! directory entries it writes today. Only a ceremony holding the
//! account secret can ever open the row again.

use anyhow::{Context, Result};
use dialog_operator::{Operator, Profile};
use dialog_query::{Output as _, Query, Term};
use dialog_repository::Branch;
use dialog_storage::provider::storage::NativeSpace;
use dialog_varsig::Did;
use tonk_identity::sealed::RecipientKey;
use tonk_schema::{
    AccountSealedInbox, SecretMessage, SecretPrincipal, SeedKind, prelude::DidExt as _,
};
use zeroize::Zeroizing;

/// The account's published X25519 recipient, read from the account
/// branch. `None` when the account predates the encryption key — a
/// signed-in browser publishes it on its next visit.
pub async fn account_recipient(
    account: &Branch,
    root: &Did,
    operator: &Operator<NativeSpace>,
) -> Result<Option<Did>> {
    let rows: Vec<AccountSealedInbox> = account
        .query()
        .select(Query::<AccountSealedInbox> {
            this: Term::from(root.this()),
            address: Term::var("address"),
        })
        .perform(operator)
        .try_vec()
        .await
        .map_err(|error| {
            anyhow::anyhow!("failed to read the account sealed-inbox address: {error:?}")
        })?;
    rows.into_iter()
        .next()
        .map(|row| {
            row.address
                .0
                .to_string()
                .parse()
                .context("the published sealed-inbox address is not a DID")
        })
        .transpose()
}

/// Seal `seed` (deriving `subject`) to `recipient` and record the
/// custody row. The row is what the account's other devices recover the
/// space from, so it is pushed right away — but like the directory
/// record, a failed push warns rather than fails: the row is committed
/// locally and the next account push carries it.
pub async fn custody_space_seed(
    account: &Branch,
    subject: &Did,
    recipient: &Did,
    seed: &Zeroizing<[u8; 32]>,
    operator: &Operator<NativeSpace>,
) -> Result<()> {
    let key = RecipientKey::try_from(recipient).map_err(|error| {
        anyhow::anyhow!("the account sealed-inbox address is unusable: {error}")
    })?;
    let sealed = key
        .secret()
        .conceal(seed, subject)
        .map_err(|error| anyhow::anyhow!("failed to seal the space seed: {error}"))?
        .encode();
    // Two rows: the envelope, and the principal whose seed it carries.
    let message = SecretMessage::new(recipient, sealed);
    account
        .transaction()
        .assert(message.clone())
        .assert(SecretPrincipal::new(
            subject,
            SeedKind::Space,
            message.this(),
        ))
        .commit()
        .perform(operator)
        .await
        .context("failed to record the custodied seed")?;
    if let Err(error) = account.push().perform(operator).await {
        eprintln!("warning: custodied seed recorded locally; push failed: {error:#}");
    }
    Ok(())
}

/// The profile repository's `main` branch, opened locally — the same
/// branch the account mounts with a remote after sign-in, reachable
/// before any account exists. Where an unlinked device's custody rows
/// live, so they ride straight into the account when it arrives.
pub async fn open_local_account_branch(
    profile: &Profile,
    operator: &Operator<NativeSpace>,
) -> Result<Branch> {
    dialog_repository::Repository::from(profile)
        .branch(tonk_account::MAIN_BRANCH)
        .open()
        .perform(operator)
        .await
        .context("failed to open the local account branch")
}

/// Whether the account branch already holds a custody row for `subject`.
pub async fn has_custody(
    account: &Branch,
    subject: &Did,
    operator: &Operator<NativeSpace>,
) -> Result<bool> {
    let rows: Vec<SecretPrincipal> = account
        .query()
        .select(Query::<SecretPrincipal> {
            this: Term::from(subject.this()),
            kind: Term::var("kind"),
            seed: Term::var("seed"),
        })
        .perform(operator)
        .try_vec()
        .await
        .map_err(|error| anyhow::anyhow!("failed to read sealed principals: {error:?}"))?;
    Ok(!rows.is_empty())
}

/// The signing seed a locally created space's stored credential carries.
/// `None` for a space this machine only ever held a verifier for — a
/// joined or delegated space, whose seed is someone else's to custody.
pub async fn site_seed(site: &crate::site::TonkSite) -> Result<Option<Zeroizing<[u8; 32]>>> {
    let Some(signer) = site.repository.credential().signer() else {
        return Ok(None);
    };
    // `Signer` gains arms only when dialog-credentials is built with
    // another algorithm, which this crate never enables.
    let dialog_credentials::Signer::Ed25519(signer) = signer;
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
    let account = crate::account_state::open_account_branch_in(&site.profile, &operator, store)
        .await?
        .context("the account repository is not ready to custody spaces")?;
    let recipient = account_recipient(&account, account_root, &operator)
        .await?
        .context(
            "the account has not published its sealed-inbox address yet; \
             open /account in a signed-in browser once, then run `tonk account status`",
        )?;

    let prefix = crate::site::adopt_account_root_prefix_for(
        &site.profile,
        site.operator.local(),
        &subject,
        account_root,
    )
    .await?;
    crate::account_state::retain_space_delegation_in(&site.profile, &operator, store, &prefix)
        .await?;

    if has_custody(&account, &subject, &operator).await? {
        return Ok(SpaceRotation::Already);
    }
    custody_space_seed(&account, &subject, &recipient, &seed, &operator).await?;
    Ok(SpaceRotation::Moved)
}

/// Rotate everything the onboarding account custodies onto the
/// signed-in account, with the shared core the worker also runs —
/// `tonk_schema::custody::rotate` — then retire the onboarding account.
///
/// The re-issue half is this adapter's: a space seed mints its
/// `space -> root` directly, the prefix is persisted, and the chain is
/// retained into the account. An invite seed is left for a browser to
/// rotate — the CLI holds no membership re-issue path yet — and the
/// retirement waits for it.
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

    let storage = dialog_storage::provider::storage::Storage::<NativeSpace>::default();
    let profile = Profile::open(config.profile_name.clone())
        .at(config.profile_directory.clone())
        .perform(&storage)
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
    let new_recipient = account_recipient(&branch, &account_root, &operator)
        .await?
        .context(
            "the account has not published its sealed-inbox address yet; \
             open /account in a signed-in browser once, then run `tonk account status`",
        )?;
    let new_key =
        tonk_identity::sealed::RecipientKey::try_from(&new_recipient).map_err(|error| {
            anyhow::anyhow!("the account sealed-inbox address is unusable: {error}")
        })?;

    let outcome = tonk_schema::custody::rotate(
        &branch,
        secret.secret(),
        new_key,
        &operator,
        async |kind, signer, row, replacement| match kind {
            SeedKind::Space => {
                let subject = signer.did();
                let minter = dialog_repository::Repository::from(signer);
                let chain = minter
                    .access()
                    .claim(&minter)
                    .delegate(account_root.clone())
                    .perform(&operator)
                    .await
                    .map_err(|error| format!("{subject}: delegate: {error}"))?
                    .into_chain();
                let bytes = chain
                    .to_bytes()
                    .map_err(|error| format!("{subject}: serialize: {error}"))?;
                profile
                    .credential()
                    .site(tonk_account::prefix::space_root_site(
                        &subject,
                        &account_root,
                    ))
                    .save(bytes)
                    .perform(&operator)
                    .await
                    .map_err(|error| format!("{subject}: prefix: {error}"))?;
                // Every write for this row goes through one fresh
                // handle: the retention advances the branch, and the
                // replacement must commit on top of that, not on a
                // version held from before it.
                let commit_branch = open_local_account_branch(&profile, &operator)
                    .await
                    .map_err(|error| format!("{subject}: open: {error:#}"))?;
                tonk_account::delegations::retain_space_delegation(
                    &commit_branch,
                    &chain,
                    &operator,
                )
                .await
                .map_err(|error| format!("{subject}: retain: {error}"))?;
                commit_branch
                    .transaction()
                    .retract(row.clone())
                    .assert(replacement.message)
                    .assert(replacement.principal)
                    .commit()
                    .perform(&operator)
                    .await
                    .map(|_| ())
                    .map_err(|error| format!("{subject}: reseal commit: {error}"))
            }
            SeedKind::Invite => {
                Err("an invite seed rotates from a browser, not the CLI".to_string())
            }
        },
    )
    .await
    .map_err(|error| anyhow::anyhow!("rotation could not run: {error}"))?;

    if outcome.failures.is_empty() {
        crate::onboarding::retire(&profile, &operator).await?;
    }
    Ok(outcome.failures)
}
