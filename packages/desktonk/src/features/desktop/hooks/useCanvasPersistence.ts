import { getVFSService } from '@/vfs-client';
import { useEffect, useRef, useState } from 'react';
import { useEditor } from 'tldraw';
import { useVFS } from '../../../hooks/useVFS';

const CANVAS_STATE_PATH = '/.state/desktop';
const SAVE_DEBOUNCE_MS = 500;

export function useCanvasPersistence() {
  const editor = useEditor();
  const { connectionState } = useVFS();
  const hasLoadedRef = useRef(false);
  const [isReady, setIsReady] = useState(false);

  // Track previous store state for diffing
  const previousStoreRef = useRef<Record<string, unknown>>({});

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
        console.log('[useCanvasPersistence] Load check - file exists:', exists);

        if (!exists) {
          console.log('[useCanvasPersistence] No saved canvas state found');
          hasLoadedRef.current = true;
          setIsReady(true);
          return;
        }

        const doc = await vfs.readFile(CANVAS_STATE_PATH);
        const content = doc.content as {
          schema: unknown;
          store: Record<string, unknown>;
        };

        console.log(
          '[useCanvasPersistence] Loaded content keys:',
          Object.keys(content)
        );

        if (!cancelled && content.schema && content.store) {
          // Reconstruct tldraw snapshot format
          const snapshot = {
            document: {
              schema: content.schema,
              store: content.store,
            },
          };

          // Use editor.loadSnapshot to restore the canvas state
          console.log('[useCanvasPersistence] About to load snapshot...');
          editor.loadSnapshot(snapshot);

          // Initialize tracking ref
          previousStoreRef.current = { ...content.store };

          hasLoadedRef.current = true;
          setIsReady(true);
          console.log(
            '[useCanvasPersistence] Successfully loaded canvas snapshot'
          );
          console.log(
            '[useCanvasPersistence] Shapes after load:',
            editor.getCurrentPageShapes().length
          );
        } else {
          hasLoadedRef.current = true;
          setIsReady(true);
          console.log('[useCanvasPersistence] No valid snapshot to restore');
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
    let saveTimeout: ReturnType<typeof setTimeout> | null = null;

    // Save canvas state on changes (debounced)
    const unsubscribe = editor.store.listen(
      () => {
        if (saveTimeout) clearTimeout(saveTimeout);

        saveTimeout = setTimeout(async () => {
          try {
            const snapshot = editor.getSnapshot();
            const store = snapshot.document.store as Record<string, unknown>;

            const exists = await vfs.exists(CANVAS_STATE_PATH);

            if (!exists) {
              // First save: create full document
              console.log(
                '[useCanvasPersistence] Creating new canvas state file'
              );
              await vfs.writeFile(
                CANVAS_STATE_PATH,
                {
                  content: {
                    schema: snapshot.document.schema,
                    store: store,
                  },
                },
                true
              );
              previousStoreRef.current = { ...store };
              console.log(
                '[useCanvasPersistence] ✅ Created canvas state with',
                Object.keys(store).length,
                'entries'
              );
              return;
            }

            // Incremental save: patch only changed entries
            const patches: Promise<boolean>[] = [];

            // Check for new/modified entries
            for (const [key, value] of Object.entries(store)) {
              const prev = previousStoreRef.current[key];
              if (!prev || JSON.stringify(prev) !== JSON.stringify(value)) {
                patches.push(
                  vfs.patchFile(CANVAS_STATE_PATH, ['store', key], value)
                );
              }
            }

            // Check for deleted entries
            for (const key of Object.keys(previousStoreRef.current)) {
              if (!(key in store)) {
                patches.push(
                  vfs.patchFile(CANVAS_STATE_PATH, ['store', key], null)
                );
              }
            }

            if (patches.length > 0) {
              await Promise.all(patches);
              console.log(
                `[useCanvasPersistence] ✅ Patched ${patches.length} entries`
              );
            } else {
              console.log('[useCanvasPersistence] No changes to save');
            }

            previousStoreRef.current = { ...store };
          } catch (err) {
            console.error(
              '[useCanvasPersistence] ❌ Canvas state save failed',
              err
            );
          }
        }, SAVE_DEBOUNCE_MS);
      },
      { source: 'user', scope: 'document' }
    );

    return () => {
      unsubscribe();
      if (saveTimeout) clearTimeout(saveTimeout);
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
            console.log(
              '[useCanvasPersistence] External canvas state change detected'
            );

            const content = docData.content as {
              schema: unknown;
              store: Record<string, unknown>;
            };

            if (!content || !content.schema || !content.store) {
              console.log('[useCanvasPersistence] Empty external canvas state');
              return;
            }

            // Reconstruct tldraw snapshot format
            const snapshot = {
              document: {
                schema: content.schema,
                store: content.store,
              },
            };

            // Load the updated snapshot from other tab
            editor.loadSnapshot(snapshot);

            // Update our tracking ref
            previousStoreRef.current = { ...content.store };

            console.log(
              '[useCanvasPersistence] ✅ Loaded external canvas state update'
            );
          } catch (err) {
            console.error(
              '[useCanvasPersistence] Error loading external canvas state:',
              err
            );
          } finally {
            isLoadingExternal = false;
          }
        });

        console.log(
          '[useCanvasPersistence] File watcher set up for',
          CANVAS_STATE_PATH
        );
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
