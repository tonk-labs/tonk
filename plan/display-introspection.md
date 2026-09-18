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
| Alt + click the display, or click the pin | pin the observation; survives Alt release |
| do it again | release it |
| Escape | release everything |

Alt-click is swallowed in the capture phase, since a click on a button you are
inspecting must not also dispatch the command it carries. The claim is narrow:
the listener returns before touching the event unless the overlay is *already
tracking* a display, so with the hood closed the gesture is the page's.

Reaching the chrome takes two things working together, and the first shipped
without the second, which made pinning unreachable in practice:

- pointer events retargeting to the overlay host never reach the machine, so
  resting on the pin or the panel is not "leaving";
- the page *between* the display and the panel is not a display either, so the
  machine holds its target for `LEAVE_MS` after the pointer leaves everything.

The pin also sits *inside* the outline's top-left rather than above it, so the
walk to reach it does not cross page at all.

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

This was not true as first written: the handler read `altKey` into the machine
but ran `closest("tonk-display")` before it, so every mousemove on every page
walked the DOM whether or not anyone was inspecting. The guard is now the
handler's first statement, and everything that touches the DOM sits below it.
The condition is `!alt && !painting` rather than `!alt`, because a pinned
observation has to keep tracking with Alt up, and a tracked one has to be able
to stop when Alt goes up under a still pointer.

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
- [x] **3. Concept panel.** One row per field either side knows about, whether
      or not it rendered, each classified by `inspect::Status` — the four
      answers to "why isn't my value showing up?" that all look like the same
      blank space on the page. Rows carry the declared type and cardinality and
      the value as the renderer spelled it (routed through
      `render_segments_with_shadow`, so the panel cannot report a spelling the
      page did not use). Resting on a row brings the slots it feeds forward and
      dims the rest. In directory mode the panel follows the `data-this` the
      repeat stamps on each row, stickily, so walking off a card towards the
      panel keeps the panel on the card you came from. The corner readout is
      gone: everything it said is a row now.
- [x] **4. View panel.** A second tab showing the mounted template
      (`Slide::display`), with every `{field}` and command-bound attribute
      value marked. `introspect::source::pieces` cuts the text using
      `tonk_template::scan::walk`, the analyzer's own lexer, so the panel and
      the build cannot disagree about what is in a template — a `{field}` in a
      comment is prose and one in a `<style>` body is a CSS brace, in both.
      Unlike `fields::scan`, which keeps one earliest offset per name for
      diagnostics, this keeps every occurrence, because the panel highlights
      all of them.

      Highlighting is keyed on the field name in both halves rather than on
      slot ids, which is what makes it two-way: a concept row, a marked span
      and a page marker all name the same thing. Command spans key on the
      command, so they light their interaction markers the same way.

      The command test deliberately mirrors `preprocess::strip_on_prefix`,
      ambiguity included: that treats any `on<ascii-alpha>…` attribute as a
      candidate binding and says so in its own comment, so `once="yes"` really
      is a handler to this renderer and the panel marks it as one. A panel that
      quietly disagreed would be nicer and would send an author looking for the
      wrong bug.
- [ ] **5. Edit.** (the only step left) A concept field edited in the panel becomes a transaction;
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
- **The panel sits bottom-right and takes pointer events.** A display under it
  cannot be hovered while it is open. Moving it, or letting it dock, is
  outstanding.
- **The snapshot is rebuilt every 30 frames** while observing, so a row
  appearing shows up within half a second rather than immediately. Positions
  are recomputed every frame. Slot *values* also only refresh on that cadence,
  which does not matter yet because nothing displays them — step 3 will need it
  tightened.
- **Command enumeration walks every element** under the display on each
  rebuild. Painting is bounded by the marker cap; the walk is not.
