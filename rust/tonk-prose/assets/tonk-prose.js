var g=16n,E=(1n<<g)-1n;function y(e,t){return BigInt(e)<<g|BigInt(t)&E}function p(e){return Number(e>>g)}function l(e){return Number(e&E)}var u=class{#e=0n;#n;constructor(t=()=>Date.now()){this.#n=t}get last(){return this.#e}tick(){let t=this.#n(),o=p(this.#e),n=Math.max(o,t),s=n===o?l(this.#e)+1:0;return this.#e=y(n,s),this.#e}receive(t){let o=this.#n(),n=p(this.#e),s=p(t),i=Math.max(n,s,o),r;return i===n&&i===s?r=Math.max(l(this.#e),l(t))+1:i===n?r=l(this.#e)+1:i===s?r=l(t)+1:r=0,this.#e=y(i,r),this.#e}};function C(e){let t=e.trim();if(!/^\d+$/.test(t))return null;try{return BigInt(t)}catch{return null}}function c(e){return e.toString()}var m="Tonk-Prose-Version",w="1";function x(e){return e.slice(0,m.length+1).toLowerCase()===`${m.toLowerCase()}:`}function h(e){if(!x(e))return{hlc:null,value:e};let t=e.indexOf(`\r
\r
`),o=e.indexOf(`

`),n,s;if(t!==-1&&(o===-1||t<=o))n=t,s=t+4;else if(o!==-1)n=o,s=o+2;else return{hlc:null,value:e};let i=e.slice(s),r=e.slice(0,n).split(/\r\n|\n/),v=null;for(let d of r){let f=d.indexOf(":");if(f===-1)continue;d.slice(0,f).trim().toLowerCase()==="etag"&&(v=C(d.slice(f+1).trim().replace(/^"|"$/g,"")))}return{hlc:v,value:i}}function b(e){return e.hlc===null?e.value:`${m}: ${w}\r
ETag: "${c(e.hlc)}"\r
Content-Type: text/markdown\r
\r
`+e.value}var M=["content","value","readonly","placeholder","auto-focus"],O=400;async function T(){let e=globalThis.__tonkProseEditor;if(typeof e=="string"&&e)return e;if(typeof e=="function"){let t=await e();if(typeof t=="string"&&t)return t}return new URL("./tonk-prose-editor.js",import.meta.url).href}var a=null;function A(){return a||(a=T().then(e=>import(e).then(t=>t)),a.catch(()=>{a=null})),a}var k=class extends HTMLElement{static get observedAttributes(){return M}#e;#n;#t=null;#u=0;#s=null;#i=null;#r=null;#c=null;#h=new u;#o=0n;#l=!1;constructor(){super(),this.#e=this.attachShadow({mode:"open",delegatesFocus:!0});let t=document.createElement("style");t.textContent=S,this.#n=document.createElement("div"),this.#n.className="mount",this.#e.append(t,this.#n)}connectedCallback(){if(this.#l=!1,this.#i||(this.#i=new MutationObserver(()=>this.#p()),this.#i.observe(this,{childList:!0,characterData:!0,subtree:!0})),this.#t)return;let t=++this.#u;this.#g(t)}#d(){return this.textContent??""}#p(){this.#a(this.#d())}async#g(t){let o;try{o=await A()}catch(r){console.warn("[tonk-prose] failed to load editor core:",r);return}if(t!==this.#u||!this.isConnected)return;let n=this.#s;if(n===null){let r=this.#d();n=r!==""?r:this.getAttribute("content")??this.getAttribute("value")}let s="";if(n!==null){let r=h(n);s=r.value,r.hlc!==null&&r.hlc>this.#o&&(this.#o=this.#h.receive(r.hlc))}let i=o.createEditor(this.#n,{doc:s,readOnly:this.hasAttribute("readonly"),placeholder:this.getAttribute("placeholder")??"",onChange:r=>{this.#m(r)}});this.#s=null,this.#t=i,this.dispatchEvent(new CustomEvent("ready",{detail:{editor:i},bubbles:!0,composed:!0})),!this.hasAttribute("readonly")&&this.hasAttribute("auto-focus")&&setTimeout(()=>{this.#t===i&&i.focus()},0)}#m(t){this.#c=t,this.#r!==null&&clearTimeout(this.#r),this.#r=setTimeout(()=>this.#f(),O)}#f(){this.#r=null;let t=this.#c;if(this.#c=null,t===null)return;let o=this.#h.tick();this.#o=o;let n=b({hlc:o,value:t});this.dispatchEvent(new CustomEvent("change",{detail:{value:t,content:n},bubbles:!0,composed:!0}))}disconnectedCallback(){this.#l||(this.#l=!0,setTimeout(()=>{this.#l&&(this.#l=!1,!this.isConnected&&(this.#r!==null&&(clearTimeout(this.#r),this.#f()),this.#i?.disconnect(),this.#i=null,this.#u++,this.#t?.destroy(),this.#t=null))},0))}attributeChangedCallback(t,o,n){switch(t){case"content":this.#a(n??"");break;case"value":(n??"")!==this.value&&this.#a(n??"");break;case"readonly":this.#t?.setReadOnly(n!==null);break;case"placeholder":this.#t?.setPlaceholder(n??"");break;case"auto-focus":break}}#a(t){let{hlc:o,value:n}=h(t);if(o!==null){if(o<=this.#o)return;this.#o=this.#h.receive(o)}if(!this.#t){this.#s=n;return}this.#t.setMarkdown(n)}get value(){if(this.#t)return this.#t.getMarkdown();if(this.#s!==null)return this.#s;let t=this.#d(),o=t!==""?t:this.getAttribute("content")??this.getAttribute("value");return o===null?"":h(o).value}set value(t){this.#a(t)}get content(){return b({hlc:this.#o,value:this.value})}set content(t){this.#a(t)}get version(){return c(this.#o)}focus(){this.#t?this.#t.focus():super.focus()}get editor(){return this.#t}},S=`
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
`;customElements.get("tonk-prose")||customElements.define("tonk-prose",k);
//# sourceMappingURL=tonk-prose.js.map
