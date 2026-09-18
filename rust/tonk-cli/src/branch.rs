//! `tonk branch` — create, list, delete, switch, merge, and track the
//! branches of the selected space.
//!
//! A space is one dialog repository, and a repository holds any number of
//! named branches. Until now tonk only ever addressed `main` (plus the
//! `meta` branch it keeps its own records on); this module is what makes
//! the rest of them reachable.
//!
//! Three separate things record a branch, and they answer different
//! questions:
//!
//! - **Dialog's cells** (`branch/{name}/revision`, `…/upstream`) are the
//!   branch itself: its head and what it tracks. Creating a branch
//!   publishes a revision; deleting one retracts these.
//! - **The meta branch's [`BranchConcept`] facts** are the queryable
//!   mirror, written for the same reason [`crate::remote`] mirrors
//!   remotes: the browser-side worker reads them (via
//!   `GET /api/repository/{repo}`) to decide which branches to sync, so a
//!   branch tonk creates is one the web UI can reach.
//! - **`head.json` beside the site** is this device's checkout — which
//!   branch the data verbs read and write. Local, like git's `HEAD`: it
//!   never syncs, because what one device is looking at is nobody else's
//!   business.
//!
//! Selection precedence mirrors [`crate::space`]'s: `--branch` >
//! [`BRANCH_ENV`] > `head.json` > [`crate::site::BRANCH_NAME`].

use std::path::{Path, PathBuf};

use dialog_capability::{Provider, Subject};
use dialog_effects::memory::prelude::{
    CellExt as _, MemoryExt as _, MemorySubjectExt as _, SpaceExt as _,
};
use dialog_effects::memory::{Resolve as ResolveCell, Retract as RetractCell};
use dialog_repository::{Branch as DialogBranch, TreeReference, Upstream, Upstreams};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tonk_schema::domain::branch as branch_dom;
use tonk_schema::{Branch as BranchConcept, Replica, TrackingBranch};

use crate::ExitCode;
use crate::remote::{self, META_BRANCH};
use crate::site::{self, TonkSite};

/// Environment variable naming the branch to operate on, sitting between
/// the `--branch` flag and the site's own checkout in precedence.
pub const BRANCH_ENV: &str = "TONK_BRANCH";

/// File beside the site's data recording which branch it is checked out
/// on. Absent means [`crate::site::BRANCH_NAME`].
pub const HEAD_FILE: &str = "head.json";

/// The branches tonk manages on its own behalf and will not hand over.
///
/// `meta` carries the replica's remotes, tracking links, and invitation
/// records. Checking it out would point every data verb at tonk's own
/// bookkeeping, and deleting it would strand the space's remotes — so it
/// is visible in a listing and refused everywhere else.
const RESERVED: [&str; 1] = [META_BRANCH];

/// The persisted checkout. A struct rather than a bare string so a later
/// field (a detached head, a last-branch memory) can be added without
/// rewriting what is already on disk.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct Head {
    /// Name of the checked-out branch.
    branch: String,
}

/// Where a resolved branch name came from, so output and errors can say
/// which mechanism answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// The `--branch` flag.
    Flag,
    /// [`BRANCH_ENV`].
    Env,
    /// The site's [`HEAD_FILE`].
    Checkout,
    /// Nothing selected one, so `main` answered.
    Default,
}

impl std::fmt::Display for Source {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Source::Flag => "flag",
            Source::Env => "env",
            Source::Checkout => "checkout",
            Source::Default => "default",
        })
    }
}

/// A resolved branch selection: the name, and what selected it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selected {
    /// The branch name every data verb will address.
    pub name: String,
    /// Which mechanism named it.
    pub source: Source,
}

/// One row of `tonk branch`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BranchRecord {
    /// The branch's name.
    pub name: String,
    /// Whether this is the branch the site is checked out on.
    pub current: bool,
    /// The branch's tree hash, absent on a branch with no commits.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub head: Option<String>,
    /// What the branch tracks, rendered as `<remote>/<branch>` for a
    /// remote upstream or `<branch>` for a local one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upstream: Option<String>,
}

/// Outcome of [`create`].
#[derive(Debug, Clone)]
pub struct CreateOutcome {
    /// The new branch's name.
    pub name: String,
    /// The start point it was created at, as the caller named it.
    pub start: String,
    /// The head it now points at, absent when the start point had no
    /// commits (an empty branch, like git's orphan).
    pub head: Option<TreeReference>,
    /// Whether the site was also checked out onto it.
    pub switched: bool,
}

/// Outcome of [`delete`].
#[derive(Debug, Clone)]
pub struct DeleteOutcome {
    /// The deleted branch's name.
    pub name: String,
    /// The head it pointed at, for a caller that wants to say what was
    /// dropped.
    pub head: Option<TreeReference>,
}

/// Outcome of [`switch`].
#[derive(Debug, Clone)]
pub struct SwitchOutcome {
    /// The branch now checked out.
    pub name: String,
    /// The branch that was checked out before.
    pub previous: String,
    /// Whether the switch created the branch on the way.
    pub created: bool,
}

/// Outcome of [`merge`].
#[derive(Debug, Clone)]
pub struct MergeOutcome {
    /// The branch merged from.
    pub from: String,
    /// The branch merged into — the checkout.
    pub into: String,
    /// The head after the merge, absent only when both sides were empty.
    pub head: Option<TreeReference>,
    /// Whether the merge changed what the target branch holds.
    ///
    /// Read from the tree root, not from whether dialog ran a merge.
    /// Merging a branch tonk is not yet tracking always runs one — the
    /// sync base starts empty, so there is nothing to skip — and would
    /// otherwise report a branch identical to the checkout as having
    /// brought something in. What the reader wants to know is whether
    /// the facts changed.
    pub advanced: bool,
}

/// Failure modes for branch management.
#[derive(Debug, Error)]
pub enum BranchError {
    /// A branch name that cannot address a branch.
    #[error("invalid branch name '{name}': {reason}")]
    InvalidName {
        /// The offending name.
        name: String,
        /// Why it was refused.
        reason: String,
    },
    /// The named branch does not exist on this replica.
    #[error("no branch named '{0}'; list them with `tonk branch`")]
    Unknown(String),
    /// `create` was asked for a name that is already taken.
    #[error("branch '{0}' already exists")]
    Exists(String),
    /// An operation was aimed at a branch tonk manages itself.
    #[error("'{0}' is tonk's own branch and cannot be checked out, merged, or deleted")]
    Reserved(String),
    /// `delete` was aimed at the branch the site is checked out on.
    #[error(
        "branch '{0}' is checked out; switch to another branch first \
         (`tonk branch switch main`)"
    )]
    CheckedOut(String),
    /// `delete` was aimed at the content branch.
    #[error("branch '{0}' is the space's content branch and cannot be deleted")]
    ContentBranch(String),
    /// `merge` or `set-upstream` was pointed at the branch itself.
    #[error("branch '{0}' cannot track or merge itself")]
    Itself(String),
    /// A remote named in an upstream target is not registered.
    #[error("remote '{0}' is not registered; add it with `tonk remote add` first")]
    UnknownRemote(String),
    /// Anything else — dialog I/O, storage, a failed query.
    #[error("{0}")]
    Io(String),
}

impl crate::Coded for BranchError {
    /// CLI exit code for this failure mode.
    fn exit_code(&self) -> ExitCode {
        match self {
            BranchError::InvalidName { .. } => ExitCode::ParseError,
            _ => ExitCode::IoError,
        }
    }
}

/// Whether `name` can address a branch.
///
/// Deliberately narrower than what dialog would accept. A branch name
/// becomes a directory path under the site (`…/memory/branch/{name}/`)
/// and a URL segment in the web UI (`/space/{branch}@{key}`), so the
/// characters that would break either are refused here rather than
/// somewhere deeper and less explicable. `/` is allowed — `feature/x`
/// reads the way people expect — but never as a first, last, or doubled
/// character, and a `.` component can never escape the branch directory.
pub fn validate_name(name: &str) -> Result<(), BranchError> {
    let reject = |reason: &str| {
        Err(BranchError::InvalidName {
            name: name.to_owned(),
            reason: reason.to_owned(),
        })
    };
    if name.is_empty() {
        return reject("it is empty");
    }
    if name.len() > 128 {
        return reject("it is longer than 128 characters");
    }
    if name.starts_with('/') || name.ends_with('/') {
        return reject("it starts or ends with '/'");
    }
    if name.starts_with('-') {
        return reject("it starts with '-', which reads as a flag");
    }
    if name.contains("//") {
        return reject("it contains an empty path segment ('//')");
    }
    if name.contains('@') {
        return reject("'@' separates the branch from the space in a URL");
    }
    if name
        .split('/')
        .any(|segment| segment == "." || segment == "..")
    {
        return reject("a '.' or '..' segment would escape the branch directory");
    }
    if let Some(bad) = name
        .chars()
        .find(|c| !(c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/')))
    {
        return reject(&format!(
            "'{bad}' is not allowed; use letters, digits, '-', '_', '.', or '/'"
        ));
    }
    Ok(())
}

/// Read the branch a site is checked out on, or `None` when it has never
/// been switched.
///
/// An unreadable or malformed file is an error rather than a silent
/// fallback to `main`: writing to the wrong branch is exactly the mistake
/// a checkout exists to prevent.
pub fn read_head(site_root: &Path) -> Result<Option<String>, BranchError> {
    let path = head_path(site_root);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(BranchError::Io(format!(
                "could not read {}: {error}",
                path.display()
            )));
        }
    };
    let head: Head = serde_json::from_slice(&bytes).map_err(|error| {
        BranchError::Io(format!(
            "{} is not a readable checkout record ({error}); \
             delete it to fall back to '{main}'",
            path.display(),
            main = site::BRANCH_NAME,
        ))
    })?;
    validate_name(&head.branch)?;
    Ok(Some(head.branch))
}

/// Record `name` as the site's checkout.
pub fn write_head(site_root: &Path, name: &str) -> Result<(), BranchError> {
    validate_name(name)?;
    let path = head_path(site_root);
    let bytes = serde_json::to_vec_pretty(&Head {
        branch: name.to_owned(),
    })
    .map_err(|error| BranchError::Io(format!("could not encode the checkout: {error}")))?;
    std::fs::write(&path, bytes)
        .map_err(|error| BranchError::Io(format!("could not write {}: {error}", path.display())))
}

/// Resolve which branch an invocation addresses.
///
/// `flag` is `--branch`, `env` is [`BRANCH_ENV`], and the site's
/// [`HEAD_FILE`] answers when neither does. Pure over its inputs so the
/// precedence is testable without a process environment or a site.
pub fn resolve(
    flag: Option<&str>,
    env: Option<&str>,
    checkout: Option<String>,
) -> Result<Selected, BranchError> {
    if let Some(name) = flag {
        validate_name(name)?;
        return Ok(Selected {
            name: name.to_owned(),
            source: Source::Flag,
        });
    }
    if let Some(name) = env.filter(|value| !value.trim().is_empty()) {
        let name = name.trim();
        validate_name(name)?;
        return Ok(Selected {
            name: name.to_owned(),
            source: Source::Env,
        });
    }
    match checkout {
        Some(name) => Ok(Selected {
            name,
            source: Source::Checkout,
        }),
        None => Ok(Selected {
            name: site::BRANCH_NAME.to_owned(),
            source: Source::Default,
        }),
    }
}

/// Read [`BRANCH_ENV`] from the process environment.
pub fn branch_from_environment() -> Option<String> {
    std::env::var(BRANCH_ENV).ok()
}

/// Every branch on this replica, with its head and upstream, sorted by
/// name.
///
/// Two sources are unioned, because neither alone is complete. The
/// on-disk scan sees every branch that has ever been committed to,
/// including ones created by the browser or by an older tonk that wrote
/// no meta record. The meta-branch facts see branches created but never
/// committed to, which have no revision cell to find. A branch in both
/// appears once.
pub async fn list(site: &TonkSite) -> Result<Vec<BranchRecord>, BranchError> {
    let current = site.head().to_owned();
    let mut rows = Vec::new();
    for name in branch_names(site).await? {
        let branch = open(site, &name).await?;
        rows.push(BranchRecord {
            current: name == current,
            head: branch.revision().map(|revision| revision.tree.to_string()),
            upstream: branch.upstream().as_ref().map(render_upstream),
            name,
        });
    }
    Ok(rows)
}

/// Every branch name on this replica, sorted, unioned from the meta
/// records and the on-disk revision cells, plus the checkout.
///
/// The checkout is always in the set even before its first commit, so a
/// fresh `tonk branch switch -c` is visible in the listing that follows
/// it.
async fn branch_names(site: &TonkSite) -> Result<Vec<String>, BranchError> {
    let mut names: std::collections::BTreeSet<String> = recorded(site).await?;
    names.extend(stored(&site.root)?);
    names.insert(site.head().to_owned());
    Ok(names.into_iter().collect())
}

/// Create `name` at `start`, and check the site out onto it when
/// `switch` is set.
///
/// `start` names an existing branch, exactly as `git branch <name>
/// [<start-point>]` does: a local branch, or `<remote>/<branch>` for one
/// on a registered remote. `None` starts from the checkout. A start point
/// with no commits yields an empty branch rather than an error — the same
/// state a repository's first branch is in before anything is written.
///
/// There is no "create at an arbitrary tree hash": a head is a signed
/// claim by the session that minted it, so tonk cannot synthesize one
/// pointing at a tree of the caller's choosing. Naming a branch is the
/// whole vocabulary of start points there is.
pub async fn create(
    site: &TonkSite,
    name: &str,
    start: Option<&str>,
    switch_to: bool,
) -> Result<CreateOutcome, BranchError> {
    validate_name(name)?;
    if RESERVED.contains(&name) {
        return Err(BranchError::Reserved(name.to_owned()));
    }
    if exists(site, name).await? {
        return Err(BranchError::Exists(name.to_owned()));
    }

    let start = start.unwrap_or(site.head());
    let revision = start_revision(site, start).await?;

    // Point the new branch at the start point's head before recording
    // it, so a failure here leaves nothing claiming the branch exists.
    // `reset` is a plain cell publish; a branch that has never been
    // written to has no head to rewind past, which is the one case
    // dialog's reset is unreserved about.
    let branch = open(site, name).await?;
    if let Some(revision) = revision.clone() {
        branch
            .reset(revision)
            .perform(&site.operator)
            .await
            .map_err(|error| {
                BranchError::Io(format!("could not point '{name}' at '{start}': {error}"))
            })?;
    }
    record(site, name).await?;

    let mut outcome = CreateOutcome {
        name: name.to_owned(),
        start: start.to_owned(),
        head: revision.map(|revision| revision.tree),
        switched: false,
    };
    if switch_to {
        write_head(&site.root, name)?;
        outcome.switched = true;
    }
    Ok(outcome)
}

/// Delete `name`: drop its cells and retract its meta records.
///
/// Refuses the content branch, tonk's own [`RESERVED`] branches, and the
/// branch the site is checked out on. Everything else goes, head and all
/// — a branch's commits are only reachable through its head, so this is
/// the one tonk operation that can lose work that was never pushed. The
/// caller is expected to have confirmed.
pub async fn delete(site: &TonkSite, name: &str) -> Result<DeleteOutcome, BranchError> {
    validate_name(name)?;
    if name == site::BRANCH_NAME {
        return Err(BranchError::ContentBranch(name.to_owned()));
    }
    if RESERVED.contains(&name) {
        return Err(BranchError::Reserved(name.to_owned()));
    }
    if name == site.head() {
        return Err(BranchError::CheckedOut(name.to_owned()));
    }
    if !exists(site, name).await? {
        return Err(BranchError::Unknown(name.to_owned()));
    }

    let head = open(site, name)
        .await?
        .revision()
        .map(|revision| revision.tree);

    // Retract the meta records first. They are what the browser reads to
    // decide what to sync, so a half-finished delete that left them
    // behind would keep re-mounting a branch whose cells are gone.
    forget(site, name).await?;
    forget_upstreams_of(site, name).await?;
    drop_cells(site, name).await?;

    Ok(DeleteOutcome {
        name: name.to_owned(),
        head,
    })
}

/// Check the site out onto `name`, creating it from the current checkout
/// first when `create_missing` is set.
pub async fn switch(
    site: &TonkSite,
    name: &str,
    create_missing: bool,
) -> Result<SwitchOutcome, BranchError> {
    validate_name(name)?;
    if RESERVED.contains(&name) {
        return Err(BranchError::Reserved(name.to_owned()));
    }
    let previous = site.head().to_owned();
    let present = exists(site, name).await?;
    let created = match (present, create_missing) {
        (true, _) => false,
        (false, true) => {
            create(site, name, None, false).await?;
            true
        }
        (false, false) => {
            return Err(BranchError::Unknown(name.to_owned()));
        }
    };
    write_head(&site.root, name)?;
    Ok(SwitchOutcome {
        name: name.to_owned(),
        previous,
        created,
    })
}

/// Merge `name` into the checked-out branch.
///
/// This is dialog's pull with a local source, which is the same
/// three-way merge a remote pull runs: both sides' claims are integrated
/// by causality, so there is no conflicted state to resolve by hand and
/// no merge to abort. A branch that has nothing the checkout lacks is a
/// no-op, reported as such rather than as a failure.
pub async fn merge(site: &TonkSite, name: &str) -> Result<MergeOutcome, BranchError> {
    validate_name(name)?;
    let into = site.head().to_owned();
    if name == into {
        return Err(BranchError::Itself(name.to_owned()));
    }
    if RESERVED.contains(&name) {
        return Err(BranchError::Reserved(name.to_owned()));
    }
    if !exists(site, name).await? {
        return Err(BranchError::Unknown(name.to_owned()));
    }

    let source = open(site, name).await?;
    let target = open(site, &into).await?;
    // A pull records the source as a tracked upstream so the next merge
    // from it is incremental. That is a free win for a branch that
    // already tracks something — the entry appends behind the default —
    // but on a branch tracking nothing it becomes the default, and
    // `tonk push` would then aim at a local branch nobody asked it to
    // push to. So a merge onto an untracked branch puts the tracking
    // state back afterwards: a slower second merge is a better trade
    // than a push that silently changed target.
    let tracked_before = !target.upstreams().is_empty();
    let before = target.revision().map(|revision| revision.tree);
    target
        .pull()
        .from(&source)
        .perform(&site.operator)
        .await
        .map_err(|error| {
            BranchError::Io(format!("could not merge '{name}' into '{into}': {error}"))
        })?;
    if !tracked_before && !target.upstreams().is_empty() {
        publish_upstreams(site, &into, Upstreams::default()).await?;
    }

    let after = target.revision().map(|revision| revision.tree);
    Ok(MergeOutcome {
        from: name.to_owned(),
        into,
        advanced: after != before,
        head: after,
    })
}

/// Overwrite a branch's tracking entries.
///
/// Dialog's `set_upstream` only ever adds, so removing an entry means
/// writing the whole set. The cell is resolved first: a publish CASes
/// against the version it last saw, and a freshly built handle has seen
/// none, which would read as "expect this cell to be empty".
async fn publish_upstreams(
    site: &TonkSite,
    name: &str,
    upstreams: Upstreams,
) -> Result<(), BranchError> {
    let cell = site.repository.branch(name).upstream();
    cell.resolve()
        .perform(&site.operator)
        .await
        .map_err(|error| BranchError::Io(format!("could not read '{name}'s upstream: {error}")))?;
    cell.publish(upstreams)
        .perform(&site.operator)
        .await
        .map_err(|error| {
            BranchError::Io(format!("could not rewrite '{name}'s upstream: {error}"))
        })?;
    // The reactor caches an opened branch and its upstream cell with it;
    // a later acquire in this process would otherwise serve the set that
    // was just replaced.
    site.reactor.evict(site::REPO_NAME);
    Ok(())
}

/// Drop every tracking entry naming `deleted` from the other branches on
/// this replica.
///
/// A branch that tracks one that no longer exists is worse than one that
/// tracks nothing: push and pull would resolve an upstream, find no head
/// behind it, and fail in dialog's vocabulary rather than tonk's.
async fn forget_upstreams_of(site: &TonkSite, deleted: &str) -> Result<(), BranchError> {
    let target = Upstream::Local {
        branch: deleted.to_owned(),
        tree: TreeReference::default(),
    };
    for name in branch_names(site).await? {
        if name == deleted {
            continue;
        }
        let branch = open(site, &name).await?;
        let upstreams = branch.upstreams();
        if upstreams.find(&target).is_none() {
            continue;
        }
        let mut kept = Upstreams::default();
        for entry in upstreams.iter().filter(|entry| !entry.same_target(&target)) {
            kept.upsert(entry.clone());
        }
        publish_upstreams(site, &name, kept).await?;
    }
    Ok(())
}

/// Point `branch` (the checkout when `None`) at `target`.
///
/// `target` is `<remote>/<branch>` for a branch on a registered remote,
/// a bare `<remote>` for the same branch name on it, or a local branch
/// name. The dialog-side upstream and the meta-branch [`TrackingBranch`]
/// record are written together, so the browser sees the same tracking
/// relationship tonk pushes through.
pub async fn set_upstream(
    site: &TonkSite,
    branch: Option<&str>,
    target: &str,
) -> Result<remote::UpstreamOutcome, BranchError> {
    let local = branch.unwrap_or(site.head());
    validate_name(local)?;
    if !exists(site, local).await? {
        return Err(BranchError::Unknown(local.to_owned()));
    }

    // A local branch name wins over a remote name only when no remote
    // answers to it: `origin` is a remote everywhere it is registered,
    // and a branch someone also called `origin` is the ambiguity they
    // created. Splitting on the first `/` keeps `origin/feature/x`
    // readable as "branch feature/x on remote origin".
    let (remote_name, remote_branch) = match target.split_once('/') {
        Some((remote_name, rest)) => (remote_name.to_owned(), rest.to_owned()),
        None => (target.to_owned(), local.to_owned()),
    };
    if remote::find(site, &remote_name)
        .await
        .map_err(|error| BranchError::Io(error.to_string()))?
        .is_none()
    {
        // Not a remote — the only other thing a target can name is a
        // local branch on this replica.
        if target == local {
            return Err(BranchError::Itself(local.to_owned()));
        }
        if exists(site, target).await? {
            return set_local_upstream(site, local, target).await;
        }
        return Err(BranchError::UnknownRemote(remote_name));
    }

    remote::set_upstream_for(site, local, &remote_name, &remote_branch)
        .await
        .map_err(|error| BranchError::Io(error.to_string()))
}

/// Track another branch in this same repository.
async fn set_local_upstream(
    site: &TonkSite,
    local: &str,
    target: &str,
) -> Result<remote::UpstreamOutcome, BranchError> {
    let source = open(site, target).await?;
    let branch = open(site, local).await?;
    branch
        .set_upstream(&source)
        .perform(&site.operator)
        .await
        .map_err(|error| {
            BranchError::Io(format!(
                "could not point '{local}' at local branch '{target}': {error}"
            ))
        })?;

    let replica = local_replica(site);
    let tracked = replica.branch(target);
    let tracking = replica.branch(local).set_upstream(&tracked);
    commit_meta(site, |transaction| {
        transaction
            .assert(replica.branch(local))
            .assert(tracked.clone())
            .assert(tracking.clone())
    })
    .await?;

    Ok(remote::UpstreamOutcome {
        local_branch: local.to_owned(),
        remote: String::new(),
        remote_branch: target.to_owned(),
    })
}

/// Whether `name` names a branch this replica knows about.
pub async fn exists(site: &TonkSite, name: &str) -> Result<bool, BranchError> {
    if name == site::BRANCH_NAME || RESERVED.contains(&name) {
        // Tonk's own branches exist by construction: they are opened on
        // first use, whether or not anything has been written to them.
        return Ok(true);
    }
    if stored(&site.root)?.contains(name) {
        return Ok(true);
    }
    Ok(recorded(site).await?.contains(name))
}

/// Open a branch handle by name, through the reactor so its caches are
/// shared with everything else addressing the same branch.
async fn open(site: &TonkSite, name: &str) -> Result<DialogBranch, BranchError> {
    let session = site
        .named_branch(name)
        .await
        .map_err(|error| BranchError::Io(format!("could not open branch '{name}': {error}")))?;
    Ok(session.handle().clone())
}

/// The head a new branch starts at, resolved from a start-point name.
async fn start_revision(
    site: &TonkSite,
    start: &str,
) -> Result<Option<dialog_repository::Revision>, BranchError> {
    if let Some((remote_name, remote_branch)) = start.split_once('/')
        && let Some(_record) = remote::find(site, remote_name)
            .await
            .map_err(|error| BranchError::Io(error.to_string()))?
    {
        return remote_head(site, remote_name, remote_branch).await;
    }
    if !exists(site, start).await? {
        return Err(BranchError::Unknown(start.to_owned()));
    }
    Ok(open(site, start).await?.revision())
}

/// The head a remote branch currently publishes.
///
/// Resolved over the network: a start point on a remote is only useful
/// if it is the remote's current head, not whatever this device last
/// fetched.
async fn remote_head(
    site: &TonkSite,
    remote_name: &str,
    remote_branch: &str,
) -> Result<Option<dialog_repository::Revision>, BranchError> {
    let handle = site
        .repository
        .remote(remote_name)
        .load()
        .perform(&site.operator)
        .await
        .map_err(|error| {
            BranchError::Io(format!("could not load remote '{remote_name}': {error}"))
        })?;
    let branch = handle
        .branch(remote_branch)
        .open()
        .perform(&site.operator)
        .await
        .map_err(|error| {
            BranchError::Io(format!(
                "could not open '{remote_name}/{remote_branch}': {error}"
            ))
        })?;
    Ok(branch.revision())
}

/// Render an upstream the way `tonk branch` prints it.
fn render_upstream(upstream: &Upstream) -> String {
    match upstream {
        Upstream::Local { branch, .. } => branch.clone(),
        Upstream::Remote { remote, branch, .. } => format!("{remote}/{branch}"),
    }
}

/// The branch names this replica has recorded on the meta branch.
async fn recorded(site: &TonkSite) -> Result<std::collections::BTreeSet<String>, BranchError> {
    use dialog_query::{Output as _, Query, Term};

    let meta = open_meta(site).await?;
    let replica = local_replica(site);
    let rows: Vec<BranchConcept> = meta
        .query()
        .select(Query::<BranchConcept> {
            this: Term::var("this"),
            name: Term::var("name"),
            origin: Term::from(branch_dom::Origin::from(replica.this().clone())),
        })
        .perform(&site.operator)
        .try_vec()
        .await
        .map_err(|error| BranchError::Io(format!("branch enumeration failed: {error:?}")))?;
    Ok(rows.into_iter().map(|row| row.name.0).collect())
}

/// The branch names that have a revision record on disk.
///
/// Dialog writes each branch's cells under `{repo}/memory/branch/{name}/`,
/// so a directory holding a `revision` file is a branch that has been
/// committed to. Names may nest (`feature/x`), so the walk descends
/// rather than reading one level.
///
/// A missing branch directory is an empty set, not an error: a
/// repository nothing has been written to has no branches on disk.
fn stored(site_root: &Path) -> Result<std::collections::BTreeSet<String>, BranchError> {
    let root = site_root
        .join(site::REPO_NAME)
        .join("memory")
        .join("branch");
    let mut found = std::collections::BTreeSet::new();
    walk(&root, &root, &mut found)?;
    Ok(found)
}

/// Depth-first walk collecting every directory that holds a `revision`
/// record, named relative to `root`.
fn walk(
    root: &Path,
    directory: &Path,
    found: &mut std::collections::BTreeSet<String>,
) -> Result<(), BranchError> {
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(BranchError::Io(format!(
                "could not read {}: {error}",
                directory.display()
            )));
        }
    };
    for entry in entries {
        let entry = entry.map_err(|error| {
            BranchError::Io(format!(
                "could not read an entry of {}: {error}",
                directory.display()
            ))
        })?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        if path.join("revision").is_file()
            && let Some(name) = relative_name(root, &path)
        {
            found.insert(name);
            // A branch directory holds cells, not nested branches: `a`
            // and `a/b` cannot both be branches, because `a`'s cells
            // would sit where `a/b`'s directory belongs.
            continue;
        }
        walk(root, &path, found)?;
    }
    Ok(())
}

/// A branch directory's path relative to the branch root, as a name.
///
/// `None` for a path that is not under the root or does not render as
/// UTF-8 — neither can be a name tonk wrote, so neither belongs in a
/// listing.
fn relative_name(root: &Path, path: &Path) -> Option<String> {
    let relative = path.strip_prefix(root).ok()?;
    let mut name = String::new();
    for component in relative.components() {
        let std::path::Component::Normal(segment) = component else {
            return None;
        };
        if !name.is_empty() {
            name.push('/');
        }
        name.push_str(segment.to_str()?);
    }
    (!name.is_empty()).then_some(name)
}

/// Assert `name`'s meta-branch record, so the browser-side worker can
/// see the branch and decide to sync it.
async fn record(site: &TonkSite, name: &str) -> Result<(), BranchError> {
    let replica = local_replica(site);
    commit_meta(site, |transaction| {
        transaction
            .assert(replica.clone())
            .assert(replica.branch(name))
    })
    .await
}

/// Retract `name`'s meta-branch records — the branch itself and any
/// tracking link hanging off it.
async fn forget(site: &TonkSite, name: &str) -> Result<(), BranchError> {
    use dialog_query::{Output as _, Query, Term};

    let meta = open_meta(site).await?;
    let replica = local_replica(site);
    let concept = replica.branch(name);
    let tracking: Vec<TrackingBranch> = meta
        .query()
        .select(Query::<TrackingBranch> {
            this: Term::from(concept.this.clone()),
            upstream: Term::var("upstream"),
            origin: Term::var("origin"),
        })
        .perform(&site.operator)
        .try_vec()
        .await
        .map_err(|error| BranchError::Io(format!("tracking enumeration failed: {error:?}")))?;

    let mut transaction = meta.transaction().retract(concept);
    for link in tracking {
        transaction = transaction.retract(link);
    }
    transaction
        .commit()
        .publish()
        .perform(&site.operator)
        .await
        .map_err(|error| {
            BranchError::Io(format!(
                "could not retract meta records for '{name}': {error}"
            ))
        })?;
    Ok(())
}

/// Drop every cell dialog keeps for `name`.
///
/// Retraction is CAS'd against the version the cell currently holds, so
/// each one is resolved first; a cell that holds nothing is already gone
/// and is skipped. This addresses the cells directly rather than through
/// a `Branch` handle because dialog exposes no branch-level delete —
/// publishing is the only write a `Branch` offers, and a branch whose
/// head was overwritten rather than removed would keep answering reads.
async fn drop_cells(site: &TonkSite, name: &str) -> Result<(), BranchError> {
    let space = Subject::from(site.repository.did())
        .memory()
        .space(format!("branch/{name}"));
    for cell in ["revision", "upstream", "induction"] {
        let handle = space.clone().cell(cell);
        let current = Provider::<ResolveCell>::execute(&site.operator, handle.clone().resolve())
            .await
            .map_err(|error| {
                BranchError::Io(format!("could not read '{name}'s {cell} cell: {error}"))
            })?;
        let Some(edition) = current else {
            continue;
        };
        Provider::<RetractCell>::execute(&site.operator, handle.retract(edition.version))
            .await
            .map_err(|error| {
                BranchError::Io(format!("could not drop '{name}'s {cell} cell: {error}"))
            })?;
    }
    // The reactor caches an opened branch by name; a later acquire in
    // this process would otherwise serve the handle whose cells were
    // just retracted.
    site.reactor.evict(site::REPO_NAME);
    Ok(())
}

/// Run one meta-branch transaction built by `build`.
async fn commit_meta<F>(site: &TonkSite, build: F) -> Result<(), BranchError>
where
    F: FnOnce(
        dialog_repository::Transaction<&DialogBranch>,
    ) -> dialog_repository::Transaction<&DialogBranch>,
{
    let meta = open_meta(site).await?;
    build(meta.transaction())
        .commit()
        .publish()
        .perform(&site.operator)
        .await
        .map_err(|error| BranchError::Io(format!("could not write meta records: {error}")))?;
    Ok(())
}

/// Open the repository's meta branch.
async fn open_meta(site: &TonkSite) -> Result<DialogBranch, BranchError> {
    site.repository
        .branch(META_BRANCH)
        .open()
        .perform(&site.operator)
        .await
        .map_err(|error| BranchError::Io(format!("could not open the meta branch: {error}")))
}

/// This site's replica concept, the origin every meta record hangs off.
fn local_replica(site: &TonkSite) -> Replica {
    Replica::new(site.profile.did(), site.repository.did())
}

/// Path of the checkout record beside a site's data.
fn head_path(site_root: &Path) -> PathBuf {
    site_root.join(HEAD_FILE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_accepts_the_names_people_actually_use() {
        for name in ["main", "test", "feature/x", "v1.2", "fix_42", "a-b"] {
            validate_name(name).unwrap_or_else(|error| panic!("{name} rejected: {error}"));
        }
    }

    #[test]
    fn it_refuses_names_that_would_escape_the_branch_directory() {
        for name in ["", "/x", "x/", "a//b", "..", "a/../b", "a/./b"] {
            assert!(
                validate_name(name).is_err(),
                "'{name}' should not be a branch name"
            );
        }
    }

    #[test]
    fn it_refuses_an_at_sign_because_the_url_reserves_it() {
        // `/space/{branch}@{key}` splits on the first `@`, so a branch
        // carrying one could never be addressed in the web UI.
        let error = validate_name("feat@x").unwrap_err();
        assert!(error.to_string().contains('@'), "{error}");
    }

    #[test]
    fn it_prefers_the_flag_over_everything_else() {
        let selected = resolve(Some("flagged"), Some("envd"), Some("checked".into())).unwrap();
        assert_eq!(selected.name, "flagged");
        assert_eq!(selected.source, Source::Flag);
    }

    #[test]
    fn it_prefers_the_environment_over_the_checkout() {
        let selected = resolve(None, Some("envd"), Some("checked".into())).unwrap();
        assert_eq!(selected.name, "envd");
        assert_eq!(selected.source, Source::Env);
    }

    #[test]
    fn it_reads_an_empty_environment_value_as_unset() {
        // An exported-but-empty `TONK_BRANCH` is the shape a shell
        // leaves behind; reading it as a branch name would fail every
        // command with an invalid-name error nobody wrote.
        let selected = resolve(None, Some("  "), Some("checked".into())).unwrap();
        assert_eq!(selected.name, "checked");
        assert_eq!(selected.source, Source::Checkout);
    }

    #[test]
    fn it_falls_back_to_main_when_nothing_selects_a_branch() {
        let selected = resolve(None, None, None).unwrap();
        assert_eq!(selected.name, site::BRANCH_NAME);
        assert_eq!(selected.source, Source::Default);
    }

    #[test]
    fn it_round_trips_the_checkout_record() {
        let root = tempfile::tempdir().unwrap();
        assert_eq!(read_head(root.path()).unwrap(), None);
        write_head(root.path(), "feature/x").unwrap();
        assert_eq!(
            read_head(root.path()).unwrap().as_deref(),
            Some("feature/x")
        );
    }

    #[test]
    fn it_refuses_a_corrupt_checkout_instead_of_assuming_main() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(head_path(root.path()), b"{").unwrap();
        let error = read_head(root.path()).unwrap_err();
        assert!(error.to_string().contains(HEAD_FILE), "{error}");
    }

    #[test]
    fn it_finds_nested_branch_directories_by_their_revision_record() {
        let site = tempfile::tempdir().unwrap();
        let root = site
            .path()
            .join(site::REPO_NAME)
            .join("memory")
            .join("branch");
        for name in ["main", "meta", "feature/x"] {
            let directory = root.join(name);
            std::fs::create_dir_all(&directory).unwrap();
            std::fs::write(directory.join("revision"), b"head").unwrap();
        }
        // A directory with no `revision` record is not a branch — it is
        // where a nested branch's parent segment lives.
        std::fs::create_dir_all(root.join("wip").join("later")).unwrap();

        let found = stored(site.path()).unwrap();
        assert_eq!(
            found.into_iter().collect::<Vec<_>>(),
            vec!["feature/x", "main", "meta"]
        );
    }

    #[test]
    fn it_reports_no_stored_branches_for_a_site_with_no_data() {
        let site = tempfile::tempdir().unwrap();
        assert!(stored(site.path()).unwrap().is_empty());
    }
}
