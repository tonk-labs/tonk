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
    /// - Retract-polarity effects aren't dispatched yet (see
    ///   [`fire_effect`]).
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
/// V1: only `Assert`-polarity heads are dispatched. Retract
/// polarity is recognized at install time but its head
/// dispatch isn't wired yet.
async fn fire_effect<'a, Env: InduceEnv>(
    effect: Effect,
    mut txn: Transaction<'a>,
    env: &Env,
) -> Result<Transaction<'a>, InduceError> {
    if effect.polarity() != EffectPolarity::Assert {
        // V1: retract-polarity dispatch lands later.
        return Ok(txn);
    }

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

    let head = rule.conclusion().clone();
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
        // bound field into the transaction.
        if let Proposition::Concept(concept_query) = proposition {
            txn = emit_head_facts(concept_query, txn);
        }
    }

    Ok(txn)
}

/// Walk a fully-bound [`ConceptQuery`] (the instantiated head
/// of an assert-polarity rule) and emit one assertion per
/// non-blank field. Mirrors the same emission logic the
/// asserted-notation planner uses in `crate::transact`, but
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
    use crate::effect::{Effect, EffectPolarity};
    use dialog_artifacts::Statement;
    use dialog_query::artifact::Type;
    use dialog_query::attribute::Cardinality as DialogCardinality;
    use dialog_query::concept::descriptor::ConceptDescriptor;
    use dialog_query::concept::query::ConceptQuery;
    use dialog_query::premise::Premise as DialogPremise;
    use dialog_query::{AttributeDescriptor, InductiveRule, Parameters as DialogParameters};

    /// A 1-field concept descriptor. Helper because tests below
    /// build several.
    fn one_text_field_concept(domain: &str, name: &str) -> ConceptDescriptor {
        ConceptDescriptor::from(vec![(
            name,
            AttributeDescriptor::new(
                format!("{domain}/{name}").parse().unwrap(),
                "",
                DialogCardinality::One,
                Some(Type::String),
            ),
        )])
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
            txn = txn
                .assert(
                    the!("dialog.attribute/id")
                        .of(attr_entity.clone())
                        .is(format!("{}/{}", attr.domain(), attr.name())),
                )
                .assert(
                    the!("dialog.attribute/type")
                        .of(attr_entity.clone())
                        .is("String".to_string()),
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
        install = install.assert(effect);
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
}
