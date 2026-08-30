# Safe filesystem transitions implementation plan

**Goal:** Preserve the only valid copy of user data through migration, invite join, and account-space pull failures, including abrupt termination before the canonical destination is published.
**Approach:** Put directory publication behind one small `StagedDirectory` interface: build and verify in a unique sibling directory, then atomically rename it to the previously absent canonical path. `--move` records the source directory identity before copying and removes that exact source only after the verified destination is published; cleanup failure retains the verified destination and reports both exact paths.
**Constraints:**
- Never overwrite an existing canonical destination or delete a source that is not represented by a verified canonical destination.
- Preserve local profiles, passkeys, space data, and unrelated orphan directories; interrupted staging directories may be diagnosed but must never be adopted or deleted without proving their marker and target.
- Keep the final rename on the destination filesystem and support Linux and macOS; do not rely on renaming an open Dialog repository.
- Follow the existing `SpaceError`/`InviteError` and `anyhow` user-facing error conventions, with exact safe-state and retry guidance.

## File map
- `rust/tonk-cli/src/staged_directory.rs`: unique sibling staging, marker validation, cleanup-on-error, and atomic publish interface.
- `rust/tonk-cli/src/lib.rs`: expose the internal staging module to migration, invite, and account-space code.
- `rust/tonk-cli/src/migrate.rs`: copy, verify, publish, and only then remove a `--move` source.
- `rust/tonk-cli/src/invite.rs`: build a joined site outside the canonical name and publish after all required local claim work succeeds.
- `rust/tonk-cli/src/account_spaces.rs`: replace canonical-first pull cleanup with staged publication.
- `rust/tonk-cli/src/site.rs`: add the crate-internal empty-directory mount adapter required by a pre-created owned stage; keep the public fresh-path collision behavior unchanged.
- `rust/tonk-cli/src/space.rs`: keep hidden Tonk staging paths out of orphan diagnostics while leaving arbitrary directories visible.
- `rust/tonk-cli/src/bin/tonk.rs`: keep the join orchestration comment aligned with staged canonical publication and post-publication adoption.
- `rust/tonk-cli/tests/site.rs`: migration failure regressions.
- `rust/tonk-cli/tests/cli_space.rs`: join/pull canonical-path and recovery regressions where the CLI fixture is required.
- `docs/storybook/cli/command-surface.md`: document the safe-state and retry contract for partial filesystem operations.
- `docs/storybook/verification/cli-spaces-ui.md`: add restart/fault evidence rows for migrate, join, and account pull.

### Task 1: Publish verified directories through one staging interface

**Files:**
- Create: `rust/tonk-cli/src/staged_directory.rs`
- Modify: `rust/tonk-cli/src/lib.rs`
- Test: `rust/tonk-cli/src/staged_directory.rs`

**Interfaces:**
- Consumes: an absent canonical `destination: &Path` and a stable operation label.
- Produces: `StagedDirectory::beside(destination, label)`, `path(&self) -> &Path`, and `publish(self) -> Result<PathBuf>`; `publish` succeeds only by renaming the staged directory to the still-absent destination.

- [x] Add `it_never_replaces_an_existing_destination`, `it_cleans_an_unpublished_stage_on_returned_error`, and `it_publishes_by_sibling_rename` using real temporary directories and literal file contents.
- [x] Run `cargo test -p tonk-cli staged_directory -- --test-threads=1`; expect failure because the module and interface do not exist.
- [x] Implement unique hidden sibling names containing a Tonk marker, create with `create_dir`, refuse symlink/non-directory parents, and retain destination plus stage paths in every error.
- [x] Make `publish` re-check destination absence, sync written files/containing directory where supported, rename once, and disarm `Drop` only after success.
- [x] Run `cargo test -p tonk-cli staged_directory -- --test-threads=1`; expect all focused tests to pass.
- [ ] Run `cargo test -p tonk-cli --lib -- --test-threads=1`; expect success (loopback-bind failures require an unchanged rerun with loopback access).

### Task 2: Make `tonk migrate carry --move` non-destructive until verification

**Files:**
- Modify: `rust/tonk-cli/src/migrate.rs:run_inner,perform_transfer`
- Test: `rust/tonk-cli/tests/site.rs:when_migrating_from_carry`

**Interfaces:**
- Consumes: `StagedDirectory` and the existing `open_for_verify` seam.
- Produces: the existing `MigrationOutcome`; `moved == true` means the source was removed after verified publication, never merely renamed before verification.

- [x] Add a corrupt-source regression named `it_preserves_the_move_source_when_destination_verification_fails`; assert source bytes still exist, `.tonk` is absent, and the error says verification failed before publication.
- [x] Add a source-cleanup fault seam at the filesystem adapter level and a test asserting a verified `.tonk` remains intact when source removal fails; the error must say both copies remain and name both paths.
- [x] Capture the move source's device/inode before copying, revalidate immediately before deletion, and test that an identity mismatch fails before the filesystem adapter can delete the current path; unsupported identity platforms fail closed.
- [x] Run `cargo test -p tonk-cli --test site when_migrating_from_carry -- --test-threads=1`; expect the corrupt `--move` case to fail because current code deletes both paths.
- [x] Copy into the stage for both `Copy` and `Move`, verify the stage, drop repository handles, publish it, and only then remove the source for `Move`.
- [x] Ensure verification or publish failure removes only the marked stage and leaves the source untouched; never remove the canonical destination after publication.
- [x] Run the focused site test command; expect all migration tests to pass.

### Task 3: Publish join and account-pull sites only after local completion

**Files:**
- Modify: `rust/tonk-cli/src/invite.rs:claim`
- Modify: `rust/tonk-cli/src/account_spaces.rs:FreshPullTarget,pull`
- Modify: `rust/tonk-cli/src/space.rs:SpaceStore::orphaned_sites`
- Test: `rust/tonk-cli/tests/site.rs` (real invite targeting/claim state)
- Test: `rust/tonk-cli/tests/account_spaces.rs` (real account and access services)
- Test: `rust/tonk-cli/src/space.rs` (marked-stage orphan classification)

**Interfaces:**
- Consumes: canonical `spaces/<name>` path and `StagedDirectory`.
- Produces: unchanged `ClaimOutcome`/`PullOutcome`; success guarantees canonical publication. An error before publication leaves the canonical path absent unless it pre-existed. Account pull's guarded registry transaction follows publication; if that transaction alone fails, the complete canonical replica remains and the error directs the user to inspect occupied names, verify the repository subject, and adopt under an available name rather than overwriting unrelated state.

- [x] Add an invite-claim failure after site initialization and assert the canonical target is absent and a same-name retry reaches claim logic instead of `SiteAlreadyExists`.
- [x] Add an account-pull failure after local mount/member validation and assert the canonical target is absent; simulate an abandoned marked stage and assert it neither blocks retry nor appears as a user orphan.
- [x] Run the invite and orphan focused tests separately (Cargo accepts one name filter): expect failure because invite creates the canonical directory first and orphan listing exposes marked stages.
- [x] Split each operation into an inner build against `stage.path()` and an outer publish after all required local work and handles are dropped; keep network pull/push warnings best-effort exactly where they are today.
- [x] Filter only correctly prefixed Tonk staging entries from orphan display; continue showing every unmarked hidden or visible directory.
- [ ] Run the focused tests, then `cargo test -p tonk-cli --features integration-tests --test cli_space -- --test-threads=1`; expect success.

### Task 4: Document and verify the durability contract

**Files:**
- Modify: `docs/storybook/cli/command-surface.md`
- Modify: `docs/storybook/verification/cli-spaces-ui.md`

**Interfaces:**
- Consumes: the completed filesystem behavior and exact error copy.
- Produces: restart/fault verification steps that distinguish canonical data, staging residue, source retention, and retry behavior.

- [x] Document that canonical names appear only after verification, `--move` cleanup failure leaves two valid copies, and unmarked orphan data is never deleted automatically.
- [x] Add verification cases for migration verification failure, join termination before publish, and account-pull termination before publish.
- [x] Run `python3 docs/storybook/scripts/build.py` to regenerate derived Storybook data.
- [x] Run `python3 docs/storybook/scripts/build.py --check` and `python3 docs/storybook/scripts/check-links.py docs/storybook`; expect success.
- [ ] Run `cargo fmt --all -- --check` and re-run the focused Rust tests after the final documentation/generated-file change.

## Implementation evidence

- Staging red: `cargo test -p tonk-cli staged_directory -- --test-threads=1` ran three tests; all three failed at the deliberately unimplemented creation seam. The subsequent focused run passed all three. After marker-ownership and no-clobber coverage were added, the current branch-specific library executable passed all six staging tests, including same-name winner/loser and tampered-marker preservation.
- No-clobber race red: the already-built branch-specific unit executable ran `staged_directory::tests::the_publish_primitive_never_replaces_an_existing_empty_directory` directly and failed because the check-then-`std::fs::rename` primitive returned success over an empty competing destination. Publication now uses the platform atomic no-replace flag (`RENAME_NOREPLACE` on Linux and `RENAME_EXCL` on macOS) through `rustix`; no check-then-rename fallback is allowed.
- Migration red: the first attempt was infrastructure-only (`No space left on device`) and is not behavioral evidence. After cleaning only this worktree's generated target, the unchanged focused command ran four tests and failed only `it_preserves_the_move_source_when_destination_verification_fails` because the source path had been deleted. After implementation, all four passed; the private source-cleanup refusal test also passed with both literal copies intact.
- Join red: the wrong-recipient test failed after initialization because the canonical target existed. After staged publication it passed twice through claim validation without `SiteAlreadyExists`.
- Orphan red: the corrected canonical-path assertion showed the marked stage in the orphan list alongside hidden and visible user directories. After filtering through the staging module's marker parser, only the two user directories remain reportable.
- Interface correction: `mount_delegated_at` intentionally requires a nonexistent root, whereas `StagedDirectory` intentionally pre-creates and owns its root. `mount_delegated_in_empty` is therefore a crate-internal adapter that verifies the owned directory is real and empty before reusing the same account-required mount implementation; public collision behavior is unchanged.
- Cargo coordination: subsequent checks use `CARGO_TARGET_DIR=/Users/jackdouglas/tonk/tonk/target` plus the stable package-only `profile.test.package.tonk-cli.codegen-units=13` override. This avoids recreating the removed per-worktree target and prevents another worktree from substituting its `tonk-cli` test artifact in the shared target.
- Final controlled green: the qualified atomic no-replace regression passed 1/1 after the `rustix` change. After the stable registry-collision and move-source identity refinements, the freshly rebuilt library executable passed migration cleanup/identity 2/2, stable account registry collision 1/1, and all staging invariants 6/6. The marked-stage orphan classification also passed 1/1 on the preceding build and its source was unchanged afterward. Direct `rustfmt --check` over every changed Rust file, Storybook generation/check, and all 173 Storybook local-link checks passed. Non-Unix identity behavior is fail-closed by static cfg inspection; released CLI platforms are Linux and macOS.
- Resource-limited verification boundary: the full `tonk-cli --lib`, current membership-validation integration, and full `cli_space` runs remain unchecked because shared-target disk headroom fell below the coordinated build threshold. Earlier behavioral greens before the final no-replace change covered carry migration 4/4, wrong-recipient join 1/1, marked-stage orphan classification 1/1, and offline account pull 1/1; the final generic staging layer is covered by the current six-test executable.
