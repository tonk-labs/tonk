//! `tonk` — local-only CLI for reading and writing tonk facts
//! via asserted-notation.
//!
//! The mutating verb is `eval`: it consumes a notation document
//! and runs the analyze → query → plan → commit pipeline against
//! the selected spot's site. The other subcommands (`identity`,
//! `guide`, `schema`, `migrate`) are read-only or one-shot setup
//! helpers.

use std::io::{IsTerminal as _, Read as _, Write as _};
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
use tonk_cli::sync::{self, SyncOutcome};
use tonk_cli::transfer;
use tonk_cli::views::{self, ViewSummary};
use tonk_cli::{ExitCode, account, account_spots, agents, context, guide, identity, schema, site};

#[derive(Parser, Debug)]
#[command(
    name = "tonk",
    about = "CLI for a synced fact store: inspect live state, run explicit workflows, verify every write",
    version,
    propagate_version = true,
    after_help = "Start with live state, not documentation:\n  tonk context\n  tonk query <CONCEPT> --json\n  tonk assert <CONCEPT> <ENTITY> --<field> <value>\n  tonk query <CONCEPT> <ENTITY> --json\n\nBare `tonk` runs `tonk context`. Use `tonk help <COMMAND>` for more workflows."
)]
struct Cli {
    /// Operate on this spot instead of the active directory binding.
    /// Precedence: --spot > TONK_SPOT > `tonk use` in the nearest
    /// ancestor directory.
    #[arg(long, global = true, value_name = "NAME")]
    spot: Option<String>,

    /// Print full error chains: every layer of context down to the
    /// root cause, not just the outermost message.
    #[arg(long, short = 'v', global = true)]
    verbose: bool,

    #[command(subcommand)]
    command: Option<Command>,
}

/// Whether `--verbose` was passed; read by the error printers. A process
/// global rather than a threaded parameter because errors are printed from
/// dozens of leaf match arms that otherwise never see the parsed `Cli`.
static VERBOSE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[derive(Subcommand, Debug)]
enum Command {
    // -- orient -------------------------------------------------------
    /// Print live concepts and direct read-update-verify workflows
    ///
    /// Read-only. This is also what bare `tonk` runs.
    #[command(after_help = "Examples:\n  tonk\n  tonk context\n  tonk context --json")]
    Context {
        /// Emit the versioned tonk.context.v1 contract.
        #[arg(long)]
        json: bool,
    },

    /// Read or update the AGENTS.md claim carried by this spot
    ///
    /// With no subcommand, writes the raw Markdown to stdout so it can be
    /// projected with `tonk agents > AGENTS.md`. The claim on the repository
    /// subject DID remains the source of truth.
    #[command(
        after_help = "Examples:\n  tonk agents\n  tonk agents --json\n  tonk agents > AGENTS.md\n  tonk agents set AGENTS.md\n  tonk agents set - < AGENTS.md"
    )]
    Agents {
        /// Include the repository subject and observed revision.
        #[arg(long)]
        json: bool,
        #[command(subcommand)]
        command: Option<AgentsCommand>,
    },

    /// Print the built-in agent reference (the index, or one topic)
    ///
    /// With no topic, prints a one-screen index; `tonk guide <topic>`
    /// prints one section; `tonk guide all` prints everything. Useful
    /// for agent harnesses that need to learn the syntax without repo
    /// access.
    // Topic list here is hand-rolled for help text; keep in sync with `guide::TOPICS`.
    #[command(
        after_help = "Topics: notation, views, events, workspace, all\n\nBuilt-in elements (full docs): tonk guide views <element>\n  tonk-display, tonk-prose, tonk-code, tonk-table\n\nExamples:\n  tonk guide\n  tonk guide views\n  tonk guide views tonk-table\n  tonk guide all"
    )]
    Guide {
        /// One of: notation, views, events, workspace, all. Omit for
        /// the index.
        #[arg(value_name = "TOPIC")]
        topic: Option<String>,
        /// Under `views`, a built-in element to show full docs for
        /// (e.g. `tonk guide views tonk-table`).
        #[arg(value_name = "ITEM")]
        item: Option<String>,
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

    /// Report how local main relates to its upstream and its current hash
    ///
    /// Prints `synced`, `ahead`, `behind`, `diverged`, or
    /// `no-upstream`, followed by the current local tree hash. Read-only —
    /// fetches the upstream head without merging.
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
        after_help = "Safe to re-run: identical fields are a no-op.\n\nExamples:\n  tonk assert task --title \"Write the plan\" --done false\n  tonk assert task <ENTITY> --done true\n  tonk assert task --help"
    )]
    Assert {
        /// Name of the concept to assert against. Omit it with
        /// `--help` to see the generic update workflow.
        #[arg(value_name = "CONCEPT", allow_hyphen_values = true)]
        concept: Option<String>,
        /// Optional entity (a leading non-flag token selects the
        /// supersede form) followed by schema-derived `--field
        /// value` flags, captured raw (including a bare `--help`)
        /// so the dynamic per-concept parser — not clap's static
        /// subcommand parser — decides how to handle them.
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        rest: Vec<String>,
    },

    /// Read instances of a concept, every field bound
    ///
    /// With no entity, every instance; with an entity, just that
    /// one. Reads are queries in dialog — read-only, nothing
    /// commits. Filter flags (e.g. `--where`) are the intended
    /// future direction; today the whole concept is returned.
    #[command(
        after_help = "Examples:\n  tonk query task\n  tonk query task alice\n  tonk query task --json"
    )]
    Query {
        /// Name of the concept to query.
        #[arg(value_name = "CONCEPT")]
        concept: String,
        /// Optional bookmark name or `did:key:…` entity URI —
        /// fetch just this instance.
        #[arg(value_name = "ENTITY")]
        entity: Option<String>,
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
    /// Mint an invite URL granting access to this repo
    ///
    /// Mints a UCAN delegation chain over the local repo. The
    /// default form is audience-open: anyone holding the URL can
    /// claim by redelegating from the embedded ephemeral key.
    ///
    /// The link is built on the remote's own origin, so the
    /// recipient lands on the deployment that actually serves the
    /// repo — and that origin's shortcut service can shorten it.
    #[command(
        after_help = "Examples:\n  tonk invite\n  tonk invite --remote prod\n  tonk invite --no-remote\n  tonk invite --no-shorten"
    )]
    Invite {
        /// Override the URL prefix the invite is built against.
        /// Defaults to `/join` on the resolved remote's origin, or
        /// to the canonical base when no single remote resolves.
        #[arg(long, value_name = "URL")]
        base_url: Option<String>,

        /// Embed a registered remote's URL in the invite so
        /// the claimer auto-configures the same access service
        /// after redeeming. Argument is the remote's local
        /// name (as registered with `tonk remote add`).
        /// Defaults to the only registered remote when there
        /// is exactly one.
        #[arg(long, value_name = "NAME", conflicts_with = "no_remote")]
        remote: Option<String>,

        /// Mint an invite carrying no `remote=`, even when remotes
        /// are registered. The recipient joins with no upstream and
        /// wires one by hand. The link still sits on the resolved
        /// remote's origin — only the embedded endpoint is dropped.
        /// Also the way past the several-remotes error, which falls
        /// back to the canonical base. The mint still syncs this
        /// repo to its own upstream if it has one.
        #[arg(long)]
        no_remote: bool,

        /// Mint a seed-free invite for this exact recipient root DID.
        #[arg(long, value_name = "DID")]
        recipient_root: Option<String>,

        /// Print the long invite URL instead of shortening it.
        /// Shortening is a live PUT to the link's own origin, so
        /// this is the way to mint offline, against a deployment
        /// with no shortcut service, or without touching the
        /// canonical base when no remote resolves. Also settable
        /// as TONK_NO_SHORTEN.
        #[arg(long)]
        no_shorten: bool,
    },

    /// Join a shared repo from an invite URL into a new spot
    #[command(after_help = "Examples:\n  tonk join 'https://...#invite' --name garden")]
    Join {
        /// The invite URL (quote it - the #fragment matters).
        #[arg(value_name = "URL")]
        url: String,
        /// Spot name to register the joined repo under.
        #[arg(long, value_name = "NAME")]
        name: String,
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
    /// Use a spot in this directory and its descendants
    ///
    /// Stores only a pointer in the central registry; spot data stays
    /// in its central site directory. A nested binding overrides this
    /// one. Pin one invocation with --spot or TONK_SPOT instead.
    #[command(after_help = "Examples:\n  tonk use\n  tonk use garden")]
    Use {
        /// A registered spot name. Omit it to inspect the current selection.
        #[arg(value_name = "NAME")]
        name: Option<String>,
    },

    /// Manage spots: named, centrally registered fact stores
    Spot {
        #[command(subcommand)]
        command: SpotCommand,
    },

    /// Show (or reset) the local profile DID
    ///
    /// With `--reset`, deletes the on-disk profile and creates a
    /// fresh identity. Hidden from the command list: needed once,
    /// ever, mostly when debugging delegation.
    #[command(
        hide = true,
        after_help = "Examples:\n  tonk identity\n  tonk identity --reset\n\nProvisioning a root is part of `tonk account link`."
    )]
    Identity {
        /// Wipe the on-disk profile and create a new one. This removes
        /// access to existing repos without re-delegation.
        #[arg(long)]
        reset: bool,
    },

    /// Link this machine's profile to a Tonk account
    Account {
        #[command(subcommand)]
        command: AccountCommand,
    },

    /// Store and inspect content-addressed blobs (images, files)
    Blob {
        #[command(subcommand)]
        command: BlobCommand,
    },

    /// Export local main's artifacts as CSV
    ///
    /// Writes to stdout unless `--out <file>` is given. Hidden
    /// from the command list: bulk-transfer plumbing.
    #[command(
        hide = true,
        after_help = "Examples:\n  tonk export\n  tonk export --out data.csv"
    )]
    Export {
        /// Write the CSV to this file instead of stdout.
        #[arg(long, value_name = "PATH")]
        out: Option<PathBuf>,
        /// Branch to export. Branches carry separate data and migrate
        /// separately, so an upgrade covers each in turn.
        #[arg(long, value_name = "NAME", default_value = tonk_cli::site::BRANCH_NAME)]
        branch: String,
    },

    /// Import artifacts from a CSV file onto local main
    ///
    /// Commits each row as an assertion. Hidden from the command
    /// list: bulk-transfer plumbing.
    #[command(hide = true, after_help = "Examples:\n  tonk import data.csv")]
    Import {
        /// The CSV file to read (`the,of,as,is,cause` columns).
        #[arg(value_name = "PATH")]
        file: PathBuf,
        /// Branch to import onto.
        #[arg(long, value_name = "NAME", default_value = tonk_cli::site::BRANCH_NAME)]
        branch: String,
    },

    /// Migrate a .carry/ directory to .tonk/
    ///
    /// Walks up from `$PWD` to find the source unless `--from` is
    /// supplied; the destination is always a sibling `.tonk/` of
    /// the source. Hidden from the command list: a one-time
    /// converter for pre-tonk carry sites.
    #[command(
        hide = true,
        after_help = "Examples:\n  tonk migrate\n  tonk migrate --from ../old --move"
    )]
    Migrate {
        /// Explicit source `.carry/` directory. Default: walk up
        /// from `$PWD`.
        #[arg(long, value_name = "PATH")]
        from: Option<PathBuf>,

        /// Move instead of copy. Atomic rename on the same
        /// filesystem; copy + delete fallback otherwise.
        #[arg(long = "move")]
        do_move: bool,

        /// Upgrade a spot written before the dialog format change.
        ///
        /// Downloads the last build that can read it, exports each
        /// branch, rewrites the schema namespace, and imports the
        /// result here. Unrelated to the `.carry/` move above.
        #[arg(long, conflicts_with_all = ["from", "do_move"])]
        legacy: bool,

        /// Name of the registered legacy spot to upgrade. Required with
        /// `--legacy`.
        ///
        /// A name rather than a path: the build that reads it resolves
        /// spots through the registry, so one that is not registered
        /// cannot be exported by it at all.
        #[arg(long, value_name = "NAME", requires = "legacy")]
        site: Option<String>,

        /// Branches to upgrade. Repeatable; defaults to `main`.
        ///
        /// Branches are not discoverable on a legacy spot — listing them
        /// needs an open branch, which is what fails — so any beyond
        /// `main` must be named.
        #[arg(long, value_name = "NAME", requires = "legacy")]
        branch: Vec<String>,
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

    /// Update tonk to the latest release
    ///
    /// Upgrades installs made by the install script. Copies installed
    /// via npm or nix are left to those tools.
    #[command(after_help = "Examples:\n  tonk update\n  tonk update --disable-check")]
    Update {
        /// Stop checking for new releases in the background.
        #[arg(long, conflicts_with = "enable_check")]
        disable_check: bool,
        /// Resume checking for new releases in the background.
        #[arg(long)]
        enable_check: bool,
    },
}

#[derive(Subcommand, Debug)]
enum AgentsCommand {
    /// Assert a Markdown document on the selected spot's repository subject
    Set {
        /// Markdown file to assert, or `-` for stdin.
        #[arg(value_name = "PATH", default_value = "AGENTS.md")]
        path: PathBuf,
        /// Skip automatic pull-before and push-after.
        #[arg(long)]
        no_sync: bool,
    },
}

#[derive(Subcommand, Debug)]
enum AccountCommand {
    /// Show whether this native profile is linked to an account
    Status,

    /// Approve this native profile with a synced passkey in the browser
    #[command(
        after_help = "Examples:\n  tonk account link\n  tonk account link --name workstation"
    )]
    Link {
        /// Override the automatically generated OS/version device name.
        #[arg(long, value_name = "NAME")]
        name: Option<String>,
        /// Account service base URL (for staging or local development).
        #[arg(
            long,
            value_name = "URL",
            default_value = account::DEFAULT_SERVICE_URL,
            hide = true
        )]
        service_url: String,
        /// Print the approval URL without asking the OS to open it.
        #[arg(long)]
        no_open: bool,
        /// Link even though an earlier logout's detach never reached the
        /// account service. Those devices may stay listed until revoked.
        #[arg(long)]
        abandon_detach: bool,
        /// Authorize through a page that posts the grant back directly,
        /// instead of registering a handoff with the account service.
        #[arg(long, value_name = "URL")]
        via: Option<String>,
    },

    /// Disconnect account services on this device
    ///
    /// Preserves the local identity, root, and spots without revoking this
    /// device. Use `tonk account revoke <DID>` to revoke a device instead.
    Logout,

    /// List or pull the spots backed up under this account
    Spots {
        #[command(subcommand)]
        command: Option<AccountSpotsCommand>,
    },

    /// Move stored delegations into their durable homes
    ///
    /// Drains the legacy certificate store into the profile's access branch
    /// and retains each spot's authority into the account space, so another
    /// device regains access by pulling the account. Safe to re-run.
    #[command(after_help = "Examples:\n  tonk account migrate")]
    Migrate,

    /// List the devices linked to this profile's account
    Devices {
        /// Account service base URL; defaults to the linked provider.
        #[arg(long, value_name = "URL", hide = true)]
        service_url: Option<String>,
    },

    /// Revoke one of the account's devices by DID
    ///
    /// Opens a browser to approve with your passkey: cutting off another
    /// device takes the account root, which only the passkey can derive.
    #[command(after_help = "Examples:\n  tonk account revoke did:key:z6Mk...")]
    Revoke {
        /// DID of the device to revoke (see `tonk account devices`).
        #[arg(value_name = "DID")]
        did: String,
        /// Account service base URL (for staging or local development).
        #[arg(
            long,
            value_name = "URL",
            default_value = account::DEFAULT_SERVICE_URL,
            hide = true
        )]
        service_url: String,
        /// Browser page that runs the approval ceremony.
        #[arg(
            long,
            value_name = "URL",
            default_value = account::DEFAULT_ACCOUNT_PAGE,
            hide = true
        )]
        account_url: String,
        /// Print the approval URL without asking the OS to open it.
        #[arg(long)]
        no_open: bool,
    },
}

#[derive(Subcommand, Debug)]
enum AccountSpotsCommand {
    /// List remote account spots and local registration state
    List,
    /// Pull one exact repository subject into canonical local storage
    Pull {
        /// Full repository subject DID.
        #[arg(value_name = "SUBJECT")]
        subject: String,
        /// Explicit local spot slug.
        #[arg(long, value_name = "SLUG")]
        name: Option<String>,
    },
}

#[derive(Subcommand, Debug)]
enum RemoteCommand {
    /// Register a UCAN-S3 access-service remote
    ///
    /// Writes the dialog remote handle and the meta-branch
    /// `Remote` concept browsers read. When no upstream is wired
    /// yet, the new remote becomes `main`'s upstream (an existing
    /// upstream is never touched — re-point with `set-upstream`).
    #[command(
        after_help = "Examples:\n  tonk remote add prod https://access.example.com --revocation-url https://artifacts.example.com/revocations"
    )]
    Add {
        /// Local name for the remote.
        #[arg(value_name = "NAME")]
        name: String,
        /// UCAN access-service endpoint URL.
        #[arg(value_name = "URL")]
        url: String,
        /// Immutable-artifact relay used for invitation revocations.
        #[arg(long, value_name = "URL")]
        revocation_url: Option<String>,
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
enum SpotCommand {
    /// Create (or adopt) a spot, register it, and use it here
    ///
    /// The site lands in the canonical store
    /// (`~/Library/Application Support/tonk/spots/<name>` on macOS)
    /// unless --site points elsewhere. --site aimed at an existing
    /// site directory adopts it instead of creating fresh — the
    /// migration path for pre-registry `.tonk/` dirs.
    #[command(
        after_help = "Examples:\n  tonk spot new garden\n  tonk spot new work --site ~/work/site\n  tonk spot new proj --site ~/proj/.tonk"
    )]
    New {
        /// Spot name ([a-z0-9][a-z0-9-_]*).
        #[arg(value_name = "NAME")]
        name: String,
        /// Store the site at this directory instead of the
        /// canonical location.
        #[arg(long, value_name = "PATH")]
        site: Option<PathBuf>,
    },

    /// List registered spots, directory bindings, and what is active here
    #[command(after_help = "Examples:\n  tonk spot list")]
    List,

    /// Delete a spot and its data from disk
    ///
    /// This destroys the spot's facts, not just its registration.
    /// It asks for confirmation first, and says whether the data is
    /// backed up to your account (so you can pull it again) or
    /// local-only (so it is gone for good).
    ///
    /// To stop a directory from resolving to a spot without touching
    /// any data, use `tonk spot unbind`. To drop the registration but
    /// keep the data, use --keep-data.
    #[command(
        after_help = "Examples:\n  tonk spot rm garden\n  tonk spot rm garden --yes\n  tonk spot rm garden --keep-data"
    )]
    Rm {
        /// Spot name to delete.
        #[arg(value_name = "NAME")]
        name: String,
        /// Unregister the spot but leave its data on disk.
        ///
        /// The data then belongs to no spot: `tonk spot list` reports
        /// it, `tonk spot new <name> --site <path>` adopts it back,
        /// and it keeps its canonical name reserved against `tonk
        /// join` and `tonk account spots pull`.
        #[arg(long, conflicts_with = "delete")]
        keep_data: bool,
        /// Delete without asking for confirmation.
        #[arg(long, short = 'y')]
        yes: bool,
        /// Accepted for compatibility — deleting the data is now the
        /// default, so this flag does nothing.
        #[arg(long, hide = true)]
        delete: bool,
    },

    /// Unbind a directory from its spot (see `tonk use`)
    ///
    /// Only unlinks the directory: the spot stays registered and no
    /// data is touched. `tonk spot rm` is the one that deletes.
    ///
    /// Matches exactly: run from the directory that was bound,
    /// not a subdirectory of it.
    #[command(after_help = "Examples:\n  tonk spot unbind\n  tonk spot unbind ~/old-project")]
    Unbind {
        /// Directory to unbind. Default: the current directory. Pass
        /// an absolute path to clear an entry whose directory no
        /// longer exists — a vanished directory can't canonicalize,
        /// so a relative path never matches it.
        #[arg(value_name = "PATH")]
        path: Option<PathBuf>,
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

    /// List user-defined concepts on the branch
    ///
    /// One row per concept, tab-separated `name<TAB>description`.
    /// Built-in concepts (`attribute`, `concept`, …) are omitted —
    /// they're resolvable everywhere and would just be noise.
    #[command(after_help = "Examples:\n  tonk concept ls")]
    Ls,
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

    /// List renderable entities (those carrying a text/html claim)
    ///
    /// One row per entity, tab-separated `name<TAB>entity<TAB>bytes`.
    /// Claim-driven: surfaces anything the host route would serve,
    /// regardless of how the claim was asserted.
    #[command(after_help = "Examples:\n  tonk view ls")]
    Ls,
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
        Command::Context { .. } => ("context", None),
        Command::Agents { command, .. } => (
            "agents",
            command.as_ref().map(|command| match command {
                AgentsCommand::Set { .. } => "set",
            }),
        ),
        Command::Use { .. } => ("use", None),
        Command::Spot { command } => (
            "spot",
            Some(match command {
                SpotCommand::New { .. } => "new",
                SpotCommand::List => "list",
                SpotCommand::Rm { .. } => "rm",
                SpotCommand::Unbind { .. } => "unbind",
            }),
        ),
        Command::Identity { .. } => ("identity", None),
        Command::Account { command } => (
            "account",
            Some(match command {
                AccountCommand::Status => "status",
                AccountCommand::Link { .. } => "link",
                AccountCommand::Logout => "logout",
                AccountCommand::Spots { command } => match command {
                    None | Some(AccountSpotsCommand::List) => "spots-list",
                    Some(AccountSpotsCommand::Pull { .. }) => "spots-pull",
                },
                AccountCommand::Migrate => "migrate",
                AccountCommand::Devices { .. } => "devices",
                AccountCommand::Revoke { .. } => "revoke",
            }),
        ),
        Command::Eval(_) => ("eval", None),
        Command::Guide { .. } => ("guide", None),
        Command::Schema { .. } => ("schema", None),
        Command::Query { .. } => ("query", None),
        Command::Assert { .. } => ("assert", None),
        Command::Retract { .. } => ("retract", None),
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
        Command::Concept { command } => (
            "concept",
            Some(match command {
                ConceptCommand::Add { .. } => "add",
                ConceptCommand::Ls => "ls",
            }),
        ),
        Command::View { command } => (
            "view",
            Some(match command {
                ViewCommand::Add { .. } => "add",
                ViewCommand::Ls => "ls",
            }),
        ),
        Command::Home { .. } => ("home", None),
        Command::Telemetry { .. } => ("telemetry", None),
        Command::Update { .. } => ("update", None),
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

/// Whether a command opens the active spot and should name it again
/// if the operation fails.
fn uses_active_spot(command: &Command) -> bool {
    matches!(
        command,
        Command::Context { .. }
            | Command::Agents { .. }
            | Command::Eval(_)
            | Command::Schema { .. }
            | Command::Query { .. }
            | Command::Assert { .. }
            | Command::Retract { .. }
            | Command::Export { .. }
            | Command::Render { .. }
            | Command::Import { .. }
            | Command::Push
            | Command::Pull
            | Command::Status
            | Command::Invite { .. }
            | Command::Remote { .. }
            | Command::Blob { .. }
            | Command::Concept { .. }
            | Command::View { .. }
            | Command::Home { .. }
    )
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let cli = Cli::parse();
    VERBOSE.store(cli.verbose, std::sync::atomic::Ordering::Relaxed);
    let command = cli.command.unwrap_or(Command::Context { json: false });

    // The telemetry subcommand itself is never tracked — toggling
    // must not race its own event, and opt-out should be silent.
    let mut recorder = match &command {
        Command::Telemetry { .. } => None,
        command => {
            let (name, subcommand) = descriptor(command);
            tonk_cli::telemetry::begin(name, subcommand).await
        }
    };
    if let (Some(recorder), Command::Eval(args)) = (recorder.as_mut(), &command) {
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
    // `command` is moved by the dispatch below, so ask now.
    let is_update = matches!(&command, Command::Update { .. });
    let report_active_spot = uses_active_spot(&command);
    let spot = cli.spot;
    let exit = match command {
        Command::Context { json } => context_op(json, spot.as_deref()).await,
        Command::Agents { json, command } => agents_op(json, command, spot.as_deref()).await,
        Command::Use { name } => use_op(name, spot.as_deref()).await,
        Command::Spot { command } => spot_op(command, spot.as_deref()).await,
        Command::Identity { reset } => identity(reset).await,
        Command::Account { command } => account_op(command).await,
        Command::Eval(args) => eval(args, spot.as_deref()).await,
        Command::Guide { topic, item } => print_guide(topic.as_deref(), item.as_deref()),
        Command::Schema { concept } => print_schema(concept, spot.as_deref()).await,
        Command::Query {
            concept,
            entity,
            json,
        } => match entity {
            Some(entity) => get_op(concept, entity, json, spot.as_deref()).await,
            None => query_op(concept, json, spot.as_deref()).await,
        },
        Command::Assert { concept, rest } => assert_cmd(concept, rest, spot.as_deref()).await,
        Command::Retract {
            concept,
            entity,
            field,
        } => retract_op(concept, entity, field, spot.as_deref()).await,
        Command::Migrate {
            from,
            do_move,
            legacy,
            site,
            branch,
        } => {
            if legacy {
                legacy_migrate(site, branch, spot.as_deref()).await
            } else {
                migrate(from, do_move).await
            }
        }
        Command::Export { out, branch } => export_op(out, &branch, spot.as_deref()).await,
        Command::Render { route, out } => render_op(route, out, spot.as_deref()).await,
        Command::Import { file, branch } => import_op(file, &branch, spot.as_deref()).await,
        Command::Push => sync_op(SyncOp::Push, spot.as_deref()).await,
        Command::Pull => sync_op(SyncOp::Pull, spot.as_deref()).await,
        Command::Status => status_op(spot.as_deref()).await,
        Command::Invite {
            base_url,
            remote,
            no_remote,
            recipient_root,
            no_shorten,
        } => {
            mint_invite(
                base_url,
                remote,
                no_remote,
                recipient_root,
                no_shorten,
                spot.as_deref(),
            )
            .await
        }
        Command::Join { url, name } => claim_invite(url, name, spot.as_deref()).await,
        Command::Remote { command } => remote_op(command, spot.as_deref()).await,
        Command::Blob { command } => blob_op(command, spot.as_deref()).await,
        Command::Concept { command } => concept_op(command, spot.as_deref()).await,
        Command::View { command } => view_op(command, spot.as_deref()).await,
        Command::Home { models } => home_op(models, spot.as_deref()).await,
        Command::Telemetry { action } => telemetry_op(action),
        Command::Update {
            disable_check,
            enable_check,
        } => update(disable_check, enable_check).await,
    };
    if exit != ExitCode::Success && report_active_spot {
        print_active_spot_context(spot.as_deref());
    }

    let duration = started.elapsed();

    // `tonk update` speaks for itself: a nag mid-update contradicts
    // the command that just ran, and the toggle must stay silent.
    // Run the check alongside the telemetry flush rather than in
    // front of the command, so the marginal cost is one small GET
    // parallel to a request already in flight.
    let check = async {
        if !is_update {
            tonk_cli::update::check().await;
        }
    };
    match recorder {
        Some(recorder) => {
            tokio::join!(recorder.finish(exit, duration), check);
        }
        None => check.await,
    }
    if !is_update {
        tonk_cli::update::nag();
    }

    std::process::exit(exit.into_raw());
}

/// `tonk identity` — report this device's profile DID and its root, or start
/// over with `--reset`.
///
/// Provisioning a root is no longer a command of its own. It used to be: an
/// `identity link` action opened a provider-free browser ceremony that minted
/// an anonymous root and printed handoff JSON to paste back. That root looked
/// like an account to its owner and was not one — nothing could revoke it, and
/// nothing backed up what it created. `tonk account link` runs the same
/// handoff with an account behind it.
async fn identity(reset: bool) -> ExitCode {
    let result = if reset {
        identity::reset().await
    } else {
        identity::open().await
    };
    match result {
        Ok(profile) => {
            println!("device: {}", profile.did());
            match identity::local_root(&profile).await {
                Ok(Some(root)) => println!("root: {}", root.root_did),
                Ok(None) => println!("root: missing (run `tonk account link`)"),
                Err(error) => return print_failure(error),
            }
            ExitCode::Success
        }
        Err(err) => print_failure(err),
    }
}

/// `tonk context` (and bare `tonk`) — one bounded, read-only workflow card.
async fn context_op(json: bool, spot: Option<&str>) -> ExitCode {
    let (resolved, site) = match open_selected(spot).await {
        Ok(opened) => opened,
        Err(code) => return code,
    };
    let report = match context::inspect(&resolved, &site).await {
        Ok(report) => report,
        Err(err) => return print_error(format!("could not build live context: {err:#}")),
    };
    let rendered = if json {
        match report.render_json() {
            Ok(rendered) => rendered,
            Err(err) => return print_error(format!("could not encode context JSON: {err}")),
        }
    } else {
        report.render_markdown()
    };
    let mut stdout = std::io::stdout().lock();
    if let Err(err) = stdout.write_all(rendered.as_bytes()) {
        return print_error(format!("failed to write stdout: {err}"));
    }
    ExitCode::Success
}

/// `tonk agents` — read or update claim-backed spot instructions.
async fn agents_op(json: bool, command: Option<AgentsCommand>, spot: Option<&str>) -> ExitCode {
    let (_, site) = match open_selected(spot).await {
        Ok(opened) => opened,
        Err(code) => return code,
    };
    match command {
        None => {
            let claim = match agents::get(&site).await {
                Ok(Some(claim)) => claim,
                Ok(None) => {
                    return print_error(
                        "this spot has no AGENTS.md claim\ncreate one: tonk agents set AGENTS.md",
                    );
                }
                Err(err) => return print_error(format!("could not read AGENTS.md claim: {err:#}")),
            };
            let rendered = if json {
                match serde_json::to_string_pretty(&claim) {
                    Ok(json) => format!("{json}\n"),
                    Err(err) => {
                        return print_error(format!("could not encode AGENTS.md JSON: {err}"));
                    }
                }
            } else {
                claim.markdown
            };
            let mut stdout = std::io::stdout().lock();
            if let Err(err) = stdout.write_all(rendered.as_bytes()) {
                return print_error(format!("failed to write stdout: {err}"));
            }
            ExitCode::Success
        }
        Some(AgentsCommand::Set { path, no_sync }) => {
            if json {
                return print_error("`--json` reads a claim and cannot be combined with `set`");
            }
            let markdown = if path.as_os_str() == "-" {
                let mut markdown = String::new();
                if let Err(err) = std::io::stdin().read_to_string(&mut markdown) {
                    return print_error(format!("could not read AGENTS.md from stdin: {err}"));
                }
                markdown
            } else {
                match std::fs::read_to_string(&path) {
                    Ok(markdown) => markdown,
                    Err(err) => {
                        return print_error(format!("could not read {}: {err}", path.display()));
                    }
                }
            };
            match agents::set(&site, &markdown, auto_sync::enabled(no_sync)).await {
                Ok(claim) => {
                    println!(
                        "asserted AGENTS.md claim\nsource: {} {}\nentity: {}\nrevision: {}\nnext: tonk agents --json",
                        claim.source, claim.attribute, claim.entity, claim.revision
                    );
                    ExitCode::Success
                }
                Err(err) => print_error(format!("could not assert AGENTS.md claim: {err:#}")),
            }
        }
    }
}

/// How `tonk account` prints the account repository's lifecycle state.
fn account_state_label(status: tonk_account::AccountStateStatus) -> &'static str {
    match status {
        tonk_account::AccountStateStatus::Unconfigured => "not set up yet",
        tonk_account::AccountStateStatus::Unhydrated => "waiting for first sync",
        tonk_account::AccountStateStatus::Ready => "synced",
    }
}

fn render_account_status(status: account::AccountStatus) -> String {
    match status {
        account::AccountStatus::MissingRoot { device_did } => {
            format!("signed in: no\nroot: missing\nprovider: none\ndevice: {device_did}")
        }
        account::AccountStatus::Unregistered {
            root_did,
            device_did,
        } => format!("signed in: no\nroot: {root_did}\nprovider: none\ndevice: {device_did}"),
        account::AccountStatus::Registered {
            root_did,
            device_did,
            provider,
            account_state,
        } => format!(
            "signed in: yes\nroot: {root_did}\nprovider: {provider}\ndevice: {device_did}\naccount state: {}",
            account_state_label(account_state)
        ),
    }
}

/// Best-effort registration line, quiet about being offline: status
/// must answer without the network. Registration itself is web-only —
/// the browser enrolls during its passkey ceremonies, which is where
/// the account-signed deposits come from — so this only reads state
/// and points at the account page when something is missing.
async fn print_customer_line(profile: &dialog_operator::Profile) {
    use tonk_account::customer::CustomerStatus;
    match tonk_cli::customer::registration_state(profile).await {
        Ok(Some(Some(receipt))) => match receipt.status {
            CustomerStatus::Active => println!("sync service: registered"),
            CustomerStatus::Registered => {
                println!("sync service: waiting for email confirmation (check your inbox)")
            }
            CustomerStatus::Suspended => println!("sync service: suspended"),
        },
        Ok(Some(None)) => {
            let page = tonk_cli::customer::access_origin(profile)
                .await
                .ok()
                .flatten()
                .map(|origin| format!("{origin}account"))
                .unwrap_or_else(|| "the account page".to_string());
            println!("sync service: not registered (open {page} in your browser to finish setup)")
        }
        Ok(None) => {}
        Err(_) => println!("sync service: unreachable"),
    }
}

async fn account_op(command: AccountCommand) -> ExitCode {
    let profile = match identity::open().await {
        Ok(profile) => profile,
        Err(error) => return print_failure(error),
    };
    match command {
        AccountCommand::Status => match account::status(&profile).await {
            Ok(mut status) => {
                // An unhydrated account retries its first sync right
                // here, bounded: the status read is the natural moment
                // someone notices "waiting for first sync", and leaving
                // it sticky until the next link would report a state
                // nothing is working to leave.
                if matches!(
                    &status,
                    account::AccountStatus::Registered {
                        account_state: tonk_account::AccountStateStatus::Unhydrated,
                        ..
                    }
                ) {
                    match tokio::time::timeout(
                        std::time::Duration::from_secs(10),
                        tonk_cli::account_state::ensure(&profile),
                    )
                    .await
                    {
                        Ok(Ok(outcome)) => {
                            if let Some(warning) = outcome.warning {
                                eprintln!("warning: account sync attempt: {warning}");
                            }
                            if let Ok(fresh) = account::status(&profile).await {
                                status = fresh;
                            }
                        }
                        Ok(Err(error)) => eprintln!("warning: account sync attempt: {error:#}"),
                        Err(_) => eprintln!("warning: account sync attempt timed out"),
                    }
                }
                let linked = matches!(status, account::AccountStatus::Registered { .. });
                println!("{}", render_account_status(status));
                if linked {
                    print_customer_line(&profile).await;
                }
                ExitCode::Success
            }
            Err(error) => print_failure(error),
        },
        AccountCommand::Migrate => {
            match tonk_cli::account_state::migrate_delegations_here().await {
                Ok(outcome) => {
                    println!(
                        "migrated {} certificate{} into access facts",
                        outcome.certificates,
                        if outcome.certificates == 1 { "" } else { "s" }
                    );
                    println!(
                        "retained {} spot{} into the account space ({} already there)",
                        outcome.spots,
                        if outcome.spots == 1 { "" } else { "s" },
                        outcome.already
                    );
                    if outcome.account_legacy {
                        eprintln!(
                            "warning: the account repository is still in the legacy format; \
                             certificate migration completed, but spot retention was skipped"
                        );
                    }
                    ExitCode::Success
                }
                Err(error) => print_failure(error),
            }
        }
        AccountCommand::Link {
            name,
            service_url,
            no_open,
            abandon_detach,
            via,
        } => match account::link(
            &profile,
            &account::LinkOptions {
                service_url,
                device_name: name.unwrap_or_else(account::default_device_name),
                open_browser: !no_open,
                abandon_detach,
                via,
                announce: None,
                store: None,
            },
        )
        .await
        {
            Ok(outcome) => {
                println!(
                    "linked\nroot: {}\ndevice: {}\naccount state: {}",
                    outcome.root_did,
                    outcome.device_did,
                    account_state_label(outcome.account_state)
                );
                if let Some(warning) = outcome.warning {
                    eprintln!("warning: account repository is not synchronized: {warning}");
                }
                print_customer_line(&profile).await;
                back_up_all_best_effort(&profile).await;
                ExitCode::Success
            }
            Err(error) => print_failure(error),
        },
        AccountCommand::Logout => match account::logout(&profile).await {
            Ok(()) => {
                println!("logged out\ndevice: {}", profile.did());
                ExitCode::Success
            }
            Err(error) => print_failure(error),
        },
        AccountCommand::Spots { command } => {
            let store = match tonk_cli::spot::SpotStore::open() {
                Ok(store) => store,
                Err(error) => return print_failure(error),
            };
            match command.unwrap_or(AccountSpotsCommand::List) {
                AccountSpotsCommand::List => match account_spots::list(&profile, &store).await {
                    Ok(rows) => {
                        if rows.is_empty() {
                            println!("(no account spots backed up)");
                        } else {
                            for row in rows {
                                let state = if row.ambiguous {
                                    "ambiguous"
                                } else if row.local_name.is_some() {
                                    "local"
                                } else {
                                    "remote"
                                };
                                let name = row
                                    .remote_name
                                    .as_deref()
                                    .or(row.local_name.as_deref())
                                    .unwrap_or("-");
                                println!("{state}\t{name}\t{}", row.subject);
                            }
                        }
                        ExitCode::Success
                    }
                    Err(error) => print_failure(error),
                },
                AccountSpotsCommand::Pull { subject, name } => {
                    match account_spots::pull(&profile, &store, &subject, name.as_deref()).await {
                        Ok(outcome) => {
                            if outcome.already_local {
                                println!("already local\t{}\t{}", outcome.name, outcome.subject);
                            } else {
                                println!("pulled\t{}\t{}", outcome.name, outcome.subject);
                                println!("site: {}", outcome.site.display());
                            }
                            if let Some(warning) = outcome.warning {
                                eprintln!("warning: {warning}");
                            }
                            ExitCode::Success
                        }
                        Err(error) => print_failure(error),
                    }
                }
            }
        }
        AccountCommand::Devices { service_url } => {
            match account::devices(&profile, service_url.as_deref()).await {
                Ok(rows) => {
                    let own = profile.did().to_string();
                    for row in rows {
                        let marker = if row.did == own { " (this device)" } else { "" };
                        println!("{}\t{}\t{}{}", row.status, row.name, row.did, marker);
                    }
                    ExitCode::Success
                }
                Err(error) => print_failure(error),
            }
        }
        AccountCommand::Revoke {
            did,
            service_url,
            account_url,
            no_open,
        } => {
            let options = account::RevokeOptions {
                service_url,
                account_url,
                open_browser: !no_open,
            };
            match account::revoke(&profile, &options, &did).await {
                Ok(account::RevokeOutcome::Revoked) => {
                    println!("revoked\ndevice: {did}");
                    ExitCode::Success
                }
                Ok(account::RevokeOutcome::AlreadyRevoked) => {
                    println!("already revoked\ndevice: {did}");
                    ExitCode::Success
                }
                Err(error) => print_failure(error),
            }
        }
    }
}

async fn back_up_all_best_effort(profile: &dialog_operator::Profile) {
    let store = match tonk_cli::spot::SpotStore::open() {
        Ok(store) => store,
        Err(error) => {
            eprintln!("warning: account spot backup skipped: {error}");
            return;
        }
    };
    for warning in account_spots::back_up_registered(profile, &store).await {
        eprintln!(
            "warning: account spot backup for '{}' failed: {}",
            warning.name, warning.message
        );
    }
}

async fn back_up_site_best_effort(name: &str, site: &site::TonkSite) {
    if let Err(error) = account_spots::back_up_site(name, site).await {
        eprintln!("warning: account spot backup failed: {error:#}");
    }
}

/// `tonk use [name]` — inspect the active spot or bind this directory.
async fn use_op(name: Option<String>, flag: Option<&str>) -> ExitCode {
    let store = match tonk_cli::spot::SpotStore::open() {
        Ok(store) => store,
        Err(err) => return print_failure(err),
    };
    let cwd = working_directory();
    match name {
        Some(name) => {
            let Some(cwd) = cwd else {
                return print_error("could not read the current directory".to_owned());
            };
            match tonk_cli::spot::bind(&store, &name, &cwd) {
                Ok(outcome) => {
                    let was = match &outcome.previous {
                        Some(previous) if previous != &outcome.name => {
                            format!(" (was {previous})")
                        }
                        _ => String::new(),
                    };
                    println!(
                        "binding: {name}{was}\ndirectory: {directory}",
                        name = outcome.name,
                        directory = outcome.directory.display(),
                    );
                    print_active_resolution(&store, flag, Some(&cwd));
                    println!("next: tonk context");
                    ExitCode::Success
                }
                Err(err) => print_failure(err),
            }
        }
        None => {
            let env = spot_from_environment();
            let listing =
                match tonk_cli::spot::listing(&store, flag, env.as_deref(), cwd.as_deref()) {
                    Ok(listing) => listing,
                    Err(err) => return print_failure(err),
                };
            match listing.active {
                Some(active) => println!(
                    "current spot: {} ({})\nselected via: {}",
                    active.name,
                    active.site.display(),
                    active.source
                ),
                None => println!("current spot: (none)"),
            }
            if !listing.rows.is_empty() {
                println!("registered:");
                for (registered, site) in listing.rows {
                    println!("  {registered}\t{}", site.display());
                }
            }
            println!("next: tonk context");
            ExitCode::Success
        }
    }
}

/// `tonk spot new|list|rm` — registry management.
async fn spot_op(command: SpotCommand, flag: Option<&str>) -> ExitCode {
    let store = match tonk_cli::spot::SpotStore::open() {
        Ok(store) => store,
        Err(err) => return print_failure(err),
    };
    match command {
        SpotCommand::New { name, site } => {
            let Some(cwd) = working_directory() else {
                return print_error("could not read the current directory".to_owned());
            };
            match tonk_cli::spot::create(
                &store,
                &name,
                site.as_deref(),
                Some(&cwd),
                site::default_config(),
            )
            .await
            {
                Ok(outcome) => {
                    if outcome.adopted {
                        println!(
                            "Registered spot '{}' on the site data already at that path",
                            outcome.name
                        );
                    } else {
                        println!("Registered spot '{}'", outcome.name);
                    }
                    println!("site: {}", outcome.site.display());
                    println!("DID: {}", outcome.did);
                    println!("binding: {}", cwd.display());
                    print_active_resolution(&store, flag, Some(&cwd));
                    match site::TonkSite::open(&outcome.site).await {
                        Ok(site) => back_up_site_best_effort(&outcome.name, &site).await,
                        Err(error) => {
                            eprintln!("warning: account spot backup skipped: {error:#}")
                        }
                    }
                    ExitCode::Success
                }
                Err(err) => print_failure(err),
            }
        }
        SpotCommand::List => {
            let env = std::env::var(tonk_cli::spot::SPOT_ENV)
                .ok()
                .filter(|value| !value.is_empty());
            let cwd = working_directory();
            match tonk_cli::spot::listing(&store, flag, env.as_deref(), cwd.as_deref()) {
                Ok(listing) => {
                    if listing.rows.is_empty() {
                        println!("(no spots registered; create one with `tonk spot new <name>`)");
                        print_orphaned_sites(&listing.orphans);
                        return ExitCode::Success;
                    }
                    let active = listing.active.as_ref().map(|c| c.name.as_str());
                    for (name, site) in &listing.rows {
                        let marker = if Some(name.as_str()) == active {
                            '*'
                        } else {
                            ' '
                        };
                        println!("{marker} {name}\t{site}", site = site.display());
                    }
                    if let Some(resolved) = &listing.active {
                        println!(
                            "active here: {name} ({source})",
                            name = resolved.name,
                            source = resolved.source,
                        );
                    }
                    if !listing.bindings.is_empty() {
                        println!();
                        println!("directories:");
                        for (directory, name) in &listing.bindings {
                            println!("  {directory}\t{name}", directory = directory.display());
                        }
                    }
                    print_orphaned_sites(&listing.orphans);
                    ExitCode::Success
                }
                Err(err) => print_failure(err),
            }
        }
        SpotCommand::Rm {
            name,
            keep_data,
            yes,
            delete: _,
        } => spot_rm(&store, &name, keep_data, yes).await,
        SpotCommand::Unbind { path } => {
            let directory = match path.or_else(working_directory) {
                Some(directory) => directory,
                None => return print_error("could not read the current directory".to_owned()),
            };
            match tonk_cli::spot::unbind(&store, &directory) {
                Ok(outcome) => {
                    println!(
                        "unbound {directory} from {name}",
                        directory = outcome.directory.display(),
                        name = outcome.name,
                    );
                    ExitCode::Success
                }
                Err(err) => print_failure(err),
            }
        }
    }
}

/// Report canonical site data that no registered spot names.
///
/// Silent when there is none, so the common listing stays clean. When
/// there is some it belongs on screen: it is otherwise entirely
/// invisible, and it is the thing that will refuse a later `tonk
/// join` or `tonk account spots pull` on the same name.
fn print_orphaned_sites(orphans: &[PathBuf]) {
    if orphans.is_empty() {
        return;
    }
    println!();
    println!("unregistered site data (belongs to no spot):");
    for path in orphans {
        println!("  {}", path.display());
    }
    println!("  adopt it with `tonk spot new <name> --site <path>`, or delete the directory");
}

/// `tonk spot rm` — delete a spot's data, or (with --keep-data) just
/// its registration.
///
/// Deleting is the default because the alternative is worse: an
/// unregistered site directory is invisible to every command that
/// reads the registry, yet still holds the canonical name against
/// `tonk join --name` and `tonk account spots pull --name`. Making
/// that the accident-shaped path instead of the deliberate one is
/// what this command is for.
async fn spot_rm(
    store: &tonk_cli::spot::SpotStore,
    name: &str,
    keep_data: bool,
    yes: bool,
) -> ExitCode {
    use tonk_cli::spot::{Data, Deletion};

    let registry = match store.load() {
        Ok(registry) => registry,
        Err(err) => return print_failure(err),
    };
    // Resolved up front so the confirmation can name the exact
    // directory it is about to destroy, and so an unknown name fails
    // before anything is inspected or printed.
    let Some(entry) = registry.spots.get(name) else {
        return print_failure(tonk_cli::spot::SpotError::Unknown {
            name: name.to_owned(),
            available: registry.spots.keys().cloned().collect(),
            binding: None,
        });
    };
    let site = entry.site.clone();

    if keep_data {
        return match tonk_cli::spot::remove(store, name, Data::Keep) {
            Ok(outcome) => {
                println!("Unregistered spot '{}'", outcome.name);
                println!("data kept at {}", outcome.site.display());
                println!(
                    "  it belongs to no spot now: re-adopt it with \
                     `tonk spot new <name> --site {}`,",
                    outcome.site.display()
                );
                println!("  or delete it with `tonk spot rm <name>` after re-adopting");
                for directory in &outcome.unbound {
                    println!("unbound {}", directory.display());
                }
                ExitCode::Success
            }
            Err(err) => print_failure(err),
        };
    }

    if !yes {
        // Checked before anything is inspected or printed: there is
        // no one to read a warning addressed to a pipe, and
        // inspecting the site to write it means opening a repository
        // this command was never going to be allowed to delete.
        if !std::io::stdin().is_terminal() {
            return print_error(format!(
                "refusing to delete '{name}': stdin is not a terminal, so the \
                 confirmation cannot be answered. Pass --yes to delete without \
                 confirming, or --keep-data to unregister without deleting."
            ));
        }
        let recovery = tonk_cli::recovery::inspect(&site, site::default_config()).await;
        println!();
        println!("This permanently deletes the spot's data from disk:");
        println!("  {}", site.display());
        println!();
        println!("{}", recovery.consequence(name));
        if let Some(hint) = recovery.restore_hint() {
            println!("  {hint}");
        }
        println!();
        if !confirm_by_name(name) {
            // Non-zero: a caller chaining off `tonk spot rm` must not
            // read "you declined" as "it is gone".
            println!("Aborted; nothing was deleted.");
            return ExitCode::IoError;
        }
    }

    match tonk_cli::spot::remove(store, name, Data::Delete) {
        Ok(outcome) => {
            match outcome.data {
                Deletion::Deleted => {
                    println!("Deleted spot '{}' and its data", outcome.name);
                    println!("removed {}", outcome.site.display());
                }
                // Nothing was destroyed, so don't say it was.
                Deletion::AlreadyGone => {
                    println!("Unregistered spot '{}'", outcome.name);
                    println!("its data was already gone from {}", outcome.site.display());
                }
                Deletion::Kept => unreachable!("Data::Delete never keeps the site"),
            }
            for directory in &outcome.unbound {
                println!("unbound {}", directory.display());
            }
            ExitCode::Success
        }
        Err(err) => print_failure(err),
    }
}

/// Require the operator to type `name` back, returning whether they
/// did. Callers check for a terminal first.
///
/// Typing the name rather than `y` is deliberate: this is the one
/// tonk command that destroys facts, and the cost of a mistaken
/// keystroke is unbounded. Anything else — a wrong answer, a closed
/// stdin, a write that fails — reads as "no", the only safe way to
/// resolve a confirmation nobody gave.
fn confirm_by_name(name: &str) -> bool {
    print!("Type '{name}' to confirm: ");
    if std::io::stdout().flush().is_err() {
        return false;
    }
    let mut answer = String::new();
    if std::io::stdin().read_line(&mut answer).is_err() {
        return false;
    }
    answer.trim() == name
}

async fn eval(args: EvalArgs, spot: Option<&str>) -> ExitCode {
    let source = match resolve_source(&args) {
        Ok(s) => s,
        Err(message) => return print_error(message),
    };

    let options = eval::Options {
        format: args.format.into(),
        quiet: args.quiet,
        dry_run: args.dry_run,
    };

    let (_, site) = match open_selected(spot).await {
        Ok(opened) => opened,
        Err(code) => return code,
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

fn print_guide(topic: Option<&str>, item: Option<&str>) -> ExitCode {
    let text = match guide::resolve(topic, item) {
        Ok(text) => text,
        Err(err) => return print_failure(err),
    };
    let mut stdout = std::io::stdout().lock();
    if let Err(e) = stdout.write_all(text.as_bytes()) {
        return print_error(format!("failed to write stdout: {e}"));
    }
    ExitCode::Success
}

/// Selector for the [`sync_op`] handler. Both `tonk push` and
/// `tonk pull` follow the same spot-resolution + dispatch path;
/// the only thing that differs is which dialog primitive they
/// call and the verb they print on success.
#[derive(Debug, Clone, Copy)]
enum SyncOp {
    Push,
    Pull,
}

async fn sync_op(op: SyncOp, spot: Option<&str>) -> ExitCode {
    let (resolved, site) = match open_selected(spot).await {
        Ok(opened) => opened,
        Err(code) => return code,
    };

    let result = match op {
        SyncOp::Push => sync::push(&site).await,
        SyncOp::Pull => sync::pull(&site).await,
    };

    match result {
        Ok(outcome) => {
            print_sync_outcome(op, &outcome);
            back_up_site_best_effort(&resolved.name, &site).await;
            ExitCode::Success
        }
        Err(err) => {
            eprintln!("error: {err}");
            err.exit_code()
        }
    }
}

/// Upgrade a spot written before the dialog format change.
///
/// Runs the whole sequence: fetch the last build that can read the old
/// format, export each branch with it, rewrite the schema namespace, and
/// import the result into the active spot. Reports per branch rather than
/// as one number, because branches migrate separately and a partial result
/// should say which parts landed.
async fn legacy_migrate(
    site: Option<String>,
    branches: Vec<String>,
    spot: Option<&str>,
) -> ExitCode {
    let Some(site) = site else {
        return print_failure(anyhow::anyhow!(
            "--legacy needs --site <NAME>, the registered spot to upgrade"
        ));
    };
    let branches = if branches.is_empty() {
        vec![tonk_cli::site::BRANCH_NAME.to_owned()]
    } else {
        branches
    };

    let (_, destination) = match open_selected(spot).await {
        Ok(opened) => opened,
        Err(code) => return code,
    };

    // Credentials first, repositories after. A repository's authority chain
    // reaches the account root, so migrating the account is what makes the
    // migrated repositories usable — doing it the other way round leaves
    // data that imports cleanly and cannot be pushed anywhere.
    match tonk_cli::account_state::migrate_delegations_here().await {
        Ok(outcome) => {
            println!(
                "account: {} certificate(s) moved into access facts, \
                 {} spot(s) retained ({} already there)",
                outcome.certificates, outcome.spots, outcome.already
            );
            if outcome.account_legacy {
                eprintln!(
                    "warning: the account repository is still in the legacy format; \
                     certificate migration completed, but spot retention was skipped"
                );
            }
        }
        // An unlinked profile has no account to migrate, which is ordinary
        // rather than a failure — but anything else is worth stopping for,
        // since the repositories below would inherit the problem.
        Err(error) => return print_failure(error),
    }

    let workspace = match tempfile::tempdir() {
        Ok(dir) => dir,
        Err(error) => return print_failure(error),
    };
    eprintln!("fetching {} …", tonk_cli::legacy::LEGACY_RELEASE);
    let cli = match tonk_cli::legacy::fetch_legacy_cli(workspace.path()) {
        Ok(path) => path,
        Err(error) => return print_failure(error),
    };

    for branch in &branches {
        let upgraded = match tonk_cli::legacy::upgrade_branch(&cli, &site, branch, workspace.path())
        {
            Ok(upgraded) => upgraded,
            Err(error) => return print_failure(error),
        };
        let blobs = match tonk_cli::legacy::migrate_blobs(
            &cli,
            &site,
            branch,
            &upgraded.blobs,
            &destination,
            workspace.path(),
        )
        .await
        {
            Ok(blobs) => blobs,
            Err(error) => return print_failure(error),
        };
        if let Err(error) =
            tonk_cli::legacy::import_migrated_branch(&destination, branch, &upgraded.csv).await
        {
            return print_failure(error);
        }
        println!(
            "{branch}: {} rows ({} remapped, {} dropped as dialog's own), \
             {} blobs ({} bytes)",
            upgraded.migration.kept,
            upgraded.migration.remapped,
            upgraded.migration.dropped,
            blobs.copied,
            blobs.bytes
        );
    }
    println!("upgraded {} branch(es)", branches.len());
    ExitCode::Success
}

async fn export_op(out: Option<PathBuf>, branch: &str, spot: Option<&str>) -> ExitCode {
    let (_, site) = match open_selected(spot).await {
        Ok(opened) => opened,
        Err(code) => return code,
    };

    let destination = match &out {
        Some(path) => transfer::Destination::File(path.clone()),
        None => transfer::Destination::Stdout,
    };

    match transfer::export_branch(&site, branch, destination).await {
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

async fn render_op(route: String, out: Option<PathBuf>, spot: Option<&str>) -> ExitCode {
    let parsed = match RenderRoute::parse(&route) {
        Ok(r) => r,
        Err(err) => return print_failure(err),
    };
    let (_, site) = match open_selected(spot).await {
        Ok(opened) => opened,
        Err(code) => return code,
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
        Err(err) => print_failure(err),
    }
}

async fn import_op(file: PathBuf, branch: &str, spot: Option<&str>) -> ExitCode {
    let (_, site) = match open_selected(spot).await {
        Ok(opened) => opened,
        Err(code) => return code,
    };

    match transfer::import_branch(&site, branch, &file).await {
        Ok(revision) => {
            println!(
                "imported {} -> revision {}",
                file.display(),
                revision.edition.value(),
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

async fn status_op(spot: Option<&str>) -> ExitCode {
    let (resolved, site) = match open_selected(spot).await {
        Ok(opened) => opened,
        Err(code) => return code,
    };
    println!(
        "spot: {name} ({source})",
        name = resolved.name,
        source = resolved.source,
    );

    match sync::status_with_hash(&site).await {
        Ok(status) => {
            println!("{}", render_sync_state(status.state));
            if let Some(hash) = status.hash {
                println!("hash: {hash}");
            }
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

async fn remote_op(command: RemoteCommand, spot: Option<&str>) -> ExitCode {
    let (resolved, site) = match open_selected(spot).await {
        Ok(opened) => opened,
        Err(code) => return code,
    };

    match command {
        RemoteCommand::Add {
            name,
            url,
            revocation_url,
            subject,
        } => {
            let subject = match subject.as_deref() {
                Some(raw) => match raw.parse() {
                    Ok(did) => Some(did),
                    Err(e) => return print_error(format!("invalid --subject DID '{raw}': {e:?}")),
                },
                None => None,
            };
            match remote::add_with_revocation(
                &site,
                &name,
                &url,
                subject,
                revocation_url.as_deref(),
            )
            .await
            {
                Ok(outcome) => {
                    print_remote_add_outcome(&outcome);
                    // A first remote with no upstream wired is a
                    // foot-gun (writes only auto-sync once an
                    // upstream exists), and add-then-set-upstream is
                    // nearly always performed together — so the
                    // first remote becomes the upstream by default.
                    // An existing upstream is never touched.
                    match remote::upstream_configured(&site).await {
                        Ok(true) => {
                            back_up_site_best_effort(&resolved.name, &site).await;
                            ExitCode::Success
                        }
                        Ok(false) => match remote::set_upstream(&site, &name).await {
                            Ok(upstream) => {
                                print_set_upstream_outcome(&upstream);
                                back_up_site_best_effort(&resolved.name, &site).await;
                                ExitCode::Success
                            }
                            Err(err) => {
                                eprintln!(
                                    "error: remote added, but wiring it as the upstream \
                                     failed: {err}\nretry with `tonk remote set-upstream {name}`"
                                );
                                err.exit_code()
                            }
                        },
                        Err(err) => {
                            eprintln!(
                                "error: remote added, but checking the upstream failed: {err}\n\
                                 wire it manually with `tonk remote set-upstream {name}`"
                            );
                            err.exit_code()
                        }
                    }
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
                    back_up_site_best_effort(&resolved.name, &site).await;
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
    if let Some(revocation_url) = &outcome.revocation_url {
        println!("  revocation: {revocation_url}");
    }
    println!("  subject:  {}", outcome.subject);
}

fn print_remote_list(records: &[RemoteRecord]) {
    if records.is_empty() {
        println!("(no remotes registered)");
        return;
    }
    for record in records {
        println!(
            "{name}\t{endpoint}\t{subject}\t{revocation}",
            name = record.name,
            endpoint = record.endpoint,
            subject = record.subject,
            revocation = record.revocation_url.as_deref().unwrap_or("-"),
        );
    }
}

async fn blob_op(command: BlobCommand, spot: Option<&str>) -> ExitCode {
    let (_, site) = match open_selected(spot).await {
        Ok(opened) => opened,
        Err(code) => return code,
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

fn print_set_upstream_outcome(outcome: &UpstreamOutcome) {
    println!(
        "Set upstream: {local} -> {remote}/{remote_branch}",
        local = outcome.local_branch,
        remote = outcome.remote,
        remote_branch = outcome.remote_branch,
    );
}

async fn mint_invite(
    base_url: Option<String>,
    remote_name: Option<String>,
    no_remote: bool,
    recipient_root: Option<String>,
    no_shorten: bool,
    spot: Option<&str>,
) -> ExitCode {
    let (_, site) = match open_selected(spot).await {
        Ok(opened) => opened,
        Err(code) => return code,
    };

    // Resolve the remote first: it decides both what gets embedded as
    // `remote=` and, unless `--base-url` overrides, which origin the
    // link points at. Those two have to stay in step — a link on one
    // deployment carrying a remote on another can't be shortened (the
    // shortcut service is same-origin) and drops the recipient on a
    // deployment that isn't serving the repo.
    //
    // `--no-remote` suppresses only the embedded endpoint. The link
    // still belongs on the remote's origin: moving it to the canonical
    // base is the same split this resolution exists to prevent.
    let resolved = match remote::resolve(&site, remote_name.as_deref()).await {
        Ok(record) => record,
        // `--no-remote` is the documented way out of the ambiguity
        // error, so it cannot be blocked by one. Nothing picks an
        // origin here, so the canonical base is the honest answer —
        // said out loud, because it may not be the right one.
        Err(remote::RemoteError::AmbiguousRemote(names)) if no_remote => {
            eprintln!(
                "warning: several remotes are registered ({names}); building the link on \
                 {base}\n         name an origin with `--base-url <URL>` if that is wrong",
                base = invite::DEFAULT_BASE_URL,
            );
            None
        }
        Err(err) => {
            // The shared error names `--remote`; `--no-remote` is the
            // other way out, and only the invite path has it.
            let hint = match err {
                remote::RemoteError::AmbiguousRemote(_) => {
                    "\n       or pass `--no-remote` to mint a link that embeds none"
                }
                _ => "",
            };
            return print_error(format!("{err}{hint}"));
        }
    };

    // `--no-remote` keeps the origin and drops the endpoint.
    let embedded = if no_remote { None } else { resolved.clone() };

    // The mint pushes to whatever `main` tracks, which need not be the
    // remote the link embeds. Say so rather than re-route: a deliberate
    // split setup is legitimate, and a silent one is not. No upstream
    // means no push at all, so nothing to diverge from.
    if let Some(record) = &embedded {
        match remote::upstream_remote(&site).await {
            Ok(Some(upstream)) if upstream != record.name => eprintln!(
                "warning: the invite embeds remote '{embedded}' but the repo pushes to \
                 '{upstream}';\n         the recipient may join a deployment that has not \
                 received this data",
                embedded = record.name,
            ),
            Ok(_) => {}
            Err(err) => eprintln!("warning: could not check the branch's upstream: {err}"),
        }
    }

    let base_url = match (base_url, &resolved) {
        (Some(explicit), _) => explicit,
        (None, Some(record)) => match invite::base_url_for_remote(&record.endpoint) {
            Ok(derived) => derived,
            Err(err) => return print_failure(err),
        },
        (None, None) => invite::DEFAULT_BASE_URL.to_owned(),
    };

    let revocation_url = resolved
        .as_ref()
        .and_then(|record| record.revocation_url.clone());
    if embedded.is_some() && revocation_url.is_none() {
        return print_error(
            "the selected remote has no revocation relay; re-add it with \
             `tonk remote add --revocation-url <URL>`"
                .to_string(),
        );
    }
    let remote_url = embedded.map(|record| record.endpoint);

    let minted = match recipient_root {
        Some(root) => {
            invite::mint_targeted_with_relay(
                &site,
                Some(&base_url),
                remote_url.as_deref(),
                revocation_url.as_deref(),
                &root,
            )
            .await
        }
        None => {
            invite::mint_with_relay(
                &site,
                Some(&base_url),
                remote_url.as_deref(),
                revocation_url.as_deref(),
            )
            .await
        }
    };

    match minted {
        Ok(mut outcome) => {
            // Shorten against the link's own origin; the long URL is
            // fully functional, so an unreachable shortcut service
            // (offline, dev base) degrades with a warning. Skipping
            // is the only way to mint without a live PUT to that
            // origin — which, with no remote resolved, is production.
            if invite::shorten_enabled(no_shorten) {
                match invite::shorten(&outcome.url).await {
                    Ok(short) => outcome.url = short,
                    Err(err) => eprintln!("warning: could not shorten the invite URL: {err}"),
                }
            }
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

/// `tonk join` — claim an invite into a fresh canonical spot:
/// site at `spots/<name>/`, registered and bound here on success.
/// The early registry load below is only a cheap fail-fast
/// duplicate-name check; the invite claim is a network operation
/// that can take seconds, so registration itself happens only
/// after the claim succeeds, against a registry freshly reloaded
/// at that point — a concurrent `tonk spot new`/`use`/`rm` while
/// the claim is in flight is re-checked, never silently reverted.
/// A failed join never leaves a dangling registry entry (a
/// partial site dir may remain; re-running with the same name
/// reports it).
async fn claim_invite(url: String, name: String, flag: Option<&str>) -> ExitCode {
    if let Err(err) = tonk_cli::spot::validate_name(&name) {
        return print_failure(err);
    }
    let store = match tonk_cli::spot::SpotStore::open() {
        Ok(store) => store,
        Err(err) => return print_failure(err),
    };
    let cwd = match working_directory().and_then(|path| path.canonicalize().ok()) {
        Some(cwd) => cwd,
        None => return print_error("could not read the current directory".to_owned()),
    };
    let registry = match store.load() {
        Ok(registry) => registry,
        Err(err) => return print_failure(err),
    };
    if registry.spots.contains_key(&name) {
        return print_error(tonk_cli::spot::SpotError::Exists(name).to_string());
    }
    let root = store.canonical_site(&name);

    // Same default site config `tonk spot new` writes against, so
    // the joined site picks up the user's normal profile.
    match invite::claim(&root, &url, site::default_config()).await {
        Ok(outcome) => {
            // Match `spot new`'s canonicalized form, so registered
            // paths compare equal regardless of how they were added.
            let root = match root.canonicalize() {
                Ok(root) => root,
                Err(err) => {
                    return print_error(format!(
                        "joined, but could not canonicalize {}: {err}",
                        root.display()
                    ));
                }
            };

            let mut registry = match store.load() {
                Ok(registry) => registry,
                Err(err) => return print_failure(err),
            };
            if registry.spots.contains_key(&name) {
                return print_error(format!(
                    "{err}\nthe site was claimed at {root}; register it with \
                     `tonk spot new <other-name> --site {root}`",
                    err = tonk_cli::spot::SpotError::Exists(name.clone()),
                    root = root.display(),
                ));
            }

            registry.spots.insert(
                name.clone(),
                tonk_cli::spot::SpotEntry { site: root.clone() },
            );
            registry.bindings.insert(cwd.clone(), name.clone());
            if let Err(err) = store.save(&registry) {
                return print_error(format!(
                    "joined, but registering spot '{name}' failed: {err}\n\
                     re-register with `tonk spot new {name} --site {root}`",
                    root = root.display(),
                ));
            }
            print_claim_outcome(&name, &root, &cwd, &outcome);
            print_active_resolution(&store, flag, Some(&cwd));
            match site::TonkSite::open(&root).await {
                Ok(site) => back_up_site_best_effort(&name, &site).await,
                Err(error) => eprintln!("warning: account spot backup skipped: {error:#}"),
            }
            ExitCode::Success
        }
        Err(err) => {
            eprintln!("error: {err}");
            err.exit_code()
        }
    }
}

fn print_claim_outcome(
    name: &str,
    root: &std::path::Path,
    directory: &std::path::Path,
    outcome: &ClaimOutcome,
) {
    println!("Joined spot '{name}' ({})", root.display());
    println!("subject: {}", outcome.subject);
    if let Some(remote) = &outcome.auto_configured_remote
        && let Some(url) = &outcome.remote_url
    {
        println!("remote:  {remote} -> {url}");
        if outcome.synced {
            println!("synced:  pulled current state from {remote}");
        } else {
            println!("synced:  no (run `tonk pull` before making changes)");
        }
    }
    println!("binding: {}", directory.display());
    println!("next: tonk context");
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
                "register it as a spot: `tonk spot new <name> --site {}`",
                outcome.destination.display()
            );
            println!(
                "note: any sync remotes from carry's meta branch are preserved on disk; \
                 tonk doesn't read them yet."
            );
            ExitCode::Success
        }
        Err(err) => print_failure(err),
    }
}

/// List user-defined concepts (`tonk concept ls`), one
/// tab-separated `name<TAB>description` row per concept.
async fn list_concepts_op(site: &site::TonkSite) -> ExitCode {
    let concepts = match schema::list_concepts(site).await {
        Ok(c) => c,
        Err(err) => return print_failure(err),
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
async fn query_op(concept: String, json: bool, spot: Option<&str>) -> ExitCode {
    let (_, site) = match open_selected(spot).await {
        Ok(opened) => opened,
        Err(code) => return code,
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
async fn get_op(concept: String, entity: String, json: bool, spot: Option<&str>) -> ExitCode {
    let (_, site) = match open_selected(spot).await {
        Ok(opened) => opened,
        Err(code) => return code,
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

/// Generic workflow for `tonk assert` before a live concept supplies fields.
const ASSERT_USAGE: &str = "\
Write facts: create an instance, or update fields on an existing entity.

Workflow:
  1. tonk query <CONCEPT> --json
  2. tonk assert <CONCEPT> <ENTITY> --<field> <value>
  3. tonk query <CONCEPT> <ENTITY> --json

Create:
  tonk assert <CONCEPT> --<required-field> <value> ...

See the live typed flags:
  tonk assert <CONCEPT> --help

Example:
  tonk query task --json
  tonk assert task <ENTITY> --done true
  tonk query task <ENTITY> --json
";

/// Split `rest` into the optional entity and the flag argv, then
/// assert via [`data_ops::assert_op`]. A leading non-flag token is
/// always the entity (the supersede form) — an entity reference
/// never starts with `-`, and flag values always follow their
/// flag, so the first token is either a flag or the entity. Same
/// dynamic-flag / `--help` handling as the old `add`/`set`.
async fn assert_cmd(concept: Option<String>, rest: Vec<String>, spot: Option<&str>) -> ExitCode {
    let concept = match concept.as_deref() {
        Some("--help") | Some("-h") => {
            print!("{ASSERT_USAGE}");
            return ExitCode::Success;
        }
        Some(name) if name.starts_with('-') => {
            eprintln!("error: expected a concept name, got '{name}'\n\n{ASSERT_USAGE}");
            return ExitCode::AnalyzeError;
        }
        Some(name) => name.to_owned(),
        None => {
            eprintln!("error: missing <CONCEPT>\n\n{ASSERT_USAGE}");
            return ExitCode::AnalyzeError;
        }
    };
    let (entity, argv) = match rest.split_first() {
        Some((first, tail)) if !first.starts_with('-') => (Some(first.clone()), tail.to_vec()),
        _ => (None, rest),
    };
    let (_, site) = match open_selected(spot).await {
        Ok(opened) => opened,
        Err(code) => return code,
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
async fn retract_op(
    concept: String,
    entity: String,
    field: Option<String>,
    spot: Option<&str>,
) -> ExitCode {
    let (_, site) = match open_selected(spot).await {
        Ok(opened) => opened,
        Err(code) => return code,
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
async fn concept_op(command: ConceptCommand, spot: Option<&str>) -> ExitCode {
    let (_, site) = match open_selected(spot).await {
        Ok(opened) => opened,
        Err(code) => return code,
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
        ConceptCommand::Ls => list_concepts_op(&site).await,
    }
}

/// Author a declarative view, as rendered by [`data_ops::view_add`].
/// `--template-file` is read here (the thin binary owns I/O); a
/// missing or empty template surfaces as
/// [`tonk_cli::authoring::AuthoringError::EmptyTemplate`] via
/// `data_ops::view_add`'s own check.
async fn view_op(command: ViewCommand, spot: Option<&str>) -> ExitCode {
    let (_, site) = match open_selected(spot).await {
        Ok(opened) => opened,
        Err(code) => return code,
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
        ViewCommand::Ls => list_views_op(&site).await,
    }
}

/// Put one or more concepts' directories on the space home, as
/// rendered by [`data_ops::home`].
async fn home_op(models: Vec<String>, spot: Option<&str>) -> ExitCode {
    let (_, site) = match open_selected(spot).await {
        Ok(opened) => opened,
        Err(code) => return code,
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

/// List renderable entities (`tonk view ls`), one tab-separated
/// `name<TAB>entity<TAB>bytes` row per `text/html` claim carrier.
async fn list_views_op(site: &site::TonkSite) -> ExitCode {
    let listed = match views::list(site).await {
        Ok(v) => v,
        Err(err) => return print_failure(err),
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

async fn print_schema(concept: Option<String>, spot: Option<&str>) -> ExitCode {
    let (_, site) = match open_selected(spot).await {
        Ok(opened) => opened,
        Err(code) => return code,
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
            Err(err) => return print_failure(err),
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

/// `tonk update` — upgrade in place, or toggle the background check.
async fn update(disable_check: bool, enable_check: bool) -> ExitCode {
    let set_check = match (disable_check, enable_check) {
        (true, _) => Some(false),
        (_, true) => Some(true),
        _ => None,
    };
    match tonk_cli::update::run(set_check).await {
        Ok(message) => {
            println!("{message}");
            ExitCode::Success
        }
        Err(err) => print_error(format!("{err:#}")),
    }
}

fn print_error(message: impl Into<String>) -> ExitCode {
    eprintln!("error: {}", message.into());
    ExitCode::IoError
}

/// Render an error the way the active verbosity calls for: the outermost
/// context alone by default, the whole chain (`{:#}`) under `--verbose`.
/// The difference between "failed to load the local root" and the
/// "no mount for did:key:…" that actually explains it.
fn failure_text(error: &anyhow::Error) -> String {
    if VERBOSE.load(std::sync::atomic::Ordering::Relaxed) {
        format!("{error:#}")
    } else {
        error.to_string()
    }
}

/// Print an [`anyhow::Error`] as `error: …`, honoring `--verbose`.
fn print_failure(error: impl Into<anyhow::Error>) -> ExitCode {
    print_error(failure_text(&error.into()))
}

/// The process's working directory, used only as a key into the
/// binding map. A cwd the OS refuses to report (deleted out from
/// under the process) is not fatal when --spot or TONK_SPOT names
/// the active spot.
fn working_directory() -> Option<PathBuf> {
    std::env::current_dir().ok()
}

fn spot_from_environment() -> Option<String> {
    std::env::var(tonk_cli::spot::SPOT_ENV)
        .ok()
        .filter(|value| !value.is_empty())
}

/// Report the spot that would actually answer a data command after a
/// binding write, including any flag or environment override.
fn print_active_resolution(
    store: &tonk_cli::spot::SpotStore,
    flag: Option<&str>,
    cwd: Option<&std::path::Path>,
) {
    let env = spot_from_environment();
    match store.resolve(flag, env.as_deref(), cwd) {
        Ok(resolved) => println!(
            "active spot: {name} ({source})\nsite: {site}",
            name = resolved.name,
            source = resolved.source,
            site = resolved.site.display(),
        ),
        Err(err) => {
            eprintln!("warning: binding saved, but the active spot does not resolve: {err}")
        }
    }
}

/// Print stable local context after a spot-scoped command fails. This
/// deliberately does not fetch sync state while handling another
/// error.
fn print_active_spot_context(flag: Option<&str>) {
    let Ok(store) = tonk_cli::spot::SpotStore::open() else {
        return;
    };
    let env = spot_from_environment();
    let cwd = working_directory();
    if let Ok(resolved) = store.resolve(flag, env.as_deref(), cwd.as_deref()) {
        eprintln!(
            "active spot: {name} ({source})\nsite: {site}",
            name = resolved.name,
            source = resolved.source,
            site = resolved.site.display(),
        );
    }
}

/// Resolve the active spot (--spot > TONK_SPOT > nearest directory
/// binding) and open its site. The cwd is passed in only as a key
/// into the binding map — it never locates site data.
async fn open_selected(
    flag: Option<&str>,
) -> Result<(tonk_cli::spot::Resolved, site::TonkSite), ExitCode> {
    let store = match tonk_cli::spot::SpotStore::open() {
        Ok(store) => store,
        Err(err) => return Err(print_failure(err)),
    };
    let env = std::env::var(tonk_cli::spot::SPOT_ENV)
        .ok()
        .filter(|value| !value.is_empty());
    let cwd = working_directory();
    let resolved = match store.resolve(flag, env.as_deref(), cwd.as_deref()) {
        Ok(resolved) => resolved,
        Err(err) => return Err(print_failure(err)),
    };
    match site::TonkSite::open(&resolved.site).await {
        Ok(site) => Ok((resolved, site)),
        Err(err) => Err(print_error(format!(
            "could not open the active spot: {err:#}"
        ))),
    }
}

/// Specialized [`print_error`] for parse-error mapping. Kept
/// alongside the others so future lint runs notice if [`EvalError`]
/// gains variants without an exit-code mapping.
#[allow(dead_code)]
fn classify(err: &EvalError) -> ExitCode {
    err.exit_code()
}

#[cfg(test)]
mod account_spots_parser_tests {
    use super::*;

    #[test]
    fn account_status_makes_sign_in_state_explicit() {
        assert_eq!(
            render_account_status(account::AccountStatus::MissingRoot {
                device_did: "did:device".to_string(),
            }),
            "signed in: no\nroot: missing\nprovider: none\ndevice: did:device"
        );
        assert_eq!(
            render_account_status(account::AccountStatus::Unregistered {
                root_did: "did:root".to_string(),
                device_did: "did:device".to_string(),
            }),
            "signed in: no\nroot: did:root\nprovider: none\ndevice: did:device"
        );
        assert_eq!(
            render_account_status(account::AccountStatus::Registered {
                root_did: "did:root".to_string(),
                device_did: "did:device".to_string(),
                provider: "https://accounts.example".to_string(),
                account_state: tonk_account::AccountStateStatus::Ready,
            }),
            "signed in: yes\nroot: did:root\nprovider: https://accounts.example\ndevice: did:device\naccount state: synced"
        );
    }

    #[test]
    fn account_link_name_is_none_when_omitted() {
        let cli = Cli::try_parse_from(["tonk", "account", "link"]).unwrap();
        let Some(Command::Account {
            command: AccountCommand::Link { name, .. },
        }) = cli.command
        else {
            panic!("expected account link");
        };
        assert_eq!(name, None);
    }

    #[test]
    fn account_link_name_preserves_an_explicit_override() {
        let cli =
            Cli::try_parse_from(["tonk", "account", "link", "--name", "workstation"]).unwrap();
        let Some(Command::Account {
            command: AccountCommand::Link { name, .. },
        }) = cli.command
        else {
            panic!("expected account link");
        };
        assert_eq!(name.as_deref(), Some("workstation"));
    }

    #[test]
    fn account_logout_is_a_no_argument_account_operation() {
        let cli = Cli::try_parse_from(["tonk", "account", "logout"]).unwrap();
        let command = cli.command.as_ref().expect("account command");
        assert!(matches!(
            command,
            Command::Account {
                command: AccountCommand::Logout
            }
        ));
        assert_eq!(descriptor(command), ("account", Some("logout")));
        assert!(Cli::try_parse_from(["tonk", "account", "logout", "unexpected"]).is_err());
    }

    #[test]
    fn account_spots_bare_and_list_are_the_same_operation() {
        for args in [
            vec!["tonk", "account", "spots"],
            vec!["tonk", "account", "spots", "list"],
        ] {
            let cli = Cli::try_parse_from(args).unwrap();
            let Some(Command::Account {
                command: AccountCommand::Spots { command },
            }) = cli.command
            else {
                panic!("expected account spots");
            };
            assert!(command.is_none() || matches!(command, Some(AccountSpotsCommand::List)));
        }
    }

    #[test]
    fn account_spots_pull_captures_the_full_subject_and_optional_name() {
        let did = "did:key:z6MkgMn9hDxTd2saBSAouyTpPLWUmzrVTXfS1N5yB4TjJ3qL";
        let cli =
            Cli::try_parse_from(["tonk", "account", "spots", "pull", did, "--name", "garden"])
                .unwrap();
        let Some(Command::Account {
            command:
                AccountCommand::Spots {
                    command: Some(AccountSpotsCommand::Pull { subject, name }),
                },
        }) = cli.command
        else {
            panic!("expected account spots pull");
        };
        assert_eq!(subject, did);
        assert_eq!(name.as_deref(), Some("garden"));
    }
}
