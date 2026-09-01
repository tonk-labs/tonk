//! A passkey that can recover an account.

use dialog_artifacts::Entity;
use dialog_query::Concept;
use dialog_varsig::Did;

use crate::domain::recovery::{CreatedAt, CreatedOn, CredentialId, DisplayName, Name};
use crate::prelude::*;

/// A passkey that can recover this account, keyed on the custody DID its
/// PRF output derives.
///
/// One row per passkey, which is the point. These facts used to hang off the
/// ACCOUNT, so an account with three passkeys had one creation record and
/// dialog's per-attribute merge could pair one device's timestamp with
/// another's label — the flaw that concept's own doc admitted and deferred.
///
/// The same entity is the `to` of the [`SecretMessage`] holding this
/// passkey's sealed envelope, so the two join on `this`. There is
/// deliberately no `account` field: that message already names the account
/// as its `sender`, and a second copy could disagree with it.
///
/// Enumerating an account's passkeys therefore means querying
/// [`SecretMessage`] by `sender` and reading the `to`s. The custody DID is
/// PRF-derived, so nothing can compute it without a live assertion — which
/// is why the link is a stored join rather than a derivation.
///
/// Informational: no derivation, delegation, authorization, or revocation
/// path reads these. All three attributes are asserted in one transaction,
/// so a query requiring them never observes a half-written row.
///
/// [`SecretMessage`]: crate::SecretMessage
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct RecoveryPasskey {
    /// The custody DID the passkey's PRF output derives.
    pub this: Entity,
    /// The credential id the authenticator returns.
    pub credential_id: CredentialId,
    /// Unix seconds at credential creation.
    pub created_at: CreatedAt,
    /// Browser and operating-system label where creation ran.
    pub created_on: CreatedOn,
    /// The WebAuthn `user.name`, when the ceremony was given one. Absent
    /// for a credential created without an address, which the ceremony
    /// names with an opaque random string instead.
    pub name: Option<Name>,
    /// The WebAuthn `user.displayName`, when one was given.
    pub display_name: Option<DisplayName>,
}

impl RecoveryPasskey {
    /// Describe the passkey whose PRF output derives `custody`.
    pub fn new(
        custody: &Did,
        credential_id: impl Into<String>,
        created_at: u64,
        created_on: impl Into<String>,
    ) -> Self {
        Self {
            this: custody.this(),
            credential_id: CredentialId(credential_id.into()),
            created_at: CreatedAt(created_at),
            created_on: CreatedOn(created_on.into()),
            name: None,
            display_name: None,
        }
    }

    /// The same passkey, recording what a passkey manager lists it under.
    pub fn named(mut self, name: impl Into<String>, display_name: impl Into<String>) -> Self {
        self.name = Some(Name(name.into()));
        self.display_name = Some(DisplayName(display_name.into()));
        self
    }

    /// Unix seconds, in the integer form the wire DTO carries.
    pub fn seconds(&self) -> u64 {
        self.created_at.0
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
    use crate::SecretMessage;

    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_browser);

    /// A passkey is found by the custody DID it derives, and carries the
    /// credential id an assertion needs to select it.
    #[dialog_common::test]
    async fn it_finds_a_passkey_by_the_custody_it_derives() -> Result<()> {
        let (operator, profile) = helpers::test_operator_with_profile().await;
        let repository = helpers::test_repo(&operator, &profile).await;
        let branch = repository.branch("main").open().perform(&operator).await?;
        let custody = did!("test:custody-one");

        branch
            .transaction()
            .assert(RecoveryPasskey::new(
                &custody,
                "credential-one",
                1_754_380_800,
                "Chrome on macOS",
            ))
            .commit()
            .perform(&operator)
            .await?;

        let rows: Vec<RecoveryPasskey> = branch
            .query()
            .select(Query::<RecoveryPasskey> {
                this: Term::from(custody.this()),
                credential_id: Term::var("credential_id"),
                created_at: Term::var("created_at"),
                created_on: Term::var("created_on"),
                name: Term::var("name"),
                display_name: Term::var("display_name"),
            })
            .perform(&operator)
            .try_vec()
            .await?;

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].credential_id.0, "credential-one");
        assert_eq!(rows[0].seconds(), 1_754_380_800);
        assert_eq!(rows[0].created_on.0, "Chrome on macOS");
        Ok(())
    }

    /// Two passkeys are two rows, each with its own creation moment.
    ///
    /// This is what the account-keyed shape could not do: one record per
    /// account meant a second passkey either overwrote the first or merged
    /// with it per attribute, pairing one device's clock with another's
    /// label.
    #[dialog_common::test]
    async fn it_keeps_one_row_per_passkey() -> Result<()> {
        let (operator, profile) = helpers::test_operator_with_profile().await;
        let repository = helpers::test_repo(&operator, &profile).await;
        let branch = repository.branch("main").open().perform(&operator).await?;

        branch
            .transaction()
            .assert(RecoveryPasskey::new(
                &did!("test:custody-one"),
                "credential-one",
                100,
                "Chrome on macOS",
            ))
            .assert(RecoveryPasskey::new(
                &did!("test:custody-two"),
                "credential-two",
                200,
                "Safari on iOS",
            ))
            .commit()
            .perform(&operator)
            .await?;

        let rows: Vec<RecoveryPasskey> = branch
            .query()
            .select(Query::<RecoveryPasskey> {
                this: Term::var("this"),
                credential_id: Term::var("credential_id"),
                created_at: Term::var("created_at"),
                created_on: Term::var("created_on"),
                name: Term::var("name"),
                display_name: Term::var("display_name"),
            })
            .perform(&operator)
            .try_vec()
            .await?;

        let mut pairs: Vec<(u64, String)> = rows
            .into_iter()
            .map(|row| (row.seconds(), row.created_on.0))
            .collect();
        pairs.sort();
        assert_eq!(
            pairs,
            vec![
                (100, "Chrome on macOS".to_string()),
                (200, "Safari on iOS".to_string()),
            ],
            "each passkey keeps its own clock and label"
        );
        Ok(())
    }

    /// The account link is a join through the sealed envelope, not a field.
    ///
    /// The message holding a passkey's envelope names that passkey's custody
    /// DID as `to` and the account as `sender`, so an account's passkeys are
    /// found by querying messages rather than by a second copy of the
    /// account on every row.
    #[dialog_common::test]
    async fn it_reaches_the_account_through_the_sealed_envelope() -> Result<()> {
        let (operator, profile) = helpers::test_operator_with_profile().await;
        let repository = helpers::test_repo(&operator, &profile).await;
        let branch = repository.branch("main").open().perform(&operator).await?;
        let account = did!("test:account");
        let mine = did!("test:custody-one");
        let theirs = did!("test:custody-two");

        branch
            .transaction()
            .assert(SecretMessage::new(&mine, vec![1]).sealed_by(&account))
            .assert(SecretMessage::new(&theirs, vec![2]).sealed_by(&did!("test:other")))
            .assert(RecoveryPasskey::new(&mine, "credential-one", 100, "Chrome"))
            .assert(RecoveryPasskey::new(
                &theirs,
                "credential-two",
                200,
                "Safari",
            ))
            .commit()
            .perform(&operator)
            .await?;

        // Every envelope this account sealed names one of its passkeys.
        // `from` is optional on the concept, so it binds as a variable and
        // the sender is matched here rather than in the query.
        let all: Vec<SecretMessage> = branch
            .query()
            .select(Query::<SecretMessage> {
                this: Term::var("this"),
                to: Term::var("to"),
                message: Term::var("message"),
                from: Term::var("from"),
            })
            .perform(&operator)
            .try_vec()
            .await?;
        let envelopes: Vec<SecretMessage> = all
            .into_iter()
            .filter(|row| {
                row.from
                    .as_ref()
                    .is_some_and(|sender| sender.0 == account.this())
            })
            .collect();
        assert_eq!(envelopes.len(), 1, "one envelope from this account");

        let rows: Vec<RecoveryPasskey> = branch
            .query()
            .select(Query::<RecoveryPasskey> {
                this: Term::from(envelopes[0].to.0.clone()),
                credential_id: Term::var("credential_id"),
                created_at: Term::var("created_at"),
                created_on: Term::var("created_on"),
                name: Term::var("name"),
                display_name: Term::var("display_name"),
            })
            .perform(&operator)
            .try_vec()
            .await?;
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].credential_id.0, "credential-one",
            "the join reaches this account's passkey and not the other"
        );
        Ok(())
    }
}
