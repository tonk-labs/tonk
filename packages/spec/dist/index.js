class Et extends Error {
  constructor(e, n, r) {
    super(e), this.name = this.constructor.name, this.code = n, this.context = r, Error.captureStackTrace && Error.captureStackTrace(this, this.constructor);
  }
}
class gt extends Et {
  constructor(e, n) {
    super(e, "BUNDLE_PARSE_ERROR", n);
  }
  static invalidZipFile(e) {
    return new gt(`Invalid ZIP file: ${e.message}`, {
      originalError: e.message
    });
  }
  static missingManifest() {
    return new gt("Missing manifest.json file in ZIP archive", {
      expectedFile: "manifest.json"
    });
  }
  static invalidManifestJson(e) {
    return new gt(
      `Failed to parse manifest.json: ${e.message}`,
      { originalError: e.message, file: "manifest.json" }
    );
  }
  static zipLoadFailed(e) {
    return new gt(
      `Failed to load ZIP archive: ${e.message}`,
      { originalError: e.message }
    );
  }
}
class bt extends Et {
  constructor(e, n) {
    super(e, "BUNDLE_VALIDATION_ERROR", n);
  }
  static missingRequiredField(e) {
    return new bt(`Missing required field: ${e}`, {
      field: e
    });
  }
  static duplicateFilePath(e) {
    return new bt(`Duplicate file path: ${e}`, { path: e });
  }
  static manifestFileInconsistency(e, n) {
    const r = n ? `File ${e} exists in ZIP but not in manifest` : `File ${e} listed in manifest but not found in ZIP`;
    return new bt(r, {
      path: e,
      inZip: n,
      inManifest: !n
    });
  }
  static zodSchemaError(e, n) {
    return new bt(
      `Schema validation failed: ${e.message || "Invalid data structure"}`,
      { zodError: e.toString(), ...n }
    );
  }
  static invalidEntrypointPath(e, n) {
    return new bt(
      `Entrypoint "${e}" references non-existent file: ${n}`,
      { entrypoint: e, path: n }
    );
  }
  static invalidContentType(e, n) {
    return new bt(
      `Invalid content type for ${e}: "${n}" is not a valid MIME type`,
      { path: e, contentType: n }
    );
  }
}
class Mt extends Et {
  constructor(e) {
    super(`File not found: ${e}`, "FILE_NOT_FOUND", { path: e });
  }
}
class Yo extends Et {
  constructor(e) {
    super(`Entrypoint not found: ${e}`, "ENTRYPOINT_NOT_FOUND", {
      entrypoint: e
    });
  }
}
class ne extends Et {
  constructor(e, n) {
    super(e, "ZIP_OPERATION_ERROR", n);
  }
  static fileNotFoundInZip(e) {
    return new ne(`File not found in ZIP archive: ${e}`, {
      path: e
    });
  }
  static zipGenerationFailed(e) {
    return new ne(
      `Failed to generate ZIP archive: ${e.message}`,
      { originalError: e.message }
    );
  }
  static fileExtractionFailed(e, n) {
    return new ne(
      `Failed to extract file ${e} from ZIP: ${n.message}`,
      { path: e, originalError: n.message }
    );
  }
}
class Xe extends Et {
  constructor(e, n) {
    super(e, "BUNDLE_SIZE_ERROR", n);
  }
  static exceedsMaxSize(e, n) {
    return new Xe(
      `Bundle size ${e} bytes exceeds maximum allowed size ${n} bytes`,
      { actualSize: e, maxSize: n }
    );
  }
}
class Ko extends Et {
  constructor(e, n) {
    super(
      `Unsupported bundle format version: ${e}. Supported versions: ${n.join(", ")}`,
      "UNSUPPORTED_VERSION",
      { version: e, supportedVersions: n }
    );
  }
}
class qe extends Et {
  constructor(e, n) {
    super(e, "SCHEMA_VALIDATION_ERROR", n);
  }
  static fromZodError(e) {
    const n = e.issues || [], r = n[0], i = r?.message || "Schema validation failed", s = r?.path?.join(".") || "unknown";
    return new qe(
      `Schema validation failed at ${s}: ${i}`,
      {
        zodError: e.toString(),
        path: s,
        issues: n.length,
        firstIssue: r?.message
      }
    );
  }
}
class Qe extends Et {
  constructor(e, n) {
    super(e, "CIRCULAR_REFERENCE_ERROR", n);
  }
  static entrypointCycle(e) {
    return new Qe(
      `Circular entrypoint references detected: ${e.join(", ")}`,
      { cycles: e, count: e.length }
    );
  }
}
class tn extends Et {
  constructor(e, n) {
    super(e, "VALIDATION_CONTEXT_ERROR", n);
  }
  static invalidRule(e, n) {
    return new tn(
      `Custom validation rule "${e}" failed: ${n.message}`,
      {
        ruleId: e,
        originalError: n.message,
        stack: n.stack
      }
    );
  }
}
class Jo extends Et {
  constructor(e, n, r, i) {
    super(e, n, r), this.suggestions = i?.suggestions, this.severity = i?.severity || "error", this.recoverable = i?.recoverable || !1;
  }
  /**
   * Get a detailed error report including context and suggestions
   */
  getDetailedReport() {
    const e = [];
    if (e.push(`${this.severity.toUpperCase()}: ${this.message}`), e.push(`Code: ${this.code}`), this.context && Object.keys(this.context).length > 0) {
      e.push("Context:");
      for (const [n, r] of Object.entries(this.context))
        e.push(`  ${n}: ${JSON.stringify(r)}`);
    }
    if (this.suggestions && this.suggestions.length > 0) {
      e.push("Suggestions:");
      for (const n of this.suggestions)
        e.push(`  • ${n}`);
    }
    return this.stack && (e.push(`
Stack trace:`), e.push(this.stack)), e.join(`
`);
  }
}
var mt = /* @__PURE__ */ ((t) => (t.ERROR = "error", t.WARNING = "warning", t.INFO = "info", t))(mt || {});
const vt = {
  REQUIRED_FIELDS: "required-fields",
  UNIQUE_FILE_PATHS: "unique-file-paths",
  VALID_ENTRYPOINTS: "valid-entrypoints",
  VALID_MIME_TYPES: "valid-mime-types",
  BUNDLE_SIZE_LIMIT: "bundle-size-limit",
  FILE_COUNT_LIMIT: "file-count-limit",
  VALID_VERSION: "valid-version",
  MANIFEST_ZIP_CONSISTENCY: "manifest-zip-consistency",
  ZOD_SCHEMA_VALIDATION: "zod-schema-validation"
};
class Tt {
  constructor() {
    this.messages = [];
  }
  /**
   * Add an error message
   */
  addError(e, n, r, i) {
    return this.messages.push({
      severity: "error",
      message: e,
      code: n,
      context: r,
      filePath: i
    }), this;
  }
  /**
   * Add a warning message
   */
  addWarning(e, n, r, i) {
    return this.messages.push({
      severity: "warning",
      message: e,
      code: n,
      context: r,
      filePath: i
    }), this;
  }
  /**
   * Add an info message
   */
  addInfo(e, n, r, i) {
    return this.messages.push({
      severity: "info",
      message: e,
      code: n,
      context: r,
      filePath: i
    }), this;
  }
  /**
   * Build the final validation result
   */
  build() {
    const e = this.messages.filter(
      (i) => i.severity === "error"
      /* ERROR */
    ), n = this.messages.filter(
      (i) => i.severity === "warning"
      /* WARNING */
    ), r = this.messages.filter(
      (i) => i.severity === "info"
      /* INFO */
    );
    return {
      valid: e.length === 0,
      messages: [...this.messages],
      errors: e,
      warnings: n,
      info: r
    };
  }
  /**
   * Check if there are any errors
   */
  hasErrors() {
    return this.messages.some(
      (e) => e.severity === "error"
      /* ERROR */
    );
  }
  /**
   * Get the number of messages by severity
   */
  getMessageCount(e) {
    return e ? this.messages.filter((n) => n.severity === e).length : this.messages.length;
  }
  /**
   * Add validation messages from a Zod error with enhanced context
   * This method helps integrate Zod validation errors with detailed error reporting
   */
  addZodError(e, n = "Schema validation failed", r = !0) {
    if (e && e.issues && Array.isArray(e.issues))
      for (const i of e.issues) {
        const s = i.path?.join(".") || "unknown", o = `${n} at ${s}: ${i.message}`;
        this.addError(o, vt.ZOD_SCHEMA_VALIDATION, {
          zodError: i,
          path: s,
          received: i.received,
          expected: i.expected,
          code: i.code,
          stack: r && e.stack ? e.stack : void 0,
          timestamp: (/* @__PURE__ */ new Date()).toISOString()
        });
      }
    else
      this.addError(n, vt.ZOD_SCHEMA_VALIDATION, {
        zodError: e?.toString() || "Unknown error",
        stack: r && e?.stack ? e.stack : void 0,
        timestamp: (/* @__PURE__ */ new Date()).toISOString()
      });
    return this;
  }
  /**
   * Add a validation message with enhanced error context and suggestions
   */
  addEnhancedMessage(e, n, r, i, s, o) {
    const a = {
      ...i,
      timestamp: (/* @__PURE__ */ new Date()).toISOString(),
      severity: e
    };
    return this.messages.push({
      severity: e,
      message: n,
      code: r,
      context: a,
      filePath: s,
      suggestion: o
    }), this;
  }
  /**
   * Add an error with stack trace preservation
   */
  addErrorWithStack(e, n, r, i, s) {
    const o = {
      ...i,
      stack: r?.stack,
      errorName: r?.name,
      timestamp: (/* @__PURE__ */ new Date()).toISOString()
    };
    return r?.stack && (o.originalMessage = r.message), this.addError(e, n, o, s), this;
  }
}
class ve {
  /**
   * Get the bundle as a Buffer (Node.js only)
   * @returns Buffer containing the complete bundle
   */
  async toBuffer() {
    if (typeof Buffer > "u")
      throw new Error("Buffer is not available in this environment");
    const e = await this.toArrayBuffer();
    return Buffer.from(e);
  }
  // Static Factory Methods
  /**
   * Create an empty bundle
   * @param options - Bundle creation options
   * @returns New empty bundle
   */
  static createEmpty(e) {
    throw new Error("createEmpty must be implemented by concrete Bundle class");
  }
  /**
   * Create a bundle from a collection of files
   * @param files - Map of file paths to file data
   * @param options - Bundle creation options
   * @returns New bundle containing the specified files
   *
   */
  static fromFiles(e, n) {
    throw new Error("fromFiles must be implemented by concrete Bundle class");
  }
  /**
   * Parse a bundle from binary data
   * @param data - Binary bundle data
   * @returns Parsed bundle instance
   * @throws {BundleParseError} If the data cannot be parsed
   */
  static parse(e) {
    throw new Error("parse must be implemented by concrete Bundle class");
  }
  /**
   * Parse a bundle from a Buffer (Node.js only)
   * @param buffer - Buffer containing bundle data
   * @returns Parsed bundle instance
   */
  static async fromBuffer(e) {
    const n = new ArrayBuffer(e.byteLength);
    return new Uint8Array(n).set(new Uint8Array(e)), await ve.parse(n);
  }
}
var Jt = typeof globalThis < "u" ? globalThis : typeof window < "u" ? window : typeof global < "u" ? global : typeof self < "u" ? self : {};
function Nn(t) {
  return t && t.__esModule && Object.prototype.hasOwnProperty.call(t, "default") ? t.default : t;
}
function Xt(t) {
  throw new Error('Could not dynamically require "' + t + '". Please configure the dynamicRequireTargets or/and ignoreDynamicRequires option of @rollup/plugin-commonjs appropriately for this require call to work.');
}
var en = { exports: {} };
/*!

JSZip v3.10.1 - A JavaScript class for generating and reading zip files
<http://stuartk.com/jszip>

(c) 2009-2016 Stuart Knightley <stuart [at] stuartk.com>
Dual licenced under the MIT license or GPLv3. See https://raw.github.com/Stuk/jszip/main/LICENSE.markdown.

JSZip uses the library pako released under the MIT license :
https://github.com/nodeca/pako/blob/main/LICENSE
*/
(function(t, e) {
  (function(n) {
    t.exports = n();
  })(function() {
    return function n(r, i, s) {
      function o(m, k) {
        if (!i[m]) {
          if (!r[m]) {
            var _ = typeof Xt == "function" && Xt;
            if (!k && _) return _(m, !0);
            if (a) return a(m, !0);
            var y = new Error("Cannot find module '" + m + "'");
            throw y.code = "MODULE_NOT_FOUND", y;
          }
          var f = i[m] = { exports: {} };
          r[m][0].call(f.exports, function(w) {
            var h = r[m][1][w];
            return o(h || w);
          }, f, f.exports, n, r, i, s);
        }
        return i[m].exports;
      }
      for (var a = typeof Xt == "function" && Xt, l = 0; l < s.length; l++) o(s[l]);
      return o;
    }({ 1: [function(n, r, i) {
      var s = n("./utils"), o = n("./support"), a = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/=";
      i.encode = function(l) {
        for (var m, k, _, y, f, w, h, p = [], d = 0, v = l.length, z = v, I = s.getTypeOf(l) !== "string"; d < l.length; ) z = v - d, _ = I ? (m = l[d++], k = d < v ? l[d++] : 0, d < v ? l[d++] : 0) : (m = l.charCodeAt(d++), k = d < v ? l.charCodeAt(d++) : 0, d < v ? l.charCodeAt(d++) : 0), y = m >> 2, f = (3 & m) << 4 | k >> 4, w = 1 < z ? (15 & k) << 2 | _ >> 6 : 64, h = 2 < z ? 63 & _ : 64, p.push(a.charAt(y) + a.charAt(f) + a.charAt(w) + a.charAt(h));
        return p.join("");
      }, i.decode = function(l) {
        var m, k, _, y, f, w, h = 0, p = 0, d = "data:";
        if (l.substr(0, d.length) === d) throw new Error("Invalid base64 input, it looks like a data url.");
        var v, z = 3 * (l = l.replace(/[^A-Za-z0-9+/=]/g, "")).length / 4;
        if (l.charAt(l.length - 1) === a.charAt(64) && z--, l.charAt(l.length - 2) === a.charAt(64) && z--, z % 1 != 0) throw new Error("Invalid base64 input, bad content length.");
        for (v = o.uint8array ? new Uint8Array(0 | z) : new Array(0 | z); h < l.length; ) m = a.indexOf(l.charAt(h++)) << 2 | (y = a.indexOf(l.charAt(h++))) >> 4, k = (15 & y) << 4 | (f = a.indexOf(l.charAt(h++))) >> 2, _ = (3 & f) << 6 | (w = a.indexOf(l.charAt(h++))), v[p++] = m, f !== 64 && (v[p++] = k), w !== 64 && (v[p++] = _);
        return v;
      };
    }, { "./support": 30, "./utils": 32 }], 2: [function(n, r, i) {
      var s = n("./external"), o = n("./stream/DataWorker"), a = n("./stream/Crc32Probe"), l = n("./stream/DataLengthProbe");
      function m(k, _, y, f, w) {
        this.compressedSize = k, this.uncompressedSize = _, this.crc32 = y, this.compression = f, this.compressedContent = w;
      }
      m.prototype = { getContentWorker: function() {
        var k = new o(s.Promise.resolve(this.compressedContent)).pipe(this.compression.uncompressWorker()).pipe(new l("data_length")), _ = this;
        return k.on("end", function() {
          if (this.streamInfo.data_length !== _.uncompressedSize) throw new Error("Bug : uncompressed data size mismatch");
        }), k;
      }, getCompressedWorker: function() {
        return new o(s.Promise.resolve(this.compressedContent)).withStreamInfo("compressedSize", this.compressedSize).withStreamInfo("uncompressedSize", this.uncompressedSize).withStreamInfo("crc32", this.crc32).withStreamInfo("compression", this.compression);
      } }, m.createWorkerFrom = function(k, _, y) {
        return k.pipe(new a()).pipe(new l("uncompressedSize")).pipe(_.compressWorker(y)).pipe(new l("compressedSize")).withStreamInfo("compression", _);
      }, r.exports = m;
    }, { "./external": 6, "./stream/Crc32Probe": 25, "./stream/DataLengthProbe": 26, "./stream/DataWorker": 27 }], 3: [function(n, r, i) {
      var s = n("./stream/GenericWorker");
      i.STORE = { magic: "\0\0", compressWorker: function() {
        return new s("STORE compression");
      }, uncompressWorker: function() {
        return new s("STORE decompression");
      } }, i.DEFLATE = n("./flate");
    }, { "./flate": 7, "./stream/GenericWorker": 28 }], 4: [function(n, r, i) {
      var s = n("./utils"), o = function() {
        for (var a, l = [], m = 0; m < 256; m++) {
          a = m;
          for (var k = 0; k < 8; k++) a = 1 & a ? 3988292384 ^ a >>> 1 : a >>> 1;
          l[m] = a;
        }
        return l;
      }();
      r.exports = function(a, l) {
        return a !== void 0 && a.length ? s.getTypeOf(a) !== "string" ? function(m, k, _, y) {
          var f = o, w = y + _;
          m ^= -1;
          for (var h = y; h < w; h++) m = m >>> 8 ^ f[255 & (m ^ k[h])];
          return -1 ^ m;
        }(0 | l, a, a.length, 0) : function(m, k, _, y) {
          var f = o, w = y + _;
          m ^= -1;
          for (var h = y; h < w; h++) m = m >>> 8 ^ f[255 & (m ^ k.charCodeAt(h))];
          return -1 ^ m;
        }(0 | l, a, a.length, 0) : 0;
      };
    }, { "./utils": 32 }], 5: [function(n, r, i) {
      i.base64 = !1, i.binary = !1, i.dir = !1, i.createFolders = !0, i.date = null, i.compression = null, i.compressionOptions = null, i.comment = null, i.unixPermissions = null, i.dosPermissions = null;
    }, {}], 6: [function(n, r, i) {
      var s = null;
      s = typeof Promise < "u" ? Promise : n("lie"), r.exports = { Promise: s };
    }, { lie: 37 }], 7: [function(n, r, i) {
      var s = typeof Uint8Array < "u" && typeof Uint16Array < "u" && typeof Uint32Array < "u", o = n("pako"), a = n("./utils"), l = n("./stream/GenericWorker"), m = s ? "uint8array" : "array";
      function k(_, y) {
        l.call(this, "FlateWorker/" + _), this._pako = null, this._pakoAction = _, this._pakoOptions = y, this.meta = {};
      }
      i.magic = "\b\0", a.inherits(k, l), k.prototype.processChunk = function(_) {
        this.meta = _.meta, this._pako === null && this._createPako(), this._pako.push(a.transformTo(m, _.data), !1);
      }, k.prototype.flush = function() {
        l.prototype.flush.call(this), this._pako === null && this._createPako(), this._pako.push([], !0);
      }, k.prototype.cleanUp = function() {
        l.prototype.cleanUp.call(this), this._pako = null;
      }, k.prototype._createPako = function() {
        this._pako = new o[this._pakoAction]({ raw: !0, level: this._pakoOptions.level || -1 });
        var _ = this;
        this._pako.onData = function(y) {
          _.push({ data: y, meta: _.meta });
        };
      }, i.compressWorker = function(_) {
        return new k("Deflate", _);
      }, i.uncompressWorker = function() {
        return new k("Inflate", {});
      };
    }, { "./stream/GenericWorker": 28, "./utils": 32, pako: 38 }], 8: [function(n, r, i) {
      function s(f, w) {
        var h, p = "";
        for (h = 0; h < w; h++) p += String.fromCharCode(255 & f), f >>>= 8;
        return p;
      }
      function o(f, w, h, p, d, v) {
        var z, I, S = f.file, D = f.compression, T = v !== m.utf8encode, L = a.transformTo("string", v(S.name)), $ = a.transformTo("string", m.utf8encode(S.name)), V = S.comment, Q = a.transformTo("string", v(V)), x = a.transformTo("string", m.utf8encode(V)), R = $.length !== S.name.length, c = x.length !== V.length, P = "", et = "", U = "", nt = S.dir, M = S.date, tt = { crc32: 0, compressedSize: 0, uncompressedSize: 0 };
        w && !h || (tt.crc32 = f.crc32, tt.compressedSize = f.compressedSize, tt.uncompressedSize = f.uncompressedSize);
        var C = 0;
        w && (C |= 8), T || !R && !c || (C |= 2048);
        var O = 0, q = 0;
        nt && (O |= 16), d === "UNIX" ? (q = 798, O |= function(H, ht) {
          var yt = H;
          return H || (yt = ht ? 16893 : 33204), (65535 & yt) << 16;
        }(S.unixPermissions, nt)) : (q = 20, O |= function(H) {
          return 63 & (H || 0);
        }(S.dosPermissions)), z = M.getUTCHours(), z <<= 6, z |= M.getUTCMinutes(), z <<= 5, z |= M.getUTCSeconds() / 2, I = M.getUTCFullYear() - 1980, I <<= 4, I |= M.getUTCMonth() + 1, I <<= 5, I |= M.getUTCDate(), R && (et = s(1, 1) + s(k(L), 4) + $, P += "up" + s(et.length, 2) + et), c && (U = s(1, 1) + s(k(Q), 4) + x, P += "uc" + s(U.length, 2) + U);
        var Y = "";
        return Y += `
\0`, Y += s(C, 2), Y += D.magic, Y += s(z, 2), Y += s(I, 2), Y += s(tt.crc32, 4), Y += s(tt.compressedSize, 4), Y += s(tt.uncompressedSize, 4), Y += s(L.length, 2), Y += s(P.length, 2), { fileRecord: _.LOCAL_FILE_HEADER + Y + L + P, dirRecord: _.CENTRAL_FILE_HEADER + s(q, 2) + Y + s(Q.length, 2) + "\0\0\0\0" + s(O, 4) + s(p, 4) + L + P + Q };
      }
      var a = n("../utils"), l = n("../stream/GenericWorker"), m = n("../utf8"), k = n("../crc32"), _ = n("../signature");
      function y(f, w, h, p) {
        l.call(this, "ZipFileWorker"), this.bytesWritten = 0, this.zipComment = w, this.zipPlatform = h, this.encodeFileName = p, this.streamFiles = f, this.accumulate = !1, this.contentBuffer = [], this.dirRecords = [], this.currentSourceOffset = 0, this.entriesCount = 0, this.currentFile = null, this._sources = [];
      }
      a.inherits(y, l), y.prototype.push = function(f) {
        var w = f.meta.percent || 0, h = this.entriesCount, p = this._sources.length;
        this.accumulate ? this.contentBuffer.push(f) : (this.bytesWritten += f.data.length, l.prototype.push.call(this, { data: f.data, meta: { currentFile: this.currentFile, percent: h ? (w + 100 * (h - p - 1)) / h : 100 } }));
      }, y.prototype.openedSource = function(f) {
        this.currentSourceOffset = this.bytesWritten, this.currentFile = f.file.name;
        var w = this.streamFiles && !f.file.dir;
        if (w) {
          var h = o(f, w, !1, this.currentSourceOffset, this.zipPlatform, this.encodeFileName);
          this.push({ data: h.fileRecord, meta: { percent: 0 } });
        } else this.accumulate = !0;
      }, y.prototype.closedSource = function(f) {
        this.accumulate = !1;
        var w = this.streamFiles && !f.file.dir, h = o(f, w, !0, this.currentSourceOffset, this.zipPlatform, this.encodeFileName);
        if (this.dirRecords.push(h.dirRecord), w) this.push({ data: function(p) {
          return _.DATA_DESCRIPTOR + s(p.crc32, 4) + s(p.compressedSize, 4) + s(p.uncompressedSize, 4);
        }(f), meta: { percent: 100 } });
        else for (this.push({ data: h.fileRecord, meta: { percent: 0 } }); this.contentBuffer.length; ) this.push(this.contentBuffer.shift());
        this.currentFile = null;
      }, y.prototype.flush = function() {
        for (var f = this.bytesWritten, w = 0; w < this.dirRecords.length; w++) this.push({ data: this.dirRecords[w], meta: { percent: 100 } });
        var h = this.bytesWritten - f, p = function(d, v, z, I, S) {
          var D = a.transformTo("string", S(I));
          return _.CENTRAL_DIRECTORY_END + "\0\0\0\0" + s(d, 2) + s(d, 2) + s(v, 4) + s(z, 4) + s(D.length, 2) + D;
        }(this.dirRecords.length, h, f, this.zipComment, this.encodeFileName);
        this.push({ data: p, meta: { percent: 100 } });
      }, y.prototype.prepareNextSource = function() {
        this.previous = this._sources.shift(), this.openedSource(this.previous.streamInfo), this.isPaused ? this.previous.pause() : this.previous.resume();
      }, y.prototype.registerPrevious = function(f) {
        this._sources.push(f);
        var w = this;
        return f.on("data", function(h) {
          w.processChunk(h);
        }), f.on("end", function() {
          w.closedSource(w.previous.streamInfo), w._sources.length ? w.prepareNextSource() : w.end();
        }), f.on("error", function(h) {
          w.error(h);
        }), this;
      }, y.prototype.resume = function() {
        return !!l.prototype.resume.call(this) && (!this.previous && this._sources.length ? (this.prepareNextSource(), !0) : this.previous || this._sources.length || this.generatedError ? void 0 : (this.end(), !0));
      }, y.prototype.error = function(f) {
        var w = this._sources;
        if (!l.prototype.error.call(this, f)) return !1;
        for (var h = 0; h < w.length; h++) try {
          w[h].error(f);
        } catch {
        }
        return !0;
      }, y.prototype.lock = function() {
        l.prototype.lock.call(this);
        for (var f = this._sources, w = 0; w < f.length; w++) f[w].lock();
      }, r.exports = y;
    }, { "../crc32": 4, "../signature": 23, "../stream/GenericWorker": 28, "../utf8": 31, "../utils": 32 }], 9: [function(n, r, i) {
      var s = n("../compressions"), o = n("./ZipFileWorker");
      i.generateWorker = function(a, l, m) {
        var k = new o(l.streamFiles, m, l.platform, l.encodeFileName), _ = 0;
        try {
          a.forEach(function(y, f) {
            _++;
            var w = function(v, z) {
              var I = v || z, S = s[I];
              if (!S) throw new Error(I + " is not a valid compression method !");
              return S;
            }(f.options.compression, l.compression), h = f.options.compressionOptions || l.compressionOptions || {}, p = f.dir, d = f.date;
            f._compressWorker(w, h).withStreamInfo("file", { name: y, dir: p, date: d, comment: f.comment || "", unixPermissions: f.unixPermissions, dosPermissions: f.dosPermissions }).pipe(k);
          }), k.entriesCount = _;
        } catch (y) {
          k.error(y);
        }
        return k;
      };
    }, { "../compressions": 3, "./ZipFileWorker": 8 }], 10: [function(n, r, i) {
      function s() {
        if (!(this instanceof s)) return new s();
        if (arguments.length) throw new Error("The constructor with parameters has been removed in JSZip 3.0, please check the upgrade guide.");
        this.files = /* @__PURE__ */ Object.create(null), this.comment = null, this.root = "", this.clone = function() {
          var o = new s();
          for (var a in this) typeof this[a] != "function" && (o[a] = this[a]);
          return o;
        };
      }
      (s.prototype = n("./object")).loadAsync = n("./load"), s.support = n("./support"), s.defaults = n("./defaults"), s.version = "3.10.1", s.loadAsync = function(o, a) {
        return new s().loadAsync(o, a);
      }, s.external = n("./external"), r.exports = s;
    }, { "./defaults": 5, "./external": 6, "./load": 11, "./object": 15, "./support": 30 }], 11: [function(n, r, i) {
      var s = n("./utils"), o = n("./external"), a = n("./utf8"), l = n("./zipEntries"), m = n("./stream/Crc32Probe"), k = n("./nodejsUtils");
      function _(y) {
        return new o.Promise(function(f, w) {
          var h = y.decompressed.getContentWorker().pipe(new m());
          h.on("error", function(p) {
            w(p);
          }).on("end", function() {
            h.streamInfo.crc32 !== y.decompressed.crc32 ? w(new Error("Corrupted zip : CRC32 mismatch")) : f();
          }).resume();
        });
      }
      r.exports = function(y, f) {
        var w = this;
        return f = s.extend(f || {}, { base64: !1, checkCRC32: !1, optimizedBinaryString: !1, createFolders: !1, decodeFileName: a.utf8decode }), k.isNode && k.isStream(y) ? o.Promise.reject(new Error("JSZip can't accept a stream when loading a zip file.")) : s.prepareContent("the loaded zip file", y, !0, f.optimizedBinaryString, f.base64).then(function(h) {
          var p = new l(f);
          return p.load(h), p;
        }).then(function(h) {
          var p = [o.Promise.resolve(h)], d = h.files;
          if (f.checkCRC32) for (var v = 0; v < d.length; v++) p.push(_(d[v]));
          return o.Promise.all(p);
        }).then(function(h) {
          for (var p = h.shift(), d = p.files, v = 0; v < d.length; v++) {
            var z = d[v], I = z.fileNameStr, S = s.resolve(z.fileNameStr);
            w.file(S, z.decompressed, { binary: !0, optimizedBinaryString: !0, date: z.date, dir: z.dir, comment: z.fileCommentStr.length ? z.fileCommentStr : null, unixPermissions: z.unixPermissions, dosPermissions: z.dosPermissions, createFolders: f.createFolders }), z.dir || (w.file(S).unsafeOriginalName = I);
          }
          return p.zipComment.length && (w.comment = p.zipComment), w;
        });
      };
    }, { "./external": 6, "./nodejsUtils": 14, "./stream/Crc32Probe": 25, "./utf8": 31, "./utils": 32, "./zipEntries": 33 }], 12: [function(n, r, i) {
      var s = n("../utils"), o = n("../stream/GenericWorker");
      function a(l, m) {
        o.call(this, "Nodejs stream input adapter for " + l), this._upstreamEnded = !1, this._bindStream(m);
      }
      s.inherits(a, o), a.prototype._bindStream = function(l) {
        var m = this;
        (this._stream = l).pause(), l.on("data", function(k) {
          m.push({ data: k, meta: { percent: 0 } });
        }).on("error", function(k) {
          m.isPaused ? this.generatedError = k : m.error(k);
        }).on("end", function() {
          m.isPaused ? m._upstreamEnded = !0 : m.end();
        });
      }, a.prototype.pause = function() {
        return !!o.prototype.pause.call(this) && (this._stream.pause(), !0);
      }, a.prototype.resume = function() {
        return !!o.prototype.resume.call(this) && (this._upstreamEnded ? this.end() : this._stream.resume(), !0);
      }, r.exports = a;
    }, { "../stream/GenericWorker": 28, "../utils": 32 }], 13: [function(n, r, i) {
      var s = n("readable-stream").Readable;
      function o(a, l, m) {
        s.call(this, l), this._helper = a;
        var k = this;
        a.on("data", function(_, y) {
          k.push(_) || k._helper.pause(), m && m(y);
        }).on("error", function(_) {
          k.emit("error", _);
        }).on("end", function() {
          k.push(null);
        });
      }
      n("../utils").inherits(o, s), o.prototype._read = function() {
        this._helper.resume();
      }, r.exports = o;
    }, { "../utils": 32, "readable-stream": 16 }], 14: [function(n, r, i) {
      r.exports = { isNode: typeof Buffer < "u", newBufferFrom: function(s, o) {
        if (Buffer.from && Buffer.from !== Uint8Array.from) return Buffer.from(s, o);
        if (typeof s == "number") throw new Error('The "data" argument must not be a number');
        return new Buffer(s, o);
      }, allocBuffer: function(s) {
        if (Buffer.alloc) return Buffer.alloc(s);
        var o = new Buffer(s);
        return o.fill(0), o;
      }, isBuffer: function(s) {
        return Buffer.isBuffer(s);
      }, isStream: function(s) {
        return s && typeof s.on == "function" && typeof s.pause == "function" && typeof s.resume == "function";
      } };
    }, {}], 15: [function(n, r, i) {
      function s(S, D, T) {
        var L, $ = a.getTypeOf(D), V = a.extend(T || {}, k);
        V.date = V.date || /* @__PURE__ */ new Date(), V.compression !== null && (V.compression = V.compression.toUpperCase()), typeof V.unixPermissions == "string" && (V.unixPermissions = parseInt(V.unixPermissions, 8)), V.unixPermissions && 16384 & V.unixPermissions && (V.dir = !0), V.dosPermissions && 16 & V.dosPermissions && (V.dir = !0), V.dir && (S = d(S)), V.createFolders && (L = p(S)) && v.call(this, L, !0);
        var Q = $ === "string" && V.binary === !1 && V.base64 === !1;
        T && T.binary !== void 0 || (V.binary = !Q), (D instanceof _ && D.uncompressedSize === 0 || V.dir || !D || D.length === 0) && (V.base64 = !1, V.binary = !0, D = "", V.compression = "STORE", $ = "string");
        var x = null;
        x = D instanceof _ || D instanceof l ? D : w.isNode && w.isStream(D) ? new h(S, D) : a.prepareContent(S, D, V.binary, V.optimizedBinaryString, V.base64);
        var R = new y(S, x, V);
        this.files[S] = R;
      }
      var o = n("./utf8"), a = n("./utils"), l = n("./stream/GenericWorker"), m = n("./stream/StreamHelper"), k = n("./defaults"), _ = n("./compressedObject"), y = n("./zipObject"), f = n("./generate"), w = n("./nodejsUtils"), h = n("./nodejs/NodejsStreamInputAdapter"), p = function(S) {
        S.slice(-1) === "/" && (S = S.substring(0, S.length - 1));
        var D = S.lastIndexOf("/");
        return 0 < D ? S.substring(0, D) : "";
      }, d = function(S) {
        return S.slice(-1) !== "/" && (S += "/"), S;
      }, v = function(S, D) {
        return D = D !== void 0 ? D : k.createFolders, S = d(S), this.files[S] || s.call(this, S, null, { dir: !0, createFolders: D }), this.files[S];
      };
      function z(S) {
        return Object.prototype.toString.call(S) === "[object RegExp]";
      }
      var I = { load: function() {
        throw new Error("This method has been removed in JSZip 3.0, please check the upgrade guide.");
      }, forEach: function(S) {
        var D, T, L;
        for (D in this.files) L = this.files[D], (T = D.slice(this.root.length, D.length)) && D.slice(0, this.root.length) === this.root && S(T, L);
      }, filter: function(S) {
        var D = [];
        return this.forEach(function(T, L) {
          S(T, L) && D.push(L);
        }), D;
      }, file: function(S, D, T) {
        if (arguments.length !== 1) return S = this.root + S, s.call(this, S, D, T), this;
        if (z(S)) {
          var L = S;
          return this.filter(function(V, Q) {
            return !Q.dir && L.test(V);
          });
        }
        var $ = this.files[this.root + S];
        return $ && !$.dir ? $ : null;
      }, folder: function(S) {
        if (!S) return this;
        if (z(S)) return this.filter(function($, V) {
          return V.dir && S.test($);
        });
        var D = this.root + S, T = v.call(this, D), L = this.clone();
        return L.root = T.name, L;
      }, remove: function(S) {
        S = this.root + S;
        var D = this.files[S];
        if (D || (S.slice(-1) !== "/" && (S += "/"), D = this.files[S]), D && !D.dir) delete this.files[S];
        else for (var T = this.filter(function($, V) {
          return V.name.slice(0, S.length) === S;
        }), L = 0; L < T.length; L++) delete this.files[T[L].name];
        return this;
      }, generate: function() {
        throw new Error("This method has been removed in JSZip 3.0, please check the upgrade guide.");
      }, generateInternalStream: function(S) {
        var D, T = {};
        try {
          if ((T = a.extend(S || {}, { streamFiles: !1, compression: "STORE", compressionOptions: null, type: "", platform: "DOS", comment: null, mimeType: "application/zip", encodeFileName: o.utf8encode })).type = T.type.toLowerCase(), T.compression = T.compression.toUpperCase(), T.type === "binarystring" && (T.type = "string"), !T.type) throw new Error("No output type specified.");
          a.checkSupport(T.type), T.platform !== "darwin" && T.platform !== "freebsd" && T.platform !== "linux" && T.platform !== "sunos" || (T.platform = "UNIX"), T.platform === "win32" && (T.platform = "DOS");
          var L = T.comment || this.comment || "";
          D = f.generateWorker(this, T, L);
        } catch ($) {
          (D = new l("error")).error($);
        }
        return new m(D, T.type || "string", T.mimeType);
      }, generateAsync: function(S, D) {
        return this.generateInternalStream(S).accumulate(D);
      }, generateNodeStream: function(S, D) {
        return (S = S || {}).type || (S.type = "nodebuffer"), this.generateInternalStream(S).toNodejsStream(D);
      } };
      r.exports = I;
    }, { "./compressedObject": 2, "./defaults": 5, "./generate": 9, "./nodejs/NodejsStreamInputAdapter": 12, "./nodejsUtils": 14, "./stream/GenericWorker": 28, "./stream/StreamHelper": 29, "./utf8": 31, "./utils": 32, "./zipObject": 35 }], 16: [function(n, r, i) {
      r.exports = n("stream");
    }, { stream: void 0 }], 17: [function(n, r, i) {
      var s = n("./DataReader");
      function o(a) {
        s.call(this, a);
        for (var l = 0; l < this.data.length; l++) a[l] = 255 & a[l];
      }
      n("../utils").inherits(o, s), o.prototype.byteAt = function(a) {
        return this.data[this.zero + a];
      }, o.prototype.lastIndexOfSignature = function(a) {
        for (var l = a.charCodeAt(0), m = a.charCodeAt(1), k = a.charCodeAt(2), _ = a.charCodeAt(3), y = this.length - 4; 0 <= y; --y) if (this.data[y] === l && this.data[y + 1] === m && this.data[y + 2] === k && this.data[y + 3] === _) return y - this.zero;
        return -1;
      }, o.prototype.readAndCheckSignature = function(a) {
        var l = a.charCodeAt(0), m = a.charCodeAt(1), k = a.charCodeAt(2), _ = a.charCodeAt(3), y = this.readData(4);
        return l === y[0] && m === y[1] && k === y[2] && _ === y[3];
      }, o.prototype.readData = function(a) {
        if (this.checkOffset(a), a === 0) return [];
        var l = this.data.slice(this.zero + this.index, this.zero + this.index + a);
        return this.index += a, l;
      }, r.exports = o;
    }, { "../utils": 32, "./DataReader": 18 }], 18: [function(n, r, i) {
      var s = n("../utils");
      function o(a) {
        this.data = a, this.length = a.length, this.index = 0, this.zero = 0;
      }
      o.prototype = { checkOffset: function(a) {
        this.checkIndex(this.index + a);
      }, checkIndex: function(a) {
        if (this.length < this.zero + a || a < 0) throw new Error("End of data reached (data length = " + this.length + ", asked index = " + a + "). Corrupted zip ?");
      }, setIndex: function(a) {
        this.checkIndex(a), this.index = a;
      }, skip: function(a) {
        this.setIndex(this.index + a);
      }, byteAt: function() {
      }, readInt: function(a) {
        var l, m = 0;
        for (this.checkOffset(a), l = this.index + a - 1; l >= this.index; l--) m = (m << 8) + this.byteAt(l);
        return this.index += a, m;
      }, readString: function(a) {
        return s.transformTo("string", this.readData(a));
      }, readData: function() {
      }, lastIndexOfSignature: function() {
      }, readAndCheckSignature: function() {
      }, readDate: function() {
        var a = this.readInt(4);
        return new Date(Date.UTC(1980 + (a >> 25 & 127), (a >> 21 & 15) - 1, a >> 16 & 31, a >> 11 & 31, a >> 5 & 63, (31 & a) << 1));
      } }, r.exports = o;
    }, { "../utils": 32 }], 19: [function(n, r, i) {
      var s = n("./Uint8ArrayReader");
      function o(a) {
        s.call(this, a);
      }
      n("../utils").inherits(o, s), o.prototype.readData = function(a) {
        this.checkOffset(a);
        var l = this.data.slice(this.zero + this.index, this.zero + this.index + a);
        return this.index += a, l;
      }, r.exports = o;
    }, { "../utils": 32, "./Uint8ArrayReader": 21 }], 20: [function(n, r, i) {
      var s = n("./DataReader");
      function o(a) {
        s.call(this, a);
      }
      n("../utils").inherits(o, s), o.prototype.byteAt = function(a) {
        return this.data.charCodeAt(this.zero + a);
      }, o.prototype.lastIndexOfSignature = function(a) {
        return this.data.lastIndexOf(a) - this.zero;
      }, o.prototype.readAndCheckSignature = function(a) {
        return a === this.readData(4);
      }, o.prototype.readData = function(a) {
        this.checkOffset(a);
        var l = this.data.slice(this.zero + this.index, this.zero + this.index + a);
        return this.index += a, l;
      }, r.exports = o;
    }, { "../utils": 32, "./DataReader": 18 }], 21: [function(n, r, i) {
      var s = n("./ArrayReader");
      function o(a) {
        s.call(this, a);
      }
      n("../utils").inherits(o, s), o.prototype.readData = function(a) {
        if (this.checkOffset(a), a === 0) return new Uint8Array(0);
        var l = this.data.subarray(this.zero + this.index, this.zero + this.index + a);
        return this.index += a, l;
      }, r.exports = o;
    }, { "../utils": 32, "./ArrayReader": 17 }], 22: [function(n, r, i) {
      var s = n("../utils"), o = n("../support"), a = n("./ArrayReader"), l = n("./StringReader"), m = n("./NodeBufferReader"), k = n("./Uint8ArrayReader");
      r.exports = function(_) {
        var y = s.getTypeOf(_);
        return s.checkSupport(y), y !== "string" || o.uint8array ? y === "nodebuffer" ? new m(_) : o.uint8array ? new k(s.transformTo("uint8array", _)) : new a(s.transformTo("array", _)) : new l(_);
      };
    }, { "../support": 30, "../utils": 32, "./ArrayReader": 17, "./NodeBufferReader": 19, "./StringReader": 20, "./Uint8ArrayReader": 21 }], 23: [function(n, r, i) {
      i.LOCAL_FILE_HEADER = "PK", i.CENTRAL_FILE_HEADER = "PK", i.CENTRAL_DIRECTORY_END = "PK", i.ZIP64_CENTRAL_DIRECTORY_LOCATOR = "PK\x07", i.ZIP64_CENTRAL_DIRECTORY_END = "PK", i.DATA_DESCRIPTOR = "PK\x07\b";
    }, {}], 24: [function(n, r, i) {
      var s = n("./GenericWorker"), o = n("../utils");
      function a(l) {
        s.call(this, "ConvertWorker to " + l), this.destType = l;
      }
      o.inherits(a, s), a.prototype.processChunk = function(l) {
        this.push({ data: o.transformTo(this.destType, l.data), meta: l.meta });
      }, r.exports = a;
    }, { "../utils": 32, "./GenericWorker": 28 }], 25: [function(n, r, i) {
      var s = n("./GenericWorker"), o = n("../crc32");
      function a() {
        s.call(this, "Crc32Probe"), this.withStreamInfo("crc32", 0);
      }
      n("../utils").inherits(a, s), a.prototype.processChunk = function(l) {
        this.streamInfo.crc32 = o(l.data, this.streamInfo.crc32 || 0), this.push(l);
      }, r.exports = a;
    }, { "../crc32": 4, "../utils": 32, "./GenericWorker": 28 }], 26: [function(n, r, i) {
      var s = n("../utils"), o = n("./GenericWorker");
      function a(l) {
        o.call(this, "DataLengthProbe for " + l), this.propName = l, this.withStreamInfo(l, 0);
      }
      s.inherits(a, o), a.prototype.processChunk = function(l) {
        if (l) {
          var m = this.streamInfo[this.propName] || 0;
          this.streamInfo[this.propName] = m + l.data.length;
        }
        o.prototype.processChunk.call(this, l);
      }, r.exports = a;
    }, { "../utils": 32, "./GenericWorker": 28 }], 27: [function(n, r, i) {
      var s = n("../utils"), o = n("./GenericWorker");
      function a(l) {
        o.call(this, "DataWorker");
        var m = this;
        this.dataIsReady = !1, this.index = 0, this.max = 0, this.data = null, this.type = "", this._tickScheduled = !1, l.then(function(k) {
          m.dataIsReady = !0, m.data = k, m.max = k && k.length || 0, m.type = s.getTypeOf(k), m.isPaused || m._tickAndRepeat();
        }, function(k) {
          m.error(k);
        });
      }
      s.inherits(a, o), a.prototype.cleanUp = function() {
        o.prototype.cleanUp.call(this), this.data = null;
      }, a.prototype.resume = function() {
        return !!o.prototype.resume.call(this) && (!this._tickScheduled && this.dataIsReady && (this._tickScheduled = !0, s.delay(this._tickAndRepeat, [], this)), !0);
      }, a.prototype._tickAndRepeat = function() {
        this._tickScheduled = !1, this.isPaused || this.isFinished || (this._tick(), this.isFinished || (s.delay(this._tickAndRepeat, [], this), this._tickScheduled = !0));
      }, a.prototype._tick = function() {
        if (this.isPaused || this.isFinished) return !1;
        var l = null, m = Math.min(this.max, this.index + 16384);
        if (this.index >= this.max) return this.end();
        switch (this.type) {
          case "string":
            l = this.data.substring(this.index, m);
            break;
          case "uint8array":
            l = this.data.subarray(this.index, m);
            break;
          case "array":
          case "nodebuffer":
            l = this.data.slice(this.index, m);
        }
        return this.index = m, this.push({ data: l, meta: { percent: this.max ? this.index / this.max * 100 : 0 } });
      }, r.exports = a;
    }, { "../utils": 32, "./GenericWorker": 28 }], 28: [function(n, r, i) {
      function s(o) {
        this.name = o || "default", this.streamInfo = {}, this.generatedError = null, this.extraStreamInfo = {}, this.isPaused = !0, this.isFinished = !1, this.isLocked = !1, this._listeners = { data: [], end: [], error: [] }, this.previous = null;
      }
      s.prototype = { push: function(o) {
        this.emit("data", o);
      }, end: function() {
        if (this.isFinished) return !1;
        this.flush();
        try {
          this.emit("end"), this.cleanUp(), this.isFinished = !0;
        } catch (o) {
          this.emit("error", o);
        }
        return !0;
      }, error: function(o) {
        return !this.isFinished && (this.isPaused ? this.generatedError = o : (this.isFinished = !0, this.emit("error", o), this.previous && this.previous.error(o), this.cleanUp()), !0);
      }, on: function(o, a) {
        return this._listeners[o].push(a), this;
      }, cleanUp: function() {
        this.streamInfo = this.generatedError = this.extraStreamInfo = null, this._listeners = [];
      }, emit: function(o, a) {
        if (this._listeners[o]) for (var l = 0; l < this._listeners[o].length; l++) this._listeners[o][l].call(this, a);
      }, pipe: function(o) {
        return o.registerPrevious(this);
      }, registerPrevious: function(o) {
        if (this.isLocked) throw new Error("The stream '" + this + "' has already been used.");
        this.streamInfo = o.streamInfo, this.mergeStreamInfo(), this.previous = o;
        var a = this;
        return o.on("data", function(l) {
          a.processChunk(l);
        }), o.on("end", function() {
          a.end();
        }), o.on("error", function(l) {
          a.error(l);
        }), this;
      }, pause: function() {
        return !this.isPaused && !this.isFinished && (this.isPaused = !0, this.previous && this.previous.pause(), !0);
      }, resume: function() {
        if (!this.isPaused || this.isFinished) return !1;
        var o = this.isPaused = !1;
        return this.generatedError && (this.error(this.generatedError), o = !0), this.previous && this.previous.resume(), !o;
      }, flush: function() {
      }, processChunk: function(o) {
        this.push(o);
      }, withStreamInfo: function(o, a) {
        return this.extraStreamInfo[o] = a, this.mergeStreamInfo(), this;
      }, mergeStreamInfo: function() {
        for (var o in this.extraStreamInfo) Object.prototype.hasOwnProperty.call(this.extraStreamInfo, o) && (this.streamInfo[o] = this.extraStreamInfo[o]);
      }, lock: function() {
        if (this.isLocked) throw new Error("The stream '" + this + "' has already been used.");
        this.isLocked = !0, this.previous && this.previous.lock();
      }, toString: function() {
        var o = "Worker " + this.name;
        return this.previous ? this.previous + " -> " + o : o;
      } }, r.exports = s;
    }, {}], 29: [function(n, r, i) {
      var s = n("../utils"), o = n("./ConvertWorker"), a = n("./GenericWorker"), l = n("../base64"), m = n("../support"), k = n("../external"), _ = null;
      if (m.nodestream) try {
        _ = n("../nodejs/NodejsStreamOutputAdapter");
      } catch {
      }
      function y(w, h) {
        return new k.Promise(function(p, d) {
          var v = [], z = w._internalType, I = w._outputType, S = w._mimeType;
          w.on("data", function(D, T) {
            v.push(D), h && h(T);
          }).on("error", function(D) {
            v = [], d(D);
          }).on("end", function() {
            try {
              var D = function(T, L, $) {
                switch (T) {
                  case "blob":
                    return s.newBlob(s.transformTo("arraybuffer", L), $);
                  case "base64":
                    return l.encode(L);
                  default:
                    return s.transformTo(T, L);
                }
              }(I, function(T, L) {
                var $, V = 0, Q = null, x = 0;
                for ($ = 0; $ < L.length; $++) x += L[$].length;
                switch (T) {
                  case "string":
                    return L.join("");
                  case "array":
                    return Array.prototype.concat.apply([], L);
                  case "uint8array":
                    for (Q = new Uint8Array(x), $ = 0; $ < L.length; $++) Q.set(L[$], V), V += L[$].length;
                    return Q;
                  case "nodebuffer":
                    return Buffer.concat(L);
                  default:
                    throw new Error("concat : unsupported type '" + T + "'");
                }
              }(z, v), S);
              p(D);
            } catch (T) {
              d(T);
            }
            v = [];
          }).resume();
        });
      }
      function f(w, h, p) {
        var d = h;
        switch (h) {
          case "blob":
          case "arraybuffer":
            d = "uint8array";
            break;
          case "base64":
            d = "string";
        }
        try {
          this._internalType = d, this._outputType = h, this._mimeType = p, s.checkSupport(d), this._worker = w.pipe(new o(d)), w.lock();
        } catch (v) {
          this._worker = new a("error"), this._worker.error(v);
        }
      }
      f.prototype = { accumulate: function(w) {
        return y(this, w);
      }, on: function(w, h) {
        var p = this;
        return w === "data" ? this._worker.on(w, function(d) {
          h.call(p, d.data, d.meta);
        }) : this._worker.on(w, function() {
          s.delay(h, arguments, p);
        }), this;
      }, resume: function() {
        return s.delay(this._worker.resume, [], this._worker), this;
      }, pause: function() {
        return this._worker.pause(), this;
      }, toNodejsStream: function(w) {
        if (s.checkSupport("nodestream"), this._outputType !== "nodebuffer") throw new Error(this._outputType + " is not supported by this method");
        return new _(this, { objectMode: this._outputType !== "nodebuffer" }, w);
      } }, r.exports = f;
    }, { "../base64": 1, "../external": 6, "../nodejs/NodejsStreamOutputAdapter": 13, "../support": 30, "../utils": 32, "./ConvertWorker": 24, "./GenericWorker": 28 }], 30: [function(n, r, i) {
      if (i.base64 = !0, i.array = !0, i.string = !0, i.arraybuffer = typeof ArrayBuffer < "u" && typeof Uint8Array < "u", i.nodebuffer = typeof Buffer < "u", i.uint8array = typeof Uint8Array < "u", typeof ArrayBuffer > "u") i.blob = !1;
      else {
        var s = new ArrayBuffer(0);
        try {
          i.blob = new Blob([s], { type: "application/zip" }).size === 0;
        } catch {
          try {
            var o = new (self.BlobBuilder || self.WebKitBlobBuilder || self.MozBlobBuilder || self.MSBlobBuilder)();
            o.append(s), i.blob = o.getBlob("application/zip").size === 0;
          } catch {
            i.blob = !1;
          }
        }
      }
      try {
        i.nodestream = !!n("readable-stream").Readable;
      } catch {
        i.nodestream = !1;
      }
    }, { "readable-stream": 16 }], 31: [function(n, r, i) {
      for (var s = n("./utils"), o = n("./support"), a = n("./nodejsUtils"), l = n("./stream/GenericWorker"), m = new Array(256), k = 0; k < 256; k++) m[k] = 252 <= k ? 6 : 248 <= k ? 5 : 240 <= k ? 4 : 224 <= k ? 3 : 192 <= k ? 2 : 1;
      m[254] = m[254] = 1;
      function _() {
        l.call(this, "utf-8 decode"), this.leftOver = null;
      }
      function y() {
        l.call(this, "utf-8 encode");
      }
      i.utf8encode = function(f) {
        return o.nodebuffer ? a.newBufferFrom(f, "utf-8") : function(w) {
          var h, p, d, v, z, I = w.length, S = 0;
          for (v = 0; v < I; v++) (64512 & (p = w.charCodeAt(v))) == 55296 && v + 1 < I && (64512 & (d = w.charCodeAt(v + 1))) == 56320 && (p = 65536 + (p - 55296 << 10) + (d - 56320), v++), S += p < 128 ? 1 : p < 2048 ? 2 : p < 65536 ? 3 : 4;
          for (h = o.uint8array ? new Uint8Array(S) : new Array(S), v = z = 0; z < S; v++) (64512 & (p = w.charCodeAt(v))) == 55296 && v + 1 < I && (64512 & (d = w.charCodeAt(v + 1))) == 56320 && (p = 65536 + (p - 55296 << 10) + (d - 56320), v++), p < 128 ? h[z++] = p : (p < 2048 ? h[z++] = 192 | p >>> 6 : (p < 65536 ? h[z++] = 224 | p >>> 12 : (h[z++] = 240 | p >>> 18, h[z++] = 128 | p >>> 12 & 63), h[z++] = 128 | p >>> 6 & 63), h[z++] = 128 | 63 & p);
          return h;
        }(f);
      }, i.utf8decode = function(f) {
        return o.nodebuffer ? s.transformTo("nodebuffer", f).toString("utf-8") : function(w) {
          var h, p, d, v, z = w.length, I = new Array(2 * z);
          for (h = p = 0; h < z; ) if ((d = w[h++]) < 128) I[p++] = d;
          else if (4 < (v = m[d])) I[p++] = 65533, h += v - 1;
          else {
            for (d &= v === 2 ? 31 : v === 3 ? 15 : 7; 1 < v && h < z; ) d = d << 6 | 63 & w[h++], v--;
            1 < v ? I[p++] = 65533 : d < 65536 ? I[p++] = d : (d -= 65536, I[p++] = 55296 | d >> 10 & 1023, I[p++] = 56320 | 1023 & d);
          }
          return I.length !== p && (I.subarray ? I = I.subarray(0, p) : I.length = p), s.applyFromCharCode(I);
        }(f = s.transformTo(o.uint8array ? "uint8array" : "array", f));
      }, s.inherits(_, l), _.prototype.processChunk = function(f) {
        var w = s.transformTo(o.uint8array ? "uint8array" : "array", f.data);
        if (this.leftOver && this.leftOver.length) {
          if (o.uint8array) {
            var h = w;
            (w = new Uint8Array(h.length + this.leftOver.length)).set(this.leftOver, 0), w.set(h, this.leftOver.length);
          } else w = this.leftOver.concat(w);
          this.leftOver = null;
        }
        var p = function(v, z) {
          var I;
          for ((z = z || v.length) > v.length && (z = v.length), I = z - 1; 0 <= I && (192 & v[I]) == 128; ) I--;
          return I < 0 || I === 0 ? z : I + m[v[I]] > z ? I : z;
        }(w), d = w;
        p !== w.length && (o.uint8array ? (d = w.subarray(0, p), this.leftOver = w.subarray(p, w.length)) : (d = w.slice(0, p), this.leftOver = w.slice(p, w.length))), this.push({ data: i.utf8decode(d), meta: f.meta });
      }, _.prototype.flush = function() {
        this.leftOver && this.leftOver.length && (this.push({ data: i.utf8decode(this.leftOver), meta: {} }), this.leftOver = null);
      }, i.Utf8DecodeWorker = _, s.inherits(y, l), y.prototype.processChunk = function(f) {
        this.push({ data: i.utf8encode(f.data), meta: f.meta });
      }, i.Utf8EncodeWorker = y;
    }, { "./nodejsUtils": 14, "./stream/GenericWorker": 28, "./support": 30, "./utils": 32 }], 32: [function(n, r, i) {
      var s = n("./support"), o = n("./base64"), a = n("./nodejsUtils"), l = n("./external");
      function m(h) {
        return h;
      }
      function k(h, p) {
        for (var d = 0; d < h.length; ++d) p[d] = 255 & h.charCodeAt(d);
        return p;
      }
      n("setimmediate"), i.newBlob = function(h, p) {
        i.checkSupport("blob");
        try {
          return new Blob([h], { type: p });
        } catch {
          try {
            var d = new (self.BlobBuilder || self.WebKitBlobBuilder || self.MozBlobBuilder || self.MSBlobBuilder)();
            return d.append(h), d.getBlob(p);
          } catch {
            throw new Error("Bug : can't construct the Blob.");
          }
        }
      };
      var _ = { stringifyByChunk: function(h, p, d) {
        var v = [], z = 0, I = h.length;
        if (I <= d) return String.fromCharCode.apply(null, h);
        for (; z < I; ) p === "array" || p === "nodebuffer" ? v.push(String.fromCharCode.apply(null, h.slice(z, Math.min(z + d, I)))) : v.push(String.fromCharCode.apply(null, h.subarray(z, Math.min(z + d, I)))), z += d;
        return v.join("");
      }, stringifyByChar: function(h) {
        for (var p = "", d = 0; d < h.length; d++) p += String.fromCharCode(h[d]);
        return p;
      }, applyCanBeUsed: { uint8array: function() {
        try {
          return s.uint8array && String.fromCharCode.apply(null, new Uint8Array(1)).length === 1;
        } catch {
          return !1;
        }
      }(), nodebuffer: function() {
        try {
          return s.nodebuffer && String.fromCharCode.apply(null, a.allocBuffer(1)).length === 1;
        } catch {
          return !1;
        }
      }() } };
      function y(h) {
        var p = 65536, d = i.getTypeOf(h), v = !0;
        if (d === "uint8array" ? v = _.applyCanBeUsed.uint8array : d === "nodebuffer" && (v = _.applyCanBeUsed.nodebuffer), v) for (; 1 < p; ) try {
          return _.stringifyByChunk(h, d, p);
        } catch {
          p = Math.floor(p / 2);
        }
        return _.stringifyByChar(h);
      }
      function f(h, p) {
        for (var d = 0; d < h.length; d++) p[d] = h[d];
        return p;
      }
      i.applyFromCharCode = y;
      var w = {};
      w.string = { string: m, array: function(h) {
        return k(h, new Array(h.length));
      }, arraybuffer: function(h) {
        return w.string.uint8array(h).buffer;
      }, uint8array: function(h) {
        return k(h, new Uint8Array(h.length));
      }, nodebuffer: function(h) {
        return k(h, a.allocBuffer(h.length));
      } }, w.array = { string: y, array: m, arraybuffer: function(h) {
        return new Uint8Array(h).buffer;
      }, uint8array: function(h) {
        return new Uint8Array(h);
      }, nodebuffer: function(h) {
        return a.newBufferFrom(h);
      } }, w.arraybuffer = { string: function(h) {
        return y(new Uint8Array(h));
      }, array: function(h) {
        return f(new Uint8Array(h), new Array(h.byteLength));
      }, arraybuffer: m, uint8array: function(h) {
        return new Uint8Array(h);
      }, nodebuffer: function(h) {
        return a.newBufferFrom(new Uint8Array(h));
      } }, w.uint8array = { string: y, array: function(h) {
        return f(h, new Array(h.length));
      }, arraybuffer: function(h) {
        return h.buffer;
      }, uint8array: m, nodebuffer: function(h) {
        return a.newBufferFrom(h);
      } }, w.nodebuffer = { string: y, array: function(h) {
        return f(h, new Array(h.length));
      }, arraybuffer: function(h) {
        return w.nodebuffer.uint8array(h).buffer;
      }, uint8array: function(h) {
        return f(h, new Uint8Array(h.length));
      }, nodebuffer: m }, i.transformTo = function(h, p) {
        if (p = p || "", !h) return p;
        i.checkSupport(h);
        var d = i.getTypeOf(p);
        return w[d][h](p);
      }, i.resolve = function(h) {
        for (var p = h.split("/"), d = [], v = 0; v < p.length; v++) {
          var z = p[v];
          z === "." || z === "" && v !== 0 && v !== p.length - 1 || (z === ".." ? d.pop() : d.push(z));
        }
        return d.join("/");
      }, i.getTypeOf = function(h) {
        return typeof h == "string" ? "string" : Object.prototype.toString.call(h) === "[object Array]" ? "array" : s.nodebuffer && a.isBuffer(h) ? "nodebuffer" : s.uint8array && h instanceof Uint8Array ? "uint8array" : s.arraybuffer && h instanceof ArrayBuffer ? "arraybuffer" : void 0;
      }, i.checkSupport = function(h) {
        if (!s[h.toLowerCase()]) throw new Error(h + " is not supported by this platform");
      }, i.MAX_VALUE_16BITS = 65535, i.MAX_VALUE_32BITS = -1, i.pretty = function(h) {
        var p, d, v = "";
        for (d = 0; d < (h || "").length; d++) v += "\\x" + ((p = h.charCodeAt(d)) < 16 ? "0" : "") + p.toString(16).toUpperCase();
        return v;
      }, i.delay = function(h, p, d) {
        setImmediate(function() {
          h.apply(d || null, p || []);
        });
      }, i.inherits = function(h, p) {
        function d() {
        }
        d.prototype = p.prototype, h.prototype = new d();
      }, i.extend = function() {
        var h, p, d = {};
        for (h = 0; h < arguments.length; h++) for (p in arguments[h]) Object.prototype.hasOwnProperty.call(arguments[h], p) && d[p] === void 0 && (d[p] = arguments[h][p]);
        return d;
      }, i.prepareContent = function(h, p, d, v, z) {
        return l.Promise.resolve(p).then(function(I) {
          return s.blob && (I instanceof Blob || ["[object File]", "[object Blob]"].indexOf(Object.prototype.toString.call(I)) !== -1) && typeof FileReader < "u" ? new l.Promise(function(S, D) {
            var T = new FileReader();
            T.onload = function(L) {
              S(L.target.result);
            }, T.onerror = function(L) {
              D(L.target.error);
            }, T.readAsArrayBuffer(I);
          }) : I;
        }).then(function(I) {
          var S = i.getTypeOf(I);
          return S ? (S === "arraybuffer" ? I = i.transformTo("uint8array", I) : S === "string" && (z ? I = o.decode(I) : d && v !== !0 && (I = function(D) {
            return k(D, s.uint8array ? new Uint8Array(D.length) : new Array(D.length));
          }(I))), I) : l.Promise.reject(new Error("Can't read the data of '" + h + "'. Is it in a supported JavaScript type (String, Blob, ArrayBuffer, etc) ?"));
        });
      };
    }, { "./base64": 1, "./external": 6, "./nodejsUtils": 14, "./support": 30, setimmediate: 54 }], 33: [function(n, r, i) {
      var s = n("./reader/readerFor"), o = n("./utils"), a = n("./signature"), l = n("./zipEntry"), m = n("./support");
      function k(_) {
        this.files = [], this.loadOptions = _;
      }
      k.prototype = { checkSignature: function(_) {
        if (!this.reader.readAndCheckSignature(_)) {
          this.reader.index -= 4;
          var y = this.reader.readString(4);
          throw new Error("Corrupted zip or bug: unexpected signature (" + o.pretty(y) + ", expected " + o.pretty(_) + ")");
        }
      }, isSignature: function(_, y) {
        var f = this.reader.index;
        this.reader.setIndex(_);
        var w = this.reader.readString(4) === y;
        return this.reader.setIndex(f), w;
      }, readBlockEndOfCentral: function() {
        this.diskNumber = this.reader.readInt(2), this.diskWithCentralDirStart = this.reader.readInt(2), this.centralDirRecordsOnThisDisk = this.reader.readInt(2), this.centralDirRecords = this.reader.readInt(2), this.centralDirSize = this.reader.readInt(4), this.centralDirOffset = this.reader.readInt(4), this.zipCommentLength = this.reader.readInt(2);
        var _ = this.reader.readData(this.zipCommentLength), y = m.uint8array ? "uint8array" : "array", f = o.transformTo(y, _);
        this.zipComment = this.loadOptions.decodeFileName(f);
      }, readBlockZip64EndOfCentral: function() {
        this.zip64EndOfCentralSize = this.reader.readInt(8), this.reader.skip(4), this.diskNumber = this.reader.readInt(4), this.diskWithCentralDirStart = this.reader.readInt(4), this.centralDirRecordsOnThisDisk = this.reader.readInt(8), this.centralDirRecords = this.reader.readInt(8), this.centralDirSize = this.reader.readInt(8), this.centralDirOffset = this.reader.readInt(8), this.zip64ExtensibleData = {};
        for (var _, y, f, w = this.zip64EndOfCentralSize - 44; 0 < w; ) _ = this.reader.readInt(2), y = this.reader.readInt(4), f = this.reader.readData(y), this.zip64ExtensibleData[_] = { id: _, length: y, value: f };
      }, readBlockZip64EndOfCentralLocator: function() {
        if (this.diskWithZip64CentralDirStart = this.reader.readInt(4), this.relativeOffsetEndOfZip64CentralDir = this.reader.readInt(8), this.disksCount = this.reader.readInt(4), 1 < this.disksCount) throw new Error("Multi-volumes zip are not supported");
      }, readLocalFiles: function() {
        var _, y;
        for (_ = 0; _ < this.files.length; _++) y = this.files[_], this.reader.setIndex(y.localHeaderOffset), this.checkSignature(a.LOCAL_FILE_HEADER), y.readLocalPart(this.reader), y.handleUTF8(), y.processAttributes();
      }, readCentralDir: function() {
        var _;
        for (this.reader.setIndex(this.centralDirOffset); this.reader.readAndCheckSignature(a.CENTRAL_FILE_HEADER); ) (_ = new l({ zip64: this.zip64 }, this.loadOptions)).readCentralPart(this.reader), this.files.push(_);
        if (this.centralDirRecords !== this.files.length && this.centralDirRecords !== 0 && this.files.length === 0) throw new Error("Corrupted zip or bug: expected " + this.centralDirRecords + " records in central dir, got " + this.files.length);
      }, readEndOfCentral: function() {
        var _ = this.reader.lastIndexOfSignature(a.CENTRAL_DIRECTORY_END);
        if (_ < 0) throw this.isSignature(0, a.LOCAL_FILE_HEADER) ? new Error("Corrupted zip: can't find end of central directory") : new Error("Can't find end of central directory : is this a zip file ? If it is, see https://stuk.github.io/jszip/documentation/howto/read_zip.html");
        this.reader.setIndex(_);
        var y = _;
        if (this.checkSignature(a.CENTRAL_DIRECTORY_END), this.readBlockEndOfCentral(), this.diskNumber === o.MAX_VALUE_16BITS || this.diskWithCentralDirStart === o.MAX_VALUE_16BITS || this.centralDirRecordsOnThisDisk === o.MAX_VALUE_16BITS || this.centralDirRecords === o.MAX_VALUE_16BITS || this.centralDirSize === o.MAX_VALUE_32BITS || this.centralDirOffset === o.MAX_VALUE_32BITS) {
          if (this.zip64 = !0, (_ = this.reader.lastIndexOfSignature(a.ZIP64_CENTRAL_DIRECTORY_LOCATOR)) < 0) throw new Error("Corrupted zip: can't find the ZIP64 end of central directory locator");
          if (this.reader.setIndex(_), this.checkSignature(a.ZIP64_CENTRAL_DIRECTORY_LOCATOR), this.readBlockZip64EndOfCentralLocator(), !this.isSignature(this.relativeOffsetEndOfZip64CentralDir, a.ZIP64_CENTRAL_DIRECTORY_END) && (this.relativeOffsetEndOfZip64CentralDir = this.reader.lastIndexOfSignature(a.ZIP64_CENTRAL_DIRECTORY_END), this.relativeOffsetEndOfZip64CentralDir < 0)) throw new Error("Corrupted zip: can't find the ZIP64 end of central directory");
          this.reader.setIndex(this.relativeOffsetEndOfZip64CentralDir), this.checkSignature(a.ZIP64_CENTRAL_DIRECTORY_END), this.readBlockZip64EndOfCentral();
        }
        var f = this.centralDirOffset + this.centralDirSize;
        this.zip64 && (f += 20, f += 12 + this.zip64EndOfCentralSize);
        var w = y - f;
        if (0 < w) this.isSignature(y, a.CENTRAL_FILE_HEADER) || (this.reader.zero = w);
        else if (w < 0) throw new Error("Corrupted zip: missing " + Math.abs(w) + " bytes.");
      }, prepareReader: function(_) {
        this.reader = s(_);
      }, load: function(_) {
        this.prepareReader(_), this.readEndOfCentral(), this.readCentralDir(), this.readLocalFiles();
      } }, r.exports = k;
    }, { "./reader/readerFor": 22, "./signature": 23, "./support": 30, "./utils": 32, "./zipEntry": 34 }], 34: [function(n, r, i) {
      var s = n("./reader/readerFor"), o = n("./utils"), a = n("./compressedObject"), l = n("./crc32"), m = n("./utf8"), k = n("./compressions"), _ = n("./support");
      function y(f, w) {
        this.options = f, this.loadOptions = w;
      }
      y.prototype = { isEncrypted: function() {
        return (1 & this.bitFlag) == 1;
      }, useUTF8: function() {
        return (2048 & this.bitFlag) == 2048;
      }, readLocalPart: function(f) {
        var w, h;
        if (f.skip(22), this.fileNameLength = f.readInt(2), h = f.readInt(2), this.fileName = f.readData(this.fileNameLength), f.skip(h), this.compressedSize === -1 || this.uncompressedSize === -1) throw new Error("Bug or corrupted zip : didn't get enough information from the central directory (compressedSize === -1 || uncompressedSize === -1)");
        if ((w = function(p) {
          for (var d in k) if (Object.prototype.hasOwnProperty.call(k, d) && k[d].magic === p) return k[d];
          return null;
        }(this.compressionMethod)) === null) throw new Error("Corrupted zip : compression " + o.pretty(this.compressionMethod) + " unknown (inner file : " + o.transformTo("string", this.fileName) + ")");
        this.decompressed = new a(this.compressedSize, this.uncompressedSize, this.crc32, w, f.readData(this.compressedSize));
      }, readCentralPart: function(f) {
        this.versionMadeBy = f.readInt(2), f.skip(2), this.bitFlag = f.readInt(2), this.compressionMethod = f.readString(2), this.date = f.readDate(), this.crc32 = f.readInt(4), this.compressedSize = f.readInt(4), this.uncompressedSize = f.readInt(4);
        var w = f.readInt(2);
        if (this.extraFieldsLength = f.readInt(2), this.fileCommentLength = f.readInt(2), this.diskNumberStart = f.readInt(2), this.internalFileAttributes = f.readInt(2), this.externalFileAttributes = f.readInt(4), this.localHeaderOffset = f.readInt(4), this.isEncrypted()) throw new Error("Encrypted zip are not supported");
        f.skip(w), this.readExtraFields(f), this.parseZIP64ExtraField(f), this.fileComment = f.readData(this.fileCommentLength);
      }, processAttributes: function() {
        this.unixPermissions = null, this.dosPermissions = null;
        var f = this.versionMadeBy >> 8;
        this.dir = !!(16 & this.externalFileAttributes), f == 0 && (this.dosPermissions = 63 & this.externalFileAttributes), f == 3 && (this.unixPermissions = this.externalFileAttributes >> 16 & 65535), this.dir || this.fileNameStr.slice(-1) !== "/" || (this.dir = !0);
      }, parseZIP64ExtraField: function() {
        if (this.extraFields[1]) {
          var f = s(this.extraFields[1].value);
          this.uncompressedSize === o.MAX_VALUE_32BITS && (this.uncompressedSize = f.readInt(8)), this.compressedSize === o.MAX_VALUE_32BITS && (this.compressedSize = f.readInt(8)), this.localHeaderOffset === o.MAX_VALUE_32BITS && (this.localHeaderOffset = f.readInt(8)), this.diskNumberStart === o.MAX_VALUE_32BITS && (this.diskNumberStart = f.readInt(4));
        }
      }, readExtraFields: function(f) {
        var w, h, p, d = f.index + this.extraFieldsLength;
        for (this.extraFields || (this.extraFields = {}); f.index + 4 < d; ) w = f.readInt(2), h = f.readInt(2), p = f.readData(h), this.extraFields[w] = { id: w, length: h, value: p };
        f.setIndex(d);
      }, handleUTF8: function() {
        var f = _.uint8array ? "uint8array" : "array";
        if (this.useUTF8()) this.fileNameStr = m.utf8decode(this.fileName), this.fileCommentStr = m.utf8decode(this.fileComment);
        else {
          var w = this.findExtraFieldUnicodePath();
          if (w !== null) this.fileNameStr = w;
          else {
            var h = o.transformTo(f, this.fileName);
            this.fileNameStr = this.loadOptions.decodeFileName(h);
          }
          var p = this.findExtraFieldUnicodeComment();
          if (p !== null) this.fileCommentStr = p;
          else {
            var d = o.transformTo(f, this.fileComment);
            this.fileCommentStr = this.loadOptions.decodeFileName(d);
          }
        }
      }, findExtraFieldUnicodePath: function() {
        var f = this.extraFields[28789];
        if (f) {
          var w = s(f.value);
          return w.readInt(1) !== 1 || l(this.fileName) !== w.readInt(4) ? null : m.utf8decode(w.readData(f.length - 5));
        }
        return null;
      }, findExtraFieldUnicodeComment: function() {
        var f = this.extraFields[25461];
        if (f) {
          var w = s(f.value);
          return w.readInt(1) !== 1 || l(this.fileComment) !== w.readInt(4) ? null : m.utf8decode(w.readData(f.length - 5));
        }
        return null;
      } }, r.exports = y;
    }, { "./compressedObject": 2, "./compressions": 3, "./crc32": 4, "./reader/readerFor": 22, "./support": 30, "./utf8": 31, "./utils": 32 }], 35: [function(n, r, i) {
      function s(w, h, p) {
        this.name = w, this.dir = p.dir, this.date = p.date, this.comment = p.comment, this.unixPermissions = p.unixPermissions, this.dosPermissions = p.dosPermissions, this._data = h, this._dataBinary = p.binary, this.options = { compression: p.compression, compressionOptions: p.compressionOptions };
      }
      var o = n("./stream/StreamHelper"), a = n("./stream/DataWorker"), l = n("./utf8"), m = n("./compressedObject"), k = n("./stream/GenericWorker");
      s.prototype = { internalStream: function(w) {
        var h = null, p = "string";
        try {
          if (!w) throw new Error("No output type specified.");
          var d = (p = w.toLowerCase()) === "string" || p === "text";
          p !== "binarystring" && p !== "text" || (p = "string"), h = this._decompressWorker();
          var v = !this._dataBinary;
          v && !d && (h = h.pipe(new l.Utf8EncodeWorker())), !v && d && (h = h.pipe(new l.Utf8DecodeWorker()));
        } catch (z) {
          (h = new k("error")).error(z);
        }
        return new o(h, p, "");
      }, async: function(w, h) {
        return this.internalStream(w).accumulate(h);
      }, nodeStream: function(w, h) {
        return this.internalStream(w || "nodebuffer").toNodejsStream(h);
      }, _compressWorker: function(w, h) {
        if (this._data instanceof m && this._data.compression.magic === w.magic) return this._data.getCompressedWorker();
        var p = this._decompressWorker();
        return this._dataBinary || (p = p.pipe(new l.Utf8EncodeWorker())), m.createWorkerFrom(p, w, h);
      }, _decompressWorker: function() {
        return this._data instanceof m ? this._data.getContentWorker() : this._data instanceof k ? this._data : new a(this._data);
      } };
      for (var _ = ["asText", "asBinary", "asNodeBuffer", "asUint8Array", "asArrayBuffer"], y = function() {
        throw new Error("This method has been removed in JSZip 3.0, please check the upgrade guide.");
      }, f = 0; f < _.length; f++) s.prototype[_[f]] = y;
      r.exports = s;
    }, { "./compressedObject": 2, "./stream/DataWorker": 27, "./stream/GenericWorker": 28, "./stream/StreamHelper": 29, "./utf8": 31 }], 36: [function(n, r, i) {
      (function(s) {
        var o, a, l = s.MutationObserver || s.WebKitMutationObserver;
        if (l) {
          var m = 0, k = new l(w), _ = s.document.createTextNode("");
          k.observe(_, { characterData: !0 }), o = function() {
            _.data = m = ++m % 2;
          };
        } else if (s.setImmediate || s.MessageChannel === void 0) o = "document" in s && "onreadystatechange" in s.document.createElement("script") ? function() {
          var h = s.document.createElement("script");
          h.onreadystatechange = function() {
            w(), h.onreadystatechange = null, h.parentNode.removeChild(h), h = null;
          }, s.document.documentElement.appendChild(h);
        } : function() {
          setTimeout(w, 0);
        };
        else {
          var y = new s.MessageChannel();
          y.port1.onmessage = w, o = function() {
            y.port2.postMessage(0);
          };
        }
        var f = [];
        function w() {
          var h, p;
          a = !0;
          for (var d = f.length; d; ) {
            for (p = f, f = [], h = -1; ++h < d; ) p[h]();
            d = f.length;
          }
          a = !1;
        }
        r.exports = function(h) {
          f.push(h) !== 1 || a || o();
        };
      }).call(this, typeof Jt < "u" ? Jt : typeof self < "u" ? self : typeof window < "u" ? window : {});
    }, {}], 37: [function(n, r, i) {
      var s = n("immediate");
      function o() {
      }
      var a = {}, l = ["REJECTED"], m = ["FULFILLED"], k = ["PENDING"];
      function _(d) {
        if (typeof d != "function") throw new TypeError("resolver must be a function");
        this.state = k, this.queue = [], this.outcome = void 0, d !== o && h(this, d);
      }
      function y(d, v, z) {
        this.promise = d, typeof v == "function" && (this.onFulfilled = v, this.callFulfilled = this.otherCallFulfilled), typeof z == "function" && (this.onRejected = z, this.callRejected = this.otherCallRejected);
      }
      function f(d, v, z) {
        s(function() {
          var I;
          try {
            I = v(z);
          } catch (S) {
            return a.reject(d, S);
          }
          I === d ? a.reject(d, new TypeError("Cannot resolve promise with itself")) : a.resolve(d, I);
        });
      }
      function w(d) {
        var v = d && d.then;
        if (d && (typeof d == "object" || typeof d == "function") && typeof v == "function") return function() {
          v.apply(d, arguments);
        };
      }
      function h(d, v) {
        var z = !1;
        function I(T) {
          z || (z = !0, a.reject(d, T));
        }
        function S(T) {
          z || (z = !0, a.resolve(d, T));
        }
        var D = p(function() {
          v(S, I);
        });
        D.status === "error" && I(D.value);
      }
      function p(d, v) {
        var z = {};
        try {
          z.value = d(v), z.status = "success";
        } catch (I) {
          z.status = "error", z.value = I;
        }
        return z;
      }
      (r.exports = _).prototype.finally = function(d) {
        if (typeof d != "function") return this;
        var v = this.constructor;
        return this.then(function(z) {
          return v.resolve(d()).then(function() {
            return z;
          });
        }, function(z) {
          return v.resolve(d()).then(function() {
            throw z;
          });
        });
      }, _.prototype.catch = function(d) {
        return this.then(null, d);
      }, _.prototype.then = function(d, v) {
        if (typeof d != "function" && this.state === m || typeof v != "function" && this.state === l) return this;
        var z = new this.constructor(o);
        return this.state !== k ? f(z, this.state === m ? d : v, this.outcome) : this.queue.push(new y(z, d, v)), z;
      }, y.prototype.callFulfilled = function(d) {
        a.resolve(this.promise, d);
      }, y.prototype.otherCallFulfilled = function(d) {
        f(this.promise, this.onFulfilled, d);
      }, y.prototype.callRejected = function(d) {
        a.reject(this.promise, d);
      }, y.prototype.otherCallRejected = function(d) {
        f(this.promise, this.onRejected, d);
      }, a.resolve = function(d, v) {
        var z = p(w, v);
        if (z.status === "error") return a.reject(d, z.value);
        var I = z.value;
        if (I) h(d, I);
        else {
          d.state = m, d.outcome = v;
          for (var S = -1, D = d.queue.length; ++S < D; ) d.queue[S].callFulfilled(v);
        }
        return d;
      }, a.reject = function(d, v) {
        d.state = l, d.outcome = v;
        for (var z = -1, I = d.queue.length; ++z < I; ) d.queue[z].callRejected(v);
        return d;
      }, _.resolve = function(d) {
        return d instanceof this ? d : a.resolve(new this(o), d);
      }, _.reject = function(d) {
        var v = new this(o);
        return a.reject(v, d);
      }, _.all = function(d) {
        var v = this;
        if (Object.prototype.toString.call(d) !== "[object Array]") return this.reject(new TypeError("must be an array"));
        var z = d.length, I = !1;
        if (!z) return this.resolve([]);
        for (var S = new Array(z), D = 0, T = -1, L = new this(o); ++T < z; ) $(d[T], T);
        return L;
        function $(V, Q) {
          v.resolve(V).then(function(x) {
            S[Q] = x, ++D !== z || I || (I = !0, a.resolve(L, S));
          }, function(x) {
            I || (I = !0, a.reject(L, x));
          });
        }
      }, _.race = function(d) {
        var v = this;
        if (Object.prototype.toString.call(d) !== "[object Array]") return this.reject(new TypeError("must be an array"));
        var z = d.length, I = !1;
        if (!z) return this.resolve([]);
        for (var S = -1, D = new this(o); ++S < z; ) T = d[S], v.resolve(T).then(function(L) {
          I || (I = !0, a.resolve(D, L));
        }, function(L) {
          I || (I = !0, a.reject(D, L));
        });
        var T;
        return D;
      };
    }, { immediate: 36 }], 38: [function(n, r, i) {
      var s = {};
      (0, n("./lib/utils/common").assign)(s, n("./lib/deflate"), n("./lib/inflate"), n("./lib/zlib/constants")), r.exports = s;
    }, { "./lib/deflate": 39, "./lib/inflate": 40, "./lib/utils/common": 41, "./lib/zlib/constants": 44 }], 39: [function(n, r, i) {
      var s = n("./zlib/deflate"), o = n("./utils/common"), a = n("./utils/strings"), l = n("./zlib/messages"), m = n("./zlib/zstream"), k = Object.prototype.toString, _ = 0, y = -1, f = 0, w = 8;
      function h(d) {
        if (!(this instanceof h)) return new h(d);
        this.options = o.assign({ level: y, method: w, chunkSize: 16384, windowBits: 15, memLevel: 8, strategy: f, to: "" }, d || {});
        var v = this.options;
        v.raw && 0 < v.windowBits ? v.windowBits = -v.windowBits : v.gzip && 0 < v.windowBits && v.windowBits < 16 && (v.windowBits += 16), this.err = 0, this.msg = "", this.ended = !1, this.chunks = [], this.strm = new m(), this.strm.avail_out = 0;
        var z = s.deflateInit2(this.strm, v.level, v.method, v.windowBits, v.memLevel, v.strategy);
        if (z !== _) throw new Error(l[z]);
        if (v.header && s.deflateSetHeader(this.strm, v.header), v.dictionary) {
          var I;
          if (I = typeof v.dictionary == "string" ? a.string2buf(v.dictionary) : k.call(v.dictionary) === "[object ArrayBuffer]" ? new Uint8Array(v.dictionary) : v.dictionary, (z = s.deflateSetDictionary(this.strm, I)) !== _) throw new Error(l[z]);
          this._dict_set = !0;
        }
      }
      function p(d, v) {
        var z = new h(v);
        if (z.push(d, !0), z.err) throw z.msg || l[z.err];
        return z.result;
      }
      h.prototype.push = function(d, v) {
        var z, I, S = this.strm, D = this.options.chunkSize;
        if (this.ended) return !1;
        I = v === ~~v ? v : v === !0 ? 4 : 0, typeof d == "string" ? S.input = a.string2buf(d) : k.call(d) === "[object ArrayBuffer]" ? S.input = new Uint8Array(d) : S.input = d, S.next_in = 0, S.avail_in = S.input.length;
        do {
          if (S.avail_out === 0 && (S.output = new o.Buf8(D), S.next_out = 0, S.avail_out = D), (z = s.deflate(S, I)) !== 1 && z !== _) return this.onEnd(z), !(this.ended = !0);
          S.avail_out !== 0 && (S.avail_in !== 0 || I !== 4 && I !== 2) || (this.options.to === "string" ? this.onData(a.buf2binstring(o.shrinkBuf(S.output, S.next_out))) : this.onData(o.shrinkBuf(S.output, S.next_out)));
        } while ((0 < S.avail_in || S.avail_out === 0) && z !== 1);
        return I === 4 ? (z = s.deflateEnd(this.strm), this.onEnd(z), this.ended = !0, z === _) : I !== 2 || (this.onEnd(_), !(S.avail_out = 0));
      }, h.prototype.onData = function(d) {
        this.chunks.push(d);
      }, h.prototype.onEnd = function(d) {
        d === _ && (this.options.to === "string" ? this.result = this.chunks.join("") : this.result = o.flattenChunks(this.chunks)), this.chunks = [], this.err = d, this.msg = this.strm.msg;
      }, i.Deflate = h, i.deflate = p, i.deflateRaw = function(d, v) {
        return (v = v || {}).raw = !0, p(d, v);
      }, i.gzip = function(d, v) {
        return (v = v || {}).gzip = !0, p(d, v);
      };
    }, { "./utils/common": 41, "./utils/strings": 42, "./zlib/deflate": 46, "./zlib/messages": 51, "./zlib/zstream": 53 }], 40: [function(n, r, i) {
      var s = n("./zlib/inflate"), o = n("./utils/common"), a = n("./utils/strings"), l = n("./zlib/constants"), m = n("./zlib/messages"), k = n("./zlib/zstream"), _ = n("./zlib/gzheader"), y = Object.prototype.toString;
      function f(h) {
        if (!(this instanceof f)) return new f(h);
        this.options = o.assign({ chunkSize: 16384, windowBits: 0, to: "" }, h || {});
        var p = this.options;
        p.raw && 0 <= p.windowBits && p.windowBits < 16 && (p.windowBits = -p.windowBits, p.windowBits === 0 && (p.windowBits = -15)), !(0 <= p.windowBits && p.windowBits < 16) || h && h.windowBits || (p.windowBits += 32), 15 < p.windowBits && p.windowBits < 48 && !(15 & p.windowBits) && (p.windowBits |= 15), this.err = 0, this.msg = "", this.ended = !1, this.chunks = [], this.strm = new k(), this.strm.avail_out = 0;
        var d = s.inflateInit2(this.strm, p.windowBits);
        if (d !== l.Z_OK) throw new Error(m[d]);
        this.header = new _(), s.inflateGetHeader(this.strm, this.header);
      }
      function w(h, p) {
        var d = new f(p);
        if (d.push(h, !0), d.err) throw d.msg || m[d.err];
        return d.result;
      }
      f.prototype.push = function(h, p) {
        var d, v, z, I, S, D, T = this.strm, L = this.options.chunkSize, $ = this.options.dictionary, V = !1;
        if (this.ended) return !1;
        v = p === ~~p ? p : p === !0 ? l.Z_FINISH : l.Z_NO_FLUSH, typeof h == "string" ? T.input = a.binstring2buf(h) : y.call(h) === "[object ArrayBuffer]" ? T.input = new Uint8Array(h) : T.input = h, T.next_in = 0, T.avail_in = T.input.length;
        do {
          if (T.avail_out === 0 && (T.output = new o.Buf8(L), T.next_out = 0, T.avail_out = L), (d = s.inflate(T, l.Z_NO_FLUSH)) === l.Z_NEED_DICT && $ && (D = typeof $ == "string" ? a.string2buf($) : y.call($) === "[object ArrayBuffer]" ? new Uint8Array($) : $, d = s.inflateSetDictionary(this.strm, D)), d === l.Z_BUF_ERROR && V === !0 && (d = l.Z_OK, V = !1), d !== l.Z_STREAM_END && d !== l.Z_OK) return this.onEnd(d), !(this.ended = !0);
          T.next_out && (T.avail_out !== 0 && d !== l.Z_STREAM_END && (T.avail_in !== 0 || v !== l.Z_FINISH && v !== l.Z_SYNC_FLUSH) || (this.options.to === "string" ? (z = a.utf8border(T.output, T.next_out), I = T.next_out - z, S = a.buf2string(T.output, z), T.next_out = I, T.avail_out = L - I, I && o.arraySet(T.output, T.output, z, I, 0), this.onData(S)) : this.onData(o.shrinkBuf(T.output, T.next_out)))), T.avail_in === 0 && T.avail_out === 0 && (V = !0);
        } while ((0 < T.avail_in || T.avail_out === 0) && d !== l.Z_STREAM_END);
        return d === l.Z_STREAM_END && (v = l.Z_FINISH), v === l.Z_FINISH ? (d = s.inflateEnd(this.strm), this.onEnd(d), this.ended = !0, d === l.Z_OK) : v !== l.Z_SYNC_FLUSH || (this.onEnd(l.Z_OK), !(T.avail_out = 0));
      }, f.prototype.onData = function(h) {
        this.chunks.push(h);
      }, f.prototype.onEnd = function(h) {
        h === l.Z_OK && (this.options.to === "string" ? this.result = this.chunks.join("") : this.result = o.flattenChunks(this.chunks)), this.chunks = [], this.err = h, this.msg = this.strm.msg;
      }, i.Inflate = f, i.inflate = w, i.inflateRaw = function(h, p) {
        return (p = p || {}).raw = !0, w(h, p);
      }, i.ungzip = w;
    }, { "./utils/common": 41, "./utils/strings": 42, "./zlib/constants": 44, "./zlib/gzheader": 47, "./zlib/inflate": 49, "./zlib/messages": 51, "./zlib/zstream": 53 }], 41: [function(n, r, i) {
      var s = typeof Uint8Array < "u" && typeof Uint16Array < "u" && typeof Int32Array < "u";
      i.assign = function(l) {
        for (var m = Array.prototype.slice.call(arguments, 1); m.length; ) {
          var k = m.shift();
          if (k) {
            if (typeof k != "object") throw new TypeError(k + "must be non-object");
            for (var _ in k) k.hasOwnProperty(_) && (l[_] = k[_]);
          }
        }
        return l;
      }, i.shrinkBuf = function(l, m) {
        return l.length === m ? l : l.subarray ? l.subarray(0, m) : (l.length = m, l);
      };
      var o = { arraySet: function(l, m, k, _, y) {
        if (m.subarray && l.subarray) l.set(m.subarray(k, k + _), y);
        else for (var f = 0; f < _; f++) l[y + f] = m[k + f];
      }, flattenChunks: function(l) {
        var m, k, _, y, f, w;
        for (m = _ = 0, k = l.length; m < k; m++) _ += l[m].length;
        for (w = new Uint8Array(_), m = y = 0, k = l.length; m < k; m++) f = l[m], w.set(f, y), y += f.length;
        return w;
      } }, a = { arraySet: function(l, m, k, _, y) {
        for (var f = 0; f < _; f++) l[y + f] = m[k + f];
      }, flattenChunks: function(l) {
        return [].concat.apply([], l);
      } };
      i.setTyped = function(l) {
        l ? (i.Buf8 = Uint8Array, i.Buf16 = Uint16Array, i.Buf32 = Int32Array, i.assign(i, o)) : (i.Buf8 = Array, i.Buf16 = Array, i.Buf32 = Array, i.assign(i, a));
      }, i.setTyped(s);
    }, {}], 42: [function(n, r, i) {
      var s = n("./common"), o = !0, a = !0;
      try {
        String.fromCharCode.apply(null, [0]);
      } catch {
        o = !1;
      }
      try {
        String.fromCharCode.apply(null, new Uint8Array(1));
      } catch {
        a = !1;
      }
      for (var l = new s.Buf8(256), m = 0; m < 256; m++) l[m] = 252 <= m ? 6 : 248 <= m ? 5 : 240 <= m ? 4 : 224 <= m ? 3 : 192 <= m ? 2 : 1;
      function k(_, y) {
        if (y < 65537 && (_.subarray && a || !_.subarray && o)) return String.fromCharCode.apply(null, s.shrinkBuf(_, y));
        for (var f = "", w = 0; w < y; w++) f += String.fromCharCode(_[w]);
        return f;
      }
      l[254] = l[254] = 1, i.string2buf = function(_) {
        var y, f, w, h, p, d = _.length, v = 0;
        for (h = 0; h < d; h++) (64512 & (f = _.charCodeAt(h))) == 55296 && h + 1 < d && (64512 & (w = _.charCodeAt(h + 1))) == 56320 && (f = 65536 + (f - 55296 << 10) + (w - 56320), h++), v += f < 128 ? 1 : f < 2048 ? 2 : f < 65536 ? 3 : 4;
        for (y = new s.Buf8(v), h = p = 0; p < v; h++) (64512 & (f = _.charCodeAt(h))) == 55296 && h + 1 < d && (64512 & (w = _.charCodeAt(h + 1))) == 56320 && (f = 65536 + (f - 55296 << 10) + (w - 56320), h++), f < 128 ? y[p++] = f : (f < 2048 ? y[p++] = 192 | f >>> 6 : (f < 65536 ? y[p++] = 224 | f >>> 12 : (y[p++] = 240 | f >>> 18, y[p++] = 128 | f >>> 12 & 63), y[p++] = 128 | f >>> 6 & 63), y[p++] = 128 | 63 & f);
        return y;
      }, i.buf2binstring = function(_) {
        return k(_, _.length);
      }, i.binstring2buf = function(_) {
        for (var y = new s.Buf8(_.length), f = 0, w = y.length; f < w; f++) y[f] = _.charCodeAt(f);
        return y;
      }, i.buf2string = function(_, y) {
        var f, w, h, p, d = y || _.length, v = new Array(2 * d);
        for (f = w = 0; f < d; ) if ((h = _[f++]) < 128) v[w++] = h;
        else if (4 < (p = l[h])) v[w++] = 65533, f += p - 1;
        else {
          for (h &= p === 2 ? 31 : p === 3 ? 15 : 7; 1 < p && f < d; ) h = h << 6 | 63 & _[f++], p--;
          1 < p ? v[w++] = 65533 : h < 65536 ? v[w++] = h : (h -= 65536, v[w++] = 55296 | h >> 10 & 1023, v[w++] = 56320 | 1023 & h);
        }
        return k(v, w);
      }, i.utf8border = function(_, y) {
        var f;
        for ((y = y || _.length) > _.length && (y = _.length), f = y - 1; 0 <= f && (192 & _[f]) == 128; ) f--;
        return f < 0 || f === 0 ? y : f + l[_[f]] > y ? f : y;
      };
    }, { "./common": 41 }], 43: [function(n, r, i) {
      r.exports = function(s, o, a, l) {
        for (var m = 65535 & s | 0, k = s >>> 16 & 65535 | 0, _ = 0; a !== 0; ) {
          for (a -= _ = 2e3 < a ? 2e3 : a; k = k + (m = m + o[l++] | 0) | 0, --_; ) ;
          m %= 65521, k %= 65521;
        }
        return m | k << 16 | 0;
      };
    }, {}], 44: [function(n, r, i) {
      r.exports = { Z_NO_FLUSH: 0, Z_PARTIAL_FLUSH: 1, Z_SYNC_FLUSH: 2, Z_FULL_FLUSH: 3, Z_FINISH: 4, Z_BLOCK: 5, Z_TREES: 6, Z_OK: 0, Z_STREAM_END: 1, Z_NEED_DICT: 2, Z_ERRNO: -1, Z_STREAM_ERROR: -2, Z_DATA_ERROR: -3, Z_BUF_ERROR: -5, Z_NO_COMPRESSION: 0, Z_BEST_SPEED: 1, Z_BEST_COMPRESSION: 9, Z_DEFAULT_COMPRESSION: -1, Z_FILTERED: 1, Z_HUFFMAN_ONLY: 2, Z_RLE: 3, Z_FIXED: 4, Z_DEFAULT_STRATEGY: 0, Z_BINARY: 0, Z_TEXT: 1, Z_UNKNOWN: 2, Z_DEFLATED: 8 };
    }, {}], 45: [function(n, r, i) {
      var s = function() {
        for (var o, a = [], l = 0; l < 256; l++) {
          o = l;
          for (var m = 0; m < 8; m++) o = 1 & o ? 3988292384 ^ o >>> 1 : o >>> 1;
          a[l] = o;
        }
        return a;
      }();
      r.exports = function(o, a, l, m) {
        var k = s, _ = m + l;
        o ^= -1;
        for (var y = m; y < _; y++) o = o >>> 8 ^ k[255 & (o ^ a[y])];
        return -1 ^ o;
      };
    }, {}], 46: [function(n, r, i) {
      var s, o = n("../utils/common"), a = n("./trees"), l = n("./adler32"), m = n("./crc32"), k = n("./messages"), _ = 0, y = 4, f = 0, w = -2, h = -1, p = 4, d = 2, v = 8, z = 9, I = 286, S = 30, D = 19, T = 2 * I + 1, L = 15, $ = 3, V = 258, Q = V + $ + 1, x = 42, R = 113, c = 1, P = 2, et = 3, U = 4;
      function nt(u, F) {
        return u.msg = k[F], F;
      }
      function M(u) {
        return (u << 1) - (4 < u ? 9 : 0);
      }
      function tt(u) {
        for (var F = u.length; 0 <= --F; ) u[F] = 0;
      }
      function C(u) {
        var F = u.state, Z = F.pending;
        Z > u.avail_out && (Z = u.avail_out), Z !== 0 && (o.arraySet(u.output, F.pending_buf, F.pending_out, Z, u.next_out), u.next_out += Z, F.pending_out += Z, u.total_out += Z, u.avail_out -= Z, F.pending -= Z, F.pending === 0 && (F.pending_out = 0));
      }
      function O(u, F) {
        a._tr_flush_block(u, 0 <= u.block_start ? u.block_start : -1, u.strstart - u.block_start, F), u.block_start = u.strstart, C(u.strm);
      }
      function q(u, F) {
        u.pending_buf[u.pending++] = F;
      }
      function Y(u, F) {
        u.pending_buf[u.pending++] = F >>> 8 & 255, u.pending_buf[u.pending++] = 255 & F;
      }
      function H(u, F) {
        var Z, b, g = u.max_chain_length, E = u.strstart, B = u.prev_length, j = u.nice_match, A = u.strstart > u.w_size - Q ? u.strstart - (u.w_size - Q) : 0, W = u.window, K = u.w_mask, G = u.prev, X = u.strstart + V, ot = W[E + B - 1], it = W[E + B];
        u.prev_length >= u.good_match && (g >>= 2), j > u.lookahead && (j = u.lookahead);
        do
          if (W[(Z = F) + B] === it && W[Z + B - 1] === ot && W[Z] === W[E] && W[++Z] === W[E + 1]) {
            E += 2, Z++;
            do
              ;
            while (W[++E] === W[++Z] && W[++E] === W[++Z] && W[++E] === W[++Z] && W[++E] === W[++Z] && W[++E] === W[++Z] && W[++E] === W[++Z] && W[++E] === W[++Z] && W[++E] === W[++Z] && E < X);
            if (b = V - (X - E), E = X - V, B < b) {
              if (u.match_start = F, j <= (B = b)) break;
              ot = W[E + B - 1], it = W[E + B];
            }
          }
        while ((F = G[F & K]) > A && --g != 0);
        return B <= u.lookahead ? B : u.lookahead;
      }
      function ht(u) {
        var F, Z, b, g, E, B, j, A, W, K, G = u.w_size;
        do {
          if (g = u.window_size - u.lookahead - u.strstart, u.strstart >= G + (G - Q)) {
            for (o.arraySet(u.window, u.window, G, G, 0), u.match_start -= G, u.strstart -= G, u.block_start -= G, F = Z = u.hash_size; b = u.head[--F], u.head[F] = G <= b ? b - G : 0, --Z; ) ;
            for (F = Z = G; b = u.prev[--F], u.prev[F] = G <= b ? b - G : 0, --Z; ) ;
            g += G;
          }
          if (u.strm.avail_in === 0) break;
          if (B = u.strm, j = u.window, A = u.strstart + u.lookahead, W = g, K = void 0, K = B.avail_in, W < K && (K = W), Z = K === 0 ? 0 : (B.avail_in -= K, o.arraySet(j, B.input, B.next_in, K, A), B.state.wrap === 1 ? B.adler = l(B.adler, j, K, A) : B.state.wrap === 2 && (B.adler = m(B.adler, j, K, A)), B.next_in += K, B.total_in += K, K), u.lookahead += Z, u.lookahead + u.insert >= $) for (E = u.strstart - u.insert, u.ins_h = u.window[E], u.ins_h = (u.ins_h << u.hash_shift ^ u.window[E + 1]) & u.hash_mask; u.insert && (u.ins_h = (u.ins_h << u.hash_shift ^ u.window[E + $ - 1]) & u.hash_mask, u.prev[E & u.w_mask] = u.head[u.ins_h], u.head[u.ins_h] = E, E++, u.insert--, !(u.lookahead + u.insert < $)); ) ;
        } while (u.lookahead < Q && u.strm.avail_in !== 0);
      }
      function yt(u, F) {
        for (var Z, b; ; ) {
          if (u.lookahead < Q) {
            if (ht(u), u.lookahead < Q && F === _) return c;
            if (u.lookahead === 0) break;
          }
          if (Z = 0, u.lookahead >= $ && (u.ins_h = (u.ins_h << u.hash_shift ^ u.window[u.strstart + $ - 1]) & u.hash_mask, Z = u.prev[u.strstart & u.w_mask] = u.head[u.ins_h], u.head[u.ins_h] = u.strstart), Z !== 0 && u.strstart - Z <= u.w_size - Q && (u.match_length = H(u, Z)), u.match_length >= $) if (b = a._tr_tally(u, u.strstart - u.match_start, u.match_length - $), u.lookahead -= u.match_length, u.match_length <= u.max_lazy_match && u.lookahead >= $) {
            for (u.match_length--; u.strstart++, u.ins_h = (u.ins_h << u.hash_shift ^ u.window[u.strstart + $ - 1]) & u.hash_mask, Z = u.prev[u.strstart & u.w_mask] = u.head[u.ins_h], u.head[u.ins_h] = u.strstart, --u.match_length != 0; ) ;
            u.strstart++;
          } else u.strstart += u.match_length, u.match_length = 0, u.ins_h = u.window[u.strstart], u.ins_h = (u.ins_h << u.hash_shift ^ u.window[u.strstart + 1]) & u.hash_mask;
          else b = a._tr_tally(u, 0, u.window[u.strstart]), u.lookahead--, u.strstart++;
          if (b && (O(u, !1), u.strm.avail_out === 0)) return c;
        }
        return u.insert = u.strstart < $ - 1 ? u.strstart : $ - 1, F === y ? (O(u, !0), u.strm.avail_out === 0 ? et : U) : u.last_lit && (O(u, !1), u.strm.avail_out === 0) ? c : P;
      }
      function rt(u, F) {
        for (var Z, b, g; ; ) {
          if (u.lookahead < Q) {
            if (ht(u), u.lookahead < Q && F === _) return c;
            if (u.lookahead === 0) break;
          }
          if (Z = 0, u.lookahead >= $ && (u.ins_h = (u.ins_h << u.hash_shift ^ u.window[u.strstart + $ - 1]) & u.hash_mask, Z = u.prev[u.strstart & u.w_mask] = u.head[u.ins_h], u.head[u.ins_h] = u.strstart), u.prev_length = u.match_length, u.prev_match = u.match_start, u.match_length = $ - 1, Z !== 0 && u.prev_length < u.max_lazy_match && u.strstart - Z <= u.w_size - Q && (u.match_length = H(u, Z), u.match_length <= 5 && (u.strategy === 1 || u.match_length === $ && 4096 < u.strstart - u.match_start) && (u.match_length = $ - 1)), u.prev_length >= $ && u.match_length <= u.prev_length) {
            for (g = u.strstart + u.lookahead - $, b = a._tr_tally(u, u.strstart - 1 - u.prev_match, u.prev_length - $), u.lookahead -= u.prev_length - 1, u.prev_length -= 2; ++u.strstart <= g && (u.ins_h = (u.ins_h << u.hash_shift ^ u.window[u.strstart + $ - 1]) & u.hash_mask, Z = u.prev[u.strstart & u.w_mask] = u.head[u.ins_h], u.head[u.ins_h] = u.strstart), --u.prev_length != 0; ) ;
            if (u.match_available = 0, u.match_length = $ - 1, u.strstart++, b && (O(u, !1), u.strm.avail_out === 0)) return c;
          } else if (u.match_available) {
            if ((b = a._tr_tally(u, 0, u.window[u.strstart - 1])) && O(u, !1), u.strstart++, u.lookahead--, u.strm.avail_out === 0) return c;
          } else u.match_available = 1, u.strstart++, u.lookahead--;
        }
        return u.match_available && (b = a._tr_tally(u, 0, u.window[u.strstart - 1]), u.match_available = 0), u.insert = u.strstart < $ - 1 ? u.strstart : $ - 1, F === y ? (O(u, !0), u.strm.avail_out === 0 ? et : U) : u.last_lit && (O(u, !1), u.strm.avail_out === 0) ? c : P;
      }
      function st(u, F, Z, b, g) {
        this.good_length = u, this.max_lazy = F, this.nice_length = Z, this.max_chain = b, this.func = g;
      }
      function _t() {
        this.strm = null, this.status = 0, this.pending_buf = null, this.pending_buf_size = 0, this.pending_out = 0, this.pending = 0, this.wrap = 0, this.gzhead = null, this.gzindex = 0, this.method = v, this.last_flush = -1, this.w_size = 0, this.w_bits = 0, this.w_mask = 0, this.window = null, this.window_size = 0, this.prev = null, this.head = null, this.ins_h = 0, this.hash_size = 0, this.hash_bits = 0, this.hash_mask = 0, this.hash_shift = 0, this.block_start = 0, this.match_length = 0, this.prev_match = 0, this.match_available = 0, this.strstart = 0, this.match_start = 0, this.lookahead = 0, this.prev_length = 0, this.max_chain_length = 0, this.max_lazy_match = 0, this.level = 0, this.strategy = 0, this.good_match = 0, this.nice_match = 0, this.dyn_ltree = new o.Buf16(2 * T), this.dyn_dtree = new o.Buf16(2 * (2 * S + 1)), this.bl_tree = new o.Buf16(2 * (2 * D + 1)), tt(this.dyn_ltree), tt(this.dyn_dtree), tt(this.bl_tree), this.l_desc = null, this.d_desc = null, this.bl_desc = null, this.bl_count = new o.Buf16(L + 1), this.heap = new o.Buf16(2 * I + 1), tt(this.heap), this.heap_len = 0, this.heap_max = 0, this.depth = new o.Buf16(2 * I + 1), tt(this.depth), this.l_buf = 0, this.lit_bufsize = 0, this.last_lit = 0, this.d_buf = 0, this.opt_len = 0, this.static_len = 0, this.matches = 0, this.insert = 0, this.bi_buf = 0, this.bi_valid = 0;
      }
      function dt(u) {
        var F;
        return u && u.state ? (u.total_in = u.total_out = 0, u.data_type = d, (F = u.state).pending = 0, F.pending_out = 0, F.wrap < 0 && (F.wrap = -F.wrap), F.status = F.wrap ? x : R, u.adler = F.wrap === 2 ? 0 : 1, F.last_flush = _, a._tr_init(F), f) : nt(u, w);
      }
      function It(u) {
        var F = dt(u);
        return F === f && function(Z) {
          Z.window_size = 2 * Z.w_size, tt(Z.head), Z.max_lazy_match = s[Z.level].max_lazy, Z.good_match = s[Z.level].good_length, Z.nice_match = s[Z.level].nice_length, Z.max_chain_length = s[Z.level].max_chain, Z.strstart = 0, Z.block_start = 0, Z.lookahead = 0, Z.insert = 0, Z.match_length = Z.prev_length = $ - 1, Z.match_available = 0, Z.ins_h = 0;
        }(u.state), F;
      }
      function xt(u, F, Z, b, g, E) {
        if (!u) return w;
        var B = 1;
        if (F === h && (F = 6), b < 0 ? (B = 0, b = -b) : 15 < b && (B = 2, b -= 16), g < 1 || z < g || Z !== v || b < 8 || 15 < b || F < 0 || 9 < F || E < 0 || p < E) return nt(u, w);
        b === 8 && (b = 9);
        var j = new _t();
        return (u.state = j).strm = u, j.wrap = B, j.gzhead = null, j.w_bits = b, j.w_size = 1 << j.w_bits, j.w_mask = j.w_size - 1, j.hash_bits = g + 7, j.hash_size = 1 << j.hash_bits, j.hash_mask = j.hash_size - 1, j.hash_shift = ~~((j.hash_bits + $ - 1) / $), j.window = new o.Buf8(2 * j.w_size), j.head = new o.Buf16(j.hash_size), j.prev = new o.Buf16(j.w_size), j.lit_bufsize = 1 << g + 6, j.pending_buf_size = 4 * j.lit_bufsize, j.pending_buf = new o.Buf8(j.pending_buf_size), j.d_buf = 1 * j.lit_bufsize, j.l_buf = 3 * j.lit_bufsize, j.level = F, j.strategy = E, j.method = Z, It(u);
      }
      s = [new st(0, 0, 0, 0, function(u, F) {
        var Z = 65535;
        for (Z > u.pending_buf_size - 5 && (Z = u.pending_buf_size - 5); ; ) {
          if (u.lookahead <= 1) {
            if (ht(u), u.lookahead === 0 && F === _) return c;
            if (u.lookahead === 0) break;
          }
          u.strstart += u.lookahead, u.lookahead = 0;
          var b = u.block_start + Z;
          if ((u.strstart === 0 || u.strstart >= b) && (u.lookahead = u.strstart - b, u.strstart = b, O(u, !1), u.strm.avail_out === 0) || u.strstart - u.block_start >= u.w_size - Q && (O(u, !1), u.strm.avail_out === 0)) return c;
        }
        return u.insert = 0, F === y ? (O(u, !0), u.strm.avail_out === 0 ? et : U) : (u.strstart > u.block_start && (O(u, !1), u.strm.avail_out), c);
      }), new st(4, 4, 8, 4, yt), new st(4, 5, 16, 8, yt), new st(4, 6, 32, 32, yt), new st(4, 4, 16, 16, rt), new st(8, 16, 32, 32, rt), new st(8, 16, 128, 128, rt), new st(8, 32, 128, 256, rt), new st(32, 128, 258, 1024, rt), new st(32, 258, 258, 4096, rt)], i.deflateInit = function(u, F) {
        return xt(u, F, v, 15, 8, 0);
      }, i.deflateInit2 = xt, i.deflateReset = It, i.deflateResetKeep = dt, i.deflateSetHeader = function(u, F) {
        return u && u.state ? u.state.wrap !== 2 ? w : (u.state.gzhead = F, f) : w;
      }, i.deflate = function(u, F) {
        var Z, b, g, E;
        if (!u || !u.state || 5 < F || F < 0) return u ? nt(u, w) : w;
        if (b = u.state, !u.output || !u.input && u.avail_in !== 0 || b.status === 666 && F !== y) return nt(u, u.avail_out === 0 ? -5 : w);
        if (b.strm = u, Z = b.last_flush, b.last_flush = F, b.status === x) if (b.wrap === 2) u.adler = 0, q(b, 31), q(b, 139), q(b, 8), b.gzhead ? (q(b, (b.gzhead.text ? 1 : 0) + (b.gzhead.hcrc ? 2 : 0) + (b.gzhead.extra ? 4 : 0) + (b.gzhead.name ? 8 : 0) + (b.gzhead.comment ? 16 : 0)), q(b, 255 & b.gzhead.time), q(b, b.gzhead.time >> 8 & 255), q(b, b.gzhead.time >> 16 & 255), q(b, b.gzhead.time >> 24 & 255), q(b, b.level === 9 ? 2 : 2 <= b.strategy || b.level < 2 ? 4 : 0), q(b, 255 & b.gzhead.os), b.gzhead.extra && b.gzhead.extra.length && (q(b, 255 & b.gzhead.extra.length), q(b, b.gzhead.extra.length >> 8 & 255)), b.gzhead.hcrc && (u.adler = m(u.adler, b.pending_buf, b.pending, 0)), b.gzindex = 0, b.status = 69) : (q(b, 0), q(b, 0), q(b, 0), q(b, 0), q(b, 0), q(b, b.level === 9 ? 2 : 2 <= b.strategy || b.level < 2 ? 4 : 0), q(b, 3), b.status = R);
        else {
          var B = v + (b.w_bits - 8 << 4) << 8;
          B |= (2 <= b.strategy || b.level < 2 ? 0 : b.level < 6 ? 1 : b.level === 6 ? 2 : 3) << 6, b.strstart !== 0 && (B |= 32), B += 31 - B % 31, b.status = R, Y(b, B), b.strstart !== 0 && (Y(b, u.adler >>> 16), Y(b, 65535 & u.adler)), u.adler = 1;
        }
        if (b.status === 69) if (b.gzhead.extra) {
          for (g = b.pending; b.gzindex < (65535 & b.gzhead.extra.length) && (b.pending !== b.pending_buf_size || (b.gzhead.hcrc && b.pending > g && (u.adler = m(u.adler, b.pending_buf, b.pending - g, g)), C(u), g = b.pending, b.pending !== b.pending_buf_size)); ) q(b, 255 & b.gzhead.extra[b.gzindex]), b.gzindex++;
          b.gzhead.hcrc && b.pending > g && (u.adler = m(u.adler, b.pending_buf, b.pending - g, g)), b.gzindex === b.gzhead.extra.length && (b.gzindex = 0, b.status = 73);
        } else b.status = 73;
        if (b.status === 73) if (b.gzhead.name) {
          g = b.pending;
          do {
            if (b.pending === b.pending_buf_size && (b.gzhead.hcrc && b.pending > g && (u.adler = m(u.adler, b.pending_buf, b.pending - g, g)), C(u), g = b.pending, b.pending === b.pending_buf_size)) {
              E = 1;
              break;
            }
            E = b.gzindex < b.gzhead.name.length ? 255 & b.gzhead.name.charCodeAt(b.gzindex++) : 0, q(b, E);
          } while (E !== 0);
          b.gzhead.hcrc && b.pending > g && (u.adler = m(u.adler, b.pending_buf, b.pending - g, g)), E === 0 && (b.gzindex = 0, b.status = 91);
        } else b.status = 91;
        if (b.status === 91) if (b.gzhead.comment) {
          g = b.pending;
          do {
            if (b.pending === b.pending_buf_size && (b.gzhead.hcrc && b.pending > g && (u.adler = m(u.adler, b.pending_buf, b.pending - g, g)), C(u), g = b.pending, b.pending === b.pending_buf_size)) {
              E = 1;
              break;
            }
            E = b.gzindex < b.gzhead.comment.length ? 255 & b.gzhead.comment.charCodeAt(b.gzindex++) : 0, q(b, E);
          } while (E !== 0);
          b.gzhead.hcrc && b.pending > g && (u.adler = m(u.adler, b.pending_buf, b.pending - g, g)), E === 0 && (b.status = 103);
        } else b.status = 103;
        if (b.status === 103 && (b.gzhead.hcrc ? (b.pending + 2 > b.pending_buf_size && C(u), b.pending + 2 <= b.pending_buf_size && (q(b, 255 & u.adler), q(b, u.adler >> 8 & 255), u.adler = 0, b.status = R)) : b.status = R), b.pending !== 0) {
          if (C(u), u.avail_out === 0) return b.last_flush = -1, f;
        } else if (u.avail_in === 0 && M(F) <= M(Z) && F !== y) return nt(u, -5);
        if (b.status === 666 && u.avail_in !== 0) return nt(u, -5);
        if (u.avail_in !== 0 || b.lookahead !== 0 || F !== _ && b.status !== 666) {
          var j = b.strategy === 2 ? function(A, W) {
            for (var K; ; ) {
              if (A.lookahead === 0 && (ht(A), A.lookahead === 0)) {
                if (W === _) return c;
                break;
              }
              if (A.match_length = 0, K = a._tr_tally(A, 0, A.window[A.strstart]), A.lookahead--, A.strstart++, K && (O(A, !1), A.strm.avail_out === 0)) return c;
            }
            return A.insert = 0, W === y ? (O(A, !0), A.strm.avail_out === 0 ? et : U) : A.last_lit && (O(A, !1), A.strm.avail_out === 0) ? c : P;
          }(b, F) : b.strategy === 3 ? function(A, W) {
            for (var K, G, X, ot, it = A.window; ; ) {
              if (A.lookahead <= V) {
                if (ht(A), A.lookahead <= V && W === _) return c;
                if (A.lookahead === 0) break;
              }
              if (A.match_length = 0, A.lookahead >= $ && 0 < A.strstart && (G = it[X = A.strstart - 1]) === it[++X] && G === it[++X] && G === it[++X]) {
                ot = A.strstart + V;
                do
                  ;
                while (G === it[++X] && G === it[++X] && G === it[++X] && G === it[++X] && G === it[++X] && G === it[++X] && G === it[++X] && G === it[++X] && X < ot);
                A.match_length = V - (ot - X), A.match_length > A.lookahead && (A.match_length = A.lookahead);
              }
              if (A.match_length >= $ ? (K = a._tr_tally(A, 1, A.match_length - $), A.lookahead -= A.match_length, A.strstart += A.match_length, A.match_length = 0) : (K = a._tr_tally(A, 0, A.window[A.strstart]), A.lookahead--, A.strstart++), K && (O(A, !1), A.strm.avail_out === 0)) return c;
            }
            return A.insert = 0, W === y ? (O(A, !0), A.strm.avail_out === 0 ? et : U) : A.last_lit && (O(A, !1), A.strm.avail_out === 0) ? c : P;
          }(b, F) : s[b.level].func(b, F);
          if (j !== et && j !== U || (b.status = 666), j === c || j === et) return u.avail_out === 0 && (b.last_flush = -1), f;
          if (j === P && (F === 1 ? a._tr_align(b) : F !== 5 && (a._tr_stored_block(b, 0, 0, !1), F === 3 && (tt(b.head), b.lookahead === 0 && (b.strstart = 0, b.block_start = 0, b.insert = 0))), C(u), u.avail_out === 0)) return b.last_flush = -1, f;
        }
        return F !== y ? f : b.wrap <= 0 ? 1 : (b.wrap === 2 ? (q(b, 255 & u.adler), q(b, u.adler >> 8 & 255), q(b, u.adler >> 16 & 255), q(b, u.adler >> 24 & 255), q(b, 255 & u.total_in), q(b, u.total_in >> 8 & 255), q(b, u.total_in >> 16 & 255), q(b, u.total_in >> 24 & 255)) : (Y(b, u.adler >>> 16), Y(b, 65535 & u.adler)), C(u), 0 < b.wrap && (b.wrap = -b.wrap), b.pending !== 0 ? f : 1);
      }, i.deflateEnd = function(u) {
        var F;
        return u && u.state ? (F = u.state.status) !== x && F !== 69 && F !== 73 && F !== 91 && F !== 103 && F !== R && F !== 666 ? nt(u, w) : (u.state = null, F === R ? nt(u, -3) : f) : w;
      }, i.deflateSetDictionary = function(u, F) {
        var Z, b, g, E, B, j, A, W, K = F.length;
        if (!u || !u.state || (E = (Z = u.state).wrap) === 2 || E === 1 && Z.status !== x || Z.lookahead) return w;
        for (E === 1 && (u.adler = l(u.adler, F, K, 0)), Z.wrap = 0, K >= Z.w_size && (E === 0 && (tt(Z.head), Z.strstart = 0, Z.block_start = 0, Z.insert = 0), W = new o.Buf8(Z.w_size), o.arraySet(W, F, K - Z.w_size, Z.w_size, 0), F = W, K = Z.w_size), B = u.avail_in, j = u.next_in, A = u.input, u.avail_in = K, u.next_in = 0, u.input = F, ht(Z); Z.lookahead >= $; ) {
          for (b = Z.strstart, g = Z.lookahead - ($ - 1); Z.ins_h = (Z.ins_h << Z.hash_shift ^ Z.window[b + $ - 1]) & Z.hash_mask, Z.prev[b & Z.w_mask] = Z.head[Z.ins_h], Z.head[Z.ins_h] = b, b++, --g; ) ;
          Z.strstart = b, Z.lookahead = $ - 1, ht(Z);
        }
        return Z.strstart += Z.lookahead, Z.block_start = Z.strstart, Z.insert = Z.lookahead, Z.lookahead = 0, Z.match_length = Z.prev_length = $ - 1, Z.match_available = 0, u.next_in = j, u.input = A, u.avail_in = B, Z.wrap = E, f;
      }, i.deflateInfo = "pako deflate (from Nodeca project)";
    }, { "../utils/common": 41, "./adler32": 43, "./crc32": 45, "./messages": 51, "./trees": 52 }], 47: [function(n, r, i) {
      r.exports = function() {
        this.text = 0, this.time = 0, this.xflags = 0, this.os = 0, this.extra = null, this.extra_len = 0, this.name = "", this.comment = "", this.hcrc = 0, this.done = !1;
      };
    }, {}], 48: [function(n, r, i) {
      r.exports = function(s, o) {
        var a, l, m, k, _, y, f, w, h, p, d, v, z, I, S, D, T, L, $, V, Q, x, R, c, P;
        a = s.state, l = s.next_in, c = s.input, m = l + (s.avail_in - 5), k = s.next_out, P = s.output, _ = k - (o - s.avail_out), y = k + (s.avail_out - 257), f = a.dmax, w = a.wsize, h = a.whave, p = a.wnext, d = a.window, v = a.hold, z = a.bits, I = a.lencode, S = a.distcode, D = (1 << a.lenbits) - 1, T = (1 << a.distbits) - 1;
        t: do {
          z < 15 && (v += c[l++] << z, z += 8, v += c[l++] << z, z += 8), L = I[v & D];
          e: for (; ; ) {
            if (v >>>= $ = L >>> 24, z -= $, ($ = L >>> 16 & 255) === 0) P[k++] = 65535 & L;
            else {
              if (!(16 & $)) {
                if (!(64 & $)) {
                  L = I[(65535 & L) + (v & (1 << $) - 1)];
                  continue e;
                }
                if (32 & $) {
                  a.mode = 12;
                  break t;
                }
                s.msg = "invalid literal/length code", a.mode = 30;
                break t;
              }
              V = 65535 & L, ($ &= 15) && (z < $ && (v += c[l++] << z, z += 8), V += v & (1 << $) - 1, v >>>= $, z -= $), z < 15 && (v += c[l++] << z, z += 8, v += c[l++] << z, z += 8), L = S[v & T];
              n: for (; ; ) {
                if (v >>>= $ = L >>> 24, z -= $, !(16 & ($ = L >>> 16 & 255))) {
                  if (!(64 & $)) {
                    L = S[(65535 & L) + (v & (1 << $) - 1)];
                    continue n;
                  }
                  s.msg = "invalid distance code", a.mode = 30;
                  break t;
                }
                if (Q = 65535 & L, z < ($ &= 15) && (v += c[l++] << z, (z += 8) < $ && (v += c[l++] << z, z += 8)), f < (Q += v & (1 << $) - 1)) {
                  s.msg = "invalid distance too far back", a.mode = 30;
                  break t;
                }
                if (v >>>= $, z -= $, ($ = k - _) < Q) {
                  if (h < ($ = Q - $) && a.sane) {
                    s.msg = "invalid distance too far back", a.mode = 30;
                    break t;
                  }
                  if (R = d, (x = 0) === p) {
                    if (x += w - $, $ < V) {
                      for (V -= $; P[k++] = d[x++], --$; ) ;
                      x = k - Q, R = P;
                    }
                  } else if (p < $) {
                    if (x += w + p - $, ($ -= p) < V) {
                      for (V -= $; P[k++] = d[x++], --$; ) ;
                      if (x = 0, p < V) {
                        for (V -= $ = p; P[k++] = d[x++], --$; ) ;
                        x = k - Q, R = P;
                      }
                    }
                  } else if (x += p - $, $ < V) {
                    for (V -= $; P[k++] = d[x++], --$; ) ;
                    x = k - Q, R = P;
                  }
                  for (; 2 < V; ) P[k++] = R[x++], P[k++] = R[x++], P[k++] = R[x++], V -= 3;
                  V && (P[k++] = R[x++], 1 < V && (P[k++] = R[x++]));
                } else {
                  for (x = k - Q; P[k++] = P[x++], P[k++] = P[x++], P[k++] = P[x++], 2 < (V -= 3); ) ;
                  V && (P[k++] = P[x++], 1 < V && (P[k++] = P[x++]));
                }
                break;
              }
            }
            break;
          }
        } while (l < m && k < y);
        l -= V = z >> 3, v &= (1 << (z -= V << 3)) - 1, s.next_in = l, s.next_out = k, s.avail_in = l < m ? m - l + 5 : 5 - (l - m), s.avail_out = k < y ? y - k + 257 : 257 - (k - y), a.hold = v, a.bits = z;
      };
    }, {}], 49: [function(n, r, i) {
      var s = n("../utils/common"), o = n("./adler32"), a = n("./crc32"), l = n("./inffast"), m = n("./inftrees"), k = 1, _ = 2, y = 0, f = -2, w = 1, h = 852, p = 592;
      function d(x) {
        return (x >>> 24 & 255) + (x >>> 8 & 65280) + ((65280 & x) << 8) + ((255 & x) << 24);
      }
      function v() {
        this.mode = 0, this.last = !1, this.wrap = 0, this.havedict = !1, this.flags = 0, this.dmax = 0, this.check = 0, this.total = 0, this.head = null, this.wbits = 0, this.wsize = 0, this.whave = 0, this.wnext = 0, this.window = null, this.hold = 0, this.bits = 0, this.length = 0, this.offset = 0, this.extra = 0, this.lencode = null, this.distcode = null, this.lenbits = 0, this.distbits = 0, this.ncode = 0, this.nlen = 0, this.ndist = 0, this.have = 0, this.next = null, this.lens = new s.Buf16(320), this.work = new s.Buf16(288), this.lendyn = null, this.distdyn = null, this.sane = 0, this.back = 0, this.was = 0;
      }
      function z(x) {
        var R;
        return x && x.state ? (R = x.state, x.total_in = x.total_out = R.total = 0, x.msg = "", R.wrap && (x.adler = 1 & R.wrap), R.mode = w, R.last = 0, R.havedict = 0, R.dmax = 32768, R.head = null, R.hold = 0, R.bits = 0, R.lencode = R.lendyn = new s.Buf32(h), R.distcode = R.distdyn = new s.Buf32(p), R.sane = 1, R.back = -1, y) : f;
      }
      function I(x) {
        var R;
        return x && x.state ? ((R = x.state).wsize = 0, R.whave = 0, R.wnext = 0, z(x)) : f;
      }
      function S(x, R) {
        var c, P;
        return x && x.state ? (P = x.state, R < 0 ? (c = 0, R = -R) : (c = 1 + (R >> 4), R < 48 && (R &= 15)), R && (R < 8 || 15 < R) ? f : (P.window !== null && P.wbits !== R && (P.window = null), P.wrap = c, P.wbits = R, I(x))) : f;
      }
      function D(x, R) {
        var c, P;
        return x ? (P = new v(), (x.state = P).window = null, (c = S(x, R)) !== y && (x.state = null), c) : f;
      }
      var T, L, $ = !0;
      function V(x) {
        if ($) {
          var R;
          for (T = new s.Buf32(512), L = new s.Buf32(32), R = 0; R < 144; ) x.lens[R++] = 8;
          for (; R < 256; ) x.lens[R++] = 9;
          for (; R < 280; ) x.lens[R++] = 7;
          for (; R < 288; ) x.lens[R++] = 8;
          for (m(k, x.lens, 0, 288, T, 0, x.work, { bits: 9 }), R = 0; R < 32; ) x.lens[R++] = 5;
          m(_, x.lens, 0, 32, L, 0, x.work, { bits: 5 }), $ = !1;
        }
        x.lencode = T, x.lenbits = 9, x.distcode = L, x.distbits = 5;
      }
      function Q(x, R, c, P) {
        var et, U = x.state;
        return U.window === null && (U.wsize = 1 << U.wbits, U.wnext = 0, U.whave = 0, U.window = new s.Buf8(U.wsize)), P >= U.wsize ? (s.arraySet(U.window, R, c - U.wsize, U.wsize, 0), U.wnext = 0, U.whave = U.wsize) : (P < (et = U.wsize - U.wnext) && (et = P), s.arraySet(U.window, R, c - P, et, U.wnext), (P -= et) ? (s.arraySet(U.window, R, c - P, P, 0), U.wnext = P, U.whave = U.wsize) : (U.wnext += et, U.wnext === U.wsize && (U.wnext = 0), U.whave < U.wsize && (U.whave += et))), 0;
      }
      i.inflateReset = I, i.inflateReset2 = S, i.inflateResetKeep = z, i.inflateInit = function(x) {
        return D(x, 15);
      }, i.inflateInit2 = D, i.inflate = function(x, R) {
        var c, P, et, U, nt, M, tt, C, O, q, Y, H, ht, yt, rt, st, _t, dt, It, xt, u, F, Z, b, g = 0, E = new s.Buf8(4), B = [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15];
        if (!x || !x.state || !x.output || !x.input && x.avail_in !== 0) return f;
        (c = x.state).mode === 12 && (c.mode = 13), nt = x.next_out, et = x.output, tt = x.avail_out, U = x.next_in, P = x.input, M = x.avail_in, C = c.hold, O = c.bits, q = M, Y = tt, F = y;
        t: for (; ; ) switch (c.mode) {
          case w:
            if (c.wrap === 0) {
              c.mode = 13;
              break;
            }
            for (; O < 16; ) {
              if (M === 0) break t;
              M--, C += P[U++] << O, O += 8;
            }
            if (2 & c.wrap && C === 35615) {
              E[c.check = 0] = 255 & C, E[1] = C >>> 8 & 255, c.check = a(c.check, E, 2, 0), O = C = 0, c.mode = 2;
              break;
            }
            if (c.flags = 0, c.head && (c.head.done = !1), !(1 & c.wrap) || (((255 & C) << 8) + (C >> 8)) % 31) {
              x.msg = "incorrect header check", c.mode = 30;
              break;
            }
            if ((15 & C) != 8) {
              x.msg = "unknown compression method", c.mode = 30;
              break;
            }
            if (O -= 4, u = 8 + (15 & (C >>>= 4)), c.wbits === 0) c.wbits = u;
            else if (u > c.wbits) {
              x.msg = "invalid window size", c.mode = 30;
              break;
            }
            c.dmax = 1 << u, x.adler = c.check = 1, c.mode = 512 & C ? 10 : 12, O = C = 0;
            break;
          case 2:
            for (; O < 16; ) {
              if (M === 0) break t;
              M--, C += P[U++] << O, O += 8;
            }
            if (c.flags = C, (255 & c.flags) != 8) {
              x.msg = "unknown compression method", c.mode = 30;
              break;
            }
            if (57344 & c.flags) {
              x.msg = "unknown header flags set", c.mode = 30;
              break;
            }
            c.head && (c.head.text = C >> 8 & 1), 512 & c.flags && (E[0] = 255 & C, E[1] = C >>> 8 & 255, c.check = a(c.check, E, 2, 0)), O = C = 0, c.mode = 3;
          case 3:
            for (; O < 32; ) {
              if (M === 0) break t;
              M--, C += P[U++] << O, O += 8;
            }
            c.head && (c.head.time = C), 512 & c.flags && (E[0] = 255 & C, E[1] = C >>> 8 & 255, E[2] = C >>> 16 & 255, E[3] = C >>> 24 & 255, c.check = a(c.check, E, 4, 0)), O = C = 0, c.mode = 4;
          case 4:
            for (; O < 16; ) {
              if (M === 0) break t;
              M--, C += P[U++] << O, O += 8;
            }
            c.head && (c.head.xflags = 255 & C, c.head.os = C >> 8), 512 & c.flags && (E[0] = 255 & C, E[1] = C >>> 8 & 255, c.check = a(c.check, E, 2, 0)), O = C = 0, c.mode = 5;
          case 5:
            if (1024 & c.flags) {
              for (; O < 16; ) {
                if (M === 0) break t;
                M--, C += P[U++] << O, O += 8;
              }
              c.length = C, c.head && (c.head.extra_len = C), 512 & c.flags && (E[0] = 255 & C, E[1] = C >>> 8 & 255, c.check = a(c.check, E, 2, 0)), O = C = 0;
            } else c.head && (c.head.extra = null);
            c.mode = 6;
          case 6:
            if (1024 & c.flags && (M < (H = c.length) && (H = M), H && (c.head && (u = c.head.extra_len - c.length, c.head.extra || (c.head.extra = new Array(c.head.extra_len)), s.arraySet(c.head.extra, P, U, H, u)), 512 & c.flags && (c.check = a(c.check, P, H, U)), M -= H, U += H, c.length -= H), c.length)) break t;
            c.length = 0, c.mode = 7;
          case 7:
            if (2048 & c.flags) {
              if (M === 0) break t;
              for (H = 0; u = P[U + H++], c.head && u && c.length < 65536 && (c.head.name += String.fromCharCode(u)), u && H < M; ) ;
              if (512 & c.flags && (c.check = a(c.check, P, H, U)), M -= H, U += H, u) break t;
            } else c.head && (c.head.name = null);
            c.length = 0, c.mode = 8;
          case 8:
            if (4096 & c.flags) {
              if (M === 0) break t;
              for (H = 0; u = P[U + H++], c.head && u && c.length < 65536 && (c.head.comment += String.fromCharCode(u)), u && H < M; ) ;
              if (512 & c.flags && (c.check = a(c.check, P, H, U)), M -= H, U += H, u) break t;
            } else c.head && (c.head.comment = null);
            c.mode = 9;
          case 9:
            if (512 & c.flags) {
              for (; O < 16; ) {
                if (M === 0) break t;
                M--, C += P[U++] << O, O += 8;
              }
              if (C !== (65535 & c.check)) {
                x.msg = "header crc mismatch", c.mode = 30;
                break;
              }
              O = C = 0;
            }
            c.head && (c.head.hcrc = c.flags >> 9 & 1, c.head.done = !0), x.adler = c.check = 0, c.mode = 12;
            break;
          case 10:
            for (; O < 32; ) {
              if (M === 0) break t;
              M--, C += P[U++] << O, O += 8;
            }
            x.adler = c.check = d(C), O = C = 0, c.mode = 11;
          case 11:
            if (c.havedict === 0) return x.next_out = nt, x.avail_out = tt, x.next_in = U, x.avail_in = M, c.hold = C, c.bits = O, 2;
            x.adler = c.check = 1, c.mode = 12;
          case 12:
            if (R === 5 || R === 6) break t;
          case 13:
            if (c.last) {
              C >>>= 7 & O, O -= 7 & O, c.mode = 27;
              break;
            }
            for (; O < 3; ) {
              if (M === 0) break t;
              M--, C += P[U++] << O, O += 8;
            }
            switch (c.last = 1 & C, O -= 1, 3 & (C >>>= 1)) {
              case 0:
                c.mode = 14;
                break;
              case 1:
                if (V(c), c.mode = 20, R !== 6) break;
                C >>>= 2, O -= 2;
                break t;
              case 2:
                c.mode = 17;
                break;
              case 3:
                x.msg = "invalid block type", c.mode = 30;
            }
            C >>>= 2, O -= 2;
            break;
          case 14:
            for (C >>>= 7 & O, O -= 7 & O; O < 32; ) {
              if (M === 0) break t;
              M--, C += P[U++] << O, O += 8;
            }
            if ((65535 & C) != (C >>> 16 ^ 65535)) {
              x.msg = "invalid stored block lengths", c.mode = 30;
              break;
            }
            if (c.length = 65535 & C, O = C = 0, c.mode = 15, R === 6) break t;
          case 15:
            c.mode = 16;
          case 16:
            if (H = c.length) {
              if (M < H && (H = M), tt < H && (H = tt), H === 0) break t;
              s.arraySet(et, P, U, H, nt), M -= H, U += H, tt -= H, nt += H, c.length -= H;
              break;
            }
            c.mode = 12;
            break;
          case 17:
            for (; O < 14; ) {
              if (M === 0) break t;
              M--, C += P[U++] << O, O += 8;
            }
            if (c.nlen = 257 + (31 & C), C >>>= 5, O -= 5, c.ndist = 1 + (31 & C), C >>>= 5, O -= 5, c.ncode = 4 + (15 & C), C >>>= 4, O -= 4, 286 < c.nlen || 30 < c.ndist) {
              x.msg = "too many length or distance symbols", c.mode = 30;
              break;
            }
            c.have = 0, c.mode = 18;
          case 18:
            for (; c.have < c.ncode; ) {
              for (; O < 3; ) {
                if (M === 0) break t;
                M--, C += P[U++] << O, O += 8;
              }
              c.lens[B[c.have++]] = 7 & C, C >>>= 3, O -= 3;
            }
            for (; c.have < 19; ) c.lens[B[c.have++]] = 0;
            if (c.lencode = c.lendyn, c.lenbits = 7, Z = { bits: c.lenbits }, F = m(0, c.lens, 0, 19, c.lencode, 0, c.work, Z), c.lenbits = Z.bits, F) {
              x.msg = "invalid code lengths set", c.mode = 30;
              break;
            }
            c.have = 0, c.mode = 19;
          case 19:
            for (; c.have < c.nlen + c.ndist; ) {
              for (; st = (g = c.lencode[C & (1 << c.lenbits) - 1]) >>> 16 & 255, _t = 65535 & g, !((rt = g >>> 24) <= O); ) {
                if (M === 0) break t;
                M--, C += P[U++] << O, O += 8;
              }
              if (_t < 16) C >>>= rt, O -= rt, c.lens[c.have++] = _t;
              else {
                if (_t === 16) {
                  for (b = rt + 2; O < b; ) {
                    if (M === 0) break t;
                    M--, C += P[U++] << O, O += 8;
                  }
                  if (C >>>= rt, O -= rt, c.have === 0) {
                    x.msg = "invalid bit length repeat", c.mode = 30;
                    break;
                  }
                  u = c.lens[c.have - 1], H = 3 + (3 & C), C >>>= 2, O -= 2;
                } else if (_t === 17) {
                  for (b = rt + 3; O < b; ) {
                    if (M === 0) break t;
                    M--, C += P[U++] << O, O += 8;
                  }
                  O -= rt, u = 0, H = 3 + (7 & (C >>>= rt)), C >>>= 3, O -= 3;
                } else {
                  for (b = rt + 7; O < b; ) {
                    if (M === 0) break t;
                    M--, C += P[U++] << O, O += 8;
                  }
                  O -= rt, u = 0, H = 11 + (127 & (C >>>= rt)), C >>>= 7, O -= 7;
                }
                if (c.have + H > c.nlen + c.ndist) {
                  x.msg = "invalid bit length repeat", c.mode = 30;
                  break;
                }
                for (; H--; ) c.lens[c.have++] = u;
              }
            }
            if (c.mode === 30) break;
            if (c.lens[256] === 0) {
              x.msg = "invalid code -- missing end-of-block", c.mode = 30;
              break;
            }
            if (c.lenbits = 9, Z = { bits: c.lenbits }, F = m(k, c.lens, 0, c.nlen, c.lencode, 0, c.work, Z), c.lenbits = Z.bits, F) {
              x.msg = "invalid literal/lengths set", c.mode = 30;
              break;
            }
            if (c.distbits = 6, c.distcode = c.distdyn, Z = { bits: c.distbits }, F = m(_, c.lens, c.nlen, c.ndist, c.distcode, 0, c.work, Z), c.distbits = Z.bits, F) {
              x.msg = "invalid distances set", c.mode = 30;
              break;
            }
            if (c.mode = 20, R === 6) break t;
          case 20:
            c.mode = 21;
          case 21:
            if (6 <= M && 258 <= tt) {
              x.next_out = nt, x.avail_out = tt, x.next_in = U, x.avail_in = M, c.hold = C, c.bits = O, l(x, Y), nt = x.next_out, et = x.output, tt = x.avail_out, U = x.next_in, P = x.input, M = x.avail_in, C = c.hold, O = c.bits, c.mode === 12 && (c.back = -1);
              break;
            }
            for (c.back = 0; st = (g = c.lencode[C & (1 << c.lenbits) - 1]) >>> 16 & 255, _t = 65535 & g, !((rt = g >>> 24) <= O); ) {
              if (M === 0) break t;
              M--, C += P[U++] << O, O += 8;
            }
            if (st && !(240 & st)) {
              for (dt = rt, It = st, xt = _t; st = (g = c.lencode[xt + ((C & (1 << dt + It) - 1) >> dt)]) >>> 16 & 255, _t = 65535 & g, !(dt + (rt = g >>> 24) <= O); ) {
                if (M === 0) break t;
                M--, C += P[U++] << O, O += 8;
              }
              C >>>= dt, O -= dt, c.back += dt;
            }
            if (C >>>= rt, O -= rt, c.back += rt, c.length = _t, st === 0) {
              c.mode = 26;
              break;
            }
            if (32 & st) {
              c.back = -1, c.mode = 12;
              break;
            }
            if (64 & st) {
              x.msg = "invalid literal/length code", c.mode = 30;
              break;
            }
            c.extra = 15 & st, c.mode = 22;
          case 22:
            if (c.extra) {
              for (b = c.extra; O < b; ) {
                if (M === 0) break t;
                M--, C += P[U++] << O, O += 8;
              }
              c.length += C & (1 << c.extra) - 1, C >>>= c.extra, O -= c.extra, c.back += c.extra;
            }
            c.was = c.length, c.mode = 23;
          case 23:
            for (; st = (g = c.distcode[C & (1 << c.distbits) - 1]) >>> 16 & 255, _t = 65535 & g, !((rt = g >>> 24) <= O); ) {
              if (M === 0) break t;
              M--, C += P[U++] << O, O += 8;
            }
            if (!(240 & st)) {
              for (dt = rt, It = st, xt = _t; st = (g = c.distcode[xt + ((C & (1 << dt + It) - 1) >> dt)]) >>> 16 & 255, _t = 65535 & g, !(dt + (rt = g >>> 24) <= O); ) {
                if (M === 0) break t;
                M--, C += P[U++] << O, O += 8;
              }
              C >>>= dt, O -= dt, c.back += dt;
            }
            if (C >>>= rt, O -= rt, c.back += rt, 64 & st) {
              x.msg = "invalid distance code", c.mode = 30;
              break;
            }
            c.offset = _t, c.extra = 15 & st, c.mode = 24;
          case 24:
            if (c.extra) {
              for (b = c.extra; O < b; ) {
                if (M === 0) break t;
                M--, C += P[U++] << O, O += 8;
              }
              c.offset += C & (1 << c.extra) - 1, C >>>= c.extra, O -= c.extra, c.back += c.extra;
            }
            if (c.offset > c.dmax) {
              x.msg = "invalid distance too far back", c.mode = 30;
              break;
            }
            c.mode = 25;
          case 25:
            if (tt === 0) break t;
            if (H = Y - tt, c.offset > H) {
              if ((H = c.offset - H) > c.whave && c.sane) {
                x.msg = "invalid distance too far back", c.mode = 30;
                break;
              }
              ht = H > c.wnext ? (H -= c.wnext, c.wsize - H) : c.wnext - H, H > c.length && (H = c.length), yt = c.window;
            } else yt = et, ht = nt - c.offset, H = c.length;
            for (tt < H && (H = tt), tt -= H, c.length -= H; et[nt++] = yt[ht++], --H; ) ;
            c.length === 0 && (c.mode = 21);
            break;
          case 26:
            if (tt === 0) break t;
            et[nt++] = c.length, tt--, c.mode = 21;
            break;
          case 27:
            if (c.wrap) {
              for (; O < 32; ) {
                if (M === 0) break t;
                M--, C |= P[U++] << O, O += 8;
              }
              if (Y -= tt, x.total_out += Y, c.total += Y, Y && (x.adler = c.check = c.flags ? a(c.check, et, Y, nt - Y) : o(c.check, et, Y, nt - Y)), Y = tt, (c.flags ? C : d(C)) !== c.check) {
                x.msg = "incorrect data check", c.mode = 30;
                break;
              }
              O = C = 0;
            }
            c.mode = 28;
          case 28:
            if (c.wrap && c.flags) {
              for (; O < 32; ) {
                if (M === 0) break t;
                M--, C += P[U++] << O, O += 8;
              }
              if (C !== (4294967295 & c.total)) {
                x.msg = "incorrect length check", c.mode = 30;
                break;
              }
              O = C = 0;
            }
            c.mode = 29;
          case 29:
            F = 1;
            break t;
          case 30:
            F = -3;
            break t;
          case 31:
            return -4;
          case 32:
          default:
            return f;
        }
        return x.next_out = nt, x.avail_out = tt, x.next_in = U, x.avail_in = M, c.hold = C, c.bits = O, (c.wsize || Y !== x.avail_out && c.mode < 30 && (c.mode < 27 || R !== 4)) && Q(x, x.output, x.next_out, Y - x.avail_out) ? (c.mode = 31, -4) : (q -= x.avail_in, Y -= x.avail_out, x.total_in += q, x.total_out += Y, c.total += Y, c.wrap && Y && (x.adler = c.check = c.flags ? a(c.check, et, Y, x.next_out - Y) : o(c.check, et, Y, x.next_out - Y)), x.data_type = c.bits + (c.last ? 64 : 0) + (c.mode === 12 ? 128 : 0) + (c.mode === 20 || c.mode === 15 ? 256 : 0), (q == 0 && Y === 0 || R === 4) && F === y && (F = -5), F);
      }, i.inflateEnd = function(x) {
        if (!x || !x.state) return f;
        var R = x.state;
        return R.window && (R.window = null), x.state = null, y;
      }, i.inflateGetHeader = function(x, R) {
        var c;
        return x && x.state && 2 & (c = x.state).wrap ? ((c.head = R).done = !1, y) : f;
      }, i.inflateSetDictionary = function(x, R) {
        var c, P = R.length;
        return x && x.state ? (c = x.state).wrap !== 0 && c.mode !== 11 ? f : c.mode === 11 && o(1, R, P, 0) !== c.check ? -3 : Q(x, R, P, P) ? (c.mode = 31, -4) : (c.havedict = 1, y) : f;
      }, i.inflateInfo = "pako inflate (from Nodeca project)";
    }, { "../utils/common": 41, "./adler32": 43, "./crc32": 45, "./inffast": 48, "./inftrees": 50 }], 50: [function(n, r, i) {
      var s = n("../utils/common"), o = [3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131, 163, 195, 227, 258, 0, 0], a = [16, 16, 16, 16, 16, 16, 16, 16, 17, 17, 17, 17, 18, 18, 18, 18, 19, 19, 19, 19, 20, 20, 20, 20, 21, 21, 21, 21, 16, 72, 78], l = [1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537, 2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577, 0, 0], m = [16, 16, 16, 16, 17, 17, 18, 18, 19, 19, 20, 20, 21, 21, 22, 22, 23, 23, 24, 24, 25, 25, 26, 26, 27, 27, 28, 28, 29, 29, 64, 64];
      r.exports = function(k, _, y, f, w, h, p, d) {
        var v, z, I, S, D, T, L, $, V, Q = d.bits, x = 0, R = 0, c = 0, P = 0, et = 0, U = 0, nt = 0, M = 0, tt = 0, C = 0, O = null, q = 0, Y = new s.Buf16(16), H = new s.Buf16(16), ht = null, yt = 0;
        for (x = 0; x <= 15; x++) Y[x] = 0;
        for (R = 0; R < f; R++) Y[_[y + R]]++;
        for (et = Q, P = 15; 1 <= P && Y[P] === 0; P--) ;
        if (P < et && (et = P), P === 0) return w[h++] = 20971520, w[h++] = 20971520, d.bits = 1, 0;
        for (c = 1; c < P && Y[c] === 0; c++) ;
        for (et < c && (et = c), x = M = 1; x <= 15; x++) if (M <<= 1, (M -= Y[x]) < 0) return -1;
        if (0 < M && (k === 0 || P !== 1)) return -1;
        for (H[1] = 0, x = 1; x < 15; x++) H[x + 1] = H[x] + Y[x];
        for (R = 0; R < f; R++) _[y + R] !== 0 && (p[H[_[y + R]]++] = R);
        if (T = k === 0 ? (O = ht = p, 19) : k === 1 ? (O = o, q -= 257, ht = a, yt -= 257, 256) : (O = l, ht = m, -1), x = c, D = h, nt = R = C = 0, I = -1, S = (tt = 1 << (U = et)) - 1, k === 1 && 852 < tt || k === 2 && 592 < tt) return 1;
        for (; ; ) {
          for (L = x - nt, V = p[R] < T ? ($ = 0, p[R]) : p[R] > T ? ($ = ht[yt + p[R]], O[q + p[R]]) : ($ = 96, 0), v = 1 << x - nt, c = z = 1 << U; w[D + (C >> nt) + (z -= v)] = L << 24 | $ << 16 | V | 0, z !== 0; ) ;
          for (v = 1 << x - 1; C & v; ) v >>= 1;
          if (v !== 0 ? (C &= v - 1, C += v) : C = 0, R++, --Y[x] == 0) {
            if (x === P) break;
            x = _[y + p[R]];
          }
          if (et < x && (C & S) !== I) {
            for (nt === 0 && (nt = et), D += c, M = 1 << (U = x - nt); U + nt < P && !((M -= Y[U + nt]) <= 0); ) U++, M <<= 1;
            if (tt += 1 << U, k === 1 && 852 < tt || k === 2 && 592 < tt) return 1;
            w[I = C & S] = et << 24 | U << 16 | D - h | 0;
          }
        }
        return C !== 0 && (w[D + C] = x - nt << 24 | 64 << 16 | 0), d.bits = et, 0;
      };
    }, { "../utils/common": 41 }], 51: [function(n, r, i) {
      r.exports = { 2: "need dictionary", 1: "stream end", 0: "", "-1": "file error", "-2": "stream error", "-3": "data error", "-4": "insufficient memory", "-5": "buffer error", "-6": "incompatible version" };
    }, {}], 52: [function(n, r, i) {
      var s = n("../utils/common"), o = 0, a = 1;
      function l(g) {
        for (var E = g.length; 0 <= --E; ) g[E] = 0;
      }
      var m = 0, k = 29, _ = 256, y = _ + 1 + k, f = 30, w = 19, h = 2 * y + 1, p = 15, d = 16, v = 7, z = 256, I = 16, S = 17, D = 18, T = [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0], L = [0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13], $ = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 3, 7], V = [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15], Q = new Array(2 * (y + 2));
      l(Q);
      var x = new Array(2 * f);
      l(x);
      var R = new Array(512);
      l(R);
      var c = new Array(256);
      l(c);
      var P = new Array(k);
      l(P);
      var et, U, nt, M = new Array(f);
      function tt(g, E, B, j, A) {
        this.static_tree = g, this.extra_bits = E, this.extra_base = B, this.elems = j, this.max_length = A, this.has_stree = g && g.length;
      }
      function C(g, E) {
        this.dyn_tree = g, this.max_code = 0, this.stat_desc = E;
      }
      function O(g) {
        return g < 256 ? R[g] : R[256 + (g >>> 7)];
      }
      function q(g, E) {
        g.pending_buf[g.pending++] = 255 & E, g.pending_buf[g.pending++] = E >>> 8 & 255;
      }
      function Y(g, E, B) {
        g.bi_valid > d - B ? (g.bi_buf |= E << g.bi_valid & 65535, q(g, g.bi_buf), g.bi_buf = E >> d - g.bi_valid, g.bi_valid += B - d) : (g.bi_buf |= E << g.bi_valid & 65535, g.bi_valid += B);
      }
      function H(g, E, B) {
        Y(g, B[2 * E], B[2 * E + 1]);
      }
      function ht(g, E) {
        for (var B = 0; B |= 1 & g, g >>>= 1, B <<= 1, 0 < --E; ) ;
        return B >>> 1;
      }
      function yt(g, E, B) {
        var j, A, W = new Array(p + 1), K = 0;
        for (j = 1; j <= p; j++) W[j] = K = K + B[j - 1] << 1;
        for (A = 0; A <= E; A++) {
          var G = g[2 * A + 1];
          G !== 0 && (g[2 * A] = ht(W[G]++, G));
        }
      }
      function rt(g) {
        var E;
        for (E = 0; E < y; E++) g.dyn_ltree[2 * E] = 0;
        for (E = 0; E < f; E++) g.dyn_dtree[2 * E] = 0;
        for (E = 0; E < w; E++) g.bl_tree[2 * E] = 0;
        g.dyn_ltree[2 * z] = 1, g.opt_len = g.static_len = 0, g.last_lit = g.matches = 0;
      }
      function st(g) {
        8 < g.bi_valid ? q(g, g.bi_buf) : 0 < g.bi_valid && (g.pending_buf[g.pending++] = g.bi_buf), g.bi_buf = 0, g.bi_valid = 0;
      }
      function _t(g, E, B, j) {
        var A = 2 * E, W = 2 * B;
        return g[A] < g[W] || g[A] === g[W] && j[E] <= j[B];
      }
      function dt(g, E, B) {
        for (var j = g.heap[B], A = B << 1; A <= g.heap_len && (A < g.heap_len && _t(E, g.heap[A + 1], g.heap[A], g.depth) && A++, !_t(E, j, g.heap[A], g.depth)); ) g.heap[B] = g.heap[A], B = A, A <<= 1;
        g.heap[B] = j;
      }
      function It(g, E, B) {
        var j, A, W, K, G = 0;
        if (g.last_lit !== 0) for (; j = g.pending_buf[g.d_buf + 2 * G] << 8 | g.pending_buf[g.d_buf + 2 * G + 1], A = g.pending_buf[g.l_buf + G], G++, j === 0 ? H(g, A, E) : (H(g, (W = c[A]) + _ + 1, E), (K = T[W]) !== 0 && Y(g, A -= P[W], K), H(g, W = O(--j), B), (K = L[W]) !== 0 && Y(g, j -= M[W], K)), G < g.last_lit; ) ;
        H(g, z, E);
      }
      function xt(g, E) {
        var B, j, A, W = E.dyn_tree, K = E.stat_desc.static_tree, G = E.stat_desc.has_stree, X = E.stat_desc.elems, ot = -1;
        for (g.heap_len = 0, g.heap_max = h, B = 0; B < X; B++) W[2 * B] !== 0 ? (g.heap[++g.heap_len] = ot = B, g.depth[B] = 0) : W[2 * B + 1] = 0;
        for (; g.heap_len < 2; ) W[2 * (A = g.heap[++g.heap_len] = ot < 2 ? ++ot : 0)] = 1, g.depth[A] = 0, g.opt_len--, G && (g.static_len -= K[2 * A + 1]);
        for (E.max_code = ot, B = g.heap_len >> 1; 1 <= B; B--) dt(g, W, B);
        for (A = X; B = g.heap[1], g.heap[1] = g.heap[g.heap_len--], dt(g, W, 1), j = g.heap[1], g.heap[--g.heap_max] = B, g.heap[--g.heap_max] = j, W[2 * A] = W[2 * B] + W[2 * j], g.depth[A] = (g.depth[B] >= g.depth[j] ? g.depth[B] : g.depth[j]) + 1, W[2 * B + 1] = W[2 * j + 1] = A, g.heap[1] = A++, dt(g, W, 1), 2 <= g.heap_len; ) ;
        g.heap[--g.heap_max] = g.heap[1], function(it, kt) {
          var jt, St, Ut, pt, Yt, le, Ot = kt.dyn_tree, Ae = kt.max_code, $n = kt.stat_desc.static_tree, Tn = kt.stat_desc.has_stree, Rn = kt.stat_desc.extra_bits, Oe = kt.stat_desc.extra_base, Lt = kt.stat_desc.max_length, Kt = 0;
          for (pt = 0; pt <= p; pt++) it.bl_count[pt] = 0;
          for (Ot[2 * it.heap[it.heap_max] + 1] = 0, jt = it.heap_max + 1; jt < h; jt++) Lt < (pt = Ot[2 * Ot[2 * (St = it.heap[jt]) + 1] + 1] + 1) && (pt = Lt, Kt++), Ot[2 * St + 1] = pt, Ae < St || (it.bl_count[pt]++, Yt = 0, Oe <= St && (Yt = Rn[St - Oe]), le = Ot[2 * St], it.opt_len += le * (pt + Yt), Tn && (it.static_len += le * ($n[2 * St + 1] + Yt)));
          if (Kt !== 0) {
            do {
              for (pt = Lt - 1; it.bl_count[pt] === 0; ) pt--;
              it.bl_count[pt]--, it.bl_count[pt + 1] += 2, it.bl_count[Lt]--, Kt -= 2;
            } while (0 < Kt);
            for (pt = Lt; pt !== 0; pt--) for (St = it.bl_count[pt]; St !== 0; ) Ae < (Ut = it.heap[--jt]) || (Ot[2 * Ut + 1] !== pt && (it.opt_len += (pt - Ot[2 * Ut + 1]) * Ot[2 * Ut], Ot[2 * Ut + 1] = pt), St--);
          }
        }(g, E), yt(W, ot, g.bl_count);
      }
      function u(g, E, B) {
        var j, A, W = -1, K = E[1], G = 0, X = 7, ot = 4;
        for (K === 0 && (X = 138, ot = 3), E[2 * (B + 1) + 1] = 65535, j = 0; j <= B; j++) A = K, K = E[2 * (j + 1) + 1], ++G < X && A === K || (G < ot ? g.bl_tree[2 * A] += G : A !== 0 ? (A !== W && g.bl_tree[2 * A]++, g.bl_tree[2 * I]++) : G <= 10 ? g.bl_tree[2 * S]++ : g.bl_tree[2 * D]++, W = A, ot = (G = 0) === K ? (X = 138, 3) : A === K ? (X = 6, 3) : (X = 7, 4));
      }
      function F(g, E, B) {
        var j, A, W = -1, K = E[1], G = 0, X = 7, ot = 4;
        for (K === 0 && (X = 138, ot = 3), j = 0; j <= B; j++) if (A = K, K = E[2 * (j + 1) + 1], !(++G < X && A === K)) {
          if (G < ot) for (; H(g, A, g.bl_tree), --G != 0; ) ;
          else A !== 0 ? (A !== W && (H(g, A, g.bl_tree), G--), H(g, I, g.bl_tree), Y(g, G - 3, 2)) : G <= 10 ? (H(g, S, g.bl_tree), Y(g, G - 3, 3)) : (H(g, D, g.bl_tree), Y(g, G - 11, 7));
          W = A, ot = (G = 0) === K ? (X = 138, 3) : A === K ? (X = 6, 3) : (X = 7, 4);
        }
      }
      l(M);
      var Z = !1;
      function b(g, E, B, j) {
        Y(g, (m << 1) + (j ? 1 : 0), 3), function(A, W, K, G) {
          st(A), q(A, K), q(A, ~K), s.arraySet(A.pending_buf, A.window, W, K, A.pending), A.pending += K;
        }(g, E, B);
      }
      i._tr_init = function(g) {
        Z || (function() {
          var E, B, j, A, W, K = new Array(p + 1);
          for (A = j = 0; A < k - 1; A++) for (P[A] = j, E = 0; E < 1 << T[A]; E++) c[j++] = A;
          for (c[j - 1] = A, A = W = 0; A < 16; A++) for (M[A] = W, E = 0; E < 1 << L[A]; E++) R[W++] = A;
          for (W >>= 7; A < f; A++) for (M[A] = W << 7, E = 0; E < 1 << L[A] - 7; E++) R[256 + W++] = A;
          for (B = 0; B <= p; B++) K[B] = 0;
          for (E = 0; E <= 143; ) Q[2 * E + 1] = 8, E++, K[8]++;
          for (; E <= 255; ) Q[2 * E + 1] = 9, E++, K[9]++;
          for (; E <= 279; ) Q[2 * E + 1] = 7, E++, K[7]++;
          for (; E <= 287; ) Q[2 * E + 1] = 8, E++, K[8]++;
          for (yt(Q, y + 1, K), E = 0; E < f; E++) x[2 * E + 1] = 5, x[2 * E] = ht(E, 5);
          et = new tt(Q, T, _ + 1, y, p), U = new tt(x, L, 0, f, p), nt = new tt(new Array(0), $, 0, w, v);
        }(), Z = !0), g.l_desc = new C(g.dyn_ltree, et), g.d_desc = new C(g.dyn_dtree, U), g.bl_desc = new C(g.bl_tree, nt), g.bi_buf = 0, g.bi_valid = 0, rt(g);
      }, i._tr_stored_block = b, i._tr_flush_block = function(g, E, B, j) {
        var A, W, K = 0;
        0 < g.level ? (g.strm.data_type === 2 && (g.strm.data_type = function(G) {
          var X, ot = 4093624447;
          for (X = 0; X <= 31; X++, ot >>>= 1) if (1 & ot && G.dyn_ltree[2 * X] !== 0) return o;
          if (G.dyn_ltree[18] !== 0 || G.dyn_ltree[20] !== 0 || G.dyn_ltree[26] !== 0) return a;
          for (X = 32; X < _; X++) if (G.dyn_ltree[2 * X] !== 0) return a;
          return o;
        }(g)), xt(g, g.l_desc), xt(g, g.d_desc), K = function(G) {
          var X;
          for (u(G, G.dyn_ltree, G.l_desc.max_code), u(G, G.dyn_dtree, G.d_desc.max_code), xt(G, G.bl_desc), X = w - 1; 3 <= X && G.bl_tree[2 * V[X] + 1] === 0; X--) ;
          return G.opt_len += 3 * (X + 1) + 5 + 5 + 4, X;
        }(g), A = g.opt_len + 3 + 7 >>> 3, (W = g.static_len + 3 + 7 >>> 3) <= A && (A = W)) : A = W = B + 5, B + 4 <= A && E !== -1 ? b(g, E, B, j) : g.strategy === 4 || W === A ? (Y(g, 2 + (j ? 1 : 0), 3), It(g, Q, x)) : (Y(g, 4 + (j ? 1 : 0), 3), function(G, X, ot, it) {
          var kt;
          for (Y(G, X - 257, 5), Y(G, ot - 1, 5), Y(G, it - 4, 4), kt = 0; kt < it; kt++) Y(G, G.bl_tree[2 * V[kt] + 1], 3);
          F(G, G.dyn_ltree, X - 1), F(G, G.dyn_dtree, ot - 1);
        }(g, g.l_desc.max_code + 1, g.d_desc.max_code + 1, K + 1), It(g, g.dyn_ltree, g.dyn_dtree)), rt(g), j && st(g);
      }, i._tr_tally = function(g, E, B) {
        return g.pending_buf[g.d_buf + 2 * g.last_lit] = E >>> 8 & 255, g.pending_buf[g.d_buf + 2 * g.last_lit + 1] = 255 & E, g.pending_buf[g.l_buf + g.last_lit] = 255 & B, g.last_lit++, E === 0 ? g.dyn_ltree[2 * B]++ : (g.matches++, E--, g.dyn_ltree[2 * (c[B] + _ + 1)]++, g.dyn_dtree[2 * O(E)]++), g.last_lit === g.lit_bufsize - 1;
      }, i._tr_align = function(g) {
        Y(g, 2, 3), H(g, z, Q), function(E) {
          E.bi_valid === 16 ? (q(E, E.bi_buf), E.bi_buf = 0, E.bi_valid = 0) : 8 <= E.bi_valid && (E.pending_buf[E.pending++] = 255 & E.bi_buf, E.bi_buf >>= 8, E.bi_valid -= 8);
        }(g);
      };
    }, { "../utils/common": 41 }], 53: [function(n, r, i) {
      r.exports = function() {
        this.input = null, this.next_in = 0, this.avail_in = 0, this.total_in = 0, this.output = null, this.next_out = 0, this.avail_out = 0, this.total_out = 0, this.msg = "", this.state = null, this.data_type = 2, this.adler = 0;
      };
    }, {}], 54: [function(n, r, i) {
      (function(s) {
        (function(o, a) {
          if (!o.setImmediate) {
            var l, m, k, _, y = 1, f = {}, w = !1, h = o.document, p = Object.getPrototypeOf && Object.getPrototypeOf(o);
            p = p && p.setTimeout ? p : o, l = {}.toString.call(o.process) === "[object process]" ? function(I) {
              process.nextTick(function() {
                v(I);
              });
            } : function() {
              if (o.postMessage && !o.importScripts) {
                var I = !0, S = o.onmessage;
                return o.onmessage = function() {
                  I = !1;
                }, o.postMessage("", "*"), o.onmessage = S, I;
              }
            }() ? (_ = "setImmediate$" + Math.random() + "$", o.addEventListener ? o.addEventListener("message", z, !1) : o.attachEvent("onmessage", z), function(I) {
              o.postMessage(_ + I, "*");
            }) : o.MessageChannel ? ((k = new MessageChannel()).port1.onmessage = function(I) {
              v(I.data);
            }, function(I) {
              k.port2.postMessage(I);
            }) : h && "onreadystatechange" in h.createElement("script") ? (m = h.documentElement, function(I) {
              var S = h.createElement("script");
              S.onreadystatechange = function() {
                v(I), S.onreadystatechange = null, m.removeChild(S), S = null;
              }, m.appendChild(S);
            }) : function(I) {
              setTimeout(v, 0, I);
            }, p.setImmediate = function(I) {
              typeof I != "function" && (I = new Function("" + I));
              for (var S = new Array(arguments.length - 1), D = 0; D < S.length; D++) S[D] = arguments[D + 1];
              var T = { callback: I, args: S };
              return f[y] = T, l(y), y++;
            }, p.clearImmediate = d;
          }
          function d(I) {
            delete f[I];
          }
          function v(I) {
            if (w) setTimeout(v, 0, I);
            else {
              var S = f[I];
              if (S) {
                w = !0;
                try {
                  (function(D) {
                    var T = D.callback, L = D.args;
                    switch (L.length) {
                      case 0:
                        T();
                        break;
                      case 1:
                        T(L[0]);
                        break;
                      case 2:
                        T(L[0], L[1]);
                        break;
                      case 3:
                        T(L[0], L[1], L[2]);
                        break;
                      default:
                        T.apply(a, L);
                    }
                  })(S);
                } finally {
                  d(I), w = !1;
                }
              }
            }
          }
          function z(I) {
            I.source === o && typeof I.data == "string" && I.data.indexOf(_) === 0 && v(+I.data.slice(_.length));
          }
        })(typeof self > "u" ? s === void 0 ? this : s : self);
      }).call(this, typeof Jt < "u" ? Jt : typeof self < "u" ? self : typeof window < "u" ? window : {});
    }, {}] }, {}, [10])(10);
  });
})(en);
var Fn = en.exports;
const de = /* @__PURE__ */ Nn(Fn);
function N(t, e, n) {
  function r(a, l) {
    var m;
    Object.defineProperty(a, "_zod", {
      value: a._zod ?? {},
      enumerable: !1
    }), (m = a._zod).traits ?? (m.traits = /* @__PURE__ */ new Set()), a._zod.traits.add(t), e(a, l);
    for (const k in o.prototype)
      k in a || Object.defineProperty(a, k, { value: o.prototype[k].bind(a) });
    a._zod.constr = o, a._zod.def = l;
  }
  const i = n?.Parent ?? Object;
  class s extends i {
  }
  Object.defineProperty(s, "name", { value: t });
  function o(a) {
    var l;
    const m = n?.Parent ? new s() : this;
    r(m, a), (l = m._zod).deferred ?? (l.deferred = []);
    for (const k of m._zod.deferred)
      k();
    return m;
  }
  return Object.defineProperty(o, "init", { value: r }), Object.defineProperty(o, Symbol.hasInstance, {
    value: (a) => n?.Parent && a instanceof n.Parent ? !0 : a?._zod?.traits?.has(t)
  }), Object.defineProperty(o, "name", { value: t }), o;
}
class Vt extends Error {
  constructor() {
    super("Encountered Promise during synchronous parse. Use .parseAsync() instead.");
  }
}
const nn = {};
function Zt(t) {
  return nn;
}
function Dn(t) {
  const e = Object.values(t).filter((r) => typeof r == "number");
  return Object.entries(t).filter(([r, i]) => e.indexOf(+r) === -1).map(([r, i]) => i);
}
function pe(t, e) {
  return typeof e == "bigint" ? e.toString() : e;
}
function rn(t) {
  return {
    get value() {
      {
        const e = t();
        return Object.defineProperty(this, "value", { value: e }), e;
      }
    }
  };
}
function ye(t) {
  return t == null;
}
function we(t) {
  const e = t.startsWith("^") ? 1 : 0, n = t.endsWith("$") ? t.length - 1 : t.length;
  return t.slice(e, n);
}
function Pn(t, e) {
  const n = (t.toString().split(".")[1] || "").length, r = e.toString();
  let i = (r.split(".")[1] || "").length;
  if (i === 0 && /\d?e-\d?/.test(r)) {
    const l = r.match(/\d?e-(\d?)/);
    l?.[1] && (i = Number.parseInt(l[1]));
  }
  const s = n > i ? n : i, o = Number.parseInt(t.toFixed(s).replace(".", "")), a = Number.parseInt(e.toFixed(s).replace(".", ""));
  return o % a / 10 ** s;
}
const Ce = Symbol("evaluating");
function at(t, e, n) {
  let r;
  Object.defineProperty(t, e, {
    get() {
      if (r !== Ce)
        return r === void 0 && (r = Ce, r = n()), r;
    },
    set(i) {
      Object.defineProperty(t, e, {
        value: i
        // configurable: true,
      });
    },
    configurable: !0
  });
}
function Bn(t) {
  return Object.create(Object.getPrototypeOf(t), Object.getOwnPropertyDescriptors(t));
}
function Nt(t, e, n) {
  Object.defineProperty(t, e, {
    value: n,
    writable: !0,
    enumerable: !0,
    configurable: !0
  });
}
function Bt(...t) {
  const e = {};
  for (const n of t) {
    const r = Object.getOwnPropertyDescriptors(n);
    Object.assign(e, r);
  }
  return Object.defineProperties({}, e);
}
function Ze(t) {
  return JSON.stringify(t);
}
const sn = "captureStackTrace" in Error ? Error.captureStackTrace : (...t) => {
};
function me(t) {
  return typeof t == "object" && t !== null && !Array.isArray(t);
}
const jn = rn(() => {
  if (typeof navigator < "u" && navigator?.userAgent?.includes("Cloudflare"))
    return !1;
  try {
    const t = Function;
    return new t(""), !0;
  } catch {
    return !1;
  }
});
function re(t) {
  if (me(t) === !1)
    return !1;
  const e = t.constructor;
  if (e === void 0)
    return !0;
  const n = e.prototype;
  return !(me(n) === !1 || Object.prototype.hasOwnProperty.call(n, "isPrototypeOf") === !1);
}
const Un = /* @__PURE__ */ new Set(["string", "number", "symbol"]);
function oe(t) {
  return t.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}
function Ft(t, e, n) {
  const r = new t._zod.constr(e ?? t._zod.def);
  return (!e || n?.parent) && (r._zod.parent = t), r;
}
function J(t) {
  const e = t;
  if (!e)
    return {};
  if (typeof e == "string")
    return { error: () => e };
  if (e?.message !== void 0) {
    if (e?.error !== void 0)
      throw new Error("Cannot specify both `message` and `error` params");
    e.error = e.message;
  }
  return delete e.message, typeof e.error == "string" ? { ...e, error: () => e.error } : e;
}
function Ln(t) {
  return Object.keys(t).filter((e) => t[e]._zod.optin === "optional" && t[e]._zod.optout === "optional");
}
const Mn = {
  safeint: [Number.MIN_SAFE_INTEGER, Number.MAX_SAFE_INTEGER],
  int32: [-2147483648, 2147483647],
  uint32: [0, 4294967295],
  float32: [-34028234663852886e22, 34028234663852886e22],
  float64: [-Number.MAX_VALUE, Number.MAX_VALUE]
};
function Wn(t, e) {
  const n = t._zod.def, r = Bt(t._zod.def, {
    get shape() {
      const i = {};
      for (const s in e) {
        if (!(s in n.shape))
          throw new Error(`Unrecognized key: "${s}"`);
        e[s] && (i[s] = n.shape[s]);
      }
      return Nt(this, "shape", i), i;
    },
    checks: []
  });
  return Ft(t, r);
}
function Vn(t, e) {
  const n = t._zod.def, r = Bt(t._zod.def, {
    get shape() {
      const i = { ...t._zod.def.shape };
      for (const s in e) {
        if (!(s in n.shape))
          throw new Error(`Unrecognized key: "${s}"`);
        e[s] && delete i[s];
      }
      return Nt(this, "shape", i), i;
    },
    checks: []
  });
  return Ft(t, r);
}
function Gn(t, e) {
  if (!re(e))
    throw new Error("Invalid input to extend: expected a plain object");
  const n = Bt(t._zod.def, {
    get shape() {
      const r = { ...t._zod.def.shape, ...e };
      return Nt(this, "shape", r), r;
    },
    checks: []
  });
  return Ft(t, n);
}
function Hn(t, e) {
  const n = Bt(t._zod.def, {
    get shape() {
      const r = { ...t._zod.def.shape, ...e._zod.def.shape };
      return Nt(this, "shape", r), r;
    },
    get catchall() {
      return e._zod.def.catchall;
    },
    checks: []
    // delete existing checks
  });
  return Ft(t, n);
}
function Yn(t, e, n) {
  const r = Bt(e._zod.def, {
    get shape() {
      const i = e._zod.def.shape, s = { ...i };
      if (n)
        for (const o in n) {
          if (!(o in i))
            throw new Error(`Unrecognized key: "${o}"`);
          n[o] && (s[o] = t ? new t({
            type: "optional",
            innerType: i[o]
          }) : i[o]);
        }
      else
        for (const o in i)
          s[o] = t ? new t({
            type: "optional",
            innerType: i[o]
          }) : i[o];
      return Nt(this, "shape", s), s;
    },
    checks: []
  });
  return Ft(e, r);
}
function Kn(t, e, n) {
  const r = Bt(e._zod.def, {
    get shape() {
      const i = e._zod.def.shape, s = { ...i };
      if (n)
        for (const o in n) {
          if (!(o in s))
            throw new Error(`Unrecognized key: "${o}"`);
          n[o] && (s[o] = new t({
            type: "nonoptional",
            innerType: i[o]
          }));
        }
      else
        for (const o in i)
          s[o] = new t({
            type: "nonoptional",
            innerType: i[o]
          });
      return Nt(this, "shape", s), s;
    },
    checks: []
  });
  return Ft(e, r);
}
function Wt(t, e = 0) {
  for (let n = e; n < t.issues.length; n++)
    if (t.issues[n]?.continue !== !0)
      return !0;
  return !1;
}
function Pt(t, e) {
  return e.map((n) => {
    var r;
    return (r = n).path ?? (r.path = []), n.path.unshift(t), n;
  });
}
function qt(t) {
  return typeof t == "string" ? t : t?.message;
}
function $t(t, e, n) {
  const r = { ...t, path: t.path ?? [] };
  if (!t.message) {
    const i = qt(t.inst?._zod.def?.error?.(t)) ?? qt(e?.error?.(t)) ?? qt(n.customError?.(t)) ?? qt(n.localeError?.(t)) ?? "Invalid input";
    r.message = i;
  }
  return delete r.inst, delete r.continue, e?.reportInput || delete r.input, r;
}
function be(t) {
  return Array.isArray(t) ? "array" : typeof t == "string" ? "string" : "unknown";
}
function Gt(...t) {
  const [e, n, r] = t;
  return typeof e == "string" ? {
    message: e,
    code: "custom",
    input: n,
    inst: r
  } : { ...e };
}
const on = (t, e) => {
  t.name = "$ZodError", Object.defineProperty(t, "_zod", {
    value: t._zod,
    enumerable: !1
  }), Object.defineProperty(t, "issues", {
    value: e,
    enumerable: !1
  }), t.message = JSON.stringify(e, pe, 2), Object.defineProperty(t, "toString", {
    value: () => t.message,
    enumerable: !1
  });
}, an = N("$ZodError", on), un = N("$ZodError", on, { Parent: Error });
function Jn(t, e = (n) => n.message) {
  const n = {}, r = [];
  for (const i of t.issues)
    i.path.length > 0 ? (n[i.path[0]] = n[i.path[0]] || [], n[i.path[0]].push(e(i))) : r.push(e(i));
  return { formErrors: r, fieldErrors: n };
}
function Xn(t, e) {
  const n = e || function(s) {
    return s.message;
  }, r = { _errors: [] }, i = (s) => {
    for (const o of s.issues)
      if (o.code === "invalid_union" && o.errors.length)
        o.errors.map((a) => i({ issues: a }));
      else if (o.code === "invalid_key")
        i({ issues: o.issues });
      else if (o.code === "invalid_element")
        i({ issues: o.issues });
      else if (o.path.length === 0)
        r._errors.push(n(o));
      else {
        let a = r, l = 0;
        for (; l < o.path.length; ) {
          const m = o.path[l];
          l === o.path.length - 1 ? (a[m] = a[m] || { _errors: [] }, a[m]._errors.push(n(o))) : a[m] = a[m] || { _errors: [] }, a = a[m], l++;
        }
      }
  };
  return i(t), r;
}
const qn = (t) => (e, n, r, i) => {
  const s = r ? Object.assign(r, { async: !1 }) : { async: !1 }, o = e._zod.run({ value: n, issues: [] }, s);
  if (o instanceof Promise)
    throw new Vt();
  if (o.issues.length) {
    const a = new (i?.Err ?? t)(o.issues.map((l) => $t(l, s, Zt())));
    throw sn(a, i?.callee), a;
  }
  return o.value;
}, Qn = (t) => async (e, n, r, i) => {
  const s = r ? Object.assign(r, { async: !0 }) : { async: !0 };
  let o = e._zod.run({ value: n, issues: [] }, s);
  if (o instanceof Promise && (o = await o), o.issues.length) {
    const a = new (i?.Err ?? t)(o.issues.map((l) => $t(l, s, Zt())));
    throw sn(a, i?.callee), a;
  }
  return o.value;
}, cn = (t) => (e, n, r) => {
  const i = r ? { ...r, async: !1 } : { async: !1 }, s = e._zod.run({ value: n, issues: [] }, i);
  if (s instanceof Promise)
    throw new Vt();
  return s.issues.length ? {
    success: !1,
    error: new (t ?? an)(s.issues.map((o) => $t(o, i, Zt())))
  } : { success: !0, data: s.value };
}, tr = /* @__PURE__ */ cn(un), ln = (t) => async (e, n, r) => {
  const i = r ? Object.assign(r, { async: !0 }) : { async: !0 };
  let s = e._zod.run({ value: n, issues: [] }, i);
  return s instanceof Promise && (s = await s), s.issues.length ? {
    success: !1,
    error: new t(s.issues.map((o) => $t(o, i, Zt())))
  } : { success: !0, data: s.value };
}, er = /* @__PURE__ */ ln(un), nr = /^[cC][^\s-]{8,}$/, rr = /^[0-9a-z]+$/, ir = /^[0-9A-HJKMNP-TV-Za-hjkmnp-tv-z]{26}$/, sr = /^[0-9a-vA-V]{20}$/, or = /^[A-Za-z0-9]{27}$/, ar = /^[a-zA-Z0-9_-]{21}$/, ur = /^P(?:(\d+W)|(?!.*W)(?=\d|T\d)(\d+Y)?(\d+M)?(\d+D)?(T(?=\d)(\d+H)?(\d+M)?(\d+([.,]\d+)?S)?)?)$/, cr = /^([0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12})$/, $e = (t) => t ? new RegExp(`^([0-9a-fA-F]{8}-[0-9a-fA-F]{4}-${t}[0-9a-fA-F]{3}-[89abAB][0-9a-fA-F]{3}-[0-9a-fA-F]{12})$`) : /^([0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[1-8][0-9a-fA-F]{3}-[89abAB][0-9a-fA-F]{3}-[0-9a-fA-F]{12}|00000000-0000-0000-0000-000000000000)$/, lr = /^(?!\.)(?!.*\.\.)([A-Za-z0-9_'+\-\.]*)[A-Za-z0-9_+-]@([A-Za-z0-9][A-Za-z0-9\-]*\.)+[A-Za-z]{2,}$/, fr = "^(\\p{Extended_Pictographic}|\\p{Emoji_Component})+$";
function hr() {
  return new RegExp(fr, "u");
}
const dr = /^(?:(?:25[0-5]|2[0-4][0-9]|1[0-9][0-9]|[1-9][0-9]|[0-9])\.){3}(?:25[0-5]|2[0-4][0-9]|1[0-9][0-9]|[1-9][0-9]|[0-9])$/, pr = /^(([0-9a-fA-F]{1,4}:){7}[0-9a-fA-F]{1,4}|::|([0-9a-fA-F]{1,4})?::([0-9a-fA-F]{1,4}:?){0,6})$/, mr = /^((25[0-5]|2[0-4][0-9]|1[0-9][0-9]|[1-9][0-9]|[0-9])\.){3}(25[0-5]|2[0-4][0-9]|1[0-9][0-9]|[1-9][0-9]|[0-9])\/([0-9]|[1-2][0-9]|3[0-2])$/, _r = /^(([0-9a-fA-F]{1,4}:){7}[0-9a-fA-F]{1,4}|::|([0-9a-fA-F]{1,4})?::([0-9a-fA-F]{1,4}:?){0,6})\/(12[0-8]|1[01][0-9]|[1-9]?[0-9])$/, gr = /^$|^(?:[0-9a-zA-Z+/]{4})*(?:(?:[0-9a-zA-Z+/]{2}==)|(?:[0-9a-zA-Z+/]{3}=))?$/, fn = /^[A-Za-z0-9_-]*$/, vr = /^(?=.{1,253}\.?$)[a-zA-Z0-9](?:[a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?(?:\.[a-zA-Z0-9](?:[-0-9a-zA-Z]{0,61}[0-9a-zA-Z])?)*\.?$/, yr = /^\+(?:[0-9]){6,14}[0-9]$/, hn = "(?:(?:\\d\\d[2468][048]|\\d\\d[13579][26]|\\d\\d0[48]|[02468][048]00|[13579][26]00)-02-29|\\d{4}-(?:(?:0[13578]|1[02])-(?:0[1-9]|[12]\\d|3[01])|(?:0[469]|11)-(?:0[1-9]|[12]\\d|30)|(?:02)-(?:0[1-9]|1\\d|2[0-8])))", wr = /* @__PURE__ */ new RegExp(`^${hn}$`);
function dn(t) {
  const e = "(?:[01]\\d|2[0-3]):[0-5]\\d";
  return typeof t.precision == "number" ? t.precision === -1 ? `${e}` : t.precision === 0 ? `${e}:[0-5]\\d` : `${e}:[0-5]\\d\\.\\d{${t.precision}}` : `${e}(?::[0-5]\\d(?:\\.\\d+)?)?`;
}
function br(t) {
  return new RegExp(`^${dn(t)}$`);
}
function kr(t) {
  const e = dn({ precision: t.precision }), n = ["Z"];
  t.local && n.push(""), t.offset && n.push("([+-](?:[01]\\d|2[0-3]):[0-5]\\d)");
  const r = `${e}(?:${n.join("|")})`;
  return new RegExp(`^${hn}T(?:${r})$`);
}
const zr = (t) => {
  const e = t ? `[\\s\\S]{${t?.minimum ?? 0},${t?.maximum ?? ""}}` : "[\\s\\S]*";
  return new RegExp(`^${e}$`);
}, xr = /^\d+$/, Er = /^-?\d+(?:\.\d+)?/i, Ir = /true|false/i, Sr = /^[^A-Z]*$/, Ar = /^[^a-z]*$/, wt = /* @__PURE__ */ N("$ZodCheck", (t, e) => {
  var n;
  t._zod ?? (t._zod = {}), t._zod.def = e, (n = t._zod).onattach ?? (n.onattach = []);
}), pn = {
  number: "number",
  bigint: "bigint",
  object: "date"
}, mn = /* @__PURE__ */ N("$ZodCheckLessThan", (t, e) => {
  wt.init(t, e);
  const n = pn[typeof e.value];
  t._zod.onattach.push((r) => {
    const i = r._zod.bag, s = (e.inclusive ? i.maximum : i.exclusiveMaximum) ?? Number.POSITIVE_INFINITY;
    e.value < s && (e.inclusive ? i.maximum = e.value : i.exclusiveMaximum = e.value);
  }), t._zod.check = (r) => {
    (e.inclusive ? r.value <= e.value : r.value < e.value) || r.issues.push({
      origin: n,
      code: "too_big",
      maximum: e.value,
      input: r.value,
      inclusive: e.inclusive,
      inst: t,
      continue: !e.abort
    });
  };
}), _n = /* @__PURE__ */ N("$ZodCheckGreaterThan", (t, e) => {
  wt.init(t, e);
  const n = pn[typeof e.value];
  t._zod.onattach.push((r) => {
    const i = r._zod.bag, s = (e.inclusive ? i.minimum : i.exclusiveMinimum) ?? Number.NEGATIVE_INFINITY;
    e.value > s && (e.inclusive ? i.minimum = e.value : i.exclusiveMinimum = e.value);
  }), t._zod.check = (r) => {
    (e.inclusive ? r.value >= e.value : r.value > e.value) || r.issues.push({
      origin: n,
      code: "too_small",
      minimum: e.value,
      input: r.value,
      inclusive: e.inclusive,
      inst: t,
      continue: !e.abort
    });
  };
}), Or = /* @__PURE__ */ N("$ZodCheckMultipleOf", (t, e) => {
  wt.init(t, e), t._zod.onattach.push((n) => {
    var r;
    (r = n._zod.bag).multipleOf ?? (r.multipleOf = e.value);
  }), t._zod.check = (n) => {
    if (typeof n.value != typeof e.value)
      throw new Error("Cannot mix number and bigint in multiple_of check.");
    (typeof n.value == "bigint" ? n.value % e.value === BigInt(0) : Pn(n.value, e.value) === 0) || n.issues.push({
      origin: typeof n.value,
      code: "not_multiple_of",
      divisor: e.value,
      input: n.value,
      inst: t,
      continue: !e.abort
    });
  };
}), Cr = /* @__PURE__ */ N("$ZodCheckNumberFormat", (t, e) => {
  wt.init(t, e), e.format = e.format || "float64";
  const n = e.format?.includes("int"), r = n ? "int" : "number", [i, s] = Mn[e.format];
  t._zod.onattach.push((o) => {
    const a = o._zod.bag;
    a.format = e.format, a.minimum = i, a.maximum = s, n && (a.pattern = xr);
  }), t._zod.check = (o) => {
    const a = o.value;
    if (n) {
      if (!Number.isInteger(a)) {
        o.issues.push({
          expected: r,
          format: e.format,
          code: "invalid_type",
          continue: !1,
          input: a,
          inst: t
        });
        return;
      }
      if (!Number.isSafeInteger(a)) {
        a > 0 ? o.issues.push({
          input: a,
          code: "too_big",
          maximum: Number.MAX_SAFE_INTEGER,
          note: "Integers must be within the safe integer range.",
          inst: t,
          origin: r,
          continue: !e.abort
        }) : o.issues.push({
          input: a,
          code: "too_small",
          minimum: Number.MIN_SAFE_INTEGER,
          note: "Integers must be within the safe integer range.",
          inst: t,
          origin: r,
          continue: !e.abort
        });
        return;
      }
    }
    a < i && o.issues.push({
      origin: "number",
      input: a,
      code: "too_small",
      minimum: i,
      inclusive: !0,
      inst: t,
      continue: !e.abort
    }), a > s && o.issues.push({
      origin: "number",
      input: a,
      code: "too_big",
      maximum: s,
      inst: t
    });
  };
}), Zr = /* @__PURE__ */ N("$ZodCheckMaxLength", (t, e) => {
  var n;
  wt.init(t, e), (n = t._zod.def).when ?? (n.when = (r) => {
    const i = r.value;
    return !ye(i) && i.length !== void 0;
  }), t._zod.onattach.push((r) => {
    const i = r._zod.bag.maximum ?? Number.POSITIVE_INFINITY;
    e.maximum < i && (r._zod.bag.maximum = e.maximum);
  }), t._zod.check = (r) => {
    const i = r.value;
    if (i.length <= e.maximum)
      return;
    const o = be(i);
    r.issues.push({
      origin: o,
      code: "too_big",
      maximum: e.maximum,
      inclusive: !0,
      input: i,
      inst: t,
      continue: !e.abort
    });
  };
}), $r = /* @__PURE__ */ N("$ZodCheckMinLength", (t, e) => {
  var n;
  wt.init(t, e), (n = t._zod.def).when ?? (n.when = (r) => {
    const i = r.value;
    return !ye(i) && i.length !== void 0;
  }), t._zod.onattach.push((r) => {
    const i = r._zod.bag.minimum ?? Number.NEGATIVE_INFINITY;
    e.minimum > i && (r._zod.bag.minimum = e.minimum);
  }), t._zod.check = (r) => {
    const i = r.value;
    if (i.length >= e.minimum)
      return;
    const o = be(i);
    r.issues.push({
      origin: o,
      code: "too_small",
      minimum: e.minimum,
      inclusive: !0,
      input: i,
      inst: t,
      continue: !e.abort
    });
  };
}), Tr = /* @__PURE__ */ N("$ZodCheckLengthEquals", (t, e) => {
  var n;
  wt.init(t, e), (n = t._zod.def).when ?? (n.when = (r) => {
    const i = r.value;
    return !ye(i) && i.length !== void 0;
  }), t._zod.onattach.push((r) => {
    const i = r._zod.bag;
    i.minimum = e.length, i.maximum = e.length, i.length = e.length;
  }), t._zod.check = (r) => {
    const i = r.value, s = i.length;
    if (s === e.length)
      return;
    const o = be(i), a = s > e.length;
    r.issues.push({
      origin: o,
      ...a ? { code: "too_big", maximum: e.length } : { code: "too_small", minimum: e.length },
      inclusive: !0,
      exact: !0,
      input: r.value,
      inst: t,
      continue: !e.abort
    });
  };
}), ae = /* @__PURE__ */ N("$ZodCheckStringFormat", (t, e) => {
  var n, r;
  wt.init(t, e), t._zod.onattach.push((i) => {
    const s = i._zod.bag;
    s.format = e.format, e.pattern && (s.patterns ?? (s.patterns = /* @__PURE__ */ new Set()), s.patterns.add(e.pattern));
  }), e.pattern ? (n = t._zod).check ?? (n.check = (i) => {
    e.pattern.lastIndex = 0, !e.pattern.test(i.value) && i.issues.push({
      origin: "string",
      code: "invalid_format",
      format: e.format,
      input: i.value,
      ...e.pattern ? { pattern: e.pattern.toString() } : {},
      inst: t,
      continue: !e.abort
    });
  }) : (r = t._zod).check ?? (r.check = () => {
  });
}), Rr = /* @__PURE__ */ N("$ZodCheckRegex", (t, e) => {
  ae.init(t, e), t._zod.check = (n) => {
    e.pattern.lastIndex = 0, !e.pattern.test(n.value) && n.issues.push({
      origin: "string",
      code: "invalid_format",
      format: "regex",
      input: n.value,
      pattern: e.pattern.toString(),
      inst: t,
      continue: !e.abort
    });
  };
}), Nr = /* @__PURE__ */ N("$ZodCheckLowerCase", (t, e) => {
  e.pattern ?? (e.pattern = Sr), ae.init(t, e);
}), Fr = /* @__PURE__ */ N("$ZodCheckUpperCase", (t, e) => {
  e.pattern ?? (e.pattern = Ar), ae.init(t, e);
}), Dr = /* @__PURE__ */ N("$ZodCheckIncludes", (t, e) => {
  wt.init(t, e);
  const n = oe(e.includes), r = new RegExp(typeof e.position == "number" ? `^.{${e.position}}${n}` : n);
  e.pattern = r, t._zod.onattach.push((i) => {
    const s = i._zod.bag;
    s.patterns ?? (s.patterns = /* @__PURE__ */ new Set()), s.patterns.add(r);
  }), t._zod.check = (i) => {
    i.value.includes(e.includes, e.position) || i.issues.push({
      origin: "string",
      code: "invalid_format",
      format: "includes",
      includes: e.includes,
      input: i.value,
      inst: t,
      continue: !e.abort
    });
  };
}), Pr = /* @__PURE__ */ N("$ZodCheckStartsWith", (t, e) => {
  wt.init(t, e);
  const n = new RegExp(`^${oe(e.prefix)}.*`);
  e.pattern ?? (e.pattern = n), t._zod.onattach.push((r) => {
    const i = r._zod.bag;
    i.patterns ?? (i.patterns = /* @__PURE__ */ new Set()), i.patterns.add(n);
  }), t._zod.check = (r) => {
    r.value.startsWith(e.prefix) || r.issues.push({
      origin: "string",
      code: "invalid_format",
      format: "starts_with",
      prefix: e.prefix,
      input: r.value,
      inst: t,
      continue: !e.abort
    });
  };
}), Br = /* @__PURE__ */ N("$ZodCheckEndsWith", (t, e) => {
  wt.init(t, e);
  const n = new RegExp(`.*${oe(e.suffix)}$`);
  e.pattern ?? (e.pattern = n), t._zod.onattach.push((r) => {
    const i = r._zod.bag;
    i.patterns ?? (i.patterns = /* @__PURE__ */ new Set()), i.patterns.add(n);
  }), t._zod.check = (r) => {
    r.value.endsWith(e.suffix) || r.issues.push({
      origin: "string",
      code: "invalid_format",
      format: "ends_with",
      suffix: e.suffix,
      input: r.value,
      inst: t,
      continue: !e.abort
    });
  };
}), jr = /* @__PURE__ */ N("$ZodCheckOverwrite", (t, e) => {
  wt.init(t, e), t._zod.check = (n) => {
    n.value = e.tx(n.value);
  };
});
class Ur {
  constructor(e = []) {
    this.content = [], this.indent = 0, this && (this.args = e);
  }
  indented(e) {
    this.indent += 1, e(this), this.indent -= 1;
  }
  write(e) {
    if (typeof e == "function") {
      e(this, { execution: "sync" }), e(this, { execution: "async" });
      return;
    }
    const r = e.split(`
`).filter((o) => o), i = Math.min(...r.map((o) => o.length - o.trimStart().length)), s = r.map((o) => o.slice(i)).map((o) => " ".repeat(this.indent * 2) + o);
    for (const o of s)
      this.content.push(o);
  }
  compile() {
    const e = Function, n = this?.args, i = [...(this?.content ?? [""]).map((s) => `  ${s}`)];
    return new e(...n, i.join(`
`));
  }
}
const Lr = {
  major: 4,
  minor: 0,
  patch: 15
}, ut = /* @__PURE__ */ N("$ZodType", (t, e) => {
  var n;
  t ?? (t = {}), t._zod.def = e, t._zod.bag = t._zod.bag || {}, t._zod.version = Lr;
  const r = [...t._zod.def.checks ?? []];
  t._zod.traits.has("$ZodCheck") && r.unshift(t);
  for (const i of r)
    for (const s of i._zod.onattach)
      s(t);
  if (r.length === 0)
    (n = t._zod).deferred ?? (n.deferred = []), t._zod.deferred?.push(() => {
      t._zod.run = t._zod.parse;
    });
  else {
    const i = (s, o, a) => {
      let l = Wt(s), m;
      for (const k of o) {
        if (k._zod.def.when) {
          if (!k._zod.def.when(s))
            continue;
        } else if (l)
          continue;
        const _ = s.issues.length, y = k._zod.check(s);
        if (y instanceof Promise && a?.async === !1)
          throw new Vt();
        if (m || y instanceof Promise)
          m = (m ?? Promise.resolve()).then(async () => {
            await y, s.issues.length !== _ && (l || (l = Wt(s, _)));
          });
        else {
          if (s.issues.length === _)
            continue;
          l || (l = Wt(s, _));
        }
      }
      return m ? m.then(() => s) : s;
    };
    t._zod.run = (s, o) => {
      const a = t._zod.parse(s, o);
      if (a instanceof Promise) {
        if (o.async === !1)
          throw new Vt();
        return a.then((l) => i(l, r, o));
      }
      return i(a, r, o);
    };
  }
  t["~standard"] = {
    validate: (i) => {
      try {
        const s = tr(t, i);
        return s.success ? { value: s.data } : { issues: s.error?.issues };
      } catch {
        return er(t, i).then((o) => o.success ? { value: o.data } : { issues: o.error?.issues });
      }
    },
    vendor: "zod",
    version: 1
  };
}), ke = /* @__PURE__ */ N("$ZodString", (t, e) => {
  ut.init(t, e), t._zod.pattern = [...t?._zod.bag?.patterns ?? []].pop() ?? zr(t._zod.bag), t._zod.parse = (n, r) => {
    if (e.coerce)
      try {
        n.value = String(n.value);
      } catch {
      }
    return typeof n.value == "string" || n.issues.push({
      expected: "string",
      code: "invalid_type",
      input: n.value,
      inst: t
    }), n;
  };
}), ct = /* @__PURE__ */ N("$ZodStringFormat", (t, e) => {
  ae.init(t, e), ke.init(t, e);
}), Mr = /* @__PURE__ */ N("$ZodGUID", (t, e) => {
  e.pattern ?? (e.pattern = cr), ct.init(t, e);
}), Wr = /* @__PURE__ */ N("$ZodUUID", (t, e) => {
  if (e.version) {
    const r = {
      v1: 1,
      v2: 2,
      v3: 3,
      v4: 4,
      v5: 5,
      v6: 6,
      v7: 7,
      v8: 8
    }[e.version];
    if (r === void 0)
      throw new Error(`Invalid UUID version: "${e.version}"`);
    e.pattern ?? (e.pattern = $e(r));
  } else
    e.pattern ?? (e.pattern = $e());
  ct.init(t, e);
}), Vr = /* @__PURE__ */ N("$ZodEmail", (t, e) => {
  e.pattern ?? (e.pattern = lr), ct.init(t, e);
}), Gr = /* @__PURE__ */ N("$ZodURL", (t, e) => {
  ct.init(t, e), t._zod.check = (n) => {
    try {
      const r = n.value.trim(), i = new URL(r);
      e.hostname && (e.hostname.lastIndex = 0, e.hostname.test(i.hostname) || n.issues.push({
        code: "invalid_format",
        format: "url",
        note: "Invalid hostname",
        pattern: vr.source,
        input: n.value,
        inst: t,
        continue: !e.abort
      })), e.protocol && (e.protocol.lastIndex = 0, e.protocol.test(i.protocol.endsWith(":") ? i.protocol.slice(0, -1) : i.protocol) || n.issues.push({
        code: "invalid_format",
        format: "url",
        note: "Invalid protocol",
        pattern: e.protocol.source,
        input: n.value,
        inst: t,
        continue: !e.abort
      })), e.normalize ? n.value = i.href : n.value = r;
      return;
    } catch {
      n.issues.push({
        code: "invalid_format",
        format: "url",
        input: n.value,
        inst: t,
        continue: !e.abort
      });
    }
  };
}), Hr = /* @__PURE__ */ N("$ZodEmoji", (t, e) => {
  e.pattern ?? (e.pattern = hr()), ct.init(t, e);
}), Yr = /* @__PURE__ */ N("$ZodNanoID", (t, e) => {
  e.pattern ?? (e.pattern = ar), ct.init(t, e);
}), Kr = /* @__PURE__ */ N("$ZodCUID", (t, e) => {
  e.pattern ?? (e.pattern = nr), ct.init(t, e);
}), Jr = /* @__PURE__ */ N("$ZodCUID2", (t, e) => {
  e.pattern ?? (e.pattern = rr), ct.init(t, e);
}), Xr = /* @__PURE__ */ N("$ZodULID", (t, e) => {
  e.pattern ?? (e.pattern = ir), ct.init(t, e);
}), qr = /* @__PURE__ */ N("$ZodXID", (t, e) => {
  e.pattern ?? (e.pattern = sr), ct.init(t, e);
}), Qr = /* @__PURE__ */ N("$ZodKSUID", (t, e) => {
  e.pattern ?? (e.pattern = or), ct.init(t, e);
}), ti = /* @__PURE__ */ N("$ZodISODateTime", (t, e) => {
  e.pattern ?? (e.pattern = kr(e)), ct.init(t, e);
}), ei = /* @__PURE__ */ N("$ZodISODate", (t, e) => {
  e.pattern ?? (e.pattern = wr), ct.init(t, e);
}), ni = /* @__PURE__ */ N("$ZodISOTime", (t, e) => {
  e.pattern ?? (e.pattern = br(e)), ct.init(t, e);
}), ri = /* @__PURE__ */ N("$ZodISODuration", (t, e) => {
  e.pattern ?? (e.pattern = ur), ct.init(t, e);
}), ii = /* @__PURE__ */ N("$ZodIPv4", (t, e) => {
  e.pattern ?? (e.pattern = dr), ct.init(t, e), t._zod.onattach.push((n) => {
    const r = n._zod.bag;
    r.format = "ipv4";
  });
}), si = /* @__PURE__ */ N("$ZodIPv6", (t, e) => {
  e.pattern ?? (e.pattern = pr), ct.init(t, e), t._zod.onattach.push((n) => {
    const r = n._zod.bag;
    r.format = "ipv6";
  }), t._zod.check = (n) => {
    try {
      new URL(`http://[${n.value}]`);
    } catch {
      n.issues.push({
        code: "invalid_format",
        format: "ipv6",
        input: n.value,
        inst: t,
        continue: !e.abort
      });
    }
  };
}), oi = /* @__PURE__ */ N("$ZodCIDRv4", (t, e) => {
  e.pattern ?? (e.pattern = mr), ct.init(t, e);
}), ai = /* @__PURE__ */ N("$ZodCIDRv6", (t, e) => {
  e.pattern ?? (e.pattern = _r), ct.init(t, e), t._zod.check = (n) => {
    const [r, i] = n.value.split("/");
    try {
      if (!i)
        throw new Error();
      const s = Number(i);
      if (`${s}` !== i)
        throw new Error();
      if (s < 0 || s > 128)
        throw new Error();
      new URL(`http://[${r}]`);
    } catch {
      n.issues.push({
        code: "invalid_format",
        format: "cidrv6",
        input: n.value,
        inst: t,
        continue: !e.abort
      });
    }
  };
});
function gn(t) {
  if (t === "")
    return !0;
  if (t.length % 4 !== 0)
    return !1;
  try {
    return atob(t), !0;
  } catch {
    return !1;
  }
}
const ui = /* @__PURE__ */ N("$ZodBase64", (t, e) => {
  e.pattern ?? (e.pattern = gr), ct.init(t, e), t._zod.onattach.push((n) => {
    n._zod.bag.contentEncoding = "base64";
  }), t._zod.check = (n) => {
    gn(n.value) || n.issues.push({
      code: "invalid_format",
      format: "base64",
      input: n.value,
      inst: t,
      continue: !e.abort
    });
  };
});
function ci(t) {
  if (!fn.test(t))
    return !1;
  const e = t.replace(/[-_]/g, (r) => r === "-" ? "+" : "/"), n = e.padEnd(Math.ceil(e.length / 4) * 4, "=");
  return gn(n);
}
const li = /* @__PURE__ */ N("$ZodBase64URL", (t, e) => {
  e.pattern ?? (e.pattern = fn), ct.init(t, e), t._zod.onattach.push((n) => {
    n._zod.bag.contentEncoding = "base64url";
  }), t._zod.check = (n) => {
    ci(n.value) || n.issues.push({
      code: "invalid_format",
      format: "base64url",
      input: n.value,
      inst: t,
      continue: !e.abort
    });
  };
}), fi = /* @__PURE__ */ N("$ZodE164", (t, e) => {
  e.pattern ?? (e.pattern = yr), ct.init(t, e);
});
function hi(t, e = null) {
  try {
    const n = t.split(".");
    if (n.length !== 3)
      return !1;
    const [r] = n;
    if (!r)
      return !1;
    const i = JSON.parse(atob(r));
    return !("typ" in i && i?.typ !== "JWT" || !i.alg || e && (!("alg" in i) || i.alg !== e));
  } catch {
    return !1;
  }
}
const di = /* @__PURE__ */ N("$ZodJWT", (t, e) => {
  ct.init(t, e), t._zod.check = (n) => {
    hi(n.value, e.alg) || n.issues.push({
      code: "invalid_format",
      format: "jwt",
      input: n.value,
      inst: t,
      continue: !e.abort
    });
  };
}), vn = /* @__PURE__ */ N("$ZodNumber", (t, e) => {
  ut.init(t, e), t._zod.pattern = t._zod.bag.pattern ?? Er, t._zod.parse = (n, r) => {
    if (e.coerce)
      try {
        n.value = Number(n.value);
      } catch {
      }
    const i = n.value;
    if (typeof i == "number" && !Number.isNaN(i) && Number.isFinite(i))
      return n;
    const s = typeof i == "number" ? Number.isNaN(i) ? "NaN" : Number.isFinite(i) ? void 0 : "Infinity" : void 0;
    return n.issues.push({
      expected: "number",
      code: "invalid_type",
      input: i,
      inst: t,
      ...s ? { received: s } : {}
    }), n;
  };
}), pi = /* @__PURE__ */ N("$ZodNumber", (t, e) => {
  Cr.init(t, e), vn.init(t, e);
}), mi = /* @__PURE__ */ N("$ZodBoolean", (t, e) => {
  ut.init(t, e), t._zod.pattern = Ir, t._zod.parse = (n, r) => {
    if (e.coerce)
      try {
        n.value = !!n.value;
      } catch {
      }
    const i = n.value;
    return typeof i == "boolean" || n.issues.push({
      expected: "boolean",
      code: "invalid_type",
      input: i,
      inst: t
    }), n;
  };
}), _i = /* @__PURE__ */ N("$ZodAny", (t, e) => {
  ut.init(t, e), t._zod.parse = (n) => n;
}), gi = /* @__PURE__ */ N("$ZodUnknown", (t, e) => {
  ut.init(t, e), t._zod.parse = (n) => n;
}), vi = /* @__PURE__ */ N("$ZodNever", (t, e) => {
  ut.init(t, e), t._zod.parse = (n, r) => (n.issues.push({
    expected: "never",
    code: "invalid_type",
    input: n.value,
    inst: t
  }), n);
});
function Te(t, e, n) {
  t.issues.length && e.issues.push(...Pt(n, t.issues)), e.value[n] = t.value;
}
const yi = /* @__PURE__ */ N("$ZodArray", (t, e) => {
  ut.init(t, e), t._zod.parse = (n, r) => {
    const i = n.value;
    if (!Array.isArray(i))
      return n.issues.push({
        expected: "array",
        code: "invalid_type",
        input: i,
        inst: t
      }), n;
    n.value = Array(i.length);
    const s = [];
    for (let o = 0; o < i.length; o++) {
      const a = i[o], l = e.element._zod.run({
        value: a,
        issues: []
      }, r);
      l instanceof Promise ? s.push(l.then((m) => Te(m, n, o))) : Te(l, n, o);
    }
    return s.length ? Promise.all(s).then(() => n) : n;
  };
});
function Qt(t, e, n, r) {
  t.issues.length && e.issues.push(...Pt(n, t.issues)), t.value === void 0 ? n in r && (e.value[n] = void 0) : e.value[n] = t.value;
}
const wi = /* @__PURE__ */ N("$ZodObject", (t, e) => {
  ut.init(t, e);
  const n = rn(() => {
    const _ = Object.keys(e.shape);
    for (const f of _)
      if (!(e.shape[f] instanceof ut))
        throw new Error(`Invalid element at key "${f}": expected a Zod schema`);
    const y = Ln(e.shape);
    return {
      shape: e.shape,
      keys: _,
      keySet: new Set(_),
      numKeys: _.length,
      optionalKeys: new Set(y)
    };
  });
  at(t._zod, "propValues", () => {
    const _ = e.shape, y = {};
    for (const f in _) {
      const w = _[f]._zod;
      if (w.values) {
        y[f] ?? (y[f] = /* @__PURE__ */ new Set());
        for (const h of w.values)
          y[f].add(h);
      }
    }
    return y;
  });
  const r = (_) => {
    const y = new Ur(["shape", "payload", "ctx"]), f = n.value, w = (v) => {
      const z = Ze(v);
      return `shape[${z}]._zod.run({ value: input[${z}], issues: [] }, ctx)`;
    };
    y.write("const input = payload.value;");
    const h = /* @__PURE__ */ Object.create(null);
    let p = 0;
    for (const v of f.keys)
      h[v] = `key_${p++}`;
    y.write("const newResult = {}");
    for (const v of f.keys) {
      const z = h[v], I = Ze(v);
      y.write(`const ${z} = ${w(v)};`), y.write(`
        if (${z}.issues.length) {
          payload.issues = payload.issues.concat(${z}.issues.map(iss => ({
            ...iss,
            path: iss.path ? [${I}, ...iss.path] : [${I}]
          })));
        }
        
        if (${z}.value === undefined) {
          if (${I} in input) {
            newResult[${I}] = undefined;
          }
        } else {
          newResult[${I}] = ${z}.value;
        }
      `);
    }
    y.write("payload.value = newResult;"), y.write("return payload;");
    const d = y.compile();
    return (v, z) => d(_, v, z);
  };
  let i;
  const s = me, o = !nn.jitless, l = o && jn.value, m = e.catchall;
  let k;
  t._zod.parse = (_, y) => {
    k ?? (k = n.value);
    const f = _.value;
    if (!s(f))
      return _.issues.push({
        expected: "object",
        code: "invalid_type",
        input: f,
        inst: t
      }), _;
    const w = [];
    if (o && l && y?.async === !1 && y.jitless !== !0)
      i || (i = r(e.shape)), _ = i(_, y);
    else {
      _.value = {};
      const z = k.shape;
      for (const I of k.keys) {
        const D = z[I]._zod.run({ value: f[I], issues: [] }, y);
        D instanceof Promise ? w.push(D.then((T) => Qt(T, _, I, f))) : Qt(D, _, I, f);
      }
    }
    if (!m)
      return w.length ? Promise.all(w).then(() => _) : _;
    const h = [], p = k.keySet, d = m._zod, v = d.def.type;
    for (const z of Object.keys(f)) {
      if (p.has(z))
        continue;
      if (v === "never") {
        h.push(z);
        continue;
      }
      const I = d.run({ value: f[z], issues: [] }, y);
      I instanceof Promise ? w.push(I.then((S) => Qt(S, _, z, f))) : Qt(I, _, z, f);
    }
    return h.length && _.issues.push({
      code: "unrecognized_keys",
      keys: h,
      input: f,
      inst: t
    }), w.length ? Promise.all(w).then(() => _) : _;
  };
});
function Re(t, e, n, r) {
  for (const s of t)
    if (s.issues.length === 0)
      return e.value = s.value, e;
  const i = t.filter((s) => !Wt(s));
  return i.length === 1 ? (e.value = i[0].value, i[0]) : (e.issues.push({
    code: "invalid_union",
    input: e.value,
    inst: n,
    errors: t.map((s) => s.issues.map((o) => $t(o, r, Zt())))
  }), e);
}
const bi = /* @__PURE__ */ N("$ZodUnion", (t, e) => {
  ut.init(t, e), at(t._zod, "optin", () => e.options.some((i) => i._zod.optin === "optional") ? "optional" : void 0), at(t._zod, "optout", () => e.options.some((i) => i._zod.optout === "optional") ? "optional" : void 0), at(t._zod, "values", () => {
    if (e.options.every((i) => i._zod.values))
      return new Set(e.options.flatMap((i) => Array.from(i._zod.values)));
  }), at(t._zod, "pattern", () => {
    if (e.options.every((i) => i._zod.pattern)) {
      const i = e.options.map((s) => s._zod.pattern);
      return new RegExp(`^(${i.map((s) => we(s.source)).join("|")})$`);
    }
  });
  const n = e.options.length === 1, r = e.options[0]._zod.run;
  t._zod.parse = (i, s) => {
    if (n)
      return r(i, s);
    let o = !1;
    const a = [];
    for (const l of e.options) {
      const m = l._zod.run({
        value: i.value,
        issues: []
      }, s);
      if (m instanceof Promise)
        a.push(m), o = !0;
      else {
        if (m.issues.length === 0)
          return m;
        a.push(m);
      }
    }
    return o ? Promise.all(a).then((l) => Re(l, i, t, s)) : Re(a, i, t, s);
  };
}), ki = /* @__PURE__ */ N("$ZodIntersection", (t, e) => {
  ut.init(t, e), t._zod.parse = (n, r) => {
    const i = n.value, s = e.left._zod.run({ value: i, issues: [] }, r), o = e.right._zod.run({ value: i, issues: [] }, r);
    return s instanceof Promise || o instanceof Promise ? Promise.all([s, o]).then(([l, m]) => Ne(n, l, m)) : Ne(n, s, o);
  };
});
function _e(t, e) {
  if (t === e)
    return { valid: !0, data: t };
  if (t instanceof Date && e instanceof Date && +t == +e)
    return { valid: !0, data: t };
  if (re(t) && re(e)) {
    const n = Object.keys(e), r = Object.keys(t).filter((s) => n.indexOf(s) !== -1), i = { ...t, ...e };
    for (const s of r) {
      const o = _e(t[s], e[s]);
      if (!o.valid)
        return {
          valid: !1,
          mergeErrorPath: [s, ...o.mergeErrorPath]
        };
      i[s] = o.data;
    }
    return { valid: !0, data: i };
  }
  if (Array.isArray(t) && Array.isArray(e)) {
    if (t.length !== e.length)
      return { valid: !1, mergeErrorPath: [] };
    const n = [];
    for (let r = 0; r < t.length; r++) {
      const i = t[r], s = e[r], o = _e(i, s);
      if (!o.valid)
        return {
          valid: !1,
          mergeErrorPath: [r, ...o.mergeErrorPath]
        };
      n.push(o.data);
    }
    return { valid: !0, data: n };
  }
  return { valid: !1, mergeErrorPath: [] };
}
function Ne(t, e, n) {
  if (e.issues.length && t.issues.push(...e.issues), n.issues.length && t.issues.push(...n.issues), Wt(t))
    return t;
  const r = _e(e.value, n.value);
  if (!r.valid)
    throw new Error(`Unmergable intersection. Error path: ${JSON.stringify(r.mergeErrorPath)}`);
  return t.value = r.data, t;
}
const zi = /* @__PURE__ */ N("$ZodRecord", (t, e) => {
  ut.init(t, e), t._zod.parse = (n, r) => {
    const i = n.value;
    if (!re(i))
      return n.issues.push({
        expected: "record",
        code: "invalid_type",
        input: i,
        inst: t
      }), n;
    const s = [];
    if (e.keyType._zod.values) {
      const o = e.keyType._zod.values;
      n.value = {};
      for (const l of o)
        if (typeof l == "string" || typeof l == "number" || typeof l == "symbol") {
          const m = e.valueType._zod.run({ value: i[l], issues: [] }, r);
          m instanceof Promise ? s.push(m.then((k) => {
            k.issues.length && n.issues.push(...Pt(l, k.issues)), n.value[l] = k.value;
          })) : (m.issues.length && n.issues.push(...Pt(l, m.issues)), n.value[l] = m.value);
        }
      let a;
      for (const l in i)
        o.has(l) || (a = a ?? [], a.push(l));
      a && a.length > 0 && n.issues.push({
        code: "unrecognized_keys",
        input: i,
        inst: t,
        keys: a
      });
    } else {
      n.value = {};
      for (const o of Reflect.ownKeys(i)) {
        if (o === "__proto__")
          continue;
        const a = e.keyType._zod.run({ value: o, issues: [] }, r);
        if (a instanceof Promise)
          throw new Error("Async schemas not supported in object keys currently");
        if (a.issues.length) {
          n.issues.push({
            code: "invalid_key",
            origin: "record",
            issues: a.issues.map((m) => $t(m, r, Zt())),
            input: o,
            path: [o],
            inst: t
          }), n.value[a.value] = a.value;
          continue;
        }
        const l = e.valueType._zod.run({ value: i[o], issues: [] }, r);
        l instanceof Promise ? s.push(l.then((m) => {
          m.issues.length && n.issues.push(...Pt(o, m.issues)), n.value[a.value] = m.value;
        })) : (l.issues.length && n.issues.push(...Pt(o, l.issues)), n.value[a.value] = l.value);
      }
    }
    return s.length ? Promise.all(s).then(() => n) : n;
  };
}), xi = /* @__PURE__ */ N("$ZodEnum", (t, e) => {
  ut.init(t, e);
  const n = Dn(e.entries), r = new Set(n);
  t._zod.values = r, t._zod.pattern = new RegExp(`^(${n.filter((i) => Un.has(typeof i)).map((i) => typeof i == "string" ? oe(i) : i.toString()).join("|")})$`), t._zod.parse = (i, s) => {
    const o = i.value;
    return r.has(o) || i.issues.push({
      code: "invalid_value",
      values: n,
      input: o,
      inst: t
    }), i;
  };
}), Ei = /* @__PURE__ */ N("$ZodTransform", (t, e) => {
  ut.init(t, e), t._zod.parse = (n, r) => {
    const i = e.transform(n.value, n);
    if (r.async)
      return (i instanceof Promise ? i : Promise.resolve(i)).then((o) => (n.value = o, n));
    if (i instanceof Promise)
      throw new Vt();
    return n.value = i, n;
  };
});
function Fe(t, e) {
  return t.issues.length && e === void 0 ? { issues: [], value: void 0 } : t;
}
const Ii = /* @__PURE__ */ N("$ZodOptional", (t, e) => {
  ut.init(t, e), t._zod.optin = "optional", t._zod.optout = "optional", at(t._zod, "values", () => e.innerType._zod.values ? /* @__PURE__ */ new Set([...e.innerType._zod.values, void 0]) : void 0), at(t._zod, "pattern", () => {
    const n = e.innerType._zod.pattern;
    return n ? new RegExp(`^(${we(n.source)})?$`) : void 0;
  }), t._zod.parse = (n, r) => {
    if (e.innerType._zod.optin === "optional") {
      const i = e.innerType._zod.run(n, r);
      return i instanceof Promise ? i.then((s) => Fe(s, n.value)) : Fe(i, n.value);
    }
    return n.value === void 0 ? n : e.innerType._zod.run(n, r);
  };
}), Si = /* @__PURE__ */ N("$ZodNullable", (t, e) => {
  ut.init(t, e), at(t._zod, "optin", () => e.innerType._zod.optin), at(t._zod, "optout", () => e.innerType._zod.optout), at(t._zod, "pattern", () => {
    const n = e.innerType._zod.pattern;
    return n ? new RegExp(`^(${we(n.source)}|null)$`) : void 0;
  }), at(t._zod, "values", () => e.innerType._zod.values ? /* @__PURE__ */ new Set([...e.innerType._zod.values, null]) : void 0), t._zod.parse = (n, r) => n.value === null ? n : e.innerType._zod.run(n, r);
}), Ai = /* @__PURE__ */ N("$ZodDefault", (t, e) => {
  ut.init(t, e), t._zod.optin = "optional", at(t._zod, "values", () => e.innerType._zod.values), t._zod.parse = (n, r) => {
    if (n.value === void 0)
      return n.value = e.defaultValue, n;
    const i = e.innerType._zod.run(n, r);
    return i instanceof Promise ? i.then((s) => De(s, e)) : De(i, e);
  };
});
function De(t, e) {
  return t.value === void 0 && (t.value = e.defaultValue), t;
}
const Oi = /* @__PURE__ */ N("$ZodPrefault", (t, e) => {
  ut.init(t, e), t._zod.optin = "optional", at(t._zod, "values", () => e.innerType._zod.values), t._zod.parse = (n, r) => (n.value === void 0 && (n.value = e.defaultValue), e.innerType._zod.run(n, r));
}), Ci = /* @__PURE__ */ N("$ZodNonOptional", (t, e) => {
  ut.init(t, e), at(t._zod, "values", () => {
    const n = e.innerType._zod.values;
    return n ? new Set([...n].filter((r) => r !== void 0)) : void 0;
  }), t._zod.parse = (n, r) => {
    const i = e.innerType._zod.run(n, r);
    return i instanceof Promise ? i.then((s) => Pe(s, t)) : Pe(i, t);
  };
});
function Pe(t, e) {
  return !t.issues.length && t.value === void 0 && t.issues.push({
    code: "invalid_type",
    expected: "nonoptional",
    input: t.value,
    inst: e
  }), t;
}
const Zi = /* @__PURE__ */ N("$ZodCatch", (t, e) => {
  ut.init(t, e), at(t._zod, "optin", () => e.innerType._zod.optin), at(t._zod, "optout", () => e.innerType._zod.optout), at(t._zod, "values", () => e.innerType._zod.values), t._zod.parse = (n, r) => {
    const i = e.innerType._zod.run(n, r);
    return i instanceof Promise ? i.then((s) => (n.value = s.value, s.issues.length && (n.value = e.catchValue({
      ...n,
      error: {
        issues: s.issues.map((o) => $t(o, r, Zt()))
      },
      input: n.value
    }), n.issues = []), n)) : (n.value = i.value, i.issues.length && (n.value = e.catchValue({
      ...n,
      error: {
        issues: i.issues.map((s) => $t(s, r, Zt()))
      },
      input: n.value
    }), n.issues = []), n);
  };
}), $i = /* @__PURE__ */ N("$ZodPipe", (t, e) => {
  ut.init(t, e), at(t._zod, "values", () => e.in._zod.values), at(t._zod, "optin", () => e.in._zod.optin), at(t._zod, "optout", () => e.out._zod.optout), at(t._zod, "propValues", () => e.in._zod.propValues), t._zod.parse = (n, r) => {
    const i = e.in._zod.run(n, r);
    return i instanceof Promise ? i.then((s) => Be(s, e, r)) : Be(i, e, r);
  };
});
function Be(t, e, n) {
  return t.issues.length ? t : e.out._zod.run({ value: t.value, issues: t.issues }, n);
}
const Ti = /* @__PURE__ */ N("$ZodReadonly", (t, e) => {
  ut.init(t, e), at(t._zod, "propValues", () => e.innerType._zod.propValues), at(t._zod, "values", () => e.innerType._zod.values), at(t._zod, "optin", () => e.innerType._zod.optin), at(t._zod, "optout", () => e.innerType._zod.optout), t._zod.parse = (n, r) => {
    const i = e.innerType._zod.run(n, r);
    return i instanceof Promise ? i.then(je) : je(i);
  };
});
function je(t) {
  return t.value = Object.freeze(t.value), t;
}
const Ri = /* @__PURE__ */ N("$ZodCustom", (t, e) => {
  wt.init(t, e), ut.init(t, e), t._zod.parse = (n, r) => n, t._zod.check = (n) => {
    const r = n.value, i = e.fn(r);
    if (i instanceof Promise)
      return i.then((s) => Ue(s, n, r, t));
    Ue(i, n, r, t);
  };
});
function Ue(t, e, n, r) {
  if (!t) {
    const i = {
      code: "custom",
      input: n,
      inst: r,
      // incorporates params.error into issue reporting
      path: [...r._zod.def.path ?? []],
      // incorporates params.error into issue reporting
      continue: !r._zod.def.abort
      // params: inst._zod.def.params,
    };
    r._zod.def.params && (i.params = r._zod.def.params), e.issues.push(Gt(i));
  }
}
class Ni {
  constructor() {
    this._map = /* @__PURE__ */ new Map(), this._idmap = /* @__PURE__ */ new Map();
  }
  add(e, ...n) {
    const r = n[0];
    if (this._map.set(e, r), r && typeof r == "object" && "id" in r) {
      if (this._idmap.has(r.id))
        throw new Error(`ID ${r.id} already exists in the registry`);
      this._idmap.set(r.id, e);
    }
    return this;
  }
  clear() {
    return this._map = /* @__PURE__ */ new Map(), this._idmap = /* @__PURE__ */ new Map(), this;
  }
  remove(e) {
    const n = this._map.get(e);
    return n && typeof n == "object" && "id" in n && this._idmap.delete(n.id), this._map.delete(e), this;
  }
  get(e) {
    const n = e._zod.parent;
    if (n) {
      const r = { ...this.get(n) ?? {} };
      delete r.id;
      const i = { ...r, ...this._map.get(e) };
      return Object.keys(i).length ? i : void 0;
    }
    return this._map.get(e);
  }
  has(e) {
    return this._map.has(e);
  }
}
function Fi() {
  return new Ni();
}
const te = /* @__PURE__ */ Fi();
function Di(t, e) {
  return new t({
    type: "string",
    ...J(e)
  });
}
function Pi(t, e) {
  return new t({
    type: "string",
    format: "email",
    check: "string_format",
    abort: !1,
    ...J(e)
  });
}
function Le(t, e) {
  return new t({
    type: "string",
    format: "guid",
    check: "string_format",
    abort: !1,
    ...J(e)
  });
}
function Bi(t, e) {
  return new t({
    type: "string",
    format: "uuid",
    check: "string_format",
    abort: !1,
    ...J(e)
  });
}
function ji(t, e) {
  return new t({
    type: "string",
    format: "uuid",
    check: "string_format",
    abort: !1,
    version: "v4",
    ...J(e)
  });
}
function Ui(t, e) {
  return new t({
    type: "string",
    format: "uuid",
    check: "string_format",
    abort: !1,
    version: "v6",
    ...J(e)
  });
}
function Li(t, e) {
  return new t({
    type: "string",
    format: "uuid",
    check: "string_format",
    abort: !1,
    version: "v7",
    ...J(e)
  });
}
function Mi(t, e) {
  return new t({
    type: "string",
    format: "url",
    check: "string_format",
    abort: !1,
    ...J(e)
  });
}
function Wi(t, e) {
  return new t({
    type: "string",
    format: "emoji",
    check: "string_format",
    abort: !1,
    ...J(e)
  });
}
function Vi(t, e) {
  return new t({
    type: "string",
    format: "nanoid",
    check: "string_format",
    abort: !1,
    ...J(e)
  });
}
function Gi(t, e) {
  return new t({
    type: "string",
    format: "cuid",
    check: "string_format",
    abort: !1,
    ...J(e)
  });
}
function Hi(t, e) {
  return new t({
    type: "string",
    format: "cuid2",
    check: "string_format",
    abort: !1,
    ...J(e)
  });
}
function Yi(t, e) {
  return new t({
    type: "string",
    format: "ulid",
    check: "string_format",
    abort: !1,
    ...J(e)
  });
}
function Ki(t, e) {
  return new t({
    type: "string",
    format: "xid",
    check: "string_format",
    abort: !1,
    ...J(e)
  });
}
function Ji(t, e) {
  return new t({
    type: "string",
    format: "ksuid",
    check: "string_format",
    abort: !1,
    ...J(e)
  });
}
function Xi(t, e) {
  return new t({
    type: "string",
    format: "ipv4",
    check: "string_format",
    abort: !1,
    ...J(e)
  });
}
function qi(t, e) {
  return new t({
    type: "string",
    format: "ipv6",
    check: "string_format",
    abort: !1,
    ...J(e)
  });
}
function Qi(t, e) {
  return new t({
    type: "string",
    format: "cidrv4",
    check: "string_format",
    abort: !1,
    ...J(e)
  });
}
function ts(t, e) {
  return new t({
    type: "string",
    format: "cidrv6",
    check: "string_format",
    abort: !1,
    ...J(e)
  });
}
function es(t, e) {
  return new t({
    type: "string",
    format: "base64",
    check: "string_format",
    abort: !1,
    ...J(e)
  });
}
function ns(t, e) {
  return new t({
    type: "string",
    format: "base64url",
    check: "string_format",
    abort: !1,
    ...J(e)
  });
}
function rs(t, e) {
  return new t({
    type: "string",
    format: "e164",
    check: "string_format",
    abort: !1,
    ...J(e)
  });
}
function is(t, e) {
  return new t({
    type: "string",
    format: "jwt",
    check: "string_format",
    abort: !1,
    ...J(e)
  });
}
function ss(t, e) {
  return new t({
    type: "string",
    format: "datetime",
    check: "string_format",
    offset: !1,
    local: !1,
    precision: null,
    ...J(e)
  });
}
function os(t, e) {
  return new t({
    type: "string",
    format: "date",
    check: "string_format",
    ...J(e)
  });
}
function as(t, e) {
  return new t({
    type: "string",
    format: "time",
    check: "string_format",
    precision: null,
    ...J(e)
  });
}
function us(t, e) {
  return new t({
    type: "string",
    format: "duration",
    check: "string_format",
    ...J(e)
  });
}
function cs(t, e) {
  return new t({
    type: "number",
    checks: [],
    ...J(e)
  });
}
function ls(t, e) {
  return new t({
    type: "number",
    check: "number_format",
    abort: !1,
    format: "safeint",
    ...J(e)
  });
}
function fs(t, e) {
  return new t({
    type: "boolean",
    ...J(e)
  });
}
function hs(t) {
  return new t({
    type: "any"
  });
}
function ds(t) {
  return new t({
    type: "unknown"
  });
}
function ps(t, e) {
  return new t({
    type: "never",
    ...J(e)
  });
}
function Me(t, e) {
  return new mn({
    check: "less_than",
    ...J(e),
    value: t,
    inclusive: !1
  });
}
function fe(t, e) {
  return new mn({
    check: "less_than",
    ...J(e),
    value: t,
    inclusive: !0
  });
}
function We(t, e) {
  return new _n({
    check: "greater_than",
    ...J(e),
    value: t,
    inclusive: !1
  });
}
function he(t, e) {
  return new _n({
    check: "greater_than",
    ...J(e),
    value: t,
    inclusive: !0
  });
}
function Ve(t, e) {
  return new Or({
    check: "multiple_of",
    ...J(e),
    value: t
  });
}
function yn(t, e) {
  return new Zr({
    check: "max_length",
    ...J(e),
    maximum: t
  });
}
function ie(t, e) {
  return new $r({
    check: "min_length",
    ...J(e),
    minimum: t
  });
}
function wn(t, e) {
  return new Tr({
    check: "length_equals",
    ...J(e),
    length: t
  });
}
function ms(t, e) {
  return new Rr({
    check: "string_format",
    format: "regex",
    ...J(e),
    pattern: t
  });
}
function _s(t) {
  return new Nr({
    check: "string_format",
    format: "lowercase",
    ...J(t)
  });
}
function gs(t) {
  return new Fr({
    check: "string_format",
    format: "uppercase",
    ...J(t)
  });
}
function vs(t, e) {
  return new Dr({
    check: "string_format",
    format: "includes",
    ...J(e),
    includes: t
  });
}
function ys(t, e) {
  return new Pr({
    check: "string_format",
    format: "starts_with",
    ...J(e),
    prefix: t
  });
}
function ws(t, e) {
  return new Br({
    check: "string_format",
    format: "ends_with",
    ...J(e),
    suffix: t
  });
}
function Ht(t) {
  return new jr({
    check: "overwrite",
    tx: t
  });
}
function bs(t) {
  return Ht((e) => e.normalize(t));
}
function ks() {
  return Ht((t) => t.trim());
}
function zs() {
  return Ht((t) => t.toLowerCase());
}
function xs() {
  return Ht((t) => t.toUpperCase());
}
function Es(t, e, n) {
  return new t({
    type: "array",
    element: e,
    // get element() {
    //   return element;
    // },
    ...J(n)
  });
}
function Is(t, e, n) {
  return new t({
    type: "custom",
    check: "custom",
    fn: e,
    ...J(n)
  });
}
function Ss(t) {
  const e = As((n) => (n.addIssue = (r) => {
    if (typeof r == "string")
      n.issues.push(Gt(r, n.value, e._zod.def));
    else {
      const i = r;
      i.fatal && (i.continue = !1), i.code ?? (i.code = "custom"), i.input ?? (i.input = n.value), i.inst ?? (i.inst = e), i.continue ?? (i.continue = !e._zod.def.abort), n.issues.push(Gt(i));
    }
  }, t(n.value, n)));
  return e;
}
function As(t, e) {
  const n = new wt({
    check: "custom",
    ...J(e)
  });
  return n._zod.check = t, n;
}
const Os = /* @__PURE__ */ N("ZodISODateTime", (t, e) => {
  ti.init(t, e), lt.init(t, e);
});
function Cs(t) {
  return ss(Os, t);
}
const Zs = /* @__PURE__ */ N("ZodISODate", (t, e) => {
  ei.init(t, e), lt.init(t, e);
});
function $s(t) {
  return os(Zs, t);
}
const Ts = /* @__PURE__ */ N("ZodISOTime", (t, e) => {
  ni.init(t, e), lt.init(t, e);
});
function Rs(t) {
  return as(Ts, t);
}
const Ns = /* @__PURE__ */ N("ZodISODuration", (t, e) => {
  ri.init(t, e), lt.init(t, e);
});
function Fs(t) {
  return us(Ns, t);
}
const bn = (t, e) => {
  an.init(t, e), t.name = "ZodError", Object.defineProperties(t, {
    format: {
      value: (n) => Xn(t, n)
      // enumerable: false,
    },
    flatten: {
      value: (n) => Jn(t, n)
      // enumerable: false,
    },
    addIssue: {
      value: (n) => {
        t.issues.push(n), t.message = JSON.stringify(t.issues, pe, 2);
      }
      // enumerable: false,
    },
    addIssues: {
      value: (n) => {
        t.issues.push(...n), t.message = JSON.stringify(t.issues, pe, 2);
      }
      // enumerable: false,
    },
    isEmpty: {
      get() {
        return t.issues.length === 0;
      }
      // enumerable: false,
    }
  });
}, ze = N("ZodError", bn), ue = N("ZodError", bn, {
  Parent: Error
}), Ds = /* @__PURE__ */ qn(ue), Ps = /* @__PURE__ */ Qn(ue), Bs = /* @__PURE__ */ cn(ue), js = /* @__PURE__ */ ln(ue), ft = /* @__PURE__ */ N("ZodType", (t, e) => (ut.init(t, e), t.def = e, Object.defineProperty(t, "_def", { value: e }), t.check = (...n) => t.clone(
  {
    ...e,
    checks: [
      ...e.checks ?? [],
      ...n.map((r) => typeof r == "function" ? { _zod: { check: r, def: { check: "custom" }, onattach: [] } } : r)
    ]
  }
  // { parent: true }
), t.clone = (n, r) => Ft(t, n, r), t.brand = () => t, t.register = (n, r) => (n.add(t, r), t), t.parse = (n, r) => Ds(t, n, r, { callee: t.parse }), t.safeParse = (n, r) => Bs(t, n, r), t.parseAsync = async (n, r) => Ps(t, n, r, { callee: t.parseAsync }), t.safeParseAsync = async (n, r) => js(t, n, r), t.spa = t.safeParseAsync, t.refine = (n, r) => t.check(No(n, r)), t.superRefine = (n) => t.check(Fo(n)), t.overwrite = (n) => t.check(Ht(n)), t.optional = () => Ye(t), t.nullable = () => Ke(t), t.nullish = () => Ye(Ke(t)), t.nonoptional = (n) => Ao(t, n), t.array = () => xe(t), t.or = (n) => _o([t, n]), t.and = (n) => vo(t, n), t.transform = (n) => Je(t, ko(n)), t.default = (n) => Eo(t, n), t.prefault = (n) => So(t, n), t.catch = (n) => Co(t, n), t.pipe = (n) => Je(t, n), t.readonly = () => To(t), t.describe = (n) => {
  const r = t.clone();
  return te.add(r, { description: n }), r;
}, Object.defineProperty(t, "description", {
  get() {
    return te.get(t)?.description;
  },
  configurable: !0
}), t.meta = (...n) => {
  if (n.length === 0)
    return te.get(t);
  const r = t.clone();
  return te.add(r, n[0]), r;
}, t.isOptional = () => t.safeParse(void 0).success, t.isNullable = () => t.safeParse(null).success, t)), kn = /* @__PURE__ */ N("_ZodString", (t, e) => {
  ke.init(t, e), ft.init(t, e);
  const n = t._zod.bag;
  t.format = n.format ?? null, t.minLength = n.minimum ?? null, t.maxLength = n.maximum ?? null, t.regex = (...r) => t.check(ms(...r)), t.includes = (...r) => t.check(vs(...r)), t.startsWith = (...r) => t.check(ys(...r)), t.endsWith = (...r) => t.check(ws(...r)), t.min = (...r) => t.check(ie(...r)), t.max = (...r) => t.check(yn(...r)), t.length = (...r) => t.check(wn(...r)), t.nonempty = (...r) => t.check(ie(1, ...r)), t.lowercase = (r) => t.check(_s(r)), t.uppercase = (r) => t.check(gs(r)), t.trim = () => t.check(ks()), t.normalize = (...r) => t.check(bs(...r)), t.toLowerCase = () => t.check(zs()), t.toUpperCase = () => t.check(xs());
}), Us = /* @__PURE__ */ N("ZodString", (t, e) => {
  ke.init(t, e), kn.init(t, e), t.email = (n) => t.check(Pi(Ls, n)), t.url = (n) => t.check(Mi(Ms, n)), t.jwt = (n) => t.check(is(io, n)), t.emoji = (n) => t.check(Wi(Ws, n)), t.guid = (n) => t.check(Le(Ge, n)), t.uuid = (n) => t.check(Bi(ee, n)), t.uuidv4 = (n) => t.check(ji(ee, n)), t.uuidv6 = (n) => t.check(Ui(ee, n)), t.uuidv7 = (n) => t.check(Li(ee, n)), t.nanoid = (n) => t.check(Vi(Vs, n)), t.guid = (n) => t.check(Le(Ge, n)), t.cuid = (n) => t.check(Gi(Gs, n)), t.cuid2 = (n) => t.check(Hi(Hs, n)), t.ulid = (n) => t.check(Yi(Ys, n)), t.base64 = (n) => t.check(es(eo, n)), t.base64url = (n) => t.check(ns(no, n)), t.xid = (n) => t.check(Ki(Ks, n)), t.ksuid = (n) => t.check(Ji(Js, n)), t.ipv4 = (n) => t.check(Xi(Xs, n)), t.ipv6 = (n) => t.check(qi(qs, n)), t.cidrv4 = (n) => t.check(Qi(Qs, n)), t.cidrv6 = (n) => t.check(ts(to, n)), t.e164 = (n) => t.check(rs(ro, n)), t.datetime = (n) => t.check(Cs(n)), t.date = (n) => t.check($s(n)), t.time = (n) => t.check(Rs(n)), t.duration = (n) => t.check(Fs(n));
});
function zt(t) {
  return Di(Us, t);
}
const lt = /* @__PURE__ */ N("ZodStringFormat", (t, e) => {
  ct.init(t, e), kn.init(t, e);
}), Ls = /* @__PURE__ */ N("ZodEmail", (t, e) => {
  Vr.init(t, e), lt.init(t, e);
}), Ge = /* @__PURE__ */ N("ZodGUID", (t, e) => {
  Mr.init(t, e), lt.init(t, e);
}), ee = /* @__PURE__ */ N("ZodUUID", (t, e) => {
  Wr.init(t, e), lt.init(t, e);
}), Ms = /* @__PURE__ */ N("ZodURL", (t, e) => {
  Gr.init(t, e), lt.init(t, e);
}), Ws = /* @__PURE__ */ N("ZodEmoji", (t, e) => {
  Hr.init(t, e), lt.init(t, e);
}), Vs = /* @__PURE__ */ N("ZodNanoID", (t, e) => {
  Yr.init(t, e), lt.init(t, e);
}), Gs = /* @__PURE__ */ N("ZodCUID", (t, e) => {
  Kr.init(t, e), lt.init(t, e);
}), Hs = /* @__PURE__ */ N("ZodCUID2", (t, e) => {
  Jr.init(t, e), lt.init(t, e);
}), Ys = /* @__PURE__ */ N("ZodULID", (t, e) => {
  Xr.init(t, e), lt.init(t, e);
}), Ks = /* @__PURE__ */ N("ZodXID", (t, e) => {
  qr.init(t, e), lt.init(t, e);
}), Js = /* @__PURE__ */ N("ZodKSUID", (t, e) => {
  Qr.init(t, e), lt.init(t, e);
}), Xs = /* @__PURE__ */ N("ZodIPv4", (t, e) => {
  ii.init(t, e), lt.init(t, e);
}), qs = /* @__PURE__ */ N("ZodIPv6", (t, e) => {
  si.init(t, e), lt.init(t, e);
}), Qs = /* @__PURE__ */ N("ZodCIDRv4", (t, e) => {
  oi.init(t, e), lt.init(t, e);
}), to = /* @__PURE__ */ N("ZodCIDRv6", (t, e) => {
  ai.init(t, e), lt.init(t, e);
}), eo = /* @__PURE__ */ N("ZodBase64", (t, e) => {
  ui.init(t, e), lt.init(t, e);
}), no = /* @__PURE__ */ N("ZodBase64URL", (t, e) => {
  li.init(t, e), lt.init(t, e);
}), ro = /* @__PURE__ */ N("ZodE164", (t, e) => {
  fi.init(t, e), lt.init(t, e);
}), io = /* @__PURE__ */ N("ZodJWT", (t, e) => {
  di.init(t, e), lt.init(t, e);
}), zn = /* @__PURE__ */ N("ZodNumber", (t, e) => {
  vn.init(t, e), ft.init(t, e), t.gt = (r, i) => t.check(We(r, i)), t.gte = (r, i) => t.check(he(r, i)), t.min = (r, i) => t.check(he(r, i)), t.lt = (r, i) => t.check(Me(r, i)), t.lte = (r, i) => t.check(fe(r, i)), t.max = (r, i) => t.check(fe(r, i)), t.int = (r) => t.check(He(r)), t.safe = (r) => t.check(He(r)), t.positive = (r) => t.check(We(0, r)), t.nonnegative = (r) => t.check(he(0, r)), t.negative = (r) => t.check(Me(0, r)), t.nonpositive = (r) => t.check(fe(0, r)), t.multipleOf = (r, i) => t.check(Ve(r, i)), t.step = (r, i) => t.check(Ve(r, i)), t.finite = () => t;
  const n = t._zod.bag;
  t.minValue = Math.max(n.minimum ?? Number.NEGATIVE_INFINITY, n.exclusiveMinimum ?? Number.NEGATIVE_INFINITY) ?? null, t.maxValue = Math.min(n.maximum ?? Number.POSITIVE_INFINITY, n.exclusiveMaximum ?? Number.POSITIVE_INFINITY) ?? null, t.isInt = (n.format ?? "").includes("int") || Number.isSafeInteger(n.multipleOf ?? 0.5), t.isFinite = !0, t.format = n.format ?? null;
});
function Rt(t) {
  return cs(zn, t);
}
const so = /* @__PURE__ */ N("ZodNumberFormat", (t, e) => {
  pi.init(t, e), zn.init(t, e);
});
function He(t) {
  return ls(so, t);
}
const oo = /* @__PURE__ */ N("ZodBoolean", (t, e) => {
  mi.init(t, e), ft.init(t, e);
});
function At(t) {
  return fs(oo, t);
}
const ao = /* @__PURE__ */ N("ZodAny", (t, e) => {
  _i.init(t, e), ft.init(t, e);
});
function uo() {
  return hs(ao);
}
const co = /* @__PURE__ */ N("ZodUnknown", (t, e) => {
  gi.init(t, e), ft.init(t, e);
});
function se() {
  return ds(co);
}
const lo = /* @__PURE__ */ N("ZodNever", (t, e) => {
  vi.init(t, e), ft.init(t, e);
});
function fo(t) {
  return ps(lo, t);
}
const ho = /* @__PURE__ */ N("ZodArray", (t, e) => {
  yi.init(t, e), ft.init(t, e), t.element = e.element, t.min = (n, r) => t.check(ie(n, r)), t.nonempty = (n) => t.check(ie(1, n)), t.max = (n, r) => t.check(yn(n, r)), t.length = (n, r) => t.check(wn(n, r)), t.unwrap = () => t.element;
});
function xe(t, e) {
  return Es(ho, t, e);
}
const po = /* @__PURE__ */ N("ZodObject", (t, e) => {
  wi.init(t, e), ft.init(t, e), at(t, "shape", () => e.shape), t.keyof = () => wo(Object.keys(t._zod.def.shape)), t.catchall = (n) => t.clone({ ...t._zod.def, catchall: n }), t.passthrough = () => t.clone({ ...t._zod.def, catchall: se() }), t.loose = () => t.clone({ ...t._zod.def, catchall: se() }), t.strict = () => t.clone({ ...t._zod.def, catchall: fo() }), t.strip = () => t.clone({ ...t._zod.def, catchall: void 0 }), t.extend = (n) => Gn(t, n), t.merge = (n) => Hn(t, n), t.pick = (n) => Wn(t, n), t.omit = (n) => Vn(t, n), t.partial = (...n) => Yn(xn, t, n[0]), t.required = (...n) => Kn(En, t, n[0]);
});
function Dt(t, e) {
  const n = {
    type: "object",
    get shape() {
      return Nt(this, "shape", t ? Bn(t) : {}), this.shape;
    },
    ...J(e)
  };
  return new po(n);
}
const mo = /* @__PURE__ */ N("ZodUnion", (t, e) => {
  bi.init(t, e), ft.init(t, e), t.options = e.options;
});
function _o(t, e) {
  return new mo({
    type: "union",
    options: t,
    ...J(e)
  });
}
const go = /* @__PURE__ */ N("ZodIntersection", (t, e) => {
  ki.init(t, e), ft.init(t, e);
});
function vo(t, e) {
  return new go({
    type: "intersection",
    left: t,
    right: e
  });
}
const yo = /* @__PURE__ */ N("ZodRecord", (t, e) => {
  zi.init(t, e), ft.init(t, e), t.keyType = e.keyType, t.valueType = e.valueType;
});
function Ee(t, e, n) {
  return new yo({
    type: "record",
    keyType: t,
    valueType: e,
    ...J(n)
  });
}
const ge = /* @__PURE__ */ N("ZodEnum", (t, e) => {
  xi.init(t, e), ft.init(t, e), t.enum = e.entries, t.options = Object.values(e.entries);
  const n = new Set(Object.keys(e.entries));
  t.extract = (r, i) => {
    const s = {};
    for (const o of r)
      if (n.has(o))
        s[o] = e.entries[o];
      else
        throw new Error(`Key ${o} not found in enum`);
    return new ge({
      ...e,
      checks: [],
      ...J(i),
      entries: s
    });
  }, t.exclude = (r, i) => {
    const s = { ...e.entries };
    for (const o of r)
      if (n.has(o))
        delete s[o];
      else
        throw new Error(`Key ${o} not found in enum`);
    return new ge({
      ...e,
      checks: [],
      ...J(i),
      entries: s
    });
  };
});
function wo(t, e) {
  const n = Array.isArray(t) ? Object.fromEntries(t.map((r) => [r, r])) : t;
  return new ge({
    type: "enum",
    entries: n,
    ...J(e)
  });
}
const bo = /* @__PURE__ */ N("ZodTransform", (t, e) => {
  Ei.init(t, e), ft.init(t, e), t._zod.parse = (n, r) => {
    n.addIssue = (s) => {
      if (typeof s == "string")
        n.issues.push(Gt(s, n.value, e));
      else {
        const o = s;
        o.fatal && (o.continue = !1), o.code ?? (o.code = "custom"), o.input ?? (o.input = n.value), o.inst ?? (o.inst = t), n.issues.push(Gt(o));
      }
    };
    const i = e.transform(n.value, n);
    return i instanceof Promise ? i.then((s) => (n.value = s, n)) : (n.value = i, n);
  };
});
function ko(t) {
  return new bo({
    type: "transform",
    transform: t
  });
}
const xn = /* @__PURE__ */ N("ZodOptional", (t, e) => {
  Ii.init(t, e), ft.init(t, e), t.unwrap = () => t._zod.def.innerType;
});
function Ye(t) {
  return new xn({
    type: "optional",
    innerType: t
  });
}
const zo = /* @__PURE__ */ N("ZodNullable", (t, e) => {
  Si.init(t, e), ft.init(t, e), t.unwrap = () => t._zod.def.innerType;
});
function Ke(t) {
  return new zo({
    type: "nullable",
    innerType: t
  });
}
const xo = /* @__PURE__ */ N("ZodDefault", (t, e) => {
  Ai.init(t, e), ft.init(t, e), t.unwrap = () => t._zod.def.innerType, t.removeDefault = t.unwrap;
});
function Eo(t, e) {
  return new xo({
    type: "default",
    innerType: t,
    get defaultValue() {
      return typeof e == "function" ? e() : e;
    }
  });
}
const Io = /* @__PURE__ */ N("ZodPrefault", (t, e) => {
  Oi.init(t, e), ft.init(t, e), t.unwrap = () => t._zod.def.innerType;
});
function So(t, e) {
  return new Io({
    type: "prefault",
    innerType: t,
    get defaultValue() {
      return typeof e == "function" ? e() : e;
    }
  });
}
const En = /* @__PURE__ */ N("ZodNonOptional", (t, e) => {
  Ci.init(t, e), ft.init(t, e), t.unwrap = () => t._zod.def.innerType;
});
function Ao(t, e) {
  return new En({
    type: "nonoptional",
    innerType: t,
    ...J(e)
  });
}
const Oo = /* @__PURE__ */ N("ZodCatch", (t, e) => {
  Zi.init(t, e), ft.init(t, e), t.unwrap = () => t._zod.def.innerType, t.removeCatch = t.unwrap;
});
function Co(t, e) {
  return new Oo({
    type: "catch",
    innerType: t,
    catchValue: typeof e == "function" ? e : () => e
  });
}
const Zo = /* @__PURE__ */ N("ZodPipe", (t, e) => {
  $i.init(t, e), ft.init(t, e), t.in = e.in, t.out = e.out;
});
function Je(t, e) {
  return new Zo({
    type: "pipe",
    in: t,
    out: e
    // ...util.normalizeParams(params),
  });
}
const $o = /* @__PURE__ */ N("ZodReadonly", (t, e) => {
  Ti.init(t, e), ft.init(t, e), t.unwrap = () => t._zod.def.innerType;
});
function To(t) {
  return new $o({
    type: "readonly",
    innerType: t
  });
}
const Ro = /* @__PURE__ */ N("ZodCustom", (t, e) => {
  Ri.init(t, e), ft.init(t, e);
});
function No(t, e = {}) {
  return Is(Ro, t, e);
}
function Fo(t) {
  return Ss(t);
}
const In = Rt().int().positive().describe("Bundle format version"), Sn = zt().regex(
  /^[a-zA-Z0-9][a-zA-Z0-9!#$&\-^_]*\/[a-zA-Z0-9][a-zA-Z0-9!#$&\-^_.]*$/,
  "Invalid MIME type format"
).describe("MIME type for file content"), An = zt().min(1).regex(/^\//, "Path must start with /").regex(/^\/[a-zA-Z0-9._\-/]*$/, "Path contains invalid characters").refine((t) => !t.includes("//"), "Path cannot contain double slashes").refine((t) => !t.includes("/./"), "Path cannot contain /./").refine((t) => !t.includes("/../"), "Path cannot contain /../").describe("Virtual file path within bundle"), Ie = zt().regex(
  /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(\.\d{3})?Z?$/,
  "Must be a valid ISO 8601 timestamp"
).describe("ISO 8601 timestamp"), On = Dt({
  path: An,
  length: Rt().int().min(0).describe("Size of file data in bytes"),
  contentType: Sn,
  compressed: At().optional().describe("Whether file data is compressed in ZIP"),
  uncompressedSize: Rt().int().min(0).optional().describe("Original uncompressed size"),
  lastModified: Ie.optional()
}), Cn = Ee(zt().min(1), An).describe("Mapping of entrypoint names to file paths"), ce = Dt({
  version: In,
  name: zt().min(1).optional().describe("Human-readable bundle name"),
  description: zt().optional().describe("Bundle description"),
  createdAt: Ie.optional().describe("Bundle creation timestamp"),
  entrypoints: Cn,
  files: xe(On).describe("Array of all files in bundle"),
  metadata: Ee(zt(), se()).optional().describe("Optional extensibility metadata")
}), Xo = Dt({
  includeWarnings: At().default(!0),
  includeInfo: At().default(!1),
  failFast: At().default(!1),
  maxBundleSize: Rt().int().min(1).optional(),
  maxFileCount: Rt().int().min(1).optional(),
  strictMimeTypes: At().default(!0),
  customRules: xe(uo()).default([])
  // Custom rules will be typed separately
}), qo = Dt({
  strictValidation: At().default(!0),
  validateFileReferences: At().default(!0),
  maxSize: Rt().int().min(1).optional()
}), Qo = Dt({
  compressionLevel: Rt().int().min(0).max(9).default(6),
  useZip64: At().default(!1),
  comment: zt().optional()
}), ta = Dt({
  version: In.default(1),
  name: zt().min(1).optional(),
  description: zt().optional(),
  metadata: Ee(zt(), se()).optional()
}), ea = Dt({
  contentType: Sn.optional(),
  compress: At().default(!0),
  replace: At().default(!1),
  lastModified: Ie.optional()
});
function Do(t) {
  const e = /* @__PURE__ */ new Map(), n = /* @__PURE__ */ new Map(), r = /* @__PURE__ */ new Map();
  for (const [l, m] of Object.entries(t))
    n.set(l, m), r.has(m) || r.set(m, []), r.get(m).push(l), e.has(l) || e.set(l, /* @__PURE__ */ new Set());
  for (const [l, m] of n) {
    const k = r.get(m) || [];
    for (const _ of k)
      _ !== l && e.get(l).add(_);
  }
  const i = /* @__PURE__ */ new Set(), s = /* @__PURE__ */ new Set(), o = [];
  function a(l, m) {
    if (s.has(l)) {
      const _ = m.indexOf(l);
      if (_ >= 0) {
        const y = m.slice(_).concat([l]);
        o.push(y.join(" -> "));
      }
      return;
    }
    if (i.has(l))
      return;
    i.add(l), s.add(l), m.push(l);
    const k = e.get(l) || /* @__PURE__ */ new Set();
    for (const _ of k)
      a(_, [...m]);
    s.delete(l), m.pop();
  }
  for (const l of Object.keys(t))
    i.has(l) || a(l, []);
  return o;
}
function Po(t, e = !0) {
  const n = new Tt();
  try {
    return ce.parse(t), n.build();
  } catch (r) {
    if (e)
      if (r && typeof r == "object" && "issues" in r) {
        const i = r;
        for (const s of i.issues)
          n.addError(s.message, "VALIDATION_ERROR", {
            field: s.path.join(".")
          });
      } else
        n.addError(
          r instanceof Error ? r.message : "Unknown validation error",
          "VALIDATION_ERROR",
          { field: "manifest" }
        );
    else
      n.addWarning(
        "Validation skipped in non-strict mode",
        "VALIDATION_WARNING",
        { field: "manifest" }
      );
    return n.build();
  }
}
function Bo(t, e) {
  const n = new Tt();
  for (const r of e.files) {
    const i = r.path.startsWith("/") ? r.path.slice(1) : r.path;
    t.file(i) ? r.length : n.addError(
      `File declared in manifest not found in ZIP archive: ${r.path}`,
      "FILE_NOT_FOUND",
      { filePath: r.path }
    );
  }
  if (e.entrypoints)
    for (const [r, i] of Object.entries(
      e.entrypoints
    ))
      e.files.some((o) => o.path === i) || n.addError(
        `Entrypoint '${r}' references file not declared in manifest: ${i}`,
        "ENTRYPOINT_INVALID",
        { entrypointName: r, filePath: i }
      );
  if (e.entrypoints) {
    const r = Do(
      e.entrypoints
    );
    r.length > 0 && n.addError(
      `Circular entrypoint references detected: ${r.join(", ")}`,
      "CIRCULAR_ENTRYPOINT_REFERENCE",
      { circularRefs: r, count: r.length }
    );
  }
  return n.build();
}
function jo(t, e, n) {
  const r = new Tt();
  n && t > n && r.addError(
    `Bundle size ${t} bytes exceeds maximum allowed size ${n} bytes`,
    "SIZE_EXCEEDED",
    { zipSize: t, maxSize: n }
  );
  const i = 100 * 1024 * 1024;
  t > i && r.addWarning(
    `Bundle size ${t} bytes is unusually large (>100MB)`,
    "SIZE_WARNING",
    { zipSize: t, warningSize: i }
  );
  let s = 0;
  for (const o of e.files)
    o.length !== void 0 && (s += o.length);
  return s > 0 && t > s * 2 && r.addWarning(
    `ZIP size ${t} is unusually large compared to declared file sizes ${s}`,
    "COMPRESSION_WARNING",
    { zipSize: t, declaredTotalSize: s }
  ), r.build();
}
function Zn(t, e, n, r = {}) {
  const {
    maxBundleSize: i,
    maxFileCount: s,
    strictMimeTypes: o = !1,
    includeWarnings: a = !0,
    includeInfo: l = !1,
    failFast: m = !1,
    customRules: k = []
  } = r, _ = new Tt(), y = {
    bundleSize: n,
    fileCount: e.files.length,
    entrypointCount: Object.keys(e.entrypoints || {}).length,
    timestamp: (/* @__PURE__ */ new Date()).toISOString()
  };
  try {
    if (Po(e, !0).messages.forEach((p) => {
      const d = { ...p.context, ...y };
      if (p.severity === mt.ERROR) {
        if (_.addError(p.message, p.code, d, p.filePath), m) return _.build();
      } else p.severity === mt.WARNING && a ? _.addWarning(
        p.message,
        p.code,
        d,
        p.filePath
      ) : p.severity === mt.INFO && l && _.addInfo(p.message, p.code, d, p.filePath);
    }), Bo(t, e).messages.forEach((p) => {
      const d = { ...p.context, ...y };
      if (p.severity === mt.ERROR) {
        if (_.addError(p.message, p.code, d, p.filePath), m) return _.build();
      } else p.severity === mt.WARNING && a ? _.addWarning(
        p.message,
        p.code,
        d,
        p.filePath
      ) : p.severity === mt.INFO && l && _.addInfo(p.message, p.code, d, p.filePath);
    }), jo(n, e, i).messages.forEach((p) => {
      const d = { ...p.context, ...y };
      if (p.severity === mt.ERROR) {
        if (_.addError(p.message, p.code, d, p.filePath), m) return _.build();
      } else p.severity === mt.WARNING && a ? _.addWarning(
        p.message,
        p.code,
        d,
        p.filePath
      ) : p.severity === mt.INFO && l && _.addInfo(p.message, p.code, d, p.filePath);
    }), s && e.files.length > s && (_.addError(
      `Bundle contains ${e.files.length} files, which exceeds the maximum allowed ${s}`,
      "FILE_COUNT_EXCEEDED",
      {
        actualCount: e.files.length,
        maxCount: s,
        ...y
      }
    ), m))
      return _.build();
    if (o) {
      const p = /^[a-zA-Z0-9][a-zA-Z0-9!#$&\-^_]*\/[a-zA-Z0-9][a-zA-Z0-9!#$&\-^_.]*$/;
      for (const d of e.files)
        if (d.contentType && !p.test(d.contentType) && (_.addError(
          `Invalid MIME type for file ${d.path}: "${d.contentType}"`,
          "INVALID_MIME_TYPE",
          {
            filePath: d.path,
            mimeType: d.contentType,
            ...y
          }
        ), m))
          return _.build();
    }
    for (const p of k)
      try {
        p.validate({
          manifest: e,
          zipData: void 0,
          // ZIP data not exposed in this context for security
          options: r,
          createMessage: (v, z, I, S) => ({
            severity: v,
            message: z,
            code: I,
            context: { ...S, rule: p.id, ...y }
          })
        }).forEach((v) => {
          if (v.severity === mt.ERROR) {
            if (_.addError(v.message, v.code, v.context), m) return _.build();
          } else v.severity === mt.WARNING && a ? _.addWarning(v.message, v.code, v.context) : v.severity === mt.INFO && l && _.addInfo(v.message, v.code, v.context);
        });
      } catch (d) {
        if (_.addError(
          `Custom validation rule "${p.id}" failed: ${d instanceof Error ? d.message : "Unknown error"}`,
          "CUSTOM_RULE_ERROR",
          {
            ruleId: p.id,
            error: d instanceof Error ? d.message : String(d),
            ...y
          }
        ), m) return _.build();
      }
    l && _.addInfo(
      `Bundle validation completed: ${e.files.length} files, ${Object.keys(e.entrypoints || {}).length} entrypoints`,
      "VALIDATION_SUMMARY",
      y
    );
  } catch (f) {
    const w = f instanceof Error ? f.message : "Unknown validation error", h = f instanceof Error ? f.stack : void 0;
    _.addError(
      `Validation process failed: ${w}`,
      "VALIDATION_PROCESS_ERROR",
      {
        error: w,
        stack: h,
        ...y
      }
    );
  }
  return _.build();
}
function na(t, e, n, r = {}) {
  const { maxSize: i } = r;
  return Zn(t, e, n, {
    maxBundleSize: i,
    includeWarnings: !0,
    includeInfo: !1,
    failFast: !1
  });
}
function ra(t, e = {}) {
  const {
    includeContext: n = !0,
    includeErrorCodes: r = !0,
    includeFilePaths: i = !0,
    maxContextLength: s = 200
  } = e;
  if (t.length === 0)
    return "No validation messages";
  const o = /* @__PURE__ */ new Map();
  for (const l of t) {
    const m = l.code.split("_")[0];
    o.has(m) || o.set(m, []), o.get(m).push(l);
  }
  const a = [];
  for (const [l, m] of o) {
    a.push(
      `
${l.toUpperCase()} (${m.length} ${m.length === 1 ? "issue" : "issues"}):`
    );
    for (const k of m) {
      let y = `  ${k.severity === mt.ERROR ? "❌" : k.severity === mt.WARNING ? "⚠️" : "ℹ️"} ${k.message}`;
      if (r && (y += ` [${k.code}]`), i && k.filePath && (y += ` (${k.filePath})`), a.push(y), n && k.context && Object.keys(k.context).length > 0) {
        const f = JSON.stringify(k.context, null, 2), w = f.length > s ? f.substring(0, s) + "..." : f;
        a.push(`    Context: ${w}`);
      }
      k.suggestion && a.push(`    💡 Suggestion: ${k.suggestion}`);
    }
  }
  return a.join(`
`);
}
function ia(t) {
  if (t.length === 0)
    return "No validation errors";
  const e = /* @__PURE__ */ new Map();
  for (const r of t) {
    const i = r.code.split("_")[0];
    e.has(i) || e.set(i, []), e.get(i).push(r);
  }
  const n = [];
  for (const [r, i] of e) {
    n.push(`${r}:`);
    for (const s of i) {
      const o = s.severity === mt.ERROR ? "❌" : s.severity === mt.WARNING ? "⚠️" : "ℹ️";
      n.push(`  ${o} ${s.message}`);
    }
  }
  return n.join(`
`);
}
function sa(t, e = {}) {
  const {
    includeSuccessSummary: n = !0,
    includeSuggestions: r = !0,
    includeDetailedContext: i = !1,
    groupByFile: s = !1
  } = e, o = [];
  o.push("📋 Bundle Validation Report"), o.push("=".repeat(50)), o.push(""), t.valid && n ? (o.push("✅ Validation Status: PASSED"), o.push(`📊 Total checks: ${t.messages.length}`), o.push("")) : t.valid || (o.push("❌ Validation Status: FAILED"), o.push(
    `📊 Errors: ${t.errors.length}, Warnings: ${t.warnings.length}, Info: ${t.info.length}`
  ), o.push(""));
  let a;
  if (s) {
    a = /* @__PURE__ */ new Map();
    for (const l of t.messages) {
      const m = l.filePath || "(Bundle-level)";
      a.has(m) || a.set(m, []), a.get(m).push(l);
    }
  } else {
    a = /* @__PURE__ */ new Map();
    for (const l of t.messages) {
      const m = l.code.split("_")[0];
      a.has(m) || a.set(m, []), a.get(m).push(l);
    }
  }
  for (const [l, m] of a)
    if (m.length !== 0) {
      o.push(`📂 ${l.toUpperCase()}`), o.push("-".repeat(30));
      for (const k of m) {
        const _ = k.severity === mt.ERROR ? "❌" : k.severity === mt.WARNING ? "⚠️" : "ℹ️";
        o.push(`${_} ${k.message}`), k.filePath && !s && o.push(`   📁 File: ${k.filePath}`), i && k.context && o.push(
          `   🔍 Context: ${JSON.stringify(k.context, null, 4)}`
        ), r && k.suggestion && o.push(`   💡 Suggestion: ${k.suggestion}`), o.push("");
      }
    }
  return r && !t.valid && (o.push("🔧 Recommended Actions:"), o.push("-".repeat(30)), t.errors.length > 0 && o.push("1. Fix all errors before proceeding with bundle operations"), t.warnings.length > 0 && o.push("2. Review warnings to ensure optimal bundle configuration"), o.push("3. Re-run validation after making changes"), o.push(
    "4. Consider using stricter validation settings for production bundles"
  )), o.join(`
`);
}
class Ct extends ve {
  /**
   * Create a new ZipBundle instance
   * @param zip - JSZip instance containing the bundle data
   * @param manifest - Validated bundle manifest
   * @param sourceData - Original source data (optional)
   */
  constructor(e, n, r) {
    super(), this._sourceData = null, this.zip = e, this._manifest = n, this._sourceData = r || null;
  }
  get manifest() {
    return this._manifest;
  }
  get data() {
    return this._sourceData;
  }
  // File Access Methods
  getFile(e) {
    return this._manifest.files.find((n) => n.path === e) || null;
  }
  async getFileData(e) {
    const n = e.startsWith("/") ? e.slice(1) : e, r = this.zip.file(n);
    if (!r)
      return null;
    try {
      return await r.async("arraybuffer");
    } catch (i) {
      throw new gt(
        `Failed to read file data for ${e}: ${i instanceof Error ? i.message : "Unknown error"}`
      );
    }
  }
  hasFile(e) {
    return this._manifest.files.some((n) => n.path === e);
  }
  listFiles() {
    return [...this._manifest.files];
  }
  getFileCount() {
    return this._manifest.files.length;
  }
  // Entrypoint Methods
  getEntrypoint(e) {
    return this._manifest.entrypoints[e] || null;
  }
  hasEntrypoint(e) {
    return e in this._manifest.entrypoints;
  }
  listEntrypoints() {
    return { ...this._manifest.entrypoints };
  }
  getEntrypointNames() {
    return Object.keys(this._manifest.entrypoints);
  }
  // File Modification Methods
  async addFile(e, n, r = {}) {
    const {
      replace: i = !1,
      compress: s = !0,
      contentType: o,
      lastModified: a
    } = r;
    if (this.hasFile(e.path) && !i)
      throw new bt(
        `File ${e.path} already exists. Set replace=true to overwrite.`
      );
    const l = {
      ...e,
      length: n.byteLength,
      contentType: o || e.contentType,
      compressed: s,
      lastModified: a || (/* @__PURE__ */ new Date()).toISOString()
    }, m = e.path.startsWith("/") ? e.path.slice(1) : e.path;
    if (this.zip.file(m, n, {
      compression: s ? "DEFLATE" : "STORE",
      date: a ? new Date(a) : /* @__PURE__ */ new Date()
    }), i) {
      const k = this._manifest.files.findIndex((_) => _.path === e.path);
      k >= 0 ? this._manifest.files[k] = l : this._manifest.files.push(l);
    } else
      this._manifest.files.push(l);
    await this.updateManifestInZip();
  }
  async updateFile(e, n, r) {
    const i = this.getFile(e);
    if (!i)
      throw new Mt(`File ${e} not found`);
    const s = {
      ...i,
      length: n.byteLength,
      contentType: r || i.contentType,
      lastModified: (/* @__PURE__ */ new Date()).toISOString()
    }, o = e.startsWith("/") ? e.slice(1) : e;
    this.zip.file(o, n, {
      compression: i.compressed ? "DEFLATE" : "STORE",
      date: /* @__PURE__ */ new Date()
    });
    const a = this._manifest.files.findIndex((l) => l.path === e);
    a >= 0 && (this._manifest.files[a] = s), await this.updateManifestInZip();
  }
  async removeFile(e) {
    if (!this.hasFile(e))
      throw new Mt(`File ${e} not found`);
    const n = e.startsWith("/") ? e.slice(1) : e;
    this.zip.remove(n), this._manifest.files = this._manifest.files.filter((r) => r.path !== e);
    for (const [r, i] of Object.entries(
      this._manifest.entrypoints
    ))
      i === e && delete this._manifest.entrypoints[r];
    await this.updateManifestInZip();
  }
  // Entrypoint Modification Methods
  setEntrypoint(e, n) {
    if (!this.hasFile(n))
      throw new Mt(`Target file ${n} not found`);
    this._manifest.entrypoints[e] = n;
  }
  removeEntrypoint(e) {
    if (!this.hasEntrypoint(e))
      throw new bt(`Entrypoint ${e} not found`);
    delete this._manifest.entrypoints[e];
  }
  // Validation Methods
  validate(e = {}) {
    const n = this.estimateBundleSize();
    return Zn(
      this.zip,
      this._manifest,
      n,
      e
    );
  }
  isValid(e = {}) {
    return this.validate(e).valid;
  }
  // Compression Methods
  isFileCompressed(e) {
    const n = this.getFile(e);
    if (!n)
      throw new Mt(`File ${e} not found`);
    return n.compressed || !1;
  }
  getUncompressedSize(e) {
    const n = this.getFile(e);
    if (!n)
      throw new Mt(`File ${e} not found`);
    return n.uncompressedSize || null;
  }
  // Utility Methods
  getBundleInfo() {
    const e = this.estimateBundleSize(), n = this._manifest.files.filter(
      (i) => i.compressed
    ).length, r = this._manifest.files.reduce(
      (i, s) => i + (s.uncompressedSize || s.length),
      0
    );
    return {
      version: this._manifest.version,
      name: this._manifest.name,
      fileCount: this._manifest.files.length,
      totalSize: e,
      compressedFiles: n,
      entrypoints: Object.keys(this._manifest.entrypoints),
      uncompressedSize: r,
      createdAt: this._manifest.createdAt
    };
  }
  estimateBundleSize() {
    const e = this._manifest.files.reduce(
      (i, s) => i + s.length,
      0
    ), n = JSON.stringify(this._manifest).length, r = Math.floor(e * 0.1);
    return e + n + r;
  }
  async clone() {
    const e = await this.toArrayBuffer();
    return await Ct.parse(e);
  }
  async merge(e, n = {}) {
    const {
      conflictResolution: r = "error",
      entrypointConflictResolution: i = "error"
    } = n, s = await this.clone();
    for (const a of e.listFiles()) {
      if (s.getFile(a.path)) {
        if (r === "error")
          throw new bt(
            `File conflict during merge: ${a.path} exists in both bundles`
          );
        if (r === "skip")
          continue;
      }
      const m = await e.getFileData(a.path);
      m && await s.addFile(a, m, { replace: !0 });
    }
    const o = e.listEntrypoints();
    for (const [a, l] of Object.entries(o)) {
      if (s.getEntrypoint(a)) {
        if (i === "error")
          throw new bt(
            `Entrypoint conflict during merge: ${a} exists in both bundles`
          );
        if (i === "skip")
          continue;
      }
      s.hasFile(l) ? s.setEntrypoint(a, l) : console.warn(
        `Skipping entrypoint ${a} because target file ${l} doesn't exist`
      );
    }
    return s;
  }
  // Serialization Methods
  async toArrayBuffer(e = {}) {
    const { compressionLevel: n = 6, useZip64: r = !1, comment: i } = e;
    await this.updateManifestInZip();
    try {
      return await this.zip.generateAsync({
        type: "arraybuffer",
        compression: "DEFLATE",
        compressionOptions: {
          level: n
        },
        platform: r ? "UNIX" : "DOS",
        comment: i
      });
    } catch (s) {
      throw new gt(
        `Failed to serialize bundle: ${s instanceof Error ? s.message : "Unknown error"}`
      );
    }
  }
  // Private helper methods
  async updateManifestInZip() {
    const e = JSON.stringify(this._manifest, null, 2);
    this.zip.file("manifest.json", e);
  }
  // Static Factory Methods
  static async createEmpty(e = {}) {
    const { version: n = 1 } = e, r = {
      version: n,
      createdAt: (/* @__PURE__ */ new Date()).toISOString(),
      entrypoints: {},
      files: []
    }, i = new de(), s = JSON.stringify(r, null, 2);
    return i.file("manifest.json", s), new Ct(i, r);
  }
  static async fromFiles(e, n = {}) {
    const r = await Ct.createEmpty(), { contentTypes: i = /* @__PURE__ */ new Map() } = n;
    for (const [s, o] of e) {
      const a = i.get(s) || "application/octet-stream", l = {
        path: s.startsWith("/") ? s : "/" + s,
        contentType: a,
        compressed: !0,
        lastModified: (/* @__PURE__ */ new Date()).toISOString()
      };
      await r.addFile(l, o);
    }
    return r;
  }
  static async parse(e, n = {}) {
    const {
      strictValidation: r = !0,
      validateFileReferences: i = !0,
      maxSize: s
    } = n;
    if (s && e.byteLength > s)
      throw new gt(
        `Bundle size ${e.byteLength} exceeds maximum allowed size ${s}`
      );
    try {
      const o = await de.loadAsync(e), a = o.file("manifest.json");
      if (!a)
        throw new gt("No manifest.json found in bundle");
      const l = await a.async("text");
      let m;
      try {
        m = JSON.parse(l);
      } catch (y) {
        throw new gt(
          `Invalid manifest JSON: ${y instanceof Error ? y.message : "Unknown error"}`
        );
      }
      let k;
      try {
        k = ce.parse(m);
      } catch (y) {
        if (r)
          throw new gt(
            `Manifest validation failed: ${y instanceof Error ? y.message : "Unknown error"}`
          );
        k = m;
      }
      const _ = new Ct(o, k, e);
      if (i) {
        const y = _.validate();
        if (!y.valid && r)
          throw new bt(
            `Bundle validation failed: ${y.errors.map((f) => f.message).join(", ")}`
          );
      }
      return _;
    } catch (o) {
      throw o instanceof gt || o instanceof bt ? o : new gt(
        `Failed to parse bundle: ${o instanceof Error ? o.message : "Unknown error"}`
      );
    }
  }
}
function oa(t, e = {}) {
  const n = new Tt();
  try {
    const r = ce.parse(t);
    Uo(r, n, e), n.hasErrors() || n.addInfo(
      "Manifest validation passed",
      vt.ZOD_SCHEMA_VALIDATION
    );
  } catch (r) {
    r instanceof ze ? Se(r, n) : n.addError(
      `Unexpected validation error: ${r instanceof Error ? r.message : "Unknown error"}`,
      vt.ZOD_SCHEMA_VALIDATION
    );
  }
  return n.build();
}
function aa(t) {
  const e = new Tt();
  try {
    On.parse(t), e.addInfo(
      "File metadata validation passed",
      vt.ZOD_SCHEMA_VALIDATION
    );
  } catch (n) {
    n instanceof ze ? Se(n, e) : e.addError(
      `Unexpected file validation error: ${n instanceof Error ? n.message : "Unknown error"}`,
      vt.ZOD_SCHEMA_VALIDATION
    );
  }
  return e.build();
}
function ua(t) {
  const e = new Tt();
  try {
    Cn.parse(t), e.addInfo(
      "Entrypoints validation passed",
      vt.ZOD_SCHEMA_VALIDATION
    );
  } catch (n) {
    n instanceof ze ? Se(n, e) : e.addError(
      `Unexpected entrypoints validation error: ${n instanceof Error ? n.message : "Unknown error"}`,
      vt.ZOD_SCHEMA_VALIDATION
    );
  }
  return e.build();
}
function Uo(t, e, n) {
  const r = /* @__PURE__ */ new Set(), i = [];
  for (const a of t.files)
    r.has(a.path) ? i.push(a.path) : r.add(a.path);
  i.length > 0 && e.addError(
    `Duplicate file paths found: ${i.join(", ")}`,
    vt.UNIQUE_FILE_PATHS,
    { duplicatePaths: i }
  );
  const s = [];
  for (const [a, l] of Object.entries(
    t.entrypoints
  ))
    r.has(l) || s.push(`${a} -> ${l}`);
  s.length > 0 && e.addError(
    `Entrypoints reference non-existent files: ${s.join(", ")}`,
    vt.VALID_ENTRYPOINTS,
    { invalidEntrypoints: s }
  ), n.maxFileCount && t.files.length > n.maxFileCount && e.addError(
    `Bundle contains ${t.files.length} files, but maximum allowed is ${n.maxFileCount}`,
    vt.FILE_COUNT_LIMIT,
    { fileCount: t.files.length, maxFileCount: n.maxFileCount }
  );
  const o = Wo(t.entrypoints);
  o.length > 0 && e.addWarning(
    `Potential circular entrypoint references detected: ${o.join(", ")}`,
    vt.VALID_ENTRYPOINTS,
    { circularReferences: o }
  ), t.version < 1 && e.addError(
    `Bundle version ${t.version} is not supported (minimum version is 1)`,
    vt.VALID_VERSION,
    { version: t.version, minimumVersion: 1 }
  ), t.version > 1 && e.addWarning(
    `Bundle version ${t.version} is newer than expected (current version is 1)`,
    vt.VALID_VERSION,
    { version: t.version, currentVersion: 1 }
  );
}
function Se(t, e) {
  for (const n of t.issues) {
    const r = Lo(n), i = {
      path: n.path,
      zodCode: n.code,
      received: n.received,
      // Zod 4 compatibility
      expected: Mo(n)
    };
    e.addError(r, vt.ZOD_SCHEMA_VALIDATION, i);
  }
}
function Lo(t) {
  const e = t.path.length > 0 ? ` at path "${t.path.join(".")}"` : "";
  switch (t.code) {
    case "invalid_type":
      return `Expected ${t.expected} but received ${t.received}${e}`;
    case "too_small":
      return `Value${e} is too small: ${t.message}`;
    case "too_big":
      return `Value${e} is too big: ${t.message}`;
    case "custom":
      return `Validation failed${e}: ${t.message}`;
    default:
      return `Validation error${e}: ${t.message}`;
  }
}
function Mo(t) {
  switch (t.code) {
    case "invalid_type":
      return t.expected;
    case "too_small":
      return `>= ${t.minimum}`;
    case "too_big":
      return `<= ${t.maximum}`;
    default:
      return;
  }
}
function Wo(t) {
  const e = [], n = /* @__PURE__ */ new Map();
  for (const [r, i] of Object.entries(t)) {
    const s = n.get(i);
    s && t[s] === t[r] && e.push(`${r} <-> ${s}`), n.set(i, r);
  }
  return e;
}
function ca(t, e, n, r, i) {
  return {
    severity: t,
    message: e,
    code: n,
    context: r,
    filePath: i
  };
}
function la(t, e) {
  const n = new Tt(), r = new Set(
    e.filter((a) => a !== "manifest.json")
  ), i = new Set(t.files.map((a) => a.path)), s = [...i].filter(
    (a) => !r.has(a.slice(1))
  );
  s.length > 0 && n.addError(
    `Files listed in manifest but missing from ZIP: ${s.join(", ")}`,
    vt.MANIFEST_ZIP_CONSISTENCY,
    { missingFiles: s }
  );
  const o = [...r].filter(
    (a) => !i.has("/" + a)
  );
  return o.length > 0 && n.addWarning(
    `Files in ZIP but not listed in manifest: ${o.join(", ")}`,
    vt.MANIFEST_ZIP_CONSISTENCY,
    { extraFiles: o }
  ), n.build();
}
async function fa(t, e = {}) {
  return Ct.parse(t, e);
}
async function ha(t, e = !0) {
  const n = t.file("manifest.json");
  if (!n)
    throw gt.missingManifest();
  let r;
  try {
    r = await n.async("text");
  } catch (s) {
    throw new gt(
      `Failed to read manifest.json: ${s instanceof Error ? s.message : "Unknown error"}`
    );
  }
  let i;
  try {
    i = JSON.parse(r);
  } catch (s) {
    throw gt.invalidManifestJson(s);
  }
  try {
    return ce.parse(i);
  } catch (s) {
    if (e)
      throw new gt(
        `Manifest validation failed: ${s instanceof Error ? s.message : "Unknown error"}`
      );
    return i;
  }
}
function da(t, e) {
  const n = /* @__PURE__ */ new Map();
  for (const r of e.files) {
    const i = r.path.startsWith("/") ? r.path.slice(1) : r.path, s = t.file(i);
    s && n.set(r.path, s);
  }
  return n;
}
function pa(t, e) {
  const n = [];
  for (const r of e.files) {
    const i = r.path.startsWith("/") ? r.path.slice(1) : r.path;
    t.file(i) || n.push(
      `File referenced in manifest not found in ZIP: ${r.path}`
    );
  }
  if (e.entrypoints)
    for (const [r, i] of Object.entries(
      e.entrypoints
    ))
      e.files.some((o) => o.path === i) || n.push(
        `Entrypoint '${r}' references non-existent file: ${i}`
      );
  return n;
}
function ma(t, e, n) {
  return new Ct(t, e, n);
}
async function _a(t) {
  try {
    return await de.loadAsync(t), !0;
  } catch {
    return !1;
  }
}
const Vo = zt().min(2, "Path must be at least 2 characters").regex(/^\/[a-zA-Z0-9._-]+(\/[a-zA-Z0-9._-]+)*$/, {
  message: "Invalid virtual path format"
}).refine((t) => !t.includes("//"), {
  message: "Path cannot contain empty segments (//)"
}).refine((t) => !t.includes("/."), {
  message: "Path cannot contain relative path components (. or ..)"
});
function ga(t) {
  try {
    return Vo.parse(t), !0;
  } catch {
    return !1;
  }
}
function va(t) {
  return t.startsWith("/") ? t : "/" + t;
}
function ya(t) {
  const e = t.lastIndexOf("/");
  return e <= 0 ? "/" : t.substring(0, e);
}
function Go(t) {
  const e = t.lastIndexOf("/");
  return t.substring(e + 1);
}
function wa(t) {
  const e = Go(t), n = e.lastIndexOf(".");
  return n <= 0 ? "" : e.substring(n);
}
function ba(...t) {
  const e = [];
  for (const n of t)
    if (n === "/")
      e.length === 0 && e.push("");
    else if (n) {
      const r = n.replace(/^\/+|\/+$/g, "");
      r && e.push(r);
    }
  return (e.length === 0 || e[0] !== "") && e.unshift(""), e.join("/") || "/";
}
async function ka(t = {}) {
  return await Ct.createEmpty(t);
}
function za(t) {
  return t.getBundleInfo();
}
async function xa(t) {
  return t.estimateBundleSize();
}
function Ho(t) {
  const e = t.toLowerCase().split(".").pop();
  return {
    // Text
    html: "text/html",
    htm: "text/html",
    css: "text/css",
    js: "application/javascript",
    mjs: "application/javascript",
    json: "application/json",
    txt: "text/plain",
    xml: "application/xml",
    // Images
    png: "image/png",
    jpg: "image/jpeg",
    jpeg: "image/jpeg",
    gif: "image/gif",
    svg: "image/svg+xml",
    webp: "image/webp",
    ico: "image/x-icon",
    // Fonts
    woff: "font/woff",
    woff2: "font/woff2",
    ttf: "font/ttf",
    otf: "font/otf",
    // Documents
    pdf: "application/pdf",
    // Audio/Video
    mp3: "audio/mpeg",
    mp4: "video/mp4",
    webm: "video/webm",
    ogg: "audio/ogg",
    // Archives
    zip: "application/zip",
    tar: "application/x-tar",
    gz: "application/gzip",
    // Programming languages
    ts: "text/typescript",
    tsx: "text/typescript",
    jsx: "text/javascript",
    py: "text/x-python",
    rs: "text/x-rust",
    go: "text/x-go",
    java: "text/x-java",
    c: "text/x-c",
    cpp: "text/x-c++",
    h: "text/x-c",
    hpp: "text/x-c++",
    cs: "text/x-csharp",
    php: "text/x-php",
    rb: "text/x-ruby",
    swift: "text/x-swift",
    kt: "text/x-kotlin",
    // Web Assembly
    wasm: "application/wasm"
  }[e || ""] || "application/octet-stream";
}
function Ea(t, e = 2) {
  if (t === 0) return "0 Bytes";
  const n = 1024, r = e < 0 ? 0 : e, i = ["Bytes", "KB", "MB", "GB", "TB"], s = Math.floor(Math.log(t) / Math.log(n));
  return parseFloat((t / Math.pow(n, s)).toFixed(r)) + " " + i[s];
}
async function Ia(t, e = {}) {
  const { contentTypes: n = /* @__PURE__ */ new Map(), autoDetectTypes: r = !0 } = e;
  if (r)
    for (const [i] of t)
      n.has(i) || n.set(i, Ho(i));
  return await Ct.fromFiles(t, { contentTypes: n });
}
async function Sa(t, e = {}) {
  if (t.length === 0)
    throw new Error("No bundles provided to merge");
  if (t.length === 1)
    return t[0].clone();
  let n = await t[0].clone();
  for (let r = 1; r < t.length; r++)
    n = await n.merge(t[r], e);
  return n;
}
export {
  ea as AddFileOptionsSchema,
  ve as Bundle,
  Et as BundleError,
  On as BundleFileSchema,
  ce as BundleManifestSchema,
  gt as BundleParseError,
  Xe as BundleSizeError,
  bt as BundleValidationError,
  In as BundleVersionSchema,
  Qe as CircularReferenceError,
  ta as CreateBundleOptionsSchema,
  Jo as EnhancedBundleError,
  Cn as EntrypointMapSchema,
  Yo as EntrypointNotFoundError,
  Mt as FileNotFoundError,
  Sn as MimeTypeSchema,
  qo as ParseOptionsSchema,
  qe as SchemaValidationError,
  Qo as SerializationOptionsSchema,
  Ko as UnsupportedVersionError,
  tn as ValidationContextError,
  Xo as ValidationOptionsSchema,
  Tt as ValidationResultBuilder,
  vt as ValidationRules,
  mt as ValidationSeverity,
  An as VirtualPathSchema,
  Ct as ZipBundle,
  ne as ZipOperationError,
  Go as basename,
  da as buildFileMap,
  Ia as createBundleFromFiles,
  ma as createBundleFromZip,
  ka as createEmptyBundle,
  ca as createValidationMessage,
  Do as detectCircularEntrypointReferences,
  ya as dirname,
  xa as estimateBundleSize,
  wa as extname,
  ha as extractManifest,
  Ea as formatBytes,
  ia as formatValidationErrors,
  ra as formatValidationErrorsDetailed,
  sa as generateValidationReport,
  za as getBundleInfo,
  Ho as guessMimeType,
  _a as isValidZip,
  ba as join,
  Sa as mergeBundles,
  va as normalizePath,
  fa as parseBundle,
  na as validateBundle,
  Zn as validateBundleComprehensive,
  aa as validateBundleFile,
  jo as validateBundleSize,
  ua as validateEntrypoints,
  pa as validateFileReferences,
  oa as validateManifest,
  Po as validateManifestData,
  la as validateManifestZipConsistency,
  ga as validatePath,
  Bo as validateZipManifestConsistency
};
//# sourceMappingURL=index.js.map
