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

**The sync-base idea does not work, and neither would a force flag.** Both
assume the new build can talk to the old remote at all. It cannot — that is
what the format change broke. Push against a remote upstream does three
things before publishing (`push.rs:176-207`):

1. `fetch()` the upstream revision,
2. `verify()` that head,
3. `TreeDifference::compute(&base_tree, ...)` — which **walks the old tree's
   nodes** to decide what to upload.

Steps 2 and 3 require reading old-format data. No amount of base manipulation
or `.override()` avoids that, so this is not a precondition problem.

**So the migration is local, and the remote is re-established rather than
updated.** The old build exports; the new build imports into a *fresh*
repository and pushes to a remote that has no old head to reconcile with.
The pull half has to happen under the old binary, because only it can read
what is there.

That makes the sequencing concrete, and it is what the fixture test below has
to model: two binaries, not one. The open design question is what the remote
looks like afterwards — a new branch, a new repository, or a deliberately
reset one — and that decides whether peers follow automatically or have to be
re-pointed.

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

## What the upgrade path needs before it can exist

The migration is inherently two binaries: only the old build can read old
data, only the new one can write new data. Everything below follows from
that, and none of it is in place today.

**1. There is no old binary to fall back to.** `tonk update` swaps in place —
`std::fs::rename(temp, target)` (`update/swap.rs:200`) overwrites the running
binary with no `.old` copy and no rollback. The module's "nothing is ever
half-applied" refers to atomicity, not preservation. A user who has already
updated has lost the only thing that can read their data.

**2. Re-installing an old build is not currently possible either.**
`install.sh` supports `TONK_RELEASE=<tag>` to pin an explicit tag
(`install.sh:13`), which would be the escape hatch — except the releases are
**rolling tags**, not versioned ones: `tonk-latest` and `tonk-staging` are
moved in place. `tonk-staging` was updated 2026-08-15, after the dialog
change, so no published tag points at a pre-upgrade build.

**But the tag already exists.** `v0.6.7` points at `d7074057`, pins dialog
`rev = e8bbe462`, and is an ancestor of the bump — it is precisely a
pre-upgrade build. It simply was never *published*: `gh release view v0.6.7`
finds nothing, because the release job only fires on a push to `staging` and
publishes to the rolling `tonk-staging` tag.

So the first requirement is one release, not new machinery:

- **Publish binaries for the existing `v0.6.7` tag.** The CLI workflow already
  allows `workflow_dispatch`, so it can build from that ref without code
  changes; the assets need to land under `v0.6.7` rather than a rolling tag.
  `install.sh` already honours `TONK_RELEASE=<tag>` (`install.sh:13`), so
  users can then fetch the old build with a documented one-liner.

Then the migration itself:

- **Export under the old build** — `tonk export` already exists and goes
  through dialog's `CsvExporter`.
- **Import under the new build into a fresh repository**, since the remote's
  old head cannot be reconciled with (see above: push must `verify()` it and
  diff against its tree).
- **The remote is re-established, not updated.** Import lands in a fresh
  repository and pushes to a remote with no old head to reconcile against.
  This is forced rather than chosen: push must `verify()` the upstream head
  and diff against its tree, and both are unreadable old-format bytes. Peers
  must be re-pointed at the new subject — a real cost, but the alternative is
  pretending a reconciliation exists that cannot.

**Also worth doing, but separately:** teach `tonk update` to keep the previous
binary. It makes every *future* format change survivable without release
archaeology. It cannot help here, though — anyone who has already updated has
already lost the old binary, and they are exactly the population this
migration exists for.

## Tested: CSV import refuses 45% of a real legacy export

Generated a fixture the honest way — built the CLI at `a39b60bb` (dialog
`rev = e8bbe462`) in a worktree, created a spot with an isolated `HOME`, wrote
data, and ran `tonk export`. 1197 rows. Then imported it with the current
build:

```
error: import failed: Artifact decode failed during commit:
Reserved attribute (the dialog. namespace is reserved): dialog.concept.with/source
```

**533 of 1197 rows (45%) are `dialog.*`** — `dialog.concept.with/*`,
`dialog.attribute/*`, `dialog.meta/*`. That is the schema: the concept and
attribute definitions that make the other 664 application rows interpretable.
The new dialog reserves that namespace against application writes
(`dialog-artifacts/src/tree.rs:1327`), and `tonk import` commits through the
ordinary application path, so every one of them is refused.

So CSV round-tripping is **not** sufficient on its own, and this is not a
tuning problem — it is a design gap:

1. **Schema rows need a privileged import path.** Dialog writes `dialog.*`
   through `.machinery()`, which bypasses the reserved-attribute gate (that is
   how `Delegations::retain` commits `dialog.ucan/*`). Import would need the
   same, and deliberately: an import is a system operation, not an application
   write.

2. **Or the schema is re-seeded rather than carried.** Concepts and attributes
   are content-addressed, so re-declaring them under the new build should
   reproduce the same entities — in which case only the 664 application rows
   need to migrate. This is the more likely right answer, and it is testable:
   compare the concept entities a fresh seed produces against the ones in the
   fixture.

Either way the fixture is now real and reproducible, which is what makes the
question answerable rather than speculative.

## The encoding actually has to be remapped, not just re-committed

Comparing the old fixture's `dialog.*` vocabulary against a spot the new build
creates from scratch:

**Gone in the new format** (present in the legacy export, absent from a fresh
new spot):

```
dialog.attribute/cardinality   dialog.effect/conclusion   dialog.meta/concept
dialog.attribute/id            dialog.effect/on           dialog.meta/description
dialog.attribute/type          dialog.effect/polarity     dialog.meta/effect
                               dialog.effect/source       dialog.name/referent
```
plus 46 `dialog.concept.with/*` attributes — one per declared field.

**New in the new format:**

```
dialog.db/revision   dialog.rule/induces   dialog.rule/on   dialog.rule/source
```

So the schema is not merely reserved against import — it is *encoded
differently*. The `dialog.effect/*` family is the clearest: that was the old
rules system, which became `dialog.rule/*` when dialog took rules native.
`dialog.concept.with/<field>` — an attribute per field — has no counterpart at
all in a new spot, meaning concept structure is now stored some other way.

This settles the earlier open question. **Re-seeding beats carrying**, and not
merely because it is simpler: the old rows cannot be committed as they stand
even with a privileged path, because the new build does not model concepts
that way any more. Rewriting 533 rows into whatever the new encoding is would
mean reimplementing dialog's schema layer inside a migration tool, and it
would rot the moment dialog changed again.

So the migration is:

1. **Re-seed the schema** by re-running the source notation under the new
   build — `core.yaml` already lowers at repository creation, so a fresh spot
   arrives with the new-format schema for free.
2. **Carry only application rows** — the 664 non-`dialog.*` rows — which are
   ordinary facts and import cleanly.
3. **Verify identity survived** on those rows: same `of`, same `cause`.

What still needs checking is whether application rows *reference* concept
entities whose addresses changed with the encoding. If `concept:Ake…` in the
old export is not the address the new build derives for the same declaration,
the carried rows point at nothing and step 3 fails. That is the next
measurement, and the fixture makes it a five-minute check rather than a
guess.

## Tested: materialize the schema as notation, re-evaluate under the new build

`tonk schema` already does exactly what the migration needs — "Print the
branch's schema as re-submittable notation" (`bin/tonk.rs:108`). So the schema
does not have to be *translated* between encodings at all: read it out as
source, and let the new build lower it however it now lowers things.

Measured against the real legacy fixture:

- `tonk schema` under the old build → **1057 lines** of notation, 69
  `attribute!:` and 42 `concept!:` blocks.
- Evaluating that under the new build fails on one thing: 18 blocks declare
  **dialog's own vocabulary** (`the: dialog.*`), and three of their fields
  carry no `description`, which the new build requires. Those are dialog
  internals, not user data — the new build defines them itself, differently.
- Dropping those 18 leaves **93 blocks, which evaluate cleanly.**

And the identity question is answered:

```
legacy concepts:    15
reseeded concepts:  38   (the new build seeds more of its own)
PRESERVED:          15
LOST:                0
```

**Every legacy concept address survives re-evaluation.** `concept:5ksSpHB5…`
in the old export is `concept:5ksSpHB5…` after re-seeding under the new build.
Concepts are content-addressed over their declaration, and the declaration
round-trips through notation unchanged — so application rows that reference
them still point at something real.

That makes the migration concrete:

1. `tonk schema` under the old build; drop blocks declaring `dialog.*`.
2. `tonk export` under the old build for the data.
3. Fresh spot under the new build; `tonk eval` the schema; `tonk import` the
   application rows.
4. Push to a re-established remote.

Steps 1-3 are verified end to end against a real pre-upgrade fixture.

**Credentials have their own migration, and it is already wired up.**
`profile.access().migrate()` (`dialog-repository/src/repository/access.rs:9`)
moves a legacy *certificate store* into the access branch as `dialog.ucan/*`
facts — which is exactly the pre-to-post-upgrade delegation move. `tonk
account migrate` already calls it (added earlier on this branch), so the
credential half of the upgrade is a command that exists rather than one to
design.

Two things it does deliberately, both worth knowing: **self-issued
certificates are skipped** (the profile can re-sign those on demand, and
persisting them caused "one session grant per build, forever"), and
**interchangeable certificates compact** to one representative per semantic
payload.

**What the fixture cannot show.** The legacy spot was created with
`TONK_UNSAFE_ALLOW_DEVICE_ROOT` and never linked an account, so its
credential store holds only the repository's own signing key — no certificate
store, no account link, no descriptor. So `access().migrate()` is *the right
tool* by inspection, but this fixture does not exercise it. Proving that half
needs a fixture built by an old build that linked a real account, which needs
an account service the old build can talk to.

## Publishing a pre-upgrade release instead of reverting

Reverting the dialog upgrade to build migrations would undo working, tested
code. Publishing a pinned release is additive, costs nothing to keep, and is
what the migration actually needs — the export half runs under the old build,
so users need a way to *get* the old build.

**The tag already exists and is already correct.** `v0.6.7` points at
`d7074057`, which pins dialog `rev = e8bbe462` and predates the bump. Nothing
needs tagging.

**But a git tag is not enough.** `install.sh` resolves
`TONK_RELEASE=<tag>` to
`https://github.com/<repo>/releases/download/<tag>/<asset>` (`install.sh:79`),
so it needs a *published release with binary assets*. A bare tag 404s.
`gh release view v0.6.7` currently finds nothing.

**The workflow is one condition away from being able to do this.**
`cli.yml` already allows `workflow_dispatch`, and the build job produces the
platform binaries. The publish job does not run, because it is gated:

```yaml
if: github.event_name == 'push' && github.ref == 'refs/heads/staging'
```

and it hardcodes `tag_name: tonk-staging`, deleting and recreating that
rolling tag each time.

Two options, in preference order:

1. **Publish `v0.6.7` once, by hand.** Run the workflow on the `v0.6.7` ref to
   get the binaries as CI artifacts, download them, and attach them to a
   release created at that tag with `gh release create v0.6.7`. No workflow
   change; one maintainer action; produces exactly the immutable release the
   installer needs.

2. **Teach the workflow to publish version tags.** Let a dispatch carry a tag
   input and publish there instead of `tonk-staging`. Better long-term — every
   future format change gets a downloadable predecessor for free — but it
   changes release plumbing, which deserves its own PR rather than riding
   along with a migration.

Either way the user-facing instruction becomes a one-liner that already works
today:

```
curl -fsSL https://github.com/tonk-labs/tonk/releases/latest/download/install.sh \
  | TONK_RELEASE=v0.6.7 sh
```

## Publishing a pre-upgrade release: what worked, what is blocked

**Automated** (`.github/workflows/cli-pin.yml`). Dispatching `cli.yml` on an
old tag fails — GitHub reads `workflow_dispatch` inputs from the file *at the
dispatched ref*, so `release_tag` does not exist back there ("Unexpected
inputs provided", HTTP 422), and history cannot be edited to add it. The new
workflow inverts that: it lives on the default branch, so its inputs always
exist, and *checks out* the ref being published. Any point in history becomes
publishable.

**Blocked on merge.** `workflow_dispatch` only registers from the default
branch — dispatching it from a feature branch returns "workflow cli-pin.yml
not found on the default branch". So it cannot run until this lands on
`staging`. Nothing else gates it.

**Verified locally in the meantime.** The old CLI builds reproducibly through
nix from the `v0.6.7` worktree (`nix build .#tonk-cli`), reports `tonk 0.6.7`,
and has source-tree hash `yrv5fymlyxdz` — distinct from the current tree's
`4m5frrhgwdgz`, which is what makes content addressing work as a name.

**Not doable by hand.** Staging the release manually only produces the macOS
archive; there is no way to cross-compile the Linux binary here, and a release
missing it would break Linux users. So the manual route is not a shortcut past
the merge — the workflow is the way.