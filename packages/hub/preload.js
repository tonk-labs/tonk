// preload.js
const { contextBridge, ipcRenderer } = require("electron");
const { dialog } = require("@electron/remote");
const { app } = require("@electron/remote");

contextBridge.exposeInMainWorld("electronAPI", {
  launchApp: (projectPath) => ipcRenderer.invoke("launch-app", projectPath),
  launchAppDev: (projectPath) =>
    ipcRenderer.invoke("launch-app-dev", projectPath),
  isAppRunning: () => {
    return ipcRenderer.invoke("is-app-running");
  },
  openExternal: (link) => ipcRenderer.invoke("open-external-link", link),
  openUrlInElectron: (url) => ipcRenderer.invoke("open-url-in-electron", url),
  stopAndReset: () => ipcRenderer.invoke("stop-and-reset"),
  getConfig: () => ipcRenderer.invoke("get-config"),
  init: (homePath) => ipcRenderer.invoke("init", homePath),
  clearConfig: () => ipcRenderer.invoke("clear-config"),
  copyHubTemplate: () => ipcRenderer.invoke("copy-hub-template"),
  readFile: (filePath) => ipcRenderer.invoke("read-file", filePath),
  readBinary: (filePath) => ipcRenderer.invoke("read-binary", filePath),
  writeFile: (filePath, content) =>
    ipcRenderer.invoke("write-file", filePath, content),
  ls: (filePath) => ipcRenderer.invoke("ls", filePath),
  platformSensitiveJoin: (paths) =>
    ipcRenderer.invoke("platform-sensitive-join", paths),
  showOpenDialog: (options) => dialog.showOpenDialog(options),
  runShell: (dirPath) => ipcRenderer.invoke("run-shell", dirPath),
  closeShell: () => ipcRenderer.invoke("close-shell"),
  createApp: (name) => ipcRenderer.invoke("create-app", name),
  startFileWatching: () => ipcRenderer.invoke("start-file-watching"),
  stopFileWatching: () => ipcRenderer.invoke("stop-file-watching"),
  fetchRegistry: () => ipcRenderer.invoke("fetch-registry"),
  getDocumentsPath: () => app.getPath("documents"),
  installIntegration: (integrationLink) =>
    ipcRenderer.invoke("install-integration", integrationLink),
  getInstalledIntegrations: () =>
    ipcRenderer.invoke("get-installed-integrations"),
  runServer: (restart) => ipcRenderer.invoke("run-server", restart),
  saveDocument: (id, content) =>
    ipcRenderer.invoke("save-document", id, content),
  readDocument: (id) => ipcRenderer.invoke("read-document", id),
});

// Add IPC listener for file changes
ipcRenderer.on("file-change", (_event, payload) => {
  // Dispatch a custom event that the React app can listen to
  window.dispatchEvent(new CustomEvent("file-change", { detail: payload }));
});

// All the Node.js APIs are available in the preload process.
// It has the same sandbox as a Chrome extension.
window.addEventListener("DOMContentLoaded", () => {
  const replaceText = (selector, text) => {
    const element = document.getElementById(selector);
    if (element) element.innerText = text;
  };

  for (const dependency of ["chrome", "node", "electron"]) {
    replaceText(`${dependency}-version`, process.versions[dependency]);
  }

  // Initialize global objects for WebAssembly compatibility
  if (!window.wbg) {
    window.wbg = {};
  }
});
