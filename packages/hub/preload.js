// Preload script for the main window
const { ipcRenderer } = require('electron');

// Expose the API to the renderer process
window.tonkAPI = {
  // Select a project directory
  selectProject: async () => {
    return await ipcRenderer.invoke('select-project');
  },

  // Launch a Tonk app
  launchApp: async (projectPath) => {
    return await ipcRenderer.invoke('launch-app', projectPath);
  }
};

// Expose ipcRenderer for event listening
window.ipcRenderer = ipcRenderer;
