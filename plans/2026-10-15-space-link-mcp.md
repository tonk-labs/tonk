# Per-call agent-link authority for MCP

Status: spike superseded by `2026-10-15-agent-link-production.md`; local worker, MCP contracts and local-service
remote integration passed. Inspected at `bf0ba6620c` on 2026-10-15.

Confirmed target: both hosted clients and local coding agents. Use one scoped
execution backend with hosted HTTP and local stdio transports.

## Finding

OAuth and account attachment can be replaced with explicit space authority on
each query/evaluate call. This does not eliminate the evaluator or sync runtime.
A local stdio MCP process avoids a hosted MCP service; remote MCP clients still
need a reachable execution endpoint. Client-specific acceptance remains untested.

Accept the complete link produced by Copy agent link, including the fragment,
rather than asking the model to extract a seed. The v2 fragment carries a 32-byte
Ed25519 seed; the query (possibly behind a shortcut) carries signed grants. The
seed proves possession of the recipient key, but does not alone establish scope,
expiry, route, or permission. Reuse Rust validation and trusted discovery.

## Current implementation evidence

- `mcp/primitive.mjs`: account-independent query/evaluate; a host supplies the
  authorized request function and selected subject. Preserve these semantics.
- `mcp/catalog.mjs`, `core.mjs`: schemas currently support pinned or account space
  selection, not a per-call link. Add a host authority mode without introducing
  different tool names or changing pinned Desktop/browser schemas.
- `mcp/hosted/authenticated-http.mjs`: OAuth is checked before MCP dispatch.
  A link-based endpoint must allow initialization/discovery without account login
  and authorize protected tool calls from their arguments instead.
- `mcp/hosted/worker-tenants.mjs`: current provisioning imports account-root
  authority and hydrates an account. Replace this path for the new host; do not
  simply remove the HTTP auth check from the existing account backend.
- `rust/tonk-invite/src/connection.rs`: AgentInvite validates seed/key agreement,
  signed chains, exact scopes, expiry and signed remote. Candidate scopes include
  main revision cell and archive get/put, excluding management-bearing meta.
  Current default grant duration is 90 days; effective validity depends on chains.
- `rust/tonk-cli/src/join.rs`: resolves shortcuts and rejects mixed carriers.
- `rust/tonk-cli/src/connections.rs`: validates and imports isolated credentials,
  mounts a main-only replica and binds subject, recipient and grant CIDs.
  Retained replicas deliberately allow offline work after expiry; that behavior
  must not accidentally authorize hosted MCP reads from a cached replica.
- `rust/tonk-worker/src/bin/hosted.rs`: private host opens a native worker with a
  local bearer token. It is not already an agent-link execution adapter.

## Proposed contract

```text
tonk_query({ agent_link: COMPLETE_LINK, document: NOTATION })
tonk_evaluate({ agent_link: COMPLETE_LINK, document: NOTATION })
```

Each call supplies authority. The validated grants determine space and remote;
there is no independent subject, branch, origin or local-path override. Public
guides need no link. Omit account-wide listing and account creation.

Resolve/validate link -> select isolated replica -> check current remote authority
and pull -> query/evaluate -> push writes -> return the canonical result envelope.
The exact fresh-authority gate needs an executable test: parser validation alone
does not check remote revocation. Fail closed if current authorization cannot be
established before serving cached content. Never replay an uncertain write.

Cache replicas by validated subject, recipient and grant identity, never just
space DID or MCP session. Serialize writes to the same replica. Revalidate each
call even on a cache hit. Retain unpublished effects for recovery; removing OAuth
does not make runtime state disposable. Cache limits and retention need a design.

The complete link is a bearer credential. Passing it as a tool argument exposes
it to the agent platform and potentially its transcript/traces. Tonk must redact
it from logs, errors and results. Resolve shortcuts without sending the fragment
to their web server; constrain outbound destinations and redirects. A seed-only
or opaque-handle design would require a separate grant lookup or credential store;
it is not inherently stateless or safer. Local hosts can later keep the credential
outside model arguments if the client supports that configuration.

## Smallest executable next step

1. Build a disposable space-scoped Rust execution adapter using existing invite
   validation/import and ordinary evaluation/sync. Prove query and one write plus
   independent remote readback with no account-root attachment. Prefer extracting
   reusable import code over duplicating cryptographic parsing in JavaScript.
2. Exercise two grants for the same space and a second space. Prove wrong seed,
   tampered grants, expiry, revocation and cache reuse cannot cross authority.
   Test unavailable revocation decisions and missing-link requests after valid
   ones. Verify failed push/lost response never triggers a write replay.
3. Add canonical schema mode and per-call backend. Prove missing credentials fail
   before runtime selection, credentials never echo, and existing pinned/account
   contracts still pass. Align link/input limits with HTTP body limits explicitly.
4. Package the same backend for the selected transport. Local stdio removes
   hosting; remote HTTP removes account OAuth but retains execution hosting.
   Prove actual target-client discovery and invocation before removing old OAuth
   deployments. Embedded frontend sessions require a separate scoped design.

## Verification

- After installing the locked MCP dependencies, `npm test --prefix mcp`: all
  18 tests passed, including the new schema/HTTP, subprocess and shortcut checks.
- `cargo test --offline --locked -p tonk-invite --test agent_connection`: all
  10 tests passed, covering key/scope/route mismatch, expiry, tampering, unsafe
  carriers, redaction and preservation of imported identity and grants.
- Native worker unit spike passed: real signed agent link -> isolated imported
  identity -> ordinary worker evaluation -> reopen -> query. Query does not
  commit, grants have separate caches, missing/wrong keys fail, and failed remote
  pull does not fall back to cached data. The local success path deliberately
  bypasses sync inside the private unit-test helper only.
- `cargo test --offline --locked -p tonk-worker --features native-host --test
  agent_link_spike -- --nocapture`: passed against the real local access-service
  and S3 fixture. A fresh replica read back the first write; an actual SDK stdio
  MCP client then wrote and queried through the subprocess adapter, and the
  independent replica confirmed that MCP write reached the remote. The first
  sandbox run failed to bind its listener; the unchanged test passed with local
  network permission (3.26 seconds of test execution, excluding compilation).
- Actual stdio discovery and invalid-link rejection also passed against the built
  native binary. No login or account attachment was involved.
- Runtime/usage details: `mcp/AGENT-LINKS.md`. This spike pins the trusted
  service in host configuration rather than adding automatic discovery.
- No live hosted-client compatibility or remote revocation proof yet.
