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
//! use tonk_evaluator::effects::TransactionExt;
//!
//! let txn = branch.transaction()
//!     .assert(...)
//!     .induce(transients)
//!     .perform(env).await?;
//! let revision = txn.commit().perform(env).await?;
//! ```

use std::collections::{BTreeMap, BTreeSet};

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

use tonk_core::command::{
    COMMAND_ARGUMENT_RELATION_PREFIX, COMMAND_KIND_RELATION, CommandBatch, CommandOccurrence,
    InvocationMetadata, SourceInvocation,
};
use tonk_core::effect::{Effect, EffectError, EffectPolarity};
use tonk_schema::command_definition::CommandDefinition;
use tonk_schema::query_source::Source;

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
    /// A command-emitting rule produced an invalid nominal payload.
    #[error("effect {effect} emitted an invalid command: {reason}")]
    InvalidCommandOutput {
        /// Installed effect that emitted the invalid head. Boxed: an
        /// `Entity` dwarfs every other payload in this enum, and this
        /// is the rare variant, so inlining it would widen every
        /// `Result` on the induction path.
        effect: Box<Entity>,
        /// Schema-resolution or validation failure.
        reason: String,
    },
}

/// Per-occurrence command preflight and firing counts.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct InduceSummary {
    /// Installed nominal rules selected by the occurrence's kind.
    pub registered_rules_by_occurrence: BTreeMap<Entity, usize>,
    /// Selected rules whose bodies produced at least one match.
    pub fired_rules_by_occurrence: BTreeMap<Entity, usize>,
}

/// Post-fixpoint transaction paired with command induction evidence.
pub struct InduceReport<'a> {
    /// Transaction ready for durable commit.
    pub transaction: Transaction<'a>,
    /// Per-occurrence preflight and firing counts.
    pub summary: InduceSummary,
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
    /// Run induction with a separate batch of nominal command
    /// occurrences that never enters durable branch facts.
    fn induce_commands(self, transients: Changes, commands: CommandBatch) -> Induce<'a>;
}

impl<'a> TransactionExt<'a> for Transaction<'a> {
    fn induce(self, transients: Changes) -> Induce<'a> {
        Induce {
            transaction: self,
            transients,
            commands: CommandBatch::default(),
        }
    }

    fn induce_commands(self, transients: Changes, commands: CommandBatch) -> Induce<'a> {
        Induce {
            transaction: self,
            transients,
            commands,
        }
    }
}

/// Chain handle for an induction pass. Holds the transaction
/// and the transient bucket until `.perform(env)` consumes
/// them.
pub struct Induce<'a> {
    transaction: Transaction<'a>,
    transients: Changes,
    commands: CommandBatch,
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
        Ok(self.perform_report(env).await?.transaction)
    }

    /// Execute induction and retain per-command occurrence evidence.
    pub async fn perform_report<Env: InduceEnv>(
        self,
        env: &Env,
    ) -> Result<InduceReport<'a>, InduceError> {
        let Induce {
            mut transaction,
            transients,
            commands,
        } = self;

        // Each round's transient bucket — the user-submitted seed
        // initially, then this round's effect-emitted heads as
        // the next round's triggers. Swept at end-of-round so
        // their assert+retract pairs cancel at commit and the
        // next round's bodies don't see them (a transient is a
        // one-shot trigger, by design).
        let mut round: u32 = 0;
        let mut stimulus = transients;
        let mut command_stimulus = commands;
        let mut summary = InduceSummary::default();

        while !stimulus.is_empty() || !command_stimulus.is_empty() {
            if round >= MAX_ROUNDS {
                return Err(InduceError::NonTerminating(MAX_ROUNDS));
            }
            round += 1;

            // 1. From the current round's transients, collect
            //    attribute names → on:<name> reverse-index keys.
            let attribute_names: BTreeSet<String> = stimulus
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
                let hits = effects_on(&transaction, name, env).await?;
                effect_entities.extend(hits);
            }

            // 3. Load and fire each candidate against the FROZEN
            //    round-input transaction. Each fire returns its
            //    derived facts as a `Changes` batch (durable +
            //    transient) without mutating `transaction`; rule
            //    N's derivations are NOT visible to rule N+1 in
            //    the same round — sibling rules read the same
            //    input. We collect every fire's facts into
            //    `novelty`, integrate them into `transaction`
            //    once after the for-loop, and propagate the
            //    transient subset as the next round's stimulus.
            //
            //    This is standard semi-naive Datalog: derivations
            //    within an iteration don't see each other; the
            //    iteration's input is frozen. Cross-round
            //    chaining still works because round N+1 sees
            //    round N's integrated novelty.
            let mut transients = Changes::new();
            let mut novelty = Changes::new();
            for entity in effect_entities {
                let Some(effect) = load_effect(&transaction, entity.clone(), env).await? else {
                    // The reverse index pointed at an entity
                    // whose source claim is missing or
                    // unparseable. Skip — the install path is
                    // supposed to keep these in sync, and we'd
                    // rather drop a bad effect than fail the
                    // commit.
                    continue;
                };
                let outcome = fire_effect(entity, effect, &transaction, env).await?;
                merge_changes(&mut novelty, outcome.novelty);
                merge_changes(&mut transients, outcome.transients);
            }

            let mut next_commands = Vec::new();
            for occurrence in command_stimulus.into_occurrences() {
                let occurrence_entity = occurrence.occurrence().clone();
                let overlay = CommandBatch::new(vec![occurrence.clone()]).encode();
                transaction = transaction.integrate(overlay.clone());
                let effect_entities =
                    effects_for_command(&transaction, occurrence.command(), env).await?;
                summary
                    .registered_rules_by_occurrence
                    .insert(occurrence_entity.clone(), effect_entities.len());
                let mut fired = 0usize;
                for entity in effect_entities {
                    let Some(effect) = load_effect(&transaction, entity.clone(), env).await? else {
                        continue;
                    };
                    let effect = constrain_effect_to_occurrence(effect, &occurrence)?;
                    let outcome = fire_effect(entity, effect, &transaction, env).await?;
                    if outcome.firings > 0 {
                        fired += 1;
                    }
                    merge_changes(&mut novelty, outcome.novelty);
                    merge_changes(&mut transients, outcome.transients);
                    next_commands.extend(outcome.commands);
                }
                summary
                    .fired_rules_by_occurrence
                    .insert(occurrence_entity, fired);
                transaction = sweep_transients(overlay, transaction);
            }

            // 4. Sweep the round's incoming transients — they've
            //    served their purpose as triggers, retract them so
            //    they cancel at commit. Done BEFORE integrating
            //    novelty so a new transient emitted by an effect
            //    in this round is not accidentally swept by an
            //    incoming-transient retract for the same triple.
            transaction = sweep_transients(stimulus, transaction);

            // 5. Integrate all this round's novelty — durable and
            //    transient — into txn. The newly-emitted transients
            //    sit in txn until *their* round sweeps them; durable
            //    derivations stay for the commit.
            transaction = transaction.integrate(novelty);

            // 6. Promote this round's emitted transients as the
            //    next round's stimulus bucket. Empty → loop ends:
            //    no new triggers means no further rules can fire.
            stimulus = transients;
            command_stimulus = CommandBatch::new(next_commands);
        }

        Ok(InduceReport {
            transaction,
            summary,
        })
    }
}

fn constrain_effect_to_occurrence(
    effect: Effect,
    occurrence: &CommandOccurrence,
) -> Result<Effect, InduceError> {
    let polarity = effect.polarity();
    let mut descriptor = effect.descriptor();
    let mut occurrence_terms = Vec::new();
    for proposition in descriptor
        .when
        .iter_mut()
        .chain(descriptor.unless.iter_mut())
    {
        let Proposition::Concept(query) = proposition else {
            continue;
        };
        let Some((kind_field, _)) = query
            .predicate
            .with()
            .iter()
            .find(|(_, attribute)| attribute.the().to_string() == COMMAND_KIND_RELATION)
        else {
            continue;
        };
        if !matches!(
            query.terms.get(kind_field),
            Some(Term::Constant(Value::Entity(kind))) if kind == occurrence.command()
        ) {
            continue;
        }
        if let Some(term) = query.terms.get("this") {
            occurrence_terms.push(term.clone());
        }
    }
    for term in occurrence_terms {
        let equality = dialog_query::constraint::Equality::new(
            term,
            Term::<dialog_query::Any>::Constant(Value::Entity(occurrence.occurrence().clone())),
        );
        descriptor
            .when
            .push(Proposition::Constraint(equality.into()));
    }
    let rule = descriptor.compile().map_err(|error| {
        InduceError::Query(format!(
            "occurrence-constrained effect did not compile: {error}"
        ))
    })?;
    Ok(Effect::new(rule, polarity))
}

/// Sweep this round's transient instructions by emitting each
/// one's *inverse* into the transaction — assert+retract pairs
/// cancel at commit, so a transient that triggered this round
/// leaves no durable trace.
///
/// Called at end-of-round, AFTER firing and BEFORE integrating
/// the round's novelty, so an incoming-transient retract can't
/// accidentally cancel an identical new transient an effect
/// just emitted in the same round.
///
/// Each `Instruction` is inverted:
///
/// - `Assert` / `Replace` → emit a `Retract` of the same triple.
///   The fact was added as a one-shot trigger; remove it.
/// - `Retract` → emit an `Assert` of the same triple. A
///   transient-concept retraction (a user `retract!:` against a
///   transient concept, or a `retract!:`-polarity rule whose
///   head is a transient concept) made a fact temporarily
///   absent; restore it. No test exercises this path today, but
///   the symmetric inverse keeps the invariant "sweep undoes
///   the round's transient action, whatever its direction" —
///   without the arm a transient retraction would persist
///   durably, which contradicts the "transients leave no trace"
///   contract.
fn sweep_transients<'a>(transients: Changes, mut txn: Transaction<'a>) -> Transaction<'a> {
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
    txn
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

async fn effects_for_command<Env: InduceEnv>(
    txn: &Transaction<'_>,
    command: &Entity,
    env: &Env,
) -> Result<Vec<Entity>, InduceError> {
    let claims: Vec<dialog_query::Claim> = txn
        .query()
        .select(dialog_query::AttributeQuery::from(
            Term::<dialog_query::attribute::The>::from(the("dialog.effect", "command"))
                .of(Term::<Entity>::var("effect"))
                .is(Term::<Entity>::from(command.clone())),
        ))
        .perform(env)
        .try_vec()
        .await
        .map_err(|error| InduceError::Query(format!("command-index query failed: {error:?}")))?;
    let mut effects = claims.into_iter().map(|claim| claim.of).collect::<Vec<_>>();
    effects.sort();
    effects.dedup();
    Ok(effects)
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
struct FireOutcome {
    /// All the facts derived by this fire — durable + transient,
    /// asserts + retracts, accumulated as a `Changes` batch the
    /// caller integrates into the transaction AFTER the whole
    /// round's effects have fired. Keeping the round's input txn
    /// frozen during fires is the Dedalus-semantics fix:
    /// sibling rules read the same state, derived facts batch.
    novelty: Changes,
    /// Subset of `novelty`: just the transient-head asserts, so
    /// the cascade loop can promote them to the next round's
    /// stimulus bucket.
    transients: Changes,
    /// Nominal command heads promoted to the next fixpoint round.
    commands: Vec<CommandOccurrence>,
    /// Number of body frames produced by this rule.
    firings: usize,
}

/// Evaluate one effect's body against the transaction overlay
/// and instantiate the head per match. Returns the derived facts
/// as a [`FireOutcome`] WITHOUT mutating the transaction — the
/// cascade loop in [`Induce::perform`] batches every fire's
/// outcome and integrates them after the round so sibling rules
/// in one round can't read each other's mid-round writes.
///
/// For `Assert`-polarity rules each emitted head's facts land in
/// [`FireOutcome::novelty`]; transient-concept heads ALSO land
/// in [`FireOutcome::transients`] so the cascade loop can
/// promote them as the next round's stimulus.
///
/// For `Retract`-polarity rules each emitted head's facts land
/// in `novelty` as retracts. The head concept is expected to be
/// durable — retracts of a transient have no observable effect
/// — so the `transients` bucket is always empty for this
/// polarity.
async fn fire_effect<Env: InduceEnv>(
    effect_entity: Entity,
    effect: Effect,
    txn: &Transaction<'_>,
    env: &Env,
) -> Result<FireOutcome, InduceError> {
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
    let head_is_command = rule
        .conclusion()
        .with()
        .iter()
        .any(|(_, attribute)| attribute.the().to_string() == COMMAND_KIND_RELATION);
    let head_is_transient = match polarity {
        EffectPolarity::Assert if !head_is_command => {
            is_transient(txn, rule.conclusion().this(), env).await?
        }
        EffectPolarity::Retract => false,
        EffectPolarity::Assert => false,
    };

    let head = rule.conclusion().clone();
    let mut transients = Changes::new();
    let mut novelty = Changes::new();
    let mut commands = Vec::new();
    let firings = matches.len();
    for frame in matches {
        // Project the match into a `Parameters` map of the head's
        // operands. The conclusion-variable check at rule-compile
        // time guarantees every required operand is bound somewhere
        // in the body.
        //
        // `required_operands()` covers `this` + required keys;
        // optional keys are appended so a *present* optional still
        // emits its fact. An optional the frame lacks resolves to
        // `Absent` and is skipped by the `as_value()` guard, so no
        // fact is emitted for it.
        let mut parameters = Parameters::new();
        let operands = head.required_operands().map(str::to_owned).chain(
            head.with()
                .iter()
                .filter(|(_, attribute)| attribute.is_optional())
                .map(|(name, _)| name.to_owned()),
        );
        for operand in operands {
            if let Ok(binding) = frame.lookup(&Term::<dialog_query::Any>::var(&operand))
                && let Some(value) = binding.as_value()
            {
                parameters.insert(operand.clone(), Term::Constant(value.clone()));
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
            if head_is_command {
                commands.push(decode_command_head(&effect_entity, &concept_query, txn, env).await?);
                continue;
            }
            match polarity {
                EffectPolarity::Assert => {
                    if head_is_transient {
                        accumulate_head_facts(&concept_query, &mut transients);
                    }
                    emit_head_facts_into(concept_query, &mut novelty);
                }
                EffectPolarity::Retract => {
                    retract_head_facts_into(concept_query, &mut novelty);
                }
            }
        }
    }

    Ok(FireOutcome {
        novelty,
        transients,
        commands,
        firings,
    })
}

async fn decode_command_head<Env: InduceEnv>(
    effect: &Entity,
    query: &ConceptQuery,
    txn: &Transaction<'_>,
    env: &Env,
) -> Result<CommandOccurrence, InduceError> {
    let mut kind = None;
    let mut arguments = tonk_core::claim::ValueMap::new();
    for (field, attribute) in query.predicate.with().iter() {
        let relation = attribute.the().to_string();
        let value = query.terms.get(field).and_then(|term| match term {
            Term::Constant(value) => Some(value.clone()),
            _ => None,
        });
        if relation == COMMAND_KIND_RELATION {
            if let Some(Value::Entity(entity)) = value {
                kind = Some(entity);
            }
        } else if relation.starts_with(COMMAND_ARGUMENT_RELATION_PREFIX)
            && let Some(value) = value
        {
            arguments.insert(field.to_owned(), value);
        }
    }
    let kind = kind.ok_or_else(|| InduceError::InvalidCommandOutput {
        effect: Box::new(effect.clone()),
        reason: "private command head omitted its stable kind".into(),
    })?;
    let definition = CommandDefinition::by_entity(kind.clone())
        .resolve(&Source::from(txn), env)
        .await
        .map_err(|error| InduceError::InvalidCommandOutput {
            effect: Box::new(effect.clone()),
            reason: format!("schema resolution failed for {kind}: {error}"),
        })?
        .ok_or_else(|| InduceError::InvalidCommandOutput {
            effect: Box::new(effect.clone()),
            reason: format!("command schema {kind} is not installed"),
        })?;
    let validated = definition
        .schema()
        .validate(SourceInvocation {
            command: kind,
            arguments,
        })
        .map_err(|error| InduceError::InvalidCommandOutput {
            effect: Box::new(effect.clone()),
            reason: error.to_string(),
        })?;
    let occurrence = Entity::new().map_err(|error| InduceError::InvalidCommandOutput {
        effect: Box::new(effect.clone()),
        reason: format!("could not allocate occurrence: {error}"),
    })?;
    Ok(CommandOccurrence::new(
        validated,
        InvocationMetadata::new(occurrence.clone(), format!("rule:{effect}:{occurrence}")),
    ))
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
/// walk, and by `tonk_evaluator::evaluate` to seed the transient bucket
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
/// asserted-notation planner uses in `tonk_schema::transact`, but
/// writes directly into a dialog `Transaction` since the
/// induce path doesn't go through `ApplicationPlan`.
fn emit_head_facts_into(concept_query: ConceptQuery, changes: &mut Changes) {
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
            Cardinality::One => RawReplace {
                the,
                of: this_entity.clone(),
                is: value.clone(),
            }
            .assert(changes),
            Cardinality::Many => RawClaim {
                the,
                of: this_entity.clone(),
                is: value.clone(),
            }
            .assert(changes),
        }
    }
}

/// Retract-polarity sibling of [`emit_head_facts`]. Walks a
/// fully-bound head and emits one retract per bound field so
/// the body's match-bound values are dissociated from the
/// underlying entity. Cardinality doesn't change the retract
/// path — both one and many fields dissociate by the exact
/// `(attr, this, value)` triple.
fn retract_head_facts_into(concept_query: ConceptQuery, changes: &mut Changes) {
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
        RawClaim {
            the,
            of: this_entity.clone(),
            is: value.clone(),
        }
        .retract(changes);
    }
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

    use dialog_artifacts::Statement;
    use dialog_query::artifact::Type;
    use dialog_query::attribute::Cardinality as DialogCardinality;
    use dialog_query::concept::descriptor::ConceptDescriptor;
    use dialog_query::concept::query::ConceptQuery;
    use dialog_query::premise::Premise as DialogPremise;
    use dialog_query::{AttributeDescriptor, InductiveRule, Parameters as DialogParameters};
    use tonk_core::command::{CommandSchema, InvocationMetadata, SourceInvocation};
    use tonk_core::effect::{Effect, EffectPolarity};
    use tonk_schema::command_definition::CommandDefinition;
    use tonk_schema::concept::{AnonymousConcept, TransientConcept};
    use tonk_schema::rule::Rule;

    /// A 1-field concept descriptor with a configurable field
    /// type. Helper because tests below build several.
    fn one_field_concept(domain: &str, name: &str, ty: Type) -> ConceptDescriptor {
        ConceptDescriptor::try_from(vec![(
            name,
            AttributeDescriptor::new(
                format!("{domain}/{name}").parse().unwrap(),
                "",
                DialogCardinality::One,
                Some(ty),
            ),
        )])
        .unwrap()
    }

    /// Shorthand for the common String case used by the early
    /// tests below.
    fn one_text_field_concept(domain: &str, name: &str) -> ConceptDescriptor {
        one_field_concept(domain, name, Type::String)
    }

    fn nominal_schema() -> CommandSchema {
        CommandSchema {
            required: [(
                "title".into(),
                AttributeDescriptor::new(
                    "xyz.tonk.todo/title".parse().unwrap(),
                    "",
                    DialogCardinality::One,
                    Some(Type::String),
                ),
            )]
            .into_iter()
            .collect(),
            optional: Default::default(),
        }
    }

    fn evolved_nominal_schema() -> CommandSchema {
        let mut schema = nominal_schema();
        schema.required.insert(
            "note".into(),
            AttributeDescriptor::new(
                "xyz.tonk.todo/note".parse().unwrap(),
                "",
                DialogCardinality::One,
                Some(Type::String),
            ),
        );
        schema
    }

    fn nominal_premise(kind: Entity) -> DialogPremise {
        let predicate = nominal_private_predicate();
        let mut terms = DialogParameters::new();
        terms.insert("this".into(), Term::var("this"));
        terms.insert("__command_kind".into(), Term::Constant(Value::Entity(kind)));
        terms.insert("title".into(), Term::var("title"));
        DialogPremise::Assert(Proposition::Concept(ConceptQuery { terms, predicate }))
    }

    fn nominal_private_predicate() -> ConceptDescriptor {
        ConceptDescriptor::try_from(vec![
            (
                "__command_kind",
                AttributeDescriptor::new(
                    COMMAND_KIND_RELATION.parse().unwrap(),
                    "",
                    DialogCardinality::One,
                    Some(Type::Entity),
                ),
            ),
            (
                "title",
                AttributeDescriptor::new(
                    "dialog.command.argument/title".parse().unwrap(),
                    "",
                    DialogCardinality::One,
                    Some(Type::String),
                ),
            ),
        ])
        .unwrap()
    }

    fn command_to_command_effect(from: Entity, to: Entity) -> Effect {
        let equality = dialog_query::constraint::Equality::new(
            Term::<dialog_query::Any>::var("__command_kind"),
            Term::<dialog_query::Any>::Constant(Value::Entity(to)),
        );
        Effect::asserting(
            InductiveRule::new(
                nominal_private_predicate(),
                vec![
                    nominal_premise(from),
                    DialogPremise::Assert(Proposition::Constraint(equality.into())),
                ],
            )
            .unwrap(),
        )
    }

    fn nominal_occurrence(kind: Entity, title: &str) -> CommandOccurrence {
        let validated = nominal_schema()
            .validate(SourceInvocation {
                command: kind,
                arguments: [("title".into(), Value::String(title.into()))]
                    .into_iter()
                    .collect(),
            })
            .unwrap();
        let occurrence = Entity::new().unwrap();
        CommandOccurrence::new(
            validated,
            InvocationMetadata::new(occurrence.clone(), format!("test:{occurrence}")),
        )
    }

    #[dialog_common::test]
    async fn nominal_command_appends_durable_fact_and_leaves_no_private_facts() -> anyhow::Result<()>
    {
        let (operator, profile) = test_operator_with_profile().await;
        let repo = test_repo(&operator, &profile).await;
        let branch = repo.branch("main").open().perform(&operator).await?;
        let kind: Entity = "id:todo/add".parse()?;
        let target = one_text_field_concept("xyz.tonk.todo", "title");
        let effect = Effect::asserting(
            InductiveRule::new(target.clone(), vec![nominal_premise(kind.clone())]).unwrap(),
        );
        let mut install = branch.transaction();
        install = install_attribute_facts(install, &target);
        install = install.assert(AnonymousConcept::new(target));
        install = install.assert(CommandDefinition::asserting(kind.clone(), nominal_schema()));
        install = install.assert(Rule::asserting(effect));
        install.commit().perform(&operator).await?;

        let occurrence = nominal_occurrence(kind, "Buy milk");
        let occurrence_entity = occurrence.occurrence().clone();
        let report = branch
            .transaction()
            .induce_commands(Changes::new(), CommandBatch::new(vec![occurrence]))
            .perform_report(&operator)
            .await?;
        assert_eq!(
            report
                .summary
                .registered_rules_by_occurrence
                .get(&occurrence_entity),
            Some(&1)
        );
        assert_eq!(
            report
                .summary
                .fired_rules_by_occurrence
                .get(&occurrence_entity),
            Some(&1)
        );
        report.transaction.commit().perform(&operator).await?;

        let titles: Vec<dialog_query::Claim> = branch
            .query()
            .select(dialog_query::AttributeQuery::from(
                Term::from(the!("xyz.tonk.todo/title"))
                    .of(Term::from(occurrence_entity.clone()))
                    .is(Term::<String>::var("title")),
            ))
            .perform(&operator)
            .try_vec()
            .await?;
        assert_eq!(titles.len(), 1);
        let kind_claims: Vec<dialog_query::Claim> = branch
            .query()
            .select(dialog_query::AttributeQuery::from(
                Term::from(
                    COMMAND_KIND_RELATION
                        .parse::<dialog_query::attribute::The>()
                        .unwrap(),
                )
                .of(Term::from(occurrence_entity.clone()))
                .is(Term::<Entity>::var("kind")),
            ))
            .perform(&operator)
            .try_vec()
            .await?;
        let argument_claims: Vec<dialog_query::Claim> = branch
            .query()
            .select(dialog_query::AttributeQuery::from(
                Term::from(
                    "dialog.command.argument/title"
                        .parse::<dialog_query::attribute::The>()
                        .unwrap(),
                )
                .of(Term::from(occurrence_entity.clone()))
                .is(Term::<String>::var("title")),
            ))
            .perform(&operator)
            .try_vec()
            .await?;
        assert!(kind_claims.is_empty(), "private command kind leaked");
        assert!(
            argument_claims.is_empty(),
            "private command argument leaked"
        );
        Ok(())
    }

    #[dialog_common::test]
    async fn nominal_repeated_occurrences_fire_independently() -> anyhow::Result<()> {
        let (operator, profile) = test_operator_with_profile().await;
        let repo = test_repo(&operator, &profile).await;
        let branch = repo.branch("main").open().perform(&operator).await?;
        let kind: Entity = "id:todo/add".parse()?;
        let target = one_text_field_concept("xyz.tonk.todo", "title");
        let effect = Effect::asserting(
            InductiveRule::new(target.clone(), vec![nominal_premise(kind.clone())]).unwrap(),
        );
        let mut install = branch.transaction();
        install = install_attribute_facts(install, &target);
        install = install.assert(AnonymousConcept::new(target));
        install = install.assert(CommandDefinition::asserting(kind.clone(), nominal_schema()));
        install = install.assert(Rule::asserting(effect));
        install.commit().perform(&operator).await?;

        let first = nominal_occurrence(kind.clone(), "Same title");
        let second = nominal_occurrence(kind, "Same title");
        let ids = [first.occurrence().clone(), second.occurrence().clone()];
        let report = branch
            .transaction()
            .induce_commands(Changes::new(), CommandBatch::new(vec![first, second]))
            .perform_report(&operator)
            .await?;
        for id in &ids {
            assert_eq!(
                report.summary.registered_rules_by_occurrence.get(id),
                Some(&1)
            );
            assert_eq!(report.summary.fired_rules_by_occurrence.get(id), Some(&1));
        }
        report.transaction.commit().perform(&operator).await?;
        for id in ids {
            let claims: Vec<dialog_query::Claim> = branch
                .query()
                .select(dialog_query::AttributeQuery::from(
                    Term::from(the!("xyz.tonk.todo/title"))
                        .of(Term::from(id))
                        .is(Term::<String>::var("title")),
                ))
                .perform(&operator)
                .try_vec()
                .await?;
            assert_eq!(claims.len(), 1);
        }
        Ok(())
    }

    #[dialog_common::test]
    async fn nominal_command_to_command_runs_two_rounds() -> anyhow::Result<()> {
        let (operator, profile) = test_operator_with_profile().await;
        let repo = test_repo(&operator, &profile).await;
        let branch = repo.branch("main").open().perform(&operator).await?;
        let command_a: Entity = "id:command/a".parse()?;
        let command_b: Entity = "id:command/b".parse()?;
        let target = one_text_field_concept("xyz.tonk.final", "title");
        let finish = Effect::asserting(
            InductiveRule::new(target.clone(), vec![nominal_premise(command_b.clone())]).unwrap(),
        );
        let mut install = branch.transaction();
        install = install_attribute_facts(install, &target);
        install = install.assert(AnonymousConcept::new(target));
        install = install.assert(CommandDefinition::asserting(
            command_a.clone(),
            nominal_schema(),
        ));
        install = install.assert(CommandDefinition::asserting(
            command_b.clone(),
            nominal_schema(),
        ));
        install = install.assert(Rule::asserting(command_to_command_effect(
            command_a.clone(),
            command_b,
        )));
        install = install.assert(Rule::asserting(finish));
        install.commit().perform(&operator).await?;

        let seed = nominal_occurrence(command_a, "Two rounds");
        let seed_id = seed.occurrence().clone();
        let report = branch
            .transaction()
            .induce_commands(Changes::new(), CommandBatch::new(vec![seed]))
            .perform_report(&operator)
            .await?;
        assert_eq!(
            report.summary.registered_rules_by_occurrence.get(&seed_id),
            Some(&1)
        );
        assert_eq!(
            report.summary.fired_rules_by_occurrence.get(&seed_id),
            Some(&1)
        );
        assert_eq!(report.summary.registered_rules_by_occurrence.len(), 2);
        report.transaction.commit().perform(&operator).await?;
        let claims: Vec<dialog_query::Claim> = branch
            .query()
            .select(dialog_query::AttributeQuery::from(
                Term::from(the!("xyz.tonk.final/title"))
                    .of(Term::<Entity>::var("occurrence"))
                    .is(Term::<String>::var("title")),
            ))
            .perform(&operator)
            .try_vec()
            .await?;
        assert_eq!(claims.len(), 1);
        assert_eq!(claims[0].is, Value::String("Two rounds".into()));
        Ok(())
    }

    #[dialog_common::test]
    async fn nominal_registered_rule_can_have_zero_matches() -> anyhow::Result<()> {
        let (operator, profile) = test_operator_with_profile().await;
        let repo = test_repo(&operator, &profile).await;
        let branch = repo.branch("main").open().perform(&operator).await?;
        let kind: Entity = "id:todo/add".parse()?;
        let target = one_text_field_concept("xyz.tonk.target", "title");
        let guard = one_text_field_concept("xyz.tonk.guard", "present");
        let mut guard_terms = DialogParameters::new();
        guard_terms.insert("this".into(), Term::var("this"));
        guard_terms.insert("present".into(), Term::<dialog_query::Any>::blank());
        let guard_premise = DialogPremise::Assert(Proposition::Concept(ConceptQuery {
            terms: guard_terms,
            predicate: guard.clone(),
        }));
        let effect = Effect::asserting(
            InductiveRule::new(
                target.clone(),
                vec![nominal_premise(kind.clone()), guard_premise],
            )
            .unwrap(),
        );
        let mut install = branch.transaction();
        install = install_attribute_facts(install, &target);
        install = install_attribute_facts(install, &guard);
        install = install.assert(AnonymousConcept::new(target));
        install = install.assert(AnonymousConcept::new(guard));
        install = install.assert(CommandDefinition::asserting(kind.clone(), nominal_schema()));
        install = install.assert(Rule::asserting(effect));
        install.commit().perform(&operator).await?;

        let occurrence = nominal_occurrence(kind, "No guard");
        let id = occurrence.occurrence().clone();
        let report = branch
            .transaction()
            .induce_commands(Changes::new(), CommandBatch::new(vec![occurrence]))
            .perform_report(&operator)
            .await?;
        assert_eq!(
            report.summary.registered_rules_by_occurrence.get(&id),
            Some(&1)
        );
        assert_eq!(report.summary.fired_rules_by_occurrence.get(&id), Some(&0));
        report.transaction.commit().perform(&operator).await?;
        Ok(())
    }

    #[dialog_common::test]
    async fn nominal_kind_isolation_excludes_sibling_and_legacy_rules() -> anyhow::Result<()> {
        let (operator, profile) = test_operator_with_profile().await;
        let repo = test_repo(&operator, &profile).await;
        let branch = repo.branch("main").open().perform(&operator).await?;
        let command_a: Entity = "id:command/a".parse()?;
        let command_b: Entity = "id:command/b".parse()?;
        let target_a = one_text_field_concept("xyz.tonk.output-a", "title");
        let target_b = one_text_field_concept("xyz.tonk.output-b", "title");
        let legacy_target = one_text_field_concept("xyz.tonk.output-legacy", "title");
        let effect_a = Effect::asserting(
            InductiveRule::new(target_a.clone(), vec![nominal_premise(command_a.clone())]).unwrap(),
        );
        let effect_b = Effect::asserting(
            InductiveRule::new(target_b.clone(), vec![nominal_premise(command_b.clone())]).unwrap(),
        );
        let legacy_input = one_text_field_concept("xyz.tonk.todo", "title");
        let mut legacy_terms = DialogParameters::new();
        legacy_terms.insert("this".into(), Term::var("this"));
        legacy_terms.insert("title".into(), Term::var("title"));
        let legacy_effect = Effect::asserting(
            InductiveRule::new(
                legacy_target.clone(),
                vec![DialogPremise::Assert(Proposition::Concept(ConceptQuery {
                    terms: legacy_terms,
                    predicate: legacy_input.clone(),
                }))],
            )
            .unwrap(),
        );
        let mut install = branch.transaction();
        for descriptor in [&target_a, &target_b, &legacy_target, &legacy_input] {
            install = install_attribute_facts(install, descriptor);
            install = install.assert(AnonymousConcept::new(descriptor.clone()));
        }
        install = install.assert(CommandDefinition::asserting(
            command_a.clone(),
            nominal_schema(),
        ));
        install = install.assert(CommandDefinition::asserting(command_b, nominal_schema()));
        install = install.assert(Rule::asserting(effect_a));
        install = install.assert(Rule::asserting(effect_b));
        install = install.assert(Rule::asserting(legacy_effect));
        install.commit().perform(&operator).await?;

        let occurrence = nominal_occurrence(command_a, "Only A");
        let id = occurrence.occurrence().clone();
        let report = branch
            .transaction()
            .induce_commands(Changes::new(), CommandBatch::new(vec![occurrence]))
            .perform_report(&operator)
            .await?;
        assert_eq!(
            report.summary.registered_rules_by_occurrence.get(&id),
            Some(&1)
        );
        report.transaction.commit().perform(&operator).await?;
        for (relation, expected) in [
            ("xyz.tonk.output-a/title", 1usize),
            ("xyz.tonk.output-b/title", 0),
            ("xyz.tonk.output-legacy/title", 0),
        ] {
            let claims: Vec<dialog_query::Claim> = branch
                .query()
                .select(dialog_query::AttributeQuery::from(
                    Term::from(relation.parse::<dialog_query::attribute::The>().unwrap())
                        .of(Term::<Entity>::var("entity"))
                        .is(Term::<String>::var("title")),
                ))
                .perform(&operator)
                .try_vec()
                .await?;
            assert_eq!(claims.len(), expected, "{relation}");
        }
        Ok(())
    }

    #[dialog_common::test]
    async fn nominal_command_cycle_fails_without_commit() -> anyhow::Result<()> {
        let (operator, profile) = test_operator_with_profile().await;
        let repo = test_repo(&operator, &profile).await;
        let branch = repo.branch("main").open().perform(&operator).await?;
        let command_a: Entity = "id:command/a".parse()?;
        let command_b: Entity = "id:command/b".parse()?;
        let mut install = branch.transaction();
        install = install.assert(CommandDefinition::asserting(
            command_a.clone(),
            nominal_schema(),
        ));
        install = install.assert(CommandDefinition::asserting(
            command_b.clone(),
            nominal_schema(),
        ));
        install = install.assert(Rule::asserting(command_to_command_effect(
            command_a.clone(),
            command_b.clone(),
        )));
        install = install.assert(Rule::asserting(command_to_command_effect(
            command_b,
            command_a.clone(),
        )));
        install.commit().perform(&operator).await?;
        let before = branch.revision();
        let seed = nominal_occurrence(command_a, "Cycle");
        let result = branch
            .transaction()
            .induce_commands(Changes::new(), CommandBatch::new(vec![seed]))
            .perform_report(&operator)
            .await;
        match result {
            Err(InduceError::NonTerminating(MAX_ROUNDS)) => {}
            Err(other) => panic!("expected NonTerminating, got {other:?}"),
            Ok(_) => panic!("expected command cycle to fail"),
        }
        assert_eq!(branch.revision(), before);
        Ok(())
    }

    #[dialog_common::test]
    async fn nominal_rule_output_is_validated_against_current_schema() -> anyhow::Result<()> {
        let (operator, profile) = test_operator_with_profile().await;
        let repo = test_repo(&operator, &profile).await;
        let branch = repo.branch("main").open().perform(&operator).await?;
        let command_a: Entity = "id:command/a".parse()?;
        let command_b: Entity = "id:command/b".parse()?;
        let effect = command_to_command_effect(command_a.clone(), command_b.clone());
        let effect_entity = effect.this();
        let mut install = branch.transaction();
        install = install.assert(CommandDefinition::asserting(
            command_a.clone(),
            nominal_schema(),
        ));
        install = install.assert(CommandDefinition::asserting(
            command_b,
            evolved_nominal_schema(),
        ));
        install = install.assert(Rule::asserting(effect));
        install.commit().perform(&operator).await?;
        let seed = nominal_occurrence(command_a, "Missing note");
        let result = branch
            .transaction()
            .induce_commands(Changes::new(), CommandBatch::new(vec![seed]))
            .perform_report(&operator)
            .await;
        match result {
            Err(InduceError::InvalidCommandOutput { effect, reason }) => {
                assert_eq!(*effect, effect_entity);
                assert!(reason.contains("note"), "{reason}");
            }
            Err(other) => panic!("expected InvalidCommandOutput, got {other:?}"),
            Ok(_) => panic!("invalid rule output should fail induction"),
        }
        Ok(())
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
        install = install.assert(Rule::asserting(effect));
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
        install = install.assert(Rule::asserting(effect_a));
        install = install.assert(Rule::asserting(effect_b));
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
        install = install.assert(Rule::asserting(effect));
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
        install = install.assert(Rule::asserting(effect));
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
        install = install.assert(Rule::asserting(effect));
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
        install = install.assert(Rule::asserting(effect));
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
        let cmd = ConceptDescriptor::try_from(vec![(
            "tag",
            AttributeDescriptor::new(
                "io.gozala.bag-cmd/tag".parse().unwrap(),
                "",
                DialogCardinality::Many,
                Some(Type::String),
            ),
        )])
        .unwrap();
        let bag = ConceptDescriptor::try_from(vec![(
            "tag",
            AttributeDescriptor::new(
                "io.gozala.bag/tag".parse().unwrap(),
                "",
                DialogCardinality::Many,
                Some(Type::String),
            ),
        )])
        .unwrap();

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
        install = install.assert(Rule::asserting(effect));
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
