# The person in the agent's authority chain

Status: built, 2026-11-02 (account root issues). Follows #1238 (agent
joins from a link in the chat). E2E not yet run against a preview.

## Problem

In #1238 the link's key delegates straight to the agent space:

    space -> ... -> link key -> agent space

The person is not in that chain. So:

- "Your agents" cannot revoke it. The person holds no proof on that path,
  and the link's seed is not kept anywhere. `agents::revoke` only retracts
  the delegation from the agent space (`Issued::Link`). The chain stays
  valid at the space's access service. Anyone who kept a copy of it (the
  agent space's history, a writer that pulled it) can still use it until it
  expires (90 days at most).
- The space's admins see the agent as "someone who used the share link".
  They cannot remove the agent without revoking the link for everyone.
- Nothing ties the agent's access to the person. If the person leaves or is
  removed, the agent keeps its access.

## Proposal

The person claims the link first, then delegates to the agent:

    space -> ... -> link key -> person (profile) -> agent space

1. The person claims the link as a member, with the normal join
   (`join::join_invite`). The claim retains the person's own hop in the
   space (`retain_claim_authority`), so an admin can remove the person on
   their own. If the person is already a member, skip the claim and use the
   authority they hold.
2. The person's profile delegates the chosen scopes to the agent space,
   with the code the approval page and invites already use:
   `issuer_ancestors` (the profile's proof for each scope) +
   `issue_to_recipient` (the leaf to the agent). The leaf can never exceed
   what the person holds; `require_grant_deadline` caps its expiry.
3. Retain the chains and the directory entry in the agent space and push,
   as `hosted::access::retain_agent_grants` does now.

Then the grant is `Issued::Here`. "Your agents" revokes it with the path
that exists now: `ancestor` + `publish_target` publishes a revocation to
the space's access service, which refuses the agent's next call. No new
revocation code.

What this gives:

- Revocation is cryptographic, from "Your agents", with no new code path.
- The agent's access hangs off the person. An admin who removes the person
  also cuts off the person's agents (the revoked hop is on the agent's
  path).
- One rule for every agent grant: the person (or their account root)
  issues it. The `Issued::Link` branch in `agents.rs` and the link-signing
  code in `hosted::join::Offer::grant` can go.
- Invites addressed to the person (not only open share links) now work: the
  person claims them, then delegates. #1238 refuses those.

## The one constraint: only the person's browser can sign

The person's profile key, and the passkey for the account root, are in
their browser (or the desktop app). The chat card runs in the host's
sandboxed iframe on another origin. It cannot reach either key, and a
passkey prompt there fails (wrong `rp.id`, no permission policy from the
host). The hosted writer holds the agent space's key, not the person's.

So the join moves from the card into Tonk:

1. `tonk_join_space { link }` reads the link and shows the card, as now.
2. The card's button is "Join in Tonk". It calls `ui/open-link` with
   `https://<tonk>/agents/<agent key>/join#<link>`. The invite link stays
   in the fragment, so it never reaches a server.
3. The Tonk page shows the same choice: the space, "<agent> joins as
   itself, through you", read or read and write (write only when the link
   and the person allow it). It also says that the person joins the space.
4. "Join" calls a new worker route in the person's browser,
   `POST /api/agents/{key}/join { link, access }`, which does steps 1 to 3
   of the proposal. The page then says "Go back to the chat".
5. The card waits for the grant: an app-only tool
   `tonk_join_status { space }` asks the agent space's writer, which pulls
   and answers from `read_agent_access`. When the space is there, the card
   shows the joined strip and tells the model, as now.

`tonk_join_accept` (the link-signing path) goes. A host without MCP Apps
can give the person the same Tonk link as text.

## Decisions

1. **Issuer: the account root**, behind the passkey, as the approval
   page does. A claimed link's hop already ends at the root, so the root
   holds the proof it passes on. It survives the loss of a device.
2. **Grants already made from links (#1238)** cannot be revoked
   cryptographically: no one keeps the link's seed. The soft removal
   (`Issued::Link` in `agents.rs`) stays until the last one expires (90
   days at most); then delete it.
3. **The person joins the space as a side effect.** The join page says so.

## What was built

- `POST /api/agents/{key}/join { link, access }`
  (`router/join_for_agent.rs`): claims the link unless the account holds
  the space, checks the account can grant the access, then asks for the
  passkey with `CustodyIntent::GrantAgent { agent, space }`. The link never
  rides in the passkey request.
- After the passkey (`join_for_agent::authorize`): the root signs the
  grant (`agent_session::grant`), the agent space keeps it
  (`record_grants`) and is synced. Ceremony row `grant-agent` reports
  pending, working, done (detail: the space key) or failed.
- Join page: `/settings/agent-join#agent=<key>&link=<link>` in
  `profile.yaml` (pane `agent-join`).
- Hosted: `tonk_join_accept`, `Offer::grant`, `GrantBody` and `keep_grant`
  are gone. `tonk_join_space` answers `open` (the join page). The card
  opens it with `ui/open-link` and asks `tonk_join_status` (app-only) until
  the grant arrives.
- `agents::revoke` now queues the agent space to sync after revoking a
  grant it holds.

## Risks and checks

- The browser that opens the page must hold the agent space. The agent
  space is in the account's profile, so it is on every device that syncs
  the profile. Check: an agent made in one browser, joined from another.
- The page must not show the link, and must not log it.
- `ancestor` finds the profile's proof by `candidate_build_scopes`. Check
  that it finds the proof that goes through a claimed link, not only
  through a space the person made.
- Test (native, like `it_lists_agents_with_their_access`): claim an open
  link as the profile, grant read to the agent, revoke from "Your agents",
  and check that the access service refuses the agent's next read.
- E2E (`tests/e2e/hosted-agent`): share link -> card -> Tonk page -> join
  -> agent reads -> revoke in "Your agents" -> agent refused.

## Not in this change

- Admin access for an agent.
- A roster entry for the agent ("<name> bot, through <person>").
- Leaving a space from the chat.
