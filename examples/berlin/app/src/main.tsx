import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import "./index.css";
import App from "./App.tsx";
import { getVFSService } from "./lib/vfs-service";

// Initialize VFS before React mounts to ensure stores can hydrate immediately
// The service worker is already initialized by host-web, so this connects to it
getVFSService().initialize('', '').catch(err => {
  console.warn('VFS initialization warning:', err);
});

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
