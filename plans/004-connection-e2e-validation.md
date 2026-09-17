# CLI and browser connection validation

2026-09-17. Functional follow-up to the connection-view audit.

## Environment

- Current worktree CLI built with `cargo build --offline -p tonk-cli`.
- Current worktree browser artifact built with `rust/tonk-ui/scripts/build-connection-test.sh /private/tmp/tonk-connection-e2e-artifact`.
- Real headless Chrome controlled by the repository's WebDriver harness, synthetic PRF passkeys, isolated browser profiles, test email activation, local access service/storage, and per-run localhost HTTPS origins.
- The local Caddy test wrapper includes the repository's `/connection/*` proxy. The previously cached wrapper lacked it and was replaced for this run.
- No existing user browser or CLI profile is used. Port 8080 was not listening during the initial probe.

## Failures discovered through integration

1. Native integration compilation failed because registration return handling called a Wasm-only helper. Use the equivalent document lookup at that shared boundary and gate Wasm-only completion code.
2. Account setup from an agent invitation was dropped at the nested portal boundary. Relay registration through the enclosing guest's authenticated bridge, preserving the child's focus/custody completion.
3. Signup returned to the signed terminal route but its enclosing settings section remained hidden. Reopen settings when the registration ceremony finishes on that route.
4. Management assertions still expected old copy and skipped the newly added keyboard-focusable details disclosure. Update those expectations without bypassing the user interaction.
5. The reused space iframe retained its pre-signup account-required invitation state. Recheck the invitation when registration closes, using the ordinary automatic request rather than the explicit sync-consent action.

## Coverage

- Exact one-space, two-space, and all-space terminal selection; CLI-owned recipient key; no account catalogue access.
- Browser-generated agent invitation copied before browser shutdown; CLI import and offline editing; reconnect; browser setup confirmation; revocation blocks further sync while retaining offline edits.
- Fresh browser signup returns automatically to the pending terminal request; cancellation preserves the existing CLI setup.
- Local space created before signup survives account setup; the user returns to that space and its copied invitation imports into the CLI.
- Offline terminal additions, later receipt, per-space revocation, retained edits, independent sibling access, and fresh access after removal.

Test functions live in `rust/tonk-ui/src/account_flow.rs` and `terminal_management_flow.rs`. Run with `cargo test --offline -p tonk-ui --features integration-tests,connection-invites <test-name> -- --nocapture`, setting `TONK_TEST_WEB_HOST=localhost`, `TONK_UI_TEST_SERVER`, and `TONK_UI_TEST_ARTIFACT` to the local fixture.

## Final results

All six real CLI/Chrome tests passed against browser build `8f6fd822f2ec2f3a`.

| Test | Result | Duration |
| --- | --- | --- |
| Exact selected terminal spaces | passed | 18.68s |
| Signup returns to terminal request; cancellation preserves CLI | passed | 6.40s |
| Agent connects after browser closes, edits, reconnects, and is revoked | passed | 12.19s |
| Anonymous space → signup/verification → same space → CLI agent import | passed | 8.09s |
| Offline terminal additions, revocations, and re-add | passed | 21.94s |
| Agent invitations independent of CLI accounts and sibling invitations; browser restart | passed | 19.00s |

Supporting checks: 34 library tests passed; three portal registration browser tests passed; native integration compilation, formatting, and diff checks passed. The local logs are `/private/tmp/tonk-e2e-{selected-final,terminal-signup-final,agent-final,recovery,management-final,independence}.log`.

These tests do not establish Safari, production deployment, real-device passkey, or external mail delivery compatibility. No UI redesign was added in this follow-up; source changes address the functional failures above.
