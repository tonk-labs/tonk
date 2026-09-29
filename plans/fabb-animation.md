# FABB motion repair

Reference: `/Users/jackdouglas/tonk/gooey/fabb/fabb.html` and its `fabb.js`.

- Restore the 400 ms nearest-edge glide: the host needs its own easing token.
- Telescope the surface over a stable-width rail, preserving the circle and
  preventing intermediate widths from reflowing the rail contents.
- Verify rendered intermediate geometry in the existing Wasm browser suites,
  alongside native component tests and formatting.

Status: complete.

Validation:
- `cargo test -p tonk-fab --lib`: 127 passed.
- `cargo test -p tonk-fab --target wasm32-unknown-unknown --test responsive_overflow --test drag_snap`: 18 passed in headless Chrome.
- `cargo fmt --all --check` and `git diff --check`: passed.
- Isolated before/after CSS preview confirmed the original rail reflow and
  invalid host transition, and stable repaired geometry in both orientations.
- Browser runner required execution outside the sandbox to start its daemon.
  Motion fixtures explicitly wait for initial docking and configure orientation.

Full application, Safari, and device testing were not run.
