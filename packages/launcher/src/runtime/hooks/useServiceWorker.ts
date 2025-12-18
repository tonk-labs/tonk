import { useCallback } from 'react';
import { useTonk } from '../context/TonkContext';
import type { ServiceWorkerMessage } from '../types';

export function useServiceWorker() {
  const { showLoadingScreen, showError } = useTonk();

  // Generic message sender with promise-based response handling
  // biome-ignore lint/suspicious/noExplicitAny: Generic message handling
  const sendMessage = useCallback(<T = unknown>(message: any): Promise<T> => {
    return new Promise((resolve, reject) => {
      if (!navigator.serviceWorker.controller) {
        reject(new Error('No service worker controller available'));
        return;
      }

      const requestId = message.id || `${message.type}-${Date.now()}`;
      const messageWithId = { ...message, id: requestId };

      const messageHandler = (event: MessageEvent) => {
        if (
          event.data &&
          event.data.type === message.type &&
          event.data.id === requestId
        ) {
          navigator.serviceWorker.removeEventListener(
            'message',
            messageHandler
          );

          if (event.data.success) {
            resolve(event.data as T);
          } else {
            reject(new Error(event.data.error || 'Operation failed'));
          }
        }
      };

      navigator.serviceWorker.addEventListener('message', messageHandler);
      navigator.serviceWorker.controller.postMessage(messageWithId);

      // Timeout after 120 seconds
      setTimeout(() => {
        navigator.serviceWorker.removeEventListener('message', messageHandler);
        reject(new Error('Timeout waiting for service worker response'));
      }, 120000);
    });
  }, []);

  // Query available apps from service worker
  const queryAvailableApps = useCallback(
    async (launcherBundleId: string): Promise<string[]> => {
      try {
        if (!navigator.serviceWorker.controller) {
          console.log('No service worker controller, returning empty app list');
          return [];
        }

        // Try to get apps from manifest entrypoints
        try {
          const manifestResponse = await sendMessage<ServiceWorkerMessage>({
            type: 'getManifest',
            launcherBundleId,
          });

          if (manifestResponse.success && manifestResponse.data?.entrypoints) {
            const apps = manifestResponse.data.entrypoints;
            if (apps.length > 0) {
              console.log('Extracted apps from manifest entrypoints:', apps);
              return apps;
            }
          }
        } catch (_manifestError) {
          console.log(
            'Could not get manifest, falling back to directory listing'
          );
        }

        // Fallback: use first listed root directory
        const response = await sendMessage<ServiceWorkerMessage>({
          type: 'listDirectory',
          path: '/',
          launcherBundleId,
        });

        if (response.success && response.data) {
          const apps = response.data
            .filter((file: { type: string }) => file.type === 'directory')
            .map((file: { name: string }) => file.name);
          console.log('Extracted apps from directory listing:', apps);
          return apps;
        }

        return [];
      } catch (error: unknown) {
        // If VFS not initialized, return empty array instead of failing
        const errorMessage =
          error instanceof Error ? error.message : String(error);
        if (errorMessage?.includes('not initialized')) {
          console.log('VFS not initialized yet, returning empty app list');
          return [];
        }
        console.error('Failed to query apps:', error);
        throw error;
      }
    },
    [sendMessage]
  );

  // Confirm boot and redirect to app
  // New URL structure: /space/<launcherBundleId>/<appSlug>/
  const confirmBoot = useCallback(
    async (appSlug: string, launcherBundleId: string) => {
      console.log('Booting application:', { appSlug, launcherBundleId });
      showLoadingScreen(`Loading ${appSlug}...`);

      if (navigator.serviceWorker.controller) {
        navigator.serviceWorker.controller.postMessage({
          type: 'setAppSlug',
          slug: appSlug,
          launcherBundleId,
        });

        localStorage.setItem('appSlug', appSlug);
        localStorage.setItem('launcherBundleId', launcherBundleId);

        // Navigate to new URL structure: /space/<launcherBundleId>/<appSlug>/
        window.location.href = `/space/${launcherBundleId}/${appSlug}/`;
      } else {
        showError('Service worker not ready. Please refresh the page.');
      }
    },
    [showLoadingScreen, showError]
  );

  return {
    sendMessage,
    queryAvailableApps,
    confirmBoot,
  };
}
