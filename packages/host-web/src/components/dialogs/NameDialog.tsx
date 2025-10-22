import { useState } from 'preact/hooks';
import { useDialogs } from '../../context/DialogContext';
import { useServiceWorker } from '../../hooks/useServiceWorker';

export function NameDialog() {
  const { dialogs, closeNameDialog, openDownloadSpinner, closeDownloadSpinner } = useDialogs();
  const { createNewTonk } = useServiceWorker();
  const [name, setName] = useState('');

  if (!dialogs.name.isOpen) return null;

  const handleConfirm = async () => {
    const tonkName = name.trim() || dialogs.name.defaultName;
    closeNameDialog();
    openDownloadSpinner();

    try {
      await createNewTonk(tonkName, dialogs.name.hasLoadedBundles);
    } finally {
      closeDownloadSpinner();
    }
  };

  const handleKeyDown = (e: KeyboardEvent) => {
    if (e.key === 'Enter') {
      handleConfirm();
    } else if (e.key === 'Escape') {
      closeNameDialog();
    }
  };

  return (
    <div class="fixed top-1/2 left-1/2 -translate-x-1/2 -translate-y-1/2 bg-black border-2 border-[#666] p-6 w-[90%] max-w-md z-[1000]">
      <div class="text-white mb-4 text-sm font-semibold">New Tonk Name</div>
      <div class="text-[#ddd] mb-4 text-xs">Enter a name for your new Tonk:</div>
      <div class="mb-6">
        <input
          type="text"
          value={name}
          onInput={(e) => setName((e.target as HTMLInputElement).value)}
          onKeyDown={handleKeyDown as any}
          placeholder={dialogs.name.defaultName}
          maxLength={50}
          class="w-full p-3 bg-[#222] border border-[#666] text-white text-sm focus:outline-none focus:border-[#f90] transition-colors"
          autoFocus
        />
      </div>
      <div class="flex gap-3 justify-end">
        <button
          onClick={handleConfirm}
          class="px-5 py-2 bg-white text-black border border-[#666] cursor-pointer font-semibold text-sm hover:bg-[#ddd] transition-colors"
        >
          Create
        </button>
        <button
          onClick={closeNameDialog}
          class="px-5 py-2 bg-[#333] text-white border border-[#666] cursor-pointer text-sm hover:bg-[#444] transition-colors"
        >
          Cancel
        </button>
      </div>
    </div>
  );
}
