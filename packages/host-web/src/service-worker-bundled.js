const Bt = 'modulepreload',
  Rt = function (n) {
    return '/' + n;
  },
  it = {},
  K = function (t, e, r) {
    let o = Promise.resolve();
    if (e && e.length > 0) {
      document.getElementsByTagName('link');
      const a = document.querySelector('meta[property=csp-nonce]'),
        _ = a?.nonce || a?.getAttribute('nonce');
      o = Promise.allSettled(
        e.map(f => {
          if (((f = Rt(f)), f in it)) return;
          it[f] = !0;
          const S = f.endsWith('.css'),
            E = S ? '[rel="stylesheet"]' : '';
          if (document.querySelector(`link[href="${f}"]${E}`)) return;
          const b = document.createElement('link');
          if (
            ((b.rel = S ? 'stylesheet' : Bt),
            S || (b.as = 'script'),
            (b.crossOrigin = ''),
            (b.href = f),
            _ && b.setAttribute('nonce', _),
            document.head.appendChild(b),
            S)
          )
            return new Promise((R, q) => {
              b.addEventListener('load', R),
                b.addEventListener('error', () =>
                  q(new Error(`Unable to preload CSS for ${f}`))
                );
            });
        })
      );
    }
    function s(a) {
      const _ = new Event('vite:preloadError', { cancelable: !0 });
      if (((_.payload = a), window.dispatchEvent(_), !_.defaultPrevented))
        throw a;
    }
    return o.then(a => {
      for (const _ of a || []) _.status === 'rejected' && s(_.reason);
      return t().catch(s);
    });
  };
var kt = function (n, t, e, r, o) {
    if (r === 'm') throw new TypeError('Private method is not writable');
    if (r === 'a' && !o)
      throw new TypeError('Private accessor was defined without a setter');
    if (typeof t == 'function' ? n !== t || !o : !t.has(n))
      throw new TypeError(
        'Cannot write private member to an object whose class did not declare it'
      );
    return r === 'a' ? o.call(n, e) : o ? (o.value = e) : t.set(n, e), e;
  },
  h = function (n, t, e, r) {
    if (e === 'a' && !r)
      throw new TypeError('Private accessor was defined without a getter');
    if (typeof t == 'function' ? n !== t || !r : !t.has(n))
      throw new TypeError(
        'Cannot read private member from an object whose class did not declare it'
      );
    return e === 'm' ? r : e === 'a' ? r.call(n) : r ? r.value : t.get(n);
  },
  D,
  m;
class Q extends Error {
  constructor(t, e) {
    super(t), (this.code = e), (this.name = 'TonkError');
  }
}
class Dt extends Q {
  constructor(t) {
    super(t, 'CONNECTION_ERROR'), (this.name = 'ConnectionError');
  }
}
class p extends Q {
  constructor(t) {
    super(t, 'FILESYSTEM_ERROR'), (this.name = 'FileSystemError');
  }
}
class x extends Q {
  constructor(t) {
    super(t, 'BUNDLE_ERROR'), (this.name = 'BundleError');
  }
}
class Y {
  constructor(t) {
    D.set(this, void 0), kt(this, D, t, 'f');
  }
  static async fromBytes(t, e) {
    try {
      const { create_bundle_from_bytes: r } =
        e || (await K(() => Promise.resolve().then(() => J), void 0));
      return new Y(r(t));
    } catch (r) {
      throw new x(`Failed to create bundle from bytes: ${r}`);
    }
  }
  async getRootId() {
    try {
      return await h(this, D, 'f').getRootId();
    } catch (t) {
      throw new x(`Failed to get root ID: ${t}`);
    }
  }
  async get(t) {
    try {
      const e = await h(this, D, 'f').get(t);
      return e === null ? null : e;
    } catch (e) {
      throw new x(`Failed to get key ${t}: ${e}`);
    }
  }
  async getPrefix(t) {
    try {
      return (await h(this, D, 'f').getPrefix(t)).map(r => ({
        key: r.key,
        value: r.value,
      }));
    } catch (e) {
      throw new x(`Failed to get prefix ${t}: ${e}`);
    }
  }
  async listKeys() {
    try {
      return await h(this, D, 'f').listKeys();
    } catch (t) {
      throw new x(`Failed to list keys: ${t}`);
    }
  }
  async getManifest() {
    try {
      return await h(this, D, 'f').getManifest();
    } catch (t) {
      throw new x(`Failed to retrieve manifest: ${t}`);
    }
  }
  async setManifest(t) {
    try {
      await h(this, D, 'f').setManifest(t);
    } catch (e) {
      throw new x(`Failed to set manifest: ${e}`);
    }
  }
  async toBytes() {
    try {
      return await h(this, D, 'f').toBytes();
    } catch (t) {
      throw new x(`Failed to serialize bundle: ${t}`);
    }
  }
  free() {
    h(this, D, 'f').free();
  }
}
D = new WeakMap();
class B {
  constructor(t) {
    m.set(this, void 0), kt(this, m, t, 'f');
  }
  static async create(t, e) {
    const r = e || (await K(() => Promise.resolve().then(() => J), void 0));
    if (t?.peerId && t?.storage) {
      const { create_tonk_with_config: o } = r,
        s = await o(t.peerId, t.storage.type === 'indexeddb');
      return new B(s);
    } else if (t?.peerId) {
      const { create_tonk_with_peer_id: o } = r,
        s = await o(t.peerId);
      return new B(s);
    } else if (t?.storage) {
      const { create_tonk_with_storage: o } = r,
        s = await o(t.storage.type === 'indexeddb');
      return new B(s);
    } else {
      const { create_tonk: o } = r,
        s = await o();
      return new B(s);
    }
  }
  static async createWithPeerId(t, e) {
    const { create_tonk_with_peer_id: r } =
        e || (await K(() => Promise.resolve().then(() => J), void 0)),
      o = await r(t);
    return new B(o);
  }
  static async fromBundle(t, e, r) {
    const o = r || (await K(() => Promise.resolve().then(() => J), void 0));
    if (e?.storage) {
      const { create_tonk_from_bundle_with_storage: s } = o,
        a = await s(t, e.storage.type === 'indexeddb');
      return new B(a);
    } else {
      const { create_tonk_from_bundle: s } = o,
        a = await s(t);
      return new B(a);
    }
  }
  static async fromBytes(t, e, r) {
    const o = r || (await K(() => Promise.resolve().then(() => J), void 0));
    if (e?.storage) {
      const { create_tonk_from_bytes_with_storage: s } = o,
        a = await s(t, e.storage.type === 'indexeddb');
      return new B(a);
    } else {
      const { create_tonk_from_bytes: s } = o,
        a = await s(t);
      return new B(a);
    }
  }
  getPeerId() {
    return h(this, m, 'f').getPeerId();
  }
  async connectWebsocket(t) {
    try {
      await h(this, m, 'f').connectWebsocket(t);
    } catch (e) {
      throw new Dt(`Failed to connect to ${t}: ${e}`);
    }
  }
  async forkToBytes(t) {
    try {
      return await h(this, m, 'f').forkToBytes(t);
    } catch (e) {
      throw new Q(`Failed to serialize to bundle data: ${e}`);
    }
  }
  async toBytes(t) {
    try {
      return await h(this, m, 'f').toBytes(t);
    } catch (e) {
      throw new Q(`Failed to serialize to bundle data: ${e}`);
    }
  }
  async createFile(t, e) {
    try {
      await h(this, m, 'f').createFile(t, e);
    } catch (r) {
      throw new p(`Failed to create file at ${t}: ${r}`);
    }
  }
  async createFileWithBytes(t, e, r) {
    try {
      let o = typeof r == 'string' ? ct(r) : r;
      await h(this, m, 'f').createFileWithBytes(t, e, o);
    } catch (o) {
      throw new p(`Failed to create file at ${t}: ${o}`);
    }
  }
  async readFile(t) {
    try {
      const e = await h(this, m, 'f').readFile(t);
      if (e === null) throw new p(`File not found: ${t}`);
      let r;
      return (
        e.bytes && (r = st(e.bytes)),
        { ...e, content: JSON.parse(e.content), bytes: r }
      );
    } catch (e) {
      throw e instanceof p ? e : new p(`Failed to read file at ${t}: ${e}`);
    }
  }
  async updateFile(t, e) {
    try {
      return await h(this, m, 'f').updateFile(t, e);
    } catch (r) {
      throw new p(`Failed to update file at ${t}: ${r}`);
    }
  }
  async updateFileWithBytes(t, e, r) {
    try {
      let o = typeof r == 'string' ? ct(r) : r;
      return await h(this, m, 'f').updateFileWithBytes(t, e, o);
    } catch (o) {
      throw new p(`Failed to update file at ${t}: ${o}`);
    }
  }
  async deleteFile(t) {
    try {
      return await h(this, m, 'f').deleteFile(t);
    } catch (e) {
      throw new p(`Failed to delete file at ${t}: ${e}`);
    }
  }
  async createDirectory(t) {
    try {
      await h(this, m, 'f').createDirectory(t);
    } catch (e) {
      throw new p(`Failed to create directory at ${t}: ${e}`);
    }
  }
  async listDirectory(t) {
    try {
      return (await h(this, m, 'f').listDirectory(t)).map(r => ({
        name: r.name,
        type: r.type,
        timestamps: r.timestamps,
        pointer: r.pointer,
      }));
    } catch (e) {
      throw new p(`Failed to list directory at ${t}: ${e}`);
    }
  }
  async exists(t) {
    try {
      return await h(this, m, 'f').exists(t);
    } catch (e) {
      throw new p(`Failed to check existence of ${t}: ${e}`);
    }
  }
  async rename(t, e) {
    try {
      return await h(this, m, 'f').rename(t, e);
    } catch (r) {
      throw new p(`Failed to rename ${t} to ${e}: ${r}`);
    }
  }
  async getMetadata(t) {
    try {
      const e = await h(this, m, 'f').getMetadata(t);
      if (e === null) throw new p(`File or directory not found: ${t}`);
      return e;
    } catch (e) {
      throw e instanceof p ? e : new p(`Failed to get metadata for ${t}: ${e}`);
    }
  }
  async watchFile(t, e) {
    try {
      const r = await h(this, m, 'f').watchDocument(t, o => {
        let s;
        o.bytes && (s = st(o.bytes)),
          e({ ...o, content: JSON.parse(o.content), bytes: s });
      });
      if (r === null) throw new p(`File not found: ${t}`);
      return r;
    } catch (r) {
      throw r instanceof p
        ? r
        : new p(`Failed to watch file at path ${t}: ${r}`);
    }
  }
  async watchDirectory(t, e) {
    try {
      const r = await h(this, m, 'f').watchDirectory(t, e);
      if (r === null) throw new p(`Directory not found: ${t}`);
      return r;
    } catch (r) {
      throw r instanceof p
        ? r
        : new p(`Failed to watch directory at path ${t}: ${r}`);
    }
  }
  free() {
    h(this, m, 'f').free();
  }
}
m = new WeakMap();
const ct = n => {
    const t = atob(n),
      e = new Uint8Array(t.length);
    for (let r = 0; r < t.length; r++) e[r] = t.charCodeAt(r);
    return e;
  },
  st = n => {
    if (typeof n == 'string') return n;
    if (Array.isArray(n)) {
      let e = '';
      for (let r = 0; r < n.length; r += 8192) {
        const o = n.slice(r, r + 8192);
        e += String.fromCharCode(...o);
      }
      return btoa(e);
    } else throw new p(`Unrecognized bytes type in readFile ${typeof n}`);
  };
let i,
  H = null;
function O() {
  return (
    (H === null || H.byteLength === 0) && (H = new Uint8Array(i.memory.buffer)),
    H
  );
}
let Z = new TextDecoder('utf-8', { ignoreBOM: !0, fatal: !0 });
Z.decode();
const Tt = 2146435072;
let et = 0;
function Pt(n, t) {
  return (
    (et += t),
    et >= Tt &&
      ((Z = new TextDecoder('utf-8', { ignoreBOM: !0, fatal: !0 })),
      Z.decode(),
      (et = t)),
    Z.decode(O().subarray(n, n + t))
  );
}
function g(n, t) {
  return (n = n >>> 0), Pt(n, t);
}
let u = 0;
const X = new TextEncoder();
'encodeInto' in X ||
  (X.encodeInto = function (n, t) {
    const e = X.encode(n);
    return t.set(e), { read: n.length, written: e.length };
  });
function w(n, t, e) {
  if (e === void 0) {
    const _ = X.encode(n),
      f = t(_.length, 1) >>> 0;
    return (
      O()
        .subarray(f, f + _.length)
        .set(_),
      (u = _.length),
      f
    );
  }
  let r = n.length,
    o = t(r, 1) >>> 0;
  const s = O();
  let a = 0;
  for (; a < r; a++) {
    const _ = n.charCodeAt(a);
    if (_ > 127) break;
    s[o + a] = _;
  }
  if (a !== r) {
    a !== 0 && (n = n.slice(a)), (o = e(o, r, (r = a + n.length * 3), 1) >>> 0);
    const _ = O().subarray(o + a, o + r),
      f = X.encodeInto(n, _);
    (a += f.written), (o = e(o, r, a, 1) >>> 0);
  }
  return (u = a), o;
}
let z = null;
function k() {
  return (
    (z === null ||
      z.buffer.detached === !0 ||
      (z.buffer.detached === void 0 && z.buffer !== i.memory.buffer)) &&
      (z = new DataView(i.memory.buffer)),
    z
  );
}
function W(n) {
  const t = i.__externref_table_alloc();
  return i.__wbindgen_export_4.set(t, n), t;
}
function d(n, t) {
  try {
    return n.apply(this, t);
  } catch (e) {
    const r = W(e);
    i.__wbindgen_exn_store(r);
  }
}
function F(n) {
  return n == null;
}
function V(n, t) {
  return (n = n >>> 0), O().subarray(n / 1, n / 1 + t);
}
function rt(n) {
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
    let s = '[';
    o > 0 && (s += rt(n[0]));
    for (let a = 1; a < o; a++) s += ', ' + rt(n[a]);
    return (s += ']'), s;
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
const at =
  typeof FinalizationRegistry > 'u'
    ? { register: () => {}, unregister: () => {} }
    : new FinalizationRegistry(n => {
        i.__wbindgen_export_6.get(n.dtor)(n.a, n.b);
      });
function $(n, t, e, r) {
  const o = { a: n, b: t, cnt: 1, dtor: e },
    s = (...a) => {
      o.cnt++;
      const _ = o.a;
      o.a = 0;
      try {
        return r(_, o.b, ...a);
      } finally {
        --o.cnt === 0
          ? (i.__wbindgen_export_6.get(o.dtor)(_, o.b), at.unregister(o))
          : (o.a = _);
      }
    };
  return (s.original = o), at.register(s, o, o), s;
}
function A(n) {
  const t = i.__wbindgen_export_4.get(n);
  return i.__externref_table_dealloc(n), t;
}
function ot(n, t) {
  if (!(n instanceof t)) throw new Error(`expected instance of ${t.name}`);
}
function _t(n, t) {
  const e = t(n.length * 1, 1) >>> 0;
  return O().set(n, e / 1), (u = n.length), e;
}
function Wt(n) {
  return ot(n, v), i.create_tonk_from_bundle(n.__wbg_ptr);
}
function vt(n, t) {
  return ot(n, v), i.create_tonk_from_bundle_with_storage(n.__wbg_ptr, t);
}
function xt(n, t) {
  const e = w(n, i.__wbindgen_malloc, i.__wbindgen_realloc),
    r = u;
  return i.create_tonk_with_config(e, r, t);
}
function At(n) {
  return i.create_tonk_from_bytes(n);
}
function $t(n) {
  const t = i.create_bundle_from_bytes(n);
  if (t[2]) throw A(t[1]);
  return v.__wrap(t[0]);
}
function zt() {
  return i.create_tonk();
}
function Ot() {
  i.init();
}
function Ut(n, t) {
  return i.create_tonk_from_bytes_with_storage(n, t);
}
function Ct(n) {
  const t = w(n, i.__wbindgen_malloc, i.__wbindgen_realloc),
    e = u;
  return i.create_tonk_with_peer_id(t, e);
}
function Mt(n) {
  return i.create_tonk_with_storage(n);
}
function jt(n) {
  i.set_time_provider(n);
}
function Lt(n, t, e) {
  const r = i.closure717_externref_shim_multivalue_shim(n, t, e);
  if (r[1]) throw A(r[0]);
}
function Nt(n, t, e) {
  i.closure1046_externref_shim(n, t, e);
}
function qt(n, t) {
  i.wasm_bindgen__convert__closures_____invoke__hc6df8564e4ba6001(n, t);
}
function nt(n, t, e) {
  i.closure719_externref_shim(n, t, e);
}
function Vt(n, t, e) {
  i.closure1022_externref_shim(n, t, e);
}
function Gt(n, t, e, r) {
  i.closure1332_externref_shim(n, t, e, r);
}
const Kt = ['blob', 'arraybuffer'],
  Ht = ['pending', 'done'],
  Jt = ['readonly', 'readwrite', 'versionchange', 'readwriteflush', 'cleanup'],
  lt =
    typeof FinalizationRegistry > 'u'
      ? { register: () => {}, unregister: () => {} }
      : new FinalizationRegistry(n => i.__wbg_wasmbundle_free(n >>> 0, 1));
class v {
  static __wrap(t) {
    t = t >>> 0;
    const e = Object.create(v.prototype);
    return (e.__wbg_ptr = t), lt.register(e, e.__wbg_ptr, e), e;
  }
  __destroy_into_raw() {
    const t = this.__wbg_ptr;
    return (this.__wbg_ptr = 0), lt.unregister(this), t;
  }
  free() {
    const t = this.__destroy_into_raw();
    i.__wbg_wasmbundle_free(t, 0);
  }
  static fromBytes(t) {
    const e = i.wasmbundle_fromBytes(t);
    if (e[2]) throw A(e[1]);
    return v.__wrap(e[0]);
  }
  getPrefix(t) {
    const e = w(t, i.__wbindgen_malloc, i.__wbindgen_realloc),
      r = u;
    return i.wasmbundle_getPrefix(this.__wbg_ptr, e, r);
  }
  getRootId() {
    return i.wasmbundle_getRootId(this.__wbg_ptr);
  }
  getManifest() {
    return i.wasmbundle_getManifest(this.__wbg_ptr);
  }
  setManifest(t) {
    return i.wasmbundle_setManifest(this.__wbg_ptr, t);
  }
  get(t) {
    const e = w(t, i.__wbindgen_malloc, i.__wbindgen_realloc),
      r = u;
    return i.wasmbundle_get(this.__wbg_ptr, e, r);
  }
  toBytes() {
    return i.wasmbundle_toBytes(this.__wbg_ptr);
  }
  listKeys() {
    return i.wasmbundle_listKeys(this.__wbg_ptr);
  }
}
Symbol.dispose && (v.prototype[Symbol.dispose] = v.prototype.free);
const ft =
  typeof FinalizationRegistry > 'u'
    ? { register: () => {}, unregister: () => {} }
    : new FinalizationRegistry(n => i.__wbg_wasmdochandle_free(n >>> 0, 1));
class U {
  static __wrap(t) {
    t = t >>> 0;
    const e = Object.create(U.prototype);
    return (e.__wbg_ptr = t), ft.register(e, e.__wbg_ptr, e), e;
  }
  __destroy_into_raw() {
    const t = this.__wbg_ptr;
    return (this.__wbg_ptr = 0), ft.unregister(this), t;
  }
  free() {
    const t = this.__destroy_into_raw();
    i.__wbg_wasmdochandle_free(t, 0);
  }
  documentId() {
    let t, e;
    try {
      const r = i.wasmdochandle_documentId(this.__wbg_ptr);
      return (t = r[0]), (e = r[1]), g(r[0], r[1]);
    } finally {
      i.__wbindgen_free(t, e, 1);
    }
  }
  getDocument() {
    const t = i.wasmdochandle_getDocument(this.__wbg_ptr);
    if (t[2]) throw A(t[1]);
    return A(t[0]);
  }
  url() {
    let t, e;
    try {
      const r = i.wasmdochandle_url(this.__wbg_ptr);
      return (t = r[0]), (e = r[1]), g(r[0], r[1]);
    } finally {
      i.__wbindgen_free(t, e, 1);
    }
  }
}
Symbol.dispose && (U.prototype[Symbol.dispose] = U.prototype.free);
const ut =
  typeof FinalizationRegistry > 'u'
    ? { register: () => {}, unregister: () => {} }
    : new FinalizationRegistry(n =>
        i.__wbg_wasmdocumentwatcher_free(n >>> 0, 1)
      );
class C {
  static __wrap(t) {
    t = t >>> 0;
    const e = Object.create(C.prototype);
    return (e.__wbg_ptr = t), ut.register(e, e.__wbg_ptr, e), e;
  }
  __destroy_into_raw() {
    const t = this.__wbg_ptr;
    return (this.__wbg_ptr = 0), ut.unregister(this), t;
  }
  free() {
    const t = this.__destroy_into_raw();
    i.__wbg_wasmdocumentwatcher_free(t, 0);
  }
  documentId() {
    let t, e;
    try {
      const r = i.wasmdocumentwatcher_documentId(this.__wbg_ptr);
      return (t = r[0]), (e = r[1]), g(r[0], r[1]);
    } finally {
      i.__wbindgen_free(t, e, 1);
    }
  }
  stop() {
    return i.wasmdocumentwatcher_stop(this.__wbg_ptr);
  }
}
Symbol.dispose && (C.prototype[Symbol.dispose] = C.prototype.free);
const dt =
  typeof FinalizationRegistry > 'u'
    ? { register: () => {}, unregister: () => {} }
    : new FinalizationRegistry(n => i.__wbg_wasmerror_free(n >>> 0, 1));
class M {
  static __wrap(t) {
    t = t >>> 0;
    const e = Object.create(M.prototype);
    return (e.__wbg_ptr = t), dt.register(e, e.__wbg_ptr, e), e;
  }
  __destroy_into_raw() {
    const t = this.__wbg_ptr;
    return (this.__wbg_ptr = 0), dt.unregister(this), t;
  }
  free() {
    const t = this.__destroy_into_raw();
    i.__wbg_wasmerror_free(t, 0);
  }
  get message() {
    let t, e;
    try {
      const r = i.wasmerror_message(this.__wbg_ptr);
      return (t = r[0]), (e = r[1]), g(r[0], r[1]);
    } finally {
      i.__wbindgen_free(t, e, 1);
    }
  }
}
Symbol.dispose && (M.prototype[Symbol.dispose] = M.prototype.free);
const wt =
  typeof FinalizationRegistry > 'u'
    ? { register: () => {}, unregister: () => {} }
    : new FinalizationRegistry(n => i.__wbg_wasmrepo_free(n >>> 0, 1));
class j {
  static __wrap(t) {
    t = t >>> 0;
    const e = Object.create(j.prototype);
    return (e.__wbg_ptr = t), wt.register(e, e.__wbg_ptr, e), e;
  }
  __destroy_into_raw() {
    const t = this.__wbg_ptr;
    return (this.__wbg_ptr = 0), wt.unregister(this), t;
  }
  free() {
    const t = this.__destroy_into_raw();
    i.__wbg_wasmrepo_free(t, 0);
  }
  findDocument(t) {
    const e = w(t, i.__wbindgen_malloc, i.__wbindgen_realloc),
      r = u;
    return i.wasmrepo_findDocument(this.__wbg_ptr, e, r);
  }
  listDocuments() {
    const t = i.wasmrepo_listDocuments(this.__wbg_ptr);
    if (t[2]) throw A(t[1]);
    return A(t[0]);
  }
  createDocument(t) {
    return i.wasmrepo_createDocument(this.__wbg_ptr, t);
  }
  connectWebSocket(t) {
    const e = w(t, i.__wbindgen_malloc, i.__wbindgen_realloc),
      r = u,
      o = i.wasmrepo_connectWebSocket(this.__wbg_ptr, e, r);
    if (o[2]) throw A(o[1]);
    return N.__wrap(o[0]);
  }
  connectWebSocketAsync(t) {
    const e = w(t, i.__wbindgen_malloc, i.__wbindgen_realloc),
      r = u;
    return i.wasmrepo_connectWebSocketAsync(this.__wbg_ptr, e, r);
  }
  constructor() {
    return i.wasmrepo_new();
  }
  stop() {
    return i.wasmrepo_stop(this.__wbg_ptr);
  }
  peerId() {
    let t, e;
    try {
      const r = i.wasmrepo_peerId(this.__wbg_ptr);
      return (t = r[0]), (e = r[1]), g(r[0], r[1]);
    } finally {
      i.__wbindgen_free(t, e, 1);
    }
  }
}
Symbol.dispose && (j.prototype[Symbol.dispose] = j.prototype.free);
const bt =
  typeof FinalizationRegistry > 'u'
    ? { register: () => {}, unregister: () => {} }
    : new FinalizationRegistry(n => i.__wbg_wasmtonkcore_free(n >>> 0, 1));
class L {
  static __wrap(t) {
    t = t >>> 0;
    const e = Object.create(L.prototype);
    return (e.__wbg_ptr = t), bt.register(e, e.__wbg_ptr, e), e;
  }
  __destroy_into_raw() {
    const t = this.__wbg_ptr;
    return (this.__wbg_ptr = 0), bt.unregister(this), t;
  }
  free() {
    const t = this.__destroy_into_raw();
    i.__wbg_wasmtonkcore_free(t, 0);
  }
  static fromBytes(t) {
    return i.wasmtonkcore_fromBytes(t);
  }
  createFile(t, e) {
    const r = w(t, i.__wbindgen_malloc, i.__wbindgen_realloc),
      o = u;
    return i.wasmtonkcore_createFile(this.__wbg_ptr, r, o, e);
  }
  deleteFile(t) {
    const e = w(t, i.__wbindgen_malloc, i.__wbindgen_realloc),
      r = u;
    return i.wasmtonkcore_deleteFile(this.__wbg_ptr, e, r);
  }
  static fromBundle(t) {
    return ot(t, v), i.wasmtonkcore_fromBundle(t.__wbg_ptr);
  }
  getPeerId() {
    return i.wasmtonkcore_getPeerId(this.__wbg_ptr);
  }
  updateFile(t, e) {
    const r = w(t, i.__wbindgen_malloc, i.__wbindgen_realloc),
      o = u;
    return i.wasmtonkcore_updateFile(this.__wbg_ptr, r, o, e);
  }
  getMetadata(t) {
    const e = w(t, i.__wbindgen_malloc, i.__wbindgen_realloc),
      r = u;
    return i.wasmtonkcore_getMetadata(this.__wbg_ptr, e, r);
  }
  static withPeerId(t) {
    const e = w(t, i.__wbindgen_malloc, i.__wbindgen_realloc),
      r = u;
    return i.wasmtonkcore_withPeerId(e, r);
  }
  forkToBytes(t) {
    return i.wasmtonkcore_forkToBytes(this.__wbg_ptr, t);
  }
  listDirectory(t) {
    const e = w(t, i.__wbindgen_malloc, i.__wbindgen_realloc),
      r = u;
    return i.wasmtonkcore_listDirectory(this.__wbg_ptr, e, r);
  }
  watchDocument(t, e) {
    const r = w(t, i.__wbindgen_malloc, i.__wbindgen_realloc),
      o = u;
    return i.wasmtonkcore_watchDocument(this.__wbg_ptr, r, o, e);
  }
  watchDirectory(t, e) {
    const r = w(t, i.__wbindgen_malloc, i.__wbindgen_realloc),
      o = u;
    return i.wasmtonkcore_watchDirectory(this.__wbg_ptr, r, o, e);
  }
  createDirectory(t) {
    const e = w(t, i.__wbindgen_malloc, i.__wbindgen_realloc),
      r = u;
    return i.wasmtonkcore_createDirectory(this.__wbg_ptr, e, r);
  }
  connectWebsocket(t) {
    const e = w(t, i.__wbindgen_malloc, i.__wbindgen_realloc),
      r = u;
    return i.wasmtonkcore_connectWebsocket(this.__wbg_ptr, e, r);
  }
  createFileWithBytes(t, e, r) {
    const o = w(t, i.__wbindgen_malloc, i.__wbindgen_realloc),
      s = u,
      a = _t(r, i.__wbindgen_malloc),
      _ = u;
    return i.wasmtonkcore_createFileWithBytes(this.__wbg_ptr, o, s, e, a, _);
  }
  updateFileWithBytes(t, e, r) {
    const o = w(t, i.__wbindgen_malloc, i.__wbindgen_realloc),
      s = u,
      a = _t(r, i.__wbindgen_malloc),
      _ = u;
    return i.wasmtonkcore_updateFileWithBytes(this.__wbg_ptr, o, s, e, a, _);
  }
  constructor() {
    return i.wasmtonkcore_new();
  }
  exists(t) {
    const e = w(t, i.__wbindgen_malloc, i.__wbindgen_realloc),
      r = u;
    return i.wasmtonkcore_exists(this.__wbg_ptr, e, r);
  }
  rename(t, e) {
    const r = w(t, i.__wbindgen_malloc, i.__wbindgen_realloc),
      o = u,
      s = w(e, i.__wbindgen_malloc, i.__wbindgen_realloc),
      a = u;
    return i.wasmtonkcore_rename(this.__wbg_ptr, r, o, s, a);
  }
  toBytes(t) {
    return i.wasmtonkcore_toBytes(this.__wbg_ptr, t);
  }
  readFile(t) {
    const e = w(t, i.__wbindgen_malloc, i.__wbindgen_realloc),
      r = u;
    return i.wasmtonkcore_readFile(this.__wbg_ptr, e, r);
  }
}
Symbol.dispose && (L.prototype[Symbol.dispose] = L.prototype.free);
const gt =
  typeof FinalizationRegistry > 'u'
    ? { register: () => {}, unregister: () => {} }
    : new FinalizationRegistry(n =>
        i.__wbg_wasmwebsockethandle_free(n >>> 0, 1)
      );
class N {
  static __wrap(t) {
    t = t >>> 0;
    const e = Object.create(N.prototype);
    return (e.__wbg_ptr = t), gt.register(e, e.__wbg_ptr, e), e;
  }
  __destroy_into_raw() {
    const t = this.__wbg_ptr;
    return (this.__wbg_ptr = 0), gt.unregister(this), t;
  }
  free() {
    const t = this.__destroy_into_raw();
    i.__wbg_wasmwebsockethandle_free(t, 0);
  }
  waitForDisconnect() {
    return i.wasmwebsockethandle_waitForDisconnect(this.__wbg_ptr);
  }
  close() {
    i.wasmwebsockethandle_close(this.__wbg_ptr);
  }
}
Symbol.dispose && (N.prototype[Symbol.dispose] = N.prototype.free);
const Yt = new Set(['basic', 'cors', 'default']);
async function Xt(n, t) {
  if (typeof Response == 'function' && n instanceof Response) {
    if (typeof WebAssembly.instantiateStreaming == 'function')
      try {
        return await WebAssembly.instantiateStreaming(n, t);
      } catch (r) {
        if (
          n.ok &&
          Yt.has(n.type) &&
          n.headers.get('Content-Type') !== 'application/wasm'
        )
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
function St() {
  const n = {};
  return (
    (n.wbg = {}),
    (n.wbg.__wbg_Error_e17e777aac105295 = function (t, e) {
      return Error(g(t, e));
    }),
    (n.wbg.__wbg_String_8f0eb39a4a4c2f66 = function (t, e) {
      const r = String(e),
        o = w(r, i.__wbindgen_malloc, i.__wbindgen_realloc),
        s = u;
      k().setInt32(t + 4 * 1, s, !0), k().setInt32(t + 4 * 0, o, !0);
    }),
    (n.wbg.__wbg_Window_41559019033ede94 = function (t) {
      return t.Window;
    }),
    (n.wbg.__wbg_WorkerGlobalScope_d324bffbeaef9f3a = function (t) {
      return t.WorkerGlobalScope;
    }),
    (n.wbg.__wbg_abort_48fad284f76f23cd = function () {
      return d(function (t) {
        t.abort();
      }, arguments);
    }),
    (n.wbg.__wbg_bound_99d0883606949696 = function () {
      return d(function (t, e, r, o) {
        return IDBKeyRange.bound(t, e, r !== 0, o !== 0);
      }, arguments);
    }),
    (n.wbg.__wbg_buffer_8d40b1d762fb3c66 = function (t) {
      return t.buffer;
    }),
    (n.wbg.__wbg_call_13410aac570ffff7 = function () {
      return d(function (t, e) {
        return t.call(e);
      }, arguments);
    }),
    (n.wbg.__wbg_call_a5400b25a865cfd8 = function () {
      return d(function (t, e, r) {
        return t.call(e, r);
      }, arguments);
    }),
    (n.wbg.__wbg_close_6437264570d2d37f = function () {
      return d(function (t) {
        t.close();
      }, arguments);
    }),
    (n.wbg.__wbg_commit_a54edce65f3858f2 = function () {
      return d(function (t) {
        t.commit();
      }, arguments);
    }),
    (n.wbg.__wbg_continue_f3937b9af363e05d = function () {
      return d(function (t) {
        t.continue();
      }, arguments);
    }),
    (n.wbg.__wbg_createObjectStore_2bc52da689ca2130 = function () {
      return d(function (t, e, r) {
        return t.createObjectStore(g(e, r));
      }, arguments);
    }),
    (n.wbg.__wbg_data_9ab529722bcc4e6c = function (t) {
      return t.data;
    }),
    (n.wbg.__wbg_delete_33e805b6d49fa644 = function () {
      return d(function (t, e) {
        return t.delete(e);
      }, arguments);
    }),
    (n.wbg.__wbg_done_75ed0ee6dd243d9d = function (t) {
      return t.done;
    }),
    (n.wbg.__wbg_entries_2be2f15bd5554996 = function (t) {
      return Object.entries(t);
    }),
    (n.wbg.__wbg_error_118f1b830b6ccf22 = function () {
      return d(function (t) {
        const e = t.error;
        return F(e) ? 0 : W(e);
      }, arguments);
    }),
    (n.wbg.__wbg_error_31d7961b8e0d3f6c = function (t, e) {
      console.error(g(t, e));
    }),
    (n.wbg.__wbg_error_7534b8e9a36f1ab4 = function (t, e) {
      let r, o;
      try {
        (r = t), (o = e), console.error(g(t, e));
      } finally {
        i.__wbindgen_free(r, o, 1);
      }
    }),
    (n.wbg.__wbg_error_99981e16d476aa5c = function (t) {
      console.error(t);
    }),
    (n.wbg.__wbg_from_88bc52ce20ba6318 = function (t) {
      return Array.from(t);
    }),
    (n.wbg.__wbg_getRandomValues_38a1ff1ea09f6cc7 = function () {
      return d(function (t, e) {
        globalThis.crypto.getRandomValues(V(t, e));
      }, arguments);
    }),
    (n.wbg.__wbg_getRandomValues_3c9c0d586e575a16 = function () {
      return d(function (t, e) {
        globalThis.crypto.getRandomValues(V(t, e));
      }, arguments);
    }),
    (n.wbg.__wbg_getTime_6bb3f64e0f18f817 = function (t) {
      return t.getTime();
    }),
    (n.wbg.__wbg_get_0da715ceaecea5c8 = function (t, e) {
      return t[e >>> 0];
    }),
    (n.wbg.__wbg_get_1b2c33a63c4be73f = function () {
      return d(function (t, e) {
        return t.get(e);
      }, arguments);
    }),
    (n.wbg.__wbg_get_458e874b43b18b25 = function () {
      return d(function (t, e) {
        return Reflect.get(t, e);
      }, arguments);
    }),
    (n.wbg.__wbg_getwithrefkey_1dc361bd10053bfe = function (t, e) {
      return t[e];
    }),
    (n.wbg.__wbg_global_f5c2926e57ba457f = function (t) {
      return t.global;
    }),
    (n.wbg.__wbg_indexedDB_003e3d885edf75fc = function () {
      return d(function (t) {
        const e = t.indexedDB;
        return F(e) ? 0 : W(e);
      }, arguments);
    }),
    (n.wbg.__wbg_indexedDB_1956995e4297311c = function () {
      return d(function (t) {
        const e = t.indexedDB;
        return F(e) ? 0 : W(e);
      }, arguments);
    }),
    (n.wbg.__wbg_indexedDB_54f01430b1e194e8 = function () {
      return d(function (t) {
        const e = t.indexedDB;
        return F(e) ? 0 : W(e);
      }, arguments);
    }),
    (n.wbg.__wbg_instanceof_ArrayBuffer_67f3012529f6a2dd = function (t) {
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
    (n.wbg.__wbg_instanceof_DomException_bd63c2a0e0b53ed5 = function (t) {
      let e;
      try {
        e = t instanceof DOMException;
      } catch {
        e = !1;
      }
      return e;
    }),
    (n.wbg.__wbg_instanceof_Error_76149ae9b431750e = function (t) {
      let e;
      try {
        e = t instanceof Error;
      } catch {
        e = !1;
      }
      return e;
    }),
    (n.wbg.__wbg_instanceof_IdbCursor_2d6e2a2a3b9fb015 = function (t) {
      let e;
      try {
        e = t instanceof IDBCursor;
      } catch {
        e = !1;
      }
      return e;
    }),
    (n.wbg.__wbg_instanceof_IdbDatabase_6e6efef94c4a355d = function (t) {
      let e;
      try {
        e = t instanceof IDBDatabase;
      } catch {
        e = !1;
      }
      return e;
    }),
    (n.wbg.__wbg_instanceof_IdbRequest_a4a68ff63181a915 = function (t) {
      let e;
      try {
        e = t instanceof IDBRequest;
      } catch {
        e = !1;
      }
      return e;
    }),
    (n.wbg.__wbg_instanceof_Map_ebb01a5b6b5ffd0b = function (t) {
      let e;
      try {
        e = t instanceof Map;
      } catch {
        e = !1;
      }
      return e;
    }),
    (n.wbg.__wbg_instanceof_Uint8Array_9a8378d955933db7 = function (t) {
      let e;
      try {
        e = t instanceof Uint8Array;
      } catch {
        e = !1;
      }
      return e;
    }),
    (n.wbg.__wbg_isArray_030cce220591fb41 = function (t) {
      return Array.isArray(t);
    }),
    (n.wbg.__wbg_isSafeInteger_1c0d1af5542e102a = function (t) {
      return Number.isSafeInteger(t);
    }),
    (n.wbg.__wbg_item_dc4321784299a463 = function (t, e, r) {
      const o = e.item(r >>> 0);
      var s = F(o) ? 0 : w(o, i.__wbindgen_malloc, i.__wbindgen_realloc),
        a = u;
      k().setInt32(t + 4 * 1, a, !0), k().setInt32(t + 4 * 0, s, !0);
    }),
    (n.wbg.__wbg_iterator_f370b34483c71a1c = function () {
      return Symbol.iterator;
    }),
    (n.wbg.__wbg_key_4c46cfeacd2dbc18 = function () {
      return d(function (t) {
        return t.key;
      }, arguments);
    }),
    (n.wbg.__wbg_keys_ef52390b2ae0e714 = function (t) {
      return Object.keys(t);
    }),
    (n.wbg.__wbg_length_186546c51cd61acd = function (t) {
      return t.length;
    }),
    (n.wbg.__wbg_length_6bb7e81f9d7713e4 = function (t) {
      return t.length;
    }),
    (n.wbg.__wbg_log_0cc1b7768397bcfe = function (t, e, r, o, s, a, _, f) {
      let S, E;
      try {
        (S = t), (E = e), console.log(g(t, e), g(r, o), g(s, a), g(_, f));
      } finally {
        i.__wbindgen_free(S, E, 1);
      }
    }),
    (n.wbg.__wbg_log_2f4a53bbb94ad21b = function (t, e) {
      console.log(g(t, e));
    }),
    (n.wbg.__wbg_log_6c7b5f4f00b8ce3f = function (t) {
      console.log(t);
    }),
    (n.wbg.__wbg_log_cb9e190acc5753fb = function (t, e) {
      let r, o;
      try {
        (r = t), (o = e), console.log(g(t, e));
      } finally {
        i.__wbindgen_free(r, o, 1);
      }
    }),
    (n.wbg.__wbg_lowerBound_5a50c0a9f6e7db91 = function () {
      return d(function (t, e) {
        return IDBKeyRange.lowerBound(t, e !== 0);
      }, arguments);
    }),
    (n.wbg.__wbg_mark_7438147ce31e9d4b = function (t, e) {
      performance.mark(g(t, e));
    }),
    (n.wbg.__wbg_measure_fb7825c11612c823 = function () {
      return d(function (t, e, r, o) {
        let s, a, _, f;
        try {
          (s = t),
            (a = e),
            (_ = r),
            (f = o),
            performance.measure(g(t, e), g(r, o));
        } finally {
          i.__wbindgen_free(s, a, 1), i.__wbindgen_free(_, f, 1);
        }
      }, arguments);
    }),
    (n.wbg.__wbg_message_125a1b2998b3552a = function (t) {
      return t.message;
    }),
    (n.wbg.__wbg_message_5481231e71ccaf7b = function (t, e) {
      const r = e.message,
        o = w(r, i.__wbindgen_malloc, i.__wbindgen_realloc),
        s = u;
      k().setInt32(t + 4 * 1, s, !0), k().setInt32(t + 4 * 0, o, !0);
    }),
    (n.wbg.__wbg_message_702ebc62fa8b0c7c = function (t, e) {
      const r = e.message,
        o = w(r, i.__wbindgen_malloc, i.__wbindgen_realloc),
        s = u;
      k().setInt32(t + 4 * 1, s, !0), k().setInt32(t + 4 * 0, o, !0);
    }),
    (n.wbg.__wbg_name_f75f535832c8ea6b = function (t, e) {
      const r = e.name,
        o = w(r, i.__wbindgen_malloc, i.__wbindgen_realloc),
        s = u;
      k().setInt32(t + 4 * 1, s, !0), k().setInt32(t + 4 * 0, o, !0);
    }),
    (n.wbg.__wbg_new0_b0a0a38c201e6df5 = function () {
      return new Date();
    }),
    (n.wbg.__wbg_new_19c25a3f2fa63a02 = function () {
      return new Object();
    }),
    (n.wbg.__wbg_new_1f3a344cf3123716 = function () {
      return new Array();
    }),
    (n.wbg.__wbg_new_2e3c58a15f39f5f9 = function (t, e) {
      try {
        var r = { a: t, b: e },
          o = (a, _) => {
            const f = r.a;
            r.a = 0;
            try {
              return Gt(f, r.b, a, _);
            } finally {
              r.a = f;
            }
          };
        return new Promise(o);
      } finally {
        r.a = r.b = 0;
      }
    }),
    (n.wbg.__wbg_new_2ff1f68f3676ea53 = function () {
      return new Map();
    }),
    (n.wbg.__wbg_new_638ebfaedbf32a5e = function (t) {
      return new Uint8Array(t);
    }),
    (n.wbg.__wbg_new_8a6f238a6ece86ea = function () {
      return new Error();
    }),
    (n.wbg.__wbg_new_da9dc54c5db29dfa = function (t, e) {
      return new Error(g(t, e));
    }),
    (n.wbg.__wbg_new_e213f63d18b0de01 = function () {
      return d(function (t, e) {
        return new WebSocket(g(t, e));
      }, arguments);
    }),
    (n.wbg.__wbg_newfromslice_074c56947bd43469 = function (t, e) {
      return new Uint8Array(V(t, e));
    }),
    (n.wbg.__wbg_newnoargs_254190557c45b4ec = function (t, e) {
      return new Function(g(t, e));
    }),
    (n.wbg.__wbg_newwithlength_a167dcc7aaa3ba77 = function (t) {
      return new Uint8Array(t >>> 0);
    }),
    (n.wbg.__wbg_next_5b3530e612fde77d = function (t) {
      return t.next;
    }),
    (n.wbg.__wbg_next_692e82279131b03c = function () {
      return d(function (t) {
        return t.next();
      }, arguments);
    }),
    (n.wbg.__wbg_now_1e80617bcee43265 = function () {
      return Date.now();
    }),
    (n.wbg.__wbg_objectStoreNames_31ac72154caf5a01 = function (t) {
      return t.objectStoreNames;
    }),
    (n.wbg.__wbg_objectStore_b2a5b80b2e5c5f8b = function () {
      return d(function (t, e, r) {
        return t.objectStore(g(e, r));
      }, arguments);
    }),
    (n.wbg.__wbg_openCursor_2a1cca3c492dd5ec = function () {
      return d(function (t) {
        return t.openCursor();
      }, arguments);
    }),
    (n.wbg.__wbg_open_7281831ed8ff7bd2 = function () {
      return d(function (t, e, r, o) {
        return t.open(g(e, r), o >>> 0);
      }, arguments);
    }),
    (n.wbg.__wbg_prototypesetcall_3d4a26c1ed734349 = function (t, e, r) {
      Uint8Array.prototype.set.call(V(t, e), r);
    }),
    (n.wbg.__wbg_push_330b2eb93e4e1212 = function (t, e) {
      return t.push(e);
    }),
    (n.wbg.__wbg_put_cdfadd5d7f714201 = function () {
      return d(function (t, e, r) {
        return t.put(e, r);
      }, arguments);
    }),
    (n.wbg.__wbg_queueMicrotask_25d0739ac89e8c88 = function (t) {
      queueMicrotask(t);
    }),
    (n.wbg.__wbg_queueMicrotask_4488407636f5bf24 = function (t) {
      return t.queueMicrotask;
    }),
    (n.wbg.__wbg_readyState_a2853902a50c6d54 = function (t) {
      const e = t.readyState;
      return (Ht.indexOf(e) + 1 || 3) - 1;
    }),
    (n.wbg.__wbg_request_5079471e06223120 = function (t) {
      return t.request;
    }),
    (n.wbg.__wbg_request_fada8c23b78b3a02 = function (t) {
      return t.request;
    }),
    (n.wbg.__wbg_resolve_4055c623acdd6a1b = function (t) {
      return Promise.resolve(t);
    }),
    (n.wbg.__wbg_result_825a6aeeb31189d2 = function () {
      return d(function (t) {
        return t.result;
      }, arguments);
    }),
    (n.wbg.__wbg_send_0f09f4487d932d86 = function () {
      return d(function (t, e) {
        t.send(e);
      }, arguments);
    }),
    (n.wbg.__wbg_set_1353b2a5e96bc48c = function (t, e, r) {
      t.set(V(e, r));
    }),
    (n.wbg.__wbg_set_3f1d0b984ed272ed = function (t, e, r) {
      t[e] = r;
    }),
    (n.wbg.__wbg_set_453345bcda80b89a = function () {
      return d(function (t, e, r) {
        return Reflect.set(t, e, r);
      }, arguments);
    }),
    (n.wbg.__wbg_set_90f6c0f7bd8c0415 = function (t, e, r) {
      t[e >>> 0] = r;
    }),
    (n.wbg.__wbg_set_b7f1cf4fae26fe2a = function (t, e, r) {
      return t.set(e, r);
    }),
    (n.wbg.__wbg_setbinaryType_37f3cd35d7775a47 = function (t, e) {
      t.binaryType = Kt[e];
    }),
    (n.wbg.__wbg_setonabort_4edac498cf4576fe = function (t, e) {
      t.onabort = e;
    }),
    (n.wbg.__wbg_setonclose_159c0332c2d91b09 = function (t, e) {
      t.onclose = e;
    }),
    (n.wbg.__wbg_setoncomplete_8a32ad2d1ca4f49b = function (t, e) {
      t.oncomplete = e;
    }),
    (n.wbg.__wbg_setonerror_4b0c685c365f600d = function (t, e) {
      t.onerror = e;
    }),
    (n.wbg.__wbg_setonerror_5d9bff045f909e89 = function (t, e) {
      t.onerror = e;
    }),
    (n.wbg.__wbg_setonerror_bcdbd7f3921ffb1f = function (t, e) {
      t.onerror = e;
    }),
    (n.wbg.__wbg_setonmessage_5e486f326638a9da = function (t, e) {
      t.onmessage = e;
    }),
    (n.wbg.__wbg_setonopen_3e43af381c2901f8 = function (t, e) {
      t.onopen = e;
    }),
    (n.wbg.__wbg_setonsuccess_ffb2ddb27ce681d8 = function (t, e) {
      t.onsuccess = e;
    }),
    (n.wbg.__wbg_setonupgradeneeded_4e32d1c6a08c4257 = function (t, e) {
      t.onupgradeneeded = e;
    }),
    (n.wbg.__wbg_stack_0ed75d68575b0f3c = function (t, e) {
      const r = e.stack,
        o = w(r, i.__wbindgen_malloc, i.__wbindgen_realloc),
        s = u;
      k().setInt32(t + 4 * 1, s, !0), k().setInt32(t + 4 * 0, o, !0);
    }),
    (n.wbg.__wbg_static_accessor_GLOBAL_8921f820c2ce3f12 = function () {
      const t = typeof globalThis > 'u' ? null : globalThis;
      return F(t) ? 0 : W(t);
    }),
    (n.wbg.__wbg_static_accessor_GLOBAL_THIS_f0a4409105898184 = function () {
      const t = typeof globalThis > 'u' ? null : globalThis;
      return F(t) ? 0 : W(t);
    }),
    (n.wbg.__wbg_static_accessor_SELF_995b214ae681ff99 = function () {
      const t = typeof self > 'u' ? null : self;
      return F(t) ? 0 : W(t);
    }),
    (n.wbg.__wbg_static_accessor_WINDOW_cde3890479c675ea = function () {
      const t = typeof window > 'u' ? null : window;
      return F(t) ? 0 : W(t);
    }),
    (n.wbg.__wbg_target_f2c963b447be6283 = function (t) {
      const e = t.target;
      return F(e) ? 0 : W(e);
    }),
    (n.wbg.__wbg_then_b33a773d723afa3e = function (t, e, r) {
      return t.then(e, r);
    }),
    (n.wbg.__wbg_then_e22500defe16819f = function (t, e) {
      return t.then(e);
    }),
    (n.wbg.__wbg_toString_78df35411a4fd40c = function (t) {
      return t.toString();
    }),
    (n.wbg.__wbg_transaction_553a104dd139f032 = function () {
      return d(function (t, e, r, o) {
        return t.transaction(g(e, r), Jt[o]);
      }, arguments);
    }),
    (n.wbg.__wbg_transaction_b51dc7b903eb86c1 = function (t) {
      return t.transaction;
    }),
    (n.wbg.__wbg_upperBound_884e6dbf6030d98b = function () {
      return d(function (t, e) {
        return IDBKeyRange.upperBound(t, e !== 0);
      }, arguments);
    }),
    (n.wbg.__wbg_value_809430714c127bb5 = function () {
      return d(function (t) {
        return t.value;
      }, arguments);
    }),
    (n.wbg.__wbg_value_dd9372230531eade = function (t) {
      return t.value;
    }),
    (n.wbg.__wbg_wasmdochandle_new = function (t) {
      return U.__wrap(t);
    }),
    (n.wbg.__wbg_wasmdocumentwatcher_new = function (t) {
      return C.__wrap(t);
    }),
    (n.wbg.__wbg_wasmerror_new = function (t) {
      return M.__wrap(t);
    }),
    (n.wbg.__wbg_wasmrepo_new = function (t) {
      return j.__wrap(t);
    }),
    (n.wbg.__wbg_wasmtonkcore_new = function (t) {
      return L.__wrap(t);
    }),
    (n.wbg.__wbg_wbindgenbigintgetasi64_ac743ece6ab9bba1 = function (t, e) {
      const r = e,
        o = typeof r == 'bigint' ? r : void 0;
      k().setBigInt64(t + 8 * 1, F(o) ? BigInt(0) : o, !0),
        k().setInt32(t + 4 * 0, !F(o), !0);
    }),
    (n.wbg.__wbg_wbindgenbooleanget_3fe6f642c7d97746 = function (t) {
      const e = t,
        r = typeof e == 'boolean' ? e : void 0;
      return F(r) ? 16777215 : r ? 1 : 0;
    }),
    (n.wbg.__wbg_wbindgencbdrop_eb10308566512b88 = function (t) {
      const e = t.original;
      return e.cnt-- == 1 ? ((e.a = 0), !0) : !1;
    }),
    (n.wbg.__wbg_wbindgendebugstring_99ef257a3ddda34d = function (t, e) {
      const r = rt(e),
        o = w(r, i.__wbindgen_malloc, i.__wbindgen_realloc),
        s = u;
      k().setInt32(t + 4 * 1, s, !0), k().setInt32(t + 4 * 0, o, !0);
    }),
    (n.wbg.__wbg_wbindgenin_d7a1ee10933d2d55 = function (t, e) {
      return t in e;
    }),
    (n.wbg.__wbg_wbindgenisbigint_ecb90cc08a5a9154 = function (t) {
      return typeof t == 'bigint';
    }),
    (n.wbg.__wbg_wbindgenisfunction_8cee7dce3725ae74 = function (t) {
      return typeof t == 'function';
    }),
    (n.wbg.__wbg_wbindgenisnull_f3037694abe4d97a = function (t) {
      return t === null;
    }),
    (n.wbg.__wbg_wbindgenisobject_307a53c6bd97fbf8 = function (t) {
      const e = t;
      return typeof e == 'object' && e !== null;
    }),
    (n.wbg.__wbg_wbindgenisstring_d4fa939789f003b0 = function (t) {
      return typeof t == 'string';
    }),
    (n.wbg.__wbg_wbindgenisundefined_c4b71d073b92f3c5 = function (t) {
      return t === void 0;
    }),
    (n.wbg.__wbg_wbindgenjsvaleq_e6f2ad59ccae1b58 = function (t, e) {
      return t === e;
    }),
    (n.wbg.__wbg_wbindgenjsvallooseeq_9bec8c9be826bed1 = function (t, e) {
      return t == e;
    }),
    (n.wbg.__wbg_wbindgennumberget_f74b4c7525ac05cb = function (t, e) {
      const r = e,
        o = typeof r == 'number' ? r : void 0;
      k().setFloat64(t + 8 * 1, F(o) ? 0 : o, !0),
        k().setInt32(t + 4 * 0, !F(o), !0);
    }),
    (n.wbg.__wbg_wbindgenstringget_0f16a6ddddef376f = function (t, e) {
      const r = e,
        o = typeof r == 'string' ? r : void 0;
      var s = F(o) ? 0 : w(o, i.__wbindgen_malloc, i.__wbindgen_realloc),
        a = u;
      k().setInt32(t + 4 * 1, a, !0), k().setInt32(t + 4 * 0, s, !0);
    }),
    (n.wbg.__wbg_wbindgenthrow_451ec1a8469d7eb6 = function (t, e) {
      throw new Error(g(t, e));
    }),
    (n.wbg.__wbindgen_cast_2241b6af4c4b2941 = function (t, e) {
      return g(t, e);
    }),
    (n.wbg.__wbindgen_cast_2ffc91799785e80c = function (t, e) {
      return $(t, e, 716, nt);
    }),
    (n.wbg.__wbindgen_cast_45536695c046ade9 = function (t, e) {
      return $(t, e, 716, nt);
    }),
    (n.wbg.__wbindgen_cast_4625c577ab2ec9ee = function (t) {
      return BigInt.asUintN(64, t);
    }),
    (n.wbg.__wbindgen_cast_701f9ac6a5769b02 = function (t, e) {
      return $(t, e, 1019, Vt);
    }),
    (n.wbg.__wbindgen_cast_9ae0607507abb057 = function (t) {
      return t;
    }),
    (n.wbg.__wbindgen_cast_b5e50878beb44be5 = function (t, e) {
      return $(t, e, 716, Lt);
    }),
    (n.wbg.__wbindgen_cast_d6cd19b81560fd6e = function (t) {
      return t;
    }),
    (n.wbg.__wbindgen_cast_de82460c3994ad30 = function (t, e) {
      return $(t, e, 1045, Nt);
    }),
    (n.wbg.__wbindgen_cast_e58bb0034a0b42ed = function (t, e) {
      return $(t, e, 716, nt);
    }),
    (n.wbg.__wbindgen_cast_f6e44a557fea765a = function (t, e) {
      return $(t, e, 1019, qt);
    }),
    (n.wbg.__wbindgen_init_externref_table = function () {
      const t = i.__wbindgen_export_4,
        e = t.grow(4);
      t.set(0, void 0),
        t.set(e + 0, void 0),
        t.set(e + 1, null),
        t.set(e + 2, !0),
        t.set(e + 3, !1);
    }),
    n
  );
}
function Ft(n, t) {
  return (
    (i = n.exports),
    (tt.__wbindgen_wasm_module = t),
    (z = null),
    (H = null),
    i.__wbindgen_start(),
    i
  );
}
function Qt(n) {
  if (i !== void 0) return i;
  typeof n < 'u' &&
    (Object.getPrototypeOf(n) === Object.prototype
      ? ({ module: n } = n)
      : console.warn(
          'using deprecated parameters for `initSync()`; pass a single object instead'
        ));
  const t = St();
  n instanceof WebAssembly.Module || (n = new WebAssembly.Module(n));
  const e = new WebAssembly.Instance(n, t);
  return Ft(e, n);
}
async function tt(n) {
  if (i !== void 0) return i;
  typeof n < 'u' &&
    (Object.getPrototypeOf(n) === Object.prototype
      ? ({ module_or_path: n } = n)
      : console.warn(
          'using deprecated parameters for the initialization function; pass a single object instead'
        )),
    typeof n > 'u' &&
      (n = new URL('/assets/tonk_core_bg-DyDceEE4.wasm', import.meta.url));
  const t = St();
  (typeof n == 'string' ||
    (typeof Request == 'function' && n instanceof Request) ||
    (typeof URL == 'function' && n instanceof URL)) &&
    (n = fetch(n));
  const { instance: e, module: r } = await Xt(await n, t);
  return Ft(e, r);
}
const J = Object.freeze(
  Object.defineProperty(
    {
      __proto__: null,
      WasmBundle: v,
      WasmDocHandle: U,
      WasmDocumentWatcher: C,
      WasmError: M,
      WasmRepo: j,
      WasmTonkCore: L,
      WasmWebSocketHandle: N,
      create_bundle_from_bytes: $t,
      create_tonk: zt,
      create_tonk_from_bundle: Wt,
      create_tonk_from_bundle_with_storage: vt,
      create_tonk_from_bytes: At,
      create_tonk_from_bytes_with_storage: Ut,
      create_tonk_with_config: xt,
      create_tonk_with_peer_id: Ct,
      create_tonk_with_storage: Mt,
      default: tt,
      init: Ot,
      initSync: Qt,
      set_time_provider: jt,
    },
    Symbol.toStringTag,
    { value: 'Module' }
  )
);
let ht = !1,
  G = null;
async function pt(n) {
  if (!ht)
    return (
      G ||
      ((G = (async () => {
        try {
          n?.wasmPath ? await tt(n.wasmPath) : await tt(), (ht = !0);
        } catch (t) {
          throw ((G = null), t);
        }
      })()),
      G)
    );
}
const Et = !0;
console.log('🚀 SERVICE WORKER: Script loaded at', new Date().toISOString());
console.log('🔍 DEBUG_LOGGING enabled:', Et);
console.log('🌐 Service worker location:', self.location.href);
console.log('UNIQUE ID:', 779);
let y = { status: 'uninitialized' },
  I = null;
function T() {
  return y.status === 'ready' ? y : null;
}
function c(n, t, e) {
  const o = `[VFS Service Worker ${new Date().toISOString()}] ${n.toUpperCase()}:`;
  e !== void 0 ? console[n](o, t, e) : console[n](o, t);
}
const P = new Map();
async function l(n) {
  c('info', 'Posting response to main thread', {
    type: n.type,
    success: 'success' in n ? n.success : 'N/A',
  }),
    (await self.clients.matchAll()).forEach(e => {
      e.postMessage(n);
    });
}
c('info', 'Service worker installed, waiting for bundle');
console.log('🚀 SERVICE WORKER: Installed, waiting for bundle');
y = { status: 'uninitialized' };
self.addEventListener('install', n => {
  c('info', 'Service worker installing'),
    console.log('🔧 SERVICE WORKER: Installing SW'),
    self.skipWaiting(),
    c('info', 'Service worker install completed, skipWaiting called');
});
self.addEventListener('activate', n => {
  c('info', 'Service worker activating'),
    console.log('🚀 SERVICE WORKER: Activating service worker.'),
    n.waitUntil(
      (async () => {
        await self.clients.claim(),
          c('info', 'Service worker activation completed, clients claimed'),
          (await self.clients.matchAll()).forEach(e => {
            e.postMessage({ type: 'ready', needsBundle: !0 });
          }),
          c(
            'info',
            'Service worker activated, ready message sent to all clients'
          ),
          console.log('🚀 SERVICE WORKER: Activated and ready');
      })()
    );
});
const yt = async (n, t) => {
    if (n.bytes) {
      const e = atob(n.bytes),
        r = new Uint8Array(e.length);
      for (let o = 0; o < e.length; o++) r[o] = e.charCodeAt(o);
      return new Response(r, { headers: { 'Content-Type': n.content.mime } });
    } else
      return new Response(n.content, {
        headers: { 'Content-Type': 'application/json' },
      });
  },
  mt = n => {
    if (
      (console.log('🎯 determinePath START', {
        url: n.href,
        pathname: n.pathname,
        appSlug: I || 'none',
      }),
      !I)
    )
      throw (
        (console.error('determinePath - NO APP SLUG SET'),
        new Error(`No app slug available for ${n.pathname}`))
      );
    const t = new URL(self.registration?.scope ?? self.location.href).pathname,
      e = n.pathname.startsWith(t) ? n.pathname.slice(t.length) : n.pathname,
      r = e.replace(/^\/+/, '').split('/').filter(Boolean);
    console.log('determinePath - segments', {
      scopePath: t,
      strippedPath: e,
      segments: [...r],
      firstSegment: r[0] || 'none',
    });
    let o = r;
    if (
      (r[0] === I
        ? ((o = r.slice(1)),
          console.log(
            'determinePath - appSlug already present, using remaining segments',
            { pathSegments: [...o] }
          ))
        : console.log(
            'determinePath - appSlug not present, using all segments as path',
            { pathSegments: [...o] }
          ),
      o.length === 0 || n.pathname.endsWith('/'))
    ) {
      const a = `${I}/index.html`;
      return (
        console.log('determinePath - defaulting to index.html', { result: a }),
        a
      );
    }
    const s = `${I}/${o.join('/')}`;
    return console.log('determinePath - returning file path', { result: s }), s;
  };
self.addEventListener('fetch', n => {
  const t = new URL(n.request.url),
    e = n.request.referrer;
  console.log(
    '🔥 FETCH EVENT:',
    t.href,
    'Origin match:',
    t.origin === location.origin,
    'Pathname:',
    t.pathname
  ),
    console.log('🔥 Tonk state:', y.status),
    console.log('🔥 Current appSlug:', I),
    c('info', 'Fetch event received', {
      url: t.href,
      origin: t.origin,
      pathname: t.pathname,
      referrer: e,
      method: n.request.method,
      matchesOrigin: t.origin === location.origin,
    });
  const r = t.pathname === '/' || t.pathname === '';
  r && I && (console.log('DOING THE THING'), (I = null)),
    I && t.origin === location.origin && !r
      ? (c('info', 'Processing fetch request for same origin (non-root)'),
        n.respondWith(
          (async () => {
            try {
              const o = mt(t);
              c('info', 'Determined path for request', {
                path: o,
                originalUrl: t.href,
              });
              const s = T();
              if (
                (c('info', 'Retrieved Tonk instance', {
                  hasTonkInstance: !!s,
                  tonkState: y.status,
                  tonkStateDetails:
                    y.status === 'ready'
                      ? 'ready'
                      : y.status === 'loading'
                        ? 'loading'
                        : y.status === 'failed'
                          ? 'failed'
                          : 'uninitialized',
                }),
                !s)
              )
                throw (
                  (c('error', 'Tonk not initialized - cannot handle request', {
                    tonkState: y.status,
                    path: o,
                    url: t.href,
                  }),
                  new Error('Tonk not initialized'))
                );
              const a = `/app/${o}`;
              if (!(await s.tonk.exists(a))) {
                console.warn(
                  `🚨 File not found: ${a}, falling back to index.html`
                ),
                  c('warn', 'File not found, falling back to index.html', {
                    requestedPath: a,
                    fallbackPath: `/app/${I}/index.html`,
                  });
                const E = `/app/${I}/index.html`,
                  b = await s.tonk.readFile(E);
                return (
                  c('info', 'Successfully read index.html fallback', {
                    filePath: E,
                    hasContent: !!b.content,
                    hasBytes: !!b.bytes,
                  }),
                  yt(b, o)
                );
              }
              c('info', 'File exists, attempting to read from Tonk', {
                filePath: a,
              });
              const f = await s.tonk.readFile(a);
              c('info', 'Successfully read file from Tonk', {
                filePath: a,
                hasContent: !!f.content,
                hasBytes: !!f.bytes,
                contentType: f.content ? f.content.mime : 'unknown',
              });
              const S = yt(f, o);
              return (
                c('info', 'Created response for file', {
                  filePath: `/app/${o}`,
                  responseType: f.bytes ? 'binary' : 'text',
                }),
                S
              );
            } catch (o) {
              return (
                c(
                  'error',
                  'Failed to fetch file from Tonk, falling back to original request',
                  {
                    error: o instanceof Error ? o.message : String(o),
                    path: mt(t),
                    url: t.href,
                    tonkState: y.status,
                  }
                ),
                c('info', 'Falling back to original fetch request', {
                  url: t.href,
                }),
                fetch(n.request)
              );
            }
          })()
        ))
      : c('info', 'Ignoring fetch request for different origin', {
          requestOrigin: t.origin,
          serviceWorkerOrigin: location.origin,
        });
});
async function Zt(n) {
  if (
    (c('info', 'Received message', {
      type: n.type,
      id: 'id' in n ? n.id : 'N/A',
    }),
    n.type === 'setAppSlug')
  ) {
    (I = n.slug), c('info', 'App slug set', { slug: I });
    return;
  }
  const t = ['init', 'loadBundle', 'initializeFromUrl'];
  if (y.status !== 'ready' && !t.includes(n.type)) {
    c('error', 'Operation attempted before VFS initialization', {
      type: n.type,
      status: y.status,
    }),
      'id' in n &&
        l({
          type: n.type,
          id: n.id,
          success: !1,
          error: 'VFS not initialized. Please load a bundle first.',
        });
    return;
  }
  switch (n.type) {
    case 'init':
      c('info', 'Handling VFS init message', {
        manifestSize: n.manifest.byteLength,
        wsUrl: n.wsUrl,
        id: 'id' in n ? n.id : 'N/A',
      });
      try {
        if (y.status === 'ready') {
          c('info', 'Tonk already initialized, responding with success'),
            l({ type: 'init', success: !0 });
          return;
        }
        if (y.status === 'loading') {
          c('info', 'Tonk is loading, waiting for completion');
          try {
            const { tonk: e, manifest: r } = await y.promise;
            c('info', 'Tonk loading completed, responding with success'),
              l({ type: 'init', success: !0 });
          } catch (e) {
            c('error', 'Tonk loading failed', {
              error: e instanceof Error ? e.message : String(e),
            }),
              l({
                type: 'init',
                success: !1,
                error: e instanceof Error ? e.message : String(e),
              });
          }
          return;
        }
        if (y.status === 'failed') {
          c('error', 'Tonk initialization failed previously', {
            error: y.error.message,
          }),
            l({ type: 'init', success: !1, error: y.error.message });
          return;
        }
        c('warn', 'Tonk is uninitialized, this is unexpected'),
          l({ type: 'init', success: !1, error: 'Tonk not initialized' });
      } catch (e) {
        c('error', 'Failed to handle init message', {
          error: e instanceof Error ? e.message : String(e),
        }),
          l({
            type: 'init',
            success: !1,
            error: e instanceof Error ? e.message : String(e),
          });
      }
      break;
    case 'readFile':
      c('info', 'Reading file', { path: n.path, id: n.id });
      try {
        const { tonk: e } = T(),
          r = await e.readFile(n.path);
        c('info', 'File read successfully', {
          path: n.path,
          documentType: r.type,
          hasBytes: !!r.bytes,
          contentType: typeof r.content,
        }),
          l({ type: 'readFile', id: n.id, success: !0, data: r });
      } catch (e) {
        c('error', 'Failed to read file', {
          path: n.path,
          error: e instanceof Error ? e.message : String(e),
        }),
          l({
            type: 'readFile',
            id: n.id,
            success: !1,
            error: e instanceof Error ? e.message : String(e),
          });
      }
      break;
    case 'writeFile':
      c('info', 'Writing file', {
        path: n.path,
        id: n.id,
        create: n.create,
        hasBytes: !!n.content.bytes,
        contentType: typeof n.content.content,
      });
      try {
        const { tonk: e } = T();
        n.create
          ? (c('info', 'Creating new file', { path: n.path }),
            n.content.bytes
              ? await e.createFileWithBytes(
                  n.path,
                  n.content.content,
                  n.content.bytes
                )
              : await e.createFile(n.path, n.content.content))
          : (c('info', 'Updating existing file', { path: n.path }),
            n.content.bytes
              ? await e.updateFileWithBytes(
                  n.path,
                  n.content.content,
                  n.content.bytes
                )
              : await e.updateFile(n.path, n.content.content)),
          c('info', 'File write completed successfully', { path: n.path }),
          l({ type: 'writeFile', id: n.id, success: !0 });
      } catch (e) {
        c('error', 'Failed to write file', {
          path: n.path,
          create: n.create,
          error: e instanceof Error ? e.message : String(e),
        }),
          l({
            type: 'writeFile',
            id: n.id,
            success: !1,
            error: e instanceof Error ? e.message : String(e),
          });
      }
      break;
    case 'deleteFile':
      c('info', 'Deleting file', { path: n.path, id: n.id });
      try {
        const { tonk: e } = T();
        await e.deleteFile(n.path),
          c('info', 'File deleted successfully', { path: n.path }),
          l({ type: 'deleteFile', id: n.id, success: !0 });
      } catch (e) {
        c('error', 'Failed to delete file', {
          path: n.path,
          error: e instanceof Error ? e.message : String(e),
        }),
          l({
            type: 'deleteFile',
            id: n.id,
            success: !1,
            error: e instanceof Error ? e.message : String(e),
          });
      }
      break;
    case 'rename':
      c('info', 'Renaming file or directory', {
        oldPath: n.oldPath,
        newPath: n.newPath,
        id: n.id,
      });
      try {
        const { tonk: e } = T();
        await e.rename(n.oldPath, n.newPath),
          c('info', 'Rename completed successfully', {
            oldPath: n.oldPath,
            newPath: n.newPath,
          }),
          l({ type: 'rename', id: n.id, success: !0 });
      } catch (e) {
        c('error', 'Failed to rename', {
          oldPath: n.oldPath,
          newPath: n.newPath,
          error: e instanceof Error ? e.message : String(e),
        }),
          l({
            type: 'rename',
            id: n.id,
            success: !1,
            error: e instanceof Error ? e.message : String(e),
          });
      }
      break;
    case 'listDirectory':
      c('info', 'Listing directory', { path: n.path, id: n.id });
      try {
        const { tonk: e } = T(),
          r = await e.listDirectory(n.path);
        c('info', 'Directory listed successfully', {
          path: n.path,
          fileCount: Array.isArray(r) ? r.length : 'unknown',
        }),
          l({ type: 'listDirectory', id: n.id, success: !0, data: r });
      } catch (e) {
        c('error', 'Failed to list directory', {
          path: n.path,
          error: e instanceof Error ? e.message : String(e),
        }),
          l({
            type: 'listDirectory',
            id: n.id,
            success: !1,
            error: e instanceof Error ? e.message : String(e),
          });
      }
      break;
    case 'exists':
      c('info', 'Checking file existence', { path: n.path, id: n.id });
      try {
        const { tonk: e } = T(),
          r = await e.exists(n.path);
        c('info', 'File existence check completed', {
          path: n.path,
          exists: r,
        }),
          l({ type: 'exists', id: n.id, success: !0, data: r });
      } catch (e) {
        c('error', 'Failed to check file existence', {
          path: n.path,
          error: e instanceof Error ? e.message : String(e),
        }),
          l({
            type: 'exists',
            id: n.id,
            success: !1,
            error: e instanceof Error ? e.message : String(e),
          });
      }
      break;
    case 'watchFile':
      c('info', 'Starting file watch', { path: n.path, id: n.id });
      try {
        const { tonk: e } = T(),
          r = await e.watchFile(n.path, o => {
            c('info', 'File change detected', {
              watchId: n.id,
              path: n.path,
              documentType: o.type,
              hasBytes: !!o.bytes,
            }),
              l({ type: 'fileChanged', watchId: n.id, documentData: o });
          });
        P.set(n.id, r),
          c('info', 'File watch started successfully', {
            path: n.path,
            watchId: n.id,
            totalWatchers: P.size,
          }),
          l({ type: 'watchFile', id: n.id, success: !0 });
      } catch (e) {
        c('error', 'Failed to start file watch', {
          path: n.path,
          error: e instanceof Error ? e.message : String(e),
        }),
          l({
            type: 'watchFile',
            id: n.id,
            success: !1,
            error: e instanceof Error ? e.message : String(e),
          });
      }
      break;
    case 'unwatchFile':
      c('info', 'Stopping file watch', { watchId: n.id });
      try {
        const e = P.get(n.id);
        e
          ? (c('info', 'Found watcher, stopping it', { watchId: n.id }),
            e.stop(),
            P.delete(n.id),
            c('info', 'File watch stopped successfully', {
              watchId: n.id,
              remainingWatchers: P.size,
            }))
          : c('warn', 'No watcher found for ID', { watchId: n.id }),
          l({ type: 'unwatchFile', id: n.id, success: !0 });
      } catch (e) {
        c('error', 'Failed to stop file watch', {
          watchId: n.id,
          error: e instanceof Error ? e.message : String(e),
        }),
          l({
            type: 'unwatchFile',
            id: n.id,
            success: !1,
            error: e instanceof Error ? e.message : String(e),
          });
      }
      break;
    case 'watchDirectory':
      c('info', 'Starting directory watch', { path: n.path, id: n.id });
      try {
        const { tonk: e } = T(),
          r = await e.watchDirectory(n.path, o => {
            c('info', 'Directory change detected', {
              watchId: n.id,
              path: n.path,
            }),
              l({
                type: 'directoryChanged',
                watchId: n.id,
                path: n.path,
                changeData: o,
              });
          });
        P.set(n.id, r),
          c('info', 'Directory watch started successfully', {
            path: n.path,
            watchId: n.id,
            totalWatchers: P.size,
          }),
          l({ type: 'watchDirectory', id: n.id, success: !0 });
      } catch (e) {
        c('error', 'Failed to start directory watch', {
          path: n.path,
          error: e instanceof Error ? e.message : String(e),
        }),
          l({
            type: 'watchDirectory',
            id: n.id,
            success: !1,
            error: e instanceof Error ? e.message : String(e),
          });
      }
      break;
    case 'unwatchDirectory':
      c('info', 'Stopping directory watch', { watchId: n.id });
      try {
        const e = P.get(n.id);
        e
          ? (c('info', 'Found directory watcher, stopping it', {
              watchId: n.id,
            }),
            e.stop(),
            P.delete(n.id),
            c('info', 'Directory watch stopped successfully', {
              watchId: n.id,
              remainingWatchers: P.size,
            }))
          : c('warn', 'No directory watcher found for ID', { watchId: n.id }),
          l({ type: 'unwatchDirectory', id: n.id, success: !0 });
      } catch (e) {
        c('error', 'Failed to stop directory watch', {
          watchId: n.id,
          error: e instanceof Error ? e.message : String(e),
        }),
          l({
            type: 'unwatchDirectory',
            id: n.id,
            success: !1,
            error: e instanceof Error ? e.message : String(e),
          });
      }
      break;
    case 'toBytes':
      c('info', 'Converting tonk to bytes', { id: n.id });
      try {
        const { tonk: e, manifest: r } = T(),
          o = await e.toBytes(),
          s = r.rootId;
        c('info', 'Tonk converted to bytes successfully', {
          id: n.id,
          byteLength: o.length,
          rootId: s,
          manifestKeys: Object.keys(r),
        }),
          l({ type: 'toBytes', id: n.id, success: !0, data: o, rootId: s });
      } catch (e) {
        c('error', 'Failed to convert tonk to bytes', {
          id: n.id,
          error: e instanceof Error ? e.message : String(e),
        }),
          l({
            type: 'toBytes',
            id: n.id,
            success: !1,
            error: e instanceof Error ? e.message : String(e),
          });
      }
      break;
    case 'forkToBytes':
      c('info', 'Forking tonk to bytes', { id: n.id });
      try {
        const { tonk: e } = T(),
          r = await e.forkToBytes(),
          s = await (await Y.fromBytes(r)).getManifest(),
          a = s.rootId;
        c('info', 'Tonk forked to bytes successfully', {
          id: n.id,
          byteLength: r.length,
          rootId: a,
          manifestKeys: Object.keys(s),
        }),
          l({ type: 'forkToBytes', id: n.id, success: !0, data: r, rootId: a });
      } catch (e) {
        c('error', 'Failed to fork tonk to bytes', {
          id: n.id,
          error: e instanceof Error ? e.message : String(e),
        }),
          l({
            type: 'forkToBytes',
            id: n.id,
            success: !1,
            error: e instanceof Error ? e.message : String(e),
          });
      }
      break;
    case 'loadBundle':
      c('info', 'Loading new bundle', {
        id: n.id,
        byteLength: n.bundleBytes.byteLength,
      });
      try {
        if (y.status === 'uninitialized') {
          c('info', 'WASM not initialized, initializing now');
          const R = `http://localhost:8081/tonk_core_bg.wasm?t=${Date.now()}`;
          await pt({ wasmPath: R }),
            c('info', 'WASM initialization completed for bundle loading');
        }
        const e = new Uint8Array(n.bundleBytes);
        c('info', 'Creating bundle from bytes', { byteLength: e.length });
        const o = await (await Y.fromBytes(e)).getManifest();
        c('info', 'Bundle and manifest created successfully'),
          c('info', 'Creating new TonkCore from bundle bytes');
        const s = await B.fromBytes(e);
        c('info', 'New TonkCore created successfully');
        const _ = new URLSearchParams(self.location.search).get('bundle');
        let f = 'http://localhost:8081'.replace(/^http/, 'ws');
        if (_)
          try {
            const b = atob(_),
              R = JSON.parse(b);
            R.wsUrl && (f = R.wsUrl);
          } catch (b) {
            c('warn', 'Could not parse bundle config for wsUrl', {
              error: b instanceof Error ? b.message : String(b),
            });
          }
        c('info', 'Connecting new tonk to websocket', { wsUrl: f }),
          await s.connectWebsocket(f),
          c('info', 'Websocket connection established'),
          c('info', 'Waiting for initial data sync');
        let S = 0;
        const E = 20;
        for (; S < E; )
          try {
            await s.listDirectory('/app'),
              c('info', 'Initial data sync confirmed');
            break;
          } catch (b) {
            S++,
              S >= E
                ? c('warn', 'Initial sync timeout, proceeding anyway', {
                    attempts: S,
                    error: b instanceof Error ? b.message : String(b),
                  })
                : await new Promise(R => setTimeout(R, 500));
          }
        (y = { status: 'ready', tonk: s, manifest: o }),
          c('info', 'Tonk state updated with new instance'),
          l({ type: 'loadBundle', id: n.id, success: !0 });
      } catch (e) {
        c('error', 'Failed to load bundle', {
          id: n.id,
          error: e instanceof Error ? e.message : String(e),
        }),
          l({
            type: 'loadBundle',
            id: n.id,
            success: !1,
            error: e instanceof Error ? e.message : String(e),
          });
      }
      break;
    case 'getServerUrl':
      c('info', 'Getting server URL', { id: n.id }),
        l({
          type: 'getServerUrl',
          id: n.id,
          success: !0,
          data: 'http://localhost:8081',
        });
      break;
    case 'initializeFromUrl':
      c('info', 'Initializing from URL', {
        id: n.id,
        manifestUrl: n.manifestUrl,
        wasmUrl: n.wasmUrl,
      });
      try {
        const e = n.wasmUrl || 'http://localhost:8081/tonk_core_bg.wasm',
          r = n.manifestUrl || 'http://localhost:8081/.manifest.tonk',
          o = n.wsUrl || 'http://localhost:8081'.replace(/^http/, 'ws');
        c('info', 'Fetching WASM from URL', { wasmUrl: e });
        const s = `${e}?t=${Date.now()}`;
        await pt({ wasmPath: s }),
          c('info', 'WASM initialization completed'),
          c('info', 'Fetching manifest from URL', { manifestUrl: r });
        const a = await fetch(r),
          _ = new Uint8Array(await a.arrayBuffer());
        c('info', 'Manifest bytes loaded', { byteLength: _.length });
        const S = await (await Y.fromBytes(_)).getManifest();
        c('info', 'Bundle and manifest created successfully'),
          c('info', 'Creating TonkCore from manifest bytes');
        const E = await B.fromBytes(_);
        c('info', 'TonkCore created successfully'),
          c('info', 'Connecting to websocket', { wsUrl: o }),
          await E.connectWebsocket(o),
          c('info', 'Websocket connection established'),
          c('info', 'Waiting for initial data sync');
        let b = 0;
        const R = 20;
        for (; b < R; )
          try {
            await E.listDirectory('/app'),
              c('info', 'Initial data sync confirmed');
            break;
          } catch (q) {
            b++,
              b >= R
                ? c('warn', 'Initial sync timeout, proceeding anyway', {
                    attempts: b,
                    error: q instanceof Error ? q.message : String(q),
                  })
                : await new Promise(It => setTimeout(It, 500));
          }
        (y = { status: 'ready', tonk: E, manifest: S }),
          c('info', 'Tonk state updated to ready from URL'),
          l({ type: 'initializeFromUrl', id: n.id, success: !0 });
      } catch (e) {
        c('error', 'Failed to initialize from URL', {
          id: n.id,
          error: e instanceof Error ? e.message : String(e),
        }),
          (y = { status: 'failed', error: e }),
          l({
            type: 'initializeFromUrl',
            id: n.id,
            success: !1,
            error: e instanceof Error ? e.message : String(e),
          });
      }
      break;
  }
}
self.addEventListener('message', async n => {
  c('info', 'Raw message event received', {
    eventType: n.type,
    messageType: n.data?.type,
    hasData: !!n.data,
  });
  try {
    await Zt(n.data);
  } catch (t) {
    c('error', 'Error handling message', {
      error: t instanceof Error ? t.message : String(t),
    });
  }
});
c('info', 'VFS Service Worker started', { debugLogging: Et });
