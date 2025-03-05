// This service worker registration file should be imported in the main entry point of the application

// Check if service workers are supported in the current browser
export function registerServiceWorker() {
  if ("serviceWorker" in navigator) {
    window.addEventListener("load", () => {
      const swUrl = "/service-worker.js";

      navigator.serviceWorker
        .register(swUrl)
        .then((registration) => {
          console.log("Service Worker registered: ", registration);

          // Check for updates on page reload
          registration.addEventListener("updatefound", () => {
            const installingWorker = registration.installing;
            if (installingWorker) {
              installingWorker.addEventListener("statechange", () => {
                if (installingWorker.state === "installed") {
                  if (navigator.serviceWorker.controller) {
                    // New content is available
                    console.log(
                      "New content is available. Please refresh the page."
                    );
                  } else {
                    // Content is cached for offline use
                    console.log("Content is cached for offline use.");
                  }
                }
              });
            }
          });
        })
        .catch((error) => {
          console.error("Error during service worker registration:", error);
        });
    });
  } else {
    console.log("Service workers are not supported in this browser.");
  }
}
