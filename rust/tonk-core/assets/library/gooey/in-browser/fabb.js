// Vendored from tonk-labs/gooey in-browser/fabb.js at ready-to-tonk 406b6d7db17477fe7a5b071b3aa59c248c45d5e6.
// Unchanged below this comment: change it in gooey, then copy it here again.
/* ════════════════════════════════════════════════════════════════
   tonk-message — persistent feedback on Tonk's task pages · v0.23.0
   classic script · no dependencies · no build

   README.md is the API reference. FABB.version is the version — the ?v=
   cache-bust on every page that loads this file repeats it; bump them
   together.

     <script src="fabb.js"></script>

     <tonk-message>   persistent inline error, status or success; ink only

   skinning: --fabb-* knobs, then the --tonk-* values from tonk-look.css,
             then the same values as fallbacks.
   laws:     ink only, no semantic colours; Tonk's words lowercase, the
             person's words untouched.
   ════════════════════════════════════════════════════════════════ */
(() => {
  'use strict';
  if (customElements.get('tonk-message')) return;

  /* ── shared skin: the message's values, read from Tonk's look ── */
  const SKIN = `
    *, *::before, *::after{ box-sizing:border-box; }
    /* :host sets a display, which defeats the UA's [hidden] rule; restate it */
    :host([hidden]){ display:none !important; }
    .w{
      --_ink:   var(--fabb-ink, var(--tonk-text, #38182a));
      --_on:    var(--fabb-on-ink, var(--tonk-sheet, #f7f6f5));
      --_hover: var(--fabb-hover, var(--tonk-wash, rgba(56,24,42,.06)));
      --_press: var(--fabb-press, var(--tonk-wash-strong, rgba(56,24,42,.12)));
      --_control-leading: var(--fabb-control-leading, var(--tonk-leading-label, 1.3));
      --_message-leading: var(--fabb-message-leading, var(--tonk-leading-sentence, 1.5));
      --_control-font: 600 var(--fabb-control-size)/var(--_control-leading) var(--tonk-font-label, 'IBM Plex Sans Condensed','Arial Narrow',sans-serif);
      --_message-font: 400 var(--fabb-message-size, var(--tonk-size-sentence, 16px))/var(--_message-leading) var(--tonk-font-sentence, 'IBM Plex Sans',system-ui,sans-serif);
      -webkit-tap-highlight-color:transparent;
    }
    /* selection is ink only, never the browser's blue */
    .w ::selection{ background:var(--_ink); color:var(--_on); }
    button{ font:inherit; letter-spacing:inherit; color:inherit; background:none; border:0; padding:0; cursor:pointer; text-align:inherit; }
    /* focus is the press wash, never an outline (README: Not yet consistent);
       high contrast drops backgrounds, so it keeps an outline there */
    .w :is(button,[tabindex],input):focus-visible{ outline:none; background:var(--_press); }
    @media (forced-colors: active){ .w :is(button,[tabindex],input):focus-visible{ outline:2px solid Highlight; } }
    /* press: a control shrinks to 96% under the press wash, as on every Tonk surface */
    .w :is(button,a){ transition:scale 150ms ease-out; }
    .w :is(button,a):active{ scale:.96; }
    @media (prefers-reduced-motion: reduce){ .w :is(button,a){ transition:none; } .w :is(button,a):active{ scale:1; } }
  `;

  /* ── base: build and update ── */
  class Fabb extends HTMLElement {
    constructor(){ super(); this.attachShadow({ mode:'open' }); }
    connectedCallback(){
      if (!this._built){ this._built = true; this._render(); }
      this._update && this._update();
    }
    attributeChangedCallback(){
      if (!this._built) return;
      this._update && this._update();
    }
    _emit(type, detail){ this.dispatchEvent(new CustomEvent(type, { detail, bubbles:true, composed:true })); }
  }

  // Lucide check and circle-alert; see LUCIDE-LICENSE.txt for attribution.
  // Marks inherit the message scale.
  const MESSAGE_ICONS=Object.freeze({
    success:'<svg aria-hidden="true" focusable="false" xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" > <path d="M20 6 9 17l-5-5" /> </svg>',
    error:'<svg aria-hidden="true" focusable="false" xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" > <circle cx="12" cy="12" r="10" /> <line x1="12" x2="12" y1="8" y2="12" /> <line x1="12" x2="12.01" y1="16" y2="16" /> </svg>',
    status:''
  });

  /* ════════ tonk-message — persistent, local feedback ════════
     kind=error|status|success; heading; dismissible. No semantic colours,
     timeout or transient toast. Keep errors beside the action or field. */
  class TonkMessage extends Fabb {
    static observedAttributes = ['kind','heading','dismissible'];
    _render(){
      this.shadowRoot.innerHTML = `<style>${SKIN}
        /* a page's control size is a label's; a message's heading is a heading */
        :host{display:block;--fabb-control-size:var(--tonk-size-heading,16px)}
        :host(:focus-visible){outline:none}:host(:focus-visible) .w{background:var(--_press)}
        @media (forced-colors: active){:host(:focus-visible){outline:2px solid Highlight}}
        .w{display:flex;gap:12px;padding:var(--fabb-message-padding,14px 16px);color:var(--_ink);background:var(--fabb-message-ground,var(--_hover));font:var(--_message-font)}
        .mark:empty{display:none}.mark{font-size:inherit;line-height:inherit;flex:none;width:1em;height:calc(1em * var(--_message-leading));display:flex;align-items:center;justify-content:center}
        .mark svg{display:block;width:1em;height:1em;flex:none}
        .w:has(strong:not([hidden])) .mark{height:calc(var(--fabb-control-size) * var(--_control-leading))}.text{min-width:0;flex:1;overflow-wrap:anywhere}
        strong{display:block;font:var(--_control-font);margin-bottom:3px}.dismiss{position:relative;align-self:flex-start;min-width:32px;min-height:32px;margin:-6px -8px -6px 0}.dismiss::before{content:'';position:absolute;inset:-6px}
        @media (hover:hover) and (pointer:fine){.dismiss:hover{background:var(--_hover)}}.dismiss:active{background:var(--_press)}[hidden]{display:none!important}
      </style><div class="w" part="message"><span class="mark" aria-hidden="true"></span><div class="text"><strong hidden></strong><slot></slot></div><button type="button" class="dismiss" aria-label="dismiss message" hidden>×</button></div>`;
      this.shadowRoot.querySelector('.dismiss').addEventListener('click',()=>{this.hidden=true;this._emit('fabb-dismiss',{});});
    }
    _update(){
      const kind=this.getAttribute('kind') || 'status';
      this.setAttribute('role',kind==='error'?'alert':'status');this.setAttribute('aria-atomic','true');
      const title=this.shadowRoot.querySelector('strong');title.textContent=this.getAttribute('heading') || '';title.hidden=!title.textContent;
      this.shadowRoot.querySelector('.mark').innerHTML=MESSAGE_ICONS[kind]||MESSAGE_ICONS.status;
      this.shadowRoot.querySelector('.dismiss').hidden=!this.hasAttribute('dismissible');
    }
    show(message,{kind='status',heading=''}={}){
      this.setAttribute('kind',kind);this.setAttribute('heading',heading);this.hidden=false;this.textContent=message;
    }
  }

  customElements.define('tonk-message', TonkMessage);

  window.FABB = { version:'0.23.0', messageIcons:MESSAGE_ICONS };
})();
