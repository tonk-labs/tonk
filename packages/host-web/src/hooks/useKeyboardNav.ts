import { useEffect } from 'preact/hooks';
import { useTonk } from '../context/TonkContext';
import { useDialogs } from '../context/DialogContext';
import { useServiceWorker } from './useServiceWorker';

export function useKeyboardNav() {
  const { availableApps, selectedAppIndex, setSelectedAppIndex } = useTonk();
  const { dialogs, openConfirmationDialog, closeConfirmationDialog } = useDialogs();
  const { confirmBoot } = useServiceWorker();

  useEffect(() => {
    const handleKeyDown = async (e: KeyboardEvent) => {
      // Handle confirmation dialog
      if (dialogs.confirmation.isOpen) {
        if (e.key === 'Enter' || e.key === 'y' || e.key === 'Y') {
          closeConfirmationDialog();
          const app = availableApps[selectedAppIndex];
          const appSlug = typeof app === 'string' ? app : app.slug || app.name;
          await confirmBoot(appSlug);
        } else if (e.key === 'Escape' || e.key === 'n' || e.key === 'N') {
          closeConfirmationDialog();
        }
        return;
      }

      // Handle app list navigation
      if (availableApps.length === 0) return;

      switch (e.key) {
        case 'ArrowUp':
          e.preventDefault();
          setSelectedAppIndex(Math.max(0, selectedAppIndex - 1));
          break;
        case 'ArrowDown':
          e.preventDefault();
          setSelectedAppIndex(Math.min(availableApps.length - 1, selectedAppIndex + 1));
          break;
        case 'Enter':
          e.preventDefault();
          if (availableApps.length > 0) {
            const app = availableApps[selectedAppIndex];
            const appName = typeof app === 'string' ? app : app.name;
            openConfirmationDialog(appName);
          }
          break;
      }
    };

    document.addEventListener('keydown', handleKeyDown);
    return () => document.removeEventListener('keydown', handleKeyDown);
  }, [
    availableApps,
    selectedAppIndex,
    setSelectedAppIndex,
    dialogs.confirmation.isOpen,
    openConfirmationDialog,
    closeConfirmationDialog,
    confirmBoot,
  ]);
}
