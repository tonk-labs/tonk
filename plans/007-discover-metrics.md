# Discover usage metrics

Measure successful template creation and subsequent navigation into the resulting
space. Correlate events using existing hashed space IDs; hash the catalog reference
as a stable template ID. No catalog URLs, names, or space contents leave the browser.

- [x] Move creation notification after seed application and carry template reference.
- [x] Capture template attribution and deduplicated space entries.
- [x] Test attribution, navigation boundaries, and legacy worker messages.
- [x] Document popularity and later-day retention definitions and limitations.

Scope: new creations only. Existing spaces cannot be retrospectively attributed.
Space entries measure navigation, not rendered readiness, edits, or dwell time.

Validation:
- Native `tonk-analytics` and `tonk-worker-api`: 64 tests passed. The first run's
  local HTTP bind was blocked by the sandbox; the authorized retry passed.
- Wasm `cargo check -p tonk-ui -p tonk-worker`: passed with existing dead-code warnings.
- Formatting and diff whitespace checks passed.
- Browser capture test passed (1 test):
  `cargo test -p tonk-analytics -p tonk-identity --lib --target wasm32-unknown-unknown --locked typed_launch_capture_registers_only_hashed_reviewed_properties`.
  The standalone analytics build lacked getrandom's wasm_js feature; including
  tonk-identity supplies the existing workspace feature without dependency edits.
- Full Discover UI journey and production ingestion remain unverified.

## Pre-deployment dashboard

- [x] Version template labels and five production report queries.
- [x] Validate live empty-data queries and a synthetic retention fixture.
- [x] Publish a dedicated Discover dashboard and verify saved definitions/tiles.
- [x] Record dashboard URL and ingestion checks still pending deployment.

Published dashboard: https://eu.posthog.com/project/70116/dashboard/989095

- All five live SQL queries passed before publication (empty before deployment).
- All five synthetic SQL fixture checks passed without ingesting events. The
  fixture assertion was adjusted for the CLI's `(null)` display format.
- Dashboard 989095 has five full-width ordered tiles. API read-back verified
  every saved name, description, query, dashboard description, and tile order.
- Forced dashboard refresh returned four empty tables and coverage
  `[0, 0, 0, 0, null]`, with no query warnings or errors.
- Public catalog labels cover kanoodel, little writer, Nightsky, and Welcome to
  Tonk. Unknown hashes remain visible until the versioned mapping is updated.
- Python syntax and git whitespace checks passed. Prior Rust changes preserved.
- Helium was selected at the user's request, but its automation call stalled
  for over 20 minutes. The rendered dashboard was not visually verified.
- Production delivery and retention cohort data still require deployment/time.

## PR preparation

- Reviewed the complete diff and retained only the requested scope.
- Full `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` passed.
- Fresh Rust formatting, Python syntax/dashboard structure, and diff checks passed.
- Native/browser/SQL fixture results above apply to unchanged implementation.
- Instrumentation and dashboard tooling are split into two reviewable commits.
- Hosted CI remains separate from local validation.
