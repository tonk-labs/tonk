# Hosted agent peer (tonk.host)

Status: design, agreed 2026-10-28. Replaces the per-call agent-link MCP (#1105).

An agent gets its own space, provisioned by the user's account during an OAuth
sign-up. The OAuth token is a session credential for that space. Every MCP call
goes to `agent.tonk.host` carrying the token; the handler builds a dialog peer
for the agent and acts with full fidelity: list its spaces, see contacts (the
account it works for among them), query, evaluate, create spaces when allowed.
`tonk.host` is a multi-tenant hosted peer. The MCP server is a thin wrapper.

## Model

```
account ──owns, key sealed under account──▶ agent space     (marked as agent)
account ──create-space delegation (opt)───▶ agent space
other spaces ──invite, expires, renewable─▶ agent space
agent space ──powerline, no expiry────────▶ session key     (one per OAuth grant)
```

- **Agent space.** An ordinary space, provisioned through the user's account and
  sealed under it like any other (dialog-credentials). Accounts are spaces too,
  so an agent space DID is a valid member, audience and contact anywhere an
  account DID is. A space entry marks it as an agent space.
- **Session.** A fresh keypair per OAuth authorization. The agent space delegates
  to it with a powerline (`sub: Any`, command `/`), **no expiry**, the same shape
  as `mint_addressed_device_delegation`. Valid until revoked.
- **Access to other spaces** is by invite to the agent space DID. Invites expire
  and are renewed like any other invite. This is where expiry lives.
- **Space creation** (opt-in at authorization): a delegation from the account
  letting the agent provision spaces on the user's behalf. Created spaces are
  owned by the user and sealed under the account.
- **Attribution.** Authority is the agent space DID, operator is the session DID.

A leaked token exposes the agent's own space plus whatever invites are live at
the time. Revocation is the only bound on the session itself.

## Token

```
token = base64url(dag-cbor { v: 1, seed: <32 bytes>, proof: <powerline> })
```

The seed travels **beside** the powerline, not in its `meta`. `meta` is signed
and rides inside the envelope: dialog retains delegation chains (see
`account/device-link`, keyed by the retained blob), and proofs go to every
verifier on invocation. A seed in `meta` would be persisted in the agent space
and disclosed to the access service on every call.

The token response has no `expires_in` and no `refresh_token`. This departs
from the spec's SHOULD on short-lived tokens; expiry is on invites, revocation
on the session.

## Authorization

Resource server and authorization server are separate origins, as the MCP
authorization spec allows. The authorize page must be on the provider origin
because that is where the user's passkey works.

```
LLM host            agent.tonk.host            provider (AS + app)            browser
   │── /mcp ──────────▶│                              │                          │
   │◀─ 401, resource_metadata                         │                          │
   │── GET /.well-known/oauth-protected-resource ─▶│  │                          │
   │◀─ { authorization_servers: [provider] }          │                          │
   │── AS metadata, CIMD or DCR ──────────────────────▶│                          │
   │── open /authorize?resource&code_challenge ──────────────────────────────────▶│
   │                                                  │◀─ sign in (passkey)       │
   │                                                  │◀─ new or existing agent,  │
   │                                                  │   name, spaces, create?   │
   │                                                  │◀─ token bound to code     │
   │◀──────────────── redirect callback?code ─────────────────────────────────────│
   │── POST /token (code, verifier, resource) ────────▶│                          │
   │◀─ { access_token: token, token_type: Bearer } ───│                          │
   │── /mcp + Bearer ──▶│                              │                          │
```

In the browser, on approve:

1. New agent: provision the agent space under the account, seal its key, write
   the agent marker. Existing agent (reauthorizing from another host or device):
   unseal it.
2. Mint a session keypair and the powerline from the agent space to it.
3. Write the session record (below).
4. Mint invites from each chosen space to the agent space DID.
5. If allowed, mint the create-space delegation.
6. Hand the token to the AS, bound to the authorization code.

AS endpoints live on the access service. The code is single-use and short-lived;
`/token` checks PKCE (S256), `redirect_uri` and that `resource` is this channel's
`agent.tonk.host/mcp`, then returns the token once. Support Client ID Metadata
Documents and dynamic registration; accept `http://localhost:<port>` callbacks.

## Request handling

```
Bearer ─▶ decode ─▶ verify powerline: signature, aud == DID(seed), iss is agent space
       ─▶ revocation check (every call)
       ─▶ peer { authority: agent space, operator: session }
       ─▶ reads and writes through the access service
```

- Stateless Worker. No container, no replicas, no checkpoints. Any number of
  instances; concurrent writes resolve at the revision cell.
- A verified powerline can be cached by token hash indefinitely (it cannot
  expire). Revocation is never cached.
- Missing, malformed or revoked token: `401` with `WWW-Authenticate: Bearer
  resource_metadata=...`. A lapsed invite is a **tool error** naming the space
  and how to renew, not a 401; a 401 makes the host mint a new session.
- Reads go through the access service. Revisit direct R2 only if it measures slow.

## Records

In the agent space, mirroring how the account space records devices:

- **Agent marker.** A space entry marking it as an agent space.
- **Session rows.** Same shape as `account/device-link`: keyed by the powerline's
  blob hash, `created-at`, `title` (OAuth `client_name`, e.g. "Claude"),
  `reason: case:agent-session`. Revoking the powerline removes the row.

New concepts go in the YAML library first (`schema-first`).

## Channels

| Channel | MCP (resource server) | Authorization server |
| --- | --- | --- |
| main | `https://agent-dev.tonk.host/mcp` | `https://tonk.foundation` |
| staging | `https://agent-staging.tonk.host/mcp` | `https://staging.tonk.xyz` |
| stable | `https://agent.tonk.host/mcp` | `https://tonk.network` |
| PR N | `tonk-mcp-pr-N.<subdomain>.workers.dev/mcp` | matching preview |

## Tools

Keep `tonk_query` / `tonk_evaluate` notation semantics, addressed by `space`
instead of `agent_link`. Add listing spaces and contacts, joining from a pasted
ticket (redeemed into the agent space), and creating a space when delegated.

## Replaces

- `mcp/agent-link-backend.mjs`, `mcp/hosted/agent-service.mjs`, the container
  in `mcp/hosted/cloudflare/` and its replica/checkpoint machinery.
- Credentials leave tool arguments. Agent links travelled in the transcript;
  OAuth tokens are stored as host secrets.
- Reference for the AS side: the pre-#1105 `mcp/hosted/oauth.mjs`,
  `registration.mjs`, `cloudflare/discovery.mjs`.

## Local development

| Piece | How |
| --- | --- |
| AS + app | access service under `wrangler dev`; see the `run-app` skill |
| tonk.host | `wrangler dev` on another port; `providerOrigin()` needs a localhost exception |
| Real client | `claude mcp add --transport http tonk-local http://localhost:<port>/mcp` |
| Step-through | `npx @modelcontextprotocol/inspector` |
| Automated | Playwright + Chrome virtual authenticator (`hasPrf: true`), Node MCP SDK client |
| claude.ai | `cloudflared tunnel --url` per origin; passkeys are per tunnel hostname |

## Open

1. **Create-space delegation.** Narrow command or powerline from the account?
   Quota? Permanent and unbounded means a leaked token can create spaces the
   user pays for.
2. **Finding agents.** Does the account page list agent spaces from the account's
   space list filtered by the marker, or keep a pointer in the account space?
   Decides whether agents show up among ordinary spaces.
3. **Workers fit.** `tonk-worker` builds for `wasm32` (browser). Untested in a
   Workers isolate: bundle size limit, 128 MB memory, CPU time on large trees.
4. **Last used.** Worth showing per session so stale or unknown sessions are
   noticed, but a write per call is too chatty. Throttle, e.g. once a day.
5. **Invite renewal.** Who renews an agent's lapsed invite, and how the agent
   learns which one lapsed.
6. **Migration.** Existing agent-link clients on `agent*.tonk.host/mcp`.
