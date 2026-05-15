var g0=Object.create;var Vf=Object.defineProperty;var y0=Object.getOwnPropertyDescriptor;var b0=Object.getOwnPropertyNames;var _0=Object.getPrototypeOf,S0=Object.prototype.hasOwnProperty;var Ee=(t,e)=>()=>(e||t((e={exports:{}}).exports,e),e.exports);var x0=(t,e,l,a)=>{if(e&&typeof e=="object"||typeof e=="function")for(let n of b0(e))!S0.call(t,n)&&n!==l&&Vf(t,n,{get:()=>e[n],enumerable:!(a=y0(e,n))||a.enumerable});return t};var j=(t,e,l)=>(l=t!=null?g0(_0(t)):{},x0(e||!t||!t.__esModule?Vf(l,"default",{value:t,enumerable:!0}):l,t));var lr=Ee(lt=>{"use strict";function Pi(t,e){var l=t.length;t.push(e);t:for(;0<l;){var a=l-1>>>1,n=t[a];if(0<uu(n,e))t[a]=e,t[l]=n,l=a;else break t}}function Te(t){return t.length===0?null:t[0]}function ou(t){if(t.length===0)return null;var e=t[0],l=t.pop();if(l!==e){t[0]=l;t:for(var a=0,n=t.length,u=n>>>1;a<u;){var i=2*(a+1)-1,o=t[i],c=i+1,s=t[c];if(0>uu(o,l))c<n&&0>uu(s,o)?(t[a]=s,t[c]=l,a=c):(t[a]=o,t[i]=l,a=i);else if(c<n&&0>uu(s,l))t[a]=s,t[c]=l,a=c;else break t}}return e}function uu(t,e){var l=t.sortIndex-e.sortIndex;return l!==0?l:t.id-e.id}lt.unstable_now=void 0;typeof performance=="object"&&typeof performance.now=="function"?(Kf=performance,lt.unstable_now=function(){return Kf.now()}):($i=Date,Jf=$i.now(),lt.unstable_now=function(){return $i.now()-Jf});var Kf,$i,Jf,Ue=[],el=[],z0=1,ie=null,Nt=3,to=!1,$a=!1,Fa=!1,eo=!1,Ff=typeof setTimeout=="function"?setTimeout:null,If=typeof clearTimeout=="function"?clearTimeout:null,Wf=typeof setImmediate<"u"?setImmediate:null;function iu(t){for(var e=Te(el);e!==null;){if(e.callback===null)ou(el);else if(e.startTime<=t)ou(el),e.sortIndex=e.expirationTime,Pi(Ue,e);else break;e=Te(el)}}function lo(t){if(Fa=!1,iu(t),!$a)if(Te(Ue)!==null)$a=!0,ea||(ea=!0,ta());else{var e=Te(el);e!==null&&ao(lo,e.startTime-t)}}var ea=!1,Ia=-1,Pf=5,tr=-1;function er(){return eo?!0:!(lt.unstable_now()-tr<Pf)}function Fi(){if(eo=!1,ea){var t=lt.unstable_now();tr=t;var e=!0;try{t:{$a=!1,Fa&&(Fa=!1,If(Ia),Ia=-1),to=!0;var l=Nt;try{e:{for(iu(t),ie=Te(Ue);ie!==null&&!(ie.expirationTime>t&&er());){var a=ie.callback;if(typeof a=="function"){ie.callback=null,Nt=ie.priorityLevel;var n=a(ie.expirationTime<=t);if(t=lt.unstable_now(),typeof n=="function"){ie.callback=n,iu(t),e=!0;break e}ie===Te(Ue)&&ou(Ue),iu(t)}else ou(Ue);ie=Te(Ue)}if(ie!==null)e=!0;else{var u=Te(el);u!==null&&ao(lo,u.startTime-t),e=!1}}break t}finally{ie=null,Nt=l,to=!1}e=void 0}}finally{e?ta():ea=!1}}}var ta;typeof Wf=="function"?ta=function(){Wf(Fi)}:typeof MessageChannel<"u"?(Ii=new MessageChannel,$f=Ii.port2,Ii.port1.onmessage=Fi,ta=function(){$f.postMessage(null)}):ta=function(){Ff(Fi,0)};var Ii,$f;function ao(t,e){Ia=Ff(function(){t(lt.unstable_now())},e)}lt.unstable_IdlePriority=5;lt.unstable_ImmediatePriority=1;lt.unstable_LowPriority=4;lt.unstable_NormalPriority=3;lt.unstable_Profiling=null;lt.unstable_UserBlockingPriority=2;lt.unstable_cancelCallback=function(t){t.callback=null};lt.unstable_forceFrameRate=function(t){0>t||125<t?console.error("forceFrameRate takes a positive int between 0 and 125, forcing frame rates higher than 125 fps is not supported"):Pf=0<t?Math.floor(1e3/t):5};lt.unstable_getCurrentPriorityLevel=function(){return Nt};lt.unstable_next=function(t){switch(Nt){case 1:case 2:case 3:var e=3;break;default:e=Nt}var l=Nt;Nt=e;try{return t()}finally{Nt=l}};lt.unstable_requestPaint=function(){eo=!0};lt.unstable_runWithPriority=function(t,e){switch(t){case 1:case 2:case 3:case 4:case 5:break;default:t=3}var l=Nt;Nt=t;try{return e()}finally{Nt=l}};lt.unstable_scheduleCallback=function(t,e,l){var a=lt.unstable_now();switch(typeof l=="object"&&l!==null?(l=l.delay,l=typeof l=="number"&&0<l?a+l:a):l=a,t){case 1:var n=-1;break;case 2:n=250;break;case 5:n=1073741823;break;case 4:n=1e4;break;default:n=5e3}return n=l+n,t={id:z0++,callback:e,priorityLevel:t,startTime:l,expirationTime:n,sortIndex:-1},l>a?(t.sortIndex=l,Pi(el,t),Te(Ue)===null&&t===Te(el)&&(Fa?(If(Ia),Ia=-1):Fa=!0,ao(lo,l-a))):(t.sortIndex=n,Pi(Ue,t),$a||to||($a=!0,ea||(ea=!0,ta()))),t};lt.unstable_shouldYield=er;lt.unstable_wrapCallback=function(t){var e=Nt;return function(){var l=Nt;Nt=e;try{return t.apply(this,arguments)}finally{Nt=l}}}});var nr=Ee((Jg,ar)=>{"use strict";ar.exports=lr()});var hr=Ee(U=>{"use strict";var io=Symbol.for("react.transitional.element"),E0=Symbol.for("react.portal"),T0=Symbol.for("react.fragment"),A0=Symbol.for("react.strict_mode"),M0=Symbol.for("react.profiler"),D0=Symbol.for("react.consumer"),q0=Symbol.for("react.context"),w0=Symbol.for("react.forward_ref"),O0=Symbol.for("react.suspense"),N0=Symbol.for("react.memo"),fr=Symbol.for("react.lazy"),C0=Symbol.for("react.activity"),ur=Symbol.iterator;function R0(t){return t===null||typeof t!="object"?null:(t=ur&&t[ur]||t["@@iterator"],typeof t=="function"?t:null)}var rr={isMounted:function(){return!1},enqueueForceUpdate:function(){},enqueueReplaceState:function(){},enqueueSetState:function(){}},sr=Object.assign,dr={};function aa(t,e,l){this.props=t,this.context=e,this.refs=dr,this.updater=l||rr}aa.prototype.isReactComponent={};aa.prototype.setState=function(t,e){if(typeof t!="object"&&typeof t!="function"&&t!=null)throw Error("takes an object of state variables to update or a function which returns an object of state variables.");this.updater.enqueueSetState(this,t,e,"setState")};aa.prototype.forceUpdate=function(t){this.updater.enqueueForceUpdate(this,t,"forceUpdate")};function mr(){}mr.prototype=aa.prototype;function oo(t,e,l){this.props=t,this.context=e,this.refs=dr,this.updater=l||rr}var co=oo.prototype=new mr;co.constructor=oo;sr(co,aa.prototype);co.isPureReactComponent=!0;var ir=Array.isArray;function uo(){}var I={H:null,A:null,T:null,S:null},pr=Object.prototype.hasOwnProperty;function fo(t,e,l){var a=l.ref;return{$$typeof:io,type:t,key:e,ref:a!==void 0?a:null,props:l}}function U0(t,e){return fo(t.type,e,t.props)}function ro(t){return typeof t=="object"&&t!==null&&t.$$typeof===io}function H0(t){var e={"=":"=0",":":"=2"};return"$"+t.replace(/[=:]/g,function(l){return e[l]})}var or=/\/+/g;function no(t,e){return typeof t=="object"&&t!==null&&t.key!=null?H0(""+t.key):e.toString(36)}function k0(t){switch(t.status){case"fulfilled":return t.value;case"rejected":throw t.reason;default:switch(typeof t.status=="string"?t.then(uo,uo):(t.status="pending",t.then(function(e){t.status==="pending"&&(t.status="fulfilled",t.value=e)},function(e){t.status==="pending"&&(t.status="rejected",t.reason=e)})),t.status){case"fulfilled":return t.value;case"rejected":throw t.reason}}throw t}function la(t,e,l,a,n){var u=typeof t;(u==="undefined"||u==="boolean")&&(t=null);var i=!1;if(t===null)i=!0;else switch(u){case"bigint":case"string":case"number":i=!0;break;case"object":switch(t.$$typeof){case io:case E0:i=!0;break;case fr:return i=t._init,la(i(t._payload),e,l,a,n)}}if(i)return n=n(t),i=a===""?"."+no(t,0):a,ir(n)?(l="",i!=null&&(l=i.replace(or,"$&/")+"/"),la(n,e,l,"",function(s){return s})):n!=null&&(ro(n)&&(n=U0(n,l+(n.key==null||t&&t.key===n.key?"":(""+n.key).replace(or,"$&/")+"/")+i)),e.push(n)),1;i=0;var o=a===""?".":a+":";if(ir(t))for(var c=0;c<t.length;c++)a=t[c],u=o+no(a,c),i+=la(a,e,l,u,n);else if(c=R0(t),typeof c=="function")for(t=c.call(t),c=0;!(a=t.next()).done;)a=a.value,u=o+no(a,c++),i+=la(a,e,l,u,n);else if(u==="object"){if(typeof t.then=="function")return la(k0(t),e,l,a,n);throw e=String(t),Error("Objects are not valid as a React child (found: "+(e==="[object Object]"?"object with keys {"+Object.keys(t).join(", ")+"}":e)+"). If you meant to render a collection of children, use an array instead.")}return i}function cu(t,e,l){if(t==null)return t;var a=[],n=0;return la(t,a,"","",function(u){return e.call(l,u,n++)}),a}function B0(t){if(t._status===-1){var e=t._result;e=e(),e.then(function(l){(t._status===0||t._status===-1)&&(t._status=1,t._result=l)},function(l){(t._status===0||t._status===-1)&&(t._status=2,t._result=l)}),t._status===-1&&(t._status=0,t._result=e)}if(t._status===1)return t._result.default;throw t._result}var cr=typeof reportError=="function"?reportError:function(t){if(typeof window=="object"&&typeof window.ErrorEvent=="function"){var e=new window.ErrorEvent("error",{bubbles:!0,cancelable:!0,message:typeof t=="object"&&t!==null&&typeof t.message=="string"?String(t.message):String(t),error:t});if(!window.dispatchEvent(e))return}else if(typeof process=="object"&&typeof process.emit=="function"){process.emit("uncaughtException",t);return}console.error(t)},Y0={map:cu,forEach:function(t,e,l){cu(t,function(){e.apply(this,arguments)},l)},count:function(t){var e=0;return cu(t,function(){e++}),e},toArray:function(t){return cu(t,function(e){return e})||[]},only:function(t){if(!ro(t))throw Error("React.Children.only expected to receive a single React element child.");return t}};U.Activity=C0;U.Children=Y0;U.Component=aa;U.Fragment=T0;U.Profiler=M0;U.PureComponent=oo;U.StrictMode=A0;U.Suspense=O0;U.__CLIENT_INTERNALS_DO_NOT_USE_OR_WARN_USERS_THEY_CANNOT_UPGRADE=I;U.__COMPILER_RUNTIME={__proto__:null,c:function(t){return I.H.useMemoCache(t)}};U.cache=function(t){return function(){return t.apply(null,arguments)}};U.cacheSignal=function(){return null};U.cloneElement=function(t,e,l){if(t==null)throw Error("The argument must be a React element, but you passed "+t+".");var a=sr({},t.props),n=t.key;if(e!=null)for(u in e.key!==void 0&&(n=""+e.key),e)!pr.call(e,u)||u==="key"||u==="__self"||u==="__source"||u==="ref"&&e.ref===void 0||(a[u]=e[u]);var u=arguments.length-2;if(u===1)a.children=l;else if(1<u){for(var i=Array(u),o=0;o<u;o++)i[o]=arguments[o+2];a.children=i}return fo(t.type,n,a)};U.createContext=function(t){return t={$$typeof:q0,_currentValue:t,_currentValue2:t,_threadCount:0,Provider:null,Consumer:null},t.Provider=t,t.Consumer={$$typeof:D0,_context:t},t};U.createElement=function(t,e,l){var a,n={},u=null;if(e!=null)for(a in e.key!==void 0&&(u=""+e.key),e)pr.call(e,a)&&a!=="key"&&a!=="__self"&&a!=="__source"&&(n[a]=e[a]);var i=arguments.length-2;if(i===1)n.children=l;else if(1<i){for(var o=Array(i),c=0;c<i;c++)o[c]=arguments[c+2];n.children=o}if(t&&t.defaultProps)for(a in i=t.defaultProps,i)n[a]===void 0&&(n[a]=i[a]);return fo(t,u,n)};U.createRef=function(){return{current:null}};U.forwardRef=function(t){return{$$typeof:w0,render:t}};U.isValidElement=ro;U.lazy=function(t){return{$$typeof:fr,_payload:{_status:-1,_result:t},_init:B0}};U.memo=function(t,e){return{$$typeof:N0,type:t,compare:e===void 0?null:e}};U.startTransition=function(t){var e=I.T,l={};I.T=l;try{var a=t(),n=I.S;n!==null&&n(l,a),typeof a=="object"&&a!==null&&typeof a.then=="function"&&a.then(uo,cr)}catch(u){cr(u)}finally{e!==null&&l.types!==null&&(e.types=l.types),I.T=e}};U.unstable_useCacheRefresh=function(){return I.H.useCacheRefresh()};U.use=function(t){return I.H.use(t)};U.useActionState=function(t,e,l){return I.H.useActionState(t,e,l)};U.useCallback=function(t,e){return I.H.useCallback(t,e)};U.useContext=function(t){return I.H.useContext(t)};U.useDebugValue=function(){};U.useDeferredValue=function(t,e){return I.H.useDeferredValue(t,e)};U.useEffect=function(t,e){return I.H.useEffect(t,e)};U.useEffectEvent=function(t){return I.H.useEffectEvent(t)};U.useId=function(){return I.H.useId()};U.useImperativeHandle=function(t,e,l){return I.H.useImperativeHandle(t,e,l)};U.useInsertionEffect=function(t,e){return I.H.useInsertionEffect(t,e)};U.useLayoutEffect=function(t,e){return I.H.useLayoutEffect(t,e)};U.useMemo=function(t,e){return I.H.useMemo(t,e)};U.useOptimistic=function(t,e){return I.H.useOptimistic(t,e)};U.useReducer=function(t,e,l){return I.H.useReducer(t,e,l)};U.useRef=function(t){return I.H.useRef(t)};U.useState=function(t){return I.H.useState(t)};U.useSyncExternalStore=function(t,e,l){return I.H.useSyncExternalStore(t,e,l)};U.useTransition=function(){return I.H.useTransition()};U.version="19.2.5"});var Yt=Ee(($g,vr)=>{"use strict";vr.exports=hr()});var yr=Ee(Rt=>{"use strict";var G0=Yt();function gr(t){var e="https://react.dev/errors/"+t;if(1<arguments.length){e+="?args[]="+encodeURIComponent(arguments[1]);for(var l=2;l<arguments.length;l++)e+="&args[]="+encodeURIComponent(arguments[l])}return"Minified React error #"+t+"; visit "+e+" for the full message or use the non-minified dev environment for full errors and additional helpful warnings."}function ll(){}var Ct={d:{f:ll,r:function(){throw Error(gr(522))},D:ll,C:ll,L:ll,m:ll,X:ll,S:ll,M:ll},p:0,findDOMNode:null},X0=Symbol.for("react.portal");function L0(t,e,l){var a=3<arguments.length&&arguments[3]!==void 0?arguments[3]:null;return{$$typeof:X0,key:a==null?null:""+a,children:t,containerInfo:e,implementation:l}}var Pa=G0.__CLIENT_INTERNALS_DO_NOT_USE_OR_WARN_USERS_THEY_CANNOT_UPGRADE;function fu(t,e){if(t==="font")return"";if(typeof e=="string")return e==="use-credentials"?e:""}Rt.__DOM_INTERNALS_DO_NOT_USE_OR_WARN_USERS_THEY_CANNOT_UPGRADE=Ct;Rt.createPortal=function(t,e){var l=2<arguments.length&&arguments[2]!==void 0?arguments[2]:null;if(!e||e.nodeType!==1&&e.nodeType!==9&&e.nodeType!==11)throw Error(gr(299));return L0(t,e,null,l)};Rt.flushSync=function(t){var e=Pa.T,l=Ct.p;try{if(Pa.T=null,Ct.p=2,t)return t()}finally{Pa.T=e,Ct.p=l,Ct.d.f()}};Rt.preconnect=function(t,e){typeof t=="string"&&(e?(e=e.crossOrigin,e=typeof e=="string"?e==="use-credentials"?e:"":void 0):e=null,Ct.d.C(t,e))};Rt.prefetchDNS=function(t){typeof t=="string"&&Ct.d.D(t)};Rt.preinit=function(t,e){if(typeof t=="string"&&e&&typeof e.as=="string"){var l=e.as,a=fu(l,e.crossOrigin),n=typeof e.integrity=="string"?e.integrity:void 0,u=typeof e.fetchPriority=="string"?e.fetchPriority:void 0;l==="style"?Ct.d.S(t,typeof e.precedence=="string"?e.precedence:void 0,{crossOrigin:a,integrity:n,fetchPriority:u}):l==="script"&&Ct.d.X(t,{crossOrigin:a,integrity:n,fetchPriority:u,nonce:typeof e.nonce=="string"?e.nonce:void 0})}};Rt.preinitModule=function(t,e){if(typeof t=="string")if(typeof e=="object"&&e!==null){if(e.as==null||e.as==="script"){var l=fu(e.as,e.crossOrigin);Ct.d.M(t,{crossOrigin:l,integrity:typeof e.integrity=="string"?e.integrity:void 0,nonce:typeof e.nonce=="string"?e.nonce:void 0})}}else e==null&&Ct.d.M(t)};Rt.preload=function(t,e){if(typeof t=="string"&&typeof e=="object"&&e!==null&&typeof e.as=="string"){var l=e.as,a=fu(l,e.crossOrigin);Ct.d.L(t,l,{crossOrigin:a,integrity:typeof e.integrity=="string"?e.integrity:void 0,nonce:typeof e.nonce=="string"?e.nonce:void 0,type:typeof e.type=="string"?e.type:void 0,fetchPriority:typeof e.fetchPriority=="string"?e.fetchPriority:void 0,referrerPolicy:typeof e.referrerPolicy=="string"?e.referrerPolicy:void 0,imageSrcSet:typeof e.imageSrcSet=="string"?e.imageSrcSet:void 0,imageSizes:typeof e.imageSizes=="string"?e.imageSizes:void 0,media:typeof e.media=="string"?e.media:void 0})}};Rt.preloadModule=function(t,e){if(typeof t=="string")if(e){var l=fu(e.as,e.crossOrigin);Ct.d.m(t,{as:typeof e.as=="string"&&e.as!=="script"?e.as:void 0,crossOrigin:l,integrity:typeof e.integrity=="string"?e.integrity:void 0})}else Ct.d.m(t)};Rt.requestFormReset=function(t){Ct.d.r(t)};Rt.unstable_batchedUpdates=function(t,e){return t(e)};Rt.useFormState=function(t,e,l){return Pa.H.useFormState(t,e,l)};Rt.useFormStatus=function(){return Pa.H.useHostTransitionStatus()};Rt.version="19.2.5"});var so=Ee((Ig,_r)=>{"use strict";function br(){if(!(typeof __REACT_DEVTOOLS_GLOBAL_HOOK__>"u"||typeof __REACT_DEVTOOLS_GLOBAL_HOOK__.checkDCE!="function"))try{__REACT_DEVTOOLS_GLOBAL_HOOK__.checkDCE(br)}catch(t){console.error(t)}}br(),_r.exports=yr()});var Np=Ee(Ri=>{"use strict";var St=nr(),Vs=Yt(),j0=so();function y(t){var e="https://react.dev/errors/"+t;if(1<arguments.length){e+="?args[]="+encodeURIComponent(arguments[1]);for(var l=2;l<arguments.length;l++)e+="&args[]="+encodeURIComponent(arguments[l])}return"Minified React error #"+t+"; visit "+e+" for the full message or use the non-minified dev environment for full errors and additional helpful warnings."}function Ks(t){return!(!t||t.nodeType!==1&&t.nodeType!==9&&t.nodeType!==11)}function Gn(t){var e=t,l=t;if(t.alternate)for(;e.return;)e=e.return;else{t=e;do e=t,e.flags&4098&&(l=e.return),t=e.return;while(t)}return e.tag===3?l:null}function Js(t){if(t.tag===13){var e=t.memoizedState;if(e===null&&(t=t.alternate,t!==null&&(e=t.memoizedState)),e!==null)return e.dehydrated}return null}function Ws(t){if(t.tag===31){var e=t.memoizedState;if(e===null&&(t=t.alternate,t!==null&&(e=t.memoizedState)),e!==null)return e.dehydrated}return null}function Sr(t){if(Gn(t)!==t)throw Error(y(188))}function Q0(t){var e=t.alternate;if(!e){if(e=Gn(t),e===null)throw Error(y(188));return e!==t?null:t}for(var l=t,a=e;;){var n=l.return;if(n===null)break;var u=n.alternate;if(u===null){if(a=n.return,a!==null){l=a;continue}break}if(n.child===u.child){for(u=n.child;u;){if(u===l)return Sr(n),t;if(u===a)return Sr(n),e;u=u.sibling}throw Error(y(188))}if(l.return!==a.return)l=n,a=u;else{for(var i=!1,o=n.child;o;){if(o===l){i=!0,l=n,a=u;break}if(o===a){i=!0,a=n,l=u;break}o=o.sibling}if(!i){for(o=u.child;o;){if(o===l){i=!0,l=u,a=n;break}if(o===a){i=!0,a=u,l=n;break}o=o.sibling}if(!i)throw Error(y(189))}}if(l.alternate!==a)throw Error(y(190))}if(l.tag!==3)throw Error(y(188));return l.stateNode.current===l?t:e}function $s(t){var e=t.tag;if(e===5||e===26||e===27||e===6)return t;for(t=t.child;t!==null;){if(e=$s(t),e!==null)return e;t=t.sibling}return null}var et=Object.assign,Z0=Symbol.for("react.element"),ru=Symbol.for("react.transitional.element"),cn=Symbol.for("react.portal"),fa=Symbol.for("react.fragment"),Fs=Symbol.for("react.strict_mode"),Vo=Symbol.for("react.profiler"),Is=Symbol.for("react.consumer"),je=Symbol.for("react.context"),Xc=Symbol.for("react.forward_ref"),Ko=Symbol.for("react.suspense"),Jo=Symbol.for("react.suspense_list"),Lc=Symbol.for("react.memo"),al=Symbol.for("react.lazy");Symbol.for("react.scope");var Wo=Symbol.for("react.activity");Symbol.for("react.legacy_hidden");Symbol.for("react.tracing_marker");var V0=Symbol.for("react.memo_cache_sentinel");Symbol.for("react.view_transition");var xr=Symbol.iterator;function tn(t){return t===null||typeof t!="object"?null:(t=xr&&t[xr]||t["@@iterator"],typeof t=="function"?t:null)}var K0=Symbol.for("react.client.reference");function $o(t){if(t==null)return null;if(typeof t=="function")return t.$$typeof===K0?null:t.displayName||t.name||null;if(typeof t=="string")return t;switch(t){case fa:return"Fragment";case Vo:return"Profiler";case Fs:return"StrictMode";case Ko:return"Suspense";case Jo:return"SuspenseList";case Wo:return"Activity"}if(typeof t=="object")switch(t.$$typeof){case cn:return"Portal";case je:return t.displayName||"Context";case Is:return(t._context.displayName||"Context")+".Consumer";case Xc:var e=t.render;return t=t.displayName,t||(t=e.displayName||e.name||"",t=t!==""?"ForwardRef("+t+")":"ForwardRef"),t;case Lc:return e=t.displayName||null,e!==null?e:$o(t.type)||"Memo";case al:e=t._payload,t=t._init;try{return $o(t(e))}catch{}}return null}var fn=Array.isArray,C=Vs.__CLIENT_INTERNALS_DO_NOT_USE_OR_WARN_USERS_THEY_CANNOT_UPGRADE,Z=j0.__DOM_INTERNALS_DO_NOT_USE_OR_WARN_USERS_THEY_CANNOT_UPGRADE,Hl={pending:!1,data:null,method:null,action:null},Fo=[],ra=-1;function we(t){return{current:t}}function At(t){0>ra||(t.current=Fo[ra],Fo[ra]=null,ra--)}function F(t,e){ra++,Fo[ra]=t.current,t.current=e}var qe=we(null),An=we(null),pl=we(null),ju=we(null);function Qu(t,e){switch(F(pl,e),F(An,t),F(qe,null),e.nodeType){case 9:case 11:t=(t=e.documentElement)&&(t=t.namespaceURI)?qs(t):0;break;default:if(t=e.tagName,e=e.namespaceURI)e=qs(e),t=yp(e,t);else switch(t){case"svg":t=1;break;case"math":t=2;break;default:t=0}}At(qe),F(qe,t)}function Da(){At(qe),At(An),At(pl)}function Io(t){t.memoizedState!==null&&F(ju,t);var e=qe.current,l=yp(e,t.type);e!==l&&(F(An,t),F(qe,l))}function Zu(t){An.current===t&&(At(qe),At(An)),ju.current===t&&(At(ju),kn._currentValue=Hl)}var mo,zr;function Nl(t){if(mo===void 0)try{throw Error()}catch(l){var e=l.stack.trim().match(/\n( *(at )?)/);mo=e&&e[1]||"",zr=-1<l.stack.indexOf(`
    at`)?" (<anonymous>)":-1<l.stack.indexOf("@")?"@unknown:0:0":""}return`
`+mo+t+zr}var po=!1;function ho(t,e){if(!t||po)return"";po=!0;var l=Error.prepareStackTrace;Error.prepareStackTrace=void 0;try{var a={DetermineComponentFrameRoot:function(){try{if(e){var v=function(){throw Error()};if(Object.defineProperty(v.prototype,"props",{set:function(){throw Error()}}),typeof Reflect=="object"&&Reflect.construct){try{Reflect.construct(v,[])}catch(p){var d=p}Reflect.construct(t,[],v)}else{try{v.call()}catch(p){d=p}t.call(v.prototype)}}else{try{throw Error()}catch(p){d=p}(v=t())&&typeof v.catch=="function"&&v.catch(function(){})}}catch(p){if(p&&d&&typeof p.stack=="string")return[p.stack,d.stack]}return[null,null]}};a.DetermineComponentFrameRoot.displayName="DetermineComponentFrameRoot";var n=Object.getOwnPropertyDescriptor(a.DetermineComponentFrameRoot,"name");n&&n.configurable&&Object.defineProperty(a.DetermineComponentFrameRoot,"name",{value:"DetermineComponentFrameRoot"});var u=a.DetermineComponentFrameRoot(),i=u[0],o=u[1];if(i&&o){var c=i.split(`
`),s=o.split(`
`);for(n=a=0;a<c.length&&!c[a].includes("DetermineComponentFrameRoot");)a++;for(;n<s.length&&!s[n].includes("DetermineComponentFrameRoot");)n++;if(a===c.length||n===s.length)for(a=c.length-1,n=s.length-1;1<=a&&0<=n&&c[a]!==s[n];)n--;for(;1<=a&&0<=n;a--,n--)if(c[a]!==s[n]){if(a!==1||n!==1)do if(a--,n--,0>n||c[a]!==s[n]){var h=`
`+c[a].replace(" at new "," at ");return t.displayName&&h.includes("<anonymous>")&&(h=h.replace("<anonymous>",t.displayName)),h}while(1<=a&&0<=n);break}}}finally{po=!1,Error.prepareStackTrace=l}return(l=t?t.displayName||t.name:"")?Nl(l):""}function J0(t,e){switch(t.tag){case 26:case 27:case 5:return Nl(t.type);case 16:return Nl("Lazy");case 13:return t.child!==e&&e!==null?Nl("Suspense Fallback"):Nl("Suspense");case 19:return Nl("SuspenseList");case 0:case 15:return ho(t.type,!1);case 11:return ho(t.type.render,!1);case 1:return ho(t.type,!0);case 31:return Nl("Activity");default:return""}}function Er(t){try{var e="",l=null;do e+=J0(t,l),l=t,t=t.return;while(t);return e}catch(a){return`
Error generating stack: `+a.message+`
`+a.stack}}var Po=Object.prototype.hasOwnProperty,jc=St.unstable_scheduleCallback,vo=St.unstable_cancelCallback,W0=St.unstable_shouldYield,$0=St.unstable_requestPaint,Pt=St.unstable_now,F0=St.unstable_getCurrentPriorityLevel,Ps=St.unstable_ImmediatePriority,td=St.unstable_UserBlockingPriority,Vu=St.unstable_NormalPriority,I0=St.unstable_LowPriority,ed=St.unstable_IdlePriority,P0=St.log,th=St.unstable_setDisableYieldValue,Xn=null,te=null;function fl(t){if(typeof P0=="function"&&th(t),te&&typeof te.setStrictMode=="function")try{te.setStrictMode(Xn,t)}catch{}}var ee=Math.clz32?Math.clz32:ah,eh=Math.log,lh=Math.LN2;function ah(t){return t>>>=0,t===0?32:31-(eh(t)/lh|0)|0}var su=256,du=262144,mu=4194304;function Cl(t){var e=t&42;if(e!==0)return e;switch(t&-t){case 1:return 1;case 2:return 2;case 4:return 4;case 8:return 8;case 16:return 16;case 32:return 32;case 64:return 64;case 128:return 128;case 256:case 512:case 1024:case 2048:case 4096:case 8192:case 16384:case 32768:case 65536:case 131072:return t&261888;case 262144:case 524288:case 1048576:case 2097152:return t&3932160;case 4194304:case 8388608:case 16777216:case 33554432:return t&62914560;case 67108864:return 67108864;case 134217728:return 134217728;case 268435456:return 268435456;case 536870912:return 536870912;case 1073741824:return 0;default:return t}}function yi(t,e,l){var a=t.pendingLanes;if(a===0)return 0;var n=0,u=t.suspendedLanes,i=t.pingedLanes;t=t.warmLanes;var o=a&134217727;return o!==0?(a=o&~u,a!==0?n=Cl(a):(i&=o,i!==0?n=Cl(i):l||(l=o&~t,l!==0&&(n=Cl(l))))):(o=a&~u,o!==0?n=Cl(o):i!==0?n=Cl(i):l||(l=a&~t,l!==0&&(n=Cl(l)))),n===0?0:e!==0&&e!==n&&!(e&u)&&(u=n&-n,l=e&-e,u>=l||u===32&&(l&4194048)!==0)?e:n}function Ln(t,e){return(t.pendingLanes&~(t.suspendedLanes&~t.pingedLanes)&e)===0}function nh(t,e){switch(t){case 1:case 2:case 4:case 8:case 64:return e+250;case 16:case 32:case 128:case 256:case 512:case 1024:case 2048:case 4096:case 8192:case 16384:case 32768:case 65536:case 131072:case 262144:case 524288:case 1048576:case 2097152:return e+5e3;case 4194304:case 8388608:case 16777216:case 33554432:return-1;case 67108864:case 134217728:case 268435456:case 536870912:case 1073741824:return-1;default:return-1}}function ld(){var t=mu;return mu<<=1,!(mu&62914560)&&(mu=4194304),t}function go(t){for(var e=[],l=0;31>l;l++)e.push(t);return e}function jn(t,e){t.pendingLanes|=e,e!==268435456&&(t.suspendedLanes=0,t.pingedLanes=0,t.warmLanes=0)}function uh(t,e,l,a,n,u){var i=t.pendingLanes;t.pendingLanes=l,t.suspendedLanes=0,t.pingedLanes=0,t.warmLanes=0,t.expiredLanes&=l,t.entangledLanes&=l,t.errorRecoveryDisabledLanes&=l,t.shellSuspendCounter=0;var o=t.entanglements,c=t.expirationTimes,s=t.hiddenUpdates;for(l=i&~l;0<l;){var h=31-ee(l),v=1<<h;o[h]=0,c[h]=-1;var d=s[h];if(d!==null)for(s[h]=null,h=0;h<d.length;h++){var p=d[h];p!==null&&(p.lane&=-536870913)}l&=~v}a!==0&&ad(t,a,0),u!==0&&n===0&&t.tag!==0&&(t.suspendedLanes|=u&~(i&~e))}function ad(t,e,l){t.pendingLanes|=e,t.suspendedLanes&=~e;var a=31-ee(e);t.entangledLanes|=e,t.entanglements[a]=t.entanglements[a]|1073741824|l&261930}function nd(t,e){var l=t.entangledLanes|=e;for(t=t.entanglements;l;){var a=31-ee(l),n=1<<a;n&e|t[a]&e&&(t[a]|=e),l&=~n}}function ud(t,e){var l=e&-e;return l=l&42?1:Qc(l),l&(t.suspendedLanes|e)?0:l}function Qc(t){switch(t){case 2:t=1;break;case 8:t=4;break;case 32:t=16;break;case 256:case 512:case 1024:case 2048:case 4096:case 8192:case 16384:case 32768:case 65536:case 131072:case 262144:case 524288:case 1048576:case 2097152:case 4194304:case 8388608:case 16777216:case 33554432:t=128;break;case 268435456:t=134217728;break;default:t=0}return t}function Zc(t){return t&=-t,2<t?8<t?t&134217727?32:268435456:8:2}function id(){var t=Z.p;return t!==0?t:(t=window.event,t===void 0?32:qp(t.type))}function Tr(t,e){var l=Z.p;try{return Z.p=t,e()}finally{Z.p=l}}var Ml=Math.random().toString(36).slice(2),Dt="__reactFiber$"+Ml,Zt="__reactProps$"+Ml,Ya="__reactContainer$"+Ml,tc="__reactEvents$"+Ml,ih="__reactListeners$"+Ml,oh="__reactHandles$"+Ml,Ar="__reactResources$"+Ml,Qn="__reactMarker$"+Ml;function Vc(t){delete t[Dt],delete t[Zt],delete t[tc],delete t[ih],delete t[oh]}function sa(t){var e=t[Dt];if(e)return e;for(var l=t.parentNode;l;){if(e=l[Ya]||l[Dt]){if(l=e.alternate,e.child!==null||l!==null&&l.child!==null)for(t=Rs(t);t!==null;){if(l=t[Dt])return l;t=Rs(t)}return e}t=l,l=t.parentNode}return null}function Ga(t){if(t=t[Dt]||t[Ya]){var e=t.tag;if(e===5||e===6||e===13||e===31||e===26||e===27||e===3)return t}return null}function rn(t){var e=t.tag;if(e===5||e===26||e===27||e===6)return t.stateNode;throw Error(y(33))}function Sa(t){var e=t[Ar];return e||(e=t[Ar]={hoistableStyles:new Map,hoistableScripts:new Map}),e}function Tt(t){t[Qn]=!0}var od=new Set,cd={};function Vl(t,e){qa(t,e),qa(t+"Capture",e)}function qa(t,e){for(cd[t]=e,t=0;t<e.length;t++)od.add(e[t])}var ch=RegExp("^[:A-Z_a-z\\u00C0-\\u00D6\\u00D8-\\u00F6\\u00F8-\\u02FF\\u0370-\\u037D\\u037F-\\u1FFF\\u200C-\\u200D\\u2070-\\u218F\\u2C00-\\u2FEF\\u3001-\\uD7FF\\uF900-\\uFDCF\\uFDF0-\\uFFFD][:A-Z_a-z\\u00C0-\\u00D6\\u00D8-\\u00F6\\u00F8-\\u02FF\\u0370-\\u037D\\u037F-\\u1FFF\\u200C-\\u200D\\u2070-\\u218F\\u2C00-\\u2FEF\\u3001-\\uD7FF\\uF900-\\uFDCF\\uFDF0-\\uFFFD\\-.0-9\\u00B7\\u0300-\\u036F\\u203F-\\u2040]*$"),Mr={},Dr={};function fh(t){return Po.call(Dr,t)?!0:Po.call(Mr,t)?!1:ch.test(t)?Dr[t]=!0:(Mr[t]=!0,!1)}function Du(t,e,l){if(fh(e))if(l===null)t.removeAttribute(e);else{switch(typeof l){case"undefined":case"function":case"symbol":t.removeAttribute(e);return;case"boolean":var a=e.toLowerCase().slice(0,5);if(a!=="data-"&&a!=="aria-"){t.removeAttribute(e);return}}t.setAttribute(e,""+l)}}function pu(t,e,l){if(l===null)t.removeAttribute(e);else{switch(typeof l){case"undefined":case"function":case"symbol":case"boolean":t.removeAttribute(e);return}t.setAttribute(e,""+l)}}function He(t,e,l,a){if(a===null)t.removeAttribute(l);else{switch(typeof a){case"undefined":case"function":case"symbol":case"boolean":t.removeAttribute(l);return}t.setAttributeNS(e,l,""+a)}}function ce(t){switch(typeof t){case"bigint":case"boolean":case"number":case"string":case"undefined":return t;case"object":return t;default:return""}}function fd(t){var e=t.type;return(t=t.nodeName)&&t.toLowerCase()==="input"&&(e==="checkbox"||e==="radio")}function rh(t,e,l){var a=Object.getOwnPropertyDescriptor(t.constructor.prototype,e);if(!t.hasOwnProperty(e)&&typeof a<"u"&&typeof a.get=="function"&&typeof a.set=="function"){var n=a.get,u=a.set;return Object.defineProperty(t,e,{configurable:!0,get:function(){return n.call(this)},set:function(i){l=""+i,u.call(this,i)}}),Object.defineProperty(t,e,{enumerable:a.enumerable}),{getValue:function(){return l},setValue:function(i){l=""+i},stopTracking:function(){t._valueTracker=null,delete t[e]}}}}function ec(t){if(!t._valueTracker){var e=fd(t)?"checked":"value";t._valueTracker=rh(t,e,""+t[e])}}function rd(t){if(!t)return!1;var e=t._valueTracker;if(!e)return!0;var l=e.getValue(),a="";return t&&(a=fd(t)?t.checked?"true":"false":t.value),t=a,t!==l?(e.setValue(t),!0):!1}function Ku(t){if(t=t||(typeof document<"u"?document:void 0),typeof t>"u")return null;try{return t.activeElement||t.body}catch{return t.body}}var sh=/[\n"\\]/g;function se(t){return t.replace(sh,function(e){return"\\"+e.charCodeAt(0).toString(16)+" "})}function lc(t,e,l,a,n,u,i,o){t.name="",i!=null&&typeof i!="function"&&typeof i!="symbol"&&typeof i!="boolean"?t.type=i:t.removeAttribute("type"),e!=null?i==="number"?(e===0&&t.value===""||t.value!=e)&&(t.value=""+ce(e)):t.value!==""+ce(e)&&(t.value=""+ce(e)):i!=="submit"&&i!=="reset"||t.removeAttribute("value"),e!=null?ac(t,i,ce(e)):l!=null?ac(t,i,ce(l)):a!=null&&t.removeAttribute("value"),n==null&&u!=null&&(t.defaultChecked=!!u),n!=null&&(t.checked=n&&typeof n!="function"&&typeof n!="symbol"),o!=null&&typeof o!="function"&&typeof o!="symbol"&&typeof o!="boolean"?t.name=""+ce(o):t.removeAttribute("name")}function sd(t,e,l,a,n,u,i,o){if(u!=null&&typeof u!="function"&&typeof u!="symbol"&&typeof u!="boolean"&&(t.type=u),e!=null||l!=null){if(!(u!=="submit"&&u!=="reset"||e!=null)){ec(t);return}l=l!=null?""+ce(l):"",e=e!=null?""+ce(e):l,o||e===t.value||(t.value=e),t.defaultValue=e}a=a??n,a=typeof a!="function"&&typeof a!="symbol"&&!!a,t.checked=o?t.checked:!!a,t.defaultChecked=!!a,i!=null&&typeof i!="function"&&typeof i!="symbol"&&typeof i!="boolean"&&(t.name=i),ec(t)}function ac(t,e,l){e==="number"&&Ku(t.ownerDocument)===t||t.defaultValue===""+l||(t.defaultValue=""+l)}function xa(t,e,l,a){if(t=t.options,e){e={};for(var n=0;n<l.length;n++)e["$"+l[n]]=!0;for(l=0;l<t.length;l++)n=e.hasOwnProperty("$"+t[l].value),t[l].selected!==n&&(t[l].selected=n),n&&a&&(t[l].defaultSelected=!0)}else{for(l=""+ce(l),e=null,n=0;n<t.length;n++){if(t[n].value===l){t[n].selected=!0,a&&(t[n].defaultSelected=!0);return}e!==null||t[n].disabled||(e=t[n])}e!==null&&(e.selected=!0)}}function dd(t,e,l){if(e!=null&&(e=""+ce(e),e!==t.value&&(t.value=e),l==null)){t.defaultValue!==e&&(t.defaultValue=e);return}t.defaultValue=l!=null?""+ce(l):""}function md(t,e,l,a){if(e==null){if(a!=null){if(l!=null)throw Error(y(92));if(fn(a)){if(1<a.length)throw Error(y(93));a=a[0]}l=a}l==null&&(l=""),e=l}l=ce(e),t.defaultValue=l,a=t.textContent,a===l&&a!==""&&a!==null&&(t.value=a),ec(t)}function wa(t,e){if(e){var l=t.firstChild;if(l&&l===t.lastChild&&l.nodeType===3){l.nodeValue=e;return}}t.textContent=e}var dh=new Set("animationIterationCount aspectRatio borderImageOutset borderImageSlice borderImageWidth boxFlex boxFlexGroup boxOrdinalGroup columnCount columns flex flexGrow flexPositive flexShrink flexNegative flexOrder gridArea gridRow gridRowEnd gridRowSpan gridRowStart gridColumn gridColumnEnd gridColumnSpan gridColumnStart fontWeight lineClamp lineHeight opacity order orphans scale tabSize widows zIndex zoom fillOpacity floodOpacity stopOpacity strokeDasharray strokeDashoffset strokeMiterlimit strokeOpacity strokeWidth MozAnimationIterationCount MozBoxFlex MozBoxFlexGroup MozLineClamp msAnimationIterationCount msFlex msZoom msFlexGrow msFlexNegative msFlexOrder msFlexPositive msFlexShrink msGridColumn msGridColumnSpan msGridRow msGridRowSpan WebkitAnimationIterationCount WebkitBoxFlex WebKitBoxFlexGroup WebkitBoxOrdinalGroup WebkitColumnCount WebkitColumns WebkitFlex WebkitFlexGrow WebkitFlexPositive WebkitFlexShrink WebkitLineClamp".split(" "));function qr(t,e,l){var a=e.indexOf("--")===0;l==null||typeof l=="boolean"||l===""?a?t.setProperty(e,""):e==="float"?t.cssFloat="":t[e]="":a?t.setProperty(e,l):typeof l!="number"||l===0||dh.has(e)?e==="float"?t.cssFloat=l:t[e]=(""+l).trim():t[e]=l+"px"}function pd(t,e,l){if(e!=null&&typeof e!="object")throw Error(y(62));if(t=t.style,l!=null){for(var a in l)!l.hasOwnProperty(a)||e!=null&&e.hasOwnProperty(a)||(a.indexOf("--")===0?t.setProperty(a,""):a==="float"?t.cssFloat="":t[a]="");for(var n in e)a=e[n],e.hasOwnProperty(n)&&l[n]!==a&&qr(t,n,a)}else for(var u in e)e.hasOwnProperty(u)&&qr(t,u,e[u])}function Kc(t){if(t.indexOf("-")===-1)return!1;switch(t){case"annotation-xml":case"color-profile":case"font-face":case"font-face-src":case"font-face-uri":case"font-face-format":case"font-face-name":case"missing-glyph":return!1;default:return!0}}var mh=new Map([["acceptCharset","accept-charset"],["htmlFor","for"],["httpEquiv","http-equiv"],["crossOrigin","crossorigin"],["accentHeight","accent-height"],["alignmentBaseline","alignment-baseline"],["arabicForm","arabic-form"],["baselineShift","baseline-shift"],["capHeight","cap-height"],["clipPath","clip-path"],["clipRule","clip-rule"],["colorInterpolation","color-interpolation"],["colorInterpolationFilters","color-interpolation-filters"],["colorProfile","color-profile"],["colorRendering","color-rendering"],["dominantBaseline","dominant-baseline"],["enableBackground","enable-background"],["fillOpacity","fill-opacity"],["fillRule","fill-rule"],["floodColor","flood-color"],["floodOpacity","flood-opacity"],["fontFamily","font-family"],["fontSize","font-size"],["fontSizeAdjust","font-size-adjust"],["fontStretch","font-stretch"],["fontStyle","font-style"],["fontVariant","font-variant"],["fontWeight","font-weight"],["glyphName","glyph-name"],["glyphOrientationHorizontal","glyph-orientation-horizontal"],["glyphOrientationVertical","glyph-orientation-vertical"],["horizAdvX","horiz-adv-x"],["horizOriginX","horiz-origin-x"],["imageRendering","image-rendering"],["letterSpacing","letter-spacing"],["lightingColor","lighting-color"],["markerEnd","marker-end"],["markerMid","marker-mid"],["markerStart","marker-start"],["overlinePosition","overline-position"],["overlineThickness","overline-thickness"],["paintOrder","paint-order"],["panose-1","panose-1"],["pointerEvents","pointer-events"],["renderingIntent","rendering-intent"],["shapeRendering","shape-rendering"],["stopColor","stop-color"],["stopOpacity","stop-opacity"],["strikethroughPosition","strikethrough-position"],["strikethroughThickness","strikethrough-thickness"],["strokeDasharray","stroke-dasharray"],["strokeDashoffset","stroke-dashoffset"],["strokeLinecap","stroke-linecap"],["strokeLinejoin","stroke-linejoin"],["strokeMiterlimit","stroke-miterlimit"],["strokeOpacity","stroke-opacity"],["strokeWidth","stroke-width"],["textAnchor","text-anchor"],["textDecoration","text-decoration"],["textRendering","text-rendering"],["transformOrigin","transform-origin"],["underlinePosition","underline-position"],["underlineThickness","underline-thickness"],["unicodeBidi","unicode-bidi"],["unicodeRange","unicode-range"],["unitsPerEm","units-per-em"],["vAlphabetic","v-alphabetic"],["vHanging","v-hanging"],["vIdeographic","v-ideographic"],["vMathematical","v-mathematical"],["vectorEffect","vector-effect"],["vertAdvY","vert-adv-y"],["vertOriginX","vert-origin-x"],["vertOriginY","vert-origin-y"],["wordSpacing","word-spacing"],["writingMode","writing-mode"],["xmlnsXlink","xmlns:xlink"],["xHeight","x-height"]]),ph=/^[\u0000-\u001F ]*j[\r\n\t]*a[\r\n\t]*v[\r\n\t]*a[\r\n\t]*s[\r\n\t]*c[\r\n\t]*r[\r\n\t]*i[\r\n\t]*p[\r\n\t]*t[\r\n\t]*:/i;function qu(t){return ph.test(""+t)?"javascript:throw new Error('React has blocked a javascript: URL as a security precaution.')":t}function Qe(){}var nc=null;function Jc(t){return t=t.target||t.srcElement||window,t.correspondingUseElement&&(t=t.correspondingUseElement),t.nodeType===3?t.parentNode:t}var da=null,za=null;function wr(t){var e=Ga(t);if(e&&(t=e.stateNode)){var l=t[Zt]||null;t:switch(t=e.stateNode,e.type){case"input":if(lc(t,l.value,l.defaultValue,l.defaultValue,l.checked,l.defaultChecked,l.type,l.name),e=l.name,l.type==="radio"&&e!=null){for(l=t;l.parentNode;)l=l.parentNode;for(l=l.querySelectorAll('input[name="'+se(""+e)+'"][type="radio"]'),e=0;e<l.length;e++){var a=l[e];if(a!==t&&a.form===t.form){var n=a[Zt]||null;if(!n)throw Error(y(90));lc(a,n.value,n.defaultValue,n.defaultValue,n.checked,n.defaultChecked,n.type,n.name)}}for(e=0;e<l.length;e++)a=l[e],a.form===t.form&&rd(a)}break t;case"textarea":dd(t,l.value,l.defaultValue);break t;case"select":e=l.value,e!=null&&xa(t,!!l.multiple,e,!1)}}}var yo=!1;function hd(t,e,l){if(yo)return t(e,l);yo=!0;try{var a=t(e);return a}finally{if(yo=!1,(da!==null||za!==null)&&(wi(),da&&(e=da,t=za,za=da=null,wr(e),t)))for(e=0;e<t.length;e++)wr(t[e])}}function Mn(t,e){var l=t.stateNode;if(l===null)return null;var a=l[Zt]||null;if(a===null)return null;l=a[e];t:switch(e){case"onClick":case"onClickCapture":case"onDoubleClick":case"onDoubleClickCapture":case"onMouseDown":case"onMouseDownCapture":case"onMouseMove":case"onMouseMoveCapture":case"onMouseUp":case"onMouseUpCapture":case"onMouseEnter":(a=!a.disabled)||(t=t.type,a=!(t==="button"||t==="input"||t==="select"||t==="textarea")),t=!a;break t;default:t=!1}if(t)return null;if(l&&typeof l!="function")throw Error(y(231,e,typeof l));return l}var We=!(typeof window>"u"||typeof window.document>"u"||typeof window.document.createElement>"u"),uc=!1;if(We)try{na={},Object.defineProperty(na,"passive",{get:function(){uc=!0}}),window.addEventListener("test",na,na),window.removeEventListener("test",na,na)}catch{uc=!1}var na,rl=null,Wc=null,wu=null;function vd(){if(wu)return wu;var t,e=Wc,l=e.length,a,n="value"in rl?rl.value:rl.textContent,u=n.length;for(t=0;t<l&&e[t]===n[t];t++);var i=l-t;for(a=1;a<=i&&e[l-a]===n[u-a];a++);return wu=n.slice(t,1<a?1-a:void 0)}function Ou(t){var e=t.keyCode;return"charCode"in t?(t=t.charCode,t===0&&e===13&&(t=13)):t=e,t===10&&(t=13),32<=t||t===13?t:0}function hu(){return!0}function Or(){return!1}function Vt(t){function e(l,a,n,u,i){this._reactName=l,this._targetInst=n,this.type=a,this.nativeEvent=u,this.target=i,this.currentTarget=null;for(var o in t)t.hasOwnProperty(o)&&(l=t[o],this[o]=l?l(u):u[o]);return this.isDefaultPrevented=(u.defaultPrevented!=null?u.defaultPrevented:u.returnValue===!1)?hu:Or,this.isPropagationStopped=Or,this}return et(e.prototype,{preventDefault:function(){this.defaultPrevented=!0;var l=this.nativeEvent;l&&(l.preventDefault?l.preventDefault():typeof l.returnValue!="unknown"&&(l.returnValue=!1),this.isDefaultPrevented=hu)},stopPropagation:function(){var l=this.nativeEvent;l&&(l.stopPropagation?l.stopPropagation():typeof l.cancelBubble!="unknown"&&(l.cancelBubble=!0),this.isPropagationStopped=hu)},persist:function(){},isPersistent:hu}),e}var Kl={eventPhase:0,bubbles:0,cancelable:0,timeStamp:function(t){return t.timeStamp||Date.now()},defaultPrevented:0,isTrusted:0},bi=Vt(Kl),Zn=et({},Kl,{view:0,detail:0}),hh=Vt(Zn),bo,_o,en,_i=et({},Zn,{screenX:0,screenY:0,clientX:0,clientY:0,pageX:0,pageY:0,ctrlKey:0,shiftKey:0,altKey:0,metaKey:0,getModifierState:$c,button:0,buttons:0,relatedTarget:function(t){return t.relatedTarget===void 0?t.fromElement===t.srcElement?t.toElement:t.fromElement:t.relatedTarget},movementX:function(t){return"movementX"in t?t.movementX:(t!==en&&(en&&t.type==="mousemove"?(bo=t.screenX-en.screenX,_o=t.screenY-en.screenY):_o=bo=0,en=t),bo)},movementY:function(t){return"movementY"in t?t.movementY:_o}}),Nr=Vt(_i),vh=et({},_i,{dataTransfer:0}),gh=Vt(vh),yh=et({},Zn,{relatedTarget:0}),So=Vt(yh),bh=et({},Kl,{animationName:0,elapsedTime:0,pseudoElement:0}),_h=Vt(bh),Sh=et({},Kl,{clipboardData:function(t){return"clipboardData"in t?t.clipboardData:window.clipboardData}}),xh=Vt(Sh),zh=et({},Kl,{data:0}),Cr=Vt(zh),Eh={Esc:"Escape",Spacebar:" ",Left:"ArrowLeft",Up:"ArrowUp",Right:"ArrowRight",Down:"ArrowDown",Del:"Delete",Win:"OS",Menu:"ContextMenu",Apps:"ContextMenu",Scroll:"ScrollLock",MozPrintableKey:"Unidentified"},Th={8:"Backspace",9:"Tab",12:"Clear",13:"Enter",16:"Shift",17:"Control",18:"Alt",19:"Pause",20:"CapsLock",27:"Escape",32:" ",33:"PageUp",34:"PageDown",35:"End",36:"Home",37:"ArrowLeft",38:"ArrowUp",39:"ArrowRight",40:"ArrowDown",45:"Insert",46:"Delete",112:"F1",113:"F2",114:"F3",115:"F4",116:"F5",117:"F6",118:"F7",119:"F8",120:"F9",121:"F10",122:"F11",123:"F12",144:"NumLock",145:"ScrollLock",224:"Meta"},Ah={Alt:"altKey",Control:"ctrlKey",Meta:"metaKey",Shift:"shiftKey"};function Mh(t){var e=this.nativeEvent;return e.getModifierState?e.getModifierState(t):(t=Ah[t])?!!e[t]:!1}function $c(){return Mh}var Dh=et({},Zn,{key:function(t){if(t.key){var e=Eh[t.key]||t.key;if(e!=="Unidentified")return e}return t.type==="keypress"?(t=Ou(t),t===13?"Enter":String.fromCharCode(t)):t.type==="keydown"||t.type==="keyup"?Th[t.keyCode]||"Unidentified":""},code:0,location:0,ctrlKey:0,shiftKey:0,altKey:0,metaKey:0,repeat:0,locale:0,getModifierState:$c,charCode:function(t){return t.type==="keypress"?Ou(t):0},keyCode:function(t){return t.type==="keydown"||t.type==="keyup"?t.keyCode:0},which:function(t){return t.type==="keypress"?Ou(t):t.type==="keydown"||t.type==="keyup"?t.keyCode:0}}),qh=Vt(Dh),wh=et({},_i,{pointerId:0,width:0,height:0,pressure:0,tangentialPressure:0,tiltX:0,tiltY:0,twist:0,pointerType:0,isPrimary:0}),Rr=Vt(wh),Oh=et({},Zn,{touches:0,targetTouches:0,changedTouches:0,altKey:0,metaKey:0,ctrlKey:0,shiftKey:0,getModifierState:$c}),Nh=Vt(Oh),Ch=et({},Kl,{propertyName:0,elapsedTime:0,pseudoElement:0}),Rh=Vt(Ch),Uh=et({},_i,{deltaX:function(t){return"deltaX"in t?t.deltaX:"wheelDeltaX"in t?-t.wheelDeltaX:0},deltaY:function(t){return"deltaY"in t?t.deltaY:"wheelDeltaY"in t?-t.wheelDeltaY:"wheelDelta"in t?-t.wheelDelta:0},deltaZ:0,deltaMode:0}),Hh=Vt(Uh),kh=et({},Kl,{newState:0,oldState:0}),Bh=Vt(kh),Yh=[9,13,27,32],Fc=We&&"CompositionEvent"in window,mn=null;We&&"documentMode"in document&&(mn=document.documentMode);var Gh=We&&"TextEvent"in window&&!mn,gd=We&&(!Fc||mn&&8<mn&&11>=mn),Ur=" ",Hr=!1;function yd(t,e){switch(t){case"keyup":return Yh.indexOf(e.keyCode)!==-1;case"keydown":return e.keyCode!==229;case"keypress":case"mousedown":case"focusout":return!0;default:return!1}}function bd(t){return t=t.detail,typeof t=="object"&&"data"in t?t.data:null}var ma=!1;function Xh(t,e){switch(t){case"compositionend":return bd(e);case"keypress":return e.which!==32?null:(Hr=!0,Ur);case"textInput":return t=e.data,t===Ur&&Hr?null:t;default:return null}}function Lh(t,e){if(ma)return t==="compositionend"||!Fc&&yd(t,e)?(t=vd(),wu=Wc=rl=null,ma=!1,t):null;switch(t){case"paste":return null;case"keypress":if(!(e.ctrlKey||e.altKey||e.metaKey)||e.ctrlKey&&e.altKey){if(e.char&&1<e.char.length)return e.char;if(e.which)return String.fromCharCode(e.which)}return null;case"compositionend":return gd&&e.locale!=="ko"?null:e.data;default:return null}}var jh={color:!0,date:!0,datetime:!0,"datetime-local":!0,email:!0,month:!0,number:!0,password:!0,range:!0,search:!0,tel:!0,text:!0,time:!0,url:!0,week:!0};function kr(t){var e=t&&t.nodeName&&t.nodeName.toLowerCase();return e==="input"?!!jh[t.type]:e==="textarea"}function _d(t,e,l,a){da?za?za.push(a):za=[a]:da=a,e=si(e,"onChange"),0<e.length&&(l=new bi("onChange","change",null,l,a),t.push({event:l,listeners:e}))}var pn=null,Dn=null;function Qh(t){hp(t,0)}function Si(t){var e=rn(t);if(rd(e))return t}function Br(t,e){if(t==="change")return e}var Sd=!1;We&&(We?(gu="oninput"in document,gu||(xo=document.createElement("div"),xo.setAttribute("oninput","return;"),gu=typeof xo.oninput=="function"),vu=gu):vu=!1,Sd=vu&&(!document.documentMode||9<document.documentMode));var vu,gu,xo;function Yr(){pn&&(pn.detachEvent("onpropertychange",xd),Dn=pn=null)}function xd(t){if(t.propertyName==="value"&&Si(Dn)){var e=[];_d(e,Dn,t,Jc(t)),hd(Qh,e)}}function Zh(t,e,l){t==="focusin"?(Yr(),pn=e,Dn=l,pn.attachEvent("onpropertychange",xd)):t==="focusout"&&Yr()}function Vh(t){if(t==="selectionchange"||t==="keyup"||t==="keydown")return Si(Dn)}function Kh(t,e){if(t==="click")return Si(e)}function Jh(t,e){if(t==="input"||t==="change")return Si(e)}function Wh(t,e){return t===e&&(t!==0||1/t===1/e)||t!==t&&e!==e}var ae=typeof Object.is=="function"?Object.is:Wh;function qn(t,e){if(ae(t,e))return!0;if(typeof t!="object"||t===null||typeof e!="object"||e===null)return!1;var l=Object.keys(t),a=Object.keys(e);if(l.length!==a.length)return!1;for(a=0;a<l.length;a++){var n=l[a];if(!Po.call(e,n)||!ae(t[n],e[n]))return!1}return!0}function Gr(t){for(;t&&t.firstChild;)t=t.firstChild;return t}function Xr(t,e){var l=Gr(t);t=0;for(var a;l;){if(l.nodeType===3){if(a=t+l.textContent.length,t<=e&&a>=e)return{node:l,offset:e-t};t=a}t:{for(;l;){if(l.nextSibling){l=l.nextSibling;break t}l=l.parentNode}l=void 0}l=Gr(l)}}function zd(t,e){return t&&e?t===e?!0:t&&t.nodeType===3?!1:e&&e.nodeType===3?zd(t,e.parentNode):"contains"in t?t.contains(e):t.compareDocumentPosition?!!(t.compareDocumentPosition(e)&16):!1:!1}function Ed(t){t=t!=null&&t.ownerDocument!=null&&t.ownerDocument.defaultView!=null?t.ownerDocument.defaultView:window;for(var e=Ku(t.document);e instanceof t.HTMLIFrameElement;){try{var l=typeof e.contentWindow.location.href=="string"}catch{l=!1}if(l)t=e.contentWindow;else break;e=Ku(t.document)}return e}function Ic(t){var e=t&&t.nodeName&&t.nodeName.toLowerCase();return e&&(e==="input"&&(t.type==="text"||t.type==="search"||t.type==="tel"||t.type==="url"||t.type==="password")||e==="textarea"||t.contentEditable==="true")}var $h=We&&"documentMode"in document&&11>=document.documentMode,pa=null,ic=null,hn=null,oc=!1;function Lr(t,e,l){var a=l.window===l?l.document:l.nodeType===9?l:l.ownerDocument;oc||pa==null||pa!==Ku(a)||(a=pa,"selectionStart"in a&&Ic(a)?a={start:a.selectionStart,end:a.selectionEnd}:(a=(a.ownerDocument&&a.ownerDocument.defaultView||window).getSelection(),a={anchorNode:a.anchorNode,anchorOffset:a.anchorOffset,focusNode:a.focusNode,focusOffset:a.focusOffset}),hn&&qn(hn,a)||(hn=a,a=si(ic,"onSelect"),0<a.length&&(e=new bi("onSelect","select",null,e,l),t.push({event:e,listeners:a}),e.target=pa)))}function Ol(t,e){var l={};return l[t.toLowerCase()]=e.toLowerCase(),l["Webkit"+t]="webkit"+e,l["Moz"+t]="moz"+e,l}var ha={animationend:Ol("Animation","AnimationEnd"),animationiteration:Ol("Animation","AnimationIteration"),animationstart:Ol("Animation","AnimationStart"),transitionrun:Ol("Transition","TransitionRun"),transitionstart:Ol("Transition","TransitionStart"),transitioncancel:Ol("Transition","TransitionCancel"),transitionend:Ol("Transition","TransitionEnd")},zo={},Td={};We&&(Td=document.createElement("div").style,"AnimationEvent"in window||(delete ha.animationend.animation,delete ha.animationiteration.animation,delete ha.animationstart.animation),"TransitionEvent"in window||delete ha.transitionend.transition);function Jl(t){if(zo[t])return zo[t];if(!ha[t])return t;var e=ha[t],l;for(l in e)if(e.hasOwnProperty(l)&&l in Td)return zo[t]=e[l];return t}var Ad=Jl("animationend"),Md=Jl("animationiteration"),Dd=Jl("animationstart"),Fh=Jl("transitionrun"),Ih=Jl("transitionstart"),Ph=Jl("transitioncancel"),qd=Jl("transitionend"),wd=new Map,cc="abort auxClick beforeToggle cancel canPlay canPlayThrough click close contextMenu copy cut drag dragEnd dragEnter dragExit dragLeave dragOver dragStart drop durationChange emptied encrypted ended error gotPointerCapture input invalid keyDown keyPress keyUp load loadedData loadedMetadata loadStart lostPointerCapture mouseDown mouseMove mouseOut mouseOver mouseUp paste pause play playing pointerCancel pointerDown pointerMove pointerOut pointerOver pointerUp progress rateChange reset resize seeked seeking stalled submit suspend timeUpdate touchCancel touchEnd touchStart volumeChange scroll toggle touchMove waiting wheel".split(" ");cc.push("scrollEnd");function _e(t,e){wd.set(t,e),Vl(e,[t])}var Ju=typeof reportError=="function"?reportError:function(t){if(typeof window=="object"&&typeof window.ErrorEvent=="function"){var e=new window.ErrorEvent("error",{bubbles:!0,cancelable:!0,message:typeof t=="object"&&t!==null&&typeof t.message=="string"?String(t.message):String(t),error:t});if(!window.dispatchEvent(e))return}else if(typeof process=="object"&&typeof process.emit=="function"){process.emit("uncaughtException",t);return}console.error(t)},oe=[],va=0,Pc=0;function xi(){for(var t=va,e=Pc=va=0;e<t;){var l=oe[e];oe[e++]=null;var a=oe[e];oe[e++]=null;var n=oe[e];oe[e++]=null;var u=oe[e];if(oe[e++]=null,a!==null&&n!==null){var i=a.pending;i===null?n.next=n:(n.next=i.next,i.next=n),a.pending=n}u!==0&&Od(l,n,u)}}function zi(t,e,l,a){oe[va++]=t,oe[va++]=e,oe[va++]=l,oe[va++]=a,Pc|=a,t.lanes|=a,t=t.alternate,t!==null&&(t.lanes|=a)}function tf(t,e,l,a){return zi(t,e,l,a),Wu(t)}function Wl(t,e){return zi(t,null,null,e),Wu(t)}function Od(t,e,l){t.lanes|=l;var a=t.alternate;a!==null&&(a.lanes|=l);for(var n=!1,u=t.return;u!==null;)u.childLanes|=l,a=u.alternate,a!==null&&(a.childLanes|=l),u.tag===22&&(t=u.stateNode,t===null||t._visibility&1||(n=!0)),t=u,u=u.return;return t.tag===3?(u=t.stateNode,n&&e!==null&&(n=31-ee(l),t=u.hiddenUpdates,a=t[n],a===null?t[n]=[e]:a.push(e),e.lane=l|536870912),u):null}function Wu(t){if(50<En)throw En=0,qc=null,Error(y(185));for(var e=t.return;e!==null;)t=e,e=t.return;return t.tag===3?t.stateNode:null}var ga={};function tv(t,e,l,a){this.tag=t,this.key=l,this.sibling=this.child=this.return=this.stateNode=this.type=this.elementType=null,this.index=0,this.refCleanup=this.ref=null,this.pendingProps=e,this.dependencies=this.memoizedState=this.updateQueue=this.memoizedProps=null,this.mode=a,this.subtreeFlags=this.flags=0,this.deletions=null,this.childLanes=this.lanes=0,this.alternate=null}function Ft(t,e,l,a){return new tv(t,e,l,a)}function ef(t){return t=t.prototype,!(!t||!t.isReactComponent)}function Ve(t,e){var l=t.alternate;return l===null?(l=Ft(t.tag,e,t.key,t.mode),l.elementType=t.elementType,l.type=t.type,l.stateNode=t.stateNode,l.alternate=t,t.alternate=l):(l.pendingProps=e,l.type=t.type,l.flags=0,l.subtreeFlags=0,l.deletions=null),l.flags=t.flags&65011712,l.childLanes=t.childLanes,l.lanes=t.lanes,l.child=t.child,l.memoizedProps=t.memoizedProps,l.memoizedState=t.memoizedState,l.updateQueue=t.updateQueue,e=t.dependencies,l.dependencies=e===null?null:{lanes:e.lanes,firstContext:e.firstContext},l.sibling=t.sibling,l.index=t.index,l.ref=t.ref,l.refCleanup=t.refCleanup,l}function Nd(t,e){t.flags&=65011714;var l=t.alternate;return l===null?(t.childLanes=0,t.lanes=e,t.child=null,t.subtreeFlags=0,t.memoizedProps=null,t.memoizedState=null,t.updateQueue=null,t.dependencies=null,t.stateNode=null):(t.childLanes=l.childLanes,t.lanes=l.lanes,t.child=l.child,t.subtreeFlags=0,t.deletions=null,t.memoizedProps=l.memoizedProps,t.memoizedState=l.memoizedState,t.updateQueue=l.updateQueue,t.type=l.type,e=l.dependencies,t.dependencies=e===null?null:{lanes:e.lanes,firstContext:e.firstContext}),t}function Nu(t,e,l,a,n,u){var i=0;if(a=t,typeof t=="function")ef(t)&&(i=1);else if(typeof t=="string")i=ag(t,l,qe.current)?26:t==="html"||t==="head"||t==="body"?27:5;else t:switch(t){case Wo:return t=Ft(31,l,e,n),t.elementType=Wo,t.lanes=u,t;case fa:return kl(l.children,n,u,e);case Fs:i=8,n|=24;break;case Vo:return t=Ft(12,l,e,n|2),t.elementType=Vo,t.lanes=u,t;case Ko:return t=Ft(13,l,e,n),t.elementType=Ko,t.lanes=u,t;case Jo:return t=Ft(19,l,e,n),t.elementType=Jo,t.lanes=u,t;default:if(typeof t=="object"&&t!==null)switch(t.$$typeof){case je:i=10;break t;case Is:i=9;break t;case Xc:i=11;break t;case Lc:i=14;break t;case al:i=16,a=null;break t}i=29,l=Error(y(130,t===null?"null":typeof t,"")),a=null}return e=Ft(i,l,e,n),e.elementType=t,e.type=a,e.lanes=u,e}function kl(t,e,l,a){return t=Ft(7,t,a,e),t.lanes=l,t}function Eo(t,e,l){return t=Ft(6,t,null,e),t.lanes=l,t}function Cd(t){var e=Ft(18,null,null,0);return e.stateNode=t,e}function To(t,e,l){return e=Ft(4,t.children!==null?t.children:[],t.key,e),e.lanes=l,e.stateNode={containerInfo:t.containerInfo,pendingChildren:null,implementation:t.implementation},e}var jr=new WeakMap;function de(t,e){if(typeof t=="object"&&t!==null){var l=jr.get(t);return l!==void 0?l:(e={value:t,source:e,stack:Er(e)},jr.set(t,e),e)}return{value:t,source:e,stack:Er(e)}}var ya=[],ba=0,$u=null,wn=0,fe=[],re=0,zl=null,Ae=1,Me="";function Xe(t,e){ya[ba++]=wn,ya[ba++]=$u,$u=t,wn=e}function Rd(t,e,l){fe[re++]=Ae,fe[re++]=Me,fe[re++]=zl,zl=t;var a=Ae;t=Me;var n=32-ee(a)-1;a&=~(1<<n),l+=1;var u=32-ee(e)+n;if(30<u){var i=n-n%5;u=(a&(1<<i)-1).toString(32),a>>=i,n-=i,Ae=1<<32-ee(e)+n|l<<n|a,Me=u+t}else Ae=1<<u|l<<n|a,Me=t}function lf(t){t.return!==null&&(Xe(t,1),Rd(t,1,0))}function af(t){for(;t===$u;)$u=ya[--ba],ya[ba]=null,wn=ya[--ba],ya[ba]=null;for(;t===zl;)zl=fe[--re],fe[re]=null,Me=fe[--re],fe[re]=null,Ae=fe[--re],fe[re]=null}function Ud(t,e){fe[re++]=Ae,fe[re++]=Me,fe[re++]=zl,Ae=e.id,Me=e.overflow,zl=t}var qt=null,tt=null,L=!1,hl=null,me=!1,fc=Error(y(519));function El(t){var e=Error(y(418,1<arguments.length&&arguments[1]!==void 0&&arguments[1]?"text":"HTML",""));throw On(de(e,t)),fc}function Qr(t){var e=t.stateNode,l=t.type,a=t.memoizedProps;switch(e[Dt]=t,e[Zt]=a,l){case"dialog":Y("cancel",e),Y("close",e);break;case"iframe":case"object":case"embed":Y("load",e);break;case"video":case"audio":for(l=0;l<Un.length;l++)Y(Un[l],e);break;case"source":Y("error",e);break;case"img":case"image":case"link":Y("error",e),Y("load",e);break;case"details":Y("toggle",e);break;case"input":Y("invalid",e),sd(e,a.value,a.defaultValue,a.checked,a.defaultChecked,a.type,a.name,!0);break;case"select":Y("invalid",e);break;case"textarea":Y("invalid",e),md(e,a.value,a.defaultValue,a.children)}l=a.children,typeof l!="string"&&typeof l!="number"&&typeof l!="bigint"||e.textContent===""+l||a.suppressHydrationWarning===!0||gp(e.textContent,l)?(a.popover!=null&&(Y("beforetoggle",e),Y("toggle",e)),a.onScroll!=null&&Y("scroll",e),a.onScrollEnd!=null&&Y("scrollend",e),a.onClick!=null&&(e.onclick=Qe),e=!0):e=!1,e||El(t,!0)}function Zr(t){for(qt=t.return;qt;)switch(qt.tag){case 5:case 31:case 13:me=!1;return;case 27:case 3:me=!0;return;default:qt=qt.return}}function ua(t){if(t!==qt)return!1;if(!L)return Zr(t),L=!0,!1;var e=t.tag,l;if((l=e!==3&&e!==27)&&((l=e===5)&&(l=t.type,l=!(l!=="form"&&l!=="button")||Rc(t.type,t.memoizedProps)),l=!l),l&&tt&&El(t),Zr(t),e===13){if(t=t.memoizedState,t=t!==null?t.dehydrated:null,!t)throw Error(y(317));tt=Cs(t)}else if(e===31){if(t=t.memoizedState,t=t!==null?t.dehydrated:null,!t)throw Error(y(317));tt=Cs(t)}else e===27?(e=tt,Dl(t.type)?(t=Bc,Bc=null,tt=t):tt=e):tt=qt?he(t.stateNode.nextSibling):null;return!0}function Xl(){tt=qt=null,L=!1}function Ao(){var t=hl;return t!==null&&(jt===null?jt=t:jt.push.apply(jt,t),hl=null),t}function On(t){hl===null?hl=[t]:hl.push(t)}var rc=we(null),$l=null,Ze=null;function ul(t,e,l){F(rc,e._currentValue),e._currentValue=l}function Ke(t){t._currentValue=rc.current,At(rc)}function sc(t,e,l){for(;t!==null;){var a=t.alternate;if((t.childLanes&e)!==e?(t.childLanes|=e,a!==null&&(a.childLanes|=e)):a!==null&&(a.childLanes&e)!==e&&(a.childLanes|=e),t===l)break;t=t.return}}function dc(t,e,l,a){var n=t.child;for(n!==null&&(n.return=t);n!==null;){var u=n.dependencies;if(u!==null){var i=n.child;u=u.firstContext;t:for(;u!==null;){var o=u;u=n;for(var c=0;c<e.length;c++)if(o.context===e[c]){u.lanes|=l,o=u.alternate,o!==null&&(o.lanes|=l),sc(u.return,l,t),a||(i=null);break t}u=o.next}}else if(n.tag===18){if(i=n.return,i===null)throw Error(y(341));i.lanes|=l,u=i.alternate,u!==null&&(u.lanes|=l),sc(i,l,t),i=null}else i=n.child;if(i!==null)i.return=n;else for(i=n;i!==null;){if(i===t){i=null;break}if(n=i.sibling,n!==null){n.return=i.return,i=n;break}i=i.return}n=i}}function Xa(t,e,l,a){t=null;for(var n=e,u=!1;n!==null;){if(!u){if(n.flags&524288)u=!0;else if(n.flags&262144)break}if(n.tag===10){var i=n.alternate;if(i===null)throw Error(y(387));if(i=i.memoizedProps,i!==null){var o=n.type;ae(n.pendingProps.value,i.value)||(t!==null?t.push(o):t=[o])}}else if(n===ju.current){if(i=n.alternate,i===null)throw Error(y(387));i.memoizedState.memoizedState!==n.memoizedState.memoizedState&&(t!==null?t.push(kn):t=[kn])}n=n.return}t!==null&&dc(e,t,l,a),e.flags|=262144}function Fu(t){for(t=t.firstContext;t!==null;){if(!ae(t.context._currentValue,t.memoizedValue))return!0;t=t.next}return!1}function Ll(t){$l=t,Ze=null,t=t.dependencies,t!==null&&(t.firstContext=null)}function wt(t){return Hd($l,t)}function yu(t,e){return $l===null&&Ll(t),Hd(t,e)}function Hd(t,e){var l=e._currentValue;if(e={context:e,memoizedValue:l,next:null},Ze===null){if(t===null)throw Error(y(308));Ze=e,t.dependencies={lanes:0,firstContext:e},t.flags|=524288}else Ze=Ze.next=e;return l}var ev=typeof AbortController<"u"?AbortController:function(){var t=[],e=this.signal={aborted:!1,addEventListener:function(l,a){t.push(a)}};this.abort=function(){e.aborted=!0,t.forEach(function(l){return l()})}},lv=St.unstable_scheduleCallback,av=St.unstable_NormalPriority,vt={$$typeof:je,Consumer:null,Provider:null,_currentValue:null,_currentValue2:null,_threadCount:0};function nf(){return{controller:new ev,data:new Map,refCount:0}}function Vn(t){t.refCount--,t.refCount===0&&lv(av,function(){t.controller.abort()})}var vn=null,mc=0,Oa=0,Ea=null;function nv(t,e){if(vn===null){var l=vn=[];mc=0,Oa=wf(),Ea={status:"pending",value:void 0,then:function(a){l.push(a)}}}return mc++,e.then(Vr,Vr),e}function Vr(){if(--mc===0&&vn!==null){Ea!==null&&(Ea.status="fulfilled");var t=vn;vn=null,Oa=0,Ea=null;for(var e=0;e<t.length;e++)(0,t[e])()}}function uv(t,e){var l=[],a={status:"pending",value:null,reason:null,then:function(n){l.push(n)}};return t.then(function(){a.status="fulfilled",a.value=e;for(var n=0;n<l.length;n++)(0,l[n])(e)},function(n){for(a.status="rejected",a.reason=n,n=0;n<l.length;n++)(0,l[n])(void 0)}),a}var Kr=C.S;C.S=function(t,e){$m=Pt(),typeof e=="object"&&e!==null&&typeof e.then=="function"&&nv(t,e),Kr!==null&&Kr(t,e)};var Bl=we(null);function uf(){var t=Bl.current;return t!==null?t:$.pooledCache}function Cu(t,e){e===null?F(Bl,Bl.current):F(Bl,e.pool)}function kd(){var t=uf();return t===null?null:{parent:vt._currentValue,pool:t}}var La=Error(y(460)),of=Error(y(474)),Ei=Error(y(542)),Iu={then:function(){}};function Jr(t){return t=t.status,t==="fulfilled"||t==="rejected"}function Bd(t,e,l){switch(l=t[l],l===void 0?t.push(e):l!==e&&(e.then(Qe,Qe),e=l),e.status){case"fulfilled":return e.value;case"rejected":throw t=e.reason,$r(t),t;default:if(typeof e.status=="string")e.then(Qe,Qe);else{if(t=$,t!==null&&100<t.shellSuspendCounter)throw Error(y(482));t=e,t.status="pending",t.then(function(a){if(e.status==="pending"){var n=e;n.status="fulfilled",n.value=a}},function(a){if(e.status==="pending"){var n=e;n.status="rejected",n.reason=a}})}switch(e.status){case"fulfilled":return e.value;case"rejected":throw t=e.reason,$r(t),t}throw Yl=e,La}}function Rl(t){try{var e=t._init;return e(t._payload)}catch(l){throw l!==null&&typeof l=="object"&&typeof l.then=="function"?(Yl=l,La):l}}var Yl=null;function Wr(){if(Yl===null)throw Error(y(459));var t=Yl;return Yl=null,t}function $r(t){if(t===La||t===Ei)throw Error(y(483))}var Ta=null,Nn=0;function bu(t){var e=Nn;return Nn+=1,Ta===null&&(Ta=[]),Bd(Ta,t,e)}function ln(t,e){e=e.props.ref,t.ref=e!==void 0?e:null}function _u(t,e){throw e.$$typeof===Z0?Error(y(525)):(t=Object.prototype.toString.call(e),Error(y(31,t==="[object Object]"?"object with keys {"+Object.keys(e).join(", ")+"}":t)))}function Yd(t){function e(r,f){if(t){var m=r.deletions;m===null?(r.deletions=[f],r.flags|=16):m.push(f)}}function l(r,f){if(!t)return null;for(;f!==null;)e(r,f),f=f.sibling;return null}function a(r){for(var f=new Map;r!==null;)r.key!==null?f.set(r.key,r):f.set(r.index,r),r=r.sibling;return f}function n(r,f){return r=Ve(r,f),r.index=0,r.sibling=null,r}function u(r,f,m){return r.index=m,t?(m=r.alternate,m!==null?(m=m.index,m<f?(r.flags|=67108866,f):m):(r.flags|=67108866,f)):(r.flags|=1048576,f)}function i(r){return t&&r.alternate===null&&(r.flags|=67108866),r}function o(r,f,m,g){return f===null||f.tag!==6?(f=Eo(m,r.mode,g),f.return=r,f):(f=n(f,m),f.return=r,f)}function c(r,f,m,g){var E=m.type;return E===fa?h(r,f,m.props.children,g,m.key):f!==null&&(f.elementType===E||typeof E=="object"&&E!==null&&E.$$typeof===al&&Rl(E)===f.type)?(f=n(f,m.props),ln(f,m),f.return=r,f):(f=Nu(m.type,m.key,m.props,null,r.mode,g),ln(f,m),f.return=r,f)}function s(r,f,m,g){return f===null||f.tag!==4||f.stateNode.containerInfo!==m.containerInfo||f.stateNode.implementation!==m.implementation?(f=To(m,r.mode,g),f.return=r,f):(f=n(f,m.children||[]),f.return=r,f)}function h(r,f,m,g,E){return f===null||f.tag!==7?(f=kl(m,r.mode,g,E),f.return=r,f):(f=n(f,m),f.return=r,f)}function v(r,f,m){if(typeof f=="string"&&f!==""||typeof f=="number"||typeof f=="bigint")return f=Eo(""+f,r.mode,m),f.return=r,f;if(typeof f=="object"&&f!==null){switch(f.$$typeof){case ru:return m=Nu(f.type,f.key,f.props,null,r.mode,m),ln(m,f),m.return=r,m;case cn:return f=To(f,r.mode,m),f.return=r,f;case al:return f=Rl(f),v(r,f,m)}if(fn(f)||tn(f))return f=kl(f,r.mode,m,null),f.return=r,f;if(typeof f.then=="function")return v(r,bu(f),m);if(f.$$typeof===je)return v(r,yu(r,f),m);_u(r,f)}return null}function d(r,f,m,g){var E=f!==null?f.key:null;if(typeof m=="string"&&m!==""||typeof m=="number"||typeof m=="bigint")return E!==null?null:o(r,f,""+m,g);if(typeof m=="object"&&m!==null){switch(m.$$typeof){case ru:return m.key===E?c(r,f,m,g):null;case cn:return m.key===E?s(r,f,m,g):null;case al:return m=Rl(m),d(r,f,m,g)}if(fn(m)||tn(m))return E!==null?null:h(r,f,m,g,null);if(typeof m.then=="function")return d(r,f,bu(m),g);if(m.$$typeof===je)return d(r,f,yu(r,m),g);_u(r,m)}return null}function p(r,f,m,g,E){if(typeof g=="string"&&g!==""||typeof g=="number"||typeof g=="bigint")return r=r.get(m)||null,o(f,r,""+g,E);if(typeof g=="object"&&g!==null){switch(g.$$typeof){case ru:return r=r.get(g.key===null?m:g.key)||null,c(f,r,g,E);case cn:return r=r.get(g.key===null?m:g.key)||null,s(f,r,g,E);case al:return g=Rl(g),p(r,f,m,g,E)}if(fn(g)||tn(g))return r=r.get(m)||null,h(f,r,g,E,null);if(typeof g.then=="function")return p(r,f,m,bu(g),E);if(g.$$typeof===je)return p(r,f,m,yu(f,g),E);_u(f,g)}return null}function b(r,f,m,g){for(var E=null,w=null,x=f,M=f=0,O=null;x!==null&&M<m.length;M++){x.index>M?(O=x,x=null):O=x.sibling;var _=d(r,x,m[M],g);if(_===null){x===null&&(x=O);break}t&&x&&_.alternate===null&&e(r,x),f=u(_,f,M),w===null?E=_:w.sibling=_,w=_,x=O}if(M===m.length)return l(r,x),L&&Xe(r,M),E;if(x===null){for(;M<m.length;M++)x=v(r,m[M],g),x!==null&&(f=u(x,f,M),w===null?E=x:w.sibling=x,w=x);return L&&Xe(r,M),E}for(x=a(x);M<m.length;M++)O=p(x,r,M,m[M],g),O!==null&&(t&&O.alternate!==null&&x.delete(O.key===null?M:O.key),f=u(O,f,M),w===null?E=O:w.sibling=O,w=O);return t&&x.forEach(function(q){return e(r,q)}),L&&Xe(r,M),E}function S(r,f,m,g){if(m==null)throw Error(y(151));for(var E=null,w=null,x=f,M=f=0,O=null,_=m.next();x!==null&&!_.done;M++,_=m.next()){x.index>M?(O=x,x=null):O=x.sibling;var q=d(r,x,_.value,g);if(q===null){x===null&&(x=O);break}t&&x&&q.alternate===null&&e(r,x),f=u(q,f,M),w===null?E=q:w.sibling=q,w=q,x=O}if(_.done)return l(r,x),L&&Xe(r,M),E;if(x===null){for(;!_.done;M++,_=m.next())_=v(r,_.value,g),_!==null&&(f=u(_,f,M),w===null?E=_:w.sibling=_,w=_);return L&&Xe(r,M),E}for(x=a(x);!_.done;M++,_=m.next())_=p(x,r,M,_.value,g),_!==null&&(t&&_.alternate!==null&&x.delete(_.key===null?M:_.key),f=u(_,f,M),w===null?E=_:w.sibling=_,w=_);return t&&x.forEach(function(nt){return e(r,nt)}),L&&Xe(r,M),E}function T(r,f,m,g){if(typeof m=="object"&&m!==null&&m.type===fa&&m.key===null&&(m=m.props.children),typeof m=="object"&&m!==null){switch(m.$$typeof){case ru:t:{for(var E=m.key;f!==null;){if(f.key===E){if(E=m.type,E===fa){if(f.tag===7){l(r,f.sibling),g=n(f,m.props.children),g.return=r,r=g;break t}}else if(f.elementType===E||typeof E=="object"&&E!==null&&E.$$typeof===al&&Rl(E)===f.type){l(r,f.sibling),g=n(f,m.props),ln(g,m),g.return=r,r=g;break t}l(r,f);break}else e(r,f);f=f.sibling}m.type===fa?(g=kl(m.props.children,r.mode,g,m.key),g.return=r,r=g):(g=Nu(m.type,m.key,m.props,null,r.mode,g),ln(g,m),g.return=r,r=g)}return i(r);case cn:t:{for(E=m.key;f!==null;){if(f.key===E)if(f.tag===4&&f.stateNode.containerInfo===m.containerInfo&&f.stateNode.implementation===m.implementation){l(r,f.sibling),g=n(f,m.children||[]),g.return=r,r=g;break t}else{l(r,f);break}else e(r,f);f=f.sibling}g=To(m,r.mode,g),g.return=r,r=g}return i(r);case al:return m=Rl(m),T(r,f,m,g)}if(fn(m))return b(r,f,m,g);if(tn(m)){if(E=tn(m),typeof E!="function")throw Error(y(150));return m=E.call(m),S(r,f,m,g)}if(typeof m.then=="function")return T(r,f,bu(m),g);if(m.$$typeof===je)return T(r,f,yu(r,m),g);_u(r,m)}return typeof m=="string"&&m!==""||typeof m=="number"||typeof m=="bigint"?(m=""+m,f!==null&&f.tag===6?(l(r,f.sibling),g=n(f,m),g.return=r,r=g):(l(r,f),g=Eo(m,r.mode,g),g.return=r,r=g),i(r)):l(r,f)}return function(r,f,m,g){try{Nn=0;var E=T(r,f,m,g);return Ta=null,E}catch(x){if(x===La||x===Ei)throw x;var w=Ft(29,x,null,r.mode);return w.lanes=g,w.return=r,w}finally{}}}var jl=Yd(!0),Gd=Yd(!1),nl=!1;function cf(t){t.updateQueue={baseState:t.memoizedState,firstBaseUpdate:null,lastBaseUpdate:null,shared:{pending:null,lanes:0,hiddenCallbacks:null},callbacks:null}}function pc(t,e){t=t.updateQueue,e.updateQueue===t&&(e.updateQueue={baseState:t.baseState,firstBaseUpdate:t.firstBaseUpdate,lastBaseUpdate:t.lastBaseUpdate,shared:t.shared,callbacks:null})}function vl(t){return{lane:t,tag:0,payload:null,callback:null,next:null}}function gl(t,e,l){var a=t.updateQueue;if(a===null)return null;if(a=a.shared,Q&2){var n=a.pending;return n===null?e.next=e:(e.next=n.next,n.next=e),a.pending=e,e=Wu(t),Od(t,null,l),e}return zi(t,a,e,l),Wu(t)}function gn(t,e,l){if(e=e.updateQueue,e!==null&&(e=e.shared,(l&4194048)!==0)){var a=e.lanes;a&=t.pendingLanes,l|=a,e.lanes=l,nd(t,l)}}function Mo(t,e){var l=t.updateQueue,a=t.alternate;if(a!==null&&(a=a.updateQueue,l===a)){var n=null,u=null;if(l=l.firstBaseUpdate,l!==null){do{var i={lane:l.lane,tag:l.tag,payload:l.payload,callback:null,next:null};u===null?n=u=i:u=u.next=i,l=l.next}while(l!==null);u===null?n=u=e:u=u.next=e}else n=u=e;l={baseState:a.baseState,firstBaseUpdate:n,lastBaseUpdate:u,shared:a.shared,callbacks:a.callbacks},t.updateQueue=l;return}t=l.lastBaseUpdate,t===null?l.firstBaseUpdate=e:t.next=e,l.lastBaseUpdate=e}var hc=!1;function yn(){if(hc){var t=Ea;if(t!==null)throw t}}function bn(t,e,l,a){hc=!1;var n=t.updateQueue;nl=!1;var u=n.firstBaseUpdate,i=n.lastBaseUpdate,o=n.shared.pending;if(o!==null){n.shared.pending=null;var c=o,s=c.next;c.next=null,i===null?u=s:i.next=s,i=c;var h=t.alternate;h!==null&&(h=h.updateQueue,o=h.lastBaseUpdate,o!==i&&(o===null?h.firstBaseUpdate=s:o.next=s,h.lastBaseUpdate=c))}if(u!==null){var v=n.baseState;i=0,h=s=c=null,o=u;do{var d=o.lane&-536870913,p=d!==o.lane;if(p?(X&d)===d:(a&d)===d){d!==0&&d===Oa&&(hc=!0),h!==null&&(h=h.next={lane:0,tag:o.tag,payload:o.payload,callback:null,next:null});t:{var b=t,S=o;d=e;var T=l;switch(S.tag){case 1:if(b=S.payload,typeof b=="function"){v=b.call(T,v,d);break t}v=b;break t;case 3:b.flags=b.flags&-65537|128;case 0:if(b=S.payload,d=typeof b=="function"?b.call(T,v,d):b,d==null)break t;v=et({},v,d);break t;case 2:nl=!0}}d=o.callback,d!==null&&(t.flags|=64,p&&(t.flags|=8192),p=n.callbacks,p===null?n.callbacks=[d]:p.push(d))}else p={lane:d,tag:o.tag,payload:o.payload,callback:o.callback,next:null},h===null?(s=h=p,c=v):h=h.next=p,i|=d;if(o=o.next,o===null){if(o=n.shared.pending,o===null)break;p=o,o=p.next,p.next=null,n.lastBaseUpdate=p,n.shared.pending=null}}while(!0);h===null&&(c=v),n.baseState=c,n.firstBaseUpdate=s,n.lastBaseUpdate=h,u===null&&(n.shared.lanes=0),Al|=i,t.lanes=i,t.memoizedState=v}}function Xd(t,e){if(typeof t!="function")throw Error(y(191,t));t.call(e)}function Ld(t,e){var l=t.callbacks;if(l!==null)for(t.callbacks=null,t=0;t<l.length;t++)Xd(l[t],e)}var Na=we(null),Pu=we(0);function Fr(t,e){t=Pe,F(Pu,t),F(Na,e),Pe=t|e.baseLanes}function vc(){F(Pu,Pe),F(Na,Na.current)}function ff(){Pe=Pu.current,At(Na),At(Pu)}var ne=we(null),pe=null;function il(t){var e=t.alternate;F(st,st.current&1),F(ne,t),pe===null&&(e===null||Na.current!==null||e.memoizedState!==null)&&(pe=t)}function gc(t){F(st,st.current),F(ne,t),pe===null&&(pe=t)}function jd(t){t.tag===22?(F(st,st.current),F(ne,t),pe===null&&(pe=t)):ol(t)}function ol(){F(st,st.current),F(ne,ne.current)}function $t(t){At(ne),pe===t&&(pe=null),At(st)}var st=we(0);function ti(t){for(var e=t;e!==null;){if(e.tag===13){var l=e.memoizedState;if(l!==null&&(l=l.dehydrated,l===null||Hc(l)||kc(l)))return e}else if(e.tag===19&&(e.memoizedProps.revealOrder==="forwards"||e.memoizedProps.revealOrder==="backwards"||e.memoizedProps.revealOrder==="unstable_legacy-backwards"||e.memoizedProps.revealOrder==="together")){if(e.flags&128)return e}else if(e.child!==null){e.child.return=e,e=e.child;continue}if(e===t)break;for(;e.sibling===null;){if(e.return===null||e.return===t)return null;e=e.return}e.sibling.return=e.return,e=e.sibling}return null}var $e=0,H=null,W=null,pt=null,ei=!1,Aa=!1,Ql=!1,li=0,Cn=0,Ma=null,iv=0;function ct(){throw Error(y(321))}function rf(t,e){if(e===null)return!1;for(var l=0;l<e.length&&l<t.length;l++)if(!ae(t[l],e[l]))return!1;return!0}function sf(t,e,l,a,n,u){return $e=u,H=e,e.memoizedState=null,e.updateQueue=null,e.lanes=0,C.H=t===null||t.memoizedState===null?_m:xf,Ql=!1,u=l(a,n),Ql=!1,Aa&&(u=Zd(e,l,a,n)),Qd(t),u}function Qd(t){C.H=Rn;var e=W!==null&&W.next!==null;if($e=0,pt=W=H=null,ei=!1,Cn=0,Ma=null,e)throw Error(y(300));t===null||gt||(t=t.dependencies,t!==null&&Fu(t)&&(gt=!0))}function Zd(t,e,l,a){H=t;var n=0;do{if(Aa&&(Ma=null),Cn=0,Aa=!1,25<=n)throw Error(y(301));if(n+=1,pt=W=null,t.updateQueue!=null){var u=t.updateQueue;u.lastEffect=null,u.events=null,u.stores=null,u.memoCache!=null&&(u.memoCache.index=0)}C.H=Sm,u=e(l,a)}while(Aa);return u}function ov(){var t=C.H,e=t.useState()[0];return e=typeof e.then=="function"?Kn(e):e,t=t.useState()[0],(W!==null?W.memoizedState:null)!==t&&(H.flags|=1024),e}function df(){var t=li!==0;return li=0,t}function mf(t,e,l){e.updateQueue=t.updateQueue,e.flags&=-2053,t.lanes&=~l}function pf(t){if(ei){for(t=t.memoizedState;t!==null;){var e=t.queue;e!==null&&(e.pending=null),t=t.next}ei=!1}$e=0,pt=W=H=null,Aa=!1,Cn=li=0,Ma=null}function Ut(){var t={memoizedState:null,baseState:null,baseQueue:null,queue:null,next:null};return pt===null?H.memoizedState=pt=t:pt=pt.next=t,pt}function dt(){if(W===null){var t=H.alternate;t=t!==null?t.memoizedState:null}else t=W.next;var e=pt===null?H.memoizedState:pt.next;if(e!==null)pt=e,W=t;else{if(t===null)throw H.alternate===null?Error(y(467)):Error(y(310));W=t,t={memoizedState:W.memoizedState,baseState:W.baseState,baseQueue:W.baseQueue,queue:W.queue,next:null},pt===null?H.memoizedState=pt=t:pt=pt.next=t}return pt}function Ti(){return{lastEffect:null,events:null,stores:null,memoCache:null}}function Kn(t){var e=Cn;return Cn+=1,Ma===null&&(Ma=[]),t=Bd(Ma,t,e),e=H,(pt===null?e.memoizedState:pt.next)===null&&(e=e.alternate,C.H=e===null||e.memoizedState===null?_m:xf),t}function Ai(t){if(t!==null&&typeof t=="object"){if(typeof t.then=="function")return Kn(t);if(t.$$typeof===je)return wt(t)}throw Error(y(438,String(t)))}function hf(t){var e=null,l=H.updateQueue;if(l!==null&&(e=l.memoCache),e==null){var a=H.alternate;a!==null&&(a=a.updateQueue,a!==null&&(a=a.memoCache,a!=null&&(e={data:a.data.map(function(n){return n.slice()}),index:0})))}if(e==null&&(e={data:[],index:0}),l===null&&(l=Ti(),H.updateQueue=l),l.memoCache=e,l=e.data[e.index],l===void 0)for(l=e.data[e.index]=Array(t),a=0;a<t;a++)l[a]=V0;return e.index++,l}function Fe(t,e){return typeof e=="function"?e(t):e}function Ru(t){var e=dt();return vf(e,W,t)}function vf(t,e,l){var a=t.queue;if(a===null)throw Error(y(311));a.lastRenderedReducer=l;var n=t.baseQueue,u=a.pending;if(u!==null){if(n!==null){var i=n.next;n.next=u.next,u.next=i}e.baseQueue=n=u,a.pending=null}if(u=t.baseState,n===null)t.memoizedState=u;else{e=n.next;var o=i=null,c=null,s=e,h=!1;do{var v=s.lane&-536870913;if(v!==s.lane?(X&v)===v:($e&v)===v){var d=s.revertLane;if(d===0)c!==null&&(c=c.next={lane:0,revertLane:0,gesture:null,action:s.action,hasEagerState:s.hasEagerState,eagerState:s.eagerState,next:null}),v===Oa&&(h=!0);else if(($e&d)===d){s=s.next,d===Oa&&(h=!0);continue}else v={lane:0,revertLane:s.revertLane,gesture:null,action:s.action,hasEagerState:s.hasEagerState,eagerState:s.eagerState,next:null},c===null?(o=c=v,i=u):c=c.next=v,H.lanes|=d,Al|=d;v=s.action,Ql&&l(u,v),u=s.hasEagerState?s.eagerState:l(u,v)}else d={lane:v,revertLane:s.revertLane,gesture:s.gesture,action:s.action,hasEagerState:s.hasEagerState,eagerState:s.eagerState,next:null},c===null?(o=c=d,i=u):c=c.next=d,H.lanes|=v,Al|=v;s=s.next}while(s!==null&&s!==e);if(c===null?i=u:c.next=o,!ae(u,t.memoizedState)&&(gt=!0,h&&(l=Ea,l!==null)))throw l;t.memoizedState=u,t.baseState=i,t.baseQueue=c,a.lastRenderedState=u}return n===null&&(a.lanes=0),[t.memoizedState,a.dispatch]}function Do(t){var e=dt(),l=e.queue;if(l===null)throw Error(y(311));l.lastRenderedReducer=t;var a=l.dispatch,n=l.pending,u=e.memoizedState;if(n!==null){l.pending=null;var i=n=n.next;do u=t(u,i.action),i=i.next;while(i!==n);ae(u,e.memoizedState)||(gt=!0),e.memoizedState=u,e.baseQueue===null&&(e.baseState=u),l.lastRenderedState=u}return[u,a]}function Vd(t,e,l){var a=H,n=dt(),u=L;if(u){if(l===void 0)throw Error(y(407));l=l()}else l=e();var i=!ae((W||n).memoizedState,l);if(i&&(n.memoizedState=l,gt=!0),n=n.queue,gf(Wd.bind(null,a,n,t),[t]),n.getSnapshot!==e||i||pt!==null&&pt.memoizedState.tag&1){if(a.flags|=2048,Ca(9,{destroy:void 0},Jd.bind(null,a,n,l,e),null),$===null)throw Error(y(349));u||$e&127||Kd(a,e,l)}return l}function Kd(t,e,l){t.flags|=16384,t={getSnapshot:e,value:l},e=H.updateQueue,e===null?(e=Ti(),H.updateQueue=e,e.stores=[t]):(l=e.stores,l===null?e.stores=[t]:l.push(t))}function Jd(t,e,l,a){e.value=l,e.getSnapshot=a,$d(e)&&Fd(t)}function Wd(t,e,l){return l(function(){$d(e)&&Fd(t)})}function $d(t){var e=t.getSnapshot;t=t.value;try{var l=e();return!ae(t,l)}catch{return!0}}function Fd(t){var e=Wl(t,2);e!==null&&Qt(e,t,2)}function yc(t){var e=Ut();if(typeof t=="function"){var l=t;if(t=l(),Ql){fl(!0);try{l()}finally{fl(!1)}}}return e.memoizedState=e.baseState=t,e.queue={pending:null,lanes:0,dispatch:null,lastRenderedReducer:Fe,lastRenderedState:t},e}function Id(t,e,l,a){return t.baseState=l,vf(t,W,typeof a=="function"?a:Fe)}function cv(t,e,l,a,n){if(Di(t))throw Error(y(485));if(t=e.action,t!==null){var u={payload:n,action:t,next:null,isTransition:!0,status:"pending",value:null,reason:null,listeners:[],then:function(i){u.listeners.push(i)}};C.T!==null?l(!0):u.isTransition=!1,a(u),l=e.pending,l===null?(u.next=e.pending=u,Pd(e,u)):(u.next=l.next,e.pending=l.next=u)}}function Pd(t,e){var l=e.action,a=e.payload,n=t.state;if(e.isTransition){var u=C.T,i={};C.T=i;try{var o=l(n,a),c=C.S;c!==null&&c(i,o),Ir(t,e,o)}catch(s){bc(t,e,s)}finally{u!==null&&i.types!==null&&(u.types=i.types),C.T=u}}else try{u=l(n,a),Ir(t,e,u)}catch(s){bc(t,e,s)}}function Ir(t,e,l){l!==null&&typeof l=="object"&&typeof l.then=="function"?l.then(function(a){Pr(t,e,a)},function(a){return bc(t,e,a)}):Pr(t,e,l)}function Pr(t,e,l){e.status="fulfilled",e.value=l,tm(e),t.state=l,e=t.pending,e!==null&&(l=e.next,l===e?t.pending=null:(l=l.next,e.next=l,Pd(t,l)))}function bc(t,e,l){var a=t.pending;if(t.pending=null,a!==null){a=a.next;do e.status="rejected",e.reason=l,tm(e),e=e.next;while(e!==a)}t.action=null}function tm(t){t=t.listeners;for(var e=0;e<t.length;e++)(0,t[e])()}function em(t,e){return e}function ts(t,e){if(L){var l=$.formState;if(l!==null){t:{var a=H;if(L){if(tt){e:{for(var n=tt,u=me;n.nodeType!==8;){if(!u){n=null;break e}if(n=he(n.nextSibling),n===null){n=null;break e}}u=n.data,n=u==="F!"||u==="F"?n:null}if(n){tt=he(n.nextSibling),a=n.data==="F!";break t}}El(a)}a=!1}a&&(e=l[0])}}return l=Ut(),l.memoizedState=l.baseState=e,a={pending:null,lanes:0,dispatch:null,lastRenderedReducer:em,lastRenderedState:e},l.queue=a,l=gm.bind(null,H,a),a.dispatch=l,a=yc(!1),u=Sf.bind(null,H,!1,a.queue),a=Ut(),n={state:e,dispatch:null,action:t,pending:null},a.queue=n,l=cv.bind(null,H,n,u,l),n.dispatch=l,a.memoizedState=t,[e,l,!1]}function es(t){var e=dt();return lm(e,W,t)}function lm(t,e,l){if(e=vf(t,e,em)[0],t=Ru(Fe)[0],typeof e=="object"&&e!==null&&typeof e.then=="function")try{var a=Kn(e)}catch(i){throw i===La?Ei:i}else a=e;e=dt();var n=e.queue,u=n.dispatch;return l!==e.memoizedState&&(H.flags|=2048,Ca(9,{destroy:void 0},fv.bind(null,n,l),null)),[a,u,t]}function fv(t,e){t.action=e}function ls(t){var e=dt(),l=W;if(l!==null)return lm(e,l,t);dt(),e=e.memoizedState,l=dt();var a=l.queue.dispatch;return l.memoizedState=t,[e,a,!1]}function Ca(t,e,l,a){return t={tag:t,create:l,deps:a,inst:e,next:null},e=H.updateQueue,e===null&&(e=Ti(),H.updateQueue=e),l=e.lastEffect,l===null?e.lastEffect=t.next=t:(a=l.next,l.next=t,t.next=a,e.lastEffect=t),t}function am(){return dt().memoizedState}function Uu(t,e,l,a){var n=Ut();H.flags|=t,n.memoizedState=Ca(1|e,{destroy:void 0},l,a===void 0?null:a)}function Mi(t,e,l,a){var n=dt();a=a===void 0?null:a;var u=n.memoizedState.inst;W!==null&&a!==null&&rf(a,W.memoizedState.deps)?n.memoizedState=Ca(e,u,l,a):(H.flags|=t,n.memoizedState=Ca(1|e,u,l,a))}function as(t,e){Uu(8390656,8,t,e)}function gf(t,e){Mi(2048,8,t,e)}function rv(t){H.flags|=4;var e=H.updateQueue;if(e===null)e=Ti(),H.updateQueue=e,e.events=[t];else{var l=e.events;l===null?e.events=[t]:l.push(t)}}function nm(t){var e=dt().memoizedState;return rv({ref:e,nextImpl:t}),function(){if(Q&2)throw Error(y(440));return e.impl.apply(void 0,arguments)}}function um(t,e){return Mi(4,2,t,e)}function im(t,e){return Mi(4,4,t,e)}function om(t,e){if(typeof e=="function"){t=t();var l=e(t);return function(){typeof l=="function"?l():e(null)}}if(e!=null)return t=t(),e.current=t,function(){e.current=null}}function cm(t,e,l){l=l!=null?l.concat([t]):null,Mi(4,4,om.bind(null,e,t),l)}function yf(){}function fm(t,e){var l=dt();e=e===void 0?null:e;var a=l.memoizedState;return e!==null&&rf(e,a[1])?a[0]:(l.memoizedState=[t,e],t)}function rm(t,e){var l=dt();e=e===void 0?null:e;var a=l.memoizedState;if(e!==null&&rf(e,a[1]))return a[0];if(a=t(),Ql){fl(!0);try{t()}finally{fl(!1)}}return l.memoizedState=[a,e],a}function bf(t,e,l){return l===void 0||$e&1073741824&&!(X&261930)?t.memoizedState=e:(t.memoizedState=l,t=Im(),H.lanes|=t,Al|=t,l)}function sm(t,e,l,a){return ae(l,e)?l:Na.current!==null?(t=bf(t,l,a),ae(t,e)||(gt=!0),t):!($e&42)||$e&1073741824&&!(X&261930)?(gt=!0,t.memoizedState=l):(t=Im(),H.lanes|=t,Al|=t,e)}function dm(t,e,l,a,n){var u=Z.p;Z.p=u!==0&&8>u?u:8;var i=C.T,o={};C.T=o,Sf(t,!1,e,l);try{var c=n(),s=C.S;if(s!==null&&s(o,c),c!==null&&typeof c=="object"&&typeof c.then=="function"){var h=uv(c,a);_n(t,e,h,le(t))}else _n(t,e,a,le(t))}catch(v){_n(t,e,{then:function(){},status:"rejected",reason:v},le())}finally{Z.p=u,i!==null&&o.types!==null&&(i.types=o.types),C.T=i}}function sv(){}function _c(t,e,l,a){if(t.tag!==5)throw Error(y(476));var n=mm(t).queue;dm(t,n,e,Hl,l===null?sv:function(){return pm(t),l(a)})}function mm(t){var e=t.memoizedState;if(e!==null)return e;e={memoizedState:Hl,baseState:Hl,baseQueue:null,queue:{pending:null,lanes:0,dispatch:null,lastRenderedReducer:Fe,lastRenderedState:Hl},next:null};var l={};return e.next={memoizedState:l,baseState:l,baseQueue:null,queue:{pending:null,lanes:0,dispatch:null,lastRenderedReducer:Fe,lastRenderedState:l},next:null},t.memoizedState=e,t=t.alternate,t!==null&&(t.memoizedState=e),e}function pm(t){var e=mm(t);e.next===null&&(e=t.alternate.memoizedState),_n(t,e.next.queue,{},le())}function _f(){return wt(kn)}function hm(){return dt().memoizedState}function vm(){return dt().memoizedState}function dv(t){for(var e=t.return;e!==null;){switch(e.tag){case 24:case 3:var l=le();t=vl(l);var a=gl(e,t,l);a!==null&&(Qt(a,e,l),gn(a,e,l)),e={cache:nf()},t.payload=e;return}e=e.return}}function mv(t,e,l){var a=le();l={lane:a,revertLane:0,gesture:null,action:l,hasEagerState:!1,eagerState:null,next:null},Di(t)?ym(e,l):(l=tf(t,e,l,a),l!==null&&(Qt(l,t,a),bm(l,e,a)))}function gm(t,e,l){var a=le();_n(t,e,l,a)}function _n(t,e,l,a){var n={lane:a,revertLane:0,gesture:null,action:l,hasEagerState:!1,eagerState:null,next:null};if(Di(t))ym(e,n);else{var u=t.alternate;if(t.lanes===0&&(u===null||u.lanes===0)&&(u=e.lastRenderedReducer,u!==null))try{var i=e.lastRenderedState,o=u(i,l);if(n.hasEagerState=!0,n.eagerState=o,ae(o,i))return zi(t,e,n,0),$===null&&xi(),!1}catch{}finally{}if(l=tf(t,e,n,a),l!==null)return Qt(l,t,a),bm(l,e,a),!0}return!1}function Sf(t,e,l,a){if(a={lane:2,revertLane:wf(),gesture:null,action:a,hasEagerState:!1,eagerState:null,next:null},Di(t)){if(e)throw Error(y(479))}else e=tf(t,l,a,2),e!==null&&Qt(e,t,2)}function Di(t){var e=t.alternate;return t===H||e!==null&&e===H}function ym(t,e){Aa=ei=!0;var l=t.pending;l===null?e.next=e:(e.next=l.next,l.next=e),t.pending=e}function bm(t,e,l){if(l&4194048){var a=e.lanes;a&=t.pendingLanes,l|=a,e.lanes=l,nd(t,l)}}var Rn={readContext:wt,use:Ai,useCallback:ct,useContext:ct,useEffect:ct,useImperativeHandle:ct,useLayoutEffect:ct,useInsertionEffect:ct,useMemo:ct,useReducer:ct,useRef:ct,useState:ct,useDebugValue:ct,useDeferredValue:ct,useTransition:ct,useSyncExternalStore:ct,useId:ct,useHostTransitionStatus:ct,useFormState:ct,useActionState:ct,useOptimistic:ct,useMemoCache:ct,useCacheRefresh:ct};Rn.useEffectEvent=ct;var _m={readContext:wt,use:Ai,useCallback:function(t,e){return Ut().memoizedState=[t,e===void 0?null:e],t},useContext:wt,useEffect:as,useImperativeHandle:function(t,e,l){l=l!=null?l.concat([t]):null,Uu(4194308,4,om.bind(null,e,t),l)},useLayoutEffect:function(t,e){return Uu(4194308,4,t,e)},useInsertionEffect:function(t,e){Uu(4,2,t,e)},useMemo:function(t,e){var l=Ut();e=e===void 0?null:e;var a=t();if(Ql){fl(!0);try{t()}finally{fl(!1)}}return l.memoizedState=[a,e],a},useReducer:function(t,e,l){var a=Ut();if(l!==void 0){var n=l(e);if(Ql){fl(!0);try{l(e)}finally{fl(!1)}}}else n=e;return a.memoizedState=a.baseState=n,t={pending:null,lanes:0,dispatch:null,lastRenderedReducer:t,lastRenderedState:n},a.queue=t,t=t.dispatch=mv.bind(null,H,t),[a.memoizedState,t]},useRef:function(t){var e=Ut();return t={current:t},e.memoizedState=t},useState:function(t){t=yc(t);var e=t.queue,l=gm.bind(null,H,e);return e.dispatch=l,[t.memoizedState,l]},useDebugValue:yf,useDeferredValue:function(t,e){var l=Ut();return bf(l,t,e)},useTransition:function(){var t=yc(!1);return t=dm.bind(null,H,t.queue,!0,!1),Ut().memoizedState=t,[!1,t]},useSyncExternalStore:function(t,e,l){var a=H,n=Ut();if(L){if(l===void 0)throw Error(y(407));l=l()}else{if(l=e(),$===null)throw Error(y(349));X&127||Kd(a,e,l)}n.memoizedState=l;var u={value:l,getSnapshot:e};return n.queue=u,as(Wd.bind(null,a,u,t),[t]),a.flags|=2048,Ca(9,{destroy:void 0},Jd.bind(null,a,u,l,e),null),l},useId:function(){var t=Ut(),e=$.identifierPrefix;if(L){var l=Me,a=Ae;l=(a&~(1<<32-ee(a)-1)).toString(32)+l,e="_"+e+"R_"+l,l=li++,0<l&&(e+="H"+l.toString(32)),e+="_"}else l=iv++,e="_"+e+"r_"+l.toString(32)+"_";return t.memoizedState=e},useHostTransitionStatus:_f,useFormState:ts,useActionState:ts,useOptimistic:function(t){var e=Ut();e.memoizedState=e.baseState=t;var l={pending:null,lanes:0,dispatch:null,lastRenderedReducer:null,lastRenderedState:null};return e.queue=l,e=Sf.bind(null,H,!0,l),l.dispatch=e,[t,e]},useMemoCache:hf,useCacheRefresh:function(){return Ut().memoizedState=dv.bind(null,H)},useEffectEvent:function(t){var e=Ut(),l={impl:t};return e.memoizedState=l,function(){if(Q&2)throw Error(y(440));return l.impl.apply(void 0,arguments)}}},xf={readContext:wt,use:Ai,useCallback:fm,useContext:wt,useEffect:gf,useImperativeHandle:cm,useInsertionEffect:um,useLayoutEffect:im,useMemo:rm,useReducer:Ru,useRef:am,useState:function(){return Ru(Fe)},useDebugValue:yf,useDeferredValue:function(t,e){var l=dt();return sm(l,W.memoizedState,t,e)},useTransition:function(){var t=Ru(Fe)[0],e=dt().memoizedState;return[typeof t=="boolean"?t:Kn(t),e]},useSyncExternalStore:Vd,useId:hm,useHostTransitionStatus:_f,useFormState:es,useActionState:es,useOptimistic:function(t,e){var l=dt();return Id(l,W,t,e)},useMemoCache:hf,useCacheRefresh:vm};xf.useEffectEvent=nm;var Sm={readContext:wt,use:Ai,useCallback:fm,useContext:wt,useEffect:gf,useImperativeHandle:cm,useInsertionEffect:um,useLayoutEffect:im,useMemo:rm,useReducer:Do,useRef:am,useState:function(){return Do(Fe)},useDebugValue:yf,useDeferredValue:function(t,e){var l=dt();return W===null?bf(l,t,e):sm(l,W.memoizedState,t,e)},useTransition:function(){var t=Do(Fe)[0],e=dt().memoizedState;return[typeof t=="boolean"?t:Kn(t),e]},useSyncExternalStore:Vd,useId:hm,useHostTransitionStatus:_f,useFormState:ls,useActionState:ls,useOptimistic:function(t,e){var l=dt();return W!==null?Id(l,W,t,e):(l.baseState=t,[t,l.queue.dispatch])},useMemoCache:hf,useCacheRefresh:vm};Sm.useEffectEvent=nm;function qo(t,e,l,a){e=t.memoizedState,l=l(a,e),l=l==null?e:et({},e,l),t.memoizedState=l,t.lanes===0&&(t.updateQueue.baseState=l)}var Sc={enqueueSetState:function(t,e,l){t=t._reactInternals;var a=le(),n=vl(a);n.payload=e,l!=null&&(n.callback=l),e=gl(t,n,a),e!==null&&(Qt(e,t,a),gn(e,t,a))},enqueueReplaceState:function(t,e,l){t=t._reactInternals;var a=le(),n=vl(a);n.tag=1,n.payload=e,l!=null&&(n.callback=l),e=gl(t,n,a),e!==null&&(Qt(e,t,a),gn(e,t,a))},enqueueForceUpdate:function(t,e){t=t._reactInternals;var l=le(),a=vl(l);a.tag=2,e!=null&&(a.callback=e),e=gl(t,a,l),e!==null&&(Qt(e,t,l),gn(e,t,l))}};function ns(t,e,l,a,n,u,i){return t=t.stateNode,typeof t.shouldComponentUpdate=="function"?t.shouldComponentUpdate(a,u,i):e.prototype&&e.prototype.isPureReactComponent?!qn(l,a)||!qn(n,u):!0}function us(t,e,l,a){t=e.state,typeof e.componentWillReceiveProps=="function"&&e.componentWillReceiveProps(l,a),typeof e.UNSAFE_componentWillReceiveProps=="function"&&e.UNSAFE_componentWillReceiveProps(l,a),e.state!==t&&Sc.enqueueReplaceState(e,e.state,null)}function Zl(t,e){var l=e;if("ref"in e){l={};for(var a in e)a!=="ref"&&(l[a]=e[a])}if(t=t.defaultProps){l===e&&(l=et({},l));for(var n in t)l[n]===void 0&&(l[n]=t[n])}return l}function xm(t){Ju(t)}function zm(t){console.error(t)}function Em(t){Ju(t)}function ai(t,e){try{var l=t.onUncaughtError;l(e.value,{componentStack:e.stack})}catch(a){setTimeout(function(){throw a})}}function is(t,e,l){try{var a=t.onCaughtError;a(l.value,{componentStack:l.stack,errorBoundary:e.tag===1?e.stateNode:null})}catch(n){setTimeout(function(){throw n})}}function xc(t,e,l){return l=vl(l),l.tag=3,l.payload={element:null},l.callback=function(){ai(t,e)},l}function Tm(t){return t=vl(t),t.tag=3,t}function Am(t,e,l,a){var n=l.type.getDerivedStateFromError;if(typeof n=="function"){var u=a.value;t.payload=function(){return n(u)},t.callback=function(){is(e,l,a)}}var i=l.stateNode;i!==null&&typeof i.componentDidCatch=="function"&&(t.callback=function(){is(e,l,a),typeof n!="function"&&(yl===null?yl=new Set([this]):yl.add(this));var o=a.stack;this.componentDidCatch(a.value,{componentStack:o!==null?o:""})})}function pv(t,e,l,a,n){if(l.flags|=32768,a!==null&&typeof a=="object"&&typeof a.then=="function"){if(e=l.alternate,e!==null&&Xa(e,l,n,!0),l=ne.current,l!==null){switch(l.tag){case 31:case 13:return pe===null?ci():l.alternate===null&&ft===0&&(ft=3),l.flags&=-257,l.flags|=65536,l.lanes=n,a===Iu?l.flags|=16384:(e=l.updateQueue,e===null?l.updateQueue=new Set([a]):e.add(a),Go(t,a,n)),!1;case 22:return l.flags|=65536,a===Iu?l.flags|=16384:(e=l.updateQueue,e===null?(e={transitions:null,markerInstances:null,retryQueue:new Set([a])},l.updateQueue=e):(l=e.retryQueue,l===null?e.retryQueue=new Set([a]):l.add(a)),Go(t,a,n)),!1}throw Error(y(435,l.tag))}return Go(t,a,n),ci(),!1}if(L)return e=ne.current,e!==null?(!(e.flags&65536)&&(e.flags|=256),e.flags|=65536,e.lanes=n,a!==fc&&(t=Error(y(422),{cause:a}),On(de(t,l)))):(a!==fc&&(e=Error(y(423),{cause:a}),On(de(e,l))),t=t.current.alternate,t.flags|=65536,n&=-n,t.lanes|=n,a=de(a,l),n=xc(t.stateNode,a,n),Mo(t,n),ft!==4&&(ft=2)),!1;var u=Error(y(520),{cause:a});if(u=de(u,l),zn===null?zn=[u]:zn.push(u),ft!==4&&(ft=2),e===null)return!0;a=de(a,l),l=e;do{switch(l.tag){case 3:return l.flags|=65536,t=n&-n,l.lanes|=t,t=xc(l.stateNode,a,t),Mo(l,t),!1;case 1:if(e=l.type,u=l.stateNode,(l.flags&128)===0&&(typeof e.getDerivedStateFromError=="function"||u!==null&&typeof u.componentDidCatch=="function"&&(yl===null||!yl.has(u))))return l.flags|=65536,n&=-n,l.lanes|=n,n=Tm(n),Am(n,t,l,a),Mo(l,n),!1}l=l.return}while(l!==null);return!1}var zf=Error(y(461)),gt=!1;function Mt(t,e,l,a){e.child=t===null?Gd(e,null,l,a):jl(e,t.child,l,a)}function os(t,e,l,a,n){l=l.render;var u=e.ref;if("ref"in a){var i={};for(var o in a)o!=="ref"&&(i[o]=a[o])}else i=a;return Ll(e),a=sf(t,e,l,i,u,n),o=df(),t!==null&&!gt?(mf(t,e,n),Ie(t,e,n)):(L&&o&&lf(e),e.flags|=1,Mt(t,e,a,n),e.child)}function cs(t,e,l,a,n){if(t===null){var u=l.type;return typeof u=="function"&&!ef(u)&&u.defaultProps===void 0&&l.compare===null?(e.tag=15,e.type=u,Mm(t,e,u,a,n)):(t=Nu(l.type,null,a,e,e.mode,n),t.ref=e.ref,t.return=e,e.child=t)}if(u=t.child,!Ef(t,n)){var i=u.memoizedProps;if(l=l.compare,l=l!==null?l:qn,l(i,a)&&t.ref===e.ref)return Ie(t,e,n)}return e.flags|=1,t=Ve(u,a),t.ref=e.ref,t.return=e,e.child=t}function Mm(t,e,l,a,n){if(t!==null){var u=t.memoizedProps;if(qn(u,a)&&t.ref===e.ref)if(gt=!1,e.pendingProps=a=u,Ef(t,n))t.flags&131072&&(gt=!0);else return e.lanes=t.lanes,Ie(t,e,n)}return zc(t,e,l,a,n)}function Dm(t,e,l,a){var n=a.children,u=t!==null?t.memoizedState:null;if(t===null&&e.stateNode===null&&(e.stateNode={_visibility:1,_pendingMarkers:null,_retryCache:null,_transitions:null}),a.mode==="hidden"){if(e.flags&128){if(u=u!==null?u.baseLanes|l:l,t!==null){for(a=e.child=t.child,n=0;a!==null;)n=n|a.lanes|a.childLanes,a=a.sibling;a=n&~u}else a=0,e.child=null;return fs(t,e,u,l,a)}if(l&536870912)e.memoizedState={baseLanes:0,cachePool:null},t!==null&&Cu(e,u!==null?u.cachePool:null),u!==null?Fr(e,u):vc(),jd(e);else return a=e.lanes=536870912,fs(t,e,u!==null?u.baseLanes|l:l,l,a)}else u!==null?(Cu(e,u.cachePool),Fr(e,u),ol(e),e.memoizedState=null):(t!==null&&Cu(e,null),vc(),ol(e));return Mt(t,e,n,l),e.child}function sn(t,e){return t!==null&&t.tag===22||e.stateNode!==null||(e.stateNode={_visibility:1,_pendingMarkers:null,_retryCache:null,_transitions:null}),e.sibling}function fs(t,e,l,a,n){var u=uf();return u=u===null?null:{parent:vt._currentValue,pool:u},e.memoizedState={baseLanes:l,cachePool:u},t!==null&&Cu(e,null),vc(),jd(e),t!==null&&Xa(t,e,a,!0),e.childLanes=n,null}function Hu(t,e){return e=ni({mode:e.mode,children:e.children},t.mode),e.ref=t.ref,t.child=e,e.return=t,e}function rs(t,e,l){return jl(e,t.child,null,l),t=Hu(e,e.pendingProps),t.flags|=2,$t(e),e.memoizedState=null,t}function hv(t,e,l){var a=e.pendingProps,n=(e.flags&128)!==0;if(e.flags&=-129,t===null){if(L){if(a.mode==="hidden")return t=Hu(e,a),e.lanes=536870912,sn(null,t);if(gc(e),(t=tt)?(t=_p(t,me),t=t!==null&&t.data==="&"?t:null,t!==null&&(e.memoizedState={dehydrated:t,treeContext:zl!==null?{id:Ae,overflow:Me}:null,retryLane:536870912,hydrationErrors:null},l=Cd(t),l.return=e,e.child=l,qt=e,tt=null)):t=null,t===null)throw El(e);return e.lanes=536870912,null}return Hu(e,a)}var u=t.memoizedState;if(u!==null){var i=u.dehydrated;if(gc(e),n)if(e.flags&256)e.flags&=-257,e=rs(t,e,l);else if(e.memoizedState!==null)e.child=t.child,e.flags|=128,e=null;else throw Error(y(558));else if(gt||Xa(t,e,l,!1),n=(l&t.childLanes)!==0,gt||n){if(a=$,a!==null&&(i=ud(a,l),i!==0&&i!==u.retryLane))throw u.retryLane=i,Wl(t,i),Qt(a,t,i),zf;ci(),e=rs(t,e,l)}else t=u.treeContext,tt=he(i.nextSibling),qt=e,L=!0,hl=null,me=!1,t!==null&&Ud(e,t),e=Hu(e,a),e.flags|=4096;return e}return t=Ve(t.child,{mode:a.mode,children:a.children}),t.ref=e.ref,e.child=t,t.return=e,t}function ku(t,e){var l=e.ref;if(l===null)t!==null&&t.ref!==null&&(e.flags|=4194816);else{if(typeof l!="function"&&typeof l!="object")throw Error(y(284));(t===null||t.ref!==l)&&(e.flags|=4194816)}}function zc(t,e,l,a,n){return Ll(e),l=sf(t,e,l,a,void 0,n),a=df(),t!==null&&!gt?(mf(t,e,n),Ie(t,e,n)):(L&&a&&lf(e),e.flags|=1,Mt(t,e,l,n),e.child)}function ss(t,e,l,a,n,u){return Ll(e),e.updateQueue=null,l=Zd(e,a,l,n),Qd(t),a=df(),t!==null&&!gt?(mf(t,e,u),Ie(t,e,u)):(L&&a&&lf(e),e.flags|=1,Mt(t,e,l,u),e.child)}function ds(t,e,l,a,n){if(Ll(e),e.stateNode===null){var u=ga,i=l.contextType;typeof i=="object"&&i!==null&&(u=wt(i)),u=new l(a,u),e.memoizedState=u.state!==null&&u.state!==void 0?u.state:null,u.updater=Sc,e.stateNode=u,u._reactInternals=e,u=e.stateNode,u.props=a,u.state=e.memoizedState,u.refs={},cf(e),i=l.contextType,u.context=typeof i=="object"&&i!==null?wt(i):ga,u.state=e.memoizedState,i=l.getDerivedStateFromProps,typeof i=="function"&&(qo(e,l,i,a),u.state=e.memoizedState),typeof l.getDerivedStateFromProps=="function"||typeof u.getSnapshotBeforeUpdate=="function"||typeof u.UNSAFE_componentWillMount!="function"&&typeof u.componentWillMount!="function"||(i=u.state,typeof u.componentWillMount=="function"&&u.componentWillMount(),typeof u.UNSAFE_componentWillMount=="function"&&u.UNSAFE_componentWillMount(),i!==u.state&&Sc.enqueueReplaceState(u,u.state,null),bn(e,a,u,n),yn(),u.state=e.memoizedState),typeof u.componentDidMount=="function"&&(e.flags|=4194308),a=!0}else if(t===null){u=e.stateNode;var o=e.memoizedProps,c=Zl(l,o);u.props=c;var s=u.context,h=l.contextType;i=ga,typeof h=="object"&&h!==null&&(i=wt(h));var v=l.getDerivedStateFromProps;h=typeof v=="function"||typeof u.getSnapshotBeforeUpdate=="function",o=e.pendingProps!==o,h||typeof u.UNSAFE_componentWillReceiveProps!="function"&&typeof u.componentWillReceiveProps!="function"||(o||s!==i)&&us(e,u,a,i),nl=!1;var d=e.memoizedState;u.state=d,bn(e,a,u,n),yn(),s=e.memoizedState,o||d!==s||nl?(typeof v=="function"&&(qo(e,l,v,a),s=e.memoizedState),(c=nl||ns(e,l,c,a,d,s,i))?(h||typeof u.UNSAFE_componentWillMount!="function"&&typeof u.componentWillMount!="function"||(typeof u.componentWillMount=="function"&&u.componentWillMount(),typeof u.UNSAFE_componentWillMount=="function"&&u.UNSAFE_componentWillMount()),typeof u.componentDidMount=="function"&&(e.flags|=4194308)):(typeof u.componentDidMount=="function"&&(e.flags|=4194308),e.memoizedProps=a,e.memoizedState=s),u.props=a,u.state=s,u.context=i,a=c):(typeof u.componentDidMount=="function"&&(e.flags|=4194308),a=!1)}else{u=e.stateNode,pc(t,e),i=e.memoizedProps,h=Zl(l,i),u.props=h,v=e.pendingProps,d=u.context,s=l.contextType,c=ga,typeof s=="object"&&s!==null&&(c=wt(s)),o=l.getDerivedStateFromProps,(s=typeof o=="function"||typeof u.getSnapshotBeforeUpdate=="function")||typeof u.UNSAFE_componentWillReceiveProps!="function"&&typeof u.componentWillReceiveProps!="function"||(i!==v||d!==c)&&us(e,u,a,c),nl=!1,d=e.memoizedState,u.state=d,bn(e,a,u,n),yn();var p=e.memoizedState;i!==v||d!==p||nl||t!==null&&t.dependencies!==null&&Fu(t.dependencies)?(typeof o=="function"&&(qo(e,l,o,a),p=e.memoizedState),(h=nl||ns(e,l,h,a,d,p,c)||t!==null&&t.dependencies!==null&&Fu(t.dependencies))?(s||typeof u.UNSAFE_componentWillUpdate!="function"&&typeof u.componentWillUpdate!="function"||(typeof u.componentWillUpdate=="function"&&u.componentWillUpdate(a,p,c),typeof u.UNSAFE_componentWillUpdate=="function"&&u.UNSAFE_componentWillUpdate(a,p,c)),typeof u.componentDidUpdate=="function"&&(e.flags|=4),typeof u.getSnapshotBeforeUpdate=="function"&&(e.flags|=1024)):(typeof u.componentDidUpdate!="function"||i===t.memoizedProps&&d===t.memoizedState||(e.flags|=4),typeof u.getSnapshotBeforeUpdate!="function"||i===t.memoizedProps&&d===t.memoizedState||(e.flags|=1024),e.memoizedProps=a,e.memoizedState=p),u.props=a,u.state=p,u.context=c,a=h):(typeof u.componentDidUpdate!="function"||i===t.memoizedProps&&d===t.memoizedState||(e.flags|=4),typeof u.getSnapshotBeforeUpdate!="function"||i===t.memoizedProps&&d===t.memoizedState||(e.flags|=1024),a=!1)}return u=a,ku(t,e),a=(e.flags&128)!==0,u||a?(u=e.stateNode,l=a&&typeof l.getDerivedStateFromError!="function"?null:u.render(),e.flags|=1,t!==null&&a?(e.child=jl(e,t.child,null,n),e.child=jl(e,null,l,n)):Mt(t,e,l,n),e.memoizedState=u.state,t=e.child):t=Ie(t,e,n),t}function ms(t,e,l,a){return Xl(),e.flags|=256,Mt(t,e,l,a),e.child}var wo={dehydrated:null,treeContext:null,retryLane:0,hydrationErrors:null};function Oo(t){return{baseLanes:t,cachePool:kd()}}function No(t,e,l){return t=t!==null?t.childLanes&~l:0,e&&(t|=It),t}function qm(t,e,l){var a=e.pendingProps,n=!1,u=(e.flags&128)!==0,i;if((i=u)||(i=t!==null&&t.memoizedState===null?!1:(st.current&2)!==0),i&&(n=!0,e.flags&=-129),i=(e.flags&32)!==0,e.flags&=-33,t===null){if(L){if(n?il(e):ol(e),(t=tt)?(t=_p(t,me),t=t!==null&&t.data!=="&"?t:null,t!==null&&(e.memoizedState={dehydrated:t,treeContext:zl!==null?{id:Ae,overflow:Me}:null,retryLane:536870912,hydrationErrors:null},l=Cd(t),l.return=e,e.child=l,qt=e,tt=null)):t=null,t===null)throw El(e);return kc(t)?e.lanes=32:e.lanes=536870912,null}var o=a.children;return a=a.fallback,n?(ol(e),n=e.mode,o=ni({mode:"hidden",children:o},n),a=kl(a,n,l,null),o.return=e,a.return=e,o.sibling=a,e.child=o,a=e.child,a.memoizedState=Oo(l),a.childLanes=No(t,i,l),e.memoizedState=wo,sn(null,a)):(il(e),Ec(e,o))}var c=t.memoizedState;if(c!==null&&(o=c.dehydrated,o!==null)){if(u)e.flags&256?(il(e),e.flags&=-257,e=Co(t,e,l)):e.memoizedState!==null?(ol(e),e.child=t.child,e.flags|=128,e=null):(ol(e),o=a.fallback,n=e.mode,a=ni({mode:"visible",children:a.children},n),o=kl(o,n,l,null),o.flags|=2,a.return=e,o.return=e,a.sibling=o,e.child=a,jl(e,t.child,null,l),a=e.child,a.memoizedState=Oo(l),a.childLanes=No(t,i,l),e.memoizedState=wo,e=sn(null,a));else if(il(e),kc(o)){if(i=o.nextSibling&&o.nextSibling.dataset,i)var s=i.dgst;i=s,a=Error(y(419)),a.stack="",a.digest=i,On({value:a,source:null,stack:null}),e=Co(t,e,l)}else if(gt||Xa(t,e,l,!1),i=(l&t.childLanes)!==0,gt||i){if(i=$,i!==null&&(a=ud(i,l),a!==0&&a!==c.retryLane))throw c.retryLane=a,Wl(t,a),Qt(i,t,a),zf;Hc(o)||ci(),e=Co(t,e,l)}else Hc(o)?(e.flags|=192,e.child=t.child,e=null):(t=c.treeContext,tt=he(o.nextSibling),qt=e,L=!0,hl=null,me=!1,t!==null&&Ud(e,t),e=Ec(e,a.children),e.flags|=4096);return e}return n?(ol(e),o=a.fallback,n=e.mode,c=t.child,s=c.sibling,a=Ve(c,{mode:"hidden",children:a.children}),a.subtreeFlags=c.subtreeFlags&65011712,s!==null?o=Ve(s,o):(o=kl(o,n,l,null),o.flags|=2),o.return=e,a.return=e,a.sibling=o,e.child=a,sn(null,a),a=e.child,o=t.child.memoizedState,o===null?o=Oo(l):(n=o.cachePool,n!==null?(c=vt._currentValue,n=n.parent!==c?{parent:c,pool:c}:n):n=kd(),o={baseLanes:o.baseLanes|l,cachePool:n}),a.memoizedState=o,a.childLanes=No(t,i,l),e.memoizedState=wo,sn(t.child,a)):(il(e),l=t.child,t=l.sibling,l=Ve(l,{mode:"visible",children:a.children}),l.return=e,l.sibling=null,t!==null&&(i=e.deletions,i===null?(e.deletions=[t],e.flags|=16):i.push(t)),e.child=l,e.memoizedState=null,l)}function Ec(t,e){return e=ni({mode:"visible",children:e},t.mode),e.return=t,t.child=e}function ni(t,e){return t=Ft(22,t,null,e),t.lanes=0,t}function Co(t,e,l){return jl(e,t.child,null,l),t=Ec(e,e.pendingProps.children),t.flags|=2,e.memoizedState=null,t}function ps(t,e,l){t.lanes|=e;var a=t.alternate;a!==null&&(a.lanes|=e),sc(t.return,e,l)}function Ro(t,e,l,a,n,u){var i=t.memoizedState;i===null?t.memoizedState={isBackwards:e,rendering:null,renderingStartTime:0,last:a,tail:l,tailMode:n,treeForkCount:u}:(i.isBackwards=e,i.rendering=null,i.renderingStartTime=0,i.last=a,i.tail=l,i.tailMode=n,i.treeForkCount=u)}function wm(t,e,l){var a=e.pendingProps,n=a.revealOrder,u=a.tail;a=a.children;var i=st.current,o=(i&2)!==0;if(o?(i=i&1|2,e.flags|=128):i&=1,F(st,i),Mt(t,e,a,l),a=L?wn:0,!o&&t!==null&&t.flags&128)t:for(t=e.child;t!==null;){if(t.tag===13)t.memoizedState!==null&&ps(t,l,e);else if(t.tag===19)ps(t,l,e);else if(t.child!==null){t.child.return=t,t=t.child;continue}if(t===e)break t;for(;t.sibling===null;){if(t.return===null||t.return===e)break t;t=t.return}t.sibling.return=t.return,t=t.sibling}switch(n){case"forwards":for(l=e.child,n=null;l!==null;)t=l.alternate,t!==null&&ti(t)===null&&(n=l),l=l.sibling;l=n,l===null?(n=e.child,e.child=null):(n=l.sibling,l.sibling=null),Ro(e,!1,n,l,u,a);break;case"backwards":case"unstable_legacy-backwards":for(l=null,n=e.child,e.child=null;n!==null;){if(t=n.alternate,t!==null&&ti(t)===null){e.child=n;break}t=n.sibling,n.sibling=l,l=n,n=t}Ro(e,!0,l,null,u,a);break;case"together":Ro(e,!1,null,null,void 0,a);break;default:e.memoizedState=null}return e.child}function Ie(t,e,l){if(t!==null&&(e.dependencies=t.dependencies),Al|=e.lanes,!(l&e.childLanes))if(t!==null){if(Xa(t,e,l,!1),(l&e.childLanes)===0)return null}else return null;if(t!==null&&e.child!==t.child)throw Error(y(153));if(e.child!==null){for(t=e.child,l=Ve(t,t.pendingProps),e.child=l,l.return=e;t.sibling!==null;)t=t.sibling,l=l.sibling=Ve(t,t.pendingProps),l.return=e;l.sibling=null}return e.child}function Ef(t,e){return t.lanes&e?!0:(t=t.dependencies,!!(t!==null&&Fu(t)))}function vv(t,e,l){switch(e.tag){case 3:Qu(e,e.stateNode.containerInfo),ul(e,vt,t.memoizedState.cache),Xl();break;case 27:case 5:Io(e);break;case 4:Qu(e,e.stateNode.containerInfo);break;case 10:ul(e,e.type,e.memoizedProps.value);break;case 31:if(e.memoizedState!==null)return e.flags|=128,gc(e),null;break;case 13:var a=e.memoizedState;if(a!==null)return a.dehydrated!==null?(il(e),e.flags|=128,null):l&e.child.childLanes?qm(t,e,l):(il(e),t=Ie(t,e,l),t!==null?t.sibling:null);il(e);break;case 19:var n=(t.flags&128)!==0;if(a=(l&e.childLanes)!==0,a||(Xa(t,e,l,!1),a=(l&e.childLanes)!==0),n){if(a)return wm(t,e,l);e.flags|=128}if(n=e.memoizedState,n!==null&&(n.rendering=null,n.tail=null,n.lastEffect=null),F(st,st.current),a)break;return null;case 22:return e.lanes=0,Dm(t,e,l,e.pendingProps);case 24:ul(e,vt,t.memoizedState.cache)}return Ie(t,e,l)}function Om(t,e,l){if(t!==null)if(t.memoizedProps!==e.pendingProps)gt=!0;else{if(!Ef(t,l)&&!(e.flags&128))return gt=!1,vv(t,e,l);gt=!!(t.flags&131072)}else gt=!1,L&&e.flags&1048576&&Rd(e,wn,e.index);switch(e.lanes=0,e.tag){case 16:t:{var a=e.pendingProps;if(t=Rl(e.elementType),e.type=t,typeof t=="function")ef(t)?(a=Zl(t,a),e.tag=1,e=ds(null,e,t,a,l)):(e.tag=0,e=zc(null,e,t,a,l));else{if(t!=null){var n=t.$$typeof;if(n===Xc){e.tag=11,e=os(null,e,t,a,l);break t}else if(n===Lc){e.tag=14,e=cs(null,e,t,a,l);break t}}throw e=$o(t)||t,Error(y(306,e,""))}}return e;case 0:return zc(t,e,e.type,e.pendingProps,l);case 1:return a=e.type,n=Zl(a,e.pendingProps),ds(t,e,a,n,l);case 3:t:{if(Qu(e,e.stateNode.containerInfo),t===null)throw Error(y(387));a=e.pendingProps;var u=e.memoizedState;n=u.element,pc(t,e),bn(e,a,null,l);var i=e.memoizedState;if(a=i.cache,ul(e,vt,a),a!==u.cache&&dc(e,[vt],l,!0),yn(),a=i.element,u.isDehydrated)if(u={element:a,isDehydrated:!1,cache:i.cache},e.updateQueue.baseState=u,e.memoizedState=u,e.flags&256){e=ms(t,e,a,l);break t}else if(a!==n){n=de(Error(y(424)),e),On(n),e=ms(t,e,a,l);break t}else{switch(t=e.stateNode.containerInfo,t.nodeType){case 9:t=t.body;break;default:t=t.nodeName==="HTML"?t.ownerDocument.body:t}for(tt=he(t.firstChild),qt=e,L=!0,hl=null,me=!0,l=Gd(e,null,a,l),e.child=l;l;)l.flags=l.flags&-3|4096,l=l.sibling}else{if(Xl(),a===n){e=Ie(t,e,l);break t}Mt(t,e,a,l)}e=e.child}return e;case 26:return ku(t,e),t===null?(l=Hs(e.type,null,e.pendingProps,null))?e.memoizedState=l:L||(l=e.type,t=e.pendingProps,a=di(pl.current).createElement(l),a[Dt]=e,a[Zt]=t,Ot(a,l,t),Tt(a),e.stateNode=a):e.memoizedState=Hs(e.type,t.memoizedProps,e.pendingProps,t.memoizedState),null;case 27:return Io(e),t===null&&L&&(a=e.stateNode=Sp(e.type,e.pendingProps,pl.current),qt=e,me=!0,n=tt,Dl(e.type)?(Bc=n,tt=he(a.firstChild)):tt=n),Mt(t,e,e.pendingProps.children,l),ku(t,e),t===null&&(e.flags|=4194304),e.child;case 5:return t===null&&L&&((n=a=tt)&&(a=Qv(a,e.type,e.pendingProps,me),a!==null?(e.stateNode=a,qt=e,tt=he(a.firstChild),me=!1,n=!0):n=!1),n||El(e)),Io(e),n=e.type,u=e.pendingProps,i=t!==null?t.memoizedProps:null,a=u.children,Rc(n,u)?a=null:i!==null&&Rc(n,i)&&(e.flags|=32),e.memoizedState!==null&&(n=sf(t,e,ov,null,null,l),kn._currentValue=n),ku(t,e),Mt(t,e,a,l),e.child;case 6:return t===null&&L&&((t=l=tt)&&(l=Zv(l,e.pendingProps,me),l!==null?(e.stateNode=l,qt=e,tt=null,t=!0):t=!1),t||El(e)),null;case 13:return qm(t,e,l);case 4:return Qu(e,e.stateNode.containerInfo),a=e.pendingProps,t===null?e.child=jl(e,null,a,l):Mt(t,e,a,l),e.child;case 11:return os(t,e,e.type,e.pendingProps,l);case 7:return Mt(t,e,e.pendingProps,l),e.child;case 8:return Mt(t,e,e.pendingProps.children,l),e.child;case 12:return Mt(t,e,e.pendingProps.children,l),e.child;case 10:return a=e.pendingProps,ul(e,e.type,a.value),Mt(t,e,a.children,l),e.child;case 9:return n=e.type._context,a=e.pendingProps.children,Ll(e),n=wt(n),a=a(n),e.flags|=1,Mt(t,e,a,l),e.child;case 14:return cs(t,e,e.type,e.pendingProps,l);case 15:return Mm(t,e,e.type,e.pendingProps,l);case 19:return wm(t,e,l);case 31:return hv(t,e,l);case 22:return Dm(t,e,l,e.pendingProps);case 24:return Ll(e),a=wt(vt),t===null?(n=uf(),n===null&&(n=$,u=nf(),n.pooledCache=u,u.refCount++,u!==null&&(n.pooledCacheLanes|=l),n=u),e.memoizedState={parent:a,cache:n},cf(e),ul(e,vt,n)):(t.lanes&l&&(pc(t,e),bn(e,null,null,l),yn()),n=t.memoizedState,u=e.memoizedState,n.parent!==a?(n={parent:a,cache:a},e.memoizedState=n,e.lanes===0&&(e.memoizedState=e.updateQueue.baseState=n),ul(e,vt,a)):(a=u.cache,ul(e,vt,a),a!==n.cache&&dc(e,[vt],l,!0))),Mt(t,e,e.pendingProps.children,l),e.child;case 29:throw e.pendingProps}throw Error(y(156,e.tag))}function ke(t){t.flags|=4}function Uo(t,e,l,a,n){if((e=(t.mode&32)!==0)&&(e=!1),e){if(t.flags|=16777216,(n&335544128)===n)if(t.stateNode.complete)t.flags|=8192;else if(ep())t.flags|=8192;else throw Yl=Iu,of}else t.flags&=-16777217}function hs(t,e){if(e.type!=="stylesheet"||e.state.loading&4)t.flags&=-16777217;else if(t.flags|=16777216,!Ep(e))if(ep())t.flags|=8192;else throw Yl=Iu,of}function Su(t,e){e!==null&&(t.flags|=4),t.flags&16384&&(e=t.tag!==22?ld():536870912,t.lanes|=e,Ra|=e)}function an(t,e){if(!L)switch(t.tailMode){case"hidden":e=t.tail;for(var l=null;e!==null;)e.alternate!==null&&(l=e),e=e.sibling;l===null?t.tail=null:l.sibling=null;break;case"collapsed":l=t.tail;for(var a=null;l!==null;)l.alternate!==null&&(a=l),l=l.sibling;a===null?e||t.tail===null?t.tail=null:t.tail.sibling=null:a.sibling=null}}function P(t){var e=t.alternate!==null&&t.alternate.child===t.child,l=0,a=0;if(e)for(var n=t.child;n!==null;)l|=n.lanes|n.childLanes,a|=n.subtreeFlags&65011712,a|=n.flags&65011712,n.return=t,n=n.sibling;else for(n=t.child;n!==null;)l|=n.lanes|n.childLanes,a|=n.subtreeFlags,a|=n.flags,n.return=t,n=n.sibling;return t.subtreeFlags|=a,t.childLanes=l,e}function gv(t,e,l){var a=e.pendingProps;switch(af(e),e.tag){case 16:case 15:case 0:case 11:case 7:case 8:case 12:case 9:case 14:return P(e),null;case 1:return P(e),null;case 3:return l=e.stateNode,a=null,t!==null&&(a=t.memoizedState.cache),e.memoizedState.cache!==a&&(e.flags|=2048),Ke(vt),Da(),l.pendingContext&&(l.context=l.pendingContext,l.pendingContext=null),(t===null||t.child===null)&&(ua(e)?ke(e):t===null||t.memoizedState.isDehydrated&&!(e.flags&256)||(e.flags|=1024,Ao())),P(e),null;case 26:var n=e.type,u=e.memoizedState;return t===null?(ke(e),u!==null?(P(e),hs(e,u)):(P(e),Uo(e,n,null,a,l))):u?u!==t.memoizedState?(ke(e),P(e),hs(e,u)):(P(e),e.flags&=-16777217):(t=t.memoizedProps,t!==a&&ke(e),P(e),Uo(e,n,t,a,l)),null;case 27:if(Zu(e),l=pl.current,n=e.type,t!==null&&e.stateNode!=null)t.memoizedProps!==a&&ke(e);else{if(!a){if(e.stateNode===null)throw Error(y(166));return P(e),null}t=qe.current,ua(e)?Qr(e,t):(t=Sp(n,a,l),e.stateNode=t,ke(e))}return P(e),null;case 5:if(Zu(e),n=e.type,t!==null&&e.stateNode!=null)t.memoizedProps!==a&&ke(e);else{if(!a){if(e.stateNode===null)throw Error(y(166));return P(e),null}if(u=qe.current,ua(e))Qr(e,u);else{var i=di(pl.current);switch(u){case 1:u=i.createElementNS("http://www.w3.org/2000/svg",n);break;case 2:u=i.createElementNS("http://www.w3.org/1998/Math/MathML",n);break;default:switch(n){case"svg":u=i.createElementNS("http://www.w3.org/2000/svg",n);break;case"math":u=i.createElementNS("http://www.w3.org/1998/Math/MathML",n);break;case"script":u=i.createElement("div"),u.innerHTML="<script><\/script>",u=u.removeChild(u.firstChild);break;case"select":u=typeof a.is=="string"?i.createElement("select",{is:a.is}):i.createElement("select"),a.multiple?u.multiple=!0:a.size&&(u.size=a.size);break;default:u=typeof a.is=="string"?i.createElement(n,{is:a.is}):i.createElement(n)}}u[Dt]=e,u[Zt]=a;t:for(i=e.child;i!==null;){if(i.tag===5||i.tag===6)u.appendChild(i.stateNode);else if(i.tag!==4&&i.tag!==27&&i.child!==null){i.child.return=i,i=i.child;continue}if(i===e)break t;for(;i.sibling===null;){if(i.return===null||i.return===e)break t;i=i.return}i.sibling.return=i.return,i=i.sibling}e.stateNode=u;t:switch(Ot(u,n,a),n){case"button":case"input":case"select":case"textarea":a=!!a.autoFocus;break t;case"img":a=!0;break t;default:a=!1}a&&ke(e)}}return P(e),Uo(e,e.type,t===null?null:t.memoizedProps,e.pendingProps,l),null;case 6:if(t&&e.stateNode!=null)t.memoizedProps!==a&&ke(e);else{if(typeof a!="string"&&e.stateNode===null)throw Error(y(166));if(t=pl.current,ua(e)){if(t=e.stateNode,l=e.memoizedProps,a=null,n=qt,n!==null)switch(n.tag){case 27:case 5:a=n.memoizedProps}t[Dt]=e,t=!!(t.nodeValue===l||a!==null&&a.suppressHydrationWarning===!0||gp(t.nodeValue,l)),t||El(e,!0)}else t=di(t).createTextNode(a),t[Dt]=e,e.stateNode=t}return P(e),null;case 31:if(l=e.memoizedState,t===null||t.memoizedState!==null){if(a=ua(e),l!==null){if(t===null){if(!a)throw Error(y(318));if(t=e.memoizedState,t=t!==null?t.dehydrated:null,!t)throw Error(y(557));t[Dt]=e}else Xl(),!(e.flags&128)&&(e.memoizedState=null),e.flags|=4;P(e),t=!1}else l=Ao(),t!==null&&t.memoizedState!==null&&(t.memoizedState.hydrationErrors=l),t=!0;if(!t)return e.flags&256?($t(e),e):($t(e),null);if(e.flags&128)throw Error(y(558))}return P(e),null;case 13:if(a=e.memoizedState,t===null||t.memoizedState!==null&&t.memoizedState.dehydrated!==null){if(n=ua(e),a!==null&&a.dehydrated!==null){if(t===null){if(!n)throw Error(y(318));if(n=e.memoizedState,n=n!==null?n.dehydrated:null,!n)throw Error(y(317));n[Dt]=e}else Xl(),!(e.flags&128)&&(e.memoizedState=null),e.flags|=4;P(e),n=!1}else n=Ao(),t!==null&&t.memoizedState!==null&&(t.memoizedState.hydrationErrors=n),n=!0;if(!n)return e.flags&256?($t(e),e):($t(e),null)}return $t(e),e.flags&128?(e.lanes=l,e):(l=a!==null,t=t!==null&&t.memoizedState!==null,l&&(a=e.child,n=null,a.alternate!==null&&a.alternate.memoizedState!==null&&a.alternate.memoizedState.cachePool!==null&&(n=a.alternate.memoizedState.cachePool.pool),u=null,a.memoizedState!==null&&a.memoizedState.cachePool!==null&&(u=a.memoizedState.cachePool.pool),u!==n&&(a.flags|=2048)),l!==t&&l&&(e.child.flags|=8192),Su(e,e.updateQueue),P(e),null);case 4:return Da(),t===null&&Of(e.stateNode.containerInfo),P(e),null;case 10:return Ke(e.type),P(e),null;case 19:if(At(st),a=e.memoizedState,a===null)return P(e),null;if(n=(e.flags&128)!==0,u=a.rendering,u===null)if(n)an(a,!1);else{if(ft!==0||t!==null&&t.flags&128)for(t=e.child;t!==null;){if(u=ti(t),u!==null){for(e.flags|=128,an(a,!1),t=u.updateQueue,e.updateQueue=t,Su(e,t),e.subtreeFlags=0,t=l,l=e.child;l!==null;)Nd(l,t),l=l.sibling;return F(st,st.current&1|2),L&&Xe(e,a.treeForkCount),e.child}t=t.sibling}a.tail!==null&&Pt()>ii&&(e.flags|=128,n=!0,an(a,!1),e.lanes=4194304)}else{if(!n)if(t=ti(u),t!==null){if(e.flags|=128,n=!0,t=t.updateQueue,e.updateQueue=t,Su(e,t),an(a,!0),a.tail===null&&a.tailMode==="hidden"&&!u.alternate&&!L)return P(e),null}else 2*Pt()-a.renderingStartTime>ii&&l!==536870912&&(e.flags|=128,n=!0,an(a,!1),e.lanes=4194304);a.isBackwards?(u.sibling=e.child,e.child=u):(t=a.last,t!==null?t.sibling=u:e.child=u,a.last=u)}return a.tail!==null?(t=a.tail,a.rendering=t,a.tail=t.sibling,a.renderingStartTime=Pt(),t.sibling=null,l=st.current,F(st,n?l&1|2:l&1),L&&Xe(e,a.treeForkCount),t):(P(e),null);case 22:case 23:return $t(e),ff(),a=e.memoizedState!==null,t!==null?t.memoizedState!==null!==a&&(e.flags|=8192):a&&(e.flags|=8192),a?l&536870912&&!(e.flags&128)&&(P(e),e.subtreeFlags&6&&(e.flags|=8192)):P(e),l=e.updateQueue,l!==null&&Su(e,l.retryQueue),l=null,t!==null&&t.memoizedState!==null&&t.memoizedState.cachePool!==null&&(l=t.memoizedState.cachePool.pool),a=null,e.memoizedState!==null&&e.memoizedState.cachePool!==null&&(a=e.memoizedState.cachePool.pool),a!==l&&(e.flags|=2048),t!==null&&At(Bl),null;case 24:return l=null,t!==null&&(l=t.memoizedState.cache),e.memoizedState.cache!==l&&(e.flags|=2048),Ke(vt),P(e),null;case 25:return null;case 30:return null}throw Error(y(156,e.tag))}function yv(t,e){switch(af(e),e.tag){case 1:return t=e.flags,t&65536?(e.flags=t&-65537|128,e):null;case 3:return Ke(vt),Da(),t=e.flags,t&65536&&!(t&128)?(e.flags=t&-65537|128,e):null;case 26:case 27:case 5:return Zu(e),null;case 31:if(e.memoizedState!==null){if($t(e),e.alternate===null)throw Error(y(340));Xl()}return t=e.flags,t&65536?(e.flags=t&-65537|128,e):null;case 13:if($t(e),t=e.memoizedState,t!==null&&t.dehydrated!==null){if(e.alternate===null)throw Error(y(340));Xl()}return t=e.flags,t&65536?(e.flags=t&-65537|128,e):null;case 19:return At(st),null;case 4:return Da(),null;case 10:return Ke(e.type),null;case 22:case 23:return $t(e),ff(),t!==null&&At(Bl),t=e.flags,t&65536?(e.flags=t&-65537|128,e):null;case 24:return Ke(vt),null;case 25:return null;default:return null}}function Nm(t,e){switch(af(e),e.tag){case 3:Ke(vt),Da();break;case 26:case 27:case 5:Zu(e);break;case 4:Da();break;case 31:e.memoizedState!==null&&$t(e);break;case 13:$t(e);break;case 19:At(st);break;case 10:Ke(e.type);break;case 22:case 23:$t(e),ff(),t!==null&&At(Bl);break;case 24:Ke(vt)}}function Jn(t,e){try{var l=e.updateQueue,a=l!==null?l.lastEffect:null;if(a!==null){var n=a.next;l=n;do{if((l.tag&t)===t){a=void 0;var u=l.create,i=l.inst;a=u(),i.destroy=a}l=l.next}while(l!==n)}}catch(o){K(e,e.return,o)}}function Tl(t,e,l){try{var a=e.updateQueue,n=a!==null?a.lastEffect:null;if(n!==null){var u=n.next;a=u;do{if((a.tag&t)===t){var i=a.inst,o=i.destroy;if(o!==void 0){i.destroy=void 0,n=e;var c=l,s=o;try{s()}catch(h){K(n,c,h)}}}a=a.next}while(a!==u)}}catch(h){K(e,e.return,h)}}function Cm(t){var e=t.updateQueue;if(e!==null){var l=t.stateNode;try{Ld(e,l)}catch(a){K(t,t.return,a)}}}function Rm(t,e,l){l.props=Zl(t.type,t.memoizedProps),l.state=t.memoizedState;try{l.componentWillUnmount()}catch(a){K(t,e,a)}}function Sn(t,e){try{var l=t.ref;if(l!==null){switch(t.tag){case 26:case 27:case 5:var a=t.stateNode;break;case 30:a=t.stateNode;break;default:a=t.stateNode}typeof l=="function"?t.refCleanup=l(a):l.current=a}}catch(n){K(t,e,n)}}function De(t,e){var l=t.ref,a=t.refCleanup;if(l!==null)if(typeof a=="function")try{a()}catch(n){K(t,e,n)}finally{t.refCleanup=null,t=t.alternate,t!=null&&(t.refCleanup=null)}else if(typeof l=="function")try{l(null)}catch(n){K(t,e,n)}else l.current=null}function Um(t){var e=t.type,l=t.memoizedProps,a=t.stateNode;try{t:switch(e){case"button":case"input":case"select":case"textarea":l.autoFocus&&a.focus();break t;case"img":l.src?a.src=l.src:l.srcSet&&(a.srcset=l.srcSet)}}catch(n){K(t,t.return,n)}}function Ho(t,e,l){try{var a=t.stateNode;Bv(a,t.type,l,e),a[Zt]=e}catch(n){K(t,t.return,n)}}function Hm(t){return t.tag===5||t.tag===3||t.tag===26||t.tag===27&&Dl(t.type)||t.tag===4}function ko(t){t:for(;;){for(;t.sibling===null;){if(t.return===null||Hm(t.return))return null;t=t.return}for(t.sibling.return=t.return,t=t.sibling;t.tag!==5&&t.tag!==6&&t.tag!==18;){if(t.tag===27&&Dl(t.type)||t.flags&2||t.child===null||t.tag===4)continue t;t.child.return=t,t=t.child}if(!(t.flags&2))return t.stateNode}}function Tc(t,e,l){var a=t.tag;if(a===5||a===6)t=t.stateNode,e?(l.nodeType===9?l.body:l.nodeName==="HTML"?l.ownerDocument.body:l).insertBefore(t,e):(e=l.nodeType===9?l.body:l.nodeName==="HTML"?l.ownerDocument.body:l,e.appendChild(t),l=l._reactRootContainer,l!=null||e.onclick!==null||(e.onclick=Qe));else if(a!==4&&(a===27&&Dl(t.type)&&(l=t.stateNode,e=null),t=t.child,t!==null))for(Tc(t,e,l),t=t.sibling;t!==null;)Tc(t,e,l),t=t.sibling}function ui(t,e,l){var a=t.tag;if(a===5||a===6)t=t.stateNode,e?l.insertBefore(t,e):l.appendChild(t);else if(a!==4&&(a===27&&Dl(t.type)&&(l=t.stateNode),t=t.child,t!==null))for(ui(t,e,l),t=t.sibling;t!==null;)ui(t,e,l),t=t.sibling}function km(t){var e=t.stateNode,l=t.memoizedProps;try{for(var a=t.type,n=e.attributes;n.length;)e.removeAttributeNode(n[0]);Ot(e,a,l),e[Dt]=t,e[Zt]=l}catch(u){K(t,t.return,u)}}var Le=!1,ht=!1,Bo=!1,vs=typeof WeakSet=="function"?WeakSet:Set,Et=null;function bv(t,e){if(t=t.containerInfo,Nc=vi,t=Ed(t),Ic(t)){if("selectionStart"in t)var l={start:t.selectionStart,end:t.selectionEnd};else t:{l=(l=t.ownerDocument)&&l.defaultView||window;var a=l.getSelection&&l.getSelection();if(a&&a.rangeCount!==0){l=a.anchorNode;var n=a.anchorOffset,u=a.focusNode;a=a.focusOffset;try{l.nodeType,u.nodeType}catch{l=null;break t}var i=0,o=-1,c=-1,s=0,h=0,v=t,d=null;e:for(;;){for(var p;v!==l||n!==0&&v.nodeType!==3||(o=i+n),v!==u||a!==0&&v.nodeType!==3||(c=i+a),v.nodeType===3&&(i+=v.nodeValue.length),(p=v.firstChild)!==null;)d=v,v=p;for(;;){if(v===t)break e;if(d===l&&++s===n&&(o=i),d===u&&++h===a&&(c=i),(p=v.nextSibling)!==null)break;v=d,d=v.parentNode}v=p}l=o===-1||c===-1?null:{start:o,end:c}}else l=null}l=l||{start:0,end:0}}else l=null;for(Cc={focusedElem:t,selectionRange:l},vi=!1,Et=e;Et!==null;)if(e=Et,t=e.child,(e.subtreeFlags&1028)!==0&&t!==null)t.return=e,Et=t;else for(;Et!==null;){switch(e=Et,u=e.alternate,t=e.flags,e.tag){case 0:if(t&4&&(t=e.updateQueue,t=t!==null?t.events:null,t!==null))for(l=0;l<t.length;l++)n=t[l],n.ref.impl=n.nextImpl;break;case 11:case 15:break;case 1:if(t&1024&&u!==null){t=void 0,l=e,n=u.memoizedProps,u=u.memoizedState,a=l.stateNode;try{var b=Zl(l.type,n);t=a.getSnapshotBeforeUpdate(b,u),a.__reactInternalSnapshotBeforeUpdate=t}catch(S){K(l,l.return,S)}}break;case 3:if(t&1024){if(t=e.stateNode.containerInfo,l=t.nodeType,l===9)Uc(t);else if(l===1)switch(t.nodeName){case"HEAD":case"HTML":case"BODY":Uc(t);break;default:t.textContent=""}}break;case 5:case 26:case 27:case 6:case 4:case 17:break;default:if(t&1024)throw Error(y(163))}if(t=e.sibling,t!==null){t.return=e.return,Et=t;break}Et=e.return}}function Bm(t,e,l){var a=l.flags;switch(l.tag){case 0:case 11:case 15:Ye(t,l),a&4&&Jn(5,l);break;case 1:if(Ye(t,l),a&4)if(t=l.stateNode,e===null)try{t.componentDidMount()}catch(i){K(l,l.return,i)}else{var n=Zl(l.type,e.memoizedProps);e=e.memoizedState;try{t.componentDidUpdate(n,e,t.__reactInternalSnapshotBeforeUpdate)}catch(i){K(l,l.return,i)}}a&64&&Cm(l),a&512&&Sn(l,l.return);break;case 3:if(Ye(t,l),a&64&&(t=l.updateQueue,t!==null)){if(e=null,l.child!==null)switch(l.child.tag){case 27:case 5:e=l.child.stateNode;break;case 1:e=l.child.stateNode}try{Ld(t,e)}catch(i){K(l,l.return,i)}}break;case 27:e===null&&a&4&&km(l);case 26:case 5:Ye(t,l),e===null&&a&4&&Um(l),a&512&&Sn(l,l.return);break;case 12:Ye(t,l);break;case 31:Ye(t,l),a&4&&Xm(t,l);break;case 13:Ye(t,l),a&4&&Lm(t,l),a&64&&(t=l.memoizedState,t!==null&&(t=t.dehydrated,t!==null&&(l=Dv.bind(null,l),Vv(t,l))));break;case 22:if(a=l.memoizedState!==null||Le,!a){e=e!==null&&e.memoizedState!==null||ht,n=Le;var u=ht;Le=a,(ht=e)&&!u?Ge(t,l,(l.subtreeFlags&8772)!==0):Ye(t,l),Le=n,ht=u}break;case 30:break;default:Ye(t,l)}}function Ym(t){var e=t.alternate;e!==null&&(t.alternate=null,Ym(e)),t.child=null,t.deletions=null,t.sibling=null,t.tag===5&&(e=t.stateNode,e!==null&&Vc(e)),t.stateNode=null,t.return=null,t.dependencies=null,t.memoizedProps=null,t.memoizedState=null,t.pendingProps=null,t.stateNode=null,t.updateQueue=null}var at=null,Lt=!1;function Be(t,e,l){for(l=l.child;l!==null;)Gm(t,e,l),l=l.sibling}function Gm(t,e,l){if(te&&typeof te.onCommitFiberUnmount=="function")try{te.onCommitFiberUnmount(Xn,l)}catch{}switch(l.tag){case 26:ht||De(l,e),Be(t,e,l),l.memoizedState?l.memoizedState.count--:l.stateNode&&(l=l.stateNode,l.parentNode.removeChild(l));break;case 27:ht||De(l,e);var a=at,n=Lt;Dl(l.type)&&(at=l.stateNode,Lt=!1),Be(t,e,l),Tn(l.stateNode),at=a,Lt=n;break;case 5:ht||De(l,e);case 6:if(a=at,n=Lt,at=null,Be(t,e,l),at=a,Lt=n,at!==null)if(Lt)try{(at.nodeType===9?at.body:at.nodeName==="HTML"?at.ownerDocument.body:at).removeChild(l.stateNode)}catch(u){K(l,e,u)}else try{at.removeChild(l.stateNode)}catch(u){K(l,e,u)}break;case 18:at!==null&&(Lt?(t=at,Os(t.nodeType===9?t.body:t.nodeName==="HTML"?t.ownerDocument.body:t,l.stateNode),Ba(t)):Os(at,l.stateNode));break;case 4:a=at,n=Lt,at=l.stateNode.containerInfo,Lt=!0,Be(t,e,l),at=a,Lt=n;break;case 0:case 11:case 14:case 15:Tl(2,l,e),ht||Tl(4,l,e),Be(t,e,l);break;case 1:ht||(De(l,e),a=l.stateNode,typeof a.componentWillUnmount=="function"&&Rm(l,e,a)),Be(t,e,l);break;case 21:Be(t,e,l);break;case 22:ht=(a=ht)||l.memoizedState!==null,Be(t,e,l),ht=a;break;default:Be(t,e,l)}}function Xm(t,e){if(e.memoizedState===null&&(t=e.alternate,t!==null&&(t=t.memoizedState,t!==null))){t=t.dehydrated;try{Ba(t)}catch(l){K(e,e.return,l)}}}function Lm(t,e){if(e.memoizedState===null&&(t=e.alternate,t!==null&&(t=t.memoizedState,t!==null&&(t=t.dehydrated,t!==null))))try{Ba(t)}catch(l){K(e,e.return,l)}}function _v(t){switch(t.tag){case 31:case 13:case 19:var e=t.stateNode;return e===null&&(e=t.stateNode=new vs),e;case 22:return t=t.stateNode,e=t._retryCache,e===null&&(e=t._retryCache=new vs),e;default:throw Error(y(435,t.tag))}}function xu(t,e){var l=_v(t);e.forEach(function(a){if(!l.has(a)){l.add(a);var n=qv.bind(null,t,a);a.then(n,n)}})}function Gt(t,e){var l=e.deletions;if(l!==null)for(var a=0;a<l.length;a++){var n=l[a],u=t,i=e,o=i;t:for(;o!==null;){switch(o.tag){case 27:if(Dl(o.type)){at=o.stateNode,Lt=!1;break t}break;case 5:at=o.stateNode,Lt=!1;break t;case 3:case 4:at=o.stateNode.containerInfo,Lt=!0;break t}o=o.return}if(at===null)throw Error(y(160));Gm(u,i,n),at=null,Lt=!1,u=n.alternate,u!==null&&(u.return=null),n.return=null}if(e.subtreeFlags&13886)for(e=e.child;e!==null;)jm(e,t),e=e.sibling}var be=null;function jm(t,e){var l=t.alternate,a=t.flags;switch(t.tag){case 0:case 11:case 14:case 15:Gt(e,t),Xt(t),a&4&&(Tl(3,t,t.return),Jn(3,t),Tl(5,t,t.return));break;case 1:Gt(e,t),Xt(t),a&512&&(ht||l===null||De(l,l.return)),a&64&&Le&&(t=t.updateQueue,t!==null&&(a=t.callbacks,a!==null&&(l=t.shared.hiddenCallbacks,t.shared.hiddenCallbacks=l===null?a:l.concat(a))));break;case 26:var n=be;if(Gt(e,t),Xt(t),a&512&&(ht||l===null||De(l,l.return)),a&4){var u=l!==null?l.memoizedState:null;if(a=t.memoizedState,l===null)if(a===null)if(t.stateNode===null){t:{a=t.type,l=t.memoizedProps,n=n.ownerDocument||n;e:switch(a){case"title":u=n.getElementsByTagName("title")[0],(!u||u[Qn]||u[Dt]||u.namespaceURI==="http://www.w3.org/2000/svg"||u.hasAttribute("itemprop"))&&(u=n.createElement(a),n.head.insertBefore(u,n.querySelector("head > title"))),Ot(u,a,l),u[Dt]=t,Tt(u),a=u;break t;case"link":var i=Bs("link","href",n).get(a+(l.href||""));if(i){for(var o=0;o<i.length;o++)if(u=i[o],u.getAttribute("href")===(l.href==null||l.href===""?null:l.href)&&u.getAttribute("rel")===(l.rel==null?null:l.rel)&&u.getAttribute("title")===(l.title==null?null:l.title)&&u.getAttribute("crossorigin")===(l.crossOrigin==null?null:l.crossOrigin)){i.splice(o,1);break e}}u=n.createElement(a),Ot(u,a,l),n.head.appendChild(u);break;case"meta":if(i=Bs("meta","content",n).get(a+(l.content||""))){for(o=0;o<i.length;o++)if(u=i[o],u.getAttribute("content")===(l.content==null?null:""+l.content)&&u.getAttribute("name")===(l.name==null?null:l.name)&&u.getAttribute("property")===(l.property==null?null:l.property)&&u.getAttribute("http-equiv")===(l.httpEquiv==null?null:l.httpEquiv)&&u.getAttribute("charset")===(l.charSet==null?null:l.charSet)){i.splice(o,1);break e}}u=n.createElement(a),Ot(u,a,l),n.head.appendChild(u);break;default:throw Error(y(468,a))}u[Dt]=t,Tt(u),a=u}t.stateNode=a}else Ys(n,t.type,t.stateNode);else t.stateNode=ks(n,a,t.memoizedProps);else u!==a?(u===null?l.stateNode!==null&&(l=l.stateNode,l.parentNode.removeChild(l)):u.count--,a===null?Ys(n,t.type,t.stateNode):ks(n,a,t.memoizedProps)):a===null&&t.stateNode!==null&&Ho(t,t.memoizedProps,l.memoizedProps)}break;case 27:Gt(e,t),Xt(t),a&512&&(ht||l===null||De(l,l.return)),l!==null&&a&4&&Ho(t,t.memoizedProps,l.memoizedProps);break;case 5:if(Gt(e,t),Xt(t),a&512&&(ht||l===null||De(l,l.return)),t.flags&32){n=t.stateNode;try{wa(n,"")}catch(b){K(t,t.return,b)}}a&4&&t.stateNode!=null&&(n=t.memoizedProps,Ho(t,n,l!==null?l.memoizedProps:n)),a&1024&&(Bo=!0);break;case 6:if(Gt(e,t),Xt(t),a&4){if(t.stateNode===null)throw Error(y(162));a=t.memoizedProps,l=t.stateNode;try{l.nodeValue=a}catch(b){K(t,t.return,b)}}break;case 3:if(Gu=null,n=be,be=mi(e.containerInfo),Gt(e,t),be=n,Xt(t),a&4&&l!==null&&l.memoizedState.isDehydrated)try{Ba(e.containerInfo)}catch(b){K(t,t.return,b)}Bo&&(Bo=!1,Qm(t));break;case 4:a=be,be=mi(t.stateNode.containerInfo),Gt(e,t),Xt(t),be=a;break;case 12:Gt(e,t),Xt(t);break;case 31:Gt(e,t),Xt(t),a&4&&(a=t.updateQueue,a!==null&&(t.updateQueue=null,xu(t,a)));break;case 13:Gt(e,t),Xt(t),t.child.flags&8192&&t.memoizedState!==null!=(l!==null&&l.memoizedState!==null)&&(qi=Pt()),a&4&&(a=t.updateQueue,a!==null&&(t.updateQueue=null,xu(t,a)));break;case 22:n=t.memoizedState!==null;var c=l!==null&&l.memoizedState!==null,s=Le,h=ht;if(Le=s||n,ht=h||c,Gt(e,t),ht=h,Le=s,Xt(t),a&8192)t:for(e=t.stateNode,e._visibility=n?e._visibility&-2:e._visibility|1,n&&(l===null||c||Le||ht||Ul(t)),l=null,e=t;;){if(e.tag===5||e.tag===26){if(l===null){c=l=e;try{if(u=c.stateNode,n)i=u.style,typeof i.setProperty=="function"?i.setProperty("display","none","important"):i.display="none";else{o=c.stateNode;var v=c.memoizedProps.style,d=v!=null&&v.hasOwnProperty("display")?v.display:null;o.style.display=d==null||typeof d=="boolean"?"":(""+d).trim()}}catch(b){K(c,c.return,b)}}}else if(e.tag===6){if(l===null){c=e;try{c.stateNode.nodeValue=n?"":c.memoizedProps}catch(b){K(c,c.return,b)}}}else if(e.tag===18){if(l===null){c=e;try{var p=c.stateNode;n?Ns(p,!0):Ns(c.stateNode,!1)}catch(b){K(c,c.return,b)}}}else if((e.tag!==22&&e.tag!==23||e.memoizedState===null||e===t)&&e.child!==null){e.child.return=e,e=e.child;continue}if(e===t)break t;for(;e.sibling===null;){if(e.return===null||e.return===t)break t;l===e&&(l=null),e=e.return}l===e&&(l=null),e.sibling.return=e.return,e=e.sibling}a&4&&(a=t.updateQueue,a!==null&&(l=a.retryQueue,l!==null&&(a.retryQueue=null,xu(t,l))));break;case 19:Gt(e,t),Xt(t),a&4&&(a=t.updateQueue,a!==null&&(t.updateQueue=null,xu(t,a)));break;case 30:break;case 21:break;default:Gt(e,t),Xt(t)}}function Xt(t){var e=t.flags;if(e&2){try{for(var l,a=t.return;a!==null;){if(Hm(a)){l=a;break}a=a.return}if(l==null)throw Error(y(160));switch(l.tag){case 27:var n=l.stateNode,u=ko(t);ui(t,u,n);break;case 5:var i=l.stateNode;l.flags&32&&(wa(i,""),l.flags&=-33);var o=ko(t);ui(t,o,i);break;case 3:case 4:var c=l.stateNode.containerInfo,s=ko(t);Tc(t,s,c);break;default:throw Error(y(161))}}catch(h){K(t,t.return,h)}t.flags&=-3}e&4096&&(t.flags&=-4097)}function Qm(t){if(t.subtreeFlags&1024)for(t=t.child;t!==null;){var e=t;Qm(e),e.tag===5&&e.flags&1024&&e.stateNode.reset(),t=t.sibling}}function Ye(t,e){if(e.subtreeFlags&8772)for(e=e.child;e!==null;)Bm(t,e.alternate,e),e=e.sibling}function Ul(t){for(t=t.child;t!==null;){var e=t;switch(e.tag){case 0:case 11:case 14:case 15:Tl(4,e,e.return),Ul(e);break;case 1:De(e,e.return);var l=e.stateNode;typeof l.componentWillUnmount=="function"&&Rm(e,e.return,l),Ul(e);break;case 27:Tn(e.stateNode);case 26:case 5:De(e,e.return),Ul(e);break;case 22:e.memoizedState===null&&Ul(e);break;case 30:Ul(e);break;default:Ul(e)}t=t.sibling}}function Ge(t,e,l){for(l=l&&(e.subtreeFlags&8772)!==0,e=e.child;e!==null;){var a=e.alternate,n=t,u=e,i=u.flags;switch(u.tag){case 0:case 11:case 15:Ge(n,u,l),Jn(4,u);break;case 1:if(Ge(n,u,l),a=u,n=a.stateNode,typeof n.componentDidMount=="function")try{n.componentDidMount()}catch(s){K(a,a.return,s)}if(a=u,n=a.updateQueue,n!==null){var o=a.stateNode;try{var c=n.shared.hiddenCallbacks;if(c!==null)for(n.shared.hiddenCallbacks=null,n=0;n<c.length;n++)Xd(c[n],o)}catch(s){K(a,a.return,s)}}l&&i&64&&Cm(u),Sn(u,u.return);break;case 27:km(u);case 26:case 5:Ge(n,u,l),l&&a===null&&i&4&&Um(u),Sn(u,u.return);break;case 12:Ge(n,u,l);break;case 31:Ge(n,u,l),l&&i&4&&Xm(n,u);break;case 13:Ge(n,u,l),l&&i&4&&Lm(n,u);break;case 22:u.memoizedState===null&&Ge(n,u,l),Sn(u,u.return);break;case 30:break;default:Ge(n,u,l)}e=e.sibling}}function Tf(t,e){var l=null;t!==null&&t.memoizedState!==null&&t.memoizedState.cachePool!==null&&(l=t.memoizedState.cachePool.pool),t=null,e.memoizedState!==null&&e.memoizedState.cachePool!==null&&(t=e.memoizedState.cachePool.pool),t!==l&&(t!=null&&t.refCount++,l!=null&&Vn(l))}function Af(t,e){t=null,e.alternate!==null&&(t=e.alternate.memoizedState.cache),e=e.memoizedState.cache,e!==t&&(e.refCount++,t!=null&&Vn(t))}function ye(t,e,l,a){if(e.subtreeFlags&10256)for(e=e.child;e!==null;)Zm(t,e,l,a),e=e.sibling}function Zm(t,e,l,a){var n=e.flags;switch(e.tag){case 0:case 11:case 15:ye(t,e,l,a),n&2048&&Jn(9,e);break;case 1:ye(t,e,l,a);break;case 3:ye(t,e,l,a),n&2048&&(t=null,e.alternate!==null&&(t=e.alternate.memoizedState.cache),e=e.memoizedState.cache,e!==t&&(e.refCount++,t!=null&&Vn(t)));break;case 12:if(n&2048){ye(t,e,l,a),t=e.stateNode;try{var u=e.memoizedProps,i=u.id,o=u.onPostCommit;typeof o=="function"&&o(i,e.alternate===null?"mount":"update",t.passiveEffectDuration,-0)}catch(c){K(e,e.return,c)}}else ye(t,e,l,a);break;case 31:ye(t,e,l,a);break;case 13:ye(t,e,l,a);break;case 23:break;case 22:u=e.stateNode,i=e.alternate,e.memoizedState!==null?u._visibility&2?ye(t,e,l,a):xn(t,e):u._visibility&2?ye(t,e,l,a):(u._visibility|=2,oa(t,e,l,a,(e.subtreeFlags&10256)!==0||!1)),n&2048&&Tf(i,e);break;case 24:ye(t,e,l,a),n&2048&&Af(e.alternate,e);break;default:ye(t,e,l,a)}}function oa(t,e,l,a,n){for(n=n&&((e.subtreeFlags&10256)!==0||!1),e=e.child;e!==null;){var u=t,i=e,o=l,c=a,s=i.flags;switch(i.tag){case 0:case 11:case 15:oa(u,i,o,c,n),Jn(8,i);break;case 23:break;case 22:var h=i.stateNode;i.memoizedState!==null?h._visibility&2?oa(u,i,o,c,n):xn(u,i):(h._visibility|=2,oa(u,i,o,c,n)),n&&s&2048&&Tf(i.alternate,i);break;case 24:oa(u,i,o,c,n),n&&s&2048&&Af(i.alternate,i);break;default:oa(u,i,o,c,n)}e=e.sibling}}function xn(t,e){if(e.subtreeFlags&10256)for(e=e.child;e!==null;){var l=t,a=e,n=a.flags;switch(a.tag){case 22:xn(l,a),n&2048&&Tf(a.alternate,a);break;case 24:xn(l,a),n&2048&&Af(a.alternate,a);break;default:xn(l,a)}e=e.sibling}}var dn=8192;function ia(t,e,l){if(t.subtreeFlags&dn)for(t=t.child;t!==null;)Vm(t,e,l),t=t.sibling}function Vm(t,e,l){switch(t.tag){case 26:ia(t,e,l),t.flags&dn&&t.memoizedState!==null&&ng(l,be,t.memoizedState,t.memoizedProps);break;case 5:ia(t,e,l);break;case 3:case 4:var a=be;be=mi(t.stateNode.containerInfo),ia(t,e,l),be=a;break;case 22:t.memoizedState===null&&(a=t.alternate,a!==null&&a.memoizedState!==null?(a=dn,dn=16777216,ia(t,e,l),dn=a):ia(t,e,l));break;default:ia(t,e,l)}}function Km(t){var e=t.alternate;if(e!==null&&(t=e.child,t!==null)){e.child=null;do e=t.sibling,t.sibling=null,t=e;while(t!==null)}}function nn(t){var e=t.deletions;if(t.flags&16){if(e!==null)for(var l=0;l<e.length;l++){var a=e[l];Et=a,Wm(a,t)}Km(t)}if(t.subtreeFlags&10256)for(t=t.child;t!==null;)Jm(t),t=t.sibling}function Jm(t){switch(t.tag){case 0:case 11:case 15:nn(t),t.flags&2048&&Tl(9,t,t.return);break;case 3:nn(t);break;case 12:nn(t);break;case 22:var e=t.stateNode;t.memoizedState!==null&&e._visibility&2&&(t.return===null||t.return.tag!==13)?(e._visibility&=-3,Bu(t)):nn(t);break;default:nn(t)}}function Bu(t){var e=t.deletions;if(t.flags&16){if(e!==null)for(var l=0;l<e.length;l++){var a=e[l];Et=a,Wm(a,t)}Km(t)}for(t=t.child;t!==null;){switch(e=t,e.tag){case 0:case 11:case 15:Tl(8,e,e.return),Bu(e);break;case 22:l=e.stateNode,l._visibility&2&&(l._visibility&=-3,Bu(e));break;default:Bu(e)}t=t.sibling}}function Wm(t,e){for(;Et!==null;){var l=Et;switch(l.tag){case 0:case 11:case 15:Tl(8,l,e);break;case 23:case 22:if(l.memoizedState!==null&&l.memoizedState.cachePool!==null){var a=l.memoizedState.cachePool.pool;a!=null&&a.refCount++}break;case 24:Vn(l.memoizedState.cache)}if(a=l.child,a!==null)a.return=l,Et=a;else t:for(l=t;Et!==null;){a=Et;var n=a.sibling,u=a.return;if(Ym(a),a===l){Et=null;break t}if(n!==null){n.return=u,Et=n;break t}Et=u}}}var Sv={getCacheForType:function(t){var e=wt(vt),l=e.data.get(t);return l===void 0&&(l=t(),e.data.set(t,l)),l},cacheSignal:function(){return wt(vt).controller.signal}},xv=typeof WeakMap=="function"?WeakMap:Map,Q=0,$=null,G=null,X=0,V=0,Wt=null,sl=!1,ja=!1,Mf=!1,Pe=0,ft=0,Al=0,Gl=0,Df=0,It=0,Ra=0,zn=null,jt=null,Ac=!1,qi=0,$m=0,ii=1/0,oi=null,yl=null,_t=0,bl=null,Ua=null,Je=0,Mc=0,Dc=null,Fm=null,En=0,qc=null;function le(){return Q&2&&X!==0?X&-X:C.T!==null?wf():id()}function Im(){if(It===0)if(!(X&536870912)||L){var t=du;du<<=1,!(du&3932160)&&(du=262144),It=t}else It=536870912;return t=ne.current,t!==null&&(t.flags|=32),It}function Qt(t,e,l){(t===$&&(V===2||V===9)||t.cancelPendingCommit!==null)&&(Ha(t,0),dl(t,X,It,!1)),jn(t,l),(!(Q&2)||t!==$)&&(t===$&&(!(Q&2)&&(Gl|=l),ft===4&&dl(t,X,It,!1)),Oe(t))}function Pm(t,e,l){if(Q&6)throw Error(y(327));var a=!l&&(e&127)===0&&(e&t.expiredLanes)===0||Ln(t,e),n=a?Tv(t,e):Yo(t,e,!0),u=a;do{if(n===0){ja&&!a&&dl(t,e,0,!1);break}else{if(l=t.current.alternate,u&&!zv(l)){n=Yo(t,e,!1),u=!1;continue}if(n===2){if(u=e,t.errorRecoveryDisabledLanes&u)var i=0;else i=t.pendingLanes&-536870913,i=i!==0?i:i&536870912?536870912:0;if(i!==0){e=i;t:{var o=t;n=zn;var c=o.current.memoizedState.isDehydrated;if(c&&(Ha(o,i).flags|=256),i=Yo(o,i,!1),i!==2){if(Mf&&!c){o.errorRecoveryDisabledLanes|=u,Gl|=u,n=4;break t}u=jt,jt=n,u!==null&&(jt===null?jt=u:jt.push.apply(jt,u))}n=i}if(u=!1,n!==2)continue}}if(n===1){Ha(t,0),dl(t,e,0,!0);break}t:{switch(a=t,u=n,u){case 0:case 1:throw Error(y(345));case 4:if((e&4194048)!==e)break;case 6:dl(a,e,It,!sl);break t;case 2:jt=null;break;case 3:case 5:break;default:throw Error(y(329))}if((e&62914560)===e&&(n=qi+300-Pt(),10<n)){if(dl(a,e,It,!sl),yi(a,0,!0)!==0)break t;Je=e,a.timeoutHandle=bp(gs.bind(null,a,l,jt,oi,Ac,e,It,Gl,Ra,sl,u,"Throttled",-0,0),n);break t}gs(a,l,jt,oi,Ac,e,It,Gl,Ra,sl,u,null,-0,0)}}break}while(!0);Oe(t)}function gs(t,e,l,a,n,u,i,o,c,s,h,v,d,p){if(t.timeoutHandle=-1,v=e.subtreeFlags,v&8192||(v&16785408)===16785408){v={stylesheets:null,count:0,imgCount:0,imgBytes:0,suspenseyImages:[],waitingForImages:!0,waitingForViewTransition:!1,unsuspend:Qe},Vm(e,u,v);var b=(u&62914560)===u?qi-Pt():(u&4194048)===u?$m-Pt():0;if(b=ug(v,b),b!==null){Je=u,t.cancelPendingCommit=b(bs.bind(null,t,e,u,l,a,n,i,o,c,h,v,null,d,p)),dl(t,u,i,!s);return}}bs(t,e,u,l,a,n,i,o,c)}function zv(t){for(var e=t;;){var l=e.tag;if((l===0||l===11||l===15)&&e.flags&16384&&(l=e.updateQueue,l!==null&&(l=l.stores,l!==null)))for(var a=0;a<l.length;a++){var n=l[a],u=n.getSnapshot;n=n.value;try{if(!ae(u(),n))return!1}catch{return!1}}if(l=e.child,e.subtreeFlags&16384&&l!==null)l.return=e,e=l;else{if(e===t)break;for(;e.sibling===null;){if(e.return===null||e.return===t)return!0;e=e.return}e.sibling.return=e.return,e=e.sibling}}return!0}function dl(t,e,l,a){e&=~Df,e&=~Gl,t.suspendedLanes|=e,t.pingedLanes&=~e,a&&(t.warmLanes|=e),a=t.expirationTimes;for(var n=e;0<n;){var u=31-ee(n),i=1<<u;a[u]=-1,n&=~i}l!==0&&ad(t,l,e)}function wi(){return Q&6?!0:(Wn(0,!1),!1)}function qf(){if(G!==null){if(V===0)var t=G.return;else t=G,Ze=$l=null,pf(t),Ta=null,Nn=0,t=G;for(;t!==null;)Nm(t.alternate,t),t=t.return;G=null}}function Ha(t,e){var l=t.timeoutHandle;l!==-1&&(t.timeoutHandle=-1,Xv(l)),l=t.cancelPendingCommit,l!==null&&(t.cancelPendingCommit=null,l()),Je=0,qf(),$=t,G=l=Ve(t.current,null),X=e,V=0,Wt=null,sl=!1,ja=Ln(t,e),Mf=!1,Ra=It=Df=Gl=Al=ft=0,jt=zn=null,Ac=!1,e&8&&(e|=e&32);var a=t.entangledLanes;if(a!==0)for(t=t.entanglements,a&=e;0<a;){var n=31-ee(a),u=1<<n;e|=t[n],a&=~u}return Pe=e,xi(),l}function tp(t,e){H=null,C.H=Rn,e===La||e===Ei?(e=Wr(),V=3):e===of?(e=Wr(),V=4):V=e===zf?8:e!==null&&typeof e=="object"&&typeof e.then=="function"?6:1,Wt=e,G===null&&(ft=1,ai(t,de(e,t.current)))}function ep(){var t=ne.current;return t===null?!0:(X&4194048)===X?pe===null:(X&62914560)===X||X&536870912?t===pe:!1}function lp(){var t=C.H;return C.H=Rn,t===null?Rn:t}function ap(){var t=C.A;return C.A=Sv,t}function ci(){ft=4,sl||(X&4194048)!==X&&ne.current!==null||(ja=!0),!(Al&134217727)&&!(Gl&134217727)||$===null||dl($,X,It,!1)}function Yo(t,e,l){var a=Q;Q|=2;var n=lp(),u=ap();($!==t||X!==e)&&(oi=null,Ha(t,e)),e=!1;var i=ft;t:do try{if(V!==0&&G!==null){var o=G,c=Wt;switch(V){case 8:qf(),i=6;break t;case 3:case 2:case 9:case 6:ne.current===null&&(e=!0);var s=V;if(V=0,Wt=null,_a(t,o,c,s),l&&ja){i=0;break t}break;default:s=V,V=0,Wt=null,_a(t,o,c,s)}}Ev(),i=ft;break}catch(h){tp(t,h)}while(!0);return e&&t.shellSuspendCounter++,Ze=$l=null,Q=a,C.H=n,C.A=u,G===null&&($=null,X=0,xi()),i}function Ev(){for(;G!==null;)np(G)}function Tv(t,e){var l=Q;Q|=2;var a=lp(),n=ap();$!==t||X!==e?(oi=null,ii=Pt()+500,Ha(t,e)):ja=Ln(t,e);t:do try{if(V!==0&&G!==null){e=G;var u=Wt;e:switch(V){case 1:V=0,Wt=null,_a(t,e,u,1);break;case 2:case 9:if(Jr(u)){V=0,Wt=null,ys(e);break}e=function(){V!==2&&V!==9||$!==t||(V=7),Oe(t)},u.then(e,e);break t;case 3:V=7;break t;case 4:V=5;break t;case 7:Jr(u)?(V=0,Wt=null,ys(e)):(V=0,Wt=null,_a(t,e,u,7));break;case 5:var i=null;switch(G.tag){case 26:i=G.memoizedState;case 5:case 27:var o=G;if(i?Ep(i):o.stateNode.complete){V=0,Wt=null;var c=o.sibling;if(c!==null)G=c;else{var s=o.return;s!==null?(G=s,Oi(s)):G=null}break e}}V=0,Wt=null,_a(t,e,u,5);break;case 6:V=0,Wt=null,_a(t,e,u,6);break;case 8:qf(),ft=6;break t;default:throw Error(y(462))}}Av();break}catch(h){tp(t,h)}while(!0);return Ze=$l=null,C.H=a,C.A=n,Q=l,G!==null?0:($=null,X=0,xi(),ft)}function Av(){for(;G!==null&&!W0();)np(G)}function np(t){var e=Om(t.alternate,t,Pe);t.memoizedProps=t.pendingProps,e===null?Oi(t):G=e}function ys(t){var e=t,l=e.alternate;switch(e.tag){case 15:case 0:e=ss(l,e,e.pendingProps,e.type,void 0,X);break;case 11:e=ss(l,e,e.pendingProps,e.type.render,e.ref,X);break;case 5:pf(e);default:Nm(l,e),e=G=Nd(e,Pe),e=Om(l,e,Pe)}t.memoizedProps=t.pendingProps,e===null?Oi(t):G=e}function _a(t,e,l,a){Ze=$l=null,pf(e),Ta=null,Nn=0;var n=e.return;try{if(pv(t,n,e,l,X)){ft=1,ai(t,de(l,t.current)),G=null;return}}catch(u){if(n!==null)throw G=n,u;ft=1,ai(t,de(l,t.current)),G=null;return}e.flags&32768?(L||a===1?t=!0:ja||X&536870912?t=!1:(sl=t=!0,(a===2||a===9||a===3||a===6)&&(a=ne.current,a!==null&&a.tag===13&&(a.flags|=16384))),up(e,t)):Oi(e)}function Oi(t){var e=t;do{if(e.flags&32768){up(e,sl);return}t=e.return;var l=gv(e.alternate,e,Pe);if(l!==null){G=l;return}if(e=e.sibling,e!==null){G=e;return}G=e=t}while(e!==null);ft===0&&(ft=5)}function up(t,e){do{var l=yv(t.alternate,t);if(l!==null){l.flags&=32767,G=l;return}if(l=t.return,l!==null&&(l.flags|=32768,l.subtreeFlags=0,l.deletions=null),!e&&(t=t.sibling,t!==null)){G=t;return}G=t=l}while(t!==null);ft=6,G=null}function bs(t,e,l,a,n,u,i,o,c){t.cancelPendingCommit=null;do Ni();while(_t!==0);if(Q&6)throw Error(y(327));if(e!==null){if(e===t.current)throw Error(y(177));if(u=e.lanes|e.childLanes,u|=Pc,uh(t,l,u,i,o,c),t===$&&(G=$=null,X=0),Ua=e,bl=t,Je=l,Mc=u,Dc=n,Fm=a,e.subtreeFlags&10256||e.flags&10256?(t.callbackNode=null,t.callbackPriority=0,wv(Vu,function(){return rp(),null})):(t.callbackNode=null,t.callbackPriority=0),a=(e.flags&13878)!==0,e.subtreeFlags&13878||a){a=C.T,C.T=null,n=Z.p,Z.p=2,i=Q,Q|=4;try{bv(t,e,l)}finally{Q=i,Z.p=n,C.T=a}}_t=1,ip(),op(),cp()}}function ip(){if(_t===1){_t=0;var t=bl,e=Ua,l=(e.flags&13878)!==0;if(e.subtreeFlags&13878||l){l=C.T,C.T=null;var a=Z.p;Z.p=2;var n=Q;Q|=4;try{jm(e,t);var u=Cc,i=Ed(t.containerInfo),o=u.focusedElem,c=u.selectionRange;if(i!==o&&o&&o.ownerDocument&&zd(o.ownerDocument.documentElement,o)){if(c!==null&&Ic(o)){var s=c.start,h=c.end;if(h===void 0&&(h=s),"selectionStart"in o)o.selectionStart=s,o.selectionEnd=Math.min(h,o.value.length);else{var v=o.ownerDocument||document,d=v&&v.defaultView||window;if(d.getSelection){var p=d.getSelection(),b=o.textContent.length,S=Math.min(c.start,b),T=c.end===void 0?S:Math.min(c.end,b);!p.extend&&S>T&&(i=T,T=S,S=i);var r=Xr(o,S),f=Xr(o,T);if(r&&f&&(p.rangeCount!==1||p.anchorNode!==r.node||p.anchorOffset!==r.offset||p.focusNode!==f.node||p.focusOffset!==f.offset)){var m=v.createRange();m.setStart(r.node,r.offset),p.removeAllRanges(),S>T?(p.addRange(m),p.extend(f.node,f.offset)):(m.setEnd(f.node,f.offset),p.addRange(m))}}}}for(v=[],p=o;p=p.parentNode;)p.nodeType===1&&v.push({element:p,left:p.scrollLeft,top:p.scrollTop});for(typeof o.focus=="function"&&o.focus(),o=0;o<v.length;o++){var g=v[o];g.element.scrollLeft=g.left,g.element.scrollTop=g.top}}vi=!!Nc,Cc=Nc=null}finally{Q=n,Z.p=a,C.T=l}}t.current=e,_t=2}}function op(){if(_t===2){_t=0;var t=bl,e=Ua,l=(e.flags&8772)!==0;if(e.subtreeFlags&8772||l){l=C.T,C.T=null;var a=Z.p;Z.p=2;var n=Q;Q|=4;try{Bm(t,e.alternate,e)}finally{Q=n,Z.p=a,C.T=l}}_t=3}}function cp(){if(_t===4||_t===3){_t=0,$0();var t=bl,e=Ua,l=Je,a=Fm;e.subtreeFlags&10256||e.flags&10256?_t=5:(_t=0,Ua=bl=null,fp(t,t.pendingLanes));var n=t.pendingLanes;if(n===0&&(yl=null),Zc(l),e=e.stateNode,te&&typeof te.onCommitFiberRoot=="function")try{te.onCommitFiberRoot(Xn,e,void 0,(e.current.flags&128)===128)}catch{}if(a!==null){e=C.T,n=Z.p,Z.p=2,C.T=null;try{for(var u=t.onRecoverableError,i=0;i<a.length;i++){var o=a[i];u(o.value,{componentStack:o.stack})}}finally{C.T=e,Z.p=n}}Je&3&&Ni(),Oe(t),n=t.pendingLanes,l&261930&&n&42?t===qc?En++:(En=0,qc=t):En=0,Wn(0,!1)}}function fp(t,e){(t.pooledCacheLanes&=e)===0&&(e=t.pooledCache,e!=null&&(t.pooledCache=null,Vn(e)))}function Ni(){return ip(),op(),cp(),rp()}function rp(){if(_t!==5)return!1;var t=bl,e=Mc;Mc=0;var l=Zc(Je),a=C.T,n=Z.p;try{Z.p=32>l?32:l,C.T=null,l=Dc,Dc=null;var u=bl,i=Je;if(_t=0,Ua=bl=null,Je=0,Q&6)throw Error(y(331));var o=Q;if(Q|=4,Jm(u.current),Zm(u,u.current,i,l),Q=o,Wn(0,!1),te&&typeof te.onPostCommitFiberRoot=="function")try{te.onPostCommitFiberRoot(Xn,u)}catch{}return!0}finally{Z.p=n,C.T=a,fp(t,e)}}function _s(t,e,l){e=de(l,e),e=xc(t.stateNode,e,2),t=gl(t,e,2),t!==null&&(jn(t,2),Oe(t))}function K(t,e,l){if(t.tag===3)_s(t,t,l);else for(;e!==null;){if(e.tag===3){_s(e,t,l);break}else if(e.tag===1){var a=e.stateNode;if(typeof e.type.getDerivedStateFromError=="function"||typeof a.componentDidCatch=="function"&&(yl===null||!yl.has(a))){t=de(l,t),l=Tm(2),a=gl(e,l,2),a!==null&&(Am(l,a,e,t),jn(a,2),Oe(a));break}}e=e.return}}function Go(t,e,l){var a=t.pingCache;if(a===null){a=t.pingCache=new xv;var n=new Set;a.set(e,n)}else n=a.get(e),n===void 0&&(n=new Set,a.set(e,n));n.has(l)||(Mf=!0,n.add(l),t=Mv.bind(null,t,e,l),e.then(t,t))}function Mv(t,e,l){var a=t.pingCache;a!==null&&a.delete(e),t.pingedLanes|=t.suspendedLanes&l,t.warmLanes&=~l,$===t&&(X&l)===l&&(ft===4||ft===3&&(X&62914560)===X&&300>Pt()-qi?!(Q&2)&&Ha(t,0):Df|=l,Ra===X&&(Ra=0)),Oe(t)}function sp(t,e){e===0&&(e=ld()),t=Wl(t,e),t!==null&&(jn(t,e),Oe(t))}function Dv(t){var e=t.memoizedState,l=0;e!==null&&(l=e.retryLane),sp(t,l)}function qv(t,e){var l=0;switch(t.tag){case 31:case 13:var a=t.stateNode,n=t.memoizedState;n!==null&&(l=n.retryLane);break;case 19:a=t.stateNode;break;case 22:a=t.stateNode._retryCache;break;default:throw Error(y(314))}a!==null&&a.delete(e),sp(t,l)}function wv(t,e){return jc(t,e)}var fi=null,ca=null,wc=!1,ri=!1,Xo=!1,ml=0;function Oe(t){t!==ca&&t.next===null&&(ca===null?fi=ca=t:ca=ca.next=t),ri=!0,wc||(wc=!0,Nv())}function Wn(t,e){if(!Xo&&ri){Xo=!0;do for(var l=!1,a=fi;a!==null;){if(!e)if(t!==0){var n=a.pendingLanes;if(n===0)var u=0;else{var i=a.suspendedLanes,o=a.pingedLanes;u=(1<<31-ee(42|t)+1)-1,u&=n&~(i&~o),u=u&201326741?u&201326741|1:u?u|2:0}u!==0&&(l=!0,Ss(a,u))}else u=X,u=yi(a,a===$?u:0,a.cancelPendingCommit!==null||a.timeoutHandle!==-1),!(u&3)||Ln(a,u)||(l=!0,Ss(a,u));a=a.next}while(l);Xo=!1}}function Ov(){dp()}function dp(){ri=wc=!1;var t=0;ml!==0&&Gv()&&(t=ml);for(var e=Pt(),l=null,a=fi;a!==null;){var n=a.next,u=mp(a,e);u===0?(a.next=null,l===null?fi=n:l.next=n,n===null&&(ca=l)):(l=a,(t!==0||u&3)&&(ri=!0)),a=n}_t!==0&&_t!==5||Wn(t,!1),ml!==0&&(ml=0)}function mp(t,e){for(var l=t.suspendedLanes,a=t.pingedLanes,n=t.expirationTimes,u=t.pendingLanes&-62914561;0<u;){var i=31-ee(u),o=1<<i,c=n[i];c===-1?(!(o&l)||o&a)&&(n[i]=nh(o,e)):c<=e&&(t.expiredLanes|=o),u&=~o}if(e=$,l=X,l=yi(t,t===e?l:0,t.cancelPendingCommit!==null||t.timeoutHandle!==-1),a=t.callbackNode,l===0||t===e&&(V===2||V===9)||t.cancelPendingCommit!==null)return a!==null&&a!==null&&vo(a),t.callbackNode=null,t.callbackPriority=0;if(!(l&3)||Ln(t,l)){if(e=l&-l,e===t.callbackPriority)return e;switch(a!==null&&vo(a),Zc(l)){case 2:case 8:l=td;break;case 32:l=Vu;break;case 268435456:l=ed;break;default:l=Vu}return a=pp.bind(null,t),l=jc(l,a),t.callbackPriority=e,t.callbackNode=l,e}return a!==null&&a!==null&&vo(a),t.callbackPriority=2,t.callbackNode=null,2}function pp(t,e){if(_t!==0&&_t!==5)return t.callbackNode=null,t.callbackPriority=0,null;var l=t.callbackNode;if(Ni()&&t.callbackNode!==l)return null;var a=X;return a=yi(t,t===$?a:0,t.cancelPendingCommit!==null||t.timeoutHandle!==-1),a===0?null:(Pm(t,a,e),mp(t,Pt()),t.callbackNode!=null&&t.callbackNode===l?pp.bind(null,t):null)}function Ss(t,e){if(Ni())return null;Pm(t,e,!0)}function Nv(){Lv(function(){Q&6?jc(Ps,Ov):dp()})}function wf(){if(ml===0){var t=Oa;t===0&&(t=su,su<<=1,!(su&261888)&&(su=256)),ml=t}return ml}function xs(t){return t==null||typeof t=="symbol"||typeof t=="boolean"?null:typeof t=="function"?t:qu(""+t)}function zs(t,e){var l=e.ownerDocument.createElement("input");return l.name=e.name,l.value=e.value,t.id&&l.setAttribute("form",t.id),e.parentNode.insertBefore(l,e),t=new FormData(t),l.parentNode.removeChild(l),t}function Cv(t,e,l,a,n){if(e==="submit"&&l&&l.stateNode===n){var u=xs((n[Zt]||null).action),i=a.submitter;i&&(e=(e=i[Zt]||null)?xs(e.formAction):i.getAttribute("formAction"),e!==null&&(u=e,i=null));var o=new bi("action","action",null,a,n);t.push({event:o,listeners:[{instance:null,listener:function(){if(a.defaultPrevented){if(ml!==0){var c=i?zs(n,i):new FormData(n);_c(l,{pending:!0,data:c,method:n.method,action:u},null,c)}}else typeof u=="function"&&(o.preventDefault(),c=i?zs(n,i):new FormData(n),_c(l,{pending:!0,data:c,method:n.method,action:u},u,c))},currentTarget:n}]})}}for(zu=0;zu<cc.length;zu++)Eu=cc[zu],Es=Eu.toLowerCase(),Ts=Eu[0].toUpperCase()+Eu.slice(1),_e(Es,"on"+Ts);var Eu,Es,Ts,zu;_e(Ad,"onAnimationEnd");_e(Md,"onAnimationIteration");_e(Dd,"onAnimationStart");_e("dblclick","onDoubleClick");_e("focusin","onFocus");_e("focusout","onBlur");_e(Fh,"onTransitionRun");_e(Ih,"onTransitionStart");_e(Ph,"onTransitionCancel");_e(qd,"onTransitionEnd");qa("onMouseEnter",["mouseout","mouseover"]);qa("onMouseLeave",["mouseout","mouseover"]);qa("onPointerEnter",["pointerout","pointerover"]);qa("onPointerLeave",["pointerout","pointerover"]);Vl("onChange","change click focusin focusout input keydown keyup selectionchange".split(" "));Vl("onSelect","focusout contextmenu dragend focusin keydown keyup mousedown mouseup selectionchange".split(" "));Vl("onBeforeInput",["compositionend","keypress","textInput","paste"]);Vl("onCompositionEnd","compositionend focusout keydown keypress keyup mousedown".split(" "));Vl("onCompositionStart","compositionstart focusout keydown keypress keyup mousedown".split(" "));Vl("onCompositionUpdate","compositionupdate focusout keydown keypress keyup mousedown".split(" "));var Un="abort canplay canplaythrough durationchange emptied encrypted ended error loadeddata loadedmetadata loadstart pause play playing progress ratechange resize seeked seeking stalled suspend timeupdate volumechange waiting".split(" "),Rv=new Set("beforetoggle cancel close invalid load scroll scrollend toggle".split(" ").concat(Un));function hp(t,e){e=(e&4)!==0;for(var l=0;l<t.length;l++){var a=t[l],n=a.event;a=a.listeners;t:{var u=void 0;if(e)for(var i=a.length-1;0<=i;i--){var o=a[i],c=o.instance,s=o.currentTarget;if(o=o.listener,c!==u&&n.isPropagationStopped())break t;u=o,n.currentTarget=s;try{u(n)}catch(h){Ju(h)}n.currentTarget=null,u=c}else for(i=0;i<a.length;i++){if(o=a[i],c=o.instance,s=o.currentTarget,o=o.listener,c!==u&&n.isPropagationStopped())break t;u=o,n.currentTarget=s;try{u(n)}catch(h){Ju(h)}n.currentTarget=null,u=c}}}}function Y(t,e){var l=e[tc];l===void 0&&(l=e[tc]=new Set);var a=t+"__bubble";l.has(a)||(vp(e,t,2,!1),l.add(a))}function Lo(t,e,l){var a=0;e&&(a|=4),vp(l,t,a,e)}var Tu="_reactListening"+Math.random().toString(36).slice(2);function Of(t){if(!t[Tu]){t[Tu]=!0,od.forEach(function(l){l!=="selectionchange"&&(Rv.has(l)||Lo(l,!1,t),Lo(l,!0,t))});var e=t.nodeType===9?t:t.ownerDocument;e===null||e[Tu]||(e[Tu]=!0,Lo("selectionchange",!1,e))}}function vp(t,e,l,a){switch(qp(e)){case 2:var n=cg;break;case 8:n=fg;break;default:n=Uf}l=n.bind(null,e,l,t),n=void 0,!uc||e!=="touchstart"&&e!=="touchmove"&&e!=="wheel"||(n=!0),a?n!==void 0?t.addEventListener(e,l,{capture:!0,passive:n}):t.addEventListener(e,l,!0):n!==void 0?t.addEventListener(e,l,{passive:n}):t.addEventListener(e,l,!1)}function jo(t,e,l,a,n){var u=a;if(!(e&1)&&!(e&2)&&a!==null)t:for(;;){if(a===null)return;var i=a.tag;if(i===3||i===4){var o=a.stateNode.containerInfo;if(o===n)break;if(i===4)for(i=a.return;i!==null;){var c=i.tag;if((c===3||c===4)&&i.stateNode.containerInfo===n)return;i=i.return}for(;o!==null;){if(i=sa(o),i===null)return;if(c=i.tag,c===5||c===6||c===26||c===27){a=u=i;continue t}o=o.parentNode}}a=a.return}hd(function(){var s=u,h=Jc(l),v=[];t:{var d=wd.get(t);if(d!==void 0){var p=bi,b=t;switch(t){case"keypress":if(Ou(l)===0)break t;case"keydown":case"keyup":p=qh;break;case"focusin":b="focus",p=So;break;case"focusout":b="blur",p=So;break;case"beforeblur":case"afterblur":p=So;break;case"click":if(l.button===2)break t;case"auxclick":case"dblclick":case"mousedown":case"mousemove":case"mouseup":case"mouseout":case"mouseover":case"contextmenu":p=Nr;break;case"drag":case"dragend":case"dragenter":case"dragexit":case"dragleave":case"dragover":case"dragstart":case"drop":p=gh;break;case"touchcancel":case"touchend":case"touchmove":case"touchstart":p=Nh;break;case Ad:case Md:case Dd:p=_h;break;case qd:p=Rh;break;case"scroll":case"scrollend":p=hh;break;case"wheel":p=Hh;break;case"copy":case"cut":case"paste":p=xh;break;case"gotpointercapture":case"lostpointercapture":case"pointercancel":case"pointerdown":case"pointermove":case"pointerout":case"pointerover":case"pointerup":p=Rr;break;case"toggle":case"beforetoggle":p=Bh}var S=(e&4)!==0,T=!S&&(t==="scroll"||t==="scrollend"),r=S?d!==null?d+"Capture":null:d;S=[];for(var f=s,m;f!==null;){var g=f;if(m=g.stateNode,g=g.tag,g!==5&&g!==26&&g!==27||m===null||r===null||(g=Mn(f,r),g!=null&&S.push(Hn(f,g,m))),T)break;f=f.return}0<S.length&&(d=new p(d,b,null,l,h),v.push({event:d,listeners:S}))}}if(!(e&7)){t:{if(d=t==="mouseover"||t==="pointerover",p=t==="mouseout"||t==="pointerout",d&&l!==nc&&(b=l.relatedTarget||l.fromElement)&&(sa(b)||b[Ya]))break t;if((p||d)&&(d=h.window===h?h:(d=h.ownerDocument)?d.defaultView||d.parentWindow:window,p?(b=l.relatedTarget||l.toElement,p=s,b=b?sa(b):null,b!==null&&(T=Gn(b),S=b.tag,b!==T||S!==5&&S!==27&&S!==6)&&(b=null)):(p=null,b=s),p!==b)){if(S=Nr,g="onMouseLeave",r="onMouseEnter",f="mouse",(t==="pointerout"||t==="pointerover")&&(S=Rr,g="onPointerLeave",r="onPointerEnter",f="pointer"),T=p==null?d:rn(p),m=b==null?d:rn(b),d=new S(g,f+"leave",p,l,h),d.target=T,d.relatedTarget=m,g=null,sa(h)===s&&(S=new S(r,f+"enter",b,l,h),S.target=m,S.relatedTarget=T,g=S),T=g,p&&b)e:{for(S=Uv,r=p,f=b,m=0,g=r;g;g=S(g))m++;g=0;for(var E=f;E;E=S(E))g++;for(;0<m-g;)r=S(r),m--;for(;0<g-m;)f=S(f),g--;for(;m--;){if(r===f||f!==null&&r===f.alternate){S=r;break e}r=S(r),f=S(f)}S=null}else S=null;p!==null&&As(v,d,p,S,!1),b!==null&&T!==null&&As(v,T,b,S,!0)}}t:{if(d=s?rn(s):window,p=d.nodeName&&d.nodeName.toLowerCase(),p==="select"||p==="input"&&d.type==="file")var w=Br;else if(kr(d))if(Sd)w=Jh;else{w=Vh;var x=Zh}else p=d.nodeName,!p||p.toLowerCase()!=="input"||d.type!=="checkbox"&&d.type!=="radio"?s&&Kc(s.elementType)&&(w=Br):w=Kh;if(w&&(w=w(t,s))){_d(v,w,l,h);break t}x&&x(t,d,s),t==="focusout"&&s&&d.type==="number"&&s.memoizedProps.value!=null&&ac(d,"number",d.value)}switch(x=s?rn(s):window,t){case"focusin":(kr(x)||x.contentEditable==="true")&&(pa=x,ic=s,hn=null);break;case"focusout":hn=ic=pa=null;break;case"mousedown":oc=!0;break;case"contextmenu":case"mouseup":case"dragend":oc=!1,Lr(v,l,h);break;case"selectionchange":if($h)break;case"keydown":case"keyup":Lr(v,l,h)}var M;if(Fc)t:{switch(t){case"compositionstart":var O="onCompositionStart";break t;case"compositionend":O="onCompositionEnd";break t;case"compositionupdate":O="onCompositionUpdate";break t}O=void 0}else ma?yd(t,l)&&(O="onCompositionEnd"):t==="keydown"&&l.keyCode===229&&(O="onCompositionStart");O&&(gd&&l.locale!=="ko"&&(ma||O!=="onCompositionStart"?O==="onCompositionEnd"&&ma&&(M=vd()):(rl=h,Wc="value"in rl?rl.value:rl.textContent,ma=!0)),x=si(s,O),0<x.length&&(O=new Cr(O,t,null,l,h),v.push({event:O,listeners:x}),M?O.data=M:(M=bd(l),M!==null&&(O.data=M)))),(M=Gh?Xh(t,l):Lh(t,l))&&(O=si(s,"onBeforeInput"),0<O.length&&(x=new Cr("onBeforeInput","beforeinput",null,l,h),v.push({event:x,listeners:O}),x.data=M)),Cv(v,t,s,l,h)}hp(v,e)})}function Hn(t,e,l){return{instance:t,listener:e,currentTarget:l}}function si(t,e){for(var l=e+"Capture",a=[];t!==null;){var n=t,u=n.stateNode;if(n=n.tag,n!==5&&n!==26&&n!==27||u===null||(n=Mn(t,l),n!=null&&a.unshift(Hn(t,n,u)),n=Mn(t,e),n!=null&&a.push(Hn(t,n,u))),t.tag===3)return a;t=t.return}return[]}function Uv(t){if(t===null)return null;do t=t.return;while(t&&t.tag!==5&&t.tag!==27);return t||null}function As(t,e,l,a,n){for(var u=e._reactName,i=[];l!==null&&l!==a;){var o=l,c=o.alternate,s=o.stateNode;if(o=o.tag,c!==null&&c===a)break;o!==5&&o!==26&&o!==27||s===null||(c=s,n?(s=Mn(l,u),s!=null&&i.unshift(Hn(l,s,c))):n||(s=Mn(l,u),s!=null&&i.push(Hn(l,s,c)))),l=l.return}i.length!==0&&t.push({event:e,listeners:i})}var Hv=/\r\n?/g,kv=/\u0000|\uFFFD/g;function Ms(t){return(typeof t=="string"?t:""+t).replace(Hv,`
`).replace(kv,"")}function gp(t,e){return e=Ms(e),Ms(t)===e}function J(t,e,l,a,n,u){switch(l){case"children":typeof a=="string"?e==="body"||e==="textarea"&&a===""||wa(t,a):(typeof a=="number"||typeof a=="bigint")&&e!=="body"&&wa(t,""+a);break;case"className":pu(t,"class",a);break;case"tabIndex":pu(t,"tabindex",a);break;case"dir":case"role":case"viewBox":case"width":case"height":pu(t,l,a);break;case"style":pd(t,a,u);break;case"data":if(e!=="object"){pu(t,"data",a);break}case"src":case"href":if(a===""&&(e!=="a"||l!=="href")){t.removeAttribute(l);break}if(a==null||typeof a=="function"||typeof a=="symbol"||typeof a=="boolean"){t.removeAttribute(l);break}a=qu(""+a),t.setAttribute(l,a);break;case"action":case"formAction":if(typeof a=="function"){t.setAttribute(l,"javascript:throw new Error('A React form was unexpectedly submitted. If you called form.submit() manually, consider using form.requestSubmit() instead. If you\\'re trying to use event.stopPropagation() in a submit event handler, consider also calling event.preventDefault().')");break}else typeof u=="function"&&(l==="formAction"?(e!=="input"&&J(t,e,"name",n.name,n,null),J(t,e,"formEncType",n.formEncType,n,null),J(t,e,"formMethod",n.formMethod,n,null),J(t,e,"formTarget",n.formTarget,n,null)):(J(t,e,"encType",n.encType,n,null),J(t,e,"method",n.method,n,null),J(t,e,"target",n.target,n,null)));if(a==null||typeof a=="symbol"||typeof a=="boolean"){t.removeAttribute(l);break}a=qu(""+a),t.setAttribute(l,a);break;case"onClick":a!=null&&(t.onclick=Qe);break;case"onScroll":a!=null&&Y("scroll",t);break;case"onScrollEnd":a!=null&&Y("scrollend",t);break;case"dangerouslySetInnerHTML":if(a!=null){if(typeof a!="object"||!("__html"in a))throw Error(y(61));if(l=a.__html,l!=null){if(n.children!=null)throw Error(y(60));t.innerHTML=l}}break;case"multiple":t.multiple=a&&typeof a!="function"&&typeof a!="symbol";break;case"muted":t.muted=a&&typeof a!="function"&&typeof a!="symbol";break;case"suppressContentEditableWarning":case"suppressHydrationWarning":case"defaultValue":case"defaultChecked":case"innerHTML":case"ref":break;case"autoFocus":break;case"xlinkHref":if(a==null||typeof a=="function"||typeof a=="boolean"||typeof a=="symbol"){t.removeAttribute("xlink:href");break}l=qu(""+a),t.setAttributeNS("http://www.w3.org/1999/xlink","xlink:href",l);break;case"contentEditable":case"spellCheck":case"draggable":case"value":case"autoReverse":case"externalResourcesRequired":case"focusable":case"preserveAlpha":a!=null&&typeof a!="function"&&typeof a!="symbol"?t.setAttribute(l,""+a):t.removeAttribute(l);break;case"inert":case"allowFullScreen":case"async":case"autoPlay":case"controls":case"default":case"defer":case"disabled":case"disablePictureInPicture":case"disableRemotePlayback":case"formNoValidate":case"hidden":case"loop":case"noModule":case"noValidate":case"open":case"playsInline":case"readOnly":case"required":case"reversed":case"scoped":case"seamless":case"itemScope":a&&typeof a!="function"&&typeof a!="symbol"?t.setAttribute(l,""):t.removeAttribute(l);break;case"capture":case"download":a===!0?t.setAttribute(l,""):a!==!1&&a!=null&&typeof a!="function"&&typeof a!="symbol"?t.setAttribute(l,a):t.removeAttribute(l);break;case"cols":case"rows":case"size":case"span":a!=null&&typeof a!="function"&&typeof a!="symbol"&&!isNaN(a)&&1<=a?t.setAttribute(l,a):t.removeAttribute(l);break;case"rowSpan":case"start":a==null||typeof a=="function"||typeof a=="symbol"||isNaN(a)?t.removeAttribute(l):t.setAttribute(l,a);break;case"popover":Y("beforetoggle",t),Y("toggle",t),Du(t,"popover",a);break;case"xlinkActuate":He(t,"http://www.w3.org/1999/xlink","xlink:actuate",a);break;case"xlinkArcrole":He(t,"http://www.w3.org/1999/xlink","xlink:arcrole",a);break;case"xlinkRole":He(t,"http://www.w3.org/1999/xlink","xlink:role",a);break;case"xlinkShow":He(t,"http://www.w3.org/1999/xlink","xlink:show",a);break;case"xlinkTitle":He(t,"http://www.w3.org/1999/xlink","xlink:title",a);break;case"xlinkType":He(t,"http://www.w3.org/1999/xlink","xlink:type",a);break;case"xmlBase":He(t,"http://www.w3.org/XML/1998/namespace","xml:base",a);break;case"xmlLang":He(t,"http://www.w3.org/XML/1998/namespace","xml:lang",a);break;case"xmlSpace":He(t,"http://www.w3.org/XML/1998/namespace","xml:space",a);break;case"is":Du(t,"is",a);break;case"innerText":case"textContent":break;default:(!(2<l.length)||l[0]!=="o"&&l[0]!=="O"||l[1]!=="n"&&l[1]!=="N")&&(l=mh.get(l)||l,Du(t,l,a))}}function Oc(t,e,l,a,n,u){switch(l){case"style":pd(t,a,u);break;case"dangerouslySetInnerHTML":if(a!=null){if(typeof a!="object"||!("__html"in a))throw Error(y(61));if(l=a.__html,l!=null){if(n.children!=null)throw Error(y(60));t.innerHTML=l}}break;case"children":typeof a=="string"?wa(t,a):(typeof a=="number"||typeof a=="bigint")&&wa(t,""+a);break;case"onScroll":a!=null&&Y("scroll",t);break;case"onScrollEnd":a!=null&&Y("scrollend",t);break;case"onClick":a!=null&&(t.onclick=Qe);break;case"suppressContentEditableWarning":case"suppressHydrationWarning":case"innerHTML":case"ref":break;case"innerText":case"textContent":break;default:if(!cd.hasOwnProperty(l))t:{if(l[0]==="o"&&l[1]==="n"&&(n=l.endsWith("Capture"),e=l.slice(2,n?l.length-7:void 0),u=t[Zt]||null,u=u!=null?u[l]:null,typeof u=="function"&&t.removeEventListener(e,u,n),typeof a=="function")){typeof u!="function"&&u!==null&&(l in t?t[l]=null:t.hasAttribute(l)&&t.removeAttribute(l)),t.addEventListener(e,a,n);break t}l in t?t[l]=a:a===!0?t.setAttribute(l,""):Du(t,l,a)}}}function Ot(t,e,l){switch(e){case"div":case"span":case"svg":case"path":case"a":case"g":case"p":case"li":break;case"img":Y("error",t),Y("load",t);var a=!1,n=!1,u;for(u in l)if(l.hasOwnProperty(u)){var i=l[u];if(i!=null)switch(u){case"src":a=!0;break;case"srcSet":n=!0;break;case"children":case"dangerouslySetInnerHTML":throw Error(y(137,e));default:J(t,e,u,i,l,null)}}n&&J(t,e,"srcSet",l.srcSet,l,null),a&&J(t,e,"src",l.src,l,null);return;case"input":Y("invalid",t);var o=u=i=n=null,c=null,s=null;for(a in l)if(l.hasOwnProperty(a)){var h=l[a];if(h!=null)switch(a){case"name":n=h;break;case"type":i=h;break;case"checked":c=h;break;case"defaultChecked":s=h;break;case"value":u=h;break;case"defaultValue":o=h;break;case"children":case"dangerouslySetInnerHTML":if(h!=null)throw Error(y(137,e));break;default:J(t,e,a,h,l,null)}}sd(t,u,o,c,s,i,n,!1);return;case"select":Y("invalid",t),a=i=u=null;for(n in l)if(l.hasOwnProperty(n)&&(o=l[n],o!=null))switch(n){case"value":u=o;break;case"defaultValue":i=o;break;case"multiple":a=o;default:J(t,e,n,o,l,null)}e=u,l=i,t.multiple=!!a,e!=null?xa(t,!!a,e,!1):l!=null&&xa(t,!!a,l,!0);return;case"textarea":Y("invalid",t),u=n=a=null;for(i in l)if(l.hasOwnProperty(i)&&(o=l[i],o!=null))switch(i){case"value":a=o;break;case"defaultValue":n=o;break;case"children":u=o;break;case"dangerouslySetInnerHTML":if(o!=null)throw Error(y(91));break;default:J(t,e,i,o,l,null)}md(t,a,n,u);return;case"option":for(c in l)if(l.hasOwnProperty(c)&&(a=l[c],a!=null))switch(c){case"selected":t.selected=a&&typeof a!="function"&&typeof a!="symbol";break;default:J(t,e,c,a,l,null)}return;case"dialog":Y("beforetoggle",t),Y("toggle",t),Y("cancel",t),Y("close",t);break;case"iframe":case"object":Y("load",t);break;case"video":case"audio":for(a=0;a<Un.length;a++)Y(Un[a],t);break;case"image":Y("error",t),Y("load",t);break;case"details":Y("toggle",t);break;case"embed":case"source":case"link":Y("error",t),Y("load",t);case"area":case"base":case"br":case"col":case"hr":case"keygen":case"meta":case"param":case"track":case"wbr":case"menuitem":for(s in l)if(l.hasOwnProperty(s)&&(a=l[s],a!=null))switch(s){case"children":case"dangerouslySetInnerHTML":throw Error(y(137,e));default:J(t,e,s,a,l,null)}return;default:if(Kc(e)){for(h in l)l.hasOwnProperty(h)&&(a=l[h],a!==void 0&&Oc(t,e,h,a,l,void 0));return}}for(o in l)l.hasOwnProperty(o)&&(a=l[o],a!=null&&J(t,e,o,a,l,null))}function Bv(t,e,l,a){switch(e){case"div":case"span":case"svg":case"path":case"a":case"g":case"p":case"li":break;case"input":var n=null,u=null,i=null,o=null,c=null,s=null,h=null;for(p in l){var v=l[p];if(l.hasOwnProperty(p)&&v!=null)switch(p){case"checked":break;case"value":break;case"defaultValue":c=v;default:a.hasOwnProperty(p)||J(t,e,p,null,a,v)}}for(var d in a){var p=a[d];if(v=l[d],a.hasOwnProperty(d)&&(p!=null||v!=null))switch(d){case"type":u=p;break;case"name":n=p;break;case"checked":s=p;break;case"defaultChecked":h=p;break;case"value":i=p;break;case"defaultValue":o=p;break;case"children":case"dangerouslySetInnerHTML":if(p!=null)throw Error(y(137,e));break;default:p!==v&&J(t,e,d,p,a,v)}}lc(t,i,o,c,s,h,u,n);return;case"select":p=i=o=d=null;for(u in l)if(c=l[u],l.hasOwnProperty(u)&&c!=null)switch(u){case"value":break;case"multiple":p=c;default:a.hasOwnProperty(u)||J(t,e,u,null,a,c)}for(n in a)if(u=a[n],c=l[n],a.hasOwnProperty(n)&&(u!=null||c!=null))switch(n){case"value":d=u;break;case"defaultValue":o=u;break;case"multiple":i=u;default:u!==c&&J(t,e,n,u,a,c)}e=o,l=i,a=p,d!=null?xa(t,!!l,d,!1):!!a!=!!l&&(e!=null?xa(t,!!l,e,!0):xa(t,!!l,l?[]:"",!1));return;case"textarea":p=d=null;for(o in l)if(n=l[o],l.hasOwnProperty(o)&&n!=null&&!a.hasOwnProperty(o))switch(o){case"value":break;case"children":break;default:J(t,e,o,null,a,n)}for(i in a)if(n=a[i],u=l[i],a.hasOwnProperty(i)&&(n!=null||u!=null))switch(i){case"value":d=n;break;case"defaultValue":p=n;break;case"children":break;case"dangerouslySetInnerHTML":if(n!=null)throw Error(y(91));break;default:n!==u&&J(t,e,i,n,a,u)}dd(t,d,p);return;case"option":for(var b in l)if(d=l[b],l.hasOwnProperty(b)&&d!=null&&!a.hasOwnProperty(b))switch(b){case"selected":t.selected=!1;break;default:J(t,e,b,null,a,d)}for(c in a)if(d=a[c],p=l[c],a.hasOwnProperty(c)&&d!==p&&(d!=null||p!=null))switch(c){case"selected":t.selected=d&&typeof d!="function"&&typeof d!="symbol";break;default:J(t,e,c,d,a,p)}return;case"img":case"link":case"area":case"base":case"br":case"col":case"embed":case"hr":case"keygen":case"meta":case"param":case"source":case"track":case"wbr":case"menuitem":for(var S in l)d=l[S],l.hasOwnProperty(S)&&d!=null&&!a.hasOwnProperty(S)&&J(t,e,S,null,a,d);for(s in a)if(d=a[s],p=l[s],a.hasOwnProperty(s)&&d!==p&&(d!=null||p!=null))switch(s){case"children":case"dangerouslySetInnerHTML":if(d!=null)throw Error(y(137,e));break;default:J(t,e,s,d,a,p)}return;default:if(Kc(e)){for(var T in l)d=l[T],l.hasOwnProperty(T)&&d!==void 0&&!a.hasOwnProperty(T)&&Oc(t,e,T,void 0,a,d);for(h in a)d=a[h],p=l[h],!a.hasOwnProperty(h)||d===p||d===void 0&&p===void 0||Oc(t,e,h,d,a,p);return}}for(var r in l)d=l[r],l.hasOwnProperty(r)&&d!=null&&!a.hasOwnProperty(r)&&J(t,e,r,null,a,d);for(v in a)d=a[v],p=l[v],!a.hasOwnProperty(v)||d===p||d==null&&p==null||J(t,e,v,d,a,p)}function Ds(t){switch(t){case"css":case"script":case"font":case"img":case"image":case"input":case"link":return!0;default:return!1}}function Yv(){if(typeof performance.getEntriesByType=="function"){for(var t=0,e=0,l=performance.getEntriesByType("resource"),a=0;a<l.length;a++){var n=l[a],u=n.transferSize,i=n.initiatorType,o=n.duration;if(u&&o&&Ds(i)){for(i=0,o=n.responseEnd,a+=1;a<l.length;a++){var c=l[a],s=c.startTime;if(s>o)break;var h=c.transferSize,v=c.initiatorType;h&&Ds(v)&&(c=c.responseEnd,i+=h*(c<o?1:(o-s)/(c-s)))}if(--a,e+=8*(u+i)/(n.duration/1e3),t++,10<t)break}}if(0<t)return e/t/1e6}return navigator.connection&&(t=navigator.connection.downlink,typeof t=="number")?t:5}var Nc=null,Cc=null;function di(t){return t.nodeType===9?t:t.ownerDocument}function qs(t){switch(t){case"http://www.w3.org/2000/svg":return 1;case"http://www.w3.org/1998/Math/MathML":return 2;default:return 0}}function yp(t,e){if(t===0)switch(e){case"svg":return 1;case"math":return 2;default:return 0}return t===1&&e==="foreignObject"?0:t}function Rc(t,e){return t==="textarea"||t==="noscript"||typeof e.children=="string"||typeof e.children=="number"||typeof e.children=="bigint"||typeof e.dangerouslySetInnerHTML=="object"&&e.dangerouslySetInnerHTML!==null&&e.dangerouslySetInnerHTML.__html!=null}var Qo=null;function Gv(){var t=window.event;return t&&t.type==="popstate"?t===Qo?!1:(Qo=t,!0):(Qo=null,!1)}var bp=typeof setTimeout=="function"?setTimeout:void 0,Xv=typeof clearTimeout=="function"?clearTimeout:void 0,ws=typeof Promise=="function"?Promise:void 0,Lv=typeof queueMicrotask=="function"?queueMicrotask:typeof ws<"u"?function(t){return ws.resolve(null).then(t).catch(jv)}:bp;function jv(t){setTimeout(function(){throw t})}function Dl(t){return t==="head"}function Os(t,e){var l=e,a=0;do{var n=l.nextSibling;if(t.removeChild(l),n&&n.nodeType===8)if(l=n.data,l==="/$"||l==="/&"){if(a===0){t.removeChild(n),Ba(e);return}a--}else if(l==="$"||l==="$?"||l==="$~"||l==="$!"||l==="&")a++;else if(l==="html")Tn(t.ownerDocument.documentElement);else if(l==="head"){l=t.ownerDocument.head,Tn(l);for(var u=l.firstChild;u;){var i=u.nextSibling,o=u.nodeName;u[Qn]||o==="SCRIPT"||o==="STYLE"||o==="LINK"&&u.rel.toLowerCase()==="stylesheet"||l.removeChild(u),u=i}}else l==="body"&&Tn(t.ownerDocument.body);l=n}while(l);Ba(e)}function Ns(t,e){var l=t;t=0;do{var a=l.nextSibling;if(l.nodeType===1?e?(l._stashedDisplay=l.style.display,l.style.display="none"):(l.style.display=l._stashedDisplay||"",l.getAttribute("style")===""&&l.removeAttribute("style")):l.nodeType===3&&(e?(l._stashedText=l.nodeValue,l.nodeValue=""):l.nodeValue=l._stashedText||""),a&&a.nodeType===8)if(l=a.data,l==="/$"){if(t===0)break;t--}else l!=="$"&&l!=="$?"&&l!=="$~"&&l!=="$!"||t++;l=a}while(l)}function Uc(t){var e=t.firstChild;for(e&&e.nodeType===10&&(e=e.nextSibling);e;){var l=e;switch(e=e.nextSibling,l.nodeName){case"HTML":case"HEAD":case"BODY":Uc(l),Vc(l);continue;case"SCRIPT":case"STYLE":continue;case"LINK":if(l.rel.toLowerCase()==="stylesheet")continue}t.removeChild(l)}}function Qv(t,e,l,a){for(;t.nodeType===1;){var n=l;if(t.nodeName.toLowerCase()!==e.toLowerCase()){if(!a&&(t.nodeName!=="INPUT"||t.type!=="hidden"))break}else if(a){if(!t[Qn])switch(e){case"meta":if(!t.hasAttribute("itemprop"))break;return t;case"link":if(u=t.getAttribute("rel"),u==="stylesheet"&&t.hasAttribute("data-precedence"))break;if(u!==n.rel||t.getAttribute("href")!==(n.href==null||n.href===""?null:n.href)||t.getAttribute("crossorigin")!==(n.crossOrigin==null?null:n.crossOrigin)||t.getAttribute("title")!==(n.title==null?null:n.title))break;return t;case"style":if(t.hasAttribute("data-precedence"))break;return t;case"script":if(u=t.getAttribute("src"),(u!==(n.src==null?null:n.src)||t.getAttribute("type")!==(n.type==null?null:n.type)||t.getAttribute("crossorigin")!==(n.crossOrigin==null?null:n.crossOrigin))&&u&&t.hasAttribute("async")&&!t.hasAttribute("itemprop"))break;return t;default:return t}}else if(e==="input"&&t.type==="hidden"){var u=n.name==null?null:""+n.name;if(n.type==="hidden"&&t.getAttribute("name")===u)return t}else return t;if(t=he(t.nextSibling),t===null)break}return null}function Zv(t,e,l){if(e==="")return null;for(;t.nodeType!==3;)if((t.nodeType!==1||t.nodeName!=="INPUT"||t.type!=="hidden")&&!l||(t=he(t.nextSibling),t===null))return null;return t}function _p(t,e){for(;t.nodeType!==8;)if((t.nodeType!==1||t.nodeName!=="INPUT"||t.type!=="hidden")&&!e||(t=he(t.nextSibling),t===null))return null;return t}function Hc(t){return t.data==="$?"||t.data==="$~"}function kc(t){return t.data==="$!"||t.data==="$?"&&t.ownerDocument.readyState!=="loading"}function Vv(t,e){var l=t.ownerDocument;if(t.data==="$~")t._reactRetry=e;else if(t.data!=="$?"||l.readyState!=="loading")e();else{var a=function(){e(),l.removeEventListener("DOMContentLoaded",a)};l.addEventListener("DOMContentLoaded",a),t._reactRetry=a}}function he(t){for(;t!=null;t=t.nextSibling){var e=t.nodeType;if(e===1||e===3)break;if(e===8){if(e=t.data,e==="$"||e==="$!"||e==="$?"||e==="$~"||e==="&"||e==="F!"||e==="F")break;if(e==="/$"||e==="/&")return null}}return t}var Bc=null;function Cs(t){t=t.nextSibling;for(var e=0;t;){if(t.nodeType===8){var l=t.data;if(l==="/$"||l==="/&"){if(e===0)return he(t.nextSibling);e--}else l!=="$"&&l!=="$!"&&l!=="$?"&&l!=="$~"&&l!=="&"||e++}t=t.nextSibling}return null}function Rs(t){t=t.previousSibling;for(var e=0;t;){if(t.nodeType===8){var l=t.data;if(l==="$"||l==="$!"||l==="$?"||l==="$~"||l==="&"){if(e===0)return t;e--}else l!=="/$"&&l!=="/&"||e++}t=t.previousSibling}return null}function Sp(t,e,l){switch(e=di(l),t){case"html":if(t=e.documentElement,!t)throw Error(y(452));return t;case"head":if(t=e.head,!t)throw Error(y(453));return t;case"body":if(t=e.body,!t)throw Error(y(454));return t;default:throw Error(y(451))}}function Tn(t){for(var e=t.attributes;e.length;)t.removeAttributeNode(e[0]);Vc(t)}var ve=new Map,Us=new Set;function mi(t){return typeof t.getRootNode=="function"?t.getRootNode():t.nodeType===9?t:t.ownerDocument}var tl=Z.d;Z.d={f:Kv,r:Jv,D:Wv,C:$v,L:Fv,m:Iv,X:tg,S:Pv,M:eg};function Kv(){var t=tl.f(),e=wi();return t||e}function Jv(t){var e=Ga(t);e!==null&&e.tag===5&&e.type==="form"?pm(e):tl.r(t)}var Qa=typeof document>"u"?null:document;function xp(t,e,l){var a=Qa;if(a&&typeof e=="string"&&e){var n=se(e);n='link[rel="'+t+'"][href="'+n+'"]',typeof l=="string"&&(n+='[crossorigin="'+l+'"]'),Us.has(n)||(Us.add(n),t={rel:t,crossOrigin:l,href:e},a.querySelector(n)===null&&(e=a.createElement("link"),Ot(e,"link",t),Tt(e),a.head.appendChild(e)))}}function Wv(t){tl.D(t),xp("dns-prefetch",t,null)}function $v(t,e){tl.C(t,e),xp("preconnect",t,e)}function Fv(t,e,l){tl.L(t,e,l);var a=Qa;if(a&&t&&e){var n='link[rel="preload"][as="'+se(e)+'"]';e==="image"&&l&&l.imageSrcSet?(n+='[imagesrcset="'+se(l.imageSrcSet)+'"]',typeof l.imageSizes=="string"&&(n+='[imagesizes="'+se(l.imageSizes)+'"]')):n+='[href="'+se(t)+'"]';var u=n;switch(e){case"style":u=ka(t);break;case"script":u=Za(t)}ve.has(u)||(t=et({rel:"preload",href:e==="image"&&l&&l.imageSrcSet?void 0:t,as:e},l),ve.set(u,t),a.querySelector(n)!==null||e==="style"&&a.querySelector($n(u))||e==="script"&&a.querySelector(Fn(u))||(e=a.createElement("link"),Ot(e,"link",t),Tt(e),a.head.appendChild(e)))}}function Iv(t,e){tl.m(t,e);var l=Qa;if(l&&t){var a=e&&typeof e.as=="string"?e.as:"script",n='link[rel="modulepreload"][as="'+se(a)+'"][href="'+se(t)+'"]',u=n;switch(a){case"audioworklet":case"paintworklet":case"serviceworker":case"sharedworker":case"worker":case"script":u=Za(t)}if(!ve.has(u)&&(t=et({rel:"modulepreload",href:t},e),ve.set(u,t),l.querySelector(n)===null)){switch(a){case"audioworklet":case"paintworklet":case"serviceworker":case"sharedworker":case"worker":case"script":if(l.querySelector(Fn(u)))return}a=l.createElement("link"),Ot(a,"link",t),Tt(a),l.head.appendChild(a)}}}function Pv(t,e,l){tl.S(t,e,l);var a=Qa;if(a&&t){var n=Sa(a).hoistableStyles,u=ka(t);e=e||"default";var i=n.get(u);if(!i){var o={loading:0,preload:null};if(i=a.querySelector($n(u)))o.loading=5;else{t=et({rel:"stylesheet",href:t,"data-precedence":e},l),(l=ve.get(u))&&Nf(t,l);var c=i=a.createElement("link");Tt(c),Ot(c,"link",t),c._p=new Promise(function(s,h){c.onload=s,c.onerror=h}),c.addEventListener("load",function(){o.loading|=1}),c.addEventListener("error",function(){o.loading|=2}),o.loading|=4,Yu(i,e,a)}i={type:"stylesheet",instance:i,count:1,state:o},n.set(u,i)}}}function tg(t,e){tl.X(t,e);var l=Qa;if(l&&t){var a=Sa(l).hoistableScripts,n=Za(t),u=a.get(n);u||(u=l.querySelector(Fn(n)),u||(t=et({src:t,async:!0},e),(e=ve.get(n))&&Cf(t,e),u=l.createElement("script"),Tt(u),Ot(u,"link",t),l.head.appendChild(u)),u={type:"script",instance:u,count:1,state:null},a.set(n,u))}}function eg(t,e){tl.M(t,e);var l=Qa;if(l&&t){var a=Sa(l).hoistableScripts,n=Za(t),u=a.get(n);u||(u=l.querySelector(Fn(n)),u||(t=et({src:t,async:!0,type:"module"},e),(e=ve.get(n))&&Cf(t,e),u=l.createElement("script"),Tt(u),Ot(u,"link",t),l.head.appendChild(u)),u={type:"script",instance:u,count:1,state:null},a.set(n,u))}}function Hs(t,e,l,a){var n=(n=pl.current)?mi(n):null;if(!n)throw Error(y(446));switch(t){case"meta":case"title":return null;case"style":return typeof l.precedence=="string"&&typeof l.href=="string"?(e=ka(l.href),l=Sa(n).hoistableStyles,a=l.get(e),a||(a={type:"style",instance:null,count:0,state:null},l.set(e,a)),a):{type:"void",instance:null,count:0,state:null};case"link":if(l.rel==="stylesheet"&&typeof l.href=="string"&&typeof l.precedence=="string"){t=ka(l.href);var u=Sa(n).hoistableStyles,i=u.get(t);if(i||(n=n.ownerDocument||n,i={type:"stylesheet",instance:null,count:0,state:{loading:0,preload:null}},u.set(t,i),(u=n.querySelector($n(t)))&&!u._p&&(i.instance=u,i.state.loading=5),ve.has(t)||(l={rel:"preload",as:"style",href:l.href,crossOrigin:l.crossOrigin,integrity:l.integrity,media:l.media,hrefLang:l.hrefLang,referrerPolicy:l.referrerPolicy},ve.set(t,l),u||lg(n,t,l,i.state))),e&&a===null)throw Error(y(528,""));return i}if(e&&a!==null)throw Error(y(529,""));return null;case"script":return e=l.async,l=l.src,typeof l=="string"&&e&&typeof e!="function"&&typeof e!="symbol"?(e=Za(l),l=Sa(n).hoistableScripts,a=l.get(e),a||(a={type:"script",instance:null,count:0,state:null},l.set(e,a)),a):{type:"void",instance:null,count:0,state:null};default:throw Error(y(444,t))}}function ka(t){return'href="'+se(t)+'"'}function $n(t){return'link[rel="stylesheet"]['+t+"]"}function zp(t){return et({},t,{"data-precedence":t.precedence,precedence:null})}function lg(t,e,l,a){t.querySelector('link[rel="preload"][as="style"]['+e+"]")?a.loading=1:(e=t.createElement("link"),a.preload=e,e.addEventListener("load",function(){return a.loading|=1}),e.addEventListener("error",function(){return a.loading|=2}),Ot(e,"link",l),Tt(e),t.head.appendChild(e))}function Za(t){return'[src="'+se(t)+'"]'}function Fn(t){return"script[async]"+t}function ks(t,e,l){if(e.count++,e.instance===null)switch(e.type){case"style":var a=t.querySelector('style[data-href~="'+se(l.href)+'"]');if(a)return e.instance=a,Tt(a),a;var n=et({},l,{"data-href":l.href,"data-precedence":l.precedence,href:null,precedence:null});return a=(t.ownerDocument||t).createElement("style"),Tt(a),Ot(a,"style",n),Yu(a,l.precedence,t),e.instance=a;case"stylesheet":n=ka(l.href);var u=t.querySelector($n(n));if(u)return e.state.loading|=4,e.instance=u,Tt(u),u;a=zp(l),(n=ve.get(n))&&Nf(a,n),u=(t.ownerDocument||t).createElement("link"),Tt(u);var i=u;return i._p=new Promise(function(o,c){i.onload=o,i.onerror=c}),Ot(u,"link",a),e.state.loading|=4,Yu(u,l.precedence,t),e.instance=u;case"script":return u=Za(l.src),(n=t.querySelector(Fn(u)))?(e.instance=n,Tt(n),n):(a=l,(n=ve.get(u))&&(a=et({},l),Cf(a,n)),t=t.ownerDocument||t,n=t.createElement("script"),Tt(n),Ot(n,"link",a),t.head.appendChild(n),e.instance=n);case"void":return null;default:throw Error(y(443,e.type))}else e.type==="stylesheet"&&!(e.state.loading&4)&&(a=e.instance,e.state.loading|=4,Yu(a,l.precedence,t));return e.instance}function Yu(t,e,l){for(var a=l.querySelectorAll('link[rel="stylesheet"][data-precedence],style[data-precedence]'),n=a.length?a[a.length-1]:null,u=n,i=0;i<a.length;i++){var o=a[i];if(o.dataset.precedence===e)u=o;else if(u!==n)break}u?u.parentNode.insertBefore(t,u.nextSibling):(e=l.nodeType===9?l.head:l,e.insertBefore(t,e.firstChild))}function Nf(t,e){t.crossOrigin==null&&(t.crossOrigin=e.crossOrigin),t.referrerPolicy==null&&(t.referrerPolicy=e.referrerPolicy),t.title==null&&(t.title=e.title)}function Cf(t,e){t.crossOrigin==null&&(t.crossOrigin=e.crossOrigin),t.referrerPolicy==null&&(t.referrerPolicy=e.referrerPolicy),t.integrity==null&&(t.integrity=e.integrity)}var Gu=null;function Bs(t,e,l){if(Gu===null){var a=new Map,n=Gu=new Map;n.set(l,a)}else n=Gu,a=n.get(l),a||(a=new Map,n.set(l,a));if(a.has(t))return a;for(a.set(t,null),l=l.getElementsByTagName(t),n=0;n<l.length;n++){var u=l[n];if(!(u[Qn]||u[Dt]||t==="link"&&u.getAttribute("rel")==="stylesheet")&&u.namespaceURI!=="http://www.w3.org/2000/svg"){var i=u.getAttribute(e)||"";i=t+i;var o=a.get(i);o?o.push(u):a.set(i,[u])}}return a}function Ys(t,e,l){t=t.ownerDocument||t,t.head.insertBefore(l,e==="title"?t.querySelector("head > title"):null)}function ag(t,e,l){if(l===1||e.itemProp!=null)return!1;switch(t){case"meta":case"title":return!0;case"style":if(typeof e.precedence!="string"||typeof e.href!="string"||e.href==="")break;return!0;case"link":if(typeof e.rel!="string"||typeof e.href!="string"||e.href===""||e.onLoad||e.onError)break;switch(e.rel){case"stylesheet":return t=e.disabled,typeof e.precedence=="string"&&t==null;default:return!0}case"script":if(e.async&&typeof e.async!="function"&&typeof e.async!="symbol"&&!e.onLoad&&!e.onError&&e.src&&typeof e.src=="string")return!0}return!1}function Ep(t){return!(t.type==="stylesheet"&&!(t.state.loading&3))}function ng(t,e,l,a){if(l.type==="stylesheet"&&(typeof a.media!="string"||matchMedia(a.media).matches!==!1)&&!(l.state.loading&4)){if(l.instance===null){var n=ka(a.href),u=e.querySelector($n(n));if(u){e=u._p,e!==null&&typeof e=="object"&&typeof e.then=="function"&&(t.count++,t=pi.bind(t),e.then(t,t)),l.state.loading|=4,l.instance=u,Tt(u);return}u=e.ownerDocument||e,a=zp(a),(n=ve.get(n))&&Nf(a,n),u=u.createElement("link"),Tt(u);var i=u;i._p=new Promise(function(o,c){i.onload=o,i.onerror=c}),Ot(u,"link",a),l.instance=u}t.stylesheets===null&&(t.stylesheets=new Map),t.stylesheets.set(l,e),(e=l.state.preload)&&!(l.state.loading&3)&&(t.count++,l=pi.bind(t),e.addEventListener("load",l),e.addEventListener("error",l))}}var Zo=0;function ug(t,e){return t.stylesheets&&t.count===0&&Xu(t,t.stylesheets),0<t.count||0<t.imgCount?function(l){var a=setTimeout(function(){if(t.stylesheets&&Xu(t,t.stylesheets),t.unsuspend){var u=t.unsuspend;t.unsuspend=null,u()}},6e4+e);0<t.imgBytes&&Zo===0&&(Zo=62500*Yv());var n=setTimeout(function(){if(t.waitingForImages=!1,t.count===0&&(t.stylesheets&&Xu(t,t.stylesheets),t.unsuspend)){var u=t.unsuspend;t.unsuspend=null,u()}},(t.imgBytes>Zo?50:800)+e);return t.unsuspend=l,function(){t.unsuspend=null,clearTimeout(a),clearTimeout(n)}}:null}function pi(){if(this.count--,this.count===0&&(this.imgCount===0||!this.waitingForImages)){if(this.stylesheets)Xu(this,this.stylesheets);else if(this.unsuspend){var t=this.unsuspend;this.unsuspend=null,t()}}}var hi=null;function Xu(t,e){t.stylesheets=null,t.unsuspend!==null&&(t.count++,hi=new Map,e.forEach(ig,t),hi=null,pi.call(t))}function ig(t,e){if(!(e.state.loading&4)){var l=hi.get(t);if(l)var a=l.get(null);else{l=new Map,hi.set(t,l);for(var n=t.querySelectorAll("link[data-precedence],style[data-precedence]"),u=0;u<n.length;u++){var i=n[u];(i.nodeName==="LINK"||i.getAttribute("media")!=="not all")&&(l.set(i.dataset.precedence,i),a=i)}a&&l.set(null,a)}n=e.instance,i=n.getAttribute("data-precedence"),u=l.get(i)||a,u===a&&l.set(null,n),l.set(i,n),this.count++,a=pi.bind(this),n.addEventListener("load",a),n.addEventListener("error",a),u?u.parentNode.insertBefore(n,u.nextSibling):(t=t.nodeType===9?t.head:t,t.insertBefore(n,t.firstChild)),e.state.loading|=4}}var kn={$$typeof:je,Provider:null,Consumer:null,_currentValue:Hl,_currentValue2:Hl,_threadCount:0};function og(t,e,l,a,n,u,i,o,c){this.tag=1,this.containerInfo=t,this.pingCache=this.current=this.pendingChildren=null,this.timeoutHandle=-1,this.callbackNode=this.next=this.pendingContext=this.context=this.cancelPendingCommit=null,this.callbackPriority=0,this.expirationTimes=go(-1),this.entangledLanes=this.shellSuspendCounter=this.errorRecoveryDisabledLanes=this.expiredLanes=this.warmLanes=this.pingedLanes=this.suspendedLanes=this.pendingLanes=0,this.entanglements=go(0),this.hiddenUpdates=go(null),this.identifierPrefix=a,this.onUncaughtError=n,this.onCaughtError=u,this.onRecoverableError=i,this.pooledCache=null,this.pooledCacheLanes=0,this.formState=c,this.incompleteTransitions=new Map}function Tp(t,e,l,a,n,u,i,o,c,s,h,v){return t=new og(t,e,l,i,c,s,h,v,o),e=1,u===!0&&(e|=24),u=Ft(3,null,null,e),t.current=u,u.stateNode=t,e=nf(),e.refCount++,t.pooledCache=e,e.refCount++,u.memoizedState={element:a,isDehydrated:l,cache:e},cf(u),t}function Ap(t){return t?(t=ga,t):ga}function Mp(t,e,l,a,n,u){n=Ap(n),a.context===null?a.context=n:a.pendingContext=n,a=vl(e),a.payload={element:l},u=u===void 0?null:u,u!==null&&(a.callback=u),l=gl(t,a,e),l!==null&&(Qt(l,t,e),gn(l,t,e))}function Gs(t,e){if(t=t.memoizedState,t!==null&&t.dehydrated!==null){var l=t.retryLane;t.retryLane=l!==0&&l<e?l:e}}function Rf(t,e){Gs(t,e),(t=t.alternate)&&Gs(t,e)}function Dp(t){if(t.tag===13||t.tag===31){var e=Wl(t,67108864);e!==null&&Qt(e,t,67108864),Rf(t,67108864)}}function Xs(t){if(t.tag===13||t.tag===31){var e=le();e=Qc(e);var l=Wl(t,e);l!==null&&Qt(l,t,e),Rf(t,e)}}var vi=!0;function cg(t,e,l,a){var n=C.T;C.T=null;var u=Z.p;try{Z.p=2,Uf(t,e,l,a)}finally{Z.p=u,C.T=n}}function fg(t,e,l,a){var n=C.T;C.T=null;var u=Z.p;try{Z.p=8,Uf(t,e,l,a)}finally{Z.p=u,C.T=n}}function Uf(t,e,l,a){if(vi){var n=Yc(a);if(n===null)jo(t,e,a,gi,l),Ls(t,a);else if(sg(n,t,e,l,a))a.stopPropagation();else if(Ls(t,a),e&4&&-1<rg.indexOf(t)){for(;n!==null;){var u=Ga(n);if(u!==null)switch(u.tag){case 3:if(u=u.stateNode,u.current.memoizedState.isDehydrated){var i=Cl(u.pendingLanes);if(i!==0){var o=u;for(o.pendingLanes|=2,o.entangledLanes|=2;i;){var c=1<<31-ee(i);o.entanglements[1]|=c,i&=~c}Oe(u),!(Q&6)&&(ii=Pt()+500,Wn(0,!1))}}break;case 31:case 13:o=Wl(u,2),o!==null&&Qt(o,u,2),wi(),Rf(u,2)}if(u=Yc(a),u===null&&jo(t,e,a,gi,l),u===n)break;n=u}n!==null&&a.stopPropagation()}else jo(t,e,a,null,l)}}function Yc(t){return t=Jc(t),Hf(t)}var gi=null;function Hf(t){if(gi=null,t=sa(t),t!==null){var e=Gn(t);if(e===null)t=null;else{var l=e.tag;if(l===13){if(t=Js(e),t!==null)return t;t=null}else if(l===31){if(t=Ws(e),t!==null)return t;t=null}else if(l===3){if(e.stateNode.current.memoizedState.isDehydrated)return e.tag===3?e.stateNode.containerInfo:null;t=null}else e!==t&&(t=null)}}return gi=t,null}function qp(t){switch(t){case"beforetoggle":case"cancel":case"click":case"close":case"contextmenu":case"copy":case"cut":case"auxclick":case"dblclick":case"dragend":case"dragstart":case"drop":case"focusin":case"focusout":case"input":case"invalid":case"keydown":case"keypress":case"keyup":case"mousedown":case"mouseup":case"paste":case"pause":case"play":case"pointercancel":case"pointerdown":case"pointerup":case"ratechange":case"reset":case"resize":case"seeked":case"submit":case"toggle":case"touchcancel":case"touchend":case"touchstart":case"volumechange":case"change":case"selectionchange":case"textInput":case"compositionstart":case"compositionend":case"compositionupdate":case"beforeblur":case"afterblur":case"beforeinput":case"blur":case"fullscreenchange":case"focus":case"hashchange":case"popstate":case"select":case"selectstart":return 2;case"drag":case"dragenter":case"dragexit":case"dragleave":case"dragover":case"mousemove":case"mouseout":case"mouseover":case"pointermove":case"pointerout":case"pointerover":case"scroll":case"touchmove":case"wheel":case"mouseenter":case"mouseleave":case"pointerenter":case"pointerleave":return 8;case"message":switch(F0()){case Ps:return 2;case td:return 8;case Vu:case I0:return 32;case ed:return 268435456;default:return 32}default:return 32}}var Gc=!1,_l=null,Sl=null,xl=null,Bn=new Map,Yn=new Map,cl=[],rg="mousedown mouseup touchcancel touchend touchstart auxclick dblclick pointercancel pointerdown pointerup dragend dragstart drop compositionend compositionstart keydown keypress keyup input textInput copy cut paste click change contextmenu reset".split(" ");function Ls(t,e){switch(t){case"focusin":case"focusout":_l=null;break;case"dragenter":case"dragleave":Sl=null;break;case"mouseover":case"mouseout":xl=null;break;case"pointerover":case"pointerout":Bn.delete(e.pointerId);break;case"gotpointercapture":case"lostpointercapture":Yn.delete(e.pointerId)}}function un(t,e,l,a,n,u){return t===null||t.nativeEvent!==u?(t={blockedOn:e,domEventName:l,eventSystemFlags:a,nativeEvent:u,targetContainers:[n]},e!==null&&(e=Ga(e),e!==null&&Dp(e)),t):(t.eventSystemFlags|=a,e=t.targetContainers,n!==null&&e.indexOf(n)===-1&&e.push(n),t)}function sg(t,e,l,a,n){switch(e){case"focusin":return _l=un(_l,t,e,l,a,n),!0;case"dragenter":return Sl=un(Sl,t,e,l,a,n),!0;case"mouseover":return xl=un(xl,t,e,l,a,n),!0;case"pointerover":var u=n.pointerId;return Bn.set(u,un(Bn.get(u)||null,t,e,l,a,n)),!0;case"gotpointercapture":return u=n.pointerId,Yn.set(u,un(Yn.get(u)||null,t,e,l,a,n)),!0}return!1}function wp(t){var e=sa(t.target);if(e!==null){var l=Gn(e);if(l!==null){if(e=l.tag,e===13){if(e=Js(l),e!==null){t.blockedOn=e,Tr(t.priority,function(){Xs(l)});return}}else if(e===31){if(e=Ws(l),e!==null){t.blockedOn=e,Tr(t.priority,function(){Xs(l)});return}}else if(e===3&&l.stateNode.current.memoizedState.isDehydrated){t.blockedOn=l.tag===3?l.stateNode.containerInfo:null;return}}}t.blockedOn=null}function Lu(t){if(t.blockedOn!==null)return!1;for(var e=t.targetContainers;0<e.length;){var l=Yc(t.nativeEvent);if(l===null){l=t.nativeEvent;var a=new l.constructor(l.type,l);nc=a,l.target.dispatchEvent(a),nc=null}else return e=Ga(l),e!==null&&Dp(e),t.blockedOn=l,!1;e.shift()}return!0}function js(t,e,l){Lu(t)&&l.delete(e)}function dg(){Gc=!1,_l!==null&&Lu(_l)&&(_l=null),Sl!==null&&Lu(Sl)&&(Sl=null),xl!==null&&Lu(xl)&&(xl=null),Bn.forEach(js),Yn.forEach(js)}function Au(t,e){t.blockedOn===e&&(t.blockedOn=null,Gc||(Gc=!0,St.unstable_scheduleCallback(St.unstable_NormalPriority,dg)))}var Mu=null;function Qs(t){Mu!==t&&(Mu=t,St.unstable_scheduleCallback(St.unstable_NormalPriority,function(){Mu===t&&(Mu=null);for(var e=0;e<t.length;e+=3){var l=t[e],a=t[e+1],n=t[e+2];if(typeof a!="function"){if(Hf(a||l)===null)continue;break}var u=Ga(l);u!==null&&(t.splice(e,3),e-=3,_c(u,{pending:!0,data:n,method:l.method,action:a},a,n))}}))}function Ba(t){function e(c){return Au(c,t)}_l!==null&&Au(_l,t),Sl!==null&&Au(Sl,t),xl!==null&&Au(xl,t),Bn.forEach(e),Yn.forEach(e);for(var l=0;l<cl.length;l++){var a=cl[l];a.blockedOn===t&&(a.blockedOn=null)}for(;0<cl.length&&(l=cl[0],l.blockedOn===null);)wp(l),l.blockedOn===null&&cl.shift();if(l=(t.ownerDocument||t).$$reactFormReplay,l!=null)for(a=0;a<l.length;a+=3){var n=l[a],u=l[a+1],i=n[Zt]||null;if(typeof u=="function")i||Qs(l);else if(i){var o=null;if(u&&u.hasAttribute("formAction")){if(n=u,i=u[Zt]||null)o=i.formAction;else if(Hf(n)!==null)continue}else o=i.action;typeof o=="function"?l[a+1]=o:(l.splice(a,3),a-=3),Qs(l)}}}function Op(){function t(u){u.canIntercept&&u.info==="react-transition"&&u.intercept({handler:function(){return new Promise(function(i){return n=i})},focusReset:"manual",scroll:"manual"})}function e(){n!==null&&(n(),n=null),a||setTimeout(l,20)}function l(){if(!a&&!navigation.transition){var u=navigation.currentEntry;u&&u.url!=null&&navigation.navigate(u.url,{state:u.getState(),info:"react-transition",history:"replace"})}}if(typeof navigation=="object"){var a=!1,n=null;return navigation.addEventListener("navigate",t),navigation.addEventListener("navigatesuccess",e),navigation.addEventListener("navigateerror",e),setTimeout(l,100),function(){a=!0,navigation.removeEventListener("navigate",t),navigation.removeEventListener("navigatesuccess",e),navigation.removeEventListener("navigateerror",e),n!==null&&(n(),n=null)}}}function kf(t){this._internalRoot=t}Ci.prototype.render=kf.prototype.render=function(t){var e=this._internalRoot;if(e===null)throw Error(y(409));var l=e.current,a=le();Mp(l,a,t,e,null,null)};Ci.prototype.unmount=kf.prototype.unmount=function(){var t=this._internalRoot;if(t!==null){this._internalRoot=null;var e=t.containerInfo;Mp(t.current,2,null,t,null,null),wi(),e[Ya]=null}};function Ci(t){this._internalRoot=t}Ci.prototype.unstable_scheduleHydration=function(t){if(t){var e=id();t={blockedOn:null,target:t,priority:e};for(var l=0;l<cl.length&&e!==0&&e<cl[l].priority;l++);cl.splice(l,0,t),l===0&&wp(t)}};var Zs=Vs.version;if(Zs!=="19.2.5")throw Error(y(527,Zs,"19.2.5"));Z.findDOMNode=function(t){var e=t._reactInternals;if(e===void 0)throw typeof t.render=="function"?Error(y(188)):(t=Object.keys(t).join(","),Error(y(268,t)));return t=Q0(e),t=t!==null?$s(t):null,t=t===null?null:t.stateNode,t};var mg={bundleType:0,version:"19.2.5",rendererPackageName:"react-dom",currentDispatcherRef:C,reconcilerVersion:"19.2.5"};if(typeof __REACT_DEVTOOLS_GLOBAL_HOOK__<"u"&&(on=__REACT_DEVTOOLS_GLOBAL_HOOK__,!on.isDisabled&&on.supportsFiber))try{Xn=on.inject(mg),te=on}catch{}var on;Ri.createRoot=function(t,e){if(!Ks(t))throw Error(y(299));var l=!1,a="",n=xm,u=zm,i=Em;return e!=null&&(e.unstable_strictMode===!0&&(l=!0),e.identifierPrefix!==void 0&&(a=e.identifierPrefix),e.onUncaughtError!==void 0&&(n=e.onUncaughtError),e.onCaughtError!==void 0&&(u=e.onCaughtError),e.onRecoverableError!==void 0&&(i=e.onRecoverableError)),e=Tp(t,1,!1,null,null,l,a,null,n,u,i,Op),t[Ya]=e.current,Of(t),new kf(e)};Ri.hydrateRoot=function(t,e,l){if(!Ks(t))throw Error(y(299));var a=!1,n="",u=xm,i=zm,o=Em,c=null;return l!=null&&(l.unstable_strictMode===!0&&(a=!0),l.identifierPrefix!==void 0&&(n=l.identifierPrefix),l.onUncaughtError!==void 0&&(u=l.onUncaughtError),l.onCaughtError!==void 0&&(i=l.onCaughtError),l.onRecoverableError!==void 0&&(o=l.onRecoverableError),l.formState!==void 0&&(c=l.formState)),e=Tp(t,1,!0,e,l??null,a,n,c,u,i,o,Op),e.context=Ap(null),l=e.current,a=le(),a=Qc(a),n=vl(a),n.callback=null,gl(l,n,a),l=a,e.current.lanes=l,jn(e,l),Oe(e),t[Ya]=e.current,Of(t),new Ci(e)};Ri.version="19.2.5"});var Up=Ee((ty,Rp)=>{"use strict";function Cp(){if(!(typeof __REACT_DEVTOOLS_GLOBAL_HOOK__>"u"||typeof __REACT_DEVTOOLS_GLOBAL_HOOK__.checkDCE!="function"))try{__REACT_DEVTOOLS_GLOBAL_HOOK__.checkDCE(Cp)}catch(t){console.error(t)}}Cp(),Rp.exports=Np()});var Zp=Ee(Gi=>{"use strict";var yg=Symbol.for("react.transitional.element"),bg=Symbol.for("react.fragment");function Qp(t,e,l){var a=null;if(l!==void 0&&(a=""+l),e.key!==void 0&&(a=""+e.key),"key"in e){l={};for(var n in e)n!=="key"&&(l[n]=e[n])}else l=e;return e=l.ref,{$$typeof:yg,type:t,key:a,ref:e!==void 0?e:null,props:l}}Gi.Fragment=bg;Gi.jsx=Qp;Gi.jsxs=Qp});var xt=Ee((fy,Vp)=>{"use strict";Vp.exports=Zp()});var r0=j(Up(),1);var Ka=j(Yt(),1),i0=j(so(),1);var k=j(Yt(),1);function Hp(t,e){let l=t-16,a=e-2*8;if(l<=0||a<=0)return{cols:0,rows:0,maxW:0,maxH:0,cellSize:0};let n=Math.max(1,Math.round(l/144)),u=Math.max(1,Math.round(a/144)),i=l/(n*6),o=a/(u*6),c=Math.min(i,o);return{cols:n*6,rows:u*6,maxW:n,maxH:u,cellSize:c}}function Ht(t,e,l){return Math.max(e,Math.min(l,t))}function ge(t){return t*6}function Ui(t,e,l,a){let n=6*e,u=Math.floor(t.x/n),i=Math.floor(t.y/n),o=Math.ceil((t.x+t.w)/n),c=Math.ceil((t.y+t.h)/n),s=Ht(Math.max(1,o-u),1,l),h=Ht(Math.max(1,c-i),1,a),v=t.x+t.w/2,d=t.y+t.h/2,p=v-s*n/2,b=d-h*n/2,S=Ht(Math.round(p/n),0,l-s),T=Ht(Math.round(b/n),0,a-h);return{col:S*6,row:T*6,w:s,h}}var Va=j(Yt(),1);function kp({containerRef:t,enabled:e,onCommit:l,onCancel:a}){let[n,u]=(0,Va.useState)(null),i=(0,Va.useRef)(null);return(0,Va.useEffect)(()=>{let o=t.current;if(!o||!e)return;let c=d=>{let p=o.getBoundingClientRect();return{x:Math.max(0,d.clientX-p.left),y:Math.max(0,d.clientY-p.top)}},s=d=>{if(d.target!==o||d.button!==0)return;let{x:p,y:b}=c(d);i.current={x:p,y:b},u({x:p,y:b,w:0,h:0}),d.preventDefault()},h=d=>{let p=i.current;if(!p)return;let{x:b,y:S}=c(d),T=Math.min(p.x,b),r=Math.min(p.y,S),f=Math.max(p.x,b),m=Math.max(p.y,S);u({x:T,y:r,w:f-T,h:m-r})},v=()=>{let d=i.current;i.current=null,d&&u(p=>(p&&p.w>4&&p.h>4?l(p):a?.(),null))};return o.addEventListener("mousedown",s),window.addEventListener("mousemove",h),window.addEventListener("mouseup",v),()=>{o.removeEventListener("mousedown",s),window.removeEventListener("mousemove",h),window.removeEventListener("mouseup",v)}},[t,e,l,a]),n}function Bp(t){if(!t||typeof t!="object")return!1;let e=t.type;return typeof e=="string"&&e.startsWith("tonk:")}var In="data-square-id";var Yp=new Map;async function Hi(t,e,l){let a=`${t}::${e}::${l}`,n=Yp.get(a);if(n)return n;let u=`/api/repository/${encodeURIComponent(t)}/branch/${encodeURIComponent(e)}/resolve/${encodeURIComponent(l)}`,i=await fetch(u);if(!i.ok)throw new Error(i.status===404?`No entity bookmarked as "${l}" on branch "${e}"`:`resolve failed (${i.status})`);let o=await i.json();return Yp.set(a,o.entity),o.entity}var ki=j(Yt(),1),Ne=(0,ki.createContext)(""),Bi=(0,ki.createContext)(""),Yi=(0,ki.createContext)("canvas");var Re=j(Yt(),1);var yt=j(Yt(),1);var Gp=new Map,pg=5e3;async function Xp(t,e,l){let a=`/api/repository/${encodeURIComponent(t)}/branch/${encodeURIComponent(e)}/claim/select?the=${encodeURIComponent(l)}`,n=await fetch(a);if(!n.ok)throw new Error(`claim select '${l}' failed (${n.status})`);return(await n.json()).claims??[]}async function Lp(t,e="main"){let l=`${t}::${e}`,a=Date.now(),n=Gp.get(l);if(n&&a-n.time<pg)return n.artifacts;let[u,i]=await Promise.all([Xp(t,e,"dialog.meta/name"),Xp(t,e,"text/html")]),o=new Set(i.map(h=>h.of)),c=new Set,s=[];for(let h of u)o.has(h.of)&&(c.has(h.of)||typeof h.is=="string"&&(c.add(h.of),s.push({name:h.is,entity:h.of})));return s.sort((h,v)=>h.name.localeCompare(v.name)),Gp.set(l,{time:a,artifacts:s}),s}var hg=`<!DOCTYPE html>
<html>
<head>
<meta charset="utf-8">
<style>
@import url('https://fonts.googleapis.com/css2?family=Inconsolata:wght@400;500&display=swap');
* { box-sizing: border-box; margin: 0; padding: 0; }
html, body { height: 100%; background: #fff; }
textarea {
  display: block;
  width: 100%;
  height: 100%;
  font-family: 'Inconsolata', Menlo, 'Courier New', monospace;
  font-size: 14px;
  line-height: 1.6;
  border: none;
  outline: none;
  padding: 16px;
  resize: none;
  background: #fff;
  color: #1a1a1a;
  caret-color: #1a1a1a;
}
</style>
</head>
<body>
<textarea spellcheck="false" autocomplete="off" autocorrect="off" autocapitalize="off"></textarea>
</body>
</html>`,vg=`<!DOCTYPE html>
<html>
<head>
<meta charset="utf-8">
<style>
* { box-sizing: border-box; margin: 0; padding: 0; }
html, body { height: 100%; font-family: system-ui, sans-serif; font-size: 13px; background: #fff; color: #111; }
.wrap { display: flex; flex-direction: column; height: 100%; }
.toolbar { display: flex; gap: 6px; padding: 8px 10px; border-bottom: 1px solid #e5e5e5; background: #f9f9f9; flex-shrink: 0; }
button { padding: 4px 10px; border: 1px solid #d0d0d0; border-radius: 5px; cursor: pointer; background: #fff; font-size: 12px; color: #333; }
button:hover { background: #f0f0f0; }
.scroll { flex: 1; overflow: auto; }
table { border-collapse: collapse; min-width: 100%; }
td { border: 1px solid #e0e0e0; min-width: 100px; padding: 0; }
td div[contenteditable] { outline: none; padding: 6px 8px; min-height: 30px; white-space: pre-wrap; }
td div[contenteditable]:focus { background: #f0f7ff; }
</style>
</head>
<body>
<div class="wrap">
  <div class="toolbar">
    <button onclick="addRow()">+ Row</button>
    <button onclick="addCol()">+ Column</button>
  </div>
  <div class="scroll">
    <table id="tbl">
      <tr><td><div contenteditable="true"></div></td><td><div contenteditable="true"></div></td><td><div contenteditable="true"></div></td></tr>
      <tr><td><div contenteditable="true"></div></td><td><div contenteditable="true"></div></td><td><div contenteditable="true"></div></td></tr>
      <tr><td><div contenteditable="true"></div></td><td><div contenteditable="true"></div></td><td><div contenteditable="true"></div></td></tr>
    </table>
  </div>
</div>
<script>
function cell() {
  var td = document.createElement('td');
  var d = document.createElement('div');
  d.contentEditable = 'true';
  td.appendChild(d);
  return td;
}
function addRow() {
  var t = document.getElementById('tbl');
  var cols = t.rows[0] ? t.rows[0].cells.length : 3;
  var tr = document.createElement('tr');
  for (var i = 0; i < cols; i++) tr.appendChild(cell());
  t.appendChild(tr);
}
function addCol() {
  var t = document.getElementById('tbl');
  for (var i = 0; i < t.rows.length; i++) t.rows[i].appendChild(cell());
}
<\/script>
</body>
</html>`,gg=`<!DOCTYPE html>
<html>
<head>
<meta charset="utf-8">
<style>
* { box-sizing: border-box; margin: 0; padding: 0; }
html, body { height: 100%; display: flex; align-items: center; justify-content: center; font-family: system-ui, sans-serif; background: #fff; }
h1 { font-size: 2rem; font-weight: 600; color: #111; letter-spacing: -0.02em; }
</style>
</head>
<body>
<h1>Hello, World!</h1>
</body>
</html>`,Pn=[{entity:"builtin:editor",name:"Editor",description:"Plaintext editor",html:hg},{entity:"builtin:table",name:"Table",description:"Editable table",html:vg},{entity:"builtin:site",name:"Site",description:"HTML display",html:gg}];function jp(t){let e=Pn.find(l=>l.entity===t);return e?`data:text/html;charset=utf-8,${encodeURIComponent(e.html)}`:""}function tu(t){return t.startsWith("builtin:")}var mt=j(xt(),1),Kp="main",_g=8;function Jp({initialEntity:t,onPick:e,onClose:l}){let a=(0,yt.useContext)(Ne),[n,u]=(0,yt.useState)(t??""),[i,o]=(0,yt.useState)(null),[c,s]=(0,yt.useState)(!1),[h,v]=(0,yt.useState)([]),[d,p]=(0,yt.useState)(0),b=(0,yt.useRef)(null),S=(0,yt.useRef)(null);(0,yt.useEffect)(()=>{b.current?.focus(),b.current?.select()},[]),(0,yt.useEffect)(()=>{if(!a)return;let _=!1;return Lp(a,Kp).then(q=>{_||v(q)}).catch(()=>{}),()=>{_=!0}},[a]);let T=n.trim(),r=T.startsWith("did:"),f=(0,yt.useMemo)(()=>{if(r)return[];let _=T.toLowerCase();return(_?h.filter(nt=>nt.name.toLowerCase().includes(_)||nt.entity.toLowerCase().includes(_)):h).slice(0,_g)},[T,r,h]);(0,yt.useEffect)(()=>{d>=f.length&&p(0)},[d,f.length]);let m=async()=>{if(c)return;let _=f[d];if(_&&!r){e({entity:_.entity,name:_.name});return}if(T){if(o(null),r){e({entity:T});return}s(!0);try{let q=await Hi(a,Kp,T);e({entity:q,name:T})}catch(q){o(q instanceof Error?q.message:String(q))}finally{s(!1)}}},g=_=>{e({entity:_.entity,name:_.name})};(0,yt.useEffect)(()=>{let _=q=>{if(q.key==="Escape"){q.stopPropagation(),l();return}if(q.key==="Enter"){q.preventDefault(),m();return}if(q.key==="ArrowDown"&&f.length){q.preventDefault(),p(nt=>(nt+1)%f.length);return}if(q.key==="ArrowUp"&&f.length){q.preventDefault(),p(nt=>(nt-1+f.length)%f.length);return}};return window.addEventListener("keydown",_,!0),()=>window.removeEventListener("keydown",_,!0)},[n,d,f,c,a,e,l]),(0,yt.useEffect)(()=>{let _=q=>{S.current?.contains(q.target)||l()};return document.addEventListener("mousedown",_),()=>document.removeEventListener("mousedown",_)},[l]);let E=r||f.length===0,w=!T&&!r,[x,M]=(0,yt.useState)(!1),O=_=>{_.preventDefault(),navigator.clipboard.writeText(window.location.href).then(()=>{M(!0),setTimeout(()=>{M(!1)},1800)})};return(0,mt.jsxs)("div",{ref:S,className:"picker",onMouseDown:_=>_.stopPropagation(),children:[(0,mt.jsx)("input",{ref:b,className:"picker__input",value:n,onChange:_=>{u(_.target.value),i&&o(null)},placeholder:"search artifacts or paste did:key:\u2026",disabled:c}),w&&(0,mt.jsxs)(mt.Fragment,{children:[(0,mt.jsx)("ul",{className:"picker__list picker__list--builtins",role:"listbox",children:Pn.map(_=>(0,mt.jsxs)("li",{role:"option","aria-selected":!1,className:"picker__item picker__item--builtin",onMouseDown:q=>{q.preventDefault(),e({entity:_.entity,name:_.name})},children:[(0,mt.jsx)("span",{className:"picker__item-name",children:_.name}),(0,mt.jsx)("span",{className:"picker__item-entity",children:_.description})]},_.entity))}),(0,mt.jsxs)("button",{className:"picker__agent-link",onMouseDown:O,children:[x?"Copied!":"make your own",(0,mt.jsx)("span",{className:"picker__agent-link-sub",children:"paste into your agent to build"})]})]}),i&&(0,mt.jsx)("div",{className:"picker__error",children:i}),!r&&f.length>0&&(0,mt.jsx)("ul",{className:"picker__list",role:"listbox",children:f.map((_,q)=>(0,mt.jsxs)("li",{role:"option","aria-selected":q===d,className:`picker__item${q===d?" picker__item--highlighted":""}`,onMouseEnter:()=>p(q),onMouseDown:nt=>{nt.preventDefault(),g(_)},children:[(0,mt.jsx)("span",{className:"picker__item-name",children:_.name}),(0,mt.jsx)("span",{className:"picker__item-entity",children:_.entity})]},_.entity))}),!r&&T&&f.length===0&&h.length>0&&(0,mt.jsxs)("div",{className:"picker__empty",children:['No artifact matches "',T,'"']}),E&&(0,mt.jsx)("div",{className:"picker__actions",children:(0,mt.jsx)("button",{className:"picker__action picker__action--primary",onClick:()=>void m(),type:"button",disabled:!T||c,children:c?"Resolving\u2026":"Load"})})]})}var Wp=["#4465e9","#099268","#e16919","#ae3ec9","#f1ac4b","#4ba1f1","#4cb05e","#e03131","#e085f4","#f87777","#9fa8b2"],Bf=new Map;function Sg(t){let e=Bf.get(t);if(e)return e;let l=Wp[Bf.size%Wp.length];return Bf.set(t,l),l}function Xi(t){let e=t.id??t.entity??t.name;if(!e)return;let l=Sg(e);return{full:l,soft:l+"99"}}var Li=j(Yt(),1);function $p({cellSize:t,startCol:e,startRow:l,w:a,h:n,maxW:u,maxH:i,onCommit:o}){let[c,s]=(0,Li.useState)(null),h=(0,Li.useCallback)(v=>{v.preventDefault();let d=6*t,p=v.clientX,b=v.clientY,S=Math.round(e/6),T=Math.round(l/6);s({col:e,row:l});let r=m=>{let g=m.clientX-p,E=m.clientY-b,w=Math.round(g/d),x=Math.round(E/d),M=Ht(S+w,0,u-a),O=Ht(T+x,0,i-n);s({col:M*6,row:O*6})},f=()=>{window.removeEventListener("pointermove",r),window.removeEventListener("pointerup",f),s(m=>(m&&(m.col!==e||m.row!==l)&&o({col:m.col,row:m.row}),null))};window.addEventListener("pointermove",r),window.addEventListener("pointerup",f)},[t,e,l,a,n,u,i,o]);return{ghost:c,beginDrag:h}}var ji=j(Yt(),1);function Fp({index:t,onReorder:e}){let[l,a]=(0,ji.useState)(!1),n=(0,ji.useCallback)(u=>{u.preventDefault(),u.stopPropagation();let o=u.currentTarget.closest(".square"),c=o?.parentElement;if(!o||!c)return;let s=Array.from(c.querySelectorAll(":scope > .square")),h=s.map(S=>S.getBoundingClientRect());a(!0);let v=t,d=(S,T)=>{let r=0,f=1/0;for(let x=0;x<h.length;x++){let M=h[x],O=M.left+M.width/2,_=M.top+M.height/2,q=S-O,nt=T-_,Kt=q*q+nt*nt;Kt<f&&(f=Kt,r=x)}let m=h[r],g=m.top+m.height/2,E=m.left+m.width/2,w=r;return(T>g+1||Math.abs(T-g)<m.height/2&&S>E)&&(w=r+1),w>t&&(w-=1),w<0&&(w=0),w>s.length-1&&(w=s.length-1),w},p=S=>{v=d(S.clientX,S.clientY)},b=()=>{window.removeEventListener("pointermove",p),window.removeEventListener("pointerup",b),a(!1),v!==t&&e(t,v)};window.addEventListener("pointermove",p),window.addEventListener("pointerup",b)},[t,e]);return{dragging:l,beginReorder:n}}var Qi=j(Yt(),1);var xg={tl:{x:-1,y:-1},tr:{x:1,y:-1},bl:{x:-1,y:1},br:{x:1,y:1},t:{x:0,y:-1},r:{x:1,y:0},b:{x:0,y:1},l:{x:-1,y:0}};function zg(t,e,l,a,n,u,i){let o=ge(a)-ge(u),c=ge(n)-ge(i),s=t.includes("l"),h=t.includes("t");return{col:e+(s?o:0),row:l+(h?c:0)}}function Ip({cellSize:t,startW:e,startH:l,startCol:a,startRow:n,maxW:u,maxH:i,onCommit:o}){let[c,s]=(0,Qi.useState)(null),h=(0,Qi.useCallback)(v=>d=>{d.stopPropagation(),d.preventDefault();let p=6*t,b=xg[v],S=d.clientX,T=d.clientY;s({w:e,h:l,col:a,row:n});let r=m=>{let g=(m.clientX-S)*b.x,E=(m.clientY-T)*b.y,w=e*p,x=l*p,M=Math.max(p,w+g),O=Math.max(p,x+E),_=Ht(Math.round(M/p),1,u),q=Ht(Math.round(O/p),1,i),{col:nt,row:Kt}=zg(v,a,n,e,l,_,q);s({w:_,h:q,col:nt,row:Kt})},f=()=>{window.removeEventListener("pointermove",r),window.removeEventListener("pointerup",f),s(m=>(m&&(m.w!==e||m.h!==l)&&o({w:m.w,h:m.h,col:m.col,row:m.row}),null))};window.addEventListener("pointermove",r),window.addEventListener("pointerup",f)},[t,e,l,a,n,u,i,o]);return{ghost:c,beginResize:h}}var Fl=j(Yt(),1);var Ce=j(xt(),1);function Pp(t){if(!t.length)return`claims: []
`;let e=["claims:"];for(let l of t)if(e.push(`  - the: ${l.the}`),e.push(`    of: ${l.of}`),l.is.includes(`
`)){e.push("    is: |");for(let a of l.is.split(`
`))e.push(`      ${a}`)}else e.push(`    is: ${JSON.stringify(l.is)}`);return e.join(`
`)}function Eg(t){let e=Pn.find(l=>l.entity===t);return e?[{the:"dialog.meta/name",of:t,is:e.name},{the:"text/html",of:t,is:e.html}]:[]}function t0({entity:t,branch:e,onClose:l}){let a=(0,Fl.useContext)(Ne),[n,u]=(0,Fl.useState)(null),[i,o]=(0,Fl.useState)(null);return(0,Fl.useEffect)(()=>{if(tu(t)){u(Pp(Eg(t)));return}if(!a)return;let c=`/api/repository/${encodeURIComponent(a)}/branch/${encodeURIComponent(e)}/claim/select?of=${encodeURIComponent(t)}`;fetch(c).then(s=>{if(!s.ok)throw new Error(`HTTP ${s.status}`);return s.json()}).then(s=>u(Pp(s.claims??[]))).catch(s=>o(s instanceof Error?s.message:String(s)))},[t,e,a]),(0,Ce.jsxs)("div",{className:"source-panel",onMouseDown:c=>c.stopPropagation(),children:[(0,Ce.jsxs)("div",{className:"source-panel__bar",children:[(0,Ce.jsx)("span",{className:"source-panel__title",children:"Source"}),(0,Ce.jsx)("button",{className:"source-panel__close",onClick:l,"aria-label":"close source",children:"\u2715"})]}),(0,Ce.jsxs)("div",{className:"source-panel__body",children:[i&&(0,Ce.jsx)("div",{className:"source-panel__error",children:i}),!i&&n===null&&(0,Ce.jsx)("div",{className:"source-panel__loading",children:"Loading\u2026"}),n!==null&&(0,Ce.jsx)("pre",{className:"source-panel__pre",children:n})]})]})}var D=j(xt(),1),Tg=["tl","tr","bl","br"],Ag=["t","r","b","l"];function Mg(t,e,l,a){return`/api/repository/${encodeURIComponent(t)}/branch/${encodeURIComponent(a)}/host/${encodeURIComponent(e)}/${l}`}function Dg(){return(0,D.jsxs)("svg",{viewBox:"0 0 14 14",fill:"none",stroke:"currentColor",strokeWidth:"1.5",strokeLinecap:"round",children:[(0,D.jsx)("path",{d:"M5.5 8.5a2.5 2.5 0 0 0 3.5 0l1-1a2.5 2.5 0 0 0-3.5-3.5l-.5.5"}),(0,D.jsx)("path",{d:"M8.5 5.5a2.5 2.5 0 0 0-3.5 0l-1 1a2.5 2.5 0 0 0 3.5 3.5l.5-.5"})]})}function qg(){return(0,D.jsxs)("svg",{viewBox:"0 0 14 14",fill:"none",stroke:"currentColor",strokeWidth:"1.5",strokeLinecap:"round",strokeLinejoin:"round",children:[(0,D.jsx)("path",{d:"M11.5 7a4.5 4.5 0 1 1-1.1-2.9"}),(0,D.jsx)("path",{d:"M10.5 1.5v3h-3"})]})}function wg(){return(0,D.jsxs)("svg",{viewBox:"0 0 14 14",fill:"none",stroke:"currentColor",strokeWidth:"1.5",strokeLinecap:"round",strokeLinejoin:"round",children:[(0,D.jsx)("path",{d:"M4 4.5 1 7l3 2.5"}),(0,D.jsx)("path",{d:"M10 4.5 13 7l-3 2.5"}),(0,D.jsx)("path",{d:"M8.5 2l-3 10"})]})}function Og({half:t}){return(0,D.jsx)("svg",{viewBox:"0 0 14 14",fill:"none",stroke:"currentColor",strokeWidth:"1.5",strokeLinecap:"round",strokeLinejoin:"round",children:t?(0,D.jsxs)(D.Fragment,{children:[(0,D.jsx)("rect",{x:"1.5",y:"4",width:"4.5",height:"6",rx:"1"}),(0,D.jsx)("rect",{x:"8",y:"4",width:"4.5",height:"6",rx:"1"})]}):(0,D.jsx)("rect",{x:"1.5",y:"4",width:"11",height:"6",rx:"1"})})}function Ng({locked:t}){return(0,D.jsxs)("svg",{viewBox:"0 0 14 14",fill:"none",stroke:"currentColor",strokeWidth:"1.5",strokeLinecap:"round",children:[(0,D.jsx)("rect",{x:"2.5",y:"6.5",width:"9",height:"6.5",rx:"1.5"}),t?(0,D.jsx)("path",{d:"M4.5 6.5V4.5a2.5 2.5 0 0 1 5 0v2"}):(0,D.jsx)("path",{d:"M4.5 6.5V4.5a2.5 2.5 0 0 1 5 0"})]})}function Zi({tile:t,x:e,y:l,w:a,h:n,cellSize:u,maxW:i,maxH:o,selected:c,pickerOpen:s,fullscreen:h,onSelect:v,onMove:d,onResize:p,onClose:b,onMinimize:S,onFullscreen:T,onOpenPicker:r,onPick:f,onClosePicker:m,mode:g="canvas",index:E=0,onReorder:w,onToggleDocWidth:x}){let M=(0,Re.useContext)(Ne),O=(0,Re.useContext)(Bi),_=(0,Re.useRef)(null),[q,nt]=(0,Re.useState)(!1),[Kt,xe]=(0,Re.useState)(!1),[lu,au]=(0,Re.useState)(!1),Il=(0,Re.useRef)(null),{ghost:Ja,beginResize:nu}=Ip({cellSize:u,startW:t.w,startH:t.h,startCol:t.col,startRow:t.row,maxW:i,maxH:o,onCommit:N=>p(t.id,N)}),{ghost:ql,beginDrag:Pl}=$p({cellSize:u,startCol:t.col,startRow:t.row,w:t.w,h:t.h,maxW:i,maxH:o,onCommit:N=>d(t.id,N)}),{dragging:Wa,beginReorder:wl}=Fp({index:E,onReorder:(N,Jt)=>w?.(N,Jt)}),ze=6*u,ue=Ja?{...Ja}:ql?{col:ql.col,row:ql.row,w:t.w,h:t.h}:null,Ji=!!ue,z=Xi(t),A=t.branch??"main",R=t.name??t.entity,B=!!t.entity&&(tu(t.entity)||!!M&&!!O),ot=B?tu(t.entity)?jp(t.entity):Mg(M,O,t.entity,A):null,ut=N=>{N.stopPropagation();let Jt=ot??window.location.href;navigator.clipboard.writeText(Jt),au(!0),Il.current&&clearTimeout(Il.current),Il.current=setTimeout(()=>au(!1),2500)},it=N=>{N.stopPropagation(),_.current&&(_.current.src=_.current.src)},zt=N=>{N.stopPropagation(),t.entity&&xe(Jt=>!Jt)},rt=N=>{N.stopPropagation(),nt(Jt=>!Jt)},Bt=g==="doc",Wi=t.docWidth??"full",Lf=!h&&!q&&!Bt,jf=!h&&!q&&!Bt,Qf=!h&&!q&&Bt,d0=N=>{N.stopPropagation(),x?.(t.id)},m0=["square",c?"square--selected":"",Ji?"square--active":"",Bt?"square--doc":"",Bt?`square--doc-${Wi}`:"",Wa?"square--reorder-active":""].filter(Boolean).join(" "),Zf=h?void 0:`tp-sq-${t.id}`,p0=Bt?{viewTransitionName:Zf}:{transform:`translate(${e}px, ${l}px)`,width:a,height:n,viewTransitionName:Zf};return(0,D.jsxs)(D.Fragment,{children:[(0,D.jsxs)("div",{className:m0,style:p0,onMouseDown:N=>{N.stopPropagation(),v(t.id)},[In]:t.id,children:[!h&&(0,D.jsxs)("div",{className:"tile-pill",onMouseDown:N=>N.stopPropagation(),onPointerDown:N=>N.stopPropagation(),children:[(0,D.jsx)("button",{className:"tile-pill__btn",onClick:ut,title:"Copy link",children:(0,D.jsx)(Dg,{})}),(0,D.jsx)("button",{className:"tile-pill__btn",onClick:it,title:"Refresh",children:(0,D.jsx)(qg,{})}),(0,D.jsx)("button",{className:"tile-pill__btn",onClick:zt,title:"View source",children:(0,D.jsx)(wg,{})}),(0,D.jsx)("button",{className:`tile-pill__btn${q?" tile-pill__btn--active":""}`,onClick:rt,title:q?"Unlock":"Lock",children:(0,D.jsx)(Ng,{locked:q})}),Bt&&(0,D.jsx)("button",{className:"tile-pill__btn",onClick:d0,title:Wi==="full"?"Make half-width":"Make full-width",children:(0,D.jsx)(Og,{half:Wi==="full"})})]}),!h&&(0,D.jsx)("div",{className:"square__drag-dots",onPointerDown:Lf?Pl:Qf?wl:void 0,children:Array.from({length:6},(N,Jt)=>(0,D.jsx)("span",{className:"square__drag-dot"},Jt))}),(0,D.jsx)("div",{className:"square__bar",onPointerDown:Lf?Pl:Qf?wl:void 0,style:z?{background:z.soft}:void 0,children:R&&(0,D.jsx)("div",{className:"square__name",children:(0,D.jsx)("span",{children:R})})}),(0,D.jsx)("button",{className:"square__minimize",onPointerDown:N=>N.stopPropagation(),onMouseDown:N=>N.stopPropagation(),onClick:N=>{N.stopPropagation(),S(t.id)},"aria-label":"minimize"}),(0,D.jsx)("button",{className:`square__fullscreen${h?" square__fullscreen--on":""}`,onPointerDown:N=>N.stopPropagation(),onMouseDown:N=>N.stopPropagation(),onClick:N=>{N.stopPropagation(),T(t.id)},"aria-label":h?"exit fullscreen":"fullscreen"}),(0,D.jsx)("button",{className:"square__close",onPointerDown:N=>N.stopPropagation(),onMouseDown:N=>N.stopPropagation(),onClick:N=>{N.stopPropagation(),b(t.id)},"aria-label":"close"}),(0,D.jsxs)("div",{className:"square__body",children:[B&&(0,D.jsx)("iframe",{ref:_,className:"square__iframe",sandbox:"allow-scripts allow-same-origin",src:ot,title:R??t.entity}),Kt&&t.entity&&(0,D.jsx)(t0,{entity:t.entity,branch:A,onClose:()=>xe(!1)})]}),lu&&(0,D.jsx)("div",{className:"square__copy-scrim",children:(0,D.jsxs)("div",{className:"square__copy-toast",children:[(0,D.jsx)("span",{className:"square__copy-toast-title",children:"Link to this artifact copied"}),(0,D.jsx)("span",{className:"square__copy-toast-sub",children:"paste into your agent to make changes"})]})}),!B&&(0,D.jsxs)("button",{className:"square__pick",onClick:N=>{N.stopPropagation(),r(t.id)},children:[(0,D.jsx)("span",{className:"square__pick-plus",children:"+"}),(0,D.jsx)("span",{className:"square__pick-label",children:"Choose artifact"})]}),s&&(0,D.jsx)("div",{className:"picker-anchor",children:(0,D.jsx)(Jp,{initialEntity:t.entity,onPick:f,onClose:m})}),jf&&Ag.map(N=>(0,D.jsx)("div",{className:`square__edge square__edge--${N}`,onPointerDown:nu(N),"aria-label":`resize ${N}`},N)),jf&&Tg.map(N=>(0,D.jsx)("div",{className:`square__handle square__handle--${N}`,onPointerDown:nu(N),"aria-label":`resize ${N}`},N))]}),!h&&!Bt&&ue&&Array.from({length:ue.w*ue.h},(N,Jt)=>{let h0=Jt%ue.w,v0=Math.floor(Jt/ue.w);return(0,D.jsx)("div",{className:"selection-cell",style:{transform:`translate(${ue.col*u+h0*ze}px, ${ue.row*u+v0*ze}px)`,width:ze,height:ze}},Jt)})]})}var Yf=j(xt(),1);function e0({cellSize:t,cols:e,rows:l}){if(t<=0)return null;let a=t*6,n=Math.floor(e/6),u=Math.floor(l/6),i=[];for(let o=0;o<=u;o++)for(let c=0;c<=n;c++)i.push({x:c*a,y:o*a});return(0,Yf.jsx)("div",{className:"grid-overlay",children:i.map((o,c)=>(0,Yf.jsx)("div",{className:"grid-dot",style:{left:o.x,top:o.y}},c))})}var eu=j(xt(),1);function l0({rect:t,cellSize:e,maxW:l,maxH:a}){if(e<=0)return null;let n=6*e,{col:u,row:i,w:o,h:c}=Ui(t,e,l,a),s=u*e,h=i*e,v=[];for(let d=0;d<c;d++)for(let p=0;p<o;p++)v.push({c:p,r:d});return(0,eu.jsx)(eu.Fragment,{children:v.map(({c:d,r:p})=>(0,eu.jsx)("div",{className:"selection-cell",style:{transform:`translate(${s+d*n}px, ${h+p*n}px)`,width:n,height:n}},`${p}-${d}`))})}var Ki=j(Yt(),1);var Vi=j(xt(),1);function Cg(t){return t.name??t.entity??"Empty"}var Rg=4;function Gf({tiles:t,vertical:e=!1,onRestore:l,onReorder:a}){let[n,u]=(0,Ki.useState)(null),i=(0,Ki.useCallback)((o,c,s)=>{if(o.button!==0)return;o.preventDefault();let h=o.clientX,v=o.clientY,p=o.currentTarget.parentElement;if(!p)return;let S=Array.from(p.querySelectorAll(":scope > .rail__tab")).map(g=>g.getBoundingClientRect()),T=!1,r=c,f=g=>{let E=g.clientX-h,w=g.clientY-v;if(!T&&Math.hypot(E,w)>Rg&&(T=!0,u(s)),!T)return;let x=e?g.clientY:g.clientX,M=c,O=1/0;for(let _=0;_<S.length;_++){let q=S[_],nt=e?q.top+q.height/2:q.left+q.width/2,Kt=Math.abs(x-nt);Kt<O&&(O=Kt,M=_)}r=M},m=()=>{window.removeEventListener("pointermove",f),window.removeEventListener("pointerup",m),T?(u(null),r!==c&&a(c,r)):l(s)};window.addEventListener("pointermove",f),window.addEventListener("pointerup",m)},[l,a,e]);return t.length===0?null:(0,Vi.jsx)("div",{className:`rail${e?" rail--vertical":""}`,children:t.map((o,c)=>{let s=Cg(o),h=Xi(o),v=n===o.id;return(0,Vi.jsx)("button",{className:`rail__tab${v?" rail__tab--dragging":""}`,onPointerDown:d=>i(d,c,o.id),title:s,style:h?{background:h.full}:void 0,children:(0,Vi.jsx)("span",{className:"rail__tab-label",children:s})},o.id)})})}var bt=j(xt(),1),Ug=1,a0=()=>`tile-${Ug++}`;function Hg(t){let[e,l]=(0,k.useState)({w:0,h:0});return(0,k.useEffect)(()=>{let a=t.current;if(!a)return;let n=()=>{let i=a.getBoundingClientRect();l({w:i.width,h:i.height})};n();let u=new ResizeObserver(n);return u.observe(a),()=>u.disconnect()},[t]),e}function u0(){let t=(0,k.useRef)(null),e=(0,k.useRef)(null),l=Hg(t),a=(0,k.useContext)(Ne),n=(0,k.useContext)(Yi),[u,i]=(0,k.useState)([]),[o,c]=(0,k.useState)(null),[s,h]=(0,k.useState)(null),[v,d]=(0,k.useState)(null),{cols:p,rows:b,maxW:S,maxH:T,cellSize:r}=(0,k.useMemo)(()=>Hp(l.w,l.h),[l.w,l.h]),f=r*p,m=r*b,g=Math.max(0,(l.w-f)/2),E=Math.max(0,(l.h-m)/2),w=(0,k.useMemo)(()=>u.filter(z=>!z.minimized),[u]),x=(0,k.useMemo)(()=>u.filter(z=>z.minimized),[u]),M=(0,k.useCallback)(z=>{if(r<=0)return;let A=Ui(z,r,S,T),R=A.w,B=A.h,ot=ge(R),ut=ge(B),it=Ht(A.col,0,p-ot),zt=Ht(A.row,0,b-ut),rt=a0();i(Bt=>[...Bt,{id:rt,w:R,h:B,col:it,row:zt,minimized:!1}]),c(rt),h(rt)},[r,p,b,S,T]),O=(0,k.useCallback)(z=>c(z),[]),_=(0,k.useCallback)((z,A)=>{i(R=>R.map(B=>B.id===z?{...B,col:A.col,row:A.row}:B))},[]),q=(0,k.useCallback)((z,A)=>{i(R=>{let B=ge(A.w),ot=ge(A.h),ut=Ht(A.col,0,p-B),it=Ht(A.row,0,b-ot);return R.map(zt=>zt.id===z?{...zt,w:A.w,h:A.h,col:ut,row:it}:zt)})},[p]),nt=(0,k.useCallback)(z=>{d(A=>A===z?null:z),c(z)},[]),Kt=(0,k.useCallback)(z=>{i(A=>A.map(R=>R.id===z?{...R,minimized:!0}:R)),c(A=>A===z?null:A),h(A=>A===z?null:A),d(A=>A===z?null:A)},[]),xe=(0,k.useCallback)(z=>{i(A=>A.filter(R=>R.id!==z)),c(A=>A===z?null:A),h(A=>A===z?null:A),d(A=>A===z?null:A)},[]),lu=(0,k.useCallback)(z=>{i(A=>A.map(R=>R.id===z?{...R,minimized:!1}:R)),c(z)},[]),au=(0,k.useCallback)((z,A)=>{i(R=>{let B=R.filter(rt=>!rt.minimized),ot=R.filter(rt=>rt.minimized);if(z<0||z>=B.length)return R;let ut=Math.max(0,Math.min(B.length-1,A));if(ut===z)return R;let it=[...B],[zt]=it.splice(z,1);return it.splice(ut,0,zt),[...ot,...it]})},[]),Il=(0,k.useCallback)(z=>{let A=a0();i(R=>{let B=R.filter(rt=>!rt.minimized),ot=R.filter(rt=>rt.minimized),ut=Math.max(0,Math.min(B.length,z)),it={id:A,w:2,h:2,col:0,row:0,minimized:!1},zt=[...B];return zt.splice(ut,0,it),[...ot,...zt]}),c(A),h(A)},[]),Ja=(0,k.useCallback)((z,A)=>{i(R=>{let B=R.filter(rt=>rt.minimized),ot=R.filter(rt=>!rt.minimized);if(z<0||z>=B.length)return R;let ut=Math.max(0,Math.min(B.length-1,A));if(ut===z)return R;let it=[...B],[zt]=it.splice(z,1);return it.splice(ut,0,zt),[...it,...ot]})},[]),nu=(0,k.useCallback)(z=>{i(A=>A.map(R=>R.id===z?{...R,docWidth:(R.docWidth??"full")==="full"?"half":"full"}:R))},[]),ql=(0,k.useCallback)(z=>h(z),[]),Pl=(0,k.useCallback)(()=>h(null),[]),Wa=(0,k.useCallback)(z=>{let A=s;A&&(i(R=>R.map(B=>B.id===A?{...B,entity:z.entity,name:z.name}:B)),h(null))},[s]),wl=kp({containerRef:e,enabled:r>0,onCommit:M,onCancel:()=>c(null)});(0,k.useEffect)(()=>{if(!a)return;let z=B=>{if(!B)return null;let ot=document.querySelectorAll("iframe.square__iframe");for(let ut of ot){if(ut.contentWindow!==B)continue;return ut.closest(`[${In}]`)?.getAttribute(In)??null}return null},A=async(B,ot)=>{let ut=ot.branch?.trim()||void 0,it=ot.entity?.trim(),zt=ot.name?.trim()||void 0;if(!it&&zt)try{it=await Hi(a,ut??"main",zt)}catch(rt){console.warn(`[tonk-portals] tonk:navigate resolve failed: ${rt}`);return}if(!it){console.warn("[tonk-portals] tonk:navigate ignored: missing entity and name");return}i(rt=>rt.map(Bt=>Bt.id===B?{...Bt,entity:it,name:zt,branch:ut}:Bt))},R=B=>{if(B.origin!==window.location.origin||!Bp(B.data))return;let ot=z(B.source);if(!ot)return;let ut=B.data;switch(ut.type){case"tonk:navigate":A(ot,ut);break;case"tonk:close":xe(ot);break;default:{let it=ut}}};return window.addEventListener("message",R),()=>window.removeEventListener("message",R)},[a,xe]),(0,k.useEffect)(()=>{let z=A=>{if(A.key==="Escape"&&v){d(null);return}if(o)if(A.key==="Backspace"||A.key==="Delete"){let R=A.target;if(R&&(R.tagName==="INPUT"||R.tagName==="TEXTAREA"))return;xe(o)}else A.key==="Escape"&&c(null)};return window.addEventListener("keydown",z),()=>window.removeEventListener("keydown",z)},[o,v,xe]);let ze=v!=null?w.find(z=>z.id===v)??null:null,ue=ze!=null,Ji={position:"absolute",left:g,top:E,width:f,height:m};return n==="doc"?(0,bt.jsxs)("div",{ref:t,className:"grid-wrapper grid-wrapper--doc",children:[(0,bt.jsxs)("div",{className:"grid--doc",children:[(0,bt.jsx)(n0,{onInsert:()=>Il(0)}),w.map((z,A)=>(0,bt.jsxs)(k.Fragment,{children:[(0,bt.jsx)(Zi,{tile:z,x:0,y:0,w:0,h:0,cellSize:r,maxW:S,maxH:T,selected:z.id===o,pickerOpen:z.id===s,fullscreen:!1,mode:"doc",index:A,onSelect:O,onMove:_,onResize:q,onClose:xe,onMinimize:Kt,onFullscreen:nt,onOpenPicker:ql,onPick:Wa,onClosePicker:Pl,onReorder:au,onToggleDocWidth:nu}),(0,bt.jsx)(n0,{onInsert:()=>Il(A+1)})]},z.id))]}),(0,bt.jsx)(Gf,{tiles:x,vertical:!0,onRestore:lu,onReorder:Ja})]}):(0,bt.jsx)("div",{ref:t,className:"grid-wrapper",children:(0,bt.jsxs)("div",{className:"grid-stage",style:{width:l.w,height:l.h},onMouseDown:z=>{z.target===z.currentTarget&&c(null)},children:[(0,bt.jsxs)("div",{ref:e,className:`grid${wl?" grid--dragging":""}${u.length===0?" grid--empty":""}`,style:Ji,onMouseDown:z=>{z.target===z.currentTarget&&c(null)},children:[(0,bt.jsx)(e0,{cellSize:r,cols:p,rows:b}),!ue&&w.map(z=>(0,bt.jsx)(Zi,{tile:z,x:z.col*r,y:z.row*r,w:ge(z.w)*r,h:ge(z.h)*r,cellSize:r,maxW:S,maxH:T,selected:z.id===o,pickerOpen:z.id===s,fullscreen:!1,onSelect:O,onMove:_,onResize:q,onClose:xe,onMinimize:Kt,onFullscreen:nt,onOpenPicker:ql,onPick:Wa,onClosePicker:Pl},z.id)),ue&&ze&&(0,bt.jsx)(Zi,{tile:ze,x:0,y:0,w:f,h:m,cellSize:r,maxW:S,maxH:T,selected:!0,pickerOpen:ze.id===s,fullscreen:!0,onSelect:O,onMove:_,onResize:q,onClose:xe,onMinimize:Kt,onFullscreen:nt,onOpenPicker:ql,onPick:Wa,onClosePicker:Pl},ze.id),!ue&&wl&&(0,bt.jsx)(l0,{rect:wl,cellSize:r,maxW:S,maxH:T}),u.length===0&&!wl&&(0,bt.jsx)("div",{className:"grid__empty",children:"Drag anywhere to create a tile"})]}),(0,bt.jsx)(Gf,{tiles:x,onRestore:lu,onReorder:Ja})]})})}function n0({onInsert:t}){return(0,bt.jsx)("div",{className:"doc-inserter",onClick:e=>{e.stopPropagation(),t()},"aria-label":"add block",children:(0,bt.jsx)("button",{type:"button",className:"doc-inserter__btn",onClick:e=>{e.stopPropagation(),t()},"aria-label":"add block",children:"+"})})}var Se=j(xt(),1),kg=3e3,Bg="main",Yg="(max-width: 768px)";function Gg(t){(0,Ka.useEffect)(()=>{if(!t)return;let e=!1,l=!1,a=null,n=`/api/repository/${encodeURIComponent(t)}/branch/${encodeURIComponent(Bg)}/sync`,u=async()=>{if(!(l||e)&&!(typeof document<"u"&&document.hidden)){e=!0;try{await fetch(n,{method:"POST"})}catch(i){console.warn("[tonk-portals] background sync failed:",i)}finally{e=!1}}};return u(),a=setInterval(u,kg),()=>{l=!0,a&&clearInterval(a)}},[t])}function Xg(){return typeof window>"u"?"canvas":window.matchMedia(Yg).matches?"doc":"canvas"}function Lg({mode:t,onChange:e}){return(0,Se.jsx)("div",{className:"tp-header",children:(0,Se.jsxs)("div",{className:"tp-mode-switch",role:"tablist","aria-label":"view mode",children:[(0,Se.jsx)("button",{role:"tab","aria-selected":t==="canvas",className:`tp-mode-switch__btn${t==="canvas"?" tp-mode-switch__btn--active":""}`,onClick:()=>e("canvas"),children:"Canvas"}),(0,Se.jsx)("button",{role:"tab","aria-selected":t==="doc",className:`tp-mode-switch__btn${t==="doc"?" tp-mode-switch__btn--active":""}`,onClick:()=>e("doc"),children:"Doc"})]})})}function o0({repo:t,host:e}){Gg(t);let[l,a]=(0,Ka.useState)(Xg),n=(0,Ka.useCallback)(u=>{let i=document;if(typeof i.startViewTransition!="function"){a(u);return}i.startViewTransition(()=>{(0,i0.flushSync)(()=>a(u))})},[]);return(0,Se.jsx)(Ne.Provider,{value:t,children:(0,Se.jsx)(Bi.Provider,{value:e,children:(0,Se.jsxs)(Yi.Provider,{value:l,children:[(0,Se.jsx)(Lg,{mode:l,onChange:n}),(0,Se.jsx)(u0,{})]})})})}var c0=`/*
 * tonk-portals styles. All selectors are scoped under
 * \`.tonk-portals-root\` because the bundle injects this stylesheet
 * into the host page's <head>; we share a document with the Leptos
 * shell, so unscoped names like \`.grid\`, \`.square\`, \`.rail\` would
 * collide with anything else on the page.
 *
 * The original prototype assumed it owned the viewport (\`.app\`
 * was 100vw/100vh). Here, the element lives inside a <main> that
 * sits next to the Leptos sidebar and below the banner, so we
 * size off the wrapper element instead and let \`useElementSize\`
 * drive the grid dimensions.
 */

.tonk-portals-root {
  position: relative;
  width: 100%;
  height: 100%;
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
  /* Theme-aware tokens. We deliberately *don't* hard-code light
     colours here \u2014 the host page picks the WA theme (default is
     fairly dark/brutalist on this site), so the tile chrome and
     pick-button text need to follow the host's foreground
     colour or they go invisible on a dark surface. The fallbacks
     mirror the prototype's light-mode look for non-WA hosts. */
  --tp-bg: transparent;
  --tp-square-bg: var(--wa-color-surface-default, #ffffff);
  --tp-square-fg: var(--wa-color-text-normal, #1d1d1f);
  --tp-square-fg-quiet: color-mix(in oklab, var(--tp-square-fg) 70%, transparent);
  --tp-square-fg-quieter: color-mix(in oklab, var(--tp-square-fg) 45%, transparent);
  --tp-square-fg-faint: color-mix(in oklab, var(--tp-square-fg) 25%, transparent);
  --tp-square-border: color-mix(in oklab, var(--tp-square-fg) 18%, transparent);
  --tp-square-hover: color-mix(in oklab, var(--tp-square-fg) 8%, transparent);
  --tp-accent: var(--wa-color-brand-fill-loud, #0a84ff);
  --tp-accent-soft: color-mix(in oklab, var(--tp-accent) 12%, transparent);
  --tp-shadow: 0 2px 6px rgba(0, 0, 0, 0.06);
  --tp-shadow-strong: 0 8px 24px rgba(0, 0, 0, 0.10);
  --tp-dot: color-mix(in oklab, var(--tp-square-fg) 22%, transparent);

  /* Typography scale \u2014 two body sizes only. Tile chrome adopts
     the same WA tokens as the Leptos shell so the chrome reads
     as one continuous UI; the iframe content inside each tile
     is intentionally its own world (its own bundle, its own
     styles \u2014 we do not touch it). */
  --tp-font-size-body: var(--wa-font-size-s, 13px);
  --tp-font-size-small: var(--wa-font-size-2xs, 11px);
  --tp-font-size-hint: var(--wa-font-size-xs, 12px);

  /* Radius scale \u2014 only three steps. Icon buttons get 6px,
     medium buttons / cards get 8px, tile windows get 12px, and
     full pills use 999px. */
  --tp-radius-button: 4px;
  --tp-radius-card: 6px;
  --tp-radius-tile: 8px;

  /* Motion \u2014 two durations cover state changes vs. layout shifts. */
  --tp-motion-quick: 120ms ease;
  --tp-motion-settle: 240ms cubic-bezier(0.2, 0.8, 0.2, 1);
  background: var(--tp-bg);
  color: var(--tp-square-fg);
  user-select: none;
  -webkit-user-select: none;
  font-family: inherit;
}

.tonk-portals-root *,
.tonk-portals-root *::before,
.tonk-portals-root *::after {
  box-sizing: border-box;
}

.tonk-portals-root .grid-wrapper {
  position: relative;
  flex: 1;
  width: 100%;
  display: flex;
  align-items: stretch;
  justify-content: stretch;
  overflow: hidden;
}

/* The stage is the positioning context shared by the inner grid
 * box, the absolute-positioned tile squares, the edge rails, and
 * the insert ghost. Everything inside uses stage-local coords. */
.tonk-portals-root .grid-stage {
  position: relative;
  flex: 1;
}

.tonk-portals-root .grid {
  position: absolute;
  cursor: crosshair;
}

.tonk-portals-root .grid--dragging {
  cursor: crosshair;
}

.tonk-portals-root .grid--empty {
  cursor: crosshair;
}

.tonk-portals-root .grid__empty {
  position: absolute;
  inset: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  pointer-events: none;
  color: var(--tp-square-fg-quieter);
  font-size: var(--tp-font-size-body);
  letter-spacing: 0.01em;
}

/* Dot grid overlay \u2014 fades in on hover / during drag or resize */
.tonk-portals-root .grid-overlay {
  position: absolute;
  inset: 0;
  pointer-events: none;
  opacity: 0;
  transition: opacity 160ms ease;
}

.tonk-portals-root .grid:hover .grid-overlay {
  opacity: 0.35;
}

.tonk-portals-root .grid--dragging .grid-overlay,
.tonk-portals-root .grid:has(.square--active) .grid-overlay {
  opacity: 1;
}

.tonk-portals-root .grid-dot {
  position: absolute;
  width: 3px;
  height: 3px;
  margin-left: -1.5px;
  margin-top: -1.5px;
  border-radius: 50%;
  background: var(--tp-dot);
}

/* Selection cell \u2014 shown during draw-to-create and during drag/resize ghost */
.tonk-portals-root .selection-cell {
  position: absolute;
  top: 0;
  left: 0;
  background: var(--tp-accent-soft);
  border: 1.5px solid var(--tp-accent);
  border-radius: var(--tp-radius-card);
  pointer-events: none;
  box-shadow: inset 0 0 0 1px rgba(255, 255, 255, 0.5);
}

.tonk-portals-root .square {
  position: absolute;
  top: 0;
  left: 0;
  background: var(--tp-square-bg);
  border-radius: var(--tp-radius-tile);
  box-shadow: var(--tp-shadow);
  cursor: pointer;
  /* No transform/size transition: the tile snaps to its committed
     position the instant the drag/resize ends \u2014 no post-drop glide.
     Only box-shadow still eases (for hover/selected states). */
  transition: box-shadow 180ms ease;
}

.tonk-portals-root .square--selected {
  box-shadow:
    var(--tp-shadow-strong),
    inset 0 0 0 2px var(--tp-accent);
}

.tonk-portals-root .square__bar {
  position: absolute;
  top: 0;
  left: 0;
  right: 0;
  height: 28px;
  cursor: grab;
  border-top-left-radius: var(--tp-radius-tile);
  border-top-right-radius: var(--tp-radius-tile);
  touch-action: none;
  z-index: 1;
}

.tonk-portals-root .square__bar:active {
  cursor: grabbing;
}

/* Drag-handle dots \u2014 2 cols \xD7 3 rows, floating outside the square to the left.
   padding-right bridges the gap back to the square so hover doesn't drop out. */
.tonk-portals-root .square__drag-dots {
  position: absolute;
  right: calc(100% + 10px);
  top: 7px;
  display: grid;
  grid-template-columns: repeat(2, 3px);
  grid-template-rows: repeat(3, 3px);
  gap: 3px;
  padding: 6px;
  padding-right: 16px; /* bridge the gap to the square edge */
  margin: -6px;
  cursor: grab;
  opacity: 0;
  transition: opacity 120ms ease;
}

.tonk-portals-root .square__drag-dots:active {
  cursor: grabbing;
}

.tonk-portals-root .square:hover .square__drag-dots,
.tonk-portals-root .square__drag-dots:hover {
  opacity: 1;
}

.tonk-portals-root .square__drag-dot {
  width: 3px;
  height: 3px;
  border-radius: 50%;
  background: var(--tp-square-fg-quieter);
}

/* Pill menu \u2014 top-left of the window, half above */
.tonk-portals-root .tile-pill {
  position: absolute;
  top: 0;
  left: 8px;
  transform: translateY(-50%);
  z-index: 8;
  display: flex;
  flex-direction: row;
  align-items: center;
  gap: 2px;
  padding: 4px 8px;
  background: var(--tp-square-bg);
  border-radius: 999px;
  box-shadow:
    0 2px 8px rgba(0, 0, 0, 0.12),
    0 0 0 1px var(--tp-square-border);
  opacity: 0;
  pointer-events: none;
  transition: opacity 140ms ease;
}

.tonk-portals-root .square:hover .tile-pill {
  opacity: 1;
  pointer-events: auto;
}

.tonk-portals-root .tile-pill__btn {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 24px;
  height: 24px;
  border: none;
  background: transparent;
  border-radius: var(--tp-radius-button);
  cursor: pointer;
  padding: 0;
  color: var(--tp-square-fg-quieter);
  transition: background 100ms ease, color 100ms ease;
}

.tonk-portals-root .tile-pill__btn svg {
  width: 14px;
  height: 14px;
  display: block;
}

.tonk-portals-root .tile-pill__btn:hover {
  background: var(--tp-square-hover);
  color: var(--tp-square-fg);
}

.tonk-portals-root .tile-pill__btn--active {
  color: var(--tp-accent);
}

.tonk-portals-root .square__name {
  position: absolute;
  top: 0;
  left: 0;
  right: 0;
  height: 28px;
  display: flex;
  align-items: center;
  justify-content: center;
  pointer-events: none;
  /* right: room for 3 window buttons */
  padding: 0 80px 0 12px;
}

.tonk-portals-root .square__name > span {
  font-size: var(--tp-font-size-body);
  font-weight: 500;
  color: var(--tp-square-fg-quiet);
  letter-spacing: -0.005em;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  max-width: 100%;
}

.tonk-portals-root .square__minimize,
.tonk-portals-root .square__fullscreen,
.tonk-portals-root .square__close {
  position: absolute;
  top: 4px;
  width: 22px;
  height: 22px;
  border: none;
  background: transparent;
  display: flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
  border-radius: var(--tp-radius-button);
  color: var(--tp-square-fg-quieter);
  padding: 0;
  z-index: 5;
  transition:
    background 120ms ease,
    color 120ms ease;
}

/* Window controls \u2014 right-aligned */
.tonk-portals-root .square__close    { right: 4px;  left: auto; }
.tonk-portals-root .square__fullscreen { right: 28px; left: auto; }
.tonk-portals-root .square__minimize { right: 52px; left: auto; }

.tonk-portals-root .square__minimize:hover,
.tonk-portals-root .square__fullscreen:hover,
.tonk-portals-root .square__close:hover {
  background: var(--tp-square-hover);
  color: var(--tp-square-fg);
}

.tonk-portals-root .square__minimize::before {
  content: "";
  width: 10px;
  height: 2px;
  background: currentColor;
  border-radius: 1px;
}

.tonk-portals-root .square__fullscreen::before {
  content: "";
  width: 10px;
  height: 10px;
  border: 1.5px solid currentColor;
  border-radius: 2px;
}

.tonk-portals-root .square__fullscreen--on {
  color: var(--tp-accent);
}

.tonk-portals-root .square__close::before,
.tonk-portals-root .square__close::after {
  content: "";
  position: absolute;
  width: 12px;
  height: 1.5px;
  background: currentColor;
  border-radius: 1px;
}

.tonk-portals-root .square__close::before { transform: rotate(45deg); }
.tonk-portals-root .square__close::after { transform: rotate(-45deg); }

.tonk-portals-root .bar-menu {
  position: absolute;
  top: 4px;
  right: 8px;
  z-index: 5;
}

.tonk-portals-root .bar-menu__btn {
  width: 22px;
  height: 22px;
  border: none;
  background: transparent;
  display: flex;
  flex-direction: row;
  align-items: center;
  justify-content: center;
  gap: 2px;
  cursor: pointer;
  padding: 0;
  border-radius: var(--tp-radius-button);
  color: var(--tp-square-fg-quieter);
  transition:
    background 120ms ease,
    color 120ms ease;
}

.tonk-portals-root .bar-menu__btn:hover {
  background: var(--tp-square-hover);
  color: var(--tp-square-fg);
}

.tonk-portals-root .bar-menu__line {
  width: 3px;
  height: 3px;
  background: currentColor;
  border-radius: 50%;
}

.tonk-portals-root .bar-menu__dropdown {
  position: absolute;
  top: calc(100% + 4px);
  right: 0;
  min-width: 140px;
  background: var(--tp-square-bg);
  border-radius: var(--tp-radius-card);
  box-shadow:
    0 8px 24px rgba(0, 0, 0, 0.18),
    0 0 0 1px rgba(0, 0, 0, 0.04);
  padding: 4px;
  z-index: 51;
  display: flex;
  flex-direction: column;
  animation: tp-pop 120ms cubic-bezier(0.2, 0.8, 0.2, 1);
  transform-origin: top right;
}

.tonk-portals-root .bar-menu__item {
  background: transparent;
  border: none;
  padding: 7px 10px;
  font-size: var(--tp-font-size-body);
  font-family: inherit;
  text-align: left;
  cursor: pointer;
  border-radius: var(--tp-radius-button);
  color: var(--tp-square-fg);
  white-space: nowrap;
  transition: background 80ms ease;
}

.tonk-portals-root .bar-menu__item:hover {
  background: var(--tp-square-hover);
}

.tonk-portals-root .bar-menu__flash {
  position: absolute;
  top: calc(100% + 4px);
  right: 0;
  background: rgba(29, 29, 31, 0.92);
  color: white;
  font-size: var(--tp-font-size-small);
  font-weight: 500;
  padding: 4px 8px;
  border-radius: var(--tp-radius-button);
  white-space: nowrap;
  z-index: 51;
  letter-spacing: 0.02em;
  animation: tp-pop 120ms cubic-bezier(0.2, 0.8, 0.2, 1);
}

.tonk-portals-root .rail {
  position: absolute;
  left: 50%;
  bottom: 8px;
  transform: translateX(-50%);
  display: flex;
  flex-direction: row;
  gap: 6px;
  z-index: 20;
  max-width: calc(100% - 32px);
  overflow-x: auto;
  padding: 4px;
  pointer-events: auto;
}

/* Vertical rail variant \u2014 used in doc mode where minimised tiles
   stack on the bottom-left edge so they don't obscure the
   reading column. */
.tonk-portals-root .rail--vertical {
  left: 12px;
  bottom: 12px;
  transform: none;
  flex-direction: column;
  align-items: stretch;
  max-width: 200px;
  max-height: calc(100% - 24px);
  overflow-x: hidden;
  overflow-y: auto;
}

.tonk-portals-root .rail__tab--dragging {
  opacity: 0.5;
  cursor: grabbing;
  box-shadow: var(--tp-shadow-strong);
}

.tonk-portals-root .rail__tab {
  height: 36px;
  min-width: 100px;
  max-width: 180px;
  padding: 0 14px;
  border-radius: var(--tp-radius-card);
  background: var(--tp-square-bg);
  box-shadow: var(--tp-shadow);
  border: 1px solid var(--tp-square-border);
  cursor: pointer;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: var(--tp-font-size-body);
  font-family: inherit;
  color: var(--tp-square-fg-quiet);
  letter-spacing: 0.01em;
  transition:
    transform 120ms cubic-bezier(0.2, 0.8, 0.2, 1),
    box-shadow 120ms ease,
    color 120ms ease;
}

.tonk-portals-root .rail__tab:hover {
  box-shadow: var(--tp-shadow-strong);
  transform: translateY(-1px);
  color: var(--tp-accent);
}

.tonk-portals-root .rail__tab-label {
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  max-width: 100%;
  font-weight: 500;
}

.tonk-portals-root .square__body {
  position: absolute;
  top: 28px;
  left: 0;
  right: 0;
  bottom: 0;
  overflow: hidden;
  border-bottom-left-radius: var(--tp-radius-tile);
  border-bottom-right-radius: var(--tp-radius-tile);
}

.tonk-portals-root .square__iframe {
  width: 100%;
  height: 100%;
  border: 0;
  background: transparent;
  display: block;
}

.tonk-portals-root .square__pick {
  position: absolute;
  inset: 0;
  /* WA's native.css sets \`button { height: var(--wa-form-control-height) }\`
     plus padding / line-height / white-space defaults. Those leak into the
     light DOM here and would collapse the button to a ~40px form-control
     box centered near the tile top instead of letting \`inset: 0\` size it
     to the whole tile. Explicitly clear those defaults. */
  width: 100%;
  height: 100%;
  padding: 0;
  line-height: 1.2;
  white-space: normal;
  font-size: inherit;
  font-weight: inherit;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 6px;
  background: transparent;
  border: none;
  cursor: pointer;
  font-family: inherit;
  color: var(--tp-square-fg-quiet);
  border-radius: var(--tp-radius-tile);
  transition:
    background 120ms ease,
    color 120ms ease;
}

.tonk-portals-root .square__pick:hover {
  background: var(--tp-square-hover);
  color: var(--tp-square-fg);
}

.tonk-portals-root .square__pick-plus {
  font-size: 28px;
  font-weight: 300;
  line-height: 1;
  color: var(--tp-square-fg-faint);
}

.tonk-portals-root .square__pick:hover .square__pick-plus {
  color: var(--tp-accent);
}

.tonk-portals-root .square__pick-label {
  font-size: var(--tp-font-size-body);
  font-weight: 500;
  letter-spacing: 0.01em;
}

.tonk-portals-root .square__copy-scrim {
  position: absolute;
  inset: 0;
  border-radius: 12px;
  z-index: 20;
  background: rgba(0, 0, 0, 0.45);
  display: flex;
  align-items: center;
  justify-content: center;
  pointer-events: none;
  animation: tp-scrim-in 180ms ease;
}

@keyframes tp-scrim-in {
  from { opacity: 0; }
  to   { opacity: 1; }
}

.tonk-portals-root .square__copy-toast {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 4px;
  padding: 14px 20px;
  background: rgba(255, 255, 255, 0.12);
  border: 1px solid rgba(255, 255, 255, 0.2);
  backdrop-filter: blur(8px);
  -webkit-backdrop-filter: blur(8px);
  border-radius: 12px;
  text-align: center;
  animation: tp-toast-in 200ms cubic-bezier(0.2, 0.8, 0.2, 1);
}

@keyframes tp-toast-in {
  from { opacity: 0; transform: translateY(6px); }
  to   { opacity: 1; transform: translateY(0); }
}

.tonk-portals-root .square__copy-toast-title {
  font-size: 13px;
  font-weight: 600;
  color: #fff;
}

.tonk-portals-root .square__copy-toast-sub {
  font-size: 11px;
  color: rgba(255, 255, 255, 0.7);
}

.tonk-portals-root .picker-anchor {
  position: absolute;
  top: calc(50% + 14px);
  left: 50%;
  transform: translate(-50%, -50%);
  z-index: 50;
  pointer-events: none;
}

.tonk-portals-root .picker {
  pointer-events: auto;
  width: 280px;
  background: var(--tp-square-bg);
  border-radius: var(--tp-radius-card);
  box-shadow:
    0 8px 24px rgba(0, 0, 0, 0.14),
    0 0 0 1px rgba(0, 0, 0, 0.06);
  overflow: hidden;
  display: flex;
  flex-direction: column;
  animation: tp-pop 140ms cubic-bezier(0.2, 0.8, 0.2, 1);
  transform-origin: center;
}

.tonk-portals-root .picker__input {
  width: 100%;
  border: none;
  outline: none;
  padding: 8px 12px;
  font-size: var(--tp-font-size-body);
  font-family: inherit;
  background: transparent;
  color: var(--tp-square-fg);
}

.tonk-portals-root .picker__input::placeholder {
  color: var(--tp-square-fg-faint);
}

.tonk-portals-root .picker__list {
  list-style: none;
  margin: 0;
  padding: 3px 0;
  max-height: 240px;
  overflow-y: auto;
  border-top: 1px solid var(--tp-square-border);
}

/* Single-line row: name left, description right */
.tonk-portals-root .picker__item {
  display: flex;
  flex-direction: row;
  align-items: center;
  gap: 8px;
  padding: 5px 12px;
  cursor: pointer;
  transition: background 80ms ease;
}

.tonk-portals-root .picker__item--highlighted,
.tonk-portals-root .picker__item:hover {
  background: var(--tp-square-hover);
}

.tonk-portals-root .picker__item-name {
  font-size: var(--tp-font-size-body);
  font-weight: 500;
  color: var(--tp-square-fg);
  flex-shrink: 0;
}

.tonk-portals-root .picker__item-entity {
  font-size: var(--tp-font-size-small);
  color: var(--tp-square-fg-quieter);
  font-family: var(--wa-font-family-code, ui-monospace, monospace);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  margin-left: auto;
}

.tonk-portals-root .picker__list--builtins {
  border-top: none;
}

.tonk-portals-root .picker__list--builtins::before {
  content: "Built-in";
  display: block;
  padding: 5px 12px 2px;
  font-size: 10px;
  font-weight: 600;
  letter-spacing: 0.06em;
  text-transform: uppercase;
  color: var(--tp-square-fg-quieter);
}

.tonk-portals-root .picker__item--builtin .picker__item-entity {
  color: var(--tp-square-fg-quieter);
  font-family: inherit;
}

.tonk-portals-root .picker__agent-link {
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: 1px;
  padding: 6px 12px 8px;
  background: none;
  border: none;
  cursor: pointer;
  font-family: inherit;
  text-align: left;
}

.tonk-portals-root .picker__agent-link:hover {
  background: var(--tp-square-hover);
}

.tonk-portals-root .picker__agent-link {
  font-size: var(--tp-font-size-body);
  font-weight: 500;
  color: var(--tp-accent);
}

.tonk-portals-root .picker__agent-link-sub {
  font-size: var(--tp-font-size-small);
  font-weight: 400;
  color: var(--tp-square-fg-quieter);
}

.tonk-portals-root .picker__empty {
  padding: 7px 12px;
  font-size: var(--tp-font-size-body);
  color: var(--tp-square-fg-quieter);
}

.tonk-portals-root .picker__error {
  padding: 6px 12px;
  font-size: var(--tp-font-size-body);
  color: var(--wa-color-danger-fill-loud, #e03131);
  background: color-mix(in oklab, var(--wa-color-danger-fill-loud, #e03131) 10%, transparent);
  border-bottom: 1px solid var(--tp-square-border);
}

.tonk-portals-root .picker__actions {
  display: flex;
  gap: 6px;
  justify-content: flex-end;
  padding: 6px 8px 5px;
}

.tonk-portals-root .picker__action {
  font-family: inherit;
  font-size: var(--tp-font-size-small);
  font-weight: 500;
  padding: 4px 10px;
  border-radius: var(--tp-radius-button);
  border: 1px solid var(--tp-square-border);
  background: transparent;
  color: var(--tp-square-fg);
  cursor: pointer;
  transition:
    background 80ms ease,
    color 80ms ease,
    border-color 80ms ease;
}

.tonk-portals-root .picker__action:hover:not(:disabled) {
  background: var(--tp-square-hover);
}

.tonk-portals-root .picker__action--primary {
  background: var(--tp-accent);
  border-color: var(--tp-accent);
  color: white;
}

.tonk-portals-root .picker__action--primary:hover:not(:disabled) {
  filter: brightness(0.95);
  background: var(--tp-accent);
}

.tonk-portals-root .picker__action:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.tonk-portals-root .picker__hint {
  padding: 5px 10px 8px;
  font-size: 10px;
  color: var(--tp-square-fg-quieter);
  letter-spacing: 0.01em;
}

.tonk-portals-root .picker__hint kbd {
  display: inline-block;
  padding: 1px 4px;
  background: var(--tp-square-hover);
  border-radius: 3px;
  font-family: inherit;
  font-size: 10px;
  color: var(--tp-square-fg-quiet);
}

/* Source panel \u2014 overlays the iframe inside square__body */
.tonk-portals-root .source-panel {
  position: absolute;
  inset: 0;
  background: var(--tp-square-bg);
  display: flex;
  flex-direction: column;
  z-index: 5;
  border-radius: 0 0 var(--tp-radius-tile) var(--tp-radius-tile);
}

.tonk-portals-root .source-panel__bar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 6px 10px;
  border-bottom: 1px solid var(--tp-square-border);
  flex-shrink: 0;
}

.tonk-portals-root .source-panel__title {
  font-size: var(--tp-font-size-small);
  font-weight: 600;
  letter-spacing: 0.06em;
  text-transform: uppercase;
  color: var(--tp-square-fg-quieter);
}

.tonk-portals-root .source-panel__close {
  background: none;
  border: none;
  cursor: pointer;
  font-size: var(--tp-font-size-body);
  color: var(--tp-square-fg-quieter);
  padding: 2px 4px;
  border-radius: var(--tp-radius-button);
  line-height: 1;
}

.tonk-portals-root .source-panel__close:hover {
  background: var(--tp-square-hover);
  color: var(--tp-square-fg);
}

.tonk-portals-root .source-panel__body {
  flex: 1;
  overflow: auto;
  padding: 12px 16px;
}

.tonk-portals-root .source-panel__pre {
  font-family: 'Inconsolata', Menlo, 'Courier New', monospace;
  font-size: var(--tp-font-size-body);
  line-height: 1.6;
  color: var(--tp-square-fg);
  white-space: pre;
  margin: 0;
}

.tonk-portals-root .source-panel__loading,
.tonk-portals-root .source-panel__error {
  font-size: var(--tp-font-size-body);
  color: var(--tp-square-fg-quieter);
}

.tonk-portals-root .source-panel__error {
  color: var(--wa-color-danger-fill-loud, #e03131);
}

/* Resize handles */
.tonk-portals-root .square__edge {
  position: absolute;
  touch-action: none;
  z-index: 3;
}

.tonk-portals-root .square__edge--t {
  top: 0;
  left: 14px;
  right: 14px;
  height: 6px;
  cursor: ns-resize;
}

.tonk-portals-root .square__edge--b {
  bottom: 0;
  left: 14px;
  right: 14px;
  height: 6px;
  cursor: ns-resize;
}

.tonk-portals-root .square__edge--l {
  top: 14px;
  bottom: 14px;
  left: 0;
  width: 6px;
  cursor: ew-resize;
}

.tonk-portals-root .square__edge--r {
  top: 14px;
  bottom: 14px;
  right: 0;
  width: 6px;
  cursor: ew-resize;
}

.tonk-portals-root .square__handle {
  position: absolute;
  width: 14px;
  height: 14px;
  touch-action: none;
  z-index: 4;
}

.tonk-portals-root .square__handle--tl { top: 0; left: 0; cursor: nwse-resize; }
.tonk-portals-root .square__handle--tr { top: 0; right: 0; cursor: nesw-resize; }
.tonk-portals-root .square__handle--bl { bottom: 0; left: 0; cursor: nesw-resize; }
.tonk-portals-root .square__handle--br { bottom: 0; right: 0; cursor: nwse-resize; }

/* square--active: tile being dragged/resized fades back so ghost is legible */
.tonk-portals-root .square--active {
  opacity: 0.35;
  transition: opacity 100ms ease;
}

/* View-transition timing for the canvas \u2194 doc mode swap. Each tile
   carries \`view-transition-name: tp-sq-<id>\` so the browser morphs
   it between modes; everything else falls under the \`root\` group
   and just cross-fades. */
::view-transition-group(*) {
  animation-duration: 480ms;
  animation-timing-function: cubic-bezier(0.22, 1, 0.36, 1);
}

::view-transition-old(root),
::view-transition-new(root) {
  animation-duration: 220ms;
}

@keyframes tp-pop {
  from {
    opacity: 0;
    transform: scale(0.9);
  }
  to {
    opacity: 1;
    transform: scale(1);
  }
}

/* ---------------- View-mode header ---------------- */

.tonk-portals-root .tp-header {
  flex: 0 0 auto;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 10px 12px;
  z-index: 30;
  /* The header strip spans the full width of the portals root as a
     flex item, sitting over the host page's top-left drawer chevron
     and top-right share button. Let clicks fall through everywhere
     except the actual mode-switch pill. */
  pointer-events: none;
}

.tonk-portals-root .tp-mode-switch {
  pointer-events: auto;
  display: inline-flex;
  background: var(--tp-square-bg);
  border-radius: var(--tp-radius-card);
  box-shadow:
    0 2px 8px rgba(0, 0, 0, 0.08),
    0 0 0 1px var(--tp-square-border);
  padding: 2px;
}

.tonk-portals-root .tp-mode-switch__btn {
  font-family: inherit;
  font-size: var(--tp-font-size-small);
  font-weight: 500;
  letter-spacing: 0.01em;
  padding: 3px 10px;
  border: none;
  background: transparent;
  color: var(--tp-square-fg-quiet);
  border-radius: var(--tp-radius-button);
  cursor: pointer;
  transition: background 100ms ease, color 100ms ease;
}

.tonk-portals-root .tp-mode-switch__btn:hover {
  color: var(--tp-square-fg);
}

.tonk-portals-root .tp-mode-switch__btn--active {
  background: var(--tp-accent);
  color: white;
}

.tonk-portals-root .tp-mode-switch__btn--active:hover {
  color: white;
}

/* ---------------- Doc mode ---------------- */

.tonk-portals-root .grid-wrapper--doc {
  overflow-y: auto;
  overflow-x: hidden;
  display: block;
}

.tonk-portals-root .grid--doc {
  display: grid;
  grid-template-columns: repeat(2, 1fr);
  gap: 18px;
  max-width: 880px;
  margin: 0 auto;
  padding: 12px 24px 80px;
}

.tonk-portals-root .grid__empty--doc {
  position: static;
  grid-column: 1 / -1;
  padding: 80px 0;
  display: flex;
  align-items: center;
  justify-content: center;
}

.tonk-portals-root .square--doc {
  position: relative;
  top: auto;
  left: auto;
  transition:
    box-shadow 150ms ease,
    opacity 120ms ease;
}

.tonk-portals-root .square--doc-full {
  grid-column: 1 / -1;
  height: 480px;
}

.tonk-portals-root .square--doc-half {
  grid-column: span 1;
  height: 360px;
}

/* In doc mode, drag dots stay outside the block to the left (same
   anchor as canvas mode) but are always visible \u2014 they're the only
   reorder affordance. */
.tonk-portals-root .square--doc .square__drag-dots {
  opacity: 0.55;
}

.tonk-portals-root .square--doc:hover .square__drag-dots,
.tonk-portals-root .square--doc .square__drag-dots:hover {
  opacity: 1;
}

/* In doc mode, the top-right window controls collapse to a single
   minimize button (it sends the block to the Rail). Close/fullscreen
   live in canvas only. */
.tonk-portals-root .square--doc .square__close,
.tonk-portals-root .square--doc .square__fullscreen {
  display: none;
}

.tonk-portals-root .square--doc .square__minimize {
  right: 4px;
}

/* Title-bar right padding shrinks since only one button lives there. */
.tonk-portals-root .square--doc .square__name {
  padding-right: 36px;
}

/* Reorder visual \u2014 the block being dragged dims so the resting
   stack remains the reference frame. */
.tonk-portals-root .square--reorder-active {
  opacity: 0.45;
  box-shadow: var(--tp-shadow-strong);
}

/* Doc-mode rail positioning is now handled by \`.rail--vertical\`
   (bottom-left, column flow). Left intentionally empty here so the
   selector still exists for any future doc-specific overrides. */

/* Doc-mode inserter \u2014 notebook-style add-block divider between
   blocks. Spans both columns of the doc grid so it always breaks
   row flow (intentional: half blocks adjacent to an inserter no
   longer share a row). */
.tonk-portals-root .doc-inserter {
  grid-column: 1 / -1;
  position: relative;
  height: 14px;
  display: flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
  margin: -6px 0;
}

.tonk-portals-root .doc-inserter::before {
  content: "";
  position: absolute;
  left: 0;
  right: 0;
  top: 50%;
  height: 1px;
  background: var(--tp-square-border);
  opacity: 0;
  transition: opacity 120ms ease;
}

.tonk-portals-root .doc-inserter:hover::before {
  opacity: 1;
}

.tonk-portals-root .doc-inserter__btn {
  position: relative;
  width: 24px;
  height: 24px;
  border-radius: 999px;
  background: var(--tp-square-bg);
  border: 1px solid var(--tp-square-border);
  color: var(--tp-square-fg-quieter);
  font-size: 16px;
  line-height: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
  padding: 0;
  font-family: inherit;
  opacity: 0;
  transition:
    opacity 120ms ease,
    color 120ms ease,
    background 120ms ease,
    border-color 120ms ease;
}

.tonk-portals-root .doc-inserter:hover .doc-inserter__btn {
  opacity: 1;
}

.tonk-portals-root .doc-inserter__btn:hover {
  color: var(--tp-accent);
  border-color: var(--tp-accent);
}

/* Mobile: doc mode collapses to a single column so half-width
   blocks stack like full-width ones. Drag dots tuck inside the
   block (no room to the left); Rail spans the bottom. */
@media (max-width: 768px) {
  .tonk-portals-root .grid--doc {
    grid-template-columns: 1fr;
    padding: 8px 12px 80px;
    gap: 12px;
  }
  .tonk-portals-root .square--doc-half {
    height: 360px;
  }
  .tonk-portals-root .tp-header {
    padding: 8px;
  }
  .tonk-portals-root .square--doc .square__drag-dots {
    right: auto;
    left: 8px;
    top: 8px;
    opacity: 0.8;
  }
  .tonk-portals-root .square--doc .square__name {
    padding-left: 32px;
  }
  /* On mobile the vertical rail would crowd a narrow viewport;
     collapse it back to a horizontal row centered at the bottom. */
  .tonk-portals-root .rail--vertical {
    left: 50%;
    bottom: 8px;
    transform: translateX(-50%);
    flex-direction: row;
    align-items: center;
    max-width: calc(100% - 32px);
    max-height: none;
    overflow-x: auto;
    overflow-y: hidden;
  }
}
`;var s0=j(xt(),1),Qg=["repo","host"],Xf=class extends HTMLElement{static get observedAttributes(){return Qg}root=null;mountNode=null;connectedCallback(){if(this.root)return;Zg();let e=document.createElement("div");e.className="tonk-portals-root",this.appendChild(e),this.mountNode=e,this.root=(0,r0.createRoot)(e),this.render()}disconnectedCallback(){this.root?.unmount(),this.root=null,this.mountNode&&this.mountNode.parentNode===this&&this.removeChild(this.mountNode),this.mountNode=null}attributeChangedCallback(e){this.render()}render(){if(!this.root)return;let e={repo:this.getAttribute("repo")??"",host:this.getAttribute("host")??""};this.root.render((0,s0.jsx)(o0,{...e}))}},f0="tonk-portals-styles";function Zg(){if(document.getElementById(f0))return;let t=document.createElement("style");t.id=f0,t.textContent=c0,document.head.appendChild(t)}customElements.get("tonk-portals")||customElements.define("tonk-portals",Xf);
/*! Bundled license information:

scheduler/cjs/scheduler.production.js:
  (**
   * @license React
   * scheduler.production.js
   *
   * Copyright (c) Meta Platforms, Inc. and affiliates.
   *
   * This source code is licensed under the MIT license found in the
   * LICENSE file in the root directory of this source tree.
   *)

react/cjs/react.production.js:
  (**
   * @license React
   * react.production.js
   *
   * Copyright (c) Meta Platforms, Inc. and affiliates.
   *
   * This source code is licensed under the MIT license found in the
   * LICENSE file in the root directory of this source tree.
   *)

react-dom/cjs/react-dom.production.js:
  (**
   * @license React
   * react-dom.production.js
   *
   * Copyright (c) Meta Platforms, Inc. and affiliates.
   *
   * This source code is licensed under the MIT license found in the
   * LICENSE file in the root directory of this source tree.
   *)

react-dom/cjs/react-dom-client.production.js:
  (**
   * @license React
   * react-dom-client.production.js
   *
   * Copyright (c) Meta Platforms, Inc. and affiliates.
   *
   * This source code is licensed under the MIT license found in the
   * LICENSE file in the root directory of this source tree.
   *)

react/cjs/react-jsx-runtime.production.js:
  (**
   * @license React
   * react-jsx-runtime.production.js
   *
   * Copyright (c) Meta Platforms, Inc. and affiliates.
   *
   * This source code is licensed under the MIT license found in the
   * LICENSE file in the root directory of this source tree.
   *)
*/
//# sourceMappingURL=tonk-portals.js.map
