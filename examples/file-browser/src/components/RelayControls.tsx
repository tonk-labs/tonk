import type { Manifest } from '@tonk/core';
import type React from 'react';
import { useState } from 'react';

export interface RelayControlsProps {
  manifest: Manifest;
  connectedRelay: string | null;
  onConnect: (relayUrl: string) => Promise<void>;
}

/**
 * Filter network URIs to only HTTP(S) relay URLs
 */
function getHttpRelays(networkUris: string[]): string[] {
  return networkUris.filter(
    uri => uri.startsWith('http://') || uri.startsWith('https://')
  );
}

const RelayControls: React.FC<RelayControlsProps> = ({
  manifest,
  connectedRelay,
  onConnect,
}) => {
  const [showModal, setShowModal] = useState(false);
  const [customUrl, setCustomUrl] = useState('');
  const [isConnecting, setIsConnecting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const httpRelays = getHttpRelays(manifest.networkUris || []);

  const handleConnect = async (url: string) => {
    setIsConnecting(true);
    setError(null);
    try {
      await onConnect(url);
      setShowModal(false);
      setCustomUrl('');
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to connect');
    } finally {
      setIsConnecting(false);
    }
  };

  return (
    <>
      {/* Header bar showing connection status */}
      <div className="bg-white border-b border-[#d2d2d7] px-6 py-3 flex justify-between items-center sticky top-0 z-10 shadow-sm">
        <div className="flex items-center gap-3">
          <span
            className={`h-2.5 w-2.5 rounded-full ${
              connectedRelay ? 'bg-green-500' : 'bg-yellow-500'
            }`}
          />
          <span className="text-sm text-[#86868b]">
            {connectedRelay ? 'Connected' : 'Not connected'}
          </span>
          {connectedRelay && (
            <span className="text-xs text-[#86868b] font-mono">
              {connectedRelay}
            </span>
          )}
        </div>
        <div className="flex items-center gap-4">
          <span className="text-xs text-[#86868b]">
            {manifest.entrypoints.join(', ')}
          </span>
          <button
            onClick={() => setShowModal(true)}
            className="text-sm text-[#0066cc] hover:text-[#004499] transition-colors flex items-center gap-1"
          >
            Relay Settings
          </button>
        </div>
      </div>

      {/* Modal for relay settings */}
      {showModal && (
        <div className="fixed inset-0 bg-black bg-opacity-50 flex items-center justify-center z-50">
          <div className="bg-white rounded-2xl shadow-2xl p-8 max-w-md w-full mx-4">
            <h2 className="text-2xl font-medium mb-6 text-[#1d1d1f]">
              Relay Connection
            </h2>

            {/* Current status */}
            <div className="mb-6 p-3 bg-[#f5f5f7] rounded-lg">
              <div className="text-sm text-[#86868b] mb-1">Current status</div>
              <div className="flex items-center gap-2">
                <span
                  className={`h-2 w-2 rounded-full ${
                    connectedRelay ? 'bg-green-500' : 'bg-yellow-500'
                  }`}
                />
                <span className="text-[#1d1d1f] font-mono text-sm">
                  {connectedRelay || 'Not connected'}
                </span>
              </div>
            </div>

            {/* Relays from manifest */}
            {httpRelays.length > 0 && (
              <div className="mb-6">
                <label className="block text-sm font-medium mb-2 text-[#1d1d1f]">
                  Relays from bundle
                </label>
                <div className="space-y-2">
                  {httpRelays.map((relay, i) => (
                    <button
                      key={i}
                      onClick={() => handleConnect(relay)}
                      disabled={isConnecting || connectedRelay === relay}
                      className={`w-full text-left px-4 py-3 rounded-lg border transition-all font-mono text-sm
                        ${
                          connectedRelay === relay
                            ? 'border-green-500 bg-green-50 text-green-700'
                            : 'border-[#d2d2d7] hover:border-[#0066cc] hover:bg-[#fafafa]'
                        }
                        ${isConnecting ? 'opacity-50' : ''}
                      `}
                    >
                      {relay}
                      {connectedRelay === relay && (
                        <span className="ml-2 text-green-600">(connected)</span>
                      )}
                    </button>
                  ))}
                </div>
              </div>
            )}

            {/* Custom URL input */}
            <div className="mb-6">
              <label className="block text-sm font-medium mb-2 text-[#1d1d1f]">
                Custom relay URL
              </label>
              <div className="flex gap-2">
                <input
                  type="text"
                  value={customUrl}
                  onChange={e => setCustomUrl(e.target.value)}
                  className="flex-1 px-4 py-3 border border-[#d2d2d7] rounded-lg focus:outline-none focus:ring-2 focus:ring-[#0066cc] focus:border-transparent font-mono text-sm"
                  placeholder="http://localhost:8081"
                  disabled={isConnecting}
                />
                <button
                  onClick={() => handleConnect(customUrl)}
                  disabled={isConnecting || !customUrl.trim()}
                  className="px-5 py-2.5 bg-[#0066cc] text-white rounded-lg hover:bg-[#004499] transition-all font-medium disabled:opacity-50"
                >
                  {isConnecting ? 'Connecting...' : 'Connect'}
                </button>
              </div>
            </div>

            {error && (
              <div className="mb-6 p-3 bg-[#fef1f2] border border-[#ffccd0] rounded-lg">
                <span className="text-[#ff3b30] text-sm">{error}</span>
              </div>
            )}

            <div className="flex justify-end">
              <button
                onClick={() => {
                  setShowModal(false);
                  setError(null);
                }}
                className="px-5 py-2.5 text-[#0066cc] hover:bg-[#f5f5f7] rounded-full transition-all font-medium"
              >
                Close
              </button>
            </div>
          </div>
        </div>
      )}
    </>
  );
};

export default RelayControls;
