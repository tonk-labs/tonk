# Implementation notes — session delegations and the revoke ceremony

Running log of deviations from the two plans
(`2026-07-24-session-delegations.md`, `2026-07-24-signed-revocation-artifacts.md`)
and the edge cases behind them.

## Session delegations Task 3 was written against a client seam that does not exist

The plan says to call `extend_with_session` "where the client currently
loads its `root → device` grant for sync". There is no such place. The
presign invocation is signed by dialog's **operator** key, and its proof
chain is assembled by a `CertificateStore::prove` BFS from the operator
toward the subject — the client never hands a grant to a signing call.

The `device → session` hop the plan wants already exists structurally:
it is `profile → operator`, minted by `OperatorBuilder::build` when the
caller passes `.allow(...)`, and unexpiring. Bounding *that* is Task 3.

Two upstream facts shaped how:

- `prove` filters candidates with `range.covers(requested)`, and the
  requested range is always `TimeRange::unbounded()` (`UcanFork::authorize`
  builds `Authorize::new`, never `.during`). `covers` is vacuously true
  when both required bounds are `None`, so **an expired certificate is
  still selected**. Nothing in the walk consults the clock.
- Certificates are keyed by content hash (`{audience}/{subject}/{issuer}.{hash}`)
  and the store has no delete.

Together those mean re-minting a delegation under the same audience
leaves an expired certificate beside the fresh one, identical in
issuer/audience/subject, and `prove` picks between them in hash order.
So **renewal has to rotate the operator key**, giving each session its
own audience prefix. Dead certificates under retired audiences are never
selected because nothing proves *to* those audiences again.

The seam that makes rotation possible: `derive_operator` keys off the
caller-supplied context (`profile.derive(b"worker")`), so a random
context yields a fresh operator key per session.

## Sign-out revokes, so it must also rotate the profile

`mint_self_revocation` revokes the device's `root → device` grant, and
the access-service screen matches revoked rows on `device_did` **unscoped
by account** (`revoked_query`). Every presign chain this browser produces
carries the profile DID as a delegation issuer, so a self-revoke without
rotation permanently refuses presigns for that profile — local spaces
included — with no un-revoke anywhere.

Rotation is safe because nothing outside the device is keyed to the
device DID: `member_did` returns the account root when linked, and
`restore_spaces` re-mounts every escrowed `space → root` chain against a
fresh `root → newdevice` link.

Known gap, not closed here: `back_up_owned_space` is hooked only into
`enable_sync_inner`, so a space that was never sync-enabled is never
escrowed and does not survive rotation.

## Renewal needed an upstream change, so the pin moved to a rev

Rotating the operator means building a replacement, and
`OperatorBuilder::build` consumes the `Storage` it is handed. `Storage`
was not `Clone`, so the replacement could only get a fresh
`Storage::new()` — a second pool of connections over the same databases,
while every repository and branch handle the reactor cached still talked
to the first. Divergence with no error surface, and dropping the reactor
cache to compensate would have meant hand-migrating live subscriptions.

The fix upstream is small because the sharing is already there:
`Router` was `Clone` and holds an `Arc<Pool>`; only `Loader.mounts` was
unshared. dialog-db `feat/storage-clone` (#407) wraps it in an `Arc` and
derives the two impls, cut from the `tonk-2026-07-17` tag rather than
`main` so it carries none of the reserved-namespace or version-control
work that tonk #635 exists to absorb. tonk pins that rev until #407
lands and the pin can move to a tag.

## Two things only the wasm leg catches

Both were found by running the wasm suites locally rather than waiting
on CI (Chrome 150 at the default path plus nixpkgs chromedriver 150 —
`CHROMEDRIVER=… cargo test -p tonk-worker --target wasm32-unknown-unknown`).

- `it_refuses_to_revoke_without_a_signed_revocation` had never passed.
  It asserted a `Conflict` for an empty revocation, but the check sat
  behind `linked_service`, whose service lookup casts the global to a
  `ServiceWorkerGlobalScope` and fails outside a real worker — so the
  call returned `NotFound` and the assertion never got what it tested.
- `bootstrap_profile` seeds the standard library over a service-worker
  fetch, so calling it from sign-out made sign-out fail in any harness.
  It is now logged rather than fatal, which is also the right posture:
  by then the device is revoked and the key rotated, so there is nothing
  left to retry.

`Directory::Temp` is a stable path, so a per-run counter is unique only
*within* a run — the `device` tests name their scratch registries
randomly, or the next run inherits the last run's rotated pointer and a
profile that never rotated reads as one that did. Same trap
`router.rs`'s `session_nonce` documents.

## Identity and sharing E2E hardening

The accepted dialog prerequisite was already present in the worktree at
`25fac91eaa4d23fc220721df19f9e2593be618d7`. All workspace `dialog-*`
dependencies and `Cargo.lock` use that one revision; no mixed pin was
introduced.

The Darwin fix is the narrow top-level `remarshal` overlay described by the
plan. `nix-store -qR` resolves it to `python3.13-remarshal-1.3.0`; unrelated
Python packages remain unchanged.

Two defects were visible only in the aggregate Web gate:

- `tonk-fab` used `HtmlInputElement` for targeted invitations without enabling
  its `web-sys` feature.
- a `tonk-identity` revocation test passed `std::time::SystemTime` to dialog's
  Wasm `web_time::SystemTime`, and `tonk-account-service` used
  `#[dialog_common::test]` without its test-only `wasm-bindgen-test`
  dependency.

The targeted invitation copy cannot call `clipboard.writeText` after awaiting
the mint response because transient user activation has expired. It now shares
the existing promise-backed `ClipboardItem` seam: the form submit opens the
write synchronously and resolves it with the returned URL.

Native HTTP and credential tests cannot bind loopback ports or save temporary
signing sessions in the managed sandbox. The same binaries pass outside the
sandbox. No product error was hidden by those permission failures.

`path:.` is required while the new modules are untracked, but it also snapshots
the ignored 32 GB Cargo `target/` directory. The gates temporarily moved that
regenerable cache outside the source and restored it on exit. The
`test:native:debug` shell wrapper then invoked `git+file:.` internally and
omitted the untracked modules, so the equivalent `tests-native-debug` and
`tests-web-debug` derivations were built directly from `path:.`.

No account/access/UI deployment or staging mutation was performed. The session
had no staging ownership or deployment provenance, and the smoke requires
disposable account data plus controlled D1/R2 state. The implementation and
local/Nix gates are complete; the serial deployment and non-destructive staging
smoke remain an operator-owned release step.

Root-first account tests must derive provider ceremony metadata from the root
already persisted in the test profile. Supplying a second deterministic root is
not a harmless fixture shortcut: the account boundary correctly rejects it as a
ceremony/root mismatch. Invitation attribution likewise asserts the persisted
root entity, not the profile device DID.

The hardened sync contract distinguishes orchestration from an attempted
operation. A background sweep filters out branches with no upstream and
resolves as a no-op; a direct `POST .../sync` for that same branch is an
operational failure and returns non-2xx `SYNC_UNAVAILABLE`. Tests now pin both
sides so a manual request cannot claim reconciliation merely because there was
nothing configured to reconcile.

The crate-wide Wasm test harness runs in a browser document even when one module
requests a service-worker harness. The outbound HTTP helper therefore calls the
same `globalThis.fetch`, `setTimeout`, and `clearTimeout` functions dynamically
on both targets. Production still supplies the service-worker global; the
browser test can replace those functions to verify bytes, media type, structured
status, and abort timeout without maintaining a second transport.

The staging pre-deploy audit found that `delegation_hex` had been added to the
already-applied `0001_init.sql`, so Wrangler reported no pending migration while
the live `devices` table still lacked the column. The signed path cannot be
reconstructed from its CID or from account space backups. Migration 0003
therefore adds a nullable column: new registrations always populate it, while
legacy rows expose absent evidence explicitly and disable only cross-device
revocation. Self-revocation remains possible from the device's local grant.

# Design notes — root-owned account state repository, initial rewrite

2026-07-28 rewrite of
`docs/superpowers/specs/2026-07-27-account-profile-name-design.md`.

> Superseded in part by the V1 simplification recorded below. In particular,
> `Provision`/`OpenOnly`, mutable manifests, endpoint lists, and repository
> subject rotation are no longer the current design.

## The root DID identifies a repository but cannot locate it

The original design treated each device's default remote as if deriving the
same root subject would make devices converge on the same remote history. It
does not: identity and location are separate. Two providers can both host a
repository with the root subject and never discover each other.

The revised design requires a root-signed `AccountHomeManifest` carried by the
account link handoff. The current origin's default remote is only a
first-home proposal, never an existing account's discovery rule.

The manifest is signed directly during a root/passkey ceremony, not by an
ordinary linked device. Moving the home has account-wide blast radius; requiring
the root prevents a compromised but not-yet-revoked device from silently
redirecting every future link.

## Boot is not allowed to infer creation authority from absence

A missing branch, an unreachable remote, and a remote the caller cannot read
must not collapse into one "clone failed, initialize" path. Only the ceremony
that establishes the first manifest gets `Provision` authority, and only a
confirmed absent response permits creation. Devices receiving an existing
manifest are `OpenOnly`.

Existing accounts therefore need a distinct one-time root/passkey establishment
path. The original "same boot path, no migration machinery" claim was removed
because it cannot resolve simultaneous devices or distinguish outage from
absence.

## Account state is a system replica

The existing `Replica::new` has only profile and repository kinds. A
root-subject replica mounted with it would appear as a space and enter roster,
migration, pause, removal, and switcher paths. The revised design adds
`tonk:account` and changes space enumeration to select the real-space kind.

## Projection is an explicit worker responsibility

Reactor pulls re-poll in-memory subscriptions and broadcast frames; they do not
durably project one branch into another. `converge_account_state` is now an
explicit hook after boot, account-state writes, mount/pull, and background
pulls.

An adopting device restamps every real space it knows. The renaming device may
not hold the same set of spaces, and the current restamp loop logs and skips
per-space failures without a durable retry.

## The portability claim is deliberately narrower

The repository removes Tonk services as the data authority for already-linked
devices. It does not yet make enrollment or revocation provider-independent:
root→device grants are unexpiring, and Tonk's access service consults the
account registry. A non-Tonk remote without equivalent revocation screening is
compatible at the object protocol but weaker at the authorization boundary.

## Generality comes from typed facts, not a generic settings document

The repository is a small account control plane. `AccountDisplayName` is
separate from the device-local `ProfileName` cache. Future settings, a
space/delegation index, and bounded user-owned summaries define their own
concepts and merge behavior. Provider-metered usage, secrets, large blobs, and
unbounded event logs stay outside it.

# Design notes — account-state V1 simplification

2026-07-28 review revision of
`docs/superpowers/specs/2026-07-27-account-profile-name-design.md`.

## Generality is now concentrated at stable boundaries

The display name is the only known account-wide fact. Version 1 therefore
builds no settings API, projector registry, provider migration, or second
account fact. The reusable boundaries are the immutable account subject, a
versioned signed descriptor, the `tonk:account` replica kind, a trusted-base
write gate, and a typed fact with explicit merge and projection behavior.

A later fact must justify that it is small, bounded, user-owned,
provider-visible, genuinely account-wide, and compatible with the account
repository's active-device writer set.

## Account identity survives authority rotation

Using "root DID" for both repository identity and current signing authority
conflicted with the planned `oldRoot → newRoot` succession ceremony. Version 1
defines an immutable account subject equal to the genesis root DID. Rotation
must preserve authorization to that subject; it does not migrate the repository
or change its routing key.

## The descriptor is immutable and deliberately narrow

`AccountRepositoryDescriptorV1` contains one account subject and one canonical
remote. It has no generation, endpoint list, failover, or provider-movement
semantics. It is a durable signed artifact, not the existing five-minute
account-service invocation.

The account service stores the exact descriptor bytes atomically with new
account creation, or by set-if-absent for existing accounts, and relays them in
browser and CLI links. Root signatures remove content and authenticity
authority from the service, but the service remains a V1 availability and
first-writer coordination dependency. Calling it wholly untrusted would
overstate the design. An establishment response returns the stored winner, so a
caller does not persist its own losing candidate.

## Hydration, configuration, and reachability are separate

Mounting creates a local handle but does not prove that the local branch
descends from the established account history. The durable lifecycle is:

- unconfigured: no valid descriptor;
- unhydrated: descriptor valid, trusted base not yet acquired;
- ready: successful pull or successful create-if-absent recorded for the
  descriptor hash.

Remote reachability is transient. A ready replica remains writable offline; an
unhydrated replica accepts no account-state writes. Linked rename now fails
clearly in the latter state instead of writing to a blank branch or silently
falling back to the device-local cache.

## Atomic remote creation replaces one-shot provisioning authority

`Provision` and `OpenOnly` were removed. Once one descriptor is durably
established, every active linked device names the same subject and remote and
already has account write authority. Any of them may retry atomic
create-if-absent of the empty repository and its `main` branch:

- the winner adopts the created empty history;
- a loser pulls the winning history;
- outages and ambiguous failures do neither.

This closes the browser-failure case without minting another durable capability.
It depends on a live remote proving typed absence and compare-and-set creation.
If that primitive does not exist, implementation stops and revisits
provisioning authority rather than weakening the no-fork invariant.

## Initial names are explicit, not elected by boot

New-account creation and the one-time existing-account establishment ceremony
copy the initiating device's current `ProfileName` after the account repository
becomes ready. `device_name` is only a device label. Other devices never seed
from their local petnames merely because `AccountDisplayName` is absent.

If the initiating device disappears before writing the fact, the account is
valid but unnamed until an explicit rename. This is preferable to an implicit
multi-device election or another pending-intent protocol.

## Projection retries each target independently

Convergence cannot return early because the local `ProfileName` already
matches. It checks every real space, commits only stale targets, and leaves a
failed target eligible for the next boot or successful account sync even when
the authoritative name has not changed.

The account display name is globally canonical in version 1. Per-space aliases
are unsupported, so space `MemberName` rows remain projections rather than
overrides.

## CLI shares transport but needs no daemon

Browser and CLI link responses both carry the descriptor with the delegation
and persist them as one local account-link record. The worker ensures account
state during boot. The CLI ensures it during link and on demand before an
account-state operation; version 1 adds no native background process or
display-name editing surface.
