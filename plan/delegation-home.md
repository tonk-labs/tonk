# Delegations live in the account repository

Move access recovery from R2 chain blobs to synced facts, per the
2026-08-14 journal entry:

> the account repository is the durable home of delegations, and login
> is data flow: retain the account to profile powerline on the device,
> add the account as the upstream of the profile's access branch, and
> pull. Access comes back because the delegations are just facts in a
> branch you can sync.

## Why this is small

Almost every piece already exists. What is missing is one edge.

**Dialog implements the whole flow and pins it with a test.**
`dialog-repository/src/repository/branch/integration_tests.rs:1298`,
`it_regains_access_by_pulling_the_account`, walks exactly the four steps
above and asserts a three-hop proof, `space → account → profile →
operator`. There is no new dialog capability to build or request.

**Tonk already writes delegations as facts.**
`rust/tonk-worker/src/router/repository.rs:2688` does
`profile.access().save(UcanDelegation(prefix))` on every space creation.
Those facts land on the profile repository's `main` (dialog's
`ACCESS_BRANCH`), and `dialog-operator`'s proof walk already reads them.

**Tonk already mounts and syncs an account repository.**
`rust/tonk-worker/src/router/account_state.rs:287` sets its upstream and
registers it with the reactor's pull population.

**So the gap is one missing edge.** The account repo currently syncs only
with its own remote, which is why it carries nothing but a display name
and passkey metadata. Nothing connects the *profile's access branch* to
the *account repository*. Recovery therefore cannot read the facts and
falls back to fetching hex chain blobs over HTTP
(`rust/tonk-worker/src/router/account_backup.rs:79,99`).

## Constraints discovered

These shape the stages; each is load-bearing.

1. **`ACCESS_BRANCH == "main"`** (`dialog-repository/src/repository/access.rs:64`).
   The profile's access branch is also its content branch. Delegation
   facts and application facts share one head. Safe by construction —
   `dialog.*` is reserved against application writes
   (`dialog-artifacts/src/tree.rs:1327`) — but the head is contended,
   which is why dialog's retain path refreshes and retries.

2. **Cross-repo upstream must be `Remote`, never `Local`.**
   `Upstream::Local` resolves against the pulling branch's *own* subject
   (`pull.rs:174`), so it can only name a sibling branch in the same
   repository. Pointing at the account requires
   `repository.remote(name).create(site).subject(account_did)`.

3. **Retain before pull.** The pull is itself an authorized fetch, and
   the anchor operator that authorizes it carries an empty reach
   (`dialog-operator/src/operator/builder.rs:161`). The account→profile
   grant has to be local *before* the pull, or the walk cannot authorize
   the fetch it needs.

4. **A bare pull adopts by reference.** Envelope bytes replicate lazily
   on first read. Offline or without a reach, `admit()` silently skips
   candidates (`prove.rs:325`) and the caller sees `UnprovenSubject`
   rather than an explanation. `pull().download()` forces materialization
   and is the right default for login.

5. **The operator caches its access-branch handle** at build
   (`builder.rs:141`). Pulling through a different `Branch` handle is
   only visible after the operator's own refresh, and there is no public
   invalidation hook.

## Stages

Each stage ships and is verifiable on its own.

### 1. Give the account repo the delegation facts

On space creation, retain the space→root prefix into the *account*
repository as well as the profile's access branch. This is the same
`Retain` effect already used, with the account repo's branch as target.

Verify: create a space, then read `dialog.ucan/*` facts back off the
account branch. The R2 backup stays untouched.

### 2. Make login a pull

At sign-in on a new device, follow dialog's pinned sequence: retain the
account→profile powerline (`subject: Any`, `command: []`), create a
remote in the profile repo with `.subject(account_did)`, set it as the
upstream of the profile's access branch, and `pull().download()`.

Verify: a second device with no R2 access recovers its spaces. This is
the stage that makes the design real; it is also the natural place for
an end-to-end test mirroring dialog's.

### 3. Read facts first, R2 as fallback

Recovery tries the synced facts and falls back to `/chains/list` when
they yield nothing. Existing accounts have chains in R2 and no facts
anywhere, so this phase is what keeps them working.

Verify: a legacy account (R2 only) and a new account (facts only) both
recover.

### 4. Backfill, then retire the R2 path

Once a device has both, write the facts it learned from R2 into the
account repo. When telemetry or a migration window says the fallback is
unused, delete `chains/put|list|get|spots`, `AccountSpotBackup`, and the
R2 chain store.

Verify: a legacy account that has signed in once recovers with the
fallback disabled.

## Deliberately out of scope

Two other clusters diverge from the journal and are **not** addressed
here, because neither blocks this one:

- **Identity model** — passkey-as-PRF versus the May 12 varsig design
  where the WebAuthn assertion is itself a signature, and immutable
  `did:key` versus Aug 14's "principal with a mutable key set". Blocked
  on the unsettled did:plc / did:web question.
- **Account versus customer** — email-gated account creation and
  account-before-spot. Note `rust/tonk-ui/src/account_gate.rs` argues
  *against* the journal's position in its own doc comment, so this is a
  disagreement to settle rather than a gap to close.

## Open question

A cloud session, "Account keys and device delegation", exists
server-side and was not readable from this machine. If it settled
anything about the above, it should be folded in before stage 2.

## Follow-up: the authorizing page must retain and push

`--via` gives the CLI a grant, but the *page* currently only hands it over.
It should also:

1. **Retain the grant into the account space** — `dialog.ucan/*` facts, the
   same `retain_space_delegation` path a space creation uses. Otherwise the
   only copy of the `account → CLI profile` delegation is on the CLI, and a
   third device pulling the account learns nothing about it.
2. **Push the account branch**, so the retained grant leaves the browser.

Without both, the CLI is authorized but the account has no record of the
device — the roster is incomplete and the grant cannot be revoked by pulling
the account, only by the CLI forgetting it.

The page lives in `rust/tonk-ui/src/account.rs` (`/account/link`), which is
where `audience=` / `callback=` handling and this retain+push belong.

## Follow-up: move the account UI into the profile iframe

`/account` renders in the top document today (`bin/ui.rs:52`: "Account routes
bypass sealed guests because WebAuthn ceremonies must run in the RP ID's
top-level origin"). That constraint is real but narrow — it binds the
`navigator.credentials` call, not the surrounding UI — so the whole account
page currently forgoes the isolation the rest of the app has to satisfy one
function call.

The split: the profile iframe renders the UI and, when a ceremony is needed,
sends a **signing request** to the top document, which derives the
passkey-backed key, signs, and returns only the signature. The key and the PRF
output never cross the boundary — which is what makes this safe, and is the
shape `signRevocation` already follows (it takes a target, returns a
signature, never exposes the root).

Prerequisites: `tonk-account` is not registered in the guest bundle today, and
`tonk_identity::install()` stays in the top document as the signing authority.

## Blocker: the account path resolves its own operator from the install

Driving `account link` from a test is blocked by a pervasive coupling, not a
missing hook. Nine call sites across `account.rs` (5), `account_state.rs` (2),
and `identity.rs` (2) call `credential_operator(profile)`, which does:

```rust
let store = SpotStore::open()?;          // the real install directory
let mounted = Profile::load(PROFILE_NAME).at(Directory::Profile)...
if mounted.did() != profile.did() { bail!("account-state profile does not
match the active CLI profile") }
```

So a caller that already holds a profile and operator cannot use them: the
function re-mounts a profile *by name* from the install and rejects the
caller's. Threading an operator through one layer just moves the failure to
the next — `link` → `ensure` → `mount` → `save_local_root`, each resolving its
own.

Partial work exists (uncommitted): `link_with_operator`,
`ensure_with_operator`, `ensure_with_operator_and_store`, and a
`LinkOptions::announce` channel that hands a caller the approval URL (which
carries a callback address only `link` knows, so a test cannot construct it).
The callback round trip itself **works** — a stand-in page answers the URL and
`link` receives and validates the grant.

The fix is to take `(profile, operator)` as parameters throughout the account
path and keep `credential_operator` as the thin binary-side default, rather
than as the way every function acquires its operator. That is a refactor of
its own, not something to fold into this branch.

Until then `it_discovers_a_space_through_the_account` exercises the steps
directly rather than through `link`, which is weaker: it pins that the
authority works, not that the command performs the dance.

## Next: a CSV migration path for old-format data

Same shape as the pre-dialog-upgrade migration: rather than teach new code to
read an old on-disk layout, round-trip the *data* through a
format-independent intermediate. Export under the old build, import under the
new one.

**Most of it already exists.** `tonk export` and `tonk import` (both hidden,
`rust/tonk-cli/src/transfer.rs`) go through dialog's own `CsvExporter` /
`CsvImporter` over `(the, of, as, is, cause)` columns, committing rows as
assertions on `main` in one transaction. `tonk migrate` is the precedent for
the command shape, having moved `.carry/` to `.tonk/`.

**Scope: data repositories first, the profile repository too.** A spot's own
data is the point; the profile repository carries the local root and account
link, so an upgrade that restores data without it leaves an instance with its
spots and no authority over them.

**Branch: `main` by default, with a flag to name another.** Export is
`main`-only today, which is the right default — a flag covers meta and
history without making the common case verbose.

**Identity must be retained.** This is an upgrade path for instances that
predate the DB change, not a copy tool: a re-imported spot that peers treat
as a *different* spot is a failed upgrade, not a partial one.

Encouragingly, dialog already does this. Its own round-trip test
(`dialog-csv/src/lib.rs:77`) asserts `the`, `of`, `is`, **and `cause`** all
survive — so the entity and the causal version are preserved by construction,
not by luck. What still needs checking is whether that holds through *our*
export/import path on a real spot rather than over hand-built artifacts.

**There is no force push, and re-import needs to replace a head.** Dialog states it
plainly — "Push is fast-forward only" (`branch/push.rs:61`) — with
`PushError::NonFastForward` and no force option anywhere in
`dialog-repository`. Re-importing rebuilds the tree, so the upgraded spot's
head is not a descendant of what the remote already holds: an ordinary push
refuses it, exactly as observed while testing the account branch.

Two answers, and they are complementary rather than alternatives.

**1. Adopt the remote head as the precondition (works today).** Push does not
compare against the remote's ancestry — it compares the *locally recorded*
sync point against what the remote currently holds:

```rust
let base = upstream_state.tree().clone();          // push.rs:120
let current = upstream.revision().map(|r| r.tree).unwrap_or_default();
if current != base { return Err(PushError::NonFastForward { .. }) }
```

So refreshing that recorded base to the remote's current head satisfies the
check and the reset proceeds. `Upstream::with_tree` (`upstream.rs:47`) is
public, so this needs no dialog change at all. It is also the *better*
semantic: the precondition is still checked, against a head we deliberately
read, so a concurrent writer between the read and the push is still caught —
compare-and-swap rather than clobber.

Better still, option 1 needs no new plumbing either: `pull` already does the
base refresh. When the local context includes everything upstream has seen,
it takes the branch that "keep[s] the head and advance[s] only the sync base"
(`pull.rs:299`) — the local head is untouched, the precondition is refreshed.
So the migration sequence is the ordinary one: **pull, then push.**

Crucially this keeps the safety property. The base is refreshed from a head
we deliberately fetched, so a writer that moves the remote *between* that
fetch and the push still trips `NonFastForward`. A force flag would have
clobbered them. Compare-and-swap, not override — which is why no dialog
change is wanted here.

Either beats the alternative of deleting the remote branch and re-pushing,
which loses the very history the import exists to carry across.

**The open question is what CSV does not carry.** The columns are artifact
facts. Delegations are now `dialog.ucan/*` facts and may round-trip, but the
account descriptor, the trusted-base marker, and the local root live in the
credential store, not on a branch. Verify by round-tripping a real spot and
diffing, rather than reasoning from the column list.


## The migration test: a fixture written by the OLD dialog

An upgrade path is only as good as the evidence it upgrades something real,
so the test needs a spot **populated by the pre-upgrade dialog**, not one
this build wrote and then re-read. Reasoning about format compatibility from
the current code cannot show that.

Shape:

1. Check in a fixture exported from a spot created under the old dialog —
   CSV plus whatever credential-store state the upgrade must carry.
2. Import it under the current build into a fresh spot.
3. Assert the data is *there* (rows round-trip) and that identity survived:
   same entity, same `cause`. A spot that comes back as a different spot to
   its peers is a failed upgrade, not a partial one.
4. Push to a live access service, exercising the pull-then-push sequence
   above — the step that proves the rebuilt tree can actually reach a remote
   that already holds the old head.

Generating the fixture needs an old build to run once; committing its output
is what makes the test reproducible afterwards.