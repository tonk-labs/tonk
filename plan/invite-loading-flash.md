# Invite loading flash

The reported fresh-browser sequence is invalid link, loading pulse, space.
The supplied log shows successful redemption followed by the space route and
content loading. The invalid-link copy belongs to the space chrome's `no-model`
slot, not the join failure view.

The space chrome authors its light-DOM fallback slots without `hidden`. A local
Chrome fixture using that exact view markup reproduces a visible invalid-link
panel with no display state assigned. The portal's other lifecycle placeholders
already start hidden. Apply that convention to the space title display too;
the existing slot projection reveals the matching fallback when resolved.

Validation:
- Before change: isolated Chrome reports state=null, hidden=false, visible=true.
- Added a standard-library regression covering initially hidden fallback slots.
- Corrected markup in isolated Chrome: initial visible=false; manually projecting
  absence visible=true; manually projecting recovery visible=false. This tests
  markup and CSS, not the Wasm state handler.
- `cargo test -p tonk-worker --test standard_library --locked`: 30 passed.
- `cargo fmt --all -- --check`: passed.

This fixture isolates initial markup visibility. It does not replay the supplied
staging invite or establish whether a transient `no-model` frame also occurs in
that deployed build. No claim is made about the separate long commit timing or
guest script error in the supplied log.

## Follow-up: join failure wall (2026-09-08)

Frame inspection of the new recording shows `this share link expired` while
still on `/join`, before successful navigation. This is `tonk:join/failure`,
not the space chrome's `invalid link` fallback addressed above.

The failure template had no subject binding. When the display mounts a view it
replays its cached frame, including an empty frame before the entity query has
matched. The unbound wall renders as static chrome even without a failure row;
the join route's `:has(.edge-wall--closed)` rule then hides the opening pulse.
Binding the section with `data-id={this}` makes the whole wall conditional on
the failure row, following the join-status template's existing pattern.

Validation:
- The Wasm browser regression
  `cargo test -p tonk-display --target wasm32-unknown-unknown --locked it_renders_the_join_failure_wall_only_for_a_matching_row`
  uses the actual profile-library template. Before the change it failed with
  `a pending join must not render the expired-link wall`; after the change it
  passes empty -> matching failure -> empty, including removal on retry.
- The sandbox browser daemon failed to become healthy; the unchanged test ran
  with host access and produced the red/green results above.
- `cargo test -p tonk-worker --test standard_library --locked`: 30 passed.
- Full invite redemption from the recording has not been replayed; this
  regression exercises the real browser renderer at the failing frame boundary.
