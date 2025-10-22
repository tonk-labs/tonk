import { useEffect } from 'preact/hooks';
import { TonkProvider, useTonk } from './context/TonkContext';
import { DialogProvider, useDialogs } from './context/DialogContext';
import { useServiceWorker } from './hooks/useServiceWorker';
import { useKeyboardNav } from './hooks/useKeyboardNav';
import { useDragDrop } from './hooks/useDragDrop';
import { LoadingScreen } from './components/screens/LoadingScreen';
import { ErrorScreen } from './components/screens/ErrorScreen';
import { PromptScreen } from './components/screens/PromptScreen';
import { BootScreen } from './components/screens/BootScreen';
import { Overlay } from './components/dialogs/Overlay';
import { ConfirmationDialog } from './components/dialogs/ConfirmationDialog';
import { ShareDialog } from './components/dialogs/ShareDialog';
import { NameDialog } from './components/dialogs/NameDialog';
import { DownloadSpinnerDialog } from './components/dialogs/DownloadSpinnerDialog';
import { ScreenState } from './types/index';

function AppContent() {
  const {
    screenState,
    showLoadingScreen,
    showBootMenu,
    showPromptScreen,
    showError,
    setAvailableApps,
  } = useTonk();
  const { dialogs } = useDialogs();
  const { queryAvailableApps, sendMessage, processTonkFile } = useServiceWorker();

  // Initialize hooks
  useKeyboardNav();
  useDragDrop();

  // Expose processTonkFile globally for cross-origin drag-drop compatibility
  useEffect(() => {
    (window as any).processTonkFile = processTonkFile;
    (window as any).showError = showError;
  }, [processTonkFile, showError]);

  // Wait for service worker to be ready
  const waitForServiceWorkerReady = async () => {
    return new Promise<void>((resolve) => {
      if (navigator.serviceWorker.controller) {
        console.log('Service worker is already controlling page, assuming ready');
        resolve();
        return;
      }

      let timeoutId: number;

      const messageHandler = (event: MessageEvent) => {
        if (event.data && event.data.type === 'ready') {
          console.log('Service worker is ready to handle requests');
          clearTimeout(timeoutId);
          navigator.serviceWorker.removeEventListener('message', messageHandler);
          resolve();
        }
      };

      navigator.serviceWorker.addEventListener('message', messageHandler);

      timeoutId = window.setTimeout(() => {
        console.log('Service worker ready check timed out, proceeding anyway');
        navigator.serviceWorker.removeEventListener('message', messageHandler);
        resolve();
      }, 10000);
    });
  };

  // Proceed to BIOS menu after loading
  const proceedToBiosMenu = async () => {
    showLoadingScreen('Loading applications...');

    try {
      const apps = await queryAvailableApps();
      setAvailableApps(apps.map((name) => ({ name })));
      showBootMenu();
    } catch (error: any) {
      console.error('Failed to query available apps:', error);
      showError(`Failed to load applications: ${error.message}`);
    }
  };

  // Initialize BIOS
  useEffect(() => {
    const initializeBios = async () => {
      const urlParams = new URLSearchParams(window.location.search);
      const bundleUrl = urlParams.get('bundle');

      await waitForServiceWorkerReady();

      if (bundleUrl) {
        // Scenario A: Load from URL
        showLoadingScreen('Loading bundle from URL...');

        try {
          const response = await sendMessage({
            type: 'initializeFromUrl',
            manifestUrl: bundleUrl,
          });

          if (response.success) {
            console.log('Bundle loaded successfully from URL');
            await proceedToBiosMenu();
          } else {
            showError(`Failed to load bundle: ${response.error}`);
          }
        } catch (error: any) {
          console.error('Error loading bundle from URL:', error);
          showError(`Error loading bundle: ${error.message}`);
        }
      } else {
        // Scenario B: No URL parameter - show prompt and wait for drag-drop
        showPromptScreen();
      }

      // Set up event listeners (do this regardless of scenario)
      // Query available apps and show boot menu
      const apps = await queryAvailableApps();
      setAvailableApps(apps.map((name) => ({ name })));
      showBootMenu();
    };

    if ('serviceWorker' in navigator) {
      if (navigator.serviceWorker.controller) {
        console.log('Service worker is already controlling the page');
        initializeBios();
      } else {
        // Register service worker
        const urlParams = new URLSearchParams(window.location.search);
        const bundleParam = urlParams.get('bundle');
        let serviceWorkerUrl = './service-worker-bundled.js';
        if (bundleParam) {
          serviceWorkerUrl += `?bundle=${encodeURIComponent(bundleParam)}`;
        }

        navigator.serviceWorker
          .register(serviceWorkerUrl, { type: 'module' })
          .catch((err) => {
            console.log('ServiceWorker registration failed: ', err);
            showError(
              'Service Worker registration failed.\n\n' +
                'Firefox does not yet support ES modules in Service Workers.\n\n' +
                'Please use Chrome or Safari to run Tonks.'
            );
          });

        console.log('Waiting for service worker to take control...');
        navigator.serviceWorker.addEventListener('controllerchange', async () => {
          console.log('Service worker now controlling the page');
          await initializeBios();
        });
      }
    } else {
      showError('Service Workers are not supported in this browser.');
    }
  }, []);

  // Determine which dialogs need overlay
  const showOverlay =
    dialogs.confirmation.isOpen ||
    dialogs.share.isOpen ||
    dialogs.name.isOpen ||
    dialogs.downloadSpinner.isOpen;

  return (
    <div class="flex flex-col max-w-[800px] h-full min-h-0 mx-auto">

      <div class="flex-1 min-h-0 flex flex-col">
        {screenState === ScreenState.LOADING && <LoadingScreen />}
        {screenState === ScreenState.BOOT && <BootScreen />}
        {screenState === ScreenState.ERROR && <ErrorScreen />}
        {screenState === ScreenState.PROMPT && <PromptScreen />}
      </div>

      <Overlay isVisible={showOverlay} />
      <ConfirmationDialog />
      <ShareDialog />
      <NameDialog />
      <DownloadSpinnerDialog />
    </div>
  );
}

export function App() {
  return (
    <TonkProvider>
      <DialogProvider>
        <AppContent />
      </DialogProvider>
    </TonkProvider>
  );
}
