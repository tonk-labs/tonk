# Launch analytics runbook

Launch onboarding is measured as PostHog events within one browser session.
All insights should filter `environment` to the intended deployment before
comparing channels.

The saved [Launch onboarding dashboard](https://eu.posthog.com/project/70116/dashboard/929973)
contains the six funnel and trend insights below. They remain empty until a
build carrying this event contract is deployed and receives traffic.

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

When adding a channel to an invite URL, preserve its existing query and
fragment and add the parameter through the browser `URL` API. Invite query and
fragment values are credentials and must not be copied into reports.

## Saved insights

The launch dashboard uses these definitions:

1. **Onboarding funnel**, ordered within one PostHog session:
   - `visit`;
   - `account_created`;
   - `space_conversion`;
   - `space_shared`.
   Break down by `channel`, then `entry_type`.
2. **Space acquisition funnel**: the same first two steps filtered to
   `entry_type=shared_space`, followed by `space_conversion` where
   `conversion=joined`.
3. **Builder activation funnel**: `account_created`,
   `space_conversion` where `conversion=created`, then `space_shared`.
4. **Signup conversion by entry route**: `visit` followed by
   `account_created`, broken down by `entry_route`. Filter to the reviewed
   How to Tonk `entry_space_id` for its subpage report.
5. **Space conversion mix**: `space_conversion` trends broken down by
   `conversion` and `channel`.
6. **Shared-space reach**: unique `space_id` on `space_shared`, broken down by
   `channel`.

Every event has PostHog's native timestamp. The existing hashed profile
`distinct_id` is refreshed after account creation and supplies anonymous user
correlation; no UCAN, delegation, account DID, or profile DID is sent.
`space_id` and dynamic `entry_route` segments are short local hashes.

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
