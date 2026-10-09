# Linking completion and latency

Base: `fix/approval-request-progress` (request-scoped approval progress, PR #1076). Branch: `fix/link-completion`. Worktree: `/private/tmp/tonk-link-completion`, isolated from ongoing desktop/MCP work. This PR is stacked on #1076. Its preview is intended for Desktop end-to-end verification; no production deployment or full account sign-in is claimed.

## Confirmed defect

The page previously abandoned its custody MessagePort after 30 seconds, regardless of whether the worker was still approving the device. The worker could finish registration and publish DONE after the page had removed its listener, losing the callback. The user's existing Tonk.Foundation Helium console retained exactly `custody: Error: the service worker did not answer the custody handoff in time`.

The visible “Approved. Completing…” message alone is not proof of this sequence: the deployed UI also allows stale approval status, separately addressed by the request-scoped base branch. The timeout console entry and controlled reproduction are independent evidence. No fresh real passkey/ChatGPT/desktop authorization was performed in this investigation.

## Implemented increment

- Device approval negotiates progress messages; older pages never receive messages they would misinterpret as completion.
- The existing 30-second deadline now detects worker silence. Worker-owned progress keeps the listener alive during slow approval; only terminal success or refusal completes it.
- Progress lives inside the existing message event's `waitUntil` lifetime and stops when the handler exits.
- A 120-second worker deadline bounds the complete approval, including initial locks, key import and status publication. The unfinished Rust future is dropped before returning failure, preventing its later success reply. This does not roll back already committed registration facts or cancel already issued browser/network operations.
- Stage timings cover approval, registration push/fallback and receiver hydration without logging account identifiers, callback URLs or grants. Existing successful approval logging no longer prints its grant-bearing callback.

## Reproduction and verification

The disposable fixture serves the actual custody channel module and a service worker that takes 35 seconds. It uses no accounts or credentials. It proves terminal MessagePort delivery, not an external OAuth callback or full device sign-in.

| Browser / state | Original protocol | Progress protocol |
| --- | --- | --- |
| Isolated Chrome / visible | timed out at 30.015 s | completed at 35.006 s |
| User's Helium / hidden tab | timed out at 30.569 s | completed at 35.018 s while still hidden |

Headless tab switching did not produce a hidden document; only the Helium results establish hidden-tab behavior. All disposable Helium tabs and isolated Chrome were closed, and both fixture servers stopped.

- Rust/Wasm identity regression failed before the fix (progress was mistaken for completion), then passed. Final identity tests: 3 passed.
- Final Rust/Wasm approval deadline tests: 2 passed, including cancellation before late success and preservation of terminal results/errors.
- Device-registration Rust/Wasm fixture: passed; small offline registration took 31 ms (18 ms describe, 9 ms push attempt, 2 ms unconfigured fallback). This is not a real-account benchmark.
- Full Node UI/service-worker suite: 186 tests passed, zero failures.
- Worker Wasm compile passed with existing warnings.
- Final formatting and whitespace checks run after the last edit.

Commands:

```sh
nix develop . -c cargo test -p tonk-identity --target wasm32-unknown-unknown install::tests -- --nocapture
nix develop . -c cargo test -p tonk-worker --target wasm32-unknown-unknown deadline_tests -- --nocapture
nix develop . -c cargo test -p tonk-worker --target wasm32-unknown-unknown it_registers_a_device_as_account_space_facts -- --nocapture
node --test rust/tonk-ui/tests/*.test.mjs
nix develop . -c cargo fmt --all --check
node rust/tonk-ui/tests/fixtures/custody-channel-server.mjs
```

## Remaining latency question

The completion defect is reproduced and fixed locally. The underlying minute-long real-account operation has not been attributed to a measured stage. Do not claim a faster full sign-in from these results.

Production logs confirm hidden idle-sync backoff at 60, 120, 240 and 480 seconds, and repeated standard-library evaluation around 1–2 seconds. Source tracing shows device registration directly awaits push and, on any push error, a serialized full account sweep. That direct path does not simply wait for the hidden idle-sync timer. Changing background cadence alone is therefore not a justified fix.

Next integration checkpoint: deploy the request-scoped progress base and this patch through the normal release workflow, then capture one fresh approval using `link-timing` logs, with the page visible and hidden. Compare `custody-read`, `register-device`, `push-account`, `fallback-sweep` and receiver `hydrate-account` before removing work or narrowing fallback behavior. Verify the exact controlling worker build and actual native callback / ChatGPT token + tool use separately. The production version endpoint alone does not identify the worker controlling an already open page.
