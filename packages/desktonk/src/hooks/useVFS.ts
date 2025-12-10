import { useEffect, useState } from 'react';
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

  return {
    vfs,
    connectionState,
  };
}
