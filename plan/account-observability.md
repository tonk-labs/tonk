# Account and passkey observability implementation plan

**Goal:** Make onboarding, login, activation, account-management, and passkey
failure rates measurable and make the common failures diagnosable without
sending account content, authority material, or raw diagnostics to PostHog.

**Approach:** Put the canonical account event vocabulary, validation, and
privacy allowlist in `tonk-analytics`, then implement one deep attempt recorder
per execution interface: web in `tonk-ui` and native in `tonk-cli`. Both client
adapters emit the same `account_event`; the existing `cli_command_run` remains
the generic CLI-invocation event and is not a second account taxonomy. PostHog
holds privacy-safe journey analytics; unexpected browser exceptions use
PostHog Error Tracking only after payload redaction is proven; the access
Worker maps the same stable failure concepts into short-lived structured
Cloudflare logs rather than product analytics.

**Source audit:** `feat/log-account-errors` at
`c518c1ba49c06b9e637d9f29506ada2ff2e2ef26` on 2026-09-01, equal to
`origin/staging` when this plan was written.

**Top recommendation:** Ship the typed `account_event` journey stream and its
dashboard before enabling generic exception collection. Typed events provide
the denominator, stage, outcome, and recovery classification needed to
prioritize account work; generic console capture would collect more bytes but
less trustworthy information and would leak diagnostics that the UI
deliberately keeps out of user-facing and analytics payloads.

## Implementation status (2026-09-01)

Implemented in this worktree:

- the shared, privacy-validated `account_event` schema and native/web capture
  adapters;
- typed passkey, account API, UI recovery, CLI callback, and account-command
  failure evidence;
- web and CLI attempt recorders, including automatic-failure suppression,
  unknown-commit handling, and degraded CLI success;
- account UI, registration, activation, custody, destructive-action, and CLI
  instrumentation at their existing control seams;
- privacy-safe access Worker log records plus explicit production, staging,
  and preview Workers Logs configuration;
- telemetry inventory, investigation runbook, Storybook observability notes,
  schema/unit/integration coverage, and an in-browser PostHog capture fixture.

Still outstanding:

- complete the plan's exhaustive browser assertions for the second-device,
  failed-ceremony, revoke, and deletion journeys. The focused ordered signup
  telemetry journey passes in the real Chrome harness. The older full signup
  journey still times out waiting for its post-activation passkey-device label
  before reaching the added telemetry assertions;
- prove release source-map upload and outbound exception redaction before
  enabling generic PostHog exception collection. Generic exception capture
  intentionally remains disabled;
- deploy to staging, exercise controlled 4xx/5xx cases, and verify the saved
  Cloudflare queries and retention in the account-owning environment;
- enable the saved PostHog alerts after staging proves the stream and production
  rollout is approved. The Account health dashboard and its nine validated
  aggregate insights are saved and linked from `docs/account-observability.md`;
  the two alerts remain disabled so pre-rollout configuration cannot notify;

No production or staging deployment is implied by the local configuration
changes. Wrangler 4.128 accepts the checked-in staging observability and query
redaction fields and enters the custom build without configuration warnings.
The full cold Nix build remains incomplete because the local disk had about
6 GB free; an attempted Wrangler derivation failed while unpacking dependencies
with `No space left on device`, so this is configuration evidence rather than a
completed deployment dry run.

The architecture rule is **account totality at the schema seam, interface
locality at the recorder seam**. `tonk-analytics::account` owns what an account
event means; the web, CLI, and Worker adapters own when their runtime has enough
evidence to emit one. This keeps one dashboard vocabulary without forcing DOM,
polling, loopback-listener, process-flush, and Worker-log behavior into one
shallow cross-platform module.

## Constraints

- Keep PostHog cookieless, preserve the existing web/CLI opt-outs, and make all
  capture best-effort. Telemetry failure must never alter, delay, retry, or
  claim success for an account operation.
- Never send email addresses, raw DIDs, DID-derived event properties, account
  or device IDs, credential IDs, passkey labels/providers, callback URLs, route
  parameters, query strings, remote URLs, UCANs/delegations, invocation or
  receipt bytes, HTTP bodies, entity/space names, local paths, or raw error
  messages. Preserve the existing `tonk:<sha256(profile DID)>` PostHog distinct
  identity; do not add a second account identifier.
- Do not enable `capture_console_errors`. Account code intentionally writes
  exact diagnostics to `console.error`; PostHog documents console capture as a
  separate option, and enabling it would bypass the allowlisted event schema.
- Treat cancellation, timeout, validation refusal, awaiting activation, and a
  suspended account as outcomes, not product exceptions. Reserve PostHog Error
  Tracking for uncaught faults and Wasm panics.
- Classify from `CeremonyRefusal`, `CustodyDenial`, HTTP status, and structured
  error codes. Human-readable error matching may remain only as a compatibility
  fallback and must produce `unknown`, never a new analytics value.
- Every event property is a closed enum, boolean, bounded number, semantic
  version, or random per-attempt token. No caller may append arbitrary JSON.
- Web and CLI account adapters must emit the shared `account_event` schema.
  They may not introduce interface-local spellings for a shared journey,
  action, stage, result, or failure. Interface-specific mechanics belong in
  additional closed `stage`, `surface`, or degradation values in that schema.
- Retain one `cli_command_run` per CLI invocation for overall CLI adoption and
  exit analysis. An account command additionally emits `account_event`; account
  health insights use only `account_event`, so the two records are never added
  together as account attempts.
- An operation that might have committed before its response failed must use
  `unknown_commit`; it must not be counted as an ordinary failure or retried by
  telemetry code.
- A CLI command that established the account but failed a non-transactional
  follow-up such as hydration, content-endpoint discovery, or custody rotation
  must use `degraded_success` with a closed `degradation_kind`. It must not be
  flattened to either full success or failure merely because the process exits
  zero and prints a warning.
- Background probes and activation polling must not inflate failure counts.
  Emit the first failure in a streak, suppress identical repeats until a
  success/recovery event closes the streak, and then permit a later failure.
- Keep client analytics and server operational logs separate. PostHog honors
  the user's telemetry choice and supports aggregate product decisions;
  Cloudflare Workers Logs support short-lived infrastructure diagnosis. Do not
  create a stable identifier joining the two systems or log a profile identity
  server-side.
- Do not propagate `attempt_id` through a CLI callback URL in version 1. The
  browser ceremony and native command are separate attempts with explicit
  `surface` ownership. Exact browser-to-process correlation would change the
  handoff protocol and requires a separate privacy and threat-model review.
- Retain the current `autocapture: false`, session-recording, performance,
  dead-click, and heatmap settings. Any future change to those is a separate
  privacy review.
- Do not add source-map upload credentials to the repository or expose source
  maps as public deployed assets. A release job may upload them directly and
  then omit them from `tonk-ui` artifacts.
- Preserve the existing user-facing recovery contract in
  `rust/tonk-ui/src/user_error.rs` and all account lifecycle semantics. This
  plan observes behavior; it does not change authority, storage, passkey, or
  retry behavior.
- Update `docs/telemetry.md`, which declares itself the complete event
  inventory, in the same commit that adds or changes any captured property.

## Current and proposed seams

```text
Current
browser account/passkey error -> console + safe UI message -> no remote signal
CLI account command           -> cli_command_run with only coarse exit status
access Worker failure         -> inconsistent console diagnostics

Proposed
tonk_analytics::account
  -> canonical AccountEvent schema + privacy validation
     -> tonk_ui::account_observability
        -> web attempt/checkpoint lifecycle -> account_event -> PostHog
     -> tonk_cli::account_observability
        -> command/handoff lifecycle -> account_event -> PostHog
        -> generic invocation summary -> cli_command_run -> PostHog
     -> access-service observability adapter
        -> content-free failure projection -> Cloudflare Workers Logs
```

The shared module has one small interface: construct and validate a closed
`AccountEvent`, then hand it to a target-specific transport. It owns no clock,
randomness, polling state, callback listener, process lifetime, or retry logic.

Each client recorder exposes the same conceptual lifecycle while retaining
runtime-local mechanics:

```rust
// tonk-ui: browser randomness, automatic-failure streak suppression,
// page-local lifetime, and the PostHog web adapter.
let mut attempt = WebAccountAttempt::start(action, surface, trigger, account_state);
attempt.checkpoint(stage);
attempt.finish(AccountOutcome::success(stage));
// or
attempt.finish(AccountOutcome::from_problem(stage, &problem));

// tonk-cli: process-local randomness, command stages, zero-exit degradations,
// and queuing into the existing bounded native telemetry flush.
let mut attempt = CliAccountAttempt::start(action, account_state);
attempt.checkpoint(stage);
attempt.finish(AccountOutcome::degraded(stage, degradation_kind));
```

Both adapters hide duration, attempt IDs, terminal deduplication, PostHog
availability, and transport details. Only the web adapter owns automatic-load
failure-streak suppression; only the CLI adapter owns command finalization and
the 300 ms native flush. Serialization and the privacy allowlist remain
canonical in `tonk-analytics::account`.

## Event contract

Use one event name, `account_event`, with `schema_version = 1`. A single event
keeps trends and funnels composable while the closed properties keep cardinality
and privacy review tractable.

| Property | Required | Allowed values / rule |
| --- | --- | --- |
| `schema_version` | yes | integer `1` |
| `journey` | yes | `onboarding`, `login`, `activation`, `passkey`, `account_management`, `cli_handoff`, `account_deletion` |
| `action` | yes | closed snake-case account operation from the list below; semantically equal web and CLI operations use the same value |
| `phase` | yes | `started`, `checkpoint`, `finished` |
| `stage` | yes | `input`, `email_lookup`, `local_preflight`, `passkey_create`, `passkey_assert`, `prf`, `worker_handoff`, `access_service`, `local_commit`, `remote_commit`, `activation_wait`, `callback_bind`, `browser_open`, `callback_wait`, `callback_delivery`, `delegation_validate`, `activation_stage`, `account_sync`, `content_discovery`, `custody_rotation`, `account_load`, `complete` |
| `result` | terminal only | `success`, `degraded_success`, `cancelled`, `blocked`, `retryable_failure`, `terminal_failure`, `unknown_commit` |
| `failure_kind` | non-success terminals only | table below; absent on starts and successes |
| `degradation_kind` | degraded-success terminals only | `browser_open`, `account_sync`, `content_discovery`, `custody_rotation`, `space_rotation`; when more than one occurs, emit the earliest incomplete stage as the primary degradation and retain every exact diagnostic locally |
| `surface` | yes | `registration_dialog`, `settings`, `activation_page`, `custody_consent`, `hub`, `cli_callback`, `native_cli` |
| `trigger` | yes | `user`, `automatic`, `recovery` |
| `account_state` | yes | `none`, `onboarding`, `pending_activation`, `registered_unready`, `ready`, `unknown` |
| `attempt_id` | yes | random/opaque per attempt; never derived from account data and never used as a dashboard breakdown |
| `duration_ms` | terminal only | non-negative integer, capped at 10 minutes |
| `http_status_class` | optional | `4xx`, `5xx`; never the response body or URL |
| `service_code` | optional | allowlisted stable code already parsed from a response; unrecognized values become `unknown` |
| `version` | yes | `CARGO_PKG_VERSION`; web registers it as a super property and the native client adds it to every queued event |
| `environment` | yes after transport context | `production`, `staging`, `dev`, `cli`; web registers the deployment value and native account capture supplies `cli` |

The initial closed `action` values are `open_registration`, `load_account`,
`load_registration`, `check_email`, `create_account`, `login`, `add_passkey`,
`change_display_name`, `resend_activation`, `load_devices`, `load_profiles`,
`link_cli`, `switch_profile`, `sign_out`, `load_deletion_plan`,
`delete_account`, `delete_space`, `revoke_device`, `finish_account_backup`,
`activate_account`, `watch_activation`, `save_initial_display_name`,
`copy_invite`, `finish_previous_action`, `settle_account`,
`load_account_spaces`, `pull_account_space`, `open_account_deletion`,
`open_space_deletion`, and `sync_account`. CLI `status`, `login`, `logout`,
`delete`, `space-list`, `space-pull`, `space-delete`, `sync`, `devices`, and
`revoke` map respectively to `load_account`, `login`, `sign_out`,
`open_account_deletion`, `load_account_spaces`, `pull_account_space`,
`open_space_deletion`, `sync_account`, `load_devices`, and `revoke_device`.
This mapping is exhaustive and compile-tested; an interface does not create a
synonym merely to identify itself because `surface` already does that.

The failure vocabulary is deliberately smaller than the diagnostic vocabulary:

| `failure_kind` | Source evidence | Meaning in analysis |
| --- | --- | --- |
| `invalid_input` | local validation | User could not begin the action; break down separately from product faults. |
| `cancelled` | `CeremonyRefusal::NotAllowed` before the deadline | User dismissed the passkey prompt. |
| `timeout` | typed ceremony/worker timeout | Prompt or worker handoff did not finish in time. |
| `credential_exists` | `CeremonyRefusal::InvalidState` | Authenticator refused duplicate creation. |
| `passkey_unsupported` | `CeremonyRefusal::NotSupported` | Browser/authenticator lacks required WebAuthn behavior. |
| `prf_unsupported` | `CeremonyRefusal::NoPrf` | Passkey cannot provide Tonk's custody PRF outputs. |
| `security_context` | `CeremonyRefusal::Security` | RP/origin/secure-context failure. |
| `awaiting_activation` | `CustodyDenial::AwaitingActivation` | Expected account gate; result is `blocked`, not failure. |
| `suspended` | `CustodyDenial::Suspended` | Account policy gate; result is `blocked`. Do not capture its reason. |
| `not_provisioned` | `CustodyDenial::NotProvisioned` | Custody/account setup incomplete. Do not capture its reason. |
| `access_denied` | other typed service denial or 401/403 | Authorization refusal without the response text. |
| `conflict` | stable service code or HTTP 409 | State conflict, including wrong-account passkey when typed. |
| `not_found` | stable service code or HTTP 404 | Expected object/registration absent at the current stage. |
| `rate_limited` | HTTP 429 | Retry after service throttling. |
| `network` | client network error without response | Web or CLI could not reach its local worker, account service, or remote. |
| `service_unavailable` | HTTP 5xx or typed unavailable code | Local worker/access service failed. |
| `invalid_response` | typed decode/wire-shape error | A response arrived but violated its interface. |
| `local_state` | typed local profile/root/repository/storage error | Client-local state could not satisfy the action. |
| `callback` | typed callback bind/wait/delivery error | The CLI could not listen or wait, or the browser could not notify it. `stage` identifies the owning interface. |
| `unknown` | no typed evidence | Classification gap; alert on it and deepen the upstream error interface. |

The initial `service_code` allowlist is `root_required`,
`credential_revoked`, `upstream_timeout`, `upstream_unavailable`,
`account_state_unavailable`, `invalid`, `unauthorized`, `forbidden`,
`unknown_customer`, `unknown_consumer`, `customer_active`,
`customer_inactive`, `customer_suspended`, `address_taken`,
`consumer_provided`, and `internal`. These are normalized spellings of existing
worker/registration variants; they are never copied directly from an
unrecognized response. Adding a value requires updating the schema test and
telemetry inventory.

Do not add a raw-message fingerprint in version 1. It would be high-cardinality,
could encode user data even when hashed, and would not explain the fault. The
action/stage/failure/version/browser dimensions are sufficient to rank common
failures; `unknown` is a prompt to introduce a stable typed code at its source.

## File map

- `rust/tonk-analytics/src/account.rs`: Closed event vocabulary, property
  serialization, validation, and privacy-focused unit tests shared by every
  client interface.
- `rust/tonk-analytics/src/lib.rs`: Export the account schema and declare the
  `account_event` event constant.
- `rust/tonk-analytics/src/web.rs`: Typed web capture interface, version super
  property, optional redacted exception adapter, and capture-fixture hooks.
- `rust/tonk-analytics/src/native.rs`: Typed native account capture into the
  existing in-memory batch; no second transport or flush.
- `rust/tonk-identity/src/passkey.rs`: Reusable typed mapping from DOM exception
  name to `CeremonyRefusal`; no analytics dependency.
- `rust/tonk-ui/src/error.rs`: Structured account transport/response error
  information alongside the local diagnostic string.
- `rust/tonk-ui/src/api.rs`: Preserve network, HTTP class, service code, and
  decode kind for account endpoints instead of flattening them into prose.
- `rust/tonk-ui/src/user_error.rs`: Produce one `AccountProblem` containing
  both safe recovery text and the closed failure classification.
- `rust/tonk-ui/src/account_observability.rs`: Deep web
  attempt/checkpoint/outcome adapter, PostHog adapter, in-memory test adapter,
  duration, terminal guard, and automatic failure-streak suppression.
- `rust/tonk-ui/src/lib.rs`: Register the new module.
- `rust/tonk-ui/src/account.rs`: Instrument settings, login, account management,
  passkey addition, profile/device management, sign-out, revocation, and
  deletion actions at their existing error/presentation seam.
- `rust/tonk-ui/src/register_dialog.rs`: Instrument registration dialog views,
  email lookup, create/login, pending activation, initial name, clipboard, and
  recovery outcomes.
- `rust/tonk-ui/src/custody_relay.rs`: Preserve typed ceremony refusal and emit
  custody-consent/handoff stages without recording raw thrown values.
- `rust/tonk-ui/src/activate.rs`: Instrument activation-link validation,
  submission, expiry, service refusal, and completion.
- `rust/tonk-ui/src/analytics.rs`: Safe panic/exception capture and web-level
  standard context.
- `rust/tonk-ui/src/account_flow.rs`: Browser integration coverage for exact
  account event sequences and negative privacy assertions.
- `rust/tonk-cli/src/account_observability.rs`: Deep native command-attempt
  adapter, process-local event buffer, terminal guard, and typed CLI outcome and
  degradation classification.
- `rust/tonk-cli/src/lib.rs`: Export the native account-observability module.
- `rust/tonk-cli/src/callback.rs`: Preserve a closed callback failure kind
  (`bind`, `closed`, `server`, or `timeout`) alongside the existing local
  diagnostic so native classification never parses display text.
- `rust/tonk-cli/src/bin/tonk.rs`: Instrument account command and browser-handoff
  stages before errors are flattened to `ExitCode`; keep local diagnostic text
  out of both event streams.
- `rust/tonk-cli/src/telemetry.rs`: Queue shared `AccountEvent` values beside
  the generic `cli_command_run` record and send both in the existing bounded
  native batch flush.
- `rust/tonk-cli/tests/telemetry.rs`: Wire-shape, event-ownership, degradation,
  privacy, and opt-out coverage for account commands.
- `rust/tonk-access-service/src/observability.rs`: Structured, content-free
  Worker operational log records.
- `rust/tonk-access-service/src/lib.rs`: Request-level operational context and
  failure logging at the Worker entry seam.
- `rust/tonk-access-service/src/handlers/registration.rs`: Registration,
  activation, resend, and customer-probe outcome codes.
- `rust/tonk-access-service/src/handlers/lookup.rs`: Email-lookup operational
  outcomes without the queried address.
- `rust/tonk-access-service/src/handlers/ucan.rs`: Authorization/provisioning
  failure outcomes without subject, invocation, or reason text.
- `wrangler.toml`: Explicit production/staging Workers Logs settings with query
  redaction and invocation-log suppression.
- `.github/workflows/publish.yml`: Gated source-map upload before deployment if
  the exception-capture proof establishes useful stack resolution.
- `docs/telemetry.md`: Complete public inventory, opt-out behavior, retention
  distinction, and privacy contract.
- `docs/account-observability.md`: Internal PostHog/Cloudflare dashboard,
  investigation, alert, and rollout runbook.

## Task dependencies

- Task 1 establishes the canonical schema and blocks Tasks 3, 4, 5, 6, and 7.
- Task 2 establishes typed web failure evidence and blocks Tasks 3 through 5.
- Task 3 establishes the web adapter and blocks Tasks 4 and 5.
- Task 7 depends only on Task 1 and may proceed independently of the web
  instrumentation; it must consume the same schema rather than copying Task 3.
- Task 8 may proceed independently after its failure names are checked against
  Task 1; it emits a distinct operational-log shape, not `AccountEvent`.
- Task 9 follows the final schemas and staging payloads from Tasks 1 through 8.

### Task 1: Define and enforce the account event schema

**Files:**
- Create: `rust/tonk-analytics/src/account.rs`
- Modify: `rust/tonk-analytics/src/lib.rs:event`
- Modify: `rust/tonk-analytics/src/web.rs:ph_init, capture`
- Modify: `rust/tonk-analytics/src/native.rs:Client`
- Test: `rust/tonk-analytics/src/account.rs`
- Test: inline transport tests in `rust/tonk-analytics/src/web.rs` and
  `rust/tonk-analytics/src/native.rs`

**Interfaces:**
- Consumes: the existing guarded `tonk_analytics::web::capture` and
  `tonk_analytics::native::Client::capture` transports.
- Produces: `AccountEvent`, the closed enums in the event-contract tables, and
  `AccountOutcome`, `web::capture_account(&AccountEvent)` plus
  `native::Client::capture_account(&AccountEvent)`; target adapters cannot pass
  arbitrary properties through either account entry point.

- [ ] Add `it_serializes_only_the_account_event_allowlist`, constructing one
      start, checkpoint, success, cancellation, blocked activation, 5xx, and
      unknown-commit event. Assert exact keys and snake-case values and assert
      terminal-only fields are absent from starts.
- [ ] Add `it_rejects_invalid_account_event_shapes`: no terminal result on a
      start, failure without `failure_kind`, success with a failure kind,
      degraded success without `degradation_kind`, degradation on another
      result, duration over 600,000 ms, unrecognized service code, and a
      non-terminal phase with `duration_ms` must fail validation rather than be
      captured.
- [ ] Add `web_and_native_capture_the_same_account_event_shape`: feed one
      `AccountEvent` to both target adapters and assert the account-owned keys
      and values are identical. Permit only transport-owned standard context
      such as `environment`, `$lib`, OS, and architecture to differ. Implement
      this as paired native and Wasm golden-payload tests because the two
      transports are target-gated and cannot be linked into one test binary.
- [ ] Add a sentinel privacy test containing `person@example.com`,
      `did:key:zSensitive`, a credential ID, an activation URL with `ucan=`, a
      callback URL, and an HTTP body in nearby source inputs. Serialize the
      resulting typed event and assert none of the sentinel substrings occur.
- [ ] Run `cargo test -p tonk-analytics`; expect the new tests to fail because
      the account schema and capture entry point do not exist.
- [ ] Implement the enums and a private `validated_properties()` serializer.
      Keep arbitrary maps out of the public interface; `capture_account` is the
      only account-event path on both targets. Keep clocks, attempt generation,
      failure-streak state, callback handling, and flushing out of this module.
- [ ] Register `version` beside `environment` in `ph_init`, and set
      `capture_exceptions: false` explicitly so a PostHog project-side toggle
      cannot silently start collecting console/browser errors before Task 6.
- [ ] Make native `capture_account` add `environment=cli` through transport
      context before queuing; do not make each CLI call site supply it. Preserve
      the existing generic command event's identical environment value.
- [ ] Run `cargo test -p tonk-analytics` and
      `cargo test -p tonk-analytics --target wasm32-unknown-unknown`; expect all
      native and Wasm schema tests to pass.

### Task 2: Preserve typed failure evidence through the UI

**Files:**
- Modify: `rust/tonk-identity/src/passkey.rs:CeremonyRefusal, ceremony_error`
- Modify: `rust/tonk-ui/src/custody_relay.rs:CeremonyError::thrown`
- Modify: `rust/tonk-ui/src/error.rs:TonkUiError`
- Modify: `rust/tonk-ui/src/api.rs:account endpoint helpers`
- Modify: `rust/tonk-ui/src/user_error.rs:diagnostic, ceremony, api`
- Test: the inline test modules in those files

**Interfaces:**
- Consumes: `CeremonyRefusal`, `CustodyDenial`, structured worker `ErrorBody`,
  `reqwest::StatusCode`, and the current exact local diagnostic.
- Produces: `AccountProblem { message, failure_kind, result,
  http_status_class, service_code }`; presentation and telemetry consume the
  same value.

- [ ] Add a table-driven `CeremonyRefusal::from_name` test for
      `NotAllowedError`, `InvalidStateError`, `NotSupportedError`,
      `SecurityError`, `NoPrfError`, and an unknown name. Run
      `cargo test -p tonk-identity --target wasm32-unknown-unknown`; expect
      failure because callers cannot yet recover the typed name.
- [ ] Extend UI `CeremonyError` with `refusal: Option<CeremonyRefusal>` and read
      it from the rejected object's `name`; retain `denial` independently so a
      service refusal and browser refusal cannot overwrite one another.
- [ ] Introduce a structured account API error variant carrying only
      `transport_kind` (`network`, `http`, `decode`, `local`), optional status,
      optional already-allowlisted service code, and the existing diagnostic.
      Convert account functions from `account_status` through hosted-space
      deletion; leave unrelated editor/repository functions unchanged.
- [ ] Refactor `user_error` classification to return `AccountProblem`, and keep
      the existing string-returning helpers as thin compatibility wrappers
      during call-site migration. Typed evidence wins; compatibility prose
      matching must report `failure_kind=unknown`; deepen a typed source instead
      of promoting matched prose into an analytics classification.
- [ ] Extend current user-error tests to assert both recovery copy and
      `failure_kind/result`, including cancellation, timeout, duplicate
      credential, missing PRF, unsupported browser, awaiting activation,
      suspended, not provisioned, 401/403, 404, 409, 429, 5xx, network, decode,
      local state, and unknown commit.
- [ ] Assert a suspension reason, access-service reason, response body, email,
      DID, credential ID, and browser exception detail appear only in the local
      diagnostic and never in `AccountProblem`'s analytics fields or message.
- [ ] Run `cargo test -p tonk-ui --lib` and
      `cargo check -p tonk-ui --target wasm32-unknown-unknown`; expect success
      with no new prose-matching call sites.

### Task 3: Add the deep web account-attempt adapter

**Files:**
- Create: `rust/tonk-ui/src/account_observability.rs`
- Modify: `rust/tonk-ui/src/lib.rs`
- Test: `rust/tonk-ui/src/account_observability.rs`

**Interfaces:**
- Consumes: `AccountAction`, `AccountProblem`, and the typed
  `tonk_analytics::account` vocabulary.
- Produces: `WebAccountAttempt::start`, `checkpoint`, and `finish`; production
  `PostHogRecorder` and test `MemoryRecorder` adapters at one web-recorder seam.

- [ ] Add `it_records_one_start_and_one_terminal_outcome` with a fake monotonic
      clock. Assert a repeated `finish` and checkpoints after finish are no-ops,
      and duration is capped at 600,000 ms.
- [ ] Add `it_gives_each_attempt_an_opaque_non_content_id`; assert IDs differ,
      are bounded to 36 ASCII characters, and contain none of the action,
      account state, or supplied diagnostic.
- [ ] Add `it_reports_one_automatic_failure_per_streak`: three equal load
      failures yield one terminal event, a success yields a recovery event, and
      a later equal failure begins a new streak. User-triggered attempts remain
      unsuppressed.
- [ ] Add `it_keeps_expected_blocks_out_of_failure_totals`: cancellation,
      awaiting activation, and suspension retain their distinct result/failure
      properties rather than becoming `retryable_failure`.
- [ ] Run `cargo test -p tonk-ui --lib account_observability`; expect failure
      because the recorder module is absent.
- [ ] Implement the recorder behind the three-method interface. Use a random
      opaque attempt token from browser randomness; when randomness is
      unavailable, use a page-local monotonic token rather than account data.
      Never persist an attempt token to local storage. Keep browser-only clocks,
      failure-streak suppression, and page lifetime here rather than adding
      them to `tonk-analytics::account`.
- [ ] Make PostHog-disabled capture a no-op while still allowing UI logic to
      finish. Do not log analytics delivery errors to the account UI or retry
      them through an account operation.
- [ ] Run the focused module tests, `cargo test -p tonk-ui --lib`, and the
      target-specific check; expect success.

### Task 4: Instrument onboarding, login, activation, and passkeys

**Files:**
- Modify: `rust/tonk-ui/src/register_dialog.rs`
- Modify: `rust/tonk-ui/src/account.rs:create/login/add-passkey handlers`
- Modify: `rust/tonk-ui/src/custody_relay.rs`
- Modify: `rust/tonk-ui/src/activate.rs`
- Modify: `rust/tonk-ui/src/account_flow.rs`

**Interfaces:**
- Consumes: the Task 3 `WebAccountAttempt` interface and Task 2 problems.
- Produces: complete, correlation-safe event sequences for the four critical
  journeys, with no change to their durable state or recovery behavior.

- [ ] Add a browser capture fixture that replaces `window.posthog` with an
      in-memory stub before `analytics::install`, then exposes captured event
      JSON to `account_flow` assertions. It must exercise the real
      `before_send` function, not a second Rust-only serializer.
- [ ] Extend `it_signs_up_through_the_account_panels` to require:
      registration viewed; create started; email lookup checkpoint; passkey
      create checkpoint; terminal `blocked/awaiting_activation`; activation
      started and succeeded; settle started and succeeded. Assert one terminal
      event per attempt and ordered timestamps.
- [ ] Extend `it_waits_for_the_email_when_a_second_device_signs_in` to require
      login `blocked/awaiting_activation`, followed by a recovery settle event
      after activation; awaiting activation must not increment the product
      failure series.
- [ ] Extend `it_retries_the_committed_address_after_a_failed_passkey_ceremony`
      to distinguish a cancelled ceremony from an unknown-commit recovery and
      assert that retry creates a new attempt ID.
- [ ] Add focused stub cases for `InvalidStateError`, `NotSupportedError`,
      `SecurityError`, no PRF, worker-handoff timeout, typed service denial, and
      a 5xx. Assert their exact action, stage, result, and failure kind.
- [ ] Add a negative capture assertion over every event in those journeys for
      the actual test email, root DID, credential bytes, account endpoint,
      activation URL/UCAN, service refusal text, and callback URL.
- [ ] Run each named browser test against the baseline event code; expect it to
      fail because no `account_event` records exist.
- [ ] Instrument at the existing user-gesture and error-presentation seams.
      Define create completion as “registered and waiting for activation,”
      activation completion as successful consumption of the emailed link,
      and login completion as the account dashboard becoming ready. Do not call
      `mediate_now` success full login success when later local attachment can
      still fail.
- [ ] Record validation failures only after submission; never record field
      contents, keystrokes, or dialog focus/hover activity.
- [ ] Run the focused browser tests serially with
      `cargo test -p tonk-ui --features integration-tests -- <test-name>
      --test-threads=1`, then run `cargo test -p tonk-ui --lib` and the Wasm
      check. Expect all to pass.

### Task 5: Instrument account management and destructive recovery

**Files:**
- Modify: `rust/tonk-ui/src/account.rs`
- Modify: `rust/tonk-ui/src/register_dialog.rs`
- Modify: `rust/tonk-ui/src/custody_relay.rs`
- Modify: `rust/tonk-ui/src/account_flow.rs`

**Interfaces:**
- Consumes: the same recorder; no new event names or free-form properties.
- Produces: start/terminal coverage for every remaining `AccountAction` and
  failure-streak coverage for automatic account loads.

- [ ] Add a compile-time/table test mapping every `AccountAction` to exactly
      one stable `journey`, `action`, and default stage. Adding an enum variant
      without telemetry classification must fail the test/build.
- [ ] Cover automatic settings/account/registration/device/profile loads.
      Assert repeated polling failures collapse to one streak, successful
      reload emits recovery, and a new failure after recovery is visible.
- [ ] Cover display-name change, activation resend, add/switch profile, link
      CLI, copy callback/invite, sign out, load deletion plan, revoke device,
      delete hosted space, and delete account. Each user gesture gets a start
      and exactly one result.
- [ ] For revoke/delete, assert pre-dispatch cancellation is `cancelled`, a
      rejected mutation is `terminal_failure` when the server proves no commit,
      a lost/undecodable response after dispatch is `unknown_commit`, and a
      partial deletion result records `unknown_commit` without target IDs or
      counts that reveal account inventory.
- [ ] Extend `it_revokes_the_cli_device_from_the_browser` and
      `it_deletes_the_account_and_releases_its_email_and_profile` to inspect the
      captured sequences while retaining their existing durable-state
      assertions.
- [ ] Add a failure case for clipboard/callback delivery that proves the
      account/passkey success remains success while the separate callback
      action ends with `failure_kind=callback`.
- [ ] Run focused browser tests, `cargo test -p tonk-ui --lib`,
      `cargo check -p tonk-ui --target wasm32-unknown-unknown`, and
      `cargo fmt --all -- --check`; expect success.

### Task 6: Add safe runtime exception and panic diagnostics

**Files:**
- Modify: `rust/tonk-analytics/src/web.rs:ph_init, before_send`
- Modify: `rust/tonk-ui/src/analytics.rs:install_panic_hook`
- Modify: `rust/tonk-ui/src/account_flow.rs`
- Conditional modify: `.github/workflows/publish.yml`
- Conditional modify: `flake.nix:tonk-ui`

**Interfaces:**
- Consumes: PostHog's `$exception`/`captureException` interface and the existing
  normalized-route/privacy rules.
- Produces: useful uncaught exception issues tagged with environment/version;
  handled account outcomes remain only `account_event` records.

- [ ] First add a browser payload test for an uncaught `Error`, an unhandled
      rejection, a cross-origin/extension-shaped error, and a Wasm panic whose
      message contains every privacy sentinel from Task 1. Inspect the exact
      outbound payload, not only the PostHog stub call.
- [ ] Configure `capture_exceptions` with unhandled errors/rejections enabled
      and console errors disabled. Drop cross-origin/extension-only exceptions.
      In `before_send`, normalize/remove current/referrer URLs and query/hash,
      remove code-variable payloads, and replace exception values with a closed
      type plus safe static location. Preserve same-origin stack frames only.
- [ ] Replace the current raw first-line `panic.message` property. Capture a
      static panic type, `PanicHookInfo::location()` reduced to repository-
      relative file/line when available, and a fingerprint derived only from
      type + static location. Never hash or send the panic message itself.
- [ ] Assert handled `AccountProblem`s and all `console.error` calls generate
      zero `$exception` events. Assert opt-out and missing-key builds generate
      no account or exception request.
- [ ] Build a release-shaped UI and determine whether PostHog resolves its JS
      and Wasm stack locations with an uploaded, non-public source map. Add a
      publish upload step only if the built artifact contains useful maps and a
      staging exception resolves to repository source; otherwise document the
      limit and retain static panic location plus release version.
- [ ] If uploading, use a dedicated CI secret, associate the release with the
      Tonk version and commit SHA, upload before `wrangler deploy`, and verify
      the deployed asset directory contains no `.map` files.
- [ ] Run the browser payload tests, `nix build .#tonk-ui`, and a staging-only
      synthetic exception. Confirm one redacted issue in PostHog Error Tracking
      with environment/version and no sentinel data before enabling production.

### Task 7: Add the deep native CLI account-attempt adapter

**Files:**
- Create: `rust/tonk-cli/src/account_observability.rs`
- Modify: `rust/tonk-cli/src/lib.rs`
- Modify: `rust/tonk-cli/src/bin/tonk.rs:account_op and command dispatch`
- Modify: `rust/tonk-cli/src/account.rs:link_with_operator, link_via_callback, LinkOutcome`
- Modify: `rust/tonk-cli/src/callback.rs:Callback::bind, Callback::receive`
- Modify: `rust/tonk-cli/src/telemetry.rs:Recorder`
- Modify: `rust/tonk-cli/tests/telemetry.rs`

**Interfaces:**
- Consumes: `tonk_analytics::account::{AccountEvent, AccountOutcome, Action,
  Stage, FailureKind, DegradationKind}`, the static `AccountCommand` descriptor,
  `callback::CallbackFailureKind`, and typed evidence at each CLI account stage
  before it becomes `anyhow` prose or `ExitCode`.
- Produces: `CliAccountAttempt::start`, `checkpoint`, `finish`, and
  `into_events`; `Recorder::account_events` queues those validated events beside
  one unchanged `cli_command_run`, and `Recorder::finish` sends the single
  existing native batch within 300 ms. `CliAccountObserver` exposes only
  `checkpoint(Stage)` and `degraded(DegradationKind)` to the account link path;
  `CliAccountAttempt` and `NoopAccountObserver` are its two adapters.

- [ ] Add `it_records_native_account_start_checkpoints_and_one_terminal` with a
      fake monotonic clock. Cover repeated `finish`, checkpoints after finish,
      a 300-second browser wait, and the shared 600,000 ms duration cap. Assert
      `surface=native_cli` and an opaque process-local attempt ID.
- [ ] Add a table test mapping every `AccountCommand` descriptor (`status`,
      `login`, `logout`, `delete`, `space-list`, `space-pull`, `space-delete`,
      `sync`, `devices`, and `revoke`) to one shared `journey`, `action`, and
      initial stage using the event-contract mapping above. A new account
      subcommand without a mapping must fail.
- [ ] Add focused login tests for these exact terminal classifications:
      already signed in -> `blocked/conflict` at `local_preflight`; loopback
      bind failure -> `retryable_failure/callback` at `callback_bind`;
      callback timeout -> `retryable_failure/timeout` at `callback_wait`;
      browser denial -> `blocked/access_denied` at `callback_wait`; malformed or
      invalid grant -> `terminal_failure/invalid_response` at
      `delegation_validate`; and durable-state write failure ->
      `unknown_commit/local_state` at `activation_stage` when commitment cannot
      be disproved. Classify from the branch and typed source, never from the
      rendered error.
- [ ] Replace callback prose-only errors with
      `CallbackFailureKind::{Bind, Closed, Server, Timeout}` and a
      `CallbackFailure::kind()` accessor while preserving the current `Display`
      text and available source chain. Make `Callback::bind` and
      `Callback::receive` return the typed error. Add callback unit tests for
      each kind; the observer maps `Timeout` to `failure_kind=timeout` and the
      other callback transport failures to `failure_kind=callback`.
- [ ] Add zero-exit login fixtures for failed automatic browser opening followed
      by a successful manual handoff, an unhydrated `LinkOutcome::warning`,
      failed content-endpoint discovery, failed onboarding-custody rotation, and
      failed local-space rotation. Assert terminal `result=degraded_success`
      with `degradation_kind=browser_open`, `account_sync`,
      `content_discovery`, `custody_rotation`, or `space_rotation` respectively,
      while `cli_command_run.success=true`. When several occur, record the
      earliest incomplete stage as the primary degradation and retain all exact
      warning text only on local stderr.
- [ ] Add a native wire-shape test asserting one account invocation produces
      one `cli_command_run` plus the expected `account_event` sequence in the
      same `/batch` payload. Assert the generic event retains only its existing
      command/subcommand/exit/duration properties and does not acquire account
      `stage`, `result`, or `failure_kind` fields.
- [ ] Add a negative assertion over both event types for the fixture's email,
      callback and remote URLs, root/device DIDs, delegation, credential ID,
      local paths, argv values, warning text, and stderr error. Add opt-out
      coverage for the same failed command and assert no request arrives.
- [ ] Run `cargo test -p tonk-cli --test telemetry`; expect the account-event
      sequence and degradation assertions to fail because only the coarse
      `cli_command_run` record exists.
- [ ] Implement `CliAccountAttempt` as a process-local buffer of shared typed
      events. It may own a clock and random attempt token, but no PostHog client;
      `Recorder::account_events` validates and queues its events into the
      recorder's existing `tonk_analytics::native::Client`. Do not create a
      second HTTP client, request, flush timeout, opt-out decision, or account
      property map.
- [ ] Preserve typed stage outcomes through `account_op`, `link_account`, and
      the observed link path until the attempt is finished. Existing public
      account entry points that do not record telemetry call the observed form
      with `NoopAccountObserver`; do not duplicate the link algorithm.
      Keep browser/passkey stages owned by the web adapter: the CLI records
      `callback_bind`, `browser_open`, `callback_wait`, `delegation_validate`,
      `activation_stage`, `account_sync`, `content_discovery`, and
      `custody_rotation`; the browser records passkey, consent, and
      `callback_delivery`. Neither side emits the callback address or shares an
      attempt ID through it.
- [ ] Run `cargo test -p tonk-cli --test telemetry`, focused callback/account
      tests, `cargo test -p tonk-cli --lib`, and
      `cargo fmt --all -- --check`; expect success.

### Task 8: Add privacy-safe access Worker operational logs

**Files:**
- Create: `rust/tonk-access-service/src/observability.rs`
- Modify: `rust/tonk-access-service/src/lib.rs`
- Modify: `rust/tonk-access-service/src/handlers/registration.rs`
- Modify: `rust/tonk-access-service/src/handlers/lookup.rs`
- Modify: `rust/tonk-access-service/src/handlers/ucan.rs`
- Modify: `wrangler.toml`
- Test: inline module tests plus existing access-service integration tests

**Interfaces:**
- Consumes: stable handler operation, status, typed registration/provisioning
  refusal, deployment name, and whether a retry is safe.
- Produces: one structured JSON log for each failed account-related Worker
  request; no success/request-body logging and no user/profile identifier.

- [ ] Add serializer tests for `AccessFailureLog` with exact keys:
      `schema_version`, `system=access_worker`, `operation`, `outcome`,
      `failure_kind`, `status_class`, `retryable`, and `version`. Test
      enrollment, activation, resend, lookup, customer probe, authorization,
      and provisioning failure variants.
- [ ] Feed the serializer an email, DID, subject, R2 key, invocation, activation
      URL, and internal error. Assert none can enter serialized properties; an
      unknown internal failure becomes only `failure_kind=internal` plus a
      static `site` enum.
- [ ] Run `cargo test -p tonk-access-service --lib`; expect failure because the
      observability module does not exist.
- [ ] Emit structured objects at the HTTP response seam. Use `console_warn` for
      expected 4xx refusals and `console_error` for unexpected/unavailable 5xx.
      Never write exact internal diagnostics from the deployed Worker adapter;
      native helper/local-server stderr remains the detailed development sink.
- [ ] Replace account-related Worker logs that currently interpolate a subject,
      DID, or raw error with the structured adapter. Do not broaden this change
      to ordinary storage/metering logs in the same commit.
- [ ] Configure production and staging Workers Logs explicitly with logs
      enabled, `head_sampling_rate = 1`, `invocation_logs = false`, and query
      string redaction. The worker also serves static assets, so disabling
      invocation logs avoids recording every asset request while preserving
      every explicit account failure log. Configure preview identically so QA
      evidence has the same shape as staging and production.
- [ ] Run access-service library and registration/lookup/UCAN integration tests,
      `nix build .#tonk-access-service`, and `wrangler deploy --dry-run` for the
      checked-in configuration. Expect structured failure records and no
      request URL/query in the inspected output.
- [ ] After staging deployment, trigger one typed 4xx and one controlled 5xx;
      verify both appear under Workers & Pages -> tonk-access-service ->
      Observability with searchable closed fields and no user/account data.

### Task 9: Document dashboards, alerts, retention, and investigation

**Files:**
- Modify: `docs/telemetry.md`
- Create: `docs/account-observability.md`
- Modify: `docs/storybook/accounts/lifecycle.md:Privacy and telemetry`
- Modify: `docs/storybook/accounts/authority-and-deletion.md:Privacy and telemetry`
- Modify: generated Storybook data through the repository build script if the
  source changes affect its inventory

**Interfaces:**
- Consumes: the final event/log schemas and actual staging field names.
- Produces: reproducible PostHog insights, Cloudflare saved queries, alert
  definitions, and a privacy/audit runbook.

- [ ] Expand the complete telemetry inventory with `account_event`, every
      property/value family, the difference between analytics and operational
      logs, exception capture, opt-out behavior, and the explicit never-sent
      list. State that both web and native CLI adapters emit this event, while
      `cli_command_run` remains a separate invocation metric and must not be
      added to account-attempt counts.
- [ ] Create a PostHog “Account health” dashboard, filtered to
      `environment in (production, staging, cli)` with dashboard-level
      environment and `surface` selectors, containing only `account_event` for
      attempt/outcome insights:
      1. attempts and unique profiles by `surface`, then `action`;
      2. terminal success, degraded success, cancelled, blocked, failure, and
         unknown-commit counts;
      3. hard-failure rate (`retryable_failure + terminal_failure +
         unknown_commit` divided by `started`) plus degraded-success rate
         (`degraded_success` divided by `started`), each with a minimum-attempt
         annotation;
      4. failures broken down by `failure_kind`, then `stage`;
      5. degraded successes by `degradation_kind`, then `stage`, with CLI
         account-sync, discovery, and custody warnings visible independently of
         process exit status;
      6. impacted unique profiles by browser, OS, version, environment, and
         surface; browser-only dimensions must show “not applicable” rather
         than excluding native events;
      7. p50/p95 `duration_ms` by action and surface for terminal events;
      8. onboarding funnel: registration viewed -> create started -> pending
         activation -> activation success -> settle success;
      9. web login funnel: login started -> passkey assert -> login success, with
         cancellation and activation blocks visible as exclusions;
      10. native login funnel: login started -> callback bound -> callback
          received -> delegation validated -> activation staged -> account
          ready/degraded, without claiming exact correlation to the separate
          browser attempt;
      11. passkey-add and destructive-action outcomes by surface; and
      12. `unknown`, 5xx, panic, and `$exception` events as a release-regression
          panel.
- [ ] Save alerts for any production-web or native-CLI
      `failure_kind=unknown`, for a panic or `$exception` burst, and for a
      15-minute account hard-failure rate above 10% only when at least 20
      attempts exist. Route alerts through the team's existing PostHog
      notification destination; do not add a new external integration without
      approval.
- [ ] Save Cloudflare queries grouped by `operation/failure_kind/site` for
      access Worker 4xx and 5xx, plus a version/environment comparison. Record
      the Workers Logs retention shown by the live account rather than assuming
      it equals PostHog retention.
- [ ] Write the investigation order: identify the top PostHog
      `surface/action/stage/failure_kind` or `degradation_kind`; split by
      environment/version and then interface-relevant browser/OS dimensions;
      reproduce that typed branch; then consult matching-time Cloudflare
      aggregate logs for access-service failures. State that browser and CLI
      attempts deliberately have separate attempt IDs and that PostHog and
      Cloudflare deliberately have no stable per-user join key.
- [ ] Document event ownership: product code owners approve enum additions,
      privacy review approves new properties, and `unknown` classifications are
      fixed by deepening typed upstream errors rather than adding raw strings.
      Document the stage split: web owns passkey/PRF/consent/callback delivery;
      CLI owns listener/browser-open/wait/grant-validation/local-convergence;
      access Worker owns request-handler failures.
- [ ] Run `python3 scripts/build.py --check` from `docs/storybook` when its
      sources are modified, `python3 scripts/check-links.py .`,
      `cargo fmt --all -- --check`, and `git diff --check`; expect success.

## Rollout and completion gate

1. Deploy schema and instrumentation to staging with the PostHog capture
   fixture, native batch fixture, and both interface opt-out tests green.
2. Exercise one success and every safe synthetic failure family. Compare the
   web and CLI PostHog event exports against the same allowlist and privacy
   sentinels before opening production ingestion. Confirm one CLI account
   invocation batches one generic `cli_command_run` plus its `account_event`
   sequence without creating a second request.
3. Run staging for at least one normal QA cycle. Confirm starts and terminal
   outcomes balance except for demonstrably abandoned navigation, automatic
   polls do not dominate counts, CLI zero-exit warnings appear as degraded
   successes, and `unknown` is not the leading category on either surface.
4. Enable production account events. Keep generic exception capture disabled
   until Task 6's exact outbound-payload and source-location checks pass.
5. Enable structured access Worker logs only after `wrangler` dry-run confirms
   query redaction and invocation-log suppression for production and staging.
6. Review the dashboard after the first meaningful production sample. Rank
   fixes by impacted unique profiles and failure rate, not raw event count; use
   cancellation and awaiting activation as journey friction, not reliability
   defects.

Completion requires fresh evidence after the final change:

- all native and Wasm analytics/UI unit tests pass;
- the named account browser journeys pass serially with exact event sequences;
- the CLI telemetry integration tests contain the shared schema plus the
  unchanged generic command event in one batch, and opt-out sends nothing;
- access-service focused integration tests and Nix builds pass;
- a staging PostHog export contains every expected event and no sentinel;
- staging Error Tracking contains only deliberately triggered, redacted
  unhandled exceptions if Task 6 is enabled;
- staging Workers Logs contain the controlled 4xx/5xx structured records and no
  URL query, email, DID, subject, invocation, credential, or raw error;
- the saved dashboard, funnels, alerts, and operational queries match the
  documented field names; and
- `cargo fmt --all -- --check`, documentation build/link checks, and
  `git diff --check` pass.

## Explicit non-goals

- Recording page contents, keystrokes, session replays, heatmaps, or arbitrary
  console/network logs.
- Identifying the passkey manager or collecting authenticator/credential
  material. Browser and OS properties already supplied by PostHog are enough
  for platform comparisons.
- Sending server operational logs to PostHog or bypassing a user's telemetry
  opt-out with server-side product analytics.
- Changing account lifecycle, activation, retry, deletion, authority, or
  storage semantics while adding instrumentation.
- Building one cross-platform recorder containing DOM, polling, callback,
  process-flush, and Worker concerns; only the event contract is shared.
- Correlating an individual browser ceremony to an individual CLI process by
  adding an identifier to the callback protocol in version 1.
- Using telemetry as proof that a mutation committed. Durable state and typed
  service receipts remain canonical; telemetry is a projection for diagnosis.

## External interface references checked for this plan

- PostHog exception capture and redaction:
  <https://posthog.com/docs/error-tracking/capture>
- PostHog trends/funnels:
  <https://posthog.com/docs/product-analytics/trends/overview> and
  <https://posthog.com/docs/product-analytics/funnels>
- Cloudflare Workers structured logs and sampling:
  <https://developers.cloudflare.com/workers/observability/logs/workers-logs/>
- Cloudflare observability query builder:
  <https://developers.cloudflare.com/workers/observability/query-builder/>
