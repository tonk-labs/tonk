# Absent-space invite guidance

- Explain that a space address requires existing access; ask a member to use share > copy link and open that invite in the browser.
- Return home from the absent-space page and hide its FABB alongside its unavailable content.
- Remove the join paste form and its styles. Keep invite redemption; return bare /join visits home before custody.
- Preserve existing FABB and Storybook work.

Validation complete:
- `cargo fmt --all --check` and `git diff --check` passed.
- `cargo test -p tonk-worker --test standard_library`: 29 passed.
- `cargo test -p tonk-worker --lib invite_presence_tests`: 1 passed.
- `cargo check -p tonk-worker --target wasm32-unknown-unknown` passed with two dead-code warnings in unchanged cache.rs.
- Isolated Chrome markup preview: FABB hidden for no-model and restored for ready; home anchor points to /; 390px viewport has 390px document width. Preview omitted branding assets and the live renderer.
- Full live invite redemption and browser redirect journey were not run.
