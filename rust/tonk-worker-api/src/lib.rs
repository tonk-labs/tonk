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

/// Response header carrying a machine-readable worker error kind when a
/// client must react without consuming the structured response body.
pub const ERROR_KIND_HEADER: &str = "x-tonk-error-kind";

/// [`ERROR_KIND_HEADER`] value for a write refused across a page/worker build
/// boundary.
pub const STALE_BUILD_ERROR_KIND: &str = "stale-build";

/// Request header carrying the immutable worker generation the calling page
/// was emitted against.
pub const PAGE_BUILD_HEADER: &str = "x-tonk-build";

/// Internal request header binding the scoped language-server POST and SSE
/// stream to one trusted portal instance. Sealed guests may supply this name,
/// but the portal relay always strips it and stamps its own value after route
/// authorization; top-level clients instead use their service-worker ClientId.
pub const LSP_CLIENT_HEADER: &str = "x-tonk-lsp-client";

/// Maximum number of authorized portal relays represented in one LSP client
/// principal. Bounding the chain also keeps the complete trusted header below
/// the worker's 256-byte client-key limit.
pub const LSP_CLIENT_CHAIN_MAX_DEPTH: usize = 6;

fn is_lsp_client_segment(value: &str) -> bool {
    value.len() == 34
        && value.starts_with("p-")
        && value[2..]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn lsp_client_chain_segments(value: &str) -> Option<Vec<&str>> {
    let mut parts = value.split('/');
    if parts.next() != Some("v1") {
        return None;
    }
    let segments: Vec<_> = parts.collect();
    if segments.is_empty()
        || segments.len() > LSP_CLIENT_CHAIN_MAX_DEPTH
        || !segments
            .iter()
            .all(|segment| is_lsp_client_segment(segment))
    {
        return None;
    }
    Some(segments)
}

/// Whether `value` is the exact bounded wire spelling of a portal LSP client
/// chain. No alternate case, segment width, version, or empty segment aliases
/// are accepted.
pub fn is_canonical_lsp_client_chain(value: &str) -> bool {
    lsp_client_chain_segments(value).is_some()
}

/// Namespace an optional descendant chain beneath the current authorized
/// portal's host-minted segment. A malformed caller value is ignored rather
/// than becoming authority; a canonical value is preserved only below the
/// current portal, so it can never replace an ancestor principal. Chains that
/// would exceed the protocol bound fail closed instead of collapsing clients.
pub fn compose_lsp_client_chain(
    own: &str,
    forwarded: Option<&str>,
) -> Result<String, &'static str> {
    if !is_lsp_client_segment(own) {
        return Err("invalid host-minted LSP client segment");
    }
    let mut segments = vec![own];
    if let Some(forwarded) = forwarded
        && let Some(descendants) = lsp_client_chain_segments(forwarded)
    {
        if descendants.len() + 1 > LSP_CLIENT_CHAIN_MAX_DEPTH {
            return Err("nested LSP client chain exceeds the relay limit");
        }
        segments.extend(descendants);
    }
    Ok(format!("v1/{}", segments.join("/")))
}

mod account;
mod claim;
mod conclusion;
mod deployment;
mod evaluate;
mod identify;
mod identity;
mod invite;
mod join;
mod lsp_scope;
mod profile;
mod profiles;
mod query;
mod repository;
pub mod share;
mod sync;

pub use account::{
    AccountDeletionPlan, AccountDeletionRequest, AccountDeletionResult, AccountDeletionSpace,
    AccountDevice, AccountDisplayNameRequest, AccountDisplayNameResponse, AccountLinkRequest,
    AccountSpaceDeletionRequest, AccountStatus, AccountSummary, HostedSpaceDeletionResult,
    RevokeDeviceAcknowledgement, RevokeDeviceRequest,
};
pub use claim::{ClaimResponse, QueryResponse};
pub use conclusion::{Conclusion, Frame};
pub use deployment::DeploymentConfig;
pub use evaluate::{CommitSummary, EvaluateResponse, QueryMatchBlock, QueryResult};
pub use identify::IdentifyResponse;
pub use identity::{
    AccountCreation, CREATE_ACCOUNT_REQUEST, CUSTODY_REQUEST, CustodyIntent, DeviceLink,
    ENCRYPTION_KEY_REQUEST, Enrollment, LINK_ACCOUNT, LinkAccountRequest, PasskeyAddition,
    PasskeyMetadata, RootStatus, SaveRootRequest, WEBAUTHN, WebAuthnKind, WebAuthnRequest,
};
pub use invite::{
    CreateInviteRequest, CreateInviteResponse, InvitationKind, InvitationSummary,
    RevokeInvitationAcknowledgement,
};
pub use join::{JoinFailureKind, JoinRequest, JoinResponse};
pub use lsp_scope::{decode_lsp_scope_segment, encode_lsp_scope_segment};
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
                            "name": { "the": "dom.event.current-target.elements.name/value", "as": "Text" }
                        }
                    }
                },
                "parameters": { "name": name }
            }
        }]
    })
}

#[cfg(test)]
mod lsp_client_chain_tests {
    use super::*;

    const OUTER: &str = "p-11111111111111111111111111111111";
    const INNER_A: &str = "p-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const INNER_B: &str = "p-bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    #[test]
    fn composes_distinct_canonical_nested_principals() {
        let direct = compose_lsp_client_chain(OUTER, None).expect("direct portal");
        let child_a =
            compose_lsp_client_chain(OUTER, Some("v1/p-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"))
                .expect("first child");
        let child_b =
            compose_lsp_client_chain(OUTER, Some("v1/p-bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"))
                .expect("second child");

        assert_eq!(direct, format!("v1/{OUTER}"));
        assert_eq!(child_a, format!("v1/{OUTER}/{INNER_A}"));
        assert_eq!(child_b, format!("v1/{OUTER}/{INNER_B}"));
        assert_ne!(child_a, child_b);
        assert!(is_canonical_lsp_client_chain(&child_a));
    }

    #[test]
    fn namespaces_or_ignores_forged_values_and_bounds_depth() {
        assert_eq!(
            compose_lsp_client_chain(OUTER, Some("portal-forged")).unwrap(),
            format!("v1/{OUTER}"),
            "an authored legacy header must not become the worker principal",
        );
        assert_eq!(
            compose_lsp_client_chain(OUTER, Some(&format!("v1/{INNER_A}"))).unwrap(),
            format!("v1/{OUTER}/{INNER_A}"),
            "even a canonical descendant is namespaced below the authorized relay",
        );
        let full = format!(
            "v1/{0}/{0}/{0}/{0}/{0}/{0}",
            "p-22222222222222222222222222222222"
        );
        assert!(compose_lsp_client_chain(OUTER, Some(&full)).is_err());
        assert!(!is_canonical_lsp_client_chain(
            "v1/p-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
        ));
    }
}
