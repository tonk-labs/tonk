# Visitors, profiles, and accounts

These metrics count different things. None of the client metrics establish a
number of humans. PostHog identifies `tonk:<sha256(profile DID)>`; a guest has a
profile, multiple devices can have different profiles for the same account,
and signing in does not turn that identifier into an account identifier.

| Metric | Definition | Limit |
| --- | --- | --- |
| DAU / WAU | Distinct profile hashes with a commit, space create/join, or share per day/week | Includes guests and automatic commits; not people/accounts |
| Visitors | PostHog unique persons with `$pageview` | Includes unresolved anonymous page opens; identification can later merge them |
| Successful-action profiles (diagnostic) | Distinct complete profile hashes on `product_event`, schema 1, phase `finished`, result `success`, trigger `user` | Includes guests; observed successful operations only, not accounts |
| Completed signup receipts | `account_created` event count | Creation/enrollment before email activation; client delivery is best-effort |
| Successful activation attempts | Distinct `attempt_id` on `account_event`, action `activate_account`, phase `finished`, result `success` | Repeated attempts are not deduplicated by account |
| Account operation attempts | Distinct `attempt_id` with phase `started`, grouped by environment/action | Do not add checkpoints or terminal events |
| Current accounts | `COUNT(*)` of CONTROL `customer` rows | Current hosted records, not historical signups; deleted accounts are absent |
| Active-status accounts | CONTROL rows with `status = 'Active'` | Service eligibility, not recent usage |
| Verified current accounts | CONTROL rows with `verified_at > 0` | May include suspended accounts; not weekly activity |

The separate successful-action diagnostic tables use explicit production filters
and a 30-day window.
They require a profile ID on the successful action itself, rather than counting
an initial anonymous event as engaged. Coverage requires a deployed release emitting the product-event contract;
historical missing events cannot be reconstructed and missing rows are not zero usage.
No retroactive correction or human-count estimate is made.

## Identity resolution

PostHog remains memory-only. `metrics_version=2` and `identity_state=unresolved`
are registered at startup. After the SDK accepts the hashed profile ID,
`identity_state=profile` applies to subsequent events. These are capture-time
properties: they do not change retrospectively when PostHog merges persons.
Older events have neither property; diagnostic queries can distinguish their
profile and anonymous IDs directly.

The UI retries readiness plus identity lookup at most three times, with a
five-second bound on each attempt and 500ms/1000ms backoffs. Late results do not
identify the page. Startup and completed signup UI never wait for analytics.
The signup receipt runs in a background task after this bounded lookup; it
can still be unresolved if all attempts fail, or lost if the page closes first.
Arrival and diagnostic events remain available when identity fails; do not
count those unresolved events as active profiles. No persistent analytics ID,
account identifier, raw error, or additional browser storage is introduced.

## Dashboard maintenance

`metrics-dashboard.json` records explicit existing insight IDs and new insights.
It corrects the stable dashboard's visitor labels and referring-source chart,
adds active profiles and lifecycle receipts, and deduplicates Account health
attempts. The legacy navigation and commit-return charts are labeled according
to their actual semantics. Production D1 aggregates remain separate from
PostHog and are never inferred from its persons table.

```sh
python3 scripts/posthog-metrics.py validate
python3 scripts/posthog-metrics.py publish
```

The publisher validates SQL before writing, records IDs for new insights, and
verifies saved queries and attachments. It removes only explicitly listed tiles
from the main dashboard, preserving the underlying saved insights and other
dashboard memberships. It uses the authenticated repository `posthog-cli`. Browser source
changes require deployment separately; publishing dashboards does not deploy
Rust/Wasm or prove production ingestion of the new properties.

## Authoritative account snapshot

Run the aggregate-only query against the intended service environment:

```sh
wrangler d1 execute tonk-access-control --remote --command="$(cat docs/analytics/account-totals.sql)" --json
```

For staging, use `tonk-access-control-staging` with `--env staging`. The query
returns counts only, never identifiers or email addresses. No database schema
change, server endpoint, or PostHog export is required.

## Spike investigation

A read-only production query for `[2026-08-31, 2026-09-07)` reproduced 1,306
pageviews, 891 raw distinct IDs, and 432 PostHog persons. Only 36 profile IDs
appeared directly on pageviews. This diagnoses mixed traffic identities, not a
corrected account total: anonymous pageviews can be merged through other events,
and people can have more than one profile.

The follow-up query grouped all events by PostHog person: of those 432
pageview visitors, 75 had a profile-identified event during the week and 357
did not. This is evidence of unresolved traffic, not proof of 357 extra people
or a causal attribution to a particular browser failure.

A production CONTROL snapshot on 2026-09-25 returned 36 current accounts,
33 Active/verified and 3 Registered/awaiting activation, with zero rows written.
These present-day account totals are not the historical weekly population.

At publication, the last 30 days of production product_event data contained
automatic startup events but no user-triggered events. Successful user-triggered
events existed in staging and CLI. The production active-profile tables are
therefore empty; this does not establish zero usage. Verify user-action
instrumentation in the deployed production release before relying on them.

## Historical activity estimate

The DAU and WAU line charts reconstruct the last 90 days
from already captured `commit`, `space_conversion`, and `space_shared` events.
They count each complete profile hash once per day/week across all three event
families, with Monday-based calendar weeks. Anonymous IDs and plain pageviews
are excluded. No events are synthesized or written back into event history.

This is an estimate of observed profile activity: commits may be automatic,
guests have profiles, and multiple profiles can belong to one person/account.
It cannot reproduce the new successful-user-action definition. Missing periods
mean no recorded qualifying events, not proof nobody used the product. The
current day/week is partial. The series continues using the same definition;
it is not silently spliced into the stricter user-action series.

Published charts: [DAU](https://eu.posthog.com/project/70116/insights/hrMHaOwp)
and [WAU](https://eu.posthog.com/project/70116/insights/zvqStwlu).
For the week starting 2026-08-31, this definition yields 30 profiles with
recorded activity, compared with 432 pageview visitor identities.

## Main dashboard

The production dashboard now has one DAU and one WAU line chart, followed by
Signups/Activations and Traffic sources/Panic rate in a two-column grid.
DAU/WAU use the same commit/create/join/share definition for all dates, including
new data. There is no legacy/current cutover. They count observed profiles;
automatic commits and guest activity remain included.

The stricter successful-user-action insights and old visitor/diagnostic charts
remain saved, but are not on the main dashboard. Long interpretation notes live
here instead of repeating across tiles. The publisher reconciles only explicitly
listed tile removals, preserving saved insights and other dashboard memberships.

## Weekly retention

Weekly retention uses the same `commit`, `space_conversion`, and `space_shared`
activity as DAU/WAU, with complete profile hashes and production only. Cohorts
are the first calendar week with recorded activity for each profile across all
available history, not the week of account creation. A return is qualifying
activity in each of weeks 1–4 after that first week. The heatmap shows
percentages, with cohort sizes in row labels and week 0 as the 100% baseline. Future and partially observed return weeks are
NULL, never 0%; the current first-activity cohort is omitted.

Returning profiles is the number active in a calendar week whose first observed
activity was in an earlier week. It includes returns after a gap. It is not a
retention percentage. Both measures include guest profiles and automatic
commits and cannot establish human/account retention or recover activity from
before instrumentation began.

The main layout is DAU/WAU, Weekly retention/Returning profiles,
Signups/Activations, then Traffic sources/Panic rate. The launch dashboard
filters production, uses mature seven-day signup cohorts, and drops the
session-based visit denominator and high-cardinality route overview.

On 2026-09-25, the mature identified production signup cohort contained 15
profiles: 7 created or joined a space within seven days (46.7%), and 6 then
shared within the same window (40%). Restricting the middle step to creation
gives 6 creators and 6 sharers. These are observed cohort counts, not totals
of registered accounts. The original session/mixed-environment funnels are
not comparable to these corrected figures.

First-activity retention at that snapshot: the 2026-08-31 cohort had 8/30
profiles return in week 1 (26.7%); the 2026-09-07 cohort had 4/38 (10.5%).
Week-1 retention for the 2026-09-14 cohort remained NULL because its return
week was still in progress.

The onboarding stage charts preserve the ordered seven-day SQL cohorts. Bar
heights show profile counts; category labels show the percentage of signups.
Channel comparisons also use bars, with human-readable source labels. These
visual changes do not restore PostHog session-based funnel aggregation.
