# B-07: founder roster reconciliation

Scope: COLLAB-05. Preserve unrelated absent-space invite changes.

Confirmed source difference: invite rotation migrates the membership bundle;
created-space rotation only replaces authority. Rename writes a root-keyed name
without membership/role, leaving the onboarding founder intact.

1. Reproduce create without root -> persist root -> rotate -> rename with a full
   membership/name/role assertion (not a name-only assertion).
2. Migrate created-space membership before resealing custody and retirement.
3. Repair already-rotated founder rows only with evidence binding the old member
   to this device and the space's direct current-root authority. Preserve other
   members, roles and authority; use the existing atomic roster migration.
4. Verify repeat no-op, existing affected state, live branch delivery, and sync
   where the local test environment supports it. Record remaining boundaries.
5. Update B-07 and regenerate/check Storybook data.

## Result (2026-09-07, base 8c9ff1de2)

- Reproduced fresh transition in the service-worker runtime: founder DID stayed
  onboarding while the saved name was keyed on the passkey root. Also observed
  the retired-key repair fixture fail before the fix.
- Created-space rotation now calls the existing atomic roster migration before
  resealing custody. Account-name projection repairs historical founders only
  with a direct current space grant and a retained old-account/device grant.
- Rotation tests: 5 passed. Includes fresh founder transition, joined-member
  preservation, pre-publication rotation, retired-key repair, live FABB query
  delivery, no-op retry, unrelated-founder preservation, local replica pull.
- Profile-name tests: 5 passed. Account-wide projection/catch-up test: 1 passed.
- Formatting, diff whitespace, Storybook freshness and 172 local links passed.
- Browser runner initially failed to start in the sandbox; the unchanged test
  command ran with local-daemon access. Existing FABB/cache dead-code warnings
  remain. No storage reset, production write, commit or deployment performed.
- Remaining verification: original user's popup on a rebuilt worker, hosted
  separate-device convergence. The replica test uses sibling local branches;
  it does not claim remote transport or full browser-ceremony coverage.
