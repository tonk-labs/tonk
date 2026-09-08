# Email verification tab handoff

Status: original-tab setup revision implemented and verified in local Chrome (2026-09-08).

## Original-tab setup revision

Collect the display name in the original signup dialog before creating the
passkey. Carry it in `AccountCreation` and durably save it in the worker before
`enroll` sends the email. Verification still uses the existing accept-and-activate
step; its success page says “account verified” and asks the user to close the tab
and return to the original tab, with no resumed setup or onward navigation.
The original dialog observes activation and returns to the signed-in Hub;
interrupted sharing retains its copy-link action. Focus probes must check
activation, since a saved name now exists before verification.

Validation: all ten focused Chrome cases passed with retries disabled against
an immutable snapshot of the freshly rebuilt debug bundle: original-tab signup
(including empty-name rejection and durable name before email verification),
signup-to-share, activation on another device, unrelated-account activation,
returning login, duplicate registration actions, passkey retry, activation
layout, duplicate activation submission, and two waiting devices completing
when a third confirms. All six input/caret checks, `cargo fmt --all -- --check`,
and `git diff --check` passed. Release compilation was stopped after successful
debug-browser verification; release-mode execution, Safari, and the full E2E
suite remain unverified.

- Debug bundle snapshot: `/tmp/tonk-initial-name-spike`.
- Test server: `/tmp/tonk-initial-name-spike-server`.
- Test archive: `/nix/store/8vqdcbmsb3g23xkz5wzcarcyhx060ibw-tests-e2e-0.6.14/tests-e2e.tar.zst`.
- Logs: `/tmp/tonk-initial-name-spike.log`, `/tmp/tonk-initial-name-related.log`,
  `/tmp/tonk-initial-name-two-devices.log`.
- Runner: `/tmp/tonk-inline-run.sh` with the archive manifest
  `/tmp/tonk-original-tab-complete-artifacts.json` and the debug test server
  explicitly supplied. The archive's test bodies match this revision; browser
  execution uses the snapshot containing the corrected worker name seed.

The historical evidence below covers the superseded activation-tab flow.

The first new two-tab test stopped at the Hub trigger: its archive expected
“add an account”, while the web snapshot still contained “link an account”.
Concurrent commit `bad52a165` landed during snapshotting. Rebuilding both
artifacts from the settled worktree addresses this verification mismatch;
the failure occurred before signup and does not diagnose the new flow.

The signup-to-share regression then exposed a real ordering constraint:
`rename_display_name` requires hydrated account state, unavailable before
activation. Creation now authors the initial `AccountDisplayName` directly on
profile main for the root this ceremony just created, before enrollment. The
first activation sweep publishes and converges it; ordinary rename guards stay
in place. The static activation-page layout check passed.


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

## Inline activation-tab follow-up (2026-09-08)

The activation tab now mounts resumed setup as a section inside the existing
activated panel. The activated heading remains above the shared
email/name rows; the setup narrator and back action replace the panel's terminal
copy and links. It has no modal backdrop, fixed positioning, or focus trap.
Already-named accounts and activation for a different local account keep the
standalone activation result. Resumed setup skips email-lookup subscriptions,
which could otherwise replay a login action over the name step.

Verification:

- The strengthened two-tab regression failed against the previous UI bundle at
  the inline-parent assertion, then passed against the updated bundle (4.55s).
  It also checks static positioning, no modal, failed-save retry, durable name
  storage, and original-tab completion.
- Four related Chrome cases passed with retries disabled: another account's
  activation, activation on another device, returning login, and signup-to-share.
- All six input/caret CSS checks passed; Rust formatting and diff checks passed.
- A synthetic desktop/narrow-column Chrome preview confirmed the inline layout
  without horizontal overflow. Its font/logo assets were not served; this was
  layout evidence only.
- Tested server: `/nix/store/a952iy2zz5w0q5n2fhxx88vinml1b3rv-tonk-ui-test-server/bin/tonk-ui-test-server`.
- Tested archive: `/nix/store/fyr791xm0inrhgqndnaj1z2j82cfchnm-tests-e2e-0.6.14/tests-e2e.tar.zst`.
- Logs: `/tmp/tonk-inline-red.log`, `/tmp/tonk-inline-green.log`, and
  `/tmp/tonk-inline-related.log`; runner: `/tmp/tonk-inline-run.sh`.
- These pinned artifacts include the inline-setup changes but predate concurrent
  login auto-return, anchored-column outline, profile-library, and display-view
  edits. Those edits were preserved and are not covered by this run. Safari and
  the full E2E suite were not run.

### Presentation cleanup

Removed the redundant sync-activation receipt row and suppressed the account
page's inset focus shadow on inline ceremony editors. The native caret remains.
Updated the existing activation layout assertion to measure the remaining action.
Verified in an isolated Chrome fixture: the focused input has no shadow or outline,
retains its ink-colored caret, and the redundant row is absent. Six CSS checks,
Rust formatting, and diff checks pass. The full activation E2E was not rebuilt
for this HTML/CSS-only follow-up.

### Explicit display-name save

The name editor now offers `save display name`; Enter is a shortcut for the same
action. The standalone question is omitted. Empty narrator content collapses its
row; validation and save errors restore it. Saving disables the action and field,
then either restores Save for retry or offers the existing return/share action.
The two-tab regression exercises a failed save by clicking the button and a
successful retry using Enter.

Verification: the regression failed against the prior bundle because Save was
never offered, then passed on the new bundle (4.18s). Signup-to-share and the
activation layout test also passed, with retries disabled. All six CSS checks,
Rust formatting, and diff checks passed. Safari and the full suite were not run.

- Server: `/nix/store/dj5k6ckqf8mb8536708nwi5g8s3kl7yp-tonk-ui-test-server/bin/tonk-ui-test-server`.
- Archive: `/nix/store/cs78ca81ilcjc9b5wbfm3rh1019jcf3a-tests-e2e-0.6.14/tests-e2e.tar.zst`.
- Logs: `/tmp/tonk-name-save-red.log`, `/tmp/tonk-name-save-green.log`,
  `/tmp/tonk-name-save-related.log`.
