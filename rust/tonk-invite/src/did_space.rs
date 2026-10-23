//! Spaces named by a DID: `/space/did:web:example.com`.
//!
//! A space's own address names its repository, a `did:key`. A DID with a
//! document of its own can name a space too, by saying in that document
//! which repository it means and which peers hold replicas of it:
//!
//! ```json
//! {
//!   "id": "did:web:example.com",
//!   "service": [
//!     {
//!       "id": "did:web:example.com#space",
//!       "type": "DialogRepository",
//!       "serviceEndpoint": "did:key:z6Mk…"
//!     },
//!     {
//!       "id": "did:web:example.com#peer",
//!       "type": "UcanAccessService",
//!       "serviceEndpoint": "https://tonk.network/ucan/"
//!     }
//!   ]
//! }
//! ```
//!
//! The repository is a service of the DID rather than another name for
//! it (`alsoKnownAs`): the DID's subject is whoever publishes the space,
//! not the space. A document names one space. A DID publishing several
//! gives each its own document, under a path (`did:web:example.com:notes`).
//!
//! The peers are the access services the document names, in its order,
//! the same kind a peer's own document names its access service by. The
//! space opens from them exactly as its bare address opens it from the
//! host serving it: with the public ticket the space keeps, if it is
//! published. Naming a space grants nothing; whoever controls the domain
//! chooses which space it names, not who may read it.
//!
//! This module only reads a document's services. Fetching them is the
//! caller's (see [`dialog_did_web::Discover`]).

use dialog_did_web::{Service, did_web_url};
use dialog_varsig::Did;
use thiserror::Error;
use url::Url;

use crate::ticket::Ticket;

/// The service kind a DID document names its space's repository by.
pub const REPOSITORY_SERVICE: &str = "DialogRepository";

/// The service kind a DID document names a peer holding the space by.
pub const PEER_SERVICE: &str = dialog_remote_ucan::ACCESS_SERVICE;

/// Why a DID's document names no space this build can open.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum DidSpaceError {
    /// The DID is not one a document is fetched for.
    #[error("{did} has no document to name a space in")]
    Unsupported {
        /// The DID.
        did: String,
    },

    /// The document names no repository.
    #[error("{did} names no space")]
    NoSpace {
        /// The DID.
        did: String,
    },

    /// The document names more than one repository, so it is not clear
    /// which one its address opens.
    #[error("{did} names {count} spaces; a document names one")]
    Ambiguous {
        /// The DID.
        did: String,
        /// How many it names.
        count: usize,
    },

    /// The repository the document names is not a `did:key`.
    #[error("{did} names {repository}, which is not a space")]
    NotASpace {
        /// The DID.
        did: String,
        /// What it names.
        repository: String,
    },

    /// The document names no peer the space can be opened from.
    #[error("{did} names no peer holding its space")]
    NoPeer {
        /// The DID.
        did: String,
    },
}

/// The public tickets the space `did` names would be claimed with, one
/// for each peer its document names, in the document's order.
///
/// `services` are the services `did`'s own document names (see
/// [`dialog_did_web::Discover`]).
///
/// A peer is reached over `https`, or over plain `http` only where the
/// document itself is (a loopback `did:web`): a document fetched over
/// `https` cannot point a visitor at a plain-text peer, nor at a host on
/// the visitor's own machine. A peer that does not qualify, or is no URL,
/// is skipped.
///
/// # Errors
///
/// Returns a [`DidSpaceError`] if the document names no space, more than
/// one, something other than a `did:key`, or no peer to open it from.
pub fn tickets(did: &Did, services: &[Service]) -> Result<Vec<Ticket>, DidSpaceError> {
    let document = did_web_url(did.as_str())
        .ok()
        .and_then(|url| Url::parse(&url).ok())
        .ok_or_else(|| DidSpaceError::Unsupported {
            did: did.to_string(),
        })?;
    let repositories: Vec<&Service> = services
        .iter()
        .filter(|service| service.is(REPOSITORY_SERVICE))
        .collect();
    let repository = match repositories.as_slice() {
        [] => {
            return Err(DidSpaceError::NoSpace {
                did: did.to_string(),
            });
        }
        [one] => *one,
        many => {
            return Err(DidSpaceError::Ambiguous {
                did: did.to_string(),
                count: many.len(),
            });
        }
    };
    let named = repository.url().unwrap_or_default();
    let subject = named
        .parse::<Did>()
        .ok()
        .filter(|subject| subject.method() == "key")
        .ok_or_else(|| DidSpaceError::NotASpace {
            did: did.to_string(),
            repository: named.to_string(),
        })?;

    let tickets: Vec<Ticket> = services
        .iter()
        .filter(|service| service.is(PEER_SERVICE))
        .filter_map(Service::url)
        .filter_map(|endpoint| Url::parse(endpoint).ok())
        .filter(|endpoint| endpoint.scheme() == "https" || endpoint.scheme() == document.scheme())
        .filter_map(|endpoint| Ticket::public(subject.clone(), &endpoint).ok())
        .collect();
    if tickets.is_empty() {
        return Err(DidSpaceError::NoPeer {
            did: did.to_string(),
        });
    }
    Ok(tickets)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_browser);

    const SPACE: &str = "did:key:z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK";

    fn service(kind: &str, endpoint: serde_json::Value) -> Service {
        serde_json::from_value(json!({ "type": kind, "serviceEndpoint": endpoint }))
            .expect("a service entry")
    }

    fn did(did: &str) -> Did {
        did.parse().expect("a DID")
    }

    fn remotes(tickets: &[Ticket]) -> Vec<&str> {
        tickets
            .iter()
            .map(|ticket| ticket.remote().as_str())
            .collect()
    }

    /// The document's repository is the space, and each access service it
    /// names is a peer to claim the space's public ticket from, in order.
    #[dialog_common::test]
    fn it_opens_the_named_space_from_each_peer_in_order() {
        let tickets = tickets(
            &did("did:web:example.com"),
            &[
                service(PEER_SERVICE, json!("https://first.example/ucan/")),
                service(REPOSITORY_SERVICE, json!(SPACE)),
                service("TonkAccessService", json!("https://other.example/ucan/")),
                service(PEER_SERVICE, json!("https://second.example/ucan/")),
            ],
        )
        .expect("the document names a space");
        assert_eq!(
            remotes(&tickets),
            [
                "https://first.example/ucan/",
                "https://second.example/ucan/"
            ]
        );
        for ticket in &tickets {
            assert_eq!(ticket.subject(), &did(SPACE));
            assert!(ticket.is_public(), "naming a space grants nothing");
        }
    }

    #[dialog_common::test]
    fn it_refuses_a_document_naming_no_space_or_several() {
        let peer = service(PEER_SERVICE, json!("https://tonk.example/ucan/"));
        let space = service(REPOSITORY_SERVICE, json!(SPACE));
        let named = did("did:web:example.com");
        assert!(matches!(
            tickets(&named, std::slice::from_ref(&peer)),
            Err(DidSpaceError::NoSpace { .. })
        ));
        assert!(matches!(
            tickets(&named, &[space.clone(), space, peer]),
            Err(DidSpaceError::Ambiguous { count: 2, .. })
        ));
    }

    /// A repository is a `did:key`; anything else in its place, another
    /// DID or a URL, names no space.
    #[dialog_common::test]
    fn it_refuses_a_repository_that_is_not_a_did_key() {
        let named = did("did:web:example.com");
        for repository in [
            json!("did:web:elsewhere.example"),
            json!("https://example.com/space"),
            json!({ "repository": SPACE }),
        ] {
            assert!(
                matches!(
                    tickets(
                        &named,
                        &[
                            service(REPOSITORY_SERVICE, repository.clone()),
                            service(PEER_SERVICE, json!("https://tonk.example/ucan/")),
                        ],
                    ),
                    Err(DidSpaceError::NotASpace { .. })
                ),
                "{repository}"
            );
        }
    }

    /// A document served over `https` cannot send a visitor to a
    /// plain-text peer or one on the visitor's machine; a loopback
    /// document, itself fetched over `http`, can.
    #[dialog_common::test]
    fn it_takes_plain_http_peers_only_from_a_loopback_document() {
        let services = [
            service(REPOSITORY_SERVICE, json!(SPACE)),
            service(PEER_SERVICE, json!("http://127.0.0.1:8090/ucan/")),
            service(PEER_SERVICE, json!("not a url")),
        ];
        assert!(matches!(
            tickets(&did("did:web:example.com"), &services),
            Err(DidSpaceError::NoPeer { .. })
        ));
        let local = tickets(&did("did:web:localhost%3A8090"), &services)
            .expect("a loopback document names a loopback peer");
        assert_eq!(remotes(&local), ["http://127.0.0.1:8090/ucan/"]);
    }

    #[dialog_common::test]
    fn it_names_no_space_for_a_did_without_a_document() {
        assert!(matches!(
            tickets(&did(SPACE), &[service(REPOSITORY_SERVICE, json!(SPACE))]),
            Err(DidSpaceError::Unsupported { .. })
        ));
    }
}
