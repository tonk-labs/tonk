# iOS passkey custody handoff implementation plan

**Goal:** Make passkey login complete on iOS Safari after the person selects a passkey, while preserving PR #872's tap-bound WebAuthn start and the existing account/custody cryptography.

**Approach:** Replace the page-to-service-worker `CryptoKey` fields with the two raw 32-byte WebAuthn PRF outputs as `Uint8Array`s. The worker validates and immediately imports those bytes as the same non-extractable HKDF handles it already consumes, so custody DIDs, KEKs, envelopes, and worker-owned minting remain unchanged. Bound the reply wait and log service-worker deserialization failures so a future transport failure produces recovery UI and evidence instead of an indefinite stall.

**Constraints:**

- Keep `begin_evaluate_custody_passkey` and the synchronous `credentials.get()` invocation added by PR #872. The assertion must still start before the login click handler returns.
- Keep WebAuthn in the page and all account opening, sealing, signing, linking, and persistence in the service worker.
- Preserve both independent PRF outputs and their existing `CUSTODY_KEY_CONTEXT` and `CUSTODY_KEK_CONTEXT` derivations. Do not collapse them, change a custody DID, change a KEK, re-seal existing envelopes, or migrate stored account data.
- Never serialize PRF bytes to JSON, hex, logs, storage, analytics, URLs, or error text. Post fixed-length `Uint8Array`s, copy them through structured clone, validate exactly 32 bytes in the worker, import them with `extractable: false`, and explicitly fill both the sender and receiver typed arrays with zero immediately after the synchronous post/import step. The Rust copies remain `Zeroizing<[u8; 32]>`.
- Apply the same envelope contract to the primary `key`/`kek` pair and the optional `holderKey`/`holderKek` pair used by `AddPasskey`.
- Treat missing, non-typed-array, wrong-length, invalid-credential-id, and half-present holder fields as explicit worker replies. Do not fall through to a plausible but incorrect custodian.
- Keep the existing one-request `MessageChannel` protocol and service refusal `code`; callers and user-facing denial handling must not change on successful delivery.
- Do not interpret the PostHog content-blocker errors or blob source-map errors from the device console as part of this failure; the on-device boundary probe isolated the dropped `CryptoKey` envelope.
- Do not change `Cargo.lock`; no new dependency is required.
- Storybook is unchanged because the successful UI and recovery pattern already exist. The only new visible state is the bounded transport-failure copy, covered in Rust and browser tests rather than a new screen family.

## Confirmed failure boundary

On the PR preview, iOS Safari logged `get ok` with both PRF results and `sw post "custody"`, but no reply. A control envelope with string placeholders reached the worker and replied `the handoff carried no derivation handles`; the identical envelope containing an HKDF `CryptoKey` never reached `onmessage`. This rules out user activation, PRF evaluation, the service-worker controller, and the reply port. The current `rust/tonk-identity/src/install.rs::mediate_pair` posts two `CryptoKey`s (four for add-passkey), and `rust/tonk-worker/src/router/custody.rs::custodian_named` requires those handles, so iOS silently drops the real handoff before Rust can log or reply.

## File map

- `rust/tonk-identity/src/passkey.rs`: expose `EvaluatedCustodyCredential`, whose two PRF outputs remain zeroizing byte arrays until the handoff is built.
- `rust/tonk-identity/src/webcrypto_kek.rs`: keep `Custodian` as the worker-side non-extractable-handle type; remove the page-provider wrappers and claims/tests that a `CryptoKey` must cross the worker boundary.
- `rust/tonk-identity/src/install.rs`: build byte-only custody envelopes, include the optional holder pair, and reject a missing worker reply after a bounded timeout.
- `rust/tonk-worker/src/router/custody.rs`: parse and validate primary/holder byte pairs, import them inside the worker, and retain the existing custody intent dispatch.
- `rust/tonk-worker/src/worker.rs`: update the raw-envelope dispatch comments to describe typed PRF bytes rather than `CryptoKey` handles.
- `rust/tonk-ui/assets/service_worker.js`: log global `messageerror` events without logging event data or key material.
- `rust/tonk-ui/src/user_error.rs`: map a custody-handoff timeout to direct retry/reload guidance and a retryable timeout outcome.
- `rust/tonk-ui/src/account_flow.rs`: cover both retained tap-bound invocation and the new page-to-worker byte-envelope contract in the real-browser harness.
- `plan/custody-in-the-worker.md`: correct the implemented transport/security description while retaining the worker-owned custody design.
- `plan/system-page-commands.md`: replace the obsolete cross-worker `CryptoKey` claim with the iOS-compatible transient-byte handoff.

### Task 1: Carry PRF bytes to the worker and import them there

**Files:**

- Modify: `rust/tonk-identity/src/passkey.rs:CustodyCredential, CustodyEvaluation, PendingCustodyAssertion::finish`
- Modify: `rust/tonk-identity/src/webcrypto_kek.rs:Custodian::adopt, Custodian::from_credential, Page providers, wasm tests`
- Modify: `rust/tonk-identity/src/install.rs:create_passkey, add_passkey, use_passkey, mediate, mediate_pair`
- Modify: `rust/tonk-worker/src/router/custody.rs:receive, perform, custodian_from, custodian_named, tests`
- Modify: `rust/tonk-worker/src/worker.rs:TonkWorker::on_message`
- Test: `rust/tonk-worker/src/router/custody.rs:tests`
- Test: `rust/tonk-ui/src/account_flow.rs:tests`

**Interfaces:**

- Consumes: `CustodyCredential { id, evaluation }`, where WebAuthn returns `CustodyEvaluation { key: Zeroizing<[u8; 32]>, kek: Zeroizing<[u8; 32]> }`; `AddPasskey` consumes two such evaluated credentials.
- Produces the page-side completion type used by all three installed ceremonies:

```rust
pub(crate) struct EvaluatedCustodyCredential {
    pub id: Vec<u8>,
    pub evaluation: CustodyEvaluation,
}

impl CustodyCredential {
    pub(crate) async fn into_evaluated(
        self,
    ) -> anyhow::Result<EvaluatedCustodyCredential>;
}
```

  `into_evaluated` reuses the result when creation supplied PRF outputs and performs the existing credential-pinned follow-up assertion only when they were absent.
- Produces this JavaScript envelope shape, with no `CryptoKey` fields:

```text
{
  type: "custody",
  credentialId: <hex string>,
  key: Uint8Array(32),
  kek: Uint8Array(32),
  request: <CustodyIntent>,
  holderCredentialId?: <hex string>,
  holderKey?: Uint8Array(32),
  holderKek?: Uint8Array(32)
}
```

- Produces worker helpers with explicit absence versus invalidity:

```rust
async fn custodian_from(
    data: &wasm_bindgen::JsValue,
) -> Result<tonk_identity::custodian::Custodian, String>;

async fn custodian_named(
    data: &wasm_bindgen::JsValue,
    prefix: &str,
) -> Result<Option<tonk_identity::custodian::Custodian>, String>;
```

  `Ok(None)` is valid only when every field for the optional holder is absent. A primary pair, or any partially present holder, must return a specific `Err`.

- Preserves: `tonk_identity::webcrypto_kek::Custodian` still holds two non-extractable `CryptoKey` handles after `Custodian::adopt`; all downstream `perform`, `login`, `add_passkey`, `create`, `enroll`, and parked-login code continues to receive the same Rust custodian type.

- [ ] Add `it_rebuilds_the_custodian_from_posted_prf_bytes` in `rust/tonk-worker/src/router/custody.rs`. Build a custody envelope with `Uint8Array::from(&[11u8; 32])` and `Uint8Array::from(&[22u8; 32])`, await `custodian_from`, and assert its signer DID equals a reference `Custodian::adopt` and its opener decrypts an envelope sealed through the existing byte path.
- [ ] Add table-driven parser cases for a missing primary field, a non-`Uint8Array` value, 31- and 33-byte values, malformed credential hex, and a holder with only one PRF field. Assert each case returns an error before any custody intent runs.
- [ ] Change the existing holder/add-passkey test fixture to send all three holder fields as typed bytes and prove that the added and holder custodians remain distinct and reconstruct the expected DIDs.
- [ ] Run `nix develop path:. -c test:web:debug -p tonk-worker -E 'test(it_rebuilds_the_custodian_from_posted_prf_bytes)'`; expect the new test to fail on the current branch with `the handoff carried no derivation handles`, because `custodian_named` accepts only `CryptoKey`.
- [ ] Add `EvaluatedCustodyCredential` and `CustodyCredential::into_evaluated` in `rust/tonk-identity/src/passkey.rs`. `use_passkey` must continue to call `begin_evaluate_custody_passkey` synchronously and only convert the already-started result after awaiting it.
- [ ] Change `create_passkey`, `use_passkey`, and both halves of `add_passkey` to obtain `EvaluatedCustodyCredential` directly from the passkey module and pass it to `mediate_pair`. Remove the now-unused `CreateCustodian`, `LoadCustodian`, `Page`, and `Custodian::from_credential` page-provider path from `webcrypto_kek.rs`; `Custodian` itself remains the worker-side handle type.
- [ ] Construct fresh `Uint8Array`s for `key`, `kek`, `holderKey`, and `holderKek`; do not add them to the transfer list, which remains `[channel.port2()]`. After `post_message_with_transferable` returns or throws, fill every sender-side typed array with zero before awaiting a reply; keep the original Rust arrays under `Zeroizing` until they drop.
- [ ] Make `custodian_named` validate each `Uint8Array` at exactly 32 bytes, copy into `Zeroizing<[u8; 32]>`, fill the received typed array with zero even on a wrong-length error, and call `tonk_identity::webcrypto_kek::Custodian::adopt` in the worker. Await primary parsing in `receive` and holder parsing in the `AddPasskey` arm before invoking existing custody logic.
- [ ] Remove `Custodian::new` and `Custodian::handles` if no non-test caller remains. Rewrite `it_carries_a_derivation_handle_across_structured_clone`, `it_derives_both_custody_keys_from_cloned_handles`, and `it_carries_a_custodian_to_the_worker` so they pin the new contract: posted bytes import to non-extractable handles and derive the same signer/KEK as before. Do not weaken the existing wire-format and wrong-custodian tests.
- [ ] Update comments in `install.rs`, `webcrypto_kek.rs`, `custody.rs`, and `worker.rs` so they distinguish transient PRF transport bytes from the non-extractable handles created and retained only in the worker.
- [ ] Add `it_posts_prf_bytes_after_the_tap_bound_assertion` beside the PR #872 browser regression. Use the virtual authenticator, wrap `ServiceWorker.prototype.postMessage`, and inspect the real custody message: `key` and `kek` must be `Uint8Array`s of length 32, neither may be a `CryptoKey`, and the transferred reply port must still be present. Reply through the captured port so the test does not hang. Keep `it_starts_login_passkey_before_the_activating_click_returns` unchanged to guard the earlier boundary.
- [ ] Run `nix develop path:. -c test:web:debug -p tonk-identity -p tonk-worker`; expect the focused WASM tests to pass.
- [ ] Run `cargo test -p tonk-ui --features integration-tests it_posts_prf_bytes_after_the_tap_bound_assertion -- --test-threads=1 --nocapture`; expect the browser test to pass and to observe no `CryptoKey` in the envelope.

### Task 2: Turn a dropped handoff into a diagnosable, recoverable failure

**Files:**

- Modify: `rust/tonk-identity/src/install.rs:mediate_pair, reply-wait helper, wasm tests`
- Modify: `rust/tonk-ui/assets/service_worker.js:self.onmessage, new self.onmessageerror`
- Modify: `rust/tonk-ui/src/user_error.rs:diagnostic_message, ceremony_problem tests`
- Test: `rust/tonk-identity/src/install.rs:tests`
- Test: `rust/tonk-ui/src/user_error.rs:tests`

**Interfaces:**

- Consumes: the existing one-shot reply promise created for `channel.port1` after `worker.postMessage` succeeds.
- Produces: `const CUSTODY_HANDOFF_TIMEOUT_MS: i32 = 30_000` and a rejection whose stable diagnostic is `the service worker did not answer the custody handoff in time`.
- Produces: a `self.onmessageerror` handler that logs only `"messageerror: service-worker message could not be deserialized"`; it must not touch or stringify `event.data`.
- Produces: retryable user copy for account/passkey actions: `Your passkey was approved, but this browser did not finish the secure handoff. Reload the page and try again.` The registration dialog's existing error branch must re-enable `log in with your passkey`.

- [ ] Add `it_times_out_when_the_worker_does_not_reply`, a WASM browser test for a reply-wait helper using an unconnected/no-reply `MessagePort` and a short injected timeout. Assert the returned promise rejects with the stable handoff-timeout diagnostic instead of remaining pending.
- [ ] Run `nix develop path:. -c test:web:debug -p tonk-identity -E 'test(it_times_out_when_the_worker_does_not_reply)'`; expect the new test to fail because the current reply promise has no timeout branch.
- [ ] Factor the reply wait in `mediate_pair` so production passes `CUSTODY_HANDOFF_TIMEOUT_MS` and tests can pass a short duration. Race the worker reply against a window timer, clear the timer on reply, clear `port1.onmessage` on either outcome, and reject only if no reply settled first. Do not start this timeout until after WebAuthn/PRF evaluation has completed and `post_message_with_transferable` has succeeded.
- [ ] Add `self.onmessageerror` next to `self.onmessage` in `rust/tonk-ui/assets/service_worker.js`. Log the fixed diagnostic once per event; do not attempt to recover or reply because a failed deserialization does not reliably preserve usable message data or ports across engines.
- [ ] Add unit cases in `user_error.rs` proving the stable diagnostic maps to the exact recovery copy and `AccountOutcome::retryable(FailureKind::Timeout)` for `LogIn`, `CreateAccount`, and `AddPasskey`; unrelated ceremony failures retain their existing mapping.
- [ ] Run `cargo test -p tonk-ui user_error`; expect all copy/outcome tests to pass.
- [ ] Run `nix develop path:. -c test:web:debug -p tonk-identity`; expect the reply and timeout paths to pass without leaked callbacks or a double settlement.

### Task 3: Correct the design record and verify the complete mobile path

**Files:**

- Modify: `plan/custody-in-the-worker.md:What it does instead, Why two handles, What moves, Shape of the change`
- Modify: `plan/system-page-commands.md:Hand over a key, not key material`
- Verify: `rust/tonk-ui/src/account_flow.rs`
- Verify: PR #872 preview on the same physical iPhone/iOS Safari/passkey provider that reproduced the failure

**Interfaces:**

- Consumes: Tasks 1 and 2 complete and green.
- Produces: documentation that states the actual boundary: the page transiently receives two PRF outputs, posts two clone-safe typed arrays, and the worker immediately imports non-extractable derivation handles before doing custody work.

- [ ] Update both design documents to record why the originally verified desktop structured-clone result was insufficient: a `CryptoKey` can survive `structuredClone` and desktop worker messaging while iOS Safari still drops the service-worker message. Preserve the reason for two independent PRF outputs and the invariant that custody minting remains in the worker.
- [ ] Document the security delta directly: the PRF bytes already existed in the page realm; the compatible handoff creates one transient worker copy, never persists/logs it, and converts it to non-extractable handles immediately. Do not claim that raw PRF bytes never reach the worker.
- [ ] Run `cargo fmt --all -- --check`; expect no formatting diff.
- [ ] Run `cargo check -p tonk-identity --target wasm32-unknown-unknown`, `cargo check -p tonk-worker --target wasm32-unknown-unknown`, and `cargo check -p tonk-ui --target wasm32-unknown-unknown`; expect all three WASM consumers to compile.
- [ ] Run `cargo test -p tonk-identity --lib` and `cargo test -p tonk-ui --lib`; expect native/unit coverage to pass.
- [ ] Run `cargo test -p tonk-ui --features integration-tests it_starts_login_passkey_before_the_activating_click_returns -- --test-threads=1 --nocapture`; expect PR #872's tap-bound regression to remain green.
- [ ] Run `cargo test -p tonk-ui --features integration-tests -- --test-threads=1`; expect the serialized account E2E suite to pass, including account creation, existing-account login, activation wait/resume, and add-passkey flows.
- [ ] Run `nix develop path:. -c test:web:debug`; expect the full debug WASM matrix to pass. Report any runner or environment failure separately from a test assertion failure.
- [ ] Run `git diff --check`; expect no whitespace errors. Review `git diff --stat` and confirm only the file map above plus this plan changed; do not stage unrelated work.
- [ ] Push the reviewed branch so the preview is rebuilt from the new head SHA. On the same iPhone, confirm the passkey picker opens from the tap, closes after selection, the login completes, and Settings shows the linked account/device. In the page console, the last boundary must advance from `sw post "custody"` to a worker reply; in the service-worker console, expect custody work/commit logs and no `messageerror` diagnostic.
- [ ] Repeat one add-passkey flow (which exercises `holderKey`/`holderKek`) and one existing-account login in a desktop browser to catch an asymmetric primary/holder envelope change. Record desktop, automated, and physical-iOS evidence separately; do not call the fix complete if physical iOS remains unverified.

## Requirement coverage

- The confirmed iOS `CryptoKey` deserialization failure is fixed by Task 1's typed-byte envelope and worker-side import.
- PR #872's user-activation behavior is preserved and re-run in Tasks 1 and 3.
- Primary login, creation/enrollment, and the add-passkey holder pair share one validated transport contract in Task 1.
- Silent future message loss becomes bounded and observable through Task 2's page timeout and worker-global diagnostic.
- Cryptographic and stored-data compatibility is pinned by the DID/KEK/wire-format tests in Task 1 and the explicit no-migration constraints.
- Completion requires the same-device physical reproduction to pass in Task 3, separately from desktop and automated evidence.
