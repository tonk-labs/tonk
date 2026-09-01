//! Live, workflow-shaped orientation for agents.
//!
//! This surface intentionally avoids a product essay. It reads the selected
//! space and turns its application concepts into commands an agent can execute
//! and verify immediately.

use std::fmt::Write as _;

use anyhow::Result;
use serde::Serialize;

use crate::agents::{self, SpaceAgents};
use crate::schema::{self, FieldSummary};
use crate::site::TonkSite;
use crate::space::Resolved;

/// Legacy version retained only for the in-process context renderer.
///
/// v2 normalized the legacy key to `space` and moved the whole document to
/// camelCase, matching `tonk space --json` and the registry's own account
/// record.
///
/// v3 absorbed the sync and account sections. Four commands used to answer
/// "where am I" in four layouts, naming the same field three ways in the
/// same breath (`space:` / `current space:` / ``space: `demo` ``). They are
/// now projections of this one document, so the vocabulary is defined once.
pub const SCHEMA_VERSION: &str = "tonk.context.v3";

/// Maximum number of concepts expanded in the default text response.
const DEFAULT_CONCEPT_LIMIT: usize = 12;

/// A versioned snapshot of the selected space's executable workflows.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextReport {
    /// Structured-output contract version.
    pub schema_version: &'static str,
    /// Exact selected store.
    pub space: SpaceContext,
    /// Where the branch stands against its upstream.
    pub sync: SyncContext,
    /// Whether this device is signed in, and to what.
    pub account: AccountContext,
    /// Synced space-specific agent context, when asserted.
    pub agents: Option<SpaceAgents>,
    /// User/application concepts with command-shaped examples.
    pub concepts: Vec<ConceptContext>,
    /// Commands for an empty application vocabulary.
    pub empty_space_workflow: Vec<WorkflowStep>,
}

/// Selected space information.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpaceContext {
    /// Registry name.
    pub name: String,
    /// Absolute site path.
    pub site: String,
    /// `flag`, `env`, or `directory <path>`.
    pub selected_via: String,
    /// Tonk's writable branch.
    pub branch: &'static str,
    /// Explicit reminder that changing directory cannot change Tonk data.
    pub cwd_selects_space: bool,
}

/// Where the branch stands against its upstream.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncContext {
    /// Exact local/upstream relationship, including an offline inspection.
    pub state: ContextSyncState,
    /// Local tree hash, absent on a branch with no commits.
    pub hash: Option<String>,
    /// Whether the upstream head was fetched to reach this verdict.
    ///
    /// Whether the upstream comparison performed a fetch.
    /// stays offline. A reader that acts on `state` needs to know which
    /// it got, because an unfetched `synced` only means "nothing local
    /// has happened since the last fetch".
    pub fetched: bool,
}

/// Stable token shared by text and JSON sync reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ContextSyncState {
    /// No upstream is configured.
    NoUpstream,
    /// Local and upstream revisions match.
    Synced,
    /// Local has commits the upstream does not.
    Ahead,
    /// Upstream has commits local does not.
    Behind,
    /// Both sides have unique commits.
    Diverged,
    /// An upstream exists but this report deliberately stayed offline.
    NotFetched,
}

impl From<tonk_schema::SyncState> for ContextSyncState {
    fn from(state: tonk_schema::SyncState) -> Self {
        match state {
            tonk_schema::SyncState::NoUpstream => Self::NoUpstream,
            tonk_schema::SyncState::Synced => Self::Synced,
            tonk_schema::SyncState::Ahead => Self::Ahead,
            tonk_schema::SyncState::Behind => Self::Behind,
            tonk_schema::SyncState::Diverged => Self::Diverged,
        }
    }
}

/// Whether this device is signed in, and to what.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountContext {
    /// Whether an account is registered on this device.
    pub signed_in: bool,
    /// Root DID, absent before one is provisioned.
    pub account: Option<String>,
    /// Account service base URL, absent when unregistered.
    pub account_service: Option<String>,
    /// This device's DID, absent when the local identity could not be read.
    pub device: Option<String>,
    /// Service-side account state, absent when unregistered.
    pub state: Option<String>,
}

/// One live application concept and its direct workflows.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
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
#[serde(rename_all = "camelCase")]
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
#[serde(rename_all = "camelCase")]
pub struct WorkflowStep {
    /// Why to run it.
    pub purpose: &'static str,
    /// Shell command with explicit placeholders where live values are needed.
    pub command: String,
    /// Whether the command can change space state.
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

/// Read the selected space and derive direct workflows from its live schema.
///
/// `sync` and `account` are passed in rather than gathered here: the caller
/// already holds the profile and store they need, and how the sync state was
/// reached is the caller's decision — `tonk status` fetches, while other callers
/// does not.
pub async fn inspect(
    resolved: &Resolved,
    site: &TonkSite,
    sync: SyncContext,
    account: AccountContext,
) -> Result<ContextReport> {
    // One enumeration answers both questions: whether the branch
    // declares the standard library's `tonk/agents` concept, and which
    // concepts this space's author defined. `is_system_concept` is the
    // same filter bare `tonk concept` applies.
    let live_concepts = schema::list_all_concepts(site).await?;
    let agents_declared = live_concepts
        .iter()
        .any(|concept| concept.name == agents::CONCEPT_NAME);
    let space_agents = if agents_declared {
        agents::get_declared(site).await?
    } else {
        None
    };
    let concepts = live_concepts
        .into_iter()
        .filter(|concept| !schema::is_system_concept(&concept.name))
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
                    command: format!("tonk show {name} <ENTITY> --json"),
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
        sync,
        account,
        space: SpaceContext::new(resolved),
        agents: space_agents,
        concepts,
        empty_space_workflow: vec![
            WorkflowStep {
                purpose: "define a model",
                command:
                    "tonk concept add note --field title:text:one --field body:text:one".to_string(),
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
                command: "tonk space home note".to_string(),
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

/// One-line rendering of a sync state token: the token plus a short
/// gloss of what to do about it.
///
/// Takes the token rather than the enum so the text and JSON forms
/// cannot drift into naming the same state differently.
pub fn sync_state_gloss(state: ContextSyncState) -> &'static str {
    match state {
        ContextSyncState::NoUpstream => {
            "no-upstream (set one with `tonk remote set-upstream <name>`)"
        }
        ContextSyncState::Synced => "synced",
        ContextSyncState::Ahead => "ahead (local has unpushed commits; run `tonk push`)",
        ContextSyncState::Behind => "behind (upstream has new commits; run `tonk pull`)",
        ContextSyncState::Diverged => "diverged (run `tonk pull` to merge, then `tonk push`)",
        // An offline caller does not fetch. It
        // can see that an upstream is configured but not where the branch
        // stands against it, and says which.
        ContextSyncState::NotFetched => "upstream configured, not checked (run `tonk status`)",
    }
}

impl SpaceContext {
    /// Build the selected-space section from one resolution.
    pub fn new(resolved: &Resolved) -> Self {
        Self {
            name: resolved.name.clone(),
            site: resolved.site.display().to_string(),
            selected_via: resolved.source.to_string(),
            branch: crate::site::BRANCH_NAME,
            cwd_selects_space: false,
        }
    }

    /// What `tonk space use` prints with no name.
    ///
    /// One vocabulary for all four "where am I" commands. This field was
    /// `current space:` in `tonk space use`, `space:` in `tonk status`,
    /// and ``space: `demo` `` in status output, with `selected via:`
    /// spelled three ways to match.
    pub fn render(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "space: {}", self.name);
        let _ = writeln!(out, "site: {}", self.site);
        let _ = writeln!(out, "selected via: {}", self.selected_via);
        let _ = writeln!(out, "branch: {}", self.branch);
        out
    }
}

impl SyncContext {
    /// A live sync classification reached after fetching the upstream.
    pub fn fetched(state: tonk_schema::SyncState, hash: Option<String>) -> Self {
        Self {
            state: state.into(),
            hash,
            fetched: true,
        }
    }

    /// A bounded local-only classification for callers that avoid fetching.
    pub fn offline(upstream_configured: bool, hash: Option<String>) -> Self {
        Self {
            state: if upstream_configured {
                ContextSyncState::NotFetched
            } else {
                ContextSyncState::NoUpstream
            },
            hash,
            fetched: false,
        }
    }

    /// What `tonk status` prints.
    pub fn render(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "sync: {}", sync_state_gloss(self.state));
        if let Some(hash) = &self.hash {
            let _ = writeln!(out, "hash: {hash}");
        }
        out
    }
}

impl AccountContext {
    /// What `tonk account status` prints.
    pub fn render(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(
            out,
            "signed in: {}",
            if self.signed_in { "yes" } else { "no" }
        );
        let _ = writeln!(
            out,
            "account: {}",
            self.account.as_deref().unwrap_or("missing")
        );
        let _ = writeln!(
            out,
            "account service: {}",
            self.account_service.as_deref().unwrap_or("none")
        );
        let _ = writeln!(
            out,
            "device: {}",
            self.device.as_deref().unwrap_or("unavailable")
        );
        if let Some(state) = &self.state {
            let _ = writeln!(out, "account status: {state}");
        }
        out
    }
}

impl ContextReport {
    /// Render the bounded default response.
    pub fn render_markdown(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "# Tonk context");
        out.push_str(&self.space.render());
        out.push_str(&self.sync.render());
        out.push_str(&self.account.render());
        out.push_str("Changing cwd does not change the selected Tonk data.\n");

        if let Some(agents) = &self.agents {
            let _ = writeln!(
                out,
                "\n## Space AGENTS.md\nsource: `{}` `{}` on `{}` at `{}`\n",
                agents.source, agents.attribute, agents.entity, agents.revision
            );
            out.push_str(agents.markdown.trim_end());
            out.push('\n');
        }

        if self.concepts.is_empty() {
            out.push_str("\n## Build a first visible model\n");
            render_steps(&mut out, &self.empty_space_workflow);
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
                "\n{} more concepts; run `tonk concept --json` for the complete list.",
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
