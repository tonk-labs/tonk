# Publication reliability v1

This suite measures product publication separately from the twelve-session model
coordination comparison. It runs no models and contributes no quality points.
Do not change a past comparison's frozen protocol or omit failed reliability cases.

The protocol registers three disposable, initially empty workloads:

| Case | Operations | Work |
| --- | ---: | --- |
| independent-writes | 4 | Two barrier-start writers publish separate artifacts. |
| same-artifact | 2 | Both writers publish disjoint fields and a shared field on one artifact. |
| sustained-burst | 24 | Two writers per round, twelve rounds scheduled 100 ms apart. |

Each operation has a stable ID, artifact ID, unique field and deterministic payload
hash. Every successful independent field must survive. In the same-artifact case,
the shared field must contain one submitted value; either writer may win. A single
inventory establishes only `chosen_value_valid`. Convergence additionally requires
matching final readbacks from at least two independent replicas after writes finish.
This reflects cardinality-one convergence, without assuming a user-facing expected
revision CAS or requiring both conflicting strings to survive. A rejected operation
is reported explicitly and prevents full completion.

Create a plan before invoking a product fixture or service adapter:

```sh
python3 bench/bin/coordination_reliability.py plan --output /private/tmp/reliability-plan.json
```

The plan file is created exclusively and contains the protocol, exact operations,
schedule and canonical SHA-256 identity. An **external** adapter must execute those
operations and produce the receipt format below. The scorer does not perform writes
or recover/replay uncertain operations. The product's real local integration tests
can establish their asserted native behavior independently; they do not become
normalized suite results without producing matching receipts for this plan.

The opt-in native fixture is an executable adapter using disposable local access
services, independent writer identities/roots, real native worker processes, and
fresh observer roots. It needs no live service credential and launches no models:

```sh
TONK_RELIABILITY_PLAN=/private/tmp/reliability-plan.json \
TONK_RELIABILITY_OUTPUT=/private/tmp/reliability-native-001 \
  cargo test -p tonk-worker --features native-host --test agent_link_publication \
  reliability::emit_native_reliability_receipts -- --ignored --exact --nocapture
```

Use a new output directory for each invocation. Retain failed attempts. The fixture
records raw stdout, source/patch and native binary hashes, normalized observations,
and independent readbacks. Its `observations.json` is the scorer input; it does not
establish hosted Cloudflare behavior. The separate mechanism and fault-injection
tests run with `cargo test -p tonk-worker --features native-host --test
agent_link_publication -- --nocapture` (the opt-in emitter remains ignored there).

```sh
python3 bench/bin/coordination_reliability.py score \
  --plan /private/tmp/reliability-plan.json \
  --observations /private/tmp/reliability-observations.json
```

The observation document contains `plan_sha256`, `boundary` (`native_integration`,
`deployed_service`, or `synthetic_fixture`), `adapter_identity` with a credential-free
`command`, committed candidate `source_sha`, explicit boolean `source_dirty`, and
`source_patch_sha256` when dirty, plus `cases`. The patch hash must bind the actual
fixture/product changes exercised, including untracked sources if used. Each case has:

- `id`, `initial_inventory_complete: true`, and `initial_inventory: []` from its
  disposable target; each case uses a fresh target, not the previous case's state.
- `target_identity: {repository, build_sha}` matching the adapter's source, plus
  matching `source_patch_sha256` for a dirty candidate. `evidence_id` identifies the
  retained case log containing target identity, barrier and inventory evidence.
- `receipts`, one per registered operation: `operation_id`, `status`
  (`published_verified`, `failed`, `uncertain`, or `conflict`), `started_ms`,
  `finished_ms`, integer `publication_attempts` (zero only before publication), and
  `evidence_id` identifying the raw operation log for dispatch/publication/readback.
- `local_evaluation_count` observed at the committing local evaluation boundary:
  exactly one for an evaluated write, or zero for a failure before dispatch. A null
  or absent count is reported unknown and cannot pass. Two local evaluations fail
  even when they leave an identical final artifact. HTTP call count alone does not
  distinguish semantic validation/query requests from committing evaluations.
- `recovery: {required: false, resolution: "not_needed"}` or `required: true` with
  resolution `readback`, `retained_publication`, or `unresolved`. Record internal
  publication retries in the attempt count. Resolved recovery requires verified
  publication plus ordered `required_ms` and `resolved_ms` within the operation.
  Do not replay an uncertain evaluation.
- For a verified publication, `published_ms`, `readback_ms`, and `readback` containing
  the exact operation's `artifact_id`, `field`, and `payload_sha256`. This must be an
  independent readback after publication, not a local apply acknowledgment.
- `final_inventory_complete: true`, `final_observed_ms`, and `final_inventory`:
  records containing `artifact_id`, `fields` mapping operation field names to their
  payload hashes, plus `shared_value` for the same-artifact case.
- For the same-artifact case, `independent_shared_readbacks`, each with a distinct
  `observer_id`, actual `replica_id`, absolute `registry_root`, matching `repository`,
  `shared_value`, `synced_ms`, `read_at_ms`, and `evidence_id`. Both sync/read times must
  follow all writes; a completed query/pull can truthfully use the same sync/read
  timestamp. Replica IDs and normalized registry paths must be distinct. The adapter
  retains proof of those roots and the actual pulls, rather than relabeling one read.
- `barriers`, one per round, containing `round`, `ready_ms` keyed by writer number
  (`"0"`, `"1"`), `released_ms`, and `evidence_id`. All writers must be ready before
  release; operation starts must follow release on the same monotonic clock.

The top-level `evidence` manifest contains `{id, path, sha256}` entries for retained
raw logs or structured raw receipts. Paths are relative to the observations file's
directory; symlinks and path escapes are rejected. The scorer checks actual bytes
against each hash and requires referenced case, operation, barrier and replica
evidence. A single bound structured receipt may cover multiple related measurements.
It does not parse arbitrary native log formats: collection and truthful attribution
remain the trusted adapter's responsibility, and the logs remain inspectable.

`started_ms` is the barrier-admitted operation arrival. A faithful adapter keeps a
queue per independent writer/owner; burst arrivals remain scheduled even when that
owner is still publishing its previous operation. End-to-end latency includes that
queue time. Optional `native_execution_started_ms` and `native_execution_finished_ms`
record the actual native call interval. Queue delay and actual native overlap are
reported separately when supplied; otherwise native overlap is unknown. The reported
`round_overlap_ms` covers admitted operations and must not be presented as proof that
native critical sections overlapped. The deterministic mechanism fixture supplies
controlled publication competition separately.

All timestamps use one adapter monotonic millisecond clock. Final inventory must
follow writer completion. Starts in a round must overlap. The maximum start lateness
is registered as 250 ms; exceeding it fails the workload schedule, and the delay is
reported. This is an execution-control threshold, not a latency target or an SLO.
An adapter must separately retain barrier, request, publication and observer evidence
and identify the exercised source/build. Receipt validation alone cannot prove that
the adapter ran the real product or collected the evidence independently.

The report preserves completion rate, operation statuses, publication attempts,
required/resolved/unresolved recovery, latency distributions with sample counts,
duplicate receipts/records, missing independent fields, and lost **verified** fields.
Publication latency is publish minus start; readback latency is readback minus
publish; end-to-end latency is readback minus start. A failed acknowledgment later
resolved by independent readback may have `published_ms: null`; the publication
instant and publication-to-readback latency remain unknown, with the operation ID
listed and those samples omitted. Its observed end-to-end latency remains available.
Recovery rate is unknown when
no recovery was required. An absent case is unrun; an incomplete final inventory
makes completion and loss unknown. An absent receipt is listed and cannot pass.
No latency, token, billing, or quality value is inferred for missing work.

Run the deterministic measurement tests:

```sh
PYTHONDONTWRITEBYTECODE=1 python3 bench/bin/test_coordination_reliability.py
```

These tests use synthetic receipts to verify the planner, detector and CLI. They
are not native or hosted reliability validation. Record real fixture results and
their exact command/build before a paid model comparison, then launch a new matched
comparison with the existing efficient roster and frozen quality requirements.
