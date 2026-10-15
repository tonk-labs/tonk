# Agent-link MCP production implementation

Status: implementation complete and locally validated for stdio and hosted HTTP.
Deployment and real provider-client acceptance remain separate rollout gates.

## Contract and checkpoints

1. Promote the scoped worker into a supported native backend. Validate every
   supplied link against host-pinned routing. Add private-storage validation,
   cross-process locking, bounded admission, and a durable uncertain-write journal.
   Never pull over unpublished work or replay an evaluation. Recovery must be an
   explicit publish of retained state, not a repeated notation command.
2. Promote the Node adapter and stdio entrypoint. Bound queues, input, output,
   cancellation and shutdown; constrain shortcut redirects; redact credentials.
   Keep Desktop/pinned and account tool behavior unchanged.
3. Add a no-OAuth HTTP service with the same backend, request bounds, origin/host
   checks, stateless legacy/modern transport and durable state before acknowledging writes.
   Replace OAuth on the existing /mcp URL, as explicitly requested.
4. Prove per-call auth, expiry, remote revocation (including cached content),
   separate grants/spaces, concurrent access, uncertain-write recovery and restart,
   real stdio and HTTP calls with independent remote readback. Add CI/build/docs.

## Decisions

- Reuse ordinary worker query/evaluate and existing invitation cryptography.
- A link never falls back to account authority.
- Host configuration pins the access service and shortcut origin. Automatic
  discovery is not required to operate this backend safely.
- No automatic eviction of unpublished replicas. Capacity refusals are explicit.
- Credential-bearing request bodies never enter request logs or error responses.
- Only closed workers can be checkpointed. A checkpoint failure blocks further
  mutations until storage recovery; it must never cause evaluation replay.
- No production claim based solely on fixtures; report external acceptance and
  durability limitations precisely.

## Evidence

The preceding spike passed ordinary-worker unit checks, 18 MCP tests and a real
local access-service/S3 integration with stdio and independent remote readback.
New implementation checks are recorded here as each checkpoint is completed.

Implementation notes:
- The existing hosted executable, Docker image, generated deployment configuration,
  connection instructions and deployment smoke now select agent-link access at /mcp.
  Account/OAuth modules remain available to other hosts; their stored checkpoints
  are neither imported nor deleted. The new container uses its own state namespace.
- The native worker locks its private root, bounds admission, validates every link,
  journals uncertain writes and provides an operator-only publish_retained command.
- Legacy clients use stateless HTTP without retained account sessions. Desktop's
  internal signed-in query/evaluate contract is unchanged.
- Checkpointing covers closed replicas after each dispatched operation, before
  acknowledgment. In-flight container loss remains an unknown outcome and may
  lose unpublished local state. This is not distributed exactly-once delivery.

Checks so far:
- Core MCP: 21 passed; hosted: 63 passed; Apps compatibility: 3 passed.
- Native worker unit: passed (ordinary evaluator, isolation, invalid link,
  noncommitting query, and refusal of offline cached reads).
- Repository formatter wrapper hit npm registry DNS failure. The same pinned
  Prettier 3.6.2 from the local cache passed all repository MCP format patterns.
- Native integration compiled; first execution hit the sandbox loopback boundary.
  Rerunning unchanged with loopback permission; final result recorded below.

Final native integration result:
- `cargo test --offline --locked -p tonk-worker --features native-host --test
  agent_link -- --nocapture` passed with loopback access (5.52s execution).
- Actual native stdio and HTTP evaluations were independently read from remote
  storage. HTTP replicas were checkpointed, restored to a separate directory,
  and reopened through MCP. Expired links and contending root locks were rejected.
- A simulated post-publication interruption was explicitly recovered without
  evaluation replay. Revoking cell PUT produced retained unpublished state,
  blocked further evaluations, and refused recovery publication. Independent
  remote readback excluded that unpublished change. Revoking cell GET then
  blocked both ordinary cached and retained-state queries.
- Focused hosted service and packaging checks passed again after shutdown changes.
- No dependency lock files changed. No public deployment, Docker image run,
  real hosted-client acceptance, load test, or native Desktop UI test was performed.
  Existing native dead-code warnings are unchanged.

PR #1105 lint follow-up:
- CI Clippy rejected an unnecessary recipient `to_string()` allocation in the
  worker. The matching all-targets local check also found the same pattern in
  the integration fixture's subject provisioning. Both now borrow with `as_ref()`.
- `cargo clippy --offline --locked -p tonk-worker --all-targets --all-features --
  -D warnings` passed. The earlier native-host-only diagnostic run encountered
  existing dead-code warnings from disabled optional features; CI enables all
  features. Formatting and diff checks passed for the correction.
