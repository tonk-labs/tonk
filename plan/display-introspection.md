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
| Alt + hover a display | outline it |
| rest there past 300ms | observation on: slots boxed and labelled |
| move to another display | dwell restarts there |
| move off / release Alt | observation off |
| Alt + click | pin the observation; survives Alt release and moving away |
| Alt + click the pinned display | release it |
| Escape | release everything |

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
is the frame that receives the pointer — but it means a panel drawn in a small
nested frame is clipped by that frame. Hoisting panels to the top document
needs the `__tonkRuntime` window-message relay that the theme and press signals
already cascade through (`tonk-portal/src/bridge.rs`).

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
- [ ] **2. Observe commands.** Indicators on every element carrying an
      `on<event>`/`on:<name>` binding, labelled with the command it posts and
      the event that triggers it, bouncing when one actually dispatches. The
      DOM half is readable from `data-on<event>`; the event type behind the
      `on:` form lives on the resolved declaration in `events::delegate`'s
      `EventTable`, which needs surfacing. Dispatch hooks into
      `delegate::handle_event` where a binding wins.
- [ ] **3. Concept panel.** The matched concept's fields and the observed
      subject's values, read-only; hovering a row highlights the slots that
      read it (`Slot::reads` is already there for this). In directory mode the
      panel is per-row — the repeat already stamps `with=<this>` on each row,
      so the row under the pointer is identifiable.
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

- **An attribute slot has no region.** `with="main@{repo}"`, `html:hidden={x}`
  — there is nothing on screen to box. It borrows its element's rect, which is
  the closest honest answer, and the detail belongs in a panel.
- **A slot that rendered an empty string has no box** — a `Range` over an empty
  text node measures zero. It is hidden rather than drawn as a hairline, which
  means "the field is missing" looks the same as "the field is not there",
  exactly the case an author most wants to see. The readout's *unrendered*
  and *not on the concept* lines are the partial answer; a panel listing every
  slot including the empty ones is the real one (step 3).
- **Alt+click is taken** for as long as the overlay is mounted. Only while a
  display is under the pointer, but an app that wants alt-click loses it there.
- **The snapshot is rebuilt every 30 frames** while observing, so a row
  appearing shows up within half a second rather than immediately. Positions
  are recomputed every frame.
