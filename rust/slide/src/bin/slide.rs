//! `slide` — local-only CLI for reading and writing tonk facts
//! via asserted-notation.
//!
//! Three subcommands at v0: `init`, `identity`, `eval`. The
//! mutating verb is `eval`, which consumes a notation document
//! and runs the analyze → query → plan → commit pipeline against
//! the local `.tonk/` site. Everything else is plumbing.

use std::io::{IsTerminal as _, Write as _};
use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

use slide::eval::{self, EvalError, Source};
use slide::output::Format;
use slide::{ExitCode, identity, site};

#[derive(Parser, Debug)]
#[command(
    name = "slide",
    about = "Headless CLI for reading/writing data and views",
    version,
    propagate_version = true
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Initialize a new repo in the current working directory
    Init {
        /// Optional label for the repository.
        ///
        /// Reserved for a future `dialog.meta/name` claim;
        /// accepted but not yet persisted.
        #[arg(value_name = "LABEL")]
        label: Option<String>,
    },

    /// Show the local profile DID. With `--reset`, deletes the
    /// on-disk profile and creates a fresh identity.
    Identity {
        /// Wipe the on-disk profile and create a new one. This removes
        /// access to exisitng repos without re-delegation.
        #[arg(long)]
        reset: bool,
    },

    /// Evaluate commands in the current repo
    Eval(EvalArgs),
}

#[derive(Args, Debug)]
struct EvalArgs {
    /// Inline document. Mutually exclusive with the positional
    /// path / `-`.
    #[arg(short = 'c', long = "command", value_name = "DOC")]
    command: Option<String>,

    /// Output format. Default `notation`.
    #[arg(long = "format", value_name = "FORMAT", default_value = "notation")]
    format: FormatArg,

    /// Suppress the matches section; emit only the envelope
    /// (notation) or the structured commits-only response (JSON).
    #[arg(short = 'q', long = "quiet")]
    quiet: bool,

    /// Path to a notation document, or `-` to read from stdin.
    /// Omit to read from a piped stdin.
    #[arg(value_name = "PATH")]
    path: Option<String>,
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
enum FormatArg {
    Notation,
    Json,
}

impl From<FormatArg> for Format {
    fn from(value: FormatArg) -> Self {
        match value {
            FormatArg::Notation => Format::Notation,
            FormatArg::Json => Format::Json,
        }
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let cli = Cli::parse();
    let exit = match cli.command {
        Command::Init { label } => init(label).await,
        Command::Identity { reset } => identity(reset).await,
        Command::Eval(args) => eval(args).await,
    };
    std::process::exit(exit.into_raw());
}

async fn init(label: Option<String>) -> ExitCode {
    let cwd = match std::env::current_dir() {
        Ok(p) => p,
        Err(e) => return print_error(format!("could not determine current directory: {e}")),
    };

    match site::SlideSite::init(&cwd).await {
        Ok(site) => {
            println!("Initialized .tonk in {}", cwd.display());
            println!("DID: {}", site.repository.did());
            if let Some(label) = label {
                eprintln!(
                    "note: label '{label}' accepted but not yet persisted (reserved for a future dialog.meta/name claim)"
                );
            }
            ExitCode::Success
        }
        Err(err) => print_error(err.to_string()),
    }
}

async fn identity(reset: bool) -> ExitCode {
    let result = if reset {
        identity::reset().await
    } else {
        identity::open().await
    };
    match result {
        Ok(profile) => {
            println!("did: {}", profile.did());
            ExitCode::Success
        }
        Err(err) => print_error(err.to_string()),
    }
}

async fn eval(args: EvalArgs) -> ExitCode {
    let source = match resolve_source(&args) {
        Ok(s) => s,
        Err(message) => return print_error(message),
    };

    let cwd = match std::env::current_dir() {
        Ok(p) => p,
        Err(e) => return print_error(format!("could not determine current directory: {e}")),
    };

    let options = eval::Options {
        format: args.format.into(),
        quiet: args.quiet,
    };

    match eval::run_against_cwd(&cwd, source, options).await {
        Ok(outcome) => {
            let mut stdout = std::io::stdout().lock();
            if let Err(e) = stdout.write_all(outcome.stdout.as_bytes()) {
                return print_error(format!("failed to write stdout: {e}"));
            }
            ExitCode::Success
        }
        Err(err) => {
            eprintln!("{}", err);
            err.exit_code()
        }
    }
}

/// Resolve `-c`, positional path, and stdin into one [`Source`].
///
/// `-c` takes precedence and is incompatible with the positional
/// path (clap doesn't forbid this combination at the schema level
/// because `-c` is a flag and the path is a value, so we check
/// here). A bare `-` positional means stdin; an absent positional
/// means read stdin only when it's piped.
fn resolve_source(args: &EvalArgs) -> Result<Source, String> {
    if let Some(text) = &args.command {
        if args.path.is_some() {
            return Err("`-c` cannot be combined with a path argument".to_owned());
        }
        return Ok(Source::Inline(text.clone()));
    }

    match &args.path {
        Some(path) if path == "-" => Ok(Source::Stdin),
        Some(path) => Ok(Source::File(PathBuf::from(path))),
        None => {
            // Reading from a tty would block forever — surface a
            // helpful error instead.
            if std::io::stdin().is_terminal() {
                Err("no document supplied: pass `-c <doc>`, a file path, or pipe stdin".to_owned())
            } else {
                Ok(Source::Stdin)
            }
        }
    }
}

fn print_error(message: impl Into<String>) -> ExitCode {
    eprintln!("error: {}", message.into());
    ExitCode::IoError
}

/// Specialized [`print_error`] for parse-error mapping. Kept
/// alongside the others so future lint runs notice if [`EvalError`]
/// gains variants without an exit-code mapping.
#[allow(dead_code)]
fn classify(err: &EvalError) -> ExitCode {
    err.exit_code()
}
