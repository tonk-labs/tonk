# Member invitation graph

Replace the Fab member rows with a space-centred, directed invitation graph.
Keep the existing live membership subscription and add an independent invitation
subscription so missing provenance never hides a person. Join membership's
optional invitation reference to the recorded inviter. Draw founder edges from
the space; never guess missing inviter edges. Keep names, roles and the viewer
marker, and expose each relationship as accessible text.

Use a deterministic compact spring layout with a scrollable, keyboard-focusable canvas;
keep disconnected history visible and labelled. Test live resets, deltas,
retractions, late provenance, multiple generations, cycles, and small viewports.

Known data boundary: Invitation::from_chain currently records the first account
in a chain, so existing re-shares may be attributed to the original owner. This
UI visualizes recorded invitation history, not an authorization proof or a
complete list of active UCAN grants. Historical attribution repair requires a
separate proof-aware data change; do not infer identities from hop positions.

Status: graph UI implemented and locally verified. Invitation-history accuracy
remains bounded by the existing provenance records described above.

Validation:
- 130 native tonk-fab unit tests passed.
- 37 browser tests passed: member_panel (4), space_name_element (20),
  responsive_overflow (13). Includes delayed panel centring and live edge removal.
- Captured rendered DOM inspected in isolated Chrome, including 390x844;
  no page overflow. This is fixture evidence, not a real multi-account join.
- cargo fmt --all -- --check and git diff --check passed.
- Strict Wasm Clippy blocked by eight existing findings in agent_panel.rs,
  bar.rs, dialog.rs, element.rs and tool_connection.rs; none in changed code.
- Real synced multi-account invitation chains and historical attribution repair
  remain unverified. No changes to authorization or stored invitation records.

## Gooey presentation refinement

Removed the explanatory copy. Followed the members mock in
`/Users/jackdouglas/tonk/gooey/fabb/fabb.html` and its `fabb.js` component:
240px stage, 18px person discs / 24px viewer disc, labels beside marks,
thin directed edges, member count, drag panning, and selectable details.
The space remains central as requested. Details use known roles and invitation
provenance; the mock's activity and agent-count examples are not fabricated.

Fresh validation: 130 native tests and 37 browser tests passed, including pointer
dragging, selection/deselection, and live edge changes. Captured rendered DOM
inspected at desktop and 390x844. Formatting and diff checks passed. The eight
pre-existing strict Clippy findings remain outside the changed code.

## Peripheral scaling

Added Gooey's elliptical smoothstep lens: full size within the central half,
falling to 40% at normalized distance 1.1. Glyphs and labels scale around their
fixed map positions, and arrow endpoints follow the scaled disc radii. Updates
run during dragging, native scrolling (wheel/keyboard/focus), zoom, resize,
selection detail changes and live redraws. The scroll listener is detached with
the subscribing element.

Focused verification: four native graph tests and five member-panel browser
tests passed. The browser test measures actual rendered disc widths, confirms
edge attachment, restores centre size after scrolling back, and covers immediate
drag updates plus live redraws. Existing unrelated strict Clippy findings remain.
