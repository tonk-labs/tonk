var v=16n,T=(1n<<v)-1n;function E(n,t){return BigInt(n)<<v|BigInt(t)&T}function m(n){return Number(n>>v)}function c(n){return Number(n&T)}var f=class{#n=0n;#e;constructor(t=()=>Date.now()){this.#e=t}get last(){return this.#n}tick(){let t=this.#e(),e=m(this.#n),r=Math.max(e,t),i=r===e?c(this.#n)+1:0;return this.#n=E(r,i),this.#n}receive(t){let e=this.#e(),r=m(this.#n),i=m(t),s=Math.max(r,i,e),o;return s===r&&s===i?o=Math.max(c(this.#n),c(t))+1:s===r?o=c(this.#n)+1:s===i?o=c(t)+1:o=0,this.#n=E(s,o),this.#n}};function x(n){let t=n.trim();if(!/^\d+$/.test(t))return null;try{return BigInt(t)}catch{return null}}function p(n){return n.toString()}var k="Tonk-Prose-Version",R="1";function P(n){return n.slice(0,k.length+1).toLowerCase()===`${k.toLowerCase()}:`}function g(n){if(!P(n))return{hlc:null,value:n};let t=n.indexOf(`\r
\r
`),e=n.indexOf(`

`),r,i;if(t!==-1&&(e===-1||t<=e))r=t,i=t+4;else if(e!==-1)r=e,i=e+2;else return{hlc:null,value:n};let s=n.slice(i),o=n.slice(0,r).split(/\r\n|\n/),l=null;for(let a of o){let u=a.indexOf(":");if(u===-1)continue;a.slice(0,u).trim().toLowerCase()==="etag"&&(l=x(a.slice(u+1).trim().replace(/^"|"$/g,"")))}return{hlc:l,value:s}}function w(n){return n.hlc===null?n.value:`${k}: ${R}\r
ETag: "${p(n.hlc)}"\r
Content-Type: text/markdown\r
\r
`+n.value}function A(){return{id:crypto.randomUUID(),time:Math.floor(Date.now()/1e3)}}var b=class{#n;#e;#t=[];#r=null;#i=!1;#s=null;#a=null;#u=null;#h=!1;#o=!1;#l;#d;#f;#c=!1;#g=!1;constructor(t,e,r={}){this.#n=t,this.#e=e,this.#l=r.pinned===!0,this.#d=r.onOpen,this.#f=r.onLarge}get pinned(){return this.#l}get readonly(){return this.#l||this.#g}get heads(){return this.#t}get opened(){return this.#r!==null}async open(){let t=await this.#n.read();this.#o||this.opened||(this.#t=t.heads,this.#r=t.content,this.#g=t.readonly===!0,this.#e.apply(t.content),this.#d?.())}get dirty(){return this.#a!==null||this.#r!==null&&!this.#e.same(this.#b(),this.#r)}flush(){return this.#s?(this.#h=!0,this.#s):(this.#s=this.#p().finally(()=>{this.#s=null}),this.#s)}#b(){return this.#u?this.#u.content:this.#e.current()}async#p(){if(this.#o||this.readonly||this.#r===null)return;if(this.#i){this.#h=!0;return}let t=this.#b();if(!this.#a&&this.#e.same(t,this.#r))return;let e=this.#a??={heads:[...this.#t],content:t,edits:this.#e.edits(this.#r,t),request:A()},r=e.content;this.#i=!0;try{let i=await this.#n.write(e.heads,e.edits,e.request);if(this.#o)return;this.#a=null,i.large===!0&&!this.#c&&(this.#c=!0,this.#f?.()),this.#e.same(this.#b(),r)?(this.#t=i.heads,this.#r=i.content,this.#u&&(this.#u.content=i.content),!this.#u&&!this.#e.same(i.content,r)&&this.#e.apply(i.content)):(this.#t=i.local??i.heads,this.#r=r,this.#h=!0)}finally{this.#i=!1}this.#h&&(this.#h=!1,await this.#p())}async poll(){if(this.#o)return;if(this.#r===null)return this.open();if(this.#l||this.#i||this.dirty)return;let t=this.#t,e=await this.#n.read();this.#o||this.#i||this.dirty||this.#t!==t||q(e.heads,this.#t)||(this.#t=e.heads,this.#r=e.content,this.#e.apply(e.content))}close(){this.#o=!0}async finish(){this.#u={content:this.#e.current()},await this.flush(),this.close()}};function C(n){return(n??"").split(/\s+/).filter(t=>t!=="")}function q(n,t){if(n.length!==t.length)return!1;let e=[...n].sort(),r=[...t].sort();return e.every((i,s)=>i===r[s])}function M(n,t){return{current:n,apply:t,same:(e,r)=>e===r,edits:(e,r)=>[{edit:"set-text",text:r}]}}function O(n,t,e,r,i=[]){let s={},o=async l=>{let a={...n.isConnected?{}:s,entity:t,format:e};l?a.write={...l,format:e}:i.length>0&&(a.heads=i);let u=new CustomEvent("tonk-document",{detail:a,bubbles:!0,composed:!0,cancelable:!0});if((n.isConnected?n:n.ownerDocument).dispatchEvent(u),!u.defaultPrevented||!(a.result instanceof Promise))throw new Error("tonk-document: no host answered");s={space:a.space,branch:a.branch,profile:a.profile};let h=await a.result;return{heads:h.heads??[],local:h.local,content:r(h),readonly:h.readonly===!0,large:h.large===!0}};return{read:()=>o(),write:(l,a,u)=>o({heads:l,edits:a,request:u})}}var S=["subject","at","content","value","readonly","placeholder","auto-focus","switcher","caret"],L="automerge/text@1",D=1500,H=400;async function I(){let n=globalThis.__tonkProseEditor;if(typeof n=="string"&&n)return n;if(typeof n=="function"){let t=await n();if(typeof t=="string"&&t)return t}return new URL("./tonk-prose-editor.js",import.meta.url).href}var d=null;function _(){return d||(d=I().then(n=>import(n).then(t=>t)),d.catch(()=>{d=null})),d}var y=class extends HTMLElement{static get observedAttributes(){return S}#n;#e;candidates=[];#t=null;#r=0;#i=null;#s=null;#a=null;#u=null;#h=new f;#o=0n;#l=null;#d=null;#f="";#c=!1;constructor(){super(),this.#n=this.attachShadow({mode:"open",delegatesFocus:!0});let t=document.createElement("style");t.textContent=B,this.#e=document.createElement("div"),this.#e.className="mount",this.#n.append(t,this.#e)}connectedCallback(){if(this.#c=!1,this.#s||(this.#s=new MutationObserver(()=>this.#b()),this.#s.observe(this,{childList:!0,characterData:!0,subtree:!0})),this.#t)return;let t=++this.#r;this.#x(t)}#g(){return this.textContent??""}#b(){this.#v(this.#g())}#p(){return(this.getAttribute("subject")??"")!==""}#y(){this.#E();let t=this.getAttribute("subject")??"",e=this.#t;if(t===""||!e)return;let r=C(this.getAttribute("at")),i=r.length>0,s=new b(O(this,t,L,o=>String(o.text??""),r),M(()=>e.getMarkdown(),o=>e.setMarkdown(o)),{pinned:i,onLarge:()=>{console.warn("[tonk-prose] the document is large; it stops taking edits at 8 MiB"),this.dispatchEvent(new CustomEvent("documentlarge",{bubbles:!0,composed:!0}))},onOpen:()=>{this.#f="",this.#k(),s.readonly&&!s.pinned&&this.#m(new Error("this document is in a newer format; update the app to edit it"))}});this.#l=s,this.#k(),s.open().catch(o=>this.#m(o)),this.#d=setInterval(()=>{if(document.visibilityState!=="visible")return;(s.opened&&s.dirty?s.flush():s.poll()).catch(l=>{s.opened||this.#m(l)})},D)}#k(){let t=this.#l,e=t!==null&&(t.readonly||!t.opened);this.#t?.setReadOnly(e||this.hasAttribute("readonly"))}#m(t){let e=t instanceof Error?t.message:String(t);e!==this.#f&&(this.#f=e,console.warn("[tonk-prose] document:",t),this.dispatchEvent(new CustomEvent("documenterror",{detail:{message:e},bubbles:!0,composed:!0})))}#E(){this.#d!==null&&(clearInterval(this.#d),this.#d=null),this.#l?.finish().catch(t=>this.#m(t)),this.#l=null}async#x(t){let e;try{e=await _()}catch(o){console.warn("[tonk-prose] failed to load editor core:",o);return}if(t!==this.#r||!this.isConnected)return;let r=this.#p()?"":this.#i;if(r===null){let o=this.#g();r=o!==""?o:this.getAttribute("content")??this.getAttribute("value")}let i="";if(r!==null){let o=g(r);i=o.value,o.hlc!==null&&o.hlc>this.#o&&(this.#o=this.#h.receive(o.hlc))}let s=e.createEditor(this.#e,{doc:i,readOnly:this.hasAttribute("readonly"),placeholder:this.getAttribute("placeholder")??"",onChange:o=>{this.#C(o)},switcher:this.hasAttribute("switcher")?{candidates:()=>this.candidates,onOpen:o=>this.#w("switch",o),onCreate:o=>this.#w("create",{title:o,document:this.value}),onSuggest:(o,l)=>this.#w("suggest",{rows:o,active:l})}:void 0});this.#i=null,this.#t=s,this.#p()&&this.#y(),this.dispatchEvent(new CustomEvent("ready",{detail:{editor:s},bubbles:!0,composed:!0})),!this.hasAttribute("readonly")&&this.hasAttribute("auto-focus")&&setTimeout(()=>{if(this.#t===s){try{window.focus()}catch{}this.getAttribute("caret")==="end"&&s.caretToEnd(),s.focus()}},0)}#w(t,e){this.dispatchEvent(new CustomEvent(t,{detail:e,bubbles:!0,composed:!0}))}#C(t){this.#u=t,this.#a!==null&&clearTimeout(this.#a),this.#a=setTimeout(()=>this.#T(),H)}#T(){this.#a=null;let t=this.#u;if(this.#u=null,t===null)return;if(this.#l){this.#l.flush().then(()=>{this.#f=""}).catch(i=>{this.#m(i)}),this.dispatchEvent(new CustomEvent("change",{detail:{value:t,content:t},bubbles:!0,composed:!0}));return}let e=this.#h.tick();this.#o=e;let r=w({hlc:e,value:t});this.dispatchEvent(new CustomEvent("change",{detail:{value:t,content:r},bubbles:!0,composed:!0}))}disconnectedCallback(){this.#c||(this.#c=!0,setTimeout(()=>{this.#c&&(this.#c=!1,!this.isConnected&&(this.#a!==null&&(clearTimeout(this.#a),this.#T()),this.#E(),this.#s?.disconnect(),this.#s=null,this.#r++,this.#t?.destroy(),this.#t=null))},0))}attributeChangedCallback(t,e,r){switch(t){case"subject":case"at":this.#t&&this.#p()&&this.#y();break;case"content":this.#v(r??"");break;case"value":(r??"")!==this.value&&this.#v(r??"");break;case"readonly":this.#k();break;case"placeholder":this.#t?.setPlaceholder(r??"");break;case"auto-focus":break}}#v(t){if(this.#p())return;let{hlc:e,value:r}=g(t);if(e!==null){if(e<=this.#o)return;this.#o=this.#h.receive(e)}if(!this.#t){this.#i=r;return}this.#t.setMarkdown(r)}get value(){if(this.#t)return this.#t.getMarkdown();if(this.#i!==null)return this.#i;let t=this.#g(),e=t!==""?t:this.getAttribute("content")??this.getAttribute("value");return e===null?"":g(e).value}set value(t){this.#v(t)}get content(){return w({hlc:this.#o,value:this.value})}set content(t){this.#v(t)}get version(){return p(this.#o)}focus(){this.#t?this.#t.focus():super.focus()}get editor(){return this.#t}},B=`
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
`;customElements.get("tonk-prose")||customElements.define("tonk-prose",y);
//# sourceMappingURL=tonk-prose.js.map
