# ChatGPT integration after core MCP

Base: merged core PR #1089, main at `e7388a98f7`.
Reference: tested combined branch `integration/shared-mcp-desktop` at `a85fbdbef7`.
Working branch: `feat/chatgpt-on-mcp`; original dirty worktrees remain untouched.

## Scope

Extract only the full-worker hosted ChatGPT integration. Keep OAuth, sessions,
checkpointing, tenant processes, UI and Cloudflare configuration under
`mcp/chatgpt/`; reuse the merged core unchanged. No legacy runtime/tools,
notebook-specific editing adapter, Desktop import or application Rust implementation changes.
The new PR supersedes the older combined #1075 as the second landing slice.

## Checkpoints

- [x] Extract active dependency graph, remove retired adapters and their tests.
- [x] Preserve OAuth/PKCE, rotation, isolation, persistence and worker-restart tests.
- [x] Add tool-surface/private-metadata and production-import packaging contracts.
- [x] Assemble stock frontend assets from the existing pinned public guest cache.
- [x] Verify final tests, formatting, Worker bundle and available native integration.
- [x] Commit, push and open the focused ChatGPT PR with explicit release gates.

## Boundaries

No deployment, public submission, remote account write or Desktop work is part of
this extraction. Live ChatGPT linking/rendering, two-account acceptance, container
replacement durability and capacity/retention remain explicit gates documented in
`mcp/chatgpt/README.md`. The guest cache is public runtime data; successful asset
assembly is not evidence of live rendering. No dependency versions are changed.

## Verification

- ChatGPT contracts: 39 passed, zero skips. Core contracts: 13 passed, zero skips.
- Generic MCP/native worker integration: one passed, zero skips. Reused the core
  worker binary after verifying the Rust/Cargo sources match the merged base.
- Generated guides, formatting, actionlint and whitespace checks pass.
- Guest assets assemble from the existing pinned public guest cache; the output
  is ignored rather than checked in. No live renderer acceptance claim.
- Cloudflare dry-run bundles the Worker/config/assets successfully without rollout.
- Docker daemon is unavailable at the configured socket, so this extraction's
  Linux image was not rebuilt. Prior combined-image evidence is not substituted.
- No Rust implementation, Desktop, dependency version or lockfile modifications.
  One shared guest-bootstrap JavaScript guard is retained from the source branch.

## Review correction

Independent review found an omitted guest-bootstrap guard: the iframe requests
`selectionReporting: false`, but main ignored it and transacted text selections.
A VM test of the full bootstrap reproduced the unwanted transaction. Restored
the small opt-out guard; the test checks both suppressed embed transactions and
unchanged ordinary-guest behavior. An initial default-case assertion used the
wrong envelope key and was corrected from `body` to the actual `request`.

Published PR: https://github.com/tonk-labs/tonk/pull/1091. The independent reviewer
confirmed the selection guard and its focused test. All 39 hosted tests pass;
remote CI remains pending. This replaces the older combined PR #1075.

## Deployment follow-up (authorized October 13)

The reviewer changed the PR gate to comprehensive automated MCP tests and a usable
PR preview; they will connect it in ChatGPT themselves. Deployment is now in scope.
Development follows main, staging prerelease tags, stable final tags. Desktop stays
out of scope. The earlier no-deployment extraction boundary is superseded.

- [x] Generate isolated issuer/client/checkpoint configurations per channel and PR.
- [x] Restrict provisioning and outbound requests to the matching Tonk provider.
- [x] Package guest assets from the same frontend build in the existing Deploy job.
- [x] Add a separate Docker runner for MCP, deployment smoke checks and connection settings.
- [x] Verify 13 core and 42 hosted tests locally; new deployment CLI audit is clean.
- [x] Verify CI image build and deployed PR preview, then publish its connection settings.

Wrangler is pinned as a deployment dev dependency. Preview R2 data is retained;
production durability and capacity policies remain separate readiness limitations.

## CI follow-up

Deployment run 38426137947 passed; preview is
`https://tonk-mcp-pr-1091.tonk.workers.dev/mcp`, public client
`tonk-mcp-pr-1091-chatgpt`. Live smoke independently verified merge image
`2b244a15691cd968e1d4a8b1617ca5837c0ef259`. Core MCP CI also passed.

Test run 38426137961: all 2,650 native debug tests passed (9 skipped), then
Cachix post-job upload failed with exit 3 after its push manager drain timeout.
Matrix fail-fast cancelled the native release build. Make only the matrix's
cache action best-effort and disable fail-fast; test commands remain required.
Workflow syntax validation is the focused check; the new CI run must establish
the previously cancelled native release result.
