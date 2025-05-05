// This service worker registration file should be imported in the main entry point of the application

// Explicitly check if we're in production mode
const isProduction = import.meta.env.PROD;

// Function to unregister any existing service workers
export function unregisterServiceWorker() {
  if ("serviceWorker" in navigator) {
    navigator.serviceWorker.ready
      .then((registration) => {
        registration.unregister();
      })
      .catch((error) => {
        console.error("Error unregistering service worker:", error);
      });
  }
}

export function registerServiceWorker() {
  // In development mode, actively unregister any service workers
  if (!isProduction) {
    console.log(
      "Development mode detected - unregistering any service workers",
    );
    unregisterServiceWorker();
    return;
  }

  if ("serviceWorker" in navigator) {
    const registerSW = () => {
      const swUrl = "/service-worker.js";

      navigator.serviceWorker
        .register(swUrl)
        .then((registration) => {
          // Check for updates on page reload
          registration.addEventListener("updatefound", () => {
            const installingWorker = registration.installing;
            if (installingWorker) {
              installingWorker.addEventListener("statechange", () => {
                if (installingWorker.state === "installed") {
                  if (navigator.serviceWorker.controller) {
                    // New content is available
                    console.log(
                      "New content is available. Please refresh the page.",
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
    };

    if (document.readyState === "complete") {
      // Document already loaded, registering immediately
      registerSW();
    } else {
      // Setting up load event listener
      window.addEventListener("load", () => {
        // Window load event fired
        registerSW();
      });

      // Backup: also try after a short delay
      setTimeout(() => {
        if (!navigator.serviceWorker.controller) {
          // No service worker controller found, trying registration again
          registerSW();
        }
      }, 3000);
    }
  } else {
    console.warn("Service workers are not supported in this browser.");
  }
}
