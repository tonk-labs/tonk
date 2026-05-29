//! `slide` — local-only CLI for reading and writing tonk facts
//! via asserted-notation.
//!
//! The mutating verb is `eval`: it consumes a notation document
//! and runs the analyze → query → plan → commit pipeline against
//! the local `.tonk/` site. The other subcommands (`init`,
//! `identity`, `guide`, `schema`, `migrate`) are read-only or
//! one-shot setup helpers.

use std::io::{IsTerminal as _, Write as _};
use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

use slide::eval::{self, EvalError, Source};
use slide::invite::{self, ClaimOutcome, InviteOutcome};
use slide::migrate::{self, Mode as MigrateMode};
use slide::output::Format;
use slide::remote::{self, AddOutcome, RemoteRecord, UpstreamOutcome};
use slide::share::{self, ShareDisplayOutcome, ShareOptions, ShareOutcome, ShareViewOutcome};
use slide::sync::{self, SyncOutcome};
use slide::views::{self, ViewSummary};
use slide::{ExitCode, guide, identity, schema, site};

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

    /// Print the asserted-notation guide. Useful for agent harnesses
    /// that need to learn the syntax without repo access.
    Guide,

    /// Print the current site's schema (every named attribute and
    /// concept) as a re-submittable notation document.
    Schema,

    /// List user-defined concepts on the local branch. One row
    /// per concept, tab-separated `name<TAB>description`. Built-in
    /// concepts (`attribute`, `concept`, …) are omitted — they're
    /// resolvable everywhere and would just be noise.
    Concepts,

    /// List entities that carry a `text/html` claim on the local
    /// branch. One row per entity, tab-separated
    /// `name<TAB>entity<TAB>bytes`. Claim-driven: surfaces
    /// anything the host route would serve, regardless of how
    /// the claim was asserted.
    Views,

    /// Migrate a `.carry/` directory to `.tonk/`. Walks up from
    /// `$PWD` to find the source unless `--from` is supplied; the
    /// destination is always a sibling `.tonk/` of the source.
    Migrate {
        /// Explicit source `.carry/` directory. Default: walk up
        /// from `$PWD`.
        #[arg(long, value_name = "PATH")]
        from: Option<PathBuf>,

        /// Move instead of copy. Atomic rename on the same
        /// filesystem; copy + delete fallback otherwise.
        #[arg(long = "move")]
        do_move: bool,
    },

    /// Push the local main branch to its upstream.
    Push,

    /// Pull the local main branch from its upstream.
    Pull,

    /// Mint a UCAN delegation chain over the local repo and
    /// print a paste-able invite URL. The default form is
    /// audience-open: anyone holding the URL can claim by
    /// redelegating from the embedded ephemeral key.
    Invite {
        /// Override the URL prefix the invite is built against.
        #[arg(long, value_name = "URL", default_value_t = tonk_invite::DEFAULT_BASE_URL.to_string())]
        base_url: String,

        /// Embed a registered remote's URL in the invite so
        /// the claimer auto-configures the same access service
        /// after redeeming. Argument is the remote's local
        /// name (as registered with `slide remote add`).
        #[arg(long, value_name = "NAME")]
        remote: Option<String>,
    },

    /// Claim an invite URL into a fresh `.tonk/` under the
    /// current directory. Refuses if a site already exists.
    Join {
        /// Invite URL produced by `slide invite` or
        /// tonk-ui's invite flow.
        #[arg(value_name = "URL")]
        url: String,
    },

    /// Manage remotes attached to the local repository.
    Remote {
        #[command(subcommand)]
        command: RemoteCommand,
    },

    /// Push to the upstream and produce a launcher URL that
    /// lands the recipient on a live view of local data.
    Share {
        #[command(subcommand)]
        command: ShareCommand,
    },
}

#[derive(Subcommand, Debug)]
enum ShareCommand {
    /// Share a named concept. The recipient lands on the
    /// auto-rendered concept view at
    /// `/space/<space-name>/branch/main/concept/<name>`.
    Concept {
        /// Local name of the concept to share.
        #[arg(value_name = "NAME")]
        name: String,
        /// Override the URL prefix the launcher is built against.
        #[arg(long, value_name = "URL", default_value_t = tonk_invite::DEFAULT_BASE_URL.to_string())]
        ui_base: String,
        /// Suggested local name for the recipient's space —
        /// pre-fills the join form's "Local name" input. The
        /// recipient can rename before joining.
        #[arg(long, value_name = "NAME")]
        space_name: Option<String>,
        /// Embed an explicit remote's endpoint as the invite's
        /// `remote=` parameter. Defaults to the only registered
        /// remote when there's exactly one.
        #[arg(long, value_name = "NAME")]
        remote: Option<String>,
    },

    /// Share an HTML view. The recipient lands on the iframe
    /// viewer at `/space/<space-name>/branch/main/view/<entity>`
    /// with the body served from the entity's `text/html` claim.
    View {
        /// Bookmark name or `did:key:…` entity URI for the view.
        /// `slide views` lists what's available.
        #[arg(value_name = "NAME_OR_ENTITY")]
        target: String,
        /// Override the URL prefix the launcher is built against.
        #[arg(long, value_name = "URL", default_value_t = tonk_invite::DEFAULT_BASE_URL.to_string())]
        ui_base: String,
        /// Suggested local name for the recipient's space.
        #[arg(long, value_name = "NAME")]
        space_name: Option<String>,
        /// Embed an explicit remote's endpoint.
        #[arg(long, value_name = "NAME")]
        remote: Option<String>,
    },

    /// Share an entity rendered through `<tonk-display>`. The
    /// recipient lands on
    /// `/space/<space-name>/branch/main/display/<subject>` with
    /// the supplied `--view` carried across as a query parameter.
    /// Use this for declarative views built against the `view`
    /// concept (`{model, display}`), identified by their anchor
    /// name — `share view` is for the iframe viewer.
    Display {
        /// Bookmark name or `did:key:…` entity URI for the
        /// entity to render. Names survive entity-URI changes
        /// (re-asserting a view body) so they're usually the
        /// better choice.
        #[arg(value_name = "NAME_OR_ENTITY")]
        subject: String,
        /// The view's anchor name (the `&name` on its `view!:`),
        /// forwarded as `?view=`. `<tonk-display>` resolves it to
        /// the view entity the name points at and reads that
        /// view's own `model`. Omit it for carousel mode (every
        /// view published for `--model`). Mutually exclusive with
        /// `--model`: a named view declares its own model.
        #[arg(long, value_name = "NAME", conflicts_with = "model")]
        view: Option<String>,
        /// Concept name (validated locally) or URI for carousel
        /// mode, forwarded as `?model=`. Not needed with `--view`
        /// (the view declares its own model); required when
        /// `--view` is omitted.
        #[arg(long, value_name = "CONCEPT", required_unless_present = "view")]
        model: Option<String>,
        /// Override the URL prefix the launcher is built against.
        #[arg(long, value_name = "URL", default_value_t = tonk_invite::DEFAULT_BASE_URL.to_string())]
        ui_base: String,
        /// Suggested local name for the recipient's space.
        #[arg(long, value_name = "NAME")]
        space_name: Option<String>,
        /// Embed an explicit remote's endpoint.
        #[arg(long, value_name = "NAME")]
        remote: Option<String>,
    },
}

#[derive(Subcommand, Debug)]
enum RemoteCommand {
    /// Register a UCAN-S3 access-service remote on the local
    /// site. Writes the dialog remote handle and the
    /// meta-branch `Remote` concept browsers read.
    Add {
        /// Local name for the remote.
        #[arg(value_name = "NAME")]
        name: String,
        /// UCAN access-service endpoint URL.
        #[arg(value_name = "URL")]
        url: String,
        /// Override the remote's subject DID. Defaults to the
        /// local repository's DID — matches the worker's
        /// convention.
        #[arg(long, value_name = "DID")]
        subject: Option<String>,
    },

    /// Print every remote registered on the meta branch.
    List,

    /// Wire the local `main` branch's upstream to
    /// `<remote>/main`.
    SetUpstream {
        /// Name of the remote to track.
        #[arg(value_name = "REMOTE")]
        remote: String,
    },
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
        Command::Guide => print_guide(),
        Command::Schema => print_schema().await,
        Command::Concepts => print_concepts().await,
        Command::Views => print_views().await,
        Command::Migrate { from, do_move } => migrate(from, do_move).await,
        Command::Push => sync_op(SyncOp::Push).await,
        Command::Pull => sync_op(SyncOp::Pull).await,
        Command::Invite { base_url, remote } => mint_invite(base_url, remote).await,
        Command::Join { url } => claim_invite(url).await,
        Command::Remote { command } => remote_op(command).await,
        Command::Share { command } => share_op(command).await,
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

fn print_guide() -> ExitCode {
    let mut stdout = std::io::stdout().lock();
    if let Err(e) = stdout.write_all(guide::GUIDE.as_bytes()) {
        return print_error(format!("failed to write stdout: {e}"));
    }
    ExitCode::Success
}

/// Selector for the [`sync_op`] handler. Both `slide push` and
/// `slide pull` follow the same site-discovery + dispatch path;
/// the only thing that differs is which dialog primitive they
/// call and the verb they print on success.
#[derive(Debug, Clone, Copy)]
enum SyncOp {
    Push,
    Pull,
}

async fn sync_op(op: SyncOp) -> ExitCode {
    let cwd = match std::env::current_dir() {
        Ok(p) => p,
        Err(e) => return print_error(format!("could not determine current directory: {e}")),
    };
    let site = match site::SlideSite::discover_and_open(&cwd).await {
        Ok(s) => s,
        Err(err) => return print_error(err.to_string()),
    };

    let result = match op {
        SyncOp::Push => sync::push(&site).await,
        SyncOp::Pull => sync::pull(&site).await,
    };

    match result {
        Ok(outcome) => {
            print_sync_outcome(op, &outcome);
            ExitCode::Success
        }
        Err(err) => {
            eprintln!("error: {err}");
            err.exit_code()
        }
    }
}

fn print_sync_outcome(op: SyncOp, outcome: &SyncOutcome) {
    let verb = match op {
        SyncOp::Push if outcome.advanced => "Pushed",
        SyncOp::Push => "Nothing to push",
        SyncOp::Pull if outcome.advanced => "Pulled",
        SyncOp::Pull => "Already up to date",
    };
    if outcome.advanced {
        println!(
            "{verb}\nbefore: {before}\nafter:  {after}",
            before = render_revision(outcome.before.as_ref()),
            after = render_revision(outcome.after.as_ref()),
        );
    } else {
        println!("{verb}");
    }
}

fn render_revision(revision: Option<&dialog_repository::Revision>) -> String {
    match revision {
        Some(rev) => rev.tree.to_string(),
        None => "~".to_string(),
    }
}

async fn remote_op(command: RemoteCommand) -> ExitCode {
    let cwd = match std::env::current_dir() {
        Ok(p) => p,
        Err(e) => return print_error(format!("could not determine current directory: {e}")),
    };
    let site = match site::SlideSite::discover_and_open(&cwd).await {
        Ok(s) => s,
        Err(err) => return print_error(err.to_string()),
    };

    match command {
        RemoteCommand::Add { name, url, subject } => {
            let subject = match subject.as_deref() {
                Some(raw) => match raw.parse() {
                    Ok(did) => Some(did),
                    Err(e) => return print_error(format!("invalid --subject DID '{raw}': {e:?}")),
                },
                None => None,
            };
            match remote::add(&site, &name, &url, subject).await {
                Ok(outcome) => {
                    print_remote_add_outcome(&outcome);
                    ExitCode::Success
                }
                Err(err) => {
                    eprintln!("error: {err}");
                    err.exit_code()
                }
            }
        }
        RemoteCommand::List => match remote::list(&site).await {
            Ok(records) => {
                print_remote_list(&records);
                ExitCode::Success
            }
            Err(err) => {
                eprintln!("error: {err}");
                err.exit_code()
            }
        },
        RemoteCommand::SetUpstream { remote: name } => {
            match remote::set_upstream(&site, &name).await {
                Ok(outcome) => {
                    print_set_upstream_outcome(&outcome);
                    ExitCode::Success
                }
                Err(err) => {
                    eprintln!("error: {err}");
                    err.exit_code()
                }
            }
        }
    }
}

fn print_remote_add_outcome(outcome: &AddOutcome) {
    println!("Added remote '{name}'", name = outcome.name);
    println!("  endpoint: {}", outcome.endpoint);
    println!("  subject:  {}", outcome.subject);
}

fn print_remote_list(records: &[RemoteRecord]) {
    if records.is_empty() {
        println!("(no remotes registered)");
        return;
    }
    for record in records {
        println!(
            "{name}\t{endpoint}\t{subject}",
            name = record.name,
            endpoint = record.endpoint,
            subject = record.subject,
        );
    }
}

async fn share_op(command: ShareCommand) -> ExitCode {
    let cwd = match std::env::current_dir() {
        Ok(p) => p,
        Err(e) => return print_error(format!("could not determine current directory: {e}")),
    };
    let site = match site::SlideSite::discover_and_open(&cwd).await {
        Ok(s) => s,
        Err(err) => return print_error(err.to_string()),
    };

    match command {
        ShareCommand::Concept {
            name,
            ui_base,
            space_name,
            remote,
        } => {
            let options = ShareOptions {
                ui_base: Some(ui_base),
                remote,
                space_name,
            };
            match share::share_concept(&site, &name, options).await {
                Ok(outcome) => {
                    print_share_outcome(&outcome);
                    ExitCode::Success
                }
                Err(err) => {
                    eprintln!("error: {err}");
                    err.exit_code()
                }
            }
        }
        ShareCommand::View {
            target,
            ui_base,
            space_name,
            remote,
        } => {
            let options = ShareOptions {
                ui_base: Some(ui_base),
                remote,
                space_name,
            };
            match share::share_view(&site, &target, options).await {
                Ok(outcome) => {
                    print_share_view_outcome(&outcome);
                    ExitCode::Success
                }
                Err(err) => {
                    eprintln!("error: {err}");
                    err.exit_code()
                }
            }
        }
        ShareCommand::Display {
            subject,
            view,
            model,
            ui_base,
            space_name,
            remote,
        } => {
            let options = ShareOptions {
                ui_base: Some(ui_base),
                remote,
                space_name,
            };
            match share::share_display(&site, &subject, view.as_deref(), model.as_deref(), options)
                .await
            {
                Ok(outcome) => {
                    print_share_display_outcome(&outcome);
                    ExitCode::Success
                }
                Err(err) => {
                    eprintln!("error: {err}");
                    err.exit_code()
                }
            }
        }
    }
}

fn print_share_outcome(outcome: &ShareOutcome) {
    println!("{}", outcome.url);
    eprintln!("concept: {}", outcome.concept_name);
    eprintln!("space:   {}", outcome.space_name);
    eprintln!(
        "remote:  {} -> {}",
        outcome.remote_name, outcome.remote_endpoint,
    );
}

fn print_share_view_outcome(outcome: &ShareViewOutcome) {
    println!("{}", outcome.url);
    if let Some(name) = &outcome.view_name {
        eprintln!("view:    {} ({})", name, outcome.entity);
    } else {
        eprintln!("view:    {}", outcome.entity);
    }
    eprintln!("space:   {}", outcome.space_name);
    eprintln!(
        "remote:  {} -> {}",
        outcome.remote_name, outcome.remote_endpoint,
    );
}

fn print_share_display_outcome(outcome: &ShareDisplayOutcome) {
    println!("{}", outcome.url);
    if let Some(name) = &outcome.subject_name {
        eprintln!("subject: {} ({})", name, outcome.subject_entity);
    } else {
        eprintln!("subject: {}", outcome.subject_entity);
    }
    if let Some(view) = &outcome.view_name {
        eprintln!("view:    {}", view);
    }
    if let Some(model) = &outcome.model {
        eprintln!("model:   {}", model);
    }
    eprintln!("space:   {}", outcome.space_name);
    eprintln!(
        "remote:  {} -> {}",
        outcome.remote_name, outcome.remote_endpoint,
    );
}

fn print_set_upstream_outcome(outcome: &UpstreamOutcome) {
    println!(
        "Set upstream: {local} -> {remote}/{remote_branch}",
        local = outcome.local_branch,
        remote = outcome.remote,
        remote_branch = outcome.remote_branch,
    );
}

async fn mint_invite(base_url: String, remote_name: Option<String>) -> ExitCode {
    let cwd = match std::env::current_dir() {
        Ok(p) => p,
        Err(e) => return print_error(format!("could not determine current directory: {e}")),
    };
    let site = match site::SlideSite::discover_and_open(&cwd).await {
        Ok(s) => s,
        Err(err) => return print_error(err.to_string()),
    };

    // Resolve `--remote <name>` to its endpoint URL by reading
    // the meta branch. An unknown name surfaces as a friendly
    // error before any keys are generated.
    let remote_url = match remote_name.as_deref() {
        Some(name) => match remote::find(&site, name).await {
            Ok(Some(record)) => Some(record.endpoint),
            Ok(None) => {
                return print_error(format!(
                    "no remote registered as '{name}'; run `slide remote list` to see what's there"
                ));
            }
            Err(err) => return print_error(err.to_string()),
        },
        None => None,
    };

    match invite::mint(&site, Some(&base_url), remote_url.as_deref()).await {
        Ok(outcome) => {
            print_invite_outcome(&outcome);
            ExitCode::Success
        }
        Err(err) => {
            eprintln!("error: {err}");
            err.exit_code()
        }
    }
}

fn print_invite_outcome(outcome: &InviteOutcome) {
    println!("{url}", url = outcome.url);
    eprintln!("subject:  {}", outcome.subject);
    eprintln!("audience: {} (ephemeral)", outcome.audience);
}

async fn claim_invite(url: String) -> ExitCode {
    let cwd = match std::env::current_dir() {
        Ok(p) => p,
        Err(e) => return print_error(format!("could not determine current directory: {e}")),
    };
    // Use the same default site config slide init writes against,
    // so the joined site picks up the user's normal profile.
    match invite::claim(&cwd, &url, site::default_config()).await {
        Ok(outcome) => {
            print_claim_outcome(&cwd, &outcome);
            ExitCode::Success
        }
        Err(err) => {
            eprintln!("error: {err}");
            err.exit_code()
        }
    }
}

fn print_claim_outcome(parent: &std::path::Path, outcome: &ClaimOutcome) {
    println!("Joined .tonk in {}", parent.display());
    println!("subject: {}", outcome.subject);
    if let Some(name) = &outcome.auto_configured_remote
        && let Some(url) = &outcome.remote_url
    {
        println!("remote:  {name} -> {url}");
    }
}

async fn migrate(from: Option<PathBuf>, do_move: bool) -> ExitCode {
    let cwd = match std::env::current_dir() {
        Ok(p) => p,
        Err(e) => return print_error(format!("could not determine current directory: {e}")),
    };
    let mode = if do_move {
        MigrateMode::Move
    } else {
        MigrateMode::Copy
    };
    match migrate::run(&cwd, from.as_deref(), mode).await {
        Ok(outcome) => {
            let verb = if outcome.moved { "Moved" } else { "Copied" };
            println!(
                "{verb} {} -> {}",
                outcome.source.display(),
                outcome.destination.display()
            );
            println!("DID: {}", outcome.repo_did);
            println!(
                "note: any sync remotes from carry's meta branch are preserved on disk; \
                 slide doesn't read them yet."
            );
            ExitCode::Success
        }
        Err(err) => print_error(err.to_string()),
    }
}

async fn print_concepts() -> ExitCode {
    let cwd = match std::env::current_dir() {
        Ok(p) => p,
        Err(e) => return print_error(format!("could not determine current directory: {e}")),
    };
    let site = match site::SlideSite::discover_and_open(&cwd).await {
        Ok(s) => s,
        Err(err) => return print_error(err.to_string()),
    };
    let concepts = match schema::list_concepts(&site).await {
        Ok(c) => c,
        Err(err) => return print_error(err.to_string()),
    };
    let mut stdout = std::io::stdout().lock();
    for concept in &concepts {
        let description = concept.description.as_deref().unwrap_or("");
        if let Err(e) = writeln!(stdout, "{}\t{}", concept.name, description) {
            return print_error(format!("failed to write stdout: {e}"));
        }
    }
    ExitCode::Success
}

async fn print_views() -> ExitCode {
    let cwd = match std::env::current_dir() {
        Ok(p) => p,
        Err(e) => return print_error(format!("could not determine current directory: {e}")),
    };
    let site = match site::SlideSite::discover_and_open(&cwd).await {
        Ok(s) => s,
        Err(err) => return print_error(err.to_string()),
    };
    let listed = match views::list(&site).await {
        Ok(v) => v,
        Err(err) => return print_error(err.to_string()),
    };
    let mut stdout = std::io::stdout().lock();
    for row in &listed {
        let result = print_view_row(&mut stdout, row);
        if let Err(e) = result {
            return print_error(format!("failed to write stdout: {e}"));
        }
    }
    ExitCode::Success
}

fn print_view_row(out: &mut impl std::io::Write, row: &ViewSummary) -> std::io::Result<()> {
    let name = row.name.as_deref().unwrap_or("-");
    writeln!(out, "{}\t{}\t{}", name, row.entity, row.body_bytes)
}

async fn print_schema() -> ExitCode {
    let cwd = match std::env::current_dir() {
        Ok(p) => p,
        Err(e) => return print_error(format!("could not determine current directory: {e}")),
    };
    let site = match site::SlideSite::discover_and_open(&cwd).await {
        Ok(s) => s,
        Err(err) => return print_error(err.to_string()),
    };
    match schema::render(&site).await {
        Ok(text) => {
            let mut stdout = std::io::stdout().lock();
            if let Err(e) = stdout.write_all(text.as_bytes()) {
                return print_error(format!("failed to write stdout: {e}"));
            }
            ExitCode::Success
        }
        Err(err) => print_error(err.to_string()),
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
