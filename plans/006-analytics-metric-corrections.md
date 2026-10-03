# Analytics metric corrections

Scope: distinguish traffic, profile engagement, registration receipts, activation
receipts, and authoritative hosted account totals. Preserve memory-only PostHog
persistence and the existing no-account-identifiers telemetry contract.

- [x] Read live stable, Account health, and Launch onboarding definitions.
- [x] Reproduce 31 Aug–6 Sep 2026 spike: 1,306 pageviews, 891 raw IDs,
      432 merged visitors, 36 profile IDs directly on pageviews.
- [x] Add bounded identity retries and capture-time identity classification.
- [x] Test retry success, exhaustion, late results, opt-out, and profile labeling.
- [x] Version corrected dashboard definitions; validate queries before publishing.
- [x] Publish and read back visitor labels, active-profile metrics, lifecycle
      counts, acquisition sources, and identity diagnostics.
- [x] Add and verify aggregate-only CONTROL account snapshot query.
- [x] Run focused checks and record deployment boundaries.

Counts of profile IDs are not account or human counts. Earlier anonymous events
may be merged by PostHog; capture-time identity classification does not change
retroactively. Active profiles require an observed stable profile identity plus
a successful user-triggered product operation. Client activation receipts are
attempts, not unique account totals. D1 current rows exclude deleted accounts.

## Verification

- `node --test rust/tonk-ui/tests/analytics-identity.test.mjs`: 5 passed.
- `cargo check -p tonk-ui -p tonk-analytics --target wasm32-unknown-unknown`:
  passed; existing worker dead-code warnings remain.
- `cargo fmt --all -- --check` and `git diff --check`: passed.
- Account SQL: passed against all CONTROL migrations in SQLite, including empty,
  Registered/Active/Suspended, verification, and deletion cases.
- Production direct-query snapshot (2026-09-25): 36 current accounts, 33
  Active/verified, 3 awaiting activation; rows_written=0, changed_db=false.
- PostHog spike follow-up: 432 pageview persons; 75 had a profile-identified
  event that week, 357 did not. This is not a human-count estimate.
- Initial Wrangler file execution failed authentication on its import endpoint;
  the direct SELECT succeeded. PostHog read-back omitted null query properties;
  comparison now ignores only null fields and checks every substantive value.

Browser deployment, production ingestion of metrics_version=2, and browser/device
end-to-end flows are not verified by these checks. At the initial audit checkpoint, no commit or deployment had been
requested or performed. Dashboard publication is separate from browser release.

All 12 insight changes and both dashboard descriptions/attachments were
read back successfully. Saved source-platform and active-profile queries
execute; active-profile tables currently have no matching rows. A grouped live query
confirmed production product_event rows were exclusively automatic in the
last 30 days; successful user-triggered rows exist in staging and CLI. This
is a production coverage limitation, not evidence of zero usage.

## Historical reconstruction follow-up

- [x] Add 90-day daily/weekly legacy activity line charts from identified
  commit/create/join/share events; deduplicate profiles across event types.
- [x] Validate SQL, publish, recompute, and verify chart settings/membership.
- Scope: aggregate existing history only; no synthetic event backfill.

Historical charts: daily hrMHaOwp, weekly zvqStwlu. SQL and saved definitions
verified. The week starting 2026-08-31 has 30 identified profiles with recorded
legacy activity, versus 432 pageview persons under the visitor definition.

## Dashboard cleanup

- [x] Consolidate the main dashboard to DAU/WAU, Signups/Activations,
  Traffic sources/Panic rate in three two-column rows.
- [x] Keep one consistent commit/create/join/share definition across history
  and future dates. Save stricter and diagnostic insights off the main dashboard.
- [x] Verify six tiles, short labels, line charts, and explicit grid coordinates.

Cleanup verified through PostHog API: six tiles, ordered DAU, WAU, Signups,
Activations, Traffic sources, Panic rate; each 6 columns by 5 grid rows at
positions (0,0), (6,0), (0,5), (6,5), (0,10), (6,10). All nine detached
tiles retain their saved insights. Python syntax and diff checks passed.
Browser visual inspection was not completed: native UI automation twice
refused actions because the user was actively changing the browser.

## Retention and launch-funnel correction

- [x] Audit live launch queries: no environment filter; session-ID aggregation;
  one-day windows; no mature-window exclusion; high-cardinality route breakdown.
- [x] Add weekly first-activity retention with W1/W4 counts, percentages, and
  NULL for incomplete observation; add returning-profile trend.
- [x] Replace session funnels with production-only seven-day mature signup
  cohorts, ordered create/join/share milestones and sample sizes.
- [x] Keep channel operation/profile counts separate from conversion rates.
- [x] Publish and verify both layouts, saved definitions, and computed results.

No event-history rewriting or new account identifiers. Retention cohorts are
first observed profile activity, not first-ever use or signup dates. Historical
source coverage and automatic commits remain limitations.

Validation: all six new SQL queries execute. Retention correctly leaves the
2026-09-14 cohort immature and returns 8/30 (26.7%) for 2026-08-31 and 4/38
(10.5%) for 2026-09-07. Mature signup funnel: 15 -> 7 -> 6 profiles; creation
only: 15 -> 6 -> 6. UNION sorting initially failed PostHog field resolution;
an outer SELECT fixed it before publication. Python syntax, unique spec
names/dashboard IDs, description limits, and diff checks passed.

Published retention RxdpjBo8 and returning profiles LkJnOo2L. Main dashboard
has eight verified tiles; launch has six. Saved SQL results were recomputed;
cohort returned counts/rates and first-space funnel monotonicity were checked.
The builder query returned one 504 during concurrent checks; the unchanged
query succeeded sequentially with 15 -> 6 -> 6. Both production-only space
trends recomputed (two series and one series), and the dashboard-level
production filter was read back. Visual browser inspection remains unverified.

## Retention and onboarding visualization

- [x] Replace retention table with a cohort heatmap: weeks 0–4, row sample
  sizes, percentage values, and blank incomplete observation periods.
- [x] Replace onboarding tables with stage and channel bar charts; retain
  production-only mature seven-day cohorts and ordered milestone queries.
- [x] Shorten descriptions and use readable channel labels.
- [x] Execute all five SQL queries and verify saved definitions and layouts.
- [x] Inspect both dashboards visually in the live browser after publication.

Fresh rendered evidence: retention shows 26.7% and 10.5% week-1 rates with
blank immature cells; stage bars show 15 -> 7 -> 6 and 15 -> 6 -> 6,
channel bars show 11/4 signups and 86 joining profiles. No Rust changes
in this presentation increment. Python/spec syntax and diff checks passed.

## PR preparation

The signup receipt now awaits bounded identity lookup inside the background
task, so successful lookup precedes capture without blocking the completed
account ceremony. Exhaustion can still produce an unresolved receipt, and
closing the page can prevent delivery; client capture remains best-effort.

Fresh checks after this change: five Node identity tests passed; Wasm cargo
check passed with existing worker dead-code warnings; cargo fmt, Python AST,
dashboard JSON parsing, and git diff checks passed. Browser/device signup
flows and release ingestion are not verified. Live dashboard rendering was
verified in the preceding visualization checkpoint.
