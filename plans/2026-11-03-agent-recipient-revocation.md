# Direct agent join with recipient-side revocation

Status: in progress, 2026-11-03. Revises PR #1240 (head `9c4a1a5aa8`,
base `d9e8852745`). Supersedes the account-mediated join in
[2026-11-02-person-in-agent-chain.md](2026-11-02-person-in-agent-chain.md).
Evidence: [2026-11-03-recipient-revocation-spike.md](2026-11-03-recipient-revocation-spike.md).

## Decision

The agent joins straight from the chat card with the invitation's
authority. The person does not join the space and is not asked for a
passkey to join. A passkey is asked for when the person removes the
agent's access.

    join:    target space -> ... -> link key -> agent space -> session -> writer
    revoke:  passkey -> account root -> sealed agent key -> agent signs
             recipient revocation of each leaf -> target's access service

## Why the agent key

The spike showed that the browser's own authority over the agent
(`agent -> account -> device`) is scoped to the agent's space. A device-
signed revocation of a link-issued grant is refused. A revocation signed
under the agent's own authority (the hosted session's powerline) is
accepted, and the same writer's next read fails with `CREDENTIAL_REVOKED`.

Dialog lets a delegation's audience disclaim it: the revokers checked for
each link are the issuers at or above it plus its audience. The agent is
the audience of each link-issued leaf, so a revocation the agent signs
applies to every chain that presents that leaf, whoever presents it.

Only the account can open the agent key (`agent_session::open_held`), so
removal needs the passkey.

## Security boundary

Revoking a delegation invalidates every copy of that delegation. It does
not consume or revoke the invitation. Whoever holds the original reusable
link and its secret can redeem it again for a fresh grant. Removing the
agent does not invalidate the share link, and we do not revoke a shared
invitation for everyone. UI copy and the PR say so.

## Steps

1. Integration test (native, real local access service): create the agent
   through real repository creation and key custody; grant read and write
   to the agent directly from a link key; the person never joins; the
   agent reads; open the sealed agent key with the account through the
   same functions the passkey continuation uses; publish recipient
   revocations; the already-open writer's next read fails with
   `CREDENTIAL_REVOKED` while its grants stay published; a fresh reader
   with the old grants is refused. Keep the browser-device rejection test.
2. Removal handling:
   - `POST /api/agents/{key}/revoke { space }` revokes account-issued
     leaves as now. For link-issued leaves it asks for the passkey with
     `CustodyIntent::RevokeAgent { agent, space }` (no secrets) and answers
     `{ pending: true }`.
   - The continuation opens the account, checks it is the signed-in one,
     opens the agent key, checks the agent is this account's, revokes each
     live scoped leaf for the space (read and write, duplicates and
     renewals), checks each acknowledgement, retracts only acknowledged
     leaves, and queues the agent space to sync. Unacknowledged leaves stay
     listed and retryable.
   - Ceremony row `revoke-agent` and a per-space removal state in the
     agents listing report pending, working, failed.
3. Restore the direct chat join from the PR base (`tonk_join_accept`,
   `Offer::grant`, `GrantBody`, `keep_grant`, the card). Remove the join
   page, its route, `CustodyIntent::GrantAgent`, `join_for_agent`,
   `tonk_join_status` and the page polling. Keep the base's refusal of
   person-addressed invitations.
4. Tests and PR description around the final architecture.

## Verification log

- Step 1: `the_account_withdraws_a_link_grant_as_its_agent` passed
  (native, local access service, `cargo test -p tonk-worker --lib
  --features integration-tests`, in `nix develop`, loopback allowed).
  `mint_root_revocation` signed by the opened agent key is accepted for
  read and write leaves and a renewal; the open writer and a fresh session
  are refused with `CREDENTIAL_REVOKED`; the hosted copy still lists the
  grants. A wrong account is refused before any key opens; a failed
  publication drops nothing and the retry succeeds.
