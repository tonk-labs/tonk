import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import App from "./App";
import "./index.css";

// Ensure no root-scoped Service Worker is interfering
if ("serviceWorker" in navigator) {
  navigator.serviceWorker.getRegistrations().then((registrations) => {
    for (const registration of registrations) {
      // Unregister if the scope is exactly the root - we only want /space/ scope
      if (
        registration.scope === `${window.location.origin}/` &&
        !registration.scope.includes("/space/")
      ) {
        console.log(
          "Unregistering root Service Worker to prevent conflicts:",
          registration.scope,
        );
        registration.unregister();
      }
    }
  });
}

const rootElement = document.getElementById("root");
if (rootElement) {
  createRoot(rootElement).render(
    <StrictMode>
      <App />
    </StrictMode>,
  );
}
