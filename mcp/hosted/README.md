> The deployed executable (`server.mjs`) serves two endpoints. `/mcp` is OAuth
> where a person sponsors an agent (`sponsor-service.mjs`, `sponsor-agents.mjs`):
> each connection is a hosted agent with its own key under `agents/`, approved on
> `/settings/agent`, which grants it `/provider/add` on the person's account.
> `tonk_create_space` makes a space the person owns (founder `/`, seed sealed to
> them) with content access for the agent, and answers a claim link that adds it
> to the person's account. `/mcp/link` takes [agent links](../AGENT-LINKS.md).
> The account-attachment modules below (`service.mjs`, `worker-tenants.mjs`) are
> not deployed: OAuth no longer attaches the human's own account.

# Hosted MCP service

This host composes the general [Tonk MCP core](../README.md) with OAuth and an
embedded Tonk frontend. It exposes four core/account tools and negotiates optional `tonk_open_space` for
MCP Apps-capable clients. See the [tested compatibility matrix](COMPATIBILITY.md).
Queries and evaluations use the ordinary native worker; there is no separate
language, restricted runtime, legacy tool adapter or Desktop dependency.

## Source map

| Module | Responsibility |
| --- | --- |
| `server.mjs`, `service.mjs` | HTTP startup, reusable composition and shutdown |
| `oauth.mjs`, `authenticated-http.mjs` | PKCE account linking, discovery, per-request authentication |
| `credentials.mjs`, `durable-state.mjs` | Bounded credential families, rotation/replay checks, conditional persistence |
| `worker-tenants.mjs` | Account provisioning, per-call space selection, isolated worker processes |
| `checkpoint.mjs`, `checkpoint-limits.mjs`, `provisioning.mjs` | Closed authorization-seed recovery and serialized checkpointing |
| `space-session.mjs` | Space-scoped browser authority and direct worker HTTP/SSE |
| `../apps/`, `../chatgpt/metadata.mjs` | Standard Apps presentation and optional OpenAI metadata |
| `build-assets.mjs` | Public guest asset packaging; never includes account data |
| `cloudflare/` | Development host, outbound allowlist, private R2 proxy and container image |

Every tool request authenticates before resolving its account. The core account
adapter revalidates the selected space through this host. Browser sessions can
reach only their selected space/main branch; account and profile routes are not
exposed to the iframe. Tokens stay in private tool `_meta` and the outer bridge,
not model-visible results or guest-frame messages. Worker interruptions never
replay an evaluation. A failed remote push reports an uncertain local change.

## Local checks

Run the exact Node/Python/native-worker commands in
[local evidence](COMPATIBILITY.md#reproduce-local-evidence). `test:hosted` and
`test:apps` have explicit coverage; `test:chatgpt` remains a compatibility alias.

`TONK_UI_DIST=/absolute/path/to/tonk-ui npm run build:hosted-assets` assembles
public guest assets from the matching Tonk build into ignored `hosted/public/`.
`TONK_GUEST_PACK` can instead name a cached `space-runtime.json` for local checks.
CI always uses the guest manifest from its own frontend build.

## PR acceptance and deployments

The PR gate is automated core/native and hosted MCP tests plus a deployed preview
whose discovery, authentication challenge and public assets pass smoke checks.
The reviewer connects that preview in ChatGPT to exercise linking and tools.
Live ChatGPT acceptance and public distribution are separate from this PR gate.

[Cloudflare deployment](cloudflare/README.md) follows the existing Tonk channels:
PR preview, main development, prerelease staging and final-tag stable. Each host
uses its matching Tonk provider and separate credentials and checkpoint bucket.
CI publishes the MCP URL and public OAuth client ID in its job summary and PR
preview comment. Leave the OAuth client secret empty.

## Before advertising production readiness

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
