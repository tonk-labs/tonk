import { BrowserRouter, Routes, Route } from 'react-router-dom';
import { Desktop } from './features/desktop';
import { DesktopErrorBoundary } from './features/desktop/components/DesktopErrorBoundary';
import { TextEditorApp } from './features/text-editor';
import "./global.css";

function Router() {
  return (
    <BrowserRouter basename={import.meta.env.DEV ? "/app" : "/"}>
      <Routes>
        <Route
          path="/"
          element={
            <DesktopErrorBoundary>
              <Desktop />
            </DesktopErrorBoundary>
          }
        />
        <Route path="/text-editor" element={<TextEditorApp />} />
      </Routes>
    </BrowserRouter>
  );
}

export default Router;
