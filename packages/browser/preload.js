const { contextBridge, ipcRenderer } = require('electron')

// Expose file operations to the renderer process
contextBridge.exposeInMainWorld('electronAPI', {
  // Open file dialog
  openFileDialog: () => ipcRenderer.invoke('open-file-dialog'),
  
  // Listen for file data from main process
  onFileReceived: (callback) => ipcRenderer.on('file-received', (event, fileData) => callback(fileData))
})