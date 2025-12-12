import { useCallback, useEffect, useRef, useState } from "react";
import { Sidebar } from "./components/sidebar/sidebar";
import GradientLogo from "./components/ui/gradientLogo";
import { GradientText } from "./components/ui/gradientText";
import { bundleManager } from "./launcher/services/bundleManager";
import { bundleStorage } from "./launcher/services/bundleStorage";
import type { Bundle } from "./launcher/types";

interface PooledIframe {
  bundleId: string;
  url: string;
  lastAccessed: number;
}

const MAX_IFRAME_POOL_SIZE = 5;

function App() {
  const [bundles, setBundles] = useState<Bundle[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [importing, setImporting] = useState(false);
  const [activeBundleId, setActiveBundleId] = useState<string | null>(null);
  const [iframePool, setIframePool] = useState<Map<string, PooledIframe>>(
    () => new Map(),
  );
  const iframeRefs = useRef<Map<string, HTMLIFrameElement>>(new Map());

  const loadBundles = useCallback(async () => {
    try {
      setLoading(true);
      const list = await bundleManager.listBundles();
      setBundles(list);
      setError(null);
    } catch (err) {
      console.error("Failed to load bundles:", err);
      setError("Failed to load bundles");
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    loadBundles();
  }, [loadBundles]);

  const handleLaunch = async (id: string) => {
    try {
      // Verify bundle exists before launching
      const bundle = await bundleStorage.get(id);
      if (!bundle) {
        throw new Error("Bundle not found");
      }

      const url = `/space/_runtime/index.html?bundleId=${encodeURIComponent(id)}`;
      const now = Date.now();

      // Check if iframe already exists in pool (switching back to it)
      const iframeAlreadyExists = iframePool.has(id);

      // With multi-bundle isolation, we no longer need to deactivate the previous iframe
      // Each bundle maintains its own TonkCore instance in the SW

      setIframePool((prevPool) => {
        const newPool = new Map(prevPool);

        // Update or add the iframe to the pool
        newPool.set(id, {
          bundleId: id,
          url,
          lastAccessed: now,
        });

        // Evict oldest iframes if pool exceeds max size
        if (newPool.size > MAX_IFRAME_POOL_SIZE) {
          const entries = Array.from(newPool.entries());
          // Sort by lastAccessed ascending (oldest first)
          entries.sort((a, b) => a[1].lastAccessed - b[1].lastAccessed);

          // Remove oldest entries until we're at max size
          const toRemove = entries.slice(
            0,
            entries.length - MAX_IFRAME_POOL_SIZE,
          );
          for (const [bundleId] of toRemove) {
            // Don't evict the one we're about to activate
            if (bundleId !== id) {
              // Send unloadBundle message to SW to free resources
              if (navigator.serviceWorker.controller) {
                navigator.serviceWorker.controller.postMessage({
                  type: "unloadBundle",
                  launcherBundleId: bundleId,
                });
                console.log(
                  "[Launcher] Sent unloadBundle for evicted iframe:",
                  bundleId,
                );
              }
              newPool.delete(bundleId);
              // Clean up ref for evicted iframe
              iframeRefs.current.delete(bundleId);
            }
          }
        }

        return newPool;
      });

      setActiveBundleId(id);

      // If iframe already existed, notify it that it's being reactivated
      // With multi-bundle isolation, the bundle is already loaded in SW
      if (iframeAlreadyExists) {
        const iframe = iframeRefs.current.get(id);
        if (iframe?.contentWindow) {
          iframe.contentWindow.postMessage(
            {
              type: "tonk:activate",
              launcherBundleId: id,
            },
            "*",
          );
        }
      }
    } catch (err) {
      console.error("Failed to launch bundle:", err);
      alert("Failed to launch bundle");
    }
  };

  const handleFileUpload = async (
    event: React.ChangeEvent<HTMLInputElement>,
  ) => {
    const file = event.target.files?.[0];
    if (!file) return;

    try {
      setImporting(true);
      const id = await bundleManager.loadBundleFromFile(file);
      await loadBundles();
      // Auto-launch the new bundle
      await handleLaunch(id);

      // Reset input
      event.target.value = "";
    } catch (err) {
      console.error("Failed to import bundle:", err);
      alert(err instanceof Error ? err.message : "Failed to import bundle");
    } finally {
      setImporting(false);
    }
  };

  return (
    <div className="flex h-screen font-sans overflow-hidden w-full">
      {/* Sidebar */}
      <Sidebar
        bundles={bundles}
        handleLaunch={handleLaunch}
        handleFileUpload={handleFileUpload}
        importing={importing}
        activeBundleId={activeBundleId}
      />

      {/* Main Content */}
      <main className="flex-1 relative bg-stone-100 dark:bg-night-900 z-0">
        {error && (
          <div className="flex items-center justify-center h-full text-stone-400">
            <div className="text-center">
              <svg
                className="w-16 h-16 mx-auto mb-4 opacity-20"
                fill="none"
                stroke="currentColor"
                viewBox="0 0 24 24"
              >
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  strokeWidth={2}
                  d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-3L13.732 4c-.77-1.333-2.694-1.333-3.464 0L3.34 16c-.77 1.333.192 3 1.732 3z"
                />
              </svg>
              <p>{error}</p>
            </div>
          </div>
        )}
        {loading && "loading"}
        {/* Render all pooled iframes, showing only the active one */}
        {Array.from(iframePool.values()).map((pooledIframe) => (
          <iframe
            key={pooledIframe.bundleId}
            ref={(el) => {
              if (el) {
                iframeRefs.current.set(pooledIframe.bundleId, el);
              }
            }}
            src={pooledIframe.url}
            className="absolute inset-0 w-full h-full border-none"
            style={{
              visibility:
                pooledIframe.bundleId === activeBundleId ? "visible" : "hidden",
              pointerEvents:
                pooledIframe.bundleId === activeBundleId ? "auto" : "none",
            }}
            title={
              pooledIframe.bundleId
                ? `Runtime for bundle ${pooledIframe.bundleId}`
                : "Runtime"
            }
            allow="clipboard-read; clipboard-write"
          />
        ))}
        {/* Show placeholder when no bundle is active */}
        {!activeBundleId && (
          <div className="flex items-center justify-center h-full text-stone-400">
            <div className="flex items-center justify-center flex-col text-center">
              <GradientLogo />
              <GradientText className="font-gestalt text-lg font-medium mt-4 dark:invert-100">
                select a bundle to launch
              </GradientText>
            </div>
          </div>
        )}
      </main>
    </div>
  );
}

export default App;
