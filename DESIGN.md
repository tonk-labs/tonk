# The tonk chrome — working spec for agents

Read this before you write or edit any HTML, CSS, or JS for tonk. The system is
non-standard on purpose: nearly every default you would reach for — vertically centered
labels, icon libraries, accent colors, rounded buttons, widths that follow content — is
wrong here. This file is the operational contract: the primitives, the exact numbers, and
the traps. The reasoning lives in [fabb/README.md](fabb/README.md) of the `gooey` repository
(laws + decision log). Those decisions are settled. Do not re-litigate
one in passing; reopening one is a deliberate act, with the owner, on the record.

## The rules everyone breaks — check your diff against these

1. **Everything chrome is 36px tall.** Cells, rows, buttons, headers, the sync circle.
   Tall rows (for notices only) are 56px. The touch target is 44px, delivered invisibly 
   (wrapper or padding), never by growing the block.
2. **Words sit bottom-right.** `align-items:flex-end; justify-content:flex-end;
   line-height:1`, padding **9px bottom, 10px right**. Never vertically centered. Only
   symbol-only marks (×, +, the discs) are geometrically centered. Two seats may leave
   the right edge — both still bottom-seated: a **two-sided row** holds a noun or name
   at its left end (`margin-right:auto`) while the actionable word keeps bottom-right,
   and a label may flush a **terminal edge it owns** (the account tab's name against
   the bar's left end; headers flush left). Left-flushing anywhere else is still wrong.
3. **Two Plex registers, never mixed up.** Labels are **IBM Plex Sans Condensed**
   (600, 13px, lowercase). Reading text — sentences, descriptions, dialog bodies — is
   **IBM Plex Sans** (normal, 400, 13.5px). Condensed sentences and non-condensed labels
   are both errors.
4. **One ink, no second color.** Stone ink `#38182A` and its derivations. No green, no
   red, no blue — not for success, not for destructive, not for links. Attention blinks;
   it never takes a hue.
5. **Edges are 1px ink.** A ring around every surface, a 1px separator between flush
   cells. Corners are square; a curve appears only on mute, symbol-bearing chrome.
6. **Gaps are 7px of pure page** between blocks in a stack — no drawn dividers there.
   Inside a bar the gap is 0 and a 1px line divides. Never both.
7. **Widths are fixed.** `36 · 144 · 216 · 288 · 432`. A cell never sizes to its
   content; breakpoints swap whole rungs in and out. The one sanctioned flex follows
   the **column**, never the content (the phone spaces rung, the signed-out fact cell,
   the gate's half-rung pair).
8. **No icon library.** Every mark is Unicode geometry or a few lines of bespoke CSS/SVG,
   from the tables below.
9. **Chrome is lowercase.** User words (names, spaces) pass through untouched.
10. **The chrome themes itself, never the view** — and no unprefixed CSS variable ever
    crosses the shadow boundary.

## The block

The whole system derives from one primitive: a **36px-tall box of frost with a 1px ink
ring, its word seated in the bottom-right corner**. Canonical CSS:

```css
.block{
  height:36px;                                   /* 56px for tall rows, nothing else */
  display:flex; align-items:flex-end; justify-content:flex-end; gap:8px;
  padding:0 10px 9px 16px;                       /* right 10 · bottom 9 · left varies */
  font:600 13px/1 "IBM Plex Sans Condensed","Bahnschrift","Arial Narrow",sans-serif;
  letter-spacing:.02em; text-transform:lowercase;
  background:var(--frost);                       /* or var(--frost-solid) on a flat page */
  box-shadow:0 0 0 1px var(--ring);
  border-radius:0;                               /* square. curves are punctuation */
}
```

Bars are blocks fused flush on one surface. Stacks are blocks separated by page. Modals
are stacks floating denser. Buttons are bar cells that left the bar. If you are building
a new surface and it does not start from this block, stop.

## The seat — how words sit

- **Vertical:** `align-items:flex-end`, `line-height:1`, **9px bottom padding**. With
  those three, the 13px label, the 14px triangles, and the 9px settings glyph self-align
  to one shared bottom edge. Do not center vertically, do not use line-height to fake it.
- **Horizontal:** `justify-content:flex-end`, **10px right padding**. The label's datum
  is a straight vertical edge (a cell divider, a rail's docked edge, the box's own right
  edge). Left padding is the entry side and varies by context: bar cell 0, hub cell 12,
  space row 16, menu row 22, button 24.
- **Inline gap** between a glyph and its word: **8px** (6px inside buttons).
- **Tall rows (56px):** column layout, everything still right-flushed and bottom-seated;
  metadata lines are 11px / 400 / soft, gap 4px, 10px top padding.
- **Two-sided rows:** a noun or name seats bottom-left (`margin-right:auto`) while the
  value or verb keeps bottom-right — the ceremony's step rows (`email · goblin@…`), the
  account page's `oat works · switch account ▸`. The left seat is for what a row is
  about; the right seat stays the actionable edge.
- **The exception:** symbol-only marks — ×, +, the discs, a lone settings glyph in a
  36px symbol cell — stay geometrically centered. They are punctuation, not words.
- A divided pair (glass half / ink half) is two boxes fused at gap 0; the fill boundary
  is the divider and each word seats against its own edges. No drawn line needed (the
  reminder banner; the gate's `continue without account | join` rung — two half-rung
  cells following the cluster).

## Type

| register | family | weight / size / leading | where |
|---|---|---|---|
| chrome label | IBM Plex Sans Condensed | 600 · 13px · 1 | bar cells, hub cells, buttons, dialog headers, CTAs |
| menu row | IBM Plex Sans Condensed | 500 · 13px · 1 | `tonk-mi` rows |
| triangles ▸ ◂ | chrome font | **500 · 14px** | riding a word — smaller or heavier goes stubby |
| reading text | IBM Plex Sans (normal) | 400 · 13.5px · 1.4–1.55 | dialog bodies, hub row descriptions, settings explainers |
| emphasized name in reading text | IBM Plex Sans Condensed | 600 · 13.5px | e.g. a space name inside a sentence |

Letter-spacing `.02em` on chrome. Chrome words are lowercase — authored lowercase;
buttons and the hub's `.chrome` class also enforce `text-transform:lowercase`. User words are never transformed (the rename
editor sets `text-transform:none`). No interpuncts in chrome; nothing competes with the
circle. Fonts are a **host concern**: `@font-face` cannot live in shadow CSS, so the
components only declare the stack — every host page must load IBM Plex Sans Condensed
(hub also needs IBM Plex Sans and IBM Plex Mono).

## Color — stone ink

One ink with a hue: **`#38182A`** (wine/aubergine, hsl 326·40%·16%). The scheme is
**stone·ink**: the ink is the brand, the ground is its own warm gray (hue 30) — related,
not derived. Values come from the five-scheme study (`schemes.css`, `purple-ink` branch);
the reasoning is `fabb/COLOR.md`.

**The roles, which matter more than the values:**

- **Solid ink = the CTA register.** The coloring is the action. Primary buttons, the
  `create new space +` row, the account page's `add account +` row, `create account`,
  the gate's account door. No glyphs on primaries.
- **Current = near-ink `--cur`** — the ink's hue, +10 lightness, slightly less
  saturation. A place you are in never outshouts a thing you can do. Applied to the
  hub's current tab (the account cell keeps it through settings — settings is an
  account place) and `tonk-mi[current]`. **Current marks places, never facts**: the
  signed-out `no spaces available` rung sheds it (soft on near-ink is unreadable;
  found Sep 1).
- **Soft ink = metadata only.** **An actionable word never wears soft** — soft on a
  clickable reads as disabled.
- **On-ink** for words on solid surfaces.
- **Alerts blink** (opacity 1 → .55, 2.4s); they never take a color. Pointing at a
  blinking thing calms it.
- Selection and tap-highlight are ink on on-ink — never the browser's blue. Focus is a
  2px ink outline, offset −2.

**Component tokens** (the chrome carries **one scheme — light**; law 8):

| token | value |
|---|---|
| `--fabb-ink` | `#38182a` |
| `--fabb-ink-soft` | `#5b4953` |
| `--fabb-on-ink` | `#f7f6f5` |
| `--fabb-cur` | `#552e44` |
| `--fabb-sep` | `rgba(56,24,42,.34)` |
| `--fabb-hover` | `rgba(56,24,42,.06)` |
| `--fabb-press` | `rgba(56,24,42,.12)` |
| `--fabb-bg` (frost rest) | `rgba(253,252,252,.72)` + `blur(12px) saturate(1.5)` |
| `--fabb-panel` (modal) | `rgba(253,252,252,.92)` — no blur at modal density |
| `--fabb-ring` | `rgba(56,24,42,.85)`, drawn as `box-shadow: 0 0 0 1px` |

**Hub tokens, light** (page grammar on top of the same ink):

| token | value | | token | value |
|---|---|---|---|---|
| `--page` | `#e8e6e4` | | `--wash` / `--wash-2` | ink `.06` / `.12` |
| `--ink` / `--soft` | `#38182a` / `#5b4953` | | `--wash-p` | `rgba(247,246,245,.16)` ⚠ see below |
| `--on-ink` | `#f7f6f5` | | `--canvas` / `--stub-ink` | `#e4e2e0` / `#9f968e` |
| `--cur` | `#552e44` | | `--veil` | `rgba(232,230,228,.9)` |
| `--ring` / `--sep` | ink `.85` / `.28` | | `--dim` | `rgba(56,24,42,.32)` |
| `--frost` / `--frost-solid` | `rgba(253,252,252,.72)` / `#f7f6f5` | | `--track` | `rgba(56,24,42,.22)` |
| `--panel` / `--card` / `--card-hover` | `#d0ccc8` / `#fcfbfb` / `#eeedec` | | `--modal` | `rgba(253,252,252,.92)` |

⚠ **`--wash-p` is a wash of on-ink, not of ink.** It is the hover for solid-ink
surfaces, and a 16% ink wash over an ink solid composites to exactly the resting color —
a no-op hover (measured; the wash-p correction, Aug 20). The stone·ink block in
`schemes.css` still carries the pre-correction ink-derived value — **do not copy it**.

**Hub dark twin** — the hub follows `prefers-color-scheme` and nothing else: no stored
mode, no stamped attribute, no switch. The chrome floating over a space stays light
either way (law 8).

| token | value | | token | value |
|---|---|---|---|---|
| `--page` | `#161313` | | `--wash` / `--wash-2` | bone `.09` / `.15` |
| `--ink` / `--soft` | `#e2dfdd` / `#c8c3bf` | | `--wash-p` | `rgba(34,28,29,.14)` |
| `--on-ink` | `#221c1d` | | `--canvas` / `--stub-ink` | `#1c1718` / `#736265` |
| `--cur` | regenerate via `schemes.gen.py` — same step toward the page from `#e2dfdd`; mono's `#cdc5c9` came from a different dark ink | | `--veil` | `#161313` — the page itself, no translucency in the dark |
| `--ring` / `--sep` | bone `.55` / `.28` | | `--dim` | `rgba(0,0,0,.45)` |
| `--frost` / `--frost-solid` | `rgba(29,24,25,.78)` / `#1b1718` | | `--track` | bone `.25` |
| `--panel` / `--card` / `--card-hover` | `#3c3335` / `#261f20` / `#322a2b` | | `--modal` | `rgba(29,24,25,.88)` |

The dark stance, if the components' twin ever returns: **the hue belongs to the dark end
of the ramp, not to the ink role** — bone ink in an aubergine room, never lavender ink.

Two separator weights are deliberate: `.34` between bar cells (they sit on glass),
`.28` on the hub's flat surfaces.

## Edges

- **Every chrome surface wears a ring**: `box-shadow: 0 0 0 1px var(--ring)` — outside
  the box, so blocks stay exactly 36px. Never `border` for the ring (it eats the seat).
- **Inside a bar**, flush cells divide with `border-left: 1px solid var(--sep)`.
- **Inside a stack**, nothing divides — the 7px gap of page is the divider.
- **Corners are square.** `border-radius:0` on buttons, cells, rows, headers, footers.
  Curves live only where no word does: the circle, the × pill, the toggle, a dialog
  rail's left caps, and the bar's single end cap.
- **The bar ends on a straight line.** One 18px round cap, on the circle's end, traveling
  with the circle when the bar flips (`18px 0 0 18px` unflipped, mirrored flipped,
  `100px` when collapsed to the circle alone — the radius animates on the same 0.4s
  easing). The tail is a cell, and cells are boxes.
- Row caps (`tonk-mi[cap=left|right]`, the hub × pill) are 18px radii on the outer end
  only. A dialog header takes its cap only when a side rail is slotted; the body stays
  boxy, so the × pill sticks out past it.
- No drop shadows anywhere. Depth is frost + ring, nothing else.

## Gaps and surfaces

- **Bar = one object**: gap 0, flush cells, one continuous frost surface, 1px separators.
- **Stack = many blocks**: `gap:7px` of pure page. A menu's glass lives on **one masked
  underlay** (`.w::before`) — the mask carves the 7px gaps back to page, rows paint
  rings/washes/solids above it, capped rows keep their own frost (a rectangular underlay
  can't follow the radii). **Never add a per-row `backdrop-filter`** — one filter per
  stack is a performance law (13 live blur layers → 3, measured).
- **Modal cluster = the same glass, denser**: blocks at `.92` (dark `.88`), the backdrop
  dims at `.32` — **the page dims, the modal surface never does**. At modal density the
  blur retires; only the glass color stays.
- **Flat page = pre-composited frost**: in-flow hub blocks wear `--frost-solid` (same
  look, no filter). A blur of a flat page into itself is pure GPU spend. Real
  `backdrop-filter` frost belongs only to chrome that floats **over content** — today
  that is the bar (and its stacks) over a space; nothing in the hub floats since the
  account overlay died when the cells became tabs, so every hub page renders
  filterless.
- The more it floats, the more it ghosts: floating chrome `.72` / modal blocks `.92` /
  flat page solid.

## Geometry — the numbers

| thing | number |
|---|---|
| block height | **36px** · tall row 56px · touch target 44px (invisible) |
| seat | bottom **9px** · right **10px** · glyph-word gap 8px (buttons 6px) |
| bar cells | `circle 36 · space 216 · share 144` = **396px** |
| hub column | `account 144 · spaces 288` = **432px**, fixed, centered; bar fills it — the sheet's one cluster width (modals and the ceremony share it) |
| menu width | its anchor rung's width (`--fabb-menu-w`: space stack 216, share stack 144) |
| stack gap | 7px |
| dialog | `max-width: 26rem` (wide: 36rem) · body inset `margin-right: 43px` (36 + 7) so the × pill sticks out · body padding 14px 18px |
| button | 36 × min-144, radius 0 |
| toggle | 32 × 18 outline wrapping two 14px disc positions — hollow left off, filled right on |
| sync disc | 14px: filled = online+syncing · 2px-border hollow = offline · 135° half-fill = paused |
| block cursor | 7 × 13 ink block on the last character, `mix-blend-mode:difference` |
| float margin | `max(16px, safe-area + 8px)` per edge |
| focus ring | 2px solid ink, offset −2 |

## The bar (`tonk-bar`)

- **Two states, no middle**: fully extended, or collapsed to the circle. No fold cell,
  no folded state. When cells outgrow their room they **pan** horizontally by swipe
  (hidden scrollbar, `touch-action:pan-x`); with `responsive`, under **330px** of host
  width the bar auto-collapses.
- **The circle answers three gestures**: **tap** collapses · **drag** moves · **hold
  0.5s** pauses syncing (0.5s again resumes). Past 4px of movement the drag wins and the
  hold dies. Offline, hold is a no-op — you can't pause what isn't syncing. The hold has
  no press animation: the disc flipping at the threshold is the feedback.
- **Drag snaps to the nearest edge.** Snapped right, the bar anchors there, telescopes
  leftward, and **flips its bookends**: the circle takes the right end, cap and all.
  **Content order never changes** (`space · share`). Flip is a real **DOM reorder** —
  never `row-reverse`, never CSS `order` — so reading order, focus order, and visual
  order stay one sequence. `_snap()` derives `flip` at the halfway line.
- **Coarse pointers**: default seat bottom-right, stacks open upward, the snapped seat
  persists per device, and a bottom-seated bar rides above the keyboard (visualViewport)
  and settles back.
- A live rename commits before the bar does anything else.

## Stacks, flyouts, disclosure

- A menu hangs 7px under its cell, at its rung's width. Menu cells carry
  `aria-expanded`.
- A `slot="sub"` child flies out sideways on hover-capable pointers. **A connected
  flyout bridges its gap**: the parent row's surface spans the 7px (ring lines carried
  across), so the pair reads as one piece.
- **Flyouts flip against what clips them** — the nearest overflow-clipping ancestor,
  not the viewport.
- Below 640px or on coarse pointers, disclosure is **in place**: the picked sub-stack
  replaces its parent in the same column at the rung width. Side-flight is gated to
  pointers that can hover.
- **Verbs go symbol-only on touch**; the word stays as the accessible name and the
  desktop hover surface. While a verb speaks ("copied"), its icon steps aside for the
  word, then returns.
- A removed capability is **said, not hidden**: a visitor's share stack reads
  `sharing needs an account`; the signed-out hub reads `no spaces available`. A dead
  cell is worse chrome than an honest one.

## Dialogs (`tonk-dialog`)

Native `<dialog>`. The cluster is a stack: header block, body block, optional
`slot="side"` rail (144px, wears the left caps), optional `slot="actions"` footer — a
flush boxy run at **gap 0** that renders only when actions are slotted. Header takes its
18px cap only when a rail is slotted. Primary action is solid ink — the coloring is the
CTA, no glyphs. The × pill (36 × 36, right cap) sticks out past the body. Page dims
`.32`; the surface never dims.

## The hub (`hub.html`)

- One centered **432px column** — logotype in flow above, then the two-cell bar, then
  the current page, sharing one axis and one width. The bar **fills** the column (it
  does not shrink-wrap), so it ends flush with the page below in every state.
- **The two cells are tabs** (`account 144 · spaces 288`), and the tab you are on wears
  current. **A tab is a place, not a popover**: selecting account *replaces* the spaces
  list with the account page — no overlay, no outside-click dismissal, no open/closed
  state. The account cell flushes its name left against the bar's end and carries no ▸.
- Three pages hang from the bar. **Spaces**: the list, footed by the solid
  `create new space +` row — a verb in the list it acts on, never a bar cell.
  **Account**: `settings ⚙` first (under the name is where hands look for it), then a
  two-sided `name · switch account ▸` row per other account (the current account is
  the tab, never a row), then the solid `add account +` row. **Settings**: reached
  through the account page; the account tab stays current there, and clicking it leads
  back to the account page. Switching or adding an account lands on that account's
  spaces.
- The way home is **one hop from anywhere**: the spaces cell, or Escape.
- The settings rail is a **run of boxy tabs above the body at every width** (a 144
  side rail left the 432 body too narrow; no rail → no caps). The current tab fuses
  into the body folder-style — seam covered by an explicit 1px `::after`, border not
  ring at the joint.
- Signed out, the bar states it: `create account` solid, the spaces rung reads its
  fact in soft — and sheds the current coat (**current marks places, not facts**).
  The rung **flexes into the room the account cell leaves it** — law 7's one
  permitted flex: following the *column*, never the content.
- **Phone (≤640px)**: both rungs stay — account holds its 144, spaces follows the
  column; the spaces cell is the way back from the other pages on a screen with no
  Escape. Hub margins: column `100vw − 32`, logo 28px.
- Every in-flow hub block wears `--frost-solid` (no filter — nothing floats in the hub
  any more). Dark comes from `prefers-color-scheme` alone.

## Glyphs

**No icon library — geometry, not illustration**: circles, blocks, triangles, hairlines.
If it can't be said in those shapes it probably doesn't belong in the chrome.

Unicode, rendered in the chrome font — triangles at **14px, weight 500**, 1:1 with text:

| glyph | codepoint | meaning |
|---|---|---|
| ▸ | U+25B8 | go · open — always with a word riding it (`open ▸`) |
| ◂ | U+25C2 | back (`back ◂`) — the pair is purely semantic; the directional fold reading retired with the fold cell |
| ↖ | U+2196 | leaving the environment (`more ↖` → the directory) |
| × | U+00D7 | close |
| + | U+002B | new |

Drawn marks — CSS plus three bespoke inline SVGs, all on `currentColor`/ink tokens:

| mark | construction | meaning |
|---|---|---|
| sync disc | 14px circle: ink fill / 2px border / 135° half-fill + 1.5px border | syncing / offline / paused |
| blinking disc | `fabb-blink`, opacity 1 → .55, 2.4s | changes to review (idle in the MVP) |
| block cursor ▮ | 7 × 13 ink block, hard-blink 1.05s `steps(1,end)` | editable · focused |
| rename glyph | 6 × 12 ink block — the cursor as a noun | rename |
| toggle discs | hollow left / filled right, 14px | off / on |
| settings | 9 × 9 SVG: two 1.2px lines, two 3 × 3 knobs, square caps | settings |
| link | 10 × 10 SVG: two hollow rings, a hairline between — peers joined | copy link |
| trash | 9 × 10 SVG: 3px handle knob, full-width lid hairline, solid body, square caps | remove |

## Motion

One easing: `cubic-bezier(.25,.46,.45,.94)`.

| duration | what |
|---|---|
| 0.4s | telescope, snap, the traveling cap radius |
| 0.2s | toggle disc |
| 2.4s | calm blink (alerts) |
| 1.05s | cursor hard-blink, `steps(1,end)` |
| **0.5s** | the hold — times a hand, not a frame: exempt from `prefers-reduced-motion`, animates nothing |

Everything else respects `prefers-reduced-motion`. A hidden tab pauses every animation
(`.vispause`). Do not invent a new duration or easing.

## Component contract (`fabb.js`)

One classic script, no dependencies, no build, works from `file://` — never convert to
an ES module. Eight shadow-DOM custom elements:

| tag | role | key attrs | events |
|---|---|---|---|
| `tonk-fab` | the sync circle | `state=synced\|offline\|paused` · `alert` (idle in MVP) | `fabb-press` `fabb-pause` |
| `tonk-bar` | the bar | `space` `state` `collapsed` `up` `flip` `responsive` `static` | `fabb-cell` `-collapse` `-rename` `-pause` `-snap` |
| `tonk-menu` | a stack | `data-for=space\|share` (slotted) — width from its rung | — |
| `tonk-mi` | one block | `chrome` `muted` `current` `tall` `label` `cap` · `slot="sub"` flies out | `fabb-pick` |
| `tonk-dialog` | a cluster | `heading` `wide` · `slot="side"` `slot="actions"` | `fabb-open` `fabb-close` |
| `tonk-button` | a block button | `variant=primary\|quiet` `solid` `disabled` | `fabb-press` |
| `tonk-field` | editable value — cursor blinks at rest | `value` | `change` |
| `tonk-toggle` | the switch, form-associated | `checked` | `change` |

There is no `mode` attribute and no fold/`folded` — both retired Aug 27. Imperative
surface: `open()` / `close()` / `editSpace()`.

**Boundary rules, learned the hard way:**

- Public skin API is the `--fabb-*` set only. Internal tokens are `--_ink`, `--_bg`, …
  because a host's own `--ink` *will* arrive uninvited (it did: invisible button labels).
  Every new knob is a `--fabb-*` with a `--_*` mirror. Never an unprefixed variable.
- **Document styles beat `::slotted()`**: slotted glyphs take their colors explicitly
  from the tokens, and doc classes must never share names with slotted classes (the
  doc's `.g` once hijacked the menu icons).
- Global wiring (document listeners, viewport lift, ResizeObserver, the menu's mask
  observer) is rebuilt in `connectedCallback` — reparenting fires disconnect on every
  `appendChild` move, and in-place disclosure reparents menus routinely.
- Pre-upgrade property sets are lifted by the upgrade dance (`upgrades` list per class).
- `:host([hidden]){display:none!important}` lives in the shared skin — every `:host`
  display would otherwise defeat `[hidden]`.
- Events are namespaced `fabb-*`, composed, and bubble.

## Process

- **The doc is the integration test.** A new component gets a section in `fabb.html`
  whose stage renders the real element. If the spec page and the artifact can drift,
  you built it wrong.
- **A cut takes its chrome with it and leaves its law alone.** Removing a feature never
  removes the stance that governed it (the blink primitive outlived the changes rung).
- Chrome and doc are different registers: `fabb.html` / `onboard.html` prose may use
  Gestalte and the doc grammar; nothing of the doc leaks into components.
- The onboarding pattern: **the account question comes first.** Opening a space raises
  the gate — a doorstep cluster over the dimmed space: the space's name as a header
  value (seated right), an account row when the device holds one (its ▾ unfolds the
  other accounts and the add door *into the cluster* — no floating menu), and a split
  rung of two half-rung cells: `continue without account` on frost, the account door
  on solid ink. The narrator speaks only when the device holds no account. "add an
  account" is deliberately wide — create or sign back in; the passkey prompt settles
  which, so the words never have to.
- The ceremony re-dresses the same cluster in place — ink dim, one block per completed
  step, a narrator block whose sentence and quiet verb advance; the growing cluster
  *is* the progress (no dots, no "step 2 of 4"). There is **no ×**: the way out is the
  gate's own frost cell, the ceremony's bare `◂ back to space` ghost word, or Escape.
  Payoff: the dim lifts, the disc fills, the bar assembles.
- **A decline is answered by silence.** After `continue without account` the reminder
  banner waits (10s), then rises without taking focus, saying only "Add an account to
  keep your data safe and access it from other devices." with a verb that answers the
  device (`add an account ▸` / `join ▸`). Never an instant re-ask — a decline met on
  the spot reads as a nag — and any door opening cancels the wait.
- Voice: lowercase chrome; "people", not "users"; capability does the talking; ration
  em dashes.
- When you change a number, change it everywhere it derives: the seat constants, cell
  widths, and token values appear in `fabb.js`, `fabb.html`, `hub.html`, and
  `onboard.html`. Grep before you declare done.

## Do not

- Do not vertically center a chrome word, and do not left-flush one outside the two
  sanctioned seats — a two-sided row's left noun, a terminal edge the label owns
  (symbol-only marks center; everything else seats bottom-right).
- Do not set labels in normal Plex Sans or body text in Condensed.
- Do not introduce a second color, a semantic palette, or a colored destructive action.
- Do not round a button, a cell, or a header. Do not add a drop shadow.
- Do not put gaps between bar cells or drawn lines between stack blocks.
- Do not let a width follow content; do not squeeze a cell at a breakpoint — swap rungs.
- Do not import an icon set; compose from the glyph tables.
- Do not uppercase chrome or transform user words.
- Do not render triangles below 14px or above weight 500.
- Do not put soft ink on anything clickable.
- Do not add a `backdrop-filter` to a stack row, a modal block, or anything on a flat
  page.
- Do not theme the host view from the chrome, and do not let host styles into the
  shadow roots.
- Do not show a scrollbar on the pan strip.
- Do not treat a tab's page as a popover: no outside-click dismissal, no open/closed
  state — a tab is a place, and its page replaces the last one.
- Do not answer a decline immediately — the reminder waits, then rises without taking
  focus.
- Do not dress a fact in current; current is for places (`no spaces available` stays
  soft on frost).
- Do not use Gestalte in chrome.
- Do not re-litigate the decision log in passing.
