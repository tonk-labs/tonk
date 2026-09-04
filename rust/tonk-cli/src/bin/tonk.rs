//! `tonk` — local-only CLI for reading and writing tonk facts
//! via asserted-notation.
//!
//! The mutating verb is `eval`: it consumes a notation document
//! and runs the analyze → query → plan → commit pipeline against
//! the selected space's site. The other subcommands (`identity`,
//! `guide`, `schema`, `migrate`) are read-only or one-shot setup
//! helpers.

use std::io::{IsTerminal as _, Read as _, Write as _};
use std::path::PathBuf;

use clap::{Args, CommandFactory as _, Parser, Subcommand};

use tonk_cli::Coded;
use tonk_cli::Rows;
use tonk_cli::auto_sync;
use tonk_cli::blob::{self, AddOutcome as BlobAddOutcome};
use tonk_cli::context::SpaceContext;
use tonk_cli::data_ops;
use tonk_cli::eval::{self, Source};
use tonk_cli::invite::{self, ClaimOutcome, InviteOutcome};
use tonk_cli::listing::{self, Listing};
use tonk_cli::migrate::{self, Mode as MigrateMode};
use tonk_cli::output::Format;
use tonk_cli::remote::{self, AddOutcome, RemoteRecord, UpstreamOutcome};
use tonk_cli::render::{self, RenderRoute};
use tonk_cli::sync::{self, SyncOutcome};
use tonk_cli::transfer;
use tonk_cli::views;
use tonk_cli::{ExitCode, account, account_spaces, agents, context, guide, identity, schema, site};

const CLI_INDEX: &str = "\
usage: tonk [--space <name>] [-v] <command> [<args>]

A space is a synced store of facts about entities. A concept is a schema:
an entity that matches one is an instance with typed fields. Views render
instances. Reads and writes are notation, evaluated against the space
(see 'tonk help notation').

start a space (see also: tonk help spaces)
   space      List spaces, create one, or bind this directory to one
   join       Join a shared space from an invite URL

examine state
   status     Where you are: space, branch, sync, account
   show       Describe the schema, a concept, an entity, or a view
   query      Read the instances of a concept
   render     Render a view to HTML

write facts
   assert     Create an instance of a concept, or update fields on one
   retract    Retract a field, or a whole instance
   eval       Evaluate a notation document: anything the verbs can't say

define
   concept    List concepts, or define one with typed fields
   view       List views, or author one for a concept

collaborate (see also: tonk help sync)
   invite     Create an invite URL granting access to this space
   pull       Pull main from its upstream
   push       Push main to its upstream
   remote     List or manage remotes
   account    Sign in to a Tonk account; manage devices and spaces

'tonk help -a' lists every command; 'tonk help -g' lists the guides
(glossary, notation, spaces, tutorial, sync, views, events,
and built-in elements). See 'tonk help <command>'
or 'tonk help <guide>' for details.
";

#[derive(Parser, Debug)]
#[command(
    name = "tonk",
    version,
    propagate_version = true,
    disable_help_subcommand = true,
    override_help = CLI_INDEX
)]
struct Cli {
    /// Operate on this space instead of the active directory binding.
    /// Precedence: --space > TONK_SPACE > `tonk space use` in the nearest
    /// ancestor directory.
    #[arg(long, global = true, value_name = "NAME")]
    space: Option<String>,

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
    /// Show help for a command or built-in guide
    #[command(hide = true)]
    Help {
        /// List every command, including plumbing commands.
        #[arg(short = 'a', long = "all", conflicts_with_all = ["guides", "name"])]
        all: bool,
        /// List the built-in guides.
        #[arg(short = 'g', long = "guides", conflicts_with_all = ["all", "name"])]
        guides: bool,
        /// Command or guide name.
        #[arg(value_name = "COMMAND|GUIDE")]
        name: Option<String>,
    },
    /// Describe the schema, a concept, an entity, or a view
    Show {
        /// Concept, view, entity bookmark, or entity URI.
        #[arg(value_name = "NAME")]
        name: Option<String>,
        /// Entity bookmark or URI when NAME is a concept.
        #[arg(value_name = "ENTITY", requires = "name")]
        entity: Option<String>,
        /// Emit versioned camelCase JSON.
        #[arg(long, conflicts_with = "notation")]
        json: bool,
        /// Emit re-submittable schema notation.
        #[arg(long, conflicts_with = "json")]
        notation: bool,
    },

    /// Report how local main relates to its upstream and its current hash
    ///
    /// Prints `synced`, `ahead`, `behind`, `diverged`, or
    /// `no-upstream`, followed by the current local tree hash. Read-only —
    /// fetches the upstream head without merging.
    #[command(after_help = "Examples:\n  tonk status\n  tonk status --json")]
    Status {
        /// Emit versioned camelCase JSON.
        #[arg(long)]
        json: bool,
    },

    // -- author -------------------------------------------------------
    /// Define a concept with typed fields
    Concept {
        /// Emit versioned camelCase JSON when listing.
        #[arg(long)]
        json: bool,
        #[command(subcommand)]
        command: Option<ConceptCommand>,
    },

    /// Define a view for a concept
    View {
        /// Emit versioned camelCase JSON when listing.
        #[arg(long)]
        json: bool,
        #[command(subcommand)]
        command: Option<ViewCommand>,
    },

    // -- data ---------------------------------------------------------
    /// Create an instance of a concept, or update fields on one
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
    /// Reads every instance through a dialog query — read-only,
    /// nothing commits. Filter flags (e.g. `--where`) are the
    /// intended future direction; today the whole concept is returned.
    #[command(after_help = "Examples:\n  tonk query task\n  tonk query task --json")]
    Query {
        /// Name of the concept to query.
        #[arg(value_name = "CONCEPT")]
        concept: String,
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
        /// Print the notation document without evaluating it.
        #[arg(long)]
        notation: bool,
        #[command(flatten)]
        write: WriteArgs,
    },

    // -- power --------------------------------------------------------
    /// Evaluate an asserted-notation document (the full DSL)
    ///
    /// The escape hatch for anything the verbs don't cover: rules,
    /// multi-statement documents, joins, retractions inside
    /// assertions. `tonk help notation` documents the grammar.
    Eval(EvalArgs),

    /// Render a view to HTML, headlessly
    ///
    /// Route grammar: `{model}` (directory), `{entity}@{model}`
    /// (one entity), `{entity}@{model}!{view}` (explicit `show`
    /// facet, e.g. `label`). Writes HTML to stdout unless
    /// `--out <file>` is given.
    #[command(
        after_help = "Examples:\n  tonk render person\n  tonk render alice@person\n  tonk render alice@person!label --out alice.html"
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
    /// Create an invite URL granting access to this space
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

    /// Join a shared space from an invite URL
    #[command(after_help = "Examples:\n  tonk join 'https://...#invite' --name garden")]
    Join {
        /// The invite URL (quote it - the #fragment matters).
        #[arg(value_name = "URL")]
        url: String,
        /// Space name to register the joined repo under.
        #[arg(long, value_name = "NAME")]
        name: String,
    },

    /// Push local main to its upstream
    #[command(after_help = "Examples:\n  tonk push")]
    Push,

    /// Pull local main from its upstream
    #[command(after_help = "Examples:\n  tonk pull")]
    Pull,

    /// List or manage remotes
    Remote {
        /// Emit versioned camelCase JSON when listing.
        #[arg(long)]
        json: bool,
        #[command(subcommand)]
        command: Option<RemoteCommand>,
    },

    // -- setup --------------------------------------------------------
    /// List or manage spaces
    #[command(name = "space")]
    Space {
        /// Emit versioned camelCase JSON when listing.
        #[arg(long)]
        json: bool,
        #[command(subcommand)]
        command: Option<SpaceCommand>,
    },

    /// Show (or reset) the local profile DID
    ///
    /// With `--reset`, deletes the on-disk profile and creates a
    /// fresh identity. Hidden from the command list: needed once,
    /// ever, mostly when debugging delegation.
    #[command(
        hide = true,
        after_help = "Examples:\n  tonk identity\n  tonk identity --reset\n\nProvisioning a root is part of `tonk account login`."
    )]
    Identity {
        /// Wipe the on-disk profile and create a new one. This removes
        /// access to existing repos without re-delegation.
        #[arg(long)]
        reset: bool,
    },

    /// Sign in to a Tonk account on this device, and manage it
    Account {
        /// Emit versioned camelCase JSON for the bare status form.
        #[arg(long)]
        json: bool,
        #[command(subcommand)]
        command: Option<AccountCommand>,
    },

    /// Store and inspect content-addressed blobs (images, files)
    #[command(hide = true)]
    Blob {
        /// Emit versioned camelCase JSON when listing.
        #[arg(long)]
        json: bool,
        #[command(subcommand)]
        command: Option<BlobCommand>,
    },

    /// Export local main's artifacts as CSV
    ///
    /// Writes to stdout unless `--out <file>` is given. One row per
    /// artifact, which is the bulk path out of a space: `tonk query`
    /// answers a question, this copies everything.
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
    /// Commits each row as an assertion. The inverse of `tonk export`,
    /// and the bulk path in.
    #[command(hide = true, after_help = "Examples:\n  tonk import data.csv")]
    Import {
        /// The CSV file to read (`the,of,as,is,cause` columns).
        #[arg(value_name = "PATH")]
        file: PathBuf,
        /// Branch to import onto.
        #[arg(long, value_name = "NAME", default_value = tonk_cli::site::BRANCH_NAME)]
        branch: String,
        #[command(flatten)]
        write: WriteArgs,
    },

    /// One-time conversions: carry directories, spaces, delegations
    ///
    /// Hidden from the command list. Each subcommand is run once, from
    /// written instructions, and then never again.
    #[command(hide = true)]
    Migrate {
        #[command(subcommand)]
        command: MigrateCommand,
    },

    /// Show or toggle anonymous usage telemetry
    ///
    /// `status` (default) prints the effective state and why;
    /// `on` / `off` persist the choice.
    #[command(
        hide = true,
        after_help = "Examples:\n  tonk telemetry\n  tonk telemetry off"
    )]
    Telemetry {
        /// One of: status, on, off. Omit for status.
        #[arg(value_name = "ACTION")]
        action: Option<TelemetryAction>,
    },

    /// Update tonk to the latest release
    ///
    /// Upgrades installs made by the install script. Copies installed
    /// via npm or nix are left to those tools.
    #[command(
        hide = true,
        after_help = "Examples:\n  tonk update\n  tonk update --disable-check"
    )]
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
    /// Write the claim's Markdown to stdout
    ///
    /// What bare `tonk space agents` runs. `--json` lives here rather than on
    /// the parent because on the parent it could be passed alongside
    /// `set`, where it meant nothing and had to be rejected at runtime.
    #[command(after_help = "Examples:\n  tonk space agents get\n  tonk space agents get --json")]
    Get {
        /// Include the repository subject and observed revision.
        #[arg(long)]
        json: bool,
    },

    /// Assert a Markdown document on the selected space's repository subject
    Set {
        /// Markdown file to assert, or `-` for stdin.
        #[arg(value_name = "PATH", default_value = "AGENTS.md")]
        path: PathBuf,
        #[command(flatten)]
        write: WriteArgs,
    },
}

#[derive(Subcommand, Debug)]
enum AccountCommand {
    /// Show whether this device is signed in, and to which account
    #[command(after_help = "Examples:\n  tonk account status\n  tonk account status --json")]
    Status {
        /// Emit versioned camelCase JSON.
        #[arg(long)]
        json: bool,
    },

    /// Pull the account so devices, spaces, and names read current facts
    ///
    /// Read commands answer instantly from what this device already
    /// knows; this is the one that fetches what other devices changed.
    Sync {
        /// Print each step of the sync as it happens, naming the remote.
        #[arg(long)]
        verbose: bool,
    },

    /// Sign in to your account with a synced passkey in the browser
    ///
    /// Tonk holds one account at a time. Sign out before signing in as
    /// someone else; spaces that belong to the account you leave stay on
    /// disk and work again when it signs back in.
    #[command(
        after_help = "Examples:\n  tonk account login\n  tonk account login --name workstation"
    )]
    Login {
        /// Override the automatically generated OS/version device name.
        #[arg(long, value_name = "NAME")]
        name: Option<String>,
        /// Print the approval URL without asking the OS to open it.
        #[arg(long)]
        no_open: bool,
        /// Authorize through a page that posts the grant back directly,
        /// instead of registering a handoff with the account service.
        #[arg(long, value_name = "URL")]
        via: Option<String>,
    },

    /// Sign out of the account on this device
    ///
    /// Preserves the local identity, root, and spaces without revoking this
    /// device. Use `tonk account revoke <DID>` to revoke a device instead.
    /// Signed out, every local replica stays readable and writable here;
    /// only account services stop.
    Logout,

    /// Review and permanently delete this account in the browser
    ///
    /// This does not delete immediately. The browser shows the exact owned
    /// spaces, requires the verified email, a consequences checkbox, a final
    /// confirmation, and the account passkey. Joined spaces are left intact;
    /// copies already replicated to other devices cannot be erased by Tonk.
    Delete {
        /// Browser account page that runs the deletion ceremony.
        #[arg(
            long,
            value_name = "URL",
            default_value = account::DEFAULT_ACCOUNT_PAGE,
            hide = true
        )]
        account_url: String,
        /// Print the review URL without asking the OS to open it.
        #[arg(long)]
        no_open: bool,
    },

    /// List or pull the spaces your account directory lists
    #[command(name = "space")]
    Space {
        /// Emit versioned camelCase JSON when listing.
        #[arg(long)]
        json: bool,
        #[command(subcommand)]
        command: Option<AccountSpaceCommand>,
    },

    /// List the devices linked to this profile's account
    #[command(after_help = "Examples:\n  tonk account devices\n  tonk account devices --json")]
    Devices {
        /// Emit versioned camelCase JSON.
        #[arg(long)]
        json: bool,
    },

    /// Revoke one of the account's devices by DID
    ///
    /// This device's own account grant is enough: the revocation is
    /// minted here, published to every access service, and the device's
    /// rows leave the account space.
    #[command(after_help = "Examples:\n  tonk account revoke did:key:z6Mk...")]
    Revoke {
        /// DID of the device to revoke (see `tonk account devices`).
        #[arg(value_name = "DID")]
        did: String,
    },
}

#[derive(Subcommand, Debug)]
enum AccountSpaceCommand {
    /// Pull one account-directory space by unique name or exact subject
    Pull {
        /// Unique directory name or full repository subject DID.
        #[arg(value_name = "NAME_OR_SUBJECT")]
        name_or_subject: String,
        /// Explicit local space slug.
        #[arg(long, value_name = "SLUG")]
        name: Option<String>,
    },
    /// Review and permanently delete one owned hosted space
    ///
    /// This opens the browser for an exact-scope review, typed-email
    /// confirmation, final warning, and account-passkey authorization. The
    /// account and every other space remain.
    Delete {
        /// Full repository subject DID.
        #[arg(value_name = "SUBJECT")]
        subject: String,
        /// Browser account page that runs the deletion ceremony.
        #[arg(
            long,
            value_name = "URL",
            default_value = account::DEFAULT_ACCOUNT_PAGE,
            hide = true
        )]
        account_url: String,
        /// Print the review URL without asking the OS to open it.
        #[arg(long)]
        no_open: bool,
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

    /// Wire local main's upstream to <remote>/main
    #[command(after_help = "Examples:\n  tonk remote set-upstream prod")]
    SetUpstream {
        /// Name of the remote to track.
        #[arg(value_name = "REMOTE")]
        remote: String,
    },
}

#[derive(Subcommand, Debug)]
enum SpaceCommand {
    /// Create (or adopt) a space, register it, and use it here
    ///
    /// Signed out, the space is local-only until `tonk space link`
    /// hands it to an account. Signed in, it belongs to that account
    /// from the start: hosted, synced, and listed for your other
    /// devices.
    ///
    /// The site lands in the canonical store
    /// (`~/Library/Application Support/tonk/spaces/<name>` on macOS)
    /// unless --site points elsewhere. --site aimed at an existing
    /// site directory adopts it instead of creating fresh — the
    /// migration path for pre-registry `.tonk/` dirs.
    #[command(
        after_help = "Examples:\n  tonk space new garden\n  tonk space new work --site ~/work/site\n  tonk space new proj --site ~/proj/.tonk"
    )]
    New {
        /// Space name ([a-z0-9][a-z0-9-_]*).
        #[arg(value_name = "NAME")]
        name: String,
        /// Store the site at this directory instead of the
        /// canonical location.
        #[arg(long, value_name = "PATH")]
        site: Option<PathBuf>,
    },

    /// Use a space in this directory and its descendants
    ///
    /// Stores only a pointer in the central registry; space data stays
    /// in its central site directory. A nested binding overrides this
    /// one. Pin one invocation with --space or TONK_SPACE instead.
    ///
    #[command(after_help = "Examples:\n  tonk space use garden")]
    Use {
        /// A registered space name.
        #[arg(value_name = "NAME")]
        name: String,
    },

    /// Pin one or more concepts' directories on the space home
    Home {
        /// Concept name(s) to surface, in order.
        #[arg(value_name = "CONCEPT", required = true)]
        models: Vec<String>,
        /// Print the notation document without evaluating it.
        #[arg(long)]
        notation: bool,
        #[command(flatten)]
        write: WriteArgs,
    },

    /// Read or update the AGENTS.md claim carried by this space
    Agents {
        #[command(subcommand)]
        command: Option<AgentsCommand>,
    },

    /// Link a local-only space to the account you are signed in to
    ///
    /// The space keeps its name, its data, and its directory binding; what
    /// changes is that the account now owns it, hosts it, and lists it for
    /// your other devices. This is the only ownership move tonk makes: a
    /// space that already belongs to an account stays with it, and reaches
    /// other people through `tonk invite`.
    #[command(after_help = "Examples:\n  tonk space link garden")]
    Link {
        /// Registered space name.
        #[arg(value_name = "SPACE")]
        name: String,
    },

    /// Re-root a space under a fresh key, keeping data and history
    ///
    /// The recovery for lost space keys, or the deliberate retirement
    /// of a subject. Every fact and the whole history stay in place;
    /// the space's identity file is replaced, the old one is archived
    /// beside it, and the first commit under the new key records the
    /// transplant so the seam stays visible.
    ///
    /// Everything minted for the old subject stops working: invites,
    /// member grants, hosting. Signed in, the space is provisioned and
    /// pushed under its new identity; either way, share fresh invites.
    #[command(
        after_help = "Examples:\n  tonk space transplant garden\n  tonk space transplant garden --in-place"
    )]
    Transplant {
        /// Registered space name.
        #[arg(value_name = "SPACE")]
        name: String,
        /// Operate on the site directory directly instead of copying
        /// it aside to `<site>.pre-transplant` first.
        #[arg(long)]
        in_place: bool,
    },

    /// Delete a space and its data from disk
    ///
    /// This destroys the space's facts, not just its registration.
    /// It asks for confirmation first, and says whether the data is
    /// listed in your account directory (so you can pull it again) or
    /// local-only (so it is gone for good).
    ///
    /// To stop a directory from resolving to a space without touching
    /// any data, use `tonk space unbind`. To drop the registration but
    /// keep the data, use --keep-data.
    #[command(
        after_help = "Examples:\n  tonk space rm garden\n  tonk space rm garden --yes\n  tonk space rm garden --keep-data"
    )]
    Rm {
        /// Space name to delete.
        #[arg(value_name = "NAME")]
        name: String,
        /// Unregister the space but leave its data on disk.
        ///
        /// The data then belongs to no space: `tonk space` reports
        /// it, `tonk space new <name> --site <path>` adopts it back,
        /// and it keeps its canonical name reserved against `tonk
        /// join` and `tonk account space pull`.
        #[arg(long)]
        keep_data: bool,
        /// Delete without asking for confirmation.
        #[arg(long, short = 'y')]
        yes: bool,
    },

    /// Unbind a directory from its space (see `tonk space use`)
    ///
    /// Only unlinks the directory: the space stays registered and no
    /// data is touched. `tonk space rm` is the one that deletes.
    ///
    /// Matches exactly: run from the directory that was bound,
    /// not a subdirectory of it.
    #[command(after_help = "Examples:\n  tonk space unbind\n  tonk space unbind ~/old-project")]
    Unbind {
        /// Directory to unbind. Default: the current directory. Pass
        /// an absolute path to clear an entry whose directory no
        /// longer exists — a vanished directory can't canonicalize,
        /// so a relative path never matches it.
        #[arg(value_name = "PATH")]
        path: Option<PathBuf>,
    },
}

/// The one-time conversions.
///
/// Two operations that share the name `migrate` and nothing else: a
/// pre-tonk directory move and a delegation-store drain. A third, the
/// pre-dialog-format space upgrade, lived here until the format change
/// was old enough that carrying a downloader for the last build that
/// could read it cost more than it returned.
#[derive(Subcommand, Debug)]
enum MigrateCommand {
    /// Move a pre-tonk .carry/ directory to .tonk/
    ///
    /// Walks up from `$PWD` to find the source unless `--from` is
    /// supplied; the destination is always a sibling `.tonk/` of
    /// the source.
    #[command(
        after_help = "Examples:\n  tonk migrate carry\n  tonk migrate carry --from ../old --move"
    )]
    Carry {
        /// Explicit source `.carry/` directory. Default: walk up
        /// from `$PWD`.
        #[arg(long, value_name = "PATH")]
        from: Option<PathBuf>,

        /// Move instead of copy. Atomic rename on the same
        /// filesystem; copy + delete fallback otherwise.
        #[arg(long = "move")]
        do_move: bool,
    },

    /// Move stored delegations into their durable homes
    ///
    /// Drains the legacy certificate store into the profile's access branch
    /// and retains each space's authority into the account space, so another
    /// device regains access by pulling the account. Safe to re-run.
    #[command(after_help = "Examples:\n  tonk migrate account")]
    Account,
}

#[derive(Subcommand, Debug)]
enum BlobCommand {
    /// Ingest a file and print its blob:<hash> reference
    ///
    /// Asserts content-type (and file name) facts. Like every other
    /// write verb, pulls before and pushes after when an upstream is
    /// configured.
    ///
    /// `--dry-run` reports the type, size, and name that would be
    /// asserted, and no `blob:<hash>`: the hash is a property of the
    /// imported bytes, so producing one would mean importing them and
    /// then declining to commit the metadata that finds them again.
    #[command(
        after_help = "Examples:\n  tonk blob add photo.png\n  tonk blob add data.bin --type application/octet-stream\n  tonk blob add photo.png --dry-run"
    )]
    Add {
        /// File to ingest.
        #[arg(value_name = "FILE")]
        file: PathBuf,
        /// Override the MIME type (default: inferred from extension).
        #[arg(long = "type", value_name = "MIME")]
        content_type: Option<String>,
        #[command(flatten)]
        write: WriteArgs,
    },
    /// Write a blob's bytes to stdout
    #[command(after_help = "Examples:\n  tonk blob cat blob:zAbc...")]
    Cat {
        /// The blob:<hash> reference.
        #[arg(value_name = "BLOB_URI")]
        reference: String,
    },
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
        after_help = "Types: text, entity, unsigned-integer... run with a bad type to see the list.\n\nExamples:\n  tonk concept add habit --field name:text:one --field target:text:one --description \"a tracked habit\"\n  tonk concept add note --field body:text:one --field tag:text:many"
    )]
    Add {
        /// Name for the concept (also the anchor).
        #[arg(value_name = "NAME")]
        name: String,
        /// One field as `<field>:<type>:<cardinality>`; repeatable.
        #[arg(long = "field", value_name = "FIELD:TYPE:CARD", required = true)]
        fields: Vec<String>,
        /// Human description for the concept.
        #[arg(long, value_name = "TEXT")]
        description: Option<String>,
        /// Print the notation document without evaluating it.
        #[arg(long)]
        notation: bool,
        #[command(flatten)]
        write: WriteArgs,
    },
}

#[derive(Subcommand, Debug)]
enum ViewCommand {
    /// Assert a declarative view for a concept
    ///
    /// A first detail or directory view is auto-surfaced when the home is
    /// blank. --home explicitly replaces an existing home.
    #[command(
        after_help = "Examples:\n  tonk view add habit --template '<b>{name}</b>'\n  tonk view add habit --kind directory --template-file habit.html --home"
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
        /// Which `show` facet to author (ui, directory, label, title).
        #[arg(long, value_enum, default_value_t = ViewKindArg::Detail)]
        kind: ViewKindArg,
        /// Atomically replace the current home with this concept's directory.
        #[arg(long)]
        home: bool,
        /// Print the notation document without evaluating it.
        #[arg(long)]
        notation: bool,
        #[command(flatten)]
        write: WriteArgs,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
enum ViewKindArg {
    Detail,
    Directory,
    Label,
    Title,
}

impl From<ViewKindArg> for tonk_cli::authoring::ViewKind {
    fn from(kind: ViewKindArg) -> Self {
        match kind {
            ViewKindArg::Detail => Self::Detail,
            ViewKindArg::Directory => Self::Directory,
            ViewKindArg::Label => Self::Label,
            ViewKindArg::Title => Self::Title,
        }
    }
}

/// The switches every write verb takes, matching `tonk eval`'s.
///
/// Flattened rather than repeated so the three stay spelled, defaulted, and
/// documented identically wherever they appear. `tonk assert` is the one
/// write verb that cannot use this: everything after `<CONCEPT>` reaches it
/// raw, so its copies are built by `data_ops::flags`.
#[derive(Args, Debug, Default, Clone, Copy)]
struct WriteArgs {
    /// Analyze, query, and plan the write, then drop the transaction
    /// instead of committing. The branch is left untouched. Implies
    /// `--no-sync`: a preview never touches the remote.
    #[arg(long = "dry-run")]
    dry_run: bool,

    /// Skip the automatic pull-before / push-after that wraps a
    /// committing write when an upstream is configured. The manual
    /// `tonk pull` / `tonk push` flow stays available. Also settable via
    /// the `TONK_NO_SYNC` environment variable.
    #[arg(long = "no-sync")]
    no_sync: bool,

    /// Print the envelope without the matched rows.
    #[arg(short = 'q', long = "quiet")]
    quiet: bool,
}

impl From<WriteArgs> for tonk_cli::data_ops::WriteOptions {
    fn from(args: WriteArgs) -> Self {
        Self {
            dry_run: args.dry_run,
            no_sync: args.no_sync,
            quiet: args.quiet,
            notation: false,
        }
    }
}

impl WriteArgs {
    fn options(self, notation: bool) -> tonk_cli::data_ops::WriteOptions {
        tonk_cli::data_ops::WriteOptions {
            notation,
            ..self.into()
        }
    }
}

#[derive(Args, Debug)]
#[command(
    after_help = "Examples:\n  tonk eval -c 'person:'\n  tonk eval ./doc.notation\n  cat doc.notation | tonk eval -\n  tonk eval -c 'person:' --json\n  tonk eval ./doc.notation --home todo\n  tonk eval ./doc.notation --no-sync\n  tonk eval ./doc.notation --dry-run"
)]
struct EvalArgs {
    /// Inline document. Mutually exclusive with the positional
    /// path / `-`.
    #[arg(short = 'c', long = "command", value_name = "DOC")]
    command: Option<String>,

    /// Emit `EvaluateResponse` as pretty JSON instead of notation.
    #[arg(long)]
    json: bool,

    /// Suppress the matches section; emit only the envelope
    /// (notation) or the structured commits-only response (JSON).
    #[arg(short = 'q', long = "quiet")]
    quiet: bool,

    /// Path to a notation document, or `-` to read from stdin.
    /// Omit to read from a piped stdin.
    #[arg(value_name = "PATH")]
    path: Option<String>,

    /// Atomically replace the current home with this concept's directory.
    #[arg(long, value_name = "CONCEPT")]
    home: Option<String>,

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
        Command::Help { .. } => ("help", None),
        Command::Space { command, .. } => (
            "space",
            Some(match command {
                None => "list",
                Some(SpaceCommand::New { .. }) => "new",
                Some(SpaceCommand::Use { .. }) => "use",
                Some(SpaceCommand::Link { .. }) => "link",
                Some(SpaceCommand::Transplant { .. }) => "transplant",
                Some(SpaceCommand::Rm { .. }) => "rm",
                Some(SpaceCommand::Unbind { .. }) => "unbind",
                Some(SpaceCommand::Home { .. }) => "home",
                Some(SpaceCommand::Agents { command }) => match command {
                    None | Some(AgentsCommand::Get { .. }) => "agents-get",
                    Some(AgentsCommand::Set { .. }) => "agents-set",
                },
            }),
        ),
        Command::Identity { .. } => ("identity", None),
        Command::Account { command, .. } => (
            "account",
            Some(match command {
                None | Some(AccountCommand::Status { .. }) => "status",
                Some(AccountCommand::Login { .. }) => "login",
                Some(AccountCommand::Logout) => "logout",
                Some(AccountCommand::Delete { .. }) => "delete",
                Some(AccountCommand::Space { command, .. }) => match command {
                    None => "space-list",
                    Some(AccountSpaceCommand::Pull { .. }) => "space-pull",
                    Some(AccountSpaceCommand::Delete { .. }) => "space-delete",
                },
                Some(AccountCommand::Sync { .. }) => "sync",
                Some(AccountCommand::Devices { .. }) => "devices",
                Some(AccountCommand::Revoke { .. }) => "revoke",
            }),
        ),
        Command::Eval(_) => ("eval", None),
        Command::Show { .. } => ("show", None),
        Command::Query { .. } => ("query", None),
        Command::Assert { .. } => ("assert", None),
        Command::Retract { .. } => ("retract", None),
        Command::Migrate { command } => (
            "migrate",
            Some(match command {
                MigrateCommand::Carry { .. } => "carry",
                MigrateCommand::Account => "account",
            }),
        ),
        Command::Export { .. } => ("export", None),
        Command::Render { .. } => ("render", None),
        Command::Import { .. } => ("import", None),
        Command::Push => ("push", None),
        Command::Pull => ("pull", None),
        Command::Status { .. } => ("status", None),
        Command::Invite { .. } => ("invite", None),
        Command::Join { .. } => ("join", None),
        Command::Remote { command, .. } => (
            "remote",
            Some(match command {
                None => "list",
                Some(RemoteCommand::Add { .. }) => "add",
                Some(RemoteCommand::SetUpstream { .. }) => "set-upstream",
            }),
        ),
        Command::Concept { command, .. } => (
            "concept",
            Some(match command {
                None => "list",
                Some(ConceptCommand::Add { .. }) => "add",
            }),
        ),
        Command::View { command, .. } => (
            "view",
            Some(match command {
                None => "list",
                Some(ViewCommand::Add { .. }) => "add",
            }),
        ),
        Command::Telemetry { .. } => ("telemetry", None),
        Command::Update { .. } => ("update", None),
        Command::Blob { command, .. } => (
            "blob",
            Some(match command {
                None => "list",
                Some(BlobCommand::Add { .. }) => "add",
                Some(BlobCommand::Cat { .. }) => "cat",
            }),
        ),
    }
}

fn account_command_kind(
    command: &Command,
) -> Option<tonk_cli::account_observability::AccountCommandKind> {
    use tonk_cli::account_observability::AccountCommandKind as Kind;
    let Command::Account { command, .. } = command else {
        return None;
    };
    Some(match command {
        None | Some(AccountCommand::Status { .. }) => Kind::Status,
        Some(AccountCommand::Login { .. }) => Kind::Login,
        Some(AccountCommand::Logout) => Kind::Logout,
        Some(AccountCommand::Delete { .. }) => Kind::Delete,
        Some(AccountCommand::Space { command: None, .. }) => Kind::SpaceList,
        Some(AccountCommand::Space {
            command: Some(AccountSpaceCommand::Pull { .. }),
            ..
        }) => Kind::SpacePull,
        Some(AccountCommand::Space {
            command: Some(AccountSpaceCommand::Delete { .. }),
            ..
        }) => Kind::SpaceDelete,
        Some(AccountCommand::Sync { .. }) => Kind::Sync,
        Some(AccountCommand::Devices { .. }) => Kind::Devices,
        Some(AccountCommand::Revoke { .. }) => Kind::Revoke,
    })
}

/// Whether a command opens the active space and should name it again
/// if the operation fails.
fn uses_active_space(command: &Command) -> bool {
    matches!(
        command,
        Command::Eval(_)
            | Command::Show { .. }
            | Command::Query { .. }
            | Command::Assert { .. }
            | Command::Retract { .. }
            | Command::Export { .. }
            | Command::Render { .. }
            | Command::Import { .. }
            | Command::Push
            | Command::Pull
            | Command::Status { .. }
            | Command::Invite { .. }
            | Command::Remote { .. }
            | Command::Blob { .. }
            | Command::Concept { .. }
            | Command::View { .. }
    )
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) if error.to_string().to_ascii_lowercase().contains("spot") => {
            eprintln!(
                "error: a retired space command or option was supplied; use `tonk space --help`"
            );
            std::process::exit(ExitCode::ParseError.into_raw());
        }
        Err(error) => error.exit(),
    };
    let Some(command) = cli.command else {
        print!("{CLI_INDEX}");
        return;
    };
    for (retired, replacement) in [
        ("TONK_SPOT", "TONK_SPACE"),
        ("TONK_SPOTS_STATE", "TONK_SPACES_STATE"),
    ] {
        if std::env::var_os(retired).is_some() {
            eprintln!(
                "error: a retired space environment variable is set; unset it and use {replacement}"
            );
            std::process::exit(ExitCode::ParseError.into_raw());
        }
    }
    VERBOSE.store(cli.verbose, std::sync::atomic::Ordering::Relaxed);
    // `TONK_TRACE=1` turns on the tracing subscriber on stderr. This is
    // the diagnostic for "the remote did not answer": hyper and reqwest
    // emit request-level events, so a stalled command explains itself in
    // a log rather than in a bounded timeout. `RUST_LOG` overrides the
    // filter; without it everything logs at debug — a trace that needs a
    // second variable to say anything is a trap.
    if std::env::var_os("TONK_TRACE").is_some_and(|value| !value.is_empty() && value != "0") {
        let _ = tracing_log::LogTracer::init();
        let _ = tracing_subscriber::fmt()
            .with_env_filter(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("debug")),
            )
            .with_writer(std::io::stderr)
            .with_target(true)
            .try_init();
    }
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
        recorder.property("format", if args.json { "json" } else { "notation" });
        recorder.property("dry_run", args.dry_run);
        recorder.property("quiet", args.quiet);
    }

    let started = std::time::Instant::now();
    let account_kind = account_command_kind(&command);
    let mut account_attempt = account_kind.map(|kind| {
        tonk_cli::account_observability::CliAccountAttempt::start(
            kind,
            tonk_analytics::account::AccountState::Unknown,
        )
    });
    // `command` is moved by the dispatch below, so ask now.
    let is_update = matches!(&command, Command::Update { .. });
    let report_active_space = uses_active_space(&command);
    let space = cli.space;
    let exit = match command {
        Command::Help { all, guides, name } => print_help(all, guides, name.as_deref()),
        Command::Space { command, json } => space_op(command, json, space.as_deref()).await,
        Command::Identity { reset } => identity(reset).await,
        Command::Account { command, json } => {
            account_op(command, json, account_attempt.as_mut()).await
        }
        Command::Eval(args) => eval(args, space.as_deref()).await,
        Command::Show {
            name,
            entity,
            json,
            notation,
        } => show_op(name, entity, json, notation, space.as_deref()).await,
        Command::Query { concept, json } => query_op(concept, json, space.as_deref()).await,
        Command::Assert { concept, rest } => assert_cmd(concept, rest, space.as_deref()).await,
        Command::Retract {
            concept,
            entity,
            field,
            notation,
            write,
        } => retract_op(concept, entity, field, notation, write, space.as_deref()).await,
        Command::Migrate { command } => match command {
            MigrateCommand::Carry { from, do_move } => migrate(from, do_move).await,
            MigrateCommand::Account => migrate_account().await,
        },
        Command::Export { out, branch } => export_op(out, &branch, space.as_deref()).await,
        Command::Render { route, out } => render_op(route, out, space.as_deref()).await,
        Command::Import {
            file,
            branch,
            write,
        } => import_op(file, &branch, write, space.as_deref()).await,
        Command::Push => sync_op(SyncOp::Push, space.as_deref()).await,
        Command::Pull => sync_op(SyncOp::Pull, space.as_deref()).await,
        Command::Status { json } => status_op(json, space.as_deref()).await,
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
                space.as_deref(),
            )
            .await
        }
        Command::Join { url, name } => claim_invite(url, name, space.as_deref()).await,
        Command::Remote { command, json } => remote_op(command, json, space.as_deref()).await,
        Command::Blob { command, json } => blob_op(command, json, space.as_deref()).await,
        Command::Concept { command, json } => concept_op(command, json, space.as_deref()).await,
        Command::View { command, json } => view_op(command, json, space.as_deref()).await,
        Command::Telemetry { action } => telemetry_op(action),
        Command::Update {
            disable_check,
            enable_check,
        } => update(disable_check, enable_check).await,
    };
    if exit != ExitCode::Success && report_active_space {
        print_active_space_context(space.as_deref());
    }

    let duration = started.elapsed();

    if let Some(attempt) = account_attempt.as_mut()
        && !attempt.is_finished()
    {
        use tonk_analytics::account::{AccountOutcome, FailureKind, Stage};
        let outcome = if exit == ExitCode::Success {
            attempt.success_outcome()
        } else {
            AccountOutcome::terminal_failure(FailureKind::Unknown)
        };
        attempt.finish(Stage::Complete, outcome);
    }

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
        Some(mut recorder) => {
            if let Some(attempt) = account_attempt {
                recorder.account_events(attempt.into_events());
            }
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
/// nothing backed up what it created. `tonk account login` runs the same
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
                Ok(Some(root)) => println!("account: {}", root.root_did),
                Ok(None) => println!("account: missing (run `tonk account login`)"),
                Err(error) => return print_failure(error),
            }
            ExitCode::Success
        }
        Err(err) => print_failure(err),
    }
}

/// The account section of the context report, from a read the caller
/// already performed.
///
/// One function so `tonk account status` and `tonk status` cannot report
/// the same device differently.
fn account_context(status: &account::AccountStatus) -> context::AccountContext {
    match status {
        account::AccountStatus::MissingRoot { device_did } => context::AccountContext {
            signed_in: false,
            account: None,
            account_service: None,
            device: Some(device_did.clone()),
            state: None,
        },
        account::AccountStatus::Unregistered {
            root_did,
            device_did,
        } => context::AccountContext {
            signed_in: false,
            account: Some(root_did.clone()),
            account_service: None,
            device: Some(device_did.clone()),
            state: None,
        },
        account::AccountStatus::Registered {
            root_did,
            device_did,
            provider,
            account_state,
        } => context::AccountContext {
            signed_in: true,
            account: Some(root_did.clone()),
            account_service: Some(provider.clone()),
            device: Some(device_did.clone()),
            state: Some(account_state_label(*account_state).to_string()),
        },
    }
}

/// The account section when the profile itself cannot be read.
///
/// `tonk status` reports orientation and must not fail because the
/// account is unreadable; the space it is describing works signed out.
fn account_context_unavailable() -> context::AccountContext {
    context::AccountContext {
        signed_in: false,
        account: None,
        account_service: None,
        device: None,
        state: None,
    }
}

/// The sync section, fetching the upstream head to classify against it.
fn sync_context(status: sync::SyncStatus) -> context::SyncContext {
    context::SyncContext::fetched(status.state, status.hash.map(|hash| hash.to_string()))
}

/// `tonk space agents` — read or update claim-backed space instructions.
async fn agents_op(command: Option<AgentsCommand>, space: Option<&str>) -> ExitCode {
    let (_, site) = match open_selected(space).await {
        Ok(opened) => opened,
        Err(code) => return code,
    };
    match command.unwrap_or(AgentsCommand::Get { json: false }) {
        AgentsCommand::Get { json } => {
            let claim = match agents::get(&site).await {
                Ok(Some(claim)) => claim,
                Ok(None) => {
                    return print_error(
                        "this space has no AGENTS.md claim\ncreate one: tonk space agents set AGENTS.md",
                    );
                }
                Err(err) => return print_error(format!("could not read AGENTS.md claim: {err:#}")),
            };
            let rendered = if json {
                match serde_json::to_string_pretty(&Rows::new("tonk.agents-get.v1", vec![claim])) {
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
        AgentsCommand::Set { path, write } => {
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
            match agents::set(&site, &markdown, write.into()).await {
                Ok(Some(claim)) => {
                    if write.quiet {
                        println!("asserted AGENTS.md claim");
                    } else {
                        println!(
                            "asserted AGENTS.md claim\nsource: {} {}\nentity: {}\nrevision: {}\nnext: tonk space agents get --json",
                            claim.source, claim.attribute, claim.entity, claim.revision
                        );
                    }
                    ExitCode::Success
                }
                Ok(None) => {
                    println!(
                        "dry run — nothing committed\nwould have asserted the AGENTS.md claim"
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

fn account_login_warning(status: tonk_account::AccountStateStatus, warning: &str) -> String {
    match status {
        tonk_account::AccountStateStatus::Ready => {
            format!("warning: latest account synchronization is incomplete: {warning}")
        }
        tonk_account::AccountStateStatus::Unconfigured
        | tonk_account::AccountStateStatus::Unhydrated => {
            format!("warning: account repository is not synchronized: {warning}")
        }
    }
}

/// Best-effort registration line, quiet about being offline: status
/// must answer without the network. Registration itself is web-only —
/// the browser enrolls during its passkey ceremonies, which is where
/// the account-signed deposits come from — so this only reads state
/// and points at the account page when something is missing.
async fn print_customer_line(
    profile: &dialog_operator::Profile,
    store: &tonk_cli::space::SpaceStore,
) {
    if let Some(line) = customer_state(profile, store).await.line() {
        println!("access service: {line}");
    }
}

/// `tonk account status --json`.
///
/// One flat object across all three states rather than a tagged union: a
/// caller asking "am I signed in, and to what" should not have to branch on
/// a discriminant to find out.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct AccountStatusReport {
    schema_version: &'static str,
    /// Flattened rather than nested: this command's whole subject is the
    /// account, so a `account.signedIn` path would only repeat the name of
    /// the command. Inside `tonk status` the same section is nested,
    /// where it sits beside `space` and `sync` and the name distinguishes.
    #[serde(flatten)]
    account: context::AccountContext,
    /// Access-service registration, or `None` when it could not be read.
    /// Beyond the shared section: only this command reads it.
    access_service: Option<String>,
}

const ACCOUNT_STATUS_SCHEMA_VERSION: &str = "tonk.account-status.v1";

/// The same facts the account section and [`print_customer_line`]
/// print, as one structured record.
async fn account_status_json(
    profile: &dialog_operator::Profile,
    store: &tonk_cli::space::SpaceStore,
    status: &account::AccountStatus,
) -> AccountStatusReport {
    let access_service = match status {
        account::AccountStatus::Registered { .. } => customer_state(profile, store).await.token(),
        _ => None,
    };
    AccountStatusReport {
        schema_version: ACCOUNT_STATUS_SCHEMA_VERSION,
        account: account_context(status),
        access_service,
    }
}

/// Access-service registration as one token, or `None` when the answer
/// could not be read — which is not the same as "not registered".
async fn customer_state(
    profile: &dialog_operator::Profile,
    store: &tonk_cli::space::SpaceStore,
) -> CustomerState {
    use tonk_account::customer::CustomerStatus;
    match tonk_cli::customer::registration_state_in(profile, store).await {
        Ok(Some(Some(receipt))) => match receipt.status {
            CustomerStatus::Active => CustomerState::Registered,
            CustomerStatus::Registered => CustomerState::AwaitingEmailConfirmation,
            CustomerStatus::Suspended => CustomerState::Suspended,
        },
        Ok(Some(None)) => {
            let page = tonk_cli::customer::access_origin_in(profile, store)
                .await
                .ok()
                .flatten()
                .map(|origin| format!("{origin}account"))
                .unwrap_or_else(|| "the account page".to_string());
            CustomerState::NotRegistered { page }
        }
        Ok(None) => CustomerState::Absent,
        Err(_) => CustomerState::Unreachable,
    }
}

/// One access-service state with both its stable JSON token and text copy.
enum CustomerState {
    Registered,
    AwaitingEmailConfirmation,
    Suspended,
    NotRegistered { page: String },
    Absent,
    Unreachable,
}

impl CustomerState {
    fn token(&self) -> Option<String> {
        match self {
            Self::Registered => Some("registered".to_owned()),
            Self::AwaitingEmailConfirmation => Some("awaiting-email-confirmation".to_owned()),
            Self::Suspended => Some("suspended".to_owned()),
            Self::NotRegistered { .. } => Some("not-registered".to_owned()),
            Self::Absent | Self::Unreachable => None,
        }
    }

    fn line(&self) -> Option<String> {
        match self {
            Self::Registered => Some("registered".to_owned()),
            Self::AwaitingEmailConfirmation => {
                Some("waiting for email confirmation (check your inbox)".to_owned())
            }
            Self::Suspended => Some("suspended".to_owned()),
            Self::NotRegistered { page } => Some(format!(
                "not registered (open {page} in your browser to finish setup)"
            )),
            Self::Absent => None,
            Self::Unreachable => Some("unreachable".to_owned()),
        }
    }
}

/// `tonk account login` — run the browser ceremony and record the one account
/// this installation is signed into.
///
/// Refuses while another account is still signed in: one account at a time is
/// the whole model, and silently swapping would leave the spaces of the
/// account being replaced looking broken rather than simply not-yours.
///
/// The registry row is written whether or not the deployment answers with
/// its content endpoints. By the time they are asked for, the grant is
/// installed and the session active, so refusing to record the account
/// would leave `status` and the registry disagreeing about whether this
/// device is signed in.
async fn link_account(
    store: &tonk_cli::space::SpaceStore,
    name: Option<String>,
    no_open: bool,
    via: Option<String>,
    mut observer: Option<&mut tonk_cli::account_observability::CliAccountAttempt>,
) -> ExitCode {
    let signed_in = match store.account() {
        Ok(account) => account,
        Err(error) => return print_failure(error),
    };
    if let Some(account) = &signed_in
        && matches!(
            tonk_cli::account::sign_in_phase(store),
            Ok(tonk_cli::account::SignInPhase::Active)
        )
    {
        if let Some(observer) = observer.as_deref_mut() {
            observer.finish(
                tonk_analytics::account::Stage::LocalPreflight,
                tonk_analytics::account::AccountOutcome::blocked(
                    tonk_analytics::account::FailureKind::Conflict,
                ),
            );
        }
        return print_error(format!(
            "already signed in as {}\nrun `tonk account logout` first to sign in as another account",
            account.root
        ));
    }
    let profile = match identity::open().await {
        Ok(profile) => profile,
        Err(error) => return print_failure(error),
    };
    let ceremony_page = via
        .clone()
        .unwrap_or_else(|| account::DEFAULT_LINK_PAGE.to_owned());
    let options = account::LinkOptions {
        device_name: name.unwrap_or_else(account::default_device_name),
        open_browser: !no_open,
        via,
        announce: None,
        store: Some(store.clone()),
    };
    let linked = match observer.as_deref_mut() {
        Some(observer) => account::link_in_observed(&profile, store, &options, observer).await,
        None => account::link_in(&profile, store, &options).await,
    };
    match linked {
        Ok(outcome) => {
            // The deployment that served the ceremony page is the one
            // whose endpoints this account uses.
            if let Some(observer) = observer.as_deref_mut() {
                observer.checkpoint(tonk_analytics::account::Stage::ContentDiscovery);
            }
            let discovery = tonk_cli::deployment::discover(&ceremony_page).await;
            let record = tonk_cli::deployment::account_record(
                &outcome.root_did,
                &ceremony_page,
                discovery.as_ref().ok(),
            );
            if let Err(error) = store.set_account(Some(record)) {
                if let Some(observer) = observer.as_deref_mut() {
                    observer.finish(
                        tonk_analytics::account::Stage::ActivationStage,
                        tonk_analytics::account::AccountOutcome::unknown_commit(
                            tonk_analytics::account::FailureKind::LocalState,
                        ),
                    );
                }
                return print_failure(error);
            }
            println!(
                "signed in\naccount: {}\ndevice: {}\nstatus: {}",
                outcome.root_did,
                outcome.device_did,
                account_state_label(outcome.account_state)
            );
            if let Some(warning) = outcome.warning {
                eprintln!("{}", account_login_warning(outcome.account_state, &warning));
            }
            // The device is signed in either way. Say what is missing and
            // what restores it, rather than reporting a failure for a link
            // the account service has already granted.
            if let Err(error) = discovery {
                if let Some(observer) = observer.as_deref_mut() {
                    observer.degraded(tonk_analytics::account::DegradationKind::ContentDiscovery);
                }
                eprintln!(
                    "warning: {ceremony_page} did not answer with its content endpoints: {error:#}"
                );
                eprintln!(
                    "spaces stay local-only until `tonk account logout` and `tonk account login` reach it"
                );
            }
            // Custody moves with the sign-in, the way browser accreditation
            // rotates: every local space this account may own gets its
            // authority and sealed seed onto the account. Hosting does not
            // move — `tonk space link` remains the boundary that provisions
            // and attaches a remote.
            if let Some(observer) = observer.as_deref_mut() {
                observer.checkpoint(tonk_analytics::account::Stage::CustodyRotation);
            }
            match site::default_config() {
                Ok(config) => {
                    match tonk_cli::custody::rotate_from_onboarding(store, &config).await {
                        Ok(failures) => {
                            if !failures.is_empty()
                                && let Some(observer) = observer.as_deref_mut()
                            {
                                observer.degraded(
                                    tonk_analytics::account::DegradationKind::CustodyRotation,
                                );
                            }
                            for (subject, reason) in failures {
                                eprintln!("rotation: {subject} not rotated: {reason}");
                            }
                        }
                        Err(error) => {
                            if let Some(observer) = observer.as_deref_mut() {
                                observer.degraded(
                                    tonk_analytics::account::DegradationKind::CustodyRotation,
                                );
                            }
                            eprintln!("warning: account rotation did not run: {error:#}")
                        }
                    }
                    match tonk_cli::custody::rotate_local_spaces(store, &config).await {
                        Ok(outcomes) => {
                            if outcomes.iter().any(|(_, outcome)| {
                                matches!(outcome, tonk_cli::custody::SpaceRotation::Skipped(_))
                            }) && let Some(observer) = observer.as_deref_mut()
                            {
                                observer.degraded(
                                    tonk_analytics::account::DegradationKind::SpaceRotation,
                                );
                            }
                            for (name, outcome) in outcomes {
                                match outcome {
                                    tonk_cli::custody::SpaceRotation::Moved => {
                                        println!("custody: '{name}' moved to the account");
                                    }
                                    tonk_cli::custody::SpaceRotation::Already => {}
                                    tonk_cli::custody::SpaceRotation::Skipped(reason) => {
                                        eprintln!("custody: '{name}' not moved: {reason}");
                                    }
                                }
                            }
                        }
                        Err(error) => {
                            if let Some(observer) = observer.as_deref_mut() {
                                observer.degraded(
                                    tonk_analytics::account::DegradationKind::SpaceRotation,
                                );
                            }
                            eprintln!("warning: space custody did not move: {error:#}")
                        }
                    }
                }
                Err(error) => {
                    if let Some(observer) = observer.as_deref_mut() {
                        observer.degraded(tonk_analytics::account::DegradationKind::SpaceRotation);
                    }
                    eprintln!("warning: space custody did not move: {error:#}")
                }
            }
            print_customer_line(&profile, store).await;
            ExitCode::Success
        }
        Err(error) => {
            if let Some(observer) = observer {
                let classified = error
                    .downcast_ref::<tonk_cli::callback::CallbackFailure>()
                    .map(|callback| {
                        use tonk_cli::callback::CallbackFailureKind;
                        match callback.kind() {
                            CallbackFailureKind::Bind => (
                                tonk_analytics::account::Stage::CallbackBind,
                                tonk_analytics::account::AccountOutcome::retryable(
                                    tonk_analytics::account::FailureKind::Callback,
                                ),
                            ),
                            CallbackFailureKind::Timeout => (
                                tonk_analytics::account::Stage::CallbackWait,
                                tonk_analytics::account::AccountOutcome::retryable(
                                    tonk_analytics::account::FailureKind::Timeout,
                                ),
                            ),
                            CallbackFailureKind::Closed | CallbackFailureKind::Server => (
                                tonk_analytics::account::Stage::CallbackWait,
                                tonk_analytics::account::AccountOutcome::retryable(
                                    tonk_analytics::account::FailureKind::Callback,
                                ),
                            ),
                        }
                    })
                    .or_else(|| {
                        error
                            .downcast_ref::<tonk_cli::account::BrowserAuthorizationDenied>()
                            .map(|_| {
                                (
                                    tonk_analytics::account::Stage::CallbackWait,
                                    tonk_analytics::account::AccountOutcome::blocked(
                                        tonk_analytics::account::FailureKind::AccessDenied,
                                    ),
                                )
                            })
                    })
                    .or_else(|| {
                        error
                            .downcast_ref::<tonk_cli::account::LinkCancelled>()
                            .map(|_| {
                                (
                                    tonk_analytics::account::Stage::CallbackWait,
                                    tonk_analytics::account::AccountOutcome::cancelled(),
                                )
                            })
                    });
                let (stage, outcome) = classified.unwrap_or_else(|| {
                    let stage = observer.last_stage();
                    let outcome = match stage {
                        tonk_analytics::account::Stage::DelegationValidate => {
                            tonk_analytics::account::AccountOutcome::terminal_failure(
                                tonk_analytics::account::FailureKind::InvalidResponse,
                            )
                        }
                        tonk_analytics::account::Stage::ActivationStage
                        | tonk_analytics::account::Stage::AccountSync => {
                            tonk_analytics::account::AccountOutcome::unknown_commit(
                                tonk_analytics::account::FailureKind::LocalState,
                            )
                        }
                        _ => tonk_analytics::account::AccountOutcome::terminal_failure(
                            tonk_analytics::account::FailureKind::LocalState,
                        ),
                    };
                    (stage, outcome)
                });
                observer.finish(stage, outcome);
            }
            print_failure(error)
        }
    }
}

async fn account_op(
    command: Option<AccountCommand>,
    json: bool,
    mut observer: Option<&mut tonk_cli::account_observability::CliAccountAttempt>,
) -> ExitCode {
    let command = command.unwrap_or(AccountCommand::Status { json });
    let store = match tonk_cli::space::SpaceStore::open() {
        Ok(store) => store,
        Err(error) => return print_failure(error),
    };
    if let AccountCommand::Login { name, no_open, via } = command {
        return link_account(&store, name, no_open, via, observer.as_deref_mut()).await;
    }
    if matches!(command, AccountCommand::Space { .. }) && matches!(store.account(), Ok(None)) {
        return print_error("no account is signed in; run `tonk account login`".to_owned());
    }
    let profile = match identity::open().await {
        Ok(profile) => profile,
        Err(error) => return print_failure(error),
    };
    match command {
        AccountCommand::Login { .. } => unreachable!("handled above"),
        AccountCommand::Status { json } => match account::status_in(&profile, &store).await {
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
                    match tokio::time::timeout(std::time::Duration::from_secs(10), async {
                        let operator =
                            tonk_cli::account_state::operator_for_store(&profile, &store).await?;
                        tonk_cli::account_state::ensure_with_operator_and_store(
                            &profile,
                            operator,
                            store.clone(),
                        )
                        .await
                    })
                    .await
                    {
                        Ok(Ok(outcome)) => {
                            if let Some(warning) = outcome.warning {
                                if let Some(observer) = observer.as_deref_mut() {
                                    observer.degraded(
                                        tonk_analytics::account::DegradationKind::AccountSync,
                                    );
                                }
                                eprintln!("warning: account sync attempt: {warning}");
                            }
                            if let Ok(fresh) = account::status_in(&profile, &store).await {
                                status = fresh;
                            }
                        }
                        Ok(Err(error)) => {
                            if let Some(observer) = observer.as_deref_mut() {
                                observer.degraded(
                                    tonk_analytics::account::DegradationKind::AccountSync,
                                );
                            }
                            eprintln!("warning: account sync attempt: {error:#}")
                        }
                        Err(_) => {
                            if let Some(observer) = observer.as_deref_mut() {
                                observer.degraded(
                                    tonk_analytics::account::DegradationKind::AccountSync,
                                );
                            }
                            eprintln!("warning: account sync attempt timed out")
                        }
                    }
                }
                if json {
                    return print_json(&account_status_json(&profile, &store, &status).await);
                }
                let linked = matches!(status, account::AccountStatus::Registered { .. });
                print!("{}", account_context(&status).render());
                if linked {
                    print_customer_line(&profile, &store).await;
                }
                ExitCode::Success
            }
            Err(error) => print_failure(error),
        },
        AccountCommand::Logout => {
            let mut noop = tonk_cli::account_observability::NoopAccountObserver;
            let observed: &mut dyn tonk_cli::account_observability::CliAccountObserver =
                match observer.as_deref_mut() {
                    Some(observer) => observer,
                    None => &mut noop,
                };
            match account::logout_in_observed(&profile, &store, observed).await {
                Ok(()) => {
                    // The spaces themselves keep their account tag: logging out
                    // is not disowning them, and this device stays able to work
                    // on every replica it already holds.
                    if let Err(error) = store.set_account(None) {
                        if let Some(observer) = observer.as_deref_mut() {
                            observer.finish(
                                tonk_analytics::account::Stage::LocalCommit,
                                tonk_analytics::account::AccountOutcome::unknown_commit(
                                    tonk_analytics::account::FailureKind::LocalState,
                                ),
                            );
                        }
                        return print_failure(error);
                    }
                    println!("signed out\ndevice: {}", profile.did());
                    ExitCode::Success
                }
                Err(error) => print_failure(error),
            }
        }
        AccountCommand::Delete {
            account_url,
            no_open,
        } => match account::open_deletion(&profile, &account_url, !no_open).await {
            Ok(url) => {
                println!("Review permanent account deletion in your browser:\n{url}");
                println!(
                    "No data has been deleted yet. The browser will list owned spaces, leave joined spaces intact, and require your email plus passkey."
                );
                ExitCode::Success
            }
            Err(error) => print_failure(error),
        },
        AccountCommand::Space { command, json } => match command {
            None => match account_spaces::list(&profile, &store).await {
                Ok(rows) if json => print_json(&account_spaces_report(rows)),
                Ok(rows) => {
                    let mut listing = Listing::new(
                        &["STATE", "NAME", "SUBJECT"],
                        "no spaces listed in the account directory",
                    );
                    for row in &rows {
                        let state = if row.ambiguous {
                            "ambiguous"
                        } else if row.local_name.is_some() {
                            "local"
                        } else {
                            "remote"
                        };
                        listing.push([
                            state.to_owned(),
                            listing::cell(row.remote_name.as_deref().or(row.local_name.as_deref())),
                            row.subject.clone(),
                        ]);
                    }
                    println!("{}", listing.render());
                    ExitCode::Success
                }
                Err(error) => print_failure(error),
            },
            Some(AccountSpaceCommand::Pull {
                name_or_subject,
                name,
            }) => {
                match account_spaces::pull(&profile, &store, &name_or_subject, name.as_deref())
                    .await
                {
                    Ok(outcome) => {
                        if outcome.already_local {
                            println!("already local\t{}\t{}", outcome.name, outcome.subject);
                        } else {
                            println!("pulled\t{}\t{}", outcome.name, outcome.subject);
                            println!("site: {}", outcome.site.display());
                        }
                        if let Some(warning) = outcome.warning {
                            if let Some(observer) = observer.as_deref_mut() {
                                observer.degraded(
                                    tonk_analytics::account::DegradationKind::AccountSync,
                                );
                            }
                            eprintln!("warning: {warning}");
                        }
                        ExitCode::Success
                    }
                    Err(error) => print_failure(error),
                }
            }
            Some(AccountSpaceCommand::Delete {
                subject,
                account_url,
                no_open,
            }) => match account::open_space_deletion(&profile, &account_url, &subject, !no_open)
                .await
            {
                Ok(url) => {
                    println!("Review permanent deletion of {subject} in your browser:\n{url}");
                    println!(
                        "No data has been deleted yet. Your account and every other space will remain; the browser requires an explicit typed confirmation."
                    );
                    ExitCode::Success
                }
                Err(error) => print_failure(error),
            },
        },
        AccountCommand::Devices { json } => match account::devices_in(&profile, &store).await {
            Ok(rows) => {
                let own = profile.did().to_string();
                if json {
                    let rows: Vec<_> = rows
                        .into_iter()
                        .map(|row| DeviceRow {
                            status: "active".to_owned(),
                            name: row.name,
                            did: row.did.clone(),
                            this_device: row.did == own,
                        })
                        .collect();
                    return print_json(&account_devices_report(rows));
                }
                let mut listing = Listing::new(
                    &["STATUS", "NAME", "DID", "THIS"],
                    "no devices are linked to this account",
                );
                for row in &rows {
                    listing.push([
                        "active".to_owned(),
                        row.name.clone(),
                        row.did.clone(),
                        if row.did == own { "yes" } else { "no" }.to_owned(),
                    ]);
                }
                println!("{}", listing.render());
                ExitCode::Success
            }
            Err(error) => print_failure(error),
        },
        AccountCommand::Sync { verbose } => {
            if verbose {
                tonk_cli::account_state::enable_progress();
            }
            match account::sync(&profile).await {
                Ok(outcome) => {
                    if let Some(warning) = outcome.warning {
                        if let Some(observer) = observer.as_deref_mut() {
                            observer
                                .degraded(tonk_analytics::account::DegradationKind::AccountSync);
                        }
                        eprintln!("warning: {warning}");
                    }
                    println!("account: {:?}", outcome.status);
                    ExitCode::Success
                }
                Err(error) => print_failure(error),
            }
        }
        AccountCommand::Revoke { did } => {
            let mut noop = tonk_cli::account_observability::NoopAccountObserver;
            let observed: &mut dyn tonk_cli::account_observability::CliAccountObserver =
                match observer.as_deref_mut() {
                    Some(observer) => observer,
                    None => &mut noop,
                };
            match account::revoke_in_observed(&profile, &store, &did, observed).await {
                Ok(account::RevokeOutcome::Revoked) => {
                    if let Some(observer) = observer.as_deref_mut() {
                        observer.finish(
                            tonk_analytics::account::Stage::RemoteCommit,
                            tonk_analytics::account::AccountOutcome::success(),
                        );
                    }
                    println!("revoked\ndevice: {did}");
                    ExitCode::Success
                }
                Ok(account::RevokeOutcome::AlreadyRevoked) => {
                    if let Some(observer) = observer.as_deref_mut() {
                        observer.finish(
                            tonk_analytics::account::Stage::LocalPreflight,
                            tonk_analytics::account::AccountOutcome::success(),
                        );
                    }
                    println!("already revoked\ndevice: {did}");
                    ExitCode::Success
                }
                Err(error) => {
                    if let Some(observer) = observer {
                        let stage = observer.last_stage();
                        let outcome = if stage == tonk_analytics::account::Stage::RemoteCommit {
                            tonk_analytics::account::AccountOutcome::unknown_commit(
                                tonk_analytics::account::FailureKind::Unknown,
                            )
                        } else {
                            tonk_analytics::account::AccountOutcome::terminal_failure(
                                tonk_analytics::account::FailureKind::Unknown,
                            )
                        };
                        observer.finish(stage, outcome);
                    }
                    print_failure(error)
                }
            }
        }
    }
}

async fn record_space_best_effort(name: &str, site: &site::TonkSite) {
    if let Err(error) = account_spaces::record_site_in(name, site, &site.account_store).await {
        eprintln!("warning: account directory update failed: {error:#}");
    }
}

/// `tonk space use <name>` — bind this directory to a registered space.
async fn use_op(name: String, flag: Option<&str>) -> ExitCode {
    let store = match tonk_cli::space::SpaceStore::open() {
        Ok(store) => store,
        Err(err) => return print_failure(err),
    };
    let cwd = working_directory();
    let Some(cwd) = cwd else {
        return print_error("could not read the current directory".to_owned());
    };
    match tonk_cli::space::bind(&store, &name, &cwd) {
        Ok(outcome) => {
            let was = outcome
                .previous
                .filter(|previous| previous != &name)
                .map(|previous| format!(" (was {previous})"))
                .unwrap_or_default();
            println!(
                "binding: {name}{was}\ndirectory: {directory}",
                directory = outcome.directory.display(),
            );
            print_active_space_resolution(&store, flag, Some(&cwd));
            println!("next: tonk status");
            ExitCode::Success
        }
        Err(err) => print_failure(err),
    }
}

/// Bare `tonk space` lists; subcommands create, bind, link, or remove.
async fn space_op(command: Option<SpaceCommand>, json: bool, flag: Option<&str>) -> ExitCode {
    let command = match command {
        Some(SpaceCommand::Use { name }) => return use_op(name, flag).await,
        Some(SpaceCommand::Home {
            models,
            notation,
            write,
        }) => return home_op(models, notation, write, flag).await,
        Some(SpaceCommand::Agents { command }) => return agents_op(command, flag).await,
        command => command,
    };
    let store = match tonk_cli::space::SpaceStore::open() {
        Ok(store) => store,
        Err(err) => return print_failure(err),
    };
    let config = match site::default_config() {
        Ok(config) => config,
        Err(error) => return print_failure(error),
    };
    match command {
        Some(
            SpaceCommand::Use { .. } | SpaceCommand::Home { .. } | SpaceCommand::Agents { .. },
        ) => {
            unreachable!("taken above")
        }
        Some(SpaceCommand::New { name, site }) => {
            // Signed in, a new space is the account's from birth: it is
            // provisioned, pushed, and listed for the account's other
            // devices. Signed out, it is local-only until `tonk space link`
            // says otherwise.
            let account = match account_for_new_space(&store).await {
                Ok(account) => account,
                Err(exit) => return exit,
            };
            let Some(cwd) = working_directory() else {
                return print_error("could not read the current directory".to_owned());
            };
            let mut create_config = config.clone();
            create_config.require_account =
                account.is_some() && std::env::var_os("TONK_UNSAFE_ALLOW_DEVICE_ROOT").is_none();
            create_config.provision_account_spaces = account.is_some();
            match tonk_cli::space::create(
                &store,
                &name,
                site.as_deref(),
                None,
                create_config.clone(),
            )
            .await
            {
                Ok(outcome) => {
                    if let Err(error) = tonk_cli::space::bind(&store, &outcome.name, &cwd) {
                        return print_failure(error);
                    }
                    if outcome.adopted {
                        println!(
                            "Registered space '{}' on the site data already at that path",
                            outcome.name
                        );
                    } else {
                        println!("Registered space '{}'", outcome.name);
                    }
                    println!("site: {}", outcome.site.display());
                    println!("DID: {}", outcome.did);
                    println!("binding: {}", cwd.display());
                    print_active_space_resolution(&store, flag, Some(&cwd));
                    let Some(account) = account else {
                        return ExitCode::Success;
                    };
                    let Some(access) = &account.access_remote else {
                        unreachable!("checked before the space was created");
                    };
                    match site::TonkSite::open_with(&outcome.site, create_config).await {
                        Ok(site) => {
                            if let Err(error) = site::record_founder_membership(&site).await {
                                return print_failure(error);
                            }
                            if let Err(error) = remote::add(
                                &site,
                                remote::DEFAULT_REMOTE,
                                access,
                                Some(site.repository.did()),
                            )
                            .await
                            {
                                return print_failure(error);
                            }
                            if let Err(error) =
                                remote::set_upstream(&site, remote::DEFAULT_REMOTE).await
                            {
                                return print_failure(error);
                            }
                            if let Err(error) = sync::push(&site).await {
                                return print_failure(error);
                            }
                            if let Err(error) =
                                account_spaces::record_site_in(&outcome.name, &site, &store).await
                            {
                                return print_failure(error);
                            }
                            println!("account: {}", account.root);
                            ExitCode::Success
                        }
                        Err(error) => print_failure(error),
                    }
                }
                Err(err) => print_failure(err),
            }
        }
        None => {
            let report = match tonk_cli::inventory::list_local(&store, &config).await {
                Ok(report) => report,
                Err(error) => return print_failure(error),
            };
            for diagnostic in &report.diagnostics {
                eprintln!("warning: {diagnostic}");
            }
            if json {
                return print_json(&Rows::new("tonk.space-list.v1", report.rows));
            }
            println!("{}", tonk_cli::inventory::render(&report.rows));
            let registry = match store.load() {
                Ok(registry) => registry,
                Err(error) => return print_failure(error),
            };
            if !registry.bindings.is_empty() {
                println!();
                println!("directories:");
                for (directory, name) in &registry.bindings {
                    println!("  {}\t{name}", directory.display());
                }
            }
            print_orphaned_sites(&store.orphaned_sites(&registry));
            ExitCode::Success
        }
        Some(SpaceCommand::Link { name }) => {
            match tonk_cli::space_link::execute(&store, &config, &name).await {
                Ok(outcome) if outcome.already_linked => {
                    println!(
                        "already linked\t{}\naccount: {}",
                        outcome.name, outcome.account
                    );
                    ExitCode::Success
                }
                Ok(outcome) => {
                    println!("linked\t{}\t{}", outcome.name, outcome.subject);
                    println!("account: {}", outcome.account);
                    println!("site: {}", outcome.site.display());
                    ExitCode::Success
                }
                Err(error) => print_failure(error),
            }
        }
        Some(SpaceCommand::Transplant { name, in_place }) => {
            // Same account gate as `space new`: signed in, the re-rooted
            // space is the account's from its first commit — custodied,
            // provisioned, and pushed under the fresh subject.
            let account = match account_for_new_space(&store).await {
                Ok(account) => account,
                Err(exit) => return exit,
            };
            let mut transplant_config = config.clone();
            transplant_config.require_account =
                account.is_some() && std::env::var_os("TONK_UNSAFE_ALLOW_DEVICE_ROOT").is_none();
            transplant_config.provision_account_spaces = account.is_some();
            match tonk_cli::space::transplant(&store, &name, in_place, transplant_config.clone())
                .await
            {
                Ok(outcome) => {
                    println!("transplanted\t{}\t{}", outcome.name, outcome.did);
                    println!("origin: {}", outcome.origin);
                    println!("site: {}", outcome.site.display());
                    if let Some(backup) = &outcome.backup {
                        println!("backup: {}", backup.display());
                    }
                    println!(
                        "Invites and member grants for the origin subject no longer \
                         apply; share fresh invites."
                    );
                    let Some(account) = account else {
                        println!(
                            "local-only: sign in and run `tonk space link {}` to host \
                             the transplanted space",
                            outcome.name
                        );
                        return ExitCode::Success;
                    };
                    let Some(access) = &account.access_remote else {
                        unreachable!("checked before the space was transplanted");
                    };
                    // Mirror `space new`: wire the account's remote for
                    // the fresh subject and push — the full re-upload
                    // that gives the new identity an upstream.
                    match site::TonkSite::open_with(&outcome.site, transplant_config).await {
                        Ok(site) => {
                            if let Err(error) = site::record_founder_membership(&site).await {
                                return print_failure(error);
                            }
                            if let Err(error) = remote::add(
                                &site,
                                remote::DEFAULT_REMOTE,
                                access,
                                Some(site.repository.did()),
                            )
                            .await
                            {
                                return print_failure(error);
                            }
                            if let Err(error) =
                                remote::set_upstream(&site, remote::DEFAULT_REMOTE).await
                            {
                                return print_failure(error);
                            }
                            if let Err(error) = sync::push(&site).await {
                                return print_failure(error);
                            }
                            if let Err(error) =
                                account_spaces::record_site_in(&outcome.name, &site, &store).await
                            {
                                return print_failure(error);
                            }
                            println!("account: {}", account.root);
                            ExitCode::Success
                        }
                        Err(error) => print_failure(error),
                    }
                }
                Err(error) => print_failure(error),
            }
        }
        Some(SpaceCommand::Rm {
            name,
            keep_data,
            yes,
        }) => space_rm(&store, &config, &name, keep_data, yes).await,
        Some(SpaceCommand::Unbind { path }) => {
            let directory = match path.or_else(working_directory) {
                Some(directory) => directory,
                None => return print_error("could not read the current directory".to_owned()),
            };
            match tonk_cli::space::unbind(&store, &directory) {
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

/// The account a fresh `tonk space new` should belong to.
///
/// A recorded account that is not actually signed in is an error rather than
/// a quiet fallback to local-only: creating a local space when the user
/// expects an account-owned one is exactly the surprise `tonk space link`
/// exists to undo.
async fn account_for_new_space(
    store: &tonk_cli::space::SpaceStore,
) -> Result<Option<tonk_cli::space::AccountRecord>, ExitCode> {
    let account = match store.account() {
        Ok(account) => account,
        Err(error) => return Err(print_failure(error)),
    };
    let Some(account) = account else {
        return Ok(None);
    };
    let profile = match identity::open().await {
        Ok(profile) => profile,
        Err(error) => return Err(print_failure(error)),
    };
    match tonk_cli::account::status_in(&profile, store).await {
        Ok(account::AccountStatus::Registered { root_did, .. }) if root_did == account.root => {}
        Ok(_) => {
            return Err(print_error(format!(
                "this device is signed out of {}; run `tonk account login`",
                account.root
            )));
        }
        Err(error) => return Err(print_failure(error)),
    }
    // Checked before the space exists: creating one and only then finding
    // out it cannot be hosted leaves a half-made thing to explain.
    if account.access_remote.is_none() {
        return Err(print_error(
            "the account has no content endpoint; sign in again".to_owned(),
        ));
    }
    Ok(Some(account))
}

fn print_active_space_resolution(
    store: &tonk_cli::space::SpaceStore,
    flag: Option<&str>,
    cwd: Option<&std::path::Path>,
) {
    let env = space_from_environment();
    match store.resolve(flag, env.as_deref(), cwd) {
        Ok(resolved) => println!(
            "active space: {name} ({source})\nsite: {site}",
            name = resolved.name,
            source = resolved.source,
            site = resolved.site.display(),
        ),
        Err(err) => {
            eprintln!("warning: binding saved, but the active space does not resolve: {err}")
        }
    }
}

/// Report canonical site data that no registered space names.
///
/// Silent when there is none, so the common listing stays clean. When
/// there is some it belongs on screen: it is otherwise entirely
/// invisible, and it is the thing that will refuse a later `tonk
/// join` or `tonk account space pull` on the same name.
fn print_orphaned_sites(orphans: &[PathBuf]) {
    if orphans.is_empty() {
        return;
    }
    println!();
    println!("unregistered site data (belongs to no space):");
    for path in orphans {
        println!("  {}", path.display());
    }
    println!("  adopt it with `tonk space new <name> --site <path>`, or delete the directory");
}

/// `tonk space rm` — delete a space's data, or (with --keep-data) just
/// its registration.
///
/// Deleting is the default because the alternative is worse: an
/// unregistered site directory is invisible to every command that
/// reads the registry, yet still holds the canonical name against
/// `tonk join --name` and `tonk account space pull --name`. Making
/// that the accident-shaped path instead of the deliberate one is
/// what this command is for.
async fn space_rm(
    store: &tonk_cli::space::SpaceStore,
    config: &site::SiteConfig,
    name: &str,
    keep_data: bool,
    yes: bool,
) -> ExitCode {
    use tonk_cli::space::{Data, Deletion};

    let registry = match store.load() {
        Ok(registry) => registry,
        Err(err) => return print_failure(err),
    };
    // Resolved up front so the confirmation can name the exact
    // directory it is about to destroy, and so an unknown name fails
    // before anything is inspected or printed.
    let Some(entry) = registry.spaces.get(name) else {
        return print_failure(tonk_cli::space::SpaceError::Unknown {
            name: name.to_owned(),
            available: registry.spaces.keys().cloned().collect(),
            binding: None,
        });
    };
    let site = entry.site.clone();

    if keep_data {
        return match tonk_cli::space::remove(store, name, Data::Keep) {
            Ok(outcome) => {
                println!("Unregistered space '{}'", outcome.name);
                println!("data kept at {}", outcome.site.display());
                println!(
                    "  it belongs to no space now: re-adopt it with \
                     `tonk space new <name> --site {}`,",
                    outcome.site.display()
                );
                println!("  or delete it with `tonk space rm <name>` after re-adopting");
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
        let recovery = tonk_cli::recovery::inspect(&site, config.clone()).await;
        println!();
        println!("This permanently deletes the space's data from disk:");
        println!("  {}", site.display());
        println!();
        println!("{}", recovery.consequence(name));
        if let Some(hint) = recovery.restore_hint() {
            println!("  {hint}");
        }
        println!();
        if !confirm_by_name(name) {
            // Non-zero: a caller chaining off `tonk space rm` must not
            // read "you declined" as "it is gone".
            println!("Aborted; nothing was deleted.");
            return ExitCode::IoError;
        }
    }

    match tonk_cli::space::remove(store, name, Data::Delete) {
        Ok(outcome) => {
            match outcome.data {
                Deletion::Deleted => {
                    println!("Deleted space '{}' and its data", outcome.name);
                    println!("removed {}", outcome.site.display());
                }
                // Nothing was destroyed, so don't say it was.
                Deletion::AlreadyGone => {
                    println!("Unregistered space '{}'", outcome.name);
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

async fn eval(args: EvalArgs, space: Option<&str>) -> ExitCode {
    let source = match resolve_source(&args) {
        Ok(s) => s,
        Err(message) => return print_error(message),
    };

    let options = eval::Options {
        format: if args.json {
            Format::Json
        } else {
            Format::Notation
        },
        quiet: args.quiet,
        dry_run: args.dry_run,
        home: args.home,
    };

    let (_, site) = match open_selected(space).await {
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

fn print_help(all: bool, guides: bool, name: Option<&str>) -> ExitCode {
    if all {
        println!("commands:");
        for command in Cli::command().get_subcommands() {
            let summary = command
                .get_about()
                .map(ToString::to_string)
                .unwrap_or_default();
            println!("  {:<11} {summary}", command.get_name());
        }
        return ExitCode::Success;
    }
    if guides {
        println!("guides:");
        for topic in guide::TOPICS {
            println!(
                "  {topic:<13} {}",
                guide::description(topic).expect("every topic has a description")
            );
        }
        return ExitCode::Success;
    }
    let Some(name) = name else {
        print!("{CLI_INDEX}");
        return ExitCode::Success;
    };
    if name == "all" {
        print!("{}", guide::GUIDE);
        return ExitCode::Success;
    }
    if let Some(text) = guide::topic(name) {
        print!("{text}");
        return ExitCode::Success;
    }
    let mut root = Cli::command();
    if let Some(command) = root.find_subcommand_mut(name) {
        if let Err(error) = command.print_long_help() {
            return print_error(format!("failed to write stdout: {error}"));
        }
        println!();
        return ExitCode::Success;
    }
    eprintln!(
        "error: no command or guide named '{name}'\ncommands: {}\nguides: {}",
        root.get_subcommands()
            .map(clap::Command::get_name)
            .collect::<Vec<_>>()
            .join(", "),
        guide::TOPICS.join(", ")
    );
    ExitCode::ParseError
}

/// Selector for the [`sync_op`] handler. Both `tonk push` and
/// `tonk pull` follow the same space-resolution + dispatch path;
/// the only thing that differs is which dialog primitive they
/// call and the verb they print on success.
#[derive(Debug, Clone, Copy)]
enum SyncOp {
    Push,
    Pull,
}

async fn sync_op(op: SyncOp, space: Option<&str>) -> ExitCode {
    let (resolved, site) = match open_selected(space).await {
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
            record_space_best_effort(&resolved.name, &site).await;
            ExitCode::Success
        }
        // The service boundary is where access is decided, so its refusal is
        // relayed here rather than pre-empted at resolution: the reason it
        // gave, verbatim, wrapped in a fix read from the roster the replica
        // already holds.
        Err(err @ sync::SyncError::Rejected { .. }) => {
            let sync::SyncError::Rejected { reason } = &err else {
                unreachable!("matched one line above")
            };
            eprintln!(
                "error: {}",
                sync::rejection_report(&site, &resolved.name, reason).await
            );
            err.exit_code()
        }
        Err(err) => print_coded(err),
    }
}

async fn export_op(out: Option<PathBuf>, branch: &str, space: Option<&str>) -> ExitCode {
    let (_, site) = match open_selected(space).await {
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
        Err(err) => print_coded(err),
    }
}

async fn render_op(route: String, out: Option<PathBuf>, space: Option<&str>) -> ExitCode {
    let parsed = match RenderRoute::parse(&route) {
        Ok(r) => r,
        Err(err) => return print_failure(err),
    };
    let (_, site) = match open_selected(space).await {
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

async fn import_op(file: PathBuf, branch: &str, write: WriteArgs, space: Option<&str>) -> ExitCode {
    let (_, site) = match open_selected(space).await {
        Ok(opened) => opened,
        Err(code) => return code,
    };

    if write.dry_run {
        return match transfer::plan_import(&file).await {
            Ok(plan) => {
                println!("dry run — nothing committed");
                if !write.quiet {
                    println!(
                        "would import {} artifact(s) from {} ({} incompatible row(s) skipped)",
                        plan.artifacts,
                        file.display(),
                        plan.skipped
                    );
                }
                ExitCode::Success
            }
            Err(err) => print_coded(err),
        };
    }

    let sync = branch == tonk_cli::site::BRANCH_NAME && auto_sync::enabled(write.no_sync);
    match auto_sync::around_commit(&site, sync, transfer::import_branch(&site, branch, &file)).await
    {
        Ok(revision) => {
            if write.quiet {
                println!("imported");
            } else {
                println!(
                    "imported {} -> revision {}",
                    file.display(),
                    revision.edition.value(),
                );
            }
            ExitCode::Success
        }
        Err(err) => print_coded(err),
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

async fn status_op(json: bool, space: Option<&str>) -> ExitCode {
    let (resolved, site) = match open_selected(space).await {
        Ok(opened) => opened,
        Err(code) => return code,
    };
    let sync = match sync::status_with_hash(&site).await {
        Ok(status) => sync_context(status),
        Err(error) => {
            eprintln!("warning: upstream status unavailable: {error}");
            match sync::status_offline(&site).await {
                Ok(sync) => sync,
                Err(error) => return print_coded(error),
            }
        }
    };
    let space = SpaceContext::new(&resolved);
    let account = match identity::open().await {
        Ok(profile) => match tonk_cli::space::SpaceStore::open() {
            Ok(store) => match account::status_in(&profile, &store).await {
                Ok(status) => account_context(&status),
                Err(error) => {
                    eprintln!("warning: account status unavailable: {error:#}");
                    account_context_unavailable()
                }
            },
            Err(error) => {
                eprintln!("warning: account status unavailable: {error:#}");
                account_context_unavailable()
            }
        },
        Err(error) => {
            eprintln!("warning: account status unavailable: {error:#}");
            account_context_unavailable()
        }
    };
    if json {
        return print_json(&StatusReport {
            schema_version: STATUS_SCHEMA_VERSION,
            space,
            sync,
            account,
        });
    }
    print!("{}{}{}", space.render(), sync.render(), account.render());
    ExitCode::Success
}

/// `tonk status --json` combines the selected space, sync, and account.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct StatusReport {
    schema_version: &'static str,
    space: SpaceContext,
    sync: context::SyncContext,
    account: context::AccountContext,
}

const STATUS_SCHEMA_VERSION: &str = "tonk.status.v2";

/// One row of `tonk account devices --json`.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct DeviceRow {
    status: String,
    name: String,
    did: String,
    /// Whether this row is the device the command ran on.
    this_device: bool,
}

fn account_spaces_report(
    rows: Vec<account_spaces::AccountSpaceRow>,
) -> Rows<account_spaces::AccountSpaceRow> {
    Rows::new("tonk.account-spaces.v1", rows)
}

fn account_devices_report(rows: Vec<DeviceRow>) -> Rows<DeviceRow> {
    Rows::new("tonk.account-devices.v1", rows)
}

/// Write `value` to stdout as the pretty JSON every `--json` read prints.
fn print_json<T: serde::Serialize>(value: &T) -> ExitCode {
    match serde_json::to_string_pretty(value) {
        Ok(text) => {
            println!("{text}");
            ExitCode::Success
        }
        Err(err) => print_error(format!("could not encode JSON: {err}")),
    }
}

fn render_revision(revision: Option<&dialog_repository::Revision>) -> String {
    match revision {
        Some(rev) => rev.tree.to_string(),
        None => "~".to_string(),
    }
}

async fn remote_op(command: Option<RemoteCommand>, json: bool, space: Option<&str>) -> ExitCode {
    let (resolved, site) = match open_selected(space).await {
        Ok(opened) => opened,
        Err(code) => return code,
    };

    match command {
        Some(RemoteCommand::Add {
            name,
            url,
            revocation_url,
            subject,
        }) => {
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
                            record_space_best_effort(&resolved.name, &site).await;
                            ExitCode::Success
                        }
                        Ok(false) => match remote::set_upstream(&site, &name).await {
                            Ok(upstream) => {
                                print_set_upstream_outcome(&upstream);
                                record_space_best_effort(&resolved.name, &site).await;
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
                Err(err) => print_coded(err),
            }
        }
        None => match remote::list(&site).await {
            Ok(records) if json => print_json(&Rows::new("tonk.remote-list.v1", records)),
            Ok(records) => {
                print_remote_list(&records);
                ExitCode::Success
            }
            Err(err) => print_coded(err),
        },
        Some(RemoteCommand::SetUpstream { remote: name }) => {
            match remote::set_upstream(&site, &name).await {
                Ok(outcome) => {
                    print_set_upstream_outcome(&outcome);
                    record_space_best_effort(&resolved.name, &site).await;
                    ExitCode::Success
                }
                Err(err) => print_coded(err),
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
    let mut listing = Listing::new(
        &["NAME", "ENDPOINT", "SUBJECT", "REVOCATION"],
        "no remotes registered; add one with `tonk remote add <name> <url>`",
    );
    for record in records {
        listing.push([
            record.name.clone(),
            record.endpoint.clone(),
            record.subject.to_string(),
            listing::cell(record.revocation_url.as_deref()),
        ]);
    }
    println!("{}", listing.render());
}

async fn blob_op(command: Option<BlobCommand>, json: bool, space: Option<&str>) -> ExitCode {
    let (_, site) = match open_selected(space).await {
        Ok(opened) => opened,
        Err(code) => return code,
    };

    match command {
        Some(BlobCommand::Add {
            file,
            content_type,
            write,
        }) => {
            if write.dry_run {
                return match blob::plan(&file, content_type).await {
                    Ok(plan) => {
                        print_blob_add_plan(&plan, write.quiet);
                        ExitCode::Success
                    }
                    Err(err) => print_coded(err),
                };
            }
            let sync = auto_sync::enabled(write.no_sync);
            match auto_sync::around_commit(&site, sync, blob::add(&site, &file, content_type)).await
            {
                Ok(outcome) => {
                    print_blob_add_outcome(&outcome, write.quiet);
                    ExitCode::Success
                }
                Err(err) => print_coded(err),
            }
        }
        Some(BlobCommand::Cat { reference }) => {
            let mut stdout = tokio::io::stdout();
            match blob::cat(&site, &reference, &mut stdout).await {
                Ok(_) => ExitCode::Success,
                Err(err) => print_coded(err),
            }
        }
        None => match blob::ls(&site).await {
            Ok(rows) if json => print_json(&Rows::new("tonk.blob-ls.v1", rows)),
            Ok(rows) => {
                print_blob_ls(&rows);
                ExitCode::Success
            }
            Err(err) => print_coded(err),
        },
    }
}

fn print_blob_ls(rows: &[blob::LsRow]) {
    let mut listing = Listing::new(
        &["ENTITY", "CONTENT-TYPE", "NAME"],
        "no blobs on this branch; add one with `tonk blob add <path>`",
    );
    for row in rows {
        listing.push([
            row.entity.as_str().to_owned(),
            listing::cell(row.content_type.as_deref()),
            listing::cell(row.name.as_deref()),
        ]);
    }
    println!("{}", listing.render());
}

fn print_blob_add_outcome(outcome: &BlobAddOutcome, quiet: bool) {
    println!("{}", outcome.entity.as_str());
    if !quiet {
        eprintln!(
            "  content-type: {}, size: {} bytes",
            outcome.content_type, outcome.size
        );
    }
}

/// Report a `--dry-run` add on stderr, leaving stdout empty.
///
/// Stdout carries the `blob:<hash>` reference on a real add, and a dry
/// run has none to give — printing anything there would hand a pipeline
/// a value that does not name a stored blob.
fn print_blob_add_plan(plan: &blob::AddPlan, quiet: bool) {
    if !quiet {
        eprintln!(
            "would add {} ({}, {} bytes); nothing written",
            plan.name, plan.content_type, plan.size
        );
    }
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
    space: Option<&str>,
) -> ExitCode {
    let (selected, site) = match open_selected(space).await {
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
    // Whether `None` below means "the user waved off a choice between
    // several remotes" rather than "this space has none". The first
    // still wants the canonical base; the second has no honest origin
    // to offer.
    let mut declined_an_origin = false;
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
            declined_an_origin = true;
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
        (None, None) if declined_an_origin => invite::DEFAULT_BASE_URL.to_owned(),
        // A space with no remote has no deployment serving it, so
        // there is no origin a recipient could join it from. Minting
        // against the canonical base would hand them a link to
        // production, which holds none of this data — a share that
        // reads as successful and works for nobody. Refuse instead,
        // and name each way to give the space an origin.
        (None, None) => {
            let name = &selected.name;
            let base = invite::DEFAULT_BASE_URL;
            return print_error(format!(
                "'{name}' has no remote, so there is nowhere to invite anyone to\n\
                 \x20      its data lives only on this device, and a link would point at \
                 {base}, which serves none of it\n\
                 \x20      give it a home first: `tonk account login` then \
                 `tonk space link {name}`, or `tonk remote add <name> <URL> \
                 --revocation-url <URL>`\n\
                 \x20      to mint against a deployment tonk doesn't know about, pass \
                 `--base-url <URL>`"
            ));
        }
    };

    // Carried when the remote names one, absent when it does not. A relay is
    // no longer required to mint: revocations are invocations now, addressed
    // to the access service the invite already carries.
    let revocation_url = resolved
        .as_ref()
        .and_then(|record| record.revocation_url.clone());
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
        Err(err) => print_coded(err),
    }
}

fn print_invite_outcome(outcome: &InviteOutcome) {
    println!("{url}", url = outcome.url);
    eprintln!("subject:  {}", outcome.subject);
    eprintln!("audience: {} (ephemeral)", outcome.audience);
}

/// `tonk join` — claim an invite into a fresh canonical space:
/// site at `spaces/<name>/`, registered and bound here on success.
/// The early registry load below is only a cheap fail-fast
/// duplicate-name check; the invite claim is a network operation
/// that can take seconds, so registration itself happens only
/// after the claim succeeds, against a registry freshly reloaded
/// at that point — a concurrent `tonk space new`/`use`/`rm` while
/// the claim is in flight is re-checked, never silently reverted.
/// A failed join never leaves a dangling registry entry (a
/// partial site dir may remain; re-running with the same name
/// reports it).
async fn claim_invite(url: String, name: String, flag: Option<&str>) -> ExitCode {
    if let Err(err) = tonk_cli::space::validate_name(&name) {
        return print_failure(err);
    }
    let store = match tonk_cli::space::SpaceStore::open() {
        Ok(store) => store,
        Err(err) => return print_failure(err),
    };
    let store = &store;
    let cwd = match working_directory().and_then(|path| path.canonicalize().ok()) {
        Some(cwd) => cwd,
        None => return print_error("could not read the current directory".to_owned()),
    };
    let registry = match store.load() {
        Ok(registry) => registry,
        Err(err) => return print_failure(err),
    };
    if registry.spaces.contains_key(&name) {
        return print_error(tonk_cli::space::SpaceError::Exists(name).to_string());
    }
    let root = store.canonical_site(&name);

    // Same default site config `tonk space new` writes against, so
    // the joined site picks up the user's normal profile.
    let config = match site::default_config() {
        Ok(config) => config,
        Err(error) => return print_failure(error),
    };
    match invite::claim(&root, &url, config.clone()).await {
        Ok(outcome) => {
            // Match `space new`'s canonicalized form, so registered
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
            if registry.spaces.contains_key(&name) {
                return print_error(format!(
                    "{err}\nthe site was claimed at {root}; register it with \
                     `tonk space new <other-name> --site {root}`",
                    err = tonk_cli::space::SpaceError::Exists(name.clone()),
                    root = root.display(),
                ));
            }

            registry
                .spaces
                .insert(name.clone(), tonk_cli::space::SpaceEntry::at(root.clone()));
            if let Err(err) = store.save(&registry) {
                return print_error(format!(
                    "joined, but registering space '{name}' failed: {err}\n\
                     re-register with `tonk space new {name} --site {root}`",
                    root = root.display(),
                ));
            }
            if let Err(error) = tonk_cli::space::bind(store, &name, &cwd) {
                return print_failure(error);
            }
            print_claim_outcome(&name, &root, &cwd, &outcome);
            print_active_space_resolution(store, flag, Some(&cwd));
            match site::TonkSite::open_with(&root, config).await {
                Ok(site) => record_space_best_effort(&name, &site).await,
                Err(error) => eprintln!("warning: account directory update skipped: {error:#}"),
            }
            ExitCode::Success
        }
        Err(err) => print_coded(err),
    }
}

fn print_claim_outcome(
    name: &str,
    root: &std::path::Path,
    directory: &std::path::Path,
    outcome: &ClaimOutcome,
) {
    println!("Joined space '{name}' ({})", root.display());
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
    println!("next: tonk status");
}

/// `tonk migrate account` — drain the legacy certificate store.
async fn migrate_account() -> ExitCode {
    match tonk_cli::account_state::migrate_delegations_here().await {
        Ok(outcome) => {
            println!(
                "migrated {} certificate{} into access facts",
                outcome.certificates,
                if outcome.certificates == 1 { "" } else { "s" }
            );
            println!(
                "retained {} space{} into the account space ({} already there)",
                outcome.spaces,
                if outcome.spaces == 1 { "" } else { "s" },
                outcome.already
            );
            if outcome.account_legacy {
                eprintln!(
                    "warning: the account repository is still in the legacy format; \
                     certificate migration completed, but space retention was skipped"
                );
            }
            ExitCode::Success
        }
        Err(error) => print_failure(error),
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
                "register it as a space: `tonk space new <name> --site {}`",
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

/// List user-defined concepts (`tonk concept`), one
/// aligned `name  description` row per concept.
async fn list_concepts_op(site: &site::TonkSite, json: bool) -> ExitCode {
    let concepts = match schema::list_concepts(site).await {
        Ok(c) => c,
        Err(err) => return print_failure(err),
    };
    if json {
        return print_json(&Rows::new("tonk.concept-ls.v1", concepts));
    }
    let mut listing = Listing::new(
        &["NAME", "DESCRIPTION"],
        "this space defines no concepts; add one with `tonk concept add <name> --field <field>:<type>:<card>`",
    );
    for concept in &concepts {
        listing.push([
            concept.name.clone(),
            listing::cell(concept.description.as_deref()),
        ]);
    }
    println!("{}", listing.render());
    ExitCode::Success
}

/// Query every instance of `concept` as rendered by
/// [`data_ops::query`].
async fn query_op(concept: String, json: bool, space: Option<&str>) -> ExitCode {
    let (_, site) = match open_selected(space).await {
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
        Err(err) => print_coded(err),
    }
}

/// Print a single instance of `concept` as rendered by
/// [`data_ops::get`].
async fn get_op(concept: String, entity: String, json: bool, space: Option<&str>) -> ExitCode {
    let (_, site) = match open_selected(space).await {
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
        Err(err) => print_coded(err),
    }
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ShowConceptReport {
    schema_version: &'static str,
    name: String,
    entity: String,
    description: Option<String>,
    fields: Vec<ShowField>,
    views: Vec<String>,
    recipes: Vec<String>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ShowField {
    name: String,
    r#type: String,
    cardinality: String,
    optional: bool,
    description: String,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ShowEntityReport {
    schema_version: &'static str,
    entity: String,
    facts: Vec<views::EntityFact>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ShowViewReport {
    schema_version: &'static str,
    anchor: String,
    entity: String,
    model: Option<String>,
    template: String,
}

async fn show_op(
    name: Option<String>,
    entity: Option<String>,
    json: bool,
    notation: bool,
    space: Option<&str>,
) -> ExitCode {
    let (_, site) = match open_selected(space).await {
        Ok(opened) => opened,
        Err(code) => return code,
    };
    let Some(name) = name else {
        if json {
            return match schema::list_all_concepts(&site).await {
                Ok(rows) => print_json(&Rows::new("tonk.show-schema.v1", rows)),
                Err(error) => print_failure(error),
            };
        }
        return match schema::render(&site).await {
            Ok(text) => write_stdout(&text),
            Err(error) => print_failure(error),
        };
    };

    let concept = match schema::find_concept(&site, &name).await {
        Ok(concept) => concept,
        Err(error) => return print_failure(error),
    };
    if let Some(concept) = concept {
        if let Some(entity) = entity {
            return get_op(name, entity, json, space).await;
        }
        if notation {
            return match data_ops::schema_subset(&site, &name).await {
                Ok(text) => write_stdout(&text),
                Err(error) => print_coded(error),
            };
        }
        let view_names = match views::list(&site).await {
            Ok(rows) => rows
                .into_iter()
                .filter(|view| view.model.as_deref() == Some(name.as_str()))
                .map(|view| view.name.unwrap_or_else(|| view.entity.to_string()))
                .collect(),
            Err(error) => return print_failure(error),
        };
        let fields = concept
            .descriptor
            .with()
            .iter()
            .map(|(name, field)| ShowField {
                name: name.to_string(),
                r#type: field
                    .content_type()
                    .map(|value| format!("{value:?}"))
                    .unwrap_or_else(|| "Value".to_owned()),
                cardinality: format!("{:?}", field.cardinality()).to_lowercase(),
                optional: field.is_optional(),
                description: field.description().to_owned(),
            })
            .collect();
        let report = ShowConceptReport {
            schema_version: "tonk.show-concept.v1",
            name: name.clone(),
            entity: concept.entity,
            description: concept.description,
            fields,
            views: view_names,
            recipes: vec![
                format!("tonk query {name}"),
                format!("tonk assert {name} --<field> <value>"),
                format!("tonk assert {name} <entity> --<field> <value>"),
            ],
        };
        if json {
            return print_json(&report);
        }
        println!("concept: {}", report.name);
        if let Some(description) = &report.description {
            println!("description: {description}");
        }
        println!("entity: {}", report.entity);
        println!("fields:");
        for field in &report.fields {
            let optional = if field.optional { " optional" } else { "" };
            println!(
                "  {}\t{}\t{}{}",
                field.name, field.r#type, field.cardinality, optional
            );
        }
        if !report.views.is_empty() {
            println!("views: {}", report.views.join(", "));
        }
        println!("recipes:");
        for recipe in &report.recipes {
            println!("  {recipe}");
        }
        return ExitCode::Success;
    }

    if entity.is_some() {
        return print_error(format!("no concept named '{name}'"));
    }
    let view = match views::describe(&site, &name).await {
        Ok(view) => view,
        Err(error) => return print_failure(error),
    };
    if let Some(view) = view {
        let report = ShowViewReport {
            schema_version: "tonk.show-view.v1",
            anchor: view.anchor,
            entity: view.entity.to_string(),
            model: view.model,
            template: view.template,
        };
        if json {
            return print_json(&report);
        }
        println!("view: {}", report.anchor);
        println!("entity: {}", report.entity);
        if let Some(model) = &report.model {
            println!("model: {model}");
        }
        println!("template:\n{}", report.template);
        return ExitCode::Success;
    }
    let facts = match views::facts_for_entity(&site, &name).await {
        Ok(Some(facts)) => facts,
        Ok(None) => return print_error(format!("no concept, view, or entity named '{name}'")),
        Err(error) => return print_failure(error),
    };
    let report = ShowEntityReport {
        schema_version: "tonk.show-entity.v1",
        entity: facts.0.to_string(),
        facts: facts.1,
    };
    if json {
        return print_json(&report);
    }
    println!("entity: {}", report.entity);
    for fact in &report.facts {
        println!("  {}\t{}", fact.attribute, fact.value);
    }
    ExitCode::Success
}

fn write_stdout(text: &str) -> ExitCode {
    let mut stdout = std::io::stdout().lock();
    match stdout.write_all(text.as_bytes()) {
        Ok(()) => ExitCode::Success,
        Err(error) => print_error(format!("failed to write stdout: {error}")),
    }
}

/// Generic workflow for `tonk assert` before a live concept supplies fields.
const ASSERT_USAGE: &str = "\
Write facts: create an instance, or update fields on an existing entity.

Workflow:
  1. tonk query <CONCEPT> --json
  2. tonk assert <CONCEPT> <ENTITY> --<field> <value>
  3. tonk show <CONCEPT> <ENTITY> --json

Create:
  tonk assert <CONCEPT> --<required-field> <value> ...

See the live typed flags:
  tonk assert <CONCEPT> --help

Example:
  tonk query task --json
  tonk assert task <ENTITY> --done true
  tonk show task <ENTITY> --json
";

/// Split `rest` into the optional entity and the flag argv, then
/// assert via [`data_ops::assert_op`]. A leading non-flag token is
/// always the entity (the supersede form) — an entity reference
/// never starts with `-`, and flag values always follow their
/// flag, so the first token is either a flag or the entity. Same
/// dynamic-flag / `--help` handling as the old `add`/`set`.
async fn assert_cmd(concept: Option<String>, rest: Vec<String>, space: Option<&str>) -> ExitCode {
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
    let (_, site) = match open_selected(space).await {
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
        Err(err) => print_coded(err),
    }
}

/// Retract a single field, or a whole instance, as rendered by
/// [`data_ops::retract`].
async fn retract_op(
    concept: String,
    entity: String,
    field: Option<String>,
    notation: bool,
    write: WriteArgs,
    space: Option<&str>,
) -> ExitCode {
    let (_, site) = match open_selected(space).await {
        Ok(opened) => opened,
        Err(code) => return code,
    };

    match data_ops::retract(
        &site,
        &concept,
        &entity,
        field.as_deref(),
        write.options(notation),
    )
    .await
    {
        Ok(text) => {
            let mut stdout = std::io::stdout().lock();
            if let Err(e) = stdout.write_all(text.as_bytes()) {
                return print_error(format!("failed to write stdout: {e}"));
            }
            ExitCode::Success
        }
        Err(err) => print_coded(err),
    }
}

/// Author a new concept, as rendered by [`data_ops::concept_add`].
async fn concept_op(command: Option<ConceptCommand>, json: bool, space: Option<&str>) -> ExitCode {
    let (_, site) = match open_selected(space).await {
        Ok(opened) => opened,
        Err(code) => return code,
    };

    match command {
        Some(ConceptCommand::Add {
            name,
            fields,
            description,
            notation,
            write,
        }) => {
            match data_ops::concept_add(
                &site,
                &name,
                &fields,
                description.as_deref(),
                write.options(notation),
            )
            .await
            {
                Ok(text) => {
                    let mut stdout = std::io::stdout().lock();
                    if let Err(e) = stdout.write_all(text.as_bytes()) {
                        return print_error(format!("failed to write stdout: {e}"));
                    }
                    ExitCode::Success
                }
                Err(err) => print_coded(err),
            }
        }
        None => list_concepts_op(&site, json).await,
    }
}

/// Author a declarative view, as rendered by [`data_ops::view_add`].
/// `--template-file` is read here (the thin binary owns I/O); a
/// missing or empty template surfaces as
/// [`tonk_cli::authoring::AuthoringError::EmptyTemplate`] via
/// `data_ops::view_add`'s own check.
async fn view_op(command: Option<ViewCommand>, json: bool, space: Option<&str>) -> ExitCode {
    let (_, site) = match open_selected(space).await {
        Ok(opened) => opened,
        Err(code) => return code,
    };

    match command {
        Some(ViewCommand::Add {
            model,
            template,
            template_file,
            kind,
            home,
            notation,
            write,
        }) => {
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
            match data_ops::view_add(
                &site,
                &model,
                kind.into(),
                &template,
                home,
                write.options(notation),
            )
            .await
            {
                Ok(text) => {
                    let mut stdout = std::io::stdout().lock();
                    if let Err(e) = stdout.write_all(text.as_bytes()) {
                        return print_error(format!("failed to write stdout: {e}"));
                    }
                    ExitCode::Success
                }
                Err(err) => print_coded(err),
            }
        }
        None => list_views_op(&site, json).await,
    }
}

/// Put one or more concepts' directories on the space home, as
/// rendered by [`data_ops::home`].
async fn home_op(
    models: Vec<String>,
    notation: bool,
    write: WriteArgs,
    space: Option<&str>,
) -> ExitCode {
    let (_, site) = match open_selected(space).await {
        Ok(opened) => opened,
        Err(code) => return code,
    };

    match data_ops::home(&site, &models, write.options(notation)).await {
        Ok(text) => {
            let mut stdout = std::io::stdout().lock();
            if let Err(e) = stdout.write_all(text.as_bytes()) {
                return print_error(format!("failed to write stdout: {e}"));
            }
            ExitCode::Success
        }
        Err(err) => print_coded(err),
    }
}

/// List renderable entities (`tonk view`), one aligned
/// `name  entity  model  bytes` row per template-claim
/// carrier.
async fn list_views_op(site: &site::TonkSite, json: bool) -> ExitCode {
    let listed = match views::list(site).await {
        Ok(v) => v,
        Err(err) => return print_failure(err),
    };
    if json {
        return print_json(&Rows::new("tonk.view-ls.v1", listed));
    }
    let mut listing = Listing::new(
        &["NAME", "ENTITY", "MODEL", "BYTES"],
        "no renderable entities on this branch; author one with `tonk view add <concept> --template <html>`",
    );
    for row in &listed {
        listing.push([
            listing::cell(row.name.as_deref()),
            row.entity.to_string(),
            listing::cell(row.model.as_deref()),
            row.body_bytes.to_string(),
        ]);
    }
    println!("{}", listing.render());
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
///
/// Flattens to [`ExitCode::IoError`], so it is for failures that carry no
/// code of their own. A typed error goes through [`print_coded`] instead.
fn print_failure(error: impl Into<anyhow::Error>) -> ExitCode {
    print_error(failure_text(&error.into()))
}

/// Print a typed failure honoring `--verbose`, and return the exit code it
/// carries.
///
/// The two used to be mutually exclusive: [`print_failure`] rendered the
/// whole chain and threw the code away, while the call sites that needed a
/// real code printed the error inline and ignored `--verbose` — so `-v` on
/// any of them produced a byte-identical message, which reads as a broken
/// flag.
fn print_coded(error: impl tonk_cli::Coded) -> ExitCode {
    let code = error.exit_code();
    eprintln!("error: {}", failure_text(&anyhow::Error::new(error)));
    code
}

/// The process's working directory, used only as a key into the
/// binding map. A cwd the OS refuses to report (deleted out from
/// under the process) is not fatal when --space or TONK_SPACE names
/// the active space.
fn working_directory() -> Option<PathBuf> {
    std::env::current_dir().ok()
}

fn space_from_environment() -> Option<String> {
    std::env::var(tonk_cli::space::SPACE_ENV)
        .ok()
        .filter(|value| !value.is_empty())
}

/// Print stable local context after a space-scoped command fails. This
/// deliberately does not fetch sync state while handling another
/// error.
fn print_active_space_context(flag: Option<&str>) {
    let Ok(store) = tonk_cli::space::SpaceStore::open() else {
        return;
    };
    let env = space_from_environment();
    let cwd = working_directory();
    if let Ok(resolved) = store.resolve(flag, env.as_deref(), cwd.as_deref()) {
        eprintln!(
            "active space: {name} ({source})\nsite: {site}",
            name = resolved.name,
            source = resolved.source,
            site = resolved.site.display(),
        );
    }
}

/// Resolve the active space (--space > TONK_SPACE > nearest directory
/// binding) and open its site. The cwd is passed in only as a key
/// into the binding map — it never locates site data.
async fn open_selected(
    flag: Option<&str>,
) -> Result<(tonk_cli::space::Resolved, site::TonkSite), ExitCode> {
    let store = match tonk_cli::space::SpaceStore::open() {
        Ok(store) => store,
        Err(err) => return Err(print_failure(err)),
    };
    let env = space_from_environment();
    let cwd = working_directory();
    // `resolve` is also where a space belonging to a different account is
    // refused, so every command that opens a site inherits that check
    // without asking for it.
    let resolved = match store.resolve(flag, env.as_deref(), cwd.as_deref()) {
        Ok(resolved) => resolved,
        Err(err) => return Err(print_failure(err)),
    };
    let config = match site::default_config() {
        Ok(config) => config,
        Err(err) => return Err(print_failure(err)),
    };
    match site::TonkSite::open_with(&resolved.site, config).await {
        Ok(site) => Ok((resolved, site)),
        Err(err) => Err(print_error(format!(
            "could not open the active space: {err:#}"
        ))),
    }
}

#[cfg(test)]
mod account_spaces_parser_tests {
    use super::*;

    #[test]
    fn account_login_warning_for_ready_names_the_latest_sync() {
        let warning = account_login_warning(
            tonk_account::AccountStateStatus::Ready,
            "latest account synchronization did not finish within 10 seconds",
        );

        assert_eq!(
            warning,
            "warning: latest account synchronization is incomplete: latest account synchronization did not finish within 10 seconds"
        );
        assert!(!warning.contains("repository is not synchronized"));
        assert_eq!(
            account_state_label(tonk_account::AccountStateStatus::Ready),
            "synced"
        );
    }

    #[test]
    fn account_login_warning_for_unhydrated_names_the_repository() {
        assert_eq!(
            account_login_warning(
                tonk_account::AccountStateStatus::Unhydrated,
                "the account repository did not answer within 10 seconds",
            ),
            "warning: account repository is not synchronized: the account repository did not answer within 10 seconds"
        );
    }

    #[test]
    fn unavailable_account_context_does_not_invent_a_device_identifier() {
        let account = account_context_unavailable();
        let json = serde_json::to_value(&account).expect("account context JSON");
        assert!(json["device"].is_null(), "{json}");
        assert!(account.render().contains("device: unavailable"));
    }

    #[test]
    fn customer_text_and_json_share_one_state_mapping() {
        for (state, token, line) in [
            (
                CustomerState::Registered,
                Some("registered"),
                Some("registered"),
            ),
            (
                CustomerState::AwaitingEmailConfirmation,
                Some("awaiting-email-confirmation"),
                Some("waiting for email confirmation (check your inbox)"),
            ),
            (
                CustomerState::Suspended,
                Some("suspended"),
                Some("suspended"),
            ),
            (CustomerState::Absent, None, None),
            (CustomerState::Unreachable, None, Some("unreachable")),
        ] {
            assert_eq!(state.token().as_deref(), token);
            assert_eq!(state.line().as_deref(), line);
        }
    }

    #[test]
    fn every_data_write_parser_accepts_the_shared_switches() {
        for args in [
            vec!["tonk", "space", "agents", "set", "AGENTS.md"],
            vec![
                "tonk",
                "concept",
                "add",
                "note",
                "--field",
                "title:text:one",
            ],
            vec![
                "tonk",
                "view",
                "add",
                "note",
                "--template",
                "<p>{title}</p>",
            ],
            vec!["tonk", "space", "home", "note"],
            vec!["tonk", "retract", "note", "id:note"],
            vec!["tonk", "blob", "add", "note.txt"],
            vec!["tonk", "import", "data.csv"],
        ] {
            let mut invocation = args.clone();
            invocation.extend(["--dry-run", "--no-sync", "--quiet"]);
            assert!(
                Cli::try_parse_from(&invocation).is_ok(),
                "shared write switches rejected for {args:?}"
            );
        }
    }

    #[test]
    fn macro_writes_expose_notation_and_eval_uses_json_directly() {
        for args in [
            &[
                "tonk",
                "concept",
                "add",
                "note",
                "--field",
                "title:text:one",
                "--notation",
            ][..],
            &[
                "tonk",
                "view",
                "add",
                "note",
                "--template",
                "<p>{title}</p>",
                "--notation",
            ],
            &["tonk", "space", "home", "note", "--notation"],
            &["tonk", "retract", "note", "id:note", "--notation"],
        ] {
            assert!(
                Cli::try_parse_from(args).is_ok(),
                "new spelling rejected: {args:?}"
            );
        }
        assert!(Cli::try_parse_from(["tonk", "eval", "-c", "note:", "--json"]).is_ok());
        assert!(
            Cli::try_parse_from(["tonk", "concept", "add", "note", "--attr", "title:text:one"])
                .is_err()
        );
        assert!(Cli::try_parse_from(["tonk", "eval", "-c", "note:", "--format", "json"]).is_err());
    }

    #[test]
    fn nouns_list_bare_and_space_owns_home_and_agents() {
        for args in [
            &["tonk", "space"][..],
            &["tonk", "remote"],
            &["tonk", "concept"],
            &["tonk", "view"],
            &["tonk", "blob"],
            &["tonk", "account"],
            &["tonk", "account", "space"],
            &["tonk", "space", "home", "note"],
            &["tonk", "space", "agents"],
        ] {
            assert!(
                Cli::try_parse_from(args).is_ok(),
                "bare noun rejected: {args:?}"
            );
        }
        for args in [
            &["tonk", "space", "list"][..],
            &["tonk", "remote", "list"],
            &["tonk", "concept", "ls"],
            &["tonk", "view", "ls"],
            &["tonk", "blob", "ls"],
            &["tonk", "account", "space", "list"],
            &["tonk", "account", "spaces"],
            &["tonk", "home", "note"],
            &["tonk", "agents"],
            &["tonk", "space", "use"],
        ] {
            assert!(
                Cli::try_parse_from(args).is_err(),
                "retired spelling parsed: {args:?}"
            );
        }
    }

    #[test]
    fn status_and_show_replace_context_schema_and_entity_query() {
        for args in [
            &["tonk", "show"][..],
            &["tonk", "show", "task"],
            &["tonk", "show", "task", "id:one"],
            &["tonk", "show", "task", "--json"],
            &["tonk", "show", "task", "--notation"],
        ] {
            assert!(
                Cli::try_parse_from(args).is_ok(),
                "show form rejected: {args:?}"
            );
        }
        for args in [
            &["tonk", "context"][..],
            &["tonk", "schema"],
            &["tonk", "query", "task", "id:one"],
        ] {
            assert!(
                Cli::try_parse_from(args).is_err(),
                "retired form parsed: {args:?}"
            );
        }
        assert_eq!(STATUS_SCHEMA_VERSION, "tonk.status.v2");
    }

    #[test]
    fn index_and_parser_command_sets_cannot_drift() {
        let mut indexed: Vec<_> = CLI_INDEX
            .lines()
            .filter_map(|line| line.strip_prefix("   "))
            .filter_map(|line| line.split_whitespace().next())
            .collect();
        indexed.sort_unstable();
        let command = Cli::command();
        let mut visible: Vec<_> = command
            .get_subcommands()
            .filter(|command| !command.is_hide_set())
            .map(clap::Command::get_name)
            .collect();
        visible.sort_unstable();
        assert_eq!(indexed, visible);

        for name in indexed {
            if let Err(error) = Cli::try_parse_from(["tonk", name]) {
                assert_ne!(
                    error.kind(),
                    clap::error::ErrorKind::InvalidSubcommand,
                    "index command does not parse: {name}"
                );
            }
        }
    }

    #[test]
    fn every_listed_guide_has_a_description_and_body() {
        for topic in guide::TOPICS {
            assert!(
                guide::description(topic).is_some(),
                "missing description: {topic}"
            );
            assert!(guide::topic(topic).is_some(), "missing body: {topic}");
        }
    }

    #[test]
    fn guides_do_not_teach_retired_cli_spellings() {
        for retired in [
            "tonk context",
            "tonk guide",
            "tonk schema",
            "tonk home",
            "tonk agents",
            "tonk space list",
            "tonk remote list",
            "tonk concept ls",
            "tonk view ls",
            "tonk blob ls",
            "tonk account space list",
            "tonk account spaces",
            "--attr",
            "--format",
        ] {
            assert!(
                !guide::GUIDE.contains(retired),
                "guide still teaches retired spelling `{retired}`"
            );
        }
    }

    #[test]
    fn representative_guide_commands_parse() {
        for args in [
            &[
                "tonk",
                "concept",
                "add",
                "note",
                "--field",
                "title:text:one",
            ][..],
            &["tonk", "assert", "note", "--title", "hello"],
            &["tonk", "query", "note", "--json"],
            &["tonk", "retract", "note", "id:note"],
            &[
                "tonk",
                "view",
                "add",
                "note",
                "--template",
                "<p>{title}</p>",
            ],
            &["tonk", "space", "home", "note"],
            &["tonk", "space", "new", "scratch", "--site", "./scratch"],
            &["tonk", "space", "use", "scratch"],
            &["tonk", "space", "unbind"],
            &["tonk", "render", "alice@person!label"],
            &[
                "tonk",
                "remote",
                "add",
                "prod",
                "https://access.example.com",
            ],
            &["tonk", "remote", "set-upstream", "prod"],
            &[
                "tonk",
                "join",
                "https://example/#invite",
                "--name",
                "shared",
            ],
            &["tonk", "help", "views"],
            &["tonk", "status", "--json"],
        ] {
            assert!(
                Cli::try_parse_from(args).is_ok(),
                "guide command no longer parses: {args:?}"
            );
        }
    }

    #[test]
    fn view_kind_and_home_flags_parse() {
        for (value, expected) in [
            ("detail", ViewKindArg::Detail),
            ("directory", ViewKindArg::Directory),
            ("label", ViewKindArg::Label),
            ("title", ViewKindArg::Title),
        ] {
            let cli = Cli::try_parse_from([
                "tonk",
                "view",
                "add",
                "note",
                "--kind",
                value,
                "--template",
                "<p>{title}</p>",
                "--home",
            ])
            .expect("view kind parses");
            let Some(Command::View {
                command: Some(ViewCommand::Add { kind, home, .. }),
                ..
            }) = cli.command
            else {
                panic!("expected view add command");
            };
            assert_eq!(kind, expected);
            assert!(home);
        }

        let cli = Cli::try_parse_from([
            "tonk",
            "view",
            "add",
            "note",
            "--template",
            "<p>{title}</p>",
        ])
        .expect("default view kind parses");
        assert!(matches!(
            cli.command,
            Some(Command::View {
                command: Some(ViewCommand::Add {
                    kind: ViewKindArg::Detail,
                    home: false,
                    ..
                }),
                ..
            })
        ));
        assert!(
            Cli::try_parse_from([
                "tonk",
                "view",
                "add",
                "note",
                "--kind",
                "gallery",
                "--template",
                "<p>{title}</p>",
            ])
            .is_err()
        );
    }

    #[test]
    fn view_add_rejects_the_removed_name_and_anchor_spellings() {
        assert!(
            Cli::try_parse_from([
                "tonk",
                "view",
                "add",
                "note",
                "--anchor",
                "note-card",
                "--template",
                "<p>{title}</p>",
            ])
            .is_err(),
            "a view has no entity of its own to anchor"
        );

        assert!(
            Cli::try_parse_from([
                "tonk",
                "view",
                "add",
                "note",
                "--name",
                "note-card",
                "--template",
                "<p>{title}</p>",
            ])
            .is_err(),
            "the removed view-specific --name spelling must be rejected"
        );

        assert!(
            Cli::try_parse_from([
                "tonk",
                "join",
                "https://example/#invite",
                "--name",
                "shared"
            ])
            .is_ok(),
            "unrelated --name flags remain available"
        );
    }

    #[test]
    fn account_help_uses_space_terminology() {
        let mut command = Cli::command();
        let help = command
            .find_subcommand_mut("account")
            .expect("account command")
            .render_long_help()
            .to_string();

        assert!(help.contains("devices, spaces, and names"), "{help}");
        assert!(!help.contains("spot"), "{help}");
    }

    #[test]
    fn eval_home_flag_parses() {
        let cli = Cli::try_parse_from(["tonk", "eval", "app.notation", "--home", "todo"]).unwrap();
        let Some(Command::Eval(EvalArgs { home, .. })) = cli.command else {
            panic!("expected eval command");
        };
        assert_eq!(home.as_deref(), Some("todo"));
    }

    #[test]
    fn every_account_read_parser_owns_a_json_form() {
        for args in [
            &["tonk", "account", "status", "--json"][..],
            &["tonk", "account", "space", "--json"],
            &["tonk", "account", "devices", "--json"],
            &["tonk", "space", "agents", "get", "--json"],
        ] {
            assert!(
                Cli::try_parse_from(args).is_ok(),
                "JSON rejected for {args:?}"
            );
        }
    }

    #[test]
    fn account_listing_json_keeps_the_shared_rows_envelope() {
        let spaces = serde_json::to_value(account_spaces_report(Vec::new())).expect("spaces JSON");
        assert_eq!(spaces["schemaVersion"], "tonk.account-spaces.v1");
        assert!(spaces["rows"].is_array());

        let devices =
            serde_json::to_value(account_devices_report(Vec::new())).expect("devices JSON");
        assert_eq!(devices["schemaVersion"], "tonk.account-devices.v1");
        assert!(devices["rows"].is_array());
    }

    #[test]
    fn account_status_makes_sign_in_state_explicit() {
        assert_eq!(
            account_context(&account::AccountStatus::MissingRoot {
                device_did: "did:device".to_string(),
            })
            .render(),
            "signed in: no\naccount: missing\naccount service: none\ndevice: did:device\n"
        );
        assert_eq!(
            account_context(&account::AccountStatus::Unregistered {
                root_did: "did:root".to_string(),
                device_did: "did:device".to_string(),
            })
            .render(),
            "signed in: no\naccount: did:root\naccount service: none\ndevice: did:device\n"
        );
        // `status:` became `account status:`. Bare `status:` was ambiguous
        // once this section renders inside `tonk status` next to the sync
        // section, which has a state of its own.
        assert_eq!(
            account_context(&account::AccountStatus::Registered {
                root_did: "did:root".to_string(),
                device_did: "did:device".to_string(),
                provider: "https://accounts.example".to_string(),
                account_state: tonk_account::AccountStateStatus::Ready,
            })
            .render(),
            "signed in: yes\naccount: did:root\naccount service: https://accounts.example\ndevice: did:device\naccount status: synced\n"
        );
    }

    #[test]
    fn account_login_name_is_none_when_omitted() {
        let cli = Cli::try_parse_from(["tonk", "account", "login"]).unwrap();
        let Some(Command::Account {
            command: Some(AccountCommand::Login { name, .. }),
            ..
        }) = cli.command
        else {
            panic!("expected account login");
        };
        assert_eq!(name, None);
    }

    #[test]
    fn account_login_name_preserves_an_explicit_override() {
        let cli =
            Cli::try_parse_from(["tonk", "account", "login", "--name", "workstation"]).unwrap();
        let Some(Command::Account {
            command: Some(AccountCommand::Login { name, .. }),
            ..
        }) = cli.command
        else {
            panic!("expected account login");
        };
        assert_eq!(name.as_deref(), Some("workstation"));
    }

    #[test]
    fn account_delete_is_a_browser_review_not_an_immediate_flag() {
        let cli = Cli::try_parse_from(["tonk", "account", "delete", "--no-open"]).unwrap();
        let Some(Command::Account {
            command: Some(AccountCommand::Delete { no_open, .. }),
            ..
        }) = cli.command
        else {
            panic!("expected account delete");
        };
        assert!(no_open);
    }

    #[test]
    fn account_logout_is_a_no_argument_account_operation() {
        let cli = Cli::try_parse_from(["tonk", "account", "logout"]).unwrap();
        let command = cli.command.as_ref().expect("account command");
        assert!(matches!(
            command,
            Command::Account {
                command: Some(AccountCommand::Logout),
                ..
            }
        ));
        assert_eq!(descriptor(command), ("account", Some("logout")));
        assert!(Cli::try_parse_from(["tonk", "account", "logout", "unexpected"]).is_err());
    }

    #[test]
    fn account_space_lists_bare_and_rejects_plural_and_list() {
        assert!(Cli::try_parse_from(["tonk", "account", "space"]).is_ok());
        assert!(Cli::try_parse_from(["tonk", "account", "space", "list"]).is_err());
        assert!(Cli::try_parse_from(["tonk", "account", "spaces"]).is_err());
    }

    #[test]
    fn signing_in_is_spelled_login_and_only_login() {
        // `link` used to be the canonical spelling, with `login` an alias.
        // It collided with `tonk space link`, which links a *space* to an
        // account rather than a *device* — the same word for two different
        // objects. `login` pairs with the `logout` that was already there.
        assert!(Cli::try_parse_from(["tonk", "account", "login"]).is_ok());
        assert!(Cli::try_parse_from(["tonk", "account", "link"]).is_err());

        // One account at a time: there is no profile to add or select.
        assert!(Cli::try_parse_from(["tonk", "account", "add", "--label", "work"]).is_err());
        assert!(Cli::try_parse_from(["tonk", "account", "use", "work"]).is_err());
        assert!(Cli::try_parse_from(["tonk", "account", "list"]).is_err());
    }

    #[test]
    fn reading_the_agents_claim_is_a_subcommand_that_owns_its_json_flag() {
        // `--json` used to sit on the parent, where `tonk space agents --json set
        // AGENTS.md` parsed fine and then had to be refused at runtime. On
        // `get` the combination cannot be spelled.
        assert!(matches!(
            Cli::try_parse_from(["tonk", "space", "agents", "get", "--json"])
                .unwrap()
                .command,
            Some(Command::Space {
                command: Some(SpaceCommand::Agents {
                    command: Some(AgentsCommand::Get { json: true })
                }),
                ..
            })
        ));

        // Bare `tonk space agents` still projects the Markdown.
        assert!(matches!(
            Cli::try_parse_from(["tonk", "space", "agents"])
                .unwrap()
                .command,
            Some(Command::Space {
                command: Some(SpaceCommand::Agents { command: None }),
                ..
            })
        ));

        assert!(Cli::try_parse_from(["tonk", "agents"]).is_err());
    }

    #[test]
    fn space_rm_no_longer_accepts_the_flag_that_did_nothing() {
        // `--delete` was hidden and inert: deleting the data is the default,
        // so a script still passing it was passing a flag with no effect.
        assert!(Cli::try_parse_from(["tonk", "space", "rm", "garden", "--delete"]).is_err());

        let cli = Cli::try_parse_from(["tonk", "space", "rm", "garden", "--keep-data"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Space {
                command: Some(SpaceCommand::Rm {
                    keep_data: true,
                    yes: false,
                    ..
                }),
                ..
            })
        ));
    }

    #[test]
    fn each_migration_is_named_for_what_it_converts() {
        let cli = Cli::try_parse_from(["tonk", "migrate", "carry", "--move"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Migrate {
                command: MigrateCommand::Carry {
                    from: None,
                    do_move: true
                }
            })
        ));

        assert!(matches!(
            Cli::try_parse_from(["tonk", "migrate", "account"])
                .unwrap()
                .command,
            Some(Command::Migrate {
                command: MigrateCommand::Account
            })
        ));

        assert!(Cli::try_parse_from(["tonk", "migrate", "--from", "../old"]).is_err());
        assert!(Cli::try_parse_from(["tonk", "account", "migrate"]).is_err());

        // The pre-dialog-format space upgrade was the third conversion.
        // It is gone rather than renamed, so neither spelling resolves —
        // including the `--legacy` flag set it briefly became a subcommand
        // for.
        assert!(Cli::try_parse_from(["tonk", "migrate", "space", "garden"]).is_err());
        assert!(Cli::try_parse_from(["tonk", "migrate", "--legacy", "--site", "garden"]).is_err());

        // Bare `tonk migrate` names no conversion, so it cannot pick one.
        assert!(Cli::try_parse_from(["tonk", "migrate"]).is_err());
    }

    #[test]
    fn binding_a_directory_is_a_space_subcommand_with_no_top_level_alias() {
        // `use` and `unbind` are inverses, so they live in the same group.
        // The top-level spelling is gone rather than aliased: an alias would
        // leave half the pair where it was, which is what the move fixes.
        let cli = Cli::try_parse_from(["tonk", "space", "use", "garden"]).unwrap();
        let Some(Command::Space {
            command: Some(SpaceCommand::Use { name }),
            ..
        }) = cli.command
        else {
            panic!("expected space use");
        };
        assert_eq!(name, "garden");

        assert!(Cli::try_parse_from(["tonk", "space", "use"]).is_err());

        assert!(Cli::try_parse_from(["tonk", "use", "garden"]).is_err());
    }

    #[test]
    fn account_space_pull_captures_a_name_or_subject_and_optional_local_name() {
        let did = "did:key:z6MkgMn9hDxTd2saBSAouyTpPLWUmzrVTXfS1N5yB4TjJ3qL";
        let cli =
            Cli::try_parse_from(["tonk", "account", "space", "pull", did, "--name", "garden"])
                .unwrap();
        let Some(Command::Account {
            command:
                Some(AccountCommand::Space {
                    command:
                        Some(AccountSpaceCommand::Pull {
                            name_or_subject,
                            name,
                        }),
                    ..
                }),
            ..
        }) = cli.command
        else {
            panic!("expected account space pull");
        };
        assert_eq!(name_or_subject, did);
        assert_eq!(name.as_deref(), Some("garden"));
    }

    #[test]
    fn account_space_delete_requires_an_exact_subject_and_browser_review() {
        let did = "did:key:z6MkgMn9hDxTd2saBSAouyTpPLWUmzrVTXfS1N5yB4TjJ3qL";
        let cli =
            Cli::try_parse_from(["tonk", "account", "space", "delete", did, "--no-open"]).unwrap();
        let Some(Command::Account {
            command:
                Some(AccountCommand::Space {
                    command:
                        Some(AccountSpaceCommand::Delete {
                            subject, no_open, ..
                        }),
                    ..
                }),
            ..
        }) = cli.command
        else {
            panic!("expected account space delete");
        };
        assert_eq!(subject, did);
        assert!(no_open);
    }

    #[test]
    fn space_is_the_only_public_spelling() {
        let cli = Cli::try_parse_from(["tonk", "space", "link", "garden"]).unwrap();
        let command = cli.command.as_ref().expect("space command");
        let Command::Space {
            command: Some(SpaceCommand::Link { name }),
            ..
        } = command
        else {
            panic!("expected space link");
        };
        assert_eq!(name, "garden");
        assert_eq!(descriptor(command), ("space", Some("link")));

        assert!(Cli::try_parse_from(["tonk", "spot", "link", "garden"]).is_err());
        assert!(Cli::try_parse_from(["tonk", "--spot", "garden", "status"]).is_err());
        assert!(Cli::try_parse_from(["tonk", "account", "spots"]).is_err());

        // Linking is about this installation's one account, so it takes no
        // target; sharing with someone else is `tonk invite`.
        assert!(Cli::try_parse_from(["tonk", "space", "link", "garden", "--to", "work"]).is_err());
        assert!(Cli::try_parse_from(["tonk", "space", "move", "garden"]).is_err());
        assert!(
            Cli::try_parse_from(["tonk", "space", "share", "garden", "--with", "work"]).is_err()
        );
    }
}
