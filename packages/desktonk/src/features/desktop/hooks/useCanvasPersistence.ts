import { useEffect, useRef, useState } from 'react';
import { type TLStoreSnapshot, useEditor } from 'tldraw';
import { getVFSService, type JsonValue } from '@/vfs-client';
import { useVFS } from '../../../hooks/useVFS';

const CANVAS_STATE_PATH = '/.state/desktop';

export function useCanvasPersistence() {
  const editor = useEditor();
  const { connectionState } = useVFS();
  const hasLoadedRef = useRef(false);
  const [isReady, setIsReady] = useState(false);

  // Load canvas state once on mount
  useEffect(() => {
    // Wait for VFS to be ready
    if (connectionState !== 'connected') {
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
          schema: TLStoreSnapshot['schema'];
          store: TLStoreSnapshot['store'];
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
        console.error('[useCanvasPersistence] Canvas state load failed', err);
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
    if (connectionState !== 'connected') {
      return;
    }

    const vfs = getVFSService();

    // Save canvas state on changes
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
            // First save: create full document
            await vfs.writeFile(CANVAS_STATE_PATH, { content }, true);
            return;
          }

          await vfs.updateFile(CANVAS_STATE_PATH, content);
        } catch (err) {
          console.error(
            '[useCanvasPersistence] ❌ Canvas state save failed',
            err
          );
        }
      },
      { source: 'user', scope: 'document' }
    );

    return () => {
      unsubscribe();
    };
  }, [editor, connectionState]);

  // Watch for external changes to canvas state (cross-tab sync)
  // biome-ignore lint/correctness/useExhaustiveDependencies: editor.loadSnapshot is stable, only depends on connectionState
  useEffect(() => {
    if (connectionState !== 'connected') {
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
        watchId = await vfs.watchFile(CANVAS_STATE_PATH, async docData => {
          // Prevent loading during external update to avoid loops
          if (isLoadingExternal) return;

          try {
            isLoadingExternal = true;

            const content = docData.content as unknown as {
              schema: TLStoreSnapshot['schema'];
              store: TLStoreSnapshot['store'];
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
              '[useCanvasPersistence] Error loading external canvas state:',
              err
            );
          } finally {
            isLoadingExternal = false;
          }
        });
      } catch (err) {
        console.error(
          '[useCanvasPersistence] Error setting up file watcher:',
          err
        );
      }
    };

    setupWatcher();

    return () => {
      if (watchId) {
        vfs.unwatchFile(watchId).catch(err => {
          console.warn('[useCanvasPersistence] Error unwatching file:', err);
        });
      }
    };
  }, [connectionState]);

  return { isReady };
}
