# Plan 012: Promote verified candidates through staging to stable

Status: READINESS IMPLEMENTED LOCALLY; hosted product validation pending.
Enforcement and channel migration deferred by user decision on 2026-10-13.

Planned at `e7388a98f7af4f0d680fe682fed7821466901d0c`, 2026-10-13.
Priority P1; effort L; implementation risk high because publication spans
Cloudflare, GitHub Releases, npm, and installed CLI update channels.
Dependencies: none of the other implementation plans. Execute the checkpoints
below in order. Browser and CLI are the required surfaces for this first version;
native Desktop and the real ChatGPT connection are explicitly outside its claim.

The user authorized implementation after reviewing the design, then selected
finishing readiness before channel migration. Synthetic test repository changes
are authorized; production publication and policy cutover remain deferred.
See [the readiness runbook](../docs/releases.md) for implemented behavior and
[execution evidence](012-release-gates-execution.md) for verification boundaries.

## Outcome and operator experience

Provide one **Release readiness** entry in GitHub Actions. Each candidate has a
report naming its source commit, intended version, checks, deployed identity,
manual verification, and the precise reasons promotion is blocked.

1. Select a commit already merged to `main` and a release version. The workflow
   freezes that selection and prepares a candidate; later commits on main do
   not silently replace it.
2. Open its report, inspect the automated results and running-product evidence,
   complete any required manual checks, and approve **Promote to staging**.
3. The workflow publishes staging and records every delivery's outcome. Verify
   the actual staging installation using the same report.
4. Approve **Promote to stable** for that staging candidate. Stable cannot take
   an arbitrary main commit, another candidate's results, or a failed partial
   staging publication.

`main` is the development branch. Staging and stable are release environments
and channels, not independently developed branches. Retain existing branches
initially as clearly documented legacy refs; do not delete or repoint them as
part of the rollout. PR merges, fast-forwards, tag pushes, and the GitHub
"New release" button are not supported promotion commands.

Use the Actions run summary as the first UI, with a downloadable self-contained
HTML report for screenshots and detailed flow evidence. This provides a useful
CI-adjacent destination without first building an authenticated web application.
A later hosted release center can render the same data. Its buttons must use
GitHub authorization; client-side checkboxes never grant publication authority.

## Current state and verified gaps

Source inspection and read-only GitHub API inspection on 2026-10-13 found:

| Finding | Evidence | Consequence |
| --- | --- | --- |
| A tag independently starts three publishers | `.github/workflows/release.yml:20`, `publish.yml:13`, `cli-npm.yml:12` use `push.tags: ['v*']` | Web can ship while CLI builds or tests fail; there is no overall promotion transaction. |
| Release resolution requires ancestry, not readiness | `.github/actions/resolve-release/action.yml` calls `git merge-base --is-ancestor "$commit" origin/main` | Being on main does not prove CI, staging, or manual review passed. |
| Tags also determine version and channel | Resolver stamps Cargo.toml/Cargo.lock; suffix means staging | Preserve existing version semantics during the first rollout. |
| Main CI and release CI are separate | `test.yml` runs on main/PR/dispatch; `cli.yml` only on PRs; MCP CI has path filters | Do not reuse a PR merge SHA's success for a different candidate or interpret absent runs as success. |
| Browser E2E allows retries | `.config/nextest.toml`, `profile.e2e`, has `retries = 2` | Ordinary green can include flakes. Release evidence must expose attempts. |
| Storybook is an inventory, not live verification | `docs/storybook/README.md`, `goal.md`, `verification/README.md` | Reuse journey IDs, not its drafted/verified text as release authority. Its explorer is currently local, not deployed. |
| Release channels also depend on metadata | `install.sh`, `rust/tonk-cli/src/update.rs` use GitHub latest / tonk-staging | Audit these consumers when changing publication ordering. |
| Historical CLI publisher accepts an arbitrary tag | `.github/workflows/cli-pin.yml` accepts `inputs.tag` | Restrict it to a separate namespace so it cannot overwrite release assets or rolling channels. |

Live policy snapshot: rulesets 3652307 and 12020040 both select
`~DEFAULT_BRANCH`, including the one named "Stable branch rules". No tag
ruleset was returned. `main` was protected; `staging` and `stable` were both at
`503a5d6373f51b943d82d94d87cc3860f4c6b8e2` and reported unprotected. No staging or
stable approval environment was returned. Recheck all of this during rollout;
these are observations, not configuration to copy blindly.

## Promotion contract

### Identity and evidence

Create one versioned candidate record with:

- repository, full source SHA, candidate ID, intended semver, RC sequence;
- workflow SHA, policy SHA, Actions run ID and attempt;
- release artifact identities and SHA-256 digests;
- test inventory, required flow IDs, exact test IDs, and evidence by layer;
- deployment IDs, origins, artifact/build identity, and deployment time;
- manual reviewer identity, verification time, evidence URL, result and notes;
- promotion approval and per-destination publication receipts.

Treat report data as evidence to validate, not trusted instructions. The
privileged controller obtains run identity/conclusion and artifact identity
from GitHub, verifies the expected repository/workflow/commit/attempt, and
does not execute scripts from downloaded reports. Credentials are unavailable
to report generation and test parsing. The policy comes from the trusted
controller revision; the source candidate cannot reduce its required checks.

Statuses: `passed`, `failed`, `flaky`, `blocked`, `missing`, `pending`. Only
`passed` satisfies a required check. A skipped test, empty suite, missing shard,
expired artifact, cancelled run, malformed record, unavailable API, or unknown
flow ID blocks promotion. A flow passes only if all its required evidence
layers pass. Historical screenshots and source fixtures remain labeled and do
not satisfy running-product checks.

Use exact discovered test IDs, including test binary identity, rather than
substrings that can silently match zero tests. Store expected inventory and
results together. Never merge reports from different runs or attempts to make
one artificially green run. A new attempt supersedes the old report and needs
fresh approval. Preserve earlier failures visibly in candidate history.

Automated results remain valid for their immutable artifact and policy.
Deployed/manual results additionally expire after 24 hours or whenever the
deployment ID, configuration identity, required checklist, or artifacts change.
This 24-hour window is a proposed default, configurable in reviewed policy.
Do not expire approval merely because unrelated main commits land.

### Gate A: main to staging

Require candidate ancestry on main, all required native/web tests and lints,
browser/CLI journey results, service-worker lifecycle tests, packaged CLI
portability/signature/install/update smoke, and candidate visual review.
Include the MCP core contract and ordinary-worker integration checks because
they exercise the shared worker; this does not claim real ChatGPT verification.
Run required checks explicitly for the candidate rather than discovering
arbitrary green check names. Path-filtered or PR-only checks must be callable
by the candidate workflow with an explicit source SHA.

Before the approval job, render candidate artifacts in the existing isolated
test harness and retain screenshots. If an interactive review requires a
deployed candidate, use a dedicated disposable candidate deployment with its
own access databases; do not use a moving main URL as candidate evidence.
Reviewers record required manual results through an authenticated verification
workflow. Then the protected staging environment approval authorizes delivery.

### Gate B: staging to stable

Require successful publication to every staging destination, fresh deployment
verification for that candidate, and stable-environment approval. Resolve the
source only from the staging receipt. Reject independent SHA or version inputs.
Reject an older candidate superseded on staging; serialize promotions to avoid
one operator verifying a deployment another operator replaces mid-review.

Stable tags use the same source commit as the staged RC. Existing version
stamping means RC and final binaries differ; do not claim byte-identical
promotion. Build final-version artifacts before stable approval, run packaged
artifact checks and the required candidate journeys against those artifacts,
and record their hashes. Environment-dependent web bundles likewise need
explicit build identities. No unreviewed source fix belongs in this step:
any source change creates a new staging candidate.

After approval, revalidate policy, evidence expiry, source/artifact identities,
staging receipt and supersession before the first external mutation. Disable
cancel-in-progress for delivery; queued requests revalidate after acquiring the
channel lock. Also hold staging replacement while a stable promotion is using
its receipt. A single controller concurrency group is acceptable initially.

## Required browser and CLI flows

The inventory below is the proposed minimum gate, not a claim of current
coverage. Check existing implementations before assigning test IDs. Missing
automation can be covered by explicit manual verification with evidence, never
by declaring the whole flow green because a related unit test passed.

| Flow group | Existing anchors | Required outcome |
| --- | --- | --- |
| Account creation and activation | ACCT-B02–B06; `it_finishes_signup_in_the_original_tab` | Create passkey, activate, reach usable account; reject/cancel and retry safely. |
| Login, logout and account switch | ACCT-B07/B08; account_flow sign-in/switch tests | Returning account restores its spaces; unrelated retained spaces stay separate. |
| Local work and reload | SPACE-03/07/08; hub create/remove tests | Create and edit while signed out; reload retains data; removal respects its boundary. |
| Hosted sync and recovery | SYNC-01–04; `it_syncs_a_local_only_space_once_the_account_reconciles` | Write, observe on another actor, restart, and recover from offline without data loss. |
| Share, join, revoke and public access | COLLAB-01–06 | Recipient can access the intended space; revocation/unpublishing removes remote access. |
| Browser-to-CLI account linking | ACCT-C family; `it_authorizes_a_waiting_cli_from_the_browser`, `it_declines_a_waiting_cli_from_the_browser` | Approval completes real callback and usable CLI state; decline/timeout terminates clearly. |
| Tool invitation and CLI work | ACCT-C14; `it_connects_with_an_ordinary_bearer_after_the_issuer_closes` | Join, read/write, restart; revocation denies remote access without losing local data. |
| Account/device deletion boundaries | AUTH family; `it_deletes_the_account_and_releases_its_email_and_profile` | Correct authority/data removed; unrelated account and local data preserved. |
| CLI distribution | CLI-08; `cli-smoke.yml`, `scripts/check-cli.py` | Signed/notarized macOS arm64 and Linux glibc/musl install, launch and update correctly. |
| Browser update and visual consistency | `test:sw`; account_flow responsive/ceremony tests | Returning browser reaches candidate build; signup/settings/hub/share/link states agree visually. |

Keep the full Storybook inventory linked as coverage context. The report must
say "required release flows" rather than "all product flows" unless the entire
inventory has actually been verified. New supported flows require a deliberate
policy classification; tests renamed or removed must fail mapping validation.

At gate B, add actual staging browser signup/login/passkey, browser-to-installed
CLI handoff, tool connection, two-actor read/write/sync, and returning-browser
upgrade checks. Use isolated test accounts and spaces. Destructive matrices
belong in disposable infrastructure, not real user accounts. Real authenticator
and human visual review can remain manual in v1. Local virtual-authenticator
success does not establish the real staging passkey path.

## Readiness report layout

The first screen contains candidate/version/SHA, target channel, readiness,
blocker count, and the action permitted next. Then show one row per required
flow, with separate automated, deployed, and human verification columns.
Expand a row for test attempts, environment, screenshots/logs, timestamps and
expected durable outcome. Include links to the source workflow run, deployed
candidate, and exact Storybook journey. Show delivery state separately from
test readiness; an approved release with npm pending is not complete.

For example (illustrative only, not real results):

| Flow | Automated candidate | Staging verification | Review |
| --- | --- | --- | --- |
| Account signup | passed | passed | recorded |
| CLI linking | passed | blocked: callback did not finish | required |
| Share and revoke | flaky: passed on retry | missing | required |

This candidate would show **Stable promotion blocked: 2 flows**. Reports use
text as well as visual indicators. HTML must escape logs, labels and URLs and
contain no tokens, callback payloads or account personal data. Reuse Tonk's
existing visual tokens if creating a custom page; DESIGN.md explicitly requires
status to be expressed through words, geometry or motion, not color alone.

## Publication and bypass prevention

One trusted `promote.yml` controller is the entry point. Convert publishers to
explicit reusable jobs with pinned candidate inputs and dependencies. Retain
`cli-npm.yml` as the npm publisher: its filename is part of Trusted Publishing
configuration. Preserve normal PR previews, cleanup and main development
deployment in `publish.yml`; remove only release-triggered delivery paths.

Do not rely on pushing a tag to start child workflows. GITHUB_TOKEN-generated
pushes do not trigger ordinary workflows. Use explicit reusable workflow calls
and dependencies so all destinations belong to the same promotion graph.

Configure protected `release-staging` and `release-stable` environments with
named maintainers as required reviewers, restricted to the trusted controller
branch. Allow self-review initially so a sole releaser can prepare and approve;
require the explicit approval step regardless. A two-person rule is a separate
team policy. Add a startup audit that fails closed if required environment
protections or tag rules are missing; merely naming an environment creates no
reviewer protection.

Use a dedicated release GitHub App with narrowly scoped repository permissions
as the sole normal bypass actor for creation of `v*` tags. Separate rules
prohibit updates/deletions of version tags even for the normal release app.
Put its credential only in the protected release environments. Scope the
rolling `tonk-staging` ref separately because it intentionally moves. Restrict
historical CLI publishing to `tonk-pin-*`, reject existing tags, set explicit
target SHA, and prevent changing latest or rolling channels.

Move staging/production delivery credentials out of broadly available repo
secrets. Use distinct least-privilege preview/dev credentials, so an old tag's
workflow cannot still deploy production with a historical copy of the YAML.
Verify npm Trusted Publishing and registry permissions during rollout too.
Protect controller/policy changes through main review and ownership rules.

Important limit: tag protection is not protection of existing GitHub Release
metadata. A repository writer can potentially edit releases or latest, and
current installers/updaters follow that metadata. Before claiming the normal
operator bypass is closed, test that path and restrict release-management
authority to the automation/release custodians. If the repository's role model
cannot provide that separation, choose a separately protected channel manifest
for install/update resolution in a follow-up design and expand implementation
scope explicitly. Do not describe an Actions-only check as solving this issue.
Administrators who can edit policy or credentials remain a trust boundary.

Delivery is not atomic across services. Build and verify everything first, then
record each external mutation and reconcile its actual result before retrying.
Publish immutable artifacts/packages before moving channel pointers. Move npm
dist-tags and GitHub latest/rolling aliases only when prerequisites are complete.
Mark partial publication visibly, block subsequent promotion and offer resume
for the same candidate; never blindly retry a lost response or overwrite
different existing bytes. Stage migration/schema compatibility before code;
automatic rollback must never assume database migrations are reversible.

## Scope and conventions for implementation

Modify only `.github/workflows/{test,cli,cli-smoke,mcp,release,publish,cli-npm,cli-pin}.yml`,
`.github/actions/resolve-release/action.yml`, `.config/nextest.toml`, `flake.nix`,
the root README release documentation, and the new paths named below. Storybook
edits are limited to accurate journey links and release-readiness explanation;
follow its AGENTS.md and run its validators if it is changed. No product Rust
or lockfile changes, design-system overhaul, test deletion, or blanket removal
of Storybook PR requirements belongs in this plan. Missing product regression
tests get separate small follow-up plans rather than arbitrary product fixes.

Use Python standard-library parsing and unittest for policy/report tooling,
following `scripts/check-cli.py` and `scripts/tests/test-cli.py`. Match existing
Nix commands and artifact smoke patterns. Shell inputs go through env vars;
never interpolate user inputs into executable shell. Make each checkpoint
independently reviewable and commit separately when implementation commits are
authorized. Preserve all unrelated changes; do not publish during local work.

## Ordered implementation checkpoints

### 1. Prove a gate rejects incomplete evidence without any publishing access

Create `release/flows.json`, `scripts/release_gate.py`, and
`scripts/tests/test-release-gate.py`. Define the versioned evidence schema and
pure evaluator above. Map one actual signup and one CLI-linking test first,
then expand to the proposed minimum inventory after discovery. Make the output
deterministic JSON plus Markdown. Use explicit fixtures marked synthetic.

Tests must cover wrong SHA/run/attempt/policy, missing test/shard, duplicate or
unknown IDs, skipped/empty suites, failure, flaky success, stale deployment,
superseded staging candidate, expired manual review, and complete success.
Test incompatible artifact identities, not just status booleans.

Verify (new command): `python3 scripts/tests/test-release-gate.py` exits 0.
Negative fixture CLI invocations exit nonzero with specific blockers; the
complete synthetic fixture exits 0. No GitHub/network writes are possible.

### 2. Publish real candidate evidence and the readiness report

Add JUnit output to the nextest E2E profile and always upload uniquely named
shard reports/inventories. Extend workflows with reusable entry points that
check out the explicit source SHA. Preserve ordinary PR execution and context
names. Add `.github/workflows/release-readiness.yml` to run the complete required
candidate checks without publishing privileges. Emit a report even on failure;
its final aggregate check must still fail when any required result is missing.

Create `scripts/release_report.py` and report tests; produce Markdown summary,
JSON and self-contained HTML from one validated model. Link from README to the
Actions workflow. Do not require custom hosting for the first usable version.
Disable retries for required release journeys, or consume nextest's structured
flaky markers as blockers; leave normal PR retry policy alone. Check the pinned
nextest supports the selected format before using newer configuration fields.

The existing `test:e2e` command always selects its own Nix CLI and preview UI
artifacts. It cannot prove it tested a supplied release bundle. Add an explicit
`test:release-e2e` command in `flake.nix` that requires candidate CLI/UI paths,
verifies their recorded digests, and passes those exact paths to the existing
test harness without rebuilding or replacing them. Reuse the test archive from
the candidate SHA. If the harness cannot run a final bundle because its origin
configuration differs, stop and specify the required isolated deployment test;
do not substitute preview results and label them final-artifact evidence.

Verification: new report tests and existing `python3 scripts/tests/test-cli.py`
pass. Run `nix develop --accept-flake-config .#ci --command test:e2e --retries 0`
at the integration checkpoint and verify emitted test identity against inventory.
Run existing `test:sw` and the four native/web debug/release commands through
candidate CI. A purposely missing shard must produce a failing readiness check
and a readable report. Never infer results from workflow syntax validation.

### 3. Add authenticated manual verification and gate evaluation

Create `.github/workflows/release-verify.yml` and
`scripts/release_evidence.py`. Inputs identify candidate, required flow/group,
result, evidence URL and short notes. Resolve actor and timestamps from GitHub,
verify reviewer authorization, and store append-only records as workflow
artifacts bound to candidate/deployment/policy digests. Browser checkboxes,
arbitrary issue comments and report JSON cannot stand in for authorization.
Expired records require a new verification; report regeneration cannot refresh
their timestamp. Use artifact retention sufficient for the release cycle;
expiration blocks promotion until recollected.

Verify with mocked API responses and a disposable GitHub test repository:
unauthorized reviewer, wrong deployment, stale evidence and edited inputs all
fail; an authorized current record updates only its declared flow. Generated
report HTML escaping tests pass. Inspect the HTML in a browser at desktop and
narrow widths with passed, failed, missing and flaky fixtures, labeled as such.

### 4. Wire one promotion controller and close alternate publishers

Create `.github/workflows/promote.yml`, `scripts/release_promote.py`,
`scripts/release_policy.py`, and `scripts/tests/test-release-promote.py`.
Separate candidate checking/building, environment approval, post-approval
revalidation and publication jobs. Pass only validated immutable identities
between them. Reuse release building and smoke jobs; coordinate npm, web and
GitHub publication, with a receipt for each effect. Store completed candidate
manifests/evidence with the immutable release for longer-term audit.

Remove `v*` push release triggers only after the new path has passed disposable
tests. Narrow cli-pin inputs as described. Add read-only policy audit and
reviewable environment/ruleset templates under `release/github/`, plus a
runbook at `docs/releases.md` describing prepare, verify, promote, resume and
rollback. Explain legacy branches and eliminate conflicting "cut a tag" advice.

Verify new promotion tests cover race/supersession, stale approval, malformed
semver, version mismatch/downgrade, tag collision, missing protections,
wrong-token permissions, old workflow attempts, partial npm publication, lost
API response and safe resume. Workflow contract tests must reject any remaining
tag/release-event publication path, unsafe cli-pin namespace, or publisher
without the validated promotion dependency. Use actionlint if available and
`nix develop --accept-flake-config .#ci --command lint` for repository checks.

In a disposable GitHub repository/environment, prove: direct tag/UI release
creation cannot ship; branch movement cannot ship; missing evidence cannot
ship; approval of candidate A cannot ship B; non-reviewers cannot approve; a
stale waiting approval fails revalidation; partial delivery cannot become
stable; only the intended app can create a version tag. Explicitly exercise
existing-release metadata changes and installed-client channel resolution.
Record command transcripts and run URLs in this plan's execution log.

### 5. Roll out protections and exercise staging before stable

First run readiness in observation mode; it reports genuine failures and is
explicitly labeled non-enforcing. Resolve coverage gaps without making blocked
checks optional merely to obtain green. Prepare live policy/credential changes
as a reviewed deployment operation, including credential rotation and a pause
on legacy release creation during cutover. Do not leave both systems active.

Verify read-only `scripts/release_policy.py` against the live repository after
configuration: it must report every required rule/environment/permission
present. Retest the bypass matrix in disposable infrastructure with the same
policy. Run a real staging candidate with authorized publication, inspect its
browser/CLI flows, and exercise resume after a safely injected test-environment
failure. A stable release is a later separately selected operational action.

## Done criteria and stopping conditions

Implementation is complete only when:

- policy, evidence, report and promotion tests pass after the final change;
- required candidate CI and signed CLI artifact smoke actually execute;
- the report lists every required flow and its exact evidence and blockers;
- missing/stale/flaky evidence demonstrably prevents each gate;
- disposable promotion and bypass tests pass with recorded run URLs;
- live configuration audit passes and the staging pilot is verified;
- runbook has one promotion path and accurately describes partial recovery;
- plan/index distinguish local tests, hosted proof and unrun stable delivery.

Stop and report if live permissions cannot isolate publishing credentials, the
GitHub role model leaves a required channel-pointer bypass, the installed
nextest cannot produce trustworthy evidence, a required flow lacks an executable
or reproducible manual procedure, or implementing a stage requires product-code
changes outside scope. Do not silently weaken a gate or invent pass results.

Before implementation run `git status --short` and
`git diff --stat e7388a98f7af4f0d680fe682fed7821466901d0c..HEAD -- .github .config/nextest.toml flake.nix scripts docs/storybook install.sh rust/tonk-cli/src/update.rs`.
Reconcile changed release behavior before applying these steps.

## Alternatives considered

- Branch-to-branch PRs: add divergence/merge commits and a second release
  identity; current publishing already uses tags. Use environments instead.
- A checked-off GitHub issue as the entire gate: mutable human text cannot
  establish artifact identity or automated success. Keep authenticated evidence.
- Turn every existing Storybook assertion into a blocker immediately: its
  inventory is larger than current verified coverage. Define a visible minimum
  release contract and show the wider gaps without claiming they passed.
- Screenshot snapshots as the sole visual authority: useful for review, but a
  new inconsistent design can become a new baseline. Review running flow states
  together against DESIGN.md; keep this distinct from functional testing.
- A new dashboard service first: unnecessary authentication/storage scope before
  the policy works. Actions summaries and an HTML artifact prove the model.

## References and verification of this plan

Repository files and live API reads support the current-state findings. No
release, remote setting, secret, source code, test configuration or workflow was
changed while preparing this plan. No product test suite was executed in this
design-only pass. Flow mappings above are source leads, not fresh test passes.

External platform behavior was checked against primary documentation:

- [GitHub token event behavior](https://docs.github.com/en/actions/concepts/security/github_token)
- [Environment deployment reviews](https://docs.github.com/en/actions/how-tos/deploy/configure-and-manage-deployments/review-deployments)
- [Tag and branch rules](https://docs.github.com/en/repositories/configuring-branches-and-merges-in-your-repository/managing-rulesets/available-rules-for-rulesets)
- [Ruleset bypass actors](https://docs.github.com/en/repositories/configuring-branches-and-merges-in-your-repository/managing-rulesets/creating-rulesets-for-a-repository)
- [Nextest JUnit output](https://nexte.st/docs/machine-readable/junit/)
- [Nextest retries and flaky results](https://nexte.st/docs/features/retries/)

Maintenance: changes to supported user flows, test IDs, version stamping,
artifact names, npm publishing identity, update-channel consumers or GitHub
permissions require revisiting this policy. Reviewers should scrutinize identity
binding and negative tests at least as closely as the successful promotion.
