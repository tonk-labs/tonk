# CLI account logout implementation plan

**Goal:** Add `tonk account logout` so the native CLI can disconnect its local provider attachment without erasing or rotating the profile, revoking the device, or losing local spot authority.

**Approach:** Match the existing browser worker's `DELETE /api/account` / `unlink` semantics: write the account-provider credential's established empty-byte tombstone through the account-state operator, while leaving the local root, profile signer, account repository, trusted marker, spots, and delegations intact. Expose that operation from `tonk_cli::account`, wire it into the Clap command and telemetry descriptor, and document that remote revocation remains the separate `tonk account revoke` workflow.

**Constraints:**
- Implement against the current `feat/account-spots-cli` branch (HEAD `745ddeca2` when this plan was written) and preserve its account-spots listing, restore, and best-effort backup behavior.
- `logout` means local provider detachment, matching `rust/tonk-worker/src/router/account.rs:unlink`; it must not call the account service, self-revoke the device, open a browser, reset the profile, rotate the device DID, remove the durable local root, delete the account repository, remove a trusted-base marker, or mutate any spot/site/registry/binding.
- Keep `tonk account revoke <DID>` as the explicit server-side revocation operation. Logging out must remain usable offline and must leave the device active in the provider's device registry.
- The dialog credential API has no delete operation. Clear only `tonk_account::ACCOUNT_PROVIDER_CREDENTIAL_SITE` by saving `Vec::<u8>::new()`, which `AccountProviderRecord::decode` already defines as the detach tombstone.
- The operation is idempotent: an already-unregistered or root-missing profile may run `tonk account logout` successfully, and a second logout must remain successful.
- After logout, `tonk account status` must report `provider: none` while retaining the same root DID and device DID when they existed. A later `tonk account link` may reattach provider services to that unchanged local authority.
- Do not require an active spot or add `--spot` behavior, confirmation prompts, network options, dependencies, schema/storage migrations, a version bump, or `Cargo.lock` changes.
- Preserve unrelated current-branch work and use the existing error/exit-code path (`anyhow::Result` into `print_failure`).

## File map

- `rust/tonk-cli/src/account.rs`: implement and unit-test provider tombstoning through the mounted account credential operator.
- `rust/tonk-cli/src/bin/tonk.rs`: define and dispatch `account logout`, describe its non-destructive semantics in help, and classify it as the static `account`/`logout` telemetry descriptor.
- `rust/tonk-cli/README.md`: add the command to account usage and distinguish logout from revocation and identity reset.
- `plan/account-logout-cli.md`: durable implementation handoff; no production behavior.

### Task 1: Detach the native provider without changing local authority

**Files:**
- Modify: `rust/tonk-cli/src/account.rs:stored_provider_with_operator, new logout/logout_with_operator functions, tests`
- Test: `rust/tonk-cli/src/account.rs:tests`

**Interfaces:**
- Consumes: `account_state::credential_operator(profile)`, `ACCOUNT_LINK_SITE`, and the empty-byte tombstone contract in `tonk_account::AccountProviderRecord::decode`.
- Produces:

```rust
/// Disconnect provider services while preserving this profile's root,
/// delegations, account repository, and spots.
pub async fn logout(profile: &dialog_operator::Profile) -> anyhow::Result<()>;

// Private seam used by `logout` and isolated unit coverage.
async fn logout_with_operator(
    profile: &dialog_operator::Profile,
    operator: &dialog_operator::Operator<dialog_storage::provider::storage::NativeSpace>,
) -> anyhow::Result<()>;
```

- [ ] Add `it_logs_out_by_tombstoning_only_the_provider_attachment` in `account.rs`. Create a temp-rooted profile and `SpotStore`, build its operator with `account_state::credential_operator_for_store`, and persist: (1) a valid `LocalRoot` JSON record at `identity::LOCAL_ROOT_SITE`, (2) an encoded `AccountProviderRecord::attach_unconfigured(...)` at `ACCOUNT_LINK_SITE`, and (3) sentinel bytes at `tonk_account::TRUSTED_BASE_CREDENTIAL_SITE`. Also place a sentinel file under `store.account_dir()` and capture the profile DID. Assert `stored_provider_with_operator` is `Some` before logout.
- [ ] In that test, call the not-yet-existing `logout_with_operator`; then assert `stored_provider_with_operator` is `None`, the raw `ACCOUNT_LINK_SITE` value is exactly empty, the `LocalRoot` record is byte-for-byte unchanged, the trusted-base marker is unchanged, the account-directory sentinel still exists, and `profile.did()` is unchanged. Run `cargo test -p tonk-cli --lib account::tests::it_logs_out_by_tombstoning_only_the_provider_attachment`; expect compilation failure because `logout_with_operator` does not exist.
- [ ] Implement `logout_with_operator` as one credential save: `profile.credential().site(ACCOUNT_LINK_SITE).save(Vec::<u8>::new()).perform(operator).await`, with context `failed to clear the account provider`. Do not load a connection, issue an HTTP request, alter local-root/access credentials, or remove account-state files.
- [ ] Implement public `logout` as the production adapter that obtains `account_state::credential_operator(profile)` and delegates to `logout_with_operator`. Keep it returning `Result<()>`: once the tombstone save succeeds, a later status-read problem must not turn the completed local mutation into a misleading logout failure.
- [ ] Extend the same test (or add `it_allows_repeated_logout`) to call `logout_with_operator` a second time and assert success and the same preserved state. Run `cargo test -p tonk-cli --lib account::tests::it_logs_out_by_tombstoning_only_the_provider_attachment`; expect success.
- [ ] Run `cargo test -p tonk-cli --lib account::tests`; expect all account link/device/revoke/logout unit tests to pass.

### Task 2: Expose `tonk account logout` and document its boundary

**Files:**
- Modify: `rust/tonk-cli/src/bin/tonk.rs:AccountCommand, descriptor, account_op, account parser tests`
- Modify: `rust/tonk-cli/README.md:Usage account examples and account behavior note`
- Test: `rust/tonk-cli/src/bin/tonk.rs:account parser tests`

**Interfaces:**
- Consumes: `account::logout(&Profile) -> anyhow::Result<()>` from Task 1 and the already-opened profile in `account_op`.
- Produces: a no-argument `AccountCommand::Logout` parsed from `tonk account logout`, telemetry descriptor `("account", Some("logout"))`, success output `logged out\ndevice: <DID>`, and the usual non-zero `print_failure` path when the tombstone cannot be persisted.

- [ ] Add `account_logout_is_a_no_argument_account_operation` beside the current account-spots parser tests. Parse `tonk account logout`, assert it yields `Command::Account { command: AccountCommand::Logout }`, and assert `descriptor` returns `("account", Some("logout"))`. Also assert `tonk account logout unexpected` is rejected by Clap. Run `cargo test -p tonk-cli --bin tonk account_logout_is_a_no_argument_account_operation`; expect compilation failure because the variant is absent.
- [ ] Add `AccountCommand::Logout` with help that says it disconnects account services on this device, preserves local identity/root/spots, and does not revoke the device; direct users to `tonk account revoke <DID>` when they intend revocation. Do not add a confirmation or service URL argument because logout is local, reversible, and offline.
- [ ] Add the exhaustive `descriptor` arm returning `"logout"`, so telemetry records only the static command name and no account identifiers.
- [ ] Add the `account_op` arm. Call `account::logout(&profile)`; on success print exactly `logged out` and `device: {profile.did()}` on separate lines and return `ExitCode::Success`; on error use `print_failure(error)`. Do not invoke account-spots backup, status hydration, revocation, identity reset, or any active-spot path.
- [ ] Rerun `cargo test -p tonk-cli --bin tonk account_logout_is_a_no_argument_account_operation`; expect success.
- [ ] Update `rust/tonk-cli/README.md` account examples with `tonk account logout`. State that it writes only a local detach tombstone, keeps the root/device identity and spots available, works offline, and does not revoke the provider-side device; point to `tonk account devices` plus `tonk account revoke <DEVICE_DID>` for revocation and reserve `tonk identity --reset` for destructive identity rotation.
- [ ] Run `cargo test -p tonk-cli --bin tonk`; expect all parser/dispatch helper tests to pass.
- [ ] Run `cargo test -p tonk-cli --features integration-tests --test account_spots`; expect current-branch account inventory, pull, and backup behavior to remain green.
- [ ] Run `cargo fmt --all -- --check` and `cargo test -p tonk-cli`; expect formatting and all default CLI tests to pass. Confirm `git diff -- Cargo.lock` is empty.

## Handoff verification

- [ ] Run `rg -n "TBD|TODO|similar to|handle errors|write tests" plan/account-logout-cli.md`; expect no unresolved implementation placeholders (the command may match this verification sentence only).
- [ ] Review the final diff and verify every production change is confined to `rust/tonk-cli/src/account.rs`, `rust/tonk-cli/src/bin/tonk.rs`, and `rust/tonk-cli/README.md`; no account-service, worker, identity-reset, spot-storage, dependency, lockfile, or version changes belong in this feature.
- [ ] Verify the behavior matrix from tests and code: linked → provider tombstone; unlinked → success; repeated logout → success; local root/device/account directory/spots preserved; no network/revocation path reachable; telemetry descriptor is static `account/logout`.
