# Desktop sign-in progress and latency spike

Goal: make browser sign-in visibly progress and remove proven unnecessary wait.
Preserve the connected account, unrelated Desktop changes, and grant validation.

Evidence: the browser callback reached Desktop before the native Spaces screen.
The existing native flow gives preflight and browser wait the same label, does
not foreground Desktop until attachment completes, and reads independent local
identity/account state sequentially. The worker attachment itself hydrates the
account before returning; this spike does not skip required setup.

Implement explicit preparing/browser/authorization/account phases with elapsed
wait feedback; foreground Desktop after callback; parallelize read-only preflight;
record secret-free native phase timings. Keep attachment writes sequential and
retain the existing post-attachment reload until its removal is independently
proven safe. Verify concurrency and validation with a WebKit script fixture,
run relevant native tests, build the signed app, and inspect progress UI.

Validation: pending. No end-to-end speed claim yet.

## Implemented and validated

- Native stages distinguish preparing, browser wait, authorization verification,
  and account restoration. At 10 seconds, the current stage shows elapsed time;
  after 30 seconds it acknowledges the longer wait.
- Desktop foregrounds immediately on receipt of the browser callback.
- Independent preflight reads overlap. Grant import and account attachment remain
  ordered, and changed profiles are rejected before writes.
- Fixed phase names and monotonic durations are logged under xyz.tonk / sign-in;
  no credentials, identifiers, or callback URLs are logged.
- Stale catalog stream errors are suppressed while sign-in is active. After
  attachment, catalogLoaded is reset before the existing reload so an old empty
  catalog is not shown as a completed empty account.

Final repository-command tests passed: 14 account/startup/passkey/callback/grant
validation tests. Strict swift-format lint and git diff --check passed. The
signed local .build/Tonk.app was rebuilt successfully. Progress views were
rendered from the actual SwiftUI component and visually inspected as fixtures
(/tmp/tonk-signin-browser.png and /tmp/tonk-signin-account.png).

The initial direct xcrun fixture command failed because inherited SDK settings
hid swiftc; clearing SDKROOT and DEVELOPER_DIR resolved it. No source workaround
was required.

The running app has Metastaseis open, so it was not restarted. New progress and
native timing instrumentation take effect on the next launch. No sign-out or
new device approval was performed. End-to-end speed improvement is unmeasured;
required remote account hydration and the post-attachment reload remain.

## Stacked integration (2026-10-15)

Transferred only the five sign-in source/test files and this plan to
`feat/desktop-signin-progress` at `/tmp/tonk-desktop-signin-progress`, based on
PR #1101 head `ba9b12ea67ff775ea1a8921c9e60795fd55dc8a2`.
The four modified existing files matched the source worktree's original baseline.
The original Desktop Shared MCP working tree and unrelated edits were preserved.

Fresh checks on this branch: 14 focused native tests passed; strict swift-format,
git diff --check, and sync-desktop.mjs --check passed. The initial test attempt
stopped before compilation due to missing esbuild; npm ci installed the locked
MCP dependencies and the unchanged test command then passed. No lockfiles changed.

Target the PR at feat/desktop-rescue until #1101 merges, then retarget to main.
#1097 is already in this base's ancestry. Live passkey timings remain unmeasured
on this branch; original fixture/build evidence above is from Shared MCP.
