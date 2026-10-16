> The deployed endpoint now uses [per-call agent links](../AGENT-LINKS.md).
> The OAuth compatibility information below applies only to the retained optional
> account adapter. Hosted-client acceptance of the agent-link endpoint is a
> separate rollout check.

# Hosted MCP compatibility and acceptance

Evidence date: 2026-10-14. Implementation through `49cde56d50`; no commit from
this branch has been deployed. These are local results, not client certification.

| Client/version | Transport/protocol | Registration/callback | UI | Result |
| --- | --- | --- | --- | --- |
| Node SDK 2.3.0, Node 24.18.0 | Streamable HTTP; 2025-11-25 and 2026-07-28 | Explicit public client; literal loopback with varying port | Headless and Apps capability tests pass | Actual local HTTP and native data read/write/readback pass; fixture account approval/membership |
| Python SDK 1.26.0, Python 3.13.5 | Streamable HTTP; 2025-11-25 | Explicit public client; literal loopback | Headless tested | Actual local HTTP list, guide, write/readback pass; bearer token supplied by fixture OAuth flow |
| ChatGPT, version unavailable | Expected Streamable HTTP | Existing `${worker}-chatgpt`; exact existing callback | Local schema, private metadata and Apps-session tests pass | Live login/tools/rendering regression unverified |
| Claude hosted, version unavailable | Expected Streamable HTTP | Configured `${worker}-claude`; documented `https://claude.ai/api/mcp/auth_callback` | Unverified | No live authorization attempt or tool call; not claimed supported |
| Claude Code 2.1.259 installed | HTTP supported by client docs | Its documented `http://localhost:PORT/callback` is rejected by our literal-address-only policy | Unverified | Blocked pending verified literal-address override or separately reviewed exact-localhost policy; no live acceptance |
| stdio-only harness / Tonk Desktop | Not implemented here | None | None | Deferred |

The local SDK test drives the OAuth browser ceremony with a fixture grant. It
then supplies the issued bearer token to each SDK; it does **not** prove those
SDKs' automatic OAuth discovery/browser prompting. Native tool semantics and
storage are real. Account approval, account membership, and checkpoint service
are fixtures. Two worker stores enforce isolation in that test; independent
hosted tenant tests exercise production membership and account binding.

Official client requirements checked during implementation:
[Claude connector building](https://claude.com/docs/connectors/building),
[Claude Code MCP setup](https://code.claude.com/docs/en/mcp), and
[MCP Apps capabilities](https://apps.extensions.modelcontextprotocol.io/api/interfaces/app.McpUiClientCapabilities.html).
A documented callback is not an observed authorization attempt.

## Environment identities

| Channel | MCP URL | Tonk provider |
| --- | --- | --- |
| Development | https://agent-dev.tonk.host/mcp | https://tonk.foundation |
| Staging | https://agent-staging.tonk.host/mcp | https://staging.tonk.xyz |
| Stable | https://agent.tonk.host/mcp | https://tonk.network |
| PR N | https://tonk-mcp-pr-N.tonk.workers.dev/mcp | Matching Tonk PR preview |

These are configured destinations, not evidence of this branch being live there.
Source extraction did not rename issuers, workers, `TonkRuntime`, migration `v1`,
`chatgpt-single-space-v1`, checkpoint keys, buckets or legacy ChatGPT IDs.
Credentials and native data remain separate per environment.

## Explicit configuration and onboarding

`TONK_MCP_CLIENT_ID` and `TONK_MCP_REDIRECT_URI` retain the original single-client
configuration and legacy migration identity. Optional `TONK_MCP_CLIENTS` is a JSON
array replacing the enabled registry. Include the original ChatGPT registration
when adding clients. Each record has `clientId`, `displayName`, `type: "public"`,
`applicationType: "web" | "native"`, `enabled`, and `redirectUris`.

Generated deployments configure ChatGPT, Claude hosted and a native SDK client.
The native client ID is `${worker}-native`; redirects are `http://127.0.0.1/callback`
and `http://[::1]/callback`. Only native HTTP literal loopbacks get the port
exception; host, path and query must otherwise match. Remote HTTPS redirects
use DNS hostnames and match exactly; IP and private-name forms are rejected. `localhost`, aliases such as `127.1`, fragments, credentials in
URLs and arbitrary private-network HTTP addresses are rejected.

For a future live Claude hosted attempt, add the environment MCP URL as a custom
connector and supply `${worker}-claude` as its preconfigured public client ID,
leaving the secret empty. For ChatGPT, retain `${worker}-chatgpt` and its existing
`https://chatgpt.com/connector_platform_oauth_redirect` callback. These onboarding
steps are **not live-validated**. Do not advertise Claude Code installation until
its callback-policy gap is resolved; blindly using `--callback-port` does not fix
its hostname mismatch.

Expected first prompt for acceptance: “List my Tonk spaces, then query the named
disposable test space.” Follow with one unique write and query readback. Repeat
with two accounts and two clients, including forbidden cross-space requests.
Refresh connector metadata and start a new conversation when testing catalogs.
Record client version, deployed SHA, date, exact callback and results without
retaining credentials, authorization codes, passkeys or private account data.

## Registration, refresh and revocation

Explicit registrations are always available. Opt-in `TONK_MCP_DCR=true` enables
`POST /oauth/register` and advertises it in OAuth discovery. Public PKCE clients
only: metadata includes `client_name`, `redirect_uris`, `application_type` and
`token_endpoint_auth_method: "none"`; optional grant/response types must match
`["authorization_code", "refresh_token"]` and `["code"]`. Only scope `tonk` is
accepted. Registration does not authorize an account. Approval labels dynamic
client names as unverified; possession of a public client ID proves no identity.

DCR stores at most 32 registrations for 24 hours; enrollment is limited globally
to eight successes per minute, persisted across restart. It returns a private
management token and `registration_client_uri`. Authenticated DELETE at that URI
removes the registration and its grants. Updates are deliberately unsupported:
register a new client and delete the old one. DCR is off in generated deployment
configs until real onboarding is accepted. CIMD is neither implemented nor
advertised; a fetch-boundary DNS/address enforcement service is prerequisite.
Container egress remains limited to the provider and checkpoint service.

Access tokens expire after ten minutes. Refresh rotates tokens without extending
the eight-hour maximum family lifetime. Wrong-client refresh/code reuse cannot
rotate or revoke another client's grant. Correct-client replay revokes the family.
After the family expires, repeat interactive account authorization.

`POST /oauth/revoke` accepts form fields `client_id` and `token` (an access or
refresh token); optional `resource` must be the original MCP URL. It revokes only
that family. Unknown/wrong-client tokens return the same 200 response without
altering another grant. Removing/disabling an explicit registration blocks its
access, refresh and browser sessions on every request; preserve stored data.
Internal `revokeClientGrant(rootDid, clientId)` and `revokeAccount(rootDid)` have
account/client-wide and account-wide scope respectively; they are not unauthenticated
public administration endpoints. Local client logout alone is not server revocation.

Persisted OAuth snapshots migrate forward from the original single-client format
into version 2 without changing token hashes or tenant data. The configured
legacy client ID must match the snapshot. Do not roll back to a runtime that only
reads the old snapshot format after migration, and never downgrade/reset stored
authority to make a rollback start. Keep the durable snapshot and use a compatible
reader. Legacy snapshots did not record each family's redirect; new grants do.

## HTTP and presentation contract

Origin-bearing MCP requests remain rejected. POST is supported; GET subscription
streams and DELETE sessions return 405. No legacy HTTP+SSE or WebSocket endpoint
was added. Finite SSE and JSON MCP responses are exercised locally. Bodies are
limited to 200 KB, buffered MCP replies to 4 MiB, notation to 32 KB. Aborted HTTP
calls reach the worker's cancellation signal. A lost or oversized evaluate reply
is uncertain: query before deciding whether to write again.

There is no application-level write retry. SDK transports can reconnect or
reauthenticate; clients outside the tested versions may have different retry
policies. Never treat a client retry policy as exactly-once delivery.

Headless clients get four tools and guide resources, without browser credentials.
MCP Apps requires `extensions["io.modelcontextprotocol/ui"].mimeTypes` to include
`text/html;profile=mcp-app`. Legacy clients get SDK sessions capped at 32 and ten
minutes, bound to the authenticated grant; reconnect/initialize after session
expiry or server restart. Modern 2026-07-28 clients carry capabilities per request.
A session ID cannot substitute for a bearer token or select a different grant.
UI credentials stay in private `_meta`, absent from text/structured data and
public URLs. The optional OpenAI resource adapter preserves existing CSP metadata;
`createHostedService({ resourceMetadata: {} })` composes standard Apps without it.
Actual ChatGPT/Claude rendering is still unverified.

## Reproduce local evidence

From the repository root (requires loopback socket access):

```sh
npm --prefix mcp ci --ignore-scripts
npm --prefix mcp test
npm --prefix mcp run test:hosted
npm --prefix mcp run test:apps
npm --prefix mcp run check:format
cargo build --locked -p tonk-worker --features native-host --bin tonk-worker-host
python3 -m venv /tmp/tonk-mcp-python
/tmp/tonk-mcp-python/bin/pip install -r mcp/integration/requirements.txt
TONK_WORKER_BINARY="$PWD/target/debug/tonk-worker-host" TONK_PYTHON=/tmp/tonk-mcp-python/bin/python npm --prefix mcp run test:integration
```

Use the actual target directory if `CARGO_TARGET_DIR` is configured. Python SDK
1.26.0 is required, not skipped when unavailable. The HTTP test runs Node and
Python against disposable native storage, exercises expiry/restore/refresh, and
reads Python-authored data back after service reconstruction. It never reuses or
clears user credentials. CI installs the same pinned Python SDK.

## Local packaging evidence

The matching Nix frontend output is
`/nix/store/lfhi4b6bgcqcaym1zahlb05s3j1f04f6-tonk-ui-preview-0.6.16-rc.1`.
Generated `hosted/public/space-runtime.json` SHA-256:
`17aee7233117dba79581356c8523b447bfeae037b7a81b1eda7c2584da8b5b9f`.
Wrangler 4.150.0 dry-run passed with the real assets and a 62.08 KiB Worker bundle.
The final local amd64 image `tonk-mcp:interop` is
`sha256:b85c2b789b8cfc242287a8ba52d943522994033166e90c5b717352e358e0517f`.
It successfully imports the production composition with networking disabled.
This is local packaging evidence from the worktree, not a deployed release image
or proof of Cloudflare interception, passkeys, remote sync or container recovery.
Initial missing-daemon/assets failures were resolved by starting OrbStack and
building the matching frontend. The only final Rust warnings were existing
unused-function warnings. All 78 tests passed without skips: 13 core, 60 hosted,
3 Apps, and 2 native integration tests. Repository formatting and diff checks pass.

## Remaining release gates and exact resume sequence

1. Supply the matching Tonk frontend distribution as `TONK_UI_DIST`; run
   `npm --prefix mcp run build:hosted-assets`. This run built the matching artifact
   with `nix build --accept-flake-config .#tonk-ui-preview --out-link /tmp/tonk-mcp-ui-preview`
   and successfully assembled the real assets. Rebuild for a different candidate.
2. Start Docker, then run the image build and Wrangler dry-run documented in
   [Cloudflare packaging](cloudflare/README.md). OrbStack was started and the
   Wrangler 4.150.0 dry-run passed with real assets. The linux/amd64 Docker
   build and a network-disabled import of `hosted/service.mjs` also passed.
3. In a separately authorized preview deployment, record the candidate SHA and
   image identity, then run existing unauthenticated smoke and the authenticated
   two-account/two-client list/query/write/readback journeys. This task did not
   push, open a PR, or deploy. No staging/stable deployment is authorized here.
4. Complete real ChatGPT and hosted Claude OAuth/tool/rendering attempts. Resolve
   the Claude Code callback policy before claiming its login works. Test DCR
   URL-only onboarding before enabling it for a channel.
5. Design and prove a bounded CIMD resolver at the actual connection boundary.
   Do not enable unrestricted container or edge metadata fetching.
6. Separately prove recovery of acknowledged effects across container replacement,
   safe retention/capacity and revocation. The host still caps native workers at
   16, has an eight-hour grant window, and does not checkpoint live replicas.
   Never delete unpublished replicas to free capacity. Desktop remains deferred.
