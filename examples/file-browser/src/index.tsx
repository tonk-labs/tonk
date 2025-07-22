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

const basename =
  import.meta.env.VITE_BASE_PATH !== "/"
    ? import.meta.env.VITE_BASE_PATH?.replace(/\/$/, "")
    : "";

root.render(
  <React.StrictMode>
    <BrowserRouter basename={basename}>
      <App />
    </BrowserRouter>
  </React.StrictMode>,
);
