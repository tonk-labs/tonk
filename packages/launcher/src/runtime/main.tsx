import { createRoot } from 'react-dom/client';
import { RuntimeApp } from './RuntimeApp';
import './index.css';

const rootElement = document.getElementById('root');
if (rootElement) {
  createRoot(rootElement).render(<RuntimeApp />);
}
