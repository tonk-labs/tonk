# Approval responsiveness spike

Goal: connect Desktop to tonk.foundation without local approval reads queueing
behind network-bound sync. Base: main 55af716ef5.

## Increment

- Use shared application access for pull, push, and sync status publication;
  the reactor still owns branch synchronization and revision conflict handling.
- Split approval stage timing into lock acquisition and subsequent work.
- Exercise actual sync and pull routes against a deliberately stalled loopback
  remote, and require the identity route to respond while both remain pending.

The original diagnostic reproduced a blocked identity read twice; cancelling
only the queued pull immediately restored it. The specific blocking writer in
the original browser was not identified.

## Scope and validation

This is a contention spike, not a complete repair of subscription cancellation
or terminal-error persistence. No account data or credentials are reset.
The native regression passes (1 passed, 253 filtered); the pre-spike diagnosis
failed twice with the local identity route blocked. Browser-worker compilation
passes with existing dead-code warnings. Formatting and whitespace checks pass.
The regression also passes with its stack size explicit in the fixture, without
RUST_MIN_STACK (1 passed in 0.46 seconds).
Foundation live callback and Desktop attachment remain unverified.

Foundation deploys from main through the existing Deploy workflow. A PR preview
is a different origin and does not establish that the user's Foundation profile
or Desktop callback works.

## Second live attempt and callback-delivery increment

The fresh user-approved Foundation attempt completed local-root in 2 ms and
signed the grant within 856 ms. Account publication took 92,643 ms. Authorization
finished at 93,567 ms, then spent 26,439 ms waiting for `status-lock` and hit the
120,007 ms deadline. This directly identifies terminal status acquisition as
blocking delivery of an already-completed grant in that attempt.

The second increment schedules terminal status publication through the existing
`detach` helper and immediately returns the original completed custody result.
It does not skip registration, change grant contents, extend deadlines, or
reissue authorization. The diagnostic timings remain secret-free. A native
regression holds the write lock while success and failure are returned and
checks eventual request-scoped publication after release.

Both native regressions pass together (2 passed, 253 filtered, 0.40 seconds),
without special stack environment variables. The final browser-worker `cargo
check --locked --offline -p tonk-worker --target wasm32-unknown-unknown` passes
with existing dead-code warnings. Formatting and whitespace checks pass.
The snapshot used for local execution has byte-identical changed Rust files;
only its local worker-test optimization override differs from the committed
workspace. No lockfile or build-profile changes are included in the spike.

The old fresh retry's Desktop callback has expired. A new Desktop connection is
required for the eventual live test; do not reuse an earlier callback URL.
