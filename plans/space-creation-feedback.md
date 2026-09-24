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
