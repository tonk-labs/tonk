//! Live, workflow-shaped orientation for agents.
//!
//! This surface intentionally avoids a product essay. It reads the selected
//! spot and turns its application concepts into commands an agent can execute
//! and verify immediately.

use std::fmt::Write as _;

use anyhow::Result;
use serde::Serialize;

use crate::agents::{self, SpotAgents};
use crate::schema::{self, FieldSummary};
use crate::site::TonkSite;
use crate::spot::Resolved;

/// Version of the structured `tonk context --json` contract.
pub const SCHEMA_VERSION: &str = "tonk.context.v1";

/// Maximum number of concepts expanded in the default text response.
const DEFAULT_CONCEPT_LIMIT: usize = 12;

/// A versioned snapshot of the selected spot's executable workflows.
#[derive(Debug, Serialize)]
pub struct ContextReport {
    /// Structured-output contract version.
    pub schema_version: &'static str,
    /// Exact selected store.
    pub spot: SpotContext,
    /// Synced spot-specific agent context, when asserted.
    pub agents: Option<SpotAgents>,
    /// User/application concepts with command-shaped examples.
    pub concepts: Vec<ConceptContext>,
    /// Commands for an empty application vocabulary.
    pub empty_spot_workflow: Vec<WorkflowStep>,
}

/// Selected spot information.
#[derive(Debug, Serialize)]
pub struct SpotContext {
    /// Owning native profile label, when resolved by the install router.
    pub profile: Option<String>,
    /// Immutable account root indexed by that profile.
    pub account_root: Option<String>,
    /// Whether the owning profile currently has an active provider session.
    pub signed_in: Option<bool>,
    /// Registry name.
    pub name: String,
    /// Absolute site path.
    pub site: String,
    /// `flag`, `env`, or `global`.
    pub selected_via: String,
    /// Tonk's writable branch.
    pub branch: &'static str,
    /// Explicit reminder that changing directory cannot change Tonk data.
    pub cwd_selects_spot: bool,
}

/// One live application concept and its direct workflows.
#[derive(Debug, Serialize)]
pub struct ConceptContext {
    /// Published concept name.
    pub name: String,
    /// Human description, when present.
    pub description: Option<String>,
    /// Typed fields.
    pub fields: Vec<FieldContext>,
    /// Read all current instances.
    pub inspect: WorkflowStep,
    /// Update one current instance.
    pub update: WorkflowStep,
    /// Confirm the exact current state.
    pub verify: WorkflowStep,
    /// Mint a complete new instance.
    pub create: WorkflowStep,
}

/// One typed field.
#[derive(Debug, Serialize)]
pub struct FieldContext {
    /// Flag name accepted by `tonk assert`.
    pub name: String,
    /// Asserted-notation type.
    pub value_type: String,
    /// `one` or `many`.
    pub cardinality: String,
    /// Whether minting requires the field.
    pub required: bool,
    /// Human description.
    pub description: String,
}

/// One executable step.
#[derive(Debug, Serialize)]
pub struct WorkflowStep {
    /// Why to run it.
    pub purpose: &'static str,
    /// Shell command with explicit placeholders where live values are needed.
    pub command: String,
    /// Whether the command can change spot state.
    pub mutates: bool,
}

impl From<FieldSummary> for FieldContext {
    fn from(field: FieldSummary) -> Self {
        Self {
            name: field.name,
            value_type: field.value_type,
            cardinality: field.cardinality,
            required: field.required,
            description: field.description,
        }
    }
}

/// Read the selected spot and derive direct workflows from its live schema.
pub async fn inspect(resolved: &Resolved, site: &TonkSite) -> Result<ContextReport> {
    inspect_inner(resolved, site, None, None, None).await
}

/// Read a profile-qualified selected space and expose its account context.
pub async fn inspect_profiled(
    resolved: &crate::account_profiles::ResolvedSpace,
    site: &TonkSite,
) -> Result<ContextReport> {
    let legacy = Resolved {
        name: resolved.name.clone(),
        site: resolved.site.clone(),
        source: resolved.source.clone(),
    };
    let signed_in = matches!(
        resolved.profile.sign_in_state(),
        Ok(crate::account_profiles::ProfileSignIn::Active)
    );
    inspect_inner(
        &legacy,
        site,
        Some(resolved.profile.record.label.clone()),
        resolved.profile.record.account_root.clone(),
        Some(signed_in),
    )
    .await
}

async fn inspect_inner(
    resolved: &Resolved,
    site: &TonkSite,
    profile: Option<String>,
    account_root: Option<String>,
    signed_in: Option<bool>,
) -> Result<ContextReport> {
    let live_concepts = schema::list_concepts(site).await?;
    let agents_declared = live_concepts
        .iter()
        .any(|concept| concept.name == agents::CONCEPT_NAME);
    let spot_agents = if agents_declared {
        agents::get_declared(site).await?
    } else {
        None
    };
    let concepts = live_concepts
        .into_iter()
        .filter(|concept| {
            concept.name != "command"
                && concept.name != "space-home"
                && !crate::site::standard_library_has_name(&concept.name)
        })
        .map(|concept| {
            let name = concept.name;
            let update_field = concept
                .field_specs
                .iter()
                .find(|field| field.value_type == "boolean")
                .or_else(|| concept.field_specs.first());
            let update = match update_field {
                Some(field) => format!(
                    "tonk assert {name} <ENTITY> --{} {}",
                    field.name,
                    example_value(field)
                ),
                None => format!("tonk assert {name} <ENTITY> --help"),
            };
            let required: Vec<_> = concept
                .field_specs
                .iter()
                .filter(|field| field.required)
                .collect();
            let create_fields: Vec<_> = if required.is_empty() {
                concept.field_specs.iter().take(1).collect()
            } else {
                required
            };
            let create = if create_fields.is_empty() {
                format!("tonk assert {name} --help")
            } else {
                format!(
                    "tonk assert {name} {}",
                    create_fields
                        .iter()
                        .map(|field| format!("--{} {}", field.name, example_value(field)))
                        .collect::<Vec<_>>()
                        .join(" ")
                )
            };
            ConceptContext {
                inspect: WorkflowStep {
                    purpose: "read current instances and copy an entity ID",
                    command: format!("tonk query {name} --json"),
                    mutates: false,
                },
                update: WorkflowStep {
                    purpose: "update one existing instance",
                    command: update,
                    mutates: true,
                },
                verify: WorkflowStep {
                    purpose: "verify that exact instance after the write",
                    command: format!("tonk query {name} <ENTITY> --json"),
                    mutates: false,
                },
                create: WorkflowStep {
                    purpose: "create a complete new instance",
                    command: create,
                    mutates: true,
                },
                name,
                description: concept.description,
                fields: concept.field_specs.into_iter().map(Into::into).collect(),
            }
        })
        .collect();

    Ok(ContextReport {
        schema_version: SCHEMA_VERSION,
        spot: SpotContext {
            profile,
            account_root,
            signed_in,
            name: resolved.name.clone(),
            site: resolved.site.display().to_string(),
            selected_via: resolved.source.to_string(),
            branch: "main",
            cwd_selects_spot: false,
        },
        agents: spot_agents,
        concepts,
        empty_spot_workflow: vec![
            WorkflowStep {
                purpose: "define a model",
                command:
                    "tonk concept add note --attr title:text:one --attr body:text:one".to_string(),
                mutates: true,
            },
            WorkflowStep {
                purpose: "create its first instance",
                command: "tonk assert note --title \"example\" --body \"example\"".to_string(),
                mutates: true,
            },
            WorkflowStep {
                purpose: "give it a visible view",
                command:
                    "tonk view add note --template '<article><h2>{title}</h2><p>{body}</p></article>'"
                        .to_string(),
                mutates: true,
            },
            WorkflowStep {
                purpose: "surface it on the space home",
                command: "tonk home note".to_string(),
                mutates: true,
            },
        ],
    })
}

fn example_value(field: &FieldSummary) -> &'static str {
    match field.value_type.as_str() {
        "boolean" => "true",
        "text" => "\"example\"",
        "unsigned-integer" | "signed-integer" | "float" => "1",
        "entity" => "<ENTITY>",
        _ => "<VALUE>",
    }
}

impl ContextReport {
    /// Render the bounded default response.
    pub fn render_markdown(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "# Tonk context");
        let _ = writeln!(
            out,
            "spot: `{}` · branch: `{}` · selected via: `{}`",
            self.spot.name, self.spot.branch, self.spot.selected_via
        );
        let _ = writeln!(out, "site: `{}`", self.spot.site);
        if let Some(profile) = &self.spot.profile {
            let account = self.spot.account_root.as_deref().unwrap_or("pending");
            let signed_in = if self.spot.signed_in == Some(true) {
                "yes"
            } else {
                "no"
            };
            let _ = writeln!(
                out,
                "profile: `{profile}` · account: `{account}` · signed in: {signed_in}"
            );
        }
        out.push_str("Changing cwd does not change the selected Tonk data.\n");

        if let Some(agents) = &self.agents {
            let _ = writeln!(
                out,
                "\n## Spot AGENTS.md\nsource: `{}` `{}` on `{}` at `{}`\n",
                agents.source, agents.attribute, agents.entity, agents.revision
            );
            out.push_str(agents.markdown.trim_end());
            out.push('\n');
        }

        if self.concepts.is_empty() {
            out.push_str("\n## Build a first visible model\n");
            render_steps(&mut out, &self.empty_spot_workflow);
            return out;
        }

        let primary = &self.concepts[0];
        let _ = writeln!(out, "\n## Update an existing `{}` safely", primary.name);
        render_steps(
            &mut out,
            [&primary.inspect, &primary.update, &primary.verify],
        );

        let _ = writeln!(
            out,
            "\n## Live application concepts ({})",
            self.concepts.len()
        );
        for concept in self.concepts.iter().take(DEFAULT_CONCEPT_LIMIT) {
            let description = concept.description.as_deref().unwrap_or("no description");
            let _ = writeln!(out, "\n`{}` — {}", concept.name, description);
            if concept.fields.is_empty() {
                out.push_str("  fields: (none)\n");
            } else {
                out.push_str("  fields: ");
                for (index, field) in concept.fields.iter().enumerate() {
                    if index > 0 {
                        out.push_str(", ");
                    }
                    let required = if field.required {
                        "required"
                    } else {
                        "optional"
                    };
                    let _ = write!(
                        out,
                        "`{}` {}/{}/{}",
                        field.name, field.value_type, field.cardinality, required
                    );
                }
                out.push('\n');
            }
            let _ = writeln!(out, "  inspect: `{}`", concept.inspect.command);
            let _ = writeln!(out, "  update: `{}`", concept.update.command);
            let _ = writeln!(out, "  create: `{}`", concept.create.command);
        }
        if self.concepts.len() > DEFAULT_CONCEPT_LIMIT {
            let _ = writeln!(
                out,
                "\n{} more concepts; run `tonk context --json` for the complete contract.",
                self.concepts.len() - DEFAULT_CONCEPT_LIMIT
            );
        }
        out
    }

    /// Render the stable structured contract.
    pub fn render_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self).map(|mut json| {
            json.push('\n');
            json
        })
    }
}

fn render_steps<'a>(out: &mut String, steps: impl IntoIterator<Item = &'a WorkflowStep>) {
    for (index, step) in steps.into_iter().enumerate() {
        let safety = if step.mutates { "writes" } else { "read-only" };
        let _ = writeln!(
            out,
            "{}. {} ({safety}): `{}`",
            index + 1,
            step.purpose,
            step.command
        );
    }
}
