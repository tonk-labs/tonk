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

use dialog_artifacts::{Attribute, Changes, Entity, Instruction, Select, Statement, Update, Value};
use dialog_capability::{Fork, Provider};
use dialog_common::ConditionalSync;
use dialog_effects::archive::{Get, Put};
use dialog_effects::authority::Identify;
use dialog_effects::memory::Resolve;
use dialog_query::concept::query::ConceptQuery;
use dialog_query::error::EvaluationError;
use dialog_query::selection::{Match, Selection};
use dialog_query::source::SelectRules;
use dialog_query::{Cardinality, InductiveRule, Output as _, Parameters, Proposition, Term};
use dialog_repository::{RemoteSite, Transaction};
use thiserror::Error;

use tonk_core::effect::{Effect, EffectError, EffectPolarity};

/// Upper bound on fixpoint rounds. A rule set whose cascade
/// keeps emitting fresh transients beyond this is rejected as
/// non-terminating (cycle or self-feeding parameterized
/// transient — see `plan/effects.md`).
const MAX_ROUNDS: u32 = 16;

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
    /// Cascade: each round walks the reverse index keyed on the
    /// previous round's transients-in-flight, fires every
    /// triggered effect, and partitions emitted heads. Durable
    /// heads land in the transaction; transient heads also land
    /// in the transaction (so the next round's body evaluation
    /// can read them through the overlay) and feed the next
    /// round's reverse-index walk. The loop terminates when a
    /// round emits no transient heads, or errors with
    /// [`InduceError::NonTerminating`] past [`MAX_ROUNDS`].
    ///
    /// User-submitted transients (the seed bucket) and every
    /// effect-emitted transient ultimately get a matching
    /// retract emitted into the transaction so they cancel at
    /// the durable commit boundary.
    ///
    /// Both `assert!:` and `retract!:` rule polarities are
    /// dispatched (see [`fire_effect`]).
    pub async fn perform<Env: InduceEnv>(self, env: &Env) -> Result<Transaction<'a>, InduceError> {
        let Induce {
            mut txn,
            transients,
        } = self;

        // Track every transient that has flowed through the
        // loop (user-submitted seed plus each round's
        // effect-emitted heads) so we can sweep them at the end.
        // Pre-seeded with the user bucket; each round appends
        // its own emitted transients before they propagate.
        let mut all_transients = transients.clone();
        let mut round_transients = transients;
        let mut round: u32 = 0;

        loop {
            if round_transients.is_empty() {
                break;
            }
            if round >= MAX_ROUNDS {
                return Err(InduceError::NonTerminating(MAX_ROUNDS));
            }
            round += 1;

            // 1. From the current round's transients, collect
            //    attribute names → on:<name> reverse-index keys.
            let attribute_names: BTreeSet<String> = round_transients
                .clone()
                .into_instructions()
                .into_iter()
                .map(|inst| match inst {
                    Instruction::Assert(a) | Instruction::Replace(a) | Instruction::Retract(a) => {
                        a.the.to_string()
                    }
                })
                .collect();

            // 2. Walk effects_on per touched attribute.
            let mut effect_entities: BTreeSet<Entity> = BTreeSet::new();
            for name in &attribute_names {
                let hits = effects_on(&txn, name, env).await?;
                effect_entities.extend(hits);
            }

            // 3. Load and fire each candidate. Each fire returns
            //    the transaction with durable + transient heads
            //    integrated, plus a `next` bucket of just the
            //    transient heads it emitted.
            let mut next_transients = Changes::new();
            for entity in effect_entities {
                let Some(effect) = load_effect(&txn, entity, env).await? else {
                    // The reverse index pointed at an entity
                    // whose source claim is missing or
                    // unparseable. Skip — the install path is
                    // supposed to keep these in sync, and we'd
                    // rather drop a bad effect than fail the
                    // commit.
                    continue;
                };
                let outcome = fire_effect(effect, txn, env).await?;
                txn = outcome.txn;
                merge_changes(&mut next_transients, outcome.transient_heads.clone());
                merge_changes(&mut all_transients, outcome.transient_heads);
            }

            // 4. Promote next round's transients. If empty, the
            //    loop's "no transients emitted" terminator fires
            //    on the next iteration's check.
            round_transients = next_transients;
        }

        // Sweep every transient (user-submitted + effect-emitted)
        // so the assert+retract pairs cancel at commit.
        for instruction in all_transients.into_instructions() {
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

/// Merge the contents of `src` into `dst` instruction-by-instruction.
fn merge_changes(dst: &mut Changes, src: Changes) {
    for instruction in src.into_instructions() {
        match instruction {
            Instruction::Assert(a) => dst.associate(a.the, a.of, a.is),
            Instruction::Replace(a) => dst.associate_unique(a.the, a.of, a.is),
            Instruction::Retract(a) => dst.dissociate(a.the, a.of, a.is),
        }
    }
}

/// Parse a `<domain>/<name>` pair into the typed
/// [`dialog_query::attribute::The`] form. Mirrors the helper in
/// [`tonk_core::effect`] so the two modules share a single style of
/// building dialog meta-attribute selectors.
fn the(domain: &str, name: &str) -> dialog_query::attribute::The {
    format!("{domain}/{name}")
        .parse()
        .expect("dialog meta-attribute names are always valid")
}

/// Query the transaction's overlay for effect entities whose
/// `dialog.effect/on` index lists the given attribute name.
/// Equivalent to [`effects_by_on`](crate::effect_query::effects_by_on)
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
/// [`Effect::by_entity`](crate::effect_query::effect_by_entity)'s
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

/// Result of [`fire_effect`]: the transaction with all of the
/// effect's emitted head facts integrated, plus a `Changes`
/// bucket of just the *transient* heads (one entry per claim)
/// for the fixpoint to use as next-round trigger input.
struct FireOutcome<'a> {
    txn: Transaction<'a>,
    transient_heads: Changes,
}

/// Evaluate one effect's body against the transaction overlay,
/// instantiate the head per match, and apply to the transaction.
///
/// For `Assert`-polarity rules each emitted head's facts land
/// in the transaction; transient-concept heads also accumulate
/// in [`FireOutcome::transient_heads`] so the cascade loop can
/// promote them to the next round.
///
/// For `Retract`-polarity rules each emitted head's facts land
/// as retracts. The head concept is expected to be durable —
/// retracts of a transient have no observable effect — so the
/// `transient_heads` bucket is always empty for this polarity.
async fn fire_effect<'a, Env: InduceEnv>(
    effect: Effect,
    mut txn: Transaction<'a>,
    env: &Env,
) -> Result<FireOutcome<'a>, InduceError> {
    let polarity = effect.polarity();
    let rule = effect.into_rule();

    // Evaluate the body against the transaction overlay.
    // `BodyApp` wraps the rule's plan in a `dialog_query::Application`
    // so we can route through `txn.query().select(...).perform(env)`,
    // which supplies the `Provider<Select> + Provider<SelectRules>`
    // wrapper internally.
    let body = BodyApp { rule: rule.clone() };
    let matches: Vec<Match> = dialog_query::Output::try_vec(txn.query().select(body).perform(env))
        .await
        .map_err(|e| InduceError::Query(format!("body evaluation failed: {e:?}")))?;

    // Is the head concept marked transient? Only relevant for
    // assert-polarity heads (transient retracts have no
    // observable effect). One overlay query per fire — cheaper
    // than per-match since the head's concept is fixed.
    let head_is_transient = match polarity {
        EffectPolarity::Assert => is_transient(&txn, rule.conclusion().this(), env).await?,
        EffectPolarity::Retract => false,
    };

    let head = rule.conclusion().clone();
    let mut transient_heads = Changes::new();
    for frame in matches {
        // Project the match into a `Parameters` map of the head's
        // operands. The conclusion-variable check at rule-compile
        // time guarantees every operand is bound somewhere in the
        // body.
        let mut parameters = Parameters::new();
        for operand in head.operands() {
            if let Ok(value) = frame.lookup(&Term::<dialog_query::Any>::var(operand)) {
                parameters.insert(operand.to_string(), Term::Constant(value));
            }
        }

        let proposition = rule
            .apply(parameters)
            .map_err(|e| InduceError::Query(format!("head instantiation failed: {e}")))?;

        // V1 inductive rules produce concept-shaped heads. Walk
        // the predicate and emit one `(attr, this, value)` per
        // bound field into the transaction. For asserts, transient
        // heads also accumulate in the bucket the caller
        // propagates to the next round.
        if let Proposition::Concept(concept_query) = proposition {
            match polarity {
                EffectPolarity::Assert => {
                    if head_is_transient {
                        accumulate_head_facts(&concept_query, &mut transient_heads);
                    }
                    txn = emit_head_facts(concept_query, txn);
                }
                EffectPolarity::Retract => {
                    txn = retract_head_facts(concept_query, txn);
                }
            }
        }
    }

    Ok(FireOutcome {
        txn,
        transient_heads,
    })
}

/// Query the transaction overlay for the
/// `(<concept>, dialog.concept/transient, db:transient)` marker
/// so the loop can classify emitted heads.
async fn is_transient<Env: InduceEnv>(
    txn: &Transaction<'_>,
    concept_entity: Entity,
    env: &Env,
) -> Result<bool, InduceError> {
    let marker_target: Entity = "db:transient"
        .parse()
        .expect("db:transient is a valid entity URI");
    let claims: Vec<dialog_query::Claim> = dialog_query::Output::try_vec(
        txn.query()
            .select(dialog_query::AttributeQuery::from(
                Term::<dialog_query::attribute::The>::from(the("dialog.concept", "transient"))
                    .of(Term::from(concept_entity))
                    .is(Term::from(marker_target)),
            ))
            .perform(env),
    )
    .await
    .map_err(|e| InduceError::Query(format!("transient marker query failed: {e:?}")))?;
    Ok(!claims.is_empty())
}

/// Walk a fully-bound head [`ConceptQuery`] and accumulate one
/// `(attr, this, value)` instruction per bound field into the
/// given [`Changes`] bucket. Used to record an effect's transient
/// head emissions for the next fixpoint round's reverse-index
/// walk, and by `tonk_analyzer::evaluate` to seed the transient bucket
/// from a transient-concept assertion. Mirrors [`emit_head_facts`]
/// but emits into a `Changes` rather than into a `Transaction`.
pub fn accumulate_head_facts(concept_query: &ConceptQuery, sink: &mut Changes) {
    let Some(this_term) = concept_query.terms.get("this") else {
        return;
    };
    let this_entity = match this_term {
        Term::Constant(Value::Entity(e)) => e.clone(),
        _ => return,
    };
    for (field_name, attribute) in concept_query.predicate.with().iter() {
        let Some(term) = concept_query.terms.get(field_name) else {
            continue;
        };
        let Term::Constant(value) = term else {
            continue;
        };
        let the: Attribute = attribute.the().clone().into();
        match attribute.cardinality() {
            Cardinality::One => sink.associate_unique(the, this_entity.clone(), value.clone()),
            Cardinality::Many => sink.associate(the, this_entity.clone(), value.clone()),
        }
    }
}

/// Walk a fully-bound [`ConceptQuery`] (the instantiated head
/// of an assert-polarity rule) and emit one assertion per
/// non-blank field. Mirrors the same emission logic the
/// asserted-notation planner uses in `tonk_core::transact`, but
/// writes directly into a dialog `Transaction` since the
/// induce path doesn't go through `ApplicationPlan`.
fn emit_head_facts<'a>(concept_query: ConceptQuery, mut txn: Transaction<'a>) -> Transaction<'a> {
    let Some(this_term) = concept_query.terms.get("this") else {
        return txn;
    };
    let this_entity = match this_term {
        Term::Constant(Value::Entity(e)) => e.clone(),
        _ => return txn,
    };
    for (field_name, attribute) in concept_query.predicate.with().iter() {
        let Some(term) = concept_query.terms.get(field_name) else {
            continue;
        };
        let Term::Constant(value) = term else {
            continue;
        };
        let the: Attribute = attribute.the().clone().into();
        txn = match attribute.cardinality() {
            Cardinality::One => txn.assert(RawReplace {
                the,
                of: this_entity.clone(),
                is: value.clone(),
            }),
            Cardinality::Many => txn.assert(RawClaim {
                the,
                of: this_entity.clone(),
                is: value.clone(),
            }),
        };
    }
    txn
}

/// Retract-polarity sibling of [`emit_head_facts`]. Walks a
/// fully-bound head and emits one retract per bound field so
/// the body's match-bound values are dissociated from the
/// underlying entity. Cardinality doesn't change the retract
/// path — both one and many fields dissociate by the exact
/// `(attr, this, value)` triple.
fn retract_head_facts<'a>(
    concept_query: ConceptQuery,
    mut txn: Transaction<'a>,
) -> Transaction<'a> {
    let Some(this_term) = concept_query.terms.get("this") else {
        return txn;
    };
    let this_entity = match this_term {
        Term::Constant(Value::Entity(e)) => e.clone(),
        _ => return txn,
    };
    for (field_name, attribute) in concept_query.predicate.with().iter() {
        let Some(term) = concept_query.terms.get(field_name) else {
            continue;
        };
        let Term::Constant(value) = term else {
            continue;
        };
        let the: Attribute = attribute.the().clone().into();
        txn = txn.retract(RawClaim {
            the,
            of: this_entity.clone(),
            is: value.clone(),
        });
    }
    txn
}

/// Wrap an [`InductiveRule`] as a [`dialog_query::Application`]
/// so its body can be evaluated against a [`Transaction::query`]
/// overlay. The conclusion is the raw [`Match`] — the induce
/// loop projects head operands out of it after the fact.
#[derive(Clone)]
struct BodyApp {
    rule: InductiveRule,
}

impl dialog_query::Application for BodyApp {
    type Conclusion = Match;

    fn evaluate<'a, Env, M: Selection + 'a>(self, selection: M, env: &'a Env) -> impl Selection + 'a
    where
        Env: Provider<Select<'a>> + Provider<SelectRules> + ConditionalSync,
    {
        let plan = self.rule.plan(&Default::default());
        plan.evaluate(selection, env)
    }

    fn realize(&self, input: Match) -> Result<Match, EvaluationError> {
        Ok(input)
    }
}

/// One concrete `(the, of, is)` triple wrapped as a
/// [`Statement`] so the transient sweep can hand it to
/// [`Transaction::assert`] / [`Transaction::retract`].
struct RawClaim {
    the: Attribute,
    of: Entity,
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

/// Cardinality-one variant of [`RawClaim`] — emits via
/// `associate_unique` so re-assertion of the same `(the, of)`
/// pair supersedes the prior value.
struct RawReplace {
    the: Attribute,
    of: Entity,
    is: Value,
}

impl Statement for RawReplace {
    fn assert(self, update: &mut impl Update) {
        update.associate_unique(self.the, self.of, self.is);
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

    use crate::concept::{AnonymousConcept, TransientConcept};
    use crate::effect_query::EffectStatement;
    use dialog_artifacts::Statement;
    use dialog_query::artifact::Type;
    use dialog_query::attribute::Cardinality as DialogCardinality;
    use dialog_query::concept::descriptor::ConceptDescriptor;
    use dialog_query::concept::query::ConceptQuery;
    use dialog_query::premise::Premise as DialogPremise;
    use dialog_query::{AttributeDescriptor, InductiveRule, Parameters as DialogParameters};
    use tonk_core::effect::{Effect, EffectPolarity};

    /// A 1-field concept descriptor with a configurable field
    /// type. Helper because tests below build several.
    fn one_field_concept(domain: &str, name: &str, ty: Type) -> ConceptDescriptor {
        ConceptDescriptor::from(vec![(
            name,
            AttributeDescriptor::new(
                format!("{domain}/{name}").parse().unwrap(),
                "",
                DialogCardinality::One,
                Some(ty),
            ),
        )])
    }

    /// Shorthand for the common String case used by the early
    /// tests below.
    fn one_text_field_concept(domain: &str, name: &str) -> ConceptDescriptor {
        one_field_concept(domain, name, Type::String)
    }

    /// The string form dialog stores in `dialog.attribute/type`
    /// for each `Type` variant the tests need. The labels match
    /// dialog's `TypeDescriptor` names (Text for String,
    /// UnsignedInteger for UnsignedInt, etc.), not the variant
    /// names of `Type` itself.
    fn type_storage_string(ty: Type) -> &'static str {
        match ty {
            Type::String => "Text",
            Type::Entity => "Entity",
            Type::UnsignedInt => "UnsignedInteger",
            _ => "Text",
        }
    }

    /// Install the attribute-side facts a concept's fields need
    /// so the concept's query can be rehydrated against the
    /// branch. Mirrors the pattern in `concept.rs`'s round-trip
    /// test.
    fn install_attribute_facts<'a>(
        mut txn: dialog_repository::Transaction<'a>,
        descriptor: &ConceptDescriptor,
    ) -> dialog_repository::Transaction<'a> {
        for (_, attr) in descriptor.with().iter() {
            let attr_entity: Entity = attr.to_uri().parse().expect("attribute URI");
            let type_label = attr
                .content_type()
                .map(type_storage_string)
                .unwrap_or("String")
                .to_string();
            txn = txn
                .assert(
                    the!("dialog.attribute/id")
                        .of(attr_entity.clone())
                        .is(format!("{}/{}", attr.domain(), attr.name())),
                )
                .assert(
                    the!("dialog.attribute/type")
                        .of(attr_entity.clone())
                        .is(type_label),
                )
                .assert(
                    the!("dialog.attribute/cardinality")
                        .of(attr_entity.clone())
                        .is("one".to_string()),
                )
                .assert(
                    the!("dialog.meta/description")
                        .of(attr_entity)
                        .is(String::new()),
                );
        }
        txn
    }

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

    /// End-to-end fire path: a transient `ping{this, tag}`
    /// triggers a `pong{this, tag}` head, which lands durably.
    /// Verifies discovery + body evaluation + head emission
    /// together.
    #[dialog_common::test]
    async fn it_fires_an_assert_rule_on_a_transient() -> anyhow::Result<()> {
        let (operator, profile) = test_operator_with_profile().await;
        let repo = test_repo(&operator, &profile).await;
        let branch = repo.branch("main").open().perform(&operator).await?;

        // Concepts: ping (transient) and pong (durable), each
        // with a single `tag: Text` field.
        let ping = one_text_field_concept("io.gozala.ping", "tag");
        let pong = one_text_field_concept("io.gozala.pong", "tag");

        // Body: read a ping instance, binding its this and tag.
        // Head: pong with the same this/tag.
        let mut body_terms = DialogParameters::new();
        body_terms.insert("this".to_string(), Term::var("this"));
        body_terms.insert("tag".to_string(), Term::var("tag"));
        let body_premise =
            DialogPremise::Assert(dialog_query::Proposition::Concept(ConceptQuery {
                terms: body_terms,
                predicate: ping.clone(),
            }));
        let rule = InductiveRule::new(pong.clone(), vec![body_premise]).expect("rule compiles");
        let effect = Effect::new(rule, EffectPolarity::Assert);

        // Install everything: concept facts, transient marker
        // on ping, the effect itself.
        let mut install = branch.transaction();
        install = install_attribute_facts(install, &ping);
        install = install_attribute_facts(install, &pong);
        install = install.assert(AnonymousConcept::new(pong.clone()));
        install = install.assert(TransientConcept::new(ping.clone()));
        install = install.assert(EffectStatement(effect));
        install.commit().perform(&operator).await?;

        // Submit a transient `ping{this: e1, tag: "hello"}`.
        let subject: Entity = "did:key:zPingSubject".parse()?;
        let ping_tag_attr = the!("io.gozala.ping/tag");
        let mut transients = Changes::new();
        ping_tag_attr
            .clone()
            .of(subject.clone())
            .is("hello".to_string())
            .assert(&mut transients);

        // Drive induce + commit through the chain.
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

        // Expect the durable pong claim landed.
        let pong_tag_attr = the!("io.gozala.pong/tag");
        let pong_claims: Vec<dialog_query::Claim> = branch
            .query()
            .select(dialog_query::AttributeQuery::from(
                Term::from(pong_tag_attr)
                    .of(Term::from(subject.clone()))
                    .is(Term::<String>::var("v")),
            ))
            .perform(&operator)
            .try_vec()
            .await?;

        assert_eq!(
            pong_claims.len(),
            1,
            "expected one pong claim from the firing rule; saw {pong_claims:?}"
        );

        // And the ping claim should not have survived.
        let ping_claims: Vec<dialog_query::Claim> = branch
            .query()
            .select(dialog_query::AttributeQuery::from(
                Term::from(ping_tag_attr)
                    .of(Term::from(subject))
                    .is(Term::<String>::var("v")),
            ))
            .perform(&operator)
            .try_vec()
            .await?;
        assert!(
            ping_claims.is_empty(),
            "transient ping should have been swept; saw {ping_claims:?}"
        );

        Ok(())
    }

    /// Two-round cascade: a transient `cmd_a` fires effect A
    /// which emits a transient `cmd_b`, which in turn fires
    /// effect B emitting a durable `final`. The fixpoint loop
    /// runs at least two rounds; both transients get swept
    /// before commit so the only durable artifact is the
    /// `final` claim.
    #[dialog_common::test]
    async fn it_cascades_through_transient_intermediates() -> anyhow::Result<()> {
        let (operator, profile) = test_operator_with_profile().await;
        let repo = test_repo(&operator, &profile).await;
        let branch = repo.branch("main").open().perform(&operator).await?;

        let cmd_a = one_text_field_concept("io.gozala.cmd-a", "tag");
        let cmd_b = one_text_field_concept("io.gozala.cmd-b", "tag");
        let target = one_text_field_concept("io.gozala.target", "tag");

        // Effect A: cmd_b{this, tag} when cmd_a{this, tag}.
        let mut a_body_terms = DialogParameters::new();
        a_body_terms.insert("this".to_string(), Term::var("this"));
        a_body_terms.insert("tag".to_string(), Term::var("tag"));
        let a_body = DialogPremise::Assert(dialog_query::Proposition::Concept(ConceptQuery {
            terms: a_body_terms,
            predicate: cmd_a.clone(),
        }));
        let rule_a = InductiveRule::new(cmd_b.clone(), vec![a_body]).expect("rule a compiles");
        let effect_a = Effect::new(rule_a, EffectPolarity::Assert);

        // Effect B: target{this, tag} when cmd_b{this, tag}.
        let mut b_body_terms = DialogParameters::new();
        b_body_terms.insert("this".to_string(), Term::var("this"));
        b_body_terms.insert("tag".to_string(), Term::var("tag"));
        let b_body = DialogPremise::Assert(dialog_query::Proposition::Concept(ConceptQuery {
            terms: b_body_terms,
            predicate: cmd_b.clone(),
        }));
        let rule_b = InductiveRule::new(target.clone(), vec![b_body]).expect("rule b compiles");
        let effect_b = Effect::new(rule_b, EffectPolarity::Assert);

        // Install attributes, concepts (cmd_a and cmd_b are
        // transient, target is durable), and both effects.
        let mut install = branch.transaction();
        install = install_attribute_facts(install, &cmd_a);
        install = install_attribute_facts(install, &cmd_b);
        install = install_attribute_facts(install, &target);
        install = install.assert(TransientConcept::new(cmd_a.clone()));
        install = install.assert(TransientConcept::new(cmd_b.clone()));
        install = install.assert(AnonymousConcept::new(target.clone()));
        install = install.assert(EffectStatement(effect_a));
        install = install.assert(EffectStatement(effect_b));
        install.commit().perform(&operator).await?;

        // Seed a single transient cmd_a.
        let subject: Entity = "did:key:zCascadeSubject".parse()?;
        let cmd_a_attr = the!("io.gozala.cmd-a/tag");
        let mut transients = Changes::new();
        cmd_a_attr
            .clone()
            .of(subject.clone())
            .is("hello".to_string())
            .assert(&mut transients);

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

        // Durable target must have landed exactly once.
        let target_attr = the!("io.gozala.target/tag");
        let target_claims: Vec<dialog_query::Claim> = branch
            .query()
            .select(dialog_query::AttributeQuery::from(
                Term::from(target_attr)
                    .of(Term::from(subject.clone()))
                    .is(Term::<String>::var("v")),
            ))
            .perform(&operator)
            .try_vec()
            .await?;
        assert_eq!(
            target_claims.len(),
            1,
            "expected one durable target claim from the cascade; saw {target_claims:?}"
        );

        // Neither transient should have survived.
        let cmd_b_attr = the!("io.gozala.cmd-b/tag");
        for (label, attr) in [("cmd_a", cmd_a_attr), ("cmd_b", cmd_b_attr)] {
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
                "transient {label} should have been swept; saw {claims:?}"
            );
        }

        Ok(())
    }

    /// A self-feeding cascade: a rule reads its own concept and
    /// re-emits it, so each round produces a fresh trigger for
    /// the next. `MAX_ROUNDS` must reject this rather than loop
    /// forever.
    #[dialog_common::test]
    async fn it_errors_on_runaway_cascade() -> anyhow::Result<()> {
        let (operator, profile) = test_operator_with_profile().await;
        let repo = test_repo(&operator, &profile).await;
        let branch = repo.branch("main").open().perform(&operator).await?;

        let tick = one_text_field_concept("io.gozala.tick", "tag");

        // assert!: tick{this, tag} when tick{this, tag}. Reading
        // the head's own concept guarantees re-emission every
        // round; the value passes through unchanged. Both
        // emitted facts collapse onto the same cell (cardinality
        // one), so a smart engine could fixpoint after round 1
        // — but our V1 doesn't dedupe at the head level, so
        // each round emits a fresh "tick" transient and triggers
        // the next.
        let mut body_terms = DialogParameters::new();
        body_terms.insert("this".to_string(), Term::var("this"));
        body_terms.insert("tag".to_string(), Term::var("tag"));
        let body_premise =
            DialogPremise::Assert(dialog_query::Proposition::Concept(ConceptQuery {
                terms: body_terms,
                predicate: tick.clone(),
            }));
        let rule = InductiveRule::new(tick.clone(), vec![body_premise]).expect("rule compiles");
        let effect = Effect::new(rule, EffectPolarity::Assert);

        let mut install = branch.transaction();
        install = install_attribute_facts(install, &tick);
        install = install.assert(TransientConcept::new(tick.clone()));
        install = install.assert(EffectStatement(effect));
        install.commit().perform(&operator).await?;

        let subject: Entity = "did:key:zRunawaySubject".parse()?;
        let tick_attr = the!("io.gozala.tick/tag");
        let mut transients = Changes::new();
        tick_attr
            .of(subject)
            .is("seed".to_string())
            .assert(&mut transients);

        let result = branch
            .transaction()
            .integrate(transients.clone())
            .induce(transients)
            .perform(&operator)
            .await;

        match result {
            Err(InduceError::NonTerminating(n)) => {
                assert_eq!(n, MAX_ROUNDS, "should report the configured bound");
                Ok(())
            }
            Err(other) => Err(anyhow::anyhow!("expected NonTerminating; got {other:?}")),
            Ok(_) => Err(anyhow::anyhow!(
                "expected NonTerminating; loop unexpectedly settled"
            )),
        }
    }

    /// Retract-polarity rule, mailbox-with-ack shape.
    ///
    /// A durable `message{body}` exists on the branch. A
    /// transient `ack{target}` arrives. The rule
    /// `retract!: message{this: ?m, body: ?b} when ack{target:
    /// ?m}, message{this: ?m, body: ?b}` removes the message
    /// for that target. After commit the message is gone and
    /// the ack — being transient — never persisted.
    #[dialog_common::test]
    async fn it_fires_a_retract_rule_on_an_ack() -> anyhow::Result<()> {
        let (operator, profile) = test_operator_with_profile().await;
        let repo = test_repo(&operator, &profile).await;
        let branch = repo.branch("main").open().perform(&operator).await?;

        let message = one_text_field_concept("io.gozala.mailbox", "body");
        let ack = one_field_concept("io.gozala.mailbox", "target", Type::Entity);

        // Body: ack{target: ?this}, message{this: ?this, body:
        // ?body}. Sharing the variable name `this` between
        // ack.target and message.this joins them: the engine
        // will only emit matches where ack's target equals the
        // message entity. Variable names align with the head's
        // operand names so the conclusion-variable check passes
        // (`this` and `body` are the message descriptor's
        // operands).
        let mut ack_terms = DialogParameters::new();
        ack_terms.insert("this".to_string(), Term::var("__ack_this"));
        ack_terms.insert("target".to_string(), Term::var("this"));
        let ack_premise = DialogPremise::Assert(dialog_query::Proposition::Concept(ConceptQuery {
            terms: ack_terms,
            predicate: ack.clone(),
        }));
        let mut msg_terms = DialogParameters::new();
        msg_terms.insert("this".to_string(), Term::var("this"));
        msg_terms.insert("body".to_string(), Term::var("body"));
        let message_premise =
            DialogPremise::Assert(dialog_query::Proposition::Concept(ConceptQuery {
                terms: msg_terms,
                predicate: message.clone(),
            }));

        let rule = InductiveRule::new(message.clone(), vec![ack_premise, message_premise])
            .expect("rule compiles");
        let effect = Effect::new(rule, EffectPolarity::Retract);

        // Install: attributes, durable message concept, transient ack
        // concept, effect.
        let mut install = branch.transaction();
        install = install_attribute_facts(install, &message);
        install = install_attribute_facts(install, &ack);
        install = install.assert(AnonymousConcept::new(message.clone()));
        install = install.assert(TransientConcept::new(ack.clone()));
        install = install.assert(EffectStatement(effect));
        install.commit().perform(&operator).await?;

        // Seed a durable message{this: m1, body: "hello"}.
        let m1: Entity = "did:key:zMailboxM1".parse()?;
        let body_attr = the!("io.gozala.mailbox/body");
        branch
            .transaction()
            .assert(body_attr.clone().of(m1.clone()).is("hello".to_string()))
            .commit()
            .perform(&operator)
            .await?;

        // Submit transient ack{this: <anon>, target: m1}.
        let ack_subject: Entity = "did:key:zMailboxAck".parse()?;
        let target_attr = the!("io.gozala.mailbox/target");
        let mut transients = Changes::new();
        target_attr
            .clone()
            .of(ack_subject.clone())
            .is(m1.clone())
            .assert(&mut transients);

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

        // Message must be gone from durable state.
        let msg_claims: Vec<dialog_query::Claim> = branch
            .query()
            .select(dialog_query::AttributeQuery::from(
                Term::from(body_attr)
                    .of(Term::from(m1.clone()))
                    .is(Term::<String>::var("v")),
            ))
            .perform(&operator)
            .try_vec()
            .await?;
        assert!(
            msg_claims.is_empty(),
            "retract!: message should have removed the message body; saw {msg_claims:?}"
        );

        // Ack must have been swept.
        let ack_claims: Vec<dialog_query::Claim> = branch
            .query()
            .select(dialog_query::AttributeQuery::from(
                Term::from(target_attr)
                    .of(Term::from(ack_subject))
                    .is(Term::<Entity>::var("v")),
            ))
            .perform(&operator)
            .try_vec()
            .await?;
        assert!(
            ack_claims.is_empty(),
            "transient ack should have been swept; saw {ack_claims:?}"
        );

        Ok(())
    }

    /// Silent drop: an effect is installed reading concept A,
    /// but the submitted transient is concept B. The reverse
    /// index doesn't match, no rule fires, and the submitted
    /// transient is still swept by the end-of-loop sweep.
    /// Confirms that "no candidates" is the loop's natural
    /// no-op state and that an unrelated transient doesn't leak
    /// into durable storage.
    #[dialog_common::test]
    async fn it_silently_drops_unrelated_transients() -> anyhow::Result<()> {
        let (operator, profile) = test_operator_with_profile().await;
        let repo = test_repo(&operator, &profile).await;
        let branch = repo.branch("main").open().perform(&operator).await?;

        // Effect reads `io.gozala.ping/tag` only. Unrelated
        // attribute `io.gozala.noise/tag` won't match.
        let ping = one_text_field_concept("io.gozala.ping", "tag");
        let pong = one_text_field_concept("io.gozala.pong", "tag");
        let noise = one_text_field_concept("io.gozala.noise", "tag");

        let mut body_terms = DialogParameters::new();
        body_terms.insert("this".to_string(), Term::var("this"));
        body_terms.insert("tag".to_string(), Term::var("tag"));
        let body_premise =
            DialogPremise::Assert(dialog_query::Proposition::Concept(ConceptQuery {
                terms: body_terms,
                predicate: ping.clone(),
            }));
        let rule = InductiveRule::new(pong.clone(), vec![body_premise]).expect("rule compiles");
        let effect = Effect::new(rule, EffectPolarity::Assert);

        let mut install = branch.transaction();
        install = install_attribute_facts(install, &ping);
        install = install_attribute_facts(install, &pong);
        install = install_attribute_facts(install, &noise);
        install = install.assert(AnonymousConcept::new(pong.clone()));
        install = install.assert(TransientConcept::new(ping.clone()));
        install = install.assert(TransientConcept::new(noise.clone()));
        install = install.assert(EffectStatement(effect));
        install.commit().perform(&operator).await?;

        let subject: Entity = "did:key:zNoiseSubject".parse()?;
        let noise_attr = the!("io.gozala.noise/tag");
        let mut transients = Changes::new();
        noise_attr
            .clone()
            .of(subject.clone())
            .is("ignored".to_string())
            .assert(&mut transients);

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

        // No pong claim — the effect didn't fire.
        let pong_attr = the!("io.gozala.pong/tag");
        let pong_claims: Vec<dialog_query::Claim> = branch
            .query()
            .select(dialog_query::AttributeQuery::from(
                Term::from(pong_attr)
                    .of(Term::from(subject.clone()))
                    .is(Term::<String>::var("v")),
            ))
            .perform(&operator)
            .try_vec()
            .await?;
        assert!(
            pong_claims.is_empty(),
            "no pong should have landed; saw {pong_claims:?}"
        );

        // And the unrelated transient was still swept.
        let noise_claims: Vec<dialog_query::Claim> = branch
            .query()
            .select(dialog_query::AttributeQuery::from(
                Term::from(noise_attr)
                    .of(Term::from(subject))
                    .is(Term::<String>::var("v")),
            ))
            .perform(&operator)
            .try_vec()
            .await?;
        assert!(
            noise_claims.is_empty(),
            "noise transient should have been swept; saw {noise_claims:?}"
        );

        Ok(())
    }

    /// Increment-counter via a formula in the body. A durable
    /// `counter{this: ?c, count: ?prev}` exists; submitting a
    /// transient `increment{this: ?c}` triggers the rule whose
    /// body reads `counter.count` and uses `math/sum` to bind
    /// `?count = ?prev + 1`. The head re-asserts the counter
    /// (cardinality-one `count`, so the prior value is
    /// replaced). After commit the counter holds the new value.
    #[dialog_common::test]
    async fn it_fires_a_rule_with_a_formula_body() -> anyhow::Result<()> {
        use dialog_query::formula::Formula;
        use dialog_query::formula::math::Sum;

        let (operator, profile) = test_operator_with_profile().await;
        let repo = test_repo(&operator, &profile).await;
        let branch = repo.branch("main").open().perform(&operator).await?;

        let counter = one_field_concept("io.gozala.counter", "count", Type::UnsignedInt);
        let increment = one_field_concept("io.gozala.increment", "subject", Type::Entity);

        // Body: counter{this: ?this, count: ?prev},
        //       increment{target: ?this},
        //       Sum{of: ?prev, with: 1, is: ?count}.
        let mut counter_terms = DialogParameters::new();
        counter_terms.insert("this".to_string(), Term::var("this"));
        counter_terms.insert("count".to_string(), Term::var("prev"));
        let counter_premise =
            DialogPremise::Assert(dialog_query::Proposition::Concept(ConceptQuery {
                terms: counter_terms,
                predicate: counter.clone(),
            }));
        let mut inc_terms = DialogParameters::new();
        inc_terms.insert("this".to_string(), Term::var("__inc_this"));
        inc_terms.insert("subject".to_string(), Term::var("this"));
        let inc_premise = DialogPremise::Assert(dialog_query::Proposition::Concept(ConceptQuery {
            terms: inc_terms,
            predicate: increment.clone(),
        }));
        let mut sum_terms = DialogParameters::new();
        sum_terms.insert("of".to_string(), Term::var("prev"));
        sum_terms.insert("with".to_string(), Term::constant(1u64));
        sum_terms.insert("is".to_string(), Term::var("count"));
        let sum_premise = Sum::apply(sum_terms).expect("Sum::apply compiles").into();

        let rule = InductiveRule::new(
            counter.clone(),
            vec![counter_premise, inc_premise, sum_premise],
        )
        .expect("rule compiles");
        let effect = Effect::new(rule, EffectPolarity::Assert);

        let mut install = branch.transaction();
        install = install_attribute_facts(install, &counter);
        install = install_attribute_facts(install, &increment);
        install = install.assert(AnonymousConcept::new(counter.clone()));
        install = install.assert(TransientConcept::new(increment.clone()));
        install = install.assert(EffectStatement(effect));
        install.commit().perform(&operator).await?;

        // Seed the counter at 41.
        let c1: Entity = "did:key:zCounterC1".parse()?;
        let count_attr = the!("io.gozala.counter/count");
        branch
            .transaction()
            .assert(count_attr.clone().of(c1.clone()).is(41u64))
            .commit()
            .perform(&operator)
            .await?;

        // Submit transient increment{this: <anon>, subject: c1}.
        let inc_subject: Entity = "did:key:zIncrementCmd".parse()?;
        let subject_attr = the!("io.gozala.increment/subject");
        let mut transients = Changes::new();
        subject_attr
            .of(inc_subject)
            .is(c1.clone())
            .assert(&mut transients);

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

        let claims: Vec<dialog_query::Claim> = branch
            .query()
            .select(dialog_query::AttributeQuery::from(
                Term::from(count_attr)
                    .of(Term::from(c1))
                    .is(Term::<u64>::var("v")),
            ))
            .perform(&operator)
            .try_vec()
            .await?;
        assert_eq!(
            claims.len(),
            1,
            "expected exactly one count claim post-cardinality-one supersede; saw {claims:?}"
        );
        let Value::UnsignedInt(n) = &claims[0].is else {
            return Err(anyhow::anyhow!(
                "expected UnsignedInt count value; saw {:?}",
                claims[0].is
            ));
        };
        assert_eq!(*n, 42, "increment should bump 41 → 42");

        Ok(())
    }

    /// Cardinality-many head field: a rule with a many-cardinality
    /// `tag` accumulates values instead of replacing. Fire the
    /// rule twice (two transients with different tags) and
    /// verify both tags survive in the durable head.
    #[dialog_common::test]
    async fn it_accumulates_many_cardinality_head_facts() -> anyhow::Result<()> {
        let (operator, profile) = test_operator_with_profile().await;
        let repo = test_repo(&operator, &profile).await;
        let branch = repo.branch("main").open().perform(&operator).await?;

        // Both fields are cardinality-many. The bag head field
        // accumulates, so two firings should leave two claims.
        let cmd = ConceptDescriptor::from(vec![(
            "tag",
            AttributeDescriptor::new(
                "io.gozala.bag-cmd/tag".parse().unwrap(),
                "",
                DialogCardinality::Many,
                Some(Type::String),
            ),
        )]);
        let bag = ConceptDescriptor::from(vec![(
            "tag",
            AttributeDescriptor::new(
                "io.gozala.bag/tag".parse().unwrap(),
                "",
                DialogCardinality::Many,
                Some(Type::String),
            ),
        )]);

        let mut body_terms = DialogParameters::new();
        body_terms.insert("this".to_string(), Term::var("this"));
        body_terms.insert("tag".to_string(), Term::var("tag"));
        let body_premise =
            DialogPremise::Assert(dialog_query::Proposition::Concept(ConceptQuery {
                terms: body_terms,
                predicate: cmd.clone(),
            }));
        let rule = InductiveRule::new(bag.clone(), vec![body_premise]).expect("rule compiles");
        let effect = Effect::new(rule, EffectPolarity::Assert);

        let mut install = branch.transaction();
        install = install_attribute_facts(install, &cmd);
        install = install_attribute_facts(install, &bag);
        install = install.assert(AnonymousConcept::new(bag.clone()));
        install = install.assert(TransientConcept::new(cmd.clone()));
        install = install.assert(EffectStatement(effect));
        install.commit().perform(&operator).await?;

        let subject: Entity = "did:key:zBagSubject".parse()?;
        let cmd_attr = the!("io.gozala.bag-cmd/tag");

        // Two separate commits, each submitting a different tag.
        for tag in ["first", "second"] {
            let mut transients = Changes::new();
            cmd_attr
                .clone()
                .of(subject.clone())
                .is(tag.to_string())
                .assert(&mut transients);
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
        }

        let bag_attr = the!("io.gozala.bag/tag");
        let claims: Vec<dialog_query::Claim> = branch
            .query()
            .select(dialog_query::AttributeQuery::from(
                Term::from(bag_attr)
                    .of(Term::from(subject))
                    .is(Term::<String>::var("v")),
            ))
            .perform(&operator)
            .try_vec()
            .await?;
        let mut values: Vec<String> = claims
            .iter()
            .filter_map(|c| match &c.is {
                Value::String(s) => Some(s.clone()),
                _ => None,
            })
            .collect();
        values.sort();
        assert_eq!(
            values,
            vec!["first".to_string(), "second".to_string()],
            "many-cardinality head should accumulate both tags; saw {claims:?}"
        );

        Ok(())
    }
}
