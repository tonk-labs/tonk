//! Effects evaluator — runs the inductive-rule fixpoint
//! against a transaction's overlay and retracts transient
//! claims, returning the post-induction [`Transaction`] to the
//! caller. Commit is the caller's choice.
//!
//! See `plan/effects.md` for the conceptual model.
//!
//! The public surface is [`TransactionExt::induce`], which
//! returns an [`Induce`] chain (mirroring dialog's
//! `Branch::commit(...)` pattern). Callers reach the
//! post-fixpoint transaction via `.perform(env).await`:
//!
//! ```ignore
//! use tonk_schema::effects::TransactionExt;
//!
//! let txn = branch.transaction()
//!     .assert(...)
//!     .induce(transients)        // <- run rules over the overlay
//!     .perform(env).await?;
//! let revision = txn.commit().perform(env).await?;
//! ```
//!
//! V1 is a skeleton: the fixpoint loop and transient sweep are
//! both no-ops. Effects evaluation lands incrementally; this
//! file establishes the public seam so worker, slide, and any
//! future caller route through the same surface.

use dialog_artifacts::{Changes, Instruction, Statement, Update, Value};
use dialog_capability::Provider;
use dialog_common::ConditionalSync;
use dialog_effects::archive::Get;
use dialog_effects::memory::Resolve;
use dialog_repository::{RemoteSite, Transaction};
use thiserror::Error;

/// Failure modes for [`Induce::perform`].
#[derive(Debug, Error)]
pub enum InduceError {
    /// The fixpoint ran past [`MAX_ROUNDS`] without quiescing.
    /// Indicates a cyclic or self-feeding inductive rule set.
    #[error("inductive rule set did not quiesce within {0} rounds")]
    NonTerminating(u32),
    /// A query against the transaction's overlay failed.
    #[error("query failed during induction: {0}")]
    Query(String),
}

/// Provider bound that the induction loop needs. Effects
/// evaluation queries the transaction's overlay (via
/// [`Transaction::query`]) for both rule definitions and rule
/// bodies, so it needs the same archive/resolve providers any
/// normal query does. Kept here as an alias so the public
/// chain stays compact.
pub trait InduceEnv:
    Provider<Get>
    + Provider<Resolve>
    + Provider<dialog_capability::Fork<RemoteSite, Get>>
    + Provider<dialog_capability::Fork<RemoteSite, Resolve>>
    + ConditionalSync
    + 'static
{
}

impl<T> InduceEnv for T where
    T: Provider<Get>
        + Provider<Resolve>
        + Provider<dialog_capability::Fork<RemoteSite, Get>>
        + Provider<dialog_capability::Fork<RemoteSite, Resolve>>
        + ConditionalSync
        + 'static
{
}

/// Extension trait that adds [`Self::induce`] to dialog's
/// [`Transaction`]. Imported at call sites to use the chain.
///
/// The lifetime parameter mirrors `Transaction<'a>` so the
/// returned [`Induce`] keeps the same borrow of the underlying
/// branch.
pub trait TransactionExt<'a> {
    /// Run the inductive-rule fixpoint against this
    /// transaction's overlay, then sweep the `transients`
    /// bucket. Returns a chain handle; call `.perform(env)` to
    /// execute.
    ///
    /// `transients` is the user-asserted transient claims
    /// already integrated into the transaction. The sweep
    /// emits a matching retract for each so the assert+retract
    /// pair cancels at commit.
    fn induce(self, transients: Changes) -> Induce<'a>;
}

impl<'a> TransactionExt<'a> for Transaction<'a> {
    fn induce(self, transients: Changes) -> Induce<'a> {
        Induce {
            txn: self,
            transients,
        }
    }
}

/// Chain handle for an induction pass. Holds the transaction
/// and the transient bucket until `.perform(env)` consumes
/// them.
pub struct Induce<'a> {
    txn: Transaction<'a>,
    transients: Changes,
}

impl<'a> Induce<'a> {
    /// Execute the induction pass.
    ///
    /// V1: the fixpoint loop is still a no-op. What lands here
    /// today is the transient sweep — every claim the caller
    /// integrated into the transient bucket gets a matching
    /// inverse-polarity claim emitted into the transaction so
    /// the assert+retract pair cancels at commit.
    ///
    /// The fixpoint loop lands incrementally on top of this.
    pub async fn perform<Env: InduceEnv>(self, env: &Env) -> Result<Transaction<'a>, InduceError> {
        let _env = env;
        let mut txn = self.txn;

        // TODO: fixpoint loop over user-defined inductive
        // effects. Each round retracts the round's own
        // transients before the next round runs; the final
        // sweep below handles user-asserted transients.

        // Sweep user-submitted transients. Each assert in the
        // bucket gets a matching retract; each retract gets a
        // matching assert. The result is the bucket cancels
        // itself out at the durable commit boundary.
        for instruction in self.transients.into_instructions() {
            txn = match instruction {
                Instruction::Assert(a) | Instruction::Replace(a) => txn.retract(RawClaim {
                    the: a.the,
                    of: a.of,
                    is: a.is,
                }),
                Instruction::Retract(a) => txn.assert(RawClaim {
                    the: a.the,
                    of: a.of,
                    is: a.is,
                }),
            };
        }

        Ok(txn)
    }
}

/// One concrete `(the, of, is)` triple wrapped as a
/// [`Statement`] so the transient sweep can hand it to
/// [`Transaction::assert`] / [`Transaction::retract`].
struct RawClaim {
    the: dialog_artifacts::Attribute,
    of: dialog_artifacts::Entity,
    is: Value,
}

impl Statement for RawClaim {
    fn assert(self, update: &mut impl Update) {
        update.associate(self.the, self.of, self.is);
    }
    fn retract(self, update: &mut impl Update) {
        update.dissociate(self.the, self.of, self.is);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dialog_artifacts::Entity;
    use dialog_query::{Output as _, Term, the};
    use dialog_repository::helpers::{test_operator_with_profile, test_repo};

    /// User-submitted transient assertions must cancel against
    /// the matching retracts the sweep emits — net effect after
    /// commit: nothing landed durably for those facts. This is
    /// the contract `/transact` relies on for transient
    /// concepts: assert+retract pair in one transaction means
    /// the concept's facts never reach durable storage.
    #[dialog_common::test]
    async fn it_cancels_transient_asserts_at_commit() -> anyhow::Result<()> {
        let (operator, profile) = test_operator_with_profile().await;
        let repo = test_repo(&operator, &profile).await;
        let branch = repo.branch("main").open().perform(&operator).await?;

        let subject: Entity = "did:key:zTransientSubject".parse()?;
        let attr = the!("xyz.tonk.command/subject");

        // Build a transient bucket holding one assertion.
        let mut transients = Changes::new();
        transients.assert(attr.clone().of(subject.clone()).is("hello".to_string()));

        // Drive the bucket through the sweep: integrate into a
        // transaction, then induce. The sweep retracts every
        // entry; integrate + retract cancels at commit.
        branch
            .transaction()
            .integrate(transients.clone())
            .induce(transients)
            .perform(&operator)
            .await
            .map_err(|e| anyhow::anyhow!("induce failed: {e}"))?
            .commit()
            .perform(&operator)
            .await?;

        // Query the branch directly: no transient claim should
        // be visible.
        let claims: Vec<dialog_query::Claim> = branch
            .query()
            .select(dialog_query::AttributeQuery::from(
                Term::from(attr)
                    .of(Term::from(subject.clone()))
                    .is(Term::<String>::var("v")),
            ))
            .perform(&operator)
            .try_vec()
            .await?;
        assert!(
            claims.is_empty(),
            "transient assert+retract should cancel; saw {claims:?}"
        );
        Ok(())
    }
}
