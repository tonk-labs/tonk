# Launch analytics runbook

Launch onboarding uses production-only profile activity. Signup cohorts have
a seven-day follow-up window across page loads. Client events measure observed
profile activity, not authoritative account totals.

The saved [Launch onboarding dashboard](https://eu.posthog.com/project/70116/dashboard/929973)
contains the six production-only insights below. Definitions are versioned in
`docs/analytics/metrics-dashboard.json`.

## Acquisition channels

Use these canonical campaign parameters:

| Channel | Parameter |
|---|---|
| Warm outreach | `tonk_channel=outreach` |
| Organic re-share | `tonk_channel=reshare` |
| Clearnet discovery | `tonk_channel=clearnet` |

`tonk_channel` takes precedence over reviewed `utm_source`, `utm_medium`, and
`utm_campaign` values. Untagged space/join entries fall back to
`organic_reshare`; other untagged Tonk entries fall back to
`clearnet_discovery`. `attribution_source` distinguishes an explicit URL
parameter, UTM, external referrer, and route-based inference.

Add `tonk_source=<platform>` when the publishing platform is known. The
canonical values are `email`, `search`, `x`, `linkedin`, `instagram`,
`facebook`, `reddit`, `discord`, `slack`, `telegram`, `whatsapp`, `bluesky`,
`mastodon`, `github`, `product_hunt`, and `hacker_news`. Tonk also recognizes
equivalent `utm_source` values and those public hosts in `document.referrer`.
Unknown external sources are `other`; absent or same-origin sources are
`direct`. `source_detection` says whether the result came from
`url_parameter`, `utm`, `referrer`, or `direct`.

When adding a channel to an invite URL, preserve its existing query and
fragment and add the parameter through the browser `URL` API. This also works
on a short `@/...` invite: its redirect forwards only the public campaign and
source parameters. Invite query and fragment values are credentials and must
not be copied into reports.

Tonk-generated space and invite links already carry
`tonk_channel=reshare`. They also carry a `tonk_space` token derived from the
space routing key. The token matches PostHog's `space_id`/`entry_space_id` but
does not disclose the space DID or name.

## Dashboard insights

The saved launch dashboard contains:

1. **Signup → first space**: identified signup profiles, then successful creation
   or joining, then sharing. Each stage must follow the previous stage and occur
   within seven days of signup. Only signup cohorts with seven full days of
   observation enter the denominator. Counts and percentages share that cohort.
2. **Signup → create & share**: the same cohort/window, restricted to creating a
   space before sharing.
3. **Signup profiles by channel**: distinct identified profiles with a completed
   signup in the last 90 days. Creation precedes email activation.
4. **Joined profiles by channel**: distinct identified profiles with successful
   joins, including people who already had an account. This is not a signup rate.
5. **Spaces created / joined**: weekly successful operation counts.
6. **Spaces shared**: weekly distinct hashed space IDs with an invite minted.

All six queries explicitly filter production. The dashboard also carries that
filter. The two conversion charts deduplicate by the stable profile hash and
follow activity across page loads, rather than requiring the same in-memory
PostHog session. Unresolved anonymous visits are not a conversion denominator.
These are profile journeys; sharing need not refer to the same space as the
earlier conversion. No account or person count is inferred from these profile counts. Cohorts and
conversion rates omit unidentified or unobserved client events and cannot
reconstruct cross-device account journeys.

Raw arrival counts and reviewed source-platform breakdowns remain available
separately on the main dashboard. Avoid treating page opens as people, and do
not splice them into the signup cohort denominator. High-cardinality hashed
routes are intentionally absent from the overview; drill into a reviewed route
when answering a specific acquisition question.

See [population and retention definitions](analytics/metrics.md) for the exact
limits and authoritative account query. No UCAN, delegation, account DID, or
profile DID is sent. Space IDs and dynamic entry-route segments are hashes.

## How to Tonk

Find the hashed `entry_space_id` on a controlled How to Tonk visit, filter the
onboarding funnel to that value, and break it down by `entry_route`. Dynamic
route segments are independently hashed, which distinguishes demos,
introduction text, wiki pages, and future subpages without disclosing their
names. Keep a private dashboard annotation mapping the reviewed route hashes to
human labels; do not put the raw DID or invite URL in repository configuration.

## Tonk-space dashboard

An internal Tonk tab can render these saved insights through the PostHog API,
but it must call a Tonk-controlled server-side proxy. The PostHog project
ingestion key baked into the web app is public and write-only; a personal API
key capable of reading insights must never be shipped to a space or browser.
The proxy should expose only the saved dashboard's aggregate results, not an
arbitrary PostHog query surface.

For population counts, use the [metric definitions](analytics/metrics.md).
`account_created` precedes email activation; funnel persons are not unique
accounts. Signup receipts, activation attempts, and CONTROL account totals
remain separate.
