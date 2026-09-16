# Milestone 2 implementation handoff

Read-only source review, 2026-09-16. This is an implementation seam map, not
completion evidence. The ordinary-grant CLI build gate must pass before broadening
production flows. Preserve `handoff.rs`, legacy `join --agent`, and unrelated dirty
changes. No production CLI code was changed for this review.

## First executable increment: import exactly the bearer identity

Add `rust/tonk-cli/src/connections.rs`, exported from `lib.rs`. Use
`tonk_invite::connection::AgentInvite::parse_url`, `grants()`, `secret_seed()`, and
`SpaceGrantBundle::{subject,recipient,chains,remote,expires_at}`. All grant chains
must validate before local installation. Identify a connection deterministically
from recipient DID, subject, and sorted leaf delegation CIDs; do not hash/store the
full bearer URL as a resume credential. Same identity/grant set resumes; another
invitation or explicitly separate binding gets separate replica storage.

The current parser requires caller-supplied subject-bearing scopes and a trusted
remote. The bare CLI has neither a selected space nor a trusted URL derived from
the invite. Add a bounded envelope inspection step that extracts the *untrusted*
claimed subject to construct the fixed rights preset, then run the existing full
validation; this is not authority until validation succeeds. Resolve the endpoint
through the independently trusted service configuration, not from an invite query
or fragment. Preserve the current endpoint/executor DID resolution mechanism.
Do not accidentally make the unsigned carrier URL an approval destination.

`Profile::open/create` generate a key and cannot import the invitation key. The
existing public lower-level APIs can import without another Dialog patch:

- `Ed25519Signer::import(seed)` -> `Credential::Signer(SignerCredential::from(...))`.
- Mirror `dialog-identity/src/profile/open.rs`: capability
  `Subject::from(did!("local:storage")).attenuate(storage::Storage)
  .attenuate(Location::new(directory, profile_name)).create(credential)
  .perform(&Storage::<NativeSpace>::default())`.
- Reopen with `Profile::load(name).at(directory).perform(&storage)` and require its
  DID to equal the invitation recipient. Existing mismatching credentials fail;
  never overwrite them. Resume must use `load`, never `open` with random-key
  creation on missing credentials.
- Derive the local operator from this imported profile and retain every public
  chain through `profile.save(UcanDelegation(chain)).perform(&operator)`.

Use the existing native credential serialization/provider and verify actual file
permissions and persistence barriers in a focused disk roundtrip before declaring
secret storage complete. A per-connection private directory and profile context
must contain only this identity/grants. Do not import into the default `tonk`
profile or its access branch, which also holds account and other-space authority.
The public journal stores credential references and CIDs, never seed/URL.

## Persisted authority selection and mounting

`TonkSite` already funnels local/remote operations through
`AccountBoundOperator`. `TonkSite::open_with` calls
`build_profile_and_operator` and then `account_authority::wrap`; `open_selected`
in `bin/tonk.rs` currently always supplies `default_config()`. Therefore a one-time
special config at import is insufficient: the next process would open the global
profile and account guard.

Add a versioned optional authority binding to `SpaceEntry` (currently only `site`)
and carry it through `Resolved` or a single `open_resolved` helper. Suggested
binding: kind, connection ID, public subject/recipient, credential-store reference,
and grant-set CIDs. Keep absent bindings explicitly legacy. Also write a site-local
public identity marker; if a scoped site's registry binding disappears/mismatches,
fail its remote setup rather than interpreting it as legacy/default authority.

### Older binary behavior requires a structural layout boundary

A marker alone is insufficient: old `TonkSite::open_with` ignores unknown files,
uses its default profile, and loads `main` from the supplied directory. A
verifier-only `main` credential is not profile-bound; Dialog's space load checks
that the request names the current profile, then reads the credential from the
operator's base directory. An older CLI that already has same-subject account
proofs can therefore reopen a conventional scoped replica with ambient authority.

Use a registry-owned **outer connection root** with a fixed nested data root:

```text
spaces/<alias>/                  # SpaceEntry.site and Resolved.site
  connection.json                # version, connection ID, public credential ref
  import-journal.json            # public checkpoints only
  data/                          # operator base; TonkSite.root after opening
    main/                        # actual verifier credential and repository data
```

Never install an ordinary `main` credential at the outer root. Keep imported
profile credentials in the dedicated credential store, outside journal/data
metadata. New `open_resolved`/`TonkSite::open_with` first recognize and validate the
outer layout, load the exact imported profile, then derive the operator against
`outer/data`; old normal registry-selected opens address `outer/main` and fail
instead of reaching invitation data. This is source-grounded expected behavior,
not yet an old-binary executable test. Old open may still mint its normal profile
session before the missing-repository error; do not promise zero legacy writes.

Keep `TonkSite.root` as the **actual Dialog data base**: readability checks,
repository files and raw byte access already assume `root/main`. Add a distinct
connection-root/binding field or accessor for resume journal location; never
infer the outer directory by generic `parent()` calls from arbitrary legacy sites.
Keep `Resolved.site`/registry paths as the outer root for alias ownership, staging,
listing, binding and explicit removal. Constructors must check the outer marker,
fixed data child, imported recipient, mounted subject and grant IDs together;
reject contradictory outer `main` storage rather than letting a partially old-
initialized root select the account path. Reject symlink/path traversal in the
managed layout. For direct new-code opens of `data`, recognize an inner scoped
format marker or the exact validated parent layout and route correctly/refuse;
never silently treat the nested replica as legacy.

Publish the whole outer staging directory atomically. Recovery and marker-loss
probes must never create `outer/main` or adopt downloaded data under the global
profile. Old registry rewrites may drop the new `SpaceEntry` field, so the outer
marker must independently identify the scoped constructor; missing both markers
still cannot expose a conventional repository at the outer path. Add an actual
released-old-CLI test with unrelated/default account state and *same-subject*
ambient authority: normal selected read/push must fail and nested data/heads remain
unchanged. New CLI restart, directory binding, orphan listing and keep-data/remove
must address the correct outer/data paths. Explicitly pointing an old binary at
`data` or modifying files as the same OS user is outside the application-isolation
claim; the layout protects ordinary compatibility behavior, not an OS sandbox.

Use one constructor for all registry-selected opens, including legacy direct
`open_with` call sites in connect/resume/listing paths. Normal `--space` selection
continues to work; new `connect LINK` import does not let `TONK_SPACE` substitute a
different identity/site.

Add an explicit scoped authority mode at the existing wrapper boundary. Keep
account mode's `active`, `authorize_guarded`, `Guarded`, and remote `Provider<Fork>`
semantics intact. Scoped mode must select only the imported profile, expected
subject and exact installed grant set; it must not call account initialization,
`local_root_for_store`, prefix recovery or account attachment. Reuse the raw
operator only after proving its isolated proof store cannot contain broader
credentials. Reusing the existing `require_account = false` mechanism may be an
internal implementation detail, but do not expose it as a general account bypass
or use it with the default profile. Guard every remote fork, including alternative
remote subjects, and do not bypass the wrapper via `.inner()` for data work.

`site::mount_delegated_at/with/inner` are unsuitable unchanged: they accept one
chain and inspect local/account/onboarding roots, reusable prefixes and recovery.
Extract/add a dedicated scoped mount using only the reusable lower half:
verifier-only subject credential -> `Space::new(REPO_NAME).create(...)` ->
`profile.repository(REPO_NAME).load()` -> `Reactor::new(profile)` -> scoped wrapper.
Do not run repository bootstrap, membership, ownership or prefix adoption.
Use the same constructor for import and reopen, checking the mounted repository DID.

Operator derivation currently persists a fresh `Subject::any()` session in
`derive_operator_for_profile` and uses a fixed context. Reuse its mechanism only
inside the isolated invitation profile; prove ancestor limits survive it. Prefer
bounded in-memory operator scopes (`DeriveOperator::derive`,
`OperatorBuilder::allow_until`) if adapting the required local capabilities is
small. Operator rotation does not issue new invitation grants or extend expiry.
Local reopen/edit must remain possible after grant expiry; check time/signatures
for new import and remote authorization, not by making downloaded data unreadable.

## Main-only remote wiring is required

`remote::add_with_revocation` writes replicated `meta` concepts;
`remote::set_upstream` also tracks remote `meta`. The six scoped grants exclude
that branch. Reuse the actual main-only operations from
`tests/connections.rs::scoped_site`: `repository.remote("origin").create(...)`,
load remote, `remote.branch("main").open()`, and main `set_upstream`.
Store/discover remote display metadata locally for scoped connections.
`sync::{push,pull}` open local meta but only sync it when it has an upstream;
explicitly preserve the no-meta-upstream invariant after restart and refuse a
scoped remote configuration requesting meta. Keep ordinary account remotes intact.

Audit all commands using `open_selected`, `TonkSite::open/open_with`, `remote`,
`auto_sync`, and wrapper `.inner()`/`.local()` access. Existing eval/query/schema/
view/blob operations can reuse `TonkSite` once selection is correct. Account,
invite-management and ownership commands need explicit scoped-mode refusal before
local management writes, rather than assuming data UCANs prevent local mutations.

## Crash-safe transitions, then process coverage

This live checkout has **no** `StagedDirectory`, registry mutation lock or
`register_existing_bound`. `SpaceStore::save` uses a fixed `spaces.json.tmp` and
rename without fsync/locking. `register_existing_unbound` load/check/save is not
concurrency-safe despite its comment. Do not claim the newer registry machinery
exists here or rely on atomic rename alone to prevent lost updates.

Implement a narrow locked registry transaction shared by mutation paths, with a
unique sibling temp file and file/directory fsync. Follow the barrier pattern in
`HandoffMetadata::save`. Build a sibling staging directory and publish it by rename
only after verifying credential/grants/repository subject. Journal checkpoints:
validated credentials retained -> replica mounted -> remote configured -> pulled
-> registry registered -> directory bound -> confirmation acknowledged. Recovery
checks actual on-disk identities/subject before advancing or reusing a checkpoint.
Keep the registry lock short and out of network awaits. A final combined register
and bind transaction must recheck alias/directory conflicts. Never remove existing
replicas or user files to resolve a collision; preserve interrupted staging state
and unsynced data for resume.

Extend the already registered CLI `tests/connections.rs` with actual subprocess
cases in isolated homes. First prove import -> process exit -> reopen/pull with
exact recipient and no account. Then unrelated account, same-subject separate
replicas/invites, repeated import, missing/tampered credential/binding, expired and
revoked grant with offline edits and no fallback, each journal crash boundary, and
concurrent alias/directory claims. Check no secret-bearing strings in journals,
registry/output and no account/profile changes outside the connection. After the
full data command matrix, add grant-keyed confirmation; the legacy singleton
`id:tonk:agent-connection` must not become the new confirmation identity.
