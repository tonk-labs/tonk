# Fail-closed account deletion hotfix plan

**Goal:** Prevent whole-account deletion from crossing any local or remote boundary until a passkey-root-signed, replay-safe deletion protocol is available.

**Approach:** Route every direct `POST /api/account/delete` request to a fixed unavailable response before profile state, inventory, or remote services are touched. Disable only the whole-account Settings entry with explicit unchanged-state copy; retain the existing implementation and the separately scoped owned-space deletion path for the follow-up saga.

**Constraints:**

- Keep the existing deletion orchestration intact for a reversible follow-up.
- The worker refusal is authoritative even when the UI is bypassed.
- Do not clear, rotate, unlink, or otherwise mutate local account/profile/space state.
- Do not change the current exact owned-space deletion authorization policy in this hotfix.

## File map

- `rust/tonk-worker/src/router.rs`: bind the whole-account endpoint to the fail-closed handler and prove the public response/state contract.
- `rust/tonk-worker/src/router/account_deletion.rs`: fixed unavailable handler; preserve the existing deletion implementation below it.
- `rust/tonk-ui/src/account.html`: disabled whole-account control and honest recovery copy.
- `rust/tonk-ui/src/account.rs`: preserve the disabled state across busy transitions while keeping exact-space review available.
- `docs/storybook/accounts/authority-and-deletion.md`: current user-visible safety state.
- `docs/storybook/bug-triage.md`: critical authorization finding and mitigation status.
- `docs/storybook/journey-catalog.md`: current versus historical whole-account deletion evidence.
- `docs/storybook/verification/accounts.md`: executable hotfix acceptance item.
- `docs/storybook/app/data.json`, `docs/storybook/app/data.js`: regenerated product map.

### Task 1: Refuse direct whole-account deletion

**Interfaces:**

- Consumes: `POST /api/account/delete` with any request body.
- Produces: HTTP 503 with an `account_state_unavailable` envelope stating that no account, spaces, or local data changed.

- [x] Add a public-router regression that expects the fixed unavailable response and unchanged active profile/provider state.
- [x] Run the focused test and observe the existing route fail the new expectation.
- [x] Route the endpoint to a no-extractor fail-closed handler, leaving `delete` unchanged for the future protocol.
- [x] Re-run the unchanged focused worker test successfully.

### Task 2: Disable only the whole-account UI entry

**Interfaces:**

- Consumes: ordinary `/settings` and exact `?delete-space=SUBJECT` settings routes.
- Produces: unavailable whole-account copy/control; unchanged exact-space review control.

- [x] Add a DOM regression for both variants. Its separate RED run was intentionally skipped under the disk/Cargo coordination constraint after the worker boundary supplied the required RED proof.
- [x] Author the disabled control/copy and keep it disabled after busy-state changes.
- [x] Explicitly re-enable the exact-space review variant.
- [x] Run the focused UI test successfully.

### Task 3: Document and publish the hotfix

- [x] Update Storybook source documentation and verification coverage.
- [x] Regenerate and validate Storybook data and links.
- [x] Run formatting, diff/static checks, and the committed base-aware Storybook impact check.
- [x] Commit reviewed files, push the branch, and open unmerged PR #834 against `staging`.
- [x] Record the complete fail-closed follow-up protocol in
  `plan/account-deletion-saga-v1.md`; do not re-enable the route in this PR.
