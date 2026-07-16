# Clicking links inside a spot

## Problem

A link inside a spot's content does nothing. No navigation, no new tab, no error.

The click relay exists — `rust/tonk-guest/src/guest_host.rs:81-106` catches clicks at
the guest document in capture phase, walks to the nearest `<a href>`, and posts the
href over `window.tonk.navigate` for the trusted parent to perform. But it is gated to
in-app paths:

```rust
// guest_host.rs:96
if !href.starts_with('/') || href.starts_with("//") {
    return;
}
```

Everything else falls through to a native navigation the sandbox blocks. The iframe is
`sandbox="allow-scripts allow-forms"` (`rust/tonk-portal/src/shared.rs:60`), with
`allow-same-origin` and `allow-top-navigation` deliberately withheld and `allow-popups`
never granted. So `https://…`, `mailto:`, `target="_blank"`, and modified clicks
(cmd/ctrl/middle — which bail at `guest_host.rs:86-90`) are all inert.

In scope: external `http(s)` links, `mailto:`/`tel:`, `target="_blank"` on in-app
paths, and modified clicks. Out of scope: in-page fragments.

## The depth constraint is the whole design

Spot content is a **depth-2** guest:

```
top page (real origin)
  └─ <tonk-site with="main@profile:tonk" allow="*">   depth-1: profile chrome
       └─ <tonk-site with="main@{id}">                depth-2: the spot's content
```

A guest's port dispatcher runs in its **parent** document (`install_message_listener`
is guarded once per document by `LISTENER_INSTALLED`, `bridge.rs:1051-1058`). So a
message from spot content is dispatched **inside the depth-1 chrome guest** — itself an
opaque-origin `about:srcdoc` document.

`handle_navigate` performs its effect right there, with no forwarding:

```rust
// bridge.rs:1430-1435
fn handle_navigate(data: &JsValue) {
    let Some(href) = get_str(data, "href").filter(|h| !h.is_empty()) else { return; };
    tonk_host::navigate_to(&href);
}
```

An exhaustive grep for `handle_navigate|navigate_to` returns six lines. There is no
`is_top` check, no re-post to `parent`, no nested forward. The chain dead-ends one hop
up.

**This matters for the feature because the depth-1 chrome cannot open a tab either** —
it is sandboxed without `allow-popups`. Only the top page can. Forwarding is therefore
mandatory, not a refinement.

## Latent bug: depth-2 navigation is already broken

Not a new problem, just an unreached one. Every anchor in the product lives at depth 1
— all four are in `profile.yaml` (`:508` hub row, `:722` FAB switcher, `:863` all-spots,
`:2136` 404 home). `core.yaml`'s spot-branch route table (`route/directory`,
`route/artifact`, `route/adhoc`) contains **zero anchors**. Nothing a user clicks
exercises depth 2, and there are no tests for `navigate_to` or `handle_navigate`.

If it were reached, `navigate_to` would do this:

```rust
// navigate.rs:117-133
let pushed = win.history().ok()
    .map(|h| h.push_state_with_url(&JsValue::NULL, "", Some(href)).is_ok())
    .unwrap_or(false);
if pushed { /* dispatch popstate */ } else {
    // No history access — fall back to a real (reloading) navigation.
    let _ = win.location().assign(href);
}
```

`pushState` from `about:srcdoc` is rejected — target URL and document URL differ in
scheme, so "can have its URL rewritten" is false → `SecurityError`. The comment
attributes `pushed == false` to an absent `History`; it does not contemplate a present
`History` that *rejects the URL*, and `.is_ok()` swallows both into one branch. The
fallback then `location.assign`es the **chrome iframe itself** onto the real origin
(self-navigation needs no `allow-top-navigation`), reloading the whole app inside an
opaque-origin frame where `navigator.serviceWorker` throws, with the URL bar unchanged.

The same-URL guard (`navigate.rs:110-115`) is also inert at depth 2:
`Url::new_with_base(href, "about:srcdoc")` fails to parse — `about:srcdoc` is an
opaque-path URL, not a valid base — so the `if let` chain never fires.

This is a live hazard: the moment any spot view grows an `<a href="/…">`, it lands here.
PR 1 fixes it as a precondition of the feature.

## Also discovered: `<tonk-fab-portal>` is dead code

`register_fab_portal` (`fab.rs:340`) has no caller. `guest.rs`'s `start()` registers
`register()`, `register_site()`, `register_title()`, and `tonk_fab::register()` — never
`register_fab_portal`. The element is never `define`d, and `tonk-fab-portal` appears in
no yaml or html. The real `<tonk-fab>` (`profile.yaml:811-815`) is plain DOM in the
depth-1 chrome guest, which is why FAB spot-switching works.

354 lines of unreachable code, plus stale comments at `profile.yaml:807-808`,
`fab.rs:5-8`, and `shared.rs:3` describing an architecture that no longer runs. Not
touched here — see Scope.

## PR 1 — page-effect forwarding

`navigate.rs:9-14` predicted this exactly:

> It is the first "main-thread command provider"; when a second page-only effect
> appears (clipboard, focus, title), generalize this into a small registry keyed by
> message `type`.

`title` then arrived as a parallel special case (`bridge.rs:1189`, `handle_title`),
leaving the registry unbuilt and two hand-rolled capabilities that both work only one
hop up. PR 1 builds it.

**The rule.** Before performing a page-only effect, ask: am I myself a guest? If so,
re-post the effect over `window.tonk`; otherwise perform it. O(depth) hops. It is the
same recursion as `context_origin()` (`tonk-host/src/bridge.rs:40-52`), which solves the
identical "the real value lives N frames up" problem.

**The discriminator is `window.tonk` presence, not `window === window.top`.**
`window.tonk` is assigned at exactly one place — `bridge.rs:381`, inside
`BOOTSTRAP_JS`, which only ever runs in a guest's `srcdoc`. No Rust sets it; the top
page never has one. So its presence means precisely "I am a portal guest with a bridge
to my parent". A `window.top` check would encode "I am the outermost frame", which is a
different claim and would break if the Tonk page were ever itself embedded.

Applies to `navigate` and `title`. `open` (PR 2) registers as the third.

Forwarding also unblocks depth-2 titles, which the title spec deferred as needing "a
depth-2 relay ... Not built"
(`docs/superpowers/specs/2026-07-16-spot-tab-title-design.md:280`). Sub-route titles
remain out of scope here; PR 1 only removes the obstacle.

## PR 2 — the `open` effect

### Guest: classify instead of bailing

`guest_host.rs`'s listener currently returns early on anything that isn't a `/…` path,
and on any modified click. It gains a classifier:

| Click | Destination | Action |
|---|---|---|
| **any** | `#fragment` | untouched — left native. Checked FIRST, before modifiers |
| plain | `/…` (not `//`) | `navigate` (unchanged) |
| plain | `http`/`https`/`mailto`/`tel` | `preventDefault` + `open` |
| plain, `target="_blank"` | `/…` | `preventDefault` + `open` |
| cmd/ctrl/shift/middle | `/…` or external | `preventDefault` + `open` |
| any | anything else | `preventDefault` + `open` — the host refuses it and warns |

**The fragment row comes first, and beats the modifier row.** A cmd-clicked `#x` is left
native rather than opened in a tab, even though that is what a browser would do
natively. The fragment addresses the **guest's** document, and the guest is
`about:srcdoc`: the host resolving `#x` against its own URL (`/space/{id}`) would open a
duplicate spot scrolled nowhere, not the anchor the user aimed at. Ignoring is both in
scope ("fragments left native") and the better outcome.

**The last row is not a filter, and the guest does not "drop" anything.** It relays
hrefs it fully expects the host to refuse, `javascript:` included. Guest-side scheme
filtering would be security theatre: a component can call `window.tonk.open` directly,
so the guest is never the control — the host's allowlist is, and it is the only thing
standing between an attacker-authored href and the real origin. Relaying also gets a
console warning out of the host instead of a silently dead click, which is the bug this
whole change exists to fix.

`closest_anchor` (`guest_host.rs`) reads the **raw attribute**, not the resolved `.href`
property, which an opaque origin mangles to `null/…`. `target` likewise, via
`get_attribute` — which also keeps `HtmlAnchorElement` out of the crate's web-sys
features. Resolution is the host's job: it is the only frame with a real base URL.

### Bridge: a sixth message type

`window.tonk.open(href)` posts `{v:1, type:"open", href}`, fire-and-forget, mirroring
`navigate` (`bridge.rs:235-237`) and `setTitle` (`bridge.rs:241-243`). Dispatched at
`bridge.rs:1172-1194` through PR 1's forwarding.

### Host: `tonk-host/src/open.rs`

A new module, sibling to `navigate.rs` and `title.rs`, following the `title` precedent:
a page-only effect performed by the trusted top document. It is the **single policy
point** — forwarding stays dumb ("not me, pass it up"), so there is no intermediate
gate for spot content to forge past.

1. **Resolve** `href` against the top document's base. The guest sends a raw attribute;
   the real origin does the resolving. This is why `/…` and `target="_blank"` compose
   without the guest knowing its own origin.
2. **Classify by origin**, using the browser's URL parser (`web_sys::Url`) — never
   string prefix matching:
   - **Same origin as the top page** → no dialog. Nothing to warn about.
   - **Different origin, scheme in {`http`, `https`, `mailto`, `tel`}** → dialog.
   - **Anything else** → reject.
3. **Open.**

**The dialog gates leaving the origin, not opening a tab.** That single rule covers
every case: a cmd-clicked in-app link opens silently; an external link is always
announced.

`classify` returns `Destination::{ SameOrigin(url), External { url, label }, Rejected }`.

### What the dialog names: `label`, and why it is not the host

`label` is the destination's identity: the **full origin** (`scheme://host:port`) for
http(s), the address for `mailto:`/`tel:` (which have no origin). The dialog renders it
verbatim. Every plausible-looking simplification here is a demonstrated vulnerability —
each was found by probing the real parser, not by reading the spec:

- **`hostname` drops the port.** `tonk.example:8443` is a different origin from
  `tonk.example`, and anyone can bind a port.
- **`host` drops the scheme.** `http://tonk.example/` would announce itself as
  `tonk.example` — our own site — for a destination that is not our origin. A plain
  downgrade reads as home.
- **A reconstructed URL can carry userinfo.** `https://tonk.example@evil.com/` reads as
  ours while going to `evil.com`.

So `classify` strips userinfo (username **and** password — clearing only the username
leaves `https://:pw@evil.com/`, where the password and the disguising `@` both survive)
and hands the dialog strings that are already correct. The invariant is: **what we
display is exactly what we open.** Stripping cannot flip a classification, because
userinfo is not part of an origin.

`mailto:`/`tel:` must name something a person can judge, so an address whose decoded form
carries no alphanumeric is rejected, as is one with an authority or a rooted path —
`mailto://tonk.example/x` would otherwise name `/x` beside a URL reading `tonk.example`,
which is display and destination disagreeing.

One thing the dialog does **not** have to defend against: the parser percent-encodes
non-ASCII in path/query/fragment and punycodes hosts, so both fields are always ASCII. A
right-to-left override (`https://evil.com/#‮gpj.exe`) arrives as `#%E2%80%AEgpj.exe`.
That is *why* a text node suffices, not merely convention.

### One dialog at a time

`open_external` runs straight from the guest relay, so a hostile spot could post `open`
in a loop. Without a gate that stacks N modal dialogs on the trusted page — and, before
the closures were given an owner, leaked the whole detached dialog subtree each time
(measured: 7 wasm heap slots per link). So `confirm_then_open` refuses while a dialog is
already up.

This costs nothing real: a modal dialog makes the rest of the top document inert,
and its backdrop hit-tests even over the guest's iframe, so a *user* cannot reach a
second link while one is open. Any second call is scripted — hostile or buggy — and the
dialog already up is the one the user is answering.

(Inertness does not propagate into a nested browsing context for *programmatic* focus: a
scripted `focus()` inside the guest still lands. That is measured, and it strengthens the
case for the gate rather than weakening it.)

### Two open paths, two mechanisms

`window.open` and programmatic `anchor.click({target:_blank})` both require transient
user activation. The paths differ in whether they have one, so they differ in
mechanism:

| Path | Activation | Mechanism | If blocked |
|---|---|---|---|
| Dialog (external) | the **Open press**, in the top document | synthesize `<a target="_blank" rel="noopener noreferrer">` and click it | cannot be — activation is guaranteed |
| No dialog (same-origin) | must survive two `postMessage` hops from depth 2 | `window.open(href, "_blank", "noopener")` | returns `null` → fall back to same-tab `navigate_to(href)` |

**The interstitial removes the popup-blocker risk rather than adding cost.** Without it,
every external open would depend on transient activation propagating across the relay —
plausible (activation propagates to ancestors, the window is ~5s, two hops are
sub-millisecond) but browser-dependent and weakest in Safari. The Open press *is* an
activation in the top document, so the dialog path never gambles.

The no-dialog path does gamble, which is why it uses `window.open` — the one mechanism
whose failure is **detectable** (`null` return). An in-app path degrading to a same-tab
navigation is a reasonable outcome; silently doing nothing is not.

`anchor.click()` is used on the dialog path because it handles `mailto:`/`tel:`
correctly — `window.open("mailto:…")` can strand a blank tab, while an anchor click
inherits normal browser handling across all four schemes.

### The dialog is plain DOM, not `<wa-dialog>`

The Web Awesome loader is idle-injected, not eager (`index.html:230-258`), because its
~16 statically-imported chunks would otherwise starve the boot data plane. A `wa-*`
component could therefore be undefined at click time inside the ~3s window.

A native `<dialog>` + `showModal()` gives focus trap and Esc for free, styled with
`var(--wa-token, literal)` — exactly the technique the boot shell already uses
(`index.html:47-52`) so it cannot flash unstyled. This also keeps `index.html:236`'s
claim ("Nothing on the TOP page uses a `<wa-*>` COMPONENT") true, and adds no dependency
on loader timing.

Copy names the destination's **origin** prominently and the full URL beneath it:

```
Open in a new tab?

https://example.com
https://example.com/docs/x

            [ Cancel ]  [ Open ]
```

The prominent line is the full origin — `scheme://host:port` — not a bare host. This
is not cosmetic. A bare host cannot express an origin, so `http://tonk.example/` would
announce itself as `tonk.example`: our own site, for a destination that is not our
origin. The same argument kills `hostname`, which drops the port when
`tonk.example:8443` is a different origin anyone can bind. Both were demonstrated
against the real parser, and both are pinned by tests.

Not "Leave this spot?" — with `noopener` and `_blank`, the spot stays open and the user
does not leave.

## Data flow

```
click in spot content (depth 2)
  → guest_host classifies → window.tonk.open(href)
  → port → dispatcher in the depth-1 chrome guest
  → window.tonk present → re-post up
  → port → dispatcher in the top page
  → window.tonk absent → perform
  → open.rs: resolve → classify by origin
       ├─ same origin  → window.open → null? → navigate_to (same tab)
       └─ external     → allowlist → dialog → [Open] → anchor.click()
```

## Security

The relay hands an **attacker-controlled string to the trusted origin**. Spot content is
data: views and components are facts a collaborator or agent can assert into a space.

- **`javascript:` must never reach the anchor.** It would execute on the real origin and
  defeat the entire sandbox the architecture exists to maintain. The allowlist is the
  control, and it lives at the host — the trusted end — never in the guest.
- **Parse, don't prefix-match.** `web_sys::Url` only. `JaVaScRiPt:`, leading whitespace,
  and embedded newlines all defeat string comparison and are exactly how this class of
  bug ships.
- **`rel="noopener noreferrer"`** on every synthesized anchor; `noopener` on every
  `window.open`. Prevents reverse tabnabbing.
- **No intermediate gate.** All policy is at the top page, so there is no
  "confirmed" flag an intermediate document could forward and spot content could forge.
- **Accepted:** spot content can render a *fake* dialog inside its own iframe rect. It
  cannot suppress the real one, so the attack buys confusion, not a bypass. Mitigated
  only by the real dialog reading as top-page chrome.

## Errors

A rejected scheme is dropped and `console.warn`ed by the top page, naming the href and
the allowlist. No bridge channel is involved: rejection happens on the page, which is
already the real console. (The `__tonkRuntime:"warn"` relay at `bridge.rs:447-458` exists
to lift **guest** errors out of an opaque origin that sanitizes them out of the parent
console — a different problem. An earlier draft of this spec said the rejection rode that
channel; it does not.)

`confirm_then_open` warns on each of its own bail paths too — no document, no body, the
dialog failing to build. All are unreachable in practice, but a silently-dropped click is
the bug this change exists to fix, and reproducing it in a narrower form inside the fix
would be worse than the original: the user would have clicked a link that the code
deliberately chose not to open, and said nothing.

## Testing

- **`open_href`** — the message parse, mirroring `navigate_href` (`navigate.rs:173-200`)
  and `title_text` (`bridge.rs:1451`). Accepts only `{type:"open", href}` with non-empty
  href; rejects other types and non-object payloads.
- **The scheme allowlist** — table-driven, and the security core of the change. Accepts
  `http`, `https`, `mailto`, `tel`. Rejects `javascript:`, `data:`, `blob:`, `file:`,
  `vbscript:`, and the evasions: `JaVaScRiPt:`, leading/embedded whitespace, embedded
  newlines.
- **Origin classification** — same-origin resolves to the no-dialog path; a `/…` path
  resolves against the top origin and is therefore same-origin; an absolute external URL
  is not.
- **Forwarding** — the dispatcher re-posts when `window.tonk` is present and performs
  when it is absent. This is PR 1's whole contract.
- **Browser-verified**, not CI: an external link in a spot shows the dialog and opens a
  tab; Cancel opens nothing; a cmd-clicked in-app link opens with no dialog; an in-app
  link still navigates in place. Per `project_wasm_tests_need_safari_automation`, local
  wasm tests need Safari automation or a major-matched chromedriver.

All tests use `#[dialog_common::test]` and `it_does_x` naming, per repo convention.

## Risks

- **The allowlist is the whole security story.** A bypass is a full sandbox escape onto
  the real origin, not a degraded feature. It gets the most testing per line of any code
  here.
- **Popup blocked on the no-dialog path.** Mitigated by the `null`-return fallback to a
  same-tab navigation. If activation propagation turns out worse than expected in
  Safari, the fallback is the behaviour, and it is acceptable.
- **Forwarding depends on `window.tonk` meaning "I am a guest".** Verified: assigned
  only at `bridge.rs:381` inside `BOOTSTRAP_JS`. If a future change ever installs
  `window.tonk` on the top page, every page effect silently no-ops by posting to a
  parent that isn't listening. Pin the assumption in a comment at the forwarding site.
- **`navigate_to`'s fallback is still wrong** even after PR 1 — it will still misread a
  `SecurityError` as "no history access". PR 1 makes it unreachable rather than correct.
  Worth a comment; a real fix means distinguishing the two failures.
- **Guest bootstrap is a JS string.** `open` is added to hand-written JS inside a Rust
  string literal (`bridge.rs:225-243`), so a typo is a runtime failure the compiler
  cannot catch. Keep it a near-copy of `navigate` and exercise it in the browser check.
- **Dead code misleads.** `fab.rs` and its stale comments cost real time during this
  investigation and will cost the next reader the same.

## Scope

In scope: PR 1 (forwarding for `navigate` + `title`), PR 2 (the `open` effect —
external links, `mailto:`/`tel:`, `target="_blank"`, modified clicks, allowlist,
dialog).

Out of scope, deliberately:

- **In-page fragments.** Left native.
- **Sub-route titles.** PR 1 unblocks them; nobody has asked for them.
- **Deleting `fab.rs` and fixing the stale comments.** A worthwhile separate cleanup,
  not mixed into a security-sensitive change.
- **A real fix for `navigate_to`'s `SecurityError` misread.** Unreachable after PR 1.
- **Hover destination chips.** Considered and dropped: a chatty message stream that
  shows nothing on touch devices.
- **Per-host "don't ask again".** Speculative until the dialog proves annoying.
