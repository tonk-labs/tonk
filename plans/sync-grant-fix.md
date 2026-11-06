# Device grant replacement

Fix the locally reproduced grant selection failure in the shared worker.

- Regressions: fresh device grant replaces older same-account/device powerlines;
  grants for other devices/accounts/scopes survive; repeat reconciliation repairs
  imported obsolete grants; sign-out refreshes before no-op-sensitive retraction.
- Preserve the canonical root record before cleanup, so interrupted cleanup retries
  against the new grant. Reconcile at boot as well as sign-in for existing profiles.
- Work in this isolated checkout; preserve tonk-town's unrelated dirty work.
- Validate native account helper tests and the worker compilation/tests supported
  by the local toolchain. Deployment and live desktop recovery remain separate.

## Completed

- Shared reconciliation refreshes the profile branch, retains the canonical
  grant, and retracts older same-issuer/audience powerlines. Other accounts,
  devices and scoped grants survive.
- Sign-in, identical-record retries and worker startup reconcile against the
  persisted root record. Typed publication conflicts retry from a fresh head.
- Sign-out refreshes before retraction, including the first attempt: a stale
  handle previously returned a successful no-op without a CAS conflict.

## Validation (2026-10-08)

- Regression failed before cleanup at “obsolete grant must not remain selectable”.
- `cargo test -p tonk-account --lib --offline`: 40 passed.
- Worker WebAssembly test compilation passed (existing warnings remain).
- Repository `wbg-pool` browser runner: identity group 13 passed; onboarding
  group 10 passed, including startup repair and stale-handle sign-out tests.
- Direct wasm-bindgen runner initially could not bind inside the sandbox; outside
  it returned HTTP 404 through the available ChromeDriver. The repository runner
  successfully used installed Chrome directly.
- No production deployment or live desktop recovery performed. The earlier
  production investigation did not capture the exact rejected outgoing proof.

Checkout: `/tmp/tonk-sync-grant-fix`, based on `c67472a85`.

## Live test blocked: mixed storage formats (2026-10-08)

The user completed one sign-in with the local test app. The next browser link
failed registering the device: non-fast-forward account push, followed by a
recovery pull error `Failed to access part of the tree: failed without error
information` from rkyv/rancor.

The test app was built from c67472a85, with dialog 120fba8, which includes
self-describing/tagged search-tree nodes (dialog dd634aa, September 30). That
change reads legacy nodes but writes tagged nodes on edited paths. The deployed
public worker hash is 95545ca8a87c08a7; its binary lacks the tagged-node decoder
marker present in local worker 531b4c788d620cee. This strongly suggests the local
client wrote a newer account-tree format that the deployed browser cannot read.
The exact failing remote block has not been inspected, so causal attribution
remains a strong hypothesis rather than block-level proof.

Stopped the separate test app to prevent further sync. Do not repeat production
account tests with this newer runtime. Do not clear local stores, force-push old
heads, or downgrade serialized data. Preserve local and remote state. Next:
identify exact deployed dependency/build and inspect the failing block read-only;
then choose a compatible recovery and test the grant patch on the deployed
baseline or a wholly isolated fixture. A fresh local profile was not an isolated
backend: its account grant still authorized production account sync.

## Foundation approval test

User requested testing via tonk.foundation. Its live build 8c02fe37eac5d48c,
worker 0cf0ca69b08044b8, contains the tagged-node decoder marker. Reconfigured
only the temporary desktop copy in /tmp/tonk-sync-desktop to approve via
https://tonk.foundation/settings/link. Callback transport validation permits
only foundation or the existing network account home; worker signature/audience
validation remains in force. A focused test accepts those two exact services
and rejects unrelated hosts, HTTP, wrong paths, query strings and loopback.
The runtime remains the locally fixed build at 127.0.0.1:4189. This is still a
production-backed account test, not isolated remote data. Live sign-in and
repeated sign-out/sign-in remain to be verified by the user.
