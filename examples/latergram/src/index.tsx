import React from 'react';
import './index.css';
import { install } from '@twind/core';
import * as ReactDOM from 'react-dom/client';
import { createRoot } from 'react-dom/client';
import { BrowserRouter } from 'react-router-dom';
import * as ReactRouterDOM from 'react-router-dom';
import * as zustand from 'zustand';
import * as ChakraUI from '@chakra-ui/react';
import {
  ChakraProvider,
  defaultSystem,
  createToaster,
  Toaster,
} from '@chakra-ui/react';
import App from './App';
import { getAgentService } from './lib/agent/agent-service';
import { sync } from './middleware';
import { getUserService } from './services/user-service';
import { getVFSService } from './services/vfs-service';
import twConfig from './twind.config';

install(twConfig);

(window as any).React = React;
(window as any).ReactDOM = ReactDOM;

(window as any).ReactRouterDOM = ReactRouterDOM;

(window as any).zustand = zustand;
(window as any).sync = sync;

(window as any).ChakraUI = ChakraUI;
(window as any).defaultSystem = defaultSystem;
(window as any).createToaster = createToaster;

const mainToaster = createToaster({
  placement: 'bottom-end',
  pauseOnPageIdle: true,
});

const container = document.getElementById('root');
if (!container) throw new Error('Failed to find the root element');
const root = createRoot(container);

const slug = localStorage.getItem('appSlug');
const basename = slug !== null ? `/${slug}/` : '';

const isLocalhost =
  window.location.hostname === 'localhost' ||
  window.location.hostname === '127.0.0.1';
const relayUrl = isLocalhost
  ? 'http://ec2-16-16-146-55.eu-north-1.compute.amazonaws.com:8080'
  : 'https://relay.tonk.xyz';

const manifestUrl = `${relayUrl}/.manifest.tonk`;
const wsUrl = relayUrl.replace(/^http/, 'ws');

const initializeVFS = async () => {
  try {
    console.log('🔵 INIT: Starting VFS initialization...');
    const vfs = getVFSService();
    await vfs.initialize(manifestUrl, wsUrl);
    console.log('✅ INIT: VFS service initialized successfully');
  } catch (error) {
    console.error('❌ INIT: Failed to initialize VFS service:', error);
    throw error;
  }
};

const initialize = async () => {
  try {
    console.log('🚀 INIT: Starting full initialization...');

    console.log('🔵 INIT: About to initialize VFS...');
    await initializeVFS();
    console.log('✅ INIT: VFS initialization completed');

    console.log('🔵 INIT: About to initialize user service...');
    await getUserService().initialize();
    console.log('✅ INIT: User service initialized');

    console.log('🔵 INIT: About to initialize agent service...');
    await getAgentService().initialize({ manifestUrl, wsUrl });
    console.log('✅ INIT: Agent service initialized');

    console.log('🔵 INIT: About to render React app...');
    root.render(
      <React.StrictMode>
        <ChakraProvider value={defaultSystem}>
          <BrowserRouter basename={basename}>
            <App />
            <Toaster toaster={mainToaster} />
          </BrowserRouter>
        </ChakraProvider>
      </React.StrictMode>
    );
    console.log('✅ INIT: React app rendered successfully!');
  } catch (error) {
    console.error('💥 INIT: Initialization failed at some point:', error);
  }
};

initialize();
