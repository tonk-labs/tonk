# Agents as a first-class primitive

Status: in progress. Step 0 (spike), the authorship primitive in dialog-db and
stable worker session keys are done or in review; roster provenance is next.
See **Progress**. First inspected at 8d5035bea4, plus `feat/desktop-links`,
`origin/feat/agent-peer-views` and `feat/harness`.

## Progress (2026-10-20)

- **Spike (step 0)**: Pi Durable DO + Discord stand-in + hosted agent-link MCP,
  run against a real space. Branch `claude/magical-almeida-b33ceb` (unpushed);
  findings in its `spikes/pi-team-agent/LOG.md`. Confirmed the agent needs a
  stable identity (per-link keys orphan DO state on 90-day renewal), roster
  provenance (it never appears on the roster) and cross-space grants. Hosted MCP
  calls take 13-18 s each (native worker spawned per call); error codes are
  opaque. Real model, real Discord app and deployment not yet exercised.
- **Authorship in dialog-db**: dialog-db/dialog-db#614, open for review. See
  **Authorship**. Tonk consumes it after it lands and the dialog pins move.
- **Stable worker session key**: tonk-labs/tonk#1130, merged. The worker's
  operator is derived from the profile plus access branch instead of random
  bytes per boot; renewal re-issues only the bounded grant.
- **Local agent peer**: landed via tonk-labs/tonk#1111 (#1108 closed). It is the
  local agent runtime this plan assumed.

Motivating build: a shared team agent on Pi Durable in a Cloudflare Durable
Object, reached first through a bot in the team Discord, acting in Tonk spaces.

## Where we are

Today an agent is not a principal. It is a fresh Ed25519 key minted per tool
link (`tonk-invite/src/connection.rs`, `AgentInvite`), holding a fixed data
scope set for one space (`candidate_build_scopes`: main branch get/put, no
`meta`), 90-day TTL. The chain is
`space -> inviter -> account root -> issuing profile (powerline) -> link key`.

Consequences:

- One agent in N spaces = N links = N keys = N CLI connection dirs = N MCP
  replicas. Nothing ties them back to "the same agent".
- Agents are absent from the roster (`tonk:member`). The only records are the
  issuer-side `AgentGrantGroup` ledger on the human's account branch and
  self-reported `agent-connection` / `agent-installation` receipts in the space.
- Write provenance exists only as the revision issuer DID; nothing maps that
  DID to "agent X, sponsored by Y".
- `agent-grant`, `agent-revocation`, `agent-installation` attributes have no
  library YAML concept (schema-first gap).
- Desktop's harness (Claude Code / Codex / API chat) acts as the signed-in
  account itself, so its writes are indistinguishable from the human's.

Two models already exist in tree and pull in opposite directions:

| | Agent links (main) | Agent peer (`feat/agent-peer-views`) |
| --- | --- | --- |
| Identity | seed in the link, per link | long-lived key in `~/.tonk/agent-peer` |
| Joins as | scoped data grant | ordinary member via share link |
| Cross-space | no | yes |
| Scope / revocation | narrow, per grant set | whole membership |
| Provenance | issuer ledger | roster row, no "agent" marker |

And a vestigial hook pointing at the convergence: `PublicGroup.terminal_request`
in `rust/tonk-worker/src/router/agent_connections.rs:40` ("Flow A": browser
delegates to a key the agent already holds). Every mint sets it to `None`.

## Proposal

### 1. An agent is a principal with a stable DID, sponsored by default

Recommendation: **agent first, then choose spaces** (the direction you were
leaning). An agent is registered once at the account level and receives
per-space grants addressed to its own DID. Spaces never own agents; they hold
grants to them and show them on the roster.

Why not space-first: cross-space work (the team agent reads the roadmap space
and writes to the standup space) would need N identities stitched together
after the fact, which is exactly the current gap.

Converge the two in-tree models: **long-lived identity (peer model) + scoped,
audience-bound grants (link model)**. Concretely, finish Flow A: the agent
presents its DID, the issuer mints the grant bundle with `audience = agent DID`.
The seed never travels. Agent links with an embedded seed remain as the
bootstrap for agents that cannot hold a key (the link *is* custody), and the
hosted agent-link MCP keeps working unchanged.

Three properties describe any agent, independently:

- **custody**: who holds the key. `self` (the DO's secret, the local peer's
  data dir), `tonk-hosted` (agent-link service R2 checkpoint), `link` (seed in
  a bearer URL).
- **runtime**: `local` (Desktop harness, CLI peer), `hosted` (our DO / service),
  `external` (someone else's long-running agent: Grok, ChatGPT, Hermes, ...).
- **controller**: who may reconfigure and revoke it. Initially one sponsoring
  account. Later a set (see 4).

### 2. Provenance on the roster

At grant time the issuer also writes a roster row in the space, the way
`router/join.rs` writes `InvitedVia` for people:

- `tonk:member` gains `member/kind` (`person` | `agent`) and, for agents,
  `member/sponsor` (the account root DID the chain passes through) and
  `member/grant` (grant set id).
- Writes resolve to these rows through authorship (see **Authorship**): the
  profile a revision is proven to act for is a roster member's key, which turns
  "commit by did:key:z6Mk..." into "Standup Bot, for Jack".
- The full chain back to a human is already decomposable into
  `dialog.ucan/*` authority facts (`plan/authority-facts.md`); a provenance view
  renders it. The roster row is the cheap index, the chain is the proof.

Role stays descriptive, as for people: the row grants nothing.

### 3. Account-level registry

A `tonk:agent` concept on the account branch (the human's profile repo), keyed
by agent DID: name, runtime, custody, endpoint (if external/hosted), controller,
created. Grants for that agent group under it, replacing the per-link
`AgentGrantGroup` as the unit the UI shows. Declare it, and the existing
`agent-grant` / `agent-revocation` / `agent-installation` attributes, in the
library YAML.

Revocation is unchanged in mechanism: revoke the leaf per space
(`agent_connections::revoke`), or revoke all of an agent's grants at once
because they now share an audience.

### 4. Agents that do not report to one human

Ordering, smallest first:

1. **Sponsored** (now): chain runs through one human account root. Human leaves
   or revokes, agent loses access. Correct default.
2. **Team-controlled**: space admins issue the grant directly
   (`space -> admin -> agent DID`) instead of via a personal account, and the
   controller set is "admins of a home space". Ownership survives any one person.
   This is the right shape for the team agent fairly early.
3. **Independent**: the agent holds its own account root (a ceremony without a
   passkey, key custodied by its runtime) and is invited like a person. The
   model above already accommodates it: `sponsor` is simply absent.

### 5. The agent's home is a Tonk space

Per the survivability heuristic, durable agent state lives in Tonk, not in the
runtime. Each agent gets a home space holding its configuration (system prompt
sections, enabled tools/integrations, which channels bind to which spaces) and
its session journal (`feat/harness` already journals `agent-session` facts).
Controllers edit config as ordinary Tonk data; the runtime reads it and
hot-reloads (Pi Durable's `registry.install()` hot-replaces extensions).
The DO's SQLite is the working set, not the record.

## Authorship: who made a change

Membership (who is in the space, how they got in) and authorship (who made this
change) are separate. The roster answers the first; dialog-db answers the second.

What was wrong: a revision record names the profile its session key acted for
(`authority`), but only the session key signed it, and nothing proved the link.
Writes are signed by per-session operator keys, for humans and agents alike.

Fix (dialog-db/dialog-db#614): a session's `profile -> operator` delegation
(whole authority: any subject, root command) is stored once per tree as a
`dialog.db/delegation` fact on the operator's own entity. Records are unchanged.
`dialog.revision/attested` lists what a record proves: always the operator, and
the profile when a delegation on the operator binds it. Attribution, not
authorization, so expiry is not checked. Measured against two alternatives
(delegation embedded per record; hash reference per record), it is the only one
that passes dialog-db's perf gate and leaves the record format unchanged.

Not covered yet: the proven authority is the device profile, not the account.
For humans, resolving to the roster's account needs the `account -> device`
hop as well. For agents with their own key, the profile already is the member.

A deterministic session key was considered and is not a substitute: a key
derived from the profile's secret cannot be checked by anyone else, so the
profile's signed delegation is the proof either way. (#1130 made the key stable
for lineage reasons; the proof still comes from the delegation.)

## Agents vs integrations (Desktop)

The distinction that holds up: **does it hold its own authority?**

- **Integration**: a capability with no identity: MCP servers, CLIs, bash,
  HTTP APIs, computer use. It acts as whoever invokes it. Configured per agent
  (or per Desktop harness) with a policy.
- **Agent**: a principal. Own DID, own grants, own memory, addressable, can act
  on its own schedule. Uses integrations.

So the Desktop sidebar gets two top-level sections next to Spaces:

- **Agents**: one row per registered agent (local harness, hosted team agent,
  connected external agent). Detail: runtime, controller, spaces with access
  (add / revoke per space), last activity from the roster/journal, provenance.
  The current "For agents" page (links listed by space) regroups by agent.
- **Integrations** (today's "Connections"): tools available to agents.

"Add agent" flow: pick the kind, then pick spaces.

- *Local coding agent*: Desktop harness gets its own key and an
  `account -> harness` sub-delegation, so its writes stop being attributed to
  the human. Spaces chosen per agent.
- *Hosted team agent*: create, choose a home space, choose spaces, choose
  surfaces (Discord, Tonk thread).
- *External long-running agent*: two directions, both need the same identity
  + grants:
  - inbound: the agent is an MCP client of Tonk (exists: agent-link MCP).
    Give it a link (custody `link`) or, if it can hold a key, register its DID.
  - outbound: Tonk messages the agent (its endpoint: A2A, webhook, a Pi Durable
    socket). Needed for "@agent" from a Tonk view. Later.

## The team agent on Pi Durable

Pi Durable gives the runtime: durable conversations, inbox serialisation,
alarms, hibernatable sockets, subagents as tools, hot-swappable extensions.
It ships **no auth** and no remote execution env; that is the Tonk part.

Shape:

```
Discord gateway/interactions -> Worker -> DO "agent:<did>" (Pi Durable harness)
                                          |  conversation per Discord thread
                                          |  tools: tonk_query / tonk_evaluate
                                          v
                               Tonk (agent DID's grants, per space)
```

- One DO per *agent* (not per Discord session); Discord threads and Tonk
  threads map to Pi conversations. `whenBusy: "followUp"` gives ordered
  multi-user input for free; `attributedPrompt` carries the Discord author.
- Key: agent seed as a Worker secret (custody `self`). Its DID is what Jack
  grants spaces to.
- Tonk access, spike: call the hosted MCP (`agent.tonk.host/mcp`) with an
  ordinary agent link; zero Tonk changes. After Flow A: same service, but grants
  addressed to the agent's DID and the DO signs (or a native replica keyed by
  the DO's seed, like `agent_link.rs` replicas today).
- **Confused deputy is the main risk.** Anyone in the Discord can drive the
  agent, and the agent acts with its own authority, not the caller's. Bind each
  Discord channel to one space in the home-space config, refuse cross-space
  writes from a channel, and record `requested-by` (Discord user) on writes.
  Linking Discord users to Tonk accounts is a later step and should gate
  *which* bindings they can use, not hand the agent their delegation.
- Coding (shell/files) needs a Pi execution env on Cloudflare Containers or a
  remote box; out of scope for the first cut.

## Sequence (each an independently landable PR)

0. **Spike, no Tonk changes** (done): Pi Durable DO + Discord stand-in + hosted
   MCP with an agent link to one space. See **Progress**.
1. **Roster provenance** (priority, in progress on `feat/agent-roster-provenance`):
   `member/kind`, `member/sponsor`, `member/grant` written in the shared claim
   path and at agent-grant time, with `kind` derived as in **How `kind` is
   decided**; library YAML for the agent attributes. Agents that join via the
   local peer must show up correctly. Needs nothing from dialog-db.
1b. **Attribution on the roster** (after dialog-db#614 lands and pins move):
   join `dialog.revision/attested` to roster rows, and add the
   `account -> device` hop for humans.
2. **One key per agent install / Flow A server-side**: mint a grant bundle to a
   presented DID (`terminal_request`) instead of a key per link. The peer
   already gets this by re-delegating from the link key. Also closes the gap
   #1130 surfaced: one agent link used from two hosts now signs as one
   operator, two unlocked writers on one profile.
3. **Registry + Desktop Agents section**: `tonk:agent` on the account branch;
   group grants by agent; add/revoke spaces per agent.
4. **Agent home space**: config + journal; DO reads config from it.
5. **Team-controlled agents**: admin-issued grants, controller set.
6. **Outbound messaging** to external/hosted agents from Tonk views.

## Decisions (2026-10-19)

- **One key per install**, sub-delegated from the account. Not per conversation.
- **No grant-scope tracking for now.** Agents keep the fixed data scope set; no
  `meta`, no admin or invite rights, and nothing to model about scope variants.
- **Roster provenance is the priority.** For any actor it must be cheap to
  answer: human or agent, and who invited whom (back to a human when there is one).
- **Renewal, not unbounded grants.** Keep the 90-day TTL; long-lived agents are
  renewed by their controller.

## How `kind` is decided

Same primitive for humans and agents; only the label differs, and the
sponsoring account sets it as signed meta on its `account root -> key`
delegation (`kind: agent`). The roster label is *derived from the chain*, not
declared by the leaf:

- **Agent is sticky.** If any hop in the chain is an `account -> key`
  delegation marked agent, the leaf is an agent, whatever later hops say. An
  agent that re-delegates (or one day invites) another key cannot launder it
  into a human, because its own marked hop stays in the chain.
- **Human** means the chain ends at an account root, or at that root's devices
  via a device link, with no agent hop on the way.
- **Who invited whom** is the chain itself, so the roster always shows the
  inviting actor and its kind. Even a mislabelled actor sits visibly under an
  agent.
- **Unsponsored agents still trace to a human.** An agent that never signed in
  (e.g. the local peer before sign-in) got in through a link, and every link is
  minted by a human's hop. The roster shows the nearest human above the first
  agent mark: "joined via Jack's link". The kind comes from either signer:
  - Agent links (`connect agent`) put `kind: agent` in the leaf's signed meta,
    so the human declared it.
  - On a share link nobody declared it, so the claimer self-declares when it
    redeems. That is safe because the asymmetry runs one way: claiming *agent*
    is never an escalation, so a self-declaration is accepted. Claiming
    *human* is the only direction that needs the chain's backing. The peer
    always self-declares.
  - Wording: share links are bearer and can be forwarded, so "joined via
    Jack's link" is accurate. "Jack's agent" is only true for a per-agent agent
    link or after sign-in sponsorship.
- Residual case: an agent holding its own account root (independent) can call
  itself human. Cryptography cannot stop that; the inviter's hop is still
  visible. Not a concern while agents have no invite rights.

## The local agent peer (landed via #1111)

Irakli's agent peer is the *local runtime* half of this, now on main: a per-machine MCP server (`tonk-agent-peer`, Claude Code plugin + Desktop
`.mcpb`) with one long-lived key in `~/.tonk/agent-peer`. Every agent session on
the machine is the same peer; one session owns the data dir and syncs, others
forward over a socket. `tonk_join` takes a pasted link:

- share link: ordinary `/api/profile/join`, so the peer becomes a **roster
  member** like a person (claim redelegates to the peer's own root).
- agent link: the peer uses the link key once to delegate each grant onward to
  its own key, stores the extended chains, discards the link key. This is Flow A
  done client-side: stable identity, multi-space, link not retained.

#1111 adds live views as MCP Apps (`tonk_show`, `tonk_watch` long-poll,
`tonk_display` running the real `<tonk-display>` guest runtime against a
loopback worker API).

How it fits:

- It is the right local runtime. The plan's "local coding agent" row is this
  peer, not a new thing. Desktop's harness could launch the same peer instead of
  acting as the account.
- Provenance gap: joined via a share link it is indistinguishable from a human
  on the roster, and before sign-in its root is a fresh onboarding account, so
  there is no sponsor in the chain at all. Sign-in (re-rooting under an account,
  listed as "not in this PR") is what makes it a sponsored agent.
- So roster provenance must land in the shared claim path (`router/join.rs`),
  not only in agent-link minting. `kind: agent` may come from the claimer's
  own declaration (never an escalation); `human` only from the chain (see
  **How `kind` is decided**).

## Open questions

- Loopback API on the peer is unauthenticated (noted in its README); matters
  once a peer holds many spaces.
