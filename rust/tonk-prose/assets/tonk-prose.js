var m=16n,E=(1n<<m)-1n;function y(r,t){return BigInt(r)<<m|BigInt(t)&E}function b(r){return Number(r>>m)}function c(r){return Number(r&E)}var h=class{#r=0n;#e;constructor(t=()=>Date.now()){this.#e=t}get last(){return this.#r}tick(){let t=this.#e(),n=b(this.#r),e=Math.max(n,t),s=e===n?c(this.#r)+1:0;return this.#r=y(e,s),this.#r}receive(t){let n=this.#e(),e=b(this.#r),s=b(t),i=Math.max(e,s,n),o;return i===e&&i===s?o=Math.max(c(this.#r),c(t))+1:i===e?o=c(this.#r)+1:i===s?o=c(t)+1:o=0,this.#r=y(i,o),this.#r}};function T(r){let t=r.trim();if(!/^\d+$/.test(t))return null;try{return BigInt(t)}catch{return null}}function d(r){return r.toString()}var v="Tonk-Prose-Version",M="1";function A(r){return r.slice(0,v.length+1).toLowerCase()===`${v.toLowerCase()}:`}function f(r){if(!A(r))return{hlc:null,value:r};let t=r.indexOf(`\r
\r
`),n=r.indexOf(`

`),e,s;if(t!==-1&&(n===-1||t<=n))e=t,s=t+4;else if(n!==-1)e=n,s=n+2;else return{hlc:null,value:r};let i=r.slice(s),o=r.slice(0,e).split(/\r\n|\n/),a=null;for(let l of o){let g=l.indexOf(":");if(g===-1)continue;l.slice(0,g).trim().toLowerCase()==="etag"&&(a=T(l.slice(g+1).trim().replace(/^"|"$/g,"")))}return{hlc:a,value:i}}function k(r){return r.hlc===null?r.value:`${v}: ${M}\r
ETag: "${d(r.hlc)}"\r
Content-Type: text/markdown\r
\r
`+r.value}var p=class{#r;#e;#t=[];#n=null;#s=!1;#i=!1;#o=!1;constructor(t,n){this.#r=t,this.#e=n}get heads(){return this.#t}get opened(){return this.#n!==null}async open(){let t=await this.#r.read();this.#o||(this.#t=t.heads,this.#n=t.content,this.#e.apply(t.content))}get dirty(){return this.#n!==null&&!this.#e.same(this.#e.current(),this.#n)}async flush(){if(this.#o||this.#n===null)return;if(this.#s){this.#i=!0;return}let t=this.#e.current();if(!this.#e.same(t,this.#n)){this.#s=!0;try{let n=this.#e.edits(this.#n,t),e=await this.#r.write(this.#t,n);if(this.#o)return;this.#e.same(this.#e.current(),t)?(this.#t=e.heads,this.#n=e.content,this.#e.same(e.content,t)||this.#e.apply(e.content)):(this.#t=e.local??e.heads,this.#n=t,this.#i=!0)}finally{this.#s=!1}this.#i&&(this.#i=!1,await this.flush())}}async poll(){if(this.#o||this.#n===null||this.#s||this.dirty)return;let t=await this.#r.read();this.#o||this.#s||this.dirty||O(t.heads,this.#t)||(this.#t=t.heads,this.#n=t.content,this.#e.apply(t.content))}close(){this.#o=!0}};function O(r,t){if(r.length!==t.length)return!1;let n=[...r].sort(),e=[...t].sort();return n.every((s,i)=>s===e[i])}function x(r,t){return{current:r,apply:t,same:(n,e)=>n===e,edits:(n,e)=>[{edit:"set-text",text:e}]}}function C(r,t,n,e){let s=async i=>{let o={entity:t,format:n};i&&(o.write={...i,format:n});let a=new CustomEvent("tonk-document",{detail:o,bubbles:!0,composed:!0,cancelable:!0});if(r.dispatchEvent(a),!a.defaultPrevented||!(o.result instanceof Promise))throw new Error("tonk-document: no host answered");let l=await o.result;return{heads:l.heads??[],local:l.local,content:e(l)}};return{read:()=>s(),write:(i,o)=>s({heads:i,edits:o})}}var P=["subject","content","value","readonly","placeholder","auto-focus","switcher","caret"],S="automerge/text@1",R=1500,L=400;async function D(){let r=globalThis.__tonkProseEditor;if(typeof r=="string"&&r)return r;if(typeof r=="function"){let t=await r();if(typeof t=="string"&&t)return t}return new URL("./tonk-prose-editor.js",import.meta.url).href}var u=null;function H(){return u||(u=D().then(r=>import(r).then(t=>t)),u.catch(()=>{u=null})),u}var w=class extends HTMLElement{static get observedAttributes(){return P}#r;#e;candidates=[];#t=null;#n=0;#s=null;#i=null;#o=null;#d=null;#f=new h;#a=0n;#l=null;#h=null;#c=!1;constructor(){super(),this.#r=this.attachShadow({mode:"open",delegatesFocus:!0});let t=document.createElement("style");t.textContent=_,this.#e=document.createElement("div"),this.#e.className="mount",this.#r.append(t,this.#e)}connectedCallback(){if(this.#c=!1,this.#i||(this.#i=new MutationObserver(()=>this.#w()),this.#i.observe(this,{childList:!0,characterData:!0,subtree:!0})),this.#t)return;let t=++this.#n;this.#y(t)}#p(){return this.textContent??""}#w(){this.#u(this.#p())}#g(){return(this.getAttribute("subject")??"")!==""}#m(){this.#v();let t=this.getAttribute("subject")??"",n=this.#t;if(t===""||!n)return;let e=new p(C(this,t,S,s=>String(s.text??"")),x(()=>n.getMarkdown(),s=>n.setMarkdown(s)));this.#l=e,e.open().catch(s=>{console.warn("[tonk-prose] could not open the document:",s)}),this.#h=setInterval(()=>{document.visibilityState==="visible"&&e.poll().catch(()=>{})},R)}#v(){this.#h!==null&&(clearInterval(this.#h),this.#h=null),this.#l?.close(),this.#l=null}async#y(t){let n;try{n=await H()}catch(o){console.warn("[tonk-prose] failed to load editor core:",o);return}if(t!==this.#n||!this.isConnected)return;let e=this.#g()?"":this.#s;if(e===null){let o=this.#p();e=o!==""?o:this.getAttribute("content")??this.getAttribute("value")}let s="";if(e!==null){let o=f(e);s=o.value,o.hlc!==null&&o.hlc>this.#a&&(this.#a=this.#f.receive(o.hlc))}let i=n.createEditor(this.#e,{doc:s,readOnly:this.hasAttribute("readonly"),placeholder:this.getAttribute("placeholder")??"",onChange:o=>{this.#E(o)},switcher:this.hasAttribute("switcher")?{candidates:()=>this.candidates,onOpen:o=>this.#b("switch",o),onCreate:o=>this.#b("create",{title:o,document:this.value}),onSuggest:(o,a)=>this.#b("suggest",{rows:o,active:a})}:void 0});this.#s=null,this.#t=i,this.#g()&&this.#m(),this.dispatchEvent(new CustomEvent("ready",{detail:{editor:i},bubbles:!0,composed:!0})),!this.hasAttribute("readonly")&&this.hasAttribute("auto-focus")&&setTimeout(()=>{if(this.#t===i){try{window.focus()}catch{}this.getAttribute("caret")==="end"&&i.caretToEnd(),i.focus()}},0)}#b(t,n){this.dispatchEvent(new CustomEvent(t,{detail:n,bubbles:!0,composed:!0}))}#E(t){this.#d=t,this.#o!==null&&clearTimeout(this.#o),this.#o=setTimeout(()=>this.#k(),L)}#k(){this.#o=null;let t=this.#d;if(this.#d=null,t===null)return;if(this.#l){this.#l.flush().catch(s=>{console.warn("[tonk-prose] the edit was not saved:",s)}),this.dispatchEvent(new CustomEvent("change",{detail:{value:t,content:t},bubbles:!0,composed:!0}));return}let n=this.#f.tick();this.#a=n;let e=k({hlc:n,value:t});this.dispatchEvent(new CustomEvent("change",{detail:{value:t,content:e},bubbles:!0,composed:!0}))}disconnectedCallback(){this.#c||(this.#c=!0,setTimeout(()=>{this.#c&&(this.#c=!1,!this.isConnected&&(this.#o!==null&&(clearTimeout(this.#o),this.#k()),this.#l?.flush().catch(()=>{}),this.#v(),this.#i?.disconnect(),this.#i=null,this.#n++,this.#t?.destroy(),this.#t=null))},0))}attributeChangedCallback(t,n,e){switch(t){case"subject":this.#t&&this.#m();break;case"content":this.#u(e??"");break;case"value":(e??"")!==this.value&&this.#u(e??"");break;case"readonly":this.#t?.setReadOnly(e!==null);break;case"placeholder":this.#t?.setPlaceholder(e??"");break;case"auto-focus":break}}#u(t){if(this.#g())return;let{hlc:n,value:e}=f(t);if(n!==null){if(n<=this.#a)return;this.#a=this.#f.receive(n)}if(!this.#t){this.#s=e;return}this.#t.setMarkdown(e)}get value(){if(this.#t)return this.#t.getMarkdown();if(this.#s!==null)return this.#s;let t=this.#p(),n=t!==""?t:this.getAttribute("content")??this.getAttribute("value");return n===null?"":f(n).value}set value(t){this.#u(t)}get content(){return k({hlc:this.#a,value:this.value})}set content(t){this.#u(t)}get version(){return d(this.#a)}focus(){this.#t?this.#t.focus():super.focus()}get editor(){return this.#t}},_=`
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
