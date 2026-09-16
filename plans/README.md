# Implementation plans

Read each plan fully before starting, follow its verification gates, and record
fresh evidence when updating status. Do not infer implementation from a plan.

## Execution order and status

| Plan | Title | Priority | Effort | Depends on | Status |
| --- | --- | --- | --- | --- | --- |
| [002](002-cli-access-workflows.md) | Keep only link and one-way connect for CLI access | P1 | M | 001 | LOCAL IMPLEMENTATION VERIFIED; removes join and browser-mediated connect |
| [001](001-cli-space-connections.md) | Give the CLI explicit access to spaces | P1 | L | Revised milestones 0–7, in order | LOCAL IMPLEMENTATION VERIFIED (2026-09-16), milestones 0–6; milestone 7 release preparation complete, external release gates unrun |

Plan 001 was originally written on 2026-09-15 and revised to the user's clarified
protocol on 2026-09-16. It now specifies two ordinary UCAN delegation flows:

- Browser-generated agent invite: fresh invitation key pair and scoped grants
  carried in a reusable bearer link; no CLI login or browser round trip.
- CLI-initiated linking: CLI retains its private key, browser selects spaces and
  issues specific delegations to the CLI's public key.

Both use long-lived grants with explicit expiry and standard UCAN revocation.
The additive contract requests 90 days and explicitly reports ancestor limits. Browser
connection records are management metadata, not a second authorization database.
Single-use redemption, short user-facing sessions and connection checkpoints are
not part of this version.

The [historical authorizer experiment](001-connection-authorizer-proposal.md)
and its tested patch/harness are preserved but superseded. They are not publication
or integration prerequisites. The independent presigned-S3 expiry issue remains
in scope; extract a minimal fix if needed. Earlier tests do not complete the
revised protocol's gates.

## Superseded direction

The target design in [CLI join agent mode](../plan/cli-join-agent.md) is superseded
by plan 001. Its implementation was retained in commit `5026010c8`. Plan 002 now removes
its CLI join and browser-mediated agent entry points without deleting local data.

## Existing plans

These files predate this index. Their implementation status was not audited here;
they remain unchanged and are not dependencies of plan 001.

- [Cold-start deferral](cold-start-deferral-implementation.md)
- [Cold-start implementation](cold-start-implementation.md)
- [Cold-start network fixes](cold-start-network-fixes.md)
- [Preserve Welcome across seed checks](welcome-seed-upgrade-fix.md)

## Considered approaches not selected

- Merely renaming `connect` to `join --agent`: keeps account-wide CLI setup and
  does not supply scoped invitation/terminal authority.
- Single-use bootstrap redemption and a custom active-connection authorization
  store: superseded by reusable bearer invitations and direct CLI delegations.
- A short-lived child as the only lifetime bound: the retained parent controls
  the maximum lifetime; both flows intentionally use long-lived parent grants.
- Globally removing account checks: would mix legacy and session authority and
  risks falling back to credentials outside the selected space grant.
