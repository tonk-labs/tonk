import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import "./index.css";
import Router from "./Router";
import { getVFSService } from "./lib/vfs-service";

// Initialize VFS before React mounts
getVFSService().initialize('', '').catch(err => {
  console.warn('VFS initialization warning:', err);
});

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <Router />
  </StrictMode>,
);
