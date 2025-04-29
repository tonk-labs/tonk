import React from "react";
import "./index.css";
import { createRoot } from "react-dom/client";
import { BrowserRouter } from "react-router-dom";
import App from "./App";
import { configureSyncEngine, NetworkAdapterInterface } from "@tonk/keepsync";
import { BrowserWebSocketClientAdapter } from "@automerge/automerge-repo-network-websocket";
import { IndexedDBStoraegAdapter } from "@automerge/automerge-repo-storage-indexeddb";

const wsProtocol = window.location.protocol === "https:" ? "wss:" : "ws:";
const wsUrl = `${wsProtocol}//${window.location.host}/sync`;
const wsAdapter = new BrowserWebSocketClientAdapter(wsUrl);
const storage = new IndexedDBStoraegAdapter();

configureSyncEngine({
  hostname: window.location.host,
  networkAdapters: [wsAdapter as any as NetworkAdapterInterface],
  storage,
});

const container = document.getElementById("root");
if (!container) throw new Error("Failed to find the root element");
const root = createRoot(container);

root.render(
  <React.StrictMode>
    <BrowserRouter>
      <App />
    </BrowserRouter>
  </React.StrictMode>
);
