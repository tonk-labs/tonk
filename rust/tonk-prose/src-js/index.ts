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
// Content channel — the element's *text content*, the way `<textarea>`
// carries its value: put the content between the tags
// (`<tonk-prose>…</tonk-prose>`), and a store binds it as element text.
// Text content is newline- and markup-safe (a `<` in the markdown stays
// literal, and CRLF survives) — unlike an attribute value, which a
// binding layer can normalize in transit. The content may be a versioned
// `content` envelope (content.ts: markdown + HLC ETag) or bare markdown
// (no version → always adopted). Store-driven updates are observed via a
// MutationObserver; the element drops its own round-tripped echo by the
// HLC version, so a store write-back never fights the caret.
//
// Attributes:
//   value       — legacy/convenience content source, same parsing as the
//                 text channel; the text child takes precedence when both
//                 are present.
//   readonly    — boolean attribute. Presence locks the editor.
//   placeholder — ghost text shown while the document is empty.
//   auto-focus  — boolean attribute. Focus the editor once mounted.
//
// Properties:
//   .value : string   — round-trips the document as bare markdown.
//   .content : string — round-trips as the versioned envelope.
//   .version : string — the current HLC as a decimal string (read).
//
// Events:
//   change   — CustomEvent<{value, content}>. Fires after user edits
//              go idle (debounced), coalescing a typing burst into one
//              event; programmatic writes do not refire. `value` is the
//              markdown; `content` is the versioned envelope. Writing
//              `content` back through a store round-trips the HLC, so
//              the element recognizes its own echo (version not newer)
//              and drops it — the caret is never disturbed by a
//              round-trip. A genuinely newer write patches only the
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
import { Clock, formatHlc } from "./editor/hlc";
import { parseContent, formatContent } from "./editor/content";

const OBSERVED = [
  "content",
  "value",
  "readonly",
  "placeholder",
  "auto-focus",
  "switcher",
  "caret",
] as const;
type ObservedAttr = (typeof OBSERVED)[number];

/** Idle gap before a burst of edits dispatches one `change`. Long
 *  enough to coalesce fast typing, short enough that a store commit
 *  still feels responsive after a pause. */
const CHANGE_DEBOUNCE_MS = 400;

/** Detail object dispatched on the `change` event. */
export type ChangeDetail = {
  /** Markdown serialization of the document after the edit. The plain
   *  text, for consumers that only want the value. */
  value: string;
  /** The versioned `content` envelope (value + HLC ETag). Write this
   *  back through a store and the element recognizes its own echo by
   *  the HLC, dropping it instead of fighting the caret. */
  content: string;
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
  /** Documents the heading switcher can offer, set by the host.
   *
   *  A property rather than an attribute: this is a list, read on every
   *  keystroke, and serializing it through the DOM would be both lossy and
   *  slow. The host assigns it; the switcher reads it fresh each time, so a
   *  document created elsewhere appears without remounting. */
  candidates: { title: string; href: string }[] = [];

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

  /** Observes the light-DOM text child — the primary content channel,
   *  the way `<textarea>` carries its value. A store binds the content
   *  as element text (`<tonk-prose>{content}</tonk-prose>`), which the
   *  view writes via `set_text_content`: newline- and markup-safe,
   *  unlike an attribute value that can be normalized in transit. */
  #textObserver: MutationObserver | null = null;

  /** Debounce timer for the outward `change` event. Coalesces a burst
   *  of keystrokes into one dispatch after an idle gap, so a consumer
   *  that commits each `change` to a store isn't hit per-keystroke. */
  #changeTimer: ReturnType<typeof setTimeout> | null = null;

  /** The latest edited markdown awaiting a debounced `change`
   *  dispatch, or null when nothing is pending. */
  #pendingChange: string | null = null;

  /** This element's Hybrid Logical Clock. Every outbound `change`
   *  stamps a fresh HLC; every adopted inbound `content` advances it.
   *  Monotonic, so it orders our own writes even across a wonky system
   *  clock. */
  readonly #clock = new Clock();

  /** The highest HLC this element has issued or adopted. An incoming
   *  `content` is our own echo (or otherwise not newer) exactly when
   *  its HLC is `<= this`; such writes are ignored, which is what
   *  stops a store round-trip from fighting the caret. A genuinely
   *  newer write (greater HLC, or a bare value with no HLC) is
   *  adopted. */
  #lastKnownHlc = 0n;

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

    // Watch the light-DOM text child for store-driven content updates.
    // Installed once; survives the editor's own lifecycle since the
    // editor lives in the shadow root and never touches light DOM.
    if (!this.#textObserver) {
      this.#textObserver = new MutationObserver(() => this.#onLightContent());
      this.#textObserver.observe(this, {
        childList: true,
        characterData: true,
        subtree: true,
      });
    }

    if (this.#editor) return;

    const token = ++this.#mountToken;
    void this.#mountEditor(token);
  }

  /** The element's light-DOM text — its content channel. Excludes the
   *  shadow root (where the editor renders), so it's the clean input a
   *  store writes via `set_text_content`. */
  #lightContent(): string {
    return this.textContent ?? "";
  }

  /** A light-DOM text change (the store wrote new content). Adopt it —
   *  the HLC gate drops our own echo, a newer write patches the doc. */
  #onLightContent(): void {
    this.#adopt(this.#lightContent());
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

    // Initial document, in priority order:
    //   1. a `.value`/`content` property write buffered before mount,
    //   2. the light-DOM text child (the primary content channel),
    //   3. the `content`/`value` attribute (legacy / convenience).
    // Each is parsed through the envelope so an initial versioned
    // content seeds `#lastKnownHlc` — a later echo of that same version
    // is then correctly recognized and dropped.
    let raw: string | null = this.#pendingValue;
    if (raw === null) {
      const light = this.#lightContent();
      raw = light !== "" ? light : (this.getAttribute("content") ?? this.getAttribute("value"));
    }
    let doc = "";
    if (raw !== null) {
      const parsed = parseContent(raw);
      doc = parsed.value;
      if (parsed.hlc !== null && parsed.hlc > this.#lastKnownHlc) {
        this.#lastKnownHlc = this.#clock.receive(parsed.hlc);
      }
    }

    const editor = mod.createEditor(this.#mount, {
      doc,
      readOnly: this.hasAttribute("readonly"),
      placeholder: this.getAttribute("placeholder") ?? "",
      onChange: (value) => {
        this.#scheduleChange(value);
      },
      // The heading acts as a document switcher only where the host says
      // so. Read once, at construction: an editor that could gain the
      // behaviour later could navigate away from a document being renamed.
      switcher: this.hasAttribute("switcher")
        ? {
            candidates: () => this.candidates,
            onOpen: (candidate) => this.#emit("switch", candidate),
            // The whole document, not just the title: a draft's body is
            // real content the author typed, and the notebook that gets
            // created has to carry it or naming the document throws away
            // everything written under the heading.
            onCreate: (title) =>
              this.#emit("create", { title, document: this.value }),
            onSuggest: (rows, active) => this.#emit("suggest", { rows, active }),
          }
        : undefined,
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
        if (this.#editor !== editor) return;
        // Claim the FRAME first. In a sealed guest this editor lives two
        // iframes deep, and focusing an element inside a frame the browser
        // has not focused leaves the caret invisible and the keyboard
        // pointed at the top document — the page looks ready to type in
        // and silently is not.
        try {
          window.focus();
        } catch {
          // A cross-origin parent can refuse; the element focus below
          // still works once the user clicks in.
        }
        // `caret="end"` opens ready to continue writing rather than to
        // overwrite: the caret lands after what is already there. Arriving
        // from the switcher, naming a notebook and carrying on typing is
        // then one motion.
        if (this.getAttribute("caret") === "end") editor.caretToEnd();
        editor.focus();
      }, 0);
    }
  }

  /** Dispatch a bubbling, composed event carrying `detail`. Composed so
   *  it crosses this element's shadow boundary to the host listening
   *  outside it. */
  #emit(name: string, detail: unknown) {
    this.dispatchEvent(
      new CustomEvent(name, { detail, bubbles: true, composed: true }),
    );
  }

  /** Record the latest edited markdown and (re)arm the debounce. The
   *  `change` event dispatches once the edits go idle, carrying the
   *  most recent value. */
  #scheduleChange(value: string): void {
    this.#pendingChange = value;
    if (this.#changeTimer !== null) clearTimeout(this.#changeTimer);
    this.#changeTimer = setTimeout(() => this.#flushChange(), CHANGE_DEBOUNCE_MS);
  }

  /** Dispatch the pending debounced `change`, if any. Stamps a fresh
   *  HLC and advances `#lastKnownHlc`, so when the consumer round-trips
   *  the emitted `content` back through a store the element recognizes
   *  it (HLC not newer) and drops it instead of re-applying. */
  #flushChange(): void {
    this.#changeTimer = null;
    const value = this.#pendingChange;
    this.#pendingChange = null;
    if (value === null) return;
    const hlc = this.#clock.tick();
    this.#lastKnownHlc = hlc;
    const content = formatContent({ hlc, value });
    this.dispatchEvent(
      new CustomEvent<ChangeDetail>("change", {
        detail: { value, content },
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
      this.#textObserver?.disconnect();
      this.#textObserver = null;
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
      case "content":
        this.#adopt(next ?? "");
        break;
      case "value":
        // Reflect attribute → property only when they diverge so we
        // don't fight the user's keystrokes. `value=` is the bare
        // (un-versioned) channel; it flows through the same adoption.
        if ((next ?? "") !== this.value) {
          this.#adopt(next ?? "");
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

  /** Adopt an incoming write — either a versioned `content` envelope
   *  or a bare markdown string (which parses as an HLC-less value and
   *  is always adopted). The HLC gate is the whole point: a write whose
   *  HLC is not newer than what we've issued/seen is our own echo (or
   *  otherwise stale) and is ignored, so a store round-trip can't
   *  disturb the caret; a genuinely newer write advances our clock and
   *  patches the document. */
  #adopt(raw: string): void {
    const { hlc, value } = parseContent(raw);
    if (hlc !== null) {
      if (hlc <= this.#lastKnownHlc) return; // our echo, or not newer
      this.#lastKnownHlc = this.#clock.receive(hlc);
    }
    if (!this.#editor) {
      // Editor core still loading (or element not yet connected):
      // buffer the value, the mount applies it.
      this.#pendingValue = value;
      return;
    }
    // `setMarkdown` no-ops when `value` already matches the buffer and
    // otherwise replaces only the span that differs, preserving the
    // caret — so a genuine out-of-band change doesn't reset the view.
    this.#editor.setMarkdown(value);
  }

  /** Current document as markdown (the bare value — a subset of
   *  `content`). */
  get value(): string {
    if (this.#editor) return this.#editor.getMarkdown();
    if (this.#pendingValue !== null) return this.#pendingValue;
    const light = this.#lightContent();
    const raw =
      light !== "" ? light : (this.getAttribute("content") ?? this.getAttribute("value"));
    return raw === null ? "" : parseContent(raw).value;
  }

  set value(next: string) {
    this.#adopt(next);
  }

  /** Current document as a versioned `content` envelope: the markdown
   *  plus the last HLC this element issued or adopted. Writing this
   *  back through a store round-trips the version, so the element drops
   *  its own echo. */
  get content(): string {
    return formatContent({ hlc: this.#lastKnownHlc, value: this.value });
  }

  set content(next: string) {
    this.#adopt(next);
  }

  /** The last HLC this element issued or adopted, as a decimal string
   *  (`"0"` before any edit). Read-only view for consumers that want
   *  the version without parsing `content`. */
  get version(): string {
    return formatHlc(this.#lastKnownHlc);
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

/** Host stylesheet. Each `--tonk-prose-*` variable is the theming
 *  contract, and its DEFAULT inherits the page's design tokens where the
 *  consumer exposes them — WebAwesome `--wa-*` tokens (the tonk app's
 *  palette: a yellow-green brand, adaptive surfaces) — falling back to a
 *  self-contained GitHub-flavored value so the element still looks right
 *  mounted on a bare page. A consumer can still override any
 *  `--tonk-prose-*` directly. Because custom properties inherit through
 *  the shadow boundary, links, highlights, surfaces, and code read the
 *  same as the surrounding page when those `--wa-*` tokens are present.
 *  The editor core adds the rules for the rendered document (headings,
 *  marks, code blocks, the syntax-marker reveal) when it mounts. */
const SHADOW_STYLESHEET = `
  :host {
    --tonk-prose-font: var(--wa-font-family-body, ui-sans-serif, -apple-system,
                       "Segoe UI", Helvetica, Arial, sans-serif);
    --tonk-prose-mono: var(--wa-font-family-code, ui-monospace, SFMono-Regular,
                       Menlo, Consolas, "Liberation Mono", monospace);
    --tonk-prose-heading-font: var(--wa-font-family-heading,
                       var(--tonk-prose-font));
    --tonk-prose-font-size: var(--wa-font-size-m, 1rem);
    --tonk-prose-radius: var(--wa-border-radius-m, 6px);
    --tonk-prose-padding: 1rem 1.25rem;
    --tonk-prose-max-width: none;

    /* Surfaces & text — inherit the page's WebAwesome tokens, GitHub
       light values as the standalone fallback. */
    --tonk-prose-bg: var(--wa-color-surface-default, #ffffff);
    --tonk-prose-fg: var(--wa-color-text-normal, #1f2328);
    --tonk-prose-fg-muted: var(--wa-color-text-quiet, #59636e);
    --tonk-prose-border: var(--wa-color-neutral-border-quiet, #d1d9e0);
    /* Links → the page's dedicated link color (readable on any surface);
       accent (caret, focus ring) → the yellow-green brand. */
    --tonk-prose-link: var(--wa-color-text-link, #0969da);
    --tonk-prose-accent: var(--wa-color-brand-fill-loud, #0969da);
    --tonk-prose-selection: var(--wa-color-brand-fill-quiet, #0969da33);
    --tonk-prose-focus-ring: var(--wa-color-brand-border-normal, #0969da66);
    /* Revealed markdown syntax markers (the Typora trick). */
    --tonk-prose-marker: var(--wa-color-text-quiet, #9198a1);
    /* Inline code + code block surfaces. */
    --tonk-prose-code-bg: var(--wa-color-neutral-fill-quiet, #f6f8fa);
    --tonk-prose-code-fg: var(--wa-color-text-normal, #1f2328);
    --tonk-prose-blockquote: var(--wa-color-text-quiet, #59636e);
    /* Highlight (== marks) → the page's LOUD brand fill (bright
       yellow-green) with its matching on-color, a readable dark-on-bright
       pairing in both themes (the normal fill is too dark for text). */
    --tonk-prose-highlight-bg: var(--wa-color-brand-fill-loud, #fef08a);
    --tonk-prose-highlight-fg: var(--wa-color-brand-on-loud, #1f2328);

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

  /* Standalone dark fallback (no WebAwesome tokens present). When the page
     provides \`--wa-*\` the rules above already track its light/dark
     palette, so this only bites a bare page in dark mode. */
  @media (prefers-color-scheme: dark) {
    :host {
      --tonk-prose-bg: var(--wa-color-surface-default, #0d1117);
      --tonk-prose-fg: var(--wa-color-text-normal, #f0f6fc);
      --tonk-prose-fg-muted: var(--wa-color-text-quiet, #9198a1);
      --tonk-prose-border: var(--wa-color-neutral-border-quiet, #3d444d);
      --tonk-prose-link: var(--wa-color-text-link, #48b9f4);
      --tonk-prose-accent: var(--wa-color-brand-fill-loud, #1f6feb);
      --tonk-prose-selection: var(--wa-color-brand-fill-quiet, #1f6feb59);
      --tonk-prose-focus-ring: var(--wa-color-brand-border-normal, #1f6feb99);
      --tonk-prose-marker: var(--wa-color-text-quiet, #6e7681);
      --tonk-prose-code-bg: var(--wa-color-neutral-fill-quiet, #151b23);
      --tonk-prose-code-fg: var(--wa-color-text-normal, #f0f6fc);
      --tonk-prose-blockquote: var(--wa-color-text-quiet, #9198a1);
      --tonk-prose-highlight-bg: var(--wa-color-brand-fill-loud, #fef08a);
      --tonk-prose-highlight-fg: var(--wa-color-brand-on-loud, #1f2328);
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
