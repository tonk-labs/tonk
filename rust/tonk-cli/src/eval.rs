//! `tonk eval` — read a notation document, evaluate it against
//! the local site, and render the response.

use std::path::PathBuf;

use thiserror::Error;
use tokio::io::AsyncReadExt as _;
use tonk_evaluator::evaluate::{EvaluateError, SyntaxEvaluateExt};
use tonk_notation::{Parsed, Syntax, parse};

use crate::ExitCode;
use crate::authoring::build_home_recipe;
use crate::output::{self, EvaluateResponse, Format};
use crate::site::TonkSite;

/// Where the document text comes from. Picked by the CLI front
/// end based on `-c`, the positional argument, or piped stdin.
#[derive(Debug, Clone)]
pub enum Source {
    /// Inline string from `-c "<doc>"`.
    Inline(String),
    /// File on disk — the path becomes the diagnostic source
    /// label.
    File(PathBuf),
    /// Piped stdin or `-`. Diagnostics are labelled `<stdin>`.
    Stdin,
}

impl Source {
    /// Human-friendly source label used in
    /// `<source>:<line>:<col>:` diagnostics.
    fn label(&self) -> String {
        match self {
            Source::Inline(_) => "<inline>".to_string(),
            Source::File(path) => path.display().to_string(),
            Source::Stdin => "<stdin>".to_string(),
        }
    }

    /// Read the document text — async because stdin and file IO
    /// both go through tokio.
    async fn read(&self) -> Result<String, EvalError> {
        match self {
            Source::Inline(text) => Ok(text.clone()),
            Source::File(path) => tokio::fs::read_to_string(path)
                .await
                .map_err(|e| EvalError::Io(format!("failed to read {}: {e}", path.display()))),
            Source::Stdin => {
                let mut buf = String::new();
                tokio::io::stdin()
                    .read_to_string(&mut buf)
                    .await
                    .map_err(|e| EvalError::Io(format!("failed to read stdin: {e}")))?;
                Ok(buf)
            }
        }
    }
}

/// Per-invocation knobs for `tonk eval`. Mirrors the CLI's
/// flags so the binary is a thin parser → struct-builder → call
/// shim.
#[derive(Debug, Clone)]
pub struct Options {
    /// Output format selector. Default is notation.
    pub format: Format,
    /// Suppress the matches section and emit only the envelope.
    pub quiet: bool,
    /// Run analysis + queries + planning but drop the transaction
    /// instead of committing. Mirrors the worker's `transact=false`
    /// preview: `commits.claims` is zeroed and `revision_after ==
    /// revision_before`, so the branch is left untouched.
    pub dry_run: bool,
    /// Atomically replace the home with this concept's directory.
    pub home: Option<String>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            format: Format::Notation,
            quiet: false,
            dry_run: false,
            home: None,
        }
    }
}

/// What `tonk eval` returns once successful — rendered output
/// ready for stdout, plus the structured response in case
/// callers want to inspect it (integration tests do).
#[derive(Debug)]
pub struct Outcome {
    /// Rendered notation/JSON document, ready for stdout.
    pub stdout: String,
    /// Underlying response from [`evaluate::run`].
    pub response: EvaluateResponse,
    /// `true` iff a dialog commit was attempted.
    pub committed: bool,
}

/// Failure modes for [`run_against_site`]. Each maps onto a CLI
/// exit code via [`Self::exit_code`].
#[derive(Debug, Error)]
pub enum EvalError {
    /// Source failed to parse — diagnostics are joined into the
    /// message (`<src>:<line>:<col>: <msg>` per diagnostic).
    #[error("{0}")]
    Parse(String),
    /// Document parsed but produced no expressions.
    #[error("{0}")]
    Empty(String),
    /// Analyzer rejected the document.
    #[error("{0}")]
    Analyze(String),
    /// Plan or commit failed.
    #[error("{0}")]
    Commit(String),
    /// I/O, repo-not-found, or identity failure.
    #[error("{0}")]
    Io(String),
}

impl crate::Coded for EvalError {
    /// CLI exit code for this failure mode.
    fn exit_code(&self) -> ExitCode {
        match self {
            EvalError::Parse(_) | EvalError::Empty(_) => ExitCode::ParseError,
            EvalError::Analyze(_) => ExitCode::AnalyzeError,
            EvalError::Commit(_) => ExitCode::CommitError,
            EvalError::Io(_) => ExitCode::IoError,
        }
    }
}

/// Evaluate `source` against an already-opened [`TonkSite`].
/// Lets integration tests reuse a single site across many
/// `eval` calls without paying the open cost each time.
pub async fn run_against_site(
    site: &TonkSite,
    source: Source,
    options: Options,
) -> Result<Outcome, EvalError> {
    let label = source.label();
    let mut text = source.read().await?;
    if let Some(model) = &options.home {
        text.push('\n');
        // Keep validation inside this same analyzed document: an empty query
        // binds no variables and writes nothing, but forces ordinary concept
        // resolution to accept an in-document declaration or reject an
        // unknown name before the transaction can commit the home recipe.
        text.push_str(model);
        text.push_str(":\n\n");
        text.push_str(&build_home_recipe(std::slice::from_ref(model)));
    }
    let syntax = parse_or_diagnose(&label, &text)?;

    let session = site
        .branch()
        .await
        .map_err(|e| EvalError::Io(format!("acquire branch: {e}")))?;
    let branch = session.handle();

    let revision_before = branch.revision();
    let evaluated = syntax
        .evaluate(branch.transaction())
        .perform(&site.operator)
        .await
        .map_err(map_evaluate_error)?;

    // Compute the post-evaluation match view by re-running the
    // analyzer's queries against the txn overlay. The overlay
    // reflects every applied write plus the induce pass, so this
    // is the same answer a post-commit branch query would give —
    // computed *before* commit so we don't need the branch after
    // the txn is consumed.
    let matches_after = evaluated
        .matches_after(&site.operator)
        .await
        .map_err(map_evaluate_error)?;

    // Commit only a mutating document that wasn't run as a dry
    // run. Pure-query docs and `--dry-run` short-circuit so we
    // don't pay for (or apply) a commit.
    let (response, committed) = if !options.dry_run && evaluated.analysis.analysis.has_statements()
    {
        let revision_after = evaluated
            .txn
            .commit()
            .perform(&site.operator)
            .await
            .map_err(|e| EvalError::Io(format!("commit failed: {e}")))?;
        // Re-poll the branch's subscriptions so the reactor's
        // commit contract holds. Tonk opens none, so this is a
        // no-op today, kept for parity with the worker's path.
        session.poll(&site.operator).await;
        (
            EvaluateResponse {
                revision_before,
                revision_after: Some(revision_after),
                matches_before: evaluated.matches,
                matches_after,
                commits: evaluated.commits,
            },
            true,
        )
    } else {
        // Pure-query or dry-run: drop the transaction. The
        // pre-mutation matches double as "after" because nothing
        // landed. Zero `claims` so the summary reflects what *did*
        // commit (nothing), not what *would* have — same contract
        // the worker's `transact=false` preview honors.
        let mut commits = evaluated.commits;
        commits.claims = 0;
        (
            EvaluateResponse {
                revision_before: revision_before.clone(),
                revision_after: revision_before,
                matches_before: evaluated.matches.clone(),
                matches_after: evaluated.matches,
                commits,
            },
            false,
        )
    };

    let stdout = output::render(&response, options.format, options.quiet)
        .map_err(|e| EvalError::Io(format!("output rendering failed: {e}")))?;

    Ok(Outcome {
        stdout,
        response,
        committed,
    })
}

/// Drive the parser and project diagnostics onto either a clean
/// [`Syntax`] or a parse error formatted for stderr.
fn parse_or_diagnose(source: &str, text: &str) -> Result<Syntax, EvalError> {
    let parsed = parse(text);
    surface_parse_diagnostics(source, parsed)
}

fn surface_parse_diagnostics(source: &str, parsed: Parsed) -> Result<Syntax, EvalError> {
    if !parsed.diagnostics.is_empty() {
        let messages = parsed
            .diagnostics
            .iter()
            .map(|d| format_diagnostic(source, d))
            .collect::<Vec<_>>()
            .join("\n");
        return Err(EvalError::Parse(messages));
    }
    parsed
        .syntax
        .ok_or_else(|| EvalError::Empty(format!("{source}: empty document")))
}

/// Format an LSP diagnostic as `source:line:col: message`. LSP
/// positions are 0-based; editors and tooling expect 1-based, so
/// we shift before printing.
fn format_diagnostic(source: &str, diagnostic: &lsp_types::Diagnostic) -> String {
    let line = diagnostic.range.start.line.saturating_add(1);
    let col = diagnostic.range.start.character.saturating_add(1);
    format!(
        "{source}:{line}:{col}: {message}",
        message = diagnostic.message
    )
}

fn map_evaluate_error(error: EvaluateError) -> EvalError {
    match error {
        // Tonk just renders to stderr, so flatten back to a
        // string here. The structured `code`/`range` only
        // matters for editor consumers.
        EvaluateError::Analyze(analyze_error) => EvalError::Analyze(analyze_error.to_string()),
        EvaluateError::Plan(message) => EvalError::Commit(format!("plan failed: {message}")),
        EvaluateError::Query(message) => EvalError::Commit(message),
    }
}
