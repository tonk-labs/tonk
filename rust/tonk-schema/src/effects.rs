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
//! post-fixpoint transaction via `.perform(env).await`. All
//! reads go through [`Transaction::query`] so rules see
//! branch state union pending writes from the same commit; no
//! `&Branch` is needed at the boundary.
//!
//! ```ignore
//! use tonk_schema::effects::TransactionExt;
//!
//! let txn = branch.transaction()
//!     .assert(...)
//!     .induce(transients)
//!     .perform(env).await?;
//! let revision = txn.commit().perform(env).await?;
//! ```

use std::collections::BTreeSet;

use dialog_artifacts::{Changes, Entity, Instruction, Statement, Update, Value};
use dialog_capability::{Fork, Provider};
use dialog_common::ConditionalSync;
use dialog_effects::archive::{Get, Put};
use dialog_effects::authority::Identify;
use dialog_effects::memory::Resolve;
use dialog_query::{Output as _, Term};
use dialog_repository::{RemoteSite, Transaction};
use thiserror::Error;

use crate::effect::{Effect, EffectError, EffectPolarity};

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

/// Provider bound the induction loop needs. Effect lookup
/// (reverse index, `Effect::by_entity`) queries the branch;
/// rule-body evaluation queries the transaction overlay via
/// [`Transaction::query`]. Both routes share the same
/// archive/resolve provider set.
pub trait InduceEnv:
    Provider<Get>
    + Provider<Put>
    + Provider<Resolve>
    + Provider<Identify>
    + Provider<Fork<RemoteSite, Get>>
    + Provider<Fork<RemoteSite, Resolve>>
    + ConditionalSync
    + 'static
{
}

impl<T> InduceEnv for T where
    T: Provider<Get>
        + Provider<Put>
        + Provider<Resolve>
        + Provider<Identify>
        + Provider<Fork<RemoteSite, Get>>
        + Provider<Fork<RemoteSite, Resolve>>
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
    /// Execute the induction pass: run the fixpoint loop, then
    /// sweep user-submitted transients.
    ///
    /// All reads go through [`Transaction::query`] so the
    /// overlay (branch state union pending writes) is the
    /// effect-lookup source. Rules can therefore react to
    /// effects installed in the same commit, and to transients
    /// submitted in the same commit.
    ///
    /// V1 limits:
    ///
    /// - Single round only (no cascade yet). A future revision
    ///   wraps this in a loop terminated on "no transients
    ///   emitted" or `MAX_ROUNDS`.
    /// - Body evaluation isn't wired yet (see [`fire_effect`]).
    /// - Retract-polarity effects aren't dispatched yet.
    pub async fn perform<Env: InduceEnv>(self, env: &Env) -> Result<Transaction<'a>, InduceError> {
        let Induce {
            mut txn,
            transients,
        } = self;

        // Collect attribute names touched by user-submitted
        // transients. The reverse index is keyed on
        // `on:<domain>/<name>` URIs, recoverable from the
        // runtime `Attribute(String)` form alone — no schema
        // lookup needed.
        let attribute_names: BTreeSet<String> = transients
            .clone()
            .into_instructions()
            .into_iter()
            .map(|inst| match inst {
                Instruction::Assert(a) | Instruction::Replace(a) | Instruction::Retract(a) => {
                    a.the.to_string()
                }
            })
            .collect();

        // Walk the reverse index per touched attribute to find
        // candidate effects, union and dedupe.
        let mut effect_entities: BTreeSet<Entity> = BTreeSet::new();
        for name in &attribute_names {
            let hits = effects_on(&txn, name, env).await?;
            effect_entities.extend(hits);
        }

        // Load and fire each candidate.
        for entity in effect_entities {
            let Some(effect) = load_effect(&txn, entity, env).await? else {
                // The reverse index pointed at an entity whose
                // source claim is missing or unparseable. Skip
                // — the install path is supposed to keep these
                // in sync, and we'd rather drop a bad effect
                // than fail the commit.
                continue;
            };
            txn = fire_effect(effect, txn, env).await?;
        }

        // Sweep user-submitted transients. Each assert in the
        // bucket gets a matching retract; each retract gets a
        // matching assert. The pair cancels at the durable
        // commit boundary.
        for instruction in transients.into_instructions() {
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

/// Parse a `<domain>/<name>` pair into the typed
/// [`dialog_query::attribute::The`] form. Mirrors the helper in
/// [`crate::effect`] so the two modules share a single style of
/// building dialog meta-attribute selectors.
fn the(domain: &str, name: &str) -> dialog_query::attribute::The {
    format!("{domain}/{name}")
        .parse()
        .expect("dialog meta-attribute names are always valid")
}

/// Query the transaction's overlay for effect entities whose
/// `dialog.effect/on` index lists the given attribute name.
/// Equivalent to [`effects_by_on`](crate::effect::effects_by_on)
/// but reads through the transaction so in-flight effect
/// installs and retracts are visible.
async fn effects_on<Env: InduceEnv>(
    txn: &Transaction<'_>,
    attribute_name: &str,
    env: &Env,
) -> Result<Vec<Entity>, InduceError> {
    let attribute_entity: Entity = format!("on:{attribute_name}")
        .parse()
        .expect("on:<domain>/<name> is a valid entity URI");

    let claims: Vec<dialog_query::Claim> = txn
        .query()
        .select(dialog_query::AttributeQuery::from(
            Term::<dialog_query::attribute::The>::from(the("dialog.effect", "on"))
                .of(Term::<Entity>::var("effect"))
                .is(Term::<Entity>::from(attribute_entity)),
        ))
        .perform(env)
        .try_vec()
        .await
        .map_err(|e| InduceError::Query(format!("on-index query failed: {e:?}")))?;

    let mut out: Vec<Entity> = claims.into_iter().map(|c| c.of).collect();
    out.sort();
    out.dedup();
    Ok(out)
}

/// Query the transaction's overlay for an effect's `source`
/// and `polarity` claims, rehydrating it. Mirrors
/// [`Effect::by_entity`](crate::effect::Effect::by_entity)'s
/// resolve path but reads through the transaction.
async fn load_effect<Env: InduceEnv>(
    txn: &Transaction<'_>,
    entity: Entity,
    env: &Env,
) -> Result<Option<Effect>, InduceError> {
    let source_claims: Vec<dialog_query::Claim> = txn
        .query()
        .select(dialog_query::AttributeQuery::from(
            Term::<dialog_query::attribute::The>::from(the("dialog.effect", "source"))
                .of(Term::<Entity>::from(entity.clone()))
                .is(Term::<String>::var("source")),
        ))
        .perform(env)
        .try_vec()
        .await
        .map_err(|e| InduceError::Query(format!("effect source query failed: {e:?}")))?;

    let Some(source_claim) = source_claims.into_iter().next() else {
        return Ok(None);
    };
    let source = match source_claim.is {
        Value::String(s) => s,
        other => {
            return Err(InduceError::Query(format!(
                "dialog.effect/source was not a string: {other:?}"
            )));
        }
    };

    let polarity_claims: Vec<dialog_query::Claim> = txn
        .query()
        .select(dialog_query::AttributeQuery::from(
            Term::<dialog_query::attribute::The>::from(the("dialog.effect", "polarity"))
                .of(Term::<Entity>::from(entity))
                .is(Term::<String>::var("polarity")),
        ))
        .perform(env)
        .try_vec()
        .await
        .map_err(|e| InduceError::Query(format!("effect polarity query failed: {e:?}")))?;

    let polarity_claim = polarity_claims
        .into_iter()
        .next()
        .ok_or_else(|| InduceError::Query("missing dialog.effect/polarity".to_string()))?;
    let polarity_str = match polarity_claim.is {
        Value::String(s) => s,
        _ => {
            return Err(InduceError::Query(
                "dialog.effect/polarity was not a string".to_string(),
            ));
        }
    };
    let polarity = EffectPolarity::parse(&polarity_str)
        .ok_or_else(|| InduceError::Query(format!("invalid polarity {polarity_str:?}")))?;

    let effect = Effect::from_source(&source, polarity).map_err(|e: EffectError| match e {
        EffectError::Deserialize(msg) => {
            InduceError::Query(format!("effect source deserialize failed: {msg}"))
        }
        other => InduceError::Query(format!("effect rehydrate failed: {other}")),
    })?;
    Ok(Some(effect))
}

/// Evaluate one effect's body against the transaction overlay,
/// instantiate the head per match, and apply to the transaction.
///
/// **Body evaluation is currently a no-op.** Dialog's
/// `Conjunction::evaluate(selection, env)` needs an env that
/// provides `Provider<Select> + Provider<SelectRules>`. The
/// plain operator env doesn't — those wrappers are constructed
/// internally by `Transaction::query()`. To run a `Conjunction`
/// against the overlay, we need to wrap it in a custom
/// `dialog_query::Application` and pass it through
/// `txn.query().select(<wrapper>).perform(env)`. Tracked in
/// `plan/effects.md` Phase 3.
async fn fire_effect<'a, Env: InduceEnv>(
    effect: Effect,
    txn: Transaction<'a>,
    env: &Env,
) -> Result<Transaction<'a>, InduceError> {
    let _ = (effect, env);
    Ok(txn)
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
    use dialog_query::{Term, the};
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
        attr.clone()
            .of(subject.clone())
            .is("hello".to_string())
            .assert(&mut transients);

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
