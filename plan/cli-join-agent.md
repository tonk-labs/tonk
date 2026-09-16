# Explicit agent mode for join

## Contract

- `tonk join URL --name NAME` keeps ordinary joining under the current identity.
- `tonk join --agent URL` runs the existing account-scoped agent handoff.
- `tonk --space NAME join --agent` resumes an interrupted handoff.
- `connect` remains a hidden compatibility command with its existing flags.
- New copied prompts, help, and recovery instructions use `join --agent`.
- Account switching still requires exact consent and browser approval. Building
  remains directed by the copied prompt and the user's request.

## Work

1. Add explicit parser/dispatch mode and prove its flag boundaries and compatibility.
2. Update copied prompts, recovery text, and documentation with their contract tests.
3. Run focused CLI/handoff and prompt tests, formatting, and Storybook checks.

## Verification

- Parser and help unit tests: 37 passed.
- Handoff integration tests: 11 passed (including both agent command spellings,
  invite rejection before local mutation, rendered prompt, and receipt sync).
- Accountless join: 1 passed.
- CLI help process tests: 5 passed.
- Handoff/account library tests: 6 passed.
- Formatting and diff whitespace checks passed.
- Storybook build/freshness and 176 local links passed.
- Worker contract test for both copy prompts: 1 passed.
- Parser/help tests rerun after final help wording: 37 passed.

All three implementation steps are complete. Changes are uncommitted.

Live browser approval, browser clipboard E2E, and published CLI verification
have not been run. Browser test sources now exercise `join --agent`.
Publish a CLI supporting `--agent` before serving the new prompt assets.
