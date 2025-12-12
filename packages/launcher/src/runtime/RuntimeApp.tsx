import { useCallback, useEffect, useRef } from "react";
import { bundleStorage } from "../launcher/services/bundleStorage";
import { ErrorScreen } from "./components/screens/ErrorScreen";
import { LoadingScreen } from "./components/screens/LoadingScreen";
import { TonkProvider, useTonk } from "./context/TonkContext";
import { useServiceWorker } from "./hooks/useServiceWorker";
import { ScreenState } from "./types";

function AppContent() {
  const { screenState, showLoadingScreen, showError } = useTonk();
  const { queryAvailableApps, sendMessage, confirmBoot } = useServiceWorker();
  const initializingRef = useRef(false);

  // Wait for service worker to be ready
  const waitForServiceWorkerReady = useCallback(async () => {
    return new Promise<void>((resolve) => {
      let timeoutId: number | undefined;
      let resolved = false;

      const maybeResolve = () => {
        if (!resolved) {
          resolved = true;
          if (timeoutId) clearTimeout(timeoutId);
          navigator.serviceWorker.removeEventListener(
            "message",
            messageHandler,
          );
          resolve();
        }
      };

      const messageHandler = (event: MessageEvent) => {
        if (event.data && event.data.type === "ready") {
          console.log("Service worker is ready to handle requests", event.data);
          maybeResolve();
        }
      };

      navigator.serviceWorker.addEventListener("message", messageHandler);

      if (navigator.serviceWorker.controller) {
        console.log("Service worker is controlling page, sending ping...");
        // Send ping to get ready response (works even if SW was already active)
        navigator.serviceWorker.controller.postMessage({ type: "ping" });
      }

      // Short timeout - ping should respond quickly if SW is healthy
      timeoutId = window.setTimeout(() => {
        if (!resolved) {
          console.log(
            "Service worker ready check timed out, proceeding anyway",
          );
          maybeResolve();
        }
      }, 500);
    });
  }, []);

  // Boot the first available app
  const bootFirstApp = useCallback(async () => {
    showLoadingScreen("Loading application...");

    try {
      const apps = await queryAvailableApps();

      if (apps.length > 0) {
        await confirmBoot(apps[0]);
      } else {
        showError("No applications found in bundle");
      }
    } catch (error: unknown) {
      console.error("Failed to boot application:", error);
      const message = error instanceof Error ? error.message : String(error);
      showError(`Failed to load application: ${message}`);
    }
  }, [showLoadingScreen, queryAvailableApps, confirmBoot, showError]);

  // Handle activation message from parent (when iframe becomes visible again)
  useEffect(() => {
    const handleActivate = async (event: MessageEvent) => {
      if (event.data?.type !== "tonk:activate") return;

      console.log("[Runtime] Received tonk:activate, reloading bundle...");

      const urlParams = new URLSearchParams(window.location.search);
      const bundleId = urlParams.get("bundleId");

      if (!bundleId) {
        console.warn("[Runtime] No bundleId in URL, cannot reload");
        return;
      }

      try {
        const bundleData = await bundleStorage.get(bundleId);
        if (!bundleData) {
          console.error("[Runtime] Bundle not found:", bundleId);
          return;
        }

        // Re-send loadBundle to ensure correct TonkCore is active in SW
        // Include launcherBundleId so SW can differentiate bundles with same rootId
        const response = await sendMessage({
          type: "loadBundle",
          bundleBytes: bundleData.bytes,
          manifest: bundleData.manifest,
          launcherBundleId: bundleId,
        });

        // @ts-expect-error - Response type is generic
        if (response.success) {
          // @ts-expect-error - Response type is generic
          if (response.skipped) {
            console.log("[Runtime] Bundle already active, no reload needed");
          } else {
            console.log(
              "[Runtime] Bundle reloaded, notifying app to reset state",
            );
            // Notify the app (desktonk) to reset its state and reload from VFS
            window.postMessage({ type: "tonk:bundleReloaded" }, "*");
          }
        } else {
          // @ts-expect-error - Response type is generic
          console.error("[Runtime] Failed to reload bundle:", response.error);
        }
      } catch (error) {
        console.error("[Runtime] Error reloading bundle:", error);
      }
    };

    window.addEventListener("message", handleActivate);
    return () => window.removeEventListener("message", handleActivate);
  }, [sendMessage]);

  // Initialize and boot
  useEffect(() => {
    const notifyServiceWorkerSupport = (supported: boolean, error?: string) => {
      if (window.parent !== window) {
        window.parent.postMessage(
          {
            type: "tonk:serviceWorkerSupport",
            supported,
            error,
            timestamp: Date.now(),
          },
          "*",
        );
      }
    };

    const initialize = async () => {
      // Prevent duplicate initialization (React StrictMode, etc)
      if (initializingRef.current) {
        console.log("Already initializing, skipping duplicate call");
        return;
      }
      initializingRef.current = true;

      const urlParams = new URLSearchParams(window.location.search);
      const bundleId = urlParams.get("bundleId");

      await waitForServiceWorkerReady();

      if (bundleId) {
        showLoadingScreen("Loading bundle...");

        try {
          // Fetch bundle bytes from shared IndexedDB
          const bundleData = await bundleStorage.get(bundleId);
          if (!bundleData) {
            showError(`Bundle not found: ${bundleId}`);
            return;
          }

          console.log("Fetched bundle from IndexedDB:", {
            id: bundleId,
            size: bundleData.bytes.byteLength,
          });

          // Send bundle bytes to service worker via loadBundle message
          // Include cached manifest to skip redundant Bundle.fromBytes in SW
          // Include launcherBundleId so SW can differentiate bundles with same rootId
          const response = await sendMessage({
            type: "loadBundle",
            bundleBytes: bundleData.bytes,
            manifest: bundleData.manifest,
            launcherBundleId: bundleId,
          });

          // @ts-expect-error - Response type is generic
          if (response.success) {
            console.log("Bundle loaded successfully from IndexedDB");
            await bootFirstApp();
          } else {
            // @ts-expect-error - Response type is generic
            showError(`Failed to load bundle: ${response.error}`);
          }
        } catch (error: unknown) {
          console.error("Error loading bundle from IndexedDB:", error);
          const message =
            error instanceof Error ? error.message : String(error);
          showError(`Error loading bundle: ${message}`);
        }
      } else {
        // No bundleId - try to boot from already loaded bundle (cached state)
        await bootFirstApp();
      }
    };

    if ("serviceWorker" in navigator) {
      notifyServiceWorkerSupport(true);

      if (navigator.serviceWorker.controller) {
        console.log("Service worker is already controlling the page");
        initialize();
      } else {
        // SW is at /space/service-worker-bundled.js, runtime is at /space/_runtime/
        // Use explicit scope /space/ to intercept all /space/* requests
        const serviceWorkerUrl = "../service-worker-bundled.js";

        navigator.serviceWorker
          .register(serviceWorkerUrl, { type: "module", scope: "/space/" })
          .then((registration) => {
            if (import.meta.env.DEV) {
              setInterval(() => {
                registration.update();
              }, 3000);
            }

            registration.addEventListener("updatefound", () => {
              const newWorker = registration.installing;
              if (newWorker) {
                console.log("[SW] New service worker installing...");
                newWorker.addEventListener("statechange", () => {
                  if (
                    newWorker.state === "installed" &&
                    navigator.serviceWorker.controller
                  ) {
                    console.log(
                      "[SW] New service worker installed, reloading...",
                    );
                    newWorker.postMessage({ type: "skipWaiting" });
                    window.location.reload();
                  }
                });
              }
            });
          })
          .catch((err) => {
            console.log("ServiceWorker registration failed: ", err);
            const errorMsg =
              "Service Worker registration failed.\n\n" +
              "Firefox does not yet support ES modules in Service Workers.\n\n" +
              "Please use Chrome or Safari to run Tonks.";
            showError(errorMsg);
            notifyServiceWorkerSupport(false, errorMsg);
          });

        console.log("Waiting for service worker to take control...");
        navigator.serviceWorker.addEventListener(
          "controllerchange",
          async () => {
            console.log("Service worker now controlling the page");
            await initialize();
          },
        );
      }
    } else {
      const errorMsg = "Service Workers are not supported in this browser.";
      showError(errorMsg);
      notifyServiceWorkerSupport(false, errorMsg);
    }
  }, [
    bootFirstApp,
    sendMessage,
    showLoadingScreen,
    showError,
    waitForServiceWorkerReady,
  ]);

  return (
    <div className="w-full h-full">
      {screenState === ScreenState.LOADING && <LoadingScreen />}
      {screenState === ScreenState.ERROR && <ErrorScreen />}
    </div>
  );
}

export function RuntimeApp() {
  return (
    <TonkProvider>
      <AppContent />
    </TonkProvider>
  );
}
