import { createContext, type ReactNode, useContext, useState } from 'react';
import { ScreenState, type TonkContextValue } from '../types';

const TonkContext = createContext<TonkContextValue | undefined>(undefined);

export function TonkProvider({ children }: { children: ReactNode }) {
  const [screenState, setScreenState] = useState<ScreenState>(
    ScreenState.LOADING
  );
  const [loadingMessage, setLoadingMessage] = useState('Initializing...');
  const [errorMessage, setErrorMessage] = useState('');

  const showError = (message: string) => {
    setErrorMessage(message);
    setScreenState(ScreenState.ERROR);
  };

  const showLoadingScreen = (message = 'Loading...') => {
    setLoadingMessage(message);
    setScreenState(ScreenState.LOADING);
  };

  const value: TonkContextValue = {
    screenState,
    loadingMessage,
    errorMessage,
    showError,
    showLoadingScreen,
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
