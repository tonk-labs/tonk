// preload.js
const { contextBridge, ipcRenderer } = require('electron');

contextBridge.exposeInMainWorld('electronAPI', {
  getCurrentDocId: () => ipcRenderer.invoke('get-current-doc-id'),
  getProjectDocIds: (projectPath) => ipcRenderer.invoke('get-project-doc-ids', projectPath),
  saveDocId: (projectPath, docId) => ipcRenderer.invoke('save-doc-id', projectPath, docId),
  launchApp: (path, docId) => ipcRenderer.invoke('launch-app', path, docId)
});

// All the Node.js APIs are available in the preload process.
// It has the same sandbox as a Chrome extension.
window.addEventListener('DOMContentLoaded', () => {
  const replaceText = (selector, text) => {
    const element = document.getElementById(selector)
    if (element) element.innerText = text
  }

  for (const dependency of ['chrome', 'node', 'electron']) {
    replaceText(`${dependency}-version`, process.versions[dependency])
  }

  // Initialize global objects for WebAssembly compatibility
  if (!window.wbg) {
    window.wbg = {};
  }
})
