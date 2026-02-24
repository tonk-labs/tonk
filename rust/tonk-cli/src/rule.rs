//! Rule management: define, list, show, and delete deductive rules.
//!
//! A rule derives entities of a conclusion concept from patterns across
//! existing facts. Rules are stored as JSON blobs in dialog-db and compiled
//! into `DeductiveRule`s at query time.

use crate::schema::*;
use anyhow::{Context, Result};
use dialog_artifacts::{Artifact, ArtifactStore, ArtifactStoreMut, Instruction};
use dialog_query::claim::Attribute;
use dialog_query::{DeductiveRule, Entity, Premise, Term, Value};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::Read;
use std::str::FromStr;

// ---------------------------------------------------------------------------
// Rule definition JSON types
// ---------------------------------------------------------------------------

/// The complete rule definition, stored as JSON in `rule/definition`.
///
/// Use [`RuleDefinition::from_json`] to parse and structurally validate
/// a definition from user-supplied JSON. Direct deserialization via `serde`
/// is still available for loading from storage (already validated at write time).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleDefinition {
    /// The concept being derived and its variable bindings.
    pub conclusion: RuleConclusion,

    /// Positive premises — fact patterns that must hold.
    pub when: Vec<RulePremise>,

    /// Negative premises — fact patterns that must NOT hold.
    #[serde(default)]
    pub unless: Vec<RulePremise>,
}

impl RuleDefinition {
    /// Parse and structurally validate a rule definition from JSON.
    ///
    /// This performs structural checks that are independent of any
    /// particular concept schema:
    /// - The JSON must be a valid `RuleDefinition`
    /// - There must be at least one positive premise in `when`
    /// - The conclusion concept name must not be empty
    pub fn from_json(json: &str) -> Result<Self> {
        let def: Self =
            serde_json::from_str(json).context("Failed to parse rule definition JSON")?;

        if def.when.is_empty() {
            anyhow::bail!(
                "Rule must have at least one positive premise in 'when'. \
                 Rules with no positive premises cannot derive any entities."
            );
        }

        if def.conclusion.concept.is_empty() {
            anyhow::bail!("Rule conclusion must specify a non-empty 'concept' name.");
        }

        if !def.conclusion.bindings.contains_key("this") {
            anyhow::bail!(
                "Rule conclusion bindings must include a 'this' key that specifies \
                 which entity variable provides the derived entity's identity. \
                 For example: \"bindings\": {{ \"this\": \"?entity\", ... }}"
            );
        }

        Ok(def)
    }
}

/// The conclusion of a rule: which concept is derived, and how its
/// attributes map to variables.
///
/// The `bindings` map must include a `"this"` key that designates which
/// entity variable provides the identity of each derived instance. All
/// other keys map concept attribute short names to premise variables.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleConclusion {
    /// Name of the conclusion concept (e.g. "AllergyConflict").
    pub concept: String,

    /// Maps concept attribute short names (and `"this"`) to variables.
    ///
    /// The `"this"` key is required and designates which entity variable
    /// in the premises provides the *identity* of each derived instance.
    /// e.g. `{"this": "?allergy", "attendee": "?person", "recipe": "?recipe"}`
    pub bindings: HashMap<String, String>,
}

/// A single fact-level premise: an EAV pattern with variables.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RulePremise {
    /// Fully qualified attribute name (always a constant).
    pub the: String,

    /// Entity pattern: `"?variable"`, `"_"` (wildcard), or a constant.
    pub of: String,

    /// Value pattern: `"?variable"`, `"_"` (wildcard), or a constant.
    pub is: String,
}

// ---------------------------------------------------------------------------
// Premise term parsing
// ---------------------------------------------------------------------------

/// A parsed term from a rule premise string.
///
/// Provides type-safe representation of the three possible term forms,
/// replacing ad-hoc `strip_prefix('?')` / `== "_"` checks throughout
/// the codebase.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PremiseTerm {
    /// A variable binding: `"?name"` → `Variable("name")`.
    Variable(String),
    /// A wildcard: `"_"` — matches anything, no binding.
    Wildcard,
    /// A constant literal: any other string.
    Constant(String),
}

impl PremiseTerm {
    /// Parse a raw term string from a rule definition.
    pub fn parse(s: &str) -> Self {
        if let Some(var_name) = s.strip_prefix('?') {
            Self::Variable(var_name.to_string())
        } else if s == "_" {
            Self::Wildcard
        } else {
            Self::Constant(s.to_string())
        }
    }

    /// If this is a variable, return its name (without the `?` prefix).
    pub fn variable_name(&self) -> Option<&str> {
        match self {
            Self::Variable(name) => Some(name),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Rule compilation
// ---------------------------------------------------------------------------

/// Convert a `PremiseTerm` into a `Term<Value>`.
fn term_to_value(pt: &PremiseTerm) -> Term<Value> {
    match pt {
        PremiseTerm::Variable(name) => Term::var(name.as_str()),
        PremiseTerm::Wildcard => Term::blank(),
        PremiseTerm::Constant(s) => Term::Constant(parse_value(s)),
    }
}

/// Convert a `PremiseTerm` into a `Term<Entity>`.
fn term_to_entity(pt: &PremiseTerm) -> Result<Term<dialog_query::Entity>> {
    match pt {
        PremiseTerm::Variable(name) => Ok(Term::var(name.as_str())),
        PremiseTerm::Wildcard => Ok(Term::blank()),
        PremiseTerm::Constant(s) => {
            let entity = dialog_query::Entity::from_str(s)
                .context(format!("Invalid entity in premise 'of' field: {}", s))?;
            Ok(Term::Constant(entity))
        }
    }
}

/// Parse a term string from a rule definition into a `Term<Value>`.
///
/// - `"?name"` → `Term::var("name")`
/// - `"_"` → `Term::blank()`
/// - anything else → `Term::Constant(parse_value(s))`
fn parse_term_value(s: &str) -> Term<Value> {
    term_to_value(&PremiseTerm::parse(s))
}

/// Parse a term string into a `Term<Entity>`.
///
/// Delegates to [`PremiseTerm::parse`] then [`term_to_entity`].
fn parse_term_entity(s: &str) -> Result<Term<dialog_query::Entity>> {
    term_to_entity(&PremiseTerm::parse(s))
}

/// Collect all variable names used in the positive (`when`) premises (`of` and `is` fields).
///
/// Variables used only in `unless` premises are unsafe in Datalog (they have no
/// grounding) so this intentionally excludes them. Use [`collect_all_premise_vars`]
/// when you need variables from both `when` and `unless` (e.g. for collision detection).
fn collect_when_vars(definition: &RuleDefinition) -> std::collections::HashSet<String> {
    let mut vars = std::collections::HashSet::new();
    for p in &definition.when {
        if let Some(name) = PremiseTerm::parse(&p.of).variable_name() {
            vars.insert(name.to_string());
        }
        if let Some(name) = PremiseTerm::parse(&p.is).variable_name() {
            vars.insert(name.to_string());
        }
    }
    vars
}

/// Collect all variable names used in all premises (both `when` and `unless`).
///
/// Used for rename-collision detection where we need the full variable universe.
fn collect_all_premise_vars(definition: &RuleDefinition) -> std::collections::HashSet<String> {
    let mut vars = std::collections::HashSet::new();
    for p in definition.when.iter().chain(definition.unless.iter()) {
        if let Some(name) = PremiseTerm::parse(&p.of).variable_name() {
            vars.insert(name.to_string());
        }
        if let Some(name) = PremiseTerm::parse(&p.is).variable_name() {
            vars.insert(name.to_string());
        }
    }
    vars
}

/// Build a variable renaming map from the conclusion bindings.
///
/// The conclusion bindings map concept attribute short names to user
/// variables: `{"attendee": "?person"}` means the premise variable
/// `?person` should be renamed to `?attendee` (the concept operand name).
///
/// The required `"this"` key in bindings designates which entity variable
/// becomes `"this"` — the implicit entity operand every Concept has.
///
/// If renaming would collide with existing premise variables that are
/// NOT themselves being renamed, those existing variables are also
/// given new names (suffixed with `_0`, `_1`, etc.) to avoid clashes.
///
/// Returns a map from user variable name (without `?`) to new variable name.
fn build_rename_map(
    definition: &RuleDefinition,
    namespace: &str,
) -> Result<HashMap<String, String>> {
    let all_vars = collect_all_premise_vars(definition);
    let mut rename: HashMap<String, String> = HashMap::new();

    // Extract and validate `this` from bindings first
    let this_str = definition.conclusion.bindings.get("this").context(
        "Rule conclusion bindings must include a 'this' key that specifies \
             which entity variable provides the derived entity's identity.",
    )?;

    let this_var = match PremiseTerm::parse(this_str) {
        PremiseTerm::Variable(name) => name,
        _ => anyhow::bail!(
            "Conclusion binding 'this' must be a variable (starting with '?'), got: {}",
            this_str
        ),
    };

    rename.insert(this_var.clone(), "this".to_string());

    // Map each remaining conclusion binding: user variable → attribute short name
    let mut binding_vars: std::collections::HashSet<String> = std::collections::HashSet::new();
    binding_vars.insert(this_var);

    for (attr_short, var_str) in &definition.conclusion.bindings {
        if attr_short == "this" {
            continue; // Already handled above
        }
        if let PremiseTerm::Variable(var_name) = PremiseTerm::parse(var_str) {
            // Validate the attribute is in the concept
            let _qualified = qualify_attribute(namespace, attr_short)?;

            // A variable can't map to two different operand names
            if binding_vars.contains(&var_name) {
                anyhow::bail!(
                    "Variable '?{}' is used in multiple conclusion bindings. \
                     Each variable can only be bound to one operand name. \
                     Use separate variables for 'this' and attribute bindings.",
                    var_name
                );
            }

            rename.insert(var_name.clone(), attr_short.clone());
            binding_vars.insert(var_name);
        }
        // Constants in bindings are allowed — they don't need renaming
    }

    // Detect collisions: a rename target name might already be used by
    // a different variable in the premises that is NOT being renamed.
    // e.g. bindings map ?recipe_name → "recipe", but ?recipe is already
    // used as an entity variable. We need to rename ?recipe to avoid clash.
    let rename_targets: std::collections::HashSet<String> = rename.values().cloned().collect();
    let renamed_sources: std::collections::HashSet<String> = rename.keys().cloned().collect();

    let mut collision_renames: Vec<(String, String)> = Vec::new();
    let mut generated_names: std::collections::HashSet<String> = std::collections::HashSet::new();
    for target_name in &rename_targets {
        // Is there a variable with the same name as a rename target,
        // that is NOT itself being renamed?
        if all_vars.contains(target_name) && !renamed_sources.contains(target_name) {
            // This variable would collide. Generate a safe name.
            let mut suffix = 0;
            loop {
                let candidate = format!("{}_{}", target_name, suffix);
                if !all_vars.contains(&candidate)
                    && !rename_targets.contains(&candidate)
                    && !generated_names.contains(&candidate)
                {
                    collision_renames.push((target_name.clone(), candidate.clone()));
                    generated_names.insert(candidate);
                    break;
                }
                suffix += 1;
            }
        }
    }

    for (old_name, new_name) in collision_renames {
        rename.insert(old_name, new_name);
    }

    Ok(rename)
}

/// Apply the variable renaming map to a term string.
///
/// If the string is `"?varname"` and `varname` is in the rename map,
/// returns the renamed version `"?newname"`. Otherwise returns as-is.
fn rename_var(s: &str, rename: &HashMap<String, String>) -> String {
    match PremiseTerm::parse(s) {
        PremiseTerm::Variable(ref var_name) => {
            if let Some(new_name) = rename.get(var_name.as_str()) {
                format!("?{}", new_name)
            } else {
                s.to_string()
            }
        }
        _ => s.to_string(),
    }
}

/// Compile a `RuleDefinition` into a `DeductiveRule` using the given
/// concept attributes for the conclusion.
///
/// The `cardinalities` map provides per-attribute cardinality from stored
/// metadata. It is propagated to `build_dynamic_concept()` so that the
/// query planner assigns correct costs for `Cardinality::Many` attributes.
///
/// The `namespace` is the attribute namespace for the conclusion concept
/// (used to qualify short attribute names in conclusion bindings).
pub fn compile_rule(
    definition: &RuleDefinition,
    _concept_name: &ConceptName,
    concept_attrs: &[String],
    cardinalities: &std::collections::HashMap<String, dialog_query::Cardinality>,
    namespace: &str,
) -> Result<DeductiveRule> {
    // 1. Build dynamic Concept from conclusion concept's attributes
    let concept = build_dynamic_concept(concept_attrs, cardinalities)?;

    // 2. Build variable renaming map from conclusion bindings
    let rename = build_rename_map(definition, namespace)?;

    // 3. Build positive premises (with renamed variables)
    let mut premises: Vec<Premise> = Vec::new();

    for p in &definition.when {
        let renamed_of = rename_var(&p.of, &rename);
        let renamed_is = rename_var(&p.is, &rename);

        let of_term = parse_term_entity(&renamed_of)?;
        let is_term = parse_term_value(&renamed_is);

        let fact_app = dialog_query::predicate::Fact::select()
            .the(p.the.as_str())
            .of(of_term)
            .is(is_term)
            .compile()
            .context(format!(
                "Failed to compile premise: the={}, of={}, is={}",
                p.the, p.of, p.is
            ))?;
        premises.push(fact_app.into());
    }

    // 4. Build negative premises (unless) with renamed variables.
    //
    // Unlike positive premises where each EAV triple is independent,
    // negative premises must be grouped by entity (`of` value) and
    // negated as a conjunction. This is because the YAML `unless` section
    // uses concept-level references where multiple attributes of the same
    // entity form a single condition: "there must NOT exist an entity with
    // ALL these attributes simultaneously."
    //
    // We achieve this by building a dynamic Concept per entity group and
    // negating its application, which internally joins the attribute facts.
    {
        // Group unless premises by their (renamed) entity value
        let mut groups: Vec<(String, Vec<&RulePremise>)> = Vec::new();
        for p in &definition.unless {
            let renamed_of = rename_var(&p.of, &rename);
            if let Some(group) = groups.iter_mut().find(|(of, _)| of == &renamed_of) {
                group.1.push(p);
            } else {
                groups.push((renamed_of, vec![p]));
            }
        }

        for (renamed_of, group) in &groups {
            // Build a dynamic Concept from the attributes in this group.
            // Each attribute `the` field is "namespace/name" (e.g., "allergy/person").
            // We split on "/" to get the namespace and short name, then use the
            // short name as the operand key.
            let attr_schemas: Vec<(&str, dialog_query::AttributeSchema<Value>)> = group
                .iter()
                .map(|p| {
                    let (ns, name) = p.the.split_once('/').unwrap_or(("", &p.the));
                    let short = leak_str(name);
                    let schema = dialog_query::AttributeSchema::<Value>::new(
                        leak_str(ns),
                        short,
                        leak_str(""),
                        dialog_query::Type::String,
                    );
                    (short, schema)
                })
                .collect();

            let neg_concept = dialog_query::predicate::Concept::new(attr_schemas.into());

            // Build parameters: "this" for entity, each attribute gets its renamed value.
            // Parameter keys use the short name (e.g., "person", "substance").
            let mut neg_params = dialog_query::Parameters::new();

            let of_term: Term<Value> = match PremiseTerm::parse(renamed_of) {
                PremiseTerm::Variable(name) => Term::var(leak_str(&name)),
                PremiseTerm::Wildcard => Term::blank(),
                PremiseTerm::Constant(s) => Term::Constant(parse_value(&s)),
            };
            neg_params.insert("this".to_string(), of_term);

            for p in group {
                let (_ns, name) = p.the.split_once('/').unwrap_or(("", &p.the));
                let renamed_is = rename_var(&p.is, &rename);
                let is_term = parse_term_value(&renamed_is);
                neg_params.insert(name.to_string(), is_term);
            }

            let neg_app = neg_concept.apply(neg_params).map_err(|e| {
                anyhow::anyhow!(
                    "Failed to apply negated concept for unless group (of={}): {}",
                    renamed_of,
                    e
                )
            })?;

            premises.push(neg_app.not());
        }
    }

    // 5. Compile the DeductiveRule
    let rule = DeductiveRule::new(concept, premises).map_err(|e| {
        anyhow::anyhow!(
            "Failed to compile rule for concept '{}': {}",
            definition.conclusion.concept,
            e
        )
    })?;

    Ok(rule)
}

/// Validate a rule definition against the known concept schemas.
///
/// Checks:
/// - Conclusion binding keys are valid attributes of the conclusion concept
/// - All conclusion binding variables appear in at least one premise
pub fn validate_definition(
    definition: &RuleDefinition,
    conclusion_attrs: &[String],
    conclusion_name: &ConceptName,
    namespace: &str,
) -> Result<()> {
    // Check that all binding keys (except "this") are valid concept attributes.
    // A binding key like "comment" matches either the concept-derived path
    // (e.g. "annotatedlink/comment") or any attribute whose short name
    // matches (e.g. "carry.links/comment" where the part after "/" is "comment").
    for short_name in definition.conclusion.bindings.keys() {
        if short_name == "this" {
            continue; // "this" designates entity identity, not a concept attribute
        }
        let concept_qualified = qualify_attribute(namespace, short_name)?;
        let matches = conclusion_attrs.iter().any(|a| {
            *a == concept_qualified || a.rsplit_once('/').is_some_and(|(_, n)| n == short_name)
        });
        if !matches {
            anyhow::bail!(
                "Conclusion binding '{}' is not an attribute of concept '{}'. \
                 Known attributes: {}",
                short_name,
                conclusion_name,
                conclusion_attrs
                    .iter()
                    .map(|a| short_attribute(namespace, a))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
    }

    // Collect variables from positive (`when`) premises only.
    // Variables must be grounded in at least one positive premise — appearing
    // only in `unless` is unsafe (Datalog safety requirement).
    let when_vars = collect_when_vars(definition);

    // Check that all conclusion binding variables (including "this") appear
    // in at least one `when` premise
    for (attr, var_str) in &definition.conclusion.bindings {
        if let PremiseTerm::Variable(var_name) = PremiseTerm::parse(var_str)
            && !when_vars.contains(&var_name)
        {
            anyhow::bail!(
                "Variable '?{}' in conclusion binding '{}' does not appear in any positive \
                 ('when') premise. All conclusion variables must be grounded in 'when' premises \
                 (appearing only in 'unless' is not sufficient).",
                var_name,
                attr,
            );
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// List all rules
// ---------------------------------------------------------------------------

/// List all rules in the active space.
pub async fn list(ctx: &SpaceContext, json: bool) -> Result<()> {
    let session = open_session(ctx).await?;

    let rule_entries = find_all_rules(&session).await?;

    if rule_entries.is_empty() {
        if json {
            println!("[]");
        } else {
            println!("No rules defined. Use 'tonk rule define' to create one.");
        }
        return Ok(());
    }

    // (entity, name_or_none, description, conclusion)
    let mut rules: Vec<(String, Option<String>, Option<String>, String)> = Vec::new();
    for (entity, name) in &rule_entries {
        let description = fetch_string(
            &session,
            entity,
            parse_claim_attribute(ATTR_RULE_DESCRIPTION)?,
        )
        .await?;
        let conclusion = match fetch_string(
            &session,
            entity,
            parse_claim_attribute(ATTR_RULE_CONCLUSION)?,
        )
        .await?
        {
            Some(c) => c,
            None => {
                let entity_str = entity.to_string();
                let label = name.as_deref().unwrap_or(&entity_str);
                eprintln!(
                    "Warning: rule '{}' is missing its 'rule/conclusion' attribute — possible data corruption",
                    label
                );
                "???".to_string()
            }
        };
        rules.push((entity.to_string(), name.clone(), description, conclusion));
    }

    if json {
        let items: Vec<serde_json::Value> = rules
            .iter()
            .map(|(entity, name, desc, conclusion)| {
                let mut obj = serde_json::json!({
                    "entity": entity,
                    "conclusion": conclusion,
                });
                if let Some(n) = name {
                    obj.as_object_mut()
                        .unwrap()
                        .insert("name".to_string(), serde_json::json!(n));
                }
                if let Some(d) = desc {
                    obj.as_object_mut()
                        .unwrap()
                        .insert("description".to_string(), serde_json::json!(d));
                }
                obj
            })
            .collect();
        println!("{}", serde_json::to_string(&items)?);
    } else {
        println!("Rules:\n");
        for (entity, name, desc, conclusion) in &rules {
            let label = name.as_deref().unwrap_or(entity.as_str());
            let desc_str = desc
                .as_ref()
                .map(|d| format!(" - {}", d))
                .unwrap_or_default();
            println!("  {} -> {}{}", label, conclusion, desc_str);
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Define a new rule
// ---------------------------------------------------------------------------

/// Define a new rule from a JSON definition.
///
/// If `name` is `Some`, the rule is stored with that name and can be looked
/// up by name. If `None`, a deterministic entity ID is derived from the
/// definition JSON hash, making the define idempotent for the same definition.
///
/// Uses raw Branch + Instruction (not Session/Transaction) because
/// Transaction deduplicates by `(entity, attribute)` — only one value
/// survives per pair.
pub async fn define(
    ctx: &SpaceContext,
    name: Option<String>,
    file: Option<String>,
    stdin: bool,
    description: String,
    json: bool,
) -> Result<()> {
    if let Some(ref n) = name {
        validate_safe_name(n, "Rule")?;
    }

    if file.is_some() && stdin {
        anyhow::bail!(
            "Cannot use both --file and --stdin. Provide the rule definition via one source."
        );
    }

    // Read JSON definition from file or stdin
    let definition_json = if let Some(path) = &file {
        std::fs::read_to_string(path).context(format!("Failed to read file: {}", path))?
    } else if stdin {
        let mut buf = String::new();
        std::io::stdin()
            .read_to_string(&mut buf)
            .context("Failed to read from stdin")?;
        buf
    } else {
        anyhow::bail!("Provide a rule definition via --file <path> or --stdin");
    };

    let definition = RuleDefinition::from_json(definition_json.trim())?;

    let mut branch = open_branch(ctx).await?;

    // Serialize definition to canonical JSON for storage (and for hashing if unnamed)
    let definition_str = serde_json::to_string(&definition)?;

    // Derive entity ID: from name if provided, from definition hash otherwise
    let rule = if let Some(ref n) = name {
        rule_entity(&ctx.space_did, n)?
    } else {
        rule_entity_from_definition(&ctx.space_did, &definition_str)?
    };

    // Check for duplicates
    if let Some(ref n) = name {
        if lookup_rule_by_name(&branch, n).await?.is_some() {
            anyhow::bail!(
                "Rule '{}' already exists. Delete it first with 'tonk rule delete {}'.",
                n,
                n
            );
        }
    } else {
        // For unnamed rules, same definition = same entity = idempotent.
        // Check if this exact entity already has a conclusion attribute.
        if fetch_string(&branch, &rule, parse_claim_attribute(ATTR_RULE_CONCLUSION)?)
            .await?
            .is_some()
        {
            if json {
                let output = serde_json::json!({
                    "ok": true,
                    "entity": rule.to_string(),
                    "conclusion": definition.conclusion.concept,
                    "already_exists": true,
                });
                println!("{}", serde_json::to_string(&output)?);
            } else {
                println!("Rule already exists (entity: {})", rule);
            }
            return Ok(());
        }
    }

    // Verify the conclusion concept exists and get its attributes
    let conclusion_concept = ConceptName::new(&definition.conclusion.concept)?;
    let concept = lookup_concept_by_name(&branch, &conclusion_concept)
        .await?
        .context(format!(
            "Conclusion concept '{}' not found. Define it first.",
            definition.conclusion.concept
        ))?;
    let concept_name = ConceptName::from_stored(
        fetch_string(&branch, &concept, concept_name_selector())
            .await?
            .unwrap_or_else(|| conclusion_concept.to_string()),
    );

    let concept_attrs =
        fetch_string_values(&branch, &concept, concept_attribute_selector()).await?;
    let concept_ns = fetch_string(&branch, &concept, concept_namespace_selector())
        .await?
        .unwrap_or_else(|| ctx.space_name.clone());
    let cardinalities = fetch_attribute_cardinalities(&branch, &concept_attrs).await?;

    // Validate the definition
    validate_definition(&definition, &concept_attrs, &concept_name, &concept_ns)?;

    // Try to compile the rule to catch errors early
    compile_rule(
        &definition,
        &concept_name,
        &concept_attrs,
        &cardinalities,
        &concept_ns,
    )
    .context(
        "Rule definition is invalid. Check that variable names match between \
         conclusion bindings and premises.",
    )?;

    // Build instructions
    let mut instructions = Vec::new();

    // Rule name (only if provided)
    if let Some(ref n) = name {
        instructions.push(Instruction::Assert(Artifact {
            the: Attribute::from_str(ATTR_RULE_NAME)?,
            of: rule.clone(),
            is: Value::String(n.clone()),
            cause: None,
        }));
    }

    // Rule conclusion concept name
    instructions.push(Instruction::Assert(Artifact {
        the: Attribute::from_str(ATTR_RULE_CONCLUSION)?,
        of: rule.clone(),
        is: Value::String(concept_name.to_string()),
        cause: None,
    }));

    // Rule definition JSON
    instructions.push(Instruction::Assert(Artifact {
        the: Attribute::from_str(ATTR_RULE_DEFINITION)?,
        of: rule.clone(),
        is: Value::String(definition_str.clone()),
        cause: None,
    }));

    // Description
    instructions.push(Instruction::Assert(Artifact {
        the: Attribute::from_str(ATTR_RULE_DESCRIPTION)?,
        of: rule.clone(),
        is: Value::String(description.clone()),
        cause: None,
    }));

    branch
        .commit(futures_util::stream::iter(instructions))
        .await?;

    if json {
        let mut output = serde_json::json!({
            "ok": true,
            "entity": rule.to_string(),
            "conclusion": concept_name.as_str(),
            "when_count": definition.when.len(),
            "unless_count": definition.unless.len(),
        });
        if let Some(ref n) = name {
            output
                .as_object_mut()
                .unwrap()
                .insert("name".to_string(), serde_json::json!(n));
        }
        println!("{}", serde_json::to_string(&output)?);
    } else {
        if let Some(ref n) = name {
            println!("Defined rule '{}'", n);
        } else {
            println!("Defined rule (entity: {})", rule);
        }
        println!("  Conclusion: {}", concept_name);
        println!(
            "  Premises: {} when, {} unless",
            definition.when.len(),
            definition.unless.len()
        );
        println!("  Description: {}", description);
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Show rule details
// ---------------------------------------------------------------------------

/// Show the full definition of a rule (by name or entity ID).
pub async fn show(ctx: &SpaceContext, name_or_id: String, json: bool) -> Result<()> {
    let session = open_session(ctx).await?;

    // Try lookup by name first, then by entity ID
    let rule = if let Some(entity) = lookup_rule_by_name(&session, &name_or_id).await? {
        entity
    } else if let Ok(entity) = Entity::from_str(&name_or_id) {
        // Verify this entity is actually a rule (has a conclusion attribute)
        if fetch_string(
            &session,
            &entity,
            parse_claim_attribute(ATTR_RULE_CONCLUSION)?,
        )
        .await?
        .is_none()
        {
            anyhow::bail!("Rule '{}' not found", name_or_id);
        }
        entity
    } else {
        anyhow::bail!("Rule '{}' not found", name_or_id);
    };

    let stored_name = fetch_string(&session, &rule, parse_claim_attribute(ATTR_RULE_NAME)?).await?;

    let description = fetch_string(
        &session,
        &rule,
        parse_claim_attribute(ATTR_RULE_DESCRIPTION)?,
    )
    .await?;
    let conclusion = match fetch_string(
        &session,
        &rule,
        parse_claim_attribute(ATTR_RULE_CONCLUSION)?,
    )
    .await?
    {
        Some(c) => c,
        None => {
            let rule_str = rule.to_string();
            let label = stored_name.as_deref().unwrap_or(&rule_str);
            eprintln!(
                "Warning: rule '{}' is missing its 'rule/conclusion' attribute — possible data corruption",
                label
            );
            "???".to_string()
        }
    };
    let definition_str = fetch_string(
        &session,
        &rule,
        parse_claim_attribute(ATTR_RULE_DEFINITION)?,
    )
    .await?
    .context("Rule definition not found")?;

    let definition: RuleDefinition =
        serde_json::from_str(&definition_str).context("Failed to parse stored rule definition")?;

    if json {
        let mut output = serde_json::json!({
            "conclusion": conclusion,
            "definition": definition,
            "entity": rule.to_string(),
        });
        if let Some(ref n) = stored_name {
            output
                .as_object_mut()
                .unwrap()
                .insert("name".to_string(), serde_json::json!(n));
        }
        if let Some(desc) = &description {
            output
                .as_object_mut()
                .unwrap()
                .insert("description".to_string(), serde_json::json!(desc));
        }
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        let rule_str = rule.to_string();
        let display_name = stored_name.as_deref().unwrap_or(&rule_str);
        println!("Rule: {}", display_name);
        if let Some(desc) = &description {
            println!("  Description: {}", desc);
        }
        println!("  Conclusion: {}", conclusion);
        println!("  Bindings:");
        for (attr, var) in &definition.conclusion.bindings {
            println!("    {} = {}", attr, var);
        }
        println!("  When:");
        for (i, p) in definition.when.iter().enumerate() {
            println!("    {}: the={}, of={}, is={}", i + 1, p.the, p.of, p.is);
        }
        if !definition.unless.is_empty() {
            println!("  Unless:");
            for (i, p) in definition.unless.iter().enumerate() {
                println!("    {}: the={}, of={}, is={}", i + 1, p.the, p.of, p.is);
            }
        }
        println!("  Entity: {}", rule);
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Delete a rule
// ---------------------------------------------------------------------------

/// Delete a rule (by name or entity ID).
///
/// For named rules, retracts the `rule/name` attribute (soft delete).
/// For unnamed rules (identified by entity ID), retracts the `rule/conclusion`
/// attribute to make the rule undiscoverable.
///
/// Uses raw Branch + Instruction (not Session/Transaction) because
/// Transaction deduplicates by `(entity, attribute)`.
pub async fn delete(ctx: &SpaceContext, name_or_id: String, json: bool) -> Result<()> {
    let mut branch = open_branch(ctx).await?;

    // Try lookup by name first, then by entity ID
    let (rule, stored_name) =
        if let Some(entity) = lookup_rule_by_name(&branch, &name_or_id).await? {
            let sn = fetch_string(&branch, &entity, parse_claim_attribute(ATTR_RULE_NAME)?).await?;
            (entity, sn)
        } else if let Ok(entity) = Entity::from_str(&name_or_id) {
            // Verify this entity is actually a rule
            if fetch_string(
                &branch,
                &entity,
                parse_claim_attribute(ATTR_RULE_CONCLUSION)?,
            )
            .await?
            .is_none()
            {
                anyhow::bail!("Rule '{}' not found", name_or_id);
            }
            (entity, None)
        } else {
            anyhow::bail!("Rule '{}' not found", name_or_id);
        };

    let mut instructions = Vec::new();

    // Retract the name if present (soft delete for named rules)
    if let Some(ref sn) = stored_name {
        instructions.push(Instruction::Retract(Artifact {
            the: Attribute::from_str(ATTR_RULE_NAME)?,
            of: rule.clone(),
            is: Value::String(sn.clone()),
            cause: None,
        }));
    }

    // Retract the conclusion to make the rule fully undiscoverable
    if let Some(conclusion) =
        fetch_string(&branch, &rule, parse_claim_attribute(ATTR_RULE_CONCLUSION)?).await?
    {
        instructions.push(Instruction::Retract(Artifact {
            the: Attribute::from_str(ATTR_RULE_CONCLUSION)?,
            of: rule.clone(),
            is: Value::String(conclusion),
            cause: None,
        }));
    }

    if !instructions.is_empty() {
        branch
            .commit(futures_util::stream::iter(instructions))
            .await?;
    }

    let display = stored_name.as_deref().unwrap_or(&name_or_id);
    if json {
        let mut output = serde_json::json!({
            "ok": true,
            "deleted": display,
            "entity": rule.to_string(),
        });
        if let Some(ref n) = stored_name {
            output
                .as_object_mut()
                .unwrap()
                .insert("name".to_string(), serde_json::json!(n));
        }
        println!("{}", serde_json::to_string(&output)?);
    } else {
        println!("Deleted rule '{}'", display);
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Rule loading helpers (used by entity::query)
// ---------------------------------------------------------------------------

/// Load all rule definitions that conclude a given concept name.
///
/// Returns the parsed `RuleDefinition` for each matching rule.
pub async fn load_rules_for_concept<S: ArtifactStore>(
    store: &S,
    concept_name: &ConceptName,
) -> Result<Vec<RuleDefinition>> {
    let rule_entries = find_all_rules(store).await?;

    let mut rules = Vec::new();
    for (rule_ent, _maybe_name) in &rule_entries {
        let conclusion = fetch_string(
            store,
            rule_ent,
            parse_claim_attribute(ATTR_RULE_CONCLUSION)?,
        )
        .await?;
        if conclusion.as_deref() != Some(concept_name.as_str()) {
            continue;
        }
        if let Some(def_str) = fetch_string(
            store,
            rule_ent,
            parse_claim_attribute(ATTR_RULE_DEFINITION)?,
        )
        .await?
        {
            match serde_json::from_str::<RuleDefinition>(&def_str) {
                Ok(def) => rules.push(def),
                Err(e) => {
                    eprintln!(
                        "Warning: skipping rule with malformed definition for concept '{}': {}",
                        concept_name, e
                    );
                }
            }
        } else {
            eprintln!(
                "Warning: rule entity '{}' for concept '{}' has no definition attribute",
                rule_ent, concept_name
            );
        }
    }

    Ok(rules)
}

/// Load all rule definitions in the space, paired with their conclusion concept name.
///
/// Returns `(conclusion_concept_name, RuleDefinition)` for each rule.
pub async fn load_all_rules<S: ArtifactStore>(store: &S) -> Result<Vec<(String, RuleDefinition)>> {
    let rule_entries = find_all_rules(store).await?;

    let mut rules = Vec::new();
    for (rule_ent, _maybe_name) in &rule_entries {
        let conclusion = match fetch_string(
            store,
            rule_ent,
            parse_claim_attribute(ATTR_RULE_CONCLUSION)?,
        )
        .await?
        {
            Some(c) => c,
            None => continue,
        };
        if let Some(def_str) = fetch_string(
            store,
            rule_ent,
            parse_claim_attribute(ATTR_RULE_DEFINITION)?,
        )
        .await?
        {
            match serde_json::from_str::<RuleDefinition>(&def_str) {
                Ok(def) => rules.push((conclusion, def)),
                Err(e) => {
                    eprintln!("Warning: skipping rule with malformed definition: {}", e);
                }
            }
        }
    }

    Ok(rules)
}
