# Nested completion contract spike

`render-barrier.rs` is a dependency-free executable model, not renderer wiring.
It tests the completion contract needed after query checkpoint delivery. It is
intentionally not exposed through the inspector or the running app.

## What the renderer trace established

In the recorded Tonk transport candidate:

- `element.rs::handle_view_frame` sets `view_settled` when a view frame arrives.
  That flag alone does not establish completion of nested displays.
- `start_downstream` awaits opening view and entity subscriptions separately.
  The first frame can arrive before subscription setup returns.
- `spawn_default_view` performs a one-shot query after an asynchronous boundary.
  Its result has no checkpoint in the current protocol.
- Attribute changes and model changes have distinct `generation` and
  `downstream_generation` counters. Both protect asynchronous continuations.
- The history recorder can hold the visible frame while live input continues.
- Host callbacks currently identify display inputs by a reusable `tag`; the
  completion adapter must also identify the particular subscription/frame epoch.

These are boundaries the future adapter must cover. A nearby branch revision,
`view_settled`, or a quiet DOM does not cover them.

## Tested contract

Each tree, display generation, input epoch and outstanding task has a unique
opaque ticket. Replacing a display retires its subtree's tickets. Replacing an
input retires its previous ticket, even when its user-facing tag is unchanged.
A checkpoint from retired work is ignored.

A caller registers asynchronous work **before** scheduling or awaiting it. A
node is sealed only after its synchronous reconciliation has registered all
inputs, children and asynchronous work. Adding a child, input or task unseals
it. Completion requires all nodes to be sealed, all tasks finished and every
input to have a checkpoint. Finishing a parent task does not finish its child.
Removing or replacing a child also requires the parent to reconcile and seal.

A receipt is a snapshot of this tree version and a vector of input checkpoints.
It does not collapse different spaces/branches/session overlays to one revision.
Any tracked change invalidates an older receipt. Duplicate checkpoints do not
invalidate it; checkpoint-only advancement does, without adding mount work.
A receipt from another tree cannot be accepted.

Untracked fallback queries, errors, held history and opaque portals block
completion until that node restarts. An empty tree/input set is not proof.
This conservative model can later support explicitly proven empty-render cases.

## Validation

```sh
rustfmt --edition 2024 --check experiments/render-provenance/render-barrier.rs
rustc --edition 2024 --test experiments/render-provenance/render-barrier.rs \
  -o /tmp/tonk-render-barrier-tests
/tmp/tonk-render-barrier-tests
```

Seven native tests passed. One uses two channel-controlled pauses: the parent
starts a mount, the mount resumes and discovers a child, and the child receives
its checkpoint only after a second release. No receipt is available in either
pending interval. The other tests cover stale navigation acknowledgments,
subscription replacement, late child membership, checkpoint-only advancement,
unsupported paths and cross-tree identity.

This proves the state-machine contract under explicit lifecycle notifications.
It does **not** prove that the real renderer emits every notification, that DOM
updates/portals have finished, or that a DOM snapshot and receipt are captured
atomically. Peak memory and traversal cost are not measured.

## Next integration slice

Connect one parent plus one nested `tonk-display` to this contract in the browser
harness. Register tickets at subscription creation and before asynchronous mount
work; retire them on cancellation, navigation, row replacement and disconnect.
Pause the child's real input delivery and inspect both the DOM and barrier before
and after release. Repeat with navigation while paused. Keep fallback/history/
portal paths unsupported until each has its own provenance and completion proof.

Only after that adapter test should this model move into production renderer
code. The public inspector continues to report unavailable revision tracking.
