import { useEffect, useState } from "react";
import {
  type DesktopState,
  getDesktopService,
  softRefreshDesktopService,
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

  // Listen for bundle reload/activate to handle state appropriately
  useEffect(() => {
    const handler = (event: MessageEvent) => {
      if (event.data?.type === "tonk:bundleReloaded") {
        // Full reset - new TonkCore instance loaded, need to re-subscribe to new service
        console.log(
          "[useDesktop] Bundle reloaded, will re-subscribe to new service",
        );
        setState({ files: [], positions: new Map(), isLoading: true });
        setSubscriptionKey((k) => k + 1);
      } else if (event.data?.type === "tonk:activate") {
        // Soft refresh - just switching back to this bundle in the iframe pool
        // Don't destroy watchers, just reload data from VFS to ensure we have latest state
        console.log("[useDesktop] Bundle activated, triggering soft refresh");
        softRefreshDesktopService();
      }
    };

    window.addEventListener("message", handler);
    return () => window.removeEventListener("message", handler);
  }, []);

  // biome-ignore lint/correctness/useExhaustiveDependencies: subscriptionKey is intentionally used to trigger re-subscription on bundle reload
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
