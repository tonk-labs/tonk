//! Where the standard library lives, so its documents can `!include`.
//!
//! The service worker already fetches the library from its own origin
//! (`/library/core.yaml`), so a library document has a real location:
//! the served `/library/` directory. Documents are parsed there and an
//! `!include` resolves against it the way a relative URL in a served
//! page would, so `./table/connected.js` beside `table.yaml` is fetched
//! from `/library/table/connected.js`, the same served asset set.
//!
//! The location is the directory rather than one file because seeds are
//! composed: a scaffold, a generated snippet naming the repository, an
//! agent supplement, concatenated into one body. Every library document
//! sits in that one directory, so relative references resolve alike
//! whichever file they were written in.
//!
//! Natively there is no origin, and the library documents are compiled
//! in, so they live at [`tonk_library::ROOT`] and what they include is
//! served from the copies `tonk-library` bundles. Reading the source
//! tree instead would work on a developer's machine and nowhere else:
//! CI runs native suites from a test archive that does not carry it.
//!
//! Only the library is loaded this way. The loader refuses anything
//! outside the library directory, so a library document cannot reach
//! the data plane (`/api/...`) or another origin by including it.

use tonk_notation::{Load, Syntax, Url, expand, parse_at};

use crate::TonkWorkerError;

/// Where library documents live, or `None` where the library has no
/// location: a wasm test harness is not a service worker, so it has no
/// origin to serve from. A document parsed without a location still
/// seeds; it just cannot include.
pub(super) fn location() -> Option<Url> {
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    {
        // The directory the service worker serves the library from.
        let origin = super::repository::worker_origin()?;
        Url::parse(&format!("{origin}/library/")).ok()
    }
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    {
        Url::parse(tonk_library::ROOT).ok()
    }
}

/// Loads what a library document includes, from the library directory
/// and nowhere else.
pub(super) struct Library {
    root: Url,
}

impl Library {
    fn new(root: Url) -> Self {
        Self { root }
    }

    fn contains(&self, uri: &Url) -> bool {
        within(&self.root, uri)
    }
}

impl Load for Library {
    async fn load(&self, uri: &Url) -> Result<Vec<u8>, String> {
        if !self.contains(uri) {
            return Err(format!(
                "a library document can only include files from `{}`",
                self.root
            ));
        }
        read(uri).await
    }
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn read(uri: &Url) -> Result<Vec<u8>, String> {
    super::repository::fetch_library_bytes(uri.path())
        .await
        .map_err(|error| error.to_string())
}

#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
async fn read(uri: &Url) -> Result<Vec<u8>, String> {
    tonk_library::Bundled.load(uri).await
}

/// Whether `uri` is inside the directory `root`: same origin, a path
/// under the root's, and no query. `Url` has already normalized `..`
/// segments away, so a prefix test cannot be walked out of.
pub(super) fn within(root: &Url, uri: &Url) -> bool {
    uri.scheme() == root.scheme()
        && uri.host_str() == root.host_str()
        && uri.port_or_known_default() == root.port_or_known_default()
        && uri.path().starts_with(root.path())
        && uri.query().is_none()
}

/// Parse a library body at the library's location and inline what it
/// includes. A parse or include failure is a deployment fault, not a
/// client one, and surfaces as an internal error naming the first
/// diagnostic.
pub(super) async fn parse(text: &str) -> Result<Syntax, TonkWorkerError> {
    let Some(root) = location() else {
        return parsed(tonk_notation::parse(text));
    };
    let mut syntax = parsed(parse_at(root.clone(), text))?;
    let unexpanded = expand(&mut syntax, &Library::new(root)).await;
    if let Some(first) = unexpanded.first() {
        return Err(TonkWorkerError::Internal(format!(
            "library include failed at {}:{}: {}",
            first.range.start.line + 1,
            first.range.start.character + 1,
            first.message
        )));
    }
    Ok(syntax)
}

fn parsed(parsed: tonk_notation::Parsed) -> Result<Syntax, TonkWorkerError> {
    if let Some(first) = parsed.diagnostics.first() {
        return Err(TonkWorkerError::Internal(format!(
            "library does not parse at {}:{}: {}",
            first.range.start.line + 1,
            first.range.start.character + 1,
            first.message
        )));
    }
    parsed
        .syntax
        .ok_or_else(|| TonkWorkerError::Internal("library is empty".to_owned()))
}

/// The library's components a space can install from the palette
/// (`library/install`): every library document but the two each branch is
/// seeded with, `core.yaml` (every space) and `profile.yaml` (the profile
/// branch). Only these: a component names a library file by its name, so
/// the list is what keeps an install to the library.
pub(super) const COMPONENTS: [&str; 6] = [
    "issue",
    "meta",
    "notebook",
    "prose",
    "table",
    "version-control",
];

/// Whether `source` is a component's library file (`/library/<name>.yaml`).
pub(super) fn is_component_url(source: &str) -> bool {
    source
        .strip_prefix("/library/")
        .and_then(|file| file.strip_suffix(".yaml"))
        .is_some_and(|name| COMPONENTS.contains(&name))
}

/// The library file a component entity (`tonk:library/<name>`) names, when
/// it is one of [`COMPONENTS`].
pub(super) fn component_url(component: &str) -> Option<String> {
    let name = component.strip_prefix("tonk:library/")?;
    COMPONENTS
        .contains(&name)
        .then(|| format!("/library/{name}.yaml"))
}

/// Install a library component into the space it was asked in, or bring
/// the installed one up to the shipped version. Like the space's seed, a
/// component is recorded and upgraded on mount; unlike it, a space can have
/// several (see `repository::install_component`). Its catalog lines join
/// the space's AGENTS.md, and what came of the request is recorded as a
/// [`LibraryOutcome`](tonk_schema::LibraryOutcome) on the request's entity.
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl dialog_capability::Provider<tonk_schema::command::InstallComponent>
    for crate::router::CommandEnv
{
    async fn execute(&self, command: tonk_schema::command::InstallComponent) {
        let request = Request {
            this: command.this,
            component: command.component.0,
            time: command.time.0,
            action: Action::Install,
        };
        let (repo, branch) = (self.origin().repo.clone(), self.origin().branch.clone());
        let tonk = self.state().read().await;
        settle(&tonk, &repo, &branch, request).await;
    }
}

/// Uninstall a library component from the space it was asked in: the
/// inverse of `library/install`, so undoing an install is asking this.
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl dialog_capability::Provider<tonk_schema::command::UninstallComponent>
    for crate::router::CommandEnv
{
    async fn execute(&self, command: tonk_schema::command::UninstallComponent) {
        let request = Request {
            this: command.this,
            component: command.component.0,
            time: command.time.0,
            action: Action::Uninstall,
        };
        let (repo, branch) = (self.origin().repo.clone(), self.origin().branch.clone());
        let tonk = self.state().read().await;
        settle(&tonk, &repo, &branch, request).await;
    }
}

/// What a request asks: install or uninstall.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Install the component, or bring it up to date.
    Install,
    /// Withdraw what its install added.
    Uninstall,
}

impl Action {
    /// "install" or "uninstall".
    pub fn word(self) -> &'static str {
        match self {
            Action::Install => "install",
            Action::Uninstall => "uninstall",
        }
    }

    #[cfg(any(feature = "volatile-storage", test))]
    /// The action an undo of this one runs.
    pub fn inverse(self) -> Self {
        match self {
            Action::Install => Action::Uninstall,
            Action::Uninstall => Action::Install,
        }
    }

    #[cfg(any(feature = "volatile-storage", test))]
    /// The action `word` names.
    pub fn from_word(word: &str) -> Option<Self> {
        match word {
            "install" => Some(Action::Install),
            "uninstall" => Some(Action::Uninstall),
            _ => None,
        }
    }
}

/// One install or uninstall request, as its command carried it.
pub(crate) struct Request {
    pub(crate) this: dialog_artifacts::Entity,
    pub(crate) component: dialog_artifacts::Entity,
    pub(crate) time: f64,
    pub(crate) action: Action,
}

#[cfg(any(feature = "volatile-storage", test))]
/// Run `request` against branch `branch` of space `repo` now, as its
/// command would, and answer the outcome it recorded: what the hosted
/// writer awaits, where a command's dispatch would answer before it ran.
pub(crate) async fn run_now(
    tonk: &crate::worker::TonkState,
    repo: &str,
    branch: &str,
    request: Request,
) -> Result<tonk_schema::LibraryOutcome, String> {
    settle(tonk, repo, branch, request)
        .await
        .ok_or_else(|| "the outcome could not be recorded".to_owned())
}

#[cfg(any(feature = "volatile-storage", test))]
/// What the catalog says of `component` on branch `branch` of space
/// `repo`: its name, label and version, the AGENTS.md lines it adds, and
/// one line per part it adds (`library/manifest`). Empty where the
/// space's core predates them.
pub(crate) async fn catalog(
    tonk: &crate::worker::TonkState,
    repo: &str,
    branch: &str,
    component: &dialog_artifacts::Entity,
) -> Result<Catalog, String> {
    use dialog_query::{Output as _, Query, Term};

    let session = open(tonk, repo, branch).await?;
    let entry = session
        .handle()
        .query()
        .select(Query::<tonk_schema::LibraryComponent> {
            this: Term::from(component.clone()),
            name: Term::var("name"),
            label: Term::var("label"),
            version: Term::var("version"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .map_err(|e| format!("{e:?}"))?
        .into_iter()
        .next();
    let (_, agents) = catalog_entry(tonk, &session, component).await?;
    let adds = session
        .handle()
        .query()
        .select(Query::<tonk_schema::LibraryManifest> {
            this: Term::from(component.clone()),
            adds: Term::var("adds"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .map_err(|e| format!("{e:?}"))?
        .into_iter()
        .next()
        .map(|row| agent_lines(&row.adds.0))
        .unwrap_or_default();
    let fallback = component
        .to_string()
        .strip_prefix("tonk:library/")
        .unwrap_or_default()
        .to_owned();
    Ok(Catalog {
        name: entry
            .as_ref()
            .map(|entry| entry.name.0.clone())
            .unwrap_or_else(|| fallback.clone()),
        label: entry
            .as_ref()
            .map(|entry| entry.label.0.clone())
            .unwrap_or(fallback),
        version: entry.map(|entry| entry.version.0),
        agents: agents.as_deref().map(agent_lines).unwrap_or_default(),
        adds,
    })
}

#[cfg(any(feature = "volatile-storage", test))]
/// What the catalog says of one component.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Catalog {
    /// The component's file name without `.yaml`.
    pub name: String,
    /// What a person calls it.
    pub label: String,
    /// Its release, where the space's core says one.
    pub version: Option<String>,
    /// Its AGENTS.md lines.
    pub agents: Vec<String>,
    /// One line per part it adds: "concept x", "command x", "view x slot".
    pub adds: Vec<String>,
}

/// Run `request` against branch `branch` of space `repo` and record what
/// came of it, with the AGENTS.md edit it makes, in one commit.
async fn settle(
    tonk: &crate::worker::TonkState,
    repo: &str,
    branch: &str,
    request: Request,
) -> Option<tonk_schema::LibraryOutcome> {
    let settled = run(tonk, repo, branch, &request).await;
    let word = request.action.word();
    let outcome = match record(tonk, repo, branch, &request, &settled).await {
        Ok(outcome) => outcome,
        Err(error) => {
            tonk_common::log!("library/{word}: record outcome in {repo}: {error}");
            return None;
        }
    };
    match &outcome.reason {
        Some(reason) => tonk_common::log!(
            "library/{word}: {} in {repo} failed: {}",
            request.component,
            reason.0
        ),
        None => tonk_common::log!(
            "library/{word}: {} in {repo}: {}",
            request.component,
            outcome.status.0
        ),
    }
    Some(outcome)
}

/// What came of running a request, before its AGENTS.md edit.
struct Settled {
    status: &'static str,
    version: Option<String>,
    /// The catalog's AGENTS.md lines for the component.
    agents: Vec<String>,
    reason: Option<String>,
}

impl Settled {
    fn failed(reason: impl Into<String>) -> Self {
        Self {
            status: "failed",
            version: None,
            agents: Vec::new(),
            reason: Some(reason.into()),
        }
    }
}

async fn run(
    tonk: &crate::worker::TonkState,
    repo: &str,
    branch: &str,
    request: &Request,
) -> Settled {
    let component = request.component.to_string();
    let Some(source) = component_url(&component) else {
        return Settled::failed(format!("'{component}' is not a library component"));
    };
    let session = match open(tonk, repo, branch).await {
        Ok(session) => session,
        Err(error) => return Settled::failed(error),
    };
    let entry = catalog_entry(tonk, &session, &request.component).await;
    let done = match request.action {
        Action::Install => super::repository::install_component(tonk, repo, branch, &source)
            .await
            .map(|installed| if installed { "installed" } else { "current" }),
        Action::Uninstall => super::repository::uninstall_component(tonk, repo, branch, &source)
            .await
            .map(|removed| if removed { "uninstalled" } else { "absent" }),
    };
    let status = match done {
        Ok("absent") => return Settled::failed(format!("{component} is not installed here")),
        Ok(status) => status,
        Err(error) => return Settled::failed(error.to_string()),
    };
    let (version, agents) = match entry {
        Ok(entry) => entry,
        Err(error) => {
            tonk_common::log!("library: read the catalog entry for {component}: {error}");
            (None, None)
        }
    };
    Settled {
        status,
        version,
        agents: agents.as_deref().map(agent_lines).unwrap_or_default(),
        reason: None,
    }
}

/// An open session on `branch` of space `repo`.
async fn open(
    tonk: &crate::worker::TonkState,
    repo: &str,
    branch: &str,
) -> Result<crate::reactor::BranchSession, String> {
    tonk.reactor
        .repository(repo)
        .branch(branch)
        .acquire(&tonk.operator)
        .await
        .map_err(|e| format!("open '{repo}' branch '{branch}': {e}"))
}

/// The catalog's version and AGENTS.md lines for `component`, as this
/// space's core declares them. `None` where its core predates them.
async fn catalog_entry(
    tonk: &crate::worker::TonkState,
    session: &crate::reactor::BranchSession,
    component: &dialog_artifacts::Entity,
) -> Result<(Option<String>, Option<String>), String> {
    use dialog_query::{Output as _, Query, Term};

    let entries: Vec<tonk_schema::LibraryComponent> = session
        .handle()
        .query()
        .select(Query::<tonk_schema::LibraryComponent> {
            this: Term::from(component.clone()),
            name: Term::var("name"),
            label: Term::var("label"),
            version: Term::var("version"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .map_err(|e| format!("{e:?}"))?;
    let agents: Vec<tonk_schema::LibraryComponentAgents> = session
        .handle()
        .query()
        .select(Query::<tonk_schema::LibraryComponentAgents> {
            this: Term::from(component.clone()),
            agents: Term::var("agents"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .map_err(|e| format!("{e:?}"))?;
    Ok((
        entries.into_iter().next().map(|entry| entry.version.0),
        agents.into_iter().next().map(|entry| entry.agents.0),
    ))
}

/// The catalog's AGENTS.md lines for `component`.
#[cfg(test)]
pub(super) async fn catalog_agents(
    tonk: &crate::worker::TonkState,
    session: &crate::reactor::BranchSession,
    component: &dialog_artifacts::Entity,
) -> Option<Vec<String>> {
    let (_, agents) = catalog_entry(tonk, session, component).await.ok()?;
    agents.as_deref().map(agent_lines)
}

/// The space's AGENTS.md (`tonk/agents`), if it has one.
async fn read_agents(
    tonk: &crate::worker::TonkState,
    session: &crate::reactor::BranchSession,
    subject: &dialog_artifacts::Entity,
) -> Result<Option<String>, String> {
    use dialog_query::{Output as _, Query, Term};

    let rows: Vec<tonk_schema::RepositoryAgents> = session
        .handle()
        .query()
        .select(Query::<tonk_schema::RepositoryAgents> {
            this: Term::from(subject.clone()),
            agents: Term::var("agents"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .map_err(|e| format!("{e:?}"))?;
    Ok(rows.into_iter().next().map(|row| row.agents.0))
}

/// The AGENTS.md lines this space's installs of `component` added and no
/// uninstall has removed since, in the order they were added: the recorded
/// outcomes replayed by `time`. A line that was in AGENTS.md before an
/// install was not added by it, so it is not owed.
async fn owed_lines(
    tonk: &crate::worker::TonkState,
    session: &crate::reactor::BranchSession,
    component: &dialog_artifacts::Entity,
) -> Result<Vec<String>, String> {
    use dialog_query::{Output as _, Query, Term};

    let mut outcomes: Vec<tonk_schema::LibraryOutcome> = session
        .handle()
        .query()
        .select(Query::<tonk_schema::LibraryOutcome> {
            this: Term::var("this"),
            component: Term::from(tonk_schema::domain::library::outcome::Component(
                component.clone(),
            )),
            action: Term::var("action"),
            status: Term::var("status"),
            time: Term::var("time"),
            version: Term::var("version"),
            lines: Term::var("lines"),
            reason: Term::var("reason"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .map_err(|e| format!("{e:?}"))?;
    outcomes.sort_by(|a, b| a.time.0.total_cmp(&b.time.0));
    let mut owed: Vec<String> = Vec::new();
    for outcome in outcomes {
        let Some(lines) = outcome.lines else {
            continue;
        };
        let lines = agent_lines(&lines.0);
        if outcome.action.0 == Action::Install.word() {
            for line in lines {
                if !owed.contains(&line) {
                    owed.push(line);
                }
            }
        } else {
            owed.retain(|line| !lines.contains(line));
        }
    }
    Ok(owed)
}

/// Record the request's outcome, with the AGENTS.md edit it makes, in one
/// commit. AGENTS.md is read and edited under the branch's writer lock, and
/// again after a race, so a concurrent install or edit is never overwritten
/// by a stale copy.
async fn record(
    tonk: &crate::worker::TonkState,
    repo: &str,
    branch: &str,
    request: &Request,
    settled: &Settled,
) -> Result<tonk_schema::LibraryOutcome, String> {
    use dialog_artifacts::Statement as _;
    use tonk_schema::domain::library::outcome;

    let subject = super::repository::space_entity(repo).map_err(|e| e.to_string())?;
    let session = open(tonk, repo, branch).await?;
    let _committing = session.transactor().lock().await;
    let mut attempt = 0;
    loop {
        let mut changes = dialog_artifacts::Changes::new();
        let mut reason = settled.reason.clone();
        let mut edited = Vec::new();
        if settled.reason.is_none() {
            let edit = async {
                let before = read_agents(tonk, &session, &subject).await?;
                let current = before.clone().unwrap_or_default();
                let (after, lines) = match request.action {
                    Action::Install => with_lines(&current, &settled.agents),
                    Action::Uninstall => {
                        let owed = owed_lines(tonk, &session, &request.component).await?;
                        without_lines(&current, &owed)
                    }
                };
                Ok::<_, String>((before, after, lines))
            };
            match edit.await {
                Ok((before, after, lines)) if !lines.is_empty() => {
                    // Cardinality one: the new text replaces the old.
                    if !after.trim().is_empty() {
                        tonk_schema::RepositoryAgents {
                            this: subject.clone(),
                            agents: tonk_schema::domain::repo::Agents(after),
                        }
                        .assert(&mut changes);
                    } else if let Some(before) = before {
                        tonk_schema::RepositoryAgents {
                            this: subject.clone(),
                            agents: tonk_schema::domain::repo::Agents(before),
                        }
                        .retract(&mut changes);
                    }
                    edited = lines;
                }
                Ok(_) => {}
                Err(error) => {
                    reason = Some(format!("AGENTS.md unreadable, left as it was: {error}"))
                }
            }
        }
        let lines = (!edited.is_empty()).then(|| {
            let mut text = edited.join("\n");
            text.push('\n');
            outcome::Lines(text)
        });
        let recorded = tonk_schema::LibraryOutcome {
            this: request.this.clone(),
            component: outcome::Component(request.component.clone()),
            action: outcome::Action(request.action.word().to_owned()),
            status: outcome::Status(settled.status.to_owned()),
            time: outcome::Time(request.time),
            version: settled.version.clone().map(outcome::Version),
            lines,
            reason: reason.map(outcome::Reason),
        };
        recorded.clone().assert(&mut changes);
        match super::repository::publish_instructions(tonk, &session, changes.into_instructions())
            .await
        {
            Ok(()) => {
                session.poll(&tonk.operator).await;
                return Ok(recorded);
            }
            Err(error) => {
                super::repository::retry_after_race(tonk, repo, &session, &mut attempt, error)
                    .await
                    .map_err(|e| e.to_string())?;
            }
        }
    }
}

/// The non-blank lines of a component's `agents` text.
fn agent_lines(agents: &str) -> Vec<String> {
    agents
        .lines()
        .map(str::trim_end)
        .filter(|line| !line.trim().is_empty())
        .map(str::to_owned)
        .collect()
}

/// `markdown` with each of `lines` it lacks appended as one paragraph after
/// a blank line, and the lines that were appended.
fn with_lines(markdown: &str, lines: &[String]) -> (String, Vec<String>) {
    let present: std::collections::HashSet<&str> = markdown.lines().map(str::trim_end).collect();
    let missing: Vec<String> = lines
        .iter()
        .filter(|line| !present.contains(line.as_str()))
        .cloned()
        .collect();
    if missing.is_empty() {
        return (markdown.to_owned(), missing);
    }
    let mut out = markdown.trim_end().to_owned();
    if !out.is_empty() {
        out.push_str("\n\n");
    }
    out.push_str(&missing.join("\n"));
    out.push('\n');
    (out, missing)
}

/// `markdown` without `lines`, and the lines that were removed. The
/// paragraph [`with_lines`] appended goes with the blank line before it;
/// otherwise each line goes where it stands, once, and nothing else moves.
fn without_lines(markdown: &str, lines: &[String]) -> (String, Vec<String>) {
    if lines.is_empty() {
        return (markdown.to_owned(), Vec::new());
    }
    let paragraph = format!("{}\n", lines.join("\n"));
    if markdown == paragraph {
        return (String::new(), lines.to_vec());
    }
    let appended = format!("\n\n{paragraph}");
    if let Some(at) = markdown.rfind(&appended) {
        let mut out = markdown[..at].to_owned();
        out.push('\n');
        out.push_str(&markdown[at + appended.len()..]);
        return (out, lines.to_vec());
    }
    let mut kept: Vec<&str> = markdown.lines().collect();
    let mut removed = Vec::new();
    for line in lines {
        if let Some(at) = kept.iter().position(|kept| kept.trim_end() == line) {
            kept.remove(at);
            removed.push(line.clone());
        }
    }
    let mut out = kept.join("\n");
    if markdown.ends_with('\n') && !out.is_empty() {
        out.push('\n');
    }
    (out, removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_service_worker);

    fn library() -> Library {
        Library::new(Url::parse("https://tonk.test/library/").unwrap())
    }

    #[dialog_common::test]
    fn it_admits_files_inside_the_library() {
        let root = library();
        let base = Url::parse("https://tonk.test/library/table.yaml").unwrap();
        assert!(root.contains(&base.join("./table/connected.js").unwrap()));
        assert!(root.contains(&base.join("/library/core.yaml").unwrap()));
    }

    #[dialog_common::test]
    fn it_refuses_anything_outside_the_library() {
        let root = library();
        let base = Url::parse("https://tonk.test/library/table.yaml").unwrap();
        for reference in [
            "../api/repository/x",
            "/api/repository/x",
            "https://elsewhere.test/library/core.yaml",
            "./core.yaml?x=1",
        ] {
            assert!(
                !root.contains(&base.join(reference).unwrap()),
                "{reference} must not load"
            );
        }
    }

    fn lines(text: &[&str]) -> Vec<String> {
        text.iter().map(|line| (*line).to_owned()).collect()
    }

    #[dialog_common::test]
    fn it_appends_a_components_lines_as_a_paragraph() {
        let wanted = lines(&["Use table/edit-cell.", "Store raw input."]);
        let (after, added) = with_lines("# Space\n\nBe kind.\n", &wanted);
        assert_eq!(
            after,
            "# Space\n\nBe kind.\n\nUse table/edit-cell.\nStore raw input.\n"
        );
        assert_eq!(added, wanted);

        let (empty, added) = with_lines("", &wanted);
        assert_eq!(empty, "Use table/edit-cell.\nStore raw input.\n");
        assert_eq!(added, wanted);
    }

    #[dialog_common::test]
    fn it_adds_only_the_lines_agents_md_lacks() {
        let wanted = lines(&["Use table/edit-cell.", "Store raw input."]);
        let (after, added) = with_lines("Store raw input.\n", &wanted);
        assert_eq!(after, "Store raw input.\n\nUse table/edit-cell.\n");
        assert_eq!(added, lines(&["Use table/edit-cell."]));

        let (same, added) = with_lines(&after, &wanted);
        assert_eq!(same, after);
        assert!(added.is_empty());
    }

    #[dialog_common::test]
    fn it_removes_exactly_the_lines_an_install_added() {
        let wanted = lines(&["Use table/edit-cell.", "Store raw input."]);
        for before in ["# Space\n\nBe kind.\n", "", "Be kind."] {
            let (installed, _) = with_lines(before, &wanted);
            let (after, removed) = without_lines(&installed, &wanted);
            assert_eq!(removed, wanted);
            let expected = if before.is_empty() || before.ends_with('\n') {
                before.to_owned()
            } else {
                format!("{before}\n")
            };
            assert_eq!(after, expected, "uninstall restores {before:?}");
        }

        // Written after the install: the paragraph goes, what follows stays.
        let (installed, _) = with_lines("Be kind.\n", &wanted);
        let edited = format!("{installed}\nShip on Fridays.\n");
        let (after, _) = without_lines(&edited, &wanted);
        assert_eq!(after, "Be kind.\n\nShip on Fridays.\n");
    }

    #[dialog_common::test]
    fn it_removes_moved_lines_one_by_one_and_nothing_else() {
        let wanted = lines(&["Use table/edit-cell.", "Store raw input."]);
        let markdown = "Store raw input.\nBe kind.\nUse table/edit-cell.\n";
        let (after, removed) = without_lines(markdown, &wanted);
        assert_eq!(after, "Be kind.\n");
        assert_eq!(
            removed,
            lines(&["Use table/edit-cell.", "Store raw input."])
        );

        let (untouched, removed) = without_lines("Be kind.\n", &wanted);
        assert_eq!(untouched, "Be kind.\n");
        assert!(removed.is_empty());
    }

    /// Natively the library is the bundled copy: a library body parses
    /// at the bundled root, and an include it has no business making is
    /// refused rather than read from wherever the process happens to be.
    #[cfg(not(target_arch = "wasm32"))]
    #[dialog_common::test]
    async fn it_parses_a_library_body_at_the_bundled_root() {
        let syntax = parse("xyz.test!:\n  this: id:x\n  body: \"text\"\n")
            .await
            .unwrap();
        assert_eq!(syntax.base.as_str(), tonk_library::ROOT);

        let error = parse("xyz.test!:\n  this: id:x\n  body: !include ../../etc/passwd\n")
            .await
            .unwrap_err();
        assert!(error.to_string().contains("include"), "{error}");
    }
}
