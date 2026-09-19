var m=16n,E=(1n<<m)-1n;function y(n,t){return BigInt(n)<<m|BigInt(t)&E}function b(n){return Number(n>>m)}function u(n){return Number(n&E)}var d=class{#n=0n;#e;constructor(t=()=>Date.now()){this.#e=t}get last(){return this.#n}tick(){let t=this.#e(),r=b(this.#n),e=Math.max(r,t),s=e===r?u(this.#n)+1:0;return this.#n=y(e,s),this.#n}receive(t){let r=this.#e(),e=b(this.#n),s=b(t),i=Math.max(e,s,r),o;return i===e&&i===s?o=Math.max(u(this.#n),u(t))+1:i===e?o=u(this.#n)+1:i===s?o=u(t)+1:o=0,this.#n=y(i,o),this.#n}};function x(n){let t=n.trim();if(!/^\d+$/.test(t))return null;try{return BigInt(t)}catch{return null}}function f(n){return n.toString()}var v="Tonk-Prose-Version",A="1";function O(n){return n.slice(0,v.length+1).toLowerCase()===`${v.toLowerCase()}:`}function p(n){if(!O(n))return{hlc:null,value:n};let t=n.indexOf(`\r
\r
`),r=n.indexOf(`

`),e,s;if(t!==-1&&(r===-1||t<=r))e=t,s=t+4;else if(r!==-1)e=r,s=r+2;else return{hlc:null,value:n};let i=n.slice(s),o=n.slice(0,e).split(/\r\n|\n/),a=null;for(let c of o){let l=c.indexOf(":");if(l===-1)continue;c.slice(0,l).trim().toLowerCase()==="etag"&&(a=x(c.slice(l+1).trim().replace(/^"|"$/g,"")))}return{hlc:a,value:i}}function k(n){return n.hlc===null?n.value:`${v}: ${A}\r
ETag: "${f(n.hlc)}"\r
Content-Type: text/markdown\r
\r
`+n.value}var g=class{#n;#e;#t=[];#r=null;#s=!1;#i=!1;#o=!1;#a;constructor(t,r,e={}){this.#n=t,this.#e=r,this.#a=e.pinned===!0}get pinned(){return this.#a}get heads(){return this.#t}get opened(){return this.#r!==null}async open(){let t=await this.#n.read();this.#o||(this.#t=t.heads,this.#r=t.content,this.#e.apply(t.content))}get dirty(){return this.#r!==null&&!this.#e.same(this.#e.current(),this.#r)}async flush(){if(this.#o||this.#a||this.#r===null)return;if(this.#s){this.#i=!0;return}let t=this.#e.current();if(!this.#e.same(t,this.#r)){this.#s=!0;try{let r=this.#e.edits(this.#r,t),e=await this.#n.write(this.#t,r);if(this.#o)return;this.#e.same(this.#e.current(),t)?(this.#t=e.heads,this.#r=e.content,this.#e.same(e.content,t)||this.#e.apply(e.content)):(this.#t=e.local??e.heads,this.#r=t,this.#i=!0)}finally{this.#s=!1}this.#i&&(this.#i=!1,await this.flush())}}async poll(){if(this.#o||this.#a||this.#r===null||this.#s||this.dirty)return;let t=await this.#n.read();this.#o||this.#s||this.dirty||P(t.heads,this.#t)||(this.#t=t.heads,this.#r=t.content,this.#e.apply(t.content))}close(){this.#o=!0}};function T(n){return(n??"").split(/\s+/).filter(t=>t!=="")}function P(n,t){if(n.length!==t.length)return!1;let r=[...n].sort(),e=[...t].sort();return r.every((s,i)=>s===e[i])}function C(n,t){return{current:n,apply:t,same:(r,e)=>r===e,edits:(r,e)=>[{edit:"set-text",text:e}]}}function M(n,t,r,e,s=[]){let i=async o=>{let a={entity:t,format:r};o?a.write={...o,format:r}:s.length>0&&(a.heads=s);let c=new CustomEvent("tonk-document",{detail:a,bubbles:!0,composed:!0,cancelable:!0});if(n.dispatchEvent(c),!c.defaultPrevented||!(a.result instanceof Promise))throw new Error("tonk-document: no host answered");let l=await a.result;return{heads:l.heads??[],local:l.local,content:e(l)}};return{read:()=>i(),write:(o,a)=>i({heads:o,edits:a})}}var S=["subject","at","content","value","readonly","placeholder","auto-focus","switcher","caret"],R="automerge/text@1",L=1500,H=400;async function D(){let n=globalThis.__tonkProseEditor;if(typeof n=="string"&&n)return n;if(typeof n=="function"){let t=await n();if(typeof t=="string"&&t)return t}return new URL("./tonk-prose-editor.js",import.meta.url).href}var h=null;function _(){return h||(h=D().then(n=>import(n).then(t=>t)),h.catch(()=>{h=null})),h}var w=class extends HTMLElement{static get observedAttributes(){return S}#n;#e;candidates=[];#t=null;#r=0;#s=null;#i=null;#o=null;#a=null;#p=new d;#l=0n;#c=null;#d=null;#u=!1;constructor(){super(),this.#n=this.attachShadow({mode:"open",delegatesFocus:!0});let t=document.createElement("style");t.textContent=q,this.#e=document.createElement("div"),this.#e.className="mount",this.#n.append(t,this.#e)}connectedCallback(){if(this.#u=!1,this.#i||(this.#i=new MutationObserver(()=>this.#w()),this.#i.observe(this,{childList:!0,characterData:!0,subtree:!0})),this.#t)return;let t=++this.#r;this.#y(t)}#g(){return this.textContent??""}#w(){this.#h(this.#g())}#f(){return(this.getAttribute("subject")??"")!==""}#m(){this.#v();let t=this.getAttribute("subject")??"",r=this.#t;if(t===""||!r)return;let e=T(this.getAttribute("at")),s=e.length>0,i=new g(M(this,t,R,o=>String(o.text??""),e),C(()=>r.getMarkdown(),o=>r.setMarkdown(o)),{pinned:s});this.#c=i,r.setReadOnly(s||this.hasAttribute("readonly")),i.open().catch(o=>{console.warn("[tonk-prose] could not open the document:",o)}),!s&&(this.#d=setInterval(()=>{document.visibilityState==="visible"&&i.poll().catch(()=>{})},L))}#v(){this.#d!==null&&(clearInterval(this.#d),this.#d=null),this.#c?.close(),this.#c=null}async#y(t){let r;try{r=await _()}catch(o){console.warn("[tonk-prose] failed to load editor core:",o);return}if(t!==this.#r||!this.isConnected)return;let e=this.#f()?"":this.#s;if(e===null){let o=this.#g();e=o!==""?o:this.getAttribute("content")??this.getAttribute("value")}let s="";if(e!==null){let o=p(e);s=o.value,o.hlc!==null&&o.hlc>this.#l&&(this.#l=this.#p.receive(o.hlc))}let i=r.createEditor(this.#e,{doc:s,readOnly:this.hasAttribute("readonly"),placeholder:this.getAttribute("placeholder")??"",onChange:o=>{this.#E(o)},switcher:this.hasAttribute("switcher")?{candidates:()=>this.candidates,onOpen:o=>this.#b("switch",o),onCreate:o=>this.#b("create",{title:o,document:this.value}),onSuggest:(o,a)=>this.#b("suggest",{rows:o,active:a})}:void 0});this.#s=null,this.#t=i,this.#f()&&this.#m(),this.dispatchEvent(new CustomEvent("ready",{detail:{editor:i},bubbles:!0,composed:!0})),!this.hasAttribute("readonly")&&this.hasAttribute("auto-focus")&&setTimeout(()=>{if(this.#t===i){try{window.focus()}catch{}this.getAttribute("caret")==="end"&&i.caretToEnd(),i.focus()}},0)}#b(t,r){this.dispatchEvent(new CustomEvent(t,{detail:r,bubbles:!0,composed:!0}))}#E(t){this.#a=t,this.#o!==null&&clearTimeout(this.#o),this.#o=setTimeout(()=>this.#k(),H)}#k(){this.#o=null;let t=this.#a;if(this.#a=null,t===null)return;if(this.#c){this.#c.flush().catch(s=>{console.warn("[tonk-prose] the edit was not saved:",s)}),this.dispatchEvent(new CustomEvent("change",{detail:{value:t,content:t},bubbles:!0,composed:!0}));return}let r=this.#p.tick();this.#l=r;let e=k({hlc:r,value:t});this.dispatchEvent(new CustomEvent("change",{detail:{value:t,content:e},bubbles:!0,composed:!0}))}disconnectedCallback(){this.#u||(this.#u=!0,setTimeout(()=>{this.#u&&(this.#u=!1,!this.isConnected&&(this.#o!==null&&(clearTimeout(this.#o),this.#k()),this.#c?.flush().catch(()=>{}),this.#v(),this.#i?.disconnect(),this.#i=null,this.#r++,this.#t?.destroy(),this.#t=null))},0))}attributeChangedCallback(t,r,e){switch(t){case"subject":case"at":this.#t&&this.#f()&&this.#m();break;case"content":this.#h(e??"");break;case"value":(e??"")!==this.value&&this.#h(e??"");break;case"readonly":this.#t?.setReadOnly(e!==null||this.#c?.pinned===!0);break;case"placeholder":this.#t?.setPlaceholder(e??"");break;case"auto-focus":break}}#h(t){if(this.#f())return;let{hlc:r,value:e}=p(t);if(r!==null){if(r<=this.#l)return;this.#l=this.#p.receive(r)}if(!this.#t){this.#s=e;return}this.#t.setMarkdown(e)}get value(){if(this.#t)return this.#t.getMarkdown();if(this.#s!==null)return this.#s;let t=this.#g(),r=t!==""?t:this.getAttribute("content")??this.getAttribute("value");return r===null?"":p(r).value}set value(t){this.#h(t)}get content(){return k({hlc:this.#l,value:this.value})}set content(t){this.#h(t)}get version(){return f(this.#l)}focus(){this.#t?this.#t.focus():super.focus()}get editor(){return this.#t}},q=`
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
