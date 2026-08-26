//! Root-owned facts stored in the hidden account repository.

use dialog_artifacts::Entity;
use dialog_query::Concept;

use crate::domain::account::{
    CustomerEmail, CustomerStatus, DisplayName, PasskeyCreatedAt, PasskeyCreatedOn, ProviderAddress,
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

/// Facts Tonk recorded when it created this account's passkey, keyed by the
/// immutable account subject.
///
/// Informational only: no derivation, delegation, authorization, or revocation
/// path reads these. Both attributes are asserted in one transaction, so a
/// query requiring both never observes a half-written pair on one replica.
///
/// Merge is per attribute, not per concept: two replicas that recorded
/// *different* pairs converge on one value for each attribute independently,
/// which can pair one device's clock with another device's label. Only the
/// browser that ran `navigator.credentials.create()` ever records this
/// metadata — evaluating an existing passkey carries none — so one account has
/// at most one pair to converge and that mismatch has no way to arise. A
/// second recorded pair per account would need this keyed on the credential
/// instead of the account, which is where per-credential modelling belongs.
///
/// Derives `PartialOrd` but not `Ord`, because [`PasskeyCreatedAt`] wraps an
/// `f64` — the same shape `command::Invite` uses for its `TimeStamp`.
#[derive(Concept, Debug, Clone, PartialEq, PartialOrd)]
pub struct AccountPasskeyCreated {
    /// The immutable account subject.
    pub this: Entity,
    /// Unix seconds at credential creation.
    pub created_at: PasskeyCreatedAt,
    /// Browser and operating-system label where creation ran.
    pub created_on: PasskeyCreatedOn,
}

impl AccountPasskeyCreated {
    /// Record creation facts on the account subject.
    pub fn new(account: Entity, created_at: u64, created_on: String) -> Self {
        Self {
            this: account,
            created_at: PasskeyCreatedAt(created_at as f64),
            created_on: PasskeyCreatedOn(created_on),
        }
    }

    /// Unix seconds, back in the integer form the wire DTO carries.
    pub fn seconds(&self) -> u64 {
        self.created_at.0 as u64
    }
}

/// The account's registration with the access service, keyed by the
/// immutable account subject.
///
/// This is the account's registration state as a FACT, not as a cached
/// HTTP answer. Every device on the account reads it with an ordinary
/// query and converges on it through sync, so a device that never ran
/// the enrollment — and never probed the service — still knows whether
/// the account is servable.
///
/// Written at two moments: when enrollment records `Registered`, and
/// when activation is observed and promotes it to `Active`.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct AccountCustomer {
    /// The immutable account subject, which is the customer DID.
    pub this: Entity,
    /// One of `Registered`, `Active`, or `Suspended`.
    pub status: CustomerStatus,
    /// The address enrollment named.
    pub email: CustomerEmail,
    /// The provider serving this account: the UCAN access-service
    /// endpoint its spaces attach their remotes to.
    pub provider: ProviderAddress,
}

impl AccountCustomer {
    /// Record the account's registration state.
    pub fn new(account: Entity, status: &str, email: String, provider: String) -> Self {
        Self {
            this: account,
            status: CustomerStatus(status.to_owned()),
            email: CustomerEmail(email),
            provider: ProviderAddress(provider),
        }
    }

    /// The provider serving this account, absent when registration
    /// recorded none.
    pub fn provider(&self) -> Option<&str> {
        Some(self.provider.0.as_str()).filter(|address| !address.is_empty())
    }

    /// Whether the access service will serve this account's subjects.
    ///
    /// Only `Active` is servable: `Registered` awaits email activation
    /// and `Suspended` was withdrawn, and the provisioning gate refuses
    /// both. An unrecognised value — a status written by a newer build —
    /// reads as not servable, which fails closed.
    pub fn is_active(&self) -> bool {
        self.status.0 == "Active"
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

    #[dialog_common::test]
    async fn it_round_trips_passkey_creation_facts_on_the_account_subject() -> Result<()> {
        let (operator, profile) = helpers::test_operator_with_profile().await;
        let repository = helpers::test_repo(&operator, &profile).await;
        let branch = repository.branch("main").open().perform(&operator).await?;
        let account = did!("test:account").this();

        branch
            .transaction()
            .assert(AccountPasskeyCreated::new(
                account.clone(),
                1_754_380_800,
                "Chrome on macOS".into(),
            ))
            .commit()
            .perform(&operator)
            .await?;

        let rows: Vec<AccountPasskeyCreated> = branch
            .query()
            .select(Query::<AccountPasskeyCreated> {
                this: Term::from(account),
                created_at: Term::var("created_at"),
                created_on: Term::var("created_on"),
            })
            .perform(&operator)
            .try_vec()
            .await?;

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].seconds(), 1_754_380_800);
        assert_eq!(rows[0].created_on.0, "Chrome on macOS");
        Ok(())
    }

    /// Record `first` on one replica and `second` on the other, exchange them
    /// in the given order, and report the single pair that survived.
    async fn converge_passkey(
        a_first: bool,
        first: (u64, &str),
        second: (u64, &str),
    ) -> Result<(u64, String)> {
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
            .assert(AccountPasskeyCreated::new(
                account.clone(),
                first.0,
                first.1.to_string(),
            ))
            .commit()
            .perform(&operator)
            .await?;
        b.transaction()
            .assert(AccountPasskeyCreated::new(
                account.clone(),
                second.0,
                second.1.to_string(),
            ))
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

        let rows: Vec<AccountPasskeyCreated> = a
            .query()
            .select(Query::<AccountPasskeyCreated> {
                this: Term::from(account),
                created_at: Term::var("created_at"),
                created_on: Term::var("created_on"),
            })
            .perform(&operator)
            .try_vec()
            .await?;
        assert_eq!(rows.len(), 1, "an account has one passkey creation moment");
        let row = rows.into_iter().next().expect("one creation fact");
        Ok((row.seconds(), row.created_on.0))
    }

    #[dialog_common::test]
    async fn it_keeps_one_passkey_creation_fact_per_account() -> Result<()> {
        let recorded = (1_754_380_800, "Chrome on macOS");

        // Two devices seeding the same recorded pair is the only concurrency
        // this fact can actually see: the seed reads the account space first
        // and only the browser that created the passkey holds metadata to
        // contribute, so every writer contributes the same pair.
        let a_then_b = converge_passkey(true, recorded, recorded).await?;
        let b_then_a = converge_passkey(false, recorded, recorded).await?;
        assert_eq!(a_then_b, (recorded.0, recorded.1.to_string()));
        assert_eq!(b_then_a, (recorded.0, recorded.1.to_string()));

        // Divergent pairs converge on one value per attribute, in an order
        // the merge decides, but *independently* — so the surviving row can
        // pair one write's time with the other's label. Nothing asserts a
        // second pair today; this pins the behaviour that per-credential
        // modelling would have to answer for before one could.
        let divergent = (1_600_000_000, "Safari on iOS");
        assert_eq!(
            converge_passkey(true, recorded, divergent).await?,
            converge_passkey(false, recorded, divergent).await?,
            "concurrent creation facts converge regardless of merge order"
        );
        Ok(())
    }
}
