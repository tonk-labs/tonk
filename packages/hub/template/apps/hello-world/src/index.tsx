import React from "react";
import "./index.css";
import { createRoot } from "react-dom/client";
import { BrowserRouter } from "react-router-dom";
import App from "./App";
import { configureSyncEngine, setDocIdPrefix, mapDocId } from "@tonk/keepsync";
import {
  registerServiceWorker,
  unregisterServiceWorker,
} from "./serviceWorkerRegistration";

// Service worker logic based on environment
if (process.env.NODE_ENV === "production") {
  // Only register service worker in production mode
  registerServiceWorker();
} else {
  // In development, make sure to unregister any existing service workers
  unregisterServiceWorker();
}

const wsProtocol = window.location.protocol === "https:" ? "wss:" : "ws:";
const wsUrl = `${wsProtocol}//${window.location.host}/sync`;

configureSyncEngine({
  url: wsUrl,
  onSync: (docId) => console.log(`Document ${docId} synced`),
  onError: (error) => console.error("Sync error:", error),
});

// Extend the Window interface to include our custom properties
declare global {
  interface Window {
    __TONK_SET_DOC_ID_PREFIX__: (prefix: string) => void;
    __TONK_MAP_DOC_ID__: (logicalId: string, actualId: string) => void;
  }
}

// Set up the bridge for the Tonk GUI to control document IDs
if (typeof window !== "undefined") {
  // Function to set document ID prefix from the Tonk GUI
  window.__TONK_SET_DOC_ID_PREFIX__ = (prefix: string) => {
    console.log(`Setting document ID prefix to: ${prefix}`);
    setDocIdPrefix(prefix);
  };

  // Function to map a logical document ID to an actual document ID
  window.__TONK_MAP_DOC_ID__ = (logicalId: string, actualId: string) => {
    console.log(`Mapping document ID: ${logicalId} → ${actualId}`);
    mapDocId(logicalId, actualId);
  };
}

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
