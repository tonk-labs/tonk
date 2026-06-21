// chunks/chunk.R7QX4M6R.js
var MirrorValidator = () => {
  return {
    checkValidity(element) {
      const formControl = element.input;
      const validity = {
        message: "",
        isValid: true,
        invalidKeys: []
      };
      if (!formControl) {
        return validity;
      }
      let isValid = true;
      if ("checkValidity" in formControl) {
        isValid = formControl.checkValidity();
      }
      if (isValid) {
        return validity;
      }
      validity.isValid = false;
      if ("validationMessage" in formControl) {
        validity.message = formControl.validationMessage;
      }
      if (!("validity" in formControl)) {
        validity.invalidKeys.push("customError");
        return validity;
      }
      for (const key in formControl.validity) {
        if (key === "valid") {
          continue;
        }
        const checkedKey = key;
        if (formControl.validity[checkedKey]) {
          validity.invalidKeys.push(checkedKey);
        }
      }
      return validity;
    }
  };
};

// chunks/chunk.VC3BPUZJ.js
var WaInvalidEvent = class extends Event {
  constructor() {
    super("wa-invalid", { bubbles: true, cancelable: false, composed: true });
  }
};

// chunks/chunk.BKE5EYM3.js
var t = globalThis;
var i = t.trustedTypes;
var s = i ? i.createPolicy("lit-html", { createHTML: (t23) => t23 }) : void 0;
var e = "$lit$";
var h = `lit$${Math.random().toFixed(9).slice(2)}$`;
var o = "?" + h;
var n = `<${o}>`;
var r = document;
var l = () => r.createComment("");
var c = (t23) => t23 === null || typeof t23 != "object" && typeof t23 != "function";
var a = Array.isArray;
var u = (t23) => a(t23) || typeof t23?.[Symbol.iterator] == "function";
var d = "[ 	\n\f\r]";
var f = /<(?:(!--|\/[^a-zA-Z])|(\/?[a-zA-Z][^>\s]*)|(\/?$))/g;
var v = /-->/g;
var _ = />/g;
var m = RegExp(`>|${d}(?:([^\\s"'>=/]+)(${d}*=${d}*(?:[^ 	
\f\r"'\`<>=]|("|')|))|$)`, "g");
var p = /'/g;
var g = /"/g;
var $ = /^(?:script|style|textarea|title)$/i;
var y = (t23) => (i24, ...s23) => ({ _$litType$: t23, strings: i24, values: s23 });
var x = y(1);
var b = y(2);
var w = y(3);
var T = Symbol.for("lit-noChange");
var E = Symbol.for("lit-nothing");
var A = /* @__PURE__ */ new WeakMap();
var C = r.createTreeWalker(r, 129);
function P(t23, i24) {
  if (!a(t23) || !t23.hasOwnProperty("raw"))
    throw Error("invalid template strings array");
  return s !== void 0 ? s.createHTML(i24) : i24;
}
var V = (t23, i24) => {
  const s23 = t23.length - 1, o25 = [];
  let r24, l22 = i24 === 2 ? "<svg>" : i24 === 3 ? "<math>" : "", c23 = f;
  for (let i32 = 0; i32 < s23; i32++) {
    const s3 = t23[i32];
    let a22, u22, d22 = -1, y22 = 0;
    for (; y22 < s3.length && (c23.lastIndex = y22, u22 = c23.exec(s3), u22 !== null); )
      y22 = c23.lastIndex, c23 === f ? u22[1] === "!--" ? c23 = v : u22[1] !== void 0 ? c23 = _ : u22[2] !== void 0 ? ($.test(u22[2]) && (r24 = RegExp("</" + u22[2], "g")), c23 = m) : u22[3] !== void 0 && (c23 = m) : c23 === m ? u22[0] === ">" ? (c23 = r24 ?? f, d22 = -1) : u22[1] === void 0 ? d22 = -2 : (d22 = c23.lastIndex - u22[2].length, a22 = u22[1], c23 = u22[3] === void 0 ? m : u22[3] === '"' ? g : p) : c23 === g || c23 === p ? c23 = m : c23 === v || c23 === _ ? c23 = f : (c23 = m, r24 = void 0);
    const x2 = c23 === m && t23[i32 + 1].startsWith("/>") ? " " : "";
    l22 += c23 === f ? s3 + n : d22 >= 0 ? (o25.push(a22), s3.slice(0, d22) + e + s3.slice(d22) + h + x2) : s3 + h + (d22 === -2 ? i32 : x2);
  }
  return [P(t23, l22 + (t23[s23] || "<?>") + (i24 === 2 ? "</svg>" : i24 === 3 ? "</math>" : "")), o25];
};
var N = class _N {
  constructor({ strings: t23, _$litType$: s23 }, n24) {
    let r24;
    this.parts = [];
    let c23 = 0, a22 = 0;
    const u22 = t23.length - 1, d22 = this.parts, [f22, v2] = V(t23, s23);
    if (this.el = _N.createElement(f22, n24), C.currentNode = this.el.content, s23 === 2 || s23 === 3) {
      const t32 = this.el.content.firstChild;
      t32.replaceWith(...t32.childNodes);
    }
    for (; (r24 = C.nextNode()) !== null && d22.length < u22; ) {
      if (r24.nodeType === 1) {
        if (r24.hasAttributes())
          for (const t32 of r24.getAttributeNames())
            if (t32.endsWith(e)) {
              const i24 = v2[a22++], s3 = r24.getAttribute(t32).split(h), e25 = /([.?@])?(.*)/.exec(i24);
              d22.push({ type: 1, index: c23, name: e25[2], strings: s3, ctor: e25[1] === "." ? H : e25[1] === "?" ? I : e25[1] === "@" ? L : k }), r24.removeAttribute(t32);
            } else
              t32.startsWith(h) && (d22.push({ type: 6, index: c23 }), r24.removeAttribute(t32));
        if ($.test(r24.tagName)) {
          const t32 = r24.textContent.split(h), s3 = t32.length - 1;
          if (s3 > 0) {
            r24.textContent = i ? i.emptyScript : "";
            for (let i24 = 0; i24 < s3; i24++)
              r24.append(t32[i24], l()), C.nextNode(), d22.push({ type: 2, index: ++c23 });
            r24.append(t32[s3], l());
          }
        }
      } else if (r24.nodeType === 8)
        if (r24.data === o)
          d22.push({ type: 2, index: c23 });
        else {
          let t32 = -1;
          for (; (t32 = r24.data.indexOf(h, t32 + 1)) !== -1; )
            d22.push({ type: 7, index: c23 }), t32 += h.length - 1;
        }
      c23++;
    }
  }
  static createElement(t23, i24) {
    const s23 = r.createElement("template");
    return s23.innerHTML = t23, s23;
  }
};
function S(t23, i24, s23 = t23, e25) {
  if (i24 === T)
    return i24;
  let h22 = e25 !== void 0 ? s23._$Co?.[e25] : s23._$Cl;
  const o25 = c(i24) ? void 0 : i24._$litDirective$;
  return h22?.constructor !== o25 && (h22?._$AO?.(false), o25 === void 0 ? h22 = void 0 : (h22 = new o25(t23), h22._$AT(t23, s23, e25)), e25 !== void 0 ? (s23._$Co ?? (s23._$Co = []))[e25] = h22 : s23._$Cl = h22), h22 !== void 0 && (i24 = S(t23, h22._$AS(t23, i24.values), h22, e25)), i24;
}
var M = class {
  constructor(t23, i24) {
    this._$AV = [], this._$AN = void 0, this._$AD = t23, this._$AM = i24;
  }
  get parentNode() {
    return this._$AM.parentNode;
  }
  get _$AU() {
    return this._$AM._$AU;
  }
  u(t23) {
    const { el: { content: i24 }, parts: s23 } = this._$AD, e25 = (t23?.creationScope ?? r).importNode(i24, true);
    C.currentNode = e25;
    let h22 = C.nextNode(), o25 = 0, n24 = 0, l22 = s23[0];
    for (; l22 !== void 0; ) {
      if (o25 === l22.index) {
        let i32;
        l22.type === 2 ? i32 = new R(h22, h22.nextSibling, this, t23) : l22.type === 1 ? i32 = new l22.ctor(h22, l22.name, l22.strings, this, t23) : l22.type === 6 && (i32 = new z(h22, this, t23)), this._$AV.push(i32), l22 = s23[++n24];
      }
      o25 !== l22?.index && (h22 = C.nextNode(), o25++);
    }
    return C.currentNode = r, e25;
  }
  p(t23) {
    let i24 = 0;
    for (const s23 of this._$AV)
      s23 !== void 0 && (s23.strings !== void 0 ? (s23._$AI(t23, s23, i24), i24 += s23.strings.length - 2) : s23._$AI(t23[i24])), i24++;
  }
};
var R = class _R {
  get _$AU() {
    return this._$AM?._$AU ?? this._$Cv;
  }
  constructor(t23, i24, s23, e25) {
    this.type = 2, this._$AH = E, this._$AN = void 0, this._$AA = t23, this._$AB = i24, this._$AM = s23, this.options = e25, this._$Cv = e25?.isConnected ?? true;
  }
  get parentNode() {
    let t23 = this._$AA.parentNode;
    const i24 = this._$AM;
    return i24 !== void 0 && t23?.nodeType === 11 && (t23 = i24.parentNode), t23;
  }
  get startNode() {
    return this._$AA;
  }
  get endNode() {
    return this._$AB;
  }
  _$AI(t23, i24 = this) {
    t23 = S(this, t23, i24), c(t23) ? t23 === E || t23 == null || t23 === "" ? (this._$AH !== E && this._$AR(), this._$AH = E) : t23 !== this._$AH && t23 !== T && this._(t23) : t23._$litType$ !== void 0 ? this.$(t23) : t23.nodeType !== void 0 ? this.T(t23) : u(t23) ? this.k(t23) : this._(t23);
  }
  O(t23) {
    return this._$AA.parentNode.insertBefore(t23, this._$AB);
  }
  T(t23) {
    this._$AH !== t23 && (this._$AR(), this._$AH = this.O(t23));
  }
  _(t23) {
    this._$AH !== E && c(this._$AH) ? this._$AA.nextSibling.data = t23 : this.T(r.createTextNode(t23)), this._$AH = t23;
  }
  $(t23) {
    const { values: i24, _$litType$: s23 } = t23, e25 = typeof s23 == "number" ? this._$AC(t23) : (s23.el === void 0 && (s23.el = N.createElement(P(s23.h, s23.h[0]), this.options)), s23);
    if (this._$AH?._$AD === e25)
      this._$AH.p(i24);
    else {
      const t32 = new M(e25, this), s3 = t32.u(this.options);
      t32.p(i24), this.T(s3), this._$AH = t32;
    }
  }
  _$AC(t23) {
    let i24 = A.get(t23.strings);
    return i24 === void 0 && A.set(t23.strings, i24 = new N(t23)), i24;
  }
  k(t23) {
    a(this._$AH) || (this._$AH = [], this._$AR());
    const i24 = this._$AH;
    let s23, e25 = 0;
    for (const h22 of t23)
      e25 === i24.length ? i24.push(s23 = new _R(this.O(l()), this.O(l()), this, this.options)) : s23 = i24[e25], s23._$AI(h22), e25++;
    e25 < i24.length && (this._$AR(s23 && s23._$AB.nextSibling, e25), i24.length = e25);
  }
  _$AR(t23 = this._$AA.nextSibling, i24) {
    for (this._$AP?.(false, true, i24); t23 && t23 !== this._$AB; ) {
      const i32 = t23.nextSibling;
      t23.remove(), t23 = i32;
    }
  }
  setConnected(t23) {
    this._$AM === void 0 && (this._$Cv = t23, this._$AP?.(t23));
  }
};
var k = class {
  get tagName() {
    return this.element.tagName;
  }
  get _$AU() {
    return this._$AM._$AU;
  }
  constructor(t23, i24, s23, e25, h22) {
    this.type = 1, this._$AH = E, this._$AN = void 0, this.element = t23, this.name = i24, this._$AM = e25, this.options = h22, s23.length > 2 || s23[0] !== "" || s23[1] !== "" ? (this._$AH = Array(s23.length - 1).fill(new String()), this.strings = s23) : this._$AH = E;
  }
  _$AI(t23, i24 = this, s23, e25) {
    const h22 = this.strings;
    let o25 = false;
    if (h22 === void 0)
      t23 = S(this, t23, i24, 0), o25 = !c(t23) || t23 !== this._$AH && t23 !== T, o25 && (this._$AH = t23);
    else {
      const e33 = t23;
      let n24, r24;
      for (t23 = h22[0], n24 = 0; n24 < h22.length - 1; n24++)
        r24 = S(this, e33[s23 + n24], i24, n24), r24 === T && (r24 = this._$AH[n24]), o25 || (o25 = !c(r24) || r24 !== this._$AH[n24]), r24 === E ? t23 = E : t23 !== E && (t23 += (r24 ?? "") + h22[n24 + 1]), this._$AH[n24] = r24;
    }
    o25 && !e25 && this.j(t23);
  }
  j(t23) {
    t23 === E ? this.element.removeAttribute(this.name) : this.element.setAttribute(this.name, t23 ?? "");
  }
};
var H = class extends k {
  constructor() {
    super(...arguments), this.type = 3;
  }
  j(t23) {
    this.element[this.name] = t23 === E ? void 0 : t23;
  }
};
var I = class extends k {
  constructor() {
    super(...arguments), this.type = 4;
  }
  j(t23) {
    this.element.toggleAttribute(this.name, !!t23 && t23 !== E);
  }
};
var L = class extends k {
  constructor(t23, i24, s23, e25, h22) {
    super(t23, i24, s23, e25, h22), this.type = 5;
  }
  _$AI(t23, i24 = this) {
    if ((t23 = S(this, t23, i24, 0) ?? E) === T)
      return;
    const s23 = this._$AH, e25 = t23 === E && s23 !== E || t23.capture !== s23.capture || t23.once !== s23.once || t23.passive !== s23.passive, h22 = t23 !== E && (s23 === E || e25);
    e25 && this.element.removeEventListener(this.name, this, s23), h22 && this.element.addEventListener(this.name, this, t23), this._$AH = t23;
  }
  handleEvent(t23) {
    typeof this._$AH == "function" ? this._$AH.call(this.options?.host ?? this.element, t23) : this._$AH.handleEvent(t23);
  }
};
var z = class {
  constructor(t23, i24, s23) {
    this.element = t23, this.type = 6, this._$AN = void 0, this._$AM = i24, this.options = s23;
  }
  get _$AU() {
    return this._$AM._$AU;
  }
  _$AI(t23) {
    S(this, t23);
  }
};
var j = t.litHtmlPolyfillSupport;
j?.(N, R), (t.litHtmlVersions ?? (t.litHtmlVersions = [])).push("3.3.0");
var B = (t23, i24, s23) => {
  const e25 = s23?.renderBefore ?? i24;
  let h22 = e25._$litPart$;
  if (h22 === void 0) {
    const t32 = s23?.renderBefore ?? null;
    e25._$litPart$ = h22 = new R(i24.insertBefore(l(), t32), t32, void 0, s23 ?? {});
  }
  return h22._$AI(t23), h22;
};

// chunks/chunk.7OBLIRXR.js
var t2 = globalThis;
var e2 = t2.ShadowRoot && (t2.ShadyCSS === void 0 || t2.ShadyCSS.nativeShadow) && "adoptedStyleSheets" in Document.prototype && "replace" in CSSStyleSheet.prototype;
var s2 = Symbol();
var o2 = /* @__PURE__ */ new WeakMap();
var n2 = class {
  constructor(t23, e33, o52) {
    if (this._$cssResult$ = true, o52 !== s2)
      throw Error("CSSResult is not constructable. Use `unsafeCSS` or `css` instead.");
    this.cssText = t23, this.t = e33;
  }
  get styleSheet() {
    let t23 = this.o;
    const s3 = this.t;
    if (e2 && t23 === void 0) {
      const e33 = s3 !== void 0 && s3.length === 1;
      e33 && (t23 = o2.get(s3)), t23 === void 0 && ((this.o = t23 = new CSSStyleSheet()).replaceSync(this.cssText), e33 && o2.set(s3, t23));
    }
    return t23;
  }
  toString() {
    return this.cssText;
  }
};
var r2 = (t23) => new n2(typeof t23 == "string" ? t23 : t23 + "", void 0, s2);
var i2 = (t23, ...e33) => {
  const o52 = t23.length === 1 ? t23[0] : e33.reduce((e42, s3, o62) => e42 + ((t32) => {
    if (t32._$cssResult$ === true)
      return t32.cssText;
    if (typeof t32 == "number")
      return t32;
    throw Error("Value passed to 'css' function must be a 'css' function result: " + t32 + ". Use 'unsafeCSS' to pass non-literal values, but take care to ensure page security.");
  })(s3) + t23[o62 + 1], t23[0]);
  return new n2(o52, t23, s2);
};
var S2 = (s3, o52) => {
  if (e2)
    s3.adoptedStyleSheets = o52.map((t23) => t23 instanceof CSSStyleSheet ? t23 : t23.styleSheet);
  else
    for (const e33 of o52) {
      const o62 = document.createElement("style"), n32 = t2.litNonce;
      n32 !== void 0 && o62.setAttribute("nonce", n32), o62.textContent = e33.cssText, s3.appendChild(o62);
    }
};
var c2 = e2 ? (t23) => t23 : (t23) => t23 instanceof CSSStyleSheet ? ((t32) => {
  let e33 = "";
  for (const s3 of t32.cssRules)
    e33 += s3.cssText;
  return r2(e33);
})(t23) : t23;
var { is: i22, defineProperty: e22, getOwnPropertyDescriptor: h2, getOwnPropertyNames: r22, getOwnPropertySymbols: o22, getPrototypeOf: n22 } = Object;
var a2 = globalThis;
var c22 = a2.trustedTypes;
var l2 = c22 ? c22.emptyScript : "";
var p2 = a2.reactiveElementPolyfillSupport;
var d2 = (t23, s3) => t23;
var u2 = { toAttribute(t23, s3) {
  switch (s3) {
    case Boolean:
      t23 = t23 ? l2 : null;
      break;
    case Object:
    case Array:
      t23 = t23 == null ? t23 : JSON.stringify(t23);
  }
  return t23;
}, fromAttribute(t23, s3) {
  let i42 = t23;
  switch (s3) {
    case Boolean:
      i42 = t23 !== null;
      break;
    case Number:
      i42 = t23 === null ? null : Number(t23);
      break;
    case Object:
    case Array:
      try {
        i42 = JSON.parse(t23);
      } catch (t32) {
        i42 = null;
      }
  }
  return i42;
} };
var f2 = (t23, s3) => !i22(t23, s3);
var b2 = { attribute: true, type: String, converter: u2, reflect: false, useDefault: false, hasChanged: f2 };
Symbol.metadata ?? (Symbol.metadata = Symbol("metadata")), a2.litPropertyMetadata ?? (a2.litPropertyMetadata = /* @__PURE__ */ new WeakMap());
var y2 = class extends HTMLElement {
  static addInitializer(t23) {
    this._$Ei(), (this.l ?? (this.l = [])).push(t23);
  }
  static get observedAttributes() {
    return this.finalize(), this._$Eh && [...this._$Eh.keys()];
  }
  static createProperty(t23, s3 = b2) {
    if (s3.state && (s3.attribute = false), this._$Ei(), this.prototype.hasOwnProperty(t23) && ((s3 = Object.create(s3)).wrapped = true), this.elementProperties.set(t23, s3), !s3.noAccessor) {
      const i42 = Symbol(), h22 = this.getPropertyDescriptor(t23, i42, s3);
      h22 !== void 0 && e22(this.prototype, t23, h22);
    }
  }
  static getPropertyDescriptor(t23, s3, i42) {
    const { get: e33, set: r32 } = h2(this.prototype, t23) ?? { get() {
      return this[s3];
    }, set(t32) {
      this[s3] = t32;
    } };
    return { get: e33, set(s4) {
      const h22 = e33?.call(this);
      r32?.call(this, s4), this.requestUpdate(t23, h22, i42);
    }, configurable: true, enumerable: true };
  }
  static getPropertyOptions(t23) {
    return this.elementProperties.get(t23) ?? b2;
  }
  static _$Ei() {
    if (this.hasOwnProperty(d2("elementProperties")))
      return;
    const t23 = n22(this);
    t23.finalize(), t23.l !== void 0 && (this.l = [...t23.l]), this.elementProperties = new Map(t23.elementProperties);
  }
  static finalize() {
    if (this.hasOwnProperty(d2("finalized")))
      return;
    if (this.finalized = true, this._$Ei(), this.hasOwnProperty(d2("properties"))) {
      const t32 = this.properties, s3 = [...r22(t32), ...o22(t32)];
      for (const i42 of s3)
        this.createProperty(i42, t32[i42]);
    }
    const t23 = this[Symbol.metadata];
    if (t23 !== null) {
      const s3 = litPropertyMetadata.get(t23);
      if (s3 !== void 0)
        for (const [t32, i42] of s3)
          this.elementProperties.set(t32, i42);
    }
    this._$Eh = /* @__PURE__ */ new Map();
    for (const [t32, s3] of this.elementProperties) {
      const i42 = this._$Eu(t32, s3);
      i42 !== void 0 && this._$Eh.set(i42, t32);
    }
    this.elementStyles = this.finalizeStyles(this.styles);
  }
  static finalizeStyles(s3) {
    const i42 = [];
    if (Array.isArray(s3)) {
      const e33 = new Set(s3.flat(1 / 0).reverse());
      for (const s4 of e33)
        i42.unshift(c2(s4));
    } else
      s3 !== void 0 && i42.push(c2(s3));
    return i42;
  }
  static _$Eu(t23, s3) {
    const i42 = s3.attribute;
    return i42 === false ? void 0 : typeof i42 == "string" ? i42 : typeof t23 == "string" ? t23.toLowerCase() : void 0;
  }
  constructor() {
    super(), this._$Ep = void 0, this.isUpdatePending = false, this.hasUpdated = false, this._$Em = null, this._$Ev();
  }
  _$Ev() {
    this._$ES = new Promise((t23) => this.enableUpdating = t23), this._$AL = /* @__PURE__ */ new Map(), this._$E_(), this.requestUpdate(), this.constructor.l?.forEach((t23) => t23(this));
  }
  addController(t23) {
    (this._$EO ?? (this._$EO = /* @__PURE__ */ new Set())).add(t23), this.renderRoot !== void 0 && this.isConnected && t23.hostConnected?.();
  }
  removeController(t23) {
    this._$EO?.delete(t23);
  }
  _$E_() {
    const t23 = /* @__PURE__ */ new Map(), s3 = this.constructor.elementProperties;
    for (const i42 of s3.keys())
      this.hasOwnProperty(i42) && (t23.set(i42, this[i42]), delete this[i42]);
    t23.size > 0 && (this._$Ep = t23);
  }
  createRenderRoot() {
    const t23 = this.shadowRoot ?? this.attachShadow(this.constructor.shadowRootOptions);
    return S2(t23, this.constructor.elementStyles), t23;
  }
  connectedCallback() {
    this.renderRoot ?? (this.renderRoot = this.createRenderRoot()), this.enableUpdating(true), this._$EO?.forEach((t23) => t23.hostConnected?.());
  }
  enableUpdating(t23) {
  }
  disconnectedCallback() {
    this._$EO?.forEach((t23) => t23.hostDisconnected?.());
  }
  attributeChangedCallback(t23, s3, i42) {
    this._$AK(t23, i42);
  }
  _$ET(t23, s3) {
    const i42 = this.constructor.elementProperties.get(t23), e33 = this.constructor._$Eu(t23, i42);
    if (e33 !== void 0 && i42.reflect === true) {
      const h22 = (i42.converter?.toAttribute !== void 0 ? i42.converter : u2).toAttribute(s3, i42.type);
      this._$Em = t23, h22 == null ? this.removeAttribute(e33) : this.setAttribute(e33, h22), this._$Em = null;
    }
  }
  _$AK(t23, s3) {
    const i42 = this.constructor, e33 = i42._$Eh.get(t23);
    if (e33 !== void 0 && this._$Em !== e33) {
      const t32 = i42.getPropertyOptions(e33), h22 = typeof t32.converter == "function" ? { fromAttribute: t32.converter } : t32.converter?.fromAttribute !== void 0 ? t32.converter : u2;
      this._$Em = e33, this[e33] = h22.fromAttribute(s3, t32.type) ?? this._$Ej?.get(e33) ?? null, this._$Em = null;
    }
  }
  requestUpdate(t23, s3, i42) {
    if (t23 !== void 0) {
      const e33 = this.constructor, h22 = this[t23];
      if (i42 ?? (i42 = e33.getPropertyOptions(t23)), !((i42.hasChanged ?? f2)(h22, s3) || i42.useDefault && i42.reflect && h22 === this._$Ej?.get(t23) && !this.hasAttribute(e33._$Eu(t23, i42))))
        return;
      this.C(t23, s3, i42);
    }
    this.isUpdatePending === false && (this._$ES = this._$EP());
  }
  C(t23, s3, { useDefault: i42, reflect: e33, wrapped: h22 }, r32) {
    i42 && !(this._$Ej ?? (this._$Ej = /* @__PURE__ */ new Map())).has(t23) && (this._$Ej.set(t23, r32 ?? s3 ?? this[t23]), h22 !== true || r32 !== void 0) || (this._$AL.has(t23) || (this.hasUpdated || i42 || (s3 = void 0), this._$AL.set(t23, s3)), e33 === true && this._$Em !== t23 && (this._$Eq ?? (this._$Eq = /* @__PURE__ */ new Set())).add(t23));
  }
  async _$EP() {
    this.isUpdatePending = true;
    try {
      await this._$ES;
    } catch (t32) {
      Promise.reject(t32);
    }
    const t23 = this.scheduleUpdate();
    return t23 != null && await t23, !this.isUpdatePending;
  }
  scheduleUpdate() {
    return this.performUpdate();
  }
  performUpdate() {
    if (!this.isUpdatePending)
      return;
    if (!this.hasUpdated) {
      if (this.renderRoot ?? (this.renderRoot = this.createRenderRoot()), this._$Ep) {
        for (const [t42, s4] of this._$Ep)
          this[t42] = s4;
        this._$Ep = void 0;
      }
      const t32 = this.constructor.elementProperties;
      if (t32.size > 0)
        for (const [s4, i42] of t32) {
          const { wrapped: t42 } = i42, e33 = this[s4];
          t42 !== true || this._$AL.has(s4) || e33 === void 0 || this.C(s4, void 0, i42, e33);
        }
    }
    let t23 = false;
    const s3 = this._$AL;
    try {
      t23 = this.shouldUpdate(s3), t23 ? (this.willUpdate(s3), this._$EO?.forEach((t32) => t32.hostUpdate?.()), this.update(s3)) : this._$EM();
    } catch (s4) {
      throw t23 = false, this._$EM(), s4;
    }
    t23 && this._$AE(s3);
  }
  willUpdate(t23) {
  }
  _$AE(t23) {
    this._$EO?.forEach((t32) => t32.hostUpdated?.()), this.hasUpdated || (this.hasUpdated = true, this.firstUpdated(t23)), this.updated(t23);
  }
  _$EM() {
    this._$AL = /* @__PURE__ */ new Map(), this.isUpdatePending = false;
  }
  get updateComplete() {
    return this.getUpdateComplete();
  }
  getUpdateComplete() {
    return this._$ES;
  }
  shouldUpdate(t23) {
    return true;
  }
  update(t23) {
    this._$Eq && (this._$Eq = this._$Eq.forEach((t32) => this._$ET(t32, this[t32]))), this._$EM();
  }
  updated(t23) {
  }
  firstUpdated(t23) {
  }
};
y2.elementStyles = [], y2.shadowRootOptions = { mode: "open" }, y2[d2("elementProperties")] = /* @__PURE__ */ new Map(), y2[d2("finalized")] = /* @__PURE__ */ new Map(), p2?.({ ReactiveElement: y2 }), (a2.reactiveElementVersions ?? (a2.reactiveElementVersions = [])).push("2.1.0");
var o3 = false;
var s22 = globalThis;
var i3 = class extends y2 {
  constructor() {
    super(...arguments), this.renderOptions = { host: this }, this._$Do = void 0;
  }
  createRenderRoot() {
    var _a;
    const t23 = super.createRenderRoot();
    return (_a = this.renderOptions).renderBefore ?? (_a.renderBefore = t23.firstChild), t23;
  }
  update(t23) {
    const r32 = this.render();
    this.hasUpdated || (this.renderOptions.isConnected = this.isConnected), super.update(t23), this._$Do = B(r32, this.renderRoot, this.renderOptions);
  }
  connectedCallback() {
    super.connectedCallback(), this._$Do?.setConnected(true);
  }
  disconnectedCallback() {
    super.disconnectedCallback(), this._$Do?.setConnected(false);
  }
  render() {
    return T;
  }
};
i3._$litElement$ = true, i3["finalized"] = true, s22.litElementHydrateSupport?.({ LitElement: i3 });
var o4 = s22.litElementPolyfillSupport;
o4?.({ LitElement: i3 });
(s22.litElementVersions ?? (s22.litElementVersions = [])).push("4.2.0");

// chunks/chunk.JHZRD2LV.js
var __defProp = Object.defineProperty;
var __getOwnPropDesc = Object.getOwnPropertyDescriptor;
var __typeError = (msg) => {
  throw TypeError(msg);
};
var __decorateClass = (decorators, target, key, kind) => {
  var result = kind > 1 ? void 0 : kind ? __getOwnPropDesc(target, key) : target;
  for (var i7 = decorators.length - 1, decorator; i7 >= 0; i7--)
    if (decorator = decorators[i7])
      result = (kind ? decorator(target, key, result) : decorator(result)) || result;
  if (kind && result)
    __defProp(target, key, result);
  return result;
};
var __accessCheck = (obj, member, msg) => member.has(obj) || __typeError("Cannot " + msg);
var __privateGet = (obj, member, getter) => (__accessCheck(obj, member, "read from private field"), getter ? getter.call(obj) : member.get(obj));
var __privateAdd = (obj, member, value) => member.has(obj) ? __typeError("Cannot add the same private member more than once") : member instanceof WeakSet ? member.add(obj) : member.set(obj, value);
var __privateSet = (obj, member, value, setter) => (__accessCheck(obj, member, "write to private field"), setter ? setter.call(obj, value) : member.set(obj, value), value);

// chunks/chunk.K4C5PQDP.js
var t3 = (t32) => (e33, o33) => {
  o33 !== void 0 ? o33.addInitializer(() => {
    customElements.define(t32, e33);
  }) : customElements.define(t32, e33);
};
var o23 = { attribute: true, type: String, converter: u2, reflect: false, hasChanged: f2 };
var r3 = (t32 = o23, e33, r42) => {
  const { kind: n24, metadata: i32 } = r42;
  let s3 = globalThis.litPropertyMetadata.get(i32);
  if (s3 === void 0 && globalThis.litPropertyMetadata.set(i32, s3 = /* @__PURE__ */ new Map()), n24 === "setter" && ((t32 = Object.create(t32)).wrapped = true), s3.set(r42.name, t32), n24 === "accessor") {
    const { name: o33 } = r42;
    return { set(r5) {
      const n32 = e33.get.call(this);
      e33.set.call(this, r5), this.requestUpdate(o33, n32, t32);
    }, init(e42) {
      return e42 !== void 0 && this.C(o33, void 0, t32, e42), e42;
    } };
  }
  if (n24 === "setter") {
    const { name: o33 } = r42;
    return function(r5) {
      const n32 = this[o33];
      e33.call(this, r5), this.requestUpdate(o33, n32, t32);
    };
  }
  throw Error("Unsupported decorator location: " + n24);
};
function n3(t32) {
  return (e33, o33) => typeof o33 == "object" ? r3(t32, e33, o33) : ((t42, e42, o43) => {
    const r42 = e42.hasOwnProperty(o43);
    return e42.constructor.createProperty(o43, t42), r42 ? Object.getOwnPropertyDescriptor(e42, o43) : void 0;
  })(t32, e33, o33);
}
function r23(r42) {
  return n3({ ...r42, state: true, attribute: false });
}
function t22(t32) {
  return (n24, o33) => {
    const c4 = typeof n24 == "function" ? n24 : n24[o33];
    Object.assign(c4, t32);
  };
}
var e3 = (e33, t32, c4) => (c4.configurable = true, c4.enumerable = true, Reflect.decorate && typeof t32 != "object" && Object.defineProperty(e33, t32, c4), c4);
function e23(e33, r42) {
  return (n24, s3, i32) => {
    const o33 = (t32) => t32.renderRoot?.querySelector(e33) ?? null;
    if (r42) {
      const { get: e42, set: r5 } = typeof s3 == "object" ? n24 : i32 ?? (() => {
        const t32 = Symbol();
        return { get() {
          return this[t32];
        }, set(e52) {
          this[t32] = e52;
        } };
      })();
      return e3(n24, s3, { get() {
        let t32 = e42.call(this);
        return t32 === void 0 && (t32 = o33(this), (t32 !== null || this.hasUpdated) && r5.call(this, t32)), t32;
      } });
    }
    return e3(n24, s3, { get() {
      return o33(this);
    } });
  };
}
var host_styles_default = i2`
  :host {
    box-sizing: border-box;
  }

  :host *,
  :host *::before,
  :host *::after {
    box-sizing: inherit;
  }

  [hidden] {
    display: none !important;
  }
`;
var _hasRecordedInitialProperties;
var WebAwesomeElement = class extends i3 {
  constructor() {
    super();
    __privateAdd(this, _hasRecordedInitialProperties, false);
    this.initialReflectedProperties = /* @__PURE__ */ new Map();
    this.didSSR = o3 || Boolean(this.shadowRoot);
    this.customStates = {
      set: (customState, active) => {
        if (!Boolean(this.internals?.states))
          return;
        try {
          if (active) {
            this.internals.states.add(customState);
          } else {
            this.internals.states.delete(customState);
          }
        } catch (e33) {
          if (String(e33).includes("must start with '--'")) {
            console.error("Your browser implements an outdated version of CustomStateSet. Consider using a polyfill");
          } else {
            throw e33;
          }
        }
      },
      has: (customState) => {
        if (!Boolean(this.internals?.states))
          return false;
        try {
          return this.internals.states.has(customState);
        } catch {
          return false;
        }
      }
    };
    try {
      this.internals = this.attachInternals();
    } catch {
      console.error("Element internals are not supported in your browser. Consider using a polyfill");
    }
    this.customStates.set("wa-defined", true);
    let Self = this.constructor;
    for (let [property, spec] of Self.elementProperties) {
      if (spec.default === "inherit" && spec.initial !== void 0 && typeof property === "string") {
        this.customStates.set(`initial-${property}-${spec.initial}`, true);
      }
    }
  }
  static get styles() {
    const styles = Array.isArray(this.css) ? this.css : this.css ? [this.css] : [];
    return [host_styles_default, ...styles];
  }
  connectedCallback() {
    super.connectedCallback();
    if (!o3) {
      this.shadowRoot?.prepend(document.createComment(` Web Awesome: https://webawesome.com/docs/components/${this.localName.replace("wa-", "")} `));
    }
  }
  attributeChangedCallback(name, oldValue, newValue) {
    if (!__privateGet(this, _hasRecordedInitialProperties)) {
      this.constructor.elementProperties.forEach((obj, prop) => {
        if (obj.reflect && this[prop] != null) {
          this.initialReflectedProperties.set(prop, this[prop]);
        }
      });
      __privateSet(this, _hasRecordedInitialProperties, true);
    }
    super.attributeChangedCallback(name, oldValue, newValue);
  }
  willUpdate(changedProperties) {
    super.willUpdate(changedProperties);
    this.initialReflectedProperties.forEach((value, prop) => {
      if (changedProperties.has(prop) && this[prop] == null) {
        this[prop] = value;
      }
    });
  }
  firstUpdated(changedProperties) {
    super.firstUpdated(changedProperties);
    if (this.didSSR) {
      this.shadowRoot?.querySelectorAll("slot").forEach((slotElement) => {
        slotElement.dispatchEvent(new Event("slotchange", { bubbles: true, composed: false, cancelable: false }));
      });
    }
  }
  update(changedProperties) {
    try {
      super.update(changedProperties);
    } catch (e33) {
      if (this.didSSR && !this.hasUpdated) {
        const event = new Event("lit-hydration-error", { bubbles: true, composed: true, cancelable: false });
        event.error = e33;
        this.dispatchEvent(event);
      }
      throw e33;
    }
  }
  relayNativeEvent(event, eventOptions) {
    event.stopImmediatePropagation();
    this.dispatchEvent(new event.constructor(event.type, {
      ...event,
      ...eventOptions
    }));
  }
};
_hasRecordedInitialProperties = new WeakMap();
__decorateClass([
  n3()
], WebAwesomeElement.prototype, "dir", 2);
__decorateClass([
  n3()
], WebAwesomeElement.prototype, "lang", 2);
__decorateClass([
  n3({ type: Boolean, reflect: true, attribute: "did-ssr" })
], WebAwesomeElement.prototype, "didSSR", 2);

// chunks/chunk.I3XGXHPO.js
var CustomErrorValidator = () => {
  return {
    observedAttributes: ["custom-error"],
    checkValidity(element) {
      const validity = {
        message: "",
        isValid: true,
        invalidKeys: []
      };
      if (element.customError) {
        validity.message = element.customError;
        validity.isValid = false;
        validity.invalidKeys = ["customError"];
      }
      return validity;
    }
  };
};
var WebAwesomeFormAssociatedElement = class extends WebAwesomeElement {
  constructor() {
    super();
    this.name = null;
    this.disabled = false;
    this.required = false;
    this.assumeInteractionOn = ["input"];
    this.validators = [];
    this.valueHasChanged = false;
    this.hasInteracted = false;
    this.customError = null;
    this.emittedEvents = [];
    this.emitInvalid = (e6) => {
      if (e6.target !== this)
        return;
      this.hasInteracted = true;
      this.dispatchEvent(new WaInvalidEvent());
    };
    this.handleInteraction = (event) => {
      const emittedEvents = this.emittedEvents;
      if (!emittedEvents.includes(event.type)) {
        emittedEvents.push(event.type);
      }
      if (emittedEvents.length === this.assumeInteractionOn?.length) {
        this.hasInteracted = true;
      }
    };
    if (!o3) {
      this.addEventListener("invalid", this.emitInvalid);
    }
  }
  static get validators() {
    return [CustomErrorValidator()];
  }
  static get observedAttributes() {
    const parentAttrs = new Set(super.observedAttributes || []);
    for (const validator of this.validators) {
      if (!validator.observedAttributes) {
        continue;
      }
      for (const attr of validator.observedAttributes) {
        parentAttrs.add(attr);
      }
    }
    return [...parentAttrs];
  }
  connectedCallback() {
    super.connectedCallback();
    this.updateValidity();
    this.assumeInteractionOn.forEach((event) => {
      this.addEventListener(event, this.handleInteraction);
    });
  }
  firstUpdated(...args) {
    super.firstUpdated(...args);
    this.updateValidity();
  }
  willUpdate(changedProperties) {
    if (!o3 && changedProperties.has("customError")) {
      if (!this.customError) {
        this.customError = null;
      }
      this.setCustomValidity(this.customError || "");
    }
    if (changedProperties.has("value") || changedProperties.has("disabled") || changedProperties.has("defaultValue")) {
      const value = this.value;
      if (Array.isArray(value)) {
        if (this.name) {
          const formData = new FormData();
          for (const val of value) {
            formData.append(this.name, val);
          }
          this.setValue(formData, formData);
        }
      } else {
        this.setValue(value, value);
      }
    }
    if (changedProperties.has("disabled")) {
      this.customStates.set("disabled", this.disabled);
      if (this.hasAttribute("disabled") || !o3 && !this.matches(":disabled")) {
        this.toggleAttribute("disabled", this.disabled);
      }
    }
    super.willUpdate(changedProperties);
    this.updateValidity();
  }
  get labels() {
    return this.internals.labels;
  }
  getForm() {
    return this.internals.form;
  }
  set form(val) {
    if (val) {
      this.setAttribute("form", val);
    } else {
      this.removeAttribute("form");
    }
  }
  get form() {
    return this.internals.form;
  }
  get validity() {
    return this.internals.validity;
  }
  get willValidate() {
    return this.internals.willValidate;
  }
  get validationMessage() {
    return this.internals.validationMessage;
  }
  checkValidity() {
    this.updateValidity();
    return this.internals.checkValidity();
  }
  reportValidity() {
    this.updateValidity();
    this.hasInteracted = true;
    return this.internals.reportValidity();
  }
  get validationTarget() {
    return this.input || void 0;
  }
  setValidity(...args) {
    const flags = args[0];
    const message = args[1];
    let anchor = args[2];
    if (!anchor) {
      anchor = this.validationTarget;
    }
    this.internals.setValidity(flags, message, anchor || void 0);
    this.requestUpdate("validity");
    this.setCustomStates();
  }
  setCustomStates() {
    const required = Boolean(this.required);
    const isValid = this.internals.validity.valid;
    const hasInteracted = this.hasInteracted;
    this.customStates.set("required", required);
    this.customStates.set("optional", !required);
    this.customStates.set("invalid", !isValid);
    this.customStates.set("valid", isValid);
    this.customStates.set("user-invalid", !isValid && hasInteracted);
    this.customStates.set("user-valid", isValid && hasInteracted);
  }
  setCustomValidity(message) {
    if (!message) {
      this.customError = null;
      this.setValidity({});
      return;
    }
    this.customError = message;
    this.setValidity({ customError: true }, message, this.validationTarget);
  }
  formResetCallback() {
    this.resetValidity();
    this.hasInteracted = false;
    this.valueHasChanged = false;
    this.emittedEvents = [];
    this.updateValidity();
  }
  formDisabledCallback(isDisabled) {
    this.disabled = isDisabled;
    this.updateValidity();
  }
  formStateRestoreCallback(state, reason) {
    this.value = state;
    if (reason === "restore") {
      this.resetValidity();
    }
    this.updateValidity();
  }
  setValue(...args) {
    const [value, state] = args;
    this.internals.setFormValue(value, state);
  }
  get allValidators() {
    const staticValidators = this.constructor.validators || [];
    const validators = this.validators || [];
    return [...staticValidators, ...validators];
  }
  resetValidity() {
    this.setCustomValidity("");
    this.setValidity({});
  }
  updateValidity() {
    if (this.disabled || this.hasAttribute("disabled") || !this.willValidate) {
      this.resetValidity();
      return;
    }
    const validators = this.allValidators;
    if (!validators?.length) {
      return;
    }
    const flags = {
      customError: Boolean(this.customError)
    };
    const formControl = this.validationTarget || this.input || void 0;
    let finalMessage = "";
    for (const validator of validators) {
      const { isValid, message, invalidKeys } = validator.checkValidity(this);
      if (isValid) {
        continue;
      }
      if (!finalMessage) {
        finalMessage = message;
      }
      if (invalidKeys?.length >= 0) {
        invalidKeys.forEach((str) => flags[str] = true);
      }
    }
    if (!finalMessage) {
      finalMessage = this.validationMessage;
    }
    this.setValidity(flags, finalMessage, formControl);
  }
};
WebAwesomeFormAssociatedElement.formAssociated = true;
__decorateClass([
  n3({ reflect: true })
], WebAwesomeFormAssociatedElement.prototype, "name", 2);
__decorateClass([
  n3({ type: Boolean })
], WebAwesomeFormAssociatedElement.prototype, "disabled", 2);
__decorateClass([
  n3({ state: true, attribute: false })
], WebAwesomeFormAssociatedElement.prototype, "valueHasChanged", 2);
__decorateClass([
  n3({ state: true, attribute: false })
], WebAwesomeFormAssociatedElement.prototype, "hasInteracted", 2);
__decorateClass([
  n3({ attribute: "custom-error", reflect: true })
], WebAwesomeFormAssociatedElement.prototype, "customError", 2);
__decorateClass([
  n3({ attribute: false, state: true, type: Object })
], WebAwesomeFormAssociatedElement.prototype, "validity", 1);

// chunks/chunk.H23DVATU.js
var t4 = { ATTRIBUTE: 1, CHILD: 2, PROPERTY: 3, BOOLEAN_ATTRIBUTE: 4, EVENT: 5, ELEMENT: 6 };
var e4 = (t23) => (...e25) => ({ _$litDirective$: t23, values: e25 });
var i4 = class {
  constructor(t23) {
  }
  get _$AU() {
    return this._$AM._$AU;
  }
  _$AT(t23, e25, i24) {
    this._$Ct = t23, this._$AM = e25, this._$Ci = i24;
  }
  _$AS(t23, e25) {
    return this.update(t23, e25);
  }
  update(t23, e25) {
    return this.render(...e25);
  }
};

// chunks/chunk.KWDPKKFO.js
var e24 = e4(class extends i4 {
  constructor(t23) {
    if (super(t23), t23.type !== t4.ATTRIBUTE || t23.name !== "class" || t23.strings?.length > 2)
      throw Error("`classMap()` can only be used in the `class` attribute and must be the only part in the attribute.");
  }
  render(t23) {
    return " " + Object.keys(t23).filter((s3) => t23[s3]).join(" ") + " ";
  }
  update(s3, [i24]) {
    if (this.st === void 0) {
      this.st = /* @__PURE__ */ new Set(), s3.strings !== void 0 && (this.nt = new Set(s3.strings.join(" ").split(/\s/).filter((t23) => t23 !== "")));
      for (const t23 in i24)
        i24[t23] && !this.nt?.has(t23) && this.st.add(t23);
      return this.render(i24);
    }
    const r5 = s3.element.classList;
    for (const t23 of this.st)
      t23 in i24 || (r5.remove(t23), this.st.delete(t23));
    for (const t23 in i24) {
      const s23 = !!i24[t23];
      s23 === this.st.has(t23) || this.nt?.has(t23) || (s23 ? (r5.add(t23), this.st.add(t23)) : (r5.remove(t23), this.st.delete(t23)));
    }
    return T;
  }
});

// chunks/chunk.KIHB3VMB.js
var HasSlotController = class {
  constructor(host, ...slotNames) {
    this.slotNames = [];
    this.handleSlotChange = (event) => {
      const slot = event.target;
      if (this.slotNames.includes("[default]") && !slot.name || slot.name && this.slotNames.includes(slot.name)) {
        this.host.requestUpdate();
      }
    };
    (this.host = host).addController(this);
    this.slotNames = slotNames;
  }
  hasDefaultSlot() {
    if (!this.host.childNodes) {
      return false;
    }
    return [...this.host.childNodes].some((node) => {
      if (node.nodeType === Node.TEXT_NODE && node.textContent.trim() !== "") {
        return true;
      }
      if (node.nodeType === Node.ELEMENT_NODE) {
        const el = node;
        const tagName = el.tagName.toLowerCase();
        if (tagName === "wa-visually-hidden") {
          return false;
        }
        if (!el.hasAttribute("slot")) {
          return true;
        }
      }
      return false;
    });
  }
  hasNamedSlot(name) {
    return this.host.querySelector?.(`:scope > [slot="${name}"]`) !== null;
  }
  test(slotName) {
    return slotName === "[default]" ? this.hasDefaultSlot() : this.hasNamedSlot(slotName);
  }
  hostConnected() {
    this.host.shadowRoot?.addEventListener?.("slotchange", this.handleSlotChange);
  }
  hostDisconnected() {
    this.host.shadowRoot?.removeEventListener?.("slotchange", this.handleSlotChange);
  }
};

// chunks/chunk.RTFTF4YY.js
var button_styles_default = i2`
  @layer wa-component {
    :host {
      display: inline-block;

      /* Workaround because Chrome doesn't like :host(:has()) below
       * https://issues.chromium.org/issues/40062355
       * Firefox doesn't like this nested rule, so both are needed */
      &:has(wa-badge) {
        position: relative;
      }
    }

    /* Apply relative positioning only when needed to position wa-badge
     * This avoids creating a new stacking context for every button */
    :host(:has(wa-badge)) {
      position: relative;
    }
  }

  .button {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    text-decoration: none;
    user-select: none;
    -webkit-user-select: none;
    white-space: nowrap;
    vertical-align: middle;
    transition-property: background, border, box-shadow, color, opacity;
    transition-duration: var(--wa-transition-fast);
    transition-timing-function: var(--wa-transition-easing);
    cursor: pointer;
    padding: 0 var(--wa-form-control-padding-inline);
    font-family: inherit;
    font-size: inherit;
    font-weight: var(--wa-font-weight-action);
    line-height: calc(var(--wa-form-control-height) - var(--wa-form-control-border-width) * 2);
    height: var(--wa-form-control-height);
    width: 100%;

    background-color: var(--wa-color-fill-loud, var(--wa-color-neutral-fill-loud));
    border-color: transparent;
    color: var(--wa-color-on-loud, var(--wa-color-neutral-on-loud));
    border-start-start-radius: var(--_button-start-start-radius, var(--wa-form-control-border-radius));
    border-start-end-radius: var(--_button-start-end-radius, var(--wa-form-control-border-radius));
    border-end-start-radius: var(--_button-end-start-radius, var(--wa-form-control-border-radius));
    border-end-end-radius: var(--_button-end-end-radius, var(--wa-form-control-border-radius));
    border-style: var(--wa-form-control-border-style);
    border-width: var(--wa-form-control-border-width);
  }

  /* Appearance modifiers */
  :host([appearance='plain']) {
    /* Indentation overrides for grouping */
    margin-inline-start: var(--_button-horizontal-indent);
    margin-block-start: var(--_button-vertical-indent);

    .button {
      color: var(--wa-color-on-quiet, var(--wa-color-neutral-on-quiet));
      background-color: transparent;
      border-color: transparent;
    }
    @media (hover: hover) {
      .button:not(.disabled):not(.loading):hover {
        color: var(--wa-color-on-quiet, var(--wa-color-neutral-on-quiet));
        background-color: var(--wa-color-fill-quiet, var(--wa-color-neutral-fill-quiet));
      }
    }
    .button:not(.disabled):not(.loading):active {
      color: var(--wa-color-on-quiet, var(--wa-color-neutral-on-quiet));
      background-color: color-mix(
        in oklab,
        var(--wa-color-fill-quiet, var(--wa-color-neutral-fill-quiet)),
        var(--wa-color-mix-active)
      );
    }
  }

  :host([appearance='outlined']) {
    /* Indentation overrides for grouping outlined */
    margin-inline-start: var(--_button-horizontal-indent-outlined);
    margin-block-start: var(--_button-vertical-indent-outlined);

    .button {
      color: var(--wa-color-on-quiet, var(--wa-color-neutral-on-quiet));
      background-color: transparent;
      border-color: var(--wa-color-border-loud, var(--wa-color-neutral-border-loud));
    }
    @media (hover: hover) {
      .button:not(.disabled):not(.loading):hover {
        color: var(--wa-color-on-quiet, var(--wa-color-neutral-on-quiet));
        background-color: var(--wa-color-fill-quiet, var(--wa-color-neutral-fill-quiet));
      }
    }
    .button:not(.disabled):not(.loading):active {
      color: var(--wa-color-on-quiet, var(--wa-color-neutral-on-quiet));
      background-color: color-mix(
        in oklab,
        var(--wa-color-fill-quiet, var(--wa-color-neutral-fill-quiet)),
        var(--wa-color-mix-active)
      );
    }
  }

  :host([appearance='filled']) {
    /* Indentation overrides for grouping */
    margin-inline-start: var(--_button-horizontal-indent);
    margin-block-start: var(--_button-vertical-indent);

    .button {
      color: var(--wa-color-on-normal, var(--wa-color-neutral-on-normal));
      background-color: var(--wa-color-fill-normal, var(--wa-color-neutral-fill-normal));
      border-color: transparent;
    }
    @media (hover: hover) {
      .button:not(.disabled):not(.loading):hover {
        color: var(--wa-color-on-normal, var(--wa-color-neutral-on-normal));
        background-color: color-mix(
          in oklab,
          var(--wa-color-fill-normal, var(--wa-color-neutral-fill-normal)),
          var(--wa-color-mix-hover)
        );
      }
    }
    .button:not(.disabled):not(.loading):active {
      color: var(--wa-color-on-normal, var(--wa-color-neutral-on-normal));
      background-color: color-mix(
        in oklab,
        var(--wa-color-fill-normal, var(--wa-color-neutral-fill-normal)),
        var(--wa-color-mix-active)
      );
    }
  }

  :host([appearance='filled-outlined']) {
    /* Indentation overrides for grouping outlined */
    margin-inline-start: var(--_button-horizontal-indent-outlined);
    margin-block-start: var(--_button-vertical-indent-outlined);

    .button {
      color: var(--wa-color-on-normal, var(--wa-color-neutral-on-normal));
      background-color: var(--wa-color-fill-normal, var(--wa-color-neutral-fill-normal));
      border-color: var(--wa-color-border-normal, var(--wa-color-neutral-border-normal));
    }
    @media (hover: hover) {
      .button:not(.disabled):not(.loading):hover {
        color: var(--wa-color-on-normal, var(--wa-color-neutral-on-normal));
        background-color: color-mix(
          in oklab,
          var(--wa-color-fill-normal, var(--wa-color-neutral-fill-normal)),
          var(--wa-color-mix-hover)
        );
      }
    }
    .button:not(.disabled):not(.loading):active {
      color: var(--wa-color-on-normal, var(--wa-color-neutral-on-normal));
      background-color: color-mix(
        in oklab,
        var(--wa-color-fill-normal, var(--wa-color-neutral-fill-normal)),
        var(--wa-color-mix-active)
      );
    }
  }

  :host([appearance='accent']) {
    /* Indentation overrides for grouping */
    margin-inline-start: var(--_button-horizontal-indent);
    margin-block-start: var(--_button-vertical-indent);

    .button {
      color: var(--wa-color-on-loud, var(--wa-color-neutral-on-loud));
      background-color: var(--wa-color-fill-loud, var(--wa-color-neutral-fill-loud));
      border-color: transparent;
    }
    @media (hover: hover) {
      .button:not(.disabled):not(.loading):hover {
        background-color: color-mix(
          in oklab,
          var(--wa-color-fill-loud, var(--wa-color-neutral-fill-loud)),
          var(--wa-color-mix-hover)
        );
      }
    }
    .button:not(.disabled):not(.loading):active {
      background-color: color-mix(
        in oklab,
        var(--wa-color-fill-loud, var(--wa-color-neutral-fill-loud)),
        var(--wa-color-mix-active)
      );
    }
  }

  /* Focus states */
  .button:focus {
    outline: none;
  }

  .button:focus-visible {
    outline: var(--wa-focus-ring);
    outline-offset: var(--wa-focus-ring-offset);
  }

  /* Disabled state */
  :host([disabled]) {
    opacity: 0.5;
    cursor: not-allowed;

    /* When disabled, prevent mouse events from bubbling up from children */
    .button {
      pointer-events: none;
    }
  }

  /* Keep it last so Safari doesn't stop parsing this block */
  .button::-moz-focus-inner {
    border: 0;
  }

  /* Icon buttons */
  .button.is-icon-button {
    outline-offset: 2px;
    width: var(--wa-form-control-height);
    aspect-ratio: 1;
  }

  .button.is-icon-button:has(wa-icon) {
    width: auto;
  }

  /* Pill modifier */
  :host([pill]) .button {
    border-start-start-radius: var(--_button-start-start-radius, var(--wa-border-radius-pill));
    border-start-end-radius: var(--_button-start-end-radius, var(--wa-border-radius-pill));
    border-end-start-radius: var(--_button-end-start-radius, var(--wa-border-radius-pill));
    border-end-end-radius: var(--_button-end-end-radius, var(--wa-border-radius-pill));
  }

  /*
   * Label
   */

  .start,
  .end {
    flex: 0 0 auto;
    display: flex;
    align-items: center;
    pointer-events: none;
  }

  .label {
    display: inline-block;
  }

  .is-icon-button .label {
    display: flex;
  }

  .label::slotted(wa-icon) {
    align-self: center;
  }

  /*
   * Caret modifier
   */

  wa-icon[part='caret'] {
    display: flex;
    align-self: center;
    align-items: center;

    &::part(svg) {
      width: 0.875em;
      height: 0.875em;
    }

    .button:has(&) .end {
      display: none;
    }
  }

  /*
   * Loading modifier
   */

  .loading {
    position: relative;
    cursor: wait;

    .start,
    .label,
    .end,
    .caret {
      visibility: hidden;
    }

    wa-spinner {
      --indicator-color: currentColor;
      --track-color: color-mix(in oklab, currentColor, transparent 90%);

      position: absolute;
      font-size: 1em;
      height: 1em;
      width: 1em;
      top: calc(50% - 0.5em);
      left: calc(50% - 0.5em);
    }
  }

  /*
   * Badges
   */

  .button ::slotted(wa-badge) {
    border-color: var(--wa-color-surface-default);
    position: absolute;
    inset-block-start: 0;
    inset-inline-end: 0;
    translate: 50% -50%;
    pointer-events: none;
  }

  :host(:dir(rtl)) ::slotted(wa-badge) {
    translate: -50% -50%;
  }

  /*
  * Button spacing
  */

  slot[name='start']::slotted(*) {
    margin-inline-end: 0.75em;
  }

  slot[name='end']::slotted(*),
  .button:not(.visually-hidden-label) [part='caret'] {
    margin-inline-start: 0.75em;
  }
`;

// chunks/chunk.MEYJNQF4.js
var size_styles_default = i2`
  :host([size='small']),
  .wa-size-s {
    font-size: var(--wa-font-size-s);
  }

  :host([size='medium']),
  .wa-size-m {
    font-size: var(--wa-font-size-m);
  }

  :host([size='large']),
  .wa-size-l {
    font-size: var(--wa-font-size-l);
  }
`;

// chunks/chunk.UVLZVEH2.js
var variants_styles_default = i2`
  :where(:root),
  .wa-neutral,
  :host([variant='neutral']) {
    --wa-color-fill-loud: var(--wa-color-neutral-fill-loud);
    --wa-color-fill-normal: var(--wa-color-neutral-fill-normal);
    --wa-color-fill-quiet: var(--wa-color-neutral-fill-quiet);
    --wa-color-border-loud: var(--wa-color-neutral-border-loud);
    --wa-color-border-normal: var(--wa-color-neutral-border-normal);
    --wa-color-border-quiet: var(--wa-color-neutral-border-quiet);
    --wa-color-on-loud: var(--wa-color-neutral-on-loud);
    --wa-color-on-normal: var(--wa-color-neutral-on-normal);
    --wa-color-on-quiet: var(--wa-color-neutral-on-quiet);
  }

  .wa-brand,
  :host([variant='brand']) {
    --wa-color-fill-loud: var(--wa-color-brand-fill-loud);
    --wa-color-fill-normal: var(--wa-color-brand-fill-normal);
    --wa-color-fill-quiet: var(--wa-color-brand-fill-quiet);
    --wa-color-border-loud: var(--wa-color-brand-border-loud);
    --wa-color-border-normal: var(--wa-color-brand-border-normal);
    --wa-color-border-quiet: var(--wa-color-brand-border-quiet);
    --wa-color-on-loud: var(--wa-color-brand-on-loud);
    --wa-color-on-normal: var(--wa-color-brand-on-normal);
    --wa-color-on-quiet: var(--wa-color-brand-on-quiet);
  }

  .wa-success,
  :host([variant='success']) {
    --wa-color-fill-loud: var(--wa-color-success-fill-loud);
    --wa-color-fill-normal: var(--wa-color-success-fill-normal);
    --wa-color-fill-quiet: var(--wa-color-success-fill-quiet);
    --wa-color-border-loud: var(--wa-color-success-border-loud);
    --wa-color-border-normal: var(--wa-color-success-border-normal);
    --wa-color-border-quiet: var(--wa-color-success-border-quiet);
    --wa-color-on-loud: var(--wa-color-success-on-loud);
    --wa-color-on-normal: var(--wa-color-success-on-normal);
    --wa-color-on-quiet: var(--wa-color-success-on-quiet);
  }

  .wa-warning,
  :host([variant='warning']) {
    --wa-color-fill-loud: var(--wa-color-warning-fill-loud);
    --wa-color-fill-normal: var(--wa-color-warning-fill-normal);
    --wa-color-fill-quiet: var(--wa-color-warning-fill-quiet);
    --wa-color-border-loud: var(--wa-color-warning-border-loud);
    --wa-color-border-normal: var(--wa-color-warning-border-normal);
    --wa-color-border-quiet: var(--wa-color-warning-border-quiet);
    --wa-color-on-loud: var(--wa-color-warning-on-loud);
    --wa-color-on-normal: var(--wa-color-warning-on-normal);
    --wa-color-on-quiet: var(--wa-color-warning-on-quiet);
  }

  .wa-danger,
  :host([variant='danger']) {
    --wa-color-fill-loud: var(--wa-color-danger-fill-loud);
    --wa-color-fill-normal: var(--wa-color-danger-fill-normal);
    --wa-color-fill-quiet: var(--wa-color-danger-fill-quiet);
    --wa-color-border-loud: var(--wa-color-danger-border-loud);
    --wa-color-border-normal: var(--wa-color-danger-border-normal);
    --wa-color-border-quiet: var(--wa-color-danger-border-quiet);
    --wa-color-on-loud: var(--wa-color-danger-on-loud);
    --wa-color-on-normal: var(--wa-color-danger-on-normal);
    --wa-color-on-quiet: var(--wa-color-danger-on-quiet);
  }
`;

// chunks/chunk.3MSWQ3RG.js
var o5 = (o25) => o25 ?? E;

// chunks/chunk.HPOJN4W7.js
var connectedElements = /* @__PURE__ */ new Set();
var translations = /* @__PURE__ */ new Map();
var fallback;
var documentDirection = "ltr";
var documentLanguage = "en";
var isClient = typeof MutationObserver !== "undefined" && typeof document !== "undefined" && typeof document.documentElement !== "undefined";
if (isClient) {
  const documentElementObserver = new MutationObserver(update);
  documentDirection = document.documentElement.dir || "ltr";
  documentLanguage = document.documentElement.lang || navigator.language;
  documentElementObserver.observe(document.documentElement, {
    attributes: true,
    attributeFilter: ["dir", "lang"]
  });
}
function registerTranslation(...translation2) {
  translation2.map((t5) => {
    const code = t5.$code.toLowerCase();
    if (translations.has(code)) {
      translations.set(code, Object.assign(Object.assign({}, translations.get(code)), t5));
    } else {
      translations.set(code, t5);
    }
    if (!fallback) {
      fallback = t5;
    }
  });
  update();
}
function update() {
  if (isClient) {
    documentDirection = document.documentElement.dir || "ltr";
    documentLanguage = document.documentElement.lang || navigator.language;
  }
  [...connectedElements.keys()].map((el) => {
    if (typeof el.requestUpdate === "function") {
      el.requestUpdate();
    }
  });
}
var LocalizeController = class {
  constructor(host) {
    this.host = host;
    this.host.addController(this);
  }
  hostConnected() {
    connectedElements.add(this.host);
  }
  hostDisconnected() {
    connectedElements.delete(this.host);
  }
  dir() {
    return `${this.host.dir || documentDirection}`.toLowerCase();
  }
  lang() {
    return `${this.host.lang || documentLanguage}`.toLowerCase();
  }
  getTranslationData(lang) {
    var _a, _b;
    const locale = new Intl.Locale(lang.replace(/_/g, "-"));
    const language = locale === null || locale === void 0 ? void 0 : locale.language.toLowerCase();
    const region = (_b = (_a = locale === null || locale === void 0 ? void 0 : locale.region) === null || _a === void 0 ? void 0 : _a.toLowerCase()) !== null && _b !== void 0 ? _b : "";
    const primary = translations.get(`${language}-${region}`);
    const secondary = translations.get(language);
    return { locale, language, region, primary, secondary };
  }
  exists(key, options) {
    var _a;
    const { primary, secondary } = this.getTranslationData((_a = options.lang) !== null && _a !== void 0 ? _a : this.lang());
    options = Object.assign({ includeFallback: false }, options);
    if (primary && primary[key] || secondary && secondary[key] || options.includeFallback && fallback && fallback[key]) {
      return true;
    }
    return false;
  }
  term(key, ...args) {
    const { primary, secondary } = this.getTranslationData(this.lang());
    let term;
    if (primary && primary[key]) {
      term = primary[key];
    } else if (secondary && secondary[key]) {
      term = secondary[key];
    } else if (fallback && fallback[key]) {
      term = fallback[key];
    } else {
      console.error(`No translation found for: ${String(key)}`);
      return String(key);
    }
    if (typeof term === "function") {
      return term(...args);
    }
    return term;
  }
  date(dateToFormat, options) {
    dateToFormat = new Date(dateToFormat);
    return new Intl.DateTimeFormat(this.lang(), options).format(dateToFormat);
  }
  number(numberToFormat, options) {
    numberToFormat = Number(numberToFormat);
    return isNaN(numberToFormat) ? "" : new Intl.NumberFormat(this.lang(), options).format(numberToFormat);
  }
  relativeTime(value, unit, options) {
    return new Intl.RelativeTimeFormat(this.lang(), options).format(value, unit);
  }
};

// chunks/chunk.5OV4QM6R.js
var translation = {
  $code: "en",
  $name: "English",
  $dir: "ltr",
  carousel: "Carousel",
  clearEntry: "Clear entry",
  close: "Close",
  createOption: (value) => `Create "${value}"`,
  copied: "Copied",
  copy: "Copy",
  currentValue: "Current value",
  dropFileHere: "Drop file here or click to browse",
  decrement: "Decrement",
  dropFilesHere: "Drop files here or click to browse",
  error: "Error",
  goToSlide: (slide, count) => `Go to slide ${slide} of ${count}`,
  hidePassword: "Hide password",
  increment: "Increment",
  loading: "Loading",
  nextSlide: "Next slide",
  numCharacters: (num) => {
    if (num === 1)
      return "1 character";
    return `${num} characters`;
  },
  numCharactersRemaining: (num) => {
    if (num === 1)
      return "1 character remaining";
    return `${num} characters remaining`;
  },
  numOptionsSelected: (num) => {
    if (num === 0)
      return "No options selected";
    if (num === 1)
      return "1 option selected";
    return `${num} options selected`;
  },
  pauseAnimation: "Pause animation",
  playAnimation: "Play animation",
  previousSlide: "Previous slide",
  progress: "Progress",
  remove: "Remove",
  resize: "Resize",
  scrollableRegion: "Scrollable region",
  scrollToEnd: "Scroll to end",
  scrollToStart: "Scroll to start",
  selectAColorFromTheScreen: "Select a color from the screen",
  showPassword: "Show password",
  slideNum: (slide) => `Slide ${slide}`,
  toggleColorFormat: "Toggle color format",
  zoomIn: "Zoom in",
  zoomOut: "Zoom out"
};
registerTranslation(translation);
var en_default = translation;

// chunks/chunk.G3ZVQTTB.js
var LocalizeController2 = class extends LocalizeController {
};
registerTranslation(en_default);

// chunks/chunk.PZAN6FPN.js
function watch(propertyName, options) {
  const resolvedOptions = {
    waitUntilFirstUpdate: false,
    ...options
  };
  return (proto, decoratedFnName) => {
    const { update: update2 } = proto;
    const watchedProperties = Array.isArray(propertyName) ? propertyName : [propertyName];
    proto.update = function(changedProps) {
      watchedProperties.forEach((property) => {
        const key = property;
        if (changedProps.has(key)) {
          const oldValue = changedProps.get(key);
          const newValue = this[key];
          if (oldValue !== newValue) {
            if (!resolvedOptions.waitUntilFirstUpdate || this.hasUpdated) {
              this[decoratedFnName](oldValue, newValue);
            }
          }
        }
      });
      update2.call(this, changedProps);
    };
  };
}

// chunks/chunk.CNYRG3XY.js
var a3 = Symbol.for("");
var o24 = (t23) => {
  if (t23?.r === a3)
    return t23?._$litStatic$;
};
var i5 = (t23, ...r24) => ({ _$litStatic$: r24.reduce((r32, e33, a22) => r32 + ((t32) => {
  if (t32._$litStatic$ !== void 0)
    return t32._$litStatic$;
  throw Error(`Value passed to 'literal' function must be a 'literal' result: ${t32}. Use 'unsafeStatic' to pass non-literal values, but
            take care to ensure page security.`);
})(e33) + t23[a22 + 1], t23[0]), r: a3 });
var l3 = /* @__PURE__ */ new Map();
var n23 = (t23) => (r24, ...e33) => {
  const a22 = e33.length;
  let s3, i24;
  const n32 = [], u22 = [];
  let c23, $22 = 0, f4 = false;
  for (; $22 < a22; ) {
    for (c23 = r24[$22]; $22 < a22 && (i24 = e33[$22], s3 = o24(i24)) !== void 0; )
      c23 += s3 + r24[++$22], f4 = true;
    $22 !== a22 && u22.push(i24), n32.push(c23), $22++;
  }
  if ($22 === a22 && n32.push(r24[a22]), f4) {
    const t32 = n32.join("$$lit$$");
    (r24 = l3.get(t32)) === void 0 && (n32.raw = n32, l3.set(t32, r24 = n32)), e33 = u22;
  }
  return t23(r24, ...e33);
};
var u3 = n23(x);
var c3 = n23(b);
var $2 = n23(w);
var WaButton = class extends WebAwesomeFormAssociatedElement {
  constructor() {
    super(...arguments);
    this.assumeInteractionOn = ["click"];
    this.hasSlotController = new HasSlotController(this, "[default]", "start", "end");
    this.localize = new LocalizeController2(this);
    this.invalid = false;
    this.isIconButton = false;
    this.title = "";
    this.variant = "neutral";
    this.appearance = "accent";
    this.size = "medium";
    this.withCaret = false;
    this.withStart = false;
    this.withEnd = false;
    this.disabled = false;
    this.loading = false;
    this.pill = false;
    this.type = "button";
  }
  static get validators() {
    return [...super.validators, MirrorValidator()];
  }
  constructLightDOMButton() {
    const button = document.createElement("button");
    for (const attribute of this.attributes) {
      if (attribute.name === "style") {
        continue;
      }
      button.setAttribute(attribute.name, attribute.value);
    }
    button.type = this.type;
    button.style.position = "absolute !important";
    button.style.width = "0 !important";
    button.style.height = "0 !important";
    button.style.clipPath = "inset(50%) !important";
    button.style.overflow = "hidden !important";
    button.style.whiteSpace = "nowrap !important";
    if (this.name) {
      button.name = this.name;
    }
    button.value = this.value || "";
    return button;
  }
  handleClick(event) {
    if (this.disabled || this.loading) {
      event.preventDefault();
      event.stopImmediatePropagation();
      return;
    }
    if (this.type !== "submit" && this.type !== "reset") {
      return;
    }
    const form = this.getForm();
    if (!form)
      return;
    const lightDOMButton = this.constructLightDOMButton();
    this.parentElement?.append(lightDOMButton);
    lightDOMButton.click();
    lightDOMButton.remove();
  }
  handleInvalid() {
    this.dispatchEvent(new WaInvalidEvent());
  }
  handleLabelSlotChange() {
    const nodes = this.labelSlot.assignedNodes({ flatten: true });
    let hasIconLabel = false;
    let hasIcon = false;
    let hasText = false;
    let hasOtherElements = false;
    [...nodes].forEach((node) => {
      if (node.nodeType === Node.ELEMENT_NODE) {
        const element = node;
        if (element.localName === "wa-icon") {
          hasIcon = true;
          if (!hasIconLabel)
            hasIconLabel = element.label !== void 0;
        } else {
          hasOtherElements = true;
        }
      } else if (node.nodeType === Node.TEXT_NODE) {
        const text = node.textContent?.trim() || "";
        if (text.length > 0) {
          hasText = true;
        }
      }
    });
    this.isIconButton = hasIcon && !hasText && !hasOtherElements;
    this.customStates.set("icon-button", this.isIconButton);
    if (this.isIconButton && !hasIconLabel) {
      console.warn('Icon buttons must have a label for screen readers. Add <wa-icon label="..."> to remove this warning.', this);
    }
  }
  isButton() {
    return this.href ? false : true;
  }
  isLink() {
    return this.href ? true : false;
  }
  handleDisabledChange() {
    this.customStates.set("disabled", this.disabled);
    this.updateValidity();
  }
  handleHrefChange() {
    this.customStates.set("link", this.isLink());
  }
  handleLoadingChange() {
    this.customStates.set("loading", this.loading);
  }
  setValue(..._args) {
  }
  click() {
    this.button.click();
  }
  focus(options) {
    this.button.focus(options);
  }
  blur() {
    this.button.blur();
  }
  render() {
    const isLink = this.isLink();
    const tag = isLink ? i5`a` : i5`button`;
    return u3`
      <${tag}
        part="base"
        class=${e24({
      button: true,
      caret: this.withCaret,
      disabled: this.disabled,
      loading: this.loading,
      rtl: this.localize.dir() === "rtl",
      "has-label": this.hasSlotController.test("[default]"),
      "has-start": this.hasUpdated ? this.hasSlotController.test("start") : this.withStart,
      "has-end": this.hasUpdated ? this.hasSlotController.test("end") : this.withEnd,
      "is-icon-button": this.isIconButton
    })}
        ?disabled=${o5(isLink ? void 0 : this.disabled)}
        type=${o5(isLink ? void 0 : this.type)}
        title=${this.title}
        name=${o5(isLink ? void 0 : this.name)}
        value=${o5(isLink ? void 0 : this.value)}
        href=${o5(isLink ? this.href : void 0)}
        target=${o5(isLink ? this.target : void 0)}
        download=${o5(isLink ? this.download : void 0)}
        rel=${o5(isLink && this.rel ? this.rel : void 0)}
        role=${o5(isLink ? void 0 : "button")}
        aria-disabled=${o5(isLink && this.disabled ? "true" : void 0)}
        tabindex=${this.disabled ? "-1" : "0"}
        @invalid=${this.isButton() ? this.handleInvalid : null}
        @click=${this.handleClick}
      >
        <slot name="start" part="start" class="start"></slot>
        <slot part="label" class="label" @slotchange=${this.handleLabelSlotChange}></slot>
        <slot name="end" part="end" class="end"></slot>
        ${this.withCaret ? u3`
                <wa-icon part="caret" class="caret" library="system" name="chevron-down" variant="solid"></wa-icon>
              ` : ""}
        ${this.loading ? u3`<wa-spinner part="spinner"></wa-spinner>` : ""}
      </${tag}>
    `;
  }
};
WaButton.shadowRootOptions = { ...WebAwesomeFormAssociatedElement.shadowRootOptions, delegatesFocus: true };
WaButton.css = [button_styles_default, variants_styles_default, size_styles_default];
__decorateClass([
  e23(".button")
], WaButton.prototype, "button", 2);
__decorateClass([
  e23("slot:not([name])")
], WaButton.prototype, "labelSlot", 2);
__decorateClass([
  r23()
], WaButton.prototype, "invalid", 2);
__decorateClass([
  r23()
], WaButton.prototype, "isIconButton", 2);
__decorateClass([
  n3()
], WaButton.prototype, "title", 2);
__decorateClass([
  n3({ reflect: true })
], WaButton.prototype, "variant", 2);
__decorateClass([
  n3({ reflect: true })
], WaButton.prototype, "appearance", 2);
__decorateClass([
  n3({ reflect: true })
], WaButton.prototype, "size", 2);
__decorateClass([
  n3({ attribute: "with-caret", type: Boolean, reflect: true })
], WaButton.prototype, "withCaret", 2);
__decorateClass([
  n3({ attribute: "with-start", type: Boolean })
], WaButton.prototype, "withStart", 2);
__decorateClass([
  n3({ attribute: "with-end", type: Boolean })
], WaButton.prototype, "withEnd", 2);
__decorateClass([
  n3({ type: Boolean })
], WaButton.prototype, "disabled", 2);
__decorateClass([
  n3({ type: Boolean, reflect: true })
], WaButton.prototype, "loading", 2);
__decorateClass([
  n3({ type: Boolean, reflect: true })
], WaButton.prototype, "pill", 2);
__decorateClass([
  n3()
], WaButton.prototype, "type", 2);
__decorateClass([
  n3({ reflect: true })
], WaButton.prototype, "name", 2);
__decorateClass([
  n3({ reflect: true })
], WaButton.prototype, "value", 2);
__decorateClass([
  n3({ reflect: true })
], WaButton.prototype, "href", 2);
__decorateClass([
  n3()
], WaButton.prototype, "target", 2);
__decorateClass([
  n3()
], WaButton.prototype, "rel", 2);
__decorateClass([
  n3()
], WaButton.prototype, "download", 2);
__decorateClass([
  n3({ attribute: "formaction" })
], WaButton.prototype, "formAction", 2);
__decorateClass([
  n3({ attribute: "formenctype" })
], WaButton.prototype, "formEnctype", 2);
__decorateClass([
  n3({ attribute: "formmethod" })
], WaButton.prototype, "formMethod", 2);
__decorateClass([
  n3({ attribute: "formnovalidate", type: Boolean })
], WaButton.prototype, "formNoValidate", 2);
__decorateClass([
  n3({ attribute: "formtarget" })
], WaButton.prototype, "formTarget", 2);
__decorateClass([
  watch("disabled", { waitUntilFirstUpdate: true })
], WaButton.prototype, "handleDisabledChange", 1);
__decorateClass([
  watch("href")
], WaButton.prototype, "handleHrefChange", 1);
__decorateClass([
  watch("loading", { waitUntilFirstUpdate: true })
], WaButton.prototype, "handleLoadingChange", 1);
WaButton = __decorateClass([
  t3("wa-button")
], WaButton);
WaButton.disableWarning?.("change-in-update");

// chunks/chunk.IDV4GXTH.js
var spinner_styles_default = i2`
  :host {
    --track-width: 2px;
    --track-color: var(--wa-color-neutral-fill-normal);
    --indicator-color: var(--wa-color-brand-fill-loud);
    --speed: 2s;
    --size: 1em;

    /*
      Resizing a spinner element using anything but font-size will break the animation because the animation uses em
      units. Therefore, if a spinner is used in a flex container without \`flex: none\` applied, the spinner can
      grow/shrink and break the animation. The use of \`flex: none\` on the host element prevents this by always having
      the spinner sized according to its actual dimensions.
    */
    flex: none;
    display: inline-flex;
    width: var(--size);
    height: var(--size);
  }

  svg {
    width: 100%;
    height: 100%;
    aspect-ratio: 1;
    animation: spin var(--speed) linear infinite;
  }

  .track,
  .indicator {
    --radius: calc(var(--size) / 2 - var(--track-width) / 2);
    --circumference: calc(var(--radius) * 2 * 3.141592654);

    cx: calc(var(--size) / 2);
    cy: calc(var(--size) / 2);
    r: var(--radius);
    fill: none;
    stroke-width: var(--track-width);
  }

  .track {
    stroke: var(--track-color);
  }

  .indicator {
    stroke: var(--indicator-color);
    stroke-linecap: round;
    stroke-dasharray: calc(0.597 * var(--circumference)), calc(0.796 * var(--circumference));
    stroke-dashoffset: calc(-0.04 * var(--circumference));
    animation: dash 1.5s ease-in-out infinite;
  }

  @keyframes spin {
    0% {
      transform: rotate(0deg);
    }
    100% {
      transform: rotate(360deg);
    }
  }

  @keyframes dash {
    0% {
      stroke-dasharray: calc(0.008 * var(--circumference)), calc(1.194 * var(--circumference));
      stroke-dashoffset: 0;
    }
    50% {
      stroke-dasharray: calc(0.716 * var(--circumference)), calc(1.194 * var(--circumference));
      stroke-dashoffset: calc(-0.278 * var(--circumference));
    }
    100% {
      stroke-dasharray: calc(0.716 * var(--circumference)), calc(1.194 * var(--circumference));
      stroke-dashoffset: calc(-0.987 * var(--circumference));
    }
  }
`;

// chunks/chunk.NOITEVBZ.js
var WaSpinner = class extends WebAwesomeElement {
  constructor() {
    super(...arguments);
    this.localize = new LocalizeController2(this);
  }
  render() {
    return x`
      <svg
        part="base"
        role="progressbar"
        aria-label=${this.localize.term("loading")}
        fill="none"
        xmlns="http://www.w3.org/2000/svg"
      >
        <circle class="track" />
        <circle class="indicator" />
      </svg>
    `;
  }
};
WaSpinner.css = spinner_styles_default;
WaSpinner = __decorateClass([
  t3("wa-spinner")
], WaSpinner);

// chunks/chunk.YDQCS2HK.js
var WaErrorEvent = class extends Event {
  constructor() {
    super("wa-error", { bubbles: true, cancelable: false, composed: true });
  }
};

// chunks/chunk.WDIIGUNP.js
var WaLoadEvent = class extends Event {
  constructor() {
    super("wa-load", { bubbles: true, cancelable: false, composed: true });
  }
};

// chunks/chunk.UYB2BRKW.js
var icon_styles_default = i2`
  :host {
    --primary-color: currentColor;
    --primary-opacity: 1;
    --secondary-color: currentColor;
    --secondary-opacity: 0.4;
    --rotate-angle: 0deg;

    box-sizing: content-box;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    vertical-align: -0.125em;
  }

  /* Standard */
  :host(:not([auto-width])) {
    width: 1.25em;
    height: 1em;
  }

  /* Auto-width */
  :host([auto-width]) {
    width: auto;
    height: 1em;
  }

  svg {
    height: 1em;
    overflow: visible;
    width: auto;

    /* Duotone colors with path-specific opacity fallback */
    path[data-duotone-primary] {
      color: var(--primary-color);
      opacity: var(--path-opacity, var(--primary-opacity));
    }

    path[data-duotone-secondary] {
      color: var(--secondary-color);
      opacity: var(--path-opacity, var(--secondary-opacity));
    }
  }

  /* Rotation */
  :host([rotate]) {
    transform: rotate(var(--rotate-angle, 0deg));
  }

  /* Flipping */
  :host([flip='x']) {
    transform: scaleX(-1);
  }
  :host([flip='y']) {
    transform: scaleY(-1);
  }
  :host([flip='both']) {
    transform: scale(-1, -1);
  }

  /* Rotation and Flipping combined */
  :host([rotate][flip='x']) {
    transform: rotate(var(--rotate-angle, 0deg)) scaleX(-1);
  }
  :host([rotate][flip='y']) {
    transform: rotate(var(--rotate-angle, 0deg)) scaleY(-1);
  }
  :host([rotate][flip='both']) {
    transform: rotate(var(--rotate-angle, 0deg)) scale(-1, -1);
  }

  /* Animations */
  :host([animation='beat']) {
    animation-name: beat;
    animation-delay: var(--animation-delay, 0s);
    animation-direction: var(--animation-direction, normal);
    animation-duration: var(--animation-duration, 1s);
    animation-iteration-count: var(--animation-iteration-count, infinite);
    animation-timing-function: var(--animation-timing, ease-in-out);
  }

  :host([animation='fade']) {
    animation-name: fade;
    animation-delay: var(--animation-delay, 0s);
    animation-direction: var(--animation-direction, normal);
    animation-duration: var(--animation-duration, 1s);
    animation-iteration-count: var(--animation-iteration-count, infinite);
    animation-timing-function: var(--animation-timing, cubic-bezier(0.4, 0, 0.6, 1));
  }

  :host([animation='beat-fade']) {
    animation-name: beat-fade;
    animation-delay: var(--animation-delay, 0s);
    animation-direction: var(--animation-direction, normal);
    animation-duration: var(--animation-duration, 1s);
    animation-iteration-count: var(--animation-iteration-count, infinite);
    animation-timing-function: var(--animation-timing, cubic-bezier(0.4, 0, 0.6, 1));
  }

  :host([animation='bounce']) {
    animation-name: bounce;
    animation-delay: var(--animation-delay, 0s);
    animation-direction: var(--animation-direction, normal);
    animation-duration: var(--animation-duration, 1s);
    animation-iteration-count: var(--animation-iteration-count, infinite);
    animation-timing-function: var(--animation-timing, cubic-bezier(0.28, 0.84, 0.42, 1));
  }

  :host([animation='flip']) {
    animation-name: flip;
    animation-delay: var(--animation-delay, 0s);
    animation-direction: var(--animation-direction, normal);
    animation-duration: var(--animation-duration, 1s);
    animation-iteration-count: var(--animation-iteration-count, infinite);
    animation-timing-function: var(--animation-timing, ease-in-out);
  }

  :host([animation='shake']) {
    animation-name: shake;
    animation-delay: var(--animation-delay, 0s);
    animation-direction: var(--animation-direction, normal);
    animation-duration: var(--animation-duration, 1s);
    animation-iteration-count: var(--animation-iteration-count, infinite);
    animation-timing-function: var(--animation-timing, linear);
  }

  :host([animation='spin']) {
    animation-name: spin;
    animation-delay: var(--animation-delay, 0s);
    animation-direction: var(--animation-direction, normal);
    animation-duration: var(--animation-duration, 2s);
    animation-iteration-count: var(--animation-iteration-count, infinite);
    animation-timing-function: var(--animation-timing, linear);
  }

  :host([animation='spin-pulse']) {
    animation-name: spin-pulse;
    animation-direction: var(--animation-direction, normal);
    animation-duration: var(--animation-duration, 1s);
    animation-iteration-count: var(--animation-iteration-count, infinite);
    animation-timing-function: var(--animation-timing, steps(8));
  }

  :host([animation='spin-reverse']) {
    animation-name: spin;
    animation-delay: var(--animation-delay, 0s);
    animation-direction: var(--animation-direction, reverse);
    animation-duration: var(--animation-duration, 2s);
    animation-iteration-count: var(--animation-iteration-count, infinite);
    animation-timing-function: var(--animation-timing, linear);
  }

  /* Keyframes */
  @media (prefers-reduced-motion: reduce) {
    :host([animation='beat']),
    :host([animation='bounce']),
    :host([animation='fade']),
    :host([animation='beat-fade']),
    :host([animation='flip']),
    :host([animation='shake']),
    :host([animation='spin']),
    :host([animation='spin-pulse']),
    :host([animation='spin-reverse']) {
      animation: none !important;
      transition: none !important;
    }
  }
  @keyframes beat {
    0%,
    90% {
      transform: scale(1);
    }
    45% {
      transform: scale(var(--beat-scale, 1.25));
    }
  }

  @keyframes fade {
    50% {
      opacity: var(--fade-opacity, 0.4);
    }
  }

  @keyframes beat-fade {
    0%,
    100% {
      opacity: var(--beat-fade-opacity, 0.4);
      transform: scale(1);
    }
    50% {
      opacity: 1;
      transform: scale(var(--beat-fade-scale, 1.125));
    }
  }

  @keyframes bounce {
    0% {
      transform: scale(1, 1) translateY(0);
    }
    10% {
      transform: scale(var(--bounce-start-scale-x, 1.1), var(--bounce-start-scale-y, 0.9)) translateY(0);
    }
    30% {
      transform: scale(var(--bounce-jump-scale-x, 0.9), var(--bounce-jump-scale-y, 1.1))
        translateY(var(--bounce-height, -0.5em));
    }
    50% {
      transform: scale(var(--bounce-land-scale-x, 1.05), var(--bounce-land-scale-y, 0.95)) translateY(0);
    }
    57% {
      transform: scale(1, 1) translateY(var(--bounce-rebound, -0.125em));
    }
    64% {
      transform: scale(1, 1) translateY(0);
    }
    100% {
      transform: scale(1, 1) translateY(0);
    }
  }

  @keyframes flip {
    50% {
      transform: rotate3d(var(--flip-x, 0), var(--flip-y, 1), var(--flip-z, 0), var(--flip-angle, -180deg));
    }
  }

  @keyframes shake {
    0% {
      transform: rotate(-15deg);
    }
    4% {
      transform: rotate(15deg);
    }
    8%,
    24% {
      transform: rotate(-18deg);
    }
    12%,
    28% {
      transform: rotate(18deg);
    }
    16% {
      transform: rotate(-22deg);
    }
    20% {
      transform: rotate(22deg);
    }
    32% {
      transform: rotate(-12deg);
    }
    36% {
      transform: rotate(12deg);
    }
    40%,
    100% {
      transform: rotate(0deg);
    }
  }

  @keyframes spin {
    0% {
      transform: rotate(0deg);
    }
    100% {
      transform: rotate(360deg);
    }
  }

  @keyframes spin-pulse {
    0% {
      transform: rotate(0deg);
    }
    100% {
      transform: rotate(360deg);
    }
  }
`;

// chunks/chunk.T3OVPJUT.js
var e5 = (o7, t23) => t23 === void 0 ? o7?._$litType$ !== void 0 : o7?._$litType$ === t23;
var f3 = (o7) => o7.strings === void 0;
var u4 = {};
var m2 = (o7, t23 = u4) => o7._$AH = t23;

// chunks/chunk.KPN3YZ6U.js
function dataUri(svg) {
  return `data:image/svg+xml,${encodeURIComponent(svg)}`;
}
var icons = {
  solid: {
    check: `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 448 512"><!--! Font Awesome Free 7.0.0 by @fontawesome - https://fontawesome.com License - https://fontawesome.com/license/free Copyright 2025 Fonticons, Inc. --><path fill="currentColor" d="M434.8 70.1c14.3 10.4 17.5 30.4 7.1 44.7l-256 352c-5.5 7.6-14 12.3-23.4 13.1s-18.5-2.7-25.1-9.3l-128-128c-12.5-12.5-12.5-32.8 0-45.3s32.8-12.5 45.3 0l101.5 101.5 234-321.7c10.4-14.3 30.4-17.5 44.7-7.1z"/></svg>`,
    "chevron-down": `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 448 512"><!--! Font Awesome Free 7.0.0 by @fontawesome - https://fontawesome.com License - https://fontawesome.com/license/free Copyright 2025 Fonticons, Inc. --><path fill="currentColor" d="M201.4 406.6c12.5 12.5 32.8 12.5 45.3 0l192-192c12.5-12.5 12.5-32.8 0-45.3s-32.8-12.5-45.3 0L224 338.7 54.6 169.4c-12.5-12.5-32.8-12.5-45.3 0s-12.5 32.8 0 45.3l192 192z"/></svg>`,
    "chevron-left": `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 320 512"><!--! Font Awesome Free 7.0.0 by @fontawesome - https://fontawesome.com License - https://fontawesome.com/license/free Copyright 2025 Fonticons, Inc. --><path fill="currentColor" d="M9.4 233.4c-12.5 12.5-12.5 32.8 0 45.3l192 192c12.5 12.5 32.8 12.5 45.3 0s12.5-32.8 0-45.3L77.3 256 246.6 86.6c12.5-12.5 12.5-32.8 0-45.3s-32.8-12.5-45.3 0l-192 192z"/></svg>`,
    "chevron-right": `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 320 512"><!--! Font Awesome Free 7.0.0 by @fontawesome - https://fontawesome.com License - https://fontawesome.com/license/free Copyright 2025 Fonticons, Inc. --><path fill="currentColor" d="M311.1 233.4c12.5 12.5 12.5 32.8 0 45.3l-192 192c-12.5 12.5-32.8 12.5-45.3 0s-12.5-32.8 0-45.3L243.2 256 73.9 86.6c-12.5-12.5-12.5-32.8 0-45.3s32.8-12.5 45.3 0l192 192z"/></svg>`,
    circle: `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 512 512"><!--! Font Awesome Free 7.0.0 by @fontawesome - https://fontawesome.com License - https://fontawesome.com/license/free Copyright 2025 Fonticons, Inc. --><path fill="currentColor" d="M0 256a256 256 0 1 1 512 0 256 256 0 1 1 -512 0z"/></svg>`,
    eyedropper: `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 512 512"><!--! Font Awesome Free 7.0.0 by @fontawesome - https://fontawesome.com License - https://fontawesome.com/license/free Copyright 2025 Fonticons, Inc. --><path fill="currentColor" d="M341.6 29.2l-101.6 101.6-9.4-9.4c-12.5-12.5-32.8-12.5-45.3 0s-12.5 32.8 0 45.3l160 160c12.5 12.5 32.8 12.5 45.3 0s12.5-32.8 0-45.3l-9.4-9.4 101.6-101.6c39-39 39-102.2 0-141.1s-102.2-39-141.1 0zM55.4 323.3c-15 15-23.4 35.4-23.4 56.6l0 42.4-26.6 39.9c-8.5 12.7-6.8 29.6 4 40.4s27.7 12.5 40.4 4l39.9-26.6 42.4 0c21.2 0 41.6-8.4 56.6-23.4l109.4-109.4-45.3-45.3-109.4 109.4c-3 3-7.1 4.7-11.3 4.7l-36.1 0 0-36.1c0-4.2 1.7-8.3 4.7-11.3l109.4-109.4-45.3-45.3-109.4 109.4z"/></svg>`,
    file: `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 640 640"><!--!Font Awesome Free 7.1.0 by @fontawesome - https://fontawesome.com License - https://fontawesome.com/license/free Copyright 2026 Fonticons, Inc.--><path fill="currentColor" d="M192 64C156.7 64 128 92.7 128 128L128 512C128 547.3 156.7 576 192 576L448 576C483.3 576 512 547.3 512 512L512 234.5C512 217.5 505.3 201.2 493.3 189.2L386.7 82.7C374.7 70.7 358.5 64 341.5 64L192 64zM453.5 240L360 240C346.7 240 336 229.3 336 216L336 122.5L453.5 240z"/></svg>`,
    "file-audio": `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 640 640"><!--!Font Awesome Free 7.1.0 by @fontawesome - https://fontawesome.com License - https://fontawesome.com/license/free Copyright 2026 Fonticons, Inc.--><path fill="currentColor" d="M128 128C128 92.7 156.7 64 192 64L341.5 64C358.5 64 374.8 70.7 386.8 82.7L493.3 189.3C505.3 201.3 512 217.6 512 234.6L512 512C512 547.3 483.3 576 448 576L192 576C156.7 576 128 547.3 128 512L128 128zM336 122.5L336 216C336 229.3 346.7 240 360 240L453.5 240L336 122.5zM389.8 307.7C380.7 301.4 368.3 303.6 362 312.7C355.7 321.8 357.9 334.2 367 340.5C390.9 357.2 406.4 384.8 406.4 416C406.4 447.2 390.8 474.9 367 491.5C357.9 497.8 355.7 510.3 362 519.3C368.3 528.3 380.8 530.6 389.8 524.3C423.9 500.5 446.4 460.8 446.4 416C446.4 371.2 424 331.5 389.8 307.7zM208 376C199.2 376 192 383.2 192 392L192 440C192 448.8 199.2 456 208 456L232 456L259.2 490C262.2 493.8 266.8 496 271.7 496L272 496C280.8 496 288 488.8 288 480L288 352C288 343.2 280.8 336 272 336L271.7 336C266.8 336 262.2 338.2 259.2 342L232 376L208 376zM336 448.2C336 458.9 346.5 466.4 354.9 459.8C367.8 449.5 376 433.7 376 416C376 398.3 367.8 382.5 354.9 372.2C346.5 365.5 336 373.1 336 383.8L336 448.3z"/></svg>`,
    "file-code": `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 640 640"><!--!Font Awesome Free 7.1.0 by @fontawesome - https://fontawesome.com License - https://fontawesome.com/license/free Copyright 2026 Fonticons, Inc.--><path fill="currentColor" d="M128 128C128 92.7 156.7 64 192 64L341.5 64C358.5 64 374.8 70.7 386.8 82.7L493.3 189.3C505.3 201.3 512 217.6 512 234.6L512 512C512 547.3 483.3 576 448 576L192 576C156.7 576 128 547.3 128 512L128 128zM336 122.5L336 216C336 229.3 346.7 240 360 240L453.5 240L336 122.5zM282.2 359.6C290.8 349.5 289.7 334.4 279.6 325.8C269.5 317.2 254.4 318.3 245.8 328.4L197.8 384.4C190.1 393.4 190.1 406.6 197.8 415.6L245.8 471.6C254.4 481.7 269.6 482.8 279.6 474.2C289.6 465.6 290.8 450.4 282.2 440.4L247.6 400L282.2 359.6zM394.2 328.4C385.6 318.3 370.4 317.2 360.4 325.8C350.4 334.4 349.2 349.6 357.8 359.6L392.4 400L357.8 440.4C349.2 450.5 350.3 465.6 360.4 474.2C370.5 482.8 385.6 481.7 394.2 471.6L442.2 415.6C449.9 406.6 449.9 393.4 442.2 384.4L394.2 328.4z"/></svg>`,
    "file-excel": `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 640 640"><!--!Font Awesome Free 7.1.0 by @fontawesome - https://fontawesome.com License - https://fontawesome.com/license/free Copyright 2026 Fonticons, Inc.--><path fill="currentColor" d="M128 128C128 92.7 156.7 64 192 64L341.5 64C358.5 64 374.8 70.7 386.8 82.7L493.3 189.3C505.3 201.3 512 217.6 512 234.6L512 512C512 547.3 483.3 576 448 576L192 576C156.7 576 128 547.3 128 512L128 128zM336 122.5L336 216C336 229.3 346.7 240 360 240L453.5 240L336 122.5zM292 330.7C284.6 319.7 269.7 316.7 258.7 324C247.7 331.3 244.7 346.3 252 357.3L291.2 416L252 474.7C244.6 485.7 247.6 500.6 258.7 508C269.8 515.4 284.6 512.4 292 501.3L320 459.3L348 501.3C355.4 512.3 370.3 515.3 381.3 508C392.3 500.7 395.3 485.7 388 474.7L348.8 416L388 357.3C395.4 346.3 392.4 331.4 381.3 324C370.2 316.6 355.4 319.6 348 330.7L320 372.7L292 330.7z"/></svg>`,
    "file-image": `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 640 640"><!--!Font Awesome Free 7.1.0 by @fontawesome - https://fontawesome.com License - https://fontawesome.com/license/free Copyright 2026 Fonticons, Inc.--><path fill="currentColor" d="M128 128C128 92.7 156.7 64 192 64L341.5 64C358.5 64 374.8 70.7 386.8 82.7L493.3 189.3C505.3 201.3 512 217.6 512 234.6L512 512C512 547.3 483.3 576 448 576L192 576C156.7 576 128 547.3 128 512L128 128zM336 122.5L336 216C336 229.3 346.7 240 360 240L453.5 240L336 122.5zM256 320C256 302.3 241.7 288 224 288C206.3 288 192 302.3 192 320C192 337.7 206.3 352 224 352C241.7 352 256 337.7 256 320zM220.6 512L419.4 512C435.2 512 448 499.2 448 483.4C448 476.1 445.2 469 440.1 463.7L343.3 361.9C337.3 355.6 328.9 352 320.1 352L319.8 352C311 352 302.7 355.6 296.6 361.9L199.9 463.7C194.8 469 192 476.1 192 483.4C192 499.2 204.8 512 220.6 512z"/></svg>`,
    "file-pdf": `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 640 640"><!--!Font Awesome Free 7.1.0 by @fontawesome - https://fontawesome.com License - https://fontawesome.com/license/free Copyright 2026 Fonticons, Inc.--><path fill="currentColor" d="M128 64C92.7 64 64 92.7 64 128L64 512C64 547.3 92.7 576 128 576L208 576L208 464C208 428.7 236.7 400 272 400L448 400L448 234.5C448 217.5 441.3 201.2 429.3 189.2L322.7 82.7C310.7 70.7 294.5 64 277.5 64L128 64zM389.5 240L296 240C282.7 240 272 229.3 272 216L272 122.5L389.5 240zM272 444C261 444 252 453 252 464L252 592C252 603 261 612 272 612C283 612 292 603 292 592L292 564L304 564C337.1 564 364 537.1 364 504C364 470.9 337.1 444 304 444L272 444zM304 524L292 524L292 484L304 484C315 484 324 493 324 504C324 515 315 524 304 524zM400 444C389 444 380 453 380 464L380 592C380 603 389 612 400 612L432 612C460.7 612 484 588.7 484 560L484 496C484 467.3 460.7 444 432 444L400 444zM420 572L420 484L432 484C438.6 484 444 489.4 444 496L444 560C444 566.6 438.6 572 432 572L420 572zM508 464L508 592C508 603 517 612 528 612C539 612 548 603 548 592L548 548L576 548C587 548 596 539 596 528C596 517 587 508 576 508L548 508L548 484L576 484C587 484 596 475 596 464C596 453 587 444 576 444L528 444C517 444 508 453 508 464z"/></svg>`,
    "file-powerpoint": `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 640 640"><!--!Font Awesome Free 7.1.0 by @fontawesome - https://fontawesome.com License - https://fontawesome.com/license/free Copyright 2026 Fonticons, Inc.--><path fill="currentColor" d="M128 128C128 92.7 156.7 64 192 64L341.5 64C358.5 64 374.8 70.7 386.8 82.7L493.3 189.3C505.3 201.3 512 217.6 512 234.6L512 512C512 547.3 483.3 576 448 576L192 576C156.7 576 128 547.3 128 512L128 128zM336 122.5L336 216C336 229.3 346.7 240 360 240L453.5 240L336 122.5zM280 320C266.7 320 256 330.7 256 344L256 488C256 501.3 266.7 512 280 512C293.3 512 304 501.3 304 488L304 464L328 464C367.8 464 400 431.8 400 392C400 352.2 367.8 320 328 320L280 320zM328 416L304 416L304 368L328 368C341.3 368 352 378.7 352 392C352 405.3 341.3 416 328 416z"/></svg>`,
    "file-video": `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 640 640"><!--!Font Awesome Free 7.1.0 by @fontawesome - https://fontawesome.com License - https://fontawesome.com/license/free Copyright 2026 Fonticons, Inc.--><path fill="currentColor" d="M128 128C128 92.7 156.7 64 192 64L341.5 64C358.5 64 374.8 70.7 386.8 82.7L493.3 189.3C505.3 201.3 512 217.6 512 234.6L512 512C512 547.3 483.3 576 448 576L192 576C156.7 576 128 547.3 128 512L128 128zM336 122.5L336 216C336 229.3 346.7 240 360 240L453.5 240L336 122.5zM208 368L208 464C208 481.7 222.3 496 240 496L336 496C353.7 496 368 481.7 368 464L368 440L403 475C406.2 478.2 410.5 480 415 480C424.4 480 432 472.4 432 463L432 368.9C432 359.5 424.4 351.9 415 351.9C410.5 351.9 406.2 353.7 403 356.9L368 391.9L368 367.9C368 350.2 353.7 335.9 336 335.9L240 335.9C222.3 335.9 208 350.2 208 367.9z"/></svg>`,
    "file-word": `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 640 640"><!--!Font Awesome Free 7.1.0 by @fontawesome - https://fontawesome.com License - https://fontawesome.com/license/free Copyright 2026 Fonticons, Inc.--><path fill="currentColor" d="M128 128C128 92.7 156.7 64 192 64L341.5 64C358.5 64 374.8 70.7 386.8 82.7L493.3 189.3C505.3 201.3 512 217.6 512 234.6L512 512C512 547.3 483.3 576 448 576L192 576C156.7 576 128 547.3 128 512L128 128zM336 122.5L336 216C336 229.3 346.7 240 360 240L453.5 240L336 122.5zM263.4 338.8C260.5 325.9 247.7 317.7 234.8 320.6C221.9 323.5 213.7 336.3 216.6 349.2L248.6 493.2C250.9 503.7 260 511.4 270.8 512C281.6 512.6 291.4 505.9 294.8 495.6L320 419.9L345.2 495.6C348.6 505.8 358.4 512.5 369.2 512C380 511.5 389.1 503.8 391.4 493.2L423.4 349.2C426.3 336.3 418.1 323.4 405.2 320.6C392.3 317.8 379.4 325.9 376.6 338.8L363.4 398.2L342.8 336.4C339.5 326.6 330.4 320 320 320C309.6 320 300.5 326.6 297.2 336.4L276.6 398.2L263.4 338.8z"/></svg>`,
    "file-zipper": `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 640 640"><!--!Font Awesome Free 7.1.0 by @fontawesome - https://fontawesome.com License - https://fontawesome.com/license/free Copyright 2026 Fonticons, Inc.--><path fill="currentColor" d="M128 128C128 92.7 156.7 64 192 64L341.5 64C358.5 64 374.8 70.7 386.8 82.7L493.3 189.3C505.3 201.3 512 217.6 512 234.6L512 512C512 547.3 483.3 576 448 576L192 576C156.7 576 128 547.3 128 512L128 128zM336 122.5L336 216C336 229.3 346.7 240 360 240L453.5 240L336 122.5zM192 136C192 149.3 202.7 160 216 160L264 160C277.3 160 288 149.3 288 136C288 122.7 277.3 112 264 112L216 112C202.7 112 192 122.7 192 136zM192 232C192 245.3 202.7 256 216 256L264 256C277.3 256 288 245.3 288 232C288 218.7 277.3 208 264 208L216 208C202.7 208 192 218.7 192 232zM256 304L224 304C206.3 304 192 318.3 192 336L192 384C192 410.5 213.5 432 240 432C266.5 432 288 410.5 288 384L288 336C288 318.3 273.7 304 256 304zM240 368C248.8 368 256 375.2 256 384C256 392.8 248.8 400 240 400C231.2 400 224 392.8 224 384C224 375.2 231.2 368 240 368z"/></svg>`,
    "grip-vertical": `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 320 512"><!--! Font Awesome Free 7.0.0 by @fontawesome - https://fontawesome.com License - https://fontawesome.com/license/free Copyright 2025 Fonticons, Inc. --><path fill="currentColor" d="M128 40c0-22.1-17.9-40-40-40L40 0C17.9 0 0 17.9 0 40L0 88c0 22.1 17.9 40 40 40l48 0c22.1 0 40-17.9 40-40l0-48zm0 192c0-22.1-17.9-40-40-40l-48 0c-22.1 0-40 17.9-40 40l0 48c0 22.1 17.9 40 40 40l48 0c22.1 0 40-17.9 40-40l0-48zM0 424l0 48c0 22.1 17.9 40 40 40l48 0c22.1 0 40-17.9 40-40l0-48c0-22.1-17.9-40-40-40l-48 0c-22.1 0-40 17.9-40 40zM320 40c0-22.1-17.9-40-40-40L232 0c-22.1 0-40 17.9-40 40l0 48c0 22.1 17.9 40 40 40l48 0c22.1 0 40-17.9 40-40l0-48zM192 232l0 48c0 22.1 17.9 40 40 40l48 0c22.1 0 40-17.9 40-40l0-48c0-22.1-17.9-40-40-40l-48 0c-22.1 0-40 17.9-40 40zM320 424c0-22.1-17.9-40-40-40l-48 0c-22.1 0-40 17.9-40 40l0 48c0 22.1 17.9 40 40 40l48 0c22.1 0 40-17.9 40-40l0-48z"/></svg>`,
    indeterminate: `<svg part="indeterminate-icon" class="icon" viewBox="0 0 16 16"><g stroke="none" stroke-width="1" fill="none" fill-rule="evenodd" stroke-linecap="round"><g stroke="currentColor" stroke-width="2"><g transform="translate(2.285714 6.857143)"><path d="M10.2857143,1.14285714 L1.14285714,1.14285714"/></g></g></g></svg>`,
    minus: `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 448 512"><!--! Font Awesome Free 7.0.0 by @fontawesome - https://fontawesome.com License - https://fontawesome.com/license/free Copyright 2025 Fonticons, Inc. --><path fill="currentColor" d="M0 256c0-17.7 14.3-32 32-32l384 0c17.7 0 32 14.3 32 32s-14.3 32-32 32L32 288c-17.7 0-32-14.3-32-32z"/></svg>`,
    pause: `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 384 512"><!--! Font Awesome Free 7.0.0 by @fontawesome - https://fontawesome.com License - https://fontawesome.com/license/free Copyright 2025 Fonticons, Inc. --><path fill="currentColor" d="M48 32C21.5 32 0 53.5 0 80L0 432c0 26.5 21.5 48 48 48l64 0c26.5 0 48-21.5 48-48l0-352c0-26.5-21.5-48-48-48L48 32zm224 0c-26.5 0-48 21.5-48 48l0 352c0 26.5 21.5 48 48 48l64 0c26.5 0 48-21.5 48-48l0-352c0-26.5-21.5-48-48-48l-64 0z"/></svg>`,
    play: `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 448 512"><!--! Font Awesome Free 7.0.0 by @fontawesome - https://fontawesome.com License - https://fontawesome.com/license/free Copyright 2025 Fonticons, Inc. --><path fill="currentColor" d="M91.2 36.9c-12.4-6.8-27.4-6.5-39.6 .7S32 57.9 32 72l0 368c0 14.1 7.5 27.2 19.6 34.4s27.2 7.5 39.6 .7l336-184c12.8-7 20.8-20.5 20.8-35.1s-8-28.1-20.8-35.1l-336-184z"/></svg>`,
    plus: `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 640 640"><!--!Font Awesome Free 7.1.0 by @fontawesome - https://fontawesome.com License - https://fontawesome.com/license/free Copyright 2026 Fonticons, Inc.--><path fill="currentColor" d="M352 128C352 110.3 337.7 96 320 96C302.3 96 288 110.3 288 128L288 288L128 288C110.3 288 96 302.3 96 320C96 337.7 110.3 352 128 352L288 352L288 512C288 529.7 302.3 544 320 544C337.7 544 352 529.7 352 512L352 352L512 352C529.7 352 544 337.7 544 320C544 302.3 529.7 288 512 288L352 288L352 128z"/></svg>`,
    star: `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 576 512"><!--! Font Awesome Free 7.0.0 by @fontawesome - https://fontawesome.com License - https://fontawesome.com/license/free Copyright 2025 Fonticons, Inc. --><path fill="currentColor" d="M309.5-18.9c-4.1-8-12.4-13.1-21.4-13.1s-17.3 5.1-21.4 13.1L193.1 125.3 33.2 150.7c-8.9 1.4-16.3 7.7-19.1 16.3s-.5 18 5.8 24.4l114.4 114.5-25.2 159.9c-1.4 8.9 2.3 17.9 9.6 23.2s16.9 6.1 25 2L288.1 417.6 432.4 491c8 4.1 17.7 3.3 25-2s11-14.2 9.6-23.2L441.7 305.9 556.1 191.4c6.4-6.4 8.6-15.8 5.8-24.4s-10.1-14.9-19.1-16.3L383 125.3 309.5-18.9z"/></svg>`,
    upload: `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 640 640"><!--!Font Awesome Free 7.1.0 by @fontawesome - https://fontawesome.com License - https://fontawesome.com/license/free Copyright 2026 Fonticons, Inc.--><path fill="currentColor" d="M352 173.3L352 384C352 401.7 337.7 416 320 416C302.3 416 288 401.7 288 384L288 173.3L246.6 214.7C234.1 227.2 213.8 227.2 201.3 214.7C188.8 202.2 188.8 181.9 201.3 169.4L297.3 73.4C309.8 60.9 330.1 60.9 342.6 73.4L438.6 169.4C451.1 181.9 451.1 202.2 438.6 214.7C426.1 227.2 405.8 227.2 393.3 214.7L352 173.3zM320 464C364.2 464 400 428.2 400 384L480 384C515.3 384 544 412.7 544 448L544 480C544 515.3 515.3 544 480 544L160 544C124.7 544 96 515.3 96 480L96 448C96 412.7 124.7 384 160 384L240 384C240 428.2 275.8 464 320 464zM464 488C477.3 488 488 477.3 488 464C488 450.7 477.3 440 464 440C450.7 440 440 450.7 440 464C440 477.3 450.7 488 464 488z"/></svg>`,
    user: `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 448 512"><!--! Font Awesome Free 7.0.0 by @fontawesome - https://fontawesome.com License - https://fontawesome.com/license/free Copyright 2025 Fonticons, Inc. --><path fill="currentColor" d="M224 248a120 120 0 1 0 0-240 120 120 0 1 0 0 240zm-29.7 56C95.8 304 16 383.8 16 482.3 16 498.7 29.3 512 45.7 512l356.6 0c16.4 0 29.7-13.3 29.7-29.7 0-98.5-79.8-178.3-178.3-178.3l-59.4 0z"/></svg>`,
    xmark: `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 384 512"><!--! Font Awesome Free 7.0.0 by @fontawesome - https://fontawesome.com License - https://fontawesome.com/license/free Copyright 2025 Fonticons, Inc. --><path fill="currentColor" d="M55.1 73.4c-12.5-12.5-32.8-12.5-45.3 0s-12.5 32.8 0 45.3L147.2 256 9.9 393.4c-12.5 12.5-12.5 32.8 0 45.3s32.8 12.5 45.3 0L192.5 301.3 329.9 438.6c12.5 12.5 32.8 12.5 45.3 0s12.5-32.8 0-45.3L237.8 256 375.1 118.6c12.5-12.5 12.5-32.8 0-45.3s-32.8-12.5-45.3 0L192.5 210.7 55.1 73.4z"/></svg>`
  },
  regular: {
    "circle-question": `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 512 512"><!--! Font Awesome Free 7.0.0 by @fontawesome - https://fontawesome.com License - https://fontawesome.com/license/free Copyright 2025 Fonticons, Inc. --><path fill="currentColor" d="M464 256a208 208 0 1 0 -416 0 208 208 0 1 0 416 0zM0 256a256 256 0 1 1 512 0 256 256 0 1 1 -512 0zm256-80c-17.7 0-32 14.3-32 32 0 13.3-10.7 24-24 24s-24-10.7-24-24c0-44.2 35.8-80 80-80s80 35.8 80 80c0 47.2-36 67.2-56 74.5l0 3.8c0 13.3-10.7 24-24 24s-24-10.7-24-24l0-8.1c0-20.5 14.8-35.2 30.1-40.2 6.4-2.1 13.2-5.5 18.2-10.3 4.3-4.2 7.7-10 7.7-19.6 0-17.7-14.3-32-32-32zM224 368a32 32 0 1 1 64 0 32 32 0 1 1 -64 0z"/></svg>`,
    "circle-xmark": `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 512 512"><!--! Font Awesome Free 7.0.0 by @fontawesome - https://fontawesome.com License - https://fontawesome.com/license/free Copyright 2025 Fonticons, Inc. --><path fill="currentColor" d="M256 48a208 208 0 1 1 0 416 208 208 0 1 1 0-416zm0 464a256 256 0 1 0 0-512 256 256 0 1 0 0 512zM167 167c-9.4 9.4-9.4 24.6 0 33.9l55 55-55 55c-9.4 9.4-9.4 24.6 0 33.9s24.6 9.4 33.9 0l55-55 55 55c9.4 9.4 24.6 9.4 33.9 0s9.4-24.6 0-33.9l-55-55 55-55c9.4-9.4 9.4-24.6 0-33.9s-24.6-9.4-33.9 0l-55 55-55-55c-9.4-9.4-24.6-9.4-33.9 0z"/></svg>`,
    copy: `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 448 512"><!--! Font Awesome Free 7.0.0 by @fontawesome - https://fontawesome.com License - https://fontawesome.com/license/free Copyright 2025 Fonticons, Inc. --><path fill="currentColor" d="M384 336l-192 0c-8.8 0-16-7.2-16-16l0-256c0-8.8 7.2-16 16-16l133.5 0c4.2 0 8.3 1.7 11.3 4.7l58.5 58.5c3 3 4.7 7.1 4.7 11.3L400 320c0 8.8-7.2 16-16 16zM192 384l192 0c35.3 0 64-28.7 64-64l0-197.5c0-17-6.7-33.3-18.7-45.3L370.7 18.7C358.7 6.7 342.5 0 325.5 0L192 0c-35.3 0-64 28.7-64 64l0 256c0 35.3 28.7 64 64 64zM64 128c-35.3 0-64 28.7-64 64L0 448c0 35.3 28.7 64 64 64l192 0c35.3 0 64-28.7 64-64l0-16-48 0 0 16c0 8.8-7.2 16-16 16L64 464c-8.8 0-16-7.2-16-16l0-256c0-8.8 7.2-16 16-16l16 0 0-48-16 0z"/></svg>`,
    eye: `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 576 512"><!--! Font Awesome Free 7.0.0 by @fontawesome - https://fontawesome.com License - https://fontawesome.com/license/free Copyright 2025 Fonticons, Inc. --><path fill="currentColor" d="M288 80C222.8 80 169.2 109.6 128.1 147.7 89.6 183.5 63 226 49.4 256 63 286 89.6 328.5 128.1 364.3 169.2 402.4 222.8 432 288 432s118.8-29.6 159.9-67.7C486.4 328.5 513 286 526.6 256 513 226 486.4 183.5 447.9 147.7 406.8 109.6 353.2 80 288 80zM95.4 112.6C142.5 68.8 207.2 32 288 32s145.5 36.8 192.6 80.6c46.8 43.5 78.1 95.4 93 131.1 3.3 7.9 3.3 16.7 0 24.6-14.9 35.7-46.2 87.7-93 131.1-47.1 43.7-111.8 80.6-192.6 80.6S142.5 443.2 95.4 399.4c-46.8-43.5-78.1-95.4-93-131.1-3.3-7.9-3.3-16.7 0-24.6 14.9-35.7 46.2-87.7 93-131.1zM288 336c44.2 0 80-35.8 80-80 0-29.6-16.1-55.5-40-69.3-1.4 59.7-49.6 107.9-109.3 109.3 13.8 23.9 39.7 40 69.3 40zm-79.6-88.4c2.5 .3 5 .4 7.6 .4 35.3 0 64-28.7 64-64 0-2.6-.2-5.1-.4-7.6-37.4 3.9-67.2 33.7-71.1 71.1zm45.6-115c10.8-3 22.2-4.5 33.9-4.5 8.8 0 17.5 .9 25.8 2.6 .3 .1 .5 .1 .8 .2 57.9 12.2 101.4 63.7 101.4 125.2 0 70.7-57.3 128-128 128-61.6 0-113-43.5-125.2-101.4-1.8-8.6-2.8-17.5-2.8-26.6 0-11 1.4-21.8 4-32 .2-.7 .3-1.3 .5-1.9 11.9-43.4 46.1-77.6 89.5-89.5z"/></svg>`,
    "eye-slash": `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 576 512"><!--! Font Awesome Free 7.0.0 by @fontawesome - https://fontawesome.com License - https://fontawesome.com/license/free Copyright 2025 Fonticons, Inc. --><path fill="currentColor" d="M41-24.9c-9.4-9.4-24.6-9.4-33.9 0S-2.3-.3 7 9.1l528 528c9.4 9.4 24.6 9.4 33.9 0s9.4-24.6 0-33.9l-96.4-96.4c2.7-2.4 5.4-4.8 8-7.2 46.8-43.5 78.1-95.4 93-131.1 3.3-7.9 3.3-16.7 0-24.6-14.9-35.7-46.2-87.7-93-131.1-47.1-43.7-111.8-80.6-192.6-80.6-56.8 0-105.6 18.2-146 44.2L41-24.9zM176.9 111.1c32.1-18.9 69.2-31.1 111.1-31.1 65.2 0 118.8 29.6 159.9 67.7 38.5 35.7 65.1 78.3 78.6 108.3-13.6 30-40.2 72.5-78.6 108.3-3.1 2.8-6.2 5.6-9.4 8.4L393.8 328c14-20.5 22.2-45.3 22.2-72 0-70.7-57.3-128-128-128-26.7 0-51.5 8.2-72 22.2l-39.1-39.1zm182 182l-108-108c11.1-5.8 23.7-9.1 37.1-9.1 44.2 0 80 35.8 80 80 0 13.4-3.3 26-9.1 37.1zM103.4 173.2l-34-34c-32.6 36.8-55 75.8-66.9 104.5-3.3 7.9-3.3 16.7 0 24.6 14.9 35.7 46.2 87.7 93 131.1 47.1 43.7 111.8 80.6 192.6 80.6 37.3 0 71.2-7.9 101.5-20.6L352.2 422c-20 6.4-41.4 10-64.2 10-65.2 0-118.8-29.6-159.9-67.7-38.5-35.7-65.1-78.3-78.6-108.3 10.4-23.1 28.6-53.6 54-82.8z"/></svg>`,
    star: `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 576 512"><!--! Font Awesome Free 7.0.0 by @fontawesome - https://fontawesome.com License - https://fontawesome.com/license/free Copyright 2025 Fonticons, Inc. --><path fill="currentColor" d="M288.1-32c9 0 17.3 5.1 21.4 13.1L383 125.3 542.9 150.7c8.9 1.4 16.3 7.7 19.1 16.3s.5 18-5.8 24.4L441.7 305.9 467 465.8c1.4 8.9-2.3 17.9-9.6 23.2s-17 6.1-25 2L288.1 417.6 143.8 491c-8 4.1-17.7 3.3-25-2s-11-14.2-9.6-23.2L134.4 305.9 20 191.4c-6.4-6.4-8.6-15.8-5.8-24.4s10.1-14.9 19.1-16.3l159.9-25.4 73.6-144.2c4.1-8 12.4-13.1 21.4-13.1zm0 76.8L230.3 158c-3.5 6.8-10 11.6-17.6 12.8l-125.5 20 89.8 89.9c5.4 5.4 7.9 13.1 6.7 20.7l-19.8 125.5 113.3-57.6c6.8-3.5 14.9-3.5 21.8 0l113.3 57.6-19.8-125.5c-1.2-7.6 1.3-15.3 6.7-20.7l89.8-89.9-125.5-20c-7.6-1.2-14.1-6-17.6-12.8L288.1 44.8z"/></svg>`
  }
};
var systemLibrary = {
  name: "system",
  resolver: (name, _family = "classic", variant = "solid") => {
    let collection = icons[variant];
    let svg = collection[name] ?? icons.regular[name] ?? icons.regular["circle-question"];
    if (svg) {
      return dataUri(svg);
    }
    return "";
  }
};
var library_system_default = systemLibrary;

// chunks/chunk.HGBRCPUS.js
var iconPath = "";
var kitCode = "";
function getIconPath() {
  return iconPath.replace(/\/$/, "");
}
function setKitCode(code) {
  kitCode = code;
}
function getKitCode() {
  if (!kitCode) {
    const el = document.querySelector("[data-fa-kit-code]");
    if (el) {
      setKitCode(el.getAttribute("data-fa-kit-code") || "");
    }
  }
  return kitCode;
}

// chunks/chunk.HCXBOJYW.js
var FA_VERSION = "7.2.0";
function getIconFolder(_name, family, variant) {
  let folder = "solid";
  if (family === "chisel") {
    folder = "chisel-regular";
  }
  if (family === "etch") {
    folder = "etch-solid";
  }
  if (family === "graphite") {
    folder = "graphite-thin";
  }
  if (family === "jelly") {
    folder = "jelly-regular";
    if (variant === "duo-regular")
      folder = "jelly-duo-regular";
    if (variant === "fill-regular")
      folder = "jelly-fill-regular";
  }
  if (family === "jelly-duo") {
    folder = "jelly-duo-regular";
  }
  if (family === "jelly-fill") {
    folder = "jelly-fill-regular";
  }
  if (family === "notdog") {
    if (variant === "solid")
      folder = "notdog-solid";
    if (variant === "duo-solid")
      folder = "notdog-duo-solid";
  }
  if (family === "notdog-duo") {
    folder = "notdog-duo-solid";
  }
  if (family === "slab") {
    if (variant === "solid" || variant === "regular")
      folder = "slab-regular";
    if (variant === "press-regular")
      folder = "slab-press-regular";
  }
  if (family === "slab-press") {
    folder = "slab-press-regular";
  }
  if (family === "thumbprint") {
    folder = "thumbprint-light";
  }
  if (family === "utility") {
    folder = "utility-semibold";
  }
  if (family === "utility-duo") {
    folder = "utility-duo-semibold";
  }
  if (family === "utility-fill") {
    folder = "utility-fill-semibold";
  }
  if (family === "whiteboard") {
    folder = "whiteboard-semibold";
  }
  if (family === "classic") {
    if (variant === "thin")
      folder = "thin";
    if (variant === "light")
      folder = "light";
    if (variant === "regular")
      folder = "regular";
    if (variant === "solid")
      folder = "solid";
  }
  if (family === "duotone") {
    if (variant === "thin")
      folder = "duotone-thin";
    if (variant === "light")
      folder = "duotone-light";
    if (variant === "regular")
      folder = "duotone-regular";
    if (variant === "solid")
      folder = "duotone";
  }
  if (family === "sharp") {
    if (variant === "thin")
      folder = "sharp-thin";
    if (variant === "light")
      folder = "sharp-light";
    if (variant === "regular")
      folder = "sharp-regular";
    if (variant === "solid")
      folder = "sharp-solid";
  }
  if (family === "sharp-duotone") {
    if (variant === "thin")
      folder = "sharp-duotone-thin";
    if (variant === "light")
      folder = "sharp-duotone-light";
    if (variant === "regular")
      folder = "sharp-duotone-regular";
    if (variant === "solid")
      folder = "sharp-duotone-solid";
  }
  if (family === "brands") {
    folder = "brands";
  }
  return folder;
}
function getIconUrl(name, family, variant) {
  const folder = getIconFolder(name, family, variant);
  const iconBase = getIconPath();
  if (iconBase) {
    return `${iconBase}/${folder}/${name}.svg`;
  }
  const kitCode2 = getKitCode();
  const isPro = kitCode2.length > 0;
  return isPro ? `https://ka-p.fontawesome.com/releases/v${FA_VERSION}/svgs/${folder}/${name}.svg?token=${encodeURIComponent(kitCode2)}` : `https://ka-f.fontawesome.com/releases/v${FA_VERSION}/svgs/${folder}/${name}.svg`;
}
var library = {
  name: "default",
  resolver: (name, family = "classic", variant = "solid") => {
    return getIconUrl(name, family, variant);
  },
  mutator: (svg, hostEl) => {
    if (hostEl?.family && !svg.hasAttribute("data-duotone-initialized")) {
      const { family, variant } = hostEl;
      if (family === "duotone" || family === "sharp-duotone" || family === "notdog-duo" || family === "notdog" && variant === "duo-solid" || family === "jelly-duo" || family === "jelly" && variant === "duo-regular" || family === "utility-duo" || family === "thumbprint") {
        const paths = [...svg.querySelectorAll("path")];
        const primaryPath = paths.find((p3) => !p3.hasAttribute("opacity"));
        const secondaryPath = paths.find((p3) => p3.hasAttribute("opacity"));
        if (!primaryPath || !secondaryPath)
          return;
        primaryPath.setAttribute("data-duotone-primary", "");
        secondaryPath.setAttribute("data-duotone-secondary", "");
        if (hostEl.swapOpacity && primaryPath && secondaryPath) {
          const originalOpacity = secondaryPath.getAttribute("opacity") || "0.4";
          primaryPath.style.setProperty("--path-opacity", originalOpacity);
          secondaryPath.style.setProperty("--path-opacity", "1");
        }
        svg.setAttribute("data-duotone-initialized", "");
      }
    }
  }
};
var library_default_default = library;

// chunks/chunk.S6RZOJNM.js
var defaultIconFamily = "classic";
var registry = [library_default_default, library_system_default];
var watchedIcons = [];
function watchIcon(icon) {
  watchedIcons.push(icon);
}
function unwatchIcon(icon) {
  watchedIcons = watchedIcons.filter((el) => el !== icon);
}
function getIconLibrary(name) {
  return registry.find((lib) => lib.name === name);
}
function getDefaultIconFamily() {
  return defaultIconFamily;
}

// chunks/chunk.GNPK466U.js
var CACHEABLE_ERROR = Symbol();
var RETRYABLE_ERROR = Symbol();
var parser;
var iconCache = /* @__PURE__ */ new Map();
var WaIcon = class extends WebAwesomeElement {
  constructor() {
    super(...arguments);
    this.svg = null;
    this.autoWidth = false;
    this.swapOpacity = false;
    this.label = "";
    this.library = "default";
    this.rotate = 0;
    this.resolveIcon = async (url, library2) => {
      let fileData;
      if (library2?.spriteSheet) {
        if (!this.hasUpdated) {
          await this.updateComplete;
        }
        this.svg = x`<svg part="svg">
        <use part="use" href="${url}"></use>
      </svg>`;
        await this.updateComplete;
        const svg = this.shadowRoot.querySelector("[part='svg']");
        if (typeof library2.mutator === "function") {
          library2.mutator(svg, this);
        }
        return this.svg;
      }
      try {
        fileData = await fetch(url, { mode: "cors" });
        if (!fileData.ok)
          return fileData.status === 410 ? CACHEABLE_ERROR : RETRYABLE_ERROR;
      } catch {
        return RETRYABLE_ERROR;
      }
      try {
        const div = document.createElement("div");
        div.innerHTML = await fileData.text();
        const svg = div.firstElementChild;
        if (svg?.tagName?.toLowerCase() !== "svg")
          return CACHEABLE_ERROR;
        if (!parser)
          parser = new DOMParser();
        const doc = parser.parseFromString(svg.outerHTML, "text/html");
        const svgEl = doc.body.querySelector("svg");
        if (!svgEl)
          return CACHEABLE_ERROR;
        svgEl.part.add("svg");
        return document.adoptNode(svgEl);
      } catch {
        return CACHEABLE_ERROR;
      }
    };
  }
  connectedCallback() {
    super.connectedCallback();
    watchIcon(this);
  }
  firstUpdated(changedProperties) {
    super.firstUpdated(changedProperties);
    if (this.hasAttribute("rotate")) {
      this.style.setProperty("--rotate-angle", `${this.rotate}deg`);
    }
    this.setIcon();
  }
  disconnectedCallback() {
    super.disconnectedCallback();
    unwatchIcon(this);
  }
  async getIconSource() {
    const library2 = getIconLibrary(this.library);
    const family = this.family || getDefaultIconFamily();
    if (this.name && library2) {
      let url;
      try {
        url = await library2.resolver(this.name, family, this.variant, this.autoWidth);
      } catch {
        url = void 0;
      }
      return { url, fromLibrary: true };
    }
    return {
      url: this.src,
      fromLibrary: false
    };
  }
  handleLabelChange() {
    const hasLabel = typeof this.label === "string" && this.label.length > 0;
    if (hasLabel) {
      this.setAttribute("role", "img");
      this.setAttribute("aria-label", this.label);
      this.removeAttribute("aria-hidden");
    } else {
      this.removeAttribute("role");
      this.removeAttribute("aria-label");
      this.setAttribute("aria-hidden", "true");
    }
  }
  async setIcon() {
    const { url, fromLibrary } = await this.getIconSource();
    const library2 = fromLibrary ? getIconLibrary(this.library) : void 0;
    if (!url) {
      this.svg = null;
      return;
    }
    let iconResolver = iconCache.get(url);
    if (!iconResolver) {
      iconResolver = this.resolveIcon(url, library2);
      iconCache.set(url, iconResolver);
    }
    const svg = await iconResolver;
    if (svg === RETRYABLE_ERROR) {
      iconCache.delete(url);
    }
    const sourceAfterFetch = await this.getIconSource();
    if (url !== sourceAfterFetch.url) {
      return;
    }
    if (e5(svg)) {
      this.svg = svg;
      return;
    }
    switch (svg) {
      case RETRYABLE_ERROR:
      case CACHEABLE_ERROR:
        this.svg = null;
        this.dispatchEvent(new WaErrorEvent());
        break;
      default:
        this.svg = svg.cloneNode(true);
        library2?.mutator?.(this.svg, this);
        this.dispatchEvent(new WaLoadEvent());
    }
  }
  updated(changedProperties) {
    super.updated(changedProperties);
    const library2 = getIconLibrary(this.library);
    if (this.hasAttribute("rotate")) {
      this.style.setProperty("--rotate-angle", `${this.rotate}deg`);
    }
    const svg = this.shadowRoot?.querySelector("svg");
    if (svg) {
      library2?.mutator?.(svg, this);
    }
  }
  render() {
    if (this.hasUpdated) {
      return this.svg;
    }
    return x`<svg part="svg" width="16" height="16"></svg>`;
  }
};
WaIcon.css = icon_styles_default;
__decorateClass([
  r23()
], WaIcon.prototype, "svg", 2);
__decorateClass([
  n3({ reflect: true })
], WaIcon.prototype, "name", 2);
__decorateClass([
  n3({ reflect: true })
], WaIcon.prototype, "family", 2);
__decorateClass([
  n3({ reflect: true })
], WaIcon.prototype, "variant", 2);
__decorateClass([
  n3({ attribute: "auto-width", type: Boolean, reflect: true })
], WaIcon.prototype, "autoWidth", 2);
__decorateClass([
  n3({ attribute: "swap-opacity", type: Boolean, reflect: true })
], WaIcon.prototype, "swapOpacity", 2);
__decorateClass([
  n3()
], WaIcon.prototype, "src", 2);
__decorateClass([
  n3()
], WaIcon.prototype, "label", 2);
__decorateClass([
  n3({ reflect: true })
], WaIcon.prototype, "library", 2);
__decorateClass([
  n3({ type: Number, reflect: true })
], WaIcon.prototype, "rotate", 2);
__decorateClass([
  n3({ type: String, reflect: true })
], WaIcon.prototype, "flip", 2);
__decorateClass([
  n3({ type: String, reflect: true })
], WaIcon.prototype, "animation", 2);
__decorateClass([
  watch("label")
], WaIcon.prototype, "handleLabelChange", 1);
__decorateClass([
  watch(["family", "name", "library", "variant", "src", "autoWidth", "swapOpacity"], { waitUntilFirstUpdate: true })
], WaIcon.prototype, "setIcon", 1);
WaIcon = __decorateClass([
  t3("wa-icon")
], WaIcon);

// chunks/chunk.JRPGUCID.js
var callout_styles_default = i2`
  :host {
    display: flex;
    position: relative;
    align-items: stretch;
    border-radius: var(--wa-panel-border-radius);
    background-color: var(--wa-color-fill-quiet, var(--wa-color-brand-fill-quiet));
    border-color: var(--wa-color-border-quiet, var(--wa-color-brand-border-quiet));
    border-style: var(--wa-panel-border-style);
    border-width: var(--wa-panel-border-width);
    color: var(--wa-color-text-normal);
    padding: 1em;
  }

  /* Appearance modifiers */
  :host([appearance~='plain']) {
    background-color: transparent;
    border-color: transparent;
  }

  :host([appearance~='outlined']) {
    background-color: transparent;
    border-color: var(--wa-color-border-loud, var(--wa-color-brand-border-loud));
  }

  :host([appearance~='filled']) {
    background-color: var(--wa-color-fill-quiet, var(--wa-color-brand-fill-quiet));
    border-color: transparent;
  }

  :host([appearance~='filled-outlined']) {
    border-color: var(--wa-color-border-quiet, var(--wa-color-brand-border-quiet));
  }

  :host([appearance~='accent']) {
    color: var(--wa-color-on-loud, var(--wa-color-brand-on-loud));
    background-color: var(--wa-color-fill-loud, var(--wa-color-brand-fill-loud));
    border-color: transparent;

    [part~='icon'] {
      color: currentColor;
    }
  }

  [part~='icon'] {
    flex: 0 0 auto;
    display: flex;
    align-items: center;
    color: var(--wa-color-on-quiet);
    font-size: 1.25em;
  }

  ::slotted([slot='icon']) {
    margin-inline-end: var(--wa-form-control-padding-inline);
  }

  [part~='message'] {
    flex: 1 1 auto;
    display: block;
    overflow: hidden;
  }
`;

// chunks/chunk.EQZFZGSD.js
var WaCallout = class extends WebAwesomeElement {
  constructor() {
    super(...arguments);
    this.variant = "brand";
    this.size = "medium";
  }
  render() {
    return x`
      <div part="icon">
        <slot name="icon"></slot>
      </div>

      <div part="message">
        <slot></slot>
      </div>
    `;
  }
};
WaCallout.css = [callout_styles_default, variants_styles_default, size_styles_default];
__decorateClass([
  n3({ reflect: true })
], WaCallout.prototype, "variant", 2);
__decorateClass([
  n3({ reflect: true })
], WaCallout.prototype, "appearance", 2);
__decorateClass([
  n3({ reflect: true })
], WaCallout.prototype, "size", 2);
WaCallout = __decorateClass([
  t3("wa-callout")
], WaCallout);

// chunks/chunk.D2ESGZCU.js
var card_styles_default = i2`
  :host {
    --spacing: var(--wa-space-l);

    /* Internal calculated properties */
    --inner-border-radius: calc(var(--wa-panel-border-radius) - var(--wa-panel-border-width));

    display: flex;
    flex-direction: column;
    background-color: var(--wa-color-surface-default);
    border-color: var(--wa-color-surface-border);
    border-radius: var(--wa-panel-border-radius);
    border-style: var(--wa-panel-border-style);
    box-shadow: var(--wa-shadow-s);
    border-width: var(--wa-panel-border-width);
    color: var(--wa-color-text-normal);
  }

  /* Appearance modifiers */
  :host([appearance='plain']) {
    background-color: transparent;
    border-color: transparent;
    box-shadow: none;
  }

  :host([appearance='outlined']) {
    background-color: var(--wa-color-surface-default);
    border-color: var(--wa-color-surface-border);
  }

  :host([appearance='filled']) {
    background-color: var(--wa-color-neutral-fill-quiet);
    border-color: transparent;
  }

  :host([appearance='filled-outlined']) {
    background-color: var(--wa-color-neutral-fill-quiet);
    border-color: var(--wa-color-surface-border);
  }

  :host([appearance='accent']) {
    color: var(--wa-color-neutral-on-loud);
    background-color: var(--wa-color-neutral-fill-loud);
    border-color: transparent;
  }

  /* Take care of top and bottom radii */
  .media,
  :host(:not([with-media])) .header,
  :host(:not([with-media], [with-header])) .body {
    border-start-start-radius: var(--inner-border-radius);
    border-start-end-radius: var(--inner-border-radius);
  }

  :host(:not([with-footer])) .body,
  .footer {
    border-end-start-radius: var(--inner-border-radius);
    border-end-end-radius: var(--inner-border-radius);
  }

  .media {
    display: flex;
    overflow: hidden;

    &::slotted(*) {
      display: block;
      width: 100%;
      border-radius: 0 !important;
    }
  }

  /* Round all corners for plain appearance */
  :host([appearance='plain']) .media {
    border-radius: var(--inner-border-radius);

    &::slotted(*) {
      border-radius: inherit !important;
    }
  }

  .header {
    display: block;
    border-block-end-style: inherit;
    border-block-end-color: var(--wa-color-surface-border);
    border-block-end-width: var(--wa-panel-border-width);
    padding: calc(var(--spacing) / 2) var(--spacing);
  }

  .body {
    display: block;
    padding: var(--spacing);
  }

  .footer {
    display: block;
    border-block-start-style: inherit;
    border-block-start-color: var(--wa-color-surface-border);
    border-block-start-width: var(--wa-panel-border-width);
    padding: var(--spacing);
  }

  /* Push slots to sides when the action slots renders */
  .has-actions {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  :host(:not([with-header])) .header,
  :host(:not([with-footer])) .footer,
  :host(:not([with-media])) .media {
    display: none;
  }

  /* Orientation Styles */
  :host([orientation='horizontal']) {
    flex-direction: row;

    .media {
      border-start-start-radius: var(--inner-border-radius);
      border-end-start-radius: var(--inner-border-radius);
      border-start-end-radius: 0;

      &::slotted(*) {
        block-size: 100%;
        inline-size: 100%;
        object-fit: cover;
      }
    }
  }

  :host([orientation='horizontal']) .body slot::slotted(*) {
    display: block;
    height: 100%;
    margin: 0;
  }

  :host([orientation='horizontal']) slot[name='actions']::slotted(*) {
    display: flex;
    align-items: center;
    padding: var(--spacing);
  }
`;

// chunks/chunk.UV6ZR4VK.js
var WaCard = class extends WebAwesomeElement {
  constructor() {
    super(...arguments);
    this.hasSlotController = new HasSlotController(this, "footer", "header", "media", "header-actions", "footer-actions", "actions");
    this.appearance = "outlined";
    this.withHeader = false;
    this.withMedia = false;
    this.withFooter = false;
    this.orientation = "vertical";
  }
  willUpdate() {
    if (!this.withHeader && this.hasSlotController.test("header"))
      this.withHeader = true;
    if (!this.withMedia && this.hasSlotController.test("media"))
      this.withMedia = true;
    if (!this.withFooter && this.hasSlotController.test("footer"))
      this.withFooter = true;
  }
  render() {
    if (this.orientation === "horizontal") {
      return x`
        <slot name="media" part="media" class="media"></slot>
        <div part="body" class="body"><slot></slot></div>
        <slot name="actions" part="actions" class="actions"></slot>
      `;
    }
    return x`
      <slot name="media" part="media" class="media"></slot>

      ${this.hasSlotController.test("header-actions") ? x` <header part="header" class="header has-actions">
            <slot name="header"></slot>
            <slot name="header-actions"></slot>
          </header>` : x` <header part="header" class="header">
            <slot name="header"></slot>
          </header>`}

      <div part="body" class="body"><slot></slot></div>
      ${this.hasSlotController.test("footer-actions") ? x` <footer part="footer" class="footer has-actions">
            <slot name="footer"></slot>
            <slot name="footer-actions"></slot>
          </footer>` : x` <footer part="footer" class="footer">
            <slot name="footer"></slot>
          </footer>`}
    `;
  }
};
WaCard.css = [size_styles_default, card_styles_default];
__decorateClass([
  n3({ reflect: true })
], WaCard.prototype, "appearance", 2);
__decorateClass([
  n3({ attribute: "with-header", type: Boolean, reflect: true })
], WaCard.prototype, "withHeader", 2);
__decorateClass([
  n3({ attribute: "with-media", type: Boolean, reflect: true })
], WaCard.prototype, "withMedia", 2);
__decorateClass([
  n3({ attribute: "with-footer", type: Boolean, reflect: true })
], WaCard.prototype, "withFooter", 2);
__decorateClass([
  n3({ reflect: true })
], WaCard.prototype, "orientation", 2);
WaCard = __decorateClass([
  t3("wa-card")
], WaCard);
WaCard.disableWarning?.("change-in-update");

// chunks/chunk.EF25YJJE.js
var WaSlideChangeEvent = class extends Event {
  constructor(detail) {
    super("wa-slide-change", { bubbles: true, cancelable: false, composed: true });
    this.detail = detail;
  }
};

// chunks/chunk.O6IZ4I7T.js
var urlAlphabet = "useandom-26T198340PX75pxJACKVERYMINDBUSHWOLF_GQZbfghjklqvwyzrict";
var nanoid = (size3 = 21) => {
  let id = "";
  let bytes = crypto.getRandomValues(new Uint8Array(size3 |= 0));
  while (size3--) {
    id += urlAlphabet[bytes[size3] & 63];
  }
  return id;
};
function clamp(value, min2, max2) {
  const noNegativeZero = (n5) => Object.is(n5, -0) ? 0 : n5;
  if (value < min2) {
    return noNegativeZero(min2);
  }
  if (value > max2) {
    return noNegativeZero(max2);
  }
  return noNegativeZero(value);
}
function uniqueId(prefix = "") {
  return `${prefix}${nanoid()}`;
}

// chunks/chunk.F25QOBDY.js
function waitForEvent(el, eventName) {
  return new Promise((resolve) => {
    function done(event) {
      if (event.target === el) {
        el.removeEventListener(eventName, done);
        resolve();
      }
    }
    el.addEventListener(eventName, done);
  });
}

// chunks/chunk.L6CIKOFQ.js
function animateWithClass(el, className) {
  return new Promise((resolve) => {
    const controller = new AbortController();
    const { signal } = controller;
    if (el.classList.contains(className)) {
      return;
    }
    el.classList.add(className);
    let resolved = false;
    let onEnd = () => {
      if (resolved) {
        return;
      }
      resolved = true;
      el.classList.remove(className);
      resolve();
      controller.abort();
    };
    el.addEventListener("animationend", onEnd, { once: true, signal });
    el.addEventListener("animationcancel", onEnd, { once: true, signal });
    requestAnimationFrame(() => {
      if (!resolved && el.getAnimations().length === 0) {
        onEnd();
      }
    });
  });
}
function prefersReducedMotion() {
  const query = window.matchMedia("(prefers-reduced-motion: reduce)");
  return query.matches;
}

// chunks/chunk.6SNQOYNK.js
var AutoplayController = class {
  constructor(host, tickCallback) {
    this.timerId = 0;
    this.activeInteractions = 0;
    this.paused = false;
    this.stopped = true;
    this.pause = () => {
      if (!this.activeInteractions++) {
        this.paused = true;
        this.host.requestUpdate();
      }
    };
    this.resume = () => {
      if (!--this.activeInteractions) {
        this.paused = false;
        this.host.requestUpdate();
      }
    };
    host.addController(this);
    this.host = host;
    this.tickCallback = tickCallback;
  }
  hostConnected() {
    this.host.addEventListener("mouseenter", this.pause);
    this.host.addEventListener("mouseleave", this.resume);
    this.host.addEventListener("focusin", this.pause);
    this.host.addEventListener("focusout", this.resume);
    this.host.addEventListener("touchstart", this.pause, { passive: true });
    this.host.addEventListener("touchend", this.resume);
  }
  hostDisconnected() {
    this.stop();
    this.host.removeEventListener("mouseenter", this.pause);
    this.host.removeEventListener("mouseleave", this.resume);
    this.host.removeEventListener("focusin", this.pause);
    this.host.removeEventListener("focusout", this.resume);
    this.host.removeEventListener("touchstart", this.pause);
    this.host.removeEventListener("touchend", this.resume);
  }
  start(interval) {
    this.stop();
    this.stopped = false;
    this.timerId = window.setInterval(() => {
      if (!this.paused) {
        this.tickCallback();
      }
    }, interval);
  }
  stop() {
    clearInterval(this.timerId);
    this.stopped = true;
    this.host.requestUpdate();
  }
};

// chunks/chunk.IYXV6LE3.js
var carousel_styles_default = i2`
  :host {
    --aspect-ratio: 16 / 9;
    --scroll-hint: 0px;
    --slide-gap: var(--wa-space-m, 1rem); /* fallback value is necessary */

    display: flex;
  }

  .carousel {
    display: grid;
    grid-template-columns: min-content 1fr min-content;
    grid-template-rows: 1fr min-content;
    grid-template-areas:
      '. slides .'
      '. pagination .';
    gap: var(--wa-space-m);
    align-items: center;
    min-height: 100%;
    min-width: 100%;
    position: relative;
  }

  .pagination {
    grid-area: pagination;
    display: flex;
    flex-wrap: wrap;
    justify-content: center;
    gap: var(--wa-space-s);
  }

  .slides {
    grid-area: slides;

    display: grid;
    height: 100%;
    width: 100%;
    align-items: center;
    justify-items: center;
    overflow: auto;
    overscroll-behavior-x: contain;
    scrollbar-width: none;
    aspect-ratio: calc(var(--aspect-ratio) * var(--slides-per-page));
    border-radius: var(--wa-border-radius-m);

    --slide-size: calc((100% - (var(--slides-per-page) - 1) * var(--slide-gap)) / var(--slides-per-page));
  }

  @media (prefers-reduced-motion) {
    :where(.slides) {
      scroll-behavior: auto;
    }
  }

  .slides-horizontal {
    grid-auto-flow: column;
    grid-auto-columns: var(--slide-size);
    grid-auto-rows: 100%;
    column-gap: var(--slide-gap);
    scroll-snap-type: x mandatory;
    scroll-padding-inline: var(--scroll-hint);
    padding-inline: var(--scroll-hint);
    overflow-y: hidden;
  }

  .slides-vertical {
    grid-auto-flow: row;
    grid-auto-columns: 100%;
    grid-auto-rows: var(--slide-size);
    row-gap: var(--slide-gap);
    scroll-snap-type: y mandatory;
    scroll-padding-block: var(--scroll-hint);
    padding-block: var(--scroll-hint);
    overflow-x: hidden;
  }

  .slides-dragging,
  .slides-dropping {
    scroll-snap-type: unset;
  }

  :host([vertical]) ::slotted(wa-carousel-item) {
    height: 100%;
  }

  .slides::-webkit-scrollbar {
    display: none;
  }

  .navigation {
    grid-area: navigation;
    display: contents;
    font-size: var(--wa-font-size-l);
  }

  .navigation-button {
    flex: 0 0 auto;
    display: flex;
    align-items: center;
    background: none;
    border: none;
    border-radius: var(--wa-border-radius-m);
    font-size: inherit;
    color: var(--wa-color-text-quiet);
    padding: var(--wa-space-xs);
    cursor: pointer;
    transition: var(--wa-transition-normal) color;
    appearance: none;
  }

  .navigation-button-disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .navigation-button-disabled::part(base) {
    pointer-events: none;
  }

  .navigation-button-previous {
    grid-column: 1;
    grid-row: 1;
  }

  .navigation-button-next {
    grid-column: 3;
    grid-row: 1;
  }

  .pagination-item {
    display: block;
    cursor: pointer;
    background: none;
    border: 0;
    border-radius: var(--wa-border-radius-circle);
    width: var(--wa-space-s);
    height: var(--wa-space-s);
    background-color: var(--wa-color-neutral-fill-normal);
    padding: 0;
    margin: 0;
    transition: transform var(--wa-transition-slow);
  }

  .pagination-item-active {
    background-color: var(--wa-form-control-activated-color);
    transform: scale(1.25);
  }

  /* Focus styles */
  .slides:focus-visible,
  .navigation-button:focus-visible,
  .pagination-item:focus-visible {
    outline: var(--wa-focus-ring);
    outline-offset: var(--wa-focus-ring-offset);
  }
`;

// chunks/chunk.BQNDCXAL.js
var n4 = "important";
var i23 = " !" + n4;
var o6 = e4(class extends i4 {
  constructor(t23) {
    if (super(t23), t23.type !== t4.ATTRIBUTE || t23.name !== "style" || t23.strings?.length > 2)
      throw Error("The `styleMap` directive must be used in the `style` attribute and must be the only part in the attribute.");
  }
  render(t23) {
    return Object.keys(t23).reduce((e25, r5) => {
      const s3 = t23[r5];
      return s3 == null ? e25 : e25 + `${r5 = r5.includes("-") ? r5 : r5.replace(/(?:^(webkit|moz|ms|o)|)(?=[A-Z])/g, "-$&").toLowerCase()}:${s3};`;
    }, "");
  }
  update(e25, [r5]) {
    const { style: s3 } = e25.element;
    if (this.ft === void 0)
      return this.ft = new Set(Object.keys(r5)), this.render(r5);
    for (const t23 of this.ft)
      r5[t23] == null && (this.ft.delete(t23), t23.includes("-") ? s3.removeProperty(t23) : s3[t23] = null);
    for (const t23 in r5) {
      const e33 = r5[t23];
      if (e33 != null) {
        this.ft.add(t23);
        const r24 = typeof e33 == "string" && e33.endsWith(i23);
        t23.includes("-") || r24 ? s3.setProperty(t23, r24 ? e33.slice(0, -11) : e33, r24 ? n4 : "") : s3[t23] = e33;
      }
    }
    return T;
  }
});

// chunks/chunk.ARYES5AV.js
(() => {
  if (o3) {
    return;
  }
  const debounce = (fn, delay) => {
    let timerId = 0;
    return function(...args) {
      window.clearTimeout(timerId);
      timerId = window.setTimeout(() => {
        fn.call(this, ...args);
      }, delay);
    };
  };
  const decorate = (proto, method, decorateFn) => {
    const superFn = proto[method];
    proto[method] = function(...args) {
      superFn.call(this, ...args);
      decorateFn.call(this, superFn, ...args);
    };
  };
  const isSupported = "onscrollend" in window;
  if (!isSupported) {
    const pointers = /* @__PURE__ */ new Set();
    const scrollHandlers = /* @__PURE__ */ new WeakMap();
    const handlePointerDown = (event) => {
      pointers.add(event.pointerId);
    };
    const handlePointerUp = (event) => {
      pointers.delete(event.pointerId);
    };
    document.addEventListener("pointerdown", handlePointerDown);
    document.addEventListener("pointerup", handlePointerUp);
    decorate(EventTarget.prototype, "addEventListener", function(addEventListener, type) {
      if (type !== "scroll")
        return;
      const handleScrollEnd = debounce(() => {
        if (!pointers.size) {
          this.dispatchEvent(new Event("scrollend"));
        } else {
          handleScrollEnd();
        }
      }, 100);
      addEventListener.call(this, "scroll", handleScrollEnd, { passive: true });
      scrollHandlers.set(this, handleScrollEnd);
    });
    decorate(EventTarget.prototype, "removeEventListener", function(removeEventListener, type) {
      if (type !== "scroll")
        return;
      const scrollHandler = scrollHandlers.get(this);
      if (scrollHandler) {
        removeEventListener.call(this, "scroll", scrollHandler, { passive: true });
      }
    });
  }
})();
function* o32(o52, f4) {
  if (o52 !== void 0) {
    let i7 = 0;
    for (const t32 of o52)
      yield f4(t32, i7++);
  }
}
function* o42(o52, t32, e33 = 1) {
  const i7 = t32 === void 0 ? 0 : o52;
  t32 ?? (t32 = o52);
  for (let o62 = i7; e33 > 0 ? o62 < t32 : t32 < o62; o62 += e33)
    yield o62;
}
var WaCarousel = class extends WebAwesomeElement {
  constructor() {
    super(...arguments);
    this.loop = false;
    this.slides = 0;
    this.currentSlide = 0;
    this.navigation = false;
    this.pagination = false;
    this.autoplay = false;
    this.autoplayInterval = 3e3;
    this.slidesPerPage = 1;
    this.slidesPerMove = 1;
    this.orientation = "horizontal";
    this.mouseDragging = false;
    this.activeSlide = 0;
    this.scrolling = false;
    this.dragging = false;
    this.autoplayController = new AutoplayController(this, () => this.next());
    this.dragStartPosition = [-1, -1];
    this.localize = new LocalizeController2(this);
    this.pendingSlideChange = false;
    this.handleMouseDrag = (event) => {
      if (!this.dragging) {
        this.scrollContainer.style.setProperty("scroll-snap-type", "none");
        this.dragging = true;
        this.dragStartPosition = [event.clientX, event.clientY];
      }
      this.scrollContainer.scrollBy({
        left: -event.movementX,
        top: -event.movementY,
        behavior: "instant"
      });
    };
    this.handleMouseDragEnd = () => {
      const scrollContainer = this.scrollContainer;
      document.removeEventListener("pointermove", this.handleMouseDrag, { capture: true });
      const startLeft = scrollContainer.scrollLeft;
      const startTop = scrollContainer.scrollTop;
      scrollContainer.style.removeProperty("scroll-snap-type");
      scrollContainer.style.setProperty("overflow", "hidden");
      const finalLeft = scrollContainer.scrollLeft;
      const finalTop = scrollContainer.scrollTop;
      scrollContainer.style.removeProperty("overflow");
      scrollContainer.style.setProperty("scroll-snap-type", "none");
      scrollContainer.scrollTo({ left: startLeft, top: startTop, behavior: "instant" });
      requestAnimationFrame(async () => {
        if (startLeft !== finalLeft || startTop !== finalTop) {
          scrollContainer.scrollTo({
            left: finalLeft,
            top: finalTop,
            behavior: prefersReducedMotion() ? "auto" : "smooth"
          });
          await waitForEvent(scrollContainer, "scrollend");
        }
        scrollContainer.style.removeProperty("scroll-snap-type");
        this.dragging = false;
        this.dragStartPosition = [-1, -1];
        this.handleScrollEnd();
      });
    };
    this.handleSlotChange = (mutations) => {
      const needsInitialization = mutations.some((mutation) => [...mutation.addedNodes, ...mutation.removedNodes].some((el) => this.isCarouselItem(el) && !el.hasAttribute("data-clone")));
      if (needsInitialization) {
        this.initializeSlides();
      }
      this.requestUpdate();
    };
  }
  connectedCallback() {
    super.connectedCallback();
    if (!o3) {
      this.setAttribute("role", "region");
      this.setAttribute("aria-label", this.localize.term("carousel"));
    }
  }
  disconnectedCallback() {
    super.disconnectedCallback();
    this.mutationObserver?.disconnect();
    this.resizeObserver?.disconnect();
  }
  firstUpdated() {
    this.initializeSlides();
    this.mutationObserver = new MutationObserver(this.handleSlotChange);
    this.mutationObserver.observe(this, {
      childList: true,
      subtree: true
    });
    this.resizeObserver = new ResizeObserver(() => {
      if (this.scrollContainer?.clientWidth || this.scrollContainer?.clientHeight) {
        this.synchronizeSlides();
        this.resizeObserver?.disconnect();
        this.resizeObserver = void 0;
      }
    });
    this.resizeObserver.observe(this);
  }
  willUpdate(changedProperties) {
    if (changedProperties.has("slidesPerMove") || changedProperties.has("slidesPerPage")) {
      this.slidesPerMove = Math.min(this.slidesPerMove, this.slidesPerPage);
    }
  }
  getPageCount() {
    const slidesCount = this.getSlides().length;
    const { slidesPerPage, slidesPerMove, loop } = this;
    const pages = loop ? slidesCount / slidesPerMove : (slidesCount - slidesPerPage) / slidesPerMove + 1;
    return Math.ceil(pages);
  }
  getCurrentPage() {
    return Math.ceil(this.activeSlide / this.slidesPerMove);
  }
  canScrollNext() {
    return this.loop || this.getCurrentPage() < this.getPageCount() - 1;
  }
  canScrollPrev() {
    return this.loop || this.getCurrentPage() > 0;
  }
  getSlides({ excludeClones = true } = {}) {
    return [...this.children].filter((el) => this.isCarouselItem(el) && (!excludeClones || !el.hasAttribute("data-clone")));
  }
  handleClick(event) {
    if (this.dragging && this.dragStartPosition[0] > 0 && this.dragStartPosition[1] > 0) {
      const deltaX = Math.abs(this.dragStartPosition[0] - event.clientX);
      const deltaY = Math.abs(this.dragStartPosition[1] - event.clientY);
      const delta = Math.sqrt(deltaX * deltaX + deltaY * deltaY);
      if (delta >= 10) {
        event.preventDefault();
      }
    }
  }
  handleKeyDown(event) {
    if (["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown", "Home", "End"].includes(event.key)) {
      const target = event.target;
      const isRtl = this.localize.dir() === "rtl";
      const isFocusInPagination = target.closest('[part~="pagination-item"]') !== null;
      const isNext = event.key === "ArrowDown" || !isRtl && event.key === "ArrowRight" || isRtl && event.key === "ArrowLeft";
      const isPrevious = event.key === "ArrowUp" || !isRtl && event.key === "ArrowLeft" || isRtl && event.key === "ArrowRight";
      event.preventDefault();
      if (isPrevious) {
        this.previous();
      }
      if (isNext) {
        this.next();
      }
      if (event.key === "Home") {
        this.goToSlide(0);
      }
      if (event.key === "End") {
        this.goToSlide(this.getSlides().length - 1);
      }
      if (isFocusInPagination) {
        this.updateComplete.then(() => {
          const activePaginationItem = this.shadowRoot?.querySelector('[part~="pagination-item-active"]');
          if (activePaginationItem) {
            activePaginationItem.focus();
          }
        });
      }
    }
  }
  handleMouseDragStart(event) {
    const canDrag = this.mouseDragging && event.button === 0;
    if (canDrag) {
      event.preventDefault();
      document.addEventListener("pointermove", this.handleMouseDrag, { capture: true, passive: true });
      document.addEventListener("pointerup", this.handleMouseDragEnd, { capture: true, once: true });
    }
  }
  handleScroll() {
    this.scrolling = true;
    if (!this.pendingSlideChange) {
      this.synchronizeSlides();
    }
  }
  synchronizeSlides() {
    const io = new IntersectionObserver((entries) => {
      io.disconnect();
      for (const entry of entries) {
        const slide = entry.target;
        slide.toggleAttribute("inert", !entry.isIntersecting);
        slide.classList.toggle("--in-view", entry.isIntersecting);
        slide.setAttribute("aria-hidden", entry.isIntersecting ? "false" : "true");
      }
      const firstIntersecting = entries.find((entry) => entry.isIntersecting);
      if (!firstIntersecting) {
        return;
      }
      const slidesWithClones = this.getSlides({ excludeClones: false });
      const slidesCount = this.getSlides().length;
      const slideIndex = slidesWithClones.indexOf(firstIntersecting.target);
      const normalizedIndex = this.loop ? slideIndex - this.slidesPerPage : slideIndex;
      if (firstIntersecting) {
        this.activeSlide = (Math.ceil(normalizedIndex / this.slidesPerMove) * this.slidesPerMove + slidesCount) % slidesCount;
        if (!this.scrolling) {
          if (this.loop && firstIntersecting.target.hasAttribute("data-clone")) {
            const clonePosition = Number(firstIntersecting.target.getAttribute("data-clone"));
            this.goToSlide(clonePosition, "instant");
          }
        }
      }
    }, {
      root: this.scrollContainer,
      threshold: 0.6
    });
    this.getSlides({ excludeClones: false }).forEach((slide) => {
      io.observe(slide);
    });
  }
  handleScrollEnd() {
    if (!this.scrolling || this.dragging)
      return;
    this.synchronizeSlides();
    this.scrolling = false;
    this.pendingSlideChange = false;
    this.synchronizeSlides();
  }
  isCarouselItem(node) {
    return node instanceof Element && node.tagName.toLowerCase() === "wa-carousel-item";
  }
  initializeSlides() {
    this.getSlides({ excludeClones: false }).forEach((slide, index) => {
      slide.classList.remove("--in-view");
      slide.classList.remove("--is-active");
      slide.setAttribute("aria-label", this.localize.term("slideNum", index + 1));
      if (slide.hasAttribute("data-clone")) {
        slide.remove();
      }
    });
    this.updateSlidesSnap();
    if (this.loop) {
      this.createClones();
    }
    this.goToSlide(this.activeSlide, "auto");
    this.synchronizeSlides();
  }
  createClones() {
    const slides = this.getSlides();
    const slidesPerPage = this.slidesPerPage;
    const lastSlides = slides.slice(-slidesPerPage);
    const firstSlides = slides.slice(0, slidesPerPage);
    lastSlides.reverse().forEach((slide, i7) => {
      const clone = slide.cloneNode(true);
      clone.setAttribute("data-clone", String(slides.length - i7 - 1));
      this.prepend(clone);
    });
    firstSlides.forEach((slide, i7) => {
      const clone = slide.cloneNode(true);
      clone.setAttribute("data-clone", String(i7));
      this.append(clone);
    });
  }
  handleSlideChange() {
    const slides = this.getSlides();
    slides.forEach((slide, i7) => {
      slide.classList.toggle("--is-active", i7 === this.activeSlide);
    });
    if (this.hasUpdated) {
      this.dispatchEvent(new WaSlideChangeEvent({
        index: this.activeSlide,
        slide: slides[this.activeSlide]
      }));
    }
  }
  updateSlidesSnap() {
    const slides = this.getSlides();
    const slidesPerMove = this.slidesPerMove;
    slides.forEach((slide, i7) => {
      const shouldSnap = (i7 + slidesPerMove) % slidesPerMove === 0;
      if (shouldSnap) {
        slide.style.removeProperty("scroll-snap-align");
      } else {
        slide.style.setProperty("scroll-snap-align", "none");
      }
    });
  }
  handleAutoplayChange() {
    this.autoplayController.stop();
    if (this.autoplay) {
      this.autoplayController.start(this.autoplayInterval);
    }
  }
  previous(behavior = "smooth") {
    this.goToSlide(this.activeSlide - this.slidesPerMove, behavior);
  }
  next(behavior = "smooth") {
    this.goToSlide(this.activeSlide + this.slidesPerMove, behavior);
  }
  goToSlide(index, behavior = "smooth") {
    const { slidesPerPage, loop } = this;
    const slides = this.getSlides();
    const slidesWithClones = this.getSlides({ excludeClones: false });
    if (!slides.length) {
      return;
    }
    const newActiveSlide = loop ? (index + slides.length) % slides.length : clamp(index, 0, slides.length - slidesPerPage);
    this.activeSlide = newActiveSlide;
    const isRtl = this.localize.dir() === "rtl";
    const nextSlideIndex = clamp(index + (loop ? slidesPerPage : 0) + (isRtl ? slidesPerPage - 1 : 0), 0, slidesWithClones.length - 1);
    const nextSlide = slidesWithClones[nextSlideIndex];
    this.scrollToSlide(nextSlide, prefersReducedMotion() ? "auto" : behavior);
  }
  scrollToSlide(slide, behavior = "smooth") {
    this.pendingSlideChange = true;
    window.requestAnimationFrame(() => {
      if (!this.scrollContainer) {
        return;
      }
      const scrollContainer = this.scrollContainer;
      const scrollContainerRect = scrollContainer.getBoundingClientRect();
      const nextSlideRect = slide.getBoundingClientRect();
      const nextLeft = nextSlideRect.left - scrollContainerRect.left;
      const nextTop = nextSlideRect.top - scrollContainerRect.top;
      if (nextLeft || nextTop) {
        this.pendingSlideChange = true;
        scrollContainer.scrollTo({
          left: nextLeft + scrollContainer.scrollLeft,
          top: nextTop + scrollContainer.scrollTop,
          behavior
        });
      } else {
        this.pendingSlideChange = false;
      }
    });
  }
  render() {
    const { slidesPerMove, scrolling } = this;
    let pagesCount = 0;
    let currentPage = 0;
    let prevEnabled = false;
    let nextEnabled = false;
    if (this.hasUpdated) {
      pagesCount = this.getPageCount();
      currentPage = this.getCurrentPage();
      prevEnabled = this.canScrollPrev();
      nextEnabled = this.canScrollNext();
    }
    const isRTL2 = o3 ? this.dir === "rtl" : this.localize.dir() === "rtl";
    return x`
      <div part="base" class="carousel">
        <div
          id="scroll-container"
          part="scroll-container"
          class="${e24({
      slides: true,
      "slides-horizontal": this.orientation === "horizontal",
      "slides-vertical": this.orientation === "vertical",
      "slides-dragging": this.dragging
    })}"
          style=${o6({ "--slides-per-page": this.slidesPerPage })}
          aria-busy="${scrolling ? "true" : "false"}"
          aria-atomic="true"
          tabindex="0"
          @keydown=${this.handleKeyDown}
          @mousedown="${this.handleMouseDragStart}"
          @scroll="${this.handleScroll}"
          @scrollend=${this.handleScrollEnd}
          @click=${this.handleClick}
        >
          <slot @slotchange=${() => this.requestUpdate()}></slot>
        </div>

        ${this.navigation ? x`
              <div part="navigation" class="navigation">
                <button
                  part="navigation-button navigation-button-previous"
                  class="${e24({
      "navigation-button": true,
      "navigation-button-previous": true,
      "navigation-button-disabled": !prevEnabled
    })}"
                  aria-label="${this.localize.term("previousSlide")}"
                  aria-controls="scroll-container"
                  aria-disabled="${prevEnabled ? "false" : "true"}"
                  @click=${prevEnabled ? () => this.previous() : null}
                >
                  <slot name="previous-icon">
                    <wa-icon library="system" name="${isRTL2 ? "chevron-right" : "chevron-left"}"></wa-icon>
                  </slot>
                </button>

                <button
                  part="navigation-button navigation-button-next"
                  class=${e24({
      "navigation-button": true,
      "navigation-button-next": true,
      "navigation-button-disabled": !nextEnabled
    })}
                  aria-label="${this.localize.term("nextSlide")}"
                  aria-controls="scroll-container"
                  aria-disabled="${nextEnabled ? "false" : "true"}"
                  @click=${nextEnabled ? () => this.next() : null}
                >
                  <slot name="next-icon">
                    <wa-icon library="system" name="${isRTL2 ? "chevron-left" : "chevron-right"}"></wa-icon>
                  </slot>
                </button>
              </div>
            ` : ""}
        ${this.pagination ? x`
              <div part="pagination" role="tablist" class="pagination" aria-controls="scroll-container">
                ${o32(o42(pagesCount), (index) => {
      const isActive = index === currentPage;
      return x`
                    <button
                      part="pagination-item ${isActive ? "pagination-item-active" : ""}"
                      class="${e24({
        "pagination-item": true,
        "pagination-item-active": isActive
      })}"
                      role="tab"
                      aria-selected="${isActive ? "true" : "false"}"
                      aria-label="${this.localize.term("goToSlide", index + 1, pagesCount)}"
                      tabindex=${isActive ? "0" : "-1"}
                      @click=${() => this.goToSlide(index * slidesPerMove)}
                      @keydown=${this.handleKeyDown}
                    ></button>
                  `;
    })}
              </div>
            ` : x``}
      </div>
    `;
  }
};
WaCarousel.css = carousel_styles_default;
__decorateClass([
  n3({ type: Boolean, reflect: true })
], WaCarousel.prototype, "loop", 2);
__decorateClass([
  n3({ type: Number, reflect: true })
], WaCarousel.prototype, "slides", 2);
__decorateClass([
  n3({ type: Number, reflect: true })
], WaCarousel.prototype, "currentSlide", 2);
__decorateClass([
  n3({ type: Boolean, reflect: true })
], WaCarousel.prototype, "navigation", 2);
__decorateClass([
  n3({ type: Boolean, reflect: true })
], WaCarousel.prototype, "pagination", 2);
__decorateClass([
  n3({ type: Boolean, reflect: true })
], WaCarousel.prototype, "autoplay", 2);
__decorateClass([
  n3({ type: Number, attribute: "autoplay-interval" })
], WaCarousel.prototype, "autoplayInterval", 2);
__decorateClass([
  n3({ type: Number, attribute: "slides-per-page" })
], WaCarousel.prototype, "slidesPerPage", 2);
__decorateClass([
  n3({ type: Number, attribute: "slides-per-move" })
], WaCarousel.prototype, "slidesPerMove", 2);
__decorateClass([
  n3()
], WaCarousel.prototype, "orientation", 2);
__decorateClass([
  n3({ type: Boolean, reflect: true, attribute: "mouse-dragging" })
], WaCarousel.prototype, "mouseDragging", 2);
__decorateClass([
  e23(".slides")
], WaCarousel.prototype, "scrollContainer", 2);
__decorateClass([
  e23(".pagination")
], WaCarousel.prototype, "paginationContainer", 2);
__decorateClass([
  r23()
], WaCarousel.prototype, "activeSlide", 2);
__decorateClass([
  r23()
], WaCarousel.prototype, "scrolling", 2);
__decorateClass([
  r23()
], WaCarousel.prototype, "dragging", 2);
__decorateClass([
  t22({ passive: true })
], WaCarousel.prototype, "handleScroll", 1);
__decorateClass([
  watch("loop", { waitUntilFirstUpdate: true }),
  watch("slidesPerPage", { waitUntilFirstUpdate: true })
], WaCarousel.prototype, "initializeSlides", 1);
__decorateClass([
  watch("activeSlide")
], WaCarousel.prototype, "handleSlideChange", 1);
__decorateClass([
  watch("slidesPerMove")
], WaCarousel.prototype, "updateSlidesSnap", 1);
__decorateClass([
  watch("autoplay")
], WaCarousel.prototype, "handleAutoplayChange", 1);
WaCarousel = __decorateClass([
  t3("wa-carousel")
], WaCarousel);

// chunks/chunk.UGUGWXGF.js
var carousel_item_styles_default = i2`
  :host {
    --aspect-ratio: inherit;

    display: flex;
    align-items: center;
    justify-content: center;
    flex-direction: column;
    width: 100%;
    max-height: 100%;
    aspect-ratio: var(--aspect-ratio);
    scroll-snap-align: start;
    scroll-snap-stop: always;
  }

  ::slotted(img) {
    width: 100% !important;
    height: 100% !important;
    object-fit: cover;
  }
`;

// chunks/chunk.2V3SZRCM.js
var WaCarouselItem = class extends WebAwesomeElement {
  connectedCallback() {
    super.connectedCallback();
    this.setAttribute("role", "group");
  }
  render() {
    return x` <slot></slot> `;
  }
};
WaCarouselItem.css = carousel_item_styles_default;
WaCarouselItem = __decorateClass([
  t3("wa-carousel-item")
], WaCarouselItem);

// chunks/chunk.NY2PQ35L.js
var WaCopyEvent = class extends Event {
  constructor(detail) {
    super("wa-copy", { bubbles: true, cancelable: false, composed: true });
    this.detail = detail;
  }
};

// chunks/chunk.I4KXAHPX.js
var visually_hidden_styles_default = i2`
  .wa-visually-hidden:not(:focus-within),
  .wa-visually-hidden-force,
  .wa-visually-hidden-hint::part(hint),
  .wa-visually-hidden-label::part(label),
  .wa-visually-hidden-label::part(form-control-label) {
    position: absolute !important;
    width: 1px !important;
    height: 1px !important;
    clip: rect(0 0 0 0) !important;
    clip-path: inset(50%) !important;
    border: none !important;
    overflow: hidden !important;
    white-space: nowrap !important;
    padding: 0 !important;
  }
`;

// chunks/chunk.6NYGANGC.js
var copy_button_styles_default = i2`
  :host {
    display: inline-block;
    color: var(--wa-color-neutral-on-quiet);
  }

  .copy-button__trigger {
    display: inline-flex;
  }

  .button {
    flex: 0 0 auto;
    display: flex;
    align-items: center;
    background-color: transparent;
    border: none;
    border-radius: var(--wa-form-control-border-radius);
    color: inherit;
    font-size: inherit;
    padding: 0.5em;
    cursor: pointer;
    transition: color var(--wa-transition-fast) var(--wa-transition-easing);
  }

  @media (hover: hover) {
    .button:hover:not([disabled]) {
      background-color: var(--wa-color-neutral-fill-quiet);
      color: color-mix(in oklab, currentColor, var(--wa-color-mix-hover));
    }
  }

  .button:focus-visible:not([disabled]) {
    background-color: var(--wa-color-neutral-fill-quiet);
    color: color-mix(in oklab, currentColor, var(--wa-color-mix-hover));
  }

  .button:active:not([disabled]) {
    color: color-mix(in oklab, currentColor, var(--wa-color-mix-active));
  }

  slot[name='success-icon'] {
    color: var(--wa-color-success-on-quiet);
  }

  slot[name='error-icon'] {
    color: var(--wa-color-danger-on-quiet);
  }

  .button:focus-visible {
    outline: var(--wa-focus-ring);
    outline-offset: var(--wa-focus-ring-offset);
  }

  .button[disabled] {
    opacity: 0.5;
    cursor: not-allowed !important;
  }

  slot {
    display: inline-flex;
  }

  .show {
    animation: show 100ms ease;
  }

  .hide {
    animation: show 100ms ease reverse;
  }

  @keyframes show {
    from {
      scale: 0.25;
      opacity: 0.25;
    }
    to {
      scale: 1;
      opacity: 1;
    }
  }
`;

// chunks/chunk.QE6ZFROZ.js
var WaCopyButton = class extends WebAwesomeElement {
  constructor() {
    super(...arguments);
    this.hasSlotController = new HasSlotController(this, "[default]");
    this.localize = new LocalizeController2(this);
    this.isCopying = false;
    this.status = "rest";
    this.value = "";
    this.from = "";
    this.disabled = false;
    this.copyLabel = "";
    this.successLabel = "";
    this.errorLabel = "";
    this.feedbackDuration = 1e3;
    this.tooltipPlacement = "top";
  }
  get currentLabel() {
    if (this.status === "success") {
      return this.successLabel || this.localize.term("copied");
    }
    if (this.status === "error") {
      return this.errorLabel || this.localize.term("error");
    }
    return this.copyLabel || this.localize.term("copy");
  }
  handleStatusChange() {
    this.customStates.set("success", this.status === "success");
    this.customStates.set("error", this.status === "error");
  }
  async handleCopy() {
    if (this.disabled || this.isCopying) {
      return;
    }
    this.isCopying = true;
    let valueToCopy = this.value;
    if (this.from) {
      const root = this.getRootNode();
      const isProperty = this.from.includes(".");
      const isAttribute = this.from.includes("[") && this.from.includes("]");
      let id = this.from;
      let field = "";
      if (isProperty) {
        [id, field] = this.from.trim().split(".");
      } else if (isAttribute) {
        [id, field] = this.from.trim().replace(/\]$/, "").split("[");
      }
      const target = "getElementById" in root ? root.getElementById(id) : null;
      if (target) {
        if (isAttribute) {
          valueToCopy = target.getAttribute(field) || "";
        } else if (isProperty) {
          valueToCopy = target[field] || "";
        } else {
          valueToCopy = target.textContent || "";
        }
      } else {
        this.showStatus("error");
        this.dispatchEvent(new WaErrorEvent());
      }
    }
    if (!valueToCopy) {
      this.showStatus("error");
      this.dispatchEvent(new WaErrorEvent());
    } else {
      try {
        await navigator.clipboard.writeText(valueToCopy);
        this.showStatus("success");
        this.dispatchEvent(new WaCopyEvent({ value: valueToCopy }));
      } catch (error) {
        this.showStatus("error");
        this.dispatchEvent(new WaErrorEvent());
      }
    }
  }
  async showStatus(status) {
    this.status = status;
    if (this.copyIcon) {
      const iconToShow = status === "success" ? this.successIcon : this.errorIcon;
      await animateWithClass(this.copyIcon, "hide");
      this.copyIcon.hidden = true;
      iconToShow.hidden = false;
      await animateWithClass(iconToShow, "show");
    }
    setTimeout(async () => {
      if (this.copyIcon) {
        const iconToShow = status === "success" ? this.successIcon : this.errorIcon;
        await animateWithClass(iconToShow, "hide");
        iconToShow.hidden = true;
        this.copyIcon.hidden = false;
        await animateWithClass(this.copyIcon, "show");
      }
      this.status = "rest";
      this.isCopying = false;
    }, this.feedbackDuration);
  }
  render() {
    const hasCustomTrigger = this.hasSlotController.test("[default]");
    return x`
      <div class="copy-button__trigger" @click=${this.handleCopy}>
        <slot></slot>
        <button
          class="button"
          part="button"
          type="button"
          id="copy-button"
          ?disabled=${this.disabled}
          ?hidden=${hasCustomTrigger}
        >
          <!-- Render a visually hidden label to appease the accessibility checking gods -->
          <span class="wa-visually-hidden">${this.currentLabel}</span>
          <slot part="copy-icon" name="copy-icon">
            <wa-icon library="system" name="copy" variant="regular"></wa-icon>
          </slot>
          <slot part="success-icon" name="success-icon" variant="solid" hidden>
            <wa-icon library="system" name="check"></wa-icon>
          </slot>
          <slot part="error-icon" name="error-icon" variant="solid" hidden>
            <wa-icon library="system" name="xmark"></wa-icon>
          </slot>
          <wa-tooltip
            class=${e24({
      "copy-button": true,
      "copy-button-success": this.status === "success",
      "copy-button-error": this.status === "error"
    })}
            for="copy-button"
            placement=${this.tooltipPlacement}
            ?disabled=${this.disabled}
            exportparts="
              base:tooltip__base,
              base__popup:tooltip__base__popup,
              base__arrow:tooltip__base__arrow,
              body:tooltip__body
            "
            >${this.currentLabel}</wa-tooltip
          >
        </button>
      </div>
    `;
  }
};
WaCopyButton.css = [host_styles_default, visually_hidden_styles_default, copy_button_styles_default];
__decorateClass([
  e23('slot[name="copy-icon"]')
], WaCopyButton.prototype, "copyIcon", 2);
__decorateClass([
  e23('slot[name="success-icon"]')
], WaCopyButton.prototype, "successIcon", 2);
__decorateClass([
  e23('slot[name="error-icon"]')
], WaCopyButton.prototype, "errorIcon", 2);
__decorateClass([
  e23("wa-tooltip")
], WaCopyButton.prototype, "tooltip", 2);
__decorateClass([
  r23()
], WaCopyButton.prototype, "isCopying", 2);
__decorateClass([
  r23()
], WaCopyButton.prototype, "status", 2);
__decorateClass([
  n3()
], WaCopyButton.prototype, "value", 2);
__decorateClass([
  n3()
], WaCopyButton.prototype, "from", 2);
__decorateClass([
  n3({ type: Boolean, reflect: true })
], WaCopyButton.prototype, "disabled", 2);
__decorateClass([
  n3({ attribute: "copy-label" })
], WaCopyButton.prototype, "copyLabel", 2);
__decorateClass([
  n3({ attribute: "success-label" })
], WaCopyButton.prototype, "successLabel", 2);
__decorateClass([
  n3({ attribute: "error-label" })
], WaCopyButton.prototype, "errorLabel", 2);
__decorateClass([
  n3({ attribute: "feedback-duration", type: Number })
], WaCopyButton.prototype, "feedbackDuration", 2);
__decorateClass([
  n3({ attribute: "tooltip-placement" })
], WaCopyButton.prototype, "tooltipPlacement", 2);
__decorateClass([
  watch("status")
], WaCopyButton.prototype, "handleStatusChange", 1);
WaCopyButton = __decorateClass([
  t3("wa-copy-button")
], WaCopyButton);

// chunks/chunk.Y2DUNANZ.js
var tooltip_styles_default = i2`
  :host {
    --max-width: 30ch;

    /** These styles are added so we don't interfere in the DOM. */
    display: inline-block;
    position: absolute;

    /** Defaults for inherited CSS properties */
    color: var(--wa-tooltip-content-color);
    font-size: var(--wa-tooltip-font-size);
    line-height: var(--wa-tooltip-line-height);
    text-align: start;
    white-space: normal;
  }

  .tooltip {
    --arrow-size: var(--wa-tooltip-arrow-size);
    --arrow-color: var(--wa-tooltip-background-color);
  }

  .tooltip::part(popup) {
    z-index: 1000;
  }

  .tooltip[placement^='top']::part(popup) {
    transform-origin: bottom;
  }

  .tooltip[placement^='bottom']::part(popup) {
    transform-origin: top;
  }

  .tooltip[placement^='left']::part(popup) {
    transform-origin: right;
  }

  .tooltip[placement^='right']::part(popup) {
    transform-origin: left;
  }

  .body {
    display: block;
    width: max-content;
    max-width: var(--max-width);
    border-radius: var(--wa-tooltip-border-radius);
    background-color: var(--wa-tooltip-background-color);
    border: var(--wa-tooltip-border-width) var(--wa-tooltip-border-style) var(--wa-tooltip-border-color);
    padding: 0.25em 0.5em;
    user-select: none;
    -webkit-user-select: none;
  }

  .tooltip {
    --popup-border-width: var(--wa-tooltip-border-width);

    &::part(arrow) {
      border-bottom: var(--wa-tooltip-border-width) var(--wa-tooltip-border-style) var(--wa-tooltip-border-color);
      border-right: var(--wa-tooltip-border-width) var(--wa-tooltip-border-style) var(--wa-tooltip-border-color);
    }
  }
`;

// chunks/chunk.ZWQCGLB5.js
var WaRepositionEvent = class extends Event {
  constructor() {
    super("wa-reposition", { bubbles: true, cancelable: false, composed: true });
  }
};

// chunks/chunk.MAFCUMJD.js
var popup_styles_default = i2`
  :host {
    --arrow-color: black;
    --arrow-size: var(--wa-tooltip-arrow-size);
    --popup-border-width: 0px;
    --show-duration: 100ms;
    --hide-duration: 100ms;

    /*
     * These properties are computed to account for the arrow's dimensions after being rotated 45º. The constant
     * 0.7071 is derived from sin(45) to calculate the length of the arrow after rotation.
     *
     * The diamond will be translated inward by --arrow-base-offset, the border thickness, to centralise it on
     * the inner edge of the popup border. This also means we need to increase the size of the arrow by the
     * same amount to compensate.
     *
     * A diamond shaped clipping mask is used to avoid overlap of popup content. This extends slightly inward so
     * the popup border is covered with no sub-pixel rounding artifacts. The diamond corners are mitred at 22.5º
     * to properly merge any arrow border with the popup border. The constant 1.4142 is derived from 1 + tan(22.5).
     *
     */
    --arrow-base-offset: var(--popup-border-width);
    --arrow-size-diagonal: calc((var(--arrow-size) + var(--arrow-base-offset)) * 0.7071);
    --arrow-padding-offset: calc(var(--arrow-size-diagonal) - var(--arrow-size));
    --arrow-size-div: calc(var(--arrow-size-diagonal) * 2);
    --arrow-clipping-corner: calc(var(--arrow-base-offset) * 1.4142);

    display: contents;
  }

  .popup {
    position: absolute;
    isolation: isolate;
    max-width: var(--auto-size-available-width, none);
    max-height: var(--auto-size-available-height, none);

    /* Clear UA styles for [popover] */
    :where(&) {
      inset: unset;
      padding: unset;
      margin: unset;
      width: unset;
      height: unset;
      color: unset;
      background: unset;
      border: unset;
      overflow: unset;
    }
  }

  .popup-fixed {
    position: fixed;
  }

  .popup:not(.popup-active) {
    display: none;
  }

  .arrow {
    position: absolute;
    width: var(--arrow-size-div);
    height: var(--arrow-size-div);
    background: var(--arrow-color);
    z-index: 3;
    clip-path: polygon(
      var(--arrow-clipping-corner) 100%,
      var(--arrow-base-offset) calc(100% - var(--arrow-base-offset)),
      calc(var(--arrow-base-offset) - 2px) calc(100% - var(--arrow-base-offset)),
      calc(100% - var(--arrow-base-offset)) calc(var(--arrow-base-offset) - 2px),
      calc(100% - var(--arrow-base-offset)) var(--arrow-base-offset),
      100% var(--arrow-clipping-corner),
      100% 100%
    );
    rotate: 45deg;
  }

  :host([data-current-placement|='left']) .arrow {
    rotate: -45deg;
  }

  :host([data-current-placement|='right']) .arrow {
    rotate: 135deg;
  }

  :host([data-current-placement|='bottom']) .arrow {
    rotate: 225deg;
  }

  /* Hover bridge */
  .popup-hover-bridge:not(.popup-hover-bridge-visible) {
    display: none;
  }

  .popup-hover-bridge {
    position: fixed;
    z-index: 899;
    top: 0;
    right: 0;
    bottom: 0;
    left: 0;
    clip-path: polygon(
      var(--hover-bridge-top-left-x, 0) var(--hover-bridge-top-left-y, 0),
      var(--hover-bridge-top-right-x, 0) var(--hover-bridge-top-right-y, 0),
      var(--hover-bridge-bottom-right-x, 0) var(--hover-bridge-bottom-right-y, 0),
      var(--hover-bridge-bottom-left-x, 0) var(--hover-bridge-bottom-left-y, 0)
    );
  }

  /* Built-in animations */
  .show {
    animation: show var(--show-duration) ease;
  }

  .hide {
    animation: show var(--hide-duration) ease reverse;
  }

  @keyframes show {
    from {
      opacity: 0;
    }
    to {
      opacity: 1;
    }
  }

  .show-with-scale {
    animation: show-with-scale var(--show-duration) ease;
  }

  .hide-with-scale {
    animation: show-with-scale var(--hide-duration) ease reverse;
  }

  @keyframes show-with-scale {
    from {
      opacity: 0;
      scale: 0.8;
    }
    to {
      opacity: 1;
      scale: 1;
    }
  }
`;

// chunks/chunk.CCGRUHHE.js
var min = Math.min;
var max = Math.max;
var round = Math.round;
var floor = Math.floor;
var createCoords = (v2) => ({
  x: v2,
  y: v2
});
var oppositeSideMap = {
  left: "right",
  right: "left",
  bottom: "top",
  top: "bottom"
};
var oppositeAlignmentMap = {
  start: "end",
  end: "start"
};
function clamp2(start, value, end) {
  return max(start, min(value, end));
}
function evaluate(value, param) {
  return typeof value === "function" ? value(param) : value;
}
function getSide(placement) {
  return placement.split("-")[0];
}
function getAlignment(placement) {
  return placement.split("-")[1];
}
function getOppositeAxis(axis) {
  return axis === "x" ? "y" : "x";
}
function getAxisLength(axis) {
  return axis === "y" ? "height" : "width";
}
function getSideAxis(placement) {
  return ["top", "bottom"].includes(getSide(placement)) ? "y" : "x";
}
function getAlignmentAxis(placement) {
  return getOppositeAxis(getSideAxis(placement));
}
function getAlignmentSides(placement, rects, rtl) {
  if (rtl === void 0) {
    rtl = false;
  }
  const alignment = getAlignment(placement);
  const alignmentAxis = getAlignmentAxis(placement);
  const length = getAxisLength(alignmentAxis);
  let mainAlignmentSide = alignmentAxis === "x" ? alignment === (rtl ? "end" : "start") ? "right" : "left" : alignment === "start" ? "bottom" : "top";
  if (rects.reference[length] > rects.floating[length]) {
    mainAlignmentSide = getOppositePlacement(mainAlignmentSide);
  }
  return [mainAlignmentSide, getOppositePlacement(mainAlignmentSide)];
}
function getExpandedPlacements(placement) {
  const oppositePlacement = getOppositePlacement(placement);
  return [getOppositeAlignmentPlacement(placement), oppositePlacement, getOppositeAlignmentPlacement(oppositePlacement)];
}
function getOppositeAlignmentPlacement(placement) {
  return placement.replace(/start|end/g, (alignment) => oppositeAlignmentMap[alignment]);
}
function getSideList(side, isStart, rtl) {
  const lr = ["left", "right"];
  const rl = ["right", "left"];
  const tb = ["top", "bottom"];
  const bt = ["bottom", "top"];
  switch (side) {
    case "top":
    case "bottom":
      if (rtl)
        return isStart ? rl : lr;
      return isStart ? lr : rl;
    case "left":
    case "right":
      return isStart ? tb : bt;
    default:
      return [];
  }
}
function getOppositeAxisPlacements(placement, flipAlignment, direction, rtl) {
  const alignment = getAlignment(placement);
  let list = getSideList(getSide(placement), direction === "start", rtl);
  if (alignment) {
    list = list.map((side) => side + "-" + alignment);
    if (flipAlignment) {
      list = list.concat(list.map(getOppositeAlignmentPlacement));
    }
  }
  return list;
}
function getOppositePlacement(placement) {
  return placement.replace(/left|right|bottom|top/g, (side) => oppositeSideMap[side]);
}
function expandPaddingObject(padding) {
  return {
    top: 0,
    right: 0,
    bottom: 0,
    left: 0,
    ...padding
  };
}
function getPaddingObject(padding) {
  return typeof padding !== "number" ? expandPaddingObject(padding) : {
    top: padding,
    right: padding,
    bottom: padding,
    left: padding
  };
}
function rectToClientRect(rect) {
  const {
    x: x2,
    y: y3,
    width,
    height
  } = rect;
  return {
    width,
    height,
    top: y3,
    left: x2,
    right: x2 + width,
    bottom: y3 + height,
    x: x2,
    y: y3
  };
}
function computeCoordsFromPlacement(_ref, placement, rtl) {
  let {
    reference,
    floating
  } = _ref;
  const sideAxis = getSideAxis(placement);
  const alignmentAxis = getAlignmentAxis(placement);
  const alignLength = getAxisLength(alignmentAxis);
  const side = getSide(placement);
  const isVertical = sideAxis === "y";
  const commonX = reference.x + reference.width / 2 - floating.width / 2;
  const commonY = reference.y + reference.height / 2 - floating.height / 2;
  const commonAlign = reference[alignLength] / 2 - floating[alignLength] / 2;
  let coords;
  switch (side) {
    case "top":
      coords = {
        x: commonX,
        y: reference.y - floating.height
      };
      break;
    case "bottom":
      coords = {
        x: commonX,
        y: reference.y + reference.height
      };
      break;
    case "right":
      coords = {
        x: reference.x + reference.width,
        y: commonY
      };
      break;
    case "left":
      coords = {
        x: reference.x - floating.width,
        y: commonY
      };
      break;
    default:
      coords = {
        x: reference.x,
        y: reference.y
      };
  }
  switch (getAlignment(placement)) {
    case "start":
      coords[alignmentAxis] -= commonAlign * (rtl && isVertical ? -1 : 1);
      break;
    case "end":
      coords[alignmentAxis] += commonAlign * (rtl && isVertical ? -1 : 1);
      break;
  }
  return coords;
}
var computePosition = async (reference, floating, config) => {
  const {
    placement = "bottom",
    strategy = "absolute",
    middleware = [],
    platform: platform2
  } = config;
  const validMiddleware = middleware.filter(Boolean);
  const rtl = await (platform2.isRTL == null ? void 0 : platform2.isRTL(floating));
  let rects = await platform2.getElementRects({
    reference,
    floating,
    strategy
  });
  let {
    x: x2,
    y: y3
  } = computeCoordsFromPlacement(rects, placement, rtl);
  let statefulPlacement = placement;
  let middlewareData = {};
  let resetCount = 0;
  for (let i24 = 0; i24 < validMiddleware.length; i24++) {
    const {
      name,
      fn
    } = validMiddleware[i24];
    const {
      x: nextX,
      y: nextY,
      data,
      reset
    } = await fn({
      x: x2,
      y: y3,
      initialPlacement: placement,
      placement: statefulPlacement,
      strategy,
      middlewareData,
      rects,
      platform: platform2,
      elements: {
        reference,
        floating
      }
    });
    x2 = nextX != null ? nextX : x2;
    y3 = nextY != null ? nextY : y3;
    middlewareData = {
      ...middlewareData,
      [name]: {
        ...middlewareData[name],
        ...data
      }
    };
    if (reset && resetCount <= 50) {
      resetCount++;
      if (typeof reset === "object") {
        if (reset.placement) {
          statefulPlacement = reset.placement;
        }
        if (reset.rects) {
          rects = reset.rects === true ? await platform2.getElementRects({
            reference,
            floating,
            strategy
          }) : reset.rects;
        }
        ({
          x: x2,
          y: y3
        } = computeCoordsFromPlacement(rects, statefulPlacement, rtl));
      }
      i24 = -1;
    }
  }
  return {
    x: x2,
    y: y3,
    placement: statefulPlacement,
    strategy,
    middlewareData
  };
};
async function detectOverflow(state, options) {
  var _await$platform$isEle;
  if (options === void 0) {
    options = {};
  }
  const {
    x: x2,
    y: y3,
    platform: platform2,
    rects,
    elements,
    strategy
  } = state;
  const {
    boundary = "clippingAncestors",
    rootBoundary = "viewport",
    elementContext = "floating",
    altBoundary = false,
    padding = 0
  } = evaluate(options, state);
  const paddingObject = getPaddingObject(padding);
  const altContext = elementContext === "floating" ? "reference" : "floating";
  const element = elements[altBoundary ? altContext : elementContext];
  const clippingClientRect = rectToClientRect(await platform2.getClippingRect({
    element: ((_await$platform$isEle = await (platform2.isElement == null ? void 0 : platform2.isElement(element))) != null ? _await$platform$isEle : true) ? element : element.contextElement || await (platform2.getDocumentElement == null ? void 0 : platform2.getDocumentElement(elements.floating)),
    boundary,
    rootBoundary,
    strategy
  }));
  const rect = elementContext === "floating" ? {
    x: x2,
    y: y3,
    width: rects.floating.width,
    height: rects.floating.height
  } : rects.reference;
  const offsetParent = await (platform2.getOffsetParent == null ? void 0 : platform2.getOffsetParent(elements.floating));
  const offsetScale = await (platform2.isElement == null ? void 0 : platform2.isElement(offsetParent)) ? await (platform2.getScale == null ? void 0 : platform2.getScale(offsetParent)) || {
    x: 1,
    y: 1
  } : {
    x: 1,
    y: 1
  };
  const elementClientRect = rectToClientRect(platform2.convertOffsetParentRelativeRectToViewportRelativeRect ? await platform2.convertOffsetParentRelativeRectToViewportRelativeRect({
    elements,
    rect,
    offsetParent,
    strategy
  }) : rect);
  return {
    top: (clippingClientRect.top - elementClientRect.top + paddingObject.top) / offsetScale.y,
    bottom: (elementClientRect.bottom - clippingClientRect.bottom + paddingObject.bottom) / offsetScale.y,
    left: (clippingClientRect.left - elementClientRect.left + paddingObject.left) / offsetScale.x,
    right: (elementClientRect.right - clippingClientRect.right + paddingObject.right) / offsetScale.x
  };
}
var arrow = (options) => ({
  name: "arrow",
  options,
  async fn(state) {
    const {
      x: x2,
      y: y3,
      placement,
      rects,
      platform: platform2,
      elements,
      middlewareData
    } = state;
    const {
      element,
      padding = 0
    } = evaluate(options, state) || {};
    if (element == null) {
      return {};
    }
    const paddingObject = getPaddingObject(padding);
    const coords = {
      x: x2,
      y: y3
    };
    const axis = getAlignmentAxis(placement);
    const length = getAxisLength(axis);
    const arrowDimensions = await platform2.getDimensions(element);
    const isYAxis = axis === "y";
    const minProp = isYAxis ? "top" : "left";
    const maxProp = isYAxis ? "bottom" : "right";
    const clientProp = isYAxis ? "clientHeight" : "clientWidth";
    const endDiff = rects.reference[length] + rects.reference[axis] - coords[axis] - rects.floating[length];
    const startDiff = coords[axis] - rects.reference[axis];
    const arrowOffsetParent = await (platform2.getOffsetParent == null ? void 0 : platform2.getOffsetParent(element));
    let clientSize = arrowOffsetParent ? arrowOffsetParent[clientProp] : 0;
    if (!clientSize || !await (platform2.isElement == null ? void 0 : platform2.isElement(arrowOffsetParent))) {
      clientSize = elements.floating[clientProp] || rects.floating[length];
    }
    const centerToReference = endDiff / 2 - startDiff / 2;
    const largestPossiblePadding = clientSize / 2 - arrowDimensions[length] / 2 - 1;
    const minPadding = min(paddingObject[minProp], largestPossiblePadding);
    const maxPadding = min(paddingObject[maxProp], largestPossiblePadding);
    const min$1 = minPadding;
    const max2 = clientSize - arrowDimensions[length] - maxPadding;
    const center = clientSize / 2 - arrowDimensions[length] / 2 + centerToReference;
    const offset3 = clamp2(min$1, center, max2);
    const shouldAddOffset = !middlewareData.arrow && getAlignment(placement) != null && center !== offset3 && rects.reference[length] / 2 - (center < min$1 ? minPadding : maxPadding) - arrowDimensions[length] / 2 < 0;
    const alignmentOffset = shouldAddOffset ? center < min$1 ? center - min$1 : center - max2 : 0;
    return {
      [axis]: coords[axis] + alignmentOffset,
      data: {
        [axis]: offset3,
        centerOffset: center - offset3 - alignmentOffset,
        ...shouldAddOffset && {
          alignmentOffset
        }
      },
      reset: shouldAddOffset
    };
  }
});
var flip = function(options) {
  if (options === void 0) {
    options = {};
  }
  return {
    name: "flip",
    options,
    async fn(state) {
      var _middlewareData$arrow, _middlewareData$flip;
      const {
        placement,
        middlewareData,
        rects,
        initialPlacement,
        platform: platform2,
        elements
      } = state;
      const {
        mainAxis: checkMainAxis = true,
        crossAxis: checkCrossAxis = true,
        fallbackPlacements: specifiedFallbackPlacements,
        fallbackStrategy = "bestFit",
        fallbackAxisSideDirection = "none",
        flipAlignment = true,
        ...detectOverflowOptions
      } = evaluate(options, state);
      if ((_middlewareData$arrow = middlewareData.arrow) != null && _middlewareData$arrow.alignmentOffset) {
        return {};
      }
      const side = getSide(placement);
      const initialSideAxis = getSideAxis(initialPlacement);
      const isBasePlacement = getSide(initialPlacement) === initialPlacement;
      const rtl = await (platform2.isRTL == null ? void 0 : platform2.isRTL(elements.floating));
      const fallbackPlacements = specifiedFallbackPlacements || (isBasePlacement || !flipAlignment ? [getOppositePlacement(initialPlacement)] : getExpandedPlacements(initialPlacement));
      const hasFallbackAxisSideDirection = fallbackAxisSideDirection !== "none";
      if (!specifiedFallbackPlacements && hasFallbackAxisSideDirection) {
        fallbackPlacements.push(...getOppositeAxisPlacements(initialPlacement, flipAlignment, fallbackAxisSideDirection, rtl));
      }
      const placements2 = [initialPlacement, ...fallbackPlacements];
      const overflow = await detectOverflow(state, detectOverflowOptions);
      const overflows = [];
      let overflowsData = ((_middlewareData$flip = middlewareData.flip) == null ? void 0 : _middlewareData$flip.overflows) || [];
      if (checkMainAxis) {
        overflows.push(overflow[side]);
      }
      if (checkCrossAxis) {
        const sides2 = getAlignmentSides(placement, rects, rtl);
        overflows.push(overflow[sides2[0]], overflow[sides2[1]]);
      }
      overflowsData = [...overflowsData, {
        placement,
        overflows
      }];
      if (!overflows.every((side2) => side2 <= 0)) {
        var _middlewareData$flip2, _overflowsData$filter;
        const nextIndex = (((_middlewareData$flip2 = middlewareData.flip) == null ? void 0 : _middlewareData$flip2.index) || 0) + 1;
        const nextPlacement = placements2[nextIndex];
        if (nextPlacement) {
          var _overflowsData$;
          const ignoreCrossAxisOverflow = checkCrossAxis === "alignment" ? initialSideAxis !== getSideAxis(nextPlacement) : false;
          const hasInitialMainAxisOverflow = ((_overflowsData$ = overflowsData[0]) == null ? void 0 : _overflowsData$.overflows[0]) > 0;
          if (!ignoreCrossAxisOverflow || hasInitialMainAxisOverflow) {
            return {
              data: {
                index: nextIndex,
                overflows: overflowsData
              },
              reset: {
                placement: nextPlacement
              }
            };
          }
        }
        let resetPlacement = (_overflowsData$filter = overflowsData.filter((d3) => d3.overflows[0] <= 0).sort((a4, b3) => a4.overflows[1] - b3.overflows[1])[0]) == null ? void 0 : _overflowsData$filter.placement;
        if (!resetPlacement) {
          switch (fallbackStrategy) {
            case "bestFit": {
              var _overflowsData$filter2;
              const placement2 = (_overflowsData$filter2 = overflowsData.filter((d3) => {
                if (hasFallbackAxisSideDirection) {
                  const currentSideAxis = getSideAxis(d3.placement);
                  return currentSideAxis === initialSideAxis || currentSideAxis === "y";
                }
                return true;
              }).map((d3) => [d3.placement, d3.overflows.filter((overflow2) => overflow2 > 0).reduce((acc, overflow2) => acc + overflow2, 0)]).sort((a4, b3) => a4[1] - b3[1])[0]) == null ? void 0 : _overflowsData$filter2[0];
              if (placement2) {
                resetPlacement = placement2;
              }
              break;
            }
            case "initialPlacement":
              resetPlacement = initialPlacement;
              break;
          }
        }
        if (placement !== resetPlacement) {
          return {
            reset: {
              placement: resetPlacement
            }
          };
        }
      }
      return {};
    }
  };
};
async function convertValueToCoords(state, options) {
  const {
    placement,
    platform: platform2,
    elements
  } = state;
  const rtl = await (platform2.isRTL == null ? void 0 : platform2.isRTL(elements.floating));
  const side = getSide(placement);
  const alignment = getAlignment(placement);
  const isVertical = getSideAxis(placement) === "y";
  const mainAxisMulti = ["left", "top"].includes(side) ? -1 : 1;
  const crossAxisMulti = rtl && isVertical ? -1 : 1;
  const rawValue = evaluate(options, state);
  let {
    mainAxis,
    crossAxis,
    alignmentAxis
  } = typeof rawValue === "number" ? {
    mainAxis: rawValue,
    crossAxis: 0,
    alignmentAxis: null
  } : {
    mainAxis: rawValue.mainAxis || 0,
    crossAxis: rawValue.crossAxis || 0,
    alignmentAxis: rawValue.alignmentAxis
  };
  if (alignment && typeof alignmentAxis === "number") {
    crossAxis = alignment === "end" ? alignmentAxis * -1 : alignmentAxis;
  }
  return isVertical ? {
    x: crossAxis * crossAxisMulti,
    y: mainAxis * mainAxisMulti
  } : {
    x: mainAxis * mainAxisMulti,
    y: crossAxis * crossAxisMulti
  };
}
var offset = function(options) {
  if (options === void 0) {
    options = 0;
  }
  return {
    name: "offset",
    options,
    async fn(state) {
      var _middlewareData$offse, _middlewareData$arrow;
      const {
        x: x2,
        y: y3,
        placement,
        middlewareData
      } = state;
      const diffCoords = await convertValueToCoords(state, options);
      if (placement === ((_middlewareData$offse = middlewareData.offset) == null ? void 0 : _middlewareData$offse.placement) && (_middlewareData$arrow = middlewareData.arrow) != null && _middlewareData$arrow.alignmentOffset) {
        return {};
      }
      return {
        x: x2 + diffCoords.x,
        y: y3 + diffCoords.y,
        data: {
          ...diffCoords,
          placement
        }
      };
    }
  };
};
var shift = function(options) {
  if (options === void 0) {
    options = {};
  }
  return {
    name: "shift",
    options,
    async fn(state) {
      const {
        x: x2,
        y: y3,
        placement
      } = state;
      const {
        mainAxis: checkMainAxis = true,
        crossAxis: checkCrossAxis = false,
        limiter = {
          fn: (_ref) => {
            let {
              x: x3,
              y: y22
            } = _ref;
            return {
              x: x3,
              y: y22
            };
          }
        },
        ...detectOverflowOptions
      } = evaluate(options, state);
      const coords = {
        x: x2,
        y: y3
      };
      const overflow = await detectOverflow(state, detectOverflowOptions);
      const crossAxis = getSideAxis(getSide(placement));
      const mainAxis = getOppositeAxis(crossAxis);
      let mainAxisCoord = coords[mainAxis];
      let crossAxisCoord = coords[crossAxis];
      if (checkMainAxis) {
        const minSide = mainAxis === "y" ? "top" : "left";
        const maxSide = mainAxis === "y" ? "bottom" : "right";
        const min2 = mainAxisCoord + overflow[minSide];
        const max2 = mainAxisCoord - overflow[maxSide];
        mainAxisCoord = clamp2(min2, mainAxisCoord, max2);
      }
      if (checkCrossAxis) {
        const minSide = crossAxis === "y" ? "top" : "left";
        const maxSide = crossAxis === "y" ? "bottom" : "right";
        const min2 = crossAxisCoord + overflow[minSide];
        const max2 = crossAxisCoord - overflow[maxSide];
        crossAxisCoord = clamp2(min2, crossAxisCoord, max2);
      }
      const limitedCoords = limiter.fn({
        ...state,
        [mainAxis]: mainAxisCoord,
        [crossAxis]: crossAxisCoord
      });
      return {
        ...limitedCoords,
        data: {
          x: limitedCoords.x - x2,
          y: limitedCoords.y - y3,
          enabled: {
            [mainAxis]: checkMainAxis,
            [crossAxis]: checkCrossAxis
          }
        }
      };
    }
  };
};
var size = function(options) {
  if (options === void 0) {
    options = {};
  }
  return {
    name: "size",
    options,
    async fn(state) {
      var _state$middlewareData, _state$middlewareData2;
      const {
        placement,
        rects,
        platform: platform2,
        elements
      } = state;
      const {
        apply = () => {
        },
        ...detectOverflowOptions
      } = evaluate(options, state);
      const overflow = await detectOverflow(state, detectOverflowOptions);
      const side = getSide(placement);
      const alignment = getAlignment(placement);
      const isYAxis = getSideAxis(placement) === "y";
      const {
        width,
        height
      } = rects.floating;
      let heightSide;
      let widthSide;
      if (side === "top" || side === "bottom") {
        heightSide = side;
        widthSide = alignment === (await (platform2.isRTL == null ? void 0 : platform2.isRTL(elements.floating)) ? "start" : "end") ? "left" : "right";
      } else {
        widthSide = side;
        heightSide = alignment === "end" ? "top" : "bottom";
      }
      const maximumClippingHeight = height - overflow.top - overflow.bottom;
      const maximumClippingWidth = width - overflow.left - overflow.right;
      const overflowAvailableHeight = min(height - overflow[heightSide], maximumClippingHeight);
      const overflowAvailableWidth = min(width - overflow[widthSide], maximumClippingWidth);
      const noShift = !state.middlewareData.shift;
      let availableHeight = overflowAvailableHeight;
      let availableWidth = overflowAvailableWidth;
      if ((_state$middlewareData = state.middlewareData.shift) != null && _state$middlewareData.enabled.x) {
        availableWidth = maximumClippingWidth;
      }
      if ((_state$middlewareData2 = state.middlewareData.shift) != null && _state$middlewareData2.enabled.y) {
        availableHeight = maximumClippingHeight;
      }
      if (noShift && !alignment) {
        const xMin = max(overflow.left, 0);
        const xMax = max(overflow.right, 0);
        const yMin = max(overflow.top, 0);
        const yMax = max(overflow.bottom, 0);
        if (isYAxis) {
          availableWidth = width - 2 * (xMin !== 0 || xMax !== 0 ? xMin + xMax : max(overflow.left, overflow.right));
        } else {
          availableHeight = height - 2 * (yMin !== 0 || yMax !== 0 ? yMin + yMax : max(overflow.top, overflow.bottom));
        }
      }
      await apply({
        ...state,
        availableWidth,
        availableHeight
      });
      const nextDimensions = await platform2.getDimensions(elements.floating);
      if (width !== nextDimensions.width || height !== nextDimensions.height) {
        return {
          reset: {
            rects: true
          }
        };
      }
      return {};
    }
  };
};
function hasWindow() {
  return typeof window !== "undefined";
}
function getNodeName(node) {
  if (isNode(node)) {
    return (node.nodeName || "").toLowerCase();
  }
  return "#document";
}
function getWindow(node) {
  var _node$ownerDocument;
  return (node == null || (_node$ownerDocument = node.ownerDocument) == null ? void 0 : _node$ownerDocument.defaultView) || window;
}
function getDocumentElement(node) {
  var _ref;
  return (_ref = (isNode(node) ? node.ownerDocument : node.document) || window.document) == null ? void 0 : _ref.documentElement;
}
function isNode(value) {
  if (!hasWindow()) {
    return false;
  }
  return value instanceof Node || value instanceof getWindow(value).Node;
}
function isElement(value) {
  if (!hasWindow()) {
    return false;
  }
  return value instanceof Element || value instanceof getWindow(value).Element;
}
function isHTMLElement(value) {
  if (!hasWindow()) {
    return false;
  }
  return value instanceof HTMLElement || value instanceof getWindow(value).HTMLElement;
}
function isShadowRoot(value) {
  if (!hasWindow() || typeof ShadowRoot === "undefined") {
    return false;
  }
  return value instanceof ShadowRoot || value instanceof getWindow(value).ShadowRoot;
}
function isOverflowElement(element) {
  const {
    overflow,
    overflowX,
    overflowY,
    display
  } = getComputedStyle2(element);
  return /auto|scroll|overlay|hidden|clip/.test(overflow + overflowY + overflowX) && !["inline", "contents"].includes(display);
}
function isTableElement(element) {
  return ["table", "td", "th"].includes(getNodeName(element));
}
function isTopLayer(element) {
  return [":popover-open", ":modal"].some((selector) => {
    try {
      return element.matches(selector);
    } catch (e42) {
      return false;
    }
  });
}
function isContainingBlock(elementOrCss) {
  const webkit = isWebKit();
  const css = isElement(elementOrCss) ? getComputedStyle2(elementOrCss) : elementOrCss;
  return ["transform", "translate", "scale", "rotate", "perspective"].some((value) => css[value] ? css[value] !== "none" : false) || (css.containerType ? css.containerType !== "normal" : false) || !webkit && (css.backdropFilter ? css.backdropFilter !== "none" : false) || !webkit && (css.filter ? css.filter !== "none" : false) || ["transform", "translate", "scale", "rotate", "perspective", "filter"].some((value) => (css.willChange || "").includes(value)) || ["paint", "layout", "strict", "content"].some((value) => (css.contain || "").includes(value));
}
function getContainingBlock(element) {
  let currentNode = getParentNode(element);
  while (isHTMLElement(currentNode) && !isLastTraversableNode(currentNode)) {
    if (isContainingBlock(currentNode)) {
      return currentNode;
    } else if (isTopLayer(currentNode)) {
      return null;
    }
    currentNode = getParentNode(currentNode);
  }
  return null;
}
function isWebKit() {
  if (typeof CSS === "undefined" || !CSS.supports)
    return false;
  return CSS.supports("-webkit-backdrop-filter", "none");
}
function isLastTraversableNode(node) {
  return ["html", "body", "#document"].includes(getNodeName(node));
}
function getComputedStyle2(element) {
  return getWindow(element).getComputedStyle(element);
}
function getNodeScroll(element) {
  if (isElement(element)) {
    return {
      scrollLeft: element.scrollLeft,
      scrollTop: element.scrollTop
    };
  }
  return {
    scrollLeft: element.scrollX,
    scrollTop: element.scrollY
  };
}
function getParentNode(node) {
  if (getNodeName(node) === "html") {
    return node;
  }
  const result = node.assignedSlot || node.parentNode || isShadowRoot(node) && node.host || getDocumentElement(node);
  return isShadowRoot(result) ? result.host : result;
}
function getNearestOverflowAncestor(node) {
  const parentNode = getParentNode(node);
  if (isLastTraversableNode(parentNode)) {
    return node.ownerDocument ? node.ownerDocument.body : node.body;
  }
  if (isHTMLElement(parentNode) && isOverflowElement(parentNode)) {
    return parentNode;
  }
  return getNearestOverflowAncestor(parentNode);
}
function getOverflowAncestors(node, list, traverseIframes) {
  var _node$ownerDocument2;
  if (list === void 0) {
    list = [];
  }
  if (traverseIframes === void 0) {
    traverseIframes = true;
  }
  const scrollableAncestor = getNearestOverflowAncestor(node);
  const isBody = scrollableAncestor === ((_node$ownerDocument2 = node.ownerDocument) == null ? void 0 : _node$ownerDocument2.body);
  const win = getWindow(scrollableAncestor);
  if (isBody) {
    const frameElement = getFrameElement(win);
    return list.concat(win, win.visualViewport || [], isOverflowElement(scrollableAncestor) ? scrollableAncestor : [], frameElement && traverseIframes ? getOverflowAncestors(frameElement) : []);
  }
  return list.concat(scrollableAncestor, getOverflowAncestors(scrollableAncestor, [], traverseIframes));
}
function getFrameElement(win) {
  return win.parent && Object.getPrototypeOf(win.parent) ? win.frameElement : null;
}
function getCssDimensions(element) {
  const css = getComputedStyle2(element);
  let width = parseFloat(css.width) || 0;
  let height = parseFloat(css.height) || 0;
  const hasOffset = isHTMLElement(element);
  const offsetWidth = hasOffset ? element.offsetWidth : width;
  const offsetHeight = hasOffset ? element.offsetHeight : height;
  const shouldFallback = round(width) !== offsetWidth || round(height) !== offsetHeight;
  if (shouldFallback) {
    width = offsetWidth;
    height = offsetHeight;
  }
  return {
    width,
    height,
    $: shouldFallback
  };
}
function unwrapElement(element) {
  return !isElement(element) ? element.contextElement : element;
}
function getScale(element) {
  const domElement = unwrapElement(element);
  if (!isHTMLElement(domElement)) {
    return createCoords(1);
  }
  const rect = domElement.getBoundingClientRect();
  const {
    width,
    height,
    $: $3
  } = getCssDimensions(domElement);
  let x2 = ($3 ? round(rect.width) : rect.width) / width;
  let y3 = ($3 ? round(rect.height) : rect.height) / height;
  if (!x2 || !Number.isFinite(x2)) {
    x2 = 1;
  }
  if (!y3 || !Number.isFinite(y3)) {
    y3 = 1;
  }
  return {
    x: x2,
    y: y3
  };
}
var noOffsets = /* @__PURE__ */ createCoords(0);
function getVisualOffsets(element) {
  const win = getWindow(element);
  if (!isWebKit() || !win.visualViewport) {
    return noOffsets;
  }
  return {
    x: win.visualViewport.offsetLeft,
    y: win.visualViewport.offsetTop
  };
}
function shouldAddVisualOffsets(element, isFixed, floatingOffsetParent) {
  if (isFixed === void 0) {
    isFixed = false;
  }
  if (!floatingOffsetParent || isFixed && floatingOffsetParent !== getWindow(element)) {
    return false;
  }
  return isFixed;
}
function getBoundingClientRect(element, includeScale, isFixedStrategy, offsetParent) {
  if (includeScale === void 0) {
    includeScale = false;
  }
  if (isFixedStrategy === void 0) {
    isFixedStrategy = false;
  }
  const clientRect = element.getBoundingClientRect();
  const domElement = unwrapElement(element);
  let scale = createCoords(1);
  if (includeScale) {
    if (offsetParent) {
      if (isElement(offsetParent)) {
        scale = getScale(offsetParent);
      }
    } else {
      scale = getScale(element);
    }
  }
  const visualOffsets = shouldAddVisualOffsets(domElement, isFixedStrategy, offsetParent) ? getVisualOffsets(domElement) : createCoords(0);
  let x2 = (clientRect.left + visualOffsets.x) / scale.x;
  let y3 = (clientRect.top + visualOffsets.y) / scale.y;
  let width = clientRect.width / scale.x;
  let height = clientRect.height / scale.y;
  if (domElement) {
    const win = getWindow(domElement);
    const offsetWin = offsetParent && isElement(offsetParent) ? getWindow(offsetParent) : offsetParent;
    let currentWin = win;
    let currentIFrame = getFrameElement(currentWin);
    while (currentIFrame && offsetParent && offsetWin !== currentWin) {
      const iframeScale = getScale(currentIFrame);
      const iframeRect = currentIFrame.getBoundingClientRect();
      const css = getComputedStyle2(currentIFrame);
      const left = iframeRect.left + (currentIFrame.clientLeft + parseFloat(css.paddingLeft)) * iframeScale.x;
      const top = iframeRect.top + (currentIFrame.clientTop + parseFloat(css.paddingTop)) * iframeScale.y;
      x2 *= iframeScale.x;
      y3 *= iframeScale.y;
      width *= iframeScale.x;
      height *= iframeScale.y;
      x2 += left;
      y3 += top;
      currentWin = getWindow(currentIFrame);
      currentIFrame = getFrameElement(currentWin);
    }
  }
  return rectToClientRect({
    width,
    height,
    x: x2,
    y: y3
  });
}
function getWindowScrollBarX(element, rect) {
  const leftScroll = getNodeScroll(element).scrollLeft;
  if (!rect) {
    return getBoundingClientRect(getDocumentElement(element)).left + leftScroll;
  }
  return rect.left + leftScroll;
}
function getHTMLOffset(documentElement, scroll, ignoreScrollbarX) {
  if (ignoreScrollbarX === void 0) {
    ignoreScrollbarX = false;
  }
  const htmlRect = documentElement.getBoundingClientRect();
  const x2 = htmlRect.left + scroll.scrollLeft - (ignoreScrollbarX ? 0 : getWindowScrollBarX(documentElement, htmlRect));
  const y3 = htmlRect.top + scroll.scrollTop;
  return {
    x: x2,
    y: y3
  };
}
function convertOffsetParentRelativeRectToViewportRelativeRect(_ref) {
  let {
    elements,
    rect,
    offsetParent,
    strategy
  } = _ref;
  const isFixed = strategy === "fixed";
  const documentElement = getDocumentElement(offsetParent);
  const topLayer = elements ? isTopLayer(elements.floating) : false;
  if (offsetParent === documentElement || topLayer && isFixed) {
    return rect;
  }
  let scroll = {
    scrollLeft: 0,
    scrollTop: 0
  };
  let scale = createCoords(1);
  const offsets = createCoords(0);
  const isOffsetParentAnElement = isHTMLElement(offsetParent);
  if (isOffsetParentAnElement || !isOffsetParentAnElement && !isFixed) {
    if (getNodeName(offsetParent) !== "body" || isOverflowElement(documentElement)) {
      scroll = getNodeScroll(offsetParent);
    }
    if (isHTMLElement(offsetParent)) {
      const offsetRect = getBoundingClientRect(offsetParent);
      scale = getScale(offsetParent);
      offsets.x = offsetRect.x + offsetParent.clientLeft;
      offsets.y = offsetRect.y + offsetParent.clientTop;
    }
  }
  const htmlOffset = documentElement && !isOffsetParentAnElement && !isFixed ? getHTMLOffset(documentElement, scroll, true) : createCoords(0);
  return {
    width: rect.width * scale.x,
    height: rect.height * scale.y,
    x: rect.x * scale.x - scroll.scrollLeft * scale.x + offsets.x + htmlOffset.x,
    y: rect.y * scale.y - scroll.scrollTop * scale.y + offsets.y + htmlOffset.y
  };
}
function getClientRects(element) {
  return Array.from(element.getClientRects());
}
function getDocumentRect(element) {
  const html = getDocumentElement(element);
  const scroll = getNodeScroll(element);
  const body = element.ownerDocument.body;
  const width = max(html.scrollWidth, html.clientWidth, body.scrollWidth, body.clientWidth);
  const height = max(html.scrollHeight, html.clientHeight, body.scrollHeight, body.clientHeight);
  let x2 = -scroll.scrollLeft + getWindowScrollBarX(element);
  const y3 = -scroll.scrollTop;
  if (getComputedStyle2(body).direction === "rtl") {
    x2 += max(html.clientWidth, body.clientWidth) - width;
  }
  return {
    width,
    height,
    x: x2,
    y: y3
  };
}
function getViewportRect(element, strategy) {
  const win = getWindow(element);
  const html = getDocumentElement(element);
  const visualViewport = win.visualViewport;
  let width = html.clientWidth;
  let height = html.clientHeight;
  let x2 = 0;
  let y3 = 0;
  if (visualViewport) {
    width = visualViewport.width;
    height = visualViewport.height;
    const visualViewportBased = isWebKit();
    if (!visualViewportBased || visualViewportBased && strategy === "fixed") {
      x2 = visualViewport.offsetLeft;
      y3 = visualViewport.offsetTop;
    }
  }
  return {
    width,
    height,
    x: x2,
    y: y3
  };
}
function getInnerBoundingClientRect(element, strategy) {
  const clientRect = getBoundingClientRect(element, true, strategy === "fixed");
  const top = clientRect.top + element.clientTop;
  const left = clientRect.left + element.clientLeft;
  const scale = isHTMLElement(element) ? getScale(element) : createCoords(1);
  const width = element.clientWidth * scale.x;
  const height = element.clientHeight * scale.y;
  const x2 = left * scale.x;
  const y3 = top * scale.y;
  return {
    width,
    height,
    x: x2,
    y: y3
  };
}
function getClientRectFromClippingAncestor(element, clippingAncestor, strategy) {
  let rect;
  if (clippingAncestor === "viewport") {
    rect = getViewportRect(element, strategy);
  } else if (clippingAncestor === "document") {
    rect = getDocumentRect(getDocumentElement(element));
  } else if (isElement(clippingAncestor)) {
    rect = getInnerBoundingClientRect(clippingAncestor, strategy);
  } else {
    const visualOffsets = getVisualOffsets(element);
    rect = {
      x: clippingAncestor.x - visualOffsets.x,
      y: clippingAncestor.y - visualOffsets.y,
      width: clippingAncestor.width,
      height: clippingAncestor.height
    };
  }
  return rectToClientRect(rect);
}
function hasFixedPositionAncestor(element, stopNode) {
  const parentNode = getParentNode(element);
  if (parentNode === stopNode || !isElement(parentNode) || isLastTraversableNode(parentNode)) {
    return false;
  }
  return getComputedStyle2(parentNode).position === "fixed" || hasFixedPositionAncestor(parentNode, stopNode);
}
function getClippingElementAncestors(element, cache) {
  const cachedResult = cache.get(element);
  if (cachedResult) {
    return cachedResult;
  }
  let result = getOverflowAncestors(element, [], false).filter((el) => isElement(el) && getNodeName(el) !== "body");
  let currentContainingBlockComputedStyle = null;
  const elementIsFixed = getComputedStyle2(element).position === "fixed";
  let currentNode = elementIsFixed ? getParentNode(element) : element;
  while (isElement(currentNode) && !isLastTraversableNode(currentNode)) {
    const computedStyle = getComputedStyle2(currentNode);
    const currentNodeIsContaining = isContainingBlock(currentNode);
    if (!currentNodeIsContaining && computedStyle.position === "fixed") {
      currentContainingBlockComputedStyle = null;
    }
    const shouldDropCurrentNode = elementIsFixed ? !currentNodeIsContaining && !currentContainingBlockComputedStyle : !currentNodeIsContaining && computedStyle.position === "static" && !!currentContainingBlockComputedStyle && ["absolute", "fixed"].includes(currentContainingBlockComputedStyle.position) || isOverflowElement(currentNode) && !currentNodeIsContaining && hasFixedPositionAncestor(element, currentNode);
    if (shouldDropCurrentNode) {
      result = result.filter((ancestor) => ancestor !== currentNode);
    } else {
      currentContainingBlockComputedStyle = computedStyle;
    }
    currentNode = getParentNode(currentNode);
  }
  cache.set(element, result);
  return result;
}
function getClippingRect(_ref) {
  let {
    element,
    boundary,
    rootBoundary,
    strategy
  } = _ref;
  const elementClippingAncestors = boundary === "clippingAncestors" ? isTopLayer(element) ? [] : getClippingElementAncestors(element, this._c) : [].concat(boundary);
  const clippingAncestors = [...elementClippingAncestors, rootBoundary];
  const firstClippingAncestor = clippingAncestors[0];
  const clippingRect = clippingAncestors.reduce((accRect, clippingAncestor) => {
    const rect = getClientRectFromClippingAncestor(element, clippingAncestor, strategy);
    accRect.top = max(rect.top, accRect.top);
    accRect.right = min(rect.right, accRect.right);
    accRect.bottom = min(rect.bottom, accRect.bottom);
    accRect.left = max(rect.left, accRect.left);
    return accRect;
  }, getClientRectFromClippingAncestor(element, firstClippingAncestor, strategy));
  return {
    width: clippingRect.right - clippingRect.left,
    height: clippingRect.bottom - clippingRect.top,
    x: clippingRect.left,
    y: clippingRect.top
  };
}
function getDimensions(element) {
  const {
    width,
    height
  } = getCssDimensions(element);
  return {
    width,
    height
  };
}
function getRectRelativeToOffsetParent(element, offsetParent, strategy) {
  const isOffsetParentAnElement = isHTMLElement(offsetParent);
  const documentElement = getDocumentElement(offsetParent);
  const isFixed = strategy === "fixed";
  const rect = getBoundingClientRect(element, true, isFixed, offsetParent);
  let scroll = {
    scrollLeft: 0,
    scrollTop: 0
  };
  const offsets = createCoords(0);
  function setLeftRTLScrollbarOffset() {
    offsets.x = getWindowScrollBarX(documentElement);
  }
  if (isOffsetParentAnElement || !isOffsetParentAnElement && !isFixed) {
    if (getNodeName(offsetParent) !== "body" || isOverflowElement(documentElement)) {
      scroll = getNodeScroll(offsetParent);
    }
    if (isOffsetParentAnElement) {
      const offsetRect = getBoundingClientRect(offsetParent, true, isFixed, offsetParent);
      offsets.x = offsetRect.x + offsetParent.clientLeft;
      offsets.y = offsetRect.y + offsetParent.clientTop;
    } else if (documentElement) {
      setLeftRTLScrollbarOffset();
    }
  }
  if (isFixed && !isOffsetParentAnElement && documentElement) {
    setLeftRTLScrollbarOffset();
  }
  const htmlOffset = documentElement && !isOffsetParentAnElement && !isFixed ? getHTMLOffset(documentElement, scroll) : createCoords(0);
  const x2 = rect.left + scroll.scrollLeft - offsets.x - htmlOffset.x;
  const y3 = rect.top + scroll.scrollTop - offsets.y - htmlOffset.y;
  return {
    x: x2,
    y: y3,
    width: rect.width,
    height: rect.height
  };
}
function isStaticPositioned(element) {
  return getComputedStyle2(element).position === "static";
}
function getTrueOffsetParent(element, polyfill) {
  if (!isHTMLElement(element) || getComputedStyle2(element).position === "fixed") {
    return null;
  }
  if (polyfill) {
    return polyfill(element);
  }
  let rawOffsetParent = element.offsetParent;
  if (getDocumentElement(element) === rawOffsetParent) {
    rawOffsetParent = rawOffsetParent.ownerDocument.body;
  }
  return rawOffsetParent;
}
function getOffsetParent(element, polyfill) {
  const win = getWindow(element);
  if (isTopLayer(element)) {
    return win;
  }
  if (!isHTMLElement(element)) {
    let svgOffsetParent = getParentNode(element);
    while (svgOffsetParent && !isLastTraversableNode(svgOffsetParent)) {
      if (isElement(svgOffsetParent) && !isStaticPositioned(svgOffsetParent)) {
        return svgOffsetParent;
      }
      svgOffsetParent = getParentNode(svgOffsetParent);
    }
    return win;
  }
  let offsetParent = getTrueOffsetParent(element, polyfill);
  while (offsetParent && isTableElement(offsetParent) && isStaticPositioned(offsetParent)) {
    offsetParent = getTrueOffsetParent(offsetParent, polyfill);
  }
  if (offsetParent && isLastTraversableNode(offsetParent) && isStaticPositioned(offsetParent) && !isContainingBlock(offsetParent)) {
    return win;
  }
  return offsetParent || getContainingBlock(element) || win;
}
var getElementRects = async function(data) {
  const getOffsetParentFn = this.getOffsetParent || getOffsetParent;
  const getDimensionsFn = this.getDimensions;
  const floatingDimensions = await getDimensionsFn(data.floating);
  return {
    reference: getRectRelativeToOffsetParent(data.reference, await getOffsetParentFn(data.floating), data.strategy),
    floating: {
      x: 0,
      y: 0,
      width: floatingDimensions.width,
      height: floatingDimensions.height
    }
  };
};
function isRTL(element) {
  return getComputedStyle2(element).direction === "rtl";
}
var platform = {
  convertOffsetParentRelativeRectToViewportRelativeRect,
  getDocumentElement,
  getClippingRect,
  getOffsetParent,
  getElementRects,
  getClientRects,
  getDimensions,
  getScale,
  isElement,
  isRTL
};
function rectsAreEqual(a4, b3) {
  return a4.x === b3.x && a4.y === b3.y && a4.width === b3.width && a4.height === b3.height;
}
function observeMove(element, onMove) {
  let io = null;
  let timeoutId;
  const root = getDocumentElement(element);
  function cleanup() {
    var _io;
    clearTimeout(timeoutId);
    (_io = io) == null || _io.disconnect();
    io = null;
  }
  function refresh(skip, threshold) {
    if (skip === void 0) {
      skip = false;
    }
    if (threshold === void 0) {
      threshold = 1;
    }
    cleanup();
    const elementRectForRootMargin = element.getBoundingClientRect();
    const {
      left,
      top,
      width,
      height
    } = elementRectForRootMargin;
    if (!skip) {
      onMove();
    }
    if (!width || !height) {
      return;
    }
    const insetTop = floor(top);
    const insetRight = floor(root.clientWidth - (left + width));
    const insetBottom = floor(root.clientHeight - (top + height));
    const insetLeft = floor(left);
    const rootMargin = -insetTop + "px " + -insetRight + "px " + -insetBottom + "px " + -insetLeft + "px";
    const options = {
      rootMargin,
      threshold: max(0, min(1, threshold)) || 1
    };
    let isFirstUpdate = true;
    function handleObserve(entries) {
      const ratio = entries[0].intersectionRatio;
      if (ratio !== threshold) {
        if (!isFirstUpdate) {
          return refresh();
        }
        if (!ratio) {
          timeoutId = setTimeout(() => {
            refresh(false, 1e-7);
          }, 1e3);
        } else {
          refresh(false, ratio);
        }
      }
      if (ratio === 1 && !rectsAreEqual(elementRectForRootMargin, element.getBoundingClientRect())) {
        refresh();
      }
      isFirstUpdate = false;
    }
    try {
      io = new IntersectionObserver(handleObserve, {
        ...options,
        root: root.ownerDocument
      });
    } catch (_e) {
      io = new IntersectionObserver(handleObserve, options);
    }
    io.observe(element);
  }
  refresh(true);
  return cleanup;
}
function autoUpdate(reference, floating, update2, options) {
  if (options === void 0) {
    options = {};
  }
  const {
    ancestorScroll = true,
    ancestorResize = true,
    elementResize = typeof ResizeObserver === "function",
    layoutShift = typeof IntersectionObserver === "function",
    animationFrame = false
  } = options;
  const referenceEl = unwrapElement(reference);
  const ancestors = ancestorScroll || ancestorResize ? [...referenceEl ? getOverflowAncestors(referenceEl) : [], ...getOverflowAncestors(floating)] : [];
  ancestors.forEach((ancestor) => {
    ancestorScroll && ancestor.addEventListener("scroll", update2, {
      passive: true
    });
    ancestorResize && ancestor.addEventListener("resize", update2);
  });
  const cleanupIo = referenceEl && layoutShift ? observeMove(referenceEl, update2) : null;
  let reobserveFrame = -1;
  let resizeObserver = null;
  if (elementResize) {
    resizeObserver = new ResizeObserver((_ref) => {
      let [firstEntry] = _ref;
      if (firstEntry && firstEntry.target === referenceEl && resizeObserver) {
        resizeObserver.unobserve(floating);
        cancelAnimationFrame(reobserveFrame);
        reobserveFrame = requestAnimationFrame(() => {
          var _resizeObserver;
          (_resizeObserver = resizeObserver) == null || _resizeObserver.observe(floating);
        });
      }
      update2();
    });
    if (referenceEl && !animationFrame) {
      resizeObserver.observe(referenceEl);
    }
    resizeObserver.observe(floating);
  }
  let frameId;
  let prevRefRect = animationFrame ? getBoundingClientRect(reference) : null;
  if (animationFrame) {
    frameLoop();
  }
  function frameLoop() {
    const nextRefRect = getBoundingClientRect(reference);
    if (prevRefRect && !rectsAreEqual(prevRefRect, nextRefRect)) {
      update2();
    }
    prevRefRect = nextRefRect;
    frameId = requestAnimationFrame(frameLoop);
  }
  update2();
  return () => {
    var _resizeObserver2;
    ancestors.forEach((ancestor) => {
      ancestorScroll && ancestor.removeEventListener("scroll", update2);
      ancestorResize && ancestor.removeEventListener("resize", update2);
    });
    cleanupIo == null || cleanupIo();
    (_resizeObserver2 = resizeObserver) == null || _resizeObserver2.disconnect();
    resizeObserver = null;
    if (animationFrame) {
      cancelAnimationFrame(frameId);
    }
  };
}
var offset2 = offset;
var shift2 = shift;
var flip2 = flip;
var size2 = size;
var arrow2 = arrow;
var computePosition2 = (reference, floating, options) => {
  const cache = /* @__PURE__ */ new Map();
  const mergedOptions = {
    platform,
    ...options
  };
  const platformWithCache = {
    ...mergedOptions.platform,
    _c: cache
  };
  return computePosition(reference, floating, {
    ...mergedOptions,
    platform: platformWithCache
  });
};
function e32(t23) {
  return i6(t23);
}
function r4(t23) {
  return t23.assignedSlot ? t23.assignedSlot : t23.parentNode instanceof ShadowRoot ? t23.parentNode.host : t23.parentNode;
}
function i6(e42) {
  for (let t23 = e42; t23; t23 = r4(t23))
    if (t23 instanceof Element && getComputedStyle(t23).display === "none")
      return null;
  for (let n24 = r4(e42); n24; n24 = r4(n24)) {
    if (!(n24 instanceof Element))
      continue;
    const e52 = getComputedStyle(n24);
    if (e52.display !== "contents") {
      if (e52.position !== "static" || isContainingBlock(e52))
        return n24;
      if (n24.tagName === "BODY")
        return n24;
    }
  }
  return null;
}
function isVirtualElement(e42) {
  return e42 !== null && typeof e42 === "object" && "getBoundingClientRect" in e42 && ("contextElement" in e42 ? e42 instanceof Element : true);
}
var SUPPORTS_POPOVER = globalThis?.HTMLElement?.prototype.hasOwnProperty("popover");
var WaPopup = class extends WebAwesomeElement {
  constructor() {
    super(...arguments);
    this.localize = new LocalizeController2(this);
    this.active = false;
    this.placement = "top";
    this.boundary = "viewport";
    this.distance = 0;
    this.skidding = 0;
    this.arrow = false;
    this.arrowPlacement = "anchor";
    this.arrowPadding = 10;
    this.flip = false;
    this.flipFallbackPlacements = "";
    this.flipFallbackStrategy = "best-fit";
    this.flipPadding = 0;
    this.shift = false;
    this.shiftPadding = 0;
    this.autoSizePadding = 0;
    this.hoverBridge = false;
    this.updateHoverBridge = () => {
      if (this.hoverBridge && this.anchorEl && this.popup) {
        const anchorRect = this.anchorEl.getBoundingClientRect();
        const popupRect = this.popup.getBoundingClientRect();
        const isVertical = this.placement.includes("top") || this.placement.includes("bottom");
        let topLeftX = 0;
        let topLeftY = 0;
        let topRightX = 0;
        let topRightY = 0;
        let bottomLeftX = 0;
        let bottomLeftY = 0;
        let bottomRightX = 0;
        let bottomRightY = 0;
        if (isVertical) {
          if (anchorRect.top < popupRect.top) {
            topLeftX = anchorRect.left;
            topLeftY = anchorRect.bottom;
            topRightX = anchorRect.right;
            topRightY = anchorRect.bottom;
            bottomLeftX = popupRect.left;
            bottomLeftY = popupRect.top;
            bottomRightX = popupRect.right;
            bottomRightY = popupRect.top;
          } else {
            topLeftX = popupRect.left;
            topLeftY = popupRect.bottom;
            topRightX = popupRect.right;
            topRightY = popupRect.bottom;
            bottomLeftX = anchorRect.left;
            bottomLeftY = anchorRect.top;
            bottomRightX = anchorRect.right;
            bottomRightY = anchorRect.top;
          }
        } else {
          if (anchorRect.left < popupRect.left) {
            topLeftX = anchorRect.right;
            topLeftY = anchorRect.top;
            topRightX = popupRect.left;
            topRightY = popupRect.top;
            bottomLeftX = anchorRect.right;
            bottomLeftY = anchorRect.bottom;
            bottomRightX = popupRect.left;
            bottomRightY = popupRect.bottom;
          } else {
            topLeftX = popupRect.right;
            topLeftY = popupRect.top;
            topRightX = anchorRect.left;
            topRightY = anchorRect.top;
            bottomLeftX = popupRect.right;
            bottomLeftY = popupRect.bottom;
            bottomRightX = anchorRect.left;
            bottomRightY = anchorRect.bottom;
          }
        }
        this.style.setProperty("--hover-bridge-top-left-x", `${topLeftX}px`);
        this.style.setProperty("--hover-bridge-top-left-y", `${topLeftY}px`);
        this.style.setProperty("--hover-bridge-top-right-x", `${topRightX}px`);
        this.style.setProperty("--hover-bridge-top-right-y", `${topRightY}px`);
        this.style.setProperty("--hover-bridge-bottom-left-x", `${bottomLeftX}px`);
        this.style.setProperty("--hover-bridge-bottom-left-y", `${bottomLeftY}px`);
        this.style.setProperty("--hover-bridge-bottom-right-x", `${bottomRightX}px`);
        this.style.setProperty("--hover-bridge-bottom-right-y", `${bottomRightY}px`);
      }
    };
  }
  async connectedCallback() {
    super.connectedCallback();
    await this.updateComplete;
    this.start();
  }
  disconnectedCallback() {
    super.disconnectedCallback();
    this.stop();
  }
  async updated(changedProperties) {
    super.updated(changedProperties);
    if (changedProperties.has("active")) {
      if (this.active) {
        this.start();
      } else {
        this.stop();
      }
    }
    if (changedProperties.has("anchor")) {
      this.handleAnchorChange();
    }
    if (this.active) {
      await this.updateComplete;
      this.reposition();
    }
  }
  async handleAnchorChange() {
    await this.stop();
    if (this.anchor && typeof this.anchor === "string") {
      const root = this.getRootNode();
      this.anchorEl = root.getElementById(this.anchor);
    } else if (this.anchor instanceof Element || isVirtualElement(this.anchor)) {
      this.anchorEl = this.anchor;
    } else {
      this.anchorEl = this.querySelector('[slot="anchor"]');
    }
    if (this.anchorEl instanceof HTMLSlotElement) {
      this.anchorEl = this.anchorEl.assignedElements({ flatten: true })[0];
    }
    if (this.anchorEl) {
      this.start();
    }
  }
  start() {
    if (!this.anchorEl || !this.active || !this.isConnected) {
      return;
    }
    this.popup?.showPopover?.();
    this.cleanup = autoUpdate(this.anchorEl, this.popup, () => {
      this.reposition();
    });
  }
  async stop() {
    return new Promise((resolve) => {
      this.popup?.hidePopover?.();
      if (this.cleanup) {
        this.cleanup();
        this.cleanup = void 0;
        this.removeAttribute("data-current-placement");
        this.style.removeProperty("--auto-size-available-width");
        this.style.removeProperty("--auto-size-available-height");
        requestAnimationFrame(() => resolve());
      } else {
        resolve();
      }
    });
  }
  reposition() {
    if (!this.active || !this.anchorEl || !this.popup) {
      return;
    }
    const middleware = [
      offset2({ mainAxis: this.distance, crossAxis: this.skidding })
    ];
    if (this.sync) {
      middleware.push(size2({
        apply: ({ rects }) => {
          const syncWidth = this.sync === "width" || this.sync === "both";
          const syncHeight = this.sync === "height" || this.sync === "both";
          this.popup.style.width = syncWidth ? `${rects.reference.width}px` : "";
          this.popup.style.height = syncHeight ? `${rects.reference.height}px` : "";
        }
      }));
    } else {
      this.popup.style.width = "";
      this.popup.style.height = "";
    }
    let defaultBoundary;
    if (SUPPORTS_POPOVER && !isVirtualElement(this.anchor) && this.boundary === "scroll") {
      defaultBoundary = getOverflowAncestors(this.anchorEl).filter((el) => el instanceof Element);
    }
    if (this.flip) {
      middleware.push(flip2({
        boundary: this.flipBoundary || defaultBoundary,
        fallbackPlacements: this.flipFallbackPlacements,
        fallbackStrategy: this.flipFallbackStrategy === "best-fit" ? "bestFit" : "initialPlacement",
        padding: this.flipPadding
      }));
    }
    if (this.shift) {
      middleware.push(shift2({
        boundary: this.shiftBoundary || defaultBoundary,
        padding: this.shiftPadding
      }));
    }
    if (this.autoSize) {
      middleware.push(size2({
        boundary: this.autoSizeBoundary || defaultBoundary,
        padding: this.autoSizePadding,
        apply: ({ availableWidth, availableHeight }) => {
          if (this.autoSize === "vertical" || this.autoSize === "both") {
            this.style.setProperty("--auto-size-available-height", `${availableHeight}px`);
          } else {
            this.style.removeProperty("--auto-size-available-height");
          }
          if (this.autoSize === "horizontal" || this.autoSize === "both") {
            this.style.setProperty("--auto-size-available-width", `${availableWidth}px`);
          } else {
            this.style.removeProperty("--auto-size-available-width");
          }
        }
      }));
    } else {
      this.style.removeProperty("--auto-size-available-width");
      this.style.removeProperty("--auto-size-available-height");
    }
    if (this.arrow) {
      middleware.push(arrow2({
        element: this.arrowEl,
        padding: this.arrowPadding
      }));
    }
    const getOffsetParent2 = SUPPORTS_POPOVER ? (element) => platform.getOffsetParent(element, e32) : platform.getOffsetParent;
    computePosition2(this.anchorEl, this.popup, {
      placement: this.placement,
      middleware,
      strategy: SUPPORTS_POPOVER ? "absolute" : "fixed",
      platform: {
        ...platform,
        getOffsetParent: getOffsetParent2
      }
    }).then(({ x: x2, y: y3, middlewareData, placement }) => {
      const isRtl = this.localize.dir() === "rtl";
      const staticSide = { top: "bottom", right: "left", bottom: "top", left: "right" }[placement.split("-")[0]];
      this.setAttribute("data-current-placement", placement);
      Object.assign(this.popup.style, {
        left: `${x2}px`,
        top: `${y3}px`
      });
      if (this.arrow) {
        const arrowX = middlewareData.arrow.x;
        const arrowY = middlewareData.arrow.y;
        let top = "";
        let right = "";
        let bottom = "";
        let left = "";
        if (this.arrowPlacement === "start") {
          const value = typeof arrowX === "number" ? `calc(${this.arrowPadding}px - var(--arrow-padding-offset))` : "";
          top = typeof arrowY === "number" ? `calc(${this.arrowPadding}px - var(--arrow-padding-offset))` : "";
          right = isRtl ? value : "";
          left = isRtl ? "" : value;
        } else if (this.arrowPlacement === "end") {
          const value = typeof arrowX === "number" ? `calc(${this.arrowPadding}px - var(--arrow-padding-offset))` : "";
          right = isRtl ? "" : value;
          left = isRtl ? value : "";
          bottom = typeof arrowY === "number" ? `calc(${this.arrowPadding}px - var(--arrow-padding-offset))` : "";
        } else if (this.arrowPlacement === "center") {
          left = typeof arrowX === "number" ? `calc(50% - var(--arrow-size-diagonal))` : "";
          top = typeof arrowY === "number" ? `calc(50% - var(--arrow-size-diagonal))` : "";
        } else {
          left = typeof arrowX === "number" ? `${arrowX}px` : "";
          top = typeof arrowY === "number" ? `${arrowY}px` : "";
        }
        Object.assign(this.arrowEl.style, {
          top,
          right,
          bottom,
          left,
          [staticSide]: "calc(var(--arrow-base-offset) - var(--arrow-size-diagonal))"
        });
      }
    });
    requestAnimationFrame(() => this.updateHoverBridge());
    this.dispatchEvent(new WaRepositionEvent());
  }
  render() {
    return x`
      <slot name="anchor" @slotchange=${this.handleAnchorChange}></slot>

      <span
        part="hover-bridge"
        class=${e24({
      "popup-hover-bridge": true,
      "popup-hover-bridge-visible": this.hoverBridge && this.active
    })}
      ></span>

      <div
        popover="manual"
        part="popup"
        class=${e24({
      popup: true,
      "popup-active": this.active,
      "popup-fixed": !SUPPORTS_POPOVER,
      "popup-has-arrow": this.arrow
    })}
      >
        <slot></slot>
        ${this.arrow ? x`<div part="arrow" class="arrow" role="presentation"></div>` : ""}
      </div>
    `;
  }
};
WaPopup.css = popup_styles_default;
__decorateClass([
  e23(".popup")
], WaPopup.prototype, "popup", 2);
__decorateClass([
  e23(".arrow")
], WaPopup.prototype, "arrowEl", 2);
__decorateClass([
  n3()
], WaPopup.prototype, "anchor", 2);
__decorateClass([
  n3({ type: Boolean, reflect: true })
], WaPopup.prototype, "active", 2);
__decorateClass([
  n3({ reflect: true })
], WaPopup.prototype, "placement", 2);
__decorateClass([
  n3()
], WaPopup.prototype, "boundary", 2);
__decorateClass([
  n3({ type: Number })
], WaPopup.prototype, "distance", 2);
__decorateClass([
  n3({ type: Number })
], WaPopup.prototype, "skidding", 2);
__decorateClass([
  n3({ type: Boolean })
], WaPopup.prototype, "arrow", 2);
__decorateClass([
  n3({ attribute: "arrow-placement" })
], WaPopup.prototype, "arrowPlacement", 2);
__decorateClass([
  n3({ attribute: "arrow-padding", type: Number })
], WaPopup.prototype, "arrowPadding", 2);
__decorateClass([
  n3({ type: Boolean })
], WaPopup.prototype, "flip", 2);
__decorateClass([
  n3({
    attribute: "flip-fallback-placements",
    converter: {
      fromAttribute: (value) => {
        return value.split(" ").map((p3) => p3.trim()).filter((p3) => p3 !== "");
      },
      toAttribute: (value) => {
        return value.join(" ");
      }
    }
  })
], WaPopup.prototype, "flipFallbackPlacements", 2);
__decorateClass([
  n3({ attribute: "flip-fallback-strategy" })
], WaPopup.prototype, "flipFallbackStrategy", 2);
__decorateClass([
  n3({ type: Object })
], WaPopup.prototype, "flipBoundary", 2);
__decorateClass([
  n3({ attribute: "flip-padding", type: Number })
], WaPopup.prototype, "flipPadding", 2);
__decorateClass([
  n3({ type: Boolean })
], WaPopup.prototype, "shift", 2);
__decorateClass([
  n3({ type: Object })
], WaPopup.prototype, "shiftBoundary", 2);
__decorateClass([
  n3({ attribute: "shift-padding", type: Number })
], WaPopup.prototype, "shiftPadding", 2);
__decorateClass([
  n3({ attribute: "auto-size" })
], WaPopup.prototype, "autoSize", 2);
__decorateClass([
  n3()
], WaPopup.prototype, "sync", 2);
__decorateClass([
  n3({ type: Object })
], WaPopup.prototype, "autoSizeBoundary", 2);
__decorateClass([
  n3({ attribute: "auto-size-padding", type: Number })
], WaPopup.prototype, "autoSizePadding", 2);
__decorateClass([
  n3({ attribute: "hover-bridge", type: Boolean })
], WaPopup.prototype, "hoverBridge", 2);
WaPopup = __decorateClass([
  t3("wa-popup")
], WaPopup);

// chunks/chunk.52WA2DJO.js
var dismissibleStack = [];
function registerDismissible(key) {
  dismissibleStack.push(key);
}
function unregisterDismissible(key) {
  for (let i7 = dismissibleStack.length - 1; i7 >= 0; i7--) {
    if (dismissibleStack[i7] === key) {
      dismissibleStack.splice(i7, 1);
      break;
    }
  }
}
function isTopDismissible(key) {
  return dismissibleStack.length > 0 && dismissibleStack[dismissibleStack.length - 1] === key;
}

// chunks/chunk.4ZAKP7NY.js
var WaShowEvent = class extends Event {
  constructor() {
    super("wa-show", { bubbles: true, cancelable: true, composed: true });
  }
};

// chunks/chunk.MQODJ75V.js
var WaHideEvent = class extends Event {
  constructor(detail) {
    super("wa-hide", { bubbles: true, cancelable: true, composed: true });
    this.detail = detail;
  }
};

// chunks/chunk.3NKIHICW.js
var WaAfterHideEvent = class extends Event {
  constructor() {
    super("wa-after-hide", { bubbles: true, cancelable: false, composed: true });
  }
};

// chunks/chunk.PX3HMKF7.js
var WaAfterShowEvent = class extends Event {
  constructor() {
    super("wa-after-show", { bubbles: true, cancelable: false, composed: true });
  }
};

// chunks/chunk.BPZLEXWB.js
var WaTooltip = class extends WebAwesomeElement {
  constructor() {
    super(...arguments);
    this.placement = "top";
    this.disabled = false;
    this.distance = 8;
    this.open = false;
    this.skidding = 0;
    this.showDelay = 150;
    this.hideDelay = 0;
    this.trigger = "hover focus";
    this.withoutArrow = false;
    this.for = null;
    this.anchor = null;
    this.eventController = new AbortController();
    this.handleBlur = () => {
      if (this.hasTrigger("focus")) {
        this.hide();
      }
    };
    this.handleClick = () => {
      if (this.hasTrigger("click")) {
        if (this.open) {
          this.hide();
        } else {
          this.show();
        }
      }
    };
    this.handleFocus = () => {
      if (this.hasTrigger("focus")) {
        this.show();
      }
    };
    this.handleDocumentKeyDown = (event) => {
      if (event.key === "Escape" && this.open && isTopDismissible(this)) {
        event.preventDefault();
        event.stopPropagation();
        this.hide();
      }
    };
    this.handleMouseOver = () => {
      if (this.hasTrigger("hover")) {
        clearTimeout(this.hoverTimeout);
        this.hoverTimeout = window.setTimeout(() => this.show(), this.showDelay);
      }
    };
    this.handleMouseOut = () => {
      if (this.hasTrigger("hover")) {
        const anchorHovered = Boolean(this.anchor?.matches(":hover"));
        const tooltipHovered = this.matches(":hover");
        if (anchorHovered || tooltipHovered) {
          return;
        }
        clearTimeout(this.hoverTimeout);
        if (!(anchorHovered || tooltipHovered)) {
          this.hoverTimeout = window.setTimeout(() => {
            this.hide();
          }, this.hideDelay);
        }
      }
    };
  }
  connectedCallback() {
    super.connectedCallback();
    if (this.eventController.signal.aborted) {
      this.eventController = new AbortController();
    }
    this.addEventListener("mouseout", this.handleMouseOut);
    if (this.open) {
      this.open = false;
      this.updateComplete.then(() => {
        this.open = true;
      });
    }
    if (!this.id) {
      this.id = uniqueId("wa-tooltip-");
    }
    if (this.for && this.anchor) {
      this.anchor = null;
      this.handleForChange();
    } else if (this.for) {
      this.handleForChange();
    }
  }
  disconnectedCallback() {
    super.disconnectedCallback();
    document.removeEventListener("keydown", this.handleDocumentKeyDown);
    unregisterDismissible(this);
    this.eventController.abort();
    if (this.anchor) {
      this.removeFromAriaLabelledBy(this.anchor, this.id);
    }
  }
  firstUpdated() {
    this.body.hidden = !this.open;
    if (this.open) {
      this.popup.active = true;
      this.popup.reposition();
    }
  }
  hasTrigger(triggerType) {
    const triggers = this.trigger.split(" ");
    return triggers.includes(triggerType);
  }
  addToAriaLabelledBy(element, id) {
    const currentLabel = element.getAttribute("aria-labelledby") || "";
    const labels = currentLabel.split(/\s+/).filter(Boolean);
    if (!labels.includes(id)) {
      labels.push(id);
      element.setAttribute("aria-labelledby", labels.join(" "));
    }
  }
  removeFromAriaLabelledBy(element, id) {
    const currentLabel = element.getAttribute("aria-labelledby") || "";
    const labels = currentLabel.split(/\s+/).filter(Boolean);
    const filteredLabels = labels.filter((label) => label !== id);
    if (filteredLabels.length > 0) {
      element.setAttribute("aria-labelledby", filteredLabels.join(" "));
    } else {
      element.removeAttribute("aria-labelledby");
    }
  }
  async handleOpenChange() {
    if (this.open) {
      if (this.disabled) {
        return;
      }
      const waShowEvent = new WaShowEvent();
      this.dispatchEvent(waShowEvent);
      if (waShowEvent.defaultPrevented) {
        this.open = false;
        return;
      }
      document.addEventListener("keydown", this.handleDocumentKeyDown, { signal: this.eventController.signal });
      registerDismissible(this);
      this.body.hidden = false;
      this.popup.active = true;
      await animateWithClass(this.popup.popup, "show-with-scale");
      this.popup.reposition();
      this.dispatchEvent(new WaAfterShowEvent());
    } else {
      const waHideEvent = new WaHideEvent();
      this.dispatchEvent(waHideEvent);
      if (waHideEvent.defaultPrevented) {
        this.open = false;
        return;
      }
      document.removeEventListener("keydown", this.handleDocumentKeyDown);
      unregisterDismissible(this);
      await animateWithClass(this.popup.popup, "hide-with-scale");
      this.popup.active = false;
      this.body.hidden = true;
      this.dispatchEvent(new WaAfterHideEvent());
    }
  }
  handleForChange() {
    const rootNode = this.getRootNode();
    if (!rootNode) {
      return;
    }
    const newAnchor = this.for ? rootNode.getElementById(this.for) : null;
    const oldAnchor = this.anchor;
    if (newAnchor === oldAnchor) {
      return;
    }
    const { signal } = this.eventController;
    if (newAnchor) {
      this.addToAriaLabelledBy(newAnchor, this.id);
      newAnchor.addEventListener("blur", this.handleBlur, { capture: true, signal });
      newAnchor.addEventListener("focus", this.handleFocus, { capture: true, signal });
      newAnchor.addEventListener("click", this.handleClick, { signal });
      newAnchor.addEventListener("mouseover", this.handleMouseOver, { signal });
      newAnchor.addEventListener("mouseout", this.handleMouseOut, { signal });
    }
    if (oldAnchor) {
      this.removeFromAriaLabelledBy(oldAnchor, this.id);
      oldAnchor.removeEventListener("blur", this.handleBlur, { capture: true });
      oldAnchor.removeEventListener("focus", this.handleFocus, { capture: true });
      oldAnchor.removeEventListener("click", this.handleClick);
      oldAnchor.removeEventListener("mouseover", this.handleMouseOver);
      oldAnchor.removeEventListener("mouseout", this.handleMouseOut);
    }
    this.anchor = newAnchor;
  }
  async handleOptionsChange() {
    if (this.hasUpdated) {
      await this.updateComplete;
      this.popup.reposition();
    }
  }
  handleDisabledChange() {
    if (this.disabled && this.open) {
      this.hide();
    }
  }
  async show() {
    if (this.open) {
      return void 0;
    }
    this.open = true;
    return waitForEvent(this, "wa-after-show");
  }
  async hide() {
    if (!this.open) {
      return void 0;
    }
    this.open = false;
    return waitForEvent(this, "wa-after-hide");
  }
  render() {
    return x`
      <wa-popup
        part="base"
        exportparts="
          popup:base__popup,
          arrow:base__arrow
        "
        class=${e24({
      tooltip: true,
      "tooltip-open": this.open
    })}
        placement=${this.placement}
        distance=${this.distance}
        skidding=${this.skidding}
        flip
        shift
        ?arrow=${!this.withoutArrow}
        hover-bridge
        .anchor=${this.anchor}
      >
        <div part="body" class="body">
          <slot></slot>
        </div>
      </wa-popup>
    `;
  }
};
WaTooltip.css = tooltip_styles_default;
WaTooltip.dependencies = { "wa-popup": WaPopup };
__decorateClass([
  e23("slot:not([name])")
], WaTooltip.prototype, "defaultSlot", 2);
__decorateClass([
  e23(".body")
], WaTooltip.prototype, "body", 2);
__decorateClass([
  e23("wa-popup")
], WaTooltip.prototype, "popup", 2);
__decorateClass([
  n3()
], WaTooltip.prototype, "placement", 2);
__decorateClass([
  n3({ type: Boolean, reflect: true })
], WaTooltip.prototype, "disabled", 2);
__decorateClass([
  n3({ type: Number })
], WaTooltip.prototype, "distance", 2);
__decorateClass([
  n3({ type: Boolean, reflect: true })
], WaTooltip.prototype, "open", 2);
__decorateClass([
  n3({ type: Number })
], WaTooltip.prototype, "skidding", 2);
__decorateClass([
  n3({ attribute: "show-delay", type: Number })
], WaTooltip.prototype, "showDelay", 2);
__decorateClass([
  n3({ attribute: "hide-delay", type: Number })
], WaTooltip.prototype, "hideDelay", 2);
__decorateClass([
  n3()
], WaTooltip.prototype, "trigger", 2);
__decorateClass([
  n3({ attribute: "without-arrow", type: Boolean, reflect: true })
], WaTooltip.prototype, "withoutArrow", 2);
__decorateClass([
  n3()
], WaTooltip.prototype, "for", 2);
__decorateClass([
  r23()
], WaTooltip.prototype, "anchor", 2);
__decorateClass([
  watch("open", { waitUntilFirstUpdate: true })
], WaTooltip.prototype, "handleOpenChange", 1);
__decorateClass([
  watch("for")
], WaTooltip.prototype, "handleForChange", 1);
__decorateClass([
  watch(["distance", "placement", "skidding"])
], WaTooltip.prototype, "handleOptionsChange", 1);
__decorateClass([
  watch("disabled")
], WaTooltip.prototype, "handleDisabledChange", 1);
WaTooltip = __decorateClass([
  t3("wa-tooltip")
], WaTooltip);

// chunks/chunk.LY4FQ3ZV.js
var dialog_styles_default = i2`
  :host {
    --width: 31rem;
    --spacing: var(--wa-space-l);
    --show-duration: 200ms;
    --hide-duration: 200ms;

    display: none;
  }

  :host([open]) {
    display: block;
  }

  .dialog {
    display: flex;
    flex-direction: column;
    top: 0;
    right: 0;
    bottom: 0;
    left: 0;
    width: var(--width);
    max-width: calc(100% - var(--wa-space-2xl));
    max-height: calc(100% - var(--wa-space-2xl));
    color: inherit;
    background-color: var(--wa-color-surface-raised);
    border-radius: var(--wa-panel-border-radius);
    border: none;
    box-shadow: var(--wa-shadow-l);
    padding: 0;
    margin: auto;

    &.show {
      animation: show-dialog var(--show-duration) ease;

      &::backdrop {
        animation: show-backdrop var(--show-duration, 200ms) ease;
      }
    }

    &.hide {
      animation: show-dialog var(--hide-duration) ease reverse;

      &::backdrop {
        animation: show-backdrop var(--hide-duration, 200ms) ease reverse;
      }
    }

    &.pulse {
      animation: pulse 250ms ease;
    }
  }

  .dialog:focus {
    outline: none;
  }

  /* Ensure there's enough vertical padding for phones that don't update vh when chrome appears (e.g. iPhone) */
  @media screen and (max-width: 420px) {
    .dialog {
      max-height: 80vh;
    }
  }

  .open {
    display: flex;
    opacity: 1;
  }

  .header {
    flex: 0 0 auto;
    display: flex;
    flex-wrap: nowrap;

    padding-inline-start: var(--spacing);
    padding-block-end: 0;

    /* Subtract the close button's padding so that the X is visually aligned with the edges of the dialog content */
    padding-inline-end: calc(var(--spacing) - var(--wa-form-control-padding-block));
    padding-block-start: calc(var(--spacing) - var(--wa-form-control-padding-block));
  }

  .title {
    align-self: center;
    flex: 1 1 auto;
    font-family: inherit;
    font-size: var(--wa-font-size-l);
    font-weight: var(--wa-font-weight-heading);
    line-height: var(--wa-line-height-condensed);
    margin: 0;
  }

  .header-actions {
    align-self: start;
    display: flex;
    flex-shrink: 0;
    flex-wrap: wrap;
    justify-content: end;
    gap: var(--wa-space-2xs);
    padding-inline-start: var(--spacing);
  }

  .header-actions wa-button,
  .header-actions ::slotted(wa-button) {
    flex: 0 0 auto;
    display: flex;
    align-items: center;
  }

  .body {
    flex: 1 1 auto;
    display: block;
    padding: var(--spacing);
    overflow: auto;
    -webkit-overflow-scrolling: touch;

    &:focus {
      outline: none;
    }

    &:focus-visible {
      outline: var(--wa-focus-ring);
      outline-offset: var(--wa-focus-ring-offset);
    }
  }

  .footer {
    flex: 0 0 auto;
    display: flex;
    flex-wrap: wrap;
    gap: var(--wa-space-xs);
    justify-content: end;
    padding: var(--spacing);
    padding-block-start: 0;
  }

  .footer ::slotted(wa-button:not(:first-of-type)) {
    margin-inline-start: var(--wa-spacing-xs);
  }

  .dialog::backdrop {
    /*
      NOTE: the ::backdrop element doesn't inherit properly in Safari yet, but it will in 17.4! At that time, we can
      remove the fallback values here.
    */
    background-color: var(--wa-color-overlay-modal, rgb(0 0 0 / 0.25));
  }

  @keyframes pulse {
    0% {
      scale: 1;
    }
    50% {
      scale: 1.02;
    }
    100% {
      scale: 1;
    }
  }

  @keyframes show-dialog {
    from {
      opacity: 0;
      scale: 0.8;
    }
    to {
      opacity: 1;
      scale: 1;
    }
  }

  @keyframes show-backdrop {
    from {
      opacity: 0;
    }
    to {
      opacity: 1;
    }
  }

  @media (forced-colors: active) {
    .dialog {
      border: solid 1px white;
    }
  }
`;

// chunks/chunk.VQZ46MYI.js
var locks = /* @__PURE__ */ new Set();
function getScrollbarWidth() {
  const documentWidth = document.documentElement.clientWidth;
  return Math.abs(window.innerWidth - documentWidth);
}
function getExistingBodyPadding() {
  const padding = Number(getComputedStyle(document.body).paddingRight.replace(/px/, ""));
  if (isNaN(padding) || !padding) {
    return 0;
  }
  return padding;
}
function lockBodyScrolling(lockingEl) {
  locks.add(lockingEl);
  if (!document.documentElement.classList.contains("wa-scroll-lock")) {
    const scrollbarWidth = getScrollbarWidth() + getExistingBodyPadding();
    let scrollbarGutterProperty = getComputedStyle(document.documentElement).scrollbarGutter;
    if (!scrollbarGutterProperty || scrollbarGutterProperty === "auto") {
      scrollbarGutterProperty = "stable";
    }
    if (scrollbarWidth < 2) {
      scrollbarGutterProperty = "";
    }
    document.documentElement.style.setProperty("--wa-scroll-lock-gutter", scrollbarGutterProperty);
    document.documentElement.classList.add("wa-scroll-lock");
    document.documentElement.style.setProperty("--wa-scroll-lock-size", `${scrollbarWidth}px`);
  }
}
function unlockBodyScrolling(lockingEl) {
  locks.delete(lockingEl);
  if (locks.size === 0) {
    document.documentElement.classList.remove("wa-scroll-lock");
    document.documentElement.style.removeProperty("--wa-scroll-lock-size");
  }
}

// chunks/chunk.RMZ7BVDM.js
function parseSpaceDelimitedTokens(input) {
  return input.split(" ").map((token) => token.trim()).filter((token) => token !== "");
}

// chunks/chunk.ZJAKOEXX.js
var WaDialog = class extends WebAwesomeElement {
  constructor() {
    super(...arguments);
    this.localize = new LocalizeController2(this);
    this.hasSlotController = new HasSlotController(this, "footer", "header-actions", "label");
    this.open = false;
    this.label = "";
    this.withoutHeader = false;
    this.lightDismiss = false;
    this.withFooter = false;
    this.handleDocumentKeyDown = (event) => {
      if (event.key === "Escape" && this.open && isTopDismissible(this)) {
        event.preventDefault();
        event.stopPropagation();
        this.requestClose(this.dialog);
      }
    };
  }
  firstUpdated() {
    if (this.open) {
      this.addOpenListeners();
      this.dialog.showModal();
      lockBodyScrolling(this);
    }
  }
  disconnectedCallback() {
    super.disconnectedCallback();
    unlockBodyScrolling(this);
    this.removeOpenListeners();
  }
  async requestClose(source) {
    const waHideEvent = new WaHideEvent({ source });
    this.dispatchEvent(waHideEvent);
    if (waHideEvent.defaultPrevented) {
      this.open = true;
      animateWithClass(this.dialog, "pulse");
      return;
    }
    this.removeOpenListeners();
    await animateWithClass(this.dialog, "hide");
    this.open = false;
    this.dialog.close();
    unlockBodyScrolling(this);
    const trigger = this.originalTrigger;
    if (typeof trigger?.focus === "function") {
      setTimeout(() => trigger.focus());
    }
    this.dispatchEvent(new WaAfterHideEvent());
  }
  addOpenListeners() {
    document.addEventListener("keydown", this.handleDocumentKeyDown);
    registerDismissible(this);
  }
  removeOpenListeners() {
    document.removeEventListener("keydown", this.handleDocumentKeyDown);
    unregisterDismissible(this);
  }
  handleDialogCancel(event) {
    event.preventDefault();
    if (!this.dialog.classList.contains("hide") && event.target === this.dialog && isTopDismissible(this)) {
      this.requestClose(this.dialog);
    }
  }
  handleDialogClick(event) {
    const target = event.target;
    const button = target.closest('[data-dialog="close"]');
    if (button) {
      event.stopPropagation();
      this.requestClose(button);
    }
  }
  async handleDialogPointerDown(event) {
    if (event.target === this.dialog) {
      if (this.lightDismiss) {
        this.requestClose(this.dialog);
      } else {
        await animateWithClass(this.dialog, "pulse");
      }
    }
  }
  handleOpenChange() {
    if (this.open && !this.dialog.open) {
      this.show();
    } else if (!this.open && this.dialog.open) {
      this.open = true;
      this.requestClose(this.dialog);
    }
  }
  async show() {
    const waShowEvent = new WaShowEvent();
    this.dispatchEvent(waShowEvent);
    if (waShowEvent.defaultPrevented) {
      this.open = false;
      return;
    }
    this.addOpenListeners();
    this.originalTrigger = document.activeElement;
    this.open = true;
    this.dialog.showModal();
    lockBodyScrolling(this);
    requestAnimationFrame(() => {
      const elementToFocus = this.querySelector("[autofocus]");
      if (elementToFocus && typeof elementToFocus.focus === "function") {
        elementToFocus.focus();
      } else {
        this.dialog.focus();
      }
    });
    await animateWithClass(this.dialog, "show");
    this.dispatchEvent(new WaAfterShowEvent());
  }
  render() {
    const hasHeader = !this.withoutHeader;
    const hasFooter = this.hasUpdated ? this.hasSlotController.test("footer") : this.withFooter;
    return x`
      <dialog
        part="dialog"
        class=${e24({
      dialog: true,
      open: this.open
    })}
        @cancel=${this.handleDialogCancel}
        @click=${this.handleDialogClick}
        @pointerdown=${this.handleDialogPointerDown}
      >
        ${hasHeader ? x`
              <header part="header" class="header">
                <h2 part="title" class="title" id="title">
                  <!-- If there's no label, use an invisible character to prevent the header from collapsing -->
                  <slot name="label"> ${this.label.length > 0 ? this.label : String.fromCharCode(8203)} </slot>
                </h2>
                <div part="header-actions" class="header-actions">
                  <slot name="header-actions"></slot>
                  <wa-button
                    part="close-button"
                    exportparts="base:close-button__base"
                    class="close"
                    appearance="plain"
                    @click="${(event) => this.requestClose(event.target)}"
                  >
                    <wa-icon
                      name="xmark"
                      label=${this.localize.term("close")}
                      library="system"
                      variant="solid"
                    ></wa-icon>
                  </wa-button>
                </div>
              </header>
            ` : ""}

        <div part="body" class="body"><slot></slot></div>

        ${hasFooter ? x`
              <footer part="footer" class="footer">
                <slot name="footer"></slot>
              </footer>
            ` : ""}
      </dialog>
    `;
  }
};
WaDialog.css = dialog_styles_default;
__decorateClass([
  e23(".dialog")
], WaDialog.prototype, "dialog", 2);
__decorateClass([
  n3({ type: Boolean, reflect: true })
], WaDialog.prototype, "open", 2);
__decorateClass([
  n3({ reflect: true })
], WaDialog.prototype, "label", 2);
__decorateClass([
  n3({ attribute: "without-header", type: Boolean, reflect: true })
], WaDialog.prototype, "withoutHeader", 2);
__decorateClass([
  n3({ attribute: "light-dismiss", type: Boolean })
], WaDialog.prototype, "lightDismiss", 2);
__decorateClass([
  n3({ attribute: "with-footer", type: Boolean })
], WaDialog.prototype, "withFooter", 2);
__decorateClass([
  watch("open", { waitUntilFirstUpdate: true })
], WaDialog.prototype, "handleOpenChange", 1);
WaDialog = __decorateClass([
  t3("wa-dialog")
], WaDialog);
if (!o3) {
  document.addEventListener("click", (event) => {
    const dialogAttrEl = event.target.closest("[data-dialog]");
    if (dialogAttrEl instanceof Element) {
      const [command, id] = parseSpaceDelimitedTokens(dialogAttrEl.getAttribute("data-dialog") || "");
      if (command === "open" && id?.length) {
        const doc = dialogAttrEl.getRootNode();
        const dialog = doc.getElementById(id);
        if (dialog?.localName === "wa-dialog") {
          dialog.open = true;
        } else {
          console.warn(`A dialog with an ID of "${id}" could not be found in this document.`);
        }
      }
    }
  });
  document.addEventListener("pointerdown", () => {
  });
}

// chunks/chunk.JTOY5KP3.js
var WaClearEvent = class extends Event {
  constructor() {
    super("wa-clear", { bubbles: true, cancelable: false, composed: true });
  }
};

// chunks/chunk.DOFHHKB4.js
function submitOnEnter(event, el) {
  const hasModifier = event.metaKey || event.ctrlKey || event.shiftKey || event.altKey;
  if (event.key === "Enter" && !hasModifier) {
    setTimeout(() => {
      if (!event.defaultPrevented && !event.isComposing) {
        submitForm(el);
      }
    });
  }
}
function submitForm(el) {
  let form = null;
  if ("form" in el) {
    form = el.form;
  }
  if (!form && "getForm" in el) {
    form = el.getForm();
  }
  if (!form) {
    return;
  }
  const formElements = [...form.elements];
  if (formElements.length === 1) {
    form.requestSubmit(null);
    return;
  }
  const button = formElements.find((el2) => el2.type === "submit" && !el2.matches(":disabled"));
  if (!button) {
    return;
  }
  if (["input", "button"].includes(button.localName)) {
    form.requestSubmit(button);
  } else {
    button.click();
  }
}

// chunks/chunk.BE5VT7MB.js
var input_styles_default = i2`
  :host {
    border-width: 0;
  }

  :host(:focus) {
    outline: none;
  }

  .text-field {
    display: flex;
    align-items: stretch;
    justify-content: start;
    position: relative;
    transition: inherit;
    height: var(--wa-form-control-height);
    border-color: var(--wa-form-control-border-color);
    border-radius: var(--wa-form-control-border-radius);
    border-style: var(--wa-form-control-border-style);
    border-width: var(--wa-form-control-border-width);
    cursor: text;
    color: var(--wa-form-control-value-color);
    font-size: var(--wa-form-control-value-font-size);
    font-family: inherit;
    font-weight: var(--wa-form-control-value-font-weight);
    line-height: var(--wa-form-control-value-line-height);
    vertical-align: middle;
    width: 100%;
    transition:
      background-color var(--wa-transition-normal),
      border-color var(--wa-transition-normal),
      outline-color var(--wa-transition-fast);
    transition-timing-function: var(--wa-transition-easing);
    background-color: var(--wa-form-control-background-color);
    box-shadow: var(--box-shadow);
    padding: 0 var(--wa-form-control-padding-inline);
    outline: var(--wa-focus-ring-style) var(--wa-focus-ring-width) transparent;
    outline-offset: var(--wa-focus-ring-offset);

    &:focus-within {
      outline-color: var(--wa-color-focus);
    }

    /* Style disabled inputs */
    &:has(:disabled) {
      cursor: not-allowed;
      opacity: 0.5;
    }
  }

  /* Appearance modifiers */
  :host([appearance='outlined']) .text-field {
    background-color: var(--wa-form-control-background-color);
    border-color: var(--wa-form-control-border-color);
  }

  :host([appearance='filled']) .text-field {
    background-color: var(--wa-color-neutral-fill-quiet);
    border-color: var(--wa-color-neutral-fill-quiet);
  }

  :host([appearance='filled-outlined']) .text-field {
    background-color: var(--wa-color-neutral-fill-quiet);
    border-color: var(--wa-form-control-border-color);
  }

  :host([pill]) .text-field {
    border-radius: var(--wa-border-radius-pill) !important;
  }

  .text-field {
    /* Show autofill styles over the entire text field, not just the native <input> */
    &:has(:autofill),
    &:has(:-webkit-autofill) {
      background-color: var(--wa-color-brand-fill-quiet) !important;
    }

    input,
    textarea {
      /*
      Fixes an alignment issue with placeholders.
      https://github.com/shoelace-style/webawesome/issues/342
    */
      height: 100%;

      padding: 0;
      border: none;
      outline: none;
      box-shadow: none;
      margin: 0;
      cursor: inherit;
      -webkit-appearance: none;
      font: inherit;

      /* Turn off Safari's autofill styles */
      &:-webkit-autofill,
      &:-webkit-autofill:hover,
      &:-webkit-autofill:focus,
      &:-webkit-autofill:active {
        -webkit-background-clip: text;
        background-color: transparent;
        -webkit-text-fill-color: inherit;
      }
    }
  }

  input {
    flex: 1 1 auto;
    min-width: 0;
    height: 100%;
    transition: inherit;

    /* prettier-ignore */
    background-color: rgb(118 118 118 / 0); /* ensures proper placeholder styles in webkit's date input */
    height: calc(var(--wa-form-control-height) - var(--border-width) * 2);
    padding-block: 0;
    color: inherit;

    &:autofill {
      &,
      &:hover,
      &:focus,
      &:active {
        box-shadow: none;
        caret-color: var(--wa-form-control-value-color);
      }
    }

    &::placeholder {
      color: var(--wa-form-control-placeholder-color);
      user-select: none;
      -webkit-user-select: none;
    }

    &::-webkit-search-decoration,
    &::-webkit-search-cancel-button,
    &::-webkit-search-results-button,
    &::-webkit-search-results-decoration {
      -webkit-appearance: none;
    }

    &:focus {
      outline: none;
    }
  }

  textarea {
    &:autofill {
      &,
      &:hover,
      &:focus,
      &:active {
        box-shadow: none;
        caret-color: var(--wa-form-control-value-color);
      }
    }

    &::placeholder {
      color: var(--wa-form-control-placeholder-color);
      user-select: none;
      -webkit-user-select: none;
    }
  }

  .start,
  .end {
    display: inline-flex;
    flex: 0 0 auto;
    align-items: center;
    cursor: default;

    &::slotted(wa-icon) {
      color: var(--wa-color-neutral-on-quiet);
    }
  }

  .start::slotted(*) {
    margin-inline-end: var(--wa-form-control-padding-inline);
  }

  .end::slotted(*) {
    margin-inline-start: var(--wa-form-control-padding-inline);
  }

  /*
   * Clearable + Password Toggle
   */

  .clear,
  .password-toggle {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    font-size: inherit;
    color: var(--wa-color-neutral-on-quiet);
    border: none;
    background: none;
    padding: 0;
    transition: var(--wa-transition-normal) color;
    cursor: pointer;
    margin-inline-start: var(--wa-form-control-padding-inline);

    @media (hover: hover) {
      &:hover {
        color: color-mix(in oklab, currentColor, var(--wa-color-mix-hover));
      }
    }

    &:active {
      color: color-mix(in oklab, currentColor, var(--wa-color-mix-active));
    }

    &:focus {
      outline: none;
    }
  }

  /* Don't show the browser's password toggle in Edge */
  ::-ms-reveal {
    display: none;
  }

  /* Hide the built-in number spinner */
  :host([without-spin-buttons]) input[type='number'] {
    -moz-appearance: textfield;

    &::-webkit-outer-spin-button,
    &::-webkit-inner-spin-button {
      -webkit-appearance: none;
      display: none;
    }
  }
`;

// chunks/chunk.346V4PTX.js
var form_control_styles_default = i2`
  :host {
    display: flex;
    flex-direction: column;
  }

  /* Treat wrapped labels, inputs, and hints as direct children of the host element */
  [part~='form-control'] {
    display: contents;
  }

  /* Label */
  :is([part~='form-control-label'], [part~='label']):has(*:not(:empty)),
  :is([part~='form-control-label'], [part~='label']).has-label {
    display: inline-flex;
    color: var(--wa-form-control-label-color);
    font-weight: var(--wa-form-control-label-font-weight);
    line-height: var(--wa-form-control-label-line-height);
    margin-block-end: 0.5em;
  }

  :host([required]) :is([part~='form-control-label'], [part~='label'])::after {
    content: var(--wa-form-control-required-content);
    margin-inline-start: var(--wa-form-control-required-content-offset);
    color: var(--wa-form-control-required-content-color);
  }

  /* Help text */
  [part~='hint'] {
    display: block;
    color: var(--wa-form-control-hint-color);
    font-weight: var(--wa-form-control-hint-font-weight);
    line-height: var(--wa-form-control-hint-line-height);
    margin-block-start: 0.5em;
    font-size: var(--wa-font-size-smaller);

    &:not(.has-slotted, .has-hint) {
      display: none;
    }
  }
`;

// chunks/chunk.KZZR6Z6I.js
var l4 = e4(class extends i4 {
  constructor(r5) {
    if (super(r5), r5.type !== t4.PROPERTY && r5.type !== t4.ATTRIBUTE && r5.type !== t4.BOOLEAN_ATTRIBUTE)
      throw Error("The `live` directive is not allowed on child or event bindings");
    if (!f3(r5))
      throw Error("`live` bindings can only contain a single expression");
  }
  render(r5) {
    return r5;
  }
  update(i24, [t23]) {
    if (t23 === T || t23 === E)
      return t23;
    const o7 = i24.element, l22 = i24.name;
    if (i24.type === t4.PROPERTY) {
      if (t23 === o7[l22])
        return T;
    } else if (i24.type === t4.BOOLEAN_ATTRIBUTE) {
      if (!!t23 === o7.hasAttribute(l22))
        return T;
    } else if (i24.type === t4.ATTRIBUTE && o7.getAttribute(l22) === t23 + "")
      return T;
    return m2(i24), t23;
  }
});

// chunks/chunk.Y2L6BJPS.js
var WaInput = class extends WebAwesomeFormAssociatedElement {
  constructor() {
    super(...arguments);
    this.assumeInteractionOn = ["blur", "input"];
    this.hasSlotController = new HasSlotController(this, "hint", "label");
    this.localize = new LocalizeController2(this);
    this.title = "";
    this.type = "text";
    this._value = null;
    this.defaultValue = this.getAttribute("value") || null;
    this.size = "medium";
    this.appearance = "outlined";
    this.pill = false;
    this.label = "";
    this.hint = "";
    this.withClear = false;
    this.placeholder = "";
    this.readonly = false;
    this.passwordToggle = false;
    this.passwordVisible = false;
    this.withoutSpinButtons = false;
    this.required = false;
    this.spellcheck = true;
    this.withLabel = false;
    this.withHint = false;
  }
  static get validators() {
    return [...super.validators, MirrorValidator()];
  }
  get value() {
    if (this.valueHasChanged) {
      return this._value;
    }
    return this._value ?? this.defaultValue;
  }
  set value(val) {
    if (this._value === val) {
      return;
    }
    this.valueHasChanged = true;
    this._value = val;
  }
  handleChange(event) {
    this.value = this.input.value;
    this.relayNativeEvent(event, { bubbles: true, composed: true });
  }
  handleClearClick(event) {
    event.preventDefault();
    if (this.value !== "") {
      this.value = "";
      this.updateComplete.then(() => {
        this.dispatchEvent(new WaClearEvent());
        this.dispatchEvent(new InputEvent("input", { bubbles: true, composed: true }));
        this.dispatchEvent(new Event("change", { bubbles: true, composed: true }));
      });
    }
    this.input.focus();
  }
  handleInput() {
    this.value = this.input.value;
  }
  handleKeyDown(event) {
    submitOnEnter(event, this);
  }
  handlePasswordToggle() {
    this.passwordVisible = !this.passwordVisible;
  }
  updated(changedProperties) {
    super.updated(changedProperties);
    if (changedProperties.has("value") || changedProperties.has("defaultValue")) {
      this.customStates.set("blank", !this.value);
      this.updateValidity();
    }
  }
  handleStepChange() {
    this.input.step = String(this.step);
    this.updateValidity();
  }
  focus(options) {
    this.input.focus(options);
  }
  blur() {
    this.input.blur();
  }
  select() {
    this.input.select();
  }
  setSelectionRange(selectionStart, selectionEnd, selectionDirection = "none") {
    this.input.setSelectionRange(selectionStart, selectionEnd, selectionDirection);
  }
  setRangeText(replacement, start, end, selectMode = "preserve") {
    const selectionStart = start ?? this.input.selectionStart;
    const selectionEnd = end ?? this.input.selectionEnd;
    this.input.setRangeText(replacement, selectionStart, selectionEnd, selectMode);
    if (this.value !== this.input.value) {
      this.value = this.input.value;
    }
  }
  showPicker() {
    if ("showPicker" in HTMLInputElement.prototype) {
      this.input.showPicker();
    }
  }
  stepUp() {
    this.input.stepUp();
    if (this.value !== this.input.value) {
      this.value = this.input.value;
    }
  }
  stepDown() {
    this.input.stepDown();
    if (this.value !== this.input.value) {
      this.value = this.input.value;
    }
  }
  formResetCallback() {
    this.value = null;
    if (this.input) {
      this.input.value = this.value;
    }
    super.formResetCallback();
  }
  render() {
    const hasLabelSlot = this.hasUpdated ? this.hasSlotController.test("label") : this.withLabel;
    const hasHintSlot = this.hasUpdated ? this.hasSlotController.test("hint") : this.withHint;
    const hasLabel = this.label ? true : !!hasLabelSlot;
    const hasHint = this.hint ? true : !!hasHintSlot;
    const hasClearIcon = this.withClear && !this.disabled && !this.readonly;
    const isClearIconVisible = (o3 || this.hasUpdated) && hasClearIcon && (typeof this.value === "number" || this.value && this.value.length > 0);
    return x`
      <label
        part="form-control-label label"
        class=${e24({
      label: true,
      "has-label": hasLabel
    })}
        for="input"
        aria-hidden=${hasLabel ? "false" : "true"}
      >
        <slot name="label">${this.label}</slot>
      </label>

      <div part="base" class="text-field">
        <slot name="start" part="start" class="start"></slot>

        <input
          part="input"
          id="input"
          class="control"
          type=${this.type === "password" && this.passwordVisible ? "text" : this.type}
          title=${this.title}
          name=${o5(this.name)}
          ?disabled=${this.disabled}
          ?readonly=${this.readonly}
          ?required=${this.required}
          placeholder=${o5(this.placeholder)}
          minlength=${o5(this.minlength)}
          maxlength=${o5(this.maxlength)}
          min=${o5(this.min)}
          max=${o5(this.max)}
          step=${o5(this.step)}
          .value=${l4(this.value ?? "")}
          autocapitalize=${o5(this.autocapitalize)}
          autocomplete=${o5(this.autocomplete)}
          autocorrect=${this.autocorrect ? "on" : "off"}
          ?autofocus=${this.autofocus}
          spellcheck=${this.spellcheck}
          pattern=${o5(this.pattern)}
          enterkeyhint=${o5(this.enterkeyhint)}
          inputmode=${o5(this.inputmode)}
          aria-describedby="hint"
          @change=${this.handleChange}
          @input=${this.handleInput}
          @keydown=${this.handleKeyDown}
        />

        ${isClearIconVisible ? x`
              <button
                part="clear-button"
                class="clear"
                type="button"
                aria-label=${this.localize.term("clearEntry")}
                @click=${this.handleClearClick}
                tabindex="-1"
              >
                <slot name="clear-icon">
                  <wa-icon name="circle-xmark" library="system" variant="regular"></wa-icon>
                </slot>
              </button>
            ` : ""}
        ${this.passwordToggle && !this.disabled ? x`
              <button
                part="password-toggle-button"
                class="password-toggle"
                type="button"
                aria-label=${this.localize.term(this.passwordVisible ? "hidePassword" : "showPassword")}
                @click=${this.handlePasswordToggle}
                tabindex="-1"
              >
                ${!this.passwordVisible ? x`
                      <slot name="show-password-icon">
                        <wa-icon name="eye" library="system" variant="regular"></wa-icon>
                      </slot>
                    ` : x`
                      <slot name="hide-password-icon">
                        <wa-icon name="eye-slash" library="system" variant="regular"></wa-icon>
                      </slot>
                    `}
              </button>
            ` : ""}

        <slot name="end" part="end" class="end"></slot>
      </div>

      <slot
        id="hint"
        part="hint"
        name="hint"
        class=${e24({
      "has-slotted": hasHint
    })}
        aria-hidden=${hasHint ? "false" : "true"}
        >${this.hint}</slot
      >
    `;
  }
};
WaInput.css = [size_styles_default, form_control_styles_default, input_styles_default];
WaInput.shadowRootOptions = { ...WebAwesomeFormAssociatedElement.shadowRootOptions, delegatesFocus: true };
__decorateClass([
  e23("input")
], WaInput.prototype, "input", 2);
__decorateClass([
  n3()
], WaInput.prototype, "title", 2);
__decorateClass([
  n3({ reflect: true })
], WaInput.prototype, "type", 2);
__decorateClass([
  r23()
], WaInput.prototype, "value", 1);
__decorateClass([
  n3({ attribute: "value", reflect: true })
], WaInput.prototype, "defaultValue", 2);
__decorateClass([
  n3({ reflect: true })
], WaInput.prototype, "size", 2);
__decorateClass([
  n3({ reflect: true })
], WaInput.prototype, "appearance", 2);
__decorateClass([
  n3({ type: Boolean, reflect: true })
], WaInput.prototype, "pill", 2);
__decorateClass([
  n3()
], WaInput.prototype, "label", 2);
__decorateClass([
  n3({ attribute: "hint" })
], WaInput.prototype, "hint", 2);
__decorateClass([
  n3({ attribute: "with-clear", type: Boolean })
], WaInput.prototype, "withClear", 2);
__decorateClass([
  n3()
], WaInput.prototype, "placeholder", 2);
__decorateClass([
  n3({ type: Boolean, reflect: true })
], WaInput.prototype, "readonly", 2);
__decorateClass([
  n3({ attribute: "password-toggle", type: Boolean })
], WaInput.prototype, "passwordToggle", 2);
__decorateClass([
  n3({ attribute: "password-visible", type: Boolean })
], WaInput.prototype, "passwordVisible", 2);
__decorateClass([
  n3({ attribute: "without-spin-buttons", type: Boolean, reflect: true })
], WaInput.prototype, "withoutSpinButtons", 2);
__decorateClass([
  n3({ type: Boolean, reflect: true })
], WaInput.prototype, "required", 2);
__decorateClass([
  n3()
], WaInput.prototype, "pattern", 2);
__decorateClass([
  n3({ type: Number })
], WaInput.prototype, "minlength", 2);
__decorateClass([
  n3({ type: Number })
], WaInput.prototype, "maxlength", 2);
__decorateClass([
  n3()
], WaInput.prototype, "min", 2);
__decorateClass([
  n3()
], WaInput.prototype, "max", 2);
__decorateClass([
  n3()
], WaInput.prototype, "step", 2);
__decorateClass([
  n3()
], WaInput.prototype, "autocapitalize", 2);
__decorateClass([
  n3({
    type: Boolean,
    converter: {
      fromAttribute: (value) => !value || value === "off" ? false : true,
      toAttribute: (value) => value ? "on" : "off"
    }
  })
], WaInput.prototype, "autocorrect", 2);
__decorateClass([
  n3()
], WaInput.prototype, "autocomplete", 2);
__decorateClass([
  n3({ type: Boolean })
], WaInput.prototype, "autofocus", 2);
__decorateClass([
  n3()
], WaInput.prototype, "enterkeyhint", 2);
__decorateClass([
  n3({
    type: Boolean,
    converter: {
      fromAttribute: (value) => !value || value === "false" ? false : true,
      toAttribute: (value) => value ? "true" : "false"
    }
  })
], WaInput.prototype, "spellcheck", 2);
__decorateClass([
  n3()
], WaInput.prototype, "inputmode", 2);
__decorateClass([
  n3({ attribute: "with-label", type: Boolean })
], WaInput.prototype, "withLabel", 2);
__decorateClass([
  n3({ attribute: "with-hint", type: Boolean })
], WaInput.prototype, "withHint", 2);
__decorateClass([
  watch("step", { waitUntilFirstUpdate: true })
], WaInput.prototype, "handleStepChange", 1);
WaInput = __decorateClass([
  t3("wa-input")
], WaInput);
WaInput.disableWarning?.("change-in-update");
/*! Bundled license information:

@lit/reactive-element/css-tag.js:
  (**
   * @license
   * Copyright 2019 Google LLC
   * SPDX-License-Identifier: BSD-3-Clause
   *)

@lit/reactive-element/reactive-element.js:
lit-element/lit-element.js:
  (**
   * @license
   * Copyright 2017 Google LLC
   * SPDX-License-Identifier: BSD-3-Clause
   *)

lit-html/is-server.js:
  (**
   * @license
   * Copyright 2022 Google LLC
   * SPDX-License-Identifier: BSD-3-Clause
   *)
*/
/*! Bundled license information:

@lit/reactive-element/decorators/custom-element.js:
@lit/reactive-element/decorators/property.js:
@lit/reactive-element/decorators/state.js:
@lit/reactive-element/decorators/event-options.js:
@lit/reactive-element/decorators/base.js:
@lit/reactive-element/decorators/query.js:
@lit/reactive-element/decorators/query-async.js:
@lit/reactive-element/decorators/query-all.js:
@lit/reactive-element/decorators/query-assigned-nodes.js:
  (**
   * @license
   * Copyright 2017 Google LLC
   * SPDX-License-Identifier: BSD-3-Clause
   *)

@lit/reactive-element/decorators/query-assigned-elements.js:
  (**
   * @license
   * Copyright 2021 Google LLC
   * SPDX-License-Identifier: BSD-3-Clause
   *)
*/
/*! Bundled license information:

lit-html/directive-helpers.js:
  (**
   * @license
   * Copyright 2020 Google LLC
   * SPDX-License-Identifier: BSD-3-Clause
   *)
*/
/*! Bundled license information:

lit-html/directive.js:
  (**
   * @license
   * Copyright 2017 Google LLC
   * SPDX-License-Identifier: BSD-3-Clause
   *)
*/
/*! Bundled license information:

lit-html/directives/class-map.js:
  (**
   * @license
   * Copyright 2018 Google LLC
   * SPDX-License-Identifier: BSD-3-Clause
   *)
*/
/*! Bundled license information:

lit-html/directives/if-defined.js:
  (**
   * @license
   * Copyright 2018 Google LLC
   * SPDX-License-Identifier: BSD-3-Clause
   *)
*/
/*! Bundled license information:

lit-html/directives/live.js:
  (**
   * @license
   * Copyright 2020 Google LLC
   * SPDX-License-Identifier: BSD-3-Clause
   *)
*/
/*! Bundled license information:

lit-html/directives/map.js:
lit-html/directives/range.js:
  (**
   * @license
   * Copyright 2021 Google LLC
   * SPDX-License-Identifier: BSD-3-Clause
   *)
*/
/*! Bundled license information:

lit-html/directives/style-map.js:
  (**
   * @license
   * Copyright 2018 Google LLC
   * SPDX-License-Identifier: BSD-3-Clause
   *)
*/
/*! Bundled license information:

lit-html/lit-html.js:
  (**
   * @license
   * Copyright 2017 Google LLC
   * SPDX-License-Identifier: BSD-3-Clause
   *)
*/
/*! Bundled license information:

lit-html/static.js:
  (**
   * @license
   * Copyright 2020 Google LLC
   * SPDX-License-Identifier: BSD-3-Clause
   *)
*/
/*! Copyright 2026 Fonticons, Inc. - https://webawesome.com/license */
