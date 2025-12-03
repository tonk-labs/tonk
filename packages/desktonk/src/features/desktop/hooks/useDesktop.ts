import { useState, useEffect } from 'react';
import { getDesktopService, type DesktopState } from '../services/DesktopService';

/**
 * React hook to consume desktop state from DesktopService.
 *
 * Replaces: useDesktopSync, usePositionSync, useDeletionSync, and all coordination hooks.
 *
 * @example
 * ```tsx
 * function Desktop() {
 *   const { files, positions, isLoading } = useDesktop();
 *   // Use state to render UI
 * }
 * ```
 */
export function useDesktop(): DesktopState {
  const [state, setState] = useState<DesktopState>(() => getDesktopService().getState());

  useEffect(() => {
    const service = getDesktopService();

    // Subscribe to service updates
    const unsubscribe = service.subscribe((newState) => {
      setState(newState);
    });

    return unsubscribe;
  }, []);

  return state;
}

/**
 * Hook to get service methods for updating positions.
 *
 * @example
 * ```tsx
 * function FileIcon({ fileId }) {
 *   const { setPosition } = useDesktopActions();
 *
 *   const handleDragEnd = (x: number, y: number) => {
 *     setPosition(fileId, x, y);
 *   };
 * }
 * ```
 */
export function useDesktopActions() {
  const service = getDesktopService();

  return {
    setPosition: (fileId: string, x: number, y: number) => service.setPosition(fileId, x, y),

    getPosition: (fileId: string) => service.getPosition(fileId),

    onFileAdded: (path: string) => service.onFileAdded(path),

    onFileDeleted: (fileId: string) => service.onFileDeleted(fileId),
  };
}
