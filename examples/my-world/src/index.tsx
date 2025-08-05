import React from "react";
import "./index.css";
import "./styles/mobile-fixes.css";
import { createRoot } from "react-dom/client";
import { BrowserRouter } from "react-router-dom";
import App from "./App";
import { configureSyncEngine } from "@tonk/keepsync";
import { IndexedDBStorageAdapter } from "@automerge/automerge-repo-storage-indexeddb";
import { BrowserWebSocketClientAdapter } from "@automerge/automerge-repo-network-websocket";

// Setup sync engine
const wsProtocol = window.location.protocol === "https:" ? "wss:" : "ws:";
const wsUrl = `${wsProtocol}//${window.location.host}/sync`;
const wsAdapter = new BrowserWebSocketClientAdapter(wsUrl);
const storage = new IndexedDBStorageAdapter();

const engine = await configureSyncEngine({
  url: `${window.location.protocol}//${window.location.host}`,
  network: [wsAdapter as any],
  storage,
});

await engine.whenReady();

const container = document.getElementById("root");
if (!container) throw new Error("Failed to find the root element");
const root = createRoot(container);

root.render(
  <React.StrictMode>
    <BrowserRouter>
      <App />
    </BrowserRouter>
  </React.StrictMode>,
);
