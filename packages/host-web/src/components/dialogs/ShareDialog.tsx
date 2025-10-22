import { useEffect } from 'preact/hooks';
import { useDialogs } from '../../context/DialogContext';
import { useServiceWorker } from '../../hooks/useServiceWorker';

export function ShareDialog() {
  const { dialogs, closeShareDialog, updateShareDialog, openDownloadSpinner, closeDownloadSpinner } = useDialogs();
  const { shareAsUrl, downloadTonk } = useServiceWorker();

  useEffect(() => {
    if (dialogs.share.isOpen && !dialogs.share.shareUrl && !dialogs.share.error) {
      // Auto-start upload when dialog opens
      shareAsUrl();
    }
  }, [dialogs.share.isOpen]);

  if (!dialogs.share.isOpen) return null;

  const handleCopyUrl = async () => {
    if (!dialogs.share.shareUrl) return;

    try {
      await navigator.clipboard.writeText(dialogs.share.shareUrl);
      // Could add visual feedback here
    } catch (err) {
      // Fallback - not implemented in this version
      console.error('Failed to copy:', err);
    }
  };

  const handleDownload = async () => {
    openDownloadSpinner();
    try {
      await downloadTonk();
    } finally {
      closeDownloadSpinner();
    }
  };

  const handleRetry = () => {
    updateShareDialog({ error: null, isLoading: false, shareUrl: null, qrCodeUrl: null });
    shareAsUrl();
  };

  return (
    <div class="fixed top-1/2 left-1/2 -translate-x-1/2 -translate-y-1/2 bg-black border-2 border-[#666] p-6 w-[90%] max-w-md z-[1000]">
      <div class="text-white mb-4 text-sm font-semibold">Share Tonk</div>

      {dialogs.share.isLoading && (
        <div class="text-center py-6">
          <div class="w-10 h-10 mx-auto mb-3 border-4 border-[#333] border-t-white rounded-full animate-[spin_1s_linear_infinite]" />
          <p class="mt-3">Uploading bundle to server...</p>
        </div>
      )}

      {dialogs.share.error && (
        <div>
          <div class="text-red-500 mb-6">{dialogs.share.error}</div>
          <button
            onClick={handleRetry}
            class="w-full px-5 py-2 mb-3 bg-white text-black border border-[#666] cursor-pointer font-semibold text-sm hover:bg-[#ddd] transition-colors"
          >
            Try Again
          </button>
          <button
            onClick={closeShareDialog}
            class="w-full px-5 py-2 bg-[#333] text-white border border-[#666] cursor-pointer text-sm hover:bg-[#444] transition-colors"
          >
            Close
          </button>
        </div>
      )}

      {dialogs.share.shareUrl && !dialogs.share.isLoading && (
        <div class="text-center">
          {dialogs.share.qrCodeUrl && (
            <img
              src={dialogs.share.qrCodeUrl}
              alt="QR Code"
              class="w-[200px] h-[200px] mx-auto mb-4 bg-white p-3 block"
            />
          )}
          <div class="text-[#999] mb-6">Scan to access Tonk</div>

          <div class="space-y-3">
            <input
              type="text"
              value={dialogs.share.shareUrl}
              readOnly
              class="w-full p-3 bg-[#222] border border-[#666] text-white text-xs text-center focus:outline-none focus:border-[#f90] transition-colors"
            />
            <button
              onClick={handleCopyUrl}
              class="w-full px-5 py-2 bg-white text-black border border-[#666] cursor-pointer font-semibold text-sm hover:bg-[#ddd] transition-colors"
            >
              Copy URL
            </button>
            <button
              onClick={handleDownload}
              class="w-full px-5 py-2 bg-white text-black border border-[#666] cursor-pointer font-semibold text-sm hover:bg-[#ddd] transition-colors"
            >
              Download as File
            </button>
            <button
              onClick={closeShareDialog}
              class="w-full px-5 py-2 bg-[#333] text-white border border-[#666] cursor-pointer text-sm hover:bg-[#444] transition-colors"
            >
              Close
            </button>
          </div>
        </div>
      )}
    </div>
  );
}
