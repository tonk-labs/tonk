# Agent handoff

Smallest flow: copy hidden agent instructions from an empty space, run one
`tonk connect` command, approve account linking in the browser if needed,
join/pull the originating invite, then push an acknowledgement into that space.
Reuse the current browser callback ceremony; no OTP service for this iteration.

The receipt is ordinary replicated data and means a connection succeeded, not
that an agent is currently online. Keep it visible in the workspace shell even
when the agent replaces the empty home. No heartbeat, expiry, or multi-agent roster.

An already-active CLI account is reused and printed, not switched automatically.
The invite selects the space independently of that account. For the intended
same-account walkthrough use an unlinked CLI and approve from the source browser.
Invites retain their existing reusable bearer semantics, not one-time semantics.
Omit --name to derive a local alias from the pulled space name. Retry after a failed join/pull with explicit
`tonk --space NAME connect` to finish confirmation on the retained local space.
No cleanup or destructive reset is part of this flow.

Validation: focused CLI receipt test, standard-library lowering, CLI parser,
formatting and Storybook checks. Browser-to-native end-to-end remains a separate
manual check unless executed and recorded below.

## Walkthrough

1. Serve this checkout's UI and library assets, then create a fresh empty space.
   Existing spaces retain their previously seeded library.
2. Copy the prompt and give it to the agent. It runs
   `npx --yes @tonk/cli connect INVITE`.
3. The CLI validates the complete invite before changing account authority.
   The built-in production page is used by default; no unverified invite data
   selects an account page. `--via URL` explicitly overrides it for trusted
   local or staging development. `--no-open` prints the approval URL.
4. If the CLI is unlinked, approve from the browser holding the intended account.
   Return to the original space tab after approval.
5. Check that “Agent connection confirmed” appears, then ask the agent to build
   something and verify that the acknowledgement remains after changing the home.

## Verification, 2026-09-07

- `cargo test -p tonk-cli --test handoff --bin tonk`: 35 passed.
  Includes real fresh-site standard-library seeding, receipt query/rendering,
  isolation from another space, prompt rendering, and CLI argument parsing.
- `cargo fmt --all -- --check` and `git diff --check`: passed.
- Storybook build check and links check: passed (168 local references).
- `target/debug/tonk connect --help`: passed.
- Not run: browser clipboard interaction, account approval through `connect`,
  remote sync arrival in the browser, and badge visibility after a home change.

## Interrupted connection correction

User transcript: Pi killed `connect --name agent-space-2` at 45 seconds after the
join/pull output. Its later synced status was incorrectly reported as connection
success. A live `query agent-connection --json` returned `[]` on that space.

Agent joins now skip the optional account-directory update before confirmation.
`tonk --space NAME connect` resumes the receipt step with a fresh pull and push;
no new space or account ceremony. Copied prompts require the explicit connection
success message, permit browser-approval time, and give the resume command.
Regression checks cover publishing the receipt upstream and refusing to assert
it when the pull fails, plus the explicit-space resume parser.

Verification after the correction:
- `cargo test -p tonk-cli --test handoff --bin tonk`: 38 passed.
- Formatting, diff whitespace, Storybook build and 168-link check passed.
- Ran `target/debug/tonk --space agent-space-2 connect` against the user's existing
  affected space: exited successfully with “Agent connection confirmed”.
- Read back `agent-connection`: one receipt with that status; rendered the stored
  receipt view from the same space successfully. The command completed its push.
- Did not inspect the user's live Chrome DOM; browser arrival remains unobserved.
- The global ~/.cargo/bin/tonk executable was not replaced; resume support is in
  this checkout's rebuilt target/debug/tonk until the development CLI is reinstalled.

## Synced naming

`connect` now defaults its local registration name from the pulled space's
RepositoryName. Normalize display names to CLI-safe aliases (Test Garden becomes
test-garden) and suffix collisions; --name remains an explicit override. The
copied prompt uses the current directory binding rather than hardcoding a name.
Automatic-name connections use an independent internal storage directory because
the real name is known only after pulling. Choosing an alias never changes the
shared name. Existing aliases are not migrated and later remote renames do not
rewrite local bindings.

Naming validation: `cargo test -p tonk-cli --test handoff --bin tonk` passed
40 tests. Formatting, whitespace, Storybook build/link checks and connect help
passed. No additional live account/space was joined for the naming change.

## Productionization, 2026-09-07

- The copied command uses the npm-distributed CLI through
  `npx --yes @tonk/cli`; resume and orientation commands do not assume a global
  `tonk` executable. It becomes executable from the live prompt only after a
  package release containing `connect` is published.
- The complete invite capability is parsed before browser approval or local
  site creation. An explicitly named orphaned site is also refused before the
  passkey ceremony.
- No invite field is trusted to select an account page before the account
  ceremony verifies its authority. Tonk's built-in production page is the
  default, and local/staging development remains an explicit `--via` override.
- If the first pull cannot supply `RepositoryName`, the joined site is
  registered under a stable DID-derived fallback name. The subsequent
  confirmation attempt may fail while the remote is unavailable, but its
  printed `--space NAME connect` command can resume the same retained site.
- Reusing the original invite matches its content-derived invitation identity
  against non-replicated local claim metadata and resumes that local space. A fresh invite to the
  same repository is not collapsed onto possibly stale or revoked authority.
  Unrelated unreadable registry entries produce warnings without blocking all
  new connections, and copied invitation rows in another repository cannot
  capture the retry. Supplying a different explicit `--name` deliberately
  reclaims the reusable invite as a fresh local replica when old authority is
  no longer usable.
- The worker standard-library test now asserts the human/machine boundary and
  production command instead of pinning the retired visible machine prompt.

Fresh productionization verification:

- `cargo test -p tonk-cli --test handoff --bin tonk`: 43 passed (35 CLI
  parser tests and 8 handoff integration tests).
- `cargo test -p tonk-cli --lib`: 199 passed with loopback/local-storage access.
- `cargo test -p tonk-worker --test standard_library`: 29 passed.
- `cargo test -p tonk-ui --features integration-tests
  it_authorizes_a_waiting_cli_from_the_browser -- --nocapture
  --test-threads=1`: passed with the real Nix browser fixture. The restricted
  first attempt could not allocate a loopback port; the unchanged escalated run
  passed.
- `cargo check -p tonk-worker -p tonk-ui`, `cargo fmt --all -- --check`,
  `git diff --check`, and `target/debug/tonk connect --help`: passed.
- Storybook build freshness passed with 26 screens, 79 journeys, and 117
  verification items; 174 local references passed the link check.
- Still unverified: a published npm package through a live deployed empty-space
  prompt, remote-offline process interruption/resume, live receipt arrival in
  the source browser, and receipt visibility after replacing the home view.

## Review corrections, 2026-09-07

Both P2 findings reproduced in focused regressions:
- Copying a fresh invitation row into an older replica caused retry matching to
  select that replica even though it had only claimed the first invitation.
- `_ Garden` normalized to `-garden`, which fails local alias validation.

Claims now save the invitation identity beside the local site after authority
installation, before pulling replicated content. Retry matching checks that
marker and the repository DID; replicated roster rows cannot capture a retry.
The marker contains no bearer URL or secret. Older sites without the marker
must resume with `tonk --space NAME connect`; they are not inferred from roster
rows or automatically migrated.

Generated aliases strip every leading non-alphanumeric separator, use `space`
when empty, and pass `space::validate_name` before registration. Regression
coverage includes missing roster rows, replicated claims, legacy marker absence,
mixed separators, empty/non-ASCII names, and digit-leading aliases.

Fresh review-fix verification:
- `cargo test -p tonk-cli --test handoff --bin tonk --test join_profile`:
  44 passed (35 parser, 8 handoff, 1 profile-claim tests).
- `cargo test -p tonk-cli --test handoff --bin tonk --lib handoff`:
  5 matching library tests passed; the binary/integration tests were filtered
  out by that name filter and ran unfiltered in the command above.
- Storybook build freshness and all 174 local links passed.
- `cargo fmt --all -- --check` and `git diff --check`: passed.
- No live browser round trip or hosted revoked-authority scenario was run.
  The retry regression reproduces the replicated-row decision locally.
