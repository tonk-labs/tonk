# First nested browser adapter

`tonk-nested-barrier-browser.patch` is incremental: apply it after the existing
`tonk-query-checkpoint-transport.patch` on Tonk base
`7ff32b38c32d08c4fc00e4dd0757df4dfd7bee0a`, with the dialog-db candidate and local
Cargo overrides described in the README.

This patch adds test-only wiring. Real `tonk-display` and `tonk-view` elements
mount a parent template containing another display. The existing controllable
host fixture delivers model, view and entity frames separately for each element.
It now supports distinct model responses and retains each subscription consumer,
so a child's subscriptions cannot accidentally stand in for its parent's.

A test-only lifecycle probe distinguishes a frame arriving from completion of
awaited downstream subscription setup. The adapter walks the actual light-DOM
display tree, reads renderer introspection, and supplies the barrier model with
its input checkpoints. It requires connected, ready displays, completed setup
and all three checkpoints at each node. Held history and portals are rejected.
The fixture uses directly addressed models and static inline templates without
commands, fallback queries or arbitrary custom elements.

The browser test observes these boundaries:

1. The parent text is visible and its state is `ready`, but child entity delivery
   is withheld: no barrier result.
2. The child's entity frame renders `Child 2`, but its checkpoint is withheld:
   still no barrier result.
3. The child's checkpoint arrives: six input checkpoints are accounted for,
   three per real display.
4. Changing the child entity immediately invalidates the result while replacement
   data is pending. Disconnecting the parent also makes inspection unavailable.

The capture helper creates a new barrier from current state for each observation.
It does not maintain an ongoing production coordinator or validate old receipts
across observations. In particular it does not exercise an old subscription's
late callback after navigation. The contract's stale-ticket tests remain native
model evidence until an actual host/renderer epoch adapter is installed.

## Validation

With a Tonk-specific `CARGO_TARGET_DIR`, in the patched Tonk checkout:

```sh
nix develop --command cargo --config /tmp/tonk-overrides.toml test \
  -p tonk-display --lib --target wasm32-unknown-unknown \
  it_blocks_the_browser_barrier_until_nested_data_and_checkpoint_arrive
nix develop --command cargo --config /tmp/tonk-overrides.toml test \
  -p tonk-display --lib --target wasm32-unknown-unknown element::tests::hook
cargo --config /tmp/tonk-overrides.toml check \
  -p tonk-display --target wasm32-unknown-unknown
```

The focused browser test, all 28 display-hook browser tests, the non-test Wasm
check, Rust formatting and source whitespace checks passed. The incremental patch
also applies cleanly over the recorded transport patch. The test
uses real DOM rendering with a controlled host, not a real worker/SSE stream or
the native Mac application. Existing worker unused/dead-code warnings remain in
the browser test build. No dependencies, installed assets, public inspector
response or production renderer behavior are changed by this incremental patch.

The next slice is to carry a subscription/frame epoch through the host callback
options, reject retired epochs in the renderer, and hold/release a stale callback
after navigation in this fixture. Reusable tags alone cannot establish that
identity. Fallback, history, custom components and portals remain outside a
trusted generic receipt until each path is covered.
