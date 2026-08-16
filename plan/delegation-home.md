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
