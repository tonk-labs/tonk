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
- [ ] Commit, push and open the focused ChatGPT PR with explicit release gates.

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
