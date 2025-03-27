const { contextBridge } = require('electron');

// Wait for the window to load
window.addEventListener('DOMContentLoaded', () => {
  // Get the docId from the query parameters
  const urlParams = new URLSearchParams(window.location.search);
  const docId = urlParams.get('docId');

  if (docId) {
    console.log(`Injecting document ID: ${docId}`);

    // Expose the docId to the renderer process
    contextBridge.exposeInMainWorld('TONK_DOC_ID', docId);
  }

  // Patch fetch to handle WASM files properly
  const originalFetch = window.fetch;
  window.fetch = async function(url, options) {
    if (typeof url === 'string' && url.endsWith('.wasm')) {
      console.log('Intercepting WASM fetch:', url);

      // If it's a file:// URL, convert it to a blob URL
      if (url.startsWith('file://')) {
        try {
          // Use XMLHttpRequest to load the file as an ArrayBuffer
          const xhr = new XMLHttpRequest();
          xhr.open('GET', url, true);
          xhr.responseType = 'arraybuffer';

          const response = await new Promise((resolve, reject) => {
            xhr.onload = () => {
              if (xhr.status === 200) {
                const blob = new Blob([xhr.response], { type: 'application/wasm' });
                const blobUrl = URL.createObjectURL(blob);
                resolve(new Response(blob, {
                  status: 200,
                  headers: new Headers({ 'Content-Type': 'application/wasm' })
                }));
              } else {
                reject(new Error(`Failed to load WASM file: ${xhr.statusText}`));
              }
            };
            xhr.onerror = () => reject(new Error('Network error loading WASM file'));
            xhr.send();
          });

          return response;
        } catch (error) {
          console.error('Error loading WASM file:', error);
          throw error;
        }
      }
    }

    // For all other requests, use the original fetch
    return originalFetch.apply(this, arguments);
  };
});

// Patch service worker registration to handle WASM files
if ('serviceWorker' in navigator) {
  const originalRegister = navigator.serviceWorker.register;
  navigator.serviceWorker.register = function(scriptURL, options) {
    console.log('Intercepting service worker registration:', scriptURL);

    // Add a custom handler for WASM files in the service worker
    return originalRegister.call(this, scriptURL, options).then(registration => {
      console.log('Service worker registered successfully');
      return registration;
    }).catch(error => {
      console.error('Service worker registration failed:', error);
      throw error;
    });
  };
}
