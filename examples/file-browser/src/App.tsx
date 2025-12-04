import React, { useState } from 'react';
import { Route, Routes } from 'react-router-dom';
import { Manifest } from '@tonk/core';
import { FileBrowser, FileViewer } from './views';
import { TonkService } from './services/tonkService';
import BundleLoader from './components/BundleLoader';
import RelayControls from './components/RelayControls';

type AppState =
  | { status: 'no-bundle' }
  | { status: 'loading' }
  | { status: 'error'; message: string }
  | { status: 'ready'; manifest: Manifest; connectedRelay: string | null };

const App: React.FC = () => {
  const [state, setState] = useState<AppState>({ status: 'no-bundle' });

  const handleBundleLoad = async (bytes: Uint8Array, manifest: Manifest) => {
    setState({ status: 'loading' });

    try {
      const { connectedRelay } = await TonkService.initializeFromBundle(bytes);
      setState({ status: 'ready', manifest, connectedRelay });
    } catch (err) {
      setState({
        status: 'error',
        message: err instanceof Error ? err.message : 'Failed to initialize bundle',
      });
    }
  };

  const handleConnectRelay = async (relayUrl: string) => {
    if (state.status !== 'ready') return;
    await TonkService.connectRelay(relayUrl);
    setState({ ...state, connectedRelay: relayUrl });
  };

  // Show bundle loader when no bundle is loaded
  if (state.status === 'no-bundle' || state.status === 'loading') {
    return (
      <BundleLoader
        onBundleLoad={handleBundleLoad}
        isLoading={state.status === 'loading'}
      />
    );
  }

  // Show error state
  if (state.status === 'error') {
    return (
      <div className="min-h-screen flex items-center justify-center bg-[#f5f5f7]">
        <div className="text-center max-w-md bg-white rounded-2xl shadow-md p-8">
          <div className="text-xl font-medium text-[#ff3b30] mb-4">
            Failed to Load Bundle
          </div>
          <div className="text-[#ff3b30] text-sm mb-6 bg-[#fef1f2] p-3 rounded-lg border border-[#ffccd0]">
            {state.message}
          </div>
          <button
            onClick={() => setState({ status: 'no-bundle' })}
            className="px-6 py-3 bg-[#0066cc] text-white rounded-full hover:bg-[#004499] transition-all font-medium"
          >
            Try Again
          </button>
        </div>
      </div>
    );
  }

  // Ready state - show file browser
  return (
    <>
      <RelayControls
        manifest={state.manifest}
        connectedRelay={state.connectedRelay}
        onConnect={handleConnectRelay}
      />
      <Routes>
        <Route path="/" element={<FileBrowser />} />
        <Route path="/view" element={<FileViewer />} />
      </Routes>
    </>
  );
};

export default App;
