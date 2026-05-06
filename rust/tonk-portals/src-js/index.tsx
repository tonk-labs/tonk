// Custom-element entry. Registers `<tonk-portals>` and mounts the
// React panel grid inside a scoped container under the element.
//
// Why a custom element (vs. just a <script>): the Leptos shell can
// drop `<tonk-portals repo branch host>` into its template like any
// other tag, and attribute changes (e.g. branch switch) flow in via
// `attributeChangedCallback` without re-running React's whole tree.
//
// Why a scoped class container instead of shadow DOM: webawesome
// styles and fonts on the host page are useful inside the panel
// chrome too, and shadow DOM would cut us off from them. We trade
// style isolation for ergonomics and prefix our own selectors.

import { createRoot, type Root } from "react-dom/client";
import { App, type AppProps } from "./App";
import css from "./styles.css";

// Portals are a UI layer over a repo. The element knows the
// repo name and the host id; individual artifact tiles inside
// the React app pick their own branch when composing data URLs
// (`/api/repository/{repo}/branch/{branch}/host/{host}/{entity}`).
const ATTRS = ["repo", "host"] as const;
type AttrName = (typeof ATTRS)[number];

class TonkPortalsElement extends HTMLElement {
  static get observedAttributes(): readonly string[] {
    return ATTRS;
  }

  private root: Root | null = null;
  private mountNode: HTMLDivElement | null = null;

  connectedCallback() {
    if (this.root) return;
    injectStylesOnce();
    const mount = document.createElement("div");
    mount.className = "tonk-portals-root";
    this.appendChild(mount);
    this.mountNode = mount;
    this.root = createRoot(mount);
    this.render();
  }

  disconnectedCallback() {
    this.root?.unmount();
    this.root = null;
    if (this.mountNode && this.mountNode.parentNode === this) {
      this.removeChild(this.mountNode);
    }
    this.mountNode = null;
  }

  attributeChangedCallback(_name: AttrName) {
    this.render();
  }

  private render() {
    if (!this.root) return;
    const props: AppProps = {
      repo: this.getAttribute("repo") ?? "",
      host: this.getAttribute("host") ?? "",
    };
    this.root.render(<App {...props} />);
  }
}

const STYLE_ID = "tonk-portals-styles";

function injectStylesOnce() {
  if (document.getElementById(STYLE_ID)) return;
  const style = document.createElement("style");
  style.id = STYLE_ID;
  style.textContent = css;
  document.head.appendChild(style);
}

if (!customElements.get("tonk-portals")) {
  customElements.define("tonk-portals", TonkPortalsElement);
}
