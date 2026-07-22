// `<tonk-table>` — the IronCalc spreadsheet engine packaged as a custom
// element with a framework-free grid: cells evaluate formulas live,
// selection/navigation follow canonical spreadsheet semantics (they
// live in the engine), and the workbook round-trips through a store as
// a versioned envelope.
//
// THIS FILE IS THE SHELL. It registers the custom element and nothing
// else — no engine or grid code reaches this chunk (type-only imports
// are erased at build time). The grid core (engine wasm + DOM grid)
// lives in `tonk-table-grid.js`, dynamically imported the first time an
// element connects; the multi-megabyte engine bytes live one level
// deeper in `tonk-table-engine.js`, pulled in by the grid. Pages that
// ship the bundle but never mount a table pay only for this shell;
// pages that mount many tables fetch the core exactly once.
//
// TWO DOCUMENT MODES, selected by the `subject` attribute:
//
// STANDALONE (no `subject`) — the element's *text content* is the
// content channel, the way `<textarea>` carries its value: put the
// content between the tags and a store binds it as element text. The
// content may be:
//
//   - a versioned `content` envelope (content.ts: headers + HLC ETag)
//     whose body is base64 of the engine's own binary workbook
//     serialization — the LOSSLESS channel the element emits and
//     round-trips through a store, or
//   - bare CSV (no version → always adopted), the human-authorable
//     channel: `<tonk-table>a,b\n1,2</tonk-table>` just works, and
//     formulas ride as `=…` cell text.
//
// Store-driven updates are observed via a MutationObserver; the element
// drops its own round-tripped echo by the HLC version, so a store
// write-back never fights the active cell.
//
// CLAIMS (`subject` present) — the workbook lives in the dialog store
// as INDIVIDUATED claims: one instance per sheet, one per non-empty
// cell. The view materializes each claim as a hidden data-row div in
// the element's light DOM (the board pattern):
//
//   <tonk-table subject={this} oneditcell=table/edit-cell …>
//     <div hidden>
//       <tonk-display model=tonk:table/sheet></tonk-display>
//       <tonk-display model=tonk:table/cell></tonk-display>
//     </div>
//   </tonk-table>
//
// The element observes the rows (claims IN, applied to the engine as
// per-cell writes — never a model reload) and emits one typed
// CustomEvent per user mutation (claims OUT): `createsheet`,
// `renamesheet`, `createcell`, `editcell`, `clearcell` — each bound by
// the view to a command whose rule asserts/retracts the one claim.
// Concurrent editors merge at cell grain; an optimistic-echo ledger
// keeps our own in-flight writes painted until they round-trip. No
// envelope and no `change` event in this mode — the store is the
// document.
//
// Attributes:
//   subject     — the workbook entity this element renders; presence
//                 selects claims mode.
//   content     — standalone content source (same parsing as the text
//                 channel); the text child takes precedence.
//   value       — legacy/convenience standalone content source.
//   readonly    — boolean attribute. Presence locks the grid.
//   auto-focus  — boolean attribute. Focus the grid once mounted.
//   min-rows,
//   min-cols    — minimum rendered grid extent (defaults 100×26); small
//                 embeds shrink it, content/selection still grow it.
//
// Programmatic surface — `el.grid` (null until `ready`): setCell /
// getCell / addSheet / setColumnWidth / setRowHeight / insertRows /
// deleteRows / insertColumns / deleteColumns / select / selection /
// toCsv / serialize / model. Every mutating method routes through the
// same commit path as typing, so repaint, events, and persistence fire
// identically. `selectionchange` (both modes) reports movement.
//
// Properties:
//   .value : string   — CSV of the ACTIVE sheet (raw cell content, so
//                       formulas survive). The human-readable, LOSSY
//                       view; assignment adopts like the text channel.
//   .content : string — the versioned envelope around the LOSSLESS
//                       workbook bytes (base64).
//   .version : string — the current HLC as a decimal string (read).
//   .grid             — live grid handle (`null` until `ready`);
//                       `.model` exposes the raw IronCalc model.
//
// Events:
//   change   — CustomEvent<{value, content}>. Fires after user edits go
//              idle (debounced), coalescing an edit burst into one
//              event; programmatic writes do not refire. `value` is the
//              active sheet's CSV; `content` is the versioned envelope.
//              Writing `content` back through a store round-trips the
//              HLC, so the element recognizes its own echo (version not
//              newer) and drops it.
//   ready    — fires once per mount, after the grid core chunk loaded,
//              the engine wasm instantiated, and the grid rendered.
//
// Restyling — three layers, weakest to strongest, so the WHOLE look
// is overridable from outside:
//
//   1. `--tonk-table-*` custom properties — semantic theming (colors,
//      fonts, sizes), inheriting the page's `--wa-*` tokens by
//      default. Plus plain host CSS (`tonk-table { height: … }`).
//   2. `::part()` — every structural element exports a part: frame,
//      formula-bar, cell-reference, formula-input, body, grid, corner,
//      column-header, row-header, row, cell (state tokens: number /
//      error / selected / range), tab-strip, tab (token: active),
//      add-sheet, cell-editor, sheet-rename. Full CSS power per part:
//      `tonk-table::part(cell selected) { outline-color: hotpink }`.
//   3. `<style>` children — any style element placed inside
//      `<tonk-table>` is adopted into the shadow root LAST (it wins
//      the cascade), for rules parts can't express (e.g. zebra rows:
//      `tbody tr:nth-child(even) td { … }`). Views restyle per-use by
//      authoring one; live-updated via the light-DOM observer.

import type { GridMode, GridModule, TableGrid, TableSource } from "./grid/api";
import {
  readCellRows,
  readColumnRows,
  readRowSizeRows,
  readSheetRows,
} from "./grid/claims";
import { Clock, formatHlc } from "./hlc";
import {
  type Content,
  WORKBOOK_TYPE,
  formatContent,
  isWorkbookType,
  parseContent,
} from "./content";
import { base64ToBytes, bytesToBase64 } from "./b64";

const OBSERVED = [
  "subject",
  "content",
  "value",
  "readonly",
  "auto-focus",
  "min-rows",
  "min-cols",
] as const;
type ObservedAttr = (typeof OBSERVED)[number];

/** The data-row attributes whose mutations mean "the store changed a
 *  claim" in claims mode. The light-DOM observer filters on these (plus
 *  structural childList changes) so a row update triggers one debounced
 *  re-read. */
const ROW_ATTRS = [
  "subject",
  "data-table",
  "data-name",
  "data-order",
  "data-sheet",
  "data-at",
  "data-content",
  "data-style",
  "data-width",
  "data-height",
];

/** Debounce for claims-mode row re-reads: the renderer updates row
 *  divs in bursts (one per changed claim); one tick folds a burst into
 *  one engine reconcile. Matches the board's observer cadence. */
const ROWS_DEBOUNCE_MS = 30;

/** Idle gap before a burst of edits dispatches one `change`. Long
 *  enough to coalesce rapid cell entry, short enough that a store
 *  commit still feels responsive after a pause. */
const CHANGE_DEBOUNCE_MS = 400;

/** Detail object dispatched on the `change` event. */
export type ChangeDetail = {
  /** CSV of the active sheet after the edit (raw cell content, so
   *  formulas survive as `=…`). The human-readable, lossy view. */
  value: string;
  /** The versioned `content` envelope (base64 workbook bytes + HLC
   *  ETag). Write this back through a store and the element recognizes
   *  its own echo by the HLC, dropping it instead of reloading. */
  content: string;
};

/** Detail object dispatched on the `ready` event. */
export type ReadyDetail = {
  /** The mounted grid handle. Escape hatch for callers that want to
   *  reach past the attribute surface (e.g. to grab the raw IronCalc
   *  model). Power-user surface — most consumers should stick to
   *  attributes and events. */
  grid: TableGrid;
};

/** Resolve the URL of the grid-core chunk.
 *
 *  Default: relative to this module — `import.meta.url` is the URL
 *  `tonk-table.js` was loaded from, so the core chunk lives next to it
 *  regardless of where the consumer mounted the bundle (root,
 *  /assets/, CDN, …).
 *
 *  Override: `window.__tonkTableGrid`. Sealed environments that import
 *  this shell from a minted `blob:` URL (tonk-portal guests) have no
 *  working relative resolution — the injector provides this global
 *  instead. Either a URL string (core already minted), or a function
 *  returning the URL / a promise of it — the lazy form: the guest asks
 *  its trusted parent for the core's bytes only when the first element
 *  actually connects, keeping the boot payload at the shell alone. */
async function gridChunkUrl(): Promise<string> {
  const override = (globalThis as { __tonkTableGrid?: unknown })
    .__tonkTableGrid;
  if (typeof override === "string" && override) return override;
  if (typeof override === "function") {
    const url = await (override as () => string | Promise<string>)();
    if (typeof url === "string" && url) return url;
  }
  return new URL("./tonk-table-grid.js", import.meta.url).href;
}

/** Single in-flight / resolved load of the grid core, shared by every
 *  element instance. Cleared on failure so a later connect can retry
 *  after e.g. a network blip. */
let gridModule: Promise<GridModule> | null = null;

function loadGridModule(): Promise<GridModule> {
  if (!gridModule) {
    gridModule = gridChunkUrl().then(
      (url) => import(/* @vite-ignore */ url).then((mod) => mod as GridModule),
    );
    gridModule.catch(() => {
      gridModule = null;
    });
  }
  return gridModule;
}

/** Decode a parsed content body into the grid's source form: workbook
 *  bytes when the envelope says so, CSV otherwise. A corrupt base64
 *  body degrades to an empty workbook rather than throwing — a bad
 *  store write must render an empty grid, not crash the element. */
function toSource(content: Content): TableSource {
  if (isWorkbookType(content.contentType)) {
    const bytes = base64ToBytes(content.value);
    if (bytes && bytes.length > 0) return { kind: "workbook", bytes };
    console.warn("[tonk-table] workbook body was not valid base64; starting empty");
    return { kind: "csv", csv: "" };
  }
  return { kind: "csv", csv: content.value };
}

class TonkTableElement extends HTMLElement {
  static get observedAttributes(): readonly string[] {
    return OBSERVED;
  }

  /** Shadow root the grid mounts into. Owning a shadow root keeps the
   *  grid styled and its focus behavior intact no matter which outer
   *  shadow tree (e.g. `<wa-page>`) the host lands in. */
  readonly #shadow: ShadowRoot;

  /** Mount point handed to the grid core. */
  readonly #mount: HTMLDivElement;

  /** Live grid handle once the core chunk resolved and the engine
   *  mounted. Null while loading and after teardown. */
  #grid: TableGrid | null = null;

  /** Monotonic mount token. Bumped on every connect *and* teardown; an
   *  async mount whose token no longer matches destroys its grid
   *  instead of wiring it up (covers connect → disconnect → reconnect
   *  races while the chunk or the engine wasm is in flight). */
  #mountToken = 0;

  /** Content writes that land while the core is still loading. Applied
   *  (last-write-wins) when the grid mounts. */
  #pendingRaw: string | null = null;

  /** Observes the light DOM — the text content channel (standalone) or
   *  the hidden claim data-rows (claims mode). One observer serves
   *  both; the callback branches on the current mode. */
  #textObserver: MutationObserver | null = null;

  /** Debounce timer for claims-mode row re-reads. */
  #rowsTimer: ReturnType<typeof setTimeout> | null = null;

  /** Debounce timer for the outward `change` event. Coalesces a burst
   *  of edits into one dispatch after an idle gap, so a consumer that
   *  commits each `change` to a store isn't hit per-commit. */
  #changeTimer: ReturnType<typeof setTimeout> | null = null;

  /** Whether user edits happened since the last `change` dispatch. The
   *  grid signals dirtiness; the flush PULLS the serialization, so an
   *  edit burst costs one workbook serialization, not one per edit. */
  #dirty = false;

  /** This element's Hybrid Logical Clock. Every outbound `change`
   *  stamps a fresh HLC; every adopted inbound `content` advances it.
   *  Monotonic, so it orders our own writes even across a wonky system
   *  clock. */
  readonly #clock = new Clock();

  /** The highest HLC this element has issued or adopted. An incoming
   *  `content` is our own echo (or otherwise not newer) exactly when
   *  its HLC is `<= this`; such writes are ignored, which is what stops
   *  a store round-trip from reloading the grid under the user. A
   *  genuinely newer write (greater HLC, or a bare value with no HLC)
   *  is adopted. */
  #lastKnownHlc = 0n;

  /** Pending teardown scheduled by `disconnectedCallback`. Reactive
   *  frameworks (Leptos, mostly) detach and re-attach DOM nodes during
   *  rerenders; deferring the teardown one macrotask lets a re-attach
   *  cancel it, preserving grid state (undo history, selection) across
   *  the move. */
  #teardownScheduled = false;

  /** Mirror of the element's light-DOM `<style>` children, adopted
   *  into the shadow root LAST so user rules win the cascade (at equal
   *  specificity) over both the shell's and the grid's own sheets.
   *  This is restyling layer 3 — arbitrary CSS against the internal
   *  classes when `::part()` can't express it. */
  readonly #userStyles: HTMLStyleElement;

  constructor() {
    super();
    this.#shadow = this.attachShadow({ mode: "open", delegatesFocus: true });

    const style = document.createElement("style");
    style.textContent = SHADOW_STYLESHEET;

    this.#mount = document.createElement("div");
    this.#mount.className = "mount";

    this.#userStyles = document.createElement("style");

    this.#shadow.append(style, this.#mount, this.#userStyles);
  }

  /** Adopt the element's direct `<style>` children into the shadow
   *  root. Live: the light-DOM observer re-runs this on any mutation,
   *  so a view (or host code) can add/edit a stylesheet at any time. */
  #syncUserStyles(): void {
    let css = "";
    for (const child of Array.from(this.children)) {
      if (child instanceof HTMLStyleElement) {
        css += `${child.textContent ?? ""}\n`;
      }
    }
    if (this.#userStyles.textContent !== css) {
      this.#userStyles.textContent = css;
    }
  }

  connectedCallback(): void {
    // If we were about to tear down (framework move), cancel — the
    // re-attach happened first and the live grid carries on.
    this.#teardownScheduled = false;

    // Watch the light DOM: the text child in standalone mode, the
    // hidden claim data-rows in claims mode. Installed once; survives
    // the grid's own lifecycle since the grid lives in the shadow root
    // and never touches light DOM.
    if (!this.#textObserver) {
      this.#textObserver = new MutationObserver(() => {
        this.#syncUserStyles();
        if (this.#claimsMode()) this.#scheduleRows();
        else this.#onLightContent();
      });
      this.#textObserver.observe(this, {
        childList: true,
        characterData: true,
        subtree: true,
        attributes: true,
        attributeFilter: ROW_ATTRS,
      });
    }
    this.#syncUserStyles();

    if (this.#grid) return;

    const token = ++this.#mountToken;
    void this.#mountGrid(token);
  }

  /** Whether this element renders a store subject (claims mode). */
  #claimsMode(): boolean {
    return this.hasAttribute("subject");
  }

  /** Debounced claims-mode row re-read: parse the light-DOM data rows
   *  and hand them to the grid to reconcile. */
  #scheduleRows(): void {
    if (this.#rowsTimer !== null) return;
    this.#rowsTimer = setTimeout(() => {
      this.#rowsTimer = null;
      this.#applyRowsNow();
    }, ROWS_DEBOUNCE_MS);
  }

  #applyRowsNow(): void {
    const grid = this.#grid;
    const subject = this.getAttribute("subject");
    if (!grid || subject === null) return;
    const sheets = readSheetRows(this, subject);
    const sheetIds = new Set(sheets.map((s) => s.id));
    grid.applyRows(
      sheets,
      readCellRows(this, sheetIds),
      readColumnRows(this, sheetIds),
      readRowSizeRows(this, sheetIds),
    );
  }

  /** Parse a positive-integer attribute, `undefined` when absent or
   *  malformed (the grid then keeps its default). */
  #intAttr(name: string): number | undefined {
    const raw = this.getAttribute(name);
    if (raw === null) return undefined;
    const n = Number(raw);
    return Number.isInteger(n) && n >= 1 ? n : undefined;
  }

  /** The element's light-DOM DIRECT text nodes — its content channel.
   *  Deliberately not `textContent`: element children (a user
   *  `<style>` sheet, claims data rows) must never leak into the
   *  workbook content. A store writes via `set_text_content`, which
   *  replaces the children with one text node, so the channel is
   *  unaffected. */
  #lightContent(): string {
    let text = "";
    for (const node of Array.from(this.childNodes)) {
      if (node.nodeType === Node.TEXT_NODE) text += node.nodeValue ?? "";
    }
    return text;
  }

  /** The last light content adopted, so mutations that don't change
   *  the channel (style-sheet edits, stray attribute flips) don't
   *  re-adopt — a bare (unversioned) re-adopt would reload the grid. */
  #lastLightContent: string | null = null;

  /** A light-DOM text change (the store wrote new content). Adopt it —
   *  the HLC gate drops our own echo, a newer write reloads the grid.
   *  Standalone mode only; claims mode reads rows, not text. */
  #onLightContent(): void {
    if (this.#claimsMode()) return;
    const text = this.#lightContent();
    if (text === this.#lastLightContent) return;
    this.#lastLightContent = text;
    this.#adopt(text);
  }

  async #mountGrid(token: number): Promise<void> {
    let mod: GridModule;
    try {
      mod = await loadGridModule();
    } catch (err) {
      // A missing/broken chunk shouldn't crash the host page. The
      // element renders empty; a later reconnect retries the load.
      console.warn("[tonk-table] failed to load grid core:", err);
      return;
    }
    // Stale mount: the element disconnected (or reconnected, which
    // bumps the token and starts its own mount) while the chunk was in
    // flight.
    if (token !== this.#mountToken || !this.isConnected) return;

    let mode: GridMode;
    const subject = this.getAttribute("subject");
    if (subject !== null) {
      // Claims mode: the store is the document. Typed mutation events
      // re-dispatch from the host element, where a tonk view binds each
      // to a command and any other host just listens.
      mode = { kind: "claims", subject };
    } else {
      // Standalone: initial document, in priority order:
      //   1. a `.value`/`.content` property write buffered before mount,
      //   2. the light-DOM text child (the primary content channel),
      //   3. the `content`/`value` attribute (legacy / convenience).
      // Each is parsed through the envelope so an initial versioned
      // content seeds `#lastKnownHlc` — a later echo of that same
      // version is then correctly recognized and dropped.
      let raw: string | null = this.#pendingRaw;
      if (raw === null) {
        const light = this.#lightContent();
        raw =
          light !== ""
            ? light
            : (this.getAttribute("content") ?? this.getAttribute("value"));
      }
      this.#pendingRaw = null;
      const parsed = parseContent(raw ?? "");
      if (parsed.hlc !== null && parsed.hlc > this.#lastKnownHlc) {
        this.#lastKnownHlc = this.#clock.receive(parsed.hlc);
      }
      mode = { kind: "standalone", source: toSource(parsed) };
    }

    let grid: TableGrid;
    try {
      grid = await mod.createGrid(this.#mount, {
        mode,
        // Every typed event — claims mutations and observability alike
        // — re-dispatches from the host element, where a tonk view
        // binds types to commands and any other host just listens.
        emit: (type, detail) => {
          this.dispatchEvent(
            new CustomEvent(type, { bubbles: true, composed: true, detail }),
          );
        },
        readOnly: this.hasAttribute("readonly"),
        minRows: this.#intAttr("min-rows"),
        minCols: this.#intAttr("min-cols"),
        // The dirty signal drives the envelope round-trip, which only
        // exists in standalone mode; claims mode emits typed events.
        onChange:
          mode.kind === "standalone" ? () => this.#scheduleChange() : undefined,
      });
    } catch (err) {
      // Engine wasm failed to instantiate (or the bytes leaf was
      // missing). Same containment as a failed chunk load.
      console.warn("[tonk-table] failed to mount grid:", err);
      return;
    }
    if (token !== this.#mountToken || !this.isConnected) {
      // Went stale during the engine await: this grid was never
      // exposed, so tear it down before anyone can see it.
      grid.destroy();
      return;
    }
    this.#grid = grid;

    if (mode.kind === "claims") {
      // Feed the grid whatever claim rows the view has already
      // rendered; later row mutations arrive via the observer.
      this.#applyRowsNow();
    } else if (this.#pendingRaw !== null) {
      // A write may have arrived while the engine was instantiating; it
      // was buffered (and HLC-gated) by #adopt, so apply it verbatim.
      const pending = parseContent(this.#pendingRaw);
      this.#pendingRaw = null;
      grid.load(toSource(pending));
    }

    this.dispatchEvent(
      new CustomEvent<ReadyDetail>("ready", {
        detail: { grid },
        bubbles: true,
        composed: true,
      }),
    );

    // Deferred to the next macrotask so sibling mounts settling in the
    // same tick can't clobber the focus (mirrors `<tonk-prose>`).
    if (!this.hasAttribute("readonly") && this.hasAttribute("auto-focus")) {
      setTimeout(() => {
        if (this.#grid === grid) grid.focus();
      }, 0);
    }
  }

  /** Record dirtiness and (re)arm the debounce. The `change` event
   *  dispatches once the edits go idle. */
  #scheduleChange(): void {
    this.#dirty = true;
    if (this.#changeTimer !== null) clearTimeout(this.#changeTimer);
    this.#changeTimer = setTimeout(
      () => this.#flushChange(),
      CHANGE_DEBOUNCE_MS,
    );
  }

  /** Dispatch the pending debounced `change`, if any. Stamps a fresh
   *  HLC and advances `#lastKnownHlc`, so when the consumer round-trips
   *  the emitted `content` back through a store the element recognizes
   *  it (HLC not newer) and drops it instead of re-applying. */
  #flushChange(): void {
    this.#changeTimer = null;
    if (!this.#dirty) return;
    this.#dirty = false;
    const grid = this.#grid;
    if (!grid) return;
    const hlc = this.#clock.tick();
    this.#lastKnownHlc = hlc;
    const value = grid.toCsv();
    const content = formatContent({
      hlc,
      contentType: WORKBOOK_TYPE,
      value: bytesToBase64(grid.serialize()),
    });
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
      // `isConnected` flips back to true if the framework re-inserted
      // us within the deferral window.
      if (this.isConnected) return;
      // Flush any debounced edit before tearing down, so the last
      // entries before removal aren't lost.
      if (this.#changeTimer !== null) {
        clearTimeout(this.#changeTimer);
        this.#flushChange();
      }
      if (this.#rowsTimer !== null) {
        clearTimeout(this.#rowsTimer);
        this.#rowsTimer = null;
      }
      this.#textObserver?.disconnect();
      this.#textObserver = null;
      this.#mountToken++;
      this.#grid?.destroy();
      this.#grid = null;
    }, 0);
  }

  attributeChangedCallback(
    name: ObservedAttr,
    _old: string | null,
    next: string | null,
  ): void {
    switch (name) {
      case "subject":
        // The document identity changed modes or targets: remount.
        // (Rare — a view rebinding the element wholesale.)
        if (_old !== next && this.#grid) {
          this.#grid.destroy();
          this.#grid = null;
          const token = ++this.#mountToken;
          if (this.isConnected) void this.#mountGrid(token);
        }
        break;
      case "content":
        this.#adopt(next ?? "");
        break;
      case "value":
        // Reflect attribute → property only when they diverge so we
        // don't fight the user's edits. `value=` is the bare CSV
        // channel; it flows through the same adoption.
        if ((next ?? "") !== this.value) {
          this.#adopt(next ?? "");
        }
        break;
      case "readonly":
        this.#grid?.setReadOnly(next !== null);
        break;
      case "min-rows":
      case "min-cols":
        this.#grid?.setMinExtent(
          this.#intAttr("min-rows"),
          this.#intAttr("min-cols"),
        );
        break;
      case "auto-focus":
        // Applied at mount only — stealing focus on a post-mount
        // attribute write would surprise the user.
        break;
    }
  }

  /** Adopt an incoming write — either a versioned `content` envelope
   *  (base64 workbook bytes) or a bare CSV string (which parses as an
   *  HLC-less value and is always adopted). The HLC gate is the whole
   *  point: a write whose HLC is not newer than what we've issued/seen
   *  is our own echo (or otherwise stale) and is ignored, so a store
   *  round-trip can't reload the grid under the user; a genuinely
   *  newer write advances our clock and replaces the workbook
   *  (selection preserved best-effort). */
  #adopt(raw: string): void {
    // Claims mode has no content channel — the store's rows are the
    // document; a stray content/value write must not clobber them.
    if (this.#claimsMode()) return;
    const parsed = parseContent(raw);
    if (parsed.hlc !== null) {
      if (parsed.hlc <= this.#lastKnownHlc) return; // our echo, or not newer
      this.#lastKnownHlc = this.#clock.receive(parsed.hlc);
    }
    if (!this.#grid) {
      // Grid core still loading (or element not yet connected): buffer
      // the raw write, the mount applies it.
      this.#pendingRaw = raw;
      return;
    }
    this.#grid.load(toSource(parsed));
  }

  /** CSV of the active sheet (the human-readable, lossy view). Before
   *  the grid mounts this reflects the pending write's body verbatim. */
  get value(): string {
    if (this.#grid) return this.#grid.toCsv();
    const raw =
      this.#pendingRaw ??
      (this.#lightContent() !== ""
        ? this.#lightContent()
        : (this.getAttribute("content") ?? this.getAttribute("value")));
    return raw === null ? "" : parseContent(raw).value;
  }

  set value(next: string) {
    this.#adopt(next);
  }

  /** The versioned `content` envelope: base64 of the engine's binary
   *  workbook serialization plus the last HLC this element issued or
   *  adopted. Writing this back through a store round-trips the
   *  version, so the element drops its own echo. Before the grid
   *  mounts this reflects the pending write verbatim. */
  get content(): string {
    if (this.#grid) {
      return formatContent({
        hlc: this.#lastKnownHlc,
        contentType: WORKBOOK_TYPE,
        value: bytesToBase64(this.#grid.serialize()),
      });
    }
    const raw =
      this.#pendingRaw ??
      (this.#lightContent() !== ""
        ? this.#lightContent()
        : (this.getAttribute("content") ?? this.getAttribute("value")));
    return raw ?? "";
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

  /** Move keyboard focus into the grid. */
  focus(): void {
    if (this.#grid) {
      this.#grid.focus();
    } else {
      super.focus();
    }
  }

  /** The live grid handle, if mounted. Power-user surface. */
  get grid(): TableGrid | null {
    return this.#grid;
  }
}

/** Host stylesheet. Each `--tonk-table-*` variable is the theming
 *  contract, and its DEFAULT inherits the page's design tokens where
 *  the consumer exposes them — WebAwesome `--wa-*` tokens (the tonk
 *  app's palette) — falling back to a self-contained GitHub-flavored
 *  value so the element still looks right mounted on a bare page. A
 *  consumer can still override any `--tonk-table-*` directly. Because
 *  custom properties inherit through the shadow boundary, surfaces,
 *  headers, and the selection read the same as the surrounding page
 *  when those `--wa-*` tokens are present. The grid core adds the
 *  rules for its own DOM (headers, cells, tabs) when it mounts. */
const SHADOW_STYLESHEET = `
  :host {
    --tonk-table-font: var(--wa-font-family-body, ui-sans-serif, -apple-system,
                       "Segoe UI", Helvetica, Arial, sans-serif);
    --tonk-table-mono: var(--wa-font-family-code, ui-monospace, SFMono-Regular,
                       Menlo, Consolas, "Liberation Mono", monospace);
    --tonk-table-font-size: var(--wa-font-size-s, 0.875rem);
    --tonk-table-radius: var(--wa-border-radius-m, 6px);

    /* Surfaces & text — inherit the page's WebAwesome tokens, GitHub
       light values as the standalone fallback. */
    --tonk-table-bg: var(--wa-color-surface-default, #ffffff);
    --tonk-table-fg: var(--wa-color-text-normal, #1f2328);
    --tonk-table-fg-muted: var(--wa-color-text-quiet, #59636e);
    --tonk-table-border: var(--wa-color-neutral-border-quiet, #d1d9e0);
    --tonk-table-grid-line: var(--wa-color-neutral-border-quiet, #e5e9ed);
    --tonk-table-header-bg: var(--wa-color-neutral-fill-quiet, #f6f8fa);
    --tonk-table-header-fg: var(--wa-color-text-quiet, #59636e);
    /* Active cell + focus → the brand accent; range fill → its quiet
       counterpart. */
    --tonk-table-accent: var(--wa-color-brand-fill-loud, #0969da);
    --tonk-table-selection: var(--wa-color-brand-fill-quiet, #0969da1a);
    --tonk-table-focus-ring: var(--wa-color-brand-border-normal, #0969da66);
    --tonk-table-error: var(--wa-color-danger-fill-loud, #d1242f);

    display: flex;
    flex-direction: column;
    block-size: var(--tonk-table-height, 26rem);
    position: relative;
    box-sizing: border-box;
    background: var(--tonk-table-bg);
    color: var(--tonk-table-fg);
    border: 1px solid var(--tonk-table-border);
    border-radius: var(--tonk-table-radius);
    overflow: hidden;
    transition: border-color 120ms ease, box-shadow 120ms ease;
  }

  /* Standalone dark fallback (no WebAwesome tokens present). When the
     page provides \`--wa-*\` the rules above already track its
     light/dark palette, so this only bites a bare page in dark mode. */
  @media (prefers-color-scheme: dark) {
    :host {
      --tonk-table-bg: var(--wa-color-surface-default, #0d1117);
      --tonk-table-fg: var(--wa-color-text-normal, #f0f6fc);
      --tonk-table-fg-muted: var(--wa-color-text-quiet, #9198a1);
      --tonk-table-border: var(--wa-color-neutral-border-quiet, #3d444d);
      --tonk-table-grid-line: var(--wa-color-neutral-border-quiet, #2a3038);
      --tonk-table-header-bg: var(--wa-color-neutral-fill-quiet, #151b23);
      --tonk-table-header-fg: var(--wa-color-text-quiet, #9198a1);
      --tonk-table-accent: var(--wa-color-brand-fill-loud, #1f6feb);
      --tonk-table-selection: var(--wa-color-brand-fill-quiet, #1f6feb33);
      --tonk-table-focus-ring: var(--wa-color-brand-border-normal, #1f6feb99);
      --tonk-table-error: var(--wa-color-danger-fill-loud, #f85149);
    }
  }

  :host([hidden]) { display: none; }

  :host(:focus-within) {
    border-color: var(--tonk-table-accent);
    box-shadow: 0 0 0 2px var(--tonk-table-focus-ring);
  }

  .mount {
    flex: 1;
    min-block-size: 0;
    display: flex;
    flex-direction: column;
  }
  .mount > .table-root { flex: 1; }
`;

if (!customElements.get("tonk-table")) {
  customElements.define("tonk-table", TonkTableElement);
}

export type { TonkTableElement };
