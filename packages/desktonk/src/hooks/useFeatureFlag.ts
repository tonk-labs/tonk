import { useFeatureFlagContext } from '../contexts/FeatureFlagContext';
import type { FeatureFlags } from '../lib/featureFlags';

export function useFeatureFlag(key: keyof FeatureFlags): boolean {
  const { flags } = useFeatureFlagContext();
  return flags[key];
}
