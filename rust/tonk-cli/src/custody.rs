//! Sealing a space's seed to the account — the CLI half of the custody
//! model the worker follows.
//!
//! A space's signing seed must survive this machine: any device on the
//! account can then re-issue the space after a passkey ceremony opens
//! the seed. The account publishes an X25519 recipient
//! ([`AccountEncryptionKey`]) exactly so that sealing needs no passkey —
//! the CLI reads the public half from the account branch it already
//! syncs, seals, and records a [`CustodiedSeed`] row beside the
//! directory entries it writes today. Only a ceremony holding the
//! account secret can ever open the row again.

use anyhow::{Context, Result};
use dialog_operator::Operator;
use dialog_query::{Output as _, Query, Term};
use dialog_repository::Branch;
use dialog_storage::provider::storage::NativeSpace;
use dialog_varsig::Did;
use tonk_identity::sealed::RecipientKey;
use tonk_schema::{AccountEncryptionKey, CustodiedSeed, SeedKind, prelude::DidExt as _};
use zeroize::Zeroizing;

/// The account's published X25519 recipient, read from the account
/// branch. `None` when the account predates the encryption key — a
/// signed-in browser publishes it on its next visit.
pub async fn account_recipient(
    account: &Branch,
    root: &Did,
    operator: &Operator<NativeSpace>,
) -> Result<Option<Did>> {
    let rows: Vec<AccountEncryptionKey> = account
        .query()
        .select(Query::<AccountEncryptionKey> {
            this: Term::from(root.this()),
            key: Term::var("key"),
        })
        .perform(operator)
        .try_vec()
        .await
        .map_err(|error| anyhow::anyhow!("failed to read the account encryption key: {error:?}"))?;
    rows.into_iter()
        .next()
        .map(|row| {
            row.key
                .0
                .to_string()
                .parse()
                .context("the published account encryption key is not a DID")
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
    let key = RecipientKey::from_did(recipient)
        .map_err(|error| anyhow::anyhow!("the account encryption key is unusable: {error}"))?;
    let sealed = key
        .seal(seed, subject)
        .map_err(|error| anyhow::anyhow!("failed to seal the space seed: {error}"))?
        .encode();
    account
        .transaction()
        .assert(CustodiedSeed::new(
            subject.clone(),
            SeedKind::Space,
            recipient.clone(),
            sealed,
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

/// Whether the account branch already holds a custody row for `subject`.
pub async fn has_custody(
    account: &Branch,
    subject: &Did,
    operator: &Operator<NativeSpace>,
) -> Result<bool> {
    let rows: Vec<CustodiedSeed> = account
        .query()
        .select(Query::<CustodiedSeed> {
            this: Term::var("this"),
            subject: Term::from(tonk_schema::domain::custody::Subject(subject.this())),
            kind: Term::var("kind"),
            recipient: Term::var("recipient"),
            sealed: Term::var("sealed"),
        })
        .perform(operator)
        .try_vec()
        .await
        .map_err(|error| anyhow::anyhow!("failed to read custodied seeds: {error:?}"))?;
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

/// One space's outcome under [`accredit_local_spaces`].
#[derive(Debug)]
pub enum Accreditation {
    /// Authority and custody now reach the account.
    Moved,
    /// Nothing to do: the account already holds this space's custody.
    Already,
    /// Skipped, with the reason: no signer (a joined space), or a
    /// founder row naming a different account.
    Skipped(String),
}

/// Move custody of every registered local space to the signed-in
/// account — the CLI's accreditation, mirroring what the browser's
/// rotation does at sign-in. Authority (`space → root`, retained into
/// the account) and the sealed seed move; hosting does not: a space
/// gains its remote and provisioning through `tonk space link`.
///
/// Best-effort per space: a failure is reported and the rest continue,
/// and running again converges.
pub async fn accredit_local_spaces(
    store: &crate::space::SpaceStore,
    config: &crate::site::SiteConfig,
) -> Result<Vec<(String, Accreditation)>> {
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
                    Accreditation::Skipped(format!("could not open: {error:#}")),
                ));
                continue;
            }
        };
        match accredit_site(&site, &account_root, store).await {
            Ok(outcome) => outcomes.push((name.clone(), outcome)),
            Err(error) => outcomes.push((
                name.clone(),
                Accreditation::Skipped(format!("failed: {error:#}")),
            )),
        }
    }
    Ok(outcomes)
}

async fn accredit_site(
    site: &crate::site::TonkSite,
    account_root: &Did,
    store: &crate::space::SpaceStore,
) -> Result<Accreditation> {
    let subject = site.repository.did();
    // Ownership is the space's own answer: a founder row naming another
    // account is final — a synced space stays with its owner.
    let roster = crate::inventory::read_roster(site).await?;
    if let Some(founder) = roster.founder()
        && founder.did != account_root.to_string()
    {
        return Ok(Accreditation::Skipped(format!("owned by {}", founder.did)));
    }
    let Some(seed) = site_seed(site).await? else {
        return Ok(Accreditation::Skipped(
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
            "the account has not published its encryption key yet; \
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
        return Ok(Accreditation::Already);
    }
    custody_space_seed(&account, &subject, &recipient, &seed, &operator).await?;
    Ok(Accreditation::Moved)
}
