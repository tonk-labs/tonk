import { BrowserRouter, Routes, Route } from 'react-router-dom';
import { Desktop } from './features/desktop';
import App from './App';

function Router() {
  return (
    <BrowserRouter basename={import.meta.env.DEV ? "/app" : "/"}>
      <Routes>
        <Route path="/" element={<Desktop />} />
        <Route path="/text-editor" element={<App />} />
      </Routes>
    </BrowserRouter>
  );
}

export default Router;
