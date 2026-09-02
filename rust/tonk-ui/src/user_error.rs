//! User-facing recovery messages for account actions.

use crate::error::TonkUiError;
use tonk_analytics::account::{AccountOutcome, FailureKind, HttpStatusClass, ServiceCode};

/// One account failure projected into safe presentation and analytics fields.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AccountProblem {
    /// Caller-facing recovery copy.
    pub message: String,
    /// Closed terminal outcome.
    pub outcome: AccountOutcome,
}

impl AccountProblem {
    fn new(message: String, outcome: AccountOutcome) -> Self {
        Self { message, outcome }
    }
}

/// The account action whose failure needs to be explained.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AccountAction {
    OpenRegistration,
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
    SettleAccount,
    LoadAccountSpaces,
    PullAccountSpace,
    OpenAccountDeletion,
    OpenSpaceDeletion,
    SyncAccount,
}

/// Present an internal/browser diagnostic at an account action boundary.
pub(crate) fn diagnostic(action: AccountAction, detail: &str) -> String {
    problem_from_diagnostic(action, detail).message
}

/// Compatibility diagnostics have no typed evidence. Their prose may improve
/// recovery copy, but analytics always sees `unknown`.
pub(crate) fn problem_from_diagnostic(action: AccountAction, detail: &str) -> AccountProblem {
    AccountProblem::new(
        diagnostic_message(action, detail),
        AccountOutcome::retryable(FailureKind::Unknown),
    )
}

fn diagnostic_message(action: AccountAction, detail: &str) -> String {
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
#[allow(dead_code)]
pub(crate) fn ceremony(
    action: AccountAction,
    error: &crate::custody_relay::CeremonyError,
) -> String {
    ceremony_problem(action, error).message
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) fn ceremony_problem(
    action: AccountAction,
    error: &crate::custody_relay::CeremonyError,
) -> AccountProblem {
    use tonk_identity::custody::CustodyDenial;
    use tonk_identity::passkey::CeremonyRefusal;

    if error.update_safety {
        return AccountProblem::new(
            error.message.clone(),
            update_safety_outcome(error.retry_unsafe),
        );
    }

    match &error.denial {
        Some(CustodyDenial::AwaitingActivation) => AccountProblem::new(
            "Open the confirmation link in your email to finish signing in. You can leave this page open.".to_owned(),
            AccountOutcome::blocked(FailureKind::AwaitingActivation),
        ),
        Some(CustodyDenial::Suspended(_)) => AccountProblem::new(
            "This account is suspended, so it cannot be used on this device. Contact support to restore it.".to_owned(),
            AccountOutcome::blocked(FailureKind::Suspended),
        ),
        Some(CustodyDenial::NotProvisioned(_)) => AccountProblem::new(
            "This account is not set up for syncing yet. Finish creating it on the browser that holds its passkey, then try again.".to_owned(),
            AccountOutcome::blocked(FailureKind::NotProvisioned),
        ),
        Some(CustodyDenial::Other(_)) => AccountProblem::new(
            diagnostic_message(action, &error.message),
            AccountOutcome::terminal_failure(FailureKind::AccessDenied),
        ),
        None => {
            let outcome = match error.refusal.unwrap_or(CeremonyRefusal::Other) {
                CeremonyRefusal::NotAllowed => AccountOutcome::cancelled(),
                CeremonyRefusal::InvalidState => AccountOutcome::terminal_failure(FailureKind::CredentialExists),
                CeremonyRefusal::NotSupported => AccountOutcome::terminal_failure(FailureKind::PasskeyUnsupported),
                CeremonyRefusal::Security => AccountOutcome::terminal_failure(FailureKind::SecurityContext),
                CeremonyRefusal::NoPrf => AccountOutcome::terminal_failure(FailureKind::PrfUnsupported),
                CeremonyRefusal::Other => AccountOutcome::retryable(FailureKind::Unknown),
            };
            AccountProblem::new(diagnostic_message(action, &error.message), outcome)
        }
    }
}

#[cfg(any(all(target_arch = "wasm32", target_os = "unknown"), test))]
fn update_safety_outcome(retry_unsafe: bool) -> AccountOutcome {
    if retry_unsafe {
        AccountOutcome::unknown_commit(FailureKind::LocalState)
    } else {
        AccountOutcome::retryable(FailureKind::LocalState)
    }
}

/// Present a typed local API error at an account action boundary.
#[allow(dead_code)]
pub(crate) fn api(action: AccountAction, error: &TonkUiError) -> String {
    api_problem(action, error).message
}

pub(crate) fn api_problem(action: AccountAction, error: &TonkUiError) -> AccountProblem {
    use crate::error::AccountTransportKind;
    let message = match error {
        TonkUiError::AccountApi {
            service_code: Some(code),
            ..
        } if action == AccountAction::ChangeDisplayName
            && ServiceCode::from_wire(code) == ServiceCode::AccountStateUnavailable =>
        {
            "Please verify your email using the verification link we sent before changing your display name."
                .to_owned()
        }
        TonkUiError::Account(message) => diagnostic_message(action, message),
        TonkUiError::Sync { message, .. } => message.clone(),
        TonkUiError::AccountApi { diagnostic, .. } => diagnostic_message(action, diagnostic),
        TonkUiError::ApiError(_) | TonkUiError::Analyze { .. } => {
            diagnostic_message(action, &error.to_string())
        }
    };
    match error {
        TonkUiError::AccountApi {
            transport_kind,
            status,
            service_code,
            ..
        } => {
            let kind = match (transport_kind, status) {
                (AccountTransportKind::Network, _) => FailureKind::Network,
                (AccountTransportKind::Decode, None | Some(200..=299)) => {
                    FailureKind::InvalidResponse
                }
                (AccountTransportKind::Local, _) => FailureKind::LocalState,
                (AccountTransportKind::Http | AccountTransportKind::Decode, Some(404)) => {
                    FailureKind::NotFound
                }
                (AccountTransportKind::Http | AccountTransportKind::Decode, Some(409)) => {
                    FailureKind::Conflict
                }
                (AccountTransportKind::Http | AccountTransportKind::Decode, Some(429)) => {
                    FailureKind::RateLimited
                }
                (AccountTransportKind::Http | AccountTransportKind::Decode, Some(401 | 403)) => {
                    FailureKind::AccessDenied
                }
                (AccountTransportKind::Http | AccountTransportKind::Decode, Some(500..=599)) => {
                    FailureKind::ServiceUnavailable
                }
                _ => FailureKind::Unknown,
            };
            let mut outcome = if matches!(
                kind,
                FailureKind::Network | FailureKind::RateLimited | FailureKind::ServiceUnavailable
            ) {
                AccountOutcome::retryable(kind)
            } else {
                AccountOutcome::terminal_failure(kind)
            };
            if let Some(status) = status {
                if (400..500).contains(status) {
                    outcome = outcome.with_http_status_class(HttpStatusClass::ClientError);
                }
                if (500..600).contains(status) {
                    outcome = outcome.with_http_status_class(HttpStatusClass::ServerError);
                }
            }
            if let Some(code) = service_code {
                outcome = outcome.with_service_code(ServiceCode::from_wire(code));
            }
            AccountProblem::new(message, outcome)
        }
        TonkUiError::Sync { code, .. } => AccountProblem::new(
            message,
            AccountOutcome::terminal_failure(FailureKind::AccessDenied)
                .with_service_code(ServiceCode::from_wire(code)),
        ),
        _ => AccountProblem::new(message, AccountOutcome::retryable(FailureKind::Unknown)),
    }
}

/// Classify a dispatched destructive mutation. A missing or malformed reply
/// cannot prove that the service did not commit the mutation.
#[cfg(any(all(target_arch = "wasm32", target_os = "unknown"), test))]
pub(crate) fn mutation_api_problem(action: AccountAction, error: &TonkUiError) -> AccountProblem {
    let problem = api_problem(action, error);
    if matches!(
        action,
        AccountAction::DeleteAccount | AccountAction::DeleteSpace | AccountAction::RevokeDevice
    ) && matches!(
        error,
        TonkUiError::AccountApi {
            transport_kind: crate::error::AccountTransportKind::Network,
            ..
        } | TonkUiError::AccountApi {
            transport_kind: crate::error::AccountTransportKind::Decode,
            status: None | Some(200..=299) | Some(500..=599),
            ..
        } | TonkUiError::AccountApi {
            transport_kind: crate::error::AccountTransportKind::Http,
            status: None | Some(500..=599),
            ..
        }
    ) {
        let kind = problem
            .outcome
            .failure_kind()
            .unwrap_or(FailureKind::Unknown);
        return AccountProblem::new(problem.message, AccountOutcome::unknown_commit(kind));
    }
    problem
}

fn fallback(action: AccountAction) -> &'static str {
    match action {
        AccountAction::OpenRegistration => {
            "We couldn't open account options. Close settings and try again."
        }
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
        AccountAction::SettleAccount | AccountAction::SyncAccount => {
            "We couldn't finish syncing your account. Check your connection and try again."
        }
        AccountAction::LoadAccountSpaces => {
            "We couldn't load your account spaces. Check your connection and try again."
        }
        AccountAction::PullAccountSpace => {
            "We couldn't pull this account space. Check your connection and try again."
        }
        AccountAction::OpenAccountDeletion => {
            "We couldn't open account deletion. Reload settings and try again."
        }
        AccountAction::OpenSpaceDeletion => {
            "We couldn't open space deletion. Reload settings and try again."
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::AccountTransportKind;

    #[test]
    fn update_safety_distinguishes_safe_retry_from_an_unknown_commit() {
        assert_eq!(
            update_safety_outcome(false).result(),
            tonk_analytics::account::AccountResult::RetryableFailure
        );
        assert_eq!(
            update_safety_outcome(true).result(),
            tonk_analytics::account::AccountResult::UnknownCommit
        );
        assert_eq!(
            update_safety_outcome(true).failure_kind(),
            Some(FailureKind::LocalState)
        );
    }

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
    fn it_classifies_typed_account_boundaries_without_leaking_diagnostics() {
        let cases = [
            (AccountTransportKind::Network, None, FailureKind::Network),
            (
                AccountTransportKind::Decode,
                Some(200),
                FailureKind::InvalidResponse,
            ),
            (AccountTransportKind::Local, None, FailureKind::LocalState),
            (AccountTransportKind::Http, Some(404), FailureKind::NotFound),
            (AccountTransportKind::Http, Some(409), FailureKind::Conflict),
            (
                AccountTransportKind::Http,
                Some(429),
                FailureKind::RateLimited,
            ),
            (
                AccountTransportKind::Http,
                Some(503),
                FailureKind::ServiceUnavailable,
            ),
        ];
        for (transport_kind, status, expected) in cases {
            let error = TonkUiError::AccountApi {
                transport_kind,
                status,
                service_code: Some("upstream_timeout".to_owned()),
                diagnostic: "person@example.com did:key:zSensitive response body secret".to_owned(),
            };
            let problem = api_problem(AccountAction::LoadAccount, &error);
            assert_eq!(problem.outcome.failure_kind(), Some(expected));
            assert!(!problem.message.contains("person@example.com"));
            assert!(!problem.message.contains("did:key"));
            assert!(!problem.message.contains("response body"));
        }
    }

    #[test]
    fn destructive_mutations_distinguish_unknown_commit_from_proved_rejection() {
        for transport_kind in [AccountTransportKind::Network, AccountTransportKind::Decode] {
            let error = TonkUiError::AccountApi {
                transport_kind,
                status: (transport_kind == AccountTransportKind::Decode).then_some(200),
                service_code: None,
                diagnostic: "person@example.com response was lost after dispatch".to_owned(),
            };
            let problem = mutation_api_problem(AccountAction::DeleteAccount, &error);
            assert_eq!(
                problem.outcome.result(),
                tonk_analytics::account::AccountResult::UnknownCommit
            );
            assert!(!problem.message.contains("person@example.com"));
        }

        let rejected = TonkUiError::AccountApi {
            transport_kind: AccountTransportKind::Http,
            status: Some(409),
            service_code: Some("customer_active".to_owned()),
            diagnostic: "server proved the deletion did not commit".to_owned(),
        };
        let problem = mutation_api_problem(AccountAction::DeleteAccount, &rejected);
        assert_eq!(
            problem.outcome.result(),
            tonk_analytics::account::AccountResult::TerminalFailure
        );
        assert_eq!(problem.outcome.failure_kind(), Some(FailureKind::Conflict));

        let unreadable_server_response = TonkUiError::AccountApi {
            transport_kind: AccountTransportKind::Decode,
            status: Some(503),
            service_code: None,
            diagnostic: "server rejected the mutation with an unreadable body".to_owned(),
        };
        let problem =
            mutation_api_problem(AccountAction::DeleteAccount, &unreadable_server_response);
        assert_eq!(
            problem.outcome.result(),
            tonk_analytics::account::AccountResult::UnknownCommit
        );

        let server_failure = TonkUiError::AccountApi {
            transport_kind: AccountTransportKind::Http,
            status: Some(503),
            service_code: Some("upstream_unavailable".to_owned()),
            diagnostic: "service failed after accepting the mutation".to_owned(),
        };
        let problem = mutation_api_problem(AccountAction::RevokeDevice, &server_failure);
        assert_eq!(
            problem.outcome.result(),
            tonk_analytics::account::AccountResult::UnknownCommit
        );
    }

    #[test]
    fn unavailable_display_name_keeps_the_activation_recovery_copy() {
        let error = TonkUiError::AccountApi {
            transport_kind: AccountTransportKind::Http,
            status: Some(503),
            service_code: Some("account_state_unavailable".to_owned()),
            diagnostic: "raw upstream response".to_owned(),
        };
        let problem = api_problem(AccountAction::ChangeDisplayName, &error);
        assert!(problem.message.contains("verify your email"));
        assert!(!problem.message.contains("upstream"));
        assert_eq!(
            problem.outcome.failure_kind(),
            Some(FailureKind::ServiceUnavailable)
        );
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
