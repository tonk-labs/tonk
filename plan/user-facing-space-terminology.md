# User-facing space terminology fast follow

## Goal

Remove the remaining user-facing uses of “spot” in favor of “space” without
renaming compatibility formats, wire vocabulary, or historical implementation
notes.

## Scope

- Update browser copy, accessibility labels, join/share failures, and CLI help.
- Update the existing tests at those copy boundaries.
- Refresh the matching Storybook verification row and CLI capture.
- Regenerate and validate Storybook data before opening a PR against `staging`.

## Verification

- [x] Focused Rust tests pass for the affected crates.
- [x] `cargo fmt --all --check` passes.
- [x] Storybook generated data and links pass their checks.
- [x] A final source audit finds no active user-facing “spot” copy.
- [x] The committed Storybook impact passes against the current `origin/staging`.
