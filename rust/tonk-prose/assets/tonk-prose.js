var a=["value","readonly","placeholder","auto-focus"];async function d(){let o=globalThis.__tonkProseEditor;if(typeof o=="string"&&o)return o;if(typeof o=="function"){let e=await o();if(typeof e=="string"&&e)return e}return new URL("./tonk-prose-editor.js",import.meta.url).href}var r=null;function l(){return r||(r=d().then(o=>import(o).then(e=>e)),r.catch(()=>{r=null})),r}var i=class extends HTMLElement{static get observedAttributes(){return a}#i;#r;#e=null;#s=0;#t=null;#n=!1;#o=!1;constructor(){super(),this.#i=this.attachShadow({mode:"open",delegatesFocus:!0});let e=document.createElement("style");e.textContent=u,this.#r=document.createElement("div"),this.#r.className="mount",this.#i.append(e,this.#r)}connectedCallback(){if(this.#o=!1,this.#e)return;let e=++this.#s;this.#a(e)}async#a(e){let s;try{s=await l()}catch(n){console.warn("[tonk-prose] failed to load editor core:",n);return}if(e!==this.#s||!this.isConnected)return;let t=s.createEditor(this.#r,{doc:this.#t??this.getAttribute("value")??"",readOnly:this.hasAttribute("readonly"),placeholder:this.getAttribute("placeholder")??"",onChange:n=>{this.#n||this.dispatchEvent(new CustomEvent("change",{detail:{value:n},bubbles:!0,composed:!0}))}});this.#t=null,this.#e=t,this.dispatchEvent(new CustomEvent("ready",{detail:{editor:t},bubbles:!0,composed:!0})),!this.hasAttribute("readonly")&&this.hasAttribute("auto-focus")&&setTimeout(()=>{this.#e===t&&t.focus()},0)}disconnectedCallback(){this.#o||(this.#o=!0,setTimeout(()=>{this.#o&&(this.#o=!1,!this.isConnected&&(this.#s++,this.#e?.destroy(),this.#e=null))},0))}attributeChangedCallback(e,s,t){switch(e){case"value":(t??"")!==this.value&&(this.value=t??"");break;case"readonly":this.#e?.setReadOnly(t!==null);break;case"placeholder":this.#e?.setPlaceholder(t??"");break;case"auto-focus":break}}get value(){return this.#e?this.#e.getMarkdown():this.#t!==null?this.#t:this.getAttribute("value")??""}set value(e){if(!this.#e){this.#t=e;return}if(e!==this.#e.getMarkdown()){this.#n=!0;try{this.#e.setMarkdown(e)}finally{this.#n=!1}}}focus(){this.#e?this.#e.focus():super.focus()}get editor(){return this.#e}},u=`
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
`;customElements.get("tonk-prose")||customElements.define("tonk-prose",i);
//# sourceMappingURL=tonk-prose.js.map
