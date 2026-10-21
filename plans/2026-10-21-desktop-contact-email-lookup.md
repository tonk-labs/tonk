# Desktop Add Contact email lookup

Base: desktop redesign PR #1134, `5d08b701bf`. Scope: resolve email in the
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

Checkpoint 2 in progress. Integrated focused run currently passes all 11 tests
(`EmailContactLookupTests`, `ContactLookupTests`, `AccessModelTests`). Storybook
build and 184 local link checks pass. Native app inspection remains pending.
