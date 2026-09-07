# Bug triage

A consolidated list of likely defects and behavioral contradictions raised by
the storybook audit. Each entry is pinned to current source or an existing test;
none has yet been confirmed in a fresh running-product pass. The list exists so
the product team can decide whether to fix, document as intended, or replace a
stale design contract before implementation work begins.

## Summary

Four findings remain after merging related observations: two high and two
medium; `B-02` is fixed and kept for its history. `B-06` is gone with the
account service it described. The high findings share one theme: a
user-visible account transition can cross an irreversible authority or
durability boundary without a tested, monotonic recovery state. The medium findings make real service errors or
duplicate activation results ambiguous. Coverage gaps without a concrete wrong
behavior remain in the verification backlog rather than this file.

| ID | Title | Severity | Area | Decision needed | Issue |
| --- | --- | --- | --- | --- | --- |
| `B-01` | CLI login recovery does not span every account boundary | high | CLI account login | finish recovery contract | — |
| `B-02` | Duplicate-email account creation leaves an orphaned passkey | high | Browser account creation | fixed | — |
| `B-04` | Busy account pages leave navigation links operational | high | Browser account lifecycle | fix or require restart reconciliation | — |
| `B-03` | Browser account reads can hide service errors as JSON decoder errors | medium | Browser API/error UX | fix | — |
| `B-05` | Activation accepts concurrent duplicate submissions | medium | Activation page | fix | — |

## High

### B-01: CLI login recovery does not span every account boundary

- **Where the user meets it:** `tonk account login`, after the browser has
  registered/authorized the CLI but before the native process has finished
  persisting and hydrating the account.
- **What happens / what was expected:** The first implementation slice now
  checkpoints the exact callback generation before writing local grant, root,
  or provider projections, and retries converge from both `Activating` and
  `Active`. Two boundaries remain outside that monotonic transition. The
  browser registers the waiting device before callback delivery, but a dead CLI
  listener has no replay token or acknowledgement protocol. At the other end,
  hydration and the CLI's account-registry projection settle after the session
  lock is released, so concurrent logout can invalidate the generation while
  stale login work is still finishing.
- **Reproduce:** Add deterministic fault exits immediately after browser device
  registration and after each native write. Start login, approve it, terminate
  at the fault, then run `tonk account status`, `account login`, `account
  devices`, and `account logout` in fresh processes. Record duplicate/stale
  attachments, partial local records, and whether any command can safely finish
  or clean up the original generation.
- **Why (from the code):**
  [`account_session.rs`](../../rust/tonk-cli/src/account_session.rs) owns exact
  staging/finalization and cross-process transition locking.
  [`account.rs`](../../rust/tonk-cli/src/account.rs) deliberately hydrates only
  after Active so a slow remote cannot hold the exclusive transition lock.
  [`tonk.rs`](../../rust/tonk-cli/src/bin/tonk.rs) writes the outer account
  registry only after the library link returns. The browser-side registration
  and callback form submission remain separate operations in
  [`ui_account_settings.rs`](../../rust/tonk-workspace/src/ui_account_settings.rs). Existing
  [`account_interrupt.rs`](../../rust/tonk-cli/tests/account_interrupt.rs)
  correctly pins fresh restart before approval, when nothing is yet durable.
- **Severity:** `high`. A common account authorization can commit remote or
  local authority and leave the user without a proved recovery or cleanup path.
- **Decision needed:** `finish recovery contract`. Add a browser/service
  acknowledgement or replay protocol for registration-before-callback, and a
  settlement seam that atomically revalidates the Active generation while
  committing the outer registry projection. Cover both with deterministic
  barriers rather than timing-based process kills.
- **Raised by:** [account lifecycle](accounts/lifecycle.md#open-questions-and-verification),
  [browser/CLI handoff](accounts/browser-cli-handoff.md#open-questions-and-verification),
  [state model](foundations/state-model.md#open-questions-and-verification).
- **Status:** Partially implemented, not yet verified. The first recovery slice
  now stages the exact callback generation before compatibility writes, resumes
  `Activating` and post-promotion `Active` state without another browser
  ceremony, rejects contradictory active-plus-pending state, and clears legacy
  `Waiting` state on logout. Pre-callback remote registration, per-write fault
  injection, and concurrent login/logout settlement remain open.

### B-02: Duplicate-email account creation leaves an orphaned passkey

- **Where the user meets it:** A fresh browser chooses Create and enters an
  email that already owns a Tonk account.
- **What happens / what was expected:** Current behavior creates a WebAuthn
  credential before the account service reports the duplicate email. Retrying
  with another address creates a second credential. The current E2E test calls
  the first one an orphan and asserts credential counts of one then two. A
  completed earlier plan and regression described code-authenticated preflight
  before WebAuthn, yielding zero then one credential while retaining
  enumeration resistance. The two contracts cannot both be intentional.
- **Reproduce:** In one browser/authenticator, create an account for
  `existing@example.test`. Open a fresh profile, attempt Create with that email,
  inspect authenticator credential count, then retry with an available email and
  count again. Reload between attempts and inspect local root/profile state.
- **Why (from the code):** The submit path starts root/passkey creation before
  `complete_remote("/accounts", ...)` at
  [`ui_account_settings.rs`](../../rust/tonk-workspace/src/ui_account_settings.rs). The real-browser
  test deliberately expects the orphan at
  [`account_flow.rs:590-660`](../../rust/tonk-ui/src/account_flow.rs). The
  completed opposite contract remains at
  [`plan/account-creation-preflight.md:1-51`](../../plan/account-creation-preflight.md).
- **Severity:** `high`. A common recoverable input conflict performs an
  irreversible external passkey action that the user did not need, and current
  code/test contradict a previously completed hot-path invariant.
- **Decision needed:** the product call is made — zero credentials on a
  conflict — but only the registration dialog implements it. The address
  decides which ceremony runs there, so a known address offers sign-in without
  touching the authenticator. Enumeration resistance is kept by answering from
  one `EmailStatus` fact that also names `invalid` and `unavailable`, rather
  than by a separate code-authenticated preflight; the stale plan is retired
  with the code ceremony it described.
- **Raised by:** [account lifecycle](accounts/lifecycle.md#edge-cases),
  [journey `ACCT-B03`](journey-catalog.md#accounts-browser-lifecycle).
- **Status:** Fixed. `it_offers_sign_in_for_a_taken_address_without_minting`
  asserts a credential count of zero. The account panel's Create / Log in fork
  is gone: one "link an account" button raises the same dialog, so there is no
  longer a place to answer the question wrongly.

### B-04: Busy account pages leave navigation links operational

- **Where the user meets it:** Any slow account create, login, handoff,
  revocation, or deletion transition while the header/home link or another
  anchor remains visible.
- **What happens / what was expected:** Busy state disables buttons and inputs,
  but anchors receive only `aria-disabled="true"` and `tabindex="-1"`.
  `aria-disabled` does not prevent pointer activation, and no CSS or click guard
  blocks it. Navigation destroys the custom element and its asynchronous task
  after passkey/local/remote stages may already have committed. The user can
  therefore turn a normal slow hot path into an unlabelled partial account
  state.
- **Reproduce:** Delay the remote response after passkey/root or after remote
  account acceptance. Submit Create, then click the Tonk/home anchor while the
  button says it is working. Reload settings and inspect credential count,
  selected profile, root, provider attachment, remote account/device, and
  customer state. Repeat at login, handoff, revoke, and delete checkpoints.
- **Why (from the code):**
  [`ui_account_settings.rs`](../../rust/tonk-workspace/src/ui_account_settings.rs) disables buttons and
  inputs but only annotates anchors; account submit launches asynchronous work
  at [`ui_account_settings.rs`](../../rust/tonk-workspace/src/ui_account_settings.rs). The older
  account-preflight audit explicitly records that top-level navigation destroys
  the task at
  [`plan/account-creation-preflight.md:80-96`](../../plan/account-creation-preflight.md).
- **Severity:** `high`. A common pointer action during a common slow transition
  can leave authority or identity partially committed without a visible result.
- **Decision needed:** `fix or require restart reconciliation`. Prevent pointer
  and keyboard navigation while cancellation is unsafe, or make every stage
  durably restart-reconciled and label navigation as leaving work in progress.
  The safest design may combine both.
- **Raised by:** [account lifecycle](accounts/lifecycle.md#cancel-and-interrupt),
  [failure checkpoints](cross-cutting/failure-and-recovery.md#account-fault-checkpoints).
- **Status:** Not run. Source-audit finding at `a3f8670b1`.

## Medium

### B-03: Browser account reads can hide service errors as JSON decoder errors

- **Where the user meets it:** Initial settings load, customer-status load,
  root/account status load, or another account read when the local worker
  returns a non-success error envelope or non-JSON body.
- **What happens / what was expected:** Several reads deserialize the success
  type without checking HTTP status. A valid error envelope is not a valid
  `AccountStatus`/`RootStatus`, so the user can receive a generic local API JSON
  decode message instead of the worker's curated error code/message and next
  action.
- **Reproduce:** Make `/api/account`, `/api/customer`, `/api/identity/root`, and
  `/api/identify` return representative 401/403/409/500 envelopes and one
  malformed body. Load settings and record the visible errors. Compare with a
  write helper that branches on status first.
- **Why (from the code):**
  [`api.rs:420-442`](../../rust/tonk-ui/src/api.rs) and
  [`api.rs:501-521`](../../rust/tonk-ui/src/api.rs) call `response.json()`
  directly. The conversion at [`api.rs:31-36`](../../rust/tonk-ui/src/api.rs)
  wraps decoder text as `Error from local API`, while
  [`error.rs:5-23`](../../rust/tonk-ui/src/error.rs) has distinct curated account
  and structured sync variants that these paths cannot reach.
- **Severity:** `medium`. The operation is recoverable, but error UX becomes
  least informative precisely on account hot-path failures and can hide
  revoked/suspended/malformed distinctions.
- **Decision needed:** `fix`. Centralize status-first success/error decoding and
  table-test every response class before adding more endpoint-specific branches.
- **Raised by:** [failure and recovery](cross-cutting/failure-and-recovery.md#browser-response-contract),
  [journeys `ACCT-B11` and `ACCT-B12`](journey-catalog.md#accounts-browser-lifecycle).
- **Status:** Not run. Source-audit finding at `a3f8670b1`.

### B-05: Activation accepts concurrent duplicate submissions

- **Where the user meets it:** `/activate?ucan=...`, by double-clicking Accept
  and activate or activating again before the first request returns.
- **What happens / what was expected:** Each click spawns an independent request.
  The button is not disabled and there is no in-flight guard. Two responses can
  race; for a one-use invocation, a success and later unauthorized/expired
  answer can produce a confusing final combination of done panel and error.
  Activation should be one monotonic interaction.
- **Reproduce:** Delay `/ucan/`, double-click Accept, and count requests. Return
  success for the first and Unauthorized for the second in both response
  orders. Record the visible panel/error, customer status, receipt reports, and
  behavior after reload.
- **Why (from the code):** The button is an ordinary enabled element at
  [`activate.html:11-19`](../../rust/tonk-ui/src/activate.html). Its handler at
  [`activate.rs:127-193`](../../rust/tonk-ui/src/activate.rs) starts a new local
  task per click and never disables the button or rejects a second invocation.
- **Severity:** `medium`. Activation likely commits once, but the result and
  recovery can be contradictory on a critical onboarding step.
- **Decision needed:** `fix`. Disable/guard immediately, make success terminal,
  and treat already-activated replay as an idempotent success where the service
  contract permits.
- **Raised by:** [browser runtime](ui/routing-and-runtime.md#edge-cases),
  [journey `ACCT-B04`](journey-catalog.md#accounts-browser-lifecycle).
- **Status:** Not run. Source-audit finding at `a3f8670b1`.

## Not triaged as defects yet

- Zero focused tests in `activate.rs` and `custody_relay.rs` are coverage gaps.
  They become defects only when an
  observable mismatch is reproduced.
- Best-effort activation receipt reporting and best-effort provider detach on
  logout are current explicit contracts. Their recovery still needs tests, but
  the best-effort choice itself needs a product decision before being called a
  bug.
- Customer suspension, malformed local session, and service-worker update
  behavior remain unverified rather than known-wrong.
