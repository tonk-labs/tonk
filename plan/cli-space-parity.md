# Local and account space ownership implementation plan

**Goal:** Give the native CLI one simple space model: every installation starts with a local profile, linked accounts are labeled profiles, local replicas are listed together with their profile and role, remote account spaces can be listed and pulled, and a space may move exactly once from local-only ownership into one account.
**Approach:** Reimplement the feature on `staging` after merged PR #726 (`53821ebe3`), using its account-as-profile-main upstream, signed account directory, membership roles, and `/provider/add` protocol. Preserve separate per-profile identities, credentials, registries, and replica storage internally, but aggregate them for inventory. Replace automatic profile-wide enrollment with an explicit, retryable `local -> account` move; an attempted account-to-account move becomes an explanatory error that can perform a targeted share-and-claim instead.
**Constraints:**
- Execute this plan in a fresh worktree based on the current `origin/staging`, which already contains PR #726. The dirty `feat/cli-parity` checkout is reference material only; do not merge or rebase its old account-backup implementation wholesale.
- PR #726 removed the hidden account repository, `AccountSpotBackup`, `/chains/*` escrow APIs, and worker backup/restore modules. Do not recreate them. The account is the profile's `main` upstream, signed `tonk-schema::directory` facts are the remote-space inventory, and retained delegations replicate through that branch.
- The built-in profile label and ID `local` are reserved. A new installation behaves as though `local` is selected before writing any state; the first mutating command persists it. Read-only commands must not create a Dialog profile or registry file.
- Each profile retains an isolated Dialog identity, account session, account branch, credential store, `spots.json`, and replica directories. The aggregate inventory is a read model, not shared authority or shared mutable repository storage.
- A local replica has exactly one profile association and one role: `local`, `owner`, or `member`. `local` means no account membership and no configured content upstream; `owner` means the profile's account initially provisioned the space; `member` means that account claimed an invitation.
- `tonk space new` creates local-only when `local` is selected and creates an account-owned space when a linked account profile is selected. Linking an account never scans, provisions, or enrolls spaces in `local`.
- The only ownership transition is `local -> owner account`, one space at a time. Account-owned and account-member spaces cannot move to `local` or another account.
- An attempted account-to-account move must explain, without delegation terminology, that a synced space stays with its owning account to keep existing shares working. Interactive use may offer one-step targeted share-and-claim; non-interactive use must fail without mutation and print one exact alternative command.
- Sharing never changes ownership. The target account becomes a `member`, may list the space in its signed directory, and may pull its own local replica.
- Local removal, account ownership, provider hosting, collaboration membership, authority, and remote/peer bytes remain separate. Removing a local replica does not remove account directory facts, revoke grants, deprovision hosting, or delete peers' copies.
- `/provider/remove` is destructive in PR #726 and is never part of move, retry, rollback, local removal, or share-and-claim. Account/hosted-space deletion stays a separately confirmed existing workflow.
- A move must be resumable and fail safe. Until the destination account has pushed the exact content revision, retained authority, committed its directory record, and mounted a verified destination replica, the source remains registered and usable under `local`.
- A local space with an existing upstream, durable membership beyond its founder, or recorded invitations is not eligible for move. Fail before provider or registry mutation and explain that only a genuinely local-only space can move.
- Preserve canonical public vocabulary `space`, `--space`, `TONK_SPACE`, `account spaces`, and `share`, with visible `spot`, `--spot`, `TONK_SPOT`, `account spots`, and `invite` compatibility aliases.
- This plan covers the native CLI and shared provider/schema behavior required by it. Browser profile UX, account-to-account ownership transfer, delegation-chain rebasing, rotating existing bearer links, and provider billing transfer are out of scope.

## Approved product contract

Users need remember only two rules:

1. A local space can move into an account.
2. Once a space belongs to an account, it stays there, but it can be shared with another account.

The aggregate local inventory renders the distinction directly:

```text
NAME      SUBJECT         PROFILE     ROLE     LOCAL
scratch   did:key:...     local       local    yes
garden    did:key:...     personal    owner    yes
garden    did:key:...     work        member   yes
roadmap   did:key:...     work        owner    yes
```

An invalid move is actionable:

```text
$ tonk space move garden --to work

Can't move "garden" from "personal" to "work".

Once a space is synced with an account, it stays owned by that account.
This keeps existing shares working.

Share it with "work" and add it there instead? [y/N]
```

If stdin is not interactive, the command exits without mutation and prints:

```text
Run instead:
  tonk space share garden --with work --claim
```

Accepted interactive remediation and the explicit non-interactive command perform the same operation: mint a targeted invitation from the source profile, claim it under the target account, register/pull the target's member replica, and leave the selected profile unchanged.

## Rejected alternatives

- Do not silently reinterpret `move` as `share`; that hides the owner/member distinction.
- Do not support account-to-account ownership transfer; revoking the old shared authority prefix can invalidate downstream users and existing invite chains.
- Do not bridge the new account back through the old account to preserve descendants; that leaves the old account broadly authoritative and is not a real transfer.
- Do not enroll every local space when an account links; account attachment and per-space synchronization are separate choices.
- Do not flatten all profiles into one credential or replica store; aggregate presentation must not weaken profile-bound authorization.

## Durable data model

Add an install-level registry while retaining one existing `SpotStore` per profile:

```rust
pub enum NativeProfileKind {
    Local,
    Account,
}

pub struct NativeProfileRecord {
    pub kind: NativeProfileKind,
    pub label: String,
    pub dialog_profile_name: String,
    pub state_dir: PathBuf,
    pub account_root: Option<String>,
    pub ceremony_origin: Option<String>,
    pub default_access_remote: Option<String>,
    pub default_revocation_relay: Option<String>,
}

pub struct BoundSpace {
    pub profile: NativeProfileId,
    pub space: String,
}

pub struct NativeProfileRegistryV1 {
    pub version: u8,
    pub selected: NativeProfileId,
    pub profiles: BTreeMap<NativeProfileId, NativeProfileRecord>,
    pub bindings: BTreeMap<PathBuf, BoundSpace>,
}
```

`NativeProfileId::local()` serializes as `local`. Generated account-profile IDs remain opaque `p-<32 lowercase hex>`. A fresh `local` profile uses the legacy install root; the linked-legacy exception is defined below. Generated account profiles use `profiles/<id>/spots.json`, `profiles/<id>/spots/`, and `profiles/<id>/account/`; their Dialog key profiles remain in Dialog's platform profile storage.

`state_dir` is a normalized path relative to the installation root; absolute paths, `..`, and aliases of another profile's directory are rejected. A fresh or unlinked legacy install records `local.state_dir = "."`. A linked legacy install instead records its grandfathered account profile with `state_dir = "."` and creates `local` at `profiles/local/`. Every subsequently added account uses `profiles/<id>/`. This makes the existing authority-bearing Dialog profile and bytes stay exactly where they are without letting two profiles share mutable state.

Moves use a separate install-level `moves.json` journal written atomically through `moves.json.tmp` plus rename:

```rust
pub enum MovePhase {
    Prepared,
    AuthorityDeposited,
    ProviderAdded,
    ContentPushed,
    AccountPublished,
    DestinationVerified,
    DestinationRegistered,
    BindingsRewritten,
}

pub struct PendingMoveV1 {
    pub version: u8,
    pub subject: String,
    pub source_profile: NativeProfileId,
    pub source_space: String,
    pub target_profile: NativeProfileId,
    pub confirmed_revision: Option<String>,
    pub phase: MovePhase,
}
```

There is at most one journal row for `{subject, target_profile}`. Each phase records only after its postcondition is verified. Unknown versions or phases fail closed; a corrupt journal never triggers cleanup. The final commit unregisters and removes the source replica, verifies that cleanup, and only then removes the journal row.

The local role is derived offline from the profile kind plus the space repository's signed `Membership` and `MemberRole` facts:

```rust
pub enum SpaceProfileRole {
    Local,
    Owner,
    Member,
}

pub struct LocalSpaceInventoryRowV1 {
    pub version: u8,
    pub profile_id: String,
    pub profile_label: String,
    pub profile_kind: NativeProfileKind,
    pub role: SpaceProfileRole,
    pub name: String,
    pub subject: String,
    pub site: PathBuf,
    pub local: bool, // always true in this local-replica inventory
}
```

Do not store `role` as an independently mutable ownership flag. The registry selects the profile that may open the site; signed repository membership establishes founder versus invited member.

## Legacy bootstrap

- An empty install synthesizes selected `local` without writing files. The first local mutation persists the registry and opens Dialog profile `tonk`.
- An existing unlinked single-profile install becomes `local` in place; its `spots.json`, `spots/`, bindings, Dialog profile `tonk`, and bytes do not move.
- An existing linked PR-#726 install becomes grandfathered account profile ID `legacy` at state directory `.` because its `tonk` Dialog profile already carries the account grant. Create reserved profile ID `local` at `profiles/local/` with Dialog profile name `tonk-local`, and preserve `legacy` as selected so existing commands retain their authority context.
- Bootstrap copies legacy name-only directory bindings into install-level `{ profile, space }` bindings but leaves old fields on disk for downgrade visibility.
- Unknown registry/profile fields survive round trips. Corrupt JSON, unsupported versions, duplicate labels, unknown profiles, and dangling bindings fail closed.

## File map

- `plan/cli-space-parity.md`: Approved contract, implementation tasks, and verification state.
- `rust/tonk-cli/src/account_profiles.rs`: Install-level local/account profile roster, migration, selection, profile contexts, and exact directory bindings.
- `rust/tonk-cli/src/inventory.rs`: Aggregate offline local-replica inventory and rendered role classification.
- `rust/tonk-cli/src/space_move.rs`: Durable local-to-account move state machine, install-level `moves.json`, and preflight.
- `rust/tonk-cli/src/cross_profile_share.rs`: Targeted invite plus claim/pull across two locally linked accounts without changing selection.
- `rust/tonk-cli/src/spot.rs`: One-profile registry primitives and move-safe register/unregister operations.
- `rust/tonk-cli/src/site.rs`: Explicit profile configuration and authority adoption for move destination mounting.
- `rust/tonk-cli/src/account.rs`: Link/login/logout against an explicit account profile atop PR #726 custody.
- `rust/tonk-cli/src/account_state.rs`: Explicit profile/account-main hydration and retained delegation access.
- `rust/tonk-cli/src/account_spots.rs`: Profile-explicit signed-directory list and pull.
- `rust/tonk-cli/src/account_authority.rs`: Strict profile-bound remote authorization and local-to-account adoption boundary.
- `rust/tonk-cli/src/invite.rs`: Profile-explicit targeted mint and durable claim adapters reused by assisted sharing.
- `rust/tonk-cli/src/bin/tonk.rs`: Profile commands, aggregate inventory, move/share UX, prompts, and canonical aliases.
- `rust/tonk-cli/src/lib.rs`: Export new modules.
- `rust/tonk-cli/tests/account_profiles.rs`: Empty/local bootstrap, legacy migration, labels, switching, isolation, and bindings.
- `rust/tonk-cli/tests/space_inventory.rs`: Aggregate role/profile rows, collision handling, and offline behavior.
- `rust/tonk-cli/tests/space_move.rs`: Live provider local-to-account move and failure-injection coverage.
- `rust/tonk-cli/tests/cross_profile_share.rs`: Invalid-move recovery, targeted claim, and member replica coverage.
- `rust/tonk-cli/tests/account_spots.rs`: Per-account signed-directory list and pull coverage.
- `rust/tonk-cli/tests/cli_spot.rs`: Parser, help, output, prompt, alias, and non-interactive regressions.
- `rust/tonk-cli/tests/common.rs`: Two-account fixtures and explicit profile contexts.
- `rust/tonk-cli/README.md`: Two-rule mental model, commands, inventory, failure copy, and lifecycle boundaries.
- `rust/tonk-schema/src/directory.rs`: Reuse PR #726 signed remote-space directory; extend only if an owner/member projection cannot be derived from repository membership.
- `rust/tonk-schema/src/membership.rs`: Reuse founder/member roles as canonical collaboration role.
- `rust/tonk-account/src/delegations.rs`: Reuse retained `space -> account-root` authority in account `main`.
- `rust/tonk-access-service/src/registration.rs`: Reuse idempotent `/provider/add`; change only if focused move tests expose a missing non-destructive idempotence case.

### Task 1: Add the built-in local profile and isolated labeled account profiles

**Files:**
- Create: `rust/tonk-cli/src/account_profiles.rs`
- Modify: `rust/tonk-cli/src/lib.rs:module exports`
- Modify: `rust/tonk-cli/src/bin/tonk.rs:AccountCommand and account dispatch`
- Test: `rust/tonk-cli/tests/account_profiles.rs`

**Interfaces:**
- Consumes: PR #726's fixed `tonk` Dialog profile, `SpotStore`, account link/session APIs, `TONK_SPOTS_STATE`, and existing name-only bindings.
- Produces: `NativeProfileStore::{load_or_bootstrap, selected, select, create_account_pending, context, bind, resolve}`, reserved `NativeProfileId::local()`, and explicit `NativeProfileContext` values used by every later task.

- [ ] Add `it_synthesizes_local_on_an_empty_install_without_writing_state`. Require `selected().label == "local"`, no `profiles.json`, no Dialog profile, and no `spots.json` after `account list` and `space list`.
- [ ] Add `it_persists_local_on_the_first_space_write`. Require profile ID/label `local`, kind `local`, Dialog profile `tonk`, and the new space only in the install-root `spots.json`.
- [ ] Add migration tests for an unlinked legacy install becoming `local` in place and a linked PR-#726 install becoming a selected grandfathered account profile beside an empty local profile. Snapshot every pre-existing path and byte before/after.
- [ ] Add `it_creates_labels_and_switches_without_network_io`, including reserved/duplicate `local`, case-insensitive duplicate labels, exact generated IDs, and `account use local`.
- [ ] Add `it_keeps_account_identity_session_directory_and_spots_disjoint`, creating two labeled account profiles whose Dialog names, account roots, account directories, and `spots.json` paths never alias.
- [ ] Run `cargo test -p tonk-cli --test account_profiles`; expect compilation failure because `account_profiles` and the commands do not exist on PR #726.
- [ ] Implement atomic `profiles.json.tmp` plus rename persistence, strict version validation, flattened unknown fields, deterministic legacy bootstrap, and explicit context construction. Read-only synthesized local must remain allocation- and filesystem-only; it must not open Dialog storage.
- [ ] Add `tonk account add --label <LABEL>`, `account use <LABEL|ID>`, `account list`, and same-root `account login`. `account link` remains compatibility behavior: from local it creates/resumes an account profile rather than converting local; from a rooted account profile it logs that profile back into the same immutable root.
- [ ] Run `cargo test -p tonk-cli --test account_profiles`; expect success.
- [ ] Run `cargo test -p tonk-cli --test account_interrupt --test account_authority`; expect PR-#726 link interruption and authority checks to remain green under explicit contexts.

### Task 2: Resolve profile-qualified spaces and aggregate every local replica

**Files:**
- Create: `rust/tonk-cli/src/inventory.rs`
- Modify: `rust/tonk-cli/src/account_profiles.rs:bindings and aggregate iteration`
- Modify: `rust/tonk-cli/src/spot.rs:profile-local registry adapters`
- Modify: `rust/tonk-cli/src/bin/tonk.rs:space list, use, and selection output`
- Test: `rust/tonk-cli/tests/space_inventory.rs`
- Test: `rust/tonk-cli/tests/cli_spot.rs`

**Interfaces:**
- Consumes: Task 1 profile contexts, each profile's `spots.json`, repository DID, `Membership`, `MemberRole`, `--space/--spot`, environment selection, and install-level directory bindings.
- Produces: `inventory::list_local(&NativeProfileStore) -> Result<Vec<LocalSpaceInventoryRowV1>>` and `ResolvedSpace { profile, name, site, source }`.

- [ ] Add `it_lists_local_owner_and_member_replicas_with_their_profiles`. Arrange the four approved-contract rows and require stable sort by profile label, space name, then subject.
- [ ] Add `it_lists_offline_without_opening_account_or_remote_services`. Deny all provider requests and require role/profile output from local registries and repository facts.
- [ ] Add duplicate-name coverage: `personal/garden` and `work/garden` both render; an unqualified explicit name in the selected profile resolves there; a directory binding resolves its exact `{ profile, space }` even after another profile is selected.
- [ ] Add corrupt/unreadable-site coverage that retains other rows and emits one row-specific diagnostic without borrowing another profile's identity to inspect it.
- [ ] Add exact text and JSON snapshots for `PROFILE`, `PROFILE TYPE`, `ROLE`, `LOCAL`, canonical camelCase fields, and `local|owner|member` values.
- [ ] Run `cargo test -p tonk-cli --test space_inventory --test cli_spot`; expect missing aggregate inventory/profile columns.
- [ ] Move directory binding ownership into `NativeProfileStore`. Keep `SpotStore` responsible for one profile's entries and paths. Resolve once to `ResolvedSpace`; no later command may consult the globally selected profile again.
- [ ] Classify `local` from profile kind and absence of account/remote state; classify `owner` from `MemberRole::FOUNDER`; classify `member` from `MemberRole::MEMBER`. Conflicting or absent signed role evidence on an account profile is an error row, never guessed ownership.
- [ ] Make `tonk space list` installation-wide and offline. Preserve `tonk account spaces` for remote account-directory inventory rather than mixing absent remote rows into this command.
- [ ] Run the focused tests; expect success.
- [ ] Run `cargo test -p tonk-cli --test spot --test site`; expect one-profile registry and site behavior to remain green.

### Task 3: Make account lifecycle operate on labeled account profiles without enrolling local spaces

**Files:**
- Modify: `rust/tonk-cli/src/account.rs:explicit profile link/login/logout`
- Modify: `rust/tonk-cli/src/account_state.rs:profile-main account upstream adapters`
- Modify: `rust/tonk-cli/src/account_authority.rs:profile-bound grants`
- Modify: `rust/tonk-cli/src/site.rs:create under selected profile`
- Modify: `rust/tonk-cli/src/bin/tonk.rs:account lifecycle and space new dispatch`
- Test: `rust/tonk-cli/tests/account_profiles.rs`
- Test: `rust/tonk-cli/tests/account_authority.rs`

**Interfaces:**
- Consumes: Task 1 account contexts and PR #726 custody/account-main APIs.
- Produces: a linked account profile with immutable root/deployment defaults, plus account-owned creation only when that account profile is explicitly selected.

- [ ] Add `it_links_an_account_without_touching_local_spaces`. Snapshot local registry, site bytes, revisions, remotes, memberships, and invitations; link `personal`; require every snapshot unchanged and no `/provider/add` for local subjects.
- [ ] Add `it_creates_local_when_local_is_selected_and_owner_when_an_account_is_selected`. The local case has no upstream/account directory row; the account case provisions only the new subject, stamps founder role, pushes exact content, and records its mount in that account directory.
- [ ] Add cross-profile denial coverage: selecting `work` cannot authorize, publish, list as local, or mutate a `personal` space unless resolution explicitly returns the `personal` context.
- [ ] Run `cargo test -p tonk-cli --test account_profiles --test account_authority`; expect failures while production still assumes one fixed profile/store.
- [ ] Thread `NativeProfileContext` through account session/state/authority/site APIs. Do not retain a space prefix, provision a consumer, or record a directory row from account link itself.
- [ ] Keep logout local/offline and profile-scoped. Preserve local edits, account root, account branch, remote configuration, and replicas; deny later network operations before HTTP until that account profile logs in.
- [ ] Run focused tests; expect success.

### Task 4: List and pull each account's signed remote directory

**Files:**
- Modify: `rust/tonk-cli/src/account_spots.rs:list and pull`
- Modify: `rust/tonk-cli/src/bin/tonk.rs:AccountSpotsCommand`
- Test: `rust/tonk-cli/tests/account_spots.rs`

**Interfaces:**
- Consumes: explicit account-profile context, PR #726 `tonk_schema::directory::{spaces,mount_record}`, local aggregate inventory, and signed membership roles.
- Produces: `account_spots::{list_in,pull_in}` pinned to one account profile and `tonk account spaces [--profile <LABEL>] list|pull`.

- [ ] Add `it_lists_remote_spaces_for_each_account_without_cross_contamination`. Give `personal` owner/member rows and `work` different rows; require profile-qualified list results even while `local` is selected.
- [ ] Add offline fallback coverage: a failed account pull warns and renders the last signed local directory; it does not silently use another profile's account branch.
- [ ] Add `it_pulls_an_invited_space_as_a_member_replica`. Require target profile registration, member role, exact subject and remote, successful initial pull, and no owner/provider mutation.
- [ ] Add same-name coverage within different account profiles and refusal to overwrite an existing entry or orphan inside the target profile.
- [ ] Run `cargo test -p tonk-cli --test account_spots`; expect failures on fixed global profile/store usage.
- [ ] Parameterize PR #726's list/pull implementation by `NativeProfileContext`. Default `--profile` to the selected account profile; if `local` is selected, require `--profile` and list available account labels instead of guessing.
- [ ] Register pulled replicas only after mount, initial pull, and signed membership verification succeed. On failure remove only newly staged target storage and leave directory facts untouched.
- [ ] Run focused tests; expect success.

### Task 5: Implement a resumable one-space local-to-account move

**Files:**
- Create: `rust/tonk-cli/src/space_move.rs`
- Modify: `rust/tonk-cli/src/spot.rs:move-safe register and unregister`
- Modify: `rust/tonk-cli/src/site.rs:explicit target adoption`
- Modify: `rust/tonk-cli/src/account_state.rs:retain and directory push`
- Modify: `rust/tonk-cli/src/bin/tonk.rs:SpotCommand::Move`
- Modify: `rust/tonk-cli/src/lib.rs:module export`
- Test: `rust/tonk-cli/tests/space_move.rs`
- Test: `rust/tonk-access-service/tests/registration.rs`

**Interfaces:**
- Consumes: exact local `ResolvedSpace`, linked target account context, source space signing authority, target deployment defaults, `/provider/add`, account-main delegation retention/directory record, and Task 4 pull.
- Produces: `space_move::execute(store, source, target) -> MoveOutcome` and atomic install-level `moves.json` entries keyed by space subject and target profile.

- [ ] Add preflight tests rejecting a source that is owner/member, has an upstream, lacks the space signing credential, has non-founder durable membership, has recorded invitations, targets `local`, or targets an unlinked/unhydrated/signed-out account. Assert zero provider, repository, registry, and binding mutation.
- [ ] Add `it_moves_one_local_space_into_one_account`. Require the same repository subject and exact tree, idempotent provider ownership under the target account, target `space -> account-root` authority, confirmed content push, account-directory mount record, founder role, target local registration, rewritten directory bindings, and no remaining source registration/data.
- [ ] Add failure injection after authority mint, `/provider/add`, content push, account retention, directory push, destination pull, destination registration, and binding rewrite. Before final commit the source remains registered/readable; retry resumes without another subject, provider, or account-directory entity.
- [ ] Add crash recovery tests for every journal phase. A stale journal whose destination is fully verified finishes local registry cleanup; any earlier phase resumes forward and never calls `/provider/remove`.
- [ ] Add exact revision coverage: move cannot commit merely because a remote exists; destination local revision, provider-accepted source revision, and account directory's mount configuration must all describe the moved subject.
- [ ] Run `cargo test -p tonk-cli --features integration-tests --test space_move`; expect missing command/state machine failures.
- [ ] Implement the move in the journal phases above: preflight; mint/deposit target authority; add provider; attach/push source and record the confirmed revision; retain authority and record/push target account directory; pull/verify destination replica; register destination; atomically rewrite install-level bindings; unregister and delete source replica; verify source cleanup; clear the journal row. Provider/account side effects before commit are safe idempotent progress, not rollback targets.
- [ ] Store no account-to-account move path. Re-running a completed move returns an already-owned success only when source subject and target account exactly match the committed destination.
- [ ] Run the focused move and provider-registration tests; expect success.

### Task 6: Turn invalid account moves into assisted targeted sharing

**Files:**
- Create: `rust/tonk-cli/src/cross_profile_share.rs`
- Modify: `rust/tonk-cli/src/invite.rs:profile-explicit targeted mint and claim adapters`
- Modify: `rust/tonk-cli/src/bin/tonk.rs:space move/share UX`
- Modify: `rust/tonk-cli/src/lib.rs:module export`
- Test: `rust/tonk-cli/tests/cross_profile_share.rs`
- Test: `rust/tonk-cli/tests/cli_spot.rs`

**Interfaces:**
- Consumes: an account-owned/member source context, another linked account's immutable root DID, existing targeted invite mint, durable claim, Task 4 pull/register, and terminal interactivity.
- Produces: `cross_profile_share::share_and_claim(source, target, subject) -> ShareClaimOutcome`, explicit `tonk space share <SPACE> --with <ACCOUNT> --claim`, and the approved invalid-move remediation.

- [ ] Add exact output tests for the approved account-to-account move error, including source/target labels, the two simple explanatory sentences, `[y/N]`, and the one-line non-interactive alternative. Forbid “UCAN,” “delegation,” “provider,” “prefix,” and root DID strings in ordinary copy.
- [ ] Add `it_declines_assisted_share_without_mutation`, covering `n`, EOF, and non-interactive stdin.
- [ ] Add `it_shares_and_claims_in_one_step_without_switching_profiles`. Require a targeted audience equal to the target account root, durable member role, target account directory row, target local member replica, unchanged source ownership, and unchanged selected profile.
- [ ] Add target-already-member coverage that pulls/registers the existing membership rather than minting a duplicate invitation.
- [ ] Add claim/pull failure coverage: source ownership and current selection remain unchanged, no target local registration is claimed, and retry output names the exact `space share ... --with ... --claim` command without printing capability material.
- [ ] Run `cargo test -p tonk-cli --features integration-tests --test cross_profile_share --test cli_spot`; expect missing command and remediation failures.
- [ ] Implement one shared operation used by both the explicit command and accepted prompt. Resolve source/target contexts once; do not call `account use`, mutate selection, or shell out to nested CLI processes.
- [ ] Keep ownership immutable. The target is always stamped/verified as `member`; share-and-claim never calls `/provider/add` or changes the source account's directory/provider ownership.
- [ ] Run focused tests; expect success.

### Task 7: Make local removal and synchronization obey the two-rule model

**Files:**
- Modify: `rust/tonk-cli/src/bin/tonk.rs:space rm, push/pull, account sync`
- Modify: `rust/tonk-cli/src/auto_sync.rs:resolved-profile routing`
- Modify: `rust/tonk-cli/src/account_spots.rs:directory persistence boundary`
- Test: `rust/tonk-cli/tests/cli_spot.rs`
- Test: `rust/tonk-cli/tests/account_authority.rs`
- Test: `rust/tonk-cli/tests/account_spots.rs`

**Interfaces:**
- Consumes: role-aware `ResolvedSpace`, exact profile context, existing PR #726 sync, local registry removal, and hosted-space deletion commands.
- Produces: profile-routed sync and local-only replica removal without account/provider mutation.

- [ ] Add `it_removes_only_the_selected_local_replica`. For owner and member replicas require local registry/data removal according to flags while signed directory, memberships, invitations, provider row, authority, remote blobs, and another profile's replica remain unchanged.
- [ ] Add `it_never_enrolls_local_spaces_during_link_login_or_sync`. Count provider calls and account-directory writes across all three commands.
- [ ] Add profile-routing tests showing a cwd bound to `personal/garden` continues syncing with personal authority while `work` is selected; local spaces without upstream remain offline and editable.
- [ ] Run focused CLI/account tests; expect failures where single-profile/global state or account-wide removal remains.
- [ ] Route every space operation from `ResolvedSpace.profile`. Restrict account sync to replicas already registered under that account profile; do not scan `local` or other profiles.
- [ ] Keep `account spaces delete` and account deletion as the existing explicit destructive browser-review paths. Do not alias them from `space rm`, `move`, or failed cleanup.
- [ ] Run focused tests; expect success.

### Task 8: Document and verify the complete CLI journey on top of PR #726

**Files:**
- Modify: `rust/tonk-cli/README.md`
- Modify: `rust/tonk-cli/tests/cli_spot.rs`
- Modify: `rust/tonk-cli/tests/common.rs`
- Modify: `plan/cli-space-parity.md:verification checkpoint only`

**Interfaces:**
- Consumes: Tasks 1-7 and PR #726 provider/account fixtures.
- Produces: one documented, compatibility-tested native CLI workflow; no new production interface.

- [ ] Add `it_drives_the_two_rule_space_lifecycle`. Start empty; prove synthesized local; create two local spaces; link labeled personal/work accounts; move only one space to personal; list aggregate local rows; list each account's remote rows; reject move personal->work; accept assisted share; claim/pull a work member replica; and require source ownership plus selected profile unchanged.
- [ ] Add restart persistence after every major step: local bootstrap, interrupted account add, completed move journal, assisted claim, local replica removal, logout/login, and account directory refresh.
- [ ] Add canonical/legacy parser and help coverage for `space|spot`, `--space|--spot`, `TONK_SPACE|TONK_SPOT`, `account spaces|account spots`, and `share|invite`. Canonical terms lead help and output.
- [ ] Update the README with only the two user rules, the aggregate inventory example, `account spaces --profile`, local-to-account move, assisted sharing copy, local removal boundary, and owner/member definitions. Keep internal profile storage and delegation details out of the quick-start mental model.
- [ ] Run `cargo test -p tonk-cli`; expect success.
- [ ] Run `cargo test -p tonk-cli --features integration-tests --test account_profiles --test space_inventory --test account_spots --test space_move --test cross_profile_share --test cli_spot --test account_authority --test account_interrupt --test site --test spot`; expect success with serial execution where provider fixtures require it.
- [ ] Run `cargo test -p tonk-access-service --test registration`; expect success.
- [ ] Run `nix develop . -c env CARGO_INCREMENTAL=0 test:web:debug`; expect the complete WASM suite to pass because shared schema/provider changes remain browser-compatible. Use `.`, not `path:.`, to avoid snapshotting the local `target` tree.
- [ ] Run `cargo check --workspace --all-targets --all-features`; expect success.
- [ ] Run `cargo clippy --workspace --all-targets --all-features -- -D warnings`; expect success.
- [ ] Run `cargo fmt --all -- --check`, `nixfmt --check flake.nix`, and `git diff --check`; expect no formatting or whitespace changes.
- [ ] Inspect `git diff --stat`, `git status --short`, and the merge base. Confirm the implementation is based on current `origin/staging`, contains no retired backup/escrow API, no provider-remove move path, no account-to-account ownership transfer, no automatic local-space enrollment, and no unrelated browser redesign.

## Acceptance criteria

- A new install behaves as selected profile `local`; read-only commands create no state.
- Users can add, label, list, select, log into, and log out of multiple isolated account profiles.
- `tonk space list` shows every local replica with its exact profile and `local|owner|member` role, offline and deterministically.
- `tonk account spaces --profile <LABEL>` lists that account's signed remote directory and can pull a missing owner/member space into that profile.
- Linking or synchronizing an account never enrolls unrelated local spaces.
- `tonk space move <SPACE> --to <ACCOUNT>` accepts only a genuinely local-only source and commits only after exact remote, account-directory, authority, and destination-replica verification.
- A failed/interrupted move preserves the local source and resumes idempotently; no move path calls destructive provider removal.
- Account-owned/member spaces never move. The error explains the rule in simple terms and offers one-step share-and-claim.
- Assisted share-and-claim preserves ownership, creates/verifies target membership and a local member replica, and never changes the selected profile.
- Local replica removal does not alter account discovery, membership, authority, hosting, remote data, or peer replicas.
- The implementation uses PR #726's account-main directory and retained delegations and does not restore its removed hidden-account/escrow architecture.

## Explicitly deferred

- Account-to-account ownership transfer or account-to-local detachment.
- Delegation-chain rebasing, bridge grants, or preservation of already-issued bearer links across ownership transfer.
- Non-destructive provider/billing ownership handoff.
- Compelling remote or peer deletion, or calling local removal “delete everywhere.”
- Browser multi-profile UX and browser-assisted local-to-account moves.
- Encrypting local profile/space data from another process running as the same OS user.
- Removing compatibility aliases or renaming serialized/internal `Spot*`, `spots.json`, capability, or protocol identifiers.
