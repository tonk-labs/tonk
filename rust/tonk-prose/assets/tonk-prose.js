var m=16n,E=(1n<<m)-1n;function y(r,t){return BigInt(r)<<m|BigInt(t)&E}function b(r){return Number(r>>m)}function c(r){return Number(r&E)}var d=class{#n=0n;#e;constructor(t=()=>Date.now()){this.#e=t}get last(){return this.#n}tick(){let t=this.#e(),e=b(this.#n),n=Math.max(e,t),i=n===e?c(this.#n)+1:0;return this.#n=y(n,i),this.#n}receive(t){let e=this.#e(),n=b(this.#n),i=b(t),s=Math.max(n,i,e),o;return s===n&&s===i?o=Math.max(c(this.#n),c(t))+1:s===n?o=c(this.#n)+1:s===i?o=c(t)+1:o=0,this.#n=y(s,o),this.#n}};function x(r){let t=r.trim();if(!/^\d+$/.test(t))return null;try{return BigInt(t)}catch{return null}}function f(r){return r.toString()}var v="Tonk-Prose-Version",M="1";function A(r){return r.slice(0,v.length+1).toLowerCase()===`${v.toLowerCase()}:`}function p(r){if(!A(r))return{hlc:null,value:r};let t=r.indexOf(`\r
\r
`),e=r.indexOf(`

`),n,i;if(t!==-1&&(e===-1||t<=e))n=t,i=t+4;else if(e!==-1)n=e,i=e+2;else return{hlc:null,value:r};let s=r.slice(i),o=r.slice(0,n).split(/\r\n|\n/),a=null;for(let u of o){let l=u.indexOf(":");if(l===-1)continue;u.slice(0,l).trim().toLowerCase()==="etag"&&(a=x(u.slice(l+1).trim().replace(/^"|"$/g,"")))}return{hlc:a,value:s}}function k(r){return r.hlc===null?r.value:`${v}: ${M}\r
ETag: "${f(r.hlc)}"\r
Content-Type: text/markdown\r
\r
`+r.value}var g=class{#n;#e;#t=[];#r=null;#s=!1;#i=!1;#o=!1;#l;#c;#a=!1;constructor(t,e,n={}){this.#n=t,this.#e=e,this.#l=n.pinned===!0,this.#c=n.onOpen}get pinned(){return this.#l}get readonly(){return this.#l||this.#a}get heads(){return this.#t}get opened(){return this.#r!==null}async open(){let t=await this.#n.read();this.#o||(this.#t=t.heads,this.#r=t.content,this.#a=t.readonly===!0,this.#e.apply(t.content),this.#c?.())}get dirty(){return this.#r!==null&&!this.#e.same(this.#e.current(),this.#r)}async flush(){if(this.#o||this.readonly||this.#r===null)return;if(this.#s){this.#i=!0;return}let t=this.#e.current();if(!this.#e.same(t,this.#r)){this.#s=!0;try{let e=this.#e.edits(this.#r,t),n=await this.#n.write(this.#t,e);if(this.#o)return;this.#e.same(this.#e.current(),t)?(this.#t=n.heads,this.#r=n.content,this.#e.same(n.content,t)||this.#e.apply(n.content)):(this.#t=n.local??n.heads,this.#r=t,this.#i=!0)}finally{this.#s=!1}this.#i&&(this.#i=!1,await this.flush())}}async poll(){if(this.#o)return;if(this.#r===null)return this.open();if(this.#l||this.#s||this.dirty)return;let t=await this.#n.read();this.#o||this.#s||this.dirty||P(t.heads,this.#t)||(this.#t=t.heads,this.#r=t.content,this.#e.apply(t.content))}close(){this.#o=!0}};function T(r){return(r??"").split(/\s+/).filter(t=>t!=="")}function P(r,t){if(r.length!==t.length)return!1;let e=[...r].sort(),n=[...t].sort();return e.every((i,s)=>i===n[s])}function C(r,t){return{current:r,apply:t,same:(e,n)=>e===n,edits:(e,n)=>[{edit:"set-text",text:n}]}}function O(r,t,e,n,i=[]){let s=async o=>{let a={entity:t,format:e};o?a.write={...o,format:e}:i.length>0&&(a.heads=i);let u=new CustomEvent("tonk-document",{detail:a,bubbles:!0,composed:!0,cancelable:!0});if(r.dispatchEvent(u),!u.defaultPrevented||!(a.result instanceof Promise))throw new Error("tonk-document: no host answered");let l=await a.result;return{heads:l.heads??[],local:l.local,content:n(l),readonly:l.readonly===!0}};return{read:()=>s(),write:(o,a)=>s({heads:o,edits:a})}}var S=["subject","at","content","value","readonly","placeholder","auto-focus","switcher","caret"],R="automerge/text@1",L=1500,D=400;async function H(){let r=globalThis.__tonkProseEditor;if(typeof r=="string"&&r)return r;if(typeof r=="function"){let t=await r();if(typeof t=="string"&&t)return t}return new URL("./tonk-prose-editor.js",import.meta.url).href}var h=null;function _(){return h||(h=H().then(r=>import(r).then(t=>t)),h.catch(()=>{h=null})),h}var w=class extends HTMLElement{static get observedAttributes(){return S}#n;#e;candidates=[];#t=null;#r=0;#s=null;#i=null;#o=null;#l=null;#c=new d;#a=0n;#u=null;#f=null;#g="";#h=!1;constructor(){super(),this.#n=this.attachShadow({mode:"open",delegatesFocus:!0});let t=document.createElement("style");t.textContent=q,this.#e=document.createElement("div"),this.#e.className="mount",this.#n.append(t,this.#e)}connectedCallback(){if(this.#h=!1,this.#i||(this.#i=new MutationObserver(()=>this.#x()),this.#i.observe(this,{childList:!0,characterData:!0,subtree:!0})),this.#t)return;let t=++this.#r;this.#T(t)}#b(){return this.textContent??""}#x(){this.#d(this.#b())}#p(){return(this.getAttribute("subject")??"")!==""}#w(){this.#y();let t=this.getAttribute("subject")??"",e=this.#t;if(t===""||!e)return;let n=T(this.getAttribute("at")),i=n.length>0,s=new g(O(this,t,R,o=>String(o.text??""),n),C(()=>e.getMarkdown(),o=>e.setMarkdown(o)),{pinned:i,onOpen:()=>{this.#g="",this.#m(),s.readonly&&!s.pinned&&this.#v(new Error("this document is in a newer format; update the app to edit it"))}});this.#u=s,this.#m(),s.open().catch(o=>this.#v(o)),this.#f=setInterval(()=>{document.visibilityState==="visible"&&s.poll().catch(o=>{s.opened||this.#v(o)})},L)}#m(){let t=this.#u,e=t!==null&&(t.readonly||!t.opened);this.#t?.setReadOnly(e||this.hasAttribute("readonly"))}#v(t){let e=t instanceof Error?t.message:String(t);e!==this.#g&&(this.#g=e,console.warn("[tonk-prose] could not open the document:",t),this.dispatchEvent(new CustomEvent("documenterror",{detail:{message:e},bubbles:!0,composed:!0})))}#y(){this.#f!==null&&(clearInterval(this.#f),this.#f=null),this.#u?.close(),this.#u=null}async#T(t){let e;try{e=await _()}catch(o){console.warn("[tonk-prose] failed to load editor core:",o);return}if(t!==this.#r||!this.isConnected)return;let n=this.#p()?"":this.#s;if(n===null){let o=this.#b();n=o!==""?o:this.getAttribute("content")??this.getAttribute("value")}let i="";if(n!==null){let o=p(n);i=o.value,o.hlc!==null&&o.hlc>this.#a&&(this.#a=this.#c.receive(o.hlc))}let s=e.createEditor(this.#e,{doc:i,readOnly:this.hasAttribute("readonly"),placeholder:this.getAttribute("placeholder")??"",onChange:o=>{this.#C(o)},switcher:this.hasAttribute("switcher")?{candidates:()=>this.candidates,onOpen:o=>this.#k("switch",o),onCreate:o=>this.#k("create",{title:o,document:this.value}),onSuggest:(o,a)=>this.#k("suggest",{rows:o,active:a})}:void 0});this.#s=null,this.#t=s,this.#p()&&this.#w(),this.dispatchEvent(new CustomEvent("ready",{detail:{editor:s},bubbles:!0,composed:!0})),!this.hasAttribute("readonly")&&this.hasAttribute("auto-focus")&&setTimeout(()=>{if(this.#t===s){try{window.focus()}catch{}this.getAttribute("caret")==="end"&&s.caretToEnd(),s.focus()}},0)}#k(t,e){this.dispatchEvent(new CustomEvent(t,{detail:e,bubbles:!0,composed:!0}))}#C(t){this.#l=t,this.#o!==null&&clearTimeout(this.#o),this.#o=setTimeout(()=>this.#E(),D)}#E(){this.#o=null;let t=this.#l;if(this.#l=null,t===null)return;if(this.#u){this.#u.flush().catch(i=>{console.warn("[tonk-prose] the edit was not saved:",i)}),this.dispatchEvent(new CustomEvent("change",{detail:{value:t,content:t},bubbles:!0,composed:!0}));return}let e=this.#c.tick();this.#a=e;let n=k({hlc:e,value:t});this.dispatchEvent(new CustomEvent("change",{detail:{value:t,content:n},bubbles:!0,composed:!0}))}disconnectedCallback(){this.#h||(this.#h=!0,setTimeout(()=>{this.#h&&(this.#h=!1,!this.isConnected&&(this.#o!==null&&(clearTimeout(this.#o),this.#E()),this.#u?.flush().catch(()=>{}),this.#y(),this.#i?.disconnect(),this.#i=null,this.#r++,this.#t?.destroy(),this.#t=null))},0))}attributeChangedCallback(t,e,n){switch(t){case"subject":case"at":this.#t&&this.#p()&&this.#w();break;case"content":this.#d(n??"");break;case"value":(n??"")!==this.value&&this.#d(n??"");break;case"readonly":this.#m();break;case"placeholder":this.#t?.setPlaceholder(n??"");break;case"auto-focus":break}}#d(t){if(this.#p())return;let{hlc:e,value:n}=p(t);if(e!==null){if(e<=this.#a)return;this.#a=this.#c.receive(e)}if(!this.#t){this.#s=n;return}this.#t.setMarkdown(n)}get value(){if(this.#t)return this.#t.getMarkdown();if(this.#s!==null)return this.#s;let t=this.#b(),e=t!==""?t:this.getAttribute("content")??this.getAttribute("value");return e===null?"":p(e).value}set value(t){this.#d(t)}get content(){return k({hlc:this.#a,value:this.value})}set content(t){this.#d(t)}get version(){return f(this.#a)}focus(){this.#t?this.#t.focus():super.focus()}get editor(){return this.#t}},q=`
  :host {
    --tonk-prose-font: var(--wa-font-family-body, ui-sans-serif, -apple-system,
                       "Segoe UI", Helvetica, Arial, sans-serif);
    --tonk-prose-mono: var(--wa-font-family-code, ui-monospace, SFMono-Regular,
                       Menlo, Consolas, "Liberation Mono", monospace);
    --tonk-prose-heading-font: var(--wa-font-family-heading,
                       var(--tonk-prose-font));
    --tonk-prose-font-size: var(--wa-font-size-m, 1rem);
    --tonk-prose-radius: var(--wa-border-radius-m, 6px);
    --tonk-prose-padding: 1rem 1.25rem;
    --tonk-prose-max-width: none;

    /* Surfaces & text \u2014 inherit the page's WebAwesome tokens, GitHub
       light values as the standalone fallback. */
    --tonk-prose-bg: var(--wa-color-surface-default, #ffffff);
    --tonk-prose-fg: var(--wa-color-text-normal, #1f2328);
    --tonk-prose-fg-muted: var(--wa-color-text-quiet, #59636e);
    --tonk-prose-border: var(--wa-color-neutral-border-quiet, #d1d9e0);
    /* Links \u2192 the page's dedicated link color (readable on any surface);
       accent (caret, focus ring) \u2192 the yellow-green brand. */
    --tonk-prose-link: var(--wa-color-text-link, #0969da);
    --tonk-prose-accent: var(--wa-color-brand-fill-loud, #0969da);
    --tonk-prose-selection: var(--wa-color-brand-fill-quiet, #0969da33);
    --tonk-prose-focus-ring: var(--wa-color-brand-border-normal, #0969da66);
    /* Revealed markdown syntax markers (the Typora trick). */
    --tonk-prose-marker: var(--wa-color-text-quiet, #9198a1);
    /* Inline code + code block surfaces. */
    --tonk-prose-code-bg: var(--wa-color-neutral-fill-quiet, #f6f8fa);
    --tonk-prose-code-fg: var(--wa-color-text-normal, #1f2328);
    --tonk-prose-blockquote: var(--wa-color-text-quiet, #59636e);
    /* Highlight (== marks) \u2192 the page's LOUD brand fill (bright
       yellow-green) with its matching on-color, a readable dark-on-bright
       pairing in both themes (the normal fill is too dark for text). */
    --tonk-prose-highlight-bg: var(--wa-color-brand-fill-loud, #fef08a);
    --tonk-prose-highlight-fg: var(--wa-color-brand-on-loud, #1f2328);

    display: block;
    position: relative;
    box-sizing: border-box;
    background: var(--tonk-prose-bg);
    color: var(--tonk-prose-fg);
    border: 1px solid var(--tonk-prose-border);
    border-radius: var(--tonk-prose-radius);
    overflow: hidden;
    transition: border-color 120ms ease, box-shadow 120ms ease;
  }

  /* Standalone dark fallback (no WebAwesome tokens present). When the page
     provides \`--wa-*\` the rules above already track its light/dark
     palette, so this only bites a bare page in dark mode. */
  @media (prefers-color-scheme: dark) {
    :host {
      --tonk-prose-bg: var(--wa-color-surface-default, #0d1117);
      --tonk-prose-fg: var(--wa-color-text-normal, #f0f6fc);
      --tonk-prose-fg-muted: var(--wa-color-text-quiet, #9198a1);
      --tonk-prose-border: var(--wa-color-neutral-border-quiet, #3d444d);
      --tonk-prose-link: var(--wa-color-text-link, #48b9f4);
      --tonk-prose-accent: var(--wa-color-brand-fill-loud, #1f6feb);
      --tonk-prose-selection: var(--wa-color-brand-fill-quiet, #1f6feb59);
      --tonk-prose-focus-ring: var(--wa-color-brand-border-normal, #1f6feb99);
      --tonk-prose-marker: var(--wa-color-text-quiet, #6e7681);
      --tonk-prose-code-bg: var(--wa-color-neutral-fill-quiet, #151b23);
      --tonk-prose-code-fg: var(--wa-color-text-normal, #f0f6fc);
      --tonk-prose-blockquote: var(--wa-color-text-quiet, #9198a1);
      --tonk-prose-highlight-bg: var(--wa-color-brand-fill-loud, #fef08a);
      --tonk-prose-highlight-fg: var(--wa-color-brand-on-loud, #1f2328);
    }
  }

  :host([hidden]) { display: none; }

  :host(:focus-within) {
    border-color: var(--tonk-prose-accent);
    box-shadow: 0 0 0 2px var(--tonk-prose-focus-ring);
  }

  .mount { height: 100%; }
`;customElements.get("tonk-prose")||customElements.define("tonk-prose",w);
//# sourceMappingURL=tonk-prose.js.map
