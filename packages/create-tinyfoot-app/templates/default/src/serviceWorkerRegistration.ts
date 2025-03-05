// This service worker registration file should be imported in the main entry point of the application

// Explicitly check if we're in production mode using webpack-injected environment variable
// This is more reliable than relying on process.env.NODE_ENV
const isProduction = process.env.NODE_ENV === "production";

// Function to unregister any existing service workers
export function unregisterServiceWorker() {
  if ("serviceWorker" in navigator) {
    navigator.serviceWorker.ready
      .then((registration) => {
        registration.unregister();
        console.log("Service worker unregistered");
      })
      .catch((error) => {
        console.error("Error unregistering service worker:", error);
      });
  }
}

// Check if service workers are supported in the current browser
export function registerServiceWorker() {
  // In development mode, actively unregister any service workers
  if (!isProduction) {
    console.log(
      "Development mode detected - unregistering any service workers"
    );
    unregisterServiceWorker();
    return;
  }

  if ("serviceWorker" in navigator) {
    // Use a timeout to ensure the registration doesn't interfere with page load
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
