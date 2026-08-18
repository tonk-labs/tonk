//! A branch-or-transaction query seam.
//!
//! `dialog-repository` exposes two staged query types with no shared
//! trait: [`SelectQuery`] (from `branch.query().select(q)`) and
//! [`TransactionSelectQuery`] (from `txn.query().select(q)`). Both
//! carry a `.perform(env)` that runs the query and yields an
//! `impl Output<Q::Conclusion>`, but the two are distinct concrete
//! types — code that wants to run "the same query against a branch
//! *or* a transaction overlay" cannot abstract over them.
//!
//! [`QuerySource`] is that abstraction: a tonk-local enum unifying
//! the two staged types. `From` lifts either staged query into it,
//! and [`QuerySource::perform`] dispatches to the underlying
//! `perform`, reconciling the two distinct `impl Output` return
//! types behind a single [`QueryStream`] type.
//!
//! # The return-type reconciliation
//!
//! `SelectQuery::perform` and `TransactionSelectQuery::perform`
//! return *different* concrete `impl Output<Q::Conclusion>` types,
//! so a bare `match` over `-> impl Output<..>` will not type-check —
//! the arms produce distinct opaque types.
//!
//! [`Output`] is a blanket trait: anything that is a
//! `Stream<Item = Result<T, EvaluationError>> + ConditionalSend`
//! *is* an `Output<T>`. So the unifying type only needs to be a
//! [`Stream`] of the right item type and it gets `Output` for free.
//! [`QueryStream`] is a type alias for [`futures_util::future::Either`]
//! of the two opaque streams: `Either` already carries a safe,
//! audited `Stream` impl that delegates `poll_next` per arm — so the
//! result is allocation-free and monomorphic, no boxing and no
//! hand-rolled `unsafe` pin projection. See the change report for
//! why this (option #1, result enum) was preferred over `Box<dyn>`.

use dialog_query::query::{Application, Output};
use dialog_repository::{Branch, QueryLayer, SelectQuery, Transaction, TransactionSelectQuery};
use futures_util::future::Either;

use crate::concept::QueryEnv;

/// A staged query against either a branch or a transaction overlay.
///
/// Construct one via [`From`] of a [`SelectQuery`] (branch side) or a
/// [`TransactionSelectQuery`] (transaction side), then call
/// [`perform`](Self::perform) to run it.
pub enum QuerySource<'a, Q> {
    /// A query staged against a branch session.
    Branch(SelectQuery<'a, Q>),
    /// A query staged against a transaction's pending-writes overlay.
    Transaction(TransactionSelectQuery<'a, Q>),
}

/// The result stream of [`QuerySource::perform`].
///
/// An [`Either`] of the two underlying `impl Output` streams — the
/// branch side in `Left`, the transaction side in `Right`. `Either`
/// is a [`Stream`](dialog_query::query::Stream) whenever both arms
/// are streams of the same item, so via the blanket impl in
/// `dialog-query` this is an [`Output`] with no boxing.
///
/// `B` and `T` are the two opaque `impl Output` types; they are
/// inferred at the [`QuerySource::perform`] call site.
pub type QueryStream<B, T> = Either<B, T>;

impl<'a, Q> From<SelectQuery<'a, Q>> for QuerySource<'a, Q> {
    fn from(query: SelectQuery<'a, Q>) -> Self {
        Self::Branch(query)
    }
}

impl<'a, Q> From<TransactionSelectQuery<'a, Q>> for QuerySource<'a, Q> {
    fn from(query: TransactionSelectQuery<'a, Q>) -> Self {
        Self::Transaction(query)
    }
}

/// An *unstaged* query source — a branch or a transaction overlay —
/// that resolution code holds in place of a bare `&Branch`.
///
/// Where [`QuerySource`] is a single staged query, `Source` is the
/// thing you stage queries *against*: call [`select`](Self::select)
/// once per lookup to get a fresh [`QuerySource`], then `.perform`.
///
/// This is what lets `resolution.rs` (and the `concept.rs` builders
/// it composes) resolve a definition against either a committed
/// branch or a transaction's pending-writes overlay, without each
/// builder having to know which.
///
/// # Why the branch arm carries a [`QueryLayer`]
///
/// `dialog-repository`'s `branch.query().select(q)` ties the staged
/// `SelectQuery`'s lifetime to the intermediate `QueryLayer`. To
/// hand back a `QuerySource` that borrows `self` rather than a
/// dropped temporary, `Source` owns the `QueryLayer` (cheap — it is
/// `Clone`, holding only branch references and an empty `Changes`).
/// The transaction arm needs no such storage: `TransactionQuery`
/// snapshots its `Changes` into the staged query, so a temporary
/// suffices there.
#[derive(Clone)]
pub enum Source<'a> {
    /// Resolve against a committed branch.
    Branch(QueryLayer<'a>),
    /// Resolve against a transaction's "as-if committed" view.
    Transaction(&'a Transaction<'a>),
}

impl<'a> From<&'a Branch> for Source<'a> {
    fn from(branch: &'a Branch) -> Self {
        Self::Branch(branch.query())
    }
}

impl<'a> From<&'a Transaction<'a>> for Source<'a> {
    fn from(transaction: &'a Transaction<'a>) -> Self {
        Self::Transaction(transaction)
    }
}

impl<'a> From<&Source<'a>> for Source<'a> {
    fn from(source: &Source<'a>) -> Self {
        source.clone()
    }
}

impl Source<'_> {
    /// Stage a query against this source. The resulting
    /// [`QuerySource`] is run with `.perform(env)`.
    ///
    /// The returned handle borrows `self`: on the branch side the
    /// staged `SelectQuery` is cloned out of the stored
    /// [`QueryLayer`], and on the transaction side
    /// `TransactionQuery::select` snapshots the pending changes.
    pub fn select<Q: Application>(&self, query: Q) -> QuerySource<'_, Q> {
        match self {
            Self::Branch(layer) => QuerySource::Branch(layer.select(query)),
            Self::Transaction(transaction) => {
                QuerySource::Transaction(transaction.query().select(query))
            }
        }
    }
}

impl<'a, Q: Application> QuerySource<'a, Q> {
    /// Run the staged query against `env`, yielding a stream of
    /// results.
    ///
    /// The `Env` bound is the union of what both underlying
    /// `perform` methods require — which is exactly tonk-schema's
    /// [`QueryEnv`]. (`SelectQuery::perform` additionally needs
    /// `Provider<Identify>` for its auto-injected session metadata;
    /// `TransactionSelectQuery::perform` does not, but requiring it
    /// uniformly costs nothing — every real environment provides it.)
    ///
    /// The two arms produce distinct opaque `impl Output` types; the
    /// returned [`QueryStream`] unifies them and is itself an
    /// [`Output`].
    pub fn perform<Env: QueryEnv>(
        self,
        env: &'a Env,
    ) -> QueryStream<impl Output<Q::Conclusion> + 'a, impl Output<Q::Conclusion> + 'a> {
        match self {
            Self::Branch(query) => Either::Left(query.perform(env)),
            Self::Transaction(query) => Either::Right(query.perform(env)),
        }
    }
}

#[cfg(test)]
mod tests {
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_dedicated_worker);

    use dialog_artifacts::Entity;
    use dialog_operator::helpers::{test_operator_with_profile, test_repo};
    use dialog_query::query::Output as _;
    use dialog_query::{Concept, Query, Term};

    use super::{QuerySource, Source};

    /// `test/name` attribute used by the `Person` concept fixture.
    mod people {
        /// The person's name.
        #[derive(dialog_query::Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
        #[domain("test")]
        pub struct Name(
            /// The name string.
            pub String,
        );
    }

    /// A minimal concept used to exercise the query seam.
    #[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
    pub struct Person {
        /// The person entity.
        pub this: Entity,
        /// Their `test/name`.
        pub name: people::Name,
    }

    #[dialog_common::test]
    async fn it_matches_a_direct_branch_query_when_performed_through_query_source()
    -> anyhow::Result<()> {
        let (operator, profile) = test_operator_with_profile().await;
        let repo = test_repo(&operator, &profile).await;
        let branch = repo.branch("main").open().perform(&operator).await?;

        let alice: Entity = "id:alice".parse()?;
        branch
            .transaction()
            .assert(Person {
                this: alice.clone(),
                name: people::Name("Alice".into()),
            })
            .commit()
            .perform(&operator)
            .await?;

        let query = Query::<Person> {
            this: alice.clone().into(),
            name: Term::var("name"),
        };

        // Direct: branch.query().select(q).perform(env).
        let direct: Vec<Person> = branch
            .query()
            .select(query.clone())
            .perform(&operator)
            .try_vec()
            .await?;

        // Through the seam: Source::Branch -> QuerySource::perform.
        let via_source: Vec<Person> = Source::from(&branch)
            .select(query)
            .perform(&operator)
            .try_vec()
            .await?;

        assert_eq!(direct, via_source);
        assert_eq!(via_source.len(), 1);
        assert_eq!(via_source[0].name.0, "Alice");
        Ok(())
    }

    #[dialog_common::test]
    async fn it_surfaces_pending_writes_when_performed_through_a_transaction_source()
    -> anyhow::Result<()> {
        let (operator, profile) = test_operator_with_profile().await;
        let repo = test_repo(&operator, &profile).await;
        let branch = repo.branch("main").open().perform(&operator).await?;

        let alice: Entity = "id:alice".parse()?;
        // Uncommitted transaction — the assert lives only in the overlay.
        let txn = branch.transaction().assert(Person {
            this: alice.clone(),
            name: people::Name("Alice".into()),
        });

        let query = Query::<Person> {
            this: alice.clone().into(),
            name: Term::var("name"),
        };

        let via_source: Vec<Person> = Source::from(&txn)
            .select(query)
            .perform(&operator)
            .try_vec()
            .await?;

        assert_eq!(via_source.len(), 1);
        assert_eq!(via_source[0].this, alice);
        assert_eq!(via_source[0].name.0, "Alice");
        Ok(())
    }

    #[dialog_common::test]
    async fn it_lifts_both_staged_query_types_via_from() -> anyhow::Result<()> {
        let (operator, profile) = test_operator_with_profile().await;
        let repo = test_repo(&operator, &profile).await;
        let branch = repo.branch("main").open().perform(&operator).await?;

        let query = Query::<Person> {
            this: Term::var("this"),
            name: Term::var("name"),
        };

        // From<SelectQuery>.
        let branch_layer = branch.query();
        let _: QuerySource<'_, _> = branch_layer.select(query.clone()).into();

        // From<TransactionSelectQuery>.
        let txn = branch.transaction();
        let _: QuerySource<'_, _> = txn.query().select(query).into();
        Ok(())
    }
}
