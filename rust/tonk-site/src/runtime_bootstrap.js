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

  // Bring the element runtime up. `d` says how: a guest on its own origin
  // loads everything from there (`fromOrigin`), a sealed one is handed it.
  var start=async function(d){
    try {
      // Apply the parent document's exact root classes (WA theme + palette +
      // dark/light), so the injected WA CSS resolves its custom properties
      // identically to the host page.
      if (d.rootClass) document.documentElement.className=d.rootClass;
      // The theme and palette the app's stylesheet is written for. The page
      // that frames the outermost site carries no theme of its own to hand
      // down, so each guest names them itself.
      document.documentElement.classList.add("wa-theme-default","wa-palette-shoelace");
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
      if (d.fromOrigin) {
        // On its own origin the guest brings the runtime in itself: it reads
        // which build's files to load from its own origin, links the
        // stylesheets (fonts then resolve against them) and imports the
        // modules. Its worker and the HTTP cache keep them, and nothing
        // crosses the frame.
        var manifest=window.tonkBuildFiles||await (await fetch("/guest/manifest.json",{cache:"no-cache"})).json();
        var link=function(href){
          var l=document.createElement("link");
          l.rel="stylesheet"; l.href=href;
          document.head.appendChild(l);
        };
        link("/guest/"+manifest.waCss);
        if (manifest.css) link("/"+manifest.css);
        // Icons are the ones the app ships, read from this origin: the
        // page reaches no other, and nothing is fetched from a third party.
        (await import("/guest/"+manifest.waJs)).setIconPath("/webawesome/icons");
        var mod=await import("/guest/"+manifest.js);
        await mod.default({ module_or_path: "/guest/"+manifest.wasm });
        mod.start();
      } else {
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
      }
    } catch(err) {
      parent.postMessage({__tonkRuntime:"error",error:String(err)+(err&&err.stack?"\n"+err.stack:"")},"*");
    }
  };
  if (location.origin!=="null") {
    // On an origin of its own the guest starts at once: it has everything
    // it needs there, and nothing to wait on the page around it for.
    window.tonkRuntime=start({fromOrigin:true});
  } else {
    window.addEventListener("message", function(e){
      var d=e.data; if(!d||d.__tonkRuntime!=="inject") return;
      start(d);
    });
    parent.postMessage({__tonkRuntime:"runtime-ready"},"*");
  }
})();