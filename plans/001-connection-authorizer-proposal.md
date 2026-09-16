# Historical connection authorizer feasibility spike (superseded)

Status: SUPERSEDED by the user's 2026-09-16 protocol decision in
[Plan 001](001-cli-space-connections.md). This file records an earlier experiment,
not current implementation instructions or a request to publish its patch.

The accepted flows use long-lived ordinary UCAN grants: a browser-generated bearer
invitation key pair, or delegations to a CLI-generated public key after browser
selection. Standard UCAN validation/revocation supplies authorization. There is
no single-use redemption, connection-state gate, or authority-bearing marker.
Direct use of valid invitation authority is intended behavior, not a bypass.

Preserve the patch, harness and test evidence as historical work. The independent
S3 lifetime finding remains relevant: bound signed URLs by effective authority
expiry and a documented transport ceiling. Extract a minimal signing-expiry fix
if necessary; do not publish/integrate the broad checkpoint patch unchanged.
All recommendations and “next” steps below describe the superseded experiment.

## Implemented dependency increment

`patches/dialog-connection-authorizer.patch` applies to the exact revision below.
It adds `VerifiedInvocation`, `authorize_with_policy`, and
`authorize_with_policy_and_clock`. The callback receives only the verified path;
it returns a typed denial or optional absolute deadline. The bounded entry point
requires signing credentials and clamps descriptors to 60 seconds, effective
ancestor expiry, and policy expiry, with a fresh clock sample after the callback.
The callback must be read-only: command decoding/signing can still fail after it
returns. Existing `authorize` callers retain their compatibility behavior.

The upstream diff changes four files: authorizer implementation/tests, its public
export, the crate manifest, and one lockfile dependency-list entry for the existing
workspace `chrono` dependency. The main Dialog checkout and Tonk's dependency pin
and lockfile are untouched. The isolated clone is useful for review, but the
ordinary patch file is the durable artifact.

Fresh upstream checks: 41 native authorizer tests passed (7 new policy/deadline
tests), Wasm library check passed, formatting and whitespace passed. Existing
listener tests required an unchanged retry with local-network access. Wasm emitted
five existing unused-constant warnings in `dialog-credentials`.

`spikes/connection-policy.rs` is registered only by the isolation harness. It
exercises the new API with signed local S3 PUT/GET, exact registered grant CIDs,
bootstrap/wrong-key refusal, descendant marker shadowing, session/operator lineage,
and refusal after fixture revocation. Existing signed URLs can still be read in
their bounded lifetime. Fixture management is trusted setup; this is not an
authenticated redemption endpoint, durable connection store, or the production
Tonk handler. These limitations describe the earlier experiment; they are not a
requirement to implement redemption or a policy-state database under the revised plan.

## Findings at the pinned dependency

`Cargo.lock` pins Dialog `tonk-2026-09-14` to
`4c16de9e345d2b2d888d1008c5d3f0ca990c4807`. Paths below are relative to that
Dialog checkout, not a proposed dependency update.

- `rust/dialog-remote-ucan-s3/src/authorizer.rs:450`: `authorize` verifies the
  invocation and returns an already signed `Permit`. It discards the effective
  `TimeRange` returned by verification. It exposes no policy callback or expiry
  argument. Its dispatch macro at line 234 creates and immediately signs an
  `S3Request`.
- `rust/dialog-remote-s3/src/request.rs:45`: request expiry defaults to 3,600
  seconds. `S3Request.expires` is public, and
  `rust/dialog-remote-s3/src/s3/credential.rs:91` signs that value. Thus the
  low-level signer supports a short deadline; the integrated authorizer does not.
- `rust/dialog-ucan-core/src/container/invocation.rs:96`: separate verification
  is available and returns the effective ancestor/invocation time window.
  `proofs()` and `delegation(cid)` expose the signed proof blocks for that verified
  chain. Chain visibility is **not** an absolute API blocker.
- The same file at line 156 exposes `meta(key)` with invocation-first, then
  leaf-first lookup. It explicitly calls metadata informational. An attacker may
  shadow a grant marker using descendant metadata. A connection gate must inspect
  the immutable registered ancestor CID's own `Delegation.meta()`, after verifying
  the exact chain; it cannot use this convenience lookup.
- Tonk native `src/helpers/server.rs:751` and Cloudflare
  `src/handlers/ucan.rs:275` both call the integrated authorizer. Neither presently
  interprets connection purposes or binds a bootstrap to a redeemed session key.

## Executable characterization

`rust/tonk-access-service/tests/connections.rs` contains two tests named
`connection_spike_*`. They deliberately assert the current behavior; these are
feasibility probes, not passing session-security gates:

1. A signed, narrowly scoped, purpose-marked bootstrap grant receives a GET
   descriptor from a real local `/ucan/` endpoint without registration/redemption.
   Ordinary customer provisioning is active, so provisioning does not mask the
   missing connection restriction.
2. A delegation expiring within 120 seconds receives a signed URL whose
   `X-Amz-Expires` is 3,600 seconds. Neither ancestor expiry nor the proposed
   60-second ceiling clamps the descriptor.

Run: `cargo test -p tonk-access-service --features helpers --test connections connection`.
The plan execution log records actual run results. No race/revoke/D1/browser
claims follow from these probes.

## Historical bounded API recommendation (superseded)

Add an upstream additive authorization entry point retaining the current
`authorize` for compatibility:

1. Parse and verify once using the configured DID resolver, explicit verification
   clock, and existing ancestry-aware revocation checker.
2. Expose an immutable verified invocation view to an async embedder policy:
   subject, invocation audience/issuer, command/arguments, ordered proof CIDs and
   delegation blocks, and effective `TimeRange`. The view must identify only the
   proof path actually verified, not unrelated container blocks.
3. The policy checks authoritative connection state, exact registered marker CID,
   service binding, subject, command/branch allowlist, and bound session-key
   lineage. It returns a refusal or an absolute maximum descriptor deadline.
4. The existing command dispatcher creates the `S3Request`. At signing time,
   clamp its expiry to the earliest of the effective ancestor expiry, policy
   deadline, and signing time plus 60 seconds. Reject an exhausted window rather
   than rounding it up. Preserve request path, body checksum, conditional headers
   and all existing typed denial distinctions.
5. Return the signed descriptor only after this policy succeeds. Use the same
   entry point in native and Cloudflare services and other granting transports.

An even smaller upstream `authorize_with_deadline(container, deadline)` method
could solve the presign bound while Tonk separately verifies and gates the chain.
It would duplicate verification/resolution and must still preserve exact proof
identity, authoritative state timing, and shared typed errors. This is a viable
intermediate option; it does not need a new signing trust model.

## Alternatives considered

- Tonk can separately verify/gate with existing UCAN APIs. That solves policy
  visibility but cannot shorten the signature returned by `authorize`. Editing
  `X-Amz-Expires` after signing invalidates SigV4.
- Tonk can own the command-to-`S3Request` dispatcher and use the existing public
  low-level signer. That is technically possible without a dependency change,
  but duplicates capability argument decoding, seven canonical commands, legacy
  aliases, error mapping and security-sensitive request construction. Do not
  quietly create this second implementation while treating milestone 1 as proved.
- Deserializing private `S3Authorization` fields, reconstructing a request from
  an already signed URL, or changing the dependency checkout outside the repo
  would rely on private representation or untracked changes. None is proposed.
- Server custody of per-invitation private keys is unnecessary for these API
  changes and remains outside the chosen trust model.

## Historical decisions and next proof (superseded)

Recommend the verified-view policy hook: it preserves one verifier and one typed
dispatcher while giving the embedder the exact authority and deadline it needs.
The small deadline extension plus explicit Tonk verification/gate remains a
bounded fallback. Publish/pin the additive API through the repository's
dependency workflow, then replace the characterization assertions with actual
rejection and expiration contracts before progressing to SQLite redemption.

Still required: proof of marker discovery through operator rotation; rejection
of removed/tampered markers, alternate audience and direct bootstrap use; exact
command/branch coverage; atomic race/revoke ordering; sibling isolation; descriptor
expiry at an explicit clock; alternate transport coverage. No UI, CLI authority
migration, release, or production issuance work is justified by this spike alone.

The current integrated authorizer samples wall time internally; these probes do
not establish deterministic clock-controlled expiry behavior. A future API must
permit explicit clock fixtures for the deadline boundary tests.

The source-derived candidate build preset is six exact leaf operations: memory
cell get/put restricted to `space=branch/main, cell=revision`; archive block
get/put restricted to `catalog=index`; archive blob get/put. Normal fact deletion
is a content write and does not require memory retract. Issue an exact leaf grant
bundle or prove an equally narrow enforceable representation; `/use` alone is
broader and also covers future commands. Current predicate evaluation sees only
invocation arguments, so an argument predicate cannot independently express an OR
of command paths. This coverage remains to be proved by real sync/build commands.

## Reproducing the local dependency experiment

`patches/dialog-connection-authorizer.patch` carries the additive Dialog experiment
against `4c16de9e345d2b2d888d1008c5d3f0ca990c4807`. Run it through
`bash scripts/test-connection-authorizer.sh`. The default command selects the
Tonk `connection_policy_spike` tests; pass Cargo arguments after `--` to choose a
different focused check. This is a local experiment, not a published dependency.

The harness copies the current Tonk working files, including dirty changes, into
a fresh temporary directory. It clones the exact Dialog revision, applies the
patch, and generates source overrides for every package from that Dialog source
in the copied `Cargo.lock`, including transitive packages. Cargo may update only
the copied lockfile; the original lockfile is never changed or restored. Stop
editing relevant files while the snapshot is copied to obtain a coherent input.
The script rejects an unexpected pinned revision rather than silently rebasing.
It registers `spikes/connection-policy.rs` as a `connection_policy_spike` test
target only in the copied access-service manifest. This keeps the unreleased API
consumer out of ordinary builds against the unchanged git dependency. Its policy
and signed S3 roundtrip checks do not establish production connection integration.

By default the harness clones `.wt/dialog-connection-authorizer` if present, or
the Dialog GitHub repository otherwise. Set `DIALOG_SOURCE` to another local
clone to avoid that network fetch. Dirty edits in the source Dialog clone are
ignored: only the pinned commit plus the checked-in patch become test inputs.
Git, rsync, Cargo and Python 3.11 or newer are required. Cargo may still need
network access for uncached dependencies.

`bash scripts/test-connection-authorizer.sh --prepare-only` prepares and reports
the snapshot/config paths without running Cargo. The sandbox is retained after
success or failure for inspection. Build artifacts are isolated by default; an
explicit absolute `CARGO_TARGET_DIR` can reuse a cache, with the usual shared Cargo
lock. The script performs no publishing, deployment, original-lockfile cleanup,
or automatic sandbox deletion.
