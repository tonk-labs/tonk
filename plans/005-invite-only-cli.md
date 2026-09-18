# Plan 005: Invite-only CLI access

Status: implementation complete in the worktree; local verification recorded below (2026-09-18).

This supersedes the two-flow scope of plans 001 and 002 for the first PR.
PR #955 currently targets staging at head b7b4ecca7. Preserve that committed
implementation as the source for a later browser-linking PR. Preserve existing
local replicas and credentials, including the three untracked replica directories.

## Product contract

- `tonk connect INVITE_LINK` imports the single browser-generated scoped bearer
  format, retains its key and grants locally, pulls the space and confirms setup.
- An explicitly selected connection can resume using its retained credentials.
- No CLI account login, logout, deletion, device administration, account-space
  catalogue, account migration or ownership-adoption commands.
- No `tonk link`, browser space picker, terminal approval, delivery mailbox or
  terminal management in this first PR.
- Keep ordinary space-scoped UCAN verification and revocation, credential
  isolation, interrupted-import recovery and preservation of local edits.
- Local space creation and existing replica access need explicit compatibility
  coverage; removing account commands must not delete or reinterpret old state.

## Identity finding

The existing browser issuer proves its authority for each space operation and
delegates it to a fresh invitation key. The CLI imports that key and the proof
chains. This operates under authority granted by the UI account, but the CLI
signer is a distinct DID. Import does not create ordinary membership or confer
account-wide authority. Showing account identity for activity is a separate
attribution requirement and must not be implemented by exporting the account key.

The user prefers the UI account's display name in roster/activity, but accepts a
separate scoped key for this first PR. A browser approval round trip is not a
prerequisite for eventual verified attribution; that presentation work is deferred.

## Checkpoints

1. Remove CLI account and browser-link entry points; prove parser refusal and
   retained-state preservation, alongside successful invite import/resume.
2. Extract terminal-only browser, worker, protocol and service machinery from
   this PR while preserving shared invitation issuance and revocation code.
3. Update current help, guides and Storybook for one invitation workflow.
4. Run focused native/browser checks after final edits; record exact results and
   unrun gates here. Review the resulting diff against the live PR base.

## Evidence

- Live PR metadata and local HEAD agree at b7b4ecca7; base is staging.
- Worktree initially has no tracked edits; three generated replica directories
  are untracked and outside the edit scope.
- Existing `connections::import_at` mounts a main-only replica with a retained
  invitation credential. Browser `agent_connections::mint` issues exact space
  scopes using the browser profile's proof chain.
- Removed CLI account command dispatch, ownership adoption, account migration,
  browser linking, and automatic account provisioning/catalogue updates from
  CLI space creation, transplant, remote configuration and explicit sync.
- Removed terminal approval codecs, delivery endpoints/stores/migrations,
  browser picker and management, CLI terminal importer, and terminal-only tests.
  These remain recoverable in the original committed PR head. Old optional
  source metadata is readable for already-imported local connection replicas.
- Local replica/account compatibility internals remain: removing commands does
  not erase credentials or weaken existing legacy/scoped authorization guards.
- Native invitation protocol tests: 13 passed. Native worker invitation issuer
  and partial-revocation/restart tests: 4 passed.
- Wasm checks for UI, workspace and worker with connection invitations enabled
  passed; six existing worker dead-code warnings remain.
- First process pass found retired help expectations. The initial connection
  pass also hit sandbox-denied loopback access, a macOS/XDG fixture mismatch,
  and outdated receipt text. Localhost retry passed import/resume; snapshots now
  cover the whole isolated home and receipt tests use the actual UI wording.
- Four old browser tests requiring CLI account login/catalogue are explicitly
  historical/ignored. The current multiple-invitation browser test uses a retained
  legacy registry fixture instead of invoking removed account login. A fresh
  browser-to-browser account catalogue backup journey remains outside this work.
- The full CLI regression suite passed, including 227 library tests, 20 parser
  tests and 60 space-command tests. Two environment-dependent tests were ignored.
- Native UI integration-test compilation passed with invitation features enabled.
- CLI all-target/all-feature Clippy passed with warnings denied. The final
  focused space-command rerun passed all 60 tests after simplifying assertions.
- The final workspace Wasm check passed after updating legacy approval copy.
- All 34 standard-library contract tests passed after replacing one retired
  `tonk link` copy assertion; the default-feature worker reports dead-code warnings.
- Formatting/whitespace, Storybook generation/impact and 200 local links passed.
- Fresh live browser journeys, Safari, deployed service/CLI compatibility and
  hosted CI have not been run for this narrowed implementation.
