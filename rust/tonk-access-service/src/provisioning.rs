//! The provisioning gate on presign: a subject is served only while
//! someone pays for it.
//!
//! Cryptographic authorization proves the caller may act on the
//! subject; this gate asks whether the subject is anyone's to serve.
//! A subject is servable when it is an active customer itself, or a
//! provisioned consumer whose provider is an active customer — the
//! rule the `Subscription` row has always documented. Without it, any
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

pub mod cache;

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
    // One query, not three. This runs before every presign, and read in
    // three separate steps it could see a customer that activates
    // between the first and the last.
    //
    // The DID keeps its own name: every refusal below names it, because
    // "the subject is not provisioned" alone leaves a caller with no way
    // to tell WHICH subject a failing request was about — and these
    // arrive in logs where the request that produced them is long gone.
    let did = subject;
    let subject = store.servability(did).await?;

    // An account is both a customer and its own consumer, so its own
    // registration answers without consulting the consumer half.
    if let Some(status) = subject.own {
        return Ok(servable(status, &format!("{did}'s own registration")));
    }
    if !subject.consumer {
        return Ok(Err(denial(
            Recourse::None,
            &format!("{did} is not provisioned"),
        )));
    }
    // A purge in flight refuses first: the objects may already be
    // half gone, and reading a partly deleted space is worse than
    // reading none of it.
    if subject.deleted_at.is_some() {
        return Ok(Err(denial(
            Recourse::None,
            &format!("the subscription for {did} is being deleted"),
        )));
    }
    // Archived data is gone by definition; the row survives only so what
    // it accrued can still be billed.
    if subject.archived_at.is_some() {
        return Ok(Err(denial(
            Recourse::None,
            &format!("the subscription for {did} is archived"),
        )));
    }
    // A suspension with a deadline lifts itself: past that moment the
    // row still carries the reason, and it no longer applies. Retryable
    // for the same reason — waiting is what clears it.
    if let Some(suspension) = &subject.suspension {
        match suspension.until {
            Some(until) if until <= now => {}
            Some(_) => {
                return Ok(Err(denial(
                    Recourse::Retry,
                    &format!(
                        "the subscription for {did} is suspended: {}",
                        suspension.message
                    ),
                )));
            }
            None => {
                return Ok(Err(denial(
                    Recourse::None,
                    &format!(
                        "the subscription for {did} is suspended: {}",
                        suspension.message
                    ),
                )));
            }
        }
    }
    // An expired subscription serves nothing, whatever the customer
    // behind it is doing. Null never expires.
    if subject
        .expires_at
        .is_some_and(|expires_at| expires_at <= now)
    {
        return Ok(Err(denial(
            Recourse::None,
            &format!("the subscription for {did} has expired"),
        )));
    }
    match subject.provider {
        Some(status) => Ok(servable(status, &format!("the provider of {did}"))),
        // A subscription always names a provider, so this is that
        // customer having gone missing. Unreachable under SQLite, which
        // enforces the foreign key and refuses to delete a customer any
        // subscription still names — but D1 does not enforce foreign
        // keys, so this refuses rather than assuming it cannot arise.
        // Untested for the same reason: the fixture will not build it.
        None => Ok(Err(denial(
            Recourse::None,
            &format!("the provider of {did} is not a customer"),
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
    use crate::store::Enrollment;
    use crate::store::sqlite::SqliteStore;

    async fn store_with(
        customers: &[(&str, CustomerStatus)],
        consumers: &[(&str, &str)],
    ) -> SqliteStore {
        let store = SqliteStore::in_memory().expect("in-memory store");
        for (did, status) in customers {
            store
                .enroll_customer(Enrollment {
                    did,
                    email: &format!("{did}@example.com"),
                    plan: "trial@2026-08",
                    ledger: did,
                    custody: &format!("{}-custody", did),
                    now: 0,
                    expires_at: u64::MAX,
                })
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
                .add_subscription(consumer, provider, 0, crate::store::SubscriptionKind::Space)
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

    /// An expired subscription serves nothing, however healthy the
    /// customer behind it is. `None` never expires, which is what every
    /// subscription carries today.
    #[dialog_common::test]
    async fn it_refuses_an_expired_subscription() {
        let store = store_with(
            &[("did:key:zCustomer", CustomerStatus::Active)],
            &[("did:key:zSpace", "did:key:zCustomer")],
        )
        .await;
        assert_eq!(recourse_of(&store, "did:key:zSpace").await, None);

        store.expire_for_test("did:key:zSpace", NOW - 1).await;
        assert_eq!(
            recourse_of(&store, "did:key:zSpace").await,
            Some(Recourse::None),
            "an expired subscription is not worth retrying: it needs renewing"
        );
    }

    /// The boundary: a subscription is served up to the moment it
    /// expires, not up to the moment before.
    #[dialog_common::test]
    async fn it_serves_a_subscription_until_the_instant_it_expires() {
        let store = store_with(
            &[("did:key:zCustomer", CustomerStatus::Active)],
            &[("did:key:zSpace", "did:key:zCustomer")],
        )
        .await;
        store.expire_for_test("did:key:zSpace", NOW + 1).await;
        assert_eq!(recourse_of(&store, "did:key:zSpace").await, None);

        store.expire_for_test("did:key:zSpace", NOW).await;
        assert_eq!(
            recourse_of(&store, "did:key:zSpace").await,
            Some(Recourse::None)
        );
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
