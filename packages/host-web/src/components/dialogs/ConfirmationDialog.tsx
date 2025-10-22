import { useDialogs } from '../../context/DialogContext';
import { useServiceWorker } from '../../hooks/useServiceWorker';
import { useTonk } from '../../context/TonkContext';

export function ConfirmationDialog() {
  const { dialogs, closeConfirmationDialog } = useDialogs();
  const { confirmBoot } = useServiceWorker();
  const { availableApps, selectedAppIndex } = useTonk();

  if (!dialogs.confirmation.isOpen) return null;

  const handleConfirm = async () => {
    closeConfirmationDialog();
    const app = availableApps[selectedAppIndex];
    const appSlug = typeof app === 'string' ? app : app.slug || app.name;
    await confirmBoot(appSlug);
  };

  return (
    <div class="fixed top-1/2 left-1/2 -translate-x-1/2 -translate-y-1/2 bg-black border-2 border-[#666] p-6 w-[90%] max-w-md z-[1000]">
      <div class="text-white mb-4 text-sm font-semibold">Boot Confirmation</div>
      <div class="text-[#999] mb-6 text-sm">
        Are you sure you want to boot "{dialogs.confirmation.appName}"?
      </div>
      <div class="flex gap-3">
        <button
          onClick={handleConfirm}
          class="px-5 py-2 bg-white text-black border border-[#666] cursor-pointer font-semibold text-sm hover:bg-[#ddd] transition-colors"
        >
          Yes
        </button>
        <button
          onClick={closeConfirmationDialog}
          class="px-5 py-2 bg-[#333] text-white border border-[#666] cursor-pointer text-sm hover:bg-[#444] transition-colors"
        >
          No
        </button>
      </div>
    </div>
  );
}
