//! Library handlers for the data verbs — the testable core the thin
//! `bin/tonk.rs` handlers call. Each returns rendered stdout; the
//! binary maps errors to exit codes.

use crate::eval::{self, Options, Source};
use crate::output::Format;
use crate::schema::{self, type_to_notation};
use crate::site::TonkSite;

/// Failure modes shared by the data verbs (`describe`, and the
/// upcoming `add`/`set`/`get`/`list`/`rm`). Each maps onto a CLI
/// exit code via [`Self::exit_code`], mirroring [`crate::eval::EvalError`]
/// and [`crate::invite::InviteError`].
#[derive(Debug, thiserror::Error)]
pub enum DataOpError {
    /// No user concept with this name; lists the known concept names.
    #[error("no concept named '{name}'; known concepts: {}", known.join(", "))]
    NoConcept {
        /// The concept name that was looked up.
        name: String,
        /// Names of every concept actually defined on the branch.
        known: Vec<String>,
    },
    /// A field/value error from the notation builders.
    #[error(transparent)]
    Data(#[from] crate::data::DataError),
    /// The underlying eval pipeline failed.
    #[error(transparent)]
    Eval(#[from] crate::eval::EvalError),
    /// I/O, schema read, or repo-not-found failure.
    #[error("{0}")]
    Io(String),
}

impl DataOpError {
    /// CLI exit code for this failure mode.
    pub fn exit_code(&self) -> crate::ExitCode {
        match self {
            DataOpError::NoConcept { .. } | DataOpError::Io(_) => crate::ExitCode::IoError,
            // A bad field/value is an analysis-level rejection, not
            // an I/O failure.
            DataOpError::Data(_) => crate::ExitCode::AnalyzeError,
            DataOpError::Eval(e) => e.exit_code(),
        }
    }
}

/// Look up a user-defined concept by name, or build a
/// [`DataOpError::NoConcept`] enriched with every concept name
/// actually on the branch. Shared by every data verb that takes a
/// `<concept>` argument.
pub async fn require_concept(
    site: &TonkSite,
    concept: &str,
) -> Result<schema::ConceptInfo, DataOpError> {
    match schema::find_concept(site, concept)
        .await
        .map_err(|e| DataOpError::Io(e.to_string()))?
    {
        Some(info) => Ok(info),
        None => {
            let known = schema::list_concepts(site)
                .await
                .map_err(|e| DataOpError::Io(e.to_string()))?
                .into_iter()
                .map(|c| c.name)
                .collect();
            Err(DataOpError::NoConcept {
                name: concept.to_string(),
                known,
            })
        }
    }
}

/// Render a concept's schema as a human-readable field list.
pub async fn describe(site: &TonkSite, concept: &str) -> Result<String, DataOpError> {
    let info = require_concept(site, concept).await?;
    let mut out = String::new();
    if let Some(desc) = info.descriptor.description() {
        out.push_str(desc);
        out.push_str("\n\n");
    }
    for (field, fd) in info.descriptor.with().iter() {
        let ty = fd
            .content_type()
            .map(|t| type_to_notation(&t))
            .unwrap_or_else(|| "any".into());
        let card = format!("{:?}", fd.cardinality()).to_lowercase();
        let req = if fd.is_optional() { "" } else { " (required)" };
        out.push_str(&format!(
            "  --{field} <{ty}> [{card}]{req}  {}\n",
            fd.description()
        ));
    }
    Ok(out)
}

/// Build a query-notation document that binds every field of
/// `concept`'s descriptor: `this: ?e` (or `this: <entity>` when a
/// specific instance is requested) followed by one `<field>:
/// ?<field>` line per field. No `!` head, so evaluating the
/// document is a pure query — nothing commits.
fn query_doc(
    descriptor: &dialog_query::ConceptDescriptor,
    concept: &str,
    entity: Option<&str>,
) -> String {
    let mut doc = format!("{concept}:\n");
    match entity {
        Some(e) => doc.push_str(&format!("  this: {e}\n")),
        None => doc.push_str("  this: ?e\n"),
    }
    for (field, _) in descriptor.with().iter() {
        doc.push_str(&format!("  {field}: ?{field}\n"));
    }
    doc
}

/// Run a read-only query document through the eval pipeline and
/// return its rendered stdout.
async fn run_read(site: &TonkSite, doc: String, json: bool) -> Result<String, DataOpError> {
    let options = Options {
        format: if json { Format::Json } else { Format::Notation },
        quiet: false,
        dry_run: false,
    };
    let outcome = eval::run_against_site(site, Source::Inline(doc), options).await?;
    Ok(outcome.stdout)
}

/// List every instance of `concept`, with every field bound.
/// Rendered as notation by default, or as JSON when `json` is
/// `true`.
pub async fn list(site: &TonkSite, concept: &str, json: bool) -> Result<String, DataOpError> {
    let info = require_concept(site, concept).await?;
    run_read(site, query_doc(&info.descriptor, concept, None), json).await
}

/// Fetch a single instance of `concept` by `entity`, with every
/// field bound. Rendered as notation by default, or as JSON when
/// `json` is `true`.
pub async fn get(
    site: &TonkSite,
    concept: &str,
    entity: &str,
    json: bool,
) -> Result<String, DataOpError> {
    let info = require_concept(site, concept).await?;
    run_read(
        site,
        query_doc(&info.descriptor, concept, Some(entity)),
        json,
    )
    .await
}
