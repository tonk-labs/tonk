var h={tag:"#c792ea",entity:"#ff8da1",attribute:"#ffc78e",valueType:"#a1c8ff",valueRef:"#4ecbc4"};function f(s,e=6,t=4){let n=s.startsWith("#")?s.slice(1):s;return n.length>e+t+1?`${n.slice(0,e)}\u2026${n.slice(-t)}`:n}var v=`
<style>
  :host { display: inline-block; font-family: ui-monospace, monospace; }
  .bar { display: flex; align-items: stretch; gap: 0; }
  .cell {
    display: flex; flex-direction: column; padding: 0 6px 2px;
    border-bottom: 3px solid currentColor;
  }
  .val { font-weight: 600; font-size: 13px; line-height: 1.4; color: #e8e8e8; }
  .label { font-size: 9px; line-height: 1.2; opacity: 0.85; letter-spacing: 0.02em; }
  .sep { align-self: center; color: #666; padding: 0 1px; }
</style>
<div class="bar" part="bar"></div>
`,u=class extends HTMLElement{#t;#e=null;constructor(){super(),this.#t=this.attachShadow({mode:"open"}),this.#t.innerHTML=v}get key(){return this.#e}set key(e){this.#e=e,this.#n()}#n(){let e=this.#t.querySelector(".bar");e.textContent="";let t=this.#e;if(!t)return;[[h.tag,t.tag,"Tag"],[h.entity,f(t.entity),"Entity"],[h.attribute,f(t.attribute),"Attribute"],[h.valueType,String(t.valueType),"Type"],[h.valueRef,f(t.valueRef),"Value"]].forEach(([a,r,d],i)=>{if(i>0){let p=document.createElement("span");p.className="sep",p.textContent=".",e.appendChild(p)}let c=document.createElement("div");c.className="cell",c.style.color=a;let o=document.createElement("span");o.className="val",o.textContent=r;let l=document.createElement("span");l.className="label",l.textContent=d,c.append(o,l),e.appendChild(c)})}};function g(s="dialog-tree-key"){customElements.get(s)||customElements.define(s,u)}function C(s){return s<1024?`${s} B`:s<1024*1024?`${(s/1024).toFixed(s<10*1024?1:0)} KB`:`${(s/(1024*1024)).toFixed(1)} MB`}function E(s,e=8,t=4){let n=s.startsWith("#")?s.slice(1):s;return n.length>e+t+1?`${n.slice(0,e)}\u2026${n.slice(-t)}`:n}var T=`
<style>
  :host {
    display: block; font-family: ui-monospace, monospace; font-size: 13px;
    color: #ddd; --row-pad: 16px; --max-bar: 120px;
  }
  .tree { display: flex; flex-direction: column; }
  .row {
    display: flex; align-items: center; gap: 8px;
    padding: 3px 6px; border-radius: 4px; cursor: default; white-space: nowrap;
  }
  .row:hover { background: rgba(255,255,255,0.05); }
  .twist {
    width: 14px; text-align: center; cursor: pointer; color: #888;
    user-select: none; flex: none;
  }
  .twist.leaf-marker { cursor: default; color: #555; }
  .hash { color: #9ad; }
  .kind { font-size: 11px; padding: 1px 5px; border-radius: 3px; }
  .kind.branch { background: #2a3b52; color: #a1c8ff; }
  .kind.leaf { background: #2a4536; color: #7fd6a0; }
  .count { color: #888; font-size: 11px; }
  .sizewrap { display: flex; align-items: center; gap: 6px; margin-left: auto; }
  .sizebar { height: 8px; background: #4ecbc4; border-radius: 2px; min-width: 2px; }
  .sizenum { color: #aaa; font-size: 11px; width: 56px; text-align: right; }
  .children { margin-left: var(--row-pad); border-left: 1px solid #2a2a2a; }
  .entry { padding: 2px 6px; }
  .entry .meta { color: #888; font-size: 11px; margin-left: 22px; }
  .status { color: #888; padding: 4px 6px; font-style: italic; }
  .error { color: #ff8da1; padding: 4px 6px; }
</style>
<div class="tree" part="tree"></div>
`,x=class extends HTMLElement{#t;#e=null;#n=1;constructor(){super(),this.#t=this.attachShadow({mode:"open"}),this.#t.innerHTML=T}get loader(){return this.#e}set loader(e){this.#e=e,this.#r()}async refresh(){await this.#r()}get#i(){return this.#t.querySelector(".tree")}async#r(){let e=this.#i;if(e.textContent="",!this.#e){this.#a(e,"no loader set");return}this.#a(e,"loading\u2026");try{let t=await this.#e.root();if(e.textContent="",!t){this.#a(e,"empty tree");return}this.#n=Math.max(this.#n,t.size),e.appendChild(this.#o(t,0))}catch(t){e.textContent="",this.#s(e,t)}}#o(e,t){let n=document.createElement("div"),a=document.createElement("div");a.className="row";let r=document.createElement("span");r.className="twist";let d=e.count>0;r.textContent=d?"\u25B8":"\xB7",d||r.classList.add("leaf-marker"),a.appendChild(r);let i=document.createElement("span");i.className="hash",i.textContent=E(e.hash),i.title=e.hash,a.appendChild(i);let c=document.createElement("span");c.className=`kind ${e.kind}`,c.textContent=e.kind,a.appendChild(c);let o=document.createElement("span");o.className="count",o.textContent=e.kind==="branch"?`${e.count} children`:`${e.count} entries`,a.appendChild(o),a.appendChild(this.#l(e.size));let l=document.createElement("div");l.className="children",l.hidden=!0;let p=!1,m=!1,y=async()=>{d&&(m=!m,r.textContent=m?"\u25BE":"\u25B8",l.hidden=!m,m&&!p&&(p=!0,await this.#c(e,l,t+1)))};return r.addEventListener("click",y),a.addEventListener("dblclick",y),n.append(a,l),n}#l(e){let t=document.createElement("span");t.className="sizewrap";let n=document.createElement("span");n.className="sizebar";let a=Math.max(.02,Math.min(1,e/this.#n));n.style.width=`calc(var(--max-bar) * ${a})`;let r=document.createElement("span");return r.className="sizenum",r.textContent=C(e),t.append(n,r),t}async#c(e,t,n){if(this.#e){this.#a(t,"loading\u2026");try{if(t.textContent="",e.kind==="branch"){let a=await this.#e.children(e.hash);for(let r of a)this.#n=Math.max(this.#n,r.size);for(let r of a)t.appendChild(this.#o(r,n))}else{let a=await this.#e.entries(e.hash);for(let r of a)t.appendChild(this.#d(r))}}catch(a){t.textContent="",this.#s(t,a)}}}#d(e){let t=document.createElement("div");t.className="entry";let n=document.createElement("div");n.className="row";let a=document.createElement("span");a.className="twist",a.textContent="\u25B8",n.appendChild(a);let r=document.createElement("span");r.textContent=e.state==="removed"?"(retracted)":`${e.attribute??"?"}`,r.style.color=e.state==="removed"?"#888":"#ffc78e",n.appendChild(r);let d=document.createElement("span");d.className="count",d.textContent=e.entity?E(e.entity):"",n.appendChild(d);let i=document.createElement("div");i.className="children",i.hidden=!0;let c=!1,o=!1,l=async()=>{o=!o,a.textContent=o?"\u25BE":"\u25B8",i.hidden=!o,o&&!c&&(c=!0,await this.#p(e.key,i))};return a.addEventListener("click",l),n.addEventListener("dblclick",l),t.append(n,i),t}async#p(e,t){if(this.#e){this.#a(t,"decoding\u2026");try{let n=await this.#e.decodeKey(e);t.textContent="";let a=document.createElement("dialog-tree-key");a.key=n,a.style.margin="4px 0 4px 22px",t.appendChild(a)}catch(n){t.textContent="",this.#s(t,n)}}}#a(e,t){let n=document.createElement("div");n.className="status",n.textContent=t,e.appendChild(n)}#s(e,t){let n=document.createElement("div");n.className="error",n.textContent=t instanceof Error?t.message:String(t),e.appendChild(n)}};function w(s="dialog-arboretum"){g(),customElements.get(s)||customElements.define(s,x)}w();export{x as DialogArboretum,u as DialogTreeKey,w as define};
//# sourceMappingURL=dialog-arboretum.js.map
