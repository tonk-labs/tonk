(function(){
  // Surface guest errors to the parent log: an opaque (null) origin sanitizes
  // `Uncaught (in promise)` / error details in the parent console to a bare
  // message, so a sealed-guest failure is otherwise undebuggable. Forwarding the
  // stack via the bridge keeps the sealed runtime diagnosable. The parent logs
  // these under "portal guest runtime warn:".
  window.addEventListener("unhandledrejection", function(ev){
    var r=ev.reason;
    parent.postMessage({__tonkRuntime:"warn",error:"unhandledrejection: "+(r&&r.stack?r.stack:String(r))},"*");
  });
  window.addEventListener("error", function(ev){
    parent.postMessage({__tonkRuntime:"warn",error:"error: "+(ev.error&&ev.error.stack?ev.error.stack:ev.message)},"*");
  });

  // Global submit guard: the iframe sandbox grants `allow-forms` only so a
  // `<form>`'s `submit` event fires (declarative `onsubmit=` bindings run on
  // it). This capture-phase listener `preventDefault`s EVERY submission
  // before its native action, so a form can never navigate the guest away or
  // POST anywhere — the event is observable, the navigation is not. Runs on
  // every submit regardless of whether the form has an app handler.
  document.addEventListener("submit", function(ev){ ev.preventDefault(); }, true);

  // A press in here is a press "outside" for every overlay an ancestor frame
  // holds open. A nested guest fills its parent's whole viewport, so once
  // content renders in one, NO click ever reaches the frame the FABB lives
  // in, and its open stack could not be dismissed by clicking away at all.
  // Events do not cross a frame boundary, so relay the fact of the press and
  // let each ancestor redispatch it on its own document, where the existing
  // dismiss listeners already handle it. Only the fact travels: no
  // coordinates, no target, nothing the ancestor could use to observe what
  // was pressed inside a sealed guest.
  document.addEventListener("pointerdown", function(){
    try{ parent.postMessage({__tonkRuntime:"press"},"*"); }catch(_){}
  }, true);

  window.addEventListener("message", function(e){
    var d=e.data; if(!d||d.__tonkRuntime!=="press") return;
    // Redispatch on THIS document so an overlay held open here closes, then
    // keep it travelling so every ancestor up to the top page does the same.
    document.dispatchEvent(new PointerEvent("pointerdown",{bubbles:true}));
    try{ parent.postMessage({__tonkRuntime:"press"},"*"); }catch(_){}
  });

  // A light/dark change made in some ancestor frame, relayed down. The
  // theme is a whole-app property, but each guest is its own document with
  // its own root element, so the only way a toggle reaches nested content is
  // to walk the frame tree. Each guest applies it and passes it on, so one
  // message reaches every depth.
  window.addEventListener("message", function(e){
    var d=e.data; if(!d||d.__tonkRuntime!=="mode") return;
    var isDark=d.mode==="dark";
    var cls=document.documentElement.classList;
    cls.toggle("wa-dark",isDark); cls.toggle("wa-light",!isDark);
    var frames=document.querySelectorAll("iframe");
    for(var i=0;i<frames.length;i++){
      try{ frames[i].contentWindow.postMessage({__tonkRuntime:"mode",mode:d.mode},"*"); }catch(_){}
    }
  });

  window.addEventListener("message", async function(e){
    var d=e.data; if(!d||d.__tonkRuntime!=="inject") return;
    try {
      // Apply the parent document's exact root classes (WA theme + palette +
      // dark/light), so the injected WA CSS resolves its custom properties
      // identically to the host page.
      if (d.rootClass) document.documentElement.className=d.rootClass;
      // The injected rootClass is a one-time snapshot, so a later OS
      // light/dark switch wouldn't reach the guest (the parent retoggles its
      // own `wa-dark`/`wa-light` on `prefers-color-scheme`, but the guest's
      // class is frozen). Watch the same OS signal here and keep the guest's
      // dark/light class live — `prefers-color-scheme` is identical inside the
      // iframe, so guest and parent stay in agreement. The theme/palette
      // classes from rootClass are untouched (they don't change).
      (function(){
        var mq=window.matchMedia("(prefers-color-scheme: dark)");
        var apply=function(isDark){
          var cls=document.documentElement.classList;
          cls.toggle("wa-dark",isDark); cls.toggle("wa-light",!isDark);
        };
        apply(mq.matches);
        mq.addEventListener("change",function(ev){apply(ev.matches);});
      })();
      // Base layout: the guest fills the iframe and lays out as a column so
      // the injected view (a `.display-route` chain) can flex to full height.
      // `color-scheme:light dark` is load-bearing, not cosmetic: a NESTED
      // guest is a cross-origin frame, and its `prefers-color-scheme` comes
      // from THIS document's used color-scheme — leave it undeclared and the
      // OS dark preference dies here, waking every deeper frame up light.
      // (The app stylesheet declares it too; this covers the beat before it
      // lands, and any guest injected without it.)
      var base=document.createElement("style");
      base.textContent="html{color-scheme:light dark}html,body{height:100%;margin:0}body{display:flex;flex-direction:column;min-height:100%}";
      document.head.appendChild(base);
      if (d.css) {
        var style=document.createElement("style");
        // Tag the injected app CSS so a NESTED guest (whose parent is THIS guest,
        // not the top document) can discover it: the parent has no
        // `<link rel=stylesheet href=/styles-*.css>` to read the href from — its
        // app CSS lives in this inline `<style>` — so `app_stylesheet_css()`
        // reads the content back off `[data-tonk-app-css]`.
        style.setAttribute("data-tonk-app-css","");
        style.textContent=d.css;
        document.head.appendChild(style);
      }
      // Web Awesome component bundle: a self-contained ESM (no dynamic or
      // relative imports). `d.wa` is the transferred ArrayBuffer (ownership
      // moved, no copy); wrap it in a Blob (a zero-copy view over the bytes)
      // and import the URL so the <wa-*> elements upgrade with no network.
      if (d.wa) {
        var waUrl=URL.createObjectURL(new Blob([d.wa],{type:"text/javascript"}));
        await import(waUrl);
      }
      // Rewrite each snippet import statement to a guest-minted blob URL.
      var glue=d.glue;
      for (var i=0;i<d.snippets.length;i++){
        var s=d.snippets[i];
        var url=URL.createObjectURL(new Blob([s.src],{type:"text/javascript"}));
        glue=glue.replace(s.stmt, s.stmt.replace(/from\s*['"][^'"]*['"]/, 'from "'+url+'"'));
      }
      var glueUrl=URL.createObjectURL(new Blob([glue],{type:"text/javascript"}));
      var mod=await import(glueUrl);
      await mod.default({ module_or_path: d.wasm });
      mod.start();
      // Code-split editor bundles load sibling chunks via RELATIVE imports,
      // dead at this opaque origin. Mint a blob per file in DEPENDENCY ORDER
      // so each file's relative imports rewrite to the FINAL blob URLs of
      // already-minted deps. The esbuild chunk graph is a DAG (shared chunks
      // are leaves), so repeated passes that mint any file whose deps are all
      // minted converge; a file with an unminted relative dep is deferred to
      // a later pass. `rewrite` hooks per-bundle source patching (runtime URL
      // templates that the static "./<name>" rewrite can't reach).
      var mintGraph=function(files, rewrite){
        var srcByName={};
        for (var ci=0; ci<files.length; ci++){ srcByName[files[ci].name]=files[ci].src; }
        var relImports=function(src){
          var out=[],re=/['"]\.\/([^'"$]+)['"]/g,m;
          while((m=re.exec(src))) if(out.indexOf(m[1])<0) out.push(m[1]);
          return out;
        };
        var blobs={};            // name -> final blob URL
        var pending=Object.keys(srcByName);
        var guard=0;
        while (pending.length && guard++ < 20){
          var next=[];
          for (var pi=0; pi<pending.length; pi++){
            var name=pending[pi];
            var deps=relImports(srcByName[name]).filter(function(n){return srcByName[n]!==undefined;});
            var ready=deps.every(function(n){return blobs[n];});
            if(!ready){ next.push(name); continue; }
            var out=srcByName[name];
            for (var di=0; di<deps.length; di++){
              out=out.split('"./'+deps[di]+'"').join('"'+blobs[deps[di]]+'"');
              out=out.split("'./"+deps[di]+"'").join("'"+blobs[deps[di]]+"'");
            }
            if (rewrite) out=rewrite(out);
            blobs[name]=URL.createObjectURL(new Blob([out],{type:"text/javascript"}));
          }
          pending=next;
        }
        return blobs;
      };
      // The <tonk-code> editor bundle. LAZY end-to-end: nothing rides the
      // boot payload at all, not even a shell. Unlike tonk-prose/tonk-table
      // — whose builds split a tiny registration shell from a heavy core —
      // `tonk-code.js` IS the element definition, so there is nothing cheap
      // to register up front. The whole ~659 kB graph (main + dialog-yaml
      // pack + shared chunks) crosses the boundary only once something that
      // needs it appears in the DOM.
      //
      // The trigger is the DOM, not a connectedCallback: with the element
      // undefined, `<tonk-code>` gets no callbacks, so it cannot ask for
      // itself. Both consumers (tonk-inspector, tonk-notebook) append a
      // `<tonk-diagnostics-provider>` and THEN await
      // `customElements.whenDefined("tonk-code")` before mounting an editor
      // — a promise that simply stays pending until the import below runs
      // `define`. So observing either tag's arrival catches every real use,
      // and the consumers need no change: their existing wait resolves when
      // the bundle lands. Both mount into LIGHT dom, so a document-wide
      // subtree observer reaches them.
      (function(){
        var CODE_TAGS=["TONK-CODE","TONK-DIAGNOSTICS-PROVIDER"];
        var requested=false;
        var observer=null;
        var wants=function(node){
          if (!node || node.nodeType!==1) return false;
          if (CODE_TAGS.indexOf(node.tagName)>=0) return true;
          // A subtree can arrive in one mutation (a node view, an innerHTML
          // swap), so the added node itself is not necessarily the match.
          return typeof node.querySelector==="function"
            && !!node.querySelector("tonk-code,tonk-diagnostics-provider");
        };
        var load=function(){
          if (requested) return;
          requested=true;
          if (observer) { observer.disconnect(); observer=null; }
          // A failed relay must not poison the trigger: clear `requested` and
          // re-arm the observer so the next element to appear retries the
          // whole handshake. (tonk-prose/tonk-table clear their cached core
          // promise for the same reason — there the next connect retries; here
          // the element is still undefined, so the next arrival is the retry.)
          var retry=function(){
            requested=false;
            if (!observer) {
              observer=new MutationObserver(onMutations);
              observer.observe(document.documentElement,{childList:true,subtree:true});
            }
          };
          var timer=setTimeout(function(){
            window.removeEventListener("message",onCode);
            parent.postMessage({__tonkRuntime:"warn",error:"tonk-code: no inject-code reply from parent"},"*");
            retry();
          },15000);
          var onCode=function(e){
            var m=e.data; if(!m||m.__tonkRuntime!=="inject-code") return;
            clearTimeout(timer);
            window.removeEventListener("message",onCode);
            try {
              var codeBlobs=mintGraph(m.code||[]);
              // Seed the minted blob map BEFORE importing: the element's
              // on-demand language loader reads `window.__tonkCodeChunks` at
              // module-eval time and reuses these SHARED chunk-*.js blobs
              // (esp. @codemirror/state/view/language). Re-minting them for a
              // language pack would create a second @codemirror/state
              // identity, and CodeMirror's instanceof checks reject the pack
              // ("Unrecognized extension value … multiple instances of
              // @codemirror/state").
              window.__tonkCodeChunks=codeBlobs;
              var entry=codeBlobs["tonk-code.js"];
              if (!entry) throw new Error("tonk-code: element bundle missing from inject-code");
              // Defining the element resolves the consumers' pending
              // `whenDefined`, which is what actually mounts the editors.
              import(entry).catch(function(importErr){
                parent.postMessage({__tonkRuntime:"warn",error:"tonk-code import: "+String(importErr)+(importErr&&importErr.stack?"\n"+importErr.stack:"")},"*");
                retry();
              });
            } catch(err) {
              // A missing editor must not abort the rest of the guest runtime.
              parent.postMessage({__tonkRuntime:"warn",error:"tonk-code inject: "+String(err)+(err&&err.stack?"\n"+err.stack:"")},"*");
              retry();
            }
          };
          window.addEventListener("message",onCode);
          parent.postMessage({__tonkRuntime:"need-code"},"*");
        };
        var onMutations=function(records){
          for (var ri=0; ri<records.length; ri++){
            var added=records[ri].addedNodes;
            for (var ai=0; ai<added.length; ai++){
              if (wants(added[ai])) { load(); return; }
            }
          }
        };
        // Anything already in the document (a server-rendered view, or a
        // fast consumer that mounted before this ran) counts as demand.
        if (document.querySelector("tonk-code,tonk-diagnostics-provider")) { load(); return; }
        observer=new MutationObserver(onMutations);
        observer.observe(document.documentElement,{childList:true,subtree:true});
      })();
      // The <tonk-prose> markdown editor. LAZY end-to-end: the boot payload
      // carries only the ~4 kB registration shell; the ~400 kB editor core
      // crosses the boundary only when the first <tonk-prose> actually
      // connects. The shell resolves the core via import.meta.url, dead at
      // this origin — it consults window.__tonkProseEditor first, and
      // accepts a FUNCTION returning a promised URL: ours asks the trusted
      // parent for the core's bytes (`need-prose`), mints blobs from the
      // `inject-prose` reply, and resolves the core's blob URL. Imported
      // AFTER tonk-code so code blocks inside documents upgrade to embedded
      // <tonk-code> editors (the node view checks for the element at draw
      // time).
      if (d.prose && d.prose.length) {
        try {
          var proseBlobs=mintGraph(d.prose);
          var proseCore=null;
          window.__tonkProseEditor=function(){
            if (!proseCore) {
              proseCore=new Promise(function(resolve,reject){
                var timer=setTimeout(function(){
                  window.removeEventListener("message",onProse);
                  reject(new Error("tonk-prose: no inject-prose reply from parent"));
                },15000);
                var onProse=function(e){
                  var m=e.data; if(!m||m.__tonkRuntime!=="inject-prose") return;
                  clearTimeout(timer);
                  window.removeEventListener("message",onProse);
                  try {
                    var blobs=mintGraph(m.prose||[]);
                    var url=blobs["tonk-prose-editor.js"];
                    if (url) resolve(url);
                    else reject(new Error("tonk-prose: editor core missing from inject-prose"));
                  } catch(err) { reject(err); }
                };
                window.addEventListener("message",onProse);
                parent.postMessage({__tonkRuntime:"need-prose"},"*");
              });
              // A failed request must not poison the cache — the shell also
              // clears its module promise on failure, so the next element
              // connect retries the whole handshake.
              proseCore.catch(function(){ proseCore=null; });
            }
            return proseCore;
          };
          await import(proseBlobs["tonk-prose.js"]);
        } catch(proseErr) {
          // Same containment as tonk-code: a missing markdown editor must not
          // abort the rest of the guest runtime.
          parent.postMessage({__tonkRuntime:"warn",error:"tonk-prose inject: "+String(proseErr)+(proseErr&&proseErr.stack?"\n"+proseErr.stack:"")},"*");
        }
      }
      // The <tonk-table> spreadsheet. LAZY end-to-end like <tonk-prose>
      // above: the boot payload carries only the registration shell; the
      // grid core AND the multi-megabyte engine-bytes leaf cross the
      // boundary only when the first <tonk-table> actually connects. The
      // shell consults window.__tonkTableGrid — ours asks the trusted
      // parent for the grid graph (`need-table`), mints blobs from the
      // `inject-table` reply (the grid's relative import of the engine
      // leaf rewrites to its blob in dependency order), and resolves the
      // grid's blob URL. The engine then instantiates from the leaf's
      // embedded bytes — no fetch, which is why it works at this opaque
      // origin at all.
      if (d.table && d.table.length) {
        try {
          var tableBlobs=mintGraph(d.table);
          var tableGrid=null;
          window.__tonkTableGrid=function(){
            if (!tableGrid) {
              tableGrid=new Promise(function(resolve,reject){
                var timer=setTimeout(function(){
                  window.removeEventListener("message",onTable);
                  reject(new Error("tonk-table: no inject-table reply from parent"));
                },15000);
                var onTable=function(e){
                  var m=e.data; if(!m||m.__tonkRuntime!=="inject-table") return;
                  clearTimeout(timer);
                  window.removeEventListener("message",onTable);
                  try {
                    var blobs=mintGraph(m.table||[]);
                    var url=blobs["tonk-table-grid.js"];
                    if (url) resolve(url);
                    else reject(new Error("tonk-table: grid core missing from inject-table"));
                  } catch(err) { reject(err); }
                };
                window.addEventListener("message",onTable);
                parent.postMessage({__tonkRuntime:"need-table"},"*");
              });
              // A failed request must not poison the cache — the shell also
              // clears its module promise on failure, so the next element
              // connect retries the whole handshake.
              tableGrid.catch(function(){ tableGrid=null; });
            }
            return tableGrid;
          };
          await import(tableBlobs["tonk-table.js"]);
        } catch(tableErr) {
          // Same containment as tonk-prose: a missing spreadsheet must not
          // abort the rest of the guest runtime.
          parent.postMessage({__tonkRuntime:"warn",error:"tonk-table inject: "+String(tableErr)+(tableErr&&tableErr.stack?"\n"+tableErr.stack:"")},"*");
        }
      }
    } catch(err) {
      parent.postMessage({__tonkRuntime:"error",error:String(err)+(err&&err.stack?"\n"+err.stack:"")},"*");
    }
  });
  parent.postMessage({__tonkRuntime:"runtime-ready"},"*");
})();