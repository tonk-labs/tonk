# Remove Spot Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let a user remove a spot (space) from the Hub launcher: retract its replica record, stop syncing it, and delete its local storage.

**Architecture:** A new transient `space/remove` command in `profile.yaml` (fired by a per-row hover-`×` + confirm overlay, script-free CSS like the create wizard) decodes into a new `tonk_schema::command::RemoveSpace`, handled by a `RemoveSpaceHandler` in tonk-worker that (1) retracts every fact on the replica entity from the profile meta branch, (2) evicts the repo from the reactor cache via a new `Reactor::evict`, (3) deletes the space's IndexedDB database + OPFS blob dir via inline JS. Spec: `docs/superpowers/specs/2026-07-06-remove-spot-design.md`.

**Tech Stack:** Rust (wasm32 service worker), dialog-db concepts, tonk notation (yaml), wasm-bindgen inline_js.

## Global Constraints

- Work happens in the worktree `/Users/jackdouglas/tonk/tonk/.claude/worktrees/remove-spot` on branch `remove-spot` (tracking `origin/staging`). Run all commands from that directory.
- Use plain `git` for commits in this worktree. Do NOT run `jj` here — the repo is jj-colocated and any `jj` command in this directory operates on the MAIN workspace, not this worktree.
- Commit messages: Conventional Commits (`type(scope): subject`), imperative, lowercase, no trailing period, no emojis. End the body with `Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>`.
- Lint gate (matches CI): `cargo clippy --all-targets --all-features -- -D warnings` must pass natively. wasm-gated code needs `#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]`; helpers called only from wasm-gated code must carry the same gate or clippy flags them dead.
- Tests use `#[dialog_common::test]` (or plain `#[test]` for the pure-native library-lowering suite), named `it_does_x`.
- No `mod.rs` files.
- This repo's doc-comment culture is heavy and load-bearing: every new pub(crate)+ item gets a doc comment explaining constraints and cross-references, in the style of the neighboring code. The doc comments in this plan's code blocks are part of the deliverable.
- The critical safety invariant: the command must NOT read `dom.event.current-target.dataset/subject`. The `tonk/rename-repository` transient (core.yaml) already carries that attribute, and command decode ignores extra facts — a remove command matched on `dataset/subject` alone would also decode every rename and delete the space being renamed. The command reads `dataset/remove` instead. Task 1 encodes this as a regression test; do not "simplify" it away.

---

### Task 1: `RemoveSpace` command in tonk-schema + decode tests

**Files:**
- Modify: `rust/tonk-schema/src/domain.rs` (add `command::remove` module, after the `rename` module which ends near line 315)
- Modify: `rust/tonk-schema/src/command.rs` (add `RemoveSpace` after `ProfileRename`, near line 198)
- Test: `rust/dialog-reactor/src/command.rs` (tests module at bottom, alongside `it_decodes_create_space_from_name_only_facts` at line 491)

**Interfaces:**
- Produces: `tonk_schema::domain::command::remove::Remove(pub Entity)` with derived attribute `dom.event.current-target.dataset/remove`; `tonk_schema::command::RemoveSpace { this: Entity, subject: remove::Remove }` implementing `dialog_capability::Command`. Task 3's handler decodes this type; Task 4's yaml asserts the matching fact shape.

- [ ] **Step 1: Write the failing decode tests**

In `rust/dialog-reactor/src/command.rs`, inside the existing `mod tests`, after `it_decodes_create_space_from_name_only_facts` (the helpers `entity`, `facts_for`, and the `the!` macro are already in scope there):

```rust
    /// The Hub's per-row delete confirm asserts a `space/remove`
    /// transient carrying only `data-remove` (the subject DID).
    #[dialog_common::test]
    fn it_decodes_remove_space_from_a_data_remove_fact() {
        use tonk_schema::command::RemoveSpace;

        let this = entity("did:key:zRemoveSpace");
        let subject = entity("did:key:zSpaceSubject");
        let mut changes = Changes::new();
        the!("dom.event.current-target.dataset/remove")
            .of(this.clone())
            .is(subject.clone())
            .assert(&mut changes);
        let (this, facts) = facts_for(changes);

        let decoded = RemoveSpace::decode(this, &facts)
            .expect("RemoveSpace must decode from a data-remove-only transient");
        assert_eq!(decoded.subject.0, subject);
    }

    /// Regression: a `tonk/rename-repository` transient carries
    /// `dataset/subject` plus the new name. It must NOT decode as
    /// `RemoveSpace` — a remove command keyed on `dataset/subject`
    /// would turn every banner rename into a space deletion.
    #[dialog_common::test]
    fn it_does_not_decode_a_rename_transient_as_remove_space() {
        use tonk_schema::command::RemoveSpace;

        let mut changes = Changes::new();
        the!("dom.event.current-target.dataset/subject")
            .of(entity("did:key:zRename"))
            .is(entity("did:key:zSpaceSubject"))
            .assert(&mut changes);
        the!("dom.event.current-target/value")
            .of(entity("did:key:zRename"))
            .is("new name".to_string())
            .assert(&mut changes);
        let (this, facts) = facts_for(changes);

        assert!(
            RemoveSpace::decode(this, &facts).is_none(),
            "a rename-shaped transient must not decode as RemoveSpace"
        );
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p dialog-reactor remove_space`
Expected: FAIL to compile with `cannot find ... RemoveSpace` (a compile error on the not-yet-existing type is the red state).

- [ ] **Step 3: Add the attribute module**

In `rust/tonk-schema/src/domain.rs`, inside `pub mod command`, after the closing brace of `pub mod rename` (~line 315):

```rust
    /// Attributes the `space/remove` command reads from its submit event.
    pub mod remove {
        use super::super::Entity;
        use super::Attribute;

        /// The subject DID of the space to remove, read from the Hub
        /// confirm form's `data-remove` attribute. Deliberately NOT
        /// `dataset/subject`: the declarative `tonk/rename-repository`
        /// transient (core.yaml) already carries `dataset/subject`, and
        /// a remove command matched on `subject` alone would ALSO decode
        /// every rename — deleting the space being renamed. The
        /// distinctly named attribute is both the payload and the
        /// command's unique shape, so no separate marker field is
        /// needed. An `Entity` because the value (a did:key) carries a
        /// `:` — see the invite marker note. Derived attribute:
        /// `dom.event.current-target.dataset/remove`.
        #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
        #[domain("dom.event.current-target.dataset")]
        pub struct Remove(pub Entity);
    }
```

- [ ] **Step 4: Add the command struct**

In `rust/tonk-schema/src/command.rs`, after the `impl Command for ProfileRename` block (~line 198):

```rust
/// Request to remove a space from this device: retract its replica
/// record from the profile meta branch (the Hub row's source of
/// truth), detach it from the reactor/sync, and delete its local
/// storage.
///
/// Asserted transiently when the user confirms a Hub row's delete
/// overlay (`<form onsubmit=space/remove data-remove={subject}>` in
/// `profile.yaml`). Removal is device-local: a synced space can be
/// rejoined via an invite link; server-side data is untouched.
///
/// Deliberately a single matched field, like [`CreateSpace`], so an
/// older profile descriptor keeps decoding it. The field also doubles
/// as the command's distinct shape: `dataset/remove` is read by no
/// other command, whereas a `dataset/subject` field would also match
/// every `tonk/rename-repository` transient (which carries
/// `dataset/subject`) and turn each rename into a deletion — see
/// [`crate::domain::command::remove::Remove`].
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct RemoveSpace {
    /// The command entity (a fresh id per invocation).
    pub this: Entity,
    /// The subject DID of the space to remove, from `data-remove`.
    pub subject: crate::domain::command::remove::Remove,
}

/// `RemoveSpace` is a [`dialog_capability::Command`]; the worker
/// registers a custom `RemoveSpaceHandler` (the work needs the profile
/// handle, the reactor cache, and storage — state the decoded command
/// doesn't carry).
impl Command for RemoveSpace {
    type Input = Self;
    type Output = ();
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p dialog-reactor remove_space`
Expected: PASS (2 tests).

- [ ] **Step 6: Commit**

```bash
git add rust/tonk-schema/src/domain.rs rust/tonk-schema/src/command.rs rust/dialog-reactor/src/command.rs
git commit -m "feat(schema): RemoveSpace command decoded from data-remove

The attribute is deliberately dataset/remove, not dataset/subject:
tonk/rename-repository transients carry dataset/subject and would
also decode as a subject-keyed remove command.

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 2: `Reactor::evict` in dialog-reactor

**Files:**
- Modify: `rust/dialog-reactor/src/lib.rs` (add method on `impl Reactor`, directly after `shutdown` which ends at ~line 166)

**Interfaces:**
- Consumes: existing `Reactor.repos: RwLock<HashMap<String, Arc<RepositoryState>>>`, `RepositoryState::branches()`, `BranchState::clear_subscribers()` (all already used by `shutdown`).
- Produces: `pub fn evict(&self, name: &str)` — Task 3's handler calls `tonk.reactor.evict(key)`.

Why no unit test here: dialog-reactor has no native test infrastructure that can construct a `RepositoryState` (its only tests are pure decode tests in `command.rs`; building repo state needs a running storage stack, which in this workspace exists only in tonk-worker's wasm test harness). `evict` is exercised end-to-end by Task 3's wasm test, which asserts the cache entry is gone after removal.

- [ ] **Step 1: Add the method**

In `rust/dialog-reactor/src/lib.rs`, after the closing brace of `pub fn shutdown` (~line 166):

```rust
    /// Drop one repository's cached handles and active subscribers —
    /// the per-repo analog of [`shutdown`](Self::shutdown). Used when a
    /// space is removed: the background sync sweep builds its repo set
    /// from this cache, so eviction is what actually stops the space
    /// from syncing, and clearing each branch's subscriber map ends its
    /// SSE streams (see `shutdown` for why removing the cache entry
    /// alone isn't enough). No-op when the repo isn't cached.
    pub fn evict(&self, name: &str) {
        let Some(repo) = self.repos.write().remove(name) else {
            return;
        };
        let branches = {
            let mut map = repo.branches().write();
            std::mem::take(&mut *map)
        };
        for (_, branch) in branches {
            branch.clear_subscribers();
        }
    }
```

- [ ] **Step 2: Verify it compiles clean**

Run: `cargo clippy -p dialog-reactor --all-targets --all-features -- -D warnings`
Expected: PASS. (`evict` is `pub` on a `pub` type, so it is not dead code natively.)

- [ ] **Step 3: Commit**

```bash
git add rust/dialog-reactor/src/lib.rs
git commit -m "feat(reactor): per-repo evict dropping cached handles and subscribers

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 3: `RemoveSpaceHandler` in tonk-worker + wasm tests

**Files:**
- Modify: `rust/tonk-worker/src/router/repository.rs` (handler + helpers after `ProfileRenameHandler`'s `run` impl ends ~line 880; wasm tests in the existing `mod tests` at line 2929)
- Modify: `rust/tonk-worker/src/router/command.rs` (register handler in `command_registry`, line 115-120)

**Interfaces:**
- Consumes: `tonk_schema::command::RemoveSpace` (Task 1), `Reactor::evict` (Task 2), existing `Replica::new`, `META_BRANCH`, `broadcast`/`Notification`, `super::claim::RawClaim { the, of, is, unique }` (pub(crate) in `router/claim.rs`, a sibling module — path `super::claim::RawClaim` from `router/repository.rs`), `tonk_schema::prelude::DidExt::repo_key` (already imported in repository.rs — `subject.repo_key()` is used by `create_space_inner`).
- Produces: `RemoveSpaceHandler` (registered), `remove_space_inner(state: &AppState, subject: &Did) -> Result<(), RepositoryError>` (called by the handler and by the wasm tests).

Checked assumptions (verify while implementing; both are read from current code, not guessed):
- `tonk.reactor.profile_repository().branch(META_BRANCH).acquire(&tonk.operator).await` returns a state whose `.handle()` is a `dialog_repository::Branch` — the migration (`router/migration.rs:77-102`) calls `.handle().query()` on it, and `Branch` also carries `.claims()` (used on a `Branch` in `router/claim.rs:432`). If `.handle()` is not a `&Branch`, follow whatever type migration.rs's `meta.handle()` actually is — it must expose `claims()` the same way it exposes `query()`.
- `dialog_varsig::Did` parses from its string form (`"did:key:z...".parse::<Did>()`). If `FromStr` is missing, use the conversion the join handler uses to turn an invite's did string into a `Did` (see `router/join.rs`).

- [ ] **Step 1: Write the failing wasm tests**

In `rust/tonk-worker/src/router/repository.rs`, inside the existing wasm `mod tests` (line 2929+), after `it_reports_the_founder_in_members`:

```rust
    /// All `Replica` rows on the profile meta branch (any kind), read
    /// through the reactor's cached profile handle — the same handle
    /// the Hub and the removal path use.
    async fn profile_replicas(state: &AppState) -> Vec<tonk_schema::Replica> {
        use dialog_query::{Output as _, Query, Term};
        let tonk = state.read().await;
        let meta = tonk
            .reactor
            .profile_repository()
            .branch(super::META_BRANCH)
            .acquire(&tonk.operator)
            .await
            .expect("profile meta acquires");
        meta.handle()
            .query()
            .select(Query::<tonk_schema::Replica> {
                this: Term::var("this"),
                subject: Term::var("subject"),
                profile: Term::var("profile"),
                kind: Term::var("kind"),
            })
            .perform(&tonk.operator)
            .try_vec()
            .await
            .expect("replica query")
    }

    /// Removing a space retracts its replica record from the profile
    /// meta branch and evicts the repo from the reactor cache (which is
    /// what drops it from the background sync sweep).
    #[dialog_common::test]
    async fn it_removes_a_space_from_the_profile_index() {
        let (_app, state, key) = fresh_repo("test-remove-space").await;

        let subject: dialog_varsig::Did = {
            let tonk = state.read().await;
            use dialog_repository::RepositoryExt as _;
            let repository: dialog_repository::Repository = tonk
                .profile
                .repository(&key)
                .load()
                .perform(&tonk.operator)
                .await
                .expect("repo loads");
            repository.did()
        };
        let recorded = profile_replicas(&state).await;
        assert!(
            recorded.iter().any(|r| r.subject.0 == subject.this()),
            "the fresh repo must be recorded before removal"
        );

        super::remove_space_inner(&state, &subject)
            .await
            .expect("remove succeeds");

        let remaining = profile_replicas(&state).await;
        assert!(
            !remaining.iter().any(|r| r.subject.0 == subject.this()),
            "the replica record must be gone after removal"
        );
        {
            let tonk = state.read().await;
            assert!(
                !tonk.reactor.repos().read().contains_key(&key),
                "the repo must be evicted from the reactor cache"
            );
        }
    }

    /// The self-replica (subject == profile) is refused: deleting the
    /// profile's own storage would take every space with it.
    #[dialog_common::test]
    async fn it_refuses_to_remove_the_self_replica() {
        let (_app, state, _key) = fresh_repo("test-remove-self").await;

        let profile_did = {
            let tonk = state.read().await;
            tonk.profile.did()
        };
        super::remove_space_inner(&state, &profile_did)
            .await
            .expect_err("removing the self-replica must fail");

        let remaining = profile_replicas(&state).await;
        assert!(
            remaining
                .iter()
                .any(|r| r.subject.0 == profile_did.this()),
            "the self-replica record must survive"
        );
    }
```

Notes for the implementer:
- `Replica.subject` is `tonk_schema::domain::replica::Subject(pub Entity)` (a wrapped entity) and `Did::this()` yields the DID's entity — this pairing (`r.subject.0 == did.this()`) is exactly how `migration.rs:159` compares (`replica.subject.0 == profile_entity`). If field/method names differ, mirror migration.rs.
- If `dialog_query::Query::<tonk_schema::Replica>` needs different import spelling, copy the exact imports from `router/migration.rs` (it runs this very query).

- [ ] **Step 2: Verify the tests fail to compile**

Run: `cargo check -p tonk-worker --target wasm32-unknown-unknown --tests`
Expected: FAIL with `cannot find function remove_space_inner`.
(This check is the red state; actually *running* wasm tests happens in Step 6.)

- [ ] **Step 3: Implement the removal path**

In `rust/tonk-worker/src/router/repository.rs`, after `ProfileRenameHandler`'s `CommandHandler` impl (~line 880):

```rust
/// Post-commit handler for the [`RemoveSpace`] command.
///
/// Fired when the user confirms a Hub row's delete overlay. Removal is
/// device-local and ordered so the visible state commits first and
/// cleanup is best-effort behind it — see [`remove_space_inner`].
///
/// A custom handler (not a plain `Provider<RemoveSpace>`) for the same
/// reason as [`CreateSpaceHandler`]: the work needs the profile handle,
/// the reactor cache, and storage, reached through state rather than
/// carried by the decoded command.
///
/// [`RemoveSpace`]: tonk_schema::command::RemoveSpace
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) struct RemoveSpaceHandler {
    attributes: Vec<String>,
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
impl RemoveSpaceHandler {
    /// Cache `RemoveSpace`'s trigger attributes (its `subject` field) so
    /// the registry indexes this handler under them.
    pub(crate) fn new() -> Self {
        use crate::reactor::Decode as _;
        Self {
            attributes: tonk_schema::command::RemoveSpace::trigger_attributes(),
        }
    }
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
impl crate::reactor::CommandHandler<crate::router::CommandEnv> for RemoveSpaceHandler {
    fn trigger_attributes(&self) -> &[String] {
        &self.attributes
    }

    fn matches(&self, facts: &crate::reactor::EntityFacts) -> bool {
        use crate::reactor::Decode as _;
        facts
            .first()
            .map(|artifact| artifact.of.clone())
            .and_then(|this| tonk_schema::command::RemoveSpace::decode(this, facts))
            .is_some()
    }

    fn run(
        &self,
        facts: &crate::reactor::EntityFacts,
        env: &crate::router::CommandEnv,
    ) -> crate::reactor::RunFuture {
        use crate::reactor::Decode as _;

        // Decode synchronously (the caller still holds the lock), then
        // hand the owned subject + an env clone to the `'static` future.
        let subject = facts
            .first()
            .map(|artifact| artifact.of.clone())
            .and_then(|entity| tonk_schema::command::RemoveSpace::decode(entity, facts))
            .map(|command| command.subject.0);
        let env = env.clone();

        Box::pin(async move {
            let Some(subject) = subject else {
                return;
            };
            log!("command RemoveSpace subject={}", subject);
            let subject: Did = match subject.to_string().parse() {
                Ok(did) => did,
                Err(error) => {
                    log!("RemoveSpace: '{}' is not a DID: {}", subject, error);
                    return;
                }
            };
            if let Err(error) = remove_space_inner(env.state(), &subject).await {
                log!("RemoveSpace '{}' failed: {}", subject, error);
            }
        })
    }
}

/// Remove a space device-locally, in three ordered steps:
///
/// 1. Retract its replica record from the profile meta branch
///    ([`remove_replica_from_profile`]) — the Hub row's source of
///    truth, so the spot disappears immediately. This is the commit
///    point; everything after is cleanup.
/// 2. Evict the repository from the reactor cache
///    ([`Reactor::evict`](crate::Reactor::evict)). The background sync
///    sweep builds its repo set from that cache (NOT from replica
///    records), so eviction is what actually stops syncing; it also
///    ends the removed row's SSE streams.
/// 3. Delete local storage ([`delete_space_storage`]) — best-effort
///    and outside the state lock; a failure only orphans invisible
///    bytes, so it is logged, never surfaced.
///
/// The self-replica (subject == profile) is refused: its row is hidden
/// chrome in the Hub, and deleting the profile's own storage would take
/// every space with it.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn remove_space_inner(state: &AppState, subject: &Did) -> Result<(), RepositoryError> {
    {
        let tonk = state.write().await;
        if *subject == tonk.profile.did() {
            return Err(RepositoryError::Internal(
                "refusing to remove the profile's self-replica".to_string(),
            ));
        }
        remove_replica_from_profile(&tonk, subject).await?;
        // Drain the poll the retraction scheduled so the Hub's meta
        // subscription reflects the removal (mirrors set_replica_status).
        tonk.reactor.run_scheduled_polls(&tonk.operator).await;
        tonk.reactor.evict(subject.repo_key());
    }
    // Storage cleanup after the lock is released — the delete awaits
    // browser IO and must not stall other requests.
    let _ = wasm_bindgen_futures::JsFuture::from(delete_space_storage(subject.repo_key())).await;
    Ok(())
}

/// Retract every fact keyed on `subject`'s replica entity from the
/// profile repository's meta branch — the reverse of
/// [`record_replica_in_profile`]. Selecting the entity's actual claims
/// (rather than re-asserting typed concepts to retract) sweeps every
/// stamp regardless of vintage — the `Replica` fields, `SpaceStatus`,
/// a migration's `SpaceKind`, a legacy `name` — without knowing their
/// current values.
///
/// Reads and writes through the reactor's cached profile handle for the
/// same reason `record_replica_in_profile` does: the Hub reads through
/// that handle, so a commit on a separate handle would be invisible to
/// it. Broadcasts `/api/profile` like the record path.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn remove_replica_from_profile(
    tonk: &TonkState,
    subject: &Did,
) -> Result<(), RepositoryError> {
    use dialog_artifacts::ArtifactSelector;
    use futures_util::StreamExt as _;

    let entity = Replica::new(tonk.profile.did(), subject.clone())
        .this()
        .clone();

    let meta = tonk
        .reactor
        .profile_repository()
        .branch(META_BRANCH)
        .acquire(&tonk.operator)
        .await
        .map_err(|e| RepositoryError::Internal(format!("open profile meta: {e}")))?;
    let stream = meta
        .handle()
        .claims()
        .select(ArtifactSelector::new().of(entity.clone()))
        .perform(&tonk.operator)
        .await
        .map_err(|e| RepositoryError::Internal(format!("select replica claims: {e}")))?;
    tokio::pin!(stream);

    let mut transaction = tonk
        .reactor
        .profile_repository()
        .branch(META_BRANCH)
        .transaction();
    let mut found = false;
    while let Some(artifact) = stream.next().await {
        let artifact = artifact
            .map_err(|e| RepositoryError::Internal(format!("read replica claim: {e}")))?;
        found = true;
        transaction = transaction.retract(super::claim::RawClaim {
            the: artifact.the,
            of: artifact.of,
            is: artifact.is,
            unique: false,
        });
    }
    if !found {
        // Nothing recorded — a stale row or a repeated submit. Not an
        // error: the desired end state (no record) already holds.
        log!("remove replica: no facts for {} in profile meta", entity);
        return Ok(());
    }

    let revision = transaction
        .commit()
        .perform(&tonk.operator)
        .await
        .map_err(|e| RepositoryError::Internal(format!("retract replica record: {e}")))?;

    broadcast(
        "/api/profile",
        &Notification {
            branch: META_BRANCH.to_string(),
            revision,
        },
    );
    Ok(())
}

/// Delete a space's local storage: its IndexedDB database (archive,
/// memory, credential, certificate object stores) and its OPFS blob
/// subtree. The names are the storage loader's `Directory::Current`
/// mapping for a repository space (see dialog-storage's IndexedDb and
/// FileSystem providers): the database is named exactly the routing
/// key, the blobs live under `current/<key>`.
///
/// Inline JS rather than web-sys: `deleteDatabase` and recursive
/// `removeEntry` have no plumbing here, and the whole operation is two
/// promise chains. Never rejects — each half settles on error/absence.
/// `onblocked` also resolves: the worker's own pooled connection closes
/// itself on the `versionchange` the delete fires (see
/// [`crate::patch_idb_versionchange`]), after which the browser
/// completes the delete; waiting for the completion event would hang if
/// another tab pins the database open.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
#[wasm_bindgen::prelude::wasm_bindgen(inline_js = r#"
export function delete_space_storage(name) {
    const database = new Promise((resolve) => {
        const request = indexedDB.deleteDatabase(name);
        request.onsuccess = request.onerror = request.onblocked = () => resolve();
    });
    const blobs = navigator.storage.getDirectory()
        .then((root) => root.getDirectoryHandle('current'))
        .then((dir) => dir.removeEntry(name, { recursive: true }))
        .catch(() => {});
    return Promise.all([database, blobs]);
}
"#)]
extern "C" {
    /// Delete the IndexedDB database and OPFS blob directory for a
    /// space's routing key. Resolves once both halves settle; never
    /// rejects.
    fn delete_space_storage(name: &str) -> js_sys::Promise;
}
```

Implementation notes:
- `Did` needs importing if not already in scope at that point in the file (`use dialog_varsig::Did;` — check the file head; `repository.did()` returns one so it may already be there).
- If `js_sys` is not a direct dependency of tonk-worker, add `js-sys = { workspace = true }` to `rust/tonk-worker/Cargo.toml` (workspace root already pins it — verify with `grep js-sys Cargo.toml`; if the workspace lacks it, add `js-sys = "0.3"` to `[workspace.dependencies]`).
- `wasm_bindgen_futures` is already a tonk-worker dependency (`spawn_local` in `spawn_seed`).

- [ ] **Step 4: Register the handler**

In `rust/tonk-worker/src/router/command.rs`, in `command_registry()` after the `CreateSpaceHandler` registration (line 115):

```rust
        registry.register(Box::new(super::repository::RemoveSpaceHandler::new()));
```

And extend the function's doc comment list with one line noting the removal handler, e.g. after the paragraph about `CreateSpaceHandler`:

```rust
/// [`RemoveSpaceHandler`] serves the Hub's per-row delete confirm
/// (`space/remove`): replica retraction, reactor eviction, storage
/// cleanup.
```

(with the corresponding `/// [`RemoveSpaceHandler`]: super::repository::RemoveSpaceHandler` link line next to the existing `CreateSpaceHandler` link.)

- [ ] **Step 5: Native gates**

Run: `cargo clippy -p tonk-worker --all-targets --all-features -- -D warnings && cargo check -p tonk-worker --target wasm32-unknown-unknown --tests`
Expected: both PASS. Clippy runs the native side (handler code is wasm-gated so it must not leak native-dead helpers); the wasm check compiles the handler and tests.

- [ ] **Step 6: Run the wasm tests (or hand off to CI)**

The repo's `.cargo/config.toml` already sets `runner = "wasm-bindgen-test-runner"` for the wasm target, so the invocation is a plain cargo test — but on this darwin machine it needs a Chrome at the default `/Applications` path plus a major-version-matched `chromedriver` on PATH (or `CHROMEDRIVER=` env). If available:

Run: `CHROMEDRIVER=$(command -v chromedriver) cargo test -p tonk-worker --target wasm32-unknown-unknown remove_`
Expected: `it_removes_a_space_from_the_profile_index` and `it_refuses_to_remove_the_self_replica` PASS.

If no local browser automation is available, state that explicitly in the task report and rely on CI's web test leg (`.github/workflows/test.yml` matrix `platform: web`) — do NOT claim the tests ran.

- [ ] **Step 7: Commit**

```bash
git add rust/tonk-worker/src/router/repository.rs rust/tonk-worker/src/router/command.rs rust/tonk-worker/Cargo.toml Cargo.lock
git commit -m "feat(worker): RemoveSpace handler - retract replica, evict, delete storage

Retraction through the reactor's profile handle is the commit point;
eviction is what drops the repo from the background sync sweep (it
iterates the reactor cache, not replica records); storage deletion is
best-effort inline JS (IDB database <key> + OPFS current/<key>).

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

(Drop `Cargo.toml`/`Cargo.lock` from the add if no dependency change was needed.)

---

### Task 4: Hub UI — command, row affordance, confirm overlay in profile.yaml

**Files:**
- Modify: `rust/tonk-core/assets/library/profile.yaml` (command block after `&space/create` ends ~line 106; row markup at lines 399-418; CSS in the `<style>` block, rows section lines 217-239)
- Test: `rust/tonk-worker/tests/standard_library.rs` (existing `it_lowers_the_profile_library` — no new test code, it validates the edited document)

**Interfaces:**
- Consumes: the `space/remove` fact shape from Task 1 (`dom.event.current-target.dataset/remove` + `dom.event.do/prevent-default`), the existing `data-close-radio` delegate behavior (`rust/tonk-display/src/events/delegate.rs:143` — on submit, checks the radio with that id).
- Produces: the user-facing affordance. No other task consumes it.

- [ ] **Step 1: Add the command block**

In `rust/tonk-core/assets/library/profile.yaml`, after the `command!: &space/create` block (ends ~line 106, before `command!: &tonk/load`):

```yaml
# The Remove Space command. Submitting a Hub row's delete-confirm form
# asserts this transient; the worker's `RemoveSpaceHandler` retracts the
# replica record from this branch, evicts the space from the reactor
# (stopping background sync), and deletes its local storage. Removal is
# device-local — a synced spot can be rejoined via an invite link; a
# local-only spot is gone for good, which the confirm copy states.
# `subject` reads the form's `data-remove` attribute. Deliberately NOT
# `data-subject`: `tonk/rename-repository` transients already carry
# `dataset/subject`, and a remove command matched on that attribute
# alone would also decode every rename. The distinct attribute doubles
# as the command's unique shape, so no marker field is needed. The
# command is transient: it fires the handler once and is swept before
# the durable commit.
command!: &space/remove
  description: A request to remove a space from this device (its list entry and local data).
  with:
    subject:
      description: The subject DID of the space to remove, read from `data-remove`.
      the: dom.event.current-target.dataset/remove
      as: entity
    prevent-default:
      description: Stop the form submission from reloading the page
      the: dom.event.do/prevent-default
```

- [ ] **Step 2: Restructure the directory row markup**

Replace the current `.spot-list` block (lines 399-418):

```html
      <div class="spot-list">
        <div class="spot-dir-head">Directory</div>
        <a class="srow" href="/space/{subject}" data-kind={kind} data-status={status} with={this}>
          <span class="srow-dot"><tonk-host><tonk-repository name={subject}><tonk-branch name="main"><ui-sync-status></ui-sync-status></tonk-branch></tonk-repository></tonk-host></span>
          <span class="srow-name">
            <tonk-host>
              <tonk-repository name={subject}>
                <tonk-branch name="main">
                  <tonk-display entity={subject} model=tonk:repository view=tonk:view/label>
                    <span slot="no-entity" class="spot-untitled" hidden>Untitled</span>
                    <span slot="no-model" class="spot-untitled" hidden>Untitled</span>
                    <span slot="no-view" class="spot-untitled" hidden>Untitled</span>
                    <span slot="loading" class="spot-untitled" hidden>Untitled</span>
                  </tonk-display>
                </tonk-branch>
              </tonk-repository>
            </tonk-host>
          </span>
        </a>
      </div>
```

with:

```html
      <div class="spot-list">
        <div class="spot-dir-head">Directory</div>
        <!-- The remove radios form one `__rm` group with the rows' per-row
             radios below: rm-closed (chrome, default) hides every confirm;
             a row's radio opens its overlay, and opening another row's
             closes the first. The confirm form's data-close-radio re-checks
             rm-closed on submit; cancel is a plain label for it. -->
        <input class="spot-wiz" type="radio" name="__rm" id="rm-closed" checked>
        <!-- One row per space. The wrapper (not the <a>) carries with={this}
             so the remove affordance and confirm overlay repeat with the
             row while staying outside the link — hovering or clicking the
             x never navigates. -->
        <div class="srow-wrap" data-kind={kind} data-status={status} with={this}>
          <input class="spot-wiz" type="radio" name="__rm" id="rm-{subject}">
          <a class="srow" href="/space/{subject}">
            <span class="srow-dot"><tonk-host><tonk-repository name={subject}><tonk-branch name="main"><ui-sync-status></ui-sync-status></tonk-branch></tonk-repository></tonk-host></span>
            <span class="srow-name">
              <tonk-host>
                <tonk-repository name={subject}>
                  <tonk-branch name="main">
                    <tonk-display entity={subject} model=tonk:repository view=tonk:view/label>
                      <span slot="no-entity" class="spot-untitled" hidden>Untitled</span>
                      <span slot="no-model" class="spot-untitled" hidden>Untitled</span>
                      <span slot="no-view" class="spot-untitled" hidden>Untitled</span>
                      <span slot="loading" class="spot-untitled" hidden>Untitled</span>
                    </tonk-display>
                  </tonk-branch>
                </tonk-repository>
              </tonk-host>
            </span>
          </a>
          <label class="srm-open" for="rm-{subject}" aria-label="Remove spot">
            <wa-icon name="xmark"></wa-icon>
          </label>
          <div class="srm-overlay">
            <form class="srm-card" onsubmit=space/remove data-remove={subject} data-close-radio="rm-closed">
              <div class="onb-kick">remove spot</div>
              <h3 class="srm-title">Delete this spot?</h3>
              <div class="srm-name">
                <tonk-host>
                  <tonk-repository name={subject}>
                    <tonk-branch name="main">
                      <tonk-display entity={subject} model=tonk:repository view=tonk:view/label>
                        <span slot="no-entity" class="spot-untitled" hidden>Untitled</span>
                        <span slot="no-model" class="spot-untitled" hidden>Untitled</span>
                        <span slot="no-view" class="spot-untitled" hidden>Untitled</span>
                        <span slot="loading" class="spot-untitled" hidden>Untitled</span>
                      </tonk-display>
                    </tonk-branch>
                  </tonk-repository>
                </tonk-host>
              </div>
              <p class="onb-lede">Removes this spot and its local data from this device.
                A synced spot can be rejoined with an invite link; a local-only spot is gone for good.</p>
              <div class="srm-actions">
                <label class="srm-cancel" for="rm-closed">
                  <wa-button variant="neutral" appearance="plain" size="small">cancel</wa-button>
                </label>
                <wa-button type="submit" variant="danger" size="small">delete spot</wa-button>
              </div>
            </form>
          </div>
        </div>
      </div>
```

Notes:
- The repeat element is discovered as the LCA of the row's field bindings (`rust/tonk-display/src/template.rs:174` `this_repeat_root`), so moving `with={this}` and the other bindings onto `.srow-wrap` makes the wrapper the cloned row; `rm-closed` and the heading carry no bindings and stay chrome.
- The cancel `<label>` wraps a `wa-button` exactly like the wizard's nav buttons; `.srm-cancel wa-button { pointer-events: none; }` (CSS below) makes the label receive the click, same trick as `.onb-nav`.
- The submit `<wa-button type="submit">` matches the wizard's working submit (profile.yaml line 504).

- [ ] **Step 3: Add the CSS**

In the same `<style>` block, replace the row rules (current lines 228-238) — `.srow` keeps its grid but the hover moves to the wrapper, and the kind/status selectors move to `.srow-wrap`:

```css
        /* One row: a wrapper holding the link plus the remove affordance
           and its confirm overlay. The wrapper (not the <a>) is the
           repeat element, so the x and overlay travel with the row while
           staying outside the link. */
        .srow-wrap { position: relative; }
        .srow { display: grid; grid-template-columns: 56px minmax(0,1fr); align-items: stretch;
          background: transparent; text-decoration: none; color: var(--wa-color-text-normal); }
        .srow > span { display: flex; align-items: center; padding: 18px 0; font-size: 15px; white-space: nowrap; }
        .srow-dot { justify-content: center; }
        .srow-name { padding-left: 4px; min-width: 0; overflow: hidden; text-overflow: ellipsis;
          /* keep the name clear of the hover-revealed remove x */
          padding-right: 44px; }
        .srow-wrap:hover .srow { background: var(--wa-color-neutral-fill-quiet); }
        /* hide the profile's self-replica row */
        .srow-wrap[data-kind="tonk:profile"] { display: none; }
        /* dim the row while the content branch is still seeding */
        .srow-wrap[data-status="tonk:blank"] .srow-name { opacity: .6; }
        .spot-untitled { color: var(--wa-color-text-quiet); }

        /* ── Remove-spot affordance ─────────────────────────────────────
           A hover-revealed x per row opens a per-row confirm overlay,
           driven by the same native-radio trick as the create wizard:
           the row's `__rm` radio shows its overlay; rm-closed (checked
           by default, re-checked by the form's data-close-radio on
           submit and by the cancel label) hides them all. The x stays
           clickable when transparent, so touch works without hover. */
        .srm-open { position: absolute; right: 10px; top: 50%; transform: translateY(-50%);
          display: flex; align-items: center; justify-content: center;
          width: 30px; height: 30px; cursor: pointer; opacity: 0;
          color: var(--wa-color-text-quiet); border-radius: var(--wa-border-radius-s); }
        .srow-wrap:hover .srm-open { opacity: 1; }
        .srm-open:hover { color: var(--wa-color-danger-on-quiet);
          background: var(--wa-color-danger-fill-quiet); }
        .srm-overlay { display: none; position: fixed; inset: 0; z-index: 60;
          background: var(--wa-color-surface-default); color: var(--wa-color-text-normal); }
        .srow-wrap > input:checked ~ .srm-overlay { display: flex;
          align-items: center; justify-content: center; }
        .srm-card { display: flex; flex-direction: column; align-items: center;
          gap: 14px; max-width: 480px; padding: 40px; text-align: center; }
        .srm-title { font-family: var(--spot-font-display); font-weight: 300; font-size: 32px;
          letter-spacing: -1px; margin: 0; color: var(--wa-color-text-normal);
          /* opt out of the global heading "headline-cover" dark box */
          background: none; padding: 0; display: block; }
        .srm-name { font-size: 17px; font-weight: 600; }
        .srm-actions { display: flex; align-items: center; gap: 14px; margin-top: 8px; }
        .srm-cancel { cursor: pointer; }
        .srm-cancel wa-button { pointer-events: none; }
```

(The wizard's radio-hiding class `spot-wiz` is reused for the new radios — it is exactly `position: absolute; opacity: 0; pointer-events: none;`.)

- [ ] **Step 4: Run the library-lowering test**

Run: `cargo test -p tonk-worker --test standard_library it_lowers_the_profile_library`
Expected: PASS — the edited document parses, analyzes locally (the new `&space/remove` anchor resolves), and lowers to claims.

- [ ] **Step 5: Commit**

```bash
git add rust/tonk-core/assets/library/profile.yaml
git commit -m "feat(hub): per-row remove-spot affordance with confirm overlay

Hover x opens a radio-driven confirm; the form asserts the transient
space/remove with the row subject in data-remove.

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 5: Full verification pass

**Files:** none (verification only)

- [ ] **Step 1: Format + workspace lint + native tests**

Run, from the worktree root:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo nextest run -p tonk-schema -p dialog-reactor -p tonk-worker
```

Expected: all PASS. (`cargo nextest run` here runs native tests only; the wasm suite is Task 3 Step 6 / CI's web leg.) If `cargo fmt` reports diffs, run `cargo fmt --all` and amend the relevant commit.

- [ ] **Step 2: wasm compile check of everything wasm-facing**

```bash
cargo check -p tonk-worker -p tonk-display -p tonk-schema --target wasm32-unknown-unknown
```

Expected: PASS.

- [ ] **Step 3: Manual browser verification (or explicit deferral)**

Serve the web app (tonk-ui trunk serve or the repo's dev server), create a throwaway spot, then: hover its row (× appears), click × (confirm overlay opens, names the spot), cancel (overlay closes, spot intact), reopen and delete (overlay closes, row disappears without reload). In devtools Application storage, confirm the space's IndexedDB database and `current/<key>` OPFS directory are gone. If a browser session isn't available in the execution environment, report exactly which of these checks ran.

- [ ] **Step 4: Report**

Summarize: tests run (native counts, wasm ran-or-deferred), lint status, manual verification status. Do not open a PR yet — integration choice (PR to staging) is a separate decision for the finishing step.
