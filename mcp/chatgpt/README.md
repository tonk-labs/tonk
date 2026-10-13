# ChatGPT integration

This host composes the general [Tonk MCP core](../README.md) with OAuth and an
embedded Tonk frontend. It exposes the four core tools plus `tonk_open_space`.
Queries and evaluations use the ordinary native worker; there is no separate
language, restricted runtime, legacy tool adapter or Desktop dependency.

## Source map

| Module | Responsibility |
| --- | --- |
| `server.mjs` | HTTP startup, composition and shutdown |
| `oauth.mjs`, `authenticated-http.mjs` | PKCE account linking, discovery, per-request authentication |
| `credentials.mjs`, `durable-state.mjs` | Bounded credential families, rotation/replay checks, conditional persistence |
| `worker-tenants.mjs` | Account provisioning, per-call space selection, isolated worker processes |
| `checkpoint.mjs`, `checkpoint-limits.mjs`, `provisioning.mjs` | Closed authorization-seed recovery and serialized checkpointing |
| `space-session.mjs` | Space-scoped browser authority and direct worker HTTP/SSE |
| `ui.mjs`, `ui/` | ChatGPT resource registration, private session metadata, sandboxed stock frontend |
| `build-assets.mjs` | Public guest asset packaging; never includes account data |
| `cloudflare/` | Development host, outbound allowlist, private R2 proxy and container image |

Every tool request authenticates before resolving its account. The core account
adapter revalidates the selected space through this host. Browser sessions can
reach only their selected space/main branch; account and profile routes are not
exposed to the iframe. Tokens stay in private tool `_meta` and the outer bridge,
not model-visible results or guest-frame messages. Worker interruptions never
replay an evaluation. A failed remote push reports an uncertain local change.

## Local checks

From `mcp/`, run `npm ci --ignore-scripts`, `npm test`, and
`npm run test:chatgpt`. Run `npm run check:format` for source formatting. The core
native integration remains `TONK_WORKER_BINARY=/absolute/path/to/tonk-worker-host
npm run test:integration` (on one shell line).

`npm run build:chatgpt-assets` assembles public guest assets into ignored
`chatgpt/public/`. Its manifest pins the guest runtime; `TONK_GUEST_PACK` can name
a matching cached `space-runtime.json` to build without network downloads. Local
component sources and fonts come from this repository. Updating the guest pin
requires checking it with the current frontend/worker before deployment.

See [Cloudflare setup](cloudflare/README.md) for the explicit development
configuration. Tests and PR creation do not deploy or publish this host.

## Before landing as a production integration

This is a development integration for existing account spaces. Keep these gates
visible while reviewing it:

- Exercise actual ChatGPT linking, discovery, query/evaluate and embedded rendering,
  including two independent accounts, expiry, replay, revocation and reconnect.
- Verify the packaged image, outbound interception, R2 conditional writes and
  public assets together in the real host. Local mocks do not prove those paths.
- Establish recovery across container replacement for all acknowledged effects.
  Authorization seeds and credentials are checkpointed; live replicas are not.
  Successful content pushes alone do not prove every profile/branch effect is
  durable. Never discard unpublished replicas to reclaim capacity.
- Define idle process and disk retention. This development host admits at most
  16 workers and uses a fixed eight-hour connection window. It has no safe
  automatic capacity reclamation policy yet.
- Space creation is not exposed. Rename uses the general worker command; verify
  account-list readback in live acceptance before advertising it.

## Public distribution

Choose and verify a stable HTTPS issuer, exact registered client/callback,
privacy/support information and reviewer access before submitting. This PR does
not supply invented production values or a public distribution package.

Current official guidance describes [OAuth authentication](https://developers.openai.com/plugins/build/auth),
[embedded UI](https://developers.openai.com/plugins/build/chatgpt-ui), and
[plugin packaging](https://developers.openai.com/plugins/build/plugins). The host
uses predefined public-client PKCE configuration. Validate that configuration in
the intended ChatGPT connection and prepare the submission package separately.
Refreshing connection metadata and starting a new conversation is necessary when
checking changed tool discovery. HTTP discovery alone is not acceptance evidence.

Desktop remains a separate, later PR.
