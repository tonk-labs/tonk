# Passkey link UI refresh

Implemented 2026-10-08, based on the supplied screenshot and the current FABB
account panels in the adjacent gooey repository.

The approval screen uses a spacious card, inset definition-list details, a
full-width split action row, 48px controls, and narrow-screen stacking. It names
a Tonk connection rather than a terminal. The only details shown are connection
name and registered account email; the email uses the same reactive account
model as settings. Device DIDs and callback destinations remain internal.
Authorization data, callback validation, decline behavior, and custody overlay
handling are preserved. Existing handoff tests reflect the new visible contract.

Validation:

- Desktop light and mobile dark source-fixture screenshots reviewed, with a
  sample email. Final screenshots show only name and account.
- Earlier 320px fixture checks covered wrapping, touch targets, keyboard focus,
  and ceremony pane/status replacement and restoration.
- Formatting and diff checks passed. Focused handoff tests compiled with Apple
  Clang and SDK after ambient GCC failed in aws-lc-sys.
- Browser integration did not execute: the sandbox initially blocked loopback;
  the unrestricted retry reached a cold Nix test-server build, which was stopped.
  Live email resolution and actual passkey handoff remain unverified.

Storybook WEB-10 documents the final screen and its evidence boundary.

CI follow-up: web debug failed in the registry test that still expected the old
host-specific heading. Updated that test and the neighboring loopback test to
use shared connection wording and preserve the email supplied by the account
view, while retaining authority-data and decline-callback assertions. A focused
Node execution of the actual profile methods passed for loopback and HTTPS
requests; formatting and diff checks passed. Full Wasm test execution remains
for CI. The other test matrix legs were cancelled by fail-fast, not independent
failures. The unrelated macOS CLI cache-download failure passed on retry.
