# Fresh space home updates

The space shell mounts `model=tonk/space`. The library points that name at
`tonk:blank`; CLI view authoring auto-surfaces the first view by superseding
`id:tonk/space`. The display currently resolves the name once and subscribes
only to the resolved concept, so it never notices the changed home alias.

Implementation contract:
- Reproduce a home alias supersession on a mounted display in the existing
  browser test harness.
- Keep a live Name subscription for named models; restart model/view/entity
  resolution only when its referent changes. URI models need no name watch.
- Cancel the watch with the display lifecycle and invalidate stale async work.
- Verify first content and subsequent updates without remounting the display.

Completed:
- Added a live Name watch to named models, cancelling it with the other
  subscriptions and restarting only when the referent changes.
- Invalidated old downstream setup before resolving the replacement model.
- Regression checks the actual concept query target, unchanged alias snapshots,
  the blank-to-content transition, and subsequent rendered text updates.

Fresh validation:
- Before the fix, `cargo test -p tonk-display --target wasm32-unknown-unknown
  it_follows_the_space_home_alias_without_a_reload -- --nocapture` failed:
  the model subscription count stayed at 1 instead of becoming 2.
- After the fix, the focused regression passed.
- `cargo test -p tonk-display --target wasm32-unknown-unknown`: 192 passed,
  0 failed, after the final source change.
- `cargo fmt -p tonk-display -- --check` and scoped `git diff --check` passed.
- Browser execution required sandbox escalation: without it, the local browser
  daemon did not become healthy within 30 seconds.
A fake-host browser regression covers rendering and subscription lifecycle;
real CLI-to-hosted-service sync is a separate validation boundary.
