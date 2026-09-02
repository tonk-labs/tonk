// Dev-only hot reload control.
//
// Trunk's autoreload (disabled via `no_autoreload` in Trunk.toml) is
// replaced by this client so a change to a *served asset* — notably
// the standard library `/library/core.yaml` — re-seeds the live
// repository in place instead of reloading the whole page. A genuine
// code change (the wasm hash moved) still reloads.
//
// It connects to trunk's own change channel (`.well-known/trunk/ws`),
// which exists only under `trunk serve`. In a production build there
// is no such socket, so this module connects, fails, and stays inert
// — safe to ship unconditionally.
//
// The `<hot-swap>` element renders a small corner pill: click it to
// toggle hot reload on/off (the checkbox is an invisible overlay over
// the whole pill), the `version` label shows the live bootstrap hash after
// a reseed, and the status dot spins while seeding. Visual style is
// ported from the interactivate-dat live-reload pill.

;(async () => {
  const LIBRARY_URL = "/library/core.yaml"
  // The profile branch's own library — the Hub view and the FAB chrome. It is
  // seeded onto the profile's meta branch, NOT the space content branch, so a
  // core.yaml reseed never reaches it. Without reseeding this too, every
  // profile.yaml edit needs the profile recreated to be seen.
  const PROFILE_LIBRARY_URL = "/library/profile.yaml"
  // The notebook library. It rides on the CONTENT branch, same as core, but
  // is served as its own document rather than being folded into core.yaml.
  // Without it here a notebook.yaml edit needs the space recreated to be
  // seen — the symptom being a notebook page that renders the version
  // installed when the space was made, however many times the dev server
  // rebuilds.
  const NOTEBOOK_LIBRARY_URL = "/library/notebook.yaml"

  // Pill label glyphs: a recycle mark for an in-place standard-library
  // reseed (live update, no reload), an eject mark for a full page
  // reload (the running code is replaced).
  // Idle is blank — the plain handle circle is enough; a glyph only
  // appears for an actual event.
  const GLYPH_IDLE = ""
  const GLYPH_LIBRARY = "♺"
  const GLYPH_RELOAD = "⏏"
  const GLYPH_ERROR = "⚠"

  class HotSwap extends HTMLElement {
    // Persists the auto-apply toggle across reloads, and doubles as the
    // cross-tab channel: the `storage` event fires in other tabs when
    // this key changes, so a flip propagates without any extra plumbing.
    static STORAGE_KEY = "tonk:hot-swap:enabled"
    static insert(target) {
      const doc = target.ownerDocument
      const view = doc.createElement("hot-swap")
      target.appendChild(view)
      return view
    }
    constructor(...args) {
      super(...args)
      this.root = this.attachShadow({ mode: "open" })
    }
    connectedCallback() {
      this.root.innerHTML = `
      <style>
      :host {
        /* Use the app's Web Awesome design tokens so the pill matches
           the active theme (including the brutalist palette overrides)
           and follows light/dark automatically. WA custom properties
           are defined on the document root and inherit through the
           shadow boundary; the fallbacks keep the pill legible if WA
           isn't loaded (e.g. a bare page). */
        --hs-bg: var(--wa-color-surface-raised, #261f20);
        --hs-fg: var(--wa-color-text-normal, #e2dfdd);
        /* Monochrome: ON inverts to a solid LIGHT pill, OFF is the muted
           dark pill — distinguished by inversion + opacity, no colour.
           The error state is solid ink (one-ink system): the pill's
           wording and inversion carry the alarm, never a hue. */
        --hs-dot-idle: var(--wa-color-neutral-border-normal, rgb(226 223 221 / 28%));
        --hs-danger: var(--wa-color-danger-fill-loud, #38182a);
        --hs-danger-fg: var(--wa-color-danger-on-loud, #f7f6f5);
        --hs-radius: var(--wa-border-radius-pill, 3em);
        --hs-font: var(--wa-font-family-body, -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif);
        /* A soft glow shadow like the original pill — readable on any
           background. WA's shadow tokens compose into this if present. */
        --hs-shadow: 0 0 0 1px var(--wa-color-surface-border, rgb(226 223 221 / 28%));
      }

      .notification {
        line-height: 1.15;
        -webkit-text-size-adjust: 100%;

        pointer-events: none;
        position: fixed;
        display: inline-block;
        z-index: 2147483647;
        /* Bottom-right corner: the top-right is the workspace top bar's
           real controls (share + sync toggle), and this dev pill's
           invisible hover-zone (.hide::after below) would otherwise
           intercept clicks on them. */
        bottom: 0px;
        right: 0px;
        border: none;
        margin: 10px;
        padding: 0;

        font-family: var(--hs-font);
        max-width: 400px;
      }

      /* Hide the native checkbox; clicking the pill toggles it (the
         pill is a <label>). */
      .notification input[type=checkbox] {
        position: absolute;
        opacity: 0;
        width: 0;
        height: 0;
        margin: 0;
        pointer-events: none;
      }

      /* The pill — a DRAWER that opens leftward from the handle. The
         handle (dot) is pinned at the right; the body grows to the left
         (a width transition) to reveal the version, which then slides +
         fades in from the left AFTER the width has opened. Modelled on
         the new-notification drawer animation. The whole pill also
         scales away to hide / scales back to conjure. */
      .pill {
        position: relative;
        display: flex;
        flex-direction: row;          /* version left, handle right */
        align-items: center;
        justify-content: flex-end;    /* keep handle pinned right as width grows */
        box-sizing: border-box;
        height: 1.9em;
        width: 7em;                   /* unfolded by default; .folded shrinks it */
        padding: 0 0.4em;
        border-radius: var(--hs-radius);
        /* Inverse fill (light pill, dark handle/text) so the page shows
           through when translucent — a dark pill on the dark page hides
           the opacity change. */
        background: var(--hs-fg);
        color: var(--hs-bg);
        box-shadow: var(--hs-shadow);
        font-size: 0.7rem;
        line-height: 1;
        white-space: nowrap;
        overflow: hidden;
        /* OFF: muted/translucent. ON (:checked) goes solid. The opacity
           gap is the only on/off signal — same colour either way. */
        opacity: 0.4;
        cursor: pointer;
        pointer-events: all;
        transform: scale(1);
        transform-origin: center;
        transition: width 0.45s cubic-bezier(.86,0,.07,1),
                    transform 0.3s cubic-bezier(.68,-0.55,.27,1.55),
                    background-color 0.2s ease,
                    opacity 0.2s ease;
      }
      /* Hover nudges OFF up a little for affordance, but stays below the
         solid ON level so the states never look identical. */
      .notification input:not(:checked) + .pill:hover { opacity: 0.7; }

      /* Enabled (auto-apply on): the same pill, fully solid (opacity 1).
         OFF is the muted version of the very same pill — opacity is the
         only difference. */
      .notification input:checked + .pill {
        opacity: 1;
      }

      /* The version sits beside the handle, clear of it. Off (handle
         right) → label shifted left; the margin flips with the toggle
         so the handle never overlays the text. It reveals (fades +
         slides in) only after the drawer has opened (transition-delay). */
      .version {
        font-variant-numeric: tabular-nums;
        margin: 0 1.9em 0 0.6em;   /* handle right: room on the right */
        opacity: 1;
        transform: translateX(0);
        transition: opacity 0.25s ease 0.25s,
                    transform 0.3s ease 0.25s,
                    margin 0.45s cubic-bezier(.68,-0.55,.27,1.55);
      }

      /* The handle — an absolutely-positioned circle that SLIDES across
         the pill on toggle (wa-switch thumb), pinned to the right end
         when off and the left end when on. The glyph sits inside it.
         It's the click target and the status indicator (spins as a ring
         while applying / on error). */
      .dot {
        position: absolute;
        top: 50%;
        right: 0.25em;
        transform: translateY(-50%);
        box-sizing: border-box;
        width: 1.4em;
        height: 1.4em;
        border-radius: 50%;
        /* Dark handle on the light pill (the inverse), so it stays
           visible whether the pill is solid or translucent. */
        background: var(--hs-bg);
        color: var(--hs-fg);
        display: inline-flex;
        align-items: center;
        justify-content: center;
        transition: right 0.45s cubic-bezier(.68,-0.55,.27,1.55),
                    background-color 0.2s ease;
      }
      .glyph { font-size: 0.7em; line-height: 1; }

      /* Enabled: handle slides to the LEFT end (same colours); the
         version margin flips so it sits to the handle's right. */
      .notification input:checked + .pill .dot {
        right: calc(100% - 1.4em - 0.25em);
      }
      .notification input:checked + .pill .version {
        margin: 0 0.6em 0 1.9em;   /* handle left: room on the left */
      }

      /* Pulse while announcing / applying a change. */
      .notification.update .pill { animation: upgrade 1.2s infinite ease-in-out; }
      /* While applying (toggle on + pulsing), the handle spins as a ring.
         The ON pill is light, so the ring is dark to read against it. */
      .notification.update input:checked + .pill .dot {
        background: transparent;
        border: 2px solid transparent;
        border-left-color: var(--hs-bg);
        animation: spin 0.8s infinite linear;
      }

      /* Error: danger fill, forced visible, and it keeps pulsing (the
         update class stays on) to announce the trouble. The handle is
         hidden — a spinning ring reads as "in progress" and overlaps
         the message, and a solid dot adds nothing, so the error pill is
         just the pulsing danger drawer with its text. The dot selectors
         below are specific enough to win over the update /
         input-checked spinner. */
      .notification.error .pill { background: var(--hs-danger); color: var(--hs-danger-fg); opacity: 1; }
      .notification.error .dot,
      .notification.update.error input:checked + .pill .dot {
        background: transparent;
        border: none;
        animation: none;
        opacity: 0;
      }

      /* FOLDED: retract the drawer — width shrinks back to the handle
         circle and the version slides/fades back out to the left. */
      .notification.folded .pill { width: 1.9em; }
      .notification.folded .version {
        opacity: 0;
        transform: translateX(-0.6em);
        /* No delay folding — the text leaves first, then the drawer
           closes. */
        transition: opacity 0.15s ease, transform 0.2s ease;
      }

      /* HIDDEN at rest: the whole pill scales to nothing (the conjure).
         A hover-zone behind it (z-index 0) holds the hover in the corner
         so the pill (z-index 1) scales back and stays clickable. The zone
         hugs the TRUE corner (negative offsets cancel the .notification
         margin) and stays within the 16px gutter the FAB never enters —
         any bigger and it blankets the FAB's bottom-right dock, eating
         its drag pointerdowns. */
      .notification.hide .pill { transform: scale(0); position: relative; z-index: 1; }
      .notification.hide::after {
        content: "";
        position: absolute;
        bottom: -10px; right: -10px;
        width: 16px; height: 16px;
        pointer-events: all;
        z-index: 0;
      }
      .notification.hide:hover .pill { transform: scale(1); }

      @keyframes spin {
        0% { transform: translateY(-50%) rotate(0deg); }
        100% { transform: translateY(-50%) rotate(360deg); }
      }
      @keyframes upgrade {
        0% { transform: scale(1); }
        50% { transform: scale(1.05); }
        100% { transform: scale(1); }
      }
      </style>
      <aside class="notification hide">
        <input id="hotswap" type="checkbox" checked />
        <label class="pill" for="hotswap">
          <span class="version"></span>
          <span class="dot"><span class="glyph"></span></span>
        </label>
      </aside>`
      this.toggle = this.root.querySelector("input")
      // Restore the persisted preference so toggling off survives a
      // reload (otherwise every refresh re-enables auto-apply). Stored
      // value "0" means the user turned it off.
      try {
        this.toggle.checked = localStorage.getItem(HotSwap.STORAGE_KEY) !== "0"
      } catch (_) {
        // localStorage unavailable (private mode, etc.) — default on.
      }
      this.toggle.addEventListener("change", () => {
        try {
          localStorage.setItem(HotSwap.STORAGE_KEY, this.toggle.checked ? "1" : "0")
        } catch (_) {
          // Non-fatal: preference just won't persist this session.
        }
        // Re-enabling auto-apply while a change is pending applies it.
        if (this.toggle.checked && this.onenable) this.onenable()
      })
      // Mirror the toggle across tabs. The `storage` event fires in
      // *other* tabs (never the one that wrote) when a shared key
      // changes, so writing the preference above already announces the
      // flip — no separate channel to keep in step with the persisted
      // value. Receiving tabs sync their checkbox and run the same
      // enable side-effect, so an off→on elsewhere applies a held
      // change here too. Without this, each tab carried its own stale
      // state and every enabled tab independently reseeded the same
      // branch.
      this._onStorage = (event) => {
        if (event.key !== HotSwap.STORAGE_KEY) return
        // Read the committed value back from localStorage rather than
        // trusting `event.newValue` — same source the initial state is
        // read from, so the two paths can't drift, and it's robust to
        // null/lagging event payloads.
        let checked = true
        try {
          checked = localStorage.getItem(HotSwap.STORAGE_KEY) !== "0"
        } catch (_) {
          // localStorage unavailable — fall back to the event payload.
          checked = event.newValue !== "0"
        }
        if (this.toggle.checked === checked) return
        this.toggle.checked = checked
        if (checked && this.onenable) this.onenable()
      }
      window.addEventListener("storage", this._onStorage)
    }
    disconnectedCallback() {
      if (this._onStorage) {
        window.removeEventListener("storage", this._onStorage)
        this._onStorage = null
      }
    }
    // Whether auto-apply is on. Reads localStorage — the shared source
    // of truth across tabs — rather than this tab's checkbox, which can
    // be stale if another tab toggled while a `storage` event was still
    // in flight. Falls back to the checkbox when storage is unavailable.
    get enabled() {
      try {
        const stored = localStorage.getItem(HotSwap.STORAGE_KEY)
        if (stored !== null) return stored !== "0"
      } catch (_) {
        // localStorage unavailable — fall through to the checkbox.
      }
      return this.toggle?.checked ?? true
    }
    set visible(value) {
      this.root.querySelector(".notification").classList.toggle("hide", !value)
    }
    // Folded = collapsed to a circle (icon only, no hash). Unfolded
    // (default) shows the hash.
    set folded(value) {
      this.root.querySelector(".notification").classList.toggle("folded", value)
    }
    // Pulse = the pill pulses (announcing / applying a change).
    set pulse(value) {
      this.root.querySelector(".notification").classList.toggle("update", value)
    }
    // Error = a build failed. Forces the pill visible (even though it's
    // hidden at rest), tints it danger, and keeps it pulsing until the
    // next successful build clears it.
    set error(value) {
      const aside = this.root.querySelector(".notification")
      aside.classList.toggle("error", value)
      if (value) {
        aside.classList.remove("hide")
        aside.classList.add("update")
      } else {
        // Cleared by a good build: stop pulsing and settle back to
        // hidden (the onChange that follows drives any further UI).
        aside.classList.remove("update")
        aside.classList.add("hide")
      }
    }
    /// Set the handle glyph (lives inside the sliding circle) and the
    /// hash/label text (centred, the handle slides over it).
    setStatus(glyph, label) {
      this.root.querySelector(".glyph").textContent = glyph
      this.root.querySelector(".version").textContent = label
    }
  }
  customElements.define("hot-swap", HotSwap)

  // The wasm bundle hash trunk emits into the served index, used to
  // detect a genuine code change. A different hash on a fresh change
  // signal is a real rebuild (reload); an unchanged hash means only
  // an asset moved (trunk fires a generic `reload` for any pipeline
  // run, so we can't trust the signal alone).
  // The served library text. BOTH documents are fetched and joined, because
  // this string is what the change signal hashes: keying only on core.yaml
  // would leave a profile.yaml-only edit undetected, so the FAB chrome would
  // never reseed. `reseed` splits it back apart and sends each half to the
  // branch it belongs on.
  const LIBRARY_SEPARATOR = "\n#--- hot-swap: profile library ---\n"
  const fetchLibrary = async () => {
    const [core, profile, notebook] = await Promise.all(
      [LIBRARY_URL, PROFILE_LIBRARY_URL, NOTEBOOK_LIBRARY_URL].map(
        async (url) => {
          const response = await fetch(url, { cache: "no-store" })
          if (!response.ok) throw new Error(`GET ${url} -> ${response.status}`)
          return await response.text()
        },
      ),
    )
    // Core and notebook both land on the content branch, so they are joined
    // into one document and seeded together.
    return core + "\n" + notebook + LIBRARY_SEPARATOR + profile
  }
  // Split a joined library back into `{ core, profile }`. Tolerates an older
  // cached string with no separator (all of it is core).
  const splitLibrary = (library) => {
    const at = library.indexOf(LIBRARY_SEPARATOR)
    return at === -1
      ? { core: library, profile: null }
      : {
          core: library.slice(0, at),
          profile: library.slice(at + LIBRARY_SEPARATOR.length),
        }
  }
  // The previously-cached library — what was served (and applied) on
  // the last load. `force-cache` returns the cached copy without
  // revalidating; a miss falls through to network. Used on startup to
  // tell whether the library changed since last load (cached vs
  // fresh), so a reload picks up bootstrap edits made while away.
  const cachedLibrary = async () => {
    try {
      const parts = await Promise.all(
        [LIBRARY_URL, PROFILE_LIBRARY_URL, NOTEBOOK_LIBRARY_URL].map(async (url) => {
          const response = await fetch(url, { cache: "force-cache" })
          return response.ok ? await response.text() : null
        }),
      )
      if (parts.some((part) => part === null)) return null
      // Must match `fetchLibrary`'s shape exactly, or every load reads the
      // cached copy as different from the served one and reseeds.
      const [core, profile, notebook] = parts
      return core + "\n" + notebook + LIBRARY_SEPARATOR + profile
    } catch (_) {
      return null
    }
  }
  // Prime the HTTP cache with the current served libraries so the next
  // load's `cachedLibrary()` reflects what we just applied (otherwise
  // it would re-detect the same change and reseed every load).
  const primeLibraryCache = async () => {
    try {
      await Promise.all(
        [LIBRARY_URL, PROFILE_LIBRARY_URL, NOTEBOOK_LIBRARY_URL].map((url) =>
          fetch(url, { cache: "reload" }),
        ),
      )
    } catch (_) {
      // Non-fatal: a failed prime just means an extra reseed next load.
    }
  }
  const servedWasmHash = async () => {
    const response = await fetch("/", { cache: "no-store" })
    const html = await response.text()
    const match = html.match(/ui-[a-f0-9]+_bg\.wasm/)
    return match ? match[0] : null
  }

  // Re-seed by re-evaluating the library document through the page's
  // own routing context, not a guessed repo/branch. Each mounted
  // `with="branch@repo"` element carries its resolved context;
  // dispatching the `tonk-evaluate` event on it bubbles to the
  // installed host on the document, which routes the document to
  // exactly the branch that element's views read. Re-asserting is
  // idempotent (stable entity URIs), so only the edits land.
  //
  // If several contexts are mounted, every one is re-seeded — each is
  // a distinct context a view might be rendering against.
  const reseed = async (library) => {
    const branches = [...document.querySelectorAll("[with]")]
      .filter((el) => !el.getAttribute("with").includes("{"))
    if (branches.length === 0) {
      // No routing context mounted in this view (e.g. the repo landing
      // page) — there is nothing to re-seed here, which is not a
      // failure. The fetched library is cached; the next view with a
      // context picks it up. Return quietly rather than flagging an
      // error.
      return
    }

    // The PROFILE branch carries a DIFFERENT library: `profile.yaml` (the Hub
    // and the FAB chrome), not `core.yaml`. Seeding core.yaml there would leave
    // every FAB edit unreachable until the profile was recreated — which is
    // exactly what it used to mean.
    const { core, profile } = splitLibrary(library)
    const libraryFor = (branch) =>
      branch.getAttribute("with").includes("@profile:") ? profile : core

    for (const branch of branches) {
      const document = libraryFor(branch)
      // An older cached string may carry no profile half; skip rather than
      // seed `null` onto the profile branch.
      if (!document) continue
      const detail = { document }
      const event = new CustomEvent("tonk-evaluate", {
        detail,
        bubbles: true,
        composed: true,
        cancelable: true,
      })
      branch.dispatchEvent(event)
      // The host calls preventDefault() and writes detail.result (a
      // promise) when it handles the event; an unprevented event means
      // no installed host caught it.
      if (!event.defaultPrevented || !detail.result) {
        throw new Error("tonk-evaluate not handled (no tonk host installed)")
      }
      const result = await detail.result
      console.debug("[hot-swap] reseed", {
        branch,
        commits: result?.commits,
      })
    }
  }

  // A short content hash of the library text, shown in the pill as the
  // live bootstrap "version" (there is no version system, so the
  // content hash is the identity). SHA-256, first 6 hex chars.
  const libraryHash = async (text) => {
    try {
      const bytes = new TextEncoder().encode(text)
      const digest = await crypto.subtle.digest("SHA-256", bytes)
      return [...new Uint8Array(digest)]
        .map((b) => b.toString(16).padStart(2, "0"))
        .join("")
        .slice(0, 6)
    } catch (_) {
      return ""
    }
  }

  let lastLibrary = null
  try {
    lastLibrary = await fetchLibrary()
  } catch (_) {
    // No served library (e.g. not running the tonk shell) — stay inert.
    return
  }
  const baselineWasm = await servedWasmHash().catch(() => null)

  const view = HotSwap.insert(document.documentElement)

  // Changes detected while hot reload is off, held until re-enabled.
  // `pending` is a library text awaiting reseed; `pendingReload` marks
  // a page change that needs a full reload.
  let pending = null
  let pendingReload = false

  // A library change that arrived while this tab was in the background.
  // A reseed writes the branch through the service worker, which
  // propagates to every tab — so only ONE tab needs to apply it, and we
  // let the active (foreground) tab do it to avoid every tab racing to
  // re-evaluate the same document. If no tab is active when the change
  // fires, each tab holds it here and the first to come to the
  // foreground applies it.
  let pendingActive = null

  // Whether this is the tab that should perform the reseed. The
  // foreground tab owns the write; backgrounded tabs defer to it.
  const isActive = () => document.visibilityState === "visible"

  // Cross-tab signal that a library was applied. The reseed itself
  // reaches other tabs' DATA via the SW; this tells their PILLS so they
  // reflect the new version (and drop any held copy of the same change)
  // without re-applying it. A transient fire-and-forget signal, so a
  // BroadcastChannel fits better than a persisted key.
  let appliedChannel = null
  try {
    appliedChannel = new BroadcastChannel("tonk:hot-swap:applied")
  } catch (_) {
    // BroadcastChannel unavailable — tabs just won't cross-reflect
    // applies; each still handles its own WS signal correctly.
  }
  const broadcastApplied = (hash) => {
    try {
      appliedChannel?.postMessage({ type: "applied", hash })
    } catch (_) {
      // Non-fatal: other tabs simply won't reflect this apply.
    }
  }

  // How long the toggled-off announcement stays unfolded before it
  // folds back into a pulsing circle.
  const FOLD_DELAY = 45000
  let foldTimer = null
  const clearFoldTimer = () => {
    if (foldTimer) { clearTimeout(foldTimer); foldTimer = null }
  }

  // Conjure as a folded circle, then unfold the drawer once the
  // conjure (scale-in) has settled — so the pill pops in as a circle
  // first and opens afterward, never appearing already-unfolded.
  const conjure = (glyph, hash) =>
    new Promise((resolve) => {
      clearFoldTimer()
      view.setStatus(glyph, hash)
      view.folded = true    // start folded (a circle)
      view.visible = true   // conjure: scale the circle in
      // Let the conjure animation play, then open the drawer.
      setTimeout(() => {
        view.folded = false // unfold: drawer opens, version reveals
        resolve()
      }, 350)
    })

  // Toggled-ON flow: conjure (circle) → unfold showing the version,
  // pulse while applying, then on completion fold back, clear the icon,
  // and vanish.
  const apply = async (library, glyph = GLYPH_LIBRARY) => {
    const hash = await libraryHash(library)
    await conjure(glyph, hash)
    view.error = false    // clear any prior failure before retrying
    view.pulse = true     // pulsing = applying
    try {
      await reseed(library)
      view.pulse = false
      pending = null
      pendingActive = null
      await primeLibraryCache()
      // Tell other tabs this landed so their pills reflect it (the data
      // already reached them via the SW); they won't re-apply.
      broadcastApplied(hash)
      // Done: fold back, clear the icon to idle, then vanish.
      view.folded = true
      view.setStatus(GLYPH_IDLE, hash)
      setTimeout(() => {
        view.visible = false
        view.folded = false
      }, 600)
    } catch (e) {
      // Applying the new library failed — it didn't lower (a parse /
      // analyze error, a dangling anchor) or the evaluate was
      // rejected. Surface it loudly: red error state, forced visible
      // and unfolded, still pulsing to announce the trouble, so a bad
      // edit can't silently leave repos unseeded. The `error` setter
      // keeps the pulse on; cleared on the next successful apply.
      console.error("[hot-swap]", e)
      view.error = true
      view.folded = false
      view.setStatus(GLYPH_ERROR, "apply failed")
    }
  }

  // Toggled-OFF flow: conjure + unfold showing the version (no pulse),
  // hold ~45s, then fold into a circle that pulses to keep announcing
  // the pending change. Stays until the toggle is flipped on.
  const announce = async (library, glyph = GLYPH_LIBRARY) => {
    pending = library
    await conjure(glyph, await libraryHash(library))
    view.pulse = false    // no pulse while unfolded
    foldTimer = setTimeout(() => {
      view.folded = true  // fold into a circle...
      view.pulse = true   // ...and pulse only while folded
    }, FOLD_DELAY)
  }
  // Re-enabling hot reload applies whatever was held: a page reload
  // takes precedence (the running code may be stale), otherwise the
  // pending library reseed.
  view.onenable = () => {
    if (pendingReload) {
      window.location.reload()
    } else if (pending) {
      apply(pending)
    }
  }

  // Briefly flash the pill to reflect a change another tab applied: the
  // data already arrived here via the SW, so we only update the version
  // label — no reseed. Conjure → show the hash → settle back to hidden.
  const reflectApplied = async (hash) => {
    clearFoldTimer()
    await conjure(GLYPH_LIBRARY, hash)
    view.pulse = false
    view.folded = true
    view.setStatus(GLYPH_IDLE, hash)
    setTimeout(() => {
      view.visible = false
      view.folded = false
    }, 600)
  }

  // Another tab applied a library change. Its reseed already reached
  // our data through the SW, so drop any copy we were holding for the
  // same change and just reflect the new version in the pill.
  if (appliedChannel) {
    appliedChannel.onmessage = (event) => {
      if (event.data?.type !== "applied") return
      pendingActive = null
      pending = null
      clearFoldTimer()
      view.error = false
      reflectApplied(event.data.hash)
    }
  }

  // If a change arrived while this tab was backgrounded and no other
  // tab has applied it yet, apply it when we return to the foreground.
  // Guard on `enabled` (re-read from storage) in case the toggle was
  // flipped off meanwhile.
  document.addEventListener("visibilitychange", () => {
    if (isActive() && pendingActive && view.enabled) {
      const library = pendingActive
      pendingActive = null
      apply(library)
    }
  })

  // Trunk fires `reload` as its pipeline runs, but the `copy-file`
  // re-copy of `core.yaml` into dist can land a beat *after* the
  // signal — so the first fetch right after a reload often still
  // reads the previous content (the "one version behind" race). Poll
  // (cache-sidestepped) until the served text actually differs from
  // what we last applied, or give up after a short window (the change
  // was elsewhere, e.g. a Rust file).
  const awaitChangedLibrary = async () => {
    for (let attempt = 0; attempt < 20; attempt++) {
      let library
      try {
        library = await fetchLibrary()
      } catch (_) {
        return null
      }
      if (library !== lastLibrary) return library
      await new Promise((resolve) => setTimeout(resolve, 100))
    }
    return null
  }

  // Reload now if hot reload is on, else announce the held reload via
  // the off flow (unfold → hold → fold + pulse) until re-enabled.
  const reloadOrHold = () => {
    if (view.enabled) {
      window.location.reload()
    } else {
      pendingReload = true
      clearFoldTimer()
      view.setStatus(GLYPH_RELOAD, "reload")
      view.visible = true
      view.folded = false
      view.pulse = false
      foldTimer = setTimeout(() => {
        view.folded = true
        view.pulse = true
      }, FOLD_DELAY)
    }
  }

  const onChange = async () => {
    // Reload is the safe default: trunk rebuilt *something* and with
    // its own autoreload disabled we own the decision. The ONE case we
    // handle specially is a change confined to the standard library —
    // applied in place without losing page state. Anything else (page
    // wasm, the service-worker bundle, index.html, CSS, fonts, any
    // asset we don't model) reloads, so a change is never dropped.

    // Check the page wasm FIRST — it's a fast single fetch, and a code
    // change must reload regardless of the library. This avoids the
    // multi-second library poll on every Rust edit.
    try {
      const served = await servedWasmHash()
      if (served && baselineWasm && served !== baselineWasm) {
        reloadOrHold()
        return
      }
    } catch (_) {
      // Unknown wasm state — fall through; the library check or the
      // reload fallback still covers the change.
    }

    // Wasm unchanged. Did the library change? (Polls briefly for the
    // re-copied asset to settle.)
    const library = await awaitChangedLibrary()
    if (library !== null) {
      // Library-only change.
      lastLibrary = library
      if (!view.enabled) {
        // Toggled off → announce + hold (the off flow).
        announce(library)
      } else if (isActive()) {
        // Enabled and this is the foreground tab → apply in place. The
        // reseed propagates to the other tabs through the SW; this tab
        // then broadcasts the hash so their pills reflect it.
        apply(library)
      } else {
        // Enabled but backgrounded → don't reseed (the active tab
        // will). Hold it: if no tab is active right now, the first to
        // come to the foreground applies it. If an active tab beats us,
        // its broadcast clears this.
        pendingActive = library
      }
      return
    }

    // Neither wasm nor library changed that we can see — but trunk
    // signalled *something* (index.html, CSS, an asset). Reload.
    reloadOrHold()
  }

  // Wait for at least one routing context to mount, so a load-time
  // reseed has a `with=` element to dispatch on. Returns false if none
  // appears within the window (e.g. a page with no tonk views).
  const awaitBranch = async () => {
    for (let attempt = 0; attempt < 40; attempt++) {
      if (document.querySelector("[with]")) return true
      await new Promise((resolve) => setTimeout(resolve, 100))
    }
    return false
  }

  // On load, reconcile the seeded library with the served one. The
  // HTTP cache holds what was applied on the previous load; comparing
  // it against the fresh copy tells us whether the library changed
  // while the page was away (an SW reactivation / hard reload doesn't
  // re-seed on its own). If it differs, apply it once the views are
  // mounted. `lastLibrary` is the fresh copy fetched at startup.
  const reconcileOnLoad = async () => {
    // Seed the idle hash so a later hover/conjure shows the live
    // version (the pill stays hidden until there's something to show).
    view.setStatus(GLYPH_IDLE, await libraryHash(lastLibrary))
    const cached = await cachedLibrary()
    if (cached !== null && cached === lastLibrary) return
    if (!(await awaitBranch())) return
    // The library changed while away.
    if (!view.enabled) {
      // Toggled off → announce + hold.
      announce(lastLibrary)
    } else if (isActive()) {
      // Foreground → apply; the reseed reaches the other tabs via the
      // SW and the broadcast updates their pills.
      await apply(lastLibrary)
    } else {
      // Background → defer to the active tab; apply on focus if no
      // other tab beats us to it.
      pendingActive = lastLibrary
    }
  }

  const connect = () => {
    const base = document.querySelector("base")?.getAttribute("href") || "/"
    const scheme = window.location.protocol === "https:" ? "wss" : "ws"
    const url = `${scheme}://${window.location.host}${base}.well-known/trunk/ws`
    const ws = new WebSocket(url)
    ws.onmessage = (event) => {
      let message
      try {
        message = JSON.parse(event.data)
      } catch (_) {
        return
      }
      if (message.type === "buildFailure") {
        // Build broke: show the danger pill, keep it spinning, and
        // surface the reason in the label until the next good build.
        view.error = true
        view.setStatus(GLYPH_ERROR, "build failed")
        console.error("[hot-swap] build failed", message.data?.reason ?? "")
        return
      }
      if (message.type === "reload") {
        // A reload signal means the build recovered — clear any error.
        view.error = false
        onChange()
      }
    }
    // Reconnect on drop (trunk restart, brief disconnects).
    ws.onclose = () => setTimeout(connect, 1000)
    ws.onerror = () => ws.close()
  }

  reconcileOnLoad()
  connect()
})()
