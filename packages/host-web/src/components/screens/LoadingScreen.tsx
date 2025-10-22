import { useTonk } from '../../context/TonkContext';
import icon_hourglass from "../../assets/icon-small-hourglass.svg";

export function LoadingScreen() {
  const { loadingMessage } = useTonk();

  return (
    <div class="flex min-h-0 h-full w-full grow items-center justify-center bg-[#E0E0E0]">
        <img src={icon_hourglass} alt="hourglass icon" class="w-8 h-8 mx-auto animate-spin"/>
    </div>
  );
}
