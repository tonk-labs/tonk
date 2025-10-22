import { useState } from 'preact/hooks';
import icon_tonk from '../../assets/icon-tonk.svg';

export function SplashScreen() {
  const [imageLoaded, setImageLoaded] = useState(false);

  return (
    <div class="border-1 border-[#000] flex min-h-0 h-full w-full grow items-center justify-center bg-[#E0E0E0]">
      <img
        src={icon_tonk}
        alt={'Tonk'}
        class={`w-8 h-8 mx-auto animate-bounce transition-opacity duration-200 ${imageLoaded ? 'opacity-100' : 'opacity-0'}`}
        onLoad={() => setImageLoaded(true)}
      />
    </div>
  );
}
