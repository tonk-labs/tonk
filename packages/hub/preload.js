// preload.js
const { contextBridge, ipcRenderer } = require('electron');
const { dialog } = require('@electron/remote');

contextBridge.exposeInMainWorld('electronAPI', {
  launchApp: (projectPath) => ipcRenderer.invoke('launch-app', projectPath),
  getConfig: () => ipcRenderer.invoke('get-config'),
  init: (homePath) => ipcRenderer.invoke('init', homePath),
  readFile: (filePath) => ipcRenderer.invoke('readFile', filePath),
  writeFile: (filePath, content) => ipcRenderer.invoke('writeFile', filePath, content),
  ls: (filePath) => ipcRenderer.invoke('ls', filePath),
  showOpenDialog: (options) => dialog.showOpenDialog(options)
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
