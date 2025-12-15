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
  d = `tonk-sw-state-v3`,
  f = `/tonk-state/appSlug`,
  p = `/tonk-state/bundleBytes`,
  ee = `/tonk-state/wsUrl`,
  te = `/tonk-state/namespace`,
  ne = `/tonk-state/lastActiveBundleId`;
async function re(e) {
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
async function ie() {
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
async function m(e) {
  try {
    let t = await caches.open(d);
    if (e === null) await t.delete(p);
    else {
      let n = new Blob([e], { type: `application/octet-stream` }),
        r = new Response(n, {
          headers: { "Content-Type": `application/octet-stream` },
        });
      await t.put(p, r);
    }
    u.debug(`Bundle bytes persisted to cache`, { size: e ? e.length : 0 });
  } catch (e) {
    u.error(`Failed to persist bundle bytes`, {
      error: e instanceof Error ? e.message : String(e),
    });
  }
}
async function ae() {
  try {
    let e = await (await caches.open(d)).match(p);
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
async function h(e) {
  try {
    let t = await caches.open(d);
    if (e === null) await t.delete(ee);
    else {
      let n = new Response(JSON.stringify({ url: e }), {
        headers: { "Content-Type": `application/json` },
      });
      await t.put(ee, n);
    }
    u.debug(`WS URL persisted to cache`, { url: e });
  } catch (e) {
    u.error(`Failed to persist WS URL`, {
      error: e instanceof Error ? e.message : String(e),
    });
  }
}
async function oe() {
  try {
    let e = await (await caches.open(d)).match(ee);
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
async function g(e) {
  try {
    let t = await caches.open(d);
    if (e === null) await t.delete(te);
    else {
      let n = new Response(JSON.stringify({ namespace: e }), {
        headers: { "Content-Type": `application/json` },
      });
      await t.put(te, n);
    }
    u.debug(`Namespace persisted to cache`, { namespace: e });
  } catch (e) {
    u.error(`Failed to persist namespace`, {
      error: e instanceof Error ? e.message : String(e),
    });
  }
}
async function se() {
  try {
    let e = await (await caches.open(d)).match(te);
    return (e && (await e.json()).namespace) || null;
  } catch (e) {
    return (
      u.error(`Failed to restore namespace`, {
        error: e instanceof Error ? e.message : String(e),
      }),
      null
    );
  }
}
async function _(e) {
  try {
    let t = await caches.open(d);
    if (e === null) await t.delete(ne);
    else {
      let n = new Response(JSON.stringify({ id: e }), {
        headers: { "Content-Type": `application/json` },
      });
      await t.put(ne, n);
    }
    u.debug(`Last active bundle ID persisted to cache`, { id: e });
  } catch (e) {
    u.error(`Failed to persist last active bundle ID`, {
      error: e instanceof Error ? e.message : String(e),
    });
  }
}
async function ce() {
  try {
    let e = await (await caches.open(d)).match(ne);
    return (e && (await e.json()).id) || null;
  } catch (e) {
    return (
      u.error(`Failed to restore last active bundle ID`, {
        error: e instanceof Error ? e.message : String(e),
      }),
      null
    );
  }
}
async function le() {
  await Promise.all([re(null), m(null), h(null), g(null), _(null)]);
}
var v = new Map(),
  ue = null,
  de = null;
function fe() {
  return de;
}
function pe(e) {
  de = e;
}
function me() {
  return ue;
}
function y(e) {
  ((ue = e), u.debug(`Last active bundle ID updated`, { id: e }));
}
function b(e) {
  return v.get(e);
}
function x(e, t) {
  let n = v.get(e);
  (u.debug(`Setting bundle state`, {
    launcherBundleId: e,
    oldStatus: n?.status ?? `none`,
    newStatus: t.status,
  }),
    n?.status === `active` && ge(n),
    v.set(e, t));
}
function he(e) {
  let t = v.get(e);
  return t
    ? (u.debug(`Removing bundle state`, {
        launcherBundleId: e,
        status: t.status,
      }),
      t.status === `active` && ge(t),
      v.delete(e),
      ue === e && (ue = null),
      !0)
    : (u.debug(`Bundle state not found for removal`, { launcherBundleId: e }),
      !1);
}
function S(e) {
  let t = v.get(e);
  return t?.status === `active` ? { tonk: t.tonk, manifest: t.manifest } : null;
}
function C(e) {
  let t = v.get(e);
  return t?.status === `active` ? t : null;
}
function ge(e) {
  (u.debug(`Cleaning up active bundle state`, {
    launcherBundleId: e.launcherBundleId,
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
function _e(e, t) {
  let n = v.get(e);
  return n?.status === `active` ? (v.set(e, { ...n, appSlug: t }), !0) : !1;
}
function ve(e, t) {
  let n = v.get(e);
  n?.status === `active` && v.set(e, { ...n, connectionHealthy: t });
}
function ye(e) {
  let t = v.get(e);
  if (t?.status === `active`) {
    let n = t.reconnectAttempts + 1;
    return (v.set(e, { ...t, reconnectAttempts: n }), n);
  }
  return 0;
}
function be(e) {
  let t = v.get(e);
  t?.status === `active` && v.set(e, { ...t, reconnectAttempts: 0 });
}
function xe(e, t) {
  let n = v.get(e);
  n?.status === `active` && v.set(e, { ...n, healthCheckInterval: t });
}
function Se(e, t, n) {
  let r = v.get(e);
  r?.status === `active` && r.watchers.set(t, n);
}
function Ce(e, t) {
  let n = v.get(e);
  if (n?.status === `active`) {
    let e = n.watchers.get(t);
    if (e) {
      try {
        e.stop();
      } catch (e) {
        u.warn(`Error stopping watcher on remove`, {
          watcherId: t,
          error: e instanceof Error ? e.message : String(e),
        });
      }
      return (n.watchers.delete(t), !0);
    }
  }
  return !1;
}
function we(e, t) {
  let n = v.get(e);
  if (n?.status === `active`) return n.watchers.get(t);
}
function Te(e) {
  let t = v.get(e);
  return t?.status === `active` ? Array.from(t.watchers.entries()) : [];
}
function Ee(e) {
  let t = e.match(/^\/space\/([^/]+)\/([^/]+)(\/.*)?$/);
  return t
    ? { launcherBundleId: t[1], appSlug: t[2], remainingPath: t[3] || `/` }
    : null;
}
function De(e, t) {
  if (
    (console.log(`determinePath START`, {
      url: e.href,
      pathname: e.pathname,
      appSlug: t || `none`,
    }),
    !t)
  )
    throw (
      console.error(`determinePath - NO APP SLUG SET`),
      Error(`No app slug available for ${e.pathname}`)
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
  let a;
  if (
    (i.length >= 2 && i[1] === t
      ? ((a = i.slice(2)),
        console.log(
          `determinePath - new URL structure detected, using path after appSlug`,
          { launcherBundleId: i[0], appSlug: i[1], pathSegments: [...a] },
        ))
      : i[0] === t
        ? ((a = i.slice(1)),
          console.log(
            `determinePath - old URL structure, using path after appSlug`,
            { appSlug: i[0], pathSegments: [...a] },
          ))
        : ((a = i),
          console.log(
            `determinePath - unknown structure, using all segments as path`,
            { pathSegments: [...a] },
          )),
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
async function w(e) {
  (l(`info`, `Posting response to main thread`, {
    type: e.type,
    success: `success` in e ? e.success : `N/A`,
  }),
    (await self.clients.matchAll()).forEach((t) => {
      t.postMessage(e);
    }));
}
async function Oe(e) {
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
function ke(e) {
  let t = new URL(e.request.url),
    n = e.request.referrer;
  u.debug(`Fetch event`, { url: t.href, pathname: t.pathname });
  let r = e.request.headers.get(`upgrade`);
  if (r && r.toLowerCase() === `websocket`) {
    u.debug(`WebSocket upgrade request - passing through`, { url: t.href });
    return;
  }
  let i =
    t.pathname === `/` ||
    t.pathname === `` ||
    t.pathname === `/space/` ||
    t.pathname === `/space`;
  if (i) {
    Promise.all([re(null), m(null)]).catch((e) => {
      u.error(`Failed to persist state reset`, { error: e });
    });
    return;
  }
  let a = t.pathname.startsWith(`/space/_runtime/`),
    o = t.searchParams.has(`bundleId`),
    s = [
      `/space/_runtime/index.html`,
      `/space/_runtime/main.js`,
      `/space/_runtime/main.css`,
      `/space/service-worker-bundled.js`,
    ].some((e) => t.pathname === e),
    c =
      t.pathname.startsWith(`/space/_runtime/`) &&
      [`.otf`, `.ttf`, `.woff`, `.woff2`, `.eot`].some((e) =>
        t.pathname.endsWith(e),
      );
  if (s || (a && (o || c))) {
    u.debug(`Runtime static asset - passing through`, { pathname: t.pathname });
    return;
  }
  let l = Ee(t.pathname);
  if (l?.launcherBundleId === `_runtime`) {
    u.debug(`Reserved _runtime path - passing through`, {
      pathname: t.pathname,
    });
    return;
  }
  {
    let n = l?.appSlug;
    if (
      t.pathname.startsWith(`/@vite`) ||
      t.pathname.startsWith(`/@react-refresh`) ||
      t.pathname.startsWith(`/@fs/`) ||
      t.pathname.startsWith(`/src/`) ||
      t.pathname.startsWith(`/node_modules`) ||
      t.pathname.includes(`__vite__`) ||
      t.searchParams.has(`t`) ||
      (n &&
        (t.pathname.startsWith(`/${n}/@vite`) ||
          t.pathname.startsWith(`/${n}/node_modules`) ||
          t.pathname.startsWith(`/${n}/src/`)))
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
    if (n && !i) {
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
  }
  if (l && t.origin === location.origin && !i) {
    let { launcherBundleId: r, appSlug: i } = l;
    (u.debug(`Processing VFS request`, {
      pathname: t.pathname,
      launcherBundleId: r,
      appSlug: i,
      referrer: n,
    }),
      e.respondWith(
        (async () => {
          try {
            let e = De(t, i);
            u.debug(`Resolved VFS path`, {
              path: e,
              url: t.pathname,
              launcherBundleId: r,
              appSlug: i,
            });
            let n = fe(),
              a = b(r);
            if (n && (!a || a.status !== `active`)) {
              u.debug(`Waiting for initialization`, {
                launcherBundleId: r,
                status: a?.status ?? `none`,
              });
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
            let o = C(r);
            if (!o)
              throw (
                u.error(
                  `Tonk not initialized for bundle - cannot handle request`,
                  {
                    launcherBundleId: r,
                    status: b(r)?.status ?? `none`,
                    path: e,
                  },
                ),
                Error(`Bundle not initialized: ${r}`)
              );
            let s = `/${e}`;
            if (!(await o.tonk.exists(s))) {
              u.debug(`File not found, falling back to index.html`, {
                path: s,
              });
              let e = `/${i}/index.html`,
                t = await o.tonk.readFile(e);
              return Oe(t);
            }
            u.debug(`Serving file from VFS`, { path: s });
            let c = await o.tonk.readFile(s);
            return await Oe(c);
          } catch (e) {
            let n = e instanceof Error ? e.message : String(e);
            return (
              u.error(`Failed to fetch from VFS`, {
                error: n,
                url: t.href,
                launcherBundleId: r,
              }),
              new Response(
                `<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>Bundle Error</title>
  <style>
    body {
      font-family: system-ui, -apple-system, sans-serif;
      padding: 2rem;
      max-width: 600px;
      margin: 0 auto;
      background: #f5f5f5;
    }
    .error-container {
      background: white;
      border-radius: 8px;
      padding: 2rem;
      box-shadow: 0 2px 8px rgba(0,0,0,0.1);
    }
    h1 { color: #e53e3e; margin-top: 0; }
    .details { 
      background: #f7fafc; 
      padding: 1rem; 
      border-radius: 4px; 
      font-family: monospace;
      font-size: 0.875rem;
      word-break: break-all;
    }
    .actions { margin-top: 1.5rem; }
    button {
      background: #3182ce;
      color: white;
      border: none;
      padding: 0.5rem 1rem;
      border-radius: 4px;
      cursor: pointer;
      font-size: 1rem;
    }
    button:hover { background: #2c5282; }
  </style>
</head>
<body>
  <div class="error-container">
    <h1>Failed to Load Bundle</h1>
    <p>The application could not be loaded from the bundle.</p>
    <div class="details">
      <strong>Bundle ID:</strong> ${r}<br>
      <strong>Path:</strong> ${t.pathname}<br>
      <strong>Error:</strong> ${n}
    </div>
    <div class="actions">
      <button onclick="window.location.reload()">Retry</button>
    </div>
  </div>
</body>
</html>`,
                { status: 500, headers: { "Content-Type": `text/html` } },
              )
            );
          }
        })(),
      ));
  } else
    u.debug(
      `Ignoring fetch - not a valid /space/<bundleId>/<appSlug>/ request`,
      {
        requestOrigin: t.origin,
        swOrigin: location.origin,
        parsed: l ? `yes` : `no`,
        isRoot: i,
      },
    );
}
var Ae = `modulepreload`,
  je = function (e) {
    return `/` + e;
  },
  Me = {};
const T = function (e, t, n) {
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
        if (((t = je(t, n)), t in Me)) return;
        Me[t] = !0;
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
          ((o.rel = r ? `stylesheet` : Ae),
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
var E = n({
  WasmBundle: () => U,
  WasmDocHandle: () => mt,
  WasmDocumentWatcher: () => gt,
  WasmError: () => vt,
  WasmRepo: () => bt,
  WasmTonkCore: () => St,
  WasmWebSocketHandle: () => wt,
  create_bundle_from_bytes: () => Ve,
  create_tonk: () => qe,
  create_tonk_from_bundle: () => We,
  create_tonk_from_bundle_with_storage: () => Re,
  create_tonk_from_bytes: () => He,
  create_tonk_from_bytes_with_storage: () => Ke,
  create_tonk_with_config: () => ze,
  create_tonk_with_peer_id: () => Ge,
  create_tonk_with_storage: () => Be,
  default: () => Et,
  init: () => Ue,
  initSync: () => it,
  set_time_provider: () => Je,
});
function D() {
  return (
    (L === null || L.byteLength === 0) && (L = new Uint8Array(I.memory.buffer)),
    L
  );
}
function Ne(e, t) {
  return (
    (z += t),
    z >= ot &&
      ((R =
        typeof TextDecoder < `u`
          ? new TextDecoder(`utf-8`, { ignoreBOM: !0, fatal: !0 })
          : {
              decode: () => {
                throw Error(`TextDecoder not available`);
              },
            }),
      R.decode(),
      (z = t)),
    R.decode(D().subarray(e, e + t))
  );
}
function O(e, t) {
  return ((e >>>= 0), Ne(e, t));
}
function k(e, t, n) {
  if (n === void 0) {
    let n = V.encode(e),
      r = t(n.length, 1) >>> 0;
    return (
      D()
        .subarray(r, r + n.length)
        .set(n),
      (B = n.length),
      r
    );
  }
  let r = e.length,
    i = t(r, 1) >>> 0,
    a = D(),
    o = 0;
  for (; o < r; o++) {
    let t = e.charCodeAt(o);
    if (t > 127) break;
    a[i + o] = t;
  }
  if (o !== r) {
    (o !== 0 && (e = e.slice(o)),
      (i = n(i, r, (r = o + e.length * 3), 1) >>> 0));
    let t = D().subarray(i + o, i + r),
      a = st(e, t);
    ((o += a.written), (i = n(i, r, o, 1) >>> 0));
  }
  return ((B = o), i);
}
function A() {
  return (
    (H === null ||
      H.buffer.detached === !0 ||
      (H.buffer.detached === void 0 && H.buffer !== I.memory.buffer)) &&
      (H = new DataView(I.memory.buffer)),
    H
  );
}
function j(e) {
  let t = I.__externref_table_alloc();
  return (I.__wbindgen_export_4.set(t, e), t);
}
function M(e, t) {
  try {
    return e.apply(this, t);
  } catch (e) {
    let t = j(e);
    I.__wbindgen_exn_store(t);
  }
}
function N(e) {
  return e == null;
}
function Pe(e, t) {
  return ((e >>>= 0), D().subarray(e / 1, e / 1 + t));
}
function P(e, t, n, r) {
  let i = { a: e, b: t, cnt: 1, dtor: n },
    a = (...e) => {
      i.cnt++;
      let t = i.a;
      i.a = 0;
      try {
        return r(t, i.b, ...e);
      } finally {
        --i.cnt === 0
          ? (I.__wbindgen_export_6.get(i.dtor)(t, i.b), ct.unregister(i))
          : (i.a = t);
      }
    };
  return ((a.original = i), ct.register(a, i, i), a);
}
function Fe(e) {
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
    t > 0 && (n += Fe(e[0]));
    for (let r = 1; r < t; r++) n += `, ` + Fe(e[r]);
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
function F(e) {
  let t = I.__wbindgen_export_4.get(e);
  return (I.__externref_table_dealloc(e), t);
}
function Ie(e, t) {
  if (!(e instanceof t)) throw Error(`expected instance of ${t.name}`);
}
function Le(e, t) {
  let n = t(e.length * 1, 1) >>> 0;
  return (D().set(e, n / 1), (B = e.length), n);
}
function Re(e, t, n) {
  Ie(e, U);
  var r = N(n) ? 0 : k(n, I.__wbindgen_malloc, I.__wbindgen_realloc),
    i = B;
  return I.create_tonk_from_bundle_with_storage(e.__wbg_ptr, t, r, i);
}
function ze(e, t, n) {
  let r = k(e, I.__wbindgen_malloc, I.__wbindgen_realloc),
    i = B;
  var a = N(n) ? 0 : k(n, I.__wbindgen_malloc, I.__wbindgen_realloc),
    o = B;
  return I.create_tonk_with_config(r, i, t, a, o);
}
function Be(e, t) {
  var n = N(t) ? 0 : k(t, I.__wbindgen_malloc, I.__wbindgen_realloc),
    r = B;
  return I.create_tonk_with_storage(e, n, r);
}
function Ve(e) {
  let t = I.create_bundle_from_bytes(e);
  if (t[2]) throw F(t[1]);
  return U.__wrap(t[0]);
}
function He(e) {
  return I.create_tonk_from_bytes(e);
}
function Ue() {
  I.init();
}
function We(e) {
  return (Ie(e, U), I.create_tonk_from_bundle(e.__wbg_ptr));
}
function Ge(e) {
  let t = k(e, I.__wbindgen_malloc, I.__wbindgen_realloc),
    n = B;
  return I.create_tonk_with_peer_id(t, n);
}
function Ke(e, t, n) {
  var r = N(n) ? 0 : k(n, I.__wbindgen_malloc, I.__wbindgen_realloc),
    i = B;
  return I.create_tonk_from_bytes_with_storage(e, t, r, i);
}
function qe() {
  return I.create_tonk();
}
function Je(e) {
  I.set_time_provider(e);
}
function Ye(e, t, n) {
  I.closure730_externref_shim(e, t, n);
}
function Xe(e, t, n) {
  let r = I.closure733_externref_shim_multivalue_shim(e, t, n);
  if (r[1]) throw F(r[0]);
}
function Ze(e, t) {
  I.wasm_bindgen__convert__closures_____invoke__h546e6a644418f5b7(e, t);
}
function Qe(e, t, n) {
  I.closure1011_externref_shim(e, t, n);
}
function $e(e, t, n) {
  I.closure1027_externref_shim(e, t, n);
}
function et(e, t, n, r) {
  I.closure1285_externref_shim(e, t, n, r);
}
async function tt(e, t) {
  if (typeof Response == `function` && e instanceof Response) {
    if (typeof WebAssembly.instantiateStreaming == `function`)
      try {
        return await WebAssembly.instantiateStreaming(e, t);
      } catch (t) {
        if (
          e.ok &&
          Tt.has(e.type) &&
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
function nt() {
  let e = {};
  return (
    (e.wbg = {}),
    (e.wbg.__wbg_Error_0497d5bdba9362e5 = function (e, t) {
      return Error(O(e, t));
    }),
    (e.wbg.__wbg_String_8f0eb39a4a4c2f66 = function (e, t) {
      let n = k(String(t), I.__wbindgen_malloc, I.__wbindgen_realloc),
        r = B;
      (A().setInt32(e + 4, r, !0), A().setInt32(e + 0, n, !0));
    }),
    (e.wbg.__wbg_Window_41559019033ede94 = function (e) {
      return e.Window;
    }),
    (e.wbg.__wbg_WorkerGlobalScope_d324bffbeaef9f3a = function (e) {
      return e.WorkerGlobalScope;
    }),
    (e.wbg.__wbg_abort_601b12a63a2f3a3a = function () {
      return M(function (e) {
        e.abort();
      }, arguments);
    }),
    (e.wbg.__wbg_bound_eb572b424befade3 = function () {
      return M(function (e, t, n, r) {
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
      return M(function (e, t, n) {
        return e.call(t, n);
      }, arguments);
    }),
    (e.wbg.__wbg_call_fbe8be8bf6436ce5 = function () {
      return M(function (e, t) {
        return e.call(t);
      }, arguments);
    }),
    (e.wbg.__wbg_close_b08c03c920ee0bba = function () {
      return M(function (e) {
        e.close();
      }, arguments);
    }),
    (e.wbg.__wbg_commit_a54edce65f3858f2 = function () {
      return M(function (e) {
        e.commit();
      }, arguments);
    }),
    (e.wbg.__wbg_continue_7d9cdafc888cb902 = function () {
      return M(function (e) {
        e.continue();
      }, arguments);
    }),
    (e.wbg.__wbg_createObjectStore_b1f08961900155dd = function () {
      return M(function (e, t, n) {
        return e.createObjectStore(O(t, n));
      }, arguments);
    }),
    (e.wbg.__wbg_data_fffd43bf0ca75fff = function (e) {
      return e.data;
    }),
    (e.wbg.__wbg_delete_71b7921c73aa9378 = function () {
      return M(function (e, t) {
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
      console.error(O(e, t));
    }),
    (e.wbg.__wbg_error_4e978abc9692c0c5 = function () {
      return M(function (e) {
        let t = e.error;
        return N(t) ? 0 : j(t);
      }, arguments);
    }),
    (e.wbg.__wbg_error_51ecdd39ec054205 = function (e) {
      console.error(e);
    }),
    (e.wbg.__wbg_error_7534b8e9a36f1ab4 = function (e, t) {
      let n, r;
      try {
        ((n = e), (r = t), console.error(O(e, t)));
      } finally {
        I.__wbindgen_free(n, r, 1);
      }
    }),
    (e.wbg.__wbg_from_12ff8e47307bd4c7 = function (e) {
      return Array.from(e);
    }),
    (e.wbg.__wbg_getRandomValues_1c61fac11405ffdc = function () {
      return M(function (e, t) {
        globalThis.crypto.getRandomValues(Pe(e, t));
      }, arguments);
    }),
    (e.wbg.__wbg_getTime_2afe67905d873e92 = function (e) {
      return e.getTime();
    }),
    (e.wbg.__wbg_get_92470be87867c2e5 = function () {
      return M(function (e, t) {
        return Reflect.get(e, t);
      }, arguments);
    }),
    (e.wbg.__wbg_get_a131a44bd1eb6979 = function (e, t) {
      return e[t >>> 0];
    }),
    (e.wbg.__wbg_get_d37904b955701f99 = function () {
      return M(function (e, t) {
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
      return M(function (e) {
        let t = e.indexedDB;
        return N(t) ? 0 : j(t);
      }, arguments);
    }),
    (e.wbg.__wbg_indexedDB_54f01430b1e194e8 = function () {
      return M(function (e) {
        let t = e.indexedDB;
        return N(t) ? 0 : j(t);
      }, arguments);
    }),
    (e.wbg.__wbg_indexedDB_63b82e158eb67cbd = function () {
      return M(function (e) {
        let t = e.indexedDB;
        return N(t) ? 0 : j(t);
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
      var i = N(r) ? 0 : k(r, I.__wbindgen_malloc, I.__wbindgen_realloc),
        a = B;
      (A().setInt32(e + 4, a, !0), A().setInt32(e + 0, i, !0));
    }),
    (e.wbg.__wbg_iterator_4068add5b2aef7a6 = function () {
      return Symbol.iterator;
    }),
    (e.wbg.__wbg_key_a17a68df9ec1b180 = function () {
      return M(function (e) {
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
        ((c = e), (l = t), console.log(O(e, t), O(n, r), O(i, a), O(o, s)));
      } finally {
        I.__wbindgen_free(c, l, 1);
      }
    }),
    (e.wbg.__wbg_log_cb9e190acc5753fb = function (e, t) {
      let n, r;
      try {
        ((n = e), (r = t), console.log(O(e, t)));
      } finally {
        I.__wbindgen_free(n, r, 1);
      }
    }),
    (e.wbg.__wbg_log_ea240990d83e374e = function (e) {
      console.log(e);
    }),
    (e.wbg.__wbg_lowerBound_13c8e875a3fb9f7d = function () {
      return M(function (e, t) {
        return IDBKeyRange.lowerBound(e, t !== 0);
      }, arguments);
    }),
    (e.wbg.__wbg_mark_7438147ce31e9d4b = function (e, t) {
      performance.mark(O(e, t));
    }),
    (e.wbg.__wbg_measure_fb7825c11612c823 = function () {
      return M(function (e, t, n, r) {
        let i, a, o, s;
        try {
          ((i = e),
            (a = t),
            (o = n),
            (s = r),
            performance.measure(O(e, t), O(n, r)));
        } finally {
          (I.__wbindgen_free(i, a, 1), I.__wbindgen_free(o, s, 1));
        }
      }, arguments);
    }),
    (e.wbg.__wbg_message_2d95ea5aff0d63b9 = function (e, t) {
      let n = t.message,
        r = k(n, I.__wbindgen_malloc, I.__wbindgen_realloc),
        i = B;
      (A().setInt32(e + 4, i, !0), A().setInt32(e + 0, r, !0));
    }),
    (e.wbg.__wbg_message_4159c15dac08c5e9 = function (e) {
      return e.message;
    }),
    (e.wbg.__wbg_message_44ef9b801b7d8bc3 = function (e, t) {
      let n = t.message,
        r = k(n, I.__wbindgen_malloc, I.__wbindgen_realloc),
        i = B;
      (A().setInt32(e + 4, i, !0), A().setInt32(e + 0, r, !0));
    }),
    (e.wbg.__wbg_name_2acff1e83d9735f9 = function (e, t) {
      let n = t.name,
        r = k(n, I.__wbindgen_malloc, I.__wbindgen_realloc),
        i = B;
      (A().setInt32(e + 4, i, !0), A().setInt32(e + 0, r, !0));
    }),
    (e.wbg.__wbg_new0_97314565408dea38 = function () {
      return new Date();
    }),
    (e.wbg.__wbg_new_07b483f72211fd66 = function () {
      return {};
    }),
    (e.wbg.__wbg_new_476169e6d59f23ae = function (e, t) {
      return Error(O(e, t));
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
            return et(r, n.b, e, t);
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
      return M(function (e, t) {
        return new WebSocket(O(e, t));
      }, arguments);
    }),
    (e.wbg.__wbg_newfromslice_7c05ab1297cb2d88 = function (e, t) {
      return new Uint8Array(Pe(e, t));
    }),
    (e.wbg.__wbg_newnoargs_ff528e72d35de39a = function (e, t) {
      return Function(O(e, t));
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
      return M(function (e) {
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
      return M(function (e, t, n) {
        return e.objectStore(O(t, n));
      }, arguments);
    }),
    (e.wbg.__wbg_openCursor_7c13a2cd32c6258b = function () {
      return M(function (e) {
        return e.openCursor();
      }, arguments);
    }),
    (e.wbg.__wbg_open_0f04f50fa4d98f67 = function () {
      return M(function (e, t, n, r) {
        return e.open(O(t, n), r >>> 0);
      }, arguments);
    }),
    (e.wbg.__wbg_push_73fd7b5550ebf707 = function (e, t) {
      return e.push(t);
    }),
    (e.wbg.__wbg_put_7f0b4dcc666f09e3 = function () {
      return M(function (e, t, n) {
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
      return (ut.indexOf(t) + 1 || 3) - 1;
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
      return M(function (e) {
        return e.result;
      }, arguments);
    }),
    (e.wbg.__wbg_send_05456d2bf190b017 = function () {
      return M(function (e, t) {
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
      return M(function (e, t, n) {
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
      e.binaryType = lt[t];
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
        r = k(n, I.__wbindgen_malloc, I.__wbindgen_realloc),
        i = B;
      (A().setInt32(e + 4, i, !0), A().setInt32(e + 0, r, !0));
    }),
    (e.wbg.__wbg_static_accessor_GLOBAL_487c52c58d65314d = function () {
      let e = typeof globalThis > `u` ? null : globalThis;
      return N(e) ? 0 : j(e);
    }),
    (e.wbg.__wbg_static_accessor_GLOBAL_THIS_ee9704f328b6b291 = function () {
      let e = typeof globalThis > `u` ? null : globalThis;
      return N(e) ? 0 : j(e);
    }),
    (e.wbg.__wbg_static_accessor_SELF_78c9e3071b912620 = function () {
      let e = typeof self > `u` ? null : self;
      return N(e) ? 0 : j(e);
    }),
    (e.wbg.__wbg_static_accessor_WINDOW_a093d21393777366 = function () {
      let e = typeof self > `u` ? null : self;
      return N(e) ? 0 : j(e);
    }),
    (e.wbg.__wbg_target_15f1da583855ac4e = function (e) {
      let t = e.target;
      return N(t) ? 0 : j(t);
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
      return M(function (e, t, n, r) {
        return e.transaction(O(t, n), dt[r]);
      }, arguments);
    }),
    (e.wbg.__wbg_transaction_d1f21f4378880521 = function (e) {
      return e.transaction;
    }),
    (e.wbg.__wbg_upperBound_a0bd8ece19d98580 = function () {
      return M(function (e, t) {
        return IDBKeyRange.upperBound(e, t !== 0);
      }, arguments);
    }),
    (e.wbg.__wbg_value_17b896954e14f896 = function (e) {
      return e.value;
    }),
    (e.wbg.__wbg_value_648dc44894c8dc95 = function () {
      return M(function (e) {
        return e.value;
      }, arguments);
    }),
    (e.wbg.__wbg_wasmdochandle_new = function (e) {
      return mt.__wrap(e);
    }),
    (e.wbg.__wbg_wasmdocumentwatcher_new = function (e) {
      return gt.__wrap(e);
    }),
    (e.wbg.__wbg_wasmerror_new = function (e) {
      return vt.__wrap(e);
    }),
    (e.wbg.__wbg_wasmrepo_new = function (e) {
      return bt.__wrap(e);
    }),
    (e.wbg.__wbg_wasmtonkcore_new = function (e) {
      return St.__wrap(e);
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
      (A().setBigInt64(e + 8, N(r) ? BigInt(0) : r, !0),
        A().setInt32(e + 0, !N(r), !0));
    }),
    (e.wbg.__wbindgen_boolean_get = function (e) {
      let t = e;
      return typeof t == `boolean` ? (t ? 1 : 0) : 2;
    }),
    (e.wbg.__wbindgen_cb_drop = function (e) {
      let t = e.original;
      return t.cnt-- == 1 ? ((t.a = 0), !0) : !1;
    }),
    (e.wbg.__wbindgen_closure_wrapper1930 = function (e, t, n) {
      return P(e, t, 729, Ye);
    }),
    (e.wbg.__wbindgen_closure_wrapper1932 = function (e, t, n) {
      return P(e, t, 729, Ye);
    }),
    (e.wbg.__wbindgen_closure_wrapper1934 = function (e, t, n) {
      return P(e, t, 729, Xe);
    }),
    (e.wbg.__wbindgen_closure_wrapper1936 = function (e, t, n) {
      return P(e, t, 729, Ye);
    }),
    (e.wbg.__wbindgen_closure_wrapper2633 = function (e, t, n) {
      return P(e, t, 1008, Ze);
    }),
    (e.wbg.__wbindgen_closure_wrapper2635 = function (e, t, n) {
      return P(e, t, 1008, Qe);
    }),
    (e.wbg.__wbindgen_closure_wrapper2663 = function (e, t, n) {
      return P(e, t, 1026, $e);
    }),
    (e.wbg.__wbindgen_debug_string = function (e, t) {
      let n = Fe(t),
        r = k(n, I.__wbindgen_malloc, I.__wbindgen_realloc),
        i = B;
      (A().setInt32(e + 4, i, !0), A().setInt32(e + 0, r, !0));
    }),
    (e.wbg.__wbindgen_in = function (e, t) {
      return e in t;
    }),
    (e.wbg.__wbindgen_init_externref_table = function () {
      let e = I.__wbindgen_export_4,
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
      return I.memory;
    }),
    (e.wbg.__wbindgen_number_get = function (e, t) {
      let n = t,
        r = typeof n == `number` ? n : void 0;
      (A().setFloat64(e + 8, N(r) ? 0 : r, !0), A().setInt32(e + 0, !N(r), !0));
    }),
    (e.wbg.__wbindgen_number_new = function (e) {
      return e;
    }),
    (e.wbg.__wbindgen_string_get = function (e, t) {
      let n = t,
        r = typeof n == `string` ? n : void 0;
      var i = N(r) ? 0 : k(r, I.__wbindgen_malloc, I.__wbindgen_realloc),
        a = B;
      (A().setInt32(e + 4, a, !0), A().setInt32(e + 0, i, !0));
    }),
    (e.wbg.__wbindgen_string_new = function (e, t) {
      return O(e, t);
    }),
    (e.wbg.__wbindgen_throw = function (e, t) {
      throw Error(O(e, t));
    }),
    e
  );
}
function rt(e, t) {
  return (
    (I = e.exports),
    (at.__wbindgen_wasm_module = t),
    (H = null),
    (L = null),
    I.__wbindgen_start(),
    I
  );
}
function it(e) {
  if (I !== void 0) return I;
  e !== void 0 &&
    (Object.getPrototypeOf(e) === Object.prototype
      ? ({ module: e } = e)
      : console.warn(
          "using deprecated parameters for `initSync()`; pass a single object instead",
        ));
  let t = nt();
  e instanceof WebAssembly.Module || (e = new WebAssembly.Module(e));
  let n = new WebAssembly.Instance(e, t);
  return rt(n, e);
}
async function at(e) {
  if (I !== void 0) return I;
  (e !== void 0 &&
    (Object.getPrototypeOf(e) === Object.prototype
      ? ({ module_or_path: e } = e)
      : console.warn(
          `using deprecated parameters for the initialization function; pass a single object instead`,
        )),
    e === void 0 && (e = new URL(`/tonk_core_bg.wasm`, `` + import.meta.url)));
  let t = nt();
  (typeof e == `string` ||
    (typeof Request == `function` && e instanceof Request) ||
    (typeof URL == `function` && e instanceof URL)) &&
    (e = fetch(e));
  let { instance: n, module: r } = await tt(await e, t);
  return rt(n, r);
}
var I,
  L,
  R,
  ot,
  z,
  B,
  V,
  st,
  H,
  ct,
  lt,
  ut,
  dt,
  ft,
  U,
  pt,
  mt,
  ht,
  gt,
  _t,
  vt,
  yt,
  bt,
  xt,
  St,
  Ct,
  wt,
  Tt,
  Et,
  W = t(() => {
    ((L = null),
      (R =
        typeof TextDecoder < `u`
          ? new TextDecoder(`utf-8`, { ignoreBOM: !0, fatal: !0 })
          : {
              decode: () => {
                throw Error(`TextDecoder not available`);
              },
            }),
      typeof TextDecoder < `u` && R.decode(),
      (ot = 2146435072),
      (z = 0),
      (B = 0),
      (V =
        typeof TextEncoder < `u`
          ? new TextEncoder(`utf-8`)
          : {
              encode: () => {
                throw Error(`TextEncoder not available`);
              },
            }),
      (st =
        typeof V.encodeInto == `function`
          ? function (e, t) {
              return V.encodeInto(e, t);
            }
          : function (e, t) {
              let n = V.encode(e);
              return (t.set(n), { read: e.length, written: n.length });
            }),
      (H = null),
      (ct =
        typeof FinalizationRegistry > `u`
          ? { register: () => {}, unregister: () => {} }
          : new FinalizationRegistry((e) => {
              I.__wbindgen_export_6.get(e.dtor)(e.a, e.b);
            })),
      (lt = [`blob`, `arraybuffer`]),
      (ut = [`pending`, `done`]),
      (dt = [
        `readonly`,
        `readwrite`,
        `versionchange`,
        `readwriteflush`,
        `cleanup`,
      ]),
      (ft =
        typeof FinalizationRegistry > `u`
          ? { register: () => {}, unregister: () => {} }
          : new FinalizationRegistry((e) =>
              I.__wbg_wasmbundle_free(e >>> 0, 1),
            )),
      (U = class e {
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
          I.__wbg_wasmbundle_free(e, 0);
        }
        static fromBytes(t) {
          let n = I.wasmbundle_fromBytes(t);
          if (n[2]) throw F(n[1]);
          return e.__wrap(n[0]);
        }
        getPrefix(e) {
          let t = k(e, I.__wbindgen_malloc, I.__wbindgen_realloc),
            n = B;
          return I.wasmbundle_getPrefix(this.__wbg_ptr, t, n);
        }
        getRootId() {
          return I.wasmbundle_getRootId(this.__wbg_ptr);
        }
        getManifest() {
          return I.wasmbundle_getManifest(this.__wbg_ptr);
        }
        setManifest(e) {
          return I.wasmbundle_setManifest(this.__wbg_ptr, e);
        }
        get(e) {
          let t = k(e, I.__wbindgen_malloc, I.__wbindgen_realloc),
            n = B;
          return I.wasmbundle_get(this.__wbg_ptr, t, n);
        }
        toBytes() {
          return I.wasmbundle_toBytes(this.__wbg_ptr);
        }
        listKeys() {
          return I.wasmbundle_listKeys(this.__wbg_ptr);
        }
      }),
      (pt =
        typeof FinalizationRegistry > `u`
          ? { register: () => {}, unregister: () => {} }
          : new FinalizationRegistry((e) =>
              I.__wbg_wasmdochandle_free(e >>> 0, 1),
            )),
      (mt = class e {
        static __wrap(t) {
          t >>>= 0;
          let n = Object.create(e.prototype);
          return ((n.__wbg_ptr = t), pt.register(n, n.__wbg_ptr, n), n);
        }
        __destroy_into_raw() {
          let e = this.__wbg_ptr;
          return ((this.__wbg_ptr = 0), pt.unregister(this), e);
        }
        free() {
          let e = this.__destroy_into_raw();
          I.__wbg_wasmdochandle_free(e, 0);
        }
        documentId() {
          let e, t;
          try {
            let n = I.wasmdochandle_documentId(this.__wbg_ptr);
            return ((e = n[0]), (t = n[1]), O(n[0], n[1]));
          } finally {
            I.__wbindgen_free(e, t, 1);
          }
        }
        getDocument() {
          let e = I.wasmdochandle_getDocument(this.__wbg_ptr);
          if (e[2]) throw F(e[1]);
          return F(e[0]);
        }
        url() {
          let e, t;
          try {
            let n = I.wasmdochandle_url(this.__wbg_ptr);
            return ((e = n[0]), (t = n[1]), O(n[0], n[1]));
          } finally {
            I.__wbindgen_free(e, t, 1);
          }
        }
      }),
      (ht =
        typeof FinalizationRegistry > `u`
          ? { register: () => {}, unregister: () => {} }
          : new FinalizationRegistry((e) =>
              I.__wbg_wasmdocumentwatcher_free(e >>> 0, 1),
            )),
      (gt = class e {
        static __wrap(t) {
          t >>>= 0;
          let n = Object.create(e.prototype);
          return ((n.__wbg_ptr = t), ht.register(n, n.__wbg_ptr, n), n);
        }
        __destroy_into_raw() {
          let e = this.__wbg_ptr;
          return ((this.__wbg_ptr = 0), ht.unregister(this), e);
        }
        free() {
          let e = this.__destroy_into_raw();
          I.__wbg_wasmdocumentwatcher_free(e, 0);
        }
        documentId() {
          let e, t;
          try {
            let n = I.wasmdocumentwatcher_documentId(this.__wbg_ptr);
            return ((e = n[0]), (t = n[1]), O(n[0], n[1]));
          } finally {
            I.__wbindgen_free(e, t, 1);
          }
        }
        stop() {
          return I.wasmdocumentwatcher_stop(this.__wbg_ptr);
        }
      }),
      (_t =
        typeof FinalizationRegistry > `u`
          ? { register: () => {}, unregister: () => {} }
          : new FinalizationRegistry((e) =>
              I.__wbg_wasmerror_free(e >>> 0, 1),
            )),
      (vt = class e {
        static __wrap(t) {
          t >>>= 0;
          let n = Object.create(e.prototype);
          return ((n.__wbg_ptr = t), _t.register(n, n.__wbg_ptr, n), n);
        }
        __destroy_into_raw() {
          let e = this.__wbg_ptr;
          return ((this.__wbg_ptr = 0), _t.unregister(this), e);
        }
        free() {
          let e = this.__destroy_into_raw();
          I.__wbg_wasmerror_free(e, 0);
        }
        get message() {
          let e, t;
          try {
            let n = I.wasmerror_message(this.__wbg_ptr);
            return ((e = n[0]), (t = n[1]), O(n[0], n[1]));
          } finally {
            I.__wbindgen_free(e, t, 1);
          }
        }
      }),
      (yt =
        typeof FinalizationRegistry > `u`
          ? { register: () => {}, unregister: () => {} }
          : new FinalizationRegistry((e) => I.__wbg_wasmrepo_free(e >>> 0, 1))),
      (bt = class e {
        static __wrap(t) {
          t >>>= 0;
          let n = Object.create(e.prototype);
          return ((n.__wbg_ptr = t), yt.register(n, n.__wbg_ptr, n), n);
        }
        __destroy_into_raw() {
          let e = this.__wbg_ptr;
          return ((this.__wbg_ptr = 0), yt.unregister(this), e);
        }
        free() {
          let e = this.__destroy_into_raw();
          I.__wbg_wasmrepo_free(e, 0);
        }
        findDocument(e) {
          let t = k(e, I.__wbindgen_malloc, I.__wbindgen_realloc),
            n = B;
          return I.wasmrepo_findDocument(this.__wbg_ptr, t, n);
        }
        listDocuments() {
          let e = I.wasmrepo_listDocuments(this.__wbg_ptr);
          if (e[2]) throw F(e[1]);
          return F(e[0]);
        }
        createDocument(e) {
          return I.wasmrepo_createDocument(this.__wbg_ptr, e);
        }
        connectWebSocket(e) {
          let t = k(e, I.__wbindgen_malloc, I.__wbindgen_realloc),
            n = B,
            r = I.wasmrepo_connectWebSocket(this.__wbg_ptr, t, n);
          if (r[2]) throw F(r[1]);
          return wt.__wrap(r[0]);
        }
        connectWebSocketAsync(e) {
          let t = k(e, I.__wbindgen_malloc, I.__wbindgen_realloc),
            n = B;
          return I.wasmrepo_connectWebSocketAsync(this.__wbg_ptr, t, n);
        }
        constructor() {
          return I.wasmrepo_new();
        }
        stop() {
          return I.wasmrepo_stop(this.__wbg_ptr);
        }
        peerId() {
          let e, t;
          try {
            let n = I.wasmrepo_peerId(this.__wbg_ptr);
            return ((e = n[0]), (t = n[1]), O(n[0], n[1]));
          } finally {
            I.__wbindgen_free(e, t, 1);
          }
        }
      }),
      (xt =
        typeof FinalizationRegistry > `u`
          ? { register: () => {}, unregister: () => {} }
          : new FinalizationRegistry((e) =>
              I.__wbg_wasmtonkcore_free(e >>> 0, 1),
            )),
      (St = class e {
        static __wrap(t) {
          t >>>= 0;
          let n = Object.create(e.prototype);
          return ((n.__wbg_ptr = t), xt.register(n, n.__wbg_ptr, n), n);
        }
        __destroy_into_raw() {
          let e = this.__wbg_ptr;
          return ((this.__wbg_ptr = 0), xt.unregister(this), e);
        }
        free() {
          let e = this.__destroy_into_raw();
          I.__wbg_wasmtonkcore_free(e, 0);
        }
        static fromBytes(e) {
          return I.wasmtonkcore_fromBytes(e);
        }
        patchFile(e, t, n) {
          let r = k(e, I.__wbindgen_malloc, I.__wbindgen_realloc),
            i = B;
          return I.wasmtonkcore_patchFile(this.__wbg_ptr, r, i, t, n);
        }
        createFile(e, t) {
          let n = k(e, I.__wbindgen_malloc, I.__wbindgen_realloc),
            r = B;
          return I.wasmtonkcore_createFile(this.__wbg_ptr, n, r, t);
        }
        deleteFile(e) {
          let t = k(e, I.__wbindgen_malloc, I.__wbindgen_realloc),
            n = B;
          return I.wasmtonkcore_deleteFile(this.__wbg_ptr, t, n);
        }
        static fromBundle(e) {
          return (Ie(e, U), I.wasmtonkcore_fromBundle(e.__wbg_ptr));
        }
        getPeerId() {
          return I.wasmtonkcore_getPeerId(this.__wbg_ptr);
        }
        spliceText(e, t, n, r, i) {
          let a = k(e, I.__wbindgen_malloc, I.__wbindgen_realloc),
            o = B,
            s = k(i, I.__wbindgen_malloc, I.__wbindgen_realloc),
            c = B;
          return I.wasmtonkcore_spliceText(this.__wbg_ptr, a, o, t, n, r, s, c);
        }
        updateFile(e, t) {
          let n = k(e, I.__wbindgen_malloc, I.__wbindgen_realloc),
            r = B;
          return I.wasmtonkcore_updateFile(this.__wbg_ptr, n, r, t);
        }
        getMetadata(e) {
          let t = k(e, I.__wbindgen_malloc, I.__wbindgen_realloc),
            n = B;
          return I.wasmtonkcore_getMetadata(this.__wbg_ptr, t, n);
        }
        isConnected() {
          return I.wasmtonkcore_isConnected(this.__wbg_ptr);
        }
        static withPeerId(e) {
          let t = k(e, I.__wbindgen_malloc, I.__wbindgen_realloc),
            n = B;
          return I.wasmtonkcore_withPeerId(t, n);
        }
        forkToBytes(e) {
          return I.wasmtonkcore_forkToBytes(this.__wbg_ptr, e);
        }
        listDirectory(e) {
          let t = k(e, I.__wbindgen_malloc, I.__wbindgen_realloc),
            n = B;
          return I.wasmtonkcore_listDirectory(this.__wbg_ptr, t, n);
        }
        watchDocument(e, t) {
          let n = k(e, I.__wbindgen_malloc, I.__wbindgen_realloc),
            r = B;
          return I.wasmtonkcore_watchDocument(this.__wbg_ptr, n, r, t);
        }
        watchDirectory(e, t) {
          let n = k(e, I.__wbindgen_malloc, I.__wbindgen_realloc),
            r = B;
          return I.wasmtonkcore_watchDirectory(this.__wbg_ptr, n, r, t);
        }
        createDirectory(e) {
          let t = k(e, I.__wbindgen_malloc, I.__wbindgen_realloc),
            n = B;
          return I.wasmtonkcore_createDirectory(this.__wbg_ptr, t, n);
        }
        connectWebsocket(e) {
          let t = k(e, I.__wbindgen_malloc, I.__wbindgen_realloc),
            n = B;
          return I.wasmtonkcore_connectWebsocket(this.__wbg_ptr, t, n);
        }
        setFileWithBytes(e, t, n) {
          let r = k(e, I.__wbindgen_malloc, I.__wbindgen_realloc),
            i = B,
            a = Le(n, I.__wbindgen_malloc),
            o = B;
          return I.wasmtonkcore_setFileWithBytes(this.__wbg_ptr, r, i, t, a, o);
        }
        getConnectionState() {
          return I.wasmtonkcore_getConnectionState(this.__wbg_ptr);
        }
        createFileWithBytes(e, t, n) {
          let r = k(e, I.__wbindgen_malloc, I.__wbindgen_realloc),
            i = B,
            a = Le(n, I.__wbindgen_malloc),
            o = B;
          return I.wasmtonkcore_createFileWithBytes(
            this.__wbg_ptr,
            r,
            i,
            t,
            a,
            o,
          );
        }
        constructor() {
          return I.wasmtonkcore_new();
        }
        exists(e) {
          let t = k(e, I.__wbindgen_malloc, I.__wbindgen_realloc),
            n = B;
          return I.wasmtonkcore_exists(this.__wbg_ptr, t, n);
        }
        rename(e, t) {
          let n = k(e, I.__wbindgen_malloc, I.__wbindgen_realloc),
            r = B,
            i = k(t, I.__wbindgen_malloc, I.__wbindgen_realloc),
            a = B;
          return I.wasmtonkcore_rename(this.__wbg_ptr, n, r, i, a);
        }
        setFile(e, t) {
          let n = k(e, I.__wbindgen_malloc, I.__wbindgen_realloc),
            r = B;
          return I.wasmtonkcore_setFile(this.__wbg_ptr, n, r, t);
        }
        toBytes(e) {
          return I.wasmtonkcore_toBytes(this.__wbg_ptr, e);
        }
        readFile(e) {
          let t = k(e, I.__wbindgen_malloc, I.__wbindgen_realloc),
            n = B;
          return I.wasmtonkcore_readFile(this.__wbg_ptr, t, n);
        }
      }),
      (Ct =
        typeof FinalizationRegistry > `u`
          ? { register: () => {}, unregister: () => {} }
          : new FinalizationRegistry((e) =>
              I.__wbg_wasmwebsockethandle_free(e >>> 0, 1),
            )),
      (wt = class e {
        static __wrap(t) {
          t >>>= 0;
          let n = Object.create(e.prototype);
          return ((n.__wbg_ptr = t), Ct.register(n, n.__wbg_ptr, n), n);
        }
        __destroy_into_raw() {
          let e = this.__wbg_ptr;
          return ((this.__wbg_ptr = 0), Ct.unregister(this), e);
        }
        free() {
          let e = this.__destroy_into_raw();
          I.__wbg_wasmwebsockethandle_free(e, 0);
        }
        waitForDisconnect() {
          return I.wasmwebsockethandle_waitForDisconnect(this.__wbg_ptr);
        }
        close() {
          I.wasmwebsockethandle_close(this.__wbg_ptr);
        }
      }),
      (Tt = new Set([`basic`, `cors`, `default`])),
      (Et = at));
  }),
  Dt = function (e, t, n, r, i) {
    if (r === `m`) throw TypeError(`Private method is not writable`);
    if (r === `a` && !i)
      throw TypeError(`Private accessor was defined without a setter`);
    if (typeof t == `function` ? e !== t || !i : !t.has(e))
      throw TypeError(
        `Cannot write private member to an object whose class did not declare it`,
      );
    return (r === `a` ? i.call(e, n) : i ? (i.value = n) : t.set(e, n), n);
  },
  G = function (e, t, n, r) {
    if (n === `a` && !r)
      throw TypeError(`Private accessor was defined without a getter`);
    if (typeof t == `function` ? e !== t || !r : !t.has(e))
      throw TypeError(
        `Cannot read private member from an object whose class did not declare it`,
      );
    return n === `m` ? r : n === `a` ? r.call(e) : r ? r.value : t.get(e);
  },
  K,
  q,
  J = class extends Error {
    constructor(e, t) {
      (super(e), (this.code = t), (this.name = `TonkError`));
    }
  },
  Ot = class extends J {
    constructor(e) {
      (super(e, `CONNECTION_ERROR`), (this.name = `ConnectionError`));
    }
  },
  Y = class extends J {
    constructor(e) {
      (super(e, `FILESYSTEM_ERROR`), (this.name = `FileSystemError`));
    }
  },
  X = class extends J {
    constructor(e) {
      (super(e, `BUNDLE_ERROR`), (this.name = `BundleError`));
    }
  },
  Z = class e {
    constructor(e) {
      (K.set(this, void 0), Dt(this, K, e, `f`));
    }
    static async fromBytes(t, n) {
      try {
        let { create_bundle_from_bytes: r } =
          n || (await T(() => Promise.resolve().then(() => (W(), E)), void 0));
        return new e(r(t));
      } catch (e) {
        throw new X(`Failed to create bundle from bytes: ${e}`);
      }
    }
    async getRootId() {
      try {
        return await G(this, K, `f`).getRootId();
      } catch (e) {
        throw new X(`Failed to get root ID: ${e}`);
      }
    }
    async get(e) {
      try {
        let t = await G(this, K, `f`).get(e);
        return t === null ? null : t;
      } catch (t) {
        throw new X(`Failed to get key ${e}: ${t}`);
      }
    }
    async getPrefix(e) {
      try {
        return (await G(this, K, `f`).getPrefix(e)).map((e) => ({
          key: e.key,
          value: e.value,
        }));
      } catch (t) {
        throw new X(`Failed to get prefix ${e}: ${t}`);
      }
    }
    async listKeys() {
      try {
        return await G(this, K, `f`).listKeys();
      } catch (e) {
        throw new X(`Failed to list keys: ${e}`);
      }
    }
    async getManifest() {
      try {
        return await G(this, K, `f`).getManifest();
      } catch (e) {
        throw new X(`Failed to retrieve manifest: ${e}`);
      }
    }
    async setManifest(e) {
      try {
        await G(this, K, `f`).setManifest(e);
      } catch (e) {
        throw new X(`Failed to set manifest: ${e}`);
      }
    }
    async toBytes() {
      try {
        return await G(this, K, `f`).toBytes();
      } catch (e) {
        throw new X(`Failed to serialize bundle: ${e}`);
      }
    }
    free() {
      G(this, K, `f`).free();
    }
  };
K = new WeakMap();
var kt = class e {
  constructor(e) {
    (q.set(this, void 0), Dt(this, q, e, `f`));
  }
  static async create(t, n) {
    let r =
      n || (await T(() => Promise.resolve().then(() => (W(), E)), void 0));
    if (t?.peerId && t?.storage) {
      let { create_tonk_with_config: n } = r,
        i = await n(
          t.peerId,
          t.storage.type === `indexeddb`,
          t.storage.namespace,
        );
      return new e(i);
    } else if (t?.peerId) {
      let { create_tonk_with_peer_id: n } = r,
        i = await n(t.peerId);
      return new e(i);
    } else if (t?.storage) {
      let { create_tonk_with_storage: n } = r,
        i = await n(t.storage.type === `indexeddb`, t.storage.namespace);
      return new e(i);
    } else {
      let { create_tonk: t } = r,
        n = await t();
      return new e(n);
    }
  }
  static async createWithPeerId(t, n) {
    let { create_tonk_with_peer_id: r } =
        n || (await T(() => Promise.resolve().then(() => (W(), E)), void 0)),
      i = await r(t);
    return new e(i);
  }
  static async fromBundle(t, n, r) {
    let i =
      r || (await T(() => Promise.resolve().then(() => (W(), E)), void 0));
    if (n?.storage) {
      let { create_tonk_from_bundle_with_storage: r } = i,
        a = await r(t, n.storage.type === `indexeddb`, n.storage.namespace);
      return new e(a);
    } else {
      let { create_tonk_from_bundle: n } = i,
        r = await n(t);
      return new e(r);
    }
  }
  static async fromBytes(t, n, r) {
    let i =
      r || (await T(() => Promise.resolve().then(() => (W(), E)), void 0));
    if (n?.storage) {
      let { create_tonk_from_bytes_with_storage: r } = i,
        a = await r(t, n.storage.type === `indexeddb`, n.storage.namespace);
      return new e(a);
    } else {
      let { create_tonk_from_bytes: n } = i,
        r = await n(t);
      return new e(r);
    }
  }
  getPeerId() {
    return G(this, q, `f`).getPeerId();
  }
  async connectWebsocket(e) {
    try {
      await G(this, q, `f`).connectWebsocket(e);
    } catch (t) {
      throw new Ot(`Failed to connect to ${e}: ${t}`);
    }
  }
  async isConnected() {
    try {
      return await G(this, q, `f`).isConnected();
    } catch (e) {
      return (console.error(`🔌 [CORE-JS] isConnected() error:`, e), !1);
    }
  }
  async getConnectionState() {
    try {
      return await G(this, q, `f`).getConnectionState();
    } catch (e) {
      return (
        console.error(`📡 [CORE-JS] getConnectionState() error:`, e),
        `failed:` + String(e)
      );
    }
  }
  async forkToBytes(e) {
    try {
      return await G(this, q, `f`).forkToBytes(e);
    } catch (e) {
      throw new J(`Failed to serialize to bundle data: ${e}`);
    }
  }
  async toBytes(e) {
    try {
      return await G(this, q, `f`).toBytes(e);
    } catch (e) {
      throw new J(`Failed to serialize to bundle data: ${e}`);
    }
  }
  async createFile(e, t) {
    try {
      await G(this, q, `f`).createFile(e, t);
    } catch (t) {
      throw new Y(`Failed to create file at ${e}: ${t}`);
    }
  }
  async createFileWithBytes(e, t, n) {
    try {
      let r = typeof n == `string` ? At(n) : n;
      await G(this, q, `f`).createFileWithBytes(e, t, r);
    } catch (t) {
      throw new Y(`Failed to create file at ${e}: ${t}`);
    }
  }
  async readFile(e) {
    try {
      let t = await G(this, q, `f`).readFile(e);
      if (t === null) throw new Y(`File not found: ${e}`);
      let n;
      return (
        t.bytes && (n = jt(t.bytes)),
        {
          ...t,
          content:
            typeof t.content == `string` ? JSON.parse(t.content) : t.content,
          bytes: n,
        }
      );
    } catch (t) {
      throw t instanceof Y ? t : new Y(`Failed to read file at ${e}: ${t}`);
    }
  }
  async setFile(e, t) {
    try {
      return await G(this, q, `f`).setFile(e, t);
    } catch (t) {
      throw new Y(`Failed to set file at ${e}: ${t}`);
    }
  }
  async setFileWithBytes(e, t, n) {
    try {
      let r = typeof n == `string` ? At(n) : n;
      return await G(this, q, `f`).setFileWithBytes(e, t, r);
    } catch (t) {
      throw new Y(`Failed to set file at ${e}: ${t}`);
    }
  }
  async updateFile(e, t) {
    try {
      return await G(this, q, `f`).updateFile(e, t);
    } catch (t) {
      throw new Y(`Failed to update file at ${e}: ${t}`);
    }
  }
  async patchFile(e, t, n) {
    try {
      return await G(this, q, `f`).patchFile(e, t, n);
    } catch (t) {
      throw new Y(`Failed to patch file at ${e}: ${t}`);
    }
  }
  async spliceText(e, t, n, r, i) {
    try {
      return await G(this, q, `f`).spliceText(e, t, n, r, i);
    } catch (t) {
      throw new Y(`Failed to splice text at ${e}: ${t}`);
    }
  }
  async deleteFile(e) {
    try {
      return await G(this, q, `f`).deleteFile(e);
    } catch (t) {
      throw new Y(`Failed to delete file at ${e}: ${t}`);
    }
  }
  async createDirectory(e) {
    try {
      await G(this, q, `f`).createDirectory(e);
    } catch (t) {
      throw new Y(`Failed to create directory at ${e}: ${t}`);
    }
  }
  async listDirectory(e) {
    try {
      return (await G(this, q, `f`).listDirectory(e)).map((e) => ({
        name: e.name,
        type: e.type,
        timestamps: e.timestamps,
        pointer: e.pointer,
      }));
    } catch (t) {
      throw new Y(`Failed to list directory at ${e}: ${t}`);
    }
  }
  async exists(e) {
    try {
      return await G(this, q, `f`).exists(e);
    } catch (t) {
      throw new Y(`Failed to check existence of ${e}: ${t}`);
    }
  }
  async rename(e, t) {
    try {
      return await G(this, q, `f`).rename(e, t);
    } catch (n) {
      throw new Y(`Failed to rename ${e} to ${t}: ${n}`);
    }
  }
  async getMetadata(e) {
    try {
      let t = await G(this, q, `f`).getMetadata(e);
      if (t === null) throw new Y(`File or directory not found: ${e}`);
      return t;
    } catch (t) {
      throw t instanceof Y ? t : new Y(`Failed to get metadata for ${e}: ${t}`);
    }
  }
  async watchFile(e, t) {
    try {
      let n = await G(this, q, `f`).watchDocument(e, (e) => {
        let n;
        (e.bytes && (n = jt(e.bytes)),
          t({
            ...e,
            content:
              typeof e.content == `string` ? JSON.parse(e.content) : e.content,
            bytes: n,
          }));
      });
      if (n === null) throw new Y(`File not found: ${e}`);
      return n;
    } catch (t) {
      throw t instanceof Y
        ? t
        : new Y(`Failed to watch file at path ${e}: ${t}`);
    }
  }
  async watchDirectory(e, t) {
    try {
      let n = await G(this, q, `f`).watchDirectory(e, t);
      if (n === null) throw new Y(`Directory not found: ${e}`);
      return n;
    } catch (t) {
      throw t instanceof Y
        ? t
        : new Y(`Failed to watch directory at path ${e}: ${t}`);
    }
  }
  free() {
    G(this, q, `f`).free();
  }
};
q = new WeakMap();
var At = (e) => {
    let t = atob(e),
      n = new Uint8Array(t.length);
    for (let e = 0; e < t.length; e++) n[e] = t.charCodeAt(e);
    return n;
  },
  jt = (e) => {
    if (typeof e == `string`) return e;
    if (Array.isArray(e)) {
      let t = 8192,
        n = ``;
      for (let r = 0; r < e.length; r += t) {
        let i = e.slice(r, r + t);
        n += String.fromCharCode(...i);
      }
      return btoa(n);
    } else throw new Y(`Unrecognized bytes type in readFile ${typeof e}`);
  };
W();
var Mt = !1,
  Nt = null;
async function Pt(e) {
  if (!Mt)
    return (
      Nt ||
      ((Nt = (async () => {
        try {
          (e?.wasmPath ? await Et({ module_or_path: e.wasmPath }) : await Et(),
            (Mt = !0));
        } catch (e) {
          throw ((Nt = null), e);
        }
      })()),
      Nt)
    );
}
async function Ft(e) {
  let t = C(e);
  if (!t) return !1;
  try {
    return await t.tonk.isConnected();
  } catch (t) {
    return (
      u.error(`performHealthCheck() failed`, {
        launcherBundleId: e,
        error: t instanceof Error ? t.message : String(t),
      }),
      !1
    );
  }
}
async function It(e) {
  let t = C(e);
  if (!t) {
    u.error(`Cannot reconnect: no active bundle`, { launcherBundleId: e });
    return;
  }
  let n = t.wsUrl;
  if (!n) {
    u.error(`Cannot reconnect: wsUrl not stored`, { launcherBundleId: e });
    return;
  }
  let r = ye(e);
  (r >= 10 && be(e),
    u.debug(`Attempting to reconnect`, {
      launcherBundleId: e,
      attempt: r,
      maxAttempts: 10,
      wsUrl: n,
    }),
    await w({ type: `reconnecting`, launcherBundleId: e, attempt: r }));
  try {
    if (
      (await t.tonk.connectWebsocket(n),
      await new Promise((e) => setTimeout(e, 1e3)),
      await t.tonk.isConnected())
    )
      (ve(e, !0),
        be(e),
        u.info(`Reconnection successful`, { launcherBundleId: e }),
        await w({ type: `reconnected`, launcherBundleId: e }),
        await Lt(e));
    else throw Error(`Connection check failed after reconnect attempt`);
  } catch (t) {
    u.warn(`Reconnection failed`, {
      launcherBundleId: e,
      error: t instanceof Error ? t.message : String(t),
      attempt: r,
    });
    let n = Math.min(1e3 * 2 ** (r - 1), 3e4);
    (u.debug(`Scheduling next reconnect attempt`, {
      launcherBundleId: e,
      delayMs: n,
      nextAttempt: r + 1,
    }),
      setTimeout(() => It(e), n));
  }
}
async function Lt(e) {
  let t = Te(e);
  (u.debug(`Re-establishing watchers after reconnection`, {
    launcherBundleId: e,
    watcherCount: t.length,
  }),
    u.debug(`Watcher re-establishment complete`, {
      launcherBundleId: e,
      watcherCount: t.length,
    }),
    await w({
      type: `watchersReestablished`,
      launcherBundleId: e,
      count: t.length,
    }));
}
function Rt(e) {
  let t = C(e);
  if (!t) {
    u.warn(`Cannot start health monitoring: no active bundle`, {
      launcherBundleId: e,
    });
    return;
  }
  (t.healthCheckInterval && clearInterval(t.healthCheckInterval),
    u.debug(`Starting health monitoring`, {
      launcherBundleId: e,
      intervalMs: r,
    }));
  let n = setInterval(async () => {
    let t = await Ft(e),
      r = C(e);
    if (!r) {
      clearInterval(n);
      return;
    }
    !t && r.connectionHealthy
      ? (ve(e, !1),
        u.warn(`Connection lost, starting reconnection attempts`, {
          launcherBundleId: e,
        }),
        await w({ type: `disconnected`, launcherBundleId: e }),
        It(e))
      : t &&
        !r.connectionHealthy &&
        (ve(e, !0),
        be(e),
        u.debug(`Connection health restored`, { launcherBundleId: e }));
  }, r);
  xe(e, n);
}
function zt(e, t = `http://localhost:8081`) {
  return e.networkUris && e.networkUris.length > 0
    ? e.networkUris[0].replace(/^http/, `ws`)
    : t.replace(/^http/, `ws`);
}
var Q = null,
  Bt = !1;
async function Vt() {
  if (Bt) {
    u.debug(`WASM already initialized`);
    return;
  }
  return Q
    ? (u.debug(`WASM initialization in progress, waiting...`), Q)
    : (u.debug(`Starting WASM initialization`),
      (Q = (async () => {
        try {
          let e = `/tonk_core_bg.wasm?t=${Date.now()}`;
          (await Pt({ wasmPath: e }),
            (Bt = !0),
            u.info(`WASM initialization completed`));
        } catch (e) {
          throw (
            (Q = null),
            (Bt = !1),
            u.error(`WASM initialization failed`, {
              error: e instanceof Error ? e.message : String(e),
            }),
            e
          );
        }
      })()),
      Q);
}
async function $(e) {
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
async function Ht() {
  try {
    let e = await ce(),
      t = await ie(),
      n = await ae(),
      r = await oe(),
      i = await se();
    if (!t || !n) {
      u.debug(`No cached state found, waiting for initialization message`, {
        hasSlug: !!t,
        hasBundle: !!n,
      });
      return;
    }
    let a = e || i;
    if (!a) {
      u.debug(
        `No launcher bundle ID found in cache, waiting for initialization message`,
      );
      return;
    }
    (u.info(`Auto-initializing from cache`, {
      slug: t,
      bundleSize: n.length,
      launcherBundleId: a,
    }),
      await Vt());
    let o = await (await Z.fromBytes(n)).getManifest();
    u.debug(`Bundle and manifest restored from cache`, { rootId: o.rootId });
    let s = { type: `indexeddb`, namespace: a },
      c = await kt.fromBytes(n, { storage: s });
    u.debug(`TonkCore created from cached bundle`, { namespace: a });
    let l = r || zt(o);
    (!r && l && (await h(l)),
      u.debug(`Connecting to websocket...`, {
        wsUrl: l,
        localRootId: o.rootId,
      }),
      await c.connectWebsocket(l),
      u.debug(`Websocket connected`),
      await $(c),
      u.info(`Auto-initialization complete`),
      x(a, {
        status: `active`,
        bundleId: o.rootId,
        launcherBundleId: a,
        tonk: c,
        manifest: o,
        appSlug: t,
        wsUrl: l,
        healthCheckInterval: null,
        watchers: new Map(),
        connectionHealthy: !0,
        reconnectAttempts: 0,
      }),
      y(a),
      Rt(a));
  } catch (e) {
    (u.error(`Auto-initialization failed`, {
      error: e instanceof Error ? e.message : String(e),
    }),
      await le(),
      (await self.clients.matchAll()).forEach((e) => {
        e.postMessage({
          type: `needsReinit`,
          appSlug: null,
          reason: `Auto-initialization failed`,
        });
      }));
  }
}
async function Ut(e, t, n, r, i) {
  u.debug(`Loading bundle`, {
    byteLength: e.length,
    serverUrl: t,
    hasCachedManifest: !!i,
    launcherBundleId: n,
  });
  try {
    let r = b(n);
    if (r?.status === `active`)
      return (
        u.debug(`Bundle already loaded, skipping reload`, {
          launcherBundleId: n,
          bundleId: r.bundleId,
        }),
        y(n),
        await _(n),
        { success: !0, skipped: !0 }
      );
    if (r?.status === `loading`)
      return (
        u.debug(`Bundle is currently loading, waiting...`, {
          launcherBundleId: n,
        }),
        await r.promise,
        y(n),
        await _(n),
        { success: !0, skipped: !0 }
      );
    await Vt();
    let a;
    if (i)
      (u.info(`Using cached manifest, skipping Bundle.fromBytes`, {
        rootId: i.rootId,
      }),
        (a = i));
    else {
      u.debug(`No cached manifest, parsing bundle`, { byteLength: e.length });
      let t = await Z.fromBytes(e);
      ((a = await t.getManifest()),
        t.free(),
        u.debug(`Bundle manifest extracted`, { rootId: a.rootId }));
    }
    let o,
      s = new Promise((e) => {
        o = e;
      });
    (x(n, {
      status: `loading`,
      launcherBundleId: n,
      bundleId: a.rootId,
      promise: s,
    }),
      u.debug(`Creating new TonkCore from bundle bytes`, {
        launcherBundleId: n,
      }));
    let c = { type: `indexeddb`, namespace: n },
      l = await kt.fromBytes(e, { storage: c });
    u.debug(`New TonkCore created successfully`, {
      rootId: a.rootId,
      namespace: n,
    });
    let d = zt(a, t),
      f = new URLSearchParams(self.location.search).get(`bundle`);
    if (f)
      try {
        let e = atob(f),
          t = JSON.parse(e);
        t.wsUrl && (d = t.wsUrl);
      } catch (e) {
        u.warn(`Could not parse bundle config for wsUrl`, {
          error: e instanceof Error ? e.message : String(e),
        });
      }
    (u.debug(`Determined websocket URL`, { wsUrl: d, serverUrl: t }),
      await h(d),
      u.debug(`Connecting new tonk to websocket`, {
        wsUrl: d,
        localRootId: a.rootId,
      }),
      d &&
        (await l.connectWebsocket(d),
        u.debug(`Websocket connection established`),
        await $(l),
        u.debug(`PathIndex sync complete after loadBundle`)));
    let p = a.entrypoints?.[0] || `app`;
    return (
      x(n, {
        status: `active`,
        bundleId: a.rootId,
        launcherBundleId: n,
        tonk: l,
        manifest: a,
        appSlug: p,
        wsUrl: d,
        healthCheckInterval: null,
        watchers: new Map(),
        connectionHealthy: !0,
        reconnectAttempts: 0,
      }),
      y(n),
      await _(n),
      Rt(n),
      await m(e),
      await g(n),
      u.debug(`Bundle bytes and namespace persisted to cache`, {
        namespace: n,
      }),
      o(),
      u.info(`Bundle loaded successfully`, {
        rootId: a.rootId,
        launcherBundleId: n,
      }),
      { success: !0 }
    );
  } catch (e) {
    return (
      u.error(`Failed to load bundle`, {
        error: e instanceof Error ? e.message : String(e),
        launcherBundleId: n,
      }),
      x(n, {
        status: `error`,
        launcherBundleId: n,
        error: e instanceof Error ? e : Error(String(e)),
      }),
      { success: !1, error: e instanceof Error ? e.message : String(e) }
    );
  }
}
async function Wt(e) {
  u.debug(`Unloading bundle`, { launcherBundleId: e });
  let t = he(e);
  return (
    t
      ? (u.info(`Bundle unloaded successfully`, { launcherBundleId: e }),
        me() === null && (await le()))
      : u.warn(`Bundle not found for unload`, { launcherBundleId: e }),
    t
  );
}
async function Gt(e) {
  if (
    (u.debug(`Loading new bundle`, {
      byteLength: e.bundleBytes.byteLength,
      serverUrl: e.serverUrl,
      hasCachedManifest: !!e.manifest,
      launcherBundleId: e.launcherBundleId,
    }),
    !e.launcherBundleId)
  ) {
    w({
      type: `loadBundle`,
      id: e.id,
      success: !1,
      error: `launcherBundleId is required`,
    });
    return;
  }
  let t = e.serverUrl || `http://localhost:8081`,
    n = new Uint8Array(e.bundleBytes),
    r = await Ut(n, t, e.launcherBundleId, e.id, e.manifest);
  w({
    type: `loadBundle`,
    id: e.id,
    success: r.success,
    skipped: r.skipped,
    error: r.error,
  });
}
async function Kt(e) {
  if (
    (u.debug(`Unloading bundle`, { launcherBundleId: e.launcherBundleId }),
    !e.launcherBundleId)
  ) {
    w({
      type: `unloadBundle`,
      id: e.id,
      success: !1,
      error: `launcherBundleId is required`,
    });
    return;
  }
  let t = await Wt(e.launcherBundleId);
  w({ type: `unloadBundle`, id: e.id, success: t });
}
async function qt(e) {
  u.debug(`Converting tonk to bytes`, { launcherBundleId: e.launcherBundleId });
  try {
    let t = S(e.launcherBundleId);
    if (!t) throw Error(`Tonk not initialized`);
    let n = await t.tonk.toBytes(),
      r = t.manifest.rootId;
    (u.debug(`Tonk converted to bytes`, { byteLength: n.length, rootId: r }),
      w({ type: `toBytes`, id: e.id, success: !0, data: n, rootId: r }));
  } catch (t) {
    (u.error(`Failed to convert tonk to bytes`, {
      error: t instanceof Error ? t.message : String(t),
    }),
      w({
        type: `toBytes`,
        id: e.id,
        success: !1,
        error: t instanceof Error ? t.message : String(t),
      }));
  }
}
async function Jt(e) {
  u.debug(`Forking tonk to bytes`, { launcherBundleId: e.launcherBundleId });
  try {
    let t = S(e.launcherBundleId);
    if (!t) throw Error(`Tonk not initialized`);
    let n = await t.tonk.forkToBytes(),
      r = (await (await Z.fromBytes(n)).getManifest()).rootId;
    (u.debug(`Tonk forked to bytes`, { byteLength: n.length, rootId: r }),
      w({ type: `forkToBytes`, id: e.id, success: !0, data: n, rootId: r }));
  } catch (t) {
    (u.error(`Failed to fork tonk to bytes`, {
      error: t instanceof Error ? t.message : String(t),
    }),
      w({
        type: `forkToBytes`,
        id: e.id,
        success: !1,
        error: t instanceof Error ? t.message : String(t),
      }));
  }
}
async function Yt(e) {
  u.debug(`Listing directory`, {
    path: e.path,
    launcherBundleId: e.launcherBundleId,
  });
  try {
    let t = S(e.launcherBundleId);
    if (!t) throw Error(`Tonk not initialized`);
    let n = await t.tonk.listDirectory(e.path);
    (u.debug(`Directory listed`, {
      path: e.path,
      fileCount: Array.isArray(n) ? n.length : `unknown`,
    }),
      w({ type: `listDirectory`, id: e.id, success: !0, data: n }));
  } catch (t) {
    (u.error(`Failed to list directory`, {
      path: e.path,
      error: t instanceof Error ? t.message : String(t),
    }),
      w({
        type: `listDirectory`,
        id: e.id,
        success: !1,
        error: t instanceof Error ? t.message : String(t),
      }));
  }
}
async function Xt(e) {
  u.debug(`Reading file`, {
    path: e.path,
    launcherBundleId: e.launcherBundleId,
  });
  try {
    let t = S(e.launcherBundleId);
    if (!t) throw Error(`Tonk not initialized`);
    let n = await t.tonk.readFile(e.path);
    (u.debug(`File read successfully`, { path: e.path }),
      w({ type: `readFile`, id: e.id, success: !0, data: n }));
  } catch (t) {
    (u.error(`Failed to read file`, {
      path: e.path,
      error: t instanceof Error ? t.message : String(t),
    }),
      w({
        type: `readFile`,
        id: e.id,
        success: !1,
        error: t instanceof Error ? t.message : String(t),
      }));
  }
}
async function Zt(e) {
  u.debug(`Writing file`, {
    path: e.path,
    create: e.create,
    hasBytes: !!e.content.bytes,
    launcherBundleId: e.launcherBundleId,
  });
  try {
    let t = S(e.launcherBundleId);
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
      w({ type: `writeFile`, id: e.id, success: !0 }));
  } catch (t) {
    (u.error(`Failed to write file`, {
      path: e.path,
      create: e.create,
      error: t instanceof Error ? t.message : String(t),
    }),
      w({
        type: `writeFile`,
        id: e.id,
        success: !1,
        error: t instanceof Error ? t.message : String(t),
      }));
  }
}
async function Qt(e) {
  u.debug(`Deleting file`, {
    path: e.path,
    launcherBundleId: e.launcherBundleId,
  });
  try {
    let t = S(e.launcherBundleId);
    if (!t) throw Error(`Tonk not initialized`);
    (await t.tonk.deleteFile(e.path),
      u.debug(`File deleted successfully`, { path: e.path }),
      w({ type: `deleteFile`, id: e.id, success: !0 }));
  } catch (t) {
    (u.error(`Failed to delete file`, {
      path: e.path,
      error: t instanceof Error ? t.message : String(t),
    }),
      w({
        type: `deleteFile`,
        id: e.id,
        success: !1,
        error: t instanceof Error ? t.message : String(t),
      }));
  }
}
async function $t(e) {
  u.debug(`Renaming file or directory`, {
    oldPath: e.oldPath,
    newPath: e.newPath,
    launcherBundleId: e.launcherBundleId,
  });
  try {
    let t = S(e.launcherBundleId);
    if (!t) throw Error(`Tonk not initialized`);
    (await t.tonk.rename(e.oldPath, e.newPath),
      u.debug(`Rename completed`, { oldPath: e.oldPath, newPath: e.newPath }),
      w({ type: `rename`, id: e.id, success: !0 }));
  } catch (t) {
    (u.error(`Failed to rename`, {
      oldPath: e.oldPath,
      newPath: e.newPath,
      error: t instanceof Error ? t.message : String(t),
    }),
      w({
        type: `rename`,
        id: e.id,
        success: !1,
        error: t instanceof Error ? t.message : String(t),
      }));
  }
}
async function en(e) {
  u.debug(`Checking file existence`, {
    path: e.path,
    launcherBundleId: e.launcherBundleId,
  });
  try {
    let t = S(e.launcherBundleId);
    if (!t) throw Error(`Tonk not initialized`);
    let n = await t.tonk.exists(e.path);
    (u.debug(`File existence check completed`, { path: e.path, exists: n }),
      w({ type: `exists`, id: e.id, success: !0, data: n }));
  } catch (t) {
    (u.error(`Failed to check file existence`, {
      path: e.path,
      error: t instanceof Error ? t.message : String(t),
    }),
      w({
        type: `exists`,
        id: e.id,
        success: !1,
        error: t instanceof Error ? t.message : String(t),
      }));
  }
}
async function tn(e) {
  u.debug(`Updating file with smart diff`, {
    path: e.path,
    launcherBundleId: e.launcherBundleId,
  });
  try {
    let t = S(e.launcherBundleId);
    if (!t) throw Error(`Tonk not initialized`);
    let n = e.content,
      r = await t.tonk.updateFile(e.path, n);
    (u.debug(`File update completed`, { path: e.path, changed: r }),
      w({ type: `updateFile`, id: e.id, success: !0, data: r }));
  } catch (t) {
    (u.error(`Failed to update file`, {
      path: e.path,
      error: t instanceof Error ? t.message : String(t),
    }),
      w({
        type: `updateFile`,
        id: e.id,
        success: !1,
        error: t instanceof Error ? t.message : String(t),
      }));
  }
}
async function nn(e) {
  u.debug(`Patching file`, {
    path: e.path,
    jsonPath: e.jsonPath,
    launcherBundleId: e.launcherBundleId,
  });
  try {
    let t = S(e.launcherBundleId);
    if (!t) throw Error(`Tonk not initialized`);
    let n = e.value,
      r = await t.tonk.patchFile(e.path, e.jsonPath, n);
    (u.debug(`File patch completed`, { path: e.path, result: r }),
      w({ type: `patchFile`, id: e.id, success: !0, data: r }));
  } catch (t) {
    (u.error(`Failed to patch file`, {
      path: e.path,
      error: t instanceof Error ? t.message : String(t),
    }),
      w({
        type: `patchFile`,
        id: e.id,
        success: !1,
        error: t instanceof Error ? t.message : String(t),
      }));
  }
}
async function rn(e) {
  u.debug(`Handling VFS init message`, {
    manifestSize: e.manifest.byteLength,
    wsUrl: e.wsUrl,
    launcherBundleId: e.launcherBundleId,
  });
  try {
    let t = b(e.launcherBundleId);
    if (t?.status === `active`) {
      (u.debug(`Tonk already initialized for bundle`, {
        launcherBundleId: e.launcherBundleId,
      }),
        w({ type: `init`, success: !0 }));
      return;
    }
    if (t?.status === `loading`) {
      u.debug(`Tonk is loading, waiting for completion`, {
        launcherBundleId: e.launcherBundleId,
      });
      try {
        (await t.promise,
          u.debug(`Tonk loading completed`),
          w({ type: `init`, success: !0 }));
      } catch (e) {
        (u.error(`Tonk loading failed`, {
          error: e instanceof Error ? e.message : String(e),
        }),
          w({
            type: `init`,
            success: !1,
            error: e instanceof Error ? e.message : String(e),
          }));
      }
      return;
    }
    if (t?.status === `error`) {
      (u.error(`Tonk initialization failed previously`, {
        error: t.error.message,
      }),
        w({ type: `init`, success: !1, error: t.error.message }));
      return;
    }
    (u.warn(`Tonk is uninitialized, this is unexpected`, {
      launcherBundleId: e.launcherBundleId,
    }),
      w({ type: `init`, success: !1, error: `Tonk not initialized` }));
  } catch (e) {
    (u.error(`Failed to handle init message`, {
      error: e instanceof Error ? e.message : String(e),
    }),
      w({
        type: `init`,
        success: !1,
        error: e instanceof Error ? e.message : String(e),
      }));
  }
}
async function an(e) {
  u.debug(`Initializing from URL`, {
    manifestUrl: e.manifestUrl,
    wasmUrl: e.wasmUrl,
    launcherBundleId: e.launcherBundleId,
  });
  try {
    let t = e.manifestUrl || `http://localhost:8081/.manifest.tonk`;
    (await Vt(), u.debug(`Fetching manifest from URL`, { manifestUrl: t }));
    let n = await fetch(t),
      r = new Uint8Array(await n.arrayBuffer());
    u.debug(`Manifest bytes loaded`, { byteLength: r.length });
    let i = await (await Z.fromBytes(r)).getManifest();
    u.debug(`Bundle and manifest created`);
    let a = e.wsUrl || zt(i);
    u.debug(`Creating TonkCore from manifest bytes`);
    let o = await kt.fromBytes(r, {
      storage: { type: `indexeddb`, namespace: e.launcherBundleId },
    });
    (u.debug(`TonkCore created`),
      await h(a),
      u.debug(`Connecting to websocket`, { wsUrl: a }),
      await o.connectWebsocket(a),
      u.debug(`Websocket connection established`),
      await $(o),
      u.debug(`PathIndex sync complete`),
      x(e.launcherBundleId, {
        status: `active`,
        bundleId: i.rootId,
        launcherBundleId: e.launcherBundleId,
        tonk: o,
        manifest: i,
        appSlug: i.entrypoints?.[0] || `app`,
        wsUrl: a,
        healthCheckInterval: null,
        watchers: new Map(),
        connectionHealthy: !0,
        reconnectAttempts: 0,
      }),
      y(e.launcherBundleId),
      Rt(e.launcherBundleId),
      await m(r),
      await g(e.launcherBundleId),
      await _(e.launcherBundleId),
      u.debug(`Bundle bytes persisted`),
      u.info(`Initialized from URL successfully`),
      w({ type: `initializeFromUrl`, id: e.id, success: !0 }));
  } catch (t) {
    (u.error(`Failed to initialize from URL`, {
      error: t instanceof Error ? t.message : String(t),
    }),
      x(e.launcherBundleId, {
        status: `error`,
        launcherBundleId: e.launcherBundleId,
        error: t instanceof Error ? t : Error(String(t)),
      }),
      w({
        type: `initializeFromUrl`,
        id: e.id,
        success: !1,
        error: t instanceof Error ? t.message : String(t),
      }));
  }
}
async function on(e) {
  u.debug(`Initializing from bytes`, {
    byteLength: e.bundleBytes.byteLength,
    serverUrl: e.serverUrl,
    launcherBundleId: e.launcherBundleId,
  });
  try {
    let t = e.serverUrl || `http://localhost:8081`;
    (u.debug(`Using server URL`, { serverUrl: t }), await Vt());
    let n = new Uint8Array(e.bundleBytes);
    u.debug(`Creating bundle from bytes`, { byteLength: n.length });
    let r = await (await Z.fromBytes(n)).getManifest();
    u.debug(`Bundle and manifest created`, { rootId: r.rootId });
    let i = e.wsUrl || zt(r, t);
    u.debug(`Creating TonkCore from bundle bytes`);
    let a = await kt.fromBytes(n, {
      storage: { type: `indexeddb`, namespace: e.launcherBundleId },
    });
    (u.debug(`TonkCore created`),
      u.debug(`Connecting to websocket`, { wsUrl: i }),
      i &&
        (await a.connectWebsocket(i),
        u.debug(`Websocket connection established`),
        await $(a),
        u.debug(`PathIndex sync complete`)),
      x(e.launcherBundleId, {
        status: `active`,
        bundleId: r.rootId,
        launcherBundleId: e.launcherBundleId,
        tonk: a,
        manifest: r,
        appSlug: r.entrypoints?.[0] || `app`,
        wsUrl: i,
        healthCheckInterval: null,
        watchers: new Map(),
        connectionHealthy: !0,
        reconnectAttempts: 0,
      }),
      y(e.launcherBundleId),
      Rt(e.launcherBundleId),
      await m(n),
      await g(e.launcherBundleId),
      await _(e.launcherBundleId),
      u.debug(`Bundle bytes persisted`),
      u.info(`Initialized from bytes successfully`),
      w({ type: `initializeFromBytes`, id: e.id, success: !0 }));
  } catch (t) {
    (u.error(`Failed to initialize from bytes`, {
      error: t instanceof Error ? t.message : String(t),
    }),
      x(e.launcherBundleId, {
        status: `error`,
        launcherBundleId: e.launcherBundleId,
        error: t instanceof Error ? t : Error(String(t)),
      }),
      w({
        type: `initializeFromBytes`,
        id: e.id,
        success: !1,
        error: t instanceof Error ? t.message : String(t),
      }));
  }
}
async function sn(e) {
  (u.debug(`Getting server URL`),
    w({
      type: `getServerUrl`,
      id: e.id,
      success: !0,
      data: `http://localhost:8081`,
    }));
}
async function cn(e) {
  u.debug(`Getting manifest`, { launcherBundleId: e.launcherBundleId });
  try {
    let t = S(e.launcherBundleId);
    if (!t) throw Error(`Tonk not initialized`);
    w({ type: `getManifest`, id: e.id, success: !0, data: t.manifest });
  } catch (t) {
    (u.error(`Failed to get manifest`, {
      error: t instanceof Error ? t.message : String(t),
    }),
      w({
        type: `getManifest`,
        id: e.id,
        success: !1,
        error: t instanceof Error ? t.message : String(t),
      }));
  }
}
async function ln() {
  (u.debug(`Ping received`), w({ type: `ready`, needsBundle: !0 }));
}
async function un(e) {
  (_e(e.launcherBundleId, e.slug),
    await re(e.slug),
    u.debug(`App slug set and persisted`, {
      slug: e.slug,
      launcherBundleId: e.launcherBundleId,
    }));
}
async function dn(e) {
  u.debug(`Starting file watch`, {
    path: e.path,
    watchId: e.id,
    launcherBundleId: e.launcherBundleId,
  });
  try {
    let t = S(e.launcherBundleId);
    if (!t) throw Error(`Tonk not initialized`);
    let n = await t.tonk.watchFile(e.path, (t) => {
      (u.debug(`File change detected`, { watchId: e.id, path: e.path }),
        w({ type: `fileChanged`, watchId: e.id, documentData: t }));
    });
    (n && Se(e.launcherBundleId, e.id, n),
      u.debug(`File watch started`, { path: e.path, watchId: e.id }),
      w({ type: `watchFile`, id: e.id, success: !0 }));
  } catch (t) {
    (u.error(`Failed to start file watch`, {
      path: e.path,
      error: t instanceof Error ? t.message : String(t),
    }),
      w({
        type: `watchFile`,
        id: e.id,
        success: !1,
        error: t instanceof Error ? t.message : String(t),
      }));
  }
}
async function fn(e) {
  u.debug(`Stopping file watch`, {
    watchId: e.id,
    launcherBundleId: e.launcherBundleId,
  });
  try {
    (we(e.launcherBundleId, e.id)
      ? (u.debug(`Found watcher, stopping it`, { watchId: e.id }),
        Ce(e.launcherBundleId, e.id),
        u.debug(`File watch stopped`, { watchId: e.id }))
      : u.debug(`No watcher found for ID`, { watchId: e.id }),
      w({ type: `unwatchFile`, id: e.id, success: !0 }));
  } catch (t) {
    (u.error(`Failed to stop file watch`, {
      watchId: e.id,
      error: t instanceof Error ? t.message : String(t),
    }),
      w({
        type: `unwatchFile`,
        id: e.id,
        success: !1,
        error: t instanceof Error ? t.message : String(t),
      }));
  }
}
async function pn(e) {
  u.debug(`Starting directory watch`, {
    path: e.path,
    watchId: e.id,
    launcherBundleId: e.launcherBundleId,
  });
  try {
    let t = S(e.launcherBundleId);
    if (!t) throw Error(`Tonk not initialized`);
    let n = await t.tonk.watchDirectory(e.path, (t) => {
      (u.debug(`Directory change detected`, { watchId: e.id, path: e.path }),
        w({
          type: `directoryChanged`,
          watchId: e.id,
          path: e.path,
          changeData: t,
        }));
    });
    (n && Se(e.launcherBundleId, e.id, n),
      u.debug(`Directory watch started`, { path: e.path, watchId: e.id }),
      w({ type: `watchDirectory`, id: e.id, success: !0 }));
  } catch (t) {
    (u.error(`Failed to start directory watch`, {
      path: e.path,
      error: t instanceof Error ? t.message : String(t),
    }),
      w({
        type: `watchDirectory`,
        id: e.id,
        success: !1,
        error: t instanceof Error ? t.message : String(t),
      }));
  }
}
async function mn(e) {
  u.debug(`Stopping directory watch`, {
    watchId: e.id,
    launcherBundleId: e.launcherBundleId,
  });
  try {
    (we(e.launcherBundleId, e.id)
      ? (u.debug(`Found directory watcher, stopping it`, { watchId: e.id }),
        Ce(e.launcherBundleId, e.id),
        u.debug(`Directory watch stopped`, { watchId: e.id }))
      : u.debug(`No directory watcher found for ID`, { watchId: e.id }),
      w({ type: `unwatchDirectory`, id: e.id, success: !0 }));
  } catch (t) {
    (u.error(`Failed to stop directory watch`, {
      watchId: e.id,
      error: t instanceof Error ? t.message : String(t),
    }),
      w({
        type: `unwatchDirectory`,
        id: e.id,
        success: !1,
        error: t instanceof Error ? t.message : String(t),
      }));
  }
}
var hn = [
  `init`,
  `loadBundle`,
  `unloadBundle`,
  `initializeFromUrl`,
  `initializeFromBytes`,
  `getServerUrl`,
  `ping`,
  `setAppSlug`,
];
async function gn(e) {
  let t = e.type,
    n = e.id,
    r = e.launcherBundleId;
  if (
    (u.debug(`Received message`, {
      type: t,
      id: n || `N/A`,
      launcherBundleId: r || `N/A`,
    }),
    t === `ping`)
  ) {
    await ln();
    return;
  }
  if (t === `setAppSlug`) {
    if (!r) {
      u.warn(`setAppSlug missing launcherBundleId`);
      return;
    }
    await un({ launcherBundleId: r, slug: e.slug });
    return;
  }
  if (!hn.includes(t)) {
    let e = r || me();
    if (!e) {
      (u.warn(`Operation attempted without bundle context`, { type: t }),
        n &&
          w({
            type: t,
            id: n,
            success: !1,
            error: `No bundle context. Please load a bundle first.`,
          }));
      return;
    }
    let i = b(e);
    if (!i || i.status !== `active`) {
      (u.warn(`Operation attempted before bundle initialization`, {
        type: t,
        launcherBundleId: e,
        status: i?.status || `none`,
      }),
        n &&
          w({
            type: t,
            id: n,
            success: !1,
            error: `Bundle not initialized. Please load a bundle first.`,
          }));
      return;
    }
  }
  let i = r || me() || ``;
  switch (t) {
    case `init`:
      await rn({
        ...(n !== void 0 && { id: n }),
        manifest: e.manifest,
        ...(e.wsUrl !== void 0 && { wsUrl: e.wsUrl }),
        launcherBundleId: i,
      });
      break;
    case `initializeFromUrl`:
      await an({
        ...(n !== void 0 && { id: n }),
        ...(e.manifestUrl !== void 0 && { manifestUrl: e.manifestUrl }),
        ...(e.wasmUrl !== void 0 && { wasmUrl: e.wasmUrl }),
        ...(e.wsUrl !== void 0 && { wsUrl: e.wsUrl }),
        launcherBundleId: i,
      });
      break;
    case `initializeFromBytes`:
      await on({
        ...(n !== void 0 && { id: n }),
        bundleBytes: e.bundleBytes,
        ...(e.serverUrl !== void 0 && { serverUrl: e.serverUrl }),
        ...(e.wsUrl !== void 0 && { wsUrl: e.wsUrl }),
        launcherBundleId: i,
      });
      break;
    case `getServerUrl`:
      await sn({ id: n, launcherBundleId: i });
      break;
    case `getManifest`:
      await cn({ id: n, launcherBundleId: i });
      break;
    case `loadBundle`:
      await Gt({
        ...(n !== void 0 && { id: n }),
        bundleBytes: e.bundleBytes,
        ...(e.serverUrl !== void 0 && { serverUrl: e.serverUrl }),
        ...(e.manifest !== void 0 && { manifest: e.manifest }),
        launcherBundleId: e.launcherBundleId,
      });
      break;
    case `unloadBundle`:
      await Kt({
        ...(n !== void 0 && { id: n }),
        launcherBundleId: e.launcherBundleId,
      });
      break;
    case `toBytes`:
      await qt({ id: n, launcherBundleId: i });
      break;
    case `forkToBytes`:
      await Jt({ id: n, launcherBundleId: i });
      break;
    case `readFile`:
      await Xt({ id: n, path: e.path, launcherBundleId: i });
      break;
    case `writeFile`:
      await Zt({
        id: n,
        path: e.path,
        ...(e.create !== void 0 && { create: e.create }),
        content: e.content,
        launcherBundleId: i,
      });
      break;
    case `deleteFile`:
      await Qt({ id: n, path: e.path, launcherBundleId: i });
      break;
    case `rename`:
      await $t({
        id: n,
        oldPath: e.oldPath,
        newPath: e.newPath,
        launcherBundleId: i,
      });
      break;
    case `exists`:
      await en({ id: n, path: e.path, launcherBundleId: i });
      break;
    case `patchFile`:
      await nn({
        id: n,
        path: e.path,
        jsonPath: e.jsonPath,
        value: e.value,
        launcherBundleId: i,
      });
      break;
    case `updateFile`:
      await tn({
        id: n,
        path: e.path,
        content: e.content,
        launcherBundleId: i,
      });
      break;
    case `listDirectory`:
      await Yt({ id: n, path: e.path, launcherBundleId: i });
      break;
    case `watchFile`:
      await dn({ id: n, path: e.path, launcherBundleId: i });
      break;
    case `unwatchFile`:
      await fn({ id: n, launcherBundleId: i });
      break;
    case `watchDirectory`:
      await pn({ id: n, path: e.path, launcherBundleId: i });
      break;
    case `unwatchDirectory`:
      await mn({ id: n, launcherBundleId: i });
      break;
    default:
      (u.warn(`Unknown message type`, { type: t }),
        n &&
          w({
            type: t,
            id: n,
            success: !1,
            error: `Unknown message type: ${t}`,
          }));
  }
}
var _n = self;
(u.info(`Service worker starting`, {
  version: `mj73zisi`,
  buildTime: `2025-12-15T12:07:13.890Z`,
  location: self.location.href,
}),
  u.debug(`Checking for cached state`),
  pe(Ht()),
  self.addEventListener(`install`, (e) => {
    (u.info(`Service worker installing`),
      _n.skipWaiting(),
      u.debug(`skipWaiting called`));
  }),
  self.addEventListener(`activate`, (e) => {
    (u.info(`Service worker activating`),
      e.waitUntil(
        (async () => {
          (await _n.clients.claim(), u.debug(`Clients claimed`));
          let e = await _n.clients.matchAll();
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
      await gn(e.data);
    } catch (e) {
      u.error(`Error handling message`, {
        error: e instanceof Error ? e.message : String(e),
      });
    }
  }),
  self.addEventListener(`fetch`, (e) => {
    ke(e);
  }),
  u.debug(`VFS Service Worker initialized`));
