# Serialized space registry implementation plan

**Goal:** Prevent concurrent Tonk CLI processes from losing spaces, bindings, or account selection while preserving atomic, corruption-safe reads.
**Approach:** Put the complete registry read/validate/mutate/save sequence behind an exclusive cross-process `RegistryWriteGuard`, mirroring the account-session lock pattern. Persist through a unique same-directory temporary file, sync it, atomically rename, and then sync the state directory; read-only commands retain lock-free reads except for the one-time legacy migration.
**Constraints:**
- Never replace or recreate a malformed `spaces.json`; corruption continues to fail closed.
- Preserve unknown JSON fields and all entries written by a concurrent process.
- A lock covers the whole logical mutation, including site creation/adoption where publication and registry insertion must be one serial operation.
- Read-only help/list/status paths must not create lock files or mutate an empty store.
- Do not delete site data to resolve a registry conflict.

**Implementation status (2026-08-30):** Complete on
`fix/audit-space-registry-lock`. The initial unit regressions failed because
mutators ignored `spaces.lock` and shared `spaces.json.tmp`; the final branch
passes 193 library tests and all 61 `cli_space` tests with a branch-isolated
binary. Account-space pull staging is intentionally owned by the coupled
filesystem-transitions plan; this branch serializes its final registry
publication through the existing helper without duplicating staging logic.

## File map
- `rust/tonk-cli/src/space.rs`: registry lock/guard interface, unique atomic save, and guarded create/bind/unbind/remove/register/account mutations.
- `rust/tonk-cli/src/bin/tonk.rs`: join registration and account record callers that currently perform direct load/modify/save.
- `rust/tonk-cli/src/account_spaces.rs`: audited call sites; final publication already uses the now-guarded registration helper.
- `rust/tonk-cli/src/account_session.rs`: audited lock ordering; account-pointer callers already settle through `set_account` after their account transition.
- `rust/tonk-cli/tests/cli_space.rs`: independent-process concurrency coverage.
- `docs/storybook/cli/command-surface.md`: document serialization/conflict behavior.
- `docs/storybook/verification/cli-spaces-ui.md`: concurrent writer verification row.

### Task 1: Add a cross-process registry transaction interface

**Files:**
- Modify: `rust/tonk-cli/src/space.rs:SpaceStore::load,save,set_account`
- Test: `rust/tonk-cli/src/space.rs:loading_and_saving`

**Interfaces:**
- Consumes: `SpaceStore` and an exclusive file lock at `<state>/spaces.lock`.
- Produces: `SpaceStore::write_guard() -> Result<RegistryWriteGuard, SpaceError>`, `RegistryWriteGuard::load()`, and `RegistryWriteGuard::save(&Registry)`; the guard retains the lock until the logical mutation is complete.

- [x] Add lock-barrier regressions for registry, account, registration, unbind, and removal writers; each must wait and then load under the retained guard.
- [x] Prove a pre-existing writer-owned `spaces.json.tmp` survives and a lock-open failure leaves the previous JSON byte-for-byte unchanged.
- [x] Run the focused loading/saving regressions red; the original mutators ignored the held lock and overwrote the shared temporary path.
- [x] Implement lock acquisition with `OpenOptions` plus `File::lock`, matching account-session error context. Re-read canonical state only after acquiring the lock.
- [x] Replace `spaces.json.tmp` with `tempfile::NamedTempFile::new_in`, write/`sync_all`, persist by same-directory rename, and sync the directory where supported.
- [x] Keep ordinary `SpaceStore::load` read-only. When it sees the legacy layout, acquire the write guard, re-check both current and legacy files, and run migration once under the lock.
- [x] Run 47 focused space tests and all 193 `tonk-cli` library tests serially.

### Task 2: Move every registry mutation under the guard

**Files:**
- Modify: `rust/tonk-cli/src/space.rs:register_existing_unbound,register_existing_bound,create,bind,unbind,remove,set_account`
- Audit: `rust/tonk-cli/src/account_spaces.rs:pull/delete registry writes`
- Modify: `rust/tonk-cli/src/bin/tonk.rs:join registration and account record updates`
- Test: `rust/tonk-cli/src/space.rs`
- Audit: `rust/tonk-cli/src/account_session.rs`

**Interfaces:**
- Consumes: `RegistryWriteGuard`; each mutator loads through that guard and saves through the same guard.
- Produces: unchanged public outcomes, with lost-update prevention across independent processes.

- [x] Add deterministic held-lock coverage for bind, account selection, mounted-space registration, unbind, and removal, plus real-process create/create and bind/remove overlaps.
- [x] Run the focused regressions red; at least bind and account selection completed while another process retained the lock.
- [x] Refactor every production `load -> mutate -> save` site to one retained guard. Keep pure listing/inventory reads lock-free.
- [x] Retain the guard across async site initialization, registration, and the invocation-directory binding so `space new` is one logical registry transaction.
- [x] Register and bind a joined site atomically; preserve delete-before-registry ordering while preventing stale-snapshot resurrection.
- [x] Re-run focused tests and the complete real-binary `cli_space` target.

### Task 3: Prove independent CLI processes do not overwrite each other

**Files:**
- Modify: `rust/tonk-cli/tests/cli_space.rs`

**Interfaces:**
- Consumes: the real `tonk` binary, one isolated `TONK_SPACES_STATE`, an externally held file lock, and distinct operations.
- Produces: stable final `spaces.json` with both valid operations or an explicit serialized conflict; never malformed JSON, a missing entry, or an unregistered created site.

- [x] Hold `spaces.lock` in the test process, spawn both commands, then release it to race two distinct `space new --site` operations.
- [x] Race independent `space use` and `space rm --keep-data` writers and assert binding cleanup without last-writer snapshot loss.
- [x] Isolate the spawned `tonk` binary from shared-worktree artifact replacement and run the two focused process regressions.
- [x] Adjust only production synchronization; add no test-only production hooks.
- [x] Re-run the complete 61-test `cli_space` target against the isolated branch binary.

### Task 4: Document concurrency and run final checks

**Files:**
- Modify: `docs/storybook/cli/command-surface.md`
- Modify: `docs/storybook/verification/cli-spaces-ui.md`

**Interfaces:**
- Consumes: serialized mutation behavior.
- Produces: clear lock/conflict/safe-state documentation and a repeatable two-process verification case.

- [x] Document lock-free complete reads, serialized registry mutations, atomic publication, and command-specific recovery after any pre-publication site work.
- [x] Add a verification row for distinct creates and bind/remove overlap; retain corruption and interrupted-write coverage in the shared CLI fault rows.
- [x] Regenerate Storybook data, check it, and validate all 174 local links.
- [x] Run `cargo fmt --all -- --check`, 47 focused unit tests, 193 library tests, and all 61 real-process CLI tests after the final source change.
