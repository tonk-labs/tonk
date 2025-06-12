import React from "react";
import "./index.css";
import { createRoot } from "react-dom/client";
import { BrowserRouter } from "react-router-dom";
import App from "./App";
import { configureSyncEngine } from "@tonk/keepsync";
import { BrowserWebSocketClientAdapter } from "@automerge/automerge-repo-network-websocket";
import { IndexedDBStorageAdapter } from "@automerge/automerge-repo-storage-indexeddb";

const wsProtocol = window.location.protocol === "https:" ? "wss:" : "ws:";
const wsUrl = `${wsProtocol}//${window.location.host}/sync`;
const wsAdapter = new BrowserWebSocketClientAdapter(wsUrl);
const storage = new IndexedDBStorageAdapter();

const engine = configureSyncEngine({
  url: `${window.location.protocol}//${window.location.host}`,
  network: [wsAdapter as any],
  storage,
});

await engine.whenReady();

const container = document.getElementById("root");
if (!container) throw new Error("Failed to find the root element");
const root = createRoot(container);

// Get the base path from the document's base tag or current path
const getBasename = () => {
  const base = document.querySelector("base")?.getAttribute("href");
  if (base && base !== "/") {
    return base.replace(/\/$/, ""); // Remove trailing slash
  }

  // Fallback: detect from current path if it looks like we're in a sub-route
  const path = window.location.pathname;
  const segments = path.split("/").filter(Boolean);

  // If we're clearly in a sub-route (e.g., /file-browser/something), use the first segment
  if (segments.length > 0 && !path.endsWith(".html") && !path.includes(".")) {
    return `/${segments[0]}`;
  }

  return "";
};

const basename = getBasename();

root.render(
  <React.StrictMode>
    <BrowserRouter basename={basename}>
      <App />
    </BrowserRouter>
  </React.StrictMode>,
);
