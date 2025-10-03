import { useEffect, useState } from 'react';

interface ViewportState {
  height: number;
  isKeyboardOpen: boolean;
  keyboardHeight: number;
}

export function useVisualViewport() {
  const [viewportState, setViewportState] = useState<ViewportState>(() => {
    if (typeof window === 'undefined' || !window.visualViewport) {
      return {
        height: window.innerHeight,
        isKeyboardOpen: false,
        keyboardHeight: 0,
      };
    }

    const visualViewport = window.visualViewport;
    const keyboardHeight = window.innerHeight - visualViewport.height;

    return {
      height: visualViewport.height,
      isKeyboardOpen: keyboardHeight > 100,
      keyboardHeight: Math.max(0, keyboardHeight),
    };
  });

  useEffect(() => {
    if (typeof window === 'undefined' || !window.visualViewport) {
      return;
    }

    const visualViewport = window.visualViewport;

    const handleResize = () => {
      const keyboardHeight = window.innerHeight - visualViewport.height;

      setViewportState({
        height: visualViewport.height,
        isKeyboardOpen: keyboardHeight > 100,
        keyboardHeight: Math.max(0, keyboardHeight),
      });
    };

    visualViewport.addEventListener('resize', handleResize);
    visualViewport.addEventListener('scroll', handleResize);

    return () => {
      visualViewport.removeEventListener('resize', handleResize);
      visualViewport.removeEventListener('scroll', handleResize);
    };
  }, []);

  return viewportState;
}
