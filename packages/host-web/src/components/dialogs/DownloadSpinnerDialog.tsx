import { useDialogs } from '../../context/DialogContext';
import icon_hourglass from "../../assets/icon-hourglass.svg";
export function DownloadSpinnerDialog() {
  const { dialogs } = useDialogs();

  if (!dialogs.downloadSpinner.isOpen) return null;

  return (
    <div class="fixed top-1/2 left-1/2 -translate-x-1/2 -translate-y-1/2 bg-black border-2 border-[#666] p-8 w-[90%] max-w-xs z-[1001] text-center">
      <div class="w-10 h-10 mx-auto mb-3 border-4 border-[#333] border-t-white rounded-full animate-[spin_1s_linear_infinite]"><img src={icon_hourglass} alt="hourglass" class="w-16 h-16 mx-auto"/></div>
      <div class="text-[#999] mt-6">Preparing tonk...</div>
    </div>
  );
}
