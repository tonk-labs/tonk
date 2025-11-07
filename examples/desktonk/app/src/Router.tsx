import { BrowserRouter, Routes, Route } from 'react-router-dom';
import { Desktop } from './features/desktop';
import { TextEditorApp } from './features/text-editor';

function Router() {
  return (
    <BrowserRouter basename={import.meta.env.DEV ? "/app" : "/"}>
      <Routes>
        <Route path="/" element={<Desktop />} />
        <Route path="/text-editor" element={<TextEditorApp />} />
      </Routes>
    </BrowserRouter>
  );
}

export default Router;
