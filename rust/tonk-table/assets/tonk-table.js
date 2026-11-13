function S(n){if(!/^[A-Z]+$/.test(n))return null;let t=0;for(let e of n)t=t*26+(e.charCodeAt(0)-64);return t}function D(n){let t=/^([A-Z]+)([1-9][0-9]*)$/.exec(n);if(!t)return null;let e=S(t[1]);return e===null?null:{row:Number(t[2]),column:e}}function q(n,t){return n.order<t.order?-1:n.order>t.order?1:n.id<t.id?-1:1}function R(n,t){let e=[];for(let r of Array.from(n.querySelectorAll(".table-sheet-row"))){let o=r,s=o.getAttribute("subject")??"",i=o.dataset.table??"";!s||i!==t||e.push({id:s,table:i,name:o.dataset.name??"",order:o.dataset.order??""})}return e.sort(q)}function E(n,t){let e=[];for(let r of Array.from(n.querySelectorAll(".table-cell-row"))){let o=r,s=o.getAttribute("subject")??"",i=o.dataset.sheet??"",l=o.dataset.at??"";if(!(!s||!t.has(i))){if(D(l)===null){console.warn(`[tonk-table] ignoring cell claim with bad address: ${l}`);continue}e.push({id:s,sheet:i,at:l,content:o.dataset.content??"",style:o.dataset.style??""})}}return e}function M(n,t){let e=[];for(let r of Array.from(n.querySelectorAll(".table-column-row"))){let o=r,s=o.getAttribute("subject")??"",i=o.dataset.sheet??"",l=o.dataset.at??"",a=o.dataset.width??"";if(!(!s||!t.has(i))){if(S(l)===null||!(Number(a)>0)){console.warn(`[tonk-table] ignoring column claim ${l}=${a}`);continue}e.push({id:s,sheet:i,at:l,width:a})}}return e}function O(n,t){let e=[];for(let r of Array.from(n.querySelectorAll(".table-rowsize-row"))){let o=r,s=o.getAttribute("subject")??"",i=o.dataset.sheet??"",l=o.dataset.at??"",a=o.dataset.height??"";if(!(!s||!t.has(i))){if(!/^[1-9][0-9]*$/.test(l)||!(Number(a)>0)){console.warn(`[tonk-table] ignoring row claim ${l}=${a}`);continue}e.push({id:s,sheet:i,at:l,height:a})}}return e}var p=16n,H=(1n<<p)-1n;function N(n,t){return BigInt(n)<<p|BigInt(t)&H}function m(n){return Number(n>>p)}function c(n){return Number(n&H)}var h=class{#e=0n;#n;constructor(t=()=>Date.now()){this.#n=t}get last(){return this.#e}tick(){let t=this.#n(),e=m(this.#e),r=Math.max(e,t),o=r===e?c(this.#e)+1:0;return this.#e=N(r,o),this.#e}receive(t){let e=this.#n(),r=m(this.#e),o=m(t),s=Math.max(r,o,e),i;return s===r&&s===o?i=Math.max(c(this.#e),c(t))+1:s===r?i=c(this.#e)+1:s===o?i=c(t)+1:i=0,this.#e=N(s,i),this.#e}};function L(n){let t=n.trim();if(!/^\d+$/.test(t))return null;try{return BigInt(t)}catch{return null}}function f(n){return n.toString()}var y="application/vnd.ironcalc";function $(n){return n!==null&&n.toLowerCase().includes("vnd.ironcalc")}var w="Tonk-Table-Version",z="1";function B(n){return n.slice(0,w.length+1).toLowerCase()===`${w.toLowerCase()}:`}function u(n){if(!B(n))return{hlc:null,contentType:null,value:n};let t=n.indexOf(`\r
\r
`),e=n.indexOf(`

`),r,o;if(t!==-1&&(e===-1||t<=e))r=t,o=t+4;else if(e!==-1)r=e,o=e+2;else return{hlc:null,contentType:null,value:n};let s=n.slice(o),i=n.slice(0,r).split(/\r\n|\n/),l=null,a=null;for(let b of i){let g=b.indexOf(":");if(g===-1)continue;let T=b.slice(0,g).trim().toLowerCase(),A=b.slice(g+1).trim();T==="etag"?l=L(A.replace(/^"|"$/g,"")):T==="content-type"&&(a=A)}return{hlc:l,contentType:a,value:s}}function v(n){if(n.hlc===null)return n.value;let t=n.contentType===null?"":`Content-Type: ${n.contentType}\r
`;return`${w}: ${z}\r
ETag: "${f(n.hlc)}"\r
`+t+`\r
`+n.value}function k(n){let e="";for(let r=0;r<n.length;r+=32768)e+=String.fromCharCode(...n.subarray(r,r+32768));return btoa(e)}function G(n){let t=n.replace(/\s+/g,"");if(!/^[A-Za-z0-9+/]*={0,2}$/.test(t)||t.length%4===1)return null;try{let e=atob(t),r=new Uint8Array(e.length);for(let o=0;o<e.length;o++)r[o]=e.charCodeAt(o);return r}catch{return null}}var j=["subject","content","value","readonly","auto-focus","min-rows","min-cols"],I=["subject","data-table","data-name","data-order","data-sheet","data-at","data-content","data-style","data-width","data-height"],P=30,U=400,d=null;function _(){return d||(d=import(new URL("./tonk-table-grid.js",import.meta.url).href).then(t=>t),d.catch(()=>{d=null})),d}function C(n){if($(n.contentType)){let t=G(n.value);return t&&t.length>0?{kind:"workbook",bytes:t}:(console.warn("[tonk-table] workbook body was not valid base64; starting empty"),{kind:"csv",csv:""})}return{kind:"csv",csv:n.value}}var x=class extends HTMLElement{static get observedAttributes(){return j}#e;#n;#t=null;#a=0;#r=null;#c=null;#i=null;#s=null;#b=!1;#g=new h;#o=0n;#u=!1;#h;constructor(){super(),this.#e=this.attachShadow({mode:"open",delegatesFocus:!0});let t=document.createElement("style");t.textContent=W,this.#n=document.createElement("div"),this.#n.className="mount",this.#h=document.createElement("style"),this.#e.append(t,this.#n,this.#h)}#p(){let t="";for(let e of Array.from(this.children))e instanceof HTMLStyleElement&&(t+=`${e.textContent??""}
`);this.#h.textContent!==t&&(this.#h.textContent=t)}connectedCallback(){if(this.#u=!1,this.#c||(this.#c=new MutationObserver(()=>{this.#p(),this.#m()?this.#C():this.#x()}),this.#c.observe(this,{childList:!0,characterData:!0,subtree:!0,attributes:!0,attributeFilter:I})),this.#p(),this.#t)return;let t=++this.#a;this.#v(t)}#m(){return this.hasAttribute("subject")}#C(){this.#i===null&&(this.#i=setTimeout(()=>{this.#i=null,this.#w()},P))}#w(){let t=this.#t,e=this.getAttribute("subject");if(!t||e===null)return;let r=R(this,e),o=new Set(r.map(s=>s.id));t.applyRows(r,E(this,o),M(this,o),O(this,o))}#f(t){let e=this.getAttribute(t);if(e===null)return;let r=Number(e);return Number.isInteger(r)&&r>=1?r:void 0}#l(){let t="";for(let e of Array.from(this.childNodes))e.nodeType===Node.TEXT_NODE&&(t+=e.nodeValue??"");return t}#y=null;#x(){if(this.#m())return;let t=this.#l();t!==this.#y&&(this.#y=t,this.#d(t))}async#v(t){let e;try{e=await _()}catch(i){console.warn("[tonk-table] failed to load grid core:",i);return}if(t!==this.#a||!this.isConnected)return;let r,o=this.getAttribute("subject");if(o!==null)r={kind:"claims",subject:o};else{let i=this.#r;if(i===null){let a=this.#l();i=a!==""?a:this.getAttribute("content")??this.getAttribute("value")}this.#r=null;let l=u(i??"");l.hlc!==null&&l.hlc>this.#o&&(this.#o=this.#g.receive(l.hlc)),r={kind:"standalone",source:C(l)}}let s;try{s=await e.createGrid(this.#n,{mode:r,emit:(i,l)=>{this.dispatchEvent(new CustomEvent(i,{bubbles:!0,composed:!0,detail:l}))},readOnly:this.hasAttribute("readonly"),minRows:this.#f("min-rows"),minCols:this.#f("min-cols"),onChange:r.kind==="standalone"?()=>this.#T():void 0})}catch(i){console.warn("[tonk-table] failed to mount grid:",i);return}if(t!==this.#a||!this.isConnected){s.destroy();return}if(this.#t=s,r.kind==="claims")this.#w();else if(this.#r!==null){let i=u(this.#r);this.#r=null,s.load(C(i))}this.dispatchEvent(new CustomEvent("ready",{detail:{grid:s},bubbles:!0,composed:!0})),!this.hasAttribute("readonly")&&this.hasAttribute("auto-focus")&&setTimeout(()=>{this.#t===s&&s.focus()},0)}#T(){this.#b=!0,this.#s!==null&&clearTimeout(this.#s),this.#s=setTimeout(()=>this.#k(),U)}#k(){if(this.#s=null,!this.#b)return;this.#b=!1;let t=this.#t;if(!t)return;let e=this.#g.tick();this.#o=e;let r=t.toCsv(),o=v({hlc:e,contentType:y,value:k(t.serialize())});this.dispatchEvent(new CustomEvent("change",{detail:{value:r,content:o},bubbles:!0,composed:!0}))}disconnectedCallback(){this.#u||(this.#u=!0,setTimeout(()=>{this.#u&&(this.#u=!1,!this.isConnected&&(this.#s!==null&&(clearTimeout(this.#s),this.#k()),this.#i!==null&&(clearTimeout(this.#i),this.#i=null),this.#c?.disconnect(),this.#c=null,this.#a++,this.#t?.destroy(),this.#t=null))},0))}attributeChangedCallback(t,e,r){switch(t){case"subject":if(e!==r&&this.#t){this.#t.destroy(),this.#t=null;let o=++this.#a;this.isConnected&&this.#v(o)}break;case"content":this.#d(r??"");break;case"value":(r??"")!==this.value&&this.#d(r??"");break;case"readonly":this.#t?.setReadOnly(r!==null);break;case"min-rows":case"min-cols":this.#t?.setMinExtent(this.#f("min-rows"),this.#f("min-cols"));break;case"auto-focus":break}}#d(t){if(this.#m())return;let e=u(t);if(e.hlc!==null){if(e.hlc<=this.#o)return;this.#o=this.#g.receive(e.hlc)}if(!this.#t){this.#r=t;return}this.#t.load(C(e))}get value(){if(this.#t)return this.#t.toCsv();let t=this.#r??(this.#l()!==""?this.#l():this.getAttribute("content")??this.getAttribute("value"));return t===null?"":u(t).value}set value(t){this.#d(t)}get content(){return this.#t?v({hlc:this.#o,contentType:y,value:k(this.#t.serialize())}):this.#r??(this.#l()!==""?this.#l():this.getAttribute("content")??this.getAttribute("value"))??""}set content(t){this.#d(t)}get version(){return f(this.#o)}focus(){this.#t?this.#t.focus():super.focus()}get grid(){return this.#t}},W=`
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
