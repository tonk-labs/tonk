var g=16n,y=(1n<<g)-1n;function v(e,t){return BigInt(e)<<g|BigInt(t)&y}function p(e){return Number(e>>g)}function l(e){return Number(e&y)}var u=class{#e=0n;#o;constructor(t=()=>Date.now()){this.#o=t}get last(){return this.#e}tick(){let t=this.#o(),n=p(this.#e),o=Math.max(n,t),r=o===n?l(this.#e)+1:0;return this.#e=v(o,r),this.#e}receive(t){let n=this.#o(),o=p(this.#e),r=p(t),s=Math.max(o,r,n),i;return s===o&&s===r?i=Math.max(l(this.#e),l(t))+1:s===o?i=l(this.#e)+1:s===r?i=l(t)+1:i=0,this.#e=v(s,i),this.#e}};function E(e){let t=e.trim();if(!/^\d+$/.test(t))return null;try{return BigInt(t)}catch{return null}}function c(e){return e.toString()}var C="Tonk-Prose-Version",w="1";function x(e){let t=`${C}:`;return e.startsWith(t)}function h(e){if(!x(e))return{hlc:null,value:e};let t=e.indexOf(`\r
\r
`),n=e.indexOf(`

`),o,r;if(t!==-1&&(n===-1||t<=n))o=t,r=t+4;else if(n!==-1)o=n,r=n+2;else return{hlc:null,value:e};let s=e.slice(r),i=e.slice(0,o).split(/\r\n|\n/),k=null;for(let d of i){let f=d.indexOf(":");if(f===-1)continue;d.slice(0,f).trim().toLowerCase()==="etag"&&(k=E(d.slice(f+1).trim().replace(/^"|"$/g,"")))}return{hlc:k,value:s}}function m(e){return e.hlc===null?e.value:`${C}: ${w}\r
ETag: "${c(e.hlc)}"\r
Content-Type: text/markdown\r
\r
`+e.value}var M=["content","value","readonly","placeholder","auto-focus"],T=400;async function A(){let e=globalThis.__tonkProseEditor;if(typeof e=="string"&&e)return e;if(typeof e=="function"){let t=await e();if(typeof t=="string"&&t)return t}return new URL("./tonk-prose-editor.js",import.meta.url).href}var a=null;function S(){return a||(a=A().then(e=>import(e).then(t=>t)),a.catch(()=>{a=null})),a}var b=class extends HTMLElement{static get observedAttributes(){return M}#e;#o;#t=null;#a=0;#r=null;#s=null;#u=null;#c=new u;#n=0n;#i=!1;constructor(){super(),this.#e=this.attachShadow({mode:"open",delegatesFocus:!0});let t=document.createElement("style");t.textContent=O,this.#o=document.createElement("div"),this.#o.className="mount",this.#e.append(t,this.#o)}connectedCallback(){if(this.#i=!1,this.#t)return;let t=++this.#a;this.#d(t)}async#d(t){let n;try{n=await S()}catch(s){console.warn("[tonk-prose] failed to load editor core:",s);return}if(t!==this.#a||!this.isConnected)return;let o;if(this.#r!==null)o=this.#r;else{let s=this.getAttribute("content")??this.getAttribute("value");if(s===null)o="";else{let i=h(s);o=i.value,i.hlc!==null&&i.hlc>this.#n&&(this.#n=this.#c.receive(i.hlc))}}let r=n.createEditor(this.#o,{doc:o,readOnly:this.hasAttribute("readonly"),placeholder:this.getAttribute("placeholder")??"",onChange:s=>{this.#f(s)}});this.#r=null,this.#t=r,this.dispatchEvent(new CustomEvent("ready",{detail:{editor:r},bubbles:!0,composed:!0})),!this.hasAttribute("readonly")&&this.hasAttribute("auto-focus")&&setTimeout(()=>{this.#t===r&&r.focus()},0)}#f(t){this.#u=t,this.#s!==null&&clearTimeout(this.#s),this.#s=setTimeout(()=>this.#h(),T)}#h(){this.#s=null;let t=this.#u;if(this.#u=null,t===null)return;let n=this.#c.tick();this.#n=n;let o=m({hlc:n,value:t});this.dispatchEvent(new CustomEvent("change",{detail:{value:t,content:o},bubbles:!0,composed:!0}))}disconnectedCallback(){this.#i||(this.#i=!0,setTimeout(()=>{this.#i&&(this.#i=!1,!this.isConnected&&(this.#s!==null&&(clearTimeout(this.#s),this.#h()),this.#a++,this.#t?.destroy(),this.#t=null))},0))}attributeChangedCallback(t,n,o){switch(t){case"content":this.#l(o??"");break;case"value":(o??"")!==this.value&&this.#l(o??"");break;case"readonly":this.#t?.setReadOnly(o!==null);break;case"placeholder":this.#t?.setPlaceholder(o??"");break;case"auto-focus":break}}#l(t){let{hlc:n,value:o}=h(t);if(n!==null){if(n<=this.#n)return;this.#n=this.#c.receive(n)}if(!this.#t){this.#r=o;return}this.#t.setMarkdown(o)}get value(){if(this.#t)return this.#t.getMarkdown();if(this.#r!==null)return this.#r;let t=this.getAttribute("content")??this.getAttribute("value");return t===null?"":h(t).value}set value(t){this.#l(t)}get content(){return m({hlc:this.#n,value:this.value})}set content(t){this.#l(t)}get version(){return c(this.#n)}focus(){this.#t?this.#t.focus():super.focus()}get editor(){return this.#t}},O=`
  :host {
    --tonk-prose-font: ui-sans-serif, -apple-system, "Segoe UI", Helvetica,
                       Arial, sans-serif;
    --tonk-prose-mono: ui-monospace, SFMono-Regular, Menlo, Consolas,
                       "Liberation Mono", monospace;
    --tonk-prose-font-size: 1rem;
    --tonk-prose-radius: 6px;
    --tonk-prose-padding: 1rem 1.25rem;
    --tonk-prose-max-width: none;

    /* Surfaces & text \u2014 GitHub light defaults */
    --tonk-prose-bg: #ffffff;
    --tonk-prose-fg: #1f2328;
    --tonk-prose-fg-muted: #59636e;
    --tonk-prose-border: #d1d9e0;
    --tonk-prose-accent: #0969da;
    --tonk-prose-selection: #0969da33;
    --tonk-prose-focus-ring: #0969da66;
    /* Revealed markdown syntax markers (the Typora trick). */
    --tonk-prose-marker: #9198a1;
    /* Inline code + code block surfaces. */
    --tonk-prose-code-bg: #f6f8fa;
    --tonk-prose-code-fg: #1f2328;
    --tonk-prose-blockquote: #59636e;

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

  @media (prefers-color-scheme: dark) {
    :host {
      --tonk-prose-bg: #0d1117;
      --tonk-prose-fg: #f0f6fc;
      --tonk-prose-fg-muted: #9198a1;
      --tonk-prose-border: #3d444d;
      --tonk-prose-accent: #1f6feb;
      --tonk-prose-selection: #1f6feb59;
      --tonk-prose-focus-ring: #1f6feb99;
      --tonk-prose-marker: #6e7681;
      --tonk-prose-code-bg: #151b23;
      --tonk-prose-code-fg: #f0f6fc;
      --tonk-prose-blockquote: #9198a1;
    }
  }

  :host([hidden]) { display: none; }

  :host(:focus-within) {
    border-color: var(--tonk-prose-accent);
    box-shadow: 0 0 0 2px var(--tonk-prose-focus-ring);
  }

  .mount { height: 100%; }
`;customElements.get("tonk-prose")||customElements.define("tonk-prose",b);
//# sourceMappingURL=tonk-prose.js.map
