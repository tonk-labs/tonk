# Remove the retired noun from public surfaces

## Stack

- Parent: `fix/user-facing-space-terminology` / PR #826.
- This branch changes only the remaining public diagnostics and documentation.

## Scope

- Sanitize parser errors for retired command and option spellings.
- Keep rejecting retired environment variables without repeating their names.
- Replace the retired noun in rendered Storybook and generated Rust documentation.
- Preserve machine-required compatibility literals, migration filenames, and tests
  that prove old inputs are rejected or converted.

## Verification

- [x] Focused CLI rejection tests prove public errors use only current terminology.
- [x] Rust formatting and affected crate tests pass.
- [x] Rustdoc builds for affected public crates; unrelated existing broken-link warnings are recorded.
- [x] Storybook data, links, and stacked-base impact checks pass.
- [x] A final audit classifies every remaining literal as compatibility or test-only.

## Verification note

Normal Rustdoc generation succeeds for all affected public crates. A strict
`-D warnings` run stops on existing broken intra-doc links, beginning with
`tonk-host/src/lib.rs` linking to `resolve_with`; those unrelated warnings are
outside this terminology-only stack.
