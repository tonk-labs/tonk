import { getVFSService } from '@tonk/host-web/client';
import { StrictMode, useEffect, useState } from 'react';
import { createRoot } from 'react-dom/client';
import Router from './Router';

// Import sample files utility to make it available in browser console
import './utils/sampleFiles';

function AppLauncher() {
  const [ready, setReady] = useState(false);
  const [error, setError] = useState<Error | null>(null);

  useEffect(() => {
    // In launcher environment, we just connect to the existing kernel
    getVFSService()
      .connect()
      .then(() => setReady(true))
      .catch(err => {
        console.warn('VFS connection warning:', err);
        setError(err instanceof Error ? err : new Error(String(err)));
      });
  }, []);

  if (error) {
    return (
      <div className="flex items-center justify-center h-screen bg-gray-50 text-red-600 font-mono p-4">
        <div className="max-w-md">
          <h1 className="text-xl font-bold mb-2">Kernel Connection Failed</h1>
          <p className="bg-white p-4 rounded border border-red-200 overflow-auto text-sm">
            {error.message}
          </p>
          <button
            type="button"
            onClick={() => window.location.reload()}
            className="mt-4 px-4 py-2 bg-red-600 text-white rounded hover:bg-red-700"
          >
            Retry
          </button>
        </div>
      </div>
    );
  }

  if (!ready) {
    return (
      <div className="flex items-center justify-center h-screen bg-gray-50 text-gray-500 font-mono">
        <div className="animate-pulse">Connecting to Kernel...</div>
      </div>
    );
  }

  return <Router />;
}

// biome-ignore lint/style/noNonNullAssertion: <lol>
createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <AppLauncher />
  </StrictMode>
);
