# Tonk UI account error audit and implementation plan

Status: complete

Source audit pinned to `f17d431c0` on 2026-08-29. The implementation scope is
the user-visible account, passkey, registration, activation, device,
browser/CLI handoff, and account-deletion surfaces owned by `rust/tonk-ui`.
Generic editor, analyzer, and rendered-space diagnostics are outside this
account-focused slice because they have different recovery actions and stable
structured error contracts.

## Observed failure

Creating an account and opening `/settings` before confirming the email can
leave this notice visible:

> Account state is not synchronized yet. Reload /settings to retry before
> changing your account name.

The account is intentionally unhydrated until the access service accepts the
emailed activation link. On load, `account_status` paints the generic notice
before `customer_state` has classified the reason. If the customer row has not
settled yet, `load_activation_notice` dispatches re-enrollment but renders the
old null state once and exits. It neither polls the command result nor replaces
the generic notice. Reloading cannot satisfy the actual prerequisite.

Root cause trace:

```text
generic synchronization warning remains visible
<- registration renderer receives a null customer status and exits
<- re-enrollment dispatch returns before the resulting customer fact settles
<- settings renders the unhydrated fallback before activation state is known
```

The existing browser test covers the standing registration ceremony and a
failed display-name edit, but not a fresh navigation to `/settings` during this
settling window.

## User-facing error contract

Every message in this scope must answer, in ordinary product language:

1. What did not complete?
2. What state or work is still safe, when a partial commit matters?
3. What should the person do next?

Routes, HTTP methods/statuses, JSON/error kinds, DIDs, credential identifiers,
UCAN/invocation/delegation terminology, hydration, and JavaScript exception
text are diagnostics, not UI copy. Exact diagnostics remain in the browser
console. A user-facing message may name a passkey, account, email verification,
device, browser profile, settings, activation link, connection, reload, or
retry when that is the actionable product concept.

Passkey errors require three distinct outcomes:

- cancellation or timeout: retry and complete the prompt;
- unsupported PRF/security capability: use another passkey or device;
- unavailable or malformed ceremony integration: reload, then use another
  supported browser/device if it persists.

Remote/API failures use the operation that failed to choose the recovery. They
must not echo an unclassified response body. Service messages explicitly
curated for an account conflict or precondition may remain visible.

## Audit inventory

| Surface | Current leak or ambiguity | Required result |
| --- | --- | --- |
| `/settings` load | generic synchronization warning can outlive the state probe; `account_status` errors render verbatim | Pending email names the activation link; unexpected load failures say connection/reload; unsafe authoritative edits remain disabled. |
| Create/log in dialog | composed ceremony strings and browser rejection text render verbatim | Passkey cancellation, unsupported authenticator, account conflict, remote failure, and local-save-after-remote-success each have distinct recovery. |
| Registration support | email lookup and subscription failures can leave a blank ceremony; display-name and clipboard failures can claim success | Say connection/retry for lookup/options; route a failed activation watch through settings; preserve account readiness when the initial name fails; never claim an invite was copied when clipboard writing failed. |
| Display name and profiles | roster/API diagnostics render beside the field | Rename failure says retry; unavailable shared state says verify email; profile-list failure says reload without implying the account was lost. |
| Summary and devices | fetch/identity diagnostics render verbatim | Settings facts/device list say they could not load and offer reload; local account remains visible. |
| Add passkey | identity, custody, and API internals render verbatim | Explain cancellation, unsupported device, or incomplete passkey addition and whether retry is safe. |
| CLI handoff | callback/API diagnostics and callback-supplied messages can render verbatim | Incomplete/unsafe link says restart from the terminal; delivery failure says the CLI was not linked and to restart there. |
| Sign out/profile switch | API diagnostics render verbatim | Say the action did not complete and whether to retry/reload; never imply remote revocation. |
| Device revoke | passkey/API diagnostics render in the destructive dialog | Say no access was removed on pre-commit failure; cancellation remains retryable; stale target asks for refresh. |
| Account/space deletion | plan, passkey, and mutation diagnostics render verbatim; incomplete result is technical | Say nothing was deleted before authorization; response uncertainty asks for settings refresh before retry; partial result names completed/remaining work where available. |
| Activation page | unknown service message is rendered verbatim | Preserve explicit missing/damaged/expired outcomes; unknown failures say activation did not complete and to retry/check connection. |
| Custody consent | passkey/identity error renders in a transient card | Say account backup was not enabled and how to retry from settings; keep the exact cause in the console. |

## Implementation slices

- [x] Add a pure account-action error presenter with table-driven tests. It
      accepts an operation plus a diagnostic, recognizes cancellation and
      unsupported passkey capability, and otherwise returns an operation-
      specific fallback. It never returns an unclassified diagnostic.
- [x] Add the missing browser regression: create without activation, navigate
      to `/settings`, and assert the final notice names email verification and
      contains no synchronization, hydration, route, or HTTP vocabulary.
- [x] Keep the activation-state probe alive across the asynchronous
      re-enrollment settling window and disable display-name mutation while the
      account repository is unhydrated.
- [x] Route every dynamic error sink in the audit inventory through either a
      curated safe message or the presenter, while logging the exact diagnostic
      once at the point it is translated.
- [x] Add focused tests for passkey cancellation, missing PRF support,
      unavailable ceremony integration, API/transport fallback, and preservation
      of curated account errors.
- [x] Update Storybook journey/verification sources for `ACCT-B02`, `ACCT-B04`,
      `ACCT-B11`, `ACCT-B13`, `ACCT-B14`, `AUTH-03`, `AUTH-08`, `AUTH-09`,
      `LIFE-19`, and `LIFE-21`; regenerate Storybook data and validate links.
- [x] Run repository formatting, focused Tonk UI tests, the original browser
      reproduction, broader relevant checks, and `git diff --check` after the
      final change. Record any browser or infrastructure boundary that remains
      unverified.

## Verification record

- `cargo test -p tonk-ui --lib`: 21 passed.
- `cargo test -p tonk-fab --lib`: 109 passed.
- `cargo check -p tonk-ui --target wasm32-unknown-unknown`: passed; only the
  three pre-existing dead-code warnings for `focus_input`, `resettle`, and
  `register_claim` remain.
- `it_explains_email_verification_before_account_sync`: passed in Chrome
  152.0.7977.65 with ChromeDriver 152.0.7977.64. The first attempt was blocked
  before page load by the repository shell's ChromeDriver 150; rerunning with
  the matching temporary driver exercised the behavior.
- `it_names_pending_activation_consistently_in_a_space`: passed with the same
  Chrome/ChromeDriver pair, covering the FABB banner and share action.
- `python3 scripts/build.py --check`: passed with 26 screens, 78 journeys, 114
  verification items, and 6 triage findings.
- `python3 scripts/check-links.py .`: 172 local references valid.
- Isolated headless Storybook inspection: `WEB-11` and `ACCT-B11` rendered the
  updated recovery contract and evidence with no console messages.
- `git diff --check`: passed after the implementation review; rerun in the
  final verification pass after this record update.
