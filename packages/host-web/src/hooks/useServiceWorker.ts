import { useCallback } from 'preact/hooks';
import { useTonk } from '../context/TonkContext';
import { useDialogs } from '../context/DialogContext';
import type { ServiceWorkerMessage } from '../types/index';
import { ALLOWED_ORIGINS } from '../constants';

const sendSuccessToParent = (fileName: string) => {
  if (window.parent !== window) {
    ALLOWED_ORIGINS.forEach(origin => {
      try {
        window.parent.postMessage(
          {
            type: 'tonk:dropResponse',
            status: 'success',
            fileName,
          },
          origin
        );
      } catch (err) {
        console.log(err);
        // Ignore errors for wrong origins
      }
    });
  }
};

export function useServiceWorker() {
  const {
    showLoadingScreen,
    showError,
    setAvailableApps,
    showBootMenu,
    showSplashScreen,
  } = useTonk();
  const { updateShareDialog, closeDownloadSpinner } = useDialogs();

  // Generic message sender with promise-based response handling
  const sendMessage = useCallback(<T = any>(message: any): Promise<T> => {
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
  const queryAvailableApps = useCallback(async (): Promise<string[]> => {
    try {
      if (!navigator.serviceWorker.controller) {
        console.log('No service worker controller, returning empty app list');
        return [];
      }

      const response = await sendMessage<ServiceWorkerMessage>({
        type: 'listDirectory',
        path: '/',
      });

      if (response.success && response.data) {
        const apps = response.data
          .filter((file: any) => file.type === 'directory')
          .map((file: any) => file.name);
        console.log('Extracted apps from directory listing:', apps);
        return apps;
      }

      return [];
    } catch (error: any) {
      // If VFS not initialized, return empty array instead of failing
      if (error.message && error.message.includes('not initialized')) {
        console.log('VFS not initialized yet, returning empty app list');
        return [];
      }
      console.error('Failed to query apps:', error);
      throw error;
    }
  }, [sendMessage]);

  // Confirm boot and redirect to app
  const confirmBoot = useCallback(
    async (appSlug: string) => {
      console.log('Booting application:', appSlug);
      showLoadingScreen(`Loading ${appSlug}...`);

      if (navigator.serviceWorker.controller) {
        navigator.serviceWorker.controller.postMessage({
          type: 'setAppSlug',
          slug: appSlug,
        });

        localStorage.setItem('appSlug', appSlug);
        await new Promise(resolve => setTimeout(resolve, 500));
        window.location.href = `/${appSlug}/`;
      } else {
        showError('Service worker not ready. Please refresh the page.');
      }
    },
    [showLoadingScreen, showError]
  );

  // Get server URL from service worker
  const getServerUrl = useCallback(async (): Promise<string> => {
    const response = await sendMessage<ServiceWorkerMessage>({
      type: 'getServerUrl',
    });

    if (response.success && response.data) {
      return response.data;
    }
    throw new Error('Failed to get server URL');
  }, [sendMessage]);

  // Download tonk bundle
  const downloadTonk = useCallback(async () => {
    console.log('Downloading tonk bundle...');

    try {
      const response = await sendMessage<ServiceWorkerMessage>({
        type: 'toBytes',
      });

      if (response.success && response.data) {
        const blob = new Blob([response.data], {
          type: 'application/octet-stream',
        });
        const url = URL.createObjectURL(blob);
        const a = document.createElement('a');
        a.href = url;
        a.download = `${response.rootId}.tonk`;
        document.body.appendChild(a);
        a.click();
        document.body.removeChild(a);
        URL.revokeObjectURL(url);
      } else {
        showError(`Failed to download bundle: ${response.error}`);
      }
    } catch (error: any) {
      console.error('Error downloading tonk bundle:', error);
      showError(`Error downloading bundle: ${error.message}`);
    }
  }, [sendMessage, showError]);

  // Share as URL with QR code
  const shareAsUrl = useCallback(async (): Promise<{
    shareUrl: string;
    qrCodeUrl: string;
  }> => {
    updateShareDialog({ isLoading: true, error: null });

    try {
      const serverUrl = await getServerUrl();

      // Get bundle bytes
      const bytesResponse = await sendMessage<ServiceWorkerMessage>({
        type: 'toBytes',
      });

      if (!bytesResponse.success || !bytesResponse.data) {
        throw new Error(bytesResponse.error || 'Failed to get bundle bytes');
      }

      const bundleBytes = bytesResponse.data;

      // Upload to server
      const uploadResponse = await fetch(`${serverUrl}/api/bundles`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/octet-stream' },
        body: bundleBytes,
      });

      if (!uploadResponse.ok) {
        const errorData = await uploadResponse.json();
        throw new Error(errorData.error || 'Failed to upload bundle');
      }

      const uploadResult = await uploadResponse.json();
      const bundleId = uploadResult.id;

      const manifestUrl = `${serverUrl}/api/bundles/${bundleId}`;
      const shareUrl = `${window.location.origin}/?bundle=${encodeURIComponent(manifestUrl)}`;

      // Generate QR code
      const QRCode = await import('https://esm.sh/qrcode@1.5.3');
      const qrDataUrl = await QRCode.toDataURL(shareUrl, {
        width: 200,
        margin: 2,
        color: { dark: '#000000', light: '#ffffff' },
      });

      updateShareDialog({
        isLoading: false,
        qrCodeUrl: qrDataUrl,
        shareUrl,
      });

      return { shareUrl, qrCodeUrl: qrDataUrl };
    } catch (error: any) {
      console.error('Failed to generate share URL:', error);
      updateShareDialog({
        isLoading: false,
        error: error.message || 'Failed to upload bundle',
      });
      throw error;
    }
  }, [sendMessage, getServerUrl, updateShareDialog]);

  // Create new tonk
  const createNewTonk = useCallback(
    async (tonkName: string, hasLoadedBundles: boolean) => {
      console.log('Creating new tonk with name:', tonkName);
      showLoadingScreen('Creating new tonk...');

      try {
        // If no bundles loaded, fetch blank tonk first
        if (!hasLoadedBundles) {
          const serverUrl = await getServerUrl();
          const response = await fetch(`${serverUrl}/api/blank-tonk`);

          if (!response.ok) {
            throw new Error('Failed to fetch blank tonk template');
          }

          const blankBytes = new Uint8Array(await response.arrayBuffer());

          // Load the blank tonk
          await sendMessage({
            type: 'loadBundle',
            bundleBytes: blankBytes.buffer.slice(
              blankBytes.byteOffset,
              blankBytes.byteOffset + blankBytes.byteLength
            ),
          });
        }

        // Fork the bundle
        const forkResponse = await sendMessage<ServiceWorkerMessage>({
          type: 'forkToBytes',
        });

        if (!forkResponse.success || !forkResponse.data) {
          throw new Error(forkResponse.error || 'Failed to fork tonk');
        }

        const bundleBytes = forkResponse.data;

        // Load the forked bundle
        await sendMessage({
          type: 'loadBundle',
          bundleBytes: bundleBytes.buffer.slice(
            bundleBytes.byteOffset,
            bundleBytes.byteOffset + bundleBytes.byteLength
          ),
        });

        // Rename the app directory
        const oldName = hasLoadedBundles ? 'latergram' : 'latergram';
        await sendMessage({
          type: 'rename',
          oldPath: `/app/${oldName}`,
          newPath: `/app/${tonkName}`,
        });

        // Refresh app list
        const apps = await queryAvailableApps();
        setAvailableApps(apps.map(name => ({ name })));
        showBootMenu();
      } catch (error: any) {
        console.error('Error creating new tonk:', error);
        showError(`Error creating new tonk: ${error.message}`);
      } finally {
        closeDownloadSpinner();
      }
    },
    [
      sendMessage,
      getServerUrl,
      queryAvailableApps,
      setAvailableApps,
      showBootMenu,
      showLoadingScreen,
      showError,
      closeDownloadSpinner,
    ]
  );

  // Process tonk file
  const processTonkFile = useCallback(
    async (file: File) => {
      console.log('Processing .tonk file:', file.name);
      showLoadingScreen('Loading bundle from file...');

      try {
        const arrayBuffer = await file.arrayBuffer();
        console.log('File read as ArrayBuffer, size:', arrayBuffer.byteLength);

        const response = await sendMessage<ServiceWorkerMessage>({
          type: 'loadBundle',
          bundleBytes: arrayBuffer,
        });

        if (response.success) {
          console.log('Bundle loaded successfully:', file.name);

          // Refresh app list
          const apps = await queryAvailableApps();
          setAvailableApps(apps.map(name => ({ name })));

          if (apps.length > 0) {
            // Send success to parent BEFORE redirecting
            sendSuccessToParent(file.name);

            // Show splash screen for 500ms, then auto-boot first app
            showSplashScreen();
            await new Promise(resolve => setTimeout(resolve, 500));
            await confirmBoot(apps[0]);
          } else {
            // No apps found in bundle
            showError('No applications found in bundle');
          }
        } else {
          showError(`Failed to load bundle: ${response.error}`);
        }
      } catch (error: any) {
        console.error('Error processing tonk file:', error);
        showError(`Error processing ${file.name}: ${error.message}`);
      }
    },
    [
      sendMessage,
      queryAvailableApps,
      setAvailableApps,
      confirmBoot,
      showSplashScreen,
      showLoadingScreen,
      showError,
    ]
  );

  return {
    sendMessage,
    queryAvailableApps,
    confirmBoot,
    downloadTonk,
    createNewTonk,
    getServerUrl,
    shareAsUrl,
    processTonkFile,
  };
}
