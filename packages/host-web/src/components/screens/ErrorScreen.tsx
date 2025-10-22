import { useTonk } from '../../context/TonkContext';

export function ErrorScreen() {
  const { errorMessage } = useTonk();

  return (
    <div>
      <p class="text-red-500">{errorMessage}</p>
    </div>
  );
}
