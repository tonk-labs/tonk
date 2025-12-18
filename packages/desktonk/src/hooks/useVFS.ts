import { useCallback, useEffect, useState } from 'react';
import { getVFSService } from '@/vfs-client';

type ConnectionState =
  | 'disconnected'
  | 'connecting'
  | 'open'
  | 'connected'
  | 'reconnecting';

export function useVFS() {
  const [vfs] = useState(() => getVFSService());
  const [connectionState, setConnectionState] =
    useState<ConnectionState>('disconnected');

  useEffect(() => {
    return vfs.onConnectionStateChange(state => {
      console.log('[useVFS] Connection state changed:', state);
      setConnectionState(state);
    });
  }, [vfs]);

  // TODO: Currently unused. Re-enable when bundle lifecycle management is needed.
  // Reset connection when TonkCore changes (e.g., switching between tonks)
  const resetConnection = useCallback(async () => {
    await vfs.reset();
  }, [vfs]);

  return {
    vfs,
    connectionState,
    resetConnection,
  };
}
