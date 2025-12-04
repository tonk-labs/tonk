import { useState } from 'react';
import icon_hourglass from '../../assets/icon-small-hourglass.svg';
import { useTonk } from '../../context/TonkContext';

export function LoadingScreen() {
  const { loadingMessage } = useTonk();
  const [imageLoaded, setImageLoaded] = useState(false);

  return (
    <div className="flex w-full h-full items-center justify-center bg-stone-100 dark:bg-night-900">
      <img
        src={icon_hourglass}
        alt={loadingMessage}
        className={`w-8 h-8 animate-spin transition-opacity duration-200 ${imageLoaded ? 'opacity-100' : 'opacity-0'}`}
        onLoad={() => setImageLoaded(true)}
      />
    </div>
  );
}
