import { useTonk } from '../../context/TonkContext';

export function ErrorScreen() {
  const { errorMessage } = useTonk();

  return (
    <div className="flex w-full h-full items-center justify-center bg-stone-100 dark:bg-night-900 p-4">
      <p className="text-red-600 dark:text-red-400 text-center whitespace-pre-wrap">
        {errorMessage}
      </p>
    </div>
  );
}
