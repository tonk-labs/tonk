# Agents from delegations

Status: proposed 2026-10-30. Replaces the self-declared agent label of #1137
and the roster rows #1228 writes for hosted agents.

## Goal

Any member of a space can tell which members are agents, who admitted each
one, and to what, by reading the delegations that admit them. No separate
label says so: the kind is a signed statement by whoever admitted the member,
and the roster follows from the delegations.

## Where we are

- **Hosted agents (#1210).** An agent is an agent space (`AgentSpace`
  mark). Approving it signs `space → account → agent space` grants
  (`agent_session::grant`) and keeps them only in the agent space
  (`record_grants`). The shared space keeps nothing, so its other members
  cannot see the agent, and only the approving account can revoke it.
- **Invites.** A claim keeps its chain in the shared space
  (`join::retain_claim_authority`). The invite's leaf already carries signed
  attribution: `invitation.inviter` (`tonk_invite::attribution`), checked to
  be the leaf's issuer or reachable from it through identity grants.
- **Labels (#1137, #1139, #1228, open).** `xyz.tonk.membership/kind` is a fact.
  On join the claimer declares itself (`agent: true`), which proves nothing.
  #1228 writes the kind for hosted agents from the approving worker, which is
  trustworthy only because that worker wrote it.
- **Dialog** decomposes a retained delegation's issuer, audience, subject,
  command and expiration into queryable facts, but not its signed meta.

## The rule

A member is an agent when the delegation that admits them to the space says
so in its signed meta, and the signer is the account that admitted them.

- Who admitted them: the leaf's `invitation.inviter`, checked as today.
- What they are: a new `member.kind` in the same leaf meta, `agent` or absent.
  It is read only when the inviter check passes, so only the admitting account
  can make the statement, never the member.
- A person is a member whose admitting leaf has no kind.

## Changes

1. **Sign the kind.** `tonk-invite`: `member_meta(account, kind)` writes
   `invitation.inviter` and `member.kind`; `member_kind(chain)` reads it back
   through the same check as `attribution::inviter`.
   - The approval (`agent_session::grant`) signs it on each grant's leaf
     (`account → agent space`).
   - An invite minted for an agent (an agent link) signs it on its leaf.
   - An ordinary share link signs no kind. A peer that joins through one is
     admitted as a member like anyone else; to be listed as an agent it needs a
     grant that says so.
2. **Keep the agent's grant in the shared space.** The approval retains each
   grant chain in the granted space's `main` too, as a claim does, not only
   in the agent space. Any member's peer then holds the delegation that admits
   the agent.
3. **Read the roster from delegations.** The member list joins retained
   delegations on the space (subject = space, audience = member) with the
   leaf's verified meta.
   - Dialog does not decompose meta, so the worker records, when it retains a
     chain, the verified `inviter` and `kind` as facts keyed on the
     delegation's CID (not on the member). Each fact names the delegation it
     came from, so any peer can re-check it against the chain and a
     revocation retracts it with the delegation.
   - The roster is a rule over those facts plus the decomposed audience, so a
     revoked or expired delegation drops the member without a separate
     roster write.
4. **Remove by revocation.** `member/expel` revokes the admitting delegation
   found in the space, for people and agents alike. This replaces #1228's
   special path, and lets any admin remove an agent, not just the approver.
5. **Retire the label.** Drop `agent: true` from join and the `MemberKind`
   write; FAB and Desktop read the kind from the roster rule. Existing kind
   facts are left in place but no longer read.

## What happens to the open PRs

- **#1137 (self-declared kind):** close; superseded by changes 1 and 5.
- **#1138 (inviter):** keep its `inviting_account` logic. Its `InvitedBy` fact
  becomes the per-delegation `inviter` fact of change 3.
- **#1139 (FAB and Desktop display):** keep the display; rebase it onto the
  roster rule.
- **#1228 (roster rows on approval):** replace with changes 2 and 4.

## Open questions

- **Dialog meta.** Should dialog decompose a declared set of meta keys, like
  issuer and audience, instead of the worker recording facts at retain time?
  Cleaner, but a dialog-db change and a pin bump.
- **Several leaves.** A member admitted by two delegations, one saying agent
  and one not: the most recent in force wins, or agent wins? Proposed: any
  delegation in force that says agent.
- **Old members.** Members admitted before this have no signed kind and show
  as people until they are admitted again.
- **Name.** The roster name of an agent: the agent space's name (what the
  person called it on approval) signed into the same meta, or read from the
  agent space? Signing it keeps the roster readable without the agent space.

## Verification

- Unit: `member_kind` accepts a kind signed by the inviter and refuses one
  signed by the member or below the inviter.
- Worker: approving an agent puts it on the space's roster as an agent for a
  second member's peer that never saw the agent space; expelling it as that
  second member, when an admin, revokes the grant.
- Live: approve an agent in ChatGPT, see it as an agent in another member's
  FAB and Desktop, remove it from there, and see the agent's next call refused.
