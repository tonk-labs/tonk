//! Library handlers for the data verbs — the testable core the thin
//! `bin/tonk.rs` handlers call. Each returns rendered stdout; the
//! binary maps errors to exit codes.

use crate::auto_sync;
use crate::data::{build_assert, build_retract, build_supersede};
use crate::eval::{self, Options, Source};
use crate::output::Format;
use crate::schema;
use crate::site::TonkSite;

pub mod flags;

/// Failure modes shared by the data verbs (`assert`, `retract`,
/// `query`, `get`, `schema_subset`). Each maps onto a CLI exit code via
/// [`Self::exit_code`], mirroring [`crate::eval::EvalError`] and
/// [`crate::invite::InviteError`].
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
    /// The supersede form of `assert` named an entity that doesn't
    /// currently match the concept (every `with:` field bound).
    /// Closes the validation backdoor: without this, a typo'd
    /// entity would silently mint a partial orphan instance,
    /// bypassing the mint form's required-field check.
    #[error("no {concept} instance at '{entity}'; run `tonk query {concept}` to see what exists")]
    NoInstance {
        /// Concept the entity was checked against.
        concept: String,
        /// The entity reference that didn't resolve.
        entity: String,
    },
    /// The mint form of `assert` was missing one or more required
    /// fields. Rendered like [`DataOpError::Flags`], plus a hint
    /// for the agent who intended the supersede form and forgot
    /// the entity — clap's own message points at the wrong fix.
    #[error("{}\nto update an existing instance, pass the entity before the flags: tonk assert <concept> <entity> --<field> <value>", strip_clap_error_header(.0))]
    MissingRequired(clap::Error),
    /// A field/value error from the notation builders.
    #[error(transparent)]
    Data(#[from] crate::data::DataError),
    /// The underlying eval pipeline failed.
    #[error(transparent)]
    Eval(#[from] crate::eval::EvalError),
    /// The raw CLI flags for `assert` failed clap's
    /// dynamically-built parse: an unknown `--flag` or a bad value
    /// for the arg's type. Display text mirrors clap's own rendered
    /// error (its usage line already enumerates the concept's real
    /// flags), minus clap's own `"error: "` header — every other
    /// `DataOpError` variant's `Display` is header-free, and callers
    /// that print `"error: {err}"` (matching every other data-verb
    /// handler in `bin/tonk.rs`) would otherwise get a doubled
    /// header.
    #[error("{}", strip_clap_error_header(.0))]
    Flags(clap::Error),
    /// The supersede form of `assert` was called with no `--field
    /// value` pairs at all — asserting against an existing entity
    /// with nothing to change would commit nothing.
    #[error("assert with an entity needs at least one --field to change")]
    NoFields,
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
            DataOpError::NoConcept { .. } | DataOpError::Io(_) | DataOpError::NoInstance { .. } => {
                crate::ExitCode::IoError
            }
            // A bad field/value, or a rejected flag parse, is an
            // analysis-level rejection, not an I/O failure.
            DataOpError::Data(_)
            | DataOpError::Flags(_)
            | DataOpError::NoFields
            | DataOpError::MissingRequired(_) => crate::ExitCode::AnalyzeError,
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

/// Render one concept's schema subset — same notation as bare
/// `tonk schema`, filtered — or the enumerating [`DataOpError::NoConcept`].
/// The human field/type table this replaces lives in
/// `tonk assert <concept> --help`, where the flags are.
pub async fn schema_subset(site: &TonkSite, concept: &str) -> Result<String, DataOpError> {
    require_concept(site, concept).await?;
    match schema::render_one(site, concept).await {
        Ok(Some(text)) => Ok(text),
        Ok(None) => Err(DataOpError::Io(format!(
            "concept '{concept}' vanished between lookup and render"
        ))),
        Err(e) => Err(DataOpError::Io(e.to_string())),
    }
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

/// Query every instance of `concept`, with every field bound —
/// reads are queries in dialog. Rendered as notation by default,
/// or as JSON when `json` is `true`.
pub async fn query(site: &TonkSite, concept: &str, json: bool) -> Result<String, DataOpError> {
    let info = require_concept(site, concept).await?;
    run_read(site, query_doc(&info.descriptor, concept, None), json).await
}

/// True iff `entity` currently matches `concept` — every `with:`
/// field bound, the same completeness [`get`] requires. A partial
/// instance (a field retracted) does not count; repairing one is
/// `tonk eval` territory.
async fn instance_exists(
    site: &TonkSite,
    descriptor: &dialog_query::ConceptDescriptor,
    concept: &str,
    entity: &str,
) -> Result<bool, DataOpError> {
    let doc = query_doc(descriptor, concept, Some(entity));
    let outcome = match eval::run_against_site(site, Source::Inline(doc), Options::default()).await
    {
        Ok(outcome) => outcome,
        // `entity` is the only untrusted input in this document, so
        // any analyzer rejection of it (most commonly `this:` naming
        // a bookmark that was never declared — `UnknownNameReference`)
        // means the same thing a query with no results would: there's
        // no instance there. Any other failure (I/O, commit) still
        // propagates.
        Err(eval::EvalError::Analyze(_)) => return Ok(false),
        Err(e) => return Err(e.into()),
    };
    Ok(outcome
        .response
        .matches_after
        .iter()
        .find(|block| block.label == concept)
        .is_some_and(|block| !block.results.is_empty()))
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

/// Assert claims against `concept` — dialog's one write operation.
/// With `entity: None`, mints a new instance: every non-optional
/// field is required (`all_required=true`), so a partial mint fails
/// clap's required-argument check before anything is built. With
/// `entity: Some(_)`, asserts superseding claims on that entity:
/// every field is optional, but the entity must already match the
/// concept ([`instance_exists`]) — checked first, so a bad entity
/// surfaces as [`DataOpError::NoInstance`] instead of silently
/// minting a partial orphan. Only once that holds does at least
/// one field need to be supplied, or the call is rejected as
/// [`DataOpError::NoFields`].
///
/// A `--help` anywhere in `argv` is not an error: it returns
/// `Ok(help_text)` so the caller prints it and exits successfully —
/// the mint form renders required markers, the entity form renders
/// everything optional. A missing required field on the mint form
/// maps to [`DataOpError::MissingRequired`], whose display hints
/// the supersede form; any other flag rejection is
/// [`DataOpError::Flags`].
///
/// Commits sync to the upstream like `tonk eval` (pull-before /
/// push-after; `TONK_NO_SYNC` opts out).
pub async fn assert_op(
    site: &TonkSite,
    concept: &str,
    entity: Option<&str>,
    argv: &[String],
) -> Result<String, DataOpError> {
    let info = require_concept(site, concept).await?;
    let all_required = entity.is_none();
    let pairs = match flags::parse_field_flags(&info.descriptor, concept, argv, all_required) {
        Ok(pairs) => pairs,
        Err(e) if e.kind() == clap::error::ErrorKind::DisplayHelp => return Ok(e.to_string()),
        Err(e) if all_required && e.kind() == clap::error::ErrorKind::MissingRequiredArgument => {
            return Err(DataOpError::MissingRequired(e));
        }
        Err(e) => return Err(DataOpError::Flags(e)),
    };
    match entity {
        None => {
            let doc = build_assert(&info.descriptor, concept, &pairs)?;
            let outcome = auto_sync::run_eval(
                site,
                Source::Inline(doc),
                Options::default(),
                auto_sync::enabled(false),
            )
            .await?;
            Ok(format!("asserted {concept}\n{}", outcome.stdout))
        }
        Some(entity) => {
            if !instance_exists(site, &info.descriptor, concept, entity).await? {
                return Err(DataOpError::NoInstance {
                    concept: concept.to_string(),
                    entity: entity.to_string(),
                });
            }
            if pairs.is_empty() {
                return Err(DataOpError::NoFields);
            }
            let doc = build_supersede(&info.descriptor, concept, entity, &pairs)?;
            let outcome = auto_sync::run_eval(
                site,
                Source::Inline(doc),
                Options::default(),
                auto_sync::enabled(false),
            )
            .await?;
            Ok(format!("asserted {entity}\n{}", outcome.stdout))
        }
    }
}

/// Retract one field, or the whole instance, from `entity`. A
/// retraction is itself an assertion — a claim invalidating an old
/// one — not a deletion. With `field: Some(f)`, retracts `f`
/// (validated against `concept`'s descriptor first, enumerating the
/// valid fields on a miss); on a many-cardinality field this
/// retracts every value. With `field: None`, retracts the whole
/// instance ([`build_retract`]'s `..: _` form).
///
/// Commits sync to the upstream like `tonk eval` (pull-before /
/// push-after; `TONK_NO_SYNC` opts out).
pub async fn retract(
    site: &TonkSite,
    concept: &str,
    entity: &str,
    field: Option<&str>,
) -> Result<String, DataOpError> {
    let info = require_concept(site, concept).await?;
    if let Some(f) = field {
        let valid: Vec<String> = info
            .descriptor
            .with()
            .iter()
            .map(|(name, _)| name.to_string())
            .collect();
        if !valid.iter().any(|v| v == f) {
            return Err(crate::data::DataError::UnknownField {
                concept: concept.to_string(),
                field: f.to_string(),
                valid,
            }
            .into());
        }
    }
    let doc = build_retract(concept, entity, field);
    let outcome = auto_sync::run_eval(
        site,
        Source::Inline(doc),
        Options::default(),
        auto_sync::enabled(false),
    )
    .await?;
    Ok(match field {
        Some(f) => format!("retracted {f} from {entity}\n{}", outcome.stdout),
        None => format!("retracted {entity}\n{}", outcome.stdout),
    })
}
