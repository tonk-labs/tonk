import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import Router from "./Router";
import { getVFSService } from "./lib/vfs-service";

// Import sample files utility to make it available in browser console
import "./utils/sampleFiles";

// Initialize VFS before React mounts
getVFSService().initialize('', '').catch(err => {
  console.warn('VFS initialization warning:', err);
});

// biome-ignore lint/style/noNonNullAssertion: <lol>
createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <Router />
  </StrictMode>,
);
