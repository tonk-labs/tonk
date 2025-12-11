import { createContext, type ReactNode, useContext } from 'react';
import { type FeatureFlags, useFeatureFlagStore } from '../lib/featureFlags';

interface FeatureFlagContextValue {
  flags: FeatureFlags;
  setFlag: (key: keyof FeatureFlags, value: boolean) => void;
  resetFlags: () => void;
}

const FeatureFlagContext = createContext<FeatureFlagContextValue | undefined>(undefined);

export function FeatureFlagProvider({ children }: { children: ReactNode }) {
  const { flags, setFlag, resetFlags } = useFeatureFlagStore();

  return (
    <FeatureFlagContext.Provider value={{ flags, setFlag, resetFlags }}>
      {children}
    </FeatureFlagContext.Provider>
  );
}

export function useFeatureFlagContext() {
  const context = useContext(FeatureFlagContext);
  if (!context) {
    throw new Error('useFeatureFlagContext must be used within FeatureFlagProvider');
  }
  return context;
}
