let _,
  i = 0,
  k = null;
function h() {
  return (
    (k === null || k.byteLength === 0) && (k = new Uint8Array(_.memory.buffer)),
    k
  );
}
const I =
    typeof TextEncoder < 'u'
      ? new TextEncoder('utf-8')
      : {
          encode: () => {
            throw Error('TextEncoder not available');
          },
        },
  C =
    typeof I.encodeInto == 'function'
      ? function (n, t) {
          return I.encodeInto(n, t);
        }
      : function (n, t) {
          const e = I.encode(n);
          return (t.set(e), { read: n.length, written: e.length });
        };
function u(n, t, e) {
  if (e === void 0) {
    const g = I.encode(n),
      d = t(g.length, 1) >>> 0;
    return (
      h()
        .subarray(d, d + g.length)
        .set(g),
      (i = g.length),
      d
    );
  }
  let r = n.length,
    o = t(r, 1) >>> 0;
  const c = h();
  let b = 0;
  for (; b < r; b++) {
    const g = n.charCodeAt(b);
    if (g > 127) break;
    c[o + b] = g;
  }
  if (b !== r) {
    (b !== 0 && (n = n.slice(b)),
      (o = e(o, r, (r = b + n.length * 3), 1) >>> 0));
    const g = h().subarray(o + b, o + r),
      d = C(n, g);
    ((b += d.written), (o = e(o, r, b, 1) >>> 0));
  }
  return ((i = b), o);
}
let p = null;
function a() {
  return (
    (p === null ||
      p.buffer.detached === !0 ||
      (p.buffer.detached === void 0 && p.buffer !== _.memory.buffer)) &&
      (p = new DataView(_.memory.buffer)),
    p
  );
}
function l(n) {
  const t = _.__externref_table_alloc();
  return (_.__wbindgen_export_4.set(t, n), t);
}
function s(n, t) {
  try {
    return n.apply(this, t);
  } catch (e) {
    const r = l(e);
    _.__wbindgen_exn_store(r);
  }
}
const v =
  typeof TextDecoder < 'u'
    ? new TextDecoder('utf-8', { ignoreBOM: !0, fatal: !0 })
    : {
        decode: () => {
          throw Error('TextDecoder not available');
        },
      };
typeof TextDecoder < 'u' && v.decode();
function f(n, t) {
  return ((n = n >>> 0), v.decode(h().subarray(n, n + t)));
}
function w(n) {
  return n == null;
}
function F(n, t) {
  return ((n = n >>> 0), h().subarray(n / 1, n / 1 + t));
}
const j =
  typeof FinalizationRegistry > 'u'
    ? { register: () => {}, unregister: () => {} }
    : new FinalizationRegistry(n => {
        _.__wbindgen_export_6.get(n.dtor)(n.a, n.b);
      });
function m(n, t, e, r) {
  const o = { a: n, b: t, cnt: 1, dtor: e },
    c = (...b) => {
      o.cnt++;
      const g = o.a;
      o.a = 0;
      try {
        return r(g, o.b, ...b);
      } finally {
        --o.cnt === 0
          ? (_.__wbindgen_export_6.get(o.dtor)(g, o.b), j.unregister(o))
          : (o.a = g);
      }
    };
  return ((c.original = o), j.register(c, o, o), c);
}
function B(n) {
  const t = typeof n;
  if (t == 'number' || t == 'boolean' || n == null) return `${n}`;
  if (t == 'string') return `"${n}"`;
  if (t == 'symbol') {
    const o = n.description;
    return o == null ? 'Symbol' : `Symbol(${o})`;
  }
  if (t == 'function') {
    const o = n.name;
    return typeof o == 'string' && o.length > 0 ? `Function(${o})` : 'Function';
  }
  if (Array.isArray(n)) {
    const o = n.length;
    let c = '[';
    o > 0 && (c += B(n[0]));
    for (let b = 1; b < o; b++) c += ', ' + B(n[b]);
    return ((c += ']'), c);
  }
  const e = /\[object ([^\]]+)\]/.exec(toString.call(n));
  let r;
  if (e && e.length > 1) r = e[1];
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
function S(n) {
  const t = _.__wbindgen_export_4.get(n);
  return (_.__externref_table_dealloc(n), t);
}
function D(n, t) {
  if (!(n instanceof t)) throw new Error(`expected instance of ${t.name}`);
}
function A(n, t) {
  const e = t(n.length * 1, 1) >>> 0;
  return (h().set(n, e / 1), (i = n.length), e);
}
function J(n) {
  return (D(n, y), _.create_tonk_from_bundle(n.__wbg_ptr));
}
function Q(n) {
  return _.create_tonk_from_bytes(n);
}
function X(n) {
  const t = _.create_bundle_from_bytes(n);
  if (t[2]) throw S(t[1]);
  return y.__wrap(t[0]);
}
function Y() {
  return _.create_tonk();
}
function Z(n, t) {
  return (D(n, y), _.create_tonk_from_bundle_with_storage(n.__wbg_ptr, t));
}
function tt(n, t) {
  return _.create_tonk_from_bytes_with_storage(n, t);
}
function et(n, t) {
  const e = u(n, _.__wbindgen_malloc, _.__wbindgen_realloc),
    r = i;
  return _.create_tonk_with_config(e, r, t);
}
function nt(n) {
  return _.create_tonk_with_storage(n);
}
function rt() {
  _.init();
}
function _t(n) {
  const t = u(n, _.__wbindgen_malloc, _.__wbindgen_realloc),
    e = i;
  return _.create_tonk_with_peer_id(t, e);
}
function ot(n) {
  _.set_time_provider(n);
}
function x(n, t, e) {
  _.closure246_externref_shim(n, t, e);
}
function U(n, t, e) {
  const r = _.closure250_externref_shim_multivalue_shim(n, t, e);
  if (r[1]) throw S(r[0]);
}
function z(n, t, e) {
  _.closure754_externref_shim(n, t, e);
}
function L(n, t) {
  _._dyn_core__ops__function__FnMut_____Output___R_as_wasm_bindgen__closure__WasmClosure___describe__invoke__h4939de6411d337f8(
    n,
    t
  );
}
function P(n, t, e) {
  _.closure771_externref_shim(n, t, e);
}
function N(n, t, e, r) {
  _.closure1070_externref_shim(n, t, e, r);
}
const V = ['blob', 'arraybuffer'],
  $ = ['pending', 'done'],
  K = ['readonly', 'readwrite', 'versionchange', 'readwriteflush', 'cleanup'],
  T =
    typeof FinalizationRegistry > 'u'
      ? { register: () => {}, unregister: () => {} }
      : new FinalizationRegistry(n => _.__wbg_wasmbundle_free(n >>> 0, 1));
class y {
  static __wrap(t) {
    t = t >>> 0;
    const e = Object.create(y.prototype);
    return ((e.__wbg_ptr = t), T.register(e, e.__wbg_ptr, e), e);
  }
  __destroy_into_raw() {
    const t = this.__wbg_ptr;
    return ((this.__wbg_ptr = 0), T.unregister(this), t);
  }
  free() {
    const t = this.__destroy_into_raw();
    _.__wbg_wasmbundle_free(t, 0);
  }
  static fromBytes(t) {
    const e = _.wasmbundle_fromBytes(t);
    if (e[2]) throw S(e[1]);
    return y.__wrap(e[0]);
  }
  getPrefix(t) {
    const e = u(t, _.__wbindgen_malloc, _.__wbindgen_realloc),
      r = i;
    return _.wasmbundle_getPrefix(this.__wbg_ptr, e, r);
  }
  getRootId() {
    return _.wasmbundle_getRootId(this.__wbg_ptr);
  }
  getManifest() {
    return _.wasmbundle_getManifest(this.__wbg_ptr);
  }
  get(t) {
    const e = u(t, _.__wbindgen_malloc, _.__wbindgen_realloc),
      r = i;
    return _.wasmbundle_get(this.__wbg_ptr, e, r);
  }
  toBytes() {
    return _.wasmbundle_toBytes(this.__wbg_ptr);
  }
  listKeys() {
    return _.wasmbundle_listKeys(this.__wbg_ptr);
  }
}
const O =
  typeof FinalizationRegistry > 'u'
    ? { register: () => {}, unregister: () => {} }
    : new FinalizationRegistry(n =>
        _.__wbg_wasmdocumentwatcher_free(n >>> 0, 1)
      );
class W {
  static __wrap(t) {
    t = t >>> 0;
    const e = Object.create(W.prototype);
    return ((e.__wbg_ptr = t), O.register(e, e.__wbg_ptr, e), e);
  }
  __destroy_into_raw() {
    const t = this.__wbg_ptr;
    return ((this.__wbg_ptr = 0), O.unregister(this), t);
  }
  free() {
    const t = this.__destroy_into_raw();
    _.__wbg_wasmdocumentwatcher_free(t, 0);
  }
  documentId() {
    let t, e;
    try {
      const r = _.wasmdocumentwatcher_documentId(this.__wbg_ptr);
      return ((t = r[0]), (e = r[1]), f(r[0], r[1]));
    } finally {
      _.__wbindgen_free(t, e, 1);
    }
  }
  stop() {
    return _.wasmdocumentwatcher_stop(this.__wbg_ptr);
  }
}
const M =
  typeof FinalizationRegistry > 'u'
    ? { register: () => {}, unregister: () => {} }
    : new FinalizationRegistry(n => _.__wbg_wasmtonkcore_free(n >>> 0, 1));
class R {
  static __wrap(t) {
    t = t >>> 0;
    const e = Object.create(R.prototype);
    return ((e.__wbg_ptr = t), M.register(e, e.__wbg_ptr, e), e);
  }
  __destroy_into_raw() {
    const t = this.__wbg_ptr;
    return ((this.__wbg_ptr = 0), M.unregister(this), t);
  }
  free() {
    const t = this.__destroy_into_raw();
    _.__wbg_wasmtonkcore_free(t, 0);
  }
  static fromBytes(t) {
    return _.wasmtonkcore_fromBytes(t);
  }
  createFile(t, e) {
    const r = u(t, _.__wbindgen_malloc, _.__wbindgen_realloc),
      o = i;
    return _.wasmtonkcore_createFile(this.__wbg_ptr, r, o, e);
  }
  deleteFile(t) {
    const e = u(t, _.__wbindgen_malloc, _.__wbindgen_realloc),
      r = i;
    return _.wasmtonkcore_deleteFile(this.__wbg_ptr, e, r);
  }
  static fromBundle(t) {
    return (D(t, y), _.wasmtonkcore_fromBundle(t.__wbg_ptr));
  }
  getPeerId() {
    return _.wasmtonkcore_getPeerId(this.__wbg_ptr);
  }
  updateFile(t, e) {
    const r = u(t, _.__wbindgen_malloc, _.__wbindgen_realloc),
      o = i;
    return _.wasmtonkcore_updateFile(this.__wbg_ptr, r, o, e);
  }
  getMetadata(t) {
    const e = u(t, _.__wbindgen_malloc, _.__wbindgen_realloc),
      r = i;
    return _.wasmtonkcore_getMetadata(this.__wbg_ptr, e, r);
  }
  static withPeerId(t) {
    const e = u(t, _.__wbindgen_malloc, _.__wbindgen_realloc),
      r = i;
    return _.wasmtonkcore_withPeerId(e, r);
  }
  listDirectory(t) {
    const e = u(t, _.__wbindgen_malloc, _.__wbindgen_realloc),
      r = i;
    return _.wasmtonkcore_listDirectory(this.__wbg_ptr, e, r);
  }
  watchDocument(t, e) {
    const r = u(t, _.__wbindgen_malloc, _.__wbindgen_realloc),
      o = i;
    return _.wasmtonkcore_watchDocument(this.__wbg_ptr, r, o, e);
  }
  watchDirectory(t, e) {
    const r = u(t, _.__wbindgen_malloc, _.__wbindgen_realloc),
      o = i;
    return _.wasmtonkcore_watchDirectory(this.__wbg_ptr, r, o, e);
  }
  createDirectory(t) {
    const e = u(t, _.__wbindgen_malloc, _.__wbindgen_realloc),
      r = i;
    return _.wasmtonkcore_createDirectory(this.__wbg_ptr, e, r);
  }
  connectWebsocket(t) {
    const e = u(t, _.__wbindgen_malloc, _.__wbindgen_realloc),
      r = i;
    return _.wasmtonkcore_connectWebsocket(this.__wbg_ptr, e, r);
  }
  createFileWithBytes(t, e, r) {
    const o = u(t, _.__wbindgen_malloc, _.__wbindgen_realloc),
      c = i,
      b = A(r, _.__wbindgen_malloc),
      g = i;
    return _.wasmtonkcore_createFileWithBytes(this.__wbg_ptr, o, c, e, b, g);
  }
  updateFileWithBytes(t, e, r) {
    const o = u(t, _.__wbindgen_malloc, _.__wbindgen_realloc),
      c = i,
      b = A(r, _.__wbindgen_malloc),
      g = i;
    return _.wasmtonkcore_updateFileWithBytes(this.__wbg_ptr, o, c, e, b, g);
  }
  constructor() {
    return _.wasmtonkcore_new();
  }
  exists(t) {
    const e = u(t, _.__wbindgen_malloc, _.__wbindgen_realloc),
      r = i;
    return _.wasmtonkcore_exists(this.__wbg_ptr, e, r);
  }
  toBytes() {
    return _.wasmtonkcore_toBytes(this.__wbg_ptr);
  }
  readFile(t) {
    const e = u(t, _.__wbindgen_malloc, _.__wbindgen_realloc),
      r = i;
    return _.wasmtonkcore_readFile(this.__wbg_ptr, e, r);
  }
}
async function G(n, t) {
  if (typeof Response == 'function' && n instanceof Response) {
    if (typeof WebAssembly.instantiateStreaming == 'function')
      try {
        return await WebAssembly.instantiateStreaming(n, t);
      } catch (r) {
        if (n.headers.get('Content-Type') != 'application/wasm')
          console.warn(
            '`WebAssembly.instantiateStreaming` failed because your server does not serve Wasm with `application/wasm` MIME type. Falling back to `WebAssembly.instantiate` which is slower. Original error:\n',
            r
          );
        else throw r;
      }
    const e = await n.arrayBuffer();
    return await WebAssembly.instantiate(e, t);
  } else {
    const e = await WebAssembly.instantiate(n, t);
    return e instanceof WebAssembly.Instance ? { instance: e, module: n } : e;
  }
}
function E() {
  const n = {};
  return (
    (n.wbg = {}),
    (n.wbg.__wbg_String_8f0eb39a4a4c2f66 = function (t, e) {
      const r = String(e),
        o = u(r, _.__wbindgen_malloc, _.__wbindgen_realloc),
        c = i;
      (a().setInt32(t + 4 * 1, c, !0), a().setInt32(t + 4 * 0, o, !0));
    }),
    (n.wbg.__wbg_Window_41559019033ede94 = function (t) {
      return t.Window;
    }),
    (n.wbg.__wbg_WorkerGlobalScope_d324bffbeaef9f3a = function (t) {
      return t.WorkerGlobalScope;
    }),
    (n.wbg.__wbg_abort_99fc644e2c79c9fb = function () {
      return s(function (t) {
        t.abort();
      }, arguments);
    }),
    (n.wbg.__wbg_bound_f2afc3766d4545cf = function () {
      return s(function (t, e, r, o) {
        return IDBKeyRange.bound(t, e, r !== 0, o !== 0);
      }, arguments);
    }),
    (n.wbg.__wbg_buffer_09165b52af8c5237 = function (t) {
      return t.buffer;
    }),
    (n.wbg.__wbg_buffer_609cc3eee51ed158 = function (t) {
      return t.buffer;
    }),
    (n.wbg.__wbg_call_672a4d21634d4a24 = function () {
      return s(function (t, e) {
        return t.call(e);
      }, arguments);
    }),
    (n.wbg.__wbg_call_7cccdd69e0791ae2 = function () {
      return s(function (t, e, r) {
        return t.call(e, r);
      }, arguments);
    }),
    (n.wbg.__wbg_close_2893b7d056a0627d = function () {
      return s(function (t) {
        t.close();
      }, arguments);
    }),
    (n.wbg.__wbg_commit_a54edce65f3858f2 = function () {
      return s(function (t) {
        t.commit();
      }, arguments);
    }),
    (n.wbg.__wbg_continue_c46c11d3dbe1b030 = function () {
      return s(function (t) {
        t.continue();
      }, arguments);
    }),
    (n.wbg.__wbg_createObjectStore_e566459f7161f82f = function () {
      return s(function (t, e, r) {
        return t.createObjectStore(f(e, r));
      }, arguments);
    }),
    (n.wbg.__wbg_data_432d9c3df2630942 = function (t) {
      return t.data;
    }),
    (n.wbg.__wbg_delete_200677093b4cf756 = function () {
      return s(function (t, e) {
        return t.delete(e);
      }, arguments);
    }),
    (n.wbg.__wbg_done_769e5ede4b31c67b = function (t) {
      return t.done;
    }),
    (n.wbg.__wbg_entries_3265d4158b33e5dc = function (t) {
      return Object.entries(t);
    }),
    (n.wbg.__wbg_error_31d7961b8e0d3f6c = function (t, e) {
      console.error(f(t, e));
    }),
    (n.wbg.__wbg_error_7534b8e9a36f1ab4 = function (t, e) {
      let r, o;
      try {
        ((r = t), (o = e), console.error(f(t, e)));
      } finally {
        _.__wbindgen_free(r, o, 1);
      }
    }),
    (n.wbg.__wbg_error_ff4ddaabdfc5dbb3 = function () {
      return s(function (t) {
        const e = t.error;
        return w(e) ? 0 : l(e);
      }, arguments);
    }),
    (n.wbg.__wbg_getRandomValues_38a1ff1ea09f6cc7 = function () {
      return s(function (t, e) {
        globalThis.crypto.getRandomValues(F(t, e));
      }, arguments);
    }),
    (n.wbg.__wbg_getRandomValues_3c9c0d586e575a16 = function () {
      return s(function (t, e) {
        globalThis.crypto.getRandomValues(F(t, e));
      }, arguments);
    }),
    (n.wbg.__wbg_getTime_46267b1c24877e30 = function (t) {
      return t.getTime();
    }),
    (n.wbg.__wbg_get_67b2ba62fc30de12 = function () {
      return s(function (t, e) {
        return Reflect.get(t, e);
      }, arguments);
    }),
    (n.wbg.__wbg_get_8da03f81f6a1111e = function () {
      return s(function (t, e) {
        return t.get(e);
      }, arguments);
    }),
    (n.wbg.__wbg_get_b9b93047fe3cf45b = function (t, e) {
      return t[e >>> 0];
    }),
    (n.wbg.__wbg_global_f5c2926e57ba457f = function (t) {
      return t.global;
    }),
    (n.wbg.__wbg_indexedDB_54f01430b1e194e8 = function () {
      return s(function (t) {
        const e = t.indexedDB;
        return w(e) ? 0 : l(e);
      }, arguments);
    }),
    (n.wbg.__wbg_indexedDB_b1f49280282046f8 = function () {
      return s(function (t) {
        const e = t.indexedDB;
        return w(e) ? 0 : l(e);
      }, arguments);
    }),
    (n.wbg.__wbg_indexedDB_f6b47b0dc333fd2f = function () {
      return s(function (t) {
        const e = t.indexedDB;
        return w(e) ? 0 : l(e);
      }, arguments);
    }),
    (n.wbg.__wbg_instanceof_ArrayBuffer_e14585432e3737fc = function (t) {
      let e;
      try {
        e = t instanceof ArrayBuffer;
      } catch {
        e = !1;
      }
      return e;
    }),
    (n.wbg.__wbg_instanceof_CursorSys_4b6a8aba0e823e75 = function (t) {
      let e;
      try {
        e = t instanceof IDBCursorWithValue;
      } catch {
        e = !1;
      }
      return e;
    }),
    (n.wbg.__wbg_instanceof_DomException_ed1ccb7aaf39034c = function (t) {
      let e;
      try {
        e = t instanceof DOMException;
      } catch {
        e = !1;
      }
      return e;
    }),
    (n.wbg.__wbg_instanceof_Error_4d54113b22d20306 = function (t) {
      let e;
      try {
        e = t instanceof Error;
      } catch {
        e = !1;
      }
      return e;
    }),
    (n.wbg.__wbg_instanceof_IdbCursor_4f02b0cddf69c141 = function (t) {
      let e;
      try {
        e = t instanceof IDBCursor;
      } catch {
        e = !1;
      }
      return e;
    }),
    (n.wbg.__wbg_instanceof_IdbDatabase_a3ef009ca00059f9 = function (t) {
      let e;
      try {
        e = t instanceof IDBDatabase;
      } catch {
        e = !1;
      }
      return e;
    }),
    (n.wbg.__wbg_instanceof_IdbRequest_4813c3f207666aa4 = function (t) {
      let e;
      try {
        e = t instanceof IDBRequest;
      } catch {
        e = !1;
      }
      return e;
    }),
    (n.wbg.__wbg_instanceof_Map_f3469ce2244d2430 = function (t) {
      let e;
      try {
        e = t instanceof Map;
      } catch {
        e = !1;
      }
      return e;
    }),
    (n.wbg.__wbg_instanceof_Uint8Array_17156bcf118086a9 = function (t) {
      let e;
      try {
        e = t instanceof Uint8Array;
      } catch {
        e = !1;
      }
      return e;
    }),
    (n.wbg.__wbg_isArray_a1eab7e0d067391b = function (t) {
      return Array.isArray(t);
    }),
    (n.wbg.__wbg_isSafeInteger_343e2beeeece1bb0 = function (t) {
      return Number.isSafeInteger(t);
    }),
    (n.wbg.__wbg_item_c3c26b4103ad5aaf = function (t, e, r) {
      const o = e.item(r >>> 0);
      var c = w(o) ? 0 : u(o, _.__wbindgen_malloc, _.__wbindgen_realloc),
        b = i;
      (a().setInt32(t + 4 * 1, b, !0), a().setInt32(t + 4 * 0, c, !0));
    }),
    (n.wbg.__wbg_iterator_9a24c88df860dc65 = function () {
      return Symbol.iterator;
    }),
    (n.wbg.__wbg_key_29fefecef430db96 = function () {
      return s(function (t) {
        return t.key;
      }, arguments);
    }),
    (n.wbg.__wbg_length_a446193dc22c12f8 = function (t) {
      return t.length;
    }),
    (n.wbg.__wbg_length_e2d2a49132c1b256 = function (t) {
      return t.length;
    }),
    (n.wbg.__wbg_log_2f4a53bbb94ad21b = function (t, e) {
      console.log(f(t, e));
    }),
    (n.wbg.__wbg_lowerBound_1872d19f5bcf83c6 = function () {
      return s(function (t, e) {
        return IDBKeyRange.lowerBound(t, e !== 0);
      }, arguments);
    }),
    (n.wbg.__wbg_message_5c5d919204d42400 = function (t, e) {
      const r = e.message,
        o = u(r, _.__wbindgen_malloc, _.__wbindgen_realloc),
        c = i;
      (a().setInt32(t + 4 * 1, c, !0), a().setInt32(t + 4 * 0, o, !0));
    }),
    (n.wbg.__wbg_message_97a2af9b89d693a3 = function (t) {
      return t.message;
    }),
    (n.wbg.__wbg_message_d1685a448ba00178 = function (t, e) {
      const r = e.message,
        o = u(r, _.__wbindgen_malloc, _.__wbindgen_realloc),
        c = i;
      (a().setInt32(t + 4 * 1, c, !0), a().setInt32(t + 4 * 0, o, !0));
    }),
    (n.wbg.__wbg_name_f2d27098bfd843e7 = function (t, e) {
      const r = e.name,
        o = u(r, _.__wbindgen_malloc, _.__wbindgen_realloc),
        c = i;
      (a().setInt32(t + 4 * 1, c, !0), a().setInt32(t + 4 * 0, o, !0));
    }),
    (n.wbg.__wbg_new0_f788a2397c7ca929 = function () {
      return new Date();
    }),
    (n.wbg.__wbg_new_23a2665fac83c611 = function (t, e) {
      try {
        var r = { a: t, b: e },
          o = (b, g) => {
            const d = r.a;
            r.a = 0;
            try {
              return N(d, r.b, b, g);
            } finally {
              r.a = d;
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
      return s(function (t, e) {
        return new WebSocket(f(t, e));
      }, arguments);
    }),
    (n.wbg.__wbg_new_a12002a7f91c75be = function (t) {
      return new Uint8Array(t);
    }),
    (n.wbg.__wbg_new_c68d7209be747379 = function (t, e) {
      return new Error(f(t, e));
    }),
    (n.wbg.__wbg_newnoargs_105ed471475aaf50 = function (t, e) {
      return new Function(f(t, e));
    }),
    (n.wbg.__wbg_newwithbyteoffsetandlength_d97e637ebe145a9a = function (
      t,
      e,
      r
    ) {
      return new Uint8Array(t, e >>> 0, r >>> 0);
    }),
    (n.wbg.__wbg_newwithlength_a381634e90c276d4 = function (t) {
      return new Uint8Array(t >>> 0);
    }),
    (n.wbg.__wbg_next_25feadfc0913fea9 = function (t) {
      return t.next;
    }),
    (n.wbg.__wbg_next_6574e1a8a62d1055 = function () {
      return s(function (t) {
        return t.next();
      }, arguments);
    }),
    (n.wbg.__wbg_now_807e54c39636c349 = function () {
      return Date.now();
    }),
    (n.wbg.__wbg_objectStoreNames_9bb1ab04a7012aaf = function (t) {
      return t.objectStoreNames;
    }),
    (n.wbg.__wbg_objectStore_21878d46d25b64b6 = function () {
      return s(function (t, e, r) {
        return t.objectStore(f(e, r));
      }, arguments);
    }),
    (n.wbg.__wbg_openCursor_238e247d18bde2cd = function () {
      return s(function (t) {
        return t.openCursor();
      }, arguments);
    }),
    (n.wbg.__wbg_open_e0c0b2993eb596e1 = function () {
      return s(function (t, e, r, o) {
        return t.open(f(e, r), o >>> 0);
      }, arguments);
    }),
    (n.wbg.__wbg_push_737cfc8c1432c2c6 = function (t, e) {
      return t.push(e);
    }),
    (n.wbg.__wbg_put_066faa31a6a88f5b = function () {
      return s(function (t, e, r) {
        return t.put(e, r);
      }, arguments);
    }),
    (n.wbg.__wbg_queueMicrotask_97d92b4fcc8a61c5 = function (t) {
      queueMicrotask(t);
    }),
    (n.wbg.__wbg_queueMicrotask_d3219def82552485 = function (t) {
      return t.queueMicrotask;
    }),
    (n.wbg.__wbg_readyState_4013cfdf4f22afb0 = function (t) {
      const e = t.readyState;
      return ($.indexOf(e) + 1 || 3) - 1;
    }),
    (n.wbg.__wbg_request_5079471e06223120 = function (t) {
      return t.request;
    }),
    (n.wbg.__wbg_request_fada8c23b78b3a02 = function (t) {
      return t.request;
    }),
    (n.wbg.__wbg_resolve_4851785c9c5f573d = function (t) {
      return Promise.resolve(t);
    }),
    (n.wbg.__wbg_result_f29afabdf2c05826 = function () {
      return s(function (t) {
        return t.result;
      }, arguments);
    }),
    (n.wbg.__wbg_send_7c4769e24cf1d784 = function () {
      return s(function (t, e) {
        t.send(e);
      }, arguments);
    }),
    (n.wbg.__wbg_set_37837023f3d740e8 = function (t, e, r) {
      t[e >>> 0] = r;
    }),
    (n.wbg.__wbg_set_3f1d0b984ed272ed = function (t, e, r) {
      t[e] = r;
    }),
    (n.wbg.__wbg_set_65595bdd868b3009 = function (t, e, r) {
      t.set(e, r >>> 0);
    }),
    (n.wbg.__wbg_set_8fc6bf8a5b1071d1 = function (t, e, r) {
      return t.set(e, r);
    }),
    (n.wbg.__wbg_set_bb8cecf6a62b9f46 = function () {
      return s(function (t, e, r) {
        return Reflect.set(t, e, r);
      }, arguments);
    }),
    (n.wbg.__wbg_setbinaryType_92fa1ffd873b327c = function (t, e) {
      t.binaryType = V[e];
    }),
    (n.wbg.__wbg_setonabort_3bf4db6614fa98e9 = function (t, e) {
      t.onabort = e;
    }),
    (n.wbg.__wbg_setonclose_14fc475a49d488fc = function (t, e) {
      t.onclose = e;
    }),
    (n.wbg.__wbg_setoncomplete_4d19df0dadb7c4d4 = function (t, e) {
      t.oncomplete = e;
    }),
    (n.wbg.__wbg_setonerror_8639efe354b947cd = function (t, e) {
      t.onerror = e;
    }),
    (n.wbg.__wbg_setonerror_b0d9d723b8fddbbb = function (t, e) {
      t.onerror = e;
    }),
    (n.wbg.__wbg_setonerror_d7e3056cc6e56085 = function (t, e) {
      t.onerror = e;
    }),
    (n.wbg.__wbg_setonmessage_6eccab530a8fb4c7 = function (t, e) {
      t.onmessage = e;
    }),
    (n.wbg.__wbg_setonopen_2da654e1f39745d5 = function (t, e) {
      t.onopen = e;
    }),
    (n.wbg.__wbg_setonsuccess_afa464ee777a396d = function (t, e) {
      t.onsuccess = e;
    }),
    (n.wbg.__wbg_setonupgradeneeded_fcf7ce4f2eb0cb5f = function (t, e) {
      t.onupgradeneeded = e;
    }),
    (n.wbg.__wbg_stack_0ed75d68575b0f3c = function (t, e) {
      const r = e.stack,
        o = u(r, _.__wbindgen_malloc, _.__wbindgen_realloc),
        c = i;
      (a().setInt32(t + 4 * 1, c, !0), a().setInt32(t + 4 * 0, o, !0));
    }),
    (n.wbg.__wbg_static_accessor_GLOBAL_88a902d13a557d07 = function () {
      const t = typeof globalThis > 'u' ? null : globalThis;
      return w(t) ? 0 : l(t);
    }),
    (n.wbg.__wbg_static_accessor_GLOBAL_THIS_56578be7e9f832b0 = function () {
      const t = typeof globalThis > 'u' ? null : globalThis;
      return w(t) ? 0 : l(t);
    }),
    (n.wbg.__wbg_static_accessor_SELF_37c5d418e4bf5819 = function () {
      const t = typeof self > 'u' ? null : self;
      return w(t) ? 0 : l(t);
    }),
    (n.wbg.__wbg_static_accessor_WINDOW_5de37043a91a9c40 = function () {
      const t = typeof window > 'u' ? null : window;
      return w(t) ? 0 : l(t);
    }),
    (n.wbg.__wbg_target_0a62d9d79a2a1ede = function (t) {
      const e = t.target;
      return w(e) ? 0 : l(e);
    }),
    (n.wbg.__wbg_then_44b73946d2fb3e7d = function (t, e) {
      return t.then(e);
    }),
    (n.wbg.__wbg_then_48b406749878a531 = function (t, e, r) {
      return t.then(e, r);
    }),
    (n.wbg.__wbg_toString_5285597960676b7b = function (t) {
      return t.toString();
    }),
    (n.wbg.__wbg_transaction_7d452475ef3c3c33 = function (t) {
      return t.transaction;
    }),
    (n.wbg.__wbg_transaction_babc423936946a37 = function () {
      return s(function (t, e, r, o) {
        return t.transaction(f(e, r), K[o]);
      }, arguments);
    }),
    (n.wbg.__wbg_upperBound_76a00814e81f6891 = function () {
      return s(function (t, e) {
        return IDBKeyRange.upperBound(t, e !== 0);
      }, arguments);
    }),
    (n.wbg.__wbg_value_68c4e9a54bb7fd5e = function () {
      return s(function (t) {
        return t.value;
      }, arguments);
    }),
    (n.wbg.__wbg_value_cd1ffa7b1ab794f1 = function (t) {
      return t.value;
    }),
    (n.wbg.__wbg_wasmdocumentwatcher_new = function (t) {
      return W.__wrap(t);
    }),
    (n.wbg.__wbg_wasmtonkcore_new = function (t) {
      return R.__wrap(t);
    }),
    (n.wbg.__wbindgen_bigint_from_i64 = function (t) {
      return t;
    }),
    (n.wbg.__wbindgen_bigint_from_u64 = function (t) {
      return BigInt.asUintN(64, t);
    }),
    (n.wbg.__wbindgen_bigint_get_as_i64 = function (t, e) {
      const r = e,
        o = typeof r == 'bigint' ? r : void 0;
      (a().setBigInt64(t + 8 * 1, w(o) ? BigInt(0) : o, !0),
        a().setInt32(t + 4 * 0, !w(o), !0));
    }),
    (n.wbg.__wbindgen_boolean_get = function (t) {
      const e = t;
      return typeof e == 'boolean' ? (e ? 1 : 0) : 2;
    }),
    (n.wbg.__wbindgen_cb_drop = function (t) {
      const e = t.original;
      return e.cnt-- == 1 ? ((e.a = 0), !0) : !1;
    }),
    (n.wbg.__wbindgen_closure_wrapper1823 = function (t, e, r) {
      return m(t, e, 755, z);
    }),
    (n.wbg.__wbindgen_closure_wrapper1825 = function (t, e, r) {
      return m(t, e, 755, L);
    }),
    (n.wbg.__wbindgen_closure_wrapper1885 = function (t, e, r) {
      return m(t, e, 772, P);
    }),
    (n.wbg.__wbindgen_closure_wrapper636 = function (t, e, r) {
      return m(t, e, 247, x);
    }),
    (n.wbg.__wbindgen_closure_wrapper638 = function (t, e, r) {
      return m(t, e, 247, U);
    }),
    (n.wbg.__wbindgen_closure_wrapper640 = function (t, e, r) {
      return m(t, e, 247, x);
    }),
    (n.wbg.__wbindgen_closure_wrapper642 = function (t, e, r) {
      return m(t, e, 247, x);
    }),
    (n.wbg.__wbindgen_debug_string = function (t, e) {
      const r = B(e),
        o = u(r, _.__wbindgen_malloc, _.__wbindgen_realloc),
        c = i;
      (a().setInt32(t + 4 * 1, c, !0), a().setInt32(t + 4 * 0, o, !0));
    }),
    (n.wbg.__wbindgen_error_new = function (t, e) {
      return new Error(f(t, e));
    }),
    (n.wbg.__wbindgen_in = function (t, e) {
      return t in e;
    }),
    (n.wbg.__wbindgen_init_externref_table = function () {
      const t = _.__wbindgen_export_4,
        e = t.grow(4);
      (t.set(0, void 0),
        t.set(e + 0, void 0),
        t.set(e + 1, null),
        t.set(e + 2, !0),
        t.set(e + 3, !1));
    }),
    (n.wbg.__wbindgen_is_bigint = function (t) {
      return typeof t == 'bigint';
    }),
    (n.wbg.__wbindgen_is_function = function (t) {
      return typeof t == 'function';
    }),
    (n.wbg.__wbindgen_is_null = function (t) {
      return t === null;
    }),
    (n.wbg.__wbindgen_is_object = function (t) {
      const e = t;
      return typeof e == 'object' && e !== null;
    }),
    (n.wbg.__wbindgen_is_string = function (t) {
      return typeof t == 'string';
    }),
    (n.wbg.__wbindgen_is_undefined = function (t) {
      return t === void 0;
    }),
    (n.wbg.__wbindgen_jsval_eq = function (t, e) {
      return t === e;
    }),
    (n.wbg.__wbindgen_jsval_loose_eq = function (t, e) {
      return t == e;
    }),
    (n.wbg.__wbindgen_memory = function () {
      return _.memory;
    }),
    (n.wbg.__wbindgen_number_get = function (t, e) {
      const r = e,
        o = typeof r == 'number' ? r : void 0;
      (a().setFloat64(t + 8 * 1, w(o) ? 0 : o, !0),
        a().setInt32(t + 4 * 0, !w(o), !0));
    }),
    (n.wbg.__wbindgen_number_new = function (t) {
      return t;
    }),
    (n.wbg.__wbindgen_string_get = function (t, e) {
      const r = e,
        o = typeof r == 'string' ? r : void 0;
      var c = w(o) ? 0 : u(o, _.__wbindgen_malloc, _.__wbindgen_realloc),
        b = i;
      (a().setInt32(t + 4 * 1, b, !0), a().setInt32(t + 4 * 0, c, !0));
    }),
    (n.wbg.__wbindgen_string_new = function (t, e) {
      return f(t, e);
    }),
    (n.wbg.__wbindgen_throw = function (t, e) {
      throw new Error(f(t, e));
    }),
    n
  );
}
function q(n, t) {
  return (
    (_ = n.exports),
    (H.__wbindgen_wasm_module = t),
    (p = null),
    (k = null),
    _.__wbindgen_start(),
    _
  );
}
function ct(n) {
  if (_ !== void 0) return _;
  typeof n < 'u' &&
    (Object.getPrototypeOf(n) === Object.prototype
      ? ({ module: n } = n)
      : console.warn(
          'using deprecated parameters for `initSync()`; pass a single object instead'
        ));
  const t = E();
  n instanceof WebAssembly.Module || (n = new WebAssembly.Module(n));
  const e = new WebAssembly.Instance(n, t);
  return q(e, n);
}
async function H(n) {
  if (_ !== void 0) return _;
  (typeof n < 'u' &&
    (Object.getPrototypeOf(n) === Object.prototype
      ? ({ module_or_path: n } = n)
      : console.warn(
          'using deprecated parameters for the initialization function; pass a single object instead'
        )),
    typeof n > 'u' &&
      (n = new URL('/assets/tonk_core_bg-BjovLCWw.wasm', import.meta.url)));
  const t = E();
  (typeof n == 'string' ||
    (typeof Request == 'function' && n instanceof Request) ||
    (typeof URL == 'function' && n instanceof URL)) &&
    (n = fetch(n));
  const { instance: e, module: r } = await G(await n, t);
  return q(e, r);
}
export {
  y as WasmBundle,
  W as WasmDocumentWatcher,
  R as WasmTonkCore,
  X as create_bundle_from_bytes,
  Y as create_tonk,
  J as create_tonk_from_bundle,
  Z as create_tonk_from_bundle_with_storage,
  Q as create_tonk_from_bytes,
  tt as create_tonk_from_bytes_with_storage,
  et as create_tonk_with_config,
  _t as create_tonk_with_peer_id,
  nt as create_tonk_with_storage,
  H as default,
  rt as init,
  ct as initSync,
  ot as set_time_provider,
};
