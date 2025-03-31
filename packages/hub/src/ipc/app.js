const { ipcMain, shell } = require('electron');
const { getConfig } = require('../config.js');
const path = require('node:path');

ipcMain.handle('launch-app', async (event, projectPath) => {
  try {
    // Launch the app with the docId as a query parameter
    // You'll need to adjust this based on how your app is actually launched
    const child = require('child_process');
    // Assuming you're using npm start or similar to launch the app
    let config = getConfig();
    let storesPath = path.join(config.home, 'stores');
    child.exec(`cd "${projectPath}" && tonk serve --f ${storesPath}`, (error, stdout, stderr) => {
      if (error) {
        console.error(`Error launching app: ${error}`);
        return;
      }
    });

    return true;
  } catch (error) {
    console.error('Error launching app:', error);
    throw error;
  }
});

ipcMain.handle('open-external-link', async (event, link) => {
  shell.openExternal(link);
})