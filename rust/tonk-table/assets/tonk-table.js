function S(e){if(!/^[A-Z]+$/.test(e))return null;let t=0;for(let n of e)t=t*26+(n.charCodeAt(0)-64);return t}function D(e){let t=/^([A-Z]+)([1-9][0-9]*)$/.exec(e);if(!t)return null;let n=S(t[1]);return n===null?null:{row:Number(t[2]),column:n}}function q(e,t){return e.order<t.order?-1:e.order>t.order?1:e.id<t.id?-1:1}function R(e,t){let n=[];for(let r of Array.from(e.querySelectorAll(".table-sheet-row"))){let o=r,s=o.getAttribute("subject")??"",i=o.dataset.table??"";!s||i!==t||n.push({id:s,table:i,name:o.dataset.name??"",order:o.dataset.order??""})}return n.sort(q)}function E(e,t){let n=[];for(let r of Array.from(e.querySelectorAll(".table-cell-row"))){let o=r,s=o.getAttribute("subject")??"",i=o.dataset.sheet??"",l=o.dataset.at??"";if(!(!s||!t.has(i))){if(D(l)===null){console.warn(`[tonk-table] ignoring cell claim with bad address: ${l}`);continue}n.push({id:s,sheet:i,at:l,content:o.dataset.content??"",style:o.dataset.style??""})}}return n}function M(e,t){let n=[];for(let r of Array.from(e.querySelectorAll(".table-column-row"))){let o=r,s=o.getAttribute("subject")??"",i=o.dataset.sheet??"",l=o.dataset.at??"",a=o.dataset.width??"";if(!(!s||!t.has(i))){if(S(l)===null||!(Number(a)>0)){console.warn(`[tonk-table] ignoring column claim ${l}=${a}`);continue}n.push({id:s,sheet:i,at:l,width:a})}}return n}function O(e,t){let n=[];for(let r of Array.from(e.querySelectorAll(".table-rowsize-row"))){let o=r,s=o.getAttribute("subject")??"",i=o.dataset.sheet??"",l=o.dataset.at??"",a=o.dataset.height??"";if(!(!s||!t.has(i))){if(!/^[1-9][0-9]*$/.test(l)||!(Number(a)>0)){console.warn(`[tonk-table] ignoring row claim ${l}=${a}`);continue}n.push({id:s,sheet:i,at:l,height:a})}}return n}var p=16n,H=(1n<<p)-1n;function N(e,t){return BigInt(e)<<p|BigInt(t)&H}function m(e){return Number(e>>p)}function u(e){return Number(e&H)}var h=class{#e=0n;#n;constructor(t=()=>Date.now()){this.#n=t}get last(){return this.#e}tick(){let t=this.#n(),n=m(this.#e),r=Math.max(n,t),o=r===n?u(this.#e)+1:0;return this.#e=N(r,o),this.#e}receive(t){let n=this.#n(),r=m(this.#e),o=m(t),s=Math.max(r,o,n),i;return s===r&&s===o?i=Math.max(u(this.#e),u(t))+1:s===r?i=u(this.#e)+1:s===o?i=u(t)+1:i=0,this.#e=N(s,i),this.#e}};function L(e){let t=e.trim();if(!/^\d+$/.test(t))return null;try{return BigInt(t)}catch{return null}}function f(e){return e.toString()}var y="application/vnd.ironcalc";function G(e){return e!==null&&e.toLowerCase().includes("vnd.ironcalc")}var w="Tonk-Table-Version",z="1";function P(e){return e.slice(0,w.length+1).toLowerCase()===`${w.toLowerCase()}:`}function c(e){if(!P(e))return{hlc:null,contentType:null,value:e};let t=e.indexOf(`\r
\r
`),n=e.indexOf(`

`),r,o;if(t!==-1&&(n===-1||t<=n))r=t,o=t+4;else if(n!==-1)r=n,o=n+2;else return{hlc:null,contentType:null,value:e};let s=e.slice(o),i=e.slice(0,r).split(/\r\n|\n/),l=null,a=null;for(let b of i){let g=b.indexOf(":");if(g===-1)continue;let T=b.slice(0,g).trim().toLowerCase(),A=b.slice(g+1).trim();T==="etag"?l=L(A.replace(/^"|"$/g,"")):T==="content-type"&&(a=A)}return{hlc:l,contentType:a,value:s}}function v(e){if(e.hlc===null)return e.value;let t=e.contentType===null?"":`Content-Type: ${e.contentType}\r
`;return`${w}: ${z}\r
ETag: "${f(e.hlc)}"\r
`+t+`\r
`+e.value}function k(e){let n="";for(let r=0;r<e.length;r+=32768)n+=String.fromCharCode(...e.subarray(r,r+32768));return btoa(n)}function $(e){let t=e.replace(/\s+/g,"");if(!/^[A-Za-z0-9+/]*={0,2}$/.test(t)||t.length%4===1)return null;try{let n=atob(t),r=new Uint8Array(n.length);for(let o=0;o<n.length;o++)r[o]=n.charCodeAt(o);return r}catch{return null}}var _=["subject","content","value","readonly","auto-focus","min-rows","min-cols"],B=["subject","data-table","data-name","data-order","data-sheet","data-at","data-content","data-style","data-width","data-height"],j=30,I=400;async function U(){let e=globalThis.__tonkTableGrid;if(typeof e=="string"&&e)return e;if(typeof e=="function"){let t=await e();if(typeof t=="string"&&t)return t}return new URL("./tonk-table-grid.js",import.meta.url).href}var d=null;function W(){return d||(d=U().then(e=>import(e).then(t=>t)),d.catch(()=>{d=null})),d}function C(e){if(G(e.contentType)){let t=$(e.value);return t&&t.length>0?{kind:"workbook",bytes:t}:(console.warn("[tonk-table] workbook body was not valid base64; starting empty"),{kind:"csv",csv:""})}return{kind:"csv",csv:e.value}}var x=class extends HTMLElement{static get observedAttributes(){return _}#e;#n;#t=null;#a=0;#r=null;#u=null;#i=null;#s=null;#b=!1;#g=new h;#o=0n;#c=!1;#h;constructor(){super(),this.#e=this.attachShadow({mode:"open",delegatesFocus:!0});let t=document.createElement("style");t.textContent=K,this.#n=document.createElement("div"),this.#n.className="mount",this.#h=document.createElement("style"),this.#e.append(t,this.#n,this.#h)}#p(){let t="";for(let n of Array.from(this.children))n instanceof HTMLStyleElement&&(t+=`${n.textContent??""}
`);this.#h.textContent!==t&&(this.#h.textContent=t)}connectedCallback(){if(this.#c=!1,this.#u||(this.#u=new MutationObserver(()=>{this.#p(),this.#m()?this.#C():this.#x()}),this.#u.observe(this,{childList:!0,characterData:!0,subtree:!0,attributes:!0,attributeFilter:B})),this.#p(),this.#t)return;let t=++this.#a;this.#v(t)}#m(){return this.hasAttribute("subject")}#C(){this.#i===null&&(this.#i=setTimeout(()=>{this.#i=null,this.#w()},j))}#w(){let t=this.#t,n=this.getAttribute("subject");if(!t||n===null)return;let r=R(this,n),o=new Set(r.map(s=>s.id));t.applyRows(r,E(this,o),M(this,o),O(this,o))}#f(t){let n=this.getAttribute(t);if(n===null)return;let r=Number(n);return Number.isInteger(r)&&r>=1?r:void 0}#l(){let t="";for(let n of Array.from(this.childNodes))n.nodeType===Node.TEXT_NODE&&(t+=n.nodeValue??"");return t}#y=null;#x(){if(this.#m())return;let t=this.#l();t!==this.#y&&(this.#y=t,this.#d(t))}async#v(t){let n;try{n=await W()}catch(i){console.warn("[tonk-table] failed to load grid core:",i);return}if(t!==this.#a||!this.isConnected)return;let r,o=this.getAttribute("subject");if(o!==null)r={kind:"claims",subject:o};else{let i=this.#r;if(i===null){let a=this.#l();i=a!==""?a:this.getAttribute("content")??this.getAttribute("value")}this.#r=null;let l=c(i??"");l.hlc!==null&&l.hlc>this.#o&&(this.#o=this.#g.receive(l.hlc)),r={kind:"standalone",source:C(l)}}let s;try{s=await n.createGrid(this.#n,{mode:r,emit:(i,l)=>{this.dispatchEvent(new CustomEvent(i,{bubbles:!0,composed:!0,detail:l}))},readOnly:this.hasAttribute("readonly"),minRows:this.#f("min-rows"),minCols:this.#f("min-cols"),onChange:r.kind==="standalone"?()=>this.#T():void 0})}catch(i){console.warn("[tonk-table] failed to mount grid:",i);return}if(t!==this.#a||!this.isConnected){s.destroy();return}if(this.#t=s,r.kind==="claims")this.#w();else if(this.#r!==null){let i=c(this.#r);this.#r=null,s.load(C(i))}this.dispatchEvent(new CustomEvent("ready",{detail:{grid:s},bubbles:!0,composed:!0})),!this.hasAttribute("readonly")&&this.hasAttribute("auto-focus")&&setTimeout(()=>{this.#t===s&&s.focus()},0)}#T(){this.#b=!0,this.#s!==null&&clearTimeout(this.#s),this.#s=setTimeout(()=>this.#k(),I)}#k(){if(this.#s=null,!this.#b)return;this.#b=!1;let t=this.#t;if(!t)return;let n=this.#g.tick();this.#o=n;let r=t.toCsv(),o=v({hlc:n,contentType:y,value:k(t.serialize())});this.dispatchEvent(new CustomEvent("change",{detail:{value:r,content:o},bubbles:!0,composed:!0}))}disconnectedCallback(){this.#c||(this.#c=!0,setTimeout(()=>{this.#c&&(this.#c=!1,!this.isConnected&&(this.#s!==null&&(clearTimeout(this.#s),this.#k()),this.#i!==null&&(clearTimeout(this.#i),this.#i=null),this.#u?.disconnect(),this.#u=null,this.#a++,this.#t?.destroy(),this.#t=null))},0))}attributeChangedCallback(t,n,r){switch(t){case"subject":if(n!==r&&this.#t){this.#t.destroy(),this.#t=null;let o=++this.#a;this.isConnected&&this.#v(o)}break;case"content":this.#d(r??"");break;case"value":(r??"")!==this.value&&this.#d(r??"");break;case"readonly":this.#t?.setReadOnly(r!==null);break;case"min-rows":case"min-cols":this.#t?.setMinExtent(this.#f("min-rows"),this.#f("min-cols"));break;case"auto-focus":break}}#d(t){if(this.#m())return;let n=c(t);if(n.hlc!==null){if(n.hlc<=this.#o)return;this.#o=this.#g.receive(n.hlc)}if(!this.#t){this.#r=t;return}this.#t.load(C(n))}get value(){if(this.#t)return this.#t.toCsv();let t=this.#r??(this.#l()!==""?this.#l():this.getAttribute("content")??this.getAttribute("value"));return t===null?"":c(t).value}set value(t){this.#d(t)}get content(){return this.#t?v({hlc:this.#o,contentType:y,value:k(this.#t.serialize())}):this.#r??(this.#l()!==""?this.#l():this.getAttribute("content")??this.getAttribute("value"))??""}set content(t){this.#d(t)}get version(){return f(this.#o)}focus(){this.#t?this.#t.focus():super.focus()}get grid(){return this.#t}},K=`
  :host {
    --tonk-table-font: var(--wa-font-family-body, ui-sans-serif, -apple-system,
                       "Segoe UI", Helvetica, Arial, sans-serif);
    --tonk-table-mono: var(--wa-font-family-code, ui-monospace, SFMono-Regular,
                       Menlo, Consolas, "Liberation Mono", monospace);
    --tonk-table-font-size: var(--wa-font-size-s, 0.875rem);
    --tonk-table-radius: var(--wa-border-radius-m, 6px);

    /* Surfaces & text \u2014 inherit the page's WebAwesome tokens, GitHub
       light values as the standalone fallback. */
    --tonk-table-bg: var(--wa-color-surface-default, #ffffff);
    --tonk-table-fg: var(--wa-color-text-normal, #1f2328);
    --tonk-table-fg-muted: var(--wa-color-text-quiet, #59636e);
    --tonk-table-border: var(--wa-color-neutral-border-quiet, #d1d9e0);
    --tonk-table-grid-line: var(--wa-color-neutral-border-quiet, #e5e9ed);
    --tonk-table-header-bg: var(--wa-color-neutral-fill-quiet, #f6f8fa);
    --tonk-table-header-fg: var(--wa-color-text-quiet, #59636e);
    /* Active cell + focus \u2192 the brand accent; range fill \u2192 its quiet
       counterpart. */
    --tonk-table-accent: var(--wa-color-brand-fill-loud, #0969da);
    --tonk-table-selection: var(--wa-color-brand-fill-quiet, #0969da1a);
    --tonk-table-focus-ring: var(--wa-color-brand-border-normal, #0969da66);
    --tonk-table-error: var(--wa-color-danger-fill-loud, #d1242f);

    display: flex;
    flex-direction: column;
    block-size: var(--tonk-table-height, 26rem);
    position: relative;
    box-sizing: border-box;
    background: var(--tonk-table-bg);
    color: var(--tonk-table-fg);
    border: 1px solid var(--tonk-table-border);
    border-radius: var(--tonk-table-radius);
    overflow: hidden;
    transition: border-color 120ms ease, box-shadow 120ms ease;
  }

  /* Standalone dark fallback (no WebAwesome tokens present). When the
     page provides \`--wa-*\` the rules above already track its
     light/dark palette, so this only bites a bare page in dark mode. */
  @media (prefers-color-scheme: dark) {
    :host {
      --tonk-table-bg: var(--wa-color-surface-default, #0d1117);
      --tonk-table-fg: var(--wa-color-text-normal, #f0f6fc);
      --tonk-table-fg-muted: var(--wa-color-text-quiet, #9198a1);
      --tonk-table-border: var(--wa-color-neutral-border-quiet, #3d444d);
      --tonk-table-grid-line: var(--wa-color-neutral-border-quiet, #2a3038);
      --tonk-table-header-bg: var(--wa-color-neutral-fill-quiet, #151b23);
      --tonk-table-header-fg: var(--wa-color-text-quiet, #9198a1);
      --tonk-table-accent: var(--wa-color-brand-fill-loud, #1f6feb);
      --tonk-table-selection: var(--wa-color-brand-fill-quiet, #1f6feb33);
      --tonk-table-focus-ring: var(--wa-color-brand-border-normal, #1f6feb99);
      --tonk-table-error: var(--wa-color-danger-fill-loud, #f85149);
    }
  }

  :host([hidden]) { display: none; }

  :host(:focus-within) {
    border-color: var(--tonk-table-accent);
    box-shadow: 0 0 0 2px var(--tonk-table-focus-ring);
  }

  .mount {
    flex: 1;
    min-block-size: 0;
    display: flex;
    flex-direction: column;
  }
  .mount > .table-root { flex: 1; }
`;customElements.get("tonk-table")||customElements.define("tonk-table",x);
//# sourceMappingURL=tonk-table.js.map
