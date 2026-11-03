# Direct agent join with recipient-side revocation

Status: in progress, 2026-11-03. Revises PR #1240 (head `9c4a1a5aa8`,
base `d9e8852745`). Supersedes the account-mediated join the PR first proposed (see
"Rejected alternative").
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

## Rejected alternative

The PR's first version put the person's account in the chain
(`link key -> account -> agent`): the person claimed the link (joining the
space), then signed the agent's grant behind the passkey, on a Tonk page
the card opened. That made the account's existing revocation path work,
but made the person join and approve every agent join with a passkey.
The direct join keeps the chat flow; the agent's key gives the revocation.

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
- Steps 2 and 3: the revoke route asks for the passkey for link-issued
  grants (`CustodyIntent::RevokeAgent { agent, space }`, public names only)
  and the continuation calls the tested `withdraw`. The direct join
  (`tonk_join_accept`, `Offer::grant`, card) is restored from the base; the
  join page, `/api/agents/{key}/join`, `GrantAgent`, `tonk_join_status`
  are gone. Native: worker lib 346 passed (3 ignored) with
  `integration-tests`; standard_library 54; tonk-ui 44, tonk-analytics 30,
  tonk-schema 137, tonk-worker-api 36; tonk-ui node tests 192 (4 new for
  the agents panel). `cargo fmt --all --check` clean; clippy on touched
  crates shows only pre-existing dead-code warnings; wasm32 checks of
  tonk-worker, tonk-ui and tonk-host-worker pass.
- Not covered natively: loading the account from the custody cell
  (`custody::held_account`), the passkey prompt, and the page. The native
  test calls `withdraw` with the account root directly.
