import { BrowserRouter, Routes, Route } from 'react-router-dom';
import App from './App';

function Router() {
  return (
    <BrowserRouter basename={import.meta.env.DEV ? "/app" : "/"}>
      <Routes>
        <Route path="/" element={<div>Desktop placeholder</div>} />
        <Route path="/text-editor" element={<App />} />
      </Routes>
    </BrowserRouter>
  );
}

export default Router;
