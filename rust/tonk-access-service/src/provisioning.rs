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
//! Always enforced, on both the worker and the native server: a flag
//! defaulted off is a gate that is not there, and one defaulted on is a
//! switch for turning billing enforcement off in production. Clients
//! hold work that lands before activation as a pending queue and replay
//! it once the email is confirmed, rather than relying on a ramp.
//!
//! The refusal carries which registration state said no, so a client
//! waiting on an email confirmation can tell that from a suspension
//! without reading prose. That also makes this gate the activation
//! signal: a client whose account space is refused with
//! `AwaitingActivation` learns it is confirmed when the same sync
//! succeeds, with no separate probe.

use dialog_capability::access::{AuthorizeError, Recourse};
use dialog_ucan_core::{Container, Invocation};
use dialog_varsig::AnySignature;
use tonk_account::customer::CustomerStatus;

use crate::store::{Store, StoreError};

/// The refusal reported when the gate says no.
///
/// [`AuthorizeError::Declined`] rather than a policy violation: the
/// chain authorized the request, and what refused is this service's own
/// policy about whose subjects it carries. Dialog does not model that
/// policy and should not, so the cause travels as our sentence and the
/// only structured part is whether waiting resolves it.
///
/// That one bit is what a client needs. An account awaiting its
/// activation email is [`Recourse::Retry`]: the same request succeeds
/// once someone opens the link, so a client showing "check your email"
/// keeps its work in hand and learns it was confirmed when the retry
/// goes through. Everything else is [`Recourse::None`] -- a suspension
/// and an unprovisioned subject change only when someone acts, and a
/// client that kept retrying would be waiting on nothing.
///
/// This is the first-class upstream variant the predicate-formatting
/// version named as its upgrade path (dialog-db#470).
fn denial(recourse: Recourse, cause: &str) -> AuthorizeError {
    AuthorizeError::Declined {
        recourse,
        reason: cause.to_string(),
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
    now: u64,
) -> Result<Result<(), AuthorizeError>, StoreError> {
    if let Some(customer) = store.customer(subject).await? {
        return Ok(servable(customer.status, "the subject's own registration"));
    }
    let Some(consumer) = store.consumer(subject).await? else {
        return Ok(Err(denial(
            Recourse::None,
            "the subject is not provisioned",
        )));
    };
    // A live reservation holds the name and nothing more. The row carries
    // a provider so the claim can be checked, which would otherwise read
    // here as provisioned — so the reservation is what this asks about
    // first. Retryable: the provisioning that follows is what serves it.
    if consumer.reserved_until.is_some_and(|until| until > now) {
        return Ok(Err(denial(
            Recourse::Retry,
            "the subject is reserved but not yet provisioned",
        )));
    }
    let Some(provider) = consumer.provider else {
        return Ok(Err(denial(Recourse::None, "the subject has no provider")));
    };
    match store.customer(&provider).await? {
        Some(customer) => Ok(servable(customer.status, "the provider's registration")),
        None => Ok(Err(denial(
            Recourse::None,
            "the provider is not a customer",
        ))),
    }
}

/// Map a customer's registration to whether its subjects are served.
///
/// The recourse is about *this* subject, not about who holds the
/// registration: a consumer whose provider awaits activation is itself
/// worth retrying, because confirming that email is what serves it.
/// `who` names which registration refused, for the reader.
fn servable(status: CustomerStatus, who: &str) -> Result<(), AuthorizeError> {
    match status {
        CustomerStatus::Active => Ok(()),
        CustomerStatus::Registered => Err(denial(
            Recourse::Retry,
            &format!("{who} awaits email activation"),
        )),
        CustomerStatus::Suspended => Err(denial(Recourse::None, &format!("{who} is suspended"))),
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
            match status {
                CustomerStatus::Registered => {}
                CustomerStatus::Active => {
                    store
                        .activate_customer(did, "v1", 1)
                        .await
                        .expect("activate");
                }
                // Nothing writes `Suspended` yet: there is no admin path
                // that suspends a customer, so the fixture sets the
                // column the way one eventually will.
                CustomerStatus::Suspended => store.suspend_for_test(did).await,
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

    /// A fixed "now" for the gate. The fixtures register at 0, so any
    /// positive value reads as the present.
    const NOW: u64 = 1_000;

    #[dialog_common::test]
    async fn it_serves_an_active_customer_and_its_provisioned_consumer() {
        let store = store_with(
            &[("did:key:zCustomer", CustomerStatus::Active)],
            &[("did:key:zSpace", "did:key:zCustomer")],
        )
        .await;
        assert_eq!(
            screen(&store, "did:key:zCustomer", NOW).await.unwrap(),
            Ok(())
        );
        assert_eq!(screen(&store, "did:key:zSpace", NOW).await.unwrap(), Ok(()));
    }

    #[dialog_common::test]
    async fn it_refuses_an_unknown_subject_and_an_inactive_chain() {
        let store = store_with(
            &[("did:key:zPending", CustomerStatus::Registered)],
            &[("did:key:zSpace", "did:key:zPending")],
        )
        .await;
        assert!(
            screen(&store, "did:key:zNobody", NOW)
                .await
                .unwrap()
                .is_err()
        );
        assert!(
            screen(&store, "did:key:zPending", NOW)
                .await
                .unwrap()
                .is_err()
        );
        assert!(
            screen(&store, "did:key:zSpace", NOW)
                .await
                .unwrap()
                .is_err()
        );
    }

    /// The recourse a refusal carries, or `None` when it served.
    async fn recourse_of(store: &SqliteStore, subject: &str) -> Option<Recourse> {
        match screen(store, subject, NOW).await.unwrap() {
            Ok(()) => None,
            Err(AuthorizeError::Declined { recourse, .. }) => Some(recourse),
            Err(other) => panic!("the gate refused with {other:?}, not a declined request"),
        }
    }

    /// The distinction the typed recourse exists for: a client waiting
    /// on an email confirmation keeps retrying, and one told the account
    /// is suspended must stop. Everything finer than that stays in the
    /// sentence, which is deliberately not something a client parses.
    #[dialog_common::test]
    async fn it_says_whether_the_refusal_is_worth_retrying() {
        let store = store_with(
            &[
                ("did:key:zPending", CustomerStatus::Registered),
                ("did:key:zStopped", CustomerStatus::Suspended),
            ],
            &[],
        )
        .await;
        assert_eq!(
            recourse_of(&store, "did:key:zPending").await,
            Some(Recourse::Retry),
            "an unconfirmed email resolves when someone opens the link"
        );
        assert_eq!(
            recourse_of(&store, "did:key:zStopped").await,
            Some(Recourse::None),
            "a suspension does not clear by asking again"
        );
        assert_eq!(
            recourse_of(&store, "did:key:zNobody").await,
            Some(Recourse::None),
            "nor does a subject nobody provisioned"
        );
    }

    /// A consumer reports what would serve IT, not a description of who
    /// holds the registration. A space whose provider awaits activation
    /// is worth retrying, because confirming that email serves the space.
    #[dialog_common::test]
    async fn it_reports_a_consumer_by_what_would_serve_it() {
        let store = store_with(
            &[
                ("did:key:zPending", CustomerStatus::Registered),
                ("did:key:zStopped", CustomerStatus::Suspended),
            ],
            &[
                ("did:key:zWaiting", "did:key:zPending"),
                ("did:key:zHalted", "did:key:zStopped"),
            ],
        )
        .await;
        assert_eq!(
            recourse_of(&store, "did:key:zWaiting").await,
            Some(Recourse::Retry)
        );
        assert_eq!(
            recourse_of(&store, "did:key:zHalted").await,
            Some(Recourse::None)
        );
    }

    /// Activation is the refusal clearing. This is the transition a
    /// waiting client watches for, so it is worth pinning that the same
    /// subject answers differently either side of it.
    #[dialog_common::test]
    async fn it_serves_the_subject_once_activation_lands() {
        let store = store_with(&[("did:key:zCustomer", CustomerStatus::Registered)], &[]).await;
        assert_eq!(
            recourse_of(&store, "did:key:zCustomer").await,
            Some(Recourse::Retry)
        );

        store
            .activate_customer("did:key:zCustomer", "2026-08", 1)
            .await
            .expect("activation records");
        assert_eq!(recourse_of(&store, "did:key:zCustomer").await, None);
    }

    /// A reservation holds the name and serves nothing. The row carries
    /// a provider so the claim can be checked, which without this gate
    /// would read as provisioned and serve a space nobody has paid for
    /// yet.
    #[dialog_common::test]
    async fn it_refuses_a_reserved_subject_until_it_is_provisioned() {
        let store = store_with(&[("did:key:zCustomer", CustomerStatus::Active)], &[]).await;
        store
            .reserve_consumer(
                "did:key:zHeld",
                "did:key:zCustomer",
                0,
                crate::store::ConsumerKind::Custody,
                NOW + 1,
            )
            .await
            .expect("reservation");

        assert_eq!(
            recourse_of(&store, "did:key:zHeld").await,
            Some(Recourse::Retry),
            "the provisioning that follows is what serves it, so the client retries"
        );

        // Claiming it clears the hold, and the same subject is served.
        store
            .add_consumer(
                "did:key:zHeld",
                "did:key:zCustomer",
                0,
                crate::store::ConsumerKind::Custody,
            )
            .await
            .expect("claim");
        assert_eq!(recourse_of(&store, "did:key:zHeld").await, None);
    }

    /// A reservation that has lapsed no longer holds anything back: the
    /// row is claimable, and until someone claims it the subject is
    /// judged by its provider like any other.
    #[dialog_common::test]
    async fn it_stops_holding_a_subject_once_the_reservation_lapses() {
        let store = store_with(&[("did:key:zCustomer", CustomerStatus::Active)], &[]).await;
        store
            .reserve_consumer(
                "did:key:zLapsed",
                "did:key:zCustomer",
                0,
                crate::store::ConsumerKind::Custody,
                NOW - 1,
            )
            .await
            .expect("reservation");
        assert_eq!(recourse_of(&store, "did:key:zLapsed").await, None);
    }

    /// The sentence is for a person, not a client. It is asserted here
    /// only so a refusal that says nothing useful fails loudly; nothing
    /// in the system matches on it.
    #[dialog_common::test]
    async fn it_explains_itself_in_words_too() {
        let store = store_with(&[("did:key:zPending", CustomerStatus::Registered)], &[]).await;
        let Err(AuthorizeError::Declined { reason, .. }) =
            screen(&store, "did:key:zPending", NOW).await.unwrap()
        else {
            panic!("an unconfirmed customer is declined");
        };
        assert!(
            reason.contains("awaits email activation"),
            "the refusal must say why: {reason}"
        );
    }
}
