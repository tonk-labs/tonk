# FABB share interaction

Match `/Users/jackdouglas/tonk/gooey/fabb/fabb.html`: share opens copy link and a live member count; successful clipboard completion shows copied for 1.2 seconds before closing the share stack. Members opens a scrollable native dialog with names and viewer/owner labels. Preserve account gating and existing compact docking behavior.

Implementation checkpoints:
- Keep copy selection open and tie success feedback to clipboard completion.
- Replace inline roster with count and a roster-owned dialog; clear stale membership on space changes.
- Run focused native and browser Wasm checks; refresh Storybook if its generator includes these components.

Completed: copy waits for browser completion, reports rejection, and closes the share stack after the success pause. The member count opens an owned dialog with live names and viewer/role labels; space changes clear it and disconnect removes it. Storybook contract and generated data updated.

Validation: 113 native unit tests, 46 browser Wasm unit tests, and 3 responsive browser regressions passed. Formatting, Storybook generation/check, and 168 local links passed. Browser runner required local daemon access after its sandboxed startup timed out. Clipboard state tests use controlled browser promises; no live account/invite end-to-end journey or physical mobile-device check was run.

Follow-up: keep copy link nearest the bar in both opening directions, including after a direction change. Use direction-dependent row ordering to track the bar’s opening side.

Follow-up: recorded the live membership/name mismatch as Storybook B-07 (not fixed), including the unproved transition history. Members dialog now overrides its surface token with DESIGN.md card color #fcfbfb.
