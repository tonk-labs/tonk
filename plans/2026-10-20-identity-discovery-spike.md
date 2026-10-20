# Email discovery and targeted invitation spike

Status: complete; native integration proof passed. The original run exposed a
reshare attribution bug; the [sender-attribution follow-up](2026-10-20-invitation-sender-attribution.md)
fixes new worker invitations and the updated spike verifies Bob as inviter while
Alice remains founder.
Base: `0df8e73610`, 2026-10-20.

Scope: a native integration test over the real local access service and worker
routes. Synthetic account activation uses the captured test inbox; no real email
is sent. Space provisioning uses the service fixture helper. Existing application
email discovery behavior remains unchanged. The follow-up changes invitation
attribution as described below.

Checkpoints:

1. Resolve a missing email and a pending registration without treating either as
   a confirmed recipient; confirm the email and obtain its actual account root.
   Attempt enrollment of the same normalized address under another account and
   require a conflict with the original mapping preserved.
2. Pass that root through `recipientRoot` to the worker invite route; assert a
   scoped invitation without a bearer seed.
3. Attempt the same invitation from another real worker account. Require 403,
   unchanged profile revision and no usable space authority.
4. Join as the resolved account, then mint a second targeted invitation from that
   joined account to a third account. Inspect the minted delegation chain and
   the persisted member attribution after the third account joins.

Test: `router::identity_discovery_spike::email_lookup_to_targeted_join_and_reshare_attribution`.
Requires `tonk-worker` feature `integration-tests` and loopback test services.
Results, failures and verification limits are recorded below.

## Results

The original characterization passed on 2026-10-20 in 24.72 seconds. After the
sender fix, the updated four-account regression passed in 13.13 seconds.

| Check | Observed result |
| --- | --- |
| Missing address | HTTP 404; no recipient root accepted |
| Unconfirmed registration | HTTP 202; no recipient root accepted |
| Confirmed plus-address | HTTP 200; document identity, embedded key, and account root agree |
| Another account registers the same address, with different casing and whitespace | HTTP 409, `AddressTaken`; lookup still returns Bob's root |
| Alice mints using the discovered Bob root | Scoped invite, exact recipient, no bearer seed fragment |
| Mallory submits that same invitation | HTTP 403, audience-mismatch message; profile revision unchanged and no space authority |
| Bob submits it | HTTP 201; joins the seeded space and can prove space authority |
| Bob discovers Carol and mints a new targeted invite | Scoped invite to Carol; actual proof chain includes Bob |
| Carol joins | HTTP 201; can prove space authority |
| Sender attribution after the fix | Both the invitation projection and Carol's persisted roster name Bob; Bob's entry names Alice |
| Original owner after the fix | Carol's replicated content still identifies Alice as founder and Bob as member; Alice has no inviter |
| Rejected account in roster | Mallory is absent |

The invite targets an account DID: copying its URL does not make it redeemable by
another account. The email mapping is the discovery step, not the access grant.

The original attribution assertion characterized the bug: `Invitation::from_chain`
selected the first proof's audience. The updated regression verifies the
minting account bound to the leaf signer by signed metadata and subject-open
identity grants. Older leaves without this metadata retain their old
interpretation; it is not verified sender evidence. Forwarding an existing URL
does not change its signed sender.

## Implementation

`rust/tonk-worker/src/router/identity_discovery_spike.rs` drives four independent
worker profiles: Alice creates a space, Bob receives it, Mallory attempts to
redeem Bob's invitation, and Bob reshares to Carol. Each account has its own
root and device credentials. Lookup uses HTTP against the local access service;
invitation creation, joining, syncing, and roster reads use the actual Axum
worker router. Only email-to-recipient glue is test-only.

The test is native-only and gated by `integration-tests`. The existing account
test fixture's `device_on` helper is visible within the crate for reuse. No
dependency versions change. The sender follow-up adds a direct internal schema
dependency on `tonk-invite` and changes attribution for new worker invitations.

## Verification log

- The first baseline compile in the workspace's existing `target/` failed with
  missing-field and wrong-argument errors referring to a different worktree's
  dependency sources. Source signatures here did not match those diagnostics.
  Retried in an isolated target directory instead of changing unrelated APIs.
- The isolated compile caught a test-helper return type: branch revisions are
  optional. Corrected the helper to preserve and compare `Option<Revision>`.
- The first test startup was blocked by the sandbox's local-listener restriction
  (`Operation not permitted` in the local S3 fixture). Retried the exact focused
  binary outside that boundary.
- That run exposed an incorrect test URL for a plus-address: the HTTP lookup
  returned 404 because the glue used the DID's encoded segment in the URL.
  Switched to the existing worker's `email_status::split_address`, which feeds
  the decoded segment to this endpoint. The `+` address remains in the test.
- Source review also corrected fixture provisioning to use `RepositoryInfo.subject`
  (the DID); worker URLs use `RepositoryInfo.name` (its routing key).
  The create request explicitly names `main` so the real standard-library
  seeding path runs before the recipients join. Interrupted outdated
  compilations before rebuilding these corrections.
- The next runtime run passed the missing/pending/confirmed email checks and
  refused the competing account with `409 AddressTaken`. It created and seeded
  the space, then failed on a non-JSON response before the invite handler ran.
  The request fixture lacked the `RequestOrigin` extension supplied by the
  browser adapter and existing native invite tests. Added that extension and
  endpoint/status diagnostics for any future non-JSON response.
- Local run uses the repository's `ci-test` profile with command-only
  optimization overrides for the fresh build. The 32 MiB test-thread stack is
  explicit for this unoptimized native integration run.

```sh
CARGO_TARGET_DIR=/private/tmp/tonk-identity-discovery-target \
RUST_MIN_STACK=33554432 \
cargo test --offline --profile ci-test \
  --config 'profile.ci-test.opt-level=0' \
  --config 'profile.ci-test.package."*".opt-level=0' \
  -p tonk-worker --lib --features integration-tests \
  identity_discovery_spike -- --test-threads=1
```

The final build used the same command with `--no-run`; the resulting binary was
run outside the listener-restricted sandbox with the exact test filter and
`--nocapture`. Displayed output filtered out captured activation URLs.

Final checks:

- New integration test: **1 passed**, 0 failed.
- Existing `router::join::failure_vocabulary` tests from the same fresh binary:
  **7 passed**, 0 failed.
- `cargo fmt --all -- --check`: passed after the final source edit.
- Scoped `git diff --check`: passed.
- The initial characterization changed no Cargo manifests or lock files; the
  sender fix adds the internal schema-to-invite dependency described above.
  Two existing unused-function warnings in `agent_connections.rs` remain.

## Limits and next step

This proves the native worker and local access-service flow. It does not prove
browser/Desktop UI, passkey ceremonies, real email delivery, hosted cache/rate
limits, or cross-device account recovery. The fixture activates synthetic
accounts through its captured inbox and pre-provisions the test space.

The run also logged a best-effort account-retention revision mismatch during
space creation. The tested space authority, sync, and joins succeeded, but
account-backup convergence was not asserted. Do not treat this fixture as
evidence of cross-device restoration or attribute the warning to production
without a dedicated reproduction.

The next small product increment is a typed confirmed-email resolver feeding
`recipientRoot`, with no fallback to an open invitation. Sender attribution needs
its own proof contract and regression expectation before any “verified sender”
label is added. No public discovery endpoint, address book, or sharing UI has
been added by this spike.
