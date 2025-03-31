// preload.js
const { contextBridge, ipcRenderer } = require('electron');
const { dialog } = require('@electron/remote');

contextBridge.exposeInMainWorld('electronAPI', {
  launchApp: (projectPath) => ipcRenderer.invoke('launch-app', projectPath),
  openExternal: (link) => ipcRenderer.invoke('open-external-link', link),
  getConfig: () => ipcRenderer.invoke('get-config'),
  init: (homePath) => ipcRenderer.invoke('init', homePath),
  copyHubTemplate: () => ipcRenderer.invoke('copy-hub-template'),
  readFile: (filePath) => ipcRenderer.invoke('read-file', filePath),
  writeFile: (filePath, content) => ipcRenderer.invoke('write-file', filePath, content),
  ls: (filePath) => ipcRenderer.invoke('ls', filePath),
  platformSensitiveJoin: (paths) => ipcRenderer.invoke('platform-sensitive-join', paths),
  showOpenDialog: (options) => dialog.showOpenDialog(options),
  runShell: (dirPath) => ipcRenderer.invoke('run-shell', dirPath),
  closeShell: () => ipcRenderer.invoke('close-shell')
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
