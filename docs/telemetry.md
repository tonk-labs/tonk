# Telemetry

Tonk collects anonymous usage analytics via PostHog to understand
which features get used and where errors occur. This page is the
complete inventory of what is sent and how to turn it off.

## What is never sent

Notation documents, entity names or values, attribute values, email addresses,
account or device IDs, credential IDs, passkey labels/providers, callback or
remote URLs, route parameters, query strings, UCANs/delegations, invocation or
receipt bytes, HTTP bodies, local paths, and raw error messages. The existing
profile identity is a SHA-256 hash; account events add no identifier derived
from account data.

## Identity

The distinct id is `tonk:<sha256(profile DID)>` — stable per profile,
not reversible. `tonk identity --reset` rotates it. Commands run
before a profile exists report as `tonk:anonymous`.

## Events

### CLI

| Event | Properties |
|---|---|
| `cli_command_run` | `command`, `subcommand`, `success`, `exit` (success / parse-error / analyze-error / commit-error / io-error), `duration_ms`, `version`, `os`, `arch`, `environment` (constant `"cli"`), ``$lib`` (constant `"tonk-analytics"`); eval adds `source` (inline / file / stdin), `format`, `dry_run`, `quiet` |
| `account_event` | The closed account journey schema below. Account commands emit this lifecycle in addition to one `cli_command_run`; never add the two event types together when counting account attempts. |

### Web app

| Event | Properties |
|---|---|
| `app_loaded` | `version` |
| `$pageview` | `route` — the path with every non-literal segment replaced by a hash (e.g. `/space/1a2b3c4d5e6f7a8b/view/9c8d7e6f5a4b3c2d`); ``$current_url`` carries the same normalized value |
| `commit` | `branch` (`main` / `meta`; anything else reports as `other`) — fired on the worker's `tonk:local-commit` broadcast for every durable transact/evaluate commit; only the visible tab records it |
| `sheet_activated` | none |
| `panic` | `type` (constant `wasm_panic`), repository-relative `location`, and a fingerprint derived only from those static values; the panic message stays in the local console |
| `account_event` | The same closed account journey schema emitted by the CLI. |
| `visit` | `schema_version=2`, `channel` (`warm_outreach` / `organic_reshare` / `clearnet_discovery`), `attribution_source` (`url_parameter` / `utm` / `referrer` / `inferred`), `source_platform` (the closed list below), `source_detection` (`url_parameter` / `utm` / `referrer` / `direct`), `entry_type` (`tonk_network` / `shared_space`), normalized `entry_route`, and optional hashed `entry_space_id` |
| `account_created` | `schema_version`; fired after account creation, configured enrollment, and a best-effort refresh of the hashed profile identity, even though the account journey then waits for email activation |
| `space_conversion` | `schema_version`, `conversion` (`created` / `joined`), hashed `space_id` |
| `space_shared` | `schema_version`, hashed `space_id`; fired only after an invite is successfully minted |

`visit` is captured before the other web events and its reviewed attribution
properties are registered for the current in-memory PostHog session. That puts
the same `channel`, `attribution_source`, `entry_type`, `entry_route`, and
optional `entry_space_id`, plus `source_platform` and `source_detection`, on
subsequent account, space, and sharing events. PostHog supplies the event
timestamp and session ID. Tonk does not duplicate them as custom properties.

Campaign links use `tonk_channel=outreach`, `tonk_channel=reshare`, or
`tonk_channel=clearnet`. Equivalent reviewed UTM values are accepted. Without
an explicit campaign value, a shared-space or join route is classified as an
organic re-share and the Tonk network shell as clearnet discovery. Referrers
are reduced locally; their domains and paths are never captured.

Source-platform links use `tonk_source=email`, `search`, `x`, `linkedin`,
`instagram`, `facebook`, `reddit`, `discord`, `slack`, `telegram`, `whatsapp`,
`bluesky`, `mastodon`, `github`, `product_hunt`, or `hacker_news`. Equivalent
reviewed `utm_source` values are accepted. Without a source parameter, Tonk
classifies the same allowlist from the external document referrer. Everything
else becomes `other`; no external source becomes `direct`. `source_detection`
records which safe input won, never its raw value.

Every Tonk-generated space or invite referral link carries
`tonk_channel=reshare` and `tonk_space=<16 hex characters>`. The space token is
the same local SHA-256-derived identifier used by `space_conversion` and
`space_shared`, so a `/join` visit can be attributed without putting a space
DID or name in the invite query or event. A visible `/space/{key}` route is
hashed locally and wins over a query token. Short-link redirects forward only
public campaign/source tags added to the `@/...` URL; they do not accept
overrides for the stored invite capability or space token.

### Account journey schema

`account_event` has `schema_version=1` and only these properties:

- `journey`: `onboarding`, `login`, `activation`, `passkey`,
  `account_management`, `cli_handoff`, or `account_deletion`;
- `action`: a reviewed account operation from
  `open_registration`, `load_account`, `load_registration`, `check_email`,
  `create_account`, `login`, `add_passkey`, `change_display_name`,
  `resend_activation`, `load_devices`, `load_profiles`, `link_cli`,
  `switch_profile`, `sign_out`, `load_deletion_plan`, `delete_account`,
  `delete_space`, `revoke_device`, `finish_account_backup`,
  `activate_account`, `watch_activation`, `save_initial_display_name`,
  `copy_invite`, `finish_previous_action`, `settle_account`,
  `load_account_spaces`, `pull_account_space`, `open_account_deletion`,
  `open_space_deletion`, or `sync_account`;
- `phase`: `started`, `checkpoint`, or `finished`; `stage` is a reviewed
  lifecycle boundary: `input`, `email_lookup`, `local_preflight`,
  `passkey_create`, `passkey_assert`, `prf`, `worker_handoff`,
  `access_service`, `local_commit`, `remote_commit`, `activation_wait`,
  `callback_bind`, `browser_open`, `callback_wait`, `callback_delivery`,
  `delegation_validate`, `activation_stage`, `account_sync`,
  `content_discovery`, `custody_rotation`, `account_load`, or `complete`;
- `result` on terminal events: `success`, `degraded_success`, `cancelled`,
  `blocked`, `retryable_failure`, `terminal_failure`, or `unknown_commit`;
- `failure_kind` on non-success terminal events: `invalid_input`, `cancelled`,
  `timeout`, `credential_exists`, `passkey_unsupported`, `prf_unsupported`,
  `security_context`, `awaiting_activation`, `suspended`, `not_provisioned`,
  `access_denied`, `conflict`, `not_found`, `rate_limited`, `network`,
  `service_unavailable`, `invalid_response`, `local_state`, `callback`, or
  `unknown`;
- `degradation_kind` only on degraded successes: `browser_open`,
  `account_sync`, `content_discovery`, `custody_rotation`, or
  `space_rotation`;
- `surface` (`registration_dialog`, `settings`, `activation_page`,
  `custody_consent`, `hub`, `cli_callback`, or `native_cli`), `trigger`
  (`user`, `automatic`, or `recovery`), `account_state` (`none`, `onboarding`,
  `pending_activation`, `registered_unready`, `ready`, or `unknown`), and a random per-attempt
  `attempt_id` (not an account identifier and not a dashboard breakdown);
- terminal `duration_ms`, capped at ten minutes; optional status class (`4xx`
  or `5xx`) and an allowlisted service code (`root_required`,
  `credential_revoked`, `upstream_timeout`, `upstream_unavailable`,
  `account_state_unavailable`, `invalid`, `unauthorized`, `forbidden`,
  `unknown_customer`, `unknown_consumer`, `customer_active`,
  `customer_inactive`, `customer_suspended`, `address_taken`,
  `consumer_provided`, `internal`, or `unknown`); and
- transport context: `version`, `environment`, and the native library/OS/arch
  fields where applicable.

Cancellation, activation waits, and suspension are outcomes, not uncaught
exceptions. A response lost after a possible mutation is `unknown_commit`.
Automatic probes suppress repeated equal failures until a successful recovery.
The account capture entry point accepts no arbitrary JSON properties.

Every web event additionally carries an `environment` super property
derived from the hostname: `production` (tonk.network), `staging`
(staging.tonk.xyz), or `dev` (anything else). The marketing site's
own snippet registers `environment = "website"` in its repo, so all
surfaces share one filterable dimension within the PostHog project.

Autocapture and session recording are disabled. PostHog runs
cookieless (`persistence: "memory"`); no third-party script is loaded
(the bundle is self-hosted).

Browser exception capture and console-error capture are disabled. A project
setting cannot override that client configuration. Handled account failures
are represented only by `account_event`; exact diagnostics remain local.

The web shell also listens for a generic `tonk:analytics` DOM event
(`detail: { name, props }`) so future components can emit events
without new dependencies. Nothing dispatches it today; any future
dispatcher is responsible for keeping its payload content-free
(hashes and counts only), like every event above.
The bridge cannot emit `account_event`, `visit`, `account_created`,
`space_conversion`, or `space_shared`; those names are reserved for typed,
validated capture interfaces.

## Operational logs

The access Worker writes failed account requests to Cloudflare Workers Logs,
not PostHog. Each structured object contains only schema version,
`system=access_worker`, a closed operation/outcome/failure/site, status class,
retryability, and release version. It contains no request URL, query, subject,
profile identity, or diagnostic. Invocation logs are disabled for production,
staging, and preview, so request URLs and their query strings are not ingested;
the explicit application records contain no URL field. These short-lived
infrastructure records deliberately have no stable join key to PostHog.

## Turning it off

- CLI: `tonk telemetry off` (persisted), or `DO_NOT_TRACK=1`, or
  `TONK_TELEMETRY=0` per invocation. `tonk telemetry` shows the
  effective state. The persisted choice lives at ``<platform data dir>/tonk/telemetry.json`` (``TONK_TELEMETRY_STATE`` overrides the directory).
- Web: run `localStorage.setItem("tonk:telemetry", "off")` in the
  console and reload. (A settings toggle is a planned follow-up.)
- Builds without a `TONK_POSTHOG_KEY` baked in send nothing at all —
  this is the default for local builds, CI, and forks.

## For release builds

Set `TONK_POSTHOG_KEY` (a PostHog *project API key* — public by
design) and optionally `TONK_POSTHOG_HOST` (default
`https://eu.i.posthog.com`) in the environment of the release build.
`TONK_POSTHOG_KEY`/`TONK_POSTHOG_HOST` are also read at runtime, and
`TONK_POSTHOG_ENDPOINT` overrides the host for integration tests.

The nix packages (`tonk-cli`, `tonk-ui`, and therefore
`tonk-cloudflare-artifacts`) bake the key in from the `posthogKey`
binding in `flake.nix`, so `wrangler deploy` release builds carry it
automatically; dev-shell `cargo`/`trunk` builds stay key-less unless
the env var is exported.
