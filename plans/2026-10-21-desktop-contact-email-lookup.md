# Desktop Add Contact email lookup

Base: desktop redesign PR #1134, rebased onto `92b1bfcb9e`. Scope: resolve email in the
existing Address Book, review its account DID, and save in the existing local,
per-account `address-book.json`. Manual DID entry remains available.

## Checkpoints

1. Add a native client for the existing deployment's public customer lookup.
   Confirm only HTTP 200 / Active documents whose email DID and account key
   agree. Keep pending, missing, suspended, and unavailable distinct. Encode
   path segments once and refuse redirects. Prove the HTTP contract with tests.
2. Add explicit lookup to Add Contact, invalidate stale results on edits,
   cancellation, and account switches, and retain email/directory/time alongside
   the pinned DID. Preserve legacy contacts and saved labels. Prove persistence
   and stale-response handling, then build and inspect the native UI.
3. Update collaboration documentation and publish a focused follow-up PR stacked
   on the redesign while it remains open.

No invitation email delivery or signup flow is added in this increment. Email
lookup is a directory assertion at a point in time; it does not grant access or
silently change an existing contact's key. PR #1145 remains a separate protocol
spike for the later invitation/signup flow.

## Evidence

Checkpoint 1: four `EmailContactLookupTests` passed with a real loopback HTTP
server, including normalized plus-address lookup, all service status branches,
malformed/mismatched documents, and redirect refusal. Run through
`bash desktop/scripts/dev.sh test:unit --filter EmailContactLookupTests`.

Checkpoint 2 complete at `9b14d56402` on the updated redesign base. The focused
11-test run passed, followed by the full desktop suite after rebasing:
170 discovered tests, 166 passed, four expected opt-in/environment skips (two
cold-stream diagnostics and two live Claude checks), no failures.

- `bash desktop/scripts/dev.sh test:unit` passed after the rebase.
- `TONK_SIGN_IDENTITY=- bash desktop/scripts/build-app.sh` built and verified an
  ad-hoc signed app after the rebase. No installation or release was performed.
- Scoped `swift-format lint --strict`, generated-source
  `node mcp/scripts/sync-desktop.mjs --check`, and `git diff --check` passed.
- Storybook build: 29 screens, 88 journeys, 125 verification items. All 184 local
  links passed.
- Native AX and screenshot walkthrough used a separately identified copy of the
  built app, isolated data/WebKit storage, the existing links fixture, and a
  synthetic loopback HTTP server. Confirmed plus-address resolution, immediate
  result clearing on edit, pending/missing messages, manual DID entry, and
  cancellation. The walkthrough caught clipped confirmation text; wrapping was
  fixed and checked again in the final rebased app. The redesign's newer inline
  label controls remain present.

The fixture deliberately has no authenticated account; Save remains disabled
there. Save/reopen, label retention, legacy decoding, and account isolation were
verified through the real AccessModel/file store tests, not a signed-in native
click journey. No production email was queried. Production-directory lookup,
signed-in native Save, cross-device sync, invitation delivery, and account
creation remain unverified or outside this increment.

Fresh-worktree setup initially lacked `esbuild`; `npm --prefix mcp ci
--ignore-scripts` installed the locked build prerequisites. No lockfile changed.
