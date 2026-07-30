//! Root-owned facts stored in the hidden account repository.

use dialog_artifacts::Entity;
use dialog_query::Concept;

use crate::domain::account::DisplayName;

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

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use dialog_query::{Output as _, Query, Term};
    use dialog_repository::helpers;
    use dialog_varsig::did;

    use super::*;
    use crate::prelude::DidExt as _;

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
}
