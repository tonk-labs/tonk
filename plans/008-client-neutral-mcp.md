# Plan 008: Make hosted Tonk MCP usable across agent clients

Status: IMPLEMENTED LOCALLY; external acceptance and CIMD blocked/deferred. Priority: P1. Effort: L, split into reviewable PRs below.
Risk: high for authorization/state migration; medium for transport and packaging.
Planned at `4f1c6060ec`, 2026-10-14, in `feat/chatgpt-on-mcp`.
Depends on the core MCP and hosted ChatGPT integration landing first (#1089, #1091).
This is a separate follow-up plan, not an expansion of the ChatGPT landing gate.

## Outcome and boundaries

Users can connect Claude's hosted connector, Claude Code, and a standards-based
agent harness to the same environment-specific Tonk MCP URL, authenticate with
Tonk, discover authorized spaces, and query/evaluate them. ChatGPT keeps working.
UI is optional. One canonical tool implementation remains the narrow API over
the ordinary Tonk worker; client-specific tools or data models are not introduced.

First milestone: explicit registrations for ChatGPT, Claude and a native CLI
client, plus a generic SDK client proving headless use. Broad availability adds
self-service registration and verified installation instructions. Do not label a
client supported until its actual authentication and tool flow has been tested.

Out of scope: Tonk Desktop integration, public app-directory submissions, legacy
runtime/tools, arbitrary account/profile evaluation, new space-management APIs,
new worker semantics, and service-account/client-credentials grants. Unattended
harnesses can initially use an interactively authorized connection only within
its existing refresh lifetime; indefinite unattended access is a separate design.

## Baseline and drift check

Run before implementing:

```sh
git status --short
git diff --stat 4f1c6060ec..HEAD -- mcp .github/workflows/publish.yml .github/workflows/chatgpt.yml .github/workflows/mcp.yml
```

Reconcile changed files with the facts below. Start a fresh branch/worktree from
the landed integration; preserve unrelated work. Commit each tested checkpoint
separately. Do not implement this plan in the original dirty ChatGPT worktree.

Current source map:

- `mcp/primitive.mjs`, `tools.json`, `catalog.mjs`, `core.mjs`: canonical tools,
  descriptions, authority-independent dispatch. Preserve names and result envelopes.
- `mcp/account-backend.mjs`, `account-spaces.mjs`: account-scoped space selection.
- `mcp/http.mjs`: SDK `createMcpHandler`, bounded request bodies, optional UI hook.
- `mcp/chatgpt/authenticated-http.mjs`: authenticates each request; rejects any
  Origin header; only accepts POST; buffers finite SDK responses and closes the
  handler after each request. No GET subscription stream is promised.
- `mcp/chatgpt/oauth.mjs:90-139`: one `clientId`, HTTPS-only redirect validation,
  S256 public-client OAuth, resource metadata and snapshot client-ID matching.
  At line 172: `p.client_id !== clientId || !redirects.has(p.redirect_uri)`.
- `mcp/chatgpt/credentials.mjs`: opaque token hashes, rotation/replay detection,
  fixed maximum family lifetime, serialized principal payloads.
- `mcp/chatgpt/server.mjs`, `provisioning.mjs`, `worker-tenants.mjs`,
  `checkpoint.mjs`, `durable-state.mjs`: hosted tenant ownership, activation,
  seeds and credential persistence. These responsibilities are client-neutral.
- `mcp/chatgpt/cloudflare/deployment.mjs:58-59`: generates one `*-chatgpt` client
  and the ChatGPT callback. `worker.mjs` has an explicit route allowlist.
- `mcp/chatgpt/cloudflare/outbound.mjs:6-16`: only the configured Tonk provider
  and private checkpoint service are reachable. Arbitrary metadata fetching
  cannot be added without an explicit network design.
- `mcp/chatgpt/ui.mjs`: registers `tonk_open_space` and an MCP Apps resource;
  session credentials are in `_meta`, not text. `core.mjs:42` invokes the UI
  registration hook without a client-capability gate.
- `.github/workflows/mcp.yml`, `chatgpt.yml`, `publish.yml`: existing core tests,
  hosted contracts and environment deployment. Package scripts currently glob
  only root, integration, ChatGPT and Cloudflare modules; moving files requires
  updating test/format/package discovery, not just imports.

Use ESM modules, Node's test runner, existing dependency lockfiles and Prettier.
Model OAuth tests on `chatgpt/oauth.test.mjs`; isolation tests on
`chatgpt/authenticated-http.test.mjs` and `worker-tenants.test.mjs`.

## Scope

Allowed implementation paths: `mcp/**`, the three named workflows, this plan,
and its entry in `plans/README.md`. Proposed new directories are `mcp/hosted/`
(client-neutral hosting) and `mcp/apps/` (optional standard UI). Preserve public
URLs and storage identities when moving source. Do not touch Rust or Tonk Desktop
unless a separately reviewed prerequisite demonstrates that it is necessary.

## Checkpoint 1: Prove one non-ChatGPT login and tool call

Implement a bounded configured client registry in the existing host before
reorganizing files. Each registration has a client ID, display name, exact allowed
redirects, public/confidential client type and enabled status. Initially support
public clients with PKCE; never pretend a public client ID authenticates software.
Keep the current ChatGPT client and callback unchanged.

Verify Claude's actual connector registration and callback requirements using its
current documentation and an authorization attempt. Record them; do not guess
callback URLs. Add a CLI registration using an explicit loopback redirect policy.
HTTPS remains required for remote redirects. Permit HTTP loopback only for native
clients, validate literal loopback hosts, path and query, and apply the standard
port exception only where applicable. Never allow arbitrary localhost-like names,
private-network URLs, open redirect patterns or wildcard remote callbacks.

Bind pending requests, codes, token families and refresh grants to client ID,
issuer/resource, redirect and authorized account. Check those bindings during
code exchange and refresh before mutation. A wrong client's refresh attempt must
not rotate or revoke another client's family. Retain reuse detection for the
correct client. Existing per-call space membership checks remain mandatory.

Version persisted OAuth state. Add a fixture for the existing single-client
snapshot and deterministically attribute its grants to that configured client.
Do not clear credentials or tenant data to make migration pass. Disabled/removed
clients must not retain access via previously issued tokens. Define revocation
scope explicitly: one client grant versus all grants for an account.

Verification: existing hosted tests pass; add registry, loopback redirect,
cross-client code/refresh, client disablement, two-account isolation and snapshot
migration tests. Deploy a preview, complete one non-ChatGPT OAuth login, then call
`tonk_list_spaces` and `tonk_query`. This executable slice must work before moves.

## Checkpoint 2: Separate reusable hosting from presentation

Move OAuth, credential persistence, authenticated HTTP, provisioning, tenant
workers and server composition into `mcp/hosted/`. Move the standard MCP Apps UI
into `mcp/apps/`; isolate OpenAI-specific metadata in a small optional adapter.
Keep `mcp/core.mjs` free of hosting, login and provider-specific dependencies.
Compose the same hosted server for all supported clients, using the configured
registry rather than different data backends per client.

Update production imports, Docker COPY/entrypoint paths, build-assets relative
paths, Cloudflare bundling, test/format globs and package scripts. Add
`test:hosted` and `test:apps` scripts with explicit coverage; retain only intentional
compatibility entrypoints while migrating CI. Update the import-graph contract
so deployment tests exercise actual production modules.

Do not rename deployed workers, issuers, DO class/storage identities, container
instance names or R2 keys as part of a directory cleanup. Preserve restore of an
existing checkpoint. Verify unchanged ChatGPT tool schemas and private metadata.

Verification: all original contracts plus migrated tests pass, Docker image builds,
Worker dry-run passes, preview smoke passes, and both ChatGPT and the checkpoint-1
client can reconnect after deploying the reorganized image.

## Checkpoint 3: Establish HTTP and UI interoperability

Use real SDK clients over HTTP against the production server composition, with
disposable worker storage and a fixture checkpoint service. Exercise initialize,
initialized notification, tools/list, guide resources, query, one write and
readback, token expiry/refresh, shutdown and reconnect. Test both JSON and finite
SSE POST responses, protocol negotiation, unsupported methods and malformed input.
Record exact SDK/protocol versions. Use a second independent SDK implementation
(e.g. Python) so Node-client/Node-server agreement is not the only evidence.

POST-only stateless operation is not automatically a bug. Verify the selected
clients handle it and the documented 405 for unsupported GET streams. Do not add
legacy HTTP+SSE, WebSockets or stdio by default. A stdio-only harness is initially
unsupported; add a thin adapter only for a demonstrated requirement, reusing core
authorization and dispatch rather than recreating worker behavior.

Keep hostile Origin requests rejected. If a supported browser client requires an
Origin, introduce a narrow reviewed origin policy and tests, not wildcard CORS.
Exercise request/response limits, aborts and disconnects. A lost evaluate reply
must not trigger application-level replay; query to resolve uncertain outcomes.
Document any harness auto-retry behavior that Tonk cannot control.

Negotiate MCP Apps capability and MIME support before advertising UI resources
and `tonk_open_space`. Headless clients should get the four useful core/account
tools and no browser session issuance. Do not branch on untrusted client names.
If the request-scoped SDK loses initialize capabilities, first prove that behavior
and use its supported negotiation/session mechanism; isolate any resulting state
by authenticated grant and bound its lifetime. Never trust a supplied session ID
as authority. Do not introduce an unbounded global capability cache.

MCP Apps-capable hosts may receive the existing preview tool. Preserve credentials
in private metadata and add assertions that they never appear in model-visible
text, structured data, logs or public URLs. Verify ChatGPT rendering remains intact;
report Claude UI as unsupported/unverified until an actual render succeeds.

Verification: automated headless and UI-capable tool catalogs, concurrent accounts,
new-request capability behavior and transport cases pass. One actual Claude Code
session and one hosted Claude session complete read/write/readback on the preview.

## Checkpoint 4: Add self-service client registration

Choose and document the supported MCP authorization revision from the client
matrix. Support CIMD for clients using metadata documents and bounded DCR for
clients needing dynamic registration; ship them in separate increments. Explicit
registrations from checkpoint 1 remain supported. Do not advertise either discovery
capability until its complete implementation and negative tests are enabled.

CIMD requires a separate bounded metadata-fetch path. Do not enable unrestricted
container egress. Design the resolver at the edge or a dedicated restricted
service: HTTPS validation, public-address enforcement through DNS resolution,
redirect restrictions, response/time/size limits, schema validation, identity URL
matching, caching/expiry and fail-closed errors. Prevent rebinding/private-address
access at the actual fetch boundary, not only through a preliminary DNS check.
Keep fetched client metadata untrusted and never forward user tokens with it.
If the platform cannot enforce that policy, defer CIMD rather than weaken egress.

DCR must persist registrations in bounded durable storage, validate redirect/client
metadata, rate-limit enrollment, and implement expiry/removal. Registration must
never grant account access: normal user authorization still applies. Show the
requesting client's identity and requested authority during approval. Neither a
registration nor a fetched document proves that the caller is trusted software.

Verification: test concurrent registration, restart, quotas, metadata outages,
private-IP/redirect attacks, changed metadata, client deletion and cross-client
exchanges. Complete URL-only onboarding for each client that claims that support.
A static allowlist alone completes the first milestone, not broad self-service.

## Checkpoint 5: Publish a tested compatibility matrix and rollout

Keep the same deployment URLs for every client:

| Channel | MCP URL | Tonk provider |
| --- | --- | --- |
| Development | https://tonk-mcp-dev.tonk.workers.dev/mcp | https://tonk.foundation |
| Staging | https://tonk-mcp-staging.tonk.workers.dev/mcp | https://staging.tonk.xyz |
| Stable | https://tonk-mcp.tonk.workers.dev/mcp | https://tonk.network |
| PR N | https://tonk-mcp-pr-N.tonk.workers.dev/mcp | matching Tonk PR preview |

Preserve separate storage and credentials per environment. Record client/version,
transport, registration mechanism, callback policy, UI support, date, deployed
commit and results for ChatGPT, Claude hosted, Claude Code, Node SDK and Python SDK.
Include copyable setup commands/configurations validated with those exact versions,
expected first prompt, reconnect/revoke instructions and known unsupported modes.
Never put tokens, passkeys, authorization codes or private account data in fixtures
or public evidence. Keep OAuth client IDs distinct from secrets.

Roll out preview -> development -> staging -> stable. Require authenticated
list/query/evaluate/readback and two-account/two-client isolation in disposable
spaces, plus ChatGPT regression checks. Keep the existing unauthenticated deployment
smoke; it cannot replace these acceptance tests. Automate controlled fixture-account
flows where possible and record manual external-client evidence separately.

General compatibility is not unlimited public hosting readiness. Existing limits
include 16 workers, an eight-hour grant window and live replicas not checkpointed.
Before advertising unrestricted availability, separately prove recovery of
acknowledged effects across container replacement, safe capacity/retention and
revocation behavior. Never delete unpublished replicas to reclaim capacity.

## Verification commands

Existing commands (repository root; run focused tests while iterating):

```sh
npm --prefix mcp ci --ignore-scripts
npm --prefix mcp test
npm --prefix mcp run test:chatgpt
npm --prefix mcp run check:format
cargo build --locked -p tonk-worker --features native-host --bin tonk-worker-host
TONK_WORKER_BINARY="$PWD/target/debug/tonk-worker-host" npm --prefix mcp run test:integration
npm --prefix mcp/chatgpt/cloudflare ci
TONK_UI_DIST=/absolute/path/to/matching/tonk-ui npm --prefix mcp run build:chatgpt-assets
docker build --platform linux/amd64 -f mcp/chatgpt/cloudflare/Dockerfile.worker -t tonk-mcp:interop .
(cd mcp/chatgpt/cloudflare && npx --no-install wrangler deploy --dry-run --containers-rollout=none --config wrangler.toml)
git diff --check
```

Expected: exit 0, no skipped required integration cases. Use the actual Cargo target
path if CARGO_TARGET_DIR is configured. After checkpoint 2 add `npm --prefix mcp run
test:hosted` and `test:apps` to CI and update the paths above to their moved locations.
These new scripts do not exist at planning time. Preserve the repository's Rust/Nix
checks if implementation unexpectedly requires a separately approved Rust change.

## Completion and stop conditions

- [ ] Explicit-client non-ChatGPT login and read succeed before refactoring.
- [ ] Shared host extraction preserves existing deployments and persisted authority.
- [x] Two independent SDK clients pass authenticated HTTP read/write/readback (fixture account approval).
- [x] Headless clients neither need UI nor receive browser-session credentials.
- [ ] ChatGPT, hosted Claude and Claude Code have recorded live acceptance results.
- [ ] Self-service registration mechanisms pass negative tests and match discovery.
- [x] Documentation states exact supported modes and operational limits.
- [x] Update this plan and its README index entry with evidence; do not claim broad
      availability from discovery or mock tests alone.

Stop and report if migration requires resetting account storage, compatibility
requires removing authority checks, safe metadata fetching is unavailable, or
SDK lifecycle behavior cannot preserve isolated capability negotiation. Record a
focused prerequisite instead of inventing a bypass. Missing access to a target
client leaves that compatibility row unverified; it is not a reason to fabricate
success or silently substitute a different client.

## Sources and maintenance

Verified during planning; recheck versions and requirements during implementation:

- [Claude Code MCP setup and authentication](https://code.claude.com/docs/en/mcp)
- [MCP authorization, 2026-07-28](https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization)
- [MCP Streamable HTTP transport](https://modelcontextprotocol.io/specification/2025-11-25/basic/transports)
- [MCP Apps capability negotiation and progressive enhancement](https://apps.extensions.modelcontextprotocol.io/api/documents/overview.html)

Scope of planning review: MCP core/host/UI source, auth persistence, Cloudflare
configuration and CI scripts. No new client interoperability or deployment was
executed for this plan, and the wider worker/Desktop codebase was not audited.

## Implementation log (2026-10-14)

- Fresh implementation worktree `feat/client-neutral-mcp`, base `56cd67d214`.
  Initial status contained only this copied handoff. Drift from `4f1c6060ec`
  affects only `mcp/authoring-content.mjs` and the authoring tasks fixture;
  hosted contracts match the planned baseline. No other worktree was edited.
- First slice: bounded explicit public-client registry, exact HTTPS redirects,
  native literal IPv4/IPv6 loopback port exception, client/resource-bound code,
  refresh and replay checks, snapshot version 2 with legacy single-client migration.
  Wrong-client attempts cannot consume or revoke another client's grant.
  Registry removal/disablement is checked on access, refresh and browser sessions.
  Internal revocation distinguishes account-wide versus account/client grants.
  Existing environment variables remain the legacy migration identity; optional
  `TONK_MCP_CLIENTS` replaces the registry without renaming deployment/storage.
- Focused registry/OAuth tests: 20 passed before browser-session integration.
  Native build initially failed resolving github.com for locked dialog-db v0.3.0;
  retrying unchanged with network access. Docker daemon socket is absent.
- Claude Code installed version: 2.1.259. Current official setup docs describe
  `http://localhost:PORT/callback` (127.0.0.1 was a version-specific exception).
  Literal-loopback-only policy intentionally does not silently expand to DNS
  localhost. Claude Code compatibility therefore requires a verified client
  callback override or a separately reviewed exact-localhost policy.
  Source: https://code.claude.com/docs/en/mcp (checked 2026-10-14).
- Source moves remain behind checkpoint 1's explicit executable acceptance gate.
  Independent security, registration, transport and documentation work continues.
  No deployment or live external-client acceptance has been claimed.
- First increment verified: all 49 hosted tests pass (including browser-session
  parent revocation), Prettier 3.6.2 applied to changed modules, diff check clean.
  Dependencies installed with the existing lockfile. Native build proceeds after
  the network retry. Official Claude connector docs confirm the hosted callback
  https://claude.ai/api/mcp/auth_callback; explicit deployment configuration now
  includes that registration and a literal-loopback native SDK registration.
  https://claude.com/docs/connectors/building (checked 2026-10-14).
  This is configured support, not a live Claude authorization result.
- Checkpoint 4 independent increment: opt-in DCR (`TONK_MCP_DCR=true`), 32 durable
  registrations, 8 successful enrollments/minute across restarts, 24-hour expiry,
  8 KiB request limit, PKCE public clients only, hashed management tokens and
  authenticated deletion. Discovery and edge routes are enabled only with the
  complete implementation. Dynamic names are labeled unverified at approval.
  All 52 hosted tests pass, including quota/restart/deletion/negative metadata.
  DCR is not enabled in generated deployment configurations yet; URL-only live
  onboarding remains unverified. Existing explicit clients remain supported.
- CIMD deferred: no actual-fetch DNS/address pinning boundary exists in the
  current container/edge adapter. Container egress remains provider/checkpoint
  only. A restricted resolver enforcing address policy at connection time,
  response/timeout limits, no redirects, and metadata cache expiry is prerequisite.
  CIMD is neither fetched nor advertised. No fallback to unrestricted fetch.
- Checkpoint 3 capability slice: SDK 2.3.0 test reproduces lost initialization
  capabilities in the old request-scoped legacy factory. Apps clients now use
  supported SDK sessions, maximum 32 and ten-minute lifetime, bound to a unique
  authenticated OAuth grant on every request. Session IDs are never authority.
  Modern 2026-07-28 SDK requests negotiate capabilities in each request envelope;
  headless clients expose four tools and no UI/session issuance. Origin rejection
  and POST-only transport remain; request/response limits are 200 KB / 4 MiB.
  Oversized evaluate replies fail without replay. All 57 hosted tests pass.
  SDK tests use fixture authority/backends, not live client or native-worker proof.
  MCP Apps capability reference checked 2026-10-14:
  https://apps.extensions.modelcontextprotocol.io/api/interfaces/app.McpUiClientCapabilities.html
- Production composition is now exported as `createHostedService`; the executable
  uses that same function. Real loopback HTTP acceptance passed with Node SDK
  2.3.0 (2025-11-25 and 2026-07-28) and Python SDK 1.26.0 (2025-11-25): OAuth
  approval/code exchange, four-tool catalogs, native worker writes/readback,
  two accounts/clients, forbidden cross-space selection, guides, credential
  persistence/restart, refresh and reconnect. JSON and finite SSE responses observed.
  Native data and canonical dispatch are real; account approval/membership and
  checkpoint service are explicit fixtures. This is not passkey/provider or
  Claude/ChatGPT acceptance. CI installs the independent Python SDK.
- Native build passed (existing dead-code warnings). Both integration tests passed
  with `TONK_WORKER_BINARY=$PWD/target/debug/tonk-worker-host` and
  `TONK_PYTHON=/tmp/tonk-mcp-python/bin/python`. Initial sandbox runs failed at
  worker loopback bind (`PermissionDenied`, OS error 1); unchanged elevated run
  passed. No authority/network policy was weakened.
- Reconciled checkpoint ordering: local non-ChatGPT OAuth/tool vertical slice is
  now executable and proven. Per the implementation request, unavailable external
  client/deployment acceptance will remain unverified while independently safe
  source extraction proceeds. No claim that the original live preview gate passed.
- Checkpoint 2 source extraction: shared service, OAuth/state, worker tenancy,
  provisioning and Cloudflare packaging moved to `mcp/hosted/`; standard UI and
  assets moved to `mcp/apps/`; `mcp/chatgpt/metadata.mjs` is the optional OpenAI CSP
  adapter. Existing npm ChatGPT script names remain intentional aliases. CI now
  explicitly runs `test:hosted` and `test:apps`; publish paths point to the moved
  production modules. Core has no hosting or provider imports. Import-graph tests
  exercise the actual executable/service. Worker names, issuer URLs, DO class and
  migration tag, container instance name, R2 keys and legacy client IDs unchanged.
- Post-extraction checks: 13 core + 55 hosted + 3 Apps tests passed; both native
  integration tests passed again; formatting and diff checks passed. Restore is
  covered by the same versioned credential tests and actual HTTP reconnect test.
- Packaging limitations (not product failures): `build:hosted-assets` reports
  `Set TONK_UI_DIST or TONK_GUEST_PACK.`; Wrangler 4.150.0 dry-run reports missing
  `mcp/hosted/public`; Docker build cannot connect to the absent OrbStack socket.
  Wrangler's initial log-path permission error was avoided with a /tmp log path;
  missing assets remains the blocker. No substitute assets or deployment used.
- Final authority/lifecycle slice: RFC-style `/oauth/revoke` discovery and endpoint
  revoke only the supplied client-bound token family (access or refresh). Unknown
  or wrong-client tokens return the same success response without affecting the
  rightful client. Internal account-wide/client-wide APIs remain distinct. Added
  a checked-in legacy snapshot containing synthetic hashes only.
- Disconnect test exposed the old core reading `context.signal`; SDK 2.3.0 exposes
  `context.mcpReq.signal`. Corrected that field and combined it with the SDK's
  original HTTP request signal. Backend cancellation and exactly one dispatch
  now pass for an aborted write; oversized replies also never replay writes.
  An initial implementation using the old field caused the focused test to time
  out; inspecting the installed SDK types identified the corrected field.
- Latest checks: 58 hosted tests, 13 core tests, 3 Apps tests and both native
  integration tests passed. Real HTTP integration now explicitly advances the
  fixture clock past token expiry, observes 401, restores durable state, refreshes
  and reads back Python-authored native data. MCP-only wire responses include
  JSON and finite SSE (OAuth JSON is excluded from that assertion).


## Current completion and resume state

Local implementation is committed through `579bdff1c4`. The copied plan remains
in this worktree and its README entry was appended without replacing entries.
See [compatibility and exact resume steps](../mcp/hosted/COMPATIBILITY.md) for the
client matrix, current source paths, commands, authority boundaries and blockers.

- Explicit multi-client OAuth and SDK native read/write/readback: verified locally
  with fixture account approval; actual external login remains unverified.
- Shared hosting/Apps extraction: implemented and locally tested; packaged image,
  deployed checkpoint restore and reconnect remain unverified.
- Two independent SDK implementations and headless/UI capability isolation: pass.
- Actual ChatGPT, hosted Claude and Claude Code acceptance: unverified. Claude
  Code additionally needs resolution of its localhost-versus-literal callback.
- Bounded opt-in DCR: implemented and negative-tested; live URL-only onboarding
  unverified, default remains off. CIMD explicitly deferred, not advertised.
- Documentation/operational limits: recorded. No public availability claim.
- Docker/asset/preview checks: blocked by missing daemon and matching guest assets.
  No push, PR, development/staging/stable deployment or stored-data reset occurred.

Tested increments:

| Commit | Scope | Evidence |
| --- | --- | --- |
| `b5cfc1cc76` | Registry, migration, cross-client bindings | 49 hosted tests |
| `03d9372b5e` | Durable opt-in DCR | 52 hosted tests |
| `053c13faf0` | Capability sessions and transport limits | 57 hosted tests |
| `8d7d30b6bf` | Production composition and independent SDKs | 57 hosted + 2 native integration tests |
| `89c17c783a` | Hosted/Apps source extraction | 13 core + 55 hosted + 3 Apps + 2 native integration tests |
| `579bdff1c4` | Revocation, checked-in migration fixture, cancellation, expiry | 13 core + 58 hosted + 3 Apps + 2 native integration tests |

- Final redirect review tightened remote HTTPS callbacks to DNS hostnames,
  rejecting IP/private-name forms and empty fragments as well as nonempty ones.
  Native literal-loopback HTTP remains the sole port exception. All 26 focused
  registry/OAuth/DCR tests passed. This does not add callback DNS fetching.
- Packaging follow-through: OrbStack was installed and successfully started.
  Docker build and the repository `nix build .#tonk-ui-preview` are now running;
  earlier missing-daemon/assets observations above are superseded where these
  subsequent checks succeed. No deployment command without `--dry-run` was run.
- Final boundary checks: malformed JSON-RPC primitives/arrays are rejected before
  backend resolution; 60 hosted tests and both native SDK integration tests pass
  after these last source changes. Core (13) and Apps (3) remain passing from the
  preceding source checkpoint. Formatting and diff checks remain required below.
