//! Keys what a space wrote itself by the identities its library now carries.
//!
//! dialog names an attribute by a hash of its relation, type and pick, and
//! an unnamed concept by a hash of its attributes. The release before picks
//! hashed less, so every attribute's identity changed, and every
//! unnamed concept's with it. An upgrade reverts what the last install wrote
//! and installs the library again, which moves the library's own definitions
//! to their new identities. What the space wrote itself stays as it was: a
//! route whose concept is a library concept, a name pointing at one, a
//! template's concept whose field reads a library attribute, a view the space
//! wrote for a library concept. Each of those then names an entity nothing
//! defines, and the page that reads it reports the model missing.
//!
//! [`moves`] pairs each identity the installed library had under the earlier
//! release with the one it has now, and [`rekey`] lists the writes that make
//! the space's facts follow.

use std::collections::{BTreeMap, HashMap, HashSet};

use dialog_artifacts::{Artifact, ArtifactSelector, Entity, Pick, Relation, Value};
use dialog_query::migration::{attribute_uri_v0, concept_identity_v0};
use dialog_query::{AttributeDescriptor, ConceptDescriptor, ConceptFieldDescriptor};
use futures_util::StreamExt as _;

use super::super::claim::RawClaim;
use crate::worker::TonkState;

/// What a write of a changed identity changes.
pub(super) struct Rekey {
    /// Each identity the library had and the one it has now.
    pub(super) moves: Vec<(Entity, Entity)>,
    /// The space's claims naming an earlier identity, to retract.
    pub(super) retract: Vec<RawClaim>,
    /// The same claims naming the current identity, to assert.
    pub(super) assert: Vec<RawClaim>,
}

/// Each attribute and unnamed concept `entities` holds on `batch`, paired
/// with the identity the release before picks gave it, where the
/// two differ. See [`moves_in`].
pub(super) async fn moves(
    tonk: &TonkState,
    batch: &dialog_repository::TransactionBatch,
    entities: &HashSet<Entity>,
) -> Result<Vec<(Entity, Entity)>, String> {
    let mut facts: Facts = HashMap::new();
    for entity in entities {
        let claims = select(tonk, batch, ArtifactSelector::new().of(entity.clone())).await?;
        facts.insert(
            entity.clone(),
            claims.into_iter().map(|c| (c.the, c.is)).collect(),
        );
    }
    // A concept may read an attribute another library declares (a
    // component's concept, core's attribute): its definition is on the
    // branch, not in this install.
    let referenced: HashSet<Entity> = facts
        .values()
        .flatten()
        .filter(|(the, _)| the.to_string().starts_with("db.concept.with/"))
        .filter_map(|(_, is)| match is {
            Value::Entity(attribute) => Some(attribute.clone()),
            _ => None,
        })
        .filter(|attribute| !facts.contains_key(attribute))
        .collect();
    for attribute in referenced {
        let claims = select(tonk, batch, ArtifactSelector::new().of(attribute.clone())).await?;
        facts.insert(
            attribute,
            claims.into_iter().map(|c| (c.the, c.is)).collect(),
        );
    }
    Ok(moves_in(&facts))
}

/// Facts about entities, each as its `(attribute, value)` pairs.
pub(super) type Facts = HashMap<Entity, Vec<(Relation, Value)>>;

/// Each attribute and unnamed concept `facts` define, paired with the
/// identity the release before picks gave it, where the two
/// differ: `(identity then, identity now)`.
///
/// An attribute's earlier identity hashed its relation, cardinality and type,
/// which its `db.attribute/*` facts spell: the cardinality is its pick's
/// arity. A concept's hashed its fields'
/// attributes and whether each is optional, which its `db.concept.with/*` and
/// `db.concept.optional/*` facts spell. A concept named by an entity of its
/// own (`tonk:workspace/shell`) kept that entity and has no move.
pub(super) fn moves_in(facts: &Facts) -> Vec<(Entity, Entity)> {
    let mut attributes: HashMap<&Entity, AttributeDescriptor> = HashMap::new();
    let mut moves = Vec::new();
    for (entity, claims) in facts {
        if let Some(descriptor) = attribute_descriptor(claims) {
            push_move(&mut moves, attribute_uri_v0(&descriptor), entity);
            attributes.insert(entity, descriptor);
        }
    }
    for (entity, claims) in facts {
        if !entity.as_str().starts_with("concept:") {
            continue;
        }
        let mut fields: BTreeMap<String, &Entity> = BTreeMap::new();
        let mut optional: HashSet<String> = HashSet::new();
        for (the, is) in claims {
            let the = the.to_string();
            if let Some(field) = the.strip_prefix("db.concept.with/")
                && let Value::Entity(attribute) = is
            {
                fields.insert(field.to_owned(), attribute);
            } else if let Some(field) = the.strip_prefix("db.concept.optional/")
                && *is == Value::Boolean(true)
            {
                optional.insert(field.to_owned());
            }
        }
        if fields.is_empty() {
            continue;
        }
        let descriptors: Option<Vec<(String, ConceptFieldDescriptor)>> = fields
            .iter()
            .map(|(field, attribute)| {
                let descriptor = attributes.get(attribute)?.clone();
                Some((
                    field.clone(),
                    if optional.contains(field) {
                        ConceptFieldDescriptor::optional(descriptor)
                    } else {
                        ConceptFieldDescriptor::required(descriptor)
                    },
                ))
            })
            .collect();
        let Some(Ok(descriptor)) = descriptors.map(ConceptDescriptor::try_from) else {
            continue;
        };
        push_move(
            &mut moves,
            concept_identity_v0(&descriptor).to_string(),
            entity,
        );
    }
    moves.sort();
    moves.dedup();
    moves
}

/// The writes that make every claim on `batch` naming an earlier identity
/// in `moves` name the current one instead.
///
/// A claim whose value is an earlier identity is retracted and asserted with
/// the current one. A claim about an earlier identity moves to the current
/// one, apart from the definition the install has just written there: what
/// is left on the earlier entity after the uninstall is what the space wrote
/// itself (a view, a description it gave a library attribute), which was the
/// newest word on that entity, so it succeeds what the library says there
/// now. The definition's own facts (`db.attribute/*`, `db.concept.*`, the
/// concept marker) are the install's, and stay with it.
pub(super) async fn rekey(
    tonk: &TonkState,
    batch: &dialog_repository::TransactionBatch,
    moves: Vec<(Entity, Entity)>,
) -> Result<Rekey, String> {
    let current: HashMap<Entity, Entity> = moves.iter().cloned().collect();
    let follow = |value: &Value| match value {
        Value::Entity(entity) => current
            .get(entity)
            .map(|now| Value::Entity(now.clone()))
            .unwrap_or_else(|| value.clone()),
        other => other.clone(),
    };
    let mut retract = Vec::new();
    let mut assert = Vec::new();
    for (earlier, now) in &moves {
        let naming = select(
            tonk,
            batch,
            ArtifactSelector::new().is(Value::Entity(earlier.clone())),
        )
        .await?;
        for claim in naming {
            if current.contains_key(&claim.of) {
                // Moved with the entity it is about, below.
                continue;
            }
            retract.push(raw(&claim.the, &claim.of, &claim.is, Pick::All));
            assert.push(raw(&claim.the, &claim.of, &follow(&claim.is), Pick::All));
        }

        let about = select(tonk, batch, ArtifactSelector::new().of(earlier.clone())).await?;
        let mut by_relation: BTreeMap<String, Vec<Artifact>> = BTreeMap::new();
        for claim in about {
            if is_definition(&claim.the) {
                continue;
            }
            by_relation
                .entry(claim.the.to_string())
                .or_default()
                .push(claim);
        }
        for claims in by_relation.into_values() {
            let policy = if claims.len() == 1 {
                Pick::Last
            } else {
                Pick::All
            };
            for claim in claims {
                retract.push(raw(&claim.the, &claim.of, &claim.is, Pick::All));
                assert.push(raw(&claim.the, now, &follow(&claim.is), policy.clone()));
            }
        }
    }
    Ok(Rekey {
        moves,
        retract,
        assert,
    })
}

/// Whether a fact about an attribute or concept entity is part of its
/// definition, which the install writes.
fn is_definition(the: &Relation) -> bool {
    let the = the.to_string();
    the.starts_with("db.attribute/")
        || the.starts_with("db.concept.")
        || the == "db.meta/concept"
        || the == "dialog.concept/transient"
}

/// The descriptor an attribute's `db.attribute/*` facts spell, enough to
/// compute its earlier identity: relation, type and pick, whose arity is
/// the cardinality the earlier identity hashed.
fn attribute_descriptor(claims: &[(Relation, Value)]) -> Option<AttributeDescriptor> {
    let fact = |name: &str| {
        claims
            .iter()
            .find_map(|(the, is)| (the.as_str() == name).then_some(is))
    };
    let Some(Value::String(id)) = fact("db.attribute/id") else {
        return None;
    };
    let mut shape = serde_json::Map::new();
    shape.insert("the".to_owned(), serde_json::Value::String(id.clone()));
    if let Some(Value::Entity(kind)) = fact("db.attribute/as") {
        shape.insert("as".to_owned(), serde_json::Value::String(kind.to_string()));
    }
    if let Some(Value::String(pick)) = fact("db.attribute/pick") {
        shape.insert("pick".to_owned(), serde_json::Value::String(pick.clone()));
    }
    serde_json::from_value(serde_json::Value::Object(shape)).ok()
}

fn push_move(moves: &mut Vec<(Entity, Entity)>, earlier: String, now: &Entity) {
    if let Ok(earlier) = earlier.parse::<Entity>()
        && &earlier != now
    {
        moves.push((earlier, now.clone()));
    }
}

fn raw(the: &Relation, of: &Entity, is: &Value, policy: Pick) -> RawClaim {
    RawClaim {
        the: the.clone(),
        of: of.clone(),
        is: is.clone(),
        policy,
    }
}

/// Every claim `selector` matches on the staged chain `batch`.
async fn select(
    tonk: &TonkState,
    batch: &dialog_repository::TransactionBatch,
    selector: ArtifactSelector<dialog_artifacts::selector::Constrained>,
) -> Result<Vec<Artifact>, String> {
    let stream = batch
        .claims()
        .select(selector)
        .perform(&tonk.operator)
        .await
        .map_err(|error| error.to_string())?;
    futures_util::pin_mut!(stream);
    let mut claims = Vec::new();
    while let Some(claim) = stream.next().await {
        let claim = claim.map_err(|error| error.to_string())?;
        claims.push(claim.to_owned().map_err(|error| error.to_string())?);
    }
    Ok(claims)
}
