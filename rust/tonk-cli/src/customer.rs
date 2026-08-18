//! Customer registration against the access service.
//!
//! The access service serves the account's sync remote, so its origin
//! comes from the attached repository descriptor and its identity from
//! `/.well-known/tonk` there. Registration is per account, not per
//! device: a linked device usually finds the customer already active,
//! and enrollment exists for first registration and for a service whose
//! control state was reset.

use anyhow::{Context, Result, bail};
use dialog_varsig::Did;
use tonk_account::customer::{CustomerStatus, Receipt, RegistrationError};
use url::Url;

use dialog_operator::Profile;

/// What `ensure_registered` found and did.
#[derive(Debug)]
pub struct RegistrationReport {
    /// The customer's registration state after this call.
    pub status: CustomerStatus,
    /// The email an activation link was (re)sent to, when one was.
    pub emailed: Option<String>,
}

/// The access service origin for this profile's account: the attached
/// repository descriptor's remote, with its `/ucan/` path stripped.
pub async fn access_origin(profile: &Profile) -> Result<Option<Url>> {
    let Some(provider) = crate::account::stored_provider(profile).await? else {
        return Ok(None);
    };
    let Some(descriptor) = provider.descriptor() else {
        return Ok(None);
    };
    let origin: Url = descriptor
        .remote()
        .origin()
        .ascii_serialization()
        .parse()
        .context("the account repository remote has no origin")?;
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

/// The access service's signing DID, from its deployment configuration.
async fn service_did(origin: &Url) -> Result<Did> {
    let endpoint = origin.join(".well-known/tonk")?;
    let config: serde_json::Value = reqwest::Client::new()
        .get(endpoint)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .context("failed to read the deployment configuration")?
        .error_for_status()
        .context("the deployment configuration is unavailable")?
        .json()
        .await
        .context("the deployment configuration is invalid")?;
    config["serviceDid"]
        .as_str()
        .context("this deployment publishes no service identity, so enrollment cannot address it")?
        .parse()
        .map_err(|error| anyhow::anyhow!("deployment service DID is invalid: {error:?}"))
}

/// Enroll this profile's account as a customer, sending the activation
/// link to `email`, or to the account's recorded address when none is
/// given. Idempotent: re-enrolling while registered resends the link,
/// and an already-active customer answers as active.
pub async fn enroll(profile: &Profile, email: Option<String>) -> Result<Receipt> {
    let connection = crate::account::optional_connection(profile)
        .await?
        .context("no active account; run `tonk account link`")?;
    let origin = access_origin(profile)
        .await?
        .context("the account has no repository descriptor to locate its service by")?;
    let email = match email {
        Some(email) => email,
        None => account_email(profile, &connection).await?,
    };
    let service = service_did(&origin).await?;
    let body = tonk_identity::request::build_enroll_invocation(
        profile.signer().signer().clone(),
        &connection.link,
        &service,
        &email,
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
        return response
            .json()
            .await
            .context("enrollment answered an unreadable receipt");
    }
    let status = response.status();
    let refusal: serde_json::Value = response.json().await.unwrap_or_default();
    match serde_json::from_value::<RegistrationError>(refusal["error"].clone()) {
        // An already-active customer is the outcome enrollment exists to
        // reach: answer it as one rather than surfacing a refusal.
        Ok(RegistrationError::CustomerActive) => Ok(Receipt {
            customer: connection.root_did.clone(),
            status: CustomerStatus::Active,
        }),
        Ok(refusal) => bail!("the access service refused enrollment: {refusal}"),
        Err(_) => bail!("access service rejected enrollment ({status})"),
    }
}

/// The service's view of this profile's account: `Ok(None)` when the
/// profile is not linked or its account has no located service, and an
/// inner `None` when the service does not know the customer.
pub async fn registration_state(profile: &Profile) -> Result<Option<Option<Receipt>>> {
    let Some(origin) = access_origin(profile).await? else {
        return Ok(None);
    };
    let Some(connection) = crate::account::optional_connection(profile).await? else {
        return Ok(None);
    };
    Ok(Some(probe(&origin, &connection.root_did).await?))
}

/// Probe the service and enroll when it does not know the account.
/// Answers what state the customer is in and whether an activation
/// email went out.
pub async fn ensure_registered(
    profile: &Profile,
    email: Option<String>,
) -> Result<RegistrationReport> {
    let connection = crate::account::optional_connection(profile)
        .await?
        .context("no active account; run `tonk account link`")?;
    let origin = access_origin(profile)
        .await?
        .context("the account has no repository descriptor to locate its service by")?;
    if email.is_none()
        && let Some(receipt) = probe(&origin, &connection.root_did).await?
        && receipt.status != CustomerStatus::Registered
    {
        return Ok(RegistrationReport {
            status: receipt.status,
            emailed: None,
        });
    }
    // Unknown to the service, awaiting activation (re-enrolling resends
    // the link), or the caller named an address: enroll.
    let recorded = match email {
        Some(email) => email,
        None => account_email(profile, &connection).await?,
    };
    let receipt = enroll(profile, Some(recorded.clone())).await?;
    let emailed = (receipt.status == CustomerStatus::Registered).then_some(recorded);
    Ok(RegistrationReport {
        status: receipt.status,
        emailed,
    })
}

/// The account's recorded email, from the account service.
async fn account_email(
    profile: &Profile,
    connection: &crate::account::AccountConnection,
) -> Result<String> {
    let response = connection
        .signed_post(
            profile,
            "account/summary",
            vec!["account".into(), "summary".into()],
            std::collections::BTreeMap::new(),
        )
        .await?;
    if !response.status().is_success() {
        let status = response.status();
        bail!("account service rejected the summary request ({status})");
    }
    let summary: serde_json::Value = response
        .json()
        .await
        .context("account service answered an unreadable summary")?;
    summary["email"]
        .as_str()
        .map(str::to_owned)
        .context("the account records no email address to enroll with")
}
