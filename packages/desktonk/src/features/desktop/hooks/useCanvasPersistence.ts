import { getVFSService } from '@tonk/host-web/client';
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

        const json = await vfs.readBytesAsString(CANVAS_STATE_PATH);
        console.log('[useCanvasPersistence] Raw JSON length:', json.length);

        const snapshot = JSON.parse(json);
        console.log(
          '[useCanvasPersistence] Snapshot keys:',
          Object.keys(snapshot)
        );

        if (!cancelled && snapshot.document) {
          // Filter out file-icon shapes from the snapshot before loading
          // File icons are dynamically managed by DesktopService, not persisted in canvas state
          const filteredSnapshot = {
            ...snapshot,
            document: {
              ...snapshot.document,
              store: Object.fromEntries(
                Object.entries(snapshot.document.store).filter(
                  // biome-ignore lint/suspicious/noExplicitAny: tldraw snapshot value type
                  ([_key, value]: [string, any]) =>
                    !(
                      value?.type === 'shape' && value?.typeName === 'file-icon'
                    )
                )
              ),
            },
          };

          // Use editor.loadSnapshot to restore the canvas state
          console.log('[useCanvasPersistence] About to load snapshot...');
          editor.loadSnapshot(filteredSnapshot);
          hasLoadedRef.current = true;
          setIsReady(true);
          console.log(
            '[useCanvasPersistence] Successfully loaded canvas snapshot (filtered file-icons)'
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
            // Get snapshot returns { document, session }
            const snapshot = editor.getSnapshot();
            console.log(
              '[useCanvasPersistence] Snapshot keys:',
              Object.keys(snapshot)
            );

            // Get all shapes to count them
            const allShapes = editor.getCurrentPageShapes();
            const fileIconShapes = allShapes.filter(
              s => s.type === 'file-icon'
            );
            const otherShapes = allShapes.filter(s => s.type !== 'file-icon');

            console.log(
              '[useCanvasPersistence] Total shapes:',
              allShapes.length
            );
            console.log(
              '[useCanvasPersistence] File-icon shapes:',
              fileIconShapes.length
            );
            console.log(
              '[useCanvasPersistence] Other shapes to save:',
              otherShapes.length
            );
            console.log(
              '[useCanvasPersistence] Shape types being saved:',
              otherShapes.map(s => s.type).slice(0, 10)
            );

            // Filter out file-icon shapes when saving (they're managed by VFS, not canvas state)
            // Also exclude session state (camera position) to prevent it from being restored
            const toSave = {
              ...snapshot,
              document: {
                ...snapshot.document,
                store: Object.fromEntries(
                  Object.entries(snapshot.document.store).filter(
                    // biome-ignore lint/suspicious/noExplicitAny: tldraw snapshot value type
                    ([_key, value]: [string, any]) =>
                      !(
                        value?.type === 'shape' &&
                        value?.typeName === 'file-icon'
                      )
                  )
                ),
              },
              session: undefined, // Don't save camera state
            };

            // Debug: Check the size of what we're trying to save
            const jsonString = JSON.stringify(toSave);
            console.log('[DEBUG] JSON size:', jsonString.length, 'bytes');

            // Debug: What's in the store?
            const storeEntries = Object.entries(toSave.document.store);
            console.log('[DEBUG] Store entries count:', storeEntries.length);
            const byType = storeEntries.reduce(
              // biome-ignore lint/suspicious/noExplicitAny: tldraw snapshot value type
              (acc, [_key, value]: [string, any]) => {
                const type = value?.typeName || value?.type || 'unknown';
                acc[type] = (acc[type] || 0) + 1;
                return acc;
              },
              {} as Record<string, number>
            );
            console.log('[DEBUG] Store entries by type:', byType);

            // Find largest entries
            const entrySizes = storeEntries
              .map(([key, value]) => ({
                key,
                // biome-ignore lint/suspicious/noExplicitAny: Value type from tldraw snapshot is not strictly typed
                type: (value as any)?.typeName || (value as any)?.type,
                size: JSON.stringify(value).length,
              }))
              .sort((a, b) => b.size - a.size)
              .slice(0, 5);
            console.log('[DEBUG] Top 5 largest entries:', entrySizes);

            // Check if file exists to determine if we should create or update
            const exists = await vfs.exists(CANVAS_STATE_PATH);
            await vfs.writeStringAsBytes(
              CANVAS_STATE_PATH,
              JSON.stringify(toSave),
              !exists
            );
            console.log(
              '[useCanvasPersistence] ✅ Saved canvas state to',
              CANVAS_STATE_PATH
            );
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
        watchId = await vfs.watchFile(CANVAS_STATE_PATH, async content => {
          // Prevent loading during external update to avoid loops
          if (isLoadingExternal) return;

          try {
            isLoadingExternal = true;
            console.log(
              '[useCanvasPersistence] External canvas state change detected'
            );

            const json = (content.content as { text?: string })?.text || '';
            if (!json) {
              console.log('[useCanvasPersistence] Empty external canvas state');
              return;
            }

            const snapshot = JSON.parse(json);

            // Filter out file-icon shapes before loading
            const filteredSnapshot = {
              ...snapshot,
              document: {
                ...snapshot.document,
                store: Object.fromEntries(
                  Object.entries(snapshot.document.store).filter(
                    // biome-ignore lint/suspicious/noExplicitAny: Value type from tldraw snapshot is not strictly typed
                    ([_key, value]: [string, any]) =>
                      !(
                        value?.type === 'shape' &&
                        value?.typeName === 'file-icon'
                      )
                  )
                ),
              },
            };

            // Load the updated snapshot from other tab
            editor.loadSnapshot(filteredSnapshot);
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
  }, [connectionState]); // Removed 'editor' and 'hasLoadedRef' as they are stable/refs

  return { isReady };
}
