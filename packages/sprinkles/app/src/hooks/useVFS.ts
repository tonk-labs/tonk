import { useState, useEffect, useCallback } from 'react';
import { getVFSService } from '../lib/vfs-service';

type ConnectionState =
  | 'disconnected'
  | 'connecting'
  | 'open'
  | 'connected'
  | 'reconnecting';

export function useVFS() {
  const [vfs] = useState(() => getVFSService());
  const [connectionState, setConnectionState] = useState<ConnectionState>('disconnected');
  const [isInitializing, setIsInitializing] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    return vfs.onConnectionStateChange((state) => {
      console.log('[useVFS] Connection state changed:', state);
      setConnectionState(state);
    });
  }, [vfs]);

  const initialize = useCallback(async (manifestUrl: string, wsUrl: string) => {
    setIsInitializing(true);
    setError(null);
    try {
      console.log('[useVFS] Initializing VFS with:', { manifestUrl, wsUrl });
      await vfs.initialize(manifestUrl, wsUrl);
      console.log('[useVFS] VFS initialized successfully');
    } catch (err) {
      const errorMessage = err instanceof Error ? err.message : String(err);
      console.error('[useVFS] Failed to initialize VFS:', errorMessage);
      setError(errorMessage);
      throw err;
    } finally {
      setIsInitializing(false);
    }
  }, [vfs]);

  const isReady = vfs.isInitialized() && connectionState === 'connected';

  return {
    vfs,
    connectionState,
    initialize,
    isReady,
    isInitializing,
    error,
  };
}
