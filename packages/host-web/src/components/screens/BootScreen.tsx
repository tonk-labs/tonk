import { AppList } from '../AppList';
import { useDialogs } from '../../context/DialogContext';
import { useServiceWorker } from '../../hooks/useServiceWorker';
import { useTonk } from '../../context/TonkContext';

declare const TONK_SERVE_LOCAL: boolean;

export function BootScreen() {
  const { openShareDialog, openNameDialog, openDownloadSpinner, closeDownloadSpinner } = useDialogs();
  const { downloadTonk, createNewTonk } = useServiceWorker();
  const { availableApps } = useTonk();

  const handleShare = async () => {
    if (typeof TONK_SERVE_LOCAL !== 'undefined' && TONK_SERVE_LOCAL) {
      // Dev mode: just download
      await downloadTonk();
    } else {
      // Production: show share dialog
      openShareDialog();
    }
  };

  const handleNew = async () => {
    if (typeof TONK_SERVE_LOCAL !== 'undefined' && TONK_SERVE_LOCAL) {
      // Dev mode: skip name dialog, auto-generate name
      const timestamp = new Date().toISOString().slice(0, 19).replace(/[:.]/g, '-');
      const tonkName = `tonk-${timestamp}`;
      const hasLoadedBundles = availableApps.length > 0;

      openDownloadSpinner();
      try {
        await createNewTonk(tonkName, hasLoadedBundles);
      } finally {
        closeDownloadSpinner();
      }
    } else {
      // Production: show name dialog
      const defaultName = 'new-tonk';
      const hasLoadedBundles = availableApps.length > 0;
      openNameDialog(defaultName, hasLoadedBundles);
    }
  };

  return (
    <div class="flex min-h-0 h-full flex-col">
      <div class="border-2 border-white p-6 mb-6 flex grow transition-[border-color] duration-200 ease-in-out boot-menu">
        <AppList />
      </div>

      <div class="text-[#ddd] text-xs leading-relaxed space-y-1">
        <div>Use the ↑ and ↓ keys to select which entry is highlighted.</div>
        <div>Press enter to boot the selected application</div>
        <div class="pt-2">Drag a .tonk file onto the applications box to load it.</div>
      </div>

      <div class="mt-6 flex gap-3">
        <button
          onClick={handleShare}
          class="px-6 py-2 bg-white text-black border border-[#666] cursor-pointer font-semibold hover:bg-[#ddd] transition-colors"
        >
          {typeof TONK_SERVE_LOCAL !== 'undefined' && TONK_SERVE_LOCAL ? 'Export Tonk' : 'Share Tonk'}
        </button>
        <button
          onClick={handleNew}
          class="px-6 py-2 bg-white text-black border border-[#666] cursor-pointer font-semibold hover:bg-[#ddd] transition-colors"
        >
          New Tonk
        </button>
      </div>
    </div>
  );
}
