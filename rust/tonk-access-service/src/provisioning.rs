//! The provisioning gate on presign: a subject is served only while
//! someone pays for it.
//!
//! Cryptographic authorization proves the caller may act on the
//! subject; this gate asks whether the subject is anyone's to serve.
//! A subject is servable when it is an active customer itself, or a
//! provisioned consumer whose provider is an active customer — the
//! rule the `Consumer` row has always documented. Without it, any
//! self-minted keypair stores bytes under its own namespace unbilled.
//!
//! Behind a flag (`REQUIRE_PROVISIONING`), default off: existing
//! deployments ramp it only once their spaces are provisioned, and the
//! browser's creation ceremony still publishes the custody cell before
//! the account registers — that ordering must move before a deployment
//! that serves browsers can enforce.

use dialog_capability::access::AuthorizeError;
use dialog_ucan_core::{Container, Invocation};
use dialog_varsig::AnySignature;
use tonk_account::customer::CustomerStatus;

use crate::store::{Store, StoreError};

/// The predicate reported when the gate refuses. One string on the
/// wire, deliberately: the client-facing vocabulary is dialog's
/// [`AuthorizeError`], and the service's serving policy failing is a
/// policy violation; the detail distinguishes the causes for logs and
/// metering without teaching clients a new code table. A first-class
/// upstream variant is the upgrade path.
fn denial(cause: &str) -> AuthorizeError {
    AuthorizeError::PolicyViolation {
        predicate: format!("subject is provisioned by an active customer ({cause})"),
    }
}

/// The subject of the presented container — the same field metering
/// attributes the invocation to.
pub fn container_subject(container_bytes: &[u8]) -> Option<String> {
    let tokens = Container::from_bytes(container_bytes).ok()?.into_tokens();
    let body = tokens.into_iter().next()?;
    let invocation: Invocation<AnySignature> = serde_ipld_dagcbor::from_slice(&body).ok()?;
    Some(invocation.subject().to_string())
}

/// Screen `subject` against the control store.
///
/// `Ok(Ok(()))` is servable; `Ok(Err(_))` is a refusal the caller
/// answers as 403 and meters as a denial; `Err(_)` is a store failure —
/// the gate fails closed, but as the service's own unavailability, not
/// as a denial billed to the customer.
pub async fn screen<S: Store>(
    store: &S,
    subject: &str,
) -> Result<Result<(), AuthorizeError>, StoreError> {
    if let Some(customer) = store.customer(subject).await? {
        return Ok(servable(customer.status, "the subject's own registration"));
    }
    let Some(consumer) = store.consumer(subject).await? else {
        return Ok(Err(denial("the subject is not provisioned")));
    };
    let Some(provider) = consumer.provider else {
        return Ok(Err(denial("the subject has no provider")));
    };
    match store.customer(&provider).await? {
        Some(customer) => Ok(servable(customer.status, "the provider's registration")),
        None => Ok(Err(denial("the provider is not a customer"))),
    }
}

fn servable(status: CustomerStatus, who: &str) -> Result<(), AuthorizeError> {
    match status {
        CustomerStatus::Active => Ok(()),
        CustomerStatus::Registered => Err(denial(&format!("{who} awaits email activation"))),
        CustomerStatus::Suspended => Err(denial(&format!("{who} is suspended"))),
    }
}

#[cfg(all(test, feature = "helpers", not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use crate::store::sqlite::SqliteStore;

    async fn store_with(
        customers: &[(&str, CustomerStatus)],
        consumers: &[(&str, &str)],
    ) -> SqliteStore {
        let store = SqliteStore::in_memory().expect("in-memory store");
        for (did, status) in customers {
            store
                .enroll_customer(did, &format!("{did}@example.com"), b"", "trial@2026-08", 0)
                .await
                .expect("customer");
            if *status == CustomerStatus::Active {
                store
                    .activate_customer(did, "v1", 1)
                    .await
                    .expect("activate");
            }
        }
        for (consumer, provider) in consumers {
            store
                .add_consumer(consumer, provider, 0, crate::store::ConsumerKind::Space)
                .await
                .expect("consumer");
        }
        store
    }

    #[dialog_common::test]
    async fn it_serves_an_active_customer_and_its_provisioned_consumer() {
        let store = store_with(
            &[("did:key:zCustomer", CustomerStatus::Active)],
            &[("did:key:zSpace", "did:key:zCustomer")],
        )
        .await;
        assert_eq!(screen(&store, "did:key:zCustomer").await.unwrap(), Ok(()));
        assert_eq!(screen(&store, "did:key:zSpace").await.unwrap(), Ok(()));
    }

    #[dialog_common::test]
    async fn it_refuses_an_unknown_subject_and_an_inactive_chain() {
        let store = store_with(
            &[("did:key:zPending", CustomerStatus::Registered)],
            &[("did:key:zSpace", "did:key:zPending")],
        )
        .await;
        assert!(screen(&store, "did:key:zNobody").await.unwrap().is_err());
        assert!(screen(&store, "did:key:zPending").await.unwrap().is_err());
        assert!(screen(&store, "did:key:zSpace").await.unwrap().is_err());
    }
}
