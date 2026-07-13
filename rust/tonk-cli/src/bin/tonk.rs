//! `tonk` — local-only CLI for reading and writing tonk facts
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

use tonk_cli::auto_sync;
use tonk_cli::blob::{self, AddOutcome as BlobAddOutcome};
use tonk_cli::data_ops;
use tonk_cli::eval::{self, EvalError, Source};
use tonk_cli::invite::{self, ClaimOutcome, InviteOutcome};
use tonk_cli::migrate::{self, Mode as MigrateMode};
use tonk_cli::output::Format;
use tonk_cli::remote::{self, AddOutcome, RemoteRecord, UpstreamOutcome};
use tonk_cli::render::{self, RenderRoute};
use tonk_cli::share::{self, ShareDisplayOutcome, ShareOptions, ShareOutcome, ShareViewOutcome};
use tonk_cli::sync::{self, SyncOutcome};
use tonk_cli::transfer;
use tonk_cli::views::{self, ViewSummary};
use tonk_cli::{ExitCode, guide, identity, schema, site};

#[derive(Parser, Debug)]
#[command(
    name = "tonk",
    about = "Headless CLI for a datalog-flavoured, syncable fact store: define concepts, assert facts, query them, render views",
    version,
    propagate_version = true,
    after_help = "The loop: orient, define concepts, assert facts, give them a view, share.\n\n  orient   guide · schema · concepts · views · status\n  author   concept add · view add · home\n  data     assert · query · get · retract\n  power    eval (asserted-notation) · render\n  collab   share · invite · join · push · pull · remote\n  setup    init · identity · blob · export · import · migrate · telemetry\n\nStart with `tonk guide`; every command's --help carries examples."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    // -- orient -------------------------------------------------------
    /// Print the built-in agent reference (the index, or one topic)
    ///
    /// With no topic, prints a one-screen index; `tonk guide <topic>`
    /// prints one section; `tonk guide all` prints everything. Useful
    /// for agent harnesses that need to learn the syntax without repo
    /// access.
    // Topic list here is hand-rolled for help text; keep in sync with `guide::TOPICS`.
    #[command(
        after_help = "Topics: notation, views, events, workspace, all\n\nExamples:\n  tonk guide\n  tonk guide notation\n  tonk guide views\n  tonk guide all"
    )]
    Guide {
        /// One of: notation, views, events, workspace, all. Omit for
        /// the index.
        #[arg(value_name = "TOPIC")]
        topic: Option<String>,
    },

    /// Print the branch's schema as re-submittable notation
    ///
    /// Every named attribute and concept, or just one concept's
    /// subset when `<CONCEPT>` is given. The human field/type view
    /// lives in `tonk assert <concept> --help`.
    #[command(
        after_help = "Examples:\n  tonk schema\n  tonk schema task\n  tonk schema > schema.notation"
    )]
    Schema {
        /// Optional concept name — emit only that concept's
        /// `concept!:` block plus the `attribute!:` declarations
        /// it references.
        #[arg(value_name = "CONCEPT")]
        concept: Option<String>,
    },

    /// List user-defined concepts on the branch
    ///
    /// One row per concept, tab-separated `name<TAB>description`.
    /// Built-in concepts (`attribute`, `concept`, …) are omitted —
    /// they're resolvable everywhere and would just be noise.
    #[command(after_help = "Examples:\n  tonk concepts")]
    Concepts,

    /// List renderable entities (those carrying a text/html claim)
    ///
    /// One row per entity, tab-separated `name<TAB>entity<TAB>bytes`.
    /// Claim-driven: surfaces anything the host route would serve,
    /// regardless of how the claim was asserted.
    #[command(after_help = "Examples:\n  tonk views")]
    Views,

    /// Report how local main relates to its upstream
    ///
    /// Prints `synced`, `ahead`, `behind`, `diverged`, or
    /// `no-upstream`. Read-only — fetches the upstream head without
    /// merging.
    #[command(after_help = "Examples:\n  tonk status")]
    Status,

    // -- author -------------------------------------------------------
    /// Define a concept (schema) with typed attributes
    Concept {
        #[command(subcommand)]
        command: ConceptCommand,
    },

    /// Author a declarative HTML view for a concept
    View {
        #[command(subcommand)]
        command: ViewCommand,
    },

    /// Pin one or more concepts' directories on the space home
    ///
    /// Authors the origin-keyed root-concept recipe and re-points
    /// the `tonk/space` alias (cardinality-one — safe to re-run;
    /// each run replaces the home wholesale).
    #[command(after_help = "Examples:\n  tonk home habit\n  tonk home habit entry")]
    Home {
        /// Concept name(s) to surface, in order.
        #[arg(value_name = "CONCEPT", required = true)]
        models: Vec<String>,
    },

    // -- data ---------------------------------------------------------
    /// Write facts: mint an instance, or supersede fields on one
    ///
    /// With no entity, mints a new instance of the concept (every
    /// non-optional field required); with an entity, asserts
    /// superseding claims on it (only the named fields change, and
    /// the entity must already match the concept). The flags after
    /// `<CONCEPT>` are built at runtime from the concept's own
    /// schema — run `tonk assert <concept> --help` to see them.
    // `--help` is deliberately NOT handled by clap here
    // (`disable_help_flag`): with clap's automatic `-h`/`--help`
    // left on, it would intercept a trailing `--help` before it
    // ever reached `rest`, so `tonk assert task --help` would show
    // this static text instead of `task`'s real flags. Disabling
    // it routes any `--help`/`-h` after `<CONCEPT>` into `rest`,
    // where `data_ops::assert_op` builds the concept's own dynamic
    // `clap::Command` and renders its help instead.
    #[command(
        disable_help_flag = true,
        after_help = "Examples:\n  tonk assert task --title \"Write the plan\" --done false\n  tonk assert task <entity> --done true\n  tonk assert task --help"
    )]
    Assert {
        /// Name of the concept to assert against.
        #[arg(value_name = "CONCEPT")]
        concept: String,
        /// Optional entity (a leading non-flag token selects the
        /// supersede form) followed by schema-derived `--field
        /// value` flags, captured raw (including a bare `--help`)
        /// so the dynamic per-concept parser — not clap's static
        /// subcommand parser — decides how to handle them.
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        rest: Vec<String>,
    },

    /// Read every instance of a concept, every field bound
    ///
    /// Reads are queries in dialog. Read-only; nothing commits.
    /// Filter flags (e.g. `--where`) are the intended future
    /// direction; today the whole concept is returned.
    #[command(after_help = "Examples:\n  tonk query task\n  tonk query task --json")]
    Query {
        /// Name of the concept to query.
        #[arg(value_name = "CONCEPT")]
        concept: String,
        /// Emit `EvaluateResponse` as pretty JSON instead of notation.
        #[arg(long)]
        json: bool,
    },

    /// Read one instance of a concept by entity
    ///
    /// Every field bound. Read-only — the query commits nothing.
    #[command(after_help = "Examples:\n  tonk get task alice\n  tonk get task alice --json")]
    Get {
        /// Name of the concept to fetch from.
        #[arg(value_name = "CONCEPT")]
        concept: String,
        /// Bookmark name or `did:key:…` entity URI of the instance.
        #[arg(value_name = "ENTITY")]
        entity: String,
        /// Emit `EvaluateResponse` as pretty JSON instead of notation.
        #[arg(long)]
        json: bool,
    },

    /// Retract a field, or a whole instance
    ///
    /// A retraction is itself an assertion — a claim invalidating an
    /// old one — not a deletion. Omit `--field` to retract the whole
    /// instance; on a many-cardinality field, `--field` retracts
    /// every value (value-level retraction is not yet surfaced).
    #[command(
        after_help = "Examples:\n  tonk retract task alice --field done\n  tonk retract task alice"
    )]
    Retract {
        /// Name of the concept the instance belongs to.
        #[arg(value_name = "CONCEPT")]
        concept: String,
        /// Bookmark name or `did:key:…` entity URI of the instance.
        #[arg(value_name = "ENTITY")]
        entity: String,
        /// Retract just this field instead of the whole instance.
        #[arg(long)]
        field: Option<String>,
    },

    // -- power --------------------------------------------------------
    /// Evaluate an asserted-notation document (the full DSL)
    ///
    /// The escape hatch for anything the verbs don't cover: rules,
    /// multi-statement documents, joins, retractions inside
    /// assertions. `tonk guide notation` documents the grammar.
    Eval(EvalArgs),

    /// Render a view to HTML, headlessly
    ///
    /// Route grammar: `{model}` (directory), `{entity}@{model}`
    /// (one entity), `{entity}@{model}!{view}` (explicit view).
    /// Writes HTML to stdout unless `--out <file>` is given.
    #[command(
        after_help = "Examples:\n  tonk render person\n  tonk render alice@person\n  tonk render alice@person!card --out alice.html"
    )]
    Render {
        /// The render route (e.g. `alice@person!card`).
        #[arg(value_name = "ROUTE")]
        route: String,
        /// Write the HTML to this file instead of stdout.
        #[arg(long, value_name = "PATH")]
        out: Option<PathBuf>,
    },

    // -- collab -------------------------------------------------------
    /// Push, then mint a launcher URL onto a live view
    Share {
        #[command(subcommand)]
        command: ShareCommand,
    },

    /// Mint an invite URL granting access to this repo
    ///
    /// Mints a UCAN delegation chain over the local repo. The
    /// default form is audience-open: anyone holding the URL can
    /// claim by redelegating from the embedded ephemeral key.
    #[command(after_help = "Examples:\n  tonk invite\n  tonk invite --remote prod")]
    Invite {
        /// Override the URL prefix the invite is built against.
        #[arg(long, value_name = "URL", default_value_t = tonk_invite::DEFAULT_BASE_URL.to_string())]
        base_url: String,

        /// Embed a registered remote's URL in the invite so
        /// the claimer auto-configures the same access service
        /// after redeeming. Argument is the remote's local
        /// name (as registered with `tonk remote add`).
        #[arg(long, value_name = "NAME")]
        remote: Option<String>,
    },

    /// Claim an invite URL into a fresh .tonk/ here
    ///
    /// Refuses if a site already exists under the current directory.
    #[command(after_help = "Examples:\n  tonk join 'https://...#invite'")]
    Join {
        /// Invite URL produced by `tonk invite` or
        /// tonk-ui's invite flow.
        #[arg(value_name = "URL")]
        url: String,
    },

    /// Push local main to its upstream
    #[command(after_help = "Examples:\n  tonk push")]
    Push,

    /// Pull local main from its upstream
    #[command(after_help = "Examples:\n  tonk pull")]
    Pull,

    /// Manage remotes (add, list, set-upstream)
    Remote {
        #[command(subcommand)]
        command: RemoteCommand,
    },

    // -- setup --------------------------------------------------------
    /// Create a new .tonk/ repo in the current directory
    #[command(after_help = "Examples:\n  tonk init\n  tonk init my-repo")]
    Init {
        /// Optional label for the repository.
        ///
        /// Reserved for a future `dialog.meta/name` claim;
        /// accepted but not yet persisted.
        #[arg(value_name = "LABEL")]
        label: Option<String>,
    },

    /// Show (or reset) the local profile DID
    ///
    /// With `--reset`, deletes the on-disk profile and creates a
    /// fresh identity.
    #[command(after_help = "Examples:\n  tonk identity\n  tonk identity --reset")]
    Identity {
        /// Wipe the on-disk profile and create a new one. This removes
        /// access to existing repos without re-delegation.
        #[arg(long)]
        reset: bool,
    },

    /// Store and inspect content-addressed blobs (images, files)
    Blob {
        #[command(subcommand)]
        command: BlobCommand,
    },

    /// Export local main's artifacts as CSV
    ///
    /// Writes to stdout unless `--out <file>` is given.
    #[command(after_help = "Examples:\n  tonk export\n  tonk export --out data.csv")]
    Export {
        /// Write the CSV to this file instead of stdout.
        #[arg(long, value_name = "PATH")]
        out: Option<PathBuf>,
    },

    /// Import artifacts from a CSV file onto local main
    ///
    /// Commits each row as an assertion.
    #[command(after_help = "Examples:\n  tonk import data.csv")]
    Import {
        /// The CSV file to read (`the,of,as,is,cause` columns).
        #[arg(value_name = "PATH")]
        file: PathBuf,
    },

    /// Migrate a .carry/ directory to .tonk/
    ///
    /// Walks up from `$PWD` to find the source unless `--from` is
    /// supplied; the destination is always a sibling `.tonk/` of
    /// the source.
    #[command(after_help = "Examples:\n  tonk migrate\n  tonk migrate --from ../old --move")]
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

    /// Show or toggle anonymous usage telemetry
    ///
    /// `status` (default) prints the effective state and why;
    /// `on` / `off` persist the choice.
    #[command(after_help = "Examples:\n  tonk telemetry\n  tonk telemetry off")]
    Telemetry {
        /// One of: status, on, off. Omit for status.
        #[arg(value_name = "ACTION")]
        action: Option<TelemetryAction>,
    },
}

#[derive(Subcommand, Debug)]
enum ShareCommand {
    /// Share a concept's auto-rendered listing
    ///
    /// The recipient lands on the auto-rendered concept view at
    /// `/space/<space-name>/branch/main/concept/<name>`.
    #[command(
        after_help = "Examples:\n  tonk share concept person\n  tonk share concept person --space-name demo"
    )]
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

    /// Share a raw HTML page through the iframe viewer
    ///
    /// The recipient lands on the iframe viewer at
    /// `/space/<space-name>/branch/main/view/<entity>` with the body
    /// served from the entity's `text/html` claim. Events don't fire
    /// there — for interactive, data-bound views use `share display`.
    #[command(after_help = "Examples:\n  tonk share view my-page")]
    View {
        /// Bookmark name or `did:key:…` entity URI for the view.
        /// `tonk views` lists what's available.
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

    /// Share an entity rendered live through <tonk-display>
    ///
    /// The recipient lands on
    /// `/space/<space-name>/branch/main/display/<subject>` with
    /// the supplied `--view` carried across as a query parameter.
    /// Use this for declarative views built against the `view`
    /// concept (`{model, display}`), identified by their anchor
    /// name — `share view` is for the iframe viewer.
    #[command(
        after_help = "Examples:\n  tonk share display alice --view person-card\n  tonk share display alice --model person"
    )]
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
    /// Register a UCAN-S3 access-service remote
    ///
    /// Writes the dialog remote handle and the meta-branch
    /// `Remote` concept browsers read.
    #[command(after_help = "Examples:\n  tonk remote add prod https://access.example.com")]
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

    /// Print every remote registered on the meta branch
    #[command(after_help = "Examples:\n  tonk remote list")]
    List,

    /// Wire local main's upstream to <remote>/main
    #[command(after_help = "Examples:\n  tonk remote set-upstream prod")]
    SetUpstream {
        /// Name of the remote to track.
        #[arg(value_name = "REMOTE")]
        remote: String,
    },
}

#[derive(Subcommand, Debug)]
enum BlobCommand {
    /// Ingest a file and print its blob:<hash> reference
    ///
    /// Asserts content-type (and file name) facts.
    #[command(
        after_help = "Examples:\n  tonk blob add photo.png\n  tonk blob add data.bin --type application/octet-stream"
    )]
    Add {
        /// File to ingest.
        #[arg(value_name = "FILE")]
        file: PathBuf,
        /// Override the MIME type (default: inferred from extension).
        #[arg(long = "type", value_name = "MIME")]
        content_type: Option<String>,
    },
    /// Write a blob's bytes to stdout
    #[command(after_help = "Examples:\n  tonk blob cat blob:zAbc...")]
    Cat {
        /// The blob:<hash> reference.
        #[arg(value_name = "BLOB_URI")]
        reference: String,
    },
    /// List blobs in the index with size and content type
    #[command(after_help = "Examples:\n  tonk blob ls")]
    Ls,
}

#[derive(Subcommand, Debug)]
enum ConceptCommand {
    /// Assert a new concept with typed attributes
    ///
    /// Attributes are anchored (`&{concept}-{field}`), so the
    /// concept and its fields resolve by name immediately —
    /// `tonk assert <name> --help` shows the typed flags right
    /// after this succeeds.
    #[command(
        after_help = "Types: text, entity, unsigned-integer... run with a bad type to see the list.\n\nExamples:\n  tonk concept add habit --attr name:text:one --attr target:text:one --description \"a tracked habit\"\n  tonk concept add note --attr body:text:one --attr tag:text:many"
    )]
    Add {
        /// Name for the concept (also the anchor).
        #[arg(value_name = "NAME")]
        name: String,
        /// One field as `<field>:<type>:<cardinality>`; repeatable.
        #[arg(long = "attr", value_name = "FIELD:TYPE:CARD", required = true)]
        attrs: Vec<String>,
        /// Human description for the concept.
        #[arg(long, value_name = "TEXT")]
        description: Option<String>,
    },
}

#[derive(Subcommand, Debug)]
enum ViewCommand {
    /// Assert a declarative view for a concept
    ///
    /// When no home is set yet, the build is auto-surfaced onto the
    /// space home so it's immediately visible.
    #[command(
        after_help = "Examples:\n  tonk view add habit --template '<b>{name}</b>'\n  tonk view add habit --template-file card.html --name habit-card"
    )]
    Add {
        /// The concept this view renders.
        #[arg(value_name = "CONCEPT")]
        model: String,
        /// Inline HTML template ({field} interpolation).
        #[arg(
            long,
            value_name = "HTML",
            conflicts_with = "template_file",
            required_unless_present = "template_file"
        )]
        template: Option<String>,
        /// Read the template from a file instead.
        #[arg(long, value_name = "PATH")]
        template_file: Option<PathBuf>,
        /// Anchor name for the view (default: <concept>-view).
        #[arg(long, value_name = "NAME")]
        name: Option<String>,
    },
}

#[derive(Args, Debug)]
#[command(
    after_help = "Examples:\n  tonk eval -c 'person:'\n  tonk eval ./doc.notation\n  cat doc.notation | tonk eval -\n  tonk eval -c 'person:' --format json\n  tonk eval ./doc.notation --no-sync\n  tonk eval ./doc.notation --dry-run"
)]
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

    /// Skip the automatic pull-before / push-after that wraps a
    /// committing eval when an upstream is configured. The manual
    /// `tonk pull` / `tonk push` flow stays available. Also
    /// settable via the `TONK_NO_SYNC` environment variable.
    #[arg(long = "no-sync")]
    no_sync: bool,

    /// Run the document without committing: analyze, query, and
    /// plan, then drop the transaction. Query matches are returned
    /// and the commit summary shows zero claims; the branch is left
    /// untouched. Implies `--no-sync` (a preview never touches the
    /// remote). Mirrors the worker's `transact=false` preview the
    /// notebook editor uses to render results as you type.
    #[arg(long = "dry-run")]
    dry_run: bool,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
enum TelemetryAction {
    Status,
    On,
    Off,
}

/// Static command/subcommand names for telemetry. Only these strings
/// (never argument values) are ever reported.
fn descriptor(command: &Command) -> (&'static str, Option<&'static str>) {
    match command {
        Command::Init { .. } => ("init", None),
        Command::Identity { .. } => ("identity", None),
        Command::Eval(_) => ("eval", None),
        Command::Guide { .. } => ("guide", None),
        Command::Schema { .. } => ("schema", None),
        Command::Concepts => ("concepts", None),
        Command::Query { .. } => ("query", None),
        Command::Get { .. } => ("get", None),
        Command::Assert { .. } => ("assert", None),
        Command::Retract { .. } => ("retract", None),
        Command::Views => ("views", None),
        Command::Migrate { .. } => ("migrate", None),
        Command::Export { .. } => ("export", None),
        Command::Render { .. } => ("render", None),
        Command::Import { .. } => ("import", None),
        Command::Push => ("push", None),
        Command::Pull => ("pull", None),
        Command::Status => ("status", None),
        Command::Invite { .. } => ("invite", None),
        Command::Join { .. } => ("join", None),
        Command::Remote { command } => (
            "remote",
            Some(match command {
                RemoteCommand::Add { .. } => "add",
                RemoteCommand::List => "list",
                RemoteCommand::SetUpstream { .. } => "set-upstream",
            }),
        ),
        Command::Share { command } => (
            "share",
            Some(match command {
                ShareCommand::Concept { .. } => "concept",
                ShareCommand::View { .. } => "view",
                ShareCommand::Display { .. } => "display",
            }),
        ),
        Command::Concept { command } => (
            "concept",
            Some(match command {
                ConceptCommand::Add { .. } => "add",
            }),
        ),
        Command::View { command } => (
            "view",
            Some(match command {
                ViewCommand::Add { .. } => "add",
            }),
        ),
        Command::Home { .. } => ("home", None),
        Command::Telemetry { .. } => ("telemetry", None),
        Command::Blob { command } => (
            "blob",
            Some(match command {
                BlobCommand::Add { .. } => "add",
                BlobCommand::Cat { .. } => "cat",
                BlobCommand::Ls => "ls",
            }),
        ),
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let cli = Cli::parse();

    // The telemetry subcommand itself is never tracked — toggling
    // must not race its own event, and opt-out should be silent.
    let mut recorder = match &cli.command {
        Command::Telemetry { .. } => None,
        command => {
            let (name, subcommand) = descriptor(command);
            tonk_cli::telemetry::begin(name, subcommand).await
        }
    };
    if let (Some(recorder), Command::Eval(args)) = (recorder.as_mut(), &cli.command) {
        recorder.property(
            "source",
            match (&args.command, &args.path) {
                (Some(_), _) => "inline",
                (None, Some(path)) if path == "-" => "stdin",
                (None, Some(_)) => "file",
                (None, None) => "stdin",
            },
        );
        recorder.property("format", format!("{:?}", args.format).to_lowercase());
        recorder.property("dry_run", args.dry_run);
        recorder.property("quiet", args.quiet);
    }

    let started = std::time::Instant::now();
    let exit = match cli.command {
        Command::Init { label } => init(label).await,
        Command::Identity { reset } => identity(reset).await,
        Command::Eval(args) => eval(args).await,
        Command::Guide { topic } => print_guide(topic.as_deref()),
        Command::Schema { concept } => print_schema(concept).await,
        Command::Concepts => print_concepts().await,
        Command::Query { concept, json } => query_op(concept, json).await,
        Command::Get {
            concept,
            entity,
            json,
        } => get_op(concept, entity, json).await,
        Command::Assert { concept, rest } => assert_cmd(concept, rest).await,
        Command::Retract {
            concept,
            entity,
            field,
        } => retract_op(concept, entity, field).await,
        Command::Views => print_views().await,
        Command::Migrate { from, do_move } => migrate(from, do_move).await,
        Command::Export { out } => export_op(out).await,
        Command::Render { route, out } => render_op(route, out).await,
        Command::Import { file } => import_op(file).await,
        Command::Push => sync_op(SyncOp::Push).await,
        Command::Pull => sync_op(SyncOp::Pull).await,
        Command::Status => status_op().await,
        Command::Invite { base_url, remote } => mint_invite(base_url, remote).await,
        Command::Join { url } => claim_invite(url).await,
        Command::Remote { command } => remote_op(command).await,
        Command::Blob { command } => blob_op(command).await,
        Command::Share { command } => share_op(command).await,
        Command::Concept { command } => concept_op(command).await,
        Command::View { command } => view_op(command).await,
        Command::Home { models } => home_op(models).await,
        Command::Telemetry { action } => telemetry_op(action),
    };

    if let Some(recorder) = recorder {
        recorder.finish(exit, started.elapsed()).await;
    }
    std::process::exit(exit.into_raw());
}

async fn init(label: Option<String>) -> ExitCode {
    let cwd = match std::env::current_dir() {
        Ok(p) => p,
        Err(e) => return print_error(format!("could not determine current directory: {e}")),
    };

    match site::TonkSite::init(&cwd).await {
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
        dry_run: args.dry_run,
    };

    let site = match site::TonkSite::discover_and_open(&cwd).await {
        Ok(s) => s,
        Err(err) => return print_error(err.to_string()),
    };

    // A dry run never commits, so there's nothing to push; force
    // auto-sync off so a preview can't pull the remote in either.
    let sync = !args.dry_run && auto_sync::enabled(args.no_sync);
    match auto_sync::run_eval(&site, source, options, sync).await {
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

fn print_guide(topic: Option<&str>) -> ExitCode {
    let text = match guide::resolve(topic) {
        Ok(text) => text,
        Err(err) => return print_error(err.to_string()),
    };
    let mut stdout = std::io::stdout().lock();
    if let Err(e) = stdout.write_all(text.as_bytes()) {
        return print_error(format!("failed to write stdout: {e}"));
    }
    ExitCode::Success
}

/// Selector for the [`sync_op`] handler. Both `tonk push` and
/// `tonk pull` follow the same site-discovery + dispatch path;
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
    let site = match site::TonkSite::discover_and_open(&cwd).await {
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

async fn export_op(out: Option<PathBuf>) -> ExitCode {
    let cwd = match std::env::current_dir() {
        Ok(p) => p,
        Err(e) => return print_error(format!("could not determine current directory: {e}")),
    };
    let site = match site::TonkSite::discover_and_open(&cwd).await {
        Ok(s) => s,
        Err(err) => return print_error(err.to_string()),
    };

    let destination = match &out {
        Some(path) => transfer::Destination::File(path.clone()),
        None => transfer::Destination::Stdout,
    };

    match transfer::export(&site, destination).await {
        Ok(bytes) => {
            // The CSV may be on stdout, so status goes to stderr.
            if let Some(path) = out {
                eprintln!("exported {bytes} bytes to {}", path.display());
            } else {
                eprintln!("exported {bytes} bytes");
            }
            ExitCode::Success
        }
        Err(err) => {
            eprintln!("error: {err}");
            err.exit_code()
        }
    }
}

async fn render_op(route: String, out: Option<PathBuf>) -> ExitCode {
    let parsed = match RenderRoute::parse(&route) {
        Ok(r) => r,
        Err(err) => return print_error(err.to_string()),
    };
    let cwd = match std::env::current_dir() {
        Ok(p) => p,
        Err(e) => return print_error(format!("could not determine current directory: {e}")),
    };
    let site = match site::TonkSite::discover_and_open(&cwd).await {
        Ok(s) => s,
        Err(err) => return print_error(err.to_string()),
    };

    match render::render(&site, &parsed).await {
        Ok(html) => match &out {
            Some(path) => match std::fs::write(path, &html) {
                Ok(()) => {
                    eprintln!("rendered {} bytes to {}", html.len(), path.display());
                    ExitCode::Success
                }
                Err(e) => print_error(format!("could not write {}: {e}", path.display())),
            },
            None => {
                println!("{html}");
                ExitCode::Success
            }
        },
        Err(err) => print_error(err.to_string()),
    }
}

async fn import_op(file: PathBuf) -> ExitCode {
    let cwd = match std::env::current_dir() {
        Ok(p) => p,
        Err(e) => return print_error(format!("could not determine current directory: {e}")),
    };
    let site = match site::TonkSite::discover_and_open(&cwd).await {
        Ok(s) => s,
        Err(err) => return print_error(err.to_string()),
    };

    match transfer::import(&site, &file).await {
        Ok(revision) => {
            println!(
                "imported {} -> revision {}.{}",
                file.display(),
                revision.period,
                revision.moment,
            );
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

async fn status_op() -> ExitCode {
    let cwd = match std::env::current_dir() {
        Ok(p) => p,
        Err(e) => return print_error(format!("could not determine current directory: {e}")),
    };
    let site = match site::TonkSite::discover_and_open(&cwd).await {
        Ok(s) => s,
        Err(err) => return print_error(err.to_string()),
    };

    match sync::status(&site).await {
        Ok(state) => {
            println!("{}", render_sync_state(state));
            ExitCode::Success
        }
        Err(err) => {
            eprintln!("error: {err}");
            err.exit_code()
        }
    }
}

/// One-line rendering of a [`tonk_schema::SyncState`]: the
/// kebab-case token plus a short gloss of what to do about it.
fn render_sync_state(state: tonk_schema::SyncState) -> &'static str {
    use tonk_schema::SyncState;
    match state {
        SyncState::NoUpstream => "no-upstream (set one with `tonk remote set-upstream <name>`)",
        SyncState::Synced => "synced",
        SyncState::Ahead => "ahead (local has unpushed commits; run `tonk push`)",
        SyncState::Behind => "behind (upstream has new commits; run `tonk pull`)",
        SyncState::Diverged => "diverged (run `tonk pull` to merge, then `tonk push`)",
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
    let site = match site::TonkSite::discover_and_open(&cwd).await {
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

async fn blob_op(command: BlobCommand) -> ExitCode {
    let cwd = match std::env::current_dir() {
        Ok(p) => p,
        Err(e) => return print_error(format!("could not determine current directory: {e}")),
    };
    let site = match site::TonkSite::discover_and_open(&cwd).await {
        Ok(s) => s,
        Err(err) => return print_error(err.to_string()),
    };

    match command {
        BlobCommand::Add { file, content_type } => {
            match blob::add(&site, &file, content_type).await {
                Ok(outcome) => {
                    print_blob_add_outcome(&outcome);
                    ExitCode::Success
                }
                Err(err) => {
                    eprintln!("error: {err}");
                    err.exit_code()
                }
            }
        }
        BlobCommand::Cat { reference } => {
            let mut stdout = tokio::io::stdout();
            match blob::cat(&site, &reference, &mut stdout).await {
                Ok(_) => ExitCode::Success,
                Err(err) => {
                    eprintln!("error: {err}");
                    err.exit_code()
                }
            }
        }
        BlobCommand::Ls => match blob::ls(&site).await {
            Ok(rows) => {
                print_blob_ls(&rows);
                ExitCode::Success
            }
            Err(err) => {
                eprintln!("error: {err}");
                err.exit_code()
            }
        },
    }
}

fn print_blob_ls(rows: &[blob::LsRow]) {
    for row in rows {
        println!(
            "{uri}  {size}  {content_type}",
            uri = row.entity.as_str(),
            size = row.size,
            content_type = row.content_type.as_deref().unwrap_or("-"),
        );
    }
}

fn print_blob_add_outcome(outcome: &BlobAddOutcome) {
    println!("{}", outcome.entity.as_str());
    eprintln!(
        "  content-type: {}, size: {} bytes",
        outcome.content_type, outcome.size
    );
}

async fn share_op(command: ShareCommand) -> ExitCode {
    let cwd = match std::env::current_dir() {
        Ok(p) => p,
        Err(e) => return print_error(format!("could not determine current directory: {e}")),
    };
    let site = match site::TonkSite::discover_and_open(&cwd).await {
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
    let site = match site::TonkSite::discover_and_open(&cwd).await {
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
                    "no remote registered as '{name}'; run `tonk remote list` to see what's there"
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
    // Use the same default site config tonk init writes against,
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
                 tonk doesn't read them yet."
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
    let site = match site::TonkSite::discover_and_open(&cwd).await {
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

/// Query every instance of `concept` as rendered by
/// [`data_ops::query`].
async fn query_op(concept: String, json: bool) -> ExitCode {
    let cwd = match std::env::current_dir() {
        Ok(p) => p,
        Err(e) => return print_error(format!("could not determine current directory: {e}")),
    };
    let site = match site::TonkSite::discover_and_open(&cwd).await {
        Ok(s) => s,
        Err(err) => return print_error(err.to_string()),
    };

    match data_ops::query(&site, &concept, json).await {
        Ok(text) => {
            let mut stdout = std::io::stdout().lock();
            if let Err(e) = stdout.write_all(text.as_bytes()) {
                return print_error(format!("failed to write stdout: {e}"));
            }
            ExitCode::Success
        }
        Err(err) => {
            eprintln!("error: {err}");
            err.exit_code()
        }
    }
}

/// Print a single instance of `concept` as rendered by
/// [`data_ops::get`].
async fn get_op(concept: String, entity: String, json: bool) -> ExitCode {
    let cwd = match std::env::current_dir() {
        Ok(p) => p,
        Err(e) => return print_error(format!("could not determine current directory: {e}")),
    };
    let site = match site::TonkSite::discover_and_open(&cwd).await {
        Ok(s) => s,
        Err(err) => return print_error(err.to_string()),
    };

    match data_ops::get(&site, &concept, &entity, json).await {
        Ok(text) => {
            let mut stdout = std::io::stdout().lock();
            if let Err(e) = stdout.write_all(text.as_bytes()) {
                return print_error(format!("failed to write stdout: {e}"));
            }
            ExitCode::Success
        }
        Err(err) => {
            eprintln!("error: {err}");
            err.exit_code()
        }
    }
}

/// Split `rest` into the optional entity and the flag argv, then
/// assert via [`data_ops::assert_op`]. A leading non-flag token is
/// always the entity (the supersede form) — an entity reference
/// never starts with `-`, and flag values always follow their
/// flag, so the first token is either a flag or the entity. Same
/// dynamic-flag / `--help` handling as the old `add`/`set`.
async fn assert_cmd(concept: String, rest: Vec<String>) -> ExitCode {
    let (entity, argv) = match rest.split_first() {
        Some((first, tail)) if !first.starts_with('-') => (Some(first.clone()), tail.to_vec()),
        _ => (None, rest),
    };
    let cwd = match std::env::current_dir() {
        Ok(p) => p,
        Err(e) => return print_error(format!("could not determine current directory: {e}")),
    };
    let site = match site::TonkSite::discover_and_open(&cwd).await {
        Ok(s) => s,
        Err(err) => return print_error(err.to_string()),
    };

    match data_ops::assert_op(&site, &concept, entity.as_deref(), &argv).await {
        Ok(text) => {
            let mut stdout = std::io::stdout().lock();
            if let Err(e) = stdout.write_all(text.as_bytes()) {
                return print_error(format!("failed to write stdout: {e}"));
            }
            ExitCode::Success
        }
        Err(err) => {
            eprintln!("error: {err}");
            err.exit_code()
        }
    }
}

/// Retract a single field, or a whole instance, as rendered by
/// [`data_ops::retract`].
async fn retract_op(concept: String, entity: String, field: Option<String>) -> ExitCode {
    let cwd = match std::env::current_dir() {
        Ok(p) => p,
        Err(e) => return print_error(format!("could not determine current directory: {e}")),
    };
    let site = match site::TonkSite::discover_and_open(&cwd).await {
        Ok(s) => s,
        Err(err) => return print_error(err.to_string()),
    };

    match data_ops::retract(&site, &concept, &entity, field.as_deref()).await {
        Ok(text) => {
            let mut stdout = std::io::stdout().lock();
            if let Err(e) = stdout.write_all(text.as_bytes()) {
                return print_error(format!("failed to write stdout: {e}"));
            }
            ExitCode::Success
        }
        Err(err) => {
            eprintln!("error: {err}");
            err.exit_code()
        }
    }
}

/// Author a new concept, as rendered by [`data_ops::concept_add`].
async fn concept_op(command: ConceptCommand) -> ExitCode {
    let cwd = match std::env::current_dir() {
        Ok(p) => p,
        Err(e) => return print_error(format!("could not determine current directory: {e}")),
    };
    let site = match site::TonkSite::discover_and_open(&cwd).await {
        Ok(s) => s,
        Err(err) => return print_error(err.to_string()),
    };

    match command {
        ConceptCommand::Add {
            name,
            attrs,
            description,
        } => match data_ops::concept_add(&site, &name, &attrs, description.as_deref()).await {
            Ok(text) => {
                let mut stdout = std::io::stdout().lock();
                if let Err(e) = stdout.write_all(text.as_bytes()) {
                    return print_error(format!("failed to write stdout: {e}"));
                }
                ExitCode::Success
            }
            Err(err) => {
                eprintln!("error: {err}");
                err.exit_code()
            }
        },
    }
}

/// Author a declarative view, as rendered by [`data_ops::view_add`].
/// `--template-file` is read here (the thin binary owns I/O); a
/// missing or empty template surfaces as
/// [`tonk_cli::authoring::AuthoringError::EmptyTemplate`] via
/// `data_ops::view_add`'s own check.
async fn view_op(command: ViewCommand) -> ExitCode {
    let cwd = match std::env::current_dir() {
        Ok(p) => p,
        Err(e) => return print_error(format!("could not determine current directory: {e}")),
    };
    let site = match site::TonkSite::discover_and_open(&cwd).await {
        Ok(s) => s,
        Err(err) => return print_error(err.to_string()),
    };

    match command {
        ViewCommand::Add {
            model,
            template,
            template_file,
            name,
        } => {
            let template = match (template, template_file) {
                (Some(inline), _) => inline,
                (None, Some(path)) => match tokio::fs::read_to_string(&path).await {
                    Ok(text) => text,
                    Err(e) => {
                        return print_error(format!(
                            "could not read template file {}: {e}",
                            path.display()
                        ));
                    }
                },
                (None, None) => {
                    return print_error(
                        "one of --template or --template-file is required".to_string(),
                    );
                }
            };
            match data_ops::view_add(&site, &model, name.as_deref(), &template).await {
                Ok(text) => {
                    let mut stdout = std::io::stdout().lock();
                    if let Err(e) = stdout.write_all(text.as_bytes()) {
                        return print_error(format!("failed to write stdout: {e}"));
                    }
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

/// Put one or more concepts' directories on the space home, as
/// rendered by [`data_ops::home`].
async fn home_op(models: Vec<String>) -> ExitCode {
    let cwd = match std::env::current_dir() {
        Ok(p) => p,
        Err(e) => return print_error(format!("could not determine current directory: {e}")),
    };
    let site = match site::TonkSite::discover_and_open(&cwd).await {
        Ok(s) => s,
        Err(err) => return print_error(err.to_string()),
    };

    match data_ops::home(&site, &models).await {
        Ok(text) => {
            let mut stdout = std::io::stdout().lock();
            if let Err(e) = stdout.write_all(text.as_bytes()) {
                return print_error(format!("failed to write stdout: {e}"));
            }
            ExitCode::Success
        }
        Err(err) => {
            eprintln!("error: {err}");
            err.exit_code()
        }
    }
}

async fn print_views() -> ExitCode {
    let cwd = match std::env::current_dir() {
        Ok(p) => p,
        Err(e) => return print_error(format!("could not determine current directory: {e}")),
    };
    let site = match site::TonkSite::discover_and_open(&cwd).await {
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

async fn print_schema(concept: Option<String>) -> ExitCode {
    let cwd = match std::env::current_dir() {
        Ok(p) => p,
        Err(e) => return print_error(format!("could not determine current directory: {e}")),
    };
    let site = match site::TonkSite::discover_and_open(&cwd).await {
        Ok(s) => s,
        Err(err) => return print_error(err.to_string()),
    };
    let rendered = match &concept {
        Some(name) => match data_ops::schema_subset(&site, name).await {
            Ok(text) => text,
            Err(err) => {
                eprintln!("error: {err}");
                return err.exit_code();
            }
        },
        None => match schema::render(&site).await {
            Ok(text) => text,
            Err(err) => return print_error(err.to_string()),
        },
    };
    let mut stdout = std::io::stdout().lock();
    if let Err(e) = stdout.write_all(rendered.as_bytes()) {
        return print_error(format!("failed to write stdout: {e}"));
    }
    ExitCode::Success
}

fn telemetry_op(action: Option<TelemetryAction>) -> ExitCode {
    use tonk_cli::telemetry;
    match action.unwrap_or(TelemetryAction::Status) {
        TelemetryAction::Status => {
            let settings = telemetry::load();
            let env_off = tonk_analytics::env_opt_out(|key| std::env::var(key).ok());
            let has_key = tonk_analytics::api_key().is_some();
            let effective = settings.enabled && !env_off && has_key;
            println!("telemetry: {}", if effective { "on" } else { "off" });
            if !settings.enabled {
                println!("  disabled via `tonk telemetry off`");
            }
            if env_off {
                println!("  disabled via DO_NOT_TRACK / TONK_TELEMETRY");
            }
            if !has_key {
                println!("  no API key in this build (nothing is ever sent)");
            }
            ExitCode::Success
        }
        action @ (TelemetryAction::On | TelemetryAction::Off) => {
            let enabled = action == TelemetryAction::On;
            let settings = telemetry::Settings {
                enabled,
                notice_shown: true,
            };
            match telemetry::store(&settings) {
                Ok(()) => {
                    println!("telemetry {}", if enabled { "on" } else { "off" });
                    ExitCode::Success
                }
                Err(e) => print_error(format!("could not persist telemetry setting: {e}")),
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
