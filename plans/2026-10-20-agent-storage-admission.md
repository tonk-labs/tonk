# Agent storage admission diagnostics

## Scope

Distinguish fixed safe root-admission error categories before opening any replica.
Do not change storage limits, eviction, retries, evaluation dispatch, journals,
publication recovery, or frozen coordination inputs/state.

## Observed boundary

V6 participant 09 returned a successful query at 2026-10-20T15:08:17.141Z,
then `Agent storage is unavailable or busy.` at 15:08:26.872Z, 15:08:36.398Z,
15:08:47.194Z, 15:09:26.119Z, 15:10:10.490Z, 15:10:53.563Z, and 15:11:26.851Z.
W4 observer started 15:11:30.374392Z and failed 9.499s later at collect/tool_rejection
with that same text, without a verified snapshot. This was sustained admission
failure, not evidence of a malformed query or an unresolved publication.

The exact message originates only from `lock_root` failure in AgentLinkWorker,
before replica open, pull, query/evaluation, or journal inspection/creation.
It currently conflates lock contention, invalid storage, ordinary I/O failure,
and aggregate capacity (192 MiB or3500 files). The Node owner serializes subprocesses;
each container has a distinct private temporary root, so a multi-container shared
filesystem lock is not established by current evidence.

Main rollout `7d4ec19bb6386975941bcfad2d398835ef099751` overlapped these calls:
deploy step 15:08:11–15:08:57Z, smoke completed 15:10:05Z. Timing does not prove cause.
It also invalidates the trial's intended fixed `f25d59f` deployment assumption.

W3 snapshot JSON was 1,684,414bytes (initial 531,671; w1 579,700; w2 1,090,973).
These are observable snapshots, not native filesystem usage: grant partitions,
block/index caches and other callers are absent. They cannot establish whether
192 MiB or3500 files was exceeded. Checkpoints admit 4096 files/256 MiB expanded,
so a previously acknowledged invocation can leave state above the next native
admission threshold; this is a hypothesis, not a claim about the live checkpoint.

## Smallest executable regression

Use synthetic signed authority and an unreachable loopback access endpoint.
Inject a root with 3500 empty files plus the worker lock, a sparse file larger than
192 MiB, a held exclusive lock, invalid permissions, a dangling lock symlink,
and an ordinary I/O failure from a non-directory parent. Require distinct fixed
capacity/busy/invalid errors, and no replica directory (hence no upstream request,
evaluation, or journal). Remove only this test's synthetic fixture afterward.

## Verification

- Baseline: the actual-worker regression failed all four injected cases with
  the exact captured generic message. Target built in 4m35s; test ran 0.45s.
  The test cleaned its own synthetic fixture. Log: `/private/tmp/tonk-storage-admission-baseline.log`.
- Typed admission categories now distinguish busy, invalid, byte capacity,
  file capacity and unavailable at the existing pre-open seam. No paths,
  credentials, filesystem messages or submitted notation enter tool errors.
- An extracted, unchanged final production `lock_root` function passed a
  standalone Rust spike for file/byte capacity, dangling lock without target
  creation, held lock, and successful readmission after release (2 seconds).
- An intermediate typed-category library passed the two existing integration
  tests and failed only the new dangling-lock assertion (14.84s tests after
  6m56s build). It was superseded by the reviewed lstat correction. This is not
  final green evidence; final focused integration remains pending.
- Final focused regression and existing agent-link recovery integration: pending.
- `cargo fmt --all --check` and `git diff --check` pass. Independent source review
  found no blockers after replacing the existence check with `symlink_metadata`.
  Pre-existing malformed/dangling lock entries are rejected before open; no
  TOCTOU/no-follow hardening is claimed, and lock acquisition remains unchanged.
- Hosted diagnostic category: unverified; separately authorized future single
  read-only call after deployment must be labeled diagnostic, never trial retry.
