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
- [ ] Add independent core CI and usage/authority documentation. Native integration
  tests must fail, not skip, if their required worker binary is absent.
- [ ] Final format, generated-guide, native and MCP checks; report scope and gaps.

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

## Deferred to the second PR

Extract the full-worker ChatGPT adapter on top of this core without reintroducing
the restricted runtime. Verify live account linking and rendering, two-account
isolation, expiry/revocation/reconnect, uncertain writes and container recovery.
Safe idle-process and disk retention require a durability policy; never discard
an unpublished replica simply because its connection expired. Public submission
is a separate launch gate. No branch push, deployment or publication is part of
this local extraction.
