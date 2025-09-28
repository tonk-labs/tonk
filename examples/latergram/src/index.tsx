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

const pathSegments = window.location.pathname.split('/');
const basename = '/latergram/';
// import.meta.env.VITE_BASE_PATH !== '/'
//   ? import.meta.env.VITE_BASE_PATH?.replace(/\/$/, '')
//   : '/latergram/';

const URL = import.meta.env.VITE_BASE_URL || 'http://localhost:8081';
const URL_LOCATION = URL.replace(/^https?:\/\//, '');
const manifestUrl = `${URL}/.manifest.tonk`;
const wsUrl = `ws://${URL_LOCATION}`;

// Initialize VFS for sync middleware
const initializeVFS = async () => {
  try {
    console.log('🔵 INIT: Starting VFS initialization...');
    const vfs = getVFSService();
    await vfs.initialize(manifestUrl, wsUrl);
    console.log('✅ INIT: VFS service initialized successfully');
  } catch (error) {
    console.error('❌ INIT: Failed to initialize VFS service:', error);
    throw error; // Re-throw to see if this stops the chain
  }
};

// Initialize everything
const initialize = async () => {
  try {
    console.log('🚀 INIT: Starting full initialization...');

    // Initialize VFS first
    console.log('🔵 INIT: About to initialize VFS...');
    await initializeVFS();
    console.log('✅ INIT: VFS initialization completed');

    // Initialize dependents
    console.log('🔵 INIT: About to initialize user service...');
    await getUserService().initialize();
    console.log('✅ INIT: User service initialized');

    console.log('🔵 INIT: About to initialize agent service...');
    await getAgentService().initialize({ manifestUrl, wsUrl });
    console.log('✅ INIT: Agent service initialized');

    // Render the app
    console.log('🔵 INIT: About to render React app...');
    root.render(
      <React.StrictMode>
        <BrowserRouter basename={basename}>
          <App />
        </BrowserRouter>
      </React.StrictMode>
    );
    console.log('✅ INIT: React app rendered successfully!');
  } catch (error) {
    console.error('💥 INIT: Initialization failed at some point:', error);
  }
};

initialize();
