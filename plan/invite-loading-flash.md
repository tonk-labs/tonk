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
