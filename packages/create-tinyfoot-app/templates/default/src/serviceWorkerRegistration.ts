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
    // Check if this is a standalone PWA on iOS
    const isIOSPWA = window.matchMedia("(display-mode: standalone)").matches;

    // Check if this is the first launch after installation
    const isFirstLaunch = !localStorage.getItem("pwaInitialized");

    const registerSW = () => {
      const swUrl = "/service-worker.js";

      navigator.serviceWorker
        .register(swUrl)
        .then((registration) => {
          // For iOS PWAs on first launch, force cache all critical resources
          if (isIOSPWA && isFirstLaunch) {
            // Mark as initialized for future launches
            localStorage.setItem("pwaInitialized", "true");

            // Force cache warming for critical resources
            if (registration.active) {
              registration.active.postMessage({
                type: "CACHE_WARMING",
                payload: { forceUpdate: true },
              });
            }
          }

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

                    // For iOS PWAs, show a message to the user
                    if (isIOSPWA && isFirstLaunch) {
                      showIOSPWAMessage();
                    }
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

    // Function to show a message for iOS PWA users
    const showIOSPWAMessage = () => {
      // Only show if we're in a standalone PWA on iOS
      const isIOS =
        /iPad|iPhone|iPod/.test(navigator.userAgent) ||
        (navigator.userAgent.includes("Mac") && "ontouchend" in document);

      if (window.matchMedia("(display-mode: standalone)").matches && isIOS) {
        const notification = document.createElement("div");
        notification.style.position = "fixed";
        notification.style.bottom = "20px";
        notification.style.left = "50%";
        notification.style.transform = "translateX(-50%)";
        notification.style.backgroundColor = "#4a4a4a";
        notification.style.color = "white";
        notification.style.padding = "12px 20px";
        notification.style.borderRadius = "8px";
        notification.style.boxShadow = "0 2px 10px rgba(0,0,0,0.2)";
        notification.style.zIndex = "9999";
        notification.style.textAlign = "center";
        notification.style.maxWidth = "90%";
        notification.style.fontSize = "14px";

        notification.textContent =
          "App is ready for offline use! For best results, close the app completely and reopen.";

        document.body.appendChild(notification);

        // Remove after 8 seconds
        setTimeout(() => {
          if (notification.parentNode) {
            notification.parentNode.removeChild(notification);
          }
        }, 8000);
      }
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
