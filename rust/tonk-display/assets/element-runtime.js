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

  /** Call `fn` as `<tag>`'s `key`, containing anything it throws. */
  const invoke = (tag, key, fn, ...args) => {
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

  /** Call `<tag>`'s CURRENT `key`, whatever the table now holds. */
  const call = (tag, key, ...args) => invoke(tag, key, table.get(tag)?.[key], ...args);

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

    // An edit only reaches instances already in the DOM if something
    // re-runs against them: call-time dispatch makes FUTURE calls live,
    // not past ones. Without this a `connected` edit would appear to do
    // nothing until the next render.
    if (!previous || !changed(previous, methods)) return;
    const instances = [...(live.get(tag) ?? [])];

    // The OUTGOING definition gets to tear down first, and it has to be
    // called through its own function rather than the table, which now
    // holds the replacement. Without this the default below would run
    // `connected` a second time over setup the previous definition left
    // behind — a listener, a timer, an observer per edit, with no way
    // for an author to undo any of it.
    for (const self of instances) {
      invoke(tag, "released", previous.released, self);
    }

    for (const self of instances) {
      if (methods.swapped) {
        // The incoming definition said how to take over a live
        // instance, so it decides — `connected` is for a fresh mount.
        call(tag, "swapped", self);
      } else if (previous.connected !== methods.connected) {
        // No migration declared: re-run `connected`, which is what an
        // author who has not thought about swapping expects.
        call(tag, "connected", self);
      }
    }
  };

  /** Whether two method tables differ in any key. */
  const changed = (before, after) => {
    const keys = new Set([...Object.keys(before), ...Object.keys(after)]);
    for (const key of keys) {
      if (before[key] !== after[key]) return true;
    }
    return false;
  };

  const LIFECYCLE = new Set([
    "connected",
    "disconnected",
    "adopted",
    "attribute-changed",
    "released",
    "swapped",
  ]);

  // Tags already announced, so a tag is asked about once per realm
  // rather than once per occurrence. Realm-global because
  // `customElements` is: one registration serves every instance.
  const announced = new Set();

  /** Tag prefixes whose elements are registered by someone else. */
  const FOREIGN = ["tonk-", "wa-"];

  /** The event announcing that an undefined custom element rendered. */
  const NEEDED = "tonk-element-needed";

  const mine = (tag) =>
    tag.includes("-") && !FOREIGN.some((prefix) => tag.startsWith(prefix));

  /**
   * Announce every undefined custom element under `root`, once per tag.
   *
   * `:not(:defined)` is the browser's own answer to "what is on this
   * page that nobody has registered", so nothing has to be declared,
   * mounted, or scanned ahead of time: a view renders `<tally-widget>`
   * and the tag is announced because it is THERE. An element stays
   * inert until its definition lands and then upgrades in place, which
   * is what makes announcing after render safe.
   *
   * The event is dispatched ON THE ELEMENT and bubbles, not on
   * `document` directly. Two reasons: a handler reads routing context
   * (`with="branch@repo"`) off the element's ancestors, which a
   * document-level dispatch would have thrown away; and anything that
   * renders into a corner of the DOM this observer cannot see -- a
   * shadow root, a detached fragment -- can announce its own tags the
   * same way, and the same listener services them.
   */
  const announce = (root) => {
    const found = new Map();
    const consider = (el) => {
      const tag = el.tagName.toLowerCase();
      if (!mine(tag) || customElements.get(tag) || found.has(tag)) return;
      found.set(tag, el);
    };
    if (root instanceof Element) consider(root);
    for (const el of root.querySelectorAll?.(":not(:defined)") ?? []) consider(el);
    for (const [tag, el] of found) {
      if (announced.has(tag)) continue;
      const event = new CustomEvent(NEEDED, {
        bubbles: true,
        composed: true,
        cancelable: true,
        detail: { tag },
      });
      el.dispatchEvent(event);
      // Remember the tag only if someone CLAIMED the announcement, the
      // same way a tonk consumer event is claimed. An announcement made
      // before any listener exists would otherwise be the only one ever
      // made and the tag would stay inert for good; leaving it unmarked
      // means the next APPEARANCE of the tag offers it again. (Only the
      // next appearance: the observer reports added subtrees, so an
      // element sitting unclaimed where it already is does not
      // re-announce. The ordering that actually matters is guaranteed
      // elsewhere — an installer adds its listener before this module
      // evaluates.)
      //
      // Claiming says "mine to answer", not "answered": a listener that
      // finds no definition still claims, so an unknown tag costs one
      // lookup rather than one per mutation.
      if (event.defaultPrevented) announced.add(tag);
    }
  };

  const observer = new MutationObserver((records) => {
    for (const record of records) {
      for (const node of record.addedNodes) {
        if (node.nodeType === Node.ELEMENT_NODE) announce(node);
      }
    }
  });

  /**
   * Start watching the document for undefined custom elements.
   * Idempotent.
   *
   * Nothing here knows how a tag is resolved -- it only says one is
   * needed. Whatever listens for `tonk-element-needed` and calls
   * `defineTonkElement` is a separate concern, installed once at
   * bootstrap so no page can forget it.
   */
  globalThis.startTonkElements = () => {
    announce(document);
    observer.observe(document.documentElement, { subtree: true, childList: true });
  };

  /** Announce the tags under `root` by hand (a shadow root, say). */
  globalThis.announceTonkElements = (root) => announce(root ?? document);

  // Start on our own evaluation rather than waiting to be called. This
  // file is injected as a module script, which evaluates on a later
  // task than the insertion that appended it — so an installer cannot
  // append it and then call `startTonkElements()` synchronously, and
  // one that tried would silently never start watching.
  globalThis.startTonkElements();

  // Exposed for tests and for the inspector; not part of the authoring
  // surface.
  globalThis.__tonkElements = { table, live, defined, announced, announce, NEEDED };
})();
