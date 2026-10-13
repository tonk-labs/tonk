# Release gates execution, 2026-10-13

Scope decision: finish the observation/readiness phase, then plan protected
channel migration. User authorized a private synthetic repository and later
making it public. No production branch/tag/release/environment was changed.
No existing release App is available. Source changes remain uncommitted locally.

## Implemented

- Versioned browser/CLI flow policy; exact test and Storybook journey mappings.
- Immutable candidate context, artifact hashes, strict nextest inventory/JUnit
  collector, no-retry archived-test launcher, conservative evaluator.
- Candidate workflow reusing regression/MCP/CLI builds; candidate-specific web
  and signed CLI identities; E2E against selected artifact bytes.
- Actions summary and standalone HTML report; authenticated manual observations
  with candidate binding, freshness and latest-result replacement.
- Read-only promotion preflight and external policy audit. Publication remains
  disabled in code; stable preparation requires an unimplemented receipt collector.
- [Operator runbook](../docs/releases.md). Existing publishers remain unchanged.

## Fresh evidence

- 47 Python release tests pass (24 gate, 12 evidence, 11 policy).
- CLI tests: 11 run, seven passed and four Linux-only cases skipped on macOS.
- Actionlint 1.7.12 passes all workflows with ShellCheck disabled; ShellCheck was
  not run. Nixfmt 1.4.0 check and `git diff --check` pass.
- Real pinned nextest 0.9.140 ran the new Nix launcher against a tiny archived
  synthetic Rust fixture: one pass, one ignored test. Inventory/JUnit IDs matched.
  This proves launcher/parser integration, not Tonk product flows.
- Synthetic mixed-status report inspected in isolated headless Chrome, desktop
  and 390px mobile. Mobile document width equals viewport width; no console
  errors. Screenshots and temporary fixture output are local, not product proof.
- Reviewer role resolution handles GitHub's `role_name`; legacy `permission`
  maps maintain to write. See [GitHub collaborator API](https://docs.github.com/en/rest/collaborators/collaborators#get-repository-permissions-for-a-user).

## Hosted synthetic probes

Repository: https://github.com/tonk-labs/tonk-release-gates-test (public, synthetic
README/workflow/release only; no product assets or secrets).

Required reviewers returned HTTP 422 while private due to billing. Once public,
`release-staging` accepted required reviewer jackddouglas, main-only deployment
branch policy, and `can_admins_bypass: false`. Self-review is allowed in this
single-actor synthetic test; independent-review enforcement was not tested.

[Approval run 38427882323](https://github.com/tonk-labs/tonk-release-gates-test/actions/runs/38427882323)
was observed `waiting`, with no steps executed and a pending deployment. After
explicit synthetic approval it completed successfully. This proves environment
waiting/resume, not the complete production promotion or bypass matrix.

Tag ruleset 25012117 blocks creation/update/deletion of v* with no bypass actors.
A new tag creation was rejected (HTTP 422). Creating a GitHub Release on the
preexisting protected `v0.0.0-fixture` tag with `make_latest: true` succeeded:
[synthetic release](https://github.com/tonk-labs/tonk-release-gates-test/releases/tag/v0.0.0-fixture).
`releases/latest` resolved to release 411857501. Thus protected tags alone do not
protect the channel pointer followed by today's CLI/installers.

## Remaining boundaries

No hosted Tonk candidate run, signing/notarization run, full product E2E suite,
live manual product review or staging deployment has executed for this change.
Artifact download/digest and API collectors have local negative tests; their
end-to-end GitHub integration still needs a candidate run after merge. Current
web identity hashes the UI tree, not a complete publication manifest covering
all worker packages. This must expand before publishing is enabled.

Next phase must design a protected channel manifest and migrate `install.sh`,
CLI self-update, npm/channel metadata and existing installed clients. Define
compatibility explicitly: old clients still trusting GitHub latest cannot be
protected retroactively by changing only new clients. Then create/install a
release App, isolate credentials, implement staging receipts and stable
preparation, serialize resumable deliveries, close legacy publisher/cli-pin
bypasses, test the full permission matrix and perform an authorized staging
pilot. Do not enable enforcement by simply filling in policy.json or treating
this synthetic approval probe as cutover evidence.
