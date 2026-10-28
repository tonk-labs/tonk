# Hosted agent accounts over OAuth

Status: draft for review, 2026-10-28. Nothing implemented.

## User story

1. I talk with ChatGPT (or Claude on claude.ai). I say "save this into a Tonk space".
2. ChatGPT calls the Tonk connector. The first time, it opens the OAuth flow.
3. The flow sends me to the Tonk app. I sign in with my passkey and approve
   "ChatGPT, an agent sponsored by you". The server mints an account for the agent.
4. ChatGPT creates a space. I am the owner and admin. The agent has access that
   I can revoke.
5. ChatGPT writes the data and gives me a link. I open it and the space is in my
   space list. I see the data, and the roster shows the agent as an agent that I invited.
6. Later, I revoke the agent. It can no longer read or write the space. I keep the space.

Acceptance: steps 1 to 6 work live in ChatGPT and in claude.ai on the stable or
staging channel.

## What exists (main at f694dde904)

- Hosted MCP: Cloudflare Worker, one container, Node MCP server, one Rust
  `tonk-agent-worker` process for each call. Tools take `{agent_link, document}`. No OAuth.
- The OAuth code from before #1105 is still in the tree and unused:
  `mcp/hosted/oauth.mjs`, `credentials.mjs`, `durable-state.mjs`, `checkpoint.mjs`,
  `provisioning.mjs`, `worker-tenants.mjs`, `clients.mjs`, `registration.mjs`,
  `authenticated-http.mjs`, `mcp/account-backend.mjs`. It linked the human's own
  account: the consent page `/settings/link` minted a full account powerline
  (`subject: Any`, no expiry) to a device key on the server. #1105 removed it for
  that reason. It was never tested live with ChatGPT.
- Space creation (`rust/tonk-worker/src/router/repository.rs`, `create_repository`):
  - One owner, the creator's account. There is no field for a second admin.
  - One delegation, `space -> owner` with command `/`.
  - The space seed is not discarded. It is sealed in custody for the owner account
    (`custody_seed`), and rotation signs with it again.
  - Sync needs the access service: `/provider/add`, invoked by an active customer
    with a `space -> customer` consent.
- Sealing a seed to an account needs only that account's public X25519 key
  (`rust/tonk-identity/src/sealed.rs`). The owner does not need to be online.
- Revocation: any holder of `/` on a space can revoke any hop in that space through
  `/ucan/revoke`. The service records it only if the space is provisioned.
- Accounts without a passkey already exist (onboarding accounts, custody by
  `KekMethod::Custodian`). A server can mint an account root.
- Open stack #1137 to #1150: roster `kind = agent`, `InvitedBy`, display in the FAB
  and Desktop, agent-keyed grants, access requests. Not merged, about 27 commits behind.

## Design

### Identity

- One agent account for each (sponsor account, OAuth client). For example
  "ChatGPT for Jack". When the same client authorizes again, it gets the same agent account.
- The server generates the agent `AccountSecret`. Custody uses `KekMethod::Custodian`,
  with a service key that lives only in a Cloudflare secret. The sealed envelope is
  stored in R2. It is never stored in plain text, which is different from the old tenant checkpoints.
- The agent account gets no authority from the sponsor's account (no powerline).
  It has authority only in spaces where it holds a grant.

### Consent (OAuth authorize step)

- Re-use `oauth.mjs`. Change the provisioner: mint the agent account (or find the
  existing one), then send the human to a new page `/settings/agent`.
- The page dispatches a new command, `tonk:sponsor-agent { agent, client }`, on the
  human's device. It:
  1. Writes an agent row on the human's account branch. This is the account-level
     agent registry, slice 3 of the agents plan.
  2. Returns, in the URL fragment: the sponsor account DID, the sponsor's X25519
     recipient key, and (if spike S0 passes) a provisioning delegation.
- The callback stores the principal `{agentAccount, sponsor, sponsorRecipient, clientId}`
  and issues the tokens.
- Clients:
  - Add CIMD, which ChatGPT prefers. Claude uses CIMD only if the server advertises
    `client_id_metadata_document_supported` and `none`.
  - Keep DCR as a fallback.
  - Accept loopback callbacks on any port, for Claude Code.

### Space creation (`tonk_create_space { name }`)

Runs in the agent's replica on the server:

1. Generate the space seed.
2. Sign `space -> sponsor` with `/`. The sponsor is the founder.
3. Sign `space -> agent` with content scopes: main branch read and write, and archive.
   This is not `/`. A `/` holder can revoke any hop, so `/` would let the agent revoke the sponsor.
4. Seal the seed to the sponsor's recipient key, then drop it from the agent's
   custody. The agent cannot mint itself new grants later.
5. Write the roster rows:
   - The sponsor: founder and admin.
   - The agent: `kind = agent`, `InvitedBy` set to the sponsor. This needs #1137 and #1138.
6. Provision the space with the access service so it syncs (see S0).
7. Return a claim link to the model, so it can give it to the human.

### Claim (the human opens the link)

- The link carries `space -> sponsor` and the sealed seed (or a pointer to it).
- The human's device:
  - Writes the directory row and the mount.
  - Pulls the space.
  - Opens the space.
- The agent cannot write the human's account branch, so this step must run on the human's device.
- Round trip: when the human claims, the device:
  1. Issues a new grant to the agent under the human (`space -> sponsor -> agent`, same scopes, renewable).
  2. Revokes the first `space -> agent` grant.

  After this, the agent's access comes from the human, and the human revokes it
  from the roster or from the agents list.

### Tools on the OAuth endpoint

- `tonk_list_spaces`
- `tonk_create_space`
- `tonk_query { space, document }`
- `tonk_evaluate { space, document }`
- `tonk_guide`

No secret goes through the model.

## Slices

| Slice | Content | Done when |
| --- | --- | --- |
| S0 spike | The agent invokes `/provider/add` with a delegation from the sponsor (subject = sponsor account), so the space is billed to the sponsor and syncs at once | Local access-service test passes, or we fall back: the space syncs only after the human claims and provisions |
| 0 | Rebase and land #1137 and #1138 (roster kind, inviter) | Merged |
| 1 | OAuth back with the agent-account provisioner, the `/settings/agent` page, CIMD. Tools: list spaces (empty) and guide | Live connect works in ChatGPT and claude.ai |
| 2 | `tonk_create_space` with the sponsor as founder, the agent's grant, the sealed seed, roster rows and provisioning | Native and wasm tests, plus a live create |
| 3 | Claim link: directory row, mount, open | Steps 1 to 5 of the user story work live |
| 4 | Round trip and revoke from the UI | Step 6 works live |

## S0 result (2026-10-28): passes

The access service already accepts a delegated `/provider/add`. Its auth is a
general UCAN chain check (`registration.rs` `verified_chain`), and the provider is
the invocation subject. The agent:

1. invokes with subject = the sponsor account and issuer = the agent;
2. sends the proof `sponsor -> agent`, command `/provider/add`, subject = the sponsor;
3. carries the consent `space -> sponsor`, signed with the space seed it holds at creation.

The space is billed to the sponsor. The agent never becomes a customer. When the
sponsor revokes the grant, the next call is `Unauthorized` and nothing is recorded.
No service change is needed.

Tests in `rust/tonk-access-service/tests/access_service/registration.rs`:

- `it_provisions_a_space_for_a_sponsor_through_a_delegate`
- `it_refuses_a_delegate_whose_sponsor_revoked_it`

Run them with:

```sh
nix develop -c cargo test -p tonk-access-service --features integration-tests \
  --test access_service -- delegate
```

What this means for the design:

- The consent page signs `sponsor -> agent` for `/provider/add` with a 90-day expiry,
  renewed on re-auth. Revoking it is part of revoking the agent.
- The space syncs at once, so the claim step only mounts and pulls.
- The same grant can also provision the agent's own account space (`kind` is
  `space` or `custody`), billed to the sponsor. This may answer question 5.
- Not covered by S0: the grant limits only the command, not how many spaces or how
  much storage. Any quota must come from the sponsor's plan.

## Open questions

1. Endpoint: OAuth on `/mcp`, with agent links moved to their own path? Or a new path for OAuth?
2. Agent scope: content read and write only (proposed), or also invite and admin?
3. Billing: the sponsor pays (proposed; S0 shows it works today), or the agent account
   is its own sponsored customer?
4. Does ChatGPT on your plan allow write tools for a custom connector? The docs say
   Business, Enterprise and Edu have full MCP. They do not agree about Plus and Pro.
5. Agent account durability: the agent's own account space also needs a sync remote,
   so that it survives the loss of the service. Until then it lives in the R2 checkpoint.
