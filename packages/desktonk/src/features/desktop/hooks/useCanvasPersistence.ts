import { useEffect, useRef, useState } from "react";
import { type Editor, type TLStoreSnapshot, useEditor } from "tldraw";
import { getVFSService, type JsonValue } from "@/vfs-client";
import { useVFS } from "../../../hooks/useVFS";

const CANVAS_STATE_PATH = "/.state/desktop";

/**
 * Helper to load canvas snapshot from VFS into editor.
 * Shared by initial load and external change watcher.
 */
async function loadCanvasSnapshot(editor: Editor): Promise<boolean> {
  const vfs = getVFSService();
  const exists = await vfs.exists(CANVAS_STATE_PATH);
  if (!exists) return false;

  const doc = await vfs.readFile(CANVAS_STATE_PATH);
  const content = doc.content as unknown as {
    schema: TLStoreSnapshot["schema"];
    store: TLStoreSnapshot["store"];
  };

  if (content?.schema && content?.store) {
    editor.loadSnapshot({
      schema: content.schema,
      store: content.store,
    });
    return true;
  }
  return false;
}

export function useCanvasPersistence() {
  const editor = useEditor();
  const { connectionState } = useVFS();
  const hasLoadedRef = useRef(false);
  const [isReady, setIsReady] = useState(false);

  // Load canvas state once on mount
  useEffect(() => {
    if (connectionState !== "connected") return;
    if (hasLoadedRef.current) return;

    let cancelled = false;

    (async () => {
      try {
        const loaded = await loadCanvasSnapshot(editor);
        if (!cancelled) {
          hasLoadedRef.current = true;
          setIsReady(true);
          if (loaded) {
            console.log("[useCanvasPersistence] Canvas state loaded from VFS");
          }
        }
      } catch (err) {
        console.error("[useCanvasPersistence] Canvas state load failed", err);
        if (!cancelled) {
          hasLoadedRef.current = true;
          setIsReady(true);
        }
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [editor, connectionState]);

  // Save canvas state on changes
  useEffect(() => {
    if (connectionState !== "connected") return;

    const vfs = getVFSService();

    const unsubscribe = editor.store.listen(
      async () => {
        try {
          const snapshot = editor.getSnapshot();
          const content = {
            schema: snapshot.document.schema as unknown as JsonValue,
            store: snapshot.document.store as unknown as JsonValue,
          };

          const exists = await vfs.exists(CANVAS_STATE_PATH);

          if (!exists) {
            await vfs.writeFile(CANVAS_STATE_PATH, { content }, true);
            return;
          }

          await vfs.updateFile(CANVAS_STATE_PATH, content);
        } catch (err) {
          console.error("[useCanvasPersistence] Canvas state save failed", err);
        }
      },
      { source: "user", scope: "document" },
    );

    return unsubscribe;
  }, [editor, connectionState]);

  // Watch for external changes to canvas state (cross-tab sync)
  // biome-ignore lint/correctness/useExhaustiveDependencies: editor.loadSnapshot is stable, only depends on connectionState
  useEffect(() => {
    if (connectionState !== "connected") return;
    if (!hasLoadedRef.current) return;

    const vfs = getVFSService();
    let watchId: string | null = null;
    let isLoadingExternal = false;

    const setupWatcher = async () => {
      try {
        watchId = await vfs.watchFile(CANVAS_STATE_PATH, async (docData) => {
          if (isLoadingExternal) return;

          try {
            isLoadingExternal = true;

            const content = docData.content as unknown as {
              schema: TLStoreSnapshot["schema"];
              store: TLStoreSnapshot["store"];
            };

            if (content?.schema && content?.store) {
              editor.loadSnapshot({
                schema: content.schema,
                store: content.store,
              });
            }
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
