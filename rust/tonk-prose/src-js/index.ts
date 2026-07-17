// `<tonk-prose>` — ProseMirror packaged as a custom element with
// Typora-style markdown editing: the document renders rich, but the
// markdown syntax around the caret reveals itself for editing and
// collapses again when the caret leaves.
//
// THIS FILE IS THE SHELL. It registers the custom element and nothing
// else — no ProseMirror import reaches this chunk (type-only imports
// are erased at build time). The editor core (schema, plugins,
// markdown round-trip) lives in `tonk-prose-editor.js`, dynamically
// imported the first time an element connects. Pages that ship the
// bundle but never mount an editor pay only for this shell; pages
// that mount many editors fetch the core exactly once.
//
// Attributes (all reflected on the host element):
//   value       — initial markdown source. Reactive: writes after
//                 connect replace the document in-place.
//   readonly    — boolean attribute. Presence locks the editor.
//   placeholder — ghost text shown while the document is empty.
//   auto-focus  — boolean attribute. Focus the editor once mounted.
//
// Properties:
//   .value : string — round-trips with the document as markdown.
//                     Reads serialize the current doc; writes parse
//                     and replace it.
//
// Events:
//   change   — CustomEvent<{value}>. Fires after user edits go idle
//              (debounced), coalescing a typing burst into one event;
//              programmatic `.value` writes do not refire. `value` is
//              the markdown serialization of the current document. A
//              value written back into `.value` that matches one we
//              just dispatched (a store round-trip of our own edit) is
//              recognized as an echo and dropped, so it can't disturb
//              the caret; a genuine out-of-band value replaces only the
//              changed span (see the editor's `setMarkdown`).
//   ready    — fires once, after the editor core chunk has loaded
//              and the ProseMirror view is mounted.
//
// Code blocks: rendered as embedded code editors. When the
// `<tonk-code>` element is defined on the page the node view mounts
// one (inheriting its lazily-loaded language chunks + LSP wiring);
// otherwise a styled plain-text fallback keeps the block editable.
//
// Theming runs through `--tonk-prose-*` CSS custom properties so the
// element stays decoupled from any consumer's design system.

import type { ProseEditor, EditorModule } from "./editor/api";

const OBSERVED = ["value", "readonly", "placeholder", "auto-focus"] as const;
type ObservedAttr = (typeof OBSERVED)[number];

/** Idle gap before a burst of edits dispatches one `change`. Long
 *  enough to coalesce fast typing, short enough that a store commit
 *  still feels responsive after a pause. */
const CHANGE_DEBOUNCE_MS = 400;

/** How many dispatched-but-not-yet-echoed values to remember for the
 *  round-trip drop. A handful covers overlapping in-flight commits; a
 *  value whose echo never lands is evicted once the cap is passed. */
const SENT_VALUES_CAP = 8;

/** Detail object dispatched on the `change` event. */
export type ChangeDetail = {
  /** Markdown serialization of the document after the edit. */
  value: string;
};

/** Detail object dispatched on the `ready` event. */
export type ReadyDetail = {
  /** The mounted editor handle. Escape hatch for callers that want
   *  to reach past the attribute surface (e.g. to grab the raw
   *  ProseMirror `EditorView`). Power-user surface — most consumers
   *  should stick to attributes and events. */
  editor: ProseEditor;
};

/** Resolve the URL of the editor-core chunk.
 *
 *  Default: relative to this module — `import.meta.url` is the URL
 *  `tonk-prose.js` was loaded from, so the core chunk lives next to
 *  it regardless of where the consumer mounted the bundle (root,
 *  /assets/, CDN, …).
 *
 *  Override: `window.__tonkProseEditor`. Sealed environments that
 *  import this shell from a minted `blob:` URL (tonk-portal guests)
 *  have no working relative resolution — the injector provides this
 *  global instead. Either a URL string (core already minted), or a
 *  function returning the URL / a promise of it — the lazy form:
 *  the guest asks its trusted parent for the core's bytes only when
 *  the first element actually connects, keeping the boot payload at
 *  the shell alone. */
async function editorChunkUrl(): Promise<string> {
  const override = (globalThis as { __tonkProseEditor?: unknown })
    .__tonkProseEditor;
  if (typeof override === "string" && override) return override;
  if (typeof override === "function") {
    const url = await (override as () => string | Promise<string>)();
    if (typeof url === "string" && url) return url;
  }
  return new URL("./tonk-prose-editor.js", import.meta.url).href;
}

/** Single in-flight / resolved load of the editor core, shared by
 *  every element instance. Cleared on failure so a later connect can
 *  retry after e.g. a network blip. */
let editorModule: Promise<EditorModule> | null = null;

function loadEditorModule(): Promise<EditorModule> {
  if (!editorModule) {
    editorModule = editorChunkUrl().then(
      (url) =>
        import(/* @vite-ignore */ url).then((mod) => mod as EditorModule),
    );
    editorModule.catch(() => {
      editorModule = null;
    });
  }
  return editorModule;
}

class TonkProseElement extends HTMLElement {
  static get observedAttributes(): readonly string[] {
    return OBSERVED;
  }

  /** Shadow root the editor mounts into. Same rationale as
   *  `<tonk-code>`: ProseMirror injects its stylesheet via
   *  `style-mod` onto the element's *root node*, so owning a shadow
   *  root keeps the editor styled no matter which outer shadow tree
   *  (e.g. `<wa-page>`) the host lands in. */
  readonly #shadow: ShadowRoot;

  /** Mount point handed to the editor core. */
  readonly #mount: HTMLDivElement;

  /** Live editor handle once the core chunk resolved and the view
   *  mounted. Null while loading and after teardown. */
  #editor: ProseEditor | null = null;

  /** Monotonic mount token. Bumped on every connect *and* teardown;
   *  an async mount whose token no longer matches discards its
   *  editor instead of wiring it up (covers connect → disconnect →
   *  reconnect races while the chunk is in flight). */
  #mountToken = 0;

  /** `.value` writes that land while the core chunk is still
   *  loading. Applied (last-write-wins) when the editor mounts. */
  #pendingValue: string | null = null;

  /** Debounce timer for the outward `change` event. Coalesces a burst
   *  of keystrokes into one dispatch after an idle gap, so a consumer
   *  that commits each `change` to a store isn't hit per-keystroke. */
  #changeTimer: ReturnType<typeof setTimeout> | null = null;

  /** The latest edited markdown awaiting a debounced `change`
   *  dispatch, or null when nothing is pending. */
  #pendingChange: string | null = null;

  /** Values we've dispatched via `change` but not yet seen written
   *  back. When a consumer round-trips our own edit through a store
   *  and back into `value`, the incoming write matches one of these
   *  and is dropped instead of re-applied — that's what stops the
   *  store echo from fighting the caret. Keyed by value string;
   *  bounded (a late echo that never arrives is evicted once the set
   *  passes a small cap, so it can't grow without bound). */
  #sentValues: Set<string> = new Set();

  /** Pending teardown scheduled by `disconnectedCallback`. Reactive
   *  frameworks (Leptos, mostly) detach and re-attach DOM nodes
   *  during rerenders; deferring the teardown one macrotask lets a
   *  re-attach cancel it, preserving editor state (undo history,
   *  selection) across the move. */
  #teardownScheduled = false;

  constructor() {
    super();
    this.#shadow = this.attachShadow({ mode: "open", delegatesFocus: true });

    const style = document.createElement("style");
    style.textContent = SHADOW_STYLESHEET;

    this.#mount = document.createElement("div");
    this.#mount.className = "mount";

    this.#shadow.append(style, this.#mount);
  }

  connectedCallback(): void {
    // If we were about to tear down (framework move), cancel — the
    // re-attach happened first and the live editor carries on.
    this.#teardownScheduled = false;
    if (this.#editor) return;

    const token = ++this.#mountToken;
    void this.#mountEditor(token);
  }

  async #mountEditor(token: number): Promise<void> {
    let mod: EditorModule;
    try {
      mod = await loadEditorModule();
    } catch (err) {
      // A missing/broken chunk shouldn't crash the host page. The
      // element renders empty; a later reconnect retries the load.
      console.warn("[tonk-prose] failed to load editor core:", err);
      return;
    }
    // Stale mount: the element disconnected (or reconnected, which
    // bumps the token and starts its own mount) while the chunk was
    // in flight.
    if (token !== this.#mountToken || !this.isConnected) return;

    const editor = mod.createEditor(this.#mount, {
      doc: this.#pendingValue ?? this.getAttribute("value") ?? "",
      readOnly: this.hasAttribute("readonly"),
      placeholder: this.getAttribute("placeholder") ?? "",
      onChange: (value) => {
        this.#scheduleChange(value);
      },
    });
    this.#pendingValue = null;
    this.#editor = editor;

    this.dispatchEvent(
      new CustomEvent<ReadyDetail>("ready", {
        detail: { editor },
        bubbles: true,
        composed: true,
      }),
    );

    // Deferred to the next macrotask so sibling mounts settling in
    // the same tick can't clobber the focus (mirrors `<tonk-code>`).
    if (!this.hasAttribute("readonly") && this.hasAttribute("auto-focus")) {
      setTimeout(() => {
        if (this.#editor === editor) editor.focus();
      }, 0);
    }
  }

  /** Record the latest edited markdown and (re)arm the debounce. The
   *  `change` event dispatches once the edits go idle, carrying the
   *  most recent value. */
  #scheduleChange(value: string): void {
    this.#pendingChange = value;
    if (this.#changeTimer !== null) clearTimeout(this.#changeTimer);
    this.#changeTimer = setTimeout(() => this.#flushChange(), CHANGE_DEBOUNCE_MS);
  }

  /** Dispatch the pending debounced `change`, if any. Records the
   *  value as sent so a consumer round-tripping it back into `value`
   *  is recognized as our own echo and dropped. */
  #flushChange(): void {
    this.#changeTimer = null;
    const value = this.#pendingChange;
    this.#pendingChange = null;
    if (value === null) return;
    // Remember what we emitted so the store echo can be dropped.
    // Cap the set so a value that never round-trips can't leak.
    this.#sentValues.add(value);
    if (this.#sentValues.size > SENT_VALUES_CAP) {
      const oldest = this.#sentValues.values().next().value;
      if (oldest !== undefined) this.#sentValues.delete(oldest);
    }
    this.dispatchEvent(
      new CustomEvent<ChangeDetail>("change", {
        detail: { value },
        bubbles: true,
        composed: true,
      }),
    );
  }

  disconnectedCallback(): void {
    if (this.#teardownScheduled) return;
    this.#teardownScheduled = true;
    setTimeout(() => {
      if (!this.#teardownScheduled) return;
      this.#teardownScheduled = false;
      // `isConnected` flips back to true if the framework
      // re-inserted us within the deferral window.
      if (this.isConnected) return;
      // Flush any debounced edit before tearing down, so the last
      // keystrokes before removal aren't lost.
      if (this.#changeTimer !== null) {
        clearTimeout(this.#changeTimer);
        this.#flushChange();
      }
      this.#mountToken++;
      this.#editor?.destroy();
      this.#editor = null;
    }, 0);
  }

  attributeChangedCallback(
    name: ObservedAttr,
    _old: string | null,
    next: string | null,
  ): void {
    switch (name) {
      case "value":
        // Reflect attribute → property only when they diverge so we
        // don't fight the user's keystrokes.
        if ((next ?? "") !== this.value) {
          this.value = next ?? "";
        }
        break;
      case "readonly":
        this.#editor?.setReadOnly(next !== null);
        break;
      case "placeholder":
        this.#editor?.setPlaceholder(next ?? "");
        break;
      case "auto-focus":
        // Applied at mount only — stealing focus on a post-mount
        // attribute write would surprise the user.
        break;
    }
  }

  /** Current document as markdown. */
  get value(): string {
    if (this.#editor) return this.#editor.getMarkdown();
    if (this.#pendingValue !== null) return this.#pendingValue;
    return this.getAttribute("value") ?? "";
  }

  set value(next: string) {
    // Our own edit, round-tripped back through a store: drop it. The
    // value matches one we dispatched via `change` and haven't yet
    // seen written back, so re-applying it would only disturb the
    // caret (and any edits made since). Consume the token so a later
    // genuine write of the same text still lands.
    if (this.#sentValues.delete(next)) return;

    if (!this.#editor) {
      // Editor core still loading (or element not yet connected):
      // buffer the write, the mount applies it.
      this.#pendingValue = next;
      return;
    }
    // `setMarkdown` no-ops when `next` already matches the buffer and
    // otherwise replaces only the blocks that differ, preserving the
    // caret — so a genuine out-of-band change doesn't reset the view.
    this.#editor.setMarkdown(next);
  }

  /** Move keyboard focus into the document. */
  focus(): void {
    if (this.#editor) {
      this.#editor.focus();
    } else {
      super.focus();
    }
  }

  /** The live editor handle, if mounted. Power-user surface. */
  get editor(): ProseEditor | null {
    return this.#editor;
  }
}

/** Host stylesheet. Two layers of `--tonk-prose-*` variables — the
 *  document palette (adapting to `prefers-color-scheme`) and the
 *  host frame — form the theming contract; the element imports
 *  nothing from the consumer's design system. The editor core adds
 *  its own rules for the rendered document (headings, marks, code
 *  blocks, the syntax-marker reveal) when it mounts. */
const SHADOW_STYLESHEET = `
  :host {
    --tonk-prose-font: ui-sans-serif, -apple-system, "Segoe UI", Helvetica,
                       Arial, sans-serif;
    --tonk-prose-mono: ui-monospace, SFMono-Regular, Menlo, Consolas,
                       "Liberation Mono", monospace;
    --tonk-prose-font-size: 1rem;
    --tonk-prose-radius: 6px;
    --tonk-prose-padding: 1rem 1.25rem;
    --tonk-prose-max-width: none;

    /* Surfaces & text — GitHub light defaults */
    --tonk-prose-bg: #ffffff;
    --tonk-prose-fg: #1f2328;
    --tonk-prose-fg-muted: #59636e;
    --tonk-prose-border: #d1d9e0;
    --tonk-prose-accent: #0969da;
    --tonk-prose-selection: #0969da33;
    --tonk-prose-focus-ring: #0969da66;
    /* Revealed markdown syntax markers (the Typora trick). */
    --tonk-prose-marker: #9198a1;
    /* Inline code + code block surfaces. */
    --tonk-prose-code-bg: #f6f8fa;
    --tonk-prose-code-fg: #1f2328;
    --tonk-prose-blockquote: #59636e;

    display: block;
    position: relative;
    box-sizing: border-box;
    background: var(--tonk-prose-bg);
    color: var(--tonk-prose-fg);
    border: 1px solid var(--tonk-prose-border);
    border-radius: var(--tonk-prose-radius);
    overflow: hidden;
    transition: border-color 120ms ease, box-shadow 120ms ease;
  }

  @media (prefers-color-scheme: dark) {
    :host {
      --tonk-prose-bg: #0d1117;
      --tonk-prose-fg: #f0f6fc;
      --tonk-prose-fg-muted: #9198a1;
      --tonk-prose-border: #3d444d;
      --tonk-prose-accent: #1f6feb;
      --tonk-prose-selection: #1f6feb59;
      --tonk-prose-focus-ring: #1f6feb99;
      --tonk-prose-marker: #6e7681;
      --tonk-prose-code-bg: #151b23;
      --tonk-prose-code-fg: #f0f6fc;
      --tonk-prose-blockquote: #9198a1;
    }
  }

  :host([hidden]) { display: none; }

  :host(:focus-within) {
    border-color: var(--tonk-prose-accent);
    box-shadow: 0 0 0 2px var(--tonk-prose-focus-ring);
  }

  .mount { height: 100%; }
`;

if (!customElements.get("tonk-prose")) {
  customElements.define("tonk-prose", TonkProseElement);
}

export type { TonkProseElement };
