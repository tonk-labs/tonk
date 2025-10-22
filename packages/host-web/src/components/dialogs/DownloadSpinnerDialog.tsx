import { useDialogs } from '../../context/DialogContext';

export function DownloadSpinnerDialog() {
  const { dialogs } = useDialogs();

  if (!dialogs.downloadSpinner.isOpen) return null;

  return (
    <div class="fixed top-1/2 left-1/2 -translate-x-1/2 -translate-y-1/2 bg-black border-2 border-[#666] p-8 w-[90%] max-w-xs z-[1001] text-center">
      <div class="spinner" />
      <div class="text-[#999] mt-6">Preparing tonk...</div>
    </div>
  );
}
