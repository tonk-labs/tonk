# Email verification tab handoff

Status: implemented and verified locally.

## Behavior

- A successful activation receipt resumes initial display-name setup in the activation tab only when its customer matches the local account root.
- Returning to the original signup tab re-reads the account's persisted name. Completed setup returns to signed-in home; an interrupted share retains its share-link action.
- Name submission uses the existing `/api/account/display-name` endpoint and waits for the durable write. Failed saves retain the editable field and allow retry.
- No account schema, core descriptor, or storage migration was needed.

## Evidence

- The unchanged two-tab regression failed three times with `no row named display name` in the activation tab.
- The original regression passed against the fixed web bundle.
- Five focused Chrome scenarios passed with retries disabled: activation-tab completion including failed-save retry; another account's activation; activation on another device; returning login with the synced name; signup followed by sharing.
- All seven `user_error::tests` passed with retries disabled.
- `cargo fmt -p tonk-ui -- --check` and `git diff --check` passed.
- The failed-save fixture initially supplied a synthetic response with an empty URL, causing reqwest to throw `url parse`. Supplying the request URL corrected the fixture; the final strengthened regression passed in 5.68 seconds.

## Reproduction

The initial test wrapper re-evaluated the concurrently changing worktree after building and selected an unbuilt archive. Verification therefore pins immutable artifacts:

- Web server: `/nix/store/52dyvxcbjjb3y1c83h2yapb5m48r4vg3-tonk-ui-test-server/bin/tonk-ui-test-server`.
- Final native tests: `/nix/store/dlpi5if3zvvdpbq5q2rxmbhnad67b3r1-tests-e2e-0.6.14/tests-e2e.tar.zst`.
- Final run: `nix develop --accept-flake-config . -c bash /tmp/tonk-email-handoff-final.sh`.
- Logs: `/tmp/tonk-email-handoff-final.log`, `/tmp/tonk-email-handoff-green.log`, `/tmp/tonk-email-handoff-unit.log`.

The fixed web bundle predates a concurrent terms-of-service paragraph added to the dialog. That copy change was preserved; the tested handoff logic matches current production source. Safari and the full E2E suite were not run.
