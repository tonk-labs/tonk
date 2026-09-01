//! Customer registration against the access service.
//!
//! The access service serves the account's sync remote, so its origin
//! comes from the attached repository descriptor. Registration is per
//! account and web-only — the browser enrolls during its passkey
//! ceremonies, which is where the account-signed deposits come from —
//! so this module only probes registration state and provisions spaces,
//! which needs no passkey.

use anyhow::{Context, Result, bail};
use dialog_varsig::Did;
use tonk_account::customer::{Receipt, RegistrationError};
use url::Url;

use dialog_operator::Profile;

/// The access service origin for this profile's account: the attached
/// repository descriptor's remote, with its `/ucan/` path stripped.
pub async fn access_origin(profile: &Profile) -> Result<Option<Url>> {
    let store = crate::space::SpaceStore::open().context("failed to locate account state")?;
    access_origin_in(profile, &store).await
}

/// Resolve the access-service origin for one explicit native profile store.
pub async fn access_origin_in(
    profile: &Profile,
    store: &crate::space::SpaceStore,
) -> Result<Option<Url>> {
    let operator = crate::account_state::credential_operator_for_store(profile, store).await?;
    let Some(provider) = crate::account::stored_provider_in(profile, &operator, store).await?
    else {
        return Ok(None);
    };
    let Some(remote) = provider.remote() else {
        return Ok(None);
    };
    let remote: Url = remote.parse().context("the account remote is not a URL")?;
    let origin: Url = remote
        .origin()
        .ascii_serialization()
        .parse()
        .context("the account remote has no origin")?;
    Ok(Some(origin))
}

/// The service's registration state for `customer`, `None` when the
/// service does not know it.
pub async fn probe(origin: &Url, customer: &Did) -> Result<Option<Receipt>> {
    let endpoint = origin.join(&format!("customer/{customer}"))?;
    let response = reqwest::Client::new()
        .get(endpoint)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .context("failed to reach the access service")?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        bail!("access service rejected the customer probe ({status}): {text}");
    }
    Ok(Some(response.json().await.context(
        "access service answered an unreadable customer state",
    )?))
}

/// Provision `consumer` with the access service under this profile's
/// account, depositing `consent` — the space's powerline to the account.
/// A consumer another customer already provides is left alone: the space
/// exists and works locally either way.
pub async fn provision(
    profile: &Profile,
    consumer: &Did,
    consent: &dialog_ucan_core::DelegationChain,
) -> Result<()> {
    let store = crate::space::SpaceStore::open().context("failed to locate account state")?;
    provision_in(profile, &store, consumer, consent).await
}

/// Provision a space under one explicit account profile.
pub async fn provision_in(
    profile: &Profile,
    store: &crate::space::SpaceStore,
    consumer: &Did,
    consent: &dialog_ucan_core::DelegationChain,
) -> Result<()> {
    let connection = crate::account::optional_connection_in(profile, store)
        .await?
        .context("no active account; run `tonk account login`")?;
    let origin = access_origin_in(profile, store)
        .await?
        .context("the account has no repository descriptor to locate its service by")?;
    let body = tonk_identity::request::build_provider_add_invocation(
        profile.signer().signer().clone(),
        &connection.link,
        consumer,
        consent,
        None,
    )
    .await?;
    let response = reqwest::Client::new()
        .post(origin.join("ucan/")?)
        .header("content-type", "application/cbor")
        .body(body)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .context("failed to reach the access service")?;
    if response.status().is_success() {
        return Ok(());
    }
    let status = response.status();
    let refusal: serde_json::Value = response.json().await.unwrap_or_default();
    match serde_json::from_value::<RegistrationError>(refusal["error"].clone()) {
        Ok(RegistrationError::ConsumerProvided) => Ok(()),
        Ok(refusal) => bail!("the access service refused provisioning: {refusal}"),
        Err(_) => bail!("access service rejected provisioning ({status})"),
    }
}

/// The service's view of this profile's account: `Ok(None)` when the
/// profile is not linked or its account has no located service, and an
/// inner `None` when the service does not know the customer.
pub async fn registration_state(profile: &Profile) -> Result<Option<Option<Receipt>>> {
    let store = crate::space::SpaceStore::open().context("failed to locate account state")?;
    registration_state_in(profile, &store).await
}

/// The service's view of one profile from its explicit native store.
pub async fn registration_state_in(
    profile: &Profile,
    store: &crate::space::SpaceStore,
) -> Result<Option<Option<Receipt>>> {
    let Some(origin) = access_origin_in(profile, store).await? else {
        return Ok(None);
    };
    let Some(connection) = crate::account::optional_connection_in(profile, store).await? else {
        return Ok(None);
    };
    Ok(Some(probe(&origin, &connection.root_did).await?))
}
