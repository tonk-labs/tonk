import { useTonk } from '../../context/TonkContext';

export function LoadingScreen() {
  const { loadingMessage } = useTonk();

  return (
    <div>
      <p class="loading-message">{loadingMessage}</p>
    </div>
  );
}
