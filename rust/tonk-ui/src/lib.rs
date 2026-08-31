#![warn(missing_docs)]
//! Tonk UI web application.
//!
//! This crate provides the web-based user interface for Tonk.

/// Top-document account creation and self-link element.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub mod account;
/// Customer activation page reached from the activation email.
pub mod activate;

/// The account panel's registration row, kept live by a subscription to
/// the fact rather than a fetch, so an activation performed elsewhere
/// reaches this tab.

/// Top-document gate sending a signed-out user to sign up, and replaying
/// what they were doing when it fired.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub mod account_gate;

/// Running a WebAuthn ceremony on the service worker's behalf.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub mod custody_relay;

/// The registration dialog raised when sharing needs an account.
pub mod register_dialog;

/// API client for interacting with the Tonk service worker.
pub mod api;

mod worker_client;

#[cfg(any(all(target_arch = "wasm32", target_os = "unknown"), test))]
mod callback_url {
    //! Browser-to-loopback callback URL construction.

    /// Build the loopback navigation target carrying delivery fields in its
    /// URL fragment.
    pub(crate) fn delivery_url(callback: &str, fields: &[(&str, &str)]) -> Result<String, String> {
        let mut target = url::Url::parse(callback)
            .map_err(|_| "the authorization callback address is invalid".to_owned())?;
        let is_loopback_callback = target.scheme() == "http"
            && target.host_str() == Some("127.0.0.1")
            && target.port().is_some()
            && target.path() == "/"
            && target.query().is_none()
            && target.fragment().is_none()
            && target.username().is_empty()
            && target.password().is_none();
        if !is_loopback_callback {
            return Err("the authorization callback is not a Tonk loopback address".to_owned());
        }
        let mut serializer = url::form_urlencoded::Serializer::new(String::new());
        serializer.extend_pairs(fields.iter().copied());
        target.set_fragment(Some(&serializer.finish()));
        Ok(target.into())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn it_carries_callback_fields_in_the_url_fragment() {
            let target = delivery_url(
                "http://127.0.0.1:4321",
                &[
                    ("authorize", "grant+/="),
                    ("redirect", "https://tonk.test/settings?from=cli"),
                ],
            )
            .unwrap();

            assert_eq!(
                target,
                "http://127.0.0.1:4321/#authorize=grant%2B%2F%3D&redirect=https%3A%2F%2Ftonk.test%2Fsettings%3Ffrom%3Dcli"
            );
            let parsed = url::Url::parse(&target).unwrap();
            assert!(
                parsed.query().is_none(),
                "the cross-scheme GET must be bodyless"
            );
        }

        #[test]
        fn it_rejects_a_non_loopback_callback() {
            for callback in [
                "javascript:alert(document.cookie)",
                "https://attacker.example/collect",
                "http://localhost:4321/",
                "http://127.0.0.1/collect",
            ] {
                assert!(
                    delivery_url(callback, &[("authorize", "grant")]).is_err(),
                    "callback navigation must stay on Tonk's loopback endpoint: {callback}"
                );
            }
        }
    }
}

/// PostHog wiring for the shell page: panic hook, pageviews, and
/// DOM-event listeners. Wasm-only — depends on `tonk_analytics::web`,
/// which only exists for `wasm32-unknown-unknown`.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub mod analytics;

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod deployment;

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod device_name;

/// Error types for the Tonk UI.
pub mod error;

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod identity_bridge;

mod user_error;

/// Test helpers for integration testing.
#[cfg(any(test, feature = "helpers"))]
pub mod helpers;

/// Real-browser account-panel and CLI roundtrip tests.
#[cfg(test)]
mod account_flow;

/// Real-browser passkey ceremony tests.
#[cfg(test)]
mod identity;

/// Real-browser service-worker load-time upgrade tests.
#[cfg(test)]
mod service_worker_upgrade;
