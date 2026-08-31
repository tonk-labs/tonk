var b=16n,y=(1n<<b)-1n;function w(e,t){return BigInt(e)<<b|BigInt(t)&y}function p(e){return Number(e>>b)}function a(e){return Number(e&y)}var u=class{#e=0n;#o;constructor(t=()=>Date.now()){this.#o=t}get last(){return this.#e}tick(){let t=this.#o(),o=p(this.#e),r=Math.max(o,t),s=r===o?a(this.#e)+1:0;return this.#e=w(r,s),this.#e}receive(t){let o=this.#o(),r=p(this.#e),s=p(t),i=Math.max(r,s,o),n;return i===r&&i===s?n=Math.max(a(this.#e),a(t))+1:i===r?n=a(this.#e)+1:i===s?n=a(t)+1:n=0,this.#e=w(i,n),this.#e}};function E(e){let t=e.trim();if(!/^\d+$/.test(t))return null;try{return BigInt(t)}catch{return null}}function h(e){return e.toString()}var m="Tonk-Prose-Version",C="1";function x(e){return e.slice(0,m.length+1).toLowerCase()===`${m.toLowerCase()}:`}function d(e){if(!x(e))return{hlc:null,value:e};let t=e.indexOf(`\r
\r
`),o=e.indexOf(`

`),r,s;if(t!==-1&&(o===-1||t<=o))r=t,s=t+4;else if(o!==-1)r=o,s=o+2;else return{hlc:null,value:e};let i=e.slice(s),n=e.slice(0,r).split(/\r\n|\n/),c=null;for(let f of n){let g=f.indexOf(":");if(g===-1)continue;f.slice(0,g).trim().toLowerCase()==="etag"&&(c=E(f.slice(g+1).trim().replace(/^"|"$/g,"")))}return{hlc:c,value:i}}function v(e){return e.hlc===null?e.value:`${m}: ${C}\r
ETag: "${h(e.hlc)}"\r
Content-Type: text/markdown\r
\r
`+e.value}var A=["content","value","readonly","placeholder","auto-focus","switcher","caret"],O=400;async function M(){let e=globalThis.__tonkProseEditor;if(typeof e=="string"&&e)return e;if(typeof e=="function"){let t=await e();if(typeof t=="string"&&t)return t}return new URL("./tonk-prose-editor.js",import.meta.url).href}var l=null;function S(){return l||(l=M().then(e=>import(e).then(t=>t)),l.catch(()=>{l=null})),l}var k=class extends HTMLElement{static get observedAttributes(){return A}#e;#o;candidates=[];#t=null;#c=0;#i=null;#s=null;#n=null;#u=null;#h=new u;#r=0n;#a=!1;constructor(){super(),this.#e=this.attachShadow({mode:"open",delegatesFocus:!0});let t=document.createElement("style");t.textContent=T,this.#o=document.createElement("div"),this.#o.className="mount",this.#e.append(t,this.#o)}connectedCallback(){if(this.#a=!1,this.#s||(this.#s=new MutationObserver(()=>this.#p()),this.#s.observe(this,{childList:!0,characterData:!0,subtree:!0})),this.#t)return;let t=++this.#c;this.#b(t)}#d(){return this.textContent??""}#p(){this.#l(this.#d())}async#b(t){let o;try{o=await S()}catch(n){console.warn("[tonk-prose] failed to load editor core:",n);return}if(t!==this.#c||!this.isConnected)return;let r=this.#i;if(r===null){let n=this.#d();r=n!==""?n:this.getAttribute("content")??this.getAttribute("value")}let s="";if(r!==null){let n=d(r);s=n.value,n.hlc!==null&&n.hlc>this.#r&&(this.#r=this.#h.receive(n.hlc))}let i=o.createEditor(this.#o,{doc:s,readOnly:this.hasAttribute("readonly"),placeholder:this.getAttribute("placeholder")??"",onChange:n=>{this.#m(n)},switcher:this.hasAttribute("switcher")?{candidates:()=>this.candidates,onOpen:n=>this.#f("switch",n),onCreate:n=>this.#f("create",{title:n,document:this.value}),onSuggest:(n,c)=>this.#f("suggest",{rows:n,active:c})}:void 0});this.#i=null,this.#t=i,this.dispatchEvent(new CustomEvent("ready",{detail:{editor:i},bubbles:!0,composed:!0})),!this.hasAttribute("readonly")&&this.hasAttribute("auto-focus")&&setTimeout(()=>{if(this.#t===i){try{window.focus()}catch{}this.getAttribute("caret")==="end"&&i.caretToEnd(),i.focus()}},0)}#f(t,o){this.dispatchEvent(new CustomEvent(t,{detail:o,bubbles:!0,composed:!0}))}#m(t){this.#u=t,this.#n!==null&&clearTimeout(this.#n),this.#n=setTimeout(()=>this.#g(),O)}#g(){this.#n=null;let t=this.#u;if(this.#u=null,t===null)return;let o=this.#h.tick();this.#r=o;let r=v({hlc:o,value:t});this.dispatchEvent(new CustomEvent("change",{detail:{value:t,content:r},bubbles:!0,composed:!0}))}disconnectedCallback(){this.#a||(this.#a=!0,setTimeout(()=>{this.#a&&(this.#a=!1,!this.isConnected&&(this.#n!==null&&(clearTimeout(this.#n),this.#g()),this.#s?.disconnect(),this.#s=null,this.#c++,this.#t?.destroy(),this.#t=null))},0))}attributeChangedCallback(t,o,r){switch(t){case"content":this.#l(r??"");break;case"value":(r??"")!==this.value&&this.#l(r??"");break;case"readonly":this.#t?.setReadOnly(r!==null);break;case"placeholder":this.#t?.setPlaceholder(r??"");break;case"auto-focus":break}}#l(t){let{hlc:o,value:r}=d(t);if(o!==null){if(o<=this.#r)return;this.#r=this.#h.receive(o)}if(!this.#t){this.#i=r;return}this.#t.setMarkdown(r)}get value(){if(this.#t)return this.#t.getMarkdown();if(this.#i!==null)return this.#i;let t=this.#d(),o=t!==""?t:this.getAttribute("content")??this.getAttribute("value");return o===null?"":d(o).value}set value(t){this.#l(t)}get content(){return v({hlc:this.#r,value:this.value})}set content(t){this.#l(t)}get version(){return h(this.#r)}focus(){this.#t?this.#t.focus():super.focus()}get editor(){return this.#t}},T=`
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
`;customElements.get("tonk-prose")||customElements.define("tonk-prose",k);
//# sourceMappingURL=tonk-prose.js.map
