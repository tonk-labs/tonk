import { createContext } from 'preact';
import { useState, useContext } from 'preact/hooks';
import type { ComponentChildren } from 'preact';
import { ScreenState, App, TonkContextValue } from '../types/index';

const TonkContext = createContext<TonkContextValue | undefined>(undefined);

export function TonkProvider({ children }: { children: ComponentChildren }) {
  const [screenState, setScreenState] = useState<ScreenState>(ScreenState.LOADING);
  const [loadingMessage, setLoadingMessage] = useState('Initializing TONK runtime...');
  const [errorMessage, setErrorMessage] = useState('');
  const [availableApps, setAvailableApps] = useState<App[]>([]);
  const [selectedAppIndex, setSelectedAppIndex] = useState(0);

  const showError = (message: string) => {
    setErrorMessage(message);
    setScreenState(ScreenState.ERROR);
  };

  const showLoadingScreen = (message = 'Initializing TONK runtime...') => {
    setLoadingMessage(message);
    setScreenState(ScreenState.LOADING);
  };

  const showBootMenu = () => {
    setScreenState(ScreenState.BOOT);
  };

  const showPromptScreen = () => {
    setScreenState(ScreenState.PROMPT);
  };

  const showSplashScreen = () => {
    setScreenState(ScreenState.SPLASH);
  };

  const value: TonkContextValue = {
    screenState,
    setScreenState,
    loadingMessage,
    setLoadingMessage,
    errorMessage,
    setErrorMessage,
    availableApps,
    setAvailableApps,
    selectedAppIndex,
    setSelectedAppIndex,
    showError,
    showLoadingScreen,
    showBootMenu,
    showPromptScreen,
    showSplashScreen,
  };

  return <TonkContext.Provider value={value}>{children}</TonkContext.Provider>;
}

export function useTonk() {
  const context = useContext(TonkContext);
  if (!context) {
    throw new Error('useTonk must be used within TonkProvider');
  }
  return context;
}
