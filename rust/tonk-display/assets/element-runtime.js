// The author-element runtime: one live table, one generated wrapper
// per tag.
//
// A view template renders through inert fragments, so a `<script>` in
// a `show:` template never executes. Author elements are branch data
// instead — a `method` dictionary per `element:<tag>` entity — and
// this file is what turns those facts into registered custom
// elements.
//
// The load-bearing idea is the indirection. `customElements.define`
// cannot redefine a name, so a runtime that baked the author's
// functions into the class would need a page reload for every edit.
// Here the class is a WRAPPER that resolves through `table` on every
// call, so a method edit is a table write: the tag stays registered,
// live instances pick up the new function on their next call, and
// `connected` is re-run against instances already in the DOM.
//
// Loaded once per realm; every subsequent element module is a
// `defineTonkElement(tag, methods)` call appended to <head>.

(() => {
  if (globalThis.defineTonkElement) return;

  /** tag -> { method key -> function }. The single source of dispatch. */
  const table = new Map();
  /** tag -> Set of connected instances, for re-running edited hooks. */
  const live = new Map();
  /** Tags whose wrapper class is already registered. */
  const defined = new Set();

  /** `attribute-changed` -> `attributeChanged`; `bump` -> `bump`. */
  const property = (key) =>
    key.replace(/-([a-z0-9])/g, (_, c) => c.toUpperCase());

  const call = (tag, key, ...args) => {
    const fn = table.get(tag)?.[key];
    if (typeof fn !== "function") return undefined;
    try {
      return fn(...args);
    } catch (error) {
      // One element's bad method must not take down the view that
      // renders it, nor the sibling elements sharing this realm.
      console.error(`<${tag}> ${key} failed:`, error);
      return undefined;
    }
  };

  // Attribute changes go through a MutationObserver rather than
  // `attributeChangedCallback`, whose `observedAttributes` is static:
  // read once at define time, it would force authors to declare the
  // list up front and could never grow with an edited method. The
  // observer watches every attribute and needs no declaration. It
  // fires on a microtask rather than synchronously, and reports no
  // attributes present at upgrade -- `connectedCallback` replays
  // those, so a hook sees its initial values either way.
  const observers = new WeakMap();

  const watchAttributes = (tag, self) => {
    if (observers.has(self)) return;
    const observer = new MutationObserver((records) => {
      for (const record of records) {
        if (record.type !== "attributes" || !record.attributeName) continue;
        const after = self.getAttribute(record.attributeName);
        // A MutationObserver reports an attribute WRITE, not a
        // CHANGE: setting the same value again still queues a record.
        // A hook has nothing to do when nothing changed.
        if (record.oldValue === after) continue;
        call(
          tag,
          "attribute-changed",
          self,
          record.attributeName,
          record.oldValue,
          after,
        );
        // Discard what the hook itself just wrote. Observing EVERY
        // attribute is what frees an author from declaring
        // `observedAttributes`, but it also means a hook that writes
        // an attribute -- mirroring one onto another, setting a
        // `data-` flag -- would re-enter itself forever, which
        // `attributeChangedCallback` avoids only because its declared
        // list happens not to name the attribute being written.
        // Draining the queue after each call restores that property
        // without the declaration.
        observer.takeRecords();
      }
    });
    observer.observe(self, { attributes: true, attributeOldValue: true });
    observers.set(self, observer);
  };

  const replayAttributes = (tag, self) => {
    if (!table.get(tag)?.["attribute-changed"]) return;
    // The observer reports nothing about attributes already present
    // when it starts watching, so an upgrade would otherwise deliver a
    // hook its element's initial state only on the NEXT change. Replay
    // them, then drain what the replay itself wrote.
    for (const { name, value } of Array.from(self.attributes)) {
      call(tag, "attribute-changed", self, name, null, value);
    }
    observers.get(self)?.takeRecords();
  };

  const connect = (tag, self) => {
    let set = live.get(tag);
    if (!set) live.set(tag, (set = new Set()));
    set.add(self);
    if (table.get(tag)?.["attribute-changed"]) {
      watchAttributes(tag, self);
      replayAttributes(tag, self);
    }
    call(tag, "connected", self);
  };

  /**
   * Register `tag` with a wrapper class that resolves every call
   * through `table`. Idempotent: a second call for an already-defined
   * tag only refreshes the table.
   */
  const define = (tag) => {
    if (defined.has(tag)) return;
    if (customElements.get(tag)) {
      // Something else owns this name -- a built-in, or a module that
      // called `customElements.define` itself. Leave it alone rather
      // than throwing; the table entry is simply never consulted.
      console.warn(`<${tag}> is already defined; branch methods ignored`);
      defined.add(tag);
      return;
    }
    defined.add(tag);
    customElements.define(
      tag,
      class extends HTMLElement {
        connectedCallback() {
          connect(tag, this);
        }
        disconnectedCallback() {
          live.get(tag)?.delete(this);
          observers.get(this)?.disconnect();
          observers.delete(this);
          call(tag, "disconnected", this);
        }
        adoptedCallback() {
          call(tag, "adopted", this);
        }
      },
    );
  };

  /**
   * Install `methods` for `tag` and register it.
   *
   * Custom keys (anything but the four lifecycle names) are installed
   * on the wrapper's prototype camelCased, so `el.myMethod()` is
   * callable where `el['my-method']()` is not. A key that would shadow
   * an existing member is refused here as well as at authoring time:
   * hand-written notation never passes through the CLI's check.
   *
   * When `methods.define` is present it is called for the class to
   * register INSTEAD of the wrapper, and every other key is ignored --
   * the escape hatch for what a method table cannot say.
   */
  globalThis.defineTonkElement = (tag, methods) => {
    if (methods.define) {
      if (customElements.get(tag)) return;
      try {
        customElements.define(tag, methods.define());
        defined.add(tag);
      } catch (error) {
        console.error(`<${tag}> define failed:`, error);
      }
      return;
    }

    const previous = table.get(tag);
    table.set(tag, methods);
    define(tag);

    const prototype = customElements.get(tag)?.prototype;
    if (prototype) {
      for (const key of Object.keys(methods)) {
        if (LIFECYCLE.has(key)) continue;
        const name = property(key);
        if (name in HTMLElement.prototype) {
          console.error(`<${tag}> method '${key}' would shadow HTMLElement.${name}`);
          continue;
        }
        // Resolved through the table at call time, like the lifecycle
        // hooks -- so an edited custom method is live too.
        Object.defineProperty(prototype, name, {
          configurable: true,
          writable: true,
          value: function (...args) {
            return call(tag, key, this, ...args);
          },
        });
      }
    }

    // An edit only reaches instances already in the DOM if we re-run
    // the hook: call-time dispatch makes FUTURE calls live, not past
    // ones. Without this a `connected` edit would appear to do nothing
    // until the next render.
    if (previous && previous.connected !== methods.connected) {
      for (const self of live.get(tag) ?? []) call(tag, "connected", self);
    }
  };

  const LIFECYCLE = new Set([
    "connected",
    "disconnected",
    "adopted",
    "attribute-changed",
  ]);

  // Exposed for tests and for the inspector; not part of the authoring
  // surface.
  globalThis.__tonkElements = { table, live, defined };
})();
