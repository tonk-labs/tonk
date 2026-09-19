# Doctor route

Add an exact `/doctor` (and `/doctor/`) top-document diagnostic route that runs
without UI Wasm or service-worker readiness. Reuse existing read APIs, project
identity responses onto public diagnostic fields, and report independent probe
errors with bounded waits. Show browser/build/storage, worker health, account,
profile/operator, roster, and spaces. Explicit controls refresh/copy the report,
check for a worker update, and unregister only this page's registration without
deleting caches or local data. Skip automatic boot recovery on this route.

Validate probe failures, redaction, unregister scope, and boot isolation with
Node tests; run the existing UI JavaScript suite and Rust formatting. Inspect
layout and interactions in isolated Chrome with fixture APIs. Real Wasm and
hosted/browser lifecycle integration remain separate evidence.

## Completed validation

Implemented the static diagnostic module, exact route boot bypass, and access
through a failed worker's shell fallback. Added a Doctor link to the worker
failure page and documented the route in the UI README.

- 121 UI JavaScript tests passed, including bounded/error probes, secret-field
  projection, exact route isolation, scoped worker actions, and cached Doctor
  navigation/module availability after Rust initialization fails.
- `cargo check -p tonk-ui --target wasm32-unknown-unknown` passed (two existing
  tonk-worker dead-code warnings); Rust formatting and diff whitespace passed.
- Isolated Chrome rendered the source shell with no controller and with synthetic
  API responses. Copy succeeded, excluded secret markers did not render, update
  without a registration reported that fact. At the browser's 500px minimum
  resized viewport there was no horizontal overflow and buttons were 43px high.
- Preview listener and Chrome startup required host access after sandbox failures.
- Full Trunk build, live account APIs, actual installed-worker unregister/update
  lifecycle, Safari, and hosted deployment were not exercised.

## Agent debug bundle follow-up

Replaced Copy report with Copy debug bundle for agent. The bundle contains the
snapshot, probe sources/errors/timings, optional issue description, and the latest
200 worker log entries (timestamps and severity). Worker logs have an expandable
preview. Identity projection still excludes authority fields; best-effort
filtering covers known credential fields, authorization/token patterns, and URL
parameters in logs and bundle content. Coverage explicitly excludes prior page
console history and server logs, and notes worker restart loss. Copy operates
synchronously from the click on the already captured snapshot to retain browser
clipboard activation.

Validation: 124 UI JavaScript tests passed, including bundle contents, credential
filtering, and missing logs. Isolated Chrome with a synthetic worker log verified
copying issue context, the log, and API failures together without the synthetic
credential. Actual worker/server incident capture and Safari remain unverified.

## Layout and copy feedback

Scoped Doctor headings override the global inline boxed-heading treatment.
Grouped bundle and worker controls into separate panels, made diagnostics a
responsive two-column grid with bounded scrollable output, and tightened spacing
and explanatory copy. Copy success changes only the button text to Copied for
two seconds at a stable width. Clipboard failures appear beside that button;
worker operation feedback remains in the worker panel.

Validation: eight focused Doctor tests and diff whitespace checks passed.
Isolated Chrome loaded the actual global stylesheet: headings were block-level
with transparent backgrounds, copy success/reset both measured 272px, and the
worker notice stayed empty. At a 500px viewport diagnostics used one column
without horizontal overflow. A simulated clipboard rejection appeared only
beside the copy button. No worker/API behavior changed in this increment.

## Consistent tile spacing

Changed the control and diagnostic grids from start alignment to stretch, so
both tiles fill their shared row height. Isolated Chrome measured matching
heights for every tile pair and exactly 16px vertical gaps in both columns,
including unequal diagnostic content. The control panels also match heights.
`git diff --check` passed. This increment changes only grid alignment.
