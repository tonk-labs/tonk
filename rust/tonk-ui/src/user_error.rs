//! User-facing recovery messages for account actions.

use crate::error::TonkUiError;

/// The account action whose failure needs to be explained.
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    allow(dead_code)
)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AccountAction {
    LoadAccount,
    LoadRegistration,
    CheckEmail,
    CreateAccount,
    LogIn,
    AddPasskey,
    ChangeDisplayName,
    ResendActivation,
    LoadDevices,
    LoadProfiles,
    LinkCli,
    SwitchProfile,
    SignOut,
    LoadDeletionPlan,
    DeleteAccount,
    DeleteSpace,
    RevokeDevice,
    FinishAccountBackup,
    ActivateAccount,
    WatchActivation,
    SaveInitialDisplayName,
    CopyInvite,
    FinishPreviousAction,
}

/// Present an internal/browser diagnostic at an account action boundary.
pub(crate) fn diagnostic(action: AccountAction, detail: &str) -> String {
    let original = detail.trim();
    if original
        == "Your account is ready, but this browser couldn't finish signing in. Log in to continue."
    {
        return original.to_owned();
    }
    if original
        == "Please verify your email using the verification link we sent before changing your display name."
    {
        return original.to_owned();
    }
    let detail = detail.to_ascii_lowercase();
    if detail.contains("an account already exists for this email address") {
        return "An account already exists for this email address. Log in instead.".to_owned();
    }
    if detail.contains("an account already exists for this passkey") {
        return "This passkey already belongs to an account. Log in instead.".to_owned();
    }
    if detail.contains("created before shared account state existed") {
        return "This older account needs a one-time update before it can be used on a new browser. Open account settings on a browser that is already signed in, finish setup there, then try again."
            .to_owned();
    }
    if action == AccountAction::DeleteAccount
        && detail.contains("passkey belongs to a different account")
    {
        return "This passkey belongs to a different account. Nothing was deleted. Open settings for the account you meant to delete and try again."
            .to_owned();
    }
    if action == AccountAction::DeleteAccount
        && detail.contains("no account passkey is registered on this device")
    {
        return "This device does not have an account passkey to confirm deletion. Nothing was deleted. Try from a device that has your account passkey."
            .to_owned();
    }
    let passkey_action = matches!(
        action,
        AccountAction::CreateAccount
            | AccountAction::LogIn
            | AccountAction::AddPasskey
            | AccountAction::LinkCli
            | AccountAction::DeleteAccount
            | AccountAction::FinishAccountBackup
    );

    if passkey_action
        && (detail.contains("notallowederror")
            || detail.contains("aborterror")
            || detail.contains("timed out or was not allowed")
            || detail.contains("passkey prompt was cancelled"))
    {
        return if action == AccountAction::DeleteAccount {
            "The passkey prompt was cancelled or timed out. Nothing was deleted. Try again and complete the prompt."
        } else {
            "The passkey prompt was cancelled or timed out. Try again and complete the prompt."
        }
        .to_owned();
    }
    if passkey_action
        && (detail.contains("no prf")
            || detail.contains("prf output")
            || detail.contains("cannot unlock custody"))
    {
        return "This passkey does not support the security feature Tonk needs. Try another passkey or device."
            .to_owned();
    }
    if passkey_action
        && (detail.contains("identity ceremon")
            || detail.contains("notsupportederror")
            || detail.contains("securityerror"))
    {
        return "Passkeys are not available in this browser right now. Reload the page, or try another supported browser or device."
            .to_owned();
    }

    fallback(action).to_owned()
}

/// Present a failed passkey ceremony.
///
/// A refusal the access service explained is answered from its REASON,
/// not from its wording: an unconfirmed email is a step someone can take,
/// and telling them to "check your connection" instead sends them to fix
/// something that is not broken. Anything the service did not refuse --
/// a dismissed prompt, an unsupported authenticator -- falls through to
/// the ordinary diagnostic.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) fn ceremony(
    action: AccountAction,
    error: &crate::custody_relay::CeremonyError,
) -> String {
    use tonk_identity::custody::CustodyDenial;

    match &error.denial {
        Some(CustodyDenial::AwaitingActivation) => {
            "Open the confirmation link in your email to finish signing in. You can leave this page open."
                .to_owned()
        }
        Some(CustodyDenial::Suspended(_)) => {
            "This account is suspended, so it cannot be used on this device. Contact support to restore it."
                .to_owned()
        }
        Some(CustodyDenial::NotProvisioned(_)) => {
            "This account is not set up for syncing yet. Finish creating it on the browser that holds its passkey, then try again."
                .to_owned()
        }
        Some(CustodyDenial::Other(reason)) => diagnostic(action, reason.as_str()),
        None => diagnostic(action, &error.message),
    }
}

/// Present a typed local API error at an account action boundary.
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    allow(dead_code)
)]
pub(crate) fn api(action: AccountAction, error: &TonkUiError) -> String {
    match error {
        TonkUiError::Account(message) => diagnostic(action, message),
        TonkUiError::Sync { message, .. } => message.clone(),
        TonkUiError::ApiError(_) | TonkUiError::Analyze { .. } => {
            diagnostic(action, &error.to_string())
        }
    }
}

fn fallback(action: AccountAction) -> &'static str {
    match action {
        AccountAction::LoadAccount => {
            "We couldn't load your account settings. Check your connection and reload the page."
        }
        AccountAction::LoadRegistration => {
            "We couldn't load account options. Check your connection, close this dialog, and try again."
        }
        AccountAction::CheckEmail => {
            "We couldn't check this email address. Check your connection and try again."
        }
        AccountAction::CreateAccount => {
            "We couldn't finish creating your account. Check your connection and try again. If you already approved the passkey, log in instead of creating another account."
        }
        AccountAction::LogIn => {
            "We couldn't finish logging you in. Check your connection and try again. If you already approved the passkey, reload settings before retrying."
        }
        AccountAction::AddPasskey => {
            "We couldn't finish adding the passkey. Reload settings before trying again."
        }
        AccountAction::ChangeDisplayName => {
            "We couldn't change your display name. Check your connection and try again."
        }
        AccountAction::ResendActivation => {
            "We couldn't send another verification email. Check your connection and try again."
        }
        AccountAction::LoadDevices => {
            "We couldn't load your connected devices. Check your connection and reload settings."
        }
        AccountAction::LoadProfiles => {
            "We couldn't load the accounts saved in this browser. Reload settings to try again."
        }
        AccountAction::LinkCli => {
            "The terminal didn't receive the account link. Return to the terminal and start login again."
        }
        AccountAction::SwitchProfile => {
            "We couldn't switch accounts. Reload settings and try again."
        }
        AccountAction::SignOut => {
            "We couldn't confirm that this browser signed out. Reload settings before trying again."
        }
        AccountAction::LoadDeletionPlan => {
            "We couldn't load the deletion review. Reload settings and try again."
        }
        AccountAction::DeleteAccount => {
            "We couldn't confirm whether your account was deleted. Reload settings before trying again."
        }
        AccountAction::DeleteSpace => {
            "We couldn't confirm whether the space was deleted. Reload settings before trying again."
        }
        AccountAction::RevokeDevice => {
            "We couldn't confirm whether device access was removed. Reload settings before trying again."
        }
        AccountAction::FinishAccountBackup => {
            "We couldn't finish enabling account backup. Reload settings and try again."
        }
        AccountAction::ActivateAccount => {
            "We couldn't activate your account. Check your connection and try the link again."
        }
        AccountAction::WatchActivation => {
            "We couldn't update this screen automatically. Open the verification link, then return to settings to continue."
        }
        AccountAction::SaveInitialDisplayName => {
            "Your account is ready, but we couldn't save that display name. You can change it later in settings."
        }
        AccountAction::CopyInvite => {
            "The invite link is ready, but this browser couldn't copy it. Share the space again to retry."
        }
        AccountAction::FinishPreviousAction => {
            "You're signed in, but we couldn't finish what you started. Return to the previous page and try again."
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_turns_passkey_diagnostics_into_specific_recovery_steps() {
        let cases = [
            (
                AccountAction::LogIn,
                "identity ceremony failed: NotAllowedError: The operation either timed out or was not allowed",
                "The passkey prompt was cancelled or timed out. Try again and complete the prompt.",
            ),
            (
                AccountAction::AddPasskey,
                "identity ceremony failed: the authenticator returned no PRF outputs",
                "This passkey does not support the security feature Tonk needs. Try another passkey or device.",
            ),
            (
                AccountAction::DeleteAccount,
                "identity ceremony verifyPasskey is unavailable",
                "Passkeys are not available in this browser right now. Reload the page, or try another supported browser or device.",
            ),
        ];

        for (action, detail, expected) in cases {
            assert_eq!(diagnostic(action, detail), expected);
        }
    }

    #[test]
    fn it_masks_transport_and_implementation_details_by_account_action() {
        let cases = [
            (
                AccountAction::LoadAccount,
                "Error from local API: GET /api/account/status returned 503 Service Unavailable: upstream timeout",
                "We couldn't load your account settings. Check your connection and reload the page.",
            ),
            (
                AccountAction::ChangeDisplayName,
                "Error from local API: POST /api/account/display-name returned 500 Internal Server Error",
                "We couldn't change your display name. Check your connection and try again.",
            ),
            (
                AccountAction::LoadDevices,
                "invalid invocation bytes: unexpected end of file",
                "We couldn't load your connected devices. Check your connection and reload settings.",
            ),
            (
                AccountAction::RevokeDevice,
                "Upstream service returned HTTP 403: invalid delegation",
                "We couldn't confirm whether device access was removed. Reload settings before trying again.",
            ),
            (
                AccountAction::DeleteAccount,
                "POST /api/account/delete returned 502 Bad Gateway: <html>proxy error</html>",
                "We couldn't confirm whether your account was deleted. Reload settings before trying again.",
            ),
            (
                AccountAction::CheckEmail,
                "profile transaction failed: invalid invocation bytes",
                "We couldn't check this email address. Check your connection and try again.",
            ),
            (
                AccountAction::LoadRegistration,
                "host did not write detail.subscription",
                "We couldn't load account options. Check your connection, close this dialog, and try again.",
            ),
            (
                AccountAction::WatchActivation,
                "activation subscription failed: UnboundVariable",
                "We couldn't update this screen automatically. Open the verification link, then return to settings to continue.",
            ),
            (
                AccountAction::SaveInitialDisplayName,
                "POST /api/profile returned 503 Service Unavailable",
                "Your account is ready, but we couldn't save that display name. You can change it later in settings.",
            ),
            (
                AccountAction::CopyInvite,
                "NotAllowedError: Write permission denied",
                "The invite link is ready, but this browser couldn't copy it. Share the space again to retry.",
            ),
        ];

        for (action, detail, expected) in cases {
            let message = diagnostic(action, detail);
            assert_eq!(message, expected);
            for technical in [
                "/api/",
                "HTTP",
                "Error from",
                "invocation",
                "delegation",
                "<html>",
            ] {
                assert!(
                    !message.contains(technical),
                    "message exposed {technical:?}: {message:?}"
                );
            }
        }
    }

    #[test]
    fn it_preserves_curated_account_errors_but_masks_raw_api_errors() {
        let curated = TonkUiError::Account(
            "An account already exists for this email address. Log in instead.".to_owned(),
        );
        assert_eq!(
            api(AccountAction::CreateAccount, &curated),
            "An account already exists for this email address. Log in instead."
        );

        let unclassified = TonkUiError::Account(
            "provider rejected malformed invocation for did:key:zTechnical".to_owned(),
        );
        assert_eq!(
            api(AccountAction::LogIn, &unclassified),
            "We couldn't finish logging you in. Check your connection and try again. If you already approved the passkey, reload settings before retrying."
        );

        let raw = TonkUiError::ApiError(
            "POST /accounts returned 500 Internal Server Error: database unavailable".to_owned(),
        );
        assert_eq!(
            api(AccountAction::CreateAccount, &raw),
            "We couldn't finish creating your account. Check your connection and try again. If you already approved the passkey, log in instead of creating another account."
        );
    }

    #[test]
    fn it_preserves_only_known_safe_flow_outcomes() {
        assert_eq!(
            diagnostic(
                AccountAction::CreateAccount,
                "the account service refused the ceremony: an account already exists for this email address",
            ),
            "An account already exists for this email address. Log in instead."
        );
        assert_eq!(
            diagnostic(
                AccountAction::LogIn,
                "Your account is ready, but this browser couldn't finish signing in. Log in to continue.",
            ),
            "Your account is ready, but this browser couldn't finish signing in. Log in to continue."
        );
        assert_eq!(
            diagnostic(
                AccountAction::DeleteAccount,
                "identity ceremony failed: NotAllowedError",
            ),
            "The passkey prompt was cancelled or timed out. Nothing was deleted. Try again and complete the prompt."
        );
        assert_eq!(
            diagnostic(
                AccountAction::FinishPreviousAction,
                "Error from local API: replay failed: invalid invocation",
            ),
            "You're signed in, but we couldn't finish what you started. Return to the previous page and try again."
        );
    }
}
