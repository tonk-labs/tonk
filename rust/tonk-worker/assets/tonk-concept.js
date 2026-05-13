// `<tonk-concept>` — vanilla-JS port of the Rust custom element
// at `rust/tonk-concept/`. Lives here so the tonk-worker can
// inline it into the wrapper HTML it serves for `text/html`
// view bodies; the iframe loads from the worker, so its own
// `customElements` registry needs an independent definition.
//
// Contract — wire protocol, source-attribute parsing, template
// detection, `{field}` substitution, entity-keyed reconciliation,
// lifecycle events — must match `rust/tonk-concept/SPEC.md`.

(() => {
  if (customElements.get("tonk-concept")) return;

  // --- source-attribute parsing -----------------------------------------

  function parseSource(input) {
    const qIdx = input.indexOf("?");
    const head = qIdx < 0 ? input : input.slice(0, qIdx);
    const query = qIdx < 0 ? "" : input.slice(qIdx + 1);
    const filters = {};
    if (query) {
      for (const pair of query.split("&")) {
        if (!pair) continue;
        const eq = pair.indexOf("=");
        if (eq < 0) continue;
        const k = decodeForm(pair.slice(0, eq));
        const v = decodeForm(pair.slice(eq + 1));
        filters[k] = v;
      }
    }
    return { nameOrUri: head, filters, isUri: head.includes(":") };
  }

  function decodeForm(s) {
    return decodeURIComponent(s.replace(/\+/g, " "));
  }

  // --- query construction -----------------------------------------------

  function phase1Body(parsed) {
    const terms = {
      this: { "?": { name: "this" } },
      name: { "?": { name: "name" } },
      source: { "?": { name: "source" } },
    };
    if (parsed.isUri) {
      terms.this = parsed.nameOrUri;
    } else {
      terms.name = parsed.nameOrUri;
    }
    return JSON.stringify({
      terms,
      predicate: {
        with: {
          concept: { the: "dialog.meta/concept", as: "Entity", cardinality: "one" },
          name: { the: "dialog.meta/name", as: "Text", cardinality: "one" },
          description: { the: "dialog.meta/description", as: "Text", cardinality: "one" },
          source: { the: "dialog.meta/source", as: "Text", cardinality: "one" },
        },
      },
    });
  }

  function phase2Body(descriptorJson, filters) {
    const predicate = JSON.parse(descriptorJson);
    const withMap = (predicate && predicate.with) || {};
    const terms = { this: { "?": { name: "this" } } };
    for (const field of Object.keys(withMap)) {
      terms[field] = field in filters ? filters[field] : { "?": { name: field } };
    }
    return JSON.stringify({ terms, predicate });
  }

  // --- segment parsing --------------------------------------------------

  function parseSegments(input) {
    const out = [];
    let buf = "";
    let i = 0;
    while (i < input.length) {
      const ch = input[i];
      if (ch !== "{") {
        buf += ch;
        i++;
        continue;
      }
      const close = input.indexOf("}", i + 1);
      if (close < 0) {
        buf += input.slice(i);
        break;
      }
      if (buf) {
        out.push({ kind: "text", value: buf });
        buf = "";
      }
      out.push({ kind: "field", name: input.slice(i + 1, close) });
      i = close + 1;
    }
    if (buf) out.push({ kind: "text", value: buf });
    return out;
  }

  function hasField(segments) {
    return segments.some((s) => s.kind === "field");
  }

  // --- template snapshot + plan ----------------------------------------

  function snapshotTemplate(host) {
    const tpl = host.querySelector("template");
    if (tpl) {
      const fragment = tpl.content;
      const container = tpl.parentElement;
      if (!container) {
        throw new Error("template has no parent");
      }
      container.removeChild(tpl);
      return { fragment, container };
    }
    const fragment = document.createDocumentFragment();
    let n = host.firstChild;
    while (n) {
      const next = n.nextSibling;
      const keep = n.nodeType === Node.ELEMENT_NODE;
      host.removeChild(n);
      if (keep) fragment.appendChild(n);
      n = next;
    }
    if (!fragment.hasChildNodes()) {
      throw new Error("<tonk-concept> has no row template");
    }
    return { fragment, container: host };
  }

  // Walk fragment, replace any text node containing `{...}` with a
  // sequence of single-segment text nodes, build a plan of paths.
  function extractPlan(fragment) {
    const bindings = [];
    const textTargets = [];
    const attrTargets = [];

    function walk(node, path) {
      const children = node.childNodes;
      for (let i = 0; i < children.length; i++) {
        const child = children[i];
        path.push(i);
        if (child.nodeType === Node.TEXT_NODE) {
          const raw = child.textContent || "";
          if (raw.includes("{")) textTargets.push({ path: path.slice(), raw });
        } else if (child.nodeType === Node.ELEMENT_NODE) {
          const interp = [];
          for (const attr of child.attributes) {
            if (attr.value.includes("{")) interp.push({ name: attr.name, value: attr.value });
          }
          if (interp.length) attrTargets.push({ path: path.slice(), attrs: interp });
          walk(child, path);
        }
        path.pop();
      }
    }
    walk(fragment, []);

    for (const { path, raw } of textTargets) {
      const segments = parseSegments(raw);
      if (!hasField(segments)) continue;
      const node = navigate(fragment, path);
      if (!node) continue;
      const parent = node.parentNode;
      if (!parent) continue;
      const newNodes = [];
      for (const seg of segments) {
        if (seg.kind === "text") {
          newNodes.push(document.createTextNode(seg.value));
        } else {
          newNodes.push(document.createTextNode(""));
        }
      }
      let last = node;
      for (const n of newNodes) {
        parent.insertBefore(n, last.nextSibling);
        last = n;
      }
      parent.removeChild(node);

      const originalIdx = path[path.length - 1] || 0;
      const prefix = path.slice(0, -1);
      for (let i = 0; i < segments.length; i++) {
        if (segments[i].kind === "field") {
          bindings.push({
            path: prefix.concat(originalIdx + i),
            kind: "text",
            segments: [segments[i]],
          });
        }
      }
    }

    for (const { path, attrs } of attrTargets) {
      for (const { name, value } of attrs) {
        const segments = parseSegments(value);
        if (!hasField(segments)) continue;
        bindings.push({ path, kind: "attribute", attrName: name, segments });
      }
    }
    return { bindings };
  }

  function navigate(root, path) {
    let current = root;
    for (const idx of path) {
      if (!current.childNodes || idx >= current.childNodes.length) return null;
      current = current.childNodes[idx];
    }
    return current;
  }

  function renderSegments(segments, rowThis, fields) {
    let out = "";
    for (const seg of segments) {
      if (seg.kind === "text") {
        out += seg.value;
      } else if (seg.name === "this") {
        out += rowThis;
      } else if (seg.name in fields) {
        out += renderValue(fields[seg.name]);
      }
    }
    return out;
  }

  function renderValue(v) {
    if (v == null) return "";
    if (typeof v === "string") return v;
    if (typeof v === "number" || typeof v === "boolean") return String(v);
    return JSON.stringify(v);
  }

  // --- renderer ---------------------------------------------------------

  class Renderer {
    constructor(plan, template, container) {
      this.plan = plan;
      this.template = template;
      this.container = container;
      this.rows = new Map(); // entityUri -> { nodes, lastValues }
    }

    apply(frame) {
      const seen = new Set();
      for (const conclusion of frame) {
        seen.add(conclusion.this);
        if (this.rows.has(conclusion.this)) {
          this.updateRow(conclusion);
        } else {
          this.insertRow(conclusion);
        }
      }
      for (const key of [...this.rows.keys()]) {
        if (seen.has(key)) continue;
        const row = this.rows.get(key);
        this.rows.delete(key);
        for (const n of row.nodes) if (n.parentNode) n.parentNode.removeChild(n);
      }
    }

    insertRow(c) {
      const clone = this.template.cloneNode(true);
      const values = [];
      for (const b of this.plan.bindings) {
        const rendered = renderSegments(b.segments, c.this, c.fields || {});
        applyBinding(clone, b, rendered);
        values.push(rendered);
      }
      const nodes = [];
      for (const n of clone.childNodes) nodes.push(n);
      const firstEl = nodes.find((n) => n.nodeType === Node.ELEMENT_NODE);
      if (firstEl) firstEl.setAttribute("data-this", c.this);
      this.container.appendChild(clone);
      this.rows.set(c.this, { nodes, lastValues: values });
    }

    updateRow(c) {
      const row = this.rows.get(c.this);
      for (let i = 0; i < this.plan.bindings.length; i++) {
        const b = this.plan.bindings[i];
        const rendered = renderSegments(b.segments, c.this, c.fields || {});
        if (row.lastValues[i] === rendered) continue;
        patchRow(row, b, rendered);
        row.lastValues[i] = rendered;
      }
    }
  }

  function applyBinding(fragment, binding, rendered) {
    const target = navigate(fragment, binding.path);
    if (!target) return;
    writeBinding(target, binding, rendered);
  }

  function patchRow(row, binding, rendered) {
    const first = binding.path[0];
    const root = row.nodes[first];
    if (!root) return;
    const target = navigate(root, binding.path.slice(1));
    if (!target) return;
    writeBinding(target, binding, rendered);
  }

  function writeBinding(target, binding, rendered) {
    if (binding.kind === "text") {
      target.textContent = rendered;
    } else if (target.setAttribute) {
      target.setAttribute(binding.attrName, rendered);
    }
  }

  // --- SSE reader -------------------------------------------------------

  async function openSse(url, body, onFrame, onError) {
    const abort = new AbortController();
    let resp;
    try {
      resp = await fetch(url, {
        method: "POST",
        headers: { accept: "text/event-stream", "content-type": "application/json" },
        body,
        signal: abort.signal,
      });
    } catch (e) {
      throw { kind: "Network", message: `fetch: ${e}` };
    }
    if (!resp.ok) throw { kind: "Network", message: `HTTP ${resp.status}` };
    if (!resp.body) throw { kind: "Network", message: "response has no body stream" };

    const reader = resp.body.getReader();
    const decoder = new TextDecoder();
    let buffer = "";
    (async () => {
      try {
        while (true) {
          const { done, value } = await reader.read();
          if (done) break;
          buffer += decoder.decode(value, { stream: true });
          let idx;
          while ((idx = buffer.indexOf("\n\n")) >= 0) {
            const frame = buffer.slice(0, idx);
            buffer = buffer.slice(idx + 2);
            const payload = stripDataPrefix(frame);
            if (payload != null) onFrame(payload);
          }
        }
      } catch (e) {
        if (e && e.name === "AbortError") return;
        onError({ kind: "Network", message: `stream read failed: ${e}` });
      }
    })();
    return abort;
  }

  function stripDataPrefix(frame) {
    if (frame.startsWith("data: ")) return frame.slice(6);
    if (frame.startsWith("data:")) return frame.slice(5);
    return null;
  }

  // --- the custom element ----------------------------------------------

  class TonkConcept extends HTMLElement {
    static get observedAttributes() {
      return ["source", "space", "branch"];
    }

    connectedCallback() {
      // When the HTML parser creates this element, children are not
      // yet in the DOM — `connectedCallback` fires after the opening
      // tag, before the closing tag and its descendants are parsed.
      // Wait for DOMContentLoaded so the declarative `<template>` /
      // row markup is present when we snapshot.
      if (document.readyState === "loading") {
        const onReady = () => {
          document.removeEventListener("DOMContentLoaded", onReady);
          this._snapshotAndStart();
        };
        document.addEventListener("DOMContentLoaded", onReady);
      } else {
        this._snapshotAndStart();
      }
    }

    _snapshotAndStart() {
      try {
        const snap = snapshotTemplate(this);
        this._snap = snap;
        this._plan = extractPlan(snap.fragment);
      } catch (err) {
        this._dispatchError({ kind: "Descriptor", message: String(err && err.message || err) });
        return;
      }
      this._start();
    }

    disconnectedCallback() {
      if (this._abort) {
        this._abort.abort();
        this._abort = null;
      }
    }

    attributeChangedCallback() {
      if (!this._snap) return; // not connected yet
      if (this._abort) {
        this._abort.abort();
        this._abort = null;
      }
      this._renderer = null;
      this._start();
    }

    async _start() {
      try {
        const sourceAttr = this.getAttribute("source") || "";
        if (!sourceAttr) {
          this._dispatchError({ kind: "Descriptor", message: "<tonk-concept> requires a `source` attribute" });
          return;
        }
        const parsed = parseSource(sourceAttr);
        // No space/branch attrs → use a relative URL and let the
        // service worker's virtual-root rewriter resolve `/query`
        // to the iframe's own branch. Override path goes through
        // /api/* directly (no rewrite) so the agent can cross-link
        // to a known absolute repository.
        const spaceAttr = this.getAttribute("space");
        const branchAttr = this.getAttribute("branch");
        const url = (spaceAttr || branchAttr)
          ? `/api/repository/${spaceAttr || "home"}/branch/${branchAttr || "main"}/query`
          : `/query`;

        // Phase 1 — resolve the concept descriptor.
        const phase1 = await fetch(url, {
          method: "POST",
          headers: { accept: "application/json", "content-type": "application/json" },
          body: phase1Body(parsed),
        });
        if (!phase1.ok) {
          this._dispatchError({ kind: "Network", message: `phase1 HTTP ${phase1.status}` });
          return;
        }
        const conclusions = await phase1.json();
        const first = Array.isArray(conclusions) ? conclusions[0] : null;
        const source = first && first.fields && first.fields.source;
        if (typeof source !== "string") {
          this._dispatchError({ kind: "UnknownSource", message: `no concept matched '${parsed.nameOrUri}'` });
          return;
        }

        // Phase 2 — open the live subscription.
        let body;
        try {
          body = phase2Body(source, parsed.filters);
        } catch (e) {
          this._dispatchError({ kind: "Descriptor", message: `phase2: ${e}` });
          return;
        }
        this._renderer = new Renderer(this._plan, this._snap.fragment, this._snap.container);
        const abort = await openSse(
          url,
          body,
          (frame) => {
            let conclusions;
            try {
              conclusions = JSON.parse(frame);
            } catch (e) {
              this._dispatchError({ kind: "Parse", message: `frame: ${e}` });
              return;
            }
            if (this._renderer) this._renderer.apply(conclusions);
            this._dispatch("tonk-concept:result", { count: conclusions.length });
          },
          (err) => {
            this._dispatchError(err);
            this._renderer = null;
          },
        );
        this._abort = abort;
        this._dispatch("tonk-concept:connected", null);
      } catch (err) {
        this._dispatchError({ kind: "Network", message: String(err && err.message || err) });
      }
    }

    _dispatchError(detail) {
      this._dispatch("tonk-concept:error", detail);
    }

    _dispatch(name, detail) {
      this.dispatchEvent(
        new CustomEvent(name, { detail, bubbles: true, composed: true }),
      );
    }
  }

  customElements.define("tonk-concept", TonkConcept);
})();
