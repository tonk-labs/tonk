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
.bar{ min-width:0; position:relative; display:flex; flex-direction:column; }
:host([up]) .bar{ flex-direction:column-reverse; }
.header{ height:48px; display:flex; align-items:stretch; cursor:grab; touch-action:none; user-select:none; }
:host([dragging]) .header,:host([dragging]) .header button{ cursor:grabbing; }
.header:hover,.w.menu-open .header{ background:var(--_hover); }
.header:active{ background:var(--_press); }
button,a{ min-height:48px; font:600 17px/1.1 'IBM Plex Sans Condensed','Arial Narrow',sans-serif; }
.fab{ width:48px; flex:none; display:grid; place-items:center; touch-action:none; user-select:none; }
.fab .disc{ width:18px; height:18px; }
.space{ flex:1; min-width:0; display:flex; align-items:center; justify-content:flex-end;
  padding:0 28px 0 12px; text-align:right; white-space:nowrap; overflow:hidden; text-overflow:ellipsis; }
.space .n{ min-width:0; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
.run{ display:grid; }
.run[hidden],.panel[hidden],.more,.mw{ display:none!important; }
.action{ display:flex; align-items:center; justify-content:space-between; gap:18px;
  padding:0 28px 0 16px; text-align:right; text-decoration:none; color:var(--_ink); border-radius:0; }
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
.agent-status{ margin:0; padding:4px 24px 12px; font:400 16px/1.35 'IBM Plex Sans Condensed','Arial Narrow',sans-serif; }
.panel-message{ margin:auto; padding:24px; font:400 18px/1.5 'IBM Plex Sans Condensed','Arial Narrow',sans-serif; text-align:center; }
.members-list{ min-height:0; overflow:auto; padding:0 18px 16px; }
.members-list .mem-row{ min-height:42px; display:flex; align-items:center; justify-content:flex-end; gap:8px;
  border-bottom:1px solid var(--_sep); font:600 15px/1.2 'IBM Plex Sans Condensed','Arial Narrow',sans-serif; }
.members-list .mem-row:last-child{ border-bottom:0; }
.members-list .mem-you{ color:var(--_soft); font-size:12px; font-weight:500; }
.members-list .mem-self{ text-decoration:underline; text-underline-offset:3px; }
.members-empty{ margin:auto; padding:24px; font:400 18px/1.5 'IBM Plex Sans Condensed','Arial Narrow',sans-serif; }
.share-gate{ background:var(--_hover); display:grid; place-items:center; }
.share-continue{ display:flex; align-items:center; justify-content:center; gap:12px; padding:0 24px; text-align:center; }
.share-continue span{ text-decoration:underline; text-underline-offset:4px; }
:host(:not([data-account-required])) #share-panel .share-gate{ display:none; }
:host([data-account-required]) #share-panel .share-progress{ display:none; }
:host([data-unknown-space]) .share{ display:none; }
:host([alert]) .disc.st{ animation:fabb-blink var(--_blink) var(--_ease) infinite; }
:host([alert]) .share{ animation:fabb-wash var(--_blink) var(--_ease) infinite; }
:host([alert]) .share:hover{ animation:none; }
:host([data-account-required][alert]) .disc.st{ animation:none; }
.w.collapsed{ width:51px; grid-template-columns:48px; border-radius:50%; overflow:hidden; }
.w.collapsed .space,.w.collapsed .run,.w.collapsed .panel{ display:none!important; }
.w.flip.has-panel:not(.stacked){ grid-template-columns:minmax(0,1fr) calc(var(--fabb-space-width) - 3px); }
.w.flip.has-panel:not(.stacked) .bar{ grid-column:2; grid-row:1; }
.w.flip.has-panel:not(.stacked) .panel{ grid-column:1; grid-row:1; border-left:0; border-right:1.5px solid var(--_ringc); }
.w.flip .action{ flex-direction:row-reverse; justify-content:flex-start; padding:0 14px 0 16px; }
.w.flip .space{ padding:0 5px 0 12px; }
.w.stacked.has-panel{ grid-template-columns:minmax(0,1fr); width:min(var(--fabb-space-width),var(--_room,calc(100vw - 32px))); }
.w.stacked .panel{ border-left:0; border-top:1.5px solid var(--_ringc); max-height:max(96px,calc(var(--_height,550px) - 243px)); }
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
      <button class="action share" data-cell="share" data-panel="share" aria-controls="share-panel" aria-expanded="false"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><circle cx="6" cy="12" r="2.5"/><circle cx="18" cy="5" r="2.5"/><circle cx="18" cy="19" r="2.5"/><path d="m8 11 8-5M8 13l8 5"/></svg><span>copy share link</span></button>
      <button class="action members" data-panel="members" aria-controls="members-panel" aria-expanded="false"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><circle cx="12" cy="10" r="2.5"/><circle cx="5" cy="5" r="2"/><circle cx="19" cy="5" r="2"/><path d="M7 21v-2a5 5 0 0 1 10 0v2M2 14v-2a3 3 0 0 1 3-3M22 14v-2a3 3 0 0 0-3-3"/></svg><span>view members</span></button>
      <button class="action agent" data-panel="agent" aria-controls="agent-panel" aria-expanded="false"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><circle cx="6" cy="7" r="2.5"/><circle cx="18" cy="17" r="2.5"/><path d="M9 7h11M4 17h11"/></svg><span>connect agent</span></button>
      <button class="action tool" data-action="tool"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="2" y="4" width="20" height="16" rx="2"/><path d="m6 9 4 3-4 3m7 0h5"/></svg><span>connect a tool</span></button>
      <button class="action home" data-action="home"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M8 20V4m-5 5 5-5 5 5M8 16q0 5 5 5h7"/></svg><span>go to tonk home</span></button>
      <button class="more" data-cell="more" tabindex="-1" hidden></button>
    </nav>
  </div>
  <section class="panel" id="share-panel" aria-label="share this space" hidden><div class="share-gate"><button class="share-continue"><span>add an account to share this space</span><b aria-hidden="true">&#9656;</b></button></div><p class="panel-message share-progress" aria-live="polite">creating share link…</p></section>
  <section class="panel" id="agent-panel" aria-label="connect agent" hidden><div class="panel-head"><button class="back">&#9666; menu</button><button class="panel-copy" hidden>copy prompt</button><button class="agent-retry" hidden>try again</button></div><p class="agent-status" aria-live="polite">create an agent invitation when you open this panel</p><pre class="panel-copytext" hidden></pre></section>
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
<tonk-agent-panel headless space="{space}"></tonk-agent-panel>
<ui-member-roster headless space="{space}"></ui-member-roster>"#;
