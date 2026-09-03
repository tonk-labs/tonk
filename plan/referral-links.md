# Referral links and traffic attribution

## Goal

Measure which public platforms and shared Tonk spaces bring people into the
product, without sending invite credentials, raw URLs, referrer domains, space
names, or space DIDs to PostHog.

## Contract

- `tonk_source=<platform>` is the canonical explicit source tag for product
  and invite links. Only a closed platform vocabulary reaches PostHog.
- `tonk_space=<token>` is a locally hashed space attribution token. Tonk adds
  it to every minted invite before shortening, so `/join` visits can be joined
  to the originating space's create/share/conversion events.
- Tonk-generated space and invite links carry `tonk_channel=reshare`.
- When there is no explicit source tag, classify an allowlisted public
  platform from `utm_source` or `document.referrer`. Unknown external sources
  become `other`; absent/same-origin sources become `direct`. Never capture the
  raw input.
- A visible `/space/{key}` route wins over a `tonk_space` query value. On a
  `/join` route, accept only a 16-character hexadecimal token.

## Delivery

- [x] Extend the typed launch-attribution schema and focused tests.
- [x] Add referral metadata to browser, HTTP, and CLI invite minting.
- [x] Turn copied `/space/...` product URLs into organic referral links.
- [x] Preserve explicit campaign/source tags across short-link redirects.
- [x] Update the telemetry inventory and PostHog dashboard runbook.
- [x] Run focused native/Wasm checks, formatting, and final diff review.

## Verification

- `cargo test -p tonk-analytics --lib` — 23 passed.
- `cargo test -p tonk-access-service --features integration-tests shortcut::tests`
  — 2 passed.
- `cargo test -p tonk-access-service --features integration-tests --test shortcut it_stores_and_redirects -- --test-threads=1`
  — 1 passed with loopback access.
- `cargo test -p tonk-cli --test site it_honors_a_custom_base_url` — 1
  passed.
- Focused stock-runner Chrome/Wasm tests passed for the launch schema (9),
  typed PostHog payload (1), copied product link (1), reactive worker invite
  URL (1), and HTTP worker invite mint (1).
- `cargo check -p tonk-analytics -p tonk-access-service -p tonk-cli -p tonk-worker -p tonk-workspace`,
  `cargo fmt --all -- --check`, and `git diff --check` passed.

## Trust boundary

PostHog is directional product analytics, not a payment ledger. Its browser
ingestion key and referral query parameters are public and traffic can be
replayed or forged. Compensation decisions may use these reports to identify
and investigate meaningful traffic, but automated payouts require a separate
server-side, signed attribution and fraud/deduplication design.
