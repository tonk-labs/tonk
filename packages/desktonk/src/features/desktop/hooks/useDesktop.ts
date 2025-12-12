import { useEffect, useState } from "react";
import {
  type DesktopState,
  getDesktopService,
} from "../services/DesktopService";

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
  const [state, setState] = useState<DesktopState>(() =>
    getDesktopService().getState(),
  );
  // Counter to trigger re-subscription when bundle reloads
  const [subscriptionKey, setSubscriptionKey] = useState(0);

  // Listen for bundle reload to re-subscribe to new service instance
  useEffect(() => {
    const handler = (event: MessageEvent) => {
      if (event.data?.type === "tonk:bundleReloaded") {
        console.log(
          "[useDesktop] Bundle reloaded, will re-subscribe to new service",
        );
        // Reset state to loading while new service initializes
        setState({ files: [], positions: new Map(), isLoading: true });
        // Trigger re-subscription
        setSubscriptionKey((k) => k + 1);
      }
    };

    window.addEventListener("message", handler);
    return () => window.removeEventListener("message", handler);
  }, []);

  useEffect(() => {
    const service = getDesktopService();

    // Subscribe to service updates
    const unsubscribe = service.subscribe((newState) => {
      setState(newState);
    });

    return unsubscribe;
  }, [subscriptionKey]);

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
    setPosition: (fileId: string, x: number, y: number) =>
      service.setPosition(fileId, x, y),

    getPosition: (fileId: string) => service.getPosition(fileId),

    onFileAdded: (path: string) => service.onFileAdded(path),

    onFileDeleted: (fileId: string) => service.onFileDeleted(fileId),
  };
}
