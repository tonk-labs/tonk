var e = Object.defineProperty,
  t = (e, t) => () => (e && (t = e((e = 0))), t),
  n = (t) => {
    let n = {};
    for (var r in t) e(n, r, { get: t[r], enumerable: !0 });
    return n;
  };
const r = 5e3;
var i = { debug: 0, info: 1, warn: 2, error: 3, none: 4 },
  a = `warn`;
function o() {
  let e = self;
  return e.SW_LOG_LEVEL && e.SW_LOG_LEVEL in i ? e.SW_LOG_LEVEL : a;
}
function s(e) {
  let t = o();
  return i[e] >= i[t];
}
function c(e) {
  return `[SW ${new Date().toISOString()}] ${e.toUpperCase()}:`;
}
function l(e, t, n) {
  if (!s(e)) return;
  let r = c(e),
    i = e === `debug` ? `log` : e;
  n === void 0 ? console[i](r, t) : console[i](r, t, n);
}
const u = {
    debug: (e, t) => l(`debug`, e, t),
    info: (e, t) => l(`info`, e, t),
    warn: (e, t) => l(`warn`, e, t),
    error: (e, t) => l(`error`, e, t),
    setLevel: (e) => {
      let t = self;
      ((t.SW_LOG_LEVEL = e), console.log(`[SW] Log level set to: ${e}`));
    },
    getLevel: () => o(),
  },
  d = `tonk-sw-state-v2`,
  f = `/tonk-state/appSlug`,
  ee = `/tonk-state/bundleBytes`,
  te = `/tonk-state/wsUrl`;
async function ne(e) {
  try {
    let t = await caches.open(d);
    if (e === null) await t.delete(f);
    else {
      let n = new Response(JSON.stringify({ slug: e }), {
        headers: { "Content-Type": `application/json` },
      });
      await t.put(f, n);
    }
    u.debug(`AppSlug persisted to cache`, { slug: e });
  } catch (e) {
    u.error(`Failed to persist appSlug`, {
      error: e instanceof Error ? e.message : String(e),
    });
  }
}
async function re() {
  try {
    let e = await (await caches.open(d)).match(f);
    return (e && (await e.json()).slug) || null;
  } catch (e) {
    return (
      u.error(`Failed to restore appSlug`, {
        error: e instanceof Error ? e.message : String(e),
      }),
      null
    );
  }
}
async function p(e) {
  try {
    let t = await caches.open(d);
    if (e === null) await t.delete(ee);
    else {
      let n = new Blob([e], { type: `application/octet-stream` }),
        r = new Response(n, {
          headers: { "Content-Type": `application/octet-stream` },
        });
      await t.put(ee, r);
    }
    u.debug(`Bundle bytes persisted to cache`, { size: e ? e.length : 0 });
  } catch (e) {
    u.error(`Failed to persist bundle bytes`, {
      error: e instanceof Error ? e.message : String(e),
    });
  }
}
async function ie() {
  try {
    let e = await (await caches.open(d)).match(ee);
    if (!e) return null;
    let t = await e.arrayBuffer();
    return new Uint8Array(t);
  } catch (e) {
    return (
      u.error(`Failed to restore bundle bytes`, {
        error: e instanceof Error ? e.message : String(e),
      }),
      null
    );
  }
}
async function m(e) {
  try {
    let t = await caches.open(d);
    if (e === null) await t.delete(te);
    else {
      let n = new Response(JSON.stringify({ url: e }), {
        headers: { "Content-Type": `application/json` },
      });
      await t.put(te, n);
    }
    u.debug(`WS URL persisted to cache`, { url: e });
  } catch (e) {
    u.error(`Failed to persist WS URL`, {
      error: e instanceof Error ? e.message : String(e),
    });
  }
}
async function ae() {
  try {
    let e = await (await caches.open(d)).match(te);
    return (e && (await e.json()).url) || null;
  } catch (e) {
    return (
      u.error(`Failed to restore WS URL`, {
        error: e instanceof Error ? e.message : String(e),
      }),
      null
    );
  }
}
async function oe() {
  await Promise.all([ne(null), p(null), m(null)]);
}
var h = { status: `idle` },
  se = null;
function g() {
  return h;
}
function ce() {
  return se;
}
function le(e) {
  se = e;
}
function _() {
  return h.status === `active` ? h : null;
}
function v() {
  return h.status === `active` ? { tonk: h.tonk, manifest: h.manifest } : null;
}
function ue(e) {
  (u.debug(`Cleaning up active bundle state`, {
    bundleId: e.bundleId,
    watcherCount: e.watchers.size,
    hasHealthCheck: !!e.healthCheckInterval,
  }),
    e.healthCheckInterval && clearInterval(e.healthCheckInterval),
    e.watchers.forEach((e, t) => {
      try {
        (e.stop(), u.debug(`Stopped watcher`, { id: t }));
      } catch (e) {
        u.warn(`Error stopping watcher`, {
          id: t,
          error: e instanceof Error ? e.message : String(e),
        });
      }
    }));
  try {
    typeof e.tonk.disconnectWebsocket == `function` &&
      (e.tonk.disconnectWebsocket(), u.debug(`Disconnected WebSocket`));
  } catch (e) {
    u.warn(`Error disconnecting WebSocket`, {
      error: e instanceof Error ? e.message : String(e),
    });
  }
}
function y(e) {
  let t = h;
  (u.debug(`State transition`, {
    from: t.status,
    to: e.status,
    bundleId: `bundleId` in e ? e.bundleId : void 0,
  }),
    t.status === `active` && ue(t),
    (h = e));
}
function de(e) {
  return h.status === `active` ? ((h = { ...h, appSlug: e }), !0) : !1;
}
function fe(e) {
  h.status === `active` && (h = { ...h, connectionHealthy: e });
}
function pe() {
  return h.status === `active`
    ? ((h = { ...h, reconnectAttempts: h.reconnectAttempts + 1 }),
      h.reconnectAttempts)
    : 0;
}
function me() {
  h.status === `active` && (h = { ...h, reconnectAttempts: 0 });
}
function he(e) {
  h.status === `active` && (h = { ...h, healthCheckInterval: e });
}
function ge(e, t) {
  h.status === `active` && h.watchers.set(e, t);
}
function _e(e) {
  if (h.status === `active`) {
    let t = h.watchers.get(e);
    if (t) {
      try {
        t.stop();
      } catch (t) {
        u.warn(`Error stopping watcher on remove`, {
          id: e,
          error: t instanceof Error ? t.message : String(t),
        });
      }
      return (h.watchers.delete(e), !0);
    }
  }
  return !1;
}
function ve(e) {
  if (h.status === `active`) return h.watchers.get(e);
}
function ye() {
  return h.status === `active` ? Array.from(h.watchers.entries()) : [];
}
function be(e, t) {
  if (
    (console.log(`determinePath START`, {
      url: e.href,
      pathname: e.pathname,
      spaceName: t || `none`,
    }),
    !t)
  )
    throw (
      console.error(`determinePath - NO SPACE NAME SET`),
      Error(`No space name available for ${e.pathname}`)
    );
  let n = new URL(self.registration?.scope ?? self.location.href).pathname,
    r = e.pathname.startsWith(n) ? e.pathname.slice(n.length) : e.pathname,
    i = r.replace(/^\/+/, ``).split(`/`).filter(Boolean);
  console.log(`determinePath - segments`, {
    scopePath: n,
    strippedPath: r,
    segments: [...i],
    firstSegment: i[0] || `none`,
  });
  let a = i;
  if (
    (i[0] === t
      ? ((a = i.slice(1)),
        console.log(
          `determinePath - spaceName already present, using remaining segments`,
          { pathSegments: [...a] },
        ))
      : console.log(
          `determinePath - spaceName not present, using all segments as path`,
          { pathSegments: [...a] },
        ),
    a.length === 0 || e.pathname.endsWith(`/`))
  ) {
    let e = `${t}/index.html`;
    return (
      console.log(`determinePath - defaulting to index.html`, { result: e }),
      e
    );
  }
  let o = `${t}/${a.join(`/`)}`;
  return (console.log(`determinePath - returning file path`, { result: o }), o);
}
async function b(e) {
  (l(`info`, `Posting response to main thread`, {
    type: e.type,
    success: `success` in e ? e.success : `N/A`,
  }),
    (await self.clients.matchAll()).forEach((t) => {
      t.postMessage(e);
    }));
}
async function xe(e) {
  if (e.bytes) {
    let t = atob(e.bytes),
      n = new Uint8Array(t.length);
    for (let e = 0; e < t.length; e++) n[e] = t.charCodeAt(e);
    return new Response(n, {
      headers: { "Content-Type": e.content.mime || `application/octet-stream` },
    });
  } else
    return new Response(e.content, {
      headers: { "Content-Type": `application/json` },
    });
}
function Se(e) {
  let t = new URL(e.request.url),
    n = e.request.referrer,
    r = g(),
    i = _()?.appSlug || null;
  u.debug(`Fetch event`, {
    url: t.href,
    pathname: t.pathname,
    state: r.status,
    appSlug: i,
  });
  let a = e.request.headers.get(`upgrade`);
  if (a && a.toLowerCase() === `websocket`) {
    u.debug(`WebSocket upgrade request - passing through`, { url: t.href });
    return;
  }
  let o =
    t.pathname === `/` ||
    t.pathname === `` ||
    t.pathname === `/space/` ||
    t.pathname === `/space`;
  o &&
    i &&
    Promise.all([ne(null), p(null)]).catch((e) => {
      u.error(`Failed to persist state reset`, { error: e });
    });
  let s = t.pathname.startsWith(`/space/_runtime/`),
    c = t.searchParams.has(`bundleId`),
    l = [
      `/space/_runtime/index.html`,
      `/space/_runtime/main.js`,
      `/space/_runtime/main.css`,
      `/space/service-worker-bundled.js`,
    ].some((e) => t.pathname === e),
    d =
      t.pathname.startsWith(`/space/_runtime/`) &&
      [`.otf`, `.ttf`, `.woff`, `.woff2`, `.eot`].some((e) =>
        t.pathname.endsWith(e),
      );
  if (l || (s && (c || d))) {
    u.debug(`Runtime static asset - passing through`, { pathname: t.pathname });
    return;
  }
  let f = t.pathname.match(/^\/space\/([^\/]+)/)?.[1];
  if (f === `_runtime`) {
    u.debug(`Reserved _runtime path - passing through`, {
      pathname: t.pathname,
    });
    return;
  }
  if (i && !o) {
    if (
      t.pathname.startsWith(`/@vite`) ||
      t.pathname.startsWith(`/@react-refresh`) ||
      t.pathname.startsWith(`/src/`) ||
      t.pathname.startsWith(`/${i}/@vite`) ||
      t.pathname.startsWith(`/${i}/node_modules`) ||
      t.pathname.startsWith(`/${i}/src/`) ||
      t.pathname.startsWith(`/node_modules`) ||
      t.pathname.includes(`__vite__`) ||
      t.searchParams.has(`t`)
    ) {
      (u.debug(`Vite HMR asset - proxying to dev server`, {
        pathname: t.pathname,
      }),
        e.respondWith(
          (async () => {
            try {
              let e = `http://localhost:4001${t.pathname}${t.search}`,
                n = await fetch(e),
                r = new Headers(n.headers);
              return (
                r.set(`Cache-Control`, `no-cache, no-store, must-revalidate`),
                r.set(`Pragma`, `no-cache`),
                r.set(`Expires`, `0`),
                new Response(n.body, {
                  status: n.status,
                  statusText: n.statusText,
                  headers: r,
                })
              );
            } catch (e) {
              return (
                u.error(`Failed to fetch Vite asset`, {
                  error: e instanceof Error ? e.message : String(e),
                  pathname: t.pathname,
                }),
                new Response(`Vite dev server unreachable`, {
                  status: 502,
                  headers: { "Content-Type": `text/plain` },
                })
              );
            }
          })(),
        ));
      return;
    }
    (u.debug(`TONK_SERVE_LOCAL - proxying to dev server`, {
      pathname: t.pathname,
    }),
      e.respondWith(
        (async () => {
          try {
            let e = `http://localhost:4001${t.pathname}${t.search}`,
              n = await fetch(e),
              r = new Headers(n.headers);
            return (
              r.set(`Cache-Control`, `no-cache, no-store, must-revalidate`),
              r.set(`Pragma`, `no-cache`),
              r.set(`Expires`, `0`),
              new Response(n.body, {
                status: n.status,
                statusText: n.statusText,
                headers: r,
              })
            );
          } catch (e) {
            return (
              u.error(`Failed to fetch from local dev server`, {
                error: e instanceof Error ? e.message : String(e),
                pathname: t.pathname,
              }),
              new Response(`Local dev server unreachable on port 4001`, {
                status: 502,
                headers: { "Content-Type": `text/plain` },
              })
            );
          }
        })(),
      ));
    return;
  }
  f && t.origin === location.origin && !o
    ? (u.debug(`Processing VFS request`, {
        pathname: t.pathname,
        spaceName: f,
        referrer: n,
      }),
      e.respondWith(
        (async () => {
          try {
            let e = be(t, f);
            u.debug(`Resolved VFS path`, {
              path: e,
              url: t.pathname,
              spaceName: f,
            });
            let n = ce();
            if (n && g().status !== `active`) {
              u.debug(`Waiting for initialization`, { status: g().status });
              try {
                await Promise.race([
                  n,
                  new Promise((e, t) =>
                    setTimeout(() => t(Error(`Initialization timeout`)), 15e3),
                  ),
                ]);
              } catch (e) {
                u.warn(`Initialization wait failed or timed out`, {
                  error: e instanceof Error ? e.message : String(e),
                });
              }
            }
            let r = _();
            if (!r)
              throw (
                u.error(`Tonk not initialized - cannot handle request`, {
                  status: g().status,
                  path: e,
                }),
                Error(`Tonk not initialized`)
              );
            let i = `/${e}`;
            if (!(await r.tonk.exists(i))) {
              u.debug(`File not found, falling back to index.html`, {
                path: i,
              });
              let e = `/${f}/index.html`,
                t = await r.tonk.readFile(e);
              return xe(t);
            }
            u.debug(`Serving file from VFS`, { path: i });
            let a = await r.tonk.readFile(i);
            return await xe(a);
          } catch (n) {
            return (
              u.error(`Failed to fetch from VFS`, {
                error: n instanceof Error ? n.message : String(n),
                url: t.href,
                status: g().status,
              }),
              u.debug(`Falling back to network request`, { url: t.href }),
              fetch(e.request)
            );
          }
        })(),
      ))
    : u.debug(`Ignoring fetch - not a /space/<space-name>/ request`, {
        requestOrigin: t.origin,
        swOrigin: location.origin,
        spaceName: f || `none`,
        isRoot: o,
      });
}
var Ce = `modulepreload`,
  we = function (e) {
    return `/` + e;
  },
  Te = {};
const x = function (e, t, n) {
  let r = Promise.resolve();
  if (t && t.length > 0) {
    let e = document.getElementsByTagName(`link`),
      i = document.querySelector(`meta[property=csp-nonce]`),
      a = i?.nonce || i?.getAttribute(`nonce`);
    function o(e) {
      return Promise.all(
        e.map((e) =>
          Promise.resolve(e).then(
            (e) => ({ status: `fulfilled`, value: e }),
            (e) => ({ status: `rejected`, reason: e }),
          ),
        ),
      );
    }
    r = o(
      t.map((t) => {
        if (((t = we(t, n)), t in Te)) return;
        Te[t] = !0;
        let r = t.endsWith(`.css`),
          i = r ? `[rel="stylesheet"]` : ``;
        if (n)
          for (let n = e.length - 1; n >= 0; n--) {
            let i = e[n];
            if (i.href === t && (!r || i.rel === `stylesheet`)) return;
          }
        else if (document.querySelector(`link[href="${t}"]${i}`)) return;
        let o = document.createElement(`link`);
        if (
          ((o.rel = r ? `stylesheet` : Ce),
          r || (o.as = `script`),
          (o.crossOrigin = ``),
          (o.href = t),
          a && o.setAttribute(`nonce`, a),
          document.head.appendChild(o),
          r)
        )
          return new Promise((e, n) => {
            (o.addEventListener(`load`, e),
              o.addEventListener(`error`, () =>
                n(Error(`Unable to preload CSS for ${t}`)),
              ));
          });
      }),
    );
  }
  function i(e) {
    let t = new Event(`vite:preloadError`, { cancelable: !0 });
    if (((t.payload = e), self.dispatchEvent(t), !t.defaultPrevented)) throw e;
  }
  return r.then((t) => {
    for (let e of t || []) e.status === `rejected` && i(e.reason);
    return e().catch(i);
  });
};
var S = n({
  WasmBundle: () => z,
  WasmDocHandle: () => st,
  WasmDocumentWatcher: () => lt,
  WasmError: () => dt,
  WasmRepo: () => pt,
  WasmTonkCore: () => ht,
  WasmWebSocketHandle: () => _t,
  create_bundle_from_bytes: () => Pe,
  create_tonk: () => Be,
  create_tonk_from_bundle: () => Le,
  create_tonk_from_bundle_with_storage: () => je,
  create_tonk_from_bytes: () => Fe,
  create_tonk_from_bytes_with_storage: () => ze,
  create_tonk_with_config: () => Me,
  create_tonk_with_peer_id: () => Re,
  create_tonk_with_storage: () => Ne,
  default: () => B,
  init: () => Ie,
  initSync: () => Ze,
  set_time_provider: () => Ve,
});
function C() {
  return (
    (N === null || N.byteLength === 0) && (N = new Uint8Array(M.memory.buffer)),
    N
  );
}
function Ee(e, t) {
  return (
    (F += t),
    F >= $e &&
      ((P =
        typeof TextDecoder < `u`
          ? new TextDecoder(`utf-8`, { ignoreBOM: !0, fatal: !0 })
          : {
              decode: () => {
                throw Error(`TextDecoder not available`);
              },
            }),
      P.decode(),
      (F = t)),
    P.decode(C().subarray(e, e + t))
  );
}
function w(e, t) {
  return ((e >>>= 0), Ee(e, t));
}
function T(e, t, n) {
  if (n === void 0) {
    let n = L.encode(e),
      r = t(n.length, 1) >>> 0;
    return (
      C()
        .subarray(r, r + n.length)
        .set(n),
      (I = n.length),
      r
    );
  }
  let r = e.length,
    i = t(r, 1) >>> 0,
    a = C(),
    o = 0;
  for (; o < r; o++) {
    let t = e.charCodeAt(o);
    if (t > 127) break;
    a[i + o] = t;
  }
  if (o !== r) {
    (o !== 0 && (e = e.slice(o)),
      (i = n(i, r, (r = o + e.length * 3), 1) >>> 0));
    let t = C().subarray(i + o, i + r),
      a = et(e, t);
    ((o += a.written), (i = n(i, r, o, 1) >>> 0));
  }
  return ((I = o), i);
}
function E() {
  return (
    (R === null ||
      R.buffer.detached === !0 ||
      (R.buffer.detached === void 0 && R.buffer !== M.memory.buffer)) &&
      (R = new DataView(M.memory.buffer)),
    R
  );
}
function D(e) {
  let t = M.__externref_table_alloc();
  return (M.__wbindgen_export_4.set(t, e), t);
}
function O(e, t) {
  try {
    return e.apply(this, t);
  } catch (e) {
    let t = D(e);
    M.__wbindgen_exn_store(t);
  }
}
function k(e) {
  return e == null;
}
function De(e, t) {
  return ((e >>>= 0), C().subarray(e / 1, e / 1 + t));
}
function A(e, t, n, r) {
  let i = { a: e, b: t, cnt: 1, dtor: n },
    a = (...e) => {
      i.cnt++;
      let t = i.a;
      i.a = 0;
      try {
        return r(t, i.b, ...e);
      } finally {
        --i.cnt === 0
          ? (M.__wbindgen_export_6.get(i.dtor)(t, i.b), tt.unregister(i))
          : (i.a = t);
      }
    };
  return ((a.original = i), tt.register(a, i, i), a);
}
function Oe(e) {
  let t = typeof e;
  if (t == `number` || t == `boolean` || e == null) return `${e}`;
  if (t == `string`) return `"${e}"`;
  if (t == `symbol`) {
    let t = e.description;
    return t == null ? `Symbol` : `Symbol(${t})`;
  }
  if (t == `function`) {
    let t = e.name;
    return typeof t == `string` && t.length > 0 ? `Function(${t})` : `Function`;
  }
  if (Array.isArray(e)) {
    let t = e.length,
      n = `[`;
    t > 0 && (n += Oe(e[0]));
    for (let r = 1; r < t; r++) n += `, ` + Oe(e[r]);
    return ((n += `]`), n);
  }
  let n = /\[object ([^\]]+)\]/.exec(toString.call(e)),
    r;
  if (n && n.length > 1) r = n[1];
  else return toString.call(e);
  if (r == `Object`)
    try {
      return `Object(` + JSON.stringify(e) + `)`;
    } catch {
      return `Object`;
    }
  return e instanceof Error ? `${e.name}: ${e.message}\n${e.stack}` : r;
}
function j(e) {
  let t = M.__wbindgen_export_4.get(e);
  return (M.__externref_table_dealloc(e), t);
}
function ke(e, t) {
  if (!(e instanceof t)) throw Error(`expected instance of ${t.name}`);
}
function Ae(e, t) {
  let n = t(e.length * 1, 1) >>> 0;
  return (C().set(e, n / 1), (I = e.length), n);
}
function je(e, t) {
  return (ke(e, z), M.create_tonk_from_bundle_with_storage(e.__wbg_ptr, t));
}
function Me(e, t) {
  let n = T(e, M.__wbindgen_malloc, M.__wbindgen_realloc),
    r = I;
  return M.create_tonk_with_config(n, r, t);
}
function Ne(e) {
  return M.create_tonk_with_storage(e);
}
function Pe(e) {
  let t = M.create_bundle_from_bytes(e);
  if (t[2]) throw j(t[1]);
  return z.__wrap(t[0]);
}
function Fe(e) {
  return M.create_tonk_from_bytes(e);
}
function Ie() {
  M.init();
}
function Le(e) {
  return (ke(e, z), M.create_tonk_from_bundle(e.__wbg_ptr));
}
function Re(e) {
  let t = T(e, M.__wbindgen_malloc, M.__wbindgen_realloc),
    n = I;
  return M.create_tonk_with_peer_id(t, n);
}
function ze(e, t) {
  return M.create_tonk_from_bytes_with_storage(e, t);
}
function Be() {
  return M.create_tonk();
}
function Ve(e) {
  M.set_time_provider(e);
}
function He(e, t, n) {
  M.closure729_externref_shim(e, t, n);
}
function Ue(e, t, n) {
  let r = M.closure732_externref_shim_multivalue_shim(e, t, n);
  if (r[1]) throw j(r[0]);
}
function We(e, t) {
  M.wasm_bindgen__convert__closures_____invoke__h546e6a644418f5b7(e, t);
}
function Ge(e, t, n) {
  M.closure1010_externref_shim(e, t, n);
}
function Ke(e, t, n) {
  M.closure1026_externref_shim(e, t, n);
}
function qe(e, t, n, r) {
  M.closure1284_externref_shim(e, t, n, r);
}
async function Je(e, t) {
  if (typeof Response == `function` && e instanceof Response) {
    if (typeof WebAssembly.instantiateStreaming == `function`)
      try {
        return await WebAssembly.instantiateStreaming(e, t);
      } catch (t) {
        if (
          e.ok &&
          vt.has(e.type) &&
          e.headers.get(`Content-Type`) !== `application/wasm`
        )
          console.warn(
            "`WebAssembly.instantiateStreaming` failed because your server does not serve Wasm with `application/wasm` MIME type. Falling back to `WebAssembly.instantiate` which is slower. Original error:\n",
            t,
          );
        else throw t;
      }
    let n = await e.arrayBuffer();
    return await WebAssembly.instantiate(n, t);
  } else {
    let n = await WebAssembly.instantiate(e, t);
    return n instanceof WebAssembly.Instance ? { instance: n, module: e } : n;
  }
}
function Ye() {
  let e = {};
  return (
    (e.wbg = {}),
    (e.wbg.__wbg_Error_0497d5bdba9362e5 = function (e, t) {
      return Error(w(e, t));
    }),
    (e.wbg.__wbg_String_8f0eb39a4a4c2f66 = function (e, t) {
      let n = T(String(t), M.__wbindgen_malloc, M.__wbindgen_realloc),
        r = I;
      (E().setInt32(e + 4, r, !0), E().setInt32(e + 0, n, !0));
    }),
    (e.wbg.__wbg_Window_41559019033ede94 = function (e) {
      return e.Window;
    }),
    (e.wbg.__wbg_WorkerGlobalScope_d324bffbeaef9f3a = function (e) {
      return e.WorkerGlobalScope;
    }),
    (e.wbg.__wbg_abort_601b12a63a2f3a3a = function () {
      return O(function (e) {
        e.abort();
      }, arguments);
    }),
    (e.wbg.__wbg_bound_eb572b424befade3 = function () {
      return O(function (e, t, n, r) {
        return IDBKeyRange.bound(e, t, n !== 0, r !== 0);
      }, arguments);
    }),
    (e.wbg.__wbg_buffer_a1a27a0dfa70165d = function (e) {
      return e.buffer;
    }),
    (e.wbg.__wbg_buffer_e495ba54cee589cc = function (e) {
      return e.buffer;
    }),
    (e.wbg.__wbg_call_f2db6205e5c51dc8 = function () {
      return O(function (e, t, n) {
        return e.call(t, n);
      }, arguments);
    }),
    (e.wbg.__wbg_call_fbe8be8bf6436ce5 = function () {
      return O(function (e, t) {
        return e.call(t);
      }, arguments);
    }),
    (e.wbg.__wbg_close_b08c03c920ee0bba = function () {
      return O(function (e) {
        e.close();
      }, arguments);
    }),
    (e.wbg.__wbg_commit_a54edce65f3858f2 = function () {
      return O(function (e) {
        e.commit();
      }, arguments);
    }),
    (e.wbg.__wbg_continue_7d9cdafc888cb902 = function () {
      return O(function (e) {
        e.continue();
      }, arguments);
    }),
    (e.wbg.__wbg_createObjectStore_b1f08961900155dd = function () {
      return O(function (e, t, n) {
        return e.createObjectStore(w(t, n));
      }, arguments);
    }),
    (e.wbg.__wbg_data_fffd43bf0ca75fff = function (e) {
      return e.data;
    }),
    (e.wbg.__wbg_delete_71b7921c73aa9378 = function () {
      return O(function (e, t) {
        return e.delete(t);
      }, arguments);
    }),
    (e.wbg.__wbg_done_4d01f352bade43b7 = function (e) {
      return e.done;
    }),
    (e.wbg.__wbg_entries_41651c850143b957 = function (e) {
      return Object.entries(e);
    }),
    (e.wbg.__wbg_error_31d7961b8e0d3f6c = function (e, t) {
      console.error(w(e, t));
    }),
    (e.wbg.__wbg_error_4e978abc9692c0c5 = function () {
      return O(function (e) {
        let t = e.error;
        return k(t) ? 0 : D(t);
      }, arguments);
    }),
    (e.wbg.__wbg_error_51ecdd39ec054205 = function (e) {
      console.error(e);
    }),
    (e.wbg.__wbg_error_7534b8e9a36f1ab4 = function (e, t) {
      let n, r;
      try {
        ((n = e), (r = t), console.error(w(e, t)));
      } finally {
        M.__wbindgen_free(n, r, 1);
      }
    }),
    (e.wbg.__wbg_from_12ff8e47307bd4c7 = function (e) {
      return Array.from(e);
    }),
    (e.wbg.__wbg_getRandomValues_1c61fac11405ffdc = function () {
      return O(function (e, t) {
        globalThis.crypto.getRandomValues(De(e, t));
      }, arguments);
    }),
    (e.wbg.__wbg_getTime_2afe67905d873e92 = function (e) {
      return e.getTime();
    }),
    (e.wbg.__wbg_get_92470be87867c2e5 = function () {
      return O(function (e, t) {
        return Reflect.get(e, t);
      }, arguments);
    }),
    (e.wbg.__wbg_get_a131a44bd1eb6979 = function (e, t) {
      return e[t >>> 0];
    }),
    (e.wbg.__wbg_get_d37904b955701f99 = function () {
      return O(function (e, t) {
        return e.get(t);
      }, arguments);
    }),
    (e.wbg.__wbg_getwithrefkey_1dc361bd10053bfe = function (e, t) {
      return e[t];
    }),
    (e.wbg.__wbg_global_f5c2926e57ba457f = function (e) {
      return e.global;
    }),
    (e.wbg.__wbg_indexedDB_317016dcb8a872d6 = function () {
      return O(function (e) {
        let t = e.indexedDB;
        return k(t) ? 0 : D(t);
      }, arguments);
    }),
    (e.wbg.__wbg_indexedDB_54f01430b1e194e8 = function () {
      return O(function (e) {
        let t = e.indexedDB;
        return k(t) ? 0 : D(t);
      }, arguments);
    }),
    (e.wbg.__wbg_indexedDB_63b82e158eb67cbd = function () {
      return O(function (e) {
        let t = e.indexedDB;
        return k(t) ? 0 : D(t);
      }, arguments);
    }),
    (e.wbg.__wbg_instanceof_ArrayBuffer_a8b6f580b363f2bc = function (e) {
      let t;
      try {
        t = e instanceof ArrayBuffer;
      } catch {
        t = !1;
      }
      return t;
    }),
    (e.wbg.__wbg_instanceof_CursorSys_4b6a8aba0e823e75 = function (e) {
      let t;
      try {
        t = e instanceof IDBCursorWithValue;
      } catch {
        t = !1;
      }
      return t;
    }),
    (e.wbg.__wbg_instanceof_DomException_77720ed8752d7409 = function (e) {
      let t;
      try {
        t = e instanceof DOMException;
      } catch {
        t = !1;
      }
      return t;
    }),
    (e.wbg.__wbg_instanceof_Error_58a92d81483a4b16 = function (e) {
      let t;
      try {
        t = e instanceof Error;
      } catch {
        t = !1;
      }
      return t;
    }),
    (e.wbg.__wbg_instanceof_IdbCursor_07be219dc9364535 = function (e) {
      let t;
      try {
        t = e instanceof IDBCursor;
      } catch {
        t = !1;
      }
      return t;
    }),
    (e.wbg.__wbg_instanceof_IdbDatabase_0ed56ed115d533bc = function (e) {
      let t;
      try {
        t = e instanceof IDBDatabase;
      } catch {
        t = !1;
      }
      return t;
    }),
    (e.wbg.__wbg_instanceof_IdbRequest_c4498c7b5a3a0fa3 = function (e) {
      let t;
      try {
        t = e instanceof IDBRequest;
      } catch {
        t = !1;
      }
      return t;
    }),
    (e.wbg.__wbg_instanceof_Map_80cc65041c96417a = function (e) {
      let t;
      try {
        t = e instanceof Map;
      } catch {
        t = !1;
      }
      return t;
    }),
    (e.wbg.__wbg_instanceof_Uint8Array_ca460677bc155827 = function (e) {
      let t;
      try {
        t = e instanceof Uint8Array;
      } catch {
        t = !1;
      }
      return t;
    }),
    (e.wbg.__wbg_isArray_5f090bed72bd4f89 = function (e) {
      return Array.isArray(e);
    }),
    (e.wbg.__wbg_isSafeInteger_90d7c4674047d684 = function (e) {
      return Number.isSafeInteger(e);
    }),
    (e.wbg.__wbg_item_15285ca2d766f142 = function (e, t, n) {
      let r = t.item(n >>> 0);
      var i = k(r) ? 0 : T(r, M.__wbindgen_malloc, M.__wbindgen_realloc),
        a = I;
      (E().setInt32(e + 4, a, !0), E().setInt32(e + 0, i, !0));
    }),
    (e.wbg.__wbg_iterator_4068add5b2aef7a6 = function () {
      return Symbol.iterator;
    }),
    (e.wbg.__wbg_key_a17a68df9ec1b180 = function () {
      return O(function (e) {
        return e.key;
      }, arguments);
    }),
    (e.wbg.__wbg_keys_42062809bf87339e = function (e) {
      return Object.keys(e);
    }),
    (e.wbg.__wbg_length_ab6d22b5ead75c72 = function (e) {
      return e.length;
    }),
    (e.wbg.__wbg_length_f00ec12454a5d9fd = function (e) {
      return e.length;
    }),
    (e.wbg.__wbg_log_0cc1b7768397bcfe = function (e, t, n, r, i, a, o, s) {
      let c, l;
      try {
        ((c = e), (l = t), console.log(w(e, t), w(n, r), w(i, a), w(o, s)));
      } finally {
        M.__wbindgen_free(c, l, 1);
      }
    }),
    (e.wbg.__wbg_log_cb9e190acc5753fb = function (e, t) {
      let n, r;
      try {
        ((n = e), (r = t), console.log(w(e, t)));
      } finally {
        M.__wbindgen_free(n, r, 1);
      }
    }),
    (e.wbg.__wbg_log_ea240990d83e374e = function (e) {
      console.log(e);
    }),
    (e.wbg.__wbg_lowerBound_13c8e875a3fb9f7d = function () {
      return O(function (e, t) {
        return IDBKeyRange.lowerBound(e, t !== 0);
      }, arguments);
    }),
    (e.wbg.__wbg_mark_7438147ce31e9d4b = function (e, t) {
      performance.mark(w(e, t));
    }),
    (e.wbg.__wbg_measure_fb7825c11612c823 = function () {
      return O(function (e, t, n, r) {
        let i, a, o, s;
        try {
          ((i = e),
            (a = t),
            (o = n),
            (s = r),
            performance.measure(w(e, t), w(n, r)));
        } finally {
          (M.__wbindgen_free(i, a, 1), M.__wbindgen_free(o, s, 1));
        }
      }, arguments);
    }),
    (e.wbg.__wbg_message_2d95ea5aff0d63b9 = function (e, t) {
      let n = t.message,
        r = T(n, M.__wbindgen_malloc, M.__wbindgen_realloc),
        i = I;
      (E().setInt32(e + 4, i, !0), E().setInt32(e + 0, r, !0));
    }),
    (e.wbg.__wbg_message_4159c15dac08c5e9 = function (e) {
      return e.message;
    }),
    (e.wbg.__wbg_message_44ef9b801b7d8bc3 = function (e, t) {
      let n = t.message,
        r = T(n, M.__wbindgen_malloc, M.__wbindgen_realloc),
        i = I;
      (E().setInt32(e + 4, i, !0), E().setInt32(e + 0, r, !0));
    }),
    (e.wbg.__wbg_name_2acff1e83d9735f9 = function (e, t) {
      let n = t.name,
        r = T(n, M.__wbindgen_malloc, M.__wbindgen_realloc),
        i = I;
      (E().setInt32(e + 4, i, !0), E().setInt32(e + 0, r, !0));
    }),
    (e.wbg.__wbg_new0_97314565408dea38 = function () {
      return new Date();
    }),
    (e.wbg.__wbg_new_07b483f72211fd66 = function () {
      return {};
    }),
    (e.wbg.__wbg_new_476169e6d59f23ae = function (e, t) {
      return Error(w(e, t));
    }),
    (e.wbg.__wbg_new_58353953ad2097cc = function () {
      return [];
    }),
    (e.wbg.__wbg_new_8a6f238a6ece86ea = function () {
      return Error();
    }),
    (e.wbg.__wbg_new_a979b4b45bd55c7f = function () {
      return new Map();
    }),
    (e.wbg.__wbg_new_e30c39c06edaabf2 = function (e, t) {
      try {
        var n = { a: e, b: t };
        return new Promise((e, t) => {
          let r = n.a;
          n.a = 0;
          try {
            return qe(r, n.b, e, t);
          } finally {
            n.a = r;
          }
        });
      } finally {
        n.a = n.b = 0;
      }
    }),
    (e.wbg.__wbg_new_e52b3efaaa774f96 = function (e) {
      return new Uint8Array(e);
    }),
    (e.wbg.__wbg_new_f42a001532528172 = function () {
      return O(function (e, t) {
        return new WebSocket(w(e, t));
      }, arguments);
    }),
    (e.wbg.__wbg_newfromslice_7c05ab1297cb2d88 = function (e, t) {
      return new Uint8Array(De(e, t));
    }),
    (e.wbg.__wbg_newnoargs_ff528e72d35de39a = function (e, t) {
      return Function(w(e, t));
    }),
    (e.wbg.__wbg_newwithbyteoffsetandlength_3b01ecda099177e8 = function (
      e,
      t,
      n,
    ) {
      return new Uint8Array(e, t >>> 0, n >>> 0);
    }),
    (e.wbg.__wbg_newwithlength_08f872dc1e3ada2e = function (e) {
      return new Uint8Array(e >>> 0);
    }),
    (e.wbg.__wbg_next_8bb824d217961b5d = function (e) {
      return e.next;
    }),
    (e.wbg.__wbg_next_e2da48d8fff7439a = function () {
      return O(function (e) {
        return e.next();
      }, arguments);
    }),
    (e.wbg.__wbg_now_eb0821f3bd9f6529 = function () {
      return Date.now();
    }),
    (e.wbg.__wbg_objectStoreNames_e82275eb2d403a92 = function (e) {
      return e.objectStoreNames;
    }),
    (e.wbg.__wbg_objectStore_b463d32c86d6b543 = function () {
      return O(function (e, t, n) {
        return e.objectStore(w(t, n));
      }, arguments);
    }),
    (e.wbg.__wbg_openCursor_7c13a2cd32c6258b = function () {
      return O(function (e) {
        return e.openCursor();
      }, arguments);
    }),
    (e.wbg.__wbg_open_0f04f50fa4d98f67 = function () {
      return O(function (e, t, n, r) {
        return e.open(w(t, n), r >>> 0);
      }, arguments);
    }),
    (e.wbg.__wbg_push_73fd7b5550ebf707 = function (e, t) {
      return e.push(t);
    }),
    (e.wbg.__wbg_put_7f0b4dcc666f09e3 = function () {
      return O(function (e, t, n) {
        return e.put(t, n);
      }, arguments);
    }),
    (e.wbg.__wbg_queueMicrotask_46c1df247678729f = function (e) {
      queueMicrotask(e);
    }),
    (e.wbg.__wbg_queueMicrotask_8acf3ccb75ed8d11 = function (e) {
      return e.queueMicrotask;
    }),
    (e.wbg.__wbg_readyState_249e5707a38b7a7a = function (e) {
      let t = e.readyState;
      return (rt.indexOf(t) + 1 || 3) - 1;
    }),
    (e.wbg.__wbg_request_5079471e06223120 = function (e) {
      return e.request;
    }),
    (e.wbg.__wbg_request_fada8c23b78b3a02 = function (e) {
      return e.request;
    }),
    (e.wbg.__wbg_resolve_0dac8c580ffd4678 = function (e) {
      return Promise.resolve(e);
    }),
    (e.wbg.__wbg_result_a0f1bf2fe64a516c = function () {
      return O(function (e) {
        return e.result;
      }, arguments);
    }),
    (e.wbg.__wbg_send_05456d2bf190b017 = function () {
      return O(function (e, t) {
        e.send(t);
      }, arguments);
    }),
    (e.wbg.__wbg_set_3f1d0b984ed272ed = function (e, t, n) {
      e[t] = n;
    }),
    (e.wbg.__wbg_set_7422acbe992d64ab = function (e, t, n) {
      e[t >>> 0] = n;
    }),
    (e.wbg.__wbg_set_c43293f93a35998a = function () {
      return O(function (e, t, n) {
        return Reflect.set(e, t, n);
      }, arguments);
    }),
    (e.wbg.__wbg_set_d6bdfd275fb8a4ce = function (e, t, n) {
      return e.set(t, n);
    }),
    (e.wbg.__wbg_set_fe4e79d1ed3b0e9b = function (e, t, n) {
      e.set(t, n >>> 0);
    }),
    (e.wbg.__wbg_setbinaryType_52787d6025601cc5 = function (e, t) {
      e.binaryType = nt[t];
    }),
    (e.wbg.__wbg_setonabort_479ebb5884fcb171 = function (e, t) {
      e.onabort = t;
    }),
    (e.wbg.__wbg_setonclose_c6db38f935250174 = function (e, t) {
      e.onclose = t;
    }),
    (e.wbg.__wbg_setoncomplete_27bdbca012e45c05 = function (e, t) {
      e.oncomplete = t;
    }),
    (e.wbg.__wbg_setonerror_537b68f474e27d4e = function (e, t) {
      e.onerror = t;
    }),
    (e.wbg.__wbg_setonerror_ab02451cd01cb480 = function (e, t) {
      e.onerror = t;
    }),
    (e.wbg.__wbg_setonerror_ce5c4d34aed931bb = function (e, t) {
      e.onerror = t;
    }),
    (e.wbg.__wbg_setonmessage_49ca623a77cfb3e6 = function (e, t) {
      e.onmessage = t;
    }),
    (e.wbg.__wbg_setonopen_1475cbeb761c101f = function (e, t) {
      e.onopen = t;
    }),
    (e.wbg.__wbg_setonsuccess_0b2b45bd8cc13b95 = function (e, t) {
      e.onsuccess = t;
    }),
    (e.wbg.__wbg_setonupgradeneeded_be2e0ae927917f82 = function (e, t) {
      e.onupgradeneeded = t;
    }),
    (e.wbg.__wbg_stack_0ed75d68575b0f3c = function (e, t) {
      let n = t.stack,
        r = T(n, M.__wbindgen_malloc, M.__wbindgen_realloc),
        i = I;
      (E().setInt32(e + 4, i, !0), E().setInt32(e + 0, r, !0));
    }),
    (e.wbg.__wbg_static_accessor_GLOBAL_487c52c58d65314d = function () {
      let e = typeof globalThis > `u` ? null : globalThis;
      return k(e) ? 0 : D(e);
    }),
    (e.wbg.__wbg_static_accessor_GLOBAL_THIS_ee9704f328b6b291 = function () {
      let e = typeof globalThis > `u` ? null : globalThis;
      return k(e) ? 0 : D(e);
    }),
    (e.wbg.__wbg_static_accessor_SELF_78c9e3071b912620 = function () {
      let e = typeof self > `u` ? null : self;
      return k(e) ? 0 : D(e);
    }),
    (e.wbg.__wbg_static_accessor_WINDOW_a093d21393777366 = function () {
      let e = typeof self > `u` ? null : self;
      return k(e) ? 0 : D(e);
    }),
    (e.wbg.__wbg_target_15f1da583855ac4e = function (e) {
      let t = e.target;
      return k(t) ? 0 : D(t);
    }),
    (e.wbg.__wbg_then_82ab9fb4080f1707 = function (e, t, n) {
      return e.then(t, n);
    }),
    (e.wbg.__wbg_then_db882932c0c714c6 = function (e, t) {
      return e.then(t);
    }),
    (e.wbg.__wbg_toString_bc7a05a172b5cf14 = function (e) {
      return e.toString();
    }),
    (e.wbg.__wbg_transaction_36c8b28ed4349a9a = function () {
      return O(function (e, t, n, r) {
        return e.transaction(w(t, n), it[r]);
      }, arguments);
    }),
    (e.wbg.__wbg_transaction_d1f21f4378880521 = function (e) {
      return e.transaction;
    }),
    (e.wbg.__wbg_upperBound_a0bd8ece19d98580 = function () {
      return O(function (e, t) {
        return IDBKeyRange.upperBound(e, t !== 0);
      }, arguments);
    }),
    (e.wbg.__wbg_value_17b896954e14f896 = function (e) {
      return e.value;
    }),
    (e.wbg.__wbg_value_648dc44894c8dc95 = function () {
      return O(function (e) {
        return e.value;
      }, arguments);
    }),
    (e.wbg.__wbg_wasmdochandle_new = function (e) {
      return st.__wrap(e);
    }),
    (e.wbg.__wbg_wasmdocumentwatcher_new = function (e) {
      return lt.__wrap(e);
    }),
    (e.wbg.__wbg_wasmerror_new = function (e) {
      return dt.__wrap(e);
    }),
    (e.wbg.__wbg_wasmrepo_new = function (e) {
      return pt.__wrap(e);
    }),
    (e.wbg.__wbg_wasmtonkcore_new = function (e) {
      return ht.__wrap(e);
    }),
    (e.wbg.__wbindgen_bigint_from_i64 = function (e) {
      return e;
    }),
    (e.wbg.__wbindgen_bigint_from_u64 = function (e) {
      return BigInt.asUintN(64, e);
    }),
    (e.wbg.__wbindgen_bigint_get_as_i64 = function (e, t) {
      let n = t,
        r = typeof n == `bigint` ? n : void 0;
      (E().setBigInt64(e + 8, k(r) ? BigInt(0) : r, !0),
        E().setInt32(e + 0, !k(r), !0));
    }),
    (e.wbg.__wbindgen_boolean_get = function (e) {
      let t = e;
      return typeof t == `boolean` ? (t ? 1 : 0) : 2;
    }),
    (e.wbg.__wbindgen_cb_drop = function (e) {
      let t = e.original;
      return t.cnt-- == 1 ? ((t.a = 0), !0) : !1;
    }),
    (e.wbg.__wbindgen_closure_wrapper1905 = function (e, t, n) {
      return A(e, t, 728, He);
    }),
    (e.wbg.__wbindgen_closure_wrapper1907 = function (e, t, n) {
      return A(e, t, 728, He);
    }),
    (e.wbg.__wbindgen_closure_wrapper1909 = function (e, t, n) {
      return A(e, t, 728, Ue);
    }),
    (e.wbg.__wbindgen_closure_wrapper1911 = function (e, t, n) {
      return A(e, t, 728, He);
    }),
    (e.wbg.__wbindgen_closure_wrapper2607 = function (e, t, n) {
      return A(e, t, 1007, We);
    }),
    (e.wbg.__wbindgen_closure_wrapper2609 = function (e, t, n) {
      return A(e, t, 1007, Ge);
    }),
    (e.wbg.__wbindgen_closure_wrapper2637 = function (e, t, n) {
      return A(e, t, 1025, Ke);
    }),
    (e.wbg.__wbindgen_debug_string = function (e, t) {
      let n = Oe(t),
        r = T(n, M.__wbindgen_malloc, M.__wbindgen_realloc),
        i = I;
      (E().setInt32(e + 4, i, !0), E().setInt32(e + 0, r, !0));
    }),
    (e.wbg.__wbindgen_in = function (e, t) {
      return e in t;
    }),
    (e.wbg.__wbindgen_init_externref_table = function () {
      let e = M.__wbindgen_export_4,
        t = e.grow(4);
      (e.set(0, void 0),
        e.set(t + 0, void 0),
        e.set(t + 1, null),
        e.set(t + 2, !0),
        e.set(t + 3, !1));
    }),
    (e.wbg.__wbindgen_is_bigint = function (e) {
      return typeof e == `bigint`;
    }),
    (e.wbg.__wbindgen_is_function = function (e) {
      return typeof e == `function`;
    }),
    (e.wbg.__wbindgen_is_null = function (e) {
      return e === null;
    }),
    (e.wbg.__wbindgen_is_object = function (e) {
      let t = e;
      return typeof t == `object` && !!t;
    }),
    (e.wbg.__wbindgen_is_string = function (e) {
      return typeof e == `string`;
    }),
    (e.wbg.__wbindgen_is_undefined = function (e) {
      return e === void 0;
    }),
    (e.wbg.__wbindgen_jsval_eq = function (e, t) {
      return e === t;
    }),
    (e.wbg.__wbindgen_jsval_loose_eq = function (e, t) {
      return e == t;
    }),
    (e.wbg.__wbindgen_memory = function () {
      return M.memory;
    }),
    (e.wbg.__wbindgen_number_get = function (e, t) {
      let n = t,
        r = typeof n == `number` ? n : void 0;
      (E().setFloat64(e + 8, k(r) ? 0 : r, !0), E().setInt32(e + 0, !k(r), !0));
    }),
    (e.wbg.__wbindgen_number_new = function (e) {
      return e;
    }),
    (e.wbg.__wbindgen_string_get = function (e, t) {
      let n = t,
        r = typeof n == `string` ? n : void 0;
      var i = k(r) ? 0 : T(r, M.__wbindgen_malloc, M.__wbindgen_realloc),
        a = I;
      (E().setInt32(e + 4, a, !0), E().setInt32(e + 0, i, !0));
    }),
    (e.wbg.__wbindgen_string_new = function (e, t) {
      return w(e, t);
    }),
    (e.wbg.__wbindgen_throw = function (e, t) {
      throw Error(w(e, t));
    }),
    e
  );
}
function Xe(e, t) {
  return (
    (M = e.exports),
    (Qe.__wbindgen_wasm_module = t),
    (R = null),
    (N = null),
    M.__wbindgen_start(),
    M
  );
}
function Ze(e) {
  if (M !== void 0) return M;
  e !== void 0 &&
    (Object.getPrototypeOf(e) === Object.prototype
      ? ({ module: e } = e)
      : console.warn(
          "using deprecated parameters for `initSync()`; pass a single object instead",
        ));
  let t = Ye();
  e instanceof WebAssembly.Module || (e = new WebAssembly.Module(e));
  let n = new WebAssembly.Instance(e, t);
  return Xe(n, e);
}
async function Qe(e) {
  if (M !== void 0) return M;
  (e !== void 0 &&
    (Object.getPrototypeOf(e) === Object.prototype
      ? ({ module_or_path: e } = e)
      : console.warn(
          `using deprecated parameters for the initialization function; pass a single object instead`,
        )),
    e === void 0 && (e = new URL(`/tonk_core_bg.wasm`, `` + import.meta.url)));
  let t = Ye();
  (typeof e == `string` ||
    (typeof Request == `function` && e instanceof Request) ||
    (typeof URL == `function` && e instanceof URL)) &&
    (e = fetch(e));
  let { instance: n, module: r } = await Je(await e, t);
  return Xe(n, r);
}
var M,
  N,
  P,
  $e,
  F,
  I,
  L,
  et,
  R,
  tt,
  nt,
  rt,
  it,
  at,
  z,
  ot,
  st,
  ct,
  lt,
  ut,
  dt,
  ft,
  pt,
  mt,
  ht,
  gt,
  _t,
  vt,
  B,
  V = t(() => {
    ((N = null),
      (P =
        typeof TextDecoder < `u`
          ? new TextDecoder(`utf-8`, { ignoreBOM: !0, fatal: !0 })
          : {
              decode: () => {
                throw Error(`TextDecoder not available`);
              },
            }),
      typeof TextDecoder < `u` && P.decode(),
      ($e = 2146435072),
      (F = 0),
      (I = 0),
      (L =
        typeof TextEncoder < `u`
          ? new TextEncoder(`utf-8`)
          : {
              encode: () => {
                throw Error(`TextEncoder not available`);
              },
            }),
      (et =
        typeof L.encodeInto == `function`
          ? function (e, t) {
              return L.encodeInto(e, t);
            }
          : function (e, t) {
              let n = L.encode(e);
              return (t.set(n), { read: e.length, written: n.length });
            }),
      (R = null),
      (tt =
        typeof FinalizationRegistry > `u`
          ? { register: () => {}, unregister: () => {} }
          : new FinalizationRegistry((e) => {
              M.__wbindgen_export_6.get(e.dtor)(e.a, e.b);
            })),
      (nt = [`blob`, `arraybuffer`]),
      (rt = [`pending`, `done`]),
      (it = [
        `readonly`,
        `readwrite`,
        `versionchange`,
        `readwriteflush`,
        `cleanup`,
      ]),
      (at =
        typeof FinalizationRegistry > `u`
          ? { register: () => {}, unregister: () => {} }
          : new FinalizationRegistry((e) =>
              M.__wbg_wasmbundle_free(e >>> 0, 1),
            )),
      (z = class e {
        static __wrap(t) {
          t >>>= 0;
          let n = Object.create(e.prototype);
          return ((n.__wbg_ptr = t), at.register(n, n.__wbg_ptr, n), n);
        }
        __destroy_into_raw() {
          let e = this.__wbg_ptr;
          return ((this.__wbg_ptr = 0), at.unregister(this), e);
        }
        free() {
          let e = this.__destroy_into_raw();
          M.__wbg_wasmbundle_free(e, 0);
        }
        static fromBytes(t) {
          let n = M.wasmbundle_fromBytes(t);
          if (n[2]) throw j(n[1]);
          return e.__wrap(n[0]);
        }
        getPrefix(e) {
          let t = T(e, M.__wbindgen_malloc, M.__wbindgen_realloc),
            n = I;
          return M.wasmbundle_getPrefix(this.__wbg_ptr, t, n);
        }
        getRootId() {
          return M.wasmbundle_getRootId(this.__wbg_ptr);
        }
        getManifest() {
          return M.wasmbundle_getManifest(this.__wbg_ptr);
        }
        setManifest(e) {
          return M.wasmbundle_setManifest(this.__wbg_ptr, e);
        }
        get(e) {
          let t = T(e, M.__wbindgen_malloc, M.__wbindgen_realloc),
            n = I;
          return M.wasmbundle_get(this.__wbg_ptr, t, n);
        }
        toBytes() {
          return M.wasmbundle_toBytes(this.__wbg_ptr);
        }
        listKeys() {
          return M.wasmbundle_listKeys(this.__wbg_ptr);
        }
      }),
      (ot =
        typeof FinalizationRegistry > `u`
          ? { register: () => {}, unregister: () => {} }
          : new FinalizationRegistry((e) =>
              M.__wbg_wasmdochandle_free(e >>> 0, 1),
            )),
      (st = class e {
        static __wrap(t) {
          t >>>= 0;
          let n = Object.create(e.prototype);
          return ((n.__wbg_ptr = t), ot.register(n, n.__wbg_ptr, n), n);
        }
        __destroy_into_raw() {
          let e = this.__wbg_ptr;
          return ((this.__wbg_ptr = 0), ot.unregister(this), e);
        }
        free() {
          let e = this.__destroy_into_raw();
          M.__wbg_wasmdochandle_free(e, 0);
        }
        documentId() {
          let e, t;
          try {
            let n = M.wasmdochandle_documentId(this.__wbg_ptr);
            return ((e = n[0]), (t = n[1]), w(n[0], n[1]));
          } finally {
            M.__wbindgen_free(e, t, 1);
          }
        }
        getDocument() {
          let e = M.wasmdochandle_getDocument(this.__wbg_ptr);
          if (e[2]) throw j(e[1]);
          return j(e[0]);
        }
        url() {
          let e, t;
          try {
            let n = M.wasmdochandle_url(this.__wbg_ptr);
            return ((e = n[0]), (t = n[1]), w(n[0], n[1]));
          } finally {
            M.__wbindgen_free(e, t, 1);
          }
        }
      }),
      (ct =
        typeof FinalizationRegistry > `u`
          ? { register: () => {}, unregister: () => {} }
          : new FinalizationRegistry((e) =>
              M.__wbg_wasmdocumentwatcher_free(e >>> 0, 1),
            )),
      (lt = class e {
        static __wrap(t) {
          t >>>= 0;
          let n = Object.create(e.prototype);
          return ((n.__wbg_ptr = t), ct.register(n, n.__wbg_ptr, n), n);
        }
        __destroy_into_raw() {
          let e = this.__wbg_ptr;
          return ((this.__wbg_ptr = 0), ct.unregister(this), e);
        }
        free() {
          let e = this.__destroy_into_raw();
          M.__wbg_wasmdocumentwatcher_free(e, 0);
        }
        documentId() {
          let e, t;
          try {
            let n = M.wasmdocumentwatcher_documentId(this.__wbg_ptr);
            return ((e = n[0]), (t = n[1]), w(n[0], n[1]));
          } finally {
            M.__wbindgen_free(e, t, 1);
          }
        }
        stop() {
          return M.wasmdocumentwatcher_stop(this.__wbg_ptr);
        }
      }),
      (ut =
        typeof FinalizationRegistry > `u`
          ? { register: () => {}, unregister: () => {} }
          : new FinalizationRegistry((e) =>
              M.__wbg_wasmerror_free(e >>> 0, 1),
            )),
      (dt = class e {
        static __wrap(t) {
          t >>>= 0;
          let n = Object.create(e.prototype);
          return ((n.__wbg_ptr = t), ut.register(n, n.__wbg_ptr, n), n);
        }
        __destroy_into_raw() {
          let e = this.__wbg_ptr;
          return ((this.__wbg_ptr = 0), ut.unregister(this), e);
        }
        free() {
          let e = this.__destroy_into_raw();
          M.__wbg_wasmerror_free(e, 0);
        }
        get message() {
          let e, t;
          try {
            let n = M.wasmerror_message(this.__wbg_ptr);
            return ((e = n[0]), (t = n[1]), w(n[0], n[1]));
          } finally {
            M.__wbindgen_free(e, t, 1);
          }
        }
      }),
      (ft =
        typeof FinalizationRegistry > `u`
          ? { register: () => {}, unregister: () => {} }
          : new FinalizationRegistry((e) => M.__wbg_wasmrepo_free(e >>> 0, 1))),
      (pt = class e {
        static __wrap(t) {
          t >>>= 0;
          let n = Object.create(e.prototype);
          return ((n.__wbg_ptr = t), ft.register(n, n.__wbg_ptr, n), n);
        }
        __destroy_into_raw() {
          let e = this.__wbg_ptr;
          return ((this.__wbg_ptr = 0), ft.unregister(this), e);
        }
        free() {
          let e = this.__destroy_into_raw();
          M.__wbg_wasmrepo_free(e, 0);
        }
        findDocument(e) {
          let t = T(e, M.__wbindgen_malloc, M.__wbindgen_realloc),
            n = I;
          return M.wasmrepo_findDocument(this.__wbg_ptr, t, n);
        }
        listDocuments() {
          let e = M.wasmrepo_listDocuments(this.__wbg_ptr);
          if (e[2]) throw j(e[1]);
          return j(e[0]);
        }
        createDocument(e) {
          return M.wasmrepo_createDocument(this.__wbg_ptr, e);
        }
        connectWebSocket(e) {
          let t = T(e, M.__wbindgen_malloc, M.__wbindgen_realloc),
            n = I,
            r = M.wasmrepo_connectWebSocket(this.__wbg_ptr, t, n);
          if (r[2]) throw j(r[1]);
          return _t.__wrap(r[0]);
        }
        connectWebSocketAsync(e) {
          let t = T(e, M.__wbindgen_malloc, M.__wbindgen_realloc),
            n = I;
          return M.wasmrepo_connectWebSocketAsync(this.__wbg_ptr, t, n);
        }
        constructor() {
          return M.wasmrepo_new();
        }
        stop() {
          return M.wasmrepo_stop(this.__wbg_ptr);
        }
        peerId() {
          let e, t;
          try {
            let n = M.wasmrepo_peerId(this.__wbg_ptr);
            return ((e = n[0]), (t = n[1]), w(n[0], n[1]));
          } finally {
            M.__wbindgen_free(e, t, 1);
          }
        }
      }),
      (mt =
        typeof FinalizationRegistry > `u`
          ? { register: () => {}, unregister: () => {} }
          : new FinalizationRegistry((e) =>
              M.__wbg_wasmtonkcore_free(e >>> 0, 1),
            )),
      (ht = class e {
        static __wrap(t) {
          t >>>= 0;
          let n = Object.create(e.prototype);
          return ((n.__wbg_ptr = t), mt.register(n, n.__wbg_ptr, n), n);
        }
        __destroy_into_raw() {
          let e = this.__wbg_ptr;
          return ((this.__wbg_ptr = 0), mt.unregister(this), e);
        }
        free() {
          let e = this.__destroy_into_raw();
          M.__wbg_wasmtonkcore_free(e, 0);
        }
        static fromBytes(e) {
          return M.wasmtonkcore_fromBytes(e);
        }
        patchFile(e, t, n) {
          let r = T(e, M.__wbindgen_malloc, M.__wbindgen_realloc),
            i = I;
          return M.wasmtonkcore_patchFile(this.__wbg_ptr, r, i, t, n);
        }
        createFile(e, t) {
          let n = T(e, M.__wbindgen_malloc, M.__wbindgen_realloc),
            r = I;
          return M.wasmtonkcore_createFile(this.__wbg_ptr, n, r, t);
        }
        deleteFile(e) {
          let t = T(e, M.__wbindgen_malloc, M.__wbindgen_realloc),
            n = I;
          return M.wasmtonkcore_deleteFile(this.__wbg_ptr, t, n);
        }
        static fromBundle(e) {
          return (ke(e, z), M.wasmtonkcore_fromBundle(e.__wbg_ptr));
        }
        getPeerId() {
          return M.wasmtonkcore_getPeerId(this.__wbg_ptr);
        }
        spliceText(e, t, n, r, i) {
          let a = T(e, M.__wbindgen_malloc, M.__wbindgen_realloc),
            o = I,
            s = T(i, M.__wbindgen_malloc, M.__wbindgen_realloc),
            c = I;
          return M.wasmtonkcore_spliceText(this.__wbg_ptr, a, o, t, n, r, s, c);
        }
        updateFile(e, t) {
          let n = T(e, M.__wbindgen_malloc, M.__wbindgen_realloc),
            r = I;
          return M.wasmtonkcore_updateFile(this.__wbg_ptr, n, r, t);
        }
        getMetadata(e) {
          let t = T(e, M.__wbindgen_malloc, M.__wbindgen_realloc),
            n = I;
          return M.wasmtonkcore_getMetadata(this.__wbg_ptr, t, n);
        }
        isConnected() {
          return M.wasmtonkcore_isConnected(this.__wbg_ptr);
        }
        static withPeerId(e) {
          let t = T(e, M.__wbindgen_malloc, M.__wbindgen_realloc),
            n = I;
          return M.wasmtonkcore_withPeerId(t, n);
        }
        forkToBytes(e) {
          return M.wasmtonkcore_forkToBytes(this.__wbg_ptr, e);
        }
        listDirectory(e) {
          let t = T(e, M.__wbindgen_malloc, M.__wbindgen_realloc),
            n = I;
          return M.wasmtonkcore_listDirectory(this.__wbg_ptr, t, n);
        }
        watchDocument(e, t) {
          let n = T(e, M.__wbindgen_malloc, M.__wbindgen_realloc),
            r = I;
          return M.wasmtonkcore_watchDocument(this.__wbg_ptr, n, r, t);
        }
        watchDirectory(e, t) {
          let n = T(e, M.__wbindgen_malloc, M.__wbindgen_realloc),
            r = I;
          return M.wasmtonkcore_watchDirectory(this.__wbg_ptr, n, r, t);
        }
        createDirectory(e) {
          let t = T(e, M.__wbindgen_malloc, M.__wbindgen_realloc),
            n = I;
          return M.wasmtonkcore_createDirectory(this.__wbg_ptr, t, n);
        }
        connectWebsocket(e) {
          let t = T(e, M.__wbindgen_malloc, M.__wbindgen_realloc),
            n = I;
          return M.wasmtonkcore_connectWebsocket(this.__wbg_ptr, t, n);
        }
        setFileWithBytes(e, t, n) {
          let r = T(e, M.__wbindgen_malloc, M.__wbindgen_realloc),
            i = I,
            a = Ae(n, M.__wbindgen_malloc),
            o = I;
          return M.wasmtonkcore_setFileWithBytes(this.__wbg_ptr, r, i, t, a, o);
        }
        getConnectionState() {
          return M.wasmtonkcore_getConnectionState(this.__wbg_ptr);
        }
        createFileWithBytes(e, t, n) {
          let r = T(e, M.__wbindgen_malloc, M.__wbindgen_realloc),
            i = I,
            a = Ae(n, M.__wbindgen_malloc),
            o = I;
          return M.wasmtonkcore_createFileWithBytes(
            this.__wbg_ptr,
            r,
            i,
            t,
            a,
            o,
          );
        }
        constructor() {
          return M.wasmtonkcore_new();
        }
        exists(e) {
          let t = T(e, M.__wbindgen_malloc, M.__wbindgen_realloc),
            n = I;
          return M.wasmtonkcore_exists(this.__wbg_ptr, t, n);
        }
        rename(e, t) {
          let n = T(e, M.__wbindgen_malloc, M.__wbindgen_realloc),
            r = I,
            i = T(t, M.__wbindgen_malloc, M.__wbindgen_realloc),
            a = I;
          return M.wasmtonkcore_rename(this.__wbg_ptr, n, r, i, a);
        }
        setFile(e, t) {
          let n = T(e, M.__wbindgen_malloc, M.__wbindgen_realloc),
            r = I;
          return M.wasmtonkcore_setFile(this.__wbg_ptr, n, r, t);
        }
        toBytes(e) {
          return M.wasmtonkcore_toBytes(this.__wbg_ptr, e);
        }
        readFile(e) {
          let t = T(e, M.__wbindgen_malloc, M.__wbindgen_realloc),
            n = I;
          return M.wasmtonkcore_readFile(this.__wbg_ptr, t, n);
        }
      }),
      (gt =
        typeof FinalizationRegistry > `u`
          ? { register: () => {}, unregister: () => {} }
          : new FinalizationRegistry((e) =>
              M.__wbg_wasmwebsockethandle_free(e >>> 0, 1),
            )),
      (_t = class e {
        static __wrap(t) {
          t >>>= 0;
          let n = Object.create(e.prototype);
          return ((n.__wbg_ptr = t), gt.register(n, n.__wbg_ptr, n), n);
        }
        __destroy_into_raw() {
          let e = this.__wbg_ptr;
          return ((this.__wbg_ptr = 0), gt.unregister(this), e);
        }
        free() {
          let e = this.__destroy_into_raw();
          M.__wbg_wasmwebsockethandle_free(e, 0);
        }
        waitForDisconnect() {
          return M.wasmwebsockethandle_waitForDisconnect(this.__wbg_ptr);
        }
        close() {
          M.wasmwebsockethandle_close(this.__wbg_ptr);
        }
      }),
      (vt = new Set([`basic`, `cors`, `default`])),
      (B = Qe));
  }),
  yt = function (e, t, n, r, i) {
    if (r === `m`) throw TypeError(`Private method is not writable`);
    if (r === `a` && !i)
      throw TypeError(`Private accessor was defined without a setter`);
    if (typeof t == `function` ? e !== t || !i : !t.has(e))
      throw TypeError(
        `Cannot write private member to an object whose class did not declare it`,
      );
    return (r === `a` ? i.call(e, n) : i ? (i.value = n) : t.set(e, n), n);
  },
  H = function (e, t, n, r) {
    if (n === `a` && !r)
      throw TypeError(`Private accessor was defined without a getter`);
    if (typeof t == `function` ? e !== t || !r : !t.has(e))
      throw TypeError(
        `Cannot read private member from an object whose class did not declare it`,
      );
    return n === `m` ? r : n === `a` ? r.call(e) : r ? r.value : t.get(e);
  },
  U,
  W,
  G = class extends Error {
    constructor(e, t) {
      (super(e), (this.code = t), (this.name = `TonkError`));
    }
  },
  bt = class extends G {
    constructor(e) {
      (super(e, `CONNECTION_ERROR`), (this.name = `ConnectionError`));
    }
  },
  K = class extends G {
    constructor(e) {
      (super(e, `FILESYSTEM_ERROR`), (this.name = `FileSystemError`));
    }
  },
  q = class extends G {
    constructor(e) {
      (super(e, `BUNDLE_ERROR`), (this.name = `BundleError`));
    }
  },
  J = class e {
    constructor(e) {
      (U.set(this, void 0), yt(this, U, e, `f`));
    }
    static async fromBytes(t, n) {
      try {
        let { create_bundle_from_bytes: r } =
          n || (await x(() => Promise.resolve().then(() => (V(), S)), void 0));
        return new e(r(t));
      } catch (e) {
        throw new q(`Failed to create bundle from bytes: ${e}`);
      }
    }
    async getRootId() {
      try {
        return await H(this, U, `f`).getRootId();
      } catch (e) {
        throw new q(`Failed to get root ID: ${e}`);
      }
    }
    async get(e) {
      try {
        let t = await H(this, U, `f`).get(e);
        return t === null ? null : t;
      } catch (t) {
        throw new q(`Failed to get key ${e}: ${t}`);
      }
    }
    async getPrefix(e) {
      try {
        return (await H(this, U, `f`).getPrefix(e)).map((e) => ({
          key: e.key,
          value: e.value,
        }));
      } catch (t) {
        throw new q(`Failed to get prefix ${e}: ${t}`);
      }
    }
    async listKeys() {
      try {
        return await H(this, U, `f`).listKeys();
      } catch (e) {
        throw new q(`Failed to list keys: ${e}`);
      }
    }
    async getManifest() {
      try {
        return await H(this, U, `f`).getManifest();
      } catch (e) {
        throw new q(`Failed to retrieve manifest: ${e}`);
      }
    }
    async setManifest(e) {
      try {
        await H(this, U, `f`).setManifest(e);
      } catch (e) {
        throw new q(`Failed to set manifest: ${e}`);
      }
    }
    async toBytes() {
      try {
        return await H(this, U, `f`).toBytes();
      } catch (e) {
        throw new q(`Failed to serialize bundle: ${e}`);
      }
    }
    free() {
      H(this, U, `f`).free();
    }
  };
U = new WeakMap();
var Y = class e {
  constructor(e) {
    (W.set(this, void 0), yt(this, W, e, `f`));
  }
  static async create(t, n) {
    let r =
      n || (await x(() => Promise.resolve().then(() => (V(), S)), void 0));
    if (t?.peerId && t?.storage) {
      let { create_tonk_with_config: n } = r,
        i = await n(t.peerId, t.storage.type === `indexeddb`);
      return new e(i);
    } else if (t?.peerId) {
      let { create_tonk_with_peer_id: n } = r,
        i = await n(t.peerId);
      return new e(i);
    } else if (t?.storage) {
      let { create_tonk_with_storage: n } = r,
        i = await n(t.storage.type === `indexeddb`);
      return new e(i);
    } else {
      let { create_tonk: t } = r,
        n = await t();
      return new e(n);
    }
  }
  static async createWithPeerId(t, n) {
    let { create_tonk_with_peer_id: r } =
        n || (await x(() => Promise.resolve().then(() => (V(), S)), void 0)),
      i = await r(t);
    return new e(i);
  }
  static async fromBundle(t, n, r) {
    let i =
      r || (await x(() => Promise.resolve().then(() => (V(), S)), void 0));
    if (n?.storage) {
      let { create_tonk_from_bundle_with_storage: r } = i,
        a = await r(t, n.storage.type === `indexeddb`);
      return new e(a);
    } else {
      let { create_tonk_from_bundle: n } = i,
        r = await n(t);
      return new e(r);
    }
  }
  static async fromBytes(t, n, r) {
    let i =
      r || (await x(() => Promise.resolve().then(() => (V(), S)), void 0));
    if (n?.storage) {
      let { create_tonk_from_bytes_with_storage: r } = i,
        a = await r(t, n.storage.type === `indexeddb`);
      return new e(a);
    } else {
      let { create_tonk_from_bytes: n } = i,
        r = await n(t);
      return new e(r);
    }
  }
  getPeerId() {
    return H(this, W, `f`).getPeerId();
  }
  async connectWebsocket(e) {
    try {
      await H(this, W, `f`).connectWebsocket(e);
    } catch (t) {
      throw new bt(`Failed to connect to ${e}: ${t}`);
    }
  }
  async isConnected() {
    try {
      return await H(this, W, `f`).isConnected();
    } catch (e) {
      return (console.error(`🔌 [CORE-JS] isConnected() error:`, e), !1);
    }
  }
  async getConnectionState() {
    try {
      return await H(this, W, `f`).getConnectionState();
    } catch (e) {
      return (
        console.error(`📡 [CORE-JS] getConnectionState() error:`, e),
        `failed:` + String(e)
      );
    }
  }
  async forkToBytes(e) {
    try {
      return await H(this, W, `f`).forkToBytes(e);
    } catch (e) {
      throw new G(`Failed to serialize to bundle data: ${e}`);
    }
  }
  async toBytes(e) {
    try {
      return await H(this, W, `f`).toBytes(e);
    } catch (e) {
      throw new G(`Failed to serialize to bundle data: ${e}`);
    }
  }
  async createFile(e, t) {
    try {
      await H(this, W, `f`).createFile(e, t);
    } catch (t) {
      throw new K(`Failed to create file at ${e}: ${t}`);
    }
  }
  async createFileWithBytes(e, t, n) {
    try {
      let r = typeof n == `string` ? xt(n) : n;
      await H(this, W, `f`).createFileWithBytes(e, t, r);
    } catch (t) {
      throw new K(`Failed to create file at ${e}: ${t}`);
    }
  }
  async readFile(e) {
    try {
      let t = await H(this, W, `f`).readFile(e);
      if (t === null) throw new K(`File not found: ${e}`);
      let n;
      return (
        t.bytes && (n = St(t.bytes)),
        {
          ...t,
          content:
            typeof t.content == `string` ? JSON.parse(t.content) : t.content,
          bytes: n,
        }
      );
    } catch (t) {
      throw t instanceof K ? t : new K(`Failed to read file at ${e}: ${t}`);
    }
  }
  async setFile(e, t) {
    try {
      return await H(this, W, `f`).setFile(e, t);
    } catch (t) {
      throw new K(`Failed to set file at ${e}: ${t}`);
    }
  }
  async setFileWithBytes(e, t, n) {
    try {
      let r = typeof n == `string` ? xt(n) : n;
      return await H(this, W, `f`).setFileWithBytes(e, t, r);
    } catch (t) {
      throw new K(`Failed to set file at ${e}: ${t}`);
    }
  }
  async updateFile(e, t) {
    try {
      return await H(this, W, `f`).updateFile(e, t);
    } catch (t) {
      throw new K(`Failed to update file at ${e}: ${t}`);
    }
  }
  async patchFile(e, t, n) {
    try {
      return await H(this, W, `f`).patchFile(e, t, n);
    } catch (t) {
      throw new K(`Failed to patch file at ${e}: ${t}`);
    }
  }
  async spliceText(e, t, n, r, i) {
    try {
      return await H(this, W, `f`).spliceText(e, t, n, r, i);
    } catch (t) {
      throw new K(`Failed to splice text at ${e}: ${t}`);
    }
  }
  async deleteFile(e) {
    try {
      return await H(this, W, `f`).deleteFile(e);
    } catch (t) {
      throw new K(`Failed to delete file at ${e}: ${t}`);
    }
  }
  async createDirectory(e) {
    try {
      await H(this, W, `f`).createDirectory(e);
    } catch (t) {
      throw new K(`Failed to create directory at ${e}: ${t}`);
    }
  }
  async listDirectory(e) {
    try {
      return (await H(this, W, `f`).listDirectory(e)).map((e) => ({
        name: e.name,
        type: e.type,
        timestamps: e.timestamps,
        pointer: e.pointer,
      }));
    } catch (t) {
      throw new K(`Failed to list directory at ${e}: ${t}`);
    }
  }
  async exists(e) {
    try {
      return await H(this, W, `f`).exists(e);
    } catch (t) {
      throw new K(`Failed to check existence of ${e}: ${t}`);
    }
  }
  async rename(e, t) {
    try {
      return await H(this, W, `f`).rename(e, t);
    } catch (n) {
      throw new K(`Failed to rename ${e} to ${t}: ${n}`);
    }
  }
  async getMetadata(e) {
    try {
      let t = await H(this, W, `f`).getMetadata(e);
      if (t === null) throw new K(`File or directory not found: ${e}`);
      return t;
    } catch (t) {
      throw t instanceof K ? t : new K(`Failed to get metadata for ${e}: ${t}`);
    }
  }
  async watchFile(e, t) {
    try {
      let n = await H(this, W, `f`).watchDocument(e, (e) => {
        let n;
        (e.bytes && (n = St(e.bytes)),
          t({
            ...e,
            content:
              typeof e.content == `string` ? JSON.parse(e.content) : e.content,
            bytes: n,
          }));
      });
      if (n === null) throw new K(`File not found: ${e}`);
      return n;
    } catch (t) {
      throw t instanceof K
        ? t
        : new K(`Failed to watch file at path ${e}: ${t}`);
    }
  }
  async watchDirectory(e, t) {
    try {
      let n = await H(this, W, `f`).watchDirectory(e, t);
      if (n === null) throw new K(`Directory not found: ${e}`);
      return n;
    } catch (t) {
      throw t instanceof K
        ? t
        : new K(`Failed to watch directory at path ${e}: ${t}`);
    }
  }
  free() {
    H(this, W, `f`).free();
  }
};
W = new WeakMap();
var xt = (e) => {
    let t = atob(e),
      n = new Uint8Array(t.length);
    for (let e = 0; e < t.length; e++) n[e] = t.charCodeAt(e);
    return n;
  },
  St = (e) => {
    if (typeof e == `string`) return e;
    if (Array.isArray(e)) {
      let t = 8192,
        n = ``;
      for (let r = 0; r < e.length; r += t) {
        let i = e.slice(r, r + t);
        n += String.fromCharCode(...i);
      }
      return btoa(n);
    } else throw new K(`Unrecognized bytes type in readFile ${typeof e}`);
  };
V();
var Ct = !1,
  X = null;
async function wt(e) {
  if (!Ct)
    return (
      X ||
      ((X = (async () => {
        try {
          (e?.wasmPath ? await B({ module_or_path: e.wasmPath }) : await B(),
            (Ct = !0));
        } catch (e) {
          throw ((X = null), e);
        }
      })()),
      X)
    );
}
async function Tt() {
  let e = _();
  if (!e) return !1;
  try {
    return await e.tonk.isConnected();
  } catch (e) {
    return (
      u.error(`performHealthCheck() failed`, {
        error: e instanceof Error ? e.message : String(e),
      }),
      !1
    );
  }
}
async function Et() {
  let e = _();
  if (!e) {
    u.error(`Cannot reconnect: no active bundle`);
    return;
  }
  let t = e.wsUrl;
  if (!t) {
    u.error(`Cannot reconnect: wsUrl not stored`);
    return;
  }
  let n = pe();
  (n >= 10 && me(),
    u.debug(`Attempting to reconnect`, {
      attempt: n,
      maxAttempts: 10,
      wsUrl: t,
    }),
    await b({ type: `reconnecting`, attempt: n }));
  try {
    if (
      (await e.tonk.connectWebsocket(t),
      await new Promise((e) => setTimeout(e, 1e3)),
      await e.tonk.isConnected())
    )
      (fe(!0),
        me(),
        u.info(`Reconnection successful`),
        await b({ type: `reconnected` }),
        await Dt());
    else throw Error(`Connection check failed after reconnect attempt`);
  } catch (e) {
    u.warn(`Reconnection failed`, {
      error: e instanceof Error ? e.message : String(e),
      attempt: n,
    });
    let t = Math.min(1e3 * 2 ** (n - 1), 3e4);
    (u.debug(`Scheduling next reconnect attempt`, {
      delayMs: t,
      nextAttempt: n + 1,
    }),
      setTimeout(Et, t));
  }
}
async function Dt() {
  let e = ye();
  (u.debug(`Re-establishing watchers after reconnection`, {
    watcherCount: e.length,
  }),
    u.debug(`Watcher re-establishment complete`, { watcherCount: e.length }),
    await b({ type: `watchersReestablished`, count: e.length }));
}
function Z() {
  let e = _();
  if (!e) {
    u.warn(`Cannot start health monitoring: no active bundle`);
    return;
  }
  (e.healthCheckInterval && clearInterval(e.healthCheckInterval),
    u.debug(`Starting health monitoring`, { intervalMs: r }));
  let t = setInterval(async () => {
    let e = await Tt(),
      n = _();
    if (!n) {
      clearInterval(t);
      return;
    }
    !e && n.connectionHealthy
      ? (fe(!1),
        u.warn(`Connection lost, starting reconnection attempts`),
        await b({ type: `disconnected` }),
        Et())
      : e &&
        !n.connectionHealthy &&
        (fe(!0), me(), u.debug(`Connection health restored`));
  }, r);
  he(t);
}
function Q(e, t = `http://localhost:8081`) {
  return e.networkUris && e.networkUris.length > 0
    ? e.networkUris[0].replace(/^http/, `ws`)
    : t.replace(/^http/, `ws`);
}
var $ = null,
  Ot = !1;
async function kt() {
  if (Ot) {
    u.debug(`WASM already initialized`);
    return;
  }
  return $
    ? (u.debug(`WASM initialization in progress, waiting...`), $)
    : (u.debug(`Starting WASM initialization`),
      ($ = (async () => {
        try {
          let e = `/tonk_core_bg.wasm?t=${Date.now()}`;
          (await wt({ wasmPath: e }),
            (Ot = !0),
            u.info(`WASM initialization completed`));
        } catch (e) {
          throw (
            ($ = null),
            (Ot = !1),
            u.error(`WASM initialization failed`, {
              error: e instanceof Error ? e.message : String(e),
            }),
            e
          );
        }
      })()),
      $);
}
async function At(e) {
  return (
    u.debug(`Waiting for PathIndex to sync from remote...`),
    new Promise((t) => {
      let n = !1,
        r = null,
        i = setTimeout(() => {
          if (r)
            try {
              r.stop();
            } catch (e) {
              u.warn(`Error stopping PathIndex watcher on timeout`, {
                error: e instanceof Error ? e.message : String(e),
              });
            }
          n ||
            (u.debug(`No PathIndex changes detected after 1s - proceeding`),
            t());
        }, 1e3);
      e.watchDirectory(`/`, (e) => {
        if (!n) {
          if (
            ((n = !0),
            u.debug(`PathIndex synced from remote`, { documentType: e.type }),
            clearTimeout(i),
            r)
          )
            try {
              r.stop();
            } catch (e) {
              u.warn(`Error stopping PathIndex watcher after sync`, {
                error: e instanceof Error ? e.message : String(e),
              });
            }
          t();
        }
      })
        .then((e) => {
          ((r = e),
            u.debug(`PathIndex watcher established`, {
              watcherId: e?.document_id || `unknown`,
            }));
        })
        .catch((e) => {
          (u.error(`Failed to establish PathIndex watcher`, {
            error: e.message,
          }),
            clearTimeout(i),
            t());
        });
    })
  );
}
async function jt() {
  try {
    let e = await re(),
      t = await ie(),
      n = await ae();
    if (!e || !t) {
      u.debug(`No cached state found, waiting for initialization message`, {
        hasSlug: !!e,
        hasBundle: !!t,
      });
      return;
    }
    (u.info(`Auto-initializing from cache`, { slug: e, bundleSize: t.length }),
      await kt());
    let r = await (await J.fromBytes(t)).getManifest();
    u.debug(`Bundle and manifest restored from cache`, { rootId: r.rootId });
    let i = await Y.fromBytes(t, { storage: { type: `indexeddb` } });
    u.debug(`TonkCore created from cached bundle`);
    let a = n || Q(r);
    (!n && a && (await m(a)),
      u.debug(`Connecting to websocket...`, {
        wsUrl: a,
        localRootId: r.rootId,
      }),
      await i.connectWebsocket(a),
      u.debug(`Websocket connected`),
      await At(i),
      u.info(`Auto-initialization complete`),
      y({
        status: `active`,
        bundleId: r.rootId,
        tonk: i,
        manifest: r,
        appSlug: e,
        wsUrl: a,
        healthCheckInterval: null,
        watchers: new Map(),
        connectionHealthy: !0,
        reconnectAttempts: 0,
      }),
      Z());
  } catch (e) {
    (u.error(`Auto-initialization failed`, {
      error: e instanceof Error ? e.message : String(e),
    }),
      y({ status: `idle` }),
      await oe(),
      (await self.clients.matchAll()).forEach((e) => {
        e.postMessage({
          type: `needsReinit`,
          appSlug: null,
          reason: `Auto-initialization failed`,
        });
      }));
  }
}
async function Mt(e, t, n, r) {
  u.debug(`Loading new bundle`, {
    byteLength: e.length,
    serverUrl: t,
    hasCachedManifest: !!r,
  });
  try {
    let n = g();
    await kt();
    let i;
    if (r)
      (u.info(`Using cached manifest, skipping Bundle.fromBytes`, {
        rootId: r.rootId,
      }),
        (i = r));
    else {
      u.debug(`No cached manifest, parsing bundle`, { byteLength: e.length });
      let t = await J.fromBytes(e);
      ((i = await t.getManifest()),
        t.free(),
        u.debug(`Bundle manifest extracted`, { rootId: i.rootId }));
    }
    if (n.status === `active` && n.manifest.rootId === i.rootId)
      return (
        u.debug(`Bundle already loaded with same rootId, skipping reload`, {
          rootId: i.rootId,
        }),
        { success: !0, skipped: !0 }
      );
    u.debug(`Creating new TonkCore from bundle bytes`);
    let a = await Y.fromBytes(e, { storage: { type: `indexeddb` } });
    u.debug(`New TonkCore created successfully`, { rootId: i.rootId });
    let o = Q(i, t),
      s = new URLSearchParams(self.location.search).get(`bundle`);
    if (s)
      try {
        let e = atob(s),
          t = JSON.parse(e);
        t.wsUrl && (o = t.wsUrl);
      } catch (e) {
        u.warn(`Could not parse bundle config for wsUrl`, {
          error: e instanceof Error ? e.message : String(e),
        });
      }
    (u.debug(`Determined websocket URL`, { wsUrl: o, serverUrl: t }),
      await m(o),
      u.debug(`Connecting new tonk to websocket`, {
        wsUrl: o,
        localRootId: i.rootId,
      }),
      o &&
        (await a.connectWebsocket(o),
        u.debug(`Websocket connection established`),
        await At(a),
        u.debug(`PathIndex sync complete after loadBundle`)));
    let c = g(),
      l = c.status === `active` ? c.appSlug : i.entrypoints?.[0] || `app`;
    return (
      y({
        status: `active`,
        bundleId: i.rootId,
        tonk: a,
        manifest: i,
        appSlug: l,
        wsUrl: o,
        healthCheckInterval: null,
        watchers: new Map(),
        connectionHealthy: !0,
        reconnectAttempts: 0,
      }),
      Z(),
      await p(e),
      u.debug(`Bundle bytes persisted to cache`),
      u.info(`Bundle loaded successfully`, { rootId: i.rootId }),
      { success: !0 }
    );
  } catch (e) {
    return (
      u.error(`Failed to load bundle`, {
        error: e instanceof Error ? e.message : String(e),
      }),
      y({ status: `error`, error: e instanceof Error ? e : Error(String(e)) }),
      { success: !1, error: e instanceof Error ? e.message : String(e) }
    );
  }
}
async function Nt(e) {
  u.debug(`Loading new bundle`, {
    byteLength: e.bundleBytes.byteLength,
    serverUrl: e.serverUrl,
    hasCachedManifest: !!e.manifest,
  });
  let t = e.serverUrl || `http://localhost:8081`,
    n = new Uint8Array(e.bundleBytes),
    r = await Mt(n, t, e.id, e.manifest);
  b({
    type: `loadBundle`,
    id: e.id,
    success: r.success,
    skipped: r.skipped,
    error: r.error,
  });
}
async function Pt(e) {
  u.debug(`Converting tonk to bytes`);
  try {
    let t = v();
    if (!t) throw Error(`Tonk not initialized`);
    let n = await t.tonk.toBytes(),
      r = t.manifest.rootId;
    (u.debug(`Tonk converted to bytes`, { byteLength: n.length, rootId: r }),
      b({ type: `toBytes`, id: e.id, success: !0, data: n, rootId: r }));
  } catch (t) {
    (u.error(`Failed to convert tonk to bytes`, {
      error: t instanceof Error ? t.message : String(t),
    }),
      b({
        type: `toBytes`,
        id: e.id,
        success: !1,
        error: t instanceof Error ? t.message : String(t),
      }));
  }
}
async function Ft(e) {
  u.debug(`Forking tonk to bytes`);
  try {
    let t = v();
    if (!t) throw Error(`Tonk not initialized`);
    let n = await t.tonk.forkToBytes(),
      r = (await (await J.fromBytes(n)).getManifest()).rootId;
    (u.debug(`Tonk forked to bytes`, { byteLength: n.length, rootId: r }),
      b({ type: `forkToBytes`, id: e.id, success: !0, data: n, rootId: r }));
  } catch (t) {
    (u.error(`Failed to fork tonk to bytes`, {
      error: t instanceof Error ? t.message : String(t),
    }),
      b({
        type: `forkToBytes`,
        id: e.id,
        success: !1,
        error: t instanceof Error ? t.message : String(t),
      }));
  }
}
async function It(e) {
  u.debug(`Listing directory`, { path: e.path });
  try {
    let t = v();
    if (!t) throw Error(`Tonk not initialized`);
    let n = await t.tonk.listDirectory(e.path);
    (u.debug(`Directory listed`, {
      path: e.path,
      fileCount: Array.isArray(n) ? n.length : `unknown`,
    }),
      b({ type: `listDirectory`, id: e.id, success: !0, data: n }));
  } catch (t) {
    (u.error(`Failed to list directory`, {
      path: e.path,
      error: t instanceof Error ? t.message : String(t),
    }),
      b({
        type: `listDirectory`,
        id: e.id,
        success: !1,
        error: t instanceof Error ? t.message : String(t),
      }));
  }
}
async function Lt(e) {
  u.debug(`Reading file`, { path: e.path });
  try {
    let t = v();
    if (!t) throw Error(`Tonk not initialized`);
    let n = await t.tonk.readFile(e.path);
    (u.debug(`File read successfully`, { path: e.path }),
      b({ type: `readFile`, id: e.id, success: !0, data: n }));
  } catch (t) {
    (u.error(`Failed to read file`, {
      path: e.path,
      error: t instanceof Error ? t.message : String(t),
    }),
      b({
        type: `readFile`,
        id: e.id,
        success: !1,
        error: t instanceof Error ? t.message : String(t),
      }));
  }
}
async function Rt(e) {
  u.debug(`Writing file`, {
    path: e.path,
    create: e.create,
    hasBytes: !!e.content.bytes,
  });
  try {
    let t = v();
    if (!t) throw Error(`Tonk not initialized`);
    let n = e.content.content;
    (e.create
      ? (u.debug(`Creating new file`, { path: e.path }),
        e.content.bytes
          ? await t.tonk.createFileWithBytes(e.path, n, e.content.bytes)
          : await t.tonk.createFile(e.path, n))
      : (u.debug(`Setting existing file`, { path: e.path }),
        e.content.bytes
          ? await t.tonk.setFileWithBytes(e.path, n, e.content.bytes)
          : await t.tonk.setFile(e.path, n)),
      u.debug(`File write completed`, { path: e.path }),
      b({ type: `writeFile`, id: e.id, success: !0 }));
  } catch (t) {
    (u.error(`Failed to write file`, {
      path: e.path,
      create: e.create,
      error: t instanceof Error ? t.message : String(t),
    }),
      b({
        type: `writeFile`,
        id: e.id,
        success: !1,
        error: t instanceof Error ? t.message : String(t),
      }));
  }
}
async function zt(e) {
  u.debug(`Deleting file`, { path: e.path });
  try {
    let t = v();
    if (!t) throw Error(`Tonk not initialized`);
    (await t.tonk.deleteFile(e.path),
      u.debug(`File deleted successfully`, { path: e.path }),
      b({ type: `deleteFile`, id: e.id, success: !0 }));
  } catch (t) {
    (u.error(`Failed to delete file`, {
      path: e.path,
      error: t instanceof Error ? t.message : String(t),
    }),
      b({
        type: `deleteFile`,
        id: e.id,
        success: !1,
        error: t instanceof Error ? t.message : String(t),
      }));
  }
}
async function Bt(e) {
  u.debug(`Renaming file or directory`, {
    oldPath: e.oldPath,
    newPath: e.newPath,
  });
  try {
    let t = v();
    if (!t) throw Error(`Tonk not initialized`);
    (await t.tonk.rename(e.oldPath, e.newPath),
      u.debug(`Rename completed`, { oldPath: e.oldPath, newPath: e.newPath }),
      b({ type: `rename`, id: e.id, success: !0 }));
  } catch (t) {
    (u.error(`Failed to rename`, {
      oldPath: e.oldPath,
      newPath: e.newPath,
      error: t instanceof Error ? t.message : String(t),
    }),
      b({
        type: `rename`,
        id: e.id,
        success: !1,
        error: t instanceof Error ? t.message : String(t),
      }));
  }
}
async function Vt(e) {
  u.debug(`Checking file existence`, { path: e.path });
  try {
    let t = v();
    if (!t) throw Error(`Tonk not initialized`);
    let n = await t.tonk.exists(e.path);
    (u.debug(`File existence check completed`, { path: e.path, exists: n }),
      b({ type: `exists`, id: e.id, success: !0, data: n }));
  } catch (t) {
    (u.error(`Failed to check file existence`, {
      path: e.path,
      error: t instanceof Error ? t.message : String(t),
    }),
      b({
        type: `exists`,
        id: e.id,
        success: !1,
        error: t instanceof Error ? t.message : String(t),
      }));
  }
}
async function Ht(e) {
  u.debug(`Updating file with smart diff`, { path: e.path });
  try {
    let t = v();
    if (!t) throw Error(`Tonk not initialized`);
    let n = e.content,
      r = await t.tonk.updateFile(e.path, n);
    (u.debug(`File update completed`, { path: e.path, changed: r }),
      b({ type: `updateFile`, id: e.id, success: !0, data: r }));
  } catch (t) {
    (u.error(`Failed to update file`, {
      path: e.path,
      error: t instanceof Error ? t.message : String(t),
    }),
      b({
        type: `updateFile`,
        id: e.id,
        success: !1,
        error: t instanceof Error ? t.message : String(t),
      }));
  }
}
async function Ut(e) {
  u.debug(`Patching file`, { path: e.path, jsonPath: e.jsonPath });
  try {
    let t = v();
    if (!t) throw Error(`Tonk not initialized`);
    let n = e.value,
      r = await t.tonk.patchFile(e.path, e.jsonPath, n);
    (u.debug(`File patch completed`, { path: e.path, result: r }),
      b({ type: `patchFile`, id: e.id, success: !0, data: r }));
  } catch (t) {
    (u.error(`Failed to patch file`, {
      path: e.path,
      error: t instanceof Error ? t.message : String(t),
    }),
      b({
        type: `patchFile`,
        id: e.id,
        success: !1,
        error: t instanceof Error ? t.message : String(t),
      }));
  }
}
async function Wt(e) {
  u.debug(`Handling VFS init message`, {
    manifestSize: e.manifest.byteLength,
    wsUrl: e.wsUrl,
  });
  try {
    let e = g();
    if (e.status === `active`) {
      (u.debug(`Tonk already initialized`), b({ type: `init`, success: !0 }));
      return;
    }
    if (e.status === `loading`) {
      u.debug(`Tonk is loading, waiting for completion`);
      try {
        (await e.promise,
          u.debug(`Tonk loading completed`),
          b({ type: `init`, success: !0 }));
      } catch (e) {
        (u.error(`Tonk loading failed`, {
          error: e instanceof Error ? e.message : String(e),
        }),
          b({
            type: `init`,
            success: !1,
            error: e instanceof Error ? e.message : String(e),
          }));
      }
      return;
    }
    if (e.status === `error`) {
      (u.error(`Tonk initialization failed previously`, {
        error: e.error.message,
      }),
        b({ type: `init`, success: !1, error: e.error.message }));
      return;
    }
    (u.warn(`Tonk is uninitialized, this is unexpected`),
      b({ type: `init`, success: !1, error: `Tonk not initialized` }));
  } catch (e) {
    (u.error(`Failed to handle init message`, {
      error: e instanceof Error ? e.message : String(e),
    }),
      b({
        type: `init`,
        success: !1,
        error: e instanceof Error ? e.message : String(e),
      }));
  }
}
async function Gt(e) {
  u.debug(`Initializing from URL`, {
    manifestUrl: e.manifestUrl,
    wasmUrl: e.wasmUrl,
  });
  try {
    let t = e.manifestUrl || `http://localhost:8081/.manifest.tonk`;
    (await kt(), u.debug(`Fetching manifest from URL`, { manifestUrl: t }));
    let n = await fetch(t),
      r = new Uint8Array(await n.arrayBuffer());
    u.debug(`Manifest bytes loaded`, { byteLength: r.length });
    let i = await (await J.fromBytes(r)).getManifest();
    u.debug(`Bundle and manifest created`);
    let a = e.wsUrl || Q(i);
    u.debug(`Creating TonkCore from manifest bytes`);
    let o = await Y.fromBytes(r, { storage: { type: `indexeddb` } });
    (u.debug(`TonkCore created`),
      await m(a),
      u.debug(`Connecting to websocket`, { wsUrl: a }),
      await o.connectWebsocket(a),
      u.debug(`Websocket connection established`),
      await At(o),
      u.debug(`PathIndex sync complete`),
      y({
        status: `active`,
        bundleId: i.rootId,
        tonk: o,
        manifest: i,
        appSlug: i.entrypoints?.[0] || `app`,
        wsUrl: a,
        healthCheckInterval: null,
        watchers: new Map(),
        connectionHealthy: !0,
        reconnectAttempts: 0,
      }),
      Z(),
      await p(r),
      u.debug(`Bundle bytes persisted`),
      u.info(`Initialized from URL successfully`),
      b({ type: `initializeFromUrl`, id: e.id, success: !0 }));
  } catch (t) {
    (u.error(`Failed to initialize from URL`, {
      error: t instanceof Error ? t.message : String(t),
    }),
      y({ status: `error`, error: t instanceof Error ? t : Error(String(t)) }),
      b({
        type: `initializeFromUrl`,
        id: e.id,
        success: !1,
        error: t instanceof Error ? t.message : String(t),
      }));
  }
}
async function Kt(e) {
  u.debug(`Initializing from bytes`, {
    byteLength: e.bundleBytes.byteLength,
    serverUrl: e.serverUrl,
  });
  try {
    let t = e.serverUrl || `http://localhost:8081`;
    (u.debug(`Using server URL`, { serverUrl: t }), await kt());
    let n = new Uint8Array(e.bundleBytes);
    u.debug(`Creating bundle from bytes`, { byteLength: n.length });
    let r = await (await J.fromBytes(n)).getManifest();
    u.debug(`Bundle and manifest created`, { rootId: r.rootId });
    let i = e.wsUrl || Q(r, t);
    u.debug(`Creating TonkCore from bundle bytes`);
    let a = await Y.fromBytes(n, { storage: { type: `indexeddb` } });
    (u.debug(`TonkCore created`),
      u.debug(`Connecting to websocket`, { wsUrl: i }),
      i &&
        (await a.connectWebsocket(i),
        u.debug(`Websocket connection established`),
        await At(a),
        u.debug(`PathIndex sync complete`)),
      y({
        status: `active`,
        bundleId: r.rootId,
        tonk: a,
        manifest: r,
        appSlug: r.entrypoints?.[0] || `app`,
        wsUrl: i,
        healthCheckInterval: null,
        watchers: new Map(),
        connectionHealthy: !0,
        reconnectAttempts: 0,
      }),
      Z(),
      await p(n),
      u.debug(`Bundle bytes persisted`),
      u.info(`Initialized from bytes successfully`),
      b({ type: `initializeFromBytes`, id: e.id, success: !0 }));
  } catch (t) {
    (u.error(`Failed to initialize from bytes`, {
      error: t instanceof Error ? t.message : String(t),
    }),
      y({ status: `error`, error: t instanceof Error ? t : Error(String(t)) }),
      b({
        type: `initializeFromBytes`,
        id: e.id,
        success: !1,
        error: t instanceof Error ? t.message : String(t),
      }));
  }
}
async function qt(e) {
  (u.debug(`Getting server URL`),
    b({
      type: `getServerUrl`,
      id: e.id,
      success: !0,
      data: `http://localhost:8081`,
    }));
}
async function Jt(e) {
  u.debug(`Getting manifest`);
  try {
    let t = v();
    if (!t) throw Error(`Tonk not initialized`);
    b({ type: `getManifest`, id: e.id, success: !0, data: t.manifest });
  } catch (t) {
    (u.error(`Failed to get manifest`, {
      error: t instanceof Error ? t.message : String(t),
    }),
      b({
        type: `getManifest`,
        id: e.id,
        success: !1,
        error: t instanceof Error ? t.message : String(t),
      }));
  }
}
async function Yt() {
  let e = g();
  (u.debug(`Ping received`),
    b({ type: `ready`, status: e.status, needsBundle: e.status !== `active` }));
}
async function Xt(e) {
  (g().status === `active` && de(e.slug),
    await ne(e.slug),
    u.debug(`App slug set and persisted`, { slug: e.slug }));
}
async function Zt(e) {
  u.debug(`Starting file watch`, { path: e.path, watchId: e.id });
  try {
    let t = v();
    if (!t) throw Error(`Tonk not initialized`);
    let n = await t.tonk.watchFile(e.path, (t) => {
      (u.debug(`File change detected`, { watchId: e.id, path: e.path }),
        b({ type: `fileChanged`, watchId: e.id, documentData: t }));
    });
    (n && ge(e.id, n),
      u.debug(`File watch started`, { path: e.path, watchId: e.id }),
      b({ type: `watchFile`, id: e.id, success: !0 }));
  } catch (t) {
    (u.error(`Failed to start file watch`, {
      path: e.path,
      error: t instanceof Error ? t.message : String(t),
    }),
      b({
        type: `watchFile`,
        id: e.id,
        success: !1,
        error: t instanceof Error ? t.message : String(t),
      }));
  }
}
async function Qt(e) {
  u.debug(`Stopping file watch`, { watchId: e.id });
  try {
    (ve(e.id)
      ? (u.debug(`Found watcher, stopping it`, { watchId: e.id }),
        _e(e.id),
        u.debug(`File watch stopped`, { watchId: e.id }))
      : u.debug(`No watcher found for ID`, { watchId: e.id }),
      b({ type: `unwatchFile`, id: e.id, success: !0 }));
  } catch (t) {
    (u.error(`Failed to stop file watch`, {
      watchId: e.id,
      error: t instanceof Error ? t.message : String(t),
    }),
      b({
        type: `unwatchFile`,
        id: e.id,
        success: !1,
        error: t instanceof Error ? t.message : String(t),
      }));
  }
}
async function $t(e) {
  u.debug(`Starting directory watch`, { path: e.path, watchId: e.id });
  try {
    let t = v();
    if (!t) throw Error(`Tonk not initialized`);
    let n = await t.tonk.watchDirectory(e.path, (t) => {
      (u.debug(`Directory change detected`, { watchId: e.id, path: e.path }),
        b({
          type: `directoryChanged`,
          watchId: e.id,
          path: e.path,
          changeData: t,
        }));
    });
    (n && ge(e.id, n),
      u.debug(`Directory watch started`, { path: e.path, watchId: e.id }),
      b({ type: `watchDirectory`, id: e.id, success: !0 }));
  } catch (t) {
    (u.error(`Failed to start directory watch`, {
      path: e.path,
      error: t instanceof Error ? t.message : String(t),
    }),
      b({
        type: `watchDirectory`,
        id: e.id,
        success: !1,
        error: t instanceof Error ? t.message : String(t),
      }));
  }
}
async function en(e) {
  u.debug(`Stopping directory watch`, { watchId: e.id });
  try {
    (ve(e.id)
      ? (u.debug(`Found directory watcher, stopping it`, { watchId: e.id }),
        _e(e.id),
        u.debug(`Directory watch stopped`, { watchId: e.id }))
      : u.debug(`No directory watcher found for ID`, { watchId: e.id }),
      b({ type: `unwatchDirectory`, id: e.id, success: !0 }));
  } catch (t) {
    (u.error(`Failed to stop directory watch`, {
      watchId: e.id,
      error: t instanceof Error ? t.message : String(t),
    }),
      b({
        type: `unwatchDirectory`,
        id: e.id,
        success: !1,
        error: t instanceof Error ? t.message : String(t),
      }));
  }
}
var tn = [
  `init`,
  `loadBundle`,
  `initializeFromUrl`,
  `initializeFromBytes`,
  `getServerUrl`,
  `ping`,
  `setAppSlug`,
];
async function nn(e) {
  let t = e.type,
    n = e.id;
  u.debug(`Received message`, { type: t, id: n || `N/A` });
  let r = g();
  if (t === `setAppSlug`) {
    await Xt({ slug: e.slug });
    return;
  }
  if (t === `ping`) {
    await Yt();
    return;
  }
  if (r.status !== `active` && !tn.includes(t)) {
    (u.warn(`Operation attempted before VFS initialization`, {
      type: t,
      status: r.status,
    }),
      n &&
        b({
          type: t,
          id: n,
          success: !1,
          error: `VFS not initialized. Please load a bundle first.`,
        }));
    return;
  }
  switch (t) {
    case `init`:
      await Wt({
        ...(n !== void 0 && { id: n }),
        manifest: e.manifest,
        ...(e.wsUrl !== void 0 && { wsUrl: e.wsUrl }),
      });
      break;
    case `initializeFromUrl`:
      await Gt({
        ...(n !== void 0 && { id: n }),
        ...(e.manifestUrl !== void 0 && { manifestUrl: e.manifestUrl }),
        ...(e.wasmUrl !== void 0 && { wasmUrl: e.wasmUrl }),
        ...(e.wsUrl !== void 0 && { wsUrl: e.wsUrl }),
      });
      break;
    case `initializeFromBytes`:
      await Kt({
        ...(n !== void 0 && { id: n }),
        bundleBytes: e.bundleBytes,
        ...(e.serverUrl !== void 0 && { serverUrl: e.serverUrl }),
        ...(e.wsUrl !== void 0 && { wsUrl: e.wsUrl }),
      });
      break;
    case `getServerUrl`:
      await qt({ id: n });
      break;
    case `getManifest`:
      await Jt({ id: n });
      break;
    case `loadBundle`:
      await Nt({
        ...(n !== void 0 && { id: n }),
        bundleBytes: e.bundleBytes,
        ...(e.serverUrl !== void 0 && { serverUrl: e.serverUrl }),
        ...(e.manifest !== void 0 && { manifest: e.manifest }),
      });
      break;
    case `toBytes`:
      await Pt({ id: n });
      break;
    case `forkToBytes`:
      await Ft({ id: n });
      break;
    case `readFile`:
      await Lt({ id: n, path: e.path });
      break;
    case `writeFile`:
      await Rt({
        id: n,
        path: e.path,
        ...(e.create !== void 0 && { create: e.create }),
        content: e.content,
      });
      break;
    case `deleteFile`:
      await zt({ id: n, path: e.path });
      break;
    case `rename`:
      await Bt({ id: n, oldPath: e.oldPath, newPath: e.newPath });
      break;
    case `exists`:
      await Vt({ id: n, path: e.path });
      break;
    case `patchFile`:
      await Ut({ id: n, path: e.path, jsonPath: e.jsonPath, value: e.value });
      break;
    case `updateFile`:
      await Ht({ id: n, path: e.path, content: e.content });
      break;
    case `listDirectory`:
      await It({ id: n, path: e.path });
      break;
    case `watchFile`:
      await Zt({ id: n, path: e.path });
      break;
    case `unwatchFile`:
      await Qt({ id: n });
      break;
    case `watchDirectory`:
      await $t({ id: n, path: e.path });
      break;
    case `unwatchDirectory`:
      await en({ id: n });
      break;
    default:
      (u.warn(`Unknown message type`, { type: t }),
        n &&
          b({
            type: t,
            id: n,
            success: !1,
            error: `Unknown message type: ${t}`,
          }));
  }
}
var rn = self;
(u.info(`Service worker starting`, {
  version: `mj2zw6l0`,
  buildTime: `2025-12-12T15:01:34.931Z`,
  location: self.location.href,
}),
  u.debug(`Checking for cached state`),
  le(jt()),
  self.addEventListener(`install`, (e) => {
    (u.info(`Service worker installing`),
      rn.skipWaiting(),
      u.debug(`skipWaiting called`));
  }),
  self.addEventListener(`activate`, (e) => {
    (u.info(`Service worker activating`),
      e.waitUntil(
        (async () => {
          (await rn.clients.claim(), u.debug(`Clients claimed`));
          let e = await rn.clients.matchAll();
          (e.forEach((e) => {
            e.postMessage({ type: `ready`, needsBundle: !0 });
          }),
            u.info(`Service worker activated`, { clientCount: e.length }));
        })(),
      ));
  }),
  self.addEventListener(`message`, async (e) => {
    u.debug(`Message received`, { type: e.data?.type });
    try {
      await nn(e.data);
    } catch (e) {
      u.error(`Error handling message`, {
        error: e instanceof Error ? e.message : String(e),
      });
    }
  }),
  self.addEventListener(`fetch`, (e) => {
    Se(e);
  }),
  u.debug(`VFS Service Worker initialized`));
