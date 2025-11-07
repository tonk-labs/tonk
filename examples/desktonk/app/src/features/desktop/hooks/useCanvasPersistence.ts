import { useEffect } from 'react';
import { useEditor } from 'tldraw';
import { getVFSService } from '../../../lib/vfs-service';
import { useVFS } from '../../../hooks/useVFS';

const CANVAS_STATE_PATH = '/.state/desktop';
const SAVE_DEBOUNCE_MS = 500;

export function useCanvasPersistence() {
  const editor = useEditor();
  const { connectionState } = useVFS();

  useEffect(() => {
    // Wait for VFS to be ready
    if (connectionState !== 'connected') {
      return;
    }

    const vfs = getVFSService();
    let cancelled = false;
    let saveTimeout: ReturnType<typeof setTimeout> | null = null;

    // Load canvas state on mount
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
          // Use editor.loadSnapshot to restore the canvas state
          editor.loadSnapshot(snapshot);
          console.log('[useCanvasPersistence] Successfully loaded canvas snapshot');
        } else {
          console.log('[useCanvasPersistence] No valid snapshot to restore');
        }
      } catch (err) {
        console.error('[useCanvasPersistence] Canvas state load failed', err);
      }
    })();

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
            
            // Save the complete snapshot (tldraw will handle it properly)
            // We'll filter out file-icons when loading instead
            const toSave = snapshot;
            
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
      cancelled = true;
      unsubscribe();
      if (saveTimeout) clearTimeout(saveTimeout);
    };
  }, [editor, connectionState]);
}
