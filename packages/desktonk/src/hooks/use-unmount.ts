import { useEffect, useRef } from 'react';

/**
 * Hook that executes a callback when the component unmounts.
 *
 * @param callback Function to be called on component unmount
 */
// eslint-disable-next-line @typescript-eslint/no-explicit-any
// biome-ignore lint/suspicious/noExplicitAny: Generic callback signature
export const useUnmount = (callback: (...args: Array<any>) => any) => {
  const ref = useRef(callback);
  ref.current = callback;

  useEffect(
    () => () => {
      ref.current();
    },
    []
  );
};

export default useUnmount;
