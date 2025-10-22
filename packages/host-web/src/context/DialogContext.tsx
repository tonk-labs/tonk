import { createContext } from 'preact';
import { useState, useContext } from 'preact/hooks';
import type { ComponentChildren } from 'preact';
import { DialogState, DialogContextValue } from '../types/index';

const DialogContext = createContext<DialogContextValue | undefined>(undefined);

const initialDialogState: DialogState = {
  confirmation: {
    isOpen: false,
    appName: '',
  },
  share: {
    isOpen: false,
    isLoading: false,
    qrCodeUrl: null,
    shareUrl: null,
    error: null,
  },
  name: {
    isOpen: false,
    defaultName: 'new-tonk',
    hasLoadedBundles: false,
  },
  downloadSpinner: {
    isOpen: false,
  },
};

export function DialogProvider({ children }: { children: ComponentChildren }) {
  const [dialogs, setDialogs] = useState<DialogState>(initialDialogState);

  const openConfirmationDialog = (appName: string) => {
    setDialogs((prev) => ({
      ...prev,
      confirmation: { isOpen: true, appName },
    }));
  };

  const closeConfirmationDialog = () => {
    setDialogs((prev) => ({
      ...prev,
      confirmation: { ...prev.confirmation, isOpen: false },
    }));
  };

  const openShareDialog = () => {
    setDialogs((prev) => ({
      ...prev,
      share: {
        isOpen: true,
        isLoading: false,
        qrCodeUrl: null,
        shareUrl: null,
        error: null,
      },
    }));
  };

  const closeShareDialog = () => {
    setDialogs((prev) => ({
      ...prev,
      share: { ...initialDialogState.share },
    }));
  };

  const updateShareDialog = (updates: Partial<DialogState['share']>) => {
    setDialogs((prev) => ({
      ...prev,
      share: { ...prev.share, ...updates },
    }));
  };

  const openNameDialog = (defaultName: string, hasLoadedBundles: boolean) => {
    setDialogs((prev) => ({
      ...prev,
      name: { isOpen: true, defaultName, hasLoadedBundles },
    }));
  };

  const closeNameDialog = () => {
    setDialogs((prev) => ({
      ...prev,
      name: { ...prev.name, isOpen: false },
    }));
  };

  const openDownloadSpinner = () => {
    setDialogs((prev) => ({
      ...prev,
      downloadSpinner: { isOpen: true },
    }));
  };

  const closeDownloadSpinner = () => {
    setDialogs((prev) => ({
      ...prev,
      downloadSpinner: { isOpen: false },
    }));
  };

  const value: DialogContextValue = {
    dialogs,
    openConfirmationDialog,
    closeConfirmationDialog,
    openShareDialog,
    closeShareDialog,
    updateShareDialog,
    openNameDialog,
    closeNameDialog,
    openDownloadSpinner,
    closeDownloadSpinner,
  };

  return <DialogContext.Provider value={value}>{children}</DialogContext.Provider>;
}

export function useDialogs() {
  const context = useContext(DialogContext);
  if (!context) {
    throw new Error('useDialogs must be used within DialogProvider');
  }
  return context;
}
