//! Library handlers for the data verbs — the testable core the thin
//! `bin/tonk.rs` handlers call. Each returns rendered stdout; the
//! binary maps errors to exit codes.

use crate::data::build_add;
use crate::eval::{self, Options, Source};
use crate::output::Format;
use crate::schema::{self, type_to_notation};
use crate::site::TonkSite;

pub mod flags;

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
    /// The raw CLI flags for a schema-aware verb (`add`, `set`)
    /// failed clap's dynamically-built parse: an unknown `--flag`,
    /// a missing required one, or a bad value for the arg's type.
    /// Display text mirrors clap's own rendered error (its usage
    /// line already enumerates the concept's real flags), minus
    /// clap's own `"error: "` header — every other `DataOpError`
    /// variant's `Display` is header-free, and callers that print
    /// `"error: {err}"` (matching every other data-verb handler in
    /// `bin/tonk.rs`) would otherwise get a doubled header.
    #[error("{}", strip_clap_error_header(.0))]
    Flags(clap::Error),
    /// I/O, schema read, or repo-not-found failure.
    #[error("{0}")]
    Io(String),
}

/// Strip clap's own `"error: "` header off a rendered
/// [`clap::Error`], leaving the usage/help body intact. Used by
/// [`DataOpError::Flags`]'s `Display` impl — see that variant's doc
/// comment for why.
fn strip_clap_error_header(e: &clap::Error) -> String {
    let rendered = e.to_string();
    match rendered.strip_prefix("error: ") {
        Some(rest) => rest.to_string(),
        None => rendered,
    }
}

impl DataOpError {
    /// CLI exit code for this failure mode.
    pub fn exit_code(&self) -> crate::ExitCode {
        match self {
            DataOpError::NoConcept { .. } | DataOpError::Io(_) => crate::ExitCode::IoError,
            // A bad field/value, or a rejected flag parse, is an
            // analysis-level rejection, not an I/O failure.
            DataOpError::Data(_) | DataOpError::Flags(_) => crate::ExitCode::AnalyzeError,
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

/// Assert a new instance of `concept` from schema-derived `--field
/// value` flags in `argv`. Every non-optional field is required
/// (`parse_field_flags(.., all_required=true)`), so a `--title`-only
/// `add` on a concept with two required fields fails clap's
/// required-argument check before anything is built or committed.
///
/// A `--help` anywhere in `argv` is not an error: it returns
/// `Ok(help_text)` so the caller can print it to stdout and exit
/// successfully, mirroring `tonk add <concept> --help`'s dynamic,
/// schema-driven help. Any other flag rejection (unknown field,
/// missing required field, bad value) is
/// [`DataOpError::Flags`], whose display text is clap's own
/// rendered error — the usage line already enumerates the
/// concept's real flags.
pub async fn add(site: &TonkSite, concept: &str, argv: &[String]) -> Result<String, DataOpError> {
    let info = require_concept(site, concept).await?;
    let pairs = match flags::parse_field_flags(&info.descriptor, concept, argv, true) {
        Ok(pairs) => pairs,
        Err(e) if e.kind() == clap::error::ErrorKind::DisplayHelp => return Ok(e.to_string()),
        Err(e) => return Err(DataOpError::Flags(e)),
    };
    let doc = build_add(&info.descriptor, concept, &pairs)?;
    let outcome = eval::run_against_site(site, Source::Inline(doc), Options::default()).await?;
    Ok(format!("added {concept}\n{}", outcome.stdout))
}
