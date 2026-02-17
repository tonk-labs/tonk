//! Rule management: define, list, show, and delete deductive rules.
//!
//! A rule derives entities of a conclusion concept from patterns across
//! existing facts. Rules are stored as JSON blobs in dialog-db and compiled
//! into `DeductiveRule`s at query time.

use crate::schema::*;
use anyhow::{Context, Result};
use dialog_artifacts::{Artifact, ArtifactStore, ArtifactStoreMut, Instruction};
use dialog_query::claim::Attribute;
use dialog_query::{DeductiveRule, Premise, Term, Value};
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

        Ok(def)
    }
}

/// The conclusion of a rule: which concept is derived, and how its
/// attributes map to variables.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleConclusion {
    /// Name of the conclusion concept (e.g. "SafeMeal").
    pub concept: String,

    /// Maps concept attribute short names to variables.
    /// e.g. `{"attendee": "?person", "recipe": "?recipe"}`
    pub bindings: HashMap<String, String>,

    /// Which entity variable in the premises should bind to the
    /// concept's implicit `this` entity. e.g. `"?allergy"`.
    ///
    /// This is distinct from attribute `bindings`: it designates the
    /// *identity* of each derived instance rather than an attribute value.
    ///
    /// If omitted, defaults to the first entity variable (`of` field)
    /// found in the `when` premises.
    #[serde(default)]
    pub this: Option<String>,
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
/// Additionally, the `this` field (or first entity variable) is mapped
/// to `"this"` — the implicit entity operand every Concept has.
///
/// If renaming would collide with existing premise variables that are
/// NOT themselves being renamed, those existing variables are also
/// given new names (suffixed with `_0`, `_1`, etc.) to avoid clashes.
///
/// Returns a map from user variable name (without `?`) to new variable name.
fn build_rename_map(
    definition: &RuleDefinition,
    concept_name: &ConceptName,
) -> Result<HashMap<String, String>> {
    let all_vars = collect_all_premise_vars(definition);
    let mut rename: HashMap<String, String> = HashMap::new();

    // Map each conclusion binding: user variable → attribute short name
    let mut binding_vars: std::collections::HashSet<String> = std::collections::HashSet::new();
    for (attr_short, var_str) in &definition.conclusion.bindings {
        if let PremiseTerm::Variable(var_name) = PremiseTerm::parse(var_str) {
            // Validate the attribute is in the concept
            let _qualified = qualify_attribute(concept_name, attr_short)?;
            rename.insert(var_name.clone(), attr_short.clone());
            binding_vars.insert(var_name);
        }
        // Constants in bindings are allowed — they don't need renaming
    }

    // Map `this`: find which entity variable should become "this"
    let this_var = if let Some(ref this_str) = definition.conclusion.this {
        let name = match PremiseTerm::parse(this_str) {
            PremiseTerm::Variable(name) => name,
            _ => anyhow::bail!(
                "Conclusion 'this' must be a variable (starting with '?'), got: {}",
                this_str
            ),
        };

        // If the user explicitly sets `this` to a variable that's also
        // used as a conclusion binding, that's an error — one variable
        // can't map to two different operand names.
        if binding_vars.contains(&name) {
            anyhow::bail!(
                "Variable '?{}' is used both as 'this' and as a conclusion binding attribute. \
                 Use separate variables for the entity (this) and the attribute binding. \
                 For example, add a dedicated entity variable like '?meal' with 'this: ?meal' \
                 in the conclusion.",
                name
            );
        }

        name
    } else {
        // Auto-detect: use the first entity variable from `when` premises
        // that is NOT already claimed by a conclusion binding.
        definition
            .when
            .iter()
            .filter_map(|p| {
                PremiseTerm::parse(&p.of)
                    .variable_name()
                    .map(|v| v.to_string())
            })
            .find(|v| !binding_vars.contains(v))
            .context(
                "No suitable entity variable found in 'when' premises to use as 'this'. \
                 All entity variables are already claimed by conclusion bindings. \
                 Add an explicit 'this: ?var' to a when-premise that uses a dedicated \
                 entity variable, or set 'this' in the conclusion.",
            )?
    };

    rename.insert(this_var, "this".to_string());

    // Detect collisions: a rename target name might already be used by
    // a different variable in the premises that is NOT being renamed.
    // e.g. bindings map ?recipe_name → "recipe", but ?recipe is already
    // used as an entity variable. We need to rename ?recipe to avoid clash.
    let rename_targets: std::collections::HashSet<String> = rename.values().cloned().collect();
    let renamed_sources: std::collections::HashSet<String> = rename.keys().cloned().collect();

    let mut collision_renames: Vec<(String, String)> = Vec::new();
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
                    && !collision_renames.iter().any(|(_, t)| t == &candidate)
                {
                    collision_renames.push((target_name.clone(), candidate));
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
pub fn compile_rule(
    definition: &RuleDefinition,
    concept_name: &ConceptName,
    concept_attrs: &[String],
    cardinalities: &std::collections::HashMap<String, dialog_query::Cardinality>,
) -> Result<DeductiveRule> {
    // 1. Build dynamic Concept from conclusion concept's attributes
    let concept = build_dynamic_concept(concept_attrs, cardinalities)?;

    // 2. Build variable renaming map from conclusion bindings
    let rename = build_rename_map(definition, concept_name)?;

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
) -> Result<()> {
    // Check that all binding keys are valid concept attributes.
    // A binding key like "comment" matches either the concept-derived path
    // (e.g. "annotatedlink/comment") or any attribute whose short name
    // matches (e.g. "carry.links/comment" where the part after "/" is "comment").
    for short_name in definition.conclusion.bindings.keys() {
        if short_name == "this" {
            continue; // `this` is the entity binding, not an attribute
        }
        let concept_qualified = qualify_attribute(conclusion_name, short_name)?;
        let matches = conclusion_attrs.iter().any(|a| {
            *a == concept_qualified || a.rsplit_once('/').is_some_and(|(_, n)| n == short_name)
        });
        if !matches {
            anyhow::bail!(
                "Conclusion binding '{}' is not an attribute of concept '{}'. \
                 Known attributes: {}",
                short_name,
                conclusion_name,
                conclusion_attrs.join(", ")
            );
        }
    }

    // Collect variables from positive (`when`) premises only.
    // Variables must be grounded in at least one positive premise — appearing
    // only in `unless` is unsafe (Datalog safety requirement).
    let when_vars = collect_when_vars(definition);

    // Check that all conclusion binding variables appear in at least one `when` premise
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

    // Also validate that the `this` variable (if explicit) appears in a `when` premise
    if let Some(ref this_str) = definition.conclusion.this
        && let PremiseTerm::Variable(var_name) = PremiseTerm::parse(this_str)
        && !when_vars.contains(&var_name)
    {
        anyhow::bail!(
            "Variable '?{}' used as 'this' does not appear in any positive ('when') \
             premise. The entity variable must be grounded in 'when' premises.",
            var_name,
        );
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// List all rules
// ---------------------------------------------------------------------------

/// List all rules in the active space.
pub async fn list(json: bool) -> Result<()> {
    let ctx = get_space_context()?;
    let session = open_session(&ctx).await?;

    let registry = registry_entity(&ctx.space_did)?;
    let rule_entities = fetch_entity_values(&session, &registry, ATTR_REGISTRY_RULE).await?;

    if rule_entities.is_empty() {
        if json {
            println!("[]");
        } else {
            println!("No rules defined. Use 'tonk rule define <name>' to create one.");
        }
        return Ok(());
    }

    let mut rules: Vec<(String, Option<String>, String)> = Vec::new();
    for entity in &rule_entities {
        let name = match fetch_string(&session, entity, ATTR_RULE_NAME).await? {
            Some(n) => n,
            None => {
                eprintln!(
                    "Warning: rule entity '{}' is missing its 'rule/name' attribute — possible data corruption",
                    entity
                );
                "???".to_string()
            }
        };
        let description = fetch_string(&session, entity, ATTR_RULE_DESCRIPTION).await?;
        let conclusion = match fetch_string(&session, entity, ATTR_RULE_CONCLUSION).await? {
            Some(c) => c,
            None => {
                eprintln!(
                    "Warning: rule '{}' is missing its 'rule/conclusion' attribute — possible data corruption",
                    name
                );
                "???".to_string()
            }
        };
        rules.push((name, description, conclusion));
    }

    if json {
        let items: Vec<serde_json::Value> = rules
            .iter()
            .map(|(name, desc, conclusion)| {
                let mut obj = serde_json::json!({
                    "name": name,
                    "conclusion": conclusion,
                });
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
        for (name, desc, conclusion) in &rules {
            let desc_str = desc
                .as_ref()
                .map(|d| format!(" - {}", d))
                .unwrap_or_default();
            println!("  {} -> {}{}", name, conclusion, desc_str);
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Define a new rule
// ---------------------------------------------------------------------------

/// Define a new rule from a JSON definition.
///
/// Uses raw Branch + Instruction (not Session/Transaction) because the
/// registry entity has multi-valued `registry/rule` attributes. Transaction
/// deduplicates by `(entity, attribute)` — only one value survives per pair —
/// so it cannot correctly accumulate multiple rule references on the registry.
pub async fn define(
    name: String,
    file: Option<String>,
    stdin: bool,
    description: Option<String>,
    json: bool,
) -> Result<()> {
    validate_safe_name(&name, "Rule")?;

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

    let ctx = get_space_context()?;
    let mut branch = open_branch(&ctx).await?;

    let registry = registry_entity(&ctx.space_did)?;
    let rule = rule_entity(&ctx.space_did, &name)?;

    // Check if rule already exists
    let existing = fetch_string(&branch, &rule, ATTR_RULE_NAME).await?;
    if existing.is_some() {
        anyhow::bail!(
            "Rule '{}' already exists. Delete it first with 'tonk rule delete {}'.",
            name,
            name
        );
    }

    // Verify the conclusion concept exists and get its attributes
    let conclusion_concept = ConceptName::new(&definition.conclusion.concept)?;
    let concept = concept_entity(&ctx.space_did, &conclusion_concept)?;
    let concept_name = fetch_string(&branch, &concept, ATTR_CONCEPT_NAME)
        .await?
        .context(format!(
            "Conclusion concept '{}' not found. Define it first.",
            definition.conclusion.concept
        ))?;
    let concept_name = ConceptName::from_stored(concept_name);

    let concept_attrs = fetch_string_values(&branch, &concept, ATTR_CONCEPT_ATTRIBUTE).await?;
    let cardinalities =
        fetch_attribute_cardinalities(&branch, &ctx.space_did, &concept_name, &concept_attrs)
            .await?;

    // Validate the definition
    validate_definition(&definition, &concept_attrs, &concept_name)?;

    // Try to compile the rule to catch errors early
    compile_rule(&definition, &concept_name, &concept_attrs, &cardinalities).context(
        "Rule definition is invalid. Check that variable names match between \
         conclusion bindings and premises.",
    )?;

    // Serialize definition back to canonical JSON for storage
    let definition_str = serde_json::to_string(&definition)?;

    // Build instructions
    let mut instructions = vec![
        // Register in registry
        Instruction::Assert(Artifact {
            the: Attribute::from_str(ATTR_REGISTRY_RULE)?,
            of: registry.clone(),
            is: Value::Entity(rule.clone()),
            cause: None,
        }),
        // Rule name
        Instruction::Assert(Artifact {
            the: Attribute::from_str(ATTR_RULE_NAME)?,
            of: rule.clone(),
            is: Value::String(name.clone()),
            cause: None,
        }),
        // Rule conclusion concept name
        Instruction::Assert(Artifact {
            the: Attribute::from_str(ATTR_RULE_CONCLUSION)?,
            of: rule.clone(),
            is: Value::String(concept_name.to_string()),
            cause: None,
        }),
        // Rule definition JSON
        Instruction::Assert(Artifact {
            the: Attribute::from_str(ATTR_RULE_DEFINITION)?,
            of: rule.clone(),
            is: Value::String(definition_str.clone()),
            cause: None,
        }),
    ];

    // Description (optional)
    if let Some(desc) = &description {
        instructions.push(Instruction::Assert(Artifact {
            the: Attribute::from_str(ATTR_RULE_DESCRIPTION)?,
            of: rule.clone(),
            is: Value::String(desc.clone()),
            cause: None,
        }));
    }

    branch
        .commit(futures_util::stream::iter(instructions))
        .await?;

    if json {
        let output = serde_json::json!({
            "ok": true,
            "name": name,
            "conclusion": concept_name.as_str(),
            "when_count": definition.when.len(),
            "unless_count": definition.unless.len(),
        });
        println!("{}", serde_json::to_string(&output)?);
    } else {
        println!("Defined rule '{}'", name);
        println!("  Conclusion: {}", concept_name);
        println!(
            "  Premises: {} when, {} unless",
            definition.when.len(),
            definition.unless.len()
        );
        if let Some(desc) = &description {
            println!("  Description: {}", desc);
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Show rule details
// ---------------------------------------------------------------------------

/// Show the full definition of a rule.
pub async fn show(name: String, json: bool) -> Result<()> {
    let ctx = get_space_context()?;
    let session = open_session(&ctx).await?;

    let rule = rule_entity(&ctx.space_did, &name)?;

    let stored_name = fetch_string(&session, &rule, ATTR_RULE_NAME)
        .await?
        .context(format!("Rule '{}' not found", name))?;

    let description = fetch_string(&session, &rule, ATTR_RULE_DESCRIPTION).await?;
    let conclusion = match fetch_string(&session, &rule, ATTR_RULE_CONCLUSION).await? {
        Some(c) => c,
        None => {
            eprintln!(
                "Warning: rule '{}' is missing its 'rule/conclusion' attribute — possible data corruption",
                name
            );
            "???".to_string()
        }
    };
    let definition_str = fetch_string(&session, &rule, ATTR_RULE_DEFINITION)
        .await?
        .context("Rule definition not found")?;

    let definition: RuleDefinition =
        serde_json::from_str(&definition_str).context("Failed to parse stored rule definition")?;

    if json {
        let mut output = serde_json::json!({
            "name": stored_name,
            "conclusion": conclusion,
            "definition": definition,
            "entity": rule.to_string(),
        });
        if let Some(desc) = &description {
            output
                .as_object_mut()
                .unwrap()
                .insert("description".to_string(), serde_json::json!(desc));
        }
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        println!("Rule: {}", stored_name);
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

/// Delete a rule.
///
/// Uses raw Branch + Instruction (not Session/Transaction) for the same
/// reason as [`define`]: the registry's multi-valued `registry/rule`
/// attribute cannot be correctly retracted via Transaction.
pub async fn delete(name: String, json: bool) -> Result<()> {
    let ctx = get_space_context()?;
    let mut branch = open_branch(&ctx).await?;

    let registry = registry_entity(&ctx.space_did)?;
    let rule = rule_entity(&ctx.space_did, &name)?;

    // Verify rule exists
    let stored_name = fetch_string(&branch, &rule, ATTR_RULE_NAME)
        .await?
        .context(format!("Rule '{}' not found", name))?;

    let mut instructions = Vec::new();

    // Retract rule name
    instructions.push(Instruction::Retract(Artifact {
        the: Attribute::from_str(ATTR_RULE_NAME)?,
        of: rule.clone(),
        is: Value::String(stored_name),
        cause: None,
    }));

    // Retract conclusion
    if let Some(conclusion) = fetch_string(&branch, &rule, ATTR_RULE_CONCLUSION).await? {
        instructions.push(Instruction::Retract(Artifact {
            the: Attribute::from_str(ATTR_RULE_CONCLUSION)?,
            of: rule.clone(),
            is: Value::String(conclusion),
            cause: None,
        }));
    }

    // Retract definition
    if let Some(def) = fetch_string(&branch, &rule, ATTR_RULE_DEFINITION).await? {
        instructions.push(Instruction::Retract(Artifact {
            the: Attribute::from_str(ATTR_RULE_DEFINITION)?,
            of: rule.clone(),
            is: Value::String(def),
            cause: None,
        }));
    }

    // Retract description
    if let Some(desc) = fetch_string(&branch, &rule, ATTR_RULE_DESCRIPTION).await? {
        instructions.push(Instruction::Retract(Artifact {
            the: Attribute::from_str(ATTR_RULE_DESCRIPTION)?,
            of: rule.clone(),
            is: Value::String(desc),
            cause: None,
        }));
    }

    // Retract registry entry
    instructions.push(Instruction::Retract(Artifact {
        the: Attribute::from_str(ATTR_REGISTRY_RULE)?,
        of: registry.clone(),
        is: Value::Entity(rule.clone()),
        cause: None,
    }));

    branch
        .commit(futures_util::stream::iter(instructions))
        .await?;

    if json {
        let output = serde_json::json!({
            "ok": true,
            "deleted": name,
        });
        println!("{}", serde_json::to_string(&output)?);
    } else {
        println!("Deleted rule '{}'", name);
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
    space_did: &str,
    concept_name: &ConceptName,
) -> Result<Vec<RuleDefinition>> {
    let registry = registry_entity(space_did)?;
    let rule_entities = fetch_entity_values(store, &registry, ATTR_REGISTRY_RULE).await?;

    let mut rules = Vec::new();
    for rule_entity in &rule_entities {
        let conclusion = fetch_string(store, rule_entity, ATTR_RULE_CONCLUSION).await?;
        if conclusion.as_deref() != Some(concept_name.as_str()) {
            continue;
        }
        if let Some(def_str) = fetch_string(store, rule_entity, ATTR_RULE_DEFINITION).await? {
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
                rule_entity, concept_name
            );
        }
    }

    Ok(rules)
}

/// Load all rule definitions in the space, paired with their conclusion concept name.
///
/// Returns `(conclusion_concept_name, RuleDefinition)` for each rule.
pub async fn load_all_rules<S: ArtifactStore>(
    store: &S,
    space_did: &str,
) -> Result<Vec<(String, RuleDefinition)>> {
    let registry = registry_entity(space_did)?;
    let rule_entities = fetch_entity_values(store, &registry, ATTR_REGISTRY_RULE).await?;

    let mut rules = Vec::new();
    for rule_entity in &rule_entities {
        let conclusion = match fetch_string(store, rule_entity, ATTR_RULE_CONCLUSION).await? {
            Some(c) => c,
            None => continue,
        };
        if let Some(def_str) = fetch_string(store, rule_entity, ATTR_RULE_DEFINITION).await? {
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
