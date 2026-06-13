// Render a key as a row of color-coded, tooltipped segments — the
// components sit directly adjacent, distinguished by color alone (no
// separator glyph), like the index-key diagram. Each segment carries a
// wa-tooltip naming what it is. Optionally front-coded against a parent
// key so shared leading segments dim, surfacing only where this key
// diverges.
//
// Requires Web Awesome's <wa-tooltip> on the page (the host app loads
// it). If absent, the segment's `title` attribute is the fallback.

import { frontCode, type KeySegmentString, keySegments } from "./key-string.js";


/** Monotonic id source for anchoring wa-tooltips to segments. */
let segId = 0;

/** Build the colored, tooltipped segment row for a key string. */
export function renderKeyRow(
  keyStr: string,
  parentKey: string | null = null,
): DocumentFragment {
  const segs = keySegments(keyStr);
  const parent = parentKey ? keySegments(parentKey) : null;
  const coded = frontCode(segs, parent);

  const frag = document.createDocumentFragment();
  // Segments sit directly adjacent — color alone separates them.
  coded.forEach((seg) => frag.appendChild(segmentEl(seg)));
  return frag;
}

function segmentEl(seg: KeySegmentString & { shared: boolean }): HTMLElement {
  const span = document.createElement("span");
  span.className = "key-seg" + (seg.shared ? " shared" : "");
  span.style.color = seg.color;
  span.textContent = seg.text;
  span.title = `${seg.label}: ${seg.full}`;

  // Prefer a wa-tooltip when available; fall back to `title` otherwise.
  if (customElements.get("wa-tooltip")) {
    const tip = document.createElement("wa-tooltip");
    tip.textContent = seg.label;
    // wa-tooltip targets by `for` (an id) or by wrapping; wrapping is
    // simplest and self-contained.
    const wrap = document.createElement("span");
    wrap.className = "key-seg-wrap";
    wrap.appendChild(span);
    wrap.appendChild(tip);
    // Anchor the tooltip to the segment via a generated id.
    const id = `kseg-${segId++}`;
    span.id = id;
    (tip as HTMLElement & { for: string }).for = id;
    return wrap;
  }
  return span;
}

/** Shared CSS for key rows — include in a component's shadow style. */
export const KEY_ROW_STYLE = `
  .key-seg { font-weight: var(--wa-font-weight-semibold, 600); padding: 0 1px; }
  .key-seg.shared { opacity: 0.3; font-weight: var(--wa-font-weight-normal, 400); }
  .key-seg-wrap { display: inline; }
`;
