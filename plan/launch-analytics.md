# Launch analytics

## Goal

Measure the onboarding funnel from a first Tonk visit through account creation,
space creation or joining, and successful sharing, split by warm outreach,
organic re-share, and clearnet discovery.

## Privacy and attribution decisions

- Keep PostHog cookieless and session-scoped. Do not send invite URLs, query
  strings, referrers, DIDs, UCANs, or delegation bytes.
- Accept `tonk_channel=outreach|reshare|clearnet` as the canonical campaign
  parameter. Also classify equivalent reviewed UTM values.
- When no explicit campaign is present, infer shared-space entry as
  `organic_reshare` and the Tonk network shell as `clearnet_discovery`.
- Hash the landing route and stable space key locally. The existing hashed
  profile `distinct_id` is the user correlation key; account credentials are
  not an analytics identifier.
- Use PostHog's event timestamp and session ID rather than duplicating either
  as custom properties.

## Event contract

- `visit`: `channel`, `attribution_source`, `entry_type`, `entry_route`, and
  optional `entry_space_id`.
- `account_created`: emitted after account creation and configured enrollment
  succeed, after a best-effort refresh of the hashed profile identity. The
  existing `account_event` remains the operational journey and may then finish
  `blocked/awaiting_activation`, which is the normal email confirmation state.
  Landing properties are registered as session super properties before either
  event fires.
- `space_conversion`: `conversion=created|joined` and `space_id`, emitted only
  after the worker has completed the operation.
- `space_shared`: `space_id`, emitted only after an invite is durably minted.

## Checkpoints

- [x] Add and test the closed attribution and launch-event schema.
- [x] Register landing attribution and capture `visit` before other web events.
- [x] Relay worker-confirmed create, join, and share success to the web
      analytics client.
- [x] Document the PostHog funnel and How to Tonk subpage breakdown.
- [x] Create and verify the saved PostHog dashboard and six insights.
- [x] Run focused native and Wasm checks, formatting, and review the final diff.

## Dashboard follow-up

The saved PostHog dashboard exists and will populate after this contract is
deployed. An in-space Tonk report tab requires a server-side proxy holding a
PostHog personal API key; the public project ingestion key must not be used for
reads.
