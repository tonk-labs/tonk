# Popping the hood on `<tonk-display>`

**Goal:** make the rendering pipeline visible at the point of use. Hold Alt
over a rendered view and see which concept it matched, which template it
mounted, which slot each `{field}` filled, and which of them just changed.

**Why it is possible at all:** everything the tool needs already exists in
memory. `<tonk-display>` keeps the resolved concept descriptor, the effective
facet, the last folded frame and the mounted slide's template text.
`<tonk-view>`'s `Renderer` keeps the binding plan (where every interpolation
landed) and a cache of the last string each binding produced. Introspection is
a read of that state, not a re-parse of the DOM.

**Why it cannot come from the DOM:** a rendered `with="main@repo"` does not say
which half was a field. A binding applied as a JS property (`Reflect::set`,
which is what a non-string value does) leaves no attribute behind at all. And a
text slot's value is indistinguishable from literal text once written. The
renderer is the only thing that knows.

## Shape

`<tonk-introspect>`, registered and auto-mounted by `tonk_display::register()`,
one per document. Inert until Alt goes down.

- `introspect/mode.rs` — the arm/observe/latch state machine. No DOM, so it
  tests under plain `cargo test`.
- `introspect/slot.rs` — what a slot is: its fields, their origin (concept
  field / `{this}` / `{dom.host/*}` / iteration key), where it wrote, what it
  last rendered. Plus the `Snapshot` a display reports. Pure, native-tested.
- `introspect/registry.rs` — thread-local `Weak` registries the elements add
  themselves to on connect, behind two narrow traits. This is how the overlay
  reaches element state without JS interop or a global element map.
- `introspect/overlay.rs` — the element: listeners, the rAF loop, the painted
  boxes.

`Renderer::describe()` walks plan and mounted tree in lockstep — the same walk
`update_nodes` does — and returns `(Slot, Option<Node>)` per binding.

## Interaction

| gesture | effect |
| --- | --- |
| Alt + hover a display | outline it, with a **pin** button above its top-left |
| rest there past 300ms | observation on: slots and commands marked |
| move to another display | dwell restarts there |
| move off / release Alt | observation off |
| click the pin (Alt still held) | pin the observation; survives Alt release |
| click it again | release it |
| Escape | release everything |

Pinning is a click on the overlay's own chrome, not a modifier-click on the
page. A modifier-click would have to be swallowed — inspecting a button must
never dispatch the command that button carries — and that takes the gesture
away from every app for as long as the overlay is mounted. The pin costs the
page nothing. `<tonk-introspect alt-click>` restores alt-click pinning for a
page that wants it.

Moving onto the overlay's own chrome does not count as moving off the display:
pointer events that retarget to the overlay host are ignored by the machine,
which is what makes reaching for the pin possible at all.

Alt state is read off the *pointer* event, not remembered from a `keydown`. A
sealed guest iframe that has never had focus receives no key events, but every
mouse event it gets carries `altKey` — so hovering works in a frame that was
never clicked, which is the common case. `keyup` is still listened for, as the
only way to notice Alt going up under a pointer that is not moving.

Alt-click is swallowed in the capture phase. Inspecting a button must never
dispatch the command that button carries.

## Frames

A `<tonk-display>` renders inside a sealed guest iframe, and a nested
`<tonk-site>` opens more below it. Events do not cross those boundaries and
neither does hit-testing, so each frame runs its own overlay over its own
displays. That falls out correctly for hovering — the frame under the pointer
is the frame that receives the pointer — and it means a panel drawn in a nested
frame is clipped by that frame. Accepted: sites run full screen, so the clip is
the viewport. If that stops being true, hoisting panels to the top document
would go through the `__tonkRuntime` window-message relay the theme and press
signals already cascade through (`tonk-portal/src/bridge.rs`).

## Marking something with no extent

A slot that rendered an empty string has nothing to box, and that is exactly
the case an author most wants to see. So a marker is not always a box:

- **Extent** — the slot rendered glyphs. Box them.
- **Point** — the slot is empty. Tick the caret position it would have
  occupied: the trailing edge of the previous sibling, else the leading edge
  of the next, else the parent's content corner. `<p>Hello {name}</p>` with
  `name` absent ticks immediately after `Hello `, which answers "it would be
  here" rather than "it is missing somewhere".
- **Edge** — the slot wrote an element property (`with="main@{repo}"`,
  `html:hidden={x}`). Tick the element's top edge instead of filling it: the
  element is where the value went, but the element is not the value, and a
  filled box says otherwise.

Every marker carries a label badge whatever its placement, so an empty slot is
still named. Badges that would collide are pushed down and joined to their
anchor by a dashed leader. Badge width is arithmetic off the label length
(monospace at 11px), not a layout read, so the collision pass never forces a
reflow.

At most 160 markers are painted at once; the readout says when that bit.

## Cost when closed

One document `mousemove` listener per frame whose first act is to read `altKey`
and return, plus a `Cell<bool>` read on the renderer's change path. Nothing
else runs, no rAF loop is scheduled, and no snapshot is built.

## Steps

- [x] **1. Observe values.** The state machine, the slot description, the
      registry, `Renderer::describe`, the overlay: outline, slot boxes labelled
      by field and coloured by origin, change flash, a corner readout naming
      the concept, facet, mode, subject count, slot count, and the two
      mismatches worth seeing — concept fields no slot renders, and template
      fields the concept does not declare.
- [x] **2. Observe commands.** Every element carrying an `on<event>` or
      `on:<name>` binding is outlined and labelled `click -> space/create`, and
      bounces when it actually posts. The older form carries its trigger in the
      attribute name; the newer one names a declaration, and only the
      `EventTable` says which platform event that declaration reads — so
      `Delegate` now retains its table. A binding whose declaration did not
      resolve is drawn **inert** rather than hidden: it installs no listener
      and will never fire, which was previously invisible. The bounce is raised
      at the two points the winning binding is known
      (`delegate::try_binding`, `binding::resolve_binding`), because dispatch
      walks up until a binding resolves and the element that posted is not
      always the one clicked.
- [ ] **3. Concept panel.** The matched concept's fields and the observed
      subject's values, read-only; hovering a row highlights the slots that
      read it (`Slot::reads` is already there for this), and hovering a slot
      badge highlights the row. Lists every slot including the empty ones,
      which is the complete answer the Point marker only gestures at. In
      directory mode the panel is per-row — the repeat already stamps
      `with=<this>` on each row, so the row under the pointer is identifiable.
- [ ] **4. View panel.** The template source the slide mounted
      (`Slide::display`), read-only, with the slot under the pointer located
      in it.
- [ ] **5. Edit.** A concept field edited in the panel becomes a transaction;
      a template edited in the view panel supersedes the `show` facet. Both go
      through the ordinary transact path. This is where the real work is:
      superseding a cardinality-one field needs the prior value to retract,
      values need coercing back to their declared Ipld types, and a refused
      write needs somewhere to say so.

## Known limits

- **A caret position is a guess in the hard cases.** The sibling-edge walk
  covers `Hello {name}` and `{name} trailing`, but a slot alone in an empty
  block falls back to the parent's corner, which is the right area and not the
  right spot. A panel listing every slot (step 3) is the complete answer.
- **Badge collision is resolved by pushing down only.** Dense layouts still
  produce a stack of badges to one side of the thing they name, joined by
  leaders. Legible, not pretty.
- **The snapshot is rebuilt every 30 frames** while observing, so a row
  appearing shows up within half a second rather than immediately. Positions
  are recomputed every frame. Slot *values* also only refresh on that cadence,
  which does not matter yet because nothing displays them — step 3 will need it
  tightened.
- **Command enumeration walks every element** under the display on each
  rebuild. Painting is bounded by the marker cap; the walk is not.
