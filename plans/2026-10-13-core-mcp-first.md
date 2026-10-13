# Core MCP first

Base: main at `6828aac5f4`. Source/reference: the tested combined branch
`integration/shared-mcp-desktop` at `a85fbdbef7`.

Land in order: core MCP, ChatGPT integration, Desktop. Desktop is on hold.
The combined branch remains a reference, not a merge candidate for the core PR.

## Scope

Keep the portable query/evaluate primitive, canonical guides, generic MCP HTTP
registration, optional account discovery/selection, and an isolated ordinary
worker host for native verification. Remove superseded tool implementations from
the landing surface entirely: no preview/apply/CLI translations, special schema,
rename or library-install tools, space-info wrapper, notebook editing adapter,
restricted Rust MCP runtime, compatibility UI or fixtures. Query/evaluate cover
these space operations; guides document rename and library commands.

Native proposal/inspection tools are host-specific rather than legacy, but also
do not belong in this PR. Their definitions can be considered with Desktop later.
No ChatGPT UI, OAuth host, Cloudflare deployment, Swift sources or generated
Desktop/browser application outputs are included. The portable browser module is
an ordinary consumer of the same core and needs no application integration.

## Checkpoints

- [x] Extract onto current main, preserving new invite/public-space work; prove
  generic HTTP/browser contracts and native worker authoring; commit separately.
- [x] Enforce response byte limits during streaming on success and error paths;
  prove bounded consumption/cancellation and no uncertain-write replay; commit.
- [x] Add independent core CI and usage/authority documentation. Native integration
  tests must fail, not skip, if their required worker binary is absent.
- [x] Final format, generated-guide, native and MCP checks; report scope and gaps.

## Extraction evidence

Ten core contracts passed with no skipped tests. A fresh `tonk-worker-host` build
on this branch passed, followed by the native authoring integration (one test,
zero skips). It covers schema/views, query immutability, rename and library
installation. Rust formatting and JavaScript formatting checks passed. The
separate Rust native storage/restart test is still compiling at this checkpoint.

The canonical agent context now lives in `rust/tonk-cli/src/guide-agent.md`,
alongside the other manuals. This avoids requiring `mcp/` in the Nix Rust source
filter while keeping MCP's generated instructions identical to CLI documentation.
No legacy tool definitions or implementations were copied into the extracted PR.

`npm audit` reports the existing esbuild 0.24.2 development-server advisory.
The dependency is used solely to bundle the primitive in memory in a portability
test; this package does not run an esbuild development server. No dependency or
lockfile version changes are included in this extraction.

## Bounded-response checkpoint

The new regressions first failed because oversized streams and unused bodies
were not cancelled. Response reading now enforces the byte limit before decoding
each chunk, handles split UTF-8 correctly, and cancels excess/unneeded bodies.
Success and analysis-error paths share the bound; dispatched writes still have
an uncertain outcome and are never replayed. All 13 core tests and the native
authoring integration pass after this change. Extraction commit: `9a07a87b4f`.

## Independent CI and final verification

Response-limit commit: `d6af21c95f`. The native integration now runs through an
ordinary MCP client and HTTP handler into the real worker, rather than only
calling the primitive directly. Running it without `TONK_WORKER_BINARY` was
verified to fail explicitly. `.github/workflows/mcp.yml` runs the standalone core
contracts, format/generated checks, worker library suite and native integration.
It does not depend on ChatGPT or Desktop. Actionlint passes; remote CI is tracked
in the PR checkpoint below.

The first broad worker run found one path regression and eight failures amid
file-descriptor exhaustion (the local soft limit was 256). The receipt-projection
fixture's absolute-path workaround reproduced in isolation; changing it to a
repository name under the explicit native root fixed it. This fixture tests
legacy receipt data, not the removed legacy MCP tools; that coverage is retained.

Final checks:

- Core JavaScript suite: 13 passed, zero skips.
- Generic MCP client / fresh native worker integration: one passed, zero skips.
- Worker library suite: 244 passed, zero failures, three existing ignored tests
  (two timing probes and the public Honky Tonks service test). Local command:
  `ulimit -n 4096`, then
  `nix develop --command cargo test --locked -p tonk-worker --lib -- --test-threads=2`.
  CI uses two test threads as well.
- Rust formatting, JavaScript formatting, generated-guide parity, actionlint
  and diff whitespace checks pass. Formatter registry access initially failed
  in the sandbox and passed on the scoped retry; actionlint passed using the
  repository's pinned nixpkgs after a global-registry resource error.
- The actual filtered `tonk-cli.src` contains the canonical agent guide without
  containing or depending on the `mcp/` directory.
- New main's invite/public-space changes are preserved. Cargo/flake locks are
  unchanged; the MCP lockfile is copied with its existing dependency versions.

The core branch is published for PR review and remote CI (see below), and has
not been merged. The prior combined branch is preserved only as a reference;
it must not be merged as the core PR or used to reintroduce compatibility tools.

## Deferred to the second PR

Extract the full-worker ChatGPT adapter on top of this core without reintroducing
the restricted runtime. Verify live account linking and rendering, two-account
isolation, expiry/revocation/reconnect, uncertain writes and container recovery.
Safe idle-process and disk retention require a durability policy; never discard
an unpublished replica simply because its connection expired. Public submission
is a separate launch gate. ChatGPT deployment and publication remain deferred.


## PR and independent review

Published core-only PR: https://github.com/tonk-labs/tonk/pull/1089. A reviewer
started with fresh context inspected the PR, reran all 13 MCP contracts and the
real-worker integration, and checked account discovery against a disposable
worker. It found no additional confirmed issues. The existing worker binary was
used for that independent pass; the fresh build and full worker suite are
recorded above.

The parent review reproduced a CLI regression: adding the agent manual before
the glossary broke the existing `tonk help all` contract. The glossary now stays
first, followed by the agent guide; MCP's generated manuals were refreshed.
The direct Rust assertion failed before the fix and passes afterward. MCP tests,
generated-guide parity and Rust/JavaScript formatting pass after the fix.
All five existing CLI guide tests pass, including executable notation examples.
Remote CI is running; local validation does not claim completed remote CI.
