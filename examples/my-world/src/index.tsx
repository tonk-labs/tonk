import React from "react";
import "./index.css";
import "./styles/mobile-fixes.css";
import { createRoot } from "react-dom/client";
import { BrowserRouter } from "react-router-dom";
import App from "./App";
import { configureSyncEngine } from "@tonk/keepsync";
import {
  registerServiceWorker,
  unregisterServiceWorker,
} from "./serviceWorkerRegistration";
import { IndexedDBStorageAdapter } from "@automerge/automerge-repo-storage-indexeddb";
import { BrowserWebSocketClientAdapter } from "@automerge/automerge-repo-network-websocket";

// Service worker logic based on environment
if (process.env.NODE_ENV === "production") {
  // Only register service worker in production mode
  registerServiceWorker();
} else {
  // In development, make sure to unregister any existing service workers
  unregisterServiceWorker();
}

const wsProtocol = window.location.protocol === "https:" ? "wss:" : "ws:";
const wsUrl = `${wsProtocol}//${window.location.host}`;

configureSyncEngine({
  storage: new IndexedDBStorageAdapter(),
  networkAdapters: [new BrowserWebSocketClientAdapter(wsUrl)],
});

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
