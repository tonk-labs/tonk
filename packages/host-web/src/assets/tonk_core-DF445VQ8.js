let _,
  i = 0,
  m = null;
function k() {
  return (
    (m === null || m.byteLength === 0) && (m = new Uint8Array(_.memory.buffer)),
    m
  );
}
const F =
    typeof TextEncoder < 'u'
      ? new TextEncoder('utf-8')
      : {
          encode: () => {
            throw Error('TextEncoder not available');
          },
        },
  P =
    typeof F.encodeInto == 'function'
      ? function (n, e) {
          return F.encodeInto(n, e);
        }
      : function (n, e) {
          const t = F.encode(n);
          return (e.set(t), { read: n.length, written: t.length });
        };
function a(n, e, t) {
  if (t === void 0) {
    const b = F.encode(n),
      u = e(b.length, 1) >>> 0;
    return (
      k()
        .subarray(u, u + b.length)
        .set(b),
      (i = b.length),
      u
    );
  }
  let r = n.length,
    o = e(r, 1) >>> 0;
  const c = k();
  let s = 0;
  for (; s < r; s++) {
    const b = n.charCodeAt(s);
    if (b > 127) break;
    c[o + s] = b;
  }
  if (s !== r) {
    (s !== 0 && (n = n.slice(s)),
      (o = t(o, r, (r = s + n.length * 3), 1) >>> 0));
    const b = k().subarray(o + s, o + r),
      u = P(n, b);
    ((s += u.written), (o = t(o, r, s, 1) >>> 0));
  }
  return ((i = s), o);
}
let d = null;
function w() {
  return (
    (d === null ||
      d.buffer.detached === !0 ||
      (d.buffer.detached === void 0 && d.buffer !== _.memory.buffer)) &&
      (d = new DataView(_.memory.buffer)),
    d
  );
}
function h(n) {
  const e = _.__externref_table_alloc();
  return (_.__wbindgen_export_4.set(e, n), e);
}
function g(n, e) {
  try {
    return n.apply(this, e);
  } catch (t) {
    const r = h(t);
    _.__wbindgen_exn_store(r);
  }
}
const B =
  typeof TextDecoder < 'u'
    ? new TextDecoder('utf-8', { ignoreBOM: !0, fatal: !0 })
    : {
        decode: () => {
          throw Error('TextDecoder not available');
        },
      };
typeof TextDecoder < 'u' && B.decode();
function f(n, e) {
  return ((n = n >>> 0), B.decode(k().subarray(n, n + e)));
}
function O(n, e) {
  return ((n = n >>> 0), k().subarray(n / 1, n / 1 + e));
}
function l(n) {
  return n == null;
}
const S =
  typeof FinalizationRegistry > 'u'
    ? { register: () => {}, unregister: () => {} }
    : new FinalizationRegistry(n => {
        _.__wbindgen_export_6.get(n.dtor)(n.a, n.b);
      });
function y(n, e, t, r) {
  const o = { a: n, b: e, cnt: 1, dtor: t },
    c = (...s) => {
      o.cnt++;
      const b = o.a;
      o.a = 0;
      try {
        return r(b, o.b, ...s);
      } finally {
        --o.cnt === 0
          ? (_.__wbindgen_export_6.get(o.dtor)(b, o.b), S.unregister(o))
          : (o.a = b);
      }
    };
  return ((c.original = o), S.register(c, o, o), c);
}
function R(n) {
  const e = typeof n;
  if (e == 'number' || e == 'boolean' || n == null) return `${n}`;
  if (e == 'string') return `"${n}"`;
  if (e == 'symbol') {
    const o = n.description;
    return o == null ? 'Symbol' : `Symbol(${o})`;
  }
  if (e == 'function') {
    const o = n.name;
    return typeof o == 'string' && o.length > 0 ? `Function(${o})` : 'Function';
  }
  if (Array.isArray(n)) {
    const o = n.length;
    let c = '[';
    o > 0 && (c += R(n[0]));
    for (let s = 1; s < o; s++) c += ', ' + R(n[s]);
    return ((c += ']'), c);
  }
  const t = /\[object ([^\]]+)\]/.exec(toString.call(n));
  let r;
  if (t && t.length > 1) r = t[1];
  else return toString.call(n);
  if (r == 'Object')
    try {
      return 'Object(' + JSON.stringify(n) + ')';
    } catch {
      return 'Object';
    }
  return n instanceof Error
    ? `${n.name}: ${n.message}
${n.stack}`
    : r;
}
function E(n) {
  const e = _.__wbindgen_export_4.get(n);
  return (_.__externref_table_dealloc(n), e);
}
function v(n, e) {
  if (!(n instanceof e)) throw new Error(`expected instance of ${e.name}`);
}
function N(n) {
  return (v(n, p), _.create_tonk_from_bundle(n.__wbg_ptr));
}
function K() {
  _.init();
}
function G(n) {
  const e = a(n, _.__wbindgen_malloc, _.__wbindgen_realloc),
    t = i;
  return _.create_tonk_with_peer_id(e, t);
}
function H(n) {
  const e = _.create_bundle_from_bytes(n);
  if (e[2]) throw E(e[1]);
  return p.__wrap(e[0]);
}
function J(n) {
  return _.create_tonk_from_bytes(n);
}
function Q() {
  return _.create_tonk();
}
function X(n) {
  _.set_time_provider(n);
}
function x(n, e, t) {
  _.closure452_externref_shim(n, e, t);
}
function U(n, e, t) {
  _.closure649_externref_shim(n, e, t);
}
function V(n, e, t, r) {
  _.closure948_externref_shim(n, e, t, r);
}
const $ = ['blob', 'arraybuffer'],
  W =
    typeof FinalizationRegistry > 'u'
      ? { register: () => {}, unregister: () => {} }
      : new FinalizationRegistry(n => _.__wbg_wasmbundle_free(n >>> 0, 1));
class p {
  static __wrap(e) {
    e = e >>> 0;
    const t = Object.create(p.prototype);
    return ((t.__wbg_ptr = e), W.register(t, t.__wbg_ptr, t), t);
  }
  __destroy_into_raw() {
    const e = this.__wbg_ptr;
    return ((this.__wbg_ptr = 0), W.unregister(this), e);
  }
  free() {
    const e = this.__destroy_into_raw();
    _.__wbg_wasmbundle_free(e, 0);
  }
  static fromBytes(e) {
    const t = _.wasmbundle_fromBytes(e);
    if (t[2]) throw E(t[1]);
    return p.__wrap(t[0]);
  }
  getPrefix(e) {
    const t = a(e, _.__wbindgen_malloc, _.__wbindgen_realloc),
      r = i;
    return _.wasmbundle_getPrefix(this.__wbg_ptr, t, r);
  }
  getRootId() {
    return _.wasmbundle_getRootId(this.__wbg_ptr);
  }
  getManifest() {
    return _.wasmbundle_getManifest(this.__wbg_ptr);
  }
  get(e) {
    const t = a(e, _.__wbindgen_malloc, _.__wbindgen_realloc),
      r = i;
    return _.wasmbundle_get(this.__wbg_ptr, t, r);
  }
  toBytes() {
    return _.wasmbundle_toBytes(this.__wbg_ptr);
  }
  listKeys() {
    return _.wasmbundle_listKeys(this.__wbg_ptr);
  }
}
const j =
  typeof FinalizationRegistry > 'u'
    ? { register: () => {}, unregister: () => {} }
    : new FinalizationRegistry(n => _.__wbg_wasmrepo_free(n >>> 0, 1));
class T {
  static __wrap(e) {
    e = e >>> 0;
    const t = Object.create(T.prototype);
    return ((t.__wbg_ptr = e), j.register(t, t.__wbg_ptr, t), t);
  }
  __destroy_into_raw() {
    const e = this.__wbg_ptr;
    return ((this.__wbg_ptr = 0), j.unregister(this), e);
  }
  free() {
    const e = this.__destroy_into_raw();
    _.__wbg_wasmrepo_free(e, 0);
  }
  getPeerId() {
    return _.wasmrepo_getPeerId(this.__wbg_ptr);
  }
  findDocument(e) {
    const t = a(e, _.__wbindgen_malloc, _.__wbindgen_realloc),
      r = i;
    return _.wasmrepo_findDocument(this.__wbg_ptr, t, r);
  }
  createDocument(e) {
    const t = a(e, _.__wbindgen_malloc, _.__wbindgen_realloc),
      r = i;
    return _.wasmrepo_createDocument(this.__wbg_ptr, t, r);
  }
}
const M =
  typeof FinalizationRegistry > 'u'
    ? { register: () => {}, unregister: () => {} }
    : new FinalizationRegistry(n => _.__wbg_wasmtonkcore_free(n >>> 0, 1));
class I {
  static __wrap(e) {
    e = e >>> 0;
    const t = Object.create(I.prototype);
    return ((t.__wbg_ptr = e), M.register(t, t.__wbg_ptr, t), t);
  }
  __destroy_into_raw() {
    const e = this.__wbg_ptr;
    return ((this.__wbg_ptr = 0), M.unregister(this), e);
  }
  free() {
    const e = this.__destroy_into_raw();
    _.__wbg_wasmtonkcore_free(e, 0);
  }
  static fromBytes(e) {
    return _.wasmtonkcore_fromBytes(e);
  }
  static fromBundle(e) {
    return (v(e, p), _.wasmtonkcore_fromBundle(e.__wbg_ptr));
  }
  getPeerId() {
    return _.wasmtonkcore_getPeerId(this.__wbg_ptr);
  }
  static withPeerId(e) {
    const t = a(e, _.__wbindgen_malloc, _.__wbindgen_realloc),
      r = i;
    return _.wasmtonkcore_withPeerId(t, r);
  }
  connectWebsocket(e) {
    const t = a(e, _.__wbindgen_malloc, _.__wbindgen_realloc),
      r = i;
    return _.wasmtonkcore_connectWebsocket(this.__wbg_ptr, t, r);
  }
  constructor() {
    return _.wasmtonkcore_new();
  }
  getVfs() {
    return _.wasmtonkcore_getVfs(this.__wbg_ptr);
  }
  getRepo() {
    return _.wasmtonkcore_getRepo(this.__wbg_ptr);
  }
  toBytes() {
    return _.wasmtonkcore_toBytes(this.__wbg_ptr);
  }
}
const D =
  typeof FinalizationRegistry > 'u'
    ? { register: () => {}, unregister: () => {} }
    : new FinalizationRegistry(n => _.__wbg_wasmvfs_free(n >>> 0, 1));
class A {
  static __wrap(e) {
    e = e >>> 0;
    const t = Object.create(A.prototype);
    return ((t.__wbg_ptr = e), D.register(t, t.__wbg_ptr, t), t);
  }
  __destroy_into_raw() {
    const e = this.__wbg_ptr;
    return ((this.__wbg_ptr = 0), D.unregister(this), e);
  }
  free() {
    const e = this.__destroy_into_raw();
    _.__wbg_wasmvfs_free(e, 0);
  }
  createFile(e, t) {
    const r = a(e, _.__wbindgen_malloc, _.__wbindgen_realloc),
      o = i;
    return _.wasmvfs_createFile(this.__wbg_ptr, r, o, t);
  }
  deleteFile(e) {
    const t = a(e, _.__wbindgen_malloc, _.__wbindgen_realloc),
      r = i;
    return _.wasmvfs_deleteFile(this.__wbg_ptr, t, r);
  }
  getMetadata(e) {
    const t = a(e, _.__wbindgen_malloc, _.__wbindgen_realloc),
      r = i;
    return _.wasmvfs_getMetadata(this.__wbg_ptr, t, r);
  }
  listDirectory(e) {
    const t = a(e, _.__wbindgen_malloc, _.__wbindgen_realloc),
      r = i;
    return _.wasmvfs_listDirectory(this.__wbg_ptr, t, r);
  }
  createDirectory(e) {
    const t = a(e, _.__wbindgen_malloc, _.__wbindgen_realloc),
      r = i;
    return _.wasmvfs_createDirectory(this.__wbg_ptr, t, r);
  }
  exists(e) {
    const t = a(e, _.__wbindgen_malloc, _.__wbindgen_realloc),
      r = i;
    return _.wasmvfs_exists(this.__wbg_ptr, t, r);
  }
  readFile(e) {
    const t = a(e, _.__wbindgen_malloc, _.__wbindgen_realloc),
      r = i;
    return _.wasmvfs_readFile(this.__wbg_ptr, t, r);
  }
}
async function C(n, e) {
  if (typeof Response == 'function' && n instanceof Response) {
    if (typeof WebAssembly.instantiateStreaming == 'function')
      try {
        return await WebAssembly.instantiateStreaming(n, e);
      } catch (r) {
        if (n.headers.get('Content-Type') != 'application/wasm')
          console.warn(
            '`WebAssembly.instantiateStreaming` failed because your server does not serve Wasm with `application/wasm` MIME type. Falling back to `WebAssembly.instantiate` which is slower. Original error:\n',
            r
          );
        else throw r;
      }
    const t = await n.arrayBuffer();
    return await WebAssembly.instantiate(t, e);
  } else {
    const t = await WebAssembly.instantiate(n, e);
    return t instanceof WebAssembly.Instance ? { instance: t, module: n } : t;
  }
}
function z() {
  const n = {};
  return (
    (n.wbg = {}),
    (n.wbg.__wbg_String_8f0eb39a4a4c2f66 = function (e, t) {
      const r = String(t),
        o = a(r, _.__wbindgen_malloc, _.__wbindgen_realloc),
        c = i;
      (w().setInt32(e + 4 * 1, c, !0), w().setInt32(e + 4 * 0, o, !0));
    }),
    (n.wbg.__wbg_buffer_09165b52af8c5237 = function (e) {
      return e.buffer;
    }),
    (n.wbg.__wbg_buffer_609cc3eee51ed158 = function (e) {
      return e.buffer;
    }),
    (n.wbg.__wbg_call_672a4d21634d4a24 = function () {
      return g(function (e, t) {
        return e.call(t);
      }, arguments);
    }),
    (n.wbg.__wbg_call_7cccdd69e0791ae2 = function () {
      return g(function (e, t, r) {
        return e.call(t, r);
      }, arguments);
    }),
    (n.wbg.__wbg_close_2893b7d056a0627d = function () {
      return g(function (e) {
        e.close();
      }, arguments);
    }),
    (n.wbg.__wbg_data_432d9c3df2630942 = function (e) {
      return e.data;
    }),
    (n.wbg.__wbg_error_31d7961b8e0d3f6c = function (e, t) {
      console.error(f(e, t));
    }),
    (n.wbg.__wbg_error_7534b8e9a36f1ab4 = function (e, t) {
      let r, o;
      try {
        ((r = e), (o = t), console.error(f(e, t)));
      } finally {
        _.__wbindgen_free(r, o, 1);
      }
    }),
    (n.wbg.__wbg_getRandomValues_38a1ff1ea09f6cc7 = function () {
      return g(function (e, t) {
        globalThis.crypto.getRandomValues(O(e, t));
      }, arguments);
    }),
    (n.wbg.__wbg_getRandomValues_3c9c0d586e575a16 = function () {
      return g(function (e, t) {
        globalThis.crypto.getRandomValues(O(e, t));
      }, arguments);
    }),
    (n.wbg.__wbg_getTime_46267b1c24877e30 = function (e) {
      return e.getTime();
    }),
    (n.wbg.__wbg_instanceof_ArrayBuffer_e14585432e3737fc = function (e) {
      let t;
      try {
        t = e instanceof ArrayBuffer;
      } catch {
        t = !1;
      }
      return t;
    }),
    (n.wbg.__wbg_length_a446193dc22c12f8 = function (e) {
      return e.length;
    }),
    (n.wbg.__wbg_log_2f4a53bbb94ad21b = function (e, t) {
      console.log(f(e, t));
    }),
    (n.wbg.__wbg_message_d1685a448ba00178 = function (e, t) {
      const r = t.message,
        o = a(r, _.__wbindgen_malloc, _.__wbindgen_realloc),
        c = i;
      (w().setInt32(e + 4 * 1, c, !0), w().setInt32(e + 4 * 0, o, !0));
    }),
    (n.wbg.__wbg_new0_f788a2397c7ca929 = function () {
      return new Date();
    }),
    (n.wbg.__wbg_new_23a2665fac83c611 = function (e, t) {
      try {
        var r = { a: e, b: t },
          o = (s, b) => {
            const u = r.a;
            r.a = 0;
            try {
              return V(u, r.b, s, b);
            } finally {
              r.a = u;
            }
          };
        return new Promise(o);
      } finally {
        r.a = r.b = 0;
      }
    }),
    (n.wbg.__wbg_new_405e22f390576ce2 = function () {
      return new Object();
    }),
    (n.wbg.__wbg_new_5e0be73521bc8c17 = function () {
      return new Map();
    }),
    (n.wbg.__wbg_new_78feb108b6472713 = function () {
      return new Array();
    }),
    (n.wbg.__wbg_new_8a6f238a6ece86ea = function () {
      return new Error();
    }),
    (n.wbg.__wbg_new_92c54fc74574ef55 = function () {
      return g(function (e, t) {
        return new WebSocket(f(e, t));
      }, arguments);
    }),
    (n.wbg.__wbg_new_a12002a7f91c75be = function (e) {
      return new Uint8Array(e);
    }),
    (n.wbg.__wbg_newnoargs_105ed471475aaf50 = function (e, t) {
      return new Function(f(e, t));
    }),
    (n.wbg.__wbg_newwithbyteoffsetandlength_d97e637ebe145a9a = function (
      e,
      t,
      r
    ) {
      return new Uint8Array(e, t >>> 0, r >>> 0);
    }),
    (n.wbg.__wbg_newwithlength_a381634e90c276d4 = function (e) {
      return new Uint8Array(e >>> 0);
    }),
    (n.wbg.__wbg_now_807e54c39636c349 = function () {
      return Date.now();
    }),
    (n.wbg.__wbg_push_737cfc8c1432c2c6 = function (e, t) {
      return e.push(t);
    }),
    (n.wbg.__wbg_queueMicrotask_97d92b4fcc8a61c5 = function (e) {
      queueMicrotask(e);
    }),
    (n.wbg.__wbg_queueMicrotask_d3219def82552485 = function (e) {
      return e.queueMicrotask;
    }),
    (n.wbg.__wbg_resolve_4851785c9c5f573d = function (e) {
      return Promise.resolve(e);
    }),
    (n.wbg.__wbg_send_7c4769e24cf1d784 = function () {
      return g(function (e, t) {
        e.send(t);
      }, arguments);
    }),
    (n.wbg.__wbg_set_37837023f3d740e8 = function (e, t, r) {
      e[t >>> 0] = r;
    }),
    (n.wbg.__wbg_set_3f1d0b984ed272ed = function (e, t, r) {
      e[t] = r;
    }),
    (n.wbg.__wbg_set_65595bdd868b3009 = function (e, t, r) {
      e.set(t, r >>> 0);
    }),
    (n.wbg.__wbg_set_8fc6bf8a5b1071d1 = function (e, t, r) {
      return e.set(t, r);
    }),
    (n.wbg.__wbg_set_bb8cecf6a62b9f46 = function () {
      return g(function (e, t, r) {
        return Reflect.set(e, t, r);
      }, arguments);
    }),
    (n.wbg.__wbg_setbinaryType_92fa1ffd873b327c = function (e, t) {
      e.binaryType = $[t];
    }),
    (n.wbg.__wbg_setonclose_14fc475a49d488fc = function (e, t) {
      e.onclose = t;
    }),
    (n.wbg.__wbg_setonerror_8639efe354b947cd = function (e, t) {
      e.onerror = t;
    }),
    (n.wbg.__wbg_setonmessage_6eccab530a8fb4c7 = function (e, t) {
      e.onmessage = t;
    }),
    (n.wbg.__wbg_setonopen_2da654e1f39745d5 = function (e, t) {
      e.onopen = t;
    }),
    (n.wbg.__wbg_stack_0ed75d68575b0f3c = function (e, t) {
      const r = t.stack,
        o = a(r, _.__wbindgen_malloc, _.__wbindgen_realloc),
        c = i;
      (w().setInt32(e + 4 * 1, c, !0), w().setInt32(e + 4 * 0, o, !0));
    }),
    (n.wbg.__wbg_static_accessor_GLOBAL_88a902d13a557d07 = function () {
      const e = typeof globalThis > 'u' ? null : globalThis;
      return l(e) ? 0 : h(e);
    }),
    (n.wbg.__wbg_static_accessor_GLOBAL_THIS_56578be7e9f832b0 = function () {
      const e = typeof globalThis > 'u' ? null : globalThis;
      return l(e) ? 0 : h(e);
    }),
    (n.wbg.__wbg_static_accessor_SELF_37c5d418e4bf5819 = function () {
      const e = typeof self > 'u' ? null : self;
      return l(e) ? 0 : h(e);
    }),
    (n.wbg.__wbg_static_accessor_WINDOW_5de37043a91a9c40 = function () {
      const e = typeof window > 'u' ? null : window;
      return l(e) ? 0 : h(e);
    }),
    (n.wbg.__wbg_then_44b73946d2fb3e7d = function (e, t) {
      return e.then(t);
    }),
    (n.wbg.__wbg_then_48b406749878a531 = function (e, t, r) {
      return e.then(t, r);
    }),
    (n.wbg.__wbg_wasmrepo_new = function (e) {
      return T.__wrap(e);
    }),
    (n.wbg.__wbg_wasmtonkcore_new = function (e) {
      return I.__wrap(e);
    }),
    (n.wbg.__wbg_wasmvfs_new = function (e) {
      return A.__wrap(e);
    }),
    (n.wbg.__wbindgen_bigint_from_i64 = function (e) {
      return e;
    }),
    (n.wbg.__wbindgen_bigint_from_u64 = function (e) {
      return BigInt.asUintN(64, e);
    }),
    (n.wbg.__wbindgen_cb_drop = function (e) {
      const t = e.original;
      return t.cnt-- == 1 ? ((t.a = 0), !0) : !1;
    }),
    (n.wbg.__wbindgen_closure_wrapper1426 = function (e, t, r) {
      return y(e, t, 650, U);
    }),
    (n.wbg.__wbindgen_closure_wrapper941 = function (e, t, r) {
      return y(e, t, 453, x);
    }),
    (n.wbg.__wbindgen_closure_wrapper943 = function (e, t, r) {
      return y(e, t, 453, x);
    }),
    (n.wbg.__wbindgen_closure_wrapper945 = function (e, t, r) {
      return y(e, t, 453, x);
    }),
    (n.wbg.__wbindgen_closure_wrapper947 = function (e, t, r) {
      return y(e, t, 453, x);
    }),
    (n.wbg.__wbindgen_debug_string = function (e, t) {
      const r = R(t),
        o = a(r, _.__wbindgen_malloc, _.__wbindgen_realloc),
        c = i;
      (w().setInt32(e + 4 * 1, c, !0), w().setInt32(e + 4 * 0, o, !0));
    }),
    (n.wbg.__wbindgen_error_new = function (e, t) {
      return new Error(f(e, t));
    }),
    (n.wbg.__wbindgen_init_externref_table = function () {
      const e = _.__wbindgen_export_4,
        t = e.grow(4);
      (e.set(0, void 0),
        e.set(t + 0, void 0),
        e.set(t + 1, null),
        e.set(t + 2, !0),
        e.set(t + 3, !1));
    }),
    (n.wbg.__wbindgen_is_function = function (e) {
      return typeof e == 'function';
    }),
    (n.wbg.__wbindgen_is_string = function (e) {
      return typeof e == 'string';
    }),
    (n.wbg.__wbindgen_is_undefined = function (e) {
      return e === void 0;
    }),
    (n.wbg.__wbindgen_memory = function () {
      return _.memory;
    }),
    (n.wbg.__wbindgen_number_get = function (e, t) {
      const r = t,
        o = typeof r == 'number' ? r : void 0;
      (w().setFloat64(e + 8 * 1, l(o) ? 0 : o, !0),
        w().setInt32(e + 4 * 0, !l(o), !0));
    }),
    (n.wbg.__wbindgen_number_new = function (e) {
      return e;
    }),
    (n.wbg.__wbindgen_string_get = function (e, t) {
      const r = t,
        o = typeof r == 'string' ? r : void 0;
      var c = l(o) ? 0 : a(o, _.__wbindgen_malloc, _.__wbindgen_realloc),
        s = i;
      (w().setInt32(e + 4 * 1, s, !0), w().setInt32(e + 4 * 0, c, !0));
    }),
    (n.wbg.__wbindgen_string_new = function (e, t) {
      return f(e, t);
    }),
    (n.wbg.__wbindgen_throw = function (e, t) {
      throw new Error(f(e, t));
    }),
    n
  );
}
function L(n, e) {
  return (
    (_ = n.exports),
    (q.__wbindgen_wasm_module = e),
    (d = null),
    (m = null),
    _.__wbindgen_start(),
    _
  );
}
function Y(n) {
  if (_ !== void 0) return _;
  typeof n < 'u' &&
    (Object.getPrototypeOf(n) === Object.prototype
      ? ({ module: n } = n)
      : console.warn(
          'using deprecated parameters for `initSync()`; pass a single object instead'
        ));
  const e = z();
  n instanceof WebAssembly.Module || (n = new WebAssembly.Module(n));
  const t = new WebAssembly.Instance(n, e);
  return L(t, n);
}
async function q(n) {
  if (_ !== void 0) return _;
  (typeof n < 'u' &&
    (Object.getPrototypeOf(n) === Object.prototype
      ? ({ module_or_path: n } = n)
      : console.warn(
          'using deprecated parameters for the initialization function; pass a single object instead'
        )),
    typeof n > 'u' &&
      (n = new URL('/assets/tonk_core_bg-BSpKFF6b.wasm', import.meta.url)));
  const e = z();
  (typeof n == 'string' ||
    (typeof Request == 'function' && n instanceof Request) ||
    (typeof URL == 'function' && n instanceof URL)) &&
    (n = fetch(n));
  const { instance: t, module: r } = await C(await n, e);
  return L(t, r);
}
export {
  p as WasmBundle,
  T as WasmRepo,
  I as WasmTonkCore,
  A as WasmVfs,
  H as create_bundle_from_bytes,
  Q as create_tonk,
  N as create_tonk_from_bundle,
  J as create_tonk_from_bytes,
  G as create_tonk_with_peer_id,
  q as default,
  K as init,
  Y as initSync,
  X as set_time_provider,
};
