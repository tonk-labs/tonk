//! The bar's markup and its component-local CSS.
//!
//! No DOM imports — this compiles and tests on the native target, like
//! [`crate::logic`] and [`crate::skin`], so the geometry laws are checked by
//! a plain `cargo test` rather than only under wasm.
//!
//! The strings here are the bar's half of the FABB spec; [`crate::bar`] owns
//! the behaviour that drives them. The shared token block they layer over
//! lives in [`crate::skin::SKIN`].
//!
//! ## The absent rungs
//!
//! The full product bar is `[circle 36][space 216][share 144]`.
//! The reference's `changes` rung is omitted here — it drives preview /
//! accept / discard / restore over proposals and history points, and this
//! repo implements neither. See `plan/fabb-conformance.md`. The mode cell
//! left with the switcher: the theme follows the system, and only the
//! system.

/// The `.w` state classes and cell geometry, layered over
/// [`crate::skin::SKIN`] in the bar's shadow root.
pub const BAR_CSS: &str = r#"
:host{ display:inline-block; max-width:100%; vertical-align:top; --fabb-space-width:360px;
  transition:left .4s var(--_ease),top .4s var(--_ease),transform .2s var(--_ease); }
:host([dragging]){ transition:none; }
:host([hidden]){ display:none; }
:host([data-task-hosted]) .w{ visibility:hidden; }
.w{ position:relative; display:grid; grid-template-columns:minmax(0,1fr);
  width:min(var(--fabb-space-width),var(--_room,calc(100vw - 32px)),calc(100vw - 32px));
  max-height:var(--_height,none); color:var(--_ink); border:1.5px solid var(--_ringc);
  border-radius:25px; background:var(--_bg); backdrop-filter:var(--_filter);
  -webkit-backdrop-filter:var(--_filter); overflow:auto; isolation:isolate;
  transition:width .4s var(--_ease),border-radius .4s var(--_ease); }
.w.has-panel{ width:min(calc(var(--fabb-space-width) + 600px),var(--_room,calc(100vw - 32px)),calc(100vw - 32px));
  grid-template-columns:calc(var(--fabb-space-width) - 3px) minmax(0,1fr); }
.w.closing-panel{ width:min(var(--fabb-space-width),var(--_room,calc(100vw - 32px)),calc(100vw - 32px)); }
.w.closing-panel .panel > *{ visibility:hidden; }
.bar{ min-width:0; position:relative; display:flex; flex-direction:column; }
:host([up]) .bar{ flex-direction:column-reverse; }
.header{ height:48px; display:flex; align-items:stretch; cursor:grab; touch-action:none; user-select:none; }
:host([dragging]) .header,:host([dragging]) .header button{ cursor:grabbing; }
.header:hover,.w.menu-open .header{ background:var(--_hover); }
.header:active{ background:var(--_press); }
button,a{ min-height:48px; font:600 17px/1.1 'IBM Plex Sans Condensed','Arial Narrow',sans-serif; }
.fab{ width:48px; flex:none; display:grid; place-items:center; touch-action:none; user-select:none; }
.fab .disc{ width:18px; height:18px; }
.space{ flex:1; min-width:0; display:flex; align-items:center; justify-content:flex-start;
  padding:0 6px; text-align:left; white-space:nowrap; overflow:hidden; text-overflow:ellipsis; }
.space .n{ min-width:0; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
.space .edit{ text-align:left; }
.run{ display:grid; }
.run[hidden],.action[hidden],.panel[hidden],.more,.mw{ display:none!important; }
.action{ display:flex; align-items:center; justify-content:flex-start; gap:18px;
  padding:0 28px 0 16px; text-align:left; text-decoration:none; color:var(--_ink); border-radius:0; }
.action svg{ width:21px; height:21px; flex:none; }
.action span{ min-width:0; overflow-wrap:anywhere; }
.action:hover,.back:hover{ background:var(--_hover); }
.action:active{ background:var(--_press); }
.action[aria-expanded=true]{ background:var(--_cur); color:var(--_on); }
.action[aria-disabled=true]{ opacity:.5; cursor:not-allowed; }
.panel{ min-width:0; height:240px; min-height:0; border-left:1.5px solid var(--_ringc);
  position:relative; display:flex; flex-direction:column; overflow:auto; }
.panel-head{ min-height:48px; display:flex; align-items:stretch; justify-content:flex-end; padding:0 17px; }
.back{ display:none; margin-right:auto; padding:0 10px; }
.panel-copy{ padding:0 10px; text-decoration:underline; text-underline-offset:4px; }
.panel-copytext{ margin:0; min-height:0; overflow:auto; padding:8px 24px 16px;
  font:500 13px/1.65 'IBM Plex Mono',ui-monospace,monospace; white-space:pre-wrap; overflow-wrap:anywhere; }
#agent-panel{ overflow:hidden; }
#agent-panel .panel-copytext{ flex:1 1 0; overflow-y:auto; }
.agent-status{ margin:0; padding:4px 24px 12px; font:400 16px/1.35 'IBM Plex Sans Condensed','Arial Narrow',sans-serif; }
.panel-message{ margin:auto; padding:24px; font:400 18px/1.5 'IBM Plex Sans Condensed','Arial Narrow',sans-serif; text-align:center; }
.members-list{ min-height:0; overflow:auto; padding:0 18px 16px; }
.members-list .mem-row{ min-height:42px; display:flex; align-items:center; justify-content:flex-end; gap:8px;
  border-bottom:1px solid var(--_sep); font:600 15px/1.2 'IBM Plex Sans Condensed','Arial Narrow',sans-serif; }
.members-list .mem-row:last-child{ border-bottom:0; }
.members-list .mem-tag{ color:var(--_soft); font-size:12px; font-weight:500; }
.members-list .mem-self{ text-decoration:underline; text-underline-offset:3px; }
.members-empty{ margin:auto; padding:24px; font:400 18px/1.5 'IBM Plex Sans Condensed','Arial Narrow',sans-serif; }
.share-gate,.agent-gate{ background:var(--_hover); display:grid; place-items:center; flex:1; }
.share-continue,.agent-continue{ display:flex; align-items:center; justify-content:center; gap:12px; padding:0 24px; text-align:center; }
.share-continue span,.agent-continue span{ text-decoration:underline; text-underline-offset:4px; }
/* The drawer grows for 400ms. Reveal its prompt only once text has its final width. */
.share-continue,.agent-continue{ animation:fabb-gate-in .2s var(--_ease) .4s both; }
@keyframes fabb-gate-in{ from{ opacity:0; transform:translateY(6px); } to{ opacity:1; transform:none; } }
@media (prefers-reduced-motion:reduce){ .share-continue,.agent-continue{ animation:none; } }
:host(:not([data-account-required])) #share-panel .share-gate{ display:none; }
:host([data-account-required]) #share-panel .share-progress{ display:none; }
:host(:not([data-account-required])) #agent-panel .agent-gate{ display:none; }
:host([data-account-required]) #agent-panel .panel-head,
:host([data-account-required]) #agent-panel .agent-status,
:host([data-account-required]) #agent-panel .panel-copytext{ display:none; }
:host([data-unknown-space]) .share{ display:none; }
:host([alert]) .disc.st{ animation:fabb-blink var(--_blink) var(--_ease) infinite; }
:host([alert]) .share{ animation:fabb-wash var(--_blink) var(--_ease) infinite; }
:host([alert]) .share:hover{ animation:none; }
:host([data-account-required][alert]) .disc.st{ animation:none; }
.w.collapsed{ width:51px; grid-template-columns:48px; border-radius:50%; overflow:hidden; }
.w.collapsed .space,.w.collapsed .run,.w.collapsed .panel{ display:none!important; }
.w.flip.has-panel:not(.stacked){ grid-template-columns:minmax(0,1fr) calc(var(--fabb-space-width) - 3px); }
.w.flip.has-panel:not(.stacked) .bar{ grid-column:2; grid-row:1; }
.w.flip.has-panel:not(.stacked) .panel{ grid-column:1; grid-row:1; }
.w.flip .action{ flex-direction:row-reverse; justify-content:flex-start; padding:0 14px 0 16px; }
.w.flip .header{ flex-direction:row-reverse; }
.w.flip .space{ justify-content:flex-end; text-align:right; }
.w.flip .space .edit{ text-align:right; }
.w.has-panel:not(.stacked) .bar{ border-right:1.5px solid var(--_ringc); }
.w.has-panel:not(.stacked) .panel{ height:auto; min-height:240px; border-left:0; border-right:0; }
.w.has-panel:not(.stacked) #agent-panel{ height:var(--_rail-height,240px);
  max-height:var(--_rail-height,240px); min-height:0; }
.w.flip.has-panel:not(.stacked) .bar{ border-right:0; border-left:1.5px solid var(--_ringc); }
.w.stacked.has-panel{ grid-template-columns:minmax(0,1fr); width:min(var(--fabb-space-width),var(--_room,calc(100vw - 32px))); }
.w.stacked .panel{ border-left:0; border-top:1.5px solid var(--_ringc); max-height:max(96px,calc(var(--_height,550px) - var(--_rail-height,240px) - 3px)); }
.w.stacked .back{ display:block; }
:host([up]) .w.stacked .panel{ grid-row:1; border-top:0; border-bottom:1.5px solid var(--_ringc); }
:host([up]) .w.stacked .bar{ grid-row:2; }
/* contained tasks keep the FABB's seat and replace its visible surface. The
   native dialog owns modality; the wrapper still owns the material. */
.request-layer{ position:fixed; inset:auto; margin:0; padding:0; border:0;
  max-width:none; max-height:none; overflow:visible; background:transparent; color:inherit; }
.request-layer::backdrop{ background:var(--fabb-dim,rgba(56,24,42,.32)); }
.w.requesting{ display:flex!important; flex-direction:column; width:var(--_task-width,360px)!important; max-width:calc(100vw - 32px);
  max-height:var(--_task-height,calc(100vh - 32px)); overflow:auto;
  background:var(--_panel); }
.w.requesting > :not(.task){ display:none!important; }
.task[hidden]{ display:none; }
.task{ min-width:0; font-family:'IBM Plex Sans Condensed','Arial Narrow',sans-serif;
  color:var(--_ink); }
.task-head{ min-height:48px; display:flex; align-items:center; gap:14px;
  padding:0 18px; border-bottom:1.5px solid var(--_ringc); cursor:grab; }
.task-head .disc{ width:18px; height:18px; flex:none; }
.task-title{ min-width:0; margin:0; margin-left:auto; text-align:right;
  font-size:17px; font-weight:600; line-height:1.1; text-wrap:balance; }
.task-body{ min-width:0; max-height:calc(var(--_task-height,100vh) - 98px);
  overflow:auto; padding:16px 18px; font-size:18px; font-weight:400; line-height:1.5; }
.task-actions{ display:flex; min-height:48px; border-top:1.5px solid var(--_ringc); }
.task-actions[hidden]{ display:none; }
.task-ack{ width:100%; min-height:48px; padding:12px 18px; border:0;
  border-radius:0; background:var(--_ink); color:var(--_on);
  text-align:right; font:600 17px/1.1 'IBM Plex Sans Condensed','Arial Narrow',sans-serif; }
.task-ack:hover{ background:linear-gradient(var(--_wash-on),var(--_wash-on)),var(--_ink); }
.task-ack:focus-visible,.task-title:focus-visible{ outline:2px solid currentColor; outline-offset:-3px; }
@media (prefers-reduced-motion: reduce){
  :host,.w{ transition:none; }
  :host([alert]) .disc.st,:host([alert]) .share{ animation:none!important; }
}
"#;

/// The bar's shadow tree.
///
/// `.run` holds the canonical actions; [`crate::bar::apply_flip`] reorders its
/// real nodes so visual and focus order mirror together.
pub const BAR_HTML: &str = r#"<div class="w">
  <div class="bar" part="bar">
    <div class="header">
      <button class="fab" data-cell="sync" part="fab" aria-label="collapse bar"><span class="disc st"></span></button>
      <button class="space" data-cell="space" aria-expanded="false" aria-controls="fabb-actions"><span class="n"></span></button>
    </div>
    <nav class="run" id="fabb-actions" aria-label="space actions" hidden>
      <button class="action login" data-action="account" hidden><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M14 3h6v18h-6M3 12h12m-5-5 5 5-5 5"/></svg><span>add an account</span></button>
      <button class="action condition" data-action="condition" hidden><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><circle cx="12" cy="12" r="9"/><path d="M12 7v6m0 4h.01"/></svg><span></span></button>
      <button class="action share" data-cell="share" data-panel="share" aria-controls="share-panel" aria-expanded="false"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><circle cx="6" cy="12" r="2.5"/><circle cx="18" cy="5" r="2.5"/><circle cx="18" cy="19" r="2.5"/><path d="m8 11 8-5M8 13l8 5"/></svg><span>copy share link</span></button>
      <button class="action members" data-panel="members" aria-controls="members-panel" aria-expanded="false"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><circle cx="12" cy="10" r="2.5"/><circle cx="5" cy="5" r="2"/><circle cx="19" cy="5" r="2"/><path d="M7 21v-2a5 5 0 0 1 10 0v2M2 14v-2a3 3 0 0 1 3-3M22 14v-2a3 3 0 0 0-3-3"/></svg><span>view members</span></button>
      <button class="action agent" data-panel="agent" aria-controls="agent-panel" aria-expanded="false"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M12 9.5V6.5"/><circle cx="12" cy="4" r="2.5"/><rect x="1.5" y="9.5" width="21" height="13" rx="4"/><circle cx="8" cy="16" r="1.5"/><circle cx="16" cy="16" r="1.5"/></svg><span>connect agent</span></button>
      <button class="action home" data-action="home"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M8 20V4m-5 5 5-5 5 5M8 16q0 5 5 5h7"/></svg><span>go to tonk home</span></button>
      <button class="more" data-cell="more" tabindex="-1" hidden></button>
    </nav>
  </div>
  <section class="panel" id="share-panel" aria-label="share this space" hidden><div class="share-gate"><button class="share-continue"><span>add an account to share this space</span><b aria-hidden="true">&#9656;</b></button></div><p class="panel-message share-progress" aria-live="polite">creating share link…</p></section>
  <section class="panel" id="agent-panel" aria-label="connect agent" hidden><div class="agent-gate"><button class="agent-continue"><span>add an account to connect an agent</span><b aria-hidden="true">&#9656;</b></button></div><div class="panel-head"><button class="back">&#9666; menu</button><button class="panel-copy" hidden>copy prompt</button><button class="agent-retry" hidden>try again</button></div><p class="agent-status" aria-live="polite">create an agent invitation when you open this panel</p><pre class="panel-copytext" hidden></pre></section>
  <section class="panel" id="members-panel" aria-label="space members" hidden><div class="panel-head"><button class="back">&#9666; menu</button></div><div class="members-list" role="list" aria-live="polite"><p class="members-empty">no members are available</p></div></section>
  <div class="mw" aria-hidden="true"><slot name="menu"></slot></div>
  <section class="task" hidden>
    <header class="task-head">
      <span class="disc st" aria-hidden="true"></span>
      <h2 class="task-title" id="fabb-task-title" tabindex="-1"></h2>
    </header>
    <div class="task-body"><slot name="request"></slot></div>
    <div class="task-actions"><button class="task-ack" type="button">got it</button></div>
  </section>
</div>
<dialog class="request-layer" aria-labelledby="fabb-task-title"></dialog>"#;

/// The gap between the bar and a stack, and between blocks within one — the
/// 7px of pure page that makes a stack many blocks rather than one panel
/// (law 2). One number, referenced by the CSS above and asserted below.
pub const STACK_GAP_PX: i32 = 7;

/// The bar's light-DOM headless subscribers.
///
/// The stacks are slotted (`slot="menu"`), so they render; the subscribers
/// are not, so they do not. That is deliberate — an unslotted light child of
/// a shadow host is never rendered, which is exactly what a headless element
/// wants. `<ui-space-name>` and `<ui-sync-status>` subscribe to their space
/// and write `label` and `state` onto the bar, so the bar renders text and a
/// disc it owns rather than hosting foreign elements inside its cells.
///
/// Their output is projected into the shadow-owned v0.17 rail and its attached
/// share, agent, and members panels. Unslotted light children stay invisible,
/// which keeps space data from owning product chrome.
///
/// ## Glyphs
///
/// Every mark is geometry, not illustration (see the FABB glyph table): `+`
/// for new, `▸` for open, `↖` for leaving the environment, and a 6×12 ink
/// block for rename — the terminal block cursor again, as a noun. No icon
/// library.
pub const STACKS_HTML: &str = r#"<ui-sync-status headless with="main@{space}"></ui-sync-status>
<ui-space-name headless space="{space}"></ui-space-name>
<tonk-share headless space="{space}"></tonk-share>
<tonk-tool-connection headless space="{space}"></tonk-tool-connection>
<tonk-agent-panel headless space="{space}" with="main@{space}"></tonk-agent-panel>
<ui-member-roster headless space="{space}"></ui-member-roster>"#;

/// Styles for the slotted stack content.
///
/// These rules cannot live in a component's shadow CSS: slotted content is
/// styled by the DOCUMENT, and document styles beat `::slotted()`. So the
/// marks a stack row carries are painted here, in the light tree, next to the
/// markup that uses them.
pub const STACKS_CSS: &str = r#"
.fabb-tool-connection p{ margin:0; }
.fabb-tool-connection [data-tool-connection-status]{ margin-top:10px; }
.fabb-tool-connection tonk-button[hidden]{ display:none !important; }
/* the rename glyph — the block cursor as a noun, at the label's own size */
tonk-fab .rename-mark{ display:inline-block; width:6px; height:12px; background:currentColor; }
/* the headless subscribers render nothing; they are unslotted, but say so */
tonk-fab > ui-sync-status[headless],
tonk-fab > ui-space-name[headless],
tonk-fab > tonk-share[headless],
tonk-fab > tonk-tool-connection[headless],
tonk-fab > tonk-agent-panel[headless]{ display:none; }
/* the row producers render their rows as SIBLINGS (see stack_rows), so they
   hold nothing themselves — laid out they would only add a stack gap where
   they sit */
tonk-fab ui-space-switcher, tonk-fab ui-member-roster{ display:none; }
tonk-fab > [slot="request"]{ display:block; min-width:0; margin:0;
  color:var(--fabb-ink,#38182a);
  font:400 18px/1.5 'IBM Plex Sans Condensed','Arial Narrow',sans-serif;
  overflow-wrap:anywhere; text-wrap:pretty; }
tonk-fab > [slot="request"] [data-fabb-actions]{ display:flex; margin:16px -18px -16px; }
tonk-fab > [slot="request"] [data-fabb-result]{ flex:1; min-width:0; min-height:48px;
  border:0; border-radius:0; padding:12px 18px; text-align:left;
  color:var(--fabb-ink,#38182a); background:transparent;
  font:600 17px/1.1 'IBM Plex Sans Condensed','Arial Narrow',sans-serif; }
tonk-fab > [slot="request"] [data-fabb-result]:last-child{ text-align:right;
  color:var(--fabb-on-ink,#f7f6f5); background:var(--fabb-ink,#38182a); }
tonk-fab > [slot="request"] [data-fabb-result]:focus-visible{ outline:2px solid currentColor; outline-offset:-3px; }
/* DESIGN.md card surface: opaque so content cannot show through the roster. */
.fabb-members{ --fabb-panel:#fcfbfb; }
.fabb-members .mem-row{ display:flex; align-items:baseline; gap:8px; padding:8px 2px; border-bottom:1px solid rgba(127,127,120,.25); font-size:13.5px; overflow-wrap:anywhere; }
.fabb-members .mem-row:last-child{ border-bottom:none; }
.fabb-members .mem-you{ font-size:11px; opacity:.65; flex-shrink:0; }
.fabb-members .mem-self{ font-weight:600; }
tonk-fab [data-share-members]{ font-variant-numeric:tabular-nums; }
/* the share row answers in place: one word at a time, the copy state
   choosing which. idle is the default, so a row that has never been used —
   and one whose element never stamped a state — still reads "copy link". */
/* Copy always touches the bar side of the stack, including the overflow route. */
tonk-fab [data-share-link]{ order:-1; }
tonk-fab[up] [data-share-link]{ order:1; }
tonk-fab [data-share-link] .say{ display:none; }
/* idle before the element has ever stamped a state, and whenever it says so.
   `blocked` also reads as idle: a prompt is up asking the user a question,
   and the row behind it is offering the retry, not reporting a failure. */
tonk-fab [data-share-link]:not([data-share-state]) .say--idle,
tonk-fab [data-share-link][data-share-state="idle"] .say--idle,
tonk-fab [data-share-link][data-share-state="blocked"] .say--idle{ display:inline; }
tonk-fab [data-share-link][data-share-state="copying"] .say--copying{ display:inline; }
tonk-fab [data-share-link][data-share-state="copied"] .say--copied{ display:inline; }
tonk-fab [data-share-link][data-share-state="failed"] .say--failed{ display:inline; }
tonk-fab [data-share-link][data-activation-blocked] .say{ display:none; }
tonk-fab [data-share-link][data-activation-blocked] .say--activation{ display:inline; }
"#;

/// The share flow's repairable sync refusal.
///
/// Every member can share through its own delegation chain. The remaining
/// prompt handles a missing sync remote; `share.rs` rewrites its marked lines
/// per refusal class and drives the dialog's `open` property.
///
/// Mounted on `<body>` rather than inside the bar: these are modals, and an
/// unslotted light-DOM child of a shadow host never renders, so a dialog
/// parked there could not be shown at all.
///
/// The action run reads left-to-right as dismiss-then-commit, and the two
/// fuse flush — the fill boundary between quiet and primary IS the divider
/// (law 3), which is why there is no gap and no separator between them.
pub const REFUSAL_DIALOGS_HTML: &str = r#"<tonk-cluster id="fabb-connect-cluster" hidden>
  <p slot="statement" data-enable-sync-statement>connect this space</p>
  <tonk-field noun="sync server" value="" data-enable-sync-remote></tonk-field>
  <p slot="narrator"><span data-enable-sync-detail>This space only exists on this device.</span> <span data-enable-sync-action>Connect it so other people can open it.</span></p>
  <tonk-button slot="run" variant="primary" solid data-enable-sync-confirm>connect</tonk-button>
  <span slot="ghost">keep it on this device</span>
</tonk-cluster>
<tonk-dialog id="fabb-tool-connection-cluster" class="fabb-tool-connection" heading="connect a tool" hidden data-tool-space="">
  <p>give a tool access to this space under your account</p>
  <p data-tool-connection-status>creating a private link&hellip;</p>
  <tonk-button slot="actions" solid data-tool-copy-prompt disabled>copy agent prompt</tonk-button>
  <tonk-button slot="actions" variant="primary" solid data-tool-copy-link disabled>copy link</tonk-button>
  <tonk-button slot="actions" solid data-tool-retry hidden>try again</tonk-button>
</tonk-dialog>"#;

/// Stamp the space DID into [`STACKS_HTML`].
///
/// Each cross-branch child carries its OWN `space` / `with`: the routing
/// helpers read the element's own attribute and never walk ancestors, so a
/// child left unstamped is pointed at nothing rather than inheriting.
///
/// `<ui-sync-status>` is the one exception to the bare-DID form — its `with`
/// contract is `"branch@repo"` (see `crate::sync_status`), so it
/// is stamped `main@{did}`, which the template already spells out.
pub fn stacks_html(space_did: &str) -> String {
    STACKS_HTML.replace("{space}", space_did)
}

/// Every `{space}` slot in [`STACKS_HTML`], as `(selector, attribute,
/// prefix)` — the value written is `prefix` followed by the space DID.
///
/// One table, read twice. [`stacks_html`] stamps these slots when the
/// subtree is authored, and `element::restamp_space` writes the same
/// attributes again when a bar authored with a BLANK space finally
/// learns its own — the unsubstituted first projection the space route
/// hands it before `{id}` resolves.
///
/// The two lists used to be written out separately, and `<tonk-share>`
/// was in the first but not the second. A bar that came up blank
/// therefore kept `<tonk-share space="">` for the life of the page: its
/// invite subscription never opened (an empty subject is a query error)
/// and its click handler returned on the spot, so picking "copy link"
/// dispatched nothing at all — no mint, no spinner, no refusal. Keep
/// them one table, and `it_binds_every_space_slot` keeps it honest.
pub const SPACE_BINDINGS: &[(&str, &str, &str)] = &[
    // The sync disc's contract is `branch@repo`, not a bare DID.
    ("ui-sync-status", "with", "main@"),
    ("ui-space-name", "space", ""),
    ("tonk-share", "space", ""),
    ("tonk-tool-connection", "space", ""),
    ("tonk-agent-panel", "space", ""),
    ("tonk-agent-panel", "with", "main@"),
    ("ui-member-roster", "space", ""),
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skin::SKIN;

    #[test]
    fn it_exposes_the_mono_ink_palette() {
        // One scheme (law 8): the light twin is the whole palette, and every
        // default is the mono scheme's own value (COLOR.md).
        for declaration in [
            "--fabb-ink, #38182a",
            "--fabb-ink-soft, #5b4953",
            "--fabb-on-ink, #f7f6f5",
            "--fabb-sep, rgba(56,24,42,.28)",
            "--fabb-hover, rgba(56,24,42,.06)",
            "--fabb-press, rgba(56,24,42,.12)",
            "--fabb-ring, rgba(56,24,42,.85)",
        ] {
            assert!(
                SKIN.contains(declaration),
                "the shared FABB skin must expose `{declaration}`",
            );
        }
        assert!(!SKIN.contains("#34332b"), "the old olive ink must be gone");
        assert!(
            !SKIN.contains("rgba(43,44,20"),
            "the old olive alpha colors must be gone",
        );
        assert!(
            SKIN.contains("var(--fabb-ring"),
            "the internal ring must read the public ring token",
        );
    }

    #[test]
    fn it_omits_the_changes_rung() {
        // Deliberate — see the module docs and plan/fabb-conformance.md.
        // This guards against it reappearing as dead chrome.
        assert!(!BAR_HTML.contains("data-cell=\"changes\""));
        assert!(!BAR_CSS.contains(".changes"));
    }

    #[test]
    fn it_carries_the_v017_geometry_and_type_registers() {
        assert!(BAR_CSS.contains("--fabb-space-width:360px"));
        assert!(BAR_CSS.contains(".header{ height:48px"));
        assert!(BAR_CSS.contains(".fab{ width:48px"));
        assert!(BAR_CSS.contains(".fab .disc{ width:18px; height:18px"));
        assert!(BAR_CSS.contains("border-radius:25px"));
        assert!(BAR_CSS.contains("border:1.5px solid var(--_ringc)"));
        assert!(BAR_CSS.contains("font:600 17px/1.1"));
        assert!(BAR_CSS.contains("font:400 18px/1.5"));
    }

    #[test]
    fn it_joins_or_stacks_the_panel_inside_one_surface() {
        assert!(BAR_CSS.contains(".w.has-panel"));
        assert!(BAR_CSS.contains("+ 600px"));
        assert!(BAR_CSS.contains(".panel{ min-width:0; height:240px"));
        assert!(BAR_CSS.contains(".w.stacked.has-panel"));
        assert!(BAR_CSS.contains("border-top:1.5px solid var(--_ringc)"));
    }

    #[test]
    fn it_lets_the_user_word_through_untouched() {
        assert!(BAR_HTML.contains(r#"class="space""#));
        assert!(!BAR_CSS.contains("text-transform:lowercase"));
    }

    #[test]
    fn it_alerts_without_a_colour() {
        // Law 5: ink only. Alerts blink or wash; pointing at one calms it.
        assert!(BAR_CSS.contains(":host([alert]) .disc.st{ animation:fabb-blink"));
        assert!(BAR_CSS.contains(":host([alert]) .share{ animation:fabb-wash"));
        assert!(BAR_CSS.contains(":host([alert]) .share:hover{ animation:none; }"));
    }

    #[test]
    fn it_collapses_to_the_48px_circle_at_every_width() {
        assert!(BAR_CSS.contains(".w.collapsed{ width:51px"));
        assert!(BAR_CSS.contains("grid-template-columns:48px"));
        assert!(BAR_CSS.contains(".w.collapsed .space,.w.collapsed .run,.w.collapsed .panel"));
        assert!(!BAR_CSS.contains(".w.compact"));
    }

    #[test]
    fn it_keeps_the_way_out_but_hides_share_for_an_unknown_space() {
        assert!(BAR_CSS.contains(":host([data-unknown-space]) .share{ display:none; }"));
        assert!(BAR_HTML.contains(r#"data-cell="space""#));
    }

    #[test]
    fn it_moves_the_joined_panel_to_the_visual_inside_edge() {
        assert!(BAR_CSS.contains(".w.flip.has-panel:not(.stacked)"));
        assert!(BAR_CSS.contains(".w.flip.has-panel:not(.stacked) .bar{ grid-column:2"));
        assert!(BAR_CSS.contains("border-right:1.5px solid var(--_ringc)"));
    }

    #[test]
    fn it_holds_the_sync_disc_outside_the_action_run() {
        let run = BAR_HTML.find(r#"<nav class="run""#).expect("an action run");
        let circle = BAR_HTML.find(r#"class="fab""#).expect("the circle");
        assert!(
            circle < run,
            "the circle is the persistent bookend, not a retracting action"
        );
        assert!(!BAR_HTML.contains("data-cell=\"fold\""));
        assert!(!BAR_HTML.contains("tele"));
    }

    #[test]
    fn it_stamps_the_space_onto_every_cross_branch_child() {
        let html = stacks_html("did:key:z6Mk");
        // Each child must carry its OWN space: the routing helpers read the
        // element's own attribute and never walk ancestors, so an unstamped
        // child is pointed at nothing.
        assert!(html.contains(r#"<ui-space-name headless space="did:key:z6Mk""#));
        assert!(html.contains(r#"<ui-member-roster headless space="did:key:z6Mk""#));
        assert!(html.contains(
            r#"<tonk-agent-panel headless space="did:key:z6Mk" with="main@did:key:z6Mk""#
        ));
        assert!(!html.contains("ui-space-switcher"));
        // The sync disc's contract is branch@repo, not a bare DID.
        assert!(html.contains(r#"with="main@did:key:z6Mk""#));
        assert!(!html.contains("{space}"), "every slot must be substituted");
    }

    /// [`SPACE_BINDINGS`] must name every `{space}` slot, and no others.
    ///
    /// Both directions matter. A slot missing from the table is a child
    /// the restamp leaves pointed at nothing when the space arrives late
    /// — which is how `<tonk-share>` came to swallow every click. A
    /// table entry with no slot is a selector that matches nothing, and
    /// would fail silently in the other direction.
    #[test]
    fn it_binds_every_space_slot() {
        let did = "did:key:z6Mk";
        let html = stacks_html(did);
        for &(selector, attribute, prefix) in SPACE_BINDINGS {
            assert!(
                html.contains(&format!(r#"<{selector} "#))
                    || html.contains(&format!(r#"<{selector}>"#)),
                "{selector} is bound but never authored",
            );
            assert!(
                html.contains(&format!(r#"{attribute}="{prefix}{did}""#)),
                "{selector} must carry {attribute}=\"{prefix}{did}\"",
            );
        }
        assert_eq!(
            html.matches(did).count(),
            SPACE_BINDINGS.len(),
            "every authored slot must be bound, so the restamp reaches it",
        );
    }

    #[test]
    fn it_carries_the_bars_information_architecture() {
        let order: Vec<usize> = [
            "copy share link",
            "view members",
            "connect agent",
            "go to tonk home",
        ]
        .iter()
        .map(|label| {
            BAR_HTML
                .find(label)
                .unwrap_or_else(|| panic!("{label} present"))
        })
        .collect();
        assert!(
            order.windows(2).all(|pair| pair[0] < pair[1]),
            "the rail actions keep the reference order",
        );
        for removed in [
            "data-mi-new",
            "data-mi-open",
            "data-mi-rename",
            "data-mi-cfg",
        ] {
            assert!(!stacks_html("did:key:z6Mk").contains(removed));
        }
    }

    #[test]
    fn it_needs_no_compact_overflow_route() {
        let html = stacks_html("did:key:z6Mk");
        assert!(!html.contains("data-overflow"));
        assert!(!BAR_HTML.contains("more actions"));
        assert!(BAR_HTML.contains(r#"data-panel="share""#));
    }

    #[test]
    fn it_names_the_navigation_and_attached_panels() {
        assert!(
            !BAR_HTML.contains("aria-haspopup"),
            "disclosure triggers must not claim menu behavior"
        );
        for label in [
            "space actions",
            "share this space",
            "connect agent",
            "space members",
        ] {
            assert!(
                BAR_HTML.contains(&format!(r#"aria-label="{label}""#)),
                "the {label} surface must be named"
            );
        }
    }

    #[test]
    fn it_offers_the_way_back_out_of_the_space() {
        assert!(BAR_HTML.contains(r#"data-action="home""#));
        assert!(BAR_HTML.contains("go to tonk home"));
    }

    #[test]
    fn it_keeps_the_subscribers_headless() {
        // They write `label` and `state` onto the bar; they must render
        // nothing themselves. Unslotted is what achieves that — a light child
        // with no slot never renders inside a shadow host — and the CSS says
        // so out loud.
        let html = stacks_html("did:key:z6Mk");
        for headless in [
            "ui-sync-status",
            "ui-space-name",
            "tonk-share",
            "tonk-tool-connection",
        ] {
            let tag = html
                .split(&format!("<{headless}"))
                .nth(1)
                .expect("the subscriber");
            let tag = tag.split('>').next().expect("the tag closes");
            assert!(
                tag.contains("headless"),
                "{headless} must be marked headless"
            );
            assert!(
                !tag.contains("slot="),
                "{headless} must not be slotted, or it would render",
            );
            assert!(
                STACKS_CSS.contains(&format!("tonk-fab > {headless}[headless]")),
                "{headless} must be hidden explicitly, not only by being unslotted",
            );
        }
    }

    #[test]
    fn it_places_the_share_action_in_the_live_rail() {
        assert!(BAR_HTML.contains(r#"class="action share""#));
        assert!(BAR_HTML.contains("copy share link"));
        assert!(BAR_HTML.contains(r#"id="share-panel""#));
    }

    #[test]
    fn it_defaults_to_login_instead_of_copy_for_an_unattached_profile() {
        assert!(BAR_HTML.contains(r#"class="action login" data-action="account" hidden"#));
        assert!(BAR_HTML.contains("add an account"));
        assert!(BAR_HTML.contains("add an account to share this space"));
    }

    #[test]
    fn it_separates_person_invites_from_tool_connections() {
        let html = stacks_html("did:key:z6Mk");
        assert!(html.contains("<tonk-share headless space=\"did:key:z6Mk\""));
        assert!(html.contains("<tonk-tool-connection headless space=\"did:key:z6Mk\""));
        assert!(!BAR_HTML.contains("data-action=\"tool\""));
        assert!(!BAR_HTML.contains("connect a tool"));
        assert!(BAR_HTML.contains("connect agent"));
        assert!(
            REFUSAL_DIALOGS_HTML.contains("give a tool access to this space under your account")
        );
        assert!(REFUSAL_DIALOGS_HTML.contains("data-tool-copy-link"));
        assert!(REFUSAL_DIALOGS_HTML.contains("data-tool-copy-prompt"));
        assert!(REFUSAL_DIALOGS_HTML.contains("<tonk-dialog id=\"fabb-tool-connection-cluster\""));
        assert!(REFUSAL_DIALOGS_HTML.contains("heading=\"connect a tool\""));
        assert!(REFUSAL_DIALOGS_HTML.contains("slot=\"actions\" solid data-tool-copy-prompt"));
    }

    #[test]
    fn it_leaves_remote_selection_to_the_worker() {
        let html = stacks_html("did:key:z6Mk");
        assert!(!html.contains("tonk-default-remote"));
        assert!(!html.contains(r#"name="remote""#));
        assert!(!html.contains(r#"name="revocation""#));
    }

    #[test]
    fn it_draws_its_marks_as_geometry() {
        assert_eq!(BAR_HTML.matches("<svg").count(), 6);
        assert_eq!(
            BAR_HTML
                .matches(r#"stroke-linejoin="round" aria-hidden="true""#)
                .count(),
            6
        );
        assert!(BAR_HTML.contains("stroke=\"currentColor\""));
        assert!(BAR_HTML.contains(r#"<rect x="1.5" y="9.5" width="21" height="13" rx="4"/>"#));
        assert!(!BAR_HTML.contains("<wa-icon"));
    }

    #[test]
    fn it_gives_sync_a_connect_ceremony() {
        assert!(!REFUSAL_DIALOGS_HTML.contains(r#"id="fab-enable-sync""#));
        assert!(REFUSAL_DIALOGS_HTML.contains(r#"id="fabb-connect-cluster""#));
        assert!(REFUSAL_DIALOGS_HTML.contains("<tonk-cluster"));
        assert!(REFUSAL_DIALOGS_HTML.contains(r#"noun="sync server""#));
        assert!(REFUSAL_DIALOGS_HTML.contains("keep it on this device"));
        assert!(REFUSAL_DIALOGS_HTML.contains("data-enable-sync-confirm"));
        // The line share.rs rewrites per refusal class. Without the hook the
        // prompt is silently stuck on the `not-synced` wording.
        assert!(REFUSAL_DIALOGS_HTML.contains("data-enable-sync-detail"));
        assert!(REFUSAL_DIALOGS_HTML.contains("data-enable-sync-action"));
        assert!(!REFUSAL_DIALOGS_HTML.contains("fab-join-first"));
    }

    #[test]
    fn it_offers_a_way_out_of_every_prompt() {
        // The ceremony bails only through the cluster's ghost or Escape.
        assert_eq!(
            REFUSAL_DIALOGS_HTML
                .matches(r#"data-dialog="close""#)
                .count(),
            0
        );
        assert!(REFUSAL_DIALOGS_HTML.contains(r#"slot="ghost""#));
    }

    #[test]
    fn it_keeps_prompt_chrome_lowercase() {
        // Law 4. The prompt BODIES carry sentences and stay as written; the
        // headings and the action labels are chrome.
        assert!(REFUSAL_DIALOGS_HTML.contains(">connect</tonk-button>"));
    }

    #[test]
    fn it_respects_reduced_motion_on_every_transition() {
        let reduced = BAR_CSS
            .split("@media (prefers-reduced-motion: reduce)")
            .nth(1)
            .expect("reduced motion block");
        assert!(reduced.contains(":host,.w{ transition:none; }"));
        assert!(reduced.contains("animation:none!important"));
        assert!(!BAR_CSS.contains("transition:all"));
        assert!(!BAR_CSS.contains("transition: all"));
    }
}
