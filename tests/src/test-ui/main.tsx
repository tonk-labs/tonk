import React from 'react';
import ReactDOM from 'react-dom/client';
import App from './App';

async function initializeApp() {
  // Check if service workers are supported
  if (!('serviceWorker' in navigator)) {
    console.error('Service workers are not supported in this browser');
    alert(
      'Service workers are required but not supported in this browser. Please use Chrome, Edge, or Safari.'
    );
    return;
  }

  try {
    console.log('[TEST-UI] Registering service worker...');

    // Register the service worker
    const registration = await navigator.serviceWorker.register(
      '/service-worker.js',
      { type: 'module', scope: '/' }
    );

    console.log('[TEST-UI] Service worker registered:', registration);

    // Wait for service worker to be ready
    await navigator.serviceWorker.ready;
    console.log('[TEST-UI] Service worker is ready');

    // If there's no controller yet, wait for it
    if (!navigator.serviceWorker.controller) {
      console.log('[TEST-UI] Waiting for service worker to take control...');
      await new Promise<void>(resolve => {
        navigator.serviceWorker.addEventListener(
          'controllerchange',
          () => {
            console.log('[TEST-UI] Service worker now controls the page');
            resolve();
          },
          { once: true }
        );
      });
    } else {
      console.log('[TEST-UI] Service worker already controlling page');
    }

    // Small delay to ensure service worker is fully initiated
    await new Promise(resolve => setTimeout(resolve, 100));
  } catch (err) {
    console.error('[TEST-UI] Service worker registration failed:', err);
    return;
  }

  const root = document.getElementById('root');
  if (root) {
    ReactDOM.createRoot(root).render(
      <React.StrictMode>
        <App />
      </React.StrictMode>
    );
  }
}

initializeApp();
