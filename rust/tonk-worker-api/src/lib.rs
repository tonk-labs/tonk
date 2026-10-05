#![warn(missing_docs)]
//! Wire DTOs for the Tonk service-worker HTTP API.
//!
//! These are the request/response shapes that cross the HTTP
//! boundary between the worker (`tonk-worker`) and its clients (the
//! `tonk-ui` page, the `tonk` CLI). They are plain serde data types
//! with no engine dependency, so a client can name and (de)serialize
//! them without linking the datalog engine that the worker itself
//! runs.
//!
//! `tonk-worker` re-exports every type defined here at the same
//! module paths it used to define them, so its handler code is
//! unchanged.

mod account;
mod agent_connections;
pub use agent_connections::*;
mod analytics;
mod claim;
mod conclusion;
mod deployment;
mod evaluate;
mod identify;
mod identity;
mod invite;
mod join;
mod profile;
mod profiles;
mod query;
mod repository;
pub mod share;
mod sync;

pub use account::{
    AccountDeletionPlan, AccountDeletionSpace, AccountDevice, AccountDisplayNameRequest,
    AccountDisplayNameResponse, AccountLinkRequest, AccountStatus, AccountSummary,
    RevokeDeviceAcknowledgement, RevokeDeviceRequest,
};
pub use analytics::{ANALYTICS_MESSAGE, AnalyticsEvent, AnalyticsMessage};
pub use claim::{ClaimResponse, QueryResponse};
pub use conclusion::{Conclusion, Frame};
pub use deployment::{DeploymentConfig, SiteOrigins};
pub use evaluate::{CommitSummary, EvaluateResponse, QueryMatchBlock, QueryResult};
pub use identify::IdentifyResponse;
pub use identity::{
    AccountCreation, AccountPurge, CUSTODY_REQUEST, CustodyIntent, DeviceAuthorization, DeviceLink,
    ENCRYPTION_KEY_REQUEST, Enrollment, PasskeyAddition, PasskeyMetadata, RootStatus,
    SaveRootRequest, WEBAUTHN, WebAuthnKind, WebAuthnRequest,
};
pub use invite::{
    CreateInviteRequest, CreateInviteResponse, InvitationKind, InvitationSummary,
    RevokeInvitationAcknowledgement,
};
pub use join::{JoinFailureKind, JoinRequest, JoinResponse};
pub use profile::{ProfileInfo, SpaceEntry};
pub use profiles::{ActivateProfileRequest, ProfileRosterEntry, ProfilesResponse};
pub use query::Query;
pub use repository::{
    BranchConfiguration, MemberInfo, RemoteConfiguration, RepositoryConfiguration, RepositoryInfo,
    UpstreamConfiguration,
};
use serde_json::{Value, json};
pub use sync::{
    Comparison, SyncDisposition, SyncResponse, SyncState, SyncStatusResponse, classify,
};

/// The `space/create` claim, in the shape the seeded descriptor decodes.
///
/// Defined here so every dispatcher builds the same transient: the Hub's
/// form, the FAB's `new` row, and the browser tests that drive creation
/// the way the app does.
///
/// `name` alone. No `remote`: where a space syncs is resolved worker-side
/// from the account's own registration, and a page that supplied one made
/// every create look like a deliberate choice of this server — which wired
/// spaces created before anyone registered to a service that refuses to
/// serve them. No `template` either: template seeding went with the
/// template libraries, and a field the form does not carry fails to
/// resolve and aborts the whole command.
///
/// The inline `with:` block must stay identical to the descriptor in
/// `profile.yaml`, or the transient mints a different entity and no
/// handler fires.
///
/// The attribute is the command's own (`xyz.tonk.command.create-space/name`),
/// not the DOM read path that used to fill it. A branch seeded before that
/// change still asserts the old path; the worker's handler accepts both and
/// converts (`tonk_schema::command::legacy::CreateSpace`).
pub fn create_space_claim_json(name: &str) -> Value {
    json!({
        "claims": [{
            "op": "assert",
            "application": {
                "predicate": {
                    "kind": "transient",
                    "concept": {
                        "description": "A request to create a new space.",
                        "with": {
                            "name": { "the": "xyz.tonk.command.create-space/name", "as": "Text" }
                        }
                    }
                },
                "parameters": { "name": name }
            }
        }]
    })
}

/// Callback URL construction, for handing a waiting device what the page
/// authorized: the CLI's loopback listener, or a page on another
/// deployment signing a browser in through this one.
pub mod callback {
    /// Build the navigation target carrying delivery fields in its URL
    /// fragment, which the browser never sends to a server.
    ///
    /// Two shapes are accepted. A loopback callback is exactly
    /// `http://127.0.0.1:<port>/`, the listener a CLI binds. A web
    /// callback is any `https` page: another deployment asking to sign a
    /// browser in. Any origin may ask; the approval pane names the page a
    /// grant goes to, and approving it is the person's decision. Plain
    /// `http` anywhere but loopback is refused, because the grant would
    /// travel in the clear, and so is a callback carrying credentials or a
    /// fragment of its own, which the delivery fields replace.
    pub fn delivery_url(callback: &str, fields: &[(&str, &str)]) -> Result<String, String> {
        let mut target = url::Url::parse(callback)
            .map_err(|_| "the authorization callback address is invalid".to_owned())?;
        let unadorned = target.fragment().is_none()
            && target.username().is_empty()
            && target.password().is_none();
        let is_loopback_callback = target.scheme() == "http"
            && target.host_str() == Some("127.0.0.1")
            && target.port().is_some()
            && target.path() == "/"
            && target.query().is_none();
        let is_web_callback = target.scheme() == "https" && target.host_str().is_some();
        if !unadorned || !(is_loopback_callback || is_web_callback) {
            return Err(
                "the authorization callback is neither a Tonk loopback address nor an https page"
                    .to_owned(),
            );
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
        fn it_delivers_to_an_https_page_with_its_path_and_query() {
            let target = delivery_url(
                "https://tonk.host/settings/link?via=https%3A%2F%2Ftonk.network&request=n1",
                &[("authorize", "grant+/=")],
            )
            .unwrap();

            let parsed = url::Url::parse(&target).unwrap();
            assert_eq!(parsed.origin().ascii_serialization(), "https://tonk.host");
            assert_eq!(parsed.path(), "/settings/link");
            assert_eq!(
                parsed.query(),
                Some("via=https%3A%2F%2Ftonk.network&request=n1"),
                "the requester's own query, which names its pending request, survives"
            );
            assert_eq!(parsed.fragment(), Some("authorize=grant%2B%2F%3D"));
        }

        #[test]
        fn it_rejects_callbacks_that_are_neither_loopback_nor_https() {
            for callback in [
                "javascript:alert(document.cookie)",
                "data:text/html,collect",
                "http://localhost:4321/",
                "http://127.0.0.1/collect",
                "http://tonk.host/settings/link",
                "https://user:secret@tonk.host/settings/link",
                "https://tonk.host/settings/link#already",
            ] {
                assert!(
                    delivery_url(callback, &[("authorize", "grant")]).is_err(),
                    "a grant must only travel to loopback or an https page: {callback}"
                );
            }
        }
    }
}
