# Login dialog anchor recovery

The anchored account dialog survives Hub remounts. Reproduced on staging in
isolated Chrome by replacing `ui-hub-account` under a temporarily hidden parent:
the real dialog changed from left=312, width=576 to left=0, width=0. Its rows
collapsed to 26px, and revealing the Hub did not repair the dialog.

Scope: preserve valid positioning while the Hub has no layout, and publish a
fresh anchor when the replacement bar becomes visible. Keep existing scroll,
tab suspension, and disconnect behavior. Preserve unrelated account work.

Validation:
- [x] Reproduce collapsed layout with staging's real components.
- [x] Browser regression for hidden remount and subsequent visible geometry.
- [x] Reject unusable measurements and observe bar layout changes.
- [x] Run focused browser tests, workspace browser suite, and formatting.
- [x] Verify the compiled component with production dialog markup/CSS in Chrome.

Results:
- Before the fix the new Wasm browser regression failed: the hidden remount
  published one registration request with zero geometry.
- `cargo test -p tonk-workspace --target wasm32-unknown-unknown --lib`: 80 passed.
  Includes hidden remount recovery, existing scroll/tab behavior, and preserving
  the profile-transition reload command when its opener has detached (without
  stashing an unusable anchor).
- `cargo fmt --all -- --check` and `git diff --check` passed.
- Compiled-component visual probe: the standing dialog retained width=576 and
  left=312 during the hidden remount (zero requests), then followed the revealed
  bar to width=432 and left=384, exactly 7px below it, without a scroll or resize.
  This probe used production dialog markup/CSS and a small registration relay
  adapter; the staging reproduction used the real cross-frame dialog bridge.
- Temporary example and browser/server probe removed after verification.

The reproduction isolates rendering; it does not exercise a real passkey login.
No deployment or Safari check was performed. Initial sandboxed browser/test
daemon launches failed; both succeeded after retrying with local browser access.
