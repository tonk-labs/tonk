import { useState } from 'preact/hooks';
import { useTonk } from '../../context/TonkContext';
import icon_hourglass from '../../assets/icon-small-hourglass.svg';

export function LoadingScreen() {
  const { loadingMessage } = useTonk();
  const [imageLoaded, setImageLoaded] = useState(false);

  return (
    <div class="border-1 border-[#000] flex min-h-0 h-full w-full grow items-center justify-center bg-[#E0E0E0] animate-pulse">
      <img
        src={icon_hourglass}
        alt={loadingMessage}
        class={`w-8 h-8 mx-auto animate-spin transition-opacity duration-200 ${imageLoaded ? 'opacity-100' : 'opacity-0'}`}
        onLoad={() => setImageLoaded(true)}
      />
    </div>
  );
}
