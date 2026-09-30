# Leave and delete space dialog feedback

Fix menu button rules leaking into nested dialogs, preserving the Hub action's
ink/on-ink contrast in both themes. Capture removal submission in `space-remove`
so it disables repeated submissions and displays leaving/deleting immediately.
Subscribe to a request-specific worker receipt before dispatch; restore the
button and show failures for retry. Cancel the subscription on disconnect.

Validation: focused display browser regression covers pending feedback,
duplicate submit suppression, failure and retry; worker regression covers
success and refusal to remove the profile itself. Browser fixture checks the
actual library CSS, dialog CSS and removal methods in both color schemes.
Validation results are recorded after checks finish.

Browser fixture: passed light/dark foreground/background checks, immediate
pending text and disabled state, duplicate-submit suppression, failure recovery,
successful modal close and subscription cancellation. Final markup includes a
polite live announcement on the submit control. This fixture uses extracted
library/dialog CSS and library methods with a mocked host bridge, not a full
Tonk session.

Display wasm regressions passed via `wbg-pool` with browser access. The sandboxed
runner cannot start its daemon and times out; this is an environment boundary.
Native worker receipt regression passed (success and refused self-removal).
The sandboxed run failed opening its test profile with Operation not permitted;
the unchanged compiled binary passed with storage access. Built with the local
`profile.test.package.tonk-worker.opt-level=0` override to reduce compile time.
Final `cargo fmt --all -- --check` and `git diff --check` passed. No full app E2E or hosted CI
has been run.
