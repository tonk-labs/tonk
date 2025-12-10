// --- Icons ---
import { Moon, Sun } from 'lucide-react';
import * as React from 'react';
// --- UI Primitives ---
import { Button } from '@/features/editor/components/tiptap-ui-primitive/button';

export function ThemeToggle() {
  const [isDarkMode, setIsDarkMode] = React.useState<boolean>(() => {
    return (
      localStorage.theme === 'dark' ||
      (!('theme' in localStorage) &&
        window.matchMedia('(prefers-color-scheme: dark)').matches)
    );
  });

  // Listen for system preference changes (only when no localStorage value)
  React.useEffect(() => {
    const mediaQuery = window.matchMedia('(prefers-color-scheme: dark)');
    const handleChange = () => {
      if (!('theme' in localStorage)) {
        setIsDarkMode(mediaQuery.matches);
      }
    };
    mediaQuery.addEventListener('change', handleChange);
    return () => mediaQuery.removeEventListener('change', handleChange);
  }, []);

  // Apply dark class to document
  React.useEffect(() => {
    document.documentElement.classList.toggle('dark', isDarkMode);
  }, [isDarkMode]);

  // Toggle and persist to localStorage
  const toggleDarkMode = () => {
    setIsDarkMode(isDark => {
      const newValue = !isDark;
      localStorage.setItem('theme', newValue ? 'dark' : 'light');
      return newValue;
    });
  };

  return (
    <Button
      onClick={toggleDarkMode}
      aria-label={`Switch to ${isDarkMode ? 'light' : 'dark'} mode`}
      data-style="ghost"
    >
      {isDarkMode ? (
        <Moon className="tiptap-button-icon" />
      ) : (
        <Sun className="tiptap-button-icon" />
      )}
    </Button>
  );
}
