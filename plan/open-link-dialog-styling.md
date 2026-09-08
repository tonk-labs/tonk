# Open-link dialog styling

Gooey has no dedicated open-link mock. Its `fabb/fabb.html` confirmation
cluster and the current `tonk-fab/src/dialog.rs` establish the reference:
light FABB surfaces, a bottom-right heading, aligned blocks, and fused actions.

The isolated browser reproduces the old dark palette from `open.css`. It does
not reproduce the screenshot's uneven block widths with the current checkout.
Use FABB tokens, explicitly stretch the blocks within the 432px border-box
column, and keep long destinations in one scrolling body. Preserve native
modal behavior and the existing trusted text rendering and link actions.

Validation: isolated browser using the production stylesheet and equivalent
dialog markup; check desktop, narrow/short viewport, long URLs, both system
schemes, keyboard focus and Escape. This does not validate a deployed build.

Verified: desktop column is 432px with equal-width children; 390px dark-system
preview retains the same light palette; at 320x240 a 1,458-character URL scrolls
vertically without horizontal overflow and both 44px actions remain on-screen.
Tab moves from Cancel to Open; Escape closes the native dialog. Screenshots
inspected at desktop and 390px. `git diff --check` passes. No Rust or link-action
code changed; deployed rendering and actual navigation were not exercised.

User adjustment: restored the 144px button widths and reduced button height
to 36px. Updated the standalone preview; the earlier 44px measurements above
describe the previous version. `git diff --check` passes after this adjustment.
