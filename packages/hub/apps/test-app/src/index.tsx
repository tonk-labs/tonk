import React from "react";
import "./index.css";
import { createRoot } from "react-dom/client";
import { BrowserRouter, HashRouter } from "react-router-dom";
import App from "./App";
import { configureSyncEngine } from "@tonk/keepsync";
import {
  registerServiceWorker,
  unregisterServiceWorker,
} from "./serviceWorkerRegistration";

// Only use service workers if not in Electron environment and in production
if (process.env.IS_ELECTRON !== "true") {
  if (process.env.NODE_ENV === "production") {
    registerServiceWorker();
  } else {
    unregisterServiceWorker();
  }
}

// Configure sync engine with appropriate URL
// If in Electron environment, use localhost if no host is available
const wsProtocol = window.location.protocol === "https:" ? "wss:" : "ws:";
let host = window.location.host;
if (!host && process.env.IS_ELECTRON === "true") {
  host = "localhost:8080";
}
const wsUrl = `${wsProtocol}//${host || "localhost:8080"}/sync`;

configureSyncEngine({
  url: wsUrl,
  onSync: (docId) => console.log(`Document ${docId} synced`),
  onError: (error) => console.error("Sync error:", error),
});

const container = document.getElementById("root");
if (!container) throw new Error("Failed to find the root element");
const root = createRoot(container);

// Use HashRouter for Electron to avoid file path issues
const Router = process.env.IS_ELECTRON === "true" ? HashRouter : BrowserRouter;

root.render(
  <React.StrictMode>
    <Router>
      <App />
    </Router>
  </React.StrictMode>,
);
