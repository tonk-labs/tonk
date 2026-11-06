# Agent connection feedback in the FAB

Replace the workspace's replicated receipt toast with a brief FAB confirmation.
Keep durable receipts for access management and CLI confirmation. Publish the
current invitation's receipt identity only in the worker's local overlay, and
watch that identity after the user copies the invitation in this FAB. Existing
receipts, other people's invitations, reopened spaces, and repeated frames stay
quiet. Snapshots establish history; only live assertions trigger feedback.

- [x] Publish the local receipt identity and prove session-scoped consumption.
- [x] Animate the FAB confirmation and remove the workspace toast.
- [x] Run focused native and browser checks; record validation boundaries.

The worker publishes the grant-specific receipt ID in its local overlay. The
FAB arms it only after a copy action in that component, consumes it once,
and drops pending feedback on navigation or unmount. The 3.2-second popup uses
the existing width transition plus a vertical reveal and respects reduced motion. It leaves panel and
collapse state intact. Explicit receipt inspection remains an ordinary list.

The popup takes its checkmark, 18px message type and 32px by 18px message
padding from the success-message treatment in `gooey/fabb/fabb.html` and
`fabb.js`. It expands within the FAB toward the page interior, preserves the
space name, and dismisses automatically without requiring acknowledgement.

Validation:

- All 132 native FAB tests passed.
- All nine agent-panel browser tests passed in headless Chrome, including an
  actual width transition, vertical popup expansion and collapse, automatic return, another viewer, repeated frames,
  reopening, delayed historical snapshots, copying retained invitations,
  navigation cleanup, and preserving the open panel.
- The worker overlay test passed using the FAB's exact wire queries, proving
  the local reference does not change durable content and disappears when the
  session overlay is cleared. The actual CLI acknowledgement shape is readable.
- The standard-library regression and both focused CLI receipt-rendering tests
  passed, retaining grant-specific inspection without automatic notifications.
- `cargo fmt --all --check` and `git diff --check` passed.
- Browser startup and worker profile creation initially failed at sandbox
  boundaries; the unchanged focused operations passed with required access.
- Full packaged browser-to-CLI sync and hosted CI have not been run.
