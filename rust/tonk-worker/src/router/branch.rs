//! Making a branch a space does not have yet (`space/create-branch`).
//!
//! A page at `/space/{branch}@{did}` names a branch as well as a space.
//! Opening one the space lacks makes nothing: the stamp reports it
//! ([`super::session::branch_missing`]) and the chrome offers to make it,
//! which is this command.
//!
//! Where the branch starts from:
//!
//! 1. The branch of the same name on the space's remote, when the remote
//!    has one. Another device may already have made `test`; starting a
//!    second line beside it would fork what both mean by `test`.
//! 2. Otherwise `main` as it is now (`base: main`), or the standard library
//!    alone (`base: empty`).
//!
//! With a remote, the branch then tracks the remote branch of its name in
//! both directions, which is all the sync sweep needs to push and pull it
//! like `main`. It does not track local `main`: taking `main`'s later work,
//! or bringing the branch's back, is a merge, a separate act.

use dialog_repository::{RepositoryExt as _, Revision};
use tonk_common::log;

use super::AppState;
use super::remotes::RecordedRemote;
use super::repository::{BranchConfiguration, RemoteConfiguration, RepositoryConfiguration};

/// What `base` names when the branch starts from the space's default branch.
const BASE_MAIN: &str = "main";

/// What `base` names when the branch starts with the standard library alone.
const BASE_EMPTY: &str = "empty";

/// What [`create_branch`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Created {
    /// The branch already had a head; nothing was written.
    Existing,
    /// Started from the remote branch of the same name.
    FromRemote,
    /// Started from `main` as it is now.
    FromMain,
    /// Started with the standard library alone.
    Empty,
}

/// Why a branch was not made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// The name cannot be a branch a URL addresses.
    InvalidName(String),
    /// `base` is neither `main` nor `empty`.
    InvalidBase(String),
    /// The space is not on this device, or has no head on `main`.
    NoSpace(String),
    /// The space has a remote, and whether it holds the branch could not be
    /// asked. Making the branch anyway could fork one the remote has.
    RemoteUnknown(String),
    /// Storage or the remote failed partway.
    Failed(String),
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refusal::InvalidName(name) => write!(f, "'{name}' cannot name a branch"),
            Refusal::InvalidBase(base) => {
                write!(
                    f,
                    "base '{base}' is neither '{BASE_MAIN}' nor '{BASE_EMPTY}'"
                )
            }
            Refusal::NoSpace(reason) => write!(f, "the space is not here: {reason}"),
            Refusal::RemoteUnknown(reason) => {
                write!(f, "could not ask the remote for the branch: {reason}")
            }
            Refusal::Failed(reason) => write!(f, "{reason}"),
        }
    }
}

/// Whether `name` can be a branch a `/space/{branch}@{did}` URL addresses:
/// not empty, no `@` (which ends the branch in the segment), no `/` (which
/// ends the segment), and nothing a URL would have to escape around.
fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':'))
}

/// Make branch `name` of `repo`, starting it as the module docs say.
///
/// No state guard is held across the remote: asking it is a round trip, and
/// every other request would wait on it.
pub(crate) async fn create_branch(
    state: &AppState,
    repo: &str,
    name: &str,
    base: &str,
) -> Result<Created, Refusal> {
    if !valid_name(name) {
        return Err(Refusal::InvalidName(name.to_owned()));
    }
    if base != BASE_MAIN && base != BASE_EMPTY {
        return Err(Refusal::InvalidBase(base.to_owned()));
    }

    let (operator, session, main_head, remote) = {
        let tonk = state.read().await;
        let repository = tonk
            .profile
            .space(repo)
            .load()
            .perform(&tonk.operator)
            .await
            .map_err(|e| Refusal::NoSpace(e.to_string()))?;
        let session = tonk
            .reactor
            .repository(repo)
            .branch(name)
            .acquire(&tonk.operator)
            .await
            .map_err(|e| Refusal::Failed(format!("open branch '{name}': {e}")))?;
        session
            .handle()
            .refresh(&tonk.operator)
            .await
            .map_err(|e| Refusal::Failed(format!("refresh branch '{name}': {e}")))?;
        if session.handle().revision().is_some() {
            return Ok(Created::Existing);
        }
        let main_head = tonk
            .reactor
            .repository(repo)
            .branch(super::DEFAULT_BRANCH)
            .acquire(&tonk.operator)
            .await
            .ok()
            .and_then(|main| main.handle().revision())
            .ok_or_else(|| Refusal::NoSpace(format!("'{repo}' has no head on main")))?;
        let remote = default_remote(&tonk, repo, &repository).await?;
        (tonk.operator.clone(), session, main_head, remote)
    };

    // Ask the remote, with no guard held, whether it has the branch.
    let mut created = None;
    if let Some((_, recorded)) = &remote {
        let connected = recorded
            .connect(&operator)
            .await
            .map_err(|e| Refusal::RemoteUnknown(e.to_string()))?;
        let upstream = connected
            .branch(name)
            .open()
            .perform(&operator)
            .await
            .map_err(|e| Refusal::RemoteUnknown(e.to_string()))?;
        let head = upstream
            .fetch()
            .perform(&operator)
            .await
            .map_err(|e| Refusal::RemoteUnknown(e.to_string()))?;
        if head.is_some() {
            session
                .handle()
                .pull(dialog_repository::Download::MinimalRanges)
                .from(&upstream)
                .perform(&operator)
                .await
                .map_err(|e| Refusal::Failed(format!("pull remote '{name}': {e}")))?;
            created = Some(Created::FromRemote);
        }
    }

    let tonk = state.read().await;
    let created = match created {
        Some(created) => created,
        None if base == BASE_MAIN => {
            start_from(&session, main_head, &tonk.operator).await?;
            Created::FromMain
        }
        None => {
            super::repository::seed_empty_branch(&tonk, repo, name)
                .await
                .map_err(|e| Refusal::Failed(e.to_string()))?;
            Created::Empty
        }
    };

    if let Some((remote_name, recorded)) = remote {
        track_remote(&tonk, repo, name, &remote_name, recorded).await?;
    }
    session.poll(&tonk.operator).await;
    Ok(created)
}

/// Move the headless `session` onto `revision`. A branch with no head may
/// move to any revision ([`dialog_repository::Branch::reset`]).
async fn start_from(
    session: &dialog_reactor::BranchSession,
    revision: Revision,
    operator: &crate::worker::DefaultOperator,
) -> Result<(), Refusal> {
    session
        .handle()
        .reset(revision)
        .perform(operator)
        .await
        .map_err(|e| Refusal::Failed(format!("start from main: {e}")))
}

/// The remote `main` syncs with, by its name on `meta`: where the space's
/// other devices meet, so where a branch of the same name would already be.
async fn default_remote<C>(
    tonk: &crate::worker::TonkState,
    repo: &str,
    repository: &dialog_repository::Repository<C>,
) -> Result<Option<(String, RecordedRemote)>, Refusal>
where
    C: dialog_capability::Principal + Clone,
{
    let info = super::repository::build_repository_info(tonk, repo, repository).await;
    let Some(upstream) = info
        .branch
        .get(super::DEFAULT_BRANCH)
        .and_then(|main| main.upstream.clone())
    else {
        return Ok(None);
    };
    let recorded = super::remotes::find(repository, &upstream.remote, &tonk.operator)
        .await
        .map_err(|e| Refusal::RemoteUnknown(e.to_string()))?
        .ok_or_else(|| {
            Refusal::RemoteUnknown(format!("main tracks unknown remote '{}'", upstream.remote))
        })?;
    Ok(Some((upstream.remote, recorded)))
}

/// Have branch `name` track the branch of its name on `remote`, recorded on
/// `meta` as `main`'s upstream is, so the sync sweep pushes and pulls it.
async fn track_remote(
    tonk: &crate::worker::TonkState,
    repo: &str,
    name: &str,
    remote_name: &str,
    recorded: RecordedRemote,
) -> Result<(), Refusal> {
    let repository = tonk
        .profile
        .space(repo)
        .load()
        .perform(&tonk.operator)
        .await
        .map_err(|e| Refusal::NoSpace(e.to_string()))?;
    let configuration = RepositoryConfiguration::default()
        .remote(
            remote_name,
            RemoteConfiguration::new(recorded.address).subject(recorded.subject),
        )
        .branch(
            name,
            BranchConfiguration::default().upstream(remote_name, name),
        );
    super::repository::ensure_remote_config(tonk, &repository, repo, &configuration)
        .await
        .map(|_| ())
        .map_err(|e| Refusal::Failed(format!("track '{remote_name}/{name}': {e}")))
}

/// Run [`CreateBranch`](tonk_schema::command::CreateBranch).
///
/// Only the profile branch may fire it: the panel that asks lives in the
/// profile's space chrome, and a content branch matched by shape must not
/// make branches in another space.
#[dialog_capability::provider]
impl dialog_capability::Provider<tonk_schema::command::CreateBranch> for crate::router::CommandEnv {
    async fn execute(&self, command: tonk_schema::command::CreateBranch) {
        if !self.origin().repo.is_empty() {
            log!(
                "CreateBranch ignored: origin '{}' is not the profile branch",
                self.origin().repo
            );
            return;
        }
        let repo = command.space.0.to_string();
        let name = command.name.0.trim().to_owned();
        let base = command.base.0.trim().to_owned();
        let site = command.site.0;
        log!("command CreateBranch space={repo} name={name} base={base}");

        match create_branch(self.state(), &repo, &name, &base).await {
            Ok(created) => {
                log!("CreateBranch '{repo}' '{name}': {created:?}");
                let tonk = self.state().read().await;
                super::session::clear_missing_branch(&tonk, &self.origin().branch, &site).await;
                super::session::stamp_waiting_sites(&tonk, &repo).await;
            }
            Err(refusal) => log!("CreateBranch '{repo}' '{name}' refused: {refusal}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use ::axum::Router;
    use ::axum::body::Body;
    use ::axum::http::{Request, StatusCode};
    use tower::ServiceExt as _;

    use super::{Created, Refusal, create_branch};
    use crate::router::{AppState, CommandEnv, CommandOrigin, api_router_with_state};

    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    wasm_bindgen_test_configure!(run_in_service_worker);

    const CORE_LIBRARY: &str = include_str!("../../../tonk-core/assets/library/core.yaml");

    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    use crate::router::repository::profile_library_tests::test_state;
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    use crate::router::tests::test_state;

    /// A space with the standard library on `main`, as creation leaves one.
    async fn space(label: &str) -> (Router, AppState, String) {
        let (app, state, _lsp) = api_router_with_state(test_state().await);
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repository/{label}"))
                    .method("PUT")
                    .header("content-type", "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
        let body = ::axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let info: crate::router::RepositoryInfo = serde_json::from_slice(&body).unwrap();
        {
            let tonk = state.read().await;
            crate::router::repository::install_fresh_seed(
                &tonk,
                &info.name,
                "main",
                CORE_LIBRARY,
                &[],
            )
            .await
            .expect("main seeds");
        }
        (app, state, info.name)
    }

    async fn head(
        state: &AppState,
        repo: &str,
        branch: &str,
    ) -> Option<dialog_repository::Revision> {
        let tonk = state.read().await;
        let session = tonk
            .reactor
            .repository(repo)
            .branch(branch)
            .acquire(&tonk.operator)
            .await
            .expect("the branch opens");
        session
            .handle()
            .refresh(&tonk.operator)
            .await
            .expect("refresh");
        session.handle().revision()
    }

    /// Whether the branch renders: it has the replica the stamp reads and a
    /// route for the space root.
    async fn renders(state: &AppState, repo: &str, branch: &str) -> bool {
        let tonk = state.read().await;
        let session = tonk
            .reactor
            .repository(repo)
            .branch(branch)
            .acquire(&tonk.operator)
            .await
            .expect("the branch opens");
        crate::router::session::origin_entity(&tonk, &session)
            .await
            .is_some()
            && crate::router::session::matched_route(&tonk, &session, "/")
                .await
                .is_some()
    }

    async fn missing(state: &AppState, repo: &str, branch: &str) -> bool {
        let tonk = state.read().await;
        crate::router::session::branch_missing(&tonk, repo, branch).await
    }

    #[dialog_common::test]
    async fn it_reports_only_a_branch_the_space_lacks() {
        let (_app, state, repo) = space("missing-branch").await;

        assert!(missing(&state, &repo, "test").await);
        assert!(
            !missing(&state, &repo, "main").await,
            "main is never missing"
        );
        assert!(
            !missing(
                &state,
                "did:key:z6MkNotHereNotHereNotHereNotHereNotHereNotHe",
                "test"
            )
            .await,
            "a space not on this device is not missing a branch, it is missing"
        );
    }

    /// Opening a branch makes nothing: the tab's `<tonk-site>` asserts its
    /// `tonk:load` through the branch's `/transact`, exactly as here, and
    /// the branch stays missing until someone asks for it to be made. The
    /// load still reaches its handler: the site waits for the branch.
    #[dialog_common::test]
    async fn it_makes_no_branch_when_a_tab_opens_one() {
        let (app, state, repo) = space("opened-branch").await;
        let load = r#"{"claims":[{"op":"assert","application":{"predicate":{"kind":"transient","concept":{"with":{"path":{"the":"xyz.tonk.site/path","as":"Text","cardinality":"one"}}}},"parameters":{"this":"site:opened-branch","path":"/"}}}]}"#;

        for _ in 0..2 {
            let mut request = Request::builder()
                .method("POST")
                .uri(format!("/api/repository/{repo}/branch/test/transact"))
                .header("content-type", "application/json")
                .body(Body::from(load))
                .unwrap();
            request
                .extensions_mut()
                .insert(crate::router::ClientId("opened-branch".to_owned()));
            let response = app.clone().oneshot(request).await.unwrap();
            assert_eq!(response.status(), StatusCode::OK, "the load is accepted");
        }

        assert!(missing(&state, &repo, "test").await);
        assert_eq!(head(&state, &repo, "test").await, None);
        let waiting = state
            .read()
            .await
            .clients
            .read()
            .await
            .get(&crate::router::ClientId("opened-branch".to_owned()))
            .is_some_and(|client| {
                client
                    .waiting
                    .iter()
                    .any(|site| site.waits_on(&repo, "test"))
            });
        assert!(
            waiting,
            "the load reached its handler, and the site waits for the branch"
        );
    }

    #[dialog_common::test]
    async fn it_starts_a_branch_from_main() {
        let (_app, state, repo) = space("branch-from-main").await;

        let created = create_branch(&state, &repo, "test", "main").await;

        assert_eq!(created, Ok(Created::FromMain));
        assert_eq!(
            head(&state, &repo, "test").await,
            head(&state, &repo, "main").await
        );
        assert!(!missing(&state, &repo, "test").await);
        assert!(
            renders(&state, &repo, "test").await,
            "the branch renders like main"
        );
    }

    #[dialog_common::test]
    async fn it_starts_an_empty_branch_with_the_standard_library() {
        let (_app, state, repo) = space("empty-branch").await;

        let created = create_branch(&state, &repo, "fresh", "empty").await;

        assert_eq!(created, Ok(Created::Empty));
        assert_ne!(
            head(&state, &repo, "fresh").await,
            head(&state, &repo, "main").await
        );
        assert!(
            renders(&state, &repo, "fresh").await,
            "the library routes the space root"
        );
    }

    /// Asking again for a branch that exists writes nothing.
    #[dialog_common::test]
    async fn it_leaves_an_existing_branch_alone() {
        let (_app, state, repo) = space("existing-branch").await;
        create_branch(&state, &repo, "test", "empty").await.unwrap();
        let before = head(&state, &repo, "test").await;

        let created = create_branch(&state, &repo, "test", "main").await;

        assert_eq!(created, Ok(Created::Existing));
        assert_eq!(head(&state, &repo, "test").await, before);
    }

    #[dialog_common::test]
    async fn it_refuses_a_name_a_url_cannot_carry() {
        let (_app, state, repo) = space("bad-branch-name").await;

        for name in ["", "a@b", "a/b", "a b"] {
            let created = create_branch(&state, &repo, name, "main").await;
            assert!(
                matches!(created, Err(Refusal::InvalidName(_))),
                "{name:?}: {created:?}"
            );
        }
        assert!(matches!(
            create_branch(&state, &repo, "test", "elsewhere").await,
            Err(Refusal::InvalidBase(_))
        ));
        assert!(
            missing(&state, &repo, "test").await,
            "a refusal makes nothing"
        );
    }

    /// Fire `space/create-branch` for branch `test` of `repo`, as if it were
    /// committed on `origin_repo` (empty for the profile branch).
    async fn fire_create_branch(state: &AppState, repo: &str, origin_repo: &str) {
        use tonk_schema::domain::command::current::create_branch as field;

        <CommandEnv as dialog_capability::Provider<tonk_schema::command::CreateBranch>>::execute(
            &CommandEnv::new(
                state.clone(),
                CommandOrigin {
                    repo: origin_repo.to_owned(),
                    branch: "main".to_owned(),
                    client: None,
                },
            ),
            tonk_schema::command::CreateBranch {
                this: "id:create-branch".parse().unwrap(),
                space: field::Space(repo.parse().unwrap()),
                name: field::Name("test".to_owned()),
                base: field::Base("main".to_owned()),
                site: field::Site("site:create-branch".parse().unwrap()),
            },
        )
        .await;
    }

    /// The command is the profile's: fired from a content branch, which
    /// matches commands by shape, it makes no branch.
    #[dialog_common::test]
    async fn it_ignores_a_create_branch_fired_from_a_content_branch() {
        let (_app, state, repo) = space("content-fired-branch").await;

        fire_create_branch(&state, &repo, &repo).await;
        assert!(
            missing(&state, &repo, "test").await,
            "a content branch makes nothing"
        );
        assert_eq!(head(&state, &repo, "test").await, None);

        fire_create_branch(&state, &repo, "").await;
        assert!(
            !missing(&state, &repo, "test").await,
            "the profile branch makes it"
        );
    }

    const PROFILE_LIBRARY: &str = include_str!("../../../tonk-core/assets/library/profile.yaml");

    /// The `xyz.tonk.site/{field}` text the profile branch holds for `site`.
    async fn profile_site_field(
        app: &Router,
        branch: &str,
        site: &str,
        field: &str,
    ) -> Option<String> {
        let query = format!(
            r#"{{"predicate":{{"with":{{"{field}":{{"the":"xyz.tonk.site/{field}","as":"Text","cardinality":"one"}}}}}},"terms":{{"this":"{site}","{field}":{{"?":{{"name":"{field}"}}}}}}}}"#
        );
        let mut request = Request::builder()
            .method("POST")
            .uri(format!(
                "/api/repository/profile:tonk/branch/{branch}/query"
            ))
            .header("content-type", "application/json")
            .body(Body::from(query))
            .unwrap();
        request
            .extensions_mut()
            .insert(crate::router::ClientId("profile-tab".to_owned()));
        let response = app.clone().oneshot(request).await.unwrap();
        let body = ::axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let rows: serde_json::Value = serde_json::from_slice(&body).ok()?;
        rows.as_array()?
            .first()?
            .get("fields")?
            .get(field)?
            .as_str()
            .map(str::to_owned)
    }

    /// A profile tab at `/space/test@{did}` is told which space and branch
    /// the segment names, and that the branch is missing; making it clears
    /// that.
    #[dialog_common::test]
    async fn it_tells_a_profile_tab_its_branch_is_missing_until_it_is_made() {
        use tonk_schema::domain::command::current::create_branch as field;

        let (app, state, repo) = space("profile-tab-branch").await;
        let profile_branch = {
            let tonk = state.read().await;
            crate::router::evaluate::evaluate_profile_body_recording(
                &tonk,
                &tonk.active_branch,
                PROFILE_LIBRARY.to_owned(),
                &|_| Vec::new(),
            )
            .await
            .expect("the profile library seeds");
            tonk.active_branch.clone()
        };
        let site = "site:profile-tab";
        let profile = CommandEnv::new(
            state.clone(),
            CommandOrigin {
                repo: String::new(),
                branch: profile_branch.clone(),
                client: Some(crate::router::ClientId("profile-tab".to_owned())),
            },
        );
        <CommandEnv as dialog_capability::Provider<tonk_schema::command::Load>>::execute(
            &profile,
            tonk_schema::command::Load {
                this: site.parse().unwrap(),
                path: tonk_schema::domain::command::load::Path(format!("/space/test@{repo}")),
            },
        )
        .await;

        let read = |name: &'static str| profile_site_field(&app, &profile_branch, site, name);
        assert_eq!(read("space-repo").await.as_deref(), Some(repo.as_str()));
        assert_eq!(read("space-branch").await.as_deref(), Some("test"));
        assert_eq!(read("space-branch-state").await.as_deref(), Some("missing"));

        <CommandEnv as dialog_capability::Provider<tonk_schema::command::CreateBranch>>::execute(
            &profile,
            tonk_schema::command::CreateBranch {
                this: "id:profile-tab-create".parse().unwrap(),
                space: field::Space(repo.parse().unwrap()),
                name: field::Name("test".to_owned()),
                base: field::Base("main".to_owned()),
                site: field::Site(site.parse().unwrap()),
            },
        )
        .await;

        assert_eq!(
            read("space-branch-state").await,
            None,
            "made, so no longer missing"
        );
        assert_eq!(read("space-branch").await.as_deref(), Some("test"));
    }

    /// The bare space segment names `main`, which is never missing.
    #[dialog_common::test]
    async fn it_reads_a_bare_space_segment_as_main() {
        assert_eq!(
            super::super::session::space_target("/space/did:key:z6MkA/inspector"),
            Some(("did:key:z6MkA".to_owned(), "main".to_owned()))
        );
        assert_eq!(
            super::super::session::space_target("/space/test@home:z6MkA"),
            Some(("did:key:z6MkA".to_owned(), "test".to_owned()))
        );
        assert_eq!(super::super::session::space_target("/join"), None);
    }

    /// A remote holding the branch wins over `base`: the branch starts from
    /// the remote's head, as it would on a second device that never made it,
    /// and tracks the remote branch so the sweep keeps both in step. A
    /// branch the remote lacks starts from `base` and tracks the remote
    /// branch of its name all the same.
    ///
    /// Native only: the remote is the native access service, which the wasm
    /// runner cannot start. Without a remote, both targets run the `base`
    /// paths above (`it_starts_a_branch_from_main`,
    /// `it_starts_an_empty_branch_with_the_standard_library`).
    #[cfg(not(target_arch = "wasm32"))]
    #[dialog_common::test]
    async fn it_starts_from_the_remote_branch_of_the_same_name() {
        use dialog_repository::RepositoryExt as _;

        let (tonk, _service, root, remote) =
            crate::router::account_state::tests::ready_account_state(None).await;
        let grant =
            tonk_identity::delegation::mint_device_delegation(root.clone(), &tonk.profile.did())
                .await
                .expect("the device grant mints");
        crate::router::identity::persist_root(
            &tonk,
            tonk_worker_api::SaveRootRequest {
                credential_id: "create-branch-remote".to_string(),
                delegation_hex: hex::encode(grant.to_bytes().expect("the grant serializes")),
                passkey: None,
                encryption_key: None,
            },
        )
        .await
        .expect("the root persists");
        let state: AppState = std::sync::Arc::new(crate::StateLock::new(tonk));
        let key = crate::router::repository::create_space_inner(&state, "Branched", None)
            .await
            .expect("the space creates");
        crate::router::repository::enable_sync_inner(&state, &key, &remote)
            .await
            .expect("the remote attaches");

        // Another device made `shared` and pushed it: here, `main`'s head
        // published to the remote as `shared`, which this device never had.
        let (published, remote_name) = {
            let tonk = state.read().await;
            let main = tonk
                .reactor
                .repository(&key)
                .branch("main")
                .acquire(&tonk.operator)
                .await
                .expect("main opens");
            main.handle()
                .push()
                .perform(&tonk.operator)
                .await
                .expect("main reaches the remote");
            let published = main.handle().revision().expect("main has a head");
            let repository = tonk
                .profile
                .space(&key)
                .load()
                .perform(&tonk.operator)
                .await
                .expect("the space loads");
            let info =
                crate::router::repository::build_repository_info(&tonk, &key, &repository).await;
            let remote_name = info.branch["main"]
                .upstream
                .clone()
                .expect("main tracks the remote")
                .remote;
            let connected = crate::router::remotes::find(&repository, &remote_name, &tonk.operator)
                .await
                .expect("the remotes read")
                .expect("main's remote is recorded")
                .connect(&tonk.operator)
                .await
                .expect("the remote connects");
            connected
                .branch("shared")
                .open()
                .perform(&tonk.operator)
                .await
                .expect("the remote branch opens")
                .publish(published.clone())
                .perform(&tonk.operator)
                .await
                .expect("the remote branch takes main's head");
            (published, remote_name)
        };
        assert!(
            missing(&state, &key, "shared").await,
            "not on this device yet"
        );

        let created = create_branch(&state, &key, "shared", "empty").await;
        assert_eq!(
            created,
            Ok(Created::FromRemote),
            "the remote's branch, not an empty one"
        );
        assert_eq!(head(&state, &key, "shared").await, Some(published));

        let created = create_branch(&state, &key, "solo", "main").await;
        assert_eq!(created, Ok(Created::FromMain), "the remote has no `solo`");

        let tonk = state.read().await;
        let repository = tonk
            .profile
            .space(&key)
            .load()
            .perform(&tonk.operator)
            .await
            .expect("the space loads");
        let info = crate::router::repository::build_repository_info(&tonk, &key, &repository).await;
        for name in ["shared", "solo"] {
            let upstream = info.branch[name]
                .upstream
                .clone()
                .unwrap_or_else(|| panic!("{name} tracks the remote"));
            assert_eq!(
                (upstream.remote.as_str(), upstream.branch.as_str()),
                (remote_name.as_str(), name)
            );
        }
    }
}
