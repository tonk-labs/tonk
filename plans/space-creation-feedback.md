# Space creation feedback and first render

User report: staging Hub creation has no immediate feedback, the new row appears
before navigation, and the space briefly shows a yellow “No directory view” notice.

Changes in this worktree:

- Give the Hub a branch-defined `space-create` control with immediate pending
  feedback, duplicate-submit prevention, and per-command completion/failure.
- Keep progress copy in the button; additional progress announcements are only
  for screen readers. Failure messages remain visible beneath the button.
- Keep local library seeding before navigation. Send navigation before draining
  the Hub subscriptions; remote attachment continues afterwards.
- Initialize display mode/facet before subscriptions can deliver their first
  frames. Ignore asynchronous fallback results from superseded display flows.

Validation completed:

- Two Wasm renderer regressions pass: immediate first frame and stale fallback.
- Wasm element test passes: pending, duplicate submit, failure, retry, navigation,
  and rejected transaction.
- Native worker transact/query test passes: receipt identity and seeded route.
  The initial sandbox run could not open its test profile; the unchanged binary
  passed with filesystem access. Its generated space was moved out of the checkout.
- All 46 standard-library tests pass. `cargo fmt --all --check` and
  `git diff --check` pass.
- Trunk preview build succeeds. Isolated local Chrome creation shows the disabled
  pending button and opening status, navigates, and settles into the new space.
  This smoke test preceded the small change hiding redundant visible status copy.
  The static preview lacks Trunk's development websocket and the optional account
  connections endpoint; their console errors are unrelated to space creation.

Live staging was inspected using an isolated browser. A local test-profile space
was created there; no existing user browser state was accessed. These source
changes have not been deployed. Full creation timing with an active account,
frame-by-frame confirmation of the reported flash on staging, CI, and Safari/device
behavior remain unverified. No measured latency improvement is claimed.

## PR 1002 CI follow-up

- Native CI exposed an analyzer assertion tied to the removed declarative create
  form. It now verifies the still-declarative remove form; the focused analyzer
  test passes locally.
- The persisted-worker upgrade test sampled the roster before the new custom
  element upgraded, then immediately required an enabled button. Its existing
  bounded wait now requires both the roster and an enabled create button. A missing
  button no longer counts as enabled. This E2E adjustment awaits hosted validation.
- The invitation signup test passed on retry; the account aggregate failed because
  the upgrade-test shard failed. Formatting and whitespace checks pass.

The subsequent run on merge `746b50015` advances past Hub readiness but stalls
with the successor installed and the old document still controlled. The exact
persisted-upgrade test passes locally on that revision with preview features,
Chrome 153 and Chrome for Testing 150.0.7871.124, including the exact release
artifact from the failing CI run (build `6e6da3e4e5665c5d`). This does not establish
hosted recovery. The timeout now collects incumbent health/logs after the normal
observation window, without extra fetches during activation or weakened checks,
to diagnose the remaining CI-only stall.

The diagnostic run `36026578641` still failed. Its incumbent log confirms startup
retirement and `Streams are released` immediately, followed by ordinary Hub load
requests while the successor remains waiting. Offline preparation independently
extends fetch/message lifetimes and was neither cancelled nor gated by retirement.
A deterministic test reproduced that scheduled lifetime remaining pending after
retirement. The fix cancels its idle timer, settles the lifetime, prevents later
messages from rearming it, and avoids obsolete-worker cache maintenance after stop.
The new regression passes, all 91 service-worker tests pass, and the exact CI
release binaries with this JS fix pass the persisted-upgrade browser test on Chrome
150.0.7871.115. Hosted confirmation of this fix remains pending.

Run `36032615266` passed E2E only after the upgrade test retried: the first attempt
still showed an installed successor after incumbent stream release. Do not treat
that green aggregate as proof of deterministic recovery. The page's activation
nudge was one-shot and could be missed when the waiting registration became
visible after the installed state event. A new deterministic regression fails
before the page rechecks durable registration state and retries activation. The
recheck never polls the incumbent's data plane, retires it only once, and stops on
adoption or failure. All 92 service-worker tests and the persisted-upgrade browser
test pass locally with both fixes; first-attempt CI confirmation remains pending.

Run `36039517272` still fails all three persisted-upgrade attempts after both
fixes. Streams release promptly, but the successor remains installed/waiting.
The fix is not yet verified. A test-only worker probe now records pending event
lifetime promises and activation requests, sampled from both workers only after
the existing deadline. It does not alter those promises or consume response
bodies. Its observation check passes and the instrumented browser test passes
locally in 50.69 seconds; Linux diagnostics remain necessary.

Run `36045095796` identifies the remaining blocker. Every failed attempt has
fresh incumbent query `waitUntil` promises; completed ones take exactly 500ms.
Successor activation requests arrive but remain pending. `schedule_sync_drain`
checks the stopped latch only after its debounce sleep, so frequent handoff
queries continuously extend the retired worker's event lifetimes. Check the
latch before creating a ticket, promise, or `waitUntil` extension.

The new Wasm regression fails before this guard (six lifetimes instead of one)
and passes after it, including settlement of the existing live-worker timer.
The guard alone still fails locally. Chromium tracing identifies a subsequent
profile query restarting the incumbent immediately after the browser stops it,
before the successor takes control. The page now closes its readiness gate
during the installed-worker handoff, and the host checks that gate before each
new IO instead of memoizing initial readiness forever. Existing work settles;
new requests resume when the handoff outcome resolves.

The previously failing local persisted-upgrade test passes in 56.04 seconds with
the combined fix. All 58 host Wasm tests, 32 scheduler/routing Wasm tests, and
136 JavaScript tests pass. The temporary activation retry timer and lifetime
probe are removed; cache retirement and timeout health diagnostics remain.
Hosted first-attempt confirmation is still pending.


Run `36054213400` confirms the persisted worker upgrade passes on its first
attempt (94.854s). It exposes two other repeated failures: registration focus
restoration and the competing historical account writer. The latter did not
actually remain historical: a local reproduction found its generation-A cache
removed after its login/reload initiated a background upgrade. The test server
now honors a browser-local fixture cookie selecting generation A for static
assets, while account endpoints and worker update behavior remain real. Only
the old writer gets this cookie. The unchanged migration assertions pass locally
with this fixture (288.61s, including debug-artifact generation and upgrade).
Focus restoration is still under investigation; exact CI artifact reproduction
also catches Escape before the deferred dialog opening has settled.
