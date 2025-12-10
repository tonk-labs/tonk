import { useCallback, useEffect, useState } from 'react';

// Broadcast theme change to all iframes
function broadcastThemeToIframes(isDark: boolean) {
  const iframes = document.querySelectorAll('iframe');
  iframes.forEach(iframe => {
    iframe.contentWindow?.postMessage({ type: 'theme-change', isDark }, '*');
  });
}

export function useTheme() {
  const [isDark, setIsDark] = useState(() => {
    if (typeof window !== 'undefined') {
      return document.documentElement.classList.contains('dark');
    }
    return false;
  });

  // Listen for storage events (changes from other tabs)
  useEffect(() => {
    const handleStorage = (e: StorageEvent) => {
      if (e.key === 'theme') {
        const newIsDark = e.newValue === 'dark';
        document.documentElement.classList.toggle('dark', newIsDark);
        setIsDark(newIsDark);
        broadcastThemeToIframes(newIsDark);
      }
    };
    window.addEventListener('storage', handleStorage);
    return () => window.removeEventListener('storage', handleStorage);
  }, []);

  const toggleTheme = useCallback(() => {
    const isDarkNow = document.documentElement.classList.contains('dark');
    const newIsDark = !isDarkNow;

    document.documentElement.classList.toggle('dark', newIsDark);
    localStorage.setItem('theme', newIsDark ? 'dark' : 'light');
    setIsDark(newIsDark);
    broadcastThemeToIframes(newIsDark);
  }, []);

  return { isDark, toggleTheme };
}
