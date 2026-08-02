# Account spots CLI implementation plan

**Goal:** Let an account-linked native CLI list account-backed spots, pull one remote spot into its local registry, and automatically back up every pullable spot the CLI already knows so browser and CLI devices converge on the same account spot inventory.

**Approach:** Promote the browser worker's private space-backup JSON into a provider-neutral `tonk_account::backup` contract, while preserving the existing snake_case artifact shape so deployed backups remain readable. Keep immutable backup blobs behind the existing `/chains/put` and `/chains/get` endpoints, add a subject-level head index plus `/chains/spots` inventory endpoint so one account spot has one current record, and synthesize explicit legacy/ambiguous rows for unindexed blobs. Reuse that contract from the worker and a new native `account_spots` module; native pull mounts the backed-up repository without inventing invitation or roster facts, then registers it under a validated local slug.

**Constraints:**
- This work is stacked on PR #673 (`fix/shared-account-handoff-contract`); implementation must start from that branch and open its PR against `fix/shared-account-handoff-contract`, not `staging`.
- Preserve `POST /chains/put`, `/chains/list`, and `/chains/get`, their HTTP statuses, their current UCAN commands, and `/chains/list`'s `Vec<String>` response. Deployed workers must continue to operate during rollout.
- Preserve the existing backup artifact's snake_case keys: `chain_hex`, `remote_url`, and `revocation_url`. Additive `name` deserialization must accept legacy artifacts where the field is absent. Do not add `deny_unknown_fields`.
- A reusable backup contains only the `space → … → account-root` delegation prefix. It must never include the current `root → device` link or a device/session suffix.
- `tonk account spots` and `tonk account spots list` are aliases. They list remote account spots and mark subjects already registered locally. `tonk account spots pull <subject> [--name <slug>]` pulls exactly one subject.
- Pull does not bind the current working directory. It writes only the canonical `SpotStore::canonical_site(name)` directory and an unbound `spots.json` entry.
- Without `--name`, pull uses the stored UI/CLI spot name only when it is a valid, unused CLI slug (`[a-z0-9][a-z0-9-_]*`). Missing legacy names, arbitrary UI labels, or occupied names fail before local mutation and explicitly ask for `--name`; do not slugify, overwrite, or silently suffix.
- A subject already registered locally is not mounted twice. Listing/pull must identify local spots by repository subject DID, not by local registry name.
- Account restore must not assert `Membership`, `MemberRole`, `MemberName`, `Invitation`, or `InvitedVia`; those facts are authoritative on the synced repository. Invite claim retains its existing roster/provenance writes after using the lower-level mount helper.
- A backup is pullable only with a usable sync remote. A temporarily failing initial pull mirrors `tonk join`: retain the mounted/registered spot and return a warning so `tonk pull` can retry. Missing or malformed remote metadata fails before registration.
- New account-service writes update one subject head. Repeated identical writes are idempotent; an unnamed retry must not erase an existing named head. Legacy conflicts without a head are reported as ambiguous rather than resolved by hash-map or R2 listing order.
- Existing unnamed, unindexed browser backups remain listable by subject and pullable with explicit `--name`. Malformed non-spot objects already accepted by the generic chain store are skipped during legacy discovery and must not poison other spots.
- Automatic native backup is best-effort: account-service failure cannot turn successful link, create, join, remote setup, pull/push, or evaluated mutation into failure. Only spots with an actual upstream remote are uploaded; local-only spots are retried when an upstream is later configured.
- Use the remote tracked by local `main` (`remote::upstream_remote` then `remote::find`), never an arbitrary lone remote. Prefer the synced `RepositoryName` as backup metadata and fall back to the CLI registry name when content has no name.
- Preserve the operator derivation context `b"slide"`, profile layout, canonical spot storage, account repository isolation, invitation behavior, account-service authorization, revocation relay metadata, and existing remote names (`origin`).
- Do not add account-spot deletion, bulk pull, cwd binding, silent name normalization, a second account provider, or a CLI-only backup format.
- No version bump belongs in this feature PR. Workspace releases remain separate `release/*` changes.
- The account service must be deployable before new clients use `/chains/spots`; successful `/chains/list` responses advertise `X-Tonk-Account-Spots: v1` (and expose it through CORS), and new worker and CLI readers use the already-returned keys with `/chains/get` when that capability header is absent.

## File map

- `rust/tonk-account/src/backup.rs`: canonical account spot backup artifact, inventory DTO, validation, and shared credential-key constants.
- `rust/tonk-account/src/lib.rs`: expose the namespaced `backup` contract.
- `rust/tonk-account-service/src/chains.rs`: extend storage abstraction with subject-head operations.
- `rust/tonk-account-service/src/chains/r2.rs`: persist/paginate immutable blobs and subject heads in separate R2 prefixes.
- `rust/tonk-account-service/src/core/backup.rs`: index valid spot artifacts on put and produce current/legacy/ambiguous inventory rows.
- `rust/tonk-account-service/src/core.rs`: expose account-spot inventory core logic.
- `rust/tonk-account-service/src/handlers/chains.rs`: add the authorized `/chains/spots` Worker handler without changing existing chain routes.
- `rust/tonk-account-service/src/handlers.rs`: expose the wasm spot-inventory handler.
- `rust/tonk-account-service/src/helpers/server.rs`: mirror `/chains/spots` in the native integration server.
- `rust/tonk-account-service/src/lib.rs`: register Worker `/chains/spots` POST/OPTIONS routes.
- `rust/tonk-account-service/tests/service.rs`: exercise authorized put, inventory, update, legacy, ambiguity, and account isolation over HTTP.
- `rust/tonk-worker/src/router/account_backup.rs`: use the shared artifact, carry repository names, update subject heads through existing put, and read semantic inventory with legacy fallback.
- `rust/tonk-worker/src/router/restore.rs`: restore inventory rows by subject/head while preserving per-spot failure isolation.
- `rust/tonk-worker/src/router/repository.rs`: use shared prefix keys and refresh backup metadata after a UI repository rename.
- `rust/tonk-worker/src/router/join.rs`: persist the accepted root-ending prefix and pass the actual repository name to backup.
- `rust/tonk-cli/src/account.rs`: expose one internal authenticated account-service connection/request primitive for account spots.
- `rust/tonk-cli/src/account_spots.rs`: native list, pull, validation, mount orchestration, backup reconciliation, and best-effort upload.
- `rust/tonk-cli/src/site.rs`: persist/extract account-root prefixes and mount a delegated subject without roster writes.
- `rust/tonk-cli/src/invite.rs`: reuse the delegated mount helper, then retain invite-specific roster/provenance writes.
- `rust/tonk-cli/src/spot.rs`: atomically register an existing canonical site without a cwd binding.
- `rust/tonk-cli/src/auto_sync.rs`: refresh account backup after successful automatic synchronization.
- `rust/tonk-cli/src/lib.rs`: expose `account_spots` for the binary and integration tests.
- `rust/tonk-cli/src/bin/tonk.rs`: add `account spots [list|pull]`, output rendering, and best-effort backup hooks after successful native operations.
- `rust/tonk-cli/Cargo.toml`: register the new `account_spots` integration test target; no runtime dependency is expected.
- `rust/tonk-cli/tests/common.rs`: account-linked profile/account-service/access-service fixtures shared by account-spots integration tests.
- `rust/tonk-cli/tests/account_spots.rs`: native list/pull and bidirectional backup integration coverage.
- `Cargo.lock`: change only if Cargo records a genuinely required dependency change; none is planned.

### Task 1: Define and validate the shared account spot backup contract

**Files:**
- Create: `rust/tonk-account/src/backup.rs`
- Modify: `rust/tonk-account/src/lib.rs:module declarations`
- Test: `rust/tonk-account/src/backup.rs:tests`

**Interfaces:**
- Consumes: existing stored JSON shaped as `{"chain_hex":"…","remote_url":"…","revocation_url":"…"}` and `DelegationChain` bytes whose audience is the account root.
- Produces:

```rust
pub const SPACE_ROOT_SITE_PREFIX: &str = "tonk-space-root-v1/";
pub const ACCOUNT_SPOT_BACKUP_MARKER_PREFIX: &str = "tonk-account-spot-backup-v1/";

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AccountSpotBackup {
    pub chain_hex: String,
    pub remote_url: Option<String>,
    #[serde(default)]
    pub revocation_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountSpotSummary {
    pub subject: String,
    pub key: Option<String>,
    pub name: Option<String>,
    pub remote_url: Option<String>,
    pub revocation_url: Option<String>,
    pub ambiguous: bool,
}

pub struct ValidatedAccountSpot {
    pub subject: dialog_varsig::Did,
    pub chain: dialog_ucan_core::DelegationChain,
}

impl AccountSpotBackup {
    pub async fn validate_for(
        &self,
        account_root: &dialog_varsig::Did,
    ) -> Result<ValidatedAccountSpot, AccountSpotBackupError>;
}
```

- [ ] Add `it_reads_legacy_unnamed_backups_and_round_trips_named_backups` before the types exist. Assert the legacy snake_case fixture yields `name == None`, a named artifact emits only `chain_hex`, `remote_url`, `revocation_url`, and `name`, and `AccountSpotSummary` emits camelCase `remoteUrl`/`revocationUrl`. Run `cargo test -p tonk-account backup::tests::it_reads_legacy_unnamed_backups_and_round_trips_named_backups`; expect unresolved-type compilation failures.
- [ ] Implement the documented DTOs/constants without top-level re-exports and rerun the focused test; expect one pass.
- [ ] Add `it_accepts_only_a_verified_space_to_account_root_prefix`. Independently build a valid `space → account-root` specific-subject chain, then assert rejection for malformed hex/container, a subject-open chain, a subject differing from the issuer space, a wrong account-root audience, a corrupted signature, an empty `name: Some("")`, and malformed remote/revocation URLs. Run it before `validate_for`; expect a compile failure for the missing method/error type.
- [ ] Implement `validate_for`: decode hex, parse and cryptographically verify the complete chain with `Ed25519KeyResolver`, require `subject == Some(chain.issuer())`, require `audience == account_root`, parse every present URL, and reject empty names. Return stable typed `AccountSpotBackupError` variants suitable for service/client context; do not validate the name as a CLI slug here because UI display names are deliberately broader.
- [ ] Run both focused tests, then `cargo test -p tonk-account`; expect all shared-contract tests to pass.

### Task 2: Add subject heads and an account spot inventory to the account service

**Files:**
- Modify: `rust/tonk-account-service/src/chains.rs:ChainStore, MemoryChainStore`
- Modify: `rust/tonk-account-service/src/chains/r2.rs:R2ChainStore`
- Modify: `rust/tonk-account-service/src/core/backup.rs:put_chain and inventory helpers`
- Modify: `rust/tonk-account-service/src/core.rs:module declarations`
- Modify: `rust/tonk-account-service/src/handlers/chains.rs:put/list/get adapters and new inventory adapter`
- Modify: `rust/tonk-account-service/src/handlers.rs:module declarations`
- Modify: `rust/tonk-account-service/src/helpers/server.rs:route table and chain routes`
- Modify: `rust/tonk-account-service/src/lib.rs:Worker routes`
- Modify: `rust/tonk-account-service/tests/service.rs:chain backup lifecycle`
- Test: `rust/tonk-account-service/src/core/backup.rs`
- Test: `rust/tonk-account-service/tests/service.rs`

**Interfaces:**
- Consumes: `AccountSpotBackup::validate_for`, existing generic chain blobs, existing `/chains/put|list|get`, and account-authorized invocations.
- Produces:

```rust
// Additive ChainStore methods; immutable blobs and heads use separate namespaces.
async fn put_spot_head(
    &self,
    root_did: &str,
    subject_key: &str,
    blob_key: &str,
) -> Result<(), ChainError>;
async fn spot_head(
    &self,
    root_did: &str,
    subject_key: &str,
) -> Result<Option<String>, ChainError>;
async fn list_spot_heads(
    &self,
    root_did: &str,
) -> Result<Vec<(String, String)>, ChainError>;

pub async fn put_chain_and_index_spot<C: ChainStore>(
    chains: &C,
    account: &Account,
    bytes: &[u8],
) -> Result<String, CeremonyError>;

pub async fn list_account_spots<C: ChainStore>(
    chains: &C,
    account: &Account,
) -> Result<Vec<AccountSpotSummary>, CeremonyError>;
```

- [ ] Extend the existing core tests first with `it_indexes_one_current_head_per_spot_subject`. Store a valid named artifact, store an updated named artifact for the same subject, and assert inventory has one row pointing at the second blob; repeat identical put and assert blob/head counts do not grow. Add an unnamed retry and assert it does not replace the named head. Run `cargo test -p tonk-account-service --features helpers core::backup::tests::it_indexes_one_current_head_per_spot_subject`; expect compilation failure for missing head methods/inventory functions.
- [ ] Extend `ChainStore` and `MemoryChainStore`. Keep existing blob keying unchanged. Derive `subject_key` as lowercase BLAKE3 hex of the canonical subject DID bytes, so object names contain no DID punctuation. Sort memory-store blob/head listings for deterministic tests.
- [ ] Implement R2 heads under `spot-heads/{root_did}/{subject_key}` while immutable blobs remain `chains/{root_did}/{blob_key}`. Update both R2 list loops to continue while `Objects::truncated()` using `Objects::cursor()` and `Bucket::list().cursor(cursor)`; never assume one R2 page is complete.
- [ ] Implement `put_chain_and_index_spot`: always preserve generic content-addressed storage; if bytes structurally deserialize as `AccountSpotBackup`, require `validate_for(account.root_did)` before accepting the put, then update its subject head. Before replacing a named head with an unnamed incoming artifact, fetch the current head artifact and retain the named head. Bytes that are not structurally an account-spot artifact remain valid generic chain blobs and receive no head.
- [ ] Implement deterministic inventory. Read/validate every head and fail with `Internal` if a head points to missing/corrupt data. Then scan legacy blob keys not selected by a head: skip non-artifact/malformed generic objects, group valid artifacts by subject, synthesize one unnamed/non-ambiguous row when exactly one legacy candidate exists, and synthesize `key: None, ambiguous: true` when materially different candidates exist. A real head always wins over legacy candidates. Sort rows by subject.
- [ ] Rerun the focused core test and add `it_reports_legacy_and_ambiguous_spots_without_poisoning_valid_rows`, including one arbitrary non-JSON chain blob, one valid legacy artifact, and two conflicting legacy artifacts for another subject. Expect the non-JSON blob to disappear, the single legacy row to be pullable, and the conflict to be explicit/without a key.
- [ ] Add `POST /chains/spots` and OPTIONS to both Worker and native helper servers. Authorize command `account/chain/spots`; return `Vec<AccountSpotSummary>`. Mark successful `/chains/list` responses with `X-Tonk-Account-Spots: v1` and include that name in `Access-Control-Expose-Headers`, while preserving the list JSON shape. Change `/chains/put` adapters to call `put_chain_and_index_spot`, while `/chains/get` stays wire-compatible.
- [ ] Update `service.rs` to use production invocation builders for: named put → one inventory row; renamed put → same subject/new name and one row; unnamed retry → named row retained; get through returned key → exact artifact bytes; another account → no row; revoked device → authorization rejection. Add a legacy blob through the unchanged generic core/helper path and assert it appears unnamed. Run `cargo test -p tonk-account-service --features helpers --test service`; expect all HTTP lifecycle tests to pass.
- [ ] Run `cargo check -p tonk-account-service --target wasm32-unknown-unknown` and `nix build .#tonk-account-service --no-link`; expect the production Worker routes and R2 implementation to compile.

### Task 3: Migrate browser backup and restore to named subject inventory

**Files:**
- Modify: `rust/tonk-worker/src/router/account_backup.rs:ClaimBackup, transport, dispatch and call sites`
- Modify: `rust/tonk-worker/src/router/restore.rs:try_restore_spaces, restore_one`
- Modify: `rust/tonk-worker/src/router/repository.rs:SPACE_ROOT_SITE_PREFIX, run_rename_repository, owned-space backup`
- Modify: `rust/tonk-worker/src/router/join.rs:durable join backup and prefix persistence`
- Test: `rust/tonk-worker/src/router/account_backup.rs:tests`
- Test: `rust/tonk-worker/src/router/restore.rs:tests`
- Test: `rust/tonk-worker/src/router/join.rs:tests`
- Test: `rust/tonk-worker/src/router/repository.rs:tests`

**Interfaces:**
- Consumes: shared `AccountSpotBackup`, `AccountSpotSummary`, `/chains/spots`, and existing `/chains/*` fallback routes.
- Produces:

```rust
async fn list_backed_up_spots(...) -> Result<Vec<AccountSpotSummary>, TonkWorkerError>;
async fn get_backed_up_spot(..., key: &str) -> Result<AccountSpotBackup, TonkWorkerError>;
async fn back_up_subject(tonk: &TonkState, subject: &Did) -> Result<(), TonkWorkerError>;
```

- [ ] Replace the temporary dispatch counter test with/extend it by `it_builds_a_named_root_ending_backup_for_a_durable_join`: after a durable join, capture the artifact before HTTP dispatch and assert its `name` equals the repository's `RepositoryName`, its chain subject equals the joined repository, and its audience is the account root rather than the worker device. Run the focused wasm test before production changes; expect failure because `ClaimBackup` has no name/shared validation and joined prefixes are not persisted under the shared key.
- [ ] Remove private `ClaimBackup` in favor of `tonk_account::backup::AccountSpotBackup`. Persist every accepted durable join's exact claimed root-ending chain under `SPACE_ROOT_SITE_PREFIX + subject`; retain owned-space persistence under the same unchanged key.
- [ ] Add a content-name helper that queries `RepositoryName` on `main` for the subject and returns `None` when absent. Thread that name into both joined and owned backups. Do not use a DID-key fallback as the stored name.
- [ ] Resolve automatic backup against the actual UCAN remote tracked by `main`, independently of invite readiness: require an access URL, carry the revocation relay as optional metadata, and skip only when no usable upstream exists. A synced pre-relay spot must back up with `revocation_url: None`; invite minting still requires a relay.
- [ ] Keep uploads on `POST /chains/put`/`account/chain/put`; the new service indexes valid artifacts automatically. Preserve fire-and-forget/best-effort behavior and ensure identical artifacts remain content-address-idempotent.
- [ ] Discover semantic inventory support through the existing CORS-enabled `/chains/list`: read its unchanged `Vec<String>` body and the exposed `X-Tonk-Account-Spots: v1` response header. Call `/chains/spots` only when advertised; otherwise fetch the already-returned keys through `/chains/get` and validate/group client-side with the same rules (one candidate is usable; conflicting candidates are ambiguous). Do not probe the new route or fall back for authorization, transport, or 5xx failures.
- [ ] Change restore to iterate inventory rows, skip/log `ambiguous` rows without aborting others, fetch by the selected key, validate the returned subject, and retain the existing no-roster mount path and per-spot failure isolation. Artifact `name` remains metadata only; restored synced repository content remains authoritative in the UI.
- [ ] After a successful `run_rename_repository` commit, call `back_up_subject` best-effort so a new UI name advances the account spot head. The rename itself remains successful if backup fails.
- [ ] Add/adjust worker tests for: absent capability never requests `/chains/spots`, advertised capability does; a synced spot without a relay backs up with `revocation_url: None` while a truly missing upstream skips; old unnamed artifact restore; new named artifact restore; ambiguous row skipped while another restores; rename dispatch carries the new name; and one bad artifact does not stop another. Every wasm test that replaces `fetch` or helper globals must hold a failure-safe RAII guard that restores/deletes them on panic or early return. Run focused tests, then `cargo test -p tonk-worker --target wasm32-unknown-unknown --lib -- --nocapture`; expect all worker wasm tests to pass in Chrome.

### Task 4: Add a native no-roster delegated mount and recover existing prefixes

**Files:**
- Modify: `rust/tonk-cli/src/site.rs:bootstrap_repository and new delegated mount/prefix helpers`
- Modify: `rust/tonk-cli/src/invite.rs:claim mount sequence`
- Modify: `rust/tonk-cli/src/spot.rs:existing-site registration helper`
- Test: `rust/tonk-cli/tests/site.rs`
- Test: `rust/tonk-cli/tests/spot.rs`

**Interfaces:**
- Consumes: a validated root-ending `DelegationChain`, canonical site path, `SiteConfig`, and existing invite claim flow.
- Produces:

```rust
pub async fn mount_delegated_at(
    root: &Path,
    chain: DelegationChain,
    config: SiteConfig,
) -> anyhow::Result<TonkSite>;

pub async fn account_root_prefix(
    site: &TonkSite,
    account_root: &Did,
) -> anyhow::Result<DelegationChain>;

pub fn register_existing_unbound(
    store: &SpotStore,
    name: &str,
    site: &Path,
) -> Result<(), SpotError>;
```

- [ ] Add `it_mounts_a_root_delegated_subject_without_inventing_roster_facts`. Build a `space → root` chain, call the not-yet-existing `mount_delegated_at`, assert the resulting repository DID equals the space DID, the exact prefix is persisted under `SPACE_ROOT_SITE_PREFIX + subject`, and meta/content queries contain no membership, role, member-name, invitation, or provenance rows. Run the focused site test; expect a compile failure for the missing helper.
- [ ] Extract the common non-roster part of `invite::claim` into `mount_delegated_at`: refuse an existing target directory, create the normal `slide` operator, save the supplied delegation, persist its exact bytes under the shared prefix key, provision verifier-only `main`, and return an opened `TonkSite`. It must not configure a remote or write roster facts.
- [ ] Refactor `invite::claim` to call the helper after invite claim, then perform its current invitation/membership/provenance writes, remote configuration, and best-effort pull unchanged. Run the existing invite claim tests before and after; they are characterization coverage and must remain green.
- [ ] During new native repository bootstrap, persist the freshly minted `space → account-root` chain under the same shared prefix key before saving it into profile access.
- [ ] Implement `account_root_prefix`: load/validate the persisted prefix first. For pre-feature CLI sites where it is absent, call `profile.access().prove(&site.repository)`, rebuild the proof sequence only through the delegation whose audience equals `account_root`, validate that prefix through the shared contract rules, persist it, and return it. Never serialize proofs after the account-root audience.
- [ ] Add `it_recovers_a_pre_feature_prefix_from_profile_authority` by creating a site, deleting only the prefix credential while leaving certificate access intact, and asserting recovery ends at root and becomes persisted. This test protects backfill of spots the CLI already knew before this feature.
- [ ] Add `it_registers_an_existing_site_without_binding_the_cwd` for `register_existing_unbound`: validate the slug, canonicalize the site, reload immediately before save, reject an occupied name, preserve unknown registry fields, and insert no binding. Run `cargo test -p tonk-cli --test site --test spot`; expect all site/spot tests to pass.

### Task 5: Add native account spot inventory and CLI listing

**Files:**
- Create: `rust/tonk-cli/src/account_spots.rs`
- Modify: `rust/tonk-cli/src/account.rs:linked_chain, stored provider and authenticated POST helpers`
- Modify: `rust/tonk-cli/src/lib.rs:module declarations and crate docs`
- Modify: `rust/tonk-cli/src/bin/tonk.rs:AccountCommand and account_op`
- Modify: `rust/tonk-cli/Cargo.toml:test targets`
- Create: `rust/tonk-cli/tests/account_spots.rs`
- Modify: `rust/tonk-cli/tests/common.rs:account spot fixtures`
- Test: `rust/tonk-cli/tests/account_spots.rs`

**Interfaces:**
- Consumes: attached provider URL, root→device chain, account spot summary endpoint/fallback, and local `SpotStore`.
- Produces:

```rust
pub struct AccountSpotRow {
    pub subject: String,
    pub remote_name: Option<String>,
    pub local_name: Option<String>,
    pub ambiguous: bool,
    pub pullable: bool,
}

pub async fn list(
    profile: &Profile,
    store: &SpotStore,
) -> anyhow::Result<Vec<AccountSpotRow>>;
```

CLI grammar:

```text
tonk account spots
tonk account spots list
tonk account spots pull <SUBJECT> [--name <SLUG>]
```

- [ ] Add parser tests in `bin/tonk.rs` proving bare `account spots` and explicit `account spots list` produce the same list variant, while pull captures a full DID and optional name. Run `cargo test -p tonk-cli --bin tonk account_spots`; expect failure because the nested command enums do not exist.
- [ ] Refactor `account.rs` only enough to expose an internal `AccountConnection` containing the stored provider URL, root DID, and parsed linked chain, plus a signed POST helper that returns the raw `reqwest::Response` so account-spots code can inspect capability headers. Existing devices/revoke behavior and error wording remain unchanged.
- [ ] Implement remote inventory in `account_spots.rs`: sign `account/chain/list` first, preserve its `Vec<String>` body, and call `account/chain/spots` only when `X-Tonk-Account-Spots: v1` is advertised; when absent, use those keys with per-key `account/chain/get` to validate/group legacy artifacts. Sort by subject and preserve explicit ambiguity.
- [ ] Build local status by loading `spots.json` and opening each registered site with the normal profile config to map repository subject → local registry name. A broken local entry may be reported as a warning but must not hide remote rows; two local names resolving to one subject are a local-corruption error rather than an arbitrary choice.
- [ ] Render each row as one tab-separated line with stable columns `STATE\tNAME\tSUBJECT`, where `STATE` is `local`, `remote`, or `ambiguous`, `NAME` is the remote name, else local name, else `-`. Bare and explicit list output must be byte-identical; an empty account prints `(no account spots backed up)`.
- [ ] In `account_spots.rs` unit tests, cover absent-capability fallback without a `/chains/spots` request, advertised-capability inventory, one malformed legacy object beside a valid one, legacy conflict ambiguity, and local subject detection. Then add an HTTP integration fixture using `AccountServer` and a real root/device invocation; assert named, unnamed, local, and account-isolated rows. Run `cargo test -p tonk-cli --features integration-tests --test account_spots list -- --nocapture`; expect all list tests to pass.

### Task 6: Pull one account spot into canonical local storage

**Files:**
- Modify: `rust/tonk-cli/src/account_spots.rs:pull and restore helpers`
- Modify: `rust/tonk-cli/src/bin/tonk.rs:AccountSpotsCommand::Pull rendering`
- Modify: `rust/tonk-cli/tests/account_spots.rs:pull lifecycle`
- Test: `rust/tonk-cli/tests/account_spots.rs`

**Interfaces:**
- Consumes: exact subject, optional explicit slug, one non-ambiguous summary/key, validated artifact, `mount_delegated_at`, remote helpers, and `register_existing_unbound`.
- Produces:

```rust
pub struct PullOutcome {
    pub subject: String,
    pub name: String,
    pub site: PathBuf,
    pub already_local: bool,
    pub warning: Option<String>,
}

pub async fn pull(
    profile: &Profile,
    store: &SpotStore,
    subject: &str,
    requested_name: Option<&str>,
) -> anyhow::Result<PullOutcome>;
```

- [ ] Add `it_requires_an_explicit_name_for_unnamed_invalid_or_occupied_remote_names`. Cover: unnamed legacy row; UI name `"My Garden"`; valid remote name already occupied by another subject; invalid explicit name; and explicit occupied name. In every case assert the canonical target path and registry remain absent/unchanged and the error says `pass --name`. Run the focused test before `pull`; expect a compile failure for the missing function.
- [ ] Implement lookup by exact parsed subject DID. Resolve and validate the account inventory row before considering local registration, so an unbacked local subject remains unknown and ambiguous/unselected rows remain errors. If a valid account spot is already local, return `already_local: true` with its existing local name and exact registered `SpotEntry` path (including adopted noncanonical sites), and do not fetch/mount again.
- [ ] Select/validate the local name before fetching artifact: explicit name wins; otherwise require the summary's stored name. Apply `spot::validate_name`, reject occupied names, and require the canonical target path not to exist. Never slugify or delete.
- [ ] Fetch `/chains/get` using the summary key, deserialize and `validate_for` the connection root, require returned subject to equal the requested subject, and require a parseable `remote_url`. Reject these failures before registry mutation.
- [ ] Call `mount_delegated_at`, configure `origin` with the artifact remote and relay, set `main` upstream, and attempt `sync::pull`. Register through `register_existing_unbound` only after mount/remote setup succeeds. If initial pull alone fails, retain/register the spot and put the exact diagnostic in `warning`, matching join's recovery instruction.
- [ ] Add end-to-end integration coverage using `AccessServiceAddress` plus `AccountServer`: publish content/name to a real access service, store its root-ending backup, pull it into an empty isolated store, assert repository DID and content match, `origin` tracks the expected subject, the registry has one unbound canonical entry, and a second pull is an idempotent already-local result. Add a remote-outage case proving warning + retained registration.
- [ ] Run `cargo test -p tonk-cli --features integration-tests --test account_spots pull -- --nocapture`; expect all pull tests to pass.

### Task 7: Back up CLI-known spots automatically in both directions

**Files:**
- Modify: `rust/tonk-cli/src/account_spots.rs:back_up_site, back_up_registered`
- Modify: `rust/tonk-cli/src/auto_sync.rs:successful sync hook`
- Modify: `rust/tonk-cli/src/bin/tonk.rs:account link, spot new, join, remote add/set-upstream, push/pull hooks`
- Modify: `rust/tonk-cli/tests/account_spots.rs:backup reconciliation`
- Modify: `rust/tonk-cli/tests/cli_spot.rs:primary-command output/failure behavior if needed`
- Test: `rust/tonk-cli/tests/account_spots.rs`

**Interfaces:**
- Consumes: local registry name/site, synced `RepositoryName`, actual upstream remote, `account_root_prefix`, and `/chains/put`.
- Produces:

```rust
pub async fn back_up_site(
    registry_name: &str,
    site: &TonkSite,
) -> Result<BackupOutcome, anyhow::Error>;

pub async fn back_up_registered(
    profile: &Profile,
    store: &SpotStore,
) -> Vec<BackupWarning>;
```

- [ ] Add `it_reconciles_owned_joined_and_pre_feature_spots_without_failing_primary_work`. The fixture must include: a newly created owned spot; a joined spot; a pre-feature spot with its prefix marker removed; a local-only spot without upstream; and an account-service failure. Assert the first three produce valid root-ending artifacts under one subject head each, local-only is skipped until remote setup, and simulated upload failure is a warning while the successful local operation remains successful. Run before backup functions/hooks; expect missing-function failures.
- [ ] Implement `back_up_site`: no-op when no provider is attached; resolve only the actual `main` upstream (`upstream_remote` then `find`); query synced `RepositoryName` and fall back to `registry_name`; extract/recover the account-root prefix; construct/validate `AccountSpotBackup`; serialize it; derive its content key; compare with `ACCOUNT_SPOT_BACKUP_MARKER_PREFIX + subject` in the profile credential store; skip unchanged payloads; POST through `account/chain/put`; persist the marker only after success.
- [ ] Implement `back_up_registered`: reload the registry, open each site, call `back_up_site`, and return per-name warnings without stopping the sweep. Missing upstream is a normal skipped outcome, not a warning.
- [ ] Add best-effort hooks only after successful primary work: after account link, sweep all registered spots; after `spot new`/adopt, back up that site; after successful join registration, back up the joined site; after remote add's automatic upstream and explicit set-upstream, retry that site; after manual pull/push, back up that site; after successful auto-sync/eval, back up that site. Preserve the resolved registry name at call sites that currently discard it.
- [ ] Every hook must print at most a warning to stderr and preserve the primary command's exit code/output. Do not upload on failed create/join/sync, and do not upload local-only spots before an upstream exists.
- [ ] Add tests proving: account link sweeps all usable registry entries; UI `RepositoryName` beats a differing CLI registry alias in artifact metadata; fallback registry name is used when content is unnamed; unchanged markers suppress a second HTTP put; setting the first upstream makes a previously local-only spot appear in account inventory; and account-service 5xx does not change create/join/sync success.
- [ ] Run `cargo test -p tonk-cli --features integration-tests --test account_spots backup -- --nocapture`, `cargo test -p tonk-cli --test cli_spot`, and then `cargo test -p tonk-cli`; expect all native CLI tests to pass.

### Task 8: Verify native, wasm, ChromeDriver, and production builds

**Files:**
- Modify: none unless a verification command exposes a defect in the planned files.

**Interfaces:**
- Consumes: completed shared contract, service inventory, worker compatibility, native list/pull, and backup hooks.
- Produces: fresh evidence for a stacked PR against #673.

- [ ] Run `cargo fmt --all -- --check`; expect no output.
- [ ] Run `git diff --check`; expect no whitespace errors.
- [ ] Run `cargo test -p tonk-account`; expect all shared contract tests to pass.
- [ ] Run `cargo test -p tonk-account-service --features helpers --test service`; expect the existing account lifecycle and new subject-inventory lifecycle to pass.
- [ ] Run `cargo check -p tonk-account-service --target wasm32-unknown-unknown`; expect the Worker adapters and R2 head implementation to compile.
- [ ] Run `nix build .#tonk-account-service --no-link`; expect the production Worker artifact to build.
- [ ] Run `cargo test -p tonk-cli`; expect all native CLI unit/integration tests not gated by live integration features to pass.
- [ ] Run `cargo test -p tonk-cli --features integration-tests --test account_spots -- --nocapture`; expect live local account/access-service list, pull, and backup tests to pass.
- [ ] Run `cargo test -p tonk-worker --target wasm32-unknown-unknown --lib -- --nocapture`; expect all worker wasm tests to pass in headless Chrome.
- [ ] Run `cargo test -p tonk-ui --target wasm32-unknown-unknown --lib -- --nocapture`; expect all stacked UI wasm tests from #673 to remain green.
- [ ] Run `cargo check -p tonk-ui --target wasm32-unknown-unknown`; expect success.
- [ ] Run `cargo clippy -p tonk-account -p tonk-account-service -p tonk-cli -p tonk-worker --all-targets --no-deps -- -D warnings`; if target-specific code requires separate invocations, run native packages first and `cargo clippy -p tonk-account-service -p tonk-worker --target wasm32-unknown-unknown --all-targets --no-deps -- -D warnings`. Do not fix unrelated workspace lint findings.
- [ ] Run `nix build .#tests-web-debug --no-link` and then execute the archive through `nix develop -c test:web:debug`; expect all wasm tests to execute successfully.
- [ ] Explicitly verify ChromeDriver-backed WebAuthn on the stacked handoff work with `nix develop -c cargo test -p tonk-ui --features web-integration-tests identity::tests::it_builds_a_root_signed_cli_handoff -- --nocapture`; expect ChromeDriver to start, `completeLink` to run, and the test to pass. Also run `identity::tests::it_builds_a_root_signed_account_creation_in_one_browser_ceremony` the same way; do not substitute compile-only evidence.
- [ ] Run `nix build .#tonk-cli --no-link`, `nix build .#tonk-ui --no-link`, and `cargo test -p tonk-ui --features web-integration-tests --no-run`; expect all production/host targets to build.
- [ ] Inspect `rg -n "struct ClaimBackup|SPACE_ROOT_SITE_PREFIX|/chains/spots|AccountSpotsCommand|account_root_prefix|mount_delegated_at" rust --glob '*.rs'`; expect no private duplicate backup DTO or credential-prefix literal, and expect both native/wasm producers to use the shared contract.
- [ ] Inspect the final diff to confirm `/chains/list` shape, existing UCAN commands/endpoints, root-ending authority, invite roster behavior, no-cwd-binding pull, local-name refusal rules, and best-effort primary command semantics are unchanged where required.
- [ ] Perform a manual local flow with an account service and access service: create/rename a browser spot, link CLI, run `tonk account spots`, pull it without `--name` when its UI name is a valid slug, confirm `tonk spot list` shows an unbound canonical local entry, create/sync a CLI spot, and confirm a second browser restores it. Repeat with an arbitrary UI name and verify pull refuses until `--name` is supplied.

## Handoff and Git requirements

- Start a new branch from `fix/shared-account-handoff-contract` (the head of PR #673); suggested name: `feat/account-spots-cli`.
- Preserve the current #673 implementation and its tests. Do not fold unrelated cleanup into this stack.
- After every required native, wasm, Nix, and ChromeDriver check passes, commit the implementation and this plan with a concise conventional commit, push the branch, and open a non-draft PR whose base is `fix/shared-account-handoff-contract`. The PR body must link #673, summarize list/pull/bidirectional backup behavior, list the exact checks run, and report any skipped/manual-only evidence honestly.
