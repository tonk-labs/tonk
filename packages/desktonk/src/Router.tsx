import { BrowserRouter, Routes, Route } from 'react-router-dom';
import { Desktop } from './features/desktop';
import { DesktopErrorBoundary } from './features/desktop/components/DesktopErrorBoundary';
import { TextEditorApp } from './features/text-editor';
import { FeatureFlagProvider } from './contexts/FeatureFlagContext';
import { RootLayout } from './components/layout/RootLayout';
import { usePresenceTracking } from './features/presence';
import './global.css';

// Initialize dark mode from localStorage before React renders
// This prevents flash of wrong theme
const initDarkMode = () => {
  const isDark =
    localStorage.theme === 'dark' ||
    (!('theme' in localStorage) && window.matchMedia('(prefers-color-scheme: dark)').matches);
  document.documentElement.classList.toggle('dark', isDark);
};
initDarkMode();

// Apply theme change and sync to localStorage
function applyTheme(isDark: boolean) {
  document.documentElement.classList.toggle('dark', isDark);
  localStorage.setItem('theme', isDark ? 'dark' : 'light');
  // Dispatch custom event for components that need to react (e.g., tldraw)
  window.dispatchEvent(new CustomEvent('theme-changed', { detail: { isDark } }));
}

// Listen for theme changes from parent window (launcher)
if (typeof window !== 'undefined') {
  window.addEventListener('message', (event) => {
    if (event.data?.type === 'theme-change') {
      applyTheme(event.data.isDark);
    }
  });
}

function AppInit({ children }: { children: React.ReactNode }) {
  // Initialize presence tracking (VFS sync is handled by middleware)
  usePresenceTracking();

  return <>{children}</>;
}

// Calculate basename from current URL
// Dev: /app, Production: /runtime/{uuid}/app
function getBasename(): string {
  if (import.meta.env.DEV) {
    return '/app';
  }

  // Find path up to and including /app
  const path = window.location.pathname;
  const appIndex = path.indexOf('/app');
  if (appIndex !== -1) {
    return path.substring(0, appIndex + 4);
  }
  return '/';
}

function Router() {
  return (
    <FeatureFlagProvider>
      <AppInit>
        <BrowserRouter basename={getBasename()}>
          <Routes>
            <Route element={<RootLayout />}>
              <Route
                path="/"
                element={
                  <DesktopErrorBoundary>
                    <Desktop />
                  </DesktopErrorBoundary>
                }
              />
              <Route path="/text-editor" element={<TextEditorApp />} />
            </Route>
          </Routes>
        </BrowserRouter>
      </AppInit>
    </FeatureFlagProvider>
  );
}

export default Router;
