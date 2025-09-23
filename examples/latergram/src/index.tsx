import React from 'react';
import './index.css';
import { createRoot } from 'react-dom/client';
import * as ReactDOM from 'react-dom/client';
import { BrowserRouter } from 'react-router-dom';
import App from './App';
import { getVFSService } from './services/vfs-service';

// Make React available globally for hot injection
(window as any).React = React;
(window as any).ReactDOM = ReactDOM;

const container = document.getElementById('root');
if (!container) throw new Error('Failed to find the root element');
const root = createRoot(container);

const basename =
  import.meta.env.VITE_BASE_PATH !== '/'
    ? import.meta.env.VITE_BASE_PATH?.replace(/\/$/, '')
    : '';

const URL = 'localhost:8081';

// Initialize VFS for sync middleware
const initializeVFS = async () => {
  try {
    console.log('Initializing VFS service...');
    const vfs = getVFSService();
    const manifestUrl = `http://${URL}/.manifest.tonk`;
    const wsUrl = `ws://${URL}`;
    // const wsUrl = `ws://localhost:8081`;
    await vfs.initialize(manifestUrl, wsUrl);
    console.log('VFS service initialized successfully');
  } catch (error) {
    console.error('Failed to initialize VFS service:', error);
  }
};

initializeVFS();

root.render(
  <React.StrictMode>
    <BrowserRouter basename={basename}>
      <App />
    </BrowserRouter>
  </React.StrictMode>
);
