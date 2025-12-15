import { useCallback, useEffect, useRef, useState } from "react";
import { type TLStoreSnapshot, useEditor } from "tldraw";
import { getVFSService, type JsonValue } from "@/vfs-client";
import { useVFS } from "../../../hooks/useVFS";
import { resetDesktopService } from "../services/DesktopService";

const CANVAS_STATE_PATH = "/.state/desktop";

export function useCanvasPersistence() {
  const editor = useEditor();
  const { connectionState, resetConnection } = useVFS();
  const hasLoadedRef = useRef(false);
  const [isReady, setIsReady] = useState(false);
  // Flag to prevent saves during bundle switching - avoids race conditions
  const isSwitchingRef = useRef(false);

  // Handle full bundle reload - reset everything for new TonkCore instance
  const handleBundleReload = useCallback(() => {
    console.log(
      "[useCanvasPersistence] Bundle reloaded, resetting all app state",
    );
    hasLoadedRef.current = false;
    setIsReady(false);

    // Reset DesktopService to clear its state and watchers
    resetDesktopService();

    // Reset VFS connection to re-establish watchers with new TonkCore
    // This will trigger 'reconnecting' → 'connected', causing sync middleware to reload
    resetConnection();
  }, [resetConnection]);

  // Handle soft activation - just reload canvas state without destroying services
  const handleBundleActivate = useCallback(async () => {
    console.log(
      "[useCanvasPersistence] Bundle activated, soft refresh canvas state",
    );

    // Block saves during activation to prevent race conditions
    isSwitchingRef.current = true;

    try {
      const vfs = getVFSService();
      const exists = await vfs.exists(CANVAS_STATE_PATH);

      if (!exists) {
        console.log("[useCanvasPersistence] No canvas state to reload");
        return;
      }

      const doc = await vfs.readFile(CANVAS_STATE_PATH);
      const content = doc.content as unknown as {
        schema: TLStoreSnapshot["schema"];
        store: TLStoreSnapshot["store"];
      };

      if (content?.schema && content?.store) {
        const snapshot: TLStoreSnapshot = {
          schema: content.schema,
          store: content.store,
        };

        // Reload canvas state from VFS
        editor.loadSnapshot(snapshot);
        console.log("[useCanvasPersistence] Canvas state reloaded from VFS");
      }
    } catch (err) {
      console.error("[useCanvasPersistence] Error during soft refresh:", err);
    } finally {
      // Re-enable saves after a short delay to let state settle
      setTimeout(() => {
        isSwitchingRef.current = false;
      }, 100);
    }
  }, [editor]);

  // Listen for tonk:bundleReloaded and tonk:activate messages from runtime/launcher
  // tonk:activate is sent when switching back to an already-loaded tonk in the iframe pool
  //   → Soft refresh: reload data from VFS without destroying services
  // tonk:bundleReloaded is sent after the SW loads a new TonkCore
  //   → Full reset: destroy services and reconnect to new TonkCore
  // Note: tonk:deactivate is no longer needed with multi-bundle isolation - each bundle
  // has its own TonkCore instance, so there's no cross-contamination risk
  useEffect(() => {
    const handler = (event: MessageEvent) => {
      if (event.data?.type === "tonk:bundleReloaded") {
        handleBundleReload();
      } else if (event.data?.type === "tonk:activate") {
        handleBundleActivate();
      }
    };

    window.addEventListener("message", handler);
    return () => window.removeEventListener("message", handler);
  }, [handleBundleReload, handleBundleActivate]);

  // Load canvas state once on mount
  useEffect(() => {
    // Wait for VFS to be ready
    if (connectionState !== "connected") {
      return;
    }

    // Only load once
    if (hasLoadedRef.current) {
      return;
    }

    const vfs = getVFSService();
    let cancelled = false;

    // Load canvas state on mount (only runs once)
    (async () => {
      try {
        const exists = await vfs.exists(CANVAS_STATE_PATH);

        if (!exists) {
          hasLoadedRef.current = true;
          setIsReady(true);
          return;
        }

        const doc = await vfs.readFile(CANVAS_STATE_PATH);
        const content = doc.content as unknown as {
          schema: TLStoreSnapshot["schema"];
          store: TLStoreSnapshot["store"];
        };

        if (!cancelled && content.schema && content.store) {
          // Reconstruct tldraw snapshot format
          const snapshot: TLStoreSnapshot = {
            schema: content.schema,
            store: content.store,
          };

          // Use editor.loadSnapshot to restore the canvas state
          editor.loadSnapshot(snapshot);

          hasLoadedRef.current = true;
          setIsReady(true);
        } else {
          hasLoadedRef.current = true;
          setIsReady(true);
        }
      } catch (err) {
        console.error("[useCanvasPersistence] Canvas state load failed", err);
        hasLoadedRef.current = true;
        setIsReady(true);
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [editor, connectionState]);

  // Save canvas state on changes (separate effect)
  useEffect(() => {
    if (connectionState !== "connected") {
      return;
    }

    const vfs = getVFSService();

    // Save canvas state on changes
    const unsubscribe = editor.store.listen(
      async () => {
        // Skip saves during bundle switching to prevent race conditions
        if (isSwitchingRef.current) {
          return;
        }

        try {
          const snapshot = editor.getSnapshot();
          const content = {
            schema: snapshot.document.schema as unknown as JsonValue,
            store: snapshot.document.store as unknown as JsonValue,
          };

          const exists = await vfs.exists(CANVAS_STATE_PATH);

          if (!exists) {
            // First save: create full document
            await vfs.writeFile(CANVAS_STATE_PATH, { content }, true);
            return;
          }

          await vfs.updateFile(CANVAS_STATE_PATH, content);
        } catch (err) {
          console.error(
            "[useCanvasPersistence] ❌ Canvas state save failed",
            err,
          );
        }
      },
      { source: "user", scope: "document" },
    );

    return () => {
      unsubscribe();
    };
  }, [editor, connectionState]);

  // Watch for external changes to canvas state (cross-tab sync)
  // biome-ignore lint/correctness/useExhaustiveDependencies: editor.loadSnapshot is stable, only depends on connectionState
  useEffect(() => {
    if (connectionState !== "connected") {
      return;
    }

    // Don't set up watcher until initial load is complete
    if (!hasLoadedRef.current) {
      return;
    }

    const vfs = getVFSService();
    let watchId: string | null = null;
    let isLoadingExternal = false;

    const setupWatcher = async () => {
      try {
        watchId = await vfs.watchFile(CANVAS_STATE_PATH, async (docData) => {
          // Prevent loading during external update to avoid loops
          if (isLoadingExternal) return;

          try {
            isLoadingExternal = true;

            const content = docData.content as unknown as {
              schema: TLStoreSnapshot["schema"];
              store: TLStoreSnapshot["store"];
            };

            if (!content || !content.schema || !content.store) {
              return;
            }

            // Reconstruct tldraw snapshot format
            const snapshot: TLStoreSnapshot = {
              schema: content.schema,
              store: content.store,
            };

            // Load the updated snapshot from other tab
            editor.loadSnapshot(snapshot);
          } catch (err) {
            console.error(
              "[useCanvasPersistence] Error loading external canvas state:",
              err,
            );
          } finally {
            isLoadingExternal = false;
          }
        });
      } catch (err) {
        console.error(
          "[useCanvasPersistence] Error setting up file watcher:",
          err,
        );
      }
    };

    setupWatcher();

    return () => {
      if (watchId) {
        vfs.unwatchFile(watchId).catch((err) => {
          console.warn("[useCanvasPersistence] Error unwatching file:", err);
        });
      }
    };
  }, [connectionState]);

  return { isReady };
}
