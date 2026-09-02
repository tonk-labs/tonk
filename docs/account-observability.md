# Account observability runbook

Account health uses the typed `account_event` stream in PostHog and structured
access-service failures in Cloudflare Workers Logs. They are deliberately not
joinable per user. Browser and CLI handoff attempts also have separate random
attempt IDs.

## PostHog dashboard

The saved [Account health dashboard](https://eu.posthog.com/project/70116/dashboard/928106)
uses only `account_event`. Its aggregate HogQL insights were executed once
while the event stream was still empty, proving their query shapes without
introducing test account data. It contains:

1. starts and unique profiles by surface and action;
2. terminal results, separating success, degraded success, cancellation,
   blocks, hard failures, and unknown commits;
3. hard-failure rate (`retryable_failure + terminal_failure + unknown_commit`
   divided by `started`) and degraded-success rate, annotated only after 20
   starts;
4. failures by `failure_kind`, then `stage`;
5. degraded successes by `degradation_kind`, then `stage`;
6. impacted unique profiles by environment, version, surface, browser, and OS
   (native browser dimensions are “not applicable”);
7. p50/p95 terminal duration by action and surface;
8. exact attempt-level journey progression using the documented starts,
   checkpoints, and terminals; and
9. release regressions for `failure_kind=unknown`, 5xx, `panic`, and
   `$exception` (the latter remains empty while exception capture is disabled).

Never combine `cli_command_run` with `account_event` attempt counts. The
generic CLI record answers adoption and exit questions; account health uses the
typed stream, including degraded zero-exit commands.

Two saved alerts cover any unknown/5xx/panic/exception and a 15-minute
hard-failure rate above 10 percent after 20 starts. They are deliberately
disabled until staging proves the stream and production rollout is approved.
The project plan does not include the add-on required for 15-minute evaluation,
so PostHog accepted an hourly evaluation cadence over the exact trailing
15-minute query windows. Enabling them or adding another notification
integration requires rollout approval.

## Worker queries

Save access Worker queries grouped by `operation`, `failure_kind`, and `site`,
split into 4xx and 5xx status classes and compared by version/environment.
Record the retention displayed by the live Cloudflare account at rollout time;
do not assume it matches PostHog retention.

## Investigation order

1. Find the leading PostHog surface/action/stage/failure or degradation.
2. Split by environment and version, then by browser/OS only where relevant.
3. Reproduce the corresponding typed branch without using captured content.
4. For access-service stages, inspect matching-time aggregate Worker logs.

There is no per-user bridge between the systems. Exact local diagnostics are
used during reproduction, not copied into either remote system. An `unknown`
classification is fixed by deepening the upstream typed error; it is not fixed
by adding error text or fingerprints.

## Ownership and rollout

Product code owners approve enum additions; privacy review approves new
properties. Web owns passkey, PRF, consent, and callback-delivery stages. CLI
owns listener bind, browser open, callback wait, grant validation, and local
convergence. The access Worker owns request-handler failures.

Before production, compare staging web and CLI exports against the property
allowlist and privacy sentinels, confirm starts and terminals balance except for
abandoned navigation, and verify automatic polls do not dominate. Confirm a
controlled Worker 4xx and 5xx contain no URL/query, email, DID, subject,
invocation, credential, or raw error. Generic exception capture stays disabled
until an exact outbound-payload test and non-public source-map staging proof
both pass.
