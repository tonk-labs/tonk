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
- More Options: the custom NSMenu popup failed through accessibility; deferring
  it exposed an AppKit assertion in `popUpMenuPositioningItem`. Replaced its
  detached manual anchor with a native SwiftUI Menu. Actual app checks now open
  the menu and Sync Status sheet successfully; the verification space reports
  matching local/remote revisions. New Link's keyboard shortcut opens its sheet.
  Existing person-link rows visibly expose both Reset and Revoke; no existing
  links were modified.
- Final full Swift run after menu replacement: 64 + 80 tests, 4 total skipped,
  zero failures. Signed build and `codesign --verify --strict` passed with system
  trust-store access. `cargo fmt --all --check`, generator `--check`, and
  `git diff --check` passed.
- Fresh worker invitation test compiled but did not execute: pooled Chrome
  timed out discovering DevTools; stock Chrome returned HTTP 404 during driver
  setup; Firefox returned HTTP 500 / "Could not find profile folder". Retried
  with canonical TMPDIR and both installed geckodriver 0.37.0 and 0.37.1.
  A launch probe confirmed the generated profile and user.js exist before
  Firefox starts. This remains a browser-runner blocker, not a test pass.

## Walkthrough still to record

After approved live checks pass, record the tested native build showing:
catalog/search, All Links/For Agents, overview/roster, Home/directory navigation,
chat toggle/focus, new person link and independent claim, reset and old-link
denial, agent MCP use and revocation denial, public read/unpublish denial,
restart/Copy/history persistence, and sync/error recovery. Keep bearer URLs
offscreen. Pair video timestamps with external-client results and source SHA.

## Approved live checks (2026-10-17)

- User explicitly approved person write/no-expiry links, agent write/90-day
  links, and temporary public reads in the dedicated synthetic space.
- Created person link; restarted Desktop; Copy and activity persisted.
- A browser address field contained extra text during the first paste and sent
  the test URL as a search. Immediately revoked that grant. A fresh private
  session then refused it with "this share link expired". No real user data
  was in this space. Keep all bearer URLs out of saved evidence.
- A fresh anonymous session claimed a replacement person link; roster reflected
  independent guest identities. Reset minted/copied another link and revoked
  its predecessor. Existing cached browser state is not proof of remote access.
- Found and corrected misleading copy promising joined members always keep
  access: their delegation paths can depend on the revoked invitation.
- A stale guest replica reintroduced a revoked invitation in the listing,
  although fresh clients were refused. Desktop now retains confirmed target
  revocations beyond its bounded activity feed and filters those stale rows.
  Tests cover revoke, reset, legacy journal decoding, trimming and persistence.
- Full Swift suite after these changes: 65 TonkTests +81 HarnessCoreTests,
  four total skips, zero failures.
- Public Make/Copy rendered both synthetic tasks in an independent IAB reader
  ("read-only, observing"). Make Private removed the public row; a new private
  browser session showed "this space is private". Cached copies may remain.
- CI for de825cdb18 passed all Rust/web/E2E/MCP checks, but Desktop Swift failed
  type inference at ContentView's startup error expression. Extracted a typed
  property; full local suite passed; pushed 7ef8d47f3a for CI verification.
- Agent grant minted through Desktop; real MCP client holds it only in memory
  while the native worker builds. Agent revocation/readback and video pending.

## Final candidate evidence

Application source candidate: `745fbcd42b` (subsequent changes are tests and this
verification record). Signed executable SHA-256:
`b939b54ad5163ddf50bc6494bd68e1286b973189eeabc25d3b060205c33e330e`.

- Split the startup view into separately type-checked views after CI showed
  that extracting only its error expression was insufficient.
- Corrected agent usage copy: per-call MCP does not register installations, so
  "No registered clients" replaces the unsupported claim "Not used yet".
- Final integrated Swift suite: 66 TonkTests +81 HarnessCoreTests, four total
  skips, zero failures. Permission-denial fault injection covers all three
  mint types and revoke, asserting no copied secret, success activity, or
  confirmed-revocation receipt is recorded on refusal.
- Hosted browser CI at de825cdb18 ran the exact
  `tonk-worker router::create_invite::tests::it_records_the_invitation_on_mint`
  regression successfully (Web tests debug 1/4, run 39065321620). Local browser
  driver setup remains unreliable, but this test is no longer unexecuted.
- Real stdio MCP client used the Desktop-minted agent link on tonk.foundation:
  listed tonk_query/tonk_evaluate, queried the task concept, wrote one synthetic
  `desktop-check` record, and read its marker back. After native Revoke, the
  same client was refused at pull. No evaluation was dispatched after revoke.
  The first client attempt lacked the host's configured CA bundle; passing the
  existing `/etc/ssl/cert.pem` to the child fixed it without disabling TLS.
- Fresh anonymous visits refused both a revoked person invitation and the
  pre-reset invitation. A valid replacement admitted an independent guest.
- Final cleanup: all person and agent grants created by this run were revoked;
  public access was withdrawn. Restarted Desktop shows 0 links /0 agents in
  the verification space and preserves local activity. Other spaces/grants
  were not modified. Synthetic records and guest roster entries remain.
- Final native Sync Status: matching local/remote revisions at 22:19:14 BST.
- Captioned walkthrough: actual native-window frames captured via
  the computer-use API, with pauses between checks trimmed. Includes a live
  rapid-toggle sequence, view navigation, scope controls, Copy, reset, cleanup,
  sync and restart. Public browser checks are labelled still evidence; MCP
  results are a labelled summary of the separate real-client test. No bearer
  URLs appear in the delivered video.

### Evidence limits

The video proves the displayed flows, not every possible desktop state. Network,
clipboard, Keychain, permission and stale-account failure paths were exercised
with deterministic fault injection. Approved live sign-out cleared the account,
spaces and access lists; browser passkey recovery restored the same jack account,
its catalog, and existing task data, with revoked test links still absent.
Different-account switching, expired sessions, and a system-wide offline
transition were not performed. The four
skipped diagnostic tests require separate local diagnostic servers. Signing is
local build evidence, not notarization or release-channel proof.

The video is a local artifact associated with this Codex task, not committed as
binary content in the repository. The final PR checks are tracked on PR #1116.

## Final hosted and recovery results

At revision `9823527bb1`, Desktop CI run 39070778586 passed native tests,
canonical generation/contracts, signing validators, and app build/verification.
All PR test checks passed; the preview MCP deployment was still running when
this record was updated. The walkthrough includes signed-out Welcome, restored
account/catalog, and reopened task data after browser passkey recovery.
