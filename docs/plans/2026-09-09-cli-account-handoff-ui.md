# CLI account handoff rows

The link approval pane should use the Hub's separated row layout: heading,
terminal name, account, device, explanation, actions, and status. Keep ordinary
account settings on their existing sheet. Long identifiers must wrap on mobile.
The callback page should use the same column width, with its result before its
close action.

The Hub is an opaque-origin guest. Passkeys must still run in the top document.
Publish the reserved passkey seat through the existing page-effect registration
relay with a distinct custody-anchor reason; resize and scroll update that seat.
Only device authorization uses it; other custody prompts retain their placement.

Validation: check both changed Rust crates for wasm, exercise the anchor relay
with browser tests, and inspect the source-based layout at desktop/mobile sizes.
Actual hosted CLI authorization and physical passkeys require separate evidence.

Implemented the row styling, callback row order/width, and custody anchor relay.
The anchored prompt waits for its seat if the worker arrives first, avoiding a
flash in the floating position. The guest replaces the approval rows once the top-page prompt is ready. The
account bar remains visible. A distinct custody lifecycle reply restores the
approval rows on dismissal; it does not close the Hub registration menu.

Validation completed: Wasm check for tonk-ui and tonk-workspace; the focused
custody anchor browser test (including late anchor arrival, repositioning, and
unrelated floating prompts); the callback shell unit test; formatting and diff
checks. The browser pool initially timed out inside the sandbox and passed with
browser access. Source-based browser previews cover desktop and 320px mobile;
they do not exercise hosted authorization or a physical passkey.

The final desktop preview shows only the account bar and passkey confirmation.
Focused browser tests verify that approval and status rows are hidden during
confirmation, that its position matches the old approval screen, and that
dismissal restores the approval rows.

The desktop passkey message now uses the same 36px row height as the account
bar, heading, and action buttons. Text can still grow the message row if it
wraps. Browser measurements confirmed all four desktop rows are 36px.

CI regression: wrong-account handoff rejection timed out reading its status.
A focused browser reproducer confirmed that `tonk:custody-closed` erased the
worker's refusal after the passkey screen had hidden it. Closing now preserves
refused, failed, and completed ceremony results, while clearing temporary
waiting text. The new refusal regression and existing dismissal test pass.
