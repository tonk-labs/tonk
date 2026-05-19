//! Reactor-side effects evaluator.
//!
//! Sits inside [`Commit::perform`](super::Commit::perform) between the
//! user's commit changes and the durable write. The flow per commit:
//!
//! 1. Build a [`Transaction`] from the user's [`Changes`].
//! 2. [`evaluate_effects`] runs the fixpoint loop:
//!     - Each round, find effects whose body could be affected by the
//!       current transaction state (via the
//!       [`effects_by_premise`](tonk_schema::effect::effects_by_premise)
//!       attribute-keyed reverse index).
//!     - Evaluate each rule's body against the transaction (which sees
//!       both committed branch state and pending writes through
//!       [`Transaction::query`]).
//!     - For each binding, instantiate the head and integrate into the
//!       transaction (assert for assert-polarity effects, retract for
//!       retract-polarity).
//!     - Stop when a round produces no new changes, or when the depth
//!       limit is hit.
//! 3. [`retract_transients`] queries the transaction for facts of
//!    transient concepts and retracts each. The assert+retract pair
//!    cancels at commit, so transient facts never reach durable
//!    storage.
//!
//! V1 keeps the evaluator simple: no rule registration, no rule
//! evaluation, no transient discovery. The skeleton ships first; the
//! real logic lands incrementally.

use dialog_repository::{Branch, Transaction};

use super::env::SelectProvider;
use super::error::ReactorError;

/// Maximum fixpoint depth. A pathological effect set that keeps
/// producing new facts will hit this bound; we log a warning and
/// stop. Real cascades should settle in single-digit rounds.
#[allow(dead_code)]
const MAX_DEPTH: u32 = 16;

/// Run the effects fixpoint loop against the given transaction.
///
/// V1 skeleton: passes the transaction through unchanged. The real
/// loop will land incrementally as the supporting machinery
/// (rule-body evaluation against a transaction, head instantiation
/// from bindings) lands.
pub(super) async fn evaluate_effects<'a, Env>(
    _branch: &'a Branch,
    txn: Transaction<'a>,
    _env: &Env,
) -> Result<Transaction<'a>, ReactorError>
where
    Env: SelectProvider,
{
    // TODO: implement fixpoint loop.
    Ok(txn)
}

/// Retract every fact in the transaction whose attribute is owned
/// by a transient concept. The retraction pairs with the assertion
/// already in the transaction; at commit they cancel, so transient
/// facts never enter durable storage.
///
/// V1 skeleton: passes the transaction through unchanged. The real
/// implementation will query the transaction for transient-attribute
/// facts (using [`Transaction::query`] so it sees pending writes
/// alongside branch state) and retract each.
pub(super) async fn retract_transients<'a, Env>(
    _branch: &'a Branch,
    txn: Transaction<'a>,
    _env: &Env,
) -> Result<Transaction<'a>, ReactorError>
where
    Env: SelectProvider,
{
    // TODO: query transient-attribute facts via txn.query(), retract each.
    Ok(txn)
}
