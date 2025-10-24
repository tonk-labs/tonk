import React, { useState, useEffect } from 'react';
import { Route, Routes } from 'react-router-dom';
import { FileBrowser, FileViewer } from './views';
import { TonkService } from './services/tonkService';

const getDefaultRelayUrl = () => {
  const urlParams = new URLSearchParams(window.location.search);
  const urlParam = urlParams.get('relayUrl');
  if (urlParam) return urlParam;

  const stored = localStorage.getItem('tonk_relay_url');
  if (stored) return stored;

  if (import.meta.env.VITE_RELAY_URL) {
    return import.meta.env.VITE_RELAY_URL;
  }

  const isLocalhost =
    window.location.hostname === 'localhost' ||
    window.location.hostname === '127.0.0.1';

  return isLocalhost ? 'http://localhost:8081' : 'https://relay.tonk.xyz';
};

const App: React.FC = () => {
  const [relayUrl, setRelayUrl] = useState(getDefaultRelayUrl());
  const [isConnected, setIsConnected] = useState(false);
  const [isConnecting, setIsConnecting] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [showSettings, setShowSettings] = useState(false);
  const [tempRelayUrl, setTempRelayUrl] = useState(relayUrl);

  useEffect(() => {
    initializeTonk(relayUrl);
  }, [relayUrl]);

  const initializeTonk = async (url: string) => {
    setIsConnecting(true);
    setError(null);

    try {
      await TonkService.initialize(url);
      setIsConnected(true);
      localStorage.setItem('tonk_relay_url', url);
    } catch (err) {
      setError(
        err instanceof Error ? err.message : 'Failed to connect to relay server'
      );
      setIsConnected(false);
    } finally {
      setIsConnecting(false);
    }
  };

  const handleSaveRelayUrl = () => {
    setRelayUrl(tempRelayUrl);
    setShowSettings(false);
  };

  return (
    <>
      {/* Settings Modal - Always rendered so it's accessible from any state */}
      {showSettings && (
        <div className="fixed inset-0 bg-black bg-opacity-50 flex items-center justify-center z-50">
          <div className="bg-white rounded-2xl shadow-2xl p-8 max-w-md w-full mx-4">
            <h2 className="text-2xl font-medium mb-6 text-[#1d1d1f]">
              Relay Server Settings
            </h2>
            <div className="mb-6">
              <label className="block text-sm font-medium mb-2 text-[#1d1d1f]">
                Relay URL
              </label>
              <input
                type="text"
                value={tempRelayUrl}
                onChange={e => setTempRelayUrl(e.target.value)}
                className="w-full px-4 py-3 border border-[#d2d2d7] rounded-lg focus:outline-none focus:ring-2 focus:ring-[#0066cc] focus:border-transparent font-mono text-sm"
                placeholder="http://localhost:8081"
              />
              <div className="text-xs text-[#86868b] mt-2">
                Examples:{' '}
                <span className="font-mono">http://localhost:8081</span>,{' '}
                <span className="font-mono">https://relay.tonk.xyz</span>
              </div>
            </div>
            <div className="flex justify-end gap-3">
              <button
                onClick={() => {
                  setShowSettings(false);
                  setTempRelayUrl(relayUrl);
                }}
                className="px-5 py-2.5 text-[#0066cc] hover:bg-[#f5f5f7] rounded-full transition-all font-medium"
              >
                Cancel
              </button>
              <button
                onClick={handleSaveRelayUrl}
                className="px-5 py-2.5 bg-[#0066cc] text-white rounded-full hover:bg-[#004499] transition-all font-medium"
              >
                Connect
              </button>
            </div>
          </div>
        </div>
      )}

      {/* Connecting State */}
      {isConnecting && (
        <div className="min-h-screen flex items-center justify-center bg-[#f5f5f7]">
          <div className="text-center">
            <div className="mb-4">
              <svg
                className="animate-spin h-12 w-12 text-[#0066cc] mx-auto"
                xmlns="http://www.w3.org/2000/svg"
                fill="none"
                viewBox="0 0 24 24"
              >
                <circle
                  className="opacity-25"
                  cx="12"
                  cy="12"
                  r="10"
                  stroke="currentColor"
                  strokeWidth="4"
                ></circle>
                <path
                  className="opacity-75"
                  fill="currentColor"
                  d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4zm2 5.291A7.962 7.962 0 014 12H0c0 3.042 1.135 5.824 3 7.938l3-2.647z"
                ></path>
              </svg>
            </div>
            <div className="text-xl mb-2 text-[#1d1d1f]">
              Connecting to Tonk relay...
            </div>
            <div className="text-[#86868b] font-mono text-sm">{relayUrl}</div>
          </div>
        </div>
      )}

      {/* Error State */}
      {error && !isConnecting && (
        <div className="min-h-screen flex items-center justify-center bg-[#f5f5f7]">
          <div className="text-center max-w-md bg-white rounded-2xl shadow-md p-8">
            <div className="text-xl font-medium text-[#ff3b30] mb-4">
              Connection Failed
            </div>
            <div className="text-[#1d1d1f] mb-2">
              Could not connect to relay server:
            </div>
            <div className="text-[#86868b] text-sm mb-4 font-mono bg-[#f5f5f7] p-3 rounded-lg break-all">
              {relayUrl}
            </div>
            <div className="text-[#ff3b30] text-sm mb-6 bg-[#fef1f2] p-3 rounded-lg border border-[#ffccd0]">
              {error}
            </div>
            <button
              onClick={() => setShowSettings(true)}
              className="px-6 py-3 bg-[#0066cc] text-white rounded-full hover:bg-[#004499] transition-all font-medium"
            >
              Change Relay URL
            </button>
          </div>
        </div>
      )}

      {/* Connected State - Main App */}
      {isConnected && !isConnecting && (
        <>
          <div className="bg-white border-b border-[#d2d2d7] px-6 py-3 flex justify-between items-center sticky top-0 z-10 shadow-sm">
            <div className="flex items-center gap-3">
              <span
                className={`h-2.5 w-2.5 rounded-full ${isConnected ? 'bg-green-500' : 'bg-red-500'}`}
              />
              <span className="text-sm text-[#86868b]">
                {isConnected ? 'Connected' : 'Disconnected'}
              </span>
              <span className="text-xs text-[#86868b] font-mono">
                {relayUrl}
              </span>
            </div>
            <button
              onClick={() => setShowSettings(true)}
              className="text-sm text-[#0066cc] hover:text-[#004499] transition-colors flex items-center gap-1"
            >
              <span>⚙️</span> Settings
            </button>
          </div>

          <Routes>
            <Route path="/" element={<FileBrowser />} />
            <Route path="/view" element={<FileViewer />} />
          </Routes>
        </>
      )}
    </>
  );
};

export default App;
