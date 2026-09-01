# Tonk design system

This is the default visual language for Tonk product UI. Read it before changing
web layouts, components, CSS, or product chrome.

Tonk uses warm stone surfaces, aubergine ink, compact rectangular controls, and
careful typography. The product is compact and deliberate. Decoration is sparse;
hierarchy comes from value, type, geometry, and spacing.

## Sources and scope

This guide captures the current product direction shown in the Tonk account UI
and the latest FABB, Hub, onboarding, and edge studies in the adjacent
`tonk-labs/gooey` repository. When an existing surface already has tokens or
components, reuse them rather than creating a parallel system. The current
account implementation is a useful reference:
[`rust/tonk-ui/src/account.css`](rust/tonk-ui/src/account.css).

Gooey also contains older lime-and-magenta explorations, generic presets, and TUI
studies. They are not the default product style. Use them only when a task names
them explicitly. This guide covers product web UI and embedded chrome; CLI output
should follow the CLI's existing output patterns.

## The visual language

| Property | Tonk treatment |
| --- | --- |
| Atmosphere | Warm, low-chroma, technical, compact |
| Primary color | Aubergine ink on a warm stone ground |
| Shape | Rectangular word-bearing controls; curves reserved for discs, switches, close controls, and end caps |
| Structure | Fixed-width blocks, fused bars, and stacks separated by page-colored gaps |
| Type | Condensed lowercase labels; normal-width reading text |
| Depth | A one-pixel ink ring and, when floating, translucent frost; no drop shadows |
| Emphasis | Solid ink for the primary action; near-ink for the current location |
| Motion | Short, interruptible transitions with no bounce or glow |

## Color

Tonk is a one-ink system. Do not add a second brand color or a semantic red,
green, blue, or yellow palette to product chrome. Status must also be expressed
through words, geometry, or motion so color is never the only signal.

### Core palette

| Role | Light | Dark | Use |
| --- | --- | --- | --- |
| Page | `#e8e6e4` | `#161313` | App canvas |
| Ink | `#38182a` | `#e2dfdd` | Text, glyphs, rings, primary actions |
| On ink | `#f7f6f5` | `#221c1d` | Content on solid ink |
| Soft ink | `#5b4953` | `#c8c3bf` | Metadata and secondary reading text only |
| Current | `#552e44` | `#cdc5c9` | Selected view or location, not a call to action |
| Flat frost | `#f7f6f5` | `#1b1718` | In-flow blocks on a flat page |
| Panel | `#d0ccc8` | `#3c3335` | Settings and joined secondary surfaces |
| Card | `#fcfbfb` | `#261f20` | Dialog and sturdy content surfaces |
| Ring | `rgb(56 24 42 / 85%)` | `rgb(226 223 221 / 55%)` | Outer surface edge |
| Separator | `rgb(56 24 42 / 28%)` | `rgb(226 223 221 / 28%)` | Dividers inside a fused surface |
| Hover wash | Ink at `6%` | Light ink at `9%` | Hover on frost or card surfaces |
| Press wash | Ink at `12%` | Light ink at `15%` | Pressed state on frost or card surfaces |
| Scrim | `rgb(56 24 42 / 32%)` | `rgb(0 0 0 / 45%)` | Modal backdrop |

For hover on a solid-ink action, wash it with `on-ink`, not more ink:
`rgb(247 246 245 / 16%)` in light mode and `rgb(34 28 29 / 14%)` in dark
mode. An ink wash over an ink surface produces no visible state change.

### Frost

Floating chrome may use translucent frost over content:

```css
background: rgb(253 252 252 / 72%);
backdrop-filter: blur(12px) saturate(1.5);
box-shadow: 0 0 0 1px rgb(56 24 42 / 85%);
```

Use real blur only where content can pass behind the surface. On a flat page,
use the pre-composited `Flat frost` color. A stack shares one blurred underlay;
do not put `backdrop-filter` on every row. Dialog surfaces are dense enough to
use the card color without blur.

App-owned pages may follow the system light or dark scheme. Embedded chrome
owns its own scoped colors and must never recolor the host view. Do not add a
manual theme switch unless the product behavior calls for one.

## Typography

Load the fonts at the host page. Shadow-root CSS may declare the stacks but
cannot supply the font files.

| Register | Family | Specification | Use |
| --- | --- | --- | --- |
| Chrome label | IBM Plex Sans Condensed | `600 13px/1`, `0.02em` tracking | Buttons, cells, headers, section actions |
| Menu label | IBM Plex Sans Condensed | `500 13px/1` | Dense menus when less emphasis is needed |
| Reading text | IBM Plex Sans | `400 13–13.5px/1.45–1.55` | Sentences, explanations, dialog bodies |
| Emphasized name | IBM Plex Sans Condensed | `600 13.5px` | Entity names inside reading text |
| Technical metadata | IBM Plex Mono | `11–12px` | IDs, code, counts, timestamps where monospacing helps |

Product-owned chrome is lowercase. Write the source copy in lowercase and use
`text-transform: lowercase` as a backstop. Never transform names, space titles,
handles, typed values, or other user-authored text.

Do not set sentences in the condensed face or labels in normal Plex Sans. Large
Gestalte display type belongs to editorial and design-document contexts, not
ordinary product chrome. Use the existing Tonk wordmark asset instead of
recreating the logo in text.

Apply font smoothing once at the page root. Balance short headings, use pretty
wrapping for short-to-medium prose, and leave long text and code to normal
wrapping. Use tabular numerals for values that update in place.

## Geometry and spacing

The compact chrome block is the main unit:

```css
.tonk-block {
  box-sizing: border-box;
  height: 36px;
  display: flex;
  align-items: flex-end;
  justify-content: flex-end;
  gap: 8px;
  padding: 0 10px 9px 16px;
  background: var(--frost-solid);
  box-shadow: 0 0 0 1px var(--ring);
  font: 600 13px/1 var(--cond);
  letter-spacing: 0.02em;
}
```

The bottom-right label placement is deliberate. It applies to compact chrome
labels, not to paragraphs, form explanations, or data-heavy content rows.
Symbol-only controls are centered geometrically.

Two label seats may leave the right edge, both still bottom-seated. A two-sided
row holds a noun or name at its left end while the actionable word keeps the
bottom-right seat (`oat works · switch account ▸`). A label may also flush a
terminal edge it owns, such as the hub account tab's name against the bar's
left end.

| Measure | Value | Notes |
| --- | --- | --- |
| Compact block | `36px` high | Bar cells, compact rows, headers |
| Minimum hit target | `44 × 44px` | May be an invisible extension or a visible 44px control in dialogs and touch layouts |
| Label seat | `9px` bottom, `10px` right | Keep `line-height: 1` |
| Stack gap | `7px` | Page color shows between separate blocks |
| Inline glyph gap | `8px` | Use `6px` inside tight buttons |
| Symbol cell | `36px` | Sync disc and compact icon cells |
| Small column | `144px` | Actions, menus, the hub account tab |
| Medium column | `216px` | The bar's space cell and its menus |
| Paired column | `288px` | Two fused small columns — the hub spaces tab |
| Main column | `432px` | Lists, forms, dialogs, and the hub column |
| Page inset | At least `16px` | Add safe-area insets on touch devices |

These widths are a compositional grid, not a requirement to make prose fit a
small box. Combine the units for larger panels. At narrow widths, stack or swap
whole regions instead of squeezing every cell around its text. A cell may
follow its column — a split choice rung is two half cells of the column, and a
remaining cell may absorb a departed neighbor's room — but never its content.

## Edges, shape, and depth

- Draw a surface edge with `box-shadow: 0 0 0 1px var(--ring)` so the ring does
  not consume the 36px block. Use a real border where an input, separator, or
  joined seam needs to participate in layout.
- Use square corners on controls and containers that carry words. Rounded forms
  are reserved for discs, switches, close controls, and the exposed end of a
  bar or rail.
- Do not add conventional elevation shadows. Tonk depth is page, frost, ring,
  and value contrast.
- Fused bar cells have no gap and use a one-pixel separator. Separate stack
  rows have a 7px gap and no drawn divider. Never use both treatments at once.
- When imagery appears, add an inset one-pixel neutral outline: pure black at
  10% in light mode and pure white at 10% in dark mode.

## Component treatments

### Actions

| Role | Treatment |
| --- | --- |
| Primary | Solid ink, on-ink label, square corners. The fill is the emphasis; no decorative icon is needed. |
| Secondary | Flat frost or card surface with an ink ring |
| Quiet | Bare text with a one-pixel underline; thicken the underline on hover |
| Current location | Near-ink fill. It must not compete with the primary action. |
| Disabled or busy | Reduced opacity plus the correct disabled cursor and accessible state |

Soft ink is never used for an actionable label because it reads as disabled.
Standard press feedback is `scale: 0.96` over `150ms`. Do not scale drag handles,
long-press controls, or any control whose state change already provides the
physical feedback.

Near-ink current marks a place, never a fact. A cell that only states a fact,
such as `no spaces available`, stays soft on its resting surface rather than
wearing the current fill.

### Rows, fields, and menus

- Compact action rows use the block geometry. Reading rows use normal Plex Sans
  with `1.45–1.55` leading and may align on the baseline instead of using the
  chrome seat.
- Text fields are square and usually transparent with a one-pixel underline.
  Labels or values align right when they complete a chrome row; long forms may
  use the clearer conventional alignment.
- Menus are stacks of blocks, not a rounded floating panel. Match the menu width
  to its anchor column.
- On hover-capable devices, secondary row actions may appear on approach. On
  coarse pointers, keep the action available without relying on hover.
- State the absence of a capability in plain text instead of leaving a dead or
  unexplained control.

### Tabs and pages

- A tab is a place, not a popover. Selecting a tab replaces the page below it;
  there is no overlay, no outside-click dismissal, and no open state to manage.
  In the hub, the bar's two cells — account and spaces — work this way: the
  account page (settings, switch-account rows, add account) replaces the
  spaces list rather than floating over it.
- The current tab wears near-ink. A section reached through a tab's page keeps
  that tab current: hub settings lives under the account tab, and the account
  tab stays current while settings is open, leading back to its page when
  pressed.
- The way home is one hop from anywhere: the home tab itself, or Escape. Keep
  the home tab visible on touch layouts, where there is no Escape.
- Switching to another context, such as another account, lands on that
  context's main page, not back on the switcher.
- Secondary navigation, such as the settings sections, is a run of square tabs
  above its body at every width. The current tab shares the body's surface and
  fuses with it; cover the joining seam explicitly.

### Dialogs

Use native `<dialog>` behavior where possible. A Tonk dialog is a small cluster
of independently edged blocks: header, body, optional explanatory or arming
block, and actions. Separate blocks with 7px; fuse the action run at gap 0.
The page scrim dims, not the dialog surface. The close control may form the one
rounded exposed end.

Dialogs must remain usable in short viewports. Constrain and scroll the dialog
body so the title, close control, and actions remain reachable.

### Gates, ceremonies, and reminders

- When someone opens a shared space, ask the account question first. A
  doorstep cluster stands over the dimmed space: the space's name as a header
  value, the account that would enter (with an in-cluster picker when the
  device holds several — no floating menu), and one split rung offering both
  answers at once: `continue without account` on a frost cell, the account
  door on solid ink. Declining is a real answer, so it gets a real control,
  never a ghost link.
- Write the account door as `add an account`. The phrase is deliberately wide:
  it covers creating an account and signing back in, and the passkey prompt
  resolves which, so the label never has to.
- A ceremony meant to be finished may omit the close control entirely. The way
  out is then a quiet word plus Escape; a gate's own decline cell counts.
- A decline is answered by silence. The reminder returns only after a pause
  (about ten seconds), rises without taking focus, and says: "Add an account
  to keep your data safe and access it from other devices." Its verb answers
  the device — `add an account ▸` when none is present, `join ▸` when one is.
  Opening any account door cancels the pending reminder. Never re-ask at the
  moment of decline.

### Glyphs and branding

Product chrome uses simple geometry: discs, blocks, triangles, hairlines, and a
small set of bespoke SVG marks. Prefer `currentColor` and square line caps. Do
not introduce a general icon library just to decorate FABB chrome.

Use these text glyphs consistently:

| Glyph | Meaning |
| --- | --- |
| `▸` | Open or continue, normally paired with a word |
| `◂` | Back, normally paired with a word |
| `↖` | Leave the current environment |
| `×` | Close |
| `+` | Create or add |
| `▾` | Unfold a picker in place, normally on the row it expands |

The sync disc communicates state through fill: filled for online/syncing,
hollow for offline, and diagonally divided for deliberately paused. Every glyph
still needs an accessible name.

## Interaction and motion

Motion explains state or preserves spatial context. It is never decorative.

| Timing | Use |
| --- | --- |
| `150ms ease-out` | Hover, press, opacity, and standard controls |
| `200ms` | Toggle-disc movement |
| `400ms cubic-bezier(.25,.46,.45,.94)` | FABB telescope, snap, and other spatial movement |
| `450ms`, at most twice | Brief error or state flash |
| `1.05s steps(1, end)` | Editable block cursor |
| `2.4s` | Calm waiting or attention pulse |
| `500ms` | Long-press threshold; this times the gesture and is not itself an animation |

Use CSS transitions for interactive state changes so they can reverse when the
user changes direction. Specify the properties; never use `transition: all`.
Avoid bounce, glow, sweeping gradients, and staged entrance animation on routine
product pages. Only add `will-change` after observing a real first-frame issue.

Honor `prefers-reduced-motion`. Remove decorative transitions, waiting pulses,
and flashes while preserving immediate state changes and gesture thresholds.

## Responsive and accessible behavior

- Adapt when content stops fitting, not from a guessed device name. Preserve an
  existing component's tested breakpoints when editing it.
- Swap, stack, or disclose whole regions. Do not compress 36px chrome until its
  text becomes illegible. Keep user-authored text wrappable where the surface
  permits it.
- On coarse pointers, use in-place disclosure instead of hover flyouts. Keep
  draggable chrome inside safe-area insets and above the on-screen keyboard.
- Interactive targets are at least 44px in each dimension and must not overlap.
- Use `:focus-visible`. The focus treatment must remain visible on both frost
  and solid ink; use an inset on-ink/ink pair when one color is insufficient.
- Use ink/on-ink selection colors and remove the browser-blue tap highlight only
  when an equivalent pressed state is present.
- Do not rely on hue, hover, motion, or an icon alone to communicate meaning.
- Preserve DOM, reading, and focus order when a component changes visual order.

## Voice

Product-owned labels are short, direct, and lowercase. Prefer capability or
state language such as `copy link`, `create account`, `continue without
account`, or `no spaces available`. Use “people” rather than “users” in product
copy. Preserve the spelling and case of names and other user-authored content.

Errors should say what happened and what the person can do next. Success,
warning, and destructive actions use the same ink palette; wording and layout
carry their meaning.

## Before shipping a UI change

- Reuse the surface's existing tokens and components.
- Check the light and dark schemes that the surface actually supports.
- Confirm label font, case, alignment, and user-text preservation.
- Check 36px geometry, 44px hit targets, 7px gaps, and fixed column alignment.
- Confirm primary, current, quiet, disabled, hover, press, focus, busy, and error
  states are distinguishable without a semantic color.
- Test keyboard order, coarse-pointer behavior, reduced motion, narrow width,
  and short viewport height.
- Inspect the rendered result. Static CSS review is not visual verification.
