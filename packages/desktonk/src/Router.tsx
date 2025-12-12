import { BrowserRouter, Route, Routes } from "react-router-dom";
import { RootLayout } from "./components/layout/RootLayout";
import { FeatureFlagProvider } from "./contexts/FeatureFlagContext";
import { Desktop } from "./features/desktop";
import { DesktopErrorBoundary } from "./features/desktop/components/DesktopErrorBoundary";
import { usePresenceTracking } from "./features/presence";
import { TextEditorApp } from "./features/text-editor";
import "./global.css";

// Initialize dark mode from localStorage before React renders
// This prevents flash of wrong theme
const initDarkMode = () => {
  const isDark =
    localStorage.theme === "dark" ||
    (!("theme" in localStorage) &&
      window.matchMedia("(prefers-color-scheme: dark)").matches);
  document.documentElement.classList.toggle("dark", isDark);
};
initDarkMode();

// Apply theme change and sync to localStorage
function applyTheme(isDark: boolean) {
  document.documentElement.classList.toggle("dark", isDark);
  localStorage.setItem("theme", isDark ? "dark" : "light");
  // Dispatch custom event for components that need to react (e.g., tldraw)
  window.dispatchEvent(
    new CustomEvent("theme-changed", { detail: { isDark } }),
  );
}

// Listen for theme changes from parent window (launcher)
if (typeof window !== "undefined") {
  window.addEventListener("message", (event) => {
    if (event.data?.type === "theme-change") {
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
// Handles: /space/<space-name> (launcher), standalone dev mode
function getBasename(): string {
  const path = window.location.pathname;

  // Running in launcher: /space/<space-name>/...
  const spaceMatch = path.match(/^\/space\/([^/]+)/);
  if (spaceMatch) {
    return `/space/${spaceMatch[1]}`;
  }

  // Standalone dev mode: use first path segment as basename
  const segments = path.split("/").filter(Boolean);
  if (segments.length > 0) {
    return `/${segments[0]}`;
  }

  return "/";
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
              <Route
                path="/text-editor"
                element={
                  <DesktopErrorBoundary>
                    <TextEditorApp />
                  </DesktopErrorBoundary>
                }
              />
            </Route>
          </Routes>
        </BrowserRouter>
      </AppInit>
    </FeatureFlagProvider>
  );
}

export default Router;
