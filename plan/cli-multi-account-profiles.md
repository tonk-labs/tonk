# CLI multi-account profiles and space authority implementation plan

**Goal:** Let one native Tonk installation use several accounts without ever allowing the currently selected account to adopt, authorize, reconcile, or publish another account profile's spaces, while preserving offline local edits and later synchronization through the space's own profile.
**Approach:** Mirror the browser's profile-per-account boundary. Each native profile owns its Dialog profile, account session, account repository, named-space registry, and canonical space directories; a small install-level registry selects the default profile and maps directory bindings to an exact `(profile, space)` pair. Space-scoped commands resolve that pair first and therefore never borrow authority from the currently selected account. Local writes remain available while the owning profile is logged out; remote forks require that same profile's active account grant and an already-persisted space-to-account prefix.
**Implementation status (2026-08-19):** Built and verified. Tasks 1–8 are represented by the production paths and focused regressions described below. The complete ordinary and feature-gated suites pass with `RUST_TEST_THREADS=1`; parallel full-suite execution can race the pre-existing carry-migration signing-session fixture, while its exact isolated test passes.
**Constraints:**
- The local filesystem is a trusted cache for one OS user, not an account security boundary. This work prevents accidental cross-account operations and unauthorized remote publication through supported CLI paths; it does not claim to protect unencrypted files or credentials from a malicious process running as the same OS user.
- Offline commits made through profile A while A is logged out are valid local work. They remain pending and may synchronize after A signs in again.
- A command resolved to profile A must never use profile B's account grant, provider attachment, account repository, deployment defaults, or certificate store, even when B is the install's selected profile.
- Login, account switching, ordinary sync, status, backup reconciliation, and account migration must never mint a new `space -> current account root` prefix. Prefix minting is allowed only during explicit create, join, pull/recovery, or legacy adoption into the chosen profile.
- Signing into another account must not replace a rooted profile in place. A known profile can sign back into the same immutable account root; a different account requires a new profile.
- One profile has at most one immutable account root. The same account root may appear in more than one local profile because each profile is a distinct device identity; `account add` warns instead of attempting to merge incompatible device grants.
- Logging out detaches provider access only for that profile. It preserves the profile DID, account root, account repository, local spaces, remote configuration, historical authority, and pending local revisions.
- Directory bindings retain their current precedence role. A directory binding resolves both the space and its profile; the selected account is only the default for explicit `--spot`/`TONK_SPOT`, `spot new`, `join`, and account-scoped commands.
- Existing installs keep the `tonk` Dialog profile and current install-level `spots.json`, `spots/`, and `account/` directories in place as the grandfathered `legacy` profile. Migration creates metadata only and never moves or deletes user data.
- New profiles use isolated state directories so older CLI releases can see only the grandfathered profile, not spaces created under another account profile.
- Space names are profile-scoped. Two profiles may both have a `garden`; a persisted directory binding remains unambiguous because it stores the profile ID as well as the name.
- Existing custom content remotes and upstreams are never overwritten. Automatic account convergence provisions the deployment default only when a space has no upstream.
- Account-service backup rows remain projections. Canonical account authority stays in the signed, root-owned account repository; provider APIs never become authorization authority.
- Do not add encryption, OS-user isolation, signed per-revision authorship, UI removal tombstones, or remote deletion semantics in this change. Those are separate designs.
- Preserve old `spot`, `--spot`, `TONK_SPOT`, and `account spots` spellings during this migration. The broader `spot` to `space` compatibility migration is a separate reviewable change.
- Introduce no new cryptographic format or third-party dependency. Reuse the existing profile keys, account-session records, account repository, UCAN delegation validation, access-service protocol, and the existing workspace `tonk-worker-api` crate's `DeploymentConfig` contract.

## User-visible semantics

The install has a selected profile for account-scoped commands and unbound space creation, but a bound space carries its own profile context:

```text
cwd binding / --spot / TONK_SPOT
        |
        v
ResolvedSpace { profile_id, name, site }
        |
        +--> Dialog profile and credential store
        +--> profile-scoped account session and account repository
        +--> profile-scoped content remote authorization
```

- In a directory bound to account A's `garden`, `tonk eval` opens A's profile even when account B is selected.
- If A is logged out, the eval commits locally and reports that synchronization is pending for A. It does not try B's credentials.
- `tonk account use work` changes the default profile locally and performs no network request.
- `tonk account logout` detaches only the selected profile and leaves it selected.
- `tonk account login` signs the selected, rooted profile back into the same account. A handoff returning another root is rejected without changing the profile.
- `tonk account add --label work` creates a fresh local profile before starting the browser ceremony. An interrupted ceremony remains resumable in that profile.
- `tonk account link` remains a compatibility alias: it adds the first profile or resumes an unrooted one, and otherwise behaves as `account login` for the selected profile.
- `tonk account list` shows every local profile, its label, root, local sign-in state, and whether it is selected.
- New `spot new`, `join`, and `account spots pull` operations use the selected profile and register the resulting space only in that profile's store.

## On-disk layout and contracts

Keep the current install root selected by `TONK_SPOTS_STATE`, but add an install-level profile registry:

```text
tonk/
  profiles.json                 install-level profile roster and directory bindings
  spots.json                    grandfathered profile's unchanged registry
  spots/                        grandfathered profile's unchanged site directories
  account/                      grandfathered profile's unchanged account state
  profiles/
    p-<32 lowercase hex>/
      spots.json                this profile's named-space registry
      spots/<name>/             this profile's canonical site directories
      account/                  this profile's account session/repository state
```

Dialog profile key material remains in Dialog's existing platform profile storage. The install registry stores only non-secret routing metadata:

```rust
pub const LEGACY_PROFILE_ID: &str = "legacy";

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct NativeProfileId(String);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeProfileRecord {
    pub label: String,
    pub dialog_profile_name: String,
    pub account_root: Option<String>,
    pub ceremony_origin: Option<String>,
    pub default_access_remote: Option<String>,
    pub default_revocation_relay: Option<String>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BoundSpace {
    pub profile: NativeProfileId,
    pub space: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeProfileRegistryV1 {
    pub version: u8, // exactly 1
    pub selected: Option<NativeProfileId>,
    pub profiles: BTreeMap<NativeProfileId, NativeProfileRecord>,
    pub bindings: BTreeMap<PathBuf, BoundSpace>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}
```

`NativeProfileId::generate` uses 16 random bytes encoded as `p-` plus 32 lowercase hexadecimal characters. The generated Dialog profile name is `tonk-<32 hex>`; the legacy record alone uses the existing literal `tonk`. `account_root` is an index and mismatch guard, not the canonical root record; the signed `tonk-local-root-v1` credential inside the Dialog profile remains canonical.

`label` is a unique, case-insensitive local slug using `[a-z0-9][a-z0-9-_]*`. `account use` accepts either that label or the exact profile ID. An omitted label chooses `account`, then `account-2`, `account-3`, and so on; the grandfathered profile uses `default`. Labels are routing conveniences only and never enter a signed account or authorization object.

`NativeProfileContext` is the only value allowed to open native account or space state:

```rust
#[derive(Clone, Debug)]
pub struct NativeProfileContext {
    pub id: NativeProfileId,
    pub record: NativeProfileRecord,
    pub store: SpotStore,
}

impl NativeProfileContext {
    pub async fn open_profile(&self) -> anyhow::Result<dialog_operator::Profile>;
    pub fn site_config(&self) -> site::SiteConfig;
}
```

For `legacy`, `store` is the install root. For generated IDs, it is `install_root/profiles/<id>`. All account-session locks, account-repository bytes, spot registry reads, canonical site paths, backup sweeps, and migration loops take the context's `SpotStore`; none call the global `SpotStore::open()` after resolution.

## File map

- `plan/cli-multi-account-profiles.md`: Durable design, task order, acceptance criteria, and verification commands.
- `rust/tonk-cli/src/account_profiles.rs`: Versioned install registry, legacy bootstrap, profile creation/selection, directory bindings, context opening, and deployment-default persistence.
- `rust/tonk-cli/src/deployment.rs`: Validate the browser ceremony deployment and discover the default access remote and revocation relay.
- `rust/tonk-cli/src/spot.rs`: Profile-local spot registry primitives; remove install-global directory-binding resolution from this layer.
- `rust/tonk-cli/src/site.rs`: Open/create a site with the resolved profile context and account-state directory; split strict prefix loading from explicit adoption.
- `rust/tonk-cli/src/identity.rs`: Open, inspect, and reset an explicitly selected Dialog profile rather than the fixed global profile.
- `rust/tonk-cli/src/account_session.rs`: Scope transition locks, canonical session state, pending handoffs, and detach outboxes to one profile store.
- `rust/tonk-cli/src/account.rs`: Add/login/logout/status operations for an explicit profile context and same-root login guard.
- `rust/tonk-cli/src/account_state.rs`: Mount, hydrate, migrate, and retain delegations in only the resolved profile's account repository.
- `rust/tonk-cli/src/account_authority.rs`: Authorize remote forks from the resolved profile's active grant and strict pre-existing space prefix.
- `rust/tonk-cli/src/account_spots.rs`: List, pull, project, and reconcile only the resolved profile's spaces.
- `rust/tonk-cli/src/account_sync.rs`: Profile-scoped provision/pull/push/retain/project reconciliation and confirmed-revision markers.
- `rust/tonk-cli/src/auto_sync.rs`: Preserve successful local commits while reporting profile-specific pending synchronization.
- `rust/tonk-cli/src/bin/tonk.rs`: Account profile commands, context resolution, profile-aware output, and reconciliation triggers.
- `rust/tonk-cli/src/lib.rs`: Register the new modules and expose the bounded integration-test interfaces.
- `rust/tonk-cli/Cargo.toml`: Register `account_profiles` integration tests and add the existing workspace `tonk-worker-api` crate for the deployment contract.
- `rust/tonk-cli/tests/account_profiles.rs`: Registry migration, profile selection, interrupted add/login, same-root guard, and account-switch integration coverage.
- `rust/tonk-cli/tests/account_authority.rs`: Cross-profile remote denial, explicit adoption, logout/offline edit, and later-sync coverage.
- `rust/tonk-cli/tests/account_spots.rs`: Profile-filtered inventory, pull, backup projection, and convergence coverage.
- `rust/tonk-cli/tests/spot.rs`: Profile-local duplicate names and stable binding resolution.
- `rust/tonk-cli/tests/cli_spot.rs`: CLI output and directory-binding behavior across selected profiles.
- `rust/tonk-cli/tests/site.rs`: Dynamic profile configuration and strict/adopt prefix behavior.
- `rust/tonk-cli/tests/account_interrupt.rs`: Interrupted handoff recovery in the provisional profile.
- `rust/tonk-cli/tests/common.rs`: Multi-profile account fixtures with isolated state roots.
- `rust/tonk-cli/README.md`: Multi-account, offline edit, account selection, profile-bound space, and threat-boundary documentation.

### Task 1: Add the install-level native profile registry and grandfather existing state

**Files:**
- Create: `rust/tonk-cli/src/account_profiles.rs`
- Modify: `rust/tonk-cli/src/spot.rs:SpotStore path accessors and legacy binding helpers`
- Modify: `rust/tonk-cli/src/lib.rs:module registration and test exports`
- Modify: `rust/tonk-cli/Cargo.toml:[[test]] account_profiles`
- Create: `rust/tonk-cli/tests/account_profiles.rs`

**Interfaces:**
- Consumes: `SpotStore::open/at`, the current `spots.json` registry, `site::PROFILE_NAME == "tonk"`, and `identity::local_root_with_operator`.
- Produces: `NativeProfileId`, `NativeProfileRecord`, `BoundSpace`, `NativeProfileRegistryV1`, `NativeProfileContext`, and `NativeProfileStore::{open, at, load_or_bootstrap, selected, select, create_pending, context, bind, unbind}`.

- [ ] Add `it_bootstraps_the_legacy_profile_without_moving_state`. Arrange an install root containing the current `spots.json`, one canonical site path, one directory binding, and existing `account/` bytes; run bootstrap and assert `profiles.json` contains one selected `legacy` record using Dialog profile `tonk`, the binding becomes `{ profile: legacy, space: garden }`, and every original path and byte remains unchanged.
- [ ] Add `it_starts_empty_without_creating_a_dialog_profile`. An install with no `profiles.json`, `spots.json`, `spots/`, or `account/` returns no selected profile and performs no Dialog profile I/O.
- [ ] Add `it_rejects_unknown_versions_corrupt_json_and_dangling_bindings`. Require errors that name `profiles.json`, the unsupported version, unknown profile ID, or missing profile-local space; never silently recreate the registry.
- [ ] Add `it_preserves_unknown_registry_and_profile_fields` so a read/write round trip retains newer-version metadata.
- [ ] Add `it_creates_distinct_pending_profiles_with_isolated_state_roots`. Use deterministic injected random bytes in the test, assert IDs/profile names follow the contract, and assert neither generated state root aliases the legacy root or another profile.
- [ ] Run `cargo test -p tonk-cli --test account_profiles`; expect compilation failure because the module and types do not exist.
- [ ] Implement atomic `profiles.json.tmp` plus rename writes, canonicalized binding keys, exact version checking, generated IDs, derived state roots, and the legacy bootstrap. Copy legacy directory bindings into the install registry but leave the legacy `Registry.bindings` field on disk untouched for downgrade visibility.
- [ ] Do not open/create a Dialog profile from a read-only registry/list command. `NativeProfileContext::open_profile` is the explicit boundary that may touch key storage.
- [ ] Run `cargo test -p tonk-cli --test account_profiles`; expect all registry and migration tests to pass.
- [ ] Run `cargo test -p tonk-cli --test spot`; expect existing spot-registry behavior to remain green before resolution moves in Task 2.

### Task 2: Resolve every space to an exact profile and keep profiles' local state disjoint

**Files:**
- Modify: `rust/tonk-cli/src/account_profiles.rs:resolve, bind, unbind, list_spaces`
- Modify: `rust/tonk-cli/src/spot.rs:Registry, Resolved, listing, bind/unbind call boundaries`
- Modify: `rust/tonk-cli/src/site.rs:SiteConfig, open_with, init_at_with, mount_delegated_at`
- Modify: `rust/tonk-cli/src/bin/tonk.rs:open_selected, use_op, spot_op, join_op, active-context diagnostics`
- Modify: `rust/tonk-cli/tests/spot.rs`
- Modify: `rust/tonk-cli/tests/cli_spot.rs`
- Modify: `rust/tonk-cli/tests/site.rs`

**Interfaces:**
- Consumes: Task 1's selected profile, per-profile `SpotStore`, install-level binding map, and existing `--spot > TONK_SPOT > nearest cwd binding` precedence.
- Produces:

```rust
pub struct ResolvedSpace {
    pub profile: NativeProfileContext,
    pub name: String,
    pub site: PathBuf,
    pub source: spot::Source,
}

pub fn resolve(
    &self,
    flag: Option<&str>,
    env: Option<&str>,
    cwd: Option<&Path>,
) -> Result<ResolvedSpace, ProfileError>;
```

- [ ] Add `it_resolves_a_directory_binding_to_its_profile_even_when_another_profile_is_selected`. Give A and B a `garden`, select B, bind the cwd to A/garden, and assert a bare resolve returns A while `--spot garden` and `TONK_SPOT=garden` resolve B under the existing precedence rules.
- [ ] Add `it_keeps_equal_names_and_canonical_paths_disjoint_between_profiles`. Create `garden` in A and B and assert their `spots.json`, canonical site directories, account directories, and Dialog profile names differ.
- [ ] Add `it_refuses_to_bind_a_space_absent_from_the_selected_profile` and require an error listing the selected profile's available names rather than names from every account.
- [ ] Add a CLI regression that `tonk use garden` writes `{profile, space}` to the install registry and that `tonk spot unbind` removes only the exact cwd binding.
- [ ] Run `cargo test -p tonk-cli --test account_profiles --test spot --test cli_spot`; expect failures because resolution still uses the global store and name-only bindings.
- [ ] Move directory binding lookup/write into `NativeProfileStore`. Keep `SpotStore` responsible only for one profile's named entries and canonical directories.
- [ ] Extend `SiteConfig` with the resolved profile's account-state store/context; remove production calls to `default_config()` after a space has been resolved. `TonkSite::open_with`, `init_at_with`, and delegated mounts must pass that context into `account_authority::wrap` rather than reopening global state.
- [ ] Make `open_selected` resolve once, then open the returned site's profile. Do not consult the selected account again after resolution.
- [ ] Make create, join, pull, remove, list, and backup operations use the chosen profile's `SpotStore`. Register a newly created directory binding in the install registry only after site creation and profile-local registration succeed.
- [ ] Preserve crash ordering: create/adopt the site, save its profile-local `spots.json`, then save the install-level binding. A failure before the last step leaves an unbound but listed space, never a binding to missing data.
- [ ] Run the focused tests above; expect success.

### Task 3: Implement explicit add, use, login, list, status, and logout profile lifecycle

**Files:**
- Modify: `rust/tonk-cli/src/identity.rs:open, exists, reset, profile_dir`
- Modify: `rust/tonk-cli/src/account_session.rs:all state/lock functions`
- Modify: `rust/tonk-cli/src/account.rs:LinkOptions, link, logout, status, provider connections`
- Modify: `rust/tonk-cli/src/account_state.rs:credential operators and state paths`
- Modify: `rust/tonk-cli/src/bin/tonk.rs:AccountCommand, account_op, identity, rendering`
- Modify: `rust/tonk-cli/tests/account_profiles.rs`
- Modify: `rust/tonk-cli/tests/account_interrupt.rs`
- Modify: `rust/tonk-cli/tests/common.rs`

**Interfaces:**
- Consumes: Task 1's pending/selected profile records, existing resumable `PendingLogin`, `ActiveAccount`, detach outbox, and browser handoff.
- Produces: `account add`, `account use`, `account login`, `account list`, profile-scoped `account status/logout`, and compatibility `account link` routing.

- [ ] Add parser tests requiring `account add --label work`, `account use work`, `account login`, and `account list`, while preserving all existing `account link` flags and telemetry classification.
- [ ] Add `it_adds_two_accounts_without_replacing_either_profile`. Authorize roots A and B through two generated profiles, assert distinct profile DIDs/session files/account directories, then switch locally in both directions without HTTP and verify both root records remain unchanged.
- [ ] Add `it_keeps_a_second_profile_when_add_uses_the_same_account_again`. Link two generated profile DIDs to root A, assert both grants remain bound to their own audience/profile, preserve both rows, and print a warning naming the already-present profile rather than attempting to merge or move either grant.
- [ ] Add label validation tests requiring case-insensitive uniqueness, the documented slug alphabet, deterministic `account`/`account-2` defaults, and exact lookup by label or profile ID.
- [ ] Add `it_logs_out_only_the_selected_profile`. Activate both profiles, select A, log out, and assert A has no active session plus one appropriate detach intent while B remains active and unchanged.
- [ ] Add `it_rejects_a_different_root_when_a_rooted_profile_logs_back_in`. A login handoff returning B's root must not mutate A's local-root record, provider record, account descriptor, selected profile, or spaces. Queue/deliver a detach for the mismatched provisional attachment using its own returned grant and print `this profile belongs to <A>; run tonk account add to use another account`.
- [ ] Add `it_resumes_an_interrupted_add_in_the_same_pending_profile`. Interrupt during polling, rerun `account add`/compatibility `link`, and assert the same profile DID, secret, token hash, and state directory are reused rather than creating a third profile.
- [ ] Add `it_lists_profiles_without_contacting_any_provider`, covering selected, signed-in, signed-out, and pending rows with stable labels and roots.
- [ ] Run `cargo test -p tonk-cli --features integration-tests --test account_profiles --test account_interrupt`; expect failures on the single fixed profile and global account-session lock.
- [ ] Change `identity::open/reset/exists` to take `NativeProfileContext`. Keep a narrowly named `open_legacy_profile` helper only for Task 1 migration; `rg 'PROFILE_NAME|SpotStore::open\(' rust/tonk-cli/src` must leave no production account/space path that silently selects legacy state.
- [ ] Scope `ACCOUNT_SESSION_SITE`, its sidecar state file, and `account-session.lock` through the context store. Cross-process shared/exclusive locking remains unchanged within one profile; independent accounts do not block each other's local operations.
- [ ] Implement lifecycle commands. `account use` only changes `profiles.json.selected`; `account logout` uses the selected context; `account login` requires a rooted selected profile; `account add` creates/resumes an unrooted profile before opening the browser.
- [ ] Make compatibility `account link` dispatch to add when no rooted profile is selected and to login otherwise. It must never reproduce the old logout-then-replace-root behavior.
- [ ] Make `tonk identity` report the selected profile and account root. Make `identity --reset` refuse while that profile still owns registered spaces; do not delete another profile or the install registry.
- [ ] Run the focused tests; expect success.

### Task 4: Make the resolved profile the only remote authorization source

**Files:**
- Modify: `rust/tonk-cli/src/account_authority.rs:AccountBoundOperator, active, authorize_guarded, wrap`
- Modify: `rust/tonk-cli/src/site.rs:account_root_prefix_for, recover_prefix, mint_prefix`
- Modify: `rust/tonk-cli/src/invite.rs:join/claim prefix persistence`
- Modify: `rust/tonk-cli/src/account_state.rs:migration and retained delegation paths`
- Modify: `rust/tonk-cli/tests/account_authority.rs`
- Modify: `rust/tonk-cli/tests/site.rs`

**Interfaces:**
- Consumes: `ResolvedSpace.profile`, its profile-scoped account-session store, existing root-to-profile grant validation, and account-root-specific `tonk-space-root-v2/{subject}/{root}` credentials.
- Produces:

```rust
pub async fn load_account_root_prefix_for(
    profile: &Profile,
    operator: &Operator<NativeSpace>,
    subject: &Did,
    account_root: &Did,
) -> Result<DelegationChain>;

pub async fn adopt_account_root_prefix_for(
    profile: &Profile,
    operator: &Operator<NativeSpace>,
    subject: &Did,
    account_root: &Did,
) -> Result<DelegationChain>;
```

- [ ] Replace `it_pushes_a_spot_whose_account_prefix_was_never_stored` with `it_refuses_remote_authorization_without_an_explicit_account_prefix`. Assert `AuthorizeError::UnprovenSubject` and that neither a root-specific prefix credential nor a network request is produced.
- [ ] Keep explicit legacy adoption coverage, renamed `it_adopts_legacy_authority_only_when_requested`, proving `adopt_account_root_prefix_for` can recover/mint once and later strict loads reuse the exact bytes.
- [ ] Add `it_never_uses_the_selected_b_account_for_an_a_space`. Select B globally, resolve an A-bound space, and assert the outgoing proof chain—when A is signed in—terminates at A. Then log out A while leaving B active and assert the same operation is rejected before HTTP rather than using B.
- [ ] Add `it_allows_local_commits_for_a_while_a_is_logged_out`. Commit a fact through A's resolved site with B selected, assert the local revision advances, and assert automatic pull/push reports A as signed out without rolling back the commit.
- [ ] Run `cargo test -p tonk-cli --features integration-tests --test account_authority`; expect the first regression to fail because `authorize_guarded` currently calls the minting `account_root_prefix_for`.
- [ ] Split prefix resolution. `load_account_root_prefix_for` may validate the root-specific credential and migrate an already-valid legacy key for the same root, but may not call `recover_prefix`, `mint_prefix`, `claim`, `delegate`, or save newly minted authority. `adopt_account_root_prefix_for` owns those explicit behaviors.
- [ ] Change `AccountBoundOperator` to carry the resolved profile's account-session store. Preserve its current behavior of discarding historical authorization and rebuilding the outgoing chain, but load only that profile's `ActiveAccount` and strict prefix.
- [ ] Route fresh create, explicit join/pull, and `account migrate` through `adopt_account_root_prefix_for`; route ordinary push, pull, fetch/status, invite sync, account backup, and background reconciliation through strict loading.
- [ ] Run the focused account-authority and site tests; expect success.

### Task 5: Discover and persist provider-matched content-sync defaults per profile

**Files:**
- Create: `rust/tonk-cli/src/deployment.rs`
- Modify: `rust/tonk-cli/src/account_profiles.rs:NativeProfileRecord and deployment persistence`
- Modify: `rust/tonk-cli/src/account.rs:successful add/login completion`
- Modify: `rust/tonk-cli/src/lib.rs:deployment module`
- Modify: `rust/tonk-cli/Cargo.toml:tonk-worker-api dependency`
- Modify: `rust/tonk-cli/tests/account_profiles.rs`

**Interfaces:**
- Consumes: the selected ceremony URL, linked account service URL, `tonk_worker_api::DeploymentConfig`, and the browser convention `ceremony origin + /ucan/`.
- Produces:

```rust
pub struct DeploymentDefaults {
    pub ceremony_origin: url::Url,
    pub access_remote: url::Url,
    pub revocation_relay: url::Url,
}

pub async fn discover(
    account_url: &str,
    expected_account_service: &str,
) -> anyhow::Result<DeploymentDefaults>;
```

- [ ] Add `it_discovers_the_access_remote_from_the_ceremony_deployment`. Serve `/.well-known/tonk`, pass `https://deployment.example/account/link`, and assert access is `https://deployment.example/ucan/`, relay is the typed response value, and the normalized response account-service URL matches the link provider.
- [ ] Add rejection tests for relative/non-HTTP ceremony URLs, userinfo, malformed config, response account service differing by host/path, and a non-loopback HTTP origin. Loopback HTTP remains allowed for tests/local development.
- [ ] Add `it_keeps_login_successful_when_deployment_discovery_is_offline`. The profile retains the ceremony origin, leaves defaults absent, reports `sync defaults: pending`, and retries on explicit reconciliation; it must not fall back silently to production.
- [ ] Run `cargo test -p tonk-cli --features integration-tests --test account_profiles deployment`; expect failure because discovery does not exist.
- [ ] Implement bounded discovery using the ceremony URL's origin and `GET /.well-known/tonk`. Normalize trailing slashes for comparison but preserve validated typed URLs.
- [ ] Persist only operational defaults in `profiles.json`; never place grants, passkey IDs, delegation bytes, or account descriptor bytes there.
- [ ] On later successful discovery, update only that profile's record atomically. An explicit existing space upstream still wins over these defaults.
- [ ] Run the focused discovery tests; expect success.

### Task 6: Reconcile only the owning profile's spaces and prove remote durability before projection

**Files:**
- Create: `rust/tonk-cli/src/account_sync.rs`
- Modify: `rust/tonk-cli/src/remote.rs:profile default setup helpers`
- Modify: `rust/tonk-cli/src/sync.rs:confirmed revision access`
- Modify: `rust/tonk-cli/src/account_spots.rs:profile filtering and projection order`
- Modify: `rust/tonk-cli/src/account_state.rs:profile-scoped retain/push`
- Modify: `rust/tonk-cli/src/auto_sync.rs:post-commit reconciliation`
- Modify: `rust/tonk-cli/src/bin/tonk.rs:login/create/join/account sync triggers and rendering`
- Modify: `rust/tonk-cli/tests/account_spots.rs`
- Modify: `rust/tonk-cli/tests/account_authority.rs`

**Interfaces:**
- Consumes: profile-local `SpotStore`, Task 5 defaults, existing upstream configuration, `sync::{pull,push,status_with_hash}`, strict account authority, account-repository delegation retention, and account-service backup projection.
- Produces:

```rust
pub enum EnrollmentPhase {
    LocalOnly,
    Provisioning,
    PendingPush,
    Connected { confirmed: TreeReference },
    Error { step: &'static str, detail: String },
}

pub struct ReconcileRow {
    pub name: String,
    pub subject: Did,
    pub phase: EnrollmentPhase,
}

pub struct ReconcileReport {
    pub profile: NativeProfileId,
    pub rows: Vec<ReconcileRow>,
}

pub async fn reconcile_profile(
    context: &NativeProfileContext,
) -> ReconcileReport;
```

- [ ] Add `it_reconciles_only_spaces_registered_in_the_requested_profile`. Give A and B one space each, reconcile A, and assert no B site is opened, configured, pushed, retained, or projected.
- [ ] Add `it_preserves_a_custom_upstream`. A space already tracking `custom` must use it and must not create/set the profile's deployment default.
- [ ] Add `it_provisions_pushes_retains_then_projects_in_order`. For a local-only A space with defaults, capture operations and require: configure default remote and upstream; pull/merge when the remote already exists; push local content; record the exact confirmed local revision; retain the existing prefix into A's account repository; push the account repository; then update the account-service projection. A failure before confirmed content push must produce no new projection claiming recovery.
- [ ] Add `it_keeps_offline_and_non_fast_forward_spaces_pending`. Network failure must leave the local revision and configured upstream intact with a per-space error. A non-fast-forward must pull then retry one push; a second conflict remains visible and does not loop.
- [ ] Add `it_resumes_a_logged_out_profiles_pending_change_after_same_root_login`. Commit locally while A is detached, reconcile and observe `PendingPush`, sign A into the same root, reconcile again, and assert the exact revision becomes `Connected`.
- [ ] Add `it_does_not_adopt_unowned_legacy_or_other_profile_spaces`. A profile scan is its own `spots.json` only; no install-wide sweep exists.
- [ ] Run `cargo test -p tonk-cli --features integration-tests --test account_spots --test account_authority`; expect failures because current backup sweeps use the global registry and may project before a confirmed push.
- [ ] Implement one bounded per-space state machine. Continue after individual failures and sort rows by name for deterministic output. Do not persist transient error strings.
- [ ] Store the last confirmed content revision in a root-specific local credential site such as `tonk-space-confirmed-v1/{subject}/{account_root}` only after `sync::push` succeeds. Compare it with the current local revision to derive `PendingPush` without network access.
- [ ] Trigger reconciliation after successful add/login, new-space creation, explicit join/pull, and a committed eval. During automatic paths, print warnings and keep the primary local command successful. Add explicit `tonk account sync` returning nonzero when any row remains `Error`.
- [ ] Change account backup/projection copy and APIs to describe saved access metadata, not content backup. Only a matching confirmed revision may support `recoverable`/safe-delete language.
- [ ] Run the focused tests; expect success.

### Task 7: Expose the resolved profile and pending state clearly without making accounts a local security claim

**Files:**
- Modify: `rust/tonk-cli/src/bin/tonk.rs:account/list/status, spot/list, context, status and failure context rendering`
- Modify: `rust/tonk-cli/src/context.rs:profile/account fields`
- Modify: `rust/tonk-cli/tests/cli_spot.rs`
- Modify: `rust/tonk-cli/tests/context.rs`
- Modify: `rust/tonk-cli/README.md`

**Interfaces:**
- Consumes: `ResolvedSpace`, profile lifecycle status, and `ReconcileRow`.
- Produces: stable text/JSON fields `profile`, `accountRoot`, `signedIn`, `localPresence`, `syncPhase`, and `confirmedRevision` where applicable.

- [ ] Add output regressions proving a bound A space reports `profile: personal`, `account: <A root>`, and `signed in: no` even while B is selected and signed in.
- [ ] Add account-list output covering selected marker, label, root or `pending`, provider state, and number of local spaces; ensure no delegation bytes, credential IDs, secrets, or filesystem-internal profile names are printed.
- [ ] Add status output for `local-only`, `provisioning`, `pending push`, `connected <revision>`, and per-step error. JSON must use separate fields rather than one overloaded `synced` boolean.
- [ ] Add an error-context regression proving a failed remote command prints the resolved space/profile before the actionable `tonk account use/login` guidance.
- [ ] Run `cargo test -p tonk-cli --test cli_spot --test context`; expect failures on missing profile fields.
- [ ] Update renderers and documentation. State explicitly that the same OS user can read local files, profile separation prevents accidental CLI credential mixing, and remote services enforce signed delegation chains.
- [ ] Document account commands, directory-binding semantics, same-name spaces, offline edits after logout, later sync, deployment-default discovery, and the difference between logout, account use, revocation, and the deferred destructive profile-forget operation.
- [ ] Run the focused output tests; expect success.

### Task 8: Verify migration, account switching, and the complete remote boundary

**Files:**
- Modify: `rust/tonk-cli/tests/account_profiles.rs:end-to-end scenario`
- Modify: `rust/tonk-cli/tests/common.rs:two-account fixture`
- Modify: any files above only for defects exposed by the final scenario

**Interfaces:**
- Consumes: Tasks 1–7.
- Produces: one end-to-end proof of the accepted multi-account and offline behavior; no new production interface.

- [ ] Add `it_keeps_two_accounts_and_their_spaces_disjoint_across_logout_switch_offline_edit_and_relogin` with this exact sequence: migrate a legacy A install; add/sign into B; create same-named `garden` in each profile; bind two directories; log out A; select B; commit in A's bound directory; assert B's account grant is never requested and A remains pending; reconcile B and assert only B moves; sign A back into the same root; reconcile A and assert its exact pending revision is confirmed; restart every store/profile handle and assert bindings, selected profile, roots, and confirmed revisions survive.
- [ ] Add a negative end-to-end case that forces the A site through B's profile context and requires local mount/authorization failure without changing either site's branch revision or writing a B-rooted prefix for A's subject.
- [ ] Run `cargo test -p tonk-cli --features integration-tests --test account_profiles --test account_authority --test account_spots --test account_interrupt`; expect all multi-account and remote-boundary tests to pass.
- [ ] Run `cargo test -p tonk-cli`; expect all ordinary library and integration tests to pass.
- [ ] Run `cargo test -p tonk-cli --features integration-tests`; expect all feature-gated native account tests to pass. If localhost binding is denied by the sandbox, rerun with the repository's permitted test environment and report that boundary rather than weakening assertions.
- [ ] Run `cargo fmt --all -- --check`; expect no formatting diff.
- [ ] Run `cargo clippy -p tonk-cli --all-targets --features integration-tests -- -D warnings`; expect no warnings introduced by this change.
- [ ] Run `git diff --check`; expect no whitespace errors.
- [ ] Inspect `git diff --stat` and `git status --short`; confirm only the planned CLI, shared deployment-contract dependency, tests, README, and this plan changed.

## Acceptance criteria

- One install can retain at least two rooted native profiles and switch the selected profile without a browser or network request.
- Each profile has a distinct Dialog profile, account session/lock, account repository directory, spot registry, and canonical spot root.
- Existing installations become the `legacy` profile without moving or deleting data.
- Directory bindings select an exact profile and space. A bound A directory continues using A when B is selected.
- Equal space names in different profiles are supported and unambiguous through directory bindings.
- Logout A leaves A's local spaces editable, blocks A remote requests before HTTP, and leaves B unchanged.
- Local A commits made while A is logged out synchronize after A signs back into the same immutable root.
- A handoff returning a different root cannot overwrite a rooted profile and directs the user to `account add`.
- Neither selecting B nor logging into B scans, retains, backs up, configures, or publishes A's spaces.
- Generic remote authorization cannot mint a missing space-to-account prefix. Only explicit create/join/pull/adopt paths can establish one.
- Automatic convergence preserves custom upstreams, provisions only missing remotes from provider-matched deployment defaults, proves a content push before projecting recoverability, and exposes offline/error states.
- The CLI states the honest boundary: profiles are operational and credential isolation inside Tonk, not encryption against the local OS user.

## Explicitly deferred

- Encrypting profile credentials or space data at rest.
- Protecting against a malicious process or user with access to the same OS account.
- Signed per-revision author/account provenance. If Tonk later needs to prove which account context produced every offline revision—not merely which authorized profile pushed it—that requires a separately specified signed revision envelope and remote validation rule.
- Account-level archive/removal tombstones and the browser's removed-space resurrection fix.
- Deleting a profile or its local space data (`account forget`); the first release should only add, select, login, and logout profiles.
- Duplicating/deduplicating immutable block storage across profiles. Initial profile stores remain physically separate for understandable failure behavior.
- Renaming all public `spot` commands, environment variables, serialized files, or protocol identifiers to `space`.
