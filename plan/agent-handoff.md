# Agent handoff prototype

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
Use a fresh space and the development CLI: published npm builds lack `connect`.
Omit --name to derive a local alias from the pulled space name. Retry after a failed join/pull with explicit
`tonk --space NAME connect` to finish confirmation on the retained local space.
No cleanup or destructive reset is part of this prototype.

Validation: focused CLI receipt test, standard-library lowering, CLI parser,
formatting and Storybook checks. Browser-to-native end-to-end remains a separate
manual check unless executed and recorded below.

## Walkthrough

1. Serve this checkout's UI and library assets, then create a fresh empty space.
   Existing spaces retain their previously seeded library.
2. Make this checkout's `target/debug/tonk` available as `tonk` to the agent.
   The test build produced the binary; alternatively use `cargo build -p tonk-cli`.
3. Copy the prompt and give it to the agent. It runs `tonk connect INVITE`.
   The account approval page defaults to the invite origin's `/settings/link`;
   `--via URL` overrides it for development. `--no-open` prints the approval URL.
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
