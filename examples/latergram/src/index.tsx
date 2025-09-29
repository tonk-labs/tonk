import React from 'react';
import './index.css';
import { createRoot } from 'react-dom/client';
import * as ReactDOM from 'react-dom/client';
import { BrowserRouter } from 'react-router-dom';
import App from './App';
import { getVFSService } from './services/vfs-service';
import { getUserService } from './services/user-service';
import * as zustand from 'zustand';
import { sync } from './middleware';
import { getAgentService } from './lib/agent/agent-service';
import { install } from '@twind/core';
import twConfig from './twind.config';

// Using TWIND to get tailwind
install(twConfig);

// Make React available globally for hot injection
(window as any).React = React;
(window as any).ReactDOM = ReactDOM;

// Make zustand and middleware available globally for store compilation
(window as any).zustand = zustand;
(window as any).sync = sync;

const container = document.getElementById('root');
if (!container) throw new Error('Failed to find the root element');
const root = createRoot(container);

const basename =
  import.meta.env.VITE_BASE_PATH !== '/'
    ? import.meta.env.VITE_BASE_PATH?.replace(/\/$/, '')
    : '';

const URL = import.meta.env.VITE_BASE_URL || 'http://localhost:8081';
const URL_LOCATION = URL.replace(/^https?:\/\//, '');
const manifestUrl = `${URL}/.manifest.tonk`;
const wsUrl = `ws://${URL_LOCATION}`;

// Initialize VFS for sync middleware
const initializeVFS = async () => {
  try {
    console.log('Initializing VFS service...');
    const vfs = getVFSService();
    await vfs.initialize(manifestUrl, wsUrl);
    console.log('VFS service initialized successfully');
  } catch (error) {
    console.error('Failed to initialize VFS service:', error);
  }
};

// Initialize everything
const initialize = async () => {
  // Initialize VFS first
  await initializeVFS();
  // Initialize dependents
  await getUserService().initialize();
  await getAgentService().initialize({ manifestUrl, wsUrl });

  // Render the app
  root.render(
    <React.StrictMode>
      <BrowserRouter basename={basename}>
        <App />
      </BrowserRouter>
    </React.StrictMode>
  );
};

initialize();
