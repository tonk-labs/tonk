//! The account ceremonies the registration cluster runs.
//!
//! Lifted out of the account panel this page no longer has: the cluster
//! is the one surface that creates an account or signs into one, and
//! every other passkey-gated act is a command the worker runs with the
//! handles the custody relay hands it.

/// Sign in with an existing passkey.
///
/// The address is looked up before this runs: sending someone who
/// already has an account through creation leaves an orphan passkey in
/// their authenticator and fails at the end, because saving a new root
/// over an existing one is what creation does.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) fn begin_login_ceremony(
    narrate: impl Fn(&str),
) -> Result<crate::custody_relay::Mediation, crate::custody_relay::CeremonyError> {
    use crate::custody_relay::CeremonyError;

    narrate("Waiting for your passkey…");
    // One assertion, and the worker does the rest: it opens the account
    // from its custody cell, mints this browser's delegation, records
    // the root and submits the link. The page holds no key material.
    let provider = proposed_remote().map_err(CeremonyError::said)?;
    narrate("Linking this browser…");
    crate::custody_relay::begin(
        "usePasskey",
        tonk_worker_api::CustodyIntent::Login(tonk_worker_api::DeviceLink {
            device_name: crate::device_name::current(),
            endpoint: proposed_remote().map_err(CeremonyError::said)?,
            provider,
        }),
    )
}

/// Run the account-creation ceremony.
///
/// The page's whole part is one passkey ceremony. The worker generates
/// the account secret, seals it under the new passkey's KEK, records
/// the root, signs the creation request and enrolls, so no key material
/// exists in this document at any point.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) async fn run_account_ceremony(
    email: &str,
    narrate: impl Fn(&str),
) -> Result<(), crate::custody_relay::CeremonyError> {
    narrate("Waiting for your passkey…");
    let provider = proposed_remote()?;
    narrate("Creating your account…");
    crate::custody_relay::mediate_now(
        "createPasskey",
        tonk_worker_api::CustodyIntent::CreateAccount(tonk_worker_api::AccountCreation {
            email: email.to_owned(),
            device_name: crate::device_name::current(),
            remote: proposed_remote()?,
            provider,
            created_on: Some(crate::device_name::current()),
        }),
    )
    .await
    .map_err(|error| error.message)?;
    crate::analytics::identify().await;
    tonk_analytics::web::capture_account_created();
    Ok(())
}

/// The account repository remote this browser proposes: its own origin's
/// `/ucan/` endpoint. Only a ceremony ever signs one; the stored descriptor is
/// always the service-selected winner.
pub(crate) fn proposed_remote() -> Result<String, String> {
    web_sys::window()
        .and_then(|window| window.location().origin().ok())
        .map(|origin| format!("{}/ucan/", origin.trim_end_matches('/')))
        .ok_or_else(|| "window origin is unavailable".to_string())
}
