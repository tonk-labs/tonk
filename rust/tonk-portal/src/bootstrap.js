(function(){
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
    // parent.document.title. `<tab-title>` posts its text here and the
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
    // Carry a typed, pre-validated product event toward the top page. Every
    // parent relays the same string and the final sink validates it again.
    analytics:function(event){
      ready.then(function(){port.postMessage({v:1,type:"analytics",event:event});});
    },
    // Raise the registration dialog on the HOST page. Sharing needs an
    // account, and only the top page can run the ceremony: WebAuthn wants
    // a `window` and a user gesture, which the guest's opaque realm and
    // the service worker both lack. The guest posts the refusal class so
    // the host can word the prompt. Fire-and-forget (no response).
    register:function(reason,relay){
      var opener=document.activeElement;
      // Even an unfocused opener needs the ceremony's terminal event.
      var token=mint();
      if(token){ registerFocus.set(token,{opener:opener,relay:typeof relay==="function"?relay:null}); }
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
      case "context": tonk.context=env.context; return;
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
      case "custody-open": {
        var custody=registerFocus.get(env.focusToken);
        if(custody&&custody.relay){custody.relay(env.type);}
        window.dispatchEvent(new Event("tonk:custody-opened")); return;
      }
      case "custody-focus":
      case "register-focus": {
        var registration=registerFocus.get(env.focusToken);
        var opener=registration&&registration.opener;
        registerFocus.delete(env.focusToken);
        if(registration&&registration.relay){registration.relay(env.type);opener=null;}
        // The top-page ceremony has been torn down. Its opener may have been
        // replaced by a profile-fact render while the ceremony was running,
        // so signal the guest window even when that old node can no longer
        // take focus. Hub chrome uses this terminal event to clear its durable
        // linking marker and restore the spaces page in one step.
        window.dispatchEvent(new Event(env.type==="custody-focus" ? "tonk:custody-closed" : "tonk:registration-closed"));
        if(opener&&opener.isConnected&&!opener.matches(":disabled")){
          window.focus();
          opener.focus({preventScroll:true});
        }
        return;
      }
      case "register-focus-discard": {
        var discarded=registerFocus.get(env.focusToken);
        registerFocus.delete(env.focusToken);
        if(discarded&&discarded.relay){discarded.relay(env.type);}
        return;
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

  // The product-owned agent prompt lives in rendered guest markup, outside
  // the Rust component tree. Observe only its reviewed copy control and send
  // a content-free lifecycle; never read or forward the copied value.
  var agentCopies=new WeakMap();
  function eventNode(event,selector){
    var nodes=event.composedPath?event.composedPath():[event.target];
    for(var i=0;i<nodes.length;i++){
      var node=nodes[i];
      if(node&&node.matches&&node.matches(selector)) return node;
    }
    return null;
  }
  function productAttemptId(){
    try{
      var bytes=new Uint8Array(16); crypto.getRandomValues(bytes);
      return Array.from(bytes,function(value){return value.toString(16).padStart(2,"0");}).join("");
    }catch(e){return null;}
  }
  function promptEvent(attempt,phase,result,failure){
    var props={schema_version:1,journey:"handoff",action:"copy_agent_prompt",
      phase:phase,stage:phase==="started"?"intent":"clipboard",
      surface:"workspace",trigger:"user",attempt_id:attempt.id};
    if(phase==="finished"){
      props.duration_ms=Math.min(600000,Math.max(0,Math.floor(performance.now()-attempt.started)));
      props.result=result;
      if(failure) props.failure_kind=failure;
    }
    tonk.analytics(JSON.stringify({name:"product_event",props:props}));
  }
  document.addEventListener("click",function(event){
    var target=eventNode(event,".agent-prompt__copy");
    if(!target||target.disabled||target.isCopying||agentCopies.has(target)) return;
    var id=productAttemptId(); if(!id) return;
    var attempt={id:id,started:performance.now()};
    agentCopies.set(target,attempt); promptEvent(attempt,"started");
  },true);
  document.addEventListener("wa-copy",function(event){
    var target=eventNode(event,".agent-prompt__copy");
    var attempt=target&&agentCopies.get(target); if(!attempt) return;
    agentCopies.delete(target); promptEvent(attempt,"finished","success");
  });
  document.addEventListener("wa-error",function(event){
    var target=eventNode(event,".agent-prompt__copy");
    var attempt=target&&agentCopies.get(target); if(!attempt) return;
    agentCopies.delete(target);
    promptEvent(attempt,"finished","retryable_failure","unknown");
  });

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
})();
