//! Root-owned facts stored in the hidden account repository.

use dialog_artifacts::Entity;
use dialog_query::Concept;

use crate::domain::account::{
    ActivatedAt, CustomerEmail, DisplayName, ProviderAddress, RegisteredAt, SealedInbox,
    SuspendedAt, SuspensionReason,
};

/// The account-wide display name, keyed by the immutable account subject.
///
/// The name is cardinality-one. Concurrent linked-device writes therefore
/// converge to one deterministic value; no wall-clock latest-write ordering is
/// implied.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct AccountDisplayName {
    /// The immutable account subject.
    pub this: Entity,
    /// The authoritative account display name.
    pub name: DisplayName,
}

impl AccountDisplayName {
    /// Name the account subject.
    pub fn new(account: Entity, name: String) -> Self {
        Self {
            this: account,
            name: DisplayName(name),
        }
    }
}

/// `status` URI for an account that enrolled and has gone no further.
pub const REGISTERED: &str = "case:registered";
/// `status` URI for an account the service serves.
pub const ACTIVE: &str = "case:active";
/// `status` URI for an account the service withdrew from.
pub const SUSPENDED: &str = "case:suspended";
/// `status` URI for a locally custodied account, held until a real one
/// replaces it.
pub const ONBOARDING: &str = "case:onboarding";

/// The account enrolled an address with an access service, keyed by the
/// immutable account subject.
///
/// One of three INDEPENDENT facts — registration, activation, suspension —
/// each written once by the act that proves it and never rewritten. They
/// compose rather than replace: a suspended account is still a registered
/// one, and an activated account keeps its registration.
///
/// This replaces a single cardinality-one `status` string, which had two
/// faults. String variants are unvalidated: the old `is_active()` was a
/// `== "Active"` match a typo defeats silently. And one slot written by
/// both enrollment and activation is a race — dialog's merge picks a
/// winner, so an account that IS activated could converge back to
/// registered. Adding a fact only ever narrows, so concurrent writers
/// converge instead of fighting.
///
/// A fact, not a cached HTTP answer: every device on the account reads it
/// with an ordinary query and converges through sync, so a device that
/// never ran the enrollment still knows where the account stands.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct AccountRegistered {
    /// The immutable account subject, which is the customer DID.
    pub this: Entity,
    /// When enrollment recorded the address, unix seconds.
    pub registered_at: RegisteredAt,
    /// The address enrollment named.
    pub email: CustomerEmail,
    /// Where this account syncs: the UCAN endpoint its spaces attach their
    /// remotes to. Known at enrollment and unchanged by activation, so a
    /// client can attach its remote immediately and let the gate answer —
    /// which is what makes activation observable (403 while unconfirmed,
    /// 200 once the emailed link is opened) without asking a status
    /// endpoint.
    pub provider: ProviderAddress,
}

impl AccountRegistered {
    /// Record that `account` enrolled `email` at `at`.
    pub fn new(account: Entity, email: String, provider: String, at: u64) -> Self {
        Self {
            this: account,
            registered_at: RegisteredAt(at),
            email: CustomerEmail(email),
            provider: ProviderAddress(provider),
        }
    }

    /// Where this account syncs.
    pub fn provider(&self) -> &str {
        &self.provider.0
    }
}

/// The account confirmed its address and is served.
///
/// Its PRESENCE is what makes an account active; nothing has to overwrite
/// the registration to say so. Absence means not yet served, which is read
/// from the row being missing rather than from a status field claiming it.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct AccountActive {
    /// The immutable account subject.
    pub this: Entity,
    /// When activation was observed, unix seconds. Its presence is the
    /// whole signal — where the account syncs is on the registration,
    /// since that is where it was known.
    pub activated_at: ActivatedAt,
}

impl AccountActive {
    /// Record that `account` activated at `at`.
    pub fn new(account: Entity, at: u64) -> Self {
        Self {
            this: account,
            activated_at: ActivatedAt(at),
        }
    }
}

/// The service withdrew from serving this account.
///
/// Suspension wins outright: a suspended account keeps its registration and
/// its activation, and this row is what says nothing is served.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct AccountSuspended {
    /// The immutable account subject.
    pub this: Entity,
    /// When the withdrawal took effect, unix seconds.
    pub suspended_at: SuspendedAt,
    /// Why the service withdrew, in words a person can act on.
    pub reason: SuspensionReason,
}

impl AccountSuspended {
    /// Record that `account` was suspended at `at` for `reason`.
    pub fn new(account: Entity, reason: String, at: u64) -> Self {
        Self {
            this: account,
            suspended_at: SuspendedAt(at),
            reason: SuspensionReason(reason),
        }
    }
}

/// Where anything sealed for this account is addressed, keyed by the
/// immutable account subject.
///
/// A device that holds a space seed seals it to this address and
/// publishes the result as a [`SecretMessage`]; only a live passkey
/// ceremony derives the private half and can open one. So every device
/// can deposit and none can read, which is what lets a seed reach a new
/// device without any device holding the account's secret.
///
/// An address, not a store: nothing is kept here. Written by the
/// ceremony that creates the account secret, and again at rotation.
///
/// [`SecretMessage`]: crate::SecretMessage
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct AccountSealedInbox {
    /// The immutable account subject.
    pub this: Entity,
    /// The X25519 `did:key:z6LS…`.
    pub address: SealedInbox,
}

impl AccountSealedInbox {
    /// Publish `address` as where this account's sealed material goes.
    pub fn new(account: Entity, address: Entity) -> Self {
        Self {
            this: account,
            address: SealedInbox(address),
        }
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use dialog_operator::helpers;
    use dialog_query::{Output as _, Query, Term};
    use dialog_varsig::did;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;

    use super::*;
    use crate::prelude::DidExt as _;

    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_browser);

    async fn converge(a_first: bool) -> Result<String> {
        let (operator, profile) = helpers::test_operator_with_profile().await;
        let repository = helpers::test_repo(&operator, &profile).await;
        let base = repository.branch("base").open().perform(&operator).await?;
        let base_revision = base.transaction().commit().perform(&operator).await?;
        let a = repository
            .branch("replica-a")
            .open()
            .perform(&operator)
            .await?;
        let b = repository
            .branch("replica-b")
            .open()
            .perform(&operator)
            .await?;
        a.reset(base_revision.clone()).perform(&operator).await?;
        b.reset(base_revision).perform(&operator).await?;
        a.set_upstream(&b).perform(&operator).await?;
        b.set_upstream(&a).perform(&operator).await?;

        let account = did!("test:account").this();
        a.transaction()
            .assert(AccountDisplayName::new(account.clone(), "Amber".into()))
            .commit()
            .perform(&operator)
            .await?;
        b.transaction()
            .assert(AccountDisplayName::new(account.clone(), "Violet".into()))
            .commit()
            .perform(&operator)
            .await?;

        if a_first {
            a.pull().perform(&operator).await?;
            b.pull().perform(&operator).await?;
        } else {
            b.pull().perform(&operator).await?;
            a.pull().perform(&operator).await?;
        }
        a.pull().perform(&operator).await?;
        b.pull().perform(&operator).await?;

        let a_rows: Vec<AccountDisplayName> = a
            .query()
            .select(Query::<AccountDisplayName> {
                this: Term::from(account.clone()),
                name: Term::var("name"),
            })
            .perform(&operator)
            .try_vec()
            .await?;
        let b_rows: Vec<AccountDisplayName> = b
            .query()
            .select(Query::<AccountDisplayName> {
                this: Term::from(account.clone()),
                name: Term::var("name"),
            })
            .perform(&operator)
            .try_vec()
            .await?;
        let a_value = a_rows.into_iter().next().expect("one account name").name.0;
        let b_value = b_rows.into_iter().next().expect("one account name").name.0;
        assert_eq!(a_value, b_value);

        b.transaction()
            .assert(AccountDisplayName::new(account.clone(), "Cedar".into()))
            .commit()
            .perform(&operator)
            .await?;
        a.pull().perform(&operator).await?;
        let rows: Vec<AccountDisplayName> = a
            .query()
            .select(Query::<AccountDisplayName> {
                this: Term::from(account),
                name: Term::var("name"),
            })
            .perform(&operator)
            .try_vec()
            .await?;
        assert_eq!(
            rows.into_iter().next().expect("one account name").name.0,
            "Cedar"
        );
        Ok(a_value)
    }

    /// The three registration facts are independent: each resolves on its
    /// own, and an account carries whichever have happened.
    #[dialog_common::test]
    async fn it_reads_registration_activation_and_suspension_apart() -> Result<()> {
        let (operator, profile) = helpers::test_operator_with_profile().await;
        let repository = helpers::test_repo(&operator, &profile).await;
        let branch = repository.branch("main").open().perform(&operator).await?;
        let account = did!("test:account").this();

        // Enrolled and nothing more: registration resolves, activation
        // does not. Absence is the whole signal for "not yet served".
        branch
            .transaction()
            .assert(AccountRegistered::new(
                account.clone(),
                "person@example.com".into(),
                "https://service.example/ucan/".into(),
                100,
            ))
            .commit()
            .perform(&operator)
            .await?;

        let registered: Vec<AccountRegistered> = branch
            .query()
            .select(Query::<AccountRegistered> {
                this: Term::from(account.clone()),
                registered_at: Term::var("registered_at"),
                email: Term::var("email"),
                provider: Term::var("provider"),
            })
            .perform(&operator)
            .try_vec()
            .await?;
        assert_eq!(registered.len(), 1);
        assert_eq!(registered[0].email.0, "person@example.com");
        assert_eq!(
            registered[0].provider(),
            "https://service.example/ucan/",
            "enrollment names where the account syncs"
        );

        let active: Vec<AccountActive> = branch
            .query()
            .select(Query::<AccountActive> {
                this: Term::from(account.clone()),
                activated_at: Term::var("activated_at"),
            })
            .perform(&operator)
            .try_vec()
            .await?;
        assert!(active.is_empty(), "an enrolled account is not yet served");

        // Activation ADDS a row; it does not overwrite the registration.
        branch
            .transaction()
            .assert(AccountActive::new(account.clone(), 200))
            .commit()
            .perform(&operator)
            .await?;

        let active: Vec<AccountActive> = branch
            .query()
            .select(Query::<AccountActive> {
                this: Term::from(account.clone()),
                activated_at: Term::var("activated_at"),
            })
            .perform(&operator)
            .try_vec()
            .await?;
        assert_eq!(active.len(), 1);
        assert!(active[0].activated_at.0 > 0, "and when it happened");

        let registered: Vec<AccountRegistered> = branch
            .query()
            .select(Query::<AccountRegistered> {
                this: Term::from(account.clone()),
                registered_at: Term::var("registered_at"),
                email: Term::var("email"),
                provider: Term::var("provider"),
            })
            .perform(&operator)
            .try_vec()
            .await?;
        assert_eq!(
            registered.len(),
            1,
            "an activated account keeps its registration"
        );

        // Suspension composes too: a suspended account is still both
        // registered and activated, and this row is what says nothing is
        // served.
        branch
            .transaction()
            .assert(AccountSuspended::new(account.clone(), "unpaid".into(), 300))
            .commit()
            .perform(&operator)
            .await?;

        let suspended: Vec<AccountSuspended> = branch
            .query()
            .select(Query::<AccountSuspended> {
                this: Term::from(account.clone()),
                suspended_at: Term::var("suspended_at"),
                reason: Term::var("reason"),
            })
            .perform(&operator)
            .try_vec()
            .await?;
        assert_eq!(suspended.len(), 1);
        assert_eq!(suspended[0].reason.0, "unpaid");

        let active: Vec<AccountActive> = branch
            .query()
            .select(Query::<AccountActive> {
                this: Term::from(account),
                activated_at: Term::var("activated_at"),
            })
            .perform(&operator)
            .try_vec()
            .await?;
        assert_eq!(active.len(), 1, "suspension does not retract activation");
        Ok(())
    }

    /// Two devices recording the same registration converge on one row
    /// rather than racing: the old single `status` slot could be won by
    /// either writer, so an activated account could fall back to
    /// registered.
    #[dialog_common::test]
    async fn it_does_not_race_enrollment_against_activation() -> Result<()> {
        let (operator, profile) = helpers::test_operator_with_profile().await;
        let repository = helpers::test_repo(&operator, &profile).await;
        let branch = repository.branch("main").open().perform(&operator).await?;
        let account = did!("test:account").this();

        // Activation lands first, then a late re-enrollment: with one
        // status slot the second write would demote the account.
        branch
            .transaction()
            .assert(AccountActive::new(account.clone(), 200))
            .commit()
            .perform(&operator)
            .await?;
        branch
            .transaction()
            .assert(AccountRegistered::new(
                account.clone(),
                "person@example.com".into(),
                "https://service.example/ucan/".into(),
                100,
            ))
            .commit()
            .perform(&operator)
            .await?;

        let active: Vec<AccountActive> = branch
            .query()
            .select(Query::<AccountActive> {
                this: Term::from(account),
                activated_at: Term::var("activated_at"),
            })
            .perform(&operator)
            .try_vec()
            .await?;
        assert_eq!(
            active.len(),
            1,
            "a late enrollment cannot unmake an activation"
        );
        Ok(())
    }

    #[dialog_common::test]
    async fn it_converges_divergent_display_names_in_both_orders() -> Result<()> {
        let a_then_b = converge(true).await?;
        let b_then_a = converge(false).await?;

        // Order independence is the property that matters, and it is the one
        // this pins. Which of two concurrent names wins is dialog's
        // cardinality-one merge to decide, not wall-clock latest-write, so
        // asserting the specific winner would only pin that internal choice.
        assert_eq!(a_then_b, b_then_a);
        Ok(())
    }
}

/// The answer to "is this address registered?", on the profile
/// overlay.
///
/// Written by the provider behind `account/check-email` and read by the
/// registration form, which routes on it: create an account, sign in,
/// or say why neither is on offer. Overlay-only — the form asks while
/// the user types, and a durable row per answer would write a row per
/// keystroke into a branch that syncs.
///
/// `address` is carried alongside `state` so a form that has moved on
/// can tell an answer about what is currently typed from an answer
/// about what was typed two characters ago.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct EmailStatus {
    /// The entity the form watches, `state:email-status`.
    pub this: Entity,
    /// The address this answer is about.
    pub address: crate::domain::email_status::Address,
    /// One of `unregistered`, `active`, `pending`, `suspended`,
    /// `invalid`, `unavailable`.
    pub state: crate::domain::email_status::State,
}

/// The states an answer can take.
///
/// Shared vocabulary: the worker writes these strings and the
/// registration form routes on them, so they live with the concept
/// rather than as literals duplicated on each side.
///
/// The first four mirror what the access service's lookup answers (404
/// nothing registered, 200 active, 202 enrolled but unconfirmed, 410
/// suspended); the rest are the worker's own.
pub mod email_state {
    /// Nothing is registered under the address: offer to create.
    pub const UNREGISTERED: &str = "unregistered";
    /// Registered and served: offer to sign in.
    pub const ACTIVE: &str = "active";
    /// Enrolled, activation link unopened: sign in, then wait.
    pub const PENDING: &str = "pending";
    /// Service withdrawn. Neither creating nor signing in helps.
    pub const SUSPENDED: &str = "suspended";
    /// Not an address this can look up.
    pub const INVALID: &str = "invalid";
    /// The service could not be reached, so this says nothing about the
    /// address itself.
    pub const UNAVAILABLE: &str = "unavailable";
    /// A ceremony was raised for this address and has not finished.
    pub const PENDING_CEREMONY: &str = "registering";
    /// The lookup for this address is in flight.
    ///
    /// Written before the lookup rather than painted into the DOM by
    /// the form, so the row is the whole story: whatever is on screen
    /// is the latest answer about the latest address, and a late
    /// answer about an address the user has edited away from is
    /// recognisable as stale by the `address` beside it.
    pub const CHECKING: &str = "checking";

    /// Whether a state means the form should keep out of the way rather
    /// than offer an action.
    ///
    /// `registering` is a ceremony already up; `unavailable` is a
    /// service that did not answer. Neither is a fact about the
    /// address, so neither should render as "create an account" or
    /// "sign in".
    pub fn is_transient(state: &str) -> bool {
        matches!(state, PENDING_CEREMONY | UNAVAILABLE | CHECKING)
    }

    /// Whether an answer leaves the user something to do in the form.
    ///
    /// `suspended` is terminal and `invalid` is not an address, so
    /// neither offers a ceremony that would fail at the end.
    pub fn offers_action(state: &str) -> bool {
        matches!(state, UNREGISTERED | ACTIVE | PENDING)
    }
}

impl EmailStatus {
    /// The entity every answer is written to. One row, replaced on each
    /// answer: the form only ever cares about the latest one.
    pub const ENTITY: &str = "state:email-status";

    /// An answer about `address`.
    pub fn new(this: Entity, address: String, state: &str) -> Self {
        Self {
            this,
            address: crate::domain::email_status::Address(address),
            state: crate::domain::email_status::State(state.to_owned()),
        }
    }
}

#[cfg(test)]
mod email_state_tests {
    use super::email_state::*;

    /// A state the form should not turn into an offer.
    #[dialog_common::test]
    fn it_knows_which_states_carry_no_offer() {
        assert!(is_transient(PENDING_CEREMONY), "a ceremony is up");
        assert!(is_transient(UNAVAILABLE), "nobody answered");
        assert!(is_transient(CHECKING), "the lookup is still in flight");
        // These are answers about the address, so each names an action.
        assert!(!is_transient(UNREGISTERED));
        assert!(!is_transient(ACTIVE));
        assert!(!is_transient(PENDING));
        assert!(!is_transient(SUSPENDED));
    }

    /// What the form may offer to act on.
    ///
    /// `suspended` is the one that is an answer about the address and
    /// still carries no offer: the account exists but cannot host.
    #[dialog_common::test]
    fn it_offers_an_action_only_where_one_would_work() {
        assert!(offers_action(UNREGISTERED), "create");
        assert!(offers_action(ACTIVE), "sign in");
        assert!(offers_action(PENDING), "sign in, then confirm");
        assert!(!offers_action(SUSPENDED), "terminal");
        assert!(!offers_action(INVALID), "not an address");
        assert!(!offers_action(UNAVAILABLE), "says nothing about it");
        assert!(!offers_action(CHECKING), "no answer yet");
    }
}
