import { useEffect, useRef, useState } from 'react';
import { useEditor } from 'tldraw';
import { getVFSService } from '../../../lib/vfs-service';
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
          return;
        }
        
        const json = await vfs.readBytesAsString(CANVAS_STATE_PATH);
        console.log('[useCanvasPersistence] Raw JSON length:', json.length);
        
        const snapshot = JSON.parse(json);
        console.log('[useCanvasPersistence] Snapshot keys:', Object.keys(snapshot));
        
        if (!cancelled && snapshot.document) {
          // Filter out file-icon shapes from the snapshot before loading
          // File icons are dynamically managed by useDesktopSync, not persisted in canvas state
          const filteredSnapshot = {
            ...snapshot,
            document: {
              ...snapshot.document,
              store: Object.fromEntries(
                Object.entries(snapshot.document.store).filter(([_key, value]: [string, any]) => 
                  !(value?.type === 'shape' && value?.typeName === 'file-icon')
                )
              )
            }
          };
          
          // Use editor.loadSnapshot to restore the canvas state
          console.log('[useCanvasPersistence] About to load snapshot...');
          editor.loadSnapshot(filteredSnapshot);
          hasLoadedRef.current = true;
          setIsReady(true);
          console.log('[useCanvasPersistence] Successfully loaded canvas snapshot (filtered file-icons)');
          console.log('[useCanvasPersistence] Shapes after load:', editor.getCurrentPageShapes().length);
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
            console.log('[useCanvasPersistence] Snapshot keys:', Object.keys(snapshot));
            
            // Get all shapes to count them
            const allShapes = editor.getCurrentPageShapes();
            const fileIconShapes = allShapes.filter(s => s.type === 'file-icon');
            const otherShapes = allShapes.filter(s => s.type !== 'file-icon');
            
            console.log('[useCanvasPersistence] Total shapes:', allShapes.length);
            console.log('[useCanvasPersistence] File-icon shapes:', fileIconShapes.length);
            console.log('[useCanvasPersistence] Other shapes to save:', otherShapes.length);
            console.log('[useCanvasPersistence] Shape types being saved:', 
              otherShapes.map(s => s.type).slice(0, 10)
            );
            
            // Filter out file-icon shapes when saving (they're managed by VFS, not canvas state)
            const toSave = {
              ...snapshot,
              document: {
                ...snapshot.document,
                store: Object.fromEntries(
                  Object.entries(snapshot.document.store).filter(([_key, value]: [string, any]) => 
                    !(value?.type === 'shape' && value?.typeName === 'file-icon')
                  )
                )
              }
            };
            
            // Check if file exists to determine if we should create or update
            const exists = await vfs.exists(CANVAS_STATE_PATH);
            await vfs.writeStringAsBytes(CANVAS_STATE_PATH, JSON.stringify(toSave), !exists);
            console.log('[useCanvasPersistence] ✅ Saved canvas state to', CANVAS_STATE_PATH);
          } catch (err) {
            console.error('[useCanvasPersistence] ❌ Canvas state save failed', err);
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

  return { isReady };
}
