// Preload script runs in the renderer process
// but has access to Node.js APIs
const { ipcRenderer } = require('electron');

// Expose IPC renderer to the window object
window.ipcRenderer = ipcRenderer;

// Add any other APIs you want to expose to the renderer process
window.tonkAPI = {
  executeCommand: (command) => {
    ipcRenderer.send('tonk-command', command);
    return new Promise((resolve) => {
      ipcRenderer.once('command-result', (_, result) => {
        resolve(result);
      });
    });
  }
};

// Let the renderer know when the page is ready
window.addEventListener('DOMContentLoaded', () => {
  console.log('DOM fully loaded and parsed');
});
