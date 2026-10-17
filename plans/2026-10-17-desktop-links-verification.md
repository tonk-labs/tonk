# Desktop links: fixes and executable verification

Status: in progress. Candidate starts at 3e377dac86542c1ba6856c03373a43b7e3bd5b56 (PR #1116).

## Required outcomes

1. Diagnose and fix sidebar animation completion; prove rapid reversal, focus,
   geometry and Reduce Motion with focused tests and the actual app.
2. Exercise person-link creation, joining, reset, revoke and partial failures.
3. Exercise agent-link creation through a real MCP client and verify revocation.
4. Exercise public reading and unpublishing, including denied permissions.
5. Verify Keychain copy and local history across restart, external-link Copy
   messaging, catalog/roster/connections/sync, and account-state isolation.
6. Open real Home and directory views; verify the runtime and chat pane.
7. Check offline/session/permission errors and recovery.
8. Record an actual-app walkthrough tied to the final candidate. Record which
   claims each segment proves; do not substitute fixtures for live evidence.

Use a dedicated verification space for mutations and publication, preserving
existing spaces and access links. Keep bearer URLs out of reports and video.
Record fresh commands, outcomes, and remaining gaps below. Commit independently
proven fixes, then update the draft PR with the final evidence.

## Evidence

- Baseline full Swift run: one animation reversal test failed with missing
  completion callbacks and a final collapse failure. Other tests passed.
- Live app is signed in and displays the links rewrite and account catalog.
  Existing spaces and links are not disposable test data.
- Animation fix: do not wait for AppKit animations in hidden windows. The
  rapid-reversal test now displays its window; a separate regression checks
  that hidden-window requests settle immediately and report the latest state.
- `bash desktop/scripts/dev.sh test:unit --filter AnimatedSidebarTests`: passed,
  followed by ten consecutive `swift test --skip-build --filter
  AnimatedSidebarTests` runs (4 tests each).
- `bash desktop/scripts/dev.sh test:unit`: 56 TonkTests and 80 HarnessCoreTests,
  2 skipped in each target, zero failures. `git diff --check` passed.
- Live setup: created `Desktop verification 2026-10-17` on tonk.foundation,
  subject `did:key:z6Mku9wwCXTY2iphGNMrRVE37i4PWV8NBQAGqTvLWjaFrEoM`.
  Desktop discovered it after refresh. The actual chat created two synthetic
  tasks and a directory; Home and the discovered Task view rendered both.
  Rapid chat toggles ended open with focus in the message field.
- Automatic approval review blocked live person-link creation pending explicit
  approval of whole-space write/no-expiry scope. A combined approval request
  also covers 90-day agent links and temporary public reads in this test space.
  No live link was created by this run before approval.
- Action regressions reproduced reset revoking the old link despite clipboard
  failure, and silent Keychain write failures. Fixed both; added person-link
  Revoke, account-generation guards, partial-read error presentation and catalog
  refresh on space-list changes. Eight focused action regressions cover these
  boundaries, including partial revocation failure and stale-account reads.
- Full Swift suite after access fixes: 64 TonkTests + 80 HarnessCoreTests,
  2 skipped in each target, zero failures (2026-10-17). Live access mutation,
  post-restart Keychain persistence, independent-client revocation and video
  remain pending; unit fault injection is not live account evidence.
