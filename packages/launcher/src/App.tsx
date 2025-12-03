import { useCallback, useEffect, useState } from 'react';
import { Sidebar } from './components/sidebar/sidebar';
import GradientLogo from './components/ui/gradientLogo';
import { GradientText } from './components/ui/gradientText';
import { bundleManager } from './launcher/services/bundleManager';
import { bundleStorage } from './launcher/services/bundleStorage';
import type { Bundle } from './launcher/types';

function App() {
  const [bundles, setBundles] = useState<Bundle[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [importing, setImporting] = useState(false);
  const [runtimeUrl, setRuntimeUrl] = useState<string | null>(null);

  const loadBundles = useCallback(async () => {
    try {
      setLoading(true);
      const list = await bundleManager.listBundles();
      setBundles(list);
      setError(null);
    } catch (err) {
      console.error('Failed to load bundles:', err);
      setError('Failed to load bundles');
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    loadBundles();
  }, [loadBundles]);

  const handleLaunch = async (id: string) => {
    try {
      // Verify bundle exists before launching
      const bundle = await bundleStorage.get(id);
      if (!bundle) {
        throw new Error('Bundle not found');
      }

      // Pass bundleId - RuntimeApp will fetch from shared IndexedDB
      setRuntimeUrl(`/app/index.html?bundleId=${encodeURIComponent(id)}`);
    } catch (err) {
      console.error('Failed to launch bundle:', err);
      alert('Failed to launch bundle');
    }
  };

  const handleFileUpload = async (event: React.ChangeEvent<HTMLInputElement>) => {
    const file = event.target.files?.[0];
    if (!file) return;

    try {
      setImporting(true);
      const id = await bundleManager.loadBundleFromFile(file);
      await loadBundles();
      // Auto-launch the new bundle
      await handleLaunch(id);

      // Reset input
      event.target.value = '';
    } catch (err) {
      console.error('Failed to import bundle:', err);
      alert(err instanceof Error ? err.message : 'Failed to import bundle');
    } finally {
      setImporting(false);
    }
  };

  return (
    <div className="flex h-screen font-sans overflow-hidden w-full">
      {/* Sidebar */}
      <Sidebar
        bundles={bundles}
        handleLaunch={handleLaunch}
        handleFileUpload={handleFileUpload}
        importing={importing}
      />

      {/* Main Content */}
      <main className="flex-1 relative bg-stone-100 dark:bg-night-900 z-0">
        {error && (
          <div className="flex items-center justify-center h-full text-stone-400">
            <div className="text-center">
              <svg
                className="w-16 h-16 mx-auto mb-4 opacity-20"
                fill="none"
                stroke="currentColor"
                viewBox="0 0 24 24"
              >
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  strokeWidth={2}
                  d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-3L13.732 4c-.77-1.333-2.694-1.333-3.464 0L3.34 16c-.77 1.333.192 3 1.732 3z"
                />
              </svg>
              <p>{error}</p>
            </div>
          </div>
        )}
        {loading && 'loading'}
        {runtimeUrl ? (
          <iframe
            key={runtimeUrl} // Force remount on URL change
            src={runtimeUrl}
            className="absolute inset-0 w-full h-full border-none"
            title="Runtime"
            allow="clipboard-read; clipboard-write"
          />
        ) : (
          <div className="flex items-center justify-center h-full text-stone-400">
            <div className="flex items-center justify-center flex-col text-center">
              <GradientLogo />
              <GradientText className="font-gestalt text-lg font-medium mt-4 dark:invert-100">
                select a bundle to launch
              </GradientText>
            </div>
          </div>
        )}
      </main>
    </div>
  );
}

export default App;
