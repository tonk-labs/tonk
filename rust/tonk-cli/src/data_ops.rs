//! Library handlers for the data verbs — the testable core the thin
//! `bin/tonk.rs` handlers call. Each returns rendered stdout; the
//! binary maps errors to exit codes.

use crate::authoring::{
    AuthoringError, ViewKind, build_concept_decl, build_home_recipe, build_view_decl,
    lint_view_template, parse_attr_spec,
};
use crate::auto_sync;
use crate::data::{build_assert, build_retract, build_supersede};
use crate::eval::{self, Options, Source};
use crate::output::Format;
use crate::schema;
use crate::site::TonkSite;

pub mod flags;

/// How a write verb commits and reports.
///
/// The same three switches `tonk eval` has, on every verb that writes, so
/// they are learned once rather than per command. Default is the plain
/// write: commit, sync, and print the matched rows.
#[derive(Debug, Clone, Copy, Default)]
pub struct WriteOptions {
    /// Print the notation document and stop before evaluation.
    pub notation: bool,
    /// Build, analyze, and plan the transaction, then drop it instead of
    /// committing.
    pub dry_run: bool,
    /// Skip the automatic pull-before / push-after.
    pub no_sync: bool,
    /// Print the envelope without the matched rows.
    pub quiet: bool,
}

impl WriteOptions {
    /// The evaluator knobs these imply.
    pub(crate) fn eval(self) -> Options {
        Options {
            format: Format::Notation,
            quiet: self.quiet,
            dry_run: self.dry_run,
            home: None,
        }
    }

    /// Whether to pull before and push after.
    ///
    /// A dry run never syncs: it has no commit to push, and pulling would
    /// move the branch under a command whose whole promise is that it
    /// leaves the branch alone.
    pub(crate) fn sync(self) -> bool {
        !self.dry_run && auto_sync::enabled(self.no_sync)
    }

    /// Label a write's summary line with what actually happened.
    ///
    /// A dry run's summary is otherwise word-for-word the summary of a
    /// committed write, which is the one thing it must not be mistaken
    /// for.
    fn summarize(self, line: impl std::fmt::Display) -> String {
        if self.dry_run {
            format!(
                "dry run — nothing committed
would have {line}"
            )
        } else {
            line.to_string()
        }
    }
}

/// Failure modes shared by the data verbs (`assert`, `retract`,
/// `query`, `get`, `schema_subset`). Each maps onto a CLI exit code via
/// [`Self::exit_code`], mirroring [`crate::eval::EvalError`] and
/// [`crate::invite::InviteError`].
#[derive(Debug, thiserror::Error)]
pub enum DataOpError {
    /// No concept with this name; lists the ones this space defines.
    ///
    /// The list is the author's vocabulary, not the branch's — the
    /// runtime's forty-odd concepts are still addressable, but naming
    /// them here would bury the handful an agent might have meant.
    #[error("no concept named '{name}'; {}", describe_known(known))]
    NoConcept {
        /// The concept name that was looked up.
        name: String,
        /// Names of the concepts this space's author defined.
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
    /// A `--field` flag on `concept add` failed to parse (malformed
    /// spec, unknown type, or bad cardinality).
    #[error(transparent)]
    Authoring(#[from] crate::authoring::AuthoringError),
    /// `concept add` named a concept that already exists on the
    /// branch.
    #[error("concept '{name}' already exists; inspect it with `tonk show {name}`")]
    ConceptExists {
        /// The concept name that was already taken.
        name: String,
    },
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

/// Render the tail of [`DataOpError::NoConcept`]: the concepts this
/// space defines, or — on a space that defines none yet — the command
/// that defines the first one. "known concepts: " followed by nothing
/// reads as a listing failure rather than as an empty space.
fn describe_known(known: &[String]) -> String {
    if known.is_empty() {
        "this space defines no concepts yet; add one with \
         `tonk concept add <name> --field <field>:<type>:<cardinality>`"
            .to_string()
    } else {
        format!("concepts in this space: {}", known.join(", "))
    }
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

impl crate::Coded for DataOpError {
    /// CLI exit code for this failure mode.
    fn exit_code(&self) -> crate::ExitCode {
        match self {
            DataOpError::NoConcept { .. }
            | DataOpError::Io(_)
            | DataOpError::NoInstance { .. }
            | DataOpError::ConceptExists { .. } => crate::ExitCode::IoError,
            // A bad field/value, or a rejected flag parse, is an
            // analysis-level rejection, not an I/O failure.
            DataOpError::Data(_)
            | DataOpError::Flags(_)
            | DataOpError::NoFields
            | DataOpError::MissingRequired(_)
            | DataOpError::Authoring(_) => crate::ExitCode::AnalyzeError,
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
/// `tonk show --notation`, filtered — or the enumerating [`DataOpError::NoConcept`].
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

/// Run a read-only query and return only current instances.
async fn run_read(
    site: &TonkSite,
    doc: String,
    concept: &str,
    json: bool,
) -> Result<String, DataOpError> {
    let outcome = eval::run_against_site(site, Source::Inline(doc), Options::default()).await?;
    let format = if json { Format::Json } else { Format::Notation };
    crate::output::render_results(&outcome.response, format, concept)
        .map_err(|error| DataOpError::Io(format!("output rendering failed: {error}")))
}

/// Query every instance of `concept`, with every field bound —
/// reads are queries in dialog. Rendered as notation by default,
/// or as JSON when `json` is `true`.
pub async fn query(site: &TonkSite, concept: &str, json: bool) -> Result<String, DataOpError> {
    let info = require_concept(site, concept).await?;
    run_read(
        site,
        query_doc(&info.descriptor, concept, None),
        concept,
        json,
    )
    .await
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
        concept,
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
/// push-after; `--no-sync` or `TONK_NO_SYNC` opts out).
///
/// `--dry-run`, `--no-sync` and `--quiet` are parsed out of `argv` by the
/// concept's own dynamic command rather than declared statically, because
/// everything after `<CONCEPT>` reaches this function raw — see
/// [`flags::parse_field_flags`].
pub async fn assert_op(
    site: &TonkSite,
    concept: &str,
    entity: Option<&str>,
    argv: &[String],
) -> Result<String, DataOpError> {
    let info = require_concept(site, concept).await?;
    let all_required = entity.is_none();
    let parsed = match flags::parse_field_flags(&info.descriptor, concept, argv, all_required) {
        Ok(parsed) => parsed,
        Err(e) if e.kind() == clap::error::ErrorKind::DisplayHelp => return Ok(e.to_string()),
        Err(e) if all_required && e.kind() == clap::error::ErrorKind::MissingRequiredArgument => {
            return Err(DataOpError::MissingRequired(e));
        }
        Err(e) => return Err(DataOpError::Flags(e)),
    };
    let (pairs, write) = (parsed.pairs, parsed.write);
    match entity {
        None => {
            let doc = build_assert(&info.descriptor, concept, &pairs)?;
            if write.notation {
                return Ok(doc);
            }
            let outcome =
                auto_sync::run_eval(site, Source::Inline(doc), write.eval(), write.sync()).await?;
            Ok(format!(
                "{}\n{}",
                write.summarize(format_args!("asserted {concept}")),
                outcome.stdout
            ))
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
            if write.notation {
                return Ok(doc);
            }
            let outcome =
                auto_sync::run_eval(site, Source::Inline(doc), write.eval(), write.sync()).await?;
            let before = outcome
                .response
                .revision_before
                .as_ref()
                .map(|revision| revision.tree.to_string())
                .unwrap_or_else(|| "none".to_string());
            let after = outcome
                .response
                .revision_after
                .as_ref()
                .map(|revision| revision.tree.to_string())
                .unwrap_or_else(|| "none".to_string());
            let mut rendered = format!(
                "{}\nclaims: {}\nrevision: {before} -> {after}\n",
                write.summarize(format_args!("updated {entity}")),
                outcome.response.commits.claims
            );
            match get(site, concept, entity, false).await {
                Ok(current) => {
                    rendered.push_str("current state:\n");
                    rendered.push_str(&current);
                }
                Err(error) => {
                    rendered.push_str(&format!(
                        "the read-back failed: {error}\nverify: tonk show {concept} {entity}\n"
                    ));
                }
            }
            Ok(rendered)
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
    write: WriteOptions,
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
    if write.notation {
        return Ok(doc);
    }
    let outcome =
        auto_sync::run_eval(site, Source::Inline(doc), write.eval(), write.sync()).await?;
    let line = match field {
        Some(f) => write.summarize(format_args!("retracted {f} from {entity}")),
        None => write.summarize(format_args!("retracted {entity}")),
    };
    Ok(format!("{line}\n{}", outcome.stdout))
}

/// Author a new concept: parse every raw `--field field:type:card`
/// flag ([`parse_attr_spec`]), reject a name already on the branch
/// ([`DataOpError::ConceptExists`]), then build and commit the
/// anchored `concept!:`/`attribute!:` declaration
/// ([`build_concept_decl`]) via the same auto-sync pattern as
/// `assert_op`/`retract`. The concept and its fields resolve by
/// name immediately afterward — `tonk assert <name> --help` shows
/// the typed flags.
pub async fn concept_add(
    site: &TonkSite,
    name: &str,
    attrs: &[String],
    description: Option<&str>,
    write: WriteOptions,
) -> Result<String, DataOpError> {
    let attrs = attrs
        .iter()
        .map(|raw| parse_attr_spec(raw))
        .collect::<Result<Vec<_>, _>>()?;
    if schema::find_concept(site, name)
        .await
        .map_err(|e| DataOpError::Io(e.to_string()))?
        .is_some()
    {
        return Err(DataOpError::ConceptExists {
            name: name.to_string(),
        });
    }
    let doc = build_concept_decl(name, description, &attrs);
    if write.notation {
        return Ok(doc);
    }
    let outcome =
        auto_sync::run_eval(site, Source::Inline(doc), write.eval(), write.sync()).await?;
    Ok(format!(
        "{line}\nnext: tonk assert {name} --help\n{stdout}",
        line = write.summarize(format_args!(
            "asserted concept {name} ({n} fields)",
            n = attrs.len()
        )),
        stdout = outcome.stdout
    ))
}

/// Resolve a published bookmark name (`name!: {this: id:<name>, entity:
/// …}`) to the entity it currently points at, or `None` if the name
/// was never published. Pure query — nothing commits. Always re-reads
/// the branch rather than trusting an assertion's own echoed matches,
/// because a `name!:` assertion can print a stale pre-commit echo.
async fn resolve_name(site: &TonkSite, name: &str) -> Result<Option<String>, DataOpError> {
    let doc = format!("name:\n  this: id:{name}\n  entity: ?e\n");
    let outcome = eval::run_against_site(site, Source::Inline(doc), Options::default()).await?;
    Ok(outcome
        .response
        .matches_after
        .iter()
        .find(|block| block.label == "name")
        .and_then(|block| block.results.first())
        .and_then(|row| row.fields.get("entity"))
        .and_then(|value| value.as_str())
        .map(str::to_owned))
}

/// True when the `tonk/space` alias is either unpublished or still
/// pointing at the fresh-repo default (`tonk:blank`) — the two cases
/// where `tonk view add`'s auto-surface is safe to repoint the home
/// without clobbering something a human explicitly set.
async fn home_is_unset(site: &TonkSite) -> Result<bool, DataOpError> {
    let Some(space) = resolve_name(site, "tonk/space").await? else {
        return Ok(true);
    };
    // The alias value renders as whatever shape the claim stored: on a
    // fresh repo the core.yaml seed stores the `tonk:blank` symbol
    // itself, so match the literal first; a claim that stored the
    // resolved entity instead is covered by comparing against
    // `tonk:blank`'s own name-table resolution.
    if space == "tonk:blank" {
        return Ok(true);
    }
    let blank = resolve_name(site, "tonk:blank").await?;
    Ok(blank.as_deref() == Some(space.as_str()))
}

/// Put one or more concepts' directories on the space home: validate
/// every model exists first ([`require_concept`], so a typo'd name
/// fails before anything is asserted), then author and commit the
/// verified origin-keyed root-concept recipe
/// ([`build_home_recipe`]) that re-points the `tonk/space` alias.
/// Cardinality-one — safe to re-run; each call replaces the home
/// wholesale.
pub async fn home(
    site: &TonkSite,
    models: &[String],
    write: WriteOptions,
) -> Result<String, DataOpError> {
    for model in models {
        require_concept(site, model).await?;
    }
    let doc = build_home_recipe(models);
    if write.notation {
        return Ok(doc);
    }
    let outcome =
        auto_sync::run_eval(site, Source::Inline(doc), write.eval(), write.sync()).await?;
    let did = site.repository.did();
    Ok(format!(
        "{}\nlive at /space/{did}/\n{}",
        write.summarize(format_args!("set the home to {}", models.join(", "))),
        outcome.stdout
    ))
}

/// Author a declarative view for `concept`: a `view!:` writing
/// `template` under the kind's facet of the model's `show`
/// dictionary (cardinality one per entry, so re-authoring the same
/// facet supersedes rather than duplicates). When the space home is
/// unset (a fresh repo still showing `tonk:blank`, or nothing
/// published at all), the concept is auto-surfaced onto the home via
/// [`home`] so an agent's first view build actually lands somewhere
/// visible; an explicitly-set home is left alone.
pub async fn view_add(
    site: &TonkSite,
    model: &str,
    kind: ViewKind,
    template: &str,
    set_home: bool,
    write: WriteOptions,
) -> Result<String, DataOpError> {
    let info = require_concept(site, model).await?;
    if template.trim().is_empty() {
        return Err(AuthoringError::EmptyTemplate.into());
    }
    let fields: Vec<String> = info
        .descriptor
        .with()
        .iter()
        .map(|(field, _)| field.to_string())
        .collect();
    let lint = lint_view_template(template, &fields);
    let auto_surface = !set_home && kind.can_auto_surface() && home_is_unset(site).await?;
    let surface_home = set_home || auto_surface;
    let mut doc = build_view_decl(kind, model, template);
    if surface_home {
        doc.push('\n');
        doc.push_str(&build_home_recipe(&[model.to_string()]));
    }
    if write.notation {
        return Ok(doc);
    }
    let outcome =
        auto_sync::run_eval(site, Source::Inline(doc), write.eval(), write.sync()).await?;
    let mut out = format!(
        "{}\n",
        write.summarize(format_args!(
            "asserted the {} view of {model}",
            kind.facet()
        ))
    );
    for warning in &lint {
        out.push_str(&format!("warning: {warning}\n"));
    }
    out.push_str(&outcome.stdout);
    if surface_home {
        let did = site.repository.did();
        out.push_str(&format!(
            "\n{}\nlive at /space/{did}/\n",
            write.summarize(format_args!("set the home to {model}"))
        ));
    } else {
        out.push_str("home unchanged; use --home or `tonk space home <concept>`\n");
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[dialog_common::test]
    fn it_names_the_concepts_a_space_defines() {
        assert_eq!(
            describe_known(&["page".to_string(), "note".to_string()]),
            "concepts in this space: page, note"
        );
    }

    #[dialog_common::test]
    fn it_points_at_concept_add_when_a_space_defines_none() {
        let described = describe_known(&[]);
        assert!(
            described.starts_with("this space defines no concepts yet"),
            "{described}"
        );
        assert!(described.contains("tonk concept add"), "{described}");
    }
}
