var p=16n,y=(1n<<p)-1n;function w(e,t){return BigInt(e)<<p|BigInt(t)&y}function g(e){return Number(e>>p)}function a(e){return Number(e&y)}var c=class{#e=0n;#o;constructor(t=()=>Date.now()){this.#o=t}get last(){return this.#e}tick(){let t=this.#o(),r=g(this.#e),o=Math.max(r,t),i=o===r?a(this.#e)+1:0;return this.#e=w(o,i),this.#e}receive(t){let r=this.#o(),o=g(this.#e),i=g(t),s=Math.max(o,i,r),n;return s===o&&s===i?n=Math.max(a(this.#e),a(t))+1:s===o?n=a(this.#e)+1:s===i?n=a(t)+1:n=0,this.#e=w(s,n),this.#e}};function E(e){let t=e.trim();if(!/^\d+$/.test(t))return null;try{return BigInt(t)}catch{return null}}function u(e){return e.toString()}var b="Tonk-Prose-Version",x="1";function C(e){return e.slice(0,b.length+1).toLowerCase()===`${b.toLowerCase()}:`}function h(e){if(!C(e))return{hlc:null,value:e};let t=e.indexOf(`\r
\r
`),r=e.indexOf(`

`),o,i;if(t!==-1&&(r===-1||t<=r))o=t,i=t+4;else if(r!==-1)o=r,i=r+2;else return{hlc:null,value:e};let s=e.slice(i),n=e.slice(0,o).split(/\r\n|\n/),v=null;for(let d of n){let f=d.indexOf(":");if(f===-1)continue;d.slice(0,f).trim().toLowerCase()==="etag"&&(v=E(d.slice(f+1).trim().replace(/^"|"$/g,"")))}return{hlc:v,value:s}}function m(e){return e.hlc===null?e.value:`${b}: ${x}\r
ETag: "${u(e.hlc)}"\r
Content-Type: text/markdown\r
\r
`+e.value}var M=["content","value","readonly","placeholder","auto-focus"],O=400;async function A(){let e=globalThis.__tonkProseEditor;if(typeof e=="string"&&e)return e;if(typeof e=="function"){let t=await e();if(typeof t=="string"&&t)return t}return new URL("./tonk-prose-editor.js",import.meta.url).href}var l=null;function S(){return l||(l=A().then(e=>import(e).then(t=>t)),l.catch(()=>{l=null})),l}var k=class extends HTMLElement{static get observedAttributes(){return M}#e;#o;#t=null;#c=0;#i=null;#s=null;#n=null;#u=null;#h=new c;#r=0n;#a=!1;constructor(){super(),this.#e=this.attachShadow({mode:"open",delegatesFocus:!0});let t=document.createElement("style");t.textContent=T,this.#o=document.createElement("div"),this.#o.className="mount",this.#e.append(t,this.#o)}connectedCallback(){if(this.#a=!1,this.#s||(this.#s=new MutationObserver(()=>this.#g()),this.#s.observe(this,{childList:!0,characterData:!0,subtree:!0})),this.#t)return;let t=++this.#c;this.#p(t)}#d(){return this.textContent??""}#g(){this.#l(this.#d())}async#p(t){let r;try{r=await S()}catch(n){console.warn("[tonk-prose] failed to load editor core:",n);return}if(t!==this.#c||!this.isConnected)return;let o=this.#i;if(o===null){let n=this.#d();o=n!==""?n:this.getAttribute("content")??this.getAttribute("value")}let i="";if(o!==null){let n=h(o);i=n.value,n.hlc!==null&&n.hlc>this.#r&&(this.#r=this.#h.receive(n.hlc))}let s=r.createEditor(this.#o,{doc:i,readOnly:this.hasAttribute("readonly"),placeholder:this.getAttribute("placeholder")??"",onChange:n=>{this.#b(n)}});this.#i=null,this.#t=s,this.dispatchEvent(new CustomEvent("ready",{detail:{editor:s},bubbles:!0,composed:!0})),!this.hasAttribute("readonly")&&this.hasAttribute("auto-focus")&&setTimeout(()=>{this.#t===s&&s.focus()},0)}#b(t){this.#u=t,this.#n!==null&&clearTimeout(this.#n),this.#n=setTimeout(()=>this.#f(),O)}#f(){this.#n=null;let t=this.#u;if(this.#u=null,t===null)return;let r=this.#h.tick();this.#r=r;let o=m({hlc:r,value:t});this.dispatchEvent(new CustomEvent("change",{detail:{value:t,content:o},bubbles:!0,composed:!0}))}disconnectedCallback(){this.#a||(this.#a=!0,setTimeout(()=>{this.#a&&(this.#a=!1,!this.isConnected&&(this.#n!==null&&(clearTimeout(this.#n),this.#f()),this.#s?.disconnect(),this.#s=null,this.#c++,this.#t?.destroy(),this.#t=null))},0))}attributeChangedCallback(t,r,o){switch(t){case"content":this.#l(o??"");break;case"value":(o??"")!==this.value&&this.#l(o??"");break;case"readonly":this.#t?.setReadOnly(o!==null);break;case"placeholder":this.#t?.setPlaceholder(o??"");break;case"auto-focus":break}}#l(t){let{hlc:r,value:o}=h(t);if(r!==null){if(r<=this.#r)return;this.#r=this.#h.receive(r)}if(!this.#t){this.#i=o;return}this.#t.setMarkdown(o)}get value(){if(this.#t)return this.#t.getMarkdown();if(this.#i!==null)return this.#i;let t=this.#d(),r=t!==""?t:this.getAttribute("content")??this.getAttribute("value");return r===null?"":h(r).value}set value(t){this.#l(t)}get content(){return m({hlc:this.#r,value:this.value})}set content(t){this.#l(t)}get version(){return u(this.#r)}focus(){this.#t?this.#t.focus():super.focus()}get editor(){return this.#t}},T=`
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
