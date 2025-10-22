import { useTonk } from '../../context/TonkContext';

export function LoadingScreen() {
  const { loadingMessage } = useTonk();

  return (
    <div>
      <p class="text-white animate-[blink_1s_infinite]">{loadingMessage}</p>
    </div>
  );
}
