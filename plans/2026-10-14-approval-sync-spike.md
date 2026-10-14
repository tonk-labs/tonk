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
