//! The live-data bridge injected into a portal's iframe.
//!
//! A portal mounts an **opaque-origin** iframe (`sandbox="allow-scripts"`)
//! and prepends a small bootstrap script to its document. That script
//! defines `window.tonk` synchronously, opens a [`MessageChannel`], and
//! posts a `hello` envelope to its parent transferring one port. The
//! iframe keeps the other port; thereafter author code and the parent
//! communicate only over that port.
//!
//! [`MessageChannel`]: https://developer.mozilla.org/docs/Web/API/MessageChannel
//!
//! The author-facing object is unchanged:
//!
//! ```text
//! window.tonk = {
//!   context: { this, model },
//!   query(body?)      -> Promise<Conclusion[]>,
//!   subscribe(body?)  -> ReadableStream<Conclusion[]>,
//!   transact(request) -> Promise<receipt>,
//!   navigate(href)    -> void,
//!   reload()           -> void,
//!   setTitle(text)    -> void,
//!   open(href)        -> void,
//!   ready: Promise<void>,
//! }
//! ```
//!
//! `tonk` is defined synchronously when the bootstrap runs, so author
//! top-level `tonk.query()` keeps working; each method `await`s `ready`
//! internally before posting.
//!
//! The parent is a pure **port relay**. One page-level `message`
//! listener (installed once) authenticates a `hello` by matching
//! `event.source` against the registered iframes' live `contentWindow`
//! — never by `event.origin`, which is `"null"` at an opaque origin.
//! On a match it binds the transferred port to that portal's
//! [`PortalState`] and posts `ready { context }` back. The per-port
//! dispatcher then translates each inbound envelope into the existing
//! `tonk-query` / `tonk-subscribe` / `tonk-claim` consumer events on the
//! `<tonk-portal>` element, which bubble to the installed host on the
//! document. Subscription frames arrive back through the
//! portal's `reset` / `error` methods (the same seam `<tonk-display>`
//! uses) and are posted to the iframe as `subscribe-event` /
//! `subscribe-error` envelopes.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use js_sys::{Object, Reflect};
use tonk_host::consumer::{self as host_consumer, Subscription as HostSubscription};
use tonk_host::location::{Allow, Location};
use tonk_worker_api::Conclusion;
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use wasm_bindgen::closure::Closure;
use wasm_bindgen_futures::spawn_local;
use web_sys::{AbortController, Element, HtmlIFrameElement, MessageEvent, MessagePort, window};

/// Per-portal bridge + iframe state. Held behind `Rc<RefCell<…>>` so
/// it is reachable from the element lifecycle, the prototype `reset`
/// delegate, and the page-level message listener.
pub(crate) struct PortalState {
    /// The single child iframe. Owned here so attribute callbacks can
    /// reload it and `disconnected_callback` can detach it.
    pub iframe: Option<HtmlIFrameElement>,
    /// Set by `disconnected_callback`; mirrors `<tonk-display>`.
    pub disposed: bool,
    /// Monotonic counter minting unique host subscription tags.
    next_tag: u64,
    /// Live subscriptions keyed by the host tag we minted. Dropping an
    /// entry cancels its host subscription.
    subs: BTreeMap<String, BridgeSub>,
    /// Abort handles for every fetch this portal relayed. Aborted (and
    /// drained) on teardown so no response keeps streaming into a
    /// destroyed guest realm.
    relays: Vec<AbortController>,
    /// The port bound by the latest `hello` handshake, used to relay
    /// results back to the iframe. `None` until the iframe says hello.
    port: Option<MessagePort>,
    /// The current port's `onmessage` dispatcher, kept alive for the
    /// port's lifetime. Replaced on each handshake.
    _dispatcher: Option<Closure<dyn FnMut(MessageEvent)>>,
    /// The portal's own routing context (its `with`). Relayed guest
    /// operations with no forwarded route are pinned to it explicitly;
    /// `allow`'s `self` entry resolves to it.
    with: Option<Location>,
    /// Which locations this portal permits its guest to reach. A
    /// **privilege of the trusted portal element**, set host-side at
    /// construction — NOT something the guest can assert. `<tonk-site>`
    /// derives it from its `allow` attribute; `<tonk-fab-portal>` grants
    /// `*`; the generic `<tonk-portal>` grants `self`, so a
    /// synced/untrusted content guest's forwarded route is denied with a
    /// typed error. See `forwarded_route`.
    allow: Allow,
}

/// One live subscription: the iframe's correlation id (so frames are
/// addressed to the right author stream) plus the host subscription
/// handle (whose `Drop` cancels upstream). The port to relay frames on
/// is always the portal's current [`PortalState::port`], never a stored
/// clone — a subscription cannot outlive the port it was opened under,
/// since `reload` clears the subs before the next handshake rebinds.
struct BridgeSub {
    iframe_id: String,
    _host_sub: HostSubscription,
}

impl PortalState {
    pub(crate) fn new() -> Self {
        Self {
            iframe: None,
            disposed: false,
            next_tag: 0,
            subs: BTreeMap::new(),
            relays: Vec::new(),
            port: None,
            _dispatcher: None,
            with: None,
            allow: Allow::none(),
        }
    }

    /// Set this portal's routing context and reach. Called once, host-side,
    /// by the trusted portal element during `connect_portal`.
    pub(crate) fn set_route(&mut self, with: Option<Location>, allow: Allow) {
        self.with = with;
        self.allow = allow;
    }

    /// This portal's target space (a `did:key` string), or `None` when it
    /// targets the profile/Hub. Drives the guest's synthetic `<base>` origin.
    pub(crate) fn route_space(&self) -> Option<String> {
        self.with
            .as_ref()
            .and_then(|w| w.space())
            .map(str::to_owned)
    }

    /// Whether this portal's routing context and reach match exactly.
    /// `<tonk-site>` uses this to re-route a pure path change in place —
    /// same reach, live iframe — instead of rebuilding the guest.
    pub(crate) fn same_route(&self, with: &Location, allow: &Allow) -> bool {
        self.with.as_ref() == Some(with) && self.allow == *allow
    }

    /// Cancel and forget every live subscription and relayed fetch.
    /// Dropping each `BridgeSub` cancels its host subscription, and
    /// aborting each relay cancels the underlying fetch — including a
    /// streaming response whose body was TRANSFERRED into the guest. A
    /// torn-down guest must not leave live pipes into its destroyed
    /// realm: orphaned transferred streams are the prime suspect for the
    /// renderer crash on space→hub navigation.
    pub(crate) fn clear_subs(&mut self) {
        self.subs.clear();
        for relay in self.relays.drain(..) {
            relay.abort();
        }
        // Close the bridge port too: a torn-down (or reloading) guest must
        // leave NO live browser-brokered endpoints behind — the in-flight
        // chunk drains terminate via the aborts above and close their own
        // ports.
        if let Some(port) = self.port.take() {
            port.close();
        }
    }

    /// Track a relayed fetch's abort handle for the portal's lifetime, so
    /// teardown can cancel it. Bounded by the portal's own lifetime — a
    /// navigation rebuild drains the lot.
    pub(crate) fn track_relay(&mut self, controller: AbortController) {
        self.relays.push(controller);
    }
}

/// The bootstrap script prepended into the iframe's `srcdoc`. It defines
/// `window.tonk` synchronously, opens a `MessageChannel`, and hands one
/// port to the parent via `parent.postMessage(hello, "*", [port2])`.
/// Posting to `"*"` is unavoidable from a null origin; the parent
/// authenticates by `event.source`, not `event.origin`.
const BOOTSTRAP_JS: &str = r#"(function(){
  var nextId=0, pending=new Map(), streams=new Map(), subRows=new Map(), registerFocus=new Map();
  var resolveReady; var ready=new Promise(function(r){resolveReady=r;});
  var ch=new MessageChannel(), port=ch.port1;
  function mint(){return "r"+(++nextId);}
  // Merge an optional per-call routing context ({with}) into an envelope.
  // The guest relay passes the `branch@repo` location its in-guest `with`
  // ancestry resolved. The host parses it and honors it ONLY when the
  // portal's `allow` permits it (denied with a typed error otherwise), so
  // this is always safe to send.
  function withRoute(extra,ctx){
    if(ctx&&ctx.with){ extra.with=ctx.with; }
    return extra;
  }
  // The request-context headers every relayed /api fetch carries, so the SW can
  // tie the request to this tab's SITE and route/contain it. Site, path, and hash
  // come from the injected context (the host's site id + the host's location;
  // the guest's own location is about:srcdoc). They are explicit headers because
  // a service worker reads request.headers, which never includes Referer (the
  // browser exposes it only as request.referrer, not as a header). Returns
  // [[name,value]] pairs prepended to any per-request headers.
  function contextHeaders(){
    var c=(window.tonk&&window.tonk.context)||{};
    var headers=[];
    if(c.site){ headers.push(["x-tonk-site",c.site]); }
    if(c.path){ headers.push(["x-tonk-path",c.path]); }
    if(c.hash){ headers.push(["x-tonk-hash",c.hash]); }
    return headers;
  }
  function call(type,extra){
    return ready.then(function(){
      return new Promise(function(resolve,reject){
        var id=mint(); pending.set(id,{resolve:resolve,reject:reject});
        port.postMessage(Object.assign({v:1,type:type,id:id},extra));
      });
    });
  }
  // In-flight de-duplication for one-shot queries. Many <tonk-display>
  // elements resolve the SAME concept descriptor (phase-1) or bookmark name
  // on one page load — e.g. three displays of `tonk:repository` each fire an
  // identical `db.meta/*` query. Coalesce identical concurrent queries
  // onto one request keyed by (route + body); every caller shares the single
  // promise. Purely in-flight (cleared when it settles), so no staleness —
  // just fewer round-trips. A subscription is never deduped here (it's a
  // long-lived stream), only the fire-and-forget `query`.
  var inflightQ=new Map();
  function dedupQuery(env){
    var key;
    try{ key=JSON.stringify(env); }catch(e){ return call("query",env); }
    var hit=inflightQ.get(key);
    if(hit) return hit;
    var p=call("query",env).finally(function(){ inflightQ.delete(key); });
    inflightQ.set(key,p);
    return p;
  }
  var tonk={
    context:{this:"",model:""},
    ready:ready,
    query:function(body,ctx){return dedupQuery(withRoute({body:body},ctx));},
    transact:function(request,ctx){return call("transact",withRoute({request:request},ctx));},
    // Evaluate an asserted-notation document against the branch. `detail` carries
    // {document, transact}; the parent relays it to the installed host's
    // consumer path, which performs the typed evaluate and returns its parsed result.
    evaluate:function(detail){return call("evaluate",{document:(detail&&detail.document)||"",transact:!(detail&&detail.transact===false)});},
    // Ask the HOST page to delegate: the account root lives behind the
    // passkey, and WebAuthn exists only on the top-level window, inside a
    // user gesture. A guest click posts {subject, command, audience} here;
    // the parent runs the ceremony and answers with the minted hop (base58
    // of the serialized chain), or rejects with the reason.
    delegate:function(request){return call("delegate",request||{});},
    // Navigate the HOST page: the opaque guest can't touch parent.location
    // and has no router, so a link click posts its href here and the parent
    // performs the real navigation. Fire-and-forget (no response).
    navigate:function(href){
      ready.then(function(){port.postMessage({v:1,type:"navigate",href:href});});
    },
    // Reload the HOST page after a whole-profile state swap. Unlike navigate,
    // this is meaningful when the route itself has not changed: every portal
    // and subscription owned by the previous profile must be rebuilt.
    reload:function(){
      ready.then(function(){port.postMessage({v:1,type:"reload"});});
    },
    // Retitle the HOST page's tab: the opaque guest can't touch
    // parent.document.title. `<tonk-title>` posts its text here and the
    // parent performs the real assignment. Fire-and-forget (no response).
    setTitle:function(text){
      ready.then(function(){port.postMessage({v:1,type:"title",text:text});});
    },
    // Open a link from the HOST: the opaque guest has neither `allow-popups`
    // nor `allow-top-navigation`, so a click on an external link posts its
    // raw href here and the parent decides — resolving it against the real
    // origin, allowlisting the scheme, and confirming anything off-origin.
    // Fire-and-forget (no response).
    open:function(href){
      ready.then(function(){port.postMessage({v:1,type:"open",href:href});});
    },
    // Raise the registration dialog on the HOST page. Sharing needs an
    // account, and only the top page can run the ceremony: WebAuthn wants
    // a `window` and a user gesture, which the guest's opaque realm and
    // the service worker both lack. The guest posts the refusal class so
    // the host can word the prompt. Fire-and-forget (no response).
    register:function(reason){
      var opener=document.activeElement;
      var token=(opener&&opener!==document.body)?mint():"";
      if(token){ registerFocus.set(token,opener); }
      ready.then(function(){port.postMessage({v:1,type:"register",reason:reason,focusToken:token});});
    },
    // Same-origin request performed by the HOST: the opaque guest can't reach a
    // same-origin, SW-routed `/api/...` endpoint itself. The host issues the
    // request on its real origin and streams the response back; we rebuild a
    // real `Response`. The full request (method, headers, body) is forwarded so
    // POST query/subscribe/transact route through here, not just GET. See the
    // `window.fetch` override below.
    fetch:function(path,req){
      req=req||{};
      return ready.then(function(){
        return new Promise(function(resolve,reject){
          var id=mint(); pending.set(id,{resolve:resolve,reject:reject});
          port.postMessage({v:1,type:"fetch",id:id,path:path,
            method:req.method||"GET",headers:req.headers||[],body:req.body});
        });
      });
    },
    subscribe:function(body,ctx){
      var id=mint();
      return new ReadableStream({
        start:function(controller){
          streams.set(id,controller);
          ready.then(function(){port.postMessage(withRoute({v:1,type:"subscribe",id:id,body:body},ctx));},
                     function(err){streams.delete(id);controller.error(err);});
        },
        cancel:function(){
          streams.delete(id);subRows.delete(id);
          port.postMessage({v:1,type:"unsubscribe",id:id});
        }
      });
    }
  };
  port.onmessage=function(event){
    var env=event.data; if(!env) return;
    switch(env.type){
      case "ready": tonk.context=env.context; resolveReady(); return;
      case "query-result": case "transact-result": {
        var h=pending.get(env.id); if(!h) return; pending.delete(env.id);
        h.resolve("rows" in env ? env.rows : env.receipt); return;
      }
      case "evaluate-result": {
        var h=pending.get(env.id); if(!h) return; pending.delete(env.id);
        h.resolve(env.result); return;
      }
      case "delegate-result": {
        var h=pending.get(env.id); if(!h) return; pending.delete(env.id);
        h.resolve(env.delegation); return;
      }
      case "register-focus": {
        var opener=registerFocus.get(env.focusToken);
        registerFocus.delete(env.focusToken);
        if(opener&&opener.isConnected&&!opener.matches(":disabled")){
          window.focus();
          opener.focus({preventScroll:true});
        }
        return;
      }
      case "register-focus-discard": {
        registerFocus.delete(env.focusToken); return;
      }
      case "fetch-result": {
        var h=pending.get(env.id); if(!h) return; pending.delete(env.id);
        // Rebuild a real Response from the status/headers the host captured
        // plus the body. The body arrives one of three ways:
        //   - env.body is a transferred ReadableStream (fast path) — use it.
        //   - env.streamPort is a transferred MessagePort (Safari fallback) —
        //     wrap it in a ReadableStream that pulls chunks with credit-based
        //     backpressure: grant credit when the consumer wants more, enqueue
        //     each {type:"chunk"}, close on {type:"close"}, error on
        //     {type:"error"}, and post {type:"cancel"} if the reader cancels.
        //   - neither — a bodyless response.
        var headers=new Headers(env.headers||[]);
        var body=null;
        if (env.body!==undefined) {
          body=env.body;
        } else if (env.streamPort) {
          var sp=env.streamPort;
          body=new ReadableStream({
            start:function(controller){
              sp.onmessage=function(ev){
                var m=ev.data; if(!m) return;
                if(m.type==="chunk"){
                  controller.enqueue(new Uint8Array(m.chunk,m.byteOffset||0,m.byteLength!==undefined?m.byteLength:m.chunk.byteLength));
                  // Ask for more while the consumer still has appetite.
                  if(controller.desiredSize>0){ sp.postMessage({type:"credit",n:1}); }
                } else if(m.type==="close"){
                  controller.close(); sp.close();
                } else if(m.type==="error"){
                  controller.error(new Error(m.error||"stream error")); sp.close();
                }
              };
              // Prime the pump: grant initial credit sized to the consumer's
              // appetite (default 1 when desiredSize is null).
              sp.postMessage({type:"credit",n:controller.desiredSize>0?controller.desiredSize:1});
            },
            pull:function(controller){
              sp.postMessage({type:"credit",n:controller.desiredSize>0?controller.desiredSize:1});
            },
            cancel:function(){ sp.postMessage({type:"cancel"}); sp.close(); }
          });
        }
        var rebuilt=new Response(body,
          {status:env.status,statusText:env.statusText,headers:headers});
        // `url` is a readonly getter the constructor can't populate, so a
        // rebuilt response reports "". Consumers that parse it break on
        // that: reqwest's wasm client does `Url::parse(resp.url()).
        // expect_throw("url parse")` while converting EVERY response, so
        // any Rust component fetching from inside the guest (e.g.
        // `<tonk-default-remote>` reading /.well-known/tonk) throws
        // instead of returning. Shadow the getter with an own property
        // carrying the URL the host actually fetched.
        try{ Object.defineProperty(rebuilt,"url",
          {value:env.url||"",configurable:true}); }catch(e){}
        h.resolve(rebuilt);
        return;
      }
      case "query-error": case "transact-error": case "evaluate-error": case "fetch-error": case "delegate-error": {
        var h=pending.get(env.id); if(!h) return; pending.delete(env.id);
        h.reject(new Error(env.error)); return;
      }
      case "subscribe-event": {
        var c=streams.get(env.id); if(!c) return;
        // The guest's window.tonk.subscribe() is documented as a stream of
        // full Conclusion[] snapshots. The host sends either a full set
        // (env.rows) or a delta (env.delta = {asserted,retracted}); keep a
        // retained set per stream and always enqueue the full array so the
        // author-facing contract is unchanged.
        try{
          var prev=subRows.get(env.id)||[];
          var next;
          if(env.delta){
            var rej=env.delta.retracted||[];
            var add=env.delta.asserted||[];
            var keyOf=function(r){return JSON.stringify(r);};
            // Value-equality retract, tracking which retracts found no
            // matching row (drift) and which `this` the delta asserts.
            // Mirrors tonk-display's apply_delta: an asserted row for an
            // entity whose retract didn't match a retained row supersedes
            // that entity's stale (drifted) rows, so a superseded field
            // leaves ONE row for the entity, not two that a group-by-`this`
            // fold would collapse to a stale/multi-valued field. Clean
            // supersessions, pure retracts, and directory multi-valued
            // entities (retract matches the changed tuple) are unaffected.
            var gone={};for(var i=0;i<rej.length;i++){gone[keyOf(rej[i])]=true;}
            var drifted={};for(var i=0;i<rej.length;i++){drifted[rej[i].this]=true;}
            // Slot identity mirrors tonk-display's row_slots: each field,
            // refined by the entry key when the value is a single-entry
            // object (keyed collections arrive one row per entry). The
            // heal replaces a drifted row only when an asserted row for
            // the same entity claims one of ITS slots, so a superseded
            // show{directory} never takes the sibling show{ui} with it.
            var slotsOf=function(r){
              var out={};var f=r.fields||{};
              for(var k in f){ if(k==="this") continue;
                var v=f[k];var entry=null;
                if(v&&typeof v==="object"&&!Array.isArray(v)){
                  var ks=Object.keys(v); if(ks.length===1) entry=ks[0];
                }
                out[k+"\u001e"+(entry===null?"":entry)]=true;
              }
              return out;
            };
            var asserts={};
            for(var i=0;i<add.length;i++){
              var t=add[i].this; var slots=asserts[t]||(asserts[t]={});
              var s2=slotsOf(add[i]); for(var k2 in s2) slots[k2]=true;
            }
            next=prev.filter(function(r){
              if(gone[keyOf(r)]){ delete drifted[r.this]; return false; }
              return true;
            }).filter(function(r){
              if(!drifted[r.this]) return true;
              var slots=asserts[r.this]; if(!slots) return true;
              var mine=slotsOf(r);
              for(var k3 in mine){ if(slots[k3]) return false; }
              return true;
            }).concat(add);
          }else{
            next=env.rows||[];
          }
          subRows.set(env.id,next);
          c.enqueue(next);
        }catch(e){streams.delete(env.id);subRows.delete(env.id);} return;
      }
      case "subscribe-error": {
        var c=streams.get(env.id); if(!c) return; streams.delete(env.id);subRows.delete(env.id);
        c.error(new Error(env.error)); return;
      }
    }
  };
  window.tonk=tonk;

  // Override window.fetch so guest code (and our own loaders) can fetch
  // same-origin, SW-routed resources the opaque iframe can't reach itself.
  // Host-relative requests (`/…`, not `//`) route through `tonk.fetch`, which
  // has the host perform the real fetch and transfer the response stream back;
  // everything else (absolute cross-origin, `blob:`, `data:`) passes through
  // to the native fetch — notably the runtime bootstrap's own blob-URL module
  // imports, which must never be intercepted.
  var nativeFetch=window.fetch.bind(window);
  // Normalize a fetch(input, init) call into {method, headers:[[k,v]], body}
  // the relay can postMessage. `input` may be a string or a Request; `init`
  // overrides Request fields. Body is read to text (our /api bodies are JSON
  // strings); a Request body is consumed via .text() so we return a Promise.
  function relayRequest(url,input,init){
    var method="GET", headers=contextHeaders(), bodyP=Promise.resolve(undefined);
    var reqLike=(typeof input==="object"&&input)?input:null;
    if(reqLike){ method=reqLike.method||method; }
    if(init&&init.method){ method=init.method; }
    var hsrc=(init&&init.headers)||(reqLike&&reqLike.headers);
    if(hsrc){
      if(typeof hsrc.forEach==="function"){ hsrc.forEach(function(v,k){headers.push([k,v]);}); }
      else if(Array.isArray(hsrc)){ headers=headers.concat(hsrc); }
      else { for(var k in hsrc){ if(Object.prototype.hasOwnProperty.call(hsrc,k)){headers.push([k,hsrc[k]]);} } }
    }
    if(init&&"body"in init){ bodyP=Promise.resolve(init.body); }
    else if(reqLike&&!reqLike.bodyUsed&&reqLike.body){ bodyP=reqLike.clone().text(); }
    return bodyP.then(function(body){
      return tonk.fetch(url,{method:method,headers:headers,body:body});
    });
  }
  window.fetch=function(input,init){
    var url=(typeof input==="string")?input:(input&&input.url)||"";
    // Host-relative (`/…`, not `//`): route through the relay.
    if(url.charAt(0)==="/"&&url.charAt(1)!=="/"){
      return relayRequest(url,input,init);
    }
    // Absolute URL pointing at the HOST origin: some consumers resolve a path
    // against `document.baseURI`, so a host API call can arrive fully-qualified
    // (`http://host/api/…`). At the guest's opaque origin that would be a
    // cross-origin fetch (CORS-blocked, origin `null`), so strip the origin
    // prefix and relay the path. TWO origins qualify: the REAL host origin
    // (`context.origin`), and the guest's SYNTHETIC per-space base origin
    // (`context.base`, e.g. `https://{label}.tonk.network`) — with a `<base>` set
    // to the latter, a relative `/api/…` resolves against it, so a `Request`
    // built from it is fake-origin-absolute and must be stripped the same way.
    var ctx=(window.tonk&&window.tonk.context)||{};
    var origin=ctx.origin||"";
    if(origin&&url.indexOf(origin+"/")===0){
      return relayRequest(url.slice(origin.length),input,init);
    }
    // `context.base` carries a trailing slash; drop it to get the bare origin.
    var baseOrigin=(ctx.base||"").replace(/\/$/,"");
    if(baseOrigin&&url.indexOf(baseOrigin+"/")===0){
      return relayRequest(url.slice(baseOrigin.length),input,init);
    }
    return nativeFetch(input,init);
  };

  parent.postMessage({v:1,type:"hello"},"*",[ch.port2]);
})();"#;

/// Runtime-injection bootstrap, appended after [`BOOTSTRAP_JS`] when the
/// portal is in `runtime` mode. It receives the element runtime from the
/// parent (over `window` `postMessage`, NOT the data port) and brings it up
/// inside the sealed guest: inject CSS, mint blob URLs for the glue +
/// snippet modules, rewrite the glue's relative snippet imports to those
/// blobs, import the glue, instantiate the wasm from bytes (no fetch), and
/// call `start()` to register the custom elements. The `content` markup
/// (e.g. `<tonk-display>`) is already in the document and upgrades the
/// moment the elements are defined.
///
/// The guest fetches NOTHING — the parent (trusted, networked) hands over
/// every byte. `runtime-ready` tells the parent to send.
const RUNTIME_BOOTSTRAP_JS: &str = r#"(function(){
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
      // The <tonk-code> editor bundle: import the main bundle to define
      // <tonk-code> + <tonk-diagnostics-provider>. The language pack is
      // loaded at runtime via a `./tonk-code-lang-<id>.js` URL built from
      // import.meta.url, which is the (useless) blob URL inside the guest —
      // expose a name->blob map the rewritten lookup consults instead.
      if (d.code && d.code.length) {
        try {
          var codeBlobs=mintGraph(d.code);
          // Expose the minted blob map so the element's on-demand language
          // loader reuses the SHARED chunk-*.js blobs already minted here
          // (esp. @codemirror/state/view/language). Re-minting them for a
          // language pack would create a second @codemirror/state identity,
          // and CodeMirror's instanceof checks reject the pack
          // ("Unrecognized extension value … multiple instances of
          // @codemirror/state"). The loader fetches a language chunk via the
          // proxied window.fetch and mints ONLY files not already in this map.
          window.__tonkCodeChunks=codeBlobs;
          await import(codeBlobs["tonk-code.js"]);
        } catch(codeErr) {
          // The editor failing to inject must not abort the whole runtime — the
          // rest of the view still works; the inspector just lacks an editor.
          parent.postMessage({__tonkRuntime:"warn",error:"tonk-code inject: "+String(codeErr)+(codeErr&&codeErr.stack?"\n"+codeErr.stack:"")},"*");
        }
      }
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
})();"#;

/// A `<base href>` element pinning the guest's document base to the
/// per-space synthetic origin, so the BROWSER resolves every relative URL
/// (links, forms, `new URL`, `<tonk-page>` location reads) under it. Empty
/// when there is no space origin (the profile/Hub), leaving the guest's
/// inherited base untouched. Prepended before everything so it applies from
/// the first parsed node.
fn base_tag(base: &str) -> String {
    if base.is_empty() {
        String::new()
    } else {
        // `base` is a same-origin literal we built (`https://{label}.tonk.network/`),
        // so there is nothing to escape, but keep it minimal and attribute-safe.
        format!("<base href=\"{base}\">")
    }
}

/// Prepend the bootstrap script that wires `window.tonk` to this
/// portal's bridge over a `MessagePort`. `base` is the per-space synthetic
/// origin the guest should resolve URLs against (empty = leave inherited).
pub(crate) fn bootstrap_srcdoc(content: &str, base: &str) -> String {
    format!("{}<script>{BOOTSTRAP_JS}</script>{content}", base_tag(base))
}

/// Like [`bootstrap_srcdoc`], plus the runtime-injection bootstrap: the
/// guest will ask the parent (`runtime-ready`) for the element runtime and
/// bring it up before `content`'s custom elements upgrade.
pub(crate) fn bootstrap_srcdoc_with_runtime(content: &str, base: &str) -> String {
    format!(
        "{}<script>{BOOTSTRAP_JS}</script><script>{RUNTIME_BOOTSTRAP_JS}</script>{content}",
        base_tag(base)
    )
}

/// Fetch the element runtime + app CSS (the parent is trusted + networked)
/// and post an `inject` envelope to the sealed `iframe`'s window. Called
/// when the guest signals `runtime-ready`. The guest fetches nothing; every
/// byte crosses here.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) fn inject_runtime(iframe: &HtmlIFrameElement) {
    let Some(content_window) = iframe.content_window() else {
        return;
    };
    spawn_local(async move {
        let (payload, transfer) = match build_inject_payload().await {
            Ok(p) => p,
            Err(e) => {
                tonk_common::log!("portal runtime: failed to assemble payload: {e}");
                return;
            }
        };
        // Post to the iframe window (not the data port): runtime setup is a
        // one-time window-channel handoff, distinct from the tonk data port.
        // The large binary payloads (wasm + WA bundle) are TRANSFERRED by
        // ownership via the transfer list, not structured-clone-copied.
        let _ = content_window.post_message_with_transfer(&payload, "*", &transfer);
    });
}

/// The hashed guest-asset basenames the `hash-guest.sh` post_build hook
/// writes into `guest/manifest.json`. Each names a content-hashed file under
/// the guest dir, so those assets cache immutably while the manifest itself
/// is fetched fresh on every load.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
#[derive(serde::Deserialize)]
struct GuestManifest {
    js: String,
    wasm: String,
    #[serde(rename = "waJs")]
    wa_js: String,
    #[serde(rename = "waCss")]
    wa_css: String,
}

/// Build the runtime-inject envelope by fetching the served guest bundle +
/// app stylesheet. Returns `(payload, transfer)` for
/// `post_message_with_transfer`: the payload carries the glue/css/snippets as
/// strings plus the wasm + WA bundle as ArrayBuffers, and `transfer` lists
/// those buffers so they hand off by ownership instead of being copied.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn build_inject_payload() -> Result<(JsValue, JsValue), String> {
    use wasm_bindgen::JsValue;

    // The manifest names the current build's hashed assets. It rides the
    // SW's stale-while-revalidate cache like everything else, so a sealed
    // `/space` works OFFLINE — never `no-store`, which the SW refuses to
    // cache (an offline guest could then never resolve its assets). Serving
    // a stale manifest is safe: it points at the PREVIOUS build's hashed
    // assets, which are still cached (immutable, never evicted within a cache
    // version), so the guest loads fully; SWR refreshes the manifest in the
    // background and the next load picks up the new build.
    let manifest: GuestManifest = {
        let text = fetch_text("/guest/manifest.json").await?;
        serde_json::from_str(&text).map_err(|e| format!("guest manifest: {e}"))?
    };

    let glue = fetch_text(&format!("/guest/{}", manifest.js)).await?;
    let wasm = fetch_array_buffer(&format!("/guest/{}", manifest.wasm)).await?;
    // App stylesheet — its hashed filename is discovered from the parent
    // document's own `<link rel=stylesheet href=/styles-*.css>`. The Web
    // Awesome CSS + the self-contained WA component bundle ride along so
    // `<wa-*>` elements style + upgrade inside the sealed guest with no
    // network of its own.
    let mut css = fetch_text(&format!("/guest/{}", manifest.wa_css))
        .await
        .unwrap_or_default();
    if let Some(app_css) = app_stylesheet_css().await {
        css.push('\n');
        css.push_str(&app_css);
    }
    // Inline `@font-face url("/fonts/*")` as `data:` URLs: a null-origin guest
    // can't fetch the fonts (CORS-blocked), so the host (same-origin) fetches
    // each face and base64-embeds it. Handles woff2/woff/otf/ttf — the launcher
    // ships Gestalte as `.otf`, so limiting this to woff2 left it unstyled.
    css = inline_fonts(&css).await;
    // The bundled Web Awesome components (esbuild, no dynamic/relative
    // imports), imported by the guest before its content upgrades. Fetched as
    // an ArrayBuffer (not text): the guest never manipulates it as a string,
    // it just blobs + imports it, so we transfer the bytes (ownership moved,
    // no structured-clone copy) and the guest wraps them in a Blob zero-copy.
    let wa = fetch_array_buffer(&format!("/guest/{}", manifest.wa_js))
        .await
        .unwrap_or(JsValue::UNDEFINED);

    // Find every `import … from '…/snippets/…'` statement in the glue and
    // fetch each snippet file, so the guest can rewrite them to blob URLs.
    let snippets = js_sys::Array::new();
    for (stmt, spec) in find_snippet_imports(&glue) {
        let path = format!("/guest/{}", spec.trim_start_matches("./"));
        let src = fetch_text(&path).await?;
        let entry = Object::new();
        let _ = Reflect::set(&entry, &"stmt".into(), &JsValue::from_str(&stmt));
        let _ = Reflect::set(&entry, &"src".into(), &JsValue::from_str(&src));
        snippets.push(&entry);
    }

    // The `<tonk-code>` editor bundle graph (main + lang pack + chunks), as
    // `{name, src}` entries the guest blobs and import-rewrites. Code-split, so
    // it can't be one self-contained module — the guest mints a blob per file.
    let code = bundle_graph_entries(fetch_tonk_code_bundles().await);

    // The `<tonk-prose>` markdown editor SHELL only (~4 kB): enough to
    // register the element so guest markup upgrades. The editor core stays
    // out of the boot payload — the guest requests it over `need-prose` the
    // first time an element actually connects (see the listener in
    // `install_message_listener`), so guests that never render an editor
    // never pay for one. Its code blocks embed the `<tonk-code>` element
    // injected above.
    let prose = bundle_graph_entries(fetch_tonk_prose_shell().await);

    // The `<tonk-table>` spreadsheet SHELL only: same lazy contract as
    // tonk-prose above — the grid core and the multi-megabyte IronCalc
    // engine bytes stay out of the boot payload; the guest requests them
    // over `need-table` the first time an element actually connects.
    let table = bundle_graph_entries(fetch_tonk_table_shell().await);

    let payload = Object::new();
    let _ = Reflect::set(&payload, &"__tonkRuntime".into(), &"inject".into());
    let _ = Reflect::set(&payload, &"glue".into(), &JsValue::from_str(&glue));
    let _ = Reflect::set(&payload, &"snippets".into(), &snippets);
    let _ = Reflect::set(&payload, &"code".into(), &code);
    let _ = Reflect::set(&payload, &"prose".into(), &prose);
    let _ = Reflect::set(&payload, &"table".into(), &table);
    let _ = Reflect::set(&payload, &"wasm".into(), &wasm);
    let _ = Reflect::set(&payload, &"css".into(), &JsValue::from_str(&css));
    let _ = Reflect::set(&payload, &"wa".into(), &wa);
    // Mirror the outer document's root classes (the WA theme/palette/dark
    // classes) so the guest themes identically — recomputing from
    // matchMedia inside the guest can disagree with the parent.
    let root_class = window()
        .and_then(|w| w.document())
        .and_then(|d| d.document_element())
        .map(|e| e.class_name())
        .unwrap_or_default();
    let _ = Reflect::set(
        &payload,
        &"rootClass".into(),
        &JsValue::from_str(&root_class),
    );

    // Transfer the two large binary payloads (the guest wasm + the WA bundle)
    // by OWNERSHIP rather than letting `postMessage` structured-clone-copy
    // them across the window boundary. `glue`/`css`/snippets stay as strings:
    // the guest manipulates them as text, so there's nothing to transfer.
    let transfer = js_sys::Array::new();
    if !wasm.is_undefined() {
        transfer.push(&wasm);
    }
    if !wa.is_undefined() {
        transfer.push(&wa);
    }
    Ok((payload.into(), transfer.into()))
}

/// Map a `/fonts/*` file extension to its `data:` MIME type. Any file under
/// `/fonts/` is inlined regardless of extension; this only picks a precise
/// MIME for the known font formats and falls back to a generic font MIME for
/// anything else, so a new face drops in without touching this code.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn font_mime(path: &str) -> &'static str {
    match path
        .rsplit('.')
        .next()
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("woff2") => "font/woff2",
        Some("woff") => "font/woff",
        Some("otf") => "font/otf",
        Some("ttf") => "font/ttf",
        Some("eot") => "application/vnd.ms-fontobject",
        // Unknown extension: a generic font MIME still renders (browsers sniff
        // the actual format from the bytes), so an arbitrary face still works.
        _ => "font/otf",
    }
}

/// Collect the distinct `/fonts/*` paths referenced as `url(...)` arguments
/// in `css`, in first-appearance order. Only genuine `url()` arguments
/// qualify: scanning for the raw `/fonts/` substring also matches prose in
/// comments — a comment in styles.css mentioning `` `/fonts/` `` used to
/// produce a junk `GET /fonts/%60%20(copied…` 404 on every guest boot.
fn find_font_paths(css: &str) -> Vec<String> {
    let mut paths: Vec<String> = Vec::new();
    let mut rest = css;
    while let Some(i) = rest.find("url(") {
        rest = &rest[i + "url(".len()..];
        let Some(close) = rest.find(')') else { break };
        let arg = rest[..close].trim().trim_matches(['"', '\'']);
        // Skip a bare `/fonts/` with no filename.
        if let Some(name) = arg.strip_prefix("/fonts/")
            && !name.is_empty()
            && !paths.iter().any(|p| p == arg)
        {
            paths.push(arg.to_owned());
        }
        rest = &rest[close + 1..];
    }
    paths
}

/// Replace every `url("/fonts/<name>.<ext>")` in `css` with a
/// `url("data:<mime>;base64,…")` so the sealed guest needs no font fetch.
/// Inlines ANY file under `/fonts/`, not a fixed set of extensions. Fonts
/// whose fetch/encode fails are left as-is (degrade to a fallback face).
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn inline_fonts(css: &str) -> String {
    let mut paths = find_font_paths(css);
    // Substitute longest-first so a path that is a prefix of another
    // (`a.woff` next to `a.woff2`) isn't corrupted by the shorter one's
    // replacement landing inside it.
    paths.sort_by_key(|p| std::cmp::Reverse(p.len()));

    let mut out = css.to_owned();
    for path in paths {
        if let Ok(buffer) = fetch_array_buffer(&path).await
            && let Some(b64) = array_buffer_to_base64(&buffer)
        {
            let data_url = format!("data:{};base64,{b64}", font_mime(&path));
            // Replace the path wherever it appears as a url argument.
            out = out.replace(&path, &data_url);
        }
    }
    out
}

/// Base64-encode an `ArrayBuffer` via `btoa` over a binary string. Returns
/// `None` on any JS error.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn array_buffer_to_base64(buffer: &JsValue) -> Option<String> {
    let bytes = js_sys::Uint8Array::new(buffer);
    let len = bytes.length() as usize;
    // Build a binary string (each char = one byte) for `btoa`.
    let mut binary = String::with_capacity(len);
    let vec = bytes.to_vec();
    for b in vec {
        binary.push(b as char);
    }
    window()?.btoa(&binary).ok()
}

/// Parse `import … from '<spec>'` statements whose spec contains
/// `/snippets/`, returning `(full statement, spec)` pairs.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn find_snippet_imports(glue: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for line in glue.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with("import") || !trimmed.contains("/snippets/") {
            continue;
        }
        // spec is the quoted string after `from`
        if let Some(from_idx) = trimmed.find(" from ") {
            let after = &trimmed[from_idx + 6..];
            let quote = after.chars().next();
            if let Some(q) = quote
                && let Some(end) = after[1..].find(q)
            {
                let spec = &after[1..1 + end];
                // statement without a trailing `;`-only tail variance:
                // keep the trimmed line up to and including the close quote
                let stmt_end = from_idx + 6 + 1 + end + 1;
                let stmt = trimmed[..stmt_end].to_owned();
                out.push((stmt, spec.to_owned()));
            }
        }
    }
    out
}

/// Find the relative `./…` ESM import specifiers in a module's source — both
/// static (`from"./x"`) and dynamic (`import("./x")`). Used to walk an editor
/// bundle's chunk graph (tonk-code, tonk-prose) so every referenced file can be
/// fetched and injected (the sealed guest can't fetch siblings at its opaque
/// origin).
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn find_relative_imports(src: &str) -> Vec<String> {
    let mut out = Vec::new();
    // Match `"./<name>"` and `'./<name>'` occurrences anywhere — covers
    // `from"./x.js"`, `import("./x.js")`, and the language-pack URL template.
    for quote in ['"', '\''] {
        let needle = format!("{quote}./");
        let mut rest = src;
        while let Some(i) = rest.find(&needle) {
            let after = &rest[i + needle.len()..];
            if let Some(end) = after.find(quote) {
                let name = &after[..end];
                // Skip the language-pack template literal (`./tonk-code-lang-…`
                // contains a `${…}` placeholder, handled separately).
                if !name.contains("${") && !out.contains(&name.to_owned()) {
                    out.push(name.to_owned());
                }
                rest = &after[end + 1..];
            } else {
                break;
            }
        }
    }
    out
}

/// Fetch the `<tonk-code>` editor bundle graph from `/tonk-code/` for guest
/// injection: the main element bundle, the dialog-yaml language pack, and every
/// `chunk-*.js` either transitively imports.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn fetch_tonk_code_bundles() -> Vec<(String, String)> {
    // Entry points with stable (unhashed) names served at `/tonk-code/`.
    fetch_bundle_graph(
        "/tonk-code",
        &["tonk-code.js", "tonk-code-lang-dialog-yaml.js"],
    )
    .await
}

/// Fetch ONLY the `<tonk-prose>` registration shell for the guest boot
/// payload. Deliberately not `fetch_bundle_graph`: the shell's source
/// mentions `"./tonk-prose-editor.js"` (its default-resolution fallback),
/// and the graph walk would follow it — eagerly shipping the ~400 kB core
/// to every guest, which is exactly what the lazy split avoids.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn fetch_tonk_prose_shell() -> Vec<(String, String)> {
    match fetch_text("/tonk-prose/tonk-prose.js").await {
        Ok(src) => vec![("tonk-prose.js".to_owned(), src)],
        Err(e) => {
            web_sys::console::warn_1(&JsValue::from_str(&format!(
                "/tonk-prose inject: skipping tonk-prose.js: {e}"
            )));
            Vec::new()
        }
    }
}

/// Fetch the `<tonk-prose>` editor-core graph (the core chunk plus anything
/// it transitively imports) for the on-demand `need-prose` reply.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn fetch_tonk_prose_core() -> Vec<(String, String)> {
    fetch_bundle_graph("/tonk-prose", &["tonk-prose-editor.js"]).await
}

/// Reply to a guest's `need-prose` request: fetch the editor-core graph
/// (the parent is trusted + networked; the sealed guest can't fetch) and
/// post it back as an `inject-prose` envelope on the guest's window. Called
/// from the page-level message listener when the first `<tonk-prose>` in
/// that guest connects. Best-effort like the boot inject — an empty graph
/// makes the guest's promise reject and the element render empty rather
/// than wedging the runtime.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn inject_prose_core(iframe: &HtmlIFrameElement) {
    let Some(content_window) = iframe.content_window() else {
        return;
    };
    spawn_local(async move {
        let prose = bundle_graph_entries(fetch_tonk_prose_core().await);
        let payload = Object::new();
        let _ = Reflect::set(&payload, &"__tonkRuntime".into(), &"inject-prose".into());
        let _ = Reflect::set(&payload, &"prose".into(), &prose);
        let _ = content_window.post_message(&payload, "*");
    });
}

/// Fetch ONLY the `<tonk-table>` registration shell for the guest boot
/// payload. Deliberately not `fetch_bundle_graph`: the shell's source
/// mentions `"./tonk-table-grid.js"` (its default-resolution fallback),
/// and the graph walk would follow it — eagerly shipping the grid and
/// the multi-megabyte engine-bytes leaf to every guest, which is
/// exactly what the lazy split avoids.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn fetch_tonk_table_shell() -> Vec<(String, String)> {
    match fetch_text("/tonk-table/tonk-table.js").await {
        Ok(src) => vec![("tonk-table.js".to_owned(), src)],
        Err(e) => {
            web_sys::console::warn_1(&JsValue::from_str(&format!(
                "/tonk-table inject: skipping tonk-table.js: {e}"
            )));
            Vec::new()
        }
    }
}

/// Fetch the `<tonk-table>` grid core for the on-demand `need-table`
/// reply: the grid chunk plus the engine-bytes leaf, BY NAME rather
/// than via `fetch_bundle_graph`. The grid chunk embeds the wasm
/// IMPORT-OBJECT key `"./wasm_bg.js"` — a string the engine wasm names
/// its import module by, not a real file — and the graph walk would
/// chase it into the SPA's HTML fallback, after which the guest-side
/// blob rewrite would corrupt the key and `WebAssembly.instantiate`
/// would reject the engine. The build pins the file set (three fixed
/// entries, no code splitting), so the explicit list is an invariant,
/// not a guess.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn fetch_tonk_table_core() -> Vec<(String, String)> {
    fetch_bundle_files(
        "/tonk-table",
        &["tonk-table-grid.js", "tonk-table-engine.js"],
    )
    .await
}

/// Fetch an explicit list of bundle files (no graph walk) from `base`
/// for guest injection. Best-effort like `fetch_bundle_graph` — a
/// missing file is skipped, so the feature degrades rather than
/// failing the whole inject.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn fetch_bundle_files(base: &str, names: &[&str]) -> Vec<(String, String)> {
    let mut files: Vec<(String, String)> = Vec::new();
    for name in names {
        match fetch_text(&format!("{base}/{name}")).await {
            Ok(src) => files.push(((*name).to_owned(), src)),
            Err(e) => {
                web_sys::console::warn_1(&JsValue::from_str(&format!(
                    "{base} inject: skipping {name}: {e}"
                )));
            }
        }
    }
    files
}

/// Reply to a guest's `need-table` request: fetch the grid core (the
/// parent is trusted + networked; the sealed guest can't fetch) and
/// post it back as an `inject-table` envelope on the guest's window.
/// Called from the page-level message listener when the first
/// `<tonk-table>` in that guest connects. Best-effort like the boot
/// inject — an empty graph makes the guest's promise reject and the
/// element render empty rather than wedging the runtime.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn inject_table_core(iframe: &HtmlIFrameElement) {
    let Some(content_window) = iframe.content_window() else {
        return;
    };
    spawn_local(async move {
        let table = bundle_graph_entries(fetch_tonk_table_core().await);
        let payload = Object::new();
        let _ = Reflect::set(&payload, &"__tonkRuntime".into(), &"inject-table".into());
        let _ = Reflect::set(&payload, &"table".into(), &table);
        let _ = content_window.post_message(&payload, "*");
    });
}

/// Fetch a code-split editor bundle graph (`entries` + every `./…` chunk they
/// transitively import) from `base` for guest injection. Returns
/// `(name, src)` pairs the guest blobs + import-rewrites. Best-effort — a
/// missing file is skipped, so the editor degrades rather than failing the
/// whole inject.
///
/// These bundles are code-split (esbuild `splitting:true`, required for a
/// single module identity per shared dependency), so they can't be one
/// self-contained ESM like the WA bundle; instead the guest mints a blob per
/// file and rewrites relative imports to those blobs.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn fetch_bundle_graph(base: &str, entries: &[&str]) -> Vec<(String, String)> {
    let mut files: Vec<(String, String)> = Vec::new();
    let mut queue: Vec<String> = entries.iter().map(|s| s.to_string()).collect();

    while let Some(name) = queue.pop() {
        if files.iter().any(|(n, _)| n == &name) {
            continue;
        }
        // Cache-first, like every other guest-boot asset. This used to
        // `reload` (force a network fetch, bypassing the cache) because the
        // entry points have STABLE names and a rebuilt editor must reach
        // the guest. But forcing the network made a cached load on a slow
        // connection pay the full download every time — the tonk-code graph
        // is ~3 MB, so a 3G reload took seconds to fetch bytes it already had
        // cached, while an offline reload (which can't reach the network) was
        // instant. The SW's stale-while-revalidate serves the cached copy
        // immediately and refreshes in the background, so a content change
        // reaches the guest on the NEXT load — acceptable: the chunks are
        // content-hashed (immutable), only the entry points can change,
        // and dev hot-reload already does a full page reload on a real code
        // change.
        let src = match fetch_text(&format!("{base}/{name}")).await {
            Ok(src) => src,
            Err(e) => {
                web_sys::console::warn_1(&JsValue::from_str(&format!(
                    "{base} inject: skipping {name}: {e}"
                )));
                continue;
            }
        };
        // Enqueue every chunk this file imports (e.g. a language pack also
        // pulls shared chunks).
        for spec in find_relative_imports(&src) {
            if !files.iter().any(|(n, _)| n == &spec) && !queue.contains(&spec) {
                queue.push(spec);
            }
        }
        files.push((name, src));
    }
    files
}

/// Package `(name, src)` bundle files as a JS array of `{name, src}` objects
/// for the inject payload.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn bundle_graph_entries(files: Vec<(String, String)>) -> js_sys::Array {
    let array = js_sys::Array::new();
    for (name, src) in files {
        let entry = Object::new();
        let _ = Reflect::set(&entry, &"name".into(), &JsValue::from_str(&name));
        let _ = Reflect::set(&entry, &"src".into(), &JsValue::from_str(&src));
        array.push(&entry);
    }
    array
}

/// The app stylesheet CSS to inject into a guest, read from the document that is
/// bringing the guest up.
///
/// Two cases, because a guest can nest:
/// - **Top document**: it links the app CSS as `<link rel=stylesheet
///   href=/styles-*.css>`; fetch that href's content.
/// - **A guest bringing up a NESTED guest**: it has NO such `<link>` — its own
///   app CSS was injected as an inline `<style data-tonk-app-css>` (it was itself
///   a guest). Read that style's text content directly, so the app CSS
///   propagates down every nesting level instead of stopping at level one.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn app_stylesheet_css() -> Option<String> {
    let document = window()?.document()?;

    // Top document: a `<link rel=stylesheet href=/styles-*.css>`.
    if let Ok(links) = document.query_selector_all("link[rel=stylesheet]") {
        for i in 0..links.length() {
            let Some(node) = links.item(i) else { continue };
            let Ok(el) = node.dyn_into::<Element>() else {
                continue;
            };
            if let Some(href) = el.get_attribute("href")
                && (href.contains("/styles-") || href.ends_with("styles.css"))
            {
                return fetch_text(&href).await.ok();
            }
        }
    }

    // A guest bringing up a nested guest: its injected app CSS is inline.
    if let Ok(Some(style)) = document.query_selector("style[data-tonk-app-css]") {
        return style.text_content().filter(|c| !c.is_empty());
    }

    None
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn fetch_text(url: &str) -> Result<String, String> {
    resp_text(fetch(url).await?).await
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn resp_text(resp: web_sys::Response) -> Result<String, String> {
    let text =
        wasm_bindgen_futures::JsFuture::from(resp.text().map_err(|e| format!("text(): {e:?}"))?)
            .await
            .map_err(|e| format!("await text: {e:?}"))?;
    text.as_string().ok_or_else(|| "text not a string".into())
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn fetch_array_buffer(url: &str) -> Result<JsValue, String> {
    let resp = fetch(url).await?;
    // A missing hashed asset 200s with the SPA fallback (`text/html`). Reject
    // that here so HTML bytes never reach `WebAssembly.instantiate` as a
    // bogus magic word — surface a clear error instead.
    if let Some(ct) = resp.headers().get("content-type").ok().flatten()
        && ct.contains("text/html")
    {
        return Err(format!("fetch {url}: got HTML (asset missing?)"));
    }
    wasm_bindgen_futures::JsFuture::from(
        resp.array_buffer()
            .map_err(|e| format!("array_buffer(): {e:?}"))?,
    )
    .await
    .map_err(|e| format!("await array_buffer: {e:?}"))
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn fetch(url: &str) -> Result<web_sys::Response, String> {
    let win = window().ok_or("no window")?;
    // Default cache mode (no override): these URLs are content-hashed (named
    // by the guest manifest), so the SW's stale-while-revalidate shell cache
    // can hold them immutably — a sealed `/space` works OFFLINE (populated on
    // the first online load, served from cache after) and a content change
    // is a NEW URL (cache miss → fresh), never a stale hit. The manifest rides
    // the same SWR cache so an offline guest can still resolve its assets.
    let resp_value = wasm_bindgen_futures::JsFuture::from(win.fetch_with_str(url))
        .await
        .map_err(|e| format!("fetch {url}: {e:?}"))?;
    resp_value
        .dyn_into::<web_sys::Response>()
        .map_err(|_| format!("fetch {url}: not a Response"))
}

// --- Page-level `hello` listener + registry -----------------------

struct PortalEntry {
    iframe: HtmlIFrameElement,
    host: Element,
    state: Rc<RefCell<PortalState>>,
}

thread_local! {
    static REGISTRY: Rc<RefCell<Vec<PortalEntry>>> = Rc::new(RefCell::new(Vec::new()));
    static LISTENER_INSTALLED: RefCell<bool> = const { RefCell::new(false) };
}

/// Install the single page-level `message` listener that completes the
/// handshake for every portal. Idempotent.
pub(crate) fn install_message_listener() {
    let already = LISTENER_INSTALLED.with(|c| {
        let was = *c.borrow();
        *c.borrow_mut() = true;
        was
    });
    if already {
        return;
    }
    let Some(win) = window() else {
        return;
    };
    let registry = REGISTRY.with(|r| r.clone());
    let listener: Closure<dyn FnMut(MessageEvent)> =
        Closure::wrap(Box::new(move |event: MessageEvent| {
            let data = event.data();

            // Runtime-injection handshake: the guest's runtime bootstrap
            // asks for the element runtime; match its source iframe and
            // fetch+post the bundle. Distinct from the `hello`/data-port
            // handshake below.
            let runtime_kind = get_str(&data, "__tonkRuntime");
            if let Some(kind) = runtime_kind.as_deref() {
                let source = Reflect::get(&event, &"source".into()).unwrap_or(JsValue::NULL);
                match kind {
                    "runtime-ready" => {
                        let matched = registry.borrow().iter().find_map(|entry| {
                            let cw: JsValue = entry.iframe.content_window()?.into();
                            (cw == source).then(|| entry.iframe.clone())
                        });
                        #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
                        if let Some(iframe) = matched {
                            inject_runtime(&iframe);
                        }
                    }
                    // Lazy `<tonk-prose>` editor core: the boot payload only
                    // carries the registration shell; the guest asks for the
                    // core when the first element connects.
                    "need-prose" => {
                        let matched = registry.borrow().iter().find_map(|entry| {
                            let cw: JsValue = entry.iframe.content_window()?.into();
                            (cw == source).then(|| entry.iframe.clone())
                        });
                        #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
                        if let Some(iframe) = matched {
                            inject_prose_core(&iframe);
                        }
                    }
                    // Lazy `<tonk-table>` grid core (grid + engine bytes):
                    // same contract as `need-prose` above.
                    "need-table" => {
                        let matched = registry.borrow().iter().find_map(|entry| {
                            let cw: JsValue = entry.iframe.content_window()?.into();
                            (cw == source).then(|| entry.iframe.clone())
                        });
                        #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
                        if let Some(iframe) = matched {
                            inject_table_core(&iframe);
                        }
                    }
                    "error" => {
                        tonk_common::log!(
                            "portal guest runtime error: {}",
                            get_str(&data, "error").unwrap_or_default()
                        );
                    }
                    "warn" => {
                        tonk_common::log!(
                            "portal guest runtime warn: {}",
                            get_str(&data, "error").unwrap_or_default()
                        );
                    }
                    _ => {}
                }
                return;
            }

            if get_str(&data, "type").as_deref() != Some("hello") {
                return;
            }
            // Authenticate by source identity: the message must come
            // from one of our iframes' live `contentWindow`.
            let source = Reflect::get(&event, &"source".into()).unwrap_or(JsValue::NULL);
            let port = read_first_port(&event);
            let Some(port) = port else {
                return;
            };
            let matched = registry.borrow().iter().find_map(|entry| {
                let cw: JsValue = entry.iframe.content_window()?.into();
                (cw == source).then(|| (entry.host.clone(), entry.state.clone()))
            });
            if let Some((host, state)) = matched {
                bind_port(&host, &state, port);
            }
        }) as Box<dyn FnMut(MessageEvent)>);
    let _ = win.add_event_listener_with_callback("message", listener.as_ref().unchecked_ref());
    // Lives for the page's lifetime — there is exactly one.
    listener.forget();
}

/// Register `(iframe, host, state)` so the `hello` listener can resolve
/// the portal from the iframe's live `contentWindow`.
pub(crate) fn register_portal(
    iframe: &HtmlIFrameElement,
    host: &Element,
    state: &Rc<RefCell<PortalState>>,
) {
    REGISTRY.with(|r| {
        r.borrow_mut().push(PortalEntry {
            iframe: iframe.clone(),
            host: host.clone(),
            state: state.clone(),
        })
    });
}

/// Drop the registry entry for `iframe` on teardown.
pub(crate) fn unregister_portal(iframe: &HtmlIFrameElement) {
    REGISTRY.with(|r| {
        r.borrow_mut()
            .retain(|e| !e.iframe.is_same_node(Some(iframe.as_ref())))
    });
}

/// Bind a freshly handshaked `port` to `host`/`state`: install the
/// envelope dispatcher, stash the port, and post `ready { context }`.
/// Called from the `hello` listener (and directly from tests, which
/// supply a `MessageChannel` port in place of a real iframe handshake).
pub(crate) fn bind_port(host: &Element, state: &Rc<RefCell<PortalState>>, port: MessagePort) {
    let dispatcher = make_dispatcher(host.clone(), state.clone(), port.clone());
    // Setting onmessage auto-starts the port; no port.start() needed.
    port.set_onmessage(Some(dispatcher.as_ref().unchecked_ref()));

    {
        let mut s = state.borrow_mut();
        s.port = Some(port.clone());
        s._dispatcher = Some(dispatcher);
    }

    let ready = Object::new();
    set_v1(&ready, "ready");
    let _ = Reflect::set(&ready, &"context".into(), &build_context(host, state));
    let _ = port.post_message(&ready);
}

// --- Envelope dispatch (parent side) ------------------------------

fn make_dispatcher(
    host: Element,
    state: Rc<RefCell<PortalState>>,
    port: MessagePort,
) -> Closure<dyn FnMut(MessageEvent)> {
    Closure::wrap(Box::new(move |event: MessageEvent| {
        let data = event.data();
        let Some(kind) = get_str(&data, "type") else {
            return;
        };
        match kind.as_str() {
            "query" => handle_query(&host, &state, &port, &data),
            "transact" => handle_transact(&host, &state, &port, &data),
            "evaluate" => handle_evaluate(&host, &port, &data),
            "subscribe" => handle_subscribe(&host, &state, &port, &data),
            "unsubscribe" => handle_unsubscribe(&state, &data),
            "navigate" => handle_navigate(&state, &data),
            "reload" => tonk_host::reload_page(),
            "title" => handle_title(&data),
            "open" => handle_open(&state, &data),
            "register" => handle_register(&state, &port, &data),
            "fetch" => handle_host_fetch(&state, &port, &data),
            "delegate" => handle_delegate(&port, &data),
            _ => {}
        }
    }) as Box<dyn FnMut(MessageEvent)>)
}

fn handle_query(
    host: &Element,
    state: &Rc<RefCell<PortalState>>,
    port: &MessagePort,
    data: &JsValue,
) {
    let Some(id) = get_str(data, "id") else {
        return;
    };
    let body = match query_body(host, &get_body(data)) {
        Ok(b) => b,
        Err(msg) => return post_error(port, "query-error", &id, &msg),
    };
    let (space, branch, profile) = match forwarded_route(state, data) {
        Ok(route) => route,
        Err(denied) => {
            tonk_common::log!("portal query {}", denied.message());
            return post_error(port, "query-error", &id, &denied.message());
        }
    };
    let host = host.clone();
    let port = port.clone();
    spawn_local(async move {
        match host_consumer::query_with_route(
            &host,
            &body,
            space.as_deref(),
            branch.as_deref(),
            profile,
        )
        .await
        {
            Ok(rows) => post_result(&port, "query-result", &id, "rows", &rows),
            Err(e) => post_error(&port, "query-error", &id, &e.message),
        }
    });
}

/// A forwarded route the portal refuses to relay. `Denied` is
/// deliberately a distinct variant rather than a collapse to `None`: it
/// is the seam for a future capability-request flow, where an un-listed
/// request prompts to extend `allow` rather than simply failing.
#[derive(Debug)]
enum Refused {
    /// The forwarded `with` did not parse.
    Malformed {
        spec: String,
        error: tonk_host::location::ParseError,
    },
    /// The forwarded route parsed but is not in the portal's `allow`.
    Denied { requested: Location },
}

impl Refused {
    fn message(&self) -> String {
        match self {
            Refused::Malformed { spec, error } => {
                format!("malformed forwarded with {spec:?}: {error}")
            }
            Refused::Denied { requested } => {
                format!("denied: route {requested} is not permitted by this site's allow")
            }
        }
    }
}

/// Resolve the route for a relayed guest operation.
///
/// - No forwarded route → the portal's own `with` (the pinned default).
/// - A forwarded route (the guest's resolved `with` context) → honored
///   only if this portal's `allow` permits it; otherwise a typed
///   [`Denied`], which the caller posts back as an error envelope —
///   never a silent coercion to the pinned context.
///
/// The privilege is the trusted portal element's, set host-side
/// (`PortalState::set_route`); a guest can forward a route but cannot
/// grant itself the reach to have it honored.
fn forwarded_route(
    state: &Rc<RefCell<PortalState>>,
    data: &JsValue,
) -> Result<(Option<String>, Option<String>, bool), Refused> {
    let s = state.borrow();

    let Some(spec) = get_str(data, "with").filter(|w| !w.is_empty()) else {
        // No forwarded route: pin to the portal's own context, explicitly —
        // there are no ambient DOM ancestors to fall back on.
        return Ok(match &s.with {
            Some(own) => tonk_host::route_of(own),
            None => (None, None, false),
        });
    };

    let requested: Location = spec
        .parse()
        .map_err(|error| Refused::Malformed { spec, error })?;
    if s.allow.permits(&requested) {
        Ok(tonk_host::route_of(&requested))
    } else {
        Err(Refused::Denied { requested })
    }
}

fn handle_transact(
    host: &Element,
    state: &Rc<RefCell<PortalState>>,
    port: &MessagePort,
    data: &JsValue,
) {
    let Some(id) = get_str(data, "id") else {
        return;
    };
    let request = Reflect::get(data, &"request".into()).unwrap_or(JsValue::UNDEFINED);
    let (space, branch, profile) = match forwarded_route(state, data) {
        Ok(route) => route,
        Err(denied) => {
            tonk_common::log!("portal transact {}", denied.message());
            return post_error(port, "transact-error", &id, &denied.message());
        }
    };
    let host = host.clone();
    let port = port.clone();
    spawn_local(async move {
        match host_consumer::claim_with_route(
            &host,
            &request,
            space.as_deref(),
            branch.as_deref(),
            profile,
        )
        .await
        {
            Ok(receipt) => post_result(&port, "transact-result", &id, "receipt", &receipt),
            Err(e) => post_error(&port, "transact-error", &id, &e.message),
        }
    });
}

/// Relay an `evaluate` envelope to the installed host's consumer path, which
/// performs the typed evaluate (POST `/evaluate?transact=`) and returns the
/// parsed JSON result. The guest's inspector dispatches `tonk-evaluate`; the
/// guest relay forwards here, so the inspector uses the same host consumer API
/// as the in-page editor — no direct HTTP, no hand-rolled response types.
fn handle_evaluate(host: &Element, port: &MessagePort, data: &JsValue) {
    let Some(id) = get_str(data, "id") else {
        return;
    };
    let document = get_str(data, "document").unwrap_or_default();
    // Default to a committing evaluate; only an explicit `false` is a dry run.
    let transact = Reflect::get(data, &"transact".into())
        .ok()
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let host = host.clone();
    let port = port.clone();
    spawn_local(async move {
        match host_consumer::evaluate(&host, &document, transact).await {
            Ok(result) => post_result(&port, "evaluate-result", &id, "result", &result),
            Err(e) => post_error(&port, "evaluate-error", &id, &e.message),
        }
    });
}

fn handle_subscribe(
    host: &Element,
    state: &Rc<RefCell<PortalState>>,
    port: &MessagePort,
    data: &JsValue,
) {
    let Some(id) = get_str(data, "id") else {
        return;
    };
    let body = match query_body(host, &get_body(data)) {
        Ok(b) => b,
        Err(msg) => return post_error(port, "subscribe-error", &id, &msg),
    };
    let (space, branch, profile) = match forwarded_route(state, data) {
        Ok(route) => route,
        Err(denied) => {
            tonk_common::log!("portal subscribe {}", denied.message());
            return post_error(port, "subscribe-error", &id, &denied.message());
        }
    };

    let tag = {
        let mut s = state.borrow_mut();
        s.next_tag = s.next_tag.wrapping_add(1);
        format!("portal-sub-{}", s.next_tag)
    };
    let tag_js = JsValue::from_str(&tag);
    match host_consumer::subscribe_with_route(
        host,
        &body,
        Some(&tag_js),
        space.as_deref(),
        branch.as_deref(),
        profile,
    ) {
        Ok(host_sub) => {
            state.borrow_mut().subs.insert(
                tag,
                BridgeSub {
                    iframe_id: id,
                    _host_sub: host_sub,
                },
            );
        }
        // No host ancestor / dispatch failure: surface to the author's
        // stream; nothing is tracked.
        Err(e) => post_error(port, "subscribe-error", &id, &e.message),
    }
}

fn handle_unsubscribe(state: &Rc<RefCell<PortalState>>, data: &JsValue) {
    let Some(id) = get_str(data, "id") else {
        return;
    };
    let mut s = state.borrow_mut();
    let tag = s
        .subs
        .iter()
        .find(|(_, sub)| sub.iframe_id == id)
        .map(|(tag, _)| tag.clone());
    if let Some(tag) = tag {
        // Dropping the `BridgeSub` cancels its host subscription.
        s.subs.remove(&tag);
    }
}

/// Navigate the host page to `href`. The sealed guest can't touch its
/// parent's location, so a link click inside it posts the href here and the
/// trusted parent performs the navigation — as a client-side route change
/// (`pushState` + `popstate`), never a reload: the top `<tonk-site>` re-routes
/// its path in place and the running guest re-renders via its `tonk:site`
/// subscription.
fn handle_navigate(state: &Rc<RefCell<PortalState>>, data: &JsValue) {
    let Some(href) = get_str(data, "href").filter(|h| !h.is_empty()) else {
        return;
    };
    tonk_host::navigate_to(&real_href(state, &href));
}

/// Translate a guest-world href into the REAL route the host navigates to.
///
/// The guest resolves links against its synthetic per-space origin
/// (`https://{label}.tonk.network/`), so an in-space link arrives as a bare
/// absolute path (`/activity`). The document is really served at
/// `/space/{did}/...`, so prefix the space segment. A guest with no space
/// context (profile/Hub), or an already-`/space/...` path, is left as-is.
fn real_href(state: &Rc<RefCell<PortalState>>, href: &str) -> String {
    let Some(space) = state.borrow().route_space() else {
        return href.to_owned();
    };
    // Root of the space ("/") maps to the space's own route.
    if href == "/" {
        return format!("/space/{space}");
    }
    // A leading-slash in-space path; anything else (already absolute host
    // path, or a fragment/query) is passed through untouched.
    if let Some(rest) = href.strip_prefix('/') {
        if rest.starts_with("space/") || is_top_level_route(rest) {
            href.to_owned()
        } else {
            format!("/space/{space}/{rest}")
        }
    } else {
        href.to_owned()
    }
}

/// Routes that belong to the PROFILE, not to any space.
///
/// The guest resolves every link against its synthetic per-space origin, so a
/// link to one of these arrives looking exactly like an in-space path and would
/// be rewritten to `/space/{did}/join` — a route no space defines. The page then
/// tries to boot the whole app inside the sealed frame, where the origin is
/// opaque: no service worker, every asset CORS-blocked, and the renderer dies.
///
/// These names are the profile's own route table (`profile.yaml`), which no
/// space route shadows, so passing them through is unambiguous.
fn is_top_level_route(rest: &str) -> bool {
    let head = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    matches!(head, "join" | "account" | "inspector" | "diagnose")
}

/// Set the host page's tab title on the guest's behalf. The guest's
/// `<tonk-title>` posts `{v:1, type:"title", text}`; this runs in the
/// parent document, which is where `document.title` lives.
/// Raise the host's registration dialog for a share that needs an
/// account.
///
/// The dialog itself lives in `tonk-ui`, which depends on this crate, so
/// it cannot be called by name from here. The top page registers a
/// handler at boot instead — the same shape as the other page effects,
/// where this crate carries the transport and the shell supplies the
/// behaviour.
fn handle_register(state: &Rc<RefCell<PortalState>>, port: &MessagePort, data: &JsValue) {
    let Some((reason, token)) = register_request(data) else {
        return;
    };
    let focus_return = token.map(|token| RegisterFocusReturn {
        port: port.clone(),
        frame: state.borrow().iframe.clone(),
        token,
        handled: false,
    });
    REGISTER_HANDLER.with(|handler| {
        if let Some(handler) = handler.borrow().as_ref() {
            handler(&reason, focus_return);
        }
    });
}

/// A one-shot return path to the exact control in a sealed guest that asked
/// the top page to open registration.
pub struct RegisterFocusReturn {
    port: MessagePort,
    frame: Option<HtmlIFrameElement>,
    token: String,
    handled: bool,
}

impl RegisterFocusReturn {
    /// Return focus to the still-connected guest opener and consume its token.
    pub fn restore(mut self) {
        if let Some(frame) = self.frame.as_ref()
            && frame.is_connected()
        {
            let _ = frame.focus();
        }
        self.post("register-focus");
        self.handled = true;
    }

    fn post(&self, kind: &str) {
        let envelope = Object::new();
        set_v1(&envelope, kind);
        let _ = Reflect::set(
            &envelope,
            &"focusToken".into(),
            &JsValue::from_str(&self.token),
        );
        let _ = self.port.post_message(&envelope);
    }
}

impl Drop for RegisterFocusReturn {
    fn drop(&mut self) {
        if !self.handled {
            self.post("register-focus-discard");
        }
    }
}

/// What a page does when a guest asks it to raise registration.
type RegisterHandler = Box<dyn Fn(&str, Option<RegisterFocusReturn>)>;

thread_local! {
    /// What to do when a guest asks for registration. `None` until the
    /// shell installs one, which is correct for a page with no account
    /// UI: the ask is dropped rather than half-performed.
    static REGISTER_HANDLER: std::cell::RefCell<Option<RegisterHandler>> =
        const { std::cell::RefCell::new(None) };
}

/// Install what runs when a guest asks the host to register an account.
///
/// Called once by the shell at boot. Later calls replace the handler,
/// which keeps a hot reload from stacking dialogs.
pub fn on_register(handler: impl Fn(&str, Option<RegisterFocusReturn>) + 'static) {
    REGISTER_HANDLER.with(|slot| {
        *slot.borrow_mut() = Some(Box::new(handler));
    });
}

/// Read `reason` out of a `{ type: "register", reason }` message, or
/// `None` when the message is not one. Split out so the parse is
/// testable on its own, the way [`title_text`] is.
fn register_request(data: &JsValue) -> Option<(String, Option<String>)> {
    if get_str(data, "type")? != "register" {
        return None;
    }
    let reason = get_str(data, "reason").filter(|reason| !reason.is_empty())?;
    let token = get_str(data, "focusToken").filter(|token| !token.is_empty());
    Some((reason, token))
}

fn handle_title(data: &JsValue) {
    let Some(text) = title_text(data) else {
        return;
    };
    tonk_host::set_title(&text);
}

/// Read `text` out of a `{ type: "title", text }` message, or `None` when
/// the message isn't a title or carries no usable text. The dispatcher
/// has already matched on `type`; re-checking it here keeps the parse
/// independently testable, as `navigate_href` does in `tonk-host`.
fn title_text(data: &JsValue) -> Option<String> {
    if get_str(data, "type")? != "title" {
        return None;
    }
    get_str(data, "text").filter(|text| !text.is_empty())
}

/// Open a link on the guest's behalf. The sealed guest has no `allow-popups`
/// and no `allow-top-navigation`, so it cannot open anything itself; it posts
/// the raw href and `tonk_host::open_external` — running on the page, which is
/// the only place that can both resolve and open it — decides what happens.
/// Mint a delegation under the passkey on the guest's behalf.
///
/// The guest asks `{ subject, command, audience }`; the account root that
/// signs it lives behind the passkey, which exists only on this top-level
/// window and only inside a user gesture. The guest's click propagates its
/// activation to this frame, so the ceremony runs here immediately and the
/// prompt is the user's own gesture. The hop minted is `root -> audience`
/// over `subject` at `command`; the guest carries it to the worker, which
/// checks it against what it composes it with. Answered with
/// `delegate-result` carrying the base58 chain, or `delegate-error`.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn handle_delegate(port: &MessagePort, data: &JsValue) {
    let Some(id) = get_str(data, "id") else {
        return;
    };
    let request = (
        get_str(data, "subject").unwrap_or_default(),
        get_str(data, "command").unwrap_or_default(),
        get_str(data, "audience").unwrap_or_default(),
    );
    let port = port.clone();
    wasm_bindgen_futures::spawn_local(async move {
        match mint_delegation(&request.0, &request.1, &request.2).await {
            Ok(encoded) => post_result(
                &port,
                "delegate-result",
                &id,
                "delegation",
                &JsValue::from_str(&encoded),
            ),
            Err(error) => post_error(&port, "delegate-error", &id, &format!("{error:#}")),
        }
    });
}

/// Run the passkey ceremony and mint `root -> audience` over `subject` at
/// `command`, returning the serialized chain as base58.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn mint_delegation(subject: &str, command: &str, audience: &str) -> anyhow::Result<String> {
    use dialog_ucan_core::command::Command;
    use dialog_ucan_core::subject::Subject as UcanSubject;
    use dialog_ucan_core::{DelegationBuilder, DelegationChain};
    use dialog_varsig::Did;

    let subject: Did = subject
        .parse()
        .map_err(|error| anyhow::anyhow!("the subject is not a DID: {error:?}"))?;
    let audience: Did = audience
        .parse()
        .map_err(|error| anyhow::anyhow!("the audience is not a DID: {error:?}"))?;
    let command = Command::parse(command)
        .map_err(|error| anyhow::anyhow!("the command does not parse: {error}"))?;
    // The custody endpoint the page's other ceremonies use: the account
    // service is served under `/ucan/` on the page's own origin.
    let origin = web_sys::window()
        .and_then(|window| window.location().origin().ok())
        .ok_or_else(|| anyhow::anyhow!("window origin is unavailable"))?;
    let endpoint = format!("{}/ucan/", origin.trim_end_matches('/'));
    let root = tonk_identity::ceremony::unlock_root(&endpoint).await?;
    let delegation = DelegationBuilder::new()
        .issuer(dialog_credentials::Signer::from(root))
        .audience(&audience)
        .subject(UcanSubject::Specific(subject))
        .command(command.segments().clone())
        .try_build()
        .await
        .map_err(|error| anyhow::anyhow!("failed to mint the delegation: {error}"))?;
    let bytes = DelegationChain::new(delegation).to_bytes()?;
    Ok(bs58::encode(bytes).into_string())
}

fn handle_open(state: &Rc<RefCell<PortalState>>, data: &JsValue) {
    let Some(href) = open_href(data) else {
        return;
    };
    // `open` is for hrefs that escaped the guest's synthetic origin, so the
    // href is normally a full external URL and passes through. Defensively map
    // a bare in-space path too (`real_href` no-ops on external URLs, which
    // don't start with a single `/`).
    tonk_host::open_external(&real_href(state, &href));
}

/// Read `href` out of an `{ type: "open", href }` message, or `None` when the
/// message isn't an open or carries no usable href. Mirrors `title_text`.
fn open_href(data: &JsValue) -> Option<String> {
    if get_str(data, "type")? != "open" {
        return None;
    }
    get_str(data, "href").filter(|href| !href.is_empty())
}

/// Perform a same-origin fetch on the host and stream the response back. The
/// opaque guest can't reach a same-origin, SW-routed `/api/...` endpoint
/// itself, so it asks the host (which IS same-origin). Restricted to
/// host-relative paths (`/…`, not `//`) so the guest can't drive the host to
/// fetch arbitrary cross-origin URLs.
///
/// The host does its own `fetch`, then posts a `fetch-result` envelope back
/// over the port carrying the status, status text, headers, and the response
/// body's `ReadableStream` — TRANSFERRED (not copied) so the bytes never
/// round-trip through wasm. The guest rebuilds a real streaming `Response`
/// from those, so its overridden `window.fetch` is faithful (`.text()`,
/// `.blob()`, `.arrayBuffer()`, `.body` all work) and binary-safe.
///
/// Branch data-plane paths are gated by this portal's `with`/`allow`
/// before the fetch runs — with the guest's IO riding plain `fetch`,
/// this relay IS the reach chokepoint, for the elements' requests and
/// for raw guest `fetch()` calls alike.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn handle_host_fetch(state: &Rc<RefCell<PortalState>>, port: &MessagePort, data: &JsValue) {
    let Some(id) = get_str(data, "id") else {
        return;
    };
    let Some(path) = get_str(data, "path") else {
        return post_error(port, "fetch-error", &id, "missing path");
    };
    if !path.starts_with('/') || path.starts_with("//") {
        return post_error(port, "fetch-error", &id, "path must be host-relative");
    };
    if let Some(requested) = data_plane_location(&path, state) {
        let s = state.borrow();
        let permitted = s
            .with
            .as_ref()
            .is_some_and(|own| own.same_reach(&requested))
            || s.allow.permits(&requested);
        if !permitted {
            let denied = Refused::Denied { requested };
            tonk_common::log!("portal fetch {}", denied.message());
            return post_error(port, "fetch-error", &id, &denied.message());
        }
    }
    // The guest forwards the full request so POST query/subscribe/transact work,
    // not just GET. Build the `RequestInit` (method, headers, body) and fetch the
    // bare relative path as a STRING — never a `Request`, which would resolve the
    // path against this document's baseURI. When the host is itself a sealed guest
    // (a NESTED portal), that baseURI is the real origin, so a `Request` would
    // make the path a cross-origin absolute URL its OWN `window.fetch` override
    // can't relay (origin `null` → CORS). The string path lets each level's
    // override catch the host-relative `/…` and relay up to its parent.
    let init = match build_relayed_request(data) {
        Ok(init) => init,
        Err(e) => return post_error(port, "fetch-error", &id, &e),
    };
    // Every relay is abortable and tracked on the portal: teardown aborts
    // the lot, so a torn-down guest's streams (transferred response bodies
    // included) are cancelled instead of piping into a destroyed realm.
    if let Ok(controller) = AbortController::new() {
        init.set_signal(Some(&controller.signal()));
        state.borrow_mut().track_relay(controller);
    }
    let port = port.clone();
    spawn_local(async move {
        match fetch_path(&path, &init).await {
            Ok(resp) => post_fetch_response(&port, &id, &resp).await,
            Err(e) => post_error(&port, "fetch-error", &id, &e),
        }
    });
}

/// The repository reach a relayed path targets, if any:
/// `/api/repository/{repo}`, `/api/profile/repository`,
/// `/api/repository/{repo}/branch/{branch}/…`, or
/// `/api/profile/branch/{branch}/…`. Non-data-plane paths (assets, the
/// guest bundle, `/api/sync`, and repository control routes) return `None`.
///
/// The profile endpoint is singular and its URL carries no name, so a
/// profile path canonicalizes to the portal's own profile name when the
/// portal is profile-pinned, else the worker's default (`tonk`).
fn data_plane_location(path: &str, state: &Rc<RefCell<PortalState>>) -> Option<Location> {
    use tonk_host::location::Repo;
    let path = path.split_once('?').map_or(path, |(path, _)| path);
    if path == "/api/profile/repository" {
        let name = state
            .borrow()
            .with
            .as_ref()
            .and_then(|own| match &own.repo {
                Repo::Profile(name) => Some(name.clone()),
                Repo::Named(_) => None,
            })
            .unwrap_or_else(|| "tonk".to_owned());
        return Some(Location {
            repo: Repo::Profile(name),
            branch: Some("main".to_owned()),
        });
    }
    if let Some(rest) = path.strip_prefix("/api/repository/") {
        let mut segments = rest.split('/');
        let repo = segments.next().filter(|s| !s.is_empty())?;
        match segments.next() {
            None => {
                return Some(Location {
                    repo: Repo::Named(repo.to_owned()),
                    branch: Some("main".to_owned()),
                });
            }
            Some("branch") => {}
            _ => return None,
        }
        let branch = segments.next().filter(|s| !s.is_empty())?;
        return Some(Location {
            repo: Repo::Named(repo.to_owned()),
            branch: Some(branch.to_owned()),
        });
    }
    if let Some(rest) = path.strip_prefix("/api/profile/branch/") {
        let branch = rest.split('/').next().filter(|s| !s.is_empty())?;
        let name = state
            .borrow()
            .with
            .as_ref()
            .and_then(|own| match &own.repo {
                Repo::Profile(name) => Some(name.clone()),
                Repo::Named(_) => None,
            })
            .unwrap_or_else(|| "tonk".to_owned());
        return Some(Location {
            repo: Repo::Profile(name),
            branch: Some(branch.to_owned()),
        });
    }
    None
}

/// Build a `Request` for a relayed guest fetch from the envelope's
/// `method`/`headers`/`body`. `headers` is an array of `[name, value]` pairs;
/// `body` is a string (our `/api` bodies are JSON) or absent.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn build_relayed_request(data: &JsValue) -> Result<web_sys::RequestInit, String> {
    let init = web_sys::RequestInit::new();
    let method = get_str(data, "method").unwrap_or_else(|| "GET".to_owned());
    init.set_method(&method);

    let headers = web_sys::Headers::new().map_err(|e| format!("Headers: {e:?}"))?;
    if let Ok(pairs) = Reflect::get(data, &"headers".into())
        && let Ok(pairs) = pairs.dyn_into::<js_sys::Array>()
    {
        for pair in pairs.iter() {
            let pair: js_sys::Array = match pair.dyn_into() {
                Ok(p) => p,
                Err(_) => continue,
            };
            if let (Some(name), Some(value)) = (pair.get(0).as_string(), pair.get(1).as_string()) {
                let _ = headers.append(&name, &value);
            }
        }
    }
    init.set_headers(&headers);

    // Body — only for methods that carry one. A bodyless GET/HEAD with a body
    // set throws, so only attach when present and non-null.
    let body = Reflect::get(data, &"body".into()).unwrap_or(JsValue::UNDEFINED);
    if !body.is_undefined() && !body.is_null() {
        init.set_body(&body);
    }

    Ok(init)
}

/// Perform a host-side `fetch(path, init)` and return the `Response`. The path is
/// passed as a STRING (not a `Request`) so a nested-guest host's overridden
/// `window.fetch` catches the host-relative `/…` and relays it up — see
/// [`handle_host_fetch`].
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn fetch_path(path: &str, init: &web_sys::RequestInit) -> Result<web_sys::Response, String> {
    let win = window().ok_or("no window")?;
    let resp_value = wasm_bindgen_futures::JsFuture::from(win.fetch_with_str_and_init(path, init))
        .await
        .map_err(|e| format!("fetch: {e:?}"))?;
    resp_value
        .dyn_into::<web_sys::Response>()
        .map_err(|_| "fetch: not a Response".to_string())
}

/// Post a `fetch-result` envelope carrying the response status + headers and
/// the body, streamed to the guest.
///
/// Body delivery has two paths, chosen by whether the browser can transfer a
/// `ReadableStream` over `postMessage`:
///
/// - **Fast path** (Chrome, Firefox, Safari 27+): transfer `response.body`
///   itself — one transfer, native streaming, zero plumbing.
/// - **Fallback** (Safari before 27, which throws `DataCloneError` on a
///   stream transfer): transfer one end of a fresh `MessageChannel` and drain
///   the body into it as chunks, with credit-based backpressure (see
///   [`drain_body_to_port`]). The guest rebuilds a `ReadableStream` fed by
///   that port.
///
/// Either way the guest gets a real streaming `Response`. A bodyless response
/// (e.g. 204) sends neither.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn post_fetch_response(port: &MessagePort, id: &str, resp: &web_sys::Response) {
    let env = Object::new();
    set_v1(&env, "fetch-result");
    let _ = Reflect::set(&env, &"id".into(), &JsValue::from_str(id));
    let _ = Reflect::set(
        &env,
        &"status".into(),
        &JsValue::from_f64(resp.status() as f64),
    );
    let _ = Reflect::set(
        &env,
        &"statusText".into(),
        &JsValue::from_str(&resp.status_text()),
    );
    // The final URL the host fetched (post-redirect). The guest can't
    // recover it — `new Response(...)` leaves `url` as `""` and the
    // property is readonly — so it travels on the envelope and the guest
    // shadows the getter with it. Without it, a guest consumer that parses
    // `response.url` fails on every relayed fetch.
    let _ = Reflect::set(&env, &"url".into(), &JsValue::from_str(&resp.url()));
    // Headers as an array of [name, value] pairs — structured-clonable and
    // re-hydrated into a `Headers` on the guest side.
    let _ = Reflect::set(&env, &"headers".into(), &headers_to_array(&resp.headers()));

    let Some(body) = resp.body() else {
        // Bodyless response — send the head with no body.
        let _ = port.post_message(&env);
        return;
    };

    // Fast path: attempt to transfer the stream itself. We only learn whether
    // the browser supports it by trying — a probe post on a throwaway channel,
    // so a `DataCloneError` here never reaches the guest.
    if streams_are_transferable() {
        let transfer = js_sys::Array::new();
        let _ = Reflect::set(&env, &"body".into(), &body);
        transfer.push(&body);
        match port.post_message_with_transferable(&env, &transfer) {
            Ok(()) => return,
            // Shouldn't happen once the probe passed, but if it does, fall
            // through to the chunked path rather than dropping the response.
            Err(e) => {
                tonk_common::log!("portal fetch: stream transfer failed post-probe: {e:?}");
            }
        }
    }

    // Fallback: drain the body into a MessageChannel with credit-based
    // backpressure. Strip the (untransferable) stream off the head envelope
    // and hand the guest a port instead.
    let _ = Reflect::delete_property(&env, &"body".into());
    drain_body_to_port(port, env, &body);
}

/// Whether this browser can transfer a `ReadableStream` over `postMessage`.
/// Detected once by probing a throwaway `MessageChannel` (the result is
/// cached): Safari before 27 throws `DataCloneError`, every other current
/// browser succeeds.
///
/// Transfers were briefly disabled while chasing a browser-process crash;
/// the real trigger was the SYNCHRONOUS destruction of a live nested guest
/// (now a two-phase teardown: unload to `about:blank`, remove a tick
/// later, with every relay aborted and every port closed first). With that
/// fixed, the zero-copy transfer path is back.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn streams_are_transferable() -> bool {
    thread_local! {
        static SUPPORTED: std::cell::OnceCell<bool> = const { std::cell::OnceCell::new() };
    }
    SUPPORTED.with(|cell| {
        *cell.get_or_init(|| {
            let Ok(channel) = web_sys::MessageChannel::new() else {
                return false;
            };
            let stream = web_sys::ReadableStream::new().unwrap_or_else(|_| JsValue::NULL.into());
            let transfer = js_sys::Array::new();
            transfer.push(&stream);
            channel
                .port1()
                .post_message_with_transferable(&JsValue::NULL, &transfer)
                .is_ok()
        })
    })
}

/// Drain `body` into a fresh `MessageChannel`, transferring the guest's end on
/// the `head` envelope (as `streamPort`). Credit-based backpressure: the guest
/// posts `{type:"credit", n}` and the host reads + posts up to `n` more chunks
/// (`{type:"chunk", buffer}` transferred), then `{type:"close"}` on EOF or
/// `{type:"error", error}` on a read failure. A guest `{type:"cancel"}`
/// cancels the reader. Used only when `ReadableStream` transfer is unavailable.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn drain_body_to_port(port: &MessagePort, head: Object, body: &web_sys::ReadableStream) {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    let Ok(channel) = web_sys::MessageChannel::new() else {
        return post_error_obj(port, &head, "host could not open a stream channel");
    };
    let host_port = channel.port1();
    let guest_port = channel.port2();

    // Hand the guest its end on the head envelope (transferred).
    let _ = Reflect::set(&head, &"streamPort".into(), &guest_port);
    let transfer = js_sys::Array::new();
    transfer.push(&guest_port);
    if let Err(e) = port.post_message_with_transferable(&head, &transfer) {
        tonk_common::log!("portal fetch: failed to hand off stream port: {e:?}");
        return;
    }

    let Ok(reader_val) = body
        .get_reader()
        .dyn_into::<web_sys::ReadableStreamDefaultReader>()
    else {
        return;
    };
    let reader = Rc::new(reader_val);
    // Available credit + a "pump in flight" guard so concurrent credit grants
    // don't launch overlapping reader loops (a reader allows one read at a
    // time).
    let credit = Rc::new(Cell::new(0u32));
    let pumping = Rc::new(Cell::new(false));
    let host_port = Rc::new(host_port);
    let cancelled = Rc::new(Cell::new(false));

    // The pump: while there's credit and we're not already reading, read one
    // chunk and post it, decrementing credit. Re-entrant-safe via `pumping`.
    // The pump closure re-invokes itself (to drain remaining credit) and is
    // also invoked by the credit handler, so it lives behind a shared cell.
    type PumpCell = Rc<RefCell<Option<Closure<dyn FnMut()>>>>;
    let pump: PumpCell = Rc::new(RefCell::new(None));
    {
        let reader = reader.clone();
        let credit = credit.clone();
        let pumping = pumping.clone();
        let host_port = host_port.clone();
        let cancelled = cancelled.clone();
        let pump_ref = pump.clone();
        let closure = Closure::wrap(Box::new(move || {
            if pumping.get() || cancelled.get() || credit.get() == 0 {
                return;
            }
            pumping.set(true);
            let reader = reader.clone();
            let credit = credit.clone();
            let pumping = pumping.clone();
            let host_port = host_port.clone();
            let cancelled = cancelled.clone();
            let pump_ref = pump_ref.clone();
            spawn_local(async move {
                let result = wasm_bindgen_futures::JsFuture::from(reader.read()).await;
                pumping.set(false);
                if cancelled.get() {
                    return;
                }
                match result {
                    Ok(chunk) => {
                        let done = Reflect::get(&chunk, &"done".into())
                            .ok()
                            .and_then(|v| v.as_bool())
                            .unwrap_or(true);
                        if done {
                            let env = Object::new();
                            let _ = Reflect::set(&env, &"type".into(), &"close".into());
                            let _ = host_port.post_message(&env);
                            // The stream is over — close the host end NOW.
                            // Every chunk channel is a browser-brokered pair
                            // of ports; leaving them to the GC keeps live
                            // endpoints into guest frames that may be mid-
                            // teardown, which is exactly the churn the
                            // renderer has crashed under.
                            host_port.close();
                            return;
                        }
                        // `value` is a `Uint8Array`, possibly a view windowed
                        // into a larger backing buffer (byteOffset/byteLength).
                        // Transfer the backing buffer (zero-copy — the whole
                        // point of a transfer) and carry the window offsets so
                        // the guest reconstructs a view over exactly this
                        // chunk's bytes, not the sibling bytes that may share
                        // the buffer.
                        let view = js_sys::Uint8Array::new(
                            &Reflect::get(&chunk, &"value".into()).unwrap_or(JsValue::NULL),
                        );
                        let buffer = view.buffer();
                        let env = Object::new();
                        let _ = Reflect::set(&env, &"type".into(), &"chunk".into());
                        let _ = Reflect::set(&env, &"chunk".into(), &buffer);
                        let _ = Reflect::set(
                            &env,
                            &"byteOffset".into(),
                            &JsValue::from_f64(view.byte_offset() as f64),
                        );
                        let _ = Reflect::set(
                            &env,
                            &"byteLength".into(),
                            &JsValue::from_f64(view.byte_length() as f64),
                        );
                        let transfer = js_sys::Array::new();
                        transfer.push(&buffer);
                        let _ = host_port.post_message_with_transferable(&env, &transfer);
                        credit.set(credit.get().saturating_sub(1));
                        // More credit may remain — keep pumping.
                        if let Some(cb) = pump_ref.borrow().as_ref() {
                            let _ =
                                js_sys::Function::from(cb.as_ref().clone()).call0(&JsValue::NULL);
                        }
                    }
                    Err(e) => {
                        let env = Object::new();
                        let _ = Reflect::set(&env, &"type".into(), &"error".into());
                        let _ = Reflect::set(
                            &env,
                            &"error".into(),
                            &JsValue::from_str(&format!("{e:?}")),
                        );
                        let _ = host_port.post_message(&env);
                        // Terminal — free the port pair (see the EOF arm).
                        host_port.close();
                    }
                }
            });
        }) as Box<dyn FnMut()>);
        *pump.borrow_mut() = Some(closure);
    }

    // The host port's message handler: grant credit, or cancel.
    let onmessage = {
        let credit = credit.clone();
        let cancelled = cancelled.clone();
        let reader = reader.clone();
        let pump = pump.clone();
        let host_port = host_port.clone();
        Closure::wrap(Box::new(move |event: MessageEvent| {
            let data = event.data();
            match get_str(&data, "type").as_deref() {
                Some("credit") => {
                    let n = Reflect::get(&data, &"n".into())
                        .ok()
                        .and_then(|v| v.as_f64())
                        .unwrap_or(0.0) as u32;
                    credit.set(credit.get().saturating_add(n));
                    if let Some(cb) = pump.borrow().as_ref() {
                        let _ = js_sys::Function::from(cb.as_ref().clone()).call0(&JsValue::NULL);
                    }
                }
                Some("cancel") => {
                    cancelled.set(true);
                    let _ = reader.cancel();
                    // Terminal — free the port pair (see the EOF arm).
                    host_port.close();
                }
                _ => {}
            }
        }) as Box<dyn FnMut(MessageEvent)>)
    };
    host_port.set_onmessage(Some(onmessage.as_ref().unchecked_ref()));
    // Keep everything alive for the stream's lifetime. `onmessage` is leaked
    // (it's the only owner the browser-side port references). It holds an `Rc`
    // clone of `pump` (the `RefCell<Option<Closure>>`), which keeps the pump
    // closure itself alive — so the credit handler can still invoke it. Do NOT
    // take the pump out of the cell: that would empty it and the handler would
    // find nothing to pump.
    onmessage.forget();
}

/// Post a `fetch-error` derived from a partially built `fetch-result` head
/// (reusing its `id`).
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn post_error_obj(port: &MessagePort, head: &Object, message: &str) {
    if let Some(id) = get_str(head, "id") {
        post_error(port, "fetch-error", &id, message);
    }
}

/// Serialize a `Headers` into a `[[name, value], …]` array. `Headers`
/// isn't structured-clonable, but this pair array is, and the guest
/// reconstructs a `Headers` from it.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn headers_to_array(headers: &web_sys::Headers) -> js_sys::Array {
    let out = js_sys::Array::new();
    let iter = js_sys::try_iter(headers).ok().flatten();
    if let Some(iter) = iter {
        for entry in iter.flatten() {
            // Each entry is a `[name, value]` array already.
            out.push(&entry);
        }
    }
    out
}

// --- Query-body construction --------------------------------------

/// Build the query body for a bridge call: no argument streams the
/// scoped entity; an explicit body is forwarded verbatim.
fn query_body(host: &Element, arg: &JsValue) -> Result<JsValue, String> {
    if arg.is_undefined() || arg.is_null() {
        no_arg_entity_query(host)
    } else {
        Ok(arg.clone())
    }
}

fn no_arg_entity_query(host: &Element) -> Result<JsValue, String> {
    let entity = host
        .get_attribute("entity")
        .filter(|s| !s.is_empty())
        .ok_or("tonk.subscribe()/query() with no argument requires a scoped `entity`")?;
    let descriptor = read_descriptor(host)
        .ok_or("tonk.subscribe()/query() with no argument requires a model descriptor")?;
    let query = crate::query::entity_query(&descriptor, &entity)
        .map_err(|e| format!("entity query: {e}"))?;
    serde_wasm_bindgen::to_value(&query).map_err(|e| format!("query body: {e}"))
}

fn read_descriptor(host: &Element) -> Option<String> {
    Reflect::get(host, &"descriptor".into())
        .ok()
        .and_then(|v| v.as_string())
}

/// Build the `context` object (`{ this, model, origin, repo, branch }`) the
/// iframe receives in its `ready` envelope. `this`/`model` come from the
/// host's attributes; `origin` is the host page's real origin (the opaque
/// guest's is `"null"`); `repo`/`branch` come from the portal's `with`
/// context — which lives OUTSIDE the iframe, so a guest control that would
/// normally resolve its repo via a `with` ancestor reads it from the context
/// instead. Anything needing a same-origin URL or the scoped repo reads it
/// from `window.tonk.context` rather than the DOM/`window.location`.
fn build_context(host: &Element, state: &Rc<RefCell<PortalState>>) -> Object {
    let context = Object::new();
    let this = host.get_attribute("entity").unwrap_or_default();
    let model = host.get_attribute("model").unwrap_or_default();
    let location = window().map(|w| w.location());
    // The host's real origin. Read it via `context_origin()`, not
    // `location.origin()` directly: when THIS portal is itself inside a sealed
    // guest (a NESTED `<tonk-site>`), the host document is `about:srcdoc` and
    // `location.origin()` is the opaque string `"null"`. `context_origin()`
    // prefers the origin the parent portal already forwarded in
    // `window.tonk.context.origin`, so the real origin propagates down every
    // nesting level; it falls back to `location.origin` at the top document
    // (no parent portal, so `window.tonk` is absent).
    let origin = tonk_host::bridge::context_origin().unwrap_or_default();
    // The guest's own `window.location` is `about:srcdoc`; its REAL location is
    // the parent's. Pass the parent's path + search + hash so the guest stamps
    // them on its requests (the SW reads them to route/contain) and so a
    // location-reading guest control (e.g. `<tonk-page>`, which couriers an
    // invite's `?access` + `#seed` into the join command) sees the real URL.
    // `search`/`hash` especially: browsers strip the query only from the
    // fragment, but the guest can't read EITHER off `about:srcdoc`, and the SW
    // never sees the fragment on a network request.
    let path = location
        .as_ref()
        .and_then(|l| l.pathname().ok())
        .unwrap_or_default();
    let search = location
        .as_ref()
        .and_then(|l| l.search().ok())
        .unwrap_or_default();
    let hash = location
        .as_ref()
        .and_then(|l| l.hash().ok())
        .unwrap_or_default();
    let (repo, branch, with) = state
        .borrow()
        .with
        .as_ref()
        .map(|with| {
            (
                with.space().unwrap_or_default().to_owned(),
                with.effective_branch().to_owned(),
                with.to_string(),
            )
        })
        .unwrap_or_default();
    // The host's per-tab `site` entity (`site:<uuid>`). The guest's data queries
    // are ultimately issued by the installed host over HTTP, which stamps
    // THIS site on `X-Tonk-Site` — so the SW keys this tab's `tonk:site` facts by
    // it, not by the guest's own `guest:…` id. Guest content that renders the
    // routing indirection binds `entity` to this so it resolves the facts the SW
    // actually stamped.
    let site = tonk_host::bridge::site_id();
    let _ = Reflect::set(&context, &"this".into(), &JsValue::from_str(&this));
    let _ = Reflect::set(&context, &"model".into(), &JsValue::from_str(&model));
    let _ = Reflect::set(&context, &"origin".into(), &JsValue::from_str(&origin));
    // The per-space SYNTHETIC origin this guest believes it lives at
    // (`https://{label}.tonk.network/`), so in-guest navigation resolves like an
    // ordinary page: in-space routes are plain absolute paths under it, and an
    // href that escapes it is external. Distinct from `origin` (the REAL host
    // origin, which propagates down nesting and keys the `/api` relay strip).
    // Absent for the profile/Hub (no space) — those links are genuinely
    // top-level and want the real origin.
    let base = tonk_host::space_origin::space_origin_for(&repo).unwrap_or_default();
    let _ = Reflect::set(&context, &"base".into(), &JsValue::from_str(&base));
    let _ = Reflect::set(&context, &"path".into(), &JsValue::from_str(&path));
    let _ = Reflect::set(&context, &"search".into(), &JsValue::from_str(&search));
    let _ = Reflect::set(&context, &"hash".into(), &JsValue::from_str(&hash));
    let _ = Reflect::set(&context, &"repo".into(), &JsValue::from_str(&repo));
    let _ = Reflect::set(&context, &"branch".into(), &JsValue::from_str(&branch));
    // The pinned context as one `branch@repo` location: the guest host's
    // fallback route for consumers with no `with` of their own.
    let _ = Reflect::set(&context, &"with".into(), &JsValue::from_str(&with));
    let _ = Reflect::set(&context, &"site".into(), &JsValue::from_str(&site));
    context
}

// --- Frame routing (called by the element's reset / error shims) --

/// `reset(conclusions, { tag })` — a subscription frame from the host.
/// The host serializes conclusions with `serde-wasm-bindgen`, which
/// renders maps as JS `Map`s (and integers as `BigInt`). Round-trip
/// through JSON so the wire shape is identical to what `query()` yields
/// (the host `JSON.parse`s one-shot results) — numbers, not `BigInt`s,
/// plain objects, not `Map`s — which `postMessage`'s structured clone
/// would not otherwise guarantee. The plain rows are posted to the
/// iframe as a `subscribe-event` addressed to the author's stream.
pub(crate) fn route_reset(state: &Rc<RefCell<PortalState>>, payload: JsValue, opts: JsValue) {
    let Some(tag) = read_tag(&opts) else {
        return;
    };
    let conclusions: Vec<Conclusion> = match serde_wasm_bindgen::from_value(payload) {
        Ok(v) => v,
        Err(_) => return,
    };
    let plain = match serde_json::to_string(&conclusions) {
        Ok(json) => js_sys::JSON::parse(&json).unwrap_or(JsValue::NULL),
        Err(_) => return,
    };
    let Some((port, iframe_id)) = lookup_sub(state, &tag) else {
        return;
    };
    let env = Object::new();
    set_v1(&env, "subscribe-event");
    let _ = Reflect::set(&env, &"id".into(), &JsValue::from_str(&iframe_id));
    let _ = Reflect::set(&env, &"rows".into(), &plain);
    let _ = port.post_message(&env);
}

/// `update({ asserted, retracted }, { tag })` — an incremental frame.
///
/// Relays the delta to the guest as a `subscribe-event` carrying a
/// `delta` field (rather than `rows`), normalized through JSON so the
/// wire shape matches `reset`'s rows. The guest stream enqueues the
/// tagged frame; the guest consumer applies the delta to its retained
/// set exactly as the top-level `<tonk-display>` does.
pub(crate) fn route_update(state: &Rc<RefCell<PortalState>>, payload: JsValue, opts: JsValue) {
    let Some(tag) = read_tag(&opts) else {
        return;
    };
    // Normalize the `{asserted, retracted}` object through JSON so the
    // nested `fields` are plain objects, not `Map`s, across postMessage.
    let plain = match js_sys::JSON::stringify(&payload) {
        Ok(s) => js_sys::JSON::parse(&String::from(s)).unwrap_or(JsValue::NULL),
        Err(_) => return,
    };
    let Some((port, iframe_id)) = lookup_sub(state, &tag) else {
        return;
    };
    let env = Object::new();
    set_v1(&env, "subscribe-event");
    let _ = Reflect::set(&env, &"id".into(), &JsValue::from_str(&iframe_id));
    let _ = Reflect::set(&env, &"delta".into(), &plain);
    let _ = port.post_message(&env);
}

/// `error(detail, { tag })` — a transport error on a subscription.
/// Posts a `subscribe-error` so the matching author stream errors.
pub(crate) fn route_error(state: &Rc<RefCell<PortalState>>, payload: JsValue, opts: JsValue) {
    let Some(tag) = read_tag(&opts) else {
        return;
    };
    let Some((port, iframe_id)) = lookup_sub(state, &tag) else {
        return;
    };
    post_error(
        &port,
        "subscribe-error",
        &iframe_id,
        &error_message(&payload),
    );
}

/// Resolve `(current port, iframe correlation id)` for a live tag. The
/// port is read from `state` at call time, so frames always go to the
/// portal's current handshake — never a port captured when the
/// subscription opened.
fn lookup_sub(state: &Rc<RefCell<PortalState>>, tag: &str) -> Option<(MessagePort, String)> {
    let s = state.borrow();
    let iframe_id = s.subs.get(tag)?.iframe_id.clone();
    let port = s.port.clone()?;
    Some((port, iframe_id))
}

// --- Small helpers -------------------------------------------------

fn read_tag(opts: &JsValue) -> Option<String> {
    if !opts.is_object() {
        return None;
    }
    get_str(opts, "tag")
}

fn get_str(obj: &JsValue, key: &str) -> Option<String> {
    Reflect::get(obj, &key.into())
        .ok()
        .and_then(|v| v.as_string())
}

fn get_body(data: &JsValue) -> JsValue {
    Reflect::get(data, &"body".into()).unwrap_or(JsValue::UNDEFINED)
}

fn read_first_port(event: &MessageEvent) -> Option<MessagePort> {
    let ports = Reflect::get(event, &"ports".into()).ok()?;
    let ports: js_sys::Array = ports.dyn_into().ok()?;
    ports.get(0).dyn_into::<MessagePort>().ok()
}

/// Read a human message out of an error payload: a string verbatim,
/// otherwise its `message` field, otherwise its debug form.
fn error_message(payload: &JsValue) -> String {
    if let Some(s) = payload.as_string() {
        return s;
    }
    get_str(payload, "message").unwrap_or_else(|| format!("{payload:?}"))
}

fn set_v1(env: &Object, ty: &str) {
    let _ = Reflect::set(env, &"v".into(), &JsValue::from_f64(1.0));
    let _ = Reflect::set(env, &"type".into(), &JsValue::from_str(ty));
}

fn post_result(port: &MessagePort, ty: &str, id: &str, field: &str, value: &JsValue) {
    let env = Object::new();
    set_v1(&env, ty);
    let _ = Reflect::set(&env, &"id".into(), &JsValue::from_str(id));
    let _ = Reflect::set(&env, &field.into(), value);
    let _ = port.post_message(&env);
}

fn post_error(port: &MessagePort, ty: &str, id: &str, error: &str) {
    let env = Object::new();
    set_v1(&env, ty);
    let _ = Reflect::set(&env, &"id".into(), &JsValue::from_str(id));
    let _ = Reflect::set(&env, &"error".into(), &JsValue::from_str(error));
    let _ = port.post_message(&env);
}

#[cfg(test)]
mod runtime_bootstrap_tests {
    use super::RUNTIME_BOOTSTRAP_JS;

    /// A nested guest fills its parent's whole viewport, so once content
    /// renders in one, no click reaches the frame the FABB lives in and its
    /// open stack cannot be dismissed by clicking away. The guest reports
    /// the press upward and every ancestor redispatches it, so the dismiss
    /// listeners already on those documents fire.
    #[test]
    fn a_press_in_a_guest_reaches_every_ancestor() {
        assert!(RUNTIME_BOOTSTRAP_JS.contains(r#"__tonkRuntime:"press""#));
        // Capture phase: content that stops propagation must not also
        // stop an ancestor's overlay from closing.
        assert!(RUNTIME_BOOTSTRAP_JS.contains(r#"document.addEventListener("pointerdown""#));
        // Relayed onward, so the press climbs past the first ancestor.
        assert!(
            RUNTIME_BOOTSTRAP_JS
                .contains(r#"document.dispatchEvent(new PointerEvent("pointerdown""#)
        );
    }

    /// Only the FACT of the press travels. Coordinates or a target would
    /// let an ancestor observe what was pressed inside a sealed guest.
    #[test]
    fn the_relayed_press_carries_nothing_about_what_was_pressed() {
        let at = RUNTIME_BOOTSTRAP_JS
            .find(r#"parent.postMessage({__tonkRuntime:"press"}"#)
            .expect("the relay");
        let message = &RUNTIME_BOOTSTRAP_JS[at..at + 60];
        for leak in ["clientX", "clientY", "target", "path"] {
            assert!(
                !message.contains(leak),
                "press relay leaks {leak}: {message}"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use js_sys::{Array, Function, Promise};
    use wasm_bindgen_futures::JsFuture;
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    use web_sys::{CustomEvent, Document, MessageChannel};

    wasm_bindgen_test_configure!(run_in_browser);

    fn document() -> Document {
        window().expect("window").document().expect("document")
    }

    /// Sleep `ms` milliseconds, yielding to the event loop.
    async fn sleep(ms: i32) {
        let promise = Promise::new(&mut |resolve, _reject| {
            let _ = window()
                .expect("window")
                .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms);
        });
        let _ = JsFuture::from(promise).await;
    }

    const DESCRIPTOR: &str = r#"{"with":{
        "count": { "the": "counter/count", "as": "UnsignedInteger", "cardinality": "one" }
    }}"#;

    /// The guest resolves every link against its synthetic per-space
    /// origin, so a link to a PROFILE route (`/join`) arrives looking
    /// exactly like an in-space path. Rewriting it to `/space/{did}/join`
    /// names a route no space defines, and the app then tries to boot
    /// inside the sealed frame — opaque origin, no service worker, every
    /// asset CORS-blocked, renderer crash. These must pass through.
    #[dialog_common::test]
    fn it_treats_profile_routes_as_top_level() {
        assert!(is_top_level_route("join"));
        assert!(is_top_level_route("account"));
        assert!(is_top_level_route("inspector"));
        assert!(is_top_level_route("diagnose"));
        // A query or sub-path does not disguise the route.
        assert!(is_top_level_route("join?x=1"));
        assert!(is_top_level_route("account/devices"));
        assert!(is_top_level_route("diagnose/abc123"));
    }

    /// Everything else is genuinely in-space and still gets the space
    /// prefix — the behaviour the pass-through must not swallow.
    #[dialog_common::test]
    fn it_leaves_in_space_paths_to_the_space_prefix() {
        assert!(!is_top_level_route("activity"));
        assert!(!is_top_level_route("board"));
        assert!(!is_top_level_route(""));
        // A path that merely STARTS with a top-level name is not one.
        assert!(!is_top_level_route("joinery"));
        assert!(!is_top_level_route("accounts-payable"));
    }

    // --- find_font_paths: url() argument extraction --------------------

    #[dialog_common::test]
    fn it_finds_font_paths_in_url_args_only() {
        // The comment reproduces styles.css prose that the old raw-substring
        // scan turned into `GET /fonts/%60%20(copied…` — a 404 on every
        // guest boot. Prose mentions of `/fonts/` must not count; quoted,
        // single-quoted, and bare url() arguments must; duplicates collapse.
        let css = r#"
            /* Files live under `/fonts/` (copied into the dist by Trunk's
               `copy-dir` on `./assets/fonts`). */
            @font-face { src: url("/fonts/space-grotesk-400.woff2") format("woff2"); }
            @font-face { src: url('/fonts/gestalte-400.otf'); }
            .bare { mask: url(/fonts/unquoted.ttf); }
            .other { background: url("/images/mark-white.svg"); }
            .dup { src: url("/fonts/space-grotesk-400.woff2"); }
            .empty { background: url("/fonts/"); }
        "#;
        assert_eq!(
            find_font_paths(css),
            vec![
                "/fonts/space-grotesk-400.woff2",
                "/fonts/gestalte-400.otf",
                "/fonts/unquoted.ttf",
            ]
        );
    }

    // --- forwarded_route: the allow chokepoint -------------------------

    /// A portal state pinned to `with` and granting `allow`.
    fn routed_state(with: Option<&str>, allow: &str) -> Rc<RefCell<PortalState>> {
        let state = Rc::new(RefCell::new(PortalState::new()));
        state.borrow_mut().set_route(
            with.map(|w| w.parse().expect("with parses")),
            allow.parse().expect("allow parses"),
        );
        state
    }

    /// An envelope carrying (only) a forwarded `with` route.
    fn route_envelope(with: Option<&str>) -> JsValue {
        let data = Object::new();
        if let Some(with) = with {
            let _ = Reflect::set(&data, &"with".into(), &JsValue::from_str(with));
        }
        data.into()
    }

    #[dialog_common::test]
    fn it_classifies_repository_metadata_under_the_portal_reach() {
        let named = routed_state(Some("main@did:key:zSpace"), "main@did:key:zSpace");
        assert_eq!(
            data_plane_location("/api/repository/did:key:zSpace", &named),
            Some("main@did:key:zSpace".parse().unwrap()),
        );

        let profile = routed_state(Some("main@profile:tonk"), "main@profile:tonk");
        assert_eq!(
            data_plane_location("/api/profile/repository", &profile),
            Some("main@profile:tonk".parse().unwrap()),
        );
    }

    #[dialog_common::test]
    fn it_does_not_misclassify_repository_control_routes() {
        let state = routed_state(Some("main@did:key:zSpace"), "main@did:key:zSpace");
        assert_eq!(
            data_plane_location("/api/repository/did:key:zSpace/remote", &state),
            None,
        );
        assert_eq!(
            data_plane_location("/api/repository/did:key:zSpace/invite", &state),
            None,
        );
    }

    #[dialog_common::test]
    fn it_pins_an_unrouted_operation_to_the_portals_with() {
        let state = routed_state(Some("main@profile:tonk"), "*");
        let route = forwarded_route(&state, &route_envelope(None)).expect("pinned route");
        assert_eq!(route, (None, Some("main".into()), true));

        let state = routed_state(Some("main@did:key:zA"), "main@did:key:zA");
        let route = forwarded_route(&state, &route_envelope(None)).expect("pinned route");
        assert_eq!(
            route,
            (Some("did:key:zA".into()), Some("main".into()), false)
        );
    }

    #[dialog_common::test]
    fn it_honors_a_forwarded_route_the_allow_lists() {
        let state = routed_state(Some("main@profile:tonk"), "*");
        let route =
            forwarded_route(&state, &route_envelope(Some("did:key:zB"))).expect("honored route");
        assert_eq!(route, (Some("did:key:zB".into()), None, false));

        // The sealed shape: allow lists exactly the portal's own location,
        // and a bare-repo request normalizes to the same reach.
        let state = routed_state(Some("main@did:key:zA"), "main@did:key:zA");
        let route = forwarded_route(&state, &route_envelope(Some("did:key:zA")))
            .expect("own reach honored");
        assert_eq!(route, (Some("did:key:zA".into()), None, false));
    }

    #[dialog_common::test]
    fn it_denies_a_forwarded_route_outside_the_allow() {
        let state = routed_state(Some("main@did:key:zA"), "main@did:key:zA");
        let refused = forwarded_route(&state, &route_envelope(Some("main@did:key:zEve")))
            .expect_err("off-repo route must be refused");
        assert!(
            refused.message().starts_with("denied:"),
            "expected a typed denial, got: {}",
            refused.message(),
        );
        // Another branch of the SAME repo is still outside the allow.
        let refused = forwarded_route(&state, &route_envelope(Some("draft@did:key:zA")))
            .expect_err("off-branch route must be refused");
        assert!(refused.message().starts_with("denied:"));
    }

    #[dialog_common::test]
    fn it_refuses_a_malformed_forwarded_route() {
        let state = routed_state(Some("main@did:key:zA"), "*");
        let refused = forwarded_route(&state, &route_envelope(Some("main@")))
            .expect_err("malformed route must be refused");
        assert!(
            refused.message().starts_with("malformed"),
            "expected a malformed refusal, got: {}",
            refused.message(),
        );
    }

    #[dialog_common::test]
    fn it_denies_every_forwarded_route_for_an_unpinned_portal() {
        // A portal with no `with` of its own (and thus `Allow::none()`)
        // relays nothing the guest asks for.
        let state = routed_state(None, "*");
        // `*` still honors — the grant is the caller's choice…
        assert!(forwarded_route(&state, &route_envelope(Some("did:key:zB"))).is_ok());
        // …but the generic-portal default (no with → none) denies all.
        let state = Rc::new(RefCell::new(PortalState::new()));
        let refused = forwarded_route(&state, &route_envelope(Some("did:key:zB")))
            .expect_err("default portal must deny forwarded routes");
        assert!(refused.message().starts_with("denied:"));
    }

    // --- FakeHost: a stand-in installed host --------------------------

    /// A minimal stand-in for the installed host: a container that answers the
    /// consumer events the relay dispatches with canned data, captures
    /// the live subscription's consumer + tag, and records cancellation.
    struct FakeHost {
        container: Element,
        state: Rc<RefCell<FakeState>>,
        _listeners: Vec<Closure<dyn FnMut(CustomEvent)>>,
    }

    #[derive(Default)]
    struct FakeState {
        query_result: Option<JsValue>,
        claim_result: Option<JsValue>,
        last_query_body: Option<JsValue>,
        last_claim_body: Option<JsValue>,
        sub_consumer: Option<Element>,
        sub_tag: Option<JsValue>,
        last_subscribe_body: Option<JsValue>,
        cancelled: bool,
    }

    impl FakeHost {
        fn install() -> FakeHost {
            let container = document().create_element("div").expect("div");
            document()
                .body()
                .expect("body")
                .append_child(&container)
                .expect("attach container");
            let state = Rc::new(RefCell::new(FakeState::default()));
            let mut listeners = Vec::new();

            {
                let state = state.clone();
                let cb: Closure<dyn FnMut(CustomEvent)> =
                    Closure::wrap(Box::new(move |ev: CustomEvent| {
                        ev.stop_propagation();
                        ev.prevent_default();
                        let detail: Object = ev.detail().dyn_into().unwrap();
                        let query = Reflect::get(&detail, &"query".into()).unwrap();
                        state.borrow_mut().last_query_body = Some(query);
                        let result = state
                            .borrow()
                            .query_result
                            .clone()
                            .unwrap_or(JsValue::from(Array::new()));
                        let _ = Reflect::set(&detail, &"result".into(), &Promise::resolve(&result));
                    }) as Box<dyn FnMut(CustomEvent)>);
                let _ = container
                    .add_event_listener_with_callback("tonk-query", cb.as_ref().unchecked_ref());
                listeners.push(cb);
            }
            {
                let state = state.clone();
                let cb: Closure<dyn FnMut(CustomEvent)> =
                    Closure::wrap(Box::new(move |ev: CustomEvent| {
                        ev.stop_propagation();
                        ev.prevent_default();
                        let detail: Object = ev.detail().dyn_into().unwrap();
                        let request = Reflect::get(&detail, &"request".into()).unwrap();
                        state.borrow_mut().last_claim_body = Some(request);
                        let result = state
                            .borrow()
                            .claim_result
                            .clone()
                            .unwrap_or(JsValue::from_str("ok"));
                        let _ = Reflect::set(&detail, &"result".into(), &Promise::resolve(&result));
                    }) as Box<dyn FnMut(CustomEvent)>);
                let _ = container
                    .add_event_listener_with_callback("tonk-claim", cb.as_ref().unchecked_ref());
                listeners.push(cb);
            }
            {
                let state = state.clone();
                let cb: Closure<dyn FnMut(CustomEvent)> =
                    Closure::wrap(Box::new(move |ev: CustomEvent| {
                        ev.stop_propagation();
                        ev.prevent_default();
                        let detail: Object = ev.detail().dyn_into().unwrap();
                        let query = Reflect::get(&detail, &"query".into()).unwrap();
                        let tag = Reflect::get(&detail, &"tag".into()).ok();
                        let consumer: Element = ev.target().unwrap().dyn_into().unwrap();
                        {
                            let mut s = state.borrow_mut();
                            s.last_subscribe_body = Some(query);
                            s.sub_consumer = Some(consumer);
                            s.sub_tag = tag;
                        }
                        let sub = Object::new();
                        let state_for_cancel = state.clone();
                        let cancel: Closure<dyn FnMut()> = Closure::wrap(Box::new(move || {
                            state_for_cancel.borrow_mut().cancelled = true;
                        })
                            as Box<dyn FnMut()>);
                        let cancel_fn: Function = cancel.into_js_value().unchecked_into();
                        let _ = Reflect::set(&sub, &"cancel".into(), &cancel_fn);
                        let _ = Reflect::set(&detail, &"subscription".into(), &sub);
                    }) as Box<dyn FnMut(CustomEvent)>);
                let _ = container.add_event_listener_with_callback(
                    "tonk-subscribe",
                    cb.as_ref().unchecked_ref(),
                );
                listeners.push(cb);
            }

            FakeHost {
                container,
                state,
                _listeners: listeners,
            }
        }

        fn set_query_result(&self, value: JsValue) {
            self.state.borrow_mut().query_result = Some(value);
        }
        fn set_claim_result(&self, value: JsValue) {
            self.state.borrow_mut().claim_result = Some(value);
        }
        fn last_query_body(&self) -> Option<JsValue> {
            self.state.borrow().last_query_body.clone()
        }
        fn last_claim_body(&self) -> Option<JsValue> {
            self.state.borrow().last_claim_body.clone()
        }
        fn sub_tag(&self) -> Option<JsValue> {
            self.state.borrow().sub_tag.clone()
        }
        fn cancelled(&self) -> bool {
            self.state.borrow().cancelled
        }

        /// Push a subscription frame to the captured consumer, mirroring
        /// how the real host calls `consumer.reset(conclusions, { tag })`.
        fn push_frame(&self, conclusions: &JsValue) {
            let (consumer, tag) = {
                let s = self.state.borrow();
                (s.sub_consumer.clone(), s.sub_tag.clone())
            };
            let Some(consumer) = consumer else { return };
            let opts = Object::new();
            if let Some(t) = tag {
                let _ = Reflect::set(&opts, &"tag".into(), &t);
            }
            let reset = Reflect::get(&consumer, &"reset".into()).unwrap();
            let reset: Function = reset.dyn_into().expect("reset method");
            let _ = reset.call2(&consumer, conclusions, &opts);
        }
    }

    /// A consumer element that dispatches the bridge's events: a `<div>`
    /// under the fake host carrying the scoped `entity` / `model` and the
    /// model `descriptor` the relay reads for no-argument calls.
    fn relay_consumer(
        host: &FakeHost,
        entity: Option<&str>,
        model: Option<&str>,
        descriptor: Option<&str>,
    ) -> Element {
        let consumer = document().create_element("div").expect("div");
        if let Some(e) = entity {
            consumer.set_attribute("entity", e).expect("entity");
        }
        if let Some(m) = model {
            consumer.set_attribute("model", m).expect("model");
        }
        if let Some(d) = descriptor {
            let _ = Reflect::set(
                consumer.as_ref(),
                &"descriptor".into(),
                &JsValue::from_str(d),
            );
        }
        host.container.append_child(&consumer).expect("attach");
        consumer
    }

    // --- Port plumbing for relay tests ------------------------------

    /// Collects messages arriving on a port and lets a test await the
    /// first one of a given `type`.
    struct PortListener {
        messages: Rc<RefCell<Vec<JsValue>>>,
        _cb: Closure<dyn FnMut(MessageEvent)>,
    }

    impl PortListener {
        fn attach(port: &MessagePort) -> Self {
            let messages = Rc::new(RefCell::new(Vec::new()));
            let sink = messages.clone();
            let cb: Closure<dyn FnMut(MessageEvent)> =
                Closure::wrap(Box::new(move |e: MessageEvent| {
                    sink.borrow_mut().push(e.data());
                }) as Box<dyn FnMut(MessageEvent)>);
            // Setting onmessage auto-starts the port.
            port.set_onmessage(Some(cb.as_ref().unchecked_ref()));
            PortListener { messages, _cb: cb }
        }

        async fn wait_for(&self, ty: &str) -> JsValue {
            for _ in 0..200 {
                let found = self
                    .messages
                    .borrow()
                    .iter()
                    .find(|d| get_str(d, "type").as_deref() == Some(ty))
                    .cloned();
                if let Some(found) = found {
                    return found;
                }
                sleep(5).await;
            }
            JsValue::UNDEFINED
        }

        /// Await the first message of any shape (for raw envelopes that
        /// carry no `type` field, e.g. a bare transferred-body probe).
        async fn wait_for_any(&self) -> JsValue {
            for _ in 0..200 {
                let found = self.messages.borrow().first().cloned();
                if let Some(found) = found {
                    return found;
                }
                sleep(5).await;
            }
            JsValue::UNDEFINED
        }

        /// How many messages have arrived so far.
        fn count(&self) -> usize {
            self.messages.borrow().len()
        }

        /// Drop collected messages so `wait_for_any` returns the next one.
        fn clear(&self) {
            self.messages.borrow_mut().clear();
        }
    }

    /// Build a `{ v, type, id, ...extra }` envelope to post from the
    /// test side of the channel.
    fn envelope(ty: &str, id: &str) -> Object {
        let env = Object::new();
        set_v1(&env, ty);
        let _ = Reflect::set(&env, &"id".into(), &JsValue::from_str(id));
        env
    }

    /// Wire a fresh `MessageChannel`: bind one end to the portal relay
    /// (as a `hello` would) and return the other end's listener + port
    /// for the test to drive.
    fn bind(consumer: &Element, state: &Rc<RefCell<PortalState>>) -> (PortListener, MessagePort) {
        let channel = MessageChannel::new().expect("MessageChannel");
        let test_port = channel.port1();
        let portal_port = channel.port2();
        let listener = PortListener::attach(&test_port);
        bind_port(consumer, state, portal_port);
        (listener, test_port)
    }

    /// A host-shaped subscription frame: `Vec<Conclusion>` serialized
    /// with `serde-wasm-bindgen` (which renders maps as JS `Map`s), as
    /// the installed host delivers them.
    fn host_frame(this: &str, count: i128) -> JsValue {
        use ipld_core::ipld::Ipld;
        let mut fields: BTreeMap<String, Ipld> = BTreeMap::new();
        fields.insert("count".to_owned(), Ipld::Integer(count));
        let conclusions = vec![Conclusion {
            this: this.to_owned(),
            fields,
        }];
        serde_wasm_bindgen::to_value(&conclusions).expect("serialize frame")
    }

    fn get_num(obj: &JsValue, key: &str) -> Option<f64> {
        Reflect::get(obj, &key.into()).ok().and_then(|v| v.as_f64())
    }

    // --- Relay tests (seam 1) ---------------------------------------

    #[dialog_common::test]
    async fn it_posts_ready_with_context_on_bind() {
        let host = FakeHost::install();
        let consumer = relay_consumer(&host, Some("id:demo-counter"), Some("counter"), None);
        let state = Rc::new(RefCell::new(PortalState::new()));
        let (listener, _port) = bind(&consumer, &state);

        let ready = listener.wait_for("ready").await;
        let context = Reflect::get(&ready, &"context".into()).expect("context");
        assert_eq!(
            get_str(&context, "this").as_deref(),
            Some("id:demo-counter")
        );
        assert_eq!(get_str(&context, "model").as_deref(), Some("counter"));
        // The host forwards its real `search` (the `?query`) into the guest
        // context — a sealed guest can't read it off its own `about:srcdoc`
        // location, and `<tonk-page>` needs it to courier an invite's `?access`.
        assert!(
            get_str(&context, "search").is_some(),
            "context carries a `search` field forwarded from the host location",
        );
    }

    /// When THIS portal is itself a nested guest, its host document is
    /// `about:srcdoc` and `location.origin` is `"null"`; the real origin lives
    /// in the parent-forwarded `window.tonk.context.origin`. The ready envelope
    /// must carry that forwarded origin (not `"null"`), so a further-nested
    /// guest can build a same-origin invite link. Simulated by installing a
    /// `window.tonk.context.origin` before bind.
    #[dialog_common::test]
    async fn it_forwards_the_parent_context_origin() {
        let win = window().expect("window");
        let tonk = Object::new();
        let ctx = Object::new();
        let _ = Reflect::set(&ctx, &"origin".into(), &"https://forwarded.test".into());
        let _ = Reflect::set(&tonk, &"context".into(), &ctx);
        let _ = Reflect::set(&win, &"tonk".into(), &tonk);

        let host = FakeHost::install();
        let consumer = relay_consumer(&host, Some("id:demo-counter"), Some("counter"), None);
        let state = Rc::new(RefCell::new(PortalState::new()));
        let (listener, _port) = bind(&consumer, &state);

        let ready = listener.wait_for("ready").await;
        let context = Reflect::get(&ready, &"context".into()).expect("context");

        // Restore before asserting so a failure doesn't leak `window.tonk`
        // into a later test running in the same page.
        let _ = Reflect::set(&win, &"tonk".into(), &JsValue::UNDEFINED);

        assert_eq!(
            get_str(&context, "origin").as_deref(),
            Some("https://forwarded.test"),
            "a nested portal forwards the parent context origin, not `about:srcdoc`'s null",
        );
    }

    #[dialog_common::test]
    async fn it_relays_a_query_envelope_and_returns_rows() {
        let host = FakeHost::install();
        let canned = Array::new();
        canned.push(&JsValue::from_str("row"));
        host.set_query_result(canned.into());
        let consumer = relay_consumer(&host, None, None, None);
        let state = Rc::new(RefCell::new(PortalState::new()));
        let (listener, port) = bind(&consumer, &state);

        // Explicit body is forwarded verbatim.
        let env = envelope("query", "r1");
        let body = Object::new();
        let _ = Reflect::set(&body, &"marker".into(), &JsValue::from_str("explicit"));
        let _ = Reflect::set(&env, &"body".into(), &body);
        port.post_message(&env).expect("post query");

        let result = listener.wait_for("query-result").await;
        assert_eq!(get_str(&result, "id").as_deref(), Some("r1"));
        let rows: Array = Reflect::get(&result, &"rows".into())
            .expect("rows")
            .dyn_into()
            .expect("array");
        assert_eq!(rows.get(0).as_string().as_deref(), Some("row"));

        let dispatched = host.last_query_body().expect("query dispatched");
        assert_eq!(
            get_str(&dispatched, "marker").as_deref(),
            Some("explicit"),
            "explicit body forwarded verbatim",
        );
    }

    #[dialog_common::test]
    async fn it_builds_the_no_arg_query_from_descriptor_and_entity() {
        let host = FakeHost::install();
        let consumer = relay_consumer(
            &host,
            Some("id:demo-counter"),
            Some("counter"),
            Some(DESCRIPTOR),
        );
        let state = Rc::new(RefCell::new(PortalState::new()));
        let (listener, port) = bind(&consumer, &state);

        // No `body` field — the relay must build the scoped-entity query.
        port.post_message(&envelope("query", "r1")).expect("post");
        let _ = listener.wait_for("query-result").await;

        let body = host.last_query_body().expect("query dispatched");
        let terms = Reflect::get(&body, &"terms".into()).expect("terms");
        // `serde-wasm-bindgen` renders the body as nested `Map`s.
        let this = {
            let map: js_sys::Map = terms.dyn_into().expect("terms is a Map");
            map.get(&"this".into())
        };
        assert_eq!(this.as_string().as_deref(), Some("id:demo-counter"));
    }

    #[dialog_common::test]
    async fn it_relays_a_transact_envelope_to_claim() {
        let host = FakeHost::install();
        host.set_claim_result(JsValue::from_str("receipt"));
        let consumer = relay_consumer(&host, None, None, None);
        let state = Rc::new(RefCell::new(PortalState::new()));
        let (listener, port) = bind(&consumer, &state);

        let env = envelope("transact", "r1");
        let request = Object::new();
        let _ = Reflect::set(&request, &"assert".into(), &JsValue::from_str("something"));
        let _ = Reflect::set(&env, &"request".into(), &request);
        port.post_message(&env).expect("post transact");

        let result = listener.wait_for("transact-result").await;
        assert_eq!(get_str(&result, "id").as_deref(), Some("r1"));
        assert_eq!(
            Reflect::get(&result, &"receipt".into())
                .ok()
                .and_then(|v| v.as_string())
                .as_deref(),
            Some("receipt"),
        );
        let body = host.last_claim_body().expect("claim dispatched");
        assert_eq!(get_str(&body, "assert").as_deref(), Some("something"));
    }

    #[dialog_common::test]
    async fn it_opens_a_host_subscription_and_posts_reset_frames() {
        let host = FakeHost::install();
        let consumer = relay_consumer(
            &host,
            Some("id:demo-counter"),
            Some("counter"),
            Some(DESCRIPTOR),
        );
        let state = Rc::new(RefCell::new(PortalState::new()));
        let (listener, port) = bind(&consumer, &state);

        port.post_message(&envelope("subscribe", "r1"))
            .expect("post subscribe");

        // Wait for the host subscription to open and capture its tag.
        let mut tag = JsValue::UNDEFINED;
        for _ in 0..200 {
            if let Some(t) = host.sub_tag() {
                tag = t;
                break;
            }
            sleep(5).await;
        }
        assert!(!tag.is_undefined(), "subscribe should reach the host");

        // A host frame for that tag must come back as a subscribe-event
        // addressed to the iframe's correlation id, dot-accessible.
        route_reset(&state, host_frame("id:demo-counter", 5), tag_opts(&tag));
        let event = listener.wait_for("subscribe-event").await;
        assert_eq!(get_str(&event, "id").as_deref(), Some("r1"));
        let rows: Array = Reflect::get(&event, &"rows".into())
            .expect("rows")
            .dyn_into()
            .expect("array");
        let me = rows.get(0);
        assert_eq!(get_str(&me, "this").as_deref(), Some("id:demo-counter"));
        let fields = Reflect::get(&me, &"fields".into()).expect("fields");
        assert_eq!(
            get_num(&fields, "count"),
            Some(5.0),
            "integer field is a plain number, not a BigInt",
        );
    }

    #[dialog_common::test]
    async fn it_errors_the_stream_on_a_reset_error_frame() {
        let host = FakeHost::install();
        let consumer = relay_consumer(
            &host,
            Some("id:demo-counter"),
            Some("counter"),
            Some(DESCRIPTOR),
        );
        let state = Rc::new(RefCell::new(PortalState::new()));
        let (listener, port) = bind(&consumer, &state);

        port.post_message(&envelope("subscribe", "r1"))
            .expect("post subscribe");
        let mut tag = JsValue::UNDEFINED;
        for _ in 0..200 {
            if let Some(t) = host.sub_tag() {
                tag = t;
                break;
            }
            sleep(5).await;
        }

        route_error(&state, JsValue::from_str("upstream gone"), tag_opts(&tag));
        let event = listener.wait_for("subscribe-error").await;
        assert_eq!(get_str(&event, "id").as_deref(), Some("r1"));
        assert_eq!(get_str(&event, "error").as_deref(), Some("upstream gone"));
    }

    #[dialog_common::test]
    async fn it_cancels_the_host_subscription_on_unsubscribe() {
        let host = FakeHost::install();
        let consumer = relay_consumer(
            &host,
            Some("id:demo-counter"),
            Some("counter"),
            Some(DESCRIPTOR),
        );
        let state = Rc::new(RefCell::new(PortalState::new()));
        let (_listener, port) = bind(&consumer, &state);

        port.post_message(&envelope("subscribe", "r1"))
            .expect("post subscribe");
        for _ in 0..200 {
            if host.sub_tag().is_some() {
                break;
            }
            sleep(5).await;
        }
        assert!(!host.cancelled(), "not cancelled before unsubscribe");

        port.post_message(&envelope("unsubscribe", "r1"))
            .expect("post unsubscribe");
        for _ in 0..200 {
            if host.cancelled() {
                break;
            }
            sleep(5).await;
        }
        assert!(
            host.cancelled(),
            "unsubscribe drops the BridgeSub, cancelling the host subscription",
        );
    }

    #[dialog_common::test]
    async fn it_returns_a_query_error_when_there_is_no_host_ancestor() {
        // A consumer attached to the body with no FakeHost ancestor:
        // `tonk-query` is not default-prevented, so the relay errors.
        let consumer = document().create_element("div").expect("div");
        document()
            .body()
            .expect("body")
            .append_child(&consumer)
            .expect("attach");
        let state = Rc::new(RefCell::new(PortalState::new()));
        let (listener, port) = bind(&consumer, &state);

        let env = envelope("query", "r1");
        let _ = Reflect::set(&env, &"body".into(), &Object::new());
        port.post_message(&env).expect("post query");

        let error = listener.wait_for("query-error").await;
        assert_eq!(get_str(&error, "id").as_deref(), Some("r1"));
        assert!(
            get_str(&error, "error").is_some(),
            "an error message should be relayed",
        );
    }

    fn tag_opts(tag: &JsValue) -> JsValue {
        let opts = Object::new();
        let _ = Reflect::set(&opts, &"tag".into(), tag);
        opts.into()
    }

    // --- End-to-end smoke tests (seam 2) ----------------------------

    /// Mount a real `<tonk-portal>` (opaque-origin iframe) under the
    /// fake host with the given attributes + descriptor property.
    fn mount_portal(
        host: &FakeHost,
        content: &str,
        entity: Option<&str>,
        model: Option<&str>,
        descriptor: Option<&str>,
    ) -> Element {
        crate::register();
        let portal = document()
            .create_element("tonk-portal")
            .expect("tonk-portal");
        portal.set_attribute("content", content).expect("content");
        if let Some(e) = entity {
            portal.set_attribute("entity", e).expect("entity");
        }
        if let Some(m) = model {
            portal.set_attribute("model", m).expect("model");
        }
        if let Some(d) = descriptor {
            let _ = Reflect::set(portal.as_ref(), &"descriptor".into(), &JsValue::from_str(d));
        }
        host.container.append_child(&portal).expect("attach portal");
        portal
    }

    /// Listen on `window` for the author iframe's `{ __test: tag, ... }`
    /// message posted back across the opaque-origin boundary.
    struct WindowProbe {
        message: Rc<RefCell<Option<JsValue>>>,
        _cb: Closure<dyn FnMut(MessageEvent)>,
    }

    impl WindowProbe {
        fn install(tag: &'static str) -> Self {
            let message = Rc::new(RefCell::new(None));
            let sink = message.clone();
            let cb: Closure<dyn FnMut(MessageEvent)> =
                Closure::wrap(Box::new(move |e: MessageEvent| {
                    let data = e.data();
                    if get_str(&data, "__test").as_deref() == Some(tag) {
                        *sink.borrow_mut() = Some(data);
                    }
                }) as Box<dyn FnMut(MessageEvent)>);
            let _ = window()
                .expect("window")
                .add_event_listener_with_callback("message", cb.as_ref().unchecked_ref());
            WindowProbe { message, _cb: cb }
        }

        async fn wait(&self) -> JsValue {
            for _ in 0..400 {
                if let Some(v) = self.message.borrow().clone() {
                    return v;
                }
                sleep(5).await;
            }
            JsValue::UNDEFINED
        }
    }

    #[dialog_common::test]
    async fn it_runs_a_real_query_across_the_opaque_origin_boundary() {
        let host = FakeHost::install();
        let canned = Array::new();
        canned.push(&JsValue::from_str("row"));
        host.set_query_result(canned.into());
        let probe = WindowProbe::install("q");

        // Author code runs at the opaque origin, calls tonk.query(), and
        // posts the result back to the parent (this test's window).
        let content = "<script>\
            tonk.query()\
              .then(function(rows){parent.postMessage({__test:'q',rows:rows},'*');})\
              .catch(function(err){parent.postMessage({__test:'q',error:String(err)},'*');});\
            </script>";
        mount_portal(
            &host,
            content,
            Some("id:demo-counter"),
            Some("counter"),
            Some(DESCRIPTOR),
        );

        let msg = probe.wait().await;
        assert!(
            !msg.is_undefined(),
            "author iframe should post a result back across the boundary",
        );
        assert!(
            Reflect::get(&msg, &"error".into())
                .ok()
                .filter(|v| !v.is_undefined())
                .is_none(),
            "query should not error; got: {:?}",
            Reflect::get(&msg, &"error".into()).ok(),
        );
        let rows: Array = Reflect::get(&msg, &"rows".into())
            .expect("rows")
            .dyn_into()
            .expect("array");
        assert_eq!(rows.get(0).as_string().as_deref(), Some("row"));
    }

    #[dialog_common::test]
    async fn it_delivers_subscription_frames_across_the_opaque_origin_boundary() {
        let host = FakeHost::install();
        let probe = WindowProbe::install("s");

        // Author subscribes, reads one frame, posts it back.
        let content = "<script>\
            var reader = tonk.subscribe().getReader();\
            reader.read().then(function(r){parent.postMessage({__test:'s',value:r.value},'*');});\
            </script>";
        mount_portal(
            &host,
            content,
            Some("id:demo-counter"),
            Some("counter"),
            Some(DESCRIPTOR),
        );

        // Wait for the host subscription to open, then push a frame.
        for _ in 0..400 {
            if host.sub_tag().is_some() {
                break;
            }
            sleep(5).await;
        }
        assert!(host.sub_tag().is_some(), "subscribe should reach the host");
        host.push_frame(&host_frame("id:demo-counter", 7));

        let msg = probe.wait().await;
        assert!(!msg.is_undefined(), "author should post a frame back");
        let rows: Array = Reflect::get(&msg, &"value".into())
            .expect("value")
            .dyn_into()
            .expect("Conclusion[]");
        let me = rows.get(0);
        assert_eq!(get_str(&me, "this").as_deref(), Some("id:demo-counter"));
        let fields = Reflect::get(&me, &"fields".into()).expect("fields");
        assert_eq!(get_num(&fields, "count"), Some(7.0));
    }

    #[dialog_common::test]
    async fn it_ignores_a_hello_from_an_unregistered_source() {
        // A registered portal whose iframe never speaks: the registry is
        // non-empty, but only its live `contentWindow` may complete a
        // handshake.
        install_message_listener();
        let host = FakeHost::install();
        let consumer = relay_consumer(&host, None, None, None);
        let iframe = document()
            .create_element("iframe")
            .expect("iframe")
            .dyn_into::<HtmlIFrameElement>()
            .expect("iframe cast");
        host.container.append_child(&iframe).expect("attach iframe");
        let state = Rc::new(RefCell::new(PortalState::new()));
        register_portal(&iframe, &consumer, &state);

        // Forge a `hello` from this window — not the iframe's
        // `contentWindow` — transferring a port. Source identity, not
        // the presence of a port, must reject it.
        let channel = MessageChannel::new().expect("MessageChannel");
        let listener = PortListener::attach(&channel.port1());
        let env = Object::new();
        set_v1(&env, "hello");
        let transfer = Array::new();
        transfer.push(&channel.port2());
        window()
            .expect("window")
            .post_message_with_transfer(&env, "*", &transfer)
            .expect("post foreign hello");

        // `wait_for` polls for ~1s; an unmatched hello yields nothing.
        let ready = listener.wait_for("ready").await;
        assert!(
            ready.is_undefined(),
            "a hello from an unregistered source must not be answered",
        );
        assert!(
            state.borrow().port.is_none(),
            "no port should bind for an unmatched source",
        );
    }

    #[dialog_common::test]
    async fn it_routes_each_portals_hello_to_its_own_context() {
        // Two portals share the single page-level listener. Each reports
        // the `this` it received in its handshake; the listener must
        // route each hello to its own portal's context, not cross-wire.
        let host = FakeHost::install();
        let probe_a = WindowProbe::install("a");
        let probe_b = WindowProbe::install("b");
        let report = |tag: &str| {
            format!(
                "<script>tonk.ready.then(function(){{\
                   parent.postMessage({{__test:'{tag}',this:tonk.context.this}},'*');}});\
                 </script>"
            )
        };
        mount_portal(
            &host,
            &report("a"),
            Some("id:alpha"),
            Some("counter"),
            Some(DESCRIPTOR),
        );
        mount_portal(
            &host,
            &report("b"),
            Some("id:beta"),
            Some("counter"),
            Some(DESCRIPTOR),
        );

        let a = probe_a.wait().await;
        let b = probe_b.wait().await;
        assert_eq!(
            get_str(&a, "this").as_deref(),
            Some("id:alpha"),
            "portal A's hello must bind A's context",
        );
        assert_eq!(
            get_str(&b, "this").as_deref(),
            Some("id:beta"),
            "portal B's hello must bind B's context",
        );
    }

    #[dialog_common::test]
    async fn it_cancels_live_subscriptions_when_content_reloads() {
        let host = FakeHost::install();
        let content = "<script>tonk.subscribe().getReader().read();</script>";
        let portal = mount_portal(
            &host,
            content,
            Some("id:demo-counter"),
            Some("counter"),
            Some(DESCRIPTOR),
        );

        // Wait for the subscription to reach the host.
        for _ in 0..400 {
            if host.sub_tag().is_some() {
                break;
            }
            sleep(5).await;
        }
        assert!(host.sub_tag().is_some(), "subscribe should reach the host");
        assert!(!host.cancelled(), "not cancelled before reload");

        // New content reloads the iframe; `reload` clears the subs
        // first, dropping the `BridgeSub` and cancelling the host
        // subscription so the discarded window leaves no dangling SSE.
        portal
            .set_attribute("content", "<p>reloaded</p>")
            .expect("set content");
        for _ in 0..400 {
            if host.cancelled() {
                break;
            }
            sleep(5).await;
        }
        assert!(
            host.cancelled(),
            "a reload cancels the live host subscription",
        );
    }

    /// The credit-based fallback (`drain_body_to_port`, used when a browser
    /// can't transfer a `ReadableStream`) drains a response body into a
    /// `MessageChannel`: it hands over a `streamPort`, then posts `chunk`
    /// messages only as the consumer grants credit, and `close` at EOF. This
    /// drives that protocol by hand (standing in for the guest's
    /// ReadableStream) and asserts the bytes reassemble AND that no chunk
    /// arrives before credit is granted (backpressure holds).
    #[dialog_common::test]
    async fn it_drains_a_body_to_a_port_with_credit_backpressure() {
        use web_sys::{Response, ResponseInit};

        // A body that yields a few chunks. A Response from a string gives one
        // chunk; that's enough to exercise the credit gate + close.
        let init = ResponseInit::new();
        let resp = Response::new_with_opt_str_and_init(Some("sigil-bytes"), &init)
            .expect("construct response");
        let body = resp.body().expect("response body");

        // The "guest" side: the head envelope is posted to `client`, carrying
        // the transferred stream port.
        let head_channel = MessageChannel::new().expect("head channel");
        let host_to_guest = head_channel.port1();
        let guest_in = head_channel.port2();
        let head_listener = PortListener::attach(&guest_in);

        let head = Object::new();
        set_v1(&head, "fetch-result");
        let _ = Reflect::set(&head, &"id".into(), &JsValue::from_str("r1"));
        drain_body_to_port(&host_to_guest, head, &body);

        // Receive the head + the stream port.
        let received = head_listener.wait_for("fetch-result").await;
        let stream_port: MessagePort = Reflect::get(&received, &"streamPort".into())
            .expect("streamPort")
            .dyn_into()
            .expect("a MessagePort");
        let chunk_listener = PortListener::attach(&stream_port);

        // Backpressure: before granting credit, no chunk must arrive.
        sleep(30).await;
        assert!(
            chunk_listener.count() == 0,
            "no chunk may be sent before credit is granted",
        );

        // Grant credit and collect chunks until `close`.
        let mut collected: Vec<u8> = Vec::new();
        let mut closed = false;
        for _ in 0..50 {
            let grant = Object::new();
            let _ = Reflect::set(&grant, &"type".into(), &"credit".into());
            let _ = Reflect::set(&grant, &"n".into(), &JsValue::from_f64(1.0));
            stream_port.post_message(&grant).expect("grant credit");

            let msg = chunk_listener.wait_for_any().await;
            chunk_listener.clear();
            match get_str(&msg, "type").as_deref() {
                Some("chunk") => {
                    // Reconstruct the view over exactly the chunk's window, the
                    // way the guest does — the buffer is transferred whole but
                    // the bytes live in [byteOffset, byteOffset+byteLength).
                    let chunk = Reflect::get(&msg, &"chunk".into()).expect("chunk");
                    let offset = Reflect::get(&msg, &"byteOffset".into())
                        .ok()
                        .and_then(|v| v.as_f64())
                        .unwrap_or(0.0) as u32;
                    let length = Reflect::get(&msg, &"byteLength".into())
                        .ok()
                        .and_then(|v| v.as_f64())
                        .map(|n| n as u32)
                        .unwrap_or_else(|| js_sys::ArrayBuffer::from(chunk.clone()).byte_length());
                    let bytes =
                        js_sys::Uint8Array::new_with_byte_offset_and_length(&chunk, offset, length)
                            .to_vec();
                    collected.extend_from_slice(&bytes);
                }
                Some("close") => {
                    closed = true;
                    break;
                }
                other => panic!("unexpected stream message: {other:?}"),
            }
        }
        assert!(closed, "the stream must close after the body is drained");
        assert_eq!(
            String::from_utf8(collected).unwrap(),
            "sigil-bytes",
            "the drained bytes must reassemble to the body",
        );
    }

    /// The relay forwards the FULL request (method, headers, body), not just a
    /// GET path, so POST query/subscribe/transact route through it. This
    /// verifies `build_relayed_request` reconstructs each from the envelope.
    #[dialog_common::test]
    async fn it_builds_a_relayed_request_with_method_headers_body() {
        // Envelope shaped like what the guest posts: method + [[name,value]]
        // header pairs + a string body.
        let data = Object::new();
        let _ = Reflect::set(&data, &"method".into(), &"POST".into());
        let headers = Array::new();
        let pair = Array::new();
        pair.push(&"content-type".into());
        pair.push(&"application/json".into());
        headers.push(&pair);
        let _ = Reflect::set(&data, &"headers".into(), &headers);
        let _ = Reflect::set(&data, &"body".into(), &"{\"q\":1}".into());

        // `build_relayed_request` reconstructs the `RequestInit`; the path is
        // fetched separately as a bare string (see `handle_host_fetch`).
        // Materialize a real `Request` from the init to read the fields back.
        let init = build_relayed_request(&data).expect("request");
        let request =
            web_sys::Request::new_with_str_and_init("/api/repository/x/branch/main/query", &init)
                .expect("request from init");

        assert_eq!(request.method(), "POST");
        assert!(
            request
                .url()
                .ends_with("/api/repository/x/branch/main/query"),
            "url: {}",
            request.url(),
        );
        assert_eq!(
            request
                .headers()
                .get("content-type")
                .ok()
                .flatten()
                .as_deref(),
            Some("application/json"),
        );
        let body = JsFuture::from(request.text().expect("text()"))
            .await
            .expect("await body");
        assert_eq!(body.as_string().as_deref(), Some("{\"q\":1}"));
    }

    /// A relayed response must carry the URL the host fetched.
    ///
    /// The guest rebuilds the `Response` from the envelope, and
    /// `new Response(...)` cannot set `url` — it reads back `""` unless the
    /// shim restores it. An empty `url` is not cosmetic: reqwest's wasm
    /// client parses it while converting every response and throws
    /// `url parse`, so a Rust component fetching from inside a sealed guest
    /// (`<tonk-default-remote>` reading `/.well-known/tonk`) dies mid-await
    /// with the request already served.
    #[dialog_common::test]
    async fn it_gives_the_guest_a_response_carrying_the_fetched_url() {
        let host = FakeHost::install();
        let probe = WindowProbe::install("u");

        // Author code at the opaque origin fetches through the relayed
        // `window.fetch` and reports what `url` the response carries. The
        // path need not exist — a 404 is still a Response with a URL.
        let content = "<script>\
            fetch('/.well-known/tonk')\
              .then(function(r){parent.postMessage({__test:'u',url:r.url},'*');})\
              .catch(function(err){parent.postMessage({__test:'u',error:String(err)},'*');});\
            </script>";
        mount_portal(&host, content, None, None, None);

        let msg = probe.wait().await;
        assert!(
            !msg.is_undefined(),
            "author iframe should post the relayed response back",
        );
        assert!(
            Reflect::get(&msg, &"error".into())
                .ok()
                .filter(|v| !v.is_undefined())
                .is_none(),
            "the relayed fetch should not error; got: {:?}",
            Reflect::get(&msg, &"error".into()).ok(),
        );
        let url = get_str(&msg, "url").unwrap_or_default();
        assert!(
            url.ends_with("/.well-known/tonk"),
            "the rebuilt response must report the fetched URL, got {url:?}",
        );
    }

    fn title_message(kind: &str, text: &str) -> JsValue {
        let object = js_sys::Object::new();
        let _ = Reflect::set(
            &object,
            &JsValue::from_str("type"),
            &JsValue::from_str(kind),
        );
        let _ = Reflect::set(
            &object,
            &JsValue::from_str("text"),
            &JsValue::from_str(text),
        );
        object.into()
    }

    /// `title_text` accepts only a `{ type: "title", text }` shape with
    /// non-empty text; everything else yields `None`, so an unrelated
    /// message never retitles the tab and an unresolved `{name}` never
    /// blanks it. We assert the parse, not the assignment — performing
    /// it would retitle the test harness itself.
    #[dialog_common::test]
    async fn it_reads_text_only_from_a_title_message() {
        assert_eq!(
            title_text(&title_message("title", "Notes — Tonk")),
            Some("Notes — Tonk".to_owned()),
            "a title message with text should yield it"
        );
        assert_eq!(
            title_text(&title_message("title", "")),
            None,
            "an empty text should yield None"
        );
        assert_eq!(
            title_text(&title_message("other", "Notes — Tonk")),
            None,
            "a non-title message should yield None"
        );
        assert_eq!(
            title_text(&JsValue::from_str("not an object")),
            None,
            "a non-object payload should yield None"
        );
    }

    #[dialog_common::test]
    async fn it_parses_only_non_empty_registration_focus_tokens() {
        let message = Object::new();
        let _ = Reflect::set(&message, &"type".into(), &"register".into());
        let _ = Reflect::set(&message, &"reason".into(), &"needs-account".into());
        let _ = Reflect::set(&message, &"focusToken".into(), &"focus-1".into());
        assert_eq!(
            register_request(&message.clone().into()),
            Some(("needs-account".into(), Some("focus-1".into())))
        );

        let _ = Reflect::set(&message, &"focusToken".into(), &"".into());
        assert_eq!(
            register_request(&message.clone().into()),
            Some(("needs-account".into(), None)),
            "an empty token must never create a guest focus handle"
        );
        let _ = Reflect::set(&message, &"reason".into(), &"".into());
        assert_eq!(register_request(&message.into()), None);
    }

    #[dialog_common::test]
    async fn it_returns_registration_focus_through_the_request_port() {
        let state = Rc::new(RefCell::new(PortalState::new()));
        let channel = MessageChannel::new().expect("message channel");
        let listener = PortListener::attach(&channel.port2());
        let held = Rc::new(RefCell::new(None));
        let captured = held.clone();
        on_register(move |reason, focus_return| {
            assert_eq!(reason, "needs-account");
            *captured.borrow_mut() = focus_return;
        });

        let request = Object::new();
        let _ = Reflect::set(&request, &"type".into(), &"register".into());
        let _ = Reflect::set(&request, &"reason".into(), &"needs-account".into());
        let _ = Reflect::set(&request, &"focusToken".into(), &"focus-2".into());
        handle_register(&state, &channel.port1(), &request.into());
        held.borrow_mut()
            .take()
            .expect("focus return handle")
            .restore();

        let returned = listener.wait_for("register-focus").await;
        assert_eq!(get_str(&returned, "focusToken").as_deref(), Some("focus-2"));
    }

    /// `open_href` accepts only a well-formed `{type:"open", href}`. The
    /// dispatcher has already matched on `type`; re-checking here keeps the
    /// parse independently testable, as `title_text` does.
    #[dialog_common::test]
    async fn it_reads_href_only_from_an_open_message() {
        let message = js_sys::Object::new();
        let _ = js_sys::Reflect::set(
            &message,
            &JsValue::from_str("type"),
            &JsValue::from_str("open"),
        );
        let _ = js_sys::Reflect::set(
            &message,
            &JsValue::from_str("href"),
            &JsValue::from_str("https://example.com/"),
        );
        assert_eq!(
            open_href(&message.into()),
            Some("https://example.com/".to_owned()),
            "an open message with an href should yield it"
        );

        let empty = js_sys::Object::new();
        let _ = js_sys::Reflect::set(
            &empty,
            &JsValue::from_str("type"),
            &JsValue::from_str("open"),
        );
        let _ = js_sys::Reflect::set(&empty, &JsValue::from_str("href"), &JsValue::from_str(""));
        assert_eq!(
            open_href(&empty.into()),
            None,
            "an empty href should yield None"
        );

        let other = js_sys::Object::new();
        let _ = js_sys::Reflect::set(
            &other,
            &JsValue::from_str("type"),
            &JsValue::from_str("navigate"),
        );
        let _ = js_sys::Reflect::set(
            &other,
            &JsValue::from_str("href"),
            &JsValue::from_str("https://example.com/"),
        );
        assert_eq!(
            open_href(&other.into()),
            None,
            "a non-open message should yield None"
        );

        assert_eq!(
            open_href(&JsValue::from_str("not an object")),
            None,
            "a non-object payload should yield None"
        );
    }
}
