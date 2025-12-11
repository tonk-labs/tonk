import { useCallback, useEffect, useState } from 'react';

function getInitialDarkMode(): boolean {
  if (typeof window === 'undefined') return false;
  return (
    localStorage.theme === 'dark' ||
    (!('theme' in localStorage) && window.matchMedia('(prefers-color-scheme: dark)').matches)
  );
}

export function useTheme() {
  const [isDark, setIsDark] = useState(getInitialDarkMode);

  // Apply dark class on mount and when isDark changes
  useEffect(() => {
    document.documentElement.classList.toggle('dark', isDark);
  }, [isDark]);

  // Listen for localStorage changes (from launcher or other tabs)
  useEffect(() => {
    const handleStorage = (e: StorageEvent) => {
      if (e.key === 'theme') {
        setIsDark(e.newValue === 'dark');
      }
    };
    window.addEventListener('storage', handleStorage);
    return () => window.removeEventListener('storage', handleStorage);
  }, []);

  const toggleTheme = useCallback(() => {
    const newIsDark = !isDark;
    localStorage.setItem('theme', newIsDark ? 'dark' : 'light');
    setIsDark(newIsDark);
  }, [isDark]);

  return { isDark, toggleTheme };
}
