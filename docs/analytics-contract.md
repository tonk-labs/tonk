# Product analytics contract

This contract defines the product-owned interactions Tonk sends to PostHog.
It complements the account lifecycle and launch contracts in
[`telemetry.md`](telemetry.md). Generated applications can create unbounded
domain-specific interactions; Tonk measures the platform operations they use,
without inspecting their content.

## Canonical lifecycle

`product_event` is a closed schema. An attempt has one `started` event, zero or
more `checkpoint` events, and at most one `finished` event. Count attempts with
unique `attempt_id` values whose phase is `started`. Calculate results from the
matching terminal event. A click is only a successful terminal event when the
named product interaction itself completed at that boundary.

| Property | Meaning |
|---|---|
| `schema_version` | Contract version, currently `1` |
| `journey` | `startup`, `account`, `space`, `collaboration`, `workspace`, `sync`, or `handoff` |
| `action` | Closed operation name from `tonk_analytics::product::ProductAction` |
| `phase` | `started`, `checkpoint`, or `finished` |
| `stage` | `intent`, `validation`, `worker`, `welcome`, `local_commit`, `remote_commit`, `clipboard`, `ready`, or `complete` |
| `surface` | `shell`, `welcome`, `settings`, `hub`, `workspace`, `join`, or `native_cli` |
| `trigger` | `user`, `automatic`, or `recovery` |
| `attempt_id` | Opaque per-attempt correlation token, never an account or content identifier |
| `result` | Terminal `success`, `degraded_success`, `cancelled`, `blocked`, `noop`, `retryable_failure`, `terminal_failure`, or `unknown_commit` |
| `failure_kind` | Closed diagnostic class; present only for non-success terminal results |
| `duration_ms` | Terminal elapsed time, capped at ten minutes |

Browser events also receive the in-memory `environment`, `version`, and
reviewed launch-attribution super properties. Native events add
`environment=cli`, version, OS, architecture, and library. Identity remains a
profile-level hash; metrics must not label it a human or account. Native batch
items record their RFC 3339 UTC occurrence time when queued, so end-of-command
flushing does not collapse lifecycle ordering.

## Privacy boundary

No event may contain notation, prompts, names, entity or account identifiers,
raw routes, URLs, query parameters, capabilities, request bodies, local paths,
or error messages. Browser capture removes SDK-added current/previous paths,
referrers, arbitrary UTM strings, click IDs, and initial URL fields at the
final `before_send` boundary. The sealed-guest relay carries a JSON envelope;
the top-level receiver accepts only the typed `product_event` shape and
validates it before capture.

## Coverage ledger

`source` means a current producer reaches the typed adapter. `tested` means a
focused producer or payload test exists. Browser end-to-end and hosted
ingestion remain separate release gates.

| ID | Representative interaction | Canonical receipt | Status |
|---|---|---|---|
| START-01 | Open product | worker then Welcome checkpoints; first guest ready, or bounded startup failure | source |
| ACCOUNT-01 | Save display name | account API response | source |
| ACCOUNT-02 | Load deletion plan | parsed plan response | source |
| ACCOUNT-03 | Delete hosted space | deprovision response | source |
| ACCOUNT-04 | Delete account | terminal worker ceremony row | source |
| ACCOUNT-05 | Add passkey | terminal worker ceremony row | source |
| ACCOUNT-06 | Sign out | local account removal response before reload | source |
| ACCOUNT-07 | List/add/switch profile | roster response or activation receipt before reload | source |
| HANDOFF-01 | Approve/decline terminal link | terminal worker ceremony or explicit decline | source |
| HANDOFF-02 | Copy agent prompt | Web Awesome clipboard success or error event; copied content is never read | source |
| JOIN-01 | Resolve pasted/short invite | local validation and resolution result | source |
| JOIN-02 | Retry join | recovery action dispatched | source |
| SPACE-01 | Create space | worker-confirmed local creation | source |
| SHARE-01 | Mint and copy share link | remote mint checkpoint then browser clipboard confirmation | source |
| WORK-01 | Activate sheet | selected sheet projected; manual and automatic triggers separated | source |
| WORK-02 | Create/close sheet | subscription-driven sheet count confirms the local commit; cancellation is explicit | source |
| SYNC-01 | Pause/resume auto-sync | local preference write succeeds or fails | source, tested |
| CLI-01 | Every parsed non-account CLI command family | command exit, paired with existing command summary | source, tested |
| PRIV-01 | SDK enrichment sanitizer | exact outbound sanitizer drops route/campaign sentinels | tested |
| RELAY-01 | Guest analytics transport | each frame forwards; top page validates once | source |

Existing typed `account_event` producers remain canonical for registration,
login, activation, custody, and native account commands. Existing
`space_conversion` and `space_shared` events remain compatibility projections
of worker-confirmed create/join/mint success. They must not be added to
`product_event` attempts as extra attempts.

## Dashboard questions

The Product decisions dashboard answers these questions over a 30-day window:

- Which operations are attempted, by surface and environment?
- Which attempts finish, fail, block, cancel, or disappear without a terminal
  receipt after ten minutes?
- At which stage does each journey stop?
- Which closed failure classes affect which action and release?
- What are median and 95th-percentile terminal latencies by result?
- How often does startup reach the first usable guest, and where does it stop?
- Which CLI commands run and what coarse exit classes do they return?

Rates show their sample count. A missing terminal event means abandoned or
unobserved after the follow-up interval; it is not silently converted into a
failure. Staging/dev/production/CLI stay separate. New schema results must be
cut over by deployed version and must not be blended with historical events
that had different semantics.

## Release verification

Before claiming hosted coverage, build and deploy an artifact containing this
schema, run controlled staging interactions against a disposable test profile,
and re-read PostHog for the expected event count and exact allowlisted fields.
Verify opt-out produces no request. Browser nesting, clipboard permissions,
passkey ceremonies, multi-window behavior, Safari, deployment, and production
ingestion are not proven by native unit tests.
