# Share enable-sync prompt Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A share click on a spot with no sync remote refuses to mint a dead invite, and instead offers to turn sync on and complete the share in one click.

**Architecture:** The worker resolves the remote *before* minting; an unresolvable remote asserts an overlay-only `ShareBlocked` fact keyed on the spot's subject and echoing the triggering command's timestamp. `<tonk-share>` picks that up on a second tagged subscription, abandons its pending clipboard write, and opens a dialog. Confirming dispatches a new `tonk:enable-sync` command whose handler attaches the remote and then mints — so success arrives back on the link subscription the element already holds.

**Tech Stack:** Rust → wasm32-unknown-unknown, `custom-elements`, `dialog-reactor` command handlers, `tonk-schema` `Concept`/`Attribute` derives, Web Awesome (`<wa-dialog>`).

## Global Constraints

- Spec: `docs/superpowers/specs/2026-07-22-share-enable-sync-prompt-design.md`
- Base branch: `origin/staging`. Work branch: `feat/share-enable-sync-prompt` (already created, spec already committed).
- Tests always use `#[dialog_common::test]`, never `#[test]`/`#[tokio::test]`. Name tests `it_does_x`.
- Every wasm-runnable test mod needs `wasm_bindgen_test_configure!` — `run_in_service_worker` for `tonk-worker`, `run_in_browser` for `tonk-fab`/`tonk-schema`.
- No `mod.rs`. Use `foo.rs` + `foo/`.
- No emojis in code, comments, or commit messages.
- Commit messages: Conventional Commits, `type(scope): subject`, imperative, lowercase, no trailing period.
- Never reference "Phase N" or "per the spec" in code or comments. Code stands on its own.
- The lint gate is `nix develop -c cargo clippy --workspace --all-targets --all-features` plus `cargo fmt --check`. `--all-features` compiles integration tests, so a per-crate clippy can pass while the gate fails.
- User-facing copy, exact:
  - dialog title: `Turn on sync?`
  - dialog body: `This spot only exists on this device. Turn on sync so the people you share with can open it.`
  - primary button: `Turn on sync & copy link`
  - secondary button: `Not now`
  - `not-synced` detail: `This spot only exists on this device.`
  - `unshareable-remote` detail: `This spot's sync server can't be shared.`
  - `attach-failed` detail prefix: `Could not turn on sync: `

## File Structure

| File | Responsibility |
|---|---|
| `rust/tonk-schema/src/domain.rs` | new `share` and `command::enable_sync` attribute modules |
| `rust/tonk-schema/src/command.rs` | new `ShareBlocked` concept and `EnableSync` command |
| `rust/tonk-worker/src/router/create_invite.rs` | `RemoteRequirement`; `resolve_remote_url` returns it; HTTP route refuses |
| `rust/tonk-worker/src/router/repository.rs` | `run_invite` refuses + signals; `EnableSyncHandler` |
| `rust/tonk-worker/src/router/command.rs` | register `EnableSyncHandler` |
| `rust/tonk-fab/src/subscribing.rs` | multiplex several tagged subscriptions on one element |
| `rust/tonk-fab/src/logic.rs` | `ShareState::Blocked`, claim JSON, query body, default-remote URL (all pure) |
| `rust/tonk-fab/src/share.rs` | blocked subscription, timeout, dialog open, confirm handler |
| `rust/tonk-fab/src/markup.rs` | the `<wa-dialog>` |
| `rust/tonk-fab/src/fab.css` | dialog styling |

---

### Task 1: `ShareBlocked` concept and its attributes

**Files:**
- Modify: `rust/tonk-schema/src/domain.rs` (add a `share` module after the `credential` module, which ends at line 532)
- Modify: `rust/tonk-schema/src/command.rs` (add `ShareBlocked` after `Credential`, which ends at line 315)

**Interfaces:**
- Produces: `tonk_schema::domain::share::{Blocked, Detail, Time}` and `tonk_schema::command::ShareBlocked { this: Entity, blocked: Blocked, detail: Detail, time: Time }`. Attribute names `xyz.tonk.share/blocked`, `xyz.tonk.share/detail`, `xyz.tonk.share/time`.

- [ ] **Step 1: Write the failing test**

Add to the bottom of `rust/tonk-schema/src/command.rs`, or extend the existing `mod tests` there if one exists:

```rust
#[cfg(test)]
mod share_blocked {
    use super::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_browser);

    #[dialog_common::test]
    fn it_derives_the_share_attribute_names() {
        use dialog_query::Attribute as _;

        assert_eq!(
            crate::domain::share::Blocked::attribute().to_string(),
            "xyz.tonk.share/blocked"
        );
        assert_eq!(
            crate::domain::share::Detail::attribute().to_string(),
            "xyz.tonk.share/detail"
        );
        assert_eq!(
            crate::domain::share::Time::attribute().to_string(),
            "xyz.tonk.share/time"
        );
    }
}
```

If `dialog_query::Attribute`'s accessor is spelled differently in this workspace, copy the spelling from an existing attribute-name assertion — grep for `::attribute()` under `rust/tonk-schema/src` and match it.

- [ ] **Step 2: Run test to verify it fails**

Run: `nix develop -c cargo test -p tonk-schema share_blocked`
Expected: FAIL — `could not find 'share' in 'domain'`

- [ ] **Step 3: Add the attributes**

In `rust/tonk-schema/src/domain.rs`, immediately after the closing brace of `pub mod credential` (line 532):

```rust
/// Attributes on the overlay-only `tonk:share/blocked` fact — why a share
/// click could not mint an invite. Keyed on the spot's subject entity, the
/// same entity [`crate::command::Credential`] is keyed by, so the share
/// control reads both off one subject.
///
/// Overlay-only, so it is session-scoped and never replicated: a refusal is
/// this device's answer to this click, not a property of the spot.
pub mod share {
    use super::Attribute;

    /// The refusal class: `not-synced` | `unshareable-remote` |
    /// `attach-failed`. Only `not-synced` is repairable by attaching a
    /// remote, so it is the only one that offers the prompt.
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.share")]
    #[cardinality(one)]
    pub struct Blocked(pub String);

    /// The sentence shown to the user.
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.share")]
    #[cardinality(one)]
    pub struct Detail(pub String);

    /// The timestamp of the command this refusal answers, echoed back from
    /// the transient that triggered it.
    ///
    /// Load-bearing. The fact is cardinality-one on the subject, so it
    /// lingers in the overlay and replays on every resubscribe; the share
    /// control acts only on a refusal whose timestamp matches the click it
    /// is currently holding a clipboard write for, and ignores every other
    /// frame. That is what makes the fact safe to leave in place instead of
    /// retracting it on the next success.
    #[derive(Attribute, Clone, PartialEq, PartialOrd)]
    #[domain("xyz.tonk.share")]
    #[cardinality(one)]
    pub struct Time(pub f64);
}
```

`Time` derives only `Clone, PartialEq, PartialOrd` — `f64` is not `Eq`/`Ord`. This mirrors `command::invite::TimeStamp` at `domain.rs:252`.

- [ ] **Step 4: Add the concept**

In `rust/tonk-schema/src/command.rs`, after `Credential` (line 315):

```rust
/// The overlay-only fact a refused `tonk:invite` asserts: why the mint did
/// not happen, keyed by the spot's **subject** DID (`this`) — the same
/// entity [`Credential`] is keyed by, so one subject carries both the
/// success and the refusal.
///
/// All three fields are asserted together. A concept resolves only when
/// every declared field is present, so a partial assert would never resolve
/// (the same all-fields-required gotcha `JoinStatus`/`JoinFailure` are split
/// to avoid).
#[derive(Concept, Debug, Clone, PartialEq, PartialOrd)]
pub struct ShareBlocked {
    /// The spot's subject DID.
    pub this: Entity,
    /// Refusal class: `not-synced` | `unshareable-remote` | `attach-failed`.
    pub blocked: crate::domain::share::Blocked,
    /// The sentence shown to the user.
    pub detail: crate::domain::share::Detail,
    /// The refused command's timestamp, echoed so the share control can tell
    /// this refusal from a replay of an older one.
    pub time: crate::domain::share::Time,
}
```

No `impl Command` — this is a fact, not a command.

- [ ] **Step 5: Run test to verify it passes**

Run: `nix develop -c cargo test -p tonk-schema share_blocked`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add rust/tonk-schema/src/domain.rs rust/tonk-schema/src/command.rs
git commit -m "feat(tonk-schema): add the overlay-only ShareBlocked fact"
```

---

### Task 2: `resolve_remote_url` reports which case it took

**Files:**
- Modify: `rust/tonk-worker/src/router/create_invite.rs:344-399` (both `resolve_remote_url` and `resolve_remote_url_with`)
- Modify: `rust/tonk-worker/src/router/create_invite.rs:181` (the HTTP route's call site — compile fix only, behaviour changes in Task 4)
- Modify: `rust/tonk-worker/src/router/repository.rs:778` (the mint call site — compile fix only, behaviour changes in Task 3)

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces: `pub(crate) enum RemoteRequirement { Ready(Url), Refused(RemoteRefusal) }` and `pub(crate) enum RemoteRefusal { NotSynced, UnshareableRemote }` with `RemoteRefusal::code(&self) -> &'static str` and `RemoteRefusal::detail(&self) -> &'static str`. `resolve_remote_url` and `resolve_remote_url_with` both now return `Result<RemoteRequirement, TonkWorkerError>`.

- [ ] **Step 1: Write the failing test**

Add to the existing `mod tests` at the bottom of `rust/tonk-worker/src/router/create_invite.rs` (it already has `wasm_bindgen_test_configure!(run_in_service_worker)` at its top):

```rust
/// A spot created without a remote refuses, and says which case it was.
#[dialog_common::test]
async fn it_refuses_a_repository_with_no_upstream() {
    use crate::router::create_invite::{RemoteRefusal, RemoteRequirement, resolve_remote_url};

    let state = test_state().await;
    let (app, _state, _lsp) = api_router_with_state(state.clone());
    let key = put_repo(&app, "test-no-upstream").await;

    let tonk = state.read().await;
    let repository = tonk
        .profile
        .repository(&key)
        .load()
        .perform(&tonk.operator)
        .await
        .expect("repository loads");

    let requirement = resolve_remote_url(&tonk, &repository)
        .await
        .expect("probe succeeds");

    assert!(matches!(
        requirement,
        RemoteRequirement::Refused(RemoteRefusal::NotSynced)
    ));
}

#[dialog_common::test]
async fn it_names_the_refusal_classes() {
    use crate::router::create_invite::RemoteRefusal;

    assert_eq!(RemoteRefusal::NotSynced.code(), "not-synced");
    assert_eq!(
        RemoteRefusal::UnshareableRemote.code(),
        "unshareable-remote"
    );
    assert_eq!(
        RemoteRefusal::NotSynced.detail(),
        "This spot only exists on this device."
    );
    assert_eq!(
        RemoteRefusal::UnshareableRemote.detail(),
        "This spot's sync server can't be shared."
    );
}
```

`test_state`, `put_repo` and `api_router_with_state` are already imported by that mod (see its `use crate::router::tests::{...}` line). Add any that are missing.

- [ ] **Step 2: Run test to verify it fails**

Run: `nix develop -c test:web:debug`
Expected: FAIL — `cannot find type 'RemoteRequirement'`

This is the only way to run wasm tests; it builds the whole workspace archive and drives headless Chrome. Expect it to be slow.

- [ ] **Step 3: Add the types and change the return**

In `rust/tonk-worker/src/router/create_invite.rs`, above `resolve_remote_url` (line 344):

```rust
/// Why a spot cannot produce a shareable invite.
///
/// Both variants mean the same thing to the recipient — an invite that can
/// never sync, so they land in a spot that stays permanently empty — but
/// only [`Self::NotSynced`] is repairable by attaching a remote, so only it
/// offers the prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RemoteRefusal {
    /// `main` has no upstream at all. Repairable.
    NotSynced,
    /// `main` tracks something that is not a remote, or a remote whose site
    /// address is not a UCAN endpoint. An invite URL has no way to express
    /// either, so there is nothing to offer.
    UnshareableRemote,
}

impl RemoteRefusal {
    /// The stable class string carried on `xyz.tonk.share/blocked`.
    pub(crate) fn code(self) -> &'static str {
        match self {
            Self::NotSynced => "not-synced",
            Self::UnshareableRemote => "unshareable-remote",
        }
    }

    /// The sentence shown to the user.
    pub(crate) fn detail(self) -> &'static str {
        match self {
            Self::NotSynced => "This spot only exists on this device.",
            Self::UnshareableRemote => "This spot's sync server can't be shared.",
        }
    }
}

/// The outcome of probing a repository for a shareable sync endpoint.
#[derive(Debug, Clone)]
pub(crate) enum RemoteRequirement {
    /// A UCAN endpoint an invite can advertise.
    Ready(Url),
    /// No such endpoint. See [`RemoteRefusal`].
    Refused(RemoteRefusal),
}
```

Then rewrite the two functions. Replace the doc comment on `resolve_remote_url` (lines 329-343) with:

```rust
/// Probe `main`'s upstream and, when it points at a remote, pull the
/// UCAN access-service URL off the remote's site address.
///
/// - `Ok(Ready(url))` — the endpoint an invite advertises as `&remote=`.
/// - `Ok(Refused(reason))` — no such endpoint. Callers refuse to mint:
///   an invite with no remote lands its recipient in a spot that can never
///   fill, so it has no use, and returning one silently would mask exactly
///   the config drift the inviter cannot see.
/// - `Err(...)` — branch/remote load failed or the stored UCAN endpoint
///   won't parse. Failing loudly is right for the same reason.
///
/// `main` is hardcoded; see `project_main_branch_implicit_creation` memory
/// note on why `.open()` is used here despite not being strictly read-only.
```

And change both bodies. `resolve_remote_url` keeps delegating:

```rust
pub(crate) async fn resolve_remote_url<R>(
    tonk: &crate::worker::TonkState,
    repository: &dialog_repository::Repository<R>,
) -> Result<RemoteRequirement, TonkWorkerError>
where
    R: Principal + Clone,
{
    resolve_remote_url_with(repository, &tonk.operator).await
}
```

`resolve_remote_url_with` returns the new type at each exit:

```rust
pub(crate) async fn resolve_remote_url_with<R>(
    repository: &dialog_repository::Repository<R>,
    operator: &crate::worker::DefaultOperator,
) -> Result<RemoteRequirement, TonkWorkerError>
where
    R: Principal + Clone,
{
    let main = repository
        .branch("main")
        .open()
        .perform(operator)
        .await
        .map_err(|e| {
            TonkWorkerError::Internal(format!(
                "failed to probe branch 'main' while resolving remote URL: {e}"
            ))
        })?;

    let remote_name = match main.upstream() {
        Some(Upstream::Remote { remote, .. }) => remote,
        None => return Ok(RemoteRequirement::Refused(RemoteRefusal::NotSynced)),
        Some(_) => {
            return Ok(RemoteRequirement::Refused(RemoteRefusal::UnshareableRemote));
        }
    };

    let remote = repository
        .remote(remote_name.as_str())
        .load()
        .perform(operator)
        .await
        .map_err(|e| {
            TonkWorkerError::Internal(format!(
                "branch 'main' upstream names remote '{remote_name}' but it failed to load: {e}"
            ))
        })?;

    match remote.address().site() {
        SiteAddress::Ucan(ucan) => Url::parse(ucan.endpoint())
            .map(RemoteRequirement::Ready)
            .map_err(|e| {
                TonkWorkerError::Internal(format!(
                    "remote '{remote_name}' has unparseable UCAN endpoint '{}': {e}",
                    ucan.endpoint()
                ))
            }),
        _ => Ok(RemoteRequirement::Refused(RemoteRefusal::UnshareableRemote)),
    }
}
```

Note the split of the old `Some(_) | None` arm: that collapse is exactly what made the two cases indistinguishable.

- [ ] **Step 4: Fix the two call sites so the crate compiles**

These are temporary shims; Tasks 3 and 4 replace them.

In `rust/tonk-worker/src/router/create_invite.rs:181`, replace `let remote_url = resolve_remote_url(&tonk, &repository).await?;` with:

```rust
    let remote_url = match resolve_remote_url(&tonk, &repository).await? {
        RemoteRequirement::Ready(url) => Some(url),
        RemoteRequirement::Refused(_) => None,
    };
```

In `rust/tonk-worker/src/router/repository.rs:778`, replace the `match super::create_invite::resolve_remote_url(...)` expression with:

```rust
    let remote = match super::create_invite::resolve_remote_url(&tonk, &repository).await? {
        super::create_invite::RemoteRequirement::Ready(url) => {
            let encoded: String =
                url::form_urlencoded::byte_serialize(url.as_str().as_bytes()).collect();
            format!("&remote={encoded}")
        }
        super::create_invite::RemoteRequirement::Refused(_) => String::new(),
    };
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `nix develop -c test:web:debug`
Expected: PASS, including the two new tests. Every previously-passing test still passes — behaviour is unchanged so far.

- [ ] **Step 6: Commit**

```bash
git add rust/tonk-worker/src/router/create_invite.rs rust/tonk-worker/src/router/repository.rs
git commit -m "refactor(tonk-worker): distinguish why a remote could not be resolved"
```

---

### Task 3: `run_invite` refuses before minting

**Files:**
- Modify: `rust/tonk-worker/src/router/repository.rs:705-865` (`run_invite`)

**Interfaces:**
- Consumes: `RemoteRequirement`/`RemoteRefusal` from Task 2, `tonk_schema::command::ShareBlocked` from Task 1.
- Produces: `async fn publish_share_blocked(tonk: &TonkState, repo_name: &str, subject: Entity, code: &str, detail: &str, time: f64)` — reused by Task 6's handler.

- [ ] **Step 1: Write the failing test**

Add to `rust/tonk-worker/src/router/repository.rs`'s existing wasm test mod:

```rust
/// A share click on a spot with no upstream mints nothing and leaves a
/// refusal on the overlay instead.
#[dialog_common::test]
async fn it_refuses_to_mint_without_a_remote() {
    let state = test_state().await;
    let (app, _state, _lsp) = api_router_with_state(state.clone());
    let key = put_repo(&app, "test-refuse-mint").await;

    run_invite_with_time(&state, &key, 1234.0).await;

    let blocked = share_blocked_rows(&state, &key).await;
    assert_eq!(blocked.len(), 1, "one refusal recorded");
    assert_eq!(blocked[0].0, "not-synced");
    assert_eq!(blocked[0].1, "This spot only exists on this device.");
    assert_eq!(blocked[0].2, 1234.0, "echoes the command's timestamp");

    let invitations = content_invitations(&state, &key).await;
    assert!(
        invitations.is_empty(),
        "a refused mint records no invitation"
    );
}
```

`content_invitations` already exists in `crate::router::tests` (imported at `create_invite.rs`'s test mod — re-export or import it here the same way).

Write the two helpers in the same test mod:

```rust
/// Drive `run_invite` with a fixed timestamp, the way a `tonk:invite`
/// transient would.
async fn run_invite_with_time(state: &AppState, repo: &str, time: f64) {
    let env = crate::router::CommandEnv::new(state.clone(), crate::router::CommandOrigin::default());
    let _ = super::run_invite(&env, repo, time).await;
}

/// Read back every `ShareBlocked` row on the repo's content branch overlay
/// as `(blocked, detail, time)`.
async fn share_blocked_rows(state: &AppState, repo: &str) -> Vec<(String, String, f64)> {
    use dialog_query::Term;
    use tonk_schema::command::ShareBlocked;

    let tonk = state.read().await;
    let branch = tonk
        .reactor
        .repository(repo)
        .branch(CONTENT_BRANCH)
        .acquire(&tonk.operator)
        .await
        .expect("content branch opens");
    let rows: Vec<ShareBlocked> = branch
        .handle()
        .query()
        .select(dialog_query::Query::<ShareBlocked> {
            this: Term::var("this"),
            blocked: Term::var("blocked"),
            detail: Term::var("detail"),
            time: Term::var("time"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .expect("share-blocked query");
    rows.into_iter()
        .map(|row| (row.blocked.0, row.detail.0, row.time.0))
        .collect()
}
```

If the query builder spelling differs, copy it from `replica_still_recorded` at `repository.rs:1858`, which does the same shape against the meta branch.

- [ ] **Step 2: Run test to verify it fails**

Run: `nix develop -c test:web:debug`
Expected: FAIL — `run_invite` takes 2 arguments, not 3.

- [ ] **Step 3: Thread the timestamp and refuse**

`run_invite` needs the triggering timestamp to echo. Change its signature (line 705) to:

```rust
async fn run_invite(
    env: &crate::router::CommandEnv,
    repo_name: &str,
    time: f64,
) -> Result<(), TonkWorkerError> {
```

In `InviteHandler::run` (around line 677), decode the timestamp alongside the repo and pass it through. Add next to the existing `repo_name` decode:

```rust
        let time = facts
            .first()
            .map(|artifact| artifact.of.clone())
            .and_then(|entity| tonk_schema::command::Invite::decode(entity, facts))
            .map(|command| command.time.0)
            .unwrap_or_default();
```

and change the call to `run_invite(&env, &repo_name, time).await`.

Inside `run_invite`, move the remote probe ahead of the keypair. Delete the `generate_ephemeral` call at the top (currently the first statement after the `use` block) and the `remote` block at line 778. The function now opens:

```rust
    let tonk = env.state().read().await;

    let repository = tonk
        .profile
        .repository(repo_name)
        .load()
        .perform(&tonk.operator)
        .await
        .map_err(|e| {
            TonkWorkerError::NotFound(format!("Repository '{repo_name}' not found: {e}"))
        })?;

    // Both facts are keyed by the repository's *subject* DID — the entity
    // the share view already addresses (`entity={subject}`) — not the
    // membership DID.
    let subject_entity = repository
        .did()
        .to_string()
        .parse::<Entity>()
        .map_err(|e| {
            TonkWorkerError::Internal(format!("repository subject is not a valid entity: {e}"))
        })?;

    // Resolve the sync endpoint BEFORE minting anything. An invite with no
    // remote lands its recipient in a spot that can never fill, so there is
    // nothing worth generating key material for. Refusing here also means a
    // refusal costs no delegation and rotates no credential.
    let remote_url = match super::create_invite::resolve_remote_url(&tonk, &repository).await? {
        super::create_invite::RemoteRequirement::Ready(url) => url,
        super::create_invite::RemoteRequirement::Refused(reason) => {
            log!(
                "Invite for repo '{}' refused: {}",
                repo_name,
                reason.code()
            );
            drop(tonk);
            publish_share_blocked(
                env.state(),
                repo_name,
                subject_entity,
                reason.code(),
                reason.detail(),
                time,
            )
            .await;
            return Ok(());
        }
    };

    // A ready-to-append URL query suffix (`&remote=<percent-encoded-url>`).
    // The share view appends it verbatim between `?access=…` and the `#seed`.
    let encoded: String =
        url::form_urlencoded::byte_serialize(remote_url.as_str().as_bytes()).collect();
    let remote = format!("&remote={encoded}");

    // Mint a fresh membership keypair. Its private seed becomes the invite
    // URL's `#` fragment; its public DID is the audience the repo access is
    // delegated to. The browser never sees this DID.
    let (signer, seed_bytes) = super::create_invite::generate_ephemeral().await?;
    let membership_did = signer.did();
    let seed = bs58::encode(seed_bytes).into_string();
```

`generate_ephemeral` is `async` and the state guard is held across it, which is why the original called it first. Holding the read guard across an await is what the rest of this function already does (the delegation and both commits run under it), so keep it — but note the `drop(tonk)` in the refusal arm above: `publish_share_blocked` re-locks, so the guard must be released first.

The rest of the function is unchanged from `let delegation` onwards.

- [ ] **Step 4: Add the publish helper**

Directly after `run_invite`, add:

```rust
/// Record why a share click could not mint, on the spot's content-branch
/// session overlay, keyed by the subject.
///
/// Overlay-only, exactly like the `Credential` a successful mint writes: a
/// refusal is this device's answer to this click, not a property of the spot,
/// and it must never replicate. The write schedules a poll, so the dispatcher's
/// drain fans it out to the share control's subscription in the same pass as a
/// successful mint would have been.
///
/// `time` echoes the refused command's timestamp. The fact is cardinality-one
/// on the subject, so it lingers and replays on every resubscribe; the echo is
/// what lets the control tell this refusal from a replay of an older one, which
/// is why the fact never needs retracting.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn publish_share_blocked(
    state: &AppState,
    repo_name: &str,
    subject: Entity,
    code: &str,
    detail: &str,
    time: f64,
) {
    use tonk_schema::command::ShareBlocked;
    use tonk_schema::domain::share;

    let tonk = state.read().await;
    if let Err(error) = tonk
        .reactor
        .repository(repo_name)
        .branch(CONTENT_BRANCH)
        .overlay()
        .assert(ShareBlocked {
            this: subject,
            blocked: share::Blocked(code.to_owned()),
            detail: share::Detail(detail.to_owned()),
            time: share::Time(time),
        })
        .write()
        .perform(&tonk.operator)
        .await
    {
        log!("failed to publish share refusal for '{repo_name}': {error}");
    }
}
```

Add a `#[cfg(not(...))]` no-op twin if the surrounding module needs one to build natively — follow whichever pattern the neighbouring `publish_sync_status_attr` uses.

- [ ] **Step 5: Run tests to verify they pass**

Run: `nix develop -c test:web:debug`
Expected: PASS. Any pre-existing test that minted an invite against a local-only repo now fails — fix each by attaching a remote first (see `router.rs:1146-1165`, which already exercises `POST /api/repository/{repo}/remote`), not by weakening the assertion.

- [ ] **Step 6: Commit**

```bash
git add rust/tonk-worker/src/router/repository.rs
git commit -m "feat(tonk-worker): refuse to mint an invite with no sync remote"
```

---

### Task 4: the HTTP mint route refuses too

**Files:**
- Modify: `rust/tonk-worker/src/router/create_invite.rs:130-200`

**Interfaces:**
- Consumes: `RemoteRequirement`/`RemoteRefusal` from Task 2.

- [ ] **Step 1: Write the failing test**

In `create_invite.rs`'s test mod:

```rust
/// The HTTP mint route refuses a local-only repository rather than
/// answering with an invite that can never sync.
#[dialog_common::test]
async fn it_rejects_a_mint_for_a_local_only_repository() {
    let (app, _state, _lsp) = api_router_with_state(test_state().await);
    let key = put_repo(&app, "test-http-local-only").await;

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/repository/{key}/invite"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CONFLICT);
}
```

Match the request shape (method, body) to whatever the existing mint tests in this mod send.

- [ ] **Step 2: Run test to verify it fails**

Run: `nix develop -c test:web:debug`
Expected: FAIL — got `200 OK`

- [ ] **Step 3: Refuse**

Replace the Task 2 shim at `create_invite.rs:181`:

```rust
    let remote_url = match resolve_remote_url(&tonk, &repository).await? {
        RemoteRequirement::Ready(url) => url,
        RemoteRequirement::Refused(reason) => {
            return Err(TonkWorkerError::Conflict(format!(
                "cannot mint an invite for '{name}': {} ({})",
                reason.detail(),
                reason.code()
            )));
        }
    };
```

Substitute whatever the route's repository-name binding is actually called for `name`. Downstream, `remote_url` is now a `Url` rather than `Option<Url>` — remove the `Option` handling that follows it.

`TonkWorkerError::Conflict` maps to HTTP 409 (`error.rs:26`).

- [ ] **Step 4: Run tests to verify they pass**

Run: `nix develop -c test:web:debug`
Expected: PASS

Then sweep the rest of the workspace for callers that assumed a local-only mint succeeds:

Run: `rg -n "invite" bench/ rust/tonk-cli/ --glob '!target' | rg -i "local|no.?remote"`
Fix any that mint without attaching a remote first.

- [ ] **Step 5: Commit**

```bash
git add rust/tonk-worker/src/router/create_invite.rs
git commit -m "feat(tonk-worker): reject HTTP invite minting without a remote"
```

---

### Task 5: the `tonk:enable-sync` command concept

**Files:**
- Modify: `rust/tonk-schema/src/domain.rs` (add `enable_sync` inside the existing `pub mod command`, beside `invite` at line 242)
- Modify: `rust/tonk-schema/src/command.rs` (add `EnableSync` after `Invite`, which ends at line 163)

**Interfaces:**
- Produces: `tonk_schema::command::EnableSync { this: Entity, time: enable_sync::TimeStamp, marker: enable_sync::EnableSync }`, plus `enable_sync::{Space, Remote, Share}` attributes read from raw facts (not matched fields). Attribute names: `dom.event/time-stamp`, `dom.event.current-target.dataset/enable-sync`, `xyz.tonk.enable-sync/space`, `xyz.tonk.enable-sync/remote`, `xyz.tonk.enable-sync/share`.

- [ ] **Step 1: Write the failing test**

In `rust/tonk-schema/src/command.rs`, beside the Task 1 test mod:

```rust
#[cfg(test)]
mod enable_sync {
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_browser);

    #[dialog_common::test]
    fn it_carries_a_marker_no_other_command_carries() {
        use dialog_query::Attribute as _;
        use crate::domain::command::enable_sync;

        assert_eq!(
            enable_sync::EnableSync::attribute().to_string(),
            "dom.event.current-target.dataset/enable-sync"
        );
        assert_eq!(
            enable_sync::Space::attribute().to_string(),
            "xyz.tonk.enable-sync/space"
        );
        assert_eq!(
            enable_sync::Remote::attribute().to_string(),
            "xyz.tonk.enable-sync/remote"
        );
        assert_eq!(
            enable_sync::Share::attribute().to_string(),
            "xyz.tonk.enable-sync/share"
        );
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `nix develop -c cargo test -p tonk-schema enable_sync`
Expected: FAIL — `could not find 'enable_sync'`

- [ ] **Step 3: Add the attributes**

In `rust/tonk-schema/src/domain.rs`, inside `pub mod command`, after the `invite` module:

```rust
    /// Attributes the `tonk:enable-sync` command carries.
    ///
    /// The share control dispatches this routelessly when a user accepts the
    /// offer to turn sync on, so the target spot and the endpoint both travel
    /// on the transient rather than being inferred from a dispatch origin.
    pub mod enable_sync {
        use super::super::Entity;
        use super::Attribute;

        /// The submit event's timestamp. Makes each acceptance a distinct
        /// transient, and is echoed back on any refusal so the share control
        /// can match a result to the click that caused it.
        #[derive(Attribute, Clone, PartialEq, PartialOrd)]
        #[domain("dom.event")]
        pub struct TimeStamp(pub f64);

        /// The marker giving this command an attribute no other command
        /// carries, so a transient decodes as exactly one command. Same role
        /// as [`super::invite::Invite`]; the derived attribute is
        /// `dom.event.current-target.dataset/enable-sync`.
        ///
        /// An `Entity`, not a `String`: the value (`tonk:enable-sync`) has a
        /// `:`, and the worker's untagged `Value` decode reads any `:`-bearing
        /// string as an `Entity`.
        #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
        #[domain("dom.event.current-target.dataset")]
        pub struct EnableSync(pub Entity);

        /// The spot to attach the remote to.
        #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
        #[domain("xyz.tonk.enable-sync")]
        pub struct Space(pub Entity);

        /// The UCAN access-service endpoint to attach as `origin`.
        ///
        /// Read from the raw facts rather than being a matched field: a URL
        /// round-trips through JSON and the worker's untagged `Value` decode
        /// picks `Entity` for any string with a `:`, so a `String`-typed field
        /// would never decode one. The handler tolerates both representations,
        /// the same way `remote_from_facts` does for `CreateSpace`.
        #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
        #[domain("xyz.tonk.enable-sync")]
        pub struct Remote(pub String);

        /// Present when the caller wants an invite minted once the remote is
        /// attached. Absent means attach only.
        #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
        #[domain("xyz.tonk.enable-sync")]
        pub struct Share(pub Entity);
    }
```

- [ ] **Step 4: Add the command**

In `rust/tonk-schema/src/command.rs`, after `Invite`'s `impl Command` block (line 163):

```rust
/// Attach a sync remote to an existing spot, and optionally mint an invite
/// once it is attached.
///
/// Dispatched routelessly by the share control when a user accepts the offer
/// to turn sync on. `space`, `remote` and the `share` marker ride on the
/// transient as raw facts the handler reads directly — `remote` because a URL
/// cannot be a `String`-typed field (see
/// [`crate::domain::command::enable_sync::Remote`]), the other two for
/// symmetry with it.
///
/// This is deliberately NOT the `space/enable-sync` command seeded in
/// `core.yaml`: that one shares `CreateSpace`'s trigger attribute, so a
/// handler registered against it would fire alongside `CreateSpaceHandler`
/// and mint a new spot instead of attaching to the existing one.
#[derive(Concept, Debug, Clone, PartialEq, PartialOrd)]
pub struct EnableSync {
    /// The command entity (a fresh id per invocation).
    pub this: Entity,
    /// The acceptance timestamp — distinguishes one click from the next.
    pub time: crate::domain::command::enable_sync::TimeStamp,
    /// Per-command marker that keeps this command's shape distinct from
    /// every other transient's.
    pub marker: crate::domain::command::enable_sync::EnableSync,
}

/// `EnableSync` is a [`dialog_capability::Command`]; its handler lives in
/// `tonk-worker` (attaches the remote, then mints when asked).
impl Command for EnableSync {
    type Input = Self;
    type Output = ();
}
```

- [ ] **Step 5: Run test to verify it passes**

Run: `nix develop -c cargo test -p tonk-schema enable_sync`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add rust/tonk-schema/src/domain.rs rust/tonk-schema/src/command.rs
git commit -m "feat(tonk-schema): add the tonk:enable-sync command"
```

---

### Task 6: `EnableSyncHandler`

**Files:**
- Modify: `rust/tonk-worker/src/router/repository.rs` (add after `InviteHandler`, which ends around line 695)
- Modify: `rust/tonk-worker/src/router/command.rs:120-129` (registration)

**Interfaces:**
- Consumes: `EnableSync` from Task 5, `publish_share_blocked` and the 3-arg `run_invite` from Task 3, `enable_sync_inner` (`repository.rs:1780`).
- Produces: `pub(crate) struct EnableSyncHandler` with `EnableSyncHandler::new()`.

- [ ] **Step 1: Write the failing test**

In `repository.rs`'s wasm test mod:

```rust
/// Attaching a remote through the command targets the EXISTING spot. The
/// `space/enable-sync` command in `core.yaml` shares `CreateSpace`'s trigger
/// attribute and so mints a new spot instead; this guards against that.
#[dialog_common::test]
async fn it_attaches_the_remote_to_the_existing_spot() {
    let state = test_state().await;
    let (app, _state, _lsp) = api_router_with_state(state.clone());
    let (key, subject) = put_repo_info(&app, "test-enable-sync").await;
    let before = existing_space_labels(&state).await.len();

    dispatch_enable_sync(&state, &subject, "https://example.test/ucan/", false, 1.0).await;

    assert_eq!(
        existing_space_labels(&state).await.len(),
        before,
        "no new spot was created"
    );
    assert!(
        has_remote_upstream(&state, &key).await,
        "the existing spot now tracks origin/main"
    );
}

/// Without the `share` marker the handler attaches and stops.
#[dialog_common::test]
async fn it_mints_only_when_asked_to_share() {
    let state = test_state().await;
    let (app, _state, _lsp) = api_router_with_state(state.clone());
    let (key, subject) = put_repo_info(&app, "test-enable-sync-no-share").await;

    dispatch_enable_sync(&state, &subject, "https://example.test/ucan/", false, 1.0).await;

    assert!(
        content_invitations(&state, &key).await.is_empty(),
        "attach-only records no invitation"
    );
}

/// With the marker, the attach is followed by a mint — the single-click path.
#[dialog_common::test]
async fn it_mints_after_attaching_when_asked_to_share() {
    let state = test_state().await;
    let (app, _state, _lsp) = api_router_with_state(state.clone());
    let (key, subject) = put_repo_info(&app, "test-enable-sync-share").await;

    dispatch_enable_sync(&state, &subject, "https://example.test/ucan/", true, 1.0).await;

    assert_eq!(
        content_invitations(&state, &key).await.len(),
        1,
        "the attach is followed by exactly one mint"
    );
}
```

`put_repo_info` (`router.rs:800`) returns `(routing key, subject DID)`; the command carries the subject, the handler derives the key from it. `existing_space_labels` is already defined at `repository.rs:375`.

`https://example.test/ucan/` is never dialled: `ensure_remote_config` records the remote and opens its branch handle locally. The existing wasm test at `router.rs:1146-1165` already attaches a remote this way and passes offline — if these tests turn out to need the network, copy the URL that one uses rather than adding a mock.

Two helpers, in the same test mod:

```rust
/// Build the `tonk:enable-sync` transient the FAB dispatches and run it
/// through `dispatch`, the way `/transact` does after a commit. Going through
/// `dispatch` (not the handler directly) means this also covers registration
/// and trigger matching.
async fn dispatch_enable_sync(
    state: &AppState,
    subject: &str,
    remote: &str,
    share: bool,
    time: f64,
) {
    use dialog_artifacts::{Changes, Statement};
    use dialog_query::{Entity, the};

    let of: Entity = "tonk:enable-sync-test".parse().expect("entity URI");
    let mut changes = Changes::new();
    the!("dom.event/time-stamp")
        .of(of.clone())
        .is(time)
        .assert(&mut changes);
    the!("dom.event.current-target.dataset/enable-sync")
        .of(of.clone())
        .is("tonk:enable-sync".parse::<Entity>().expect("marker entity"))
        .assert(&mut changes);
    the!("xyz.tonk.enable-sync/space")
        .of(of.clone())
        .is(subject.parse::<Entity>().expect("subject entity"))
        .assert(&mut changes);
    the!("xyz.tonk.enable-sync/remote")
        .of(of.clone())
        .is(remote.to_string())
        .assert(&mut changes);
    if share {
        the!("xyz.tonk.enable-sync/share")
            .of(of)
            .is("tonk:share".parse::<Entity>().expect("share entity"))
            .assert(&mut changes);
    }

    crate::router::dispatch(state, crate::router::CommandOrigin::default(), changes).await;
}

/// Whether the repo's `main` tracks a remote upstream — the exact condition
/// `resolve_remote_url_with` probes.
async fn has_remote_upstream(state: &AppState, repo: &str) -> bool {
    use dialog_repository::Upstream;

    let tonk = state.read().await;
    let Ok(repository) = tonk
        .profile
        .repository(repo)
        .load()
        .perform(&tonk.operator)
        .await
    else {
        return false;
    };
    let Ok(main) = repository
        .branch("main")
        .open()
        .perform(&tonk.operator)
        .await
    else {
        return false;
    };
    matches!(main.upstream(), Some(Upstream::Remote { .. }))
}
```

The `the!(...).of(...).is(...).assert(&mut changes)` construction is copied from `ping_transient` at `command.rs:246`. Match its exact import list (`dialog_artifacts::Statement`, `dialog_query::{Entity, the}`) and adjust if the `Upstream` import path differs from what `create_invite.rs` uses.

- [ ] **Step 2: Run test to verify it fails**

Run: `nix develop -c test:web:debug`
Expected: FAIL — no handler matches, so nothing is attached.

- [ ] **Step 3: Write the fact readers**

In `repository.rs`, beside `remote_from_facts` (line 283):

```rust
/// The `tonk:enable-sync` transient's target spot, read from the raw facts.
#[cfg(any(all(target_arch = "wasm32", target_os = "unknown"), test))]
const ENABLE_SYNC_SPACE_ATTR: &str = "xyz.tonk.enable-sync/space";

/// The `tonk:enable-sync` transient's endpoint, read from the raw facts.
#[cfg(any(all(target_arch = "wasm32", target_os = "unknown"), test))]
const ENABLE_SYNC_REMOTE_ATTR: &str = "xyz.tonk.enable-sync/remote";

/// Marker asking the handler to mint once the remote is attached.
#[cfg(any(all(target_arch = "wasm32", target_os = "unknown"), test))]
const ENABLE_SYNC_SHARE_ATTR: &str = "xyz.tonk.enable-sync/share";

/// Read a fact's value as a string, tolerating both the `String` and
/// `Entity` representations — a URL or a DID round-trips through JSON as an
/// `Entity` (any `:`-bearing string does), so a single-representation read
/// would silently miss them. Mirrors [`remote_from_facts`].
#[cfg(any(all(target_arch = "wasm32", target_os = "unknown"), test))]
fn text_fact(facts: &crate::reactor::EntityFacts, attribute: &str) -> Option<String> {
    use dialog_artifacts::Value;

    facts
        .iter()
        .find(|artifact| artifact.the.to_string() == attribute)
        .and_then(|artifact| match &artifact.is {
            Value::String(text) => Some(text.clone()),
            Value::Entity(entity) => Some(entity.to_string()),
            _ => None,
        })
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
}
```

- [ ] **Step 4: Write the handler**

After `InviteHandler`'s `impl` block:

```rust
/// Attach a sync remote to an existing spot, then mint an invite when the
/// transient asks for one.
///
/// The share control dispatches this when a user accepts the offer to turn
/// sync on after a refused share. Minting from inside the handler is what
/// makes that a single click: the control needs no completion signal for the
/// attach, because success reaches it as a new invite link on the
/// subscription it already holds — the same path an ordinary mint takes.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) struct EnableSyncHandler {
    attributes: Vec<String>,
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
impl EnableSyncHandler {
    pub(crate) fn new() -> Self {
        use crate::reactor::Decode as _;
        Self {
            attributes: tonk_schema::command::EnableSync::trigger_attributes(),
        }
    }
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
impl crate::reactor::CommandHandler<crate::router::CommandEnv> for EnableSyncHandler {
    fn trigger_attributes(&self) -> &[String] {
        &self.attributes
    }

    fn matches(&self, facts: &crate::reactor::EntityFacts) -> bool {
        use crate::reactor::Decode as _;
        facts
            .first()
            .map(|artifact| artifact.of.clone())
            .and_then(|this| tonk_schema::command::EnableSync::decode(this, facts))
            .is_some()
    }

    fn run(
        &self,
        facts: &crate::reactor::EntityFacts,
        env: &crate::router::CommandEnv,
    ) -> crate::reactor::RunFuture {
        use crate::reactor::Decode as _;
        use tonk_schema::prelude::DidExt as _;

        let time = facts
            .first()
            .map(|artifact| artifact.of.clone())
            .and_then(|entity| tonk_schema::command::EnableSync::decode(entity, facts))
            .map(|command| command.time.0)
            .unwrap_or_default();
        let space = text_fact(facts, ENABLE_SYNC_SPACE_ATTR);
        let remote = text_fact(facts, ENABLE_SYNC_REMOTE_ATTR);
        let share = text_fact(facts, ENABLE_SYNC_SHARE_ATTR).is_some();
        let env = env.clone();

        Box::pin(async move {
            let (Some(space), Some(remote)) = (space, remote) else {
                log!("EnableSync: missing space or remote, skipping");
                return;
            };
            let Ok(did) = space.parse::<dialog_varsig::Did>() else {
                log!("EnableSync: '{}' is not a DID", space);
                return;
            };
            let key = did.repo_key().to_owned();
            log!("command EnableSync repo={} share={}", key, share);

            if let Err(error) = enable_sync_inner(env.state(), &key, &remote).await {
                log!("EnableSync '{}' failed: {}", key, error);
                if share {
                    let subject = match space.parse::<Entity>() {
                        Ok(entity) => entity,
                        Err(e) => {
                            log!("EnableSync: '{}' is not an entity: {}", space, e);
                            return;
                        }
                    };
                    publish_share_blocked(
                        env.state(),
                        &key,
                        subject,
                        "attach-failed",
                        &format!("Could not turn on sync: {error}"),
                        time,
                    )
                    .await;
                }
                return;
            }

            if share && let Err(error) = run_invite(&env, &key, time).await {
                log!("EnableSync '{}': mint after attach failed: {}", key, error);
            }
        })
    }
}
```

- [ ] **Step 5: Register it**

In `rust/tonk-worker/src/router/command.rs`, beside the other registrations (line 122):

```rust
        registry.register(Box::new(super::repository::EnableSyncHandler::new()));
```

- [ ] **Step 6: Run tests to verify they pass**

Run: `nix develop -c test:web:debug`
Expected: PASS

- [ ] **Step 7: Commit**

```bash
git add rust/tonk-worker/src/router/repository.rs rust/tonk-worker/src/router/command.rs
git commit -m "feat(tonk-worker): add an enable-sync command that attaches and mints"
```

---

### Task 7: one element, several tagged subscriptions

**Files:**
- Modify: `rust/tonk-fab/src/subscribing.rs:92-199`

**Interfaces:**
- Produces: `Scaffold::connect_all(&self, this: &HtmlElement, behaviours: Vec<Rc<dyn Subscribing>>)`. `Scaffold::connect` keeps its signature and delegates to it. Frames are routed to the behaviour whose `tag()` matches `opts.tag`; with exactly one behaviour and no tag on the frame, it goes to that behaviour.

- [ ] **Step 1: Write the failing test**

Add a test mod to `rust/tonk-fab/src/subscribing.rs`:

```rust
#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod tests {
    use super::*;
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    wasm_bindgen_test_configure!(run_in_browser);

    use std::cell::RefCell;
    use std::rc::Rc;

    /// A behaviour that records which payloads it was handed.
    struct Recorder {
        tag: &'static str,
        seen: Rc<RefCell<Vec<String>>>,
    }

    impl Subscribing for Recorder {
        fn query_body(&self, _this: &HtmlElement) -> Result<String, String> {
            Ok("{}".to_owned())
        }
        fn render_reset(&self, _host: &HtmlElement, payload: &JsValue) {
            self.seen
                .borrow_mut()
                .push(payload.as_string().unwrap_or_default());
        }
        fn render_update(&self, _host: &HtmlElement, _payload: &JsValue) {}
        fn tag(&self) -> &'static str {
            self.tag
        }
    }

    /// A frame tagged for one behaviour reaches only that behaviour.
    #[dialog_common::test]
    fn it_routes_frames_by_tag() {
        let document = window().unwrap().document().unwrap();
        let host: HtmlElement = document
            .create_element("div")
            .unwrap()
            .dyn_into()
            .unwrap();
        host.set_attribute("space", "did:key:z6Mk").unwrap();

        let first = Rc::new(RefCell::new(Vec::new()));
        let second = Rc::new(RefCell::new(Vec::new()));
        let scaffold = Scaffold::default();
        scaffold.connect_all(
            &host,
            vec![
                Rc::new(Recorder { tag: "one", seen: Rc::clone(&first) }),
                Rc::new(Recorder { tag: "two", seen: Rc::clone(&second) }),
            ],
        );

        // Deliver a frame the way the host does: element.__tonkReset(payload, {tag}).
        let opts = js_sys::Object::new();
        Reflect::set(&opts, &"tag".into(), &"two".into()).unwrap();
        let reset = Reflect::get(&host, &"__tonkReset".into())
            .unwrap()
            .dyn_into::<Function>()
            .unwrap();
        reset
            .call2(&JsValue::NULL, &JsValue::from_str("payload"), &opts)
            .unwrap();

        assert!(first.borrow().is_empty(), "untagged behaviour untouched");
        assert_eq!(second.borrow().as_slice(), ["payload"]);
    }
}
```

Add `dialog-common`, `tokio` and `wasm-bindgen-test` dev-deps to `rust/tonk-fab/Cargo.toml` if they are missing (they are already present).

- [ ] **Step 2: Run test to verify it fails**

Run: `nix develop -c test:web:debug`
Expected: FAIL — `no method named 'connect_all'`

- [ ] **Step 3: Multiplex**

Replace the `Scaffold` struct and its `impl` (lines 92-160) with:

```rust
/// The shared subscribe/retry/teardown state a subscribing element's
/// `CustomElement` struct embeds alongside its own fields.
///
/// An element may hold SEVERAL subscriptions. The host delivers every frame
/// to the same `reset`/`update` methods, so they are told apart by the `tag`
/// in the frame's options — the same tag each behaviour supplied when it
/// subscribed. `<tonk-share>` needs this: its invite link and its refusal
/// signal are separate inline predicates over different raw attributes, and a
/// single predicate over both would resolve only when BOTH are present, which
/// is never.
#[derive(Default)]
pub struct Scaffold {
    /// Live subscriptions, paired with the tag they were opened under.
    subscriptions: Rc<RefCell<Vec<(String, Subscription)>>>,
    reset: Rc<RefCell<Option<FrameClosure>>>,
    update: Rc<RefCell<Option<FrameClosure>>>,
}

impl Scaffold {
    /// Run from `connected_callback` with a single behaviour. See
    /// [`Self::connect_all`].
    pub fn connect(&self, this: &HtmlElement, behaviour: Rc<dyn Subscribing>) {
        self.connect_all(this, vec![behaviour]);
    }

    /// Run from `connected_callback`: stamp `with`, install the `reset`/
    /// `update` frame delegates (forwarded from the prototype shims
    /// [`install_frame_shims`] installs), and subscribe each behaviour under
    /// its own tag.
    ///
    /// The routing context comes from the FIRST behaviour — every behaviour on
    /// one element shares that element's `with`. A no-op when it returns
    /// `None` (context not ready yet); the attribute-changed callback re-runs
    /// this once it lands.
    pub fn connect_all(&self, this: &HtmlElement, behaviours: Vec<Rc<dyn Subscribing>>) {
        let Some(first) = behaviours.first() else {
            return;
        };
        let Some(with) = first.resolve_with(this) else {
            return;
        };
        let _ = this.set_attribute("with", &with);

        let routed = behaviours.clone();
        let host = this.clone();
        let reset: FrameClosure = Closure::wrap(Box::new(move |payload: JsValue, opts: JsValue| {
            if let Some(behaviour) = route(&routed, &opts) {
                behaviour.render_reset(&host, &payload);
            }
        }));
        let _ = Reflect::set(this, &"__tonkReset".into(), reset.as_ref());
        *self.reset.borrow_mut() = Some(reset);

        let routed = behaviours.clone();
        let host = this.clone();
        let update: FrameClosure = Closure::wrap(Box::new(move |payload: JsValue, opts: JsValue| {
            if let Some(behaviour) = route(&routed, &opts) {
                behaviour.render_update(&host, &payload);
            }
        }));
        let _ = Reflect::set(this, &"__tonkUpdate".into(), update.as_ref());
        *self.update.borrow_mut() = Some(update);

        for behaviour in behaviours {
            let subscriptions = self.subscriptions.clone();
            let host = this.clone();
            // Each behaviour gets its own retry budget: one query failing to
            // build must not spend the other's attempts.
            let retry = Rc::new(RefCell::new(RetryPolicy::default()));
            spawn_local(async move {
                let tag = behaviour.tag().to_owned();
                if !host.is_connected()
                    || subscriptions.borrow().iter().any(|(open, _)| *open == tag)
                {
                    return;
                }
                subscribe(&host, behaviour.as_ref(), subscriptions, retry);
            });
        }
    }

    /// Run from `disconnected_callback`: drop every subscription and the frame
    /// delegates.
    pub fn disconnect(&self) {
        self.subscriptions.borrow_mut().clear();
        self.reset.borrow_mut().take();
        self.update.borrow_mut().take();
    }
}

/// Pick the behaviour a frame belongs to by the `tag` in its options — the
/// tag that behaviour supplied when it subscribed.
///
/// With exactly one behaviour, an absent or unrecognised tag still routes to
/// it: single-subscription elements predate tagged routing and must keep
/// working whether or not the host echoes a tag.
fn route<'a>(
    behaviours: &'a [Rc<dyn Subscribing>],
    opts: &JsValue,
) -> Option<&'a Rc<dyn Subscribing>> {
    let tag = Reflect::get(opts, &"tag".into())
        .ok()
        .and_then(|value| value.as_string());
    match tag {
        Some(tag) => behaviours
            .iter()
            .find(|behaviour| behaviour.tag() == tag)
            .or_else(|| (behaviours.len() == 1).then(|| &behaviours[0])),
        None => (behaviours.len() == 1).then(|| &behaviours[0]),
    }
}
```

Then change `subscribe`'s signature and its success arm:

```rust
fn subscribe(
    host: &HtmlElement,
    behaviour: &dyn Subscribing,
    subscriptions: Rc<RefCell<Vec<(String, Subscription)>>>,
    retry: Rc<RefCell<RetryPolicy>>,
) {
```

and inside, replace `*subscription.borrow_mut() = Some(sub);` with:

```rust
            subscriptions.borrow_mut().push((tag.to_owned(), sub));
```

Leave the rest of `subscribe` (query build, JSON parse, retry logging, `data-state="unavailable"`) unchanged.

- [ ] **Step 4: Run tests to verify they pass**

Run: `nix develop -c test:web:debug`
Expected: PASS, including every existing `<ui-space-name>` / `<ui-member-roster>` / `<ui-space-switcher>` test — they call `connect`, which is unchanged in behaviour.

- [ ] **Step 5: Commit**

```bash
git add rust/tonk-fab/src/subscribing.rs
git commit -m "feat(tonk-fab): route subscription frames to a behaviour by tag"
```

---

### Task 8: FAB pure logic

**Files:**
- Modify: `rust/tonk-fab/src/logic.rs` (`ShareState` at line 884; new functions beside `invite_claim_json` at 1376 and `invite_link_query_body` at 1428)
- Modify: `rust/tonk-fab/Cargo.toml` (add `"Location"` to the `web-sys` features list)

**Interfaces:**
- Produces:
  - `ShareState::Blocked` with `as_str() == "blocked"`
  - `pub fn enable_sync_claim_json(space: &str, remote: &str, share: bool, time: f64) -> Value`
  - `pub fn share_blocked_query_body(subject: &str) -> Result<String, String>`
  - `pub fn default_remote_url(origin: &str) -> String`
  - `pub const SHARE_TIMEOUT_MS: i32 = 15_000;`

- [ ] **Step 1: Write the failing tests**

Append to `rust/tonk-fab/src/logic.rs`:

```rust
#[cfg(test)]
mod enable_sync_claim {
    use super::*;

    #[test]
    fn it_names_the_space_remote_and_share_marker() {
        let claim = enable_sync_claim_json(
            "did:key:z6Mk",
            "https://tonk.spot/ucan/",
            true,
            7.0,
        );
        let app = &claim["claims"][0]["application"];
        assert_eq!(app["parameters"]["space"], "did:key:z6Mk");
        assert_eq!(app["parameters"]["remote"], "https://tonk.spot/ucan/");
        assert_eq!(app["parameters"]["share"], "tonk:share");
        assert_eq!(app["parameters"]["marker"], "tonk:enable-sync");
        assert_eq!(app["parameters"]["time"], 7.0);
    }

    #[test]
    fn it_omits_the_share_marker_when_not_sharing() {
        let claim = enable_sync_claim_json("did:key:z6Mk", "https://x.test/ucan/", false, 1.0);
        let app = &claim["claims"][0]["application"];
        assert!(app["parameters"].get("share").is_none());
        assert!(
            app["predicate"]["concept"]["with"].get("share").is_none(),
            "an omitted parameter must not be declared, or the assert is incomplete"
        );
    }
}

#[cfg(test)]
mod share_blocked_query {
    use super::*;

    #[test]
    fn it_reads_the_raw_share_attributes() {
        let body = share_blocked_query_body("did:key:z6Mk").expect("query body builds");
        assert!(body.contains("xyz.tonk.share/blocked"));
        assert!(body.contains("xyz.tonk.share/detail"));
        assert!(body.contains("xyz.tonk.share/time"));
        assert!(body.contains("did:key:z6Mk"));
    }

    #[test]
    fn it_rejects_an_empty_subject() {
        assert!(share_blocked_query_body("").is_err());
    }
}

#[cfg(test)]
mod default_remote {
    use super::*;

    #[test]
    fn it_appends_the_access_service_path() {
        assert_eq!(
            default_remote_url("https://tonk.spot"),
            "https://tonk.spot/ucan/"
        );
    }
}

#[cfg(test)]
mod share_state_blocked {
    use super::*;

    #[test]
    fn it_accepts_a_click_and_does_not_time_out() {
        assert_eq!(ShareState::Blocked.as_str(), "blocked");
        // A refused share must be retryable straight away.
        assert!(ShareState::Blocked.accepts_click());
        // The dialog is up; nothing should quietly revert it.
        assert!(!ShareState::Blocked.is_transient());
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `nix develop -c cargo test -p tonk-fab --lib`
Expected: FAIL — `cannot find function 'enable_sync_claim_json'`

- [ ] **Step 3: Add `ShareState::Blocked`**

In `logic.rs`, add the variant after `Copying` (line 889) and extend both matches:

```rust
    /// The mint was refused because the spot has no shareable sync remote.
    /// The prompt offering to attach one is up; unlike `Copied`/`Failed` this
    /// does not revert on a timer, because the user is being asked a question.
    Blocked,
```

```rust
            Self::Blocked => "blocked",
```

`accepts_click` and `is_transient` need no change: `Blocked` is neither `Copying` nor `Copied`/`Failed`, so it already accepts a click and already does not auto-revert. Do not add it to either.

- [ ] **Step 4: Add the three functions and the constant**

Beside `invite_claim_json`:

```rust
/// The `tonk:enable-sync` claim the share control dispatches when a user
/// accepts the offer to turn sync on.
///
/// `share` adds the marker asking the worker to mint an invite once the
/// remote is attached. When false the marker is omitted from BOTH the
/// declared concept and the parameters — a declared field with no value makes
/// the assert incomplete, so the transient would commit and match nothing.
pub fn enable_sync_claim_json(space: &str, remote: &str, share: bool, time: f64) -> Value {
    let mut with = json!({
        "time":   { "the": "dom.event/time-stamp", "as": "Float" },
        "space":  { "the": "xyz.tonk.enable-sync/space", "as": "Entity" },
        "remote": { "the": "xyz.tonk.enable-sync/remote", "as": "Text" },
        "marker": { "the": "dom.event.current-target.dataset/enable-sync", "as": "Entity" }
    });
    let mut parameters = json!({
        "time": time,
        "space": space,
        "remote": remote,
        "marker": "tonk:enable-sync"
    });
    if share {
        with["share"] = json!({ "the": "xyz.tonk.enable-sync/share", "as": "Entity" });
        parameters["share"] = json!("tonk:share");
    }
    json!({
        "claims": [{
            "op": "assert",
            "application": {
                "predicate": {
                    "kind": "transient",
                    "concept": {
                        "description": "Attach a sync remote to a spot, and share it.",
                        "with": with
                    }
                },
                "parameters": parameters
            }
        }]
    })
}
```

Beside `invite_link_query_body`:

```rust
/// The subscribe body for a refused share.
///
/// An INLINE predicate over the raw `xyz.tonk.share/*` attributes, for the
/// same reason [`invite_link_query_body`] is inline: rules and views are
/// frozen at whatever `core.yaml` seeded a spot with, so reading raw
/// attributes depends on nothing seeded and works on spots that predate this
/// feature. `this` binds to the spot's subject DID, the entity the worker
/// keys the refusal by.
pub fn share_blocked_query_body(subject: &str) -> Result<String, String> {
    if subject.is_empty() {
        return Err("share_blocked_query_body: empty subject".into());
    }
    Ok(json!({
        "predicate": { "with": {
            "blocked": { "the": "xyz.tonk.share/blocked", "as": "Text", "cardinality": "one" },
            "detail":  { "the": "xyz.tonk.share/detail",  "as": "Text", "cardinality": "one" },
            "time":    { "the": "xyz.tonk.share/time",    "as": "Float", "cardinality": "one" }
        } },
        "terms": {
            "this": subject,
            "blocked": { "?": { "name": "blocked" } },
            "detail":  { "?": { "name": "detail" } },
            "time":    { "?": { "name": "time" } }
        }
    })
    .to_string())
}

/// This page's default UCAN access-service endpoint: `origin + /ucan/`.
///
/// The same URL `<tonk-default-remote auto>` fills the create wizard's hidden
/// input with. Kept pure (origin in, URL out) so it is testable off-browser;
/// the caller supplies the origin.
pub fn default_remote_url(origin: &str) -> String {
    format!("{}{}", origin.trim_end_matches('/'), "/ucan/")
}

/// How long a share click waits for a result before giving up.
///
/// Without this the control has no failure path at all for anything other
/// than an explicit refusal: a mint that never lands leaves the clipboard
/// write open and the button pinned on `copying`, which
/// [`ShareState::accepts_click`] refuses, so the button is dead for the rest
/// of the session. Generous, because the enable-sync path holds the write
/// across a network round-trip.
pub const SHARE_TIMEOUT_MS: i32 = 15_000;
```

- [ ] **Step 5: Add the `Location` web-sys feature**

In `rust/tonk-fab/Cargo.toml`, add `"Location",` to the `web-sys` features list (alphabetically, after `"HtmlHeadElement"`). Task 9 reads `window().location().origin()`.

- [ ] **Step 6: Run tests to verify they pass**

Run: `nix develop -c cargo test -p tonk-fab --lib`
Expected: PASS

- [ ] **Step 7: Commit**

```bash
git add rust/tonk-fab/src/logic.rs rust/tonk-fab/Cargo.toml
git commit -m "feat(tonk-fab): add blocked-share state, claim and query builders"
```

---

### Task 9: `<tonk-share>` handles a refusal

**Files:**
- Modify: `rust/tonk-fab/src/share.rs`

**Interfaces:**
- Consumes: `Scaffold::connect_all` (Task 7); `ShareState::Blocked`, `enable_sync_claim_json`, `share_blocked_query_body`, `default_remote_url`, `SHARE_TIMEOUT_MS` (Task 8); the `#fab-enable-sync` dialog and its `[data-enable-sync-confirm]` button (Task 10, which may land after this — the lookups are `Option`-guarded, so this task's tests pass without it).

- [ ] **Step 1: Write the failing tests**

Add to `share.rs`'s existing wasm test mod:

```rust
/// A refusal whose timestamp matches the pending click abandons the copy and
/// moves the control to `blocked`.
#[dialog_common::test]
fn it_blocks_on_a_matching_refusal() {
    let host = test_host();
    let state = Rc::new(RefCell::new(ShareStateCell::default()));
    let pending_time = Rc::new(RefCell::new(Some(42.0)));
    open_clipboard_write(Rc::clone(&state), None).expect("clipboard write opens");
    set_state(&host, ShareState::Copying);

    handle_blocked(
        &host,
        &state,
        &pending_time,
        Blocked {
            code: "not-synced".to_owned(),
            detail: "This spot only exists on this device.".to_owned(),
            time: 42.0,
        },
    );

    assert_eq!(read_state(&host), ShareState::Blocked);
    assert!(
        state.borrow().pending.is_none(),
        "the clipboard write is abandoned, not left open"
    );
}

/// A refusal from an earlier click is a replay and must be ignored — the
/// fact is cardinality-one and redelivered on every resubscribe.
#[dialog_common::test]
fn it_ignores_a_refusal_from_an_earlier_click() {
    let host = test_host();
    let state = Rc::new(RefCell::new(ShareStateCell::default()));
    let pending_time = Rc::new(RefCell::new(Some(99.0)));
    open_clipboard_write(Rc::clone(&state), None).expect("clipboard write opens");
    set_state(&host, ShareState::Copying);

    handle_blocked(
        &host,
        &state,
        &pending_time,
        Blocked {
            code: "not-synced".to_owned(),
            detail: "stale".to_owned(),
            time: 42.0,
        },
    );

    assert_eq!(read_state(&host), ShareState::Copying);
    assert!(state.borrow().pending.is_some(), "copy still pending");
}

/// An unrepairable refusal fails outright rather than offering a prompt.
#[dialog_common::test]
fn it_fails_without_prompting_on_an_unshareable_remote() {
    let host = test_host();
    let state = Rc::new(RefCell::new(ShareStateCell::default()));
    let pending_time = Rc::new(RefCell::new(Some(42.0)));
    open_clipboard_write(Rc::clone(&state), None).expect("clipboard write opens");
    set_state(&host, ShareState::Copying);

    handle_blocked(
        &host,
        &state,
        &pending_time,
        Blocked {
            code: "unshareable-remote".to_owned(),
            detail: "This spot's sync server can't be shared.".to_owned(),
            time: 42.0,
        },
    );

    assert_eq!(read_state(&host), ShareState::Failed);
}
```

Add a `test_host()` helper to that mod if one is not already there:

```rust
/// A detached `<tonk-share>` host to drive the state-machine helpers against.
fn test_host() -> HtmlElement {
    window()
        .unwrap()
        .document()
        .unwrap()
        .create_element("tonk-share")
        .unwrap()
        .dyn_into()
        .unwrap()
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `nix develop -c test:web:debug`
Expected: FAIL — `cannot find function 'handle_blocked'`

- [ ] **Step 3: Add the refusal type and handler**

In `share.rs`, after `PendingCopy`:

```rust
/// A refusal delivered on the blocked subscription.
#[derive(Debug, Clone, PartialEq)]
struct Blocked {
    /// `not-synced` | `unshareable-remote` | `attach-failed`.
    code: String,
    /// The sentence to show.
    detail: String,
    /// The timestamp of the command this answers.
    time: f64,
}

/// The refusal class the enable-sync prompt can repair.
const BLOCKED_NOT_SYNCED: &str = "not-synced";

/// Subscription tag for the refusal query, distinct from [`SUB_TAG`] so the
/// scaffolding can tell the two subscriptions' frames apart.
const BLOCKED_TAG: &str = "tonk-share-blocked";
```

Extend `ShareStateCell` with the timeout handle:

```rust
    /// The `setTimeout` that fails a copy nothing ever answered. Armed when
    /// the click opens the write, cleared on every settle.
    timeout: Option<i32>,
```

Add the handler beside `handle_link`:

```rust
/// Act on a refusal, if it answers the click we are holding a clipboard write
/// for.
///
/// The refusal fact is cardinality-one on the spot's subject, so the overlay
/// keeps the last one and redelivers it on every resubscribe. Matching on the
/// echoed timestamp is what separates "this click was refused" from "here is
/// an old refusal again" — without it, one refused share would poison every
/// later one for the rest of the session.
fn handle_blocked(
    host: &HtmlElement,
    state: &Rc<RefCell<ShareStateCell>>,
    pending_time: &Rc<RefCell<Option<f64>>>,
    blocked: Blocked,
) {
    if *pending_time.borrow() != Some(blocked.time) {
        return;
    }
    if state.borrow().pending.is_none() {
        return;
    }
    pending_time.borrow_mut().take();

    if blocked.code != BLOCKED_NOT_SYNCED {
        settle(host, state, Err(&blocked.detail));
        return;
    }

    // Repairable: abandon the copy and ask. `settle` would stamp `Failed` and
    // arm the revert timer, which is wrong while a question is on screen.
    abandon(state, &blocked.detail);
    set_state(host, ShareState::Blocked);
    open_enable_sync_dialog(&blocked.detail);
}

/// Drop a pending clipboard write without moving the control's state. The
/// browser releases the write and leaves the existing clipboard alone.
fn abandon(state: &Rc<RefCell<ShareStateCell>>, reason: &str) {
    let mut cell = state.borrow_mut();
    if let Some(id) = cell.timeout.take() {
        clear_timeout(id);
    }
    if let Some(pending) = cell.pending.take() {
        let _ = pending
            .reject
            .call1(&JsValue::NULL, &JsValue::from_str(reason));
    }
}

/// Show the enable-sync prompt, filling in the reason. A no-op when the
/// dialog is absent, so the element still works in a host that does not
/// render it.
fn open_enable_sync_dialog(detail: &str) {
    let Some(document) = window().and_then(|w| w.document()) else {
        return;
    };
    let Some(dialog) = document.get_element_by_id("fab-enable-sync") else {
        return;
    };
    if let Ok(Some(slot)) = dialog.query_selector("[data-enable-sync-detail]") {
        slot.set_text_content(Some(detail));
    }
    let _ = Reflect::set(&dialog, &JsValue::from_str("open"), &JsValue::TRUE);
}
```

Add the timeout arming and clearing. In `settle`, before taking `pending`:

```rust
    if let Some(id) = state.borrow_mut().timeout.take() {
        clear_timeout(id);
    }
```

And a new function beside `arm_revert`:

```rust
/// Fail a copy that nothing ever answered, so the control never pins on
/// `copying` (which refuses further clicks) when a mint dies silently.
fn arm_timeout(host: &HtmlElement, state: &Rc<RefCell<ShareStateCell>>) {
    let mut cell = state.borrow_mut();
    if let Some(id) = cell.timeout.take() {
        clear_timeout(id);
    }
    let host = host.clone();
    let state_for_timer = Rc::clone(state);
    let expire = Closure::once_into_js(move || {
        state_for_timer.borrow_mut().timeout = None;
        settle(&host, &state_for_timer, Err("share: timed out"));
    });
    cell.timeout = Some(set_timeout(
        expire.unchecked_ref::<Function>(),
        SHARE_TIMEOUT_MS,
    ));
}
```

Import `SHARE_TIMEOUT_MS` from `crate::logic` alongside `COPIED_LINGER_MS`.

- [ ] **Step 4: Wire the second subscription**

Add `pending_time: Rc<RefCell<Option<f64>>>` to `TonkShare` and its `Default` impl.

Add the behaviour:

```rust
/// The refusal subscription's behaviour: the same routing context as the link
/// subscription, the raw `xyz.tonk.share/*` query, and acting on a refusal
/// that answers the click currently in flight.
struct ShareBlockedBehaviour {
    state: Rc<RefCell<ShareStateCell>>,
    pending_time: Rc<RefCell<Option<f64>>>,
}

impl subscribing::Subscribing for ShareBlockedBehaviour {
    fn query_body(&self, this: &HtmlElement) -> Result<String, String> {
        let space = this.get_attribute("space").unwrap_or_default();
        share_blocked_query_body(&space)
    }

    fn render_reset(&self, host: &HtmlElement, payload: &JsValue) {
        let rows = js_sys::Array::from(payload);
        if let Some(blocked) = read_blocked_row(&rows.get(0)) {
            handle_blocked(host, &self.state, &self.pending_time, blocked);
        }
    }

    fn render_update(&self, host: &HtmlElement, payload: &JsValue) {
        let asserted =
            Reflect::get(payload, &JsValue::from_str("asserted")).unwrap_or(JsValue::UNDEFINED);
        let rows = js_sys::Array::from(&asserted);
        if let Some(blocked) = read_blocked_row(&rows.get(rows.length().saturating_sub(1))) {
            handle_blocked(host, &self.state, &self.pending_time, blocked);
        }
    }

    fn tag(&self) -> &'static str {
        BLOCKED_TAG
    }
}

/// Read `conclusion.fields.{blocked,detail,time}` off a raw subscription row.
/// `None` for a missing row or any missing field — all three are asserted
/// together, so a partial row is not a refusal.
fn read_blocked_row(row: &JsValue) -> Option<Blocked> {
    if row.is_undefined() || row.is_null() {
        return None;
    }
    let fields = Reflect::get(row, &JsValue::from_str("fields")).ok()?;
    let code = Reflect::get(&fields, &JsValue::from_str("blocked"))
        .ok()
        .and_then(|v| v.as_string())
        .filter(|s| !s.is_empty())?;
    let detail = Reflect::get(&fields, &JsValue::from_str("detail"))
        .ok()
        .and_then(|v| v.as_string())?;
    let time = Reflect::get(&fields, &JsValue::from_str("time"))
        .ok()
        .and_then(|v| v.as_f64())?;
    Some(Blocked { code, detail, time })
}
```

In `connected_callback`, subscribe to both:

```rust
    fn connected_callback(&mut self, this: &HtmlElement) {
        let link: Rc<dyn subscribing::Subscribing> = Rc::new(ShareLinkBehaviour {
            state: Rc::clone(&self.state),
            current_link: Rc::clone(&self.current_link),
        });
        let blocked: Rc<dyn subscribing::Subscribing> = Rc::new(ShareBlockedBehaviour {
            state: Rc::clone(&self.state),
            pending_time: Rc::clone(&self.pending_time),
        });
        self.scaffold.connect_all(this, vec![link, blocked]);
    }
```

In the click handler in `inject_children`, capture the timestamp and arm the timeout:

```rust
            let time = js_sys::Date::now();
            *pending_time.borrow_mut() = Some(time);
            let stale = current_link.borrow().clone();
            match open_clipboard_write(Rc::clone(&state), stale) {
                Ok(()) => set_state(&host, ShareState::Copying),
                Err(e) => {
                    warn(&format!("share: clipboard unavailable: {e:?}"));
                    set_state(&host, ShareState::Copying);
                }
            }
            arm_timeout(&host, &state);
            dispatch_invite(&space, time);
```

Clone `self.pending_time` into the closure alongside `state` and `current_link`.

Also clear the timeout in `disconnected_callback`, beside the existing `revert` clear.

- [ ] **Step 5: Wire the confirm button**

Add a delegated document listener, installed in `connected_callback`, so it works regardless of when the dialog markup is parsed:

```rust
/// Listen for the enable-sync prompt's confirm, wherever it is in the
/// document.
///
/// Delegated rather than bound to the button: `<tonk-share>` and the dialog
/// are set as one `innerHTML` string, so a direct lookup at connect time can
/// race the dialog into existence.
///
/// The confirm click is a FRESH user activation, which is the whole reason
/// this can complete the copy: a new clipboard write opens here and the
/// browser holds it through the attach and the mint that follow.
fn install_confirm_listener(&mut self, this: &HtmlElement) {
    let Some(document) = window().and_then(|w| w.document()) else {
        return;
    };
    let host = this.clone();
    let state = Rc::clone(&self.state);
    let current_link = Rc::clone(&self.current_link);
    let pending_time = Rc::clone(&self.pending_time);
    let on_confirm = Closure::<dyn FnMut(web_sys::Event)>::new(move |event: web_sys::Event| {
        let Some(target) = event.target().and_then(|t| t.dyn_into::<Element>().ok()) else {
            return;
        };
        if target
            .closest("[data-enable-sync-confirm]")
            .ok()
            .flatten()
            .is_none()
        {
            return;
        }
        event.prevent_default();
        let Some(space) = host.get_attribute("space").filter(|s| !s.is_empty()) else {
            return;
        };
        let Some(origin) = page_origin() else {
            warn("share: cannot resolve this page's origin");
            return;
        };
        let remote = crate::logic::default_remote_url(&origin);

        let time = js_sys::Date::now();
        *pending_time.borrow_mut() = Some(time);
        let stale = current_link.borrow().clone();
        if let Err(e) = open_clipboard_write(Rc::clone(&state), stale) {
            warn(&format!("share: clipboard unavailable: {e:?}"));
        }
        set_state(&host, ShareState::Copying);
        arm_timeout(&host, &state);
        dispatch_enable_sync(&space, &remote, time);
        close_enable_sync_dialog();
    });
    let target: &web_sys::EventTarget = document.unchecked_ref();
    let _ = target.add_event_listener_with_callback("click", on_confirm.as_ref().unchecked_ref());
    self.listeners.push(("click".to_owned(), on_confirm));
}
```

`self.listeners` is removed from `this` on disconnect; this one is on `document`, so remove it from `document` too — either give `TonkShare` a second `Vec` for document-level listeners with its own teardown in `disconnected_callback`, or store the target alongside the closure. Do not `forget()` it: the element can be re-created.

Supporting functions:

```rust
/// The real page origin. Inside a sealed guest the document is
/// `about:srcdoc`, so `location.origin` is the opaque `"null"`; the portal
/// bridge injects the true origin at `window.tonk.context.origin`.
fn page_origin() -> Option<String> {
    let win = window()?;
    let context = Reflect::get(&win, &"tonk".into())
        .ok()
        .and_then(|tonk| Reflect::get(&tonk, &"context".into()).ok())
        .and_then(|context| Reflect::get(&context, &"origin".into()).ok())
        .and_then(|origin| origin.as_string())
        .filter(|s| !s.is_empty() && s != "null");
    if context.is_some() {
        return context;
    }
    let origin = win.location().origin().ok()?;
    (origin != "null" && !origin.is_empty()).then_some(origin)
}

/// Dispatch the `tonk:enable-sync` claim via `window.tonk.transact`,
/// routeless — same path as [`dispatch_invite`].
fn dispatch_enable_sync(space: &str, remote: &str, time: f64) {
    dispatch_claim(&crate::logic::enable_sync_claim_json(
        space, remote, true, time,
    ));
}

fn close_enable_sync_dialog() {
    let Some(document) = window().and_then(|w| w.document()) else {
        return;
    };
    let Some(dialog) = document.get_element_by_id("fab-enable-sync") else {
        return;
    };
    let _ = Reflect::set(&dialog, &JsValue::from_str("open"), &JsValue::FALSE);
}
```

Refactor `dispatch_invite`'s body into a shared `dispatch_claim(claim: &serde_json::Value)` (the `window.tonk.transact` lookup and call, lines 272-291) and have both callers use it. `dispatch_invite` becomes:

```rust
fn dispatch_invite(space: &str, time: f64) {
    dispatch_claim(&invite_claim_json(space, time));
}
```

Call `self.install_confirm_listener(this)` from `connected_callback`.

- [ ] **Step 6: Run tests to verify they pass**

Run: `nix develop -c test:web:debug`
Expected: PASS

- [ ] **Step 7: Commit**

```bash
git add rust/tonk-fab/src/share.rs
git commit -m "feat(tonk-fab): prompt to turn on sync when a share is refused"
```

---

### Task 10: the prompt markup

**Files:**
- Modify: `rust/tonk-fab/src/markup.rs:130-186` (add a second dialog after `#fab-space-create`)
- Modify: `rust/tonk-fab/src/fab.css`

**Interfaces:**
- Consumes: the ids and data attributes Task 9 looks up — `#fab-enable-sync`, `[data-enable-sync-detail]`, `[data-enable-sync-confirm]`.

- [ ] **Step 1: Write the failing test**

In `markup.rs`'s existing `mod tests`:

```rust
#[test]
fn it_renders_the_enable_sync_prompt() {
    let html = fab_html("did:key:z6Mk");
    assert!(html.contains(r#"id="fab-enable-sync""#));
    assert!(html.contains("data-enable-sync-detail"));
    assert!(html.contains("data-enable-sync-confirm"));
    assert!(html.contains("Turn on sync &amp; copy link"));
    assert!(html.contains("Not now"));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `nix develop -c cargo test -p tonk-fab --lib markup`
Expected: FAIL — assertion failed on `id="fab-enable-sync"`

- [ ] **Step 3: Add the dialog**

In `markup.rs`, immediately after the closing `</wa-dialog>` of `#fab-space-create` (line 186) and before the closing `"#` of the format string:

```html
<wa-dialog id="fab-enable-sync" label="Turn on sync?" class="fab__dialog" style="--width: 28rem">
  <p class="fab__prompt" data-enable-sync-detail>This spot only exists on this device.</p>
  <p class="fab__prompt">Turn on sync so the people you share with can open it.</p>
  <wa-button slot="footer" variant="primary" data-enable-sync-confirm>Turn on sync &amp; copy link</wa-button>
  <wa-button slot="footer" variant="neutral" appearance="plain" data-dialog="close">Not now</wa-button>
</wa-dialog>
```

The `data-enable-sync-detail` paragraph carries a default so the dialog reads correctly even if it is ever opened before a refusal has filled it in; Task 9's `open_enable_sync_dialog` overwrites it with the worker's sentence.

- [ ] **Step 4: Style it**

In `rust/tonk-fab/src/fab.css`, beside the existing `.fab__dialog` rules:

```css
.fab__prompt {
  margin: 0 0 0.75rem;
  line-height: 1.5;
}

.fab__prompt:last-of-type {
  margin-bottom: 0;
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `nix develop -c cargo test -p tonk-fab --lib markup`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add rust/tonk-fab/src/markup.rs rust/tonk-fab/src/fab.css
git commit -m "feat(tonk-fab): add the turn-on-sync prompt dialog"
```

---

### Task 11: gate and manual verification

**Files:** none — verification only.

- [ ] **Step 1: Run the full native suite**

Run: `nix develop -c test:native:debug`
Expected: PASS

- [ ] **Step 2: Run the full wasm suite**

Run: `nix develop -c test:web:debug`
Expected: PASS

- [ ] **Step 3: Run the lint gate**

Run: `nix develop -c cargo clippy --workspace --all-targets --all-features -- -D warnings && nix develop -c cargo fmt --all -- --check`
Expected: no output, exit 0.

`--all-features` matters: it compiles the integration tests, so a per-crate clippy can be green while this fails.

- [ ] **Step 4: Drive it in a browser**

Build and serve the UI, then:

1. Create a spot with the remote field blanked (edit the hidden `remote` input to `""` in devtools before submitting, so `<tonk-default-remote auto>` does not fill it).
2. Open the spot, click share.
3. Expect: the button does not stick on `copying`, the prompt appears reading "This spot only exists on this device."
4. Click "Turn on sync & copy link".
5. Expect: the dialog closes, the button goes to `copying` then `copied`, and the clipboard holds a URL containing `&remote=`.
6. Paste that URL into a second browser profile and confirm the spot loads with content rather than "Model not found".

Step 6 is the actual bug this whole change exists to prevent, so do not skip it.

- [ ] **Step 5: Open the PR**

```bash
git push -u origin feat/share-enable-sync-prompt
gh pr create --base staging --title "feat(tonk-fab): refuse remote-less invites and offer to turn on sync" --body "$(cat <<'EOF'
Sharing a spot whose `main` has no remote upstream minted an invite that could
never sync: the recipient claimed it, the join pull failed with
`BranchHasNoUpstream`, and they landed in a permanently empty space rendering
"Model not found".

The mint now resolves the remote before generating any key material and refuses
when it cannot. When the spot simply has no upstream, the share button offers to
attach one; confirming attaches, mints, and copies the link in a single click.

Also fixes a related gap: `<tonk-share>` had no failure path at all, so any mint
that never landed pinned the button on `copying` for the rest of the session.

Spec: `docs/superpowers/specs/2026-07-22-share-enable-sync-prompt-design.md`

Not fixed here: why the create path's remote attach fails or is skipped in the
first place, and the dead `space/enable-sync` command whose only handler mints a
new spot instead of attaching to the existing one.

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```
