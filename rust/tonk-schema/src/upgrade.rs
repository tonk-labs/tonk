//! Moves the attribute and concept definitions an earlier release wrote
//! to the facts and identities this release gives them.
//!
//! An earlier release recorded an attribute's value type as the string
//! dialog serialized it as (`db.attribute/type "Text"`) and its arity as
//! `db.attribute/cardinality`. This release records the type as the
//! entity dialog names it by (`db.attribute/as text:`) and which claims a
//! read picks as the entity dialog names the pick by
//! (`db.attribute/pick all:`), and hashes both into the
//! attribute's identity, so every typed attribute's identity changed, and
//! the identity of every concept over one.
//!
//! [`upgrade_definitions`] reads every attribute still recorded the
//! earlier way, whoever wrote it: a library, a template, the space
//! itself. It writes each one again under its current identity, writes
//! again every concept whose fields read one of them, and makes every
//! claim that named an earlier identity name the current one: a name, a
//! route, a view, a field of another definition. What a space wrote
//! about an earlier entity that is not part of its definition moves with
//! it. Rules embed their concepts rather than naming them, and dialog's
//! `Branch::upgrade_rules` re-installs them.
//!
//! The upgrade is idempotent: a second run finds no attribute recorded
//! the earlier way.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use dialog_artifacts::{
    Artifact, ArtifactSelector, Entity, Pick, Relation as ArtifactsRelation, Statement, Update,
    Value,
};
use dialog_query::{AttributeDescriptor, ConceptDescriptor, ConceptFieldDescriptor};
use dialog_repository::Branch;
use futures_util::StreamExt as _;

use crate::concept::{
    AnonymousConcept, AttributeByEntity, QueryEnv, TransientConcept, attribute_statements,
};
use crate::query_source::Source;

/// What [`upgrade_definitions`] did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct DefinitionsUpgraded {
    /// Each attribute and concept that moved: its earlier identity and
    /// its identity now.
    pub moves: Vec<(Entity, Entity)>,
    /// Attributes and concepts written again in place, under an identity
    /// that did not change.
    pub rewritten: Vec<Entity>,
    /// Attributes recorded the earlier way that do not describe an
    /// attribute this release reads, left as they are.
    pub undecodable: Vec<Entity>,
}

impl DefinitionsUpgraded {
    /// Whether the upgrade found nothing to do.
    pub fn is_empty(&self) -> bool {
        self.moves.is_empty() && self.rewritten.is_empty()
    }
}

/// The relations an earlier release recorded an attribute with that this
/// release does not: their presence marks an attribute to upgrade.
const LEGACY_ATTRIBUTE: [&str; 3] = [
    "db.attribute/type",
    "db.attribute/cardinality",
    "db.attribute/select",
];

/// Upgrade every attribute and concept definition `branch` holds in the
/// earlier shape, in one commit. See the [module documentation](self).
pub async fn upgrade_definitions<Env>(
    branch: &Branch,
    env: &Env,
) -> Result<DefinitionsUpgraded, String>
where
    Env: QueryEnv
        + dialog_capability::Provider<dialog_effects::memory::Publish>
        + dialog_capability::Provider<dialog_effects::archive::Import>
        + dialog_capability::Provider<dialog_effects::authority::Attest>
        + dialog_capability::Provider<dialog_effects::blob::Import>
        + dialog_capability::Provider<dialog_effects::blob::Size>,
{
    let mut upgraded = DefinitionsUpgraded::default();
    let mut writes: Vec<Write> = Vec::new();

    // Attributes recorded the earlier way, keyed by the entity they are
    // recorded under, with the descriptor their facts spell.
    let mut legacy: BTreeSet<Entity> = BTreeSet::new();
    for relation in LEGACY_ATTRIBUTE {
        for claim in select(
            branch,
            env,
            ArtifactSelector::new().the(relation_named(relation)?),
        )
        .await?
        {
            legacy.insert(claim.of);
        }
    }
    let mut attributes: HashMap<Entity, AttributeDescriptor> = HashMap::new();
    for entity in legacy {
        let claims = select(branch, env, ArtifactSelector::new().of(entity.clone())).await?;
        let Some(descriptor) = legacy_descriptor(&claims) else {
            upgraded.undecodable.push(entity);
            continue;
        };
        let now: Entity = descriptor
            .to_uri()
            .parse()
            .map_err(|e| format!("attribute identity is no entity: {e}"))?;
        for claim in claims
            .iter()
            .filter(|claim| is_attribute_definition(&claim.the))
        {
            writes.push(Write::retract(claim));
        }
        for statement in attribute_statements(&descriptor)? {
            writes.push(Write::Statement(statement));
        }
        if now == entity {
            upgraded.rewritten.push(entity.clone());
        } else {
            upgraded.moves.push((entity.clone(), now));
        }
        attributes.insert(entity, descriptor);
    }

    // Concepts with a field over one of them: written again over the
    // current attributes, under the identity that gives them. A concept
    // named by an entity of its own (`tonk:workspace/shell`) keeps it.
    let marker = relation_named("db.meta/concept")?;
    let mut concepts: BTreeSet<Entity> = BTreeSet::new();
    for claim in select(branch, env, ArtifactSelector::new().the(marker)).await? {
        concepts.insert(claim.of);
    }
    let source = Source::from(branch);
    for entity in concepts {
        let claims = select(branch, env, ArtifactSelector::new().of(entity.clone())).await?;
        let mut fields: BTreeMap<String, Entity> = BTreeMap::new();
        let mut optional: BTreeSet<String> = BTreeSet::new();
        let mut description = None;
        let mut transient = false;
        for claim in &claims {
            let the = claim.the.as_str();
            if let Some(field) = the.strip_prefix("db.concept.with/")
                && let Value::Entity(attribute) = &claim.is
            {
                fields.insert(field.to_owned(), attribute.clone());
            } else if let Some(field) = the.strip_prefix("db.concept.optional/")
                && claim.is == Value::Boolean(true)
            {
                optional.insert(field.to_owned());
            } else if the == "db.meta/description"
                && let Value::String(text) = &claim.is
            {
                description = Some(text.clone());
            } else if the == "dialog.concept/transient" && claim.is == Value::Boolean(true) {
                transient = true;
            }
        }
        if !fields
            .values()
            .any(|attribute| attributes.contains_key(attribute))
        {
            continue;
        }
        let mut with: Vec<(String, ConceptFieldDescriptor)> = Vec::new();
        let mut complete = true;
        for (field, attribute) in &fields {
            let descriptor = match attributes.get(attribute) {
                Some(descriptor) => descriptor.clone(),
                None => match AttributeByEntity::new(attribute.clone())
                    .resolve(&source, env)
                    .await
                    .map_err(|e| e.to_string())?
                {
                    Some(current) => current.descriptor,
                    None => {
                        complete = false;
                        break;
                    }
                },
            };
            with.push((
                field.clone(),
                if optional.contains(field) {
                    ConceptFieldDescriptor::optional(descriptor)
                } else {
                    ConceptFieldDescriptor::required(descriptor)
                },
            ));
        }
        if !complete {
            continue;
        }
        let Ok(mut descriptor) = ConceptDescriptor::try_from(with) else {
            continue;
        };
        if let Some(description) = description {
            descriptor = descriptor.with_description(description);
        }
        let now = if entity.as_str().starts_with("concept:") {
            descriptor.this()
        } else {
            entity.clone()
        };
        for claim in claims
            .iter()
            .filter(|claim| is_concept_definition(&claim.the))
        {
            writes.push(Write::retract(claim));
        }
        writes.push(if transient {
            Write::Transient(TransientConcept {
                this: now.clone(),
                descriptor,
            })
        } else {
            Write::Concept(AnonymousConcept {
                this: now.clone(),
                descriptor,
            })
        });
        if now == entity {
            upgraded.rewritten.push(entity);
        } else {
            upgraded.moves.push((entity, now));
        }
    }

    // Every claim naming an earlier identity names the current one, and
    // what the space wrote about an earlier entity moves with it. A
    // concept's own fields were written again above.
    let current: HashMap<Entity, Entity> = upgraded.moves.iter().cloned().collect();
    let follow = |value: &Value| match value {
        Value::Entity(entity) => current
            .get(entity)
            .map(|now| Value::Entity(now.clone()))
            .unwrap_or_else(|| value.clone()),
        other => other.clone(),
    };
    for (earlier, now) in &upgraded.moves {
        let naming = select(
            branch,
            env,
            ArtifactSelector::new().is(Value::Entity(earlier.clone())),
        )
        .await?;
        for claim in naming {
            if claim.the.as_str().starts_with("db.concept.with/") || current.contains_key(&claim.of)
            {
                continue;
            }
            writes.push(Write::retract(&claim));
            writes.push(Write::Raw(Raw {
                the: claim.the.clone(),
                of: claim.of.clone(),
                is: follow(&claim.is),
                pick: Pick::All,
            }));
        }
        let about = select(branch, env, ArtifactSelector::new().of(earlier.clone())).await?;
        let mut by_relation: BTreeMap<String, Vec<Artifact>> = BTreeMap::new();
        for claim in about {
            if is_attribute_definition(&claim.the) || is_concept_definition(&claim.the) {
                continue;
            }
            by_relation
                .entry(claim.the.as_str().to_owned())
                .or_default()
                .push(claim);
        }
        for claims in by_relation.into_values() {
            let pick = if claims.len() == 1 {
                Pick::Last
            } else {
                Pick::All
            };
            for claim in claims {
                writes.push(Write::retract(&claim));
                writes.push(Write::Raw(Raw {
                    the: claim.the.clone(),
                    of: now.clone(),
                    is: follow(&claim.is),
                    pick: pick.clone(),
                }));
            }
        }
    }

    if upgraded.is_empty() {
        return Ok(upgraded);
    }
    let mut transaction = branch.transaction();
    for write in writes {
        transaction = match write {
            Write::Statement(statement) => transaction.assert(statement),
            Write::Concept(concept) => transaction.assert(concept),
            Write::Transient(concept) => transaction.assert(concept),
            Write::Raw(raw) => transaction.assert(raw),
            Write::Retract(raw) => transaction.retract(raw),
        };
    }
    transaction
        .commit()
        .publish()
        .perform(env)
        .await
        .map_err(|e| e.to_string())?;
    upgraded.moves.sort();
    upgraded.rewritten.sort();
    Ok(upgraded)
}

/// The descriptor an attribute recorded the earlier way spells, from its
/// `db.attribute/id`, `type`, `cardinality`, `select`, `among` and
/// `db.meta/description` claims.
fn legacy_descriptor(claims: &[Artifact]) -> Option<AttributeDescriptor> {
    let text = |name: &str| {
        claims.iter().find_map(|claim| match &claim.is {
            Value::String(text) if claim.the.as_str() == name => Some(text.clone()),
            _ => None,
        })
    };
    let id = text("db.attribute/id")?;
    let mut shape = serde_json::Map::new();
    let the: dialog_query::attribute::The = id.parse().ok()?;
    shape.insert("the".to_owned(), serde_json::to_value(the).ok()?);
    if let Some(kind) = text("db.attribute/type").filter(|kind| !kind.is_empty()) {
        shape.insert("as".to_owned(), serde_json::Value::String(kind));
    }
    if let Some(cardinality) = text("db.attribute/cardinality").filter(|c| !c.is_empty()) {
        shape.insert(
            "cardinality".to_owned(),
            serde_json::Value::String(cardinality),
        );
    }
    if let Some(select) = text("db.attribute/select").filter(|s| !s.is_empty()) {
        shape.insert("pick".to_owned(), serde_json::Value::String(select));
    }
    if let Some(among) = text("db.attribute/among") {
        shape.insert("as".to_owned(), serde_json::from_str(&among).ok()?);
    }
    if let Some(description) = text("db.meta/description") {
        shape.insert(
            "description".to_owned(),
            serde_json::Value::String(description),
        );
    }
    serde_json::from_value(serde_json::Value::Object(shape)).ok()
}

/// Whether a claim about an attribute entity is part of its definition,
/// which the upgrade writes again from the descriptor.
fn is_attribute_definition(the: &ArtifactsRelation) -> bool {
    let the = the.as_str();
    the.starts_with("db.attribute/") || the == "db.meta/description"
}

/// Whether a claim about a concept entity is part of its definition,
/// which the upgrade writes again from the descriptor.
fn is_concept_definition(the: &ArtifactsRelation) -> bool {
    let the = the.as_str();
    the.starts_with("db.concept.")
        || the == "db.meta/concept"
        || the == "db.meta/description"
        || the == "dialog.concept/transient"
}

fn relation_named(name: &str) -> Result<ArtifactsRelation, String> {
    name.parse()
        .map_err(|e| format!("{name} is no relation: {e}"))
}

/// Every claim `selector` matches on `branch`.
async fn select<Env: QueryEnv>(
    branch: &Branch,
    env: &Env,
    selector: ArtifactSelector<dialog_artifacts::selector::Constrained>,
) -> Result<Vec<Artifact>, String> {
    let stream = branch
        .claims()
        .select(selector)
        .perform(env)
        .await
        .map_err(|e| e.to_string())?;
    futures_util::pin_mut!(stream);
    let mut claims = Vec::new();
    while let Some(claim) = stream.next().await {
        let claim = claim.map_err(|e| e.to_string())?;
        claims.push(claim.to_owned().map_err(|e| e.to_string())?);
    }
    Ok(claims)
}

/// One write of the upgrade's commit.
enum Write {
    Statement(dialog_query::AttributeStatement),
    Concept(AnonymousConcept),
    Transient(TransientConcept),
    Raw(Raw),
    Retract(Raw),
}

impl Write {
    fn retract(claim: &Artifact) -> Self {
        Write::Retract(Raw {
            the: claim.the.clone(),
            of: claim.of.clone(),
            is: claim.is.clone(),
            pick: Pick::All,
        })
    }
}

/// A claim by its raw relation, which a definition's field relation
/// (`db.concept.with/<field>`) may not satisfy the stricter validated
/// relation of a statement.
struct Raw {
    the: ArtifactsRelation,
    of: Entity,
    is: Value,
    pick: Pick,
}

impl Statement for Raw {
    fn assert(self, update: &mut impl Update) {
        update.associate(self.the, self.of, self.is, self.pick);
    }

    fn retract(self, update: &mut impl Update) {
        update.dissociate(self.the, self.of, self.is);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::concept::{Concept, lookup_named_entity};
    use dialog_peer::helpers::{test_repo, test_session_with_peer};
    use dialog_query::migration::{attribute_uri_v0, concept_identity_v0};
    use tonk_core::meta::{Name, name};

    fn raw(the: &str, of: &Entity, is: Value, pick: Pick) -> anyhow::Result<Raw> {
        Ok(Raw {
            the: the.parse()?,
            of: of.clone(),
            is,
            pick,
        })
    }

    /// What the earlier release wrote for a concept over a text `title`
    /// and a set of text `tags` (the type as its serde name, the arity as
    /// its cardinality, each under its earlier identity), with a name
    /// pointing at the concept and a view the space wrote about it, moves
    /// to the identities this release gives it: the concept resolves under
    /// its current identity, the name and the view follow it, nothing is
    /// left on the earlier entities, and a second run finds nothing.
    #[dialog_common::test]
    async fn it_moves_definitions_recorded_the_earlier_way() -> anyhow::Result<()> {
        let (operator, profile) = test_session_with_peer().await;
        let repo = test_repo(&operator, &profile).await;
        let branch = repo.branch("main").open().perform(&operator).await?;

        let descriptor: ConceptDescriptor = serde_json::from_value(serde_json::json!({
            "with": {
                "title": { "the": "xyz.tonk.note/title", "as": "text:", "description": "A title" },
                "tags": { "the": "xyz.tonk.note/tags", "as": "text:", "pick": "all:" }
            }
        }))?;
        let earlier_concept = concept_identity_v0(&descriptor);
        let mut transaction = branch.transaction();
        for (field, slot) in descriptor.with().iter() {
            let attribute = slot.descriptor();
            let earlier: Entity = attribute_uri_v0(attribute).parse()?;
            let cardinality = if field == "tags" { "many" } else { "one" };
            for (the, is) in [
                (
                    "db.attribute/id",
                    Value::String(attribute.the().to_string()),
                ),
                ("db.attribute/type", Value::String("Text".into())),
                (
                    "db.attribute/cardinality",
                    Value::String(cardinality.into()),
                ),
                (
                    "db.meta/description",
                    Value::String(attribute.description().into()),
                ),
            ] {
                transaction = transaction.assert(raw(the, &earlier, is, Pick::Last)?);
            }
            transaction = transaction.assert(raw(
                &format!("db.concept.with/{field}"),
                &earlier_concept,
                Value::Entity(earlier),
                Pick::All,
            )?);
        }
        transaction = transaction
            .assert(raw(
                "db.meta/concept",
                &earlier_concept,
                Value::Entity("db:concept".parse()?),
                Pick::Last,
            )?)
            .assert(raw(
                "xyz.tonk.view/ui",
                &earlier_concept,
                Value::String("<p>{title}</p>".into()),
                Pick::Last,
            )?)
            .assert(Name {
                this: "id:note".parse()?,
                entity: name::Referent(earlier_concept.clone()),
            });
        transaction.commit().publish().perform(&operator).await?;

        let upgraded = upgrade_definitions(&branch, &operator)
            .await
            .map_err(anyhow::Error::msg)?;
        let now = descriptor.this();
        assert!(
            upgraded
                .moves
                .contains(&(earlier_concept.clone(), now.clone())),
            "the concept moves: {upgraded:?}"
        );
        assert_eq!(
            upgraded.moves.len(),
            3,
            "the two attributes and the concept move: {upgraded:?}"
        );

        let source = Source::from(&branch);
        let resolved = Concept::by_entity(now.clone())
            .resolve(&source, &operator)
            .await?
            .expect("the concept resolves under its current identity");
        assert_eq!(resolved.descriptor.this(), now);
        assert_eq!(
            lookup_named_entity("note", &branch, &operator).await?,
            Some(now.clone()),
            "the name follows the concept"
        );
        let views = select(
            &branch,
            &operator,
            ArtifactSelector::new().the("xyz.tonk.view/ui".parse()?),
        )
        .await
        .map_err(anyhow::Error::msg)?;
        assert_eq!(
            views.iter().map(|view| view.of.clone()).collect::<Vec<_>>(),
            vec![now.clone()],
            "the view moves with the concept"
        );
        for relation in LEGACY_ATTRIBUTE {
            assert!(
                select(
                    &branch,
                    &operator,
                    ArtifactSelector::new().the(relation.parse()?)
                )
                .await
                .map_err(anyhow::Error::msg)?
                .is_empty(),
                "no attribute is recorded with {relation} any more"
            );
        }
        assert!(
            select(
                &branch,
                &operator,
                ArtifactSelector::new().of(earlier_concept)
            )
            .await
            .map_err(anyhow::Error::msg)?
            .is_empty(),
            "nothing is left on the earlier concept"
        );

        let again = upgrade_definitions(&branch, &operator)
            .await
            .map_err(anyhow::Error::msg)?;
        assert!(again.is_empty(), "a second run finds nothing: {again:?}");
        Ok(())
    }
}
