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

use dialog_artifacts::Changes;
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
    /// V1 skeleton: passes the transaction through unchanged.
    /// As the fixpoint loop and transient sweep land
    /// incrementally, this is the only public seam that needs
    /// to grow — callers won't have to change.
    pub async fn perform<Env: InduceEnv>(self, env: &Env) -> Result<Transaction<'a>, InduceError> {
        let _env = env;
        let _transients = self.transients;
        // TODO: fixpoint loop.
        // TODO: transient sweep.
        Ok(self.txn)
    }
}
